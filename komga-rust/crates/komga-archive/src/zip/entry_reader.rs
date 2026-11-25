//! commons-compress `ZipFile.getInputStream`: the entry data decoded by its method.

use std::io::{self, BufRead, BufReader, Read};

use bzip2::read::BzDecoder;
use deflate64::InflaterManaged;
use flate2::{Decompress, FlushDecompress, Status};

use super::explode::Explode;
use super::unshrink::Unshrink;
use super::{Entry, ZipError};

const STORED: u16 = 0;
const UNSHRINKING: u16 = 1;
const IMPLODING: u16 = 6;
const DEFLATED: u16 = 8;
const ENHANCED_DEFLATED: u16 = 9;
const BZIP2: u16 = 12;
const ZSTD_DEPRECATED: u16 = 20;
const ZSTD: u16 = 93;
const XZ: u16 = 95;
const ENCRYPTED_FLAG: u16 = 1;
const LARGE_DICTIONARY_FLAG: u16 = 1 << 1;
const THREE_TREES_FLAG: u16 = 1 << 2;

/// `ZipUtil.checkRequestedFeatures`, then the decoder for the method over the entry's data.
pub(super) fn decode<'a>(
    entry: &Entry,
    data: impl Read + 'a,
) -> Result<Box<dyn Read + 'a>, ZipError> {
    if entry.flags & ENCRYPTED_FLAG != 0 {
        return Err(unsupported(entry, "encryption"));
    }
    let reader: Box<dyn Read> = match entry.method {
        STORED => Box::new(data),
        DEFLATED => Box::new(Inflate {
            input: BufReader::new(data),
            engine: Engine::Deflate(Decompress::new(false)),
            finished: false,
        }),
        ENHANCED_DEFLATED => Box::new(Inflate {
            input: BufReader::new(data),
            engine: Engine::Deflate64(Box::new(InflaterManaged::new())),
            finished: false,
        }),
        // A single stream: commons-compress stops at its end.
        BZIP2 => Box::new(BzDecoder::new(data)),
        UNSHRINKING => Box::new(Unshrink::new(BufReader::new(data))),
        IMPLODING => Box::new(Explode::new(
            BufReader::new(data),
            entry.flags & LARGE_DICTIONARY_FLAG != 0,
            entry.flags & THREE_TREES_FLAG != 0,
        )),
        ZSTD_DEPRECATED | ZSTD | XZ => return Err(ZipError::MissingCodec(entry.method)),
        method => {
            return Err(unsupported(entry, &format!("compression method {method}")));
        }
    };
    Ok(reader)
}

fn unsupported(entry: &Entry, feature: &str) -> ZipError {
    ZipError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("Unsupported feature {feature} used in entry {}", entry.name),
    ))
}

enum Engine {
    Deflate(Decompress),
    Deflate64(Box<InflaterManaged>),
}

/// Java `InflaterInputStream` and commons-compress `Deflate64CompressorInputStream`: data
/// ends with the deflate stream, and input that ends first is an error.
struct Inflate<R> {
    input: R,
    engine: Engine,
    finished: bool,
}

impl<R: BufRead> Read for Inflate<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while !self.finished && !buffer.is_empty() {
            let input = self.input.fill_buf()?;
            let exhausted = input.is_empty();
            let (consumed, written) = match &mut self.engine {
                Engine::Deflate(decompress) => {
                    let (before_in, before_out) = (decompress.total_in(), decompress.total_out());
                    let status = decompress
                        .decompress(input, buffer, FlushDecompress::None)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    self.finished = status == Status::StreamEnd;
                    (
                        usize::try_from(decompress.total_in() - before_in).expect("Within input"),
                        usize::try_from(decompress.total_out() - before_out)
                            .expect("Within buffer"),
                    )
                }
                Engine::Deflate64(inflater) => {
                    let result = inflater.inflate(input, buffer);
                    if result.data_error {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Invalid Deflate64 input",
                        ));
                    }
                    self.finished = inflater.finished();
                    (result.bytes_consumed, result.bytes_written)
                }
            };
            self.input.consume(consumed);
            if written > 0 {
                return Ok(written);
            }
            if exhausted && !self.finished {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Unexpected end of deflate input",
                ));
            }
        }
        Ok(0)
    }
}
