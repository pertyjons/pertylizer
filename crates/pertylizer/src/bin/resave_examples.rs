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
    for path in list_dir(&root.join("projects"))? {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => {
                let mut proj = ProjectFile::load(&path)?;
                normalise_project(&mut proj);
                proj.save(&path)?;
                println!("resaved {}", path.display());
                touched += 1;
            }
            Some("zip") => {
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
    for path in list_dir(&root.join("patches"))? {
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let mut patch = Patch::load(&path)?;
            normalise_patch(&mut patch);
            patch.save(&path)?;
            println!("resaved {}", path.display());
            touched += 1;
        }
    }
    for path in list_dir(&root.join("awe"))? {
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let awe = AwePresetFile::load(&path)?;
            awe.save(&path)?;
            println!("resaved {}", path.display());
            touched += 1;
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
        let Some(value) = module.parameters.get_mut(&param_desc.type_id) else {
            continue;
        };
        if let ParamValue::Float(f) = value
            && let Some(choice) = param_desc.choice_for_value(*f)
        {
            *value = ParamValue::Choice(choice.id.clone());
        }
    }
}

/// Return the file entries directly inside `dir`, sorted. Skips files
/// silently if `dir` doesn't exist.
fn list_dir(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    Ok(out)
}
