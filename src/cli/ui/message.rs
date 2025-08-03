use ansi_term::{Color, Style};

use crate::ui::MESSAGE_PADDING;

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
