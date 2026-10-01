//! The Weather view's state, as a plain value (F13).
//!
//! The daemon owns the requests, the timer and the files; this owns every
//! decision, World Clock's shape. Browsing -- the words, the levels, the
//! pill -- is `places::browse`, shared with World Clock; this says what a
//! row of weather shows and which forecasts to ask for.
//!
//! # Asked for when typing settles, a few at a time
//!
//! Opening asks at once for the home city and *My Places* that are not
//! fresh. Typing asks for nothing until it settles: each change is a new
//! generation, the driver waits and reports back, and only the latest
//! generation asks -- for the first [`FETCHED_ROWS`] cities listed, in one
//! request. A forecast that failed is not asked for again until *Refresh*.

use std::collections::{HashMap, HashSet};

use lumen_platform::LocalTime;

use super::{Cache, Failure, Forecast, WeatherConfig, describe};
use crate::places::browse::{self, Browse, CONTINENT, COUNTRY, Rows};
use crate::places::{Catalogue, Continent, Favourites};
use crate::view::{Accessory, Action, Event, Filtering, Icon, Item, List, Section, Tone, View};

/// What the driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Draw this.
    Render(View),
    /// Empty the search field: a level was gone into.
    ClearQuery,
    /// Wait for typing to settle, then call [`Session::settled`] with this.
    Settle(u64),
    /// Fetch these cities' forecasts in one request, and answer with
    /// [`Session::fetched`].
    Fetch(Vec<u32>),
    /// Put this on the clipboard and close the launcher.
    Copy(String),
    /// Write *My Places* to `places.json`.
    Save(Favourites),
}

/// How many of the cities a search lists are asked for when it settles:
/// what fits in the window, in one request.
pub const FETCHED_ROWS: usize = 8;

// Ids. Opaque to the host; only this file reads them back.
const OPEN: &str = "open";
const COPY: &str = "copy";
const REFRESH: &str = "refresh";
const KEEP: &str = "keep";
const FORGET: &str = "forget";

/// Writes a moment as the user's regional settings do.
pub type TimeFormat = Box<dyn Fn(&LocalTime) -> String>;

pub struct Session {
    config: WeatherConfig,
    places: &'static Catalogue,
    #[allow(dead_code)]
    time: TimeFormat,
    /// Seconds since the epoch.
    now: i64,
    browse: Browse,
    favourites: Favourites,
    mine: Vec<u32>,
    /// The city the view leads with.
    home: Option<u32>,
    cache: Cache,
    pending: HashSet<u32>,
    failed: HashMap<u32, Failure>,
    /// Bumped by every change that should ask once it settles.
    generation: u64,
}

impl Session {
    /// A session over `places`. `here` is the system's IANA zone, whose
    /// largest city is home when the settings name none.
    pub fn new(
        config: WeatherConfig,
        places: &'static Catalogue,
        favourites: Favourites,
        cache: Cache,
        here: Option<&str>,
        time: TimeFormat,
    ) -> Self {
        let home = config
            .home
            .as_deref()
            .and_then(|key| places.city_by_key(key))
            .or_else(|| here.and_then(|zone| places.largest_in_zone(zone)));
        Self {
            config,
            places,
            time,
            now: 0,
            browse: Browse::default(),
            mine: favourites.resolve(places),
            favourites,
            home,
            cache,
            pending: HashSet::new(),
            failed: HashMap::new(),
            generation: 0,
        }
    }

    /// What to do on opening, at `now`: draw, and ask at once for what the
    /// first screen shows.
    pub fn start(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let mut effects = vec![Effect::Render(self.view())];
        effects.extend(self.fetch());
        effects
    }

    /// Open on `index`'s weather, as `weather in ...` in the root list asks.
    /// The list for now; its report is slice 6.
    pub fn open(&mut self, index: u32, now: i64) -> Vec<Effect> {
        let _ = (index, now);
        Vec::new()
    }

    /// Typing settled for `generation`.
    pub fn settled(&mut self, generation: u64, now: i64) -> Vec<Effect> {
        self.now = now;
        if generation != self.generation {
            return Vec::new();
        }
        self.fetch().into_iter().collect()
    }

    /// A request came back, one answer per city asked for.
    pub fn fetched(&mut self, answers: Vec<(u32, Result<Forecast, Failure>)>) -> Vec<Effect> {
        for (index, answer) in answers {
            self.pending.remove(&index);
            match answer {
                Ok(forecast) => {
                    self.failed.remove(&index);
                    if let Some(city) = self.places.city(index) {
                        self.cache.insert(self.places.key(city), forecast);
                    }
                }
                Err(failure) => {
                    self.failed.insert(index, failure);
                }
            }
        }
        vec![Effect::Render(self.view())]
    }

    /// Something happened in the view.
    pub fn event(&mut self, event: Event, now: i64) -> Vec<Effect> {
        self.now = now;
        match event {
            Event::QueryChanged { query } => {
                if !self.browse.set_query(query) {
                    return Vec::new();
                }
                self.changed()
            }
            Event::Back => {
                if !self.browse.back() {
                    return Vec::new();
                }
                self.changed()
            }
            Event::ActionInvoked { item, action } => self.act(item.as_deref(), &action),
            Event::ScopeChanged { .. } | Event::SelectionChanged { .. } | Event::Dismissed => {
                Vec::new()
            }
        }
    }

    /// The list changed: draw it, and ask once it settles.
    fn changed(&mut self) -> Vec<Effect> {
        self.generation += 1;
        vec![Effect::Render(self.view()), Effect::Settle(self.generation)]
    }

    fn act(&mut self, item: Option<&str>, action: &str) -> Vec<Effect> {
        let Some(item) = item else {
            return Vec::new();
        };
        if action == OPEN {
            if !self.browse.go_into(self.places, item) {
                return Vec::new();
            }
            let mut effects = vec![Effect::ClearQuery];
            effects.extend(self.changed());
            return effects;
        }
        let Some(index) = browse::city_index(item) else {
            return Vec::new();
        };
        match action {
            KEEP | FORGET => self.keep(index, action == KEEP),
            REFRESH => {
                if self.pending.contains(&index) {
                    return Vec::new();
                }
                self.failed.remove(&index);
                self.pending.insert(index);
                vec![Effect::Render(self.view()), Effect::Fetch(vec![index])]
            }
            COPY => self
                .sentence(index)
                .map(|text| vec![Effect::Copy(text)])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn keep(&mut self, index: u32, keep: bool) -> Vec<Effect> {
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

    /// Ask for what is shown and not fresh, not out and not failed: the
    /// home city and *My Places* at the top, the first cities listed
    /// otherwise.
    fn fetch(&mut self) -> Option<Effect> {
        let mut wanted: Vec<u32> = Vec::new();
        if self.browse.at_top() {
            wanted.extend(self.home);
            wanted.extend(self.mine.iter().copied());
        } else {
            let listing = self.browse.listing(self.places, &self.mine);
            wanted.extend(listing.mine.iter().copied());
            wanted.extend(listing.found.cities.iter().copied());
        }
        let mut asked = Vec::new();
        for index in wanted {
            if asked.len() == FETCHED_ROWS {
                break;
            }
            if asked.contains(&index)
                || self.pending.contains(&index)
                || self.failed.contains_key(&index)
                || self
                    .forecast(index)
                    .is_some_and(|forecast| forecast.is_fresh(self.now))
            {
                continue;
            }
            asked.push(index);
        }
        if asked.is_empty() {
            return None;
        }
        self.pending.extend(asked.iter().copied());
        Some(Effect::Fetch(asked))
    }

    fn forecast(&self, index: u32) -> Option<&Forecast> {
        let city = self.places.city(index)?;
        self.cache.get(&self.places.key(city))
    }

    /// What *Copy Weather* puts on the clipboard.
    fn sentence(&self, index: u32) -> Option<String> {
        let city = self.places.city(index)?;
        let forecast = self.forecast(index)?;
        let units = self.config.units;
        let mut text = format!(
            "{}: {}, {}, feels like {}",
            city.name,
            units.temperature_with_unit(forecast.current.temperature),
            describe(forecast.current.code),
            units.temperature_with_unit(forecast.current.feels_like),
        );
        if let Some(today) = forecast.today() {
            text.push_str(&format!(
                ". Today {} to {}",
                units.degrees(today.low),
                units.degrees(today.high)
            ));
        }
        text.push('.');
        Some(text)
    }

    /// What the view should show now.
    pub fn view(&self) -> View {
        let top = self.browse.at_top();
        // At the top, home leads in its own section and is not repeated.
        let mine: Vec<u32> = self
            .mine
            .iter()
            .copied()
            .filter(|index| !top || Some(*index) != self.home)
            .collect();
        let listing = self.browse.listing(self.places, &mine);
        let home = self.home.and_then(|home| {
            Some(Section {
                title: Some("Your weather".into()),
                subtitle: Some("Forecasts by Open-Meteo".into()),
                items: vec![self.city_row(home)?],
            })
        });
        let sections = self
            .browse
            .sections(&listing, home, &mut WeatherRows { session: self });

        View::List(List {
            placeholder: Some(self.browse.placeholder(self.places)),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            path: self.browse.path_names(self.places),
            empty: Some(self.browse.empty()),
            showing_detail: false,
            sections,
        })
    }

    fn city_row(&self, index: u32) -> Option<Item> {
        let city = self.places.city(index)?;
        let forecast = self.forecast(index);
        let units = self.config.units;
        let accessories = match (forecast, self.failed.get(&index)) {
            (Some(forecast), _) => {
                let mut accessories = vec![Accessory::Text {
                    text: describe(forecast.current.code).into(),
                    strong: false,
                }];
                if let Some(today) = forecast.today() {
                    accessories.push(Accessory::Text {
                        text: format!(
                            "\u{2191}{} \u{2193}{}",
                            units.degrees(today.high),
                            units.degrees(today.low)
                        ),
                        strong: false,
                    });
                }
                accessories.push(Accessory::Text {
                    text: units.degrees(forecast.current.temperature),
                    strong: true,
                });
                accessories
            }
            (None, Some(failure)) => vec![Accessory::Tag {
                text: match failure {
                    Failure::Offline => "Offline".into(),
                    Failure::Other(_) => "Unavailable".into(),
                },
                tone: Tone::Caution,
            }],
            (None, None) if self.pending.contains(&index) => vec![Accessory::Text {
                text: "Loading\u{2026}".into(),
                strong: false,
            }],
            (None, None) => Vec::new(),
        };
        let mut actions = Vec::new();
        if forecast.is_some() {
            actions.push(action(COPY, "Copy Weather"));
        }
        actions.push(action(REFRESH, "Refresh"));
        actions.push(if self.mine.contains(&index) {
            action(FORGET, "Remove from My Places")
        } else {
            action(KEEP, "Add to My Places")
        });
        Some(Item {
            id: browse::city_id(index),
            title: city.name.to_string(),
            subtitle: Some(self.places.whereabouts(city)),
            icon: forecast.map(|forecast| Icon::Glyph {
                name: forecast.condition().glyph().into(),
            }),
            accessories,
            keywords: Vec::new(),
            actions,
            detail: None,
        })
    }

    fn country_row(&self, index: u16) -> Option<Item> {
        let country = self.places.country(index)?;
        let cities = if country.cities == 1 {
            "1 city".to_string()
        } else {
            format!("{} cities", browse::grouped(country.cities as usize))
        };
        Some(Item {
            id: format!("{COUNTRY}{}", country.code),
            title: country.name.to_string(),
            subtitle: Some(country.continent.name().into()),
            icon: Some(Icon::Text {
                text: country.code.to_string(),
            }),
            accessories: vec![Accessory::Text {
                text: cities,
                strong: false,
            }],
            keywords: Vec::new(),
            actions: vec![action(OPEN, &format!("Show Cities in {}", country.name))],
            detail: None,
        })
    }

    fn continent_row(&self, continent: Continent) -> Item {
        let countries = browse::countries_of(self.places, continent);
        Item {
            id: format!("{CONTINENT}{}", continent.code()),
            title: continent.name().into(),
            subtitle: None,
            icon: Some(Icon::Text {
                text: continent.code().into(),
            }),
            accessories: vec![Accessory::Text {
                text: format!("{} countries", countries.len()),
                strong: false,
            }],
            keywords: Vec::new(),
            actions: vec![action(OPEN, &format!("Show {}", continent.name()))],
            detail: None,
        }
    }
}

struct WeatherRows<'a> {
    session: &'a Session,
}

impl Rows for WeatherRows<'_> {
    fn city(&mut self, index: u32) -> Option<Item> {
        self.session.city_row(index)
    }

    fn country(&mut self, index: u16) -> Option<Item> {
        self.session.country_row(index)
    }

    fn continent(&mut self, continent: Continent) -> Item {
        self.session.continent_row(continent)
    }
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::forecast;
    use super::*;
    use crate::places;

    const NOW: i64 = 1_790_825_000;

    fn index(name: &str, country: &str) -> u32 {
        let catalogue = places::catalogue();
        catalogue
            .cities()
            .iter()
            .position(|city| &*city.name == name && &*catalogue.country_of(city).code == country)
            .unwrap() as u32
    }

    fn key(name: &str, country: &str) -> String {
        let catalogue = places::catalogue();
        catalogue.key(catalogue.city(index(name, country)).unwrap())
    }

    fn session_with(cache: Cache, favourites: &[&str]) -> Session {
        Session::new(
            WeatherConfig::default(),
            places::catalogue(),
            Favourites {
                cities: favourites.iter().map(|key| key.to_string()).collect(),
            },
            cache,
            Some("America/Santo_Domingo"),
            Box::new(|time| format!("{:02}:{:02}", time.hour, time.minute)),
        )
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

    fn fetched(effects: &[Effect]) -> Vec<u32> {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Fetch(cities) => Some(cities.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn section<'a>(list: &'a List, title: &str) -> &'a Section {
        list.sections
            .iter()
            .find(|section| section.title.as_deref() == Some(title))
            .unwrap_or_else(|| panic!("no {title} section"))
    }

    fn texts(item: &Item) -> Vec<String> {
        item.accessories
            .iter()
            .map(|accessory| match accessory {
                Accessory::Text { text, .. } | Accessory::Tag { text, .. } => text.clone(),
            })
            .collect()
    }

    #[test]
    fn opening_leads_with_home_and_asks_for_it_and_my_places() {
        let vancouver = "CA/British Columbia/Vancouver";
        let mut session = session_with(Cache::default(), &[vancouver]);
        let effects = session.start(NOW);
        let list = list(&effects);
        let home = &section(&list, "Your weather").items[0];
        assert_eq!(home.title, "Santo Domingo");
        assert_eq!(
            section(&list, "Your weather").subtitle.as_deref(),
            Some("Forecasts by Open-Meteo")
        );
        assert_eq!(section(&list, "My Places").items[0].title, "Vancouver");
        assert_eq!(
            fetched(&effects),
            [index("Santo Domingo", "DO"), index("Vancouver", "CA")]
        );
    }

    #[test]
    fn a_fresh_forecast_is_shown_and_not_asked_for_again() {
        let mut cache = Cache::default();
        cache.insert(key("Santo Domingo", "DO"), forecast(28.0, 61, NOW - 60));
        let mut session = session_with(cache, &[]);
        let effects = session.start(NOW);
        assert!(fetched(&effects).is_empty());
        let home = list(&effects).sections[0].items[0].clone();
        assert_eq!(
            texts(&home),
            [
                "Light rain",
                "\u{2191}30\u{b0} \u{2193}23\u{b0}",
                "28\u{b0}"
            ]
        );
        assert_eq!(
            home.icon,
            Some(Icon::Glyph {
                name: "weather-rain".into()
            })
        );
    }

    #[test]
    fn a_stale_forecast_is_shown_while_a_new_one_is_asked_for() {
        let mut cache = Cache::default();
        cache.insert(
            key("Santo Domingo", "DO"),
            forecast(28.0, 0, NOW - super::super::FRESH),
        );
        let mut session = session_with(cache, &[]);
        let effects = session.start(NOW);
        assert_eq!(fetched(&effects), [index("Santo Domingo", "DO")]);
        assert_eq!(texts(&list(&effects).sections[0].items[0])[2], "28\u{b0}");
    }

    #[test]
    fn typing_asks_only_once_it_settles_and_only_the_latest() {
        let mut session = session_with(Cache::default(), &[]);
        session.start(NOW);
        let first = session.event(
            Event::QueryChanged {
                query: "vancouv".into(),
            },
            NOW,
        );
        assert!(fetched(&first).is_empty(), "nothing per keystroke");
        let second = session.event(
            Event::QueryChanged {
                query: "vancouver".into(),
            },
            NOW,
        );
        let (Some(Effect::Settle(old)), Some(Effect::Settle(new))) = (first.last(), second.last())
        else {
            panic!("each change asks to settle");
        };
        assert!(session.settled(*old, NOW).is_empty(), "superseded");
        let effects = session.settled(*new, NOW);
        let asked = fetched(&effects);
        assert_eq!(asked[0], index("Vancouver", "CA"));
        assert!(asked.len() <= FETCHED_ROWS);
        // Loading, then the answer.
        let list = list(&session.view_effects());
        assert_eq!(
            texts(&section(&list, "Cities").items[0]),
            ["Loading\u{2026}"]
        );
        let list =
            list_of(session.fetched(vec![(index("Vancouver", "CA"), Ok(forecast(12.0, 3, NOW)))]));
        assert_eq!(texts(&section(&list, "Cities").items[0])[2], "12\u{b0}");
    }

    fn list_of(effects: Vec<Effect>) -> List {
        list(&effects)
    }

    impl Session {
        fn view_effects(&self) -> Vec<Effect> {
            vec![Effect::Render(self.view())]
        }
    }

    #[test]
    fn a_failure_is_said_and_not_retried_until_refresh() {
        let mut session = session_with(Cache::default(), &[]);
        session.start(NOW);
        let home = index("Santo Domingo", "DO");
        let list = list_of(session.fetched(vec![(home, Err(Failure::Offline))]));
        assert_eq!(texts(&list.sections[0].items[0]), ["Offline"]);
        assert!(session.fetch().is_none(), "not asked again by itself");

        let effects = session.event(
            Event::ActionInvoked {
                item: Some(browse::city_id(home)),
                action: "refresh".into(),
            },
            NOW,
        );
        assert_eq!(fetched(&effects), [home]);
    }

    #[test]
    fn copying_says_the_weather_in_a_sentence() {
        let mut cache = Cache::default();
        cache.insert(key("Vancouver", "CA"), forecast(12.0, 3, NOW));
        let mut session = session_with(cache, &[]);
        session.start(NOW);
        let effects = session.event(
            Event::ActionInvoked {
                item: Some(browse::city_id(index("Vancouver", "CA"))),
                action: "copy".into(),
            },
            NOW,
        );
        assert_eq!(
            effects,
            [Effect::Copy(
                "Vancouver: 12 \u{b0}C, Overcast, feels like 10 \u{b0}C. Today 7\u{b0} to 14\u{b0}."
                    .into()
            )]
        );
    }

    #[test]
    fn my_places_are_shared_and_home_is_not_repeated() {
        let mut session = session_with(Cache::default(), &[]);
        session.start(NOW);
        let effects = session.event(
            Event::ActionInvoked {
                item: Some(browse::city_id(index("Santo Domingo", "DO"))),
                action: "keep".into(),
            },
            NOW,
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Save(_)))
        );
        let list = list(&effects);
        assert!(
            list.sections
                .iter()
                .all(|section| section.title.as_deref() != Some("My Places")),
            "home is first already"
        );
    }

    #[test]
    fn the_pill_and_the_levels_are_the_clocks() {
        let mut session = session_with(Cache::default(), &[]);
        session.start(NOW);
        let effects = session.event(
            Event::ActionInvoked {
                item: Some("continent:EU".into()),
                action: "open".into(),
            },
            NOW,
        );
        assert_eq!(effects[0], Effect::ClearQuery);
        let list = list(&effects);
        assert_eq!(list.path, ["Europe"]);
        assert!(
            section(&list, "Countries")
                .items
                .iter()
                .any(|item| item.title == "Spain")
        );
    }
}
