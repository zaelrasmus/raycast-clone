//! The editor for a custom window layout (F17, 7.60), after Raycast's
//! *Create Window Layout*.
//!
//! The name, whether the screen gap is left round each window, and what
//! happens to the other windows; then the layout's applications, one row
//! each. Selecting a row shows its application, what it is started with, its
//! display and its placement, edited with the command editor's own fields;
//! the preview draws every window on that display, the selected one lit.
//!
//! Like the command editor it writes nothing: it tells the page what to
//! save, and the page writes `window-layouts.json`, which the daemon watches.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window, div, px,
};
use lumen_core::Candidate;
use lumen_core::item::{Action, Kind};
use lumen_core::layout::settings as metrics;
use lumen_core::providers::window_layouts::{self, CustomWindowLayout};
use lumen_core::theme::Colors;
use lumen_platform::layouts::MAX_PER_DISPLAY;
use lumen_platform::{
    Anchor, AppTarget, Argument, LaunchTarget, LayoutEntry, Length, Offset, OtherWindows,
    Placement, WindowLayoutSpec,
};

use crate::placement_fields::{
    PlacementFields, display_preview, either, form_buttons, line, place_preview,
};
use crate::quicklink_form::{
    CancelForm, NextField, PreviousField, SubmitForm, TOGGLE_CONTEXT, ToggleFallback,
};
use crate::select::{Choice, Select, SelectEvent};
use crate::settings::text_button;
use crate::settings_ui as ui;
use crate::styling::color;
use crate::text_field::{TextField, TextFieldEvent};
use crate::window_command_editor::{CONTEXT, Preview, summary};

/// The displays offered when the file names none further: the platform does
/// not list its monitors to the settings window, and an entry on a display
/// that is not attached goes to the primary one.
pub const DISPLAYS: u32 = 4;

/// What an entry can be started with: its value and its label.
pub const ARGUMENT_KINDS: [(&str, &str); 4] = [
    ("none", "Nothing"),
    ("url", "URL"),
    ("path", "File or folder"),
    ("args", "Arguments"),
];

/// What happens to the other windows: the choice, its value and its label.
pub const OTHER_WINDOWS: [(OtherWindows, &str, &str); 3] = [
    (OtherWindows::None, "none", "Leave them"),
    (OtherWindows::Minimize, "minimize", "Minimise others"),
    (OtherWindows::Close, "close", "Close others"),
];

/// A layout as the editor opens on it: a new one has no id.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub id: Option<String>,
    pub name: String,
    pub spec: WindowLayoutSpec,
}

impl Draft {
    /// A new layout: no applications yet, the gaps left, the rest alone.
    pub fn new() -> Self {
        Self {
            id: None,
            name: String::new(),
            spec: WindowLayoutSpec {
                entries: Vec::new(),
                gaps: true,
                other_windows: OtherWindows::None,
            },
        }
    }

    /// *Save Current Layout*: the windows on screen, unsaved and unnamed, so
    /// the user names it before it can be saved.
    pub fn current(entries: Vec<LayoutEntry>) -> Self {
        Self {
            id: None,
            name: String::new(),
            spec: window_layouts::snapshot(entries),
        }
    }
}

impl Default for Draft {
    fn default() -> Self {
        Self::new()
    }
}

impl From<CustomWindowLayout> for Draft {
    fn from(layout: CustomWindowLayout) -> Self {
        Self {
            id: Some(layout.id),
            name: layout.name,
            spec: layout.spec,
        }
    }
}

/// An argument as its two controls show it: the kind's value and the text.
pub fn argument_parts(argument: &Argument) -> (&'static str, String) {
    match argument {
        Argument::None => ("none", String::new()),
        Argument::Url(text) => ("url", text.clone()),
        Argument::Path(text) => ("path", text.clone()),
        Argument::Args(text) => ("args", text.clone()),
    }
}

/// The argument the two controls stand for.
pub fn argument_of(kind: Option<&str>, text: &str) -> Argument {
    let text = text.to_string();
    match kind {
        Some("url") => Argument::Url(text),
        Some("path") => Argument::Path(text),
        Some("args") => Argument::Args(text),
        _ => Argument::None,
    }
}

/// The other-windows choice a dropdown's value stands for.
pub fn other_windows_of(value: Option<&str>) -> OtherWindows {
    OTHER_WINDOWS
        .iter()
        .find(|(_, written, _)| Some(*written) == value)
        .map_or(OtherWindows::None, |(other, _, _)| *other)
}

/// The applications in the launcher's catalogue, by name, and which of them
/// `current` is. One the catalogue does not have -- an executable Save
/// Current Layout found -- is kept under its own name, as the file has it.
pub fn app_choices(catalogue: &[Candidate], current: &AppTarget) -> (Vec<Choice>, usize) {
    let mut apps: Vec<Choice> = catalogue
        .iter()
        .filter(|candidate| candidate.item.kind == Kind::Application)
        .filter_map(|candidate| {
            let path = match candidate.item.actions.first()? {
                Action::Launch(LaunchTarget::ShellExecute(path)) => path.clone(),
                Action::Launch(LaunchTarget::AppUserModelId(id)) => {
                    format!(r"shell:AppsFolder\{id}")
                }
                _ => return None,
            };
            Some(Choice {
                value: Some(path),
                label: candidate.item.title.clone(),
            })
        })
        .collect();
    apps.sort_by_key(|choice| choice.label.to_lowercase());
    let mut choices = vec![Choice {
        value: None,
        label: "Choose an application".into(),
    }];
    choices.extend(apps);
    if current.path.trim().is_empty() {
        return (choices, 0);
    }
    let found = choices.iter().position(|choice| {
        choice
            .value
            .as_deref()
            .is_some_and(|path| path.eq_ignore_ascii_case(&current.path))
    });
    match found {
        Some(index) => (choices, index),
        None => {
            choices.push(Choice {
                value: Some(current.path.clone()),
                label: current.name.clone(),
            });
            let index = choices.len() - 1;
            (choices, index)
        }
    }
}

/// Display 1 to [`DISPLAYS`], or further when the file says so; the value is
/// the platform's index.
pub fn display_choices(current: u32) -> (Vec<Choice>, usize) {
    let count = DISPLAYS.max(current.saturating_add(1));
    let choices = (0..count)
        .map(|index| Choice {
            value: Some(index.to_string()),
            label: format!("Display {}", index + 1),
        })
        .collect();
    (choices, current as usize)
}

/// Whether another entry fits on `display`.
pub fn can_add(entries: &[LayoutEntry], display: u32) -> bool {
    on(entries, display) < MAX_PER_DISPLAY
}

/// Whether `display` holds no more than it may.
fn within_limit(entries: &[LayoutEntry], display: u32) -> bool {
    on(entries, display) <= MAX_PER_DISPLAY
}

fn on(entries: &[LayoutEntry], display: u32) -> usize {
    entries
        .iter()
        .filter(|entry| entry.display == display)
        .count()
}

/// A new entry on `display`: no application yet, the left half.
pub fn new_entry(display: u32) -> LayoutEntry {
    LayoutEntry {
        app: AppTarget {
            path: String::new(),
            name: String::new(),
        },
        argument: Argument::None,
        display,
        placement: Placement {
            width: Length::Percent(5000),
            height: Length::Percent(10_000),
            x: Offset::Points(0),
            y: Offset::Points(0),
            anchor: Anchor::Left,
            gaps: false,
        },
    }
}

/// The selection after removing `removed` from `count` entries: the one
/// after it, or the last, or none.
pub fn after_remove(count: usize, removed: usize) -> Option<usize> {
    match count.saturating_sub(1) {
        0 => None,
        left => Some(removed.min(left - 1)),
    }
}

/// What holds Save back, first thing first; `None` when it can be saved.
/// [`WindowLayoutSpec::validate`] decides; a full display is named as the
/// editor numbers displays, from 1, where `validate` counts from 0.
pub fn problem(name: &str, spec: Option<&WindowLayoutSpec>) -> Option<String> {
    let Some(spec) = spec else {
        return Some("Fix the figures marked on the selected application".into());
    };
    if name.trim().is_empty() {
        return Some("Give the layout a name".into());
    }
    if spec.entries.is_empty() {
        return Some("Add an application".into());
    }
    let why = spec.validate().err()?;
    let full = (0..=spec.entries.iter().map(|entry| entry.display).max()?)
        .find(|display| !within_limit(&spec.entries, *display));
    if let Some(display) = full {
        return Some(format!(
            "Display {} has more than {MAX_PER_DISPLAY} applications",
            display + 1
        ));
    }
    Some(why).map(|why| {
        let mut chars = why.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    })
}

/// What the editor asks of the page.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutEditorEvent {
    /// Save this: a new layout when `id` is `None`.
    Save {
        id: Option<String>,
        name: String,
        spec: WindowLayoutSpec,
    },
    Cancelled,
}

/// One entry's controls.
struct EntryForm {
    app: Entity<Select>,
    kind: Entity<Select>,
    argument: Entity<TextField>,
    display: Entity<Select>,
    placement: PlacementFields,
}

pub struct WindowLayoutEditor {
    colors: Colors,
    id: Option<String>,
    name: Entity<TextField>,
    gaps: bool,
    gaps_focus: FocusHandle,
    other: Entity<Select>,
    entries: Vec<EntryForm>,
    selected: Option<usize>,
    catalogue: Arc<Vec<Candidate>>,
    preview: Preview,
    _changes: Vec<Subscription>,
}

impl EventEmitter<LayoutEditorEvent> for WindowLayoutEditor {}

impl WindowLayoutEditor {
    pub fn new(
        draft: Draft,
        catalogue: Arc<Vec<Candidate>>,
        preview: Preview,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| TextField::new(draft.name, "Layout name", colors.clone(), cx));
        let choices = OTHER_WINDOWS
            .iter()
            .map(|(_, value, label)| Choice {
                value: Some((*value).into()),
                label: (*label).into(),
            })
            .collect();
        let at = OTHER_WINDOWS
            .iter()
            .position(|(other, _, _)| *other == draft.spec.other_windows)
            .unwrap_or(0);
        let other = cx.new(|cx| Select::new(choices, at, colors.clone(), cx).compact());
        let changes = vec![
            cx.subscribe(&name, |_, _, _: &TextFieldEvent, cx| cx.notify()),
            cx.subscribe(&other, |_, _, _: &SelectEvent, cx| cx.notify()),
        ];
        let mut editor = Self {
            colors,
            id: draft.id,
            name,
            gaps: draft.spec.gaps,
            gaps_focus: cx.focus_handle(),
            other,
            entries: Vec::new(),
            selected: None,
            catalogue,
            preview,
            _changes: changes,
        };
        for entry in draft.spec.entries {
            editor.push(entry, cx);
        }
        editor.selected = (!editor.entries.is_empty()).then_some(0);
        editor
    }

    fn push(&mut self, entry: LayoutEntry, cx: &mut Context<Self>) {
        let colors = self.colors.clone();
        let (apps, app_at) = app_choices(&self.catalogue, &entry.app);
        let (kind_value, text) = argument_parts(&entry.argument);
        let kinds = ARGUMENT_KINDS
            .iter()
            .map(|(value, label)| Choice {
                value: Some((*value).into()),
                label: (*label).into(),
            })
            .collect();
        let kind_at = ARGUMENT_KINDS
            .iter()
            .position(|(value, _)| *value == kind_value)
            .unwrap_or(0);
        let (displays, display_at) = display_choices(entry.display);
        let app = cx.new(|cx| Select::new(apps, app_at, colors.clone(), cx));
        let kind = cx.new(|cx| Select::new(kinds, kind_at, colors.clone(), cx).compact());
        let display = cx.new(|cx| Select::new(displays, display_at, colors.clone(), cx).compact());
        let argument =
            cx.new(|cx| TextField::new(text, "https://, a path or arguments", colors.clone(), cx));
        for select in [&app, &kind, &display] {
            self._changes
                .push(cx.subscribe(select, |_, _, _: &SelectEvent, cx| cx.notify()));
        }
        self._changes
            .push(cx.subscribe(&argument, |_, _, _: &TextFieldEvent, cx| cx.notify()));
        let placement = PlacementFields::new(entry.placement, &colors, cx);
        self.entries.push(EntryForm {
            app,
            kind,
            argument,
            display,
            placement,
        });
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.name.focus_handle(cx));
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.name
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.other
            .update(cx, |select, cx| select.set_colors(colors.clone(), cx));
        for entry in &self.entries {
            for select in [&entry.app, &entry.kind, &entry.display] {
                select.update(cx, |select, cx| select.set_colors(colors.clone(), cx));
            }
            entry
                .argument
                .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
            entry.placement.set_colors(&colors, cx);
        }
        self.colors = colors;
        cx.notify();
    }

    /// The entry as its controls stand, and why each figure does not read.
    fn read_entry(&self, form: &EntryForm, cx: &App) -> (Option<LayoutEntry>, [Option<String>; 4]) {
        let app = form.app.read(cx);
        let target = AppTarget {
            path: app.value().unwrap_or_default().to_string(),
            name: app.label().unwrap_or_default().to_string(),
        };
        let argument = argument_of(form.kind.read(cx).value(), form.argument.read(cx).text());
        let display = form
            .display
            .read(cx)
            .value()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let (placement, errors) = form.placement.read(false, cx);
        let entry = placement.map(|placement| LayoutEntry {
            app: target,
            argument,
            display,
            placement,
        });
        (entry, errors)
    }

    /// Every entry as it stands, when every figure reads.
    fn entries(&self, cx: &App) -> Option<Vec<LayoutEntry>> {
        self.entries
            .iter()
            .map(|form| self.read_entry(form, cx).0)
            .collect()
    }

    fn spec(&self, cx: &App) -> Option<WindowLayoutSpec> {
        Some(WindowLayoutSpec {
            entries: self.entries(cx)?,
            gaps: self.gaps,
            other_windows: other_windows_of(self.other.read(cx).value()),
        })
    }

    fn selected_form(editor: &mut Self) -> &mut PlacementFields {
        let index = editor.selected.unwrap_or_default();
        &mut editor.entries[index].placement
    }

    // -- Actions --------------------------------------------------------------

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_string();
        let spec = self.spec(cx);
        if problem(&name, spec.as_ref()).is_none()
            && let Some(spec) = spec
        {
            cx.emit(LayoutEditorEvent::Save {
                id: self.id.clone(),
                name,
                spec,
            });
        }
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(LayoutEditorEvent::Cancelled);
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let display = self.selected_display(cx);
        let room = self
            .entries(cx)
            .is_none_or(|entries| can_add(&entries, display));
        if room {
            self.push(new_entry(display), cx);
            self.selected = Some(self.entries.len() - 1);
            cx.notify();
        }
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected {
            self.entries.remove(index);
            self.selected = after_remove(self.entries.len() + 1, index);
            cx.notify();
        }
    }

    fn selected_display(&self, cx: &App) -> u32 {
        self.selected
            .and_then(|index| self.entries[index].display.read(cx).value())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// Name, gaps, other windows, then the selected entry's controls.
    fn step_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut stops = vec![
            self.name.focus_handle(cx),
            self.gaps_focus.clone(),
            self.other.read(cx).focus_handle().clone(),
        ];
        if let Some(form) = self.selected.map(|index| &self.entries[index]) {
            stops.push(form.app.read(cx).focus_handle().clone());
            stops.push(form.kind.read(cx).focus_handle().clone());
            if form.kind.read(cx).value() != Some("none") {
                stops.push(form.argument.focus_handle(cx));
            }
            stops.push(form.display.read(cx).focus_handle().clone());
            stops.extend(form.placement.field_stops(cx));
            stops.push(form.placement.grid_focus().clone());
        }
        let at = stops
            .iter()
            .position(|stop| stop.is_focused(window))
            .unwrap_or(0);
        let next = match forward {
            true => (at + 1) % stops.len(),
            false => (at + stops.len() - 1) % stops.len(),
        };
        window.focus(&stops[next]);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.step_focus(true, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.step_focus(false, window, cx);
    }

    fn flip_gaps(&mut self, _: &ToggleFallback, _: &mut Window, cx: &mut Context<Self>) {
        self.gaps = !self.gaps;
        cx.notify();
    }
}

impl Render for WindowLayoutEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let name = self.name.read(cx).text().to_string();
        let spec = self.spec(cx);
        let refused = problem(&name, spec.as_ref());
        let (message, is_error) = match &refused {
            Some(why) => (why.clone(), true),
            None => (
                "Ctrl+Enter saves. Esc goes back without saving.".into(),
                false,
            ),
        };
        let save = match self.id {
            Some(_) => "Save  Ctrl+Enter",
            None => "Create  Ctrl+Enter",
        };
        let buttons = form_buttons(
            "window-layout",
            save,
            refused.is_none(),
            &colors,
            cx.listener(|editor, _, window, cx| editor.cancel(&CancelForm, window, cx)),
            cx.listener(|editor, _, window, cx| editor.submit(&SubmitForm, window, cx)),
        );
        let gaps_focused = self.gaps_focus.is_focused(window);
        let gaps = div()
            .id("window-layout-gaps")
            .key_context(TOGGLE_CONTEXT)
            .track_focus(&self.gaps_focus)
            .cursor_pointer()
            .on_click(cx.listener(|editor, event: &ClickEvent, window, cx| {
                // The mouse only: Space comes through the keymap.
                if !event.is_keyboard() {
                    window.focus(&editor.gaps_focus);
                    editor.flip_gaps(&ToggleFallback, window, cx);
                }
            }))
            .child(ui::toggle(self.gaps, gaps_focused, &colors));

        let mut page = ui::page("window-layout-editor")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::flip_gaps))
            .child(ui::section_title(
                match self.id {
                    Some(_) => "Edit Layout",
                    None => "Create Layout",
                },
                &colors,
            ))
            .child(ui::card(
                [
                    ui::row("Name", None, ui::control(self.name.clone()), &colors),
                    ui::row(
                        "Use preferred gap settings",
                        Some("Leave the screen gap round each window".into()),
                        gaps,
                        &colors,
                    ),
                    ui::row(
                        "Other windows",
                        Some("On the displays the layout uses".into()),
                        self.other.clone(),
                        &colors,
                    ),
                ],
                &colors,
            ))
            .child(ui::section_title("Applications", &colors))
            .child(ui::card(self.entry_rows(&colors, cx), &colors));
        if let Some(index) = self.selected {
            page = page.child(ui::card([self.entry_detail(index, window, cx)], &colors));
        }
        page.child(ui::note(message, is_error, &colors))
            .child(buttons)
    }
}

impl WindowLayoutEditor {
    /// A row per entry, the selected one lit, then Add and Remove.
    fn entry_rows(&self, colors: &Colors, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(self.entries.len() + 1);
        for (index, form) in self.entries.iter().enumerate() {
            let (entry, _) = self.read_entry(form, cx);
            let title = form.app.read(cx).label().unwrap_or_default().to_string();
            let detail = match &entry {
                Some(entry) => format!(
                    "Display {} \u{b7} {}",
                    entry.display + 1,
                    summary(&entry.placement)
                ),
                None => "Figures to fix".into(),
            };
            let lit = self.selected == Some(index);
            rows.push(
                div()
                    .id(("window-layout-entry", index))
                    .cursor_pointer()
                    .when(lit, |row| row.bg(color(colors.selection_background)))
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.selected = Some(index);
                        cx.notify();
                    }))
                    .child(ui::row(title, Some(detail.into()), div(), colors))
                    .into_any_element(),
            );
        }
        let display = self.selected_display(cx);
        let full = self
            .entries(cx)
            .is_some_and(|entries| !can_add(&entries, display));
        let buttons = div()
            .flex()
            .gap(px(metrics::GAP))
            .when(self.selected.is_some(), |buttons| {
                buttons.child(text_button(
                    "window-layout-remove",
                    "Remove",
                    colors,
                    cx.listener(|editor, _, _, cx| editor.remove(cx)),
                ))
            })
            .when(!full, |buttons| {
                buttons.child(text_button(
                    "window-layout-add",
                    "Add Application",
                    colors,
                    cx.listener(|editor, _, _, cx| editor.add(cx)),
                ))
            });
        let count = format!(
            "{} \u{b7} at most {MAX_PER_DISPLAY} on a display",
            window_layouts::apps(self.entries.len())
        );
        rows.push(ui::row(count, None, buttons, colors));
        rows
    }

    /// The selected entry: its controls beside the preview of its display.
    fn entry_detail(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = self.colors.clone();
        let form = &self.entries[index];
        let (_, errors) = self.read_entry(form, cx);
        let with_argument = form.kind.read(cx).value() != Some("none");
        let argument: Div = div()
            .flex()
            .items_center()
            .gap(px(metrics::GAP))
            .child(form.kind.clone())
            .when(with_argument, |row| {
                row.child(ui::control(form.argument.clone()))
            });
        let fields = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(line("App", ui::control(form.app.clone()), &colors))
            .child(line("Start with", argument, &colors))
            .child(line("Display", form.display.clone(), &colors))
            .child(line(
                "Size",
                form.placement.pair(0, &colors, cx, Self::selected_form),
                &colors,
            ))
            .child(ui::note(either(&errors[0], &errors[1]), true, &colors))
            .child(line(
                "Offset",
                form.placement.pair(2, &colors, cx, Self::selected_form),
                &colors,
            ))
            .child(ui::note(either(&errors[2], &errors[3]), true, &colors))
            .child(line(
                "Position",
                form.placement
                    .grid(&colors, window, cx, Self::selected_form),
                &colors,
            ));
        div()
            .p(px(metrics::CARD_PADDING_X))
            .flex()
            .gap(px(metrics::CARD_ROW_GAP))
            .child(self.preview(index, &colors, cx))
            .child(fields)
            .into_any_element()
    }

    /// Every entry on the selected one's display, where it would go.
    fn preview(&self, selected: usize, colors: &Colors, cx: &App) -> Div {
        let display = self.selected_display(cx);
        let monitor = self.preview.monitor;
        let gaps = match self.gaps {
            true => self.preview.gaps,
            false => Default::default(),
        };
        let mut lit = None;
        let mut windows = Vec::new();
        for (index, form) in self.entries.iter().enumerate() {
            let (Some(entry), Some(monitor)) = (self.read_entry(form, cx).0, monitor) else {
                continue;
            };
            if entry.display == display {
                if index == selected {
                    lit = Some(windows.len());
                }
                windows.push(place_preview(&entry.placement, monitor, gaps));
            }
        }
        let caption: SharedString = match monitor {
            Some(_) => format!("Display {} \u{b7} drawn on this one", display + 1).into(),
            None => "No display to preview".into(),
        };
        display_preview(monitor, &windows, lit, caption, colors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::item::Item;

    fn entry(path: &str, display: u32) -> LayoutEntry {
        let mut entry = new_entry(display);
        entry.app = AppTarget {
            path: path.into(),
            name: path.into(),
        };
        entry
    }

    #[test]
    fn save_current_layout_opens_an_unsaved_snapshot() {
        let entries = vec![entry("a.exe", 0), entry("b.exe", 1)];
        let draft = Draft::current(entries.clone());
        assert_eq!(draft.id, None);
        assert_eq!(
            problem(&draft.name, Some(&draft.spec)).as_deref(),
            Some("Give the layout a name"),
            "named by the user before it can be saved"
        );
        assert_eq!(draft.spec.entries, entries);
        assert!(!draft.spec.gaps, "where the windows are, no gap added");
        assert_eq!(draft.spec.other_windows, OtherWindows::None);
        assert!(Draft::new().spec.entries.is_empty());
        assert!(Draft::new().spec.gaps);
    }

    #[test]
    fn eight_fit_on_a_display_and_a_ninth_does_not() {
        let mut entries: Vec<LayoutEntry> = Vec::new();
        for i in 0..MAX_PER_DISPLAY {
            assert!(can_add(&entries, 0));
            entries.push(entry(&format!("{i}.exe"), 0));
        }
        assert!(!can_add(&entries, 0));
        assert!(can_add(&entries, 1), "another display has room");
        assert_eq!(after_remove(3, 2), Some(1));
        assert_eq!(after_remove(3, 0), Some(0));
        assert_eq!(after_remove(1, 0), None);
    }

    #[test]
    fn an_argument_maps_to_its_kind_and_back() {
        for argument in [
            Argument::None,
            Argument::Url("https://x.com".into()),
            Argument::Path(r"C:\Notes".into()),
            Argument::Args("--new-window".into()),
        ] {
            let (kind, text) = argument_parts(&argument);
            assert!(ARGUMENT_KINDS.iter().any(|(value, _)| *value == kind));
            assert_eq!(argument_of(Some(kind), &text), argument);
        }
        assert_eq!(argument_of(None, "ignored"), Argument::None);
        for (other, value, _) in OTHER_WINDOWS {
            assert_eq!(other_windows_of(Some(value)), other);
            // What the editor writes is what the file reads.
            assert_eq!(serde_json::to_value(other).unwrap(), value);
        }
    }

    #[test]
    fn what_holds_save_back_is_said() {
        let spec = |entries| WindowLayoutSpec {
            entries,
            ..Draft::new().spec
        };
        assert!(problem("X", None).unwrap().contains("figures"));
        assert_eq!(
            problem(" ", Some(&spec(vec![entry("a", 0)]))).as_deref(),
            Some("Give the layout a name")
        );
        assert_eq!(
            problem("X", Some(&spec(Vec::new()))).as_deref(),
            Some("Add an application")
        );
        assert_eq!(
            problem("X", Some(&spec(vec![new_entry(0)]))).as_deref(),
            Some("An entry names no application")
        );
        let nine = (0..9).map(|i| entry(&i.to_string(), 2)).collect();
        let why = problem("X", Some(&spec(nine))).unwrap();
        assert_eq!(why, "Display 3 has more than 8 applications");
        assert_eq!(problem("X", Some(&spec(vec![entry("a", 0)]))), None);
    }

    #[test]
    fn the_apps_are_the_catalogues_applications_and_a_stranger_is_kept() {
        let item = |title: &str, kind, action| {
            Candidate::new(Item {
                id: format!("app:{title}"),
                title: title.into(),
                subtitle: None,
                icon: None,
                keywords: Vec::new(),
                actions: vec![action],
                kind,
            })
        };
        let shell = |path: &str| Action::Launch(LaunchTarget::ShellExecute(path.into()));
        let catalogue = vec![
            item("Zed", Kind::Application, shell(r"C:\Zed.lnk")),
            item("Calculator", Kind::Application, {
                Action::Launch(LaunchTarget::AppUserModelId("Calc!App".into()))
            }),
            item("notes.txt", Kind::File, shell(r"C:\notes.txt")),
        ];
        let none = AppTarget {
            path: String::new(),
            name: String::new(),
        };
        let (choices, at) = app_choices(&catalogue, &none);
        let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Choose an application", "Calculator", "Zed"]);
        assert_eq!(at, 0);
        assert_eq!(
            choices[1].value.as_deref(),
            Some(r"shell:AppsFolder\Calc!App")
        );

        let zed = AppTarget {
            path: r"c:\zed.LNK".into(),
            name: "Zed".into(),
        };
        assert_eq!(app_choices(&catalogue, &zed).1, 2);
        let found = AppTarget {
            path: r"C:\Code\Code.exe".into(),
            name: "Code".into(),
        };
        let (choices, at) = app_choices(&catalogue, &found);
        assert_eq!(choices[at].label, "Code");
        assert_eq!(choices[at].value.as_deref(), Some(r"C:\Code\Code.exe"));
    }

    #[test]
    fn displays_are_offered_by_number_and_a_far_one_is_kept() {
        let (choices, at) = display_choices(1);
        assert_eq!(choices.len(), DISPLAYS as usize);
        assert_eq!((choices[at].label.as_str(), at), ("Display 2", 1));
        let (choices, at) = display_choices(6);
        assert_eq!(choices.len(), 7);
        assert_eq!(choices[at].value.as_deref(), Some("6"));
    }
}
