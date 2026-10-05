//! The applications that open a file, as the shell's *Open With* menu lists
//! them, and opening a file with one of them: what a quicklink to a file or
//! folder stores and runs.
//!
//! Through `SHAssocEnumHandlers` rather than the registry: the shell merges
//! `OpenWithProgids`, `OpenWithList`, per-user choices and packaged
//! applications' declarations, and invoking a handler is the only way to
//! open a file with a packaged application at all.

use std::path::Path;

use anyhow::{Context as _, Result};
use lumen_platform::AppChoice;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_RECOMMENDED, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_NOTRUNCATE, ASSOCSTR,
    ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW, BHID_DataObject,
    IAssocHandler, IShellItem, SHAssocEnumHandlers, SHCreateItemFromParsingName,
};
use windows::core::{PCWSTR, PWSTR};

use crate::com;

/// The one application a folder is offered: Explorer, opened through the
/// shell as any folder is.
pub(crate) const EXPLORER: &str = "explorer";

/// The applications that open `path`, the default first.
///
/// On a thread of its own in a single-threaded apartment, as the shell's
/// association objects expect, and waited for: the caller is off the UI
/// thread and asked for the answer.
pub(crate) fn apps_for(path: &Path) -> Result<Vec<AppChoice>> {
    if path.is_dir() {
        return Ok(vec![AppChoice {
            id: EXPLORER.into(),
            name: "File Explorer".into(),
        }]);
    }
    let Some(extension) = extension(path) else {
        return Ok(Vec::new());
    };
    on_sta("apps-for", move || {
        let choices = handlers(&extension)?
            .into_iter()
            .map(|(choice, _)| choice)
            .collect();
        Ok(ordered(
            choices,
            query(&extension, ASSOCSTR_EXECUTABLE).as_deref(),
            query(&extension, ASSOCSTR_FRIENDLYAPPNAME).as_deref(),
        ))
    })
}

/// Open `path` with the handler named `app_id`.
///
/// `IAssocHandler::Invoke` rather than starting the handler's program: a
/// packaged application has no program to start with an argument, and the
/// shell is what knows each handler's command line.
pub(crate) fn open_path_with(path: &Path, app_id: &str) -> Result<()> {
    if app_id == EXPLORER {
        return crate::shell_execute(&path.to_string_lossy());
    }
    let extension = extension(path)
        .with_context(|| format!("{} has no extension to choose an app by", path.display()))?;
    let path = path.to_string_lossy().into_owned();
    let app_id = app_id.to_string();
    on_sta("open-path-with", move || {
        let mut found = handlers(&extension)?;
        let ids: Vec<&str> = found.iter().map(|(choice, _)| choice.id.as_str()).collect();
        let index = matching(&app_id, &ids)
            .with_context(|| format!("{app_id} no longer opens {extension} files"))?;
        let (_, handler) = found.swap_remove(index);
        unsafe {
            let item: IShellItem = SHCreateItemFromParsingName(&com::wide(&path), None)
                .with_context(|| format!("finding {path} in the shell"))?;
            let data: IDataObject = item
                .BindToHandler(None, &BHID_DataObject)
                .with_context(|| format!("reading {path} as a shell item"))?;
            handler
                .Invoke(&data)
                .with_context(|| format!("opening {path} with {app_id}"))
        }
    })
}

/// Which of `ids` is the handler stored as `wanted`: the same id, or else the
/// only one whose program has the same file name.
///
/// The second because a packaged application's id is its program's path,
/// which names its version: `...\Microsoft.WindowsNotepad_11.2607.14.0_x64_
/// _8wekyb3d8bbwe\Notepad\Notepad.exe` moves with every Store update.
fn matching(wanted: &str, ids: &[&str]) -> Option<usize> {
    if let Some(exact) = ids.iter().position(|id| id.eq_ignore_ascii_case(wanted)) {
        return Some(exact);
    }
    let file = |id: &str| id.rsplit(['\\', '/']).next().map(str::to_ascii_lowercase);
    let wanted = file(wanted)?;
    let mut same = (0..ids.len()).filter(|&index| file(ids[index]).as_ref() == Some(&wanted));
    match (same.next(), same.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// `.txt`, as the association functions take it.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
}

fn on_sta<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _apartment = com::Apartment::enter()?;
            work()
        })
        .with_context(|| format!("starting {name}"))?
        .join()
        .map_err(|_| anyhow::anyhow!("the {name} thread panicked"))?
}

/// The recommended handlers for `extension`, in the shell's order. Needs a
/// COM apartment on the calling thread.
fn handlers(extension: &str) -> Result<Vec<(AppChoice, IAssocHandler)>> {
    let mut found = Vec::new();
    unsafe {
        let list = SHAssocEnumHandlers(&com::wide(extension), ASSOC_FILTER_RECOMMENDED)
            .with_context(|| format!("listing the apps for {extension}"))?;
        loop {
            let mut slot = [None];
            let mut fetched = 0u32;
            if list.Next(&mut slot, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(handler) = slot[0].take() else {
                break;
            };
            let id = handler
                .GetName()
                .ok()
                .and_then(|id| com::take_pwstr(id).ok());
            let name = handler
                .GetUIName()
                .ok()
                .and_then(|name| com::take_pwstr(name).ok());
            if let (Some(id), Some(name)) = (id, name) {
                found.push((AppChoice { id, name }, handler));
            }
        }
    }
    Ok(found)
}

/// What the default association for `extension` says about its `open`
/// verb: the program's path or its name. `None` when it says nothing.
fn query(extension: &str, what: ASSOCSTR) -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_NOTRUNCATE | ASSOCF_INIT_IGNOREUNKNOWN,
            what,
            &com::wide(extension),
            PCWSTR::null(),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
    };
    if result.is_err() {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]);
    (!value.is_empty()).then_some(value)
}

/// `choices` each once, by id, with the default moved to the front: the one
/// whose id is the default's program, or failing that whose name is the
/// default's name. In the shell's order when neither is found.
fn ordered(
    choices: Vec<AppChoice>,
    default_program: Option<&str>,
    default_name: Option<&str>,
) -> Vec<AppChoice> {
    let mut unique: Vec<AppChoice> = Vec::with_capacity(choices.len());
    for choice in choices {
        if !unique
            .iter()
            .any(|seen| seen.id.eq_ignore_ascii_case(&choice.id))
        {
            unique.push(choice);
        }
    }

    let by = |wanted: Option<&str>, field: fn(&AppChoice) -> &str| {
        let wanted = wanted?;
        unique
            .iter()
            .position(|choice| field(choice).eq_ignore_ascii_case(wanted))
    };
    let default = by(default_program, |choice| &choice.id)
        .or_else(|| by(default_name, |choice| &choice.name));
    if let Some(index) = default {
        let choice = unique.remove(index);
        unique.insert(0, choice);
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> AppChoice {
        AppChoice {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn the_default_program_comes_first() {
        let choices = vec![
            app(r"C:\Windows\notepad.exe", "Notepad"),
            app(r"C:\Code\Code.exe", "Visual Studio Code"),
        ];
        assert_eq!(
            ordered(choices, Some(r"c:\code\code.exe"), Some("Notepad")),
            vec![
                app(r"C:\Code\Code.exe", "Visual Studio Code"),
                app(r"C:\Windows\notepad.exe", "Notepad"),
            ]
        );
    }

    /// A packaged default has no program path to match, only its name.
    #[test]
    fn the_default_is_found_by_name_without_a_program() {
        let choices = vec![
            app(r"C:\Code\Code.exe", "Visual Studio Code"),
            app("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App", "Notepad"),
        ];
        assert_eq!(ordered(choices, None, Some("notepad"))[0].name, "Notepad");
    }

    #[test]
    fn each_app_is_listed_once_in_the_shells_order() {
        let choices = vec![
            app("a.exe", "A"),
            app("b.exe", "B"),
            app("A.EXE", "A again"),
        ];
        assert_eq!(
            ordered(choices, Some("nothing.exe"), None),
            vec![app("a.exe", "A"), app("b.exe", "B")]
        );
    }

    #[test]
    fn an_updated_app_is_still_found_by_its_programs_name() {
        let ids = [
            r"C:\Code\Code.exe",
            r"C:\Program Files\WindowsApps\Notepad_11.3.0.0_x64__8w\Notepad\Notepad.exe",
        ];
        assert_eq!(matching(r"c:\code\code.exe", &ids), Some(0));
        assert_eq!(
            matching(
                r"C:\Program Files\WindowsApps\Notepad_11.2.0.0_x64__8w\Notepad\Notepad.exe",
                &ids
            ),
            Some(1)
        );
        assert_eq!(matching(r"C:\Other\Zed.exe", &ids), None);
        // Two programs of that name: neither, rather than a guess.
        assert_eq!(matching(r"X:\a.exe", &[r"C:\a.exe", r"D:\a.exe"]), None);
    }

    /// Lists what this machine offers; run with `--ignored` to see it.
    #[test]
    #[ignore = "reads this machine's associations"]
    fn a_text_file_and_a_folder_have_apps() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/assoc-live");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "x").unwrap();

        let apps = apps_for(&file).unwrap();
        println!(
            "notes.txt: {} apps, default {:?}, program {:?}, name {:?}",
            apps.len(),
            apps.first(),
            query(".txt", ASSOCSTR_EXECUTABLE),
            query(".txt", ASSOCSTR_FRIENDLYAPPNAME)
        );
        for app in &apps {
            println!("  {} | {}", app.name, app.id);
        }
        assert!(!apps.is_empty());

        let folder = apps_for(&dir).unwrap();
        println!("folder: {folder:?}");
        assert_eq!(folder[0].id, EXPLORER);

        assert!(apps_for(&dir.join("README")).unwrap().is_empty());
    }
}
