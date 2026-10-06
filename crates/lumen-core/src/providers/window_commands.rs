//! Window commands the user made (F17, 7.59).
//!
//! One file, `window-commands.json` beside `quicklinks.json`, written by the
//! editor and by hand:
//!
//! ```json
//! { "commands": [ { "id": "reading-k3x9q1", "name": "Reading",
//!   "placement": { "width": "60%", "height": "auto", "anchor": "center" } } ] }
//! ```
//!
//! The id is made once, when the command is created, and never derived from
//! the name again: it is what frecency, favourites and `shortcuts.json` hold,
//! so renaming a command must not cost it its hotkey. An entry that does not
//! read costs only itself.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::{Placement, WindowLayout};
use serde::{Deserialize, Serialize};

use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomWindowCommand {
    pub id: String,
    pub name: String,
    pub placement: Placement,
}

impl CustomWindowCommand {
    /// The root search id: `window:custom:<id>`, apart from the built-ins'
    /// slugs, which have no colon.
    pub fn item_id(&self) -> String {
        format!("window:custom:{}", self.id)
    }

    fn item(&self) -> Item {
        Item {
            id: self.item_id(),
            title: self.name.clone(),
            subtitle: Some(super::arrange::SUBTITLE.to_string()),
            icon: None,
            keywords: vec!["window".into(), "custom".into()],
            actions: vec![Action::ArrangeWindow(WindowLayout::Custom(self.placement))],
            kind: Kind::Window,
        }
    }
}

#[derive(Serialize)]
struct File<'a> {
    commands: &'a [CustomWindowCommand],
}

/// The file before each entry is read, so a bad one fails alone.
#[derive(Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    commands: Vec<serde_json::Value>,
}

/// Read the commands. Missing is none; an unreadable file is reported and
/// read as none; an entry that does not read, or repeats an id, is reported
/// and skipped.
pub fn load(path: &Path) -> Vec<CustomWindowCommand> {
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
    let mut commands: Vec<CustomWindowCommand> = Vec::new();
    for entry in raw.commands {
        match serde_json::from_value::<CustomWindowCommand>(entry) {
            Ok(command) if command.id.trim().is_empty() || command.name.trim().is_empty() => {
                log::warn!("{}: skipped a command with no id or name", path.display());
            }
            Ok(command) if commands.iter().any(|c| c.id == command.id) => {
                log::warn!("{}: skipped a second {:?}", path.display(), command.id);
            }
            Ok(command) => commands.push(command),
            Err(error) => log::warn!("{}: skipped a command: {error}", path.display()),
        }
    }
    commands
}

/// Write the commands, beside and renamed over, as the shortcuts are.
pub fn save(path: &Path, commands: &[CustomWindowCommand]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    replace(path, serde_json::to_string_pretty(&File { commands })?)
}

pub(super) fn replace(path: &Path, source: String) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, source)
        .with_context(|| format!("writing {}", temporary.display()))?;
    std::fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
}

/// Change the commands in the file and write it back: the editor's save,
/// duplicate and delete. Read afresh, so a hand edit made meanwhile is kept;
/// an entry that does not read is kept as it was, after the rest, for the
/// user to fix rather than the editor to drop. A file that is not JSON at
/// all is an error, not overwritten. Whether `change` did anything.
pub fn edit(
    path: &Path,
    change: impl FnOnce(&mut Vec<CustomWindowCommand>) -> bool,
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
    let mut commands: Vec<CustomWindowCommand> = Vec::new();
    let mut kept = Vec::new();
    for entry in raw.commands {
        match serde_json::from_value::<CustomWindowCommand>(entry.clone()) {
            Ok(command)
                if !command.id.trim().is_empty()
                    && !command.name.trim().is_empty()
                    && !commands.iter().any(|c| c.id == command.id) =>
            {
                commands.push(command)
            }
            _ => kept.push(entry),
        }
    }
    if !change(&mut commands) {
        return Ok(false);
    }
    let mut entries = commands
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    entries.extend(kept);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    replace(
        path,
        serde_json::to_string_pretty(&serde_json::json!({ "commands": entries }))?,
    )?;
    Ok(true)
}

/// A new id for a command called `name`: its slug and a suffix from the
/// clock, lengthened until no command in `existing` has it.
pub fn new_id(name: &str, existing: &[CustomWindowCommand]) -> String {
    fresh_id(name, "command", |id| {
        existing.iter().any(|command| command.id == id)
    })
}

/// [`new_id`]'s rule for anything with an id made once: the slug of `name`,
/// or `fallback`, and a suffix from the clock, until `taken` says no.
pub(super) fn fresh_id(name: &str, fallback: &str, taken: impl Fn(&str) -> bool) -> String {
    let slug: String = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = match slug.is_empty() {
        true => fallback.to_string(),
        false => slug.chars().take(32).collect(),
    };
    let mut stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    loop {
        let id = format!("{slug}-{}", base36(stamp % 36u128.pow(6)));
        if !taken(&id) {
            return id;
        }
        stamp = stamp.wrapping_add(1);
    }
}

fn base36(mut value: u128) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

/// Add a command. Its new id.
pub fn add(commands: &mut Vec<CustomWindowCommand>, name: &str, placement: Placement) -> String {
    let id = new_id(name, commands);
    commands.push(CustomWindowCommand {
        id: id.clone(),
        name: name.to_string(),
        placement,
    });
    id
}

/// Rename or reshape the command `id`, which keeps its id. Whether it exists.
pub fn update(
    commands: &mut [CustomWindowCommand],
    id: &str,
    name: &str,
    placement: Placement,
) -> bool {
    let Some(command) = commands.iter_mut().find(|command| command.id == id) else {
        return false;
    };
    command.name = name.to_string();
    command.placement = placement;
    true
}

/// Remove the command `id`. Whether it existed.
pub fn remove(commands: &mut Vec<CustomWindowCommand>, id: &str) -> bool {
    let before = commands.len();
    commands.retain(|command| command.id != id);
    commands.len() != before
}

/// Copy the command `id` under a new id, right after it. The copy's id.
pub fn duplicate(commands: &mut Vec<CustomWindowCommand>, id: &str) -> Option<String> {
    let index = commands.iter().position(|command| command.id == id)?;
    let name = format!("{} Copy", commands[index].name);
    let copy = CustomWindowCommand {
        id: new_id(&name, commands),
        name,
        placement: commands[index].placement,
    };
    let new = copy.id.clone();
    commands.insert(index + 1, copy);
    Some(new)
}

/// What *Duplicate Command* starts from for a built-in layout, where a
/// placement can say the same thing (see [`WindowLayout::placement`]).
pub fn duplicate_source(layout: WindowLayout) -> Option<Placement> {
    layout.placement()
}

/// The commands as root search entries.
#[derive(Default)]
pub struct WindowCommandsProvider(Vec<CustomWindowCommand>);

impl WindowCommandsProvider {
    pub fn new(commands: Vec<CustomWindowCommand>) -> Self {
        Self(commands)
    }
}

impl Provider for WindowCommandsProvider {
    fn id(&self) -> &'static str {
        "window"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.0.iter().map(CustomWindowCommand::item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_platform::{Anchor, Length, Offset};

    fn temporary(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lumen-window-commands-{}-{name}.json",
            std::process::id()
        ))
    }

    fn reading() -> Placement {
        Placement {
            width: Length::Percent(6000),
            height: Length::Auto,
            x: Offset::Points(0),
            y: Offset::Percent(-1000),
            anchor: Anchor::Center,
            gaps: false,
        }
    }

    #[test]
    fn a_placement_reads_and_writes_as_typed() {
        let text = r#"{"width":"60%","height":"auto","x":"0pt","y":"-10%","anchor":"center","gaps":false}"#;
        let placement: Placement = serde_json::from_str(text).unwrap();
        assert_eq!(placement, reading());
        assert_eq!(serde_json::to_string(&placement).unwrap(), text);

        // Only the size is required: no offset, centred, without gaps.
        let short: Placement =
            serde_json::from_str(r#"{"width":"33.3%","height":"400pt"}"#).unwrap();
        assert_eq!(short.width, Length::Percent(3330));
        assert_eq!(
            (short.x, short.anchor, short.gaps),
            (Offset::Points(0), Anchor::Center, false)
        );
    }

    #[test]
    fn a_bad_figure_is_refused_with_its_reason() {
        let error = serde_json::from_str::<Placement>(r#"{"width":"150%","height":"auto"}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 100%"), "{error}");
        assert!(
            serde_json::from_str::<Placement>(r#"{"width":"50%","height":"auto","x":"-120%"}"#)
                .is_err()
        );
    }

    #[test]
    fn a_missing_file_is_no_commands() {
        assert!(load(&temporary("missing")).is_empty());
    }

    #[test]
    fn a_bad_entry_costs_only_itself() {
        let path = temporary("bad-entry");
        std::fs::write(
            &path,
            r#"{ "commands": [
                { "id": "a-1", "name": "A", "placement": { "width": "50%", "height": "auto" } },
                { "id": "b-1", "name": "B", "placement": { "width": "0%", "height": "auto" } },
                { "id": "c-1", "name": "", "placement": { "width": "50%", "height": "auto" } },
                { "id": "a-1", "name": "Again", "placement": { "width": "50%", "height": "auto" } },
                { "id": "d-1", "name": "D", "placement": { "width": "10pt", "height": "10pt" } }
            ] }"#,
        )
        .unwrap();
        let ids: Vec<String> = load(&path).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["a-1", "d-1"]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn saved_commands_load_back() {
        let path = temporary("round-trip");
        let mut commands = Vec::new();
        add(&mut commands, "Reading", reading());
        add(
            &mut commands,
            "Sidebar",
            WindowLayout::LeftThird.placement().unwrap(),
        );
        save(&path, &commands).unwrap();
        assert_eq!(load(&path), commands);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_edit_keeps_what_does_not_read() {
        let path = temporary("edit");
        std::fs::remove_file(&path).ok();
        let mut id = String::new();
        assert!(
            edit(&path, |commands| {
                id = add(commands, "Reading", reading());
                true
            })
            .unwrap()
        );
        assert_eq!(load(&path)[0].id, id);

        let bad =
            r#"{ "id": "b-1", "name": "B", "placement": { "width": "0%", "height": "auto" } }"#;
        let mut source: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        source["commands"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::from_str(bad).unwrap());
        std::fs::write(&path, source.to_string()).unwrap();

        assert!(edit(&path, |commands| update(commands, &id, "Focus", reading())).unwrap());
        assert!(!edit(&path, |commands| remove(commands, "nope")).unwrap());
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"b-1\""), "{written}");
        assert_eq!(load(&path)[0].name, "Focus");

        std::fs::write(&path, "{ not json").unwrap();
        assert!(edit(&path, |_| true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_id_is_made_once_and_survives_a_rename() {
        let mut commands = Vec::new();
        let id = add(&mut commands, "Reading Mode!", reading());
        assert!(id.starts_with("reading-mode-"), "{id}");
        assert!(update(&mut commands, &id, "Focus", reading()));
        assert_eq!(commands[0].id, id);
        assert_eq!(commands[0].item_id(), format!("window:custom:{id}"));
        assert!(!update(&mut commands, "nope", "X", reading()));
    }

    #[test]
    fn ids_never_repeat() {
        let mut commands = Vec::new();
        for _ in 0..50 {
            add(&mut commands, "Same", reading());
        }
        let mut ids: Vec<&str> = commands.iter().map(|c| c.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 50);
        assert!(new_id("", &[]).starts_with("command-"));
    }

    #[test]
    fn duplicate_and_remove() {
        let mut commands = Vec::new();
        let id = add(&mut commands, "Reading", reading());
        let copy = duplicate(&mut commands, &id).unwrap();
        assert_ne!(copy, id);
        assert_eq!(commands[1].name, "Reading Copy");
        assert_eq!(commands[1].placement, commands[0].placement);
        assert!(duplicate(&mut commands, "nope").is_none());
        assert!(remove(&mut commands, &id));
        assert!(!remove(&mut commands, &id));
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn each_command_is_a_window_item_that_arranges_it() {
        let commands = vec![CustomWindowCommand {
            id: "reading-abc".into(),
            name: "Reading".into(),
            placement: reading(),
        }];
        let items = WindowCommandsProvider::new(commands).catalogue();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "window:custom:reading-abc");
        assert_eq!(item.title, "Reading");
        assert_eq!(item.kind, Kind::Window);
        assert!(item.keywords.iter().any(|k| k == "custom"));
        assert_eq!(
            item.primary(),
            Action::ArrangeWindow(WindowLayout::Custom(reading()))
        );
    }

    #[test]
    fn a_built_in_duplicates_where_a_placement_can_say_it() {
        let left = duplicate_source(WindowLayout::LeftHalf).unwrap();
        assert_eq!(
            (left.width, left.height),
            (Length::Percent(5000), Length::Percent(10_000))
        );
        assert_eq!(left.anchor, Anchor::Left);
        let second = duplicate_source(WindowLayout::SecondFourth).unwrap();
        assert_eq!(
            (second.anchor, second.x),
            (Anchor::Left, Offset::Percent(2500))
        );
        let keep = duplicate_source(WindowLayout::CenterKeepSize).unwrap();
        assert_eq!((keep.width, keep.height), (Length::Auto, Length::Auto));
        for layout in [
            WindowLayout::MaximizeWidth,
            WindowLayout::MoveUp,
            WindowLayout::PreviousMonitor,
            WindowLayout::Minimize,
            WindowLayout::FocusNextAppWindow,
        ] {
            assert_eq!(duplicate_source(layout), None, "{layout:?}");
        }
    }
}
