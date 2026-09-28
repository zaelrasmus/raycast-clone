//! Quicklinks and web search (F6).
//!
//! A quicklink is a saved link: a YouTube channel, a dashboard, a search page.
//! Web search is not a feature of its own. "Search Google" is a quicklink whose
//! link has a `{query}` in it, which is Raycast's model and the reason this is
//! one mechanism rather than two.
//!
//! # Three ways a quicklink reaches the list
//!
//! - **As a catalogue entry**, matched by name like an application. Choosing
//!   one opens its link; a link that takes a query opens with an empty one.
//! - **As an answer** to its alias followed by a space: `g lofi beats` offers
//!   the Google search for `lofi beats` above everything else, because it was
//!   computed from exactly what was typed (7.9).
//! - **As a fallback** at the foot of the list, for any query, when the link is
//!   marked `fallback`. That is "Search Google for ..." when nothing else was
//!   what you meant.
//!
//! # A data file, like snippets
//!
//! `quicklinks.json`, beside `snippets.json` and for the same reasons: a few
//! dozen entries, edited by hand, worth keeping in a dotfiles repository. The
//! clipboard's database exists because of its write pattern (7.14), and this
//! has none.

pub mod favicon;
pub mod store;
mod template;

use std::path::{Path, PathBuf};

use lumen_platform::{Browser, IconSource, Platform};
use serde::{Deserialize, Serialize};

use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

pub use template::{Template, TemplateError};

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = 1;

/// One saved link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quicklink {
    /// What appears in the list, and what it is found by.
    pub name: String,
    pub link: Template,
    /// The link as written, for the row's second line.
    pub source: String,
    /// A short word that, followed by a space, fills the link's `{query}`.
    pub alias: Option<String>,
    /// A browser's display name; the preferred browser otherwise.
    pub open_with: Option<String>,
    /// Offered at the foot of the list for any query.
    pub fallback: bool,
    /// Matched against, never shown.
    pub keywords: Vec<String>,
    /// Whether the row shows the site's icon rather than the glyph.
    pub site_icon: bool,
    /// The site's icon, once fetched ([`attach_icons`]).
    pub icon_file: Option<PathBuf>,
}

/// The `icon` value that keeps the quicklink glyph.
pub const ICON_DEFAULT: &str = "default";

/// One entry as it appears on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub link: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_with: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// Absent for the site's icon; [`ICON_DEFAULT`] for the glyph. Any other
    /// value is kept as written, for a build that knows it, and read as the
    /// site's icon here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Fields this build does not know, kept so that saving from Lumen does
    /// not strip what a newer one wrote (see [`store`]).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The file as it appears on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicklinkFile {
    pub schema_version: u32,
    #[serde(default)]
    pub quicklinks: Vec<Entry>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Quicklink {
    /// Check one entry from the file.
    ///
    /// Refused one at a time rather than failing the file: one link with a
    /// typo in its placeholder should not take the other thirty with it.
    pub fn from_entry(entry: Entry) -> Result<Self, String> {
        let name = entry.name.trim().to_string();
        if name.is_empty() {
            return Err("a quicklink has no name".into());
        }

        let source = entry.link.trim().to_string();
        let link = Template::parse(&source).map_err(|error| format!("{name:?}: {error}"))?;

        // An alias is a word the query starts with; a space inside one could
        // never be typed as a single word, and an empty one would match
        // everything.
        let alias = entry
            .alias
            .map(|alias| alias.trim().to_string())
            .filter(|alias| !alias.is_empty());
        if alias
            .as_ref()
            .is_some_and(|alias| alias.contains(char::is_whitespace))
        {
            return Err(format!("{name:?}: an alias is one word"));
        }

        // A fallback puts the whole query into the link, so a link with
        // nowhere to put it would open the same page for every query.
        if entry.fallback && !link.takes_query() {
            return Err(format!(
                "{name:?}: a fallback needs a {{query}} in its link"
            ));
        }

        Ok(Self {
            name,
            link,
            source,
            alias,
            open_with: entry
                .open_with
                .map(|browser| browser.trim().to_string())
                .filter(|browser| !browser.is_empty()),
            fallback: entry.fallback,
            keywords: entry.keywords,
            site_icon: entry.icon.as_deref().map(str::trim) != Some(ICON_DEFAULT),
            icon_file: None,
        })
    }

    /// The site to fetch an icon from, if this link shows one and has one.
    pub fn favicon_site(&self) -> Option<String> {
        self.site_icon
            .then(|| favicon::site(&self.source))
            .flatten()
    }

    fn icon(&self) -> Option<IconSource> {
        self.icon_file.clone().map(IconSource::File)
    }

    /// The frecency key. The name rather than a position, so reordering the
    /// file keeps what every link has earned.
    fn id(&self) -> String {
        format!("quicklink:{}", self.name)
    }

    fn action(&self, query: &str) -> Action {
        Action::OpenLink(OpenLink {
            link: self.link.clone(),
            query: query.to_string(),
            browser: self.open_with.clone(),
        })
    }

    /// The link itself, as a catalogue entry.
    ///
    /// A link that takes a query asks for it first: opening a search with
    /// nothing in it is almost never what choosing "Google" meant. Opening it
    /// empty stays one step away, in the action panel.
    fn catalogue_item(&self) -> Item {
        let mut actions = Vec::new();
        if self.link.takes_query() {
            actions.push(Action::AskArgument(ArgumentRequest {
                id: self.id(),
                name: self.name.clone(),
            }));
        }
        actions.push(self.action(""));
        // Copying needs the rendered link, which a clipboard placeholder would
        // only have once opened.
        if !self.link.reads_clipboard() && !self.link.takes_query() {
            actions.push(Action::CopyToClipboard(self.link.render("", "")));
        }

        let mut keywords = self.keywords.clone();
        keywords.extend(self.alias.clone());

        Item {
            id: self.id(),
            title: self.name.clone(),
            subtitle: Some(self.source.clone()),
            icon: self.icon(),
            keywords,
            actions,
            kind: Kind::Quicklink,
        }
    }

    /// The link filled in with `query`.
    ///
    /// Shares the catalogue entry's id, so using the answer also raises the
    /// link in the root list: they are the same link.
    fn query_item(&self, query: &str) -> Item {
        let mut actions = vec![self.action(query)];
        if !self.link.reads_clipboard() {
            actions.push(Action::CopyToClipboard(self.link.render(query, "")));
        }

        Item {
            id: self.id(),
            title: format!("Search {} for \u{201c}{}\u{201d}", self.name, query.trim()),
            subtitle: Some(self.link.render(query, "")),
            icon: self.icon(),
            keywords: Vec::new(),
            actions,
            kind: Kind::Quicklink,
        }
    }

    /// What follows this link's alias or name and a space, if the query starts
    /// with either.
    ///
    /// The name as well as the alias, because typing `google lofi` and getting
    /// nothing would be the launcher pretending not to know what you meant.
    fn argument<'q>(&self, query: &'q str) -> Option<&'q str> {
        if !self.link.takes_query() {
            return None;
        }
        [self.alias.as_deref(), Some(self.name.as_str())]
            .into_iter()
            .flatten()
            .find_map(|trigger| {
                let head = query.get(..trigger.len())?;
                let rest = &query[trigger.len()..];
                (head.to_lowercase() == trigger.to_lowercase()
                    && rest.starts_with(char::is_whitespace))
                .then(|| rest.trim())
                .filter(|rest| !rest.is_empty())
            })
    }
}

/// Which link the launcher is asking an argument for.
///
/// The id rather than the link, so that editing the file while the question
/// is on screen answers it with the link as it now is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentRequest {
    pub id: String,
    /// Shown beside the field, so the user can see what they are filling in.
    pub name: String,
}

/// Opening a quicklink.
///
/// The query is filled in when the row is built, but the clipboard and the
/// browser are looked up only when it is opened: reading the clipboard on
/// every keystroke would be a system call per frame, and the browser list is a
/// registry walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenLink {
    pub link: Template,
    pub query: String,
    /// The link's own choice. The preferred browser is filled in by whoever
    /// holds the configuration -- see [`OpenLink::or_browser`].
    pub browser: Option<String>,
}

impl OpenLink {
    /// Use `browser` if the link did not name one.
    pub fn or_browser(mut self, browser: Option<&str>) -> Self {
        if self.browser.is_none() {
            self.browser = browser.map(str::to_string);
        }
        self
    }

    /// Open it. Blocking, and so called off the UI thread.
    pub fn run(&self, platform: &dyn Platform) -> anyhow::Result<()> {
        let clipboard = if self.link.reads_clipboard() {
            platform.clipboard_text()?.unwrap_or_default()
        } else {
            String::new()
        };
        let url = self.link.render(&self.query, &clipboard);

        let browsers;
        let browser = match &self.browser {
            Some(wanted) => {
                browsers = platform.installed_browsers()?;
                let found = choose_browser(wanted, &browsers);
                // Opened anyway, in the default. A browser that was
                // uninstalled or misspelt should not make the link dead.
                if found.is_none() {
                    let known: Vec<&str> = browsers.iter().map(|b| b.name.as_str()).collect();
                    log::warn!("no browser matches {wanted:?}; installed: {known:?}");
                }
                found
            }
            None => None,
        };

        platform.open_url(&url, browser)
    }
}

/// Whether copied text is a web link worth offering as a new quicklink.
///
/// Web links only: a copied `mailto:` or a sentence that happens to contain a
/// colon is not what somebody opening "Create Quicklink" has just copied.
pub fn looks_like_link(text: &str) -> bool {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && text.len() > "https://".len()
        && !text.contains(char::is_whitespace)
        && Template::parse(text).is_ok()
}

/// The browser the user meant by `wanted`.
///
/// The whole display name first, ignoring case; then a part of one, so `zen`
/// finds "Zen Browser". A part that fits two browsers finds neither, because
/// guessing between them would open links somewhere the user did not choose.
pub fn choose_browser<'b>(wanted: &str, browsers: &'b [Browser]) -> Option<&'b Browser> {
    matching(wanted, browsers, |browser| &browser.name)
}

/// [`choose_browser`] over names alone, for a form that lists them: the one
/// the link would open in is the one it shows as chosen.
pub fn matching_browser<'n>(wanted: &str, names: &'n [String]) -> Option<&'n str> {
    matching(wanted, names, |name| name).map(String::as_str)
}

fn matching<'b, B>(wanted: &str, browsers: &'b [B], name: impl Fn(&B) -> &str) -> Option<&'b B> {
    let wanted = wanted.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }

    if let Some(exact) = browsers
        .iter()
        .find(|browser| name(browser).to_lowercase() == wanted)
    {
        return Some(exact);
    }

    let mut partial = browsers
        .iter()
        .filter(|browser| name(browser).to_lowercase().contains(&wanted));
    match (partial.next(), partial.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Read the quicklinks file.
///
/// A missing file is the normal case and not an error. A malformed one is
/// reported and ignored, and so is each malformed entry: the launcher is
/// summoned with no terminal attached, and one stray comma must not look like
/// a crash.
pub fn load(path: &Path) -> Vec<Quicklink> {
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let file = match serde_json::from_str::<QuicklinkFile>(&source) {
        Ok(file) => file,
        Err(error) => {
            log::error!("{} is not valid: {error}", path.display());
            return Vec::new();
        }
    };
    if file.schema_version > SCHEMA_VERSION {
        log::warn!(
            "{} declares schema version {} but this build understands {SCHEMA_VERSION}",
            path.display(),
            file.schema_version
        );
    }

    file.quicklinks
        .into_iter()
        .filter_map(|entry| {
            Quicklink::from_entry(entry)
                .inspect_err(|error| log::warn!("{}: skipped {error}", path.display()))
                .ok()
        })
        .collect()
}

/// Point each link at its site's cached icon, where one has been fetched.
///
/// Only what is on disk: fetching is [`favicon::refresh`]'s, on a thread
/// that can wait.
pub fn attach_icons(links: &mut [Quicklink], dir: &Path) {
    for link in links {
        link.icon_file = link
            .favicon_site()
            .and_then(|site| favicon::cached(dir, &site));
    }
}

/// Write the example file if none exists.
///
/// The example is where Google comes from. Shipping it as data rather than
/// building it in means removing it, replacing it with DuckDuckGo or pointing
/// it at a different browser is an edit, not a setting.
pub fn install_example(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, EXAMPLE)?;
    Ok(())
}

const EXAMPLE: &str = include_str!("../../../../../assets/quicklinks.json");

/// The quicklinks, as a catalogue, answers and fallbacks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QuicklinkProvider {
    links: Vec<Quicklink>,
}

impl QuicklinkProvider {
    pub fn new(links: Vec<Quicklink>) -> Self {
        Self { links }
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// The link whose alias or whole name `query` is, when it takes a query.
    ///
    /// What Tab falls back to when the selected row takes no argument: typing
    /// `yt` and pressing Tab means YouTube, whichever row usage happened to put
    /// first.
    pub fn named(&self, query: &str) -> Option<ArgumentRequest> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return None;
        }
        self.links
            .iter()
            .filter(|link| link.link.takes_query())
            .find(|link| {
                link.alias.as_deref().map(str::to_lowercase) == Some(query.clone())
                    || link.name.to_lowercase() == query
            })
            .map(|link| ArgumentRequest {
                id: link.id(),
                name: link.name.clone(),
            })
    }

    /// The id of the link whose alias or whole name `query` is, whatever its
    /// link. For ranking: typed exactly, a link is what was meant.
    pub fn exact(&self, query: &str) -> Option<String> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        self.links
            .iter()
            .find(|link| {
                link.name.eq_ignore_ascii_case(query)
                    || link
                        .alias
                        .as_deref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(query))
            })
            .map(Quicklink::id)
    }

    /// The one row shown while the launcher asks for `id`'s argument.
    ///
    /// Before anything is typed it is the link itself, opened empty, so Enter
    /// on a blank argument still does something rather than nothing. `None`
    /// once the link is gone from the file.
    pub fn argument_item(&self, id: &str, argument: &str) -> Option<Item> {
        let link = self.links.iter().find(|link| link.id() == id)?;
        if argument.trim().is_empty() {
            return Some(Item {
                id: link.id(),
                title: format!("Search {}", link.name),
                subtitle: Some(link.source.clone()),
                icon: link.icon(),
                keywords: Vec::new(),
                actions: vec![link.action("")],
                kind: Kind::Quicklink,
            });
        }
        Some(link.query_item(argument))
    }

    /// The links offered for any query, at the foot of the list.
    ///
    /// Nothing when an alias already answered: `g lofi` is a Google search for
    /// `lofi`, and offering a second one for `g lofi` beneath it is noise.
    pub fn fallbacks(&self, query: &str) -> Vec<Item> {
        let query = query.trim();
        if query.is_empty() || self.links.iter().any(|link| link.argument(query).is_some()) {
            return Vec::new();
        }
        self.links
            .iter()
            .filter(|link| link.fallback)
            .map(|link| link.query_item(query))
            .collect()
    }
}

impl Provider for QuicklinkProvider {
    fn id(&self) -> &'static str {
        "quicklink"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.links.iter().map(Quicklink::catalogue_item).collect()
    }

    fn answer(&self, query: &str) -> Vec<Item> {
        self.links
            .iter()
            .filter_map(|link| link.argument(query).map(|rest| link.query_item(rest)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, link: &str) -> Entry {
        Entry {
            name: name.into(),
            link: link.into(),
            ..Entry::default()
        }
    }

    fn google() -> Quicklink {
        Quicklink::from_entry(Entry {
            alias: Some("g".into()),
            fallback: true,
            ..entry("Google", "https://www.google.com/search?q={query}")
        })
        .unwrap()
    }

    fn channel() -> Quicklink {
        Quicklink::from_entry(entry("Veritasium", "https://www.youtube.com/@veritasium")).unwrap()
    }

    /// Without an `icon` field a link wants its site's icon; `default` keeps
    /// the glyph and is never fetched for.
    #[test]
    fn a_link_wants_its_sites_icon_unless_told_otherwise() {
        assert_eq!(google().favicon_site().as_deref(), Some("www.google.com"));

        let plain = Quicklink::from_entry(Entry {
            icon: Some("default".into()),
            ..entry("Plain", "https://example.com")
        })
        .unwrap();
        assert_eq!(plain.favicon_site(), None);
    }

    #[test]
    fn a_cached_icon_is_drawn_on_every_row_of_the_link() {
        let dir = std::env::temp_dir().join(format!("lumen-link-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(favicon::cache_file(&dir, "www.google.com"), b"png").unwrap();

        let mut links = vec![google(), channel()];
        attach_icons(&mut links, &dir);
        let provider = QuicklinkProvider::new(links);

        let expected = Some(IconSource::File(favicon::cache_file(
            &dir,
            "www.google.com",
        )));
        assert_eq!(provider.catalogue()[0].icon, expected);
        assert_eq!(provider.answer("g lofi")[0].icon, expected);
        // Not fetched yet: the glyph.
        assert_eq!(provider.catalogue()[1].icon, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A value this build does not know is kept, for the build that does.
    #[test]
    fn an_unknown_icon_value_survives_the_file() {
        let written: Entry = serde_json::from_str(
            r#"{ "name": "A", "link": "https://a.com", "icon": "emoji:rocket" }"#,
        )
        .unwrap();
        assert_eq!(written.icon.as_deref(), Some("emoji:rocket"));
        assert!(
            serde_json::to_string(&written)
                .unwrap()
                .contains("emoji:rocket")
        );
    }

    fn opened(item: &Item) -> OpenLink {
        match item.primary() {
            Action::OpenLink(open) => open,
            other => panic!("expected a link, got {other:?}"),
        }
    }

    fn browser(name: &str) -> Browser {
        Browser {
            name: name.into(),
            executable: PathBuf::from(format!("{name}.exe")),
        }
    }

    #[test]
    fn a_saved_link_is_a_catalogue_entry_that_opens_it() {
        let provider = QuicklinkProvider::new(vec![channel()]);

        let items = provider.catalogue();

        assert_eq!(items[0].title, "Veritasium");
        assert_eq!(items[0].kind, Kind::Quicklink);
        assert_eq!(
            opened(&items[0]).link.render("", ""),
            "https://www.youtube.com/@veritasium"
        );
        // Copying the link is the one other thing people want from it.
        assert_eq!(
            items[0].actions[1],
            Action::CopyToClipboard("https://www.youtube.com/@veritasium".into())
        );
    }

    #[test]
    fn the_alias_and_a_space_fill_the_query() {
        let provider = QuicklinkProvider::new(vec![google()]);

        let items = provider.answer("g lofi beats");

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].title,
            "Search Google for \u{201c}lofi beats\u{201d}"
        );
        assert_eq!(opened(&items[0]).query, "lofi beats");
        assert_eq!(
            items[0].subtitle.as_deref(),
            Some("https://www.google.com/search?q=lofi%20beats")
        );
    }

    #[test]
    fn the_name_works_as_well_as_the_alias_in_any_case() {
        let provider = QuicklinkProvider::new(vec![google()]);
        assert_eq!(
            provider.answer("GOOGLE rust")[0].title,
            "Search Google for \u{201c}rust\u{201d}"
        );
    }

    /// `gmail` starts with `g`, and is not a Google search for `mail`.
    #[test]
    fn an_alias_must_be_a_whole_word() {
        let provider = QuicklinkProvider::new(vec![google()]);
        assert!(provider.answer("gmail").is_empty());
        assert!(provider.answer("g").is_empty());
        assert!(provider.answer("g   ").is_empty());
    }

    /// Choosing "Google" asks what to search for; opening it empty is the
    /// second action, not the first.
    #[test]
    fn a_search_link_asks_for_its_argument_first() {
        let item = QuicklinkProvider::new(vec![google()]).catalogue().remove(0);

        assert_eq!(
            item.primary(),
            Action::AskArgument(ArgumentRequest {
                id: "quicklink:Google".into(),
                name: "Google".into(),
            })
        );
        assert!(matches!(item.actions[1], Action::OpenLink(_)));
    }

    #[test]
    fn the_argument_row_follows_what_is_typed() {
        let provider = QuicklinkProvider::new(vec![google()]);

        let empty = provider.argument_item("quicklink:Google", "  ").unwrap();
        assert_eq!(empty.title, "Search Google");
        assert_eq!(opened(&empty).query, "");

        let typed = provider.argument_item("quicklink:Google", "lofi").unwrap();
        assert_eq!(typed.title, "Search Google for \u{201c}lofi\u{201d}");
        assert_eq!(opened(&typed).query, "lofi");
    }

    #[test]
    fn an_exact_alias_or_name_names_its_link() {
        let provider = QuicklinkProvider::new(vec![google(), channel()]);

        assert_eq!(provider.named(" G ").unwrap().name, "Google");
        assert_eq!(provider.named("google").unwrap().name, "Google");
        assert!(provider.named("goo").is_none());
        // A link with nowhere to put a query is never asked for one.
        assert!(provider.named("veritasium").is_none());
        assert!(provider.named("").is_none());
    }

    /// The file can change while the question is on screen.
    #[test]
    fn a_link_that_is_gone_has_no_argument_row() {
        let provider = QuicklinkProvider::new(vec![channel()]);
        assert!(provider.argument_item("quicklink:Google", "x").is_none());
    }

    #[test]
    fn a_link_without_a_query_answers_nothing() {
        let provider = QuicklinkProvider::new(vec![channel()]);
        assert!(provider.answer("Veritasium physics").is_empty());
    }

    #[test]
    fn a_fallback_is_offered_for_any_query() {
        let provider = QuicklinkProvider::new(vec![google(), channel()]);

        let items = provider.fallbacks("how tall is everest");

        assert_eq!(items.len(), 1);
        assert_eq!(opened(&items[0]).query, "how tall is everest");
    }

    #[test]
    fn no_fallback_for_an_empty_query_or_one_an_alias_answered() {
        let provider = QuicklinkProvider::new(vec![google()]);
        assert!(provider.fallbacks("   ").is_empty());
        assert!(provider.fallbacks("g lofi").is_empty());
    }

    #[test]
    fn a_broken_entry_is_refused_with_its_name() {
        let error = Quicklink::from_entry(entry("Typo", "https://x.com/?q={qeury}")).unwrap_err();
        assert!(error.contains("Typo"), "{error}");

        assert!(Quicklink::from_entry(entry("  ", "https://x.com")).is_err());
        assert!(
            Quicklink::from_entry(Entry {
                alias: Some("two words".into()),
                ..entry("Spaced", "https://x.com/{query}")
            })
            .is_err()
        );
        assert!(
            Quicklink::from_entry(Entry {
                fallback: true,
                ..entry("Nowhere to put it", "https://x.com")
            })
            .is_err()
        );
    }

    #[test]
    fn the_preferred_browser_does_not_override_the_links_own() {
        let open = OpenLink {
            link: Template::parse("https://x.com").unwrap(),
            query: String::new(),
            browser: Some("Zen".into()),
        };
        assert_eq!(
            open.clone().or_browser(Some("Brave")).browser.as_deref(),
            Some("Zen")
        );

        let open = OpenLink {
            browser: None,
            ..open
        };
        assert_eq!(
            open.clone().or_browser(Some("Brave")).browser.as_deref(),
            Some("Brave")
        );
        assert_eq!(open.or_browser(None).browser, None);
    }

    /// The case that decided matching on names: Zen's registration key is
    /// `Firefox-<hash>`, and Helium's program is `chrome.exe`.
    #[test]
    fn a_browser_is_chosen_by_its_name_or_a_part_of_it() {
        let installed = [browser("Zen Browser"), browser("Helium"), browser("Brave")];

        assert_eq!(
            choose_browser("zen", &installed).unwrap().name,
            "Zen Browser"
        );
        assert_eq!(choose_browser("HELIUM", &installed).unwrap().name, "Helium");
        assert_eq!(choose_browser(" Brave ", &installed).unwrap().name, "Brave");
        assert!(choose_browser("firefox", &installed).is_none());
        assert!(choose_browser("", &installed).is_none());
    }

    #[test]
    fn a_whole_name_wins_over_parts_and_an_ambiguous_part_finds_nothing() {
        let installed = [browser("Edge"), browser("Microsoft Edge Dev")];
        assert_eq!(choose_browser("edge", &installed).unwrap().name, "Edge");

        let installed = [browser("Microsoft Edge"), browser("Microsoft Edge Dev")];
        assert!(choose_browser("microsoft", &installed).is_none());
    }

    #[test]
    fn one_bad_entry_does_not_lose_the_others() {
        let dir = std::env::temp_dir().join(format!("lumen-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quicklinks.json");
        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "quicklinks": [
                    { "name": "Good", "link": "https://example.com" },
                    { "name": "Bad", "link": "example.com" }
                ]
            }"#,
        )
        .unwrap();

        let links = load(&path);

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "Good");
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        assert!(load(Path::new("no/such/quicklinks.json")).is_empty());
    }

    #[test]
    fn only_a_web_link_is_offered_from_the_clipboard() {
        assert!(looks_like_link("https://www.youtube.com/@veritasium"));
        assert!(looks_like_link("  HTTP://example.com/a?b=c  "));

        assert!(!looks_like_link("https://"));
        assert!(!looks_like_link("mailto:someone@example.com"));
        assert!(!looks_like_link("see https://example.com for details"));
        assert!(!looks_like_link("note: remember this"));
        // Braces would be read as a placeholder.
        assert!(!looks_like_link("https://example.com/{id}"));
    }

    /// Parsed by this test at build time, so a broken example cannot ship --
    /// and every entry must survive the per-entry checks too, not just parse.
    #[test]
    fn the_bundled_example_parses_and_offers_google_as_the_fallback() {
        let file: QuicklinkFile = serde_json::from_str(EXAMPLE).expect("the example must parse");
        assert_eq!(file.schema_version, SCHEMA_VERSION);

        let count = file.quicklinks.len();
        let links: Vec<Quicklink> = file
            .quicklinks
            .into_iter()
            .map(|entry| Quicklink::from_entry(entry).unwrap())
            .collect();
        assert_eq!(links.len(), count);

        let provider = QuicklinkProvider::new(links);
        let fallbacks = provider.fallbacks("anything");
        assert_eq!(fallbacks.len(), 1);
        assert!(fallbacks[0].title.contains("Google"));
    }
}
