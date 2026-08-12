//! Generator for the Sound Core V2 reference render corpus.
//!
//! Run with `cargo run -p pertylizer --bin gen_corpus`. It rewrites every
//! fixture project under `corpus/v2-reference/projects/` from the builders in
//! [`pertylizer::corpus::fixtures`], then refreshes each case's `sha256` in the
//! manifest so the recorded digests describe the bytes that are actually there.
//!
//! The manifest is the input as well as the output: this tool owns the digests
//! and nothing else. Titles, purposes, render settings, and preserve/change
//! claims are written by hand, because they are judgements rather than
//! derivations — and a generator that could rewrite them would be able to make
//! a corpus agree with itself while agreeing with nothing else.
//!
//! A case whose input is not one of the generated fixtures is left alone apart
//! from its digest, which is still refreshed: that is how a case pointing at a
//! checked-in project rather than at a builder stays pinned.

use std::path::PathBuf;

use pertylizer::corpus::{CORPUS_DIR, CorpusManifest, MANIFEST_FILE, fixtures};
use pertylizer::render::receipt::FileDigest;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let corpus_dir = match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => workspace_root()?.join(CORPUS_DIR),
    };
    let manifest_path = corpus_dir.join(MANIFEST_FILE);

    let mut manifest = CorpusManifest::load(&manifest_path)?;

    for path in fixtures::write_all(&corpus_dir)? {
        println!("wrote {}", path.display());
    }

    let mut refreshed = 0_usize;
    for case in &mut manifest.cases {
        let digest = FileDigest::of(&case.input_path(&corpus_dir))?;
        if digest.sha256 != case.sha256 {
            println!("{}: digest {} → {}", case.id, case.sha256, digest.sha256);
            case.sha256 = digest.sha256;
            refreshed += 1;
        }
    }

    // Validated again before writing: the load above validated the file as it
    // was, and this tool has since edited it. Writing an invalid manifest would
    // leave the corpus unloadable until someone hand-repaired it.
    manifest.validate()?;
    pertylizer::io::atomic::write(&manifest_path, &manifest.to_json()?)?;

    println!(
        "{} case(s) in {}, {refreshed} digest(s) refreshed",
        manifest.cases.len(),
        manifest_path.display()
    );
    Ok(())
}

/// The workspace root, derived from this crate's manifest directory.
fn workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "cannot locate the workspace root from CARGO_MANIFEST_DIR".into())
}
