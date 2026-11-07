import dataclasses
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from arena_part import PartError
from isaac_part.backends import BACKENDS
from isaac_part.config import DEFAULT_ISAAC
from isaac_part.mirror import Mirror
from isaac_part.part import IsaacSim
from isaac_part.state import FACTORY_EXCLUDED, FACTORY_SAVED

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]
SEED = 7  # the factory's own default seed: check_physics.py's numbers are for it
SAVE_BYTES_MAX = 100_000  # measured 39 kB at 20 s; after the trip its frozen 60 s window adds about 180 kB
ALL_KEYS = []  # inspect with no keys returns every key


def standin():
    return IsaacSim(BACKENDS["standin"])


def isaac():
    """The isaac backend with the host's GPU left out: tests never call nvidia-smi."""
    return IsaacSim(dataclasses.replace(BACKENDS["isaac"], host=lambda: {"gpu": None, "driver": None}))


def started(instance="i1", part=None, seed=SEED, clock_ms=0, **config):
    part = part or standin()
    part.start(instance, config, seed, clock_ms)
    return part


def docs(part, instance="i1"):
    return part.inspect({"instance": instance}, ALL_KEYS)["docs"]


def kinds(events, kind):
    return [e for e in events if e["kind"] == kind]


def save_dir(saves_root, save):
    return str(saves_root / "isaac-sim" / save)


def test_scenario_alarm_on_arena_clock():
    """test_scenario.py trips PR-A1 55 to 66 s after the start, after zone C and then D; check_physics.py has
    zone C near 33 s for seed 7 (measured here: 33.7 s of arena time, the scenario starting at arena 0)."""
    part = started()
    first = part.advance("i1", 30_000)
    assert [e["kind"] for e in first["events"][:2]] == ["plant", "scenario"] and first["changed"]
    assert not kinds(first["events"], "warning")
    zone_c = kinds(part.advance("i1", 36_000)["events"], "warning")
    assert [w["data"]["detail"]["zone"] for w in zone_c] == ["C"]
    w = zone_c[0]
    detail = w["data"]["detail"]
    assert detail["machine"] == "PR-A1" and w["data"]["source"] == "PR-A1" and detail["vibration_mm_s"] >= 4.5
    assert w["data"]["t"].startswith("2026-03-10T06:00:3")
    assert "wall" not in w and "wall" not in w["data"] and "wall" not in detail
    later = part.advance("i1", 66_000)["events"]
    assert [x["data"]["detail"]["zone"] for x in kinds(later, "warning")] == ["D"]
    alarm = kinds(later, "alarm")[0]
    assert alarm["data"]["detail"]["machine"] == "PR-A1" and alarm["data"]["detail"]["vibration_mm_s"] > 11.2
    assert alarm["data"]["seq"] > w["data"]["seq"]
    trip_s = part.instances["i1"].factory.presses["PR-A1"].tripped_at
    assert 55.0 <= trip_s <= 66.0
    assert docs(part)["machine/PR-A1"]["status"] == "tripped"
    assert docs(part)["incident/INC-20260310-01"]["machine"] == "PR-A1"


def test_add_shipment_puts_a_shipment_on_the_plant():
    part = started()
    args = {"part": "SP-SPN-800", "origin": "Dock", "carrier": "Lakeshore", "transit_hours": 2, "ref": "Q-1"}
    out = part.call_tool("i1", "add_shipment", args, 0)
    sid = out["result"]["id"]
    assert out["writes"] == [f"shipment/{sid}"]
    doc = docs(part)[f"shipment/{sid}"]
    assert (doc["part"], doc["ref"], doc["status"]) == ("SP-SPN-800", "Q-1", "in-transit")


def test_advance_split_does_not_matter():
    one, many = started("a"), started("b")
    one.advance("a", 5_000)
    for t in range(0, 5_001, 333):
        many.advance("b", t)
    many.advance("b", 5_000)
    assert docs(one, "a") == docs(many, "b")


def same_inputs(part, instance):
    """The same tool calls and advances on the arena clock (the part's inputs, no agent); every
    result and event, in order."""
    out = [part.call_tool(instance, "set_line_status", {"line": "LINE-B", "status": "running", "rate": 1.2}, 2_000),
           part.advance(instance, 4_000),
           part.call_tool(instance, "dispatch_technician",
                          {"proposal_id": "P-1", "technician": "T-1", "machine": "PR-C1"}, 4_500),
           part.call_tool(instance, "machine_telemetry", {"machine": "PR-C1", "minutes": 1}, 9_000),
           part.advance(instance, 12_000)]
    return out


def test_replay_equals():
    a = started("a")
    b = started("b", part=a)  # two instances in one process, as arenad runs them
    assert same_inputs(a, "a") == same_inputs(a, "b")
    assert docs(a, "a") == docs(a, "b")
    assert docs(a, "a")["technician/T-1"]["machine"] == "PR-C1"
    assert docs(a, "a")["line/LINE-B"]["boost"] == 1.2
    assert a.diff({"instance": "a"}, {"instance": "b"}) == {"differing": []} and b is a
    other = started("c", seed=11)
    assert same_inputs(other, "c") != same_inputs(a, "a")  # floor noise follows the seed


def test_approximate_restore_continues(saves_root):
    a = started()
    a.advance("i1", 20_000)
    saved = a.save("i1", "s1", save_dir(saves_root, "s1"))
    assert saved["exact"] is False and 0 < saved["bytes"] < SAVE_BYTES_MAX
    b = standin()
    b.start("b", {}, SEED, 20_000, {"save": "s1", "mode": "approximate"})  # a second Part in this process
    a.start("b", {}, SEED, 20_000, {"save": "s1", "mode": "approximate"})
    assert docs(b, "b") == docs(a, "b") == a.inspect({"save": "s1"}, ALL_KEYS)["docs"] == docs(a)
    a_events, b_events = a.advance("i1", 36_000)["events"], a.advance("b", 36_000)["events"]
    assert [w["data"]["detail"]["zone"] for w in kinds(b_events, "warning")] == ["C"]
    assert b_events == a_events == b.advance("b", 36_000)["events"]  # the stand-in continues exactly
    assert a.diff({"instance": "i1"}, {"instance": "b"}) == {"differing": []}
    assert "plant/clock" in a.diff({"save": "s1"}, {"instance": "b"})["differing"]
    with pytest.raises(PartError) as e:
        standin().start("c", {}, SEED, 20_000, {"save": "s1", "mode": "exact"})
    assert e.value.code == -32001 and "replay" in e.value.message
    with pytest.raises(PartError) as e:
        standin().start("c", {}, SEED, 0, {"save": "nope", "mode": "approximate"})
    assert e.value.code == -32002


def over_stdio(requests, saves_root):
    """The part's replies to `requests`, from a fresh process speaking arena-part/1 over stdio."""
    lines = "".join(json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}) + "\n"
                    for i, (m, p) in enumerate(requests, 1))
    r = subprocess.run([sys.executable, "-m", "isaac_part", "--backend", "standin"], input=lines, cwd=PACKAGE,
                       capture_output=True, text=True, env={**os.environ, "ARENA_SAVES": str(saves_root)}, timeout=120)
    replies = [json.loads(line) for line in r.stdout.splitlines()]
    assert len(replies) == len(requests), r.stderr[-2000:]
    return replies


def test_approximate_restore_in_a_fresh_process(saves_root):
    a = started()
    a.advance("i1", 20_000)
    a.save("i1", "s1", save_dir(saves_root, "s1"))
    original = a.advance("i1", 36_000)["events"]
    replies = over_stdio([
        ("start", {"instance": "b", "config": {}, "seed": SEED, "clock_ms": 20_000,
                   "from": {"save": "s1", "mode": "approximate"}}),
        ("diff", {"a": {"instance": "b"}, "b": {"save": "s1"}}),
        ("advance", {"instance": "b", "clock_ms": 36_000}),
    ], saves_root)
    assert [r.get("error") for r in replies] == [None, None, None]
    assert replies[1]["result"] == {"differing": []}
    assert replies[2]["result"]["events"] == original
    assert [w["data"]["detail"]["zone"] for w in kinds(original, "warning")] == ["C"]


def test_unreadable_or_mismatched_save_is_cant_restore(saves_root, tmp_path):
    a = started()
    a.advance("i1", 1_000)
    a.save("i1", "s1", save_dir(saves_root, "s1"))
    copy = tmp_path / "ridgeline.json"  # the same content elsewhere: the fingerprint accepts it, the save doesn't
    shutil.copy(DEFAULT_ISAAC.parent / "data" / "ridgeline.json", copy)
    for config, text in [({"plant_start": "2026-04-01T06:00:00"}, "plant_start"), ({"physics_hz": 720}, "physics_hz"),
                         ({"data_path": str(copy)}, "data_path")]:
        with pytest.raises(PartError) as e:
            standin().start("b", config, SEED, 1_000, {"save": "s1", "mode": "approximate"})
        assert e.value.code == -32001 and text in e.value.message
    other = IsaacSim(dataclasses.replace(BACKENDS["standin"], name="other"))
    with pytest.raises(PartError) as e:
        other.start("b", {}, SEED, 1_000, {"save": "s1", "mode": "approximate"})
    assert e.value.code == -32001 and "backend" in e.value.message
    (saves_root / "isaac-sim" / "s1" / "plant.pkl.gz").write_bytes(b"not a save")
    with pytest.raises(PartError) as e:
        standin().start("b", {}, SEED, 1_000, {"save": "s1", "mode": "approximate"})
    assert e.value.code == -32001 and "s1" in e.value.message


def test_state_whitelist_covers_factory_attributes():
    part = started()
    part.call_tool("i1", "dispatch_technician", {"proposal_id": "P", "technician": "T-1", "machine": "PR-A1"}, 1_000)
    part.advance("i1", 3_000)
    attrs = set(vars(part.instances["i1"].factory))
    assert not set(FACTORY_SAVED) & set(FACTORY_EXCLUDED)
    assert attrs == set(FACTORY_SAVED) | set(FACTORY_EXCLUDED), (
        f"classify {sorted(attrs - set(FACTORY_SAVED) - set(FACTORY_EXCLUDED))} in isaac_part/state.py")


def isaac_copy(tmp_path, change=None):
    """A temp copy of the factory's isaac folder and data, optionally with `change` appended to factory.py."""
    root = tmp_path / "factory"
    shutil.copytree(DEFAULT_ISAAC, root / "isaac", ignore=shutil.ignore_patterns("__pycache__", "out", "spark"))
    (root / "data").mkdir()
    shutil.copy(DEFAULT_ISAAC.parent / "data" / "ridgeline.json", root / "data" / "ridgeline.json")
    if change:
        with open(root / "isaac" / "factory.py", "a") as f:
            f.write(change)
    return root / "isaac"


def test_fingerprint_hashes_the_adapter_too(tmp_path, monkeypatch):
    import shutil
    from isaac_part import source
    fp = standin().describe()["fingerprint"]
    assert {"part.py", "plant.py", "source.py", "tools.py", "state.py"} <= set(fp["adapter_sha256"])
    assert fp["adapter"] == 1
    copy = tmp_path / "isaac_part"
    shutil.copytree(source.ADAPTER_DIR, copy, ignore=shutil.ignore_patterns("__pycache__"))
    monkeypatch.setattr(source, "ADAPTER_DIR", copy)
    assert source.adapter_hashes() == fp["adapter_sha256"]
    with open(copy / "tools.py", "a") as f:
        f.write("\n# changed\n")
    assert source.adapter_hashes()["tools.py"] != fp["adapter_sha256"]["tools.py"]


def test_fingerprint_recorded_and_changed_source_refused(tmp_path):
    fp = standin().describe()["fingerprint"]
    common = {"factory.py", "press_model.py", "signals.py", "paths.py", "schedule.py", "layout.py", "ridgeline.json"}
    assert set(fp["sha256"]) == common | {"tests/fake_sim.py", "tests/press_dynamics.py"}
    assert all(len(h) == 64 for h in fp["sha256"].values())
    assert fp["backend"] == "standin" and fp["physics_hz"] == 360 and fp["adapter"] == 1
    isaac_fp = isaac().describe()["fingerprint"]
    assert set(isaac_fp["sha256"]) == common | {"sim_io.py", "ridgeline_scene.py"}
    assert isaac_fp["backend"] == "isaac" and {"gpu", "driver"} <= set(isaac_fp)
    started(isaac_path=str(isaac_copy(tmp_path / "same")))  # identical content is accepted
    changed = isaac_copy(tmp_path / "changed", "\n# changed\n")
    with pytest.raises(PartError) as e:
        started(isaac_path=str(changed))
    assert e.value.code == -32003 and "factory.py" in e.value.message and str(changed) in e.value.message


def test_bad_config_is_invalid(tmp_path):
    gone = tmp_path / "no-such-isaac"
    for config, text in [({"isaac_path": str(gone)}, str(gone)), ({"plant_start": "soon"}, "plant_start"),
                         ({"physics_hz": 0}, "physics_hz"), ({"scenario": "yes"}, "scenario")]:
        with pytest.raises(PartError) as e:
            started(**config)
        assert e.value.code == -32602 and text in e.value.message


def test_isaac_backend_missing_is_clear():
    if importlib.util.find_spec("isaacsim") is not None:
        pytest.skip("isaacsim is importable here")
    part = isaac()
    assert part.describe()["restore"] == {"exact": "replay", "approximate": True}
    with pytest.raises(PartError) as e:
        part.start("i1", {}, SEED, 0)
    assert e.value.code == -32603
    assert e.value.message == ("Isaac Sim is not importable here: run this integration with Isaac Sim's Python on "
                               "the DGX Spark (see README)")


REFUSALS = [  # (tool, args, text in the error): each changes nothing
    ("machine_telemetry", {"machine": "PR-Z9"}, "unknown machine PR-Z9"),
    ("dispatch_technician", {"proposal_id": "P", "technician": "T-1", "machine": "PR-A1"}, "busy"),
    ("dispatch_technician", {"proposal_id": "P", "technician": "T-2", "machine": "PR-A1", "task": "diagnose"},
     "task is 'inspect' or 'repair'"),
    ("set_line_status", {"line": "LINE-A", "status": "sideways"}, "status is running, stopped or down"),
    ("set_line_status", {"line": "LINE-Q", "status": "stopped"}, "unknown line LINE-Q"),
    ("predict_move", {"order": "A-1", "to_line": "LINE-F"}, "without ERPNext in the arena"),
    ("vibration_spectrum", {"machine": "PR-A1", "seconds": "lots"}, "seconds"),
]


def test_tools_keys_and_refusals():
    part = started()
    names = {t["name"] for t in part.describe()["tools"]}
    assert names == {"plant_state", "machine_telemetry", "vibration_spectrum", "recent_events", "wait_for_event",
                     "predict_move", "dispatch_technician", "set_line_status", "add_shipment"}
    sent = part.call_tool("i1", "dispatch_technician",
                          {"proposal_id": "P", "technician": "T-1", "machine": "PR-A1", "task": "inspect"}, 1_000)
    assert sent["result"]["status"] == "walking" and sent["writes"] == ["technician/T-1", "machine/PR-A1"]
    state = part.call_tool("i1", "plant_state", {}, 2_000)
    assert "machine/PR-A1" in state["reads"] and state["writes"] == []
    assert state["result"]["machines"][0]["id"] == "PR-A1"
    assert state["result"]["clock"].startswith("2026-03-10T06:00:02")
    wait = part.call_tool("i1", "wait_for_event", {"kinds": "technician", "timeout_s": 600}, 2_000)
    assert wait["result"]["events"][0]["kind"] == "technician"
    assert wait["result"]["timed_out"] is False
    before = docs(part)
    for name, args, text in REFUSALS:
        out = part.call_tool("i1", name, args, 2_000)
        assert text in out["error"] and out["reads"] == [] and out["writes"] == [] and "result" not in out, out
    assert docs(part) == before
    with pytest.raises(PartError) as e:
        part.call_tool("i1", "no_such_tool", {}, 2_000)
    assert e.value.code == -32602


class StubPlant(BaseHTTPRequestHandler):
    """The factory API's GET /state and GET /events, from `server.plant`; every request is recorded."""

    def log_message(self, *args):
        pass

    def do_GET(self):
        plant = self.server.plant
        plant["requests"].append(("GET", self.path))
        since = int(re.search(r"since=(\d+)", self.path).group(1)) if "since=" in self.path else 0
        events = plant["events"]
        body = plant["state"] if self.path == "/state" else (
            [e for e in events if e["seq"] > since] if isinstance(events, list) else events)
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        self.server.plant["requests"].append(("POST", self.path))
        self.send_response(405)
        self.end_headers()


def plant_event(seq, kind, text):
    return {"seq": seq, "t": "2026-03-10T06:00:00", "wall": 1.0, "kind": kind, "source": "PR-A1", "text": text,
            "data": {"machine": "PR-A1"}}


@pytest.fixture
def stub_plant():
    state = {"source": "isaac-sim", "t": 1.0, "clock": "2026-03-10T06:00:01", "speed": 1.0, "scenario": "running",
             "log_seq": 1, "machines": [{"id": "PR-A1", "status": "running"}], "lines": [{"id": "LINE-A"}],
             "sim": {"isaac_version": "6.2.0"}}
    server = ThreadingHTTPServer(("127.0.0.1", 0), StubPlant)
    server.plant = {"state": state, "events": [plant_event(1, "plant", "reset")], "requests": []}
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield server
    server.shutdown()
    server.server_close()


def test_mirror_mode_has_no_tools_and_relays_events(stub_plant, saves_root):
    url = f"http://127.0.0.1:{stub_plant.server_address[1]}"
    m = Mirror()
    d = m.describe()
    assert d["tools"] == [] and d["restore"] == {"exact": "none", "approximate": False} and d["uses_clock"]
    m.start("m1", {"plant_url": url}, 1, 0)
    assert m.advance("m1", 1_000) == {"changed": False, "events": []}  # events before the start are history
    stub_plant.plant["events"].append(plant_event(2, "warning", "PR-A1 vibration above zone C"))
    out = m.advance("m1", 2_000)
    assert out["changed"] and [e["text"] for e in out["events"]] == ["PR-A1 vibration above zone C"]
    assert out["events"][0]["data"] == {"source": "PR-A1", "seq": 2, "t": "2026-03-10T06:00:00",
                                     "detail": {"machine": "PR-A1"}}
    assert m.advance("m1", 3_000)["events"] == []
    assert m.inspect({"instance": "m1"}, ["machine/PR-A1", "robot/AMR-9"])["docs"] == {
        "machine/PR-A1": {"id": "PR-A1", "status": "running"}, "robot/AMR-9": None}
    assert m.save("m1", "s1", str(saves_root / "isaac-sim-mirror" / "s1"))["exact"] is False
    assert m.inspect({"save": "s1"}, ["plant/sim"])["docs"]["plant/sim"] == {"isaac_version": "6.2.0"}
    with pytest.raises(PartError) as e:
        m.call_tool("m1", "plant_state", {}, 3_000)
    assert e.value.code == -32602
    with pytest.raises(PartError) as e:
        m.start("m2", {"plant_url": url}, 1, 0, {"save": "s1", "mode": "approximate"})
    assert e.value.code == -32001
    assert {method for method, _ in stub_plant.plant["requests"]} == {"GET"}
    assert ("GET", "/events?since=1&limit=500") in stub_plant.plant["requests"]
    stub_plant.plant["events"] = {"error": "factory restarting"}
    with pytest.raises(PartError) as e:
        m.advance("m1", 4_000)
    assert e.value.code == -32603 and "factory restarting" in e.value.message


def checker():
    for profile in ("debug", "release"):
        p = ARENA / "core" / "target" / profile / "arena-part-check"
        if p.is_file():
            return p
    return None


def test_conformance(tmp_path):
    """arena-part-check on a copy of the manifest that runs the stand-in backend."""
    binary = checker()
    if binary is None or shutil.which("uv") is None:
        pytest.skip("arena-part-check is not built (cargo build -p parts --bin arena-part-check) or uv missing")
    manifest = (PACKAGE / "arena-package.toml").read_text()
    run = f'run = {{ command = ["uv", "run", "--directory", "{PACKAGE}", "python", "-m", "isaac_part", ' \
          f'"--backend", "standin"] }}'
    (tmp_path / "pkg").mkdir()
    (tmp_path / "pkg" / "arena-package.toml").write_text(re.sub(r"^run = .*$", run, manifest, flags=re.M))
    saves = tmp_path / "check-saves"
    saves.mkdir()
    env = {**os.environ, "ARENA_SAVES": str(saves)}
    r = subprocess.run([str(binary), str(tmp_path / "pkg")], capture_output=True, text=True, env=env, timeout=300)
    assert r.returncode == 0, r.stdout + r.stderr
    report = json.loads(r.stdout)
    restore = next(s for s in report["steps"] if s["name"] == "restore")["detail"]
    assert "approximate restore matches" in restore
    assert next(s for s in report["steps"] if s["name"] == "replay")["ok"]


def test_gpu_fingerprint_reads_proc_not_nvidia_smi(tmp_path, monkeypatch):
    from isaac_part import gpu

    def no_smi():
        raise AssertionError("nvidia-smi must not run when /proc has the answer")
    proc = tmp_path / "nvidia"
    (proc / "gpus" / "0000:01:00.0").mkdir(parents=True)
    (proc / "version").write_text("NVRM version: NVIDIA UNIX Open Kernel Module for aarch64  580.159.03  "
                                  "Release Build  (dvs-builder@U16)  Thu Sep 4 2025\nGCC version:  gcc 13.3.0\n")
    (proc / "gpus" / "0000:01:00.0" / "information").write_text("Model: \t\t NVIDIA GB10\nIRQ:   \t\t 196\n")
    assert gpu.gpu_host(proc, no_smi) == {"gpu": "NVIDIA GB10", "driver": "580.159.03"}
    (proc / "version").write_text("NVRM version: NVIDIA UNIX x86_64 Kernel Module  595.99.02  Wed Aug 26 2026\n")
    assert gpu.gpu_host(proc, no_smi)["driver"] == "595.99.02"
    monkeypatch.setattr(gpu.shutil, "which", lambda _name: None)
    assert gpu.gpu_host(tmp_path / "no-driver", no_smi) == {"gpu": None, "driver": None}
    with pytest.raises(PartError) as e:  # a driver without /proc answers asks nvidia-smi; a timeout is an error
        gpu.gpu_host(proc / "gpus", lambda: gpu.from_smi(["sleep", "5"], timeout=0.1))
    assert e.value.code == -32603 and "did not answer" in e.value.message


def test_isaac_stop_detaches_the_physics_callback():
    from isaac_part import isaac_backend
    isaac_backend._APP["step"] = lambda dt: pytest.fail("a stopped plant was stepped")
    isaac_backend.stop(None)  # no SimulationApp in this process, so no timeline to stop
    isaac_backend._APP["step"](1 / 360)
    assert "app" not in isaac_backend._APP


def test_diff_tolerance_is_only_for_physics_readings():
    from isaac_part.views import same
    assert same({"sensors": {"vibration_mm_s": 4.51}, "pose": {"x": 1.0}}, {"sensors": {"vibration_mm_s": 4.53},
                                                                          "pose": {"x": 1.02}})
    assert not same({"boost": 1.2}, {"boost": 1.21}) and not same({"t": 20.0}, {"t": 20.01})
    assert not same({"sensors": {"zone": "B"}}, {"sensors": {"zone": "C"}})


def test_bad_recording_config_is_invalid():
    for config, text in [({"record": ""}, "record"), ({"record": "v.mp4", "camera": 5}, "camera"),
                         ({"record": "v.mp4", "record_fps": 0}, "record_fps")]:
        with pytest.raises(PartError) as e:
            started(**config)
        assert e.value.code == -32602 and text in e.value.message
    with pytest.raises(PartError) as e:
        started(record="v.mp4")
    assert e.value.code == -32602 and "record needs the isaac backend" in e.value.message
    with pytest.raises(PartError) as e:  # checked before Isaac Sim is needed
        started(part=isaac(), record="v.mp4", camera="nope")
    assert e.value.code == -32602 and "camera must be one of" in e.value.message and "'nope'" in e.value.message


class FakeVideo:
    """Stands in for isaac_video.Video: notes the plant step of each frame."""
    made = []

    def __init__(self, cfg, _io, physics_hz, path):
        from isaac_part.recording import frame_every
        self.every, self.path, self.at, self.closed = frame_every(physics_hz, cfg.record_fps), path, [], False
        FakeVideo.made.append(self)

    def frame(self, plant):
        self.at.append(plant.steps)

    def close(self):
        self.closed = True


def test_recording_frames_fall_on_arena_time_and_change_nothing(saves_root):
    FakeVideo.made.clear()
    part = started(part=IsaacSim(dataclasses.replace(BACKENDS["standin"], video=FakeVideo)), record="run.mp4",
                   camera="auto", record_fps=30)
    plain = started("i2")
    for clock_ms in (500, 1_234, 3_000):
        assert part.advance("i1", clock_ms) == plain.advance("i2", clock_ms)
    video = FakeVideo.made[0]
    assert video.path == saves_root / "isaac-sim" / "run-i1.mp4"
    assert video.every == 12 and video.at == list(range(12, 1081, 12))  # 3 s at 360 Hz, 30 frames a second
    part.stop("i1")
    assert video.closed


def test_recording_paths_and_frame_spacing(tmp_path):
    from isaac_part.recording import frame_every, next_frame, path_for
    assert path_for("out/run.mp4", tmp_path, "b1") == tmp_path / "out" / "run-b1.mp4"
    assert path_for("/v/run", tmp_path, "b1") == Path("/v/run-b1.mp4")
    assert (frame_every(360, 30), frame_every(360, 1000), next_frame(0, 12), next_frame(12, 12)) == (12, 1, 12, 24)


def test_save_goes_to_the_save_folder_by_convention(saves_root, tmp_path):
    """arenad's `dir` is its own path; the program, which may run elsewhere, keeps saves under its ARENA_SAVES."""
    part = started()
    part.save("i1", "s1", str(tmp_path / "arenad-side" / "s1"))
    assert (saves_root / "isaac-sim" / "s1" / "plant.pkl.gz").exists()
    assert not (tmp_path / "arenad-side").exists()


def test_steps_that_never_reach_the_plant_are_an_error():
    """A backend whose physics callback has stopped must not leave the plant silently standing still."""
    part = started(part=IsaacSim(dataclasses.replace(BACKENDS["standin"], run=lambda *_args: None)))
    with pytest.raises(PartError) as e:
        part.advance("i1", 1_000)
    assert e.value.code == -32603 and "physics callback is not running" in e.value.message


def test_only_the_newest_physics_callback_reaches_a_plant():
    from types import SimpleNamespace
    from isaac_part import isaac_backend
    callbacks, stepped = [], []
    sim_io = SimpleNamespace(on_physics_step=callbacks.append)
    isaac_backend._APP["step"] = stepped.append
    isaac_backend._register_callback(sim_io)
    isaac_backend._register_callback(sim_io)  # a second plant's stage
    callbacks[0](1.0)
    callbacks[1](2.0)
    assert stepped == [2.0]
