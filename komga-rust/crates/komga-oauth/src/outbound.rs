//! Outbound HTTPS clients. Lives here because this is the lowest crate that every outbound caller
//! (OAuth providers, the Kobo store proxy, the update check) depends on.

use reqwest::{Certificate, Client, ClientBuilder};
use tracing::warn;

/// Builds `configure(Client::builder())` with the system CA roots. A system without any (a minimal
/// container image without a CA bundle) gets the bundled Mozilla roots instead, as the JVM's own
/// `cacerts` serves the baseline.
///
/// # Panics
/// When the TLS backend cannot be set up even with the bundled roots, which no configuration can
/// fix.
pub fn http_client(configure: impl Fn(ClientBuilder) -> ClientBuilder) -> Client {
    configure(Client::builder())
        .build()
        .unwrap_or_else(|error| {
            warn!(
                ?error,
                "Cannot use the system CA certificates; using the bundled ones"
            );
            let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
                .iter()
                .map(|der| Certificate::from_der(der).expect("Bundled CA certificate"));
            configure(Client::builder())
                .tls_certs_only(roots)
                .build()
                .expect("HTTP client")
        })
}
