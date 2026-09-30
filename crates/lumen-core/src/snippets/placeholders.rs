//! The placeholders offered while writing a snippet (F11).
//!
//! What the editor's picker lists when `{` is typed, in Raycast's order: the
//! placeholders, then dates and times. Each inserts its syntax and leaves the
//! caret where the next thing is typed -- inside `name=""` for an argument --
//! so choosing one never means remembering how it is spelled.

/// Where an entry is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Placeholders,
    DateAndTime,
}

impl Group {
    /// The header above the group, or none for the first.
    pub fn title(self) -> Option<&'static str> {
        match self {
            Group::Placeholders => None,
            Group::DateAndTime => Some("Date & Time"),
        }
    }
}

/// One entry of the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub title: &'static str,
    pub group: Group,
    /// What is inserted, from its `{` to its `}`.
    pub insert: &'static str,
    /// Where the caret is left, as a byte offset into `insert`.
    pub caret: usize,
    /// Also matched when filtering, never shown.
    pub keywords: &'static [&'static str],
}

const fn at_end(title: &'static str, group: Group, insert: &'static str) -> Choice {
    Choice {
        title,
        group,
        insert,
        caret: insert.len(),
        keywords: &[],
    }
}

/// Inside the empty quotes before the closing brace: `{x name="|"}`.
const fn in_quotes(
    title: &'static str,
    insert: &'static str,
    keywords: &'static [&'static str],
) -> Choice {
    Choice {
        title,
        group: Group::Placeholders,
        insert,
        caret: insert.len() - 2,
        keywords,
    }
}

/// Every entry, in the order listed.
pub const CHOICES: &[Choice] = &[
    in_quotes(
        "Calculator",
        "{calculator expression=\"\"}",
        &["math", "sum"],
    ),
    Choice {
        keywords: &["id", "random"],
        ..at_end("UUID", Group::Placeholders, "{uuid}")
    },
    Choice {
        keywords: &["copied", "paste"],
        ..at_end("Clipboard Text", Group::Placeholders, "{clipboard}")
    },
    in_quotes("Argument", "{argument name=\"\"}", &["input", "ask"]),
    Choice {
        keywords: &["caret"],
        ..at_end("Cursor Position", Group::Placeholders, "{cursor}")
    },
    in_quotes("Snippet", "{snippet name=\"\"}", &["insert"]),
    Choice {
        keywords: &["today"],
        ..at_end("Date", Group::DateAndTime, "{date}")
    },
    Choice {
        keywords: &["now", "clock"],
        ..at_end("Time", Group::DateAndTime, "{time}")
    },
    Choice {
        keywords: &["now", "datetime"],
        ..at_end("Date & Time", Group::DateAndTime, "{datetime}")
    },
    Choice {
        keywords: &["day", "weekday"],
        ..at_end("Weekday", Group::DateAndTime, "{day}")
    },
];

/// The entries for what was typed after `{`, in the order listed: every word
/// of `query` starts a word of the title or of a keyword, or is inside the
/// placeholder's name.
pub fn matching(query: &str) -> Vec<&'static Choice> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    CHOICES
        .iter()
        .filter(|choice| {
            let name = choice.insert.trim_start_matches('{').to_lowercase();
            let starts = choice
                .title
                .split(|c: char| !c.is_alphanumeric())
                .chain(choice.keywords.iter().copied())
                .map(str::to_lowercase)
                .collect::<Vec<_>>();
            words.iter().all(|word| {
                name.starts_with(word.as_str())
                    || starts.iter().any(|start| start.starts_with(word.as_str()))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::template::Template;

    /// Each one is a real placeholder once what goes at the caret is typed.
    #[test]
    fn every_choice_completes_to_a_placeholder() {
        for choice in CHOICES {
            let mut text = choice.insert.to_string();
            if choice.caret < text.len() {
                text.insert(choice.caret, 'x');
            }
            assert!(
                Template::parse(&text).has_placeholders(),
                "{} inserts {text}",
                choice.title
            );
        }
    }

    #[test]
    fn the_caret_goes_inside_the_quotes_or_at_the_end() {
        let argument = CHOICES.iter().find(|c| c.title == "Argument").unwrap();
        assert_eq!(&argument.insert[argument.caret..], "\"}");
        let uuid = CHOICES.iter().find(|c| c.title == "UUID").unwrap();
        assert_eq!(uuid.caret, uuid.insert.len());
    }

    fn titles(query: &str) -> Vec<&'static str> {
        matching(query).iter().map(|choice| choice.title).collect()
    }

    #[test]
    fn filtering_by_title_keyword_or_name() {
        assert_eq!(titles("").len(), CHOICES.len());
        assert_eq!(titles("clip"), ["Clipboard Text"]);
        assert_eq!(titles("DATE"), ["Date", "Date & Time"]);
        assert_eq!(titles("day"), ["Weekday"]);
        assert_eq!(titles("datet"), ["Date & Time"]);
        assert_eq!(titles("text"), ["Clipboard Text"]);
        assert!(titles("zzz").is_empty());
    }
}
