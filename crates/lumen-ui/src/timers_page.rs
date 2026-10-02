//! The Timers page of the settings window (F8).
//!
//! Three kinds of row in the list on the left, each with its form on the
//! right: *General* -- the alert sound, how long it rings, the quick
//! durations -- then *Focus* -- what a focus timer keeps Lumen from opening
//! -- then the presets. The Translation page's shape and keys: it shares that
//! page's list context and actions, so there is one way to move through a
//! list of presets, not two.
//!
//! # A preset is the launcher's form
//!
//! A preset's form is [`TimerForm`], the same entity the launcher shows for
//! *Create Timer Preset*, so what can be made here can be made there. This
//! page hosts it and reacts when it saves or is left; the General and Focus
//! forms are this page's own.
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
use crate::settings_ui as ui;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::timer_form::{TimerForm, TimerFormEvent};
use crate::translate_page::{EditPreset, FORM_CONTEXT, LIST_CONTEXT, NextPreset, PreviousPreset};

/// What the list has selected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    General,
    Focus,
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
    Focus {
        apps: Entity<TextField>,
        sites: Entity<TextField>,
    },
    Preset(Entity<TimerForm>),
}

impl Form {
    fn fields(&self) -> Vec<&Entity<TextField>> {
        match self {
            Form::General { durations, .. } => vec![durations],
            Form::Focus { apps, sites } => vec![apps, sites],
            Form::Preset(_) => Vec::new(),
        }
    }

    /// Every dropdown of the page's own forms, so each is watched, themed
    /// and reached with Tab.
    fn selects(&self) -> Vec<&Entity<Select>> {
        match self {
            Form::General { sound, ring, .. } => vec![sound, ring],
            Form::Focus { .. } | Form::Preset(_) => Vec::new(),
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
    /// Whether the General or Focus form has changed since it was filled in
    /// or saved. A preset's form keeps its own.
    edited: bool,
    saving: bool,
    /// Why the last save was refused, under the form.
    error: Option<String>,
    status: Option<SharedString>,
    /// The preset waiting on the delete dialog.
    confirming: Option<String>,
    /// Where the user asked to go while the form held unsaved changes: the
    /// question on screen is whether to save them first.
    leaving: Option<Selection>,
    /// Where to go once the save asked for on the way there succeeds.
    continue_to: Option<Selection>,
    dialog_focus: FocusHandle,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    /// Put the caret back in the form at the next frame: undoing rebuilds it
    /// with no `Window` at hand to focus the new fields with.
    refocus_form: bool,
    /// Put focus back on the list at the next frame, likewise.
    refocus_list: bool,
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
            confirming: None,
            leaving: None,
            continue_to: None,
            dialog_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            refocus_form: false,
            refocus_list: false,
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
                if !page.dirty(cx) {
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

    /// Whether the form on screen holds changes that are not saved.
    fn dirty(&self, cx: &App) -> bool {
        match &self.form {
            Form::Preset(form) => form.read(cx).is_edited(),
            _ => self.edited,
        }
    }

    /// Fill the form in again from the file, for what is selected.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.form = match &self.selection {
            Selection::General => general_form(&self.config, &self.sounds, &self.colors, cx),
            Selection::Focus => focus_form(&self.config, &self.colors, cx),
            Selection::Preset(name) => {
                let preset = self.find(name).cloned().unwrap_or_default();
                self.preset_form(preset, Some(name.clone()), cx)
            }
            Selection::New => self.preset_form(PresetConfig::default(), None, cx),
        };
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    fn preset_form(
        &self,
        preset: PresetConfig,
        replacing: Option<String>,
        cx: &mut Context<Self>,
    ) -> Form {
        let (path, colors) = (self.path.clone(), self.colors.clone());
        let form = cx.new(|cx| TimerForm::new(path, preset, replacing, colors, cx));
        cx.observe(&form, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&form, |page, form, event, cx| match event {
            TimerFormEvent::Saved(name) => {
                page.status = Some(format!("Saved {name}").into());
                page.selection = page
                    .continue_to
                    .take()
                    .unwrap_or_else(|| Selection::Preset(name.clone()));
                page.refocus_form = page.selection == Selection::New;
                page.reload(cx);
            }
            // Refused: stay, and let the form say why.
            TimerFormEvent::Failed => page.continue_to = None,
            TimerFormEvent::Cancelled => {
                if form.read(cx).is_edited() {
                    page.status = Some("Changes discarded".into());
                    page.rebuild(cx);
                    page.refocus_form = !matches!(page.selection, Selection::New);
                }
                if page.selection == Selection::New {
                    page.selection = Selection::General;
                    page.rebuild(cx);
                }
                page.refocus_list = !page.refocus_form;
                cx.notify();
            }
        })
        .detach();
        Form::Preset(form)
    }

    /// Take the system's alert sounds, for the General row's dropdown.
    pub fn set_sounds(&mut self, sounds: Vec<String>, cx: &mut Context<Self>) {
        self.sounds = sounds;
        if self.selection == Selection::General && !self.edited {
            self.rebuild(cx);
        }
        cx.notify();
    }

    /// Mark the page's own forms edited on any change to them.
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
        let selects: Vec<Entity<Select>> = self.form.selects().into_iter().cloned().collect();
        for select in selects {
            cx.subscribe(&select, |page, _, SelectEvent::Changed, cx| {
                page.touched(cx)
            })
            .detach();
        }
        if let Form::General { sound, .. } = &self.form {
            // Chosen is heard: the host plays it once.
            cx.subscribe(sound, |_, select, SelectEvent::Changed, cx| {
                if let Some(name) = select.read(cx).value() {
                    cx.emit(TimersPageEvent::Preview(name.to_string()));
                }
            })
            .detach();
        }
    }

    fn touched(&mut self, cx: &mut Context<Self>) {
        self.edited = true;
        self.error = None;
        cx.notify();
    }

    /// Whether the form may be replaced now. With unsaved changes in it, it
    /// may not: the page asks what to do with them, and goes to `to` once
    /// that is answered.
    fn may_leave(&mut self, to: Selection, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.dirty(cx) {
            return true;
        }
        self.leaving = Some(to);
        window.focus(&self.dialog_focus);
        cx.notify();
        false
    }

    /// *Discard* in the unsaved-changes dialog: forget them, and go.
    fn discard_and_leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(to) = self.leaving.take() else {
            return;
        };
        self.edited = false;
        self.status = Some("Changes discarded".into());
        self.selection = to;
        self.rebuild(cx);
        if self.selection == Selection::New {
            self.focus_form(window, cx);
        } else {
            self.focus(window, cx);
        }
        cx.notify();
    }

    fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if selection != self.selection {
            if !self.may_leave(selection.clone(), window, cx) {
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
        [Selection::General, Selection::Focus]
            .into_iter()
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
        if !self.may_leave(Selection::New, window, cx) {
            return;
        }
        self.selection = Selection::New;
        self.status = None;
        self.rebuild(cx);
        self.focus_form(window, cx);
        cx.notify();
    }

    // -- Focus ----------------------------------------------------------------

    /// Focus the list, which always has at least the General and Focus rows.
    pub fn focus(&self, window: &mut Window, _cx: &App) {
        window.focus(&self.list_focus);
    }

    /// The Tab stops of the page's own forms; a preset's form has its own.
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
            Form::Focus { apps, sites } => vec![apps.focus_handle(cx), sites.focus_handle(cx)],
            Form::Preset(_) => Vec::new(),
        }
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.stops(cx)
            .iter()
            .position(|stop| stop.is_focused(window))
    }

    fn focus_form(&self, window: &mut Window, cx: &App) {
        if let Form::Preset(form) = &self.form {
            form.read(cx).focus(window, cx);
            return;
        }
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        if stops.is_empty() {
            return;
        }
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

    /// Escape in the General or Focus form: undo what was typed, or back to
    /// the list.
    fn cancel(&mut self, _: &CancelForm, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edited {
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.status = Some("Changes discarded".into());
        self.rebuild(cx);
        self.refocus_form = true;
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
        let list = |field: &Entity<TextField>, cx: &App| -> Vec<String> {
            field
                .read(cx)
                .text()
                .split(',')
                .map(|written| written.trim().to_string())
                .collect()
        };

        match &self.form {
            Form::General {
                sound,
                ring,
                durations,
            } => {
                let general = store::General {
                    sound: sound.read(cx).value().map(str::to_string),
                    ring: match ring.read(cx).value() {
                        Some("until-dismissed") => Ring::UntilDismissed,
                        _ => Ring::Once,
                    },
                    durations: list(durations, cx),
                };
                self.write(
                    move || store::set_general(&path, &general),
                    Selection::General,
                    "Saved the general settings".into(),
                    cx,
                );
            }
            Form::Focus { apps, sites } => {
                let (apps, sites) = (list(apps, cx), list(sites, cx));
                self.write(
                    move || store::set_blocked(&path, &apps, &sites),
                    Selection::Focus,
                    "Saved what focus blocks".into(),
                    cx,
                );
            }
            Form::Preset(form) => form.update(cx, |form, cx| form.save(cx)),
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
                        page.selection = page.continue_to.take().unwrap_or(then);
                        page.refocus_form = page.selection == Selection::New;
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        page.continue_to = None;
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

    /// Esc: no delete, or back to the form that has the changes.
    fn cancel_dialog(&mut self, _: &CancelDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = None;
        if self.leaving.take().is_some() {
            self.focus_form(window, cx);
        } else {
            self.focus(window, cx);
        }
        cx.notify();
    }

    /// Enter: delete; or save the changes, then go where the user asked.
    fn confirm_dialog(&mut self, _: &ConfirmDialog, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(to) = self.leaving.take() {
            self.continue_to = Some(to);
            self.focus(window, cx);
            self.save(cx);
            cx.notify();
            return;
        }
        let Some(name) = self.confirming.take() else {
            return;
        };
        self.focus(window, cx);
        let path = self.path.clone();
        let removed = name.clone();
        self.edited = false;
        // The form is about to go with the preset: nothing in it to keep.
        self.selection = Selection::General;
        self.rebuild(cx);
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
        let selects: Vec<Entity<Select>> = self.form.selects().into_iter().cloned().collect();
        for select in selects {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        }
        if let Form::Preset(form) = &self.form {
            form.update(cx, |form, cx| form.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// What the footer says. Unsaved changes first: a "Saved" from before
    /// them is no longer true.
    pub fn status(&self, cx: &App) -> Option<SharedString> {
        if self.dirty(cx) {
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

fn focus_form(config: &TimersConfig, colors: &Colors, cx: &mut App) -> Form {
    Form::Focus {
        apps: cx.new(|cx| {
            let written = config.blocked_apps.join(", ");
            TextField::new(written, "Discord, Steam", colors.clone(), cx)
        }),
        sites: cx.new(|cx| {
            let written = config.blocked_sites.join(", ");
            TextField::new(written, "youtube.com, reddit.com", colors.clone(), cx)
        }),
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

/// What the list says beside a preset: its length, or that the launcher
/// skips it -- shown so it can be fixed here.
fn detail(preset: &PresetConfig) -> String {
    match presets::resolve(preset) {
        Ok(resolved) => match resolved.cycle {
            Some(plan) => format!(
                "{} \u{00d7} {}",
                plan.rounds,
                duration::describe(resolved.duration)
            ),
            None => duration::describe(resolved.duration),
        },
        Err(_) => "not used".into(),
    }
}

impl Render for TimersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.confirming.is_none() && self.leaving.is_none() {
            if std::mem::take(&mut self.refocus_form) {
                self.focus_form(window, cx);
            } else if std::mem::take(&mut self.refocus_list) || window.focused(cx).is_none() {
                // A rebuilt form takes the focused field with it, and a
                // window with nothing focused hears no keys, not even Escape.
                self.focus(window, cx);
            }
        }
        let colors = self.colors.clone();

        ui::split_page()
            .child(self.render_list(&colors, cx))
            .child(ui::editor_card(&colors).child(self.render_form(&colors, window, cx)))
    }
}

impl TimersPage {
    fn render_list(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let row = |id: SharedString, label: String, detail: String, selected: bool| {
            div()
                .id(id)
                .h(px(metrics::NAV_ROW_HEIGHT))
                .flex_none()
                .mx(px(metrics::LIST_INSET))
                .rounded(px(metrics::NAV_RADIUS))
                .px(px(metrics::CARD_PADDING_X - metrics::LIST_INSET))
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

        let general = row(
            "timers-general".into(),
            "General".into(),
            self.config
                .alert_sound
                .as_deref()
                .map_or("Silent".into(), sounds::label),
            self.selection == Selection::General,
        )
        .on_click(cx.listener(|page, _, window, cx| page.select(Selection::General, window, cx)));

        let blocked = self.config.blocked_apps.len() + self.config.blocked_sites.len();
        let focus = row(
            "timers-focus".into(),
            "Focus".into(),
            match blocked {
                0 => "Nothing blocked".into(),
                1 => "1 blocked".into(),
                count => format!("{count} blocked"),
            },
            self.selection == Selection::Focus,
        )
        .on_click(cx.listener(|page, _, window, cx| page.select(Selection::Focus, window, cx)));

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
                .child(general)
                .child(focus)
                .child(heading)
                .children(presets)
                .into_any_element(),
        };

        let deletable = matches!(self.selection, Selection::Preset(_));
        ui::list_card(colors).child(body).child(
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

        // A preset's form draws itself, rows, keys and message line.
        if let Form::Preset(form) = &self.form {
            return div()
                .flex_1()
                .py(px(metrics::PADDING))
                .child(form.clone())
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
                .into_any_element()
        };

        let (rows, hints): (Vec<AnyElement>, Vec<&'static str>) = match &self.form {
            Form::General {
                sound,
                ring,
                durations,
            } => (
                vec![
                    field_row("Alert sound", sound.clone().into_any_element()),
                    field_row("Ring", ring.clone().into_any_element()),
                    field_row("Quick start", durations.clone().into_any_element()),
                ],
                vec![
                    "Played when a timer ends. Choosing one plays it.",
                    "Until dismissed: the sound loops until Dismiss Ringing Timer.",
                    "Offered under Quick Start in Timers, shortest first: 5m, 1h30, comma separated.",
                ],
            ),
            Form::Focus { apps, sites } => (
                vec![
                    field_row("Blocked apps", apps.clone().into_any_element()),
                    field_row("Blocked sites", sites.clone().into_any_element()),
                ],
                vec![
                    "Names as the launcher shows them, comma separated. Discord also blocks Discord PTB.",
                    "Domains, comma separated; their subdomains too. Only links opened through Lumen.",
                ],
            ),
            Form::Preset(_) => unreachable!("drawn above"),
        };

        let (message, is_error) = match (&self.error, self.focused(window, cx)) {
            (Some(error), _) => (error.clone(), true),
            (None, Some(index)) => (hints.get(index).copied().unwrap_or("").to_string(), false),
            (None, None) => (
                match self.selection {
                    Selection::Focus => "While a timer with focus on runs, Lumen won't open these.",
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

    /// What the unsaved-changes dialog calls the form it is about.
    fn editing(&self) -> String {
        match &self.selection {
            Selection::General => "General".into(),
            Selection::Focus => "Focus".into(),
            Selection::Preset(name) => name.clone(),
            Selection::New => "the new preset".into(),
        }
    }

    /// The dialog on screen, if one is, over the whole window, drawn by the
    /// host: a delete to confirm, or unsaved changes to save or discard.
    pub fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = &self.colors;
        let (title, body, cancel, confirm, discard) = match (&self.confirming, &self.leaving) {
            (Some(name), _) => (
                format!("Delete {name}?"),
                "It is removed from config.json, with its row in the launcher. \
                 A timer it started keeps running.",
                "Cancel  Esc",
                "Delete  Enter",
                false,
            ),
            (None, Some(_)) => (
                format!("Unsaved changes to {}", self.editing()),
                "Save them before moving on, or discard them.",
                "Keep editing  Esc",
                "Save  Enter",
                true,
            ),
            (None, None) => return None,
        };

        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "timer-dialog-cancel",
                cancel,
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .when(discard, |buttons| {
                buttons.child(text_button(
                    "timer-dialog-discard",
                    "Discard",
                    colors,
                    cx.listener(|page, _, window, cx| page.discard_and_leave(window, cx)),
                ))
            })
            .child(text_button(
                "timer-dialog-confirm",
                confirm,
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
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(metrics::text::HINT))
                    .text_color(color(colors.text_muted))
                    .child(body),
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
        let cycle = PresetConfig {
            short_break: Some("5m".into()),
            rounds: Some(3),
            ..store::preset("Sprint", "", "50m")
        };
        assert_eq!(detail(&cycle), "3 \u{00d7} 50 min");
    }
}
