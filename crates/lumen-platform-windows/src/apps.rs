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
//!
//! # Desktop shortcuts
//!
//! Some installers never write a Start Menu entry and leave only a shortcut on
//! the Desktop; Modrinth is one. Those applications are not in the AppsFolder,
//! so the Desktop folders are read as a second, narrower source: a `.lnk`
//! counts when it starts an `.exe`, a `.url` when it opens an application's
//! own scheme such as `steam://`. Anything the AppsFolder already lists, by
//! name or by what it starts, is left out.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use lumen_platform::{AppEntry, IconSource, LaunchTarget};
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs,
    FOLDERID_PublicDesktop, IEnumShellItems, IShellItem, IShellLinkW, SHCreateItemFromParsingName,
    SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, ShellLink,
};
use windows::core::{GUID, Interface as _, PCWSTR};

use crate::{app_watch, com};

/// The folders whose shortcuts make up the AppsFolder's Win32 half.
const START_MENU: [&GUID; 2] = [&FOLDERID_Programs, &FOLDERID_CommonPrograms];

/// The per-user and all-users Desktop, read without their subfolders.
pub(crate) const DESKTOP: [&GUID; 2] = [&FOLDERID_Desktop, &FOLDERID_PublicDesktop];

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

        let start_menu = Shortcuts::read(&START_MENU, true);
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

            match describe(&item, &start_menu.urls) {
                Ok(entry) => entries.push(entry),
                // One unreadable entry must not cost the whole index. This is
                // routine: the AppsFolder contains items whose provider has
                // been uninstalled but whose registration lingers.
                Err(error) => log::debug!("skipping an AppsFolder item: {error:#}"),
            }
        }

        let desktop = Shortcuts::read(&DESKTOP, false);
        let reader = LinkReader::new();
        let resolve = |path: &Path| reader.as_ref()?.target(path.to_str()?);
        let targets = listed_targets(&entries, |guid| app_watch::known_folder(guid).ok());
        entries.extend(desktop_entries(&entries, targets, &desktop, resolve));

        Ok(entries)
    }
}

fn describe(item: &IShellItem, url_shortcuts: &HashMap<String, PathBuf>) -> Result<AppEntry> {
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

        // An internet shortcut is listed under its URL, not its file, so the
        // shell has nowhere to read `IconFile` from and draws a blank page.
        // Steam installs every game this way. The `.url` file itself does
        // resolve its icon, so point at it whenever one carries this URL.
        let icon = match url_shortcuts.get(&parsing) {
            Some(file) => IconSource::File(file.clone()),
            None => IconSource::ShellItem(full_parsing_name.clone()),
        };

        Ok(AppEntry {
            id: format!("app:{parsing}"),
            name,
            detail: detail_from(&parsing),
            target: LaunchTarget::ShellExecute(full_parsing_name),
            icon: Some(icon),
            keywords: Vec::new(),
        })
    }
}

/// The filesystem path behind an AppsFolder item, when it has one.
///
/// Win32 entries in that folder *are* shortcuts on disk, and the shell will
/// hand back where. Packaged entries have no path and return , which is
/// the correct answer rather than a failure: they are identified by their
/// model id instead.
pub fn filesystem_path(parsing_name: &str) -> Option<String> {
    // The caller may be on any thread, including one that has never touched
    // COM. Entering an apartment here rather than assuming one is what makes
    // this safe to call from the launch path.
    let _apartment = com::Apartment::enter().ok()?;

    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&com::wide(parsing_name), None).ok()?;
        com::take_pwstr(item.GetDisplayName(SIGDN_FILESYSPATH).ok()?).ok()
    }
}

/// The shortcuts found in a set of folders.
///
/// Best effort throughout: a folder or file that cannot be read costs only
/// itself, never the index. Reading a `.url` is a short file read; what a
/// `.lnk` starts is left unresolved, since asking `IShellLink` costs a
/// millisecond or more per file and most of them are never needed.
#[derive(Default)]
struct Shortcuts {
    /// Every `.url` file, keyed by the URL it opens.
    urls: HashMap<String, PathBuf>,
    /// Every `.lnk` file.
    links: Vec<PathBuf>,
}

impl Shortcuts {
    fn read(folders: &[&GUID], recursive: bool) -> Self {
        let mut found = Self::default();
        for folder in folders {
            match app_watch::known_folder(folder) {
                Ok(root) => found.collect(Path::new(&root), recursive),
                Err(error) => log::debug!("not reading a shortcut folder: {error:#}"),
            }
        }
        found
    }

    fn collect(&mut self, dir: &Path, recursive: bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if recursive {
                    self.collect(&path, recursive);
                }
            } else if has_extension(&path, "url") {
                if let Some(url) = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| shortcut_url(&bytes))
                {
                    self.urls.entry(url).or_insert(path);
                }
            } else if has_extension(&path, "lnk") {
                self.links.push(path);
            }
        }
    }
}

/// What each listed application starts, lowercased, as far as its AppsFolder
/// identity says.
///
/// A Win32 entry without a model id of its own is identified by the program
/// its shortcut starts, sometimes behind a known-folder prefix such as
/// `{7C5A40EF-...}\WinSCP\WinSCP.exe`; a URL entry by its URL. That is enough
/// to recognise a Desktop shortcut to the same thing without opening a single
/// Start Menu shortcut. An entry identified by its shortcut file instead ends
/// up here as a `.lnk` path, which [`desktop_entries`] resolves only if it has
/// to.
///
/// `known_folder` turns a known-folder id into its path.
fn listed_targets(
    listed: &[AppEntry],
    known_folder: impl Fn(&GUID) -> Option<String>,
) -> HashSet<String> {
    let mut folders: HashMap<String, Option<String>> = HashMap::new();
    let mut expand = |parsing: &str| -> String {
        let expanded = parsing
            .strip_prefix('{')
            .and_then(|rest| rest.split_once('}'))
            .and_then(|(guid, rest)| {
                let root = folders
                    .entry(guid.to_lowercase())
                    .or_insert_with(|| {
                        let guid = GUID::try_from(guid).ok()?;
                        known_folder(&guid)
                    })
                    .as_deref()?;
                Some(format!("{root}{rest}"))
            });
        expanded
            .unwrap_or_else(|| parsing.to_string())
            .to_lowercase()
    };

    listed
        .iter()
        .filter_map(|app| match &app.target {
            LaunchTarget::ShellExecute(name) => name.strip_prefix(APPS_FOLDER)?.strip_prefix('\\'),
            _ => None,
        })
        .map(&mut expand)
        .collect()
}

/// The Desktop shortcuts that add an application the AppsFolder lacks.
///
/// `resolve` reads what a `.lnk` starts. It is asked only about shortcuts whose
/// name the index does not already have, which on a typical Desktop is a
/// handful rather than all of them -- and, once the first of those turns up,
/// about the listed entries whose identity is a `.lnk` rather than a program,
/// so a renamed Desktop copy of one of them is still recognised.
fn desktop_entries(
    listed: &[AppEntry],
    mut targets: HashSet<String>,
    desktop: &Shortcuts,
    resolve: impl Fn(&Path) -> Option<String>,
) -> Vec<AppEntry> {
    let mut names: HashSet<String> = listed.iter().map(|app| app.name.to_lowercase()).collect();

    let links = desktop.links.iter().map(|path| (path, None));
    let urls = desktop
        .urls
        .iter()
        .filter(|(url, _)| is_application_url(url))
        .map(|(url, path)| (path, Some(url.as_str())));

    // Shortest name first, so two shortcuts to one program keep the tidy name
    // rather than `Program.exe - Shortcut`.
    let mut candidates: Vec<(String, &PathBuf, Option<&str>)> = links
        .chain(urls)
        .filter_map(|(path, url)| Some((shortcut_name(path)?, path, url)))
        .collect();
    candidates.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));

    let mut entries = Vec::new();
    let mut listed_links_resolved = false;
    for (name, path, url) in candidates {
        if names.contains(&name.to_lowercase()) {
            continue;
        }
        if !listed_links_resolved {
            listed_links_resolved = true;
            let started: Vec<String> = targets
                .iter()
                .filter(|target| target.ends_with(".lnk"))
                .filter_map(|link| resolve(Path::new(link)))
                .map(|target| target.to_lowercase())
                .collect();
            targets.extend(started);
        }
        let target = match url {
            Some(url) => url.to_lowercase(),
            None => match resolve(path).map(|target| target.to_lowercase()) {
                Some(target) if target.ends_with(".exe") => target,
                // A shortcut to a folder or a document is not an application.
                _ => continue,
            },
        };
        let Some(location) = path.to_str() else {
            continue;
        };
        if !targets.insert(target) {
            continue;
        }
        names.insert(name.to_lowercase());
        entries.push(AppEntry {
            id: format!("app:{location}"),
            name,
            detail: None,
            target: LaunchTarget::ShellExecute(location.to_string()),
            icon: Some(IconSource::File(path.clone())),
            keywords: Vec::new(),
        });
    }
    entries
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

/// The title a shortcut file gives its application.
///
/// The file name without `.lnk` or `.url`, and without a trailing `.exe` that
/// Explorer leaves on shortcuts made from an executable.
fn shortcut_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let stem = match stem.len().checked_sub(4) {
        Some(cut) if stem.is_char_boundary(cut) && stem[cut..].eq_ignore_ascii_case(".exe") => {
            &stem[..cut]
        }
        _ => stem,
    };
    let stem = stem.trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Whether a URL opens an application rather than a page.
///
/// `steam://rungameid/...` starts a game; a bookmark to a web page is not an
/// application, however it was saved.
fn is_application_url(url: &str) -> bool {
    const WEB: &[&str] = &["http", "https", "ftp", "file", "mailto"];
    url.split_once(':')
        .is_some_and(|(scheme, _)| !WEB.iter().any(|web| web.eq_ignore_ascii_case(scheme)))
}

/// What a `.lnk` file points at.
pub(crate) fn shortcut_target(path: &str) -> Option<String> {
    let _apartment = com::Apartment::enter().ok()?;
    LinkReader::new()?.target(path)
}

/// Reads `.lnk` files through `IShellLink`.
///
/// Rather than parsing the file: the format is undocumented in the parts that
/// matter, and the shell resolves relative paths, environment variables and
/// the 32/64-bit Program Files split on the way out. One instance is loaded
/// with each file in turn, because creating one per file is most of the cost
/// of reading a whole Start Menu.
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

    fn target(&self, path: &str) -> Option<String> {
        unsafe {
            self.file
                .Load(PCWSTR(com::wide(path).as_ptr()), STGM_READ)
                .ok()?;

            let mut buffer = [0u16; MAX_PATH as usize];
            self.link
                .GetPath(&mut buffer, std::ptr::null_mut(), 0)
                .ok()?;

            let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
        }
    }
}

/// The `URL=` value of an internet shortcut's `[InternetShortcut]` section.
fn shortcut_url(contents: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(contents);
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[InternetShortcut]");
        } else if in_section
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("URL")
        {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
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
    fn an_internet_shortcut_yields_its_url() {
        let steam = b"[{000214A0-0000-0000-C000-000000000046}]\r\nProp3=19,0\r\n\
            [InternetShortcut]\r\nIDList=\r\nIconIndex=0\r\n\
            URL=steam://rungameid/2379780\r\nIconFile=D:\\games\\x.ico\r\n";
        assert_eq!(
            shortcut_url(steam).as_deref(),
            Some("steam://rungameid/2379780")
        );
    }

    #[test]
    fn a_url_outside_the_shortcut_section_is_ignored() {
        assert_eq!(shortcut_url(b"[Other]\nURL=https://example.com\n"), None);
        assert_eq!(shortcut_url(b"[InternetShortcut]\nURL=\n"), None);
    }

    #[test]
    fn only_application_schemes_count_as_applications() {
        assert!(is_application_url("steam://rungameid/2379780"));
        assert!(is_application_url("com.epicgames.launcher://apps/x"));
        assert!(!is_application_url("https://www.amazon.com/dp/098478280X"));
        assert!(!is_application_url("HTTP://example.com"));
        assert!(!is_application_url("no scheme at all"));
    }

    #[test]
    fn a_shortcut_is_named_after_its_file() {
        let name = |file: &str| shortcut_name(Path::new(file));
        assert_eq!(
            name(r"C:\Desktop\Modrinth App.lnk").as_deref(),
            Some("Modrinth App")
        );
        assert_eq!(
            name(r"C:\Desktop\YoutubeDownloader.exe.lnk").as_deref(),
            Some("YoutubeDownloader")
        );
        assert_eq!(name(r"C:\Desktop\Balatro.url").as_deref(), Some("Balatro"));
    }

    fn app(name: &str) -> AppEntry {
        AppEntry {
            id: format!("app:{name}"),
            name: name.into(),
            detail: None,
            target: LaunchTarget::ShellExecute(name.into()),
            icon: None,
            keywords: Vec::new(),
        }
    }

    #[test]
    fn desktop_shortcuts_add_only_what_start_does_not_list() {
        let listed = [
            app("Discord"),
            app(r"shell:AppsFolder\D:\Apps\Clip\CLIPStudio.exe"),
            app(r"shell:AppsFolder\steam://rungameid/1"),
        ];
        let desktop = Shortcuts {
            urls: HashMap::from([
                ("steam://rungameid/1".into(), PathBuf::from("Balatro.url")),
                ("https://amazon.com/book".into(), PathBuf::from("Book.url")),
            ]),
            links: [
                "Discord.lnk",
                "CLIPStudio.exe - Shortcut.lnk",
                "Modrinth App.lnk",
                "Modrinth.lnk",
                "Notes.lnk",
            ]
            .map(PathBuf::from)
            .to_vec(),
        };
        let resolve = |path: &Path| {
            let target = match path.to_str()? {
                "Discord.lnk" => panic!("a name the index has needs no resolving"),
                "CLIPStudio.exe - Shortcut.lnk" => r"D:\Apps\Clip\CLIPStudio.exe",
                "Notes.lnk" => r"C:\Notes\todo.txt",
                _ => r"C:\Modrinth\Modrinth App.exe",
            };
            Some(target.to_string())
        };

        let targets = listed_targets(&listed, |_| None);
        let added = desktop_entries(&listed, targets, &desktop, resolve);
        let names: Vec<&str> = added.iter().map(|app| app.name.as_str()).collect();
        // Two shortcuts to one program keep the shorter name; a document, a web
        // page and everything Start already lists are left out.
        assert_eq!(names, ["Modrinth"]);
        assert_eq!(
            added[0].icon,
            Some(IconSource::File(PathBuf::from("Modrinth.lnk")))
        );
    }

    #[test]
    fn a_desktop_copy_of_an_entry_listed_by_its_shortcut_is_recognised() {
        // Start lists the program by its shortcut file, and the Desktop copy
        // has a different name, so only what both start can match them.
        let listed = [app(r"shell:AppsFolder\C:\Start\Editor.lnk")];
        let desktop = Shortcuts {
            urls: HashMap::new(),
            links: vec![PathBuf::from("My Editor.lnk")],
        };
        let resolve = |path: &Path| {
            let target = match path.to_str()? {
                r"c:\start\editor.lnk" | "My Editor.lnk" => r"C:\Editor\Editor.exe",
                other => panic!("resolved an unexpected shortcut {other}"),
            };
            Some(target.to_string())
        };

        let targets = listed_targets(&listed, |_| None);
        assert!(desktop_entries(&listed, targets, &desktop, resolve).is_empty());
    }

    #[test]
    fn listed_shortcuts_are_not_resolved_when_nothing_on_the_desktop_is_new() {
        let listed = [app("Editor"), app(r"shell:AppsFolder\C:\Start\Editor.lnk")];
        let desktop = Shortcuts {
            urls: HashMap::new(),
            links: vec![PathBuf::from("Editor.lnk")],
        };
        let resolve = |path: &Path| -> Option<String> { panic!("resolved {path:?}") };

        let targets = listed_targets(&listed, |_| None);
        assert!(desktop_entries(&listed, targets, &desktop, resolve).is_empty());
    }

    #[test]
    fn a_known_folder_prefix_is_expanded() {
        // FOLDERID_ProgramFilesX86, which is where WinSCP's entry points.
        let listed = [
            app(r"shell:AppsFolder\{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\WinSCP\WinSCP.exe"),
            app(r"shell:AppsFolder\{00000000-0000-0000-0000-000000000000}\Gone.exe"),
        ];
        let known_folder = |guid: &GUID| {
            (*guid == windows::Win32::UI::Shell::FOLDERID_ProgramFilesX86)
                .then(|| r"C:\Program Files (x86)".to_string())
        };

        let targets = listed_targets(&listed, known_folder);
        assert!(targets.contains(r"c:\program files (x86)\winscp\winscp.exe"));
        // A folder the system does not know is kept as it was written.
        assert!(targets.contains(r"{00000000-0000-0000-0000-000000000000}\gone.exe"));
    }

    #[test]
    fn the_start_menu_is_read_with_its_subfolders_and_the_desktop_without() {
        let root = std::env::temp_dir().join(format!("lumen-shortcuts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Steam")).unwrap();
        let game = "[InternetShortcut]\r\nURL=steam://rungameid/1\r\n";
        std::fs::write(root.join("Steam").join("Game.URL"), game).unwrap();
        std::fs::write(root.join("Copy of Game.url"), game).unwrap();
        std::fs::write(root.join("Editor.LNK"), b"").unwrap();
        std::fs::write(root.join("Steam").join("Tool.lnk"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();

        let mut nested = Shortcuts::default();
        nested.collect(&root, true);
        let mut flat = Shortcuts::default();
        flat.collect(&root, false);
        let _ = std::fs::remove_dir_all(&root);

        let mut links = nested.links.clone();
        links.sort();
        assert_eq!(
            links,
            [root.join("Editor.LNK"), root.join("Steam").join("Tool.lnk")]
        );
        // Two files for one URL: either may win, but only one is kept.
        assert_eq!(nested.urls.len(), 1);

        assert_eq!(flat.links, [root.join("Editor.LNK")]);
        assert_eq!(
            flat.urls,
            HashMap::from([("steam://rungameid/1".into(), root.join("Copy of Game.url"))])
        );
    }

    #[test]
    fn win32_shortcuts_get_no_detail() {
        // The parsing name is just the shortcut file, which would only repeat
        // the title back at the user.
        assert_eq!(detail_from("Firefox.lnk"), None);
    }
}
