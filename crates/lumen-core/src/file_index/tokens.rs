//! The words a name is indexed as on disk, and how a query meets them (F16,
//! step c2).
//!
//! # Whole words and their parts
//!
//! A name is cut into runs of letters and digits -- everything else, space
//! `-` `_` `.` `(` `[` and the rest, separates them -- and each run is
//! indexed whole. A run that changes to a capital, or to a digit, inside is
//! indexed in parts as well: `AnnualReport2024` is `annualreport2024 annual
//! report 2024`, so `report` and `2024` find it. Both, not the parts alone:
//! the c1 bench found that splitting alone loses what is typed out whole
//! across a capital -- `javascript` stopped finding `JavaScript`, `github`
//! `GitHub`. Runs of lower-case letters stay one word (`myreport`): finding
//! inside them is the substring search's job.
//!
//! # A query
//!
//! A query's words are folded as the names are and never split at a digit
//! or a capital -- it has none -- only at what separates runs: `my-inv` asks
//! for `my` and `inv`. A name matches when every word of the query is in its
//! folded name and each of the word's runs starts one of its tokens. The
//! first condition keeps `c++` from finding `cargo` through its run `c`; for
//! a word of letters and digits alone it is implied by the second.

use super::names::fold;

/// Append the tokens of `name` to `out`, folded and separated by spaces (see
/// "Whole words and their parts" above).
pub(crate) fn tokens(name: &str, out: &mut String) {
    let mut run: Vec<char> = Vec::new();
    for character in name.chars().chain(std::iter::once(' ')) {
        if character.is_alphanumeric() {
            run.push(character);
            continue;
        }
        if run.is_empty() {
            continue;
        }
        push_word(&run, out);
        let cuts = (1..run.len())
            .filter(|&at| starts_part(run[at - 1], run[at]))
            .collect::<Vec<_>>();
        if !cuts.is_empty() {
            let mut from = 0;
            for at in cuts.into_iter().chain(std::iter::once(run.len())) {
                push_word(&run[from..at], out);
                from = at;
            }
        }
        run.clear();
    }
}

/// Whether a part starts at `character`, after `previous`: a capital after
/// anything but a capital, or a digit after anything but a digit.
fn starts_part(previous: char, character: char) -> bool {
    (character.is_uppercase() && !previous.is_uppercase())
        || (character.is_numeric() && !previous.is_numeric())
}

fn push_word(characters: &[char], out: &mut String) {
    if !out.is_empty() {
        out.push(' ');
    }
    let mut buffer = [0; 4];
    for character in characters {
        fold(character.encode_utf8(&mut buffer), out);
    }
}

/// The runs of letters and digits of a folded query word: what must each
/// start a token.
pub(crate) fn runs(word: &str) -> impl Iterator<Item = &str> {
    word.split(|character: char| !character.is_alphanumeric())
        .filter(|run| !run.is_empty())
}

/// Whether a name, `folded` and with its `tokens`, holds every word of a
/// query (see "A query" above).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "asked by step c2b's ranked query")
)]
pub(crate) fn matches(words: &[String], folded: &str, tokens: &str) -> bool {
    words.iter().all(|word| {
        folded.contains(word.as_str())
            && runs(word).all(|run| tokens.split(' ').any(|token| token.starts_with(run)))
    })
}

/// The word index's query for `words`: every run of every word as a token
/// prefix, all required. `None` when the words have no letter or digit, which
/// the word index cannot be asked about.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "asked by step c2b's ranked query")
)]
pub(crate) fn fts_query(words: &[String]) -> Option<String> {
    let mut runs = words.iter().flat_map(|word| runs(word)).collect::<Vec<_>>();
    runs.sort_unstable();
    runs.dedup();
    (!runs.is_empty()).then(|| {
        runs.iter()
            .map(|run| format!("\"{run}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(name: &str) -> String {
        let mut out = String::new();
        tokens(name, &mut out);
        out
    }

    fn words(query: &str) -> Vec<String> {
        super::super::names::words(query)
    }

    #[test]
    fn a_name_is_its_whole_words_and_their_parts() {
        assert_eq!(of("JavaScript"), "javascript java script");
        assert_eq!(of("GitHub"), "github git hub");
        assert_eq!(
            of("AnnualReport2024"),
            "annualreport2024 annual report 2024"
        );
        assert_eq!(of("es2024"), "es2024 es 2024");
        assert_eq!(of("my-invoice_v2.final.pdf"), "my invoice v2 v 2 final pdf");
    }

    /// A run of capitals is one part, and lower-case letters run together
    /// stay one word.
    #[test]
    fn capitals_together_and_lower_case_together_stay_one_word() {
        assert_eq!(of("HTMLParser"), "htmlparser");
        assert_eq!(of("myreport (old)"), "myreport old");
        assert_eq!(of("İstanbul"), "i\u{307}stanbul");
        assert_eq!(of("..."), "");
    }

    #[test]
    fn a_query_matches_whole_words_and_parts_by_their_start() {
        let name = "JavaScript Notes 2024.md";
        let (folded, split) = (name.to_lowercase(), of(name));
        let found = |query: &str| matches(&words(query), &folded, &split);

        assert!(found("javascript"));
        assert!(found("java"));
        assert!(found("script"));
        assert!(found("scr not 20"));
        assert!(!found("ascript"));
        assert!(!found("javascript 2023"));
    }

    /// A word with a separator in it must be in the name as typed: `c++`
    /// does not find `cargo` through its `c`.
    #[test]
    fn a_word_with_a_separator_must_be_in_the_name_as_typed() {
        let cargo = ("cargo.toml", of("cargo.toml"));
        let plus = ("c++ notes", of("c++ notes"));
        let mine = ("my-invoice.pdf", of("my-invoice.pdf"));

        assert!(!matches(&words("c++"), cargo.0, &cargo.1));
        assert!(matches(&words("c++"), plus.0, &plus.1));
        assert!(matches(&words("my-inv"), mine.0, &mine.1));
        assert!(!matches(&words("my inv-"), mine.0, &mine.1));
    }

    #[test]
    fn the_word_index_is_asked_for_every_run_as_a_prefix() {
        assert_eq!(
            fts_query(&words("inv 2024")).as_deref(),
            Some("\"2024\"* AND \"inv\"*")
        );
        assert_eq!(
            fts_query(&words("my-inv my")).as_deref(),
            Some("\"inv\"* AND \"my\"*")
        );
        assert_eq!(fts_query(&words("++ ..")), None);
    }
}
