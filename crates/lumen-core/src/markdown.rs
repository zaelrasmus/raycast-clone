//! The restricted Markdown a view's detail is written in (F4, F10).
//!
//! [`Detail::markdown`](crate::view::Detail::markdown) promises "headings,
//! paragraphs, emphasis, code, lists and rules" and nothing else. This is that
//! promise, parsed: a closed set of blocks and spans that a renderer can draw
//! without knowing Markdown at all.
//!
//! Parsed here rather than in the UI crate so the rules are unit-tested with no
//! window in sight, and so the one reading of the subset lives beside the
//! vocabulary that names it (I-13).
//!
//! What is not in the subset is not an error. A line that looks like a table,
//! a link or an image is shown as the text it is: a detail written for a
//! richer renderer degrades to readable prose, never to nothing.

/// One block of a document, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `#`, `##` or `###`. Deeper levels are read as the third.
    Heading(u8, Vec<Span>),
    /// Consecutive lines, joined with a space, as Markdown does.
    Paragraph(Vec<Span>),
    /// One `- ` or `* ` line.
    Bullet(Vec<Span>),
    /// `---`, `***` or `___` on a line of its own.
    Rule,
}

/// A run of text with one style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
}

impl Span {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
}

/// The text of a list of spans, with the styling dropped.
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

/// Read a document into blocks.
pub fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();

    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(inline(&paragraph.join(" "))));
            paragraph.clear();
        }
    };

    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if is_rule(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
        } else if let Some((level, text)) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading(level, inline(text)));
        } else if let Some(text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet(inline(text.trim())));
        } else {
            paragraph.push(trimmed);
        }
    }
    flush(&mut paragraph, &mut blocks);

    blocks
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|mark| compact.chars().all(|c| c == *mark))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 {
        return None;
    }
    // `#hashtag` is a word, not a heading: Markdown asks for the space.
    let text = line[hashes..].strip_prefix(' ')?;
    Some((hashes.min(3) as u8, text.trim()))
}

/// Split a line into spans: `**strong**`, `*emphasis*` or `_emphasis_`, and
/// `` `code` ``. A marker with no partner is kept as text.
fn inline(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain_run = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        let styled = if let Some(after) = rest.strip_prefix('`') {
            after.find('`').map(|end| {
                let span = Span {
                    code: true,
                    ..Span::plain(&after[..end])
                };
                (span, &after[end + 1..])
            })
        } else if let Some(after) = rest.strip_prefix("**") {
            after.find("**").filter(|end| *end > 0).map(|end| {
                let span = Span {
                    strong: true,
                    ..Span::plain(&after[..end])
                };
                (span, &after[end + 2..])
            })
        } else if let Some(mark) = rest.chars().next().filter(|c| {
            // `snake_case` is a word: an underscore opens emphasis only at
            // the start of one.
            *c == '*' || (*c == '_' && !plain_run.ends_with(char::is_alphanumeric))
        }) {
            let after = &rest[1..];
            after
                .find(mark)
                .filter(|end| *end > 0 && !after.starts_with(' '))
                .map(|end| {
                    let span = Span {
                        emphasis: true,
                        ..Span::plain(&after[..end])
                    };
                    (span, &after[end + 1..])
                })
        } else {
            None
        };

        match styled {
            Some((span, after)) => {
                if !plain_run.is_empty() {
                    spans.push(Span::plain(std::mem::take(&mut plain_run)));
                }
                spans.push(span);
                rest = after;
            }
            None => {
                let character = rest.chars().next().expect("rest is not empty");
                plain_run.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    if !plain_run.is_empty() {
        spans.push(Span::plain(plain_run));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(block: &Block) -> String {
        match block {
            Block::Heading(_, spans) | Block::Paragraph(spans) | Block::Bullet(spans) => {
                plain(spans)
            }
            Block::Rule => "---".into(),
        }
    }

    #[test]
    fn reads_each_kind_of_block() {
        let blocks = parse("# display\n\nTo show.\n\n- one\n* two\n\n---\n### Forms");
        assert!(matches!(blocks[0], Block::Heading(1, _)));
        assert!(matches!(blocks[1], Block::Paragraph(_)));
        assert!(matches!(blocks[2], Block::Bullet(_)));
        assert!(matches!(blocks[3], Block::Bullet(_)));
        assert_eq!(blocks[4], Block::Rule);
        assert!(matches!(blocks[5], Block::Heading(3, _)));
        assert_eq!(text(&blocks[3]), "two");
    }

    /// A paragraph's lines are one paragraph, as Markdown reads them; only a
    /// blank line ends it.
    #[test]
    fn consecutive_lines_are_one_paragraph() {
        let blocks = parse("To make a prominent\nexhibition of something.\n\nAnother.");
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            text(&blocks[0]),
            "To make a prominent exhibition of something."
        );
    }

    #[test]
    fn a_hash_without_a_space_is_a_word() {
        assert!(matches!(parse("#hashtag")[0], Block::Paragraph(_)));
    }

    #[test]
    fn deep_headings_are_the_third_level() {
        assert!(matches!(parse("##### deep")[0], Block::Heading(3, _)));
    }

    #[test]
    fn reads_strong_emphasis_and_code() {
        let spans = inline("a **b** *c* _d_ `e`");
        let styled: Vec<_> = spans
            .iter()
            .map(|span| (span.text.as_str(), span.strong, span.emphasis, span.code))
            .collect();
        assert_eq!(
            styled,
            [
                ("a ", false, false, false),
                ("b", true, false, false),
                (" ", false, false, false),
                ("c", false, true, false),
                (" ", false, false, false),
                ("d", false, true, false),
                (" ", false, false, false),
                ("e", false, false, true),
            ]
        );
    }

    /// A marker with no partner is part of the text: `2 * 3` is arithmetic,
    /// not the start of emphasis that swallows the rest of the line.
    #[test]
    fn an_unpaired_marker_is_text() {
        assert_eq!(plain(&inline("2 * 3 and **open")), "2 * 3 and **open");
        assert_eq!(inline("2 * 3").len(), 1);
    }

    /// What the subset does not have is shown as text, never dropped.
    #[test]
    fn unsupported_syntax_degrades_to_text() {
        let blocks = parse("| a | b |\n[link](https://example.com)");
        assert_eq!(blocks.len(), 1);
        assert!(text(&blocks[0]).contains("[link](https://example.com)"));
    }

    #[test]
    fn an_underscore_inside_a_word_is_text() {
        assert_eq!(inline("snake_case_name").len(), 1);
    }

    #[test]
    fn text_outside_ascii_survives() {
        assert_eq!(
            plain(&inline("*mostrar* — /mosˈtɾaɾ/")),
            "mostrar — /mosˈtɾaɾ/"
        );
    }
}
