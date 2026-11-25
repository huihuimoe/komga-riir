//! Servlet context path syntax validation.

/// `@Pattern(regexp = "^/[\\w-/]*[a-zA-Z0-9]$")` on the context path; Java `\w` is ASCII-only.
/// Both request validation and server startup apply this rule to configured and stored paths.
#[must_use]
pub fn is_context_path(value: &str) -> bool {
    value.strip_prefix('/').is_some_and(|rest| {
        rest.chars()
            .last()
            .is_some_and(|last| last.is_ascii_alphanumeric())
            && rest
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-/".contains(character))
    })
}
