use komga_types::AuthenticationActivity;

use super::CredentialContext;
use crate::support::ADMIN_EMAIL;

pub(super) async fn assert_authentication_activity(context: &CredentialContext) {
    let activities = crate::identity::activities(&context.identity).await;
    let admin = Some(context.admin_id.as_str());
    let reader = Some(context.reader_id.as_str());
    let expected = [
        (
            "Password",
            true,
            "203.0.113.10",
            "basic-client/1.0",
            admin,
            Some(ADMIN_EMAIL),
        ),
        (
            "RememberMe",
            true,
            "203.0.113.12",
            "remember-client/1.0",
            admin,
            Some(ADMIN_EMAIL),
        ),
        (
            "ApiKey",
            true,
            "203.0.113.13",
            "api-key-client/1.0",
            reader,
            Some("reader@example.com"),
        ),
        // A wrong password for a known user keeps the user; an unknown email keeps the email.
        (
            "Password",
            false,
            "203.0.113.14",
            "bad-password-client/1.0",
            admin,
            Some(ADMIN_EMAIL),
        ),
        (
            "Password",
            false,
            "203.0.113.16",
            "unknown-password-client/1.0",
            None,
            Some("unknown@example.com"),
        ),
        (
            "ApiKey",
            false,
            "203.0.113.15",
            "bad-key-client/1.0",
            None,
            None,
        ),
    ];
    for (source, success, ip, user_agent, user_id, email) in expected {
        let activity = activity_for_request(&activities, source, success, ip, user_agent);
        assert_eq!(activity.user_id.as_deref(), user_id, "{source} from {ip}");
        assert_eq!(activity.email.as_deref(), email, "{source} from {ip}");
        let error = (!success).then_some("Bad credentials");
        assert_eq!(activity.error.as_deref(), error, "{source} from {ip}");
    }

    let api_key = activity_for_request(
        &activities,
        "ApiKey",
        true,
        "203.0.113.13",
        "api-key-client/1.0",
    );
    assert_eq!(
        api_key.api_key_id.as_deref(),
        Some(context.reader_api_key_id.as_str())
    );
    assert_eq!(api_key.api_key_comment.as_deref(), Some("reader key"));
}

fn activity_for_request<'a>(
    activities: &'a [AuthenticationActivity],
    source: &str,
    success: bool,
    ip: &str,
    user_agent: &str,
) -> &'a AuthenticationActivity {
    activities
        .iter()
        .find(|activity| {
            activity.source.as_deref() == Some(source)
                && activity.success == success
                && activity.ip.as_deref() == Some(ip)
                && activity.user_agent.as_deref() == Some(user_agent)
        })
        .expect("Authentication activity must preserve source and request metadata")
}
