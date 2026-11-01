//! Why a part that declares an exact restore still can't be restored exactly on a rerun: the
//! benchmark's world changed since the run, or the replay goes through a branch that restored
//! the part approximately.

use std::collections::{BTreeMap, BTreeSet};

use parts::proto::WorldPart;

use crate::{
    model::{Branch, RestoredHow, RestoredPart},
    restore::{How, Refused},
    store::Store,
};

fn refused(part: &str, reason: String) -> Refused {
    Refused {
        part: part.to_string(),
        reason,
    }
}

/// How a part of the current world differs from the world the run recorded.
fn changed_part(recorded: &[WorldPart], now: &WorldPart, how: Option<How>) -> Option<Refused> {
    let name = &now.part;
    let Some(was) = recorded.iter().find(|p| p.part == *name) else {
        return Some(refused(name, format!(
            "the part {name} was added to the benchmark's world since the run, so it can't be restored exactly; choose Approximate"
        )));
    };
    if was.integration != now.integration {
        return Some(refused(
            name,
            format!(
            "the part {name}'s integration changed since the run, so it can't be restored exactly"
        ),
        ));
    }
    (was.config != now.config && how == Some(How::Replay)).then(|| refused(name, format!(
        "the part {name}'s config changed since the run, so it can't be restored exactly; choose Approximate"
    )))
}

/// The first part of the world `now` that can't be restored exactly because the world changed
/// since the run recorded it as `recorded`: a part added or removed, an integration changed, or
/// the config of a part restored by replay changed (a save carries the state of a part restored
/// from it). A run that recorded no world (made before integrations) is not checked.
pub fn world_changed(
    recorded: &[WorldPart],
    now: &[WorldPart],
    hows: &BTreeMap<String, How>,
) -> Result<(), Refused> {
    if recorded.is_empty() {
        return Ok(());
    }
    if let Some(r) = now
        .iter()
        .find_map(|p| changed_part(recorded, p, hows.get(&p.part).copied()))
    {
        return Err(r);
    }
    match recorded.iter().find(|p| !now.iter().any(|n| n.part == p.part)) {
        Some(gone) => Err(refused(&gone.part, format!(
            "the part {} was removed from the benchmark's world since the run, so it can't be restored exactly; choose Approximate",
            gone.part
        ))),
        None => Ok(()),
    }
}

/// A fork in a branch's ancestry: where it started and how it restored each part.
#[derive(Debug, Clone, PartialEq)]
pub struct EarlierFork {
    pub start_seq: i64,
    pub restored: Vec<RestoredPart>,
}

/// The forks a branch's events go through, nearest first: the branch itself when it is a fork,
/// then each forked ancestor up to the run's first branch.
pub async fn ancestry(store: &Store, branch: &Branch) -> anyhow::Result<Vec<EarlierFork>> {
    let mut out = vec![];
    let mut b = branch.clone();
    loop {
        if let Some(start_seq) = b.start_seq {
            out.push(EarlierFork {
                start_seq,
                restored: b.restored.clone(),
            });
        }
        let Some(parent) = &b.parent_id else {
            return Ok(out);
        };
        b = store
            .find_branch(parent)
            .await?
            .ok_or_else(|| anyhow::anyhow!("branch {} has no parent {parent}", b.id))?;
    }
}

/// A part replayed up to `start_seq` whose state, somewhere on the way, came from an approximate
/// restore of an earlier fork: replaying it through that branch's history can't be exact. A
/// fork's own steps are on the way when `start_seq` is past its start; before it, the steps are
/// its parent's.
pub fn approximate_on_the_way(
    forks: &[EarlierFork],
    replayed: &BTreeSet<String>,
    start_seq: i64,
) -> Result<(), Refused> {
    let mut until = start_seq;
    for f in forks {
        let approximate = f
            .restored
            .iter()
            .filter(|r| r.how == RestoredHow::Approximate)
            .find(|r| replayed.contains(&r.part));
        if let Some(r) = approximate.filter(|_| until > f.start_seq) {
            return Err(refused(&r.part, format!(
                "{} was restored approximately at step {} in an earlier rerun, so it can't be restored exactly; choose Approximate",
                r.part, f.start_seq
            )));
        }
        until = until.min(f.start_seq);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn wp(part: &str, integration: &str, config: Value) -> WorldPart {
        WorldPart {
            part: part.into(),
            integration: integration.into(),
            config,
        }
    }

    fn hows(list: &[(&str, How)]) -> BTreeMap<String, How> {
        list.iter().map(|(p, h)| (p.to_string(), *h)).collect()
    }

    #[test]
    fn a_changed_world_refuses_the_part_it_changed() {
        let was = [
            wp("c", "counter", json!({})),
            wp("db", "postgres", json!({})),
        ];
        let save = How::Save(parts::proto::Mode::Exact);
        let h = hows(&[("c", How::Replay), ("db", save)]);
        assert_eq!(world_changed(&was, &was, &h), Ok(()));
        assert_eq!(world_changed(&[], &was, &h), Ok(()), "a legacy run");
        let config = [wp("c", "counter", json!({"n": 2})), was[1].clone()];
        let e = world_changed(&was, &config, &h).unwrap_err();
        assert_eq!(e.part, "c");
        assert!(e.reason.contains("config changed since the run"), "{e}");
        // A part restored from its save carries its state: its config may change.
        let db_config = [was[0].clone(), wp("db", "postgres", json!({"x": 1}))];
        assert_eq!(world_changed(&was, &db_config, &h), Ok(()));
        let db_integration = [was[0].clone(), wp("db", "mariadb", json!({}))];
        assert!(world_changed(&was, &db_integration, &h)
            .unwrap_err()
            .reason
            .contains("integration changed"));
        let added = [was[0].clone(), was[1].clone(), wp("x", "toy", json!({}))];
        assert_eq!(world_changed(&was, &added, &h).unwrap_err().part, "x");
        assert_eq!(world_changed(&was, &was[..1], &h).unwrap_err().part, "db");
    }

    fn fork(start_seq: i64, part: &str, how: RestoredHow) -> EarlierFork {
        EarlierFork {
            start_seq,
            restored: vec![RestoredPart {
                part: part.into(),
                how,
            }],
        }
    }

    fn set(parts: &[&str]) -> BTreeSet<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn an_approximate_restore_on_the_way_refuses_exact() {
        let c = set(&["c"]);
        // B forked from A at 10, restoring c approximately.
        let b = [fork(10, "c", RestoredHow::Approximate)];
        let e = approximate_on_the_way(&b, &c, 12).unwrap_err();
        assert_eq!(
            e.to_string(),
            "c was restored approximately at step 10 in an earlier rerun, so it can't be restored exactly; choose Approximate"
        );
        // At or before B's start the steps are A's.
        assert_eq!(approximate_on_the_way(&b, &c, 10), Ok(()));
        // A part restored from its save is not replayed.
        assert_eq!(approximate_on_the_way(&b, &set(&[]), 12), Ok(()));
        let replayed = [fork(10, "c", RestoredHow::Replay)];
        assert_eq!(approximate_on_the_way(&replayed, &c, 12), Ok(()));
        // C forked from B at 15 (exactly); D forks C at 12: the steps up to 12 are B's.
        let d = [fork(15, "c", RestoredHow::Replay), b[0].clone()];
        assert_eq!(approximate_on_the_way(&d, &c, 12).unwrap_err().part, "c");
        assert_eq!(approximate_on_the_way(&d, &c, 9), Ok(()));
    }
}
