//! What the calculator costs on a keystroke (F14).
//!
//! It runs on the UI thread for every key typed into the root list, inside
//! the 8 ms frame (see `crate::providers`). Written before F14 taught it
//! anything, so the queries below are measured on F3's calculator first --
//! most of them declined -- and every slice after is compared with that.
//!
//! Three kinds of query, because they cost differently:
//!
//! - **Declined**, the common case: most of what is typed is not maths, and
//!   that has to stay nearly free.
//! - **Answered**, one of each thing F14 adds.
//! - **Hostile**, the worst a user could type without meaning harm: deep
//!   nesting, a tower of powers, the longest input there is.

use criterion::{Criterion, criterion_group, criterion_main};
use lumen_core::providers::Provider;
use lumen_core::providers::calculator::CalculatorProvider;

const DECLINED: &[(&str, &str)] = &[
    ("application", "visual studio code"),
    ("seven-zip", "7-zip"),
    ("timer", "25m"),
    ("currency", "10 usd in cad"),
];

const ANSWERED: &[(&str, &str)] = &[
    ("arithmetic", "(1+2)*3/4-5"),
    ("functions", "sqrt(2)^pi / log(1000) * sin(45 deg)"),
    ("conversion", "5 km in mi"),
    ("percentage", "25% off 399"),
    ("date", "days until 25 dec"),
];

fn hostile() -> Vec<(&'static str, String)> {
    vec![
        (
            "nesting",
            format!("{}1{}", "(".repeat(100), ")".repeat(100)),
        ),
        ("powers", "2^2^2^2^2^2^2^2^2^2".to_string()),
        ("long", vec!["1.5"; 64].join("+")),
    ]
}

fn calculator(c: &mut Criterion) {
    let mut group = c.benchmark_group("calculator");

    for (name, query) in DECLINED.iter().chain(ANSWERED) {
        group.bench_function(*name, |b| b.iter(|| CalculatorProvider.answer(query)));
    }
    for (name, query) in hostile() {
        group.bench_function(name, |b| b.iter(|| CalculatorProvider.answer(&query)));
    }

    // Money, as the root list asks it (F14, slice 12): with no table, the
    // first keystroke of the day; the parse is the whole cost either way.
    let context = lumen_core::calc::Context {
        today: &lumen_core::calc::today,
        rates: None,
    };
    group.bench_function("money", |b| {
        b.iter(|| lumen_core::calc::answer("100 usd + 20 eur in canadian dollar", &context))
    });

    group.finish();
}

criterion_group!(benches, calculator);
criterion_main!(benches);
