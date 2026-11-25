use jiff::civil::date;
use komga_types::{BookAuthor, NotableEntity, ReadingDirection, WebLink};

use super::{parse_float, patch, series};
use crate::metadata::edit::patch::{BookMetadataPatch, ReadListEntry};
use crate::metadata::edit::series_patch::SeriesMetadataPatch;
use crate::metadata::providers::comic_info::{self, ComicInfo};

fn comic_info(xml: &str) -> ComicInfo {
    comic_info::parse(format!("<ComicInfo>{xml}</ComicInfo>").as_bytes()).unwrap()
}

#[test]
fn every_book_field_is_read() {
    let comic_info = comic_info(
        r"<Title>The Title</Title>
          <Summary>Summary</Summary>
          <Number>1.5</Number>
          <Year>2020</Year><Month>2</Month>
          <Writer>Alice, Bob</Writer><Penciller> Carol </Penciller><CoverArtist>Dan</CoverArtist>
          <Locations>Paris</Locations><Teams>X</Teams><Characters>Eve, ,Frank</Characters>
          <Web>https://Example.com/a  not-a-uri https://b.org/</Web>
          <Tags>Fun, SAD,</Tags>
          <GTIN>978-2-8116-3239-7</GTIN>",
    );
    assert_eq!(
        patch(&comic_info),
        Some(BookMetadataPatch {
            title: Some("The Title".to_owned()),
            summary: Some("Summary".to_owned()),
            number: Some("1.5".to_owned()),
            number_sort: Some(1.5),
            release_date: Some(date(2020, 2, 1)),
            authors: Some(vec![
                BookAuthor::new("Alice", "writer"),
                BookAuthor::new("Bob", "writer"),
                BookAuthor::new("Carol", "penciller"),
                BookAuthor::new("Dan", "cover"),
            ]),
            notable_entities: Some(vec![
                NotableEntity::new("location", "Paris"),
                NotableEntity::new("team", "X"),
                NotableEntity::new("character", "Eve"),
                NotableEntity::new("character", "Frank"),
            ]),
            isbn: Some("9782811632397".to_owned()),
            links: Some(vec![
                WebLink {
                    label: "example.com".to_owned(),
                    url: "https://Example.com/a".to_owned(),
                },
                WebLink {
                    label: "b.org".to_owned(),
                    url: "https://b.org/".to_owned(),
                },
            ]),
            tags: Some(vec!["fun".to_owned(), "sad".to_owned()]),
            read_lists: Vec::new(),
        })
    );
}

#[test]
fn blank_or_invalid_values_are_not_found() {
    let comic_info = comic_info(
        r"<Title> </Title><Number>One</Number><Writer>,</Writer><Web>mailto:a@b.c</Web>
          <Tags> </Tags><GTIN>123</GTIN>",
    );
    assert_eq!(
        patch(&comic_info),
        Some(BookMetadataPatch {
            number: Some("One".to_owned()),
            ..BookMetadataPatch::default()
        })
    );
}

#[test]
fn a_date_that_does_not_exist_fails_the_patch() {
    assert_eq!(
        patch(&comic_info("<Year>2020</Year><Month>13</Month>")),
        None
    );
    assert_eq!(
        patch(&comic_info("<Month>13</Month>")),
        Some(BookMetadataPatch::default())
    );
}

#[test]
fn read_lists_come_from_the_alternate_series_and_numbered_story_arcs() {
    let numbered = comic_info(
        r"<AlternateSeries>Event</AlternateSeries><AlternateNumber>2</AlternateNumber>
          <StoryArc>A, ,C, D</StoryArc><StoryArcNumber>1, 2, x</StoryArcNumber>",
    );
    let entry = |name: &str, number| ReadListEntry {
        name: name.to_owned(),
        number,
    };
    assert_eq!(
        patch(&numbered).unwrap().read_lists,
        [entry("Event", Some(2)), entry("A", Some(1))]
    );
    let unnumbered = comic_info("<AlternateNumber>2</AlternateNumber><StoryArc>A, B</StoryArc>");
    assert_eq!(
        patch(&unnumbered).unwrap().read_lists,
        [entry("A", None), entry("B", None)]
    );
}

#[test]
fn numbers_sort_as_finite_decimals() {
    for (number, expected) in [
        (" 2.5 ", Some(2.5)),
        ("1e1", Some(10.0)),
        ("-3", Some(-3.0)),
        ("1a", None),
        ("NaN", None),
        ("inf", None),
        ("", None),
    ] {
        assert_eq!(parse_float(number), expected, "{number:?}");
    }
}

#[test]
fn every_series_field_is_read() {
    let comic_info = comic_info(
        r"<Series>Saga</Series><Volume>2</Volume><Count>9</Count>
          <Manga>YesAndRightToLeft</Manga><Publisher>Image</Publisher>
          <AgeRating>mature 17+</AgeRating><LanguageISO>EN-us</LanguageISO>
          <Genre>Space, Opera,</Genre><SeriesGroup>A, ,B</SeriesGroup>",
    );
    let expected = |title: &str| SeriesMetadataPatch {
        title: Some(title.to_owned()),
        title_sort: Some(title.to_owned()),
        reading_direction: Some(ReadingDirection::RightToLeft),
        publisher: Some("Image".to_owned()),
        age_rating: Some(17),
        language: Some("en-US".to_owned()),
        genres: Some(vec!["Space".to_owned(), "Opera".to_owned()]),
        total_book_count: Some(9),
        collections: vec!["A".to_owned(), "B".to_owned()],
        ..SeriesMetadataPatch::default()
    };
    assert_eq!(series(&comic_info, true), expected("Saga (2)"));
    assert_eq!(series(&comic_info, false), expected("Saga"));
}

#[test]
fn unknown_series_values_are_not_found() {
    let unknown = comic_info(
        r"<Series> </Series><Volume>3</Volume><Manga>Yes</Manga><AgeRating>Unknown</AgeRating>
          <LanguageISO>english</LanguageISO><Genre> , </Genre>",
    );
    assert_eq!(series(&unknown, true), SeriesMetadataPatch::default());
    let first_volume = comic_info("<Series>Saga</Series><Volume>1</Volume><Manga>No</Manga>");
    let patch = series(&first_volume, true);
    assert_eq!(patch.title.as_deref(), Some("Saga"));
    assert_eq!(patch.reading_direction, Some(ReadingDirection::LeftToRight));
}
