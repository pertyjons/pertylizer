//! The crate can be deleted, and it reaches nothing it may not.
//!
//! The Phase 1 exit gate's last bullet is "the crate can be deleted without affecting V1
//! behavior or public APIs". That is a claim about coupling, and coupling is what rots
//! first, so it is checked from the manifests rather than asserted in prose. Two halves:
//! nothing depends on this crate, and this crate depends on nothing the work list
//! forbids. An earlier form of this check tested only the first, which would have passed
//! while the crate imported the GUI.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The one crate allowed to name the experimental crate, and only as a
/// dev-dependency.
///
/// EVD-0014 compares V1 against V2, and EVD-0016 embeds the V2 quantum in its
/// release-platform artifact. Neither can do that without linking the
/// experimental crate into a non-shipping target. Cargo forbids an optional
/// dev-dependency, so the exception has to be named here instead.
const MEASUREMENT_CONSUMER: &str = "pertylizer";

/// The **exact** line the measurement exception is permitted to be.
///
/// Pinning the literal rather than classifying tables is deliberate. TOML has
/// many valid spellings of the same dependency — a quoted key, a sub-table, a
/// target-specific table, a renamed package, single or double quotes — and an
/// earlier form of this check tried to recognise them all and kept missing one.
/// A scan for a *grammar* fails open; a scan for a *literal* fails closed. Any
/// other spelling changes either the occurrence count or this line, and either
/// one fails the test.
///
/// Changing the dependency's form therefore means changing this constant, which
/// is the point: a named exception to a phase gate should not be quietly
/// reshaped.
const PERMITTED_DECLARATION: &str = r#"synth_engine_v2 = { path = "../synth_engine_v2" }"#;

/// The only files permitted to reach the experimental crate from that consumer.
///
/// This is what keeps the exception from widening into a real coupling: the
/// dependency may exist, but only these three targets may use it, and none
/// ships.
const MEASUREMENT_HARNESSES: [&str; 3] = [
    "examples/evd_0013_equivalence.rs",
    "examples/evd_0014_cost.rs",
    "examples/evd_0016_cpal_timestamps.rs",
];

/// Nothing that ships depends on the experimental crate, and the one thing that
/// may is a measurement harness.
///
/// # Why this is narrower than "nothing names it at all"
///
/// The Phase 1 exit gate's bullet is "the crate can be deleted without affecting
/// V1 behavior or public APIs", and the earlier form of this check enforced it
/// by forbidding the *name* anywhere in any manifest. That is stricter than the
/// bullet: a dev-dependency reached only by an example affects neither V1's
/// behaviour nor any public API, and deleting the crate would delete the
/// harnesses with it — which is correct, since they exist to measure it.
///
/// So the check keeps the original manifest-wide scan, which has no false
/// negatives, and carves out exactly one occurrence in exactly one crate. The
/// companion test then requires that occurrence to be used only by the
/// harnesses, which is what makes the exception falsifiable rather than a hole.
/// Ask Cargo, rather than the manifest text, who depends on this crate.
///
/// `--edges` selects which dependency kinds count and `--invert` lists the
/// crates that reach the named one. The output's first line is the crate
/// itself; every later line is a dependent.
fn dependents(edges: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = std::process::Command::new(cargo)
        .args([
            "tree",
            "--workspace",
            "--edges",
            edges,
            "--invert",
            "synth_engine_v2",
            // Every target, not just this machine's. Cargo resolves for the host
            // by default, so a `[target.'cfg(windows)'.dependencies]` entry is
            // invisible on Linux — a shipping dependency that would prevent
            // deletion, reported as absent. This was found by probing, not by
            // reading: the earlier form of this call passed while a Windows-only
            // dependency on the crate sat in the manifest.
            "--target",
            "all",
        ])
        .current_dir(repo_root())
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree --edges {edges} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .map(str::trim)
        // The kind headers `cargo tree` prints between groups.
        .filter(|line| !line.is_empty() && !line.starts_with('['))
        .map(|line| {
            line.trim_start_matches(['└', '├', '─', '│', ' '])
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Nothing that ships depends on the experimental crate.
///
/// # Why this asks Cargo instead of reading the manifests
///
/// The Phase 1 exit gate's bullet is "the crate can be deleted without affecting
/// V1 behavior or public APIs". An earlier form of this check enforced it by
/// forbidding the crate's *name* anywhere in any `crates/*/Cargo.toml`, and a
/// later one classified dependency tables — and both were scans for a spelling.
/// TOML has many valid spellings of one dependency: a quoted key, a sub-table, a
/// target-specific table, a renamed package, a `[workspace.dependencies]` alias
/// inherited by a member, and string escapes inside any of them. A scan for a
/// grammar fails **open**, one spelling at a time.
///
/// `cargo tree --edges normal --invert` answers the question Cargo itself
/// resolves: which crates reach this one through edges that build and ship.
/// Aliases, inheritance and escapes are already resolved by the time it
/// answers, so there is no spelling left to miss.
#[test]
fn nothing_that_ships_depends_on_the_experimental_crate() {
    let shipping = dependents("normal");
    assert!(
        shipping.is_empty(),
        "these crates reach the experimental crate through a shipping edge, so it can no \
         longer be deleted: {shipping:?}"
    );
    // `build` is a separate edge kind and is equally fatal to deletability.
    let building = dependents("build");
    assert!(
        building.is_empty(),
        "these crates reach the experimental crate through a build-dependency edge: \
         {building:?}"
    );
}

/// Exactly one crate may reach it at all, and only for measurement.
///
/// The dev edge is where the measurement exceptions live: EVD-0014 requires
/// both engines in one binary, while EVD-0016 reads V2's quantum. Cargo forbids
/// an optional dev-dependency, so a feature gate is not available. Resolved by
/// Cargo for the same reason as above.
#[test]
fn only_the_measurement_consumer_reaches_it_at_all() {
    let all = dependents("normal,build,dev");
    assert_eq!(
        all,
        vec![MEASUREMENT_CONSUMER.to_owned()],
        "exactly one crate may reach the experimental crate, and only through a \
         dev-dependency for the measurement harnesses"
    );
}

/// Only the measurement harnesses **mention** the experimental crate.
///
/// Without this, the exception would be a hole rather than an exception: the
/// manifest would permit a dependency that any file in the crate could then
/// quietly use.
///
/// # What it establishes, and what it does not
///
/// It is a scan for a literal name, and it claims no more than that. It shows
/// which files spell `synth_engine_v2`, which — given the pinned declaration
/// above, so there is no alias to import under — is the only way a file can
/// reach it directly. It does **not** establish reachability: a harness could
/// re-export what it imports, and another module could then use it without
/// naming the crate. Nothing does that today, and closing it properly would
/// mean resolving the item graph rather than the text.
#[test]
fn only_the_measurement_harnesses_reach_the_experimental_crate() {
    let consumer = repo_root().join("crates").join(MEASUREMENT_CONSUMER);
    let manifest = read(&consumer.join("Cargo.toml"));
    if !manifest.contains("synth_engine_v2") {
        // No exception is in force, so there is nothing to constrain.
        return;
    }

    // The declaration is pinned to the crate's own name, so a source file that
    // reaches it has to write that name. Without this, an alias would let a file
    // import the crate as `whatever::...` and the scan below would not see it.
    //
    // This does **not** prove the manifest names the crate nowhere else — Cargo
    // answers that, in the previous test. It establishes one narrower thing: the
    // spelling a source file would have to use.
    let declared = manifest
        .lines()
        .filter(|line| line.trim() == PERMITTED_DECLARATION)
        .count();
    assert_eq!(
        declared, 1,
        "the measurement dev-dependency must be declared exactly once, as \
         `{PERMITTED_DECLARATION}`, so the source scan below can rely on the name"
    );

    let mut offenders = Vec::new();
    let mut harnesses_found = 0_usize;
    for path in rust_sources(&consumer) {
        let relative = path
            .strip_prefix(&consumer)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let uses = read(&path).contains("synth_engine_v2");
        if MEASUREMENT_HARNESSES.contains(&relative.as_str()) {
            if uses {
                harnesses_found += 1;
            }
        } else if uses {
            offenders.push(relative);
        }
    }

    assert!(
        offenders.is_empty(),
        "the experimental crate is a measurement-only dev-dependency, but these files reach \
         it: {offenders:?}"
    );
    assert_eq!(
        harnesses_found,
        MEASUREMENT_HARNESSES.len(),
        "the dev-dependency is declared but the named harnesses do not use it, so this check \
         would pass vacuously"
    );
}

/// Every `.rs` file under a crate directory.
fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    out
}

/// The dependency names in a manifest's `[dependencies]` table.
///
/// Only that table: `edition.workspace = true` is a package key, not a
/// dependency, and a scan over the whole file cannot tell them apart. Distinct
/// from [`names_the_crate`], which asks whether one specific crate is reached
/// by any table of a kind; this one enumerates a single table's entries so the
/// allowlist below can be checked against them.
fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[dependencies]";
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            names.push(key.trim().to_owned());
        }
    }
    names
}

#[test]
fn the_experimental_crate_reaches_nothing_the_work_list_forbids() {
    let manifest = read(&repo_root().join("crates/synth_engine_v2/Cargo.toml"));
    let dependencies = dependency_names(&manifest);
    assert!(
        !dependencies.is_empty(),
        "no `[dependencies]` table was found, so this check would pass vacuously"
    );

    // The Phase 1 work list fixes the surface: `synth_core` for existing domain
    // newtypes, selected DSP kernels or module implementations, and no GUI, MCP, OSC,
    // CPAL, filesystem, or project-loading dependency. `thiserror` is the repository's
    // mandated error-derive crate.
    let allowed = ["synth_core", "synth_dsp", "synth_modules", "thiserror"];
    for name in &dependencies {
        assert!(
            allowed.contains(&name.as_str()),
            "`{name}` is not on this crate's dependency allowlist; widening the surface is a \
             deliberate change to the Phase 1 work list, not a manifest edit"
        );
    }

    // Named explicitly as well as excluded by the allowlist, so the failure message says
    // *why* rather than only that the name is unfamiliar.
    for forbidden in [
        "egui",
        "eframe",
        "cpal",
        "midir",
        "rmcp",
        "rosc",
        "tokio",
        "axum",
        "hound",
        "zip",
        "dirs",
        "synth_engine",
        "synth_mcp",
        "synth_osc",
        "synth_sequencer",
        "pertylizer",
    ] {
        assert!(
            !dependencies.iter().any(|name| name == forbidden),
            "the experimental crate must not depend on {forbidden}: the renderer cannot query a \
             device, a protocol, a project, or a file"
        );
    }
}

#[test]
fn the_workspace_lists_the_crate_as_a_member() {
    // Otherwise none of these tests run, and a green build would mean nothing.
    let workspace = read(&repo_root().join("Cargo.toml"));
    assert!(
        workspace.contains("\"crates/synth_engine_v2\""),
        "the workspace must list the crate, or `cargo test --workspace` skips it entirely"
    );
}
