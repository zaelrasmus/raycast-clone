//! A URL typed into the search field (F6).
//!
//! Type `github.com` or `https://example.com/page` and the first row opens it,
//! with the same private-window and choose-a-browser actions as a quicklink,
//! and one to save it as one. Detected as the text is typed, like the
//! calculator's answer: nothing is looked up, so nothing is waited on (I-3).
//!
//! # How sure it has to be
//!
//! A scheme, `www.`, `localhost` or an IP address is a URL beyond doubt and
//! leads the list. A bare `word.word` is only probably one: `readme.md`,
//! `main.rs` and `setup.py` are file names whose endings are also countries'
//! domains. So a bare domain leads only when its ending is a common one
//! ([`COMMON_TLDS`]); otherwise it is offered at the foot of the first screen,
//! beside "Search Google for ...", where it never pushes a file or an
//! application out of first place.
//!
//! # Nothing is remembered
//!
//! Every row opened is recorded for ranking, under its id. A typed URL's id is
//! the same fixed string whatever the URL, as the calculator's is, so the
//! history on disk never holds an address the user typed.

use crate::item::{Action, Item, Kind};
use crate::providers::quicklinks::{OpenLink, Template};

/// The id every typed-URL row shares. See the module's last section.
pub const ID: &str = "web:typed";

/// Endings that lead the list as bare domains.
///
/// Common generic and country endings, minus the ones that are also file
/// extensions people type: `.md` (Markdown), `.rs` (Rust), `.py` (Python),
/// `.sh` (shell), `.pl` (Perl), `.cc` (C++), `.ps` (PostScript). Those still
/// get a row, lower down.
pub const COMMON_TLDS: &[&str] = &[
    "com", "org", "net", "io", "dev", "app", "ai", "co", "gg", "me", "tv", "xyz", "edu", "gov",
    "info", "biz", "tech", "so", "fm", "ly", "to", "page", "site", "blog", "cloud", "uk", "de",
    "fr", "es", "it", "nl", "eu", "ca", "us", "jp", "cn", "br", "mx", "ar", "cl", "in", "ru", "au",
    "ch", "se", "no", "fi", "dk", "be", "at", "cz", "pt", "kr", "tw", "nz", "ie",
];

/// A URL recognised in the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typed {
    /// What to open: the text as typed, with a scheme added if it had none.
    pub url: String,
    /// The host, for the row's title and a quicklink's suggested name.
    pub host: String,
    /// Whether it leads the list, or waits at the foot of the first screen.
    pub certain: bool,
}

/// The URL `query` is, if it is one.
pub fn recognise(query: &str) -> Option<Typed> {
    let text = query.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }

    let lower = text.to_ascii_lowercase();
    let (scheme, rest) = if let Some(rest) = lower.strip_prefix("https://") {
        (Some("https://"), &text[text.len() - rest.len()..])
    } else if let Some(rest) = lower.strip_prefix("http://") {
        (Some("http://"), &text[text.len() - rest.len()..])
    } else {
        (None, text)
    };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if port.is_some_and(|port| port.is_empty() || !port.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let host = host.to_ascii_lowercase();

    let certain = if host == "localhost" || is_ipv4(&host) {
        // No scheme on these means a local server, which is almost never
        // behind TLS.
        let url = format!("{}{rest}", scheme.unwrap_or("http://"));
        return Some(Typed {
            url,
            host,
            certain: true,
        });
    } else {
        let tld = domain_tld(&host)?;
        scheme.is_some() || host.starts_with("www.") || COMMON_TLDS.contains(&tld)
    };

    Some(Typed {
        url: format!("{}{rest}", scheme.unwrap_or("https://")),
        host,
        certain,
    })
}

/// The ending of `host`, if it is a well-formed domain name: two labels or
/// more, of letters, digits and inner hyphens, ending in letters.
fn domain_tld(host: &str) -> Option<&str> {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    let well_formed = labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let tld = *labels.last()?;
    let alphabetic = (2..=24).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_alphabetic());
    (well_formed && alphabetic).then_some(tld)
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.len() <= 3 && part.parse::<u8>().is_ok())
}

/// The row for `typed`: open it, copy it, open it privately or elsewhere, or
/// keep it as a quicklink.
pub fn item(typed: &Typed) -> Option<Item> {
    // A brace would read as a placeholder; such a URL is not worth guessing
    // about.
    let link = Template::parse(&typed.url).ok()?;
    let open = OpenLink {
        link,
        query: String::new(),
        browser: None,
        private: false,
    };

    Some(Item {
        id: ID.into(),
        title: format!("Open {}", typed.host),
        subtitle: Some(typed.url.clone()),
        icon: None,
        keywords: Vec::new(),
        actions: vec![
            Action::OpenLink(open.clone()),
            Action::CopyToClipboard(typed.url.clone()),
            Action::OpenLink(OpenLink {
                private: true,
                ..open.clone()
            }),
            Action::ChooseBrowser(open),
            Action::SaveAsQuicklink {
                name: typed.host.trim_start_matches("www.").to_string(),
                link: typed.url.clone(),
            },
        ],
        kind: Kind::Website,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(query: &str) -> Option<(String, bool)> {
        recognise(query).map(|typed| (typed.url, typed.certain))
    }

    #[test]
    fn a_scheme_is_certain_and_kept_as_typed() {
        assert_eq!(
            url("https://github.com/zed-industries/zed"),
            Some(("https://github.com/zed-industries/zed".into(), true))
        );
        assert_eq!(
            url("HTTP://Example.com"),
            Some(("http://Example.com".into(), true))
        );
        // Even with a file-like ending: the scheme says what it is.
        assert_eq!(
            url("https://notes.md"),
            Some(("https://notes.md".into(), true))
        );
    }

    #[test]
    fn a_bare_domain_gets_https() {
        assert_eq!(url("github.com"), Some(("https://github.com".into(), true)));
        assert_eq!(
            url("docs.rs/serde/latest"),
            Some(("https://docs.rs/serde/latest".into(), false))
        );
        assert_eq!(
            url("www.example.md"),
            Some(("https://www.example.md".into(), true))
        );
    }

    /// File names that are also domains are offered, but never first.
    #[test]
    fn a_file_like_ending_waits_below_the_matches() {
        for query in ["readme.md", "main.rs", "setup.py", "install.sh", "node.js"] {
            assert_eq!(
                url(query).map(|(_, certain)| certain),
                Some(false),
                "{query}"
            );
        }
    }

    #[test]
    fn local_servers_are_certain_and_plain_http() {
        assert_eq!(
            url("localhost:3000/api"),
            Some(("http://localhost:3000/api".into(), true))
        );
        assert_eq!(
            url("192.168.1.1"),
            Some(("http://192.168.1.1".into(), true))
        );
        assert_eq!(
            url("https://127.0.0.1:8443"),
            Some(("https://127.0.0.1:8443".into(), true))
        );
    }

    #[test]
    fn things_that_are_not_urls_are_left_alone() {
        for query in [
            "",
            "firefox",
            "12.5",
            "3.14159",
            "v1.2",
            "e.g",
            "a b.com",
            "foo@bar.com",
            "-bad.com",
            "bad-.com",
            "github.com:abc",
            "999.1.1.1",
            "..com",
            "google",
        ] {
            assert_eq!(url(query), None, "{query:?}");
        }
    }

    #[test]
    fn the_row_opens_it_and_offers_the_rest() {
        let row = item(&recognise("www.github.com/x").unwrap()).unwrap();

        assert_eq!(row.id, ID);
        assert_eq!(row.title, "Open www.github.com");
        assert_eq!(row.kind, Kind::Website);
        assert!(matches!(&row.actions[0], Action::OpenLink(open) if !open.private));
        assert!(matches!(&row.actions[2], Action::OpenLink(open) if open.private));
        assert!(matches!(row.actions[3], Action::ChooseBrowser(_)));
        assert_eq!(
            row.actions[4],
            Action::SaveAsQuicklink {
                name: "github.com".into(),
                link: "https://www.github.com/x".into(),
            }
        );
    }

    /// Every URL shares one id, so none of them is written to the history.
    #[test]
    fn every_typed_url_shares_one_id() {
        let a = item(&recognise("a.com").unwrap()).unwrap();
        let b = item(&recognise("https://b.org/secret").unwrap()).unwrap();
        assert_eq!(a.id, b.id);
        assert!(!a.id.contains("a.com"));
    }
}
