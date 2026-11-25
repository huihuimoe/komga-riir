use super::{ComicInfo, ComicInfoError, parse};

#[test]
fn root_children_fill_fields_and_others_are_ignored() {
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Title>  A &amp; B &#x263A; </Title>
  <Summary><![CDATA[<b>bold</b>]]></Summary>
  <Year> 2020 </Year>
  <Month/>
  <Number>1</Number>
  <Number>2</Number>
  <Pages><Page Image="0"><Title>ignored</Title></Page></Pages>
  <Unknown>x</Unknown>
</ComicInfo>"#;
    assert_eq!(
        parse(xml.as_bytes()).unwrap(),
        ComicInfo {
            title: Some("  A & B \u{263a} ".to_owned()),
            summary: Some("<b>bold</b>".to_owned()),
            year: Some(2020),
            month: None,
            number: Some("2".to_owned()),
            ..ComicInfo::default()
        }
    );
}

#[test]
fn utf16_with_a_byte_order_mark_is_read() {
    let mut content = vec![0xff, 0xfe];
    for unit in "<ComicInfo><Series>é</Series></ComicInfo>".encode_utf16() {
        content.extend(unit.to_le_bytes());
    }
    assert_eq!(parse(&content).unwrap().series.as_deref(), Some("é"));
}

#[test]
fn malformed_documents_are_rejected() {
    for xml in [
        "",
        "<ComicInfo><Title>x</ComicInfo>",
        "<ComicInfo><Title>x</Title>",
        "<ComicInfo><Title>&nbsp;</Title></ComicInfo>",
        "<ComicInfo><Year>2020a</Year></ComicInfo>",
        "<ComicInfo><PageCount>many</PageCount></ComicInfo>",
    ] {
        assert!(parse(xml.as_bytes()).is_err(), "{xml:?}");
    }
    assert!(matches!(
        parse(b"<ComicInfo><Count>1.0</Count></ComicInfo>"),
        Err(ComicInfoError::Integer {
            element: "Count",
            ..
        })
    ));
    assert!(matches!(parse(b"\xc3\x28"), Err(ComicInfoError::Encoding)));
}

#[test]
fn blank_or_null_integers_have_no_value() {
    let comic_info = parse(b"<ComicInfo><Volume> </Volume><Day>null</Day></ComicInfo>").unwrap();
    assert_eq!((comic_info.volume, comic_info.day), (None, None));
}
