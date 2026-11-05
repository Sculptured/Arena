# isaac-sim

Puts the Ridgeline factory plant in the world, with Isaac Sim as the physics (the `isaac` backend) or a stand-in
double (the `standin` backend). Restore: exact by replay from the seed and inputs; approximate from a save
(a pickle of the factory and simulation state). `uses_clock`: true. Verified on the stand-in by its pytest suite
and the `arena-part-check` conformance check. The `isaac` backend has run once on Isaac Sim 6.2.0, on a rented
RTX 3090, through the factory benchmark's gold path, and recorded it to MP4 (see "Verified on Isaac Sim"). Its
package `isaac-sim-mirror` (see `integrations/isaac-sim-mirror/README.md`)
only follows a live plant.

**The factory example this package wraps is not public yet.** Its default `isaac_path` points into a private
`Sculptured-Examples` checkout next to the arena checkout; without one the package can't start and every test in
`tests/` is skipped (pytest says why). Outside contributors can read and change the code, but can't run it.

The Ridgeline factory as an `arena-part/1` integration, on the arena clock. In the factory demo
(`Sculptured-Examples/factory`, branch `isaac-plant`), Isaac Sim 6.2 **is** the plant. This integration wraps
that demo's `isaac/factory.py` class `Factory` without editing it. Factory is plain Python and reaches the
simulation only through a `PlantIO`.

Unless a line says Isaac Sim, *verified* below means the `standin` backend: the factory's own offline double,
`isaac/tests/fake_sim.FakeIO`, which is backed by `isaac/tests/press_dynamics.py`. What ran on Isaac Sim is in
"Verified on Isaac Sim".

## Backends

The backend is a process argument, `--backend isaac|standin`, because `describe` has no config and the backend
changes the fingerprint.

| Backend | PlantIO | Used by |
|---|---|---|
| `isaac` | `sim_io.IsaacIO` in a headless `SimulationApp`, set up as `run.py` does it: scene build, `IsaacIO`, `setup_physics`, play, `start()`. PhysX steps run with `SimulationManager.step`, and `Factory.physics_step` runs in the post-step callback. | `arena-package.toml` (`needs.gpu_vram_gb = 16`) |
| `standin` | `fake_sim.FakeIO`. Each step is `io.step(dt)` and then `Factory.physics_step(dt)`, the order `test_scenario.py` and the app use. | the tests (a test manifest written to `tmp_path`) and package `isaac-sim-standin` (`integrations/isaac-sim-standin`) |

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

## Recording: `record`, `camera`, `record_fps`

| key | default |
|---|---|
| `record` | none. An MP4 path, relative to `$ARENA_SAVES/isaac-sim/`. Each instance writes `<stem>-<instance>.mp4` (arenad runs a base, then a branch) |
| `camera` | `auto`: the factory's camera director, which cuts to events. Or `free`, or a rig in the factory's `layout.CAMERAS` |
| `record_fps` | 30: frames per second of arena time. The MP4 plays at this rate, so it runs in arena time |
| `record_height` | 720: the MP4's height; the width follows at 16:9 |
| `scene` | `ridgeline`: the factory's own scene. `showcase`: a dressed stamping plant (see "The showcase scene"). Visuals only |

- Only the `isaac` backend renders; `record` on `standin` is -32602. An unknown camera is -32602 before Isaac Sim
  starts.
- It reuses the factory's own code, read only: `visuals.Visuals` (tower lights, labels, crate, truck, camera
  director) and `record.Recorder` (a Replicator render product on the main camera, piped to ffmpeg), headless at
  1280x720 with the RaytracedLighting renderer, as `run.py --headless --record` does. The difference is the clock:
  a frame is rendered every `physics_hz / record_fps` physics steps, so frames fall on arena time, not wall time.
- Label textures go to `<stem>-<instance>_labels/` beside the MP4, and the recorder's `<stem>-<instance>.json`
  (frames, duration) too. The MP4 is finished on `stop` (or when the process exits normally); the log says
  `[isaac-sim] wrote <path>`. Fetch it from the machine the program ran on.
- If physics steps stop reaching the plant (its physics callback is not running), `advance` and `call_tool` fail
  with -32603 instead of leaving the plant standing still.

## The showcase scene: `scene: showcase`

With a recording and `scene: showcase`, the stage the factory built is dressed as a stamping plant before physics
starts (`isaac_part/showcase_*.py`). It changes only what is seen:
- The factory's press articulations, iw.hub, people, physics scene and floor stay exactly as built. Its press bodies are
  made invisible, its old shells and conveyors inactive, and its labels hidden. Factory, PlantIO, the press model and
  the signals are not touched, and `scene` is not passed to the plant. A test runs the stand-in plant with and without
  `scene: showcase` and compares events and documents.
- What is added is referenced at run time from NVIDIA's public asset server (the `Assets` folder of
  `omniverse-content-production.s3-us-west-2.amazonaws.com`); nothing is copied into this repo:
  - the ArchVis industrial hall (`ArchVis/Industrial/Buildings/Warehouse/Warehouse01.usd`, authored in cm, scaled 0.01);
  - two FANUC R-2000iC/165F arms per line, loading and unloading the press (`Isaac/6.2/Isaac/Robots/Fanuc/r2000ic_165f`);
  - conveyors (`ConveyorBelt_A08`), pallets, a forklift (`Isaac/6.2/Isaac/Props/Forklift`) and stock racks
    (`ArchVis/Industrial/Racks/RackLong_A1.usd`).

  Their physics is removed (bodies, joints, colliders, articulation roots), and a check at build time fails if
  anything under `/World/Showcase` would be simulated.
- The presses, washers, stillages, parts, stack lights, beacon and screens are generated meshes
  (`showcase_press.py`, `showcase_mesh.py`), merged one per material, with OmniPBR materials.
- Everything moves kinematically, from the plant's own readings, once per rendered frame (`showcase_visuals.py`):
  - the slide follows the ram's measured position, scaled to the visual stroke;
  - the arms follow the crank: joint angles from numeric inverse kinematics of the arm's declared joints, keyed to the
    press cycle (`showcase_robot.py`, `showcase_layout.py`);
  - blanks run along the infeed and formed parts along the outfeed into the stillage (`showcase_motion.LineFlow`).
    When the press stops, the infeed queues to the gate, the outfeed runs empty and the stillage stops growing;
  - the frame shakes with the measured vibration: nothing below 2.3 mm/s, 3 cm at the 11.2 mm/s trip level. That is
    exaggerated on purpose; it only follows the reading;
  - the stack light shows the press status, and a rotating red beacon lights while it is tripped or locked out;
  - a small andon screen on the press and an operator HMI show the press's spm, vibration and count;
  - a forklift drives a loop delivering blank pallets.
- `camera: auto` follows a plan (`showcase_camera.py`): establishing shots while nothing is wrong, then cuts to PR-A1
  on its zone C and zone D warnings and on the trip (the press, the beacon, the backed-up infeed, the empty outfeed). Any
  shot name holds that shot.
- `remote/render_showcase.py` drives the part (start, advance, stop) without arenad, for a video or stills.

**Ran on Isaac Sim (RunPod, 2026-11-05).** An NVIDIA RTX A5000 (24 GB, driver 595.91.07), the image
`nvcr.io/nvidia/isaac-sim:6.2.0`, `remote/render_showcase.py --seconds 100` (seed 7, 1280x720, 30 fps, RaytracedLighting):
- The MP4 has 3000 frames, 100.0 s at 30 fps, so it plays in arena time. The plant's own events in that run: zone C at
  35 s, zone D at 48 s, the trip at 64 s (the stand-in, seed 7: 33.7 s, 46.4 s, 61.9 s).
- Wall time: 10.9 to 11.1 s per arena second with the recording. Physics alone (no frames rendered) was 5.8 to 6.0 s per
  arena second with the showcase scene loaded, and 6.4 s with the factory's own scene on the same pod, so the dressing
  adds no physics cost. A frame cost 22 to 24 ms for the visuals in Python and 129 to 132 ms to render and hand to
  ffmpeg.
- Seen on 6.2: changing a material's `emissive_intensity` while running did not show in the render (texture and
  visibility changes did), so lit lamps are separate meshes that are shown or hidden.
- The first start, with empty caches, took 265 s; later starts 45 to 48 s.
- Not run with the showcase scene: arenad, a branch from a save, the dock and the truck (they come later in the
  scenario than 100 s).


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
| `add_shipment(part, origin, carrier, transit_hours, ref)` | write | `add_shipment` | `shipment/<id>` |

- `wait_for_event` returns at once, with `timed_out` when nothing matches yet. Inside `call_tool` the arena clock
  stands still, so a wait would never end. `timeout_s` is accepted and ignored.
- `proposal_id` is accepted and not verified here. A benchmark can gate the tool on the `sculptured` part
  (`docs/protocol.md`, "Gates on tools").
- `add_shipment` is what the factory's hub calls when a carrier is booked. In a world it comes through a
  benchmark's link (`docs/protocol.md`, "Links between parts"), and then agents don't get the tool.
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

## Running on another machine: `ARENA_RUN_ISAAC_SIM`

arenad replaces a package's run command with the command line in `ARENA_RUN_<PACKAGE>` (here
`ARENA_RUN_ISAAC_SIM`), run by `/bin/sh -c` (`docs/protocol.md`). Pointed at `ssh <host> -- <command>`, the part
runs on a GPU machine with the protocol on the ssh session's stdio:

```bash
export ARENA_RUN_ISAAC_SIM="exec ssh -p <port> -i <key> -o BatchMode=yes isaac@<host> -- 'cd /isaac-sim/work/arena/integrations/isaac-sim && \
  ARENA_SAVES=/isaac-sim/work/saves PYTHONPATH=/isaac-sim/work/arena/kits/python exec /isaac-sim/python.sh -m isaac_part --backend isaac'"
```

- The remote machine needs this folder, `kits/python` and the factory's `isaac/` and `data/` in the same layout
  (`arena/` next to `Sculptured-Examples/`), since the default paths and the fingerprint are relative to it.
- Saves and recordings stay on the remote machine, under its `ARENA_SAVES`: `save` writes to
  `$ARENA_SAVES/isaac-sim/<save>` (the protocol's convention) rather than arenad's `dir`, which is a local path.
  arenad's own count of save bytes doesn't see them.
- The `nvcr.io/nvidia/isaac-sim:6.2.0` image has no sshd and runs as uid 1234 without root, so `remote/sshd.py`
  is a small SSH server (asyncssh, key login only, one command per session) to start inside the container.

## Verified on Isaac Sim (RunPod, 2026-11-04)

**What ran where.**
- On a RunPod community-cloud pod, the plant ran: this part with `--backend isaac`. The pod had:
  - an NVIDIA GeForce RTX 3090 (24 GB), driver 580.95.05;
  - the image `nvcr.io/nvidia/isaac-sim:6.2.0`, whose `plant/sim.isaac_version` reads `6.2.0.rc.14.6`;
  - a 40 GB container disk and no volume.
- Everything else ran on a Linux workstation: arenad (`live_factory_gold_path_report`), the factory benchmark with
  `--backend isaac`, ERPNext (`frappe/erpnext:v16.53.0`), `ridgeline-hub` and Sculptured.
- arenad started the part over `ssh` (`ARENA_RUN_ISAAC_SIM`, `remote/sshd.py` on the pod), with
  `ARENA_CLOCK_SPEED=0.1` and `{"record": "factory-gold.mp4", "camera": "auto", "record_fps": 10}`.
- The team was the scripted, no-model team, on the gold path.

**Result.**
- The branch passed, with all nine checks.
- Its metrics: 1311.0 s measured, 156.9 s of arena overhead (145.9 s of it restoring the base, mostly ERPNext), so
  1154.1 s estimated in production. That estimate is at 0.1× clock speed, so it measures the slow clock more than
  the team.
- The test took 1324 s.
- The branch started from the base save approximately, so `set_state` (`set_dof_positions` ...) ran on 6.2
  without an error. The continuity of the readings after it was not measured.

**The recording.** 1158 frames, 115.8 s of arena time at 10 fps, 1280x720 H.264, 21.5 MB. The camera director
cut to PR-A1 near 35 s and near 50 s. From about 65 s to 75 s it showed "Press Line A DOWN" at 0 spm, then went
back to the wide shot. That is near the stand-in's zone C (33.7 s), zone D (46.4 s) and trip (61.9 s). The events'
own times were not compared.

**Speed on the RTX 3090.**

| What | Measured |
|---|---|
| First start (shader and asset caches empty) | about 4.5 min |
| Later starts | 31 s; another plant in the same process: 6 to 8 s |
| Physics alone | about 4 s of wall time per second of arena time (about 11 ms per 360 Hz step) |
| Recording at 30 fps | 9.4 s per arena second |
| Recording at 10 fps | 5.9 s per arena second |

That is why the clock ran at 0.1. The plant must step faster than the arena clock runs, or every call waits
longer than the last.

**Found and fixed on the way.**
- The physics callback registered for the first plant's stage stopped running after `ctx.new_stage()`. A second
  plant in the process then stood still, and a recorded one looped forever. Each plant now registers its own
  callback, and only the newest one reaches a plant.
- Fetching the 21 MB MP4 through `remote/sshd.py` broke off partway several times ("Broken pipe"). It came back
  whole in 256 kB `dd` pieces (sha256 checked). `sshd.py` now copies output with backpressure. The local test sends
  25 MB whole, but this version has not run over the internet link.

**Seen, not chased.**
- Right after a start, the readings are not like the stand-in's: PR-A1 read 16.4 mm/s (zone D) at 3 s.
- The early frames' labels show PR-C1 at 888.8 mm/s. This looks like the presses spinning up from rest.

**Not run on Isaac Sim:**
- `arena-part-check`, which needs three live instances, one more than the backend allows;
- a replay, a fork or a rerun;
- the wrong path;
- real models.

## Tests

```
uv run --with pytest --with asyncssh pytest -q
```

Without `asyncssh`, the tests of `remote/sshd.py` are skipped.

All tests run on `standin`. The conformance test runs `arena-part-check` on a copy of the manifest that uses
`--backend standin`; it is skipped when the binary is not built (`cargo build -p parts --bin
arena-part-check`). A restore in a fresh process is tested by running the part over stdio. The suite takes
about 3 s.
