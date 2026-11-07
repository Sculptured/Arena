"""The deployments the summit demo drives, built from the benchmark's suggested designs
(benchmarks/pharma/data/teams.json): Team v1 and Team v2 share the team's roles (v2 adds its rules
to every prompt, as the UI's Swap architecture does), the single agent is the single design."""
from calls import TEAMS
from decisions import NAMES

DISCLOSURE = "scripted agents, illustrative"
MARK = "\n\nDeployment rules:\n"
V2_RULES = "\n".join([
    "Book a shipment only after its cold_chain_shipment change is approved; never book first and ask after.",
    "Before allocating a lot, check its remaining shelf life on arrival against the customer's minimum; "
    "never ship a short-dated lot.",
    "Before booking, list the shipments already booked so no order is shipped twice.",
])


def with_rules(prompt, rules):
    """The prompt with `rules` in place of any earlier deployment rules (the UI's withRules)."""
    base = prompt.split(MARK)[0]
    return f"{base.rstrip()}{MARK}{rules.strip()}" if rules.strip() else base


def agent(role, prompt):
    return {"id": role["id"], "name": role["name"], "kind": "external", "model": "scripted-illustrative",
            "read_only": role["read_only"], "prompt": f"{prompt} Demo: {DISCLOSURE}."}


def team(deployment, expediting=False):
    """The deployment's team. With `expediting`, the coordinator may act (book an escalated order)."""
    design = next(d for d in TEAMS["deployments"] if d["id"] == ("single" if deployment == "single" else "team"))
    agents = []
    for role in design["roles"]:
        prompt = role["prompt"]
        if expediting and role["id"] == "coordinator":
            role = {**role, "read_only": False}
            prompt += " Act on customers' urgent requests yourself."
        agents.append(agent(role, prompt))
    if deployment == "v2":
        agents = [{**a, "prompt": with_rules(a["prompt"], V2_RULES)} for a in agents]
    return {"id": f"summit-{deployment}", "name": f"Summit: {NAMES[deployment]} · {DISCLOSURE}", "agents": agents}


def fork_body(parent_team, at_seq, deployment):
    """The fork request that swaps the parent's team to `deployment`: v2's rules as prompt changes,
    or the single agent as another team (a fork of a run an external system drives may do that)."""
    if deployment == "single":
        return {"at_seq": at_seq, "team": team("single")}
    if deployment == "v2":
        changes = {a["id"]: {"prompt": with_rules(a["prompt"], V2_RULES)} for a in parent_team["agents"]}
        return {"at_seq": at_seq, "changes": changes}
    return {"at_seq": at_seq}
