//! The form that adds or edits an emoji's custom keywords (F18, task 7).
//!
//! One labelled field, the quicklink form's row: the keywords, separated by
//! commas. It does not write anything itself -- what was typed goes back to
//! its host, and from there to the emoji session, which parses and keeps it
//! (`lumen_core::emoji::store::parse_keywords`). The launcher shows it over
//! the grid it was opened from, and goes back to that grid either way.
//!
//! # Keys
//!
//! Its own key context: `Enter` saves and `Escape` leaves without saving
//! (I-14).

use gpui::prelude::*;
use gpui::{App, Context, Entity, SharedString, Window, div, px};
use lumen_core::layout;
use lumen_core::theme::Colors;

use crate::quicklink_form::{CancelForm, SubmitForm};
use crate::styling::{color, metrics, type_scale};
use crate::text_field::TextField;

/// The form's key context.
pub const CONTEXT: &str = "EmojiKeywordsForm";

/// What the form tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmojiFormEvent {
    /// Save this, as typed.
    Saved(String),
    Cancelled,
}

impl gpui::EventEmitter<EmojiFormEvent> for EmojiForm {}

pub struct EmojiForm {
    keywords: Entity<TextField>,
    colors: Colors,
}

impl EmojiForm {
    /// A form with `text`, the emoji's keywords as the session joins them.
    pub fn new(text: String, colors: Colors, cx: &mut Context<Self>) -> Self {
        let keywords =
            cx.new(|cx| TextField::new(text, "Keywords, separated by commas", colors.clone(), cx));
        Self { keywords, colors }
    }

    /// One row and the line under it: the quicklink form's measures.
    pub fn height(&self) -> f32 {
        layout::form::PADDING_Y * 2.0 + layout::form::ROW_HEIGHT + layout::form::MESSAGE_HEIGHT
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.keywords
            .update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        self.colors = colors;
        cx.notify();
    }

    /// Put the caret in the field.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.keywords.read(cx).focus(window);
    }

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.keywords.read(cx).text().to_string();
        cx.emit(EmojiFormEvent::Saved(text));
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EmojiFormEvent::Cancelled);
    }
}

impl Render for EmojiForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .h(px(self.height()))
            .w_full()
            .px(metrics::gutter())
            .py(px(layout::form::PADDING_Y))
            .flex()
            .flex_col()
            .child(
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
                            .child("Keywords"),
                    )
                    .child(div().flex_1().child(self.keywords.clone())),
            )
            .child(
                div()
                    .h(px(layout::form::MESSAGE_HEIGHT))
                    .pl(px(layout::form::LABEL_WIDTH))
                    .flex()
                    .items_center()
                    .text_size(type_scale::footer())
                    .text_color(color(colors.text_muted))
                    .child(SharedString::from(
                        "Search finds the emoji by any of these. Empty removes them.",
                    )),
            )
    }
}
