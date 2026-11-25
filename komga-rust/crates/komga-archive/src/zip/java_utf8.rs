//! JDK `UTF_8.Decoder` with commons-compress's `?` replacement: entry names decode the way
//! Komga's `ZipFile` decodes them, which differs from `String::from_utf8_lossy` in how many
//! bytes one replacement covers (an encoded surrogate is one malformed sequence, for example).

const REPLACEMENT: char = '?';

pub(super) fn decode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        let (char, length) = match decode_one(rest) {
            Some(Decoded::Char(char, length)) => (char, length),
            Some(Decoded::Malformed(length)) => (REPLACEMENT, length),
            // A sequence cut short by the end of input is one malformed sequence.
            None => (REPLACEMENT, rest.len()),
        };
        text.push(char);
        index += length;
    }
    text
}

enum Decoded {
    Char(char, usize),
    Malformed(usize),
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

/// `decodeArrayLoop` for the sequence `bytes` starts with; `None` when it needs more input.
fn decode_one(bytes: &[u8]) -> Option<Decoded> {
    let first = bytes[0];
    let decoded = match first {
        0x00..=0x7f => Decoded::Char(char::from(first), 1),
        0xc2..=0xdf => {
            let second = *bytes.get(1)?;
            if is_continuation(second) {
                let value = (u32::from(first & 0x1f) << 6) | u32::from(second & 0x3f);
                Decoded::Char(char::from_u32(value).expect("Two bytes decode a scalar"), 2)
            } else {
                Decoded::Malformed(1)
            }
        }
        0xe0..=0xef => decode_three(bytes)?,
        0xf0..=0xf7 => decode_four(bytes)?,
        _ => Decoded::Malformed(1),
    };
    Some(decoded)
}

fn decode_three(bytes: &[u8]) -> Option<Decoded> {
    let first = bytes[0];
    let bad_second =
        |second: u8| (first == 0xe0 && second & 0xe0 == 0x80) || !is_continuation(second);
    let [_, second, third, ..] = *bytes else {
        return match bytes.get(1) {
            Some(&second) if bad_second(second) => Some(Decoded::Malformed(1)),
            _ => None,
        };
    };
    if bad_second(second) {
        return Some(Decoded::Malformed(1));
    }
    if !is_continuation(third) {
        return Some(Decoded::Malformed(2));
    }
    let value =
        (u32::from(first & 0x0f) << 12) | (u32::from(second & 0x3f) << 6) | u32::from(third & 0x3f);
    // An encoded surrogate is malformed as a whole.
    Some(char::from_u32(value).map_or(Decoded::Malformed(3), |char| Decoded::Char(char, 3)))
}

fn decode_four(bytes: &[u8]) -> Option<Decoded> {
    let first = bytes[0];
    let bad_second = |second: u8| {
        (first == 0xf0 && !(0x90..=0xbf).contains(&second))
            || (first == 0xf4 && second & 0xf0 != 0x80)
            || !is_continuation(second)
    };
    let [_, second, third, fourth, ..] = *bytes else {
        if first > 0xf4 || bytes.get(1).is_some_and(|&second| bad_second(second)) {
            return Some(Decoded::Malformed(1));
        }
        if bytes.get(2).is_some_and(|&third| !is_continuation(third)) {
            return Some(Decoded::Malformed(2));
        }
        return None;
    };
    let value = (u32::from(first & 0x07) << 18)
        | (u32::from(second & 0x3f) << 12)
        | (u32::from(third & 0x3f) << 6)
        | u32::from(fourth & 0x3f);
    let valid = is_continuation(second)
        && is_continuation(third)
        && is_continuation(fourth)
        && (0x1_0000..=0x10_ffff).contains(&value);
    if valid {
        return Some(Decoded::Char(
            char::from_u32(value).expect("Supplementary scalar"),
            4,
        ));
    }
    // `malformedN(4)`: how much of the start is still a plausible prefix.
    let length = if first > 0xf4 || bad_second(second) {
        1
    } else if !is_continuation(third) {
        2
    } else {
        3
    };
    Some(Decoded::Malformed(length))
}
