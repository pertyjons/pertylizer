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

/// The **exact** line the Phase 4 lowering exception is permitted to be.
///
/// Pinned for the same reason as the declaration above, and separately from it
/// because the two are different exceptions with different strengths: the
/// dev-dependency reaches only harnesses that do not ship, while this one is a
/// normal edge that a feature can switch on. ADR-0056 selects it.
const PERMITTED_OPTIONAL_DECLARATION: &str =
    r#"synth_engine_v2 = { path = "../synth_engine_v2", optional = true }"#;

/// The one feature that may enable the optional edge.
///
/// Named rather than described, so that a second feature reaching the crate
/// changes this constant instead of passing quietly. ADR-0056's revisit
/// condition is exactly that: two would need a rule rather than a name.
const LOWERING_FEATURE: &str = "pertylizer/v2-lowering";

/// The same feature, unqualified, as the consumer's own `[features]` table spells it.
const LOWERING_FEATURE_NAME: &str = "v2-lowering";

/// The module tree the lowering feature gates.
///
/// Files under this prefix may name the experimental crate; every other file in
/// the consumer may not. A prefix rather than a file list because the lowerer is
/// a growing module tree, and a list would have to be edited for every file it
/// gains — which is the kind of edit that gets made without thought.
const LOWERING_MODULE_PREFIX: &str = "src/lowering/";

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
    dependents_with_features(edges, &[])
}

/// The same question, asked with extra features switched on.
///
/// Separate from [`dependents`] because the two carry different claims.
/// `dependents` asks what a **default** build links, which is the sentence that
/// holds the Phase 1 exit gate's deletability claim. This one asks what a build
/// that opts in links, which is what stops ADR-0056's exception from being a
/// hole: without it the default answer would stay empty while any number of
/// crates linked the experimental crate behind features, and nothing would say so.
fn dependents_with_features(edges: &str, features: &[&str]) -> Vec<String> {
    let flags: Vec<String> = features
        .iter()
        .flat_map(|f| ["--features".to_owned(), (*f).to_owned()])
        .collect();
    let borrowed: Vec<&str> = flags.iter().map(String::as_str).collect();
    dependents_with_flags(edges, &borrowed)
}

/// The same question again, with arbitrary resolution flags appended.
///
/// Split out because `--all-features` is a flag rather than a feature name, and
/// the strongest form of the question — does *any* feature add an edge — can
/// only be asked with it.
fn dependents_with_flags(edges: &str, flags: &[&str]) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = std::process::Command::new(cargo);
    let output = command
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
        // After the subcommand, not before it: `cargo --features X tree` is not a
        // valid invocation and fails before Cargo resolves anything, which would
        // make this check pass on an error rather than on an empty answer.
        .args(flags.iter().copied())
        .current_dir(repo_root())
        .output()
        .expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree --edges {edges} {flags:?} failed: {}",
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

    // ADR-0056's fifth consequence. The optional edge is pinned separately from
    // the dev edge because it is a different exception: this one is a normal
    // dependency that a feature switches on, and reshaping it quietly would
    // reshape what ships.
    let declared_optional = manifest
        .lines()
        .filter(|line| line.trim() == PERMITTED_OPTIONAL_DECLARATION)
        .count();
    assert_eq!(
        declared_optional, 1,
        "the lowering dependency must be declared exactly once, as \
         `{PERMITTED_OPTIONAL_DECLARATION}`, so the source scan below can rely on the name"
    );

    // The module tree may name the crate only because a feature gates it. If the
    // gate is removed the tree becomes ordinary library code, the scan below keeps
    // permitting it, and the crate reaches a default build with nothing objecting.
    let lib = read(&consumer.join("src").join("lib.rs"));
    // `test` rides beside the feature so that `cargo test --workspace` — which resolves
    // default features and would otherwise never compile the module — actually runs the
    // lowering tests. It adds no shipping reach: a `cfg(test)` build is not a build that
    // ships, and the crate it reaches there is the dev-dependency this file already permits.
    // An independent review found the tests ungated without it.
    assert!(
        lib.contains("#[cfg(any(feature = \"v2-lowering\", test))]\npub mod lowering;"),
        "the lowering module must be gated on the v2-lowering feature or a test build; \
         without the gate the permitted module tree below would reach a default build"
    );

    let mut offenders = Vec::new();
    let mut harnesses_found = 0_usize;
    let mut lowering_found = 0_usize;
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
        } else if relative.starts_with(LOWERING_MODULE_PREFIX) {
            if uses {
                lowering_found += 1;
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
    assert!(
        lowering_found > 0,
        "the lowering dependency is declared but no file under `{LOWERING_MODULE_PREFIX}` uses \
         it, so the permitted prefix would pass vacuously"
    );
}

/// Enabling the lowering feature adds exactly one dependent, and no other.
///
/// This is ADR-0056's second consequence, and the control on its exception. The
/// default-features check above is what carries the Phase 1 exit gate's
/// deletability claim, and on its own it is now weaker than it reads: an optional
/// dependency that no default feature enables is invisible to it, so any number
/// of crates could link the experimental crate behind features while it still
/// reported an empty answer. Asking Cargo again with the one permitted feature
/// switched on is what sees them.
///
/// It asserts the whole list rather than membership, so a second consumer appears
/// as a failure rather than as an unremarked extra line.
#[test]
fn enabling_the_lowering_feature_adds_exactly_one_dependent() {
    let shipping = dependents_with_features("normal", &[LOWERING_FEATURE]);
    assert_eq!(
        shipping,
        vec![MEASUREMENT_CONSUMER.to_owned()],
        "`{LOWERING_FEATURE}` may add exactly one normal dependent, and only \
         `{MEASUREMENT_CONSUMER}`"
    );

    // A build edge is as fatal to deletability as a normal one, and the feature
    // must not have created one.
    let building = dependents_with_features("build", &[LOWERING_FEATURE]);
    assert!(
        building.is_empty(),
        "`{LOWERING_FEATURE}` created a build-dependency edge: {building:?}"
    );
}

/// **No** feature of **any** workspace crate adds a second dependent.
///
/// The check above names one feature, and an independent review found that
/// naming is exactly its weakness: a second optional edge — another crate's own
/// non-default feature, a second feature inside the consumer, or a renamed
/// dependency — is invisible to an invocation that enables only
/// `v2-lowering`, so that check would keep reporting one dependent while two
/// existed. `--all-features` enables every feature of every workspace member at
/// once, so an edge that any feature combination can switch on is switched on
/// here.
///
/// This is the assertion that actually carries ADR-0056's second consequence;
/// the named-feature check above carries the narrower claim that the *permitted*
/// feature does what the record says it does.
#[test]
fn no_feature_of_any_crate_adds_a_second_dependent() {
    let shipping = dependents_with_flags("normal", &["--all-features"]);
    assert_eq!(
        shipping,
        vec![MEASUREMENT_CONSUMER.to_owned()],
        "with every workspace feature enabled, exactly one crate may reach the experimental \
         crate through a normal edge, and only `{MEASUREMENT_CONSUMER}`"
    );

    let building = dependents_with_flags("build", &["--all-features"]);
    assert!(
        building.is_empty(),
        "some feature created a build-dependency edge: {building:?}"
    );
}

/// Only `v2-lowering` activates the optional dependency, and nothing forwards to it.
///
/// The two tree checks above cannot see this. `--all-features` enables every feature at once,
/// so a second feature that also activates the dependency produces the *same* single
/// dependent and every assertion stays green — an independent review found exactly that.
/// What distinguishes one activating feature from two is not the resolved graph but the
/// feature table, so this reads the table.
///
/// The scan is for the crate's **name**, not for one activation syntax. An earlier form
/// matched `dep:synth_engine_v2` alone, and an independent review pointed out that Cargo has
/// more than one way to switch an optional dependency on: `foo = ["synth_engine_v2/some-feature"]`
/// activates it through the strong dependency-feature syntax and contains no `dep:` at all.
/// Matching the bare name covers every spelling that can name the crate, and consequence 7's
/// literal pinning of both declarations is what rules out a renamed alias naming it under some
/// other word.
///
/// A **forwarded** activation is a feature list containing `v2-lowering`, which would let an
/// innocuous-looking feature switch the edge on at one remove. Neither is permitted, so
/// `v2-lowering` stays the one name to look for.
#[test]
fn only_the_lowering_feature_activates_the_optional_dependency() {
    let manifest = read(
        &repo_root()
            .join("crates")
            .join(MEASUREMENT_CONSUMER)
            .join("Cargo.toml"),
    );

    // The `[features]` table, up to whichever table follows it.
    let table = manifest
        .split_once("\n[features]\n")
        .map(|(_, rest)| rest.split("\n[").next().unwrap_or(rest))
        .unwrap_or_default();
    assert!(
        !table.is_empty(),
        "the consumer must declare a [features] table for this check to mean anything"
    );

    let feature_name = |line: &str| -> Option<String> {
        let (name, _) = line.split_once('=')?;
        let name = name.trim();
        (!name.is_empty() && !name.starts_with('#')).then(|| name.to_owned())
    };

    // A feature's value list may span lines, so track which feature each line belongs to.
    let mut current: Option<String> = None;
    let mut direct = Vec::new();
    let mut forwarding = Vec::new();
    for line in table.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = feature_name(trimmed) {
            current = Some(name);
        }
        let Some(name) = current.clone() else {
            continue;
        };
        if trimmed.contains("synth_engine_v2") {
            direct.push(name.clone());
        }
        // The declaration line of `v2-lowering` itself is not a forward to it.
        if trimmed.contains(LOWERING_FEATURE_NAME) && name != LOWERING_FEATURE_NAME {
            forwarding.push(name);
        }
    }
    direct.dedup();
    forwarding.dedup();

    assert_eq!(
        direct,
        vec![LOWERING_FEATURE_NAME.to_owned()],
        "exactly one feature may activate the optional dependency, and only \
         `{LOWERING_FEATURE_NAME}`"
    );
    assert!(
        forwarding.is_empty(),
        "these features forward to `{LOWERING_FEATURE_NAME}`, which activates the optional \
         dependency at one remove: {forwarding:?}"
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
