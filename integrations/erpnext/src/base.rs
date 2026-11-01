//! The base every instance starts from: a site with ERPNext installed and set up, made once and
//! kept in `$ARENA_SAVES/erpnext/.base/` as a save (a dump-rooted manifest and the site's
//! files) plus what every site made from it needs: the shared site config and the
//! Administrator's API key.
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use arena_part::helpers::folder;
use arena_part::helpers::mariadb::{Blobs, Login, Manifest};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::frappe::{parse_keys, ApiKey};
use crate::jobs;
use crate::json_file;
use crate::names::Site;
use crate::rest::{Reply, Rest};
use crate::site_config::shared_part;
use crate::stack::Stack;

const SHARED: &str = "site_config.json";
const CREDENTIALS: &str = "api.json";
/// The setup wizard's background jobs may take a while.
const SETUP_JOBS_TIMEOUT: Duration = Duration::from_secs(600);

/// The Administrator's API key, and password for logging in to the site's web pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(flatten)]
    pub key: ApiKey,
    pub admin_password: String,
}

#[derive(Debug, Clone)]
pub struct Base {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub shared: Map<String, Value>,
    pub creds: Credentials,
}

impl Base {
    /// The base kept in `dir`; `None` when there is no complete one (its manifest is written
    /// last).
    pub fn load(dir: &Path) -> io::Result<Option<Base>> {
        let manifest = match Manifest::read(dir) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(Base {
            dir: dir.to_path_buf(),
            manifest,
            shared: json_file::read(&dir.join(SHARED))?,
            creds: json_file::read(&dir.join(CREDENTIALS))?,
        }))
    }

    /// Its files, as a save keeps them.
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }
}

/// Makes the base in `dir`: installs a site, sets it up, saves it, and removes the site. Hold
/// whatever keeps unused blobs from being swept while this runs.
pub async fn make(stack: &Stack, blobs: &Blobs, dir: &Path) -> Result<Base> {
    let site = Site::base();
    discard_site(stack, &site).await;
    folder::discard(dir);
    let made = build(stack, blobs, dir, &site).await;
    discard_site(stack, &site).await;
    if made.is_err() {
        folder::discard(dir);
    }
    made
}

/// Removes the site, its database and its account (errors are ignored: there may be none).
async fn discard_site(stack: &Stack, site: &Site) {
    let _ = stack.sites.remove(&site.name);
    let _ = stack.server.drop_db(&site.db).await;
    let _ = stack.server.drop_login(&site.db).await;
}

async fn build(stack: &Stack, blobs: &Blobs, dir: &Path, site: &Site) -> Result<Base> {
    let admin_password = install(stack, site).await?;
    let creds = Credentials {
        key: generate_key(stack, site).await?,
        admin_password,
    };
    let rest = stack.rest.with_key(&creds.key);
    complete_setup(&rest, &site.name).await?;
    jobs::drain(&rest, &site.name, SETUP_JOBS_TIMEOUT).await?;
    store(stack, blobs, dir, site, creds).await
}

/// `bench new-site` on a database and account the mariadb helper made (so the install runs
/// with the same grants every instance has). Returns the Administrator's password.
async fn install(stack: &Stack, site: &Site) -> Result<String> {
    stack.server.create_db(&site.db).await?;
    let login = stack.server.create_login(&site.db, &site.db, "").await?;
    let admin = Login::new_random("admin")?.password;
    let args: [&str; 13] = [
        "new-site",
        &site.name,
        "--no-setup-db",
        "--db-name",
        &site.db,
        "--db-user",
        &login.user,
        "--db-password",
        &login.password,
        "--admin-password",
        &admin,
        "--install-app",
        "erpnext",
    ];
    stack.bench(&args).await.context("installing ERPNext")?;
    Ok(admin)
}

/// A new API key for the Administrator.
async fn generate_key(stack: &Stack, site: &Site) -> Result<ApiKey> {
    let keys: [&str; 6] = [
        "--site",
        &site.name,
        "execute",
        "frappe.core.doctype.user.user.generate_keys",
        "--args",
        "[\"Administrator\"]",
    ];
    let out = stack.bench(&keys).await?;
    parse_keys(&out).context("generate_keys printed no key")
}

/// The setup wizard's answers: a company with the standard chart of accounts, so items, orders
/// and entries can be made.
fn setup_answers() -> Value {
    json!({"args": {
        "language": "English", "country": "United States", "timezone": "UTC",
        "currency": "USD", "company_name": "Arena Co", "company_abbr": "AC",
        "chart_of_accounts": "Standard", "fy_start_date": "2026-01-01",
        "fy_end_date": "2026-12-31", "enable_telemetry": 0,
    }})
}

async fn complete_setup(rest: &Rest, site: &str) -> Result<()> {
    let path = [
        "method",
        "frappe.desk.page.setup_wizard.setup_wizard.setup_complete",
    ];
    match rest
        .send(Method::POST, site, &path, &setup_answers())
        .await?
    {
        Reply::Ok(_) => Ok(()),
        Reply::Refused(e) => bail!("the setup wizard failed: {e}"),
    }
}

/// Saves the site into `dir`; the manifest last, so a base is complete once it has one.
async fn store(
    stack: &Stack,
    blobs: &Blobs,
    dir: &Path,
    site: &Site,
    creds: Credentials,
) -> Result<Base> {
    let dumped = stack.server.dump(&site.db, blobs).await?;
    std::fs::create_dir_all(dir)?;
    stack
        .sites
        .save_files(&site.name, &dir.join("files"), None)?;
    let shared = shared_part(&stack.sites.config(&site.name)?);
    json_file::write(&dir.join(SHARED), &shared)?;
    json_file::write(&dir.join(CREDENTIALS), &creds)?;
    let manifest = Manifest::rooted(stack.server.reference().await?, dumped.blob);
    manifest.write(dir)?;
    Ok(Base {
        dir: dir.to_path_buf(),
        manifest,
        shared,
        creds,
    })
}
