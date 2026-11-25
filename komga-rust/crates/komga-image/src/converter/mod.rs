//! Kotlin `ImageConverter`: decoding page images, scaling them down, and encoding them as JPEG
//! or PNG.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::metadata::Orientation;
use image::{
    DynamicImage, GrayImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgb, RgbImage,
    RgbaImage,
};
use jxl_oxide::integration::JxlDecoder;
use oxideav_avif::AvifError;
use oxideav_pcx::PcxError;
use oxideav_wbmp::WbmpError;
use thiserror::Error;

/// Largest decoded or rendered image, in pixels. Kotlin has no limit.
pub const MAX_PIXELS: u64 = 128 * 1024 * 1024;
/// Decoder allocations allowed beside the pixel check: 16-bit RGBA at [`MAX_PIXELS`].
const MAX_ALLOC: u64 = MAX_PIXELS * 8;
/// ImageIO's default JPEG quality, 0.75.
const JPEG_QUALITY: u8 = 75;
/// JPEG XL starts with a bare codestream or an ISO BMFF signature box.
const JPEG_XL_CODESTREAM: &[u8] = b"\xff\x0a";
const JPEG_XL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";
/// `ftyp` brands of AV1-coded HEIF (AVIF): still, sequence, and intra-only.
const AVIF_BRANDS: [&[u8; 4]; 3] = [b"avif", b"avis", b"avio"];
/// JPEG 2000 starts with the JP2 signature box or a bare codestream's SOC and SIZ markers.
const JPEG_2000_FILE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";
const JPEG_2000_CODESTREAM: &[u8] = b"\xff\x4f\xff\x51";
/// The ID string of a JBIG2 file; embedded JBIG2 streams only occur inside PDF.
const JBIG2_FILE: &[u8] = b"\x97JB2\r\n\x1a\n";
/// Bytes per sample the JPEG 2000 decoder keeps for each component while decoding.
const JPEG_2000_SAMPLE_BYTES: u64 = 4;

/// Media types of the images that can be decoded, as Tika names them.
const READ_MEDIA_TYPES: &[&str] = &[
    "image/avif",
    "image/bmp",
    "image/gif",
    "image/heic",
    "image/heic-sequence",
    "image/heif",
    "image/heif-sequence",
    "image/jp2",
    "image/jpeg",
    "image/jxl",
    "image/png",
    "image/tiff",
    "image/vnd.wap.wbmp",
    "image/vnd.zbrush.pcx",
    "image/webp",
    "image/x-jbig2",
    "image/x-jp2-codestream",
    "image/x-portable-bitmap",
    "image/x-portable-graymap",
    "image/x-portable-pixmap",
];

/// Kotlin `ImageType`: the formats images are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
}

impl ImageType {
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
        }
    }
}

#[derive(Debug, Error)]
pub enum ImageError {
    /// The image needs more pixels or decoder memory than allowed.
    #[error("Image exceeds the decoding limits of {MAX_PIXELS} pixels and {MAX_ALLOC} bytes")]
    TooLarge,
    /// Unknown, malformed, or truncated image data.
    #[error("Cannot decode image: {0}")]
    Decode(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("Cannot encode image: {0}")]
    Encode(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Kotlin `ImageConverter.supportedReadMediaTypes`, for the formats decodes.
#[must_use]
pub fn can_read(media_type: &str) -> bool {
    READ_MEDIA_TYPES.contains(&media_type)
}

/// Kotlin `ImageConverter.convertImage`: the first frame of `bytes`, oriented, written as
/// `format`. Transparent pixels are laid on white for JPEG, which has no alpha.
pub fn convert(bytes: &[u8], format: ImageType) -> Result<Vec<u8>, ImageError> {
    encode(&decode(bytes)?, format)
}

/// The first frame of `bytes`, oriented.
pub(crate) fn decode(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    open(bytes)?.decode()
}

/// Kotlin `BookAnalyzer.hashPage`'s JPEG round trip: the pixels of a JPEG written again, so that
/// pages differing only in metadata such as EXIF get the same bytes. As with ImageIO, the EXIF
/// orientation is not applied.
pub fn rewrite_jpeg(bytes: &[u8]) -> Result<Vec<u8>, ImageError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    reader.limits(limits());
    let decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    check_pixels(width.into(), height.into())?;
    let image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    encode(&image, ImageType::Jpeg)
}

/// `width` by `height` 8-bit RGB pixels, row by row, written as `format`.
///
/// # Errors
/// Returns [`ImageError::Encode`] if `rgb` does not hold `width * height * 3` bytes or encoding
/// fails.
pub fn encode_rgb(
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    format: ImageType,
) -> Result<Vec<u8>, ImageError> {
    let image = RgbImage::from_raw(width, height, rgb).ok_or_else(|| {
        ImageError::Encode("RGB buffer length does not match image dimensions".into())
    })?;
    encode(&DynamicImage::ImageRgb8(image), format)
}

/// [`resize`] for a decoded `width` by `height` 8-bit RGB buffer.
///
/// # Errors
/// Returns [`ImageError::Encode`] if `rgb` does not hold `width * height * 3` bytes or encoding
/// fails.
pub fn resize_rgb(
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    format: ImageType,
    size: u32,
) -> Result<Vec<u8>, ImageError> {
    let image = RgbImage::from_raw(width, height, rgb).ok_or_else(|| {
        ImageError::Encode("RGB buffer length does not match image dimensions".into())
    })?;
    encode_fitting(DynamicImage::ImageRgb8(image), format, size)
}

/// Kotlin `ImageConverter.resizeImageToByteArray`: [`convert`], scaled down to fit a `size`
/// square with its aspect ratio kept and never scaled up. An image already in `format` and no
/// larger than `size` is returned as given.
pub fn resize(bytes: &[u8], format: ImageType, size: u32) -> Result<Vec<u8>, ImageError> {
    let opened = open(bytes)?;
    if let Opened::Header {
        format: Some(source),
        decoder,
        ..
    } = &opened
    {
        let (width, height) = decoder.dimensions();
        if *source == format.image_format() && width.max(height) <= size {
            return Ok(bytes.to_vec());
        }
    }
    encode_fitting(opened.decode()?, format, size)
}

/// `image` written as `format`, scaled down to fit a `size` square and never scaled up.
fn encode_fitting(
    mut image: DynamicImage,
    format: ImageType,
    size: u32,
) -> Result<Vec<u8>, ImageError> {
    if image.width().max(image.height()) > size {
        image = image.thumbnail(size, size);
    }
    encode(&image, format)
}

/// An image whose size was checked against [`MAX_PIXELS`] before its pixels were allocated.
enum Opened<'a> {
    /// Read up to its header.
    Header {
        /// The format, for images `image` decodes itself.
        format: Option<ImageFormat>,
        /// From EXIF for formats `image` decodes. JPEG XL orients while rendering; JPEG 2000 has
        /// no orientation.
        orientation: Orientation,
        decoder: Box<dyn ImageDecoder + 'a>,
    },
    /// Formats whose decoders return the whole image: HEIF and AVIF, transformed as the file
    /// says, the first page of JBIG2 files, PCX, and WBMP.
    Decoded(DynamicImage),
}

impl Opened<'_> {
    /// The first frame, oriented.
    fn decode(self) -> Result<DynamicImage, ImageError> {
        match self {
            Self::Header {
                orientation,
                decoder,
                ..
            } => {
                let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
                image.apply_orientation(orientation);
                Ok(image)
            }
            Self::Decoded(image) => Ok(image),
        }
    }
}

/// The decoder of `bytes`, chosen from content; images over [`MAX_PIXELS`] are refused from their
/// header, before pixels are allocated.
fn open(bytes: &[u8]) -> Result<Opened<'_>, ImageError> {
    if let Some(brands) = file_type_brands(bytes) {
        // The HEIF decoders return whole images: check the size the `ispe` property gives first.
        if let Ok(size) = imagesize::blob_size(bytes) {
            check_pixels(size.width as u64, size.height as u64)?;
        }
        let image = if is_avif(brands) {
            decode_avif(bytes)?
        } else {
            decode_hevc(bytes)?
        };
        return Ok(Opened::Decoded(image));
    }
    if bytes.starts_with(JBIG2_FILE) {
        return decode_jbig2(bytes).map(Opened::Decoded);
    }
    let decoder: Box<dyn ImageDecoder> =
        if bytes.starts_with(JPEG_XL_CODESTREAM) || bytes.starts_with(JPEG_XL_CONTAINER) {
            let mut decoder = JxlDecoder::new(Cursor::new(bytes)).map_err(decode_error)?;
            decoder.set_limits(limits()).map_err(decode_error)?;
            Box::new(decoder)
        } else if bytes.starts_with(JPEG_2000_FILE) || bytes.starts_with(JPEG_2000_CODESTREAM) {
            Box::new(open_jpeg_2000(bytes)?)
        } else {
            return open_other(bytes);
        };
    let (width, height) = decoder.dimensions();
    check_pixels(width.into(), height.into())?;
    Ok(Opened::Header {
        format: None,
        orientation: Orientation::NoTransforms,
        decoder,
    })
}

/// [`open`] for the formats `image` recognizes, then PCX and WBMP, which have no signature it
/// knows; WBMP has none at all.
fn open_other(bytes: &[u8]) -> Result<Opened<'_>, ImageError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .expect("Reading from memory cannot fail");
    let Some(format) = reader.format() else {
        if oxideav_pcx::probe(bytes) {
            return decode_pcx(bytes).map(Opened::Decoded);
        }
        if oxideav_wbmp::probe(bytes) {
            return decode_wbmp(bytes).map(Opened::Decoded);
        }
        return Err(ImageError::Decode("Unknown image format".into()));
    };
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    check_pixels(width.into(), height.into())?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    Ok(Opened::Header {
        format: Some(format),
        orientation,
        decoder: Box::new(decoder),
    })
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

fn check_pixels(width: u64, height: u64) -> Result<(), ImageError> {
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    Ok(())
}

/// A JPEG 2000 image read up to its header. Its decoder holds every component in 32 bits
/// outside `image`'s limits, so those samples are held to [`MAX_ALLOC`] as well.
fn open_jpeg_2000(bytes: &[u8]) -> Result<hayro_jpeg2000::Image<'_>, ImageError> {
    let image = hayro_jpeg2000::Image::new(bytes, &hayro_jpeg2000::DecodeSettings::default())
        .map_err(|error| ImageError::Decode(Box::new(error)))?;
    let pixels = u64::from(image.width()) * u64::from(image.height());
    let components = u64::from(image.color_space().num_channels()) + u64::from(image.has_alpha());
    if pixels.saturating_mul(components * JPEG_2000_SAMPLE_BYTES) > MAX_ALLOC {
        return Err(ImageError::TooLarge);
    }
    Ok(image)
}

/// The first page of a JBIG2 file, whose decoder borrows the parsed file. Strict, so that
/// truncated files fail as with the baseline's reader instead of yielding a partial page.
fn decode_jbig2(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let file = hayro_jbig2::File::new(bytes, &hayro_jbig2::DecodeSettings { strict: true })
        .map_err(|error| ImageError::Decode(Box::new(error)))?;
    let page = file
        .page(0)
        .ok_or_else(|| ImageError::Decode("JBIG2 file without pages".into()))?;
    check_pixels(page.width().into(), page.height().into())?;
    DynamicImage::from_decoder(page).map_err(decode_error)
}

/// The contents of the `ftyp` box `bytes` starts with, as ISO BMFF files such as HEIF do.
fn file_type_brands(bytes: &[u8]) -> Option<&[u8]> {
    let (size, rest) = bytes.split_first_chunk::<4>()?;
    let contents = rest.strip_prefix(b"ftyp")?;
    contents.get(
        ..usize::try_from(u32::from_be_bytes(*size))
            .ok()?
            .checked_sub(8)?,
    )
}

/// Whether `brands`, the contents of an `ftyp` box, name an AVIF brand.
fn is_avif(brands: &[u8]) -> bool {
    // The major brand, a 4-byte minor version, then the compatible brands.
    brands
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .any(|(index, brand)| index != 1 && AVIF_BRANDS.contains(&brand))
}

/// The primary image of an HEVC-coded HEIF file, with its crop, rotation, and mirror applied.
fn decode_hevc(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let options = heifer::Options {
        // Decode grid tiles on the calling pool thread.
        max_threads: 1,
        max_pixels: MAX_PIXELS,
    };
    let image = heifer::decode_rgba8(bytes, &options)
        .map_err(|error| ImageError::Decode(Box::new(error)))?;
    let (width, height) = (image.width, image.height);
    if image.has_alpha {
        rgba(width, height, image.data)
    } else {
        rgb(width, height, image.into_rgb8())
    }
}

/// The primary image of an AVIF file, or the first frame of a sequence, with its crop, rotation,
/// and mirror applied.
fn decode_avif(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let mut options = oxideav_avif::DecodeOptions::default();
    options.max_pixels = Some(MAX_PIXELS);
    let image = oxideav_avif::decode_with(bytes, &options).map_err(|error| match error {
        AvifError::LimitExceeded(_) => ImageError::TooLarge,
        error => ImageError::Decode(Box::new(error)),
    })?;
    if image.has_alpha() {
        rgba(image.width(), image.height(), image.to_rgba8())
    } else {
        rgb(image.width(), image.height(), image.to_rgb8())
    }
}

fn decode_pcx(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let mut options = oxideav_pcx::DecodeOptions::default();
    options.max_pixels = Some(MAX_PIXELS);
    let image = oxideav_pcx::decode_with(bytes, &options).map_err(|error| match error {
        PcxError::LimitExceeded(_) => ImageError::TooLarge,
        error => ImageError::Decode(Box::new(error)),
    })?;
    rgb(image.width(), image.height(), image.to_rgb8())
}

fn decode_wbmp(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let mut options = oxideav_wbmp::DecodeOptions::default();
    options.max_pixels = Some(MAX_PIXELS);
    let image = oxideav_wbmp::decode_with(bytes, &options).map_err(|error| match error {
        WbmpError::LimitExceeded(_) => ImageError::TooLarge,
        error => ImageError::Decode(Box::new(error)),
    })?;
    GrayImage::from_raw(image.width(), image.height(), image.to_gray8())
        .map(DynamicImage::ImageLuma8)
        .ok_or_else(|| ImageError::Decode("Decoder returned fewer samples than pixels".into()))
}

fn rgba(width: u32, height: u32, samples: Vec<u8>) -> Result<DynamicImage, ImageError> {
    RgbaImage::from_raw(width, height, samples)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| ImageError::Decode("Decoder returned fewer samples than pixels".into()))
}

fn rgb(width: u32, height: u32, samples: Vec<u8>) -> Result<DynamicImage, ImageError> {
    RgbImage::from_raw(width, height, samples)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| ImageError::Decode("Decoder returned fewer samples than pixels".into()))
}

fn decode_error(error: image::ImageError) -> ImageError {
    match error {
        image::ImageError::Limits(_) => ImageError::TooLarge,
        error => ImageError::Decode(Box::new(error)),
    }
}

pub(crate) fn encode(image: &DynamicImage, format: ImageType) -> Result<Vec<u8>, ImageError> {
    let mut bytes = Vec::new();
    let result = match format {
        ImageType::Png => image.write_with_encoder(PngEncoder::new(&mut bytes)),
        ImageType::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
            if image.has_alpha() {
                on_white(image).write_with_encoder(encoder)
            } else {
                image.write_with_encoder(encoder)
            }
        }
    };
    result.map_err(|error| ImageError::Encode(Box::new(error)))?;
    Ok(bytes)
}

/// `image` composited over white, in 8-bit RGB.
fn on_white(image: &DynamicImage) -> DynamicImage {
    let source = image.to_rgba8();
    let result = RgbImage::from_fn(source.width(), source.height(), |x, y| {
        let [red, green, blue, alpha] = source.get_pixel(x, y).0;
        Rgb([red, green, blue].map(|channel| over_white(channel, alpha)))
    });
    DynamicImage::ImageRgb8(result)
}

/// `channel` with opacity `alpha` composited over white.
fn over_white(channel: u8, alpha: u8) -> u8 {
    let (channel, alpha) = (u32::from(channel), u32::from(alpha));
    let blended = (channel * alpha + 255 * (255 - alpha) + 127) / 255;
    u8::try_from(blended).expect("A blend of two bytes fits a byte")
}

#[cfg(test)]
mod tests;
