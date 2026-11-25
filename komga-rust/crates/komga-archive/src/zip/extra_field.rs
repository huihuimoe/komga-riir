//! The extra fields commons-compress `ZipFile` acts on: ZIP64 sizes and offsets, and the
//! Info-ZIP Unicode Path that renames entries without the UTF-8 flag.

const ZIP64: u16 = 0x0001;
const UNICODE_PATH: u16 = 0x7075;

/// `ExtraFieldUtils.parse`: `(header id, data)` until a field claims more bytes than remain,
/// which ends the parsing.
fn fields(extra: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut rest = extra;
    std::iter::from_fn(move || {
        let [id_low, id_high, length_low, length_high, data @ ..] = rest else {
            return None;
        };
        let length = usize::from(u16::from_le_bytes([*length_low, *length_high]));
        let field = data.get(..length)?;
        rest = &data[length..];
        Some((u16::from_le_bytes([*id_low, *id_high]), field))
    })
}

/// The data of the first ZIP64 extended information field; later ones are ignored.
pub(super) fn zip64(central_extra: &[u8]) -> Option<&[u8]> {
    fields(central_extra).find_map(|(id, data)| (id == ZIP64).then_some(data))
}

/// `UnicodePathExtraField`: the CRC-32 of the name it replaces and the UTF-8 name.
#[derive(Clone, Copy)]
pub(super) struct UnicodePath<'a> {
    pub(super) name_crc: u32,
    pub(super) name: &'a [u8],
}

/// The entry's Unicode Path field once its central and local fields are merged, if it has a
/// valid one. A field that fails to parse turns into an unrecognized field of the same id,
/// which later fields cannot replace.
pub(super) fn unicode_path<'a>(
    central_extra: &'a [u8],
    local_extra: &'a [u8],
) -> Option<UnicodePath<'a>> {
    let unicode_paths = |extra| {
        fields(extra)
            .filter(|&(id, _)| id == UNICODE_PATH)
            .map(|(_, data)| parse(data))
    };
    // Central fields replace nothing: the first one is the entry's.
    let mut merged = unicode_paths(central_extra).next();
    // Each local field reparses the entry's field into itself.
    for local in unicode_paths(local_extra) {
        merged = match merged {
            Some(None) => Some(None),
            _ => Some(local),
        };
    }
    merged.flatten()
}

fn parse(data: &[u8]) -> Option<UnicodePath<'_>> {
    let [1, crc @ ..] = data.get(..5)? else {
        return None;
    };
    Some(UnicodePath {
        name_crc: u32::from_le_bytes(crc.try_into().expect("Four CRC bytes")),
        name: &data[5..],
    })
}
