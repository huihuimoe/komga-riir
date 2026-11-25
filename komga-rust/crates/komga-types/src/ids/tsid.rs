//! `TsidCreator.getTsid256()`: 42 bits of milliseconds since 2020-01-01, an 8-bit node drawn
//! once per process and a 14-bit counter that starts at random each millisecond and carries
//! into the next one, in 13 Crockford base32 characters.

use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const EPOCH_MILLIS: u64 = 1_577_836_800_000;
const NODE_BITS: u32 = 8;
const COUNTER_BITS: u32 = 14;
const COUNTER_MASK: u64 = (1 << COUNTER_BITS) - 1;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

struct State {
    node: u64,
    millis: u64,
    counter: u64,
}

pub(crate) fn generate() -> String {
    static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
        Mutex::new(State {
            node: random() >> (u64::BITS - NODE_BITS),
            millis: 0,
            counter: 0,
        })
    });
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
        .saturating_sub(EPOCH_MILLIS);
    let number = {
        let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
        if now > state.millis {
            state.millis = now;
            state.counter = random() & COUNTER_MASK;
        } else {
            state.counter += 1;
            state.millis += state.counter >> COUNTER_BITS;
            state.counter &= COUNTER_MASK;
        }
        state.millis << (NODE_BITS + COUNTER_BITS) | state.node << COUNTER_BITS | state.counter
    };
    (0..13)
        .rev()
        .map(|digit| char::from(ALPHABET[(number >> (digit * 5) & 31) as usize]))
        .collect()
}

fn random() -> u64 {
    getrandom::u64().expect("the OS random source is available")
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_sort_in_creation_order() {
        let ids: Vec<String> = (0..20_000).map(|_| super::generate()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
