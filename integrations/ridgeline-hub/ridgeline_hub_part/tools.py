"""The tool table (names and arguments as the factory's ridgeline_mcp/market.py) and one dispatcher."""
from dataclasses import dataclass
from typing import Callable

from arena_part import errors

class Refusal(Exception):
    """A tool refused the call (unknown id, mismatch, bad argument): answered as a tool-level `error`."""


REQUIRED = object()
TYPES = {"string": (str,), "integer": (int,), "number": (int, float), "array": (list,), "object": (dict,)}


@dataclass(frozen=True)
class Param:
    name: str
    type: str
    default: object = REQUIRED

    def check(self, value):
        ok = isinstance(value, TYPES[self.type]) and not isinstance(value, bool)
        if not ok and not (value is None and self.default is None):
            raise Refusal(f"{self.name} must be of type {self.type}")


@dataclass(frozen=True)
class Tool:
    name: str
    description: str
    effect: str
    params: tuple
    method: str  # the Market method that does it
    args_of: Callable  # normalized args -> positional args of the method
    keys: Callable  # (args, result) -> (reads, writes)
    validate: Callable | None = None  # (world, args) -> raises invalid before anything changes
    record: Callable | None = None  # (world, args, result): store the accepted, unchecked proposal_id

    def schema(self) -> dict:
        props = {p.name: {"type": p.type} for p in self.params}
        required = [p.name for p in self.params if p.default is REQUIRED]
        return {"name": self.name, "description": self.description, "effect": self.effect,
                "inputSchema": {"type": "object", "properties": props, "required": required}}

    def normalize(self, args) -> dict:
        if not isinstance(args, dict):
            raise Refusal("args must be an object")
        out = {}
        for p in self.params:
            if p.name in args:
                p.check(args[p.name])
                out[p.name] = args[p.name]
            elif p.default is REQUIRED:
                raise Refusal(f"{self.name}: missing {p.name}")
            else:
                out[p.name] = p.default
        return out


def _pick(*names):
    return lambda a: [a[n] for n in names]


def _key(kind, field, source):
    """The key `<kind>/<id>`, the id read from the args or from the result."""
    return lambda args, result: f"{kind}/{(args if source == 'args' else result)[field]}"


def _reads(*makers):
    return lambda a, r: ([m(a, r) for m in makers], [])


def _writes(*makers):
    return lambda a, r: ([], [m(a, r) for m in makers])


def _freight_id(quote_id: str) -> str:
    return quote_id.rsplit("-", 1)[0]


def _booking_keys(args, result):
    fid = f"freight/{_freight_id(args['quote_id'])}"
    if result.get("already_booked"):
        return [fid], []
    return [], [fid, f"shipment/{result['shipment']['id']}"]


def _reply_keys(_args, result):
    ids = [r["notice"] for r in result["replies"]] + result["waiting"]
    return [f"notice/{n}" for n in ids], []


def _exists(field, collection, label):
    """A validator: args[field] names something in the market's `collection`."""
    def check(world, args):
        if args[field] not in getattr(world.market, collection):
            raise Refusal(f"unknown {label} {args[field]}")
    return check


def _all(*checks):
    def check(world, args):
        for c in checks:
            c(world, args)
    return check


def _check_supplier(world, args):
    if args["supplier"] not in world.module.QUOTES:
        raise Refusal(f"unknown supplier {args['supplier']}")


def same(a, b) -> bool:
    """Numbers within 0.005, strings compared stripped and casefolded (as the factory's approvals)."""
    if isinstance(a, (int, float)) or isinstance(b, (int, float)):
        try:
            return abs(float(a) - float(b)) < 0.005
        except (TypeError, ValueError):
            return False
    return str(a).strip().casefold() == str(b).strip().casefold()


def _check_booking(world, args):
    fid, _, n = args["quote_id"].rpartition("-")
    if fid not in world.market.freight:
        raise Refusal(f"unknown freight request {fid or args['quote_id']}")
    if not n.isdigit() or not 1 <= int(n) <= len(world.module.FREIGHT):
        raise Refusal(f"unknown quote {args['quote_id']}")
    quote = world.module.FREIGHT[int(n) - 1]
    for field, truth in (("carrier", quote["carrier"]), ("amount", quote["amount"])):
        if not same(args[field], truth):
            raise Refusal(f"{field} {args[field]!r} does not match quote {args['quote_id']} ({truth!r})")


def _proposal(collection, id_of):
    def record(world, args, result):
        getattr(world.market, collection)[id_of(args, result)]["proposal_id"] = args["proposal_id"]
    return record


def _claim_body(a):
    return [{k: a[k] for k in ("policy", "machine", "amount", "downtime_hours", "evidence")}]


def _wait_s():
    return Param("wait_s", "integer", 0)  # accepted and ignored: agents poll, the arena clock runs on


TOOLS = {t.name: t for t in [
    Tool("request_quotes", "Send a request for quotes for a part to the suppliers that stock it. Quotes "
         "arrive over about a minute (arena time).", "write",
         (Param("part", "string"), Param("qty", "integer", 1), Param("need_by", "string", None)),
         "rfq", _pick("part", "qty", "need_by"), _writes(_key("rfq", "rfq", "result"))),
    Tool("get_quotes", "The quotes received for a request and who has not answered yet. Returns at once; "
         "check back (wait_s is accepted and ignored).", "read",
         (Param("rfq", "string"), _wait_s()), "quotes", _pick("rfq"), _reads(_key("rfq", "rfq", "args")),
         validate=_exists("rfq", "rfqs", "rfq")),
    Tool("award_order", "Tell the winning supplier their quote is accepted under purchase order `po`.", "write",
         (Param("rfq", "string"), Param("supplier", "string"), Param("po", "string")),
         "award", _pick("rfq", "supplier", "po"), _writes(_key("rfq", "rfq", "args")), validate=_all(_exists("rfq", "rfqs", "rfq"), _check_supplier)),
    Tool("freight_quotes", "Ask carriers to quote moving a load from `origin` to the plant receiving dock.",
         "write", (Param("origin", "string"), Param("weight_kg", "number"), Param("need_by", "string", None)),
         "freight_quotes", _pick("origin", "weight_kg", "need_by"), _writes(_key("freight", "request", "result"))),
    Tool("get_freight_quotes", "Carrier quotes: quote_id, carrier, service, amount, transit_hours, arrives. "
         "Returns at once (wait_s is accepted and ignored).", "read",
         (Param("request", "string"), _wait_s()), "freight_status", _pick("request"),
         _reads(_key("freight", "request", "args")),
         validate=_exists("request", "freight", "freight request")),
    Tool("book_freight", "Book a carrier's quote. proposal_id is recorded, not verified here. The shipment is "
         "recorded in this part only.", "write",
         (Param("proposal_id", "string"), Param("quote_id", "string"), Param("carrier", "string"),
          Param("amount", "number"), Param("part", "string", "SP-SPN-800")),
         "book_freight", _pick("quote_id", "part"), _booking_keys, validate=_check_booking,
         record=_proposal("freight", lambda a, r: _freight_id(a["quote_id"]))),
    Tool("submit_claim", "Send an equipment-breakdown claim to the insurer's intake agent. Returns its "
         "follow-up questions.", "write",
         (Param("policy", "string"), Param("machine", "string"), Param("amount", "number"),
          Param("downtime_hours", "number"), Param("evidence", "object")),
         "claim", _claim_body, _writes(_key("claim", "claim", "result"))),
    Tool("answer_claim", "Answer the insurer's follow-up questions, in order.", "write",
         (Param("claim", "string"), Param("answers", "array")), "answer_claim", _pick("claim", "answers"),
         _writes(_key("claim", "claim", "args")), validate=_exists("claim", "claims", "claim")),
    Tool("notify_customer", "Send a customer's agent the new delivery date for one order. proposal_id is "
         "recorded, not verified here.", "write",
         (Param("proposal_id", "string"), Param("customer", "string"), Param("order", "string"),
          Param("new_date", "string"), Param("message", "string")),
         "notify", _pick("customer", "order", "new_date", "message"), _writes(_key("notice", "notice", "result")),
         record=_proposal("notices", lambda a, r: r["notice"])),
    Tool("customer_replies", "Customers' replies to the notices sent so far. Returns at once (wait_s is "
         "accepted and ignored).", "read", (_wait_s(),), "replies", lambda a: [], _reply_keys),
]}


def run_tool(world, name: str, args) -> dict:
    """Runs one tool on a world: a call_tool result. A refusal is a tool-level `error` with no keys; an
    unknown tool name is a protocol-level `invalid`."""
    tool = TOOLS.get(name)
    if tool is None:
        raise errors.invalid(f"unknown tool {name}")
    try:
        args = tool.normalize(args)
        if tool.validate:
            tool.validate(world, args)
        result = getattr(world.market, tool.method)(*tool.args_of(args))
    except (Refusal, ValueError) as e:
        return {"error": str(e), "reads": [], "writes": []}
    if tool.record:
        tool.record(world, args, result)
    reads, writes = tool.keys(args, result)
    return {"result": result, "reads": reads, "writes": writes}
