//! Authentication activity recording, retrieval and retention.

use jiff::{Timestamp, ToSpan, tz::TimeZone};
use komga_db::{AuthenticationActivityDao, KomgaUserDao};
use komga_types::{
    ActivitySort, AuthenticationActivity, KomgaUser, PageRequest, PageResult, RecordActivityParams,
    SortOrder,
};
use tracing::{info, warn};

use super::{IdentityError, IdentityService};

impl IdentityService {
    /// Records an authentication attempt (`LoginListener`); a failure to store it is logged and
    /// does not fail the request.
    pub async fn record_activity(&self, params: RecordActivityParams<'_>) {
        let result = async {
            let mut transaction = self.db.begin_immediate().await?;
            AuthenticationActivityDao::insert(&mut transaction, &params).await?;
            transaction.commit().await
        }
        .await;
        if let Err(error) = result {
            warn!(error = %error, "Failed to record the authentication activity");
        }
    }

    /// The requested page of all activity, or of `user`'s (by id or email).
    pub async fn activities(
        &self,
        user: Option<&KomgaUser>,
        sort: &[SortOrder<ActivitySort>],
        page: PageRequest,
    ) -> Result<PageResult<AuthenticationActivity>, IdentityError> {
        Ok(AuthenticationActivityDao::find_page(&self.db, user, sort, page).await?)
    }

    /// The most recent activity of a user, with `api_key_id` only that key's.
    ///
    /// # Errors
    /// [`IdentityError::UserNotFound`] when the user does not exist.
    pub async fn latest_activity(
        &self,
        user_id: &str,
        api_key_id: Option<&str>,
    ) -> Result<Option<AuthenticationActivity>, IdentityError> {
        let user = KomgaUserDao::find_by_id(&self.db, user_id)
            .await?
            .ok_or(IdentityError::UserNotFound)?;
        Ok(AuthenticationActivityDao::find_latest(&self.db, &user, api_key_id).await?)
    }

    /// `AuthenticationActivityCleanupController`: deletes the activity older than a month (UTC).
    pub async fn delete_old_activity(&self) -> Result<(), IdentityError> {
        let now = Timestamp::now().to_zoned_datetime(TimeZone::UTC).datetime();
        let cutoff = now.checked_sub(1.month()).expect("A month ago is in range");
        info!(older_than = %cutoff, "Removing authentication activity older than a month (UTC)");
        let mut transaction = self.db.begin_immediate().await?;
        AuthenticationActivityDao::delete_older_than(&mut transaction, cutoff).await?;
        transaction.commit().await?;
        Ok(())
    }
}
