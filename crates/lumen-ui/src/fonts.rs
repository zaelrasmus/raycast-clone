//! The bundled typeface.
//!
//! F2: "Typeface bundled into the binary (Inter or Geist), never the system
//! one."
//!
//! # Why this is not a detail
//!
//! AD-01 claims that GPUI gives near-total visual parity across platforms
//! because it draws every pixel itself, and section 1 narrows the exceptions to
//! two: background blur, and text rasterisation. The system font is the larger
//! half of that second exception -- Segoe UI on Windows and whatever fontconfig
//! picks on Arch are different typefaces, not different renderings of one. A
//! bundled face removes the difference entirely, leaving only rasterisation,
//! which is what the plan says the mitigation is for.
//!
//! Inter, SIL Open Font License 1.1. The licence travels with the files in
//! `assets/fonts/LICENSE-Inter.txt`, which is what the OFL requires.

use std::borrow::Cow;

use gpui::App;

/// The family name GPUI will resolve. Matches the name inside the font files.
pub const FAMILY: &str = "Inter";

/// Three weights and no italics.
///
/// The launcher uses one weight for body text, one for anything emphasised and
/// one for headings; italics never appear in the interface, and shipping faces
/// nothing draws would be a megabyte spent on nothing.
const FACES: [&[u8]; 3] = [
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
];

/// Register the bundled faces with GPUI's text system.
///
/// A failure here is not fatal. The interface falls back to whatever the
/// platform offers, which is exactly where it was before the font was bundled --
/// and a launcher that refuses to start because a font did not load would be a
/// far worse trade than one that renders in Segoe UI for a release.
pub fn register(cx: &App) {
    let faces: Vec<Cow<'static, [u8]>> = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();

    match cx.text_system().add_fonts(faces) {
        Ok(()) => log::debug!("registered the bundled {FAMILY} faces"),
        Err(error) => log::warn!("could not load the bundled typeface: {error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A truncated or missing font file would otherwise fail at runtime, on a
    /// user's machine, as text that silently falls back.
    #[test]
    fn every_bundled_face_is_a_real_truetype_file() {
        for face in FACES {
            assert!(face.len() > 100_000, "a face is suspiciously small");
            // TrueType outlines start with the version tag 0x00010000; OpenType
            // with CFF outlines would start with `OTTO`, which GPUI also reads,
            // but Inter ships the former and a change would be worth noticing.
            assert_eq!(&face[..4], &[0x00, 0x01, 0x00, 0x00], "not a TrueType file");
        }
    }

    /// The OFL requires the licence to travel with the fonts.
    #[test]
    fn the_licence_is_bundled_alongside_them() {
        let licence = include_str!("../../../assets/fonts/LICENSE-Inter.txt");
        assert!(licence.contains("SIL OPEN FONT LICENSE"));
    }
}
