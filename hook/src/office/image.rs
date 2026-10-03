//! Pictures for Word and PowerPoint: PNG or JPEG only, recognized by their bytes (not their name), with their size
//! in pixels read from the header so they keep their proportions.

use super::OfficeError;
use super::access::Access;

/// Largest picture we embed.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// EMU (the unit Office draws in) per pixel at 96 dpi.
pub const EMU_PER_PX: u64 = 9525;
/// EMU per centimetre.
pub const EMU_PER_CM: f64 = 360_000.0;

#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub data: Vec<u8>,
    /// `png` or `jpeg`: the extension inside the package.
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({} {}x{}, {} bytes)", self.ext, self.width, self.height, self.data.len())
    }
}

impl Image {
    /// Reads the picture at `raw`, which must be inside an allowed folder.
    pub fn load(raw: &str, access: &Access) -> Result<Image, OfficeError> {
        let (_, data) = access.read(raw, MAX_IMAGE_BYTES)?;
        Image::from_bytes(data).ok_or_else(|| OfficeError(format!("{raw} no es una imagen PNG o JPEG válida.")))
    }

    pub fn from_bytes(data: Vec<u8>) -> Option<Image> {
        let (ext, (width, height)) = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            ("png", png_size(&data)?)
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            ("jpeg", jpeg_size(&data)?)
        } else {
            return None;
        };
        (width > 0 && height > 0).then_some(Image { data, ext, width, height })
    }

    /// The size in EMU that fits in `max_cx` × `max_cy`, `width` wide when asked (else its size at 96 dpi), never
    /// stretched.
    pub fn fit(&self, width: Option<u64>, max_cx: u64, max_cy: u64) -> (u64, u64) {
        let natural = self.width as u64 * EMU_PER_PX;
        let mut cx = width.unwrap_or(natural).clamp(EMU_PER_CM as u64 / 2, max_cx);
        let mut cy = cx * self.height as u64 / self.width as u64;
        if cy > max_cy {
            cy = max_cy;
            cx = cy * self.width as u64 / self.height as u64;
        }
        (cx.max(1), cy.max(1))
    }
}

fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 3 < data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], *data.get(i + 3)?]) as usize;
        let is_frame = (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker);
        if is_frame {
            let h = u16::from_be_bytes([*data.get(i + 5)?, *data.get(i + 6)?]) as u32;
            let w = u16::from_be_bytes([*data.get(i + 7)?, *data.get(i + 8)?]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// A tiny valid PNG (`w`×`h`, grey) for the tests, built with our own deflate.
#[cfg(test)]
pub fn test_png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(body);
        out.extend_from_slice(&crc_input);
        out.extend_from_slice(&super::zip::crc32(&crc_input).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8-bit greyscale
    chunk(&mut out, b"IHDR", &ihdr);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        raw.extend((0..w).map(|x| ((x + y) * 7 % 256) as u8));
    }
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_their_bytes() {
        let png = Image::from_bytes(test_png(40, 20)).unwrap();
        assert_eq!((png.ext, png.width, png.height), ("png", 40, 20));
        // A minimal JPEG header: SOI, an APP0 segment, then SOF0 with 30x10.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x0A, 0x00, 0x1E, 0x01, 0x01, 0x11, 0x00]);
        let jpeg = Image::from_bytes(jpeg).unwrap();
        assert_eq!((jpeg.ext, jpeg.width, jpeg.height), ("jpeg", 30, 10));
        assert!(Image::from_bytes(b"GIF89a....".to_vec()).is_none());
        assert!(Image::from_bytes(b"\x89PNG\r\n\x1a\n".to_vec()).is_none(), "cut short");
    }

    #[test]
    fn pictures_fit_without_stretching() {
        let img = Image::from_bytes(test_png(400, 200)).unwrap();
        assert_eq!(img.fit(None, 10_000_000, 10_000_000), (400 * EMU_PER_PX, 200 * EMU_PER_PX));
        assert_eq!(img.fit(Some(2_000_000), 10_000_000, 10_000_000), (2_000_000, 1_000_000));
        assert_eq!(img.fit(Some(9_000_000), 4_000_000, 10_000_000), (4_000_000, 2_000_000));
        assert_eq!(img.fit(None, 10_000_000, 1_000_000), (2_000_000, 1_000_000));
    }
}
