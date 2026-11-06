"""The logistics tools: the qualified-lanes register, quotes, bookings and notices for agents, the scenario a
task's setup sets, and the supply chain director's review that a benchmark's link calls."""
from arena_part.jsonstate import Done, Tool
from arena_part.params import Param, Refusal

from . import director, freight, network


def _lanes(config, state, args):
    register = [{**l, "capacity_left": network.capacity_left(config, state, l)} for l in config["logistics"]["lanes"]]
    return Done({"origins": config["logistics"]["origins"], "lanes": register}, reads=["register/lanes"])


def _listed(kind, field):
    def run(config, state, args):
        rows = list(state[field].values())
        return Done({field: rows}, reads=[f"{kind}/{r['id']}" for r in rows])
    return run


def _scenario(config, state, args):
    if args["name"] not in network.scenarios(config):
        raise Refusal(f"unknown scenario {args['name']} (known: {', '.join(network.scenarios(config))})")
    state["scenario"] = args["name"]
    return Done({"scenario": args["name"]}, writes=["scenario/current"])


TOOLS = {t.name: t for t in [
    Tool("qualified_lanes", "The carriers' lanes: origin, destination, service, temperature, whether the lane is "
         "qualified for 2-8 C, transit days, price and shipments left this week.", "read", (), _lanes),
    Tool("cold_chain_quotes", "Quotes from every carrier with a lane from `origin` (US or EU) to the customer, "
         "for `qty` units shipped today.", "write",
         (Param("origin", "string"), Param("customer", "string"), Param("qty", "integer")), freight.quotes),
    Tool("book_shipment", "Book a quote for one order and batch. Needs an approved cold_chain_shipment change "
         "(proposal_id) with the same carrier, quote_id, order, batch and qty.", "write",
         (Param("proposal_id", "string"), Param("quote_id", "string"), Param("carrier", "string"),
          Param("order", "string"), Param("batch", "string"), Param("qty", "integer")), freight.book),
    Tool("shipments", "The shipments booked so far, with their arrival dates.", "read", (),
         _listed("shipment", "shipments")),
    Tool("send_customer_notice", "Send a customer the delivery date of one order (new_date YYYY-MM-DD) with a "
         "message.", "write",
         (Param("customer", "string"), Param("order", "string"), Param("new_date", "string"),
          Param("message", "string")), freight.notify),
    Tool("customer_notices", "The notices sent to customers so far.", "read", (), _listed("notice", "notices")),
    Tool("logistics_scenario", "Sets the carriers' scenario (a task's setup calls it; agents don't get it).",
         "write", (Param("name", "string"),), _scenario),
    Tool("supply_review", "The supply chain director reviews a proposed change (a benchmark's link calls it; "
         "agents don't get it).", "write",
         (Param("id", "string"), Param("action", "string"), Param("payload", "object", None),
          Param("summary", "string", None)), director.review),
]}
