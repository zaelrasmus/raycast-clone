//! Window layouts the user made (F17, 7.60): several applications placed at
//! once.
//!
//! One file, `window-layouts.json` beside `window-commands.json`, written by
//! *Save Current Layout* and by hand:
//!
//! ```json
//! { "layouts": [ { "id": "coding-k3x9q1", "name": "Coding",
//!   "spec": { "entries": [ { "app": { "path": "C:\\...\\Code.exe", "name": "Code" },
//!     "placement": { "width": "50%", "height": "100%", "anchor": "left" } } ] } } ] }
//! ```
//!
//! The id is made once, as a window command's is (7.59): it is what
//! frecency, favourites and `shortcuts.json` hold. An entry that does not
//! read, or that [`WindowLayoutSpec::validate`] refuses, costs only itself.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::{LayoutEntry, LayoutReport, OtherWindows, Platform, WindowLayoutSpec};
use serde::{Deserialize, Serialize};

use super::window_commands::{fresh_id, replace};
use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

/// The root search id of *Save Current Layout*.
pub const SAVE_CURRENT_ID: &str = "window:save-current-layout";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomWindowLayout {
    pub id: String,
    pub name: String,
    pub spec: WindowLayoutSpec,
}

impl CustomWindowLayout {
    /// The root search id: `window:layout:<id>`.
    pub fn item_id(&self) -> String {
        format!("window:layout:{}", self.id)
    }

    fn item(&self) -> Item {
        Item {
            id: self.item_id(),
            title: self.name.clone(),
            subtitle: Some(apps(self.spec.entries.len())),
            icon: None,
            keywords: vec!["window".into(), "layout".into()],
            actions: vec![Action::ApplyWindowLayout(self.id.clone())],
            kind: Kind::Window,
        }
    }

    /// Whether it reads as a layout: an id, a name, and a spec that can be
    /// applied.
    fn check(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err("no id or name".into());
        }
        self.spec.validate()
    }
}

/// `3 apps`, `1 app`.
pub fn apps(count: usize) -> String {
    match count {
        1 => "1 app".into(),
        count => format!("{count} apps"),
    }
}

#[derive(Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    layouts: Vec<serde_json::Value>,
}

/// Read the layouts. Missing is none; an unreadable file is reported and read
/// as none; an entry that does not read, cannot be applied, or repeats an id
/// is reported and skipped.
pub fn load(path: &Path) -> Vec<CustomWindowLayout> {
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let raw: RawFile = match serde_json::from_str(&source) {
        Ok(raw) => raw,
        Err(error) => {
            log::error!("{} is not valid: {error}", path.display());
            return Vec::new();
        }
    };
    let mut layouts: Vec<CustomWindowLayout> = Vec::new();
    for entry in raw.layouts {
        match serde_json::from_value::<CustomWindowLayout>(entry) {
            Ok(layout) => match layout.check() {
                Err(why) => log::warn!("{}: skipped a layout: {why}", path.display()),
                Ok(()) if layouts.iter().any(|l| l.id == layout.id) => {
                    log::warn!("{}: skipped a second {:?}", path.display(), layout.id);
                }
                Ok(()) => layouts.push(layout),
            },
            Err(error) => log::warn!("{}: skipped a layout: {error}", path.display()),
        }
    }
    layouts
}

/// Write the layouts, beside and renamed over.
pub fn save(path: &Path, layouts: &[CustomWindowLayout]) -> Result<()> {
    write(path, serde_json::to_value(layouts)?)
}

fn write(path: &Path, layouts: serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    replace(
        path,
        serde_json::to_string_pretty(&serde_json::json!({ "layouts": layouts }))?,
    )
}

/// Change the layouts in the file and write it back, as
/// [`super::window_commands::edit`] does: read afresh, an entry that does not
/// read kept as it was after the rest, a file that is not JSON an error and
/// left alone. Whether `change` did anything.
pub fn edit(
    path: &Path,
    change: impl FnOnce(&mut Vec<CustomWindowLayout>) -> bool,
) -> Result<bool> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let raw: RawFile = match source.trim() {
        "" => RawFile::default(),
        source => serde_json::from_str(source)
            .with_context(|| format!("{} is not valid", path.display()))?,
    };
    let mut layouts: Vec<CustomWindowLayout> = Vec::new();
    let mut kept = Vec::new();
    for entry in raw.layouts {
        match serde_json::from_value::<CustomWindowLayout>(entry.clone()) {
            Ok(layout) if layout.check().is_ok() && !layouts.iter().any(|l| l.id == layout.id) => {
                layouts.push(layout)
            }
            _ => kept.push(entry),
        }
    }
    if !change(&mut layouts) {
        return Ok(false);
    }
    let mut entries = layouts
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    entries.extend(kept);
    write(path, serde_json::Value::Array(entries))?;
    Ok(true)
}

/// A new id for a layout called `name`, unlike any in `existing`.
pub fn new_id(name: &str, existing: &[CustomWindowLayout]) -> String {
    fresh_id(name, "layout", |id| {
        existing.iter().any(|layout| layout.id == id)
    })
}

/// Add a layout. Its new id.
pub fn add(layouts: &mut Vec<CustomWindowLayout>, name: &str, spec: WindowLayoutSpec) -> String {
    let id = new_id(name, layouts);
    layouts.push(CustomWindowLayout {
        id: id.clone(),
        name: name.to_string(),
        spec,
    });
    id
}

/// Rename or reshape the layout `id`, which keeps its id. Whether it exists.
pub fn update(
    layouts: &mut [CustomWindowLayout],
    id: &str,
    name: &str,
    spec: WindowLayoutSpec,
) -> bool {
    let Some(layout) = layouts.iter_mut().find(|layout| layout.id == id) else {
        return false;
    };
    layout.name = name.to_string();
    layout.spec = spec;
    true
}

/// Remove the layout `id`. Whether it existed.
pub fn remove(layouts: &mut Vec<CustomWindowLayout>, id: &str) -> bool {
    let before = layouts.len();
    layouts.retain(|layout| layout.id != id);
    layouts.len() != before
}

/// Copy the layout `id` under a new id, right after it. The copy's id.
pub fn duplicate(layouts: &mut Vec<CustomWindowLayout>, id: &str) -> Option<String> {
    let index = layouts.iter().position(|layout| layout.id == id)?;
    let name = format!("{} Copy", layouts[index].name);
    let copy = CustomWindowLayout {
        id: new_id(&name, layouts),
        name,
        spec: layouts[index].spec.clone(),
    };
    let new = copy.id.clone();
    layouts.insert(index + 1, copy);
    Some(new)
}

/// What the daemon is asked to do with layouts: the two actions, once the
/// launcher has settled them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(String),
    SaveCurrent,
}

impl Request {
    pub fn of(action: &Action) -> Option<Self> {
        match action {
            Action::ApplyWindowLayout(id) => Some(Self::Apply(id.clone())),
            Action::SaveCurrentLayout => Some(Self::SaveCurrent),
            _ => None,
        }
    }
}

/// Carry out `request` against the file at `path`. Blocking -- applying
/// waits for launched applications, five seconds at most -- so the daemon
/// calls it off the UI thread, with the launcher hidden. A notice to show
/// when there is something to say: a failure, a part done, a layout saved.
pub fn carry_out(
    platform: &dyn Platform,
    path: &Path,
    request: &Request,
) -> Option<(String, String)> {
    match request {
        Request::Apply(id) => {
            let Some(layout) = load(path).into_iter().find(|layout| &layout.id == id) else {
                return Some(applied(
                    id,
                    Err(anyhow::anyhow!("no layout {id:?} in {}", path.display())),
                ));
            };
            let notice = applied(&layout.name, platform.apply_layout(&layout.spec));
            Some(notice).filter(|(title, _)| !title.is_empty())
        }
        Request::SaveCurrent => Some(saved(path, platform.current_layout(), &now_name())),
    }
}

/// What to say after applying the layout `name`: nothing (an empty title)
/// when every window was placed.
fn applied(name: &str, outcome: Result<LayoutReport>) -> (String, String) {
    match outcome {
        Err(error) => ("Couldn't apply layout".into(), format!("{name}: {error:#}")),
        Ok(report) if report.missing.is_empty() => (String::new(), String::new()),
        Ok(report) => (
            format!("Applied {name} in part"),
            format!("No window of {}", report.missing.join(", ")),
        ),
    }
}

/// Save what is on screen as a layout called `name`.
fn saved(path: &Path, entries: Result<Vec<LayoutEntry>>, name: &str) -> (String, String) {
    let failed = |error: anyhow::Error| ("Couldn't save layout".to_string(), format!("{error:#}"));
    let entries = match entries {
        Ok(entries) if entries.is_empty() => {
            return failed(anyhow::anyhow!("there are no windows to save"));
        }
        Ok(entries) => entries,
        Err(error) => return failed(error),
    };
    let count = entries.len();
    let spec = snapshot(entries);
    match edit(path, |layouts| {
        add(layouts, name, spec);
        true
    }) {
        Ok(_) => (format!("Saved layout {name}"), apps(count)),
        Err(error) => failed(error),
    }
}

/// The windows on screen as a layout: exactly where they are, so no gaps
/// added, and nothing done to the rest.
fn snapshot(entries: Vec<LayoutEntry>) -> WindowLayoutSpec {
    WindowLayoutSpec {
        entries,
        gaps: false,
        other_windows: OtherWindows::None,
    }
}

/// `Layout 2026-10-05 14:30`, in the user's time.
fn now_name() -> String {
    name_at(&jiff::Zoned::now())
}

fn name_at(when: &jiff::Zoned) -> String {
    format!("Layout {}", when.strftime("%Y-%m-%d %H:%M"))
}

/// The layouts as root search entries, and *Save Current Layout*.
#[derive(Default)]
pub struct WindowLayoutsProvider(Vec<CustomWindowLayout>);

impl WindowLayoutsProvider {
    pub fn new(layouts: Vec<CustomWindowLayout>) -> Self {
        Self(layouts)
    }
}

impl Provider for WindowLayoutsProvider {
    fn id(&self) -> &'static str {
        "window"
    }

    fn catalogue(&self) -> Vec<Item> {
        let save = Item {
            id: SAVE_CURRENT_ID.into(),
            title: "Save Current Layout".into(),
            subtitle: Some("The windows on screen, as a layout".into()),
            icon: None,
            keywords: ["window", "layout", "save", "snapshot"]
                .map(String::from)
                .to_vec(),
            actions: vec![Action::SaveCurrentLayout],
            kind: Kind::Window,
        };
        std::iter::once(save)
            .chain(self.0.iter().map(CustomWindowLayout::item))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_platform::{Anchor, AppTarget, Argument, Length, Offset, Placement};

    fn temporary(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lumen-window-layouts-{}-{name}.json",
            std::process::id()
        ))
    }

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

    fn spec(count: usize) -> WindowLayoutSpec {
        snapshot((0..count).map(|i| entry(&format!("{i}.exe"), 0)).collect())
    }

    #[test]
    fn a_missing_file_is_no_layouts() {
        assert!(load(&temporary("missing")).is_empty());
    }

    #[test]
    fn a_bad_or_invalid_entry_costs_only_itself() {
        let path = temporary("bad-entry");
        let good = |id: &str, name: &str, count| {
            serde_json::to_value(CustomWindowLayout {
                id: id.into(),
                name: name.into(),
                spec: spec(count),
            })
            .unwrap()
        };
        let layouts = serde_json::json!([
            good("a-1", "A", 2),
            { "id": "b-1", "name": "B", "spec": { "entries": "no" } },
            good("c-1", "C", 9),
            good("d-1", "", 1),
            good("a-1", "Again", 1),
            good("e-1", "E", 1),
        ]);
        std::fs::write(&path, serde_json::json!({ "layouts": layouts }).to_string()).unwrap();
        let ids: Vec<String> = load(&path).into_iter().map(|l| l.id).collect();
        assert_eq!(ids, ["a-1", "e-1"]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn only_the_entries_and_placements_are_required() {
        let text = r#"{ "id": "x", "name": "X", "spec": { "entries": [
            { "app": { "path": "code.exe", "name": "Code" },
              "placement": { "width": "50%", "height": "100%", "anchor": "left" } } ] } }"#;
        let layout: CustomWindowLayout = serde_json::from_str(text).unwrap();
        assert!(layout.spec.gaps);
        assert_eq!(layout.spec.other_windows, OtherWindows::None);
        assert_eq!(layout.spec.entries[0].display, 0);
        assert_eq!(layout.spec.entries[0].argument, Argument::None);

        // The README's keys.
        let text = r#"{ "entries": [ { "app": { "path": "f.exe", "name": "F" },
            "argument": { "url": "https://docs.rs" }, "display": 1,
            "placement": { "width": "40%", "height": "100%", "anchor": "right" } } ],
            "gaps": false, "other-windows": "minimize" }"#;
        let spec: WindowLayoutSpec = serde_json::from_str(text).unwrap();
        assert_eq!(spec.other_windows, OtherWindows::Minimize);
        assert_eq!(
            spec.entries[0].argument,
            Argument::Url("https://docs.rs".into())
        );
    }

    #[test]
    fn saved_layouts_load_back() {
        let path = temporary("round-trip");
        let mut layouts = Vec::new();
        add(&mut layouts, "Coding", spec(2));
        let mut other = spec(1);
        other.other_windows = OtherWindows::Close;
        other.entries[0].argument = Argument::Url("https://x.com".into());
        add(&mut layouts, "Reading", other);
        save(&path, &layouts).unwrap();
        assert_eq!(load(&path), layouts);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_edit_keeps_what_does_not_read() {
        let path = temporary("edit");
        std::fs::remove_file(&path).ok();
        let mut id = String::new();
        assert!(
            edit(&path, |layouts| {
                id = add(layouts, "Coding", spec(1));
                true
            })
            .unwrap()
        );
        let mut source: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        source["layouts"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "id": "b-1", "name": "B" }));
        std::fs::write(&path, source.to_string()).unwrap();

        assert!(edit(&path, |layouts| update(layouts, &id, "Focus", spec(2))).unwrap());
        assert!(!edit(&path, |layouts| remove(layouts, "nope")).unwrap());
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"b-1\""), "{written}");
        let loaded = load(&path);
        assert_eq!(
            (loaded[0].name.as_str(), loaded[0].id.as_str()),
            ("Focus", id.as_str())
        );

        std::fs::write(&path, "{ not json").unwrap();
        assert!(edit(&path, |_| true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn ids_are_stable_and_never_repeat() {
        let mut layouts = Vec::new();
        for _ in 0..50 {
            add(&mut layouts, "Same", spec(1));
        }
        let mut ids: Vec<&str> = layouts.iter().map(|l| l.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 50);
        assert!(new_id("", &[]).starts_with("layout-"));
        let id = layouts[0].id.clone();
        assert!(id.starts_with("same-"), "{id}");
        assert!(update(&mut layouts, &id, "Other", spec(2)));
        assert_eq!(layouts[0].id, id);
    }

    #[test]
    fn duplicate_and_remove() {
        let mut layouts = Vec::new();
        let id = add(&mut layouts, "Coding", spec(2));
        let copy = duplicate(&mut layouts, &id).unwrap();
        assert_ne!(copy, id);
        assert_eq!(layouts[1].name, "Coding Copy");
        assert_eq!(layouts[1].spec, layouts[0].spec);
        assert!(duplicate(&mut layouts, "nope").is_none());
        assert!(remove(&mut layouts, &id));
        assert!(!remove(&mut layouts, &id));
        assert_eq!(layouts.len(), 1);
    }

    #[test]
    fn each_layout_is_a_window_item_that_applies_it() {
        let layouts = vec![
            CustomWindowLayout {
                id: "coding-abc".into(),
                name: "Coding".into(),
                spec: spec(3),
            },
            CustomWindowLayout {
                id: "one-abc".into(),
                name: "One".into(),
                spec: spec(1),
            },
        ];
        let items = WindowLayoutsProvider::new(layouts).catalogue();
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                SAVE_CURRENT_ID,
                "window:layout:coding-abc",
                "window:layout:one-abc"
            ]
        );
        assert_eq!(items[1].title, "Coding");
        assert_eq!(items[1].subtitle.as_deref(), Some("3 apps"));
        assert_eq!(items[2].subtitle.as_deref(), Some("1 app"));
        assert!(items.iter().all(|item| item.kind == Kind::Window));
        assert!(items[1].keywords.iter().any(|k| k == "layout"));
        assert_eq!(
            items[1].primary(),
            Action::ApplyWindowLayout("coding-abc".into())
        );
        assert_eq!(items[0].primary(), Action::SaveCurrentLayout);
        assert!(items[0].keywords.iter().any(|k| k == "snapshot"));
        assert_eq!(apps(0), "0 apps");
    }

    #[test]
    fn the_actions_become_requests_and_nothing_else_does() {
        assert_eq!(
            Request::of(&Action::ApplyWindowLayout("x".into())),
            Some(Request::Apply("x".into()))
        );
        assert_eq!(
            Request::of(&Action::SaveCurrentLayout),
            Some(Request::SaveCurrent)
        );
        assert_eq!(Request::of(&Action::CopyToClipboard("x".into())), None);
    }

    #[test]
    fn applying_says_only_what_went_wrong() {
        assert!(applied("Coding", Ok(LayoutReport::default())).0.is_empty());
        let part = LayoutReport {
            placed: 1,
            launched: 2,
            missing: vec!["Code".into(), "Notes".into()],
        };
        assert_eq!(
            applied("Coding", Ok(part)),
            (
                "Applied Coding in part".into(),
                "No window of Code, Notes".into()
            )
        );
        let (title, body) = applied("Coding", Err(anyhow::anyhow!("no monitors")));
        assert_eq!(title, "Couldn't apply layout");
        assert_eq!(body, "Coding: no monitors");
    }

    #[test]
    fn saving_appends_a_snapshot_named_for_the_minute() {
        let path = temporary("snapshot");
        std::fs::remove_file(&path).ok();
        let when: jiff::Zoned = "2026-10-05T14:30:59[UTC]".parse().unwrap();
        let name = name_at(&when);
        assert_eq!(name, "Layout 2026-10-05 14:30");

        let entries = vec![entry("a.exe", 0), entry("b.exe", 1)];
        assert_eq!(
            saved(&path, Ok(entries.clone()), &name),
            (
                "Saved layout Layout 2026-10-05 14:30".into(),
                "2 apps".into()
            )
        );
        let loaded = load(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].spec.entries, entries);
        assert!(!loaded[0].spec.gaps);
        assert_eq!(loaded[0].spec.other_windows, OtherWindows::None);

        assert_eq!(
            saved(&path, Ok(Vec::new()), &name).0,
            "Couldn't save layout"
        );
        assert_eq!(
            saved(&path, Err(anyhow::anyhow!("not on Linux yet")), &name).1,
            "not on Linux yet"
        );
        assert_eq!(load(&path).len(), 1);
        std::fs::remove_file(path).ok();
    }
}
