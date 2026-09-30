//! Markdown source, as the snippet editor draws and edits it (F11, slice 5).
//!
//! The editor's text *is* the Markdown: the toolbar wraps the selection in
//! markers, and the editor styles what it shows from [`styles`], the markers
//! kept visible and dimmed. Because nothing on screen differs from the
//! source, the caret, the selection and the input method work on one string.
//!
//! The spans are the ones a snippet paste understands: `**strong**`,
//! `_emphasis_` (or `*emphasis*`), `++underline++`, `~~strike~~`,
//! `` `code` `` and `[links](url)`. Unclosed markers are text, as in
//! [`crate::markdown`]. Both are byte ranges into the source, so the editor
//! turns them into text runs with no second reading of the text.

use std::ops::Range;

/// How a byte range of the source is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    /// A marker: `**`, `_`, `](url)`. Drawn dimmed.
    pub marker: bool,
}

/// The source read once, as a tree: the one parse the editor's styles, the
/// plain text and the HTML are all read from, so they cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Text(Range<usize>),
    Span {
        kind: Kind,
        open: Range<usize>,
        children: Vec<Node>,
        close: Range<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Strong,
    Emphasis,
    Underline,
    Strike,
    Code,
    /// The address, as a range of the source.
    Link(Range<usize>),
}

impl Kind {
    fn apply(&self, style: &mut Style) {
        match self {
            Kind::Strong => style.strong = true,
            Kind::Emphasis => style.emphasis = true,
            Kind::Underline => style.underline = true,
            Kind::Strike => style.strike = true,
            Kind::Code => style.code = true,
            Kind::Link(_) => style.link = true,
        }
    }
}

fn parse(text: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    // Markdown's spans do not cross a line.
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        inline(line, start, &mut nodes);
        start += line.len();
    }
    nodes
}

/// The pairs of markers, longest first so `**` is not read as two `*`.
const PAIRS: &[(&str, Kind)] = &[
    ("**", Kind::Strong),
    ("++", Kind::Underline),
    ("~~", Kind::Strike),
    ("*", Kind::Emphasis),
    ("_", Kind::Emphasis),
];

/// Read one line's spans, `base` being where it starts in the source.
fn inline(text: &str, base: usize, out: &mut Vec<Node>) {
    let mut at = 0;
    let mut plain_from = 0;
    let flush = |from: usize, to: usize, out: &mut Vec<Node>| {
        if to > from {
            out.push(Node::Text(base + from..base + to));
        }
    };

    while at < text.len() {
        let rest = &text[at..];

        // Code is literal inside: nothing in it is a marker.
        if let Some(inner) = rest.strip_prefix('`')
            && let Some(end) = inner.find('`').filter(|end| *end > 0)
        {
            flush(plain_from, at, out);
            let open = base + at..base + at + 1;
            let body = open.end..open.end + end;
            out.push(Node::Span {
                kind: Kind::Code,
                close: body.end..body.end + 1,
                children: vec![Node::Text(body)],
                open,
            });
            at += end + 2;
            plain_from = at;
            continue;
        }

        if let Some((label, url)) = link(rest) {
            flush(plain_from, at, out);
            let mut children = Vec::new();
            inline(&rest[1..1 + label], base + at + 1, &mut children);
            let tail = base + at + 1 + label;
            out.push(Node::Span {
                // `](` then the address.
                kind: Kind::Link(tail + 2..tail + 2 + url),
                open: base + at..base + at + 1,
                children,
                close: tail..tail + 2 + url + 1,
            });
            at += 1 + label + 2 + url + 1;
            plain_from = at;
            continue;
        }

        let mut matched = false;
        for (mark, kind) in PAIRS {
            let Some(inner) = rest.strip_prefix(mark) else {
                continue;
            };
            // `snake_case` is a word: `_` opens emphasis only at the start of
            // one, as `crate::markdown` reads it.
            if *mark == "_" && text[..at].ends_with(char::is_alphanumeric) {
                continue;
            }
            // `* item` and `** x` are not emphasis: the content hugs the marker.
            if inner.starts_with(char::is_whitespace) {
                continue;
            }
            let Some(end) = inner.find(mark).filter(|end| *end > 0) else {
                continue;
            };
            if inner[..end].ends_with(char::is_whitespace) {
                continue;
            }
            flush(plain_from, at, out);
            let len = mark.len();
            let mut children = Vec::new();
            inline(&inner[..end], base + at + len, &mut children);
            out.push(Node::Span {
                kind: kind.clone(),
                open: base + at..base + at + len,
                children,
                close: base + at + len + end..base + at + 2 * len + end,
            });
            at += 2 * len + end;
            plain_from = at;
            matched = true;
            break;
        }
        if !matched {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    flush(plain_from, text.len(), out);
}

/// The styled ranges of `text`, in order, covering only what is styled.
pub fn styles(text: &str) -> Vec<(Range<usize>, Style)> {
    fn walk(nodes: &[Node], outer: Style, out: &mut Vec<(Range<usize>, Style)>) {
        for node in nodes {
            match node {
                Node::Text(range) => {
                    if outer != Style::default() {
                        out.push((range.clone(), outer));
                    }
                }
                Node::Span {
                    kind,
                    open,
                    children,
                    close,
                } => {
                    let marker = Style {
                        marker: true,
                        ..outer
                    };
                    let mut inner = outer;
                    kind.apply(&mut inner);
                    out.push((open.clone(), marker));
                    walk(children, inner, out);
                    out.push((close.clone(), marker));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&parse(text), Style::default(), &mut out);
    out
}

/// The text with the markers taken out, as a snippet is pasted where only
/// plain text is read.
pub fn plain(text: &str) -> String {
    fn walk(text: &str, nodes: &[Node], out: &mut String) {
        for node in nodes {
            match node {
                Node::Text(range) => out.push_str(&text[range.clone()]),
                Node::Span { children, .. } => walk(text, children, out),
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    walk(text, &parse(text), &mut out);
    out
}

/// [`plain`], with the caret's place carried through: `cursor` is a byte
/// offset into `text`, and the answer's second half is how many graphemes of
/// the plain text follow it -- the Left presses that reach it once pasted.
///
/// The place is marked with a private-use character before the markers are
/// taken out and found again after, so the one parser decides where it went,
/// even from inside a span.
pub fn plain_with_caret(text: &str, cursor: Option<usize>) -> (String, usize) {
    use unicode_segmentation::UnicodeSegmentation as _;

    const MARK: char = '\u{E000}';
    let Some(at) = cursor.filter(|at| text.is_char_boundary(*at)) else {
        return (plain(text), 0);
    };
    let mut marked = String::with_capacity(text.len() + MARK.len_utf8());
    marked.push_str(&text[..at]);
    marked.push(MARK);
    marked.push_str(&text[at..]);
    let plain = plain(&marked);
    match plain.split_once(MARK) {
        Some((before, after)) => (format!("{before}{after}"), after.graphemes(true).count()),
        None => (plain.replace(MARK, ""), 0),
    }
}

/// The text as HTML, for applications that read formatted text: a fragment,
/// lines broken with `<br>`, everything escaped.
pub fn html(text: &str) -> String {
    fn escape(text: &str, out: &mut String) {
        for character in text.chars() {
            match character {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\n' => out.push_str("<br>\n"),
                '\r' => {}
                other => out.push(other),
            }
        }
    }
    fn walk(text: &str, nodes: &[Node], out: &mut String) {
        for node in nodes {
            let (kind, children) = match node {
                Node::Text(range) => {
                    escape(&text[range.clone()], out);
                    continue;
                }
                Node::Span { kind, children, .. } => (kind, children),
            };
            let tag = match kind {
                Kind::Strong => "strong",
                Kind::Emphasis => "em",
                Kind::Underline => "u",
                Kind::Strike => "s",
                Kind::Code => "code",
                Kind::Link(_) => "a",
            };
            match kind {
                Kind::Link(url) => {
                    out.push_str("<a href=\"");
                    escape(&text[url.clone()], out);
                    out.push_str("\">");
                }
                _ => {
                    out.push('<');
                    out.push_str(tag);
                    out.push('>');
                }
            }
            walk(text, children, out);
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    walk(text, &parse(text), &mut out);
    out
}

/// `[label](url)` at the start of `text`: the label's and the url's lengths.
fn link(text: &str) -> Option<(usize, usize)> {
    let inner = text.strip_prefix('[')?;
    let label = inner.find(']').filter(|label| *label > 0)?;
    let after = inner[label + 1..].strip_prefix('(')?;
    let url = after.find(')')?;
    if after[..url].contains(char::is_whitespace) {
        return None;
    }
    Some((label, url))
}

/// What a toolbar button does to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Strong,
    Emphasis,
    Underline,
    Strike,
    Code,
    Link,
}

impl Wrap {
    fn marks(self) -> (&'static str, &'static str) {
        match self {
            Wrap::Strong => ("**", "**"),
            Wrap::Emphasis => ("_", "_"),
            Wrap::Underline => ("++", "++"),
            Wrap::Strike => ("~~", "~~"),
            Wrap::Code => ("`", "`"),
            Wrap::Link => ("[", "](url)"),
        }
    }
}

/// An edit to the text: replace `range` with `text`, then select `selection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

/// Wrap `selection` in `wrap`'s markers, or take them off when it is already
/// wrapped -- inside the selection or just outside it -- so pressing a button
/// twice undoes it. With nothing selected, the markers are put in and the
/// caret left between them. A link selects `url`, to be typed over.
pub fn wrap(text: &str, selection: Range<usize>, wrap: Wrap) -> Edit {
    let (open, close) = wrap.marks();
    let selected = &text[selection.clone()];

    if wrap != Wrap::Link {
        // Already wrapped, the markers selected too.
        if selected.len() >= open.len() + close.len()
            && selected.starts_with(open)
            && selected.ends_with(close)
        {
            let inner = &selected[open.len()..selected.len() - close.len()];
            return Edit {
                range: selection.clone(),
                text: inner.to_string(),
                selection: selection.start..selection.start + inner.len(),
            };
        }
        // Already wrapped, the markers just outside the selection.
        if selection.start >= open.len()
            && text[..selection.start].ends_with(open)
            && text[selection.end..].starts_with(close)
        {
            let start = selection.start - open.len();
            return Edit {
                range: start..selection.end + close.len(),
                text: selected.to_string(),
                selection: start..start + selected.len(),
            };
        }
    }

    let start = selection.start + open.len();
    let after = if wrap == Wrap::Link {
        // Select `url`, which is what is to be typed next.
        let url = start + selected.len() + 2;
        url..url + 3
    } else {
        start..start + selected.len()
    };
    Edit {
        range: selection,
        text: format!("{open}{selected}{close}"),
        selection: after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source with each styled range written as `<flags:text>`.
    fn show(text: &str) -> String {
        styles(text)
            .into_iter()
            .map(|(range, style)| {
                let mut flags = String::new();
                for (on, letter) in [
                    (style.strong, 'b'),
                    (style.emphasis, 'i'),
                    (style.underline, 'u'),
                    (style.strike, 's'),
                    (style.code, 'c'),
                    (style.link, 'l'),
                    (style.marker, 'm'),
                ] {
                    if on {
                        flags.push(letter);
                    }
                }
                format!("<{flags}:{}>", &text[range])
            })
            .collect()
    }

    #[test]
    fn each_span_is_styled_with_its_markers_dimmed() {
        assert_eq!(show("a **b** c"), "<m:**><b:b><m:**>");
        assert_eq!(show("_i_ *j*"), "<m:_><i:i><m:_><m:*><i:j><m:*>");
        assert_eq!(show("++u++ ~~s~~"), "<m:++><u:u><m:++><m:~~><s:s><m:~~>");
        assert_eq!(show("`c`"), "<m:`><c:c><m:`>");
        assert_eq!(
            show("[site](https://x.y)"),
            "<m:[><l:site><m:](https://x.y)>"
        );
    }

    #[test]
    fn spans_nest() {
        assert_eq!(show("**a _b_**"), "<m:**><b:a ><bm:_><bi:b><bm:_><m:**>");
    }

    #[test]
    fn what_is_not_a_span_is_text() {
        for text in [
            "snake_case_name",
            "* bullet",
            "a * b * c",
            "**open",
            "``",
            "[label] (not a link)",
            "[x](has space)",
            "**a\nb**",
        ] {
            assert_eq!(show(text), "", "{text}");
        }
        // Code is literal inside.
        assert_eq!(show("`**x**`"), "<m:`><c:**x**><m:`>");
    }

    #[test]
    fn ranges_are_bytes_into_the_source() {
        let text = "é **ñ**";
        let styles = styles(text);
        assert_eq!(&text[styles[1].0.clone()], "ñ");
    }

    #[test]
    fn wrapping_the_selection_keeps_it_selected() {
        let edit = wrap("say hi now", 4..6, Wrap::Strong);
        assert_eq!(edit.range, 4..6);
        assert_eq!(edit.text, "**hi**");
        assert_eq!(edit.selection, 6..8);
    }

    #[test]
    fn with_nothing_selected_the_caret_goes_between_the_markers() {
        let edit = wrap("ab", 1..1, Wrap::Underline);
        assert_eq!(edit.range, 1..1);
        assert_eq!(edit.text, "++++");
        assert_eq!(edit.selection, 3..3);
    }

    #[test]
    fn pressing_twice_takes_the_markers_off() {
        // The markers just outside the selection, as the first press left it.
        let edit = wrap("say **hi** now", 6..8, Wrap::Strong);
        assert_eq!(
            (edit.range, edit.text.as_str(), edit.selection),
            (4..10, "hi", 4..6)
        );
        // The markers selected too.
        let edit = wrap("say **hi** now", 4..10, Wrap::Strong);
        assert_eq!(
            (edit.range, edit.text.as_str(), edit.selection),
            (4..10, "hi", 4..6)
        );
    }

    #[test]
    fn a_link_selects_its_url_to_be_typed_over() {
        let text = "see docs";
        let edit = wrap(text, 4..8, Wrap::Link);
        assert_eq!(edit.text, "[docs](url)");
        let mut after = text.to_string();
        after.replace_range(edit.range.clone(), &edit.text);
        assert_eq!(&after[edit.selection], "url");
    }

    #[test]
    fn html_is_the_same_reading_escaped() {
        assert_eq!(
            html("**Hi** <you> & _them_\n[docs](https://x.y/?a=1&b=2) `a<b` ++u++ ~~s~~"),
            "<strong>Hi</strong> &lt;you&gt; &amp; <em>them</em><br>\n\
             <a href=\"https://x.y/?a=1&amp;b=2\">docs</a> <code>a&lt;b</code> <u>u</u> <s>s</s>"
        );
        assert_eq!(html("**a _b_**"), "<strong>a <em>b</em></strong>");
    }

    #[test]
    fn the_caret_is_counted_in_the_plain_text() {
        // `**Hi** {cursor}_you_`: after it, "you" once the markers are gone.
        let text = "**Hi** _you_";
        assert_eq!(plain_with_caret(text, Some(7)), ("Hi you".to_string(), 3));
        // From inside a span.
        assert_eq!(plain_with_caret("**ab**c", Some(3)), ("abc".to_string(), 2));
        assert_eq!(plain_with_caret("**ab**", None), ("ab".to_string(), 0));
    }

    #[test]
    fn plain_takes_the_markers_out() {
        assert_eq!(
            plain("**Hi** _you_, see [docs](https://x.y) and `a*b`"),
            "Hi you, see docs and a*b"
        );
        assert_eq!(plain("snake_case * 2"), "snake_case * 2");
    }
}
