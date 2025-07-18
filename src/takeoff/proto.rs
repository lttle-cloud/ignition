use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TakeoffInitArgs {
    pub envs: HashMap<String, String>,
}

impl TakeoffInitArgs {
    pub fn encode(&self) -> Result<String> {
        let bytes = serde_json::to_string(&self)?;
        let data = hex::encode(bytes);
        Ok(data)
    }

    pub fn decode(data: &str) -> Result<Self> {
        let bytes = hex::decode(data)?;
        let args = serde_json::from_slice(&bytes)?;
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let args = TakeoffInitArgs {
            envs: HashMap::from([("TEST".to_string(), "test".to_string())]),
        };
        let encoded = args.encode().unwrap();
        let decoded = TakeoffInitArgs::decode(&encoded).unwrap();
        assert_eq!(args, decoded);
    }
}
