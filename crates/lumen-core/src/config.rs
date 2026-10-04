//! User configuration.
//!
//! The window's width, its row count and where it sits live here, in the
//! user's own config. The launcher's geometry is the active style's and its
//! colours are the palette's (F15); both are chosen here by name.

use std::path::{Component, Path, PathBuf, Prefix};

use anyhow::{Context as _, Result};
use lumen_platform::{BackdropMode, Hotkey};
use serde::{Deserialize, Serialize};

use crate::calc::history::CalculatorConfig;
use crate::clipboard::Retention;
use crate::clock::ClockConfig;
use crate::currency::CurrencyConfig;
use crate::dictionary::DictionaryConfig;
use crate::file_index::{CEILING, IndexDetail, Kinds, WalkRules};
use crate::places::store::PlacesConfig;
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

/// File search (F3, F16): where the file index looks, what it leaves out,
/// and what root search shows of it. `docs/FILE-SEARCH.md`, "Configuration".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "FileSearchFile", rename_all = "kebab-case")]
pub struct FileSearchConfig {
    pub enabled: bool,
    /// What root search shows of the index.
    pub in_root: InRoot,
    /// Where to look, in order. `~` is the home folder, and is written as
    /// `~` rather than resolved, so a configuration copied to another
    /// machine, or a profile redirected to OneDrive, keeps working. Empty
    /// means nothing is indexed: home is a scope like any other.
    pub scopes: Vec<PathBuf>,
    /// Folders inside the scopes never descended into.
    pub exclusions: Vec<PathBuf>,
    /// Gitignore-style patterns applied in every scope: `*.log`.
    pub ignore_patterns: Vec<String>,
    /// `.gitignore`, `.ignore` and `.lumenignore`.
    pub use_ignore_files: bool,
    /// A leading dot, or the Windows hidden attribute.
    pub include_hidden: bool,
    /// Names from the default skip list ([`crate::file_index::default_skips`])
    /// turned back on, in any case. Only what was turned off is stored, so a
    /// new default reaches everyone; a name not on the list is ignored.
    pub default_skips_off: Vec<String>,
    /// The drives answered *Not now* in *Search Files* (slice 5), as
    /// written: `E:/`. Not offered again; any can still be added from
    /// settings.
    pub declined_drives: Vec<PathBuf>,
    /// The family a text file's preview is drawn in, in *Search Files*.
    /// Which monospace family is the palette's (`font_mono`); whether to use
    /// it is this preference.
    pub preview_font: PreviewFont,
    /// How much the index keeps on disk beside its names, and so what it
    /// finds without a scan. Not a rule of the walk: changing it rebuilds
    /// the word tables from the index, and walks nothing again.
    pub index_detail: IndexDetail,
}

/// What a text file's preview is drawn in (F16).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewFont {
    /// The palette's monospace family, lines clipped rather than wrapped:
    /// a preview is the file as written, columns and indentation included.
    #[default]
    Monospace,
    /// The palette's interface family, lines wrapped.
    Interface,
}

impl PreviewFont {
    /// The value as written in `config.json`; anything else is the default,
    /// with one log line a read, never an error: a configuration written
    /// for a later Lumen must still load on this one.
    fn read(value: Option<&serde_json::Value>) -> Self {
        match value.and_then(serde_json::Value::as_str) {
            None if value.is_none() => Self::default(),
            Some("monospace") => Self::Monospace,
            Some("interface") => Self::Interface,
            _ => {
                log::warn!(
                    "files.preview-font: ignoring {}, expected \"monospace\" or \"interface\"",
                    value.map(ToString::to_string).unwrap_or_default()
                );
                Self::default()
            }
        }
    }
}

/// What root search shows of the file index (F16). Off still indexes, for
/// the *Search Files* view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InRoot {
    Off,
    Folders,
    /// The default, because F3 showed files in root and removing them would
    /// be a regression.
    #[default]
    FoldersAndFiles,
}

impl Default for FileSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            in_root: InRoot::default(),
            scopes: vec![PathBuf::from("~")],
            exclusions: Vec::new(),
            ignore_patterns: Vec::new(),
            use_ignore_files: true,
            include_hidden: false,
            default_skips_off: Vec::new(),
            declined_drives: Vec::new(),
            preview_font: PreviewFont::default(),
            index_detail: IndexDetail::default(),
        }
    }
}

/// `files` as a file may hold it: today's keys, and F3's.
///
/// Read through this rather than straight into [`FileSearchConfig`] so a
/// configuration written for F3 still loads (FILE-SEARCH.md): `roots`, if it
/// named folders, replaced the defaults, and becomes the scopes; `limits` no
/// longer means anything, and is read and ignored rather than an error.
#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct FileSearchFile {
    enabled: bool,
    in_root: InRoot,
    scopes: Option<Vec<PathBuf>>,
    exclusions: Vec<PathBuf>,
    ignore_patterns: Vec<String>,
    use_ignore_files: bool,
    include_hidden: bool,
    default_skips_off: Vec<String>,
    declined_drives: Vec<PathBuf>,
    roots: Vec<PathBuf>,
    limits: Option<serde::de::IgnoredAny>,
    /// Read as any value, so one this build does not know is the default
    /// rather than a configuration that fails to load.
    preview_font: Option<serde_json::Value>,
    /// Read as any value, as `preview-font` is.
    index_detail: Option<serde_json::Value>,
}

impl Default for FileSearchFile {
    fn default() -> Self {
        let defaults = FileSearchConfig::default();
        Self {
            enabled: defaults.enabled,
            in_root: defaults.in_root,
            scopes: None,
            exclusions: defaults.exclusions,
            ignore_patterns: defaults.ignore_patterns,
            use_ignore_files: defaults.use_ignore_files,
            include_hidden: defaults.include_hidden,
            default_skips_off: defaults.default_skips_off,
            declined_drives: defaults.declined_drives,
            roots: Vec::new(),
            limits: None,
            preview_font: None,
            index_detail: None,
        }
    }
}

impl From<FileSearchFile> for FileSearchConfig {
    fn from(file: FileSearchFile) -> Self {
        if file.limits.is_some() {
            // Once a run: the file is read again on every reload.
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                log::info!(
                    "files.limits is no longer used: the file index has its own ceiling (F16)"
                );
            });
        }
        let unknown = file
            .default_skips_off
            .iter()
            .filter(|name| !is_default_skip(name))
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            // One line a read, naming them all.
            log::warn!(
                "files.default-skips-off: ignoring {unknown:?}, not on the default skip list"
            );
        }
        let scopes = match file.scopes {
            Some(scopes) => scopes,
            None if !file.roots.is_empty() => file.roots,
            None => FileSearchConfig::default().scopes,
        };
        Self {
            enabled: file.enabled,
            in_root: file.in_root,
            scopes,
            exclusions: file.exclusions,
            ignore_patterns: file.ignore_patterns,
            use_ignore_files: file.use_ignore_files,
            include_hidden: file.include_hidden,
            default_skips_off: file.default_skips_off,
            declined_drives: file.declined_drives,
            preview_font: PreviewFont::read(file.preview_font.as_ref()),
            index_detail: read_index_detail(file.index_detail.as_ref()),
        }
    }
}

/// `files.index-detail` as written; anything else is the default, with one
/// log line a read, as [`PreviewFont::read`] does.
fn read_index_detail(value: Option<&serde_json::Value>) -> IndexDetail {
    let Some(value) = value else {
        return IndexDetail::default();
    };
    value
        .as_str()
        .and_then(IndexDetail::parse)
        .unwrap_or_else(|| {
            log::warn!(
                "files.index-detail: ignoring {value}, expected \"complete\", \"balanced\" or \"compact\""
            );
            IndexDetail::default()
        })
}

fn is_default_skip(name: &str) -> bool {
    crate::file_index::default_skips()
        .iter()
        .any(|skip| name.eq_ignore_ascii_case(skip))
}

impl FileSearchConfig {
    /// What root search asks the index for; `None` when it asks nothing.
    pub fn root_kinds(&self) -> Option<Kinds> {
        match (self.enabled, self.in_root) {
            (false, _) | (_, InRoot::Off) => None,
            (true, InRoot::Folders) => Some(Kinds::Folders),
            (true, InRoot::FoldersAndFiles) => Some(Kinds::FoldersAndFiles),
        }
    }

    /// The rules the index is walked with: every path resolved, against the
    /// home folder and then the disk.
    ///
    /// As the disk has it -- [`std::fs::canonicalize`] -- because the walk
    /// compares exclusions with the paths it finds by their components, and
    /// those are spelled as the scope is: `~/videos` must exclude what the
    /// walk of `~` finds as `Videos`. A path that does not exist is kept as
    /// written; a scope that does not exist is simply empty.
    pub fn walk_rules(&self) -> WalkRules {
        let home = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        let resolve = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|path| resolve(path, home.as_deref()))
                .collect()
        };
        WalkRules {
            scopes: resolve(&self.scopes),
            exclusions: resolve(&self.exclusions),
            ignore_patterns: self.ignore_patterns.clone(),
            use_ignore_files: self.use_ignore_files,
            include_hidden: self.include_hidden,
            skips_off: self.skips_off(),
            ceiling: CEILING,
        }
    }

    /// The default skips turned back on, as the walk takes them: on the
    /// list, lower-cased, sorted and once each, so neither the case they
    /// were written in nor their order walks the scopes again.
    fn skips_off(&self) -> Vec<String> {
        let mut off = self
            .default_skips_off
            .iter()
            .filter(|name| is_default_skip(name))
            .map(|name| name.to_lowercase())
            .collect::<Vec<_>>();
        off.sort();
        off.dedup();
        off
    }
}

/// `path` with `~` and `~/...` resolved against `home`, then as the disk
/// spells it, when it exists.
fn resolve(path: &Path, home: Option<&Path>) -> PathBuf {
    let written = path.to_string_lossy();
    let expanded = match (home, written.as_ref()) {
        (Some(home), "~") => home.to_path_buf(),
        (Some(home), _) => match written
            .strip_prefix("~/")
            .or_else(|| written.strip_prefix("~\\"))
        {
            Some(rest) => home.join(rest),
            None => path.to_path_buf(),
        },
        (None, _) => path.to_path_buf(),
    };
    match std::fs::canonicalize(&expanded) {
        Ok(real) => plain(&real),
        Err(_) => expanded,
    }
}

/// `path` without the verbatim prefix `canonicalize` gives it on Windows:
/// `\\?\C:\Users` is `C:\Users`, and `\\?\UNC\server\share` is
/// `\\server\share`.
///
/// By its components, not by `cfg` (I-1): elsewhere a path has no prefix,
/// and this returns it as it is. Every other path the index holds -- home,
/// subtitles, frecency's ids -- is in the plain form, so the scopes must be.
fn plain(path: &Path) -> PathBuf {
    let mut components = path.components();
    let base = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(letter) => format!("{}:", letter as char),
            Prefix::VerbatimUNC(server, share) => format!(
                "\\\\{}\\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            ),
            _ => return path.to_path_buf(),
        },
        _ => return path.to_path_buf(),
    };
    let mut plain = PathBuf::from(base);
    for component in components {
        plain.push(component.as_os_str());
    }
    plain
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

    /// How tall a full root list is: [`Config::visible_rows`] whole rows.
    /// Where the window's top edge is placed from, and how tall a view is
    /// whose height does not follow its rows -- a list beside its detail pane,
    /// a detail view -- so a style's height reaches views too (F15).
    pub fn full_list_height(&self, metrics: &Metrics) -> f32 {
        self.visible_rows(metrics) as f32 * metrics.row_height
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
    /// the style's vertical offset kept above and below it, and the footer
    /// (7.45). Both sides whatever the anchor, which is what any anchor needs
    /// to stand the popover clear of its edge (`ActionsAnchor::origin`, 7.47).
    /// The popover is given an explicit height because the window is sized
    /// exactly to its content and an absolutely positioned element has no way
    /// to derive one.
    pub fn panel_height(&self, metrics: &Metrics, list: f32, popover: f32) -> f32 {
        let needed = metrics.search_height
            + metrics.actions_offset_y * 2.0
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
        assert_eq!(config.full_list_height(&metrics), 9.0 * 38.0);

        config.window.width = Some(5000.0);
        config.window.visible_rows = Some(1);
        assert_eq!(config.width(&metrics), 1000.0);
        assert_eq!(config.list_height(&metrics), 3.0 * 38.0);
        assert_eq!(config.full_list_height(&metrics), 3.0 * 38.0);
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

    fn files(source: &str) -> FileSearchConfig {
        serde_json::from_str::<Config>(source).unwrap().files
    }

    #[test]
    fn file_search_is_on_for_a_config_that_predates_the_setting() {
        let files = files(r#"{ "theme": "daylight" }"#);
        assert_eq!(files, FileSearchConfig::default());
        assert!(files.enabled);
        assert_eq!(files.in_root, InRoot::FoldersAndFiles);
        assert_eq!(files.scopes, [PathBuf::from("~")]);
        assert!(files.use_ignore_files);
        assert!(!files.include_hidden);
    }

    #[test]
    fn every_file_search_key_is_read_in_kebab_case() {
        let files = files(
            r#"{ "files": {
                "enabled": false,
                "in-root": "folders",
                "scopes": ["~", "D:/Dev"],
                "exclusions": ["~/Videos"],
                "ignore-patterns": ["*.log"],
                "use-ignore-files": false,
                "include-hidden": true
            } }"#,
        );
        assert!(!files.enabled);
        assert_eq!(files.in_root, InRoot::Folders);
        assert_eq!(files.scopes, [PathBuf::from("~"), PathBuf::from("D:/Dev")]);
        assert_eq!(files.exclusions, [PathBuf::from("~/Videos")]);
        assert_eq!(files.ignore_patterns, ["*.log"]);
        assert!(!files.use_ignore_files);
        assert!(files.include_hidden);
        assert_eq!(
            serde_json::from_str::<FileSearchConfig>(&serde_json::to_string(&files).unwrap())
                .unwrap(),
            files
        );
        assert_eq!(
            self::files(r#"{ "files": { "in-root": "off" } }"#).in_root,
            InRoot::Off
        );
    }

    /// A preview is monospace unless the user prefers the interface font,
    /// and a value this build does not know is that default, never an error.
    #[test]
    fn the_preview_font_is_monospace_unless_set_to_the_interface() {
        assert_eq!(
            FileSearchConfig::default().preview_font,
            PreviewFont::Monospace
        );
        assert_eq!(
            files(r#"{ "files": { "preview-font": "interface" } }"#).preview_font,
            PreviewFont::Interface
        );
        assert_eq!(
            files(r#"{ "files": { "preview-font": "monospace" } }"#).preview_font,
            PreviewFont::Monospace
        );
        assert_eq!(
            files(r#"{ "files": { "preview-font": "comic-sans" } }"#).preview_font,
            PreviewFont::Monospace
        );
        assert_eq!(
            files(r#"{ "files": { "preview-font": 3 } }"#).preview_font,
            PreviewFont::Monospace
        );

        let interface = FileSearchConfig {
            preview_font: PreviewFont::Interface,
            ..FileSearchConfig::default()
        };
        let json = serde_json::to_string(&interface).unwrap();
        assert!(json.contains(r#""preview-font":"interface""#), "{json}");
        assert_eq!(
            serde_json::from_str::<FileSearchConfig>(&json).unwrap(),
            interface
        );
    }

    /// Balanced unless set, and a value this build does not know is that
    /// default, never an error.
    #[test]
    fn the_index_detail_is_balanced_unless_set() {
        assert_eq!(
            FileSearchConfig::default().index_detail,
            IndexDetail::Balanced
        );
        for detail in IndexDetail::ALL {
            let source = format!(
                r#"{{ "files": {{ "index-detail": "{}" }} }}"#,
                detail.as_str()
            );
            assert_eq!(files(&source).index_detail, detail);
        }
        for unknown in [r#""everything""#, "3", "null", r#""Complete""#] {
            let source = format!(r#"{{ "files": {{ "index-detail": {unknown} }} }}"#);
            assert_eq!(
                files(&source).index_detail,
                IndexDetail::Balanced,
                "{unknown}"
            );
        }

        let compact = FileSearchConfig {
            index_detail: IndexDetail::Compact,
            ..FileSearchConfig::default()
        };
        let json = serde_json::to_string(&compact).unwrap();
        assert!(json.contains(r#""index-detail":"compact""#), "{json}");
        assert_eq!(
            serde_json::from_str::<FileSearchConfig>(&json).unwrap(),
            compact
        );
    }

    /// The level is not a rule of the walk: changing it walks nothing again.
    #[test]
    fn the_index_detail_is_not_part_of_the_walk_digest() {
        let base = FileSearchConfig::default();
        for detail in IndexDetail::ALL {
            let changed = FileSearchConfig {
                index_detail: detail,
                ..base.clone()
            };
            assert_eq!(
                crate::file_index::digest(&changed.walk_rules()),
                crate::file_index::digest(&base.walk_rules())
            );
        }
    }

    /// F3's `roots` replaced the default folders; a scope list does the same.
    #[test]
    fn legacy_roots_become_the_scopes() {
        let files = files(r#"{ "files": { "roots": ["D:/work"] } }"#);
        assert_eq!(files.scopes, [PathBuf::from("D:/work")]);

        let both = self::files(r#"{ "files": { "roots": ["D:/work"], "scopes": ["E:/"] } }"#);
        assert_eq!(both.scopes, [PathBuf::from("E:/")]);
    }

    /// Empty `roots` meant F3's default folders, which are inside home.
    #[test]
    fn empty_legacy_roots_mean_home() {
        let files = files(r#"{ "files": { "roots": [] } }"#);
        assert_eq!(files.scopes, [PathBuf::from("~")]);
    }

    /// An empty scope list is nothing indexed, not home (FILE-SEARCH.md).
    #[test]
    fn an_empty_scope_list_stays_empty() {
        assert!(files(r#"{ "files": { "scopes": [] } }"#).scopes.is_empty());
    }

    /// `limits` is read and ignored, never an error; nor is a key from a
    /// later Lumen.
    #[test]
    fn legacy_limits_and_unknown_keys_still_load() {
        let files = files(
            r#"{ "files": {
                "enabled": false,
                "limits": { "max-entries": 20000, "max-depth": 3 },
                "declined-drives": ["E:/"]
            } }"#,
        );
        assert!(!files.enabled);
        assert_eq!(files.scopes, [PathBuf::from("~")]);
    }

    /// The drives answered *Not now* are read as written (slice 5).
    #[test]
    fn declined_drives_are_read() {
        let files = files(r#"{ "files": { "declined-drives": ["E:/", "D:/"] } }"#);
        assert_eq!(
            files.declined_drives,
            [PathBuf::from("E:/"), PathBuf::from("D:/")]
        );
        assert!(FileSearchConfig::default().declined_drives.is_empty());
    }

    #[test]
    fn root_asks_the_index_for_what_in_root_says() {
        let mut files = FileSearchConfig::default();
        assert_eq!(files.root_kinds(), Some(Kinds::FoldersAndFiles));
        files.in_root = InRoot::Folders;
        assert_eq!(files.root_kinds(), Some(Kinds::Folders));
        files.in_root = InRoot::Off;
        assert_eq!(files.root_kinds(), None);
        files.in_root = InRoot::FoldersAndFiles;
        files.enabled = false;
        assert_eq!(files.root_kinds(), None);
    }

    /// What root shows of the index, and whether it is shown at all, are not
    /// how it is walked: changing them must not walk the scopes again.
    #[test]
    fn in_root_and_enabled_do_not_change_the_walk() {
        let files = FileSearchConfig::default();
        let walked = crate::file_index::digest(&files.walk_rules());
        for changed in [
            FileSearchConfig {
                in_root: InRoot::Folders,
                ..files.clone()
            },
            FileSearchConfig {
                in_root: InRoot::Off,
                ..files.clone()
            },
            FileSearchConfig {
                enabled: false,
                ..files.clone()
            },
        ] {
            assert_eq!(crate::file_index::digest(&changed.walk_rules()), walked);
        }

        let excluded = FileSearchConfig {
            exclusions: vec!["~/Videos".into()],
            ..files
        };
        assert_ne!(crate::file_index::digest(&excluded.walk_rules()), walked);
    }

    /// A default skip turned back on walks the scopes again; a name not on
    /// the list, or one written in another case or order, does not change
    /// what is walked.
    #[test]
    fn default_skips_off_change_the_walk_only_by_known_names() {
        let defaults = FileSearchConfig::default();
        let walked = crate::file_index::digest(&defaults.walk_rules());

        let on = files(r#"{ "files": { "default-skips-off": ["node_modules", "target"] } }"#);
        assert_eq!(on.walk_rules().skips_off, ["node_modules", "target"]);
        assert_ne!(crate::file_index::digest(&on.walk_rules()), walked);

        let same = files(
            r#"{ "files": { "default-skips-off": ["Target", "NODE_MODULES", "no-such-skip"] } }"#,
        );
        assert_eq!(same.default_skips_off.len(), 3);
        assert_eq!(
            crate::file_index::digest(&same.walk_rules()),
            crate::file_index::digest(&on.walk_rules())
        );

        let unknown = files(r#"{ "files": { "default-skips-off": ["no-such-skip"] } }"#);
        assert!(unknown.walk_rules().skips_off.is_empty());
        assert_eq!(crate::file_index::digest(&unknown.walk_rules()), walked);
    }

    /// `~` is the home folder, as the disk spells it; a path that does not
    /// exist is kept as written.
    #[test]
    fn walk_rules_resolve_home_and_keep_a_missing_path() {
        let files = FileSearchConfig {
            scopes: vec!["~".into(), "no/such/folder".into()],
            ..FileSearchConfig::default()
        };
        let rules = files.walk_rules();
        assert_eq!(rules.ceiling, CEILING);
        assert_eq!(rules.scopes[1], PathBuf::from("no/such/folder"));
        if let Some(dirs) = directories::UserDirs::new() {
            let home = plain(&std::fs::canonicalize(dirs.home_dir()).unwrap());
            assert_eq!(rules.scopes[0], home);
        }
    }

    /// An exclusion typed in another case must still match what the walk of
    /// its scope finds -- where the disk ignores case, as Windows' does.
    #[test]
    fn an_exclusion_is_spelled_as_the_disk_spells_it() {
        let root = std::env::temp_dir().join("lumen-config-exclusion-case");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Videos")).unwrap();
        let typed = root.join("VIDEOS");

        let files = FileSearchConfig {
            scopes: vec![root.clone()],
            exclusions: vec![typed.clone()],
            ..FileSearchConfig::default()
        };
        let rules = files.walk_rules();
        if typed.exists() {
            assert_eq!(rules.exclusions[0], rules.scopes[0].join("Videos"));
        }
        assert!(!rules.scopes[0].to_string_lossy().starts_with(r"\\?"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_verbatim_prefix_is_given_back_in_its_plain_form() {
        // Only where the platform reads the prefix: elsewhere the path has
        // none, and is returned as it is.
        let disk = Path::new(r"\\?\C:\Users\someone");
        if matches!(disk.components().next(), Some(Component::Prefix(_))) {
            assert_eq!(plain(disk), PathBuf::from(r"C:\Users\someone"));
            assert_eq!(
                plain(Path::new(r"\\?\UNC\server\share\folder")),
                PathBuf::from(r"\\server\share\folder")
            );
        }
        assert_eq!(
            plain(Path::new("relative/path")),
            PathBuf::from("relative/path")
        );
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
        let popover = crate::layout::action_panel::height(
            &[ActionGroup::Primary; 4],
            Metrics::BASE.actions_rows(),
        );

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
        let popover = crate::layout::action_panel::height(
            &[ActionGroup::Primary; 6],
            Metrics::BASE.actions_rows(),
        );

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

    /// The panel's own search field is part of the popover the window stands
    /// it in: a short list grows by the field's height too.
    #[test]
    fn the_popover_s_room_includes_its_search_field() {
        use crate::item::ActionGroup;
        use crate::layout::action_panel;

        let config = Config::default();
        let list = config.window_height(&Metrics::BASE, 0);
        let groups = [ActionGroup::Primary; 3];
        let rows = 3.0 * action_panel::ROW_HEIGHT;

        let height = config.panel_height(
            &Metrics::BASE,
            list,
            action_panel::height(&groups, Metrics::BASE.actions_rows()),
        );

        assert_eq!(
            height,
            Metrics::BASE.search_height
                + action_panel::INSET * 2.0
                + action_panel::HEADER_HEIGHT
                + action_panel::PADDING * 2.0
                + action_panel::BORDER * 2.0
                + rows
                + action_panel::SEARCH_HEIGHT
                + Metrics::BASE.footer_height
        );
    }

    /// The room kept around the popover is the style's vertical offset, on
    /// both sides whatever the anchor, so a top anchor is as clear of the
    /// search field as a bottom one is of the footer.
    #[test]
    fn a_short_list_grows_by_the_style_s_vertical_offset() {
        use crate::item::ActionGroup;
        use crate::layout::action_panel;

        let config = Config::default();
        let metrics = Metrics {
            actions_offset_y: 20.0,
            actions_max_rows: 3.0,
            ..Metrics::BASE
        };
        let list = config.window_height(&metrics, 0);
        let popover = action_panel::height(&[ActionGroup::Primary; 9], metrics.actions_rows());

        assert_eq!(
            popover,
            action_panel::height(&[ActionGroup::Primary; 3], action_panel::MAX_ROWS)
        );
        assert_eq!(
            config.panel_height(&metrics, list, popover),
            metrics.search_height + 20.0 * 2.0 + popover + metrics.footer_height
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
