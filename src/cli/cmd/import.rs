use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use serde_json::Map;

use crate::{
    config::Config,
    ui::message::{message_detail, message_info},
};

#[derive(Args)]
pub struct ImportLovableArgs {
    dir: Option<PathBuf>,
}

const LOVABLE_LTTLE_YAML: &str = include_str!("../../../templates/lovable/lttle.yaml");
const LOVABLE_VSCODE_SETTINGS: &str =
    include_str!("../../../templates/lovable/vscode-settings.json");
const LOVABLE_VSCODE_EXTENSIONS: &str =
    include_str!("../../../templates/lovable/vscode-extensions.json");
const LOVABLE_DOCKERIGNORE: &str = include_str!("../../../templates/lovable/dockerignore");
const LOVABLE_FAVICON: &str = include_str!("../../../templates/lovable/favicon.ico");
const LOVABLE_README_SIMPLE: &str = include_str!("../../../templates/lovable/readme-simple.md");
const LOVABLE_README_SUPABASE: &str = include_str!("../../../templates/lovable/readme-supabase.md");

const SUPABASE_CLI_VERSION: &str = "^2.47.2";
const SUPABASE_UPDATE_SCRIPT: &str = "supabase gen types typescript --project-id ${{ supabase_project_id }} --schema public > src/integrations/supabase/types.ts";

pub async fn run_import_lovable(_config: &Config, args: ImportLovableArgs) -> Result<()> {
    let current_dir = std::env::current_dir().unwrap();
    let path = args.dir.unwrap_or(PathBuf::from("."));
    let path = current_dir.join(path);

    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    } else {
        if !path.is_dir() {
            bail!("Target path is not a directory: {}", path.display());
        }

        let is_empty = path.read_dir().unwrap().next().is_none();
        if !is_empty {
            bail!("Target directory is not empty: {}", path.display());
        }
    }
    let path = path.canonicalize()?;

    let project_selector_server =
        lovable_client::selector::ProjectSelectorServer::new(20000u16..=25000u16);

    message_info("Select a lovable project to import");
    message_detail(format!(
        "Open this link in your browser: {}",
        project_selector_server.get_access_url()
    ));

    let selected_project = project_selector_server.run().await?;
    message_info(format!(
        "Selected project: {}",
        selected_project.project_name
    ));

    let files = match lovable_client::client::import_project(
        &selected_project.auth_token,
        &selected_project.project_id,
    )
    .await
    {
        Ok(files) => files,
        Err(e) => {
            eprintln!("Error importing project: {}", e);
            return Err(e);
        }
    };
    message_info(format!(
        "Discovered {} files from project '{}'",
        files.len(),
        selected_project.project_name
    ));

    for file in &files {
        println!("  → {} ({} bytes)", file.name, file.content.len());
        let file_path = path.join(&file.name);
        let dir = file_path.parent().unwrap();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        std::fs::write(file_path, &file.content)?;
    }

    let vscode_settings_path = path.join(".vscode/settings.json");
    if !vscode_settings_path.exists() {
        std::fs::create_dir_all(vscode_settings_path.parent().unwrap())?;
        std::fs::write(vscode_settings_path, LOVABLE_VSCODE_SETTINGS)?;
        eprintln!("  → .vscode/settings.json");
    }
    let vscode_extensions_path = path.join(".vscode/extensions.json");
    if !vscode_extensions_path.exists() {
        std::fs::create_dir_all(vscode_extensions_path.parent().unwrap())?;
        std::fs::write(vscode_extensions_path, LOVABLE_VSCODE_EXTENSIONS)?;
        eprintln!("  → .vscode/extensions.json");
    }
    let lttle_yaml_path = path.join("lttle.yaml");
    if !lttle_yaml_path.exists() {
        let content = LOVABLE_LTTLE_YAML.replace(
            "${{ lovable_project_name }}",
            &selected_project.project_name,
        );
        std::fs::write(lttle_yaml_path, content)?;
        eprintln!("  → lttle.yaml");
    }
    let dockerignore_path = path.join(".dockerignore");
    if !dockerignore_path.exists() {
        std::fs::write(dockerignore_path, LOVABLE_DOCKERIGNORE)?;
        eprintln!("  → .dockerignore");
    }

    std::fs::write(path.join("public/favicon.ico"), LOVABLE_FAVICON)?;
    eprintln!("  → public/favicon.ico");

    let supabase_config = lovable_client::client::get_supabase_config(
        &selected_project.auth_token,
        &selected_project.project_id,
    )
    .await
    .ok();

    let supabase_project_id = supabase_config.and_then(|config| {
        if config.supabase_project_id != "" {
            Some(config.supabase_project_id)
        } else {
            None
        }
    });

    let is_supabase = supabase_project_id.is_some();
    let readme_path = path.join("README.md");
    let content = if is_supabase {
        LOVABLE_README_SUPABASE.to_string()
    } else {
        LOVABLE_README_SIMPLE.to_string()
    };
    let content = content.replace(
        "${{ lovable_project_name }}",
        &selected_project.project_name,
    );
    let mut content = content.replace("${{ lovable_project_id }}", &selected_project.project_id);
    if let Some(supabase_project_id) = supabase_project_id {
        content = content.replace("${{ supabase_project_id }}", &supabase_project_id);

        let package_json_path = path.join("package.json");
        let package_json = std::fs::read_to_string(&package_json_path)?;
        let mut package_json = serde_json::from_str::<serde_json::Value>(&package_json)?;
        if package_json["devDependencies"].is_null() {
            package_json["devDependencies"] = serde_json::Value::Object(Map::new());
        }
        package_json["devDependencies"]["supabase"] =
            serde_json::Value::String(SUPABASE_CLI_VERSION.to_string());

        if package_json["scripts"].is_null() {
            package_json["scripts"] = serde_json::Value::Object(Map::new());
        }
        package_json["scripts"]["update-supabase"] = serde_json::Value::String(
            SUPABASE_UPDATE_SCRIPT
                .to_string()
                .replace("${{ supabase_project_id }}", &supabase_project_id),
        );
        let package_json = serde_json::to_string_pretty(&package_json)?;
        std::fs::write(
            package_json_path,
            serde_json::to_string_pretty(&package_json)?,
        )?;
        eprintln!("  → package.json");
    }

    std::fs::write(readme_path, content)?;
    eprintln!("  → README.md");

    message_info(format!(
        "Imported project {} successfully",
        selected_project.project_name
    ));
    message_info("Next steps:");
    // is the path the CWD?
    if path != current_dir {
        eprintln!(
            "  → Run `cd {}` to go to your project directory",
            path.display()
        );
    }
    eprintln!("  → Run `lttle deploy` to deploy your project");
    eprintln!(
        "  → Run `lttle machine get {}` to see if your machine is ready",
        selected_project.project_name
    );
    eprintln!(
        "  → Run `lttle app get {}` to get the URL of your app (and other info)",
        selected_project.project_name
    );
    eprintln!("  → Run `lttle app ls -a` to list all your apps");
    eprintln!("  → Check out the docs at https://docs.lttle.cloud");
    message_info("🎉 Vibe hard!");

    Ok(())
}
