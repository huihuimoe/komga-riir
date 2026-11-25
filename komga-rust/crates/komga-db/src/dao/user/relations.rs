use std::collections::{BTreeSet, HashMap};

use crate::iden::{UserLibrarySharingTable, UserRoleTable, UserSharingTable};
use crate::support::rows::BATCH_CHUNK_SIZE;
use komga_types::{KomgaUser, LibraryId, UserId, UserRole};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::{AssertSqlSafe, Row, SqliteConnection, SqlitePool};

pub(super) async fn find_roles_by_user_ids(
    pool: &SqlitePool,
    user_ids: &[UserId],
) -> Result<HashMap<UserId, BTreeSet<UserRole>>, sqlx::Error> {
    let mut by_user: HashMap<UserId, BTreeSet<UserRole>> = HashMap::new();
    for chunk in user_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([UserRoleTable::UserId, UserRoleTable::Role])
            .from(UserRoleTable::Table)
            .and_where(Expr::col(UserRoleTable::UserId).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);

        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(pool)
            .await?;
        for row in rows {
            let user_id: UserId = row.try_get("USER_ID")?;
            let role: String = row.try_get("ROLE")?;
            if let Ok(role) = role.parse() {
                by_user.entry(user_id).or_default().insert(role);
            }
        }
    }
    Ok(by_user)
}

pub(super) async fn find_shared_library_ids_by_user_ids(
    pool: &SqlitePool,
    user_ids: &[UserId],
) -> Result<HashMap<UserId, BTreeSet<LibraryId>>, sqlx::Error> {
    let mut by_user: HashMap<UserId, BTreeSet<LibraryId>> = HashMap::new();
    for chunk in user_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([
                UserLibrarySharingTable::UserId,
                UserLibrarySharingTable::LibraryId,
            ])
            .from(UserLibrarySharingTable::Table)
            .and_where(Expr::col(UserLibrarySharingTable::UserId).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);

        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(pool)
            .await?;
        for row in rows {
            let user_id: UserId = row.try_get("USER_ID")?;
            let library_id: LibraryId = row.try_get("LIBRARY_ID")?;
            by_user.entry(user_id).or_default().insert(library_id);
        }
    }
    Ok(by_user)
}

/// Returns `(allowed, excluded)` sharing labels per user ID.
pub(super) async fn find_sharing_labels_by_user_ids(
    pool: &SqlitePool,
    user_ids: &[UserId],
) -> Result<HashMap<UserId, (Vec<String>, Vec<String>)>, sqlx::Error> {
    let mut by_user: HashMap<UserId, (Vec<String>, Vec<String>)> = HashMap::new();
    for chunk in user_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([
                UserSharingTable::UserId,
                UserSharingTable::Label,
                UserSharingTable::Allow,
            ])
            .from(UserSharingTable::Table)
            .and_where(Expr::col(UserSharingTable::UserId).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);

        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(pool)
            .await?;
        for row in rows {
            let user_id: UserId = row.try_get("USER_ID")?;
            let allow: bool = row.try_get("ALLOW")?;
            let label: String = row.try_get("LABEL")?;
            let (allowed, excluded) = by_user.entry(user_id).or_default();
            if allow {
                allowed.push(label);
            } else {
                excluded.push(label);
            }
        }
    }
    Ok(by_user)
}

pub(super) async fn insert_user_relations(
    conn: &mut SqliteConnection,
    user: &KomgaUser,
) -> Result<(), sqlx::Error> {
    let user_id = &user.id;
    let mut statements = Vec::new();
    for role in &user.roles {
        statements.push(
            Query::insert()
                .into_table(UserRoleTable::Table)
                .columns([UserRoleTable::UserId, UserRoleTable::Role])
                .values_panic([user_id.into(), role.as_str().into()])
                .to_owned(),
        );
    }
    for library_id in &user.shared_library_ids {
        statements.push(
            Query::insert()
                .into_table(UserLibrarySharingTable::Table)
                .columns([
                    UserLibrarySharingTable::UserId,
                    UserLibrarySharingTable::LibraryId,
                ])
                .values_panic([user_id.into(), library_id.into()])
                .to_owned(),
        );
    }
    for (allow, labels) in [
        (true, user.restrictions.labels_allow()),
        (false, user.restrictions.labels_exclude()),
    ] {
        for label in labels {
            statements.push(
                Query::insert()
                    .into_table(UserSharingTable::Table)
                    .columns([
                        UserSharingTable::UserId,
                        UserSharingTable::Allow,
                        UserSharingTable::Label,
                    ])
                    .values_panic([user_id.into(), allow.into(), label.as_str().into()])
                    .to_owned(),
            );
        }
    }

    for statement in statements {
        let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
