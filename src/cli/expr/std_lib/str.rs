use std::sync::Arc;

use cel::extractors::This;
use cel::{FunctionContext, Value};

pub fn last(ftx: &FunctionContext, This(s): This<Arc<String>>, count: i64) -> Arc<String> {
    if count < 0 {
        ftx.error("count must be positive");
        return Arc::new(String::new());
    }

    let count = count as usize;

    let chars = s.chars().collect::<Vec<_>>();
    let last_chars = if chars.len() > count {
        chars[chars.len() - count..].iter().collect::<String>()
    } else {
        chars.iter().collect::<String>()
    };

    Arc::new(last_chars)
}

pub fn to_slug(This(s): This<Arc<String>>) -> Arc<String> {
    slugify(s)
}

pub fn slugify(s: Arc<String>) -> Arc<String> {
    let mut output = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            output.push(c.to_ascii_lowercase());
        } else {
            if output.ends_with('-') {
                continue;
            }

            output.push('-');
        }
    }

    output = output
        .trim_start_matches('-')
        .trim_end_matches('-')
        .to_string();

    Arc::new(output)
}

pub fn char_at(ftx: &FunctionContext, This(s): This<Arc<String>>, index: i64) -> Arc<String> {
    if index < 0 {
        ftx.error("index must be positive");
        return Arc::new(String::new());
    }

    let index = index as usize;
    let chars: Vec<char> = s.chars().collect();

    if index < chars.len() {
        Arc::new(chars[index].to_string())
    } else {
        Arc::new(String::new())
    }
}

pub fn index_of(This(s): This<Arc<String>>, substr: Arc<String>) -> i64 {
    match s.find(&*substr) {
        Some(index) => index as i64,
        None => -1,
    }
}

pub fn join_list(list: Arc<Vec<Value>>, separator: Arc<String>) -> Arc<String> {
    let strings: Vec<String> = list
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.to_string()),
            _ => None,
        })
        .collect();
    Arc::new(strings.join(&*separator))
}

pub fn last_index_of(This(s): This<Arc<String>>, substr: Arc<String>) -> i64 {
    match s.rfind(&*substr) {
        Some(index) => index as i64,
        None => -1,
    }
}

pub fn lower_ascii(This(s): This<Arc<String>>) -> Arc<String> {
    let result: String = s
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect();
    Arc::new(result)
}

pub fn quote(This(s): This<Arc<String>>) -> Arc<String> {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    Arc::new(format!("\"{}\"", escaped))
}

pub fn replace(This(s): This<Arc<String>>, old: Arc<String>, new: Arc<String>) -> Arc<String> {
    Arc::new(s.replace(&*old, &*new))
}

pub fn split_string(This(s): This<Arc<String>>, separator: Arc<String>) -> Arc<Vec<Value>> {
    if separator.is_empty() {
        // Split into individual characters
        s.chars()
            .map(|c| Value::String(Arc::new(c.to_string())))
            .collect::<Vec<_>>()
            .into()
    } else {
        s.split(&*separator)
            .map(|part| Value::String(Arc::new(part.to_string())))
            .collect::<Vec<_>>()
            .into()
    }
}

pub fn substring(
    ftx: &FunctionContext,
    This(s): This<Arc<String>>,
    start: i64,
    end: i64,
) -> Arc<String> {
    if start < 0 || end < 0 {
        ftx.error("start and end indices must be positive");
        return Arc::new(String::new());
    }

    let start = start as usize;
    let end = end as usize;
    let chars: Vec<char> = s.chars().collect();

    if start >= chars.len() {
        return Arc::new(String::new());
    }

    let actual_end = end.min(chars.len());
    if start >= actual_end {
        return Arc::new(String::new());
    }

    let result: String = chars[start..actual_end].iter().collect();
    Arc::new(result)
}

pub fn trim(This(s): This<Arc<String>>) -> Arc<String> {
    Arc::new(s.trim().to_string())
}

pub fn upper_ascii(This(s): This<Arc<String>>) -> Arc<String> {
    let result: String = s
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();
    Arc::new(result)
}

pub fn reverse(This(s): This<Arc<String>>) -> Arc<String> {
    let result: String = s.chars().rev().collect();
    Arc::new(result)
}
