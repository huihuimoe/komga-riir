use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use komga_types::{ContentRestrictions, KomgaUser, UserId, UserRole};

use super::SessionRegistry;

fn sample_user(id: &str, email: &str, roles: BTreeSet<UserRole>) -> KomgaUser {
    KomgaUser {
        id: UserId::from(id),
        email: email.to_owned(),
        password_hash: "hash".to_owned(),
        roles,
        shared_all_libraries: true,
        shared_library_ids: BTreeSet::new(),
        restrictions: ContentRestrictions::default(),
    }
}

#[test]
fn session_expiry_slides_seven_days_from_last_access() {
    let registry = SessionRegistry::new();
    let user = sample_user(
        "user-1",
        "reader@example.com",
        BTreeSet::from([UserRole::FileDownload]),
    );
    let token = registry
        .issue_token(&user, Instant::now())
        .expect("Session");
    let reference = Instant::now();
    let six_days = Duration::from_secs(6 * 24 * 60 * 60);

    assert_eq!(
        registry.resolve_user_at(&token, reference + six_days),
        Some(user.clone())
    );
    assert_eq!(
        registry.resolve_user_at(&token, reference + six_days * 2),
        Some(user)
    );
    assert!(
        registry
            .resolve_user_at(
                &token,
                reference + six_days * 2 + Duration::from_secs(7 * 24 * 60 * 60 + 1),
            )
            .is_none()
    );
}

#[test]
fn evict_expired_prunes_only_stale_sessions() {
    let registry = SessionRegistry::new();
    let user = sample_user("user-2", "reader2@example.com", BTreeSet::new());
    let stale = registry
        .issue_token(&user, Instant::now())
        .expect("Session");
    let active = registry
        .issue_token(&user, Instant::now())
        .expect("Session");
    let now = Instant::now();
    let six_days = Duration::from_secs(6 * 24 * 60 * 60);
    assert!(registry.resolve_user_at(&active, now + six_days).is_some());

    registry.evict_expired_at(now + Duration::from_secs(7 * 24 * 60 * 60 + 1));
    assert!(registry.resolve_user_at(&stale, now + six_days).is_none());
    assert!(registry.resolve_user_at(&active, now + six_days).is_some());
}

#[test]
fn a_user_read_before_their_sessions_were_revoked_gets_no_session() {
    let registry = SessionRegistry::new();
    let user = sample_user("user-3", "reader3@example.com", BTreeSet::new());
    let loaded_at = Instant::now();
    registry.invalidate_user("user-3");

    assert_eq!(registry.issue_token(&user, loaded_at), None);
    let other = sample_user("user-4", "reader4@example.com", BTreeSet::new());
    assert!(registry.issue_token(&other, loaded_at).is_some());
    assert!(registry.issue_token(&user, Instant::now()).is_some());
}
