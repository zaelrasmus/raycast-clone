//! The application index.
//!
//! # Why the AppsFolder and not a Start Menu walk
//!
//! Section 7 describes two sources: Start Menu shortcuts on disk, plus packaged
//! applications through WinRT's `PackageManager`. That is two enumerations, two
//! notions of identity, two icon paths, and a deduplication problem where a
//! packaged application also ships a shortcut.
//!
//! The shell already merges them. `shell:AppsFolder` is the virtual folder
//! behind "All apps", and it contains Win32 shortcuts and packaged applications
//! as a single item list, each with a display name and a parsing name that
//! doubles as a launch target and an icon source. One enumeration, one identity,
//! one icon path -- and it is the same list the user sees in the Start menu,
//! which is the actual definition of "100% of installed applications" in the F1
//! acceptance criteria.

use anyhow::{Context as _, Result};
use lumen_platform::{AppEntry, IconSource, LaunchTarget};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};

use crate::com;

/// The shell namespace path of the virtual folder holding every application.
const APPS_FOLDER: &str = "shell:AppsFolder";

pub fn enumerate() -> Result<Vec<AppEntry>> {
    // Called from a worker thread, so this thread needs its own apartment.
    let _apartment = com::Apartment::enter()?;

    unsafe {
        let folder: IShellItem = SHCreateItemFromParsingName(&com::wide(APPS_FOLDER), None)
            .context("opening shell:AppsFolder")?;

        let items: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .context("binding the AppsFolder enumerator")?;

        let mut entries = Vec::new();
        loop {
            let mut fetched: [Option<IShellItem>; 1] = [None];
            let mut count = 0u32;
            // `Next` returns S_FALSE when the enumeration is exhausted, which is
            // success, not an error -- hence checking the fetched count rather
            // than the HRESULT.
            if items.Next(&mut fetched, Some(&mut count)).is_err() || count == 0 {
                break;
            }
            let Some(item) = fetched[0].take() else {
                break;
            };

            match describe(&item) {
                Ok(entry) => entries.push(entry),
                // One unreadable entry must not cost the whole index. This is
                // routine: the AppsFolder contains items whose provider has
                // been uninstalled but whose registration lingers.
                Err(error) => log::debug!("skipping an AppsFolder item: {error:#}"),
            }
        }

        Ok(entries)
    }
}

fn describe(item: &IShellItem) -> Result<AppEntry> {
    unsafe {
        let name = com::take_pwstr(item.GetDisplayName(SIGDN_NORMALDISPLAY)?)
            .context("reading the display name")?;

        // The parent-relative parsing name is the application user model ID for
        // packaged applications and the shortcut's file name for Win32 ones.
        // Either way it uniquely identifies the item within the folder, which
        // makes it the right stable key for frecency across reindexes.
        let parsing = com::take_pwstr(item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING)?)
            .context("reading the parsing name")?;

        let full_parsing_name = format!("{APPS_FOLDER}\\{parsing}");

        Ok(AppEntry {
            id: format!("app:{parsing}"),
            name,
            detail: detail_from(&parsing),
            target: LaunchTarget::ShellExecute(full_parsing_name.clone()),
            icon: Some(IconSource::ShellItem(full_parsing_name)),
            keywords: Vec::new(),
        })
    }
}

/// A short, honest secondary line.
///
/// Packaged applications are identified by a family name of the shape
/// `Publisher.Product_hash!App`; the readable half is what is worth showing.
/// Win32 entries parse as a `.lnk` file name, which repeats the title and is
/// therefore worth showing nothing at all rather than noise.
fn detail_from(parsing: &str) -> Option<String> {
    let (family, _app) = parsing.split_once('!')?;
    let package = family.split('_').next().unwrap_or(family);
    let readable = package.rsplit('.').next().unwrap_or(package);
    (!readable.is_empty()).then(|| readable.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_entries_get_a_readable_detail() {
        assert_eq!(
            detail_from("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App").as_deref(),
            Some("WindowsTerminal")
        );
    }

    #[test]
    fn win32_shortcuts_get_no_detail() {
        // The parsing name is just the shortcut file, which would only repeat
        // the title back at the user.
        assert_eq!(detail_from("Firefox.lnk"), None);
    }
}
