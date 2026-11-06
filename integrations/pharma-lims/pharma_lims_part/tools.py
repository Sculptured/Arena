"""The LIMS's tools: reads of the lab's records for agents, the scenario a task's setup sets, and the QA
head's review that a benchmark's link calls."""
from arena_part.jsonstate import Done, Tool
from arena_part.params import Param, Refusal

from . import lab, qa


def _em_results(config, state, args):
    rows = [s for s in lab.samples(config, state)
            if args["batch"] in (None, s["batch"]) and args["grade"] in (None, s["grade"])]
    return Done({"samples": rows}, reads=[f"sample/{s['id']}" for s in rows])


def _batch_results(config, state, args):
    return Done(lab.batch_results(config, state, args["batch"]), reads=[f"batch/{args['batch']}"])


def _documents(config, state, args):
    listed = [{k: d[k] for k in ("id", "title", "kind")} for d in lab.documents(config, state)]
    return Done({"documents": listed}, reads=[f"document/{d['id']}" for d in listed])


def _get_document(config, state, args):
    return Done(lab.document(config, state, args["id"]), reads=[f"document/{args['id']}"])


def _scenario(config, state, args):
    if args["name"] not in lab.scenarios(config):
        raise Refusal(f"unknown scenario {args['name']} (known: {', '.join(lab.scenarios(config))})")
    state["scenario"] = args["name"]
    return Done({"scenario": args["name"]}, writes=["scenario/current"])


TOOLS = {t.name: t for t in [
    Tool("lims_em_results", "Environmental-monitoring samples with their result (CFU), limits and status; "
         "optionally only one batch's or one grade's.", "read",
         (Param("batch", "string", None), Param("grade", "string", None)), _em_results),
    Tool("lims_batch_results", "A batch's release tests (sterility, endotoxins, potency, ...), whether they are "
         "complete and passed, and its environmental-monitoring excursions.", "read",
         (Param("batch", "string"),), _batch_results),
    Tool("lims_documents", "The lab's documents (SOPs, memos): id, title and kind.", "read", (), _documents),
    Tool("lims_get_document", "One document of the lab, with its text.", "read", (Param("id", "string"),),
         _get_document),
    Tool("lims_scenario", "Sets the lab's scenario (a task's setup calls it; agents don't get it).", "write",
         (Param("name", "string"),), _scenario),
    Tool("qa_review", "The QA head reviews a proposed change (a benchmark's link calls it; agents don't get it).",
         "write", (Param("id", "string"), Param("action", "string"), Param("payload", "object", None),
                   Param("summary", "string", None)), qa.review),
]}
