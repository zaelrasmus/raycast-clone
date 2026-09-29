//! Currency conversion (F9).
//!
//! # Where the network is
//!
//! Only in [`RateSource::fetch`], called from a thread the daemon starts,
//! and at most once per published table: a table is every rate against one
//! base, and every conversion after it arrives is a division. Root search
//! and the view read a table in memory and never ask for one (I-3).
//!
//! # What is here and what is not
//!
//! The currencies, the table and how old it is, the sources that fetch it,
//! the state kept in the data folder, and [`session::Session`] -- the view's
//! decisions as a plain value. The thread, the client and the file writes
//! are the daemon's (`lumen-app`), as they are for Translate.

pub mod currencies;
pub mod format;
pub mod query;
pub mod session;
pub mod sources;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub use crate::net::{Failure, failure};
pub use currencies::{CURRENCIES, Currency};

/// The currency a first conversion is from, until one is chosen.
pub const DEFAULT_FROM: &str = "USD";

/// Now, in seconds since the epoch: what [`session::Session::start`] is given.
pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// The `currency` section of `config.json`.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CurrencyConfig {
    pub service: Service,
    /// The key for a service that needs one. Unused by `open`.
    pub api_key: Option<String>,
}

/// Written by hand so that the key never reaches a log line, as
/// `TranslateConfig` is.
impl std::fmt::Debug for CurrencyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrencyConfig")
            .field("service", &self.service)
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

impl CurrencyConfig {
    /// The key, if one is written and not blank.
    pub fn key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }
}

/// Who is asked for rates.
///
/// Chosen, never fallen back between, as in F7: the user picked who is asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    /// ExchangeRate-API's open endpoint: no key, a table a day.
    #[default]
    Open,
    /// ExchangeRate-API with the user's key: the same data, their quota.
    ExchangerateApi,
    /// UniRateAPI, with the user's key.
    Unirate,
}

impl Service {
    /// The name shown beside the date. For `open`, this is the attribution
    /// its terms ask for.
    pub fn label(self) -> &'static str {
        match self {
            Service::Open | Service::ExchangerateApi => "ExchangeRate-API",
            Service::Unirate => "UniRateAPI",
        }
    }
}

/// Which table: today's, or one day's.
///
/// No source built so far has history on a free plan, so all of them refuse
/// [`When::On`]. It is in the contract so that historical rates are one more
/// source when they come, not a change to every caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Latest,
    On(jiff::civil::Date),
}

/// One published table: every rate against one base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Rates {
    /// Who published it, so a change of service is not answered with another
    /// service's table under the new one's name.
    pub service: Service,
    pub base: String,
    /// Units of each currency per one of `base`. Codes not in the table of
    /// currencies are dropped when the table is read.
    pub rates: BTreeMap<String, f64>,
    /// When it was published, and when the next one is due, in seconds since
    /// the epoch.
    pub published: i64,
    pub next: i64,
}

impl Rates {
    /// `amount` of `from`, in `to`. `None` when either has no rate.
    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        let from = self.rates.get(from).copied().filter(|rate| *rate > 0.0)?;
        let to = self.rates.get(to).copied()?;
        Some(amount / from * to)
    }

    /// Whether a newer table should exist by `now` (seconds since the epoch).
    pub fn is_due(&self, now: i64) -> bool {
        now >= self.next
    }

    /// The day it was published, in UTC -- which is the day it is published
    /// for: "28 Sep 2026".
    pub fn date(&self) -> String {
        match jiff::Timestamp::from_second(self.published) {
            Ok(moment) => moment
                .to_zoned(jiff::tz::TimeZone::UTC)
                .strftime("%-d %b %Y")
                .to_string(),
            Err(_) => "an unknown date".into(),
        }
    }

    /// Keep only the currencies in the table, and only sensible rates.
    pub fn retain_known(&mut self) {
        self.rates.retain(|code, rate| {
            currencies::find(code).is_some() && rate.is_finite() && *rate > 0.0
        });
    }
}

/// Somewhere to get a table from.
///
/// Blocking, on purpose: it is only ever called from a thread of its own.
pub trait RateSource: Send + Sync {
    fn fetch(&self, when: When) -> Result<Rates>;
}

/// What is kept in `currency.json` in the data folder: the table last
/// fetched, and what the user chose in the view.
///
/// State, not configuration: nobody edits it by hand, and losing it costs a
/// request and a choice, not a setting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct State {
    /// The currency converted from, by code.
    pub from: Option<String>,
    /// The currencies pinned above the rest, by code (F9, slice 4).
    pub pinned: Vec<String>,
    pub rates: Option<Rates>,
}

impl State {
    /// Read the state, or start empty. Lenient: a file that does not parse
    /// costs one fetch, and is not worth refusing to open the view over.
    pub fn load(path: &Path) -> Self {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                return Self::default();
            }
        };
        let mut state: Self = serde_json::from_str(&source).unwrap_or_else(|error| {
            log::warn!("{} is not currency state: {error}", path.display());
            Self::default()
        });
        if let Some(rates) = &mut state.rates {
            rates.retain_known();
        }
        state
    }

    /// Write the state, renaming a sibling over the file so a crash mid-write
    /// leaves the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        let source = serde_json::to_string_pretty(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// The pinned currencies Lumen still knows, once each.
    pub fn pinned(&self) -> Vec<&'static Currency> {
        let mut pinned: Vec<&'static Currency> = Vec::new();
        for currency in self.pinned.iter().filter_map(|code| currencies::find(code)) {
            if !pinned.contains(&currency) {
                pinned.push(currency);
            }
        }
        pinned
    }

    /// The currency to convert from: the one chosen, or the default.
    pub fn from(&self) -> &'static Currency {
        self.from
            .as_deref()
            .and_then(currencies::find)
            .or_else(|| currencies::find(DEFAULT_FROM))
            .expect("the default currency is in the table")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rates(pairs: &[(&str, f64)]) -> Rates {
        Rates {
            service: Service::Open,
            base: "USD".into(),
            rates: pairs
                .iter()
                .map(|(code, rate)| (code.to_string(), *rate))
                .collect(),
            // 2026-09-28 00:02:31 UTC, and a day later.
            published: 1_790_640_151 - 86_400,
            next: 1_790_640_151,
        }
    }

    #[test]
    fn a_conversion_crosses_the_base() {
        let rates = rates(&[("USD", 1.0), ("CAD", 1.4), ("EUR", 0.9)]);
        let euros = rates.convert(14.0, "CAD", "EUR").unwrap();
        assert!((euros - 9.0).abs() < 1e-9, "{euros}");
        assert_eq!(rates.convert(1.0, "CAD", "XYZ"), None);
    }

    #[test]
    fn a_zero_rate_is_not_divided_by() {
        let rates = rates(&[("USD", 1.0), ("CAD", 0.0)]);
        assert_eq!(rates.convert(1.0, "CAD", "USD"), None);
    }

    #[test]
    fn the_date_is_the_publication_day_in_utc() {
        assert_eq!(rates(&[]).date(), "28 Sep 2026");
    }

    #[test]
    fn a_table_is_due_at_its_next_moment() {
        let rates = rates(&[]);
        assert!(!rates.is_due(rates.next - 1));
        assert!(rates.is_due(rates.next));
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let config: CurrencyConfig =
            serde_json::from_str(r#"{ "service": "unirate", "api-key": "secret-123" }"#).unwrap();
        let printed = format!("{config:?}");
        assert!(!printed.contains("secret-123"));
        assert!(printed.contains("<set>"));
        assert_eq!(config.service, Service::Unirate);
    }

    #[test]
    fn a_blank_key_is_no_key() {
        let config: CurrencyConfig = serde_json::from_str(r#"{ "api-key": "  " }"#).unwrap();
        assert_eq!(config.key(), None);
        assert_eq!(config.service, Service::Open);
    }

    #[test]
    fn state_round_trips_and_drops_unknown_codes() {
        let folder = std::env::temp_dir().join(format!("lumen-currency-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("currency.json");

        let state = State {
            from: Some("CAD".into()),
            pinned: vec!["EUR".into(), "ZZZ".into(), "EUR".into()],
            rates: Some(rates(&[("USD", 1.0), ("CAD", 1.4), ("ZZZ", 2.0)])),
        };
        state.save(&path).unwrap();
        let loaded = State::load(&path);
        std::fs::remove_dir_all(&folder).ok();

        assert_eq!(loaded.from().code, "CAD");
        let pinned: Vec<&str> = loaded.pinned().iter().map(|pin| pin.code).collect();
        assert_eq!(pinned, ["EUR"], "known, once each");
        let codes: Vec<&String> = loaded.rates.as_ref().unwrap().rates.keys().collect();
        assert_eq!(codes, ["CAD", "USD"]);
    }

    #[test]
    fn missing_or_broken_state_starts_empty_from_the_default() {
        let path = std::env::temp_dir().join("lumen-currency-does-not-exist.json");
        let state = State::load(&path);
        assert_eq!(state, State::default());
        assert_eq!(state.from().code, DEFAULT_FROM);
    }
}
