//! Focus (F8, slice 6): while a focus timer runs, Lumen will not open what
//! the user said distracts them.
//!
//! # A nudge through Lumen, not a wall
//!
//! Only what is opened *through Lumen* is checked: an application from the
//! root list, and a website from a quicklink, a web search or a typed URL.
//! The same application opened from the Start menu opens. That needs no
//! system hook, no administrator rights and no browser extension, and it is
//! honest about what it is: friction at the moment of reaching for the
//! distraction, with the goal and the time left in front of the user, and
//! *Open Anyway* one key away -- counted, so the timer knows.
//!
//! # One list, and a switch per timer
//!
//! The blocked apps and sites are one list in `config.json`; a timer either
//! has focus on or not. In a cycle, focus holds during work and lifts for the
//! breaks, which is what breaks are for.

use serde::{Deserialize, Serialize};

use super::duration::clock_face;
use super::{Clock, Millis, Phase, Timer};
use crate::item::{Action, Item, Kind};
use crate::view::{self, Event, Filtering, Icon, List, Section, View};

/// What focus blocks, normalised for matching.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    /// Application names, lower case.
    apps: Vec<String>,
    /// Domains, lower case, without scheme, `www.` or path.
    sites: Vec<String>,
}

impl Guard {
    pub fn new(apps: &[String], sites: &[String]) -> Self {
        let apps = apps
            .iter()
            .map(|app| app.trim().to_lowercase())
            .filter(|app| !app.is_empty())
            .collect();
        let sites = sites.iter().filter_map(|site| domain(site)).collect();
        Self { apps, sites }
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.sites.is_empty()
    }

    /// What `action` on `item` would open that focus blocks, named for the
    /// user: the application's name, or the site's host.
    pub fn blocks(&self, item: &Item, action: &Action) -> Option<String> {
        match action {
            Action::Launch(_) | Action::LaunchNew(_) if item.kind == Kind::Application => {
                let title = item.title.to_lowercase();
                self.apps
                    .iter()
                    .any(|app| title == *app || title.starts_with(&format!("{app} ")))
                    .then(|| item.title.clone())
            }
            Action::OpenLink(open) => {
                // The clipboard is not read here: it is read when the link
                // opens, and a link whose host is only in the clipboard is
                // the rare case this lets through.
                let url = open.link.render(&open.query, "");
                let host = url::Url::parse(&url).ok()?.host_str()?.to_lowercase();
                let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
                self.sites
                    .iter()
                    .any(|site| host == *site || host.ends_with(&format!(".{site}")))
                    .then_some(host)
            }
            _ => None,
        }
    }
}

/// `https://www.YouTube.com/feed` as `youtube.com`; nothing for nothing.
fn domain(site: &str) -> Option<String> {
    let site = site.trim().to_lowercase();
    let site = site
        .split_once("://")
        .map_or(site.as_str(), |(_, rest)| rest);
    let site = site.split(['/', '?', '#']).next().unwrap_or_default();
    let site = site.strip_prefix("www.").unwrap_or(site);
    (!site.is_empty()).then(|| site.to_string())
}

impl Timer {
    /// Whether focus holds right now: on, not over, and not on a break.
    pub fn focusing(&self) -> bool {
        self.focus
            && !self.is_ringing()
            && self.cycle.is_none_or(|cycle| cycle.phase == Phase::Work)
    }
}

impl Clock {
    /// The timer holding focus, if one is: the one ending soonest.
    pub fn focus(&self, now: Millis) -> Option<&Timer> {
        self.timers()
            .iter()
            .filter(|timer| timer.focusing())
            .min_by_key(|timer| timer.remaining(now))
    }

    /// Turn focus on or off for a timer. Returns whether anything changed.
    pub fn set_focus(&mut self, id: u64, on: bool) -> bool {
        let Some(timer) = self.get_mut(id) else {
            return false;
        };
        if timer.focus == on {
            return false;
        }
        timer.focus = on;
        true
    }

    /// Something blocked was opened anyway, while `id` held focus.
    pub fn count_override(&mut self, id: u64) {
        if let Some(timer) = self.get_mut(id) {
            timer.overrides += 1;
        }
    }
}

// Ids of the refusal's actions. Opaque to the host.
const BACK: &str = "back";
const OPEN: &str = "open-anyway";

/// What the user chose when something was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BackToFocus,
    OpenAnyway,
}

/// The refusal, in place of the list: what was blocked, by what, how long
/// is left, and the two ways out -- the first, which `Enter` takes, the one
/// the timer was started for.
pub fn refusal(what: &str, timer: Option<&Timer>, now: Millis) -> View {
    let subtitle = match timer {
        Some(timer) => {
            let phase = timer
                .cycle
                .map(|cycle| format!(" \u{00b7} {}", cycle.label().to_lowercase()))
                .unwrap_or_default();
            format!(
                "Focus: {}{phase} \u{00b7} {} left",
                timer.title(),
                clock_face(timer.remaining(now))
            )
        }
        None => "Focus is on".into(),
    };
    let action = |id: &str, title: &str| view::Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    };
    View::List(List {
        placeholder: Some("Esc to go back".into()),
        filtering: Filtering::Host,
        scopes: Vec::new(),
        empty: None,
        sections: vec![Section {
            title: Some("Blocked while you focus".into()),
            items: vec![view::Item {
                id: "blocked".into(),
                title: format!("{what} is blocked"),
                subtitle: Some(subtitle),
                icon: Some(Icon::Glyph {
                    name: super::view::GLYPH.into(),
                }),
                accessories: Vec::new(),
                keywords: Vec::new(),
                actions: vec![action(BACK, "Back to Focus"), action(OPEN, "Open Anyway")],
            }],
        }],
    })
}

/// What the refusal's view reported.
pub fn refused(event: &Event) -> Option<Refusal> {
    match event {
        Event::ActionInvoked { action, .. } if action == BACK => Some(Refusal::BackToFocus),
        Event::ActionInvoked { action, .. } if action == OPEN => Some(Refusal::OpenAnyway),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::providers::quicklinks::{OpenLink, Template};
    use lumen_platform::LaunchTarget;

    fn guard() -> Guard {
        Guard::new(
            &["Discord".into(), " steam ".into(), String::new()],
            &["https://www.YouTube.com/feed".into(), "reddit.com".into()],
        )
    }

    fn app(title: &str) -> (Item, Action) {
        let action = Action::Launch(LaunchTarget::ShellExecute(title.into()));
        let item = Item {
            id: title.into(),
            title: title.into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            actions: vec![action.clone()],
            kind: Kind::Application,
        };
        (item, action)
    }

    fn link(url: &str, query: &str) -> (Item, Action) {
        let action = Action::OpenLink(OpenLink {
            link: Template::parse(url).unwrap(),
            query: query.into(),
            browser: None,
            private: false,
        });
        let (mut item, _) = app("link");
        item.kind = Kind::Quicklink;
        (item, action)
    }

    #[test]
    fn a_listed_app_is_blocked_by_its_name_and_its_editions() {
        let guard = guard();
        for title in ["Discord", "discord", "Discord PTB", "Steam"] {
            let (item, action) = app(title);
            assert_eq!(
                guard.blocks(&item, &action).as_deref(),
                Some(title),
                "{title}"
            );
        }
        for title in ["Discordia Notes", "Steamworks", "Firefox"] {
            let (item, action) = app(title);
            assert_eq!(guard.blocks(&item, &action), None, "{title}");
        }
    }

    /// Copying an app's name opens nothing.
    #[test]
    fn only_opening_is_blocked() {
        let (item, _) = app("Discord");
        assert_eq!(
            guard().blocks(&item, &Action::CopyToClipboard("Discord".into())),
            None
        );
    }

    #[test]
    fn a_listed_site_is_blocked_with_its_subdomains_and_searches() {
        let guard = guard();
        for (url, query) in [
            ("https://www.youtube.com/watch?v=x", ""),
            ("https://m.youtube.com", ""),
            (
                "https://www.youtube.com/results?search_query={query}",
                "lofi",
            ),
            ("https://old.reddit.com/r/rust", ""),
        ] {
            let (item, action) = link(url, query);
            assert!(guard.blocks(&item, &action).is_some(), "{url}");
        }
        let (item, action) = link("https://notyoutube.com", "");
        assert_eq!(guard.blocks(&item, &action), None);
        let (item, action) = link("https://www.google.com/search?q={query}", "youtube.com");
        assert_eq!(guard.blocks(&item, &action), None);
    }

    #[test]
    fn a_site_is_written_any_way_and_read_as_its_domain() {
        assert_eq!(
            domain("https://www.YouTube.com/feed").as_deref(),
            Some("youtube.com")
        );
        assert_eq!(domain(" reddit.com ").as_deref(), Some("reddit.com"));
        assert_eq!(domain("   "), None);
    }

    #[test]
    fn focus_holds_during_work_and_lifts_for_breaks() {
        let mut clock = Clock::default();
        let id = clock.start_cycle(super::super::Plan::POMODORO, None, 0);
        assert!(clock.focus(0).is_none());

        assert!(clock.set_focus(id, true));
        assert_eq!(clock.focus(0).map(|timer| timer.id), Some(id));

        clock.skip(id, 0);
        assert!(clock.focus(0).is_none(), "a break is free");
        clock.skip(id, 0);
        assert!(clock.focus(0).is_some());
    }

    #[test]
    fn a_paused_focus_timer_still_holds_and_a_ringing_one_does_not() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        clock.set_focus(id, true);
        clock.pause(id, 0);
        assert!(clock.focus(0).is_some(), "pausing is not a way out");

        clock.resume(id, 0);
        clock.expire(60_000, true);
        assert!(clock.focus(60_000).is_none());
    }

    #[test]
    fn opening_anyway_is_counted_on_the_timer() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(60), None, 0);
        clock.count_override(id);
        clock.count_override(id);
        assert_eq!(clock.get(id).unwrap().overrides, 2);
    }

    #[test]
    fn the_refusal_says_what_by_whom_and_enter_goes_back() {
        let mut clock = Clock::default();
        let id = clock.start(Duration::from_secs(25 * 60), Some("Write".into()), 0);
        clock.set_focus(id, true);

        let View::List(list) = refusal("Discord", clock.get(id), 60_000) else {
            panic!("a list")
        };
        let row = list.item(0).unwrap();
        assert_eq!(row.title, "Discord is blocked");
        assert_eq!(
            row.subtitle.as_deref(),
            Some("Focus: Write \u{00b7} 24:00 left")
        );

        let invoke = |action: &str| Event::ActionInvoked {
            item: Some(row.id.clone()),
            action: action.into(),
        };
        assert_eq!(
            refused(&invoke(&row.actions[0].id)),
            Some(Refusal::BackToFocus)
        );
        assert_eq!(
            refused(&invoke(&row.actions[1].id)),
            Some(Refusal::OpenAnyway)
        );
        assert_eq!(refused(&Event::Dismissed), None);
    }
}
