//! Command line arguments and one-shot admin actions for Komga.

use std::path::PathBuf;

use clap::Parser;

/// Command line options for Komga.
#[derive(Debug, Clone, Default, Parser)]
#[command(
    name = "komga-rust",
    about = "Komga server implementation in Rust",
    version,
    after_help = "\
Settings resolve from CLI options, then environment variables, then
<config-dir>/application.{yml,yaml,properties}, then defaults.
Env vars are uppercase with '.' as '_' and '-' dropped or as '_'
(e.g. KOMGA_CONFIGDIR or KOMGA_CONFIG_DIR):
  komga.config-dir                               default ~/.komga (--config-dir; no file)
  server.port                                    default 25600 (--port)
  server.address                                 default :: (fallback 0.0.0.0) (--host)
  komga.database.file                            default <config-dir>/database.sqlite
  komga.database.check-local-filesystem          default true
  komga.tasks-db.file                            default <config-dir>/tasks.sqlite
  komga.tasks-db.check-local-filesystem          default true
  komga.fonts.data-directory                     default <config-dir>/fonts
  komga.epub-divina-letter-count-threshold       default 15
  komga.page-hashing                             default 3
  komga.kobo.sync-item-limit                     default 100
  spring.session.timeout                         default 7d
  komga.lucene.data-directory                    default <config-dir>/lucene
  komga.lucene.index-analyzer.min-gram           default 3
  komga.lucene.index-analyzer.max-gram           default 10
  komga.lucene.index-analyzer.preserve-original  default true
  komga.lucene.commit-delay                      default 2s
  komga.sort.locale                              default und
  komga.sort.library-locales                     <library id>=<locale>,...
  komga.oauth2-account-creation                  default false
  komga.oidc-email-verification                  default true
  spring.security.oauth2.client.*                login providers

Examples:
  komga-rust
  komga-rust --list-users
  komga-rust --reset alice@example.org --newpassword new-secret

Note: use --option=value when a value starts with '-'."
)]
pub struct CliArgs {
    /// Port to listen on.
    #[arg(long)]
    pub port: Option<u16>,

    /// Host IP address to bind to.
    #[arg(long)]
    pub host: Option<String>,

    /// Directory holding configuration files, databases, and the instance lock.
    #[arg(long)]
    pub config_dir: Option<PathBuf>,

    /// Resolve configuration and exit without starting the server.
    #[arg(long)]
    pub dry_run: bool,

    /// Print persisted user emails and exit.
    #[arg(
        long,
        help_heading = "Admin options",
        conflicts_with_all = ["port", "host", "dry_run"]
    )]
    pub list_users: bool,

    /// Reset password for one or more user emails and exit.
    #[arg(
        long = "reset",
        value_name = "EMAIL",
        value_parser = parse_non_blank_email,
        help_heading = "Admin options",
        requires = "new_password",
        conflicts_with_all = ["port", "host", "dry_run"]
    )]
    pub reset_emails: Vec<String>,

    /// Password used with --reset.
    #[arg(
        long = "newpassword",
        value_name = "PASSWORD",
        value_parser = parse_non_blank_password,
        help_heading = "Admin options",
        requires = "reset_emails",
        conflicts_with_all = ["port", "host", "dry_run"]
    )]
    pub new_password: Option<String>,
}

/// One-shot administrative operations requested on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminCommand {
    pub list_users: bool,
    pub reset_password: Option<PasswordReset>,
}

/// Target user emails and replacement password for `--reset` / `--newpassword`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordReset {
    pub emails: Vec<String>,
    pub new_password: String,
}

impl CliArgs {
    /// Returns the requested one-shot admin command, or `None` for normal server startup.
    #[must_use]
    pub fn admin_command(&self) -> Option<AdminCommand> {
        let reset_password = self
            .new_password
            .as_ref()
            .filter(|_| !self.reset_emails.is_empty())
            .map(|new_password| PasswordReset {
                emails: self.reset_emails.clone(),
                new_password: new_password.clone(),
            });
        if self.list_users || reset_password.is_some() {
            Some(AdminCommand {
                list_users: self.list_users,
                reset_password,
            })
        } else {
            None
        }
    }
}

fn parse_non_blank_email(value: &str) -> Result<String, &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err("must not be blank")
    } else {
        Ok(trimmed.to_owned())
    }
}

fn parse_non_blank_password(value: &str) -> Result<String, &'static str> {
    if value.trim().is_empty() {
        Err("must not be blank")
    } else {
        Ok(value.to_owned())
    }
}
