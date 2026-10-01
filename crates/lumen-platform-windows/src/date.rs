//! Dates written the way Windows writes them (F11), and numbers (F14).
//!
//! `GetDateFormatEx` and `GetTimeFormatEx` with the user's own locale, or a
//! named one: the short date and the time without seconds, which is what the
//! taskbar clock shows, so `{date}` in a snippet reads like every other date
//! on the machine. `GetLocaleInfoEx` for a number's decimal mark and grouping
//! separator, as *Region* settings have them.

use anyhow::{Result, bail};
use lumen_platform::{DateStyle, LocalTime};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    DATE_SHORTDATE, ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx,
    IsValidLocaleName, LOCALE_SDECIMAL, LOCALE_STHOUSAND, TIME_FORMAT_FLAGS, TIME_NOSECONDS,
};
use windows::core::{HSTRING, PCWSTR, w};

/// The user's decimal mark and grouping separator. A separator Windows
/// cannot say, or says as more than one character, is read as none -- the
/// decimal mark as a point -- rather than guessed.
pub fn number_separators() -> Result<lumen_platform::NumberSeparators> {
    let decimal = locale_char(LOCALE_SDECIMAL)?.unwrap_or('.');
    let grouping = locale_char(LOCALE_STHOUSAND)?.filter(|grouping| *grouping != decimal);
    Ok(lumen_platform::NumberSeparators { decimal, grouping })
}

/// One character of the user's locale, as `GetLocaleInfoEx` has it.
fn locale_char(kind: u32) -> Result<Option<char>> {
    let mut buffer = [0u16; 8];
    // Null is the user's own locale (LOCALE_NAME_USER_DEFAULT).
    let written = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut buffer)) };
    if written <= 0 {
        bail!("Windows could not say how this locale writes numbers");
    }
    // `written` counts the terminating null.
    let text = String::from_utf16_lossy(&buffer[..written as usize - 1]);
    let mut characters = text.chars();
    Ok(match (characters.next(), characters.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    })
}

pub fn format(when: &LocalTime, style: DateStyle, locale: Option<&str>) -> Result<String> {
    // Null is the user's own locale (LOCALE_NAME_USER_DEFAULT).
    let name = locale.map(HSTRING::from);
    let locale_name = match &name {
        Some(name) => {
            if !unsafe { IsValidLocaleName(name) }.as_bool() {
                bail!(
                    "{:?} is not a locale this system knows",
                    locale.unwrap_or_default()
                );
            }
            PCWSTR(name.as_ptr())
        }
        None => PCWSTR::null(),
    };
    let time = SYSTEMTIME {
        wYear: when.year as u16,
        wMonth: u16::from(when.month),
        wDayOfWeek: u16::from(when.weekday),
        wDay: u16::from(when.day),
        wHour: u16::from(when.hour),
        wMinute: u16::from(when.minute),
        wSecond: u16::from(when.second),
        wMilliseconds: 0,
    };

    Ok(match style {
        DateStyle::Date => date(locale_name, &time, DATE_SHORTDATE, PCWSTR::null())?,
        DateStyle::Time => clock(locale_name, &time)?,
        DateStyle::DateTime => format!(
            "{} {}",
            date(locale_name, &time, DATE_SHORTDATE, PCWSTR::null())?,
            clock(locale_name, &time)?
        ),
        // A picture rather than a flag: the day's name, in full, in the
        // locale's language.
        DateStyle::Weekday => date(locale_name, &time, ENUM_DATE_FORMATS_FLAGS(0), w!("dddd"))?,
    })
}

fn date(
    locale: PCWSTR,
    time: &SYSTEMTIME,
    flags: ENUM_DATE_FORMATS_FLAGS,
    picture: PCWSTR,
) -> Result<String> {
    let mut buffer = [0u16; 128];
    let written = unsafe {
        GetDateFormatEx(
            locale,
            flags,
            Some(time),
            picture,
            Some(&mut buffer),
            PCWSTR::null(),
        )
    };
    written_text(&buffer, written)
}

fn clock(locale: PCWSTR, time: &SYSTEMTIME) -> Result<String> {
    let mut buffer = [0u16; 64];
    let flags: TIME_FORMAT_FLAGS = TIME_NOSECONDS;
    let written =
        unsafe { GetTimeFormatEx(locale, flags, Some(time), PCWSTR::null(), Some(&mut buffer)) };
    written_text(&buffer, written)
}

/// The text a formatting call wrote, which counts its terminating NUL.
fn written_text(buffer: &[u16], written: i32) -> Result<String> {
    if written <= 0 {
        bail!(
            "Windows could not write the date: {}",
            windows::core::Error::from_win32()
        );
    }
    let length = (written as usize).saturating_sub(1);
    Ok(String::from_utf16_lossy(&buffer[..length]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the machine's region, it says one decimal mark, and a
    /// grouping separator that is not the same character (F14).
    #[test]
    fn the_regions_separators_are_read() {
        let separators = number_separators().unwrap();
        assert!(matches!(separators.decimal, '.' | ','), "{separators:?}");
        assert_ne!(separators.grouping, Some(separators.decimal));
        println!("{separators:?}");
    }

    fn when() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 9,
            day: 30,
            weekday: 3,
            hour: 21,
            minute: 5,
            second: 0,
        }
    }

    /// A named locale is the same on every machine, so its output can be
    /// asserted; the user's own cannot.
    #[test]
    fn a_named_locale_writes_its_own_way() {
        assert_eq!(
            format(&when(), DateStyle::Weekday, Some("fr-FR")).unwrap(),
            "mercredi"
        );
        assert_eq!(
            format(&when(), DateStyle::Date, Some("en-US")).unwrap(),
            "9/30/2026"
        );
        assert_eq!(
            format(&when(), DateStyle::Time, Some("en-US")).unwrap(),
            "9:05 PM"
        );
        assert_eq!(
            format(&when(), DateStyle::Date, Some("fr-FR")).unwrap(),
            "30/09/2026"
        );
    }

    #[test]
    fn an_unknown_locale_is_an_error() {
        assert!(format(&when(), DateStyle::Date, Some("xx-NOTREAL")).is_err());
    }

    #[test]
    fn the_users_own_locale_writes_something() {
        assert!(
            !format(&when(), DateStyle::DateTime, None)
                .unwrap()
                .is_empty()
        );
    }
}
