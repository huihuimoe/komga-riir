//! Book file digests: Kotlin `Hasher` and `KoreaderHasher`. Neither is an image perceptual hash.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use md5::{Digest, Md5};
use xxhash_rust::xxh3::Xxh3;

/// Kotlin `Hasher`: XXH3-128 with seed 0, as the lowercase hex of its canonical big-endian bytes.
#[derive(Clone, Default)]
pub struct FileDigest(Xxh3);

impl FileDigest {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    #[must_use]
    pub fn finish(&self) -> String {
        hex::encode(self.0.digest128().to_be_bytes())
    }
}

/// Kotlin `KoreaderHasher`, after KOReader's `util.partialMD5`: the MD5 of up to 1 KiB read at
/// offsets `1024 << 2i` for `i` in `-1..=10`. Offsets past the end add nothing.
///
/// # Errors
/// When the file cannot be opened or read.
pub fn koreader_hash(path: &Path) -> io::Result<String> {
    const STEP: u64 = 1024;
    let mut file = File::open(path)?;
    let mut digest = Md5::new();
    for i in -1_i32..=10 {
        // The JVM masks the shift distance to 6 bits, so `i = -1` shifts by 62 and reads at 0.
        let offset = STEP.wrapping_shl((2 * i).cast_unsigned());
        file.seek(SeekFrom::Start(offset))?;
        let mut chunk = Vec::with_capacity(1024);
        (&mut file).take(1024).read_to_end(&mut chunk)?;
        digest.update(&chunk);
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{FileDigest, koreader_hash};

    /// `(size, Hasher, KoreaderHasher)` from Kotlin's `com.appmattus.crypto:cryptohash:1.0.2` over
    /// files whose byte `i` is `i % 251`.
    const KOTLIN_SAMPLES: [(usize, &str, &str); 4] = [
        (
            0,
            "99aa06d3014798d86001c324468d497f",
            "d41d8cd98f00b204e9800998ecf8427e",
        ),
        (
            5,
            "9434532106a7c141c920d2347a85929b",
            "d05374dc381d9b52806446a71c8e79b1",
        ),
        (
            1500,
            "01fac3ed11d8c0b9696f4e652da3acf2",
            "a5fcf6debbdcc01aaa4dc9dfb5c8ae5a",
        ),
        (
            20 * 1024 * 1024,
            "c82335045a1d11ec85c584583bfd02a9",
            "616e298af0d37b61a8805ac06bd9284a",
        ),
    ];

    fn sample(size: usize) -> Vec<u8> {
        (0..size)
            .map(|i| u8::try_from(i % 251).expect("below 251"))
            .collect()
    }

    #[test]
    fn digests_match_kotlin_samples() {
        let dir = komga_test_support::tempdir().expect("temp dir");
        for (size, file_hash, koreader) in KOTLIN_SAMPLES {
            let bytes = sample(size);
            // Chunk boundaries must not change the digest.
            let mut digest = FileDigest::new();
            for chunk in bytes.chunks(7000) {
                digest.update(chunk);
            }
            assert_eq!(digest.finish(), file_hash, "file hash of {size} bytes");

            let path = dir.path().join(format!("sample-{size}"));
            fs::write(&path, &bytes).expect("write sample");
            assert_eq!(
                koreader_hash(&path).expect("hash sample"),
                koreader,
                "KOReader hash of {size} bytes"
            );
        }
    }

    #[test]
    fn koreader_hash_of_a_missing_file_fails() {
        let dir = komga_test_support::tempdir().expect("temp dir");
        assert!(koreader_hash(&dir.path().join("missing")).is_err());
    }
}
