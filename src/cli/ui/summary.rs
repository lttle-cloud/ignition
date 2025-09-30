use ansi_term::{Color, Style};
use terminal_size::terminal_size;

use crate::ui::END_PADDING;

use super::{COLUMN_GAP, DEFAULT_WIDTH};

#[derive(Debug, Clone)]
pub enum SummaryCellStyle {
    Default,
    Important,
}

impl SummaryCellStyle {
    pub fn get_style(&self) -> Style {
        match self {
            SummaryCellStyle::Default => Style::new(),
            SummaryCellStyle::Important => Style::new().fg(Color::Purple),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub name: String,
    pub cell_style: SummaryCellStyle,
    pub clip_value: bool,
    pub value: Vec<String>,
}

pub struct Summary {
    pub rows: Vec<SummaryRow>,
}

impl Summary {
    pub fn print(&self) {
        let terminal_width = terminal_size().map(|(w, _)| w.0).unwrap_or(DEFAULT_WIDTH);

        let names_max_width = self
            .rows
            .iter()
            .map(|r| r.name.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let values_max_width = terminal_width as usize - names_max_width - COLUMN_GAP - END_PADDING;

        for row in &self.rows {
            let front_padding = " ".repeat(names_max_width - row.name.chars().count());
            print!("{}", front_padding);

            print!("{}: ", Style::new().bold().paint(row.name.clone()));

            // print each value, start a new line after each and pad to the right
            for (i, value) in row.value.iter().enumerate() {
                let value_style = row.cell_style.get_style();
                let value = if value.chars().count() > values_max_width && row.clip_value {
                    let truncated: String = value.chars().take(values_max_width - 3).collect();
                    format!("{}...", value_style.paint(truncated))
                } else {
                    value_style.paint(value).to_string()
                };

                if i > 0 {
                    print!("\n{}{}", " ".repeat(names_max_width + COLUMN_GAP), value);
                } else {
                    print!("{}", value);
                }
            }

            println!();
        }
        println!();
    }
}
