//! The pane beside *Search Files*: a preview chosen by type, and the
//! metadata (F16).
//!
//! Everything here reads the disk -- a file's first bytes, its times, a
//! folder's listing -- so it is only ever called off the UI thread, once per
//! row the selection lands on.

use std::path::{Path, PathBuf};

use jiff::{Timestamp, Zoned};

use super::Found;
use super::category::{Category, extension};
use crate::view::{ItemDetail, MetadataRow, Tone};

/// The most of a file read for its preview. Enough for the first screen of
/// any text file; a log of a gigabyte costs the same as a note.
pub const PREVIEW_BYTES: usize = 64 * 1024;

/// The most lines shown. The pane shows the start of a file, not the file:
/// a 64 KB file of short lines is thousands of them, each laid out on every
/// frame the pane is drawn.
pub const PREVIEW_LINES: usize = 200;

/// What the pane shows above the metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// The picture itself, scaled to fit.
    Image,
    /// The first lines, as written: code, configuration and logs. Its own
    /// case, apart from prose, because slice 3b draws it in the palette's
    /// monospace family.
    Code,
    /// The first lines of text meant to be read: `.txt`, `.md`.
    Prose,
    /// The metadata only.
    None,
}

/// Pictures the pane can draw: what the renderer decodes, which is not every
/// image the *Images* filter holds (a `.psd` is an image and not a preview).
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp"];

/// Text meant to be read rather than run.
const PROSE: &[&str] = &["txt", "md", "markdown", "rst", "org"];

/// Text read best in columns, outside the *Code* category's extensions.
const PLAIN_TEXT: &[&str] = &["log", "csv", "tsv"];

/// Which preview a file's name gets. A folder gets none.
pub fn preview_of(name: &str, is_dir: bool) -> Preview {
    if is_dir {
        return Preview::None;
    }
    let Some(extension) = extension(name) else {
        return Preview::None;
    };
    let extension = extension.as_str();
    if PICTURES.contains(&extension) {
        Preview::Image
    } else if PROSE.contains(&extension) {
        Preview::Prose
    } else if PLAIN_TEXT.contains(&extension) || Category::of(name, false) == Some(Category::Code) {
        Preview::Code
    } else {
        Preview::None
    }
}

/// The text to preview from a file's first bytes, or `None` when they are
/// not text: invalid UTF-8, or a NUL anywhere -- what a binary file has and
/// a text file does not.
///
/// The bytes may have been cut mid-character by [`PREVIEW_BYTES`]; a
/// character left incomplete at the end is dropped, not taken as a sign of
/// a binary file. At most [`PREVIEW_LINES`] lines are kept.
pub fn text_from(bytes: &[u8]) -> Option<String> {
    let bytes = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    if bytes.contains(&0) {
        return None;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        // Valid up to a character the cut left incomplete.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut end = text.len();
    if let Some((at, _)) = text.match_indices('\n').nth(PREVIEW_LINES - 1) {
        end = at;
    }
    Some(text[..end].trim_end().to_string())
}

/// The first [`PREVIEW_BYTES`] of the file at `path`, as preview text.
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(PREVIEW_BYTES);
    file.take(PREVIEW_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    text_from(&bytes)
}

/// What [`describe`] needs besides the entry, carried to the thread that
/// reads the disk.
#[derive(Debug, Clone)]
pub struct Describer {
    home: Option<PathBuf>,
    now: Zoned,
}

impl Describer {
    /// Reads the system's time zone the first time: off the UI thread.
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            now: crate::snippets::now(),
        }
    }

    pub fn describe(&self, found: &Found) -> ItemDetail {
        describe(&found.path, found.is_dir, self.home.as_deref(), &self.now)
    }
}

/// The pane for the entry at `path`: its preview, then Name, Where, Type,
/// Size (a folder's item count instead), Created and Modified. `home`
/// shortens *Where* to `~`; `now` says what *Today* is.
///
/// What cannot be read is left out rather than shown as an error: a file
/// that vanished between the query and the selection still has a name.
pub fn describe(path: &Path, is_dir: bool, home: Option<&Path>, now: &Zoned) -> ItemDetail {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let mut detail = summary(path, is_dir, home);
    match preview_of(&name, is_dir) {
        Preview::Image => detail.image = Some(path.to_path_buf()),
        Preview::Code | Preview::Prose => detail.text = read_text(path),
        Preview::None => {}
    }

    let metadata = std::fs::metadata(path).ok();
    if is_dir {
        if let Ok(entries) = std::fs::read_dir(path) {
            let count = entries.count();
            let items = if count == 1 { "item" } else { "items" };
            detail
                .metadata
                .push(row("Items", &format!("{count} {items}")));
        }
    } else if let Some(metadata) = &metadata {
        detail.metadata.push(row(
            "Size",
            &crate::clipboard::images::size_label(metadata.len()),
        ));
    }
    let moment = |time: std::io::Result<std::time::SystemTime>| {
        let at = Timestamp::try_from(time.ok()?).ok()?;
        Some(crate::dates::moment(at, now))
    };
    if let Some(metadata) = &metadata {
        if let Some(created) = moment(metadata.created()) {
            detail.metadata.push(row("Created", &created));
        }
        if let Some(modified) = moment(metadata.modified()) {
            detail.metadata.push(row("Modified", &modified));
        }
    }
    detail
}

/// What is known without touching the disk: Name, Where and Type. Shown
/// until [`describe`] has answered, so the pane is never blank.
pub fn summary(path: &Path, is_dir: bool, home: Option<&Path>) -> ItemDetail {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let place = path
        .parent()
        .map(|parent| crate::file_index::for_display(parent, home))
        .unwrap_or_default();
    let kind = if is_dir {
        "Folder".to_string()
    } else {
        extension(&name)
            .map(|extension| extension.to_ascii_uppercase())
            .unwrap_or_else(|| "File".to_string())
    };
    ItemDetail {
        markdown: String::new(),
        text: None,
        image: None,
        metadata: vec![row("Name", &name), row("Where", &place), row("Type", &kind)],
    }
}

fn row(label: &str, value: &str) -> MetadataRow {
    MetadataRow {
        label: label.into(),
        value: value.into(),
        tone: Tone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::walk::tests::Tree;

    fn now() -> Zoned {
        "2026-09-30T21:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    #[test]
    fn each_type_gets_its_preview() {
        assert_eq!(preview_of("Screenshot.PNG", false), Preview::Image);
        assert_eq!(preview_of("carbon.json", false), Preview::Code);
        assert_eq!(preview_of("main.rs", false), Preview::Code);
        assert_eq!(preview_of("lumen.log", false), Preview::Code);
        assert_eq!(preview_of("notes.md", false), Preview::Prose);
        assert_eq!(preview_of("todo.txt", false), Preview::Prose);
        assert_eq!(preview_of("report.pdf", false), Preview::None);
        assert_eq!(preview_of("design.psd", false), Preview::None);
        assert_eq!(preview_of("Makefile", false), Preview::None);
        assert_eq!(preview_of("styles.json", true), Preview::None);
    }

    #[test]
    fn text_is_previewed_as_written() {
        assert_eq!(
            text_from(b"{\n  \"a\": 1\n}\n").as_deref(),
            Some("{\n  \"a\": 1\n}")
        );
    }

    #[test]
    fn a_nul_byte_means_it_is_not_text() {
        assert_eq!(text_from(b"MZ\0\0binary"), None);
        assert_eq!(text_from(&[0xff, 0xfe, 0x41]), None);
    }

    #[test]
    fn only_the_first_64_kb_are_read() {
        let long = "a".repeat(PREVIEW_BYTES + 1000);
        assert_eq!(text_from(long.as_bytes()).unwrap().len(), PREVIEW_BYTES);
    }

    /// The cut can land inside a character; that is the cut, not binary.
    #[test]
    fn a_character_cut_at_the_limit_is_dropped_not_refused() {
        let mut bytes = "a".repeat(PREVIEW_BYTES - 1).into_bytes();
        bytes.extend_from_slice("é".as_bytes());
        let text = text_from(&bytes).unwrap();
        assert_eq!(text.len(), PREVIEW_BYTES - 1);
    }

    #[test]
    fn only_the_first_lines_are_kept() {
        let lines = (0..PREVIEW_LINES + 50)
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let text = text_from(lines.as_bytes()).unwrap();
        assert_eq!(text.lines().count(), PREVIEW_LINES);
    }

    #[test]
    fn a_file_is_described_by_its_preview_and_metadata() {
        let tree = Tree::new("preview-file");
        tree.write("config/carbon.json", "{ \"name\": \"Carbon\" }");
        let path = tree.path("config/carbon.json");
        let detail = describe(&path, false, Some(&tree.0), &now());

        assert_eq!(detail.text.as_deref(), Some("{ \"name\": \"Carbon\" }"));
        assert_eq!(detail.image, None);
        let labels: Vec<&str> = detail.metadata.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Name", "Where", "Type", "Size", "Created", "Modified"]
        );
        let value = |label: &str| {
            detail
                .metadata
                .iter()
                .find(|row| row.label == label)
                .map(|row| row.value.clone())
                .unwrap()
        };
        assert_eq!(value("Name"), "carbon.json");
        assert_eq!(
            value("Where"),
            format!("~{}config", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(value("Type"), "JSON");
        assert_eq!(value("Size"), "20 bytes");
    }

    #[test]
    fn a_folder_counts_its_items_instead_of_a_size() {
        let tree = Tree::new("preview-folder");
        tree.file("styles/a.json").file("styles/b.json");
        let detail = describe(&tree.path("styles"), true, Some(&tree.0), &now());
        let rows: Vec<(&str, &str)> = detail
            .metadata
            .iter()
            .take(4)
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Name", "styles"),
                ("Where", "~"),
                ("Type", "Folder"),
                ("Items", "2 items")
            ]
        );
        assert_eq!(detail.text, None);
    }

    #[test]
    fn a_picture_is_shown_and_a_binary_file_is_not() {
        let tree = Tree::new("preview-binary");
        tree.write("cat.png", "not really a png");
        std::fs::write(tree.path("data.json"), b"\0\0\0").unwrap();
        let picture = describe(&tree.path("cat.png"), false, None, &now());
        assert_eq!(picture.image, Some(tree.path("cat.png")));
        assert_eq!(picture.text, None);
        let binary = describe(&tree.path("data.json"), false, None, &now());
        assert_eq!(binary.text, None);
    }
}
