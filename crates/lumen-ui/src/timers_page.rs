//! The Timers page of the settings window (F8, slice 2).
//!
//! The general settings -- the alert sound, how long it rings, the quick
//! durations -- and the presets, side by side, in the Translation
//! page's shape: a list on the left, the selected thing's form on the right,
//! a question before a delete, and the same keys -- it shares that page's
//! key contexts and list actions, so there is one way to move through a list
//! of presets, not two.
//!
//! # The file stays the source of truth
//!
//! The page reads `config.json` for its list and re-reads it after every save
//! and whenever the file changes. Every write goes through
//! `lumen_core::timers::store`, which changes only the `timers` section and
//! finds a preset by name. The daemon's watcher then updates the launcher and
//! an open *Timers* view, exactly as after a hand edit.
//!
//! # A sound is heard as it is chosen
//!
//! Choosing an alert sound in the dropdown plays it once, through the daemon,
//! so choosing is previewing. The list is the platform's, read when the
//! window opens.
//!
//! # Typed, in the launcher's grammar
//!
//! A duration is typed as it would be in the launcher -- `25m`, `1h30`,
//! `1:30` -- and checked by the same parser, so what saves here is what the
//! launcher would have understood.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, ScrollHandle, SharedString, Window,
    div, px,
};
use lumen_core::layout::{self, settings as metrics};
use lumen_core::theme::Colors;
use lumen_core::timers::{PresetConfig, Ring, TimersConfig, duration, presets, sounds, store};

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::quicklinks_page::{CancelDialog, ConfirmDialog, DIALOG_CONTEXT};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::{EditPreset, FORM_CONTEXT, LIST_CONTEXT, NextPreset, PreviousPreset};

/// What the list has selected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    General,
    /// A preset on disk, by name.
    Preset(String),
    /// A preset not saved yet.
    New,
}

enum Form {
    General {
        sound: Entity<Select>,
        ring: Entity<Select>,
        durations: Entity<TextField>,
    },
    Preset {
        name: Entity<TextField>,
        alias: Entity<TextField>,
        duration: Entity<TextField>,
    },
}

impl Form {
    fn fields(&self) -> Vec<&Entity<TextField>> {
        match self {
            Form::General { durations, .. } => vec![durations],
            Form::Preset {
                name,
                alias,
                duration,
            } => vec![name, alias, duration],
        }
    }
}

/// What the page asks of the window that hosts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimersPageEvent {
    /// Play this alert sound once: it was just chosen.
    Preview(String),
}

impl gpui::EventEmitter<TimersPageEvent> for TimersPage {}

pub struct TimersPage {
    path: PathBuf,
    colors: Colors,
    /// The system's alert sounds, by name, once the window has read them.
    sounds: Vec<String>,
    /// The section as the file last read had it.
    config: TimersConfig,
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

impl TimersPage {
    pub fn new(path: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = TimersConfig::default();
        let form = general_form(&config, &[], &colors, cx);
        let mut page = Self {
            path,
            colors,
            sounds: Vec::new(),
            config,
            problem: None,
            selection: Selection::General,
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
                        log::warn!("the timers page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                if let Selection::Preset(name) = &page.selection
                    && page.find(name).is_none()
                {
                    page.selection = Selection::General;
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
            Selection::General => general_form(&self.config, &self.sounds, &self.colors, cx),
            Selection::Preset(name) => {
                let preset = self.find(name).cloned().unwrap_or_default();
                preset_form(&preset, &self.colors, cx)
            }
            Selection::New => preset_form(&PresetConfig::default(), &self.colors, cx),
        };
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    /// Take the system's alert sounds, for the General row's dropdown.
    pub fn set_sounds(&mut self, sounds: Vec<String>, cx: &mut Context<Self>) {
        self.sounds = sounds;
        if self.selection == Selection::General && !self.edited {
            self.rebuild(cx);
        }
        cx.notify();
    }

    /// Mark the form edited on any change to it.
    fn watch_form(&mut self, cx: &mut Context<Self>) {
        let fields: Vec<Entity<TextField>> = self.form.fields().into_iter().cloned().collect();
        for field in fields {
            cx.subscribe(&field, |page, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    page.touched(cx);
                }
            })
            .detach();
        }
        if let Form::General { sound, ring, .. } = &self.form {
            // Chosen is heard: the host plays it once.
            cx.subscribe(sound, |page, select, SelectEvent::Changed, cx| {
                if let Some(name) = select.read(cx).value() {
                    cx.emit(TimersPageEvent::Preview(name.to_string()));
                }
                page.touched(cx);
            })
            .detach();
            cx.subscribe(ring, |page, _, SelectEvent::Changed, cx| page.touched(cx))
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
        std::iter::once(Selection::General)
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

    /// Focus the list, which always has at least the General row.
    pub fn focus(&self, window: &mut Window, _cx: &App) {
        window.focus(&self.list_focus);
    }

    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        match &self.form {
            Form::General {
                sound,
                ring,
                durations,
            } => vec![
                sound.read(cx).focus_handle().clone(),
                ring.read(cx).focus_handle().clone(),
                durations.focus_handle(cx),
            ],
            Form::Preset { .. } => self
                .form
                .fields()
                .into_iter()
                .map(|field| field.focus_handle(cx))
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
                self.selection = Selection::General;
                self.rebuild(cx);
            }
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.status = Some("Changes discarded".into());
        if self.selection == Selection::New {
            self.selection = Selection::General;
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
            Form::General {
                sound,
                ring,
                durations,
            } => {
                let sound = sound.read(cx).value().map(str::to_string);
                let ring = match ring.read(cx).value() {
                    Some("until-dismissed") => Ring::UntilDismissed,
                    _ => Ring::Once,
                };
                let durations: Vec<String> = durations
                    .read(cx)
                    .text()
                    .split(',')
                    .map(|written| written.trim().to_string())
                    .collect();
                self.write(
                    move || store::set_general(&path, sound.as_deref(), ring, &durations),
                    Selection::General,
                    "Saved the general settings".into(),
                    cx,
                );
            }
            Form::Preset {
                name,
                alias,
                duration,
            } => {
                let preset = store::preset(
                    name.read(cx).text().trim(),
                    alias.read(cx).text(),
                    duration.read(cx).text(),
                );
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
                        log::warn!("could not save the timer settings: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
            Selection::General,
            format!("Deleted {name}"),
            cx,
        );
    }

    // -- What the host asks -------------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        let fields: Vec<Entity<TextField>> = self.form.fields().into_iter().cloned().collect();
        for field in fields {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        if let Form::General { sound, ring, .. } = &self.form {
            for select in [sound, ring] {
                select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
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

fn general_form(config: &TimersConfig, names: &[String], colors: &Colors, cx: &mut App) -> Form {
    let (choices, selected) = sound_choices(names, config.alert_sound.as_deref());
    let rings = [Ring::Once, Ring::UntilDismissed];
    let ring_choices = rings
        .iter()
        .map(|ring| Choice {
            value: serde_json::to_value(ring)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string)),
            label: ring.label().to_string(),
        })
        .collect();
    let ring_selected = rings
        .iter()
        .position(|ring| *ring == config.ring)
        .unwrap_or(0);
    let written = config.quick_start.join(", ");
    Form::General {
        sound: cx.new(|cx| Select::new(choices, selected, colors.clone(), cx)),
        ring: cx.new(|cx| Select::new(ring_choices, ring_selected, colors.clone(), cx)),
        durations: cx.new(|cx| TextField::new(written, "2m, 5m, 10m, 30m", colors.clone(), cx)),
    }
}

/// "No sound", then every sound the system has, with `current` selected.
///
/// A saved name the system does not list -- the list has not arrived yet,
/// or the file was edited by hand -- is kept as a choice of its own, so
/// saving another field never changes the sound.
fn sound_choices(names: &[String], current: Option<&str>) -> (Vec<Choice>, usize) {
    let mut choices = vec![Choice {
        value: None,
        label: "No sound".into(),
    }];
    let listed = current.is_none_or(|current| names.iter().any(|name| name == current));
    let extra = (!listed).then_some(current).flatten();
    for name in names.iter().map(String::as_str).chain(extra) {
        choices.push(Choice {
            value: Some(name.to_string()),
            label: sounds::label(name),
        });
    }
    let selected = choices
        .iter()
        .position(|choice| choice.value.as_deref() == current)
        .unwrap_or(0);
    (choices, selected)
}

fn preset_form(preset: &PresetConfig, colors: &Colors, cx: &mut App) -> Form {
    let text = |value: String, placeholder: &'static str, cx: &mut App| {
        cx.new(|cx| TextField::new(value, placeholder, colors.clone(), cx))
    };
    Form::Preset {
        name: text(preset.name.clone(), "Deep work", cx),
        alias: text(
            preset.alias.clone().unwrap_or_default(),
            "Optional: one word to start it by, like dw",
            cx,
        ),
        duration: text(preset.duration.clone(), "25m, 1h30 or 1:30", cx),
    }
}

/// What the list says beside a preset: its length, or that the launcher
/// skips it -- shown so it can be fixed here.
fn detail(preset: &PresetConfig) -> String {
    match presets::resolve(preset) {
        Ok(resolved) => duration::describe(resolved.duration),
        Err(_) => "not used".into(),
    }
}

impl Render for TimersPage {
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

impl TimersPage {
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

        let quick = row(
            "timers-general".into(),
            "General".into(),
            self.config
                .alert_sound
                .as_deref()
                .map_or("Silent".into(), sounds::label),
            self.selection == Selection::General,
        )
        .on_click(cx.listener(|page, _, window, cx| page.select(Selection::General, window, cx)));

        let presets = self
            .config
            .presets
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let name = preset.name.trim().to_string();
                let selected = self.selection == Selection::Preset(name.clone());
                row(
                    SharedString::from(format!("timers-preset-{index}")),
                    name.clone(),
                    detail(preset),
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
                "No presets yet: name a timer you start often."
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
                .id("timers-list")
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
                .child(quick)
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
                            "new-timer-preset",
                            "New preset",
                            colors,
                            cx.listener(|page, _, window, cx| page.new_preset(window, cx)),
                        ))
                    })
                    .when(deletable, |column| {
                        column.child(text_button(
                            "delete-timer-preset",
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
            Form::General {
                sound,
                ring,
                durations,
            } => (
                vec![
                    field_row("Alert sound", sound.clone().into_any_element()).into_any_element(),
                    field_row("Ring", ring.clone().into_any_element()).into_any_element(),
                    field_row("Quick start", durations.clone().into_any_element())
                        .into_any_element(),
                ],
                vec![
                    "Played when a timer ends. Choosing one plays it.",
                    "Until dismissed: the sound loops until Dismiss Ringing Timer.",
                    "Offered under Quick Start in Timers, shortest first. Comma separated.",
                ],
            ),
            Form::Preset {
                name,
                alias,
                duration,
            } => (
                vec![
                    field_row("Name", name.clone().into_any_element()).into_any_element(),
                    field_row("Alias", alias.clone().into_any_element()).into_any_element(),
                    field_row("Duration", duration.clone().into_any_element()).into_any_element(),
                ],
                vec![
                    "Its row in the launcher, and the timer's name while it runs.",
                    "One word the launcher finds it by.",
                    "As you would type it in the launcher: 25m, 1h30, 1:30.",
                ],
            ),
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
                "timer-dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "timer-dialog-confirm",
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
                    .child(
                        "It is removed from config.json, with its row in the launcher. \
                         A timer it started keeps running.",
                    ),
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
    fn the_saved_sound_is_selected_even_before_the_list_arrives() {
        let names = vec!["Alarm01".to_string(), "Ring02".to_string()];

        let (choices, selected) = sound_choices(&names, Some("Ring02"));
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[selected].label, "Ring 2");

        let (choices, selected) = sound_choices(&[], Some("Alarm05"));
        assert_eq!(choices[selected].value.as_deref(), Some("Alarm05"));

        let (choices, selected) = sound_choices(&names, None);
        assert_eq!(choices[selected].label, "No sound");
    }

    #[test]
    fn a_preset_the_launcher_skips_says_so_in_the_list() {
        let good = store::preset("Tea", "", "3m");
        let bad = store::preset("Tea", "", "soon");
        assert_eq!(detail(&good), "3 min");
        assert_eq!(detail(&bad), "not used");
    }
}
