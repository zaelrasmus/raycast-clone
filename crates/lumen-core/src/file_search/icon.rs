//! Which shell icons a file shares with every other file of its type (F16).
//!
//! A row in *Search Files* draws the shell's icon for its entry, and the
//! renderer keeps what it extracted. Keeping it by path would hold one
//! picture per file ever shown -- hundreds an afternoon, each a texture --
//! where most of them are the same picture: every `.json` has the editor's
//! icon. So an icon is kept by **type** when the shell's icon for the type
//! is all a file of it can have, and by **path** otherwise:
//!
//! - **By path:** folders (a folder may have its own icon, and Desktop and
//!   Downloads do), files that carry their own (`.exe`, `.lnk`, `.ico`,
//!   `.url`, ...), and anything the shell may draw as a thumbnail --
//!   pictures, video, audio with its cover, documents with a first page --
//!   which is every type not known to be plain.
//! - **By type:** code, configuration, plain text and archives, whose icon
//!   is their handler's; and a name with no extension, which has the
//!   generic one.
//!
//! The renderer keeps the by-type icons for the life of the process -- one
//! per extension -- and forgets the by-path ones when the launcher hides.

use super::category::{Category, extension};
use super::preview::{Preview, preview_of};

/// The type whose icon a file has, lower case without the dot (empty for a
/// name without an extension), or `None` when its icon is its own.
pub fn shared(name: &str, is_dir: bool) -> Option<String> {
    if is_dir {
        return None;
    }
    let Some(extension) = extension(name) else {
        return Some(String::new());
    };
    let plain = matches!(
        Category::of(name, false),
        Some(Category::Code | Category::Archives)
    ) || matches!(preview_of(name, false), Preview::Code | Preview::Prose);
    plain.then_some(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_types_share_one_icon() {
        assert_eq!(shared("carbon.json", false).as_deref(), Some("json"));
        assert_eq!(shared("Main.RS", false).as_deref(), Some("rs"));
        assert_eq!(shared("notes.TXT", false).as_deref(), Some("txt"));
        assert_eq!(shared("lumen.log", false).as_deref(), Some("log"));
        assert_eq!(shared("backup.zip", false).as_deref(), Some("zip"));
        assert_eq!(shared("Makefile", false).as_deref(), Some(""));
    }

    #[test]
    fn an_icon_of_its_own_or_a_thumbnail_is_kept_by_path() {
        for name in [
            "setup.exe",
            "Discord.lnk",
            "lumen.ico",
            "Game.url",
            "photo.png",
            "clip.mp4",
            "song.mp3",
            "report.pdf",
            "letter.docx",
            "unknown.blend",
        ] {
            assert_eq!(shared(name, false), None, "{name}");
        }
        assert_eq!(shared("styles.json", true), None);
    }
}
