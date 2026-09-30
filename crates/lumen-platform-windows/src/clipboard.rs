//! Clipboard access.
//!
//! Present in F0/F1 only as the small surface `trait Platform` declares. The
//! clipboard *history* of F3 is a different thing entirely -- a system event
//! listener, a schema, a retention policy and the privacy layer adopted from
//! Asyar -- and the plan is right that it is a subproject rather than a feature.
//! Nothing here anticipates it beyond not getting in its way.

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
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
pub(crate) struct ClipboardLock;

impl ClipboardLock {
    fn acquire() -> Result<Self> {
        unsafe { OpenClipboard(None) }.context("another application is holding the clipboard")?;
        Ok(Self)
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
