//! The Weather page of the settings window (F13, slice 7).
//!
//! Units, how a report is shown, and the city the view leads with. The
//! World Clock page's shape: a form over one section of `config.json`,
//! read again after every save and whenever the file changes, written
//! through `lumen_core::weather::store`.
//!
//! The home city is `places.home`, shared with World Clock and edited from
//! either page (`home_choice`).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FocusHandle, SharedString, Window, div, px};
use lumen_core::layout::{self, settings as metrics};
use lumen_core::places::{Favourites, store as places_store};
use lumen_core::theme::Colors;
use lumen_core::weather::{Layout, Units, WeatherConfig, store};

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::select::{Choice, Select, SelectEvent};
use crate::styling::{color, type_scale};
use crate::translate_page::FORM_CONTEXT;

const UNITS: [(Units, &str, &str); 2] = [
    (Units::Metric, "metric", "Metric: \u{b0}C, km/h, mm"),
    (Units::Imperial, "imperial", "Imperial: \u{b0}F, mph, in"),
];

const LAYOUTS: [(Layout, &str, &str); 2] = [
    (
        Layout::Report,
        "report",
        "Report: Enter opens it at full width",
    ),
    (Layout::Pane, "pane", "Pane: beside the list, as you move"),
];

struct Form {
    units: Entity<Select>,
    layout: Entity<Select>,
    home: Entity<Select>,
}

pub struct WeatherPage {
    config_file: PathBuf,
    places_file: PathBuf,
    colors: Colors,
    config: WeatherConfig,
    /// The shared home city, as the file has it.
    home: Option<String>,
    /// *My Places*, as kept, for the home choices.
    mine: Vec<String>,
    problem: Option<String>,
    form: Form,
    edited: bool,
    saving: bool,
    error: Option<String>,
    status: Option<SharedString>,
    refocus: bool,
}

impl WeatherPage {
    pub fn new(
        config_file: PathBuf,
        places_file: PathBuf,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = WeatherConfig::default();
        let form = form(&config, None, &[], &colors, cx);
        let mut page = Self {
            config_file,
            places_file,
            colors,
            config,
            home: None,
            mine: Vec::new(),
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

    /// Re-read the file and *My Places*, off the UI thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let config_file = self.config_file.clone();
        let places_file = self.places_file.clone();
        cx.spawn(async move |this, cx| {
            let (read, mine) = cx
                .background_executor()
                .spawn(async move {
                    let read = store::read(&config_file)
                        .and_then(|config| Ok((config, places_store::read_home(&config_file)?)));
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
                        log::warn!("the weather page cannot read the file: {error:#}");
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
        for select in [&self.form.units, &self.form.layout, &self.form.home] {
            cx.subscribe(select, |page, _, SelectEvent::Changed, cx| {
                page.edited = true;
                page.error = None;
                cx.notify();
            })
            .detach();
        }
    }

    fn composed(&self, cx: &App) -> WeatherConfig {
        let chosen = |select: &Entity<Select>| select.read(cx).value().map(str::to_string);
        let units = chosen(&self.form.units);
        let layout = chosen(&self.form.layout);
        WeatherConfig {
            units: UNITS
                .iter()
                .find(|(_, value, _)| Some(*value) == units.as_deref())
                .map_or_else(Units::default, |(units, _, _)| *units),
            layout: LAYOUTS
                .iter()
                .find(|(_, value, _)| Some(*value) == layout.as_deref())
                .map_or_else(Layout::default, |(layout, _, _)| *layout),
            home: None,
        }
    }

    fn chosen_home(&self, cx: &App) -> Option<String> {
        crate::home_choice::chosen(self.form.home.read(cx).value())
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        if let Some(first) = self.stops(cx).first() {
            window.focus(first);
        }
    }

    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![
            self.form.units.read(cx).focus_handle().clone(),
            self.form.layout.read(cx).focus_handle().clone(),
            self.form.home.read(cx).focus_handle().clone(),
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

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.problem.is_some() {
            return;
        }
        let path = self.config_file.clone();
        let config = self.composed(cx);
        let home = self.chosen_home(cx);
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
                        log::info!("saved the weather settings");
                        page.status = Some("Saved. The next Weather opened uses it".into());
                        page.edited = false;
                        page.reload(cx);
                    }
                    Err(error) => {
                        log::warn!("could not save the weather settings: {error:#}");
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
        for select in [&self.form.units, &self.form.layout, &self.form.home] {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
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
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx))
}

fn form(
    config: &WeatherConfig,
    home: Option<&str>,
    mine: &[String],
    colors: &Colors,
    cx: &mut App,
) -> Form {
    let (choices, selected) = crate::home_choice::choices(home, mine);
    Form {
        units: select(&UNITS, config.units, colors, cx),
        layout: select(&LAYOUTS, config.layout, colors, cx),
        home: cx.new(|cx| Select::new(choices, selected, colors.clone(), cx)),
    }
}

impl Render for WeatherPage {
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

impl WeatherPage {
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

        let field_row = |text: &'static str, control: AnyElement| {
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
                        .child(text),
                )
                .child(div().flex_1().child(control))
                .into_any_element()
        };
        let rows = vec![
            field_row("Units", self.form.units.clone().into_any_element()),
            field_row("Report", self.form.layout.clone().into_any_element()),
            field_row("Home", self.form.home.clone().into_any_element()),
        ];
        let hints = [
            "How temperatures, wind and rain are written. Forecasts are kept in metric.",
            "Where a city's full report is shown.",
            crate::home_choice::HINT,
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
