use std::fs;
use std::path::Path;
use std::time::Duration;

use komga_rust::config::{AppConfig, CliArgs, ConfigError, ConfigSource, Environment};
use komga_search::NgramOptions;
use komga_test_support::TestDir;

use super::ignored_config::ignored_keys;

pub(super) fn resolve(args: &CliArgs, env: &[(&str, &str)]) -> Result<AppConfig, ConfigError> {
    AppConfig::resolve(args, &env.iter().copied().collect::<Environment>())
}

pub(super) fn args_for(config_dir: &Path) -> CliArgs {
    CliArgs {
        config_dir: Some(config_dir.to_path_buf()),
        ..CliArgs::default()
    }
}

pub(super) fn config_dir_with(files: &[(&str, &str)]) -> TestDir {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    for (name, contents) in files {
        fs::write(dir.path().join(name), contents).expect("Failed to write config file");
    }
    dir
}

#[test]
fn defaults_derive_from_home_directory() {
    let config = resolve(&CliArgs::default(), &[("HOME", "/home/reader")])
        .expect("Defaults must resolve when HOME is known");

    assert_eq!(config.config_dir.value, Path::new("/home/reader/.komga"));
    assert_eq!(config.config_dir.source, ConfigSource::Default);
    assert_eq!(
        config.database_file.value,
        Path::new("/home/reader/.komga/database.sqlite")
    );
    assert_eq!(config.database_file.source, ConfigSource::Default);
    assert!(config.host.is_none());
    assert_eq!(config.port.value, 25600);
    assert_eq!(config.port.source, ConfigSource::Default);

    assert!(matches!(
        resolve(&CliArgs::default(), &[]),
        Err(ConfigError::NoHomeDirectory)
    ));
}

#[test]
fn cli_overrides_environment_which_overrides_files() {
    let dir = config_dir_with(&[(
        "application.yml",
        "server:\n  port: 1111\n  address: 127.0.0.3\n",
    )]);
    let yml = dir.path().join("application.yml");
    let env = [("SERVER_PORT", "2222"), ("SERVER_ADDRESS", "127.0.0.2")];
    let cli = CliArgs {
        port: Some(3333),
        host: Some("127.0.0.1".to_string()),
        ..args_for(dir.path())
    };

    let config = resolve(&cli, &env).expect("Layered sources must resolve");
    assert_eq!(config.port.value, 3333);
    assert_eq!(config.port.source, ConfigSource::Cli("--port"));
    let host = config.host.expect("CLI host must resolve");
    assert_eq!(host.value, "127.0.0.1");
    assert_eq!(host.source, ConfigSource::Cli("--host"));
    assert_eq!(config.config_dir.source, ConfigSource::Cli("--config-dir"));

    let config = resolve(&args_for(dir.path()), &env).expect("Environment must resolve");
    assert_eq!(config.port.value, 2222);
    assert_eq!(
        config.port.source,
        ConfigSource::Env("SERVER_PORT".to_string())
    );
    assert_eq!(
        config.host.expect("Env host must resolve").value,
        "127.0.0.2"
    );

    let config = resolve(&args_for(dir.path()), &[]).expect("File values must resolve");
    assert_eq!(config.port.value, 1111);
    assert_eq!(config.port.source, ConfigSource::File(yml.clone()));
    let host = config.host.expect("File host must resolve");
    assert_eq!(host.value, "127.0.0.3");
    assert_eq!(host.source, ConfigSource::File(yml));
}

#[test]
fn later_imported_files_win_and_relaxed_names_bind() {
    let dir = config_dir_with(&[
        ("application.yml", "server.port: 1111\n"),
        (
            "application.yaml",
            "komga:\n  database:\n    file: ${komga.config-dir}/db/main.sqlite\n",
        ),
        ("application.properties", "server.port=2222\n"),
    ]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Files must resolve");
    assert_eq!(config.port.value, 2222);
    assert_eq!(
        config.port.source,
        ConfigSource::File(dir.path().join("application.properties"))
    );
    assert_eq!(
        config.database_file.value,
        dir.path().join("db/main.sqlite")
    );

    let dir = config_dir_with(&[(
        "application.properties",
        "komga.database_file=${user.home}/k.db\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[("HOME", "/home/reader")])
        .expect("Unknown keys are ignored");
    assert_eq!(
        ignored_keys(&config),
        ["komga.database_file"],
        "database_file is not a relaxed form of database.file"
    );

    let dir = config_dir_with(&[(
        "application.properties",
        "komga.Database.File=${user.home}/k.db\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[("HOME", "/home/reader")])
        .expect("Relaxed case must bind");
    assert_eq!(config.database_file.value, Path::new("/home/reader/k.db"));
}

#[test]
fn config_dir_accepts_both_baseline_variables_but_not_conflicts() {
    let config = resolve(
        &CliArgs::default(),
        &[("KOMGA_CONFIGDIR", "/data"), ("KOMGA_CONFIG_DIR", "/data")],
    )
    .expect("Docker sets both variables to the same value");
    assert_eq!(config.config_dir.value, Path::new("/data"));
    assert_eq!(
        config.config_dir.source,
        ConfigSource::Env("KOMGA_CONFIGDIR".to_string())
    );
    assert_eq!(
        config.database_file.value,
        Path::new("/data/database.sqlite")
    );

    let config = resolve(&CliArgs::default(), &[("KOMGA_CONFIG_DIR", "/other")])
        .expect("Placeholder variable alone must resolve");
    assert_eq!(
        config.config_dir.source,
        ConfigSource::Env("KOMGA_CONFIG_DIR".to_string())
    );

    let error = resolve(
        &CliArgs::default(),
        &[("KOMGA_CONFIGDIR", "/data"), ("KOMGA_CONFIG_DIR", "/other")],
    )
    .expect_err("Different values must conflict");
    assert_eq!(
        error.to_string(),
        "environment variables KOMGA_CONFIGDIR and KOMGA_CONFIG_DIR set different values for komga.config-dir"
    );
}

#[test]
fn file_conflicts_and_config_dir_in_file_are_rejected() {
    let dir = config_dir_with(&[("application.yml", "server:\n  port: 1\nserver.port: 2\n")]);
    assert!(matches!(
        resolve(&args_for(dir.path()), &[]),
        Err(ConfigError::FileConflict { .. })
    ));

    let dir = config_dir_with(&[("application.yml", "komga:\n  configDir: /elsewhere\n")]);
    let error =
        resolve(&args_for(dir.path()), &[]).expect_err("config-dir cannot come from its own file");
    assert!(
        matches!(error, ConfigError::ConfigDirInFile { ref key, .. } if key == "komga.configDir")
    );
}

#[test]
fn invalid_values_name_key_and_source() {
    let dir = config_dir_with(&[("application.properties", "server.port=abc\n")]);
    let error = resolve(&args_for(dir.path()), &[]).expect_err("Non-numeric port");
    assert_eq!(
        error.to_string(),
        format!(
            "invalid value \"abc\" for server.port from file {}: expected a port between 1 and 65535",
            dir.path().join("application.properties").display()
        )
    );

    let dir = config_dir_with(&[]);
    for port in ["0", "65536", "-1", ""] {
        let error = resolve(&args_for(dir.path()), &[("SERVER_PORT", port)])
            .expect_err("Out-of-range port");
        assert!(
            matches!(error, ConfigError::InvalidValue { key: "server.port", ref origin, .. } if *origin == ConfigSource::Env("SERVER_PORT".to_string())),
            "{port}: {error}"
        );
    }
    let cli = CliArgs {
        port: Some(0),
        ..args_for(dir.path())
    };
    assert!(matches!(
        resolve(&cli, &[]),
        Err(ConfigError::InvalidValue {
            key: "server.port",
            origin: ConfigSource::Cli("--port"),
            ..
        })
    ));
    let cli = CliArgs {
        host: Some("  ".to_string()),
        ..args_for(dir.path())
    };
    assert!(matches!(
        resolve(&cli, &[]),
        Err(ConfigError::InvalidValue {
            key: "server.address",
            ..
        })
    ));
    for file in [
        "${spring.home}/db.sqlite",
        "${komga.config-dir/db.sqlite",
        " ",
    ] {
        assert!(
            matches!(
                resolve(&args_for(dir.path()), &[("KOMGA_DATABASE_FILE", file)]),
                Err(ConfigError::InvalidValue {
                    key: "komga.database.file",
                    ..
                })
            ),
            "{file}"
        );
    }
}

#[test]
fn malformed_files_are_reported() {
    for contents in [
        "server: [1\n",
        "server.port: 1\n---\nserver.port: 2\n",
        "- 1\n",
    ] {
        let dir = config_dir_with(&[("application.yml", contents)]);
        assert!(
            matches!(
                resolve(&args_for(dir.path()), &[]),
                Err(ConfigError::ParseFile { .. })
            ),
            "{contents}"
        );
    }

    let dir = config_dir_with(&[]);
    fs::create_dir(dir.path().join("application.yaml")).expect("Failed to create directory");
    assert!(matches!(
        resolve(&args_for(dir.path()), &[]),
        Err(ConfigError::ReadFile { .. })
    ));
}

#[cfg(unix)]
#[test]
fn non_unicode_variables_are_rejected() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let env: Environment = [("SERVER_PORT", OsString::from_vec(vec![0xff]))]
        .into_iter()
        .collect();
    let dir = config_dir_with(&[]);
    assert!(matches!(
        AppConfig::resolve(&args_for(dir.path()), &env),
        Err(ConfigError::NonUnicodeEnv { ref name }) if name == "SERVER_PORT"
    ));
}

#[test]
fn check_local_filesystem_maps_spring_booleans() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default check");
    assert!(config.check_local_filesystem.value);
    assert_eq!(config.check_local_filesystem.source, ConfigSource::Default);

    for (value, expected) in [
        ("false", false),
        (" OFF ", false),
        ("no", false),
        ("0", false),
        ("Yes", true),
        ("on", true),
        ("1", true),
    ] {
        let config = resolve(
            &args_for(dir.path()),
            &[("KOMGA_DATABASE_CHECKLOCALFILESYSTEM", value)],
        )
        .expect(value);
        assert_eq!(config.check_local_filesystem.value, expected, "{value:?}");
    }

    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  database:\n    check-local-filesystem: false\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[]).expect("File check");
    assert!(!config.check_local_filesystem.value);
    assert_eq!(
        config.check_local_filesystem.source,
        ConfigSource::File(dir.path().join("application.yml"))
    );

    let dir = config_dir_with(&[]);
    for value in ["maybe", ""] {
        assert!(
            matches!(
                resolve(
                    &args_for(dir.path()),
                    &[("KOMGA_DATABASE_CHECKLOCALFILESYSTEM", value)]
                ),
                Err(ConfigError::InvalidValue {
                    key: "komga.database.check-local-filesystem",
                    ..
                })
            ),
            "{value:?}"
        );
    }
    let config = resolve(
        &args_for(dir.path()),
        &[
            ("KOMGA_TASKSDB_CHECKLOCALFILESYSTEM", "off"),
            (
                "KOMGA_TASKSDB_FILE",
                "${komga.config-dir}/queue/tasks.sqlite",
            ),
        ],
    )
    .expect("Tasks database settings");
    assert!(
        config.check_local_filesystem.value,
        "the main check is separate"
    );
    assert!(!config.tasks_check_local_filesystem.value);
    assert_eq!(
        config.tasks_database_file.value,
        dir.path().join("queue/tasks.sqlite")
    );
    let config = resolve(&args_for(dir.path()), &[]).expect("Defaults");
    assert!(config.tasks_check_local_filesystem.value);
    assert_eq!(
        config.tasks_database_file.value,
        dir.path().join("tasks.sqlite")
    );
    assert!(matches!(
        resolve(&args_for(dir.path()), &[("KOMGA_TASKSDB_FILE", " ")]),
        Err(ConfigError::InvalidValue {
            key: "komga.tasks-db.file",
            ..
        })
    ));
}

#[test]
fn epub_divina_letter_count_threshold_is_an_integer() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default threshold");
    assert_eq!(config.epub_divina_letter_count_threshold.value, 15);
    assert_eq!(
        config.epub_divina_letter_count_threshold.source,
        ConfigSource::Default
    );

    let variable = "KOMGA_EPUBDIVINALETTERCOUNTTHRESHOLD";
    let config = resolve(&args_for(dir.path()), &[(variable, " -3 ")]).expect("Threshold");
    assert_eq!(config.epub_divina_letter_count_threshold.value, -3);

    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  epub-divina-letter-count-threshold: 40\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[]).expect("File threshold");
    assert_eq!(config.epub_divina_letter_count_threshold.value, 40);

    for value in ["many", "1.5", ""] {
        assert!(
            matches!(
                resolve(&args_for(dir.path()), &[(variable, value)]),
                Err(ConfigError::InvalidValue {
                    key: "komga.epub-divina-letter-count-threshold",
                    ..
                })
            ),
            "{value:?}"
        );
    }
}

#[test]
fn context_path_is_optional_and_a_plain_route_prefix() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[("SERVER_SERVLET_CONTEXTPATH", "")])
        .expect("Empty context path");
    assert!(config.context_path.is_none());

    let dir = config_dir_with(&[(
        "application.yml",
        "server:\n  servlet:\n    context-path: /komga\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[]).expect("File context path");
    assert_eq!(config.context_path.expect("Context path").value, "/komga");

    for value in ["komga", "/komga/", "/my books"] {
        assert!(
            matches!(
                resolve(
                    &args_for(dir.path()),
                    &[("SERVER_SERVLET_CONTEXTPATH", value)]
                ),
                Err(ConfigError::InvalidValue {
                    key: "server.servlet.context-path",
                    ..
                })
            ),
            "{value:?}"
        );
    }
}

#[test]
fn page_hashing_is_a_positive_integer() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default page hashing");
    assert_eq!(config.page_hashing.value, 3);
    assert_eq!(config.page_hashing.source, ConfigSource::Default);

    let dir = config_dir_with(&[("application.yml", "komga:\n  page-hashing: 5\n")]);
    let config = resolve(&args_for(dir.path()), &[]).expect("File page hashing");
    assert_eq!(config.page_hashing.value, 5);

    for value in ["0", "-1", "many", ""] {
        assert!(
            matches!(
                resolve(&args_for(dir.path()), &[("KOMGA_PAGEHASHING", value)]),
                Err(ConfigError::InvalidValue {
                    key: "komga.page-hashing",
                    ..
                })
            ),
            "{value:?}"
        );
    }
}

#[test]
fn fonts_data_directory_expands_placeholders() {
    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  fonts:\n    data-directory: ${komga.config-dir}/my-fonts\n",
    )]);
    let config = resolve(&args_for(dir.path()), &[]).expect("File fonts directory");
    assert_eq!(
        config.fonts_data_directory.value,
        dir.path().join("my-fonts")
    );

    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default fonts directory");
    assert_eq!(config.fonts_data_directory.value, dir.path().join("fonts"));
    assert!(matches!(
        resolve(&args_for(dir.path()), &[("KOMGA_FONTS_DATADIRECTORY", " ")]),
        Err(ConfigError::InvalidValue {
            key: "komga.fonts.data-directory",
            ..
        })
    ));
}

#[test]
fn log_file_defaults_to_the_logs_directory_and_expands_placeholders() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default log file");
    assert_eq!(
        config.log_file.value,
        dir.path().join("logs").join("komga.log")
    );

    let config = resolve(
        &args_for(dir.path()),
        &[("LOGGING_FILE_NAME", "${komga.config-dir}/other.log")],
    )
    .expect("Environment log file");
    assert_eq!(config.log_file.value, dir.path().join("other.log"));
}

#[test]
fn lucene_settings_bind_and_validate_the_gram_range() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default index settings");
    assert_eq!(
        config.lucene.data_directory.value,
        dir.path().join("lucene")
    );
    assert_eq!(config.lucene.ngram(), NgramOptions::default());

    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  lucene:\n    data-directory: ${komga.config-dir}/index\n    index-analyzer:\n      min-gram: 2\n      max-gram: 4\n",
    )]);
    let env = [("KOMGA_LUCENE_INDEXANALYZER_PRESERVEORIGINAL", "false")];
    let config = resolve(&args_for(dir.path()), &env).expect("Configured index settings");
    assert_eq!(config.lucene.data_directory.value, dir.path().join("index"));
    assert_eq!(
        config.lucene.ngram(),
        NgramOptions::new(2, 4, false).expect("valid range")
    );

    for (name, value, key) in [
        (
            "KOMGA_LUCENE_INDEXANALYZER_MINGRAM",
            "0",
            "komga.lucene.index-analyzer.min-gram",
        ),
        (
            "KOMGA_LUCENE_INDEXANALYZER_MAXGRAM",
            "1",
            "komga.lucene.index-analyzer.max-gram",
        ),
        (
            "KOMGA_LUCENE_DATADIRECTORY",
            " ",
            "komga.lucene.data-directory",
        ),
    ] {
        let error = resolve(&args_for(dir.path()), &[(name, value)]).expect_err(name);
        assert!(
            matches!(error, ConfigError::InvalidValue { key: found, .. } if found == key),
            "{name}: {error}"
        );
    }
}

#[test]
fn lucene_commit_delay_binds_spring_durations() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default commit delay");
    assert_eq!(config.lucene.commit_delay.value, Duration::from_secs(2));

    let dir = config_dir_with(&[("application.yml", "komga.lucene.commit-delay: 500\n")]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Milliseconds by default");
    assert_eq!(config.lucene.commit_delay.value, Duration::from_millis(500));
    for (value, expected) in [("PT1.5S", 1_500), ("3s", 3_000), ("0", 0)] {
        let env = [("KOMGA_LUCENE_COMMITDELAY", value)];
        let config = resolve(&args_for(dir.path()), &env).expect(value);
        assert_eq!(
            config.lucene.commit_delay.value,
            Duration::from_millis(expected),
            "{value}"
        );
    }
    for value in ["2 weeks", "-1s", "PT"] {
        let env = [("KOMGA_LUCENE_COMMITDELAY", value)];
        let error = resolve(&args_for(dir.path()), &env).expect_err(value);
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "komga.lucene.commit-delay",
                    ..
                }
            ),
            "{value}: {error}"
        );
    }
}

#[test]
fn session_timeout_binds_spring_durations_in_seconds() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default session timeout");
    assert_eq!(
        config.session_timeout.value,
        Duration::from_secs(7 * 24 * 3600)
    );

    let dir = config_dir_with(&[("application.yml", "spring.session.timeout: 3600\n")]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Seconds by default");
    assert_eq!(config.session_timeout.value, Duration::from_secs(3600));
    let env = [("SPRING_SESSION_TIMEOUT", "30m")];
    let config = resolve(&args_for(dir.path()), &env).expect("Environment wins");
    assert_eq!(config.session_timeout.value, Duration::from_secs(1800));
    for value in ["0", "-1", "PT0S", "1w"] {
        let env = [("SPRING_SESSION_TIMEOUT", value)];
        let error = resolve(&args_for(dir.path()), &env).expect_err(value);
        assert!(
            matches!(
                error,
                ConfigError::InvalidValue {
                    key: "spring.session.timeout",
                    ..
                }
            ),
            "{value}: {error}"
        );
    }
}

#[test]
fn sort_locales_bind_and_reject_unsupported_tags() {
    let dir = config_dir_with(&[]);
    let config = resolve(&args_for(dir.path()), &[]).expect("Default sort locales");
    assert_eq!(config.sort.locale.value.tag(), "und");
    assert!(config.sort.library_locales.value.is_empty());

    let dir = config_dir_with(&[(
        "application.yml",
        "komga:\n  sort:\n    locale: ja\n    library-locales: 0A=sv, 0B = zh-u-co-stroke\n",
    )]);
    let config = resolve(
        &args_for(dir.path()),
        &[("KOMGA_SORT_LOCALE", "zh-u-co-pinyin")],
    )
    .expect("Configured sort locales");
    assert_eq!(config.sort.locale.value.tag(), "zh-u-co-pinyin");
    let libraries: Vec<(&str, &str)> = config
        .sort
        .library_locales
        .value
        .iter()
        .map(|(library, locale)| (library.as_str(), locale.tag()))
        .collect();
    assert_eq!(libraries, [("0A", "sv"), ("0B", "zh-u-co-stroke")]);

    for (name, value, key) in [
        ("KOMGA_SORT_LOCALE", "zh", "komga.sort.locale"),
        ("KOMGA_SORT_LOCALE", "de", "komga.sort.locale"),
        ("KOMGA_SORT_LOCALE", "", "komga.sort.locale"),
        (
            "KOMGA_SORT_LIBRARYLOCALES",
            "0A",
            "komga.sort.library-locales",
        ),
        (
            "KOMGA_SORT_LIBRARYLOCALES",
            "=sv",
            "komga.sort.library-locales",
        ),
        (
            "KOMGA_SORT_LIBRARYLOCALES",
            "0A=zh",
            "komga.sort.library-locales",
        ),
        (
            "KOMGA_SORT_LIBRARYLOCALES",
            "0A=sv,0A=ja",
            "komga.sort.library-locales",
        ),
        (
            "KOMGA_SORT_LIBRARYLOCALES",
            "0A=sv,",
            "komga.sort.library-locales",
        ),
    ] {
        let error = resolve(&args_for(dir.path()), &[(name, value)]).expect_err(value);
        assert!(
            matches!(error, ConfigError::InvalidValue { key: found, .. } if found == key),
            "{name}={value}: {error}"
        );
    }
}
