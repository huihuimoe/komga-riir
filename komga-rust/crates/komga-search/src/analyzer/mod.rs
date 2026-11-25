//! The analysis chains of `MultiLingualAnalyzer` (queries) and `MultiLingualNGramAnalyzer`
//! (indexing): `StandardTokenizer`, `CJKWidthFilter`, `LowerCaseFilter`, `CJKBigramFilter`, the
//! indexing-only `NGramTokenFilter`, then `ASCIIFoldingFilter` (Tantivy's port of Lucene's).

mod cjk_bigram;
mod cjk_width;
mod emoji;
mod ngram;
mod standard_tokenizer;
#[cfg(test)]
mod tests;

use std::ops::Range;

use icu_casemap::CaseMapper;
use tantivy::tokenizer::{
    AsciiFoldingFilter, PreTokenizedStream, PreTokenizedString, RawTokenizer, TextAnalyzer, Token,
    Tokenizer,
};

pub use ngram::NgramOptions;

/// Analyzer for indexed text, which also indexes CJK unigrams and the n-grams of each term.
pub(crate) fn index_analyzer(ngram: NgramOptions) -> TextAnalyzer {
    TextAnalyzer::builder(MultiLingualTokenizer { ngram: Some(ngram) })
        .filter(AsciiFoldingFilter)
        .build()
}

/// Analyzer for query text.
pub(crate) fn search_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(MultiLingualTokenizer { ngram: None })
        .filter(AsciiFoldingFilter)
        .build()
}

/// `MultiLingualAnalyzer.normalize`: the whole text as one term without splitting, for the
/// query terms Lucene does not analyze (wildcards, prefixes, ranges, fuzzy terms).
pub(crate) fn normalize(text: &str) -> String {
    let mut folding = TextAnalyzer::builder(RawTokenizer::default())
        .filter(AsciiFoldingFilter)
        .build();
    let lowercase = fold_width_and_case(text);
    let mut stream = folding.token_stream(&lowercase);
    stream
        .next()
        .map(|token| token.text.clone())
        .unwrap_or_default()
}

/// `CJKWidthFilter`, then `LowerCaseFilter`, which maps each code point alone like Java's
/// `Character.toLowerCase`, then hiragana to katakana so either kana finds the other (not in
/// Komga).
fn fold_width_and_case(text: &str) -> String {
    let case_mapper = CaseMapper::new();
    cjk_width::fold(text)
        .chars()
        .map(|ch| katakana(case_mapper.simple_lowercase(ch)))
        .collect()
}

/// The katakana of a hiragana letter or iteration mark; the voiced sound marks are shared.
fn katakana(ch: char) -> char {
    match ch {
        'ぁ'..='ゖ' | 'ゝ'..='ゟ' => {
            char::from_u32(u32::from(ch) + 0x60).expect("each hiragana has a katakana 0x60 later")
        }
        _ => ch,
    }
}

/// A term and the byte range of the source text it comes from.
struct Term {
    text: String,
    offsets: Range<usize>,
}

/// Every stage before ASCII folding, which must run after the n-grams as in Lucene: folding
/// changes lengths (`ß` becomes `ss`).
#[derive(Clone)]
struct MultiLingualTokenizer {
    ngram: Option<NgramOptions>,
}

impl Tokenizer for MultiLingualTokenizer {
    type TokenStream<'a> = PreTokenizedStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> PreTokenizedStream {
        let words = standard_tokenizer::words(text).into_iter().map(|word| {
            let term = Term {
                text: fold_width_and_case(&text[word.offsets.clone()]),
                offsets: word.offsets,
            };
            (term, word.cjk)
        });

        let mut tokens = Vec::new();
        // Indexing also keeps every CJK character, so one-character queries (`3月`) match.
        let unigrams = self.ngram.is_some();
        for (position, term) in cjk_bigram::bigrams(words, unigrams) {
            let texts = match self.ngram {
                Some(ngram) => ngram.grams(&term.text),
                None => vec![term.text],
            };
            // N-grams of a term share its position, as in Lucene.
            tokens.extend(texts.into_iter().map(|text| Token {
                offset_from: term.offsets.start,
                offset_to: term.offsets.end,
                position,
                text,
                position_length: 1,
            }));
        }
        PreTokenizedStream::from(PreTokenizedString {
            text: text.to_owned(),
            tokens,
        })
    }
}
