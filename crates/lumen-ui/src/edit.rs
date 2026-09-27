//! The editing model behind every text field.
//!
//! A string, a selection and an in-progress IME composition, and the
//! operations on them. No GPUI in here on purpose: everything a person can do
//! to a field -- move by a character or a word, extend a selection, type over
//! it, compose a character with a dead key -- is arithmetic on byte offsets,
//! and arithmetic is tested far more cheaply than a window.
//!
//! # Offsets
//!
//! Every offset is a byte offset into the UTF-8 string and always sits on a
//! character boundary. The system's input method speaks UTF-16, so the
//! conversions live here too, next to the only code that needs to agree with
//! them.
//!
//! # Characters and graphemes
//!
//! Moving and deleting step over *grapheme clusters*, not `char`s. An `é`
//! typed on a Spanish keyboard with a dead key can arrive as `e` plus a
//! combining accent: two code points, one letter on screen, and a caret that
//! could stop between them would be a caret that stops somewhere invisible.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// A single line of editable text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditBuffer {
    text: String,
    /// Where the selection started. Equal to `head` when nothing is selected.
    anchor: usize,
    /// Where the caret is. Moves; `anchor` stays put while extending.
    head: usize,
    /// Text the input method has inserted but not yet committed.
    ///
    /// While this is set, the next insertion replaces it rather than landing
    /// beside it -- which is how a dead key followed by a letter becomes one
    /// accented letter instead of an accent and a letter.
    marked: Option<Range<usize>>,
}

impl EditBuffer {
    /// A buffer holding `text`, with the caret at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            anchor: end,
            head: end,
            marked: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Where the caret is drawn.
    pub fn caret(&self) -> usize {
        self.head
    }

    /// The selected range, start before end, whichever way it was made.
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// Whether the caret is at the start of the selection rather than the end.
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Replace everything, caret at the end, nothing selected or composing.
    pub fn set_text(&mut self, text: impl Into<String>) {
        *self = Self::new(text);
    }

    /// Put the caret at `offset` and select nothing.
    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.anchor = offset;
        self.head = offset;
    }

    /// Move the caret to `offset`, keeping the anchor: extends the selection.
    pub fn select_to(&mut self, offset: usize) {
        self.head = self.clamp(offset);
    }

    /// Select `range` with the caret at its end.
    pub fn select(&mut self, range: Range<usize>) {
        self.anchor = self.clamp(range.start);
        self.head = self.clamp(range.end);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
    }

    /// Left arrow. With a selection, collapses it to its start -- the
    /// convention every text field shares, and the one that makes "select a
    /// number, press left, type" put the caret where the number began.
    pub fn move_left(&mut self) {
        let target = if self.has_selection() {
            self.selection().start
        } else {
            self.previous_grapheme(self.head)
        };
        self.move_to(target);
    }

    pub fn move_right(&mut self) {
        let target = if self.has_selection() {
            self.selection().end
        } else {
            self.next_grapheme(self.head)
        };
        self.move_to(target);
    }

    pub fn select_left(&mut self) {
        self.select_to(self.previous_grapheme(self.head));
    }

    pub fn select_right(&mut self) {
        self.select_to(self.next_grapheme(self.head));
    }

    pub fn move_word_left(&mut self) {
        self.move_to(self.previous_word_start(self.head));
    }

    pub fn move_word_right(&mut self) {
        self.move_to(self.next_word_end(self.head));
    }

    pub fn select_word_left(&mut self) {
        self.select_to(self.previous_word_start(self.head));
    }

    pub fn select_word_right(&mut self) {
        self.select_to(self.next_word_end(self.head));
    }

    pub fn move_to_start(&mut self) {
        self.move_to(0);
    }

    pub fn move_to_end(&mut self) {
        self.move_to(self.text.len());
    }

    pub fn select_to_start(&mut self) {
        self.select_to(0);
    }

    pub fn select_to_end(&mut self) {
        self.select_to(self.text.len());
    }

    /// Backspace: the selection if there is one, otherwise one grapheme.
    ///
    /// Returns whether anything changed, so the owner re-ranks only when the
    /// text did -- backspace at the start of an empty field is not an edit.
    pub fn delete_backward(&mut self) -> bool {
        if !self.has_selection() {
            self.select_to(self.previous_grapheme(self.head));
        }
        self.delete_selection()
    }

    pub fn delete_forward(&mut self) -> bool {
        if !self.has_selection() {
            self.select_to(self.next_grapheme(self.head));
        }
        self.delete_selection()
    }

    pub fn delete_word_backward(&mut self) -> bool {
        if !self.has_selection() {
            self.select_to(self.previous_word_start(self.head));
        }
        self.delete_selection()
    }

    pub fn delete_word_forward(&mut self) -> bool {
        if !self.has_selection() {
            self.select_to(self.next_word_end(self.head));
        }
        self.delete_selection()
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.replace(None, "");
        true
    }

    /// Insert `text` in place of `range`, or of the composition, or of the
    /// selection -- in that order of preference -- and commit it.
    ///
    /// The order is the input method's contract: an explicit range wins, then
    /// whatever is being composed, then what the user selected.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = self.target(range);
        self.text.replace_range(range.clone(), text);
        self.move_to(range.start + text.len());
        self.marked = None;
    }

    /// Insert `text` as an uncommitted composition.
    ///
    /// `selected`, when given, is relative to the new text: the input method
    /// placing its own caret inside what it is composing.
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = self.target(range);
        self.text.replace_range(range.clone(), text);

        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());

        match selected {
            Some(selected) => self.select(
                range.start + floor_boundary(text, selected.start)
                    ..range.start + floor_boundary(text, selected.end),
            ),
            None => self.move_to(range.start + text.len()),
        }
    }

    /// Commit the composition as it stands.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// The range an edit applies to, clamped onto character boundaries.
    fn target(&self, range: Option<Range<usize>>) -> Range<usize> {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let start = self.clamp(range.start);
        let end = self.clamp(range.end).max(start);
        start..end
    }

    /// The word containing `offset`, for a double click.
    ///
    /// A click in the gap between words selects the gap: selecting nothing
    /// would make the double click look ignored.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let offset = self.clamp(offset);
        self.text
            .split_word_bound_indices()
            .map(|(start, word)| start..start + word.len())
            .find(|range| range.contains(&offset))
            .or_else(|| {
                // Past the last character: the final segment, if any.
                self.text
                    .split_word_bound_indices()
                    .next_back()
                    .map(|(start, word)| start..start + word.len())
            })
            .unwrap_or(offset..offset)
    }

    fn previous_grapheme(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .map(|(index, _)| index)
            .find(|&index| index < offset)
            .unwrap_or(0)
    }

    fn next_grapheme(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(index, grapheme)| index + grapheme.len())
            .find(|&end| end > offset)
            .unwrap_or(self.text.len())
    }

    /// The start of the word before `offset`, skipping whitespace and
    /// punctuation on the way.
    ///
    /// "Word" in the sense of letters and digits, so `12*7` has two stops, at
    /// `12` and at `7`, and Ctrl+Left from the end of a calculation lands on
    /// the last number -- usually the one being changed.
    fn previous_word_start(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .rev()
            .map(|(start, _)| start)
            .find(|&start| start < offset)
            .unwrap_or(0)
    }

    fn next_word_end(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .map(|(start, word)| start + word.len())
            .find(|&end| end > offset)
            .unwrap_or(self.text.len())
    }

    /// Pull an offset inside the text and back onto a character boundary.
    ///
    /// Offsets arrive from outside -- a mouse position, the input method -- and
    /// slicing a `String` off a boundary panics.
    fn clamp(&self, offset: usize) -> usize {
        floor_boundary(&self.text, offset)
    }

    /// A byte offset as a UTF-16 offset, for the input method.
    pub fn to_utf16(&self, offset: usize) -> usize {
        let offset = self.clamp(offset);
        self.text[..offset].encode_utf16().count()
    }

    /// A UTF-16 offset from the input method as a byte offset.
    ///
    /// An offset that lands inside a surrogate pair rounds down to the start
    /// of that character rather than splitting it.
    pub fn from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (index, character) in self.text.char_indices() {
            if units + character.len_utf16() > offset {
                return index;
            }
            units += character.len_utf16();
        }
        self.text.len()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.from_utf16(range.start)..self.from_utf16(range.end)
    }
}

/// The largest character boundary at or before `offset`.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, caret: usize) -> EditBuffer {
        let mut buffer = EditBuffer::new(text);
        buffer.move_to(caret);
        buffer
    }

    // -- The bug that prompted all of this ------------------------------------

    /// Changing one digit of a calculation without retyping the rest.
    #[test]
    fn a_digit_in_the_middle_can_be_replaced() {
        let mut buffer = EditBuffer::new("12*7");
        buffer.move_left(); // before 7
        buffer.move_left(); // before *
        buffer.select_left(); // selects the 2
        assert_eq!(buffer.selected_text(), "2");
        buffer.replace(None, "5");
        assert_eq!(buffer.text(), "15*7");
        assert_eq!(buffer.caret(), 2);
    }

    #[test]
    fn typing_goes_where_the_caret_is() {
        let mut buffer = at("helo", 3);
        buffer.replace(None, "l");
        assert_eq!(buffer.text(), "hello");
        assert_eq!(buffer.caret(), 4);
    }

    // -- Moving -------------------------------------------------------------

    #[test]
    fn left_and_right_stop_at_the_ends() {
        let mut buffer = at("ab", 0);
        buffer.move_left();
        assert_eq!(buffer.caret(), 0);
        buffer.move_to_end();
        buffer.move_right();
        assert_eq!(buffer.caret(), 2);
    }

    #[test]
    fn an_arrow_collapses_a_selection_onto_its_edge() {
        let mut buffer = EditBuffer::new("hello");
        buffer.select(1..4);
        buffer.move_left();
        assert_eq!((buffer.caret(), buffer.has_selection()), (1, false));

        buffer.select(1..4);
        buffer.move_right();
        assert_eq!((buffer.caret(), buffer.has_selection()), (4, false));
    }

    #[test]
    fn the_caret_steps_over_a_combining_accent_in_one_move() {
        // "e" followed by U+0301: one letter, two code points.
        let text = "cafe\u{301}";
        let mut buffer = EditBuffer::new(text);
        buffer.move_left();
        assert_eq!(buffer.caret(), 3);
    }

    #[test]
    fn the_caret_steps_over_an_emoji_in_one_move() {
        let mut buffer = EditBuffer::new("ok 🚀");
        buffer.move_left();
        assert_eq!(buffer.caret(), 3);
    }

    #[test]
    fn word_motion_stops_at_numbers_in_a_calculation() {
        let mut buffer = EditBuffer::new("120*7");
        buffer.move_word_left();
        assert_eq!(buffer.caret(), 4);
        buffer.move_word_left();
        assert_eq!(buffer.caret(), 0);
        buffer.move_word_right();
        assert_eq!(buffer.caret(), 3);
    }

    #[test]
    fn word_motion_skips_runs_of_spaces() {
        let mut buffer = EditBuffer::new("visual   studio");
        buffer.move_word_left();
        assert_eq!(buffer.caret(), 9);
        buffer.move_word_left();
        assert_eq!(buffer.caret(), 0);
    }

    // -- Selecting ----------------------------------------------------------

    #[test]
    fn a_selection_can_be_made_backwards_and_reports_it() {
        let mut buffer = EditBuffer::new("hello");
        buffer.select_left();
        buffer.select_left();
        assert_eq!(buffer.selection(), 3..5);
        assert!(buffer.is_reversed());
        assert_eq!(buffer.selected_text(), "lo");
    }

    #[test]
    fn extending_past_the_anchor_flips_the_selection() {
        let mut buffer = at("hello", 2);
        buffer.select_right();
        buffer.select_left();
        buffer.select_left();
        assert_eq!(buffer.selection(), 1..2);
        assert!(buffer.is_reversed());
    }

    #[test]
    fn select_all_then_type_replaces_everything() {
        let mut buffer = EditBuffer::new("firefox");
        buffer.select_all();
        buffer.replace(None, "zed");
        assert_eq!(buffer.text(), "zed");
    }

    #[test]
    fn a_double_click_selects_the_word_under_it() {
        let buffer = EditBuffer::new("split right");
        assert_eq!(buffer.word_at(2), 0..5);
        assert_eq!(buffer.word_at(8), 6..11);
        assert_eq!(buffer.word_at(11), 6..11);
    }

    // -- Deleting -----------------------------------------------------------

    #[test]
    fn backspace_removes_the_selection_rather_than_a_character() {
        let mut buffer = EditBuffer::new("hello");
        buffer.select(1..4);
        assert!(buffer.delete_backward());
        assert_eq!(buffer.text(), "ho");
    }

    #[test]
    fn backspace_removes_a_whole_character_not_one_byte() {
        let mut buffer = EditBuffer::new("café");
        buffer.delete_backward();
        assert_eq!(buffer.text(), "caf");
    }

    #[test]
    fn backspace_at_the_start_changes_nothing() {
        let mut buffer = at("abc", 0);
        assert!(!buffer.delete_backward());
        assert_eq!(buffer.text(), "abc");
    }

    #[test]
    fn delete_removes_forward() {
        let mut buffer = at("abc", 1);
        assert!(buffer.delete_forward());
        assert_eq!(buffer.text(), "ac");
        assert_eq!(buffer.caret(), 1);
    }

    #[test]
    fn word_delete_skips_trailing_whitespace() {
        let mut buffer = EditBuffer::new("ctrl shift ");
        buffer.delete_word_backward();
        assert_eq!(buffer.text(), "ctrl ");
    }

    #[test]
    fn word_delete_on_a_single_word_clears_it() {
        let mut buffer = EditBuffer::new("ctrl");
        buffer.delete_word_backward();
        assert_eq!(buffer.text(), "");
    }

    #[test]
    fn word_delete_forward_removes_the_next_word() {
        let mut buffer = at("open zed now", 4);
        buffer.delete_word_forward();
        assert_eq!(buffer.text(), "open now");
    }

    // -- The input method ---------------------------------------------------

    /// A Spanish keyboard: the accent key, then the letter.
    #[test]
    fn a_dead_key_and_a_letter_become_one_character() {
        let mut buffer = EditBuffer::new("caf");
        buffer.replace_and_mark(None, "´", None);
        assert_eq!(buffer.marked(), Some(3..5));
        buffer.replace(None, "é");
        assert_eq!(buffer.text(), "café");
        assert_eq!(buffer.marked(), None);
        assert_eq!(buffer.caret(), 5);
    }

    #[test]
    fn a_composition_replaces_itself_as_it_grows() {
        let mut buffer = EditBuffer::new("");
        buffer.replace_and_mark(None, "k", None);
        buffer.replace_and_mark(None, "か", None);
        buffer.replace_and_mark(None, "かん", None);
        assert_eq!(buffer.text(), "かん");
        buffer.replace(None, "漢");
        assert_eq!(buffer.text(), "漢");
    }

    #[test]
    fn a_composition_can_place_its_own_caret() {
        let mut buffer = EditBuffer::new("x");
        buffer.replace_and_mark(None, "abc", Some(1..1));
        assert_eq!(buffer.caret(), 2);
    }

    #[test]
    fn utf16_offsets_round_trip_through_a_surrogate_pair() {
        let buffer = EditBuffer::new("a🚀b");
        assert_eq!(buffer.to_utf16(1), 1);
        assert_eq!(buffer.to_utf16(5), 3);
        assert_eq!(buffer.from_utf16(3), 5);
        // Inside the pair rounds down to the start of the character.
        assert_eq!(buffer.from_utf16(2), 1);
    }

    #[test]
    fn an_offset_off_a_boundary_is_pulled_back_rather_than_panicking() {
        let mut buffer = EditBuffer::new("é");
        buffer.move_to(1);
        assert_eq!(buffer.caret(), 0);
        buffer.replace(Some(1..99), "x");
        assert_eq!(buffer.text(), "x");
    }
}
