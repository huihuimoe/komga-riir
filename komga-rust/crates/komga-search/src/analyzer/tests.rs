use tantivy::tokenizer::TextAnalyzer;

use super::{NgramOptions, index_analyzer, search_analyzer};

/// Inputs with the query and index terms Lucene 9.9.1 emits for them (`position:term`), produced
/// by Komga's two analyzers with the default n-gram settings, the index one with
/// `CJKBigramFilter` `outputUnigrams`, then hiragana folded to katakana.
const LUCENE_TERMS: &[(&str, &str, &str)] = &[
    (
        "The Amazing Spider-Man",
        "0:the 1:amazing 2:spider 3:man",
        "0:the 1:ama 1:amaz 1:amazi 1:amazin 1:amazing 1:maz 1:mazi 1:mazin 1:mazing 1:azi 1:azin 1:azing 1:zin 1:zing 1:ing 2:spi 2:spid 2:spide 2:spider 2:pid 2:pide 2:pider 2:ide 2:ider 2:der 3:man",
    ),
    (
        "Café Crème déjà vu",
        "0:cafe 1:creme 2:deja 3:vu",
        "0:caf 0:cafe 0:afe 1:cre 1:crem 1:creme 1:rem 1:reme 1:eme 2:dej 2:deja 2:eja 3:vu",
    ),
    (
        "don't U.S.A. 3.14 e-mail foo_bar",
        "0:don't 1:u.s.a 2:3.14 3:e 4:mail 5:foo_bar",
        "0:don 0:don' 0:don't 0:on' 0:on't 0:n't 1:u.s 1:u.s. 1:u.s.a 1:.s. 1:.s.a 1:s.a 2:3.1 2:3.14 2:.14 3:e 4:mai 4:mail 4:ail 5:foo 5:foo_ 5:foo_b 5:foo_ba 5:foo_bar 5:oo_ 5:oo_b 5:oo_ba 5:oo_bar 5:o_b 5:o_ba 5:o_bar 5:_ba 5:_bar 5:bar",
    ),
    (
        "Ｆｕｌｌｗｉｄｔｈ ＡＢＣ１２３",
        "0:fullwidth 1:abc123",
        "0:ful 0:full 0:fullw 0:fullwi 0:fullwid 0:fullwidt 0:fullwidth 0:ull 0:ullw 0:ullwi 0:ullwid 0:ullwidt 0:ullwidth 0:llw 0:llwi 0:llwid 0:llwidt 0:llwidth 0:lwi 0:lwid 0:lwidt 0:lwidth 0:wid 0:widt 0:width 0:idt 0:idth 0:dth 1:abc 1:abc1 1:abc12 1:abc123 1:bc1 1:bc12 1:bc123 1:c12 1:c123 1:123",
    ),
    (
        "ワンピース\u{3000}１０巻",
        "0:ワン 1:ンピ 2:ピー 3:ース 4:10 5:巻",
        "0:ワ 0:ワン 1:ン 1:ンピ 2:ピ 2:ピー 3:ー 3:ース 4:ス 5:10 6:巻",
    ),
    ("１．５", "0:1.5", "0:1.5"),
    (
        "ｶﾞﾝﾀﾞﾑ ﾊﾟﾝ ﾞ",
        "0:ガン 1:ンダ 2:ダム 3:パン",
        "0:ガ 0:ガン 1:ン 1:ンダ 2:ダ 2:ダム 3:ム 4:パ 4:パン 5:ン",
    ),
    (
        "進撃の巨人",
        "0:進撃 1:撃ノ 2:ノ巨 3:巨人",
        "0:進 0:進撃 1:撃 1:撃ノ 2:ノ 2:ノ巨 3:巨 3:巨人 4:人",
    ),
    (
        "日本語カタカナ",
        "0:日本 1:本語 2:語カ 3:カタ 4:タカ 5:カナ",
        "0:日 0:日本 1:本 1:本語 2:語 2:語カ 3:カ 3:カタ 4:タ 4:タカ 5:カ 5:カナ 6:ナ",
    ),
    (
        "らーめん",
        "0:ラー 1:ーメ 2:メン",
        "0:ラ 0:ラー 1:ー 1:ーメ 2:メ 2:メン 3:ン",
    ),
    (
        "한국어 만화 한국2",
        "0:한국 1:국어 2:만화 3:한국2",
        "0:한 0:한국 1:국 1:국어 2:어 3:만 3:만화 4:화 5:한국2",
    ),
    (
        "ภาษาไทย ทดสอบ",
        "0:ภาษาไทย 1:ทดสอบ",
        "0:ภาษ 0:ภาษา 0:ภาษาไ 0:ภาษาไท 0:ภาษาไทย 0:าษา 0:าษาไ 0:าษาไท 0:าษาไทย 0:ษาไ 0:ษาไท 0:ษาไทย 0:าไท 0:าไทย 0:ไทย 1:ทดส 1:ทดสอ 1:ทดสอบ 1:ดสอ 1:ดสอบ 1:สอบ",
    ),
    (
        "İstanbul ǅemal ΣΑΣ",
        "0:istanbul 1:dzemal 2:σασ",
        "0:ist 0:ista 0:istan 0:istanb 0:istanbu 0:istanbul 0:sta 0:stan 0:stanb 0:stanbu 0:stanbul 0:tan 0:tanb 0:tanbu 0:tanbul 0:anb 0:anbu 0:anbul 0:nbu 0:nbul 0:bul 1:dzem 1:dzema 1:dzemal 1:ema 1:emal 1:mal 2:σασ",
    ),
    (
        "Æon straße ﬁle",
        "0:aeon 1:strasse 2:file",
        "0:aeon 1:str 1:stra 1:strass 1:strasse 1:tra 1:trass 1:trasse 1:rass 1:rasse 1:asse 2:file",
    ),
    (
        "Batman 🦇 🇯🇵 1\u{fe0f}\u{20e3} © ©\u{fe0f}",
        "0:batman 1:🦇 2:🇯🇵 3:1\u{fe0f}\u{20e3} 4:© 5:©\u{fe0f}",
        "0:bat 0:batm 0:batma 0:batman 0:atm 0:atma 0:atman 0:tma 0:tman 0:man 1:🦇 2:🇯🇵 3:1\u{fe0f}\u{20e3} 4:© 5:©\u{fe0f}",
    ),
    (
        "abcdefghijklmnop",
        "0:abcdefghijklmnop",
        "0:abc 0:abcd 0:abcde 0:abcdef 0:abcdefg 0:abcdefgh 0:abcdefghi 0:abcdefghij 0:bcd 0:bcde 0:bcdef 0:bcdefg 0:bcdefgh 0:bcdefghi 0:bcdefghij 0:bcdefghijk 0:cde 0:cdef 0:cdefg 0:cdefgh 0:cdefghi 0:cdefghij 0:cdefghijk 0:cdefghijkl 0:def 0:defg 0:defgh 0:defghi 0:defghij 0:defghijk 0:defghijkl 0:defghijklm 0:efg 0:efgh 0:efghi 0:efghij 0:efghijk 0:efghijkl 0:efghijklm 0:efghijklmn 0:fgh 0:fghi 0:fghij 0:fghijk 0:fghijkl 0:fghijklm 0:fghijklmn 0:fghijklmno 0:ghi 0:ghij 0:ghijk 0:ghijkl 0:ghijklm 0:ghijklmn 0:ghijklmno 0:ghijklmnop 0:hij 0:hijk 0:hijkl 0:hijklm 0:hijklmn 0:hijklmno 0:hijklmnop 0:ijk 0:ijkl 0:ijklm 0:ijklmn 0:ijklmno 0:ijklmnop 0:jkl 0:jklm 0:jklmn 0:jklmno 0:jklmnop 0:klm 0:klmn 0:klmno 0:klmnop 0:lmn 0:lmno 0:lmnop 0:mno 0:mnop 0:nop 0:abcdefghijklmnop",
    ),
    ("x", "0:x", "0:x"),
    ("ab", "0:ab", "0:ab"),
    (
        "9781234567897 978-1-234-56789-7",
        "0:9781234567897 1:978 2:1 3:234 4:56789 5:7",
        "0:978 0:9781 0:97812 0:978123 0:9781234 0:97812345 0:978123456 0:9781234567 0:781 0:7812 0:78123 0:781234 0:7812345 0:78123456 0:781234567 0:7812345678 0:812 0:8123 0:81234 0:812345 0:8123456 0:81234567 0:812345678 0:8123456789 0:123 0:1234 0:12345 0:123456 0:1234567 0:12345678 0:123456789 0:1234567897 0:234 0:2345 0:23456 0:234567 0:2345678 0:23456789 0:234567897 0:345 0:3456 0:34567 0:345678 0:3456789 0:34567897 0:456 0:4567 0:45678 0:456789 0:4567897 0:567 0:5678 0:56789 0:567897 0:678 0:6789 0:67897 0:789 0:7897 0:897 0:9781234567897 1:978 2:1 3:234 4:567 4:5678 4:56789 4:678 4:6789 4:789 5:7",
    ),
    (
        "漢字abc漢字",
        "0:漢字 1:abc 2:漢字",
        "0:漢 0:漢字 1:字 2:abc 3:漢 3:漢字 4:字",
    ),
    (
        "#\u{fe0f}\u{20e3} ★ ♥ ＠ _ ... C++ c# AT&T",
        "0:#\u{fe0f}\u{20e3} 1:★ 2:♥ 3:c 4:c 5:at 6:t",
        "0:#\u{fe0f}\u{20e3} 1:★ 2:♥ 3:c 4:c 5:at 6:t",
    ),
    (
        "o'neil l'amour 1,000 v1.2.3 x_1 カ_タ",
        "0:o'neil 1:l'amour 2:1,000 3:v1.2.3 4:x_1 5:カ_タ",
        "0:o'n 0:o'ne 0:o'nei 0:o'neil 0:'ne 0:'nei 0:'neil 0:nei 0:neil 0:eil 1:l'a 1:l'am 1:l'amo 1:l'amou 1:l'amour 1:'am 1:'amo 1:'amou 1:'amour 1:amo 1:amou 1:amour 1:mou 1:mour 1:our 2:1,0 2:1,00 2:1,000 2:,00 2:,000 2:000 3:v1. 3:v1.2 3:v1.2. 3:v1.2.3 3:1.2 3:1.2. 3:1.2.3 3:.2. 3:.2.3 3:2.3 4:x_1 5:カ_タ",
    ),
    (
        "a.１ １a ａ１ ｱｲｳ ﾗｰﾒﾝ ＡＢＣ－１２３ ゝ",
        "0:a 1:1 2:1a 3:a1 4:アイ 5:イウ 6:ラー 7:ーメ 8:メン 9:abc 10:123 11:ヽ",
        "0:a 1:1 2:1a 3:a1 4:ア 4:アイ 5:イ 5:イウ 6:ウ 7:ラ 7:ラー 8:ー 8:ーメ 9:メ 9:メン 10:ン 11:abc 12:123 13:ヽ",
    ),
    (
        "αβγ שלום مرحبا Ⅻ ½ ① हिन\u{94d}दी 👨\u{200d}👩\u{200d}👧 👍🏽",
        "0:αβγ 1:שלום 2:مرحبا 3:ⅻ 4:हिन\u{94d}दी 5:👨\u{200d}👩\u{200d}👧 6:👍🏽",
        "0:αβγ 1:שלו 1:שלום 1:לום 2:مرح 2:مرحب 2:مرحبا 2:رحب 2:رحبا 2:حبا 3:ⅻ 4:हिन 4:हिन\u{94d} 4:हिन\u{94d}द 4:हिन\u{94d}दी 4:िन\u{94d} 4:िन\u{94d}द 4:िन\u{94d}दी 4:न\u{94d}द 4:न\u{94d}दी 4:\u{94d}दी 5:👨\u{200d}👩 5:👨\u{200d}👩\u{200d} 5:👨\u{200d}👩\u{200d}👧 5:\u{200d}👩\u{200d} 5:\u{200d}👩\u{200d}👧 5:👩\u{200d}👧 6:👍🏽",
    ),
    (
        "★ ♪ ☆ 🇯 🇯🇵 👍🏽 🏽 *\u{fe0f}\u{20e3}",
        "0:★ 1:♪ 2:🇯🇵 3:👍🏽 4:🏽 5:*\u{fe0f}\u{20e3}",
        "0:★ 1:♪ 2:🇯🇵 3:👍🏽 4:🏽 5:*\u{fe0f}\u{20e3}",
    ),
    (
        "人々 a々 ทดสอบ",
        "0:人 1:々 2:a々 3:ทดสอบ",
        "0:人 1:々 2:a々 3:ทดส 3:ทดสอ 3:ทดสอบ 3:ดสอ 3:ดสอบ 3:สอบ",
    ),
    (
        "3月のライオン",
        "0:3 1:月ノ 2:ノラ 3:ライ 4:イオ 5:オン",
        "0:3 1:月 1:月ノ 2:ノ 2:ノラ 3:ラ 3:ライ 4:イ 4:イオ 5:オ 5:オン 6:ン",
    ),
    ("3月", "0:3 1:月", "0:3 1:月"),
    (
        "3月的狮子",
        "0:3 1:月的 2:的狮 3:狮子",
        "0:3 1:月 1:月的 2:的 2:的狮 3:狮 3:狮子 4:子",
    ),
    ("第3巻", "0:第 1:3 2:巻", "0:第 1:3 2:巻"),
];

fn terms(mut analyzer: TextAnalyzer, text: &str) -> String {
    let mut stream = analyzer.token_stream(text);
    let mut terms = Vec::new();
    while let Some(token) = stream.next() {
        terms.push(format!("{}:{}", token.position, token.text));
    }
    terms.join(" ")
}

#[test]
fn analyzers_emit_the_terms_of_lucene() {
    for (text, query, index) in LUCENE_TERMS {
        assert_eq!(
            terms(search_analyzer(), text),
            *query,
            "query terms of {text:?}"
        );
        assert_eq!(
            terms(index_analyzer(NgramOptions::default()), text),
            *index,
            "index terms of {text:?}"
        );
    }
}

#[test]
fn words_longer_than_255_characters_are_split() {
    let word = "a".repeat(300);

    let split = terms(search_analyzer(), &word);

    assert_eq!(split, format!("0:{} 1:{}", "a".repeat(255), "a".repeat(45)));
}

#[test]
fn ngram_options_follow_the_configuration() {
    let no_original = NgramOptions::new(2, 3, false).expect("2..=3 is a valid gram range");

    assert_eq!(
        terms(index_analyzer(no_original), "abcd x"),
        "0:ab 0:abc 0:bc 0:bcd 0:cd"
    );
    assert_eq!(NgramOptions::new(0, 3, true), None);
    assert_eq!(NgramOptions::new(4, 3, true), None);
}
