//! One alias table for every command (F17).
//!
//! An alias is a short word that names one command: `lh` for *Left Half*.
//! Typed whole, the command leads the list; typed in part, from its start,
//! it is lifted above the fuzzy matches but stays below a whole one. Never
//! matched fuzzily: an alias is chosen to be typed, and `lh` turning up for
//! `l...h` in everything else would make every short alias noise.
//!
//! The user's aliases (`shortcuts.json`) and the ones providers declare --
//! a timer preset's, a quicklink's, a translate preset's -- are one table,
//! built when any of them changes, never per keystroke. A provider may also
//! declare a *name*: typed whole it leads, as an alias does, but a part of it
//! is only a fuzzy match like any title's.

use std::collections::HashMap;

/// One key of the table and the item it names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Folded: trimmed and in lowercase.
    key: String,
    id: String,
    /// Whether typing the start of `key` lifts the item: an alias's does, a
    /// name's does not.
    prefix: bool,
}

/// Every alias and name, sorted by key so a typed query finds the keys it
/// starts by a binary search, whatever the table's size.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aliases {
    entries: Vec<Entry>,
    /// Each item's first alias, for the badge beside its title. The user's,
    /// when they gave one, because it is the one they chose.
    shown: HashMap<String, String>,
}

/// What a query names: the items it names whole, then those whose alias it
/// begins, each without the other's.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Matches<'a> {
    pub exact: Vec<&'a str>,
    pub prefix: Vec<&'a str>,
}

impl Aliases {
    /// The table for `aliases` and `names`, each `(item id, text)`, in the
    /// order they should be shown when an item has more than one alias.
    pub fn build(
        aliases: impl IntoIterator<Item = (String, String)>,
        names: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut shown = HashMap::new();
        let mut entries = Vec::new();
        for (id, alias) in aliases {
            let key = fold(&alias);
            if key.is_empty() {
                continue;
            }
            shown.entry(id.clone()).or_insert_with(|| key.clone());
            entries.push(Entry {
                key,
                id,
                prefix: true,
            });
        }
        for (id, name) in names {
            let key = fold(&name);
            if !key.is_empty() {
                entries.push(Entry {
                    key,
                    id,
                    prefix: false,
                });
            }
        }
        // The alias before a name with the same key and item, so that the
        // one kept of the two is the alias, whose start still counts.
        entries.sort_by(|a, b| (&a.key, &a.id, !a.prefix).cmp(&(&b.key, &b.id, !b.prefix)));
        entries.dedup_by(|later, kept| later.key == kept.key && later.id == kept.id);
        Self { entries, shown }
    }

    /// The items `query` names whole or by the start of an alias.
    ///
    /// The keys `query` begins are one run of the sorted table, found by a
    /// binary search: the work is the matching keys, never the whole table.
    /// The one allocation per call is the folded query.
    pub fn matches(&self, query: &str) -> Matches<'_> {
        let query = fold(query);
        let mut matches = Matches::default();
        if query.is_empty() {
            return matches;
        }
        let start = self
            .entries
            .partition_point(|entry| entry.key.as_str() < query.as_str());
        for entry in self.entries[start..]
            .iter()
            .take_while(|entry| entry.key.starts_with(&query))
        {
            if entry.key == query {
                matches.exact.push(entry.id.as_str());
            } else if entry.prefix {
                matches.prefix.push(entry.id.as_str());
            }
        }
        matches.exact.sort_unstable();
        matches.exact.dedup();
        matches.prefix.sort_unstable();
        matches.prefix.dedup();
        matches
            .prefix
            .retain(|id| matches.exact.binary_search(id).is_err());
        matches
    }

    /// The alias shown beside `id`'s title, if it has one.
    pub fn shown(&self, id: &str) -> Option<&str> {
        self.shown.get(id).map(String::as_str)
    }
}

/// A key as it is compared: trimmed, and in lowercase by ASCII's rule, as the
/// providers always compared their aliases and names. The user's aliases are
/// already stored so ([`crate::shortcuts::normalise_alias`]).
fn fold(text: &str) -> String {
    text.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(id, text)| (id.to_string(), text.to_string()))
            .collect()
    }

    fn table() -> Aliases {
        Aliases::build(
            pairs(&[
                ("window:left-half", "lh"),
                ("window:left-third", "lht"),
                ("app:code", "code"),
            ]),
            pairs(&[("quicklink:Google", "Google")]),
        )
    }

    #[test]
    fn a_whole_alias_is_exact_and_its_start_a_prefix() {
        let table = table();
        assert_eq!(
            table.matches("lh"),
            Matches {
                exact: vec!["window:left-half"],
                prefix: vec!["window:left-third"],
            }
        );
        assert_eq!(
            table.matches("l"),
            Matches {
                exact: vec![],
                prefix: vec!["window:left-half", "window:left-third"],
            }
        );
    }

    #[test]
    fn case_and_surrounding_space_do_not_matter() {
        assert_eq!(table().matches("  LH ").exact, ["window:left-half"]);
    }

    #[test]
    fn nothing_matches_an_alias_fuzzily() {
        let table = table();
        assert_eq!(table.matches("cde"), Matches::default());
        assert_eq!(table.matches("lhx"), Matches::default());
        assert_eq!(table.matches(""), Matches::default());
        assert_eq!(table.matches("   "), Matches::default());
    }

    #[test]
    fn a_name_leads_only_when_typed_whole() {
        let table = table();
        assert_eq!(table.matches("google").exact, ["quicklink:Google"]);
        assert_eq!(table.matches("goog"), Matches::default());
    }

    #[test]
    fn an_item_named_twice_is_listed_once() {
        let table = Aliases::build(
            pairs(&[("timer:preset:Zeta", "zq"), ("timer:preset:Zeta", "zqq")]),
            pairs(&[("timer:preset:Zeta", "zq")]),
        );
        assert_eq!(
            table.matches("zq"),
            Matches {
                exact: vec!["timer:preset:Zeta"],
                prefix: vec![],
            }
        );
        // A name the same as an alias does not take the alias's start away.
        let table = Aliases::build(
            pairs(&[("quicklink:Gh", "gh")]),
            pairs(&[("quicklink:Gh", "GH")]),
        );
        assert_eq!(table.matches("g").prefix, ["quicklink:Gh"]);
    }

    #[test]
    fn the_first_alias_given_is_the_one_shown() {
        let table = Aliases::build(
            pairs(&[("timer:preset:Zeta", "mine"), ("timer:preset:Zeta", "zq")]),
            pairs(&[("quicklink:Google", "Google")]),
        );
        assert_eq!(table.shown("timer:preset:Zeta"), Some("mine"));
        // A name is not an alias, and has no badge.
        assert_eq!(table.shown("quicklink:Google"), None);
    }
}
