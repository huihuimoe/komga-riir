//! Spring `ContentDisposition` values with a file name, as the servlet container sends them.

use axum::http::HeaderValue;

/// `filename(name, UTF_8)`: `<disposition>; filename="<RFC 2047 Q-encoded>";
/// filename*=UTF-8''<RFC 5987 encoded>`. Both encodings leave only printable ASCII, so the value is always a valid header.
pub(crate) fn header(disposition: &str, file_name: &str) -> HeaderValue {
    let mut quoted = String::from("=?UTF-8?Q?");
    let mut extended = String::from("UTF-8''");
    for byte in file_name.bytes() {
        match byte {
            b' ' => quoted.push('_'),
            b'"' | b'=' | b'?' | b'_' => push_escaped(&mut quoted, '=', byte),
            33..=126 => quoted.push(char::from(byte)),
            _ => push_escaped(&mut quoted, '=', byte),
        }
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            extended.push(char::from(byte));
        } else {
            push_escaped(&mut extended, '%', byte);
        }
    }
    quoted.push_str("?=");
    let value = format!("{disposition}; filename=\"{quoted}\"; filename*={extended}");
    HeaderValue::from_str(&value).expect("The encoded value is printable ASCII")
}

/// The same without a charset: `<disposition>; filename="<name>"` with `"` and `\` escaped, as
/// Tomcat writes it: ISO-8859-1, `?` for characters beyond it, and spaces for controls but tab.
pub(crate) fn latin1_header(disposition: &str, file_name: &str) -> HeaderValue {
    let mut value = format!("{disposition}; filename=\"").into_bytes();
    for ch in file_name.chars() {
        if matches!(ch, '"' | '\\') {
            value.push(b'\\');
        }
        value.push(match u8::try_from(ch) {
            Ok(byte) if (byte < 0x20 && byte != b'\t') || byte == 0x7f => b' ',
            Ok(byte) => byte,
            Err(_) => b'?',
        });
    }
    value.push(b'"');
    HeaderValue::from_bytes(&value).expect("Controls are replaced")
}

fn push_escaped(encoded: &mut String, marker: char, byte: u8) {
    encoded.push(marker);
    encoded.push_str(&format!("{byte:02X}"));
}

#[cfg(test)]
mod tests {
    use super::{header, latin1_header};

    #[test]
    fn latin1_file_names_are_escaped_like_spring_on_tomcat() {
        for (name, expected) in [
            ("a.woff2", b"attachment; filename=\"a.woff2\"".as_slice()),
            (
                "a \"b\"\\c.ttf",
                b"attachment; filename=\"a \\\"b\\\"\\\\c.ttf\"",
            ),
            (
                "caf\u{e9}\u{5b8b}\u{1}.css",
                b"attachment; filename=\"caf\xe9? .css\"",
            ),
        ] {
            assert_eq!(
                latin1_header("attachment", name).as_bytes(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn file_names_are_encoded_like_spring() {
        // Spring Web 6.2 output for the same names.
        for (name, expected) in [
            (
                "a-1.jpg",
                "inline; filename=\"=?UTF-8?Q?a-1.jpg?=\"; filename*=UTF-8''a-1.jpg",
            ),
            (
                "Série é-2.png",
                "inline; filename=\"=?UTF-8?Q?S=C3=A9rie_=C3=A9-2.png?=\"; \
                 filename*=UTF-8''S%C3%A9rie%20%C3%A9-2.png",
            ),
            (
                "a \"b\"\\c;d-3.webp",
                "inline; filename=\"=?UTF-8?Q?a_=22b=22\\c;d-3.webp?=\"; \
                 filename*=UTF-8''a%20%22b%22%5Cc%3Bd-3.webp",
            ),
            (
                "漫画 01-4.heic",
                "inline; filename=\"=?UTF-8?Q?=E6=BC=AB=E7=94=BB_01-4.heic?=\"; \
                 filename*=UTF-8''%E6%BC%AB%E7%94%BB%2001-4.heic",
            ),
            (
                "x%y=z?-5.gif",
                "inline; filename=\"=?UTF-8?Q?x%y=3Dz=3F-5.gif?=\"; \
                 filename*=UTF-8''x%25y%3Dz%3F-5.gif",
            ),
            (
                "tab\tx-1.jpg",
                "inline; filename=\"=?UTF-8?Q?tab=09x-1.jpg?=\"; filename*=UTF-8''tab%09x-1.jpg",
            ),
        ] {
            assert_eq!(header("inline", name), expected, "{name}");
        }
    }
}
