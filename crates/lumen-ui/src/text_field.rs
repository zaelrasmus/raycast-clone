//! A small single-line text field.
//!
//! The launcher's search box grows its own editing logic because it is the one
//! field in the application and it is bound up with ranking. The settings window
//! has dozens of fields, so they need a component.
//!
//! # The same known gap as the search box
//!
//! This is a plain `String` with a drawn caret, not a GPUI `EntityInputHandler`,
//! so there is no IME, no mouse selection and no composition. The trade is
//! recorded once in `lib.rs` and applies here for the same reason: a correct
//! input handler needs a custom `Element` that shapes its own text and reports
//! selection geometry back to the system, and the ASCII shortcut descriptions
//! this field exists to hold do not need any of it.
//!
//! Editing *commands* are GPUI actions, bound from the keymap like everything
//! else (I-14). Only the insertion of printable characters reads a keystroke
//! directly, because "the letter k" is not a command and cannot be one.

use gpui::prelude::*;
use gpui::{
    App, Context, FocusHandle, Focusable, KeyDownEvent, SharedString, Styled, Window, div, px,
};
use lumen_core::layout;
use lumen_core::theme::Colors;

use crate::launcher::{DeleteBackward, DeleteWordBackward};
use crate::styling::color;

/// A single-line editable string.
pub struct TextField {
    focus_handle: FocusHandle,
    text: String,
    /// Byte offset of the caret within `text`.
    cursor: usize,
    placeholder: SharedString,
    colors: Colors,
}

/// What the field tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFieldEvent {
    /// The text changed. Carries the new value so the owner does not have to
    /// reach back into the entity to read it.
    Changed(String),
    /// `Enter` was pressed in this field.
    Submitted,
}

impl gpui::EventEmitter<TextFieldEvent> for TextField {}

impl TextField {
    pub fn new(
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        colors: Colors,
        cx: &mut App,
    ) -> Self {
        let text = text.into();
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            cursor: text.len(),
            text,
            placeholder: placeholder.into(),
            colors,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replace the contents without emitting a change.
    ///
    /// Used when the owner reloads a different record into the same field. It
    /// does not emit, because nothing the *user* did caused it, and treating it
    /// as an edit would mark a freshly loaded sheet as modified.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        self.cursor = self.text.len();
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Changed(self.text.clone()));
        cx.notify();
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.changed(cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;

        if keystroke.key == "enter" {
            cx.emit(TextFieldEvent::Submitted);
            return;
        }

        let Some(character) = keystroke.key_char.as_ref() else {
            return;
        };
        // A chord is a command, and commands are actions. Without this, Ctrl+S
        // would type an "s" into whichever field happens to hold focus.
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        if character.chars().any(char::is_control) {
            return;
        }

        self.insert(character, cx);
    }

    fn delete_backward(&mut self, _: &DeleteBackward, _: &mut Window, cx: &mut Context<Self>) {
        let Some(previous) = previous_char_boundary(&self.text, self.cursor) else {
            return;
        };
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.changed(cx);
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = word_start_before(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.changed(cx);
    }

    /// Re-colour the field when the theme changes.
    ///
    /// The field holds its own palette rather than being handed one at render
    /// time, because it is an `Entity` its owner embeds directly: GPUI calls
    /// [`Render`] on it, not the owner, so there is no render call to pass
    /// colours through.
    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        let focused = self.focus_handle.is_focused(window);
        let (before, after) = self.text.split_at(self.cursor);

        div()
            .key_context("TextField")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::delete_backward))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_key_down(cx.listener(Self::on_key_down))
            .h(px(layout::settings::FIELD_HEIGHT))
            .px(px(layout::settings::FIELD_PADDING))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(layout::settings::FIELD_RADIUS))
            .border_1()
            .border_color(color(if focused {
                colors.accent
            } else {
                colors.border_subtle
            }))
            .bg(color(colors.surface_elevated))
            .text_size(px(layout::settings::text::FIELD))
            .when(self.text.is_empty() && !focused, |field| {
                field
                    .text_color(color(colors.text_placeholder))
                    .child(self.placeholder.clone())
            })
            .when(!self.text.is_empty() || focused, |field| {
                field
                    .text_color(color(colors.text_primary))
                    .child(before.to_string())
                    .child(
                        div()
                            .w(px(1.0))
                            .h(px(layout::settings::text::FIELD))
                            .bg(color(if focused {
                                colors.accent
                            } else {
                                gpui_transparent()
                            })),
                    )
                    .child(after.to_string())
            })
    }
}

/// A colour that paints nothing, for the caret of an unfocused field.
///
/// Reserved rather than omitted so that focusing a field does not shift its text
/// by a pixel (the same reasoning as I-10, one field at a time).
fn gpui_transparent() -> lumen_core::theme::Color {
    lumen_core::theme::Color::rgba(0, 0, 0, 0)
}

/// The byte offset of the character before `cursor`.
pub(crate) fn previous_char_boundary(text: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    text[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

/// The byte offset where the word before `cursor` starts.
pub(crate) fn word_start_before(text: &str, cursor: usize) -> usize {
    let trimmed = text[..cursor].trim_end();
    trimmed
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspace_removes_a_whole_character() {
        let text = "café";
        let start = previous_char_boundary(text, text.len()).unwrap();
        assert_eq!(&text[..start], "caf");
    }

    #[test]
    fn word_delete_skips_trailing_whitespace() {
        assert_eq!(word_start_before("ctrl shift ", 11), 5);
    }

    #[test]
    fn word_delete_on_a_single_word_clears_it() {
        assert_eq!(word_start_before("ctrl", 4), 0);
    }
}
