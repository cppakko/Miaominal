use anyhow::{Result, anyhow};
use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "akko";
const APPLICATION: &str = "miaominal";

pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or_else(|| anyhow!("failed to locate user config directory"))
}

pub fn config_file(file_name: &str) -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join(file_name))
}
