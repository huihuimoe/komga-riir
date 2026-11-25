//! Kotlin `OpdsAuthDto.kt`, as `OpdsGenerator.generateOpdsAuthDocument` fills it.

use serde::Serialize;

/// `AuthenticationDocumentDto`.
#[derive(Serialize)]
pub(crate) struct AuthenticationDocument {
    authentication: [AuthenticationFlow; 1],
    title: &'static str,
    id: String,
    description: &'static str,
    links: [Link; 2],
}

#[derive(Serialize)]
struct AuthenticationFlow {
    #[serde(rename = "type")]
    flow_type: &'static str,
    labels: Labels,
}

#[derive(Serialize)]
struct Labels {
    login: &'static str,
    password: &'static str,
}

#[derive(Serialize)]
struct Link {
    rel: &'static str,
    href: String,
}

impl AuthenticationDocument {
    /// The document for the server at `context`.
    pub(crate) fn new(context: &str) -> Self {
        Self {
            authentication: [AuthenticationFlow {
                flow_type: "http://opds-spec.org/auth/basic",
                labels: Labels {
                    login: "Email",
                    password: "Password",
                },
            }],
            title: "Komga",
            id: format!("{context}/opds/v2/auth"),
            description: "Enter your email and password to authenticate.",
            links: [
                Link {
                    rel: "help",
                    href: "https://komga.org".to_owned(),
                },
                Link {
                    rel: "logo",
                    href: format!("{context}/android-chrome-512x512.png"),
                },
            ],
        }
    }
}
