//! The EAN-13 barcode Kotlin `IsbnBarcodeProvider` reads with ZXing: a hybrid binarizer over
//! `RGBLuminanceSource` luminance, trying harder. Unlike Kotlin, a barcode lying on its side is
//! read as well.

use rxing::common::HybridBinarizer;
use rxing::oned::EAN13Reader;
use rxing::{BinaryBitmap, DecodeHints, Luma8Source, Reader};

use crate::ImageError;
use crate::converter;

/// The digits of the first EAN-13 barcode found in the image `bytes`; `None` without one.
///
/// # Errors
/// The image cannot be decoded or exceeds the decoding limits.
pub fn read_ean13(bytes: &[u8]) -> Result<Option<String>, ImageError> {
    let rgb = converter::decode(bytes)?.to_rgb8();
    Ok(read_ean13_rgb(rgb.width(), rgb.height(), rgb.as_raw()))
}

/// [`read_ean13`] for a decoded `width` by `height` 8-bit RGB buffer.
#[must_use]
pub fn read_ean13_rgb(width: u32, height: u32, rgb: &[u8]) -> Option<String> {
    // ZXing `RGBLuminanceSource`: (R + 2G + B) / 4.
    let luma: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[r, g, b]| {
            u8::try_from((u16::from(r) + 2 * u16::from(g) + u16::from(b)) / 4).expect("At most 255")
        })
        .collect();
    let source = Luma8Source::new_with_slice(&luma, width, height).ok()?;
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(source));
    let hints = DecodeHints {
        TryHarder: Some(true),
        ..DecodeHints::default()
    };
    EAN13Reader
        .decode_with_hints(&mut bitmap, &hints)
        .ok()
        .map(|result| result.getText().to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{read_ean13, read_ean13_rgb};
    use crate::converter;

    fn fixture(name: &str) -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("Fixture")
    }

    /// Kotlin `IsbnBarcodeProviderTest`.
    #[test]
    fn barcodes_are_read_from_page_images() {
        assert_eq!(
            read_ean13(&fixture("barcode/page_384.jpg"))
                .unwrap()
                .as_deref(),
            Some("9782811632397")
        );
        assert_eq!(read_ean13(&fixture("barcode/komga.png")).unwrap(), None);
        assert!(read_ean13(&[]).is_err(), "no image");
    }

    #[test]
    fn barcodes_lying_on_their_side_are_read() {
        let page = converter::decode(&fixture("barcode/page_384.jpg"))
            .unwrap()
            .to_rgb8();
        for rotated in [
            image::imageops::rotate90(&page),
            image::imageops::rotate270(&page),
        ] {
            assert_eq!(
                read_ean13_rgb(rotated.width(), rotated.height(), rotated.as_raw()).as_deref(),
                Some("9782811632397")
            );
        }
    }
}
