use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::LazyLock;

use super::{REGISTRY, detect, is_image, read_header};

/// `expected.tsv` holds what Tika 3.3.2 (`new TikaConfig().getDetector()`) reports for each
/// fixture, both for a stream and for `TikaInputStream.get(path)` with Komga's metadata.
#[test]
fn detection_matches_tika_on_real_files() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media_types");
    let expected = fs::read_to_string(directory.join("expected.tsv")).expect("Expected types");
    let mut mismatches = Vec::new();
    for line in expected.lines() {
        let (name, media_type) = line.split_once('\t').expect("Name and type");
        let file = File::open(directory.join(name)).expect("Fixture");
        let detected = detect(file).expect("Read fixture");
        if detected != media_type {
            mismatches.push(format!(
                "{name}: expected {media_type}, detected {detected}"
            ));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    assert_eq!(
        expected.lines().count() + 1,
        fs::read_dir(&directory).expect("Fixtures").count()
    );
}

#[test]
fn only_the_first_64_kib_counts() {
    // Text for the whole header, then control bytes: 6% of a header makes it binary.
    let mut content = vec![b'a'; 64 * 1024];
    content.extend([0; 4 * 1024]);
    assert_eq!(detect(content.as_slice()).expect("Detect"), "text/plain");
    assert_eq!(
        detect(&content[4 * 1024..]).expect("Detect"),
        "application/octet-stream"
    );
}

/// Records the size of each read and returns at most `limit` bytes per read.
struct Recorder {
    data: Vec<u8>,
    position: usize,
    limit: usize,
    requests: Vec<usize>,
}

impl Read for Recorder {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.requests.push(buffer.len());
        let count = buffer
            .len()
            .min(self.limit)
            .min(self.data.len() - self.position);
        buffer[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

#[test]
fn the_header_is_read_like_a_marked_java_buffered_stream() {
    // Requests a recording stream gets from Tika 3.3.2 `detect` over a JDK 21
    // `BufferedInputStream`.
    for (length, limit, requests) in [
        (100_000, usize::MAX, vec![8192, 8192, 16384, 32768]),
        (
            100_000,
            6000,
            vec![
                8192, 2192, 8192, 2192, 16384, 10384, 4384, 32768, 26768, 20768, 14768, 8768, 2768,
            ],
        ),
        (10_000, usize::MAX, vec![8192, 8192, 6384]),
    ] {
        let data: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
        let mut recorder = Recorder {
            data: data.clone(),
            position: 0,
            limit,
            requests: Vec::new(),
        };
        let header = read_header(&mut recorder).expect("Read header");
        assert_eq!(header, data[..length.min(64 * 1024)]);
        assert_eq!(
            recorder.requests, requests,
            "{length} bytes, {limit} per read"
        );
    }
}

#[test]
fn other_declared_encodings_keep_the_root() {
    for (encoding, text) in [
        ("Shift_JIS", &b"\x93\xfa\x96\x7b"[..]),
        ("ISO-8859-1", b"\xe9"),
    ] {
        let content = [
            format!("<?xml version=\"1.0\" encoding=\"{encoding}\"?><!--").as_bytes(),
            text,
            b"--><html xmlns=\"http://www.w3.org/1999/xhtml\"><title>",
            text,
            b"</title></html>",
        ]
        .concat();
        assert_eq!(
            detect(content.as_slice()).expect("Detect"),
            "application/xhtml+xml",
            "{encoding}"
        );
    }
}

#[test]
fn the_registry_loads_every_type_and_regex() {
    // Loading compiles every `regex` match through the translation, or panics.
    LazyLock::force(&REGISTRY);
    assert!(is_image("image/webp"));
    assert!(!is_image("application/xml"));
}

/// `registry_samples.tsv` holds hex content built from the registry's own `<match>` patterns
/// (each match with its enclosing ones, at the first and last offset of its range, some cut
/// one byte short), with what Tika 3.3.2 reports for it.
#[test]
fn detection_matches_tika_on_registry_patterns() {
    let samples = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry_samples.tsv"),
    )
    .expect("Samples");
    let mut mismatches = Vec::new();
    for line in samples.lines() {
        let (hex, media_type) = line.split_once('\t').expect("Content and type");
        let content: Vec<u8> = (0..hex.len() / 2)
            .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("Hex"))
            .collect();
        let detected = detect(content.as_slice()).expect("Detect");
        if detected != media_type {
            mismatches.push(format!("{hex}: expected {media_type}, detected {detected}"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// Tika 3.3.2 `forName(..).getExtension()`; the whole registry (1,834 names and aliases) matched
/// during development.
#[test]
fn extensions_match_tika() {
    for (name, expected) in [
        ("image/jpeg", Some(".jpg")),
        ("image/png", Some(".png")),
        ("image/webp", Some(".webp")),
        ("image/heic", Some(".heic")),
        ("image/tiff", Some(".tiff")),
        ("image/x-portable-pixmap", Some(".ppm")),
        // An alias takes its type's extension.
        ("image/x-ms-bmp", Some(".bmp")),
        ("application/activemessage", Some("")),
        ("image/jpg", None),
        ("bogus", None),
    ] {
        assert_eq!(super::extension(name), expected, "{name}");
    }
}
