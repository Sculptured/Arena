use anyhow::{Context, Result};
use tokio::sync::OnceCell;
use tokio_postgres::{Client, NoTls};

/// Administrative handle on a Postgres server (connected to the `postgres` database).
pub struct Admin {
    base_url: String,
    client: OnceCell<Client>,
}

impl Admin {
    /// `url` is a server URL such as `postgres://arena:arena@127.0.0.1:5544`
    /// (an optional trailing `/dbname` is ignored).
    pub async fn connect(url: &str) -> Result<Admin> {
        let admin = Admin::lazy(url);
        admin.client().await?;
        Ok(admin)
    }

    /// Like `connect`, but defers connecting until first use (for sync test helpers).
    pub fn lazy(url: &str) -> Admin {
        Admin { base_url: strip_db(url), client: OnceCell::new() }
    }

    async fn client(&self) -> Result<&Client> {
        self.client
            .get_or_try_init(|| async {
                let (client, conn) =
                    tokio_postgres::connect(&format!("{}/postgres", self.base_url), NoTls)
                        .await
                        .with_context(|| format!("connecting to {}", self.base_url))?;
                tokio::spawn(conn);
                Ok::<_, anyhow::Error>(client)
            })
            .await
    }

    /// Runs `f` on a fresh connection to `db`, then closes it for real (drops the
    /// client and awaits the connection task) before returning, so the backend is
    /// gone by the time a later `CREATE DATABASE ... TEMPLATE db` runs.
    pub async fn with_db<T>(
        &self,
        db: &str,
        f: impl AsyncFnOnce(&mut Client) -> Result<T>,
    ) -> Result<T> {
        let (mut client, conn) = tokio_postgres::connect(&self.db_url(db), NoTls)
            .await
            .with_context(|| format!("connecting to {db}"))?;
        let handle = tokio::spawn(conn);
        let out = f(&mut client).await;
        drop(client);
        let _ = handle.await;
        out
    }

    pub fn db_url(&self, db: &str) -> String {
        format!("{}/{}", self.base_url, db)
    }

    pub async fn create_db_from(&self, new: &str, template: &str) -> Result<()> {
        let sql = format!("CREATE DATABASE {} TEMPLATE {}", quote(new), quote(template));
        self.client().await?.batch_execute(&sql).await?;
        Ok(())
    }

    pub async fn drop_db(&self, name: &str) -> Result<()> {
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quote(name));
        self.client().await?.batch_execute(&sql).await?;
        Ok(())
    }

    pub async fn exists(&self, name: &str) -> Result<bool> {
        let row = self
            .client()
            .await?
            .query_one("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)", &[&name])
            .await?;
        Ok(row.get(0))
    }

    pub(crate) async fn create_db(&self, name: &str) -> Result<()> {
        self.client().await?.batch_execute(&format!("CREATE DATABASE {}", quote(name))).await?;
        Ok(())
    }
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn strip_db(url: &str) -> String {
    let url = url.trim_end_matches('/');
    match url.find("://") {
        Some(i) => match url[i + 3..].find('/') {
            Some(j) => url[..i + 3 + j].to_string(),
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}
