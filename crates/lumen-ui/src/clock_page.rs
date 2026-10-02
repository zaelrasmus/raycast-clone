//! The World Clock page of the settings window (F13, slice 4).
//!
//! The home city (`places.home`, shared with Weather), which value leads a
//! row, how the difference is written, and the working hours compared with
//! each place's. The Currency page's shape: a form over
//! one section of `config.json`, read again after every save and whenever
//! the file changes, written through `lumen_core::clock::store`.
//!
//! # The overlap belongs to *Difference first*
//!
//! The overlap is a column of the difference-first row, so with *Time
//! first* chosen its fields are not offered: they say why instead, and Tab
//! passes them by. The choice is read from the form, not the file, so the
//! page answers as the dropdown changes.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px,
};
use lumen_core::clock::{ClockConfig, DifferenceFormat, Overlap, RowLayout, store};
use lumen_core::layout;
use lumen_core::places::{Favourites, store as places_store};
use lumen_core::theme::Colors;

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings_ui as ui;
use crate::styling::{color, type_scale};
use crate::text_field::{TextField, TextFieldEvent};
use crate::translate_page::FORM_CONTEXT;

const ROWS: [(RowLayout, &str, &str); 2] = [
    (
        RowLayout::Difference,
        "difference",
        "Difference first: 09:14  2 h behind",
    ),
    (RowLayout::Time, "time", "Time first: 2 h behind  09:14"),
];

const FORMATS: [(DifferenceFormat, &str, &str); 2] = [
    (DifferenceFormat::Words, "words", "In words: 2 h behind"),
    (DifferenceFormat::Signed, "signed", "Signed: \u{2212}2 h"),
];

const OVERLAP: [(bool, &str, &str); 2] = [
    (false, "off", "Off"),
    (true, "on", "Show when you are both at work"),
];

/// The form's fields, rebuilt from the file.
struct Form {
    home: Entity<Select>,
    row: Entity<Select>,
    format: Entity<Select>,
    overlap: Entity<Select>,
    start: Entity<TextField>,
    end: Entity<TextField>,
}

pub struct ClockPage {
    path: PathBuf,
    places_file: PathBuf,
    /// The shared home city, as the file has it, and *My Places* to choose
    /// it from.
    home: Option<String>,
    mine: Vec<String>,
    colors: Colors,
    /// The section as the file last read had it.
    config: ClockConfig,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    form: Form,
    /// Whether the form has changed since it was filled in or saved.
    edited: bool,
    saving: bool,
    /// Why the last save was refused, under the form.
    error: Option<String>,
    status: Option<SharedString>,
    /// Put the caret back in the form at the next frame.
    refocus: bool,
}

impl ClockPage {
    pub fn new(
        path: PathBuf,
        places_file: PathBuf,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = ClockConfig::default();
        let form = form(&config, None, &[], &colors, cx);
        let mut page = Self {
            path,
            places_file,
            home: None,
            mine: Vec::new(),
            colors,
            config,
            problem: None,
            form,
            edited: false,
            saving: false,
            error: None,
            status: None,
            refocus: false,
        };
        page.watch_form(cx);
        page
    }

    /// Re-read the file, off the UI thread. An untouched form follows it; a
    /// touched one is left alone.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let places_file = self.places_file.clone();
        cx.spawn(async move |this, cx| {
            let (read, mine) = cx
                .background_executor()
                .spawn(async move {
                    let read = store::read(&path)
                        .and_then(|config| Ok((config, places_store::read_home(&path)?)));
                    (read, Favourites::load(&places_file).cities)
                })
                .await;
            this.update(cx, |page, cx| {
                page.mine = mine;
                match read {
                    Ok((config, home)) => {
                        page.config = config;
                        page.home = home;
                        page.problem = None;
                    }
                    Err(error) => {
                        log::warn!("the world clock page cannot read the file: {error:#}");
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
        self.form = form(
            &self.config,
            self.home.as_deref(),
            &self.mine,
            &self.colors,
            cx,
        );
        self.watch_form(cx);
        self.edited = false;
        self.error = None;
    }

    fn watch_form(&mut self, cx: &mut Context<Self>) {
        for field in [&self.form.start, &self.form.end] {
            cx.subscribe(field, |page, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    page.touched(cx);
                }
            })
            .detach();
        }
        for select in [
            &self.form.home,
            &self.form.row,
            &self.form.format,
            &self.form.overlap,
        ] {
            cx.subscribe(select, |page, _, SelectEvent::Changed, cx| page.touched(cx))
                .detach();
        }
    }

    fn touched(&mut self, cx: &mut Context<Self>) {
        self.edited = true;
        self.error = None;
        cx.notify();
    }

    /// The form as a section, whatever it holds: the store judges the hours.
    fn composed(&self, cx: &App) -> ClockConfig {
        let chosen = |select: &Entity<Select>| select.read(cx).value().map(str::to_string);
        let row = chosen(&self.form.row);
        let format = chosen(&self.form.format);
        let overlap = chosen(&self.form.overlap);
        ClockConfig {
            row: ROWS
                .iter()
                .find(|(_, value, _)| Some(*value) == row.as_deref())
                .map_or_else(RowLayout::default, |(row, _, _)| *row),
            difference_format: FORMATS
                .iter()
                .find(|(_, value, _)| Some(*value) == format.as_deref())
                .map_or_else(DifferenceFormat::default, |(format, _, _)| *format),
            overlap: Overlap {
                enabled: OVERLAP
                    .iter()
                    .find(|(_, value, _)| Some(*value) == overlap.as_deref())
                    .is_some_and(|(enabled, _, _)| *enabled),
                start: self.form.start.read(cx).text().trim().to_string(),
                end: self.form.end.read(cx).text().trim().to_string(),
            },
        }
    }

    /// Whether the overlap is offered: only with *Difference first*, as
    /// chosen in the form.
    fn offers_overlap(&self, cx: &App) -> bool {
        self.composed(cx).row == RowLayout::Difference
    }

    /// Whether the working hours are asked for: the overlap is offered and on.
    fn asks_hours(&self, cx: &App) -> bool {
        let config = self.composed(cx);
        config.row == RowLayout::Difference && config.overlap.enabled
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
    }

    /// The fields Tab visits: those offered.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![
            self.form.home.read(cx).focus_handle().clone(),
            self.form.row.read(cx).focus_handle().clone(),
            self.form.format.read(cx).focus_handle().clone(),
        ];
        if self.offers_overlap(cx) {
            stops.push(self.form.overlap.read(cx).focus_handle().clone());
        }
        if self.asks_hours(cx) {
            stops.push(self.form.start.focus_handle(cx));
            stops.push(self.form.end.focus_handle(cx));
        }
        stops
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
        let home = crate::home_choice::chosen(self.form.home.read(cx).value());
        let mut config = self.composed(cx);
        // Hours not asked for are kept as they were, not judged: a typo in
        // a field that is not shown must not stop the rest being saved.
        if !self.asks_hours(cx) {
            config.overlap.start.clone_from(&self.config.overlap.start);
            config.overlap.end.clone_from(&self.config.overlap.end);
        }
        self.saving = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    store::write(&path, &config)?;
                    places_store::write_home(&path, home.as_deref())
                })
                .await;
            this.update(cx, |page, cx| {
                page.saving = false;
                match result {
                    Ok(()) => {
                        log::info!("saved the world clock settings");
                        page.status = Some("Saved. The next World Clock opened uses it".into());
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the world clock settings: {error:#}");
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
        for select in [
            &self.form.home,
            &self.form.row,
            &self.form.format,
            &self.form.overlap,
        ] {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        }
        for field in [&self.form.start, &self.form.end] {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    pub fn status(&self) -> Option<SharedString> {
        if self.edited {
            return Some("Unsaved changes  Enter to save, Esc to undo".into());
        }
        self.status.clone()
    }
}

fn select<T: PartialEq + Copy>(
    options: &[(T, &str, &str)],
    current: T,
    colors: &Colors,
    cx: &mut App,
) -> Entity<Select> {
    let choices = options
        .iter()
        .map(|(_, value, label)| Choice {
            value: Some((*value).into()),
            label: (*label).into(),
        })
        .collect();
    let selected = options
        .iter()
        .position(|(option, _, _)| *option == current)
        .unwrap_or(0);
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact())
}

fn form(
    config: &ClockConfig,
    home: Option<&str>,
    mine: &[String],
    colors: &Colors,
    cx: &mut App,
) -> Form {
    let (choices, selected) = crate::home_choice::choices(home, mine);
    Form {
        home: cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact()),
        row: select(&ROWS, config.row, colors, cx),
        format: select(&FORMATS, config.difference_format, colors, cx),
        overlap: select(&OVERLAP, config.overlap.enabled, colors, cx),
        start: cx
            .new(|cx| TextField::new(config.overlap.start.clone(), "09:00", colors.clone(), cx)),
        end: cx.new(|cx| TextField::new(config.overlap.end.clone(), "17:00", colors.clone(), cx)),
    }
}

impl Render for ClockPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

impl ClockPage {
    fn render_form(&self, colors: &Colors, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(problem) = &self.problem {
            return ui::problem("clock", problem, colors);
        }

        let mut rows = vec![
            ui::row(
                "Home",
                Some("The city your time is compared from".into()),
                self.form.home.clone(),
                colors,
            ),
            ui::row(
                "Row",
                Some("Which value leads a row: how far away, or what time".into()),
                self.form.row.clone(),
                colors,
            ),
            ui::row(
                "Difference",
                Some("How the difference from your time is written".into()),
                self.form.format.clone(),
                colors,
            ),
        ];
        // The note under the cards follows the focused field, in this order.
        let mut hints = vec![
            crate::home_choice::HINT,
            "Which value leads a row, drawn like its title: how far away, or what time.",
            "How the difference from your time is written.",
        ];
        if self.offers_overlap(cx) {
            rows.push(ui::row(
                "Overlap",
                Some("When your working day and theirs meet".into()),
                self.form.overlap.clone(),
                colors,
            ));
            hints.push("Shows, on each row, when your working day and theirs meet.");
        } else {
            // Not offered, saying why in its place.
            rows.push(ui::row(
                "Overlap",
                Some("Only with the difference leading a row".into()),
                gpui::Empty,
                colors,
            ));
        }
        if self.asks_hours(cx) {
            rows.push(ui::row(
                "Working hours",
                Some("Yours, 24-hour; the same hours are assumed there".into()),
                div()
                    .flex()
                    .items_center()
                    .gap(px(layout::form::DURATION_GAP))
                    .child(ui::narrow(self.form.start.clone()))
                    .child(
                        div()
                            .text_size(type_scale::subtitle())
                            .text_color(color(colors.text_muted))
                            .child("to"),
                    )
                    .child(ui::narrow(self.form.end.clone())),
                colors,
            ));
            hints.push("When your day starts, 24-hour: the same hours are assumed there.");
            hints.push("When your day ends, 24-hour.");
        }

        let (message, is_error) = match (&self.error, self.focused(window, cx)) {
            (Some(error), _) => (error.clone(), true),
            (None, Some(index)) => (hints.get(index).copied().unwrap_or("").to_string(), false),
            (None, None) => ("Enter or Ctrl+S saves. Esc undoes.".to_string(), false),
        };

        ui::page("clock")
            .key_context(FORM_CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .child(ui::section_title("World Clock", colors))
            .child(ui::card(rows, colors))
            .child(ui::note(message, is_error, colors))
            .into_any_element()
    }
}
