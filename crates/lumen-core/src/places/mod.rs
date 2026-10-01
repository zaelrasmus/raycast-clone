//! The places Lumen knows the time and the weather of (F13).
//!
//! # Bundled, not fetched
//!
//! GeoNames' `cities15000` -- every city over 15,000 people -- reduced by
//! `scripts/make-places.ps1` to what a clock and a forecast need, and
//! compiled into the binary. Finding a city, its zone and its coordinates
//! never touches the network, so the clock works offline and the weather
//! needs no geocoding service.
//!
//! # Loaded once, when first asked for
//!
//! Decompressing and reading 34,000 lines is a few tens of milliseconds and
//! a few megabytes, so it is not done at start: a daemon nobody asks the
//! time of never pays for it. [`catalogue`] reads it on first use and keeps
//! it. Callers on the UI thread ask for it from the background executor.

pub mod browse;
mod favourites;
mod find;
pub mod store;

use std::io::Read;
use std::sync::OnceLock;

pub use favourites::Favourites;
pub use find::{Found, Within, words};

const CITIES: &[u8] = include_bytes!("../../../../assets/places/cities.tsv.gz");
const COUNTRIES: &str = include_str!("../../../../assets/places/countries.tsv");

/// The seven continents, as GeoNames assigns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Continent {
    Africa,
    Antarctica,
    Asia,
    Europe,
    NorthAmerica,
    Oceania,
    SouthAmerica,
}

impl Continent {
    pub const ALL: [Continent; 7] = [
        Continent::Africa,
        Continent::Antarctica,
        Continent::Asia,
        Continent::Europe,
        Continent::NorthAmerica,
        Continent::Oceania,
        Continent::SouthAmerica,
    ];

    fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "AF" => Continent::Africa,
            "AN" => Continent::Antarctica,
            "AS" => Continent::Asia,
            "EU" => Continent::Europe,
            "NA" => Continent::NorthAmerica,
            "OC" => Continent::Oceania,
            "SA" => Continent::SouthAmerica,
            _ => return None,
        })
    }

    /// GeoNames' two letters, which also stand for the continent in a row's
    /// icon square.
    pub fn code(self) -> &'static str {
        match self {
            Continent::Africa => "AF",
            Continent::Antarctica => "AN",
            Continent::Asia => "AS",
            Continent::Europe => "EU",
            Continent::NorthAmerica => "NA",
            Continent::Oceania => "OC",
            Continent::SouthAmerica => "SA",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Continent::Africa => "Africa",
            Continent::Antarctica => "Antarctica",
            Continent::Asia => "Asia",
            Continent::Europe => "Europe",
            Continent::NorthAmerica => "North America",
            Continent::Oceania => "Oceania",
            Continent::SouthAmerica => "South America",
        }
    }

    /// What a query is matched against: the name, and what people call the
    /// continent besides.
    fn folded(self) -> &'static str {
        match self {
            Continent::Africa => "africa",
            Continent::Antarctica => "antarctica",
            Continent::Asia => "asia",
            Continent::Europe => "europe",
            Continent::NorthAmerica => "north america",
            Continent::Oceania => "oceania australasia",
            Continent::SouthAmerica => "south america latin america",
        }
    }
}

/// Other names a country answers to, beside its own (F13).
///
/// Hand-written and small on purpose: the abbreviations and older names
/// people actually type, not a gazetteer. A name already in the country's
/// own -- `korea` in *South Korea* -- needs no entry.
const ALIASES: &[(&str, &str)] = &[
    ("US", "usa us america united states of america"),
    ("GB", "uk britain great britain"),
    ("AE", "uae emirates"),
    ("CZ", "czech republic"),
    ("NL", "holland"),
    ("MM", "burma"),
    ("TR", "turkiye"),
    ("CI", "cote d ivoire"),
    ("CV", "cabo verde"),
    ("SZ", "swaziland"),
    ("MK", "macedonia"),
    ("CD", "drc congo kinshasa"),
    ("CG", "congo brazzaville"),
    ("VA", "holy see"),
];

/// A country, and what the list says about it.
#[derive(Debug)]
pub struct Country {
    /// ISO 3166 alpha-2, which is also its icon (`Icon::Text`, as in F9).
    pub code: Box<str>,
    pub name: Box<str>,
    pub continent: Continent,
    pub population: u64,
    /// The name and its aliases, folded.
    folded: Box<str>,
    /// How many cities in the catalogue are in it.
    pub cities: u32,
    /// The zones its cities are in, each once.
    zones: Vec<u16>,
}

/// A region -- a state, a province -- interned, since thousands of cities
/// share a few thousand of them.
#[derive(Debug)]
struct Region {
    name: Box<str>,
    folded: Box<str>,
}

/// One city.
#[derive(Debug)]
pub struct City {
    pub name: Box<str>,
    /// The name, folded, for matching.
    folded: Box<str>,
    country: u16,
    /// Into the catalogue's regions, or [`NO_REGION`].
    region: u32,
    zone: u16,
    pub latitude: f32,
    pub longitude: f32,
    pub population: u32,
}

const NO_REGION: u32 = u32::MAX;

/// Every place, in population order: the largest city first.
#[derive(Debug)]
pub struct Catalogue {
    countries: Vec<Country>,
    regions: Vec<Region>,
    zones: Vec<Box<str>>,
    cities: Vec<City>,
}

static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();

/// The catalogue if it has been read, without reading it: what the root
/// list asks on a keystroke, which must not wait for it (I-3).
pub fn loaded() -> Option<&'static Catalogue> {
    CATALOGUE.get()
}

/// The catalogue, read on first use and kept. Blocks for as long as
/// reading takes, so the UI thread asks [`loaded`] instead.
pub fn catalogue() -> &'static Catalogue {
    CATALOGUE.get_or_init(|| {
        let started = std::time::Instant::now();
        let catalogue = Catalogue::bundled();
        log::info!(
            "read {} places in {} countries in {:?}",
            catalogue.cities.len(),
            catalogue.countries.len(),
            started.elapsed()
        );
        catalogue
    })
}

impl Catalogue {
    /// The catalogue compiled into the binary.
    fn bundled() -> Self {
        let mut text = String::new();
        if let Err(error) = flate2::read::GzDecoder::new(CITIES).read_to_string(&mut text) {
            // The file is the binary's own, so this is a broken build, not
            // a broken machine: say so, and run with no places.
            log::error!("the bundled places are unreadable: {error}");
            text.clear();
        }
        Self::parse(COUNTRIES, &text)
    }

    /// Read the two files `scripts/make-places.ps1` writes. A line that does
    /// not parse is skipped, not fatal: one bad row must not cost every city.
    fn parse(countries: &str, cities: &str) -> Self {
        let mut catalogue = Catalogue {
            countries: Vec::new(),
            regions: Vec::new(),
            zones: Vec::new(),
            cities: Vec::new(),
        };

        for line in countries.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [code, name, continent, population] = fields[..] else {
                continue;
            };
            let Some(continent) = Continent::from_code(continent) else {
                continue;
            };
            let aliases = ALIASES
                .iter()
                .find(|(alias_code, _)| *alias_code == code)
                .map(|(_, aliases)| *aliases)
                .unwrap_or_default();
            catalogue.countries.push(Country {
                code: code.into(),
                name: name.into(),
                continent,
                population: population.parse().unwrap_or(0),
                folded: fold(&format!("{name} {aliases}")).into(),
                cities: 0,
                zones: Vec::new(),
            });
        }

        let mut regions = std::collections::HashMap::<(u16, &str), u32>::new();
        let mut zones = std::collections::HashMap::<&str, u16>::new();
        for line in cities.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [
                name,
                ascii,
                country,
                region,
                region_ascii,
                zone,
                latitude,
                longitude,
                population,
            ] = fields[..]
            else {
                continue;
            };
            let Some(country) = catalogue
                .countries
                .iter()
                .position(|known| &*known.code == country)
            else {
                continue;
            };
            let country = country as u16;
            let (Ok(latitude), Ok(longitude)) = (latitude.parse(), longitude.parse()) else {
                continue;
            };

            let region = if region.is_empty() {
                NO_REGION
            } else {
                *regions.entry((country, region)).or_insert_with(|| {
                    let spelling = if region_ascii.is_empty() {
                        region
                    } else {
                        region_ascii
                    };
                    catalogue.regions.push(Region {
                        name: region.into(),
                        folded: fold(spelling).into(),
                    });
                    (catalogue.regions.len() - 1) as u32
                })
            };
            let zone = *zones.entry(zone).or_insert_with(|| {
                catalogue.zones.push(zone.into());
                (catalogue.zones.len() - 1) as u16
            });

            let entry = &mut catalogue.countries[usize::from(country)];
            entry.cities += 1;
            if !entry.zones.contains(&zone) {
                entry.zones.push(zone);
            }

            let spelling = if ascii.is_empty() { name } else { ascii };
            catalogue.cities.push(City {
                name: name.into(),
                folded: fold(spelling).into(),
                country,
                region,
                zone,
                latitude,
                longitude,
                population: population.parse().unwrap_or(0),
            });
        }
        catalogue
    }

    pub fn cities(&self) -> &[City] {
        &self.cities
    }

    pub fn city(&self, index: u32) -> Option<&City> {
        self.cities.get(index as usize)
    }

    pub fn countries(&self) -> &[Country] {
        &self.countries
    }

    pub fn country(&self, index: u16) -> Option<&Country> {
        self.countries.get(usize::from(index))
    }

    /// A country by its ISO code, case-insensitively.
    pub fn country_by_code(&self, code: &str) -> Option<u16> {
        self.countries
            .iter()
            .position(|country| country.code.eq_ignore_ascii_case(code))
            .map(|index| index as u16)
    }

    /// The country `city` is in.
    pub fn country_of(&self, city: &City) -> &Country {
        &self.countries[usize::from(city.country)]
    }

    pub fn country_index(&self, city: &City) -> u16 {
        city.country
    }

    /// The region `city` is in -- *British Columbia* -- if GeoNames names one.
    pub fn region_of(&self, city: &City) -> Option<&str> {
        self.regions
            .get(city.region as usize)
            .map(|region| &*region.name)
    }

    /// The IANA zone `city` keeps time in: `America/Vancouver`.
    pub fn zone_of(&self, city: &City) -> &str {
        &self.zones[usize::from(city.zone)]
    }

    /// The zones a country's cities are in, each once.
    pub fn zones_of(&self, country: &Country) -> impl Iterator<Item = &str> {
        country
            .zones
            .iter()
            .map(|zone| &*self.zones[usize::from(*zone)])
    }

    /// The largest city that keeps time in `zone`, which is what the user's
    /// own zone is called in the list: `America/Santo_Domingo` is Santo
    /// Domingo.
    pub fn largest_in_zone(&self, zone: &str) -> Option<u32> {
        let zone = self.zones.iter().position(|known| &**known == zone)? as u16;
        self.cities
            .iter()
            .position(|city| city.zone == zone)
            .map(|index| index as u32)
    }

    /// Where a city is, for its subtitle: *British Columbia, Canada*.
    pub fn whereabouts(&self, city: &City) -> String {
        let country = &self.country_of(city).name;
        match self.region_of(city) {
            // A city-state, or a capital district named after its city:
            // *Singapore, Singapore* says nothing twice.
            Some(region) if region != &*city.name && region != &**country => {
                format!("{region}, {country}")
            }
            _ => country.to_string(),
        }
    }
}

/// Lowercase ASCII words separated by single spaces: what both the places
/// and the query are reduced to before they are compared.
///
/// Accents are folded, so `bogota` finds *Bogot\u{e1}* and `zurich` finds
/// *Z\u{fc}rich*; punctuation separates words, so `saint denis` finds
/// *Saint-Denis*. A letter with no ASCII form is kept, lowercased, so a
/// name typed in its own script still matches itself.
pub fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            folded.push(c.to_ascii_lowercase());
        } else if c.is_ascii() {
            folded.push(' ');
        } else {
            let lower = c.to_lowercase().next().unwrap_or(c);
            match ascii(lower) {
                Some(plain) => folded.push_str(plain),
                None if lower.is_alphanumeric() => folded.push(lower),
                None => folded.push(' '),
            }
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The ASCII a lowercase Latin letter with a mark is typed as.
fn ascii(c: char) -> Option<&'static str> {
    const TABLE: &[(&str, &str)] = &[
        (
            "\u{e0}\u{e1}\u{e2}\u{e3}\u{e4}\u{e5}\u{101}\u{103}\u{105}",
            "a",
        ),
        ("\u{e6}", "ae"),
        ("\u{e7}\u{107}\u{109}\u{10b}\u{10d}", "c"),
        ("\u{10f}\u{111}\u{f0}", "d"),
        (
            "\u{e8}\u{e9}\u{ea}\u{eb}\u{113}\u{115}\u{117}\u{119}\u{11b}",
            "e",
        ),
        ("\u{11d}\u{11f}\u{121}\u{123}", "g"),
        ("\u{125}\u{127}", "h"),
        (
            "\u{ec}\u{ed}\u{ee}\u{ef}\u{129}\u{12b}\u{12d}\u{12f}\u{131}",
            "i",
        ),
        ("\u{135}", "j"),
        ("\u{137}", "k"),
        ("\u{13a}\u{13c}\u{13e}\u{140}\u{142}", "l"),
        ("\u{f1}\u{144}\u{146}\u{148}", "n"),
        (
            "\u{f2}\u{f3}\u{f4}\u{f5}\u{f6}\u{f8}\u{14d}\u{14f}\u{151}",
            "o",
        ),
        ("\u{153}", "oe"),
        ("\u{155}\u{157}\u{159}", "r"),
        ("\u{15b}\u{15d}\u{15f}\u{161}\u{219}", "s"),
        ("\u{df}", "ss"),
        ("\u{163}\u{165}\u{167}\u{21b}", "t"),
        ("\u{fe}", "th"),
        (
            "\u{f9}\u{fa}\u{fb}\u{fc}\u{169}\u{16b}\u{16d}\u{16f}\u{171}\u{173}",
            "u",
        ),
        ("\u{175}", "w"),
        ("\u{fd}\u{ff}\u{177}", "y"),
        ("\u{17a}\u{17c}\u{17e}", "z"),
    ];
    TABLE
        .iter()
        .find(|(marked, _)| marked.contains(c))
        .map(|(_, plain)| *plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_and_punctuation_fold_away() {
        assert_eq!(fold("Bogot\u{e1}"), "bogota");
        assert_eq!(fold("Z\u{fc}rich"), "zurich");
        assert_eq!(fold("Saint-Denis"), "saint denis");
        assert_eq!(fold("  S\u{e3}o   Paulo "), "sao paulo");
        assert_eq!(fold("Stra\u{df}e"), "strasse");
        assert_eq!(fold("\u{c5}lesund"), "alesund");
        // A script with no ASCII form still matches itself.
        assert_eq!(fold("\u{6771}\u{4eac}"), "\u{6771}\u{4eac}");
    }

    #[test]
    fn the_bundled_catalogue_reads_whole() {
        let catalogue = catalogue();
        assert!(catalogue.cities().len() > 30_000);
        assert!(catalogue.countries().len() > 240);
        // Largest first.
        let populations: Vec<u32> = catalogue
            .cities()
            .iter()
            .take(100)
            .map(|city| city.population)
            .collect();
        assert!(populations.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn a_city_knows_its_zone_region_and_country() {
        let catalogue = catalogue();
        let vancouver = catalogue
            .cities()
            .iter()
            .find(|city| &*city.name == "Vancouver")
            .unwrap();
        assert_eq!(catalogue.zone_of(vancouver), "America/Vancouver");
        assert_eq!(catalogue.region_of(vancouver), Some("British Columbia"));
        assert_eq!(&*catalogue.country_of(vancouver).name, "Canada");
        assert_eq!(catalogue.whereabouts(vancouver), "British Columbia, Canada");
    }

    #[test]
    fn the_users_zone_is_named_after_its_largest_city() {
        let catalogue = catalogue();
        let index = catalogue.largest_in_zone("America/Santo_Domingo").unwrap();
        assert_eq!(&*catalogue.city(index).unwrap().name, "Santo Domingo");
        assert!(catalogue.largest_in_zone("Mars/Olympus_Mons").is_none());
    }

    #[test]
    fn a_country_with_several_zones_knows_them() {
        let catalogue = catalogue();
        let us = catalogue.country_by_code("us").unwrap();
        let zones: Vec<&str> = catalogue.zones_of(catalogue.country(us).unwrap()).collect();
        assert!(zones.contains(&"America/New_York"));
        assert!(zones.contains(&"America/Los_Angeles"));
    }

    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let catalogue = Catalogue::parse(
            "CA\tCanada\tNA\t37000000\nXX\tNowhere\tZZ\t0\n",
            "Vancouver\t\tCA\tBritish Columbia\t\tAmerica/Vancouver\t49.25\t-123.119\t662248\n\
             broken line\n\
             Atlantis\t\tXX\t\t\tEtc/UTC\t0\t0\t1\n",
        );
        assert_eq!(catalogue.countries().len(), 1, "an unknown continent");
        assert_eq!(
            catalogue.cities().len(),
            1,
            "a broken line, an unknown country"
        );
    }
}
