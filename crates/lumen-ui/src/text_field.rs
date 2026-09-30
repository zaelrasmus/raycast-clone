//! A single-line text field with a real input handler.
//!
//! Used twice: as the launcher's search box and as every field in the
//! settings window. One component, because the two used to be separate
//! hand-rolled strings with a drawn caret, and both had the same three gaps --
//! no IME, no caret movement and no selection.
//!
//! # Why it has its own element
//!
//! GPUI delivers text through [`EntityInputHandler`], not through key events.
//! Implementing it is what makes dead keys, AltGr characters and input-method
//! composition work: the operating system translates the keystroke, and the
//! field only ever sees the finished text. The input method in turn needs to
//! ask where a given character is on screen, to put its candidate window
//! beside it, so the field has to shape its own text and remember where it
//! drew it. That is [`TextLine`].
//!
//! # Keyboard handling (I-14)
//!
//! Every editing command -- moving, selecting, deleting, the clipboard -- is a
//! GPUI action bound from the keymap. Typed text is not a command and arrives
//! through the input handler, so no keystroke is inspected by hand anywhere in
//! this file.
//!
//! The editing arithmetic itself lives in [`crate::edit`], where it is tested
//! without a window.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine,
    SharedString, Style, Styled, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, point, px, relative, size,
};
use lumen_core::layout;
use lumen_core::theme::Colors;

use crate::edit::EditBuffer;
use crate::styling::color;

actions!(
    lumen,
    [
        /// Move the caret one character left, or collapse the selection.
        MoveLeft,
        /// Move the caret one character right, or collapse the selection.
        MoveRight,
        /// Extend the selection one character left.
        SelectLeft,
        /// Extend the selection one character right.
        SelectRight,
        /// Move the caret to the start of the previous word.
        MoveWordLeft,
        /// Move the caret to the end of the next word.
        MoveWordRight,
        /// Extend the selection to the start of the previous word.
        SelectWordLeft,
        /// Extend the selection to the end of the next word.
        SelectWordRight,
        /// Move the caret to the start of the field.
        MoveToStart,
        /// Move the caret to the end of the field.
        MoveToEnd,
        /// Extend the selection to the start of the field.
        SelectToStart,
        /// Extend the selection to the end of the field.
        SelectToEnd,
        /// Select everything in the field.
        SelectAll,
        /// Delete the selection, or the character before the caret.
        DeleteBackward,
        /// Delete the selection, or the character after the caret.
        DeleteForward,
        /// Delete the selection, or the word before the caret.
        DeleteWordBackward,
        /// Delete the selection, or the word after the caret.
        DeleteWordForward,
        /// Copy the selection.
        Copy,
        /// Copy the selection and delete it.
        Cut,
        /// Insert the clipboard's text in place of the selection.
        Paste,
    ]
);

/// Which of the two fields this is. They edit identically and differ in
/// dress and in one binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A boxed field in the settings window. `Home` and `End` move the caret.
    Form,
    /// The launcher's query. Unboxed -- the launcher draws the chrome around
    /// it -- and `Home`, `End` and `Enter` are left to the list.
    Search,
}

impl Kind {
    /// The key context, which is what lets the keymap give `Home` a different
    /// meaning in each.
    fn context(self) -> &'static str {
        match self {
            Kind::Form => "TextField",
            Kind::Search => "SearchField",
        }
    }
}

/// A single-line editable string.
pub struct TextField {
    focus_handle: FocusHandle,
    buffer: EditBuffer,
    placeholder: SharedString,
    colors: Colors,
    kind: Kind,

    /// What was drawn last frame, and where. The input method and the mouse
    /// both ask "which character is at this point" and "where is this
    /// character", and both questions are about the frame on screen.
    drawn: Option<Drawn>,
    /// How far the text is scrolled left, when it is wider than the field.
    scroll: Pixels,
    /// A mouse button went down in the field and has not come up yet.
    dragging: bool,
}

/// The last frame's layout.
struct Drawn {
    line: ShapedLine,
    /// Where the text's first character starts: the element's origin, shifted
    /// by the scroll.
    origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
}

/// What the field tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFieldEvent {
    /// The text changed. Carries the new value so the owner does not have to
    /// reach back into the entity to read it.
    Changed(String),
    /// Backspace was pressed with nothing left to delete.
    ///
    /// Reported rather than ignored because an owner may have something to
    /// the left of the field to remove -- the launcher's quicklink pill -- and
    /// reading the key itself would be the hand-written comparison I-14 rules
    /// out.
    DeletedPastStart,
}

impl gpui::EventEmitter<TextFieldEvent> for TextField {}

impl TextField {
    /// A field for the settings window.
    pub fn new(
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        colors: Colors,
        cx: &mut App,
    ) -> Self {
        Self::with_kind(Kind::Form, text.into(), placeholder.into(), colors, cx)
    }

    /// The launcher's search field.
    pub fn search(placeholder: impl Into<SharedString>, colors: Colors, cx: &mut App) -> Self {
        Self::with_kind(Kind::Search, String::new(), placeholder.into(), colors, cx)
    }

    fn with_kind(
        kind: Kind,
        text: String,
        placeholder: SharedString,
        colors: Colors,
        cx: &mut App,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            buffer: EditBuffer::new(text),
            placeholder,
            colors,
            kind,
            drawn: None,
            scroll: px(0.0),
            dragging: false,
        }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// Replace the contents without emitting a change.
    ///
    /// Used when the owner reloads a different record into the same field, or
    /// clears the query itself. It does not emit, because nothing the *user*
    /// did caused it, and treating it as an edit would mark a freshly loaded
    /// sheet as modified.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.set_text(text);
        self.scroll = px(0.0);
        cx.notify();
    }

    /// Change what the empty field says.
    ///
    /// Quiet on purpose: the launcher calls this while it renders, and the
    /// field is drawn later in that same frame.
    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
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

    /// Apply an edit, and tell the owner if it changed the text.
    ///
    /// Moving the caret is not a change. The launcher re-ranks on every
    /// change, and re-ranking because an arrow key was pressed would reset the
    /// selected row while the user is only moving through their query.
    fn edit(&mut self, cx: &mut Context<Self>, apply: impl FnOnce(&mut EditBuffer)) {
        let before = self.buffer.text().to_owned();
        apply(&mut self.buffer);

        if self.buffer.text() != before {
            cx.emit(TextFieldEvent::Changed(self.buffer.text().to_owned()));
        }
        cx.notify();
    }

    /// Move or select without editing.
    fn navigate(&mut self, cx: &mut Context<Self>, apply: impl FnOnce(&mut EditBuffer)) {
        apply(&mut self.buffer);
        cx.notify();
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_left);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_right);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_left);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_right);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_word_left);
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_word_right);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_word_left);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_word_right);
    }

    fn move_to_start(&mut self, _: &MoveToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_to_start);
    }

    fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::move_to_end);
    }

    fn select_to_start(&mut self, _: &SelectToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_to_start);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_to_end);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx, EditBuffer::select_all);
    }

    fn delete_backward(&mut self, _: &DeleteBackward, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.is_empty() {
            cx.emit(TextFieldEvent::DeletedPastStart);
            return;
        }
        self.edit(cx, |buffer| {
            buffer.delete_backward();
        });
    }

    fn delete_forward(&mut self, _: &DeleteForward, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |buffer| {
            buffer.delete_forward();
        });
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |buffer| {
            buffer.delete_word_backward();
        });
    }

    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |buffer| {
            buffer.delete_word_forward();
        });
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.has_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.buffer.selected_text().to_owned(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.has_selection() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.buffer.selected_text().to_owned(),
        ));
        self.edit(cx, |buffer| buffer.replace(None, ""));
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // One line. A pasted newline would silently become part of the query
        // and match nothing, with no visible reason why.
        let text = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
        self.edit(cx, |buffer| buffer.replace(None, &text));
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_at(event.position);

        if event.click_count >= 3 {
            self.buffer.select_all();
        } else if event.click_count == 2 {
            self.buffer.select(self.buffer.word_at(offset));
        } else if event.modifiers.shift {
            self.buffer.select_to(offset);
            self.dragging = true;
        } else {
            self.buffer.move_to(offset);
            self.dragging = true;
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            self.buffer.select_to(self.offset_at(event.position));
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.dragging = false;
    }

    /// The character offset nearest a point on screen.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(drawn) = &self.drawn else {
            return self.buffer.text().len();
        };
        if self.buffer.is_empty() {
            return 0;
        }
        drawn.line.closest_index_for_x(position.x - drawn.origin.x)
    }

    /// Where a character sits horizontally, in window coordinates.
    fn x_of(&self, drawn: &Drawn, offset: usize) -> Pixels {
        if self.buffer.is_empty() {
            return drawn.origin.x;
        }
        drawn.origin.x + drawn.line.x_for_index(offset)
    }
}

/// Convert a UTF-16 offset within `text` to a byte offset.
pub(crate) fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range);
        adjusted.replace(self.buffer.range_to_utf16(&range));
        self.buffer.text().get(range).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selection()),
            reversed: self.buffer.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buffer
            .marked()
            .map(|range| self.buffer.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.unmark();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| self.buffer.range_from_utf16(&range));
        self.edit(cx, |buffer| buffer.replace(range, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| self.buffer.range_from_utf16(&range));
        // Relative to the new text, and in UTF-16 like everything else the
        // input method says.
        let selected =
            selected.map(|range| utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end));
        self.edit(cx, |buffer| buffer.replace_and_mark(range, text, selected));
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let drawn = self.drawn.as_ref()?;
        let range = self.buffer.range_from_utf16(&range);
        Some(Bounds::from_corners(
            point(self.x_of(drawn, range.start), drawn.bounds.top()),
            point(self.x_of(drawn, range.end), drawn.bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let drawn = self.drawn.as_ref()?;
        if !drawn.bounds.contains(&position) {
            return None;
        }
        Some(self.buffer.to_utf16(self.offset_at(position)))
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

        let field = div()
            .key_context(self.kind.context())
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::move_to_start))
            .on_action(cx.listener(Self::move_to_end))
            .on_action(cx.listener(Self::select_to_start))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::delete_backward))
            .on_action(cx.listener(Self::delete_forward))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .flex()
            .items_center()
            .overflow_hidden()
            .child(TextLine { field: cx.entity() });

        match self.kind {
            Kind::Search => field.flex_1().h_full(),
            Kind::Form => field
                .h(px(layout::settings::FIELD_HEIGHT))
                .px(px(layout::settings::FIELD_PADDING))
                .rounded(px(layout::settings::FIELD_RADIUS))
                .border_1()
                .border_color(color(if focused {
                    self.colors.accent
                } else {
                    self.colors.border_subtle
                }))
                .bg(color(self.colors.surface_elevated))
                .text_size(px(layout::settings::text::FIELD)),
        }
    }
}

/// The shaped line of text, its selection and its caret.
///
/// An element rather than a `div` of spans because it must know exactly where
/// each character landed -- the input method asks, and so does the mouse --
/// and because it is where the field registers itself as the window's input
/// handler, which only an element's paint phase can do.
struct TextLine {
    field: gpui::Entity<TextField>,
}

struct Prepared {
    line: ShapedLine,
    origin: Point<Pixels>,
    scroll: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextLine {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let field = self.field.read(cx);
        let buffer = &field.buffer;
        let colors = &field.colors;
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        // The placeholder is drawn in the same place the text will be, so the
        // caret sits at its start and typing replaces it without a jump.
        let (text, text_color) = if buffer.is_empty() {
            (field.placeholder.clone(), color(colors.text_placeholder))
        } else {
            (
                SharedString::from(buffer.text().to_owned()),
                color(colors.text_primary),
            )
        };

        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        // What the input method is still composing is underlined, which is
        // how every system shows "not committed yet".
        let runs = match buffer.marked().filter(|_| !buffer.is_empty()) {
            Some(marked) => [
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.len(),
                    underline: Some(UnderlineStyle {
                        color: Some(text_color),
                        thickness: px(layout::CARET_WIDTH),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>(),
            None => vec![run],
        };

        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);

        let caret_width = px(layout::CARET_WIDTH);
        let x_for = |offset: usize| {
            if buffer.is_empty() {
                px(0.0)
            } else {
                line.x_for_index(offset)
            }
        };

        // Keep the caret in view when the text is wider than the field, and
        // give back the scroll when it no longer needs it.
        let caret_x = x_for(buffer.caret());
        let room = (bounds.size.width - caret_width).max(px(0.0));
        let overflow = (line.width - room).max(px(0.0));
        let mut scroll = field.scroll.min(overflow);
        if caret_x - scroll > room {
            scroll = caret_x - room;
        }
        if caret_x < scroll {
            scroll = caret_x;
        }

        let origin = point(bounds.left() - scroll, bounds.top());
        // The caret and the selection are as tall as the type, centred on the
        // line, rather than the full line height: a caret that reaches the
        // borders of the field looks like a divider.
        let top = bounds.top() + (bounds.size.height - font_size) / 2.0;

        let selection = (buffer.has_selection() && !buffer.is_empty()).then(|| {
            let range = buffer.selection();
            fill(
                Bounds::from_corners(
                    point(origin.x + x_for(range.start), top),
                    point(origin.x + x_for(range.end), top + font_size),
                ),
                color(colors.text_selection),
            )
        });

        let focused = field.focus_handle.is_focused(window);
        let caret = (focused && !buffer.has_selection()).then(|| {
            fill(
                Bounds::new(point(origin.x + caret_x, top), size(caret_width, font_size)),
                color(colors.accent),
            )
        });

        Prepared {
            line,
            origin,
            scroll,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepared: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.field.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );

        if let Some(selection) = prepared.selection.take() {
            window.paint_quad(selection);
        }

        let line_height = window.line_height();
        if let Err(error) = prepared
            .line
            .paint(prepared.origin, line_height, window, cx)
        {
            log::debug!("could not paint a text field: {error:#}");
        }

        if let Some(caret) = prepared.caret.take() {
            window.paint_quad(caret);
        }

        let line = prepared.line.clone();
        let origin = prepared.origin;
        let scroll = prepared.scroll;
        self.field.update(cx, |field, _| {
            field.drawn = Some(Drawn {
                line,
                origin,
                bounds,
            });
            field.scroll = scroll;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_inside_new_text_convert_to_bytes() {
        assert_eq!(utf16_to_byte("abc", 1), 1);
        assert_eq!(utf16_to_byte("かん", 1), 3);
        assert_eq!(utf16_to_byte("🚀x", 2), 4);
        assert_eq!(utf16_to_byte("ab", 9), 2);
    }
}
