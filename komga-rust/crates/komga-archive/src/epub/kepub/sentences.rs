//! kepubify `splitSentences`: text cut after sentence-ending punctuation, an optional closing
//! quote, and the whitespace that follows.

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    InSentence,
    AfterPunct,
    AfterPunctExtra,
    AfterSpace,
}

/// The pieces of `text`, which join back into it; an empty text is one empty piece.
pub(super) fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut state = State::InSentence;
    for (index, char) in text.char_indices() {
        let punct = matches!(char, '.' | '!' | '?');
        let extra = matches!(char, '\'' | '"' | '”' | '’' | '“' | '…');
        // Go's `\s`: ASCII whitespace only.
        let space = matches!(char, '\t' | '\n' | '\x0c' | '\r' | ' ');
        if state == State::AfterSpace && !space {
            sentences.push(&text[start..index]);
            start = index;
        }
        state = match state {
            _ if punct => State::AfterPunct,
            State::AfterPunct if extra => State::AfterPunctExtra,
            State::AfterPunct | State::AfterPunctExtra | State::AfterSpace if space => {
                State::AfterSpace
            }
            _ => State::InSentence,
        };
    }
    if start < text.len() || sentences.is_empty() {
        sentences.push(&text[start..]);
    }
    sentences
}

#[cfg(test)]
mod tests {
    use super::split_sentences;

    #[test]
    fn splits_like_kepubify() {
        let cases: [(&str, &[&str]); 9] = [
            ("", &[""]),
            ("No punctuation", &["No punctuation"]),
            ("One. Two! Three?", &["One. ", "Two! ", "Three?"]),
            ("Quoted.\" Next", &["Quoted.\" ", "Next"]),
            ("Twice.\"\" Next", &["Twice.\"\" Next"]),
            ("Spaced.\n\t  Next.  ", &["Spaced.\n\t  ", "Next.  "]),
            ("1.5 and e.g.this", &["1.5 and e.g.this"]),
            ("Wow!?! Yes", &["Wow!?! ", "Yes"]),
            ("End.\u{a0}NBSP", &["End.\u{a0}NBSP"]),
        ];
        for (text, expected) in cases {
            assert_eq!(split_sentences(text), expected, "Split of {text:?}");
        }
    }
}
