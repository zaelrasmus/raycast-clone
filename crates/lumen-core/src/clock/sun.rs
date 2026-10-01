//! Sunrise and sunset from a place's coordinates (F13).
//!
//! The sunrise equation, as the NOAA and Wikipedia write it: good to a
//! minute or two away from the poles, which is what a clock that shows
//! minutes needs, and computed, so the pane says when it gets dark in
//! Vancouver without asking anybody.

use jiff::Timestamp;
use jiff::civil::Date;

/// When the sun rises and sets on a day, or that it does neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Daylight {
    Day {
        sunrise: Timestamp,
        sunset: Timestamp,
    },
    /// The sun stays up all day: a summer near the pole.
    AlwaysUp,
    /// The sun stays down all day: a winter near the pole.
    AlwaysDown,
}

/// Seconds from the Unix epoch to the Julian date J2000.0, and the Julian
/// date of the epoch.
const UNIX_EPOCH_JULIAN: f64 = 2_440_587.5;
const J2000: f64 = 2_451_545.0;

/// When the sun rises and sets at `latitude` and `longitude` (degrees, east
/// positive) on `date`, the date as it is at that place.
pub fn daylight(latitude: f64, longitude: f64, date: Date) -> Daylight {
    // The Julian day number at noon of `date`.
    let since_epoch = date
        .since(Date::constant(1970, 1, 1))
        .map(|span| f64::from(span.get_days()))
        .unwrap_or(0.0);
    let day = (since_epoch + UNIX_EPOCH_JULIAN + 0.5 - J2000 + 0.0008).round();

    let mean_solar_noon = day - longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_solar_noon).rem_euclid(360.0);
    let m = anomaly.to_radians();
    let centre = 1.9148 * m.sin() + 0.0200 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin();
    let ecliptic = (anomaly + centre + 180.0 + 102.9372).rem_euclid(360.0);
    let l = ecliptic.to_radians();
    let transit = J2000 + mean_solar_noon + 0.0053 * m.sin() - 0.0069 * (2.0 * l).sin();

    let declination = (l.sin() * 23.4397_f64.to_radians().sin()).asin();
    let phi = latitude.to_radians();
    // -0.833 degrees: refraction, and the sun's disc rather than its centre.
    let hour_angle = ((-0.833_f64).to_radians().sin() - phi.sin() * declination.sin())
        / (phi.cos() * declination.cos());
    if hour_angle > 1.0 {
        return Daylight::AlwaysDown;
    }
    if hour_angle < -1.0 {
        return Daylight::AlwaysUp;
    }
    let half_day = hour_angle.acos().to_degrees() / 360.0;

    let at = |julian: f64| {
        let seconds = ((julian - UNIX_EPOCH_JULIAN) * 86_400.0).round() as i64;
        Timestamp::from_second(seconds).unwrap_or(Timestamp::UNIX_EPOCH)
    };
    Daylight::Day {
        sunrise: at(transit - half_day),
        sunset: at(transit + half_day),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: Timestamp, expected: &str) {
        let expected: Timestamp = expected.parse().unwrap();
        let off = (actual.as_second() - expected.as_second()).abs();
        assert!(off <= 180, "{actual} is {off} s from {expected}");
    }

    /// Against timeanddate.com's published times, to within three minutes.
    #[test]
    fn vancouver_at_midsummer() {
        let Daylight::Day { sunrise, sunset } =
            daylight(49.25, -123.119, Date::constant(2026, 6, 21))
        else {
            panic!("the sun rises in Vancouver")
        };
        // 05:07 and 21:21, Pacific Daylight Time.
        near(sunrise, "2026-06-21T12:07:00Z");
        near(sunset, "2026-06-22T04:21:00Z");
    }

    #[test]
    fn santo_domingo_in_autumn() {
        let Daylight::Day { sunrise, sunset } =
            daylight(18.472, -69.892, Date::constant(2026, 9, 30))
        else {
            panic!("the sun rises in Santo Domingo")
        };
        // 06:30 and 18:29, Atlantic Standard Time.
        near(sunrise, "2026-09-30T10:30:00Z");
        near(sunset, "2026-09-30T22:29:00Z");
    }

    #[test]
    fn the_poles_have_days_without_a_sunrise_or_a_sunset() {
        // Troms\u{f8}, above the Arctic Circle.
        assert_eq!(
            daylight(69.649, 18.957, Date::constant(2026, 6, 21)),
            Daylight::AlwaysUp
        );
        assert_eq!(
            daylight(69.649, 18.957, Date::constant(2026, 12, 21)),
            Daylight::AlwaysDown
        );
    }
}
