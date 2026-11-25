//! The token manager of Lucene's classic `QueryParser.jj`: at each position the longest token
//! wins, and on a tie the one declared first.

use super::InvalidQuery;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    And,
    Or,
    Not,
    Plus,
    Minus,
    /// `+`, `-` or `!` followed by whitespace; the parser treats it as a one-character term.
    BareOperator,
    LeftParen,
    RightParen,
    Colon,
    Star,
    Caret,
    Quoted,
    Term,
    FuzzySlop,
    PrefixTerm,
    WildTerm,
    RegexpTerm,
    RangeInclusiveStart,
    RangeExclusiveStart,
    Number,
    RangeTo,
    RangeInclusiveEnd,
    RangeExclusiveEnd,
    RangeQuoted,
    RangeGoop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub(super) kind: Kind,
    pub(super) image: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Default,
    Boost,
    Range,
}

/// Splits a query into tokens; any character that starts no token makes the query invalid.
pub(super) fn tokens(text: &str) -> Result<Vec<Token>, InvalidQuery> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut state = State::Default;
    let mut start = 0;
    while start < chars.len() {
        let rest = &chars[start..];
        let (kind, length) = match state {
            State::Default if is_whitespace(rest[0]) => {
                start += 1;
                continue;
            }
            State::Default => default_token(rest).ok_or(InvalidQuery)?,
            State::Boost => (Kind::Number, number(rest).ok_or(InvalidQuery)?),
            State::Range => match range_token(rest) {
                Some(token) => token,
                None if is_whitespace(rest[0]) => {
                    start += 1;
                    continue;
                }
                None => return Err(InvalidQuery),
            },
        };
        state = match kind {
            Kind::Caret => State::Boost,
            Kind::RangeInclusiveStart | Kind::RangeExclusiveStart => State::Range,
            Kind::Number | Kind::RangeInclusiveEnd | Kind::RangeExclusiveEnd => State::Default,
            _ => state,
        };
        tokens.push(Token {
            kind,
            image: rest[..length].iter().collect(),
        });
        start += length;
    }
    Ok(tokens)
}

fn default_token(rest: &[char]) -> Option<(Kind, usize)> {
    let term = term(rest);
    let candidates = [
        (Kind::And, keyword(rest, &["AND", "&&"])),
        (Kind::Or, keyword(rest, &["OR", "||"])),
        (Kind::Not, keyword(rest, &["NOT", "!"])),
        (Kind::Plus, keyword(rest, &["+"])),
        (Kind::Minus, keyword(rest, &["-"])),
        (Kind::BareOperator, bare_operator(rest)),
        (Kind::LeftParen, keyword(rest, &["("])),
        (Kind::RightParen, keyword(rest, &[")"])),
        (Kind::Colon, keyword(rest, &[":"])),
        (Kind::Star, keyword(rest, &["*"])),
        (Kind::Caret, keyword(rest, &["^"])),
        (Kind::Quoted, quoted(rest)),
        (Kind::Term, term),
        (Kind::FuzzySlop, fuzzy_slop(rest)),
        (Kind::PrefixTerm, prefix_term(rest, term)),
        (Kind::WildTerm, wild_term(rest)),
        (Kind::RegexpTerm, regexp_term(rest)),
        (Kind::RangeInclusiveStart, keyword(rest, &["["])),
        (Kind::RangeExclusiveStart, keyword(rest, &["{"])),
    ];
    longest(candidates)
}

/// In a range, whitespace is skipped unless it begins a longer `RANGE_GOOP`.
fn range_token(rest: &[char]) -> Option<(Kind, usize)> {
    let goop = rest
        .iter()
        .take_while(|&&ch| !matches!(ch, ' ' | ']' | '}'))
        .count();
    if is_whitespace(rest[0]) && goop <= 1 {
        return None;
    }
    let candidates = [
        (Kind::RangeTo, keyword(rest, &["TO"])),
        (Kind::RangeInclusiveEnd, keyword(rest, &["]"])),
        (Kind::RangeExclusiveEnd, keyword(rest, &["}"])),
        (Kind::RangeQuoted, range_quoted(rest)),
        (Kind::RangeGoop, (goop > 0).then_some(goop)),
    ];
    longest(candidates)
}

/// The first of the longest matches.
fn longest<const N: usize>(candidates: [(Kind, Option<usize>); N]) -> Option<(Kind, usize)> {
    let mut best: Option<(Kind, usize)> = None;
    for (kind, length) in candidates {
        if let Some(length) = length
            && best.is_none_or(|(_, longest)| length > longest)
        {
            best = Some((kind, length));
        }
    }
    best
}

fn is_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{3000}')
}

fn keyword(rest: &[char], images: &[&str]) -> Option<usize> {
    images
        .iter()
        .find(|image| {
            rest.iter()
                .copied()
                .take(image.chars().count())
                .eq(image.chars())
        })
        .map(|image| image.chars().count())
}

fn bare_operator(rest: &[char]) -> Option<usize> {
    (matches!(rest[0], '+' | '-' | '!') && rest.get(1).is_some_and(|&ch| is_whitespace(ch)))
        .then_some(2)
}

/// Length of `_TERM_START_CHAR` at the start of `rest`; an escape covers two characters.
fn term_start_char(rest: &[char]) -> Option<usize> {
    match rest.first()? {
        '\\' => rest.get(1).map(|_| 2),
        ' ' | '\t' | '\n' | '\r' | '\u{3000}' | '+' | '-' | '!' | '(' | ')' | ':' | '^' | '['
        | ']' | '"' | '{' | '}' | '~' | '*' | '?' | '/' => None,
        _ => Some(1),
    }
}

fn term_char(rest: &[char]) -> Option<usize> {
    match rest.first()? {
        '-' | '+' => Some(1),
        _ => term_start_char(rest),
    }
}

/// Length of the longest run of `_TERM_CHAR` at the start of `rest`.
fn term_chars(rest: &[char]) -> usize {
    let mut length = 0;
    while let Some(next) = term_char(&rest[length..]) {
        length += next;
    }
    length
}

fn term(rest: &[char]) -> Option<usize> {
    term_start_char(rest).map(|start| start + term_chars(&rest[start..]))
}

/// `"~" ((NUM+ ("." NUM+)? TERM_CHAR*) | TERM_CHAR*)`: digits and dots are term characters, so
/// this is `~` followed by term characters.
fn fuzzy_slop(rest: &[char]) -> Option<usize> {
    (rest[0] == '~').then(|| 1 + term_chars(&rest[1..]))
}

fn prefix_term(rest: &[char], term: Option<usize>) -> Option<usize> {
    match term {
        Some(term) => (rest.get(term) == Some(&'*')).then_some(term + 1),
        None => (rest[0] == '*').then_some(1),
    }
}

fn wild_term(rest: &[char]) -> Option<usize> {
    let wildcard = |rest: &[char]| matches!(rest.first(), Some('*' | '?')).then_some(1);
    let mut length = term_start_char(rest).or_else(|| wildcard(rest))?;
    while let Some(next) = term_char(&rest[length..]).or_else(|| wildcard(&rest[length..])) {
        length += next;
    }
    Some(length)
}

/// `"/" (~["/"] | "\\/")* "/"`: a slash after a backslash may either continue or end the
/// expression, and the longest match wins.
fn regexp_term(rest: &[char]) -> Option<usize> {
    if rest[0] != '/' {
        return None;
    }
    let mut end = None;
    for index in 1..rest.len() {
        if rest[index] == '/' {
            end = Some(index + 1);
            if index < 2 || rest[index - 1] != '\\' {
                break;
            }
        }
    }
    end
}

fn quoted(rest: &[char]) -> Option<usize> {
    if rest[0] != '"' {
        return None;
    }
    let mut index = 1;
    while index < rest.len() {
        match rest[index] {
            '"' => return Some(index + 1),
            '\\' if index + 1 < rest.len() => index += 2,
            '\\' => return None,
            _ => index += 1,
        }
    }
    None
}

/// `"\"" (~["\""] | "\\\"")+ "\""`.
fn range_quoted(rest: &[char]) -> Option<usize> {
    if rest[0] != '"' {
        return None;
    }
    let mut end = None;
    for index in 1..rest.len() {
        if rest[index] == '"' {
            if index > 1 {
                end = Some(index + 1);
            }
            if rest[index - 1] != '\\' {
                break;
            }
        }
    }
    end
}

/// `NUM+ ("." NUM+)?`.
fn number(rest: &[char]) -> Option<usize> {
    let digits = |rest: &[char]| rest.iter().take_while(|ch| ch.is_ascii_digit()).count();
    let integer = digits(rest);
    if integer == 0 {
        return None;
    }
    if rest.get(integer) == Some(&'.') {
        let fraction = digits(&rest[integer + 1..]);
        if fraction > 0 {
            return Some(integer + 1 + fraction);
        }
    }
    Some(integer)
}
