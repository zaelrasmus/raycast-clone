//! Custom window layouts (F17, 7.60): several applications placed at once.
//!
//! The model, and the choices that need no system: which running window each
//! entry takes, which apps have to be launched, what a window on screen is as
//! a [`Placement`], and which other windows *Minimise Others* or *Close
//! Others* touch. The platform reads the windows and makes the calls.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::tiling::{Anchor, Length, Offset, Placement};

/// At most this many entries on one display, as in Raycast.
pub const MAX_PER_DISPLAY: usize = 8;

/// A layout: the apps, where each goes, and what happens to the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WindowLayoutSpec {
    pub entries: Vec<LayoutEntry>,
    /// Whether the configured screen gap is left round each placement.
    #[serde(default = "yes")]
    pub gaps: bool,
    #[serde(default)]
    pub other_windows: OtherWindows,
}

fn yes() -> bool {
    true
}

/// One application's window in a layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutEntry {
    pub app: AppTarget,
    #[serde(default)]
    pub argument: Argument,
    /// Index in the platform's monitor order, left to right; one that is not
    /// attached is the primary monitor.
    #[serde(default)]
    pub display: u32,
    pub placement: Placement,
}

/// Which application: what launches it, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTarget {
    /// An executable's path, or a packaged app's `shell:AppsFolder\<id>` as
    /// the application index has it.
    pub path: String,
    pub name: String,
}

impl AppTarget {
    /// Two entries with the same key are the same application.
    pub fn key(&self) -> String {
        self.path.to_lowercase()
    }
}

/// What the application is started with.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Argument {
    #[default]
    None,
    Url(String),
    Path(String),
    /// Passed as written.
    Args(String),
}

impl Argument {
    /// The command line it adds, `None` when it adds nothing.
    pub fn command_line(&self) -> Option<String> {
        let line = match self {
            Self::None => return None,
            Self::Url(text) | Self::Args(text) => text.trim().to_string(),
            Self::Path(path) => format!("\"{}\"", path.trim().trim_matches('"')),
        };
        (!line.is_empty() && line != "\"\"").then_some(line)
    }
}

/// What happens to the windows that are not in the layout, on the displays
/// it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OtherWindows {
    #[default]
    None,
    Minimize,
    /// Asked to close, as their own close button would: a window with unsaved
    /// work still asks.
    Close,
}

/// What applying a layout did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutReport {
    pub placed: usize,
    pub launched: usize,
    /// The names of the apps whose window never came.
    pub missing: Vec<String>,
}

impl WindowLayoutSpec {
    /// Whether it can be applied: at most [`MAX_PER_DISPLAY`] entries on a
    /// display, each naming an application.
    pub fn validate(&self) -> Result<(), String> {
        if self
            .entries
            .iter()
            .any(|entry| entry.app.path.trim().is_empty())
        {
            return Err("an entry names no application".into());
        }
        let mut displays: Vec<u32> = self.entries.iter().map(|entry| entry.display).collect();
        displays.sort_unstable();
        match displays
            .chunk_by(|a, b| a == b)
            .find(|run| run.len() > MAX_PER_DISPLAY)
        {
            Some(run) => Err(format!(
                "display {} has {} windows; a layout places at most {MAX_PER_DISPLAY} on one",
                run[0],
                run.len()
            )),
            None => Ok(()),
        }
    }
}

/// Which monitor `display` names among `count`, `primary` when it is none.
pub fn display_index(display: u32, count: usize, primary: usize) -> usize {
    match display as usize {
        index if index < count => index,
        _ => primary,
    }
}

/// The running window each entry takes, `None` for the entries to launch.
///
/// `running` gives an application's windows in z-order, front first. An
/// entry with an argument always launches, to hand the argument over; the
/// others take the frontmost window of their app no earlier entry took, so
/// two entries of one app take two windows.
pub fn assign<W: Copy + PartialEq>(
    entries: &[LayoutEntry],
    running: impl Fn(&AppTarget) -> Vec<W>,
) -> Vec<Option<W>> {
    let mut taken: Vec<W> = Vec::new();
    entries
        .iter()
        .map(|entry| {
            if entry.argument.command_line().is_some() {
                return None;
            }
            let window = running(&entry.app)
                .into_iter()
                .find(|window| !taken.contains(window))?;
            taken.push(window);
            Some(window)
        })
        .collect()
}

/// `window` as a placement on a monitor whose work area is `work_area`: its
/// size and its distance from the top-left corner, in percentages, so it
/// lands in the same place on a display of another size.
pub fn percent_placement(window: Rect, work_area: Rect) -> Placement {
    let hundredths = |part: i32, whole: i32| {
        (part as f64 * 10_000.0 / whole.max(1) as f64)
            .round()
            .clamp(-10_000.0, 10_000.0) as i32
    };
    let size = |part: i32, whole: i32| Length::Percent(hundredths(part, whole).max(1));
    Placement {
        width: size(window.width, work_area.width),
        height: size(window.height, work_area.height),
        x: Offset::Percent(hundredths(window.x - work_area.x, work_area.width)),
        y: Offset::Percent(hundredths(window.y - work_area.y, work_area.height)),
        anchor: Anchor::TopLeft,
        gaps: false,
    }
}

/// A window on screen, for [`others`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen<W> {
    pub window: W,
    /// Index in the platform's monitor order.
    pub display: usize,
    /// Whether the window filter lets it be touched.
    pub eligible: bool,
}

/// The windows *Minimise Others* or *Close Others* touch: eligible, on one of
/// `displays`, and not `placed`.
pub fn others<W: Copy + PartialEq>(
    windows: &[Seen<W>],
    displays: &[usize],
    placed: &[W],
) -> Vec<W> {
    windows
        .iter()
        .filter(|seen| seen.eligible && displays.contains(&seen.display))
        .filter(|seen| !placed.contains(&seen.window))
        .map(|seen| seen.window)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, display: u32) -> LayoutEntry {
        LayoutEntry {
            app: AppTarget {
                path: path.into(),
                name: path.into(),
            },
            argument: Argument::None,
            display,
            placement: Placement {
                width: Length::Percent(5000),
                height: Length::Percent(10_000),
                x: Offset::Points(0),
                y: Offset::Points(0),
                anchor: Anchor::Left,
                gaps: false,
            },
        }
    }

    fn spec(entries: Vec<LayoutEntry>) -> WindowLayoutSpec {
        WindowLayoutSpec {
            entries,
            gaps: true,
            other_windows: OtherWindows::Minimize,
        }
    }

    #[test]
    fn nine_entries_on_one_display_are_refused() {
        let eight: Vec<LayoutEntry> = (0..8).map(|i| entry(&format!("{i}.exe"), 0)).collect();
        assert!(spec(eight.clone()).validate().is_ok());

        let mut nine = eight.clone();
        nine.push(entry("8.exe", 0));
        let error = spec(nine).validate().unwrap_err();
        assert!(error.contains("8"), "{error}");

        // Eight on each of two displays is fine.
        let mut sixteen = eight;
        sixteen.extend((0..8).map(|i| entry(&format!("{i}.exe"), 1)));
        assert!(spec(sixteen).validate().is_ok());

        assert!(spec(vec![entry(" ", 0)]).validate().is_err());
    }

    #[test]
    fn an_absent_display_is_the_primary() {
        assert_eq!(display_index(1, 2, 0), 1);
        assert_eq!(display_index(2, 2, 1), 1);
        assert_eq!(display_index(0, 3, 2), 0);
    }

    #[test]
    fn an_argument_is_its_command_line() {
        assert_eq!(Argument::None.command_line(), None);
        assert_eq!(Argument::Args("  ".into()).command_line(), None);
        assert_eq!(
            Argument::Path(r"C:\My Notes".into())
                .command_line()
                .as_deref(),
            Some(r#""C:\My Notes""#)
        );
        assert_eq!(
            Argument::Url("https://x.com".into())
                .command_line()
                .as_deref(),
            Some("https://x.com")
        );
    }

    #[test]
    fn two_entries_of_one_app_take_two_windows() {
        let entries = vec![
            entry(r"C:\Code\Code.exe", 0),
            entry("term.exe", 0),
            entry(r"c:\code\CODE.exe", 1),
            entry(r"C:\Code\Code.exe", 1),
        ];
        let running = |app: &AppTarget| match app.key().as_str() {
            r"c:\code\code.exe" => vec![10, 11],
            _ => vec![],
        };
        assert_eq!(
            assign(&entries, running),
            [Some(10), None, Some(11), None],
            "the third takes the second window, the fourth is launched"
        );
    }

    #[test]
    fn an_entry_with_an_argument_launches_even_when_running() {
        let mut entries = vec![entry("browser.exe", 0), entry("browser.exe", 0)];
        entries[0].argument = Argument::Url("https://x.com".into());
        assert_eq!(assign(&entries, |_| vec![1, 2]), [None, Some(1)]);
    }

    #[test]
    fn a_window_becomes_percentages_of_its_work_area() {
        let area = Rect::new(0, 40, 2000, 1000);
        let window = Rect::new(500, 290, 1000, 500);
        let placement = percent_placement(window, area);
        assert_eq!(placement.width, Length::Percent(5000));
        assert_eq!(placement.height, Length::Percent(5000));
        assert_eq!(placement.x, Offset::Percent(2500));
        assert_eq!(placement.y, Offset::Percent(2500));
        assert_eq!(placement.anchor, Anchor::TopLeft);
        assert!(!placement.gaps);

        // And back, within a pixel, on a display of another size.
        let odd = Rect::new(-1920, 0, 1920, 1032);
        let window = Rect::new(-1713, 17, 777, 333);
        let back = percent_placement(window, odd).place(odd, window, 1.0, Default::default());
        for (a, b) in [
            (back.x, window.x),
            (back.y, window.y),
            (back.width, window.width),
            (back.height, window.height),
        ] {
            assert!((a - b).abs() <= 1, "{back:?} vs {window:?}");
        }
    }

    #[test]
    fn a_window_past_the_edge_is_kept_in_range() {
        let area = Rect::new(0, 0, 1000, 1000);
        let maximised = Rect::new(-8, -8, 1016, 1016);
        let placement = percent_placement(maximised, area);
        assert_eq!(placement.width, Length::Percent(10_000));
        assert_eq!(placement.x, Offset::Percent(-80));
        let far = percent_placement(Rect::new(3000, 0, 10, 10), area);
        assert_eq!(far.x, Offset::Percent(10_000));
    }

    #[test]
    fn others_are_eligible_windows_on_used_displays_not_placed() {
        let seen = |window, display, eligible| Seen {
            window,
            display,
            eligible,
        };
        let windows = [
            seen(1, 0, true),
            seen(2, 0, true),
            seen(3, 0, false),
            seen(4, 1, true),
            seen(5, 2, true),
        ];
        assert_eq!(others(&windows, &[0, 2], &[1]), [2, 5]);
        assert!(others(&windows, &[], &[]).is_empty());
    }
}
