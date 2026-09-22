//! Build script for `packdb-core`.
//!
//! `[lints.rust] missing_docs` in Cargo.toml applies to every target of the
//! package, build scripts included, so this crate doc is required.

use std::process::Command;

fn main() {
    // An explicitly supplied GIT_COMMIT wins over `git rev-parse`.
    //
    // Container builds have no `.git`: the image build context copies Cargo
    // manifests and crate sources, not the repository. `git rev-parse` therefore
    // fails there and the revision falls through to "unknown" — which is what
    // made every published image report `buildinfo.revision = unknown` (#604),
    // and what stops a shipped diagnostics report from naming the exact SHA it
    // was built from. Reading the environment first lets a build
    // system state the revision when it cannot be derived.
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    let env_commit = std::env::var("GIT_COMMIT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(git_commit) = env_commit {
        // Reject anything that is not a full hexadecimal commit SHA. A silently
        // accepted placeholder ("unknown", a tag, a truncated prefix) would
        // produce a report that names a revision nobody can resolve back to a
        // tree — worse than the honest "unknown" this replaces, because it
        // looks authoritative. Fail the build instead.
        let looks_like_sha = matches!(git_commit.len(), 40 | 64)
            && git_commit.chars().all(|c| c.is_ascii_hexdigit());
        assert!(
            looks_like_sha,
            "GIT_COMMIT must be a full hexadecimal commit SHA (40 or 64 hex \
             characters); got {git_commit:?}. Pass the real revision or leave \
             it unset to fall back to `git rev-parse HEAD`."
        );
        println!("cargo:rustc-env=GIT_COMMIT={git_commit}");
    } else if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        let git_commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !git_commit.is_empty() {
            println!("cargo:rustc-env=GIT_COMMIT={git_commit}");
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
    {
        let git_branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !git_branch.is_empty() {
            println!("cargo:rustc-env=GIT_BRANCH={git_branch}");
        }
    }

    // Capture build date
    if let Ok(date) = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
    {
        let build_date = String::from_utf8_lossy(&date.stdout).trim().to_string();
        println!("cargo:rustc-env=BUILD_DATE={build_date}");
    } else {
        println!("cargo:rustc-env=BUILD_DATE=unknown");
    }

    // Capture build user
    if let Ok(user) = std::env::var("USER") {
        println!("cargo:rustc-env=BUILD_USER={user}");
    } else if let Ok(user) = std::env::var("USERNAME") {
        println!("cargo:rustc-env=BUILD_USER={user}");
    } else {
        println!("cargo:rustc-env=BUILD_USER=unknown");
    }

    // Capture Rust compiler version
    if let Ok(output) = Command::new("rustc").args(["--version"]).output() {
        let rustc_version = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap_or("unknown")
            .to_string();
        println!("cargo:rustc-env=RUSTC_VERSION={rustc_version}");
    } else {
        println!("cargo:rustc-env=RUSTC_VERSION=unknown");
    }

    // Watch the real HEAD and refs so a commit invalidates the captured
    // revision. `../../.git/HEAD` is wrong in a linked worktree — `.git` there
    // is a file pointing at `<main>/.git/worktrees/<name>`, so cargo watches a
    // path that does not exist and an incremental rebuild after a commit keeps
    // the previous revision. `git rev-parse --git-path` resolves correctly in
    // worktrees and plain checkouts alike.
    for git_path in ["HEAD", "refs"] {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--git-path", git_path])
            .output()
            && output.status.success()
        {
            let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !resolved.is_empty() {
                println!("cargo:rerun-if-changed={resolved}");
            }
        }
    }
}
