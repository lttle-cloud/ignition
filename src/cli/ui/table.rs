use ansi_term::{Color, Style};
use terminal_size::terminal_size;

use super::{COLUMN_GAP, DEFAULT_WIDTH, END_PADDING};

#[derive(Debug, Clone)]
pub enum TableCellStyle {
    Default,
    Important,
}

#[derive(Debug, Clone)]
pub struct TableHeader {
    pub text: String,
    pub cell_style: TableCellStyle,
    pub max_width: Option<usize>,
}

impl TableCellStyle {
    pub fn get_style(&self) -> Style {
        match self {
            TableCellStyle::Default => Style::new(),
            TableCellStyle::Important => Style::new().bold().fg(Color::Purple),
        }
    }
}

pub struct Table {
    pub headers: Vec<TableHeader>,
    pub rows: Vec<Vec<Option<String>>>,
}

impl Table {
    pub fn print(&self) {
        let terminal_width = terminal_size().map(|(w, _)| w.0).unwrap_or(DEFAULT_WIDTH);

        let mut column_widths = self
            .headers
            .iter()
            .map(|h| h.text.len())
            .collect::<Vec<_>>();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                let max_width = self.headers[i].max_width.unwrap_or(usize::MAX);
                column_widths[i] = column_widths[i]
                    .min(max_width)
                    .max(cell.as_ref().map(|c| c.len()).unwrap_or(0));
            }
        }

        // shrink column widths to fit the terminal width
        loop {
            let gaps_width = if column_widths.len() > 1 {
                (column_widths.len() - 1) * COLUMN_GAP
            } else {
                0
            };
            let total_width: usize = column_widths.iter().sum::<usize>() + gaps_width + END_PADDING;
            if total_width <= terminal_width as usize {
                break;
            }

            // shrink the biggest columns first. if the there are multiple columns with the same width, shrink all of them. the minimum shrink is the len of the header + 2
            let max_width = *column_widths.iter().max().unwrap();
            let max_width_indices = column_widths
                .iter()
                .enumerate()
                .filter(|(_, w)| **w == max_width)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            let mut shrinked_count = 0;
            for index in max_width_indices.iter() {
                let new_width = column_widths[*index] - 1;
                if new_width < self.headers[*index].text.len() {
                    continue;
                }

                column_widths[*index] = new_width;
                shrinked_count += 1;
            }

            // if we can't shrink any more, break
            if shrinked_count == 0 && max_width_indices.len() != 0 {
                break;
            }
        }

        for (i, header) in self.headers.iter().enumerate() {
            print!("{}", Style::new().bold().paint(header.text.clone()));
            for _ in 0..(column_widths[i] - header.text.len()) {
                print!(" ");
            }
            if i < self.headers.len() - 1 {
                print!("{}", " ".repeat(COLUMN_GAP)); // gap between columns
            }
        }
        println!("{}", " ".repeat(END_PADDING)); // end padding

        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                let style = self.headers[i].cell_style.get_style();

                let cell_len = cell.as_ref().map(|c| c.len()).unwrap_or(0);
                if cell_len > column_widths[i] {
                    print!(
                        "{}",
                        style.paint(&cell.as_ref().unwrap()[..column_widths[i] - 3])
                    );
                    print!("{}", style.paint("..."));
                } else {
                    print!("{}", style.paint(cell.clone().unwrap_or("".to_string())));
                    for _ in 0..(column_widths[i] - cell_len) {
                        print!(" ");
                    }
                }
                if i < row.len() - 1 {
                    print!("{}", " ".repeat(COLUMN_GAP)); // gap between columns
                }
            }
            println!("{}", " ".repeat(END_PADDING)); // end padding
        }
    }
}
