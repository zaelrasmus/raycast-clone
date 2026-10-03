//! The names of the index, held in memory and matched by substring (F16).
//!
//! # One blob, not a million strings
//!
//! Every name, lower-cased, is written into one `String` separated by
//! newlines, with a table of where each starts. One allocation instead of one
//! per name, so the cost is the bytes of the names and four bytes an entry,
//! with no per-string header -- about 30 bytes an entry, measured in slice 0.
//! A query scans the blob with `memmem`, which answered every match of a
//! million names in under 5 ms, where SQLite's trigram index took 19.
//!
//! # Word start or substring, never fuzzy
//!
//! `inv` finds `Invoice-2026.pdf` and `2026-invoices`; `ive` scattered across
//! a name does not. A word of one or two characters must start a word of the
//! name -- `a` as a plain substring is in nearly every name and says nothing
//! -- while three characters or more may be anywhere in it.
//!
//! # Which word decides the tier
//!
//! Every word must be in the name; the entry's tier is the best place the
//! *longest* word was found. It is the most specific word typed, the one the
//! scan was driven by, and the one a user most often types out in full. The
//! only exception is the whole query naming the entry exactly, which is the
//! top tier however many words it has: `meeting notes` is `Meeting Notes.md`.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use memchr::memmem::Finder;

/// Which entries a query may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kinds {
    Folders,
    FoldersAndFiles,
}

/// One entry a query found, and what it is ranked by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// The entry's index, from 0 in the order the names were added.
    pub entry: u32,
    /// How well the name matched: 4 exactly, 3 at its start, 2 at the start
    /// of one of its words, 1 anywhere in it.
    pub tier: u8,
    /// Frecency, as the caller's `boost` gave it.
    pub boost: u32,
}

/// The shortest word allowed to match in the middle of a word of a name.
pub const MIN_SUBSTRING: usize = 3;

/// Every name of the index, lower-cased, in one blob.
#[derive(Debug, Default)]
pub struct Names {
    /// Each name folded by [`fold`] and followed by a newline.
    blob: String,
    /// Where each name starts in the blob, in entry order.
    starts: Vec<u32>,
    /// Whether each entry is a folder, one bit an entry: a `Vec<bool>` would
    /// be a megabyte at a million entries, this is an eighth of one.
    dirs: Vec<u64>,
}

impl Names {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, bool)>) -> Self {
        let mut names = Self::default();
        for (name, is_dir) in entries {
            names.push(name, is_dir);
        }
        names.shrink();
        names
    }

    /// Add the next entry. Its index is the number of entries before it.
    pub(crate) fn push(&mut self, name: &str, is_dir: bool) {
        let entry = self.starts.len();
        // Offsets fit in `u32` up to a 4 GB blob: a hundred times what the
        // ceiling's worth of names takes.
        self.starts.push(self.blob.len() as u32);
        fold(name, &mut self.blob);
        self.blob.push('\n');
        if entry / 64 == self.dirs.len() {
            self.dirs.push(0);
        }
        if is_dir {
            self.dirs[entry / 64] |= 1 << (entry % 64);
        }
    }

    /// Give back what growing by doubling over-allocated: up to half of the
    /// names again, which at the ceiling is several megabytes held for
    /// nothing.
    pub(crate) fn shrink(&mut self) {
        self.blob.shrink_to_fit();
        self.starts.shrink_to_fit();
        self.dirs.shrink_to_fit();
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// What the names hold on the heap.
    pub fn memory_bytes(&self) -> usize {
        self.blob.capacity() + self.starts.capacity() * 4 + self.dirs.capacity() * 8
    }

    pub fn is_dir(&self, entry: u32) -> bool {
        let entry = entry as usize;
        self.dirs
            .get(entry / 64)
            .is_some_and(|bits| bits & (1 << (entry % 64)) != 0)
    }

    /// The entry whose name holds the byte `hit`, searching forwards from
    /// `from`, which starts at or before it.
    ///
    /// Galloping rather than a binary search of everything after `from`:
    /// hits only move forwards, and for a short word the next one is nearly
    /// always in the next name or the one after, which this finds in a step
    /// or two instead of twenty.
    fn entry_at(&self, from: usize, hit: usize) -> usize {
        let starts = &self.starts;
        let mut low = from;
        let mut step = 1;
        loop {
            let high = low + step;
            if high >= starts.len() || starts[high] as usize > hit {
                let high = high.min(starts.len());
                return low + starts[low..high].partition_point(|&start| start as usize <= hit) - 1;
            }
            low = high;
            step *= 2;
        }
    }

    /// The folded name of `entry`, without its newline.
    fn name(&self, entry: usize) -> &str {
        let start = self.starts[entry] as usize;
        let end = self
            .starts
            .get(entry + 1)
            .map_or(self.blob.len(), |&next| next as usize);
        &self.blob[start..end - 1]
    }

    /// The best `limit` entries whose names hold every word of `query`, best
    /// first. `boost` is asked at most once for an entry, and only for one
    /// that could still rank.
    ///
    /// The longest word drives the scan: it is in the fewest names, so it
    /// leaves the fewest to check the other words against. A name is looked
    /// up once, at its first hit; later hits in it are passed over by offset
    /// alone. Only the best `limit` are kept while scanning, in a heap whose
    /// top is the worst of them, so the matches are never all sorted, and a
    /// match whose tier is below the worst kept goes no further.
    pub fn find(
        &self,
        query: &str,
        kinds: Kinds,
        limit: usize,
        boost: impl Fn(u32) -> u32,
    ) -> Vec<Match> {
        let words = words(query);
        if words.is_empty() || limit == 0 {
            return Vec::new();
        }
        // The first of the longest, so the choice is the same every time.
        let longest = (0..words.len())
            .rev()
            .max_by_key(|&index| words[index].len())
            .expect("at least one word");
        let others = words
            .iter()
            .enumerate()
            .filter(|&(index, _)| index != longest)
            .map(|(_, word)| word.as_str())
            .collect::<Vec<_>>();
        let whole = words.join(" ");

        let word = words[longest].as_bytes();
        let blob = self.blob.as_bytes();
        let mut heap = BinaryHeap::with_capacity(limit.min(self.len()) + 1);
        // Where the name after the last one looked at starts: a hit before
        // it is a second hit in the same name.
        let mut next = 0;
        let mut entry = 0;

        let mut consider = |hit: usize| {
            if hit < next {
                return;
            }
            entry = self.entry_at(entry, hit);
            let start = self.starts[entry] as usize;
            let name = self.name(entry);
            next = start + name.len() + 1;

            if kinds == Kinds::Folders && !self.is_dir(entry as u32) {
                return;
            }
            // This is the word's first place in the name, so it starts the
            // name or nothing does; and a word start is as good as any later
            // place can be. Only a plain substring has to look further on.
            let tier = if hit == start {
                3
            } else if starts_word(blob[hit - 1])
                || best_occurrence(name, &words[longest], hit - start + 1)
                    .is_some_and(|(_, tier)| tier >= 2)
            {
                2
            } else {
                // Only a word of three or more gets here: a shorter one's
                // hits inside a word were never considered.
                1
            };
            let tier = if is_exact(name, &whole) { 4 } else { tier };
            // Tier ranks first: below the worst kept, nothing else matters.
            if heap.len() == limit
                && heap
                    .peek()
                    .is_some_and(|worst: &Ranked| tier < worst.found.tier)
            {
                return;
            }
            if !others.iter().all(|word| word_tier(name, word).is_some()) {
                return;
            }

            let candidate = Ranked {
                found: Match {
                    entry: entry as u32,
                    tier,
                    boost: boost(entry as u32),
                },
                length: name.len() as u32,
            };
            if heap.len() < limit {
                heap.push(candidate);
            } else if heap.peek().is_some_and(|worst| candidate < *worst) {
                heap.pop();
                heap.push(candidate);
            }
        };

        // A short word counts only where it starts a word, which the blob
        // shows without looking up the entry: the byte before it is a
        // separator, or the newline that ends the previous name. Most hits
        // of `a` are inside a word, and cost one comparison.
        let short = words[longest].chars().count() < MIN_SUBSTRING;
        let at_word_start = |hit: usize| hit == 0 || starts_word(blob[hit - 1]);
        if word.len() == 1 {
            // A plain `memchr`: `memmem`'s iterator costs more per hit than
            // this whole check, and one byte has hits in nearly every name.
            for hit in memchr::memchr_iter(word[0], blob) {
                if at_word_start(hit) {
                    consider(hit);
                }
            }
        } else {
            for hit in Finder::new(word).find_iter(blob) {
                if !short || at_word_start(hit) {
                    consider(hit);
                }
            }
        }

        heap.into_sorted_vec()
            .into_iter()
            .map(|ranked| ranked.found)
            .collect()
    }
}

/// A match with what it is ordered by. *Less* is *better*, so the top of a
/// max-heap is the worst kept and `into_sorted_vec` is best first.
#[derive(PartialEq, Eq)]
struct Ranked {
    found: Match,
    length: u32,
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .found
            .tier
            .cmp(&self.found.tier)
            .then(other.found.boost.cmp(&self.found.boost))
            .then(self.length.cmp(&other.length))
            .then(self.found.entry.cmp(&other.found.entry))
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The words of a query, folded as the names are.
pub(crate) fn words(query: &str) -> Vec<String> {
    let mut folded = String::new();
    fold(query, &mut folded);
    folded.split_whitespace().map(str::to_string).collect()
}

/// Lower-case `text` into `out` a character at a time, with every newline
/// made a space so it cannot split a name in the blob.
///
/// A character at a time rather than `str::to_lowercase`, which folds a
/// final `Σ` by its context: the highlights map a title back character by
/// character, and must fold it exactly as the blob did.
pub(crate) fn fold(text: &str, out: &mut String) {
    for character in text.chars() {
        if character == '\n' {
            out.push(' ');
        } else {
            out.extend(character.to_lowercase());
        }
    }
}

/// Whether `query` is the whole name, or the name without its extension.
/// A leading dot is not an extension: `.env` is its own stem.
fn is_exact(name: &str, query: &str) -> bool {
    let Some(rest) = name.strip_prefix(query) else {
        return false;
    };
    rest.is_empty() || (!query.is_empty() && rest.starts_with('.') && !rest[1..].contains('.'))
}

/// The best place `word` is in `name` at or after the byte `from`: its byte
/// offset and its tier, 3 at the start, 2 at the start of a word, 1
/// anywhere. The first of the best,
/// so a highlight lands where a reader looks first.
///
/// A word starts after a space, `-`, `_`, `.`, `(` or `[`. Not after a
/// change from letters to digits or from lower to upper case: the names are
/// folded, and `inv2024` is rare enough not to pay for it.
///
/// Found by the word's first byte and compared from there, not with a
/// `memmem` searcher: names are a few dozen bytes, and building a searcher
/// for each one costs more than the search.
pub(crate) fn best_occurrence(name: &str, word: &str, from: usize) -> Option<(usize, u8)> {
    let bytes = name.as_bytes();
    let word = word.as_bytes();
    let last = bytes.len().checked_sub(word.len())?;
    let mut best: Option<(usize, u8)> = None;
    for at in memchr::memchr_iter(*word.first()?, bytes.get(from..=last)?) {
        let at = from + at;
        if !bytes[at..].starts_with(word) {
            continue;
        }
        let tier = match at {
            0 => 3,
            _ if starts_word(bytes[at - 1]) => 2,
            _ => 1,
        };
        if best.is_none_or(|(_, kept)| tier > kept) {
            best = Some((at, tier));
            if tier == 3 {
                break;
            }
        }
    }
    best
}

/// Whether a word starts after `byte`. The newline is the end of the name
/// before, in the blob.
fn starts_word(byte: u8) -> bool {
    matches!(byte, b' ' | b'-' | b'_' | b'.' | b'(' | b'[' | b'\n')
}

/// The tier `word` earns in `name`, or `None` when it is not there -- or is
/// there only inside a word and is too short for that to mean anything.
pub(crate) fn word_tier(name: &str, word: &str) -> Option<u8> {
    let (_, tier) = best_occurrence(name, word, 0)?;
    (tier >= 2 || word.chars().count() >= MIN_SUBSTRING).then_some(tier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[(&str, bool)]) -> Names {
        Names::from_entries(entries.iter().copied())
    }

    fn found(names: &Names, query: &str) -> Vec<u32> {
        names
            .find(query, Kinds::FoldersAndFiles, 64, |_| 0)
            .iter()
            .map(|found| found.entry)
            .collect()
    }

    fn tier(names: &Names, query: &str) -> Option<u8> {
        names
            .find(query, Kinds::FoldersAndFiles, 64, |_| 0)
            .first()
            .map(|found| found.tier)
    }

    #[test]
    fn the_tiers_are_exact_start_word_start_and_substring() {
        let one = |name: &str| names(&[(name, false)]);

        assert_eq!(tier(&one("invoice"), "invoice"), Some(4));
        assert_eq!(tier(&one("Invoice.pdf"), "invoice"), Some(4));
        assert_eq!(tier(&one("Invoice-2026.pdf"), "inv"), Some(3));
        assert_eq!(tier(&one("2026-invoices"), "inv"), Some(2));
        assert_eq!(tier(&one("old (invoices)"), "inv"), Some(2));
        assert_eq!(tier(&one("myinvoice.pdf"), "inv"), Some(1));
    }

    /// The best place in the name counts, not the first: `report` is a word
    /// start in `areport report.txt` even though it is a substring first.
    #[test]
    fn the_best_occurrence_sets_the_tier() {
        assert_eq!(
            tier(&names(&[("areport report.txt", false)]), "report"),
            Some(2)
        );
    }

    #[test]
    fn matching_ignores_case_on_both_sides() {
        let names = names(&[("README.md", false)]);
        assert_eq!(found(&names, "readme"), [0]);
        assert_eq!(found(&names, "ReadMe"), [0]);
    }

    /// `a` is in nearly every name; as a plain substring it says nothing.
    #[test]
    fn a_short_word_must_start_a_word_of_the_name() {
        let names = names(&[
            ("banana.txt", false),
            ("a-team.txt", false),
            ("my app", false),
        ]);

        assert_eq!(found(&names, "a"), [1, 2]);
        assert_eq!(found(&names, "an"), Vec::<u32>::new());
        assert_eq!(found(&names, "ana"), [0]);
    }

    #[test]
    fn every_word_must_be_in_the_name() {
        let names = names(&[
            ("invoice 2024.pdf", false),
            ("invoice 2023.pdf", false),
            ("2024 taxes", false),
        ]);

        assert_eq!(found(&names, "inv 2024"), [0]);
        assert_eq!(found(&names, "2024 inv"), [0]);
    }

    /// A short word in a query of several still has to start a word.
    #[test]
    fn the_short_word_rule_holds_in_a_query_of_several_words() {
        let names = names(&[("invoice ab.pdf", false), ("invoice cab.pdf", false)]);
        assert_eq!(found(&names, "invoice ab"), [0]);
    }

    #[test]
    fn the_whole_query_naming_the_entry_is_exact() {
        let names = names(&[("Meeting Notes.md", false)]);
        assert_eq!(tier(&names, "meeting notes"), Some(4));
    }

    #[test]
    fn an_empty_or_blank_query_finds_nothing() {
        let names = names(&[("anything", false)]);
        assert!(found(&names, "").is_empty());
        assert!(found(&names, "   ").is_empty());
    }

    #[test]
    fn folders_only_keeps_the_folders() {
        let names = names(&[("projects", true), ("projects.txt", false)]);

        let folders = names.find("proj", Kinds::Folders, 64, |_| 0);
        assert_eq!(folders.iter().map(|m| m.entry).collect::<Vec<_>>(), [0]);
        assert_eq!(found(&names, "proj"), [0, 1]);
    }

    /// Tier first, then frecency, then the shorter name, then the order of
    /// the index.
    #[test]
    fn results_are_ranked_by_tier_then_boost_then_length() {
        let names = names(&[
            ("my report.txt", false),       // 0: word start
            ("report-final-v2.txt", false), // 1: start, long
            ("report.txt", false),          // 2: exact without extension
            ("report-v2.txt", false),       // 3: start, short
            ("areport.txt", false),         // 4: substring
            ("report-old.txt", false),      // 5: start, same length as 6
            ("report-new.txt", false),      // 6
        ]);

        assert_eq!(found(&names, "report"), [2, 3, 5, 6, 1, 0, 4]);

        let boosted = names.find("report", Kinds::FoldersAndFiles, 64, |entry| {
            u32::from(entry == 6) * 10
        });
        let order = boosted.iter().map(|m| m.entry).collect::<Vec<_>>();
        assert_eq!(order, [2, 6, 3, 5, 1, 0, 4]);
        assert_eq!(boosted[1].boost, 10);
    }

    #[test]
    fn the_limit_keeps_the_best() {
        let entries = (0..100)
            .map(|index| (format!("file {index:03}.txt"), false))
            .chain([("file.txt".to_string(), false)])
            .collect::<Vec<_>>();
        let names = Names::from_entries(entries.iter().map(|(name, dir)| (name.as_str(), *dir)));

        let top = names.find("file", Kinds::FoldersAndFiles, 5, |_| 0);
        assert_eq!(top.len(), 5);
        assert_eq!(top[0].entry, 100);
        assert_eq!(
            top[1..].iter().map(|m| m.entry).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert!(
            names
                .find("file", Kinds::FoldersAndFiles, 0, |_| 0)
                .is_empty()
        );
    }

    /// A newline in a name -- legal on Linux -- must not split it in two.
    #[test]
    fn a_newline_in_a_name_does_not_split_it() {
        let names = names(&[("two\nlines", false), ("other", false)]);
        assert_eq!(names.len(), 2);
        assert_eq!(found(&names, "lines"), [0]);
        assert_eq!(found(&names, "other"), [1]);
    }

    #[test]
    fn length_and_memory_are_reported() {
        let names = names(&[("one", false), ("two", true)]);
        assert_eq!(names.len(), 2);
        assert!(names.memory_bytes() >= "one\ntwo\n".len() + 2 * 4);
    }
}
