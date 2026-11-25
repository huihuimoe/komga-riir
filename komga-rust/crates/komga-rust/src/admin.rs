//! One-shot administrative CLI operations (`--list-users`, `--reset`).

use anyhow::{Context, Result, bail};
use komga_db::Database;
use komga_service::{EventBus, IdentityService, SessionRegistry};
use komga_types::UserId;

use crate::app::local_filesystem;
use crate::config::{AdminCommand, AdminConfig, DATABASE_CHECK_LOCAL_FILESYSTEM, PasswordReset};

/// Execute the requested one-shot admin command, writing output to standard output.
pub async fn run(config: &AdminConfig, command: &AdminCommand) -> Result<()> {
    let db_path = &config.database_file;
    if config.check_local_filesystem {
        local_filesystem::check_database_is_local(db_path, DATABASE_CHECK_LOCAL_FILESYSTEM)?;
    }
    if !db_path.is_file() {
        bail!("Main database does not exist: {}", db_path.display());
    }

    let db = if command.reset_password.is_none() {
        Database::connect_existing_read_only(db_path).await
    } else {
        Database::connect_existing(db_path).await
    }
    .with_context(|| format!("Failed to open database {}", db_path.display()))?;

    let identity = IdentityService::new(db.clone(), SessionRegistry::new(), EventBus::default());
    let outcome = execute(&identity, command).await;
    db.close().await;
    outcome
}

async fn execute(identity: &IdentityService, command: &AdminCommand) -> Result<()> {
    if command.list_users {
        list_users(identity).await?;
    }
    if let Some(reset) = &command.reset_password {
        reset_passwords(identity, reset).await?;
    }
    Ok(())
}

async fn list_users(identity: &IdentityService) -> Result<()> {
    let users = identity
        .list_users()
        .await
        .context("Failed to list users")?;
    if users.is_empty() {
        println!("No users exist yet");
    } else {
        for user in users {
            println!("{}", user.email);
        }
    }
    Ok(())
}

async fn reset_passwords(identity: &IdentityService, reset: &PasswordReset) -> Result<()> {
    let mut targets: Vec<(UserId, String)> = Vec::with_capacity(reset.emails.len());
    let mut missing: Vec<&str> = Vec::new();

    for email in &reset.emails {
        let user = identity
            .find_by_email(email)
            .await
            .with_context(|| format!("Failed to look up user {email}"))?;
        match user {
            Some(user) => {
                if !targets.iter().any(|(id, _)| *id == user.id) {
                    targets.push((user.id, user.email));
                }
            }
            None => missing.push(email.as_str()),
        }
    }

    if !missing.is_empty() {
        let message = missing
            .into_iter()
            .map(|email| format!("User does not exist: {email}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!("{message}");
    }

    let user_ids: Vec<UserId> = targets.iter().map(|(id, _)| id.clone()).collect();
    identity
        .update_passwords(&user_ids, &reset.new_password)
        .await
        .context("Failed to update user passwords")?;

    for (_, email) in targets {
        println!("Reset password for user: {email}");
    }
    Ok(())
}
