//! `NGramTokenFilter`.

/// N-gram settings of the index analyzer (`komga.lucene.index-analyzer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NgramOptions {
    pub(crate) min_gram: usize,
    pub(crate) max_gram: usize,
    pub(crate) preserve_original: bool,
}

impl NgramOptions {
    /// Returns `None` unless `1 <= min_gram <= max_gram`, which Lucene requires.
    pub fn new(min_gram: usize, max_gram: usize, preserve_original: bool) -> Option<Self> {
        (1 <= min_gram && min_gram <= max_gram).then_some(Self {
            min_gram,
            max_gram,
            preserve_original,
        })
    }

    /// Grams by start, then by length; a term shorter or longer than every gram is kept whole
    /// when `preserve_original` is set, and otherwise dropped.
    pub(super) fn grams(self, term: &str) -> Vec<String> {
        let mut bounds: Vec<usize> = term.char_indices().map(|(index, _)| index).collect();
        let length = bounds.len();
        bounds.push(term.len());

        let mut grams = Vec::new();
        for start in 0..length {
            let longest = self.max_gram.min(length - start);
            for size in self.min_gram..=longest {
                grams.push(term[bounds[start]..bounds[start + size]].to_owned());
            }
        }
        if self.preserve_original && !(self.min_gram..=self.max_gram).contains(&length) {
            grams.push(term.to_owned());
        }
        grams
    }
}

/// `KomgaProperties.Lucene.IndexAnalyzer` defaults.
impl Default for NgramOptions {
    fn default() -> Self {
        Self {
            min_gram: 3,
            max_gram: 10,
            preserve_original: true,
        }
    }
}
