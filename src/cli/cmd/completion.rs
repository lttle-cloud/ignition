use std::{
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::Result;
use atty::Stream;
use clap::Command;
use clap_complete::Shell;

use crate::ui::message::{message_info, message_warn};

pub fn install_completion(shell: Shell, cmd: &mut Command) -> Result<()> {
    let auto_install = 'auto_install: {
        let outdir = match &shell {
            Shell::Zsh => zsh_target_dir(),
            Shell::Bash => bash_target_dir(),
            _ => break 'auto_install false,
        };

        let Ok(_) = std::fs::create_dir_all(&outdir) else {
            break 'auto_install false;
        };

        let file_path = outdir.join("lttle");

        let Ok(mut file) = File::create(&file_path) else {
            break 'auto_install false;
        };

        clap_complete::generate(shell.clone(), cmd, "lttle", &mut file);
        message_info(format!(
            "Installed completions for {} to {}",
            shell.to_string(),
            file_path.display()
        ));

        true
    };

    if !auto_install && atty::is(Stream::Stdout) {
        message_warn(format!(
            "Automatic installation is not supported (or failed) for {shell}. \n \
             The directory might not exist, or you might not have permission to write to it. \n \
             Please check the {shell} documentation for manual installation instructions. \n \
             To get the completion script, pipe this command to your shell's specific completion file. \n \
             For example bash on linux: `lttle completions bash > /etc/bash_completion.d/lttle`",
        ));

        return Ok(());
    }

    clap_complete::generate(shell.clone(), cmd, "lttle", &mut std::io::stdout());
    Ok(())
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = home::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

fn zsh_target_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        for d in &[
            "/opt/homebrew/share/zsh/site-functions", // Apple Silicon
            "/usr/local/share/zsh/site-functions",    // Intel / older setups
        ] {
            let p = Path::new(d);
            if p.is_dir()
                && p.metadata()
                    .map(|m| !m.permissions().readonly())
                    .unwrap_or(false)
            {
                return p.to_path_buf();
            }
        }
    }

    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Path::new(&xdg).join("zsh/site-functions");
    }
    expand_tilde("~/.local/share/zsh/site-functions")
}

fn bash_target_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        for d in &[
            "/opt/homebrew/etc/bash_completion.d",
            "/usr/local/etc/bash_completion.d",
        ] {
            let p = Path::new(d);
            if p.is_dir()
                && p.metadata()
                    .map(|m| !m.permissions().readonly())
                    .unwrap_or(false)
            {
                return p.to_path_buf();
            }
        }
    }

    PathBuf::from("/etc/bash_completion.d")
}
