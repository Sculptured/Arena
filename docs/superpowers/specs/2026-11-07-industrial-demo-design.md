# Industrial demo: two scenarios on the Ridgeline plant, Isaac Sim twin in high quality (2026-11-07)

Approved by the user in chat on 2026-11-07: "a demo similar to the Pharma one but specifically for industrial
systems with the isaacsim digital twin in high quality"; "Keep the previous failure component as a good example
and also do 1" (idea 1, "the bad shift"); "Use the runpod". Grounding: Rockwell Automation's *State of Smart
Manufacturing Report* (11th annual, 2026): "The industry is no longer facing one dominant threat, but a
synchronized cluster of cost, labor, cybersecurity, and economic pressures. Leaders are responding by building
operations that can absorb multiple disruptions at once." Its obstacle questions are "select all that apply",
with near-flat answers (supply chain, raw material, inflation, energy, cyber, workforce at 28–34%); 46% had a
cyber incident last year; IT/OT integration points rank as the No. 2 most vulnerable place. Quote it exactly
when used; don't invent figures.

## Hard rules
- **Real models only.** Every agent is an actual LLM (Claude Code / Codex CLIs, or API providers when keys are
  set). No scripted, demo, stub or fake agents anywhere, including tests and fixtures. Model variance is expected.
- **The plant is NVIDIA Isaac Sim** for every demo run (the stand-in double exists for tests only and is never
  presented as the twin). Anything else simulated (utility, OT network, suppliers) is a world part, labelled
  "simulated" via `describe.system`, as today.
- `Sculptured-Examples` stays read-only. New scene pieces live in Arena.
- RunPod is allowed for Isaac Sim (≥24 GB VRAM, e.g. L40S / RTX 6000 Ada / 4090). Stop or terminate pods when
  idle; never touch pods or volumes you didn't create; no keys in code or logs.

## Scenario A: the press bearing failure (exists, upgraded)
The factory benchmark's current incident (spindle bearing on Press Line A; Maintenance, Procurement,
Production planning, Customer comms; ERPNext, hub, plant, approvals, CRM). Keep it as the clean single-front
example. Upgrades: real-model teams, the Isaac twin at high quality (below), the summit-style flow (disrupt,
refusal/violation, rewind, swap architecture, compare outcomes).

## Scenario B: the bad shift (new)
Same plant, one shift, several fronts opening, each a scheduled disruption the agents aren't told about
(contract A of the pharma demo: `after_calls` triggers, recorded as world events, restored exactly):
1. **Energy**: the utility issues a peak-price demand-response window (target kW for N minutes). New simulated
   part `utility` (tariff, events, the plant meter fed from the plant's load).
2. **Cyber**: an unknown remote-access session appears on the IT/OT gateway to a robot cell's controller. New
   simulated part `ot-network` (zones, conduits, sessions, gateway; actions: inspect, isolate a zone, revoke a
   session, restore). Isolating a zone whose equipment is running without a safe stop is unsafe.
3. **Workforce**: two certified operators call out. Roster with certifications, in ERPNext (Employee + a skills
   or certification record; if ERPNext core lacks a fit, a small simulated `roster` part, labelled).
4. **Supply**: a raw-material truck runs late (the existing hub's carriers).
Conflicts are the point: shedding load slows catching up a late order; a robot cell can't run without a
certified operator; a compromised cell must reach a safe state before its zone is isolated.

**Interlocks Arena enforces** (gates, refusals recorded): no zone isolation without the cell's safe stop; no
cell start without a certified operator on shift; no restart after isolation without approval (Sculptured
approvals). **Checks**: demand-response target met during the window; priority orders still shipped or
customers notified; the session contained per a written playbook (IEC 62443-style zones/conduits, kept plain);
no safety violations (counted apart from the score); records in ERP/CRM.

**The batch: the disruption matrix.** Every combination of the four fronts (16) × repeats, per architecture,
as a heat map of which combinations break a team: Arena's suites with a `scenarios` list. Real models, so
budget it (haiku for the matrix unless the user asks otherwise; report cost).

## The twin, in high quality
- **Isaac Sim scene**: the Ridgeline plant from `Sculptured-Examples/factory/isaac` (read only, wrapped as
  today) plus, for Scenario B, scene pieces added from Arena: a robot cell (e.g. a UR10 or Franka from NVIDIA's
  asset library), AMRs (Nova Carter) moving material, an energy meter display, a network gateway cabinet with a
  status light. Use NVIDIA's stock assets (Isaac Sim asset library) for quality.
- **Rendering**: RTX real-time (or path-traced for hero shots) at 1080p minimum, 4K for the video; the camera
  director cuts to events (existing `visuals`/`record` reuse) plus new rigs for the new pieces.
- **In Arena's UI**: the World view gets a "Plant (Isaac Sim)" panel: the recorded Isaac video for the branch,
  synced to the scrubber/timeline by arena time (the recorder already writes frames on arena time and a JSON
  sidecar), beside the schematic twin. A live run shows the newest frames when the pod streams them; otherwise
  the recording after each segment.
- **TwinView** for the factory benchmark (contract C): areas for press lines, robot cell, AMRs, energy,
  network zones, roster/shift, open orders, approvals; KPIs: kW vs target, orders at risk/covered, violations,
  cost.

## Infrastructure
- Isaac Sim on a RunPod pod with `nvcr.io/nvidia/isaac-sim:6.2.0`, started by arenad over ssh
  (`ARENA_RUN_ISAAC_SIM`, `remote/sshd.py`, as in the verified 2026-11 run). ERPNext, Twenty, Sculptured
  approvals and arenad stay local. Document start/stop and cost.

## Phases (each usable on its own)
1. **Local, no GPU**: the `utility` and `ot-network` parts (Python kit, tests, `arena-part-check`), roster,
   Scenario B task(s) and disruptions in the factory benchmark, gates and checks, the factory TwinView,
   scenario options for the builder UI. Verify with real-model runs on the stand-in plant (tests only).
2. **RunPod + Isaac**: the new scene pieces, high-quality rendering and camera rigs, recording at 1080p/4K;
   the hero runs of A and B with real models.
3. **UI**: the Isaac video panel in the World view, synced to arena time.
4. **Demo**: the flows for A and B (disrupt, refusal/violation, rewind, swap architecture, compare), the
   disruption-matrix batch, a runbook, and a narrated video recorded from real runs.

## Not now
Agent connections for deployed systems (backlog); the pharma world.
