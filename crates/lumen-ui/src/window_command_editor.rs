//! The editor for a custom window command (F17, 7.59), after Raycast's
//! *Create Command*.
//!
//! A preview of the display on the left, the window where the command puts
//! it drawn inside; the name, size, offset, gaps and anchor on the right.
//! Every field is read through [`Placement`]'s own parsing as it is typed, so
//! what the editor accepts is exactly what the file does, and a field that
//! does not read says why under itself and holds Create back.
//!
//! The editor writes nothing: it tells the page what to save, and the page
//! writes `window-commands.json`, which the daemon watches.

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window, actions, div, px,
};
use lumen_core::layout::settings as metrics;
use lumen_core::providers::window_commands::CustomWindowCommand;
use lumen_core::theme::Colors;
use lumen_platform::{Anchor, Gaps, Length, MonitorInfo, Offset, Placement, Rect};

use crate::placement_fields::{
    PlacementFields, display_preview, either, form_buttons, line, place_preview,
};
use crate::quicklink_form::{
    CancelForm, NextField, PreviousField, SubmitForm, TOGGLE_CONTEXT, ToggleFallback,
};
use crate::settings_ui as ui;
use crate::text_field::{TextField, TextFieldEvent};

actions!(
    lumen_window_commands,
    [
        /// Move the anchor up a row.
        MoveAnchorUp,
        /// Move the anchor down a row.
        MoveAnchorDown,
        /// Move the anchor left a column.
        MoveAnchorLeft,
        /// Move the anchor right a column.
        MoveAnchorRight,
    ]
);

/// The editor's key context.
pub const CONTEXT: &str = "WindowCommandForm";

/// The anchor grid's, inside the editor's: the arrow keys move the anchor.
pub const ANCHOR_CONTEXT: &str = "AnchorGrid";

/// What the caption calls the display: the platform names none.
pub const DISPLAY: &str = "This display";

/// What the field's figure is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Points,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Percent => "%",
            Self::Points => "pt",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Percent => Self::Points,
            Self::Points => Self::Percent,
        }
    }
}

/// A size as its field shows it: the figure and its unit. Auto is empty.
pub fn length_field(length: Length) -> (String, Unit) {
    match length {
        Length::Auto => (String::new(), Unit::Percent),
        Length::Percent(_) => (figure(length.to_string()), Unit::Percent),
        Length::Points(_) => (figure(length.to_string()), Unit::Points),
    }
}

/// An offset as its field shows it. No offset is empty.
pub fn offset_field(offset: Offset) -> (String, Unit) {
    match offset {
        Offset::Percent(0) => (String::new(), Unit::Percent),
        Offset::Points(0) => (String::new(), Unit::Points),
        Offset::Percent(_) => (figure(offset.to_string()), Unit::Percent),
        Offset::Points(_) => (figure(offset.to_string()), Unit::Points),
    }
}

fn figure(text: String) -> String {
    text.trim_end_matches(['%', 'p', 't']).to_string()
}

/// The field's text with its unit, unless it was typed with one.
fn with_unit(text: &str, unit: Unit) -> String {
    if text.ends_with('%') || text.ends_with("pt") || text.eq_ignore_ascii_case("auto") {
        text.to_string()
    } else {
        format!("{text}{}", unit.suffix())
    }
}

/// A size field read as the file reads it. Empty is Auto.
pub fn parse_length(text: &str, unit: Unit) -> Result<Length, String> {
    match text.trim() {
        "" => Ok(Length::Auto),
        text => with_unit(text, unit).parse(),
    }
}

/// An offset field read as the file reads it. Empty is none.
pub fn parse_offset(text: &str, unit: Unit) -> Result<Offset, String> {
    match (text.trim(), unit) {
        ("", Unit::Percent) => Ok(Offset::Percent(0)),
        ("", Unit::Points) => Ok(Offset::Points(0)),
        (text, _) => with_unit(text, unit).parse(),
    }
}

/// The four fields read: the placement when all of them read, and why each
/// that does not.
pub fn read_placement(
    texts: [&str; 4],
    units: [Unit; 4],
    anchor: Anchor,
    gaps: bool,
) -> (Option<Placement>, [Option<String>; 4]) {
    let width = parse_length(texts[0], units[0]);
    let height = parse_length(texts[1], units[1]);
    let x = parse_offset(texts[2], units[2]);
    let y = parse_offset(texts[3], units[3]);
    let errors = [
        width.as_ref().err().cloned(),
        height.as_ref().err().cloned(),
        x.as_ref().err().cloned(),
        y.as_ref().err().cloned(),
    ];
    let placement = match (width, height, x, y) {
        (Ok(width), Ok(height), Ok(x), Ok(y)) => Some(Placement {
            width,
            height,
            x,
            y,
            anchor,
            gaps,
        }),
        _ => None,
    };
    (placement, errors)
}

/// A step through the anchor grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
    Left,
    Right,
}

/// The anchor a step away, stopping at the grid's edges.
pub fn move_anchor(anchor: Anchor, step: Step) -> Anchor {
    let index = Anchor::ALL.iter().position(|a| *a == anchor).unwrap_or(4);
    let (column, row) = (index % 3, index / 3);
    let (column, row) = match step {
        Step::Up => (column, row.saturating_sub(1)),
        Step::Down => (column, (row + 1).min(2)),
        Step::Left => (column.saturating_sub(1), row),
        Step::Right => ((column + 1).min(2), row),
    };
    Anchor::ALL[row * 3 + column]
}

pub fn anchor_name(anchor: Anchor) -> &'static str {
    match anchor {
        Anchor::TopLeft => "top left",
        Anchor::Top => "top",
        Anchor::TopRight => "top right",
        Anchor::Left => "left",
        Anchor::Center => "centre",
        Anchor::Right => "right",
        Anchor::BottomLeft => "bottom left",
        Anchor::Bottom => "bottom",
        Anchor::BottomRight => "bottom right",
    }
}

/// A command in a line: `60% × auto, centre`, then its offset and gaps when
/// it has them.
pub fn summary(placement: &Placement) -> String {
    let mut text = format!(
        "{} \u{d7} {}, {}",
        placement.width,
        placement.height,
        anchor_name(placement.anchor)
    );
    let none = |offset: Offset| matches!(offset, Offset::Percent(0) | Offset::Points(0));
    if !none(placement.x) || !none(placement.y) {
        text.push_str(&format!(", offset {} {}", placement.x, placement.y));
    }
    if placement.gaps {
        text.push_str(", with gaps");
    }
    text
}

/// The window the preview places: 60% by 60% of the work area, centred.
pub fn representative(work_area: Rect) -> Rect {
    let (width, height) = (work_area.width * 3 / 5, work_area.height * 3 / 5);
    Rect::new(
        work_area.x + (work_area.width - width) / 2,
        work_area.y + (work_area.height - height) / 2,
        width,
        height,
    )
}

/// A box in the preview, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaled {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The work area fitted inside a `frame` of logical pixels, centred, and
/// `placed` inside that, relative to it.
pub fn fit(work_area: Rect, placed: Rect, frame: (f32, f32)) -> (Scaled, Scaled) {
    let scale = if work_area.width > 0 && work_area.height > 0 {
        (frame.0 / work_area.width as f32).min(frame.1 / work_area.height as f32)
    } else {
        0.0
    };
    let (width, height) = (
        work_area.width as f32 * scale,
        work_area.height as f32 * scale,
    );
    let screen = Scaled {
        x: (frame.0 - width) / 2.0,
        y: (frame.1 - height) / 2.0,
        width,
        height,
    };
    let window = Scaled {
        x: (placed.x - work_area.x) as f32 * scale,
        y: (placed.y - work_area.y) as f32 * scale,
        width: placed.width as f32 * scale,
        height: placed.height as f32 * scale,
    };
    (screen, window)
}

/// What the preview draws on: the display the settings window is on, and
/// the screen gap there, in physical pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Preview {
    pub monitor: Option<MonitorInfo>,
    pub gaps: Gaps,
}

/// What the editor asks of the page.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    /// Save this: a new command when `id` is `None`.
    Save {
        id: Option<String>,
        name: String,
        placement: Placement,
    },
    Cancelled,
}

pub struct WindowCommandEditor {
    colors: Colors,
    /// The command edited; `None` while creating one.
    id: Option<String>,
    name: Entity<TextField>,
    placement: PlacementFields,
    gaps: bool,
    gaps_focus: FocusHandle,
    preview: Preview,
    _changes: Subscription,
}

impl EventEmitter<EditorEvent> for WindowCommandEditor {}

impl WindowCommandEditor {
    /// On `command`, or on a new one: 60% wide, its height kept, centred.
    pub fn new(
        command: Option<CustomWindowCommand>,
        preview: Preview,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let (id, name, placement) = match command {
            Some(command) => (Some(command.id), command.name, command.placement),
            None => (
                None,
                String::new(),
                Placement {
                    width: Length::Percent(6000),
                    height: Length::Auto,
                    x: Offset::Points(0),
                    y: Offset::Points(0),
                    anchor: Anchor::Center,
                    gaps: false,
                },
            ),
        };
        let name = cx.new(|cx| TextField::new(name, "Reading", colors.clone(), cx));
        let changes = cx.subscribe(&name, |_, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Changed(_)) {
                cx.notify();
            }
        });
        Self {
            id,
            name,
            placement: PlacementFields::new(placement, &colors, cx),
            gaps: placement.gaps,
            gaps_focus: cx.focus_handle(),
            preview,
            colors,
            _changes: changes,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.name.focus_handle(cx));
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.name
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.placement.set_colors(&colors, cx);
        self.colors = colors;
        cx.notify();
    }

    fn read(&self, cx: &App) -> (Option<Placement>, [Option<String>; 4]) {
        self.placement.read(self.gaps, cx)
    }

    fn fields(editor: &mut Self) -> &mut PlacementFields {
        &mut editor.placement
    }

    /// The name to save, and the placement, when both are there.
    fn ready(&self, cx: &App) -> Option<(String, Placement)> {
        let name = self.name.read(cx).text().trim().to_string();
        let placement = self.read(cx).0?;
        (!name.is_empty()).then_some((name, placement))
    }

    // -- Actions --------------------------------------------------------------

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((name, placement)) = self.ready(cx) {
            cx.emit(EditorEvent::Save {
                id: self.id.clone(),
                name,
                placement,
            });
        }
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::Cancelled);
    }

    /// Name, the four figures, gaps, the grid.
    fn step_focus(&mut self, by: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut stops = vec![self.name.focus_handle(cx)];
        stops.extend(self.placement.field_stops(cx));
        stops.extend([self.gaps_focus.clone(), self.placement.grid_focus().clone()]);
        let at = stops
            .iter()
            .position(|stop| stop.is_focused(window))
            .unwrap_or(0);
        window.focus(&stops[(at + by) % stops.len()]);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.step_focus(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.step_focus(6, window, cx);
    }

    fn flip_gaps(&mut self, _: &ToggleFallback, _: &mut Window, cx: &mut Context<Self>) {
        self.gaps = !self.gaps;
        cx.notify();
    }
}

impl Render for WindowCommandEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let (placement, errors) = self.read(cx);
        let ready = self.ready(cx).is_some();
        let message = match (self.name.read(cx).text().trim().is_empty(), placement) {
            (true, _) => "Give the command a name",
            (false, None) => "Fix the figures marked above",
            (false, Some(_)) => "Ctrl+Enter saves. Esc goes back without saving.",
        };
        let save = match self.id {
            Some(_) => "Save  Ctrl+Enter",
            None => "Create  Ctrl+Enter",
        };
        let buttons = form_buttons(
            "window-command",
            save,
            ready,
            &colors,
            cx.listener(|editor, _, window, cx| editor.cancel(&CancelForm, window, cx)),
            cx.listener(|editor, _, window, cx| editor.submit(&SubmitForm, window, cx)),
        );

        let fields = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(line("Name", ui::control(self.name.clone()), &colors))
            .child(line(
                "Size",
                self.placement.pair(0, &colors, cx, Self::fields),
                &colors,
            ))
            .child(ui::note(either(&errors[0], &errors[1]), true, &colors))
            .child(line(
                "Offset",
                self.placement.pair(2, &colors, cx, Self::fields),
                &colors,
            ))
            .child(ui::note(either(&errors[2], &errors[3]), true, &colors))
            .child(line("Use gaps", self.gaps_toggle(window, cx), &colors))
            .child(line(
                "Position",
                self.placement.grid(&colors, window, cx, Self::fields),
                &colors,
            ));

        ui::page("window-command-editor")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::flip_gaps))
            .child(ui::section_title(
                match self.id {
                    Some(_) => "Edit Command",
                    None => "Create Command",
                },
                &colors,
            ))
            .child(ui::card(
                [div()
                    .p(px(metrics::CARD_PADDING_X))
                    .flex()
                    .gap(px(metrics::CARD_ROW_GAP))
                    .child(self.preview(placement, &colors))
                    .child(fields)
                    .into_any_element()],
                &colors,
            ))
            .child(ui::note(message, false, &colors))
            .child(buttons)
    }
}

impl WindowCommandEditor {
    fn gaps_toggle(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.gaps_focus.is_focused(window);
        div()
            .id("window-command-gaps")
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
            .child(ui::toggle(self.gaps, focused, &self.colors))
    }

    /// The display, scaled into a fixed box, and the window where the
    /// command would put one at 60% by 60% in the middle of it.
    fn preview(&self, placement: Option<Placement>, colors: &Colors) -> impl IntoElement {
        let monitor = self.preview.monitor;
        let placed = monitor.and_then(|monitor| {
            Some((
                monitor,
                place_preview(&placement?, monitor, self.preview.gaps),
            ))
        });
        let caption: SharedString = match placed {
            Some((monitor, rect)) => {
                let logical = |pixels: i32| (pixels as f32 / monitor.scale_factor).round();
                format!(
                    "{DISPLAY} \u{b7} {} \u{d7} {}",
                    logical(rect.width),
                    logical(rect.height)
                )
                .into()
            }
            None if monitor.is_none() => "No display to preview".into(),
            None => DISPLAY.into(),
        };
        let windows: Vec<Rect> = placed.map(|(_, rect)| rect).into_iter().collect();
        display_preview(monitor, &windows, Some(0), caption, colors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_shows_a_figure_and_its_unit_and_reads_back() {
        for length in [
            Length::Auto,
            Length::Percent(6000),
            Length::Percent(3333),
            Length::Points(640),
        ] {
            let (text, unit) = length_field(length);
            assert_eq!(parse_length(&text, unit), Ok(length), "{length}");
        }
        assert_eq!(length_field(Length::Percent(3330)).0, "33.3");
        assert_eq!(
            length_field(Length::Points(640)),
            ("640".into(), Unit::Points)
        );
        assert_eq!(length_field(Length::Auto).0, "");
        for offset in [
            Offset::Percent(-1000),
            Offset::Points(24),
            Offset::Percent(0),
        ] {
            let (text, unit) = offset_field(offset);
            assert_eq!(parse_offset(&text, unit), Ok(offset), "{offset}");
        }
        assert_eq!(
            offset_field(Offset::Points(0)),
            (String::new(), Unit::Points)
        );
    }

    #[test]
    fn the_unit_is_the_toggles_unless_typed() {
        assert_eq!(parse_length("50", Unit::Percent), Ok(Length::Percent(5000)));
        assert_eq!(parse_length("50", Unit::Points), Ok(Length::Points(50)));
        assert_eq!(parse_length("50pt", Unit::Percent), Ok(Length::Points(50)));
        assert_eq!(parse_length(" auto ", Unit::Points), Ok(Length::Auto));
        assert_eq!(parse_offset("-5", Unit::Points), Ok(Offset::Points(-5)));
        assert_eq!(Unit::Percent.other(), Unit::Points);
    }

    #[test]
    fn a_bad_figure_says_why_and_holds_the_placement_back() {
        let units = [Unit::Percent, Unit::Percent, Unit::Points, Unit::Percent];
        let (placement, errors) =
            read_placement(["150", "", "1.5", "-200"], units, Anchor::Center, false);
        assert_eq!(placement, None);
        assert!(errors[0].as_deref().unwrap().contains("at most 100%"));
        assert_eq!(errors[1], None);
        assert!(errors[2].as_deref().unwrap().contains("whole number"));
        assert!(
            errors[3]
                .as_deref()
                .unwrap()
                .contains("between -100% and 100%")
        );
        assert!(
            parse_length("0", Unit::Points)
                .unwrap_err()
                .contains("more than 0pt")
        );
        assert!(parse_length("abc", Unit::Percent).is_err());

        let (placement, errors) = read_placement(["60", "", "", "10"], units, Anchor::Top, true);
        assert_eq!(errors, [None, None, None, None]);
        let placement = placement.unwrap();
        assert_eq!(
            (placement.width, placement.height, placement.y),
            (Length::Percent(6000), Length::Auto, Offset::Percent(1000))
        );
        assert_eq!((placement.anchor, placement.gaps), (Anchor::Top, true));
    }

    #[test]
    fn the_arrows_move_the_anchor_and_stop_at_the_edges() {
        assert_eq!(move_anchor(Anchor::Center, Step::Up), Anchor::Top);
        assert_eq!(move_anchor(Anchor::Center, Step::Right), Anchor::Right);
        assert_eq!(move_anchor(Anchor::Left, Step::Down), Anchor::BottomLeft);
        assert_eq!(move_anchor(Anchor::TopLeft, Step::Up), Anchor::TopLeft);
        assert_eq!(move_anchor(Anchor::TopLeft, Step::Left), Anchor::TopLeft);
        assert_eq!(
            move_anchor(Anchor::BottomRight, Step::Down),
            Anchor::BottomRight
        );
        assert_eq!(move_anchor(Anchor::Bottom, Step::Up), Anchor::Center);
    }

    #[test]
    fn a_command_reads_in_a_line() {
        let mut placement = Placement {
            width: Length::Percent(6000),
            height: Length::Auto,
            x: Offset::Points(0),
            y: Offset::Percent(0),
            anchor: Anchor::Center,
            gaps: false,
        };
        assert_eq!(summary(&placement), "60% \u{d7} auto, centre");
        placement.y = Offset::Percent(-1000);
        placement.anchor = Anchor::TopRight;
        placement.gaps = true;
        assert_eq!(
            summary(&placement),
            "60% \u{d7} auto, top right, offset 0pt -10%, with gaps"
        );
    }

    #[test]
    fn the_preview_fits_the_display_and_scales_the_window_with_it() {
        let work = Rect::new(0, 0, 1920, 1080);
        let window = Rect::new(960, 0, 960, 1080);
        let (screen, placed) = fit(work, window, (240.0, 150.0));
        // Wider than the frame: full width, centred top to bottom.
        assert_eq!((screen.width, screen.height), (240.0, 135.0));
        assert_eq!((screen.x, screen.y), (0.0, 7.5));
        assert_eq!(
            (placed.x, placed.y, placed.width, placed.height),
            (120.0, 0.0, 120.0, 135.0)
        );

        // A second display off the origin, taller than the frame.
        let work = Rect::new(-1080, 100, 1080, 1920);
        let (screen, placed) = fit(work, Rect::new(-1080, 100, 540, 960), (240.0, 150.0));
        assert!((screen.height - 150.0).abs() < 0.01);
        assert!((placed.width * 2.0 - screen.width).abs() < 0.01);
        assert_eq!((placed.x, placed.y), (0.0, 0.0));

        // Nothing to fit is nothing drawn, not a division by zero.
        let (screen, _) = fit(Rect::default(), window, (240.0, 150.0));
        assert_eq!(screen.width, 0.0);
        assert_eq!(
            representative(Rect::new(100, 0, 1000, 500)),
            Rect::new(300, 100, 600, 300)
        );
    }
}
