//! Instances: a site `<id>.localhost` whose database is restored from the base or a save, with
//! its own account limited to that database and its files from the save.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use arena_part::helpers::busy::Busy;
use arena_part::helpers::mariadb::{check_same_server, restore_error, Manifest, Position, Tracked};
use arena_part::proto::StartParams;
use arena_part::PartError;

use crate::names::Site;
use crate::ready::Ready;
use crate::rest::{Reply, Rest};
use crate::settings::Settings;
use crate::site_config::site_config;
use crate::stack::Stack;
use crate::Erpnext;

/// How long a new site may take to answer (Frappe keeps a site's config for up to 60 s, so a
/// site made again under an old name answers once that expires).
const SITE_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
pub struct Instance {
    pub site: Site,
    pub tracked: Tracked,
    pub settings: Settings,
    /// The files of the save it last saved or started from (a save links unchanged files).
    pub prev_files: PathBuf,
}

/// What a site is made from: a save (or the base), its manifest and its files.
pub struct Source {
    pub save: String,
    pub manifest: Manifest,
    pub files: PathBuf,
}

impl Erpnext {
    pub(crate) fn instance(&self, id: &str) -> Result<Instance, PartError> {
        let live = self.live.lock().expect("instances lock");
        let found = live.get(id).cloned();
        found.ok_or_else(|| PartError::unknown(format!("unknown instance {id}")))
    }

    pub(crate) async fn start_instance(&self, p: StartParams) -> Result<(), PartError> {
        let settings = Settings::from_config(&p.config)?;
        let site = Site::instance(&p.instance).ok_or_else(|| {
            PartError::invalid("instance ids for erpnext are at most 61 characters")
        })?;
        // Marked until it is live, so neither `gc` nor a second start of the id takes it.
        let _busy = self.claim(&p.instance, &site)?;
        let ready = self.ready_for(&self.cfg.with_config(&p.config)?).await?;
        let source = self.source(ready, p.from.as_ref().map(|f| f.save.as_str()))?;
        let _starting = self.starts.read().await;
        let start = self.open_site(ready, &source, &site).await?;
        let inst = Instance {
            tracked: Tracked::new(&site.db, source.manifest, start),
            site,
            settings,
            prev_files: source.files,
        };
        self.live
            .lock()
            .expect("instances lock")
            .insert(p.instance, inst);
        Ok(())
    }

    /// Marks the site of instance `id` busy; -32602 when the instance runs or is starting.
    fn claim(&self, id: &str, site: &Site) -> Result<Busy<'_>, PartError> {
        let running = self.live.lock().expect("instances lock").contains_key(id);
        let busy = (!running).then(|| self.busy.try_mark(&site.db)).flatten();
        busy.ok_or_else(|| PartError::invalid(format!("instance {id} is already running")))
    }

    /// The base, or the save `save` (-32002 when it has no folder, -32001 when it was made on
    /// another server).
    pub(crate) fn source(&self, ready: &Ready, save: Option<&str>) -> Result<Source, PartError> {
        let Some(save) = save else {
            return Ok(Source {
                save: "the base".into(),
                manifest: ready.base.manifest.clone(),
                files: ready.base.files(),
            });
        };
        let dir = self.dirs.find(save);
        let dir = dir.ok_or_else(|| PartError::unknown(format!("unknown save {save}")))?;
        let manifest =
            Manifest::read(&dir).map_err(|e| PartError::internal(format!("save {save}: {e}")))?;
        check_same_server(save, &manifest.server, &ready.server_ref)?;
        Ok(Source {
            save: save.into(),
            manifest,
            files: dir.join("files"),
        })
    }

    /// Makes `site` from `source` (its account is named like its database) and waits until it
    /// answers; on failure nothing of it is left. Returns the binlog position its own changes
    /// start at.
    pub(crate) async fn open_site(
        &self,
        ready: &Ready,
        source: &Source,
        site: &Site,
    ) -> Result<Position, PartError> {
        let built = self.build_site(ready, source, site).await;
        if built.is_err() {
            let _ = close_site(&ready.stack, site).await;
        }
        built
    }

    async fn build_site(
        &self,
        ready: &Ready,
        source: &Source,
        site: &Site,
    ) -> Result<Position, PartError> {
        let server = &ready.stack.server;
        let restored = server
            .restore(&source.manifest, self.dumps.blobs(), &site.db)
            .await;
        let start = restored.map_err(|e| restore_error(&source.save, e))?;
        let login = server.create_login(&site.db, &site.db, "").await?;
        let config = site_config(&ready.base.shared, &site.db, &login);
        let sites = &ready.stack.sites;
        sites
            .create(&site.name, &source.files, &config)
            .map_err(PartError::internal_from)?;
        ready.stack.clear_cache(&site.db).await?;
        wait_site(&ready.rest, &site.name).await?;
        Ok(start)
    }

    pub(crate) async fn stop_instance(&self, id: &str) -> Result<(), PartError> {
        let inst = self.live.lock().expect("instances lock").remove(id);
        let Some(i) = inst else {
            return Ok(());
        };
        let ready = self.ready().await?;
        close_site(&ready.stack, &i.site).await
    }
}

/// Removes the site's folder, account and database: all three, even when one fails (the
/// first error is the answer).
pub async fn close_site(stack: &Stack, site: &Site) -> Result<(), PartError> {
    let folder = stack
        .sites
        .remove(&site.name)
        .map_err(PartError::internal_from);
    let login = stack
        .server
        .drop_login(&site.db)
        .await
        .map_err(PartError::from);
    let db = stack
        .server
        .drop_db(&site.db)
        .await
        .map_err(PartError::from);
    folder.and(login).and(db)
}

/// Waits until `site` answers an authenticated request.
async fn wait_site(rest: &Rest, site: &str) -> Result<(), PartError> {
    let deadline = Instant::now() + SITE_TIMEOUT;
    loop {
        let why = match rest
            .get(site, &["method", "frappe.auth.get_logged_user"], &[])
            .await
        {
            Ok(Reply::Ok(_)) => return Ok(()),
            Ok(Reply::Refused { message: e, .. }) => e,
            Err(e) => format!("{e:#}"),
        };
        if Instant::now() > deadline {
            return Err(PartError::internal(format!(
                "site {site} did not answer: {why}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
