//! commons-compress `ExplodingInputStream` (with `BinaryTree` and `CircularBuffer`): PKZIP's
//! IMPLODING, with a 4 or 8 KiB dictionary and 2 or 3 Shannon-Fano trees.
//!
//! The data has no end marker; decoding stops where the input runs out, so padding or garbage
//! bits may still decode into output, and copies may reach before the start (zeros). Malformed
//! input is decoded as commons-compress does.

use std::io::{self, BufRead, Read};

use super::bit_reader::BitReader;

/// Holds the dictionary and the unread output of one step.
const WINDOW_SIZE: usize = 32 * 1024;
const UNDEFINED: i16 = -1;
const NODE: i16 = -2;

/// `ExplodingInputStream`, read like Java's default `InputStream.read(byte[], int, int)` over
/// its `read()`: an error after the first byte of a call ends that call early and is dropped,
/// and the next call resumes decoding from the current bit. So where damaged data ends, and
/// whether it fails, depends on the sizes of the reads, as it does in Komga.
pub(super) struct Explode<R> {
    bits: BitReader<R>,
    large_dictionary: bool,
    three_trees: bool,
    trees: Option<Trees>,
    window: Vec<u8>,
    read_index: usize,
    write_index: usize,
}

struct Trees {
    literal: Option<Tree>,
    length: Tree,
    distance: Tree,
}

impl<R: BufRead> Explode<R> {
    pub(super) fn new(input: R, large_dictionary: bool, three_trees: bool) -> Self {
        Self {
            bits: BitReader::new(input),
            large_dictionary,
            three_trees,
            trees: None,
            window: vec![0; WINDOW_SIZE],
            read_index: 0,
            write_index: 0,
        }
    }

    /// `fillBuffer`: decodes one literal or copy, or nothing at the end of the input.
    fn fill(&mut self) -> io::Result<()> {
        if self.trees.is_none() {
            let input = self.bits.get_mut();
            let literal = if self.three_trees {
                Some(Tree::decode(input, 256)?)
            } else {
                None
            };
            let length = Tree::decode(input, 64)?;
            let distance = Tree::decode(input, 64)?;
            self.trees = Some(Trees {
                literal,
                length,
                distance,
            });
        }
        let trees = self.trees.as_ref().expect("Decoded above");
        let bits = &mut self.bits;
        let Some(flag) = bits.read_bits(1)? else {
            return Ok(());
        };
        if flag == 1 {
            let literal = match &trees.literal {
                Some(tree) => tree.read(bits)?,
                None => bits.read_bits(8)?,
            };
            if let Some(literal) = literal {
                self.window[self.write_index] = literal as u8;
                self.write_index = (self.write_index + 1) % WINDOW_SIZE;
            }
            return Ok(());
        }
        // Java ints: a part past the end is -1 and still combines into the distance.
        let low_bits = if self.large_dictionary { 7 } else { 6 };
        let distance_low = bits.read_bits(low_bits)?.map_or(-1, i32::from);
        let distance_high = trees.distance.read(bits)?.map_or(-1, i32::from);
        if distance_high == -1 && distance_low <= 0 {
            return Ok(());
        }
        let distance = distance_high << low_bits | distance_low;
        let mut length = trees.length.read(bits)?.map_or(-1, i32::from);
        if length == 63 {
            let Some(extra) = bits.read_bits(8)? else {
                return Ok(());
            };
            length += i32::from(extra);
        }
        let minimum_length = if self.three_trees { 3 } else { 2 };
        self.copy(distance + 1, length + minimum_length);
        Ok(())
    }

    /// `CircularBuffer.copy`: `distance` may be 0 or negative after a truncated distance.
    fn copy(&mut self, distance: i32, length: i32) {
        let size = WINDOW_SIZE as i32;
        let start = self.write_index as i32 - distance;
        for source in start..start + length {
            let source = usize::try_from((source + size) % size).expect("Within one lap back");
            self.window[self.write_index] = self.window[source];
            self.write_index = (self.write_index + 1) % WINDOW_SIZE;
        }
    }

    /// `ExplodingInputStream.read()`.
    fn read_byte(&mut self) -> io::Result<Option<u8>> {
        if self.read_index == self.write_index {
            self.fill()?;
            if self.read_index == self.write_index {
                return Ok(None);
            }
        }
        let byte = self.window[self.read_index];
        self.read_index = (self.read_index + 1) % WINDOW_SIZE;
        Ok(Some(byte))
    }
}

impl<R: BufRead> Read for Explode<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some((first, rest)) = buffer.split_first_mut() else {
            return Ok(0);
        };
        let Some(byte) = self.read_byte()? else {
            return Ok(0);
        };
        *first = byte;
        let mut count = 1;
        for slot in rest {
            let Ok(Some(byte)) = self.read_byte() else {
                break;
            };
            *slot = byte;
            count += 1;
        }
        Ok(count)
    }
}

/// `BinaryTree`: node `i` has children `2i + 1` (bit 0) and `2i + 2` (bit 1).
struct Tree {
    nodes: Vec<i16>,
}

impl Tree {
    /// `BinaryTree.decode`: a byte holding the count of the following bytes minus one, each
    /// giving a run of values (high nibble + 1) with one bit length (low nibble + 1). Values
    /// past the runs get no code.
    fn decode(input: &mut impl BufRead, values: usize) -> io::Result<Self> {
        let mut size = [0];
        if input.read(&mut size)? == 0 {
            return Err(invalid(
                "Cannot read the size of the encoded tree, unexpected end of stream",
            ));
        }
        let size = usize::from(size[0]) + 1;
        let mut encoded = Vec::with_capacity(size);
        input.take(size as u64).read_to_end(&mut encoded)?;
        if encoded.len() != size {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let mut lengths = vec![0; values];
        let mut position = 0;
        for byte in encoded {
            let count = usize::from(byte >> 4) + 1;
            let Some(run) = lengths.get_mut(position..position + count) else {
                return Err(invalid(
                    "Number of values exceeds given total number of values",
                ));
            };
            run.fill(u32::from(byte & 0x0f) + 1);
            position += count;
        }
        // Codes count up from the longest bit length, in value order within one length.
        let mut order: Vec<usize> = (0..values).collect();
        order.sort_by_key(|&value| lengths[value]);
        let mut codes = vec![0_u32; values];
        let (mut code, mut increment, mut last_length) = (0_u32, 0, 0);
        for &value in order.iter().rev() {
            code += increment;
            if lengths[value] != last_length {
                last_length = lengths[value];
                increment = 1 << (16 - last_length);
            }
            codes[value] = code;
        }
        let depth = lengths.iter().max().copied().unwrap_or_default();
        let mut tree = Self {
            nodes: vec![UNDEFINED; (1 << (depth + 1)) - 1],
        };
        for (value, (&length, &code)) in lengths.iter().zip(&codes).enumerate() {
            if length > 0 {
                // The first bit read is the most significant bit of the 16-bit code.
                tree.add_leaf((code << 16).reverse_bits(), length, value)?;
            }
        }
        Ok(tree)
    }

    /// `BinaryTree.addLeaf`: marks the nodes on the path, overwriting leaves, but fails on a
    /// value already at the end.
    fn add_leaf(&mut self, mut path: u32, length: u32, value: usize) -> io::Result<()> {
        let mut node = 0;
        for _ in 0..length {
            self.nodes[node] = NODE;
            node = 2 * node + 1 + (path & 1) as usize;
            path >>= 1;
        }
        if self.nodes[node] != UNDEFINED {
            return Err(invalid(&format!(
                "Tree value at index {node} has already been assigned ({})",
                self.nodes[node]
            )));
        }
        self.nodes[node] = i16::try_from(value).expect("At most 256 values");
        Ok(())
    }

    /// `BinaryTree.read`: the value, or `None` once the input runs out.
    fn read(&self, bits: &mut BitReader<impl BufRead>) -> io::Result<Option<u16>> {
        let mut node = 0;
        loop {
            let Some(bit) = bits.read_bits(1)? else {
                return Ok(None);
            };
            let child = 2 * node + 1 + usize::from(bit);
            match self.nodes[child] {
                NODE => node = child,
                UNDEFINED => {
                    return Err(invalid(&format!(
                        "The child {bit} of node at index {node} is not defined"
                    )));
                }
                value => return Ok(Some(value as u16)),
            }
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
