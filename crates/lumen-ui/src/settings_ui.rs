//! The settings window's parts (F15), after Raycast's.
//!
//! A page is a column of cards under section titles; a card is rows; a row
//! is a title, perhaps a description under it, and a control at its right
//! edge. Every page composes these and draws none of the geometry itself, so
//! no page can drift from the others. The geometry is the project's
//! (`layout::settings`): settings follows the palette, never a style.

use gpui::prelude::*;
use gpui::{AnyElement, App, Div, ElementId, Hsla, SharedString, Stateful, Window, div, px};
use lumen_core::layout::settings as metrics;
use lumen_core::theme::Colors;

use crate::styling::color;

/// A page's content: a scrolling column, padded from the window's edges.
pub fn page(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .p(px(metrics::CONTENT_PADDING))
        .flex()
        .flex_col()
        .gap(px(metrics::CARD_GAP))
}

/// A section's title, above its card.
pub fn section_title(text: impl Into<SharedString>, colors: &Colors) -> Div {
    div()
        .h(px(metrics::SECTION_TITLE_HEIGHT))
        .flex_none()
        .px(px(metrics::CARD_PADDING_X))
        .flex()
        .items_end()
        .text_size(px(metrics::text::SECTION_TITLE))
        .text_color(color(colors.text_primary))
        .child(text.into())
}

/// Rows in a rounded card, a rule between each.
pub fn card(rows: impl IntoIterator<Item = AnyElement>, colors: &Colors) -> Div {
    let rule = color(colors.border_subtle);
    div()
        .flex_none()
        .flex()
        .flex_col()
        .rounded(px(metrics::CARD_RADIUS))
        .bg(color(colors.surface_elevated))
        .children(rows.into_iter().enumerate().map(move |(index, row)| {
            div()
                .when(index > 0, |cell| cell.border_t_1().border_color(rule))
                .child(row)
        }))
}

/// A row: its title and description on the left, `control` on the right.
///
/// Two heights, one with a description and one without, and nothing in
/// between: a row is never sized by its text (I-10's reasoning, in a form).
pub fn row(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    colors: &Colors,
) -> AnyElement {
    let height = if description.is_some() {
        metrics::CARD_ROW_TALL_HEIGHT
    } else {
        metrics::CARD_ROW_HEIGHT
    };
    div()
        .h(px(height))
        .px(px(metrics::CARD_PADDING_X))
        .flex()
        .items_center()
        .gap(px(metrics::CARD_ROW_GAP))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(metrics::CARD_ROW_LINE_GAP))
                .child(
                    div()
                        .truncate()
                        .text_size(px(metrics::text::ROW_TITLE))
                        .text_color(color(colors.text_primary))
                        .child(title.into()),
                )
                .when_some(description, |text, description| {
                    text.child(
                        div()
                            .truncate()
                            .text_size(px(metrics::text::ROW_DESCRIPTION))
                            .text_color(color(colors.text_muted))
                            .child(description),
                    )
                }),
        )
        .child(div().flex_none().flex().items_center().child(control))
        .into_any_element()
}

/// A control at a row's right edge that would otherwise run the row's
/// whole width -- a text field, a full dropdown.
pub fn control(child: impl IntoElement) -> Div {
    div().w(px(metrics::CONTROL_WIDTH)).child(child)
}

/// A toggle's track and knob, on or off; `focused` rings it in the accent.
///
/// Drawing only. The host makes it focusable and clickable, and binds Space
/// through the keymap (I-14), as the form checkbox it replaces did.
pub fn toggle(on: bool, focused: bool, colors: &Colors) -> Div {
    let track: Hsla = color(if on {
        colors.accent
    } else {
        colors.border_strong
    });
    div()
        .w(px(metrics::TOGGLE_WIDTH))
        .h(px(metrics::TOGGLE_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .when(on, |track| track.justify_end())
        .px(px(metrics::TOGGLE_INSET))
        .rounded_full()
        .bg(track)
        .border_1()
        .border_color(if focused {
            color(colors.text_primary)
        } else {
            track
        })
        .child(
            div()
                .size(px(metrics::TOGGLE_KNOB))
                .rounded_full()
                .bg(color(if on {
                    colors.text_on_accent
                } else {
                    colors.text_primary
                })),
        )
}

/// A row whose control is a toggle, with the toggle wired: focus, a mouse
/// click that flips it, and the keymap's Space through `context`.
#[allow(clippy::too_many_arguments)]
pub fn toggle_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    on: bool,
    focus: &gpui::FocusHandle,
    context: &'static str,
    window: &Window,
    colors: &Colors,
    on_flip: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let focused = focus.is_focused(window);
    let handle = focus.clone();
    row(
        title,
        description,
        div()
            .id(id)
            .key_context(context)
            .track_focus(focus)
            .cursor_pointer()
            .on_click(move |event: &gpui::ClickEvent, window, cx| {
                // The mouse only: gpui turns Space on a focused element into a
                // click when the key comes up, which would flip it twice.
                if event.is_keyboard() {
                    return;
                }
                window.focus(&handle);
                on_flip(window, cx);
            })
            .child(toggle(on, focused, colors)),
        colors,
    )
}
