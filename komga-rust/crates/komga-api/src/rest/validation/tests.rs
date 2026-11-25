use super::{client_setting_key, is_hibernate_email, matches_declared_email_pattern};

fn is_valid(value: &str) -> bool {
    is_hibernate_email(value) && matches_declared_email_pattern(value)
}

#[test]
fn accepts_baseline_email_shapes() {
    for value in [
        "reader@example.com",
        "first.last+tag@sub.example.org",
        "\"quoted name\"@example.com",
        "\"a\\\"b\"@example.com",
        "用户@例子.广告",
        "user@[192.168.0.1]",
    ] {
        assert!(is_valid(value), "{value} should be accepted");
    }
}

#[test]
fn rejects_values_hibernate_or_declared_pattern_reject() {
    for value in [
        "",
        "reader",
        "reader@localhost",
        "@example.com",
        "reader@example.",
        "reader@.example.com",
        "reader@-example.com",
        "re..ader@example.com",
        ".reader@example.com",
        "\"\"@example.com",
        "re\\ader@example.com",
        "reader@exa mple.com",
        "user@[192.168.0.1].x",
        "reader@example.com\n",
        &format!("{}@example.com", "a".repeat(65)),
        &format!("reader@{}.com", "a".repeat(64)),
    ] {
        assert!(!is_valid(value), "{value:?} should be rejected");
    }
}

/// Key samples from the baseline `ClientSettingsControllerTest`.
#[test]
fn client_setting_keys_match_baseline_pattern() {
    let is_valid = |key: &str| {
        let mut violations = Vec::new();
        client_setting_key("key", key, &mut violations);
        violations.is_empty()
    };
    for key in [
        "single",
        "one.two",
        "one.with-dash",
        "one.with_underscore",
        "one.two.three-four_five",
        "start2",
        "start2.0value",
        "start2.value2",
        "start_2.value2",
        "start-2.value2",
        "a.0",
    ] {
        assert!(is_valid(key), "{key:?} should be accepted");
    }
    for key in [
        "UPPERCASE",
        "   ",
        "",
        "symbols!",
        "two..dots",
        ".start.with.dot",
        "end.with.dot.",
        "setting.-secondstartwithdash",
        "setting.-secondstartwithunderscore",
        "setting.secondendwithdash-",
        "setting.secondendwithunderscore_",
        "-first",
        "_first",
        "first-",
        "first_",
        ".",
        "0first",
        "café",
    ] {
        assert!(!is_valid(key), "{key:?} should be rejected");
    }
}
