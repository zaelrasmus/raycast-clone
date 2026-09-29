//! Alert sounds (F8): the alarms and rings Windows ships.
//!
//! `%SystemRoot%\Media` holds `Alarm01.wav` to `Alarm10.wav` and `Ring01.wav`
//! to `Ring10.wav` on every Windows 10 and 11 install. `PlaySoundW` plays one
//! asynchronously, looping if asked, and a second call replaces the first --
//! which is exactly one alert at a time, with nothing to decode or bundle.

use std::path::PathBuf;

use anyhow::{Result, bail};
use windows::Win32::Media::Audio::{
    PlaySoundW, SND_ASYNC, SND_FILENAME, SND_FLAGS, SND_LOOP, SND_NODEFAULT,
};
use windows::core::{HSTRING, PCWSTR};

/// Where the system's sounds are.
fn media() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("Media")
}

/// Whether a file in the media folder is an alert rather than a chime for a
/// window opening: the alarms and the rings, which are long enough to notice.
fn is_alert(stem: &str) -> bool {
    stem.starts_with("Alarm") || stem.starts_with("Ring")
}

pub fn names() -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(media())?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let is_wav = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
            let stem = path.file_stem()?.to_str()?.to_string();
            (is_wav && is_alert(&stem)).then_some(stem)
        })
        .collect();
    names.sort();
    Ok(names)
}

pub fn play(name: &str, repeat: bool) -> Result<()> {
    // A name, never a path: the configuration is edited by hand, and this
    // must not play whatever file a typo or a paste points at.
    if !is_alert(name) || name.contains(['/', '\\', '.']) {
        bail!("{name:?} is not one of the system's alert sounds");
    }
    let path = media().join(format!("{name}.wav"));
    if !path.exists() {
        bail!("there is no sound called {name:?} in {}", media().display());
    }

    let mut flags = SND_FILENAME | SND_ASYNC | SND_NODEFAULT;
    if repeat {
        flags |= SND_LOOP;
    }
    let path = HSTRING::from(path.as_os_str());
    // SAFETY: the path is a null-terminated wide string that outlives the
    // call; with SND_ASYNC the system copies what it needs before returning.
    let played = unsafe { PlaySoundW(&path, None, flags) };
    if !played.as_bool() {
        bail!("Windows would not play {name:?}");
    }
    Ok(())
}

pub fn stop() -> Result<()> {
    // SAFETY: a null sound is documented as "stop what is playing".
    unsafe {
        let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every install has them; if this fails, the machine is unusual and the
    /// settings page will say there are no sounds rather than crash.
    #[test]
    fn the_system_offers_alarms_and_rings() {
        let names = names().unwrap();
        assert!(
            names.iter().any(|name| name.starts_with("Alarm")),
            "{names:?}"
        );
        assert!(names.iter().all(|name| is_alert(name)));
    }

    #[test]
    fn a_path_is_not_a_sound_name() {
        assert!(play(r"..\..\evil", false).is_err());
        assert!(play("Alarm01.wav", false).is_err());
        assert!(play("Windows Notify", false).is_err());
    }
}
