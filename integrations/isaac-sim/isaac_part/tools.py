"""The tool table (names and arguments as the factory's ridgeline_mcp/floor.py) and one dispatcher. Each tool calls
the Factory method its api.py route calls, directly."""
from dataclasses import dataclass
from typing import Callable

from arena_part import errors
from arena_part.params import Param, Refusal, input_schema, normalize, refused

from . import views

NO_ERP = ("{tool} needs the plant's Work Orders, which come from ERPNext; the plant runs without ERPNext in the "
          "arena")
STATE_KEYS = ("clock", "speed", "scenario", "lines", "machines", "orders", "technicians", "operators", "shipments",
              "robots", "sim")  # floor.py's plant_state


@dataclass(frozen=True)
class Tool:
    name: str
    description: str
    effect: str
    params: tuple
    act: Callable  # (factory, args) -> result
    keys: Callable  # (args, result) -> (reads, writes)
    checks: tuple = ()  # each (tool, factory, args) raises Refusal before anything changes

    def schema(self) -> dict:
        return {"name": self.name, "description": self.description, "effect": self.effect,
                "inputSchema": input_schema(self.params)}


def _known(field, collection, label):
    """A check: args[field] names something in the factory's `collection`."""
    def check(_tool, f, args):
        if args[field] not in getattr(f, collection):
            raise Refusal(f"unknown {label} {args[field]}")
    return check


def _needs_erp(tool, f, _args):
    if f.erp is None:
        raise Refusal(NO_ERP.format(tool=tool.name))


def _task(_tool, _f, args):
    if args["task"] not in ("inspect", "repair"):
        raise Refusal("task is 'inspect' or 'repair'")


def _key(kind, field):
    return lambda args: f"{kind}/{args[field]}"


def _reads(*makers):
    return lambda a, _r: ([m(a) for m in makers], [])


def _writes(*makers):
    return lambda a, _r: ([], [m(a) for m in makers])


def _no_keys(_a, _r):
    return [], []  # the event log is not keyed state


def _state_reads(_a, result):
    return sorted(views.docs_of(result)), []


def _plant_state(f, _a):
    s = f.state()
    return {k: s[k] for k in STATE_KEYS if k in s}


def _telemetry(f, a):
    rows = f.telemetry(a["machine"], float(a["minutes"]))
    return {"machine": a["machine"], "points": len(rows), "hz": 1, "readings": rows}


def _wait(f, a):
    """Returns at once: inside call_tool the arena clock stands still, so waiting would wait forever."""
    hits = f.events_since(a["since"], 500, {k for k in a["kinds"].split(",") if k}, a["machine"])
    return {"events": hits, "timed_out": not hits, "seq": f.seq, "source": "isaac-sim"}


MACHINE = _known("machine", "presses", "machine")
TOOLS = {t.name: t for t in [
    Tool("plant_state", "Every line (status, measured rate per hour, current order), machine (status running, "
         "degraded, tripped, locked-out, maintenance or stopped; sensors: vibration_mm_s and its ISO 10816 zone, "
         "flywheel_rpm and strokes_per_min measured, spindle_temp_c modelled; strokes counted; lockout), order, "
         "technician and operator, shipment and robot. `clock` is the plant clock; `sim` says what ran it.", "read",
         (), _plant_state, _state_reads),
    Tool("machine_telemetry", "Sensor history for one machine at 1 Hz (vibration mm/s RMS, flywheel rpm, "
         "strokes/min, spindle temperature C, which is modelled).", "read",
         (Param("machine", "string"), Param("minutes", "integer", 30)), _telemetry,
         _reads(_key("machine", "machine")), (MACHINE,)),
    Tool("vibration_spectrum", "The vibration spectrum (FFT of the IMU on the press frame, velocity mm/s RMS per "
         "bin) over the last `seconds` of running; with before_trip, the seconds just before the protective trip. "
         "Returns fs, the shaft rpm, the largest peaks (hz, mm_s, order = multiple of shaft speed) and the bins.",
         "read", (Param("machine", "string"), Param("seconds", "integer", 20), Param("before_trip", "boolean", True)),
         lambda f, a: f.spectrum(a["machine"], float(a["seconds"]), a["before_trip"]),
         _reads(_key("machine", "machine")), (MACHINE,)),
    Tool("recent_events", "The plant's event log after sequence number `since`: alarms, warnings, operators, "
         "technicians, lockouts, shipments, deliveries, repairs, production.", "read",
         (Param("since", "integer", 0), Param("limit", "integer", 60)),
         lambda f, a: f.events_since(a["since"], a["limit"]), _no_keys),
    Tool("wait_for_event", "Plant events after sequence `since`, of the comma-separated `kinds` (alarm, warning, "
         "operator, technician, lockout, shipment, delivery, repair, production). Returns at once, timed_out when "
         "there are none yet; timeout_s is accepted and ignored, since the arena clock moves between calls.",
         "read", (Param("kinds", "string", ""), Param("since", "integer", 0), Param("machine", "string", None),
                  Param("timeout_s", "integer", 240)), _wait, _no_keys),
    Tool("predict_move", "What-if, changes nothing: the ETA of this order on to_line, whether it would be late, "
         "and the orders it would displace there. Orders are ERPNext Work Orders.", "read",
         (Param("order", "string"), Param("to_line", "string")),
         lambda f, a: f.whatif_move(a["order"], a["to_line"]),
         _reads(_key("order", "order"), _key("line", "to_line")), (_needs_erp,)),
    Tool("dispatch_technician", "Send a technician to a machine. task is 'inspect' (walk there, lock the machine "
         "out, open the guard and inspect) or 'repair' (install the delivered part, brake test, run-in). "
         "proposal_id is accepted and not verified here.", "write",
         (Param("proposal_id", "string"), Param("technician", "string"), Param("machine", "string"),
          Param("task", "string", "inspect")),
         lambda f, a: f.dispatch_technician(a["technician"], a["machine"], a["task"]),
         _writes(_key("technician", "technician"), _key("machine", "machine")),
         (_task, _known("technician", "technicians", "technician"), MACHINE)),
    Tool("set_line_status", "Stop a line ('stopped') or run it ('running'); rate is a multiplier of its normal rate "
         "(0.5 to 1.25).", "write", (Param("line", "string"), Param("status", "string"), Param("rate", "number", None)),
         lambda f, a: f.set_line(a["line"], a["status"], a["rate"]), _writes(_key("line", "line")),
         (_known("line", "lines_meta", "line"),)),
]}


def _act(tool, f, args):
    for check in tool.checks:
        check(tool, f, args)
    return views.plain(tool.act(f, args))


def _message(e: Exception) -> str:
    return f"unknown id {e}" if isinstance(e, KeyError) else str(e)


def run_tool(plant, name: str, args) -> dict:
    """Runs one tool on a plant: a call_tool result. A refusal (bad argument, unknown id, or the Factory's own
    ValueError/KeyError, as api.py answers 409/404) is a tool-level `error` with no keys; an unknown tool name is
    a protocol-level `invalid`."""
    tool = TOOLS.get(name)
    if tool is None:
        raise errors.invalid(f"unknown tool {name}")
    try:
        args = normalize(tool.name, tool.params, args)
        result = plant.act(lambda f: _act(tool, f, args))
    except (Refusal, ValueError, KeyError) as e:
        return refused(_message(e))
    reads, writes = tool.keys(args, result)
    return {"result": result, "reads": reads, "writes": writes}
