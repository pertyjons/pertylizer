//! Re-save every example file under `assets/examples/` by loading,
//! normalising, and saving it back. Useful when the save format changes
//! silently (e.g. choice values flip from numeric indices to string ids)
//! and the checked-in examples should be normalised. Bundles, projects,
//! patches, and AWE presets are all handled.
//!
//! Normalisations applied during the load → save pass:
//! - Choice parameters: `ParamValue::Float(idx)` is upgraded to
//!   `ParamValue::Choice(id_string)` using each module's descriptor.
//!
//! Run with `cargo run -p pertylizer --bin resave_examples`.

use std::path::{Path, PathBuf};

use pertylizer::bundle::{load_bundle, save_bundle};
use pertylizer::module_factory::get_descriptor;
use pertylizer::patch::{AwePresetFile, ModuleState, ParamValue, Patch};
use pertylizer::project::ProjectFile;
use synth_sampler::SampleLibrary;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .ok_or("cannot locate workspace root")?
        .join("assets/examples");

    let mut touched = 0usize;
    for entry in walkdir(&root)? {
        let path = entry;
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        match ext {
            "json" => {
                if path.parent().is_some_and(|p| p.ends_with("projects")) {
                    let mut proj = ProjectFile::load(&path)?;
                    normalise_project(&mut proj);
                    proj.save(&path)?;
                } else if path.parent().is_some_and(|p| p.ends_with("patches")) {
                    let mut patch = Patch::load(&path)?;
                    normalise_patch(&mut patch);
                    patch.save(&path)?;
                } else if path.parent().is_some_and(|p| p.ends_with("awe")) {
                    let awe = AwePresetFile::load(&path)?;
                    awe.save(&path)?;
                } else {
                    continue;
                }
                println!("resaved {}", path.display());
                touched += 1;
            }
            "zip" => {
                let mut lib = SampleLibrary::default();
                let mut proj = load_bundle(&path, &mut lib)?;
                normalise_project(&mut proj);
                save_bundle(&proj, &lib, &path)?;
                println!("resaved bundle {}", path.display());
                touched += 1;
            }
            _ => {}
        }
    }
    println!("resaved {touched} file(s)");
    Ok(())
}

fn normalise_project(proj: &mut ProjectFile) {
    for inst in &mut proj.instruments {
        normalise_patch(&mut inst.patch);
    }
}

fn normalise_patch(patch: &mut Patch) {
    for module in &mut patch.modules {
        normalise_module(module);
    }
}

fn normalise_module(module: &mut ModuleState) {
    let Some(desc) = get_descriptor(module.module_type) else {
        return;
    };
    for param_desc in &desc.parameters {
        let Some(choices) = param_desc.choices.as_ref() else {
            continue;
        };
        let Some(value) = module.parameters.get_mut(&param_desc.type_id) else {
            continue;
        };
        if let ParamValue::Float(f) = value {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let idx = f.round().max(0.0) as usize;
            if let Some(choice) = choices.get(idx) {
                *value = ParamValue::Choice(choice.id.clone());
            }
        }
    }
}

fn walkdir(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
