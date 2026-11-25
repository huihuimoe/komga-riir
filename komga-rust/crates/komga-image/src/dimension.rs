//! Image size from the header bytes of a stream.

use std::io::{self, Read};

use imagesize::ImageError;
use komga_types::Dimension;

const INITIAL_PREFIX: usize = 64 * 1024;
/// Headers that only resolve past this point (e.g. a JPEG frame behind megabytes of APP segments)
/// count as unreadable rather than buffering a whole page.
const MAX_PREFIX: usize = 16 * 1024 * 1024;

/// Returns the pixel size of the first image in `reader`, or `None` when the format is unsupported,
/// the header is malformed or truncated, or a side is zero.
///
/// Reads only the prefix the format's header needs, growing it on demand up to [`MAX_PREFIX`].
/// Sizes come from the stored header: EXIF orientation is not applied, GIF and animated WebP
/// report the canvas, and HEIF/AVIF apply the `irot` rotation.
#[must_use]
pub fn read_dimension(mut reader: impl Read) -> Option<Dimension> {
    let mut prefix: Vec<u8> = Vec::new();
    let mut wanted = INITIAL_PREFIX;
    loop {
        let missing = wanted - prefix.len();
        let read = (&mut reader)
            .take(missing as u64)
            .read_to_end(&mut prefix)
            .ok()?;
        match imagesize::blob_size(&prefix) {
            Ok(size) => {
                let width = u32::try_from(size.width).ok().filter(|&side| side > 0)?;
                let mut height = u32::try_from(size.height).ok()?;
                // `imagesize` reads the signed BMP height as unsigned; negative means top-down rows.
                if matches!(
                    imagesize::image_type(&prefix),
                    Ok(imagesize::ImageType::Bmp)
                ) {
                    height = height.cast_signed().unsigned_abs();
                }
                let height = Some(height).filter(|&side| side > 0)?;
                return Some(Dimension { width, height });
            }
            Err(ImageError::IoError(error))
                if error.kind() == io::ErrorKind::UnexpectedEof
                    && read == missing
                    && wanted < MAX_PREFIX =>
            {
                wanted = (wanted * 2).min(MAX_PREFIX);
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Cursor, Read};
    use std::path::Path;

    use super::*;

    const WIDTH: u32 = 48;
    const HEIGHT: u32 = 20;

    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        fs::read(&path)
            .unwrap_or_else(|error| panic!("fixture {} must exist: {error}", path.display()))
    }

    fn dimension(width: u32, height: u32) -> Option<Dimension> {
        Some(Dimension { width, height })
    }

    /// `photo.jpg` with `segments` inserted right after SOI.
    fn jpeg_with_leading_segments(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let photo = fixture("photo.jpg");
        let mut jpeg = photo[..2].to_vec();
        for (marker, payload) in segments {
            let length =
                u16::try_from(payload.len() + 2).expect("segment payload fits a JPEG length");
            jpeg.extend_from_slice(&[0xff, *marker]);
            jpeg.extend_from_slice(&length.to_be_bytes());
            jpeg.extend_from_slice(payload);
        }
        jpeg.extend_from_slice(&photo[2..]);
        jpeg
    }

    fn filler_segments(total: usize) -> Vec<(u8, Vec<u8>)> {
        let chunk = usize::from(u16::MAX) - 2;
        (0..total.div_ceil(chunk))
            .map(|_| (0xef, vec![0; chunk]))
            .collect()
    }

    #[test]
    fn reads_real_images() {
        for name in [
            "photo.jpg",
            "photo.png",
            "photo.bmp",
            "photo.tiff",
            "animated.gif",
            "lossy.webp",
            "lossless.webp",
            "animated.webp",
            "photo.heic",
            "photo.avif",
        ] {
            assert_eq!(
                read_dimension(&fixture(name)[..]),
                dimension(WIDTH, HEIGHT),
                "{name}"
            );
        }
    }

    #[test]
    fn reads_formats_without_fixture_encoders() {
        // Naked JPEG XL codestream: small SizeHeader with height 32 and width 64.
        let jxl = [0xff, 0x0a, 0x07, 0x0e, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(read_dimension(&jxl[..]), dimension(64, 32));

        let mut ppm = b"P6\n48 20\n255\n".to_vec();
        ppm.resize(ppm.len() + 48 * 20 * 3, 0);
        assert_eq!(read_dimension(&ppm[..]), dimension(WIDTH, HEIGHT));
    }

    #[test]
    fn reports_gif_canvas_when_first_frame_is_smaller() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[40, 0, 30, 0, 0, 0, 0]);
        gif.extend_from_slice(&[0x2c, 0, 0, 0, 0, 10, 0, 5, 0, 0, 2, 2, 0x4c, 0x01, 0, 0x3b]);
        assert_eq!(read_dimension(&gif[..]), dimension(40, 30));
    }

    #[test]
    fn reads_top_down_bmp_height() {
        // A negative height marks rows stored top-down.
        let mut bmp = fixture("photo.bmp");
        let height = -i32::try_from(HEIGHT).expect("Small fixture");
        bmp[0x16..0x1a].copy_from_slice(&height.to_le_bytes());
        assert_eq!(read_dimension(&bmp[..]), dimension(WIDTH, HEIGHT));
    }

    #[test]
    fn keeps_stored_jpeg_size_regardless_of_exif_orientation() {
        // Big-endian EXIF with one IFD0 entry: Orientation (0x0112) = 6, rotate 90° clockwise.
        let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01".to_vec();
        exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0]);
        let jpeg = jpeg_with_leading_segments(&[(0xe1, exif)]);
        assert_eq!(read_dimension(&jpeg[..]), dimension(WIDTH, HEIGHT));
    }

    #[test]
    fn grows_prefix_until_header_resolves() {
        let jpeg = jpeg_with_leading_segments(&filler_segments(3 * INITIAL_PREFIX));
        assert_eq!(read_dimension(&jpeg[..]), dimension(WIDTH, HEIGHT));

        let jpeg = jpeg_with_leading_segments(&filler_segments(MAX_PREFIX));
        assert_eq!(
            read_dimension(&jpeg[..]),
            None,
            "header beyond the prefix cap"
        );
    }

    #[test]
    fn reads_only_the_header_prefix() {
        struct Counting<R> {
            inner: R,
            read: usize,
        }
        impl<R: Read> Read for Counting<R> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let read = self.inner.read(buf)?;
                self.read += read;
                Ok(read)
            }
        }

        let mut page = fixture("photo.png");
        page.resize(page.len() + 8 * MAX_PREFIX, 0);
        let mut reader = Counting {
            inner: Cursor::new(page),
            read: 0,
        };
        assert_eq!(read_dimension(&mut reader), dimension(WIDTH, HEIGHT));
        assert!(
            reader.read <= INITIAL_PREFIX,
            "read {} bytes for a PNG header",
            reader.read
        );
    }

    #[test]
    fn rejects_unreadable_input() {
        let photo = fixture("photo.jpg");
        assert_eq!(
            read_dimension(&photo[..20]),
            None,
            "JPEG truncated before its frame header"
        );
        assert_eq!(
            read_dimension(&fixture("photo.png")[..20]),
            None,
            "PNG truncated in IHDR"
        );
        assert_eq!(
            read_dimension(&fixture("photo.jp2")[..]),
            None,
            "JPEG 2000 is not probed"
        );
        assert_eq!(read_dimension(&b"plain text, not an image"[..]), None);
        assert_eq!(read_dimension(&[][..]), None);

        let mut zero_width = fixture("photo.png");
        zero_width[16..20].fill(0);
        assert_eq!(read_dimension(&zero_width[..]), None, "zero width");

        let failing = Cursor::new(photo[..4].to_vec()).chain(FailingReader);
        assert_eq!(read_dimension(failing), None, "stream error");
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("member stream failed"))
        }
    }
}
