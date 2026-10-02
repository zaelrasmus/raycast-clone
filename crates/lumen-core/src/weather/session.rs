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
//!
//! # The report, in two layouts (slice 6)
//!
//! *Report*: `Enter` on a city shows its report in the list's place -- now,
//! the details, the week -- with the city as one more level of the pill,
//! so `Escape` and `Backspace` come back to the list as they come back out
//! of a country. Typing in it searches for another place. *Pane*: the same
//! report, compact, beside the list, following the selection; `Enter`
//! copies. `weather in ...` from the root list opens the report whatever
//! the layout: one city is what was asked for.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use lumen_platform::LocalTime;

use super::{Cache, Condition, Day, Failure, Forecast, Layout, WeatherConfig, describe};
use crate::places::browse::{self, Browse, CONTINENT, COUNTRY, Rows};
use crate::places::{Catalogue, City, Continent, Favourites};
use crate::view::{
    Accessory, Action, EmptyState, Event, Filtering, Icon, Item, ItemDetail, List, MetadataRow,
    Section, Tone, View,
};

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
const SHOW: &str = "show";
const COPY: &str = "copy";
const REFRESH: &str = "refresh";
const KEEP: &str = "keep";
const FORGET: &str = "forget";

/// Writes a moment as the user's regional settings do.
pub type TimeFormat = Box<dyn Fn(&LocalTime) -> String>;

pub struct Session {
    config: WeatherConfig,
    places: &'static Catalogue,
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
    /// The city whose report is shown in the list's place.
    report: Option<u32>,
    /// The city selected, for the pane.
    selected: Option<u32>,
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
        let home = here.and_then(|zone| places.largest_in_zone(zone));
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
            report: None,
            selected: None,
        }
    }

    /// Lead with the home city keyed `home`, shared with World Clock
    /// (`places.home`), over the system zone's largest city.
    pub fn with_home(mut self, home: Option<&str>) -> Self {
        if let Some(index) = home.and_then(|key| self.places.city_by_key(key)) {
            self.home = Some(index);
        }
        self
    }

    /// What to do on opening, at `now`: draw, and ask at once for what the
    /// first screen shows.
    pub fn start(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let mut effects = vec![Effect::Render(self.view())];
        effects.extend(self.fetch());
        effects
    }

    /// Show `index`'s report, as `weather in ...` in the root list and
    /// `Enter` on a city ask.
    pub fn open(&mut self, index: u32, now: i64) -> Vec<Effect> {
        self.now = now;
        if self.places.city(index).is_none() {
            return Vec::new();
        }
        self.report = Some(index);
        // The field is emptied, so the list behind the report is too: coming
        // back must not show results for words no longer on screen.
        self.browse.set_query(String::new());
        let mut effects = vec![Effect::ClearQuery, Effect::Render(self.view())];
        effects.extend(self.fetch());
        effects
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
            // Typing in a report searches for another place; the clearing
            // that opened it changes nothing.
            Event::QueryChanged { query } if self.report.is_some() => {
                if query.trim().is_empty() {
                    return Vec::new();
                }
                self.report = None;
                self.browse.set_query(query);
                self.changed()
            }
            Event::QueryChanged { query } => {
                if !self.browse.set_query(query) {
                    return Vec::new();
                }
                self.changed()
            }
            Event::Back if self.report.take().is_some() => self.changed(),
            Event::Back => {
                if !self.browse.back() {
                    return Vec::new();
                }
                self.changed()
            }
            Event::SelectionChanged { item } if self.config.layout == Layout::Pane => {
                let selected = browse::city_index(&item);
                if selected == self.selected {
                    return Vec::new();
                }
                self.selected = selected;
                self.generation += 1;
                vec![Effect::Settle(self.generation)]
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
        // A report's rows are the city's, whichever row it is.
        let Some(index) = browse::city_index(report_city(item)) else {
            return Vec::new();
        };
        match action {
            SHOW => self.open(index, self.now),
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
        // What the pane shows first: it is what is being looked at.
        if self.config.layout == Layout::Pane {
            wanted.extend(self.selected);
        }
        if let Some(report) = self.report {
            wanted.push(report);
        } else if self.browse.at_top() {
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
        if let Some(report) = self.report
            && let Some(view) = self.report_view(report)
        {
            return view;
        }
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
            showing_detail: self.config.layout == Layout::Pane,
            sections,
        })
    }

    /// A city's report, in the list's place (*Report* layout, and `weather
    /// in ...` in either): the screenshots' shape -- now, the details, the
    /// week -- with the metrics as labelled values rather than glyphs.
    fn report_view(&self, index: u32) -> Option<View> {
        let city = self.places.city(index)?;
        let mut path = self.browse.path_names(self.places);
        path.push(city.name.to_string());
        let (sections, empty) = match (self.forecast(index), self.failed.get(&index)) {
            (Some(forecast), _) => (self.report_sections(index, city, forecast), None),
            (None, Some(failure)) => (
                vec![Section {
                    title: Some("Weather Report".into()),
                    subtitle: Some(self.places.whereabouts(city)),
                    items: vec![Item {
                        id: format!("{}#failed", browse::city_id(index)),
                        title: match failure {
                            Failure::Offline => "No internet connection".into(),
                            Failure::Other(_) => "Could not load the weather".into(),
                        },
                        subtitle: Some(match failure {
                            Failure::Offline => "Enter tries again".into(),
                            Failure::Other(message) => message.clone(),
                        }),
                        icon: None,
                        accessories: Vec::new(),
                        keywords: Vec::new(),
                        actions: vec![action(REFRESH, "Try Again")],
                        detail: None,
                    }],
                }],
                None,
            ),
            (None, None) => (
                Vec::new(),
                Some(EmptyState {
                    title: format!("Loading the weather in {}\u{2026}", city.name),
                    description: Some("From Open-Meteo".into()),
                    icon: None,
                }),
            ),
        };
        Some(View::List(List {
            placeholder: Some("Search for another place".into()),
            filtering: Filtering::Extension,
            scopes: Vec::new(),
            path,
            empty,
            showing_detail: false,
            sections,
        }))
    }

    fn report_sections(&self, index: u32, city: &City, forecast: &Forecast) -> Vec<Section> {
        let units = self.config.units;
        let current = &forecast.current;
        let id = |row: &str| format!("{}#{row}", browse::city_id(index));
        let actions = self.report_actions(index);
        let row = |name: &str, title: String, subtitle: Option<String>, values: Vec<String>| {
            let last = values.len().saturating_sub(1);
            Item {
                id: id(name),
                title,
                subtitle,
                icon: detail_glyph(name).map(|glyph| Icon::Glyph { name: glyph.into() }),
                accessories: values
                    .into_iter()
                    .enumerate()
                    .map(|(at, text)| Accessory::Text {
                        text,
                        strong: at == last,
                    })
                    .collect(),
                keywords: Vec::new(),
                actions: actions.clone(),
                detail: None,
            }
        };

        let mut updated = vec![self.places.whereabouts(city)];
        updated.push(format!(
            "Updated {}",
            super::age(self.now - forecast.fetched)
        ));
        if self.pending.contains(&index) {
            updated.push("updating\u{2026}".into());
        }
        updated.push("Open-Meteo".into());
        let now = Item {
            icon: Some(Icon::Glyph {
                name: forecast.condition().glyph().into(),
            }),
            ..row(
                "now",
                units.temperature_with_unit(current.temperature),
                Some(describe(current.code).into()),
                vec![
                    format!("Humidity {}%", current.humidity.round()),
                    format!("Rain {}", units.precipitation(current.precipitation)),
                    format!(
                        "Wind {}",
                        units.wind(current.wind_speed, current.wind_direction)
                    ),
                ],
            )
        };

        let mut details = vec![
            row(
                "feels",
                "Feels Like".into(),
                None,
                vec![units.temperature_with_unit(current.feels_like)],
            ),
            row(
                "uv",
                "UV Index".into(),
                None,
                vec![format!("{}", current.uv_index.round())],
            ),
            row(
                "clouds",
                "Cloud Cover".into(),
                None,
                vec![format!("{}%", current.cloud_cover.round())],
            ),
            row(
                "pressure",
                "Pressure".into(),
                None,
                vec![units.pressure(current.pressure)],
            ),
            row(
                "visibility",
                "Visibility".into(),
                None,
                vec![units.distance(current.visibility)],
            ),
        ];
        if let Some(today) = forecast.today() {
            details.push(row(
                "sun",
                "Sun".into(),
                None,
                vec![
                    format!("\u{2191} {}", self.time_at(city, today.sunrise)),
                    format!("\u{2193} {}", self.time_at(city, today.sunset)),
                ],
            ));
        }
        let (phase, lit) = super::moon(self.now);
        details.push(row(
            "moon",
            "Moon".into(),
            Some(phase.into()),
            vec![format!("{lit}% lit")],
        ));
        details.push(row(
            "coordinates",
            "Coordinates".into(),
            None,
            vec![format!("{:.3}, {:.3}", city.latitude, city.longitude)],
        ));

        let days = forecast
            .days
            .iter()
            .enumerate()
            .map(|(at, day)| Item {
                icon: Some(Icon::Glyph {
                    name: Condition::from_code(day.code, true).glyph().into(),
                }),
                ..row(
                    &format!("day{at}"),
                    self.day_name(city, day, at, "%A"),
                    Some(describe(day.code).into()),
                    vec![
                        chance_or_sun(day),
                        format!("\u{2193} {}", units.degrees(day.low)),
                        format!("\u{2191} {}", units.degrees(day.high)),
                    ],
                )
            })
            .collect();

        vec![
            Section {
                title: Some("Weather Report".into()),
                subtitle: Some(updated.join(" \u{00b7} ")),
                items: vec![now],
            },
            Section {
                title: Some("Details".into()),
                subtitle: None,
                items: details,
            },
            Section {
                title: Some("Daily Forecast".into()),
                subtitle: None,
                items: days,
            },
        ]
    }

    /// What every row of a report can do: it is all the one city.
    fn report_actions(&self, index: u32) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.forecast(index).is_some() {
            actions.push(action(COPY, "Copy Weather"));
        }
        actions.push(action(REFRESH, "Refresh"));
        actions.push(self.keep_action(index));
        actions
    }

    fn keep_action(&self, index: u32) -> Action {
        if self.mine.contains(&index) {
            action(FORGET, "Remove from My Places")
        } else {
            action(KEEP, "Add to My Places")
        }
    }

    /// The compact report beside the list (*Pane* layout): the same data,
    /// a forecast of one line a day.
    fn pane(&self, index: u32, city: &City) -> ItemDetail {
        let Some(forecast) = self.forecast(index) else {
            let markdown = match self.failed.get(&index) {
                Some(Failure::Offline) => "No internet connection. *Refresh* tries again.",
                Some(Failure::Other(_)) => "Could not load the weather. *Refresh* tries again.",
                None if self.pending.contains(&index) => "Loading\u{2026}",
                None => "",
            };
            return ItemDetail {
                markdown: markdown.into(),
                ..ItemDetail::default()
            };
        };
        let units = self.config.units;
        let current = &forecast.current;
        let label = |label: &str, value: String| MetadataRow {
            label: label.into(),
            value,
            tone: Tone::Neutral,
        };
        let mut metadata = vec![
            label("Humidity", format!("{}%", current.humidity.round())),
            label(
                "Wind",
                units.wind(current.wind_speed, current.wind_direction),
            ),
            label("Rain", units.precipitation(current.precipitation)),
            label("UV index", format!("{}", current.uv_index.round())),
        ];
        if let Some(today) = forecast.today() {
            metadata.push(label(
                "Sun",
                format!(
                    "\u{2191} {}  \u{2193} {}",
                    self.time_at(city, today.sunrise),
                    self.time_at(city, today.sunset)
                ),
            ));
        }
        for (at, day) in forecast.days.iter().enumerate() {
            metadata.push(label(
                &self.day_name(city, day, at, "%a"),
                format!(
                    "\u{2191}{} \u{2193}{}  {}",
                    units.degrees(day.high),
                    units.degrees(day.low),
                    describe(day.code)
                ),
            ));
        }
        ItemDetail {
            markdown: format!(
                "# {}\n\n{} \u{00b7} feels like {}\n\nUpdated {} \u{00b7} Open-Meteo",
                units.temperature_with_unit(current.temperature),
                describe(current.code),
                units.degrees(current.feels_like),
                super::age(self.now - forecast.fetched)
            ),
            text: None,
            image: None,
            metadata,
        }
    }

    /// A moment at the city, written the system's way.
    fn time_at(&self, city: &City, seconds: i64) -> String {
        let zone = crate::clock::zone(self.places.zone_of(city));
        let moment = Timestamp::from_second(seconds).unwrap_or(Timestamp::UNIX_EPOCH);
        (self.time)(&crate::clock::local_time(&moment.to_zoned(zone)))
    }

    /// *Today*, *Tomorrow*, then the weekday, as the city counts days.
    fn day_name(&self, city: &City, day: &Day, at: usize, weekday: &str) -> String {
        match at {
            0 => "Today".into(),
            1 if weekday == "%A" => "Tomorrow".into(),
            _ => {
                let zone = crate::clock::zone(self.places.zone_of(city));
                // Noon, so a day's name never slips across midnight.
                Timestamp::from_second(day.date + 12 * 3600)
                    .unwrap_or(Timestamp::UNIX_EPOCH)
                    .to_zoned(zone)
                    .strftime(weekday)
                    .to_string()
            }
        }
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
        // *Report*: `Enter` shows it. *Pane*: the pane shows it, `Enter`
        // copies.
        let mut actions = Vec::new();
        if self.config.layout == Layout::Report {
            actions.push(action(SHOW, "Show Weather"));
        }
        actions.extend(self.report_actions(index));
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
            detail: (self.config.layout == Layout::Pane).then(|| self.pane(index, city)),
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

/// The glyph a detail row draws, as the screenshots' report has one per
/// metric. The sun, the moon and the cloud are the condition glyphs; the
/// rest are their own. A row with none -- now, a day -- sets its own.
fn detail_glyph(row: &str) -> Option<&'static str> {
    Some(match row {
        "feels" => "metric-feels-like",
        "uv" => "metric-uv",
        "clouds" => "weather-cloudy",
        "pressure" => "metric-pressure",
        "visibility" => "metric-visibility",
        "sun" => "weather-clear-day",
        "moon" => "weather-clear-night",
        "coordinates" => "metric-coordinates",
        _ => return None,
    })
}

/// The city a report row belongs to: `city:12#feels` is city 12's.
fn report_city(item: &str) -> &str {
    item.split('#').next().unwrap_or(item)
}

/// A day's first column: the chance of rain, or the sunshine when the
/// forecast has no chance to give.
fn chance_or_sun(day: &Day) -> String {
    match day.precipitation_chance {
        Some(chance) => format!("Rain {}%", chance.round()),
        None => format!("Sun {:.1} h", day.sunshine / 3600.0),
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

    fn kept_vancouver() -> Cache {
        let mut cache = Cache::default();
        cache.insert(key("Vancouver", "CA"), forecast(12.0, 3, NOW - 60));
        cache
    }

    #[test]
    fn enter_shows_the_report_and_escape_comes_back() {
        let mut session = session_with(kept_vancouver(), &[]);
        session.start(NOW);
        session.event(
            Event::QueryChanged {
                query: "vancouver".into(),
            },
            NOW,
        );
        let vancouver = browse::city_id(index("Vancouver", "CA"));
        let item = list_of(session.view_effects()).sections[0].items[0].clone();
        assert_eq!(item.actions[0].id, "show", "Enter shows the report");

        let effects = session.event(
            Event::ActionInvoked {
                item: Some(vancouver.clone()),
                action: "show".into(),
            },
            NOW,
        );
        assert_eq!(effects[0], Effect::ClearQuery);
        assert!(fetched(&effects).is_empty(), "fresh");
        let report = list(&effects);
        assert_eq!(report.path, ["Vancouver"]);
        let titles: Vec<_> = report
            .sections
            .iter()
            .filter_map(|section| section.title.clone())
            .collect();
        assert_eq!(titles, ["Weather Report", "Details", "Daily Forecast"]);
        let now = &report.sections[0].items[0];
        assert_eq!(now.title, "12 \u{b0}C");
        assert_eq!(now.subtitle.as_deref(), Some("Overcast"));
        assert_eq!(texts(now)[2], "Wind 6 km/h W");
        let days = &report.sections[2].items;
        assert_eq!(days.len(), 7);
        assert_eq!(days[0].title, "Today");
        assert_eq!(days[1].title, "Tomorrow");
        assert_eq!(days[2].title, "Friday", "2 October 2026");
        assert_eq!(
            texts(&days[1]),
            ["Rain 80%", "\u{2193} 7\u{b0}", "\u{2191} 14\u{b0}"]
        );
        let sun = report.sections[1]
            .items
            .iter()
            .find(|row| row.title == "Sun")
            .unwrap();
        // 07:10 and 18:52 in Vancouver: the fixture's times, in its zone.
        assert_eq!(texts(sun), ["\u{2191} 07:10", "\u{2193} 18:52"]);
        // Every detail has its glyph, and each one is bundled.
        for row in &report.sections[1].items {
            let Some(Icon::Glyph { name }) = &row.icon else {
                panic!("{} has no glyph", row.title);
            };
            let path = format!(
                "{}/../../assets/icons/{name}.svg",
                env!("CARGO_MANIFEST_DIR")
            );
            assert!(std::path::Path::new(&path).exists(), "{name} is not drawn");
        }

        // Clearing the field as it opened changes nothing.
        assert!(
            session
                .event(
                    Event::QueryChanged {
                        query: String::new()
                    },
                    NOW
                )
                .is_empty()
        );
        // A report row acts for its city.
        assert!(matches!(
            session.event(
                Event::ActionInvoked {
                    item: Some(format!("{vancouver}#feels")),
                    action: "copy".into(),
                },
                NOW,
            )[..],
            [Effect::Copy(_)]
        ));
        // Escape takes the city off the pill.
        let list = list_of(session.event(Event::Back, NOW));
        assert!(list.path.is_empty());
        assert_eq!(list.sections[0].title.as_deref(), Some("Your weather"));
    }

    #[test]
    fn typing_in_a_report_searches_for_another_place() {
        let mut session = session_with(kept_vancouver(), &[]);
        session.start(NOW);
        session.open(index("Vancouver", "CA"), NOW);
        let list = list_of(session.event(
            Event::QueryChanged {
                query: "seoul".into(),
            },
            NOW,
        ));
        assert!(list.path.is_empty());
        assert_eq!(section(&list, "Cities").items[0].title, "Seoul");
    }

    #[test]
    fn a_report_waits_for_its_forecast_and_says_when_it_fails() {
        let mut session = session_with(Cache::default(), &[]);
        session.start(NOW);
        let seoul = index("Seoul", "KR");
        let effects = session.open(seoul, NOW);
        assert!(fetched(&effects).contains(&seoul));
        let report = list(&effects);
        assert!(report.is_empty());
        assert!(
            report
                .empty
                .unwrap()
                .title
                .starts_with("Loading the weather in Seoul")
        );

        let report = list_of(session.fetched(vec![(seoul, Err(Failure::Offline))]));
        let row = &report.sections[0].items[0];
        assert_eq!(row.title, "No internet connection");
        assert_eq!(row.actions[0].id, "refresh");
        let effects = session.event(
            Event::ActionInvoked {
                item: Some(row.id.clone()),
                action: "refresh".into(),
            },
            NOW,
        );
        assert_eq!(fetched(&effects), [seoul]);
    }

    #[test]
    fn the_pane_follows_the_selection_and_enter_copies() {
        let mut session = Session::new(
            WeatherConfig {
                layout: Layout::Pane,
                ..WeatherConfig::default()
            },
            places::catalogue(),
            Favourites::default(),
            kept_vancouver(),
            Some("America/Santo_Domingo"),
            Box::new(|time| format!("{:02}:{:02}", time.hour, time.minute)),
        );
        session.start(NOW);
        let list = list_of(session.event(
            Event::QueryChanged {
                query: "vancouver".into(),
            },
            NOW,
        ));
        assert!(list.showing_detail);
        let row = &section(&list, "Cities").items[0];
        assert_eq!(row.actions[0].id, "copy");
        let pane = row.detail.clone().unwrap();
        assert!(pane.markdown.starts_with("# 12 \u{b0}C\n\nOvercast"));
        assert!(
            pane.metadata
                .iter()
                .any(|row| row.label == "Thu" && row.value.contains("Light rain"))
        );

        // Selecting a city asks for it once the selection settles.
        let seoul = index("Seoul", "KR");
        let effects = session.event(
            Event::SelectionChanged {
                item: browse::city_id(seoul),
            },
            NOW,
        );
        let [Effect::Settle(generation)] = effects[..] else {
            panic!("settle first");
        };
        assert_eq!(fetched(&session.settled(generation, NOW))[0], seoul);
    }

    #[test]
    fn the_shared_home_leads() {
        let mut session = session_with(Cache::default(), &[]).with_home(Some(&key("Seoul", "KR")));
        let list = list(&session.start(NOW));
        assert_eq!(list.sections[0].items[0].title, "Seoul");
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
