mod resource;

use util::encoding::{json, schemars::schema_for};

fn main() {
    let schema = schema_for!(resource::Resource);
    let schema_text = json::to_string_pretty(&schema).expect("failed to serialize schema");

    let manifest_root = std::env::var("CARGO_MANIFEST_DIR").expect("missing MANIFEST_DIR env var");
    let output_path = std::path::Path::new(&manifest_root).join("schema.json");

    std::fs::write(&output_path, schema_text).expect("failed to write schema");
}
