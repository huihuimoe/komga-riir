use std::collections::BTreeSet;
use std::fs::{self, Permissions};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use komga_types::{FileUrl, Library, SidecarKind};

use super::{ScanError, scan_root_folder};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().expect("Fixture files have a parent")).expect("Fixture dir");
    fs::write(path, b"book").expect("Fixture file");
}

/// Series name, oneshot flag and sorted book names, sorted by series name.
fn summary(library: &Library) -> Vec<(String, bool, Vec<String>)> {
    let mut summary: Vec<(String, bool, Vec<String>)> = scan_root_folder(library)
        .expect("Root is accessible")
        .series
        .into_iter()
        .map(|(series, books)| {
            let mut names: Vec<String> = books.into_iter().map(|book| book.name).collect();
            names.sort();
            (series.name, series.oneshot, names)
        })
        .collect();
    summary.sort();
    summary
}

#[test]
fn groups_books_by_directory_like_walk_file_tree() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let root = directory.path().join("library");
    for file in [
        "r.cbz",
        "A/a1.cbz",
        "A/a2.CBZ",
        "A/.hidden.cbz",
        "A/notes.txt",
        "A/sub/b.pdf",
        ".hiddendir/x.cbz",
        "skip #excluded/e.cbz",
        "_oneshots/o1.cbz",
        "_oneshots/o2.epub",
    ] {
        touch(&root.join(file));
    }
    symlink(root.join("A"), root.join("link-to-a")).expect("Symlink");
    symlink(&root, root.join("A/loop")).expect("Symlink");
    symlink(root.join("missing.cbz"), root.join("A/broken.cbz")).expect("Symlink");
    let mut library = Library::new("Test".to_owned(), root.clone());
    library.scan_directory_exclusions = BTreeSet::from(["#EXCLUDED".to_owned()]);
    library.oneshots_directory = Some("_oneshots".to_owned());

    let names = |names: &[&str]| -> Vec<String> { names.iter().map(|&n| n.to_owned()).collect() };
    assert_eq!(
        summary(&library),
        vec![
            ("A".to_owned(), false, names(&["a1", "a2"])),
            ("library".to_owned(), false, names(&["r"])),
            ("link-to-a".to_owned(), false, names(&["a1", "a2"])),
            ("o1".to_owned(), true, names(&["o1"])),
            ("o2".to_owned(), true, names(&["o2"])),
            ("sub".to_owned(), false, names(&["b"])),
            // `link-to-a/sub` is a second path to the same directory.
            ("sub".to_owned(), false, names(&["b"])),
        ]
    );

    let scanned = scan_root_folder(&library)
        .expect("Root is accessible")
        .series;
    let (series, books) = scanned
        .iter()
        .find(|(series, _)| series.name == "A")
        .expect("Series A");
    assert_eq!(series.url, FileUrl::directory(&root.join("A")));
    assert!(series.url.as_str().ends_with("/library/A/"));
    assert!(books.iter().all(|book| book.library_id == library.id));
    let (oneshot, books) = scanned
        .iter()
        .find(|(series, _)| series.name == "o1")
        .expect("Oneshot o1");
    assert_eq!(
        oneshot.url, books[0].url,
        "A oneshot series is its book file"
    );
    assert!(books[0].oneshot);
}

#[test]
fn filters_extensions_by_library_settings() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let root = directory.path().join("library");
    for file in ["S/a.cbr", "S/b.pdf", "S/c.epub", "S/d.zip"] {
        touch(&root.join(file));
    }
    let mut library = Library::new("Test".to_owned(), root);
    library.scan_cbx = false;
    library.scan_epub = false;

    assert_eq!(
        summary(&library),
        vec![("S".to_owned(), false, vec!["b".to_owned()])]
    );
}

#[test]
fn hidden_root_yields_no_series() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let root = directory.path().join(".library");
    touch(&root.join("S/a.cbz"));

    assert_eq!(summary(&Library::new("Test".to_owned(), root)), Vec::new());
}

#[test]
fn missing_root_or_file_root_is_not_found() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let file = directory.path().join("file.cbz");
    touch(&file);

    for root in [directory.path().join("missing"), file] {
        let library = Library::new("Test".to_owned(), root.clone());
        assert!(
            matches!(scan_root_folder(&library), Err(ScanError::DirectoryNotFound(path)) if path == root),
            "{} must not scan",
            root.display()
        );
    }
}

#[test]
fn records_directories_it_cannot_fully_list() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let root = directory.path().join("library");
    for file in [
        "S/a.cbz",
        "Locked/b.cbz",
        "Unsearchable/c.cbz",
        "Unsearchable/Sub/d.cbz",
        ".hidden/e.cbz",
    ] {
        touch(&root.join(file));
    }
    // Unlistable directories, and one listable whose entries cannot be read.
    let modes = [
        ("Locked", 0o000),
        (".hidden", 0o000),
        ("Unsearchable", 0o600),
    ];
    for (name, mode) in modes {
        fs::set_permissions(root.join(name), Permissions::from_mode(mode)).expect("Deny access");
    }
    let scanned = scan_root_folder(&Library::new("Test".to_owned(), root.clone()));
    for (name, _) in modes {
        fs::set_permissions(root.join(name), Permissions::from_mode(0o755)).expect("Restore");
    }
    let scanned = scanned.expect("Root is accessible");

    let names: Vec<&str> = scanned
        .series
        .iter()
        .map(|(series, _)| series.name.as_str())
        .collect();
    assert_eq!(names, ["S"]);
    let unread = |path: &str| {
        scanned
            .unread
            .contains(&FileUrl::directory(&root.join(path)))
    };
    assert!(unread("Locked") && unread("Unsearchable"));
    assert!(
        unread("Unsearchable/Sub"),
        "what lies under an unread directory is undecided"
    );
    assert!(
        scanned
            .unread
            .contains(&FileUrl::file(&root.join("Locked/b.cbz")))
    );
    assert!(!unread("S") && !unread(".hidden") && !unread(""));
    assert!(
        !unread("Locked2"),
        "a sibling sharing a name prefix is not under the directory"
    );
}

#[test]
fn finds_sidecars_of_directories_with_books() {
    let directory = komga_test_support::tempdir().expect("Temp dir");
    let root = directory.path().join("library");
    for file in [
        "S/a.cbz",
        "S/a-1.cbz",
        "S/A-1.JPG",
        "S/a-2.png",
        "S/a.avif",
        "S/other.png",
        "S/Cover.jpg",
        "S/series.json",
        "Empty/cover.jpg",
        "Empty/series.json",
        "_oneshots/o.cbz",
        "_oneshots/o.png",
        "_oneshots/cover.jpg",
    ] {
        touch(&root.join(file));
    }
    let mut library = Library::new("Test".to_owned(), root.clone());
    library.oneshots_directory = Some("_oneshots".to_owned());

    let scanned = scan_root_folder(&library).expect("Root is accessible");
    let url = |path: &str| FileUrl::file(&root.join(path));
    let mut sidecars: Vec<(FileUrl, FileUrl, SidecarKind)> = scanned
        .sidecars
        .iter()
        .cloned()
        .map(|sidecar| (sidecar.url, sidecar.parent_url, sidecar.kind))
        .collect();
    sidecars.sort_by(|left, right| left.0.cmp(&right.0));
    let series = FileUrl::directory(&root.join("S"));
    // `A-1.JPG` matches both `a` (with a counter) and `a-1`; the book listed first takes it.
    let (_, books) = scanned
        .series
        .iter()
        .find(|(series, _)| series.name == "S")
        .expect("Series S");
    let first_book = books
        .iter()
        .find(|book| book.name == "a" || book.name == "a-1")
        .map(|book| book.url.clone())
        .expect("Both books are scanned");
    let mut expected = vec![
        (url("S/A-1.JPG"), first_book, SidecarKind::BookArtwork),
        (
            url("S/Cover.jpg"),
            series.clone(),
            SidecarKind::SeriesArtwork,
        ),
        (url("S/a-2.png"), url("S/a.cbz"), SidecarKind::BookArtwork),
        (url("S/series.json"), series, SidecarKind::SeriesMetadata),
        (
            url("_oneshots/o.png"),
            url("_oneshots/o.cbz"),
            SidecarKind::BookArtwork,
        ),
    ];
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(sidecars, expected);
}
