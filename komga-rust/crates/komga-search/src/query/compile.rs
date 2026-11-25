//! Binds a parsed query to the index fields.

use std::ops::Bound;

use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur as TantivyOccur, PhraseQuery, Query,
    RangeQuery, TermQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy_fst::Regex;

use super::InvalidQuery;
use super::automaton::{AutomatonQuery, Prefixed};
use super::node::{Node, Occur};
use crate::schema::SearchSchema;

/// Fails when a regular expression is invalid or too large.
pub(super) fn compile(node: &Node, schema: &SearchSchema) -> Result<Box<dyn Query>, InvalidQuery> {
    let query: Box<dyn Query> = match node {
        Node::Term { field, text } => Box::new(TermQuery::new(
            schema.target(field).term(text),
            IndexRecordOption::WithFreqs,
        )),
        Node::Phrase { field, terms, slop } => {
            let target = schema.target(field);
            let terms = terms
                .iter()
                .map(|(position, text)| (*position, target.term(text)))
                .collect();
            Box::new(PhraseQuery::new_with_offset_and_slop(terms, *slop))
        }
        Node::Boolean(clauses) => {
            let clauses = clauses
                .iter()
                .map(|(occur, node)| Ok((tantivy_occur(*occur), compile(node, schema)?)))
                .collect::<Result<_, _>>()?;
            Box::new(BooleanQuery::new(clauses))
        }
        Node::Boost(node, boost) => Box::new(BoostQuery::new(compile(node, schema)?, *boost)),
        Node::MatchAll => Box::new(AllQuery),
        Node::Prefix { field, text } => {
            let pattern = format!("{}(?s:.*)", regex_syntax::escape(text));
            regex(schema, field, &pattern)?
        }
        Node::Wildcard { field, pattern } => regex(schema, field, &wildcard_regex(pattern))?,
        Node::Regexp { field, pattern } => regex(schema, field, pattern)?,
        Node::Fuzzy { field, text, edits } => {
            let target = schema.target(field);
            // Lucene's `FuzzyQuery` counts a transposition as one edit.
            let fuzzy = FuzzyTermQuery::new(
                tantivy::Term::from_field_text(target.field, text),
                *edits,
                true,
            );
            let automaton = fuzzy.automaton().map_err(|_| InvalidQuery)?;
            let automaton = Prefixed::new(target.prefix.as_bytes(), automaton);
            Box::new(AutomatonQuery::new(target.field, automaton))
        }
        Node::Range {
            field,
            lower,
            upper,
        } => {
            let target = schema.target(field);
            let lower = match lower {
                Bound::Included(text) => Bound::Included(target.term(text)),
                Bound::Excluded(text) => Bound::Excluded(target.term(text)),
                Bound::Unbounded => Bound::Included(target.term("")),
            };
            let upper = match upper {
                Bound::Included(text) => Bound::Included(target.term(text)),
                Bound::Excluded(text) => Bound::Excluded(target.term(text)),
                Bound::Unbounded => target.upper_limit(),
            };
            Box::new(RangeQuery::new(lower, upper))
        }
    };
    Ok(query)
}

fn tantivy_occur(occur: Occur) -> TantivyOccur {
    match occur {
        Occur::Must => TantivyOccur::Must,
        Occur::Should => TantivyOccur::Should,
        Occur::MustNot => TantivyOccur::MustNot,
    }
}

/// A whole-term regular expression over the terms of `field`.
fn regex(
    schema: &SearchSchema,
    field: &str,
    pattern: &str,
) -> Result<Box<dyn Query>, InvalidQuery> {
    let target = schema.target(field);
    let regex = Regex::new(pattern).map_err(|_| InvalidQuery)?;
    let automaton = Prefixed::new(target.prefix.as_bytes(), regex);
    Ok(Box::new(AutomatonQuery::new(target.field, automaton)))
}

/// `WildcardQuery.toAutomaton`: `*` is any text, `?` any one character, and a backslash makes
/// the next character literal.
fn wildcard_regex(pattern: &str) -> String {
    let mut regex = String::from("(?s)");
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '\\' => {
                let literal = chars.next().unwrap_or('\\');
                regex.push_str(&regex_syntax::escape(literal.encode_utf8(&mut [0; 4])));
            }
            _ => regex.push_str(&regex_syntax::escape(ch.encode_utf8(&mut [0; 4]))),
        }
    }
    regex
}
