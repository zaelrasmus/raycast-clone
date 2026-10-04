//! Noticing that applications were installed or removed.
//!
//! The index is read from `shell:AppsFolder`, which is built from two sources
//! that change in two different ways, so two things are watched:
//!
//! - **The Start Menu folders**, the per-user one and the all-users one. A
//!   Win32 installer that wants to appear in Start writes a shortcut here, and
//!   that is exactly the set of Win32 applications the AppsFolder lists.
//!   The two Desktops are watched as well, without their subfolders, since
//!   the index also lists applications that only have a Desktop shortcut.
//!   That watch cannot tell a shortcut from any other file, so every Desktop
//!   change re-indexes; ARCHITECTURE 7.34 records why that is accepted.
//! - **The extra application folders** the user named (`apps.extra-folders`),
//!   each with its subfolders, so a portable program dropped into one is
//!   listed without a restart. A folder that does not exist when the watch
//!   starts is not watched.
//! - **The per-user package repository in the registry**. A packaged (MSIX or
//!   Store) application writes no shortcut anywhere; registering it for the
//!   user adds a subkey under `AppModel\Repository\Packages`, and removing it
//!   takes the subkey away.
//!
//! Both are kernel change notifications waited on by one thread, which is
//! asleep in `WaitForMultipleObjects` the rest of the time: nothing is polled,
//! so idle CPU stays where it was.
//!
//! # Coalescing
//!
//! An installer writes dozens of files and keys over several seconds. After
//! the first signal the thread keeps collecting until the sources have been
//! quiet for [`SETTLE`], then reports -- and reports once more [`FOLLOW_UP`]
//! later, because the shell updates its own list of applications a few
//! seconds after the files it is built from. The daemon re-enumerates each
//! time and, if the result is the same as before, leaves the list alone, so a
//! report about nothing Lumen lists costs one enumeration and no visible
//! change.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use lumen_platform::ApplicationsSink;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FindFirstChangeNotificationW,
    FindNextChangeNotification,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_NAME, RegNotifyChangeKeyValue,
    RegOpenKeyExW,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForMultipleObjects};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Programs, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::{GUID, HSTRING, PCWSTR, w};

/// How long the sources must be quiet before one report goes out.
const SETTLE: Duration = Duration::from_millis(1500);

/// How long after a report to look again, for the shell to have caught up.
const FOLLOW_UP: Duration = Duration::from_secs(5);

/// How many handles one `WaitForMultipleObjects` takes (`MAXIMUM_WAIT_OBJECTS`).
const MAX_WAIT: usize = 64;

/// Where the per-user package registrations live.
const PACKAGES_KEY: PCWSTR = w!(
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages"
);

/// One thing being waited on, and how to wait on it again.
enum Source {
    /// A Start Menu, Desktop or extra application folder.
    Folder(HANDLE),
    /// The package key, whose notification has to be re-requested after
    /// every signal, onto the same event.
    Registry { key: HKEY, event: HANDLE },
}

impl Source {
    fn handle(&self) -> HANDLE {
        match self {
            Source::Folder(handle) => *handle,
            Source::Registry { event, .. } => *event,
        }
    }

    /// Ask to be told about the *next* change.
    fn rearm(&self) -> Result<()> {
        unsafe {
            match self {
                Source::Folder(handle) => {
                    FindNextChangeNotification(*handle).context("re-arming a folder watch")
                }
                Source::Registry { key, event } => {
                    let status = RegNotifyChangeKeyValue(
                        *key,
                        false,
                        REG_NOTIFY_CHANGE_NAME,
                        Some(*event),
                        true,
                    );
                    if status != ERROR_SUCCESS {
                        anyhow::bail!("RegNotifyChangeKeyValue failed: {status:?}");
                    }
                    Ok(())
                }
            }
        }
    }
}

/// Start watching on a dedicated thread.
///
/// Returns once the watches are in place, so "no watch" is an error the
/// daemon can log rather than a thread that failed silently. A source that
/// cannot be watched is skipped as long as at least one can.
pub fn start(sink: ApplicationsSink, extra_folders: &[PathBuf]) -> Result<()> {
    let extra_folders = extra_folders.to_vec();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

    std::thread::Builder::new()
        .name("lumen-app-watch".into())
        .spawn(move || {
            // Opened on this thread: a registry notification belongs to the
            // thread that asked for it, and ends when that thread does.
            let sources = open_sources(&extra_folders);
            if sources.is_empty() {
                let _ = ready_tx.send(Err(anyhow::anyhow!(
                    "neither the Start Menu nor the package registry could be watched"
                )));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            run(&sources, &sink);
        })
        .context("spawning the application watcher thread")?;

    ready_rx
        .recv()
        .context("the application watcher stopped before reporting readiness")?
}

fn open_sources(extra_folders: &[PathBuf]) -> Vec<Source> {
    let mut sources = Vec::new();

    let start_menu = [&FOLDERID_Programs, &FOLDERID_CommonPrograms].map(|folder| (folder, true));
    let desktop = crate::apps::DESKTOP.map(|folder| (folder, false));
    for (folder, subtree) in start_menu.into_iter().chain(desktop) {
        match watch_folder(folder, subtree) {
            Ok(source) => sources.push(source),
            Err(error) => log::debug!("not watching a shortcut folder: {error:#}"),
        }
    }

    // One handle is kept back for the package key.
    let room = MAX_WAIT.saturating_sub(sources.len() + 1);
    for (folder, subtree) in extra_watches(extra_folders, room) {
        match watch_path(&folder, subtree) {
            Ok(source) => sources.push(source),
            Err(error) => log::debug!("not watching an application folder: {error:#}"),
        }
    }

    match watch_packages() {
        Ok(source) => sources.push(source),
        Err(error) => log::debug!("not watching packaged applications: {error:#}"),
    }

    sources
}

fn watch_folder(id: &GUID, subtree: bool) -> Result<Source> {
    watch_path(Path::new(&known_folder(id)?), subtree)
}

fn watch_path(path: &Path, subtree: bool) -> Result<Source> {
    let handle = unsafe {
        FindFirstChangeNotificationW(
            &HSTRING::from(path),
            subtree,
            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
        )
    }
    .with_context(|| format!("watching {}", path.display()))?;
    log::debug!("watching {} for installs", path.display());
    Ok(Source::Folder(handle))
}

fn watch_packages() -> Result<Source> {
    let mut key = HKEY::default();
    let status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PACKAGES_KEY, None, KEY_NOTIFY, &mut key) };
    if status != ERROR_SUCCESS {
        anyhow::bail!("opening the package repository key: {status:?}");
    }

    // Auto-reset, so a wait consumes the signal and the next wait blocks
    // until something else happens.
    let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
        .context("creating an event for the package key")?;

    let source = Source::Registry { key, event };
    source.rearm()?;
    log::debug!("watching the package repository for installs");
    Ok(source)
}

/// A known folder's path, such as the user's Start Menu programs folder.
pub(crate) fn known_folder(id: &GUID) -> Result<String> {
    unsafe {
        let raw =
            SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).context("SHGetKnownFolderPath")?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.as_ptr() as *const _));
        path.context("a known folder path that is not valid UTF-16")
    }
}

/// The extra folders to watch, each with its subfolders.
///
/// A folder that is missing, or is a file, is skipped; one named twice is
/// watched once. No more than `room` are kept, since a single wait holds at
/// most [`MAX_WAIT`] handles.
fn extra_watches(folders: &[PathBuf], room: usize) -> Vec<(PathBuf, bool)> {
    let mut watches: Vec<(PathBuf, bool)> = Vec::new();
    for folder in folders.iter().filter(|folder| folder.is_dir()) {
        if watches.iter().any(|(watched, _)| watched == folder) {
            continue;
        }
        if watches.len() == room {
            log::warn!(
                "only the first {room} application folders are watched;                  programs added to the rest appear after `lumen reload`"
            );
            break;
        }
        watches.push((folder.clone(), true));
    }
    watches
}

/// Wait, coalesce, report; forever.
fn run(sources: &[Source], sink: &ApplicationsSink) {
    let handles: Vec<HANDLE> = sources.iter().map(Source::handle).collect();

    loop {
        // Asleep until something happens.
        if !next_signal(sources, &handles, INFINITE).unwrap_or(false) {
            return;
        }

        loop {
            // Collect the rest of the install before saying anything.
            loop {
                match next_signal(sources, &handles, SETTLE.as_millis() as u32) {
                    Some(true) => continue,
                    Some(false) => break,
                    None => return,
                }
            }
            sink.emit();

            // The second look. The shell rebuilds the AppsFolder from these
            // sources asynchronously, and a new shortcut was measured taking
            // two and a half to four seconds to be listed -- so the first
            // report often re-enumerates the old list. Nothing else will
            // signal once the files stop changing, so this is the only
            // chance to catch up.
            match next_signal(sources, &handles, FOLLOW_UP.as_millis() as u32) {
                // More changes arrived meanwhile: settle and report again.
                Some(true) => continue,
                Some(false) => {
                    sink.emit();
                    break;
                }
                None => return,
            }
        }
    }
}

/// Wait up to `timeout` for a change, and re-arm whichever source fired.
///
/// `Some(true)` is a change, `Some(false)` is quiet, and `None` means the
/// watch has failed and the thread should stop.
fn next_signal(sources: &[Source], handles: &[HANDLE], timeout: u32) -> Option<bool> {
    let result = unsafe { WaitForMultipleObjects(handles, false, timeout) };
    if result == WAIT_TIMEOUT {
        return Some(false);
    }

    let index = result.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
    let Some(source) = sources.get(index) else {
        log::warn!("the application watcher stopped: wait returned {result:?}");
        return None;
    };

    match source.rearm() {
        Ok(()) => Some(true),
        Err(error) => {
            log::warn!("the application watcher stopped: {error:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn each_existing_extra_folder_is_watched_with_its_subfolders() {
        let root = scratch("app-watch-extra");
        let tools = root.join("Tools");
        let portable = root.join("Portable");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::create_dir_all(&portable).unwrap();

        let watches = extra_watches(&[tools.clone(), portable.clone()], 10);

        assert_eq!(watches, vec![(tools, true), (portable, true)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_or_a_file_is_skipped_and_a_repeat_watched_once() {
        let root = scratch("app-watch-missing");
        let tools = root.join("Tools");
        std::fs::create_dir_all(&tools).unwrap();
        let file = root.join("notes.txt");
        std::fs::write(&file, "").unwrap();

        let watches = extra_watches(&[root.join("gone"), file, tools.clone(), tools.clone()], 10);

        assert_eq!(watches, vec![(tools, true)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_more_folders_are_watched_than_one_wait_can_hold() {
        let root = scratch("app-watch-room");
        let folders: Vec<PathBuf> = (0..3).map(|i| root.join(i.to_string())).collect();
        for folder in &folders {
            std::fs::create_dir_all(folder).unwrap();
        }

        let watches = extra_watches(&folders, 2);

        assert_eq!(watches.len(), 2);
        assert!(extra_watches(&folders, 0).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
