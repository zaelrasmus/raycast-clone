//! How long the emoji catalogue (F18) takes to read, and a search to run.
//!
//! The catalogue is read once, off the UI thread; a search runs on every
//! keystroke, so it is the one the frame budget (p95 < 8 ms) is about.
//!
//! ```text
//! cargo run --release -p lumen-core --example emoji_timing
//! ```

use std::time::{Duration, Instant};

use lumen_core::emoji::{self, CustomKeywords};

fn main() {
    let started = Instant::now();
    let catalogue = emoji::catalogue();
    println!(
        "load: {} entries in {:?}",
        catalogue.entries().len(),
        started.elapsed()
    );

    let custom = CustomKeywords::new();
    for query in ["smile", "s", "heart", "grnng", ""] {
        const RUNS: usize = 200;
        let mut times: Vec<Duration> = (0..RUNS)
            .map(|_| {
                let started = Instant::now();
                std::hint::black_box(catalogue.search(query, None, &custom));
                started.elapsed()
            })
            .collect();
        times.sort();
        println!(
            "search {query:?}: {} found, p50 {:?}, p95 {:?}",
            catalogue.search(query, None, &custom).len(),
            times[RUNS / 2],
            times[RUNS * 95 / 100]
        );
    }
}
