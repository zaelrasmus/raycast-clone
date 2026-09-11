//! Shortcut sheets (tier T0).
//!
//! A sheet is a data file, exactly like a theme: same distribution model, no
//! code, no runtime, nothing to review. Dropping one into the sheets folder adds
//! it, and the same versioned-schema and default-resolution discipline applies
//! (AD-12, I-12).
//!
//! # What makes this different from the PowerToys equivalent
//!
//! Two things, per F3. The shortcuts are *the user's own*, annotated by them,
//! not the operating system's fixed set. And the sheet shown depends on the
//! foreground application.
//!
//! # The fallback chain is mandatory
//!
//! Executable, then a pattern on the window title, then the global sheet. The
//! plan states the requirement as an absolute: **the overlay never appears
//! empty**. An overlay that sometimes comes up blank teaches the user that the
//! gesture is unreliable, and they stop using it — so [`SheetLibrary::resolve`]
//! returns a sheet rather than an `Option`, and the type system carries the
//! guarantee.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use lumen_platform::ForegroundApp;
use serde::{Deserialize, Serialize};

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: u32 = 1;

/// How a sheet decides whether it applies to the foreground application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "kebab-case")]
pub enum MatchRule {
    /// Matches the foreground executable's file name, case-insensitively:
    /// `"code.exe"`, `"firefox.exe"`. The most specific rung.
    Executable { any: Vec<String> },
    /// Matches when the window title contains any of these, case-insensitively.
    ///
    /// The rung that exists because of R-14: a browser or an Electron
    /// application reports its container and not its content, so the title is
    /// the only thing left to key on.
    TitleContains { any: Vec<String> },
    /// The last resort, shown when nothing more specific matched.
    Global,
}

impl MatchRule {
    /// Lower is more specific. Used to order the chain.
    fn specificity(&self) -> u8 {
        match self {
            MatchRule::Executable { .. } => 0,
            MatchRule::TitleContains { .. } => 1,
            MatchRule::Global => 2,
        }
    }

    fn matches(&self, app: &ForegroundApp) -> bool {
        match self {
            MatchRule::Executable { any } => {
                let Some(executable) = app.executable.as_deref() else {
                    return false;
                };
                any.iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(executable))
            }
            MatchRule::TitleContains { any } => {
                let title = app.title.to_lowercase();
                any.iter()
                    .any(|needle| !needle.is_empty() && title.contains(&needle.to_lowercase()))
            }
            MatchRule::Global => true,
        }
    }
}

/// One labelled shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// The chord as the user thinks of it: `"Ctrl+Shift+P"`.
    ///
    /// Free text on purpose. A sheet describes shortcuts belonging to *other*
    /// applications, which Lumen cannot parse, validate or intercept — and
    /// pretending otherwise would mean rejecting perfectly good sheets.
    pub keys: String,
    pub description: String,
}

/// A titled block of shortcuts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub title: String,
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
}

/// A sheet, as it appears on disk and as it is used.
///
/// Unlike a theme there is no separate resolved form: a sheet has no tokens to
/// inherit, so the file *is* the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sheet {
    pub schema_version: u32,
    /// Shown as the overlay's heading: "Visual Studio Code", "Global".
    pub name: String,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub groups: Vec<Group>,
}

impl Sheet {
    pub fn shortcut_count(&self) -> usize {
        self.groups.iter().map(|group| group.shortcuts.len()).sum()
    }

    fn is_empty(&self) -> bool {
        self.shortcut_count() == 0
    }

    /// How many columns this sheet actually uses, given a maximum.
    ///
    /// A sheet with two groups gets two columns, not three with one left empty:
    /// the panel's width follows from this, so a short sheet is a small panel
    /// rather than a wide one with a hole in it.
    pub fn effective_columns(&self, max: usize) -> usize {
        max.min(self.groups.len()).max(1)
    }

    /// Distribute the groups across `columns`, returning group indices per
    /// column.
    ///
    /// The window has to be sized before it is shown, and the animation rules
    /// forbid animating width or height afterwards -- so the measurement and the
    /// rendering must agree exactly. They agree because both call this: the
    /// layout is computed once and used twice, rather than computed twice and
    /// hoped to match.
    ///
    /// Greedy shortest-column packing, in declaration order. Not optimal
    /// bin-packing, and deliberately so: the author controls the order groups
    /// appear in their own file, and an optimiser that reorders them would take
    /// that control away to save a few pixels.
    pub fn columns(&self, columns: usize) -> Vec<Vec<usize>> {
        let columns = self.effective_columns(columns);
        let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); columns];
        let mut heights = vec![0.0f32; columns];

        for (index, group) in self.groups.iter().enumerate() {
            let shortest = heights
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(index, _)| index)
                .unwrap_or(0);

            buckets[shortest].push(index);
            heights[shortest] += group_height(group);
        }

        buckets
    }

    /// The panel's logical height for a given column count.
    pub fn measured_height(&self, columns: usize) -> f32 {
        use crate::layout::overlay;

        let tallest = self
            .columns(columns)
            .iter()
            .map(|column| {
                column
                    .iter()
                    .map(|&index| group_height(&self.groups[index]))
                    .sum::<f32>()
            })
            .fold(0.0f32, f32::max);

        // The trailing gap after the last group in a column is not drawn, so it
        // must not be paid for in the height either -- that is the one-gap
        // sliver of dead space at the bottom that makes a panel look untidy.
        let body = (tallest - overlay::GROUP_GAP).max(0.0);

        overlay::HEADER_HEIGHT + body + overlay::PADDING * 2.0
    }

    /// Write this sheet to `path` as pretty JSON.
    ///
    /// Pretty rather than compact because the file is meant to be edited by
    /// hand as well as by the settings window, and a sheet saved from the UI
    /// must not become harder to read than one the user wrote themselves.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let source = serde_json::to_string_pretty(self)?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }

    /// A new, empty sheet keyed on `executable`.
    pub fn for_executable(name: &str, executable: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: name.to_string(),
            match_rule: MatchRule::Executable {
                any: vec![executable.to_lowercase()],
            },
            groups: vec![Group {
                title: "Shortcuts".into(),
                shortcuts: Vec::new(),
            }],
        }
    }

    /// A file name for this sheet, derived from its name.
    ///
    /// Reduced to lowercase ASCII words joined by hyphens, because the name is
    /// free text and may contain anything at all -- including path separators,
    /// which would otherwise let a sheet called `../../evil` write outside the
    /// sheets folder.
    pub fn file_stem(&self) -> String {
        let stem: String = self
            .name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();

        let stem = stem.trim_matches('-').to_string();
        let collapsed = stem
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");

        if collapsed.is_empty() {
            "sheet".to_string()
        } else {
            collapsed
        }
    }

    /// The panel's logical width, which narrows with the column count.
    ///
    /// A column keeps the width it would have had in a full-width panel, so
    /// sheets of different sizes still line their text up identically -- only
    /// the number of columns changes, never their proportions.
    pub fn measured_width(&self, max_columns: usize) -> f32 {
        use crate::layout::overlay;

        let used = self.effective_columns(max_columns) as f32;
        let max = max_columns.max(1) as f32;

        let inner = overlay::WIDTH - overlay::PADDING * 2.0;
        let column = (inner - (max - 1.0) * overlay::COLUMN_GAP) / max;

        overlay::PADDING * 2.0 + used * column + (used - 1.0) * overlay::COLUMN_GAP
    }
}

/// A group's height including the gap that follows it.
fn group_height(group: &Group) -> f32 {
    use crate::layout::overlay;

    overlay::GROUP_TITLE_HEIGHT
        + group.shortcuts.len() as f32 * overlay::ROW_HEIGHT
        + overlay::GROUP_GAP
}

/// Every sheet available, ordered so that resolution is a linear scan.
#[derive(Debug, Default)]
pub struct SheetLibrary {
    entries: Vec<SheetEntry>,
}

/// A sheet plus where it came from.
///
/// The origin matters because the settings editor writes sheets back: a sheet
/// loaded from a file can be saved to that file, while the generated Lumen sheet
/// has no file and must be presented as read-only rather than silently failing
/// to save.
#[derive(Debug, Clone)]
pub struct SheetEntry {
    pub sheet: Sheet,
    pub path: Option<PathBuf>,
}

impl SheetEntry {
    /// Whether the settings editor may change this sheet.
    pub fn is_editable(&self) -> bool {
        self.path.is_some()
    }
}

impl SheetLibrary {
    /// Load every `*.json` in `dir`.
    ///
    /// A malformed sheet is reported and skipped rather than fatal. One bad file
    /// must not cost the user every other sheet they have written.
    pub fn load(dir: &Path) -> Self {
        let mut library = Self::default();

        let Ok(entries) = std::fs::read_dir(dir) else {
            return library;
        };

        // Sorted so the order in the settings list is stable between runs,
        // rather than whatever order the filesystem happened to hand over.
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect();
        paths.sort();

        for path in paths {
            match read_sheet(&path) {
                Ok(sheet) => library.insert(sheet, Some(path)),
                Err(error) => log::warn!("ignoring shortcut sheet: {error:#}"),
            }
        }

        library
    }

    /// Add a sheet that has no file behind it, such as the generated one.
    pub fn add(&mut self, sheet: Sheet) {
        self.insert(sheet, None);
    }

    /// Add a sheet, keeping the collection ordered most-specific first.
    ///
    /// An empty sheet is dropped from *matching*: it could win the chain over a
    /// less specific sheet that actually has content, and produce exactly the
    /// blank overlay the fallback chain exists to prevent. It is still listed
    /// for editing, because a sheet the user has just created and not yet filled
    /// in has to be visible somewhere or it cannot be filled in at all.
    pub fn insert(&mut self, sheet: Sheet, path: Option<PathBuf>) {
        self.entries.push(SheetEntry { sheet, path });
        self.entries
            .sort_by_key(|entry| entry.sheet.match_rule.specificity());
    }

    /// The number of sheets that can actually be shown.
    pub fn len(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| !entry.sheet.is_empty())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every sheet, including empty ones, for the settings editor.
    pub fn entries(&self) -> &[SheetEntry] {
        &self.entries
    }

    /// Replace the sheet at `index` and re-sort, after an edit.
    pub fn replace(&mut self, index: usize, sheet: Sheet) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.sheet = sheet;
        }
        self.entries
            .sort_by_key(|entry| entry.sheet.match_rule.specificity());
    }

    /// Forget the sheet at `index`. Does not touch the file.
    pub fn remove(&mut self, index: usize) -> Option<SheetEntry> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }

    /// The sheet to show for `app`, following the mandatory fallback chain.
    ///
    /// Never returns nothing. When no sheet matches and no global sheet was
    /// supplied, the built-in one is used, so the overlay always has content.
    pub fn resolve<'a>(&'a self, app: Option<&ForegroundApp>, fallback: &'a Sheet) -> &'a Sheet {
        // Empty sheets are skipped throughout: they would satisfy the chain
        // without satisfying the user.
        let usable = || {
            self.entries
                .iter()
                .map(|entry| &entry.sheet)
                .filter(|sheet| !sheet.is_empty())
        };

        if let Some(app) = app {
            for sheet in usable() {
                if sheet.match_rule.matches(app) {
                    return sheet;
                }
            }
        }

        // With no foreground application at all, a global sheet is still the
        // right answer -- the user asked to see their shortcuts.
        usable()
            .find(|sheet| sheet.match_rule == MatchRule::Global)
            .unwrap_or(fallback)
    }
}

fn read_sheet(path: &Path) -> Result<Sheet> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let sheet: Sheet =
        serde_json::from_str(&source).with_context(|| format!("parsing {}", path.display()))?;

    if sheet.schema_version > SCHEMA_VERSION {
        // Same reasoning as themes (I-12): a file from a later version loads,
        // because refusing it is the breakage the versioned schema exists to
        // replace.
        log::warn!(
            "shortcut sheet {:?} declares schema version {} but this build understands \
             {SCHEMA_VERSION}; unknown fields are ignored",
            sheet.name,
            sheet.schema_version
        );
    }

    Ok(sheet)
}

/// The sheet shown when the user has written none and nothing matched.
///
/// Compiled in rather than loaded, because the guarantee that the overlay is
/// never empty cannot depend on a file existing on disk.
pub fn builtin_global() -> Sheet {
    serde_json::from_str(include_str!("../../../assets/sheets/global.json"))
        .expect("the built-in global sheet is validated by a test")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in fallback, bound once so tests can borrow it.
    static FALLBACK: std::sync::LazyLock<Sheet> = std::sync::LazyLock::new(builtin_global);

    fn app(executable: &str, title: &str) -> ForegroundApp {
        ForegroundApp {
            executable: Some(executable.to_string()),
            title: title.to_string(),
        }
    }

    fn sheet(name: &str, rule: MatchRule) -> Sheet {
        Sheet {
            schema_version: 1,
            name: name.into(),
            match_rule: rule,
            groups: vec![Group {
                title: "Editing".into(),
                shortcuts: vec![Shortcut {
                    keys: "Ctrl+P".into(),
                    description: "Go to file".into(),
                }],
            }],
        }
    }

    fn library() -> SheetLibrary {
        let mut library = SheetLibrary::default();
        // Added out of order on purpose: resolution must not depend on the
        // order the filesystem happened to hand the files over in.
        library.add(sheet("Global", MatchRule::Global));
        library.add(sheet(
            "Browsers",
            MatchRule::TitleContains {
                any: vec!["Mozilla Firefox".into()],
            },
        ));
        library.add(sheet(
            "Code",
            MatchRule::Executable {
                any: vec!["code.exe".into()],
            },
        ));
        library
    }

    #[test]
    fn the_executable_rung_wins_first() {
        let library = library();
        let resolved = library.resolve(
            Some(&app("code.exe", "main.rs - Mozilla Firefox")),
            &FALLBACK,
        );
        // The title would also have matched the browser sheet; the executable
        // is more specific and must take precedence.
        assert_eq!(resolved.name, "Code");
    }

    #[test]
    fn the_title_rung_catches_what_the_executable_cannot() {
        let library = library();
        let resolved = library.resolve(
            Some(&app("firefox.exe", "Rust docs — Mozilla Firefox")),
            &FALLBACK,
        );
        assert_eq!(resolved.name, "Browsers");
    }

    #[test]
    fn unknown_applications_fall_through_to_global() {
        let library = library();
        let resolved = library.resolve(Some(&app("notepad.exe", "Untitled")), &FALLBACK);
        assert_eq!(resolved.name, "Global");
    }

    /// The requirement stated as an absolute in F3.
    #[test]
    fn the_overlay_never_resolves_to_nothing() {
        let empty = SheetLibrary::default();

        let resolved = empty.resolve(Some(&app("anything.exe", "whatever")), &FALLBACK);
        assert!(resolved.shortcut_count() > 0);

        // ...and with no foreground application at all.
        let resolved = empty.resolve(None, &FALLBACK);
        assert!(resolved.shortcut_count() > 0);
    }

    #[test]
    fn executable_matching_ignores_case() {
        let library = library();
        let resolved = library.resolve(Some(&app("CODE.EXE", "x")), &FALLBACK);
        assert_eq!(resolved.name, "Code");
    }

    #[test]
    fn title_matching_ignores_case() {
        let library = library();
        let resolved = library.resolve(
            Some(&app("unknown.exe", "something - MOZILLA FIREFOX")),
            &FALLBACK,
        );
        assert_eq!(resolved.name, "Browsers");
    }

    /// An elevated process cannot be opened, so the executable is unknown. The
    /// chain must still produce something rather than falling over.
    #[test]
    fn a_missing_executable_still_resolves() {
        let library = library();
        let unknown = ForegroundApp {
            executable: None,
            title: "Administrator: Console".into(),
        };
        assert_eq!(library.resolve(Some(&unknown), &FALLBACK).name, "Global");
    }

    /// An empty sheet must never win the chain, or it produces the blank
    /// overlay the whole fallback design exists to prevent.
    #[test]
    fn empty_sheets_are_refused() {
        let mut library = SheetLibrary::default();
        library.add(Sheet {
            schema_version: 1,
            name: "Hollow".into(),
            match_rule: MatchRule::Executable {
                any: vec!["code.exe".into()],
            },
            groups: vec![Group {
                title: "Nothing".into(),
                shortcuts: Vec::new(),
            }],
        });

        assert!(library.is_empty());
        let resolved = library.resolve(Some(&app("code.exe", "x")), &FALLBACK);
        assert!(resolved.shortcut_count() > 0);
    }

    #[test]
    fn the_builtin_global_sheet_is_valid() {
        let sheet = builtin_global();
        assert_eq!(sheet.schema_version, SCHEMA_VERSION);
        assert_eq!(sheet.match_rule, MatchRule::Global);
        assert!(sheet.shortcut_count() > 0);
    }

    #[test]
    fn every_group_lands_in_exactly_one_column() {
        let sheet = builtin_global();
        let columns = sheet.columns(3);

        let mut placed: Vec<usize> = columns.iter().flatten().copied().collect();
        placed.sort_unstable();

        assert_eq!(placed, (0..sheet.groups.len()).collect::<Vec<_>>());
    }

    #[test]
    fn packing_prefers_the_shortest_column() {
        let sheet = Sheet {
            schema_version: 1,
            name: "Uneven".into(),
            match_rule: MatchRule::Global,
            groups: vec![
                Group {
                    title: "Big".into(),
                    shortcuts: vec![
                        Shortcut {
                            keys: "a".into(),
                            description: "a".into()
                        };
                        10
                    ],
                },
                Group {
                    title: "Small".into(),
                    shortcuts: vec![Shortcut {
                        keys: "b".into(),
                        description: "b".into(),
                    }],
                },
                Group {
                    title: "Also small".into(),
                    shortcuts: vec![Shortcut {
                        keys: "c".into(),
                        description: "c".into(),
                    }],
                },
            ],
        };

        let columns = sheet.columns(2);
        // The big group takes one column; both small ones go to the other.
        assert_eq!(columns[0], vec![0]);
        assert_eq!(columns[1], vec![1, 2]);
    }

    /// The window is sized from this before it is shown and cannot be resized
    /// afterwards, so it has to cover the tallest column, not the average one.
    #[test]
    fn the_measured_height_covers_the_tallest_column() {
        let sheet = builtin_global();
        let height = sheet.measured_height(3);

        let tallest = sheet
            .columns(3)
            .iter()
            .map(|column| {
                column
                    .iter()
                    .map(|&i| group_height(&sheet.groups[i]))
                    .sum::<f32>()
            })
            .fold(0.0f32, f32::max);

        assert!(height >= tallest, "{height} should cover {tallest}");
    }

    #[test]
    fn a_single_column_request_is_honoured() {
        let sheet = builtin_global();
        assert_eq!(sheet.columns(1).len(), 1);
        assert_eq!(sheet.columns(1)[0].len(), sheet.groups.len());
        // Zero columns is nonsense; it must not divide by zero or panic.
        assert_eq!(sheet.columns(0).len(), 1);
    }

    #[test]
    fn a_short_sheet_uses_fewer_columns_and_a_narrower_panel() {
        let two_groups = Sheet {
            schema_version: 1,
            name: "Two".into(),
            match_rule: MatchRule::Global,
            groups: vec![
                Group {
                    title: "A".into(),
                    shortcuts: vec![Shortcut {
                        keys: "a".into(),
                        description: "a".into(),
                    }],
                },
                Group {
                    title: "B".into(),
                    shortcuts: vec![Shortcut {
                        keys: "b".into(),
                        description: "b".into(),
                    }],
                },
            ],
        };

        assert_eq!(two_groups.effective_columns(3), 2);
        assert!(two_groups.measured_width(3) < crate::layout::overlay::WIDTH);
    }

    #[test]
    fn a_full_sheet_uses_the_full_width() {
        let three = builtin_global();
        assert_eq!(three.effective_columns(3), 3);
        assert!((three.measured_width(3) - crate::layout::overlay::WIDTH).abs() < 0.5);
    }

    /// One group must still produce a usable panel rather than a sliver.
    #[test]
    fn a_single_group_sheet_is_still_laid_out() {
        let one = Sheet {
            schema_version: 1,
            name: "One".into(),
            match_rule: MatchRule::Global,
            groups: vec![Group {
                title: "A".into(),
                shortcuts: vec![Shortcut {
                    keys: "a".into(),
                    description: "a".into(),
                }],
            }],
        };

        assert_eq!(one.effective_columns(3), 1);
        assert!(one.measured_width(3) > crate::layout::overlay::PADDING * 2.0);
        assert!(one.measured_height(3) > crate::layout::overlay::HEADER_HEIGHT);
    }

    /// The name is free text and reaches the filesystem, so it must not be able
    /// to escape the sheets folder.
    #[test]
    fn a_file_stem_cannot_contain_a_path() {
        let sheet = Sheet::for_executable("../../evil", "x.exe");
        let stem = sheet.file_stem();

        assert!(!stem.contains('/'), "{stem}");
        assert!(!stem.contains('\\'), "{stem}");
        assert!(!stem.contains(".."), "{stem}");
        assert_eq!(stem, "evil");
    }

    #[test]
    fn a_file_stem_is_readable() {
        assert_eq!(
            Sheet::for_executable("Visual Studio Code", "code.exe").file_stem(),
            "visual-studio-code"
        );
        assert_eq!(Sheet::for_executable("Zed", "zed.exe").file_stem(), "zed");
    }

    /// A name with nothing usable in it still has to produce a valid file name.
    #[test]
    fn an_unusable_name_still_yields_a_file_stem() {
        assert_eq!(Sheet::for_executable("???", "x.exe").file_stem(), "sheet");
        assert_eq!(Sheet::for_executable("", "x.exe").file_stem(), "sheet");
    }

    #[test]
    fn a_new_sheet_is_keyed_on_a_lowercased_executable() {
        let sheet = Sheet::for_executable("Zed", "ZED.EXE");
        assert_eq!(
            sheet.match_rule,
            MatchRule::Executable {
                any: vec!["zed.exe".into()]
            }
        );
    }

    /// A sheet the user has created but not filled in must still be listed for
    /// editing, or there is no way to fill it in.
    #[test]
    fn empty_sheets_are_listed_for_editing_but_never_matched() {
        let mut library = SheetLibrary::default();
        library.insert(
            Sheet::for_executable("New", "new.exe"),
            Some("new.json".into()),
        );

        assert_eq!(library.entries().len(), 1);
        assert!(
            library.is_empty(),
            "an empty sheet must not count as usable"
        );

        let resolved = library.resolve(Some(&app("new.exe", "x")), &FALLBACK);
        assert!(
            resolved.shortcut_count() > 0,
            "must fall through to the fallback"
        );
    }

    #[test]
    fn generated_sheets_are_not_editable() {
        let mut library = SheetLibrary::default();
        library.add(sheet("Generated", MatchRule::Global));

        assert!(!library.entries()[0].is_editable());
    }

    #[test]
    fn a_saved_sheet_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("lumen-sheets-{}", std::process::id()));
        let path = dir.join("round-trip.json");
        let original = sheet(
            "Round Trip",
            MatchRule::Executable {
                any: vec!["a.exe".into()],
            },
        );

        original.save(&path).unwrap();
        let reloaded = read_sheet(&path).unwrap();

        assert_eq!(reloaded, original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sheets_round_trip_through_json() {
        let original = sheet(
            "Code",
            MatchRule::Executable {
                any: vec!["code.exe".into()],
            },
        );
        let encoded = serde_json::to_string(&original).unwrap();
        assert_eq!(serde_json::from_str::<Sheet>(&encoded).unwrap(), original);
    }
}
