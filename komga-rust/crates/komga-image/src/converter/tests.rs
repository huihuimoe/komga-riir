use std::fs;
use std::path::Path;

use image::{GenericImageView, ImageFormat, RgbaImage};

use super::*;

const WIDTH: u32 = 48;
const HEIGHT: u32 = 20;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read(&path).unwrap_or_else(|error| panic!("fixture {} must exist: {error}", path.display()))
}

fn png(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_with_encoder(PngEncoder::new(&mut bytes))
        .expect("Encode test PNG");
    bytes
}

fn decoded(bytes: &[u8], format: ImageFormat) -> DynamicImage {
    image::load_from_memory_with_format(bytes, format)
        .unwrap_or_else(|error| panic!("Output must be a valid {format:?}: {error}"))
}

/// `photo.png` with its IHDR claiming `width`x`height`, CRC fixed so only the size is wrong.
fn png_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut png = fixture("photo.png");
    // Signature (8), length (4), then the IHDR type and data whose CRC follows.
    let ihdr = 12..29;
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32fast::hash(&png[ihdr.clone()]);
    png[ihdr.end..ihdr.end + 4].copy_from_slice(&crc.to_be_bytes());
    png
}

/// `photo.jpg` with its baseline frame header claiming `width`x`height`.
fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
    let mut jpeg = fixture("photo.jpg");
    let frame = jpeg
        .windows(2)
        .position(|marker| marker == [0xff, 0xc0])
        .expect("Baseline JPEG fixture has SOF0");
    jpeg[frame + 5..frame + 7].copy_from_slice(&height.to_be_bytes());
    jpeg[frame + 7..frame + 9].copy_from_slice(&width.to_be_bytes());
    jpeg
}

/// The HEIF fixture `name` with the payload of its one `property` box, at `offset`, replaced.
fn heif_patched(name: &str, property: &[u8], offset: usize, payload: &[u8]) -> Vec<u8> {
    let mut heif = fixture(name);
    let start = heif
        .windows(4)
        .position(|window| window == property)
        .expect("HEIF fixture has the property")
        + 4
        + offset;
    heif[start..start + payload.len()].copy_from_slice(payload);
    heif
}

/// The HEIF fixture `name` with its `ispe` property claiming `width`x`height`.
fn heif_claiming(name: &str, width: u32, height: u32) -> Vec<u8> {
    let size = [width.to_be_bytes(), height.to_be_bytes()].concat();
    // Version and flags precede the size.
    heif_patched(name, b"ispe", 4, &size)
}

/// The bare codestream inside `photo.jp2`, from its SOC marker.
fn jpeg_2000_codestream() -> Vec<u8> {
    let jp2 = fixture("photo.jp2");
    let start = jp2
        .windows(4)
        .position(|window| window == JPEG_2000_CODESTREAM)
        .expect("JP2 fixture holds a codestream");
    jp2[start..].to_vec()
}

/// `photo.jp2` with its SIZ marker claiming `width`x`height`.
fn jpeg_2000_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut jp2 = fixture("photo.jp2");
    let start = jp2
        .windows(4)
        .position(|window| window == JPEG_2000_CODESTREAM)
        .expect("JP2 fixture holds a codestream");
    // SOC, the SIZ marker, its length, and the capabilities precede the reference grid size.
    jp2[start + 8..start + 12].copy_from_slice(&width.to_be_bytes());
    jp2[start + 12..start + 16].copy_from_slice(&height.to_be_bytes());
    jp2
}

/// `photo.jbig2` with its page information segment claiming `width`x`height`.
fn jbig2_claiming(width: u32, height: u32) -> Vec<u8> {
    let mut jbig2 = fixture("photo.jbig2");
    // The file header (13 bytes) and the segment header (11) precede the page size.
    jbig2[24..28].copy_from_slice(&width.to_be_bytes());
    jbig2[28..32].copy_from_slice(&height.to_be_bytes());
    jbig2
}

/// `photo.pcx` with its window and line length claiming `side`x`side`.
fn pcx_claiming(side: u16) -> Vec<u8> {
    let mut pcx = fixture("photo.pcx");
    let last = (side - 1).to_le_bytes();
    pcx[8..10].copy_from_slice(&last);
    pcx[10..12].copy_from_slice(&last);
    pcx[66..68].copy_from_slice(&side.to_le_bytes());
    pcx
}

/// `value` as a WBMP multi-byte integer: big-endian 7-bit groups, all but the last flagged.
fn multi_byte_integer(value: u32) -> Vec<u8> {
    let mut bytes = vec![u8::try_from(value & 0x7f).expect("7 bits")];
    let mut rest = value >> 7;
    while rest > 0 {
        bytes.insert(0, 0x80 | u8::try_from(rest & 0x7f).expect("7 bits"));
        rest >>= 7;
    }
    bytes
}

/// A blank `side`x`side` WBMP; `probe` checks the payload length as WBMP has no magic bytes.
fn wbmp_claiming(side: u32) -> Vec<u8> {
    // Type and fixed header, then width and height.
    let mut wbmp = vec![0, 0];
    wbmp.extend(multi_byte_integer(side));
    wbmp.extend(multi_byte_integer(side));
    let row = usize::try_from(side.div_ceil(8)).expect("Small row");
    wbmp.resize(
        wbmp.len() + row * usize::try_from(side).expect("Small side"),
        0,
    );
    wbmp
}

#[test]
fn converts_every_readable_format_to_jpeg_and_png() {
    for (name, media_type, bytes) in [
        ("photo.avif", "image/avif", fixture("photo.avif")),
        ("photo.bmp", "image/bmp", fixture("photo.bmp")),
        ("animated.gif", "image/gif", fixture("animated.gif")),
        ("photo.jpg", "image/jpeg", fixture("photo.jpg")),
        ("photo.png", "image/png", fixture("photo.png")),
        ("photo.tiff", "image/tiff", fixture("photo.tiff")),
        ("photo.heic", "image/heic", fixture("photo.heic")),
        ("lossy.webp", "image/webp", fixture("lossy.webp")),
        ("lossless.webp", "image/webp", fixture("lossless.webp")),
        ("animated.webp", "image/webp", fixture("animated.webp")),
        ("photo.jxl", "image/jxl", fixture("photo.jxl")),
        ("photo.jp2", "image/jp2", fixture("photo.jp2")),
        (
            "codestream",
            "image/x-jp2-codestream",
            jpeg_2000_codestream(),
        ),
        ("photo.jbig2", "image/x-jbig2", fixture("photo.jbig2")),
        ("photo.pcx", "image/vnd.zbrush.pcx", fixture("photo.pcx")),
        ("photo.wbmp", "image/vnd.wap.wbmp", fixture("photo.wbmp")),
    ] {
        assert!(can_read(media_type), "{name}: {media_type}");
        for (target, format) in [
            (ImageType::Jpeg, ImageFormat::Jpeg),
            (ImageType::Png, ImageFormat::Png),
        ] {
            let output = convert(&bytes, target)
                .unwrap_or_else(|error| panic!("{name} to {target:?}: {error}"));
            let image = decoded(&output, format);
            assert_eq!(image.dimensions(), (WIDTH, HEIGHT), "{name} to {target:?}");
        }
    }
}

/// Big-endian EXIF with one IFD0 entry: Orientation (0x0112) = 6, rotate 90° clockwise.
fn exif_rotated() -> Vec<u8> {
    let mut exif = b"MM\0\x2a\0\0\0\x08\0\x01".to_vec();
    exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0]);
    exif
}

/// `photo.jxl`, a bare codestream, in the ISO BMFF container: signature, `ftyp`, an `Exif` box
/// when `exif` is given, and `jxlc`.
fn jpeg_xl_container(exif: Option<&[u8]>) -> Vec<u8> {
    let codestream = fixture("photo.jxl");
    let mut container = JPEG_XL_CONTAINER.to_vec();
    container.extend(20_u32.to_be_bytes());
    container.extend(b"ftypjxl \0\0\0\0jxl ");
    if let Some(exif) = exif {
        // The payload starts with the offset of the TIFF header.
        let length = u32::try_from(exif.len() + 12).expect("Short EXIF");
        container.extend(length.to_be_bytes());
        container.extend(b"Exif\0\0\0\0");
        container.extend(exif);
    }
    let length = u32::try_from(codestream.len() + 8).expect("Small fixture");
    container.extend(length.to_be_bytes());
    container.extend(b"jxlc");
    container.extend(codestream);
    container
}

#[test]
fn decodes_jpeg_xl_codestreams_and_containers() {
    let png = decoded(
        &convert(&fixture("photo.png"), ImageType::Png).expect("PNG to PNG"),
        ImageFormat::Png,
    );
    for (case, bytes) in [
        ("codestream", fixture("photo.jxl")),
        ("container", jpeg_xl_container(None)),
        // The codestream's orientation, upright here, overrides EXIF.
        (
            "container with EXIF",
            jpeg_xl_container(Some(&exif_rotated())),
        ),
    ] {
        // The fixture is `photo.png` encoded losslessly.
        let image = decoded(
            &convert(&bytes, ImageType::Png).expect("JPEG XL to PNG"),
            ImageFormat::Png,
        );
        assert!(image.to_rgb8() == png.to_rgb8(), "{case}: lossless pixels");
        let thumbnail = decoded(
            &resize(&bytes, ImageType::Jpeg, 24).expect("JPEG XL thumbnail"),
            ImageFormat::Jpeg,
        );
        assert_eq!(thumbnail.dimensions(), (24, 10), "{case}");
    }
}

#[test]
fn applies_heif_transforms() {
    for name in ["photo.heic", "photo.avif"] {
        // One quarter turn anticlockwise.
        let rotated = heif_patched(name, b"irot", 0, &[1]);
        let image = decoded(
            &convert(&rotated, ImageType::Png).expect("HEIF to PNG"),
            ImageFormat::Png,
        );
        assert_eq!(image.dimensions(), (HEIGHT, WIDTH), "{name}");
        let thumbnail = decoded(
            &resize(&rotated, ImageType::Jpeg, 24).expect("HEIF thumbnail"),
            ImageFormat::Jpeg,
        );
        assert_eq!(thumbnail.dimensions(), (10, 24), "{name}");
    }
}

fn png_pixels(bytes: &[u8]) -> DynamicImage {
    decoded(
        &convert(bytes, ImageType::Png).expect("To PNG"),
        ImageFormat::Png,
    )
}

#[test]
fn decodes_jpeg_2000_codestreams_like_their_files() {
    let file = png_pixels(&fixture("photo.jp2"));
    assert!(
        png_pixels(&jpeg_2000_codestream()).to_rgb8() == file.to_rgb8(),
        "same pixels"
    );
    let thumbnail = decoded(
        &resize(&jpeg_2000_codestream(), ImageType::Jpeg, 24).expect("JPEG 2000 thumbnail"),
        ImageFormat::Jpeg,
    );
    assert_eq!(thumbnail.dimensions(), (24, 10));
}

#[test]
fn decodes_pcx_and_bilevel_formats_losslessly() {
    let photo = png_pixels(&fixture("photo.png")).to_rgb8();
    assert!(
        png_pixels(&fixture("photo.pcx")).to_rgb8() == photo,
        "PCX holds `photo.png` in 24-bit RGB"
    );
    // Both hold `photo.png` thresholded at half luminance.
    let bilevel = image::DynamicImage::ImageRgb8(photo).to_luma8();
    let bilevel = image::GrayImage::from_fn(WIDTH, HEIGHT, |x, y| {
        image::Luma([if bilevel.get_pixel(x, y).0[0] < 128 {
            0
        } else {
            255
        }])
    });
    for name in ["photo.jbig2", "photo.wbmp"] {
        assert!(png_pixels(&fixture(name)).to_luma8() == bilevel, "{name}");
    }
}

#[test]
fn converts_portable_anymap() {
    let ppm = b"P6 2 1 255\n\xff\0\0\0\0\xff";
    let image = decoded(
        &convert(ppm, ImageType::Png).expect("PPM to PNG"),
        ImageFormat::Png,
    );
    assert_eq!(image.to_rgb8().into_raw(), [255, 0, 0, 0, 0, 255]);
}

#[test]
fn reads_only_formats_it_decodes() {
    for media_type in [
        "image/heif",
        "image/heif-sequence",
        "image/heic-sequence",
        "image/x-portable-bitmap",
        "image/x-portable-graymap",
        "image/x-portable-pixmap",
    ] {
        assert!(can_read(media_type), "{media_type}");
    }
    for media_type in [
        "image/jpx",
        "image/vnd.adobe.photoshop",
        "image/x-png",
        "application/xml",
    ] {
        assert!(!can_read(media_type), "{media_type}");
    }
}

#[test]
fn lays_transparency_on_white_for_jpeg_only() {
    let mut source = RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 0]));
    source.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
    let source = png(&source);

    let jpeg = decoded(
        &convert(&source, ImageType::Jpeg).expect("To JPEG"),
        ImageFormat::Jpeg,
    );
    let [red, green, blue] = jpeg.to_rgb8().get_pixel(15, 15).0;
    assert!(
        red > 250 && green > 250 && blue > 250,
        "transparent pixel must be white, got {red} {green} {blue}"
    );

    let png = decoded(
        &convert(&source, ImageType::Png).expect("To PNG"),
        ImageFormat::Png,
    );
    assert_eq!(png.to_rgba8().get_pixel(15, 15).0, [0, 0, 0, 0]);
    assert_eq!(png.to_rgba8().get_pixel(0, 0).0, [0, 0, 0, 255]);
}

#[test]
fn composites_over_white_exactly() {
    assert_eq!(over_white(0, 0), 255);
    assert_eq!(over_white(0, 255), 0);
    assert_eq!(over_white(0, 128), 127);
    assert_eq!(over_white(200, 255), 200);
}

#[test]
fn applies_exif_orientation() {
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(exif_rotated());
    let photo = fixture("photo.jpg");
    let length = u16::try_from(exif.len() + 2).expect("Short segment");
    let mut jpeg = photo[..2].to_vec();
    jpeg.extend([0xff, 0xe1]);
    jpeg.extend(length.to_be_bytes());
    jpeg.extend(exif);
    jpeg.extend(&photo[2..]);

    let image = decoded(
        &convert(&jpeg, ImageType::Png).expect("To PNG"),
        ImageFormat::Png,
    );
    assert_eq!(image.dimensions(), (HEIGHT, WIDTH));
}

#[test]
fn refuses_images_over_the_pixel_limit_from_their_header() {
    // 12,000 x 12,000 is 144,000,000 pixels; the fixtures hold only 48 x 20 of data.
    for (format, bytes) in [
        ("PNG", png_claiming(12_000, 12_000)),
        ("JPEG", jpeg_claiming(12_000, 12_000)),
        ("HEIC", heif_claiming("photo.heic", 12_000, 12_000)),
        ("AVIF", heif_claiming("photo.avif", 12_000, 12_000)),
        ("JPEG 2000", jpeg_2000_claiming(12_000, 12_000)),
        ("JBIG2", jbig2_claiming(12_000, 12_000)),
        ("PCX", pcx_claiming(12_000)),
        ("WBMP", wbmp_claiming(16_000)),
    ] {
        let result = convert(&bytes, ImageType::Jpeg);
        assert!(
            matches!(result, Err(ImageError::TooLarge)),
            "{format}: {result:?}"
        );
    }
}

#[test]
fn refuses_jpeg_2000_whose_decoding_samples_exceed_the_memory_limit() {
    // 10,000 x 10,000 is within the pixel limit, but three 32-bit components take 1.2 GB.
    assert!(matches!(
        convert(&jpeg_2000_claiming(10_000, 10_000), ImageType::Jpeg),
        Err(ImageError::TooLarge)
    ));
}

#[test]
fn reports_undecodable_data() {
    let png = fixture("photo.png");
    for (case, bytes) in [
        ("text", b"not an image".to_vec()),
        ("truncated PNG", png[..png.len() / 2].to_vec()),
        ("truncated HEIC", fixture("photo.heic")[..600].to_vec()),
        ("truncated AVIF", fixture("photo.avif")[..600].to_vec()),
        ("MP4", b"\0\0\0\x14ftypisom\0\0\x02\0isom".to_vec()),
        ("truncated JPEG 2000", fixture("photo.jp2")[..3300].to_vec()),
        ("truncated JBIG2", fixture("photo.jbig2")[..60].to_vec()),
        ("truncated PCX", fixture("photo.pcx")[..500].to_vec()),
    ] {
        assert!(
            matches!(convert(&bytes, ImageType::Png), Err(ImageError::Decode(_))),
            "{case}"
        );
    }
}

#[test]
fn resizes_to_fit_a_square_keeping_the_aspect_ratio() {
    let tall = png(&RgbaImage::from_pixel(
        600,
        900,
        image::Rgba([0, 0, 255, 255]),
    ));
    let image = decoded(
        &resize(&tall, ImageType::Jpeg, 300).expect("Resize"),
        ImageFormat::Jpeg,
    );
    assert_eq!(image.dimensions(), (200, 300));
    let [red, green, blue] = image.to_rgb8().get_pixel(100, 150).0;
    assert!(red < 10 && green < 10 && blue > 245, "{red} {green} {blue}");

    let image = decoded(
        &resize(&fixture("photo.jpg"), ImageType::Jpeg, 24).expect("Resize"),
        ImageFormat::Jpeg,
    );
    assert_eq!(image.dimensions(), (24, 10));
}

#[test]
fn never_scales_up_and_keeps_small_images_already_in_the_format() {
    let image = decoded(
        &resize(&fixture("photo.png"), ImageType::Jpeg, 300).expect("Resize"),
        ImageFormat::Jpeg,
    );
    assert_eq!(image.dimensions(), (WIDTH, HEIGHT));

    let jpeg = fixture("photo.jpg");
    for size in [300, WIDTH] {
        let resized = resize(&jpeg, ImageType::Jpeg, size).expect("Resize");
        assert!(resized == jpeg, "a JPEG within {size} is returned as given");
    }
}

#[test]
fn resizing_refuses_oversized_and_undecodable_images() {
    assert!(matches!(
        resize(&png_claiming(12_000, 12_000), ImageType::Jpeg, 300),
        Err(ImageError::TooLarge)
    ));
    assert!(matches!(
        resize(b"not an image", ImageType::Jpeg, 300),
        Err(ImageError::Decode(_))
    ));
}

#[test]
fn rewritten_jpegs_drop_metadata_and_keep_their_orientation() {
    let plain = fixture("photo.jpg");
    let mut tagged = plain[..2].to_vec();
    let exif = [b"Exif\0\0".as_slice(), &exif_rotated()].concat();
    let length = u16::try_from(exif.len() + 2).expect("Short EXIF");
    tagged.extend_from_slice(&[0xff, 0xe1]);
    tagged.extend_from_slice(&length.to_be_bytes());
    tagged.extend_from_slice(&exif);
    tagged.extend_from_slice(&plain[2..]);

    let rewritten = rewrite_jpeg(&plain).expect("Rewrite JPEG");
    assert_eq!(
        rewrite_jpeg(&tagged).expect("Rewrite tagged JPEG"),
        rewritten
    );
    let original = decoded(&plain, ImageFormat::Jpeg);
    let output = decoded(&rewritten, ImageFormat::Jpeg);
    assert_eq!(output.dimensions(), original.dimensions(), "not rotated");

    assert!(matches!(
        rewrite_jpeg(&jpeg_claiming(u16::MAX, u16::MAX)),
        Err(ImageError::TooLarge)
    ));
    assert!(matches!(
        rewrite_jpeg(&fixture("photo.png")),
        Err(ImageError::Decode(_))
    ));
}

#[test]
fn rgb_helpers_report_buffer_size_mismatch_as_error() {
    assert!(matches!(
        encode_rgb(4, 4, vec![0; 3], ImageType::Png),
        Err(ImageError::Encode(_))
    ));
    assert!(matches!(
        resize_rgb(4, 4, vec![0; 3], ImageType::Jpeg, 2),
        Err(ImageError::Encode(_))
    ));
}
