//! Installed browsers, and opening a URL in one of them (F6).
//!
//! # Where Windows keeps the list
//!
//! A browser that wants to be offered in Settings > Default apps registers a
//! key under `SOFTWARE\Clients\StartMenuInternet`, in `HKLM` for a machine-wide
//! install and in `HKCU` for a per-user one. The key's default value is the
//! display name and `shell\open\command` is how to start it.
//!
//! Both halves matter. Helium installs per user and is only in `HKCU`, Brave
//! only in `HKLM`; reading one hive misses one of them. And the key itself is
//! not a name: Zen registers as `Firefox-<hash>`, which is why [`Browser`]
//! carries the display name and nothing else.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use lumen_platform::Browser;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ,
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::PWSTR;

use crate::com;

const CLIENTS_KEY: &str = r"SOFTWARE\Clients\StartMenuInternet";

/// Every registered browser whose program still exists.
///
/// `HKCU` first, so a per-user registration wins over a machine-wide one with
/// the same key -- the same precedence Windows itself gives them.
pub fn installed() -> Result<Vec<Browser>> {
    let mut seen = Vec::<String>::new();
    let mut browsers = Vec::new();

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Some(clients) = OpenKey::open(hive, CLIENTS_KEY) else {
            continue;
        };

        for key in clients.subkeys() {
            if seen.iter().any(|known| known.eq_ignore_ascii_case(&key)) {
                continue;
            }
            seen.push(key.clone());

            let Some(name) = clients.string(&key) else {
                continue;
            };
            let Some(command) = clients.string(&format!(r"{key}\shell\open\command")) else {
                continue;
            };
            let Some(executable) = executable(&command) else {
                continue;
            };
            // Uninstallers are not reliable about removing the registration,
            // and offering a browser that fails to start is worse than not
            // offering it.
            if !executable.exists() {
                log::debug!(
                    "{name} is registered but {} is missing",
                    executable.display()
                );
                continue;
            }

            browsers.push(Browser { name, executable });
        }
    }

    Ok(browsers)
}

/// Where Windows keeps the user's choice of browser for https links, newest
/// first. Windows 11 24H2 moved it into `UserChoiceLatest\ProgId`; earlier
/// builds, and machines not yet migrated, have only `UserChoice`.
const USER_CHOICE: [&str; 2] = [
    r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoiceLatest\ProgId",
    r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
];

/// The browser https links open in, when nobody names one.
///
/// Read for a private window, which needs a program to hand its flag to:
/// the shell, which opens everything else, takes no flags. The choice names
/// a ProgId, and the ProgId's open command names the program. Matched back
/// to the registered browsers for its display name, which is what decides
/// Opera's flag.
pub fn default() -> Result<Browser> {
    let prog_id = USER_CHOICE
        .iter()
        .find_map(|path| OpenKey::open(HKEY_CURRENT_USER, path)?.value("", "ProgId"))
        .context("no default browser is set for https links")?;
    let command = OpenKey::open(HKEY_CLASSES_ROOT, &prog_id)
        .and_then(|key| key.string(r"shell\open\command"))
        .with_context(|| format!("{prog_id} has no open command"))?;
    let executable = executable(&command)
        .with_context(|| format!("{prog_id}'s open command names no program"))?;

    let name = installed()
        .unwrap_or_default()
        .into_iter()
        .find(|browser| same_file(&browser.executable, &executable))
        .map_or(prog_id, |browser| browser.name);
    Ok(Browser { name, executable })
}

fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Hand `url` to `browser`, in a private window when `private`.
///
/// Started directly rather than through the shell: the shell would pick the
/// default browser, which is exactly what naming one is meant to override.
/// Every browser Lumen has been tried with opens a tab in its running window
/// rather than a second instance.
///
/// A private window from a browser whose flag is unknown is refused. Opening
/// it in an ordinary window instead would record exactly what the user asked
/// not to be recorded.
pub fn open(url: &str, browser: &Browser, private: bool) -> Result<()> {
    let flag = match (private, browser.private_flag()) {
        (false, _) => None,
        (true, Some(flag)) => Some(flag),
        (true, None) => bail!(
            "Lumen does not know how to open a private window in {}",
            browser.name
        ),
    };

    // Null handles, or the browser writes its own diagnostics into Lumen's
    // log for as long as it runs -- Brave does, on every start.
    Command::new(&browser.executable)
        .args(flag)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {}", browser.executable.display()))?;
    Ok(())
}

/// The program in a `shell\open\command` value.
///
/// Usually just a quoted path, sometimes followed by arguments, occasionally
/// unquoted. An unquoted path with spaces is ambiguous in general; ending it at
/// `.exe` is what Windows' own loader effectively does for these keys.
fn executable(command: &str) -> Option<PathBuf> {
    let command = command.trim();

    let path = if let Some(rest) = command.strip_prefix('"') {
        &rest[..rest.find('"')?]
    } else {
        let lower = command.to_ascii_lowercase();
        match lower.find(".exe") {
            Some(end) => &command[..end + ".exe".len()],
            None => command.split_whitespace().next()?,
        }
    };

    (!path.is_empty()).then(|| Path::new(path).to_path_buf())
}

/// An open registry key, closed on drop.
struct OpenKey(HKEY);

impl OpenKey {
    fn open(parent: HKEY, path: &str) -> Option<Self> {
        let mut key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(parent, &com::wide(path), None, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    fn subkeys(&self) -> Vec<String> {
        let mut names = Vec::new();
        // The longest key name the registry allows, plus the terminator.
        let mut buffer = [0u16; 256];

        for index in 0.. {
            let mut length = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    Some(PWSTR(buffer.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };
            // ERROR_NO_MORE_ITEMS is the ordinary end; anything else ends it too.
            if status != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        names
    }

    /// The default value of `subkey`, as a string.
    fn string(&self, subkey: &str) -> Option<String> {
        self.value(subkey, "")
    }

    /// The value called `name` in `subkey` -- the default value when `name`
    /// is empty, and this key itself when `subkey` is -- as a string.
    fn value(&self, subkey: &str, name: &str) -> Option<String> {
        let subkey = com::wide(subkey);
        let name = com::wide(name);
        // REG_SZ alone still accepts a REG_EXPAND_SZ value and expands it.
        // Naming the expandable type too is refused unless expansion is off.
        let flags = RRF_RT_REG_SZ;

        // Asked once for the size and once for the value. The value could in
        // principle change in between; if it grows, the second call fails and
        // the browser is skipped, which is the right answer to a registration
        // being rewritten under us.
        let mut bytes = 0u32;
        let status =
            unsafe { RegGetValueW(self.0, &subkey, &name, flags, None, None, Some(&mut bytes)) };
        if status != ERROR_SUCCESS || bytes < 2 {
            return None;
        }

        let mut buffer = vec![0u16; bytes as usize / 2];
        let status = unsafe {
            RegGetValueW(
                self.0,
                &subkey,
                &name,
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }

        let text = String::from_utf16_lossy(&buffer[..bytes as usize / 2]);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

impl Drop for OpenKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_path_is_read_up_to_its_closing_quote() {
        assert_eq!(
            executable(r#""C:\Program Files\Zen Browser\zen.exe""#),
            Some(PathBuf::from(r"C:\Program Files\Zen Browser\zen.exe"))
        );
        assert_eq!(
            executable(r#""C:\Brave\brave.exe" --profile-directory=Default"#),
            Some(PathBuf::from(r"C:\Brave\brave.exe"))
        );
    }

    #[test]
    fn an_unquoted_path_with_spaces_ends_at_the_executable() {
        assert_eq!(
            executable(r"C:\Program Files\Browser\browser.EXE -url"),
            Some(PathBuf::from(r"C:\Program Files\Browser\browser.EXE"))
        );
    }

    #[test]
    fn nothing_useful_is_nothing() {
        assert_eq!(executable(""), None);
        assert_eq!(executable(r#""""#), None);
        assert_eq!(executable(r#""unterminated"#), None);
    }

    /// Not an assertion about which browsers exist, which depends on the
    /// machine: only that reading both hives works and every result is real.
    #[test]
    fn every_listed_browser_exists() {
        for browser in installed().unwrap() {
            assert!(!browser.name.is_empty());
            assert!(browser.executable.exists(), "{browser:?}");
        }
    }
}
