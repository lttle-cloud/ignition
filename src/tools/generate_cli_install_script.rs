use std::{collections::HashMap, os::unix::fs::PermissionsExt};

use anyhow::Result;

const TEMPLATE: &'static str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/tools/templates/cli_install_script.sh.txt"
));

const OUTPUT_PATH: &'static str = concat!(env!("CARGO_MANIFEST_DIR"), "/get/lttle.sh");
const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn render(mut tpl: String, vars: &HashMap<String, String>) -> String {
    for (k, v) in vars {
        let needle = format!("${{{}}}", k);
        tpl = tpl.replace(&needle, v);
    }
    tpl
}

fn main() -> Result<()> {
    let mut vars = HashMap::new();
    vars.insert("VERSION".into(), VERSION.to_string());

    let rendered = render(TEMPLATE.to_string(), &vars);

    let dir = std::path::Path::new(OUTPUT_PATH).parent().unwrap();
    std::fs::create_dir_all(dir)?;

    std::fs::write(OUTPUT_PATH, rendered)?;

    // chmod +x
    std::fs::set_permissions(OUTPUT_PATH, PermissionsExt::from_mode(0o755))?;

    println!("Wrote to {}", OUTPUT_PATH);

    Ok(())
}
