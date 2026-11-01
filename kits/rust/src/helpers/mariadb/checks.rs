//! Whether a save can be restored here, as protocol errors.
use super::manifest::ServerRef;
use super::position::Purged;
use super::tracked::Misplaced;
use crate::PartError;

/// -32001 unless `save` was made on the server `here` (same container and identity).
pub fn check_same_server(save: &str, saved: &ServerRef, here: &ServerRef) -> Result<(), PartError> {
    if saved.container != here.container || saved.port != here.port {
        return Err(PartError::cant_restore(format!(
            "save {save} was made on {}, this instance is set for {}",
            saved.container, here.container
        )));
    }
    if saved.id != here.id {
        return Err(PartError::cant_restore(format!(
            "can't restore {save}: the MariaDB server was replaced since this save"
        )));
    }
    Ok(())
}

/// A failed restore: -32001 when the binlog it needs was purged.
pub fn restore_error(save: &str, e: anyhow::Error) -> PartError {
    match e.downcast_ref::<Purged>() {
        Some(p) => PartError::cant_restore(format!("can't restore {save}: {p}")),
        None => PartError::internal(format!("restoring {save}: {e:#}")),
    }
}

/// A failed [`Server::chain`](super::Server::chain) of instance `id`: -32001 when DDL went to
/// its database under another default database, so the save can't be exact.
pub fn chain_error(id: &str, e: anyhow::Error) -> PartError {
    match e.downcast_ref::<Misplaced>() {
        Some(Misplaced(q)) => PartError::cant_restore(format!(
            "changes to {id} were made while another database was selected, so this save \
             can't be exact ({q})"
        )),
        None => PartError::from(e),
    }
}
