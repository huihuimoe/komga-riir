//! Keys that komga-rust does not honor.

use komga_rust::config::{AppConfig, ConfigError, ConfigSource, IgnoredSetting};

use super::config::{args_for, config_dir_with, resolve};

pub(super) fn ignored_keys(config: &AppConfig) -> Vec<&str> {
    config
        .ignored
        .iter()
        .map(|ignored| ignored.key.as_str())
        .collect()
}

#[test]
fn unsupported_keys_are_ignored_and_oauth2_ones_rejected() {
    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  kobo:\n    kepubify-path: /usr/bin/kepubify\nmanagement.endpoints: x\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Unsupported file keys are ignored");
    let file = ConfigSource::File(dir.path().join("application.yml"));
    assert_eq!(
        config.ignored,
        [
            IgnoredSetting {
                key: "komga.kobo.kepubify-path".to_owned(),
                source: file.clone(),
            },
            IgnoredSetting {
                key: "management.endpoints".to_owned(),
                source: file,
            },
        ]
    );

    let dir = config_dir_with(&[]);
    for name in ["KOMGA_KOBO_KEPUBIFYPATH", "KOMGA_DATABASE_PRAGMAS_FOO"] {
        let config = resolve(&args_for(dir.path()), &[(name, "x")]).expect(name);
        assert_eq!(
            config
                .ignored
                .iter()
                .map(|ignored| &ignored.source)
                .collect::<Vec<_>>(),
            [&ConfigSource::Env(name.to_owned())]
        );
    }
    for name in [
        "SPRING_SECURITY_OAUTH2_CLIENT_REGISTRATION_GOOGLE_CLIENTFOO",
        "SPRING_SECURITY_OAUTH2_CLIENT_REGISTRATION_GOOGLE_ISSUER_URI",
    ] {
        let error = resolve(&args_for(dir.path()), &[(name, "x")]).expect_err(name);
        assert!(
            matches!(error, ConfigError::Unsupported { ref origin, .. } if *origin == ConfigSource::Env(name.to_string())),
            "{name}: {error}"
        );
    }
    resolve(
        &args_for(dir.path()),
        &[("KOMGA_RUST_MODE", "x"), ("PATH", "/bin")],
    )
    .map(|config| assert!(config.ignored.is_empty()))
    .expect("Variables outside the baseline registry are not configuration");
}
