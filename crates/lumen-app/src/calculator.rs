//! Keeping the calculator's history (F14).
//!
//! The decisions -- what is kept, in what order, how many -- are in
//! `lumen_core::calc::history`, where they are tested; this file owns the
//! file and nothing else.

use std::path::Path;

use lumen_core::calc::history::{Entry, History};
use parking_lot::Mutex;

/// Held while the file is read, changed and written. Each use is written on
/// the background executor, and two copies a moment apart would otherwise
/// both read the file before either wrote it, and one would be lost.
static WRITING: Mutex<()> = Mutex::new(());

/// Keep `expression` and the `result` copied from it, at most `limit` in all.
pub fn record(path: &Path, expression: String, result: String, limit: usize) {
    let _writing = WRITING.lock();
    let mut history = History::load(path);
    let entry = Entry {
        expression,
        result,
        at: lumen_core::clock::now().as_second(),
    };
    if history.record(entry, limit)
        && let Err(error) = history.save(path)
    {
        log::warn!("could not keep the calculation: {error:#}");
    }
}
