//! Term automata (wildcard, prefix, regular expression, fuzzy) restricted to one query field.

use std::fmt;
use std::sync::Arc;

use tantivy::query::{AutomatonWeight, EnableScoring, Query, Weight};
use tantivy::schema::Field;
use tantivy_fst::Automaton;

/// Matches the terms of an index field that start with `prefix` and whose rest `inner` matches.
/// Fields without a dedicated index field share one, each term prefixed with its field name.
pub(super) struct Prefixed<A> {
    prefix: Vec<u8>,
    inner: A,
}

impl<A> Prefixed<A> {
    pub(super) fn new(prefix: &[u8], inner: A) -> Self {
        Self {
            prefix: prefix.to_vec(),
            inner,
        }
    }
}

#[derive(Clone)]
pub(super) enum PrefixedState<S> {
    /// Bytes of the prefix matched so far.
    Prefix(usize),
    Inner(S),
    Dead,
}

impl<A> Automaton for Prefixed<A>
where
    A: Automaton,
    A::State: Clone,
{
    type State = PrefixedState<A::State>;

    fn start(&self) -> Self::State {
        if self.prefix.is_empty() {
            PrefixedState::Inner(self.inner.start())
        } else {
            PrefixedState::Prefix(0)
        }
    }

    fn is_match(&self, state: &Self::State) -> bool {
        matches!(state, PrefixedState::Inner(inner) if self.inner.is_match(inner))
    }

    fn can_match(&self, state: &Self::State) -> bool {
        match state {
            PrefixedState::Prefix(_) => true,
            PrefixedState::Inner(inner) => self.inner.can_match(inner),
            PrefixedState::Dead => false,
        }
    }

    fn accept(&self, state: &Self::State, byte: u8) -> Self::State {
        match state {
            PrefixedState::Prefix(matched) if self.prefix[*matched] == byte => {
                if matched + 1 == self.prefix.len() {
                    PrefixedState::Inner(self.inner.start())
                } else {
                    PrefixedState::Prefix(matched + 1)
                }
            }
            PrefixedState::Inner(inner) => PrefixedState::Inner(self.inner.accept(inner, byte)),
            PrefixedState::Prefix(_) | PrefixedState::Dead => PrefixedState::Dead,
        }
    }
}

/// Matches the documents holding a term that `automaton` accepts, with a constant score as
/// Lucene's multi-term queries have.
pub(super) struct AutomatonQuery<A> {
    field: Field,
    automaton: Arc<Prefixed<A>>,
}

impl<A> AutomatonQuery<A> {
    pub(super) fn new(field: Field, automaton: Prefixed<A>) -> Self {
        Self {
            field,
            automaton: Arc::new(automaton),
        }
    }
}

impl<A> Clone for AutomatonQuery<A> {
    fn clone(&self) -> Self {
        Self {
            field: self.field,
            automaton: Arc::clone(&self.automaton),
        }
    }
}

impl<A> fmt::Debug for AutomatonQuery<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutomatonQuery")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl<A> Query for AutomatonQuery<A>
where
    A: Automaton + Send + Sync + 'static,
    A::State: Clone,
{
    fn weight(&self, _scoring: EnableScoring<'_>) -> tantivy::Result<Box<dyn Weight>> {
        Ok(Box::new(AutomatonWeight::<Prefixed<A>>::new(
            self.field,
            Arc::clone(&self.automaton),
        )))
    }
}
