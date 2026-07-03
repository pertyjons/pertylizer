//! Custom panic hook for desktop crash diagnostics.
//!
//! A default Rust panic prints a stack trace to stderr and (for the audio /
//! background threads) is easy to miss. [`install`] replaces the hook with one
//! that, in addition to the usual stderr trace, writes a self-contained crash
//! report to a file under the platform data directory so a user can attach it to
//! a bug report after the app has already gone.
//!
//! Crash reports land in:
//! - Linux: `~/.local/share/pertylizer/crashes/`
//! - macOS: `~/Library/Application Support/pertylizer/crashes/`
//! - Windows: `%APPDATA%\pertylizer\crashes\`

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Install the custom panic hook. Idempotent enough to call once at startup;
/// it chains to (does not discard) the previously installed hook so the normal
/// stderr backtrace still appears.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A forced capture ignores `RUST_BACKTRACE`, so the crash file always
        // carries a trace even when the env var is unset.
        let backtrace = Backtrace::force_capture();
        let report = build_report(info, &backtrace);

        // Best-effort: surface it through tracing (stderr + activity console)
        // and persist it to disk. Never panic from inside the hook.
        tracing::error!(target: "panic", "{report}");
        if let Some(path) = write_report(&report) {
            eprintln!("Crash report written to {}", path.display());
        }

        // Chain to the default/previous hook for the standard stderr behaviour.
        previous(info);
    }));
}

/// Assemble a human-readable crash report from the panic payload, its location,
/// the panicking thread, and the captured backtrace.
fn build_report(info: &PanicHookInfo<'_>, backtrace: &Backtrace) -> String {
    let mut out = String::with_capacity(1024);
    let _ = writeln!(out, "Pertylizer crash report");
    let _ = writeln!(
        out,
        "version: {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE"),
    );

    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let _ = writeln!(out, "thread: {thread_name}");

    // The panic message: `&str` and `String` payloads are the common cases.
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    let _ = writeln!(out, "message: {message}");

    if let Some(location) = info.location() {
        let _ = writeln!(
            out,
            "location: {}:{}:{}",
            location.file(),
            location.line(),
            location.column(),
        );
    }

    let _ = writeln!(out, "\nbacktrace:\n{backtrace}");
    out
}

/// Write the report to a timestamped file in the crashes directory. Returns the
/// path on success; silently gives up on any I/O error (a crash handler must not
/// itself fail).
fn write_report(report: &str) -> Option<PathBuf> {
    write_report_to(&crashes_dir()?, report)
}

/// Directory-parameterised core of [`write_report`], so tests can target a temp
/// directory instead of the real user data dir.
fn write_report_to(dir: &std::path::Path, report: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;

    // Seconds-since-epoch + pid keeps names unique without pulling in a
    // date-formatting dependency.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("crash-{secs}-{}.log", std::process::id()));

    let mut file = std::fs::File::create(&path).ok()?;
    file.write_all(report.as_bytes()).ok()?;
    Some(path)
}

/// `<data_dir>/pertylizer/crashes`, mirroring the other app data directories.
fn crashes_dir() -> Option<PathBuf> {
    let base = dirs::data_dir().or_else(dirs::home_dir)?;
    Some(base.join("pertylizer").join("crashes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crashes_dir_ends_in_expected_components() {
        // In any normal test environment a data/home dir resolves.
        let dir = crashes_dir().expect("a data or home directory should resolve");
        assert!(dir.ends_with("pertylizer/crashes"));
    }

    #[test]
    fn write_report_creates_a_readable_file() {
        let dir =
            std::env::temp_dir().join(format!("pertylizer-panic-test-{}", std::process::id()));
        let report = "Pertylizer crash report\nmessage: boom-xyz\n";

        let path = write_report_to(&dir, report).expect("write should succeed to a temp dir");
        let written = std::fs::read_to_string(&path).expect("crash file should be readable");
        assert_eq!(written, report);
        assert!(path.starts_with(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
