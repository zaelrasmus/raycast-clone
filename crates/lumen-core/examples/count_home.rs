//! Slice 0 of the file index (F16, `docs/FILE-SEARCH.md`): how many entries a
//! real home folder has after the skip rules.
//!
//! The design's estimate of 100 000 to 1 000 000 is a guess until something
//! walks a real disk with the rules the index will use, so this walks it with
//! the `ignore` crate configured as the design describes and counts.
//!
//! ```text
//! cargo run --release -p lumen-core --example count_home [-- <root>]
//! cargo run --release -p lumen-core --example count_home -- --check-hidden
//! ```
//!
//! `--check-hidden` verifies one claim of the design instead: that `ignore`
//! skips a file with the Windows hidden attribute and no leading dot.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ignore::{DirEntry, WalkBuilder};

/// A copy of `SKIPPED` in `providers/files.rs`, for measurement only: the
/// original is private, and this example must count what the index would skip.
const SKIPPED: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "obj",
    "bin",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".git",
    ".svn",
    ".hg",
    ".cache",
    ".gradle",
    ".idea",
    ".vs",
    ".vscode",
    "appdata",
    "application data",
    "$recycle.bin",
    "system volume information",
];

/// The skip list as a directory filter. The root itself is never skipped: a
/// walk started at a folder named `build` should still walk it.
fn keep(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !SKIPPED
        .iter()
        .any(|skipped| name.eq_ignore_ascii_case(skipped))
}

/// The walker as the index would configure it: hidden entries, ignore files and
/// `.lumenignore` all honoured, links not followed, and no depth limit.
fn walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .ignore(true)
        .add_custom_ignore_filename(".lumenignore")
        .follow_links(false)
        .max_depth(None)
        .filter_entry(keep);
    builder
}

fn count(root: &Path) {
    let start = Instant::now();
    let mut files = 0usize;
    let mut folders = 0usize;
    let mut errors = 0usize;
    let mut children: HashMap<OsString, usize> = HashMap::new();

    for result in walker(root).build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            folders += 1;
        } else {
            files += 1;
        }
        if let Some(child) = entry
            .path()
            .strip_prefix(root)
            .ok()
            .and_then(|rest| rest.components().next())
        {
            *children
                .entry(child.as_os_str().to_os_string())
                .or_default() += 1;
        }
    }
    let elapsed = start.elapsed();

    println!("root:    {}", root.display());
    println!("files:   {files}");
    println!("folders: {folders}");
    println!("total:   {}", files + folders);
    println!("errors:  {errors}");
    println!("elapsed: {:.2} s (one thread)", elapsed.as_secs_f64());

    let mut children = children.into_iter().collect::<Vec<_>>();
    children.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nlargest top-level children:");
    for (name, entries) in children.into_iter().take(10) {
        println!("{entries:>10}  {}", name.to_string_lossy());
    }
}

/// Whether `hidden(true)` skips a file hidden by attribute rather than by name.
fn check_hidden() {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/hidden-check"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the scratch folder");
    std::fs::write(dir.join("visible.txt"), "visible").expect("write visible.txt");
    std::fs::write(dir.join("secret.txt"), "secret").expect("write secret.txt");

    let status = Command::new("attrib")
        .arg("+h")
        .arg(dir.join("secret.txt"))
        .status()
        .expect("run attrib");
    println!("attrib +h secret.txt: {status}");

    for hidden in [true, false] {
        let mut yielded = WalkBuilder::new(&dir)
            .hidden(hidden)
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.depth() > 0)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        yielded.sort();
        println!("hidden({hidden}) yielded: {yielded:?}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let argument = std::env::args().nth(1);
    if argument.as_deref() == Some("--check-hidden") {
        check_hidden();
        return;
    }

    let root = match argument {
        Some(root) => PathBuf::from(root),
        None => directories::UserDirs::new()
            .expect("a home folder")
            .home_dir()
            .to_path_buf(),
    };
    count(&root);
}
