//! *My Places*: the cities the user keeps first, in World Clock and in
//! Weather alike (F13, slice 2).
//!
//! Kept in `places.json` in the data folder, by a key that names the city
//! -- `CA/British Columbia/Vancouver` -- rather than by its index in the
//! catalogue, which moves whenever GeoNames is read again. A key the
//! catalogue no longer knows is kept in the file and skipped in the list, so
//! a city renamed upstream is not silently forgotten by a save.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::{Catalogue, City};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Favourites {
    /// Keys, in the order they were added.
    pub cities: Vec<String>,
}

impl Favourites {
    /// What `places.json` holds, or nothing when there is no file or it
    /// does not read: a favourite lost is a nuisance, a launcher that will
    /// not open over one is worse.
    pub fn load(path: &Path) -> Self {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&source).unwrap_or_else(|error| {
            log::warn!("{} is not a list of places: {error}", path.display());
            Self::default()
        })
    }

    /// Write the list, renaming a sibling over the file so a crash mid-write
    /// leaves the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// The cities `catalogue` still knows, by index, once each, in order.
    pub fn resolve(&self, catalogue: &Catalogue) -> Vec<u32> {
        let mut cities: Vec<u32> = Vec::new();
        for index in self
            .cities
            .iter()
            .filter_map(|key| catalogue.city_by_key(key))
        {
            if !cities.contains(&index) {
                cities.push(index);
            }
        }
        cities
    }

    pub fn contains(&self, key: &str) -> bool {
        self.cities.iter().any(|kept| kept == key)
    }

    /// Add `key` at the end, if it is not there.
    pub fn add(&mut self, key: String) -> bool {
        if self.contains(&key) {
            return false;
        }
        self.cities.push(key);
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.cities.len();
        self.cities.retain(|kept| kept != key);
        self.cities.len() != before
    }
}

impl Catalogue {
    /// The key a city is kept by: country, region and name.
    pub fn key(&self, city: &City) -> String {
        format!(
            "{}/{}/{}",
            self.country_of(city).code,
            self.region_of(city).unwrap_or_default(),
            city.name
        )
    }

    /// The city a key names, if the catalogue still has it.
    pub fn city_by_key(&self, key: &str) -> Option<u32> {
        let mut parts = key.splitn(3, '/');
        let (country, region, name) = (parts.next()?, parts.next()?, parts.next()?);
        let country = self.country_by_code(country)?;
        self.cities
            .iter()
            .position(|city| {
                city.country == country
                    && &*city.name == name
                    && self.region_of(city).unwrap_or_default() == region
            })
            .map(|index| index as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalogue;
    use super::*;

    fn index(name: &str, country: &str) -> u32 {
        let catalogue = catalogue();
        catalogue
            .cities()
            .iter()
            .position(|city| &*city.name == name && &*catalogue.country_of(city).code == country)
            .unwrap() as u32
    }

    #[test]
    fn a_city_is_kept_by_a_key_that_names_it() {
        let catalogue = catalogue();
        let vancouver = index("Vancouver", "CA");
        let key = catalogue.key(catalogue.city(vancouver).unwrap());
        assert_eq!(key, "CA/British Columbia/Vancouver");
        assert_eq!(catalogue.city_by_key(&key), Some(vancouver));
        // The other Vancouver is another key.
        let washington = catalogue.key(catalogue.city(index("Vancouver", "US")).unwrap());
        assert_eq!(washington, "US/Washington/Vancouver");
    }

    #[test]
    fn unknown_keys_are_skipped_but_kept_and_duplicates_shown_once() {
        let favourites = Favourites {
            cities: vec![
                "CO/Bogota D.C./Bogot\u{e1}".into(),
                "XX/Nowhere/Atlantis".into(),
                "CO/Bogota D.C./Bogot\u{e1}".into(),
                "broken".into(),
            ],
        };
        assert_eq!(
            favourites.resolve(catalogue()),
            [index("Bogot\u{e1}", "CO")]
        );
        assert_eq!(favourites.cities.len(), 4, "nothing is dropped by reading");
    }

    #[test]
    fn adding_and_removing() {
        let mut favourites = Favourites::default();
        assert!(favourites.add("a".into()));
        assert!(!favourites.add("a".into()), "once");
        assert!(favourites.add("b".into()));
        assert!(favourites.remove("a"));
        assert!(!favourites.remove("a"));
        assert_eq!(favourites.cities, ["b"]);
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("lumen-places-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("places.json");
        assert_eq!(Favourites::load(&path), Favourites::default());

        let favourites = Favourites {
            cities: vec!["CA/British Columbia/Vancouver".into()],
        };
        favourites.save(&path).unwrap();
        assert_eq!(Favourites::load(&path), favourites);

        std::fs::write(&path, "{ nonsense").unwrap();
        assert_eq!(Favourites::load(&path), Favourites::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
