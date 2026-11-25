//! Image analysis and conversion for book pages, and posters made of several images.

mod barcode;
mod converter;
mod dimension;
mod mosaic;

pub use barcode::{read_ean13, read_ean13_rgb};
pub use converter::{
    ImageError, ImageType, MAX_PIXELS, can_read, convert, encode_rgb, resize, resize_rgb,
    rewrite_jpeg,
};
pub use dimension::read_dimension;
pub use mosaic::mosaic;
