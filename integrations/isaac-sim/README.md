# isaac-sim

Puts the Ridgeline factory plant in the world, with Isaac Sim as the physics (the `isaac` backend) or a stand-in
double (the `standin` backend). Restore: exact by replay from the seed and inputs; approximate from a save
(a pickle of the factory and simulation state). `uses_clock`: true. Verified on the stand-in only, by its pytest
suite and the `arena-part-check` conformance check; **the `isaac` backend has never run on Isaac Sim** (no Isaac
Sim image or 16 GB GPU here). Its package `isaac-sim-mirror` (see `integrations/isaac-sim-mirror/README.md`)
only follows a live plant.

The Ridgeline factory as an `arena-part/1` integration, on the arena clock. In the factory demo
(`Sculptured-Examples/factory`, branch `isaac-plant`), Isaac Sim 6.2 **is** the plant. This integration wraps
that demo's `isaac/factory.py` class `Factory` without editing it. Factory is plain Python and reaches the
simulation only through a `PlantIO`.

**The `isaac` backend has not run on Isaac Sim.** The machine it was written on has no Isaac Sim image and no
16 GB GPU. Everything below that says *verified* ran on the `standin` backend: the factory's own offline double,
`isaac/tests/fake_sim.FakeIO`, which is backed by `isaac/tests/press_dynamics.py`.

## Backends

The backend is a process argument, `--backend isaac|standin`, because `describe` has no config and the backend
changes the fingerprint.

| Backend | PlantIO | Used by |
|---|---|---|
| `isaac` | `sim_io.IsaacIO` in a headless `SimulationApp`, set up as `run.py` does it: scene build, `IsaacIO`, `setup_physics`, play, `start()`. PhysX steps run with `SimulationManager.step`, and `Factory.physics_step` runs in the post-step callback. | `arena-package.toml` (`needs.gpu_vram_gb = 16`) |
| `standin` | `fake_sim.FakeIO`. Each step is `io.step(dt)` and then `Factory.physics_step(dt)`, the order `test_scenario.py` and the app use. | the tests (a test manifest written to `tmp_path`) |

- If `isaacsim` can't be imported, `describe` still answers and `start` fails with -32603: "Isaac Sim is not
  importable here: run this integration with Isaac Sim's Python on the DGX Spark (see README)".
- Isaac Sim has one stage per process, so the `isaac` backend holds one live plant per process. A second `start`
  is refused with -32603 until the first instance stops. So two branches running at once on one isaac-sim
  program fail clearly at the second start. `arena-part-check` needs three live instances, so it can't pass on
  `isaac` as written.
- Stopping an `isaac` instance points the physics callback at a no-op and stops the timeline. A new plant starts
  the same way, then builds a new stage, so a later plant never steps an old one. arenad's usual base, then
  branch sequence is one plant after another, which this allows.

## Config

| key | default |
|---|---|
| `isaac_path` | `Sculptured-Examples/factory/isaac`, next to the arena checkout |
| `data_path` | `<isaac_path>/../data/ridgeline.json`, passed to `press_model.load_data` |
| `plant_start` | `2026-03-10T06:00:00`, the plant clock at arena time 0 |
| `physics_hz` | the factory's `press_model.PHYSICS_HZ` (360) |
| `scenario` | `true`: `start` begins the PR-A1 wear scenario at once |

- The factory folder is read only. Its modules are imported through `sys.path` with bytecode writing off, so
  no `__pycache__` is written there.
- A missing file makes `start` fail with -32602 naming the path. Modules are process-global, so one process
  serves one `isaac_path`.
- `physics_hz` is a knob, not a fix for speed. The factory's signal chain assumes `press_model.PHYSICS_HZ`
  (`VibrationChannel`'s sample rate, telemetry every `PHYSICS_HZ` steps), so any other rate changes the
  readings.

## Fingerprint

`describe` declares:
- `sha256` of the files at the default paths. These are `factory.py`, `press_model.py`, `signals.py`,
  `paths.py`, `schedule.py`, `layout.py` and `ridgeline.json`, plus `tests/fake_sim.py` and
  `tests/press_dynamics.py` for `standin`, or `sim_io.py` and `ridgeline_scene.py` for `isaac`;
- `adapter_sha256`: the sha256 of this adapter's own source files (`isaac_part/*.py`), since how it drives
  the factory is behaviour too;
- `backend`, `adapter` and the default `physics_hz`;
- for `isaac`, `{gpu, driver}`. These are read from `/proc/driver/nvidia/version` and
  `/proc/driver/nvidia/gpus/*/information`, which don't query the GPU. Only when those files are absent does it
  run `nvidia-smi --query-gpu=name,driver_version`. If that fails or takes over 10 s, `describe` fails with
  -32603 rather than recording a null that would break replay later. Null means there is no NVIDIA driver at
  all.

`start` refuses with -32003 a configured file whose hash differs, and names the file. The protocol's `start`
result is `{}`, so the Isaac Sim version that ran is in `inspect`, as `plant/sim.isaac_version`. It comes from
`IsaacIO.info()`; the stand-in reports `"test double"`.

## The arena clock

- The factory is never edited. The part replaces two names on the imported `factory` module:
  - `time`, whose `time()` is the plant epoch plus arena time;
  - `datetime`, whose `now()` is `plant_start` plus arena time.

  That makes `Factory.reset`'s plant clock, the promised dates and each event's `wall` the same in every run.
- `Factory` is built with `erp=None` and `triggers=None`, and its threads are never started. The part calls its
  methods directly, under its lock, and then `drain()`s its inbox.
- `uses_clock: true`. `advance(clock_ms)` runs fixed steps of `1/physics_hz` s up to `clock_ms`, counted from
  arena time 0. How the advances are split doesn't matter.
- `advance` returns the factory events since the last `advance` as `{kind, text, data}`. `data` is
  `{source, seq, t, detail}`: `t` is the plant time, `detail` is the factory event's own data, and `wall` is
  dropped. The factory's keys and these never overwrite each other.
- `call_tool` first steps to its `clock_ms`, then acts. Its events stay queued for the next `advance`.
- `start` calls `start_scenario(seed)` with the arena seed. That resets with the seed, so the floor noise
  follows it. With `scenario: false`, `start` calls `reset(seed)` instead.
- The plant clock advances by the factory's own `timedelta(seconds=dt)`, which rounds 1/360 s to 2778 µs. So it
  runs 0.016 % ahead of arena time, as it would in the app.

**Speed of the stand-in (measured on this machine):** 120 s of physics at 360 Hz took 0.91 s, or 7.6 ms per
second of physics, about 130 times real time. One arena hour costs about 30 s. With seed 7, zone C came at 33.7
s of arena time, zone D at 46.4 s and the trip at 61.9 s. `test_scenario.py` expects the trip between 55 and
66 s.

## Tools

The names and arguments are those of the factory's `mcp/src/ridgeline_mcp/floor.py`. Each tool calls the
`Factory` method that the matching `api.py` route calls, directly, not over HTTP.

| Tool | Effect | Factory call | Keys |
|---|---|---|---|
| `plant_state` | read | `state()`, the keys `floor.py` keeps | reads every document key |
| `machine_telemetry(machine, minutes=30)` | read | `telemetry` | `machine/<id>` |
| `vibration_spectrum(machine, seconds=20, before_trip=true)` | read | `spectrum` | `machine/<id>` |
| `recent_events(since=0, limit=60)` | read | `events_since` | none (the log is not keyed) |
| `wait_for_event(kinds, since, machine, timeout_s)` | read | `events_since` | none |
| `predict_move(order, to_line)` | read | `whatif_move` | `order/<id>`, `line/<id>` |
| `dispatch_technician(proposal_id, technician, machine, task)` | write | `dispatch_technician` | `technician/<id>`, `machine/<id>` |
| `set_line_status(line, status, rate)` | write | `set_line` (`rate` is `boost`) | `line/<id>` |

- `wait_for_event` returns at once, with `timed_out` when nothing matches yet. Inside `call_tool` the arena clock
  stands still, so a wait would never end. `timeout_s` is accepted and ignored.
- `proposal_id` is accepted and not verified. Approval gating is the `sculptured` integration's job.
- The plant runs without ERPNext in the arena, so it has no Work Orders. `orders` is empty, and `predict_move`
  refuses: "predict_move needs the plant's Work Orders, which come from ERPNext; the plant runs without ERPNext
  in the arena".
- Records the factory would file in ERPNext (lockouts, statements, inspection notes) come out as events saying
  they were not filed ("no ERPNext link"). That is the factory's `--no-erp` behaviour.
- Refusals come back as a tool-level `error` and change nothing. They cover:
  - an unknown machine, technician, line or order, checked before the Factory is called;
  - a `task` other than `inspect` or `repair`;
  - a bad argument type;
  - the Factory's own `ValueError`s, such as a busy technician or an unknown line status.

  An unknown tool name is -32602.

## Keys, inspect and diff

- Keys come from `state()`: `machine/<id>`, `line/<id>`, `order/<id>`, `technician/<id>`, `operator/<id>`,
  `shipment/<id>`, `robot/<id>` and `incident/<id>`. There are also `plant/clock` (`t`, `clock`, `speed`,
  `scenario`, `log_seq`) and `plant/sim`.
- `diff` compares documents exactly, except for floats inside the physics readings (`sensors` and `pose`
  subtrees). Those count as the same when they differ by at most 0.05 plus 1 % of the larger one, because they
  drift after an approximate restore. The protocol has no "not compared" mark.

## Restore: `{exact: "replay", approximate: true}`

- **Exact by replay.** Physics state on the GPU can't be saved exactly, and replay from the seed is exact
  (verified on the stand-in: same seed and inputs give equal tool results, events and `inspect`). A start
  `from` a save in `exact` mode is refused with -32001.
- **Approximate saves.** A save is a gzipped pickle (protocol 5), `plant.pkl.gz`, with `docs.json` beside it for
  `inspect` and `diff`. The pickle holds:
  - every `Factory` attribute except `io`, `lock`, `cond`, `inbox`, `erp`, `triggers` and `log`. The list is
    in `state.py`, and a test fails if `Factory` gains an attribute that isn't classified;
  - the backend's state: FakeIO's presses, robot and people for `standin`. For `isaac`, it holds articulation
    joint positions and velocities, the robots' poses and joints, the people's poses and the last IMU readings,
    all read through `IsaacIO`.
- Restoring works in a fresh process. It first loads the factory's modules and the backend's (`fake_sim` and
  `press_dynamics` for the stand-in), then unpickles. A save that can't be read is -32001. So is one made with
  another backend, `physics_hz`, `plant_start` or `data_path`, which is checked before anything is built.
- Restoring rebuilds the `Factory` and sets that state back. On `isaac` this is approximate: PhysX's solver warm
  start and contact caches are lost, and the restore calls (`set_dof_positions`, `set_world_poses`, ...) are
  unverified on 6.2.
- Each press's 120 s spectrum history (`signals.Ring`, about 0.5 MB a press) is saved empty, because no tick
  reads it. After a restore, `vibration_spectrum` therefore sees only the seconds since the restore. The
  pre-trip window that `before_trip` uses (`VibrationChannel.frozen`, 60 s) is kept.
- On the stand-in all the other simulation state is in the save, so a restored plant continues exactly as the
  original (events, documents, and later readings). Only spectra of the time before the restore differ.
- The pickle is acceptable because only this integration reads it, from its own save folder, and the
  fingerprint guards the code that reads it.
- **Disk cost per save (measured):**
  - 39 kB before the trip;
  - 221 kB after it, mostly the frozen pre-trip window;
  - 250 kB at 10 min, because the 1 Hz telemetry grows up to its 2 h cap.

  arenad saves at every step, so a run of a few hundred steps after the trip costs tens of MB.

## Mirror mode: package `isaac-sim-mirror`

`integrations/isaac-sim-mirror/arena-package.toml` runs this module with `--mode mirror`. It follows a running
factory (config `plant_url`, default `http://127.0.0.1:8090`) and only ever GETs.

- No tools.
- `start` reads `GET /state` and follows from its `log_seq`. Events from before the start are not this world's.
- `advance` returns `GET /events?since=<last seq>` as world events. A reply that isn't a list of events, such as
  `{"error"}`, is -32603.
- `inspect` returns pieces of `GET /state`, under the same keys as above. `save` stores the current `/state`
  for display (`exact: false`).
- Restore is `{exact: "none", approximate: false}`. A live plant can't be rewound, so a start `from` a save is
  -32001, and arenad's restore table refuses to fork a world that contains the mirror.
- It has no `[[example]]`, so `arena-part-check` can't run on it. Its test uses a stub HTTP server on a free port.

## Running the `isaac` backend on a DGX Spark (not yet done)

The factory's `isaac/spark/run.sh` shows how Isaac Sim 6.2 runs there: the `nvcr.io/nvidia/isaac-sim:6.2.0`
container, its cache mounts, `-u 1234:1234`, `--network=host`, and `LD_PRELOAD` of libgomp on aarch64. This
integration needs the same container, with three changes:
- `-i` instead of `-t`, because the protocol is stdin and stdout;
- the `Sculptured` folder (holding both `arena/` and `Sculptured-Examples/`) mounted, for example at `/work`;
- `ARENA_SAVES` passed and mounted read-write.

Inside the container, the command is:

```bash
cd /work/arena/integrations/isaac-sim && PYTHONPATH=/work/arena/kits/python \
  exec /isaac-sim/python.sh -m isaac_part --backend isaac
```

Put that `docker run` in a manifest under a separate `ARENA_PACKAGES` root, named `isaac-sim`, in place of the
`uv run` command. Then check:
1. `start` builds the scene and steps;
2. `inspect` shows `plant/sim.isaac_version`;
3. the zone C, zone D and trip events land near the stand-in's times;
4. an approximate restore continues without a jump in the vibration readings.

## Tests

```
uv run --with pytest pytest -q
```

All tests run on `standin`. The conformance test runs `arena-part-check` on a copy of the manifest that uses
`--backend standin`; it is skipped when the binary is not built (`cargo build -p parts --bin
arena-part-check`). A restore in a fresh process is tested by running the part over stdio. The suite takes
about 3 s.
