//! User configuration.
//!
//! Geometry lives here, in the user's own config, and explicitly *not* in the
//! theme (AD-13, I-11). A theme may change colour and type family; only the
//! person running the launcher may change how big it is.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::{BackdropMode, Hotkey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub hotkey: Hotkey,
    /// File name (without `.json`) of the theme in the themes folder.
    pub theme: String,
    pub backdrop: BackdropMode,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WindowConfig {
    /// Logical pixels. 720 is wide enough for a long application name plus its
    /// accessories without the list becoming a horizon to scan.
    pub width: f32,
    /// How many rows are visible before the list scrolls.
    ///
    /// A count, not a height: the window grows in whole rows so that a partial
    /// row is never left clipped at the bottom edge.
    pub visible_rows: usize,
    /// Where the window sits vertically in the work area, 0.0 to 1.0.
    ///
    /// Above the optical centre on purpose: 0.5 reads as low, because the eye
    /// weights the top of the screen.
    pub vertical_bias: f32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 720.0,
            visible_rows: 8,
            vertical_bias: 0.22,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            theme: "nocturne".into(),
            backdrop: BackdropMode::default(),
            window: WindowConfig::default(),
        }
    }
}

impl Config {
    /// Load from disk, falling back to defaults.
    ///
    /// A malformed config is reported and then ignored rather than fatal. The
    /// launcher is summoned by a keystroke with no terminal attached: a process
    /// that refuses to start because of a stray comma is a process that appears
    /// to be broken with no explanation anywhere the user will see.
    pub fn load(path: &Path) -> Self {
        let Ok(source) = std::fs::read_to_string(path) else {
            return Self::default();
        };

        match serde_json::from_str(&source) {
            Ok(config) => config,
            Err(error) => {
                log::error!(
                    "{} is not valid configuration ({error}); using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write the defaults out so there is something to edit.
    ///
    /// Only ever called when no file exists: this must never overwrite a config
    /// the user has written.
    pub fn write_default(path: &Path) -> Result<()> {
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let source = serde_json::to_string_pretty(&Self::default())?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }

    /// Total window height for a given number of visible rows.
    ///
    /// Computed from the layout metrics rather than configured directly, which
    /// is what makes "zero layout shift" (I-10) achievable: the window is always
    /// a whole number of fixed-height rows plus fixed chrome.
    pub fn window_height(&self, rows: usize) -> f32 {
        use crate::layout;

        let rows = rows.min(self.window.visible_rows);
        if rows == 0 {
            return layout::SEARCH_HEIGHT + layout::FOOTER_HEIGHT;
        }

        layout::SEARCH_HEIGHT
            + layout::LIST_PADDING * 2.0
            + rows as f32 * layout::ROW_HEIGHT
            + layout::FOOTER_HEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_config_yields_defaults() {
        let config = Config::load(Path::new("no/such/file.json"));
        assert_eq!(config.theme, "nocturne");
        assert_eq!(config.hotkey.to_string(), "Alt+Space");
    }

    #[test]
    fn a_partial_config_keeps_defaults_for_everything_else() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert_eq!(config.theme, "daylight");
        assert_eq!(config.window.visible_rows, 8);
    }

    #[test]
    fn round_trips_through_json() {
        let source = serde_json::to_string(&Config::default()).unwrap();
        let parsed: Config = serde_json::from_str(&source).unwrap();
        assert_eq!(parsed.hotkey, Config::default().hotkey);
        assert_eq!(parsed.backdrop, BackdropMode::SystemAcrylic);
    }

    #[test]
    fn the_window_grows_in_whole_rows_and_stops_at_the_cap() {
        let config = Config::default();

        let empty = config.window_height(0);
        let one = config.window_height(1);
        let two = config.window_height(2);

        assert!(one > empty);
        assert_eq!(two - one, crate::layout::ROW_HEIGHT);
        // Past the visible cap the window stops growing and the list scrolls.
        assert_eq!(config.window_height(50), config.window_height(8));
    }
}
