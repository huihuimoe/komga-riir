//! The grammar of Lucene's classic `QueryParser.jj` with the clause rules of `QueryParserBase`,
//! run as Komga runs it: AND as the default operator.

use std::ops::Bound;

use super::InvalidQuery;
use super::analysis::Analysis;
use super::lexer::{Kind, Token};
use super::node::{Node, Occur};

/// Groups nested deeper than this are rejected; Lucene would recurse until the stack overflows.
const MAX_DEPTH: usize = 100;

/// Lucene's `fuzzyMinSim` default, `FuzzyQuery.defaultMaxEdits`.
const DEFAULT_FUZZY_SIMILARITY: f32 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Conjunction {
    None,
    And,
    Or,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modifier {
    None,
    Required,
    Prohibited,
}

/// What follows a bare term: `~slop` and `^boost`, in either order.
#[derive(Default)]
struct Suffixes {
    fuzzy_slop: Option<String>,
    boost: Option<String>,
}

pub(super) struct Parser<'a> {
    tokens: Vec<Token>,
    next: usize,
    depth: usize,
    analysis: Analysis<'a>,
}

impl<'a> Parser<'a> {
    pub(super) fn new(tokens: Vec<Token>, analysis: Analysis<'a>) -> Self {
        Self {
            tokens,
            next: 0,
            depth: 0,
            analysis,
        }
    }

    /// `TopLevelQuery`: the query, then the end of input.
    pub(super) fn parse(mut self) -> Result<Option<Node>, InvalidQuery> {
        let query = self.query(None)?;
        if self.next != self.tokens.len() {
            return Err(InvalidQuery);
        }
        Ok(query)
    }

    fn peek(&self, offset: usize) -> Option<Kind> {
        self.tokens.get(self.next + offset).map(|token| token.kind)
    }

    fn take(&mut self, kinds: &[Kind]) -> Option<Token> {
        let token = self.tokens.get(self.next)?;
        if !kinds.contains(&token.kind) {
            return None;
        }
        self.next += 1;
        Some(token.clone())
    }

    fn expect(&mut self, kinds: &[Kind]) -> Result<Token, InvalidQuery> {
        self.take(kinds).ok_or(InvalidQuery)
    }

    /// `Query`: clauses until a token that cannot start one. `field` is `None` for the default
    /// fields.
    fn query(&mut self, field: Option<&str>) -> Result<Option<Node>, InvalidQuery> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(InvalidQuery);
        }
        let mut clauses = Vec::new();
        let modifier = self.modifier();
        let first = self.clause(field)?;
        let first_query = (modifier == Modifier::None)
            .then(|| first.clone())
            .flatten();
        add_clause(&mut clauses, Conjunction::None, modifier, first);
        while self.peek(0).is_some_and(starts_clause) {
            let conjunction = self.conjunction();
            let modifier = self.modifier();
            let query = self.clause(field)?;
            add_clause(&mut clauses, conjunction, modifier, query);
        }
        self.depth -= 1;
        match first_query {
            Some(query) if clauses.len() == 1 => Ok(Some(query)),
            _ => Ok((!clauses.is_empty()).then_some(Node::Boolean(clauses))),
        }
    }

    fn conjunction(&mut self) -> Conjunction {
        match self.take(&[Kind::And, Kind::Or]).map(|token| token.kind) {
            Some(Kind::And) => Conjunction::And,
            Some(_) => Conjunction::Or,
            None => Conjunction::None,
        }
    }

    fn modifier(&mut self) -> Modifier {
        match self
            .take(&[Kind::Plus, Kind::Minus, Kind::Not])
            .map(|token| token.kind)
        {
            Some(Kind::Plus) => Modifier::Required,
            Some(_) => Modifier::Prohibited,
            None => Modifier::None,
        }
    }

    /// `Clause`: an optional `field:` (or `*:`), then a term or a parenthesized query.
    fn clause(&mut self, field: Option<&str>) -> Result<Option<Node>, InvalidQuery> {
        let mut field = field.map(str::to_owned);
        if self.peek(1) == Some(Kind::Colon) {
            if let Some(token) = self.take(&[Kind::Term]) {
                field = Some(discard_escapes(&token.image)?);
                self.next += 1;
            } else if self.take(&[Kind::Star]).is_some() {
                field = Some("*".to_owned());
                self.next += 1;
            }
        }
        if self.take(&[Kind::LeftParen]).is_none() {
            return self.term(field.as_deref());
        }
        let query = self.query(field.as_deref())?;
        self.expect(&[Kind::RightParen])?;
        let boost = self.boost()?;
        Ok(with_boost(query, boost.as_deref()))
    }

    fn boost(&mut self) -> Result<Option<String>, InvalidQuery> {
        if self.take(&[Kind::Caret]).is_none() {
            return Ok(None);
        }
        Ok(Some(self.expect(&[Kind::Number])?.image))
    }

    fn suffixes(&mut self) -> Result<Suffixes, InvalidQuery> {
        let fuzzy_slop =
            |parser: &mut Self| parser.take(&[Kind::FuzzySlop]).map(|token| token.image);
        let mut suffixes = Suffixes::default();
        if let Some(boost) = self.boost()? {
            suffixes.boost = Some(boost);
            suffixes.fuzzy_slop = fuzzy_slop(self);
        } else if let Some(slop) = fuzzy_slop(self) {
            suffixes.fuzzy_slop = Some(slop);
            suffixes.boost = self.boost()?;
        }
        Ok(suffixes)
    }

    /// `Term`: a bare term, a range, or a quoted phrase.
    fn term(&mut self, field: Option<&str>) -> Result<Option<Node>, InvalidQuery> {
        const BARE: &[Kind] = &[
            Kind::Term,
            Kind::Star,
            Kind::PrefixTerm,
            Kind::WildTerm,
            Kind::RegexpTerm,
            Kind::Number,
            Kind::BareOperator,
        ];
        let query = if let Some(token) = self.take(BARE) {
            let suffixes = self.suffixes()?;
            let query = self.bare_term(field, &token, suffixes.fuzzy_slop.as_deref())?;
            with_boost(query, suffixes.boost.as_deref())
        } else if let Some(token) = self.take(&[Kind::Quoted]) {
            let suffixes = self.suffixes()?;
            let query = self.quoted(field, &token, suffixes.fuzzy_slop.as_deref())?;
            with_boost(query, suffixes.boost.as_deref())
        } else {
            let start = self.expect(&[Kind::RangeInclusiveStart, Kind::RangeExclusiveStart])?;
            let query = self.range(field, start.kind == Kind::RangeInclusiveStart)?;
            let boost = self.boost()?;
            with_boost(Some(query), boost.as_deref())
        };
        Ok(query)
    }

    /// `handleBareTokenQuery`.
    fn bare_term(
        &mut self,
        field: Option<&str>,
        token: &Token,
        fuzzy_slop: Option<&str>,
    ) -> Result<Option<Node>, InvalidQuery> {
        let image = match token.kind {
            Kind::BareOperator => &token.image[..1],
            _ => token.image.as_str(),
        };
        let text = discard_escapes(image)?;
        match token.kind {
            Kind::Star | Kind::WildTerm => self.analysis.wildcard(field, image).map(Some),
            Kind::PrefixTerm => {
                let text = discard_escapes(&image[..image.len() - 1])?;
                self.analysis.prefix(field, &text).map(Some)
            }
            Kind::RegexpTerm => Ok(Some(
                self.analysis.regexp(field, &image[1..image.len() - 1]),
            )),
            _ => match fuzzy_slop {
                Some(slop) => self
                    .analysis
                    .fuzzy(field, &text, fuzzy_similarity(slop)?)
                    .map(Some),
                None => Ok(self.analysis.field_query(field, &text)),
            },
        }
    }

    /// `handleQuotedTerm`: a slop that is not a number is ignored.
    fn quoted(
        &mut self,
        field: Option<&str>,
        token: &Token,
        fuzzy_slop: Option<&str>,
    ) -> Result<Option<Node>, InvalidQuery> {
        let slop = match fuzzy_slop.and_then(|slop| java_float(&slop[1..])) {
            // A negative slop fails `PhraseQuery`, which Komga reports as no results.
            Some(slop) if slop <= -1.0 => return Err(InvalidQuery),
            // Java's `(int)` cast: towards zero, saturating, NaN to zero.
            Some(slop) => slop as u32,
            None => 0,
        };
        let text = discard_escapes(&token.image[1..token.image.len() - 1])?;
        Ok(self.analysis.phrase_query(field, &text, slop))
    }

    /// The range part of `Term`, after its opening bracket.
    fn range(&mut self, field: Option<&str>, start_inclusive: bool) -> Result<Node, InvalidQuery> {
        const BOUND: &[Kind] = &[Kind::RangeGoop, Kind::RangeQuoted, Kind::RangeTo];
        let lower = range_bound(&self.expect(BOUND)?)?;
        self.expect(&[Kind::RangeTo])?;
        let upper = range_bound(&self.expect(BOUND)?)?;
        let end = self.expect(&[Kind::RangeInclusiveEnd, Kind::RangeExclusiveEnd])?;
        let end_inclusive = end.kind == Kind::RangeInclusiveEnd;
        Ok(self.analysis.range(
            field,
            bound(lower, start_inclusive),
            bound(upper, end_inclusive),
        ))
    }
}

fn starts_clause(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::And
            | Kind::Or
            | Kind::Not
            | Kind::Plus
            | Kind::Minus
            | Kind::BareOperator
            | Kind::LeftParen
            | Kind::Star
            | Kind::Quoted
            | Kind::Term
            | Kind::PrefixTerm
            | Kind::WildTerm
            | Kind::RegexpTerm
            | Kind::RangeInclusiveStart
            | Kind::RangeExclusiveStart
            | Kind::Number
    )
}

/// `QueryParserBase.addClause` for the AND default operator.
fn add_clause(
    clauses: &mut Vec<(Occur, Node)>,
    conjunction: Conjunction,
    modifier: Modifier,
    query: Option<Node>,
) {
    // AND makes the preceding clause required, OR makes it optional; prohibited stays.
    if let Some((occur, _)) = clauses.last_mut()
        && *occur != Occur::MustNot
    {
        match conjunction {
            Conjunction::And => *occur = Occur::Must,
            Conjunction::Or => *occur = Occur::Should,
            Conjunction::None => {}
        }
    }
    // The analyzer may have removed every term.
    let Some(query) = query else {
        return;
    };
    let occur = if modifier == Modifier::Prohibited {
        Occur::MustNot
    } else if conjunction == Conjunction::Or {
        Occur::Should
    } else {
        Occur::Must
    };
    clauses.push((occur, query));
}

fn with_boost(query: Option<Node>, boost: Option<&str>) -> Option<Node> {
    match (query, boost.and_then(java_float)) {
        (Some(query), Some(boost)) => Some(Node::Boost(Box::new(query), boost)),
        (query, _) => query,
    }
}

/// `handleBareFuzzy`: the similarity after `~`, 2 by default.
fn fuzzy_similarity(slop: &str) -> Result<f32, InvalidQuery> {
    let similarity = java_float(&slop[1..]).unwrap_or(DEFAULT_FUZZY_SIMILARITY);
    if similarity < 0.0 || (similarity >= 1.0 && similarity.fract() != 0.0) {
        return Err(InvalidQuery);
    }
    Ok(similarity)
}

/// The decimal forms of Java's `Float.parseFloat`, including a trailing type suffix.
fn java_float(text: &str) -> Option<f32> {
    let text = text.trim();
    let number = text.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(text);
    if !number
        .chars()
        .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    number.parse().ok()
}

/// A quoted bound loses its quotes; an unquoted `*` is open.
fn range_bound(token: &Token) -> Result<Option<String>, InvalidQuery> {
    let image = match token.kind {
        Kind::RangeQuoted => &token.image[1..token.image.len() - 1],
        _ if token.image == "*" => return Ok(None),
        _ => token.image.as_str(),
    };
    discard_escapes(image).map(Some)
}

fn bound(value: Option<String>, inclusive: bool) -> Bound<String> {
    match value {
        None => Bound::Unbounded,
        Some(value) if inclusive => Bound::Included(value),
        Some(value) => Bound::Excluded(value),
    }
}

/// `discardEscapeChar`: drops each escaping backslash and decodes `\uXXXX` UTF-16 units.
pub(super) fn discard_escapes(input: &str) -> Result<String, InvalidQuery> {
    let mut units: Vec<u16> = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buffer = [0; 2];
            units.extend_from_slice(ch.encode_utf16(&mut buffer));
            continue;
        }
        match chars.next().ok_or(InvalidQuery)? {
            'u' => {
                let mut unit = 0;
                for _ in 0..4 {
                    let digit = chars
                        .next()
                        .and_then(|ch| ch.to_digit(16))
                        .ok_or(InvalidQuery)?;
                    unit = unit * 16 + digit;
                }
                units.push(u16::try_from(unit).expect("four hex digits fit in 16 bits"));
            }
            escaped => {
                let mut buffer = [0; 2];
                units.extend_from_slice(escaped.encode_utf16(&mut buffer));
            }
        }
    }
    String::from_utf16(&units).map_err(|_| InvalidQuery)
}
