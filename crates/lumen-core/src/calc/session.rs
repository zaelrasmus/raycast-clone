//! The *Calculator History* view (F14), as decisions without a window.
//!
//! `lumen-app` owns the file and the clipboard; this decides what is shown
//! and what `Enter` does, so all of it is tested here. The shape is
//! *Clipboard History*'s: sections by date, and the list filtered by every
//! word typed. A row is the root list's answer again -- the result, and what
//! was typed under it -- so a calculation looks the same wherever it is.

use jiff::{Timestamp, Zoned};

use super::history::Entry;
use crate::view::{Action, EmptyState, Event, Filtering, Icon, Item, List, Section, View};

/// What `Enter` does, and the rest of the action panel.
pub const COPY: &str = "copy";
pub const PASTE: &str = "paste";
pub const COPY_EXPRESSION: &str = "copy-expression";
pub const DELETE: &str = "delete";

/// What the daemon is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Put the entry's result on the clipboard, close, and keep the entry
    /// again, first: using it again is using it.
    Copy(Entry),
    /// Close and paste the entry's result into the window the launcher was
    /// opened over, then keep it again, first.
    Paste(Entry),
    /// Put this text on the clipboard and close.
    CopyText(String),
    /// Forget the entry for this expression, then read the history again
    /// and hand it to [`Session::reload`].
    Delete(String),
}

pub struct Session {
    /// Newest first.
    entries: Vec<Entry>,
    /// Whether history is kept at all: `calculator.history-limit` is `0`.
    off: bool,
    /// The words typed, in lower case.
    words: Vec<String>,
    /// When the view opened, in the user's time zone: what *Today* means.
    now: Zoned,
}

impl Session {
    /// `entries` newest first. `off` when the configuration keeps none.
    pub fn new(entries: Vec<Entry>, off: bool, now: Zoned) -> Self {
        Self {
            entries,
            off,
            words: Vec::new(),
            now,
        }
    }

    /// The history again, after an entry went: the search is kept.
    pub fn reload(&mut self, entries: Vec<Entry>) -> Vec<Effect> {
        self.entries = entries;
        vec![Effect::Render(self.view())]
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
            Event::ActionInvoked {
                item: Some(item),
                action,
            } => {
                let Some(entry) = item
                    .parse::<usize>()
                    .ok()
                    .and_then(|at| self.entries.get(at))
                else {
                    return Vec::new();
                };
                match action.as_str() {
                    COPY => vec![Effect::Copy(entry.clone())],
                    PASTE => vec![Effect::Paste(entry.clone())],
                    COPY_EXPRESSION => vec![Effect::CopyText(entry.expression.clone())],
                    DELETE => vec![Effect::Delete(entry.expression.clone())],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn shown(&self, entry: &Entry) -> bool {
        let haystack = format!("{} {}", entry.expression, entry.result).to_lowercase();
        self.words
            .iter()
            .all(|word| haystack.contains(word.as_str()))
    }

    pub fn view(&self) -> View {
        let today = self.now.date();
        let mut sections: Vec<Section> = Vec::new();

        for (at, entry) in self.entries.iter().enumerate() {
            if !self.shown(entry) {
                continue;
            }
            let when = Timestamp::from_second(entry.at)
                .ok()
                .map(|when| crate::dates::day(when, &self.now));
            let title = crate::dates::period(when, today);
            if sections.last().and_then(|section| section.title.as_deref()) != Some(title) {
                sections.push(Section {
                    title: Some(title.into()),
                    subtitle: None,
                    items: Vec::new(),
                });
            }
            sections
                .last_mut()
                .expect("pushed above")
                .items
                .push(item(at, entry));
        }

        let empty = if self.off {
            EmptyState {
                title: "Calculator history is off".into(),
                description: Some("Keep some in Settings, on the Calculator page".into()),
                icon: Some(glyph()),
            }
        } else if self.entries.is_empty() {
            EmptyState {
                title: "No calculations yet".into(),
                description: Some("Copy an answer from the root list to keep it here".into()),
                icon: Some(glyph()),
            }
        } else {
            EmptyState {
                title: "No matching calculations".into(),
                description: None,
                icon: Some(glyph()),
            }
        };

        View::List(List {
            placeholder: Some("Type to filter calculations\u{2026}".into()),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            path: Vec::new(),
            empty: Some(empty),
            showing_detail: false,
            sections,
        })
    }
}

fn item(at: usize, entry: &Entry) -> Item {
    Item {
        id: at.to_string(),
        title: entry.result.clone(),
        subtitle: Some(entry.expression.clone()),
        icon: Some(glyph()),
        accessories: Vec::new(),
        keywords: Vec::new(),
        actions: vec![
            action(COPY, "Copy Result"),
            action(PASTE, "Paste Result"),
            action(COPY_EXPRESSION, "Copy Expression"),
            action(DELETE, "Delete Entry"),
        ],
        detail: None,
    }
}

fn glyph() -> Icon {
    Icon::Glyph {
        name: "calculator".into(),
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
        icon: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Thursday evening, in a zone that is not UTC.
    fn now() -> Zoned {
        "2026-10-01T21:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn entry(expression: &str, result: &str, when: &str) -> Entry {
        Entry {
            expression: expression.into(),
            result: result.into(),
            at: when.parse::<Timestamp>().unwrap().as_second(),
        }
    }

    fn history() -> Vec<Entry> {
        vec![
            entry("25% off 399", "299.25", "2026-10-01T20:00:00-04:00"),
            entry("5 km in mi", "3.1068559612 mi", "2026-10-01T09:00:00-04:00"),
            entry("2^10", "1024", "2026-09-30T23:00:00-04:00"),
        ]
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

    fn invoke(session: &mut Session, at: usize, action: &str) -> Vec<Effect> {
        session.event(Event::ActionInvoked {
            item: Some(at.to_string()),
            action: action.into(),
        })
    }

    #[test]
    fn the_result_leads_and_what_was_typed_follows_by_day() {
        let session = Session::new(history(), false, now());
        let view = session.view();
        let sections: Vec<(String, Vec<String>)> = list(&view)
            .sections
            .iter()
            .map(|section| {
                let titles = section
                    .items
                    .iter()
                    .map(|item| item.title.clone())
                    .collect();
                (section.title.clone().unwrap(), titles)
            })
            .collect();
        assert_eq!(
            sections,
            [
                (
                    "Today".into(),
                    vec!["299.25".into(), "3.1068559612 mi".into()]
                ),
                ("Yesterday".into(), vec!["1024".into()]),
            ]
        );
        let first = list(&view).items().next().unwrap();
        assert_eq!(first.subtitle.as_deref(), Some("25% off 399"));
        assert_eq!(
            first.actions[0].id, COPY,
            "Enter copies, as in the root list"
        );
    }

    #[test]
    fn every_word_typed_narrows_against_both() {
        let mut session = Session::new(history(), false, now());
        session.event(Event::QueryChanged { query: "MI".into() });
        assert_eq!(titles(&session), ["3.1068559612 mi"]);
        session.event(Event::QueryChanged {
            query: "1024".into(),
        });
        assert_eq!(titles(&session), ["1024"]);
        session.event(Event::QueryChanged {
            query: "off 399".into(),
        });
        assert_eq!(titles(&session), ["299.25"]);
    }

    #[test]
    fn the_actions_name_their_entry() {
        let mut session = Session::new(history(), false, now());
        assert_eq!(
            invoke(&mut session, 1, COPY),
            [Effect::Copy(history()[1].clone())]
        );
        assert_eq!(
            invoke(&mut session, 0, PASTE),
            [Effect::Paste(history()[0].clone())]
        );
        assert_eq!(
            invoke(&mut session, 2, COPY_EXPRESSION),
            [Effect::CopyText("2^10".into())]
        );
        assert_eq!(
            invoke(&mut session, 2, DELETE),
            [Effect::Delete("2^10".into())]
        );
        assert!(invoke(&mut session, 9, COPY).is_empty());
    }

    #[test]
    fn a_reload_keeps_the_search() {
        let mut session = Session::new(history(), false, now());
        session.event(Event::QueryChanged { query: "mi".into() });
        session.reload(history()[..1].to_vec());
        assert!(titles(&session).is_empty());
    }

    #[test]
    fn empty_says_why() {
        let empty = |session: Session| list(&session.view()).empty.clone().unwrap().title;
        assert_eq!(
            empty(Session::new(Vec::new(), true, now())),
            "Calculator history is off"
        );
        assert_eq!(
            empty(Session::new(Vec::new(), false, now())),
            "No calculations yet"
        );
    }
}
