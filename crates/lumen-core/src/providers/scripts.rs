//! Script commands (tier T1).
//!
//! Any executable that Lumen can run, with its metadata declared in comment
//! headers. The plan is emphatic about where this sits:
//!
//! > It costs days and covers a large fraction of real use cases without
//! > touching the extension system. It is the best effort-to-value ratio in the
//! > entire project and must not be deferred to F4.
//!
//! # Comment headers, and AD-08
//!
//! The metadata is read from the file's first lines, never by running it. That
//! is AD-08 stated for T1: when the user types in root search, zero script
//! processes are running, and a command appears because it is in the local index
//! rather than because anything reported it. A script is executed only when its
//! row is activated.
//!
//! ```text
//! #!/usr/bin/env pwsh
//! # @lumen.title Restart Explorer
//! # @lumen.subtitle Windows
//! # @lumen.mode silent
//! ```
//!
//! # Why the comment prefix is not checked
//!
//! A script may be PowerShell (`#`), a batch file (`::` or `REM`), JavaScript
//! (`//`) or a shell script (`#`). Rather than keep a table of comment syntaxes
//! and get one wrong, a header line is any line containing `@lumen.<key>`
//! before the first non-comment content. The prefix is whatever the language
//! needs and Lumen does not care what it is.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::item::{Action, Item, Kind};
use crate::providers::Provider;

/// How many lines are scanned for headers.
///
/// Bounded so that pointing the scripts folder at something enormous by mistake
/// reads a few lines rather than the whole file.
const HEADER_LINES: usize = 30;

/// What to do with a script's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptMode {
    /// Run it and ignore whatever it prints. For scripts that act rather than
    /// answer: restart a service, toggle a setting.
    #[default]
    Silent,
    /// Run it and put its output on the clipboard.
    ///
    /// The launcher has already been dismissed by the time a script finishes,
    /// so there is nowhere on screen to put the output -- and the clipboard is
    /// where the user wanted it anyway. Showing script output in the list is a
    /// view extension's job (F4), not T1's.
    Copy,
}

/// One script command, as found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub path: PathBuf,
    pub title: String,
    pub subtitle: Option<String>,
    pub mode: ScriptMode,
    pub keywords: Vec<String>,
}

impl Script {
    fn into_item(self) -> Item {
        Item {
            // Keyed on the path, so renaming the file is a new command and
            // editing it keeps the frecency it has earned.
            id: format!("script:{}", self.path.display()),
            title: self.title,
            subtitle: self.subtitle,
            icon: None,
            keywords: self.keywords,
            actions: vec![Action::RunScript(ScriptRun {
                path: self.path,
                mode: self.mode,
            })],
            kind: Kind::Script,
        }
    }
}

/// What the daemon needs in order to run a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRun {
    pub path: PathBuf,
    pub mode: ScriptMode,
}

/// Read the script commands in `dir`.
///
/// Touches the disk, so it belongs on a background task -- never in
/// [`Provider::catalogue`], which runs inside the frame budget.
pub fn scan(dir: &Path) -> Vec<Script> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    // Sorted so the index is stable between runs rather than in whatever order
    // the filesystem happened to hand the files over.
    paths.sort();

    paths
        .into_iter()
        .filter(|path| path.is_file())
        .filter_map(|path| match read(&path) {
            Ok(script) => Some(script),
            Err(reason) => {
                log::debug!("skipping {}: {reason}", path.display());
                None
            }
        })
        .collect()
}

/// Parse one script's headers.
///
/// An error here means "this is not a script command", which is the normal case
/// for a README sitting in the folder -- so it is a plain message, not an
/// `anyhow::Error` with a chain nobody will read.
fn read(path: &Path) -> Result<Script, String> {
    let source = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let headers = parse_headers(&source);

    // A title is the one required header: without it there is nothing to show
    // in the list, and guessing from the file name would quietly index every
    // stray file in the folder.
    let title = headers
        .iter()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.clone())
        .ok_or("no @lumen.title header")?;

    let value = |wanted: &str| {
        headers
            .iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.clone())
    };

    let mode = match value("mode").as_deref() {
        None | Some("silent") => ScriptMode::Silent,
        Some("copy") => ScriptMode::Copy,
        Some(other) => {
            // Reported rather than rejected: an unknown mode is most likely a
            // typo, and refusing to index the script would hide the command
            // instead of explaining the mistake.
            log::warn!(
                "{}: unknown @lumen.mode {other:?}, treating it as silent",
                path.display()
            );
            ScriptMode::Silent
        }
    };

    Ok(Script {
        path: path.to_path_buf(),
        title,
        subtitle: value("subtitle"),
        mode,
        keywords: value("keywords")
            .map(|keywords| {
                keywords
                    .split(',')
                    .map(|keyword| keyword.trim().to_string())
                    .filter(|keyword| !keyword.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Extract `@lumen.<key> <value>` pairs from the top of a file.
fn parse_headers(source: &str) -> Vec<(String, String)> {
    source
        .lines()
        .take(HEADER_LINES)
        .filter_map(|line| {
            let (_, rest) = line.split_once("@lumen.")?;
            let rest = rest.trim();

            // Split on the first run of whitespace: the key is one word, the
            // value is the rest of the line and may contain spaces.
            let (key, value) = match rest.find(char::is_whitespace) {
                Some(at) => (&rest[..at], rest[at..].trim()),
                None => (rest, ""),
            };

            (!key.is_empty() && !value.is_empty()).then(|| (key.to_lowercase(), value.to_string()))
        })
        .collect()
}

/// The scripts found at startup, offered as a catalogue.
#[derive(Default)]
pub struct ScriptProvider {
    items: Vec<Item>,
}

impl ScriptProvider {
    /// Build from an already-scanned list.
    ///
    /// Takes the scan result rather than a directory so that the disk work
    /// stays where the caller put it -- on a background task -- and cannot be
    /// pulled into the frame budget by a later refactor.
    pub fn new(scripts: Vec<Script>) -> Self {
        Self {
            items: scripts.into_iter().map(Script::into_item).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Provider for ScriptProvider {
    fn id(&self) -> &'static str {
        "script"
    }

    fn catalogue(&self) -> Vec<Item> {
        self.items.clone()
    }
}

/// The example written into the scripts folder on first run.
pub const EXAMPLE: &str = include_str!("../../../../assets/scripts/example.ps1");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headers_whatever_the_comment_prefix_is() {
        // PowerShell, batch, JavaScript and shell all in one file: Lumen does
        // not care which language it is, only that the marker appears.
        let source = "\
#!/usr/bin/env pwsh
# @lumen.title Restart Explorer
:: @lumen.subtitle Windows
// @lumen.mode copy
REM @lumen.keywords shell, explorer
";
        let headers = parse_headers(source);

        assert_eq!(headers.len(), 4);
        assert_eq!(headers[0], ("title".into(), "Restart Explorer".into()));
        assert_eq!(headers[1], ("subtitle".into(), "Windows".into()));
        assert_eq!(headers[2], ("mode".into(), "copy".into()));
    }

    #[test]
    fn a_value_may_contain_spaces() {
        let headers = parse_headers("# @lumen.title  Restart the Windows shell  ");
        assert_eq!(headers[0].1, "Restart the Windows shell");
    }

    #[test]
    fn a_header_with_no_value_is_ignored() {
        assert!(parse_headers("# @lumen.title").is_empty());
        assert!(parse_headers("# @lumen.").is_empty());
    }

    #[test]
    fn keys_are_case_insensitive() {
        let headers = parse_headers("# @lumen.TITLE Thing");
        assert_eq!(headers[0].0, "title");
    }

    /// Bounded so a stray enormous file in the folder costs a few lines, not a
    /// full read.
    #[test]
    fn only_the_first_lines_are_scanned() {
        let mut source = "\n".repeat(HEADER_LINES + 5);
        source.push_str("# @lumen.title Too Late");

        assert!(parse_headers(&source).is_empty());
    }

    #[test]
    fn a_file_with_no_title_is_not_a_script_command() {
        let dir = temp_dir("no-title");
        let path = dir.join("readme.txt");
        std::fs::write(&path, "just some notes").unwrap();

        assert!(read(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scanning_finds_scripts_and_skips_everything_else() {
        let dir = temp_dir("scan");
        std::fs::write(
            dir.join("a.ps1"),
            "# @lumen.title Alpha\n# @lumen.mode copy\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "not a script").unwrap();

        let scripts = scan(&dir);

        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].title, "Alpha");
        assert_eq!(scripts[0].mode, ScriptMode::Copy);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_mode_falls_back_to_silent_rather_than_hiding_the_command() {
        let dir = temp_dir("mode");
        let path = dir.join("odd.ps1");
        std::fs::write(&path, "# @lumen.title Odd\n# @lumen.mode invented\n").unwrap();

        let script = read(&path).expect("the command must still be indexed");
        assert_eq!(script.mode, ScriptMode::Silent);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_script_becomes_a_runnable_item() {
        let script = Script {
            path: PathBuf::from("C:/scripts/a.ps1"),
            title: "Alpha".into(),
            subtitle: Some("Tools".into()),
            mode: ScriptMode::Copy,
            keywords: vec!["shell".into()],
        };

        let item = script.clone().into_item();

        assert_eq!(item.title, "Alpha");
        assert_eq!(item.kind, Kind::Script);
        assert_eq!(
            item.primary(),
            Action::RunScript(ScriptRun {
                path: script.path,
                mode: ScriptMode::Copy
            })
        );
    }

    /// The shipped example has to be a valid script command, or the first thing
    /// a new user sees in their scripts folder is something that does not work.
    #[test]
    fn the_bundled_example_parses() {
        let headers = parse_headers(EXAMPLE);
        assert!(
            headers.iter().any(|(key, _)| key == "title"),
            "the example must declare a title"
        );
    }

    #[test]
    fn scanning_a_missing_directory_is_not_an_error() {
        assert!(scan(Path::new("no/such/directory")).is_empty());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-scripts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
