//! The part of I-8 that CI can honestly enforce.
//!
//! "The performance budgets are measured in CI. A regression breaks the build."
//! Two of the six can be: ranking cost, here, and installed size, which is a
//! step in the workflow because it compares bytes rather than behaviour.
//!
//! The other four -- gesture to first frame, idle CPU, idle memory, cold start
//! -- need a compositor presenting to a real display, a session that is
//! genuinely idle, and a GPU. A hosted runner has none of those. Measuring them
//! there anyway would produce numbers that look like evidence and are not, so
//! the daemon measures them on the machine it is actually running (see
//! `lumen-app/src/timing.rs`) and the README records what they were and on
//! what. That gap is stated rather than papered over; see deviation 7.24.
//!
//! # Why this asserts a shape rather than a time
//!
//! A shared CI runner is not a benchmark machine. Its clock speed varies with
//! whatever else is on the host, and an assertion like "under 8 milliseconds"
//! on such a machine is either so tight that it fails on a noisy afternoon or
//! so loose that it catches nothing.
//!
//! What actually threatens this budget is not a machine getting slower; it is
//! somebody making ranking super-linear -- an inner loop over the index, a sort
//! inside the match, a per-candidate allocation that grows. That shows up as a
//! *shape* change, and a shape is measurable on any machine: quadruple the
//! index and the time should roughly quadruple.
//!
//! So there are two assertions, and neither is a stopwatch:
//!
//! 1. **Growth is not super-linear.** Four times the entries must not cost more
//!    than eight times the work. Linear passes ~4×; quadratic would be ~16×.
//! 2. **A very loose ceiling**, fifty times the measured figure, which catches a
//!    constant-factor catastrophe without ever firing on ordinary noise.
//!
//! The real numbers live in the benchmark (`cargo bench -p lumen-core`), where
//! they belong, and in the README.

use std::time::{Duration, Instant};

use lumen_core::frecency::Frecency;
use lumen_core::item::{Action, Item, Kind};
use lumen_core::search::{Candidate, Search};
use lumen_platform::LaunchTarget;

/// The index size the budget has to hold at: F3's file-search cap, when files
/// were ranked by this matcher. Since F16 they are matched by the file index,
/// off the UI thread (`docs/FILE-SEARCH.md`), and the largest list ranked on a
/// keystroke is the application index -- 5 000 is a machine with every SDK
/// installed -- so the figure stands, for that.
const LARGE: usize = 5_000;
/// A quarter of it, for the growth comparison.
const SMALL: usize = LARGE / 4;

/// Fifty times the measured 1.06 ms. Not a budget -- the budget is 8 ms for the
/// whole frame, and this is one list inside it -- but a line no healthy build
/// comes anywhere near, on any machine.
const CEILING: Duration = Duration::from_millis(50);

/// How many times four times the entries may cost.
///
/// Linear is 4. Eight leaves room for cache effects, which are real and get
/// worse with size, while still being far below the 16 that quadratic would
/// produce.
const GROWTH_LIMIT: f64 = 8.0;

fn index(count: usize) -> Vec<Candidate> {
    const WORDS: [&str; 12] = [
        "Visual Studio Code",
        "Windows Terminal",
        "Firefox Developer Edition",
        "Adobe Acrobat Reader",
        "Microsoft Excel",
        "Slack",
        "Docker Desktop",
        "PowerToys",
        "Character Map",
        "System Information",
        "Notepad",
        "Calculator",
    ];

    (0..count)
        .map(|i| {
            Candidate::new(Item {
                id: format!("app:{i}"),
                title: format!("{} {i}", WORDS[i % WORDS.len()]),
                subtitle: Some("Publisher".into()),
                icon: None,
                keywords: vec!["tool".into(), "utility".into()],
                actions: vec![Action::Launch(LaunchTarget::ShellExecute(format!(
                    "app-{i}"
                )))],
                kind: Kind::Application,
            })
        })
        .collect()
}

/// Rank `query` against `count` entries, returning the best of several runs.
///
/// The best rather than the mean, deliberately. On a shared runner the mean is
/// mostly a measure of what else is on the host; the minimum is the closest
/// thing to the work this code actually does.
fn cost(count: usize, query: &str) -> Duration {
    let candidates = index(count);
    let mut frecency = Frecency::new();
    for i in 0..count / 10 {
        frecency.record(&format!("app:{i}"));
    }

    let mut search = Search::new();
    // One discarded run so the comparison is not measuring page faults on a
    // freshly allocated index.
    let _ = search.rank(query, &candidates, &frecency, 64);

    (0..5)
        .map(|_| {
            let started = Instant::now();
            let hits = search.rank(query, &candidates, &frecency, 64);
            let elapsed = started.elapsed();
            // Consumed so the optimiser cannot decide the work was pointless.
            assert!(hits.len() <= 64);
            elapsed
        })
        .min()
        .expect("five runs")
}

/// The worst case: one character, so nothing is filtered out early.
#[test]
fn ranking_a_full_index_stays_far_inside_the_frame() {
    let elapsed = cost(LARGE, "e");
    assert!(
        elapsed < CEILING,
        "ranking {LARGE} entries took {elapsed:?}, past the {CEILING:?} ceiling -- \
         run `cargo bench -p lumen-core --bench search` to see what happened"
    );
}

/// The root list scores everything and then sorts it, which is the other way
/// this could be made super-linear.
#[test]
fn the_root_list_stays_far_inside_the_frame() {
    let elapsed = cost(LARGE, "");
    assert!(
        elapsed < CEILING,
        "the root list of {LARGE} entries took {elapsed:?}, past the {CEILING:?} ceiling"
    );
}

/// The assertion that actually earns its place: ranking is linear in the size
/// of the index, and a change that makes it quadratic fails here rather than in
/// somebody's launcher six months later.
#[test]
fn ranking_cost_grows_no_faster_than_the_index() {
    let small = cost(SMALL, "e");
    let large = cost(LARGE, "e");

    // Below the timer's resolution the ratio is noise, not measurement.
    if small < Duration::from_micros(50) {
        return;
    }

    let growth = large.as_secs_f64() / small.as_secs_f64();
    assert!(
        growth < GROWTH_LIMIT,
        "ranking got {growth:.1}x slower for 4x the entries ({SMALL} -> {LARGE}, \
         {small:?} -> {large:?}). Linear is 4x; anything near 16x is quadratic."
    );
}

/// How much a table four times larger may cost an alias lookup (F17).
///
/// A lookup is a binary search and then the keys it matches, so it should
/// barely notice the table's size: the logarithm of four times the entries
/// is a step, not a multiple. A pass over every alias would be 4x, and so
/// would fail here, where the ranking's limit of eight would let it through.
const ALIAS_GROWTH_LIMIT: f64 = 2.5;

/// How long `LOOKUPS` lookups of queries naming one alias each take against
/// a table of `count`, the best of five.
fn alias_cost(count: usize) -> Duration {
    const LOOKUPS: usize = 5_000;
    let aliases = lumen_core::aliases::Aliases::build(
        (0..count).map(|i| (format!("app:{i}"), format!("a{i}x"))),
        (0..count).map(|i| (format!("app:{i}"), format!("Application {i}"))),
    );
    let queries: Vec<String> = (0..LOOKUPS).map(|i| format!("a{}x", i % SMALL)).collect();
    (0..5)
        .map(|_| {
            let started = Instant::now();
            let mut found = 0;
            for query in &queries {
                found += aliases.matches(query).exact.len();
            }
            let elapsed = started.elapsed();
            assert_eq!(found, LOOKUPS);
            elapsed
        })
        .min()
        .expect("five runs")
}

/// I-8 for the alias table: every keystroke looks the query up, so its cost
/// must follow what matches, not how many aliases there are.
#[test]
fn an_alias_lookup_does_not_grow_with_the_table() {
    let small = alias_cost(SMALL);
    let large = alias_cost(LARGE);
    let growth = large.as_secs_f64() / small.as_secs_f64();
    println!("alias lookups: {SMALL} aliases {small:?}, {LARGE} aliases {large:?} ({growth:.2}x)");
    assert!(
        large < CEILING,
        "5 000 alias lookups took {large:?}, past the {CEILING:?} ceiling"
    );
    assert!(
        growth < ALIAS_GROWTH_LIMIT,
        "alias lookups got {growth:.1}x slower for 4x the aliases ({small:?} -> {large:?})"
    );
}
