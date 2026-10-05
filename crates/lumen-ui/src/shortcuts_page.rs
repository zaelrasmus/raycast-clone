//! The Shortcuts page of the settings window (F17), after Raycast's.
//!
//! Every command the root list indexes -- applications, Lumen's commands,
//! window layouts, quicklinks, presets, scripts, extensions -- one row each,
//! with its alias and its hotkey. The rows are the launcher's index, handed
//! over by the daemon when the window opens, so the page lists exactly what
//! root search finds and never builds a second catalogue.
//!
//! # The file stays the source of truth
//!
//! Each change is written to `shortcuts.json` at once, atomically
//! (`Shortcuts::save`), and the daemon's watch applies it: hotkeys registered
//! again, aliases given to the launcher. After every apply the daemon hands
//! back what it loaded and how each hotkey fared ([`ShortcutsPage::applied`]).
//! The page never re-reads the file because of its own save, so nothing being
//! typed is replaced by the echo; a table it did not write -- a hand edit --
//! is taken as it is.
//!
//! # Rows
//!
//! Thousands of applications on some machines, so the list is virtualised and
//! every row is one height (I-10). The alias field is text until it is
//! clicked; one field is made for the row being edited, not one per row.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, Focusable, SharedString, Subscription, UniformListScrollHandle, Window,
    div, img, px, svg, uniform_list,
};
use lumen_core::layout::settings as metrics;
use lumen_core::shortcuts::{Shortcuts, alias_input};
use lumen_core::theme::Colors;
use lumen_core::{Candidate, Item};
use lumen_platform::{HotkeyError, Platform};

use crate::hotkey_recorder::keycaps;
use crate::icons::IconCache;
use crate::quicklink_form::{CancelForm, NextField, SubmitForm};
use crate::settings::choice_button;
use crate::styling::color;
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

/// Which rows the page shows, besides what is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Hotkey,
    Alias,
}

impl Filter {
    const ALL: [Filter; 3] = [Filter::All, Filter::Hotkey, Filter::Alias];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Hotkey => "With Hotkey",
            Filter::Alias => "With Alias",
        }
    }
}

/// Whether the row for `item` is shown: the filter, then what is typed,
/// found in the title or the alias, case ignored. `query` is lowercase.
pub fn shows(item: &Item, shortcuts: &Shortcuts, query: &str, filter: Filter) -> bool {
    let alias = shortcuts.alias_of(&item.id);
    let kept = match filter {
        Filter::All => true,
        Filter::Hotkey => shortcuts.hotkey_of(&item.id).is_some(),
        Filter::Alias => alias.is_some(),
    };
    kept && (query.is_empty()
        || item.title.to_lowercase().contains(query)
        || alias.is_some_and(|alias| alias.contains(query)))
}

/// Whether the table the daemon loaded replaces the page's. Not when it is
/// the page's own, nor one the page wrote before it: the watch is behind,
/// and the newer save is on its way.
fn follows(loaded: &Shortcuts, current: &Shortcuts, written: &[Shortcuts]) -> bool {
    loaded != current && !written.contains(loaded)
}

/// How many of its own saves the page remembers, for [`follows`].
const WRITTEN_KEPT: usize = 8;

/// The alias being typed into, on one row.
struct AliasEdit {
    id: String,
    field: Entity<TextField>,
    error: Option<&'static str>,
    _changes: Subscription,
    _blur: Subscription,
}

pub struct ShortcutsPage {
    path: PathBuf,
    colors: Colors,
    icons: IconCache,
    /// The launcher's index, as the daemon handed it over.
    catalogue: Arc<Vec<Candidate>>,
    /// Each id's place in it, for naming another command.
    places: HashMap<String, usize>,
    /// The catalogue in title order.
    order: Vec<usize>,
    /// What the search and the filter leave of `order`.
    shown: Vec<usize>,
    /// The file as the page last read or wrote it.
    shortcuts: Shortcuts,
    /// The page's recent saves, oldest first, for [`follows`].
    written: Vec<Shortcuts>,
    /// Why a hotkey could not be registered, by id, at the daemon's last
    /// apply.
    failures: BTreeMap<String, HotkeyError>,
    search: Entity<TextField>,
    query: String,
    filter: Filter,
    alias: Option<AliasEdit>,
    scroll: UniformListScrollHandle,
    status: Option<SharedString>,
}

impl ShortcutsPage {
    pub fn new(
        path: PathBuf,
        platform: Arc<dyn Platform>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = {
            let colors = colors.clone();
            cx.new(|cx| TextField::search("Search commands\u{2026}", colors, cx))
        };
        cx.subscribe(&search, |page, _, event: &TextFieldEvent, cx| {
            if let TextFieldEvent::Changed(text) = event {
                page.query = text.trim().to_lowercase();
                page.refilter(cx);
            }
        })
        .detach();
        Self {
            path,
            icons: IconCache::new(platform, metrics::SHORTCUT_ICON_RESOLUTION),
            colors,
            catalogue: Arc::new(Vec::new()),
            places: HashMap::new(),
            order: Vec::new(),
            shown: Vec::new(),
            shortcuts: Shortcuts::default(),
            written: Vec::new(),
            failures: BTreeMap::new(),
            search,
            query: String::new(),
            filter: Filter::All,
            alias: None,
            scroll: UniformListScrollHandle::new(),
            status: None,
        }
    }

    /// Re-read `shortcuts.json`, off the UI thread: when the window opens.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { Shortcuts::load(&path) })
                .await;
            this.update(cx, |page, cx| {
                page.shortcuts = loaded;
                page.written.clear();
                page.refilter(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The launcher's index: every row the page can list.
    pub fn set_catalogue(&mut self, catalogue: Arc<Vec<Candidate>>, cx: &mut Context<Self>) {
        self.places = catalogue
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.item.id.clone(), index))
            .collect();
        let mut order: Vec<usize> = (0..catalogue.len()).collect();
        order.sort_by_cached_key(|&index| catalogue[index].item.title.to_lowercase());
        self.order = order;
        self.catalogue = catalogue;
        self.refilter(cx);
    }

    /// What the daemon loaded and registered, after every apply: the
    /// failures replace the last, and a table the page did not write is
    /// taken. An alias being typed is left as it is.
    pub fn applied(
        &mut self,
        loaded: Shortcuts,
        failures: BTreeMap<String, HotkeyError>,
        cx: &mut Context<Self>,
    ) {
        self.failures = failures;
        if loaded == self.shortcuts {
            self.written.clear();
        } else if follows(&loaded, &self.shortcuts, &self.written) {
            self.shortcuts = loaded;
            self.written.clear();
            self.refilter(cx);
        }
        cx.notify();
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.search
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        if let Some(edit) = &self.alias {
            edit.field
                .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.search.read(cx).focus(window);
    }

    pub fn status(&self) -> Option<SharedString> {
        self.status.clone()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let catalogue = &self.catalogue;
        self.shown = self
            .order
            .iter()
            .copied()
            .filter(|&index| {
                shows(
                    &catalogue[index].item,
                    &self.shortcuts,
                    &self.query,
                    self.filter,
                )
            })
            .collect();
        cx.notify();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.refilter(cx);
    }

    fn title_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.places
            .get(id)
            .map_or(id, |&index| self.catalogue[index].item.title.as_str())
    }

    /// Write the table, and remember it was this page's.
    fn write(&mut self, cx: &mut Context<Self>) {
        match self.shortcuts.save(&self.path) {
            Ok(()) => {
                self.written.push(self.shortcuts.clone());
                if self.written.len() > WRITTEN_KEPT {
                    self.written.remove(0);
                }
            }
            Err(error) => {
                log::warn!("could not save shortcuts: {error:#}");
                self.status = Some(format!("Could not save: {error:#}").into());
            }
        }
        cx.notify();
    }

    // -- Aliases ---------------------------------------------------------------

    fn edit_alias(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.alias.as_ref().is_some_and(|edit| edit.id == id) {
            return;
        }
        // Another row's edit is committed, or dropped if it cannot be.
        if self.alias.is_some() && !self.commit_alias(cx) {
            self.alias = None;
        }
        let text = self.shortcuts.alias_of(&id).unwrap_or_default().to_string();
        let colors = self.colors.clone();
        let field = cx.new(|cx| TextField::new(text, "Add Alias", colors, cx));
        let changes = cx.subscribe(&field, |page, _, event: &TextFieldEvent, cx| {
            if let (TextFieldEvent::Changed(_), Some(edit)) = (event, &mut page.alias) {
                edit.error = None;
                cx.notify();
            }
        });
        let handle = field.read(cx).focus_handle(cx);
        let blur = cx.on_blur(&handle, window, |page, _, cx| {
            page.commit_alias(cx);
        });
        window.focus(&handle);
        self.alias = Some(AliasEdit {
            id,
            field,
            error: None,
            _changes: changes,
            _blur: blur,
        });
        cx.notify();
    }

    /// Save the alias typed, or say why not and keep the field. Returns
    /// whether the edit closed.
    fn commit_alias(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = &mut self.alias else {
            return true;
        };
        let typed = edit.field.read(cx).text().to_string();
        match alias_input(&typed) {
            Err(why) => {
                edit.error = Some(why);
                cx.notify();
                false
            }
            Ok(alias) => {
                let id = edit.id.clone();
                self.alias = None;
                if self.shortcuts.alias_of(&id) != alias.as_deref() {
                    self.status = Some(match &alias {
                        Some(alias) => format!("{} is {alias}", self.title_of(&id)).into(),
                        None => format!("Removed the alias of {}", self.title_of(&id)).into(),
                    });
                    self.shortcuts.set_alias(&id, alias);
                    self.write(cx);
                    self.refilter(cx);
                }
                cx.notify();
                true
            }
        }
    }

    fn submit_alias(&mut self, _: &SubmitForm, window: &mut Window, cx: &mut Context<Self>) {
        if self.commit_alias(cx) {
            self.focus(window, cx);
        }
    }

    fn next_alias_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_alias(&SubmitForm, window, cx);
    }

    fn cancel_alias(&mut self, _: &CancelForm, window: &mut Window, cx: &mut Context<Self>) {
        self.alias = None;
        self.focus(window, cx);
        cx.notify();
    }

    // -- Hotkeys ---------------------------------------------------------------

    fn clear_hotkey(&mut self, id: &str, cx: &mut Context<Self>) {
        self.shortcuts.set_hotkey(id, None);
        self.failures.remove(id);
        self.status = Some(format!("Removed the hotkey of {}", self.title_of(id)).into());
        self.write(cx);
        self.refilter(cx);
    }

    /// What a row says under its title: what is wrong with it, or its kind.
    fn row_note(&self, item: &Item, conflicts: &BTreeMap<String, Vec<String>>) -> (String, bool) {
        if let Some(edit) = self.alias.as_ref().filter(|edit| edit.id == item.id)
            && let Some(why) = edit.error
        {
            return (why.to_string(), true);
        }
        if let (Some(error), Some(chord)) = (
            self.failures.get(&item.id),
            self.shortcuts.hotkey_of(&item.id),
        ) {
            let why = match error {
                HotkeyError::Taken => "is taken by another application".to_string(),
                other => other.to_string(),
            };
            return (format!("{chord} {why}"), true);
        }
        if let Some(alias) = self.shortcuts.alias_of(&item.id)
            && let Some(ids) = conflicts.get(alias)
        {
            let others: Vec<&str> = ids
                .iter()
                .filter(|id| **id != item.id)
                .map(|id| self.title_of(id))
                .collect();
            return (format!("Alias also used by {}", others.join(", ")), true);
        }
        (item.kind_label().to_string(), false)
    }
}

impl Render for ShortcutsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let filters = Filter::ALL.map(|filter| {
            choice_button(
                filter.label(),
                filter == self.filter,
                &colors,
                cx.listener(move |page, _, _, cx| page.set_filter(filter, cx)),
            )
        });
        let toolbar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(metrics::GAP))
            .child(
                div()
                    .flex_1()
                    .h(px(metrics::FIELD_HEIGHT))
                    .px(px(metrics::FIELD_PADDING))
                    .flex()
                    .items_center()
                    .gap(px(metrics::NAV_ICON_GAP))
                    .rounded(px(metrics::FIELD_RADIUS))
                    .bg(color(colors.surface_elevated))
                    .border_1()
                    .border_color(color(colors.border_subtle))
                    .text_size(px(metrics::text::FIELD))
                    .child(
                        svg()
                            .path(crate::icons::SEARCH)
                            .flex_none()
                            .size(px(metrics::ICON_SIZE))
                            .text_color(color(colors.text_muted)),
                    )
                    .child(self.search.clone()),
            )
            .children(filters);

        let header = div()
            .flex_none()
            .h(px(metrics::SECTION_TITLE_HEIGHT))
            .px(px(metrics::CARD_PADDING_X))
            .flex()
            .items_center()
            .gap(px(metrics::CARD_ROW_GAP))
            .text_size(px(metrics::text::HINT))
            .text_color(color(colors.text_muted))
            .child(
                div()
                    .flex_1()
                    .child(format!("{} commands", self.shown.len())),
            )
            .child(
                div()
                    .w(px(metrics::SHORTCUT_ALIAS_WIDTH))
                    .flex_none()
                    .child("Alias"),
            )
            .child(
                div()
                    .w(px(metrics::SHORTCUT_HOTKEY_WIDTH))
                    .flex_none()
                    .child("Hotkey"),
            );

        let count = self.shown.len();
        let list = uniform_list(
            "shortcuts",
            count,
            cx.processor(|page, range: std::ops::Range<usize>, _window, cx| {
                let conflicts = page.shortcuts.conflicts();
                range
                    .filter_map(|row| {
                        let index = *page.shown.get(row)?;
                        Some(page.render_row(index, &conflicts, cx))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .h_full();

        div()
            .size_full()
            .p(px(metrics::CONTENT_PADDING))
            .flex()
            .flex_col()
            .gap(px(metrics::CARD_GAP))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .rounded(px(metrics::CARD_RADIUS))
                    .bg(color(colors.surface_elevated))
                    .overflow_hidden()
                    .child(header)
                    .child(div().flex_1().min_h(px(0.0)).child(list)),
            )
    }
}

impl ShortcutsPage {
    fn render_row(
        &self,
        index: usize,
        conflicts: &BTreeMap<String, Vec<String>>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = &self.colors;
        let item = &self.catalogue[index].item;
        let id = item.id.clone();
        let (note, problem) = self.row_note(item, conflicts);
        let icon = item
            .icon
            .as_ref()
            .and_then(|source| self.icons.get(source, cx));

        // Reserved whether or not there is an icon, so titles line up (I-10).
        let square = div()
            .size(px(metrics::ICON_SIZE))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(match icon {
                Some(image) => img(image).size_full().into_any_element(),
                None => svg()
                    .path(crate::icons::glyph(item.kind))
                    .size_full()
                    .text_color(color(colors.icon))
                    .into_any_element(),
            });

        let text = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(metrics::CARD_ROW_LINE_GAP))
            .child(
                div()
                    .truncate()
                    .text_size(px(metrics::text::LABEL))
                    .text_color(color(colors.text_primary))
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(if problem {
                        colors.text_error
                    } else {
                        colors.text_muted
                    }))
                    .child(note),
            );

        div()
            .id(SharedString::from(format!("shortcut-{id}")))
            .h(px(metrics::SHORTCUT_ROW_HEIGHT))
            .px(px(metrics::CARD_PADDING_X))
            .flex()
            .items_center()
            .gap(px(metrics::CARD_ROW_GAP))
            .border_t_1()
            .border_color(color(colors.border_subtle))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap(px(metrics::ICON_GAP))
                    .child(square)
                    .child(text),
            )
            .child(self.render_alias(&id, cx))
            .child(self.render_hotkey(&id, cx))
            .into_any_element()
    }

    fn render_alias(&self, id: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.colors;
        let editing = self.alias.as_ref().filter(|edit| edit.id == id);
        let cell = div()
            .w(px(metrics::SHORTCUT_ALIAS_WIDTH))
            .flex_none()
            .text_size(px(metrics::text::FIELD));
        if let Some(edit) = editing {
            return cell
                .key_context(FORM_CONTEXT)
                .on_action(cx.listener(Self::submit_alias))
                .on_action(cx.listener(Self::next_alias_field))
                .on_action(cx.listener(Self::cancel_alias))
                .child(edit.field.clone())
                .into_any_element();
        }
        let alias = self.shortcuts.alias_of(id);
        let target = id.to_string();
        cell.child(
            field_box(colors)
                .id(SharedString::from(format!("alias-{id}")))
                .cursor_pointer()
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.edit_alias(target.clone(), window, cx)
                }))
                .text_color(color(if alias.is_some() {
                    colors.text_primary
                } else {
                    colors.text_muted
                }))
                .child(
                    div()
                        .truncate()
                        .child(alias.map_or_else(|| "Add Alias".to_string(), str::to_string)),
                ),
        )
        .into_any_element()
    }

    fn render_hotkey(&self, id: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.colors;
        let chord = self.shortcuts.hotkey_of(id);
        let failed = self.failures.contains_key(id);
        let target = id.to_string();
        let shown = match chord {
            Some(chord) => {
                let caps = crate::hotkey_recorder::Capture::Chord(chord.clone()).caps();
                keycaps(&caps, failed, colors).into_any_element()
            }
            None => div()
                .text_color(color(colors.text_muted))
                .child("Record Hotkey")
                .into_any_element(),
        };
        div()
            .w(px(metrics::SHORTCUT_HOTKEY_WIDTH))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(metrics::KEYCAP_GAP))
            .text_size(px(metrics::text::FIELD))
            .child(field_box(colors).flex_1().min_w(px(0.0)).child(shown))
            // Reserved whether or not there is a hotkey to clear (I-10).
            .child(
                div()
                    .id(SharedString::from(format!("clear-{id}")))
                    .size(px(metrics::SHORTCUT_CLEAR_SIZE))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(metrics::KEYCAP_RADIUS))
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .when(chord.is_some(), |clear| {
                        clear
                            .cursor_pointer()
                            .hover(|clear| clear.text_color(color(colors.text_primary)))
                            .on_click(
                                cx.listener(move |page, _, _, cx| page.clear_hotkey(&target, cx)),
                            )
                            .child("\u{2715}")
                    }),
            )
            .into_any_element()
    }
}

/// The box a field is drawn in, for a cell showing a value until clicked.
fn field_box(colors: &Colors) -> gpui::Div {
    div()
        .h(px(metrics::FIELD_HEIGHT))
        .px(px(metrics::FIELD_PADDING))
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(px(metrics::FIELD_RADIUS))
        .border_1()
        .border_color(color(colors.border_subtle))
        .bg(color(colors.surface_background))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::Kind;
    use lumen_core::shortcuts::CommandShortcut;

    fn item(id: &str, title: &str) -> Item {
        Item {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            actions: Vec::new(),
            kind: Kind::Command,
        }
    }

    fn shortcuts() -> Shortcuts {
        let mut shortcuts = Shortcuts::default();
        shortcuts.commands.insert(
            "window:left-half".into(),
            CommandShortcut {
                hotkey: Some("Ctrl+Alt+Left".parse().unwrap()),
                alias: Some("lh".into()),
            },
        );
        shortcuts.commands.insert(
            "app:code".into(),
            CommandShortcut {
                hotkey: None,
                alias: Some("vs".into()),
            },
        );
        shortcuts
    }

    #[test]
    fn the_filters_keep_the_rows_with_a_hotkey_or_an_alias() {
        let shortcuts = shortcuts();
        let left = item("window:left-half", "Left Half");
        let code = item("app:code", "Visual Studio Code");
        let notepad = item("app:notepad", "Notepad");
        let kept = |filter| {
            [&left, &code, &notepad]
                .into_iter()
                .filter(|item| shows(item, &shortcuts, "", filter))
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kept(Filter::All),
            ["window:left-half", "app:code", "app:notepad"]
        );
        assert_eq!(kept(Filter::Hotkey), ["window:left-half"]);
        assert_eq!(kept(Filter::Alias), ["window:left-half", "app:code"]);
    }

    #[test]
    fn a_search_finds_the_title_or_the_alias() {
        let shortcuts = shortcuts();
        let code = item("app:code", "Visual Studio Code");
        assert!(shows(&code, &shortcuts, "studio", Filter::All));
        assert!(shows(&code, &shortcuts, "vs", Filter::All));
        assert!(!shows(&code, &shortcuts, "notepad", Filter::All));
        assert!(!shows(&code, &shortcuts, "studio", Filter::Hotkey));
    }

    /// The echo of the page's own save never replaces what it holds; a
    /// hand edit does.
    #[test]
    fn only_a_table_the_page_did_not_write_is_taken() {
        let first = shortcuts();
        let mut second = first.clone();
        second.set_alias("app:code", None);
        let written = [first.clone(), second.clone()];

        assert!(!follows(&second, &second, &written));
        assert!(!follows(&first, &second, &written), "an older save of ours");

        let mut by_hand = second.clone();
        by_hand.set_alias("app:notepad", Some("np".into()));
        assert!(follows(&by_hand, &second, &written));
    }
}
