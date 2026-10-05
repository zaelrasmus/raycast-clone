//! The File Search page of the settings window (F16, slice 4;
//! `docs/FILE-SEARCH.md`, "The settings page").
//!
//! The `files` section of `config.json`, in four groups: what root search
//! shows, where the index looks, what it leaves out, and the index itself.
//! Written through `lumen_core::file_search::settings`; the daemon applies
//! the file as it changes, and walks again only when the rules' digest
//! moved.
//!
//! Each choice, switch and folder is written as it is made, as on the
//! Appearance page: there is nothing half done about a switch. The ignore
//! patterns are typed, so they wait for Enter, as the clipboard's limit does
//! -- a half-typed glob must not be live.
//!
//! The page holds no index. What it says about it -- the items, each
//! scope's count, the scope cut short, whether it is indexing -- is the
//! launcher's, handed over by the daemon whenever it moves
//! ([`FileSearchPage::set_status`]); *Rebuild File Index* goes back the same
//! way, as an event.
//!
//! Whether Windows Search covers each scope, so content search finds text
//! in it (slice 6c), is asked of the platform each time the file is read:
//! on open, and after every change.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::config::{FileSearchConfig, InRoot, PreviewFont};
use lumen_core::file_index::{IndexDetail, IndexStatus, default_skips};
use lumen_core::file_search::settings::{self, Flag, List};
use lumen_core::layout::settings as metrics;
use lumen_core::theme::Colors;
use lumen_platform::Platform;

use crate::quicklink_form::{
    CancelForm, NextField, PreviousField, SubmitForm, TOGGLE_CONTEXT, ToggleFallback,
};
use crate::settings::{choice_button, text_button};
use crate::settings_ui as ui;
use crate::styling::color;
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

/// Above the scopes: what a scope costs (FILE-SEARCH.md, "Against the
/// budgets").
const SCOPE_NOTE: &str = "Every folder and drive you add is indexed on disk. A whole drive makes \
                          the index larger and its first build longer, and fills it with files \
                          you won't look for: add the folders you search.";

/// The default skip that is not like the others, and says why.
const APPDATA: &str = "AppData";

/// Whether Windows Search indexes a scope's contents (slice 6c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    Covered,
    NotCovered,
    /// Not asked yet, or the asking failed.
    Unknown,
}

/// A scope's description with its coverage after it, when contents are
/// searched and the coverage is known.
fn with_coverage(base: &str, coverage: Coverage, contents_on: bool) -> String {
    let told = match coverage {
        _ if !contents_on => None,
        Coverage::Covered => Some("Contents searchable"),
        Coverage::NotCovered => Some("Contents not indexed by Windows"),
        Coverage::Unknown => None,
    };
    match told {
        Some(told) => format!("{base} \u{b7} {told}"),
        None => base.to_string(),
    }
}

/// What the page asks of the settings window.
#[derive(Debug, Clone)]
pub enum FileSearchPageEvent {
    /// *Rebuild File Index*: the launcher holds the index.
    Rebuild,
}

impl gpui::EventEmitter<FileSearchPageEvent> for FileSearchPage {}

pub struct FileSearchPage {
    path: PathBuf,
    colors: Colors,
    platform: Arc<dyn Platform>,
    /// What a picked folder is shortened against, found once.
    home: Option<PathBuf>,
    /// The section as the file last read had it.
    config: FileSearchConfig,
    /// Why the file could not be read. Nothing is written until it is fixed.
    problem: Option<String>,
    /// The launcher's index, as last handed over.
    index: IndexStatus,
    patterns: Entity<TextField>,
    /// The patterns field was typed in since the last save or load.
    edited: bool,
    flags: Vec<(Flag, FocusHandle)>,
    /// One per default skip, in the list's order.
    skips: Vec<(&'static str, FocusHandle)>,
    /// A folder picker is up, so another is not opened over it.
    picking: bool,
    /// A whole drive picked as a scope, waiting for *Add anyway* or
    /// *Cancel* (slice 5).
    whole_drive: Option<PathBuf>,
    /// Each scope as written, and whether Windows Search covers it.
    coverage: Vec<(PathBuf, Coverage)>,
    /// Which asking of the coverage is the latest, so an older answer
    /// arriving late is dropped.
    coverage_asked: u64,
    error: Option<String>,
    status: Option<SharedString>,
}

impl FileSearchPage {
    pub fn new(
        path: PathBuf,
        platform: Arc<dyn Platform>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = FileSearchConfig::default();
        let patterns = patterns_field(&config, &colors, cx);
        let flags = [
            Flag::Enabled,
            Flag::UseIgnoreFiles,
            Flag::IncludeHidden,
            Flag::SearchContents,
        ]
        .into_iter()
        .map(|flag| (flag, cx.focus_handle()))
        .collect();
        let skips = default_skips()
            .iter()
            .map(|name| (*name, cx.focus_handle()))
            .collect();
        let mut page = Self {
            path,
            colors,
            platform,
            home: lumen_core::file_search::home_dir(),
            config,
            problem: None,
            index: IndexStatus::default(),
            patterns,
            edited: false,
            flags,
            skips,
            picking: false,
            whole_drive: None,
            coverage: Vec::new(),
            coverage_asked: 0,
            error: None,
            status: None,
        };
        page.watch_patterns(cx);
        page
    }

    /// Re-read the file, off the UI thread. An untouched patterns field
    /// follows it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { settings::read(&path) })
                .await;
            this.update(cx, |page, cx| {
                match read {
                    Ok(config) => {
                        page.config = config;
                        page.problem = None;
                        page.check_coverage(cx);
                        if !page.edited {
                            let written = page.config.ignore_patterns.join(", ");
                            // Only when it differs: set, the field keeps its
                            // focus, but not where the caret was.
                            if page.patterns.read(cx).text() != written {
                                page.patterns
                                    .update(cx, |field, cx| field.set_text(written, cx));
                            }
                        }
                    }
                    Err(error) => {
                        log::warn!("the file search page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the launcher's index says now.
    pub fn set_status(&mut self, index: IndexStatus, cx: &mut Context<Self>) {
        if self.index != index {
            self.index = index;
            cx.notify();
        }
    }

    /// Ask whether Windows Search covers each scope, off the UI thread. The
    /// last answers stay up until the new ones arrive.
    fn check_coverage(&mut self, cx: &mut Context<Self>) {
        self.coverage_asked += 1;
        let asked = self.coverage_asked;
        let platform = self.platform.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let coverage = cx
                .background_executor()
                .spawn(async move {
                    // Resolved as the walk resolves them: `~`, then the
                    // disk's spelling. In the same order as written.
                    let resolved = config.walk_rules().scopes;
                    config
                        .scopes
                        .into_iter()
                        .zip(resolved)
                        .map(|(written, scope)| {
                            let coverage = match platform.scope_covered(&scope) {
                                Ok(true) => Coverage::Covered,
                                Ok(false) => Coverage::NotCovered,
                                Err(error) => {
                                    log::debug!(
                                        "cannot tell whether {} is indexed: {error:#}",
                                        scope.display()
                                    );
                                    Coverage::Unknown
                                }
                            };
                            (written, coverage)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |page, cx| {
                if page.coverage_asked == asked && page.coverage != coverage {
                    page.coverage = coverage;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn scope_coverage(&self, scope: &std::path::Path) -> Coverage {
        self.coverage
            .iter()
            .find(|(written, _)| written == scope)
            .map_or(Coverage::Unknown, |(_, coverage)| *coverage)
    }

    /// *Indexing Options…*: Windows' own, off the UI thread.
    fn open_indexing_options(&mut self, cx: &mut Context<Self>) {
        let platform = self.platform.clone();
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move { platform.open_indexing_options() })
                .await;
            this.update(cx, |page, cx| {
                match opened {
                    Ok(()) => {
                        page.status = Some(
                            "Indexing Options opened; reopen settings to see what changed".into(),
                        )
                    }
                    Err(error) => {
                        log::warn!("could not open Indexing Options: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn watch_patterns(&mut self, cx: &mut Context<Self>) {
        cx.subscribe(&self.patterns, |page, _, event, cx| {
            if matches!(event, TextFieldEvent::Changed(_)) {
                page.edited = true;
                page.error = None;
                cx.notify();
            }
        })
        .detach();
    }

    // -- Writing ----------------------------------------------------------------

    /// Write one change through `change`, off the UI thread, then read the
    /// file back so the page shows what it now says.
    fn write(
        &mut self,
        done: String,
        change: impl FnOnce(&std::path::Path) -> anyhow::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.problem.is_some() {
            return;
        }
        let path = self.path.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { change(&path) })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(()) => page.status = Some(done.into()),
                    Err(error) => {
                        log::warn!("could not save file search: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                page.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_in_root(&mut self, in_root: InRoot, cx: &mut Context<Self>) {
        if self.config.in_root == in_root {
            return;
        }
        let done = format!(
            "Root search shows {}",
            in_root_label(in_root).to_lowercase()
        );
        self.write(done, move |path| settings::set_in_root(path, in_root), cx);
    }

    fn set_preview_font(&mut self, font: PreviewFont, cx: &mut Context<Self>) {
        if self.config.preview_font == font {
            return;
        }
        let done = format!("Previews in the {} font", font_label(font).to_lowercase());
        self.write(done, move |path| settings::set_preview_font(path, font), cx);
    }

    fn set_index_detail(&mut self, detail: IndexDetail, cx: &mut Context<Self>) {
        if self.config.index_detail == detail {
            return;
        }
        let done = format!("Index detail {}", detail_label(detail).to_lowercase());
        self.write(
            done,
            move |path| settings::set_index_detail(path, detail),
            cx,
        );
    }

    fn set_hot_memory(&mut self, megabytes: u32, cx: &mut Context<Self>) {
        if self.config.hot_memory == megabytes {
            return;
        }
        let done = format!("Instant results use {megabytes} MB");
        self.write(
            done,
            move |path| settings::set_hot_memory(path, megabytes),
            cx,
        );
    }

    fn flip_flag(&mut self, flag: Flag, cx: &mut Context<Self>) {
        let on = !flag_on(&self.config, flag);
        let done = format!("{} {}", flag_title(flag), if on { "on" } else { "off" });
        self.write(done, move |path| settings::set_flag(path, flag, on), cx);
    }

    fn flip_skip(&mut self, name: &'static str, cx: &mut Context<Self>) {
        let excluded = !settings::is_excluded(&self.config, name);
        let done = if excluded {
            format!("{name} is excluded again")
        } else {
            format!("{name} is indexed")
        };
        self.write(
            done,
            move |path| settings::set_default_skip(path, name, excluded),
            cx,
        );
    }

    fn remove(&mut self, list: List, folder: String, cx: &mut Context<Self>) {
        let done = format!("Removed {folder}");
        self.write(done, move |path| settings::remove(path, list, &folder), cx);
    }

    /// Ask for a folder with the system's picker, off the UI thread, and add
    /// it to `list` as `config.json` writes it. A whole drive as a scope is
    /// asked about first (slice 5).
    fn pick(&mut self, list: List, cx: &mut Context<Self>) {
        if self.picking || self.problem.is_some() {
            return;
        }
        self.picking = true;
        self.whole_drive = None;
        self.error = None;
        let platform = self.platform.clone();
        let title = match list {
            List::Scopes => "Add a Folder to Search",
            List::Exclusions => "Exclude a Folder",
        };
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move { platform.pick_folder(title, None) })
                .await;
            this.update(cx, |page, cx| {
                page.picking = false;
                match picked {
                    Ok(None) => {}
                    Ok(Some(folder))
                        if list == List::Scopes
                            && lumen_core::file_search::drives::is_drive_root(&folder) =>
                    {
                        page.whole_drive = Some(folder);
                    }
                    Ok(Some(folder)) => page.add(list, folder, cx),
                    Err(error) => {
                        log::warn!("could not show the folder picker: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// *Add anyway* on the whole-drive warning, or *Cancel*.
    fn answer_whole_drive(&mut self, add: bool, cx: &mut Context<Self>) {
        if let Some(root) = self.whole_drive.take() {
            if add {
                self.add(List::Scopes, root, cx);
            }
            cx.notify();
        }
    }

    /// Write `folder` into `list`, off the UI thread.
    fn add(&mut self, list: List, folder: PathBuf, cx: &mut Context<Self>) {
        let written = settings::written(&folder, self.home.as_deref());
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let written = written.clone();
                    async move { settings::add(&path, list, &written) }
                })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(true) => page.status = Some(format!("Added {written}").into()),
                    Ok(false) => {
                        page.status = Some(format!("{written} is already in the list").into());
                    }
                    Err(error) => {
                        log::warn!("could not add a folder: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                page.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Save the ignore patterns, as Enter does. For the window's Ctrl+S too.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.edited || self.problem.is_some() {
            return;
        }
        let patterns = settings::parse_patterns(self.patterns.read(cx).text());
        self.edited = false;
        let done = match patterns.len() {
            0 => "No ignore patterns".to_string(),
            1 => "1 ignore pattern".to_string(),
            count => format!("{count} ignore patterns"),
        };
        self.write(
            done,
            move |path| settings::set_ignore_patterns(path, &patterns),
            cx,
        );
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edited {
            return;
        }
        let written = self.config.ignore_patterns.join(", ");
        self.patterns
            .update(cx, |field, cx| field.set_text(written, cx));
        self.edited = false;
        self.status = Some("Changes discarded".into());
        cx.notify();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.status = Some("Rebuilding the file index".into());
        cx.emit(FileSearchPageEvent::Rebuild);
        cx.notify();
    }

    // -- Focus ----------------------------------------------------------------

    /// Everything Tab visits, in order: the switches and the field, as they
    /// are drawn.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let flag = |wanted: Flag| {
            self.flags
                .iter()
                .find(|(flag, _)| *flag == wanted)
                .map(|(_, focus)| focus.clone())
        };
        let mut stops: Vec<FocusHandle> = [
            Flag::UseIgnoreFiles,
            Flag::IncludeHidden,
            Flag::SearchContents,
        ]
        .into_iter()
        .filter_map(flag)
        .collect();
        stops.push(self.patterns.focus_handle(cx));
        stops.extend(self.skips.iter().map(|(_, focus)| focus.clone()));
        stops.extend(flag(Flag::Enabled));
        stops
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let at = stops
            .iter()
            .position(|focus| focus.is_focused(window))
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % stops.len()
        } else {
            (at + stops.len() - 1) % stops.len()
        };
        window.focus(&stops[next]);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(true, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(false, window, cx);
    }

    /// Space on a focused switch.
    fn toggle(&mut self, _: &ToggleFallback, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(flag) = self
            .flags
            .iter()
            .find(|(_, focus)| focus.is_focused(window))
            .map(|(flag, _)| *flag)
        {
            self.flip_flag(flag, cx);
        } else if let Some(name) = self
            .skips
            .iter()
            .find(|(_, focus)| focus.is_focused(window))
            .map(|(name, _)| *name)
        {
            self.flip_skip(name, cx);
        }
    }

    // -- What the host asks ---------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.patterns
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.colors = colors;
        cx.notify();
    }

    /// What was saved, or why not; the window's status line shows it.
    pub fn status(&self) -> Option<SharedString> {
        if let Some(error) = &self.error {
            return Some(error.clone().into());
        }
        if self.edited {
            return Some("Unsaved patterns  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }
}

/// The patterns field, filled in from `config`.
fn patterns_field(config: &FileSearchConfig, colors: &Colors, cx: &mut App) -> Entity<TextField> {
    let written = config.ignore_patterns.join(", ");
    cx.new(|cx| TextField::new(written, "*.log, **/build/**", colors.clone(), cx))
}

fn flag_on(config: &FileSearchConfig, flag: Flag) -> bool {
    match flag {
        Flag::Enabled => config.enabled,
        Flag::UseIgnoreFiles => config.use_ignore_files,
        Flag::IncludeHidden => config.include_hidden,
        Flag::SearchContents => config.search_contents,
    }
}

fn flag_title(flag: Flag) -> &'static str {
    match flag {
        Flag::Enabled => "File index",
        Flag::UseIgnoreFiles => "Use ignore files",
        Flag::IncludeHidden => "Include hidden files",
        Flag::SearchContents => "Search file contents",
    }
}

fn flag_description(flag: Flag) -> &'static str {
    match flag {
        Flag::Enabled => "Off drops the index and its watch; nothing is searched",
        Flag::UseIgnoreFiles => ".gitignore, .ignore and .lumenignore in the folders searched",
        Flag::IncludeHidden => "Names starting with a dot, and the hidden attribute",
        Flag::SearchContents => "Through Windows Search, from three letters",
    }
}

fn in_root_label(in_root: InRoot) -> &'static str {
    match in_root {
        InRoot::Off => "Off",
        InRoot::Folders => "Folders",
        InRoot::FoldersAndFiles => "Folders and files",
    }
}

fn font_label(font: PreviewFont) -> &'static str {
    match font {
        PreviewFont::Monospace => "Monospace",
        PreviewFont::Interface => "Interface",
    }
}

fn detail_label(detail: IndexDetail) -> &'static str {
    match detail {
        IndexDetail::Complete => "Complete",
        IndexDetail::Balanced => "Balanced",
        IndexDetail::Compact => "Compact",
    }
}

/// What a level costs on disk and what it buys, in one line: measured on a
/// real 500,000-item index (FILE-SEARCH.md, "Step c2b").
fn detail_description(detail: IndexDetail) -> &'static str {
    match detail {
        IndexDetail::Complete => {
            "Finds matches inside words without a scan; the most disk, about 150 MB for 500,000 items"
        }
        IndexDetail::Balanced => {
            "Finds names by their words, inside them with a scan; about 110 MB for 500,000 items"
        }
        IndexDetail::Compact => {
            "Finds the same as Balanced, a little slower on common words; about 90 MB for 500,000 items"
        }
    }
}

/// What `files.hot-memory` offers, in megabytes: the default and the steps
/// either side of it, within [`lumen_core::config::HOT_MEMORY`].
const HOT_MEMORY_CHOICES: [u32; 4] = [4, 8, 16, 32];

fn hot_memory_label(megabytes: u32) -> &'static str {
    match megabytes {
        4 => "4 MB",
        8 => "8 MB",
        16 => "16 MB",
        _ => "32 MB",
    }
}

/// *1 item*, *12,345 items*.
fn items(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{} items", lumen_core::places::browse::grouped(count))
    }
}

/// Seconds since the epoch, now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

impl FileSearchPage {
    /// What a scope's row says under it: its count from the index, and
    /// whether Windows Search covers it.
    fn scope_description(&self, at: usize) -> SharedString {
        let base = self.scope_count(at);
        let coverage = self
            .config
            .scopes
            .get(at)
            .map_or(Coverage::Unknown, |scope| self.scope_coverage(scope));
        with_coverage(&base, coverage, self.config.search_contents).into()
    }

    /// A scope's count from the index.
    fn scope_count(&self, at: usize) -> SharedString {
        // The counts are by position in the scopes walked, so they mean
        // nothing while the list and the index disagree on how many.
        let counted = (self.index.loaded && self.index.per_scope.len() == self.config.scopes.len())
            .then(|| self.index.per_scope.get(at).copied())
            .flatten();
        match (counted, self.index.indexing) {
            (Some(count), _) => items(count).into(),
            (None, true) => "Indexing\u{2026}".into(),
            (None, false) => "Not indexed yet".into(),
        }
    }

    /// How many entries the index holds: every one the rules allow, since
    /// nothing caps it.
    fn total_description(&self) -> String {
        items(self.index.entries)
    }

    /// The index group's status line.
    fn index_description(&self) -> String {
        if self.index.indexing {
            return "Indexing\u{2026}".into();
        }
        if !self.index.loaded {
            return if self.config.enabled {
                "Not indexed yet".into()
            } else {
                "Off".into()
            };
        }
        match self.index.walked_at {
            Some(at) => format!(
                "{}, last updated {}",
                items(self.index.entries),
                lumen_core::weather::age(now() - at as i64)
            ),
            None => items(self.index.entries),
        }
    }

    fn render_where(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = &self.colors;
        let mut scopes: Vec<AnyElement> = self
            .config
            .scopes
            .iter()
            .enumerate()
            .map(|(at, scope)| {
                let written = scope.to_string_lossy().into_owned();
                let remove = {
                    let written = written.clone();
                    text_button(
                        ("remove-scope", at),
                        "Remove",
                        colors,
                        cx.listener(move |page, _, _, cx| {
                            page.remove(List::Scopes, written.clone(), cx)
                        }),
                    )
                };
                ui::row(written, Some(self.scope_description(at)), remove, colors)
            })
            .collect();
        let uncovered = self.config.search_contents
            && self
                .config
                .scopes
                .iter()
                .any(|scope| self.scope_coverage(scope) == Coverage::NotCovered);
        if uncovered {
            scopes.push(ui::row(
                "Windows Search",
                Some("Contents are found only in the folders it indexes".into()),
                text_button(
                    "indexing-options",
                    "Indexing Options\u{2026}",
                    colors,
                    cx.listener(|page, _, _, cx| page.open_indexing_options(cx)),
                ),
                colors,
            ));
        }
        if scopes.is_empty() {
            scopes.push(ui::row(
                "No folders",
                Some("Nothing is indexed until a folder is added".into()),
                div(),
                colors,
            ));
        }
        if let Some(root) = &self.whole_drive {
            let answers = div()
                .flex()
                .gap(px(metrics::TAB_GAP))
                .child(text_button(
                    "add-whole-drive",
                    "Add anyway",
                    colors,
                    cx.listener(|page, _, _, cx| page.answer_whole_drive(true, cx)),
                ))
                .child(text_button(
                    "cancel-whole-drive",
                    "Cancel",
                    colors,
                    cx.listener(|page, _, _, cx| page.answer_whole_drive(false, cx)),
                ));
            scopes.push(ui::row(
                format!(
                    "{} is the whole drive: a larger index, slower to build, mostly of files \
                     you won't look for.",
                    root.display()
                ),
                Some("Add the folders you search on it instead, or add it anyway".into()),
                answers,
                colors,
            ));
        }
        scopes.push(ui::row(
            "Total",
            Some(self.total_description().into()),
            text_button(
                "add-scope",
                "Add Folder\u{2026}",
                colors,
                cx.listener(|page, _, _, cx| page.pick(List::Scopes, cx)),
            ),
            colors,
        ));

        let mut exclusions: Vec<AnyElement> = self
            .config
            .exclusions
            .iter()
            .enumerate()
            .map(|(at, exclusion)| {
                let written = exclusion.to_string_lossy().into_owned();
                let remove = {
                    let written = written.clone();
                    text_button(
                        ("remove-exclusion", at),
                        "Remove",
                        colors,
                        cx.listener(move |page, _, _, cx| {
                            page.remove(List::Exclusions, written.clone(), cx)
                        }),
                    )
                };
                ui::row(written, None, remove, colors)
            })
            .collect();
        exclusions.push(ui::row(
            "Exclude a folder",
            Some("Never looked inside, wherever it is in the folders searched".into()),
            text_button(
                "add-exclusion",
                "Add Folder\u{2026}",
                colors,
                cx.listener(|page, _, _, cx| page.pick(List::Exclusions, cx)),
            ),
            colors,
        ));

        vec![
            ui::section_title("Where", colors).into_any_element(),
            div()
                .px(px(metrics::CARD_PADDING_X))
                .text_size(px(metrics::text::ROW_DESCRIPTION))
                .text_color(color(colors.text_muted))
                .child(SCOPE_NOTE)
                .into_any_element(),
            ui::card(scopes, colors).into_any_element(),
            ui::card(exclusions, colors).into_any_element(),
        ]
    }

    fn render_what(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = &self.colors;
        let entity = cx.entity();
        let flag_row = |flag: Flag, entity: Entity<Self>| {
            let focus = self
                .flags
                .iter()
                .find(|(each, _)| *each == flag)
                .map(|(_, focus)| focus.clone())?;
            Some(ui::toggle_row(
                ("flag", flag as usize),
                flag_title(flag),
                Some(flag_description(flag).into()),
                flag_on(&self.config, flag),
                &focus,
                TOGGLE_CONTEXT,
                window,
                colors,
                move |_window, cx| entity.update(cx, |page, cx| page.flip_flag(flag, cx)),
            ))
        };
        let mut rules: Vec<AnyElement> = [
            Flag::UseIgnoreFiles,
            Flag::IncludeHidden,
            Flag::SearchContents,
        ]
        .into_iter()
        .filter_map(|flag| flag_row(flag, entity.clone()))
        .collect();
        rules.push(ui::row(
            "Ignore patterns",
            Some("Globs, comma separated, in every folder searched".into()),
            ui::control(self.patterns.clone()),
            colors,
        ));
        let fonts = div().flex().gap(px(metrics::TAB_GAP)).children(
            [PreviewFont::Monospace, PreviewFont::Interface].map(|font| {
                choice_button(
                    font_label(font),
                    self.config.preview_font == font,
                    colors,
                    cx.listener(move |page, _, _, cx| page.set_preview_font(font, cx)),
                )
            }),
        );
        rules.push(ui::row(
            "Preview font",
            Some("What a text file's preview in Search Files is drawn in".into()),
            fonts,
            colors,
        ));

        let skips: Vec<AnyElement> = self
            .skips
            .iter()
            .enumerate()
            .map(|(at, (name, focus))| {
                let entity = entity.clone();
                let name = *name;
                let description = (name == APPDATA).then(|| {
                    SharedString::from(
                        "Changes constantly: indexing it costs CPU and memory all the time",
                    )
                });
                ui::toggle_row(
                    ("skip", at),
                    name,
                    description,
                    settings::is_excluded(&self.config, name),
                    focus,
                    TOGGLE_CONTEXT,
                    window,
                    colors,
                    move |_window, cx| entity.update(cx, |page, cx| page.flip_skip(name, cx)),
                )
            })
            .collect();

        vec![
            ui::section_title("What", colors).into_any_element(),
            ui::card(rules, colors).into_any_element(),
            ui::section_title("Excluded by default", colors).into_any_element(),
            ui::card(skips, colors).into_any_element(),
        ]
    }

    fn render_index(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = &self.colors;
        let entity = cx.entity();
        let mut rows = Vec::new();
        if let Some((_, focus)) = self.flags.iter().find(|(flag, _)| *flag == Flag::Enabled) {
            rows.push(ui::toggle_row(
                ("flag", Flag::Enabled as usize),
                flag_title(Flag::Enabled),
                Some(flag_description(Flag::Enabled).into()),
                self.config.enabled,
                focus,
                TOGGLE_CONTEXT,
                window,
                colors,
                move |_window, cx| entity.update(cx, |page, cx| page.flip_flag(Flag::Enabled, cx)),
            ));
        }
        let details = div()
            .flex()
            .gap(px(metrics::TAB_GAP))
            .children(IndexDetail::ALL.map(|detail| {
                choice_button(
                    detail_label(detail),
                    self.config.index_detail == detail,
                    colors,
                    cx.listener(move |page, _, _, cx| page.set_index_detail(detail, cx)),
                )
            }));
        rows.push(ui::row(
            "Detail",
            Some(detail_description(self.config.index_detail).into()),
            details,
            colors,
        ));
        let memory = div()
            .flex()
            .gap(px(metrics::TAB_GAP))
            .children(HOT_MEMORY_CHOICES.map(|megabytes| {
                choice_button(
                    hot_memory_label(megabytes),
                    self.config.hot_memory == megabytes,
                    colors,
                    cx.listener(move |page, _, _, cx| page.set_hot_memory(megabytes, cx)),
                )
            }));
        rows.push(ui::row(
            "Instant results memory",
            Some(
                "Memory for the fastest part of root's file search. More finds more files                  instantly; costs idle memory"
                    .into(),
            ),
            memory,
            colors,
        ));
        rows.push(ui::row(
            "Status",
            Some(self.index_description().into()),
            text_button(
                "rebuild-file-index",
                "Rebuild File Index",
                colors,
                cx.listener(|page, _, _, cx| page.rebuild(cx)),
            ),
            colors,
        ));
        vec![
            ui::section_title("Index", colors).into_any_element(),
            ui::card(rows, colors).into_any_element(),
        ]
    }
}

impl Render for FileSearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            self.focus(window, cx);
        }
        let colors = self.colors.clone();
        if let Some(problem) = &self.problem {
            return ui::problem("file-search", problem, &colors);
        }

        let levels = div().flex().gap(px(metrics::TAB_GAP)).children(
            [InRoot::Off, InRoot::Folders, InRoot::FoldersAndFiles].map(|level| {
                choice_button(
                    in_root_label(level),
                    self.config.in_root == level,
                    &colors,
                    cx.listener(move |page, _, _, cx| page.set_in_root(level, cx)),
                )
            }),
        );

        let focused = self.patterns.focus_handle(cx).is_focused(window);
        let (message, is_error) = match (&self.error, focused) {
            (Some(error), _) => (error.clone(), true),
            (None, true) => (
                "Enter saves the patterns, Esc undoes. Changing them walks the folders again."
                    .to_string(),
                false,
            ),
            (None, false) => (
                "Switches and folders are saved as you change them.".to_string(),
                false,
            ),
        };

        let where_ = self.render_where(cx);
        let what = self.render_what(window, cx);
        let index = self.render_index(window, cx);
        ui::page("file-search")
            .key_context(FORM_CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle))
            .child(ui::section_title("In root search", &colors))
            .child(ui::card(
                [ui::row(
                    "Show",
                    Some("What the root list finds in the index; Search Files finds all".into()),
                    levels,
                    &colors,
                )],
                &colors,
            ))
            .children(where_)
            .children(what)
            .children(index)
            .child(ui::note(message, is_error, &colors))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every choice is a value the configuration keeps as written, and the
    /// default is among them, so the page always shows one selected.
    #[test]
    fn the_hot_memory_choices_are_in_range_and_include_the_default() {
        let range = lumen_core::config::HOT_MEMORY;
        assert!(HOT_MEMORY_CHOICES.iter().all(|mb| range.contains(mb)));
        assert!(HOT_MEMORY_CHOICES.contains(&FileSearchConfig::default().hot_memory));
        assert!(HOT_MEMORY_CHOICES.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn coverage_is_told_only_when_contents_are_searched_and_known() {
        let base = "12 items";
        assert_eq!(
            with_coverage(base, Coverage::Covered, true),
            "12 items \u{b7} Contents searchable"
        );
        assert_eq!(
            with_coverage(base, Coverage::NotCovered, true),
            "12 items \u{b7} Contents not indexed by Windows"
        );
        assert_eq!(with_coverage(base, Coverage::Unknown, true), base);
        assert_eq!(with_coverage(base, Coverage::Covered, false), base);
        assert_eq!(with_coverage(base, Coverage::NotCovered, false), base);
    }
}
