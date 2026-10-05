//! The link of a quicklink, with its placeholders parsed once.
//!
//! Parsed at load rather than substituted with `str::replace` at the point of
//! use, for two reasons. A typo such as `{qeury}` is found when the file is
//! read and reported against the link that has it, rather than opening a URL
//! with braces in it the first time somebody uses the link. And whether a link
//! takes a query is a question root search asks on every keystroke, which a
//! parsed template answers without looking at the string again.

use std::fmt;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Everything but RFC 3986's unreserved characters.
///
/// Encoded as a URI *component*, not as a whole URI: a query containing `&` or
/// `#` must arrive as text, not end the parameter it was put in. A space
/// becomes `%20` rather than `+`, which every search engine accepts and which,
/// unlike `+`, also means a space in a path.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// A parsed link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    /// What the user typed after the alias.
    Query,
    /// The clipboard's text, read when the link is opened.
    Clipboard,
}

/// Why a link was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// `{` with no `}` after it.
    Unclosed,
    Unknown(String),
    /// The link does not start with `scheme:`.
    ///
    /// Required because the rendered link is handed to the shell or passed to
    /// a browser as an argument. A bare word is opened as a *file* by the
    /// former, and a leading `-` is read as a flag by the latter.
    NoScheme,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::Unclosed => write!(f, "a `{{` is never closed"),
            TemplateError::Unknown(name) => write!(
                f,
                "`{{{name}}}` is not a placeholder; the known ones are {{query}} and {{clipboard}}"
            ),
            TemplateError::NoScheme => {
                write!(f, "a link must start with a scheme, such as https:")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

impl Template {
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        // A path has no placeholders: `{draft}.md` is a file's name. It never
        // reaches a browser's command line either, which is what the scheme
        // check guards, so a UNC path needs none.
        if super::is_path_link(source) {
            return Ok(Self {
                parts: vec![Part::Text(source.to_string())],
            });
        }

        let mut parts = Vec::new();
        let mut rest = source;

        while let Some(open) = rest.find('{') {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_string()));
            }
            let after = &rest[open + 1..];
            let close = after.find('}').ok_or(TemplateError::Unclosed)?;

            parts.push(match after[..close].trim() {
                "query" => Part::Query,
                "clipboard" => Part::Clipboard,
                other => return Err(TemplateError::Unknown(other.to_string())),
            });
            rest = &after[close + 1..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_string()));
        }

        let template = Self { parts };
        if !template.has_scheme() {
            return Err(TemplateError::NoScheme);
        }
        Ok(template)
    }

    /// Whether the link starts with literal `scheme:` (RFC 3986, 3.1).
    fn has_scheme(&self) -> bool {
        let Some(Part::Text(text)) = self.parts.first() else {
            return false;
        };
        let Some((scheme, _)) = text.split_once(':') else {
            return false;
        };
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    }

    /// Whether the link has somewhere to put what follows the alias.
    pub fn takes_query(&self) -> bool {
        self.parts.contains(&Part::Query)
    }

    /// Whether opening the link has to read the clipboard first.
    pub fn reads_clipboard(&self) -> bool {
        self.parts.contains(&Part::Clipboard)
    }

    /// The link with every placeholder filled in, encoded.
    pub fn render(&self, query: &str, clipboard: &str) -> String {
        let mut url = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => url.push_str(text),
                Part::Query => url.extend(utf8_percent_encode(query.trim(), COMPONENT)),
                Part::Clipboard => url.extend(utf8_percent_encode(clipboard.trim(), COMPONENT)),
            }
        }
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_link_renders_unchanged() {
        let link = Template::parse("https://www.youtube.com/@Veritasium").unwrap();

        assert!(!link.takes_query());
        assert_eq!(
            link.render("ignored", ""),
            "https://www.youtube.com/@Veritasium"
        );
    }

    #[test]
    fn the_query_is_encoded_as_a_component() {
        let link = Template::parse("https://www.google.com/search?q={query}").unwrap();

        assert!(link.takes_query());
        // `&` and `#` would otherwise end the parameter; a space is `%20`.
        assert_eq!(
            link.render("rust & c# ñ", ""),
            "https://www.google.com/search?q=rust%20%26%20c%23%20%C3%B1"
        );
    }

    #[test]
    fn the_query_is_trimmed() {
        let link = Template::parse("https://example.com/{query}").unwrap();
        assert_eq!(link.render("  a  ", ""), "https://example.com/a");
    }

    #[test]
    fn the_clipboard_is_its_own_placeholder() {
        let link = Template::parse("https://translate.google.com/?text={clipboard}").unwrap();

        assert!(link.reads_clipboard());
        assert!(!link.takes_query());
        assert_eq!(
            link.render("", "hola mundo"),
            "https://translate.google.com/?text=hola%20mundo"
        );
    }

    #[test]
    fn placeholders_can_repeat_and_sit_anywhere() {
        let link = Template::parse("https://example.com/{query}/{ query }?c={clipboard}").unwrap();
        assert_eq!(link.render("x", "y"), "https://example.com/x/x?c=y");
    }

    #[test]
    fn a_typo_is_refused_rather_than_opened() {
        assert_eq!(
            Template::parse("https://example.com/?q={qeury}"),
            Err(TemplateError::Unknown("qeury".into()))
        );
    }

    #[test]
    fn an_unclosed_brace_is_refused() {
        assert_eq!(
            Template::parse("https://example.com/?q={query"),
            Err(TemplateError::Unclosed)
        );
    }

    /// A bare word would be opened as a file, and a leading `-` would reach a
    /// browser as a flag.
    #[test]
    fn a_link_without_a_scheme_is_refused() {
        for source in ["google.com", "-flag", "{query}", "", "1http://x", ":x"] {
            assert_eq!(
                Template::parse(source),
                Err(TemplateError::NoScheme),
                "{source:?}"
            );
        }
    }

    /// A path is taken as written: braces are part of a file's name there,
    /// and a UNC path has no scheme.
    #[test]
    fn a_path_is_kept_literally() {
        for source in [
            r"D:\Dev\notes.md",
            r"C:\Users\x\",
            r"D:\Dev\{draft} notes.md",
            r"\\server\share\{query}.txt",
            "file:///C:/a%20b/{c}.txt",
        ] {
            let link = Template::parse(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
            assert!(!link.takes_query(), "{source:?}");
            assert_eq!(link.render("q", "c"), source);
        }
    }

    #[test]
    fn non_web_schemes_are_links_too() {
        for source in [
            "mailto:someone@example.com",
            "spotify:search:{query}",
            "zed://file",
        ] {
            assert!(Template::parse(source).is_ok(), "{source:?}");
        }
    }
}
