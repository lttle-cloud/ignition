use anyhow::Result;
use flate2::bufread::GzDecoder;
use std::{
    fs::{self, File},
    io::{self, BufReader},
    path::Path,
    path::PathBuf,
};
use tar::{Archive, EntryType};
use tracing::{error, info};

pub fn unpack_gzipped_tar(tar_path: impl AsRef<Path>, dest_dir: impl AsRef<Path>) -> Result<()> {
    let tar_path = tar_path.as_ref();
    let dest_dir = dest_dir.as_ref();

    info!(
        "Unpacking tar archive {} to {}",
        tar_path.display(),
        dest_dir.display()
    );

    let file = File::open(tar_path)?;
    let reader = BufReader::new(file);
    let decoder = GzDecoder::new(reader);
    let mut archive = Archive::new(decoder);

    // Configure archive settings
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.set_overwrite(true);
    archive.set_unpack_xattrs(false);

    // Canonicalize destination when possible
    let dst: PathBuf = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());

    info!("Starting sequential extraction to: {}", dst.display());

    // Hardlinks we couldn't create yet (because the target didn't exist at that moment)
    let mut pending_hardlinks: Vec<(PathBuf, PathBuf)> = Vec::new();

    // Helpers
    let ensure_parent = |p: &Path| -> io::Result<()> {
        if let Some(parent) = p.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        Ok(())
    };

    let clear_dir_contents = |dir: &Path| -> io::Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            let md = fs::symlink_metadata(&p)?;
            if md.file_type().is_dir() {
                fs::remove_dir_all(&p)?;
            } else {
                let _ = fs::remove_file(&p); // covers files & symlinks
            }
        }
        Ok(())
    };

    let is_whiteout = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.starts_with(".wh.") && s != ".wh..wh..opq")
            .unwrap_or(false)
    };

    let is_opaque_whiteout = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s == ".wh..wh..opq")
            .unwrap_or(false)
    };

    // Process entries in archive order (OCI layering semantics)
    for entry_result in archive.entries()? {
        let mut entry = entry_result?;

        let entry_path_rel: PathBuf = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(_) => {
                info!("Skipping entry with unreadable path");
                continue;
            }
        };
        let entry_type = entry.header().entry_type();
        let entry_info = entry_path_rel.display().to_string();

        // OCI whiteouts
        if is_opaque_whiteout(&entry_path_rel) {
            let target_dir = entry_path_rel
                .parent()
                .map(|p| dst.join(p))
                .unwrap_or_else(|| dst.clone());
            clear_dir_contents(&target_dir)?;
            // don't unpack the marker itself
            continue;
        }
        if is_whiteout(&entry_path_rel) {
            handle_whiteout(&entry_path_rel, &dst)?;
            continue;
        }

        match entry_type {
            EntryType::Directory => {
                let full = dst.join(&entry_path_rel);
                ensure_parent(&full)?;
                if let Err(e) = entry.unpack_in(&dst) {
                    error!("Failed to create directory {}: {}", entry_info, e);
                    return Err(e.into());
                }
            }

            EntryType::Regular => {
                let full = dst.join(&entry_path_rel);
                ensure_parent(&full)?;
                if full.exists() {
                    if full.is_dir() {
                        fs::remove_dir_all(&full)?;
                    } else {
                        let _ = fs::remove_file(&full);
                    }
                }
                if let Err(e) = entry.unpack_in(&dst) {
                    error!("Failed to extract file {}: {}", entry_info, e);
                    return Err(e.into());
                }
            }

            EntryType::Symlink => {
                let full = dst.join(&entry_path_rel);
                ensure_parent(&full)?;
                if full.exists() {
                    if full.is_dir() {
                        fs::remove_dir_all(&full)?;
                    } else {
                        let _ = fs::remove_file(&full);
                    }
                }
                if let Err(e) = entry.unpack_in(&dst) {
                    info!("Skipping problematic symlink {}: {}", entry_info, e);
                }
            }

            EntryType::Link => {
                // Hardlink: create now if target exists, otherwise defer
                let target_rel = match entry.link_name() {
                    Ok(Some(p)) => p.into_owned(),
                    _ => {
                        info!("Hardlink without target, skipping: {}", entry_info);
                        continue;
                    }
                };

                let link_abs = dst.join(&entry_path_rel);
                let target_abs = dst.join(&target_rel);

                ensure_parent(&link_abs)?;
                if link_abs.exists() {
                    if link_abs.is_dir() {
                        fs::remove_dir_all(&link_abs)?;
                    } else {
                        let _ = fs::remove_file(&link_abs);
                    }
                }

                // Check if target exists
                match fs::symlink_metadata(&target_abs) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        // Target is a symlink - create a copy of the symlink
                        match fs::read_link(&target_abs) {
                            Ok(symlink_target) => {
                                if let Err(e) =
                                    std::os::unix::fs::symlink(&symlink_target, &link_abs)
                                {
                                    info!(
                                        "Failed to create symlink copy {} -> {}: {}",
                                        link_abs.display(),
                                        symlink_target.display(),
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                info!(
                                    "Failed to read symlink target {}: {}",
                                    target_abs.display(),
                                    e
                                );
                            }
                        }
                    }
                    Ok(_) => {
                        // Target is a regular file/directory - try hardlink
                        if let Err(e) = fs::hard_link(&target_abs, &link_abs) {
                            // As a fallback, let tar try (helps preserve metadata in some cases)
                            if let Err(e2) = entry.unpack_in(&dst) {
                                info!(
                                    "Failed to hardlink {} -> {}: {}; fallback unpack failed: {}",
                                    link_abs.display(),
                                    target_abs.display(),
                                    e,
                                    e2
                                );
                            }
                        }
                    }
                    Err(_) => {
                        // Target doesn't exist - defer
                        info!(
                            "Hardlink target missing, deferring: {} -> {}",
                            link_abs.display(),
                            target_abs.display()
                        );
                        pending_hardlinks.push((link_abs, target_abs));
                    }
                }
            }

            _ => {
                if let Err(e) = entry.unpack_in(&dst) {
                    info!(
                        "Skipping unsupported/problematic entry {:?}: {} ({})",
                        entry_type, entry_info, e
                    );
                }
            }
        }
    }

    // Retry unresolved hardlinks a few times to allow targets that appeared later to resolve
    if !pending_hardlinks.is_empty() {
        let max_passes = 10usize;
        for pass in 1..=max_passes {
            let mut next = Vec::new();
            let mut progress = 0usize;

            for (link_abs, target_abs) in pending_hardlinks.into_iter() {
                match fs::symlink_metadata(&target_abs) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        // Target is a symlink - create a copy of the symlink
                        match fs::read_link(&target_abs) {
                            Ok(symlink_target) => {
                                if link_abs.exists() {
                                    if link_abs.is_dir() {
                                        fs::remove_dir_all(&link_abs)?;
                                    } else {
                                        let _ = fs::remove_file(&link_abs);
                                    }
                                }
                                match std::os::unix::fs::symlink(&symlink_target, &link_abs) {
                                    Ok(_) => progress += 1,
                                    Err(e) => {
                                        info!(
                                            "Pass {}: symlink copy still failing {} -> {} ({})",
                                            pass,
                                            link_abs.display(),
                                            symlink_target.display(),
                                            e
                                        );
                                        next.push((link_abs, target_abs));
                                    }
                                }
                            }
                            Err(_) => {
                                next.push((link_abs, target_abs));
                            }
                        }
                    }
                    Ok(_) => {
                        // Target is a regular file/directory - try hardlink
                        if link_abs.exists() {
                            if link_abs.is_dir() {
                                fs::remove_dir_all(&link_abs)?;
                            } else {
                                let _ = fs::remove_file(&link_abs);
                            }
                        }
                        match fs::hard_link(&target_abs, &link_abs) {
                            Ok(_) => progress += 1,
                            Err(e) => {
                                info!(
                                    "Pass {}: hardlink still failing {} -> {} ({})",
                                    pass,
                                    link_abs.display(),
                                    target_abs.display(),
                                    e
                                );
                                next.push((link_abs, target_abs));
                            }
                        }
                    }
                    Err(_) => {
                        next.push((link_abs, target_abs));
                    }
                }
            }

            if next.is_empty() || progress == 0 {
                pending_hardlinks = next;
                break;
            }
            pending_hardlinks = next;
        }

        for (link_abs, target_abs) in pending_hardlinks.into_iter() {
            info!(
                "Hardlink target missing after retries, skipping: {} -> {}",
                link_abs.display(),
                target_abs.display()
            );
        }
    }

    info!("Successfully unpacked tar archive {}", tar_path.display());
    Ok(())
}

fn handle_whiteout(whiteout_rel: &Path, dst: &Path) -> Result<()> {
    // .wh..wh..opq => opaque whiteout (clear the directory contents)
    if whiteout_rel
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s == ".wh..wh..opq")
        .unwrap_or(false)
    {
        let target_dir = whiteout_rel
            .parent()
            .map(|p| dst.join(p))
            .unwrap_or_else(|| dst.to_path_buf());

        if target_dir.exists() {
            for entry in fs::read_dir(&target_dir)? {
                let p = entry?.path();
                // skip the marker itself if somehow present
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s == ".wh..wh..opq")
                    .unwrap_or(false)
                {
                    continue;
                }
                let md = fs::symlink_metadata(&p)?;
                if md.file_type().is_dir() {
                    fs::remove_dir_all(&p)?;
                } else {
                    let _ = fs::remove_file(&p); // files & symlinks
                }
            }
        }
        return Ok(());
    }

    // Normal whiteout .wh.<name> => remove <name> in the same directory
    if let Some(file_name) = whiteout_rel.file_name() {
        if let Some(name_str) = file_name.to_str() {
            if let Some(stripped) = name_str.strip_prefix(".wh.") {
                let target_path = whiteout_rel
                    .parent()
                    .map(|p| dst.join(p).join(stripped))
                    .unwrap_or_else(|| dst.join(stripped));

                if target_path.exists() {
                    let md = fs::symlink_metadata(&target_path)?;
                    if md.file_type().is_dir() {
                        fs::remove_dir_all(&target_path)?;
                    } else {
                        let _ = fs::remove_file(&target_path); // files & symlinks
                    }
                }
            }
        }
    }

    Ok(())
}
