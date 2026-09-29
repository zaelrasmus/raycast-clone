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

use crate::frecency::Frecency;
use crate::item::Item;
use crate::paths::Paths;
use crate::providers::{self, Provider as _, Providers, QuicklinkProvider};
use crate::search::{Candidate, Search};

/// How many results are kept behind the visible window.
///
/// Bounded because nobody scrolls to the four hundredth fuzzy match, and an
/// unbounded list makes the sort the most expensive thing in the frame.
pub const MAX_RESULTS: usize = 64;

/// How many clipboard entries are held in the searchable snapshot.
///
/// Fewer than the history keeps. The database holds a month so that a command
/// dedicated to browsing it has something to browse; the snapshot is what root
/// search matches against, and reading a month of pastes into memory to answer
/// a three-character query is not a trade worth making.
pub const CLIPBOARD_ENTRIES: usize = 100;

/// A secondary index: searched, but never browsed (7.21).
///
/// Both of these are large, both change without the user doing anything to the
/// launcher, and neither belongs in the root list. With an empty query the
/// index is shown in frecency order, so entries with no usage history would
/// sort alphabetically among the applications -- open the launcher, see what
/// you copied last Tuesday, and a folder called `Archive`. So an aside is
/// matched only when there is a query, and capped so that a common letter
/// cannot fill the list with one kind of thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aside {
    Clipboard,
    Files,
}

impl Aside {
    pub const ALL: [Aside; 2] = [Aside::Clipboard, Aside::Files];

    fn slot(self) -> usize {
        match self {
            Aside::Clipboard => 0,
            Aside::Files => 1,
        }
    }

    /// How many of this aside's rows may appear in one set of results.
    ///
    /// Eight is enough to find a recent paste or the file you were just
    /// editing, and few enough that the list is still about applications.
    pub fn cap(self) -> usize {
        match self {
            Aside::Clipboard | Aside::Files => 8,
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
            Aside::Clipboard | Aside::Files => 6,
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
    /// The searchable index. Empty until the first enumeration completes,
    /// which is why the root list is allowed to be empty rather than treated
    /// as a bug.
    index: Arc<Vec<Candidate>>,
    /// In [`Aside::slot`] order.
    asides: [Arc<Vec<Candidate>>; 2],
    /// This query's provider answers.
    answers: Vec<Item>,
    /// The quicklinks, held here rather than among the providers because they
    /// are the one provider the user edits while the daemon runs (F6).
    quicklinks: QuicklinkProvider,
    /// The timer presets, for their names and aliases (F8): typed exactly,
    /// one leads the matches, as a quicklink does.
    timer_presets: Vec<crate::timers::Preset>,
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
            index: Arc::new(Vec::new()),
            asides: std::array::from_fn(|_| Arc::new(Vec::new())),
            answers: Vec::new(),
            quicklinks: QuicklinkProvider::default(),
            timer_presets: Vec::new(),
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

    pub fn aside_is_empty(&self, aside: Aside) -> bool {
        self.asides[aside.slot()].is_empty()
    }

    /// Rank everything against `query`, replacing the previous hits.
    ///
    /// Answers first and unmatched, then the index and the asides merged by
    /// score. Runs on the UI thread on every keystroke; nothing here waits on
    /// anything (I-3).
    pub fn rank(&mut self, query: &str) -> &[Hit] {
        // Providers are contractually instant (see `crate::providers`).
        self.answers = self.providers.answers(query);
        let trimmed = query.trim();
        if !trimmed.is_empty() {
            self.answers.extend(self.quicklinks.answer(trimmed));
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
            .rank(query, &self.index, &self.frecency, MAX_RESULTS)
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
            if let Some(at) = hits.iter().position(|hit| self.item(hit).id == id) {
                let hit = hits.remove(at);
                hits.insert(0, hit);
            }
        }

        // A quicklink typed exactly -- its whole name, or its alias -- is what
        // the user meant, and leads the matches. Scoring alone lets usage
        // put a longer name first: `google` opened Google Drive (F6).
        if let Some(id) = self.quicklinks.exact(query)
            && let Some(at) = hits.iter().position(|hit| self.item(hit).id == id)
        {
            let hit = hits.remove(at);
            hits.insert(0, hit);
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
        hits.truncate(MAX_RESULTS.saturating_sub(self.fallbacks.len()));
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

        self.hits = hits;
        &self.hits
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
        self.frecency.clone()
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
        &paths.snippets_file(),
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
    snippets_file: &Path,
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

    let snippets = providers::snippets::load(snippets_file);
    if !snippets.is_empty() {
        log::info!("indexed {} snippets", snippets.len());
    }
    providers.register(Box::new(providers::SnippetProvider::new(snippets)));

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

    #[test]
    fn results_are_bounded() {
        let many: Vec<String> = (0..200).map(|n| format!("App {n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut root = root(apps(&many));

        root.rank("app");

        assert_eq!(root.hits().len(), MAX_RESULTS);
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

        assert_eq!(root.hits().len(), MAX_RESULTS);
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
