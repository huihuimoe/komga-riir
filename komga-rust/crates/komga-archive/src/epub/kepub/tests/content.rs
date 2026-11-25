//! Cases of kepubify v4.0.4 `transform_test.go`. kepubify renders its part cases without the
//! polyglot option; here every case is rendered as `transform_content` renders, so `script`,
//! `style`, and `svg` gain the attributes polyglot adds.

use super::super::content::{charset_utf8, clean, kobo_divs, kobo_spans, transform_content};
use super::super::dom::{self, DOCUMENT, Dom};
use super::super::render;

/// kepubify `transformContentCase.Run`: `part` applied to `input`, wrapped in a document when
/// `fragment` is set, then the rendered body content, or the whole document otherwise.
fn run(part: fn(&mut Dom), input: &str, fragment: bool) -> String {
    let document = if fragment {
        format!(
            r#"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta charset="utf-8"/></head><body>{input}</body></html>"#
        )
    } else {
        input.to_owned()
    };
    let mut dom = dom::parse(&document);
    part(&mut dom);
    let mut out = String::new();
    render::render(&dom, DOCUMENT, &mut out);
    if !fragment {
        return out;
    }
    let body = out.split_once("<body>").expect("Rendered body").1;
    body.split_once("</body>")
        .expect("Closed body")
        .0
        .to_owned()
}

fn span(id: &str, text: &str) -> String {
    format!(r#"<span class="koboSpan" id="kobo.{id}">{text}</span>"#)
}

#[test]
fn charset_meta_becomes_utf8() {
    let document = |meta: &str| {
        format!(
            r"<!DOCTYPE html><head><title>Kepubify Test Case</title>{meta}</head><body></body></html>"
        )
    };
    let cases = [
        (r#"<meta charset="iso-8859-1"/>"#, r#"meta charset="UTF-8""#),
        (r#"<meta charset="UTf-8"/>"#, r#"meta charset="UTf-8""#),
        (
            r#"<meta http-equiv="content-type" content="application/xhtml+xml; charset=iso-8859-1"/>"#,
            r#"meta http-equiv="content-type" content="application/xhtml+xml; charset=utf-8""#,
        ),
        (
            r#"<meta http-equiv="content-type" content="application/xhtml+xml; charset=UTf-8"/>"#,
            r#"meta http-equiv="content-type" content="application/xhtml+xml; charset=UTf-8""#,
        ),
    ];
    for (meta, expected) in cases {
        let out = run(charset_utf8, &document(meta), false);
        assert!(out.contains(expected), "{meta} rendered as {out}");
    }
}

#[test]
fn body_is_wrapped_in_kobo_divs_once() {
    let wrapped =
        |inner: &str| format!(r#"<div id="book-columns"><div id="book-inner">{inner}</div></div>"#);
    for inner in ["", "test", "<p>Test 1</p><p>Test <b>2</b></p><p>Test 3</p>"] {
        assert_eq!(run(kobo_divs, inner, true), wrapped(inner));
    }
    assert_eq!(run(kobo_divs, &wrapped(""), true), wrapped(""));
}

#[test]
fn sentences_and_paragraphs_get_kobo_spans() {
    let cases = [
        (String::new(), String::new()),
        (
            format!("<p>{}</p>", span("1.1", "Test")),
            format!("<p>{}</p>", span("1.1", "Test")),
        ),
        (
            "<p>Sentence 1. Sentence 2. Sentence 3.</p>".to_owned(),
            format!("<p>{}{}{}</p>", span("1.1", "Sentence 1. "), span("1.2", "Sentence 2. "), span("1.3", "Sentence 3.")),
        ),
        (
            "<p>Sentence 1. Sentence 2.</p><p>Sentence 3.</p><ul><li>Sentence 4</li><li>Sentence 5</li></ul><ol><li>Sentence 6</li><li>Sentence 7</li></ol><table><tbody><tr><td>Test</td></tr><tr><td>Test</td></tr></tbody></table>".to_owned(),
            format!(
                "<p>{}{}</p><p>{}</p><ul><li>{}</li><li>{}</li></ul><ol><li>{}</li><li>{}</li></ol><table><tbody><tr><td>{}</td></tr><tr><td>{}</td></tr></tbody></table>",
                span("1.1", "Sentence 1. "), span("1.2", "Sentence 2."), span("2.1", "Sentence 3."),
                span("3.1", "Sentence 4"), span("3.2", "Sentence 5"), span("4.1", "Sentence 6"),
                span("4.2", "Sentence 7"), span("5.1", "Test"), span("5.2", "Test"),
            ),
        ),
        (
            "<p>Sentence 1. Sentence 2. Stray text\nAnother sentence.</p>".to_owned(),
            format!("<p>{}{}{}</p>", span("1.1", "Sentence 1. "), span("1.2", "Sentence 2. "), span("1.3", "Stray text\nAnother sentence.")),
        ),
        (
            "<p>This is a test.\n    This is another sentence on the next line.<span> </span>Another sentence.</p>\n    <p>Another paragraph.</p><p> </p><p></p>".to_owned(),
            format!(
                "<p>{}{}<span> </span>{}</p>\n    <p>{}</p><p>{}</p><p></p>",
                span("1.1", "This is a test.\n    "), span("1.2", "This is another sentence on the next line."),
                span("1.3", "Another sentence."), span("2.1", "Another paragraph."), span("3.1", " "),
            ),
        ),
        (
            r#"<p>Sentence<b> 1. Sente<i>nce <span>2. Se</span>nt</i>en<a href="test.html">ce 3. Another word</a></b></p>"#.to_owned(),
            format!(
                r#"<p>{}<b>{}{}<i>{}<span>{}{}</span>{}</i>{}<a href="test.html">{}{}</a></b></p>"#,
                span("1.1", "Sentence"), span("1.2", " 1. "), span("1.3", "Sente"), span("1.4", "nce "),
                span("1.5", "2. "), span("1.6", "Se"), span("1.7", "nt"), span("1.8", "en"),
                span("1.9", "ce 3. "), span("1.10", "Another word"),
            ),
        ),
        (
            "<p>Sentence.<ul><li>Another sentence.</li><li>Another sentence.<ul><li>Another sentence.</li><li>Another sentence.</li></ul></li><li>Another sentence.</li></ul> Another sentence.</p>".to_owned(),
            format!(
                "<p>{}</p><ul><li>{}</li><li>{}<ul><li>{}</li><li>{}</li></ul></li><li>{}</li></ul>{}<p></p>",
                span("1.1", "Sentence."), span("2.1", "Another sentence."), span("2.2", "Another sentence."),
                span("3.1", "Another sentence."), span("3.2", "Another sentence."), span("3.3", "Another sentence."),
                span("3.4", " Another sentence."),
            ),
        ),
        (
            "<p>Touch this.</p><script>not this</script><style>or this</style><pre>or this</pre><audio>or this</audio><video>or this</video><p>Touch this.</p>".to_owned(),
            format!(
                r#"<p>{}</p><script type="text/javascript">not this</script><style type="text/css">or this</style><pre>or this</pre><audio>or this</audio><video>or this</video><p>{}</p>"#,
                span("1.1", "Touch this."), span("2.1", "Touch this."),
            ),
        ),
        (
            r#"<p>One.</p><img src="test"><p>Three.</p>"#.to_owned(),
            format!(r#"<p>{}</p>{}<p>{}</p>"#, span("1.1", "One."), span("2.1", r#"<img src="test"/>"#), span("3.1", "Three.")),
        ),
        (
            "<p>One.</p><p> </p><p><!-- comment --></p><p>Two.</p><p><b>Three.</b></p>".to_owned(),
            format!(
                "<p>{}</p><p>{}</p><p><!-- comment --></p><p>{}</p><p><b>{}</b></p>",
                span("1.1", "One."), span("2.1", " "), span("3.1", "Two."), span("4.1", "Three."),
            ),
        ),
        (
            r#"<svg xmlns="http://www.w3.org/2000/svg"><g><text font-size="24" y="20" x="0">kepubify</text></g></svg><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi><mo>=</mo></math>"#.to_owned(),
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g><text font-size="24" y="20" x="0">kepubify</text></g></svg><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi><mo>=</mo></math>"#.to_owned(),
        ),
        (
            "<div class=\"img_container\"><p class=\"ad_image\"><img src=\"c.jpg\" alt=\"image\"/></p></div>\n    <h2 class=\"subheadline\">The Christmas Collection: All Of Your Favourite</h2>\n    <p class=\"subheadline2\"></p>\n    <p class=\"metadata\">Carr, Annie Roe</p>".to_owned(),
            format!(
                "<div class=\"img_container\"><p class=\"ad_image\">{}</p></div>\n    <h2 class=\"subheadline\">{}</h2>\n    <p class=\"subheadline2\"></p>\n    <p class=\"metadata\">{}</p>",
                span("1.1", "<img src=\"c.jpg\" alt=\"image\"/>"), span("2.1", "The Christmas Collection: All Of Your Favourite"),
                span("3.1", "Carr, Annie Roe"),
            ),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(run(kobo_spans, &input, true), expected, "Spans of {input}");
    }
}

#[test]
fn cleanup_removes_adept_word_and_replacement_characters() {
    let cases = [
        (
            r#"<meta charset="utf-8"/><meta name="Adept.expected.resource"/>"#,
            r#"<meta charset="utf-8"/>"#,
        ),
        (
            "<o:p></o:p><st1:test></st1:test><div><o:p> dfg </o:p><o:p></o:p></div>",
            "<div><o:p> dfg </o:p></div>",
        ),
        (
            "\u{fffd}<p>test\u{fffd}ing</p><p>asd<b>fgh</b></p>",
            "<p>testing</p><p>asd<b>fgh</b></p>",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(run(clean, input, true), expected, "Cleanup of {input}");
    }
}

/// kepubify `TestTransformContent` without its non-default options (smart punctuation,
/// find/replace, extra CSS).
#[test]
fn whole_document_matches_kepubify() {
    let input = r#"<?xml version="1.0" charset="utf-8"?>
<!DOCTYPE html>
<html>
<head>
    <title>Kepubify Test</title>
    <meta charset="utf-8">
</head>
<body>
    <p>Test sentence 1. <a id="test"/> Test sentence 2. <b>Test sentence 3<i>Test sentence 4</p>
    <p>Test sentence 5. <i>"This is quoted"</i> -- <b>and this is not</b>.</p>
    <pre>Test
</pre>
    <table borders><tr><td>test</td></tr></table>
    <p>  </p>
    <img src="test">
    <p>&nbsp;</p>
    <svg></svg>
</body>
</html>"#;
    let expected = format!(
        r#"<?xml version="1.0" charset="utf-8"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head>
    <title>Kepubify Test</title>
    <meta charset="utf-8"/>
<style type="text/css" class="kobostylehacks">div#book-inner {{ margin-top: 0; margin-bottom: 0;}}</style></head>
<body><div id="book-columns"><div id="book-inner">
    <p>{}<a id="test"></a>{}<b>{}<i>{}</i></b></p><b><i>
    <p>{}<i>{}</i>{}<b>{}</b>{}</p>
    <pre>Test
</pre>
    <table borders=""><tbody><tr><td>{}</td></tr></tbody></table>
    <p>{}</p>
    {}
    <p>{}</p>
    <svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"></svg>

</i></b></div></div></body></html>"#,
        span("1.1", "Test sentence 1. "),
        span("1.2", " Test sentence 2. "),
        span("1.3", "Test sentence 3"),
        span("1.4", "Test sentence 4"),
        span("2.1", "Test sentence 5. "),
        span("2.2", "&#34;This is quoted&#34;"),
        span("2.3", " -- "),
        span("2.4", "and this is not"),
        span("2.5", "."),
        span("3.1", "test"),
        span("4.1", "  "),
        span("5.1", r#"<img src="test"/>"#),
        span("6.1", "&#160;"),
    );
    let out = transform_content(input.as_bytes());
    assert_eq!(out.trim(), expected);
    assert_eq!(
        transform_content(out.as_bytes())
            .matches("koboSpan")
            .count(),
        13,
        "Spans are added once"
    );
}
