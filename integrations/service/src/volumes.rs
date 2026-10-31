//! Moving an instance's volumes into saves and back, as tar archives.
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use arena_part::helpers::compose::Project;

/// A running instance: its project and the volumes a save holds.
#[derive(Debug, Clone)]
pub struct Instance {
    pub project: Project,
    pub volumes: Vec<String>,
}

/// Where the archive of `volume` is in the folder `dir`.
pub fn tar_path(dir: &Path, volume: &str) -> PathBuf {
    dir.join(format!("{volume}.tar"))
}

/// Writes every volume to `into/<volume>.tar` (`into` must exist); returns the bytes written.
pub fn export_all(inst: &Instance, into: &Path) -> Result<u64> {
    let mut bytes = 0;
    for v in &inst.volumes {
        let tar = tar_path(into, v);
        inst.project.export_volume(v, &tar)?;
        bytes += std::fs::metadata(&tar)?.len();
    }
    Ok(bytes)
}

/// Writes every volume to `dir`, exactly as it is; returns the bytes written. The project is
/// stopped meanwhile and started again, also when copying fails.
pub fn save_all(inst: &Instance, dir: &Path) -> Result<u64> {
    inst.project.stop()?;
    let copied = std::fs::create_dir_all(dir)
        .map_err(Into::into)
        .and_then(|()| export_all(inst, dir));
    let started = inst.project.start();
    let bytes = copied?;
    started?;
    Ok(bytes)
}

/// Creates the project's volumes, fills them from the archives in `save`, and starts the project.
pub fn restore_all(inst: &Instance, save: &Path) -> Result<()> {
    if let Some(v) = inst.volumes.iter().find(|v| !tar_path(save, v).is_file()) {
        bail!("the save has no volume {v}");
    }
    inst.project.create()?;
    for v in &inst.volumes {
        inst.project.import_volume(v, &tar_path(save, v))?;
    }
    inst.project.start()
}
