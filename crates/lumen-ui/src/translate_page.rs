//! The Translation page of the settings window (F7).
//!
//! The service, the DeepL key, and the presets, side by side. The shape is
//! the Quicklinks page's: a list on the left, the selected thing's form on the
//! right, a question before a delete.
//!
//! # The file stays the source of truth
//!
//! The page reads `config.json` for its list and re-reads it after every save
//! and whenever the file changes. Every write goes through
//! `lumen_core::translate::store`, which reads the file afresh, changes only
//! the `translate` section and finds a preset by name. The daemon's watcher
//! then updates the launcher, exactly as after a hand edit.
//!
//! # Choices, not typing
//!
//! Languages are dropdowns of the table both services accept, so a preset
//! saved here cannot name a language the loader would skip. The DeepL key is
//! never shown back: the field starts empty, says how the saved key ends, and
//! an empty field on save keeps it.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, ScrollHandle, SharedString, Window,
    actions, div, px,
};
use lumen_core::layout::{self, settings as metrics};
use lumen_core::theme::Colors;
use lumen_core::translate::{
    LANGUAGES, MAX_TARGETS, PresetConfig, Service, TranslateConfig, languages, store,
};

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::quicklinks_page::{CancelDialog, ConfirmDialog, DIALOG_CONTEXT};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};

actions!(
    lumen_presets,
    [
        /// Select the row above in the list of presets.
        PreviousPreset,
        /// Select the row below.
        NextPreset,
        /// Go from the list into the selected row's form.
        EditPreset,
    ]
);

/// The list's key context.
pub const LIST_CONTEXT: &str = "PresetList";

/// The form's key context: Tab and the arrows move between fields, Enter
/// saves, Escape undoes -- the quicklink form's actions, bound again here.
pub const FORM_CONTEXT: &str = "TranslateForm";

/// What the list has selected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Service,
    /// A preset on disk, by name.
    Preset(String),
    /// A preset not saved yet.
    New,
}

enum Form {
    Service {
        service: Entity<Select>,
        key: Entity<TextField>,
    },
    Preset {
        name: Entity<TextField>,
        alias: Entity<TextField>,
        from: Entity<Select>,
        to: [Entity<Select>; MAX_TARGETS],
    },
}

pub struct TranslatePage {
    path: PathBuf,
    colors: Colors,
    /// The section as the file last read had it.
    config: TranslateConfig,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    selection: Selection,
    form: Form,
    /// Whether the form has changed since it was filled in or saved.
    edited: bool,
    saving: bool,
    /// Why the last save was refused, under the form.
    error: Option<String>,
    status: Option<SharedString>,
    /// Why the last move was refused.
    warning: Option<SharedString>,
    /// The preset waiting on the delete dialog.
    confirming: Option<String>,
    dialog_focus: FocusHandle,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    /// Put the caret back in the form at the next frame: undoing rebuilds it
    /// with no `Window` at hand to focus the new fields with.
    refocus_form: bool,
}

impl TranslatePage {
    pub fn new(path: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = TranslateConfig::default();
        let form = service_form(&config, &colors, cx);
        Self {
            path,
            colors,
            config,
            problem: None,
            selection: Selection::Service,
            form,
            edited: false,
            saving: false,
            error: None,
            status: None,
            warning: None,
            confirming: None,
            dialog_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            refocus_form: false,
        }
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
                        log::warn!("the translation page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                if let Selection::Preset(name) = &page.selection
                    && page.find(name).is_none()
                {
                    page.selection = Selection::Service;
                    page.edited = false;
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

    fn find(&self, name: &str) -> Option<&PresetConfig> {
        self.config
            .presets
            .iter()
            .find(|preset| preset.name.trim() == name.trim())
    }

    /// Fill the form in again from the file, for what is selected.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.form = match &self.selection {
            Selection::Service => service_form(&self.config, &self.colors, cx),
            Selection::Preset(name) => {
                let preset = self.find(name).cloned().unwrap_or_default();
                preset_form(&preset, &self.colors, cx)
            }
            Selection::New => preset_form(&new_preset(), &self.colors, cx),
        };
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    /// Mark the form edited on any change to it.
    fn watch_form(&mut self, cx: &mut Context<Self>) {
        let (fields, selects): (Vec<Entity<TextField>>, Vec<Entity<Select>>) = match &self.form {
            Form::Service { service, key } => (vec![key.clone()], vec![service.clone()]),
            Form::Preset {
                name,
                alias,
                from,
                to,
            } => (
                vec![name.clone(), alias.clone()],
                std::iter::once(from.clone())
                    .chain(to.iter().cloned())
                    .collect(),
            ),
        };
        for field in fields {
            cx.subscribe(&field, |page, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    page.touched(cx);
                }
            })
            .detach();
        }
        for select in selects {
            cx.subscribe(&select, |page, _, SelectEvent::Changed, cx| {
                page.touched(cx)
            })
            .detach();
        }
    }

    fn touched(&mut self, cx: &mut Context<Self>) {
        self.edited = true;
        self.error = None;
        self.warning = None;
        cx.notify();
    }

    /// Whether the form may be replaced. Not with unsaved changes in it.
    fn may_leave(&mut self, cx: &mut Context<Self>) -> bool {
        if self.edited {
            self.warning =
                Some("Save the changes first, or press Esc in the form to undo them".into());
            cx.notify();
            return false;
        }
        true
    }

    fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if selection != self.selection {
            if !self.may_leave(cx) {
                self.focus_form(window, cx);
                return;
            }
            self.selection = selection;
            self.status = None;
            self.rebuild(cx);
        }
        if let Some(index) = self.rows().iter().position(|row| *row == self.selection) {
            self.list_scroll.scroll_to_item(index);
        }
        window.focus(&self.list_focus);
        cx.notify();
    }

    /// The list's rows, in order.
    fn rows(&self) -> Vec<Selection> {
        std::iter::once(Selection::Service)
            .chain(
                self.config
                    .presets
                    .iter()
                    .map(|preset| Selection::Preset(preset.name.trim().to_string())),
            )
            .collect()
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let current = rows
            .iter()
            .position(|row| *row == self.selection)
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.select(rows[next].clone(), window, cx);
    }

    fn previous_preset(&mut self, _: &PreviousPreset, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn next_preset(&mut self, _: &NextPreset, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn edit_preset(&mut self, _: &EditPreset, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_form(window, cx);
    }

    fn new_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.may_leave(cx) {
            return;
        }
        self.selection = Selection::New;
        self.status = None;
        self.rebuild(cx);
        self.focus_form(window, cx);
        cx.notify();
    }

    // -- Focus ----------------------------------------------------------------

    /// Focus the list, which always has at least the service row.
    pub fn focus(&self, window: &mut Window, _cx: &App) {
        window.focus(&self.list_focus);
    }

    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        match &self.form {
            Form::Service { service, key } => vec![
                service.read(cx).focus_handle().clone(),
                key.focus_handle(cx),
            ],
            Form::Preset {
                name,
                alias,
                from,
                to,
            } => [name.focus_handle(cx), alias.focus_handle(cx)]
                .into_iter()
                .chain(std::iter::once(from.read(cx).focus_handle().clone()))
                .chain(
                    to.iter()
                        .map(|select| select.read(cx).focus_handle().clone()),
                )
                .collect(),
        }
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.stops(cx)
            .iter()
            .position(|stop| stop.is_focused(window))
    }

    fn focus_form(&self, window: &mut Window, cx: &App) {
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
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

    /// Escape in the form: undo what was typed, or back to the list.
    fn cancel(&mut self, _: &CancelForm, window: &mut Window, cx: &mut Context<Self>) {
        self.warning = None;
        if !self.edited {
            if self.selection == Selection::New {
                self.selection = Selection::Service;
                self.rebuild(cx);
            }
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.status = Some("Changes discarded".into());
        if self.selection == Selection::New {
            self.selection = Selection::Service;
            self.rebuild(cx);
            self.focus(window, cx);
        } else {
            self.rebuild(cx);
            self.refocus_form = true;
        }
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

        match &self.form {
            Form::Service { service, key } => {
                let service = match service.read(cx).value() {
                    Some("deepl") => Service::Deepl,
                    _ => Service::Google,
                };
                let typed = key.read(cx).text().trim().to_string();
                // Empty keeps the saved key: the field never shows it.
                let key = (!typed.is_empty()).then_some(typed);
                self.write(
                    move || store::set_service(&path, service, key.as_deref()),
                    Selection::Service,
                    "Saved the service".into(),
                    cx,
                );
            }
            Form::Preset {
                name,
                alias,
                from,
                to,
            } => {
                let preset = PresetConfig {
                    name: name.read(cx).text().trim().to_string(),
                    alias: Some(alias.read(cx).text().to_string()),
                    from: from.read(cx).value().map(str::to_string),
                    to: lumen_core::translate::Targets::Many(
                        to.iter()
                            .filter_map(|select| select.read(cx).value().map(str::to_string))
                            .collect(),
                    ),
                };
                let saved = preset.name.clone();
                let replacing = match &self.selection {
                    Selection::Preset(name) => Some(name.clone()),
                    _ => None,
                };
                self.write(
                    move || match replacing {
                        Some(old) => store::replace(&path, &old, preset),
                        None => store::add(&path, preset),
                    },
                    Selection::Preset(saved.clone()),
                    format!("Saved {saved}"),
                    cx,
                );
            }
        }
    }

    /// Run `write` off the UI thread; on success select `then` and re-read.
    fn write(
        &mut self,
        write: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
        then: Selection,
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
                        page.warning = None;
                        page.selection = then;
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the translation settings: {error:#}");
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
        if !self.may_leave(cx) {
            return;
        }
        let path = self.path.clone();
        let service = self.config.service;
        self.write(
            move || store::set_service(&path, service, Some("")),
            Selection::Service,
            "Removed the DeepL key".into(),
            cx,
        );
    }

    // -- Deleting ---------------------------------------------------------------

    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Selection::Preset(name) = &self.selection {
            self.confirming = Some(name.clone());
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
        self.focus(window, cx);
        let path = self.path.clone();
        let removed = name.clone();
        self.edited = false;
        self.write(
            move || store::remove(&path, &removed),
            Selection::Service,
            format!("Deleted {name}"),
            cx,
        );
    }

    // -- What the host asks -------------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        match &self.form {
            Form::Service { service, key } => {
                service.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
                key.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
            }
            Form::Preset {
                name,
                alias,
                from,
                to,
            } => {
                for field in [name, alias] {
                    field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
                }
                for select in std::iter::once(from).chain(to) {
                    select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
                }
            }
        }
        self.colors = colors;
        cx.notify();
    }

    /// What the footer says. Unsaved changes first: a "Saved" from before
    /// them is no longer true.
    pub fn status(&self) -> Option<SharedString> {
        if let Some(warning) = &self.warning {
            return Some(warning.clone());
        }
        if self.edited {
            return Some("Unsaved changes  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }
}

/// A new preset starts as "detect, into English": one choice from done.
fn new_preset() -> PresetConfig {
    PresetConfig {
        to: lumen_core::translate::Targets::One("en".into()),
        ..PresetConfig::default()
    }
}

fn service_form(config: &TranslateConfig, colors: &Colors, cx: &mut App) -> Form {
    let choices = vec![
        Choice {
            value: Some("google".into()),
            label: "Google Translate (no key needed)".into(),
        },
        Choice {
            value: Some("deepl".into()),
            label: "DeepL (your API key)".into(),
        },
    ];
    let selected = match config.service {
        Service::Google => 0,
        Service::Deepl => 1,
    };
    let placeholder = match config.deepl_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => format!(
            "Saved, ending in \u{2026}{}. Type to replace it.",
            key.chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        ),
        _ => "Paste your DeepL API key".into(),
    };
    Form::Service {
        service: cx.new(|cx| Select::new(choices, selected, colors.clone(), cx)),
        key: cx.new(|cx| TextField::new(String::new(), placeholder, colors.clone(), cx)),
    }
}

fn preset_form(preset: &PresetConfig, colors: &Colors, cx: &mut App) -> Form {
    let text = |value: String, placeholder: &'static str, cx: &mut App| {
        cx.new(|cx| TextField::new(value, placeholder, colors.clone(), cx))
    };
    let name = text(preset.name.clone(), "Spanish \u{2192} English", cx);
    let alias = text(
        preset.alias.clone().unwrap_or_default(),
        "Optional: another word to find it by, like esen",
        cx,
    );

    let from_code = preset
        .from
        .as_deref()
        .and_then(languages::find)
        .map(|language| language.code);
    let (choices, selected) = language_choices("Detect automatically", from_code);
    let from = cx.new(|cx| Select::new(choices, selected, colors.clone(), cx));

    let targets = match &preset.to {
        lumen_core::translate::Targets::One(code) => vec![code.clone()],
        lumen_core::translate::Targets::Many(codes) => codes.clone(),
    };
    let to = std::array::from_fn(|index| {
        let code = targets
            .get(index)
            .and_then(|code| languages::find(code))
            .map(|language| language.code);
        // The first target is required, so it has no "none".
        let (mut choices, mut selected) = language_choices("\u{2014}", code);
        if index == 0 {
            choices.remove(0);
            selected = selected.saturating_sub(1);
        }
        cx.new(|cx| Select::new(choices, selected, colors.clone(), cx))
    });

    Form::Preset {
        name,
        alias,
        from,
        to,
    }
}

/// `none` and then every language, with `current` selected.
fn language_choices(none: &str, current: Option<&str>) -> (Vec<Choice>, usize) {
    let choices: Vec<Choice> = std::iter::once(Choice {
        value: None,
        label: none.to_string(),
    })
    .chain(LANGUAGES.iter().map(|language| Choice {
        value: Some(language.code.to_string()),
        label: language.name.to_string(),
    }))
    .collect();
    let selected = current
        .and_then(|code| {
            choices
                .iter()
                .position(|choice| choice.value.as_deref() == Some(code))
        })
        .unwrap_or(0);
    (choices, selected)
}

impl Render for TranslatePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.confirming.is_none() {
            if std::mem::take(&mut self.refocus_form) {
                self.focus_form(window, cx);
            } else if window.focused(cx).is_none() {
                // A rebuilt form takes the focused field with it, and a
                // window with nothing focused hears no keys, not even Escape.
                self.focus(window, cx);
            }
        }
        let colors = self.colors.clone();

        div()
            .size_full()
            .flex()
            .overflow_hidden()
            .child(self.render_list(&colors, cx))
            .child(self.render_form(&colors, window, cx))
    }
}

impl TranslatePage {
    fn render_list(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |id: SharedString, label: String, detail: String, selected: bool| {
            div()
                .id(id)
                .h(px(metrics::SIDEBAR_ROW_HEIGHT))
                .flex_none()
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
                .cursor_pointer()
                .child(div().overflow_hidden().truncate().child(label))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(metrics::text::HINT))
                        .text_color(color(colors.text_muted))
                        .child(detail),
                )
        };

        let service = row(
            "translate-service".into(),
            "Service".into(),
            self.config.service.label().into(),
            self.selection == Selection::Service,
        )
        .on_click(cx.listener(|page, _, window, cx| page.select(Selection::Service, window, cx)));

        let presets = self
            .config
            .presets
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let name = preset.name.trim().to_string();
                let selected = self.selection == Selection::Preset(name.clone());
                let detail = match lumen_core::translate::resolve(preset) {
                    Ok(resolved) => resolved
                        .to
                        .iter()
                        .map(|language| language.code.to_uppercase())
                        .collect::<Vec<_>>()
                        .join(" "),
                    // Skipped by the loader; shown so it can be fixed here.
                    Err(_) => "not used".into(),
                };
                row(
                    SharedString::from(format!("translate-preset-{index}")),
                    name.clone(),
                    detail,
                    selected,
                )
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.select(Selection::Preset(name.clone()), window, cx)
                }))
            });

        let heading = div()
            .px(px(metrics::PADDING))
            .pt(px(metrics::GAP))
            .text_size(px(metrics::text::HINT))
            .text_color(color(colors.text_muted))
            .child(if self.config.presets.is_empty() {
                "No presets yet: Translate uses Any Language \u{2192} English."
            } else {
                "Presets"
            });

        let body = match &self.problem {
            Some(problem) => div()
                .flex_1()
                .p(px(metrics::PADDING))
                .text_size(px(metrics::text::HINT))
                .text_color(color(colors.text_error))
                .child(problem.clone())
                .into_any_element(),
            None => div()
                .id("translate-list")
                .key_context(LIST_CONTEXT)
                .track_focus(&self.list_focus)
                .track_scroll(&self.list_scroll)
                .on_action(cx.listener(Self::previous_preset))
                .on_action(cx.listener(Self::next_preset))
                .on_action(cx.listener(Self::edit_preset))
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(service)
                .child(heading)
                .children(presets)
                .into_any_element(),
        };

        let deletable = matches!(self.selection, Selection::Preset(_));
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
                    .when(self.problem.is_none(), |column| {
                        column.child(text_button(
                            "new-preset",
                            "New preset",
                            colors,
                            cx.listener(|page, _, window, cx| page.new_preset(window, cx)),
                        ))
                    })
                    .when(deletable, |column| {
                        column.child(text_button(
                            "delete-preset",
                            "Delete this preset",
                            colors,
                            cx.listener(|page, _, window, cx| page.ask_delete(window, cx)),
                        ))
                    }),
            )
    }

    fn render_form(&self, colors: &Colors, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.problem.is_some() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(metrics::text::LABEL))
                .text_color(color(colors.text_muted))
                .child("Fix config.json, and this page comes back by itself.")
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

        let (rows, hints): (Vec<AnyElement>, Vec<&'static str>) = match &self.form {
            Form::Service { service, key } => {
                let mut rows = vec![
                    field_row("Service", service.clone().into_any_element()).into_any_element(),
                    field_row("DeepL key", key.clone().into_any_element()).into_any_element(),
                ];
                if self.config.deepl_key.is_some() {
                    rows.push(
                        div()
                            .h(px(layout::form::ROW_HEIGHT))
                            .pl(px(layout::form::LABEL_WIDTH))
                            .flex()
                            .items_center()
                            .child(text_button(
                                "remove-deepl-key",
                                "Remove saved key",
                                colors,
                                cx.listener(|page, _, _, cx| page.remove_key(cx)),
                            ))
                            .into_any_element(),
                    );
                }
                (
                    rows,
                    vec![
                        "Who receives what you type. Never both: a failure is shown, not retried elsewhere.",
                        "Only used by DeepL. Free keys end in :fx. Left empty, the saved key is kept.",
                    ],
                )
            }
            Form::Preset {
                name,
                alias,
                from,
                to,
            } => {
                let mut rows = vec![
                    field_row("Name", name.clone().into_any_element()).into_any_element(),
                    field_row("Alias", alias.clone().into_any_element()).into_any_element(),
                    field_row("From", from.clone().into_any_element()).into_any_element(),
                ];
                for (index, select) in to.iter().enumerate() {
                    let text = if index == 0 { "To" } else { "Also to" };
                    rows.push(
                        field_row(text, select.clone().into_any_element()).into_any_element(),
                    );
                }
                (
                    rows,
                    vec![
                        "Its row in the launcher, and the pill in the translate view.",
                        "Another word the launcher finds it by.",
                        "Detecting works for most text; naming it helps with short words.",
                        "Each language gets its own row in the view.",
                        "Up to four languages, each its own row.",
                        "Up to four languages, each its own row.",
                        "Up to four languages, each its own row.",
                    ],
                )
            }
        };

        let (message, is_error) = match (&self.error, self.focused(window, cx)) {
            (Some(error), _) => (error.clone(), true),
            (None, Some(index)) => (hints.get(index).copied().unwrap_or("").to_string(), false),
            (None, None) => (
                match self.selection {
                    Selection::New => "A new preset. Enter saves it.",
                    _ => "Enter or Ctrl+S saves. Esc undoes, then goes back to the list.",
                }
                .to_string(),
                false,
            ),
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

    /// The delete dialog, over the whole window, drawn by the host.
    pub fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let name = self.confirming.clone()?;
        let colors = &self.colors;

        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "preset-dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "preset-dialog-confirm",
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
                    .child("It is removed from config.json, with its row in the launcher."),
            )
            .child(buttons);

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

    #[test]
    fn every_language_is_offered_after_the_none_choice() {
        let (choices, selected) = language_choices("Detect automatically", None);
        assert_eq!(choices.len(), LANGUAGES.len() + 1);
        assert_eq!(choices[0].value, None);
        assert_eq!(selected, 0);
    }

    #[test]
    fn the_saved_language_is_selected() {
        let (choices, selected) = language_choices("\u{2014}", Some("fr"));
        assert_eq!(choices[selected].value.as_deref(), Some("fr"));
        assert_eq!(choices[selected].label, "French");
    }
}
