//! The *Search Snippets* view (F11), as decisions without a window.
//!
//! `lumen-app` owns the file and the clipboard; this decides what is shown and
//! what `Enter` does, so all of it is tested here.
//!
//! # The session filters, not the host
//!
//! A snippet is found by what it says as well as by its name. The host's
//! matcher is fuzzy, and fuzzy against a paragraph matches nearly any short
//! query -- and host filtering keeps the view's order rather than the score's
//! (7.30), so the loose matches would not even sink. So the list asks to filter
//! itself, and a snippet is kept when every word typed appears in it.

use jiff::Zoned;
use jiff::civil::Date;

use super::{Format, Snippet};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow,
    Scope, ScopeOption, Section, Tone, View,
};

/// The id of the tag scope.
pub const TAG_SCOPE: &str = "tag";
/// The option that shows every snippet. Empty, because a tag never is.
const ALL_TAGS: &str = "";
/// The one action of slice 1.
pub const COPY: &str = "copy";

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Put this on the clipboard and close the launcher.
    Copy(String),
}

pub struct Session {
    /// Newest first, each with what it is matched against, in lower case.
    snippets: Vec<(Snippet, String)>,
    /// The words typed, in lower case.
    words: Vec<String>,
    /// `None` is every tag.
    tag: Option<String>,
    /// When the view opened, in the user's time zone: what *Today* means.
    now: Zoned,
}

impl Session {
    pub fn new(mut snippets: Vec<Snippet>, now: Zoned) -> Self {
        // Stable, so snippets with the same date, or none, keep the file's
        // order; those with none go last.
        snippets.sort_by_key(|snippet| std::cmp::Reverse(snippet.changed()));
        Self {
            snippets: snippets
                .into_iter()
                .map(|snippet| {
                    let haystack = haystack(&snippet);
                    (snippet, haystack)
                })
                .collect(),
            words: Vec::new(),
            tag: None,
            now,
        }
    }

    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
                if words == self.words {
                    return Vec::new();
                }
                self.words = words;
                vec![Effect::Render(self.view())]
            }
            Event::ScopeChanged { scope, option } if scope == TAG_SCOPE => {
                let tag = (option != ALL_TAGS).then_some(option);
                if tag == self.tag {
                    return Vec::new();
                }
                self.tag = tag;
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked {
                item: Some(item),
                action,
            } if action == COPY => self
                .snippet(&item)
                .map(|snippet| vec![Effect::Copy(snippet.text.clone())])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The snippet an item's id names: its place in the sorted list.
    fn snippet(&self, id: &str) -> Option<&Snippet> {
        let index: usize = id.parse().ok()?;
        self.snippets.get(index).map(|(snippet, _)| snippet)
    }

    fn shown(&self, snippet: &Snippet, haystack: &str) -> bool {
        let tagged = match &self.tag {
            None => true,
            Some(wanted) => snippet.tags().any(|tag| tag.eq_ignore_ascii_case(wanted)),
        };
        tagged
            && self
                .words
                .iter()
                .all(|word| haystack.contains(word.as_str()))
    }

    pub fn view(&self) -> View {
        let today = self.now.date();
        let mut sections: Vec<Section> = Vec::new();

        for (index, (snippet, haystack)) in self.snippets.iter().enumerate() {
            if !self.shown(snippet, haystack) {
                continue;
            }
            let when = snippet
                .changed()
                .map(|at| at.to_zoned(self.now.time_zone().clone()).date());
            let title = period(when, today);
            if sections.last().and_then(|section| section.title.as_deref()) != Some(title) {
                sections.push(Section {
                    title: Some(title.into()),
                    subtitle: None,
                    items: Vec::new(),
                });
            }
            let item = self.item(index, snippet);
            sections.last_mut().expect("pushed above").items.push(item);
        }

        let empty = if self.snippets.is_empty() {
            EmptyState {
                title: "No snippets yet".into(),
                description: Some("Snippets are kept in snippets.json, in Lumen's folder".into()),
                icon: Some(glyph()),
            }
        } else {
            EmptyState {
                title: "No matching snippets".into(),
                description: None,
                icon: Some(glyph()),
            }
        };

        View::List(List {
            placeholder: Some("Search snippets\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: vec![self.tag_scope()],
            empty: Some(empty),
            showing_detail: true,
            sections,
        })
    }

    /// *All Tags*, then every tag in use, sorted, each spelled as it was
    /// first met.
    fn tag_scope(&self) -> Scope {
        let mut tags: Vec<&str> = Vec::new();
        for tag in self.snippets.iter().flat_map(|(snippet, _)| snippet.tags()) {
            if !tags.iter().any(|seen| seen.eq_ignore_ascii_case(tag)) {
                tags.push(tag);
            }
        }
        tags.sort_by_key(|tag| tag.to_lowercase());

        let options = std::iter::once(ScopeOption {
            id: ALL_TAGS.into(),
            title: "All Tags".into(),
            keywords: Vec::new(),
        })
        .chain(tags.into_iter().map(|tag| ScopeOption {
            id: tag.into(),
            title: tag.into(),
            keywords: Vec::new(),
        }))
        .collect();

        Scope {
            id: TAG_SCOPE.into(),
            title: "Tag".into(),
            options,
            selected: self.tag.clone().unwrap_or_default(),
        }
    }

    fn item(&self, index: usize, snippet: &Snippet) -> Item {
        Item {
            id: index.to_string(),
            title: snippet.name.trim().into(),
            subtitle: None,
            icon: Some(glyph()),
            accessories: snippet
                .keyword()
                .map(|keyword| Accessory::Tag {
                    text: keyword.into(),
                    tone: Tone::Neutral,
                })
                .into_iter()
                .collect(),
            keywords: Vec::new(),
            actions: vec![Action {
                id: COPY.into(),
                title: "Copy".into(),
                shortcut: None,
            }],
            detail: Some(self.detail(snippet)),
        }
    }

    fn detail(&self, snippet: &Snippet) -> ItemDetail {
        let mut metadata = vec![row("Name", snippet.name.trim())];
        if let Some(keyword) = snippet.keyword() {
            metadata.push(row("Keyword", keyword));
        }
        let tags: Vec<&str> = snippet.tags().collect();
        if !tags.is_empty() {
            metadata.push(MetadataRow {
                label: "Tags".into(),
                value: tags.join(", "),
                tone: Tone::Accent,
            });
        }
        metadata.push(row("Content Type", snippet.format.label()));
        if let Some(at) = snippet.modified_at() {
            metadata.push(row("Modified", &self.moment(at)));
        }
        if let Some(at) = snippet.created_at() {
            metadata.push(row("Created", &self.moment(at)));
        }

        match snippet.format {
            Format::Markdown => ItemDetail {
                markdown: snippet.text.clone(),
                text: None,
                metadata,
            },
            Format::Text | Format::Unknown => ItemDetail {
                markdown: String::new(),
                text: Some(snippet.text.clone()),
                metadata,
            },
        }
    }

    /// *Today, 9:00 PM*, *Yesterday, 9:00 PM*, or *Sep 3, 2026, 9:00 PM*.
    fn moment(&self, at: jiff::Timestamp) -> String {
        let zoned = at.to_zoned(self.now.time_zone().clone());
        let today = self.now.date();
        let time = zoned.strftime("%-I:%M %p");
        if zoned.date() == today {
            format!("Today, {time}")
        } else if Some(zoned.date()) == today.yesterday().ok() {
            format!("Yesterday, {time}")
        } else {
            format!("{}, {time}", zoned.strftime("%b %-d, %Y"))
        }
    }
}

/// The section a date falls in, seen from `today`. A week starts on Monday;
/// a date in the future is *Today*, since a clock that was ahead is the
/// likely reason.
fn period(when: Option<Date>, today: Date) -> &'static str {
    let Some(when) = when else {
        return "Earlier";
    };
    if when >= today {
        return "Today";
    }
    if Some(when) == today.yesterday().ok() {
        return "Yesterday";
    }
    let into_week = i64::from(today.weekday().to_monday_zero_offset());
    let monday = today.checked_sub(jiff::Span::new().days(into_week)).ok();
    if monday.is_some_and(|monday| when >= monday) {
        return "This Week";
    }
    if when.year() == today.year() && when.month() == today.month() {
        return "This Month";
    }
    "Earlier"
}

/// Everything a word typed may be found in, in lower case.
fn haystack(snippet: &Snippet) -> String {
    let mut haystack = snippet.name.to_lowercase();
    for part in snippet
        .keyword()
        .into_iter()
        .chain(snippet.keywords.iter().map(String::as_str))
        .chain(snippet.tags())
        .chain(std::iter::once(snippet.text.as_str()))
    {
        haystack.push('\n');
        haystack.push_str(&part.to_lowercase());
    }
    haystack
}

fn glyph() -> Icon {
    Icon::Glyph {
        name: "snippet".into(),
    }
}

fn row(label: &str, value: &str) -> MetadataRow {
    MetadataRow {
        label: label.into(),
        value: value.into(),
        tone: Tone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Wednesday evening, in a zone that is not UTC, so a test that forgot
    /// the zone would be a day off.
    fn now() -> Zoned {
        "2026-09-30T21:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn snippet(name: &str, text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            text: text.into(),
            keywords: Vec::new(),
            keyword: None,
            tags: Vec::new(),
            created: None,
            modified: None,
            format: Format::Text,
        }
    }

    fn dated(name: &str, modified: &str) -> Snippet {
        Snippet {
            modified: Some(modified.into()),
            ..snippet(name, "text")
        }
    }

    fn tagged(name: &str, tags: &[&str]) -> Snippet {
        Snippet {
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            ..snippet(name, "text")
        }
    }

    fn list(view: &View) -> &List {
        match view {
            View::List(list) => list,
            View::Detail(_) => panic!("expected a list"),
        }
    }

    fn titles(session: &Session) -> Vec<String> {
        list(&session.view())
            .items()
            .map(|item| item.title.clone())
            .collect()
    }

    fn sections(session: &Session) -> Vec<(String, usize)> {
        list(&session.view())
            .sections
            .iter()
            .map(|section| (section.title.clone().unwrap(), section.items.len()))
            .collect()
    }

    fn typed(session: &mut Session, query: &str) {
        session.event(Event::QueryChanged {
            query: query.into(),
        });
    }

    #[test]
    fn newest_first_in_sections_by_when_they_changed() {
        let session = Session::new(
            vec![
                snippet("Undated", "x"),
                // 01:30 UTC on the 30th is still the 29th in New York.
                dated("Last night", "2026-09-30T01:30:00Z"),
                dated("This morning", "2026-09-30T13:00:00Z"),
                dated("Monday", "2026-09-28T15:00:00Z"),
                dated("Earlier this month", "2026-09-02T15:00:00Z"),
                dated("August", "2026-08-31T15:00:00Z"),
            ],
            now(),
        );

        assert_eq!(
            titles(&session),
            [
                "This morning",
                "Last night",
                "Monday",
                "Earlier this month",
                "August",
                "Undated"
            ]
        );
        assert_eq!(
            sections(&session),
            [
                ("Today".to_string(), 1),
                ("Yesterday".to_string(), 1),
                ("This Week".to_string(), 1),
                ("This Month".to_string(), 1),
                ("Earlier".to_string(), 2),
            ]
        );
    }

    #[test]
    fn every_word_typed_must_appear_somewhere_in_the_snippet() {
        let mut session = Session::new(
            vec![
                snippet("Address", "1 Example Street\nSpringfield"),
                Snippet {
                    keyword: Some(";sig".into()),
                    ..snippet("Signature", "Best,\nYour Name")
                },
                tagged("Standup", &["Work"]),
            ],
            now(),
        );

        typed(&mut session, "spring");
        assert_eq!(titles(&session), ["Address"]);

        typed(&mut session, "  EXAMPLE   street ");
        assert_eq!(titles(&session), ["Address"]);

        typed(&mut session, ";sig");
        assert_eq!(titles(&session), ["Signature"]);

        typed(&mut session, "work");
        assert_eq!(titles(&session), ["Standup"]);

        // Loose letters are not a match, as they would be to a fuzzy matcher.
        typed(&mut session, "adt");
        assert!(titles(&session).is_empty());
    }

    #[test]
    fn the_tag_scope_lists_each_tag_once_and_narrows_the_list() {
        let mut session = Session::new(
            vec![
                tagged("One", &["work", "email"]),
                tagged("Two", &["Work"]),
                tagged("Three", &[]),
            ],
            now(),
        );

        let view = session.view();
        let scope = &list(&view).scopes[0];
        let options: Vec<&str> = scope.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(options, ["All Tags", "email", "work"]);
        assert_eq!(scope.current().unwrap().title, "All Tags");

        let effects = session.event(Event::ScopeChanged {
            scope: TAG_SCOPE.into(),
            option: "work".into(),
        });
        assert_eq!(effects.len(), 1);
        // Tags are compared without case: "Work" is "work".
        assert_eq!(titles(&session), ["One", "Two"]);

        session.event(Event::ScopeChanged {
            scope: TAG_SCOPE.into(),
            option: ALL_TAGS.into(),
        });
        assert_eq!(titles(&session).len(), 3);
    }

    #[test]
    fn enter_copies_the_snippet_it_was_pressed_on() {
        let mut session = Session::new(
            vec![
                dated("Old", "2026-01-01T00:00:00Z"),
                Snippet {
                    text: "new text".into(),
                    ..dated("New", "2026-09-30T13:00:00Z")
                },
            ],
            now(),
        );
        let id = list(&session.view()).item(0).unwrap().id.clone();

        let effects = session.event(Event::ActionInvoked {
            item: Some(id),
            action: COPY.into(),
        });

        assert_eq!(effects, [Effect::Copy("new text".into())]);
    }

    #[test]
    fn an_unknown_item_or_action_does_nothing() {
        let mut session = Session::new(vec![snippet("A", "a")], now());
        for (item, action) in [("9", COPY), ("x", COPY), ("0", "delete")] {
            let effects = session.event(Event::ActionInvoked {
                item: Some(item.into()),
                action: action.into(),
            });
            assert!(effects.is_empty(), "{item} {action}");
        }
    }

    #[test]
    fn plain_text_is_shown_as_written_and_markdown_as_markdown() {
        let session = Session::new(
            vec![
                snippet("Plain", "a * b\nc"),
                Snippet {
                    format: Format::Markdown,
                    ..snippet("Rich", "**bold**")
                },
            ],
            now(),
        );
        let view = session.view();
        let items: Vec<&Item> = list(&view).items().collect();

        let plain = items[0].detail.as_ref().unwrap();
        assert_eq!(plain.text.as_deref(), Some("a * b\nc"));
        assert!(plain.markdown.is_empty());

        let rich = items[1].detail.as_ref().unwrap();
        assert_eq!(rich.markdown, "**bold**");
        assert_eq!(rich.text, None);
    }

    #[test]
    fn the_pane_names_the_dates_in_the_users_time_zone() {
        let session = Session::new(
            vec![Snippet {
                keyword: Some("tks".into()),
                tags: vec!["lasd".into()],
                created: Some("2026-09-03T01:00:19Z".into()),
                modified: Some("2026-09-30T01:00:19Z".into()),
                ..snippet("Test Snippet", "This is some test")
            }],
            now(),
        );
        let view = session.view();
        let item = list(&view).item(0).unwrap();
        let rows: Vec<(&str, &str)> = item
            .detail
            .as_ref()
            .unwrap()
            .metadata
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();

        assert_eq!(
            rows,
            [
                ("Name", "Test Snippet"),
                ("Keyword", "tks"),
                ("Tags", "lasd"),
                ("Content Type", "Text"),
                ("Modified", "Yesterday, 9:00 PM"),
                ("Created", "Sep 2, 2026, 9:00 PM"),
            ]
        );
        assert_eq!(
            item.accessories,
            [Accessory::Tag {
                text: "tks".into(),
                tone: Tone::Neutral
            }]
        );
    }

    #[test]
    fn an_empty_file_and_an_empty_search_say_different_things() {
        let none = Session::new(Vec::new(), now());
        assert_eq!(
            list(&none.view()).empty.as_ref().unwrap().title,
            "No snippets yet"
        );

        let mut some = Session::new(vec![snippet("A", "a")], now());
        typed(&mut some, "zzz");
        assert_eq!(
            list(&some.view()).empty.as_ref().unwrap().title,
            "No matching snippets"
        );
    }

    #[test]
    fn the_same_query_again_draws_nothing_new() {
        let mut session = Session::new(vec![snippet("A", "a")], now());
        typed(&mut session, "a");
        assert!(
            session
                .event(Event::QueryChanged {
                    query: " a ".into()
                })
                .is_empty()
        );
    }
}
