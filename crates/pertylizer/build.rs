use std::process::Command;

fn main() {
    // Embed the build date/time as UTC. `BUILD_DATE` is the ISO date for the
    // window title; `BUILD_TIMESTAMP` is a full ISO 8601 / RFC 3339 instant
    // (e.g. `2026-07-03T14:30:00Z`) — chrono's `to_rfc3339_opts` gives the
    // standard form directly, no manual field assembly.
    let now = chrono::Utc::now();
    let date = now.format("%Y-%m-%d");
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    println!("cargo:rustc-env=BUILD_DATE={date}");
    println!("cargo:rustc-env=BUILD_TIMESTAMP={timestamp}");

    embed_git_info();

    // Emitting rerun-if-changed disables cargo's default "any package file"
    // rerun, so re-add the package sources alongside the git triggers.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src");
}

/// Embed commit hash, branch, and working-tree dirtiness at build time.
/// All values fall back to empty strings when git (or the repo) is absent,
/// e.g. when building from a source tarball.
fn embed_git_info() {
    let hash = git(&["rev-parse", "HEAD"]);
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    // "1" dirty, "0" clean, "" unknown (matches the hash/branch fallback)
    let dirty = match git_output(&["status", "--porcelain"]) {
        Some(out) => {
            if out.trim().is_empty() {
                "0"
            } else {
                "1"
            }
        }
        None => "",
    };
    println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
    println!("cargo:rustc-env=GIT_BRANCH={branch}");
    println!("cargo:rustc-env=GIT_DIRTY={dirty}");

    // Rerun on commit / branch switch / staging so the embedded info tracks HEAD.
    if let Some(git_dir) = git_output(&["rev-parse", "--absolute-git-dir"]) {
        let git_dir = git_dir.trim();
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/index");
    }
}

fn git(args: &[&str]) -> String {
    git_output(args)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
