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

/// A folder to keep everything in instead of the system's (F15): its
/// `config`, `data` and `cache`. For test runs, which must never read or
/// write the files of the Lumen a person is using. A daemon started with it
/// listens on its own socket too, so it runs beside the installed one.
pub const HOME_VAR: &str = "LUMEN_HOME";

/// The folder [`HOME_VAR`] names, when it names one.
pub fn home_override() -> Option<PathBuf> {
    std::env::var_os(HOME_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

impl Paths {
    pub fn discover() -> Result<Self> {
        if let Some(home) = home_override() {
            return Ok(Self {
                config_dir: home.join("config"),
                data_dir: home.join("data"),
                cache_dir: home.join("cache"),
            });
        }
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

    /// Font files the user adds (F15): every `.ttf`, `.otf` and `.ttc` here
    /// is loaded at startup, and a palette can name its family.
    pub fn fonts_dir(&self) -> PathBuf {
        self.config_dir.join("fonts")
    }

    /// Styles: the launcher's geometry and variants (F15). Installed like
    /// themes, by dropping a file in, and chosen apart from them.
    pub fn styles_dir(&self) -> PathBuf {
        self.config_dir.join("styles")
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

    /// Window commands the user made (F17, 7.59). Configuration, like the
    /// quicklinks: written by the editor and by hand.
    pub fn window_commands_file(&self) -> PathBuf {
        self.config_dir.join("window-commands.json")
    }

    /// Window layouts the user made (F17, 7.60), beside the window commands.
    pub fn window_layouts_file(&self) -> PathBuf {
        self.config_dir.join("window-layouts.json")
    }

    /// Global hotkeys and aliases for any command, by item id (F17).
    /// Configuration, beside `config.json`: people edit it by hand.
    pub fn shortcuts_file(&self) -> PathBuf {
        self.config_dir.join("shortcuts.json")
    }

    pub fn theme_file(&self, name: &str) -> PathBuf {
        self.themes_dir().join(format!("{name}.json"))
    }

    pub fn style_file(&self, name: &str) -> PathBuf {
        self.styles_dir().join(format!("{name}.json"))
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

    /// The file index's entries (F16). In the cache: it is walked again from
    /// the disk whenever it is lost, and has no business roaming with the
    /// user to another machine.
    pub fn file_index_file(&self) -> PathBuf {
        self.cache_dir.join("files.db")
    }

    pub fn frecency_file(&self) -> PathBuf {
        self.data_dir.join("frecency.json")
    }

    /// Fingerprints of the themes and styles Lumen wrote, so an unedited
    /// copy can follow the release (F15).
    pub fn shipped_file(&self) -> PathBuf {
        self.data_dir.join("shipped.json")
    }

    /// The favourites, in the user's order (F15). State, like frecency: it
    /// is changed from the launcher, not edited as configuration.
    pub fn favorites_file(&self) -> PathBuf {
        self.data_dir.join("favorites.json")
    }

    /// The timers running or paused (F8). State, not configuration: the
    /// presets are in `config.json`.
    pub fn timers_file(&self) -> PathBuf {
        self.data_dir.join("timers.json")
    }

    /// The last table of exchange rates and the currency chosen (F9).
    pub fn currency_file(&self) -> PathBuf {
        self.data_dir.join("currency.json")
    }

    /// The calculations used (F14).
    pub fn calculator_history_file(&self) -> PathBuf {
        self.data_dir.join("calculator-history.json")
    }

    /// Favourite words and the words looked at (F10).
    pub fn dictionary_file(&self) -> PathBuf {
        self.data_dir.join("dictionary.json")
    }

    /// The emoji pinned and how often each was used (F18).
    pub fn emoji_file(&self) -> PathBuf {
        self.data_dir.join("emoji.json")
    }

    /// *My Places*, shared by World Clock and Weather (F13).
    pub fn places_file(&self) -> PathBuf {
        self.data_dir.join("places.json")
    }

    /// The forecasts fetched last, by place (F13).
    pub fn weather_file(&self) -> PathBuf {
        self.data_dir.join("weather.json")
    }

    pub fn ensure_exists(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.themes_dir(),
            &self.styles_dir(),
            &self.fonts_dir(),
            &self.sheets_dir(),
            &self.scripts_dir(),
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}
