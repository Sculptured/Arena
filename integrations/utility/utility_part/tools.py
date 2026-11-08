"""The utility's tools: the site meter and the demand-response events for agents, the reading the plant sends
(a benchmark's link calls it) and the scenario a task's setup sets."""
from arena_part.jsonstate import Done, Tool, set_scenario
from arena_part.params import Param, Refusal

from . import disrupt, meter

BASE = "base"


def _meter(config, state, args):
    return Done(meter.meter_doc(config, state), reads=["meter/site"])


def _events(config, state, args):
    rows = [meter.event_doc(config, ev) for ev in state["events"].values()]
    return Done({"events": rows}, reads=[f"dr/{r['id']}" for r in rows])


def _respond(config, state, args):
    ev = state["events"].get(args["event"])
    if ev is None:
        raise Refusal(f"unknown demand-response event {args['event']}")
    if ev["status"] == "ended":
        raise Refusal(f"{ev['id']} has ended")
    if args["committed_kw"] < 0:
        raise Refusal("committed_kw must be 0 or more")
    ev["response"] = {"committed_kw": args["committed_kw"], "plan": args["plan"],
                      "at": meter.plant_time(config, state["clock_ms"])}
    return Done(meter.event_doc(config, ev), writes=[f"dr/{ev['id']}"])


def _load(config, state, args):
    if args["kw"] < 0:
        raise Refusal("kw must be 0 or more")
    meter.read(state, args["kw"])
    return Done({"kw": state["meter"]["w"] / 1000}, writes=["meter/site"])


def scenarios(config) -> list:
    return [BASE, *config.get("scenarios", {})]


def _overlay(config, state) -> dict:
    return config.get("scenarios", {}).get(state["scenario"], {})


def _scenario(config, state, args):
    return set_scenario(config, state, args, scenarios(config), _overlay, disrupt.KINDS, TOOLS)


TOOLS = {t.name: t for t in [
    Tool("utility_meter", "The site's main meter now: the load in kW, the peak so far, the energy and its cost "
         "this shift, and the tariff period and price now.", "read", (), _meter),
    Tool("demand_response_events", "The utility's demand-response events for the site: the target kW, the "
         "window (plant time), its status (notified, active, ended), the site's response, and how the site has "
         "done so far inside the window.", "read", (), _events),
    Tool("respond_to_event", "Answer a demand-response event: the kW the site commits to hold during the "
         "window and the plan (which loads it sheds).", "write",
         (Param("event", "string"), Param("committed_kw", "number"), Param("plan", "string")), _respond),
    Tool("meter_load", "A reading of the site's load from the plant (a benchmark's link calls it; agents don't "
         "get it).", "write", (Param("kw", "number"), Param("cause", "string", None)), _load),
    Tool("utility_scenario", "Sets the utility's scenario and schedules its disruptions (a task's setup calls "
         "it; agents don't get it).", "write",
         (Param("name", "string"), Param("disruptions", "array", None)), _scenario),
]}
