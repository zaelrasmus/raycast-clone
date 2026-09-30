//! The Clipboard History page of the settings window (F12).
//!
//! How many entries are kept, and clearing them. The limit is the `clipboard`
//! section of `config.json`, written through `lumen_core::clipboard::settings`
//! and applied by the daemon as soon as the file changes. Clearing is not a
//! setting: it acts on the history itself, through the same handle the view
//! reads, and only after a dialog has said what goes and what does not.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::clipboard::{History, MAX_ENTRIES_LIMIT, Summary, images, settings};
use lumen_core::config::ClipboardConfig;
use lumen_core::layout::{self, settings as metrics};
use lumen_core::theme::Colors;
use parking_lot::Mutex;

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::quicklinks_page::{CancelDialog, ConfirmDialog, DIALOG_CONTEXT};
use crate::settings::text_button;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

pub struct ClipboardPage {
    path: PathBuf,
    colors: Colors,
    /// The section as the file last read had it.
    config: ClipboardConfig,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    limit: Entity<TextField>,
    /// `None` when history is off, or its database could not be opened.
    history: Option<Arc<Mutex<History>>>,
    /// What clearing would remove, while the dialog asks.
    confirming: Option<Summary>,
    dialog_focus: FocusHandle,
    edited: bool,
    busy: bool,
    error: Option<String>,
    status: Option<SharedString>,
    refocus: bool,
}

impl ClipboardPage {
    pub fn new(path: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = ClipboardConfig::default();
        let limit = field(&config, &colors, cx);
        let mut page = Self {
            path,
            colors,
            config,
            problem: None,
            limit,
            history: None,
            confirming: None,
            dialog_focus: cx.focus_handle(),
            edited: false,
            busy: false,
            error: None,
            status: None,
            refocus: false,
        };
        page.watch_form(cx);
        page
    }

    /// The history to clear, once the daemon has opened it.
    pub fn use_history(&mut self, history: Option<Arc<Mutex<History>>>) {
        self.history = history;
    }

    /// Re-read the file, off the UI thread. An untouched form follows it.
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
                    }
                    Err(error) => {
                        log::warn!("the clipboard page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                if !page.edited {
                    page.rebuild(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.limit = field(&self.config, &self.colors, cx);
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    fn watch_form(&mut self, cx: &mut Context<Self>) {
        cx.subscribe(&self.limit, |page, _, event, cx| {
            if matches!(event, TextFieldEvent::Changed(_)) {
                page.edited = true;
                page.error = None;
                cx.notify();
            }
        })
        .detach();
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.limit.focus_handle(cx));
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window, cx);
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edited {
            return;
        }
        self.status = Some("Changes discarded".into());
        self.rebuild(cx);
        self.refocus = true;
        cx.notify();
    }

    // -- Saving the limit ---------------------------------------------------------

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Save the limit, as Enter does. For the window's Ctrl+S too.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.problem.is_some() || !self.edited {
            return;
        }
        let typed = self.limit.read(cx).text().trim().to_string();
        let Some(limit) = typed
            .parse::<usize>()
            .ok()
            .filter(|limit| (1..=MAX_ENTRIES_LIMIT).contains(limit))
        else {
            self.error = Some(format!(
                "Type a number of entries from 1 to {MAX_ENTRIES_LIMIT}"
            ));
            cx.notify();
            return;
        };
        let path = self.path.clone();
        self.busy = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { settings::set_max_entries(&path, limit) })
                .await;
            this.update(cx, |page, cx| {
                page.busy = false;
                match result {
                    Ok(()) => {
                        log::info!("clipboard history keeps {limit} entries");
                        page.status = Some(format!("Keeping at most {limit} entries").into());
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the clipboard limit: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // -- Clearing -------------------------------------------------------------------

    /// Count what would go, then ask.
    fn ask_to_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(history) = self.history.clone() else {
            self.error = Some("Clipboard history is off, so there is nothing to clear".into());
            cx.notify();
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        let dialog = self.dialog_focus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let summary = cx
                .background_executor()
                .spawn(async move { history.lock().summary() })
                .await;
            this.update_in(cx, |page, window, cx| {
                page.busy = false;
                match summary {
                    Ok(summary) if summary.entries == 0 => {
                        page.status = Some("The history is already empty".into());
                    }
                    Ok(summary) => {
                        page.confirming = Some(summary);
                        window.focus(&dialog);
                    }
                    Err(error) => page.error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cancel_dialog(&mut self, _: &CancelDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = None;
        self.focus(window, cx);
        cx.notify();
    }

    fn confirm_dialog(&mut self, _: &ConfirmDialog, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(summary), Some(history)) = (self.confirming.take(), self.history.clone()) else {
            return;
        };
        self.focus(window, cx);
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { history.lock().clear() })
                .await;
            this.update(cx, |page, cx| {
                page.busy = false;
                match result {
                    Ok(()) => {
                        log::info!("cleared the clipboard history");
                        page.status = Some(format!("Cleared {}", entries(summary.entries)).into());
                    }
                    Err(error) => {
                        log::warn!("could not clear the clipboard history: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // -- What the host asks -------------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.limit
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.colors = colors;
        cx.notify();
    }

    pub fn status(&self) -> Option<SharedString> {
        if self.edited {
            return Some("Unsaved changes  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }

    /// The confirmation, over the whole window, drawn by the host.
    pub fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let summary = self.confirming?;
        let colors = &self.colors;
        let pictures = match summary.images {
            0 => String::new(),
            1 => format!(
                ", including 1 image ({})",
                images::size_label(summary.image_bytes)
            ),
            count => format!(
                ", including {count} images ({})",
                images::size_label(summary.image_bytes)
            ),
        };
        let body = format!(
            "Deletes all {}{pictures} from Lumen's history and from disk. \
             This cannot be undone.\n\n\
             What is on the clipboard right now stays there, and what you copy \
             from now on is recorded as before. To stop recording, turn \
             clipboard history off in config.json.",
            entries(summary.entries)
        );

        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "clipboard-dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "clipboard-dialog-confirm",
                "Clear History  Enter",
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
                    .child("Clear clipboard history?"),
            )
            .child(
                div()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .child(body),
            )
            .child(buttons);

        Some(
            div()
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
                .child(dialog)
                .into_any_element(),
        )
    }
}

/// *1 entry*, *283 entries*.
fn entries(count: usize) -> String {
    if count == 1 {
        "1 entry".into()
    } else {
        format!("{count} entries")
    }
}

/// The limit field, filled in from `config`.
fn field(config: &ClipboardConfig, colors: &Colors, cx: &mut App) -> Entity<TextField> {
    let current = config.retention.clamped().max_entries.to_string();
    cx.new(|cx| {
        TextField::new(
            current,
            format!("1 to {MAX_ENTRIES_LIMIT}"),
            colors.clone(),
            cx,
        )
    })
}

impl Render for ClipboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.confirming.is_none()
            && (std::mem::take(&mut self.refocus) || window.focused(cx).is_none())
        {
            self.focus(window, cx);
        }
        let colors = self.colors.clone();
        div()
            .size_full()
            .flex()
            .overflow_hidden()
            .child(self.render_form(&colors, window, cx))
    }
}

impl ClipboardPage {
    fn render_form(&self, colors: &Colors, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(problem) = &self.problem {
            return div()
                .flex_1()
                .p(px(metrics::PADDING))
                .text_size(px(metrics::text::HINT))
                .text_color(color(colors.text_error))
                .child(format!(
                    "{problem}\n\nFix config.json, and this page comes back by itself."
                ))
                .into_any_element();
        }

        let label = |text: &'static str| {
            div()
                .w(px(layout::form::LABEL_WIDTH))
                .flex_none()
                .text_size(type_scale::subtitle())
                .text_color(color(colors.text_muted))
                .child(text)
        };
        let row = |text: &'static str, control: AnyElement| {
            div()
                .h(px(layout::form::ROW_HEIGHT))
                .flex()
                .items_center()
                .child(label(text))
                .child(div().flex_1().child(control))
        };

        let clear = text_button(
            "clear-clipboard-history",
            "Clear History\u{2026}",
            colors,
            cx.listener(|page, _, window, cx| page.ask_to_clear(window, cx)),
        )
        .into_any_element();

        let focused = self.limit.focus_handle(cx).is_focused(window);
        let (message, is_error) = match (&self.error, focused) {
            (Some(error), _) => (error.clone(), true),
            (None, true) => (
                format!(
                    "The newest are kept. Lowering it deletes the oldest past it at once. \
                     Entries older than {} days go as well.",
                    self.config.retention.max_age_days
                ),
                false,
            ),
            (None, false) => ("Enter or Ctrl+S saves. Esc undoes.".to_string(), false),
        };

        div()
            .flex_1()
            .py(px(metrics::PADDING))
            .child(
                div()
                    .key_context(FORM_CONTEXT)
                    .on_action(cx.listener(Self::next_field))
                    .on_action(cx.listener(Self::previous_field))
                    .on_action(cx.listener(Self::submit))
                    .on_action(cx.listener(Self::cancel))
                    .w_full()
                    .px(crate::styling::metrics::gutter())
                    .py(px(layout::form::PADDING_Y))
                    .flex()
                    .flex_col()
                    .child(row("Keep at most", self.limit.clone().into_any_element()))
                    .child(row("History", clear))
                    .child(
                        div()
                            .h(px(layout::form::MESSAGE_HEIGHT))
                            .pl(px(layout::form::LABEL_WIDTH))
                            .flex()
                            .items_center()
                            .text_size(type_scale::footer())
                            .text_color(color(if is_error {
                                colors.text_error
                            } else {
                                colors.text_muted
                            }))
                            .child(div().truncate().child(message)),
                    ),
            )
            .into_any_element()
    }
}
