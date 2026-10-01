//! The World Clock view's state, as a plain value (F13).
//!
//! The daemon owns the minute tick and the clipboard; this owns every
//! decision. It is told what happened and replies with [`Effect`]s. It
//! reads no clock -- `now` is passed in -- and writes no time itself: the
//! platform writes times as the user's regional settings do, through the
//! function it is given, so a 12-hour machine reads `9:14 AM`.
//!
//! Browsing -- the words, the levels, the pill -- is `places::browse`,
//! shared with Weather; this says what a row of time shows.

use std::collections::HashMap;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use lumen_platform::LocalTime;

use super::{ClockConfig, Daylight, DifferenceFormat, RowLayout};
use crate::places::browse::{self, Browse, CONTINENT, COUNTRY, Rows, grouped};
use crate::places::{Catalogue, City, Continent, Favourites};
use crate::view::{
    Accessory, Action, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow, Section, Tone,
    View,
};

/// What the driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Empty the search field: a level was gone into, and what was typed
    /// found it.
    ClearQuery,
    /// Put this on the clipboard and close the launcher.
    Copy(String),
    /// Write *My Places* to `places.json` (F13, slice 2).
    Save(Favourites),
}

pub use crate::places::browse::CITY_ROWS;

// Ids. Opaque to the host; only this file reads them back.
const HERE: &str = "here";
const OPEN: &str = "open";
const COPY_TIME: &str = "copy-time";
const COPY_SENTENCE: &str = "copy-sentence";
const DETAILS: &str = "details";
const KEEP: &str = "keep";
const FORGET: &str = "forget";

/// Writes a moment as the user's regional settings do.
pub type TimeFormat = Box<dyn Fn(&LocalTime) -> String>;

pub struct Session {
    config: ClockConfig,
    places: &'static Catalogue,
    here: TimeZone,
    time: TimeFormat,
    now: Timestamp,
    browse: Browse,
    /// The pane beside the list, toggled from the action panel.
    details: bool,
    /// *My Places*, as kept, and the cities of it the catalogue knows.
    favourites: Favourites,
    mine: Vec<u32>,
}

impl Session {
    /// A session over `places`, comparing with `here`, the user's zone.
    pub fn new(
        config: ClockConfig,
        places: &'static Catalogue,
        favourites: Favourites,
        here: TimeZone,
        time: TimeFormat,
    ) -> Self {
        Self {
            config,
            places,
            here,
            time,
            now: Timestamp::UNIX_EPOCH,
            browse: Browse::default(),
            details: false,
            mine: favourites.resolve(places),
            favourites,
        }
    }

    /// What to do on opening, at `now`.
    pub fn start(&mut self, now: Timestamp) -> Vec<Effect> {
        self.now = now;
        vec![Effect::Render(self.view())]
    }

    /// The minute turned over.
    pub fn tick(&mut self, now: Timestamp) -> Vec<Effect> {
        self.now = now;
        vec![Effect::Render(self.view())]
    }

    /// Something happened in the view.
    pub fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::QueryChanged { query } => {
                if !self.browse.set_query(query) {
                    return Vec::new();
                }
                vec![Effect::Render(self.view())]
            }
            Event::Back => {
                if !self.browse.back() {
                    return Vec::new();
                }
                vec![Effect::Render(self.view())]
            }
            Event::ActionInvoked { item, action } => self.act(item.as_deref(), &action),
            Event::ScopeChanged { .. } | Event::SelectionChanged { .. } | Event::Dismissed => {
                Vec::new()
            }
        }
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        if action == DETAILS {
            self.details = !self.details;
            return vec![Effect::Render(self.view())];
        }
        let Some(item) = item else {
            return Vec::new();
        };
        match action {
            OPEN => {
                if !self.browse.go_into(self.places, item) {
                    return Vec::new();
                }
                vec![Effect::ClearQuery, Effect::Render(self.view())]
            }
            KEEP | FORGET => self.keep(item, action == KEEP),
            COPY_TIME | COPY_SENTENCE => {
                let Some((name, zone)) = self.place(item) else {
                    return Vec::new();
                };
                let there = self.now.to_zoned(zone);
                let time = (self.time)(&super::local_time(&there));
                if action == COPY_TIME {
                    return vec![Effect::Copy(time)];
                }
                vec![Effect::Copy(format!(
                    "{} {time} in {name} ({})",
                    there.strftime("%a"),
                    super::offset(there.offset().seconds())
                ))]
            }
            _ => Vec::new(),
        }
    }

    /// Add a city to *My Places*, or take it off.
    fn keep(&mut self, item: &str, keep: bool) -> Vec<Effect> {
        let Some(index) = browse::city_index(item) else {
            return Vec::new();
        };
        let Some(city) = self.places.city(index) else {
            return Vec::new();
        };
        let key = self.places.key(city);
        let changed = if keep {
            self.favourites.add(key)
        } else {
            self.favourites.remove(&key)
        };
        if !changed {
            return Vec::new();
        }
        self.mine = self.favourites.resolve(self.places);
        vec![
            Effect::Render(self.view()),
            Effect::Save(self.favourites.clone()),
        ]
    }

    /// A row that keeps one time: its name and zone.
    fn place(&self, item: &str) -> Option<(String, TimeZone)> {
        if item == HERE {
            return Some((self.here_name(), self.here.clone()));
        }
        if let Some(index) = browse::city_index(item) {
            let city = self.places.city(index)?;
            return Some((
                city.name.to_string(),
                super::zone(self.places.zone_of(city)),
            ));
        }
        // A country that keeps one time, however many names its zones have.
        let code = item.strip_prefix(COUNTRY)?;
        let country = self.places.country(self.places.country_by_code(code)?)?;
        let zones: Vec<&str> = self.places.zones_of(country).collect();
        let mut clocks = Clocks::new(self.now, &self.here);
        (clocks.offsets(&zones).len() == 1)
            .then(|| (country.name.to_string(), super::zone(zones[0])))
    }

    /// The user's zone, as the list calls it: its largest city, or its IANA
    /// name when the catalogue has none there.
    fn here_name(&self) -> String {
        let Some(name) = self.here.iana_name() else {
            return "Local time".into();
        };
        self.places
            .largest_in_zone(name)
            .and_then(|index| self.places.city(index))
            .map(|city| city.name.to_string())
            .unwrap_or_else(|| name.replace('_', " "))
    }

    /// The country code of the user's zone's largest city, for the icon.
    fn here_country(&self) -> Option<String> {
        let index = self.places.largest_in_zone(self.here.iana_name()?)?;
        let city = self.places.city(index)?;
        Some(self.places.country_of(city).code.to_string())
    }

    /// What the view should show now.
    pub fn view(&self) -> View {
        let listing = self.browse.listing(self.places, &self.mine);
        let mut clocks = Clocks::new(self.now, &self.here);
        let here = listing.top.then(|| Section {
            title: Some("Your time".into()),
            subtitle: None,
            items: vec![self.here_row(&mut clocks)],
        });
        let sections = self.browse.sections(
            &listing,
            here,
            &mut ClockRows {
                session: self,
                clocks: &mut clocks,
            },
        );

        View::List(List {
            placeholder: Some(self.browse.placeholder(self.places)),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            path: self.browse.path_names(self.places),
            empty: Some(self.browse.empty()),
            showing_detail: self.details,
            sections,
        })
    }

    /// The row a place that keeps one time ends with, in the chosen layout:
    /// the day when it is not today's, then the time and the difference,
    /// the one that leads last and drawn like the title. The leading value
    /// is always the last, so it lines up down the list whatever is
    /// beside it (I-10).
    ///
    /// With the working-hours overlap on, it takes the day's place, and the
    /// day joins the time -- `01:14 Thu` -- so a row still holds three
    /// (`MAX_ACCESSORIES`) and none is dropped.
    fn timed_accessories(&self, there: &Zoned, clocks: &Clocks) -> Vec<Accessory> {
        let mut time = (self.time)(&super::local_time(there));
        let seconds = super::difference(&clocks.here, there);
        let difference = super::describe(seconds, self.config.difference_format);
        let day = super::day(&clocks.here, there);
        let mut accessories = Vec::new();
        match self.config.working_hours() {
            Some((start, end)) => {
                if day.is_some() {
                    time = format!("{time} {}", there.strftime("%a"));
                }
                accessories.push(match super::overlap(start, end, seconds) {
                    Some((from, to)) => Accessory::Tag {
                        text: self.span(&clocks.here, from, to),
                        tone: Tone::Positive,
                    },
                    None => Accessory::Tag {
                        text: "No overlap".into(),
                        tone: Tone::Neutral,
                    },
                });
            }
            None => {
                if let Some(day) = day {
                    accessories.push(Accessory::Tag {
                        text: day.into(),
                        tone: Tone::Neutral,
                    });
                }
            }
        }
        let (dim, lead) = match self.config.row {
            RowLayout::Difference => (time, difference),
            RowLayout::Time => (difference, time),
        };
        accessories.push(Accessory::Text {
            text: dim,
            strong: false,
        });
        accessories.push(Accessory::Text {
            text: lead,
            strong: true,
        });
        accessories
    }

    /// `12:00\u{2013}17:00`: a span of the user's day, written the
    /// system's way.
    fn span(&self, here: &Zoned, from: u16, to: u16) -> String {
        format!(
            "{}\u{2013}{}",
            (self.time)(&super::at_minute(here, from)),
            (self.time)(&super::at_minute(here, to))
        )
    }

    /// A place that keeps several times ends with their range instead:
    /// `6 zones  UTC\u{2212}10 \u{2026} \u{2212}4`. Zones are counted by
    /// the time they keep now, not by their IANA names: Argentina has a
    /// dozen names and one time.
    fn range_accessories(&self, offsets: &[i32]) -> Vec<Accessory> {
        let (lowest, highest) = (
            offsets.first().copied().unwrap_or(0),
            offsets.last().copied().unwrap_or(0),
        );
        vec![
            Accessory::Text {
                text: format!("{} zones", offsets.len()),
                strong: false,
            },
            Accessory::Text {
                text: super::offset_range(lowest, highest),
                strong: true,
            },
        ]
    }

    fn here_row(&self, clocks: &mut Clocks) -> Item {
        let here = clocks.here.clone();
        let zone = self.here.iana_name().unwrap_or("Local time").to_string();
        let time = (self.time)(&super::local_time(&here));
        Item {
            id: HERE.into(),
            title: self.here_name(),
            subtitle: Some(format!("Your time zone \u{00b7} {zone}")),
            icon: self.here_country().map(|text| Icon::Text { text }),
            accessories: vec![
                Accessory::Text {
                    text: super::offset(here.offset().seconds()),
                    strong: false,
                },
                Accessory::Text {
                    text: time,
                    strong: true,
                },
            ],
            keywords: Vec::new(),
            actions: self.timed_actions(),
            detail: self
                .details
                .then(|| self.detail(&here, &self.here, None, clocks)),
        }
    }

    fn continent_row(&self, continent: Continent, clocks: &mut Clocks) -> Item {
        let countries = browse::countries_of(self.places, continent);
        let zones: Vec<&str> = countries
            .iter()
            .flat_map(|country| self.places.zones_of(country))
            .collect();
        let offsets = clocks.offsets(&zones);
        Item {
            id: format!("{CONTINENT}{}", continent.code()),
            title: continent.name().into(),
            subtitle: Some(format!("{} countries", countries.len())),
            icon: Some(Icon::Text {
                text: continent.code().into(),
            }),
            accessories: self.range_accessories(&offsets),
            keywords: Vec::new(),
            actions: vec![action(OPEN, &format!("Show {}", continent.name()))],
            detail: None,
        }
    }

    fn country_row(&self, index: u16, clocks: &mut Clocks) -> Option<Item> {
        let country = self.places.country(index)?;
        let zones: Vec<&str> = self.places.zones_of(country).collect();
        let offsets = clocks.offsets(&zones);
        let (accessories, single) = if offsets.len() == 1 {
            let there = clocks.at(zones[0]);
            (self.timed_accessories(&there, clocks), Some(there))
        } else {
            (self.range_accessories(&offsets), None)
        };
        let cities = if country.cities == 1 {
            "1 city".to_string()
        } else {
            format!("{} cities", grouped(country.cities as usize))
        };
        let mut actions = vec![action(OPEN, &format!("Show Cities in {}", country.name))];
        if single.is_some() {
            actions.extend(self.timed_actions());
        }
        Some(Item {
            id: format!("{COUNTRY}{}", country.code),
            title: country.name.to_string(),
            subtitle: Some(format!("{} \u{00b7} {cities}", country.continent.name())),
            icon: Some(Icon::Text {
                text: country.code.to_string(),
            }),
            accessories,
            keywords: Vec::new(),
            actions,
            detail: match (&single, self.details) {
                (Some(there), true) => {
                    let zone = super::zone(zones[0]);
                    Some(self.detail(there, &zone, None, clocks))
                }
                _ => None,
            },
        })
    }

    fn city_row(&self, index: u32, clocks: &mut Clocks) -> Option<Item> {
        let city = self.places.city(index)?;
        let zone_name = self.places.zone_of(city);
        let there = clocks.at(zone_name);
        Some(Item {
            id: browse::city_id(index),
            title: city.name.to_string(),
            subtitle: Some(self.places.whereabouts(city)),
            icon: Some(Icon::Text {
                text: self.places.country_of(city).code.to_string(),
            }),
            accessories: self.timed_accessories(&there, clocks),
            keywords: Vec::new(),
            actions: {
                let mut actions = self.timed_actions();
                actions.push(if self.mine.contains(&index) {
                    action(FORGET, "Remove from My Places")
                } else {
                    action(KEEP, "Add to My Places")
                });
                actions
            },
            detail: self.details.then(|| {
                let zone = super::zone(zone_name);
                self.detail(&there, &zone, Some(city), clocks)
            }),
        })
    }

    /// What a row that keeps one time can do: copy it, and show or hide the
    /// pane. A country puts *Show Cities* before them, so `Enter` goes in.
    fn timed_actions(&self) -> Vec<Action> {
        vec![
            action(COPY_TIME, "Copy Time"),
            action(COPY_SENTENCE, "Copy Time and Place"),
            action(
                DETAILS,
                if self.details {
                    "Hide Details"
                } else {
                    "Show Details"
                },
            ),
        ]
    }

    /// The pane: the whole answer, whatever the row's layout leads with.
    fn detail(
        &self,
        there: &Zoned,
        zone: &TimeZone,
        city: Option<&City>,
        clocks: &Clocks,
    ) -> ItemDetail {
        let time = (self.time)(&super::local_time(there));
        let info = zone.to_offset_info(self.now);
        let mut metadata = vec![
            row(
                "Difference",
                super::describe(
                    super::difference(&clocks.here, there),
                    DifferenceFormat::Words,
                ),
            ),
            row(
                "UTC offset",
                format!(
                    "{} ({})",
                    super::offset(info.offset().seconds()),
                    info.abbreviation()
                ),
            ),
            row("Time zone", zone.iana_name().unwrap_or("Local").to_string()),
            row(
                "Next change",
                super::describe_change(super::next_change(zone, self.now), zone),
            ),
        ];
        if let Some((start, end)) = self.config.working_hours() {
            let seconds = super::difference(&clocks.here, there);
            metadata.push(row(
                "Working hours",
                match super::overlap(start, end, seconds) {
                    Some((from, to)) => format!(
                        "Both at work {} your time",
                        self.span(&clocks.here, from, to)
                    ),
                    None => "No overlap with yours".into(),
                },
            ));
        }
        if let Some(city) = city {
            let format_moment =
                |moment: Timestamp| (self.time)(&super::local_time(&moment.to_zoned(zone.clone())));
            let (sunrise, sunset) = match super::daylight(
                f64::from(city.latitude),
                f64::from(city.longitude),
                there.date(),
            ) {
                Daylight::Day { sunrise, sunset } => {
                    (format_moment(sunrise), format_moment(sunset))
                }
                Daylight::AlwaysUp => ("Up all day".into(), "\u{2014}".into()),
                Daylight::AlwaysDown => ("\u{2014}".into(), "Down all day".into()),
            };
            metadata.push(row("Sunrise", sunrise));
            metadata.push(row("Sunset", sunset));
            metadata.push(row("Population", grouped(city.population as usize)));
        }
        ItemDetail {
            markdown: format!("# {time}\n\n{}", there.strftime("%A, %-d %B %Y")),
            text: None,
            image: None,
            metadata,
        }
    }
}

/// The rows of one render, with the times it has worked out.
struct ClockRows<'a> {
    session: &'a Session,
    clocks: &'a mut Clocks,
}

impl Rows for ClockRows<'_> {
    fn city(&mut self, index: u32) -> Option<Item> {
        self.session.city_row(index, self.clocks)
    }

    fn country(&mut self, index: u16) -> Option<Item> {
        self.session.country_row(index, self.clocks)
    }

    fn continent(&mut self, continent: Continent) -> Item {
        self.session.continent_row(continent, self.clocks)
    }
}

/// The time in each zone a render shows, worked out once per zone rather
/// than once per row.
struct Clocks {
    now: Timestamp,
    here: Zoned,
    zones: HashMap<String, Zoned>,
}

impl Clocks {
    fn new(now: Timestamp, here: &TimeZone) -> Self {
        Self {
            now,
            here: now.to_zoned(here.clone()),
            zones: HashMap::new(),
        }
    }

    /// The distinct offsets `zones` keep now, lowest first.
    fn offsets(&mut self, zones: &[&str]) -> Vec<i32> {
        let mut offsets: Vec<i32> = zones
            .iter()
            .map(|zone| self.at(zone).offset().seconds())
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        offsets
    }

    fn at(&mut self, zone: &str) -> Zoned {
        if let Some(zoned) = self.zones.get(zone) {
            return zoned.clone();
        }
        let zoned = self.now.to_zoned(super::zone(zone));
        self.zones.insert(zone.to_string(), zoned.clone());
        zoned
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

fn row(label: &str, value: String) -> MetadataRow {
    MetadataRow {
        label: label.into(),
        value,
        tone: Tone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::places;

    /// 24-hour, as a test can predict: the platform's own is checked by
    /// running it.
    fn session_with(config: ClockConfig) -> Session {
        session_keeping(config, Favourites::default())
    }

    fn session_keeping(config: ClockConfig, favourites: Favourites) -> Session {
        let mut session = Session::new(
            config,
            places::catalogue(),
            favourites,
            super::super::zone("America/Santo_Domingo"),
            Box::new(|time| format!("{:02}:{:02}", time.hour, time.minute)),
        );
        // 12:14 in Santo Domingo, 09:14 in Vancouver.
        session.start("2026-09-30T16:14:00Z".parse().unwrap());
        session
    }

    fn session() -> Session {
        session_with(ClockConfig::default())
    }

    fn typed(session: &mut Session, text: &str) -> List {
        list(&session.event(Event::QueryChanged { query: text.into() }))
    }

    fn list(effects: &[Effect]) -> List {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Render(View::List(list)) => Some(list.clone()),
                _ => None,
            })
            .expect("a render")
    }

    fn titles(section: &Section) -> Vec<&str> {
        section
            .items
            .iter()
            .map(|item| item.title.as_str())
            .collect()
    }

    fn section<'a>(list: &'a List, title: &str) -> &'a Section {
        list.sections
            .iter()
            .find(|section| section.title.as_deref() == Some(title))
            .unwrap_or_else(|| panic!("no {title} section"))
    }

    fn texts(item: &Item) -> Vec<(String, bool)> {
        item.accessories
            .iter()
            .map(|accessory| match accessory {
                Accessory::Text { text, strong } => (text.clone(), *strong),
                Accessory::Tag { text, .. } => (text.clone(), false),
            })
            .collect()
    }

    #[test]
    fn opening_shows_here_then_the_continents() {
        let list = list(&session().start("2026-09-30T16:14:00Z".parse().unwrap()));
        let here = &section(&list, "Your time").items[0];
        assert_eq!(here.title, "Santo Domingo");
        assert_eq!(
            texts(here),
            [
                ("UTC\u{2212}4".to_string(), false),
                ("12:14".to_string(), true)
            ]
        );
        let continents = titles(section(&list, "Continents"));
        assert!(continents.contains(&"Europe"));
        assert!(list.path.is_empty());
    }

    #[test]
    fn a_city_row_leads_with_the_difference_by_default() {
        let mut session = session();
        let list = typed(&mut session, "vancouver");
        let city = &section(&list, "Cities").items[0];
        assert_eq!(city.title, "Vancouver");
        assert_eq!(city.subtitle.as_deref(), Some("British Columbia, Canada"));
        assert_eq!(city.icon, Some(Icon::Text { text: "CA".into() }));
        assert_eq!(
            texts(city),
            [
                ("09:14".to_string(), false),
                ("3 h behind".to_string(), true)
            ]
        );
    }

    #[test]
    fn time_first_and_signed_are_settings() {
        let mut session = session_with(ClockConfig {
            row: RowLayout::Time,
            difference_format: DifferenceFormat::Signed,
            ..ClockConfig::default()
        });
        let list = typed(&mut session, "vancouver");
        assert_eq!(
            texts(&section(&list, "Cities").items[0]),
            [
                ("\u{2212}3 h".to_string(), false),
                ("09:14".to_string(), true)
            ]
        );
    }

    #[test]
    fn another_day_is_a_tag_before_the_rest() {
        let mut session = session();
        // 16:14 UTC is 01:14 tomorrow in Seoul.
        let list = typed(&mut session, "seoul");
        let seoul = &section(&list, "Cities").items[0];
        assert_eq!(
            texts(seoul),
            [
                ("Tomorrow".to_string(), false),
                ("01:14".to_string(), false),
                ("13 h ahead".to_string(), true)
            ]
        );
    }

    #[test]
    fn a_country_with_one_zone_shows_its_time_and_several_their_range() {
        let mut session = session();
        let list = typed(&mut session, "france");
        let france = &section(&list, "Countries").items[0];
        assert_eq!(france.title, "France");
        assert_eq!(texts(france).last().unwrap().0, "6 h ahead");
        // Its cities follow, the largest first.
        assert_eq!(section(&list, "Cities").items[0].title, "Paris");
        // Spain keeps two times: the Canary Islands are an hour behind.
        let list = typed(&mut session, "spain");
        assert_eq!(
            texts(&section(&list, "Countries").items[0]),
            [
                ("2 zones".to_string(), false),
                ("UTC+1 \u{2026} +2".to_string(), true)
            ]
        );
        // Argentina has a dozen zone names and one time, so it shows it.
        let list = typed(&mut session, "argentina");
        assert_eq!(
            texts(&section(&list, "Countries").items[0])
                .last()
                .unwrap()
                .0,
            "1 h ahead"
        );

        let list = typed(&mut session, "canada");
        let canada = &section(&list, "Countries").items[0];
        let accessories = texts(canada);
        assert!(accessories[0].0.ends_with("zones"));
        assert_eq!(accessories[1].0, "UTC\u{2212}7 \u{2026} \u{2212}2:30");
    }

    #[test]
    fn going_into_a_country_clears_the_query_and_fills_the_pill() {
        let mut session = session();
        typed(&mut session, "spain");
        let effects = session.event(Event::ActionInvoked {
            item: Some("country:ES".into()),
            action: "open".into(),
        });
        assert_eq!(effects[0], Effect::ClearQuery);
        let list = list(&effects);
        assert_eq!(list.path, ["Europe", "Spain"]);
        assert_eq!(list.placeholder.as_deref(), Some("Search in Spain"));
        assert_eq!(section(&list, "Cities").items[0].title, "Madrid");
        assert!(
            list.sections
                .iter()
                .all(|s| s.title.as_deref() == Some("Cities"))
        );

        // The clearing reaches the session as an edit, and changes nothing.
        assert!(
            session
                .event(Event::QueryChanged {
                    query: String::new()
                })
                .is_empty()
        );

        // Back goes up a level, then out to the top.
        let list = list_of(session.event(Event::Back));
        assert_eq!(list.path, ["Europe"]);
        assert!(titles(section(&list, "Countries")).contains(&"France"));
        let list = list_of(session.event(Event::Back));
        assert!(list.path.is_empty());
        assert!(
            session.event(Event::Back).is_empty(),
            "nothing left to take off"
        );
    }

    fn list_of(effects: Vec<Effect>) -> List {
        list(&effects)
    }

    #[test]
    fn going_into_a_continent_lists_its_countries() {
        let mut session = session();
        let list = list_of(session.event(Event::ActionInvoked {
            item: Some("continent:EU".into()),
            action: "open".into(),
        }));
        assert_eq!(list.path, ["Europe"]);
        let countries = titles(section(&list, "Countries"));
        assert!(countries.contains(&"Spain") && !countries.contains(&"Canada"));

        // Within it, a country keeps the continent in the pill.
        let list = list_of(session.event(Event::ActionInvoked {
            item: Some("country:FR".into()),
            action: "open".into(),
        }));
        assert_eq!(list.path, ["Europe", "France"]);
    }

    #[test]
    fn copying_writes_the_time_or_the_sentence() {
        let mut session = session();
        typed(&mut session, "vancouver");
        let item = Some("city:".to_string() + &vancouver_index().to_string());
        assert_eq!(
            session.event(Event::ActionInvoked {
                item: item.clone(),
                action: "copy-time".into(),
            }),
            [Effect::Copy("09:14".into())]
        );
        assert_eq!(
            session.event(Event::ActionInvoked {
                item,
                action: "copy-sentence".into(),
            }),
            [Effect::Copy("Wed 09:14 in Vancouver (UTC\u{2212}7)".into())]
        );
    }

    fn vancouver_index() -> u32 {
        places::catalogue()
            .cities()
            .iter()
            .position(|city| &*city.name == "Vancouver")
            .unwrap() as u32
    }

    #[test]
    fn the_pane_is_the_whole_answer() {
        let mut session = session();
        typed(&mut session, "vancouver");
        let list = list_of(session.event(Event::ActionInvoked {
            item: None,
            action: "details".into(),
        }));
        assert!(list.showing_detail);
        let detail = section(&list, "Cities").items[0].detail.clone().unwrap();
        assert!(detail.markdown.starts_with("# 09:14"));
        let value = |label: &str| {
            detail
                .metadata
                .iter()
                .find(|row| row.label == label)
                .unwrap_or_else(|| panic!("no {label}"))
                .value
                .clone()
        };
        assert_eq!(value("Difference"), "3 h behind");
        assert_eq!(value("UTC offset"), "UTC\u{2212}7 (PDT)");
        assert_eq!(value("Time zone"), "America/Vancouver");
        // British Columbia keeps UTC\u{2212}7 from November 2026 (IANA
        // 2026c): the name changes to MST, the clocks do not.
        assert_eq!(value("Next change"), "No change in the next year");
        assert_eq!(value("Population"), "662,248");
        assert!(value("Sunrise").starts_with("07:"));
    }

    #[test]
    fn many_cities_are_capped_and_the_section_says_so() {
        let mut session = session();
        let list = typed(&mut session, "united states");
        let cities = section(&list, "Cities");
        assert_eq!(cities.items.len(), CITY_ROWS);
        assert!(cities.subtitle.as_deref().unwrap().starts_with("200 of "));
    }

    #[test]
    fn nothing_found_says_what_to_try() {
        let mut session = session();
        let list = typed(&mut session, "zzzzqx");
        assert!(list.is_empty());
        assert!(list.empty.unwrap().title.contains("zzzzqx"));
    }

    #[test]
    fn the_minute_redraws_the_times() {
        let mut session = session();
        typed(&mut session, "vancouver");
        let list = list_of(session.tick("2026-09-30T16:15:00Z".parse().unwrap()));
        assert_eq!(texts(&section(&list, "Cities").items[0])[0].0, "09:15");
    }

    fn kept(keys: &[&str]) -> Favourites {
        Favourites {
            cities: keys.iter().map(|key| key.to_string()).collect(),
        }
    }

    #[test]
    fn my_places_come_after_here_and_before_the_continents() {
        let mut session = session_keeping(
            ClockConfig::default(),
            kept(&["CA/British Columbia/Vancouver", "KR/Seoul/Seoul"]),
        );
        let list = list(&session.start("2026-09-30T16:14:00Z".parse().unwrap()));
        let order: Vec<&str> = list
            .sections
            .iter()
            .filter_map(|section| section.title.as_deref())
            .collect();
        assert_eq!(order, ["Your time", "My Places", "Continents"]);
        assert_eq!(titles(section(&list, "My Places")), ["Vancouver", "Seoul"]);
    }

    #[test]
    fn my_places_are_filtered_with_the_list_and_shown_once() {
        let mut session = session_keeping(
            ClockConfig::default(),
            kept(&["CA/British Columbia/Vancouver", "KR/Seoul/Seoul"]),
        );
        let list = typed(&mut session, "canada");
        assert_eq!(titles(section(&list, "My Places")), ["Vancouver"]);
        assert!(
            !titles(section(&list, "Cities")).contains(&"Vancouver"),
            "not twice"
        );
        // Within a level too.
        let list = list_of(session.event(Event::ActionInvoked {
            item: Some("continent:AS".into()),
            action: "open".into(),
        }));
        assert_eq!(titles(section(&list, "My Places")), ["Seoul"]);
    }

    #[test]
    fn adding_and_removing_a_place_is_kept() {
        let mut session = session();
        typed(&mut session, "vancouver");
        let item = Some(format!("city:{}", vancouver_index()));
        let list_item = |list: &List| {
            list.items()
                .find(|row| row.id == item.clone().unwrap())
                .unwrap()
                .clone()
        };
        let actions = |row: &Item| -> Vec<String> {
            row.actions.iter().map(|action| action.id.clone()).collect()
        };
        let before = typed(&mut session, "vancouver ");
        assert!(actions(&list_item(&before)).contains(&"keep".to_string()));

        let effects = session.event(Event::ActionInvoked {
            item: item.clone(),
            action: "keep".into(),
        });
        assert!(effects.contains(&Effect::Save(kept(&["CA/British Columbia/Vancouver"]))));
        let list = list(&effects);
        assert_eq!(titles(section(&list, "My Places")), ["Vancouver"]);
        assert!(actions(&list_item(&list)).contains(&"forget".to_string()));

        // Twice is once.
        assert!(
            session
                .event(Event::ActionInvoked {
                    item: item.clone(),
                    action: "keep".into(),
                })
                .is_empty()
        );

        let effects = session.event(Event::ActionInvoked {
            item,
            action: "forget".into(),
        });
        assert!(effects.contains(&Effect::Save(Favourites::default())));
        assert!(
            list_of(effects)
                .sections
                .iter()
                .all(|section| section.title.as_deref() != Some("My Places"))
        );
    }

    fn overlapping(row: RowLayout) -> Session {
        let mut config = ClockConfig {
            row,
            ..ClockConfig::default()
        };
        config.overlap.enabled = true;
        session_with(config)
    }

    #[test]
    fn the_overlap_takes_the_days_place_and_the_day_joins_the_time() {
        let mut session = overlapping(RowLayout::Difference);
        let list = typed(&mut session, "vancouver");
        assert_eq!(
            texts(&section(&list, "Cities").items[0]),
            [
                ("12:00\u{2013}17:00".to_string(), false),
                ("09:14".to_string(), false),
                ("3 h behind".to_string(), true)
            ]
        );
        let list = typed(&mut session, "seoul");
        assert_eq!(
            texts(&section(&list, "Cities").items[0]),
            [
                ("No overlap".to_string(), false),
                ("01:14 Thu".to_string(), false),
                ("13 h ahead".to_string(), true)
            ]
        );
        // The pane says it too.
        let list = list_of(session.event(Event::ActionInvoked {
            item: None,
            action: "details".into(),
        }));
        let detail = section(&list, "Cities").items[0].detail.clone().unwrap();
        assert!(
            detail
                .metadata
                .iter()
                .any(|row| row.label == "Working hours" && row.value == "No overlap with yours")
        );
    }

    #[test]
    fn the_overlap_is_not_shown_with_time_first() {
        let mut session = overlapping(RowLayout::Time);
        let list = typed(&mut session, "seoul");
        assert_eq!(
            texts(&section(&list, "Cities").items[0])[0],
            ("Tomorrow".to_string(), false)
        );
    }
}
