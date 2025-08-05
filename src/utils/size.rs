use anyhow::{Result, bail};

pub fn parse_human_readable_size(size: &str) -> Result<u64> {
    let s = size.trim();
    if s.is_empty() {
        bail!("empty size string");
    }

    let (num_part, unit_part) = s
        .chars()
        .enumerate()
        .find_map(|(i, c)| {
            if !c.is_ascii_digit() {
                Some((&s[..i], &s[i..]))
            } else {
                None
            }
        })
        .unwrap_or((s, "")); // no suffix

    let number: u64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number"))?;

    let mut unit = unit_part.trim().to_uppercase();
    if unit.ends_with('B') {
        unit.pop();
    }

    let shift = match unit.as_str() {
        "" => 0,
        "K" | "KI" => 10,
        "M" | "MI" => 20,
        "G" | "GI" => 30,
        "T" | "TI" => 40,
        "P" | "PI" => 50,
        _ => bail!("unknown size suffix: {unit_part}"),
    };

    Ok(number
        .checked_shl(shift)
        .ok_or(anyhow::anyhow!("size too large to fit in u64"))?)
}

pub fn format_human_readable_size(size: u64) -> String {
    let mut size = size;
    let mut shift = 0;

    while size >= 1024 && shift < 50 {
        size /= 1024;
        shift += 10;
    }

    let unit = match shift {
        10 => "K",
        20 => "M",
        30 => "G",
        40 => "T",
        50 => "P",
        _ => "B",
    };

    format!("{size}{unit}")
}
