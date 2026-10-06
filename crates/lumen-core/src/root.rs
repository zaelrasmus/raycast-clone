//! The root list: what is searched, and how the pieces are ranked together.
//!
//! [`crate::search`] ranks one list of candidates. The launcher's root list is
//! several lists with different lifetimes and different rules -- the
//! application index, provider answers, and the rows the file index sends
//! when it answers -- and the policy for putting them into one order lives
//! here rather than in the view that draws it.
//!
//! That placement is AD-03. The rules below (answers first, files in a
//! section of their own after the results, capped and never browsed) are
//! what make the launcher feel right, and they are exactly the part that must
//! survive the presentation layer being replaced. Here they are also testable
//! without a window.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use lumen_platform::Platform;
use serde::{Deserialize, Serialize};

use crate::aliases::Aliases;
use crate::favorites::{Change, Favorites};
use crate::file_index::{self, FileHit, names};
use crate::file_search::{Found, browse};
use crate::frecency::Frecency;
use crate::item::Item;
use crate::paths::Paths;
use crate::providers::{self, Provider as _, Providers, QuicklinkProvider};
use crate::search::{Candidate, Search};

/// Writes a moment as the user's regional settings do (F13): the platform's
/// `format_date`, handed in by the launcher, since `core` makes no system
/// call of its own (I-1).
pub type TimeFormat = Arc<dyn Fn(&lumen_platform::LocalTime) -> String + Send + Sync>;

/// Which list a hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An index into the application index.
    Indexed(usize),
    /// A file or folder the file index found, by its place in what it
    /// last answered (F16).
    File(usize),
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

/// The title of the section the file index's rows are shown in (F16).
pub const FILES: &str = "Files";

/// How many of the file index's rows one query shows: 7.21's cap, kept.
///
/// Eight is enough to find the file you were just editing, and few enough
/// that the list is still about applications. The *Search Files* view is
/// where there is no cap (F16).
pub const FILES_CAP: usize = 8;

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
/// provider answers are rebuilt for every keystroke; the files arrive from
/// another thread when the file index answers. Concatenating them per
/// keystroke would mean cloning several hundred prepared haystacks to add
/// three rows.
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
    /// This query's provider answers.
    answers: Vec<Item>,
    /// What the file index last answered, and the query it answered (F16).
    /// Matched there, off the UI thread, and only placed here.
    files: Vec<FileHit>,
    files_query: String,
    /// Whether `files` is a typed path's listing rather than the index's
    /// answer: it answers only that path, whole and in its own order.
    files_typed: bool,
    /// That listing as it was given, row for row with `files`: what is
    /// narrowed while the same folder's next listing is read, and what a
    /// row is completed to.
    listing: Vec<Found>,
    /// The user's home folder, for `~\` in a typed path and the rows'
    /// subtitles: found as the file index finds it, so the two agree.
    user_home: Option<PathBuf>,
    /// The quicklinks, held here rather than among the providers because they
    /// are the one provider the user edits while the daemon runs (F6).
    quicklinks: QuicklinkProvider,
    /// The timer presets, for their names and aliases (F8): typed exactly,
    /// one leads the matches, as a quicklink does.
    timer_presets: Vec<crate::timers::Preset>,
    /// The translate presets, for their aliases (F17).
    translate_presets: Vec<crate::translate::Preset>,
    /// The user's aliases from `shortcuts.json`, `(item id, alias)` (F17).
    shortcut_aliases: Vec<(String, String)>,
    /// Every alias and leading name above, as one table: rebuilt when any
    /// of them changes, looked up on every keystroke (F17).
    aliases: Aliases,
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
            answers: Vec::new(),
            files: Vec::new(),
            files_query: String::new(),
            files_typed: false,
            listing: Vec::new(),
            user_home: directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
            quicklinks: QuicklinkProvider::default(),
            timer_presets: Vec::new(),
            translate_presets: Vec::new(),
            shortcut_aliases: Vec::new(),
            aliases: Aliases::default(),
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
        self.rebuild_aliases();
        true
    }

    /// Take the user's aliases from `shortcuts.json` (F17). Returns whether
    /// any changed.
    pub fn set_shortcuts(&mut self, shortcuts: &crate::shortcuts::Shortcuts) -> bool {
        let aliases: Vec<(String, String)> = shortcuts
            .aliases()
            .map(|(id, alias)| (id.to_string(), alias.to_string()))
            .collect();
        if self.shortcut_aliases == aliases {
            return false;
        }
        self.shortcut_aliases = aliases;
        self.rebuild_aliases();
        true
    }

    /// Take the translate presets as they now are in `config.json`, for
    /// their aliases (F17). Returns whether anything changed.
    pub fn replace_translate_presets(&mut self, presets: Vec<crate::translate::Preset>) -> bool {
        if self.translate_presets == presets {
            return false;
        }
        self.translate_presets = presets;
        self.rebuild_aliases();
        true
    }

    /// The alias shown beside the indexed item `id`, if it has one (F17).
    pub fn alias_of(&self, id: &str) -> Option<&str> {
        self.aliases.shown(id)
    }

    /// The one alias table, from the user's aliases and the ones the
    /// providers declare (F17). The user's first, so theirs is the badge.
    ///
    /// The ids are the providers' own (`timer:preset:<name>`,
    /// `translate:preset:<name>`, a quicklink's), which is what makes an
    /// alias of the user's for the same item land on the same row.
    fn rebuild_aliases(&mut self) {
        let timers = self.timer_presets.iter().map(|preset| {
            (
                format!("timer:preset:{}", preset.name),
                preset.alias.clone(),
                preset.name.clone(),
            )
        });
        let mut aliases = self.shortcut_aliases.clone();
        let mut names = Vec::new();
        for (id, alias, name) in timers {
            aliases.extend(alias.map(|alias| (id.clone(), alias)));
            names.push((id, name));
        }
        aliases.extend(self.translate_presets.iter().filter_map(|preset| {
            Some((
                format!("translate:preset:{}", preset.name),
                preset.alias.clone()?,
            ))
        }));
        aliases.extend(self.quicklinks.aliases());
        names.extend(self.quicklinks.names());
        self.aliases = Aliases::build(aliases, names);
    }

    /// Take the timer presets as they now are in `config.json`. Returns
    /// whether anything changed.
    pub fn replace_timer_presets(&mut self, presets: Vec<crate::timers::Preset>) -> bool {
        if self.timer_presets == presets {
            return false;
        }
        self.timer_presets = presets;
        self.rebuild_aliases();
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

    /// Take what the file index answered for `query` (F16). Shown whole by
    /// the next [`Self::rank`] of the same query; see [`Self::file_rows`]
    /// for any other.
    pub fn set_files(&mut self, query: &str, hits: Vec<FileHit>) {
        self.forget_file_hits();
        self.files = hits;
        self.files_query = query.trim().to_string();
        self.files_typed = false;
        self.listing.clear();
    }

    /// The path `query` names when the whole of it is one (F16): the
    /// launcher lists its folder with [`browse::list`] instead of asking
    /// the file index, and hands the listing to [`Self::set_typed`].
    pub fn typed(&self, query: &str) -> Option<browse::Typed> {
        browse::typed(query, self.user_home.as_deref())
    }

    /// The path `query` names, when the listing held is of the same folder:
    /// shown narrowed by its filter until its own listing arrives.
    fn same_folder(&self, query: &str) -> Option<browse::Typed> {
        let typed = self.typed(query)?;
        let held = self.typed(&self.files_query).filter(|_| self.files_typed)?;
        (held.dir == typed.dir).then_some(typed)
    }

    /// Take the listing of the folder typed in `query` (F16). Shown whole,
    /// in its own order, by the next [`Self::rank`] of the same query, and
    /// by no other.
    pub fn set_typed(&mut self, query: &str, rows: Vec<Found>) {
        self.forget_file_hits();
        let words = self
            .typed(query)
            .map(|typed| names::words(&typed.filter))
            .unwrap_or_default();
        let home = self.user_home.as_deref();
        self.files = rows
            .iter()
            .map(|found| {
                let item = file_index::item_for(&found.path, found.is_dir, home);
                FileHit {
                    highlights: file_index::highlights(&item.title, &words),
                    item,
                }
            })
            .collect();
        // Untrimmed at the end: a trailing space is part of the filter.
        self.files_query = query.trim_start().to_string();
        self.files_typed = true;
        self.listing = rows;
    }

    /// Take the listing of the folder typed in `query` and rank it, as
    /// [`Self::set_typed`] then [`Self::rank`] do; returns where the row at
    /// `selected` in the hits before it now is.
    ///
    /// The listed row selected stays selected, wherever it now is; when it
    /// is gone, the first listed row is. A fallback selected while rows
    /// were listed stays selected; one selected because nothing was listed
    /// yet gives way to the first row, which Enter and Tab are for here.
    /// Read before the listing is taken: taking it drops the file rows
    /// from the hits, and the row at `selected` would then be another.
    pub fn take_typed(&mut self, query: &str, rows: Vec<Found>, selected: usize) -> usize {
        let listed = self
            .hits
            .iter()
            .any(|hit| matches!(hit.source, Source::File(_)));
        let kept = self
            .hits
            .get(selected)
            .filter(|_| listed)
            .map(|hit| self.item(hit).id.clone());
        self.set_typed(query, rows);
        self.rank(query);
        kept.and_then(|id| self.hits.iter().position(|hit| self.item(hit).id == id))
            .unwrap_or(0)
    }

    /// What `Tab` makes of `query` with the row `selected` (an item id):
    /// the typed path completed to it ([`browse::complete`]), a folder's
    /// ending in a separator. `None` when `query` is no path or the row is
    /// not in its folder's listing.
    pub fn completion(&self, query: &str, selected: &str) -> Option<String> {
        let typed = self.same_folder(query)?;
        let found = self.listing.iter().find(|found| found.id() == selected)?;
        Some(browse::complete(&typed, found))
    }

    /// Forget the files: file search was turned off, or its rules changed.
    /// Returns whether any was being shown, so the caller knows to draw the
    /// list again.
    pub fn clear_files(&mut self) -> bool {
        self.files.clear();
        self.files_query.clear();
        self.files_typed = false;
        self.listing.clear();
        self.forget_file_hits()
    }

    /// Take the file rows out of the current hits, and their section with
    /// them: their [`Source::File`] would point at files no longer held.
    /// Until the next rank, the list is what it was without them.
    fn forget_file_hits(&mut self) -> bool {
        // By what it holds, not by its title: the empty list's sections are
        // the user's to name, `Files` included.
        let Some(at) = self.sections.iter().position(|section| {
            self.hits
                .get(section.start)
                .is_some_and(|hit| matches!(hit.source, Source::File(_)))
        }) else {
            return false;
        };
        let files = self.sections.remove(at);
        self.hits.drain(files.start..files.start + files.len);
        for section in &mut self.sections[at..] {
            section.start -= files.len;
        }
        true
    }

    /// The usage history, for the file index's boosts.
    pub fn frecency(&self) -> &Frecency {
        &self.frecency
    }

    /// The file rows for `query`, at most [`FILES_CAP`].
    ///
    /// The file index answers on another thread, so on every keystroke but
    /// the one it answered, what is held is the answer to an earlier query.
    /// Its rows that still match every word typed -- by the index's own rule,
    /// [`names::word_tier`] -- stay, highlighted for this query, until the
    /// new answer replaces them; the rest go at once. A row that no longer
    /// matches is worse than a row that has not arrived.
    fn file_rows(&self, query: &str) -> Vec<Hit> {
        let query = query.trim();
        // A listing answers its path alone: see `Self::rank_typed`.
        if query.is_empty() || self.files_typed {
            return Vec::new();
        }
        let current = self.files_query == query;
        let words = names::words(query);
        let mut folded = String::new();
        self.files
            .iter()
            .enumerate()
            .filter_map(|(index, found)| {
                let highlights = if current {
                    found.highlights.clone()
                } else {
                    folded.clear();
                    names::fold(&found.item.title, &mut folded);
                    if !words
                        .iter()
                        .all(|word| names::word_tier(&folded, word).is_some())
                    {
                        return None;
                    }
                    file_index::highlights(&found.item.title, &words)
                };
                Some(Hit {
                    source: Source::File(index),
                    highlights,
                    score: 0,
                })
            })
            .take(FILES_CAP)
            .collect()
    }

    /// Rank everything against `query`, replacing the previous hits.
    ///
    /// Answers first and unmatched, then the index by score, then the files
    /// the file index last sent, then the fallbacks. Runs on the UI thread on
    /// every keystroke; nothing here waits on anything (I-3) -- the files are
    /// whatever has arrived.
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

        if self.typed(query).is_some() {
            return self.rank_typed(query);
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

        // Sorted before the answers are prepended, so the answers keep the top
        // regardless of how well anything else scored.
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| self.item(a).title.cmp(&self.item(b).title))
        });

        // An alias is what the user meant (F17): typed whole, its command
        // leads every match; typed in part, it follows those. A keyword only
        // scores, and a short one like `dw` scores below any name that
        // happens to hold the letters -- found, but seventh (F8). The same
        // table holds the timer presets' and quicklinks' whole names, which
        // lead the same way: `google` opened Google Drive (F6). Ties go to
        // the one used more, then the id, so a shared alias is stable.
        let matches = self.aliases.matches(query);
        let by_use = |a: &&str, b: &&str| {
            self.frecency
                .boost(b)
                .cmp(&self.frecency.boost(a))
                .then_with(|| a.cmp(b))
        };
        let mut exact = matches.exact;
        let mut prefix = matches.prefix;
        exact.sort_by(by_use);
        prefix.sort_by(by_use);
        for id in exact.iter().chain(&prefix).rev() {
            self.lead(&mut hits, id);
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

        // The files close the matches, in a section of their own, and never
        // interleave with them (F16): they arrive when the file index
        // answers, and a row that arrived must not land above the one the
        // user is on (I-10). The consequence, accepted, is that a file never
        // outranks an application, as 7.26's discount let a clear match do.
        let results = hits.len();
        let files = self.file_rows(query);
        let file_count = files.len();
        hits.extend(files);

        // A fallback is for when nothing above it was what the user meant, so
        // it comes after the matches and the files -- but no later than the
        // last row of the first screen. Sixteen loose clipboard matches would
        // otherwise bury it below the window, which is where nobody looks for
        // it. Never above an answer, which is what the user asked for by
        // construction. And never inside the files: where it would split
        // their section, it goes above them instead, which keeps it on the
        // first screen too.
        let mut at = hits
            .len()
            .min(self.visible_rows.saturating_sub(self.fallbacks.len()))
            .max(self.answers.len().min(hits.len()));
        if at > results && at < results + file_count {
            at = results;
        }
        hits.splice(
            at..at,
            (0..self.fallbacks.len()).map(|index| Hit {
                source: Source::Fallback(index),
                highlights: Vec::new(),
                score: 0,
            }),
        );

        // The results, then the files under a header of their own, drawn by
        // the same rule as the results'. Fallbacks after the files close the
        // list with no header: they are not files.
        let header = self.arrangement.results_header;
        let results = if at <= results {
            results + self.fallbacks.len()
        } else {
            results
        };
        let rest = hits.len() - results - file_count;
        self.sections = [
            (RESULTS, 0, results, header),
            (FILES, results, file_count, header),
            (RESULTS, results + file_count, rest, false),
        ]
        .into_iter()
        .filter(|&(_, _, len, _)| len > 0)
        .map(|(title, start, len, header)| Section {
            title: title.into(),
            start,
            len,
            header,
        })
        .collect();
        self.hits = hits;
        &self.hits
    }

    /// Rank a query that is a path (F16): its folder's listing, whole and in
    /// the order it was listed, then the fallbacks.
    ///
    /// Nothing else: no application, command or answer matches a path, and
    /// the listing is already filtered and ordered by its own rule, so the
    /// index's word rule is not applied again. Until the listing for this
    /// very query arrives, a listing of the same folder is shown narrowed
    /// by the new filter, by the listing's own rule ([`browse::narrow`]);
    /// one of another folder, or the index's rows, answer nothing here and
    /// the section is empty.
    fn rank_typed(&mut self, query: &str) -> &[Hit] {
        self.answers.clear();
        self.conversion = false;
        self.asks_places = false;
        self.wants_weather = None;
        self.fallbacks = self.quicklinks.fallbacks(query);

        let mut hits: Vec<Hit> = if self.files_typed && self.files_query == query.trim_start() {
            self.files
                .iter()
                .enumerate()
                .map(|(index, found)| Hit {
                    source: Source::File(index),
                    highlights: found.highlights.clone(),
                    score: 0,
                })
                .collect()
        } else if let Some(typed) = self.same_folder(query) {
            let words = names::words(&typed.filter);
            browse::narrow(&self.listing, &typed.filter)
                .into_iter()
                .map(|index| Hit {
                    source: Source::File(index),
                    highlights: file_index::highlights(&self.files[index].item.title, &words),
                    score: 0,
                })
                .collect()
        } else {
            Vec::new()
        };
        let file_count = hits.len();
        hits.extend((0..self.fallbacks.len()).map(|index| Hit {
            source: Source::Fallback(index),
            highlights: Vec::new(),
            score: 0,
        }));

        // The listing under the files' header, which follows the results'
        // as it does for the index's rows; the fallbacks close the list with
        // none.
        let header = self.arrangement.results_header;
        self.sections = [
            (FILES, 0, file_count, header),
            (RESULTS, file_count, self.fallbacks.len(), false),
        ]
        .into_iter()
        .filter(|&(_, _, len, _)| len > 0)
        .map(|(title, start, len, header)| Section {
            title: title.into(),
            start,
            len,
            header,
        })
        .collect();
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

    /// The item in the index with this id, whatever the last query was: what
    /// a command hotkey runs (F17). A linear pass, once per press, over a
    /// few thousand candidates -- cheaper than a second map kept in step with
    /// every re-index.
    pub fn item_by_id(&self, id: &str) -> Option<&Item> {
        self.index
            .iter()
            .map(|candidate| &candidate.item)
            .find(|item| item.id == id)
    }

    pub fn item(&self, hit: &Hit) -> &Item {
        match hit.source {
            Source::Indexed(index) => &self.index[index].item,
            Source::File(index) => &self.files[index].item,
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
    /// Forget the usage of every command that has a hotkey (F17), and the
    /// history to save when anything was forgotten.
    ///
    /// A command run by its hotkey is one the user does not look for in the
    /// list, so its use must not push it up there; Raycast keeps such
    /// commands out of its suggestions for the same reason.
    pub fn forget_hotkeyed(&mut self, shortcuts: &crate::shortcuts::Shortcuts) -> Option<Frecency> {
        let ids = shortcuts.hotkeys().into_iter().map(|(id, _)| id);
        if !self.frecency.forget(ids) {
            return None;
        }
        self.home = None;
        Some(self.frecency.clone())
    }

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
/// holds; so are the extra application folders, resolved
/// ([`crate::config::AppsConfig::folders`]).
pub fn build_index(
    platform: &dyn Platform,
    paths: &Paths,
    quicklinks: &[providers::Quicklink],
    presets: Vec<crate::translate::Preset>,
    timers: Vec<crate::timers::Preset>,
    app_folders: &[std::path::PathBuf],
) -> Result<Vec<Candidate>> {
    let entries = platform.enumerate_applications(app_folders)?;
    log::info!("indexed {} applications", entries.len());

    let catalogue = catalogue(
        &paths.scripts_dir(),
        quicklinks,
        &paths.extensions_dir(),
        &paths.window_commands_file(),
        &paths.window_layouts_file(),
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
    window_commands: &Path,
    window_layouts: &Path,
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

    let commands = providers::window_commands::load(window_commands);
    if !commands.is_empty() {
        log::info!("indexed {} custom window commands", commands.len());
    }
    providers.register(Box::new(providers::WindowCommandsProvider::new(commands)));

    let layouts = providers::window_layouts::load(window_layouts);
    if !layouts.is_empty() {
        log::info!("indexed {} window layouts", layouts.len());
    }
    providers.register(Box::new(providers::WindowLayoutsProvider::new(layouts)));

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

    /// A command hotkey names its item by id (F17): found in the index,
    /// whatever the last query was, and nothing for an id that is gone.
    #[test]
    fn an_item_is_found_by_its_id() {
        let mut root = mixed();
        root.rank("slack");
        assert_eq!(
            root.item_by_id("Code").map(|item| item.title.as_str()),
            Some("Code")
        );
        assert!(root.item_by_id("Gone").is_none());
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

    fn found(titles: &[&str]) -> Vec<FileHit> {
        titles
            .iter()
            .map(|title| FileHit {
                item: item(&format!("file:{title}"), title, Kind::File),
                highlights: Vec::new(),
            })
            .collect()
    }

    fn file_titles(root: &RootSearch) -> Vec<String> {
        root.hits()
            .iter()
            .filter(|hit| matches!(hit.source, Source::File(_)))
            .map(|hit| root.item(hit).title.clone())
            .collect()
    }

    /// F16 replaces 7.26's discount: files arrive late, in a section of
    /// their own, so even a file named exactly what was typed sits below the
    /// application it shares a word with.
    #[test]
    fn a_file_never_ranks_above_an_application_even_on_its_exact_name() {
        let mut root = root(apps(&["Complete Anatomy"]));
        root.set_files("complete", found(&["complete"]));

        root.rank("complete");

        assert_eq!(titles(&root), ["Complete Anatomy", "complete"]);
    }

    /// Answers, then the index, then the files, then the fallbacks -- each
    /// run in one piece, the files under a header of their own.
    #[test]
    fn files_come_after_the_index_and_before_the_fallbacks() {
        let mut root = root(apps(&["Report Builder"]));
        root.replace_quicklinks(google());
        root.set_visible_rows(8);
        root.set_files("report", found(&["report.txt", "reports"]));

        root.rank("report");

        let sources: Vec<Source> = root.hits().iter().map(|hit| hit.source).collect();
        assert!(matches!(
            sources[..],
            [
                Source::Indexed(_),
                Source::File(0),
                Source::File(1),
                Source::Fallback(_)
            ]
        ));
        let files = &root.sections()[1];
        assert_eq!(
            (files.title.as_str(), files.start, files.len, files.header),
            (FILES, 1, 2, true)
        );
        assert_eq!(root.sections()[0].title, RESULTS);
    }

    /// The files' header follows the results' (F15): off for one, off for
    /// both.
    #[test]
    fn the_files_header_follows_the_results_header() {
        let mut root = root(apps(&["Report Builder"]));
        root.set_arrangement(RootConfig {
            results_header: false,
            ..RootConfig::default()
        });
        root.set_files("report", found(&["report.txt"]));

        root.rank("report");

        let files = root
            .sections()
            .iter()
            .find(|section| section.title == FILES)
            .unwrap();
        assert!(!files.header);
    }

    /// A fallback still takes the last row of the first screen, however many
    /// files arrive -- and never splits the files' section to get there.
    #[test]
    fn files_do_not_push_a_fallback_off_the_first_screen() {
        let mut root = root(apps(&["Report Builder", "Reporter"]));
        root.replace_quicklinks(google());
        root.set_visible_rows(8);
        let many: Vec<String> = (0..8).map(|n| format!("report {n}.txt")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        root.set_files("report", found(&many));

        root.rank("report");

        let sources: Vec<Source> = root.hits().iter().map(|hit| hit.source).collect();
        let fallback = sources
            .iter()
            .position(|source| matches!(source, Source::Fallback(_)))
            .unwrap();
        assert!(fallback < 8, "{sources:?}");
        let files: Vec<usize> = (0..sources.len())
            .filter(|&at| matches!(sources[at], Source::File(_)))
            .collect();
        assert_eq!(files.len(), 8);
        assert_eq!(files.last().unwrap() - files[0], 7, "in one piece");
    }

    /// 7.21, kept: open the launcher, do not see a folder called `Archive`.
    #[test]
    fn files_are_never_shown_for_an_empty_query() {
        let mut root = root(apps(&["Firefox"]));
        root.set_files("arc", found(&["Archive"]));

        root.rank("");

        assert!(file_titles(&root).is_empty());
        assert!(titles(&root).contains(&"Firefox".to_string()));
    }

    /// 7.21's cap, kept: eight is enough to find the file you were just
    /// editing, and few enough that the list is still about applications.
    #[test]
    fn at_most_eight_files_are_shown() {
        let many: Vec<String> = (0..40).map(|n| format!("report {n}.txt")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut root = root(Vec::new());
        root.set_files("report", found(&many));

        root.rank("report");

        assert_eq!(file_titles(&root).len(), FILES_CAP);
        assert_eq!(FILES_CAP, 8);
    }

    /// Until the index answers a new query, the rows it gave for the last
    /// one stay if they still match what is typed -- with the new query's
    /// highlights -- and go if they do not.
    #[test]
    fn previous_files_that_still_match_are_kept_until_the_new_answer() {
        let mut root = root(Vec::new());
        root.set_files("rep", found(&["report.txt", "repo notes.md"]));

        root.rank("report");
        assert_eq!(file_titles(&root), ["report.txt"]);
        assert_eq!(root.hits()[0].highlights, [0, 1, 2, 3, 4, 5]);

        root.set_files("report", found(&["report-final.txt"]));
        root.rank("report");
        assert_eq!(file_titles(&root), ["report-final.txt"]);
    }

    /// An answer for another query is never shown as one for this query.
    #[test]
    fn a_stale_answer_is_not_shown() {
        let mut root = root(apps(&["Firefox"]));
        root.set_files("invoice", found(&["invoice.pdf"]));

        root.rank("fire");

        assert!(file_titles(&root).is_empty());
    }

    /// Files forgotten -- search turned off, rules changed -- leave the
    /// list at once, and nothing left points at them.
    #[test]
    fn clearing_the_files_takes_their_rows_out_of_the_list() {
        let mut root = root(apps(&["Report Builder"]));
        root.replace_quicklinks(google());
        root.set_files("report", found(&["report.txt", "reports"]));
        root.rank("report");
        assert_eq!(root.hits().len(), 4);

        assert!(root.clear_files());
        assert!(!root.clear_files(), "nothing left to clear");

        let sources: Vec<Source> = root.hits().iter().map(|hit| hit.source).collect();
        assert!(matches!(
            sources[..],
            [Source::Indexed(_), Source::Fallback(_)]
        ));
        let covered: usize = root.sections().iter().map(|section| section.len).sum();
        assert_eq!(covered, 2);
        assert_eq!(root.sections().last().unwrap().start, 1);
        assert!(
            titles(&root)
                .iter()
                .all(|title| !title.starts_with("report"))
        );
    }

    /// A file is not in the index, so there is nothing to come back to.
    #[test]
    fn a_file_cannot_be_a_favourite() {
        let mut root = root(Vec::new());
        root.set_files("notes", found(&["notes.md"]));

        root.rank("notes");

        let file = root.hits()[0].clone();
        assert!(matches!(file.source, Source::File(_)));
        assert!(root.favorite_changes(&file).is_empty());
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

    /// A typed path's listing, as `browse::list` gives it.
    fn listed(entries: &[(&str, bool)]) -> Vec<Found> {
        entries
            .iter()
            .map(|&(path, is_dir)| Found {
                path: path.into(),
                is_dir,
            })
            .collect()
    }

    /// The whole query a path: its folder's listing is the first section,
    /// the only one besides the fallbacks -- nothing the index or the
    /// providers would match a path with.
    #[test]
    fn a_typed_path_shows_only_its_listing_then_the_fallbacks() {
        let mut root = root(apps(&["Users Manager", "Calculator"]));
        root.replace_quicklinks(google());
        root.set_typed(
            r"C:\Us",
            listed(&[(r"C:\Users", true), (r"C:\Users.txt", false)]),
        );

        root.rank(r"C:\Us");

        let sources: Vec<Source> = root.hits().iter().map(|hit| hit.source).collect();
        assert!(
            matches!(
                sources[..],
                [Source::File(0), Source::File(1), Source::Fallback(_)]
            ),
            "{sources:?}"
        );
        let files = &root.sections()[0];
        assert_eq!(
            (files.title.as_str(), files.start, files.len, files.header),
            (FILES, 0, 2, true)
        );
        assert_eq!(root.sections().len(), 2);
        assert!(!root.sections()[1].header);
    }

    /// The listing is already ordered and already filtered by the folder's
    /// own rule: every row stays, where it was, however many there are and
    /// whatever the words typed would say of them.
    #[test]
    fn a_listing_keeps_its_order_and_every_row() {
        let many: Vec<String> = (0..12).map(|n| format!(r"C:\src\zeta {n}")).collect();
        let mut rows: Vec<(&str, bool)> = vec![(r"C:\src\beta", true), (r"C:\src\alpha", false)];
        rows.extend(many.iter().map(|path| (path.as_str(), false)));
        let mut root = root(Vec::new());
        root.set_typed(r"C:\src\", listed(&rows));

        root.rank(r"C:\src\");

        let shown = file_titles(&root);
        assert_eq!(shown.len(), 14);
        assert_eq!(shown[..3], ["beta", "alpha", "zeta 0"]);
    }

    /// A listing answers its own folder and no other: not a folder inside
    /// it, and not a query that is no path at all.
    #[test]
    fn a_listing_for_another_folder_is_not_shown() {
        let mut root = root(apps(&["Report Builder"]));
        root.set_typed(r"C:\Re", listed(&[(r"C:\Reports", true)]));

        root.rank(r"C:\Reports\");
        assert!(file_titles(&root).is_empty());
        root.rank(r"D:\Re");
        assert!(file_titles(&root).is_empty());

        root.rank("report");
        assert!(file_titles(&root).is_empty());
        assert_eq!(titles(&root), ["Report Builder"]);
    }

    /// Typing a path before its listing arrives shows no rows for it: not
    /// the index's answer to whatever was typed before, nor anything the
    /// index matches.
    #[test]
    fn a_path_without_its_listing_shows_no_files() {
        let mut root = root(apps(&["Calculator", "Code"]));
        root.set_files(r"C:\", found(&["c notes.txt"]));

        root.rank(r"C:\");

        assert!(root.hits().is_empty(), "{:?}", titles(&root));
        assert!(root.sections().is_empty());
    }

    /// Typing on in the same folder narrows the listing held at once, by
    /// the listing's own rule, until the folder's new listing arrives: no
    /// keystroke empties the section only to fill it again.
    #[test]
    fn the_same_folder_with_another_filter_narrows_the_listing_held() {
        let mut root = root(Vec::new());
        root.set_typed(
            r"C:\src\",
            listed(&[
                (r"C:\src\old-lumen", true),
                (r"C:\src\lumen.txt", false),
                (r"C:\src\Lumen", true),
                (r"C:\src\notes", true),
            ]),
        );

        root.rank(r"C:\src\LU");

        assert_eq!(file_titles(&root), ["Lumen", "lumen.txt", "old-lumen"]);
        assert_eq!(root.hits()[0].highlights, [0, 1]);
        assert_eq!(root.sections()[0].title, FILES);

        // Its own listing, when it comes, is shown as it is.
        root.set_typed(r"C:\src\LU", listed(&[(r"C:\src\lumen.txt", false)]));
        root.rank(r"C:\src\LU");
        assert_eq!(file_titles(&root), ["lumen.txt"]);
    }

    /// The title of the row selected at `at`.
    fn title_at(root: &RootSearch, at: usize) -> String {
        root.item(&root.hits()[at]).title.clone()
    }

    /// `C:\` listed and ranked: Users, AMD, Windows, then the fallback.
    fn c_drive() -> RootSearch {
        let mut root = root(Vec::new());
        root.replace_quicklinks(google());
        root.set_typed(
            r"C:\",
            listed(&[
                (r"C:\Users", true),
                (r"C:\AMD", true),
                (r"C:\Windows", true),
            ]),
        );
        root.rank(r"C:\");
        root
    }

    /// Typing on in the same folder: the narrowed row selected is the one
    /// selected when the folder's own listing lands -- not the fallback that
    /// was at its place once the narrowed rows were dropped.
    #[test]
    fn a_listing_landing_keeps_the_narrowed_row_selected() {
        let mut root = c_drive();
        root.rank(r"C:\Us");
        assert_eq!(title_at(&root, 0), "Users");

        let at = root.take_typed(r"C:\Us", listed(&[(r"C:\Users", true)]), 0);

        assert_eq!(at, 0);
        assert_eq!(title_at(&root, at), "Users");
        assert!(matches!(root.hits()[1].source, Source::Fallback(_)));
    }

    /// Another folder, after Tab or typing: nothing was listed for it, so
    /// the fallback was all there was to select. Its listing takes the
    /// selection to its first row.
    #[test]
    fn a_listing_of_another_folder_selects_its_first_row() {
        let mut root = c_drive();
        root.rank(r"C:\AMD\");
        assert_eq!(root.hits().len(), 1);
        assert!(matches!(root.hits()[0].source, Source::Fallback(_)));

        let at = root.take_typed(
            r"C:\AMD\",
            listed(&[(r"C:\AMD\Chipset", true), (r"C:\AMD\Drivers", true)]),
            0,
        );

        assert_eq!(at, 0);
        assert_eq!(title_at(&root, at), "Chipset");
    }

    /// The same folder listed again -- the file index changed, and the
    /// launcher asked again: the row selected stays selected, the fallback
    /// included; a row no longer listed gives way to the first.
    #[test]
    fn a_listing_landing_again_keeps_the_selection() {
        let rows = || {
            listed(&[
                (r"C:\Users", true),
                (r"C:\AMD", true),
                (r"C:\Windows", true),
            ])
        };
        let mut root = c_drive();

        let at = root.take_typed(r"C:\", rows(), 2);
        assert_eq!(title_at(&root, at), "Windows");

        let at = root.take_typed(r"C:\", rows(), 3);
        assert_eq!(at, 3);
        assert!(matches!(root.hits()[at].source, Source::Fallback(_)));

        let at = root.take_typed(r"C:\", listed(&[(r"C:\Users", true)]), 1);
        assert_eq!(title_at(&root, at), "Users");
    }

    /// Tab on a listed row: the query that names it, a folder's ending in
    /// the separator typed, so the next keystroke lists inside it.
    #[test]
    fn a_listed_row_completes_the_typed_path() {
        let mut root = root(Vec::new());
        root.set_typed(
            r"c:/src/lu",
            listed(&[(r"c:/src/lumen", true), (r"c:/src/lu.txt", false)]),
        );
        root.rank(r"c:/src/lu");

        assert_eq!(
            root.completion(r"c:/src/lu", r"folder:c:/src/lumen")
                .as_deref(),
            Some("c:/src/lumen/")
        );
        assert_eq!(
            root.completion(r"c:/src/lu", r"file:c:/src/lu.txt")
                .as_deref(),
            Some("c:/src/lu.txt")
        );
        // Not a listed row, or no path typed: Tab is not a completion.
        assert_eq!(root.completion(r"c:/src/lu", "app:calculator"), None);
        assert_eq!(root.completion(r"c:/src/lu", r"folder:c:/elsewhere"), None);
        assert_eq!(root.completion("lumen", r"folder:c:/src/lumen"), None);
    }

    /// Once the query is no path, the list is what it always was.
    #[test]
    fn a_query_that_is_no_path_is_ranked_as_before() {
        let mut root = root(apps(&["Report Builder"]));
        root.rank(r"C:\");
        root.set_files("report", found(&["report.txt"]));

        root.rank("report");

        assert_eq!(titles(&root), ["Report Builder", "report.txt"]);
        assert_eq!(root.sections()[1].title, FILES);
    }

    /// A listed row is the index's row for the same path: the same id, so
    /// frecency counts it once, and the same actions.
    #[test]
    fn a_listed_row_is_the_indexs_row_for_its_path() {
        let mut root = root(Vec::new());
        root.user_home = Some(r"C:\Users\me".into());
        let path = r"C:\Users\me\Documents";
        root.set_typed(r"~\Doc", listed(&[(path, true)]));

        root.rank(r"~\Doc");

        let item = root.item(&root.hits()[0]);
        assert_eq!(
            *item,
            file_index::item_for(Path::new(path), true, Some(Path::new(r"C:\Users\me")))
        );
        assert_eq!(item.id, format!("folder:{path}"));
        assert_eq!(item.subtitle.as_deref(), Some("~"));
        assert_eq!(root.hits()[0].highlights, [0, 1, 2]);
    }

    /// Whether the launcher lists a folder or asks the file index.
    #[test]
    fn a_path_is_recognised_with_the_users_home() {
        let mut root = root(Vec::new());
        root.user_home = Some(r"C:\Users\me".into());

        let typed = root.typed(r"~\Doc").unwrap();
        assert_eq!(typed.dir, Path::new(r"C:\Users\me\"));
        assert_eq!(typed.filter, "Doc");
        assert_eq!(root.typed(r"d:/src/lu").unwrap().filter, "lu");
        assert_eq!(root.typed("report"), None);

        root.user_home = None;
        assert_eq!(root.typed(r"~\Doc"), None);
        assert!(root.typed(r"C:\").is_some());
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
        root.set_files("zq", found(&["ZedQ2016.fantome", "Zeta_backup.zip"]));
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

    fn shortcuts(aliases: &[(&str, &str)]) -> crate::shortcuts::Shortcuts {
        let mut shortcuts = crate::shortcuts::Shortcuts::default();
        for (id, alias) in aliases {
            shortcuts.commands.insert(
                id.to_string(),
                crate::shortcuts::CommandShortcut {
                    hotkey: None,
                    alias: Some(alias.to_string()),
                },
            );
        }
        shortcuts
    }

    /// Applications that match `lh` well, and two window commands that do
    /// not match it at all.
    fn windows() -> RootSearch {
        let mut index = apps(&["LH Tool", "Lighthouse", "Slack"]);
        index.extend([
            Candidate::new(item("window:left-half", "Left Half", Kind::Command)),
            Candidate::new(item("window:left-third", "Left Third", Kind::Command)),
        ]);
        let mut root = root(index);
        for _ in 0..20 {
            root.record_use("LH Tool");
        }
        root
    }

    /// F17: an alias typed whole is what the user meant, above a name that
    /// matches better and is used every day -- the case a keyword lost.
    #[test]
    fn a_user_alias_typed_whole_leads_over_a_strong_match() {
        let mut root = windows();
        assert!(root.set_shortcuts(&shortcuts(&[("window:left-half", "lh")])));

        root.rank("lh");
        assert_eq!(titles(&root)[0], "Left Half");
        root.rank(" LH ");
        assert_eq!(titles(&root)[0], "Left Half", "in any case");
        assert!(
            !root.set_shortcuts(&shortcuts(&[("window:left-half", "lh")])),
            "the same aliases are no change"
        );
    }

    #[test]
    fn the_start_of_an_alias_lifts_its_command_below_a_whole_one() {
        let mut root = windows();
        root.set_shortcuts(&shortcuts(&[
            ("window:left-half", "lh"),
            ("window:left-third", "lht"),
        ]));

        root.rank("lh");
        assert_eq!(titles(&root)[..2], ["Left Half", "Left Third"]);
        assert_eq!(titles(&root)[2], "LH Tool", "then the matches");
    }

    #[test]
    fn an_alias_is_never_matched_fuzzily() {
        let mut root = windows();
        root.set_shortcuts(&shortcuts(&[("window:left-half", "zqx")]));

        root.rank("zx");
        assert!(!titles(&root).contains(&"Left Half".to_string()));
        root.rank("qx");
        assert!(!titles(&root).contains(&"Left Half".to_string()));
    }

    /// Settings flags a shared alias; until it is fixed, both lead, the one
    /// used more first, and by id when neither is.
    #[test]
    fn commands_sharing_an_alias_both_lead_by_use_then_id() {
        let mut root = windows();
        root.set_shortcuts(&shortcuts(&[
            ("window:left-third", "w"),
            ("window:left-half", "w"),
        ]));

        root.rank("w");
        assert_eq!(titles(&root)[..2], ["Left Half", "Left Third"]);

        root.record_use("window:left-third");
        root.rank("w");
        assert_eq!(titles(&root)[..2], ["Left Third", "Left Half"]);
    }

    #[test]
    fn a_users_alias_and_a_presets_own_both_work() {
        use crate::providers::Provider as _;

        let preset = crate::timers::Preset {
            name: "Zeta".into(),
            alias: Some("zq".into()),
            duration: std::time::Duration::from_secs(420),
            cycle: None,
            focus: false,
        };
        let presets = providers::TimerPresetsProvider::new(vec![preset.clone()]);
        let mut index = apps(&["Zed Quick", "Zz Top"]);
        index.extend(presets.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_timer_presets(vec![preset]);
        root.set_shortcuts(&shortcuts(&[("timer:preset:Zeta", "zz")]));

        root.rank("zq");
        assert_eq!(titles(&root)[0], "Zeta");
        root.rank("zz");
        assert_eq!(titles(&root)[0], "Zeta");
        assert_eq!(root.alias_of("timer:preset:Zeta"), Some("zz"), "the user's");
    }

    /// F17: a translate preset's alias is one more alias, where it was only a
    /// keyword before.
    #[test]
    fn a_translate_presets_alias_leads() {
        use crate::providers::Provider as _;

        let config: crate::translate::TranslateConfig = serde_json::from_str(
            r#"{ "presets": [ { "name": "Work", "alias": "esen", "from": "es", "to": "en" } ] }"#,
        )
        .unwrap();
        let provider = providers::TranslateProvider::new(config.presets());
        let mut index = apps(&["Essential Sentences"]);
        index.extend(provider.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        for _ in 0..20 {
            root.record_use("Essential Sentences");
        }
        assert!(root.replace_translate_presets(config.presets()));

        root.rank("esen");
        assert_eq!(titles(&root)[0], "Work");
        assert_eq!(root.alias_of("translate:preset:Work"), Some("esen"));
    }

    #[test]
    fn a_quicklinks_alias_is_its_badge_and_its_name_is_not() {
        let links = google();
        let mut index = apps(&["Gimp"]);
        index.extend(links.catalogue().into_iter().map(Candidate::new));
        let mut root = root(index);
        root.replace_quicklinks(links);

        let id = &root.index()[1].item.id.clone();
        assert_eq!(root.alias_of(id), Some("g"));
        assert_eq!(root.alias_of("Gimp"), None);
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
