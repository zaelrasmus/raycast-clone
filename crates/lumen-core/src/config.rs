//! User configuration.
//!
//! Geometry lives here, in the user's own config, and explicitly *not* in the
//! theme (AD-13, I-11). A theme may change colour and type family; only the
//! person running the launcher may change how big it is.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::{BackdropMode, Hotkey};
use serde::{Deserialize, Serialize};

use crate::clipboard::Retention;
use crate::providers::files::{self, FileLimits};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub hotkey: Hotkey,
    /// File name (without `.json`) of the theme in the themes folder.
    pub theme: String,
    pub backdrop: BackdropMode,
    pub window: WindowConfig,
    pub clipboard: ClipboardConfig,
    pub files: FileSearchConfig,
}

/// File search (F3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileSearchConfig {
    pub enabled: bool,
    /// Folders to walk. Empty means the user's own directories --
    /// [`files::default_roots`].
    ///
    /// Empty rather than the resolved list, so that the defaults follow the
    /// machine: a config file copied to another computer, or a Documents
    /// folder redirected to OneDrive after Lumen was first run, keeps working
    /// instead of pointing at a path that no longer exists.
    pub roots: Vec<std::path::PathBuf>,
    pub limits: FileLimits,
}

impl Default for FileSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: Vec::new(),
            limits: FileLimits::default(),
        }
    }
}

impl FileSearchConfig {
    /// The folders to actually walk.
    pub fn roots(&self) -> Vec<std::path::PathBuf> {
        if self.roots.is_empty() {
            files::default_roots()
        } else {
            self.roots.clone()
        }
    }
}

/// Clipboard history (F3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ClipboardConfig {
    /// Whether anything is recorded at all.
    ///
    /// A real switch rather than a documented workaround: "make the folder
    /// read-only" is not an answer to somebody who would rather Lumen did not
    /// watch their clipboard. Off means the listener is never started, so the
    /// question of what reaches the database does not arise.
    pub enabled: bool,
    pub retention: Retention,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention: Retention::default(),
        }
    }
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
    /// Whether clicking away closes the launcher.
    ///
    /// On by default, because a launcher that stays on top of whatever you just
    /// clicked is one you have to dismiss before you can use the thing you
    /// chose. Configurable because the opposite preference is legitimate:
    /// people who drive it entirely from the keyboard sometimes want it to sit
    /// there while they read something behind it.
    pub dismiss_on_focus_loss: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 720.0,
            visible_rows: 8,
            vertical_bias: 0.22,
            dismiss_on_focus_loss: true,
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
            clipboard: ClipboardConfig::default(),
            files: FileSearchConfig::default(),
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

    /// Total window height for the action panel showing `actions` rows.
    ///
    /// A separate calculation from [`Config::window_height`] rather than a
    /// conversion into "equivalent result rows", because an action row is a
    /// different height from a result row -- one line against a title and a
    /// subtitle -- and rounding one into the other leaves exactly the band of
    /// empty space this exists to remove.
    pub fn panel_height(&self, actions: usize) -> f32 {
        use crate::layout;

        if actions == 0 {
            return layout::SEARCH_HEIGHT + layout::FOOTER_HEIGHT;
        }

        layout::SEARCH_HEIGHT
            + layout::action_panel::PADDING * 2.0
            + actions as f32 * layout::action_panel::ROW_HEIGHT
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

    /// A config written before the setting existed must keep the behaviour
    /// people expect rather than silently turning it off.
    #[test]
    fn dismiss_on_focus_loss_is_on_for_a_config_that_predates_it() {
        let config: Config = serde_json::from_str(r#"{ "window": { "width": 800.0 } }"#).unwrap();
        assert!(config.window.dismiss_on_focus_loss);
        assert_eq!(config.window.width, 800.0);
    }

    #[test]
    fn a_partial_config_keeps_defaults_for_everything_else() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert_eq!(config.theme, "daylight");
        assert_eq!(config.window.visible_rows, 8);
    }

    /// The switch has to survive a config written before it existed, or
    /// upgrading silently turns the clipboard watcher off.
    #[test]
    fn clipboard_history_is_on_for_a_config_that_predates_the_setting() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert!(config.clipboard.enabled);
        assert_eq!(config.clipboard.retention.max_entries, 500);
    }

    #[test]
    fn clipboard_retention_can_be_changed_without_restating_the_rest() {
        let config: Config =
            serde_json::from_str(r#"{ "clipboard": { "retention": { "max-age-days": 7 } } }"#)
                .unwrap();
        assert_eq!(config.clipboard.retention.max_age_days, 7);
        assert_eq!(config.clipboard.retention.max_entries, 500);
        assert!(config.clipboard.enabled);
    }

    #[test]
    fn file_search_is_on_for_a_config_that_predates_the_setting() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert!(config.files.enabled);
        assert!(config.files.roots.is_empty());
        assert_eq!(config.files.limits.max_depth, 6);
    }

    /// Naming roots must not also mean restating the limits.
    #[test]
    fn file_roots_can_be_named_without_restating_the_limits() {
        let config: Config =
            serde_json::from_str(r#"{ "files": { "roots": ["D:/work"] } }"#).unwrap();
        assert_eq!(
            config.files.roots(),
            vec![std::path::PathBuf::from("D:/work")]
        );
        assert_eq!(config.files.limits.max_entries, 5_000);
    }

    #[test]
    fn round_trips_through_json() {
        let source = serde_json::to_string(&Config::default()).unwrap();
        let parsed: Config = serde_json::from_str(&source).unwrap();
        assert_eq!(parsed.hotkey, Config::default().hotkey);
        assert_eq!(parsed.backdrop, BackdropMode::SystemAcrylic);
    }

    /// The panel is shorter than a list of the same length, which is the whole
    /// reason it is measured separately.
    #[test]
    fn the_action_panel_is_shorter_than_the_list_it_replaces() {
        let config = Config::default();

        assert!(config.panel_height(3) < config.window_height(3));
        assert!(config.panel_height(1) < config.window_height(1));
    }

    #[test]
    fn the_panel_grows_in_whole_action_rows() {
        let config = Config::default();
        let step = crate::layout::action_panel::ROW_HEIGHT;

        assert_eq!(config.panel_height(3) - config.panel_height(2), step);
    }

    /// Unlike the result list, the panel is not capped at `visible-rows`: an
    /// item with ten actions has to show all ten, because there is nothing else
    /// on screen to scroll.
    #[test]
    fn the_panel_is_not_capped_at_the_visible_row_count() {
        let config = Config::default();
        assert!(config.panel_height(12) > config.panel_height(8));
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
