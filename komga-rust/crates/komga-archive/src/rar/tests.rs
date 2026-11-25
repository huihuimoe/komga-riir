//! Names, sizes and data as junrar 8.1.2 reads the fixtures: `rar4*.rar`/`rar5.rar`/
//! `rar5-solid.rar` are Komga's own test archives; `rar4-names.rar` holds a Windows path (`\`,
//! host Win32), a RAR Unicode-encoded name, an ISO-8859-1 name that is not UTF-8, a directory
//! and a non-image; `rar5-names.rar` holds two versions of one file (`rar -ver`) and a name that
//! is not UTF-8; `rar7.rar` is solid with a dictionary only RAR 7 can describe;
//! `rar-unversioned.rar` is `rar5-names.rar` after a `Rar!` prefix.
//!
//! `rar{4,5}-header-encrypted.rar` are Komga's `rar{4,5}-encrypted.rar`; the other failure
//! fixtures are written by `rars` from one book: `*-file-encrypted` encrypts members only,
//! `*-volume` is a first volume, `*-corrupt` has a damaged `sub/page-2.jpg` and `*-truncated`
//! ends inside it. `rar4-oversized.rar` is `rar4.rar` with `komga-2.png` declaring 300 MiB and
//! `rar5-dictionary.rar` is `rar5-solid.rar` declaring 4 GiB dictionaries; header CRCs fixed.

use std::path::{Path, PathBuf};

use rars::ErrorKind;

use super::{RarArchive, RarError, read_entry};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rar")
        .join(name)
}

fn entries(name: &str) -> Vec<(String, u64, bool)> {
    let archive = RarArchive::open(&fixture(name)).expect("Readable fixture");
    archive
        .entries()
        .iter()
        .map(|entry| (entry.name().to_owned(), entry.size(), entry.is_directory()))
        .collect()
}

/// Each file member read in one pass, by index.
fn read_all(path: &Path) -> Vec<(usize, Vec<u8>)> {
    let archive = RarArchive::open(path).expect("Readable fixture");
    let mut members = Vec::new();
    archive
        .read_all(|index, data| members.push((index, data.expect("Readable member"))))
        .expect("Readable archive");
    members
}

#[test]
fn entries_keep_junrar_names_and_sizes() {
    let page = |name: &str| (name.to_owned(), 3108, false);
    for archive in ["rar4.rar", "rar4-solid.rar", "rar5-solid.rar"] {
        assert_eq!(
            entries(archive),
            [
                page("komga-1.png"),
                page("komga-2.png"),
                page("komga-3.png")
            ],
            "{archive}"
        );
    }
    assert_eq!(
        entries("rar4-names.rar"),
        [
            ("第1話.png".to_owned(), 251, false),
            ("sub".to_owned(), 0, true),
            ("sub\\page-2.jpg".to_owned(), 1807, false),
            ("ComicInfo.xml".to_owned(), 13, false),
            ("café-10.png".to_owned(), 251, false),
        ]
    );
    assert_eq!(entries("rar5.rar"), [page("komga.png")]);
    let rar5_names = [
        ("page.png;1".to_owned(), 251, false),
        ("page.png".to_owned(), 1719, false),
        ("caf\u{fffd}.png".to_owned(), 251, false),
    ];
    assert_eq!(entries("rar5-names.rar"), rar5_names);
    assert_eq!(entries("rar-unversioned.rar"), rar5_names);
    assert_eq!(
        entries("rar7.rar"),
        [
            ("r7/page-1.bmp".to_owned(), 2934, false),
            ("r7/page-2.png".to_owned(), 251, false),
        ]
    );
}

#[test]
fn archives_junrar_cannot_open_do_not_open() {
    assert!(matches!(
        RarArchive::open(&fixture("rar14-prefixed.rar")),
        Err(RarError::Rar14AfterPrefix)
    ));
    assert!(matches!(
        RarArchive::open(&fixture("rar-unknown.rar")),
        Err(RarError::Archive(_))
    ));
}

#[test]
fn one_pass_reads_every_file_member_as_reading_by_name_does() {
    let archives = [
        "rar4.rar",
        "rar4-solid.rar",
        "rar4-names.rar",
        "rar5.rar",
        "rar5-solid.rar",
        "rar5-names.rar",
        "rar-unversioned.rar",
        "rar7.rar",
    ];
    for archive in archives {
        let path = fixture(archive);
        let names = RarArchive::open(&path).expect("Readable fixture").entries;
        let members = read_all(&path);
        let files: Vec<usize> = (0..names.len())
            .filter(|&index| !names[index].is_directory())
            .collect();
        assert_eq!(
            members.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            files,
            "{archive}: every file member once, in order, and no directory"
        );
        for (index, data) in members {
            let entry = &names[index];
            assert_eq!(
                data.len() as u64,
                entry.size(),
                "{archive}: {}",
                entry.name()
            );
            let by_name = read_entry(&path, entry.name()).expect("Readable member");
            assert_eq!(by_name, data, "{archive}: {}", entry.name());
        }
    }
}

#[test]
fn reading_a_missing_member_is_entry_not_found() {
    let error = read_entry(&fixture("rar4.rar"), "komga-4.png").expect_err("No such member");
    assert!(
        matches!(&error, RarError::EntryNotFound(name) if name == "komga-4.png"),
        "{error:?}"
    );
}

#[test]
fn a_missing_file_is_an_io_not_found() {
    let error = read_entry(&fixture("missing.rar"), "komga-1.png").expect_err("No file");
    assert!(
        matches!(&error, RarError::Io(cause) if cause.kind() == std::io::ErrorKind::NotFound),
        "{error:?}"
    );
}

#[test]
fn encrypted_headers_and_members_are_refused_before_volumes() {
    for archive in [
        "rar4-header-encrypted.rar",
        "rar5-header-encrypted.rar",
        "rar4-file-encrypted.rar",
        "rar5-file-encrypted.rar",
    ] {
        let opened = RarArchive::open(&fixture(archive));
        assert!(matches!(opened, Err(RarError::Encrypted)), "{archive}");
    }
    for archive in ["rar4-volume.part1.rar", "rar5-volume.part01.rar"] {
        let opened = RarArchive::open(&fixture(archive));
        assert!(matches!(opened, Err(RarError::MultiVolume)), "{archive}");
    }
}

/// Each file member's name with how one pass read it: `None` when read, else the error.
fn read_outcomes(archive: &str) -> Vec<(String, Option<RarError>)> {
    let archive = RarArchive::open(&fixture(archive)).expect("Readable headers");
    let mut outcomes = Vec::new();
    archive
        .read_all(|index, data| {
            outcomes.push((archive.entries()[index].name().to_owned(), data.err()));
        })
        .expect("No failure ends the pass");
    outcomes
}

fn is_kind(error: Option<&RarError>, kind: ErrorKind) -> bool {
    matches!(error, Some(RarError::Archive(error)) if error.kind() == kind)
}

#[test]
fn a_damaged_member_fails_alone_and_a_truncated_archive_does_not_open() {
    for archive in ["rar4-corrupt.rar", "rar5-corrupt.rar"] {
        let outcomes = read_outcomes(archive);
        let names: Vec<&str> = outcomes.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "page-1.png",
                "sub/page-2.jpg",
                "ComicInfo.xml",
                "page-10.png"
            ],
            "{archive}"
        );
        for (name, error) in &outcomes {
            if name == "sub/page-2.jpg" {
                assert!(
                    is_kind(error.as_ref(), ErrorKind::ChecksumMismatch),
                    "{archive}: {error:?}"
                );
            } else {
                assert!(error.is_none(), "{archive}: {name}: {error:?}");
            }
        }
    }
    for archive in ["rar4-truncated.rar", "rar5-truncated.rar"] {
        let opened = RarArchive::open(&fixture(archive));
        assert!(matches!(opened, Err(RarError::Archive(_))), "{archive}");
    }
}

#[test]
fn members_over_the_decoding_limits_fail_before_they_are_decoded() {
    let outcomes = read_outcomes("rar4-oversized.rar");
    assert_eq!(outcomes.len(), 3, "{outcomes:?}");
    assert!(
        outcomes[0].1.is_none() && outcomes[2].1.is_none(),
        "{outcomes:?}"
    );
    assert!(
        matches!(outcomes[1], (ref name, Some(RarError::EntryTooLarge(314_572_800))) if name == "komga-2.png"),
        "{outcomes:?}"
    );
    let by_name = read_entry(&fixture("rar4-oversized.rar"), "komga-2.png");
    assert!(
        matches!(&by_name, Err(RarError::Archive(error)) if error.kind() == ErrorKind::ResourceLimit),
        "{by_name:?}"
    );
    // Later members of a solid stream need the refused one decoded, so the pass ends.
    let archive = RarArchive::open(&fixture("rar5-dictionary.rar")).expect("Readable headers");
    let mut visited = Vec::new();
    let pass = archive.read_all(|index, data| visited.push((index, data.err())));
    assert!(
        matches!(&pass, Err(RarError::Archive(error)) if error.kind() == ErrorKind::ResourceLimit),
        "{pass:?}"
    );
    assert!(
        matches!(visited.as_slice(), [(0, error)] if is_kind(error.as_ref(), ErrorKind::ResourceLimit)),
        "{visited:?}"
    );
}
