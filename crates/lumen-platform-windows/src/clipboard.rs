//! Clipboard access.
//!
//! Present in F0/F1 only as the small surface `trait Platform` declares. The
//! clipboard *history* of F3 is a different thing entirely -- a system event
//! listener, a schema, a retention policy and the privacy layer adopted from
//! Asyar -- and the plan is right that it is a subproject rather than a feature.
//! Nothing here anticipates it beyond not getting in its way.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use lumen_platform::ClipboardContent;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// Holds the clipboard open, and closes it however the body exits.
///
/// The clipboard is a single global lock for the entire session: leaving it open
/// on an early return wedges copy and paste for every application until this
/// process dies.
pub(crate) struct ClipboardLock;

impl ClipboardLock {
    fn acquire() -> Result<Self> {
        unsafe { OpenClipboard(None) }.context("another application is holding the clipboard")?;
        Ok(Self)
    }

    /// Open it for reading, tried a few times, briefly, for the reason
    /// [`content`] gives. Only for a thread of Lumen's own that may wait.
    fn acquire_patiently() -> Result<Self> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Self::acquire()
    }

    /// Open it for `owner`, which becomes the clipboard's owner on the next
    /// `EmptyClipboard` (F11). Tried a few times, briefly: a clipboard
    /// manager reading the last copy holds it for a moment, and a paste the
    /// user asked for should not fail on that.
    pub(crate) fn acquire_for(owner: HWND) -> Result<Self> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Ok(Self);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        unsafe { OpenClipboard(Some(owner)) }
            .context("another application is holding the clipboard")?;
        Ok(Self)
    }
}

/// `value` as the clipboard stores text: UTF-16 with a terminating NUL.
pub(crate) fn utf16(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A moveable block holding `bytes`, for `SetClipboardData`.
///
/// GMEM_MOVEABLE is required: the clipboard takes ownership of the block and
/// frees it itself, and it will only accept a moveable one. Once handed over,
/// freeing it would be a double free the next time the user pastes.
pub(crate) fn global(bytes: &[u8]) -> Result<HGLOBAL> {
    unsafe {
        // A zero-sized block cannot be locked; a marker format is one byte.
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).context("GlobalAlloc")?;
        let destination = GlobalLock(global) as *mut u8;
        if destination.is_null() {
            anyhow::bail!("could not lock the clipboard's memory");
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
        let _ = GlobalUnlock(global);
        Ok(global)
    }
}

impl Drop for ClipboardLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn text() -> Result<Option<String>> {
    let _lock = ClipboardLock::acquire()?;
    unsafe { open_text() }
}

/// The text, with the clipboard already open.
unsafe fn open_text() -> Result<Option<String>> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return Ok(None);
        }

        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).context("GetClipboardData")?;
        if handle.0.is_null() {
            return Ok(None);
        }

        let global = HGLOBAL(handle.0);
        let pointer = GlobalLock(global) as *const u16;
        if pointer.is_null() {
            anyhow::bail!("could not lock the clipboard's memory");
        }

        let mut length = 0usize;
        while *pointer.add(length) != 0 {
            length += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));

        let _ = GlobalUnlock(global);
        Ok(Some(value))
    }
}

pub fn set_text(value: &str) -> Result<()> {
    let bytes = utf16(value);
    let _lock = ClipboardLock::acquire()?;

    unsafe {
        EmptyClipboard().context("EmptyClipboard")?;
        let global = global(&bytes)?;
        SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(global.0)))
            .context("SetClipboardData")?;
    }

    Ok(())
}

/// What is on the clipboard, as clipboard history keeps it: files, or text
/// (F12). `None` when it is neither.
///
/// Both are read in one opening, and the opening is retried: every monitor
/// reads the clipboard the moment it changes -- Windows' own history among
/// them, which makes the copying program render delayed formats while it
/// holds the clipboard. Two openings, or one attempt, lost that race and the
/// copy with it.
pub fn content() -> Result<Option<ClipboardContent>> {
    let _lock = ClipboardLock::acquire_patiently()?;
    unsafe {
        // Files first: a program that copies files may name them in text as
        // well, and the files are what was copied.
        if let Some(files) = open_files()? {
            return Ok(Some(ClipboardContent::Files(files)));
        }
        Ok(open_text()?.map(ClipboardContent::Text))
    }
}

/// The files, when a file manager put them there, with the clipboard
/// already open.
unsafe fn open_files() -> Result<Option<Vec<PathBuf>>> {
    unsafe {
        if IsClipboardFormatAvailable(u32::from(CF_HDROP.0)).is_err() {
            return Ok(None);
        }
        let handle = GetClipboardData(u32::from(CF_HDROP.0)).context("GetClipboardData")?;
        if handle.0.is_null() {
            return Ok(None);
        }
        let drop = HDROP(handle.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        let mut files = Vec::with_capacity(count as usize);
        for index in 0..count {
            // The length without the NUL, then a buffer with room for it.
            let length = DragQueryFileW(drop, index, None) as usize;
            let mut buffer = vec![0u16; length + 1];
            let written = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
            files.push(PathBuf::from(String::from_utf16_lossy(&buffer[..written])));
        }
        Ok((!files.is_empty()).then_some(files))
    }
}

/// `files` as `CF_HDROP` stores them: a `DROPFILES` header, then each path
/// in UTF-16 with its NUL, and a second NUL to end the list.
pub(crate) fn hdrop(files: &[PathBuf]) -> Vec<u8> {
    // pFiles (the header's size), pt.x, pt.y, fNC, fWide.
    const HEADER: u32 = 20;
    let mut bytes = Vec::new();
    for field in [HEADER, 0, 0, 0, 1] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    for file in files {
        for unit in file.to_string_lossy().encode_utf16().chain([0]) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0, 0]);
    bytes
}

/// The formats `content` is written as. Files are named in text as well, one
/// path per line, for a target that takes no files: an editor pastes the
/// paths, which is what somebody pasting a file into one wants.
pub(crate) fn formats_of(content: &ClipboardContent) -> Vec<(u32, Vec<u8>)> {
    match content {
        ClipboardContent::Text(text) => vec![(u32::from(CF_UNICODETEXT.0), utf16(text))],
        ClipboardContent::Files(files) => {
            let names = files
                .iter()
                .map(|file| file.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\r\n");
            vec![
                (u32::from(CF_HDROP.0), hdrop(files)),
                (u32::from(CF_UNICODETEXT.0), utf16(&names)),
            ]
        }
    }
}

/// The marker a clipboard monitor skips, Lumen's included (7.13).
pub(crate) fn exclude_format() -> u32 {
    unsafe {
        RegisterClipboardFormatW(windows::core::w!(
            "ExcludeClipboardContentFromMonitorProcessing"
        ))
    }
}

/// Replace the clipboard with text or files, marked not-for-history when
/// asked (F12).
pub fn set_content(content: &ClipboardContent, keep_out_of_history: bool) -> Result<()> {
    let mut formats = formats_of(content);
    if keep_out_of_history {
        let exclude = exclude_format();
        if exclude != 0 {
            formats.push((exclude, vec![0]));
        }
    }
    set_formats(&formats)
}

/// Replace the clipboard with several formats of one thing at once (F11).
pub fn set_formats(formats: &[(u32, Vec<u8>)]) -> Result<()> {
    let _lock = ClipboardLock::acquire()?;
    unsafe {
        EmptyClipboard().context("EmptyClipboard")?;
        for (format, bytes) in formats {
            let global = global(bytes)?;
            SetClipboardData(*format, Some(HANDLE(global.0))).context("SetClipboardData")?;
        }
    }
    Ok(())
}

/// The registered id of `HTML Format`.
pub fn html_format() -> u32 {
    unsafe { RegisterClipboardFormatW(windows::core::w!("HTML Format")) }
}

/// An HTML fragment as `HTML Format` stores it: a header of byte offsets,
/// then a document with the fragment between two comments, in UTF-8 and
/// NUL-terminated.
pub fn cf_html(fragment: &str) -> Vec<u8> {
    const PREFIX: &str = "<html><body>\r\n<!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment-->\r\n</body></html>";
    let header = |start_html: usize, end_html: usize, start: usize, end: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
             StartFragment:{start:010}\r\nEndFragment:{end:010}\r\n"
        )
    };
    // Every number is ten digits, so the header's length does not depend on
    // them.
    let start_html = header(0, 0, 0, 0).len();
    let start = start_html + PREFIX.len();
    let end = start + fragment.len();
    let end_html = end + SUFFIX.len();
    let mut out = header(start_html, end_html, start, end).into_bytes();
    out.extend_from_slice(PREFIX.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(SUFFIX.as_bytes());
    out.push(0);
    out
}

/// Read a clipboard format whose contents are a single `DWORD`.
///
/// Used for `CanIncludeInClipboardHistory`, which is a flag rather than a
/// marker: its value is what matters, not its presence.
pub fn dword(format: u32) -> Result<Option<u32>> {
    let _lock = ClipboardLock::acquire()?;

    unsafe {
        let handle = GetClipboardData(format).context("GetClipboardData")?;
        if handle.0.is_null() {
            return Ok(None);
        }

        let global = HGLOBAL(handle.0);
        let pointer = GlobalLock(global) as *const u32;
        if pointer.is_null() {
            return Ok(None);
        }

        let value = *pointer;
        let _ = GlobalUnlock(global);
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offsets are where the parts are, in bytes of the UTF-8.
    #[test]
    fn a_file_list_is_a_dropfiles_block() {
        let bytes = hdrop(&[PathBuf::from(r"C:\a"), PathBuf::from("b")]);
        // The header says where the list starts and that it is wide.
        assert_eq!(&bytes[0..4], &20u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &1u32.to_le_bytes());
        let units: Vec<u16> = bytes[20..]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let expected: Vec<u16> = "C:\\a\0b\0\0".encode_utf16().collect();
        assert_eq!(units, expected);
    }

    #[test]
    fn html_format_offsets_point_at_the_fragment() {
        let fragment = "<strong>h\u{e9}llo</strong>";
        let bytes = cf_html(fragment);
        assert_eq!(bytes.last(), Some(&0));
        let text = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        let number = |key: &str| -> usize {
            let at = text.find(key).unwrap() + key.len();
            text[at..at + 10].parse().unwrap()
        };
        let (start, end) = (number("StartFragment:"), number("EndFragment:"));
        assert_eq!(&text[start..end], fragment);
        assert!(text[number("StartHTML:")..].starts_with("<html>"));
        assert_eq!(number("EndHTML:"), text.len());
    }
}
