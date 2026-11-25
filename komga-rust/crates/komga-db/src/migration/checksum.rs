// Flyway CRC32: strip an initial UTF-8 BOM and line terminators, then checksum the concatenated UTF-8 lines.
pub(super) fn flyway_checksum(contents: &[u8]) -> i32 {
    const CRC32_POLYNOMIAL: u32 = 0xedb8_8320;

    let mut checksum = u32::MAX;
    let mut offset = usize::from(contents.starts_with(b"\xef\xbb\xbf")) * 3;

    while offset < contents.len() {
        let byte = contents[offset];
        offset += 1;

        if byte == b'\r' || byte == b'\n' {
            continue;
        }

        checksum ^= u32::from(byte);
        for _ in 0..8 {
            checksum = if checksum & 1 == 0 {
                checksum >> 1
            } else {
                (checksum >> 1) ^ CRC32_POLYNOMIAL
            };
        }
    }

    (!checksum) as i32
}
