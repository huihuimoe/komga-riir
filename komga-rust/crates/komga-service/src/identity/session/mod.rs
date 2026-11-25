//! Thread-safe in-memory session registry and remember-me token handling.

mod remember_me;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use komga_types::KomgaUser;
use tokio_util::sync::CancellationToken;

/// The baseline `spring.session.timeout`.
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// Minimum elapsed milliseconds before refreshing `last_accessed_ms` to avoid cache-line contention
/// under concurrent requests from the same session.
const ACCESS_REFRESH_GRANULARITY_MILLIS: u64 = 1_000;
/// Session count threshold at which `issue_token` lazily prunes expired entries.
const LAZY_EVICT_THRESHOLD: usize = 1024;

struct SessionRecord {
    user: KomgaUser,
    last_accessed_ms: AtomicU64,
    logged_out: CancellationToken,
}

impl SessionRecord {
    fn is_expired(&self, now_ms: u64, ttl_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_accessed_ms.load(Ordering::Relaxed)) > ttl_ms
    }
}

struct SessionRegistryInner {
    origin: Instant,
    ttl: Duration,
    ttl_ms: u64,
    sessions: RwLock<HashMap<String, SessionRecord>>,
    /// When each user's sessions were last revoked; only touched under the `sessions` write lock.
    revoked_at: Mutex<HashMap<String, Instant>>,
}

/// In-memory session registry with support for Spring remember-me token issuance and validation.
#[derive(Clone)]
pub struct SessionRegistry {
    inner: Arc<SessionRegistryInner>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::with_ttl(DEFAULT_SESSION_TTL)
    }
}

impl SessionRegistry {
    /// Create an empty session registry with the baseline session lifetime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty session registry whose sessions live `ttl` after their last access.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(SessionRegistryInner {
                origin: Instant::now(),
                ttl,
                ttl_ms: u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX),
                sessions: RwLock::new(HashMap::new()),
                revoked_at: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// How long a session lives after its last access; the session cookie's `Max-Age` matches it.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.inner.ttl
    }

    fn elapsed_ms(&self, instant: Instant) -> u64 {
        u64::try_from(
            instant
                .saturating_duration_since(self.inner.origin)
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }

    /// Issue a new session token for `user`, as read from the database at `loaded_at`; `None`
    /// when the user's sessions were revoked since, as the snapshot may hold revoked rights.
    pub fn issue_token(&self, user: &KomgaUser, loaded_at: Instant) -> Option<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("Failed to get random bytes");
        let token = hex::encode(bytes);
        let now_ms = self.elapsed_ms(Instant::now());

        let mut lock = self
            .inner
            .sessions
            .write()
            .expect("Session registry poisoned");
        let revoked_at = self.revoked_at().get(user.id.as_str()).copied();
        if revoked_at.is_some_and(|revoked_at| revoked_at >= loaded_at) {
            return None;
        }
        if lock.len() >= LAZY_EVICT_THRESHOLD {
            lock.retain(|_, record| !record.is_expired(now_ms, self.inner.ttl_ms));
        }
        lock.insert(
            token.clone(),
            SessionRecord {
                user: user.clone(),
                last_accessed_ms: AtomicU64::new(now_ms),
                logged_out: CancellationToken::new(),
            },
        );
        Some(token)
    }

    fn revoked_at(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.inner
            .revoked_at
            .lock()
            .expect("Session registry poisoned")
    }

    /// Resolve a session user if token is valid and unexpired.
    pub fn resolve_user(&self, token: &str) -> Option<KomgaUser> {
        self.resolve_user_at(token, Instant::now())
    }

    fn resolve_user_at(&self, token: &str, now: Instant) -> Option<KomgaUser> {
        let now_ms = self.elapsed_ms(now);
        {
            let lock = self
                .inner
                .sessions
                .read()
                .expect("Session registry poisoned");
            let record = lock.get(token)?;
            let last_ms = record.last_accessed_ms.load(Ordering::Relaxed);
            let idle_ms = now_ms.saturating_sub(last_ms);
            if idle_ms <= self.inner.ttl_ms {
                if idle_ms >= ACCESS_REFRESH_GRANULARITY_MILLIS {
                    record.last_accessed_ms.fetch_max(now_ms, Ordering::Relaxed);
                }
                return Some(record.user.clone());
            }
        }
        // Only upgrade to a write lock when removing an expired session.
        let mut lock = self
            .inner
            .sessions
            .write()
            .expect("Session registry poisoned");
        let record = lock.get(token)?;
        if record.is_expired(now_ms, self.inner.ttl_ms) {
            lock.remove(token);
            return None;
        }
        record.last_accessed_ms.fetch_max(now_ms, Ordering::Relaxed);
        Some(record.user.clone())
    }

    /// Removes all sessions that have exceeded [`Self::ttl`].
    pub fn evict_expired(&self) {
        self.evict_expired_at(Instant::now());
    }

    fn evict_expired_at(&self, now: Instant) {
        let now_ms = self.elapsed_ms(now);
        let has_expired = self
            .inner
            .sessions
            .read()
            .expect("Session registry poisoned")
            .values()
            .any(|record| record.is_expired(now_ms, self.inner.ttl_ms));
        if has_expired {
            self.inner
                .sessions
                .write()
                .expect("Session registry poisoned")
                .retain(|_, record| !record.is_expired(now_ms, self.inner.ttl_ms));
        }
    }

    /// Invalidate/remove a specific session token (logout).
    pub fn invalidate(&self, token: &str) {
        let mut lock = self
            .inner
            .sessions
            .write()
            .expect("Session registry poisoned");
        if let Some(record) = lock.remove(token) {
            record.logged_out.cancel();
        }
    }

    /// Cancelled once the session is logged out of, or already if it is gone. Sessions revoked
    /// with [`Self::invalidate_user`] are not cancelled: their user is told of it first.
    #[must_use]
    pub fn logged_out(&self, token: &str) -> CancellationToken {
        let lock = self
            .inner
            .sessions
            .read()
            .expect("Session registry poisoned");
        lock.get(token).map_or_else(
            || {
                let gone = CancellationToken::new();
                gone.cancel();
                gone
            },
            |record| record.logged_out.clone(),
        )
    }

    /// Invalidate all sessions for a user ID (e.g. password update).
    pub fn invalidate_user(&self, user_id: &str) {
        let mut lock = self
            .inner
            .sessions
            .write()
            .expect("Session registry poisoned");
        self.revoked_at().insert(user_id.to_owned(), Instant::now());
        lock.retain(|_, record| record.user.id.as_str() != user_id);
    }
}
