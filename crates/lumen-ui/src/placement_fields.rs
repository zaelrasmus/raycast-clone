//! The parts of a [`Placement`] the window editors share (F17, 7.59 and
//! 7.60): the four figures with their units, the anchor grid, the scaled
//! display preview, and the Cancel and Save buttons.
//!
//! [`PlacementFields`] is held by an editor rather than being an entity of
//! its own: the editor's listeners reach it again through a `get` function,
//! so its focus stops sit in the editor's own Tab order.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, SharedString,
    Subscription, Window, div, px,
};
use lumen_core::layout::settings as metrics;
use lumen_core::theme::Colors;
use lumen_platform::{Anchor, Gaps, MonitorInfo, Placement, Rect};

use crate::settings::text_button;
use crate::settings_ui as ui;
use crate::styling::color;
use crate::text_field::{TextField, TextFieldEvent};
use crate::window_command_editor::{
    ANCHOR_CONTEXT, MoveAnchorDown, MoveAnchorLeft, MoveAnchorRight, MoveAnchorUp, Scaled, Step,
    Unit, fit, length_field, move_anchor, offset_field, read_placement, representative,
};

/// The four figures, in the order they are drawn and focused.
const FIELDS: [&str; 4] = ["W", "H", "X", "Y"];

/// Size, offset and anchor, as fields.
pub struct PlacementFields {
    fields: [Entity<TextField>; 4],
    units: [Unit; 4],
    anchor: Anchor,
    grid_focus: FocusHandle,
    _changes: Vec<Subscription>,
}

impl PlacementFields {
    /// On `placement`; its holder is notified as a figure is typed.
    pub fn new<T: 'static>(placement: Placement, colors: &Colors, cx: &mut Context<T>) -> Self {
        let shown = [
            length_field(placement.width),
            length_field(placement.height),
            offset_field(placement.x),
            offset_field(placement.y),
        ];
        let placeholders = ["Auto", "Auto", "0", "0"];
        let fields = [0, 1, 2, 3].map(|index| {
            let text = shown[index].0.clone();
            let placeholder = placeholders[index];
            cx.new(|cx| TextField::new(text, placeholder, colors.clone(), cx))
        });
        let changes = fields
            .iter()
            .map(|field| {
                cx.subscribe(field, |_, _, event: &TextFieldEvent, cx| {
                    if matches!(event, TextFieldEvent::Changed(_)) {
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            fields,
            units: shown.map(|(_, unit)| unit),
            anchor: placement.anchor,
            grid_focus: cx.focus_handle(),
            _changes: changes,
        }
    }

    /// The placement when every figure reads, and why each that does not.
    pub fn read(&self, gaps: bool, cx: &App) -> (Option<Placement>, [Option<String>; 4]) {
        let texts = [0, 1, 2, 3].map(|index| self.fields[index].read(cx).text());
        read_placement(texts, self.units, self.anchor, gaps)
    }

    /// The four figures' focus, in order.
    pub fn field_stops(&self, cx: &App) -> Vec<FocusHandle> {
        self.fields
            .iter()
            .map(|field| field.focus_handle(cx))
            .collect()
    }

    pub fn grid_focus(&self) -> &FocusHandle {
        &self.grid_focus
    }

    pub fn set_colors(&self, colors: &Colors, cx: &mut App) {
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
    }

    /// Two figures side by side, each with its unit: W and H, or X and Y.
    pub fn pair<T: 'static>(
        &self,
        first: usize,
        colors: &Colors,
        cx: &mut Context<T>,
        get: fn(&mut T) -> &mut Self,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(metrics::GAP))
            .children([first, first + 1].map(|index| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(metrics::ICON_GAP))
                    .child(
                        div()
                            .text_size(px(metrics::text::LABEL))
                            .text_color(color(colors.text_muted))
                            .child(FIELDS[index]),
                    )
                    .child(ui::narrow(self.fields[index].clone()))
                    .child(div().w(px(metrics::UNIT_WIDTH)).child(text_button(
                        ("window-command-unit", index),
                        self.units[index].suffix(),
                        colors,
                        cx.listener(move |editor, _, _, cx| {
                            let fields = get(editor);
                            fields.units[index] = fields.units[index].other();
                            cx.notify();
                        }),
                    )))
            }))
    }

    /// Nine cells, the anchor's filled; the arrow keys move it while the
    /// grid has focus.
    pub fn grid<T: 'static>(
        &self,
        colors: &Colors,
        window: &Window,
        cx: &mut Context<T>,
        get: fn(&mut T) -> &mut Self,
    ) -> impl IntoElement + use<T> {
        let focused = self.grid_focus.is_focused(window);
        let cell = |index: usize| {
            let anchor = Anchor::ALL[index];
            let selected = anchor == self.anchor;
            div()
                .id(("window-command-anchor", index))
                .size(px(metrics::ANCHOR_CELL))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(metrics::KEYCAP_RADIUS))
                .bg(color(match selected {
                    true => colors.accent,
                    false => colors.surface_background,
                }))
                .cursor_pointer()
                .on_click(cx.listener(move |editor, _, window, cx| {
                    let fields = get(editor);
                    window.focus(&fields.grid_focus);
                    fields.anchor = anchor;
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(metrics::ANCHOR_MARK))
                        .rounded_full()
                        .bg(color(match selected {
                            true => colors.text_on_accent,
                            false => colors.border_strong,
                        })),
                )
        };
        let rows: Vec<Div> = (0..3)
            .map(|row| {
                div().flex().gap(px(metrics::ANCHOR_GAP)).children(
                    (0..3)
                        .map(|column| cell(row * 3 + column))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let step = move |step: Step| {
            move |editor: &mut T, cx: &mut Context<T>| {
                let fields = get(editor);
                fields.anchor = move_anchor(fields.anchor, step);
                cx.notify();
            }
        };
        let (up, down, left, right) = (
            step(Step::Up),
            step(Step::Down),
            step(Step::Left),
            step(Step::Right),
        );
        div()
            .id("window-command-grid")
            .key_context(ANCHOR_CONTEXT)
            .track_focus(&self.grid_focus)
            .on_action(cx.listener(move |editor, _: &MoveAnchorUp, _, cx| up(editor, cx)))
            .on_action(cx.listener(move |editor, _: &MoveAnchorDown, _, cx| down(editor, cx)))
            .on_action(cx.listener(move |editor, _: &MoveAnchorLeft, _, cx| left(editor, cx)))
            .on_action(cx.listener(move |editor, _: &MoveAnchorRight, _, cx| right(editor, cx)))
            .my(px(metrics::GAP))
            .p(px(metrics::ANCHOR_GAP))
            .flex()
            .flex_col()
            .gap(px(metrics::ANCHOR_GAP))
            .rounded(px(metrics::FIELD_RADIUS))
            .border_1()
            .border_color(color(match focused {
                true => colors.accent,
                false => colors.border_subtle,
            }))
            .children(rows)
    }
}

/// Where `placement` puts the preview's representative window on `monitor`.
pub fn place_preview(placement: &Placement, monitor: MonitorInfo, gaps: Gaps) -> Rect {
    placement.place(
        monitor.work_area,
        representative(monitor.work_area),
        monitor.scale_factor,
        gaps,
    )
}

/// `windows` scaled into the preview's box with the display, which is
/// `None` when there is no display to draw.
pub fn scaled_windows(
    monitor: Option<MonitorInfo>,
    windows: &[Rect],
) -> Option<(Scaled, Vec<Scaled>)> {
    let frame = (metrics::PREVIEW_WIDTH, metrics::PREVIEW_HEIGHT);
    let work_area = monitor?.work_area;
    let (screen, _) = fit(work_area, Rect::default(), frame);
    let scaled = windows
        .iter()
        .map(|&rect| fit(work_area, rect, frame).1)
        .collect();
    Some((screen, scaled))
}

/// The display scaled into the preview's fixed box, `windows` drawn inside
/// it -- the `highlighted` one in the accent -- and a caption under it.
pub fn display_preview(
    monitor: Option<MonitorInfo>,
    windows: &[Rect],
    highlighted: Option<usize>,
    caption: SharedString,
    colors: &Colors,
) -> Div {
    let boxed = |scaled: Scaled| {
        div()
            .absolute()
            .left(px(scaled.x))
            .top(px(scaled.y))
            .w(px(scaled.width))
            .h(px(scaled.height))
            .rounded(px(metrics::KEYCAP_RADIUS))
            .border_1()
    };
    let drawn = scaled_windows(monitor, windows).map(|(screen, windows)| {
        boxed(screen)
            .bg(color(colors.surface_background))
            .border_color(color(colors.border_strong))
            .children(windows.into_iter().enumerate().map(|(index, window)| {
                let lit = highlighted == Some(index);
                boxed(window)
                    .bg(color(match lit {
                        true => colors.selection_background,
                        false => colors.surface_elevated,
                    }))
                    .border_color(color(match lit {
                        true => colors.accent,
                        false => colors.border_subtle,
                    }))
            }))
    });
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(metrics::GAP))
        .child(
            div()
                .relative()
                .w(px(metrics::PREVIEW_WIDTH))
                .h(px(metrics::PREVIEW_HEIGHT))
                .children(drawn),
        )
        .child(
            div()
                .w(px(metrics::PREVIEW_WIDTH))
                .truncate()
                .text_size(px(metrics::text::HINT))
                .text_color(color(colors.text_muted))
                .child(caption),
        )
}

/// Cancel, and Save or Create, pressable only when `ready`.
pub fn form_buttons(
    id: &'static str,
    save: &'static str,
    ready: bool,
    colors: &Colors,
    cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    submit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .justify_end()
        .gap(px(metrics::GAP))
        .child(text_button(
            SharedString::from(format!("{id}-cancel")),
            "Cancel  Esc",
            colors,
            cancel,
        ))
        .child(match ready {
            true => text_button(
                SharedString::from(format!("{id}-save")),
                save,
                colors,
                submit,
            )
            .into_any_element(),
            false => disabled_button(save, colors),
        })
}

/// A label and its control, one row of a form.
pub fn line(label: &'static str, control: impl IntoElement, colors: &Colors) -> Div {
    div()
        .min_h(px(metrics::CARD_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap(px(metrics::GAP))
        .child(
            div()
                .w(px(metrics::FORM_LABEL_WIDTH))
                .flex_none()
                .text_size(px(metrics::text::LABEL))
                .text_color(color(colors.text_primary))
                .child(label),
        )
        .child(control)
}

/// The first of two problems, or nothing.
pub fn either(first: &Option<String>, second: &Option<String>) -> String {
    first.clone().or_else(|| second.clone()).unwrap_or_default()
}

/// A button that cannot be pressed yet: as a text button, muted.
fn disabled_button(label: &'static str, colors: &Colors) -> AnyElement {
    div()
        .h(px(metrics::FIELD_HEIGHT))
        .px(px(metrics::FIELD_PADDING))
        .flex()
        .items_center()
        .rounded(px(metrics::FIELD_RADIUS))
        .border_1()
        .border_color(color(colors.border_subtle))
        .text_size(px(metrics::text::LABEL))
        .text_color(color(colors.text_placeholder))
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_windows_are_scaled_with_their_display() {
        let monitor = MonitorInfo {
            bounds: Rect::new(0, 0, 1920, 1080),
            work_area: Rect::new(0, 0, 1920, 1080),
            scale_factor: 1.0,
        };
        let halves = [Rect::new(0, 0, 960, 1080), Rect::new(960, 0, 960, 1080)];
        let (screen, windows) = scaled_windows(Some(monitor), &halves).unwrap();
        assert_eq!((screen.width, screen.height), (240.0, 135.0));
        assert_eq!(windows.len(), 2);
        assert_eq!((windows[0].x, windows[0].width), (0.0, 120.0));
        assert_eq!((windows[1].x, windows[1].width), (120.0, 120.0));
        assert!(scaled_windows(None, &halves).is_none());
    }
}
