//! Finding places by what is typed (F13).
//!
//! # Every word narrows
//!
//! A city is its name, its region, its country and its continent, and every
//! word typed must be the start of a word in one of them: `california`
//! finds Los Angeles through its region, `america canada van` finds
//! Vancouver through all four. Word starts rather than the root list's fuzzy
//! matcher, because a place is looked for by its name, not by its letters
//! scattered: `paris` must not find *Pyramids*.
//!
//! # The level is what the words name
//!
//! Thirty thousand cities are cheap to virtualise and useless to read, so
//! words that name only a continent list its countries, not every city in
//! it. Cities appear once a word reaches below a continent: a country, a
//! region or a city's own name.
//!
//! # Largest first
//!
//! There are two Vancouvers and several Madrids. The catalogue is in
//! population order, so taking matches in its order ranks them. A city
//! found only through a word that *starts* its country or region comes
//! after the rest: `van` is Vancouver before anywhere in Vanuatu. A word
//! that *is* the country or region counts as much as the city's own name,
//! so `spain` is Madrid before Port of Spain, and `korea` is Seoul before
//! Koreatown.

use super::{Catalogue, Continent, fold};

/// How far into the places the list has gone: the drill-down pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Within {
    World,
    Continent(Continent),
    /// A country, by its index in the catalogue.
    Country(u16),
}

/// What a query found, at each level.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Found {
    pub continents: Vec<Continent>,
    /// By index, those a word named first, largest first; then those found
    /// through their continent, alphabetically, for reading down a list.
    pub countries: Vec<u16>,
    /// By index, best first, at most the limit asked for.
    pub cities: Vec<u32>,
    /// How many cities matched in all, the limit notwithstanding.
    pub matched_cities: usize,
}

/// The words of a query, folded as the places are.
pub fn words(query: &str) -> Vec<String> {
    fold(query)
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether some word of `folded` starts with `word`.
fn starts_a_word(folded: &str, word: &str) -> bool {
    folded.split(' ').any(|part| part.starts_with(word))
}

/// How well a word matches a name: not at all, the start of one of its
/// words, or one of its words whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    None,
    Start,
    Whole,
}

fn fit(folded: &str, word: &str) -> Fit {
    folded
        .split(' ')
        .map(|part| {
            if part == word {
                Fit::Whole
            } else if part.starts_with(word) {
                Fit::Start
            } else {
                Fit::None
            }
        })
        .max()
        .unwrap_or(Fit::None)
}

/// What one word matches above the level of a city, worked out once per
/// query rather than once per city.
struct Word<'a> {
    text: &'a str,
    continents: Vec<Continent>,
    /// By country index: its name or an alias.
    countries: Vec<Fit>,
    /// By region index.
    regions: Vec<Fit>,
}

impl Catalogue {
    /// The places `words` find within `within`, at most `limit` cities.
    pub fn find(&self, words: &[String], within: Within, limit: usize) -> Found {
        let words = self.prepare(words);
        Found {
            continents: self.find_continents(&words, within),
            countries: self.find_countries(&words, within),
            ..self.find_cities(&words, within, limit)
        }
    }

    /// Which of `cities` -- the user's own, *My Places* -- `words` find
    /// within `within`, in their order (F13, slice 2). By the same rule as
    /// the list, so a favourite is shown exactly when it would be found;
    /// with nothing typed, every one in scope.
    pub fn find_among(&self, words: &[String], within: Within, cities: &[u32]) -> Vec<u32> {
        let words = self.prepare(words);
        cities
            .iter()
            .copied()
            .filter(|index| {
                self.city(*index).is_some_and(|city| {
                    if words.is_empty() {
                        self.in_scope(city, within)
                    } else {
                        self.city_fit(&words, within, city).is_some()
                    }
                })
            })
            .collect()
    }

    fn prepare<'a>(&self, words: &'a [String]) -> Vec<Word<'a>> {
        words
            .iter()
            .map(|word| Word {
                text: word,
                continents: Continent::ALL
                    .into_iter()
                    .filter(|continent| starts_a_word(continent.folded(), word))
                    .collect(),
                countries: self
                    .countries
                    .iter()
                    .map(|country| fit(&country.folded, word))
                    .collect(),
                regions: self
                    .regions
                    .iter()
                    .map(|region| fit(&region.folded, word))
                    .collect(),
            })
            .collect()
    }

    fn in_scope(&self, city: &super::City, within: Within) -> bool {
        match within {
            Within::World => true,
            Within::Continent(continent) => {
                self.countries[usize::from(city.country)].continent == continent
            }
            Within::Country(only) => city.country == only,
        }
    }

    /// Whether every word finds `city` within `within`, and if so whether a
    /// word *named* it: the start of its own name, or the whole of its
    /// region's or country's. `None` when it is not found, which includes
    /// words that reach no lower than its continent.
    fn city_fit(&self, words: &[Word], within: Within, city: &super::City) -> Option<bool> {
        if !self.in_scope(city, within) {
            return None;
        }
        let country = &self.countries[usize::from(city.country)];
        let mut named_by_a_word = false;
        let mut below_continent = matches!(within, Within::Country(_));
        let matches = words.iter().all(|word| {
            if starts_a_word(&city.folded, word.text) {
                named_by_a_word = true;
                below_continent = true;
                return true;
            }
            let region = if city.region == super::NO_REGION {
                Fit::None
            } else {
                word.regions[city.region as usize]
            };
            let above = region.max(word.countries[usize::from(city.country)]);
            if above != Fit::None {
                named_by_a_word |= above == Fit::Whole;
                below_continent = true;
                return true;
            }
            word.continents.contains(&country.continent)
        });
        (matches && below_continent).then_some(named_by_a_word)
    }

    /// At the top only: every continent with nothing typed, to browse; those
    /// every word names, otherwise.
    fn find_continents(&self, words: &[Word], within: Within) -> Vec<Continent> {
        if within != Within::World {
            return Vec::new();
        }
        Continent::ALL
            .into_iter()
            .filter(|continent| {
                words.iter().all(|word| word.continents.contains(continent))
                    && self
                        .countries
                        .iter()
                        .any(|country| country.continent == *continent && country.cities > 0)
            })
            .collect()
    }

    fn find_countries(&self, words: &[Word], within: Within) -> Vec<u16> {
        let scope = match within {
            Within::Country(_) => return Vec::new(),
            // At the top with nothing typed, the continents are the list.
            Within::World if words.is_empty() => return Vec::new(),
            Within::World => None,
            Within::Continent(continent) => Some(continent),
        };
        let mut named = Vec::new();
        let mut browsed = Vec::new();
        for (index, country) in self.countries.iter().enumerate() {
            if country.cities == 0 || scope.is_some_and(|scope| country.continent != scope) {
                continue;
            }
            let matches = words.iter().all(|word| {
                word.countries[index] != Fit::None || word.continents.contains(&country.continent)
            });
            if !matches {
                continue;
            }
            if words.iter().any(|word| word.countries[index] != Fit::None) {
                named.push(index as u16);
            } else {
                browsed.push(index as u16);
            }
        }
        named
            .sort_by_key(|index| std::cmp::Reverse(self.countries[usize::from(*index)].population));
        browsed.sort_by(|a, b| {
            self.countries[usize::from(*a)]
                .name
                .cmp(&self.countries[usize::from(*b)].name)
        });
        named.extend(browsed);
        named
    }

    fn find_cities(&self, words: &[Word], within: Within, limit: usize) -> Found {
        // With nothing typed, a country lists its cities; the top and a
        // continent list the level above.
        if words.is_empty() && !matches!(within, Within::Country(_)) {
            return Found::default();
        }

        let mut named = Vec::new();
        let mut found_through = Vec::new();
        let mut matched = 0;
        for (index, city) in self.cities.iter().enumerate() {
            let Some(named_by_a_word) = self.city_fit(words, within, city) else {
                continue;
            };

            matched += 1;
            if named_by_a_word {
                if named.len() < limit {
                    named.push(index as u32);
                }
            } else if found_through.len() < limit {
                found_through.push(index as u32);
            }
        }

        named.extend(found_through);
        named.truncate(limit);
        Found {
            cities: named,
            matched_cities: matched,
            ..Found::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalogue;
    use super::*;

    const LIMIT: usize = 200;

    fn find(query: &str, within: Within) -> Found {
        catalogue().find(&words(query), within, LIMIT)
    }

    fn city_names(found: &Found) -> Vec<String> {
        found
            .cities
            .iter()
            .map(|index| {
                let city = catalogue().city(*index).unwrap();
                format!("{}, {}", city.name, catalogue().country_of(city).code)
            })
            .collect()
    }

    fn country_names(found: &Found) -> Vec<&'static str> {
        found
            .countries
            .iter()
            .map(|index| &*catalogue().country(*index).unwrap().name)
            .collect()
    }

    fn country(code: &str) -> u16 {
        catalogue().country_by_code(code).unwrap()
    }

    #[test]
    fn the_largest_city_of_a_name_comes_first() {
        let found = find("vancouver", Within::World);
        assert_eq!(city_names(&found)[..2], ["Vancouver, CA", "Vancouver, US"]);
        assert_eq!(city_names(&find("paris", Within::World))[0], "Paris, FR");
    }

    #[test]
    fn a_name_is_found_without_its_accents_and_with_them() {
        for query in ["bogota", "Bogot\u{e1}", "BOGOTA"] {
            assert_eq!(
                city_names(&find(query, Within::World))[0],
                "Bogot\u{e1}, CO",
                "{query}"
            );
        }
    }

    #[test]
    fn a_word_at_any_level_narrows() {
        // Region -- Baja California's cities too, which is right.
        let california = city_names(&find("california", Within::World));
        assert_eq!(california[0], "Los Angeles, US");
        assert!(california.contains(&"San Diego, US".to_string()));
        // Continent, country and the start of a name.
        let found = find("america canada van", Within::World);
        assert_eq!(city_names(&found)[0], "Vancouver, CA");
        assert!(found.continents.is_empty());
        assert!(found.countries.is_empty());
    }

    #[test]
    fn a_continent_alone_lists_countries_not_cities() {
        let found = find("europe", Within::World);
        assert_eq!(found.continents, [Continent::Europe]);
        assert!(country_names(&found).contains(&"Spain"));
        assert!(found.cities.is_empty(), "the level is the countries");
        assert_eq!(found.matched_cities, 0);

        let found = find("america", Within::World);
        assert_eq!(
            found.continents,
            [Continent::NorthAmerica, Continent::SouthAmerica]
        );
    }

    #[test]
    fn a_country_is_listed_and_so_are_its_cities() {
        let found = find("spain", Within::World);
        assert_eq!(country_names(&found), ["Spain"]);
        assert_eq!(city_names(&found)[0], "Madrid, ES");
        assert!(found.matched_cities > 50);
    }

    #[test]
    fn countries_answer_to_their_aliases_largest_first() {
        assert_eq!(
            country_names(&find("usa", Within::World)),
            ["United States"]
        );
        assert_eq!(
            country_names(&find("uk", Within::World))[0],
            "United Kingdom"
        );
        assert_eq!(
            country_names(&find("korea", Within::World)),
            ["South Korea", "North Korea"]
        );
        assert_eq!(city_names(&find("korea", Within::World))[0], "Seoul, KR");
    }

    #[test]
    fn nothing_typed_at_the_top_is_the_continents() {
        let found = find("", Within::World);
        assert_eq!(found.continents.len(), 7);
        assert!(found.countries.is_empty());
        assert!(found.cities.is_empty());
    }

    #[test]
    fn within_a_continent_its_countries_alphabetically() {
        let found = find("", Within::Continent(Continent::Europe));
        let names = country_names(&found);
        assert!(names.contains(&"Spain"));
        assert!(!names.contains(&"Canada"));
        assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(found.cities.is_empty());

        // Typing filters the countries, and finds cities by name.
        let found = find("sp", Within::Continent(Continent::Europe));
        assert_eq!(country_names(&found), ["Spain"]);
        let found = find("madrid", Within::Continent(Continent::Europe));
        assert_eq!(city_names(&found)[0], "Madrid, ES");
        assert!(
            city_names(&found)
                .iter()
                .all(|name| !name.ends_with(", CO")),
            "Madrid in Colombia is not in Europe"
        );
    }

    #[test]
    fn within_a_country_its_cities_largest_first() {
        let found = find("", Within::Country(country("ca")));
        assert_eq!(
            city_names(&found)[..2],
            ["Toronto, CA", "Montr\u{e9}al, CA"]
        );
        assert!(found.continents.is_empty() && found.countries.is_empty());

        let found = find("van", Within::Country(country("ca")));
        assert_eq!(city_names(&found)[0], "Vancouver, CA");
        assert!(city_names(&found).iter().all(|name| name.ends_with(", CA")));
    }

    #[test]
    fn the_limit_caps_the_rows_not_the_count() {
        let found = catalogue().find(&words("united states"), Within::World, 10);
        assert_eq!(found.cities.len(), 10);
        assert!(found.matched_cities > 1000);
    }

    #[test]
    fn a_city_named_by_a_word_comes_before_one_found_through_its_country() {
        // "Canada" matches every Canadian city through its country; "Van"
        // names Vancouver itself.
        let found = find("canada van", Within::World);
        assert_eq!(city_names(&found)[0], "Vancouver, CA");
    }

    /// A word that is a country counts as much as a city's own name, so the
    /// country's largest city is not buried under a small namesake.
    #[test]
    fn a_whole_country_beats_a_namesake_town() {
        assert_eq!(city_names(&find("spain", Within::World))[0], "Madrid, ES");
        // But the start of a country's name does not: `van` is Vancouver,
        // not Port Vila in Vanuatu.
        assert_eq!(city_names(&find("van", Within::World))[0], "Vancouver, CA");
    }

    /// *My Places* are found by the list's own rule, in their own order.
    #[test]
    fn favourites_are_found_by_the_same_rule() {
        let index = |name: &str| {
            catalogue()
                .cities()
                .iter()
                .position(|city| &*city.name == name)
                .unwrap() as u32
        };
        let mine = [index("Vancouver"), index("Madrid"), index("Toronto")];
        let among = |query: &str, within| catalogue().find_among(&words(query), within, &mine);

        assert_eq!(among("", Within::World), mine);
        assert_eq!(among("canada", Within::World), [mine[0], mine[2]]);
        assert_eq!(among("mad", Within::World), [mine[1]]);
        assert!(
            among("europe", Within::World).is_empty(),
            "a continent alone"
        );
        assert_eq!(among("", Within::Continent(Continent::Europe)), [mine[1]]);
        assert_eq!(
            among("", Within::Country(country("ca"))),
            [mine[0], mine[2]]
        );
    }

    #[test]
    fn nothing_found_is_empty_everywhere() {
        let found = find("zzzzqx", Within::World);
        assert_eq!(found, Found::default());
    }
}
