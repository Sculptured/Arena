//! What a benchmark's world is made of: its parts (with their integration's `describe`) and the
//! tools agents get, each routed to the benchmark or to one part.

use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use parts::{
    proto::{BenchDescribe, Describe, Gate, Link, SetupCall, ToolSchema, ViewInfo, WorldPart},
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
    pub gates: Vec<Gate>,
    /// Present when the benchmark draws the world's view.
    pub view: Option<ViewInfo>,
}

/// The parts of `bench`'s world and its tool table: the benchmark's tools, then each part's.
/// Refuses a world in which two tools share a name (`handoff` counts as one) or a link names a
/// part or tool it doesn't have, or a gate, a task's setup, a scenario's setup or the view does. A
/// tool a link or a setup (a task's, or the one a scenario adds) calls is the world's own: agents
/// don't get it.
pub async fn plan(hub: &Hub, bench: &BenchDescribe) -> Result<Plan> {
    let parts = describe_parts(hub, bench).await?;
    let mut tools = tool_table(bench, &parts)?;
    check_links(&bench.links, &parts)?;
    let scenario_setup = scenario_setup(bench);
    let setup: Vec<&SetupCall> = (bench.tasks.iter().flat_map(|t| &t.setup))
        .chain(&scenario_setup)
        .collect();
    check_setup(&setup, &parts)?;
    check_view(bench.view.as_ref(), &parts)?;
    let worlds_own = |name: &str| {
        bench.links.iter().any(|l| l.tool == name) || setup.iter().any(|c| c.tool == name)
    };
    tools.retain(|(t, _)| !worlds_own(&t.name));
    check_gates(&bench.gates, &parts, &tools)?;
    Ok(Plan {
        parts,
        tools,
        links: bench.links.clone(),
        gates: bench.gates.clone(),
        view: bench.view.clone(),
    })
}

/// Every tool of the world, routed: the benchmark's, then each part's. Refused when two share a
/// name (`handoff` counts as one).
fn tool_table(bench: &BenchDescribe, parts: &[PlannedPart]) -> Result<Vec<(ToolSchema, Route)>> {
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
    Ok(tools)
}

/// The setup calls a scenario may add (one per part `scenario_options.setup` names).
fn scenario_setup(bench: &BenchDescribe) -> Vec<SetupCall> {
    let setup = bench.scenario_options.iter().flat_map(|o| &o.setup);
    setup
        .map(|(part, t)| SetupCall {
            part: part.clone(),
            tool: t.tool.clone(),
            args: t.args.clone(),
        })
        .collect()
}

fn check_view(view: Option<&ViewInfo>, parts: &[PlannedPart]) -> Result<()> {
    let docs = view.iter().flat_map(|v| &v.docs);
    for name in docs {
        if !parts.iter().any(|p| &p.part.part == name) {
            bail!("the view reads the docs of {name}, which is not a part");
        }
    }
    Ok(())
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

fn check_setup(calls: &[&SetupCall], parts: &[PlannedPart]) -> Result<()> {
    for c in calls {
        let Some(part) = parts.iter().find(|p| p.part.part == c.part) else {
            bail!("a task's setup calls {}, which is not a part", c.part);
        };
        if !part.describe.tools.iter().any(|t| t.name == c.tool) {
            bail!(
                "a task's setup calls {}, which {} does not have",
                c.tool,
                c.part
            );
        }
    }
    Ok(())
}

fn check_gates(gates: &[Gate], parts: &[PlannedPart], tools: &[(ToolSchema, Route)]) -> Result<()> {
    for g in gates {
        if !parts.iter().any(|p| p.part.part == g.part) {
            bail!("a gate reads {}, which is not a part", g.part);
        }
        if !tools.iter().any(|(t, _)| t.name == g.tool) {
            bail!("a gate is on {}, which agents don't have", g.tool);
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
