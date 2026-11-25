//! natural-comparator 1.1 `CaseInsensitiveSimpleNaturalComparator`, which Komga orders book
//! names and archive entries with.

use std::cmp::Ordering;

use icu_casemap::CaseMapperBorrowed;

/// `AbstractSimpleNaturalComparator.compare` over UTF-16 units: digit runs compare as unsigned
/// 64-bit numbers (overflow wraps), other units by `Character.toLowerCase`, and digits sort before
/// other units.
pub(crate) fn compare(left: &str, right: &str) -> Ordering {
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        let (left_unit, right_unit) = (left[left_index], right[right_index]);
        match (digit(left_unit), digit(right_unit)) {
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            (None, None) => {
                let ordering = lowercase(left_unit).cmp(&lowercase(right_unit));
                if ordering != Ordering::Equal {
                    return ordering;
                }
                left_index += 1;
                right_index += 1;
            }
            (Some(_), Some(_)) => {
                let left_number = number(&left, &mut left_index);
                let right_number = number(&right, &mut right_index);
                if left_number != right_number {
                    return left_number.cmp(&right_number);
                }
            }
        }
    }
    // Equal so far: the sequence with units left sorts after.
    (left_index < left.len()).cmp(&(right_index < right.len()))
}

fn digit(unit: u16) -> Option<u64> {
    (u16::from(b'0')..=u16::from(b'9'))
        .contains(&unit)
        .then(|| u64::from(unit - u16::from(b'0')))
}

fn number(units: &[u16], index: &mut usize) -> u64 {
    let mut value = 0_u64;
    while let Some(digit) = units.get(*index).copied().and_then(digit) {
        value = value.wrapping_mul(10).wrapping_add(digit);
        *index += 1;
    }
    value
}

/// Java `Character.toLowerCase(char)`; surrogate halves map to themselves.
fn lowercase(unit: u16) -> u32 {
    char::from_u32(u32::from(unit)).map_or(u32::from(unit), |character| {
        u32::from(CaseMapperBorrowed::new().simple_lowercase(character))
    })
}
