//! *Preview Alert Sounds* (F8, slice 3): hear the system's alert sounds, and
//! choose the one a timer ends with.
//!
//! The list is the platform's -- the daemon asks for it when the view opens
//! -- and this file only decides what it looks like and what an action means.
//! `Enter` plays a sound once; *Use as Alert Sound* writes it to
//! `config.json`, and the tag moves to it.

use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, List, Section, Tone, View,
};

const SOUND: &str = "sound-";
const PLAY: &str = "play";
const USE: &str = "use";

/// What an action in the view asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Play this sound once.
    Play(String),
    /// Make this the alert sound.
    Use(String),
}

/// A sound's name as people read it: `Alarm01` is "Alarm 1".
pub fn label(name: &str) -> String {
    let split = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len());
    let (word, number) = name.split_at(split);
    match number.trim_start_matches('0') {
        "" if number.is_empty() => word.to_string(),
        "" => format!("{word} 0"),
        number => format!("{word} {number}"),
    }
}

/// The sounds, with the one in use tagged.
pub fn view(names: &[String], current: Option<&str>) -> View {
    let items = names
        .iter()
        .map(|name| Item {
            detail: None,
            id: format!("{SOUND}{name}"),
            title: label(name),
            subtitle: None,
            icon: Some(Icon::Glyph {
                name: super::view::GLYPH.into(),
            }),
            accessories: if current == Some(name.as_str()) {
                vec![Accessory::Tag {
                    text: "In use".into(),
                    tone: Tone::Positive,
                }]
            } else {
                Vec::new()
            },
            keywords: vec![name.clone()],
            actions: vec![
                Action {
                    id: PLAY.into(),
                    title: "Play".into(),
                    shortcut: None,
                },
                Action {
                    id: USE.into(),
                    title: "Use as Alert Sound".into(),
                    shortcut: None,
                },
            ],
        })
        .collect();

    View::List(List {
        showing_detail: false,
        placeholder: Some("Search alert sounds".into()),
        filtering: Filtering::Host,
        scopes: Vec::new(),
        empty: Some(EmptyState {
            title: "No alert sounds".into(),
            description: Some("The system offers none that Lumen can play.".into()),
            icon: None,
        }),
        sections: vec![Section {
            title: Some("Enter plays \u{00b7} Ctrl+K to use one".into()),
            subtitle: None,
            items,
        }],
    })
}

/// What the view reported, as a [`Choice`]; `None` for anything else.
pub fn act(event: &Event) -> Option<Choice> {
    let Event::ActionInvoked {
        item: Some(item),
        action,
    } = event
    else {
        return None;
    };
    let name = item.strip_prefix(SOUND)?.to_string();
    match action.as_str() {
        PLAY => Some(Choice::Play(name)),
        USE => Some(Choice::Use(name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["Alarm01", "Alarm10", "Ring03"].map(String::from).to_vec()
    }

    #[test]
    fn a_sound_is_named_as_people_read_it() {
        assert_eq!(label("Alarm01"), "Alarm 1");
        assert_eq!(label("Ring10"), "Ring 10");
        assert_eq!(label("Chime"), "Chime");
    }

    #[test]
    fn the_sound_in_use_is_tagged_and_only_it() {
        let View::List(list) = view(&names(), Some("Alarm10")) else {
            panic!("a list")
        };
        let tagged: Vec<_> = list
            .items()
            .filter(|item| !item.accessories.is_empty())
            .map(|item| item.title.clone())
            .collect();
        assert_eq!(tagged, ["Alarm 10"]);
    }

    #[test]
    fn enter_plays_and_the_panel_chooses() {
        let View::List(list) = view(&names(), None) else {
            panic!("a list")
        };
        let row = list.item(2).unwrap();
        let invoke = |action: &str| Event::ActionInvoked {
            item: Some(row.id.clone()),
            action: action.into(),
        };

        assert_eq!(
            act(&invoke(&row.actions[0].id)),
            Some(Choice::Play("Ring03".into()))
        );
        assert_eq!(
            act(&invoke(&row.actions[1].id)),
            Some(Choice::Use("Ring03".into()))
        );
        assert_eq!(act(&Event::Dismissed), None);
    }
}
