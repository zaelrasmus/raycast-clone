//! Icon extraction.
//!
//! The F1 risk register is explicit that this is where launchers look cheap and
//! where the effort is always underestimated. Two rules follow from that:
//!
//! 1. `IShellItemImageFactory`, never `ExtractIconEx` or `SHGetFileInfo`. The
//!    legacy functions return whatever 32x32 bitmap happens to be in the
//!    resource, which is visibly soft the moment the display scales above 100%.
//! 2. Ask for more pixels than the row needs and let the GPU downscale. A 32 px
//!    row on a 150% display is 48 physical pixels, and asking for exactly 32
//!    guarantees a blurry result on the machine the author actually uses.

use anyhow::{Context as _, Result};
use lumen_platform::{IconImage, IconSource};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, ReleaseDC,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT,
};

use crate::com;

pub fn load(source: &IconSource, size_px: u32) -> Result<IconImage> {
    let parsing_name = match source {
        IconSource::ShellItem(name) => name.clone(),
        IconSource::File(path) => path.to_string_lossy().into_owned(),
        // Named icons are an XDG icon-theme concept. Windows has no equivalent
        // lookup, and silently returning a blank image would hide the mistake.
        IconSource::Named(name) => {
            anyhow::bail!("named icon {name:?} has no meaning on Windows")
        }
    };

    let _apartment = com::Apartment::enter()?;

    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&com::wide(&parsing_name), None)
                .with_context(|| format!("resolving {parsing_name:?} as a shell item"))?;

        let requested = SIZE {
            cx: size_px as i32,
            cy: size_px as i32,
        };

        // RESIZETOFIT keeps the aspect ratio; BIGGERSIZEOK lets the shell hand
        // back a larger native size rather than upscaling a smaller one, which
        // is the difference between a crisp icon and a soft one.
        let bitmap = factory
            .GetImage(requested, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK)
            .with_context(|| format!("extracting an icon for {parsing_name:?}"))?;

        let image = read_bitmap(bitmap);
        // The bitmap is ours the moment GetImage returns; leaking one per icon
        // per reindex is a GDI handle leak, and GDI handles are a hard limit.
        let _ = DeleteObject(bitmap.into());
        image
    }
}

/// Copy an `HBITMAP` out of GDI as straight RGBA8.
fn read_bitmap(bitmap: HBITMAP) -> Result<IconImage> {
    unsafe {
        let mut info = BITMAP::default();
        let written = GetObjectW(
            bitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        );
        anyhow::ensure!(written != 0, "GetObjectW rejected the icon bitmap");

        let width = info.bmWidth.max(0) as u32;
        let height = info.bmHeight.max(0) as u32;
        anyhow::ensure!(width > 0 && height > 0, "the shell returned an empty icon");

        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative height requests a top-down bitmap. Without it GDI
                // hands back bottom-up rows and every icon arrives mirrored.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut pixels = vec![0u8; (width * height * 4) as usize];

        let screen = GetDC(None);
        anyhow::ensure!(!screen.is_invalid(), "could not obtain a device context");
        let copied = GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, screen);
        anyhow::ensure!(copied != 0, "GetDIBits could not read the icon");

        to_straight_rgba(&mut pixels);

        Ok(IconImage {
            width,
            height,
            rgba: pixels,
        })
    }
}

/// Convert GDI's premultiplied BGRA in place to straight RGBA.
///
/// Both halves matter. The channel swap is obvious; the un-premultiplication is
/// not, and skipping it is what produces icons with dark fringes against a light
/// surface -- exactly the cheap look this whole module exists to avoid.
fn to_straight_rgba(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let (b, g, r, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);

        if a == 0 {
            // A fully transparent premultiplied pixel carries no colour at all.
            // Leaving the stored colour would make it visible anywhere the image
            // is filtered, as a halo around the icon's edges.
            pixel.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }

        let unpremultiply = |channel: u8| -> u8 {
            if a == 255 {
                channel
            } else {
                ((channel as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
            }
        };

        pixel[0] = unpremultiply(r);
        pixel[1] = unpremultiply(g);
        pixel[2] = unpremultiply(b);
        pixel[3] = a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_channels_for_opaque_pixels() {
        // BGRA in, RGBA out.
        let mut pixels = vec![10, 20, 30, 255];
        to_straight_rgba(&mut pixels);
        assert_eq!(pixels, vec![30, 20, 10, 255]);
    }

    #[test]
    fn undoes_premultiplication() {
        // Half-opaque mid grey: premultiplied 64 at alpha 128 is 128 straight.
        let mut pixels = vec![64, 64, 64, 128];
        to_straight_rgba(&mut pixels);
        assert_eq!(pixels[3], 128);
        assert!((127..=129).contains(&pixels[0]), "got {}", pixels[0]);
    }

    #[test]
    fn clears_colour_from_fully_transparent_pixels() {
        let mut pixels = vec![200, 200, 200, 0];
        to_straight_rgba(&mut pixels);
        assert_eq!(pixels, vec![0, 0, 0, 0]);
    }
}
