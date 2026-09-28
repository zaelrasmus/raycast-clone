//! The Quicklinks page of the settings window (F6).
//!
//! Every link side by side: add, edit, delete, and the fallback flag, which
//! the launcher's form leaves out. The editing is the launcher's form, not a
//! second one; this page is the list around it and the question before a
//! delete.
//!
//! # The file stays the source of truth
//!
//! The page holds a copy of `quicklinks.json` for its list and re-reads it
//! whenever the file changes, whoever changed it. It never writes that copy
//! back: every save and delete goes through `store`, which reads the file
//! afresh and finds the link by name. A hand edit made while this window is
//! open is kept, not overwritten by a stale list.
//!
//! # Why a delete asks first
//!
//! Every other change here can be undone by editing it back. A deleted link is
//! gone unless the user remembers its URL, so it gets a dialog, with Enter and
//! Escape bound like any other (I-14).
//!
//! # Keys
//!
//! The list and the form each have focus of their own, because Up and Down
//! mean different things in each: the next link in the list, the next field
//! in the form. Enter or Tab goes from the list into the form; Escape comes
//! back out, then closes the window.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, ScrollHandle, SharedString, Subscription,
    Window, actions, div, px,
};
use lumen_core::layout::settings as metrics;
use lumen_core::providers::quicklinks::{Entry, store};
use lumen_core::theme::Colors;

use crate::quicklink_form::{FormEvent, QuicklinkForm};
use crate::settings::text_button;
use crate::styling::color;

actions!(
    lumen_dialog,
    [
        /// Do what the dialog asks about.
        ConfirmDialog,
        /// Close the dialog without doing it.
        CancelDialog,
    ]
);

actions!(
    lumen_links,
    [
        /// Select the link above.
        PreviousLink,
        /// Select the link below.
        NextLink,
        /// Go from the list into the selected link's form.
        EditLink,
    ]
);

/// The confirmation dialog's key context.
pub const DIALOG_CONTEXT: &str = "Dialog";

/// The list of links' key context.
pub const LIST_CONTEXT: &str = "QuicklinkList";

/// Where focus goes at the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refocus {
    List,
    Form,
}

pub struct QuicklinksPage {
    path: PathBuf,
    colors: Colors,
    /// The links as the file last read had them.
    entries: Vec<Entry>,
    /// Why the file could not be read. The list is empty while this is set,
    /// and nothing can be saved until the file is fixed (`store::read`).
    problem: Option<String>,
    /// The link being edited or created.
    open: Option<Open>,
    /// The link waiting on the delete dialog.
    confirming: Option<String>,
    dialog_focus: FocusHandle,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    /// Where focus goes when the page has nothing to hold it.
    host_focus: FocusHandle,
    browsers: Vec<String>,
    status: Option<SharedString>,
    /// Why the last move was refused. Above "Unsaved changes", which is the
    /// reason for it, and gone once the form is saved, undone or left.
    warning: Option<SharedString>,
    /// Focus to move at the next frame. For the paths that replace the form
    /// without a `Window` to focus with, such as a reload.
    pending_focus: Option<Refocus>,
    /// Set `pending_focus` once the next reload has settled the form.
    refocus: Option<Refocus>,
}

struct Open {
    form: Entity<QuicklinkForm>,
    _events: Subscription,
}

impl QuicklinksPage {
    pub fn new(
        path: PathBuf,
        colors: Colors,
        host_focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            path,
            colors,
            entries: Vec::new(),
            problem: None,
            open: None,
            confirming: None,
            dialog_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            host_focus,
            browsers: Vec::new(),
            status: None,
            warning: None,
            pending_focus: None,
            refocus: None,
        }
    }

    /// Re-read the file, off the UI thread, and bring the list and the form
    /// up to date with it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { store::read(&path) })
                .await;
            this.update(cx, |page, cx| {
                match read {
                    Ok(file) => {
                        page.entries = file.quicklinks;
                        page.problem = None;
                    }
                    Err(error) => {
                        log::warn!("the quicklinks page cannot read the file: {error:#}");
                        page.entries.clear();
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                page.reconcile(cx);
                if let Some(target) = page.refocus.take() {
                    page.pending_focus = Some(target);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Make the open form agree with the file just read. See [`follow`].
    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let showing = self.open.as_ref().map(|open| {
            let form = open.form.read(cx);
            Showing {
                name: form.editing_name().map(str::to_string),
                original: form.original().clone(),
                edited: form.is_edited(),
            }
        });
        match follow(&self.entries, showing) {
            Follow::Keep => {}
            Follow::Show(entry) => self.show(Some(entry), cx),
            Follow::Clear => self.open = None,
        }
    }

    fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.name.trim() == name.trim())
    }

    /// Open the form on `entry`, or on a new link when that is `None`.
    fn show(&mut self, entry: Option<Entry>, cx: &mut Context<Self>) {
        self.warning = None;
        let path = self.path.clone();
        let colors = self.colors.clone();
        let browsers = self.browsers.clone();
        let form = cx.new(|cx| {
            let mut form = QuicklinkForm::new(path, entry, colors, cx);
            form.offer_fallback(cx);
            form.set_browsers(browsers, cx);
            form
        });
        let events = cx.subscribe(&form, |page, _, event, cx| match event {
            FormEvent::Saved(name) => {
                page.warning = None;
                page.status = Some(format!("Saved {name}").into());
                page.reload(cx);
            }
            FormEvent::Cancelled => page.cancel(cx),
        });
        self.open = Some(Open {
            form,
            _events: events,
        });
        cx.notify();
    }

    /// Escape in the form: undo what was typed, or back to the list if there
    /// is nothing to undo.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.warning = None;
        let Some(open) = &self.open else {
            return;
        };
        let form = open.form.read(cx);
        if !form.is_edited() {
            // Leaving a new link's empty form shows what was selected before.
            if form.editing_name().is_none() {
                self.open = None;
                self.reconcile(cx);
            }
            self.pending_focus = Some(Refocus::List);
            cx.notify();
            return;
        }

        let entry = form
            .editing_name()
            .and_then(|name| self.find(name))
            .cloned();
        let creating = form.editing_name().is_none();
        self.status = Some("Changes discarded".into());
        if creating {
            // Back to what was showing before "New quicklink".
            self.open = None;
            self.reconcile(cx);
        } else {
            self.show(entry, cx);
        }
        self.pending_focus = Some(Refocus::Form);
        cx.notify();
    }
}

/// What the open form is, as far as following the file goes.
#[derive(Debug, Clone)]
struct Showing {
    /// The link on disk it edits; `None` for a new one.
    name: Option<String>,
    original: Entry,
    edited: bool,
}

/// What to do with the form after the file was read.
#[derive(Debug, Clone, PartialEq)]
enum Follow {
    Keep,
    Show(Entry),
    Clear,
}

/// How the form follows the file.
///
/// A form the user has touched is left alone: their typing is worth more than
/// the file's latest word, and saving will say if the link has gone. An
/// untouched one follows the file, so an edit made by hand shows up here
/// instead of being written back over by the next save. Compared tidied,
/// because that is how the form's own save comes back.
fn follow(entries: &[Entry], showing: Option<Showing>) -> Follow {
    let first = || entries.first().cloned().map_or(Follow::Clear, Follow::Show);

    let Some(showing) = showing else {
        return match first() {
            Follow::Clear => Follow::Keep,
            show => show,
        };
    };
    let Some(name) = showing.name.filter(|_| !showing.edited) else {
        return Follow::Keep;
    };

    match entries
        .iter()
        .find(|entry| entry.name.trim() == name.trim())
    {
        Some(entry) if store::normalised(entry.clone()) != store::normalised(showing.original) => {
            Follow::Show(entry.clone())
        }
        Some(_) => Follow::Keep,
        None => first(),
    }
}

/// What the host asks of the page.
impl QuicklinksPage {
    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        if let Some(open) = &self.open {
            open.form
                .update(cx, |form, cx| form.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// Name the installed browsers, for the form's "Open with" hint.
    pub fn set_browsers(&mut self, browsers: Vec<String>, cx: &mut Context<Self>) {
        if let Some(open) = &self.open {
            open.form
                .update(cx, |form, cx| form.set_browsers(browsers.clone(), cx));
        }
        self.browsers = browsers;
    }

    /// Save the open form, for the window's Ctrl+S.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &self.open {
            open.form.update(cx, |form, cx| form.save(cx));
        }
    }

    /// What the footer says. Unsaved changes first: a "Saved" from before
    /// them is no longer true.
    pub fn status(&self, cx: &App) -> Option<SharedString> {
        if let Some(warning) = &self.warning {
            return Some(warning.clone());
        }
        if self.is_edited(cx) {
            return Some("Unsaved changes  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }

    fn is_edited(&self, cx: &App) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.form.read(cx).is_edited())
    }

    /// The name of the link the form shows, if it shows one from the file.
    fn selected(&self, cx: &App) -> Option<String> {
        self.open
            .as_ref()
            .and_then(|open| open.form.read(cx).editing_name())
            .map(str::to_string)
    }

    /// Focus the list, or the window when there is no list to focus.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        let _ = cx;
        if self.problem.is_none() && !self.entries.is_empty() {
            window.focus(&self.list_focus);
        } else {
            window.focus(&self.host_focus);
        }
    }

    fn focus_form(&self, window: &mut Window, cx: &App) {
        match &self.open {
            Some(open) => open.form.read(cx).focus(window, cx),
            None => self.focus(window, cx),
        }
    }

    /// Whether the form may be replaced. Not with unsaved changes in it:
    /// moving on would drop them without a word.
    fn may_leave_form(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_edited(cx) {
            self.warning =
                Some("Save the changes first, or press Esc in the form to undo them".into());
            cx.notify();
            return false;
        }
        true
    }

    fn select(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected(cx).as_deref() != Some(name.as_str()) {
            if !self.may_leave_form(cx) {
                // Back where the changes are. A click on the list has
                // already focused it, and Escape there closes the window
                // rather than undoing them.
                self.focus_form(window, cx);
                return;
            }
            if let Some(entry) = self.find(&name).cloned() {
                self.status = None;
                self.show(Some(entry), cx);
            }
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.name.trim() == name.trim())
        {
            self.list_scroll.scroll_to_item(index);
        }
        window.focus(&self.list_focus);
        cx.notify();
    }

    /// Move the selection by `delta` rows, stopping at either end.
    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        let current = self.selected(cx).and_then(|name| {
            self.entries
                .iter()
                .position(|entry| entry.name.trim() == name.trim())
        });
        let next = match current {
            Some(index) => (index as isize + delta).clamp(0, self.entries.len() as isize - 1),
            None => 0,
        } as usize;
        let name = self.entries[next].name.trim().to_string();
        self.select(name, window, cx);
    }

    fn previous_link(&mut self, _: &PreviousLink, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn next_link(&mut self, _: &NextLink, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn edit_link(&mut self, _: &EditLink, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_form(window, cx);
    }

    fn new_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.may_leave_form(cx) {
            return;
        }
        self.status = None;
        self.show(None, cx);
        self.focus_form(window, cx);
    }

    /// Ask before deleting the link the form shows.
    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .open
            .as_ref()
            .and_then(|open| open.form.read(cx).editing_name())
            .map(str::to_string);
        if let Some(name) = name {
            self.confirming = Some(name);
            window.focus(&self.dialog_focus);
            cx.notify();
        }
    }

    fn cancel_dialog(&mut self, _: &CancelDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = None;
        self.focus(window, cx);
        cx.notify();
    }

    fn confirm_dialog(&mut self, _: &ConfirmDialog, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.confirming.take() else {
            return;
        };
        // Focus leaves the dialog now, for the list; the reload that follows
        // puts it back there once the list has lost the deleted row.
        self.focus(window, cx);

        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let removed = cx
                .background_executor()
                .spawn({
                    let name = name.clone();
                    async move { store::remove(&path, &name) }
                })
                .await;
            this.update(cx, |page, cx| {
                match removed {
                    Ok(()) => {
                        log::info!("deleted quicklink {name:?}");
                        page.status = Some(format!("Deleted {name}").into());
                        // The form showed the link that is gone: whatever it
                        // held goes with it.
                        page.open = None;
                        page.refocus = Some(Refocus::List);
                    }
                    Err(error) => {
                        log::warn!("could not delete quicklink {name:?}: {error:#}");
                        page.status = Some(format!("Could not delete: {error:#}").into());
                    }
                }
                page.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for QuicklinksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Also when focus has gone altogether: a reload replaced the form the
        // caret was in, and a window with nothing focused hears no keys, not
        // even Escape.
        let pending = self.pending_focus.take();
        if self.confirming.is_none() {
            match pending {
                Some(Refocus::Form) => self.focus_form(window, cx),
                Some(Refocus::List) => self.focus(window, cx),
                None if window.focused(cx).is_none() => self.focus(window, cx),
                None => {}
            }
        }
        let colors = self.colors.clone();

        div()
            .size_full()
            .flex()
            .overflow_hidden()
            .child(self.render_list(&colors, cx))
            .child(self.render_editor(&colors))
    }
}

impl QuicklinksPage {
    fn render_list(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected(cx);
        let deletable = selected.is_some();

        let mut rows = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.iter().enumerate() {
            let name = entry.name.trim().to_string();
            let is_selected = selected.as_deref() == Some(name.as_str());
            let alias = entry.alias.clone().unwrap_or_default();
            rows.push(
                div()
                    .id(SharedString::from(format!("quicklink-{index}")))
                    .h(px(metrics::SIDEBAR_ROW_HEIGHT))
                    .flex_none()
                    .px(px(metrics::PADDING))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(metrics::GAP))
                    .text_size(px(metrics::text::LABEL))
                    .when(is_selected, |row| {
                        row.bg(color(colors.selection_background))
                            .text_color(color(colors.text_primary))
                    })
                    .when(!is_selected, |row| row.text_color(color(colors.text_muted)))
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let name = name.clone();
                        move |page, _, window, cx| page.select(name.clone(), window, cx)
                    }))
                    .child(div().overflow_hidden().truncate().child(name))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(metrics::text::HINT))
                            .text_color(color(colors.text_muted))
                            .child(alias),
                    ),
            );
        }

        let body = match &self.problem {
            Some(problem) => div()
                .flex_1()
                .p(px(metrics::PADDING))
                .text_size(px(metrics::text::HINT))
                .text_color(color(colors.text_error))
                .child(problem.clone())
                .into_any_element(),
            None => div()
                .id("quicklinks")
                .key_context(LIST_CONTEXT)
                .track_focus(&self.list_focus)
                .track_scroll(&self.list_scroll)
                .on_action(cx.listener(Self::previous_link))
                .on_action(cx.listener(Self::next_link))
                .on_action(cx.listener(Self::edit_link))
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(rows)
                .into_any_element(),
        };

        div()
            .w(px(metrics::SIDEBAR_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(color(colors.border_subtle))
            .child(body)
            .child(
                div()
                    .p(px(metrics::GAP))
                    .flex()
                    .flex_col()
                    .gap(px(metrics::GAP))
                    .border_t_1()
                    .border_color(color(colors.border_subtle))
                    // A file that does not parse cannot be written, so there
                    // is nothing to offer until it is fixed.
                    .when(self.problem.is_none(), |column| {
                        column.child(text_button(
                            "new-quicklink",
                            "New quicklink",
                            colors,
                            cx.listener(|page, _, window, cx| page.new_link(window, cx)),
                        ))
                    })
                    .when(deletable, |column| {
                        column.child(text_button(
                            "delete-quicklink",
                            "Delete this quicklink",
                            colors,
                            cx.listener(|page, _, window, cx| page.ask_delete(window, cx)),
                        ))
                    }),
            )
    }

    fn render_editor(&self, colors: &Colors) -> AnyElement {
        if let Some(open) = &self.open {
            return div()
                .flex_1()
                .py(px(metrics::PADDING))
                .child(open.form.clone())
                .into_any_element();
        }

        let empty = if self.problem.is_some() {
            "Fix quicklinks.json, and the list comes back by itself."
        } else if self.entries.is_empty() {
            "No quicklinks yet. Add one on the left."
        } else {
            "Choose a quicklink on the left."
        };
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(metrics::text::LABEL))
            .text_color(color(colors.text_muted))
            .child(empty)
            .into_any_element()
    }
}
impl QuicklinksPage {
    /// The delete dialog, over the whole window. Drawn by the host, which is
    /// the only element as large as the window.
    pub fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let name = self.confirming.clone()?;
        let colors = &self.colors;

        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "dialog-confirm",
                "Delete  Enter",
                colors,
                cx.listener(|page, _, window, cx| page.confirm_dialog(&ConfirmDialog, window, cx)),
            ));

        let dialog = div()
            .key_context(DIALOG_CONTEXT)
            .track_focus(&self.dialog_focus)
            .on_action(cx.listener(Self::confirm_dialog))
            .on_action(cx.listener(Self::cancel_dialog))
            .w(px(metrics::DIALOG_WIDTH))
            .p(px(metrics::PADDING))
            .flex()
            .flex_col()
            .gap(px(metrics::GAP))
            .rounded(px(metrics::RADIUS))
            .border_1()
            .border_color(color(colors.border_strong))
            .bg(color(colors.surface_elevated))
            .child(
                div()
                    .text_size(px(metrics::text::TITLE))
                    .text_color(color(colors.text_primary))
                    .child(format!("Delete {name}?")),
            )
            .child(
                div()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .child("It is removed from quicklinks.json. Lumen cannot bring it back."),
            )
            .child(buttons);

        // The scrim: dims the window and takes every click meant for what is
        // behind it, so the dialog is the only thing that can be answered.
        let scrim = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(px(metrics::RADIUS))
            .bg(color(colors.shadow))
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .child(dialog);

        Some(scrim.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, link: &str) -> Entry {
        Entry {
            name: name.into(),
            link: link.into(),
            ..Entry::default()
        }
    }

    fn open(name: Option<&str>, original: Entry, edited: bool) -> Showing {
        Showing {
            name: name.map(str::to_string),
            original,
            edited,
        }
    }

    #[test]
    fn with_nothing_open_the_first_link_is_shown() {
        let entries = [entry("A", "https://a.com"), entry("B", "https://b.com")];
        assert_eq!(follow(&entries, None), Follow::Show(entries[0].clone()));
        assert_eq!(follow(&[], None), Follow::Keep);
    }

    /// The user's typing outranks the file.
    #[test]
    fn an_edited_form_is_left_alone() {
        let entries = [entry("A", "https://changed.com")];
        let showing = open(Some("A"), entry("A", "https://a.com"), true);
        assert_eq!(follow(&entries, Some(showing)), Follow::Keep);
    }

    /// A hand edit shows up rather than being saved over later.
    #[test]
    fn an_untouched_form_follows_a_hand_edit() {
        let entries = [entry("A", "https://changed.com")];
        let showing = open(Some("A"), entry("A", "https://a.com"), false);
        assert_eq!(
            follow(&entries, Some(showing)),
            Follow::Show(entries[0].clone())
        );
    }

    /// After a save the form holds what it sent -- a blank alias, untrimmed
    /// text -- and the file holds the tidied version. That is the same link,
    /// and rebuilding the form would take the caret out of it.
    #[test]
    fn a_link_differing_only_in_tidying_is_the_same_link() {
        let entries = [entry("A", "https://a.com")];
        let sent = Entry {
            alias: Some(String::new()),
            open_with: Some("  ".into()),
            ..entry(" A ", "https://a.com ")
        };
        assert_eq!(
            follow(&entries, Some(open(Some("A"), sent, false))),
            Follow::Keep
        );
    }

    #[test]
    fn a_link_deleted_elsewhere_moves_to_the_first_or_clears() {
        let entries = [entry("B", "https://b.com")];
        let gone = open(Some("A"), entry("A", "https://a.com"), false);
        assert_eq!(
            follow(&entries, Some(gone.clone())),
            Follow::Show(entries[0].clone())
        );
        assert_eq!(follow(&[], Some(gone)), Follow::Clear);
    }

    /// A new link has nothing on disk to follow.
    #[test]
    fn a_new_link_form_is_kept() {
        let entries = [entry("A", "https://a.com")];
        let creating = open(None, Entry::default(), false);
        assert_eq!(follow(&entries, Some(creating)), Follow::Keep);
    }
}
