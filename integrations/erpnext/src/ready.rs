//! Bringing the install up on first need: the stack, then the base (made the first time).
use std::path::PathBuf;
use std::time::Instant;

use arena_part::helpers::mariadb::{Server, ServerRef};
use arena_part::PartError;

use crate::base::{self, Base};
use crate::rest::Rest;
use crate::settings::StackConfig;
use crate::stack::Stack;
use crate::Erpnext;

/// The stack up and the base there; `rest` sends the base's API key.
pub struct Ready {
    pub stack: Stack,
    pub base: Base,
    pub rest: Rest,
    pub server_ref: ServerRef,
}

impl Erpnext {
    /// The install, brought up (and the base made) once: the one a start's config chose, else
    /// the program's defaults.
    pub(crate) async fn ready(&self) -> Result<&Ready, PartError> {
        self.ready_for(&self.cfg).await
    }

    /// The install of `cfg`, brought up once; -32602 when the program already runs another.
    pub(crate) async fn ready_for(&self, cfg: &StackConfig) -> Result<&Ready, PartError> {
        if let Some(r) = self.ready.get() {
            return match &r.stack.cfg == cfg || cfg == &self.cfg {
                true => Ok(r),
                false => Err(PartError::invalid(format!(
                    "this program runs the ERPNext install of project {} on {}; one install per \
                     program",
                    r.stack.cfg.project, r.stack.cfg.server.container
                ))),
            };
        }
        self.ready
            .get_or_try_init(|| self.bring_up(cfg.clone()))
            .await
    }

    /// The MariaDB server of the install: the running one, else the program's default.
    pub(crate) fn server(&self) -> &Server {
        self.ready
            .get()
            .map_or(&self.default_server, |r| &r.stack.server)
    }

    /// `.sites` as an absolute path (the containers mount it).
    fn sites_root(&self) -> Result<PathBuf, PartError> {
        let dir = self.dirs.root().join(".sites");
        std::fs::create_dir_all(&dir).map_err(PartError::internal_from)?;
        dir.canonicalize().map_err(PartError::internal_from)
    }

    async fn bring_up(&self, cfg: StackConfig) -> Result<Ready, PartError> {
        let server = Server::new(cfg.server.clone()).map_err(PartError::internal_from)?;
        let stack = Stack::new(cfg, server, self.sites_root()?)?;
        stack.up().await?;
        let base = self.load_or_make_base(&stack).await?;
        Ok(Ready {
            rest: stack.rest.with_key(&base.creds.key),
            server_ref: stack.server.reference().await?,
            stack,
            base,
        })
    }

    async fn load_or_make_base(&self, stack: &Stack) -> Result<Base, PartError> {
        let dir = self.dirs.root().join(".base");
        if let Some(b) = Base::load(&dir).map_err(PartError::internal_from)? {
            return Ok(b);
        }
        let t = Instant::now();
        let made = {
            let _gate = self.blob_gate.lock().await;
            base::make(stack, &self.blobs, &dir).await
        };
        let made =
            made.map_err(|e| PartError::internal(format!("making the ERPNext base: {e:#}")))?;
        eprintln!("arena-erpnext: made the base site in {:?}", t.elapsed());
        // The install's binlog is not needed once the base is a dump.
        if let Err(e) = self.settle_on(&stack.server).await {
            eprintln!("arena-erpnext: tidying up after the base failed: {e}");
        }
        Ok(made)
    }
}
