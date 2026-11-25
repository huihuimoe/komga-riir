//! commons-compress `UnshrinkingInputStream` (with `LZWInputStream`): PKZIP's LZW with 9 to 13
//! bit codes, where code 256 is followed by 1 (grow the code size) or 2 (partial clear).
//!
//! Malformed input is decoded as commons-compress does, including a partial clear that keeps
//! the previous code: the next entry may then point at a freed code or at itself. An expansion
//! that runs past the output stack is Java's `ArrayIndexOutOfBoundsException`, reported here as
//! invalid data.

use std::io::{self, BufRead, Read};

use super::bit_reader::BitReader;

const MAX_CODE_SIZE: u32 = 13;
const TABLE_SIZE: usize = 1 << MAX_CODE_SIZE;
const CLEAR_CODE: usize = 256;
const UNUSED: i16 = -1;

pub(super) struct Unshrink<R> {
    bits: BitReader<R>,
    code_size: u32,
    /// The next code to try; codes in use are skipped.
    table_size: usize,
    prefixes: Vec<i16>,
    characters: Vec<u8>,
    used: Vec<bool>,
    previous_code: Option<usize>,
    previous_first: u8,
    /// Expanded codes are written backwards, ending at `TABLE_SIZE`.
    stack: Vec<u8>,
    stack_position: usize,
}

impl<R: BufRead> Unshrink<R> {
    pub(super) fn new(input: R) -> Self {
        let mut used = vec![false; TABLE_SIZE];
        used[..256].fill(true);
        let mut characters: Vec<u8> = (0..=u8::MAX).collect();
        characters.resize(TABLE_SIZE, 0);
        Self {
            bits: BitReader::new(input),
            code_size: 9,
            table_size: CLEAR_CODE + 1,
            prefixes: vec![UNUSED; TABLE_SIZE],
            characters,
            used,
            previous_code: None,
            previous_first: 0,
            stack: vec![0; TABLE_SIZE],
            stack_position: TABLE_SIZE,
        }
    }

    fn read_code(&mut self) -> io::Result<Option<usize>> {
        Ok(self.bits.read_bits(self.code_size)?.map(usize::from))
    }

    /// `addEntry`: the first unused code from `table_size` on, or `None` once the table is full.
    fn add_entry(&mut self, prefix: usize, character: u8) -> Option<usize> {
        while self.table_size < TABLE_SIZE && self.used[self.table_size] {
            self.table_size += 1;
        }
        if self.table_size == TABLE_SIZE {
            return None;
        }
        let code = self.table_size;
        self.prefixes[code] = to_entry(prefix);
        self.characters[code] = character;
        self.used[code] = true;
        self.table_size += 1;
        Some(code)
    }

    /// `decompressNextSymbol`: expands the next code onto the stack, or handles a control code
    /// and leaves the stack empty. `false` at the end of the input.
    fn decompress_next_symbol(&mut self) -> io::Result<bool> {
        let Some(code) = self.read_code()? else {
            return Ok(false);
        };
        if code != CLEAR_CODE {
            if self.used[code] {
                self.expand(Some(code), /* added_unfinished_entry= */ false)?;
            } else {
                let Some(previous) = self.previous_code else {
                    return Err(invalid(
                        "The first code can't be a reference to its preceding code",
                    ));
                };
                let code = self.add_entry(previous, self.previous_first);
                self.expand(code, /* added_unfinished_entry= */ true)?;
            }
            return Ok(true);
        }
        match self.read_code()? {
            None => return Err(invalid("Unexpected EOF;")),
            Some(1) if self.code_size >= MAX_CODE_SIZE => {
                return Err(invalid("Attempt to increase code size beyond maximum"));
            }
            Some(1) => self.code_size += 1,
            Some(2) => {
                self.partial_clear();
                self.table_size = CLEAR_CODE + 1;
            }
            Some(subcode) => return Err(invalid(&format!("Invalid clear code subcode {subcode}"))),
        }
        Ok(true)
    }

    /// `expandCodeToOutputStack`; `None` is the -1 of a full table.
    fn expand(&mut self, code: Option<usize>, added_unfinished_entry: bool) -> io::Result<()> {
        let mut entry = code.map_or(UNUSED, to_entry);
        while let Ok(index) = usize::try_from(entry) {
            if self.stack_position == 0 {
                return Err(invalid("LZW expansion overflows the output stack"));
            }
            self.stack_position -= 1;
            self.stack[self.stack_position] = self.characters[index];
            entry = self.prefixes[index];
        }
        let Some(&first) = self.stack.get(self.stack_position) else {
            return Err(invalid("LZW expansion of a full table"));
        };
        if let Some(previous) = self.previous_code
            && !added_unfinished_entry
        {
            self.add_entry(previous, first);
        }
        self.previous_code = code;
        self.previous_first = first;
        Ok(())
    }

    /// Frees every code past the clear code that no code in use extends.
    fn partial_clear(&mut self) {
        let mut parent = vec![false; TABLE_SIZE];
        for (&used, &prefix) in self.used.iter().zip(&self.prefixes) {
            if let (true, Ok(prefix)) = (used, usize::try_from(prefix)) {
                parent[prefix] = true;
            }
        }
        let codes = self.used.iter_mut().zip(&mut self.prefixes).zip(parent);
        for ((used, prefix), parent) in codes.skip(CLEAR_CODE + 1) {
            if !parent {
                *used = false;
                *prefix = UNUSED;
            }
        }
    }

    fn read_from_stack(&mut self, buffer: &mut [u8]) -> usize {
        let count = buffer.len().min(TABLE_SIZE - self.stack_position);
        buffer[..count].copy_from_slice(&self.stack[self.stack_position..][..count]);
        self.stack_position += count;
        count
    }
}

impl<R: BufRead> Read for Unshrink<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut count = self.read_from_stack(buffer);
        while count < buffer.len() {
            if !self.decompress_next_symbol()? {
                break;
            }
            count += self.read_from_stack(&mut buffer[count..]);
        }
        Ok(count)
    }
}

fn to_entry(code: usize) -> i16 {
    i16::try_from(code).expect("Codes are below TABLE_SIZE")
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
