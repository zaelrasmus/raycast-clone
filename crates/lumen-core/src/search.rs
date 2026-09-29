//! Matching and ranking.
//!
//! `nucleo-matcher` supplies the fuzzy score; this module supplies the policy
//! around it -- what gets matched, how usage weighs against similarity, and how
//! the matched characters are reported back for highlighting.
//!
//! Budget note: the per-frame cost while typing is capped at p95 < 8 ms with the
//! full list loaded (section 5), which is why this is a plain linear pass over a
//! prepared haystack rather than anything with an index to invalidate. At a few
//! thousand entries the straightforward thing is comfortably inside budget, and
//! `benches/search.rs` is what keeps that claim honest rather than assumed.

use lumen_platform::AppEntry;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use crate::frecency::Frecency;
use crate::item::Item;

/// One searchable entry with its haystacks prepared up front.
///
/// Converting to `Utf32String` is not free, and doing it per keystroke rather
/// than per reindex is the difference between a linear pass that fits the frame
/// budget and one that does not.
pub struct Candidate {
    pub item: Item,
    name: Utf32String,
    /// Keywords and detail, matched at a discount so they can find an entry
    /// without outranking a genuine name match.
    aliases: Vec<Utf32String>,
}

impl Candidate {
    pub fn new(item: Item) -> Self {
        let name = Utf32String::from(item.title.as_str());
        let aliases = item
            .keywords
            .iter()
            .map(|keyword| Utf32String::from(keyword.as_str()))
            .collect();
        Self {
            item,
            name,
            aliases,
        }
    }

    pub fn from_app(entry: AppEntry) -> Self {
        Self::new(Item::from_app(entry))
    }
}

/// A candidate that matched, with everything the UI needs to draw it.
pub struct Match<'a> {
    /// Position of the matched candidate in the slice that was ranked.
    ///
    /// Carried so the view can hold results as indices rather than cloning
    /// every item out of the index on every keystroke.
    pub index: usize,
    pub item: &'a Item,
    pub score: u32,
    /// Character offsets into `item.title` that matched, for highlighting.
    ///
    /// Empty when the match came from an alias rather than the title: there is
    /// nothing in the visible string to highlight, and inventing offsets would
    /// underline the wrong characters.
    pub highlights: Vec<u32>,
}

/// How far an alias match is discounted against a name match.
///
/// A keyword hit should surface an entry the user could not have named, without
/// letting it jump ahead of an entry whose actual title matched well.
///
/// Only well: a keyword is scored even when the title matched too, and the
/// better of the two counts (F9). Otherwise any scattered match in a title
/// beats an exact keyword -- `spain` found the Saint Helena pound, through
/// the S, P, A, I, N of "SHP Saint", before the euro, which Spain uses.
/// Thirty rather than the forty it was for the same reason: that match
/// scores 101 and an exact "Spain" 140, so forty left the keyword a point
/// short, while thirty still keeps an equally good title ahead.
const ALIAS_PENALTY: u32 = 30;

pub struct Search {
    matcher: Matcher,
    /// Reused across queries so highlighting does not allocate per candidate.
    indices: Vec<u32>,
}

impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}

impl Search {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            indices: Vec::new(),
        }
    }

    /// Rank `candidates` against `query`, best first.
    ///
    /// An empty query is not a match of everything: it is the root list, ordered
    /// by usage alone. That is what makes the launcher useful in the half second
    /// before the user has typed anything.
    pub fn rank<'a>(
        &mut self,
        query: &str,
        candidates: &'a [Candidate],
        frecency: &Frecency,
        limit: usize,
    ) -> Vec<Match<'a>> {
        let query = query.trim();

        let mut matches: Vec<Match<'a>> = if query.is_empty() {
            candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| Match {
                    index,
                    item: &candidate.item,
                    score: frecency.boost(&candidate.item.id),
                    highlights: Vec::new(),
                })
                .collect()
        } else {
            let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
            candidates
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| self.score(index, &pattern, candidate, frecency))
                .collect()
        };

        // Sort by score, then by title, so that equal scores produce a stable
        // order rather than one that reshuffles between keystrokes. Visible
        // instability in the list reads as a bug even when the ranking is right.
        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.item.title.cmp(&b.item.title))
        });
        matches.truncate(limit);
        matches
    }

    fn score<'a>(
        &mut self,
        index: usize,
        pattern: &Pattern,
        candidate: &'a Candidate,
        frecency: &Frecency,
    ) -> Option<Match<'a>> {
        self.indices.clear();
        let name_score = pattern.indices(
            candidate.name.slice(..),
            &mut self.matcher,
            &mut self.indices,
        );

        let alias_score = candidate
            .aliases
            .iter()
            .filter_map(|alias| pattern.score(alias.slice(..), &mut self.matcher))
            .max()
            .map(|best| best.saturating_sub(ALIAS_PENALTY));

        let (base, highlights) = match (name_score, alias_score) {
            // The keyword is what matched, so nothing in the title is lit.
            (name, Some(alias)) if name.is_none_or(|name| alias > name) => (alias, Vec::new()),
            (Some(name), _) => {
                // `indices` can contain duplicates across pattern atoms; the UI
                // uses them for lookup, so they must be sorted and unique.
                self.indices.sort_unstable();
                self.indices.dedup();
                (name, self.indices.clone())
            }
            _ => return None,
        };

        Some(Match {
            index,
            item: &candidate.item,
            score: base + frecency.boost(&candidate.item.id),
            highlights,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Action;
    use lumen_platform::LaunchTarget;

    fn candidate(id: &str, title: &str, keywords: &[&str]) -> Candidate {
        Candidate::new(Item {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            icon: None,
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            actions: vec![Action::Launch(LaunchTarget::ShellExecute(title.into()))],
            kind: crate::item::Kind::Application,
        })
    }

    #[test]
    fn an_empty_query_lists_everything_ordered_by_use() {
        let candidates = vec![candidate("a", "Alpha", &[]), candidate("b", "Beta", &[])];
        let mut frecency = Frecency::new();
        frecency.record("b");

        let mut search = Search::new();
        let results = search.rank("", &candidates, &frecency, 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].item.id, "b");
    }

    #[test]
    fn matches_subsequences_not_just_prefixes() {
        let candidates = vec![candidate("a", "Visual Studio Code", &[])];
        let mut search = Search::new();

        let results = search.rank("vsc", &candidates, &Frecency::new(), 10);

        assert_eq!(results.len(), 1);
        assert!(!results[0].highlights.is_empty());
    }

    #[test]
    fn reports_highlight_offsets_into_the_title() {
        let candidates = vec![candidate("a", "Firefox", &[])];
        let mut search = Search::new();

        let results = search.rank("fire", &candidates, &Frecency::new(), 10);

        assert_eq!(results[0].highlights, vec![0, 1, 2, 3]);
    }

    #[test]
    fn keywords_can_find_an_entry_the_user_could_not_name() {
        let candidates = vec![candidate("a", "Windows Terminal", &["shell", "console"])];
        let mut search = Search::new();

        let results = search.rank("console", &candidates, &Frecency::new(), 10);

        assert_eq!(results.len(), 1);
        // Nothing in the visible title matched, so nothing is highlighted.
        assert!(results[0].highlights.is_empty());
    }

    /// A title that matches only by scattered letters does not bury an entry
    /// whose keyword is exactly what was typed (F9).
    #[test]
    fn an_exact_keyword_beats_a_scattered_title() {
        let candidates = vec![
            candidate("shp", "SHP Saint Helena Pound", &["Saint Helena"]),
            candidate("eur", "EUR Euro", &["Spain", "France"]),
        ];
        let mut search = Search::new();

        let results = search.rank("spain", &candidates, &Frecency::new(), 10);

        assert_eq!(results[0].item.id, "eur");
        assert!(results[0].highlights.is_empty());
    }

    #[test]
    fn an_equally_good_title_still_beats_a_keyword() {
        let candidates = vec![
            candidate("alias", "Something Else", &["terminal"]),
            candidate("title", "Terminal", &[]),
        ];
        let mut search = Search::new();

        let results = search.rank("terminal", &candidates, &Frecency::new(), 10);

        assert_eq!(results[0].item.id, "title");
    }

    /// The whole point of frecency: a worse string match that the user actually
    /// opens should win.
    #[test]
    fn usage_outranks_a_better_string_match() {
        let candidates = vec![
            candidate("services", "Component Services", &[]),
            candidate("code", "Code", &[]),
        ];
        let mut frecency = Frecency::new();
        for _ in 0..30 {
            frecency.record("code");
        }

        let mut search = Search::new();
        let results = search.rank("co", &candidates, &frecency, 10);

        assert_eq!(results[0].item.id, "code");
    }

    #[test]
    fn non_matches_are_excluded_entirely() {
        let candidates = vec![candidate("a", "Firefox", &[])];
        let mut search = Search::new();

        assert!(
            search
                .rank("zzzzz", &candidates, &Frecency::new(), 10)
                .is_empty()
        );
    }

    #[test]
    fn equal_scores_keep_a_stable_order() {
        let candidates = vec![candidate("b", "Same", &[]), candidate("a", "Same", &[])];
        let mut search = Search::new();

        let first = search.rank("same", &candidates, &Frecency::new(), 10);
        let titles: Vec<_> = first.iter().map(|m| m.item.id.clone()).collect();

        let second = search.rank("same", &candidates, &Frecency::new(), 10);
        let again: Vec<_> = second.iter().map(|m| m.item.id.clone()).collect();

        assert_eq!(titles, again);
    }

    #[test]
    fn the_limit_is_respected() {
        let candidates: Vec<_> = (0..100)
            .map(|i| candidate(&format!("id{i}"), &format!("Item {i}"), &[]))
            .collect();
        let mut search = Search::new();

        assert_eq!(
            search.rank("item", &candidates, &Frecency::new(), 8).len(),
            8
        );
    }
}
