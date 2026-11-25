//! `CJKBigramFilter` over Han, Hiragana, Katakana and Hangul: each run of adjacent words of those
//! scripts becomes overlapping bigrams of its characters, and a lone character stays a unigram.

use std::ops::Range;

use super::Term;

/// Terms with their positions; `words` pairs each word with whether it is one of the bigrammed
/// scripts. With `unigrams` (`outputUnigrams`), every character of a run is also a term, and each
/// bigram shares the position of its first character.
pub(super) fn bigrams(
    words: impl IntoIterator<Item = (Term, bool)>,
    unigrams: bool,
) -> Vec<(usize, Term)> {
    let mut terms = Terms {
        terms: Vec::new(),
        position: 0,
        unigrams,
    };
    let mut run: Vec<(char, Range<usize>)> = Vec::new();
    for (word, cjk) in words {
        let adjacent = run
            .last()
            .is_some_and(|(_, offsets)| offsets.end == word.offsets.start);
        if !cjk || !adjacent {
            terms.flush(&mut run);
        }
        if cjk {
            run.extend(word.text.chars().map(|ch| (ch, word.offsets.clone())));
        } else {
            terms.terms.push((terms.position, word));
            terms.position += 1;
        }
    }
    terms.flush(&mut run);
    terms.terms
}

struct Terms {
    terms: Vec<(usize, Term)>,
    /// Position of the next term.
    position: usize,
    unigrams: bool,
}

impl Terms {
    /// Character `index` of a run is at the run's position plus `index`, as are the unigram and
    /// the bigram it starts.
    fn flush(&mut self, run: &mut Vec<(char, Range<usize>)>) {
        let with_unigrams = self.unigrams || run.len() == 1;
        for (index, (ch, offsets)) in run.iter().enumerate() {
            let position = self.position + index;
            if with_unigrams {
                let unigram = Term {
                    text: ch.to_string(),
                    offsets: offsets.clone(),
                };
                self.terms.push((position, unigram));
            }
            if let Some((next, next_offsets)) = run.get(index + 1) {
                let bigram = Term {
                    text: [*ch, *next].iter().collect(),
                    offsets: offsets.start..next_offsets.end,
                };
                self.terms.push((position, bigram));
            }
        }
        self.position += if with_unigrams {
            run.len()
        } else {
            run.len().saturating_sub(1)
        };
        run.clear();
    }
}
