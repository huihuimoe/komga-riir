//! Binary entrypoint for Komga.

use clap::Parser;
use komga_rust::config::{AdminCommand, AdminConfig, AppConfig, CliArgs, Environment};
use std::process::ExitCode;
use tracing::{error, info, warn};

#[global_allocator]
static GLOBAL_ALLOCATOR: rustfs_mimalloc::MiMalloc = rustfs_mimalloc::MiMalloc;

fn main() -> ExitCode {
    let runtime = match komga_rust::runtime_builder().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Failed to start the Tokio runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(start());
    // Dropping the runtime would wait for blocking work a timed-out shutdown abandoned.
    runtime.shutdown_background();
    code
}

async fn start() -> ExitCode {
    let args = CliArgs::parse();
    if let Some(command) = args.admin_command() {
        return run_admin(&args, &command).await;
    }

    print_banner();

    // The log file is part of the configuration, so configuration errors go to standard error.
    let config = match AppConfig::resolve(&args, &Environment::from_process()) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("Configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = komga_rust::logging::init(&config.log_file.value) {
        eprintln!(
            "Cannot write the log file {}: {err}",
            config.log_file.value.display()
        );
        return ExitCode::FAILURE;
    }

    info!(value = %config.config_dir.value.display(), source = %config.config_dir.source, "komga.config-dir");
    info!(value = %config.database_file.value.display(), source = %config.database_file.source, "komga.database.file");
    info!(value = config.check_local_filesystem.value, source = %config.check_local_filesystem.source, "komga.database.check-local-filesystem");
    info!(value = %config.tasks_database_file.value.display(), source = %config.tasks_database_file.source, "komga.tasks-db.file");
    info!(value = config.tasks_check_local_filesystem.value, source = %config.tasks_check_local_filesystem.source, "komga.tasks-db.check-local-filesystem");
    info!(value = %config.fonts_data_directory.value.display(), source = %config.fonts_data_directory.source, "komga.fonts.data-directory");
    info!(value = %config.log_file.value.display(), source = %config.log_file.source, "logging.file.name");
    match &config.host {
        Some(host) => info!(value = %host.value, source = %host.source, "server.address"),
        None => info!(
            value = ":: (fallback 0.0.0.0)",
            source = "default",
            "server.address"
        ),
    }
    info!(value = config.port.value, source = %config.port.source, "server.port");
    if let Some(path) = &config.context_path {
        info!(value = %path.value, source = %path.source, "server.servlet.context-path");
    }
    info!(value = config.epub_divina_letter_count_threshold.value, source = %config.epub_divina_letter_count_threshold.source, "komga.epub-divina-letter-count-threshold");
    info!(value = config.page_hashing.value, source = %config.page_hashing.source, "komga.page-hashing");
    info!(value = config.kobo_sync_item_limit.value, source = %config.kobo_sync_item_limit.source, "komga.kobo.sync-item-limit");
    info!(value = ?config.session_timeout.value, source = %config.session_timeout.source, "spring.session.timeout");
    let lucene = &config.lucene;
    info!(value = %lucene.data_directory.value.display(), source = %lucene.data_directory.source, "komga.lucene.data-directory");
    info!(value = lucene.min_gram.value, source = %lucene.min_gram.source, "komga.lucene.index-analyzer.min-gram");
    info!(value = lucene.max_gram.value, source = %lucene.max_gram.source, "komga.lucene.index-analyzer.max-gram");
    info!(value = lucene.preserve_original.value, source = %lucene.preserve_original.source, "komga.lucene.index-analyzer.preserve-original");
    info!(value = ?lucene.commit_delay.value, source = %lucene.commit_delay.source, "komga.lucene.commit-delay");
    let sort = &config.sort;
    info!(value = sort.locale.value.tag(), source = %sort.locale.source, "komga.sort.locale");
    for (library, locale) in &sort.library_locales.value {
        info!(%library, value = locale.tag(), source = %sort.library_locales.source, "komga.sort.library-locales");
    }
    for ignored in &config.ignored {
        warn!(source = %ignored.source, "{} is not supported by komga-rust and is ignored", ignored.key);
    }
    info!("Starting Komga");

    if args.dry_run {
        info!("Dry run requested, exiting cleanly");
        return ExitCode::SUCCESS;
    }

    match komga_rust::app::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(error = %format!("{err:#}"), "Komga server failed");
            eprintln!("Server error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run_admin(args: &CliArgs, command: &AdminCommand) -> ExitCode {
    let config = match AdminConfig::resolve(args, &Environment::from_process()) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("Configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };
    match komga_rust::admin::run(&config, command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_banner() {
    const BANNER: &str = include_str!("../../../../komga/src/main/resources/banner.txt");
    println!(
        "{}",
        BANNER
            .trim_end()
            .replace("${application.version}", komga_rust::BUILD_INFO.version)
    );
}
