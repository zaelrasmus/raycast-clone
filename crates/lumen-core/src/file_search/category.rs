//! What kind of file a name is, for the *All Types* filter (F16).
//!
//! By extension alone, from one table: the index holds names, not contents,
//! and a filter that opened files to sniff them would read the disk on every
//! keystroke. A name with no extension in the table is in no category, so it
//! shows under *All Types* only.

/// A choice of the *Type* filter other than *All Types*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Folders,
    Documents,
    Images,
    Video,
    Audio,
    Archives,
    Code,
}

/// Every category, in the filter's order.
pub const CATEGORIES: [Category; 7] = [
    Category::Folders,
    Category::Documents,
    Category::Images,
    Category::Video,
    Category::Audio,
    Category::Archives,
    Category::Code,
];

/// The extensions of each category of files, lower case, without the dot.
/// One table, so a category and its extensions cannot drift apart. Config
/// formats count as code: they are edited in the same editors, and read the
/// same way in the preview.
const EXTENSIONS: &[(Category, &[&str])] = &[
    (
        Category::Documents,
        &[
            "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "markdown", "rst", "org", "tex",
            "epub", "xls", "xlsx", "ods", "csv", "tsv", "ppt", "pptx", "odp", "pages", "numbers",
            "key", "log",
        ],
    ),
    (
        Category::Images,
        &[
            "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "svg", "heic",
            "heif", "avif", "raw", "cr2", "nef", "arw", "dng", "psd", "xcf",
        ],
    ),
    (
        Category::Video,
        &[
            "mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "flv", "mpg", "mpeg", "3gp",
        ],
    ),
    (
        Category::Audio,
        &[
            "mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "wma", "aiff", "mid", "midi",
        ],
    ),
    (
        Category::Archives,
        &[
            "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "cab", "iso", "dmg",
        ],
    ),
    (
        Category::Code,
        &[
            "rs", "py", "js", "mjs", "cjs", "jsx", "ts", "tsx", "c", "h", "cpp", "hpp", "cc", "cs",
            "java", "kt", "go", "rb", "php", "swift", "lua", "sh", "bash", "zsh", "fish", "ps1",
            "psm1", "bat", "cmd", "sql", "html", "htm", "css", "scss", "less", "vue", "svelte",
            "json", "jsonc", "toml", "yaml", "yml", "xml", "ini", "cfg", "conf", "env", "lock",
            "gradle", "cmake", "zig", "dart", "hs", "ex", "exs", "clj", "scala", "r", "jl", "nim",
            "wit", "proto", "graphql",
        ],
    ),
];

impl Category {
    /// Sent back by the filter; stable, unlike the label.
    pub fn id(self) -> &'static str {
        match self {
            Category::Folders => "folders",
            Category::Documents => "documents",
            Category::Images => "images",
            Category::Video => "video",
            Category::Audio => "audio",
            Category::Archives => "archives",
            Category::Code => "code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Folders => "Folders",
            Category::Documents => "Documents",
            Category::Images => "Images",
            Category::Video => "Video",
            Category::Audio => "Audio",
            Category::Archives => "Archives",
            Category::Code => "Code",
        }
    }

    /// The category an id names; `None` for *All Types* and for anything
    /// unknown.
    pub fn from_id(id: &str) -> Option<Category> {
        CATEGORIES.into_iter().find(|category| category.id() == id)
    }

    /// The category of an entry by its name: a folder is a folder whatever
    /// it is called.
    pub fn of(name: &str, is_dir: bool) -> Option<Category> {
        if is_dir {
            return Some(Category::Folders);
        }
        // Compared without folding a copy: the type filter asks this of
        // every name a scan matches.
        let extension = raw_extension(name)?;
        EXTENSIONS
            .iter()
            .find(|(_, extensions)| {
                extensions
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
            })
            .map(|(category, _)| *category)
    }

    /// Whether an entry belongs here; what the filter asks of every match.
    pub fn holds(self, name: &str, is_dir: bool) -> bool {
        Category::of(name, is_dir) == Some(self)
    }
}

/// A name's extension, lower case, without the dot. A leading dot is not an
/// extension: `.env` has none, as the index has it.
pub fn extension(name: &str) -> Option<String> {
    raw_extension(name).map(str::to_ascii_lowercase)
}

/// [`extension`] as written.
fn raw_extension(name: &str) -> Option<&str> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_in_the_category_of_its_extension_whatever_its_case() {
        assert_eq!(Category::of("Report.PDF", false), Some(Category::Documents));
        assert_eq!(Category::of("cat.jpeg", false), Some(Category::Images));
        assert_eq!(Category::of("talk.mkv", false), Some(Category::Video));
        assert_eq!(Category::of("song.flac", false), Some(Category::Audio));
        assert_eq!(Category::of("backup.7z", false), Some(Category::Archives));
        assert_eq!(Category::of("main.rs", false), Some(Category::Code));
        assert_eq!(Category::of("carbon.json", false), Some(Category::Code));
    }

    #[test]
    fn a_folder_is_a_folder_even_when_its_name_looks_like_a_file() {
        assert_eq!(Category::of("photos.png", true), Some(Category::Folders));
        assert!(!Category::Images.holds("photos.png", true));
    }

    #[test]
    fn a_name_without_a_known_extension_is_in_no_category() {
        assert_eq!(Category::of("Makefile", false), None);
        assert_eq!(Category::of(".env", false), None);
        assert_eq!(Category::of("thing.unknownext", false), None);
    }

    /// One table: an extension in two categories would put a file under both.
    #[test]
    fn no_extension_is_in_two_categories() {
        let mut seen = std::collections::HashSet::new();
        for (_, extensions) in EXTENSIONS {
            for extension in *extensions {
                assert!(seen.insert(*extension), "{extension} is listed twice");
                assert_eq!(*extension, extension.to_ascii_lowercase());
            }
        }
    }

    #[test]
    fn every_category_round_trips_through_its_id() {
        for category in CATEGORIES {
            assert_eq!(Category::from_id(category.id()), Some(category));
        }
        assert_eq!(Category::from_id(""), None);
    }
}
