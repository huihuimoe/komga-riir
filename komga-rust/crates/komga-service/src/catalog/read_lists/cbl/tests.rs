use komga_types::{ReadListRequest, ReadListRequestBook};

use super::{ComicRackListError, import_from_cbl};

fn import(xml: &str) -> Result<ReadListRequest, ComicRackListError> {
    import_from_cbl(xml.as_bytes())
}

fn book(series: &[&str], number: &str) -> ReadListRequestBook {
    ReadListRequestBook {
        series: series.iter().map(|series| (*series).to_owned()).collect(),
        number: number.to_owned(),
    }
}

#[test]
fn books_come_from_attributes_or_child_elements() {
    let request = import(
        r#"<?xml version="1.0"?>
    <ReadingList xmlns:xsd="http://www.w3.org/2001/XMLSchema">
      <Name> My &amp; list </Name>
      <Books>
        <Book Series="series 1" Number=" 4 " Volume="2005" Year="2005">
          <Id>e5cf3d6a</Id>
        </Book>
        <Book><Series>series 2</Series><Number>1</Number><Volume>1</Volume></Book>
        <Other Series="series 3" Number="" Volume=" " />
        <Book Series="ignored" Number="1"><Series>series 4</Series></Book>
      </Books>
      <Matchers />
    </ReadingList>"#,
    )
    .unwrap();
    assert_eq!(request.name, " My & list ");
    assert_eq!(
        request.books,
        [
            book(&["series 1 (2005)", "series 1"], "4"),
            book(&["series 2"], "1"),
            book(&["series 3"], ""),
            book(&["series 4"], "1"),
        ]
    );
}

#[test]
fn errors_are_checked_in_kotlin_order() {
    let cases = [
        ("", ComicRackListError::Malformed),
        (
            "<ReadingList><Name>x</ReadingList>",
            ComicRackListError::Malformed,
        ),
        ("<ReadingList><Name>x</Name>", ComicRackListError::Malformed),
        (
            r#"<ReadingList><Name>x</Name><Books><Book Series="s" Number="1" Volume="v"/></Books></ReadingList>"#,
            ComicRackListError::Malformed,
        ),
        (
            r#"<ReadingList><Books><Book Series="s" Number="1"/></Books></ReadingList>"#,
            ComicRackListError::NoName,
        ),
        (
            r#"<ReadingList><Name> </Name><Books/></ReadingList>"#,
            ComicRackListError::NoName,
        ),
        (
            "<ReadingList><Name>x</Name><Books>\n</Books></ReadingList>",
            ComicRackListError::NoBooks,
        ),
        (
            "<ReadingList><Name>x</Name></ReadingList>",
            ComicRackListError::NoBooks,
        ),
        (
            r#"<ReadingList><Name>x</Name><Books><Book Series=" " Number="4"/></Books></ReadingList>"#,
            ComicRackListError::IncompleteBook,
        ),
        (
            r#"<ReadingList><Name>x</Name><Books><Book Series="s"/></Books></ReadingList>"#,
            ComicRackListError::IncompleteBook,
        ),
    ];
    for (xml, expected) in cases {
        assert_eq!(import(xml), Err(expected), "{xml}");
    }
}

#[test]
fn a_later_books_element_replaces_the_books() {
    let request = import(
        r#"<ReadingList><Name>a</Name><Name>b</Name>
                <Books><Book Series="s" Number="1"/></Books>
                <Books><Book Series="t" Number="2"/></Books></ReadingList>"#,
    )
    .unwrap();
    assert_eq!(request.name, "b");
    assert_eq!(request.books, [book(&["t"], "2")]);
}
