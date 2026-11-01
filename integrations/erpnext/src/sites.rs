//! The bench's `sites` folder, which the stack's containers mount: one folder per site, made
//! from a save's files and a generated `site_config.json`.
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use arena_part::helpers::{folder, remove_tree};
use serde_json::Value;

use crate::json_file;
use crate::site_config::{common_config, APPS_TXT};

/// The uid the ERPNext image runs as: the folder must belong to it.
const FRAPPE_UID: u32 = 1000;

/// A site's file folders and the folder each is kept in within a save's `files/`.
const FILE_DIRS: [(&str, &str); 2] = [("public/files", "public"), ("private/files", "private")];

/// The folders Frappe expects in a site besides its files.
const SITE_DIRS: [&str; 2] = ["logs", "locks"];

#[derive(Debug, Clone)]
pub struct SitesDir {
    root: PathBuf,
}

impl SitesDir {
    pub fn new(root: PathBuf) -> SitesDir {
        SitesDir { root }
    }

    fn site(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Makes the folder with the bench's config files. It must belong to the image's uid.
    pub fn prepare(&self) -> io::Result<()> {
        self.write_bench_files()?;
        let uid = std::fs::metadata(&self.root)?.uid();
        if uid != FRAPPE_UID {
            return Err(io::Error::other(format!(
                "{} belongs to uid {uid}, but ERPNext runs as uid {FRAPPE_UID}; run arena as \
                 uid {FRAPPE_UID}",
                self.root.display()
            )));
        }
        Ok(())
    }

    fn write_bench_files(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        json_file::write(&self.root.join("common_site_config.json"), &common_config())?;
        std::fs::write(self.root.join("apps.txt"), APPS_TXT)
    }

    /// A new site folder `name` (an old one is removed): the files kept in `files` (a save's
    /// `files/`) and `config` as its `site_config.json`, written last.
    pub fn create(&self, name: &str, files: &Path, config: &Value) -> io::Result<()> {
        let dir = self.site(name);
        self.remove(name)?;
        for (live, kept) in FILE_DIRS {
            let from = files.join(kept);
            if from.is_dir() {
                folder::restore(&from, &dir.join(live))?;
            } else {
                std::fs::create_dir_all(dir.join(live))?;
            }
        }
        for d in SITE_DIRS {
            std::fs::create_dir_all(dir.join(d))?;
        }
        json_file::write(&dir.join("site_config.json"), config).map(drop)
    }

    /// Copies the site's files into `dst` (a save's `files/`), linking files unchanged since
    /// `prev` (an earlier save's `files/`). Returns the bytes written.
    pub fn save_files(&self, name: &str, dst: &Path, prev: Option<&Path>) -> io::Result<u64> {
        let mut written = 0;
        for (live, kept) in FILE_DIRS {
            let from = self.site(name).join(live);
            std::fs::create_dir_all(&from)?;
            let prev = prev.map(|p| p.join(kept));
            written += folder::copy_save(&from, &dst.join(kept), prev.as_deref())?;
        }
        Ok(written)
    }

    /// The site's `site_config.json`.
    pub fn config(&self, name: &str) -> io::Result<Value> {
        json_file::read(&self.site(name).join("site_config.json"))
    }

    /// Removes the site's folder (nothing to do when there is none).
    pub fn remove(&self, name: &str) -> io::Result<()> {
        remove_tree(&self.site(name))
    }

    /// The names of the site folders there are.
    pub fn names(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return vec![];
        };
        rd.filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".localhost"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_part::helpers::temp_dir::TempDir;
    use serde_json::json;

    /// A sites folder and a save's `files/` with one public file.
    fn setup(tmp: &TempDir) -> (SitesDir, PathBuf) {
        let sites = SitesDir::new(tmp.path().join("sites"));
        sites.write_bench_files().unwrap();
        let saved = tmp.path().join("save/files");
        std::fs::create_dir_all(saved.join("public")).unwrap();
        std::fs::write(saved.join("public/logo.png"), "png").unwrap();
        let config = json!({"db_name": "eb_a"});
        sites.create("a.localhost", &saved, &config).unwrap();
        (sites, saved)
    }

    #[test]
    fn sites_are_made_from_saved_files() {
        let tmp = TempDir::new_in(&std::env::temp_dir(), "erpnext_sites").unwrap();
        let (sites, _) = setup(&tmp);
        let site = sites.root.join("a.localhost");
        let logo = std::fs::read(site.join("public/files/logo.png")).unwrap();
        assert_eq!(logo, b"png");
        assert!(site.join("private/files").is_dir() && site.join("logs").is_dir());
        assert_eq!(sites.config("a.localhost").unwrap()["db_name"], "eb_a");
        assert_eq!(sites.names(), ["a.localhost"]);
        let apps = std::fs::read_to_string(sites.root.join("apps.txt")).unwrap();
        assert_eq!(apps, APPS_TXT);
    }

    #[test]
    fn site_files_are_saved_back_and_removed() {
        let tmp = TempDir::new_in(&std::env::temp_dir(), "erpnext_sites").unwrap();
        let (sites, saved) = setup(&tmp);
        let site = sites.root.join("a.localhost");
        std::fs::write(site.join("private/files/new.txt"), "n").unwrap();
        let out = tmp.path().join("save2/files");
        let written = sites.save_files("a.localhost", &out, Some(&saved)).unwrap();
        assert_eq!(written, 1, "the unchanged logo is linked");
        assert!(out.join("public/logo.png").is_file() && out.join("private/new.txt").is_file());
        sites.remove("a.localhost").unwrap();
        sites.remove("a.localhost").unwrap();
        assert!(sites.names().is_empty());
    }
}
