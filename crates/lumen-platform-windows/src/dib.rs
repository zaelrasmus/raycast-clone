//! Device-independent bitmaps, the form a picture takes on the clipboard
//! (F12), to and from straight RGBA.
//!
//! Pure, so the parsing -- the part with the edge cases -- is tested without a
//! clipboard. `CF_DIB` and `CF_DIBV5` are both a `BITMAPINFOHEADER` (or its
//! V4/V5 extension) followed by the pixels, with no file header.
//!
//! Read: 24 and 32 bits per pixel, `BI_RGB` and `BI_BITFIELDS`, bottom-up and
//! top-down. That is what screenshot tools, browsers and Windows' own
//! synthesis from `CF_BITMAP` produce; a palette or 16-bit picture is refused
//! rather than guessed at, and the copy is not recorded.
//!
//! Written: a `BITMAPV5HEADER` with 32 bits and an alpha mask, bottom-up,
//! which is the most widely read form that keeps transparency. Windows makes
//! `CF_DIB` and `CF_BITMAP` from it for whoever asks for those.

use anyhow::{Result, bail};
use lumen_platform::ClipboardImage;

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;
const V5_HEADER: usize = 124;
/// `LCS_sRGB`, the colour space a V5 header names.
const LCS_SRGB: u32 = 0x7352_4742;
/// `LCS_GM_IMAGES`.
const LCS_GM_IMAGES: u32 = 4;

/// A DIB's pixels as straight RGBA, top row first.
pub fn decode(bytes: &[u8]) -> Result<ClipboardImage> {
    let header_size = u32_at(bytes, 0)? as usize;
    if header_size < 40 {
        bail!("not a bitmap header ({header_size} bytes)");
    }
    let width = i32_at(bytes, 4)?;
    let height = i32_at(bytes, 8)?;
    let bit_count = u16_at(bytes, 14)?;
    let compression = u32_at(bytes, 16)?;
    if width <= 0 || height == 0 {
        bail!("a bitmap of {width}x{height}");
    }
    let top_down = height < 0;
    let (width, height) = (width as u32, height.unsigned_abs());

    let bitfields = matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS);
    if !(compression == BI_RGB || bitfields) || !(bit_count == 24 || bit_count == 32) {
        bail!("an unsupported bitmap ({bit_count} bits, compression {compression})");
    }
    if bitfields && bit_count != 32 {
        bail!("an unsupported bitmap (bit fields at {bit_count} bits)");
    }

    // The masks sit at the same offsets whether they are part of a V4/V5
    // header or follow a 40-byte one.
    let (masks, mut offset) = if bitfields {
        let has_alpha = header_size >= 56 || compression == BI_ALPHABITFIELDS;
        let masks = [
            u32_at(bytes, 40)?,
            u32_at(bytes, 44)?,
            u32_at(bytes, 48)?,
            if has_alpha { u32_at(bytes, 52)? } else { 0 },
        ];
        let trailing = match (header_size, compression) {
            (40, BI_BITFIELDS) => 12,
            (40, _) => 16,
            _ => 0,
        };
        (masks, header_size + trailing)
    } else {
        // Blue, green, red, then whatever the fourth byte is.
        (
            [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000],
            header_size,
        )
    };

    let stride = (width as usize * bit_count as usize).div_ceil(32) * 4;
    let size = stride * height as usize;
    // Some programs write the masks after a V5 header as well, as if it were
    // a 40-byte one; the length says so.
    if bitfields && header_size > 40 && bytes.len() >= offset + 12 + size {
        offset += 12;
    }
    let Some(pixels) = bytes.get(offset..offset + size) else {
        bail!(
            "the bitmap is shorter than its {width}x{height} pixels ({} bytes)",
            bytes.len()
        );
    };

    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    let mut any_alpha = false;
    for row in 0..height as usize {
        let source_row = if top_down {
            row
        } else {
            height as usize - 1 - row
        };
        let line = &pixels[source_row * stride..];
        for x in 0..width as usize {
            if bit_count == 24 {
                let pixel = &line[x * 3..x * 3 + 3];
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            } else {
                let pixel = &line[x * 4..x * 4 + 4];
                let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                let alpha = if masks[3] == 0 {
                    255
                } else {
                    channel(value, masks[3])
                };
                any_alpha |= alpha != 0;
                rgba.extend_from_slice(&[
                    channel(value, masks[0]),
                    channel(value, masks[1]),
                    channel(value, masks[2]),
                    alpha,
                ]);
            }
        }
    }
    // An alpha byte that is zero everywhere is a program that never filled
    // it in, not a picture that is entirely transparent.
    if bit_count == 32 && !any_alpha {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }

    Ok(ClipboardImage {
        width,
        height,
        rgba,
        png: None,
    })
}

/// `image` as a `CF_DIBV5`: 32 bits with alpha, bottom-up.
pub fn encode_v5(image: &ClipboardImage) -> Vec<u8> {
    let (width, height) = (image.width as usize, image.height as usize);
    let mut bytes = Vec::with_capacity(V5_HEADER + width * height * 4);
    let mut put = |value: u32| bytes.extend_from_slice(&value.to_le_bytes());
    put(V5_HEADER as u32);
    put(image.width);
    put(image.height);
    put(1 | (32 << 16)); // planes, then bits per pixel
    put(BI_BITFIELDS);
    put((width * height * 4) as u32);
    put(2835); // 72 dpi, in pixels per metre
    put(2835);
    put(0); // colours used
    put(0); // colours important
    for mask in [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
        put(mask);
    }
    put(LCS_SRGB);
    for _ in 0..9 {
        put(0); // endpoints
    }
    for _ in 0..3 {
        put(0); // gamma
    }
    put(LCS_GM_IMAGES);
    put(0); // profile data
    put(0); // profile size
    put(0); // reserved
    debug_assert_eq!(bytes.len(), V5_HEADER);

    for row in image.rgba.chunks_exact(width * 4).rev() {
        for pixel in row.chunks_exact(4) {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    bytes
}

/// One channel out of a pixel, scaled to eight bits.
fn channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let bits = mask.count_ones();
    let raw = (value & mask) >> mask.trailing_zeros();
    if bits == 8 {
        raw as u8
    } else {
        ((u64::from(raw) * 255) / ((1u64 << bits) - 1)) as u8
    }
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    match bytes.get(at..at + 4) {
        Some(b) => Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        None => bail!("the bitmap header is cut short"),
    }
}

fn i32_at(bytes: &[u8], at: usize) -> Result<i32> {
    u32_at(bytes, at).map(|value| value as i32)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    match bytes.get(at..at + 2) {
        Some(b) => Ok(u16::from_le_bytes([b[0], b[1]])),
        None => bail!("the bitmap header is cut short"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: i32, height: i32, bit_count: u16, compression: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&bit_count.to_le_bytes());
        bytes.extend_from_slice(&compression.to_le_bytes());
        bytes.extend_from_slice(&[0; 20]);
        bytes
    }

    #[test]
    fn a_picture_survives_the_round_trip_with_its_transparency() {
        let image = ClipboardImage {
            width: 3,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, //
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255,
            ],
            png: None,
        };
        let bytes = encode_v5(&image);
        assert_eq!(bytes.len(), V5_HEADER + 3 * 2 * 4);
        assert_eq!(decode(&bytes).unwrap(), image);
    }

    /// 24 bits, bottom-up, each row padded to four bytes: what Windows makes
    /// from a `CF_BITMAP`.
    #[test]
    fn a_bottom_up_24_bit_picture_is_read_with_its_padding() {
        let mut bytes = header(1, 2, 24, BI_RGB);
        // Bottom row first: blue, then red, each padded from 3 to 4 bytes.
        bytes.extend_from_slice(&[255, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 255, 0]);
        let image = decode(&bytes).unwrap();
        assert_eq!((image.width, image.height), (1, 2));
        assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn a_top_down_picture_is_not_flipped() {
        let mut bytes = header(1, -2, 32, BI_RGB);
        bytes.extend_from_slice(&[0, 0, 255, 255]); // red
        bytes.extend_from_slice(&[255, 0, 0, 255]); // blue
        let image = decode(&bytes).unwrap();
        assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    /// A 32-bit `BI_RGB` picture whose fourth byte nobody filled in is
    /// opaque, not invisible.
    #[test]
    fn an_alpha_that_is_zero_everywhere_means_opaque() {
        let mut bytes = header(2, 1, 32, BI_RGB);
        bytes.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.rgba, [3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn masks_after_a_short_header_are_read() {
        let mut bytes = header(1, 1, 32, BI_BITFIELDS);
        for mask in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff] {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
        bytes.extend_from_slice(&[30, 20, 10, 0]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.rgba, [10, 20, 30, 255]);
    }

    /// The quirk: masks written again after a V5 header that already has them.
    #[test]
    fn masks_repeated_after_a_v5_header_are_stepped_over() {
        let image = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![10, 20, 30, 255],
            png: None,
        };
        let mut bytes = encode_v5(&image);
        let pixels = bytes.split_off(V5_HEADER);
        for mask in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff] {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
        bytes.extend_from_slice(&pixels);
        assert_eq!(decode(&bytes).unwrap(), image);
    }

    #[test]
    fn what_is_not_supported_or_is_cut_short_is_refused() {
        assert!(decode(&header(1, 1, 8, BI_RGB)).is_err(), "a palette");
        assert!(decode(&header(1, 1, 32, 1)).is_err(), "run-length encoded");
        assert!(decode(&header(4, 4, 32, BI_RGB)).is_err(), "no pixels");
        assert!(decode(&[40, 0, 0]).is_err(), "no header");
        assert!(decode(&header(0, 1, 32, BI_RGB)).is_err(), "no width");
    }
}
