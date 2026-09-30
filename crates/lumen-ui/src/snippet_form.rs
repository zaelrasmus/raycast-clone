//! The form that creates or edits a snippet (F11, slice 6).
//!
//! Raycast's two columns: the snippet on the left in the Markdown editor
//! ([`TextArea`]), with a line under it about placeholders; its name, keyword
//! and tags on the right. It writes through
//! `lumen_core::snippets::store`, and tells its host how that went.
//!
//! # Keys
//!
//! Its own key context, like the quicklink form's. `Enter` saves from the
//! fields on the right and makes a new line in the editor, whose context is
//! deeper; `Ctrl+Enter` saves from anywhere. `Tab` moves between the four,
//! and `Escape` leaves without saving (I-14).
//!
//! # Formats
//!
//! A new snippet is Markdown. An existing one keeps what it was: a snippet
//! written as plain text opens without styling or toolbar and is saved as
//! plain text, since a command with a `*` in it is not emphasis.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{App, Context, Entity, FocusHandle, Focusable, SharedString, Window, div, px, relative};
use lumen_core::layout;
use lumen_core::snippets::{self, Format, Snippet, store};
use lumen_core::theme::Colors;

use crate::quicklink_form::{CancelForm, NextField, PreviousField, SubmitForm};
use crate::styling::{color, metrics, type_scale};
use crate::text_area::{TextArea, TextAreaEvent};
use crate::text_field::{TextField, TextFieldEvent};

/// The form's key context.
pub const CONTEXT: &str = "SnippetForm";

/// What the form tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetFormEvent {
    /// The snippet was written, under this name.
    Saved(String),
    Cancelled,
}

impl gpui::EventEmitter<SnippetFormEvent> for SnippetForm {}

pub struct SnippetForm {
    path: PathBuf,
    /// The name of the snippet being edited; `None` when creating one.
    replacing: Option<String>,
    /// What the form does not show but must not lose: keywords, dates, the
    /// format, fields this build does not know.
    original: Snippet,
    text: Entity<TextArea>,
    name: Entity<TextField>,
    keyword: Entity<TextField>,
    tags: Entity<TextField>,
    /// Why the last save was refused.
    error: Option<String>,
    saving: bool,
    colors: Colors,
}

impl SnippetForm {
    /// A form for `snippet`, saving over the one called `replacing` when
    /// editing, and as a new snippet otherwise.
    pub fn new(
        path: PathBuf,
        snippet: Snippet,
        replacing: Option<String>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let markdown = replacing.is_none() || snippet.format == Format::Markdown;
        let text = cx.new(|cx| {
            let mut area = TextArea::new(
                snippet.text.clone(),
                "Find your inner Shakespeare\u{2026}",
                colors.clone(),
                cx,
            );
            area.set_markdown(markdown, cx);
            area
        });
        cx.subscribe(&text, |form, _, TextAreaEvent::Changed(_), cx| {
            form.error = None;
            cx.notify();
        })
        .detach();

        let field = |value: String, placeholder: &'static str, cx: &mut Context<Self>| {
            let entity = cx.new(|cx| TextField::new(value, placeholder, colors.clone(), cx));
            // An edit answers the last refusal, so the refusal goes.
            cx.subscribe(&entity, |form, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) {
                    form.error = None;
                    cx.notify();
                }
            })
            .detach();
            entity
        };
        let name = field(snippet.name.clone(), "Snippet name", cx);
        let keyword = field(snippet.keyword.clone().unwrap_or_default(), "Keyword", cx);
        let tags = field(snippet.tags.join(", "), "Tags, separated by commas", cx);

        Self {
            path,
            replacing,
            original: Snippet {
                format: if markdown {
                    Format::Markdown
                } else {
                    snippet.format
                },
                ..snippet
            },
            text,
            name,
            keyword,
            tags,
            error: None,
            saving: false,
            colors,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.replacing.is_some()
    }

    pub fn height(&self) -> f32 {
        layout::snippet_form::HEIGHT
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.text
            .update(cx, |area, cx| area.set_colors(colors.clone(), cx));
        for field in [&self.name, &self.keyword, &self.tags] {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// Put the caret in the snippet's text.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.text.read(cx).focus(window);
    }

    /// What Tab visits, in order.
    fn stops(&self, cx: &App) -> [FocusHandle; 4] {
        [
            self.text.focus_handle(cx),
            self.name.focus_handle(cx),
            self.keyword.focus_handle(cx),
            self.tags.focus_handle(cx),
        ]
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.stops(cx);
        let next = match stops.iter().position(|stop| stop.is_focused(window)) {
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
        cx.emit(SnippetFormEvent::Cancelled);
    }

    /// What the form says, as a snippet.
    fn snippet(&self, cx: &App) -> Snippet {
        Snippet {
            name: self.name.read(cx).text().to_string(),
            text: self.text.read(cx).text().to_string(),
            keyword: Some(self.keyword.read(cx).text().to_string()),
            tags: self
                .tags
                .read(cx)
                .text()
                .split(',')
                .map(str::to_string)
                .collect(),
            ..self.original.clone()
        }
    }

    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Write the snippet, off the UI thread.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;

        let snippet = self.snippet(cx);
        let name = snippet.name.trim().to_string();
        let path = self.path.clone();
        let replacing = self.replacing.clone();

        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    let now = snippets::stamp();
                    match replacing {
                        Some(old) => store::replace(&path, &old, snippet, now),
                        None => store::add(&path, snippet, now),
                    }
                })
                .await;

            this.update(cx, |form, cx| {
                form.saving = false;
                match saved {
                    Ok(()) => {
                        log::info!("saved snippet {name:?}");
                        form.replacing = Some(name.clone());
                        cx.emit(SnippetFormEvent::Saved(name));
                    }
                    Err(error) => {
                        log::warn!("could not save snippet {name:?}: {error:#}");
                        form.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn label(&self, text: &'static str) -> impl IntoElement {
        div()
            .h(px(layout::snippet_form::LABEL_HEIGHT))
            .flex()
            .items_center()
            .text_size(type_scale::subtitle())
            .text_color(color(self.colors.text_muted))
            .child(text)
    }

    /// What is under the editor: what placeholders are, or that this
    /// snippet is plain text.
    fn hint(&self) -> impl IntoElement {
        let colors = &self.colors;
        let hint = div()
            .h(px(layout::snippet_form::HINT_HEIGHT))
            .pt(px(layout::text_area::TOOL_GAP))
            .text_size(type_scale::footer())
            .text_color(color(colors.text_muted));
        if self.original.format != Format::Markdown {
            return hint.child("Plain text: pasted exactly as written.");
        }
        hint.child(
            div()
                .flex()
                .flex_wrap()
                .child("Include\u{a0}")
                .child(
                    div()
                        .text_color(color(colors.accent))
                        .child("{Dynamic Placeholders}"),
                )
                .child("\u{a0}for context like the copied text or the current date"),
        )
    }
}

impl Render for SnippetForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        let group = |label: &'static str, field: &Entity<TextField>| {
            div()
                .flex()
                .flex_col()
                .child(self.label(label))
                .child(field.clone())
        };

        let left = div()
            .w(relative(layout::snippet_form::EDITOR_SHARE))
            .flex_none()
            .flex()
            .flex_col()
            .child(self.label("Snippet"))
            .child(
                div()
                    .h(px(layout::snippet_form::EDITOR_HEIGHT))
                    .child(self.text.clone()),
            )
            .child(self.hint());

        let right = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(layout::snippet_form::GROUP_GAP))
            .child(group("Name", &self.name))
            .child(group("Keyword", &self.keyword))
            .child(group("Tags", &self.tags))
            .child(
                div()
                    .h(px(layout::form::MESSAGE_HEIGHT))
                    .flex()
                    .items_center()
                    .text_size(type_scale::footer())
                    .text_color(color(colors.text_error))
                    // A refusal can name a path longer than the column; the
                    // whole of it is in the log.
                    .child(
                        div()
                            .truncate()
                            .child(SharedString::from(self.error.clone().unwrap_or_default())),
                    ),
            );

        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .h(px(self.height()))
            .w_full()
            .px(metrics::gutter())
            .py(px(layout::snippet_form::PADDING_Y))
            .flex()
            .gap(px(layout::snippet_form::COLUMN_GAP))
            .child(left)
            .child(right)
    }
}
