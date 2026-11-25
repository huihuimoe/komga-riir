//! The Lucene query a parsed query string stands for, before it is bound to the index.

use std::ops::Bound;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Occur {
    Must,
    Should,
    MustNot,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Node {
    Term {
        field: String,
        text: String,
    },
    /// Terms with their positions relative to the first.
    Phrase {
        field: String,
        terms: Vec<(usize, String)>,
        slop: u32,
    },
    Boolean(Vec<(Occur, Node)>),
    Boost(Box<Node>, f32),
    MatchAll,
    Prefix {
        field: String,
        text: String,
    },
    /// A `WildcardQuery` pattern: `*` and `?` are wildcards, and a backslash makes the next
    /// character literal.
    Wildcard {
        field: String,
        pattern: String,
    },
    Regexp {
        field: String,
        pattern: String,
    },
    Fuzzy {
        field: String,
        text: String,
        edits: u8,
    },
    Range {
        field: String,
        lower: Bound<String>,
        upper: Bound<String>,
    },
}

impl Node {
    /// Komga appends ` *:*` to every query so that a purely negative query matches, which also
    /// makes any query with an `OR` match every document. The appended clause is dropped when
    /// another clause can match, and every purely negative group, at any depth, matches all
    /// documents but its exclusions.
    pub(super) fn without_appended_match_all(self) -> Node {
        let Node::Boolean(mut clauses) = self else {
            return self;
        };
        let appended = clauses.pop();
        if !clauses.iter().any(|(occur, _)| *occur != Occur::MustNot) {
            clauses.extend(appended);
        }
        Node::Boolean(clauses).with_negative_groups_matching()
    }

    fn with_negative_groups_matching(self) -> Node {
        match self {
            Node::Boolean(clauses) => {
                let mut clauses: Vec<(Occur, Node)> = clauses
                    .into_iter()
                    .map(|(occur, node)| (occur, node.with_negative_groups_matching()))
                    .collect();
                if clauses.iter().all(|(occur, _)| *occur == Occur::MustNot) {
                    clauses.push((Occur::Must, Node::MatchAll));
                }
                Node::Boolean(clauses)
            }
            Node::Boost(node, boost) => {
                Node::Boost(Box::new(node.with_negative_groups_matching()), boost)
            }
            node => node,
        }
    }
}
