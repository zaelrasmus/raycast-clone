//! The Calculator page of the settings window (F14).
//!
//! How numbers are written, how many calculations are kept, and clearing
//! them: *Clipboard History*'s page (F12), for a file instead of a database. The limit is the
//! `calculator` section of `config.json`, written through
//! `lumen_core::calc::settings`; a lower one trims the history at once.
//! Clearing is not a setting: it acts on the history itself, under the lock
//! it is kept with, and only after a dialog has said what goes.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::calc::history::{CalculatorConfig, History, MAX_LIMIT, NumberFormat};
use lumen_core::calc::settings;
use lumen_core::layout::{self, settings as metrics};
use lumen_core::theme::Colors;

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::quicklinks_page::{CancelDialog, ConfirmDialog, DIALOG_CONTEXT};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

pub struct CalculatorPage {
    path: PathBuf,
    /// `calculator-history.json`, to trim and to clear.
    history: PathBuf,
    colors: Colors,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    limit: Entity<TextField>,
    format: Entity<Select>,
    /// How many calculations clearing would remove, while the dialog asks.
    confirming: Option<usize>,
    dialog_focus: FocusHandle,
    edited: bool,
    busy: bool,
    error: Option<String>,
    status: Option<SharedString>,
    refocus: bool,
    /// The section as the file last read had it.
    config: CalculatorConfig,
}

impl CalculatorPage {
    pub fn new(path: PathBuf, history: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = CalculatorConfig::default();
        let limit = field(&config, &colors, cx);
        let format = format_select(config.number_format, &colors, cx);
        let mut page = Self {
            path,
            history,
            colors,
            problem: None,
            limit,
            format,
            confirming: None,
            dialog_focus: cx.focus_handle(),
            edited: false,
            busy: false,
            error: None,
            status: None,
            refocus: false,
            config,
        };
        page.watch_form(cx);
        page
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
                        log::warn!("the calculator page cannot read the file: {error:#}");
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
        self.format = format_select(self.config.number_format, &self.colors, cx);
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
        cx.subscribe(&self.format, |page, _, SelectEvent::Changed, cx| {
            page.edited = true;
            page.error = None;
            cx.notify();
        })
        .detach();
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.limit.focus_handle(cx));
    }

    /// Two stops, so forwards and backwards are both the other one.
    fn other_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let limit = self.limit.focus_handle(cx);
        if limit.is_focused(window) {
            window.focus(self.format.read(cx).focus_handle());
        } else {
            window.focus(&limit);
        }
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.other_field(window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.other_field(window, cx);
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

    // -- Saving the limit -----------------------------------------------------

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Save the limit and the format, as Enter does. For the window's
    /// Ctrl+S too.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.problem.is_some() || !self.edited {
            return;
        }
        let typed = self.limit.read(cx).text().trim().to_string();
        let Some(limit) = typed
            .parse::<usize>()
            .ok()
            .filter(|limit| *limit <= MAX_LIMIT)
        else {
            self.error = Some(format!(
                "Type a number of calculations from 0 to {MAX_LIMIT}"
            ));
            cx.notify();
            return;
        };
        let format = chosen_format(self.format.read(cx).value());
        let path = self.path.clone();
        let history = self.history.clone();
        self.busy = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            // The oldest past a lower limit go at once, as the hint says.
            let result = cx
                .background_executor()
                .spawn(async move {
                    settings::set_number_format(&path, format)?;
                    settings::set_history_limit(&path, limit)?;
                    History::change(&history, |history| history.trim(limit)).map(|_| ())
                })
                .await;
            this.update(cx, |page, cx| {
                page.busy = false;
                match result {
                    Ok(()) => {
                        log::info!("calculator history keeps {limit} calculations");
                        page.status = Some(match limit {
                            0 => "Saved. History is off".into(),
                            limit => {
                                format!("Saved. Keeping at most {}", calculations(limit)).into()
                            }
                        });
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the calculator's limit: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // -- Clearing -------------------------------------------------------------

    /// Count what would go, then ask.
    fn ask_to_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let history = self.history.clone();
        let dialog = self.dialog_focus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let kept = cx
                .background_executor()
                .spawn(async move { History::load(&history).entries.len() })
                .await;
            this.update_in(cx, |page, window, cx| {
                page.busy = false;
                if kept == 0 {
                    page.status = Some("The history is already empty".into());
                } else {
                    page.confirming = Some(kept);
                    window.focus(&dialog);
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
        let Some(kept) = self.confirming.take() else {
            return;
        };
        self.focus(window, cx);
        self.busy = true;
        let history = self.history.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    History::change(&history, |history| {
                        let had = !history.entries.is_empty();
                        history.entries.clear();
                        had
                    })
                })
                .await;
            this.update(cx, |page, cx| {
                page.busy = false;
                match result {
                    Ok(_) => {
                        log::info!("cleared the calculator history");
                        page.status = Some(format!("Cleared {}", calculations(kept)).into());
                    }
                    Err(error) => {
                        log::warn!("could not clear the calculator history: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // -- What the host asks ---------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.limit
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.format
            .update(cx, |select, cx| select.set_colors(colors.clone(), cx));
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
        let kept = self.confirming?;
        let colors = &self.colors;
        let body = format!(
            "Deletes all {} from Lumen's calculator history. This cannot be undone.\n\n\
             What you copy from the calculator from now on is kept as before. To stop \
             keeping it, set the limit to 0.",
            calculations(kept)
        );

        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "calculator-dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "calculator-dialog-confirm",
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
                    .child("Clear calculator history?"),
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

/// *1 calculation*, *20 calculations*.
fn calculations(count: usize) -> String {
    if count == 1 {
        "1 calculation".into()
    } else {
        format!("{count} calculations")
    }
}

/// The choices of the format dropdown: its value, as `config.json` writes
/// it, and its label.
const FORMATS: [(NumberFormat, &str, &str); 2] = [
    (
        NumberFormat::System,
        "system",
        "As Windows writes them  1,234.5 or 1.234,5",
    ),
    (NumberFormat::Plain, "plain", "Plain  1234.5"),
];

fn format_select(current: NumberFormat, colors: &Colors, cx: &mut App) -> Entity<Select> {
    let choices = FORMATS
        .iter()
        .map(|(_, value, label)| Choice {
            value: Some((*value).into()),
            label: (*label).into(),
        })
        .collect();
    let selected = FORMATS
        .iter()
        .position(|(format, _, _)| *format == current)
        .unwrap_or(0);
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx))
}

fn chosen_format(value: Option<&str>) -> NumberFormat {
    FORMATS
        .iter()
        .find(|(_, written, _)| Some(*written) == value)
        .map_or_else(NumberFormat::default, |(format, _, _)| *format)
}

/// The limit field, filled in from `config`.
fn field(config: &CalculatorConfig, colors: &Colors, cx: &mut App) -> Entity<TextField> {
    let current = config.limit().to_string();
    cx.new(|cx| TextField::new(current, format!("0 to {MAX_LIMIT}"), colors.clone(), cx))
}

impl Render for CalculatorPage {
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

impl CalculatorPage {
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
            "clear-calculator-history",
            "Clear History\u{2026}",
            colors,
            cx.listener(|page, _, window, cx| page.ask_to_clear(window, cx)),
        )
        .into_any_element();

        let focused = self.limit.focus_handle(cx).is_focused(window);
        let on_format = self.format.read(cx).focus_handle().is_focused(window);
        let (message, is_error) = match (&self.error, focused) {
            (Some(error), _) => (error.clone(), true),
            (None, false) if on_format => (
                "How answers are shown and typed. Copied answers are never grouped.".to_string(),
                false,
            ),
            (None, true) => (
                "The newest are kept, and 0 keeps none. Lowering it deletes the oldest \
                 past it at once."
                    .to_string(),
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
                    .child(row("Numbers", self.format.clone().into_any_element()))
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
