//! The Window Management page of the settings window (F17, 7.58).
//!
//! The gaps Lumen leaves around the windows it arranges, and what pressing a
//! half again does: the `window-management` section of `config.json`. Each
//! choice is written as it is made, as Appearance writes its own; the
//! daemon's live reload hands it to the platform. The hotkeys for the window
//! commands are the Shortcuts page's, and a row here goes there.
//!
//! Below them, the user's own window commands (7.59): `window-commands.json`,
//! listed, and created or edited in [`WindowCommandEditor`], which takes the
//! page's place while it is open. Every change goes through
//! [`window_commands::edit`], which reads the file afresh; the daemon
//! watches it and indexes the commands again.
//!
//! Then the user's window layouts (7.60), `window-layouts.json`, the same
//! way, in [`WindowLayoutEditor`]. *Save Current Layout* reads the windows on
//! screen and opens the editor on them, unsaved, as Raycast does.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, SharedString, Subscription,
    Window, div, px,
};
use lumen_core::Candidate;
use lumen_core::config::{GAP, WindowManagementConfig};
use lumen_core::layout::settings as metrics;
use lumen_core::providers::window_commands::{self, CustomWindowCommand};
use lumen_core::providers::window_layouts::{self, CustomWindowLayout};
use lumen_core::theme::Colors;
use lumen_platform::{Cycling, LayoutEntry, MonitorInfo, Platform, WindowSettings};
use serde_json::Value;

use crate::quicklink_form::{NextField, PreviousField};
use crate::quicklinks_page::{CancelDialog, ConfirmDialog, DIALOG_CONTEXT};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::settings_ui as ui;
use crate::styling::color;
use crate::translate_page::FORM_CONTEXT;
use crate::window_command_editor::{EditorEvent, Preview, WindowCommandEditor, summary};
use crate::window_layout_editor::{Draft, LayoutEditorEvent, WindowLayoutEditor};

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

/// What the delete dialog would delete.
#[derive(Debug, Clone)]
enum Doomed {
    Command(CustomWindowCommand),
    Layout(CustomWindowLayout),
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
    /// `window-commands.json`, and the commands as it last read.
    commands_path: PathBuf,
    commands: Vec<CustomWindowCommand>,
    /// For the display the editor previews.
    platform: Arc<dyn Platform>,
    monitor: Option<MonitorInfo>,
    editor: Option<(Entity<WindowCommandEditor>, Subscription)>,
    /// `window-layouts.json`, the layouts as it last read, and the
    /// launcher's catalogue, whose applications a layout picks from.
    layouts_path: PathBuf,
    layouts: Vec<CustomWindowLayout>,
    catalogue: Arc<Vec<Candidate>>,
    layout_editor: Option<(Entity<WindowLayoutEditor>, Subscription)>,
    /// The command or layout waiting on the delete dialog.
    confirming: Option<Doomed>,
    dialog_focus: FocusHandle,
    refocus: bool,
}

impl EventEmitter<WindowManagementEvent> for WindowManagementPage {}

impl WindowManagementPage {
    pub fn new(
        path: PathBuf,
        commands_path: PathBuf,
        layouts_path: PathBuf,
        platform: Arc<dyn Platform>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
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
            commands_path,
            commands: Vec::new(),
            platform,
            monitor: None,
            editor: None,
            layouts_path,
            layouts: Vec::new(),
            catalogue: Arc::default(),
            layout_editor: None,
            confirming: None,
            dialog_focus: cx.focus_handle(),
            refocus: false,
        };
        page.watch_gap(Gap::Window, cx);
        page.watch_gap(Gap::Screen, cx);
        page.watch_cycling(cx);
        page
    }

    /// Re-read the files, and which display the window is on, off the UI
    /// thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        let commands_path = self.commands_path.clone();
        let layouts_path = self.layouts_path.clone();
        let platform = self.platform.clone();
        cx.spawn(async move |this, cx| {
            let (read, commands, layouts, monitor) = cx
                .background_executor()
                .spawn(async move {
                    (
                        WindowManagementConfig::read(&path),
                        window_commands::load(&commands_path),
                        window_layouts::load(&layouts_path),
                        platform.cursor_monitor().ok(),
                    )
                })
                .await;
            this.update(cx, |page, cx| {
                page.commands = commands;
                page.layouts = layouts;
                page.monitor = monitor.or(page.monitor);
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

    // -- Custom commands ------------------------------------------------------

    /// Open the editor on `command`, or on a new one.
    fn open_editor(
        &mut self,
        command: Option<CustomWindowCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preview = self.preview(cx);
        let colors = self.colors.clone();
        let editor = cx.new(|cx| WindowCommandEditor::new(command, preview, colors, cx));
        let events = cx.subscribe(&editor, |page, _, event: &EditorEvent, cx| {
            match event.clone() {
                EditorEvent::Save {
                    id,
                    name,
                    placement,
                } => {
                    let done = format!("Saved {name}");
                    page.change(done, cx, move |commands| match &id {
                        Some(id) => window_commands::update(commands, id, &name, placement),
                        None => {
                            window_commands::add(commands, &name, placement);
                            true
                        }
                    });
                }
                EditorEvent::Cancelled => page.status = None,
            }
            page.editor = None;
            page.refocus = true;
            cx.notify();
        });
        editor.read(cx).focus(window, cx);
        self.editor = Some((editor, events));
        self.error = None;
        cx.notify();
    }

    /// The display the editors draw on, and the screen gap there.
    fn preview(&self, cx: &App) -> Preview {
        let gap = shown_gap(&self.screen_gap, cx).unwrap_or_default();
        Preview {
            monitor: self.monitor,
            gaps: self.monitor.map_or_else(Default::default, |monitor| {
                WindowSettings {
                    screen_gap: gap.max(0) as u32,
                    ..WindowSettings::default()
                }
                .gaps(monitor.scale_factor)
            }),
        }
    }

    /// Change `window-commands.json`, off the UI thread, then read it again.
    fn change(
        &mut self,
        done: String,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Vec<CustomWindowCommand>) -> bool + Send + 'static,
    ) {
        let path = self.commands_path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { window_commands::edit(&path, change) })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(true) => page.status = Some(done.into()),
                    Ok(false) => page.error = Some("That command is no longer in the file".into()),
                    Err(error) => {
                        log::warn!("could not save window commands: {error:#}");
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

    fn duplicate(&mut self, id: String, cx: &mut Context<Self>) {
        self.change("Duplicated".into(), cx, move |commands| {
            window_commands::duplicate(commands, &id).is_some()
        });
    }

    // -- Custom layouts -------------------------------------------------------

    /// The launcher's index, whose applications a layout picks from: when
    /// the settings window opens.
    pub fn set_catalogue(&mut self, catalogue: Arc<Vec<Candidate>>) {
        self.catalogue = catalogue;
    }

    /// Open the layout editor on `draft`.
    fn open_layout_editor(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let (preview, catalogue) = (self.preview(cx), self.catalogue.clone());
        let colors = self.colors.clone();
        let editor = cx.new(|cx| WindowLayoutEditor::new(draft, catalogue, preview, colors, cx));
        let events = cx.subscribe(&editor, |page, _, event: &LayoutEditorEvent, cx| {
            match event.clone() {
                LayoutEditorEvent::Save { id, name, spec } => {
                    let done = format!("Saved {name}");
                    page.change_layouts(done, cx, move |layouts| match &id {
                        Some(id) => window_layouts::update(layouts, id, &name, spec),
                        None => {
                            window_layouts::add(layouts, &name, spec);
                            true
                        }
                    });
                }
                LayoutEditorEvent::Cancelled => page.status = None,
            }
            page.layout_editor = None;
            page.refocus = true;
            cx.notify();
        });
        editor.read(cx).focus(window, cx);
        self.layout_editor = Some((editor, events));
        self.error = None;
        cx.notify();
    }

    /// Open the editor on `entries`, the windows on screen, unsaved and
    /// unnamed: the button below, and the root command (7.60).
    pub fn edit_current(
        &mut self,
        entries: Vec<LayoutEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_layout_editor(Draft::current(entries), window, cx);
    }

    /// Read the windows on screen off the UI thread, then open the editor on
    /// them, unsaved.
    fn save_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let platform = self.platform.clone();
        self.status = Some("Reading the windows on screen\u{2026}".into());
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { platform.current_layout() })
                .await;
            this.update_in(cx, |page, window, cx| {
                page.status = None;
                match read {
                    Ok(entries) if entries.is_empty() => {
                        page.error = Some("There are no windows to save".into());
                    }
                    Ok(entries) => page.edit_current(entries, window, cx),
                    Err(error) => page.error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Change `window-layouts.json`, off the UI thread, then read it again.
    fn change_layouts(
        &mut self,
        done: String,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Vec<CustomWindowLayout>) -> bool + Send + 'static,
    ) {
        let path = self.layouts_path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { window_layouts::edit(&path, change) })
                .await;
            this.update(cx, |page, cx| {
                match result {
                    Ok(true) => page.status = Some(done.into()),
                    Ok(false) => page.error = Some("That layout is no longer in the file".into()),
                    Err(error) => {
                        log::warn!("could not save window layouts: {error:#}");
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

    fn duplicate_layout(&mut self, id: String, cx: &mut Context<Self>) {
        self.change_layouts("Duplicated".into(), cx, move |layouts| {
            window_layouts::duplicate(layouts, &id).is_some()
        });
    }

    fn ask_delete(&mut self, doomed: Doomed, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = Some(doomed);
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    fn cancel_dialog(&mut self, _: &CancelDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = None;
        self.focus(window, cx);
        cx.notify();
    }

    fn confirm_dialog(&mut self, _: &ConfirmDialog, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doomed) = self.confirming.take() else {
            return;
        };
        self.focus(window, cx);
        match doomed {
            Doomed::Command(command) => {
                let done = format!("Deleted {}", command.name);
                self.change(done, cx, move |commands| {
                    window_commands::remove(commands, &command.id)
                });
            }
            Doomed::Layout(layout) => {
                let done = format!("Deleted {}", layout.name);
                self.change_layouts(done, cx, move |layouts| {
                    window_layouts::remove(layouts, &layout.id)
                });
            }
        }
    }

    // -- Focus ----------------------------------------------------------------

    pub fn focus(&self, window: &mut Window, cx: &App) {
        match (&self.editor, &self.layout_editor) {
            (Some((editor, _)), _) => editor.read(cx).focus(window, cx),
            (None, Some((editor, _))) => editor.read(cx).focus(window, cx),
            (None, None) => window.focus(self.gap.read(cx).focus_handle()),
        }
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
        if let Some((editor, _)) = &self.editor {
            editor.update(cx, |editor, cx| editor.set_colors(colors.clone(), cx));
        }
        if let Some((editor, _)) = &self.layout_editor {
            editor.update(cx, |editor, cx| editor.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    pub fn status(&self) -> Option<SharedString> {
        if self.editor.is_some() || self.layout_editor.is_some() {
            return Some("Ctrl+Enter to save, Esc to go back".into());
        }
        self.status.clone()
    }

    /// The delete confirmation, over the whole window, drawn by the host.
    pub fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (name, body) = match self.confirming.as_ref()? {
            Doomed::Command(command) => (
                &command.name,
                "The command goes from the launcher, with its hotkey and alias.",
            ),
            Doomed::Layout(layout) => (
                &layout.name,
                "The layout goes from the launcher, with its hotkey and alias.",
            ),
        };
        let colors = &self.colors;
        let buttons = div()
            .pt(px(metrics::GAP))
            .flex()
            .justify_end()
            .gap(px(metrics::GAP))
            .child(text_button(
                "window-command-dialog-cancel",
                "Cancel  Esc",
                colors,
                cx.listener(|page, _, window, cx| page.cancel_dialog(&CancelDialog, window, cx)),
            ))
            .child(text_button(
                "window-command-dialog-confirm",
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
        if self.confirming.is_none()
            && (std::mem::take(&mut self.refocus) || window.focused(cx).is_none())
        {
            self.focus(window, cx);
        }
        let colors = self.colors.clone();
        let content = match (&self.editor, &self.layout_editor) {
            (Some((editor, _)), _) => editor.clone().into_any_element(),
            (None, Some((editor, _))) => editor.clone().into_any_element(),
            (None, None) => self.render_form(&colors, cx),
        };
        div().size_full().flex().overflow_hidden().child(content)
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
            .child(ui::section_title("Custom Commands", colors))
            .child(ui::card(self.command_rows(colors, cx), colors))
            .child(ui::section_title("Custom Layouts", colors))
            .child(ui::card(self.layout_rows(colors, cx), colors))
            .child(ui::note(message, is_error, colors))
            .into_any_element()
    }

    /// A row per command, with its buttons, then Create.
    fn command_rows(&self, colors: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.commands.len() + 1);
        for (index, command) in self.commands.iter().enumerate() {
            let edit = command.clone();
            let id = command.id.clone();
            let delete = command.clone();
            let buttons = div()
                .flex()
                .gap(px(metrics::GAP))
                .child(text_button(
                    ("window-command-edit", index),
                    "Edit",
                    colors,
                    cx.listener(move |page, _, window, cx| {
                        page.open_editor(Some(edit.clone()), window, cx)
                    }),
                ))
                .child(text_button(
                    ("window-command-duplicate", index),
                    "Duplicate",
                    colors,
                    cx.listener(move |page, _, _, cx| page.duplicate(id.clone(), cx)),
                ))
                .child(text_button(
                    ("window-command-delete", index),
                    "Delete\u{2026}",
                    colors,
                    cx.listener(move |page, _, window, cx| {
                        page.ask_delete(Doomed::Command(delete.clone()), window, cx)
                    }),
                ));
            rows.push(ui::row(
                command.name.clone(),
                Some(summary(&command.placement).into()),
                buttons,
                colors,
            ));
        }
        let create = text_button(
            "window-command-create",
            "Create Command",
            colors,
            cx.listener(|page, _, window, cx| page.open_editor(None, window, cx)),
        );
        rows.push(ui::row(
            "New window command",
            Some("A size, an anchor and an offset of your own".into()),
            create,
            colors,
        ));
        rows
    }

    /// A row per layout, with its buttons, then Create and Save Current.
    fn layout_rows(&self, colors: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.layouts.len() + 1);
        for (index, layout) in self.layouts.iter().enumerate() {
            let edit = layout.clone();
            let id = layout.id.clone();
            let delete = layout.clone();
            let buttons = div()
                .flex()
                .gap(px(metrics::GAP))
                .child(text_button(
                    ("window-layout-edit", index),
                    "Edit",
                    colors,
                    cx.listener(move |page, _, window, cx| {
                        page.open_layout_editor(edit.clone().into(), window, cx)
                    }),
                ))
                .child(text_button(
                    ("window-layout-duplicate", index),
                    "Duplicate",
                    colors,
                    cx.listener(move |page, _, _, cx| page.duplicate_layout(id.clone(), cx)),
                ))
                .child(text_button(
                    ("window-layout-delete", index),
                    "Delete\u{2026}",
                    colors,
                    cx.listener(move |page, _, window, cx| {
                        page.ask_delete(Doomed::Layout(delete.clone()), window, cx)
                    }),
                ));
            rows.push(ui::row(
                layout.name.clone(),
                Some(window_layouts::apps(layout.spec.entries.len()).into()),
                buttons,
                colors,
            ));
        }
        let buttons = div()
            .flex()
            .gap(px(metrics::GAP))
            .child(text_button(
                "window-layout-save-current",
                "Save Current Layout",
                colors,
                cx.listener(|page, _, window, cx| page.save_current(window, cx)),
            ))
            .child(text_button(
                "window-layout-create",
                "Create Layout",
                colors,
                cx.listener(|page, _, window, cx| {
                    page.open_layout_editor(Draft::new(), window, cx)
                }),
            ));
        rows.push(ui::row(
            "New window layout",
            Some("Several applications placed at once".into()),
            buttons,
            colors,
        ));
        rows
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
