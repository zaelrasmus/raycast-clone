//! The form that creates or edits a quicklink (F6).
//!
//! One entity, two hosts: the launcher shows it in place of the list when
//! "Create Quicklink" is chosen, and the settings window shows it beside its
//! list of links. It writes through `lumen_core::providers::quicklinks::store`
//! and tells its host how that went; the watcher on `quicklinks.json` does the
//! rest, exactly as after a hand edit.
//!
//! # Keys
//!
//! Its own key context, inside whichever host it is in. GPUI prefers the
//! deeper context, so here Tab means the next field rather than the launcher's
//! "type an argument", Enter saves rather than opening the selected row, and
//! Escape leaves the form rather than dismissing the launcher -- all declared
//! in the keymap (I-14).

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{App, Context, Entity, Focusable, SharedString, Window, actions, div, px};
use lumen_core::layout;
use lumen_core::providers::quicklinks::{Entry, store};
use lumen_core::theme::Colors;

use crate::styling::{color, metrics, type_scale};
use crate::text_field::{TextField, TextFieldEvent};

actions!(
    lumen_form,
    [
        /// Move to the next field.
        NextField,
        /// Move to the previous field.
        PreviousField,
        /// Save the quicklink.
        SubmitForm,
        /// Leave without saving.
        CancelForm,
    ]
);

/// The form's key context.
pub const CONTEXT: &str = "QuicklinkForm";

/// What the form tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormEvent {
    /// The link was written, under this name.
    Saved(String),
    Cancelled,
}

impl gpui::EventEmitter<FormEvent> for QuicklinkForm {}

/// The fields, in the order Tab visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Link,
    Alias,
    Browser,
}

impl Field {
    const ALL: [Field; layout::form::FIELDS] =
        [Field::Name, Field::Link, Field::Alias, Field::Browser];

    fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Link => "Link",
            Field::Alias => "Alias",
            Field::Browser => "Open with",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Field::Name => "YouTube channel",
            Field::Link => "https://www.youtube.com/@veritasium",
            Field::Alias => "Optional: a word to type before a search, like g",
            Field::Browser => "Optional: the default browser otherwise",
        }
    }

    /// What the line under the form says while this field has focus.
    fn hint(self) -> &'static str {
        match self {
            Field::Name => "What the link is called in the list.",
            Field::Link => "Put {query} where a search goes, or {clipboard} for what you copied.",
            Field::Alias => "Typing the alias and a space searches with the link: g lofi beats.",
            Field::Browser => "",
        }
    }
}

pub struct QuicklinkForm {
    path: PathBuf,
    /// The name of the link being edited; `None` when creating one.
    replacing: Option<String>,
    /// What the form does not show but must not lose when editing: the
    /// fallback flag, keywords, and fields this build does not know.
    original: Entry,
    fields: [Entity<TextField>; layout::form::FIELDS],
    /// The installed browsers' names, for the hint under "Open with".
    browsers: Vec<String>,
    /// Why the last save was refused.
    error: Option<String>,
    saving: bool,
    colors: Colors,
}

impl QuicklinkForm {
    /// A form for a new link, or for editing `existing`.
    pub fn new(
        path: PathBuf,
        existing: Option<Entry>,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Self {
        let original = existing.clone().unwrap_or_default();
        let fields = Field::ALL.map(|field| {
            let text = match field {
                Field::Name => original.name.clone(),
                Field::Link => original.link.clone(),
                Field::Alias => original.alias.clone().unwrap_or_default(),
                Field::Browser => original.open_with.clone().unwrap_or_default(),
            };
            let entity = cx.new(|cx| TextField::new(text, field.placeholder(), colors.clone(), cx));
            // An edit answers the last refusal, so the refusal goes.
            cx.subscribe(&entity, |form, _, event, cx| {
                if matches!(event, TextFieldEvent::Changed(_)) && form.error.take().is_some() {
                    cx.notify();
                }
            })
            .detach();
            entity
        });

        Self {
            path,
            replacing: existing.map(|entry| entry.name),
            original,
            fields,
            browsers: Vec::new(),
            error: None,
            saving: false,
            colors,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.replacing.is_some()
    }

    /// Name the browsers the hint can offer.
    pub fn set_browsers(&mut self, browsers: Vec<String>, cx: &mut Context<Self>) {
        self.browsers = browsers;
        cx.notify();
    }

    /// Offer `link` in the link field, if the user has not typed one.
    ///
    /// For the clipboard: a URL copied a moment ago is usually the link being
    /// saved, which is what Raycast does too.
    pub fn suggest_link(&mut self, link: String, cx: &mut Context<Self>) {
        let field = &self.fields[1];
        if field.read(cx).text().is_empty() {
            field.update(cx, |field, cx| field.set_text(link, cx));
        }
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_colors(colors.clone(), cx));
        }
        self.colors = colors;
        cx.notify();
    }

    /// Put the caret in the first field.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.fields[0].read(cx).focus(window);
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.fields
            .iter()
            .position(|field| field.focus_handle(cx).is_focused(window))
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.fields.len() as isize;
        let next = match self.focused(window, cx) {
            Some(index) => (index as isize + delta).rem_euclid(count) as usize,
            None => 0,
        };
        self.fields[next].read(cx).focus(window);
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, window, cx);
    }

    fn cancel(&mut self, _: &CancelForm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FormEvent::Cancelled);
    }

    /// What the fields say, as an entry.
    fn entry(&self, cx: &App) -> Entry {
        let text = |field: Field| {
            self.fields[Field::ALL.iter().position(|f| *f == field).unwrap_or(0)]
                .read(cx)
                .text()
                .to_string()
        };
        Entry {
            name: text(Field::Name),
            link: text(Field::Link),
            alias: Some(text(Field::Alias)),
            open_with: Some(text(Field::Browser)),
            ..self.original.clone()
        }
    }

    /// Write the link. Off the UI thread: it is a read and a write of the
    /// file, and the frame budget is not the place for disk I/O.
    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;

        let entry = self.entry(cx);
        let name = entry.name.trim().to_string();
        let path = self.path.clone();
        let replacing = self.replacing.clone();

        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    match replacing {
                        Some(old) => store::replace(&path, &old, entry),
                        None => store::add(&path, entry),
                    }
                })
                .await;

            this.update(cx, |form, cx| {
                form.saving = false;
                match saved {
                    Ok(()) => {
                        log::info!("saved quicklink {name:?}");
                        // Editing it again from here edits the saved name.
                        form.replacing = Some(name.clone());
                        cx.emit(FormEvent::Saved(name));
                    }
                    Err(error) => {
                        log::warn!("could not save quicklink {name:?}: {error:#}");
                        form.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The line under the fields.
    fn message(&self, window: &Window, cx: &App) -> (String, bool) {
        if let Some(error) = &self.error {
            return (error.clone(), true);
        }
        let field = self
            .focused(window, cx)
            .map(|index| Field::ALL[index])
            .unwrap_or(Field::Name);
        let hint = match field {
            Field::Browser if self.browsers.is_empty() => {
                "A browser's name, or part of it.".to_string()
            }
            Field::Browser => format!("Installed: {}.", self.browsers.join(", ")),
            other => other.hint().to_string(),
        };
        (hint, false)
    }
}

impl Render for QuicklinkForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.colors;
        let (message, is_error) = self.message(window, cx);

        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .h(px(layout::form::HEIGHT))
            .w_full()
            .px(metrics::gutter())
            .py(px(layout::form::PADDING_Y))
            .flex()
            .flex_col()
            .children(Field::ALL.iter().zip(&self.fields).map(|(field, entity)| {
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
                            .child(SharedString::from(field.label())),
                    )
                    .child(div().flex_1().child(entity.clone()))
            }))
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
                    // A refusal can name a path longer than the window; the
                    // whole of it is in the log.
                    .child(div().truncate().child(message)),
            )
    }
}
