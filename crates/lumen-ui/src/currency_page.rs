//! The Currency page of the settings window (F9, slice 6).
//!
//! Who is asked for exchange rates, and the key for a service that needs
//! one. The Translation page's service form on its own: there are no presets
//! to list beside it, so the form has the page.
//!
//! # The file stays the source of truth
//!
//! The page reads `config.json` and re-reads it after every save and
//! whenever the file changes. Every write goes through
//! `lumen_core::currency::store`, which changes only the `currency` section.
//! The key is never shown back: the field starts empty, says how the saved
//! key ends, and an empty field on save keeps it.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::currency::{CurrencyConfig, Service, store};
use lumen_core::layout::{self, settings as metrics};
use lumen_core::theme::Colors;

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

/// The services, in the order the dropdown lists them, with the value each
/// is chosen by.
const SERVICES: [(Service, &str, &str); 3] = [
    (
        Service::Open,
        "open",
        "ExchangeRate-API, open (no key needed)",
    ),
    (
        Service::ExchangerateApi,
        "exchangerate-api",
        "ExchangeRate-API (your API key)",
    ),
    (Service::Unirate, "unirate", "UniRateAPI (your API key)"),
];

pub struct CurrencyPage {
    path: PathBuf,
    colors: Colors,
    /// The section as the file last read had it.
    config: CurrencyConfig,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    service: Entity<Select>,
    key: Entity<TextField>,
    /// Whether the form has changed since it was filled in or saved.
    edited: bool,
    saving: bool,
    /// Why the last save was refused, under the form.
    error: Option<String>,
    status: Option<SharedString>,
    /// Put the caret back in the form at the next frame: undoing rebuilds it
    /// with no `Window` at hand to focus the new fields with.
    refocus: bool,
}

impl CurrencyPage {
    pub fn new(path: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = CurrencyConfig::default();
        let (service, key) = form(&config, &colors, cx);
        let mut page = Self {
            path,
            colors,
            config,
            problem: None,
            service,
            key,
            edited: false,
            saving: false,
            error: None,
            status: None,
            refocus: false,
        };
        page.watch_form(cx);
        page
    }

    /// Re-read the file, off the UI thread. An untouched form follows it, so
    /// a hand edit shows up here rather than being saved over; a touched one
    /// is left alone.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { store::read(&path) })
                .await;
            this.update(cx, |page, cx| {
                match read {
                    Ok(config) => {
                        page.config = config;
                        page.problem = None;
                    }
                    Err(error) => {
                        log::warn!("the currency page cannot read the file: {error:#}");
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

    /// Fill the form in again from the file.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        (self.service, self.key) = form(&self.config, &self.colors, cx);
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    /// Mark the form edited on any change to it.
    fn watch_form(&mut self, cx: &mut Context<Self>) {
        cx.subscribe(&self.key, |page, _, event, cx| {
            if matches!(event, TextFieldEvent::Changed(_)) {
                page.touched(cx);
            }
        })
        .detach();
        cx.subscribe(&self.service, |page, _, SelectEvent::Changed, cx| {
            page.touched(cx)
        })
        .detach();
    }

    fn touched(&mut self, cx: &mut Context<Self>) {
        self.edited = true;
        self.error = None;
        cx.notify();
    }

    // -- Focus ----------------------------------------------------------------

    /// Focus the first field: the page is the form.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
    }

    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![
            self.service.read(cx).focus_handle().clone(),
            self.key.focus_handle(cx),
        ]
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.stops(cx)
            .iter()
            .position(|stop| stop.is_focused(window))
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let next = match self.focused(window, cx) {
            Some(index) => (index as isize + delta).rem_euclid(stops.len() as isize) as usize,
            None => 0,
        };
        window.focus(&stops[next]);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, window, cx);
    }

    /// Escape in the form: undo what was typed.
    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        if !self.edited {
            return;
        }
        self.status = Some("Changes discarded".into());
        self.rebuild(cx);
        self.refocus = true;
        cx.notify();
    }

    // -- Saving -----------------------------------------------------------------

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Save the form, as Enter does. For the window's Ctrl+S too.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.problem.is_some() {
            return;
        }
        let path = self.path.clone();
        let chosen = self.service.read(cx).value();
        let service = SERVICES
            .iter()
            .find(|(_, value, _)| Some(*value) == chosen)
            .map_or(Service::Open, |(service, _, _)| *service);
        let typed = self.key.read(cx).text().trim().to_string();
        // Empty keeps the saved key: the field never shows it.
        let key = (!typed.is_empty()).then_some(typed);
        self.write(
            move || store::set_service(&path, service, key.as_deref()),
            "Saved the service".into(),
            cx,
        );
    }

    /// Run `write` off the UI thread; on success, re-read.
    fn write(
        &mut self,
        write: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
        done: String,
        cx: &mut Context<Self>,
    ) {
        self.saving = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { write() }).await;
            this.update(cx, |page, cx| {
                page.saving = false;
                match result {
                    Ok(()) => {
                        log::info!("{done}");
                        page.status = Some(done.into());
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the currency settings: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn remove_key(&mut self, cx: &mut Context<Self>) {
        if self.edited {
            self.error = Some("Save the changes first, or press Esc to undo them".into());
            cx.notify();
            return;
        }
        let path = self.path.clone();
        let service = self.config.service;
        self.write(
            move || store::set_service(&path, service, Some("")),
            "Removed the API key".into(),
            cx,
        );
    }

    // -- What the host asks -------------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.service
            .update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        self.key
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.colors = colors;
        cx.notify();
    }

    /// What the footer says. Unsaved changes first: a "Saved" from before
    /// them is no longer true.
    pub fn status(&self) -> Option<SharedString> {
        if self.edited {
            return Some("Unsaved changes  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }
}

/// The service dropdown and the key field, filled in from `config`.
fn form(
    config: &CurrencyConfig,
    colors: &Colors,
    cx: &mut App,
) -> (Entity<Select>, Entity<TextField>) {
    let choices = SERVICES
        .iter()
        .map(|(_, value, label)| Choice {
            value: Some((*value).into()),
            label: (*label).into(),
        })
        .collect();
    let selected = SERVICES
        .iter()
        .position(|(service, _, _)| *service == config.service)
        .unwrap_or(0);
    let placeholder = match config.key() {
        Some(key) => format!(
            "Saved, ending in \u{2026}{}. Type to replace it.",
            key.chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        ),
        None => "Paste your API key".into(),
    };
    (
        cx.new(|cx| Select::new(choices, selected, colors.clone(), cx)),
        cx.new(|cx| TextField::new(String::new(), placeholder, colors.clone(), cx)),
    )
}

impl Render for CurrencyPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A rebuilt form takes the focused field with it, and a window with
        // nothing focused hears no keys, not even Escape.
        if std::mem::take(&mut self.refocus) || window.focused(cx).is_none() {
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

impl CurrencyPage {
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
        let field_row = |text: &'static str, control: AnyElement| {
            div()
                .h(px(layout::form::ROW_HEIGHT))
                .flex()
                .items_center()
                .child(label(text))
                .child(div().flex_1().child(control))
        };

        let mut rows = vec![
            field_row("Service", self.service.clone().into_any_element()).into_any_element(),
            field_row("API key", self.key.clone().into_any_element()).into_any_element(),
        ];
        if self.config.key().is_some() {
            rows.push(
                div()
                    .h(px(layout::form::ROW_HEIGHT))
                    .pl(px(layout::form::LABEL_WIDTH))
                    .flex()
                    .items_center()
                    .child(text_button(
                        "remove-currency-key",
                        "Remove saved key",
                        colors,
                        cx.listener(|page, _, _, cx| page.remove_key(cx)),
                    ))
                    .into_any_element(),
            );
        }
        let hints = [
            "Who is asked for rates, once a day. Never another: a failure is shown, not retried elsewhere.",
            "Only used by the keyed services. Left empty, the saved key is kept.",
        ];

        let (message, is_error) = match (&self.error, self.focused(window, cx)) {
            (Some(error), _) => (error.clone(), true),
            (None, Some(index)) => (hints.get(index).copied().unwrap_or("").to_string(), false),
            (None, None) => ("Enter or Ctrl+S saves. Esc undoes.".to_string(), false),
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
                    .children(rows)
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
