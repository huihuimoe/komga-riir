//! Kotlin `MosaicGenerator`: up to four images in the quadrants of a poster.

use image::{DynamicImage, Rgba, RgbaImage, imageops};

use crate::ImageType;
use crate::converter::{self, ImageError};

/// The poster's width over its height, Kotlin's 0.7066666667.
const RATIO_NUMERATOR: u32 = 53;
const RATIO_DENOMINATOR: u32 = 75;

/// `images`, each scaled down to fit half of `height` and never scaled up, drawn from the
/// top-left corners of the quadrants in reading order over black, and written as `format`. Images
/// past the fourth are ignored; wider images run into the next quadrant until it is drawn.
///
/// Unlike Kotlin, which fails the whole poster, an image that cannot be decoded is skipped as a
/// missing one is, and the next takes its quadrant.
pub fn mosaic(images: &[&[u8]], height: u32, format: ImageType) -> Result<Vec<u8>, ImageError> {
    let width = (height * RATIO_NUMERATOR + RATIO_DENOMINATOR / 2) / RATIO_DENOMINATOR;
    let (half_width, half_height) = (width / 2, height / 2);
    let mut poster = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, u8::MAX]));
    let corners = [
        (0, 0),
        (half_width, 0),
        (0, half_height),
        (half_width, half_height),
    ];
    let decoded = images
        .iter()
        .filter_map(|bytes| converter::decode(bytes).ok());
    for (mut image, (x, y)) in decoded.zip(corners) {
        if image.width().max(image.height()) > half_height {
            image = image.thumbnail(half_height, half_height);
        }
        imageops::overlay(&mut poster, &image.to_rgba8(), x.into(), y.into());
    }
    // Opaque throughout, so JPEG's composition over white changes nothing.
    converter::encode(&DynamicImage::ImageRgba8(poster), format)
}

#[cfg(test)]
mod tests {
    use image::{GenericImageView, ImageFormat, Rgb, RgbImage};

    use super::mosaic;
    use crate::ImageType;

    fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        RgbImage::from_pixel(width, height, Rgb(color))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("Encode");
        bytes
    }

    fn pixel(poster: &image::DynamicImage, x: u32, y: u32) -> [u8; 4] {
        poster.get_pixel(x, y).0
    }

    #[test]
    fn quadrants_hold_the_images_in_order_over_black() {
        let red = png(10, 10, [255, 0, 0]);
        let green = png(300, 300, [0, 255, 0]);
        let blue = png(150, 150, [0, 0, 255]);
        let bytes =
            mosaic(&[&red, b"not an image", &green, &blue], 300, ImageType::Png).expect("Mosaic");
        let poster = image::load_from_memory(&bytes).expect("Decode");
        assert_eq!(poster.dimensions(), (212, 300));
        // Not scaled up: the rest of the first quadrant stays black.
        assert_eq!(pixel(&poster, 5, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&poster, 50, 50), [0, 0, 0, 255]);
        // Scaled down to 150 and clipped at the right edge.
        assert_eq!(pixel(&poster, 106, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&poster, 211, 149), [0, 255, 0, 255]);
        assert_eq!(pixel(&poster, 149, 150), [0, 0, 255, 255]);
        assert_eq!(pixel(&poster, 160, 160), [0, 0, 0, 255]);
    }

    #[test]
    fn no_images_give_a_black_poster() {
        let bytes = mosaic(&[], 600, ImageType::Jpeg).expect("Mosaic");
        let poster = image::load_from_memory(&bytes).expect("Decode");
        assert_eq!(poster.dimensions(), (424, 600));
        assert_eq!(pixel(&poster, 200, 300), [0, 0, 0, 255]);
    }
}
