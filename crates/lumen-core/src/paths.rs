//! Where Lumen keeps its files.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use directories::ProjectDirs;

/// Resolved locations for configuration, themes and state.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "Lumen")
            .context("could not determine this platform's configuration directory")?;

        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }

    /// The watched folder. A theme installs by being dropped in here: no code,
    /// no runtime, no review (AD-12).
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn theme_file(&self, name: &str) -> PathBuf {
        self.themes_dir().join(format!("{name}.json"))
    }

    pub fn frecency_file(&self) -> PathBuf {
        self.data_dir.join("frecency.json")
    }

    pub fn ensure_exists(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.data_dir, &self.themes_dir()] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}
