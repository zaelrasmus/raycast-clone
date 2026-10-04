//! Where the query is in a text preview (F16, slice 6b).
//!
//! A row Windows Search found by its contents is not marked: the preview
//! shows why it is there. Every word of the query is highlighted in the
//! card, in any case, and the card starts a few lines above the first
//! match when that match would otherwise sit below what the card shows.
//! The same for every text preview, whichever search found the row.

use std::ops::Range;

/// The fewest characters a trimmed query has before contents are asked
/// for, and before its words are highlighted.
pub const FROM_CHARACTERS: usize = 3;

/// A match on one of the first lines is shown from the file's start.
pub const SHOWN_FROM_START: usize = 4;

/// The lines kept above the first match when the card starts lower.
pub const LINES_ABOVE: usize = 2;

/// The words of `query` worth highlighting: none for a query under
/// [`FROM_CHARACTERS`], and never a single character, which would mark
/// half the card.
pub fn words(query: &str) -> Vec<String> {
    let query = query.trim();
    if query.chars().count() < FROM_CHARACTERS {
        return Vec::new();
    }
    query
        .split_whitespace()
        .filter(|word| word.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

/// What the card draws of `text`: from byte `start`, with `ranges` -- byte
/// ranges into the text from `start` -- highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub start: usize,
    pub ranges: Vec<Range<usize>>,
}

/// Every case-insensitive occurrence of `words` in `text`, as sorted byte
/// ranges that do not overlap, and the line the card starts from so the
/// first is visible.
pub fn shown(text: &str, words: &[String]) -> Shown {
    let ranges = matches(text, words);
    let start = match ranges.first() {
        Some(first) => start_for(text, first.start),
        None => 0,
    };
    Shown {
        start,
        ranges: ranges
            .into_iter()
            .filter(|range| range.start >= start)
            .map(|range| range.start - start..range.end - start)
            .collect(),
    }
}

/// The byte the card starts from for a first match at byte `at`: the
/// file's start when the match is on one of its first lines, otherwise
/// [`LINES_ABOVE`] lines above the match's.
fn start_for(text: &str, at: usize) -> usize {
    let line = text[..at].matches('\n').count();
    if line < SHOWN_FROM_START {
        return 0;
    }
    let skipped = line - LINES_ABOVE;
    text.match_indices('\n')
        .nth(skipped - 1)
        .map_or(0, |(index, _)| index + 1)
}

/// Every occurrence of each word, in any case, merged where they touch.
fn matches(text: &str, words: &[String]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = words
        .iter()
        .flat_map(|word| occurrences(text, word))
        .collect();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// A character as it is compared: lower case, its first character where
/// lowering makes more than one.
fn fold(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

fn occurrences(text: &str, word: &str) -> Vec<Range<usize>> {
    let word: Vec<char> = word.chars().map(fold).collect();
    let Some(&first) = word.first() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut indices = text.char_indices();
    while let Some((start, character)) = indices.next() {
        if fold(character) != first {
            continue;
        }
        let mut rest = text[start..].char_indices().skip(1);
        let mut end = start + character.len_utf8();
        let whole = word[1..].iter().all(|wanted| match rest.next() {
            Some((offset, character)) if fold(character) == *wanted => {
                end = start + offset + character.len_utf8();
                true
            }
            _ => false,
        });
        if whole {
            found.push(start..end);
            // Past the match: occurrences do not overlap.
            while indices.offset() < end {
                indices.next();
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_of(query: &str) -> Vec<String> {
        words(query)
    }

    #[test]
    fn a_query_is_highlighted_from_three_characters_without_single_letters() {
        assert!(words_of("ab").is_empty());
        assert!(words_of("  ab ").is_empty());
        assert_eq!(words_of("abc"), ["abc"]);
        assert_eq!(words_of("a budget plan"), ["budget", "plan"]);
    }

    #[test]
    fn every_word_is_found_in_any_case() {
        let text = "Budget for the plan.\nThe BUDGET and the Plan.";
        let words = words_of("budget plan");
        let shown = shown(text, &words);
        assert_eq!(shown.start, 0);
        let marked: Vec<&str> = shown.ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(marked, ["Budget", "plan", "BUDGET", "Plan"]);
    }

    #[test]
    fn ranges_are_bytes_beside_wider_characters() {
        let text = "café Über über";
        let shown = shown(text, &words_of("über"));
        let marked: Vec<&str> = shown.ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(marked, ["Über", "über"]);
    }

    #[test]
    fn overlapping_words_are_one_range() {
        let text = "the reports";
        let shown = shown(text, &words_of("report reports"));
        assert_eq!(shown.ranges, vec![4..11]);
    }

    #[test]
    fn nothing_matched_is_the_file_from_its_start() {
        assert_eq!(
            shown("one\ntwo", &words_of("zzz")),
            Shown {
                start: 0,
                ranges: Vec::new()
            }
        );
        assert_eq!(shown("one\ntwo", &[]).ranges, Vec::<Range<usize>>::new());
    }

    /// A match on the first lines leaves the card at the file's start; one
    /// further down starts it two lines above the match.
    #[test]
    fn the_card_starts_above_a_match_further_down() {
        let early = "a\nb\nc\nbudget\ne";
        assert_eq!(shown(early, &words_of("budget")).start, 0);

        let text = "l0\nl1\nl2\nl3\nl4\nl5 budget\nl6";
        let shown = shown(text, &words_of("budget"));
        assert_eq!(&text[shown.start..], "l3\nl4\nl5 budget\nl6");
        let rest = &text[shown.start..];
        assert_eq!(&rest[shown.ranges[0].clone()], "budget");
    }
}
