//! Maintainer-approved budget exceptions, each naming one file or function.

use anyhow::{Result, bail};
use serde::Deserialize;

/// Repository-relative location, read from the staged snapshot like the checked files.
pub const PATH: &str = "komga-rust/tools/code-budget/exceptions.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    /// Repository-relative Rust file.
    pub path: String,
    /// Function name as reported by the check; the file budget when absent.
    pub function: Option<String>,
    /// Replaces the hard limit.
    pub max_lines: usize,
    pub reason: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exceptions {
    #[serde(default, rename = "exception")]
    entries: Vec<Exception>,
}

impl Exceptions {
    pub fn parse(source: &str) -> Result<Self> {
        let exceptions: Self = toml::from_str(source)?;
        for (index, entry) in exceptions.entries.iter().enumerate() {
            let target = describe(&entry.path, entry.function.as_deref());
            // Directory and pattern exemptions are forbidden: an entry names one file.
            if !entry.path.ends_with(".rs") || entry.path.contains(['*', '?', '[']) {
                bail!("exception {target} must name a single Rust file");
            }
            if entry.reason.trim().is_empty() {
                bail!("exception {target} needs a reason");
            }
            let duplicate = exceptions.entries[..index]
                .iter()
                .any(|other| other.path == entry.path && other.function == entry.function);
            if duplicate {
                bail!("exception {target} is listed twice");
            }
        }
        Ok(exceptions)
    }

    pub fn find(&self, path: &str, function: Option<&str>) -> Option<&Exception> {
        self.entries
            .iter()
            .find(|entry| entry.path == path && entry.function.as_deref() == function)
    }
}

fn describe(path: &str, function: Option<&str>) -> String {
    match function {
        Some(function) => format!("{path} fn {function}"),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_file_and_function_entries() {
        let exceptions = Exceptions::parse(
            r#"
            [[exception]]
            path = "komga-rust/a.rs"
            max_lines = 650
            reason = "generated-like table reviewed in #1"

            [[exception]]
            path = "komga-rust/a.rs"
            function = "Foo::bar"
            max_lines = 120
            reason = "state machine kept in one place"
            "#,
        )
        .expect("valid exceptions");
        let file = exceptions
            .find("komga-rust/a.rs", None)
            .expect("file entry");
        assert_eq!(file.max_lines, 650);
        let function = exceptions
            .find("komga-rust/a.rs", Some("Foo::bar"))
            .expect("fn entry");
        assert_eq!(function.max_lines, 120);
        assert!(
            exceptions
                .find("komga-rust/a.rs", Some("Foo::baz"))
                .is_none()
        );
        assert!(exceptions.find("komga-rust/b.rs", None).is_none());
        assert!(
            Exceptions::parse("")
                .expect("empty file")
                .find("komga-rust/a.rs", None)
                .is_none()
        );
    }

    #[test]
    fn rejects_broad_or_unjustified_entries() {
        let cases = [
            (
                "path = \"komga-rust/crates/\"\nmax_lines = 1\nreason = \"r\"",
                "single Rust file",
            ),
            (
                "path = \"komga-rust/*.rs\"\nmax_lines = 1\nreason = \"r\"",
                "single Rust file",
            ),
            (
                "path = \"komga-rust/a.rs\"\nmax_lines = 1\nreason = \" \"",
                "needs a reason",
            ),
            (
                "path = \"komga-rust/a.rs\"\nmax_lines = 1",
                "missing field `reason`",
            ),
            (
                "path = \"komga-rust/a.rs\"\nmax_lines = 1\nreason = \"r\"\nscope = \"dir\"",
                "unknown field",
            ),
        ];
        for (entry, expected) in cases {
            let error = Exceptions::parse(&format!("[[exception]]\n{entry}\n"))
                .err()
                .unwrap_or_else(|| panic!("{entry:?} must be rejected"));
            assert!(error.to_string().contains(expected), "{entry:?}: {error}");
        }
        let entry = "[[exception]]\npath = \"komga-rust/a.rs\"\nmax_lines = 1\nreason = \"r\"\n";
        let error = Exceptions::parse(&entry.repeat(2))
            .err()
            .expect("duplicate rejected");
        assert!(error.to_string().contains("listed twice"), "{error}");
    }
}
