//! Images the user attaches, prepared once for every provider: shrunk so a screenshot does not cost a
//! fortune in tokens, and stripped of metadata. Providers only decide how to hand the file over
//! (Claude: `image` blocks, Codex: `localImage`, Gemini: the file path); none of them resizes anything.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat, imageops::FilterType};

/// Longest side sent to a model; larger images are scaled down (models downscale them anyway).
pub const MAX_SIDE: u32 = 1568;

const EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// `image/png` and friends, from the extension.
pub fn mime(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// The image ready to send: its bytes and extension. Transparent images stay PNG, the rest become JPEG
/// (much smaller for photos and screenshots). Animated GIFs keep only their first frame.
pub fn prepare(bytes: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| format!("imagen no válida: {e}"))?;
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }
    let mut out = Cursor::new(Vec::new());
    if img.color().has_alpha() {
        img.write_to(&mut out, ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok((out.into_inner(), "png"))
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut out, ImageFormat::Jpeg).map_err(|e| e.to_string())?;
        Ok((out.into_inner(), "jpg"))
    }
}

/// Saves a screen grab given as BGRA rows (Windows' GDI order) as an opaque PNG.
pub fn write_bgra_png(width: u32, height: u32, bgra: &[u8], path: &Path) -> Result<(), String> {
    let mut rgba = bgra.to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    image::RgbaImage::from_raw(width, height, rgba).ok_or("la captura no tiene el tamaño esperado")?.save(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let img = if alpha {
            DynamicImage::ImageRgba8(image::RgbaImage::new(w, h))
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::new(w, h))
        };
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn big_images_are_scaled_down_keeping_proportions() {
        let (bytes, ext) = prepare(&png(3136, 1568, false)).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height(), ext), (1568, 784, "jpg"));
    }

    #[test]
    fn small_images_keep_their_size_and_transparency_stays_png() {
        let (bytes, ext) = prepare(&png(40, 30, true)).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height(), ext), (40, 30, "png"));
    }

    #[test]
    fn garbage_is_refused_and_extensions_are_recognized() {
        assert!(prepare(b"no soy una imagen").is_err());
        assert!(is_image(Path::new("/x/Foto.JPG")) && !is_image(Path::new("/x/notas.txt")));
        assert_eq!(mime(Path::new("a.webp")), "image/webp");
    }

    #[test]
    fn a_bgra_grab_becomes_an_opaque_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.png");
        write_bgra_png(2, 1, &[255, 0, 0, 0, 0, 0, 255, 7], &path).unwrap();
        let img = image::open(&path).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255], "blue stays blue");
        assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert!(write_bgra_png(3, 3, &[0; 4], &path).is_err());
    }
}
