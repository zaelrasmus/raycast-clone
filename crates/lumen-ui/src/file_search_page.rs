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

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::config::{FileSearchConfig, InRoot, PreviewFont};
use lumen_core::file_index::{CEILING, IndexStatus, default_skips};
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
const MEMORY_NOTE: &str = "Every folder and drive you add is held in Lumen's memory to search it \
                           instantly. Add the folders you search, not whole drives you don't.";

/// The default skip that is not like the others, and says why.
const APPDATA: &str = "AppData";

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
        let flags = [Flag::Enabled, Flag::UseIgnoreFiles, Flag::IncludeHidden]
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
    /// it to `list` as `config.json` writes it.
    fn pick(&mut self, list: List, cx: &mut Context<Self>) {
        if self.picking || self.problem.is_some() {
            return;
        }
        self.picking = true;
        self.error = None;
        let platform = self.platform.clone();
        let home = self.home.clone();
        let path = self.path.clone();
        let title = match list {
            List::Scopes => "Add a Folder to Search",
            List::Exclusions => "Exclude a Folder",
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let Some(folder) = platform.pick_folder(title)? else {
                        return Ok(None);
                    };
                    let written = settings::written(&folder, home.as_deref());
                    let added = settings::add(&path, list, &written)?;
                    Ok::<_, anyhow::Error>(Some((written, added)))
                })
                .await;
            this.update(cx, |page, cx| {
                page.picking = false;
                match result {
                    Ok(None) => {}
                    Ok(Some((written, true))) => {
                        page.status = Some(format!("Added {written}").into());
                    }
                    Ok(Some((written, false))) => {
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
        let mut stops: Vec<FocusHandle> = [Flag::UseIgnoreFiles, Flag::IncludeHidden]
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
    }
}

fn flag_title(flag: Flag) -> &'static str {
    match flag {
        Flag::Enabled => "File index",
        Flag::UseIgnoreFiles => "Use ignore files",
        Flag::IncludeHidden => "Include hidden files",
    }
}

fn flag_description(flag: Flag) -> &'static str {
    match flag {
        Flag::Enabled => "Off drops the index and its watch; nothing is searched",
        Flag::UseIgnoreFiles => ".gitignore, .ignore and .lumenignore in the folders searched",
        Flag::IncludeHidden => "Names starting with a dot, and the hidden attribute",
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
    /// What a scope's row says under it: its count from the index.
    fn scope_description(&self, at: usize) -> SharedString {
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

    /// The index against the ceiling, and the scope it cut short.
    fn total_description(&self) -> String {
        let total = format!(
            "{} of {}",
            lumen_core::places::browse::grouped(self.index.entries),
            items(CEILING)
        );
        match &self.index.truncated {
            Some(cut) => format!(
                "{total}. Full: {} was cut short",
                settings::written(cut, self.home.as_deref())
            ),
            None => total,
        }
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
        if scopes.is_empty() {
            scopes.push(ui::row(
                "No folders",
                Some("Nothing is indexed until a folder is added".into()),
                div(),
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
                .child(MEMORY_NOTE)
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
        let mut rules: Vec<AnyElement> = [Flag::UseIgnoreFiles, Flag::IncludeHidden]
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
