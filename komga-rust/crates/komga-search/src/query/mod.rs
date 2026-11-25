//! Lucene's classic query syntax, parsed as Komga's `LuceneHelper.searchEntitiesIds` does.

mod analysis;
mod automaton;
mod compile;
mod lexer;
mod node;
mod parser;
#[cfg(test)]
mod tests;

use tantivy::query::Query;

use analysis::Analysis;
use node::Node;
use parser::Parser;

use crate::schema::SearchSchema;

/// The query is not valid syntax, or Lucene would refuse to run it. Komga finds nothing then.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidQuery;

/// `IndexSearcher.getMaxClauseCount`, here over the whole query.
const MAX_CLAUSES: usize = 1024;

/// Parses `text` against the default fields and binds it to the index.
pub(crate) fn build(
    text: &str,
    default_fields: &[&str],
    schema: &SearchSchema,
) -> Result<Box<dyn Query>, InvalidQuery> {
    compile::compile(&parse(text, default_fields)?, schema)
}

/// Komga parses `"$text *:*"` so that purely negative queries match; see
/// [`Node::without_appended_match_all`] for how that clause is then handled.
fn parse(text: &str, default_fields: &[&str]) -> Result<Node, InvalidQuery> {
    let node = parse_lucene(&format!("{text} *:*"), default_fields)?
        .expect("the appended *:* always yields a clause");
    if clause_count(&node) > MAX_CLAUSES {
        return Err(InvalidQuery);
    }
    Ok(node.without_appended_match_all())
}

/// What `MultiFieldQueryParser.parse` returns; `None` when the analyzer removes every term.
fn parse_lucene(text: &str, default_fields: &[&str]) -> Result<Option<Node>, InvalidQuery> {
    let tokens = lexer::tokens(text)?;
    Parser::new(tokens, Analysis::new(default_fields)).parse()
}

fn clause_count(node: &Node) -> usize {
    match node {
        Node::Boolean(clauses) => clauses.iter().map(|(_, node)| 1 + clause_count(node)).sum(),
        Node::Boost(node, _) => clause_count(node),
        _ => 0,
    }
}
