use std::process::Command;

fn main() {
    // Embed build date/time as UTC (pure Rust, no shell dependency)
    let (date, time) = {
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        // Convert Unix timestamp to date using civil day calculation
        let days = (secs / 86400) as i64;
        let (year, month, day) = civil_from_days(days);
        let tod = secs % 86400;
        let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
        (
            format!("{year}-{month:02}-{day:02}"),
            format!("{h:02}:{m:02}:{s:02}"),
        )
    };
    println!("cargo:rustc-env=BUILD_DATE={date}");
    println!("cargo:rustc-env=BUILD_TIMESTAMP={date} {time} UTC");

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

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's `chrono`-compatible civil calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
