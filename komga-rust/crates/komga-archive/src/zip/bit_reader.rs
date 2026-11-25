//! commons-compress `BitInputStream` in little-endian order, for codes of at most 16 bits.

use std::io::{self, BufRead};

pub(super) struct BitReader<R> {
    input: R,
    cache: u32,
    cached: u32,
}

impl<R: BufRead> BitReader<R> {
    pub(super) fn new(input: R) -> Self {
        Self {
            input,
            cache: 0,
            cached: 0,
        }
    }

    /// The input, which is byte aligned while nothing is cached.
    pub(super) fn get_mut(&mut self) -> &mut R {
        &mut self.input
    }

    /// The next `count` bits, least significant first. Past the end of the input this is `None`
    /// and the cached bits stay, so a later read of fewer bits can still succeed.
    pub(super) fn read_bits(&mut self, count: u32) -> io::Result<Option<u16>> {
        debug_assert!(count <= 16, "Codes are at most 16 bits, got {count}");
        while self.cached < count {
            let Some(&byte) = self.input.fill_buf()?.first() else {
                return Ok(None);
            };
            self.input.consume(1);
            self.cache |= u32::from(byte) << self.cached;
            self.cached += 8;
        }
        let bits = self.cache & ((1 << count) - 1);
        self.cache >>= count;
        self.cached -= count;
        Ok(Some(u16::try_from(bits).expect("At most 16 bits")))
    }
}
