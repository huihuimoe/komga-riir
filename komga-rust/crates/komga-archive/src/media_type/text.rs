//! Tika `TextDetector` and `TextStatistics`: content without magic is plain text when it is
//! mostly ASCII or looks like UTF-8, with few control bytes.

/// Control bytes common in text: tab, line feed, form feed, carriage return, escape.
const SAFE_CONTROL: [u8; 5] = [0x09, 0x0a, 0x0c, 0x0d, 0x1b];

pub(super) fn looks_like_text(data: &[u8]) -> bool {
    let mut counts = [0_usize; 256];
    for &byte in data {
        counts[usize::from(byte)] += 1;
    }
    let count = |range: std::ops::Range<usize>| -> usize { counts[range].iter().sum() };
    let total = data.len();
    let control = count(0..0x20);
    let safe = SAFE_CONTROL
        .iter()
        .map(|&byte| counts[usize::from(byte)])
        .sum::<usize>();
    let unsafe_control = control - safe;

    let mostly_ascii = total > 0
        && unsafe_control * 100 < total * 2
        && (count(0x20..0x80) + safe) * 100 > total * 90;

    let leading = [count(0xc0..0xe0), count(0xe0..0xf0), count(0xf0..0xf8)];
    let utf8 = count(0x20..0x80) + leading.iter().sum::<usize>();
    let expected_continuation: usize = leading
        .iter()
        .enumerate()
        .map(|(index, count)| (index + 1) * count)
        .sum();
    let continuation = count(0x80..0xc0);
    let looks_like_utf8 = utf8 > 0
        && continuation <= expected_continuation
        && continuation + 3 >= expected_continuation
        && count(0xf8..0x100) == 0
        && unsafe_control * 100 < utf8 * 2;

    mostly_ascii || looks_like_utf8
}
