//! What a benchmark's world is made of: its parts (with their integration's `describe`) and the
//! tools agents get, each routed to the benchmark or to one part.

use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use parts::{
    proto::{BenchDescribe, Describe, Link, ToolSchema, WorldPart},
    Hub,
};

/// A part of the world with its integration's `describe`.
pub struct PlannedPart {
    pub part: WorldPart,
    pub describe: Describe,
}

/// Where a tool call goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Bench,
    /// The part at this index of the world.
    Part(usize),
}

pub struct Plan {
    pub parts: Vec<PlannedPart>,
    pub tools: Vec<(ToolSchema, Route)>,
    pub links: Vec<Link>,
}

/// The parts of `bench`'s world and its tool table: the benchmark's tools, then each part's.
/// Refuses a world in which two tools share a name (`handoff` counts as one) or a link names a
/// part or tool it doesn't have. A tool a link calls is the world's own: agents don't get it.
pub async fn plan(hub: &Hub, bench: &BenchDescribe) -> Result<Plan> {
    let parts = describe_parts(hub, bench).await?;
    let mut tools: Vec<(ToolSchema, Route)> = bench
        .tools
        .iter()
        .map(|t| (t.clone(), Route::Bench))
        .collect();
    for (i, p) in parts.iter().enumerate() {
        tools.extend(p.describe.tools.iter().map(|t| (t.clone(), Route::Part(i))));
    }
    let mut seen = BTreeSet::from(["handoff"]);
    if let Some((t, _)) = tools.iter().find(|(t, _)| !seen.insert(t.name.as_str())) {
        bail!("two tools are called {}", t.name);
    }
    check_links(&bench.links, &parts)?;
    tools.retain(|(t, _)| !bench.links.iter().any(|l| l.tool == t.name));
    let links = bench.links.clone();
    Ok(Plan {
        parts,
        tools,
        links,
    })
}

fn check_links(links: &[Link], parts: &[PlannedPart]) -> Result<()> {
    let part = |name: &str| parts.iter().find(|p| p.part.part == name);
    for l in links {
        if part(&l.from).is_none() {
            bail!("a link comes from {}, which is not a part", l.from);
        }
        let Some(to) = part(&l.to) else {
            bail!("a link goes to {}, which is not a part", l.to);
        };
        if !to.describe.tools.iter().any(|t| t.name == l.tool) {
            bail!("a link calls {}, which {} does not have", l.tool, l.to);
        }
    }
    Ok(())
}

async fn describe_parts(hub: &Hub, bench: &BenchDescribe) -> Result<Vec<PlannedPart>> {
    if bench.world.is_empty() {
        bail!("the benchmark {} names no world parts", bench.name);
    }
    let mut names = BTreeSet::new();
    let mut parts = vec![];
    for wp in &bench.world {
        if !names.insert(wp.part.as_str()) {
            bail!(
                "the benchmark {} names the part {} twice",
                bench.name,
                wp.part
            );
        }
        let describe = hub.describe(&wp.integration).await.with_context(|| {
            format!(
                "describing the {} integration of part {}",
                wp.integration, wp.part
            )
        })?;
        parts.push(PlannedPart {
            part: wp.clone(),
            describe,
        });
    }
    Ok(parts)
}
