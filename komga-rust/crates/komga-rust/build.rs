//! Records the Git branch and commit for `/actuator/info`, with `GIT_*` variables overriding
//! Git, and checks the web UIs: a release build builds a missing UI, other builds go without it.

use std::path::Path;
use std::process::Command;

fn main() {
    web_ui();
    println!("cargo:rerun-if-changed=build.rs");
    for name in [
        "GIT_BRANCH",
        "GIT_COMMIT_ID",
        "GIT_COMMIT_TIME",
        "VERSION_NEXT",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for file in ["HEAD", "packed-refs"] {
            println!("cargo:rerun-if-changed={dir}/{file}");
        }
        if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{reference}");
        }
    }
    let values = [
        ("GIT_BRANCH", &["branch", "--show-current"][..]),
        ("GIT_COMMIT_ID", &["rev-parse", "HEAD"][..]),
        (
            "GIT_COMMIT_TIME",
            &["show", "-s", "--format=%cI", "HEAD"][..],
        ),
    ];
    for (name, args) in values {
        let value = std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| git(args));
        if let Some(value) = value {
            println!("cargo:rustc-env={name}={value}");
        }
    }
}

/// Git's trimmed output, or nothing outside a repository or without Git.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !value.is_empty()).then_some(value)
}

/// Sets `web_ui_built` when both UIs are built.
fn web_ui() {
    println!("cargo::rustc-check-cfg=cfg(web_ui_built)");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo sets the manifest dir");
    let root = Path::new(&manifest_dir).join("../../..");
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let mut built = true;
    // next-ui first: its build records the commit, so it must see the unpatched working tree.
    for ui in ["next-ui", "komga-webui"] {
        let dist = root.join(ui).join("dist");
        if !dist.join("index.html").exists() {
            if !release {
                println!("cargo::warning={ui}/dist is missing; this build serves no {ui}");
                built = false;
                continue;
            }
            build_ui(&root, ui);
        }
        println!("cargo:rerun-if-changed={}", dist.display());
    }
    if built {
        println!("cargo::rustc-cfg=web_ui_built");
    }
}

/// The release workflow's UI build: the classic UI builds with fork-ts-checker disabled.
fn build_ui(root: &Path, ui: &str) {
    let patch = (ui == "komga-webui")
        .then(|| root.join("patches/komga-webui-disable-fork-ts-checker.patch"));
    if let Some(patch) = &patch {
        run(Command::new("git")
            .arg("apply")
            .arg(patch)
            .current_dir(root));
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let dir = root.join(ui);
    let built = [&["ci"][..], &["run", "build"][..]].iter().all(|args| {
        let mut command = Command::new(npm);
        command.args(*args).current_dir(&dir);
        if patch.is_some() {
            command.env("NODE_OPTIONS", "--max-old-space-size=8192");
        }
        succeeded(&mut command)
    });
    if let Some(patch) = &patch {
        run(Command::new("git")
            .args(["apply", "-R"])
            .arg(patch)
            .current_dir(root));
    }
    assert!(built, "Failed to build {ui}");
}

fn run(command: &mut Command) {
    assert!(succeeded(command), "Failed to run {command:?}");
}

/// Runs a command whose output goes to the build log, away from Cargo's instructions.
fn succeeded(command: &mut Command) -> bool {
    command
        .stdout(std::io::stderr())
        .status()
        .is_ok_and(|status| status.success())
}
