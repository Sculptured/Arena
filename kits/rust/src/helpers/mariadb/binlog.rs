//! The server's binlog: where it ends, its files, purging, and replaying a segment.
use anyhow::{anyhow, bail, Context, Result};
use mysql_async::prelude::Queryable;
use mysql_async::Row;

use super::docker::{self, Replay};
use super::login::Login;
use super::manifest::Segment;
use super::position::{span, BinlogFile, Position};
use super::server::{check_db, Server};
use super::sql::valid_binlog_name;

impl Server {
    /// The end of the binlog now.
    pub async fn position(&self) -> Result<Position> {
        let row: Option<Row> = self
            .with_conn(async |c| Ok(c.query_first("SHOW BINLOG STATUS").await?))
            .await?;
        let off = || anyhow!("binary logging is off in {}", self.config().container);
        let row = row.ok_or_else(off)?;
        let file: String = row.get(0).context("SHOW BINLOG STATUS: no file")?;
        let pos: u64 = row.get(1).context("SHOW BINLOG STATUS: no position")?;
        Ok(Position::new(file, pos))
    }

    /// The binlog files on the server, oldest first.
    pub async fn binlog_files(&self) -> Result<Vec<BinlogFile>> {
        let rows: Vec<Row> = self
            .with_conn(async |c| Ok(c.query("SHOW BINARY LOGS").await?))
            .await?;
        rows.into_iter()
            .map(|r| {
                let name: Option<String> = r.get(0);
                let size: Option<u64> = r.get(1);
                Ok(BinlogFile::new(
                    name.context("no Log_name")?,
                    size.context("no File_size")?,
                ))
            })
            .collect()
    }

    /// Removes the binlog files before `file`.
    pub async fn purge_to(&self, file: &str) -> Result<()> {
        if !valid_binlog_name(file) {
            bail!("{file:?} is not a binlog file name");
        }
        let sql = format!("PURGE BINARY LOGS TO '{file}'");
        self.with_conn(async |c| Ok(c.query_drop(sql).await?)).await
    }

    /// The folder of the binlog files inside the container.
    async fn binlog_dir(&self) -> Result<String> {
        let base: Option<String> = self
            .with_conn(async |c| Ok(c.query_first("SELECT @@log_bin_basename").await?))
            .await?;
        let base = base.context("binary logging is off")?;
        let (dir, _) = base.rsplit_once('/').context("odd log_bin_basename")?;
        Ok(dir.to_string())
    }

    /// Replays `seg` (its database's events between its positions) into `into` as `login`,
    /// unlogged. `login` should be limited to `into`, so a statement that names another database
    /// fails instead of touching it.
    pub async fn apply_log(&self, seg: &Segment, into: &str, login: &Login) -> Result<()> {
        check_db(&seg.db)?;
        check_db(into)?;
        let files = self.binlog_files().await?;
        let names = span(&files, &seg.from, &seg.to)?;
        let dir = self.binlog_dir().await?;
        let replay = Replay {
            db: seg.db.clone(),
            into: into.to_string(),
            files: names.iter().map(|f| format!("{dir}/{}", f.name)).collect(),
            start: seg.from.pos,
            stop: seg.to.pos,
        };
        let (cfg, login) = (self.config().clone(), login.clone());
        let task = move || docker::apply_binlog(&cfg, &login, &replay);
        tokio::task::spawn_blocking(task).await?
    }
}
