//! The staged snapshot, as it will be committed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Budgets apply only to the workspace.
const SCOPE: &str = "komga-rust/";

/// The index of the repository containing a directory. Paths are repository-relative.
///
/// Git inherits `GIT_INDEX_FILE` from a hook, so `git commit <paths>` is checked against the
/// temporary index that is about to be committed.
pub struct Index {
    top: PathBuf,
}

impl Index {
    pub fn open(dir: &Path) -> Result<Self> {
        let top = git(dir, &["rev-parse", "--show-toplevel"])?;
        let top = String::from_utf8(top).context("repository path is not UTF-8")?;
        Ok(Self {
            top: PathBuf::from(top.trim_end()),
        })
    }

    /// Added, modified and renamed paths under [`SCOPE`]. Deleted paths are skipped and renames
    /// yield their target.
    pub fn changed_paths(&self) -> Result<Vec<String>> {
        let listing = self.git(&[
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "-M",
            "--diff-filter=AMR",
            "--",
            SCOPE,
        ])?;
        let listing = String::from_utf8(listing).context("staged path is not UTF-8")?;
        let mut fields = listing.split_terminator('\0');
        let mut paths: Vec<String> = Vec::new();
        while let Some(status) = fields.next() {
            // Renames list the source before the target.
            let path_count = if status.starts_with('R') { 2 } else { 1 };
            let Some(path) = fields.by_ref().take(path_count).last() else {
                bail!("truncated `git diff` output after status {status:?}");
            };
            paths.push(path.to_owned());
        }
        Ok(paths)
    }

    pub fn contains(&self, path: &str) -> Result<bool> {
        let pathspec = format!(":(literal){path}");
        let listed = self.git(&["ls-files", "--cached", "-z", "--", &pathspec])?;
        Ok(!listed.is_empty())
    }

    pub fn read(&self, path: &str) -> Result<String> {
        let blob = self.git(&["cat-file", "blob", &format!(":{path}")])?;
        String::from_utf8(blob).with_context(|| format!("{path} is not UTF-8"))
    }

    /// Every tracked or unignored path currently present in the working tree under [`SCOPE`].
    pub fn all_paths(&self) -> Result<Vec<String>> {
        let listing = self.git(&[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            SCOPE,
        ])?;
        let listing = String::from_utf8(listing).context("working tree path is not UTF-8")?;
        Ok(listing
            .split_terminator('\0')
            .filter(|path| self.top.join(path).is_file())
            .map(ToOwned::to_owned)
            .collect())
    }

    pub fn worktree_contains(&self, path: &str) -> bool {
        self.top.join(path).is_file()
    }

    pub fn read_worktree(&self, path: &str) -> Result<String> {
        fs::read_to_string(self.top.join(path)).with_context(|| format!("cannot read {path}"))
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        git(&self.top, args)
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn repo() -> TempDir {
        let dir = TempDir::new().expect("temporary directory");
        git(dir.path(), &["init", "--quiet"]).expect("git init");
        for (key, value) in [("user.name", "test"), ("user.email", "test@example.com")] {
            git(dir.path(), &["config", key, value]).expect("git config");
        }
        dir
    }

    fn write(repo: &TempDir, path: &str, content: &str) {
        let path = repo.path().join(path);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create dirs");
        fs::write(path, content).expect("write file");
    }

    fn commit(repo: &TempDir) {
        git(repo.path(), &["add", "--all"]).expect("git add");
        let args = [
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            "c",
        ];
        git(repo.path(), &args).expect("git commit");
    }

    #[test]
    fn reads_index_content_not_working_tree() {
        let repo = repo();
        write(&repo, "komga-rust/src/a.rs", "fn staged() {}\n");
        git(repo.path(), &["add", "."]).expect("git add");
        write(&repo, "komga-rust/src/a.rs", "fn unstaged() {}\n");

        let index = Index::open(repo.path()).expect("open index");
        assert_eq!(
            index.changed_paths().expect("changed"),
            ["komga-rust/src/a.rs"]
        );
        assert_eq!(
            index.read("komga-rust/src/a.rs").expect("blob"),
            "fn staged() {}\n"
        );
    }

    #[test]
    fn limits_scope_to_staged_komga_rust_paths() {
        let repo = repo();
        write(&repo, "komga-rust/src/unchanged.rs", "fn a() {}\n");
        commit(&repo);

        write(&repo, "komga-rust/src/unchanged.rs", "fn b() {}\n");
        write(&repo, "komga-rust/src/new.rs", "fn c() {}\n");
        write(&repo, "komga-rust/README.md", "doc\n");
        write(&repo, "komga/src/other.rs", "fn d() {}\n");
        git(
            repo.path(),
            &[
                "add",
                "komga-rust/src/new.rs",
                "komga-rust/README.md",
                "komga",
            ],
        )
        .expect("git add");

        // Opened from a subdirectory: the scope is anchored at the repository root.
        let index = Index::open(&repo.path().join("komga-rust/src")).expect("open index");
        assert_eq!(
            index.changed_paths().expect("changed"),
            ["komga-rust/README.md", "komga-rust/src/new.rs"]
        );
    }

    #[test]
    fn skips_deletions_and_reports_rename_targets() {
        let repo = repo();
        let body = "fn moved() {\n    let _ = 1;\n}\n";
        write(&repo, "komga-rust/src/deleted.rs", "fn gone() {}\n");
        write(&repo, "komga-rust/src/old.rs", body);
        write(
            &repo,
            "outside/entering.rs",
            "fn entering() {\n    let _ = 2;\n}\n",
        );
        write(
            &repo,
            "komga-rust/src/leaving.rs",
            "fn leaving() {\n    let _ = 3;\n}\n",
        );
        commit(&repo);

        fs::remove_file(repo.path().join("komga-rust/src/deleted.rs")).expect("delete");
        git(
            repo.path(),
            &["mv", "komga-rust/src/old.rs", "komga-rust/src/new.rs"],
        )
        .expect("mv");
        git(
            repo.path(),
            &["mv", "outside/entering.rs", "komga-rust/src/entering.rs"],
        )
        .expect("mv");
        git(
            repo.path(),
            &["mv", "komga-rust/src/leaving.rs", "outside/leaving.rs"],
        )
        .expect("mv");
        git(repo.path(), &["add", "--all"]).expect("git add");

        let index = Index::open(repo.path()).expect("open index");
        // The pathspec sees one side of a rename across the scope boundary: an addition inside,
        // a deletion outside.
        assert_eq!(
            index.changed_paths().expect("changed"),
            ["komga-rust/src/entering.rs", "komga-rust/src/new.rs"]
        );
        assert_eq!(index.read("komga-rust/src/new.rs").expect("blob"), body);
        assert!(
            !index
                .contains("komga-rust/src/deleted.rs")
                .expect("ls-files")
        );
        assert!(index.contains("komga-rust/src/new.rs").expect("ls-files"));
    }
}
