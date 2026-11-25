//! Turns the text of a clause into queries, as `MultiFieldQueryParser` and Lucene's
//! `QueryBuilder` do: a clause without a field searches each default field.

use std::ops::Bound;

use tantivy::tokenizer::TextAnalyzer;

use super::InvalidQuery;
use super::node::{Node, Occur};
use crate::analyzer;

pub(super) struct Analysis<'a> {
    default_fields: &'a [&'a str],
    analyzer: TextAnalyzer,
}

impl<'a> Analysis<'a> {
    pub(super) fn new(default_fields: &'a [&'a str]) -> Self {
        Self {
            default_fields,
            analyzer: analyzer::search_analyzer(),
        }
    }

    /// Lucene's `getFieldQuery` for unquoted text. Across default fields, the n-th term of each
    /// field forms one optional group, so the terms of one word are alternatives.
    pub(super) fn field_query(&mut self, field: Option<&str>, text: &str) -> Option<Node> {
        let Some(field) = field else {
            let per_field: Vec<Option<Node>> = self
                .default_fields
                .iter()
                .map(|field| self.analyzed(field, text, None))
                .collect();
            let term_count = per_field.iter().flatten().map(|query| match query {
                Node::Boolean(clauses) => clauses.len(),
                _ => 1,
            });
            let term_count = term_count.max().unwrap_or(0);
            let mut clauses = Vec::new();
            for index in 0..term_count {
                let alternatives: Vec<(Occur, Node)> = per_field
                    .iter()
                    .flatten()
                    .filter_map(|query| match query {
                        Node::Boolean(clauses) => clauses.get(index).map(|(_, node)| node.clone()),
                        _ => (index == 0).then(|| query.clone()),
                    })
                    .map(|node| (Occur::Should, node))
                    .collect();
                if term_count == 1 {
                    clauses.extend(alternatives);
                } else if !alternatives.is_empty() {
                    clauses.push((Occur::Should, Node::Boolean(alternatives)));
                }
            }
            return (!clauses.is_empty()).then_some(Node::Boolean(clauses));
        };
        self.analyzed(field, text, None)
    }

    /// Lucene's `getFieldQuery` for quoted text.
    pub(super) fn phrase_query(
        &mut self,
        field: Option<&str>,
        text: &str,
        slop: u32,
    ) -> Option<Node> {
        let Some(field) = field else {
            let alternatives: Vec<(Occur, Node)> = self
                .default_fields
                .iter()
                .filter_map(|field| self.analyzed(field, text, Some(slop)))
                .map(|node| (Occur::Should, node))
                .collect();
            return (!alternatives.is_empty()).then_some(Node::Boolean(alternatives));
        };
        self.analyzed(field, text, Some(slop))
    }

    /// `QueryBuilder.createFieldQuery`: one term, a phrase when quoted, or else every term
    /// required. The query analyzer puts each term at its own position.
    fn analyzed(&mut self, field: &str, text: &str, phrase_slop: Option<u32>) -> Option<Node> {
        let mut stream = self.analyzer.token_stream(text);
        let mut terms = Vec::new();
        while let Some(token) = stream.next() {
            terms.push((token.position, token.text.clone()));
        }
        let field = field.to_owned();
        match (terms.as_slice(), phrase_slop) {
            ([], _) => None,
            ([(_, text)], _) => Some(Node::Term {
                field,
                text: text.clone(),
            }),
            ([(first, _), ..], Some(slop)) => {
                let first = *first;
                let terms = terms
                    .into_iter()
                    .map(|(position, text)| (position - first, text))
                    .collect();
                Some(Node::Phrase { field, terms, slop })
            }
            (_, None) => {
                let clauses = terms
                    .into_iter()
                    .map(|(_, text)| {
                        let field = field.clone();
                        (Occur::Must, Node::Term { field, text })
                    })
                    .collect();
                Some(Node::Boolean(clauses))
            }
        }
    }

    /// `getWildcardQuery`: `*:*` matches everything; a leading wildcard is invalid.
    pub(super) fn wildcard(&self, field: Option<&str>, image: &str) -> Result<Node, InvalidQuery> {
        self.per_field(field, |field| {
            if field == "*" && image == "*" {
                return Ok(Node::MatchAll);
            }
            if image.starts_with(['*', '?']) {
                return Err(InvalidQuery);
            }
            Ok(Node::Wildcard {
                field: field.to_owned(),
                pattern: normalize_wildcard(image),
            })
        })
    }

    /// `getPrefixQuery`, given the text before the trailing `*`.
    pub(super) fn prefix(&self, field: Option<&str>, text: &str) -> Result<Node, InvalidQuery> {
        if text.starts_with('*') {
            return Err(InvalidQuery);
        }
        self.per_field(field, |field| {
            Ok(Node::Prefix {
                field: field.to_owned(),
                text: analyzer::normalize(text),
            })
        })
    }

    /// `getRegexpQuery`: the whole expression is normalized, operators included.
    pub(super) fn regexp(&self, field: Option<&str>, pattern: &str) -> Node {
        let regexp = |field: &str| {
            Ok(Node::Regexp {
                field: field.to_owned(),
                pattern: analyzer::normalize(pattern),
            })
        };
        self.per_field(field, regexp)
            .expect("a regular expression is always accepted here")
    }

    /// `getFuzzyQuery` with `FuzzyQuery.floatToEdits`.
    pub(super) fn fuzzy(
        &self,
        field: Option<&str>,
        text: &str,
        similarity: f32,
    ) -> Result<Node, InvalidQuery> {
        let text = analyzer::normalize(text);
        let edits = if similarity >= 1.0 {
            similarity.min(2.0) as u8
        } else if similarity == 0.0 {
            0
        } else {
            let length = text.chars().count() as f64;
            ((1.0 - f64::from(similarity)) * length).min(2.0) as u8
        };
        self.per_field(field, |field| {
            Ok(Node::Fuzzy {
                field: field.to_owned(),
                text: text.clone(),
                edits,
            })
        })
    }

    /// `getRangeQuery`: bounds are normalized, not analyzed.
    pub(super) fn range(
        &self,
        field: Option<&str>,
        lower: Bound<String>,
        upper: Bound<String>,
    ) -> Node {
        let lower = lower.map(|text| analyzer::normalize(&text));
        let upper = upper.map(|text| analyzer::normalize(&text));
        let range = |field: &str| {
            Ok(Node::Range {
                field: field.to_owned(),
                lower: lower.clone(),
                upper: upper.clone(),
            })
        };
        self.per_field(field, range)
            .expect("a range is always accepted here")
    }

    /// One query for an explicit field; otherwise one optional clause per default field.
    fn per_field(
        &self,
        field: Option<&str>,
        query: impl Fn(&str) -> Result<Node, InvalidQuery>,
    ) -> Result<Node, InvalidQuery> {
        let Some(field) = field else {
            let clauses = self
                .default_fields
                .iter()
                .map(|field| query(field).map(|node| (Occur::Should, node)))
                .collect::<Result<_, _>>()?;
            return Ok(Node::Boolean(clauses));
        };
        query(field)
    }
}

/// `analyzeWildcard`: text between wildcards and escapes is normalized; wildcards and escaped
/// characters are kept as written.
fn normalize_wildcard(image: &str) -> String {
    let mut pattern = String::with_capacity(image.len());
    let mut chunk = String::new();
    let mut chars = image.chars();
    while let Some(ch) = chars.next() {
        let kept = match ch {
            '\\' => chars.next().map(|escaped| format!("\\{escaped}")),
            '*' | '?' => Some(ch.to_string()),
            _ => None,
        };
        match kept {
            Some(kept) => {
                pattern.push_str(&analyzer::normalize(&chunk));
                chunk.clear();
                pattern.push_str(&kept);
            }
            None => chunk.push(ch),
        }
    }
    pattern.push_str(&analyzer::normalize(&chunk));
    pattern
}
