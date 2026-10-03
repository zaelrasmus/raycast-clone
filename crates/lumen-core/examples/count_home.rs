//! Slice 0 of the file index (F16, `docs/FILE-SEARCH.md`): how many entries a
//! real home folder has after the skip rules.
//!
//! The design's estimate of 100 000 to 1 000 000 is a guess until something
//! walks a real disk with the rules the index will use, so this walks it with
//! the index's own walk (`file_index::walk`) and counts.
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

use ignore::WalkBuilder;
use lumen_core::file_index::{WalkRules, walk};

fn count(root: &Path) {
    // The index's own walk, with its defaults and no ceiling: the count is
    // what the rules allow, not what the index would keep.
    let rules = WalkRules {
        scopes: vec![root.to_path_buf()],
        exclusions: Vec::new(),
        ignore_patterns: Vec::new(),
        use_ignore_files: true,
        include_hidden: false,
        ceiling: usize::MAX,
    };
    let start = Instant::now();
    let walked = walk(&rules);
    let elapsed = start.elapsed();

    let folders = walked.entries.iter().filter(|entry| entry.is_dir).count();
    let files = walked.entries.len() - folders;
    let mut children: HashMap<OsString, usize> = HashMap::new();
    for entry in &walked.entries {
        if let Some(child) = entry
            .path
            .strip_prefix(root)
            .ok()
            .and_then(|rest| rest.components().next())
        {
            *children
                .entry(child.as_os_str().to_os_string())
                .or_default() += 1;
        }
    }

    println!("root:    {}", root.display());
    println!("files:   {files}");
    println!("folders: {folders}");
    println!("total:   {}", files + folders);
    println!("elapsed: {:.2} s (one thread)", elapsed.as_secs_f64());

    let mut children = children.into_iter().collect::<Vec<_>>();
    children.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "
largest top-level children:"
    );
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
