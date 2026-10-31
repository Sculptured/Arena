//! Lazily started part programs, one per package, shared by every branch.
use crate::client::Client;
use crate::manifest::Registry;
use anyhow::{anyhow, Result};
use arena_part::proto::{BenchDescribe, Describe};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

struct Slot {
    client: Arc<Client>,
    describe: Option<Describe>,
    bench: Option<BenchDescribe>,
}

pub struct Hub {
    registry: Registry,
    bin_dir: PathBuf,
    saves_root: PathBuf,
    slots: Mutex<HashMap<String, Slot>>,
}

impl Hub {
    pub fn new(registry: Registry, bin_dir: PathBuf, saves_root: PathBuf) -> Hub {
        Hub {
            registry,
            bin_dir,
            saves_root,
            slots: Mutex::default(),
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The program of a package: started on first use, started again when it has stopped.
    pub async fn client(&self, name: &str) -> Result<Arc<Client>> {
        let mut slots = self.slots.lock().await;
        if let Some(s) = slots.get(name) {
            if !s.client.is_stopped() {
                return Ok(s.client.clone());
            }
        }
        let m = self
            .registry
            .get(name)
            .ok_or_else(|| anyhow!("no package named {name}"))?;
        let client = Arc::new(Client::spawn(m, &self.bin_dir, &self.saves_root).await?);
        slots.insert(
            name.to_string(),
            Slot {
                client: client.clone(),
                describe: None,
                bench: None,
            },
        );
        Ok(client)
    }

    /// `describe` of an integration, cached until the program is respawned.
    pub async fn describe(&self, name: &str) -> Result<Describe> {
        let client = self.client(name).await?;
        if let Some(d) = self.cached(name, &client, |s| s.describe.clone()).await {
            return Ok(d);
        }
        let d: Describe = client.call("describe", json!({})).await?;
        self.store(name, &client, |s| s.describe = Some(d.clone()))
            .await;
        Ok(d)
    }

    /// `describe` of a benchmark, cached until the program is respawned.
    pub async fn bench_describe(&self, name: &str) -> Result<BenchDescribe> {
        let client = self.client(name).await?;
        if let Some(d) = self.cached(name, &client, |s| s.bench.clone()).await {
            return Ok(d);
        }
        let d: BenchDescribe = client.call("describe", json!({})).await?;
        self.store(name, &client, |s| s.bench = Some(d.clone()))
            .await;
        Ok(d)
    }

    async fn cached<T>(
        &self,
        name: &str,
        c: &Arc<Client>,
        f: impl Fn(&Slot) -> Option<T>,
    ) -> Option<T> {
        let slots = self.slots.lock().await;
        slots
            .get(name)
            .filter(|s| Arc::ptr_eq(&s.client, c))
            .and_then(f)
    }

    async fn store(&self, name: &str, c: &Arc<Client>, f: impl FnOnce(&mut Slot)) {
        if let Some(s) = self
            .slots
            .lock()
            .await
            .get_mut(name)
            .filter(|s| Arc::ptr_eq(&s.client, c))
        {
            f(s);
        }
    }

    /// `<saves_root>/<package>/<save>`, created.
    pub fn save_dir(&self, package: &str, save: &str) -> PathBuf {
        let d = self.saves_root.join(package).join(save);
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// Bytes on disk under the save storage.
    pub fn saves_bytes(&self) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(rd) = std::fs::read_dir(p) else {
                return 0;
            };
            rd.filter_map(Result::ok)
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => walk(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.saves_root)
    }
}
