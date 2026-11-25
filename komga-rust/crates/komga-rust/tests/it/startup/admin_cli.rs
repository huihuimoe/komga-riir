use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use clap::Parser;
use komga_db::Database;
use komga_rust::config::{AdminConfig, CliArgs, Environment};
use komga_service::{EventBus, IdentityService, LibrarySharing, NewUser, SessionRegistry};
use komga_types::{ContentRestrictions, KomgaUser, UserRole};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

async fn prepare_database(config_dir: &Path, users: &[(&str, &str)]) {
    let db = Database::connect(&config_dir.join("database.sqlite"))
        .await
        .expect("Failed to create test main database");
    db.migrate()
        .await
        .expect("Failed to migrate test main database");

    let identity = IdentityService::new(db.clone(), SessionRegistry::new(), EventBus::default());
    for (email, password) in users {
        identity
            .create_user(NewUser {
                email: (*email).to_owned(),
                password: (*password).to_owned(),
                roles: BTreeSet::from([UserRole::Admin]),
                library_sharing: LibrarySharing::default(),
                restrictions: ContentRestrictions::default(),
            })
            .await
            .expect("Failed to seed test user");
    }
    db.close().await;
}

async fn authenticate(config_dir: &Path, email: &str, password: &str) -> Option<KomgaUser> {
    let db = Database::connect_existing_read_only(&config_dir.join("database.sqlite"))
        .await
        .expect("DB must open");
    let identity = IdentityService::new(db.clone(), SessionRegistry::new(), EventBus::default());
    let user = identity
        .authenticate(email, password)
        .await
        .expect("Auth query must succeed");
    db.close().await;
    user
}

fn run_bin(config_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_komga-rust"))
        .args(args)
        .env("KOMGA_CONFIG_DIR", config_dir)
        // Unrelated invalid server settings must not break one-shot admin commands.
        .env("SERVER_PORT", "not-a-port")
        .env("KOMGA_TASKSDB_FILE", " ")
        .output()
        .expect("Failed to execute komga-rust binary")
}

#[test]
fn cli_admin_flags_enforce_pairing_non_blank_and_server_conflicts() {
    let parsed = CliArgs::try_parse_from([
        "komga-rust",
        "--config-dir",
        "/data",
        "--list-users",
        "--reset=alice@example.org",
        "--reset",
        " bob@example.org ",
        "--newpassword=-secret ",
    ])
    .expect("Combined admin flags must parse");

    let command = parsed
        .admin_command()
        .expect("Admin command must be present");
    assert!(command.list_users);
    let reset = command
        .reset_password
        .expect("Reset payload must be present");
    assert_eq!(reset.emails, ["alice@example.org", "bob@example.org"]);
    assert_eq!(reset.new_password, "-secret ");

    for bad_args in [
        &["komga-rust", "--reset", "alice@example.org"][..],
        &["komga-rust", "--newpassword", "secret"][..],
        &["komga-rust", "--reset", "  ", "--newpassword", "secret"][..],
        &[
            "komga-rust",
            "--reset",
            "alice@example.org",
            "--newpassword",
            "   ",
        ][..],
        &["komga-rust", "--list-users", "--port", "8080"][..],
        &["komga-rust", "--list-users", "--host", "127.0.0.1"][..],
        &["komga-rust", "--list-users", "--dry-run"][..],
    ] {
        assert!(
            CliArgs::try_parse_from(bad_args).is_err(),
            "Expected CLI parse error for {bad_args:?}"
        );
    }
}

#[test]
fn admin_config_resolves_main_database_while_ignoring_unrelated_settings() {
    let dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    fs::write(
        dir.path().join("application.yml"),
        "server:\n  port: invalid\nkomga:\n  database:\n    file: ${komga.config-dir}/custom.sqlite\n    check-local-filesystem: false\n  tasks-db:\n    file: ' '\n",
    )
    .expect("Failed to write application.yml");

    let args = CliArgs::try_parse_from([
        "komga-rust",
        "--config-dir",
        dir.path().to_str().expect("UTF-8 temp path"),
        "--list-users",
    ])
    .expect("CLI args must parse");
    let env: Environment = [("SERVER_PORT", "0"), ("KOMGA_SORT_LOCALE", "invalid")]
        .into_iter()
        .collect();

    let config = AdminConfig::resolve(&args, &env).expect("Admin config must resolve");
    assert_eq!(config.database_file, dir.path().join("custom.sqlite"));
    assert!(!config.check_local_filesystem);
}

#[tokio::test]
async fn list_users_prints_empty_message_or_sorted_emails_without_banner() {
    let dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    prepare_database(dir.path(), &[]).await;

    let empty = run_bin(dir.path(), &["--list-users"]);
    assert!(
        empty.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&empty.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&empty.stdout).trim(),
        "No users exist yet"
    );
    assert!(empty.stderr.is_empty());

    let seeded_dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    prepare_database(
        seeded_dir.path(),
        &[
            ("bob@example.org", "pass-1"),
            ("alice@example.org", "pass-2"),
        ],
    )
    .await;

    let listed = run_bin(seeded_dir.path(), &["--list-users"]);
    assert!(
        listed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let stdout = String::from_utf8_lossy(&listed.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, ["alice@example.org", "bob@example.org"]);
    assert!(listed.stderr.is_empty());
    assert!(
        !seeded_dir.path().join("logs").exists(),
        "Admin CLI must not create log files"
    );
}

#[tokio::test]
async fn mixed_batch_reset_leaves_existing_passwords_unchanged() {
    let dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    prepare_database(dir.path(), &[("alice@example.org", "old-alice")]).await;

    let failed = run_bin(
        dir.path(),
        &[
            "--reset",
            "alice@example.org",
            "--reset",
            "missing@example.org",
            "--newpassword",
            "new-secret",
        ],
    );
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .contains("User does not exist: missing@example.org")
    );
    assert!(
        authenticate(dir.path(), "alice@example.org", "old-alice")
            .await
            .is_some(),
        "Failed batch reset must leave existing password unchanged"
    );
}

#[tokio::test]
async fn reset_updates_passwords_and_supports_combined_list_users() {
    let dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    prepare_database(
        dir.path(),
        &[
            ("alice@example.org", "old-alice"),
            ("bob@example.org", "old-bob"),
        ],
    )
    .await;

    // Case-insensitive match, duplicate email deduplication, leading '-' password, and --list-users.
    let ok = run_bin(
        dir.path(),
        &[
            "--list-users",
            "--reset=ALICE@example.org",
            "--reset",
            "alice@example.org",
            "--reset",
            "bob@example.org",
            "--newpassword=-new-secret",
        ],
    );
    assert!(
        ok.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(ok.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&ok.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [
            "alice@example.org",
            "bob@example.org",
            "Reset password for user: alice@example.org",
            "Reset password for user: bob@example.org",
        ]
    );

    let alice = authenticate(dir.path(), "alice@example.org", "-new-secret")
        .await
        .expect("Alice must authenticate with the new password");
    let bob = authenticate(dir.path(), "bob@example.org", "-new-secret")
        .await
        .expect("Bob must authenticate with the new password");
    assert_ne!(
        alice.password_hash, bob.password_hash,
        "Each user must receive a distinct salted bcrypt hash"
    );
}

#[test]
fn admin_cli_fails_fast_without_creating_or_migrating_database() {
    let missing_dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    let db_path = missing_dir.path().join("database.sqlite");

    let output = run_bin(missing_dir.path(), &["--list-users"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Main database does not exist"));
    assert!(
        !db_path.exists(),
        "Admin CLI must not create a missing database"
    );
    assert!(!missing_dir.path().join("tasks.sqlite").exists());

    // An empty unmigrated file must fail without being migrated.
    fs::write(&db_path, b"").expect("Failed to create empty file");
    let unmigrated = run_bin(missing_dir.path(), &["--list-users"]);
    assert_eq!(unmigrated.status.code(), Some(1));
    assert_eq!(
        fs::metadata(&db_path).expect("File metadata").len(),
        0,
        "Admin CLI must not run migrations on an uninitialized database"
    );
}

async fn run_bin_async(config_dir: &Path, args: &[&'static str]) -> std::process::Output {
    let dir = config_dir.to_path_buf();
    let args = args.to_vec();
    tokio::task::spawn_blocking(move || run_bin(&dir, &args))
        .await
        .expect("spawn_blocking failed")
}

#[tokio::test]
async fn admin_cli_works_while_server_holds_directory_lock() {
    let dir = komga_test_support::tempdir().expect("Failed to create temp dir");
    let config_dir = dir.path().to_path_buf();
    prepare_database(&config_dir, &[("alice@example.org", "old-secret")]).await;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(komga_rust::app::serve_with_shutdown(
        crate::support::app_config(&config_dir),
        listener,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    let lock_path = config_dir.join(".komga.lock");
    timeout(Duration::from_secs(5), async {
        while !lock_path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Server did not acquire directory lock in time");

    let listed = run_bin_async(&config_dir, &["--list-users"]).await;
    assert!(listed.status.success());
    assert_eq!(
        String::from_utf8_lossy(&listed.stdout).trim(),
        "alice@example.org"
    );

    let reset = run_bin_async(
        &config_dir,
        &["--reset=alice@example.org", "--newpassword=live-new-secret"],
    )
    .await;
    assert!(reset.status.success());
    assert_eq!(
        String::from_utf8_lossy(&reset.stdout).trim(),
        "Reset password for user: alice@example.org"
    );

    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("Server shutdown timed out")
        .expect("Server task panicked")
        .expect("Server returned an error");

    assert!(
        authenticate(&config_dir, "alice@example.org", "live-new-secret")
            .await
            .is_some(),
        "Password reset while server was running must persist"
    );
}
