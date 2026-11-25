//! Apache Commons `ISBNValidator(convert = true).validate`.

/// `(?:\-|\s)` of Java's regular expressions.
const SEPARATORS: [char; 7] = ['-', ' ', '\t', '\n', '\u{b}', '\u{c}', '\r'];
/// The digit counts of the group, publisher, and title parts of a separated ISBN.
const PART_LENGTHS: [usize; 3] = [5, 7, 6];

/// The ISBN-13 of `code`, an ISBN-10 or ISBN-13 with or without separators and surrounding
/// control characters or spaces; `None` when `code` is no valid ISBN.
#[must_use]
pub fn validate_isbn(code: &str) -> Option<String> {
    // Java `String.trim`.
    let code = code.trim_matches(|c: char| c <= ' ');
    isbn13(code).or_else(|| isbn10(code).map(|isbn10| to_isbn13(&isbn10[..9])))
}

/// Hibernate Validator `@ISBN(type = ISBN_13)`: once every character but digits and `X` is
/// dropped, thirteen digits with a valid EAN-13 check digit.
#[must_use]
pub fn is_isbn13(code: &str) -> bool {
    let kept: String = code
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X')
        .collect();
    kept.len() == 13
        && kept.bytes().all(|b| b.is_ascii_digit())
        && ean13_check_digit(&kept[..12]) == kept.as_bytes()[12]
}

/// `ISBN13_REGEX`: `978` or `979`, then ten digits, or four separated parts.
fn isbn13(code: &str) -> Option<String> {
    let rest = code
        .strip_prefix("978")
        .or_else(|| code.strip_prefix("979"))?;
    let digits = if rest.len() == 10 && rest.bytes().all(|b| b.is_ascii_digit()) {
        rest.to_owned()
    } else {
        separated(rest.strip_prefix(SEPARATORS)?, false)?
    };
    let isbn = format!("{}{digits}", &code[..3]);
    (isbn.len() == 13 && ean13_check_digit(&isbn[..12]) == isbn.as_bytes()[12]).then_some(isbn)
}

/// `ISBN10_REGEX`: nine digits and a digit or `X`, or four separated parts.
fn isbn10(code: &str) -> Option<String> {
    let bytes = code.as_bytes();
    let isbn = if bytes.len() == 10
        && bytes[..9].iter().all(u8::is_ascii_digit)
        && (bytes[9].is_ascii_digit() || bytes[9] == b'X')
    {
        code.to_owned()
    } else {
        separated(code, true)?
    };
    let sum: u32 = isbn
        .bytes()
        .zip((1..=10).rev())
        .map(|(digit, weight)| {
            let value = if digit == b'X' {
                10
            } else {
                u32::from(digit - b'0')
            };
            value * weight
        })
        .sum();
    (isbn.len() == 10 && sum.is_multiple_of(11)).then_some(isbn)
}

/// The concatenated parts of `group-publisher-title-check`, where the check is a digit, or also
/// `X` when `x_check` is set.
fn separated(code: &str, x_check: bool) -> Option<String> {
    let parts: Vec<&str> = code.split(SEPARATORS).collect();
    let [group, publisher, title, check] = parts.as_slice() else {
        return None;
    };
    for (part, max) in [group, publisher, title].into_iter().zip(PART_LENGTHS) {
        if part.is_empty() || part.len() > max || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let valid_check = match check.as_bytes() {
        [digit] => digit.is_ascii_digit() || (x_check && *digit == b'X'),
        _ => false,
    };
    valid_check.then(|| format!("{group}{publisher}{title}{check}"))
}

/// `ISBNValidator.convertToISBN13`.
fn to_isbn13(isbn10_digits: &str) -> String {
    let partial = format!("978{isbn10_digits}");
    let check = char::from(ean13_check_digit(&partial));
    format!("{partial}{check}")
}

/// `EAN13CheckDigit` of twelve ASCII digits.
fn ean13_check_digit(digits: &str) -> u8 {
    let sum: u32 = digits
        .bytes()
        .enumerate()
        .map(|(index, digit)| u32::from(digit - b'0') * if index % 2 == 0 { 1 } else { 3 })
        .sum();
    let check = u8::try_from((10 - sum % 10) % 10).expect("A digit is below 10");
    b'0' + check
}

#[cfg(test)]
mod tests {
    use super::{is_isbn13, validate_isbn};

    #[test]
    fn isbn_13_and_isbn_10_validate_to_isbn_13() {
        for (code, expected) in [
            ("9782811632397", Some("9782811632397")),
            (" 978-2-8116-3239-7\t", Some("9782811632397")),
            ("979 10 90636 07 1", Some("9791090636071")),
            ("2811632395", Some("9782811632397")),
            ("2-8116-3239-5", Some("9782811632397")),
            ("080442957X", Some("9780804429573")),
            ("0-8044-2957-X", Some("9780804429573")),
            ("9782811632398", None),
            ("2811632394", None),
            ("978-28116-32397", None),
            ("978--2-8116-3239-7", None),
            ("97828116323971", None),
            ("080442957x", None),
            ("", None),
            ("abc", None),
        ] {
            assert_eq!(validate_isbn(code).as_deref(), expected, "{code:?}");
        }
    }

    #[test]
    fn isbn_13_ignores_everything_but_digits_and_x() {
        for (code, expected) in [
            ("9782811632397", true),
            ("ISBN 978-2-8116-3239-7", true),
            ("9782811632398", false),
            ("2811632395", false),
            ("978281163239X", false),
            ("97828116323971", false),
        ] {
            assert_eq!(is_isbn13(code), expected, "{code:?}");
        }
    }
}
