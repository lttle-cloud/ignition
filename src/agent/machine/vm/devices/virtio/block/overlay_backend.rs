use anyhow::{Result, bail};
use nix::{
    errno::Errno,
    unistd::{Whence, lseek},
};
use std::{
    fs::File,
    io::{Seek, SeekFrom},
    os::fd::AsFd,
};
use vmm_sys_util::{
    file_traits::FileSync,
    write_zeroes::{PunchHole, WriteZeroesAt},
};

use vm_memory::{
    ReadVolatile, VolatileMemoryError, VolatileSlice, WriteVolatile, bitmap::BitmapSlice,
};

use crate::agent::machine::vm::constants::BLK_SIZE;

// pub trait Backend:
//     ReadVolatile + WriteVolatile + Seek + FileSync + PunchHole + WriteZeroesAt
// {
// }

#[derive(Clone, Copy)]
enum RunKind {
    Data,
    Hole,
}

/// Return (kind, run_len) where
/// * `kind == Data`  → overlay actually has bytes here
/// * `kind == Hole`  → overlay is sparse here, read from base file
fn next_run(fd: &mut (impl AsFd + Clone), off: u64, file_end: u64) -> nix::Result<(RunKind, u64)> {
    let off_i = off as i64;

    match lseek(fd.clone(), off_i, Whence::SeekData) {
        Ok(data_at) if data_at == off_i => {
            let hole_at = lseek(fd, off_i, Whence::SeekHole)?;
            Ok((RunKind::Data, (hole_at as u64) - off))
        }
        Ok(data_at) => {
            let run_end = std::cmp::min(data_at as u64, file_end);
            Ok((RunKind::Hole, run_end - off))
        }
        Err(Errno::ENXIO) => Ok((RunKind::Hole, file_end - off)),
        Err(e) => Err(e),
    }
}

pub enum OverlayBackend {
    Readonly {
        src_file: File,
    },
    Readwrite {
        src_file: File,
        ov_file: File,
        file_len: u64,
        pos: u64,
    },
}

impl Seek for OverlayBackend {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            OverlayBackend::Readonly { src_file } => src_file.seek(pos),
            OverlayBackend::Readwrite {
                src_file,
                ov_file,
                pos: cur,
                ..
            } => {
                let abs = src_file.seek(pos)?;
                ov_file.seek(SeekFrom::Start(abs))?;
                *cur = abs;
                Ok(abs)
            }
        }
    }
}

impl ReadVolatile for OverlayBackend {
    fn read_volatile<B: BitmapSlice>(
        &mut self,
        mut slice: &mut VolatileSlice<B>,
    ) -> Result<usize, VolatileMemoryError> {
        match self {
            // Pure passthrough when no overlay is present
            OverlayBackend::Readonly { src_file } => src_file.read_volatile(slice),

            // Copy‑on‑write path
            OverlayBackend::Readwrite {
                src_file,
                ov_file,
                file_len,
                pos,
            } => {
                let mut done = 0;

                let mut slice = slice.clone();
                while !slice.is_empty() && *pos < *file_len {
                    // Ask the kernel whether the overlay contains data at *pos*
                    let (kind, run_len) = next_run(&mut ov_file.as_fd(), *pos, *file_len)
                        .map_err(|e| VolatileMemoryError::IOError(e.into()))?;

                    // Never read more than the caller requested
                    let chunk = std::cmp::min(run_len as usize, slice.len());
                    let mut slice0 = slice.subslice(0, chunk)?;

                    // <<<  IMPORTANT: keep chosen file’s cursor in sync  >>>
                    match kind {
                        RunKind::Data => {
                            ov_file
                                .seek(SeekFrom::Start(*pos))
                                .map_err(VolatileMemoryError::IOError)?;
                            ov_file.read_exact_volatile(&mut slice0)?
                        }
                        RunKind::Hole => {
                            src_file
                                .seek(SeekFrom::Start(*pos))
                                .map_err(VolatileMemoryError::IOError)?;
                            src_file.read_exact_volatile(&mut slice0)?
                        }
                    };

                    slice = slice.offset(chunk)?;
                    *pos += chunk as u64;
                    done += chunk;
                }
                Ok(done)
            }
        }
    }
}

impl WriteVolatile for OverlayBackend {
    fn write_volatile<B: BitmapSlice>(
        &mut self,
        slice: &VolatileSlice<B>,
    ) -> Result<usize, VolatileMemoryError> {
        match self {
            OverlayBackend::Readonly { .. } => Err(VolatileMemoryError::IOError(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read‑only backend"),
            )),

            OverlayBackend::Readwrite {
                ov_file,
                file_len,
                pos,
                ..
            } => {
                let mut done = 0;
                let mut remain = slice.clone();

                while !remain.is_empty() {
                    // Split at BLK_SIZE only for convenience; any size works
                    let chunk = std::cmp::min(BLK_SIZE, remain.len());
                    let mut slice0 = remain.subslice(0, chunk)?;

                    // <<<  Keep overlay file cursor aligned with guest offset  >>>
                    ov_file
                        .seek(SeekFrom::Start(*pos))
                        .map_err(VolatileMemoryError::IOError)?;
                    ov_file.write_all_volatile(&mut slice0)?;

                    remain = remain.offset(chunk)?;
                    *pos += chunk as u64;
                    done += chunk;
                }

                *file_len = (*file_len).max(*pos); // maintain EOF tracker
                Ok(done)
            }
        }
    }
}

impl FileSync for OverlayBackend {
    /// Flush all dirty data to stable storage.
    fn fsync(&mut self) -> std::io::Result<()> {
        match self {
            OverlayBackend::Readonly { .. } => Ok(()),
            OverlayBackend::Readwrite { ov_file, .. } => ov_file.fsync(),
        }
    }
}

impl PunchHole for OverlayBackend {
    /// De‑allocate `len` bytes starting at `off`, turning them back into holes.
    fn punch_hole(&mut self, off: u64, len: u64) -> std::io::Result<()> {
        match self {
            OverlayBackend::Readonly { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "cannot punch hole on read‑only backend",
            )),
            OverlayBackend::Readwrite { ov_file, .. } => ov_file.punch_hole(off, len),
        }
    }
}

impl WriteZeroesAt for OverlayBackend {
    /// Ensure the range `[off, off+len)` reads back as zeroes.
    /// We write zeroes into the *overlay* so the base image data is hidden.
    fn write_zeroes_at(&mut self, off: u64, len: usize) -> std::io::Result<usize> {
        match self {
            OverlayBackend::Readonly { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "cannot write zeroes on read‑only backend",
            )),
            OverlayBackend::Readwrite {
                ov_file, file_len, ..
            } => {
                let written = ov_file.write_zeroes_at(off, len)?;
                *file_len = (*file_len).max(off + written as u64);
                Ok(written)
            }
        }
    }
}

impl OverlayBackend {
    pub fn new_readonly(src_file: File) -> Self {
        Self::Readonly { src_file }
    }

    pub fn new_readwrite(src_file: File, ov_file: File) -> Result<Self> {
        if src_file.metadata()?.len() != ov_file.metadata()?.len() {
            bail!("src_file and ov_file must have the same length");
        }

        let file_len = ov_file.metadata()?.len();

        Ok(Self::Readwrite {
            src_file,
            ov_file,
            file_len,
            pos: 0,
        })
    }
}
