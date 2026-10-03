//! Files as the shell knows them (F16): the user's recent items, an item
//! shown selected in Explorer, and the *Open With* chooser.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    FOLDERID_Recent, ILFree, IShellLinkW, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT,
    OPENASINFO, SHOpenFolderAndSelectItems, SHOpenWithDialog, SHParseDisplayName, ShellLink,
};
use windows::core::{Interface as _, PCWSTR};

use crate::{app_watch, com};

/// The most shortcuts resolved, newest first. The Recent folder keeps
/// hundreds, and *Recently Used* shows fifty: the rest would be resolved for
/// nothing.
const RECENT_LIMIT: usize = 200;

/// The targets of the shortcuts in the user's Recent folder -- what Explorer
/// lists under *Recent* -- newest first by the shortcut's own time, which is
/// when the item was last opened. Only targets that exist, each once.
///
/// A target on a network share is left out without being checked: asking
/// whether a path on an unreachable server exists takes as long as the
/// network decides, and the view opens on this answer.
pub(crate) fn recent() -> Result<Vec<PathBuf>> {
    let _apartment = com::Apartment::enter()?;
    let folder = app_watch::known_folder(&FOLDERID_Recent)?;
    let mut links: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(&folder)
        .with_context(|| format!("reading {folder}"))?
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    links.sort_by(|a, b| b.0.cmp(&a.0));

    let reader = LinkReader::new().context("creating a shell link")?;
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for (_, link) in links.into_iter().take(RECENT_LIMIT) {
        let Some(target) = reader.target(&link) else {
            continue;
        };
        if target.starts_with(r"\\") {
            continue;
        }
        let target = PathBuf::from(target);
        if target.exists() && seen.insert(target.clone()) {
            targets.push(target);
        }
    }
    Ok(targets)
}

/// Reads `.lnk` files through `IShellLink`, one instance for all of them,
/// as `apps.rs` reads the Start Menu's: creating one per file is most of
/// the cost.
struct LinkReader {
    link: IShellLinkW,
    file: IPersistFile,
}

impl LinkReader {
    /// Needs a COM apartment on the calling thread.
    fn new() -> Option<Self> {
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let file = link.cast().ok()?;
            Some(Self { link, file })
        }
    }

    /// The path the shortcut points at, as recorded: not `Resolve`, which
    /// goes looking for a target that moved and can take seconds.
    fn target(&self, path: &Path) -> Option<String> {
        unsafe {
            let wide = com::wide(&path.to_string_lossy());
            self.file.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
            let mut buffer = [0u16; MAX_PATH as usize];
            self.link
                .GetPath(&mut buffer, std::ptr::null_mut(), 0)
                .ok()?;
            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
        }
    }
}

/// Open Explorer on the folder holding `path`, with `path` selected.
///
/// On a thread of its own, in a single-threaded apartment: the shell's
/// folder windows expect one, and GPUI's background threads are in the
/// multithreaded apartment. Waited for, since it returns as soon as
/// Explorer has been asked.
pub(crate) fn show_in_folder(path: &Path) -> Result<()> {
    let path = path.to_string_lossy().into_owned();
    std::thread::spawn(move || -> Result<()> {
        let _apartment = com::Apartment::enter()?;
        let mut item: *mut ITEMIDLIST = std::ptr::null_mut();
        unsafe {
            SHParseDisplayName(&com::wide(&path), None, &mut item, 0, None)
                .with_context(|| format!("finding {path} in the shell"))?;
            let shown = SHOpenFolderAndSelectItems(item, None, 0);
            ILFree(Some(item));
            shown.with_context(|| format!("showing {path} in Explorer"))
        }
    })
    .join()
    .map_err(|_| anyhow::anyhow!("the thread showing a file in Explorer panicked"))?
}

/// The system's *Open With* chooser for `path`, which opens it with the
/// application picked and can make that the default.
///
/// The chooser is modal and holds its thread until it closes, so it gets a
/// thread of its own, in the single-threaded apartment its dialog needs;
/// this returns at once. A failure is logged there: by then nobody is
/// waiting for an answer.
pub(crate) fn open_with(path: &Path) -> Result<()> {
    let path = path.to_string_lossy().into_owned();
    std::thread::Builder::new()
        .name("open-with".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let _apartment = com::Apartment::enter()?;
                let file = com::wide(&path);
                let info = OPENASINFO {
                    pcszFile: PCWSTR(file.as_ptr()),
                    pcszClass: PCWSTR::null(),
                    oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
                };
                unsafe { SHOpenWithDialog(None, &info) }
                    .with_context(|| format!("asking which application opens {path}"))
            })();
            if let Err(error) = result {
                log::warn!("{error:#}");
            }
        })
        .context("starting the Open With chooser")?;
    Ok(())
}
