use komga_application::identity_access::AuthUser;

use crate::contracts::identity_access::UserDto;

pub(crate) fn user_payload(user: &AuthUser) -> UserDto {
    UserDto::from_user(user)
}
