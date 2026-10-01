//! Who answers what the user types (F14, slice 1).
//!
//! The calculator's hard part is declining, and F14 makes it harder: units,
//! functions and phrases bring letters into a gate that F3 kept quiet by
//! refusing every letter. So before the first unit exists, this corpus states
//! who owns each query -- an application search, a timer, a currency, a
//! clock, the weather, or the calculator -- and checks two things for every
//! row:
//!
//! 1. The calculator answers exactly the rows it owns.
//! 2. Every recogniser that is not the owner declines. Two answers for one
//!    query is one too many: the second pushes down what was looked for.
//!
//! Each slice that teaches the calculator something adds the rows it now
//! owns, and the rows it must still leave alone.

use lumen_core::providers::{Provider, TimersProvider, calculator::CalculatorProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// No answer: the query is a search, for an application or anything else.
    Search,
    Calculator,
    Timer,
    Currency,
    Clock,
    Weather,
}

use Owner::*;

const CORPUS: &[(&str, Owner)] = &[
    // Arithmetic.
    ("2+2", Calculator),
    ("12*7", Calculator),
    ("(1+2)*3/4-5", Calculator),
    ("2^10", Calculator),
    ("-3 + 10", Calculator),
    // Functions and constants (slice 3).
    ("sqrt(2)^pi / log(1000) * sin(45 deg)", Calculator),
    ("sqrt(16)", Calculator),
    ("ln(e)", Calculator),
    ("pi * 2", Calculator),
    ("5!", Calculator),
    ("1e3 * 2", Calculator),
    ("sqrt", Search),
    ("sin", Search),
    ("log", Search),
    ("exp", Search),
    ("abs", Search),
    ("round", Search),
    ("2pi", Search),
    ("1e3", Search),
    ("45 deg", Search),
    ("rad", Search),
    ("log4j 2", Search),
    ("e10 lab", Search),
    ("tan 2", Search),
    ("(2024)", Search),
    ("half-life 2", Search),
    ("sin(", Search),
    // Percentages, and the remainder by name (slice 4, 7.39).
    ("25% off 399", Calculator),
    ("20% of 50", Calculator),
    ("399 + 25%", Calculator),
    ("5 as % of 20", Calculator),
    ("10 mod 3", Calculator),
    ("50%", Search),
    ("10%3", Search),
    ("100% orange juice", Search),
    ("mod", Search),
    ("20% of", Search),
    ("5 as", Search),
    // Units (slice 5). A conversion is the calculator's, even of time; a
    // duration with a name after it is still a timer.
    ("5 km in mi", Calculator),
    ("100 f in c", Calculator),
    ("1 GiB in MB", Calculator),
    ("25 min in s", Calculator),
    ("2h + 30min", Calculator),
    ("5 km", Search),
    ("1 gb", Search),
    ("5k", Search),
    ("2f + 3", Search),
    ("5 in", Search),
    ("1 to 10", Search),
    ("3 in 1 printer", Search),
    ("windows 10 to 11", Search),
    ("5 km in kg", Search),
    ("10m in the oven", Timer),
    ("5m tea", Timer),
    ("focus 45m", Timer),
    ("3pm in tokyo", Clock),
    // More units (slice 6).
    ("100 km/h in mph", Calculator),
    ("2.4 ghz in mhz", Calculator),
    ("1 kwh in mj", Calculator),
    ("60 hz", Search),
    ("100 mph", Search),
    ("1 l", Search),
    ("hp laptop 15", Search),
    ("ps5", Search),
    ("bar", Search),
    ("w10", Search),
    ("a4 paper", Search),
    // Dates (slice 7). Whatever today is, these answer or decline alike.
    ("days until 25 dec", Calculator),
    ("days until friday", Calculator),
    ("weeks since 1 jan", Calculator),
    ("today + 3 weeks", Calculator),
    ("25 dec - today", Calculator),
    ("today", Search),
    ("friday", Search),
    ("dec", Search),
    ("may", Search),
    ("sun", Search),
    ("25 dec", Search),
    ("jan 2024", Search),
    ("until dawn", Search),
    ("since 1999", Search),
    ("28 days later", Search),
    ("500 days of summer", Search),
    ("days gone", Search),
    ("friday night funkin", Search),
    // Applications, and words that start them.
    ("code", Search),
    ("notepad", Search),
    ("visual studio code", Search),
    ("firefox", Search),
    ("7-zip", Search),
    ("7zip", Search),
    ("obs 28", Search),
    ("windows 11", Search),
    ("python 3.12", Search),
    ("c++", Search),
    ("office 365", Search),
    ("3ds max", Search),
    ("e", Search),
    ("pi", Search),
    ("ex", Search),
    // Lone units, which are also the first letters of everything.
    ("m", Search),
    ("mi", Search),
    ("km", Search),
    ("kb", Search),
    ("deg", Search),
    ("f", Search),
    ("c", Search),
    // Numbers on their own, and things shaped like numbers.
    ("2024", Search),
    ("3.5", Search),
    ("1.2.3", Search),
    ("192.168.1.1", Search),
    ("localhost:8080", Search),
    ("github.com", Search),
    // Timers (F8): a unit is required, and what follows is the name.
    ("25m", Timer),
    ("25 min", Timer),
    ("90s", Timer),
    ("1h30", Timer),
    ("1:30", Timer),
    ("1h30 deep work", Timer),
    ("timer 5m", Timer),
    // Currencies (F9).
    ("10 usd in cad", Currency),
    ("10usd to japan", Currency),
    ("2*5 EUR in canadian dollar", Currency),
    // The clock (F13).
    ("time in vancouver", Clock),
    ("vancouver time", Clock),
    ("3pm vancouver", Clock),
    ("15:00 vancouver in tokyo", Clock),
    // The weather (F13).
    ("weather in vancouver", Weather),
    ("vancouver weather", Weather),
];

/// Overlaps that were found by this corpus and are not the calculator's to
/// settle, stated exactly so that fixing one fails here and removes its row.
///
/// `15:00 vancouver in tokyo` is the clock's (F13), and is also read as a
/// fifteen-minute timer named *vancouver in tokyo* (F8): a clock reading is
/// a duration to the timers. Found before F14 changed anything.
const KNOWN_OVERLAPS: &[(&str, &[Owner])] = &[("15:00 vancouver in tokyo", &[Timer, Clock])];

/// The calculator, as the root list asks it.
fn calculator(query: &str) -> bool {
    !CalculatorProvider.answer(query).is_empty()
}

/// Every owner whose recogniser claims `query`, the calculator included.
fn claims(query: &str) -> Vec<Owner> {
    let mut owners = Vec::new();
    if calculator(query) {
        owners.push(Calculator);
    }
    if !TimersProvider.answer(query).is_empty() {
        owners.push(Timer);
    }
    if lumen_core::currency::query::recognise(query).is_some() {
        owners.push(Currency);
    }
    if lumen_core::clock::query::recognise(query).is_some() {
        owners.push(Clock);
    }
    if lumen_core::weather::query::recognise(query).is_some() {
        owners.push(Weather);
    }
    owners
}

#[test]
fn every_query_has_one_owner_at_most() {
    let mut wrong = Vec::new();
    for &(query, owner) in CORPUS {
        let claimed = claims(query);
        let expected = match KNOWN_OVERLAPS.iter().find(|(known, _)| *known == query) {
            Some((_, owners)) => owners.to_vec(),
            None if owner == Search => Vec::new(),
            None => vec![owner],
        };
        if claimed != expected {
            wrong.push(format!(
                "{query:?}: expected {expected:?}, claimed by {claimed:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}
