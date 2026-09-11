//! The per-frame search budget (section 5): p95 < 8 ms while typing, with the
//! full list loaded.
//!
//! Written as a benchmark rather than as a test because I-8 asks for a measured
//! number, and because the working rules ask for the measurement *before* the
//! functionality: a benchmark written beforehand is a constraint, written
//! afterwards it is a justification.
//!
//! What this covers is ranking, which is the part that runs on every keystroke
//! and scales with the size of the index. It does not cover the render pass, and
//! it does not cover the gesture-to-first-frame budget -- those need a harness
//! that drives the real binary, which is listed in the README as the remaining
//! F1 work rather than quietly implied to exist.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lumen_core::frecency::Frecency;
use lumen_core::item::{Action, Item, Kind};
use lumen_core::search::{Candidate, Search};
use lumen_platform::LaunchTarget;

/// A synthetic index of `count` entries.
///
/// Names are varied deliberately: a corpus of identical strings lets the matcher
/// short-circuit in ways a real Start menu never would, and would measure
/// something other than the thing being claimed.
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
            let base = WORDS[i % WORDS.len()];
            Candidate::new(Item {
                id: format!("app:{i}"),
                title: format!("{base} {i}"),
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

fn ranking(c: &mut Criterion) {
    let mut group = c.benchmark_group("rank");

    // 500 is a typical Start menu; 5000 is a machine with every SDK installed,
    // and is the size the budget actually has to hold at.
    for count in [500usize, 2000, 5000] {
        let candidates = index(count);
        let mut frecency = Frecency::new();
        for i in 0..count / 10 {
            frecency.record(&format!("app:{i}"));
        }

        // The empty query is the root list: every entry scores, so this is the
        // worst case for sorting rather than for matching.
        group.bench_with_input(BenchmarkId::new("root", count), &count, |b, _| {
            let mut search = Search::new();
            b.iter(|| search.rank("", &candidates, &frecency, 64));
        });

        // A one-character query is the worst case overall: almost everything
        // matches, so nothing is filtered out early.
        group.bench_with_input(BenchmarkId::new("one-char", count), &count, |b, _| {
            let mut search = Search::new();
            b.iter(|| search.rank("c", &candidates, &frecency, 64));
        });

        // What a real query looks like a few keystrokes in.
        group.bench_with_input(BenchmarkId::new("typical", count), &count, |b, _| {
            let mut search = Search::new();
            b.iter(|| search.rank("vsc", &candidates, &frecency, 64));
        });
    }

    group.finish();
}

criterion_group!(benches, ranking);
criterion_main!(benches);
