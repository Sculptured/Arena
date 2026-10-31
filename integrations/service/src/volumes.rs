//! Moving an instance's volumes into saves and back.
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use arena_part::helpers::compose::Project;
use arena_part::helpers::folder;
use arena_part::helpers::temp_dir::TempDir;

/// A running instance: its project, the volumes a save holds, and its last save to link against.
#[derive(Debug, Clone)]
pub struct Instance {
    pub project: Project,
    pub volumes: Vec<String>,
    pub prev_save: Option<PathBuf>,
}

/// Copies every volume into `into/<volume>` (the folders are made).
pub fn export_all(inst: &Instance, into: &Path) -> Result<()> {
    for v in &inst.volumes {
        let to = into.join(v);
        std::fs::create_dir_all(&to)?;
        inst.project.export_volume(v, &to)?;
    }
    Ok(())
}

/// Writes every volume to `dir/<volume>`, linking files unchanged since the last save; returns
/// the new bytes. The project is stopped meanwhile and started again, also when copying fails.
pub fn save_all(inst: &Instance, dir: &Path, scratch: &Path) -> Result<u64> {
    inst.project.stop()?;
    let copied = copy_stopped(inst, dir, scratch);
    let started = inst.project.start();
    let bytes = copied?;
    started?;
    Ok(bytes)
}

fn copy_stopped(inst: &Instance, dir: &Path, scratch: &Path) -> Result<u64> {
    let tmp = TempDir::new_in(scratch, "export")?;
    export_all(inst, tmp.path())?;
    std::fs::create_dir_all(dir)?;
    folder::copy_save(tmp.path(), dir, inst.prev_save.as_deref()).map_err(Into::into)
}

/// Creates the project's volumes, fills them from `save/<volume>`, and starts the project.
pub fn restore_all(inst: &Instance, save: &Path) -> Result<()> {
    if let Some(v) = inst.volumes.iter().find(|v| !save.join(v).is_dir()) {
        bail!("the save has no volume {v}");
    }
    inst.project.create()?;
    for v in &inst.volumes {
        inst.project.import_volume(v, &save.join(v))?;
    }
    inst.project.start()
}
