//! Finding an entry by what it is called.
//!
//! Three tiers, in order: a name or keyword that is the query -- the
//! user's own keywords before Unicode's -- then names
//! and keywords whose words start with the query's words, then whatever
//! `nucleo-matcher` finds -- the same matcher as the root list, for a typo.
//! Within a tier the catalogue's order stands, so *joy* lists 😂 before
//! 😹 as Unicode does, and a query gives the same grid every time.
//!
//! A linear pass over a few thousand prepared strings, as the root search
//! is: nothing to index, and well inside the frame budget.

use std::cmp::Reverse;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{Catalogue, Category, CustomKeywords, Entry};
use crate::places::fold;

/// How an entry matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// The name is the query -- *sun* is ☀️ -- or the query is the
    /// character itself.
    Name,
    /// One of the user's own keywords is the query. Above Unicode's: who
    /// gave ☀️ *sunny* means it, and 🫡 has it too.
    Custom,
    /// A keyword is the query. Below a name, or *sun* would find 😎 first:
    /// it is one of its keywords, and Unicode lists faces before the sky.
    Keyword,
    /// Every word of the query starts a word of the name or a keyword.
    Prefix,
    Fuzzy,
}

impl Catalogue {
    /// The entries `query` finds, best first, within `category` if one is
    /// given. An empty query finds every entry, in the catalogue's order.
    ///
    /// `custom` holds the user's own keywords, matched as the entry's are.
    pub fn search(
        &self,
        query: &str,
        category: Option<Category>,
        custom: &CustomKeywords,
    ) -> Vec<&Entry> {
        let within = self
            .entries
            .iter()
            .filter(|entry| category.is_none_or(|category| entry.category == category));

        let query = query.trim();
        if query.is_empty() {
            return within.collect();
        }

        let folded = fold(query);
        let words: Vec<&str> = folded.split(' ').filter(|w| !w.is_empty()).collect();
        // A symbol folds to nothing, so a query of one can only be the
        // character itself.
        let pattern = (!folded.is_empty())
            .then(|| Pattern::parse(&folded, CaseMatching::Ignore, Normalization::Smart));
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();

        let mut found: Vec<(Tier, Reverse<u32>, &Entry)> = within
            .filter_map(|entry| {
                let custom: Vec<String> = custom
                    .get(&*entry.chars)
                    .map(|keywords| keywords.iter().map(|k| fold(k)).collect())
                    .unwrap_or_default();
                let keywords = || {
                    entry
                        .folded_keywords
                        .iter()
                        .map(|k| &**k)
                        .chain(custom.iter().map(String::as_str))
                };

                if same_character(&entry.chars, query) || *entry.folded_name == *folded {
                    return Some((Tier::Name, Reverse(0), entry));
                }
                if folded.is_empty() {
                    return None;
                }
                if custom.contains(&folded) {
                    return Some((Tier::Custom, Reverse(0), entry));
                }
                if keywords().any(|keyword| keyword == folded) {
                    return Some((Tier::Keyword, Reverse(0), entry));
                }
                if words.iter().all(|word| {
                    std::iter::once(&*entry.folded_name)
                        .chain(keywords())
                        .any(|text| text.split(' ').any(|own| own.starts_with(word)))
                }) {
                    return Some((Tier::Prefix, Reverse(0), entry));
                }

                let pattern = pattern.as_ref()?;
                std::iter::once(&*entry.folded_name)
                    .chain(keywords())
                    .filter_map(|text| {
                        pattern.score(Utf32Str::new(text, &mut buffer), &mut matcher)
                    })
                    .max()
                    .map(|score| (Tier::Fuzzy, Reverse(score), entry))
            })
            .collect();

        // Stable, so equals keep the catalogue's order.
        found.sort_by_key(|(tier, score, _)| (*tier, *score));
        found.into_iter().map(|(_, _, entry)| entry).collect()
    }
}

/// Whether `query` is `chars`, with or without the emoji variation
/// selector: ☀ typed plainly is still ☀️.
fn same_character(chars: &str, query: &str) -> bool {
    let plain = |text: &str| {
        text.chars()
            .filter(|c| *c != '\u{FE0F}')
            .collect::<String>()
    };
    chars == query || plain(chars) == plain(query)
}

#[cfg(test)]
mod tests {
    use super::super::catalogue;
    use super::*;

    fn found(query: &str) -> Vec<&'static str> {
        catalogue()
            .search(query, None, &CustomKeywords::new())
            .into_iter()
            .map(|entry| &*entry.chars)
            .collect()
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let catalogue = catalogue();
        let all = catalogue.search("  ", None, &CustomKeywords::new());
        assert_eq!(all.len(), catalogue.entries().len());
        assert!(
            all.iter()
                .zip(catalogue.entries())
                .all(|(found, entry)| std::ptr::eq(*found, entry))
        );
    }

    #[test]
    fn an_exact_name_comes_first() {
        assert_eq!(found("sun").first(), Some(&"\u{2600}\u{FE0F}"));
    }

    #[test]
    fn an_exact_keyword_ranks_near_the_top() {
        let joy = found("joy");
        assert!(
            joy.iter().take(3).any(|chars| *chars == "\u{1F602}"),
            "{:?}",
            &joy[..joy.len().min(5)]
        );
    }

    #[test]
    fn a_word_prefix_finds_and_a_typo_still_finds() {
        assert!(found("grinn").contains(&"\u{1F600}"));
        assert!(found("rightwards arr").contains(&"\u{2192}"));
        // Fuzzy, last: no word of *grinning face* starts with this.
        assert!(found("grnng").contains(&"\u{1F600}"));
        assert!(found("zzqqxxj").is_empty());
    }

    #[test]
    fn a_category_restricts_the_results() {
        let catalogue = catalogue();
        let none = CustomKeywords::new();

        let arrows = catalogue.search("", Some(Category::Arrows), &none);
        assert!(!arrows.is_empty());
        assert!(arrows.iter().all(|e| e.category == Category::Arrows));

        let everywhere = catalogue.search("smile", None, &none);
        let smileys = catalogue.search("smile", Some(Category::SmileysPeople), &none);
        assert!(!smileys.is_empty());
        assert!(smileys.len() < everywhere.len());
        assert!(
            smileys
                .iter()
                .all(|e| e.category == Category::SmileysPeople)
        );
    }

    #[test]
    fn the_same_query_gives_the_same_order() {
        for query in ["smile", "heart", "arrow", "sun", "x"] {
            assert_eq!(found(query), found(query), "{query}");
        }
    }

    #[test]
    fn a_custom_keyword_finds_its_entry() {
        let catalogue = catalogue();
        assert!(found("lumenparty").is_empty());

        let mut custom = CustomKeywords::new();
        custom.insert("\u{1F389}".into(), vec!["lumenparty".into()]);
        let party = catalogue.search("lumenparty", None, &custom);
        assert_eq!(party.first().map(|e| &*e.chars), Some("\u{1F389}"));
    }

    #[test]
    fn an_exact_custom_keyword_ranks_as_an_exact_keyword() {
        let catalogue = catalogue();
        let mut custom = CustomKeywords::new();
        custom.insert("\u{2600}\u{FE0F}".into(), vec!["sunny".into()]);
        let sunny = catalogue.search("sunny", None, &custom);
        assert_eq!(sunny.first().map(|e| &*e.chars), Some("\u{2600}\u{FE0F}"));
        // Without it, 🫡 leads: *sunny* is one of Unicode's keywords for it.
        assert_ne!(found("sunny").first(), Some(&"\u{2600}\u{FE0F}"));
    }

    #[test]
    fn the_character_itself_finds_its_entry() {
        assert_eq!(found("\u{2192}").first(), Some(&"\u{2192}"));
        assert_eq!(found("\u{2600}").first(), Some(&"\u{2600}\u{FE0F}"));
    }
}
