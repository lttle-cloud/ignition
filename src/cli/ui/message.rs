use ansi_term::{Color, Style};

use crate::ui::{LOG_PADDING, MESSAGE_PADDING};

pub fn message_info(message: impl AsRef<str>) {
    let padding = "█".repeat(MESSAGE_PADDING) + " ";
    print!("{}", Style::new().fg(Color::Blue).bold().paint(padding));
    println!("{}", message.as_ref())
}

pub fn message_error(message: impl AsRef<str>) {
    let padding = "error: ";
    print!("{}", Style::new().fg(Color::Red).bold().paint(padding));
    println!("{}", message.as_ref())
}

pub fn message_warn(message: impl AsRef<str>) {
    let padding = "warning: ";
    print!("{}", Style::new().fg(Color::Yellow).bold().paint(padding));
    println!("{}", message.as_ref())
}

pub fn message_log_stdout(message: impl AsRef<str>, timestamp: Option<String>) {
    let padding = "█".repeat(LOG_PADDING) + " ";
    print!("{}", Style::new().fg(Color::Blue).bold().paint(padding));
    if let Some(timestamp) = timestamp {
        print!(
            "{} ",
            Style::new().fg(Color::Yellow).bold().paint(timestamp)
        );
    }
    println!("{}", message.as_ref())
}

pub fn message_log_stderr(message: impl AsRef<str>, timestamp: Option<String>) {
    let padding = "█".repeat(LOG_PADDING) + " ";
    print!("{}", Style::new().fg(Color::Red).bold().paint(padding));
    if let Some(timestamp) = timestamp {
        print!(
            "{} ",
            Style::new().fg(Color::Yellow).bold().paint(timestamp)
        );
    }
    println!("{}", message.as_ref())
}
