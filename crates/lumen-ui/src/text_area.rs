//! A multi-line text field that draws Markdown source styled, with a
//! formatting toolbar under it (F11, slice 5).
//!
//! # The text is the source
//!
//! What is on screen is exactly the string being edited: `**bold**` shows
//! its markers, dimmed, around bold text. So the caret, the selection, undo
//! and the input method all work on one string, with nothing to map between
//! a display and a document. The styling is
//! [`lumen_core::snippets::markup::styles`], read once per edit; the toolbar
//! is [`lumen_core::snippets::markup::wrap`].
//!
//! # What it shares with the single-line field
//!
//! The editing model, [`EditBuffer`], which never assumed one line: a line
//! break is one more grapheme to step over. The actions for moving, selecting,
//! deleting and the clipboard are the same actions, bound again in this
//! field's key context (I-14). What is its own is layout: the text wraps, so
//! Up, Down, Home and End are about *visual* lines, which only the shaped
//! text knows -- that is [`Drawn`].

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    EntityInputHandler, FocusHandle, Focusable, FontStyle, FontWeight, GlobalElementId, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, div, fill, point, px, relative, size,
};
use lumen_core::layout;
use lumen_core::snippets::markup::{self, Wrap};
use lumen_core::theme::Colors;

use crate::edit::EditBuffer;
use crate::styling::{color, type_scale};
use crate::text_field::{
    Copy, Cut, DeleteBackward, DeleteForward, DeleteWordBackward, DeleteWordForward, MoveLeft,
    MoveRight, MoveToEnd, MoveToStart, MoveWordLeft, MoveWordRight, Paste, SelectAll, SelectLeft,
    SelectRight, SelectToEnd, SelectToStart, SelectWordLeft, SelectWordRight, utf16_to_byte,
};

actions!(
    lumen,
    [
        /// Move the caret up a visual line.
        MoveUp,
        /// Move the caret down a visual line.
        MoveDown,
        /// Extend the selection up a visual line.
        SelectLineUp,
        /// Extend the selection down a visual line.
        SelectLineDown,
        /// Move the caret to the start of its visual line.
        MoveToLineStart,
        /// Move the caret to the end of its visual line.
        MoveToLineEnd,
        /// Extend the selection to the start of the visual line.
        SelectToLineStart,
        /// Extend the selection to the end of the visual line.
        SelectToLineEnd,
        /// Start a new line.
        Newline,
        /// Undo the last edit.
        Undo,
        /// Redo what was undone.
        Redo,
        /// Bold: wrap the selection in `**`, or take them off.
        ToggleStrong,
        /// Italic: `_`.
        ToggleEmphasis,
        /// Underline: `++`.
        ToggleUnderline,
        /// Strikethrough: `~~`.
        ToggleStrike,
        /// Code: `` ` ``.
        ToggleCode,
        /// Make the selection a link, and select its address to type.
        InsertLink,
        /// Take the Markdown markers out of the selection.
        ClearFormatting,
    ]
);

/// The key context the bindings apply in.
pub const CONTEXT: &str = "TextArea";

/// How many edits undo remembers.
const UNDO_DEPTH: usize = 200;

/// A multi-line editable string, drawn as styled Markdown source.
pub struct TextArea {
    focus_handle: FocusHandle,
    buffer: EditBuffer,
    /// The buffer's styling, read when the text changes rather than per frame.
    styles: Vec<(Range<usize>, markup::Style)>,
    placeholder: SharedString,
    colors: Colors,
    /// Whether the text is Markdown: styled, with the toolbar. Off for a
    /// snippet written as plain text, which is pasted exactly as written.
    markdown: bool,
    history: History,
    /// Where Up and Down aim horizontally, kept across a run of them so the
    /// caret comes back to its column after passing a short line.
    goal_x: Option<Pixels>,
    drawn: Option<Drawn>,
    /// How far the text is scrolled down.
    scroll: Pixels,
    dragging: bool,
}

/// What the field tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextAreaEvent {
    Changed(String),
}

impl gpui::EventEmitter<TextAreaEvent> for TextArea {}

/// Which kind of edit, for grouping undo steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// Typed text: a run of it undoes as one step.
    Typing,
    /// Everything else, one step each.
    Other,
}

impl TextArea {
    pub fn new(
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        colors: Colors,
        cx: &mut App,
    ) -> Self {
        let buffer = EditBuffer::new(text);
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            styles: markup::styles(buffer.text()),
            buffer,
            placeholder: placeholder.into(),
            colors,
            markdown: true,
            history: History::default(),
            goal_x: None,
            drawn: None,
            scroll: px(0.0),
            dragging: false,
        }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// Replace the contents without emitting a change, and forget the undo
    /// history, which was about other text.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.set_text(text);
        self.restyle();
        self.history = History::default();
        self.scroll = px(0.0);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn set_colors(&mut self, colors: Colors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    /// Draw the text as Markdown, with the toolbar, or as plain text.
    pub fn set_markdown(&mut self, markdown: bool, cx: &mut Context<Self>) {
        self.markdown = markdown;
        self.restyle();
        cx.notify();
    }

    fn restyle(&mut self) {
        self.styles = if self.markdown {
            markup::styles(self.buffer.text())
        } else {
            Vec::new()
        };
    }

    /// Apply an edit, remembering what was there for undo, and tell the
    /// owner if the text changed.
    fn edit(
        &mut self,
        kind: EditKind,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut EditBuffer),
    ) {
        let before = Snapshot::of(&self.buffer);
        apply(&mut self.buffer);
        self.goal_x = None;
        if self.buffer.text() != before.text {
            self.history.record(before, kind);
            self.restyle();
            cx.emit(TextAreaEvent::Changed(self.buffer.text().to_owned()));
        }
        cx.notify();
    }

    /// Move or select without editing. Ends a run of typing for undo.
    fn navigate(&mut self, cx: &mut Context<Self>, apply: impl FnOnce(&mut EditBuffer)) {
        apply(&mut self.buffer);
        self.goal_x = None;
        self.history.break_run();
        cx.notify();
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.buffer.set_text(snapshot.text);
        self.buffer.select(snapshot.selection);
        self.restyle();
        self.goal_x = None;
        cx.emit(TextAreaEvent::Changed(self.buffer.text().to_owned()));
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let current = Snapshot::of(&self.buffer);
        if let Some(snapshot) = self.history.undo(current) {
            self.restore(snapshot, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let current = Snapshot::of(&self.buffer);
        if let Some(snapshot) = self.history.redo(current) {
            self.restore(snapshot, cx);
        }
    }

    /// A toolbar button or its shortcut.
    fn format(&mut self, wrap: Wrap, cx: &mut Context<Self>) {
        if !self.markdown {
            return;
        }
        let edit = markup::wrap(self.buffer.text(), self.buffer.selection(), wrap);
        self.edit(EditKind::Other, cx, |buffer| {
            buffer.replace(Some(edit.range.clone()), &edit.text);
            buffer.select(edit.selection.clone());
        });
    }

    fn toggle_strong(&mut self, _: &ToggleStrong, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Strong, cx);
    }

    fn toggle_emphasis(&mut self, _: &ToggleEmphasis, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Emphasis, cx);
    }

    fn toggle_underline(&mut self, _: &ToggleUnderline, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Underline, cx);
    }

    fn toggle_strike(&mut self, _: &ToggleStrike, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Strike, cx);
    }

    fn toggle_code(&mut self, _: &ToggleCode, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Code, cx);
    }

    fn insert_link(&mut self, _: &InsertLink, _: &mut Window, cx: &mut Context<Self>) {
        self.format(Wrap::Link, cx);
    }

    fn clear_formatting(&mut self, _: &ClearFormatting, _: &mut Window, cx: &mut Context<Self>) {
        if !self.markdown || !self.buffer.has_selection() {
            return;
        }
        let start = self.buffer.selection().start;
        let plain = markup::plain(self.buffer.selected_text());
        self.edit(EditKind::Other, cx, |buffer| {
            buffer.replace(None, &plain);
            buffer.select(start..start + plain.len());
        });
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(EditKind::Other, cx, |buffer| buffer.replace(None, "\n"));
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

    /// Up or Down by `lines` visual lines, moving or extending.
    fn vertical(&mut self, lines: f32, select: bool, cx: &mut Context<Self>) {
        let Some(drawn) = &self.drawn else {
            return;
        };
        let caret = drawn.position(self.buffer.caret());
        let x = self.goal_x.unwrap_or(caret.x);
        let y = caret.y + drawn.line_height * lines;
        let target = if y < px(0.0) {
            0
        } else if y >= drawn.content_height {
            self.buffer.text().len()
        } else {
            drawn.offset_at(point(x, y + drawn.line_height / 2.0))
        };
        if select {
            self.buffer.select_to(target);
        } else {
            self.buffer.move_to(target);
        }
        // Kept, so the next Up or Down aims for the same column.
        self.goal_x = Some(x);
        self.history.break_run();
        cx.notify();
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1.0, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1.0, false, cx);
    }

    fn select_up(&mut self, _: &SelectLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1.0, true, cx);
    }

    fn select_down(&mut self, _: &SelectLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1.0, true, cx);
    }

    /// The start or end of the caret's visual line.
    fn line_edge(&mut self, end: bool, select: bool, cx: &mut Context<Self>) {
        let Some(drawn) = &self.drawn else {
            return;
        };
        let caret = drawn.position(self.buffer.caret());
        let x = if end { px(f32::MAX / 4.0) } else { px(0.0) };
        let target = drawn.offset_at(point(x, caret.y + drawn.line_height / 2.0));
        self.navigate(cx, |buffer| {
            if select {
                buffer.select_to(target);
            } else {
                buffer.move_to(target);
            }
        });
    }

    fn move_to_line_start(&mut self, _: &MoveToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.line_edge(false, false, cx);
    }

    fn move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.line_edge(true, false, cx);
    }

    fn select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edge(false, true, cx);
    }

    fn select_to_line_end(&mut self, _: &SelectToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.line_edge(true, true, cx);
    }

    fn delete_backward(&mut self, _: &DeleteBackward, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(EditKind::Other, cx, |buffer| {
            buffer.delete_backward();
        });
    }

    fn delete_forward(&mut self, _: &DeleteForward, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(EditKind::Other, cx, |buffer| {
            buffer.delete_forward();
        });
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(EditKind::Other, cx, |buffer| {
            buffer.delete_word_backward();
        });
    }

    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(EditKind::Other, cx, |buffer| {
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
        self.edit(EditKind::Other, cx, |buffer| buffer.replace(None, ""));
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // Lines kept, in the one form the rest of Lumen writes them.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.edit(EditKind::Other, cx, |buffer| buffer.replace(None, &text));
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window);
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
        self.goal_x = None;
        self.history.break_run();
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

    /// The offset nearest a point in window coordinates.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        match &self.drawn {
            Some(drawn) => drawn.offset_at(position - drawn.origin),
            None => self.buffer.text().len(),
        }
    }

    fn tool(
        &self,
        id: &'static str,
        label: &'static str,
        styled: impl FnOnce(gpui::Div) -> gpui::Div,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let colors = &self.colors;
        div()
            .id(id)
            // A word is wider than a letter: the button grows to fit it
            // rather than letting it spill onto the next one.
            .h(px(layout::text_area::TOOL_SIZE))
            .min_w(px(layout::text_area::TOOL_SIZE))
            .px(px(layout::text_area::TOOL_PADDING_X))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(layout::text_area::TOOL_RADIUS))
            .text_size(type_scale::title())
            .text_color(color(colors.text_muted))
            .cursor_pointer()
            // The label drawn the way the button makes text look.
            .child(styled(div()).child(label))
            .hover(|tool| tool.bg(color(colors.selection_background)))
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                // Keep the text's focus and selection: the button acts on them.
                window.prevent_default();
            })
            .on_click(cx.listener(move |area, _, window, cx| {
                run(area, window, cx);
                area.focus(window);
            }))
    }
}

/// The text and its selection, as undo keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    selection: Range<usize>,
}

impl Snapshot {
    fn of(buffer: &EditBuffer) -> Self {
        Self {
            text: buffer.text().to_owned(),
            selection: buffer.selection(),
        }
    }
}

/// Undo and redo, with a run of typing kept as one step.
#[derive(Debug, Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last edit was typing, with nothing between it and the next.
    typing: bool,
}

impl History {
    /// `before` is the text as it was before an edit of `kind`.
    fn record(&mut self, before: Snapshot, kind: EditKind) {
        let continues = kind == EditKind::Typing && self.typing;
        if !continues {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = kind == EditKind::Typing;
    }

    fn break_run(&mut self) {
        self.typing = false;
    }

    fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        self.typing = false;
        Some(previous)
    }

    fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.typing = false;
        Some(next)
    }
}

impl EntityInputHandler for TextArea {
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
        self.edit(EditKind::Typing, cx, |buffer| buffer.replace(range, text));
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
        let selected =
            selected.map(|range| utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end));
        self.edit(EditKind::Typing, cx, |buffer| {
            buffer.replace_and_mark(range, text, selected)
        });
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
        let start = drawn.origin + drawn.position(range.start);
        let end = drawn.origin + drawn.position(range.end);
        Some(Bounds::from_corners(
            start,
            point(end.x.max(start.x), start.y + drawn.line_height),
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

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let colors = self.colors.clone();
        let divider = || {
            div()
                .w(px(1.0))
                .h(px(layout::text_area::TOOL_DIVIDER_HEIGHT))
                .mx(px(layout::text_area::TOOL_GAP))
                .bg(color(colors.border_subtle))
        };

        let toolbar = div()
            .h(px(layout::text_area::TOOLBAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(layout::text_area::TOOL_GAP))
            .child(self.tool(
                "bold",
                "B",
                |t| t.font_weight(FontWeight::BOLD),
                cx,
                |a, _, cx| a.format(Wrap::Strong, cx),
            ))
            .child(self.tool(
                "italic",
                "I",
                |t| t.italic(),
                cx,
                |a, _, cx| a.format(Wrap::Emphasis, cx),
            ))
            .child(self.tool(
                "underline",
                "U",
                |t| t.underline(),
                cx,
                |a, _, cx| a.format(Wrap::Underline, cx),
            ))
            .child(self.tool(
                "strike",
                "S",
                |t| t.line_through(),
                cx,
                |a, _, cx| a.format(Wrap::Strike, cx),
            ))
            .child(divider())
            .child(self.tool(
                "code",
                "</>",
                |t| t,
                cx,
                |a, _, cx| a.format(Wrap::Code, cx),
            ))
            .child(self.tool(
                "link",
                "Link",
                |t| t,
                cx,
                |a, _, cx| a.format(Wrap::Link, cx),
            ))
            .child(divider())
            .child(self.tool(
                "plain",
                "Tx",
                |t| t,
                cx,
                |a, window, cx| a.clear_formatting(&ClearFormatting, window, cx),
            ));

        let text = div()
            .flex_1()
            .min_h(px(0.0))
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .child(TextBlock { area: cx.entity() });

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
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
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::move_to_line_start))
            .on_action(cx.listener(Self::move_to_line_end))
            .on_action(cx.listener(Self::select_to_line_start))
            .on_action(cx.listener(Self::select_to_line_end))
            .on_action(cx.listener(Self::delete_backward))
            .on_action(cx.listener(Self::delete_forward))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::toggle_strong))
            .on_action(cx.listener(Self::toggle_emphasis))
            .on_action(cx.listener(Self::toggle_underline))
            .on_action(cx.listener(Self::toggle_strike))
            .on_action(cx.listener(Self::toggle_code))
            .on_action(cx.listener(Self::insert_link))
            .on_action(cx.listener(Self::clear_formatting))
            .size_full()
            .flex()
            .flex_col()
            .p(px(layout::text_area::PADDING))
            .rounded(px(layout::text_area::RADIUS))
            .border_1()
            .border_color(color(if focused {
                self.colors.accent
            } else {
                self.colors.border_subtle
            }))
            .bg(color(self.colors.surface_elevated))
            .text_size(type_scale::title())
            .child(text)
            .when(self.markdown, |area| area.child(toolbar))
    }
}

/// The last frame's layout: each hard line, shaped and wrapped, and where.
struct Drawn {
    /// Each line's first byte in the text, its shape, and its top relative to
    /// the text's origin.
    lines: Vec<(usize, WrappedLine, Pixels)>,
    /// The empty text's placeholder was drawn: every offset is 0.
    placeholder: bool,
    /// Where offset 0 is drawn: the element's origin, shifted by the scroll.
    origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    content_height: Pixels,
}

impl Drawn {
    /// Where `offset` is drawn, relative to the origin: the top-left of its
    /// place in its visual line.
    fn position(&self, offset: usize) -> Point<Pixels> {
        if self.placeholder {
            return point(px(0.0), px(0.0));
        }
        let index = self
            .lines
            .partition_point(|(start, _, _)| *start <= offset)
            .saturating_sub(1);
        let Some((start, line, top)) = self.lines.get(index) else {
            return point(px(0.0), px(0.0));
        };
        let local = offset.saturating_sub(*start).min(line.len());
        let at = line
            .position_for_index(local, self.line_height)
            .unwrap_or_else(|| point(line.width(), px(0.0)));
        point(at.x, *top + at.y)
    }

    /// The offset nearest `position`, relative to the origin.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        if self.placeholder || self.lines.is_empty() {
            return 0;
        }
        let index = self
            .lines
            .partition_point(|(_, _, top)| *top <= position.y)
            .saturating_sub(1);
        let (start, line, top) = &self.lines[index];
        let height = line.size(self.line_height).height;
        let y = (position.y - *top)
            .max(px(0.0))
            .min((height - px(1.0)).max(px(0.0)));
        let x = position.x.max(px(0.0));
        let local = match line.closest_index_for_position(point(x, y), self.line_height) {
            Ok(index) | Err(index) => index,
        };
        start + local.min(line.len())
    }
}

/// The styled, wrapped text, its selection and its caret.
///
/// An element for the reasons [`crate::text_field`]'s line is one: it must
/// know where each character landed, and it registers the field as the
/// window's input handler, which only a paint phase can do.
struct TextBlock {
    area: gpui::Entity<TextArea>,
}

struct Prepared {
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextBlock {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextBlock {
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
        style.size.height = relative(1.0).into();
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
        let area = self.area.read(cx);
        let buffer = &area.buffer;
        let colors = &area.colors;
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let placeholder = buffer.is_empty();

        let base = TextRun {
            len: 0,
            font: style.font(),
            color: color(colors.text_primary),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let (text, runs) = if placeholder {
            let text = area.placeholder.clone();
            let run = TextRun {
                len: text.len(),
                color: color(colors.text_placeholder),
                ..base
            };
            (text, vec![run])
        } else {
            let text = SharedString::from(buffer.text().to_owned());
            let runs = runs(buffer.text(), &area.styles, buffer.marked(), &base, colors);
            (text, runs)
        };

        let lines = window
            .text_system()
            .shape_text(text, font_size, &runs, Some(bounds.size.width), None)
            .unwrap_or_default();

        let mut start = 0;
        let mut top = px(0.0);
        let mut placed = Vec::with_capacity(lines.len());
        for line in lines {
            let len = line.len();
            let height = line.size(line_height).height.max(line_height);
            placed.push((start, line, top));
            start += len + 1;
            top += height;
        }
        let content_height = top;

        let mut drawn = Drawn {
            lines: placed,
            placeholder,
            origin: bounds.origin,
            bounds,
            line_height,
            content_height,
        };

        // Keep the caret in view, and give back the scroll it no longer needs.
        let caret = drawn.position(buffer.caret());
        let overflow = (content_height - bounds.size.height).max(px(0.0));
        let mut scroll = area.scroll.min(overflow);
        if caret.y < scroll {
            scroll = caret.y;
        }
        if caret.y + line_height > scroll + bounds.size.height {
            scroll = caret.y + line_height - bounds.size.height;
        }
        drawn.origin = point(bounds.left(), bounds.top() - scroll);
        let origin = drawn.origin;

        let mut selection = Vec::new();
        if buffer.has_selection() && !placeholder {
            let range = buffer.selection();
            let from = drawn.position(range.start);
            let to = drawn.position(range.end);
            let highlight = color(colors.text_selection);
            let mut y = from.y;
            while y <= to.y {
                let left = if y == from.y { from.x } else { px(0.0) };
                let right = if y == to.y { to.x } else { bounds.size.width };
                if right > left {
                    selection.push(fill(
                        Bounds::from_corners(
                            origin + point(left, y),
                            origin + point(right, y + line_height),
                        ),
                        highlight,
                    ));
                }
                y += line_height;
            }
        }

        let focused = area.focus_handle.is_focused(window);
        let caret = (focused && !buffer.has_selection()).then(|| {
            let top = caret.y + (line_height - font_size) / 2.0;
            fill(
                Bounds::new(
                    origin + point(caret.x, top),
                    size(px(layout::CARET_WIDTH), font_size),
                ),
                color(colors.accent),
            )
        });

        self.area.update(cx, |area, _| {
            area.scroll = scroll;
            area.drawn = Some(drawn);
        });

        Prepared { selection, caret }
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
        let focus_handle = self.area.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.area.clone()),
            cx,
        );

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepared.selection.drain(..) {
                window.paint_quad(quad);
            }
            let area = self.area.read(cx);
            if let Some(drawn) = &area.drawn {
                let line_height = drawn.line_height;
                let origin = drawn.origin;
                let lines: Vec<(WrappedLine, Pixels)> = drawn
                    .lines
                    .iter()
                    .map(|(_, line, top)| (line.clone(), *top))
                    .collect();
                for (line, top) in lines {
                    // Lines out of view are not drawn.
                    let y = origin.y + top;
                    if y > bounds.bottom() || y + line.size(line_height).height < bounds.top() {
                        continue;
                    }
                    if let Err(error) = line.paint(
                        point(origin.x, y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    ) {
                        log::debug!("could not paint a text area: {error:#}");
                    }
                }
            }
            if let Some(caret) = prepared.caret.take() {
                window.paint_quad(caret);
            }
        });
    }
}

/// Text runs covering all of `text`: the Markdown styling, and the input
/// method's uncommitted text underlined.
fn runs(
    text: &str,
    styles: &[(Range<usize>, markup::Style)],
    marked: Option<Range<usize>>,
    base: &TextRun,
    colors: &Colors,
) -> Vec<TextRun> {
    // Every place the look can change.
    let mut cuts = vec![0, text.len()];
    for (range, _) in styles {
        cuts.push(range.start);
        cuts.push(range.end);
    }
    if let Some(marked) = &marked {
        cuts.push(marked.start);
        cuts.push(marked.end);
    }
    cuts.retain(|cut| *cut <= text.len());
    cuts.sort_unstable();
    cuts.dedup();

    let mut runs = Vec::with_capacity(cuts.len());
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let style = styles
            .iter()
            .find(|(range, _)| range.start <= from && from < range.end)
            .map(|(_, style)| *style)
            .unwrap_or_default();
        let mut run = TextRun {
            len: to - from,
            ..base.clone()
        };
        if style.strong {
            run.font.weight = FontWeight::BOLD;
        }
        if style.emphasis {
            run.font.style = FontStyle::Italic;
        }
        if style.link {
            run.color = color(colors.accent);
        }
        if style.marker {
            run.color = color(colors.text_muted);
        }
        if style.code && !style.marker {
            run.background_color = Some(color(colors.selection_background));
        }
        let underline = style.underline || (style.link && !style.marker);
        let composing = marked
            .as_ref()
            .is_some_and(|marked| marked.start <= from && from < marked.end);
        if underline || composing {
            run.underline = Some(UnderlineStyle {
                color: Some(run.color),
                thickness: px(layout::CARET_WIDTH),
                wavy: false,
            });
        }
        if style.strike {
            run.strikethrough = Some(StrikethroughStyle {
                color: Some(run.color),
                thickness: px(layout::CARET_WIDTH),
            });
        }
        runs.push(run);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            text: text.into(),
            selection: text.len()..text.len(),
        }
    }

    #[test]
    fn a_run_of_typing_undoes_as_one_step() {
        let mut history = History::default();
        history.record(snapshot(""), EditKind::Typing);
        history.record(snapshot("h"), EditKind::Typing);
        history.record(snapshot("hi"), EditKind::Typing);
        assert_eq!(history.undo(snapshot("hi!")), Some(snapshot("")));
        assert_eq!(history.undo(snapshot("")), None);
    }

    #[test]
    fn moving_the_caret_ends_a_run_and_other_edits_are_steps_of_their_own() {
        let mut history = History::default();
        history.record(snapshot(""), EditKind::Typing);
        history.break_run();
        history.record(snapshot("a"), EditKind::Typing);
        history.record(snapshot("ab"), EditKind::Other);
        history.record(snapshot("ab\n"), EditKind::Other);
        assert_eq!(history.undo(snapshot("ab\nc")), Some(snapshot("ab\n")));
        assert_eq!(history.undo(snapshot("ab\n")), Some(snapshot("ab")));
        assert_eq!(history.undo(snapshot("ab")), Some(snapshot("a")));
        assert_eq!(history.undo(snapshot("a")), Some(snapshot("")));
    }

    #[test]
    fn redo_comes_back_and_a_new_edit_forgets_it() {
        let mut history = History::default();
        history.record(snapshot(""), EditKind::Other);
        assert_eq!(history.undo(snapshot("x")), Some(snapshot("")));
        assert_eq!(history.redo(snapshot("")), Some(snapshot("x")));
        history.undo(snapshot("x"));
        history.record(snapshot(""), EditKind::Other);
        assert_eq!(history.redo(snapshot("y")), None);
    }

    #[test]
    fn undo_is_bounded() {
        let mut history = History::default();
        for step in 0..UNDO_DEPTH + 50 {
            history.record(snapshot(&step.to_string()), EditKind::Other);
        }
        assert_eq!(history.undo.len(), UNDO_DEPTH);
    }
}
