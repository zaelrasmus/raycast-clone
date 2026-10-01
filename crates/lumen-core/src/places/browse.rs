//! Browsing the places, as World Clock and Weather both do (F13).
//!
//! What is typed, the drill-down pill, and the sections the list is made
//! of, in one place, so the two views cannot drift apart: the same words
//! find the same places at the same level in both. Each view says only what
//! its rows show, through [`Rows`].
//!
//! # Levels, and the pill
//!
//! The list is at the level the words name (`places::find`). `Enter` on a
//! continent or a country goes into it: the place joins the path, which the
//! host draws as a pill before the search field, and the text is cleared
//! for a search within it. `Event::Back` takes the innermost level off.

use super::{Catalogue, Continent, Country, Found, Within, words};
use crate::view::{EmptyState, Item, Section};

/// The most city rows a list shows. Past this nobody is reading; the
/// section says how many more matched, and typing narrows.
pub const CITY_ROWS: usize = 200;

// Row ids: opaque to the host, read back by the views.
pub const CITY: &str = "city:";
pub const COUNTRY: &str = "country:";
pub const CONTINENT: &str = "continent:";

/// How far into the places the list has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Continent(Continent),
    Country(u16),
}

/// What is typed, and how far in the pill has gone.
#[derive(Debug, Default)]
pub struct Browse {
    query: String,
    path: Vec<Level>,
}

/// What one render lists, by index.
pub struct Listing {
    /// Nothing typed, at the top: where a view puts what it leads with.
    pub top: bool,
    /// *My Places* that match, in their order.
    pub mine: Vec<u32>,
    /// The rest, *My Places* taken out.
    pub found: Found,
}

/// What a view draws for each kind of row.
pub trait Rows {
    fn city(&mut self, index: u32) -> Option<Item>;
    fn country(&mut self, index: u16) -> Option<Item>;
    fn continent(&mut self, continent: Continent) -> Item;
}

impl Browse {
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Take what is typed. `false` when it is what was there.
    pub fn set_query(&mut self, query: String) -> bool {
        if query == self.query {
            return false;
        }
        self.query = query;
        true
    }

    /// Take the innermost level off. `false` with none to take.
    pub fn back(&mut self) -> bool {
        self.path.pop().is_some()
    }

    /// Go into the continent or country `item` names, clearing the query.
    /// A country gone into from the top brings its continent, so `Back`
    /// goes up one level rather than all the way out. `false` for any other
    /// row.
    pub fn go_into(&mut self, places: &Catalogue, item: &str) -> bool {
        if let Some(code) = item.strip_prefix(CONTINENT) {
            let Some(continent) = Continent::ALL
                .into_iter()
                .find(|continent| continent.code() == code)
            else {
                return false;
            };
            self.path = vec![Level::Continent(continent)];
        } else if let Some(code) = item.strip_prefix(COUNTRY) {
            let Some(index) = places.country_by_code(code) else {
                return false;
            };
            let continent = places.country(index).map(|country| country.continent);
            if !matches!(self.path.last(), Some(Level::Continent(_))) {
                self.path.clear();
                self.path.extend(continent.map(Level::Continent));
            }
            self.path.push(Level::Country(index));
        } else {
            return false;
        }
        self.query.clear();
        true
    }

    pub fn within(&self) -> Within {
        match self.path.last() {
            None => Within::World,
            Some(Level::Continent(continent)) => Within::Continent(*continent),
            Some(Level::Country(country)) => Within::Country(*country),
        }
    }

    /// What to list, with `mine` -- *My Places* -- in a section of its own
    /// and nowhere else.
    pub fn listing(&self, places: &Catalogue, mine: &[u32]) -> Listing {
        let words = words(&self.query);
        let mut found = places.find(&words, self.within(), CITY_ROWS);
        let mine = places.find_among(&words, self.within(), mine);
        found.cities.retain(|index| !mine.contains(index));
        Listing {
            top: self.path.is_empty() && words.is_empty(),
            mine,
            found,
        }
    }

    /// The sections, in the order both views share: `first` (at the top
    /// only), *My Places*, then continents, countries and cities.
    pub fn sections(
        &self,
        listing: &Listing,
        first: Option<Section>,
        rows: &mut dyn Rows,
    ) -> Vec<Section> {
        let mut sections = Vec::new();
        if listing.top {
            sections.extend(first);
        }
        let section = |title: &str, subtitle: Option<String>, items: Vec<Item>| Section {
            title: Some(title.into()),
            subtitle,
            items,
        };
        if !listing.mine.is_empty() {
            let items = listing
                .mine
                .iter()
                .filter_map(|index| rows.city(*index))
                .collect();
            sections.push(section("My Places", None, items));
        }
        let found = &listing.found;
        if !found.continents.is_empty() {
            let items = found
                .continents
                .iter()
                .map(|continent| rows.continent(*continent))
                .collect();
            sections.push(section("Continents", None, items));
        }
        if !found.countries.is_empty() {
            let items = found
                .countries
                .iter()
                .filter_map(|index| rows.country(*index))
                .collect();
            sections.push(section("Countries", None, items));
        }
        if !found.cities.is_empty() {
            let items = found
                .cities
                .iter()
                .filter_map(|index| rows.city(*index))
                .collect();
            // `200 of 3,412` when the rows stop short of what matched.
            let more = (found.matched_cities > found.cities.len()).then(|| {
                format!(
                    "{} of {} \u{00b7} type to narrow",
                    found.cities.len(),
                    grouped(found.matched_cities)
                )
            });
            sections.push(section("Cities", more, items));
        }
        sections
    }

    pub fn placeholder(&self, places: &Catalogue) -> String {
        match self.path.last() {
            None => "Search for a city, a country or a continent".into(),
            Some(level) => format!("Search in {}", level_name(places, *level)),
        }
    }

    /// The pill's levels, outermost first.
    pub fn path_names(&self, places: &Catalogue) -> Vec<String> {
        self.path
            .iter()
            .map(|level| level_name(places, *level))
            .collect()
    }

    /// What an empty list says.
    pub fn empty(&self) -> EmptyState {
        EmptyState {
            title: format!("No place matches \u{201c}{}\u{201d}", self.query.trim()),
            description: Some(
                "Try a city, a region, a country or a continent: vancouver, california, canada, europe"
                    .into(),
            ),
            icon: None,
        }
    }
}

fn level_name(places: &Catalogue, level: Level) -> String {
    match level {
        Level::Continent(continent) => continent.name().into(),
        Level::Country(index) => places
            .country(index)
            .map(|country| country.name.to_string())
            .unwrap_or_default(),
    }
}

/// A city's row id.
pub fn city_id(index: u32) -> String {
    format!("{CITY}{index}")
}

/// The city a row id names.
pub fn city_index(item: &str) -> Option<u32> {
    item.strip_prefix(CITY)?.parse().ok()
}

/// The countries of `continent` that have a city in the catalogue.
pub fn countries_of(places: &Catalogue, continent: Continent) -> Vec<&Country> {
    places
        .countries()
        .iter()
        .filter(|country| country.continent == continent && country.cities > 0)
        .collect()
}

/// `3,412`.
pub fn grouped(number: usize) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::catalogue;
    use super::*;

    #[test]
    fn going_in_and_back_out() {
        let places = catalogue();
        let mut browse = Browse::default();
        browse.set_query("spain".into());
        assert!(browse.go_into(places, "country:ES"));
        assert_eq!(browse.query(), "", "what was typed found it");
        assert_eq!(browse.path_names(places), ["Europe", "Spain"]);
        assert_eq!(browse.placeholder(places), "Search in Spain");
        assert!(browse.back());
        assert_eq!(browse.path_names(places), ["Europe"]);
        assert!(browse.back());
        assert!(!browse.back(), "nothing left to take off");
        assert!(!browse.go_into(places, "city:1"), "a city is not a level");
    }

    #[test]
    fn my_places_are_listed_once() {
        let places = catalogue();
        let vancouver = places
            .cities()
            .iter()
            .position(|city| &*city.name == "Vancouver")
            .unwrap() as u32;
        let mut browse = Browse::default();
        browse.set_query("vancouver".into());
        let listing = browse.listing(places, &[vancouver]);
        assert_eq!(listing.mine, [vancouver]);
        assert!(!listing.found.cities.contains(&vancouver));
        assert!(!listing.top);
        assert!(Browse::default().listing(places, &[]).top);
    }

    #[test]
    fn row_ids_round_trip_and_numbers_group() {
        assert_eq!(city_index(&city_id(42)), Some(42));
        assert_eq!(city_index("country:ES"), None);
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(3412), "3,412");
        assert_eq!(grouped(24_874_500), "24,874,500");
    }
}
