//! The window commands that are not placements (F17): minimise, restore and
//! focus. The decisions in them that need no operating system.
//!
//! [`crate::tiling`]'s reason for living on the boundary applies here too.
//! Which windows a "minimise this display" touches, which window a "restore"
//! brings back and which window "focus next" lands on are the same questions
//! on Windows and on Hyprland, and they are the parts that are easy to get
//! subtly wrong. The platform reads the facts and does the calls; the choices
//! are made here, where they are tested without a window in sight.

/// How many windows [`History`] remembers before forgetting the oldest.
///
/// Generous for one person's desktop -- nobody arranges sixty-four windows
/// and then wants the first one back -- and small enough that a linear scan
/// is free.
pub const HISTORY_LIMIT: usize = 64;

/// What a window was like before Lumen last arranged or minimised it, per
/// window, most recent last.
///
/// Generic over what is remembered so the bookkeeping is tested here; the
/// platform stores its own placement record (on Windows a `WINDOWPLACEMENT`).
/// Keyed by the platform's window handle as a number.
///
/// A handle outlives nothing: once its window closes the number can be
/// handed to a new one. So every operation first drops the entries whose
/// window is gone, asked through `alive`, and a handle reused in between two
/// operations is the one case this cannot see.
#[derive(Debug, Clone)]
pub struct History<T> {
    entries: Vec<Entry<T>>,
    limit: usize,
}

/// What [`History::pop`] hands back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded<T> {
    pub saved: T,
    /// Whether Lumen minimised the window after saving this. Restoring such a
    /// window is undoing the minimise -- on Windows `SW_RESTORE`, which keeps
    /// a snapped or maximised window snapped or maximised -- rather than
    /// re-applying a rectangle.
    pub minimised: bool,
}

#[derive(Debug, Clone)]
struct Entry<T> {
    window: u64,
    saved: T,
    /// Whether what Lumen did next was to minimise it.
    minimised: bool,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self::with_limit(HISTORY_LIMIT)
    }
}

impl<T> History<T> {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Remember `saved` for `window`, which Lumen is about to arrange, or to
    /// minimise when `minimised`.
    ///
    /// Replaces what was remembered for it: "restore" goes back to before the
    /// *last* arrangement. The window becomes the most recent, and the oldest
    /// is forgotten past the limit.
    pub fn record(&mut self, window: u64, saved: T, minimised: bool, alive: impl Fn(u64) -> bool) {
        self.prune(&alive);
        self.entries.retain(|entry| entry.window != window);
        self.entries.push(Entry {
            window,
            saved,
            minimised,
        });
        let excess = self.entries.len().saturating_sub(self.limit);
        self.entries.drain(..excess);
    }

    /// Take what was remembered for `window`, if anything.
    pub fn pop(&mut self, window: u64, alive: impl Fn(u64) -> bool) -> Option<Recorded<T>> {
        self.prune(&alive);
        let index = self
            .entries
            .iter()
            .position(|entry| entry.window == window)?;
        let entry = self.entries.remove(index);
        Some(Recorded {
            saved: entry.saved,
            minimised: entry.minimised,
        })
    }

    /// The window Lumen minimised most recently, of those for which
    /// `candidate` holds -- on Windows, that it still exists and is still
    /// minimised.
    ///
    /// This is what makes Minimise then Restore a round trip by hotkey: after
    /// minimising, the window in front is some other window, so Restore has
    /// to find the minimised one itself.
    pub fn latest_minimised(&self, candidate: impl Fn(u64) -> bool) -> Option<u64> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.minimised && candidate(entry.window))
            .map(|entry| entry.window)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn prune(&mut self, alive: &impl Fn(u64) -> bool) {
        self.entries.retain(|entry| alive(entry.window));
    }
}

/// Which way to cycle through an application's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    Next,
    Previous,
}

/// What to do to move focus one window along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleStep<T> {
    /// The window to bring to the front.
    pub activate: T,
    /// Put the window that was current directly behind this one, which is the
    /// application's backmost window.
    pub sink_behind: Option<T>,
}

/// One step through `windows`, an application's windows in z-order, front
/// first, from `current`.
///
/// Z-order alone does not cycle: bringing the second window forward makes
/// the first one second, and "next" again would only swap the two. So
/// [`Cycle::Next`] activates the window behind `current` *and* sinks
/// `current` to the back of the application's windows, which visits every
/// window in turn, as Alt+Esc does. [`Cycle::Previous`] activates the
/// backmost window, which goes the other way round the same circle with no
/// sinking at all. Both wrap.
///
/// `None` when there is nothing to move to: fewer than two windows, or
/// `current` not among them.
pub fn cycle<T: Copy + PartialEq>(
    windows: &[T],
    current: T,
    direction: Cycle,
) -> Option<CycleStep<T>> {
    let count = windows.len();
    if count < 2 {
        return None;
    }
    let index = windows.iter().position(|&window| window == current)?;
    let last = windows[count - 1];

    Some(match direction {
        Cycle::Next => CycleStep {
            activate: windows[(index + 1) % count],
            sink_behind: (index != count - 1).then_some(last),
        },
        Cycle::Previous => CycleStep {
            activate: windows[(index + count - 1) % count],
            sink_behind: None,
        },
    })
}

/// What the platform knows about a top-level window, for deciding whether
/// a window command may touch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowFacts {
    pub visible: bool,
    /// Hidden by the compositor though nominally visible: on Windows a
    /// window on another virtual desktop, or a suspended packaged app.
    pub cloaked: bool,
    /// A palette, a floating toolbar: never in the task switcher.
    pub tool_window: bool,
    /// Owned by another window: a dialog, which goes where its owner goes.
    pub owned: bool,
    /// One of Lumen's own windows.
    pub ours: bool,
    /// The desktop or the taskbar.
    pub shell: bool,
    pub titled: bool,
    pub minimised: bool,
}

/// Whether a window is one a user would switch to: what the task switcher
/// lists. The windows "focus next" cycles through, minimised ones included.
pub fn switchable(facts: WindowFacts) -> bool {
    facts.visible
        && !facts.cloaked
        && !facts.tool_window
        && !facts.owned
        && !facts.ours
        && !facts.shell
        && facts.titled
}

/// Whether "minimise this display" minimises a window, before asking which
/// monitor it is on: a switchable window that is not minimised already.
pub fn minimisable(facts: WindowFacts) -> bool {
    switchable(facts) && !facts.minimised
}

#[cfg(test)]
mod tests {
    use super::*;

    fn everything_alive(_: u64) -> bool {
        true
    }

    fn popped<T>(history: &mut History<T>, window: u64, alive: impl Fn(u64) -> bool) -> Option<T> {
        history.pop(window, alive).map(|recorded| recorded.saved)
    }

    #[test]
    fn popping_says_whether_lumen_minimised_it() {
        let mut history = History::default();
        history.record(1, 'a', true, everything_alive);
        history.record(2, 'b', false, everything_alive);
        assert_eq!(
            history.pop(1, everything_alive),
            Some(Recorded {
                saved: 'a',
                minimised: true
            })
        );
        assert!(!history.pop(2, everything_alive).unwrap().minimised);
    }

    #[test]
    fn a_recorded_window_pops_once() {
        let mut history = History::default();
        history.record(1, "before", false, everything_alive);

        assert_eq!(popped(&mut history, 1, everything_alive), Some("before"));
        assert_eq!(popped(&mut history, 1, everything_alive), None);
        assert_eq!(popped(&mut history, 2, everything_alive), None);
    }

    /// Restore goes back to before the last arrangement, not the first.
    #[test]
    fn recording_again_replaces_and_does_not_duplicate() {
        let mut history = History::default();
        history.record(1, "first", false, everything_alive);
        history.record(2, "other", false, everything_alive);
        history.record(1, "second", false, everything_alive);

        assert_eq!(history.len(), 2);
        assert_eq!(popped(&mut history, 1, everything_alive), Some("second"));
    }

    #[test]
    fn the_history_is_bounded_and_forgets_the_oldest() {
        let mut history = History::with_limit(3);
        for window in 1..=5 {
            history.record(window, window * 10, false, everything_alive);
        }

        assert_eq!(history.len(), 3);
        assert_eq!(popped(&mut history, 1, everything_alive), None);
        assert_eq!(popped(&mut history, 2, everything_alive), None);
        assert_eq!(popped(&mut history, 3, everything_alive), Some(30));
        assert_eq!(popped(&mut history, 5, everything_alive), Some(50));
    }

    /// Re-recording a window refreshes it: it is the most recent again, so
    /// the bound forgets someone else first.
    #[test]
    fn recording_again_makes_a_window_the_most_recent() {
        let mut history = History::with_limit(2);
        history.record(1, 'a', false, everything_alive);
        history.record(2, 'b', false, everything_alive);
        history.record(1, 'c', false, everything_alive);
        history.record(3, 'd', false, everything_alive);

        assert_eq!(popped(&mut history, 2, everything_alive), None);
        assert_eq!(popped(&mut history, 1, everything_alive), Some('c'));
    }

    #[test]
    fn closed_windows_are_dropped() {
        let mut history = History::default();
        history.record(1, (), false, everything_alive);
        history.record(2, (), false, everything_alive);
        history.record(3, (), true, everything_alive);

        // Window 2 closed; its handle must not be restored onto anything.
        let alive = |window: u64| window != 2;
        history.record(4, (), false, alive);
        assert_eq!(history.len(), 3);
        assert_eq!(popped(&mut history, 2, everything_alive), None);

        // Popping prunes too.
        let alive = |window: u64| window == 4;
        assert_eq!(popped(&mut history, 4, alive), Some(()));
        assert!(history.is_empty());
    }

    #[test]
    fn a_dead_window_is_not_popped() {
        let mut history = History::default();
        history.record(7, "x", false, everything_alive);
        assert_eq!(popped(&mut history, 7, |_| false), None);
        assert!(history.is_empty());
    }

    #[test]
    fn the_latest_minimised_is_the_most_recent_still_eligible() {
        let mut history = History::default();
        history.record(1, (), true, everything_alive);
        history.record(2, (), false, everything_alive);
        history.record(3, (), true, everything_alive);
        history.record(4, (), false, everything_alive);

        assert_eq!(history.latest_minimised(everything_alive), Some(3));
        // Restored by hand since, or closed: the one before it.
        assert_eq!(history.latest_minimised(|window| window != 3), Some(1));
        assert_eq!(history.latest_minimised(|_| false), None);
    }

    /// An arrangement after a minimise replaces it: the window is no longer
    /// one Lumen minimised.
    #[test]
    fn arranging_a_minimised_window_clears_its_minimised_mark() {
        let mut history = History::default();
        history.record(1, (), true, everything_alive);
        history.record(1, (), false, everything_alive);
        assert_eq!(history.latest_minimised(everything_alive), None);
    }

    #[test]
    fn next_visits_every_window_and_wraps() {
        // Simulate the z-order: activate brings to front, sink puts the
        // current window behind the named one.
        let mut z = vec!['a', 'b', 'c'];
        let mut visited = Vec::new();
        for _ in 0..3 {
            let current = z[0];
            let step = cycle(&z, current, Cycle::Next).unwrap();
            if let Some(behind) = step.sink_behind {
                z.retain(|&w| w != current);
                let at = z.iter().position(|&w| w == behind).unwrap();
                z.insert(at + 1, current);
            }
            z.retain(|&w| w != step.activate);
            z.insert(0, step.activate);
            visited.push(step.activate);
        }
        assert_eq!(visited, ['b', 'c', 'a']);
    }

    #[test]
    fn previous_goes_the_other_way_round() {
        let mut z = vec!['a', 'b', 'c'];
        let mut visited = Vec::new();
        for _ in 0..3 {
            let step = cycle(&z, z[0], Cycle::Previous).unwrap();
            assert_eq!(step.sink_behind, None);
            z.retain(|&w| w != step.activate);
            z.insert(0, step.activate);
            visited.push(step.activate);
        }
        assert_eq!(visited, ['c', 'b', 'a']);
    }

    #[test]
    fn cycling_wraps_from_either_end() {
        let z = [1, 2, 3];
        assert_eq!(
            cycle(&z, 3, Cycle::Next),
            Some(CycleStep {
                activate: 1,
                sink_behind: None
            })
        );
        assert_eq!(cycle(&z, 1, Cycle::Previous).unwrap().activate, 3);
        assert_eq!(cycle(&z, 2, Cycle::Previous).unwrap().activate, 1);
    }

    #[test]
    fn cycling_needs_two_windows_and_the_current_one() {
        assert_eq!(cycle(&[1], 1, Cycle::Next), None);
        assert_eq!(cycle::<u64>(&[], 1, Cycle::Previous), None);
        assert_eq!(cycle(&[1, 2], 9, Cycle::Next), None);
    }

    fn ordinary() -> WindowFacts {
        WindowFacts {
            visible: true,
            titled: true,
            ..WindowFacts::default()
        }
    }

    #[test]
    fn an_ordinary_window_is_minimised() {
        assert!(minimisable(ordinary()));
        assert!(switchable(ordinary()));
    }

    #[test]
    fn minimise_display_skips_what_is_not_a_users_window() {
        let skipped = [
            WindowFacts {
                visible: false,
                ..ordinary()
            },
            WindowFacts {
                cloaked: true,
                ..ordinary()
            },
            WindowFacts {
                tool_window: true,
                ..ordinary()
            },
            WindowFacts {
                owned: true,
                ..ordinary()
            },
            WindowFacts {
                ours: true,
                ..ordinary()
            },
            WindowFacts {
                shell: true,
                ..ordinary()
            },
            WindowFacts {
                titled: false,
                ..ordinary()
            },
            WindowFacts {
                minimised: true,
                ..ordinary()
            },
        ];
        for facts in skipped {
            assert!(!minimisable(facts), "{facts:?}");
        }
    }

    /// Focus cycling reaches a minimised window; minimising does not
    /// minimise it twice.
    #[test]
    fn a_minimised_window_is_switchable_but_not_minimisable() {
        let minimised = WindowFacts {
            minimised: true,
            ..ordinary()
        };
        assert!(switchable(minimised));
        assert!(!minimisable(minimised));
    }
}
