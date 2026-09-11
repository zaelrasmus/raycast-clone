//! Clipboard access.
//!
//! Present in F0/F1 only as the small surface `trait Platform` declares. The
//! clipboard *history* of F3 is a different thing entirely -- a system event
//! listener, a schema, a retention policy and the privacy layer adopted from
//! Asyar -- and the plan is right that it is a subproject rather than a feature.
//! Nothing here anticipates it beyond not getting in its way.

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Holds the clipboard open, and closes it however the body exits.
///
/// The clipboard is a single global lock for the entire session: leaving it open
/// on an early return wedges copy and paste for every application until this
/// process dies.
struct ClipboardLock;

impl ClipboardLock {
    fn acquire() -> Result<Self> {
        unsafe { OpenClipboard(None) }.context("another application is holding the clipboard")?;
        Ok(Self)
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
    let mut utf16: Vec<u16> = value.encode_utf16().collect();
    utf16.push(0);
    let bytes = std::mem::size_of_val(utf16.as_slice());

    let _lock = ClipboardLock::acquire()?;

    unsafe {
        EmptyClipboard().context("EmptyClipboard")?;

        // GMEM_MOVEABLE is required: the clipboard takes ownership of this block
        // and frees it itself, and it will only accept a moveable one.
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes).context("GlobalAlloc")?;

        let destination = GlobalLock(global) as *mut u16;
        if destination.is_null() {
            anyhow::bail!("could not lock the clipboard's memory");
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), destination, utf16.len());
        let _ = GlobalUnlock(global);

        // From here the clipboard owns the block; freeing it would be a
        // double free the next time the user pastes.
        SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(global.0)))
            .context("SetClipboardData")?;
    }

    Ok(())
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
