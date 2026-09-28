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
//!
//! # The fallback flag
//!
//! Only settings offers it ([`QuicklinkForm::offer_fallback`]). A new link in
//! the launcher needs four fields; whether it should also be offered for every
//! search that finds nothing is a decision made later, where all the links
//! are side by side. It is a row of the form rather than a control beside it
//! so that one Enter saves everything: a separate switch writing on its own
//! would be undone by the next save of fields read before it was flipped.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{App, Context, Entity, FocusHandle, Focusable, SharedString, Window, actions, div, px};
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
        /// Flip the checkbox that has focus.
        ToggleFallback,
    ]
);

/// The form's key context.
pub const CONTEXT: &str = "QuicklinkForm";

/// The fallback checkbox's key context, inside the form's.
pub const TOGGLE_CONTEXT: &str = "FormToggle";

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
    /// The fallback checkbox, when the host offers it.
    fallback: Option<Fallback>,
    /// Whether anything has changed since the form was filled or saved.
    edited: bool,
    /// Why the last save was refused.
    error: Option<String>,
    saving: bool,
    colors: Colors,
}

struct Fallback {
    on: bool,
    focus: FocusHandle,
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
                if matches!(event, TextFieldEvent::Changed(_)) {
                    form.edited = true;
                    form.error = None;
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
            fallback: None,
            edited: false,
            error: None,
            saving: false,
            colors,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.replacing.is_some()
    }

    /// The name of the link on disk this form edits, if it edits one.
    pub fn editing_name(&self) -> Option<&str> {
        self.replacing.as_deref()
    }

    /// Show the fallback checkbox, after the other fields.
    pub fn offer_fallback(&mut self, cx: &mut Context<Self>) {
        self.fallback = Some(Fallback {
            on: self.original.fallback,
            focus: cx.focus_handle(),
        });
        cx.notify();
    }

    /// The form's height, which its host reserves for it.
    pub fn height(&self) -> f32 {
        match self.fallback {
            Some(_) => layout::form::HEIGHT_WITH_FALLBACK,
            None => layout::form::HEIGHT,
        }
    }

    /// Whether the user has changed anything since the form was filled in or
    /// last saved. A host refreshing the form from disk must not throw that
    /// away.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// The link as it was when the form was filled in.
    pub fn original(&self) -> &Entry {
        &self.original
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

    /// What Tab visits, in order: the fields, then the checkbox.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        self.fields
            .iter()
            .map(|field| field.focus_handle(cx))
            .chain(self.fallback.iter().map(|fallback| fallback.focus.clone()))
            .collect()
    }

    /// Which stop has focus, if any, by its place in the Tab order.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.stops(cx)
            .iter()
            .position(|stop| stop.is_focused(window))
    }

    /// Focus a stop by its place in the Tab order, or the last if there are
    /// fewer.
    pub fn focus_stop(&self, index: usize, window: &mut Window, cx: &App) {
        let stops = self.stops(cx);
        if let Some(stop) = stops.get(index.min(stops.len().saturating_sub(1))) {
            window.focus(stop);
        }
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.stops(cx).len() as isize;
        let next = match self.focused(window, cx) {
            Some(index) => (index as isize + delta).rem_euclid(count) as usize,
            None => 0,
        };
        self.focus_stop(next, window, cx);
        cx.notify();
    }

    fn toggle_fallback(&mut self, _: &ToggleFallback, _: &mut Window, cx: &mut Context<Self>) {
        self.flip_fallback(cx);
    }

    fn flip_fallback(&mut self, cx: &mut Context<Self>) {
        if let Some(fallback) = &mut self.fallback {
            fallback.on = !fallback.on;
            self.edited = true;
            self.error = None;
            cx.notify();
        }
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
            fallback: self
                .fallback
                .as_ref()
                .map_or(self.original.fallback, |fallback| fallback.on),
            ..self.original.clone()
        }
    }

    /// Write the link. Off the UI thread: it is a read and a write of the
    /// file, and the frame budget is not the place for disk I/O.
    fn submit(&mut self, _: &SubmitForm, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    /// Save, as Enter does. For a host with its own save key.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;

        let entry = self.entry(cx);
        let name = entry.name.trim().to_string();
        let path = self.path.clone();
        let replacing = self.replacing.clone();
        let sent = entry.clone();

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
                        // Still edited if the user typed while it was saving:
                        // what is on screen is not what was written.
                        form.edited = form.entry(cx) != sent;
                        form.original = sent;
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
        let field = match self.focused(window, cx) {
            // Past the fields: the checkbox.
            Some(index) if index >= Field::ALL.len() => {
                let hint = "Offered at the foot of the list when a search finds nothing better.";
                return (hint.to_string(), false);
            }
            Some(index) => Field::ALL[index],
            None => Field::Name,
        };
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
            .on_action(cx.listener(Self::toggle_fallback))
            .h(px(self.height()))
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
            .when_some(self.render_fallback(window, cx), |form, row| {
                form.child(row)
            })
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

impl QuicklinkForm {
    /// The fallback row, when the host asked for it.
    fn render_fallback(&self, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let fallback = self.fallback.as_ref()?;
        let colors = &self.colors;
        let focused = fallback.focus.is_focused(window);

        let checkbox = div()
            .size(px(layout::form::CHECKBOX_SIZE))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(layout::form::CHECKBOX_RADIUS))
            .border_1()
            .border_color(color(if focused || fallback.on {
                colors.accent
            } else {
                colors.border_strong
            }))
            .when(fallback.on, |checkbox| {
                checkbox
                    .bg(color(colors.accent))
                    .text_color(color(colors.text_on_accent))
                    .text_size(type_scale::footer())
                    .child("\u{2713}")
            });

        let row = div()
            .h(px(layout::form::ROW_HEIGHT))
            .flex()
            .items_center()
            .child(
                div()
                    .w(px(layout::form::LABEL_WIDTH))
                    .flex_none()
                    .text_size(type_scale::subtitle())
                    .text_color(color(colors.text_muted))
                    .child("Fallback"),
            )
            .child(
                div()
                    .id("fallback")
                    .key_context(TOGGLE_CONTEXT)
                    .track_focus(&fallback.focus)
                    .flex()
                    .items_center()
                    .gap(px(layout::form::CHECKBOX_GAP))
                    .cursor_pointer()
                    .on_click(cx.listener(|form, event: &gpui::ClickEvent, window, cx| {
                        // The mouse only. GPUI also turns Enter and Space on
                        // a focused element into a click when the key comes
                        // up, which would flip the box back after Space's
                        // action and flip it again after Enter had saved.
                        if event.is_keyboard() {
                            return;
                        }
                        if let Some(fallback) = &form.fallback {
                            window.focus(&fallback.focus);
                        }
                        form.flip_fallback(cx);
                    }))
                    .child(checkbox)
                    .child(
                        div()
                            .text_size(type_scale::subtitle())
                            .text_color(color(if focused {
                                colors.text_primary
                            } else {
                                colors.text_muted
                            }))
                            .child("Offer it for searches that find nothing better"),
                    ),
            );

        Some(row.into_any_element())
    }
}
