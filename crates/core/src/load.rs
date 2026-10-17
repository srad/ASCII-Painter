//! Still-image loading and input classification.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};

use crate::CoreError;
use crate::types::Rgb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// A single image the `image` crate can decode.
    Still,
    /// An animated GIF/PNG/WebP — handled like a video.
    Animated,
    /// Not a recognised image; probably a video (decided by ffprobe).
    Other,
}

pub fn classify(path: &Path) -> InputKind {
    let Ok(reader) = ImageReader::open(path).and_then(|r| r.with_guessed_format()) else {
        return InputKind::Other;
    };
    let Some(format) = reader.format() else { return InputKind::Other };
    let animated = || -> Option<bool> {
        let file = BufReader::new(File::open(path).ok()?);
        Some(match format {
            ImageFormat::Gif => GifDecoder::new(file).ok()?.into_frames().take(2).count() > 1,
            ImageFormat::Png => PngDecoder::new(file).ok()?.is_apng().ok()?,
            ImageFormat::WebP => WebPDecoder::new(file).ok()?.has_animation(),
            _ => false,
        })
    };
    if animated().unwrap_or(false) { InputKind::Animated } else { InputKind::Still }
}

/// Decode a still image upright (EXIF orientation applied).
pub fn load_oriented(path: &Path) -> Result<DynamicImage, CoreError> {
    let mut decoder = ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Decode a still image upright with alpha flattened onto `background`.
pub fn load_still(path: &Path, background: Rgb) -> Result<RgbImage, CoreError> {
    Ok(flatten(load_oriented(path)?, background))
}

pub fn flatten(image: DynamicImage, background: Rgb) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    let bg = [background.r, background.g, background.b];
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        image::Rgb(std::array::from_fn(|c| ((p[c] as u32 * a + bg[c] as u32 * (255 - a) + 127) / 255) as u8))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixels_take_the_background() {
        let mut rgba = image::RgbaImage::new(2, 1);
        rgba.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        rgba.put_pixel(1, 0, image::Rgba([255, 0, 0, 0]));
        let flat = flatten(DynamicImage::ImageRgba8(rgba), Rgb::new(0, 0, 255));
        assert_eq!(flat.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(flat.get_pixel(1, 0).0, [0, 0, 255]);
    }
}
