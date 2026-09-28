//! Where Lumen keeps its files.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use directories::ProjectDirs;

/// Resolved locations for configuration, themes and state.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Machine-local and disposable: nothing here is the user's, and all of
    /// it can be fetched or rebuilt again.
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "Lumen")
            .context("could not determine this platform's configuration directory")?;

        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
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

    /// Shortcut sheets, distributed exactly like themes: same folder-drop
    /// model, same absence of any runtime (F3, tier T0).
    pub fn sheets_dir(&self) -> PathBuf {
        self.config_dir.join("sheets")
    }

    /// Script commands (F3, tier T1). Any executable dropped in here with a
    /// `@lumen.title` header becomes a command.
    pub fn scripts_dir(&self) -> PathBuf {
        self.config_dir.join("scripts")
    }

    /// Saved blocks of text (F3). One file rather than a folder: snippets are
    /// edited in bulk, and a file per snippet would make reordering them a
    /// filesystem operation.
    pub fn snippets_file(&self) -> PathBuf {
        self.config_dir.join("snippets.json")
    }

    /// Saved links and web searches (F6). One file, like the snippets.
    pub fn quicklinks_file(&self) -> PathBuf {
        self.config_dir.join("quicklinks.json")
    }

    pub fn theme_file(&self, name: &str) -> PathBuf {
        self.themes_dir().join(format!("{name}.json"))
    }

    /// Installed extensions, one folder each (F4).
    ///
    /// Under `config` rather than `data`: an extension is something the user
    /// installed and may edit, like a theme or a script, not state Lumen keeps
    /// on their behalf.
    pub fn extensions_dir(&self) -> PathBuf {
        self.config_dir.join("extensions")
    }

    /// The clipboard history database (F3).
    pub fn clipboard_file(&self) -> PathBuf {
        self.data_dir.join("clipboard.sqlite3")
    }

    /// Quicklink icons fetched from their sites (F6).
    pub fn favicons_dir(&self) -> PathBuf {
        self.cache_dir.join("favicons")
    }

    pub fn frecency_file(&self) -> PathBuf {
        self.data_dir.join("frecency.json")
    }

    pub fn ensure_exists(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.themes_dir(),
            &self.sheets_dir(),
            &self.scripts_dir(),
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}
