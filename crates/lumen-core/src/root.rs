//! The root list: what is searched, and how the pieces are ranked together.
//!
//! [`crate::search`] ranks one list of candidates. The launcher's root list is
//! several lists with different lifetimes and different rules -- the
//! application index, provider answers, and the secondary indexes this module
//! calls *asides* -- and the policy for putting them into one order lives here
//! rather than in the view that draws it.
//!
//! That placement is AD-03. The rules below (answers first, asides discounted
//! and capped, never browsed) are what make the launcher feel right, and they
//! are exactly the part that must survive the presentation layer being
//! replaced. Here they are also testable without a window, which is what
//! turns 7.26's worked example into an assertion.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use lumen_platform::Platform;
use serde::{Deserialize, Serialize};

use crate::favorites::{Change, Favorites};
use crate::frecency::Frecency;
use crate::item::Item;
use crate::paths::Paths;
use crate::providers::{self, Provider as _, Providers, QuicklinkProvider};
use crate::search::{Candidate, Search};

/// Writes a moment as the user's regional settings do (F13): the platform's
/// `format_date`, handed in by the launcher, since `core` makes no system
/// call of its own (I-1).
pub type TimeFormat = Arc<dyn Fn(&lumen_platform::LocalTime) -> String + Send + Sync>;

/// A secondary index: searched, but never browsed (7.21).
///
/// Files are large, change without the user doing anything to the launcher,
/// and do not belong in the root list. With an empty query the index is shown
/// in frecency order, so entries with no usage history would sort
/// alphabetically among the applications -- open the launcher, see a folder
/// called `Archive`. So an aside is matched only when there is a query, and
/// capped so that a common letter cannot fill the list with one kind of thing.
///
/// The clipboard was the other aside until F12 gave it a view of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aside {
    Files,
}

impl Aside {
    pub const ALL: [Aside; 1] = [Aside::Files];

    fn slot(self) -> usize {
        match self {
            Aside::Files => 0,
        }
    }

    /// How many of this aside's rows may appear in one set of results.
    ///
    /// Eight is enough to find the file you were just editing, and few enough that the list is still about applications.
    pub fn cap(self) -> usize {
        match self {
            Aside::Files => 8,
        }
    }

    /// What fraction of its earned score an aside row keeps, in eighths (7.26).
    ///
    /// Typing `co` matches `Command Prompt` and a file called `COMPLETE.png`
    /// about equally well, and the launcher is a launcher: the application
    /// should win. Typing `complete` is no longer ambiguous, the file scores far
    /// higher than anything else, and the discount does not change the outcome.
    ///
    /// Six eighths, because that is enough to break a near-tie and not enough
    /// to bury a clear match. Applied to the score rather than by sorting
    /// asides after applications, which would bury them permanently.
    pub fn weight(self) -> u32 {
        match self {
            Aside::Files => 6,
        }
    }
}

/// Which list a hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An index into the application index.
    Indexed(usize),
    /// An aside, and a row within it.
    Aside(Aside, usize),
    /// An index into this query's provider answers.
    Answer(usize),
    /// An index into this query's fallbacks, which always close the list.
    Fallback(usize),
}

/// What a section of the empty list can show (F15).
///
/// A closed set that grows in code, as the style's variants do: a name this
/// build does not know is reported and skipped, so a configuration written
/// for a later Lumen still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shows {
    /// The favourites, in the user's order.
    Favorites,
    /// Everything in the index that is not an application: Lumen's own
    /// commands, scripts, quicklinks, presets, extensions' commands.
    Commands,
    Applications,
}

impl Shows {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "favorites" => Shows::Favorites,
            "commands" => Shows::Commands,
            "applications" => Shows::Applications,
            _ => return None,
        })
    }

    /// Whether an item of `kind` belongs here. Favourites are claimed by id,
    /// not by kind, so they match nothing by kind.
    fn takes(self, kind: crate::item::Kind) -> bool {
        use crate::item::Kind;
        match self {
            Shows::Favorites => false,
            Shows::Commands => kind != Kind::Application,
            Shows::Applications => kind == Kind::Application,
        }
    }
}

/// One section of the empty list, as `config.json` writes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SectionConfig {
    pub title: String,
    /// What it shows, by name: `favorites`, `commands`, `applications`.
    /// Strings rather than an enum here, so one unknown name drops itself and
    /// not the whole configuration.
    pub shows: Vec<String>,
    /// Whether its header is drawn. Off, the section still orders the list.
    #[serde(default = "yes")]
    pub header: bool,
}

fn yes() -> bool {
    true
}

/// How the root list is arranged (F15): the sections of the empty list, in
/// order, and whether the typed list has its header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RootConfig {
    pub sections: Vec<SectionConfig>,
    pub results_header: bool,
}

impl Default for RootConfig {
    /// Raycast's: favourites, then commands, then applications.
    fn default() -> Self {
        let section = |title: &str, shows: &str| SectionConfig {
            title: title.into(),
            shows: vec![shows.into()],
            header: true,
        };
        Self {
            sections: vec![
                section("Favorites", "favorites"),
                section("Commands", "commands"),
                section("Applications", "applications"),
            ],
            results_header: true,
        }
    }
}

/// A run of hits under one header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// Where it starts in [`RootSearch::hits`], and how many it holds.
    pub start: usize,
    pub len: usize,
    /// Whether the header is drawn. A section without one still groups.
    pub header: bool,
}

/// The title of the one section a typed query is shown in.
pub const RESULTS: &str = "Results";

/// One result, held as an index into its list rather than a copy of it.
///
/// Re-ranking happens on every keystroke; cloning several hundred items each
/// time would spend the per-frame budget on allocation alone.
#[derive(Debug, Clone)]
pub struct Hit {
    pub source: Source,
    /// Character offsets into the title that matched, for highlighting.
    pub highlights: Vec<u32>,
    pub score: u32,
}

/// Everything the root list is ranked from, and the ranking of the last query.
///
/// Several lists rather than one because they have different lifetimes. The
/// application index is built once and reused for thousands of queries;
/// provider answers are rebuilt for every keystroke; the asides change when the
/// user copies something or saves a file. Concatenating them per keystroke
/// would mean cloning several hundred prepared haystacks to add three rows.
pub struct RootSearch {
    search: Search,
    providers: Providers,
    frecency: Frecency,
    /// The user's favourites, in their order (F15).
    favorites: Favorites,
    /// The arrangement from `config.json` (F15).
    arrangement: RootConfig,
    /// The empty list, built once and kept until something it is built from
    /// changes: the index, the frecency, the favourites or the arrangement.
    /// It depends on no query, so opening the launcher ranks nothing.
    home: Option<(Vec<Hit>, Vec<Section>)>,
    /// The sections of the current hits.
    sections: Vec<Section>,
    /// The searchable index. Empty until the first enumeration completes,
    /// which is why the root list is allowed to be empty rather than treated
    /// as a bug.
    index: Arc<Vec<Candidate>>,
    /// In [`Aside::slot`] order.
    asides: [Arc<Vec<Candidate>>; 1],
    /// This query's provider answers.
    answers: Vec<Item>,
    /// The quicklinks, held here rather than among the providers because they
    /// are the one provider the user edits while the daemon runs (F6).
    quicklinks: QuicklinkProvider,
    /// The timer presets, for their names and aliases (F8): typed exactly,
    /// one leads the matches, as a quicklink does.
    timer_presets: Vec<crate::timers::Preset>,
    /// The last table of exchange rates the daemon loaded, for `10 usd in
    /// cad` (F9) and any calculation in money (F14). Read, never fetched: I-3.
    rates: Option<crate::currency::Rates>,
    /// Whether the last query converted one currency into another, answered
    /// or not.
    conversion: bool,
    /// Whether the last table asked for could not be fetched, until one is.
    rates_failed: bool,
    /// How the platform writes a time, for `time in vancouver` (F13): as
    /// the user's regional settings do. Two digits each until it is set.
    time_format: Option<TimeFormat>,
    /// Whether the last query asked about time or the weather, answered or
    /// not: either needs the places.
    asks_places: bool,
    /// The forecasts the daemon last kept, for `weather in vancouver`
    /// (F13). Read, never fetched: I-3.
    weather: crate::weather::Cache,
    weather_units: crate::weather::Units,
    /// Why a city's forecast could not be fetched, by its key, until one is.
    weather_failed: std::collections::HashMap<String, crate::weather::Failure>,
    /// The city the last query wants a forecast fetched for.
    wants_weather: Option<u32>,
    /// This query's fallbacks.
    fallbacks: Vec<Item>,
    /// How many rows the window shows, so the fallbacks can be kept in view.
    visible_rows: usize,
    hits: Vec<Hit>,
}

impl RootSearch {
    pub fn new(providers: Providers, frecency: Frecency) -> Self {
        Self {
            search: Search::new(),
            providers,
            frecency,
            favorites: Favorites::new(),
            arrangement: RootConfig::default(),
            home: None,
            sections: Vec::new(),
            index: Arc::new(Vec::new()),
            asides: std::array::from_fn(|_| Arc::new(Vec::new())),
            answers: Vec::new(),
            quicklinks: QuicklinkProvider::default(),
            timer_presets: Vec::new(),
            rates: None,
            conversion: false,
            rates_failed: false,
            time_format: None,
            asks_places: false,
            weather: crate::weather::Cache::default(),
            weather_units: crate::weather::Units::default(),
            weather_failed: std::collections::HashMap::new(),
            wants_weather: None,
            fallbacks: Vec::new(),
            // What the configuration defaults to; the launcher sets the real
            // value before the first query.
            visible_rows: 8,
            hits: Vec::new(),
        }
    }

    /// How many rows the window shows without scrolling.
    pub fn set_visible_rows(&mut self, rows: usize) {
        self.visible_rows = rows.max(1);
    }

    /// The application index, shared so a rebuild can compare against it off
    /// the UI thread.
    pub fn index(&self) -> Arc<Vec<Candidate>> {
        self.index.clone()
    }

    pub fn replace_index(&mut self, index: Vec<Candidate>) {
        self.home = None;
        self.index = Arc::new(index);
    }

    pub fn replace_aside(&mut self, aside: Aside, candidates: Vec<Candidate>) {
        self.asides[aside.slot()] = Arc::new(candidates);
    }

    /// Take the quicklinks as they now are in the file.
    ///
    /// Only their answers and fallbacks: as catalogue entries they are part of
    /// the index, and arrive with it. Returns whether anything changed, for
    /// the same reason [`same_index`] exists.
    pub fn replace_quicklinks(&mut self, quicklinks: QuicklinkProvider) -> bool {
        if self.quicklinks == quicklinks {
            return false;
        }
        self.quicklinks = quicklinks;
        true
    }

    /// Take the timer presets as they now are in `config.json`. Returns
    /// whether anything changed.
    pub fn replace_timer_presets(&mut self, presets: Vec<crate::timers::Preset>) -> bool {
        if self.timer_presets == presets {
            return false;
        }
        self.timer_presets = presets;
        true
    }

    /// Take the table of exchange rates the daemon now holds. Returns whether
    /// anything changed.
    pub fn replace_rates(&mut self, rates: Option<crate::currency::Rates>) -> bool {
        if self.rates == rates {
            return false;
        }
        if rates.is_some() {
            self.rates_failed = false;
        }
        self.rates = rates;
        true
    }

    /// Whether the table being asked for could not be fetched (F14). Set
    /// when a fetch fails, and cleared when the next is asked for, so the
    /// row says *Fetching* again while it is out. Returns whether anything
    /// changed.
    pub fn set_rates_failed(&mut self, failed: bool) -> bool {
        std::mem::replace(&mut self.rates_failed, failed) != failed
    }

    /// Whether the last query asked for a conversion that needs a table this
    /// has not got, or one due for replacing by `now` (seconds since the
    /// epoch): the launcher asks the daemon for one, and does not wait.
    pub fn wants_rates(&self, now: impl FnOnce() -> i64) -> bool {
        self.conversion && self.rates.as_ref().is_none_or(|rates| rates.is_due(now()))
    }

    /// How times in answers are written (F13).
    pub fn set_time_format(&mut self, format: TimeFormat) {
        self.time_format = Some(format);
    }

    /// Whether the last query asked about time and the places have not been
    /// read: the launcher has them read off the UI thread, and ranks again
    /// when they are (F13). Like [`Self::wants_rates`], it does not wait.
    pub fn wants_places(&self) -> bool {
        self.asks_places && crate::places::loaded().is_none()
    }

    /// Take the forecasts the daemon now keeps (F13). Returns whether
    /// anything changed. A forecast that arrived clears its failure.
    pub fn replace_weather(&mut self, cache: crate::weather::Cache) -> bool {
        if self.weather == cache {
            return false;
        }
        self.weather_failed
            .retain(|key, _| !cache.forecasts.contains_key(key));
        self.weather = cache;
        true
    }

    /// A forecast could not be fetched for the city keyed `key`.
    pub fn weather_failed(&mut self, key: String, failure: crate::weather::Failure) {
        self.weather_failed.insert(key, failure);
    }

    /// The units the answers are written in. Returns whether they changed.
    pub fn set_weather_units(&mut self, units: crate::weather::Units) -> bool {
        std::mem::replace(&mut self.weather_units, units) != units
    }

    /// The city the last query asked the weather of, when its forecast is
    /// missing or old and has not failed: the launcher has the daemon fetch
    /// it, and does not wait.
    pub fn wants_weather(&self) -> Option<u32> {
        self.wants_weather
    }

    pub fn aside_is_empty(&self, aside: Aside) -> bool {
        self.asides[aside.slot()].is_empty()
    }

    /// Rank everything against `query`, replacing the previous hits.
    ///
    /// Answers first and unmatched, then the index and the asides merged by
    /// score. Runs on the UI thread on every keystroke; nothing here waits on
    /// anything (I-3).
    pub fn rank(&mut self, query: &str) -> &[Hit] {
        // Nothing typed: the sections, from the cache. Nothing answers an
        // empty query and nothing falls back from one, so the list is the
        // index alone, arranged.
        if query.trim().is_empty() {
            self.answers.clear();
            self.fallbacks.clear();
            self.conversion = false;
            self.asks_places = false;
            self.wants_weather = None;
            let (hits, sections) = match &self.home {
                Some(home) => home.clone(),
                None => {
                    let home = self.arrange();
                    self.home = Some(home.clone());
                    home
                }
            };
            self.hits = hits;
            self.sections = sections;
            return &self.hits;
        }

        // Providers are contractually instant (see `crate::providers`).
        self.answers = self.providers.answers(query);
        let trimmed = query.trim();
        if !trimmed.is_empty() {
            self.answers.extend(self.quicklinks.answer(trimmed));
        }
        // Money is the calculator's (F14), answered here because the table
        // is: the provider answers everything else, and declines money.
        let context = crate::calc::Context {
            today: &crate::calc::today,
            rates: self.rates.as_ref(),
            separators: crate::calc::separators(),
        };
        let money = crate::calc::answer(trimmed, &context).filter(|answer| answer.money);
        self.conversion = money.is_some_and(|answer| answer.converts);
        if let Some(answer) = money {
            let standing = providers::calculator::Standing {
                due: self
                    .rates
                    .as_ref()
                    .is_some_and(|rates| rates.is_due(crate::currency::now())),
                failed: self.rates_failed,
            };
            self.answers.push(providers::calculator::money(
                trimmed,
                &answer,
                self.rates.as_ref(),
                standing,
            ));
        }
        // A question about time, from the places in memory or not at all
        // (F13): their reading is the launcher's to start, off this thread.
        let time = crate::clock::query::recognise(trimmed);
        let weather = crate::weather::query::recognise(trimmed);
        self.asks_places = time.is_some() || weather.is_some();
        self.wants_weather = None;
        if let (Some(place), Some(places)) = (weather, crate::places::loaded())
            && let Some(city) = places.named_exactly(place)
            && let Some(key) = places.city(city).map(|found| places.key(found))
        {
            use providers::weather::Standing;
            let now = crate::clock::now().as_second();
            let kept = self.weather.get(&key);
            let failed = self.weather_failed.get(&key);
            let standing = match (kept, failed) {
                (Some(forecast), _) => Standing::Kept(forecast),
                (None, Some(failure)) => Standing::Failed(failure),
                (None, None) => Standing::Loading,
            };
            if failed.is_none() && kept.is_none_or(|forecast| !forecast.is_fresh(now)) {
                self.wants_weather = Some(city);
            }
            self.answers.extend(providers::weather::answer(
                places,
                city,
                standing,
                self.weather_units,
                now,
            ));
        }
        if let (Some(request), Some(places)) = (time, crate::places::loaded()) {
            let format = self.time_format.clone();
            let write = move |when: &lumen_platform::LocalTime| match &format {
                Some(format) => format(when),
                None => format!("{:02}:{:02}", when.hour, when.minute),
            };
            self.answers.extend(providers::clock::answer(
                &request,
                places,
                &crate::clock::here(),
                crate::clock::now(),
                &write,
            ));
        }
        self.fallbacks = self.quicklinks.fallbacks(query);

        // A typed URL: certain ones lead with the answers, probable ones wait
        // above the web search fallback. See `providers::web`.
        if let Some(typed) = providers::web::recognise(query)
            && let Some(item) = providers::web::item(&typed)
        {
            if typed.certain {
                self.answers.insert(0, item);
            } else {
                self.fallbacks.insert(0, item);
            }
        }

        let mut hits: Vec<Hit> = self
            .search
            // Every match, not the best 64 (F15): the matcher scores and
            // sorts every candidate before any cap could apply, so a cap
            // saved nothing measurable, and it cut rows a person could
            // scroll to.
            .rank(query, &self.index, &self.frecency, usize::MAX)
            .into_iter()
            .map(|m| Hit {
                source: Source::Indexed(m.index),
                highlights: m.highlights,
                score: m.score,
            })
            .collect();

        if !query.trim().is_empty() {
            for aside in Aside::ALL {
                let candidates = &self.asides[aside.slot()];
                if candidates.is_empty() {
                    continue;
                }

                hits.extend(
                    self.search
                        .rank(query, candidates, &self.frecency, aside.cap())
                        .into_iter()
                        .map(|m| Hit {
                            source: Source::Aside(aside, m.index),
                            highlights: m.highlights,
                            score: m.score / 8 * aside.weight(),
                        }),
                );
            }
        }

        // Sorted before the answers are prepended, so the answers keep the top
        // regardless of how well anything else scored.
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| self.item(a).title.cmp(&self.item(b).title))
        });

        // Likewise a timer preset (F8). Its alias is only a keyword to the
        // matcher, and a short one like `dw` scores below any file that
        // happens to contain the letters -- found, but seventh.
        let query_trimmed = query.trim();
        if let Some(preset) = self.timer_presets.iter().find(|preset| {
            crate::timers::presets::same(&preset.name, query_trimmed)
                || preset
                    .alias
                    .as_deref()
                    .is_some_and(|alias| crate::timers::presets::same(alias, query_trimmed))
        }) {
            let id = format!("timer:preset:{}", preset.name);
            self.lead(&mut hits, &id);
        }

        // A quicklink typed exactly -- its whole name, or its alias -- is what
        // the user meant, and leads the matches. Scoring alone lets usage
        // put a longer name first: `google` opened Google Drive (F6).
        if let Some(id) = self.quicklinks.exact(query) {
            self.lead(&mut hits, &id);
        }

        // An answer *is* what the user asked for -- it was computed from their
        // exact query -- so it goes above every fuzzy guess rather than
        // competing with them on a score it has no meaningful way to earn.
        let answers = (0..self.answers.len()).map(|index| Hit {
            source: Source::Answer(index),
            highlights: Vec::new(),
            score: u32::MAX,
        });
        hits.splice(0..0, answers);

        // A fallback is for when nothing above it was what the user meant, so
        // it comes after the matches -- but no later than the last row of the
        // first screen. Sixteen loose clipboard matches would otherwise bury
        // it below the window, which is where nobody looks for it. Never above
        // an answer, which is what the user asked for by construction.
        let at = hits
            .len()
            .min(self.visible_rows.saturating_sub(self.fallbacks.len()))
            .max(self.answers.len().min(hits.len()));
        hits.splice(
            at..at,
            (0..self.fallbacks.len()).map(|index| Hit {
                source: Source::Fallback(index),
                highlights: Vec::new(),
                score: 0,
            }),
        );

        self.sections = if hits.is_empty() {
            Vec::new()
        } else {
            vec![Section {
                title: RESULTS.into(),
                start: 0,
                len: hits.len(),
                header: self.arrangement.results_header,
            }]
        };
        self.hits = hits;
        &self.hits
    }

    /// The empty list: every section in order, each item once, in the first
    /// section that claims it.
    ///
    /// Favourites keep the user's order; everything else keeps the order the
    /// empty query always had, frecency then title. A section that shows
    /// favourites and something else lists the favourites first. A section
    /// that claims nothing is left out, header and all. Nothing is capped:
    /// every application and command is browsable.
    fn arrange(&mut self) -> (Vec<Hit>, Vec<Section>) {
        let ordered = self
            .search
            .rank("", &self.index, &self.frecency, usize::MAX);
        let at: std::collections::HashMap<&str, usize> = self
            .index
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.item.id.as_str(), index))
            .collect();
        let mut claimed = vec![false; self.index.len()];
        let mut hits = Vec::new();
        let mut sections = Vec::new();

        for config in &self.arrangement.sections {
            let shows: Vec<Shows> = config
                .shows
                .iter()
                .filter_map(|name| {
                    let shows = Shows::parse(name);
                    if shows.is_none() {
                        log::warn!(
                            "section {:?} shows {name:?}, which this build does not know; skipping it",
                            config.title
                        );
                    }
                    shows
                })
                .collect();
            let start = hits.len();

            if shows.contains(&Shows::Favorites) {
                for id in self.favorites.ids() {
                    if let Some(&index) = at.get(id.as_str())
                        && !claimed[index]
                    {
                        claimed[index] = true;
                        hits.push(Hit {
                            source: Source::Indexed(index),
                            highlights: Vec::new(),
                            score: u32::MAX,
                        });
                    }
                }
            }
            for found in &ordered {
                if !claimed[found.index] && shows.iter().any(|shows| shows.takes(found.item.kind)) {
                    claimed[found.index] = true;
                    hits.push(Hit {
                        source: Source::Indexed(found.index),
                        highlights: Vec::new(),
                        score: found.score,
                    });
                }
            }

            if hits.len() > start {
                sections.push(Section {
                    title: config.title.clone(),
                    start,
                    len: hits.len() - start,
                    header: config.header,
                });
            }
        }

        (hits, sections)
    }

    /// The sections of the current hits, in order, covering all of them.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Arrange the root list as `config.json` says. The empty list is built
    /// again the next time it is asked for.
    pub fn set_arrangement(&mut self, arrangement: RootConfig) {
        if arrangement != self.arrangement {
            self.arrangement = arrangement;
            self.home = None;
        }
    }

    /// Put the indexed item `id` first among `hits`.
    ///
    /// Taken from the index when the matcher did not return it. The matcher
    /// used to keep only its best 64, and a two-letter alias such as `io`
    /// matches dozens of names better than the keyword it is -- so the row it
    /// names was cut before it could be promoted (F8, found by using it).
    /// Nothing is cut now (F15); this stays, because a row that leads must
    /// not depend on the matcher having returned it.
    fn lead(&self, hits: &mut Vec<Hit>, id: &str) {
        if let Some(at) = hits.iter().position(|hit| self.item(hit).id == id) {
            let hit = hits.remove(at);
            hits.insert(0, hit);
            return;
        }
        if let Some(index) = self
            .index
            .iter()
            .position(|candidate| candidate.item.id == id)
        {
            hits.insert(
                0,
                Hit {
                    source: Source::Indexed(index),
                    highlights: Vec::new(),
                    score: u32::MAX,
                },
            );
        }
    }

    /// The quicklink `query` names exactly, for Tab (F6).
    pub fn named_link(&self, query: &str) -> Option<providers::quicklinks::ArgumentRequest> {
        self.quicklinks.named(query)
    }

    /// Show only the row for quicklink `id`, filled with `argument` (F6).
    ///
    /// What the list is while the launcher asks for a link's argument: the
    /// user has already chosen the link, and anything else matching the text
    /// they type is noise. Empty once the link is gone from the file.
    pub fn rank_argument(&mut self, id: &str, argument: &str) -> &[Hit] {
        self.answers = self
            .quicklinks
            .argument_item(id, argument)
            .into_iter()
            .collect();
        self.fallbacks.clear();
        // One row, the link being filled in: nothing to head.
        self.sections.clear();
        self.hits = (0..self.answers.len())
            .map(|index| Hit {
                source: Source::Answer(index),
                highlights: Vec::new(),
                score: u32::MAX,
            })
            .collect();
        &self.hits
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub fn item(&self, hit: &Hit) -> &Item {
        match hit.source {
            Source::Indexed(index) => &self.index[index].item,
            Source::Aside(aside, index) => &self.asides[aside.slot()][index].item,
            Source::Answer(index) => &self.answers[index],
            Source::Fallback(index) => &self.fallbacks[index],
        }
    }

    /// Which of `candidates` match `query`, by position, with the same matcher
    /// and the same usage history as the root list.
    ///
    /// For a view the host filters (7.30): one notion of "matches", not two.
    pub fn filter(&mut self, query: &str, candidates: &[Candidate]) -> Vec<usize> {
        self.search
            .rank(query, candidates, &self.frecency, usize::MAX)
            .into_iter()
            .map(|matched| matched.index)
            .collect()
    }

    /// Record that `id` was used, and return the history to persist.
    ///
    /// Returned rather than written: the write is disk I/O and belongs on a
    /// background thread, which is the caller's to choose.
    pub fn record_use(&mut self, id: &str) -> Frecency {
        self.frecency.record(id);
        self.home = None;
        self.frecency.clone()
    }

    pub fn set_favorites(&mut self, favorites: Favorites) {
        self.favorites = favorites;
        self.home = None;
    }

    /// The ids of the favourites that are drawn, in the user's order: those
    /// whose item is in the index now.
    pub fn shown_favorites(&self) -> Vec<&str> {
        let present: std::collections::HashSet<&str> = self
            .index
            .iter()
            .map(|candidate| candidate.item.id.as_str())
            .collect();
        self.favorites
            .ids()
            .iter()
            .map(String::as_str)
            .filter(|id| present.contains(id))
            .collect()
    }

    /// What can be done to `hit` as a favourite. Nothing for a row that is
    /// not in the index -- an answer, a file, a fallback -- because none of
    /// those is there to come back to.
    pub fn favorite_changes(&self, hit: &Hit) -> Vec<Change> {
        if !matches!(hit.source, Source::Indexed(_)) {
            return Vec::new();
        }
        self.favorites
            .changes(&self.item(hit).id, &self.shown_favorites())
    }

    /// Apply `change` to the item `id`, returning the list to persist when
    /// anything changed.
    pub fn change_favorite(&mut self, change: Change, id: &str) -> Option<Favorites> {
        let shown: Vec<String> = self
            .shown_favorites()
            .into_iter()
            .map(str::to_string)
            .collect();
        let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
        let changed = self.favorites.apply(change, id, &shown);
        if changed {
            self.home = None;
        }
        changed.then(|| self.favorites.clone())
    }
}

/// Build the root index: applications plus every provider's catalogue.
///
/// Blocking and slow -- the application enumeration alone is hundreds of
/// milliseconds -- so it is called off the UI thread, and root search never
/// waits for it (I-3). Nothing is executed to build it: scripts are indexed by
/// their headers and extensions by their manifests (AD-08).
///
/// The quicklinks are passed in rather than read here, because the caller
/// needs them too, for [`RootSearch::replace_quicklinks`]. Reading the file
/// twice logged every broken entry twice. The translate and timer presets are
/// passed in because they come from `config.json`, which the caller already
/// holds.
pub fn build_index(
    platform: &dyn Platform,
    paths: &Paths,
    quicklinks: &[providers::Quicklink],
    presets: Vec<crate::translate::Preset>,
    timers: Vec<crate::timers::Preset>,
) -> Result<Vec<Candidate>> {
    let entries = platform.enumerate_applications()?;
    log::info!("indexed {} applications", entries.len());

    let catalogue = catalogue(
        &paths.scripts_dir(),
        quicklinks,
        &paths.extensions_dir(),
        presets,
        timers,
    );

    Ok(entries
        .into_iter()
        .map(Candidate::from_app)
        .chain(catalogue.into_iter().map(Candidate::new))
        .collect())
}

/// Every provider's catalogue, from the folders the user edits.
fn catalogue(
    scripts_dir: &Path,
    quicklinks: &[providers::Quicklink],
    extensions_dir: &Path,
    presets: Vec<crate::translate::Preset>,
    timers: Vec<crate::timers::Preset>,
) -> Vec<Item> {
    let mut providers = Providers::builtin();
    providers.register(Box::new(providers::TranslateProvider::new(presets)));
    providers.register(Box::new(providers::TimerPresetsProvider::new(timers)));

    let scripts = providers::scripts::scan(scripts_dir);
    if !scripts.is_empty() {
        log::info!("indexed {} script commands", scripts.len());
    }
    providers.register(Box::new(providers::ScriptProvider::new(scripts)));

    if !quicklinks.is_empty() {
        log::info!("indexed {} quicklinks", quicklinks.len());
    }
    providers.register(Box::new(QuicklinkProvider::new(quicklinks.to_vec())));

    let extensions = crate::extension::scan(extensions_dir);
    if !extensions.is_empty() {
        let provider = providers::ExtensionProvider::new(&extensions);
        log::info!(
            "indexed {} commands from {} extensions",
            provider.len(),
            extensions.len()
        );
        providers.register(Box::new(provider));
    }

    providers.catalogue()
}

/// Whether a rebuilt index lists exactly what the current one does.
///
/// Usually true: a re-index is asked for on every install the system reports,
/// and most of those touch nothing Lumen lists. Replacing an identical index
/// would re-rank and put the selection back on the first row under somebody
/// who had just moved it.
pub fn same_index(current: &[Candidate], rebuilt: &[Candidate]) -> bool {
    current.len() == rebuilt.len()
        && current
            .iter()
            .zip(rebuilt)
            .all(|(before, after)| before.item == after.item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Action, Kind};
    use lumen_platform::LaunchTarget;

    fn item(id: &str, title: &str, kind: Kind) -> Item {
        Item {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            icon: None,
            keywords: Vec::new(),
            actions: vec![Action::Launch(LaunchTarget::ShellExecute(title.into()))],
            kind,
        }
    }

    fn apps(titles: &[&str]) -> Vec<Candidate> {
        titles
            .iter()
            .map(|title| Candidate::new(item(title, title, Kind::Application)))
            .collect()
    }

    fn files(titles: &[&str]) -> Vec<Candidate> {
        titles
            .iter()
            .map(|title| Candidate::new(item(&format!("file:{title}"), title, Kind::File)))
            .collect()
    }

    fn root(index: Vec<Candidate>) -> RootSearch {
        let mut root = RootSearch::new(Providers::builtin(), Frecency::new());
        root.replace_index(index);
        root
    }

    fn titles(root: &RootSearch) -> Vec<String> {
        root.hits()
            .iter()
            .map(|hit| root.item(hit).title.clone())
            .collect()
    }

    /// Two applications and two commands, as the sections are tested on.
    fn mixed() -> RootSearch {
        let mut index = apps(&["Code", "Slack"]);
        index.extend(
            ["Clipboard History", "Search Snippets"]
                .map(|title| Candidate::new(item(title, title, Kind::Command))),
        );
        root(index)
    }

    fn sections(root: &RootSearch) -> Vec<(String, Vec<String>)> {
        root.sections()
            .iter()
            .map(|section| {
                let titles = root.hits()[section.start..section.start + section.len]
                    .iter()
                    .map(|hit| root.item(hit).title.clone())
                    .collect();
                (section.title.clone(), titles)
            })
            .collect()
    }

    fn owned(sections: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        sections
            .iter()
            .map(|(title, items)| {
                (
                    title.to_string(),
                    items.iter().map(|item| item.to_string()).collect(),
                )
            })
            .collect()
    }

    /// Raycast's arrangement, and no favourites yet: their section is not
    /// drawn at all, header included.
    #[test]
    fn the_empty_list_is_in_sections_and_an_empty_one_is_left_out() {
        let mut root = mixed();
        root.rank("");
        assert_eq!(
            sections(&root),
            owned(&[
                ("Commands", &["Clipboard History", "Search Snippets"]),
                ("Applications", &["Code", "Slack"]),
            ])
        );
    }

    /// A favourite is listed once, in the first section that claims it, and
    /// the favourites keep the user's order rather than the alphabet's.
    #[test]
    fn a_favourite_is_listed_once_in_the_users_order() {
        let mut root = mixed();
        root.change_favorite(Change::Add, "Slack");
        root.change_favorite(Change::Add, "Search Snippets");
        root.rank("");
        assert_eq!(
            sections(&root),
            owned(&[
                ("Favorites", &["Slack", "Search Snippets"]),
                ("Commands", &["Clipboard History"]),
                ("Applications", &["Code"]),
            ])
        );
    }

    /// Vicinae's arrangement: one section for both, in the order the empty
    /// list always had -- frecency, then the alphabet -- not by kind.
    #[test]
    fn a_section_can_show_commands_and_applications_together() {
        let mut root = mixed();
        root.set_arrangement(RootConfig {
            sections: vec![SectionConfig {
                title: "All".into(),
                shows: vec!["commands".into(), "applications".into()],
                header: false,
            }],
            results_header: true,
        });
        root.record_use("Slack");
        root.rank("");
        assert_eq!(
            sections(&root),
            owned(&[(
                "All",
                &["Slack", "Clipboard History", "Code", "Search Snippets"]
            )])
        );
        assert!(!root.sections()[0].header);
    }

    /// What no section claims is not browsed, and a name this build does not
    /// know drops itself, not the section.
    #[test]
    fn unclaimed_items_are_not_browsed_and_unknown_names_are_skipped() {
        let mut root = mixed();
        root.set_arrangement(RootConfig {
            sections: vec![SectionConfig {
                title: "Commands".into(),
                shows: vec!["commands".into(), "from-a-later-lumen".into()],
                header: true,
            }],
            results_header: true,
        });
        root.rank("");
        assert_eq!(
            sections(&root),
            owned(&[("Commands", &["Clipboard History", "Search Snippets"])])
        );
        // Still found by name.
        root.rank("slack");
        assert_eq!(titles(&root)[0], "Slack");
    }

    /// Typing collapses the sections into one ranked list (AD-03).
    #[test]
    fn a_typed_query_is_one_section_of_results() {
        let mut root = mixed();
        root.rank("s");
        assert_eq!(root.sections().len(), 1);
        assert_eq!(root.sections()[0].title, RESULTS);
        assert_eq!(root.sections()[0].len, root.hits().len());
        root.rank("qzxv");
        assert!(root.sections().is_empty());
    }

    /// The empty list is kept between summonses and built again only when
    /// something it is built from changes -- here, a use.
    #[test]
    fn the_empty_list_is_kept_until_what_it_is_built_from_changes() {
        let mut root = mixed();
        root.rank("");
        assert_eq!(titles(&root)[2], "Code");
        root.rank("");
        assert_eq!(titles(&root)[2], "Code");
        root.record_use("Slack");
        root.rank("");
        assert_eq!(titles(&root)[2], "Slack");
    }

    /// An application can be a favourite; an answer computed from the query
    /// cannot, because there is nothing to come back to (F15).
    #[test]
    fn only_rows_of_the_index_can_be_favourites() {
        let mut root = root(apps(&["Code", "Slack"]));
        root.rank("2+2");
        let answer = root.hits()[0].clone();
        assert!(matches!(answer.source, Source::Answer(_)));
        assert!(root.favorite_changes(&answer).is_empty());

        root.rank("code");
        let code = root.hits()[0].clone();
        assert_eq!(root.favorite_changes(&code), [Change::Add]);
        let id = root.item(&code).id.clone();
        assert!(root.change_favorite(Change::Add, &id).is_some());
        assert_eq!(root.shown_favorites(), [id.as_str()]);
        assert_eq!(root.favorite_changes(&code), [Change::Remove]);
    }

    /// 7.26, the case that motivated the discount: two letters match an
    /// application and a file about equally well, and the launcher is a
    /// launcher.
    #[test]
    fn an_application_wins_a_near_tie_with_an_aside() {
        let mut root = root(apps(&["Command Prompt"]));
        root.replace_aside(Aside::Files, files(&["COMPLETE.png"]));

        root.rank("co");

        assert_eq!(titles(&root)[0], "Command Prompt");
    }

    /// The other half of 7.26: the discount must not bury a clear match.
    #[test]
    fn a_clear_aside_match_still_comes_first() {
        let mut root = root(apps(&["Command Prompt"]));
        root.replace_aside(Aside::Files, files(&["COMPLETE.png"]));

        root.rank("complete");

        assert_eq!(titles(&root)[0], "COMPLETE.png");
    }

    /// 7.21: open the launcher, do not see what you copied last Tuesday.
    #[test]
    fn asides_are_never_browsed() {
        let mut root = root(apps(&["Firefox"]));
        root.replace_aside(Aside::Files, files(&["Archive"]));

        root.rank("");

        assert!(
            root.hits()
                .iter()
                .all(|hit| !matches!(hit.source, Source::Aside(..)))
        );
        assert!(titles(&root).contains(&"Firefox".to_string()));
    }

    #[test]
    fn an_aside_cannot_fill_the_list() {
        let many: Vec<String> = (0..40).map(|n| format!("report {n}.txt")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut root = root(Vec::new());
        root.replace_aside(Aside::Files, files(&many));

        root.rank("report");

        assert_eq!(root.hits().len(), Aside::Files.cap());
    }

    /// 7.9: an answer is not matched against the query, and it leads.
    #[test]
    fn an_answer_leads_whatever_else_matched() {
        let mut root = root(apps(&["12 Days Calculator"]));

        root.rank("12*7");

        let first = &root.hits()[0];
        assert!(matches!(first.source, Source::Answer(_)));
        assert_eq!(root.item(first).title, "84");
    }

    /// Every match is listed, however many (F15): the cap of 64 saved
    /// nothing measurable and hid rows that could be scrolled to.
    #[test]
    fn every_match_is_listed() {
        let many: Vec<String> = (0..200).map(|n| format!("App {n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut root = root(apps(&many));

        root.rank("app");

        assert_eq!(root.hits().len(), 200);
    }

    #[test]
    fn recording_a_use_moves_it_up_the_root_list() {
        let mut root = root(apps(&["Alpha", "Beta"]));

        root.record_use("Beta");
        root.rank("");

        assert_eq!(titles(&root)[0], "Beta");
    }

    #[test]
    fn filtering_uses_the_same_matcher() {
        let mut root = root(Vec::new());
        let view = apps(&["Tokyo", "Paris", "Toronto"]);

        let kept = root.filter("to", &view);

        assert!(kept.contains(&0) && kept.contains(&2));
        assert!(!kept.contains(&1));
    }

    fn google() -> QuicklinkProvider {
        let file: providers::quicklinks::QuicklinkFile = serde_json::from_str(
            r#"{ "schema_version": 1, "quicklinks": [{
                "name": "Google", "alias": "g", "fallback": true,
                "link": "https://www.google.com/search?q={query}"
            }] }"#,
        )
        .unwrap();
        QuicklinkProvider::new(
            file.quicklinks
                .into_iter()
                .map(|entry| providers::Quicklink::from_entry(entry).unwrap())
                .collect(),
        )
    }

    /// F6: the alias makes an answer, so it leads like the calculator's.
    #[test]
    fn a_quicklink_alias_leads_the_list() {
        let mut root = root(apps(&["Gimp"]));
        root.replace_quicklinks(google());

        root.rank("g lofi");

        assert_eq!(titles(&root)[0], "Search Google for \u{201c}lofi\u{201d}");
    }

    /// Typing a link's whole name, or its alias, means that link -- even
    /// against an application with a longer name the user opens every day.
    #[test]
    fn a_quicklink_named_exactly_leads_the_matches() {
        let links = google();
        let mut index = apps(&["Google Drive"]);
        index.extend(links.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_quicklinks(links);
        for _ in 0..20 {
            root.record_use("Google Drive");
        }

        root.rank("google");
        assert_eq!(titles(&root)[0], "Google");

        // The alias alone, with no space and so no answer yet.
        root.rank("G");
        assert_eq!(titles(&root)[0], "Google");
    }

    /// F9: a conversion is answered first, from the table in hand; without
    /// one it is only recognised, so the launcher can ask for a table. Since
    /// F14 the row holds its place meanwhile, and says why it is empty.
    #[test]
    fn a_conversion_is_answered_from_the_table_or_asks_for_one() {
        let mut root = root(apps(&["Calculator"]));

        root.rank("10 usd in cad");
        assert!(titles(&root).iter().all(|title| !title.ends_with("CAD")));
        assert_eq!(titles(&root)[0], "Fetching exchange rates\u{2026}");
        assert!(root.wants_rates(|| 0));

        assert!(root.set_rates_failed(true));
        root.rank("10 usd in cad");
        assert_eq!(titles(&root)[0], "Couldn\u{2019}t fetch exchange rates");
        assert!(root.set_rates_failed(false), "asked again: fetching again");

        let rates = crate::currency::tests::rates(&[("USD", 1.0), ("CAD", 1.4)]);
        let fresh = rates.next - 1;
        assert!(root.replace_rates(Some(rates.clone())));
        assert!(
            !root.replace_rates(Some(rates)),
            "the same table is no change"
        );

        root.rank("10 usd in cad");
        assert_eq!(titles(&root)[0], "14.00 CAD");
        assert!(!root.wants_rates(|| fresh));
        assert!(root.wants_rates(|| fresh + 1), "a due table is replaced");

        root.rank("calc");
        assert!(!root.wants_rates(|| 0), "not a conversion");
    }

    /// A clock reading followed by a place is two questions: a timer named
    /// after the place (F8), and the time there (F13). Both are offered, and
    /// the arrows choose -- intended, not an overlap to settle (F14).
    #[test]
    fn a_clock_reading_and_a_place_offer_a_timer_and_the_time() {
        let mut root = root(apps(&["Notepad"]));
        crate::places::catalogue();
        root.set_time_format(Arc::new(|_| "TIME".into()));

        root.rank("15:00 vancouver in tokyo");
        let kinds: Vec<Kind> = root.hits().iter().map(|hit| root.item(hit).kind).collect();
        assert!(kinds.contains(&Kind::Timer), "{kinds:?}");
        assert!(kinds.contains(&Kind::Clock), "{kinds:?}");
    }

    /// F13: a question about time is answered first, from the places in
    /// memory, in the platform's way of writing a time.
    #[test]
    fn a_question_about_time_leads_once_the_places_are_read() {
        let mut root = root(apps(&["Timer Resolution"]));
        crate::places::catalogue();
        root.set_time_format(Arc::new(|_| "TIME".into()));

        root.rank("time in vancouver");
        assert_eq!(titles(&root)[0], "TIME in Vancouver");
        assert!(!root.wants_places(), "read already");

        root.rank("3pm vancouver");
        assert_eq!(titles(&root)[0], "TIME your time");

        root.rank("timer");
        assert!(titles(&root).iter().all(|title| !title.starts_with("TIME")));
    }

    /// F13: the weather is answered from the forecasts in memory, and a
    /// missing one is asked for, once named whole, without waiting.
    #[test]
    fn the_weather_is_answered_or_asked_for() {
        let mut root = root(apps(&["Weatherbug"]));
        crate::places::catalogue();

        root.rank("weather in vancouv");
        assert_eq!(root.wants_weather(), None, "not named whole yet");
        assert!(
            titles(&root)
                .iter()
                .all(|title| !title.contains("Vancouver"))
        );

        root.rank("weather in vancouver");
        let vancouver = root.wants_weather().expect("asked for");
        assert_eq!(titles(&root)[0], "Weather in Vancouver");

        let places = crate::places::catalogue();
        let key = places.key(places.city(vancouver).unwrap());
        let mut cache = crate::weather::Cache::default();
        cache.insert(
            key.clone(),
            crate::weather::tests::forecast(12.0, 3, crate::clock::now().as_second()),
        );
        assert!(root.replace_weather(cache));
        root.rank("vancouver weather");
        assert_eq!(titles(&root)[0], "12\u{b0} Overcast in Vancouver");
        assert_eq!(root.wants_weather(), None, "fresh");

        root.rank("weather in seoul");
        let seoul = root.wants_weather().unwrap();
        let seoul_key = places.key(places.city(seoul).unwrap());
        root.weather_failed(seoul_key, crate::weather::Failure::Offline);
        root.rank("weather in seoul");
        assert_eq!(root.wants_weather(), None, "not retried on every key");
    }

    /// F8, found by using it: a preset's alias found it seventh, below files
    /// whose names happen to hold the letters, and its name second.
    #[test]
    fn a_timer_preset_named_exactly_leads_the_matches() {
        use crate::providers::Provider as _;

        let preset = crate::timers::Preset {
            name: "Zeta".into(),
            alias: Some("zq".into()),
            duration: std::time::Duration::from_secs(420),
            cycle: None,
            focus: false,
        };
        let presets = providers::TimerPresetsProvider::new(vec![preset.clone()]);
        let mut index = apps(&["Zed Quick", "Modrinth App"]);
        index.extend(presets.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_aside(
            Aside::Files,
            files(&["ZedQ2016.fantome", "Zeta_backup.zip"]),
        );
        root.replace_timer_presets(vec![preset]);

        root.rank("zq");
        assert_eq!(titles(&root)[0], "Zeta");
        root.rank("ZETA");
        assert_eq!(titles(&root)[0], "Zeta");
        // A prefix still competes on score.
        root.rank("zet");
        assert!(titles(&root).contains(&"Zeta".to_string()));
    }

    /// F8, found by using it: `io` matches far more than 64 names better
    /// than a keyword, so the preset it names was cut before it could lead.
    #[test]
    fn a_short_alias_leads_even_when_it_matches_everything() {
        use crate::providers::Provider as _;

        let preset = crate::timers::Preset {
            name: "preset test 2".into(),
            alias: Some("io".into()),
            duration: std::time::Duration::from_secs(60),
            cycle: None,
            focus: false,
        };
        let presets = providers::TimerPresetsProvider::new(vec![preset.clone()]);
        let many: Vec<String> = (0..200).map(|n| format!("Studio {n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut index = apps(&many);
        index.extend(presets.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_timer_presets(vec![preset]);

        root.rank("io");

        assert_eq!(titles(&root)[0], "preset test 2");
    }

    /// Only an exact name: a prefix is still a fuzzy match like any other.
    #[test]
    fn a_partial_quicklink_name_competes_on_score() {
        let links = google();
        let mut index = apps(&["Google Drive"]);
        index.extend(links.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_quicklinks(links);
        for _ in 0..20 {
            root.record_use("Google Drive");
        }

        root.rank("goog");
        assert_eq!(titles(&root)[0], "Google Drive");
    }

    /// A URL beyond doubt leads, above every match.
    #[test]
    fn a_typed_url_leads_the_list() {
        let mut root = root(apps(&["GitHub Desktop"]));
        root.rank("github.com");
        assert_eq!(titles(&root)[0], "Open github.com");
    }

    /// `readme.md` is a file first: its URL row waits at the foot, above the
    /// web search fallback.
    #[test]
    fn a_file_like_url_waits_below_the_matches() {
        let mut root = root(files(&["readme.md"]));
        root.replace_quicklinks(google());
        root.rank("readme.md");

        let titles = titles(&root);
        assert_eq!(titles[0], "readme.md");
        let url = titles.iter().position(|t| t == "Open readme.md").unwrap();
        let search = titles
            .iter()
            .position(|t| t.starts_with("Search Google"))
            .unwrap();
        assert_eq!(url + 1, search);
    }

    #[test]
    fn a_fallback_closes_the_list() {
        let mut root = root(apps(&["Firefox", "Files"]));
        root.replace_quicklinks(google());

        root.rank("fi");

        let hits = root.hits();
        assert!(matches!(hits.last().unwrap().source, Source::Fallback(_)));
        assert!(matches!(hits[0].source, Source::Indexed(_)));
    }

    /// Found by using it: sixteen loose clipboard matches put the fallback at
    /// row seventeen, under a window eight rows tall. A fallback nobody can
    /// see is not a fallback, so it takes the last row of the first screen.
    #[test]
    fn a_long_list_keeps_the_fallback_on_the_first_screen() {
        let many: Vec<String> = (0..200).map(|n| format!("App {n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut root = root(apps(&many));
        root.replace_quicklinks(google());
        root.set_visible_rows(8);

        root.rank("app");

        // Two hundred matches and the fallback.
        assert_eq!(root.hits().len(), 201);
        assert!(matches!(root.hits()[7].source, Source::Fallback(_)));
        assert!(matches!(root.hits()[8].source, Source::Indexed(_)));
    }

    #[test]
    fn an_answer_still_leads_a_list_with_a_fallback() {
        let mut root = root(apps(&["Everest Notes"]));
        root.replace_quicklinks(google());
        root.set_visible_rows(1);

        root.rank("8848*1");

        assert!(matches!(root.hits()[0].source, Source::Answer(_)));
        assert!(matches!(root.hits()[1].source, Source::Fallback(_)));
    }

    #[test]
    fn asking_for_an_argument_shows_only_that_link() {
        let mut root = root(apps(&["Lofi Player"]));
        root.replace_quicklinks(google());

        root.rank_argument("quicklink:Google", "lofi");

        assert_eq!(
            titles(&root),
            vec!["Search Google for \u{201c}lofi\u{201d}".to_string()]
        );

        root.rank_argument("quicklink:Gone", "lofi");
        assert!(root.hits().is_empty());
    }

    #[test]
    fn no_fallback_while_browsing() {
        let mut root = root(apps(&["Firefox"]));
        root.replace_quicklinks(google());

        root.rank("");

        assert!(
            root.hits()
                .iter()
                .all(|hit| !matches!(hit.source, Source::Fallback(_)))
        );
    }

    #[test]
    fn an_identical_rebuild_is_recognised() {
        assert!(same_index(&apps(&["A", "B"]), &apps(&["A", "B"])));
        assert!(!same_index(&apps(&["A", "B"]), &apps(&["A", "C"])));
        assert!(!same_index(&apps(&["A"]), &apps(&["A", "B"])));
    }
}
