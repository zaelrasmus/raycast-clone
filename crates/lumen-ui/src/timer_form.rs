//! The form that creates or edits a timer preset (F8).
//!
//! One entity, two hosts, like the quicklink form: the settings window's
//! Timers page shows it beside the list of presets, and the launcher shows it
//! in place of the list for *Create Timer Preset*, *Edit Preset* and *Save
//! as Preset*. It writes through `lumen_core::timers::store` and tells its
//! host how that went; the watcher on `config.json` does the rest.
//!
//! # What is shown is what matters now
//!
//! A timer is a name, a duration and whether it holds focus. A cycle --
//! breaks, rounds, what happens after the set -- is four more fields that
//! mean nothing to a plain timer, so they appear only once *Cycle* is on.
//! The form's height follows, and the host reads it from [`TimerForm::height`].
//!
//! # Keys
//!
//! Its own key context, inside whichever host it is in: Tab and the arrows
//! move between fields, Enter saves, Escape leaves -- the quicklink form's
//! actions, bound again for this context in the keymap (I-14).

use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::layout;
use lumen_core::theme::Colors;
use lumen_core::timers::cycle::MAX_ROUNDS;
use lumen_core::timers::duration::written;
use lumen_core::timers::{PresetConfig, presets, store};

use crate::duration_input::{DurationEvent, DurationInput};
use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::select::{Choice, Select, SelectEvent};
use crate::styling::{color, metrics, type_scale};
use crate::text_field::{TextField, TextFieldEvent};

/// The form's key context.
pub const CONTEXT: &str = "TimerForm";

/// What the form tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerFormEvent {
    /// The preset was written, under this name.
    Saved(String),
    /// Saving was refused; the form says why, under its fields.
    Failed,
    Cancelled,
}

impl gpui::EventEmitter<TimerFormEvent> for TimerForm {}

/// The rows, in the order Tab visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Name,
    Alias,
    Duration,
    Focus,
    Cycle,
    Break,
    LongBreak,
    Rounds,
    Repeat,
}

impl Row {
    const ALWAYS: [Row; 5] = [Row::Name, Row::Alias, Row::Duration, Row::Focus, Row::Cycle];
    const CYCLE: [Row; 4] = [Row::Break, Row::LongBreak, Row::Rounds, Row::Repeat];

    fn label(self) -> &'static str {
        match self {
            Row::Name => "Name",
            Row::Alias => "Alias",
            Row::Duration => "Duration",
            Row::Focus => "Focus",
            Row::Cycle => "Cycle",
            Row::Break => "Break",
            Row::LongBreak => "Long break",
            Row::Rounds => "Rounds",
            Row::Repeat => "After the set",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Row::Name => "Its row in the launcher, and the timer's name while it runs.",
            Row::Alias => "Optional: one word the launcher finds it by.",
            Row::Duration => "How long it counts down. In a cycle, each work round.",
            Row::Focus => "On: Lumen won't open your blocked apps and sites while it runs.",
            Row::Cycle => "On: work and breaks follow each other by themselves, like Pomodoro.",
            Row::Break => "Between work rounds.",
            Row::LongBreak => "Optional: after the last round.",
            Row::Rounds => "How many work rounds in a set: 1 to 12.",
            Row::Repeat => "Start again repeats the set until you stop it.",
        }
    }
}

/// A row's control, for Tab and for drawing.
enum Control<'a> {
    Text(&'a Entity<TextField>),
    Select(&'a Entity<Select>),
    Duration(&'a Entity<DurationInput>),
}

pub struct TimerForm {
    path: PathBuf,
    /// The name of the preset on disk being edited; `None` when creating.
    replacing: Option<String>,
    name: Entity<TextField>,
    alias: Entity<TextField>,
    duration: Entity<DurationInput>,
    focus: Entity<Select>,
    cycle: Entity<Select>,
    /// Kept in step with `cycle`, for the height without an `App`.
    cycling: bool,
    short_break: Entity<DurationInput>,
    long_break: Entity<DurationInput>,
    rounds: Entity<TextField>,
    repeat: Entity<Select>,
    edited: bool,
    error: Option<String>,
    saving: bool,
    colors: Colors,
}

fn off_on(
    off: &str,
    on: &str,
    value: &str,
    is_on: bool,
    colors: &Colors,
    cx: &mut App,
) -> Entity<Select> {
    let choices = vec![
        Choice {
            value: None,
            label: off.into(),
        },
        Choice {
            value: Some(value.into()),
            label: on.into(),
        },
    ];
    cx.new(|cx| Select::new(choices, usize::from(is_on), colors.clone(), cx))
}

/// A duration as written in the file, for the three fields.
fn read(text: Option<&str>) -> Option<Duration> {
    text.and_then(presets::parse_duration)
}

impl TimerForm {
    /// A form showing `preset`; editing the one on disk called `replacing`,
    /// or creating a new one when that is `None`.
    pub fn new(
        path: PathBuf,
        preset: PresetConfig,
        replacing: Option<String>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = |value: String, placeholder: &'static str, cx: &mut Context<Self>| {
            let field = cx.new(|cx| TextField::new(value, placeholder, colors.clone(), cx));
            cx.subscribe(&field, |form, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    form.touched(cx);
                }
            })
            .detach();
            field
        };
        let name = text(preset.name.clone(), "Deep work", cx);
        let alias = text(preset.alias.clone().unwrap_or_default(), "dw", cx);
        let rounds = text(
            preset
                .rounds
                .map(|rounds| rounds.to_string())
                .unwrap_or_default(),
            "4",
            cx,
        );

        let duration_input = |value: Option<Duration>, cx: &mut Context<Self>| {
            let input = cx.new(|cx| DurationInput::new(value, colors.clone(), cx));
            cx.subscribe(&input, |form, _, DurationEvent::Changed, cx| {
                form.touched(cx)
            })
            .detach();
            input
        };
        let duration = duration_input(read(Some(&preset.duration)), cx);
        let short_break = duration_input(read(preset.short_break.as_deref()), cx);
        let long_break = duration_input(read(preset.long_break.as_deref()), cx);

        let cycling = preset.short_break.is_some();
        let focus = off_on("Off", "On", "on", preset.focus, &colors, cx);
        let cycle = off_on("Off", "On", "on", cycling, &colors, cx);
        let repeat = off_on(
            "End",
            "Start again, until stopped",
            "repeat",
            preset.repeat,
            &colors,
            cx,
        );
        for select in [&focus, &repeat] {
            cx.subscribe(select, |form, _, SelectEvent::Changed, cx| form.touched(cx))
                .detach();
        }
        cx.subscribe(&cycle, |form, select, SelectEvent::Changed, cx| {
            form.cycling = select.read(cx).value().is_some();
            form.touched(cx);
        })
        .detach();

        Self {
            path,
            replacing,
            name,
            alias,
            duration,
            focus,
            cycle,
            cycling,
            short_break,
            long_break,
            rounds,
            repeat,
            edited: false,
            error: None,
            saving: false,
            colors,
        }
    }

    fn touched(&mut self, cx: &mut Context<Self>) {
        self.edited = true;
        self.error = None;
        cx.notify();
    }

    /// Whether anything has changed since the form was filled in or saved.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// The name of the preset on disk this form edits, if it edits one.
    pub fn editing_name(&self) -> Option<&str> {
        self.replacing.as_deref()
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Row::ALWAYS.to_vec();
        if self.cycling {
            rows.extend(Row::CYCLE);
        }
        rows
    }

    /// The form's height, which its host reserves for it: more rows once
    /// *Cycle* is on.
    pub fn height(&self) -> f32 {
        layout::form::PADDING_Y * 2.0
            + layout::form::ROW_HEIGHT * self.rows().len() as f32
            + layout::form::MESSAGE_HEIGHT
    }

    /// Open the dropdowns above their fields: for the launcher, whose window
    /// ends just below the form.
    pub fn open_menus_upward(&mut self, cx: &mut Context<Self>) {
        for select in [&self.focus, &self.cycle, &self.repeat] {
            select.update(cx, |select, _| select.open_upward(true));
        }
    }

    fn control(&self, row: Row) -> Control<'_> {
        match row {
            Row::Name => Control::Text(&self.name),
            Row::Alias => Control::Text(&self.alias),
            Row::Duration => Control::Duration(&self.duration),
            Row::Focus => Control::Select(&self.focus),
            Row::Cycle => Control::Select(&self.cycle),
            Row::Break => Control::Duration(&self.short_break),
            Row::LongBreak => Control::Duration(&self.long_break),
            Row::Rounds => Control::Text(&self.rounds),
            Row::Repeat => Control::Select(&self.repeat),
        }
    }

    /// What Tab visits, in order, with the row each stop belongs to.
    fn stops(&self, cx: &App) -> Vec<(Row, FocusHandle)> {
        self.rows()
            .into_iter()
            .flat_map(|row| {
                let handles = match self.control(row) {
                    Control::Text(field) => vec![field.focus_handle(cx)],
                    Control::Select(select) => vec![select.read(cx).focus_handle().clone()],
                    Control::Duration(input) => input.read(cx).stops(cx),
                };
                handles.into_iter().map(move |handle| (row, handle))
            })
            .collect()
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<(usize, Row)> {
        self.stops(cx)
            .into_iter()
            .enumerate()
            .find(|(_, (_, handle))| handle.is_focused(window))
            .map(|(index, (row, _))| (index, row))
    }

    /// Put the caret in the first field.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.name.read(cx).focus(window);
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let next = match self.focused(window, cx) {
            Some((index, _)) => (index as isize + delta).rem_euclid(stops.len() as isize) as usize,
            None => 0,
        };
        window.focus(&stops[next].1);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, window, cx);
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TimerFormEvent::Cancelled);
    }

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// What the fields say, as a preset; or why they cannot be one.
    fn compose(&self, cx: &App) -> Result<PresetConfig, String> {
        let duration = self
            .duration
            .read(cx)
            .value(cx)?
            .ok_or("A timer needs a duration")?;
        let mut preset = PresetConfig {
            duration: written(duration),
            focus: self.focus.read(cx).value().is_some(),
            ..store::preset(
                self.name.read(cx).text().trim(),
                self.alias.read(cx).text(),
                "",
            )
        };
        if self.cycling {
            let short = self
                .short_break
                .read(cx)
                .value(cx)?
                .ok_or("A cycle needs a break between rounds")?;
            let long = self.long_break.read(cx).value(cx)?;
            let rounds = match self.rounds.read(cx).text().trim() {
                "" => None,
                typed => Some(typed.parse::<u32>().map_err(|_| {
                    format!("Rounds is a number from 1 to {MAX_ROUNDS}, not {typed:?}")
                })?),
            };
            preset.short_break = Some(written(short));
            preset.long_break = long.map(written);
            preset.rounds = rounds;
            preset.repeat = self.repeat.read(cx).value().is_some();
        }
        Ok(preset)
    }

    /// Write the preset, off the UI thread. For Enter, and for a host with
    /// its own save key.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let preset = match self.compose(cx) {
            Ok(preset) => preset,
            Err(problem) => {
                self.error = Some(problem);
                cx.emit(TimerFormEvent::Failed);
                cx.notify();
                return;
            }
        };
        self.saving = true;
        self.error = None;
        let name = preset.name.clone();
        let path = self.path.clone();
        let replacing = self.replacing.clone();

        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    match replacing {
                        Some(old) => store::replace(&path, &old, preset),
                        None => store::add(&path, preset),
                    }
                })
                .await;
            this.update(cx, |form, cx| {
                form.saving = false;
                match saved {
                    Ok(()) => {
                        log::info!("saved timer preset {name:?}");
                        form.replacing = Some(name.clone());
                        form.edited = false;
                        cx.emit(TimerFormEvent::Saved(name));
                    }
                    Err(error) => {
                        log::warn!("could not save timer preset {name:?}: {error:#}");
                        form.error = Some(format!("{error:#}"));
                        cx.emit(TimerFormEvent::Failed);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        for field in [&self.name, &self.alias, &self.rounds] {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        for select in [&self.focus, &self.cycle, &self.repeat] {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        }
        for input in [&self.duration, &self.short_break, &self.long_break] {
            input.update(cx, |input, cx| input.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// The line under the fields.
    fn message(&self, window: &Window, cx: &App) -> (String, bool) {
        if let Some(error) = &self.error {
            return (error.clone(), true);
        }
        match self.focused(window, cx) {
            Some((_, row)) => (row.hint().to_string(), false),
            None => ("Enter saves. Esc leaves without saving.".to_string(), false),
        }
    }
}

impl Render for TimerForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        let (message, is_error) = self.message(window, cx);
        let rows: Vec<AnyElement> = self
            .rows()
            .into_iter()
            .map(|row| {
                let control = match self.control(row) {
                    Control::Text(entity) => entity.clone().into_any_element(),
                    Control::Select(entity) => entity.clone().into_any_element(),
                    Control::Duration(entity) => entity.clone().into_any_element(),
                };
                div()
                    .h(px(layout::form::ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w(px(layout::form::LABEL_WIDTH))
                            .flex_none()
                            .text_size(type_scale::subtitle())
                            .text_color(color(colors.text_muted))
                            .child(SharedString::from(row.label())),
                    )
                    .child(div().flex_1().child(control))
                    .into_any_element()
            })
            .collect();

        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .h(px(self.height()))
            .w_full()
            .px(metrics::gutter())
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
            )
    }
}
