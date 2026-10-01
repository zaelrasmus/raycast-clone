//! The settings window: shortcut sheets, quicklinks (F6), translation (F7)
//! and timers (F8).
//!
//! Four pages behind tabs in the header. The sheet editor is this file; the
//! others are entities of their own, in `quicklinks_page`, `translate_page`
//! and `timers_page`, hosted here.
//!
//! The overlay's whole point is that the shortcuts are *the user's own* (F3).
//! That was true before this window existed, but only in the sense that a JSON
//! file was true — the user could write one if they knew the schema, the folder
//! and the match-rule vocabulary. This window is what turns "configurable" into
//! something a person does rather than something a format permits.
//!
//! # Shape
//!
//! Two panes. The sheet list on the left, the selected sheet on the right:
//! its name, what it matches, and its groups of shortcuts. Every text control
//! is a [`TextField`], every structural change is a button, and nothing is
//! saved until the user says so.
//!
//! # Why edits are staged rather than written through
//!
//! The editor holds a working copy and writes it on save. Writing every
//! keystroke to disk would mean a half-typed match rule is briefly live, and the
//! overlay is watched for changes — so the user would see sheets flicker between
//! states while typing. Staging also makes "discard" possible, which a
//! write-through editor cannot offer at all.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Styled, UniformListScrollHandle,
    Window, actions, div, px, uniform_list,
};
use lumen_core::layout::settings as metrics;
use lumen_core::sheets::{Group, MatchRule, SCHEMA_VERSION, Sheet, Shortcut};
use lumen_core::theme::{Colors, Theme};

use crate::clipboard_page::ClipboardPage;
use crate::clock_page::ClockPage;
use crate::currency_page::CurrencyPage;
use crate::quicklinks_page::{PageEvent, QuicklinksPage};
use crate::styling::color;
use crate::text_field::{TextField, TextFieldEvent};
use crate::timers_page::{TimersPage, TimersPageEvent};
use crate::translate_page::TranslatePage;

/// Which page the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Sheets,
    Quicklinks,
    Translation,
    Timers,
    Currency,
    Clock,
    Clipboard,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::Sheets,
        Page::Quicklinks,
        Page::Translation,
        Page::Timers,
        Page::Currency,
        Page::Clock,
        Page::Clipboard,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Sheets => "Shortcut Sheets",
            Page::Quicklinks => "Quicklinks",
            Page::Translation => "Translation",
            Page::Timers => "Timers",
            Page::Currency => "Currency",
            Page::Clock => "World Clock",
            Page::Clipboard => "Clipboard History",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Page::Sheets => "Hold the gesture key to see these while you work",
            Page::Quicklinks => "Saved links and web searches, in quicklinks.json",
            Page::Translation => "The service and your presets, in config.json",
            Page::Timers => "Quick durations and your presets, in config.json",
            Page::Currency => "Where exchange rates come from, in config.json",
            Page::Clock => "How a place's time reads beside yours, in config.json",
            Page::Clipboard => "How much is kept, in config.json, and clearing it",
        }
    }
}

actions!(
    lumen_settings,
    [
        /// Write the edited sheet back to its file.
        SaveSheet,
        /// Throw away the working copy and reload from the library.
        DiscardChanges,
        /// Close the settings window.
        CloseSettings,
    ]
);

/// What the settings window asks the daemon to do.
#[derive(Debug, Clone)]
pub enum SettingsEvent {
    /// A sheet was written to `path`. The daemon reloads the library so the
    /// overlay reflects the change without a restart.
    Saved { path: PathBuf },
    /// A sheet's file was deleted.
    Deleted { path: PathBuf },
    /// The user closed the window.
    Closed,
    /// The quicklink icon cache was refetched, so the launcher should reindex
    /// to show it.
    QuicklinkIconsChanged,
    /// An alert sound was chosen on the Timers page: play it once (F8).
    PreviewSound(String),
}

impl gpui::EventEmitter<SettingsEvent> for Settings {}

/// One sheet as the editor holds it: the values, plus the fields showing them.
struct SheetDraft {
    /// Where it will be written. `None` for the generated Lumen sheet, which is
    /// shown for reference and cannot be edited.
    path: Option<PathBuf>,
    name: Entity<TextField>,
    match_kind: MatchKind,
    /// Comma-separated match values, as the user types them.
    match_values: Entity<TextField>,
    groups: Vec<GroupDraft>,
    /// Whether anything has been changed since the last load or save.
    dirty: bool,
}

struct GroupDraft {
    title: Entity<TextField>,
    shortcuts: Vec<ShortcutDraft>,
}

struct ShortcutDraft {
    keys: Entity<TextField>,
    description: Entity<TextField>,
}

/// The match rule as a choice, separate from its values.
///
/// Split because the two are edited differently -- one is a three-way switch,
/// the other is free text -- and because switching the kind must not discard
/// values the user has already typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Executable,
    TitleContains,
    Global,
}

impl MatchKind {
    const ALL: [MatchKind; 3] = [
        MatchKind::Executable,
        MatchKind::TitleContains,
        MatchKind::Global,
    ];

    fn label(self) -> &'static str {
        match self {
            MatchKind::Executable => "Application",
            MatchKind::TitleContains => "Window title",
            MatchKind::Global => "Everything",
        }
    }

    /// What the values field is asking for, in the user's terms.
    fn hint(self) -> &'static str {
        match self {
            MatchKind::Executable => "Programs, comma separated: code.exe, zed.exe",
            MatchKind::TitleContains => "Words in the window title: Mozilla Firefox",
            MatchKind::Global => "Shown whenever nothing more specific matches",
        }
    }

    fn of(rule: &MatchRule) -> Self {
        match rule {
            MatchRule::Executable { .. } => MatchKind::Executable,
            MatchRule::TitleContains { .. } => MatchKind::TitleContains,
            MatchRule::Global => MatchKind::Global,
        }
    }

    fn values_of(rule: &MatchRule) -> String {
        match rule {
            MatchRule::Executable { any } | MatchRule::TitleContains { any } => any.join(", "),
            MatchRule::Global => String::new(),
        }
    }

    fn build(self, values: &str) -> MatchRule {
        let any: Vec<String> = values
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        match self {
            MatchKind::Executable => MatchRule::Executable { any },
            MatchKind::TitleContains => MatchRule::TitleContains { any },
            MatchKind::Global => MatchRule::Global,
        }
    }
}

pub struct Settings {
    focus_handle: FocusHandle,
    theme: Theme,
    drafts: Vec<SheetDraft>,
    selected: usize,
    /// The directory new sheets are written into.
    sheets_dir: PathBuf,
    /// The application that was in front when settings opened, offered as a
    /// one-click way to start a sheet for it.
    ///
    /// The context is already captured for the overlay, so this costs nothing
    /// and removes the step where the user has to know their own executable's
    /// file name.
    foreground: Option<(String, String)>,
    status: Option<SharedString>,
    list_scroll: UniformListScrollHandle,
    page: Page,
    quicklinks: Entity<QuicklinksPage>,
    translation: Entity<TranslatePage>,
    timers: Entity<TimersPage>,
    currency: Entity<CurrencyPage>,
    clock: Entity<ClockPage>,
    clipboard: Entity<ClipboardPage>,
}

impl Settings {
    pub fn new(
        theme: Theme,
        sheets_dir: PathBuf,
        quicklinks_file: PathBuf,
        favicons: PathBuf,
        config_file: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let quicklinks = {
            let colors = theme.colors.clone();
            let host = focus_handle.clone();
            cx.new(|cx| QuicklinksPage::new(quicklinks_file, favicons, colors, host, cx))
        };
        cx.subscribe(&quicklinks, |_, _, event: &PageEvent, cx| match event {
            PageEvent::IconsRefreshed => cx.emit(SettingsEvent::QuicklinkIconsChanged),
        })
        .detach();
        // The footer and the dialog are drawn here, from the page's state.
        cx.observe(&quicklinks, |_, _, cx| cx.notify()).detach();

        let translation = {
            let colors = theme.colors.clone();
            let config_file = config_file.clone();
            cx.new(|cx| TranslatePage::new(config_file, colors, cx))
        };
        cx.observe(&translation, |_, _, cx| cx.notify()).detach();

        let currency = {
            let colors = theme.colors.clone();
            let config_file = config_file.clone();
            cx.new(|cx| CurrencyPage::new(config_file, colors, cx))
        };
        cx.observe(&currency, |_, _, cx| cx.notify()).detach();

        let clock = {
            let colors = theme.colors.clone();
            let config_file = config_file.clone();
            cx.new(|cx| ClockPage::new(config_file, colors, cx))
        };
        cx.observe(&clock, |_, _, cx| cx.notify()).detach();

        let clipboard = {
            let colors = theme.colors.clone();
            let config_file = config_file.clone();
            cx.new(|cx| ClipboardPage::new(config_file, colors, cx))
        };
        cx.observe(&clipboard, |_, _, cx| cx.notify()).detach();

        let timers = {
            let colors = theme.colors.clone();
            cx.new(|cx| TimersPage::new(config_file, colors, cx))
        };
        cx.observe(&timers, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&timers, |_, _, event: &TimersPageEvent, cx| match event {
            TimersPageEvent::Preview(name) => cx.emit(SettingsEvent::PreviewSound(name.clone())),
        })
        .detach();

        Self {
            focus_handle,
            theme,
            drafts: Vec::new(),
            selected: 0,
            sheets_dir,
            foreground: None,
            status: None,
            list_scroll: UniformListScrollHandle::new(),
            page: Page::Sheets,
            quicklinks,
            translation,
            timers,
            currency,
            clock,
            clipboard,
        }
    }

    /// Re-read the translate section of `config.json`: on open, and whenever
    /// the file changes.
    pub fn reload_translation(&mut self, cx: &mut Context<Self>) {
        self.translation.update(cx, |page, cx| page.reload(cx));
    }

    /// Re-read the timers section of `config.json`, for the same reasons.
    pub fn reload_timers(&mut self, cx: &mut Context<Self>) {
        self.timers.update(cx, |page, cx| page.reload(cx));
    }

    /// Re-read the currency section of `config.json`, for the same reasons.
    pub fn reload_currency(&mut self, cx: &mut Context<Self>) {
        self.currency.update(cx, |page, cx| page.reload(cx));
    }

    /// Re-read the clock section of `config.json`, for the same reasons.
    pub fn reload_clock(&mut self, cx: &mut Context<Self>) {
        self.clock.update(cx, |page, cx| page.reload(cx));
    }

    /// Re-read the clipboard section of `config.json`, for the same reasons,
    /// with the history to clear (F12).
    pub fn reload_clipboard(
        &mut self,
        history: Option<std::sync::Arc<parking_lot::Mutex<lumen_core::clipboard::History>>>,
        cx: &mut Context<Self>,
    ) {
        self.clipboard.update(cx, |page, cx| {
            page.use_history(history);
            page.reload(cx);
        });
    }

    /// Re-read `quicklinks.json`: on open, and whenever the file changes.
    pub fn reload_quicklinks(&mut self, cx: &mut Context<Self>) {
        self.quicklinks.update(cx, |page, cx| page.reload(cx));
    }

    /// Name the system's alert sounds, for the Timers page's dropdown.
    pub fn set_alert_sounds(&mut self, sounds: Vec<String>, cx: &mut Context<Self>) {
        self.timers
            .update(cx, |page, cx| page.set_sounds(sounds, cx));
    }

    /// Name the installed browsers, for the Quicklinks page's form.
    pub fn set_browsers(&mut self, browsers: Vec<String>, cx: &mut Context<Self>) {
        self.quicklinks
            .update(cx, |page, cx| page.set_browsers(browsers, cx));
    }

    fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.focus(window, cx);
        cx.notify();
    }

    /// Load the library into the editor, discarding any working copies.
    pub fn load(&mut self, entries: Vec<(Sheet, Option<PathBuf>)>, cx: &mut Context<Self>) {
        let colors = self.theme.colors.clone();
        self.drafts = entries
            .into_iter()
            .map(|(sheet, path)| draft_from(sheet, path, &colors, cx))
            .collect();
        self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
        self.status = None;
        cx.notify();
    }

    /// Tell the editor which application was in front, for the "add a sheet for
    /// what I was just using" affordance.
    pub fn set_foreground(&mut self, executable: Option<String>, title: String) {
        self.foreground = executable.map(|executable| (executable, title));
    }

    pub fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;

        // Each field owns its palette, so a theme change has to reach all of
        // them. Collected first because updating an entity needs `cx` mutably
        // while `self.drafts` is still borrowed.
        let colors = self.theme.colors.clone();
        let fields: Vec<Entity<TextField>> = self
            .drafts
            .iter()
            .flat_map(|draft| {
                std::iter::once(draft.name.clone())
                    .chain(std::iter::once(draft.match_values.clone()))
                    .chain(draft.groups.iter().flat_map(|group| {
                        std::iter::once(group.title.clone()).chain(
                            group
                                .shortcuts
                                .iter()
                                .flat_map(|s| [s.keys.clone(), s.description.clone()]),
                        )
                    }))
            })
            .collect();

        for field in fields {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.quicklinks
            .update(cx, |page, cx| page.set_colors(colors.clone(), cx));
        self.translation
            .update(cx, |page, cx| page.set_colors(colors.clone(), cx));
        self.timers
            .update(cx, |page, cx| page.set_colors(colors.clone(), cx));
        self.currency
            .update(cx, |page, cx| page.set_colors(colors.clone(), cx));
        self.clock
            .update(cx, |page, cx| page.set_colors(colors.clone(), cx));
        self.clipboard
            .update(cx, |page, cx| page.set_colors(colors, cx));

        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        match self.page {
            Page::Sheets => window.focus(&self.focus_handle),
            Page::Quicklinks => self.quicklinks.read(cx).focus(window, cx),
            Page::Translation => self.translation.read(cx).focus(window, cx),
            Page::Timers => self.timers.read(cx).focus(window, cx),
            Page::Currency => self.currency.read(cx).focus(window, cx),
            Page::Clock => self.clock.read(cx).focus(window, cx),
            Page::Clipboard => self.clipboard.read(cx).focus(window, cx),
        }
    }

    fn current(&self) -> Option<&SheetDraft> {
        self.drafts.get(self.selected)
    }

    /// Rebuild a `Sheet` from the fields of the selected draft.
    fn compose(&self, cx: &App) -> Option<Sheet> {
        let draft = self.current()?;

        let groups = draft
            .groups
            .iter()
            .map(|group| Group {
                title: group.title.read(cx).text().to_string(),
                shortcuts: group
                    .shortcuts
                    .iter()
                    .map(|shortcut| Shortcut {
                        keys: shortcut.keys.read(cx).text().to_string(),
                        description: shortcut.description.read(cx).text().to_string(),
                    })
                    // A row the user added and then left completely blank is
                    // noise in the file, not data. One with only a description
                    // is kept: they are mid-thought, not mistaken.
                    .filter(|shortcut| {
                        !shortcut.keys.trim().is_empty() || !shortcut.description.trim().is_empty()
                    })
                    .collect(),
            })
            .filter(|group: &Group| !group.title.trim().is_empty() || !group.shortcuts.is_empty())
            .collect();

        Some(Sheet {
            schema_version: SCHEMA_VERSION,
            name: draft.name.read(cx).text().to_string(),
            match_rule: draft.match_kind.build(draft.match_values.read(cx).text()),
            groups,
        })
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.dirty = true;
        }
        self.status = None;
        cx.notify();
    }

    fn save(&mut self, _: &SaveSheet, _: &mut Window, cx: &mut Context<Self>) {
        // One save key for the window: on the Quicklinks page it saves the
        // link, as Enter does.
        if self.page == Page::Quicklinks {
            self.quicklinks.update(cx, |page, cx| page.save(cx));
            return;
        }
        if self.page == Page::Translation {
            self.translation.update(cx, |page, cx| page.save(cx));
            return;
        }
        if self.page == Page::Timers {
            self.timers.update(cx, |page, cx| page.save(cx));
            return;
        }
        if self.page == Page::Currency {
            self.currency.update(cx, |page, cx| page.save(cx));
            return;
        }
        if self.page == Page::Clock {
            self.clock.update(cx, |page, cx| page.save(cx));
            return;
        }
        if self.page == Page::Clipboard {
            self.clipboard.update(cx, |page, cx| page.save(cx));
            return;
        }
        let Some(sheet) = self.compose(cx) else {
            return;
        };

        // A sheet keyed on an application with no application named matches
        // nothing at all. It saves, it appears in this list, it looks finished
        // -- and it never shows up, with no way to tell from here why.
        //
        // This is not hypothetical: it is how a Global sheet became invisible
        // after somebody clicked "Application" while exploring and saved. The
        // failure is silent, permanent, and indistinguishable from the overlay
        // being broken.
        if let Some(problem) = unmatchable(&sheet.match_rule) {
            self.status = Some(problem.into());
            cx.notify();
            return;
        };
        let Some(draft) = self.drafts.get(self.selected) else {
            return;
        };
        let Some(path) = draft.path.clone() else {
            self.status =
                Some("This sheet is generated from Lumen's keymap and cannot be edited".into());
            cx.notify();
            return;
        };

        match sheet.save(&path) {
            Ok(()) => {
                if let Some(draft) = self.drafts.get_mut(self.selected) {
                    draft.dirty = false;
                }
                self.status = Some(format!("Saved to {}", path.display()).into());
                cx.emit(SettingsEvent::Saved { path });
            }
            Err(error) => {
                log::error!("could not save the sheet: {error:#}");
                self.status = Some(format!("Could not save: {error}").into());
            }
        }
        cx.notify();
    }

    fn discard(&mut self, _: &DiscardChanges, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Closed);
        self.status = Some("Reopen settings to reload from disk".into());
        cx.notify();
    }

    fn close(&mut self, _: &CloseSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Closed);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.status = None;
        cx.notify();
    }

    /// Start a new sheet, either blank or keyed on the application that was in
    /// front when settings opened.
    fn add_sheet(&mut self, seed: Option<(String, String)>, cx: &mut Context<Self>) {
        let sheet = match seed {
            Some((executable, title)) => {
                let name = title_to_name(&title, &executable);
                Sheet::for_executable(&name, &executable)
            }
            None => Sheet::for_executable("New Sheet", ""),
        };

        let path = self.sheets_dir.join(format!("{}.json", sheet.file_stem()));
        let colors = self.theme.colors.clone();
        let draft = draft_from(sheet, Some(path), &colors, cx);

        self.drafts.push(draft);
        self.selected = self.drafts.len() - 1;
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.dirty = true;
        }
        self.status = Some("Fill the sheet in, then press Ctrl+S".into());
        cx.notify();
    }

    fn delete_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.drafts.get(self.selected) else {
            return;
        };
        let Some(path) = draft.path.clone() else {
            return;
        };

        // Removing the file is what actually deletes the sheet; the draft is
        // just the editor's copy of it.
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("could not delete {}: {error}", path.display());
            self.status = Some(format!("Could not delete: {error}").into());
            cx.notify();
            return;
        }

        self.drafts.remove(self.selected);
        self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
        self.status = Some("Sheet deleted".into());
        cx.emit(SettingsEvent::Deleted { path });
        cx.notify();
    }

    fn add_group(&mut self, cx: &mut Context<Self>) {
        let title = new_field("Group name", "", self.theme.colors.clone(), cx);
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.groups.push(GroupDraft {
                title,
                shortcuts: Vec::new(),
            });
            draft.dirty = true;
        }
        cx.notify();
    }

    fn remove_group(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(draft) = self.drafts.get_mut(self.selected)
            && index < draft.groups.len()
        {
            draft.groups.remove(index);
            draft.dirty = true;
        }
        cx.notify();
    }

    fn add_shortcut(&mut self, group: usize, cx: &mut Context<Self>) {
        let keys = new_field("Ctrl+K", "", self.theme.colors.clone(), cx);
        let description = new_field("What it does", "", self.theme.colors.clone(), cx);

        if let Some(draft) = self.drafts.get_mut(self.selected)
            && let Some(group) = draft.groups.get_mut(group)
        {
            group.shortcuts.push(ShortcutDraft { keys, description });
        }
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.dirty = true;
        }
        cx.notify();
    }

    fn remove_shortcut(&mut self, group: usize, index: usize, cx: &mut Context<Self>) {
        if let Some(draft) = self.drafts.get_mut(self.selected)
            && let Some(group) = draft.groups.get_mut(group)
            && index < group.shortcuts.len()
        {
            group.shortcuts.remove(index);
        }
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.dirty = true;
        }
        cx.notify();
    }

    fn set_match_kind(&mut self, kind: MatchKind, cx: &mut Context<Self>) {
        if let Some(draft) = self.drafts.get_mut(self.selected) {
            draft.match_kind = kind;
            draft.dirty = true;
        }
        cx.notify();
    }
}

/// Why a rule would never match anything, if it would not.
///
/// Returned as the sentence to show rather than a bool, because the useful
/// half is telling the user which box to fill in.
fn unmatchable(rule: &MatchRule) -> Option<&'static str> {
    match rule {
        MatchRule::Executable { any } if any.is_empty() => {
            Some("Name at least one program, or choose Everything")
        }
        MatchRule::TitleContains { any } if any.is_empty() => {
            Some("Name at least one word from the window title, or choose Everything")
        }
        _ => None,
    }
}

/// Build a draft and subscribe to every field it contains.
///
/// Subscribing here rather than at each call site is what makes the dirty flag
/// reliable: a field added later cannot be forgotten, because there is only one
/// place a field is ever created.
fn draft_from(
    sheet: Sheet,
    path: Option<PathBuf>,
    colors: &Colors,
    cx: &mut Context<Settings>,
) -> SheetDraft {
    let match_kind = MatchKind::of(&sheet.match_rule);
    let match_values = MatchKind::values_of(&sheet.match_rule);

    SheetDraft {
        path,
        name: new_field("Sheet name", &sheet.name, colors.clone(), cx),
        match_kind,
        match_values: new_field("code.exe, zed.exe", &match_values, colors.clone(), cx),
        groups: sheet
            .groups
            .into_iter()
            .map(|group| GroupDraft {
                title: new_field("Group name", &group.title, colors.clone(), cx),
                shortcuts: group
                    .shortcuts
                    .into_iter()
                    .map(|shortcut| ShortcutDraft {
                        keys: new_field("Ctrl+K", &shortcut.keys, colors.clone(), cx),
                        description: new_field(
                            "What it does",
                            &shortcut.description,
                            colors.clone(),
                            cx,
                        ),
                    })
                    .collect(),
            })
            .collect(),
        dirty: false,
    }
}

fn new_field(
    placeholder: &'static str,
    text: &str,
    colors: Colors,
    cx: &mut Context<Settings>,
) -> Entity<TextField> {
    let field = cx.new(|cx| TextField::new(text, placeholder, colors, cx));

    cx.subscribe(&field, |settings, _field, _: &TextFieldEvent, cx| {
        settings.mark_dirty(cx);
    })
    .detach();

    field
}

/// A readable sheet name from a window title, falling back to the executable.
///
/// Window titles are usually `document - Application`, so the last segment is
/// the application's own name and the rest is whatever happened to be open.
fn title_to_name(title: &str, executable: &str) -> String {
    let candidate = title
        .rsplit(&['-', '—', '|'][..])
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or("");

    if candidate.is_empty() || candidate.len() > 40 {
        return executable.trim_end_matches(".exe").to_string();
    }
    candidate.to_string()
}

impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();

        let mut root = div()
            .key_context("Settings")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::discard))
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(metrics::RADIUS))
            .border_1()
            .border_color(color(colors.border_subtle))
            .bg(color(colors.surface_background))
            .text_color(color(colors.text_primary));

        if let Some(family) = &self.theme.font_family {
            root = root.font_family(SharedString::from(family.clone()));
        }

        let body = match self.page {
            Page::Sheets => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.render_sidebar(&colors, cx))
                .child(self.render_editor(&colors, cx))
                .into_any_element(),
            Page::Quicklinks => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.quicklinks.clone())
                .into_any_element(),
            Page::Translation => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.translation.clone())
                .into_any_element(),
            Page::Timers => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.timers.clone())
                .into_any_element(),
            Page::Currency => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.currency.clone())
                .into_any_element(),
            Page::Clock => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.clock.clone())
                .into_any_element(),
            Page::Clipboard => div()
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(self.clipboard.clone())
                .into_any_element(),
        };
        let dialog = match self.page {
            Page::Translation => self
                .translation
                .update(cx, |page, cx| page.render_dialog(cx)),
            Page::Timers => self.timers.update(cx, |page, cx| page.render_dialog(cx)),
            Page::Clipboard => self.clipboard.update(cx, |page, cx| page.render_dialog(cx)),
            _ => self
                .quicklinks
                .update(cx, |page, cx| page.render_dialog(cx)),
        };

        root.relative()
            .child(self.render_header(&colors, cx))
            .child(body)
            .child(self.render_footer(&colors, cx))
            .children(dialog)
    }
}

impl Settings {
    fn render_header(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.page;
        div()
            .h(px(metrics::HEADER_HEIGHT))
            .px(px(metrics::PADDING))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(color(colors.border_subtle))
            .child(
                div()
                    .flex()
                    .gap(px(metrics::TAB_GAP))
                    .children(Page::ALL.map(|page| {
                        choice_button(
                            page.label(),
                            page == current,
                            colors,
                            cx.listener(move |this, _event, window, cx| {
                                this.show_page(page, window, cx)
                            }),
                        )
                    })),
            )
            .child(
                div()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .child(current.hint()),
            )
    }

    fn render_sidebar(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.drafts.len();
        let selected = self.selected;
        let entity = cx.entity();
        let names: Vec<(SharedString, bool, bool)> = self
            .drafts
            .iter()
            .map(|draft| {
                (
                    SharedString::from(draft.name.read(cx).text().to_string()),
                    draft.path.is_some(),
                    draft.dirty,
                )
            })
            .collect();

        let seed = self.foreground.clone();
        let deletable = self.current().is_some_and(|draft| draft.path.is_some());

        div()
            .w(px(metrics::SIDEBAR_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(color(colors.border_subtle))
            .child(
                div().flex_1().overflow_hidden().child(
                    uniform_list(
                        "sheets",
                        count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, _cx| {
                            range
                                .map(|index| {
                                    let (name, editable, dirty) = names
                                        .get(index)
                                        .cloned()
                                        .unwrap_or_else(|| ("".into(), false, false));
                                    this.render_sidebar_row(
                                        index,
                                        name,
                                        editable,
                                        dirty,
                                        index == selected,
                                        &entity,
                                    )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(self.list_scroll.clone())
                    .h_full(),
                ),
            )
            .child(
                div()
                    .p(px(metrics::GAP))
                    .flex()
                    .flex_col()
                    .gap(px(metrics::GAP))
                    .border_t_1()
                    .border_color(color(colors.border_subtle))
                    .child(text_button(
                        "new-sheet",
                        "New sheet",
                        colors,
                        cx.listener(|this, _event, _window, cx| this.add_sheet(None, cx)),
                    ))
                    // Only for sheets that came from a file: the generated one
                    // has nothing to delete.
                    .when(deletable, |column| {
                        column.child(text_button(
                            "delete-sheet",
                            "Delete this sheet",
                            colors,
                            cx.listener(|this, _event, _window, cx| this.delete_sheet(cx)),
                        ))
                    })
                    // Offered only when there is something to seed from, because
                    // a button that sometimes does nothing is worse than one
                    // that is sometimes absent.
                    .when_some(seed, |column, (executable, title)| {
                        column.child(text_button(
                            "seed-sheet",
                            format!("New sheet for {executable}"),
                            colors,
                            cx.listener(move |this, _event, _window, cx| {
                                this.add_sheet(Some((executable.clone(), title.clone())), cx)
                            }),
                        ))
                    }),
            )
    }

    fn render_sidebar_row(
        &self,
        index: usize,
        name: SharedString,
        editable: bool,
        dirty: bool,
        selected: bool,
        entity: &Entity<Self>,
    ) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let entity = entity.clone();

        div()
            .id(index)
            .h(px(metrics::SIDEBAR_ROW_HEIGHT))
            .px(px(metrics::PADDING))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(metrics::GAP))
            .text_size(px(metrics::text::LABEL))
            .when(selected, |row| {
                row.bg(color(colors.selection_background))
                    .text_color(color(colors.text_primary))
            })
            .when(!selected, |row| row.text_color(color(colors.text_muted)))
            .on_click(move |_event, _window, cx| {
                entity.update(cx, |this, cx| this.select(index, cx));
            })
            .child(div().overflow_hidden().child(name))
            .child(
                div()
                    .flex_none()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    // "Generated" explains why the sheet cannot be edited before
                    // the user clicks it and finds out.
                    .child(if !editable {
                        "generated"
                    } else if dirty {
                        "edited"
                    } else {
                        ""
                    }),
            )
            .into_any_element()
    }

    fn render_editor(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(draft) = self.current() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(metrics::text::LABEL))
                .text_color(color(colors.text_muted))
                .child("No sheets yet. Create one on the left.")
                .into_any_element();
        };

        let editable = draft.path.is_some();
        let kind = draft.match_kind;

        // Built eagerly rather than inside `.children(..)`: that takes a lazy
        // iterator, and a closure capturing `cx` cannot outlive this call.
        let mut groups = Vec::with_capacity(draft.groups.len());
        for (index, group) in draft.groups.iter().enumerate() {
            groups.push(self.render_group(index, group, colors, cx));
        }

        div()
            .flex_1()
            .p(px(metrics::PADDING))
            .flex()
            .flex_col()
            .gap(px(metrics::GAP))
            .overflow_hidden()
            .child(section_label("Name", colors))
            .child(draft.name.clone())
            .child(section_label("Shown for", colors))
            .child(
                div()
                    .flex()
                    .gap(px(metrics::GAP))
                    .children(MatchKind::ALL.map(|option| {
                        choice_button(
                            option.label(),
                            option == kind,
                            colors,
                            cx.listener(move |this, _event, _window, cx| {
                                this.set_match_kind(option, cx)
                            }),
                        )
                    })),
            )
            .when(kind != MatchKind::Global, |editor| {
                editor.child(draft.match_values.clone())
            })
            .child(
                div()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .child(kind.hint()),
            )
            .child(section_label("Shortcuts", colors))
            .child(
                div()
                    .id("groups")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(metrics::GROUP_GAP))
                    .children(groups)
                    .child(div().pt(px(metrics::GAP)).child(text_button(
                        "add-group",
                        "Add group",
                        colors,
                        cx.listener(|this, _event, _window, cx| this.add_group(cx)),
                    ))),
            )
            .when(!editable, |editor| {
                editor.child(
                    div()
                        .text_size(px(metrics::text::HINT))
                        .text_color(color(colors.text_muted))
                        .child(
                            "This sheet is generated from Lumen's own keymap, so it always \
                             matches the keys that are actually bound.",
                        ),
                )
            })
            .into_any_element()
    }

    /// Returns `AnyElement` rather than `impl IntoElement` because the 2024
    /// capture rules would otherwise tie the returned element to the borrow of
    /// `cx`, and the caller builds one of these per group in a loop.
    fn render_group(
        &self,
        index: usize,
        group: &GroupDraft,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id(SharedString::from(format!("group-{index}")))
            .flex()
            .flex_col()
            .gap(px(metrics::GAP))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(metrics::GAP))
                    .child(div().flex_1().child(group.title.clone()))
                    .child(icon_button(
                        SharedString::from(format!("remove-group-{index}")),
                        "−",
                        colors,
                        cx.listener(move |this, _event, _window, cx| this.remove_group(index, cx)),
                    )),
            )
            .children(group.shortcuts.iter().enumerate().map(|(row, shortcut)| {
                div()
                    .id(SharedString::from(format!("shortcut-{index}-{row}")))
                    .h(px(metrics::ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(metrics::GAP))
                    .child(
                        div()
                            .w(px(metrics::KEYS_FIELD_WIDTH))
                            .flex_none()
                            .child(shortcut.keys.clone()),
                    )
                    .child(div().flex_1().child(shortcut.description.clone()))
                    .child(icon_button(
                        SharedString::from(format!("remove-shortcut-{index}-{row}")),
                        "−",
                        colors,
                        cx.listener(move |this, _event, _window, cx| {
                            this.remove_shortcut(index, row, cx)
                        }),
                    ))
            }))
            .child(text_button(
                SharedString::from(format!("add-shortcut-{index}")),
                "Add shortcut",
                colors,
                cx.listener(move |this, _event, _window, cx| this.add_shortcut(index, cx)),
            ))
            .into_any_element()
    }

    fn render_footer(&self, colors: &Colors, cx: &App) -> impl IntoElement {
        let (status, dirty) = match self.page {
            Page::Sheets => (
                self.status.clone(),
                self.current().is_some_and(|draft| draft.dirty),
            ),
            Page::Quicklinks => (self.quicklinks.read(cx).status(cx), false),
            Page::Translation => (self.translation.read(cx).status(), false),
            Page::Timers => (self.timers.read(cx).status(cx), false),
            Page::Currency => (self.currency.read(cx).status(), false),
            Page::Clock => (self.clock.read(cx).status(), false),
            Page::Clipboard => (self.clipboard.read(cx).status(), false),
        };

        div()
            .h(px(metrics::FOOTER_HEIGHT))
            .px(px(metrics::PADDING))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(color(colors.border_subtle))
            .text_size(px(metrics::text::HINT))
            .text_color(color(colors.text_muted))
            .child(div().child(match (status, dirty) {
                (Some(status), _) => status,
                (None, true) => "Unsaved changes".into(),
                (None, false) => SharedString::from(""),
            }))
            .child(
                div()
                    .flex()
                    .gap(px(metrics::GROUP_GAP))
                    .child("Save  Ctrl+S")
                    .child("Close  Esc"),
            )
    }
}

fn section_label(text: &'static str, colors: &Colors) -> impl IntoElement {
    div()
        .pt(px(metrics::GAP))
        .text_size(px(metrics::text::SECTION))
        .text_color(color(colors.accent))
        .child(text.to_uppercase())
}

/// A button.
///
/// The id is a parameter rather than derived from the label, and that is the
/// whole bug this signature exists to prevent. GPUI identifies an interactive
/// element by a path of ids, and two siblings sharing one do not both work --
/// they share the hitbox bookkeeping, and clicks land on one of them or on
/// neither. Deriving the id from the label looked tidy and meant every group's
/// "Add shortcut" button was the same element as every other group's, and
/// every "−" was the same element as every other "−".
///
/// So the caller passes something that includes the position. It reads as
/// noise until the day somebody renders two of these in a loop, which in an
/// editor for a list of groups of shortcuts is immediately.
pub(crate) fn text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    colors: &Colors,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();

    div()
        .id(id)
        .h(px(metrics::FIELD_HEIGHT))
        .px(px(metrics::FIELD_PADDING))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(metrics::FIELD_RADIUS))
        .border_1()
        .border_color(color(colors.border_subtle))
        .bg(color(colors.surface_elevated))
        .text_size(px(metrics::text::LABEL))
        .text_color(color(colors.text_primary))
        .cursor_pointer()
        .on_click(on_click)
        .child(label)
}

pub(crate) fn choice_button(
    label: &'static str,
    selected: bool,
    colors: &Colors,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .h(px(metrics::FIELD_HEIGHT))
        .px(px(metrics::FIELD_PADDING))
        .flex()
        .items_center()
        .rounded(px(metrics::FIELD_RADIUS))
        .border_1()
        .border_color(color(if selected {
            colors.accent
        } else {
            colors.border_subtle
        }))
        .bg(color(if selected {
            colors.selection_background
        } else {
            colors.surface_elevated
        }))
        .text_size(px(metrics::text::LABEL))
        .text_color(color(if selected {
            colors.text_primary
        } else {
            colors.text_muted
        }))
        .cursor_pointer()
        .on_click(on_click)
        .child(label)
}

/// A small square button. See [`text_button`] for why the id is a parameter.
fn icon_button(
    id: impl Into<gpui::ElementId>,
    glyph: &'static str,
    colors: &Colors,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(metrics::BUTTON_SIZE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(metrics::FIELD_RADIUS))
        .border_1()
        .border_color(color(colors.border_subtle))
        .text_size(px(metrics::text::LABEL))
        .text_color(color(colors.text_muted))
        .cursor_pointer()
        .on_click(on_click)
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_rule_round_trips_through_the_editor_fields() {
        let rule = MatchRule::Executable {
            any: vec!["code.exe".into(), "zed.exe".into()],
        };

        let kind = MatchKind::of(&rule);
        let values = MatchKind::values_of(&rule);

        assert_eq!(kind, MatchKind::Executable);
        assert_eq!(values, "code.exe, zed.exe");
        assert_eq!(kind.build(&values), rule);
    }

    #[test]
    fn match_values_tolerate_the_way_people_actually_type_lists() {
        let rule = MatchKind::Executable.build("  code.exe ,, zed.exe ,  ");
        assert_eq!(
            rule,
            MatchRule::Executable {
                any: vec!["code.exe".into(), "zed.exe".into()]
            }
        );
    }

    /// The guard that stops a sheet being saved into invisibility.
    #[test]
    fn a_rule_with_nothing_to_match_on_is_refused() {
        assert!(unmatchable(&MatchRule::Executable { any: Vec::new() }).is_some());
        assert!(unmatchable(&MatchRule::TitleContains { any: Vec::new() }).is_some());
    }

    #[test]
    fn a_rule_that_can_match_is_allowed() {
        assert!(
            unmatchable(&MatchRule::Executable {
                any: vec!["zed.exe".into()]
            })
            .is_none()
        );
        // Global matches by definition and has no values to check.
        assert!(unmatchable(&MatchRule::Global).is_none());
    }

    #[test]
    fn the_global_rule_ignores_its_values() {
        assert_eq!(MatchKind::Global.build("anything"), MatchRule::Global);
        assert_eq!(MatchKind::values_of(&MatchRule::Global), "");
    }

    #[test]
    fn every_match_kind_has_a_label_and_a_hint() {
        for kind in MatchKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(!kind.hint().is_empty());
        }
    }

    #[test]
    fn a_window_title_becomes_a_readable_sheet_name() {
        assert_eq!(title_to_name("main.rs — Zed", "zed.exe"), "Zed");
        assert_eq!(
            title_to_name("README.md - Visual Studio Code", "code.exe"),
            "Visual Studio Code"
        );
    }

    /// A title with nothing useful in it must not produce a nonsense name.
    #[test]
    fn an_unhelpful_title_falls_back_to_the_executable() {
        assert_eq!(title_to_name("", "zed.exe"), "zed");
        assert_eq!(title_to_name("---", "zed.exe"), "zed");
    }

    /// Some applications put the whole document path in the title; that is not
    /// a name anybody wants in their sheet list.
    #[test]
    fn an_absurdly_long_title_falls_back_to_the_executable() {
        let long = "a".repeat(80);
        assert_eq!(title_to_name(&long, "thing.exe"), "thing");
    }
}
