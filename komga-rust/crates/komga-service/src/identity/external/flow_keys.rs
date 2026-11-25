//! The keys that seal the login flow cookie. They are persisted in `SERVER_SETTINGS` so that a
//! restart does not break logins in progress, and replaced after [`KEY_VALIDITY`]; the replaced
//! key still opens cookies for one [`FLOW_TTL`] after that.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use komga_db::{Database, ServerSettingsDao};
use komga_oauth::FLOW_TTL;
use tokio::sync::Mutex;

use super::ExternalIdentityError;

const CURRENT: &str = "OAUTH2_FLOW_KEY";
const PREVIOUS: &str = "OAUTH2_FLOW_KEY_PREVIOUS";
/// How long a key seals new flows.
const KEY_VALIDITY: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The cookie crate's `Key` length, kept so that stored keys and their cookies stay valid.
pub type FlowSecret = [u8; 64];

/// The key to seal a new flow with, and the replaced key while its flows may still come back.
pub struct FlowKeyRing {
    pub current: FlowSecret,
    pub previous: Option<FlowSecret>,
}

#[derive(Clone)]
pub(super) struct FlowKeys {
    database: Database,
    ring: Arc<Mutex<Ring>>,
}

#[derive(Clone)]
struct Ring {
    current: StoredKey,
    previous: Option<StoredKey>,
}

/// `<created, seconds since the Unix epoch>:<secret, base64url>`.
#[derive(Clone)]
struct StoredKey {
    created: u64,
    secret: FlowSecret,
}

impl FlowKeys {
    pub(super) async fn load(database: Database) -> Result<Self, ExternalIdentityError> {
        let current = read(&database, CURRENT).await?;
        let previous = read(&database, PREVIOUS).await?;
        let ring = match current {
            Some(current) => Ring { current, previous },
            None => rotate(&database, None, now()).await?,
        };
        Ok(Self {
            database,
            ring: Arc::new(Mutex::new(ring)),
        })
    }

    pub(super) async fn ring(&self) -> Result<FlowKeyRing, ExternalIdentityError> {
        let now = now();
        let mut ring = self.ring.lock().await;
        if now >= ring.current.created.saturating_add(KEY_VALIDITY.as_secs()) {
            *ring = rotate(&self.database, Some(ring.current.clone()), now).await?;
        }
        Ok(ring.at(now))
    }
}

impl Ring {
    fn at(&self, now: u64) -> FlowKeyRing {
        let grace_ends = self.current.created.saturating_add(FLOW_TTL.as_secs());
        FlowKeyRing {
            current: self.current.secret,
            previous: self
                .previous
                .as_ref()
                .filter(|_| now < grace_ends)
                .map(|previous| previous.secret),
        }
    }
}

async fn rotate(
    database: &Database,
    previous: Option<StoredKey>,
    now: u64,
) -> Result<Ring, ExternalIdentityError> {
    let mut secret = [0_u8; 64];
    getrandom::fill(&mut secret)?;
    let current = StoredKey {
        created: now,
        secret,
    };
    let mut transaction = database.begin_immediate().await?;
    ServerSettingsDao::save(&mut transaction, CURRENT, &current.encode()).await?;
    match &previous {
        Some(previous) => {
            ServerSettingsDao::save(&mut transaction, PREVIOUS, &previous.encode()).await?;
        }
        None => ServerSettingsDao::delete(&mut transaction, PREVIOUS).await?,
    }
    transaction.commit().await?;
    Ok(Ring { current, previous })
}

async fn read(
    database: &Database,
    key: &'static str,
) -> Result<Option<StoredKey>, ExternalIdentityError> {
    let Some(value) = ServerSettingsDao::get(database, key).await? else {
        return Ok(None);
    };
    StoredKey::decode(&value)
        .map(Some)
        .ok_or(ExternalIdentityError::InvalidStoredKey(key))
}

impl StoredKey {
    fn encode(&self) -> String {
        format!("{}:{}", self.created, URL_SAFE_NO_PAD.encode(self.secret))
    }

    fn decode(value: &str) -> Option<Self> {
        let (created, secret) = value.split_once(':')?;
        Some(Self {
            created: created.parse().ok()?,
            secret: URL_SAFE_NO_PAD.decode(secret).ok()?.try_into().ok()?,
        })
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use komga_test_support::TestDir;

    use super::*;

    async fn database(directory: &TestDir) -> Database {
        let database = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        database.migrate().await.expect("Migrate database");
        database
    }

    async fn store(database: &Database, key: &str, value: &StoredKey) {
        let mut transaction = database.begin_immediate().await.expect("Begin");
        ServerSettingsDao::save(&mut transaction, key, &value.encode())
            .await
            .expect("Store key");
        transaction.commit().await.expect("Commit");
    }

    #[tokio::test]
    async fn the_key_survives_a_restart() {
        let directory = komga_test_support::tempdir().expect("Temporary directory");
        let first = FlowKeys::load(database(&directory).await)
            .await
            .expect("Load");
        let second = FlowKeys::load(database(&directory).await)
            .await
            .expect("Reload");

        let first = first.ring().await.expect("Ring");
        let second = second.ring().await.expect("Ring");
        assert_eq!(first.current, second.current);
        assert!(second.previous.is_none(), "A first key replaces nothing");
    }

    #[tokio::test]
    async fn an_expired_key_is_replaced_and_opens_flows_for_one_flow_ttl() {
        let directory = komga_test_support::tempdir().expect("Temporary directory");
        let database = database(&directory).await;
        let expired = StoredKey {
            created: now() - KEY_VALIDITY.as_secs(),
            secret: [7; 64],
        };
        store(&database, CURRENT, &expired).await;

        let keys = FlowKeys::load(database.clone()).await.expect("Load");
        let ring = keys.ring().await.expect("Ring");
        assert_ne!(ring.current, expired.secret);
        assert_eq!(ring.previous, Some(expired.secret));
        let stored = read(&database, PREVIOUS)
            .await
            .expect("Read")
            .expect("Kept");
        assert_eq!(stored.secret, expired.secret);

        let replaced = keys.ring.lock().await.clone();
        let after_grace = replaced.current.created + FLOW_TTL.as_secs();
        assert!(replaced.at(after_grace).previous.is_none());
    }

    #[tokio::test]
    async fn a_corrupt_key_stops_startup() {
        let directory = komga_test_support::tempdir().expect("Temporary directory");
        let database = database(&directory).await;
        let mut transaction = database.begin_immediate().await.expect("Begin");
        ServerSettingsDao::save(&mut transaction, CURRENT, "1:short")
            .await
            .expect("Store key");
        transaction.commit().await.expect("Commit");

        assert!(matches!(
            FlowKeys::load(database).await,
            Err(ExternalIdentityError::InvalidStoredKey(CURRENT))
        ));
    }
}
