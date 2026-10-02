//! User configuration.
//!
//! The window's width, its row count and where it sits live here, in the
//! user's own config. The launcher's geometry is the active style's and its
//! colours are the palette's (F15); both are chosen here by name.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::{BackdropMode, Hotkey};
use serde::{Deserialize, Serialize};

use crate::calc::history::CalculatorConfig;
use crate::clipboard::Retention;
use crate::clock::ClockConfig;
use crate::currency::CurrencyConfig;
use crate::dictionary::DictionaryConfig;
use crate::places::store::PlacesConfig;
use crate::providers::files::{self, FileLimits};
use crate::snippets::SnippetsConfig;
use crate::style::Metrics;

/// What `window.width` may be, in logical pixels (F15): narrower and a
/// title and its accessory collide; wider and the eye travels too far
/// between the name and what it is.
pub const WIDTH: std::ops::RangeInclusive<f32> = 560.0..=1000.0;

/// What `window.visible-rows` may be (F15).
pub const ROWS: std::ops::RangeInclusive<usize> = 3..=20;
use crate::timers::TimersConfig;
use crate::translate::TranslateConfig;
use crate::weather::WeatherConfig;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub hotkey: Hotkey,
    /// File name (without `.json`) of the theme -- the palette -- in the
    /// themes folder.
    pub theme: String,
    /// File name (without `.json`) of the style in the styles folder (F15):
    /// the launcher's geometry and variants, chosen apart from its colours.
    pub style: String,
    /// How the root list is arranged (F15): the empty list's sections, and
    /// whether the typed list has a header.
    pub root: crate::root::RootConfig,
    pub backdrop: BackdropMode,
    pub window: WindowConfig,
    pub clipboard: ClipboardConfig,
    pub files: FileSearchConfig,
    /// The browser quicklinks open in when they do not name one (F6), by the
    /// name the system knows it by: `"Zen Browser"`, or just `"zen"`.
    ///
    /// `null` is the system default. A name, not a path, so the setting
    /// survives the browser updating itself into a new folder.
    pub preferred_browser: Option<String>,
    /// Translation (F7): which service, and the saved presets.
    pub translate: TranslateConfig,
    /// Timers (F8): saved presets and the quick durations.
    pub timers: TimersConfig,
    /// Currency (F9): which service asks for rates, and its key.
    pub currency: CurrencyConfig,
    /// Definitions (F10): whether history is kept.
    pub dictionary: DictionaryConfig,
    /// Snippets (F11): whether a paste puts the clipboard back.
    pub snippets: SnippetsConfig,
    /// World Clock (F13): which value leads a row, and how the difference
    /// is written.
    pub clock: ClockConfig,
    /// Weather (F13): units, the report's layout, and the home city.
    pub weather: WeatherConfig,
    /// The home city, shared by World Clock and Weather (F13).
    pub places: PlacesConfig,
    /// The calculator (F14): how many calculations its history keeps.
    pub calculator: CalculatorConfig,
}

/// File search (F3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WindowConfig {
    /// Logical pixels, overriding the style's `window.width` (F15). `null`,
    /// or absent, is the style's. Kept within [`WIDTH`].
    pub width: Option<f32>,
    /// How many rows are visible before the list scrolls, overriding the
    /// style's `list.height` (F15). `null`, or absent, is the style's. Kept
    /// within [`ROWS`].
    ///
    /// A count, not a height: the window grows in whole rows so that a partial
    /// row is never left clipped at the bottom edge.
    pub visible_rows: Option<usize>,
    /// Where the window sits vertically in the work area, 0.0 to 1.0.
    ///
    /// 0.0 is against the top edge, 1.0 against the bottom, 0.5 centred. The
    /// default sits above the optical centre on purpose: 0.5 reads as low,
    /// because the eye weights the top of the screen.
    pub vertical_bias: f32,
    /// Where the window sits horizontally, 0.0 to 1.0.
    ///
    /// 0.0 is against the left edge, 1.0 against the right, and 0.5 -- the
    /// default -- centred. Worth having as a setting rather than a constant:
    /// where a launcher should appear is about where the person using it
    /// habitually looks, and on an ultrawide monitor the centre of the screen
    /// is nowhere near the centre of anybody's attention.
    pub horizontal_bias: f32,
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
            width: None,
            visible_rows: None,
            vertical_bias: 0.22,
            horizontal_bias: 0.5,
            dismiss_on_focus_loss: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            theme: "nocturne".into(),
            style: "lumen".into(),
            root: crate::root::RootConfig::default(),
            backdrop: BackdropMode::default(),
            window: WindowConfig::default(),
            clipboard: ClipboardConfig::default(),
            files: FileSearchConfig::default(),
            preferred_browser: None,
            translate: TranslateConfig::default(),
            timers: TimersConfig::default(),
            currency: CurrencyConfig::default(),
            dictionary: DictionaryConfig::default(),
            snippets: SnippetsConfig::default(),
            clock: ClockConfig::default(),
            weather: WeatherConfig::default(),
            places: PlacesConfig::default(),
            calculator: CalculatorConfig::default(),
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
        if !path.exists() {
            return Self::default();
        }

        match Self::try_load(path) {
            Ok(config) => config,
            Err(error) => {
                log::error!("{error:#}; using defaults");
                Self::default()
            }
        }
    }

    /// Load from disk, reporting a problem instead of papering over it.
    ///
    /// What a live reload uses. At startup a broken file means "run with the
    /// defaults"; while running it means "keep what already works" -- a config
    /// saved halfway through an edit must not snap the launcher back to the
    /// default position, size and theme.
    pub fn try_load(path: &Path) -> Result<Self> {
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&source)
            .with_context(|| format!("{} is not valid configuration", path.display()))
    }

    /// The settings that differ between two configs and only take effect when
    /// the daemon next starts.
    ///
    /// Everything else is applied live. These three are the exceptions because
    /// each is wired into something created once: the hotkey is registered
    /// with the system, the backdrop is applied to the window when it is made,
    /// and the clipboard listener is either started or not. How much history
    /// is kept is not among them (F12): the history applies it at once. Named in the
    /// config's own spelling, so the log can say exactly which edit is waiting.
    pub fn restart_required(&self, newer: &Self) -> Vec<&'static str> {
        let mut pending = Vec::new();
        if self.hotkey != newer.hotkey {
            pending.push("hotkey");
        }
        if self.backdrop != newer.backdrop {
            pending.push("backdrop");
        }
        if self.clipboard.enabled != newer.clipboard.enabled {
            pending.push("clipboard.enabled");
        }
        if self.timers.hotkey != newer.timers.hotkey {
            pending.push("timers.hotkey");
        }
        pending
    }

    /// `newer`, keeping this config's values for everything in
    /// [`Config::restart_required`].
    ///
    /// What a running daemon should hold after a reload: the configuration
    /// actually in effect, not the file. Holding the file's hotkey would make
    /// the next reload compare equal and stop mentioning that a restart is
    /// still pending.
    pub fn merge_live(&self, newer: &Self) -> Self {
        let mut merged = Self {
            hotkey: self.hotkey.clone(),
            backdrop: self.backdrop,
            clipboard: ClipboardConfig {
                enabled: self.clipboard.enabled,
                ..newer.clipboard
            },
            ..newer.clone()
        };
        merged.timers.hotkey = self.timers.hotkey.clone();
        merged
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

    /// The window's width: the config's when it sets one, the style's when
    /// not, within [`WIDTH`] either way.
    pub fn width(&self, metrics: &Metrics) -> f32 {
        let width = self.window.width.unwrap_or(metrics.window_width);
        if width.is_finite() {
            width.clamp(*WIDTH.start(), *WIDTH.end())
        } else {
            metrics.window_width
        }
    }

    /// How tall the list may grow before it scrolls, in logical pixels: the
    /// config's row count in the style's rows when it sets one, the style's
    /// `list.height` when not (F15).
    ///
    /// A height and not a row count, so a section's header does not cost a
    /// whole row: the list still ends on a whole row (I-10), because rows are
    /// accumulated until the next would not fit.
    pub fn list_height(&self, metrics: &Metrics) -> f32 {
        match self.window.visible_rows {
            Some(rows) => rows.clamp(*ROWS.start(), *ROWS.end()) as f32 * metrics.row_height,
            None => metrics.list_height,
        }
    }

    /// How many whole rows fit in [`Config::list_height`].
    pub fn visible_rows(&self, metrics: &Metrics) -> usize {
        ((self.list_height(metrics) / metrics.row_height).floor() as usize).max(1)
    }

    /// Total window height for a given number of visible rows.
    ///
    /// Computed from the active style's metrics rather than configured
    /// directly, which is what makes "zero layout shift" (I-10) achievable:
    /// the window is always a whole number of fixed-height rows plus fixed
    /// chrome.
    pub fn window_height(&self, metrics: &Metrics, rows: usize) -> f32 {
        let rows = rows.min(self.visible_rows(metrics));
        if rows == 0 {
            return metrics.search_height + metrics.footer_height;
        }

        metrics.search_height
            + metrics.list_padding * 2.0
            + rows as f32 * metrics.row_height
            + metrics.footer_height
    }

    /// Total window height while the action panel's popover is open, given the
    /// height the list alone would have and the popover's own.
    ///
    /// The popover floats over the list, so it needs no room of its own when the
    /// list is tall enough to stand it in -- the window then does not change at
    /// all, which is what keeps the list from being re-laid out beneath it. It
    /// grows only when the list is shorter: the search row, the popover with
    /// the inset kept above and below it, and the footer (7.45). The popover is
    /// given an explicit height because the window is sized exactly to its
    /// content and an absolutely positioned element has no way to derive one.
    pub fn panel_height(&self, metrics: &Metrics, list: f32, popover: f32) -> f32 {
        use crate::layout;

        let needed = metrics.search_height
            + layout::action_panel::INSET * 2.0
            + popover
            + metrics.footer_height;
        list.max(needed)
    }

    /// Total window height for the scope picker showing `options` rows.
    ///
    /// A separate calculation from [`Config::window_height`] rather than a
    /// conversion into "equivalent result rows", because a picker row is a
    /// different height from a result row -- one line against a title and a
    /// subtitle -- and rounding one into the other leaves exactly the band of
    /// empty space this exists to remove. The picker's own rows keep their
    /// constants (7.41); the search field and the footer around it are the
    /// style's.
    pub fn picker_height(&self, metrics: &Metrics, options: usize) -> f32 {
        use crate::layout;

        if options == 0 {
            return metrics.search_height + metrics.footer_height;
        }

        metrics.search_height
            + layout::action_panel::PADDING * 2.0
            + options as f32 * layout::action_panel::ROW_HEIGHT
            + metrics.footer_height
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
        assert_eq!(config.window.width, Some(800.0));
    }

    /// The style sizes the window unless the config says otherwise, and the
    /// config is held to its bounds (F15).
    #[test]
    fn the_style_sizes_the_window_unless_the_config_overrides_it() {
        let metrics = Metrics {
            window_width: 764.0,
            list_height: 371.0,
            row_height: 38.0,
            ..Metrics::BASE
        };
        let mut config = Config::default();
        assert_eq!(config.width(&metrics), 764.0);
        assert_eq!(config.list_height(&metrics), 371.0);
        assert_eq!(config.visible_rows(&metrics), 9);

        config.window.width = Some(5000.0);
        config.window.visible_rows = Some(1);
        assert_eq!(config.width(&metrics), 1000.0);
        assert_eq!(config.list_height(&metrics), 3.0 * 38.0);
    }

    /// A config written before the setting existed must keep the launcher
    /// where it has always been rather than jumping to a corner.
    #[test]
    fn the_launcher_stays_centred_for_a_config_that_predates_the_setting() {
        let config: Config = serde_json::from_str(r#"{ "window": { "width": 800.0 } }"#).unwrap();
        assert_eq!(config.window.horizontal_bias, 0.5);
        assert_eq!(config.window.vertical_bias, 0.22);
    }

    #[test]
    fn a_partial_config_keeps_defaults_for_everything_else() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert_eq!(config.theme, "daylight");
        assert_eq!(config.window.visible_rows, None);
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

    /// A half-saved file while the daemon runs must be reported, not quietly
    /// turned into defaults -- that would move the launcher mid-edit.
    #[test]
    fn a_broken_file_is_an_error_to_a_live_reload() {
        let dir = std::env::temp_dir().join(format!("lumen-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{ "window": { "vertical-bias": 0.0, "#).unwrap();

        assert!(Config::try_load(&path).is_err());
        // Startup still falls back rather than refusing to run.
        assert_eq!(Config::load(&path).theme, "nocturne");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn position_size_and_theme_apply_without_a_restart() {
        let before = Config::default();
        let mut after = before.clone();
        after.window.vertical_bias = 0.0;
        after.window.width = Some(900.0);
        after.window.visible_rows = Some(5);
        after.theme = "daylight".into();
        after.files.enabled = false;

        assert!(before.restart_required(&after).is_empty());
    }

    #[test]
    fn a_new_hotkey_is_reported_as_waiting_for_a_restart() {
        let before = Config::default();
        let mut after = before.clone();
        after.hotkey.key = "K".into();
        after.clipboard.enabled = false;

        assert_eq!(
            before.restart_required(&after),
            vec!["hotkey", "clipboard.enabled"]
        );
    }

    /// How much history is kept applies at once (F12); switching it off
    /// still waits, since the listener is started or not.
    #[test]
    fn the_clipboard_limit_applies_live_and_the_switch_waits() {
        let before = Config::default();
        let mut after = before.clone();
        after.clipboard.retention.max_entries = 50;
        assert!(before.restart_required(&after).is_empty());
        let applied = before.merge_live(&after);
        assert_eq!(applied.clipboard.retention.max_entries, 50);

        after.clipboard.enabled = false;
        let applied = before.merge_live(&after);
        assert!(applied.clipboard.enabled);
    }

    /// The running daemon keeps the hotkey it registered, so the next reload
    /// still reports the restart as pending.
    #[test]
    fn a_live_merge_takes_the_position_but_keeps_the_hotkey() {
        let before = Config::default();
        let mut after = before.clone();
        after.hotkey.key = "K".into();
        after.window.vertical_bias = 0.0;

        let applied = before.merge_live(&after);
        assert_eq!(applied.hotkey, before.hotkey);
        assert_eq!(applied.window.vertical_bias, 0.0);
        assert_eq!(applied.restart_required(&after), vec!["hotkey"]);
    }

    /// A config written before translation existed gets the keyless service
    /// and the implied preset, so the command works without an edit.
    #[test]
    fn translation_works_for_a_config_that_predates_it() {
        let config: Config = serde_json::from_str(r#"{ "theme": "daylight" }"#).unwrap();
        assert_eq!(config.translate.service, crate::translate::Service::Google);
        assert_eq!(config.translate.presets().len(), 1);
    }

    /// One bad preset must not take the hotkey and the theme down with it.
    #[test]
    fn an_unknown_language_in_a_preset_does_not_fail_the_file() {
        let config: Config = serde_json::from_str(
            r#"{ "theme": "daylight",
                 "translate": { "presets": [ { "name": "X", "from": "xx", "to": "yy" } ] } }"#,
        )
        .unwrap();
        assert_eq!(config.theme, "daylight");
    }

    #[test]
    fn round_trips_through_json() {
        let source = serde_json::to_string(&Config::default()).unwrap();
        let parsed: Config = serde_json::from_str(&source).unwrap();
        assert_eq!(parsed.hotkey, Config::default().hotkey);
        assert_eq!(parsed.backdrop, BackdropMode::SystemAcrylic);
    }

    /// With an ordinary list the popover fits over it, so opening it must not
    /// change the window at all: that is the whole point of floating.
    #[test]
    fn a_popover_that_fits_over_the_list_leaves_the_window_as_it_was() {
        use crate::item::ActionGroup;

        let config = Config::default();
        let list = config.window_height(&Metrics::BASE, 8);
        let popover = crate::layout::action_panel::height(&[ActionGroup::Primary; 4]);

        assert_eq!(config.panel_height(&Metrics::BASE, list, popover), list);
    }

    /// A short list gives it nowhere to stand, so the window grows, by exactly
    /// the room the popover needs: the search row, the popover with its insets
    /// above and below, and the footer.
    #[test]
    fn a_popover_taller_than_the_list_grows_the_window_to_fit_it() {
        use crate::item::ActionGroup;

        let config = Config::default();
        let list = config.window_height(&Metrics::BASE, 1);
        let popover = crate::layout::action_panel::height(&[ActionGroup::Primary; 6]);

        let height = config.panel_height(&Metrics::BASE, list, popover);

        assert!(height > list);
        assert_eq!(
            height,
            Metrics::BASE.search_height
                + crate::layout::action_panel::INSET * 2.0
                + popover
                + Metrics::BASE.footer_height
        );
    }

    #[test]
    fn the_window_is_never_shorter_than_the_list_while_the_popover_is_open() {
        let config = Config::default();

        for rows in 0..=8 {
            let list = config.window_height(&Metrics::BASE, rows);
            assert!(config.panel_height(&Metrics::BASE, list, 0.0) >= list);
        }
    }

    /// The picker takes the list's place, so its window is its own rows rather
    /// than the list's.
    #[test]
    fn the_picker_is_shorter_than_the_list_it_replaces() {
        let config = Config::default();

        assert!(config.picker_height(&Metrics::BASE, 3) < config.window_height(&Metrics::BASE, 3));
        assert!(config.picker_height(&Metrics::BASE, 1) < config.window_height(&Metrics::BASE, 1));
    }

    #[test]
    fn the_picker_grows_in_whole_rows() {
        let config = Config::default();
        let step = crate::layout::action_panel::ROW_HEIGHT;

        assert_eq!(
            config.picker_height(&Metrics::BASE, 3) - config.picker_height(&Metrics::BASE, 2),
            step
        );
    }

    /// Unlike the result list, the picker is not capped at `visible-rows`: a
    /// scope with ten options has to show all ten, because there is nothing else
    /// on screen to scroll.
    #[test]
    fn the_picker_is_not_capped_at_the_visible_row_count() {
        let config = Config::default();
        assert!(config.picker_height(&Metrics::BASE, 12) > config.picker_height(&Metrics::BASE, 8));
    }

    #[test]
    fn the_window_grows_in_whole_rows_and_stops_at_the_cap() {
        let config = Config::default();

        let empty = config.window_height(&Metrics::BASE, 0);
        let one = config.window_height(&Metrics::BASE, 1);
        let two = config.window_height(&Metrics::BASE, 2);

        assert!(one > empty);
        assert_eq!(two - one, crate::layout::ROW_HEIGHT);
        // Past the visible cap the window stops growing and the list scrolls.
        assert_eq!(
            config.window_height(&Metrics::BASE, 50),
            config.window_height(&Metrics::BASE, 8)
        );
    }
}
