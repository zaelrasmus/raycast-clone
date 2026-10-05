//! The Window Management page of the settings window (F17, 7.58).
//!
//! The gaps Lumen leaves around the windows it arranges, and what pressing a
//! half again does: the `window-management` section of `config.json`. Each
//! choice is written as it is made, as Appearance writes its own; the
//! daemon's live reload hands it to the platform. The hotkeys for the window
//! commands are the Shortcuts page's, and a row here goes there.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, EventEmitter, SharedString, Window, div};
use lumen_core::config::{GAP, WindowManagementConfig};
use lumen_core::theme::Colors;
use lumen_platform::Cycling;
use serde_json::Value;

use crate::quicklink_form::{NextField, PreviousField};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::settings_ui as ui;
use crate::translate_page::FORM_CONTEXT;

/// The gaps offered, in logical pixels. A file may say any other within
/// [`GAP`]; it is shown as its number.
const GAPS: [i32; 6] = [0, 4, 8, 12, 16, 24];

/// The choices of the cycling dropdown: its value, as `config.json` writes
/// it, and its label.
const CYCLING: [(Cycling, &str, &str); 3] = [
    (Cycling::Off, "off", "Off"),
    (
        Cycling::Sizes,
        "sizes",
        "Cycle \u{bd}, \u{2154} and \u{2153}",
    ),
    (Cycling::Displays, "displays", "Move to adjacent display"),
];

/// What the page asks the settings window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowManagementEvent {
    /// Show the Shortcuts page, searching for the window commands.
    OpenShortcuts,
}

/// Which of the two gaps.
#[derive(Debug, Clone, Copy)]
enum Gap {
    Window,
    Screen,
}

pub struct WindowManagementPage {
    path: PathBuf,
    colors: Colors,
    /// Why the file could not be read. Nothing can be saved until it is fixed.
    problem: Option<String>,
    gap: Entity<Select>,
    screen_gap: Entity<Select>,
    cycling: Entity<Select>,
    error: Option<String>,
    status: Option<SharedString>,
}

impl EventEmitter<WindowManagementEvent> for WindowManagementPage {}

impl WindowManagementPage {
    pub fn new(path: PathBuf, colors: Colors, cx: &mut Context<Self>) -> Self {
        let config = WindowManagementConfig::default();
        let gap = gap_select(config.gap, &colors, cx);
        let screen_gap = gap_select(config.screen_gap, &colors, cx);
        let cycling = cycling_select(config.cycling, &colors, cx);
        let page = Self {
            path,
            colors,
            problem: None,
            gap,
            screen_gap,
            cycling,
            error: None,
            status: None,
        };
        page.watch_gap(Gap::Window, cx);
        page.watch_gap(Gap::Screen, cx);
        page.watch_cycling(cx);
        page
    }

    /// Re-read the file, off the UI thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { WindowManagementConfig::read(&path) })
                .await;
            this.update(cx, |page, cx| {
                match read {
                    Ok(config) => {
                        page.problem = None;
                        page.rebuild(config, cx);
                    }
                    Err(error) => {
                        log::warn!("the window management page cannot read the file: {error:#}");
                        page.problem = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Build a dropdown again only when it shows something other than the
    /// file: built again, it is a new element, and the focus a keyboard
    /// choice left on it would be lost.
    fn rebuild(&mut self, config: WindowManagementConfig, cx: &mut Context<Self>) {
        if shown_gap(&self.gap, cx) != Some(clamped(config.gap)) {
            self.gap = gap_select(config.gap, &self.colors, cx);
            self.watch_gap(Gap::Window, cx);
        }
        if shown_gap(&self.screen_gap, cx) != Some(clamped(config.screen_gap)) {
            self.screen_gap = gap_select(config.screen_gap, &self.colors, cx);
            self.watch_gap(Gap::Screen, cx);
        }
        if chosen_cycling(self.cycling.read(cx).value()) != config.cycling {
            self.cycling = cycling_select(config.cycling, &self.colors, cx);
            self.watch_cycling(cx);
        }
    }

    fn watch_gap(&self, gap: Gap, cx: &mut Context<Self>) {
        let (select, key) = match gap {
            Gap::Window => (&self.gap, "gap"),
            Gap::Screen => (&self.screen_gap, "screen-gap"),
        };
        cx.subscribe(select, move |page, select, SelectEvent::Changed, cx| {
            if let Some(gap) = chosen_gap(select.read(cx).value()) {
                let done = match gap {
                    0 => "No gap".to_string(),
                    gap => format!("Gap of {gap} px"),
                };
                page.write(key, Value::from(gap), done, cx);
            }
        })
        .detach();
    }

    fn watch_cycling(&self, cx: &mut Context<Self>) {
        cx.subscribe(&self.cycling, |page, select, SelectEvent::Changed, cx| {
            let cycling = chosen_cycling(select.read(cx).value());
            if let Ok(value) = serde_json::to_value(cycling) {
                page.write("cycling", value, "Cycling saved".into(), cx);
            }
        })
        .detach();
    }

    fn write(&mut self, key: &'static str, value: Value, done: String, cx: &mut Context<Self>) {
        if self.problem.is_some() {
            return;
        }
        let path = self.path.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { WindowManagementConfig::set(&path, key, value) })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(()) => page.status = Some(done.into()),
                    Err(error) => {
                        log::warn!("could not save window management: {error:#}");
                        page.error = Some(format!("{error:#}"));
                    }
                }
                page.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(self.gap.read(cx).focus_handle());
    }

    /// The three dropdowns, in order.
    fn step(&mut self, by: usize, window: &mut Window, cx: &mut Context<Self>) {
        let selects = [&self.gap, &self.screen_gap, &self.cycling];
        let at = selects
            .iter()
            .position(|select| select.read(cx).focus_handle().is_focused(window))
            .unwrap_or(0);
        let next = selects[(at + by) % selects.len()].read(cx).focus_handle();
        window.focus(next);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.step(2, window, cx);
    }

    // -- What the host asks ---------------------------------------------------

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        for select in [&self.gap, &self.screen_gap, &self.cycling] {
            select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    pub fn status(&self) -> Option<SharedString> {
        self.status.clone()
    }
}

/// The gap dropdown's choices and which is `current`, clamped as it is
/// applied. A value not among [`GAPS`] is added in its place, as its number,
/// so the file is shown as it is.
pub fn gap_choices(current: i32) -> (Vec<Choice>, usize) {
    let current = clamped(current);
    let mut gaps = GAPS.to_vec();
    if !gaps.contains(&current) {
        gaps.push(current);
        gaps.sort_unstable();
    }
    let selected = gaps.iter().position(|gap| *gap == current).unwrap_or(0);
    let choices = gaps
        .into_iter()
        .map(|gap| Choice {
            value: Some(gap.to_string()),
            label: match gap {
                0 => "None".into(),
                gap => format!("{gap} px"),
            },
        })
        .collect();
    (choices, selected)
}

/// A gap as it is applied.
fn clamped(gap: i32) -> i32 {
    gap.clamp(*GAP.start(), *GAP.end())
}

fn shown_gap(select: &Entity<Select>, cx: &App) -> Option<i32> {
    chosen_gap(select.read(cx).value())
}

/// The gap a dropdown's value stands for.
pub fn chosen_gap(value: Option<&str>) -> Option<i32> {
    value?.parse().ok()
}

/// Which of [`CYCLING`] is `current`.
pub fn cycling_position(current: Cycling) -> usize {
    CYCLING
        .iter()
        .position(|(cycling, _, _)| *cycling == current)
        .unwrap_or(0)
}

/// The cycling a dropdown's value stands for.
pub fn chosen_cycling(value: Option<&str>) -> Cycling {
    CYCLING
        .iter()
        .find(|(_, written, _)| Some(*written) == value)
        .map_or_else(Cycling::default, |(cycling, _, _)| *cycling)
}

fn gap_select(current: i32, colors: &Colors, cx: &mut App) -> Entity<Select> {
    let (choices, selected) = gap_choices(current);
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact())
}

fn cycling_select(current: Cycling, colors: &Colors, cx: &mut App) -> Entity<Select> {
    let choices = CYCLING
        .iter()
        .map(|(_, value, label)| Choice {
            value: Some((*value).into()),
            label: (*label).into(),
        })
        .collect();
    let selected = cycling_position(current);
    cx.new(|cx| Select::new(choices, selected, colors.clone(), cx).compact())
}

impl Render for WindowManagementPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            self.focus(window, cx);
        }
        let colors = self.colors.clone();
        div()
            .size_full()
            .flex()
            .overflow_hidden()
            .child(self.render_form(&colors, cx))
    }
}

impl WindowManagementPage {
    fn render_form(&self, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        if let Some(problem) = &self.problem {
            return ui::problem("window-management", problem, colors);
        }

        let hotkeys = text_button(
            "window-management-hotkeys",
            "Open Shortcuts",
            colors,
            cx.listener(|_, _, _, cx| cx.emit(WindowManagementEvent::OpenShortcuts)),
        )
        .into_any_element();

        let (message, is_error) = match &self.error {
            Some(error) => (error.clone(), true),
            None => (
                "Each choice applies at once, to the next window arranged.".to_string(),
                false,
            ),
        };

        ui::page("window-management")
            .key_context(FORM_CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .child(ui::section_title("Gaps", colors))
            .child(ui::card(
                [
                    ui::row(
                        "Window Gap",
                        Some("Between two windows arranged side by side".into()),
                        self.gap.clone(),
                        colors,
                    ),
                    ui::row(
                        "Screen Gap",
                        Some("Between a window and the edge of the screen".into()),
                        self.screen_gap.clone(),
                        colors,
                    ),
                ],
                colors,
            ))
            .child(ui::section_title("Halves", colors))
            .child(ui::card(
                [
                    ui::row(
                        "Cycling",
                        Some("What pressing a half again does".into()),
                        self.cycling.clone(),
                        colors,
                    ),
                    ui::row(
                        "Set hotkeys for window commands",
                        Some("On the Shortcuts page, searching for window".into()),
                        hotkeys,
                        colors,
                    ),
                ],
                colors,
            ))
            .child(ui::note(message, is_error, colors))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|choice| choice.label.as_str()).collect()
    }

    #[test]
    fn a_listed_gap_is_chosen_where_it_is() {
        let (choices, selected) = gap_choices(8);
        assert_eq!(
            labels(&choices),
            ["None", "4 px", "8 px", "12 px", "16 px", "24 px"]
        );
        assert_eq!(selected, 2);
        assert_eq!(chosen_gap(choices[selected].value.as_deref()), Some(8));
        assert_eq!(chosen_gap(choices[0].value.as_deref()), Some(0));
    }

    /// The file is the user's: a gap the list does not offer is shown as
    /// its number, in order, rather than as the nearest choice.
    #[test]
    fn an_odd_file_gap_is_shown_as_its_number() {
        let (choices, selected) = gap_choices(10);
        assert_eq!(
            labels(&choices),
            ["None", "4 px", "8 px", "10 px", "12 px", "16 px", "24 px"]
        );
        assert_eq!(selected, 3);
        assert_eq!(chosen_gap(choices[selected].value.as_deref()), Some(10));
    }

    /// Out of range is shown as what applies.
    #[test]
    fn a_gap_out_of_range_is_shown_clamped() {
        let (choices, selected) = gap_choices(500);
        assert_eq!(choices[selected].label, "64 px");
        let (choices, selected) = gap_choices(-3);
        assert_eq!(choices[selected].label, "None");
        assert_eq!(choices.len(), GAPS.len());
    }

    #[test]
    fn cycling_maps_to_its_choice_and_back() {
        for (cycling, value, _) in CYCLING {
            assert_eq!(CYCLING[cycling_position(cycling)].1, value);
            assert_eq!(chosen_cycling(Some(value)), cycling);
            // What the page writes is what the config reads.
            assert_eq!(serde_json::to_value(cycling).unwrap(), Value::from(value));
        }
        assert_eq!(chosen_cycling(None), Cycling::Sizes);
    }
}
