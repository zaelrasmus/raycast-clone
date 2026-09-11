//! COM apartment management and string marshalling.

use anyhow::{Context as _, Result};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::core::{HSTRING, PWSTR};

/// An initialised COM apartment for the current thread, uninitialised on drop.
///
/// Every worker that touches the shell needs one: COM is per-thread, and the
/// index and the icon loader both run off the UI thread.
pub struct Apartment {
    /// Set when this value actually initialised the apartment. If the thread was
    /// already initialised, dropping must not tear down someone else's
    /// apartment.
    owns: bool,
}

impl Apartment {
    pub fn enter() -> Result<Self> {
        // Single-threaded apartment: the shell's item and image interfaces are
        // apartment-threaded, and a multithreaded apartment makes every call
        // marshal through a proxy.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_err() {
            // RPC_E_CHANGED_MODE means the thread is already in a different
            // apartment. That is survivable; failing to initialise at all is not.
            anyhow::ensure!(
                hr == windows::Win32::Foundation::RPC_E_CHANGED_MODE,
                "CoInitializeEx failed: {hr:?}"
            );
            return Ok(Self { owns: false });
        }
        Ok(Self { owns: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owns {
            unsafe { CoUninitialize() };
        }
    }
}

/// A UTF-16 string suitable for passing where a `PCWSTR` is expected.
pub fn wide(value: &str) -> HSTRING {
    HSTRING::from(value)
}

/// Read a shell-allocated wide string and free it.
///
/// The shell allocates these with the task allocator and hands ownership over;
/// not freeing them is a slow leak across every reindex.
pub fn take_pwstr(pointer: PWSTR) -> Result<String> {
    if pointer.is_null() {
        anyhow::bail!("the shell returned a null string");
    }
    let value = unsafe { pointer.to_string() }.context("the shell returned invalid UTF-16");
    unsafe { CoTaskMemFree(Some(pointer.0 as *const _)) };
    value
}
