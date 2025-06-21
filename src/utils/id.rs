use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn short_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let mut hasher = DefaultHasher::new();
    now.hash(&mut hasher);
    let hash = hasher.finish();

    base62::encode(hash)[..8].to_string()
}

pub fn short_id_with_prefix(prefix: &str) -> String {
    format!("{}-{}", prefix, short_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_id() {
        let id = short_id_with_prefix("test");
        assert_eq!(id.len(), 13);
    }
}
