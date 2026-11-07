# Summit demo: disrupt, run, fail, rewind, change, compare (2026-11-07)

For the Precision in Pharma Supply Chain summit, Boston, November 16–17. The user said "Get the demo done now"
and chose the **production world** (Larchmont Biologics, batch CVX-2611 on hold). Agents stay **scripted**
until the user approves credits, and every frame says so. Arena is the product being demonstrated: any
team's agents can connect to it and be tested against realistic disruptions.

## The three-minute story (what has to work)

1. **World.** Open the digital twin of Larchmont: the fill line, the QC lab, warehouse lots by status, open
   customer orders with due dates, and cold-chain lanes. These are live, interconnected state, not a dashboard.
2. **Disrupt.** In the scenario builder, pick a disruption: the usual courier loses capacity after its first
   quote, a lot's retest fails mid-run, transit is delayed 2 days, or the first reallocation is rejected.
   These are scheduled inside the simulated parts. **The agents are not told.** They find out when they query
   or get the event.
3. **Run.** The team acts through the tools. The twin and the timeline change step by step.
4. **Fail.** Team v1 tries to book a shipment before approval, or ships a short-dated lot. Arena refuses or
   flags it, and the twin shows the consequence.
5. **Rewind.** Scrub the twin back to the step before the bad decision. "Rewind here" forks the world at
   exactly that state.
6. **Change.** Swap the architecture: v1 becomes v2 ("books only after approval, checks shelf life") or the
   single agent. Rerun from the fork.
7. **Compare.** Side by side, outcomes rather than transcripts: valid plan, safety violations, orders on time,
   transport cost, agent cost, escalations, plus the divergence timeline. Then a batch of about 30 seeded
   incidents shows these are rates, not anecdotes.

## Contract (exact shapes; builders work against these in parallel)

### A. Scheduled disruptions in the simulated parts (pharma-logistics, pharma-lims)
- The scenario tool (`logistics_scenario` / `lims_scenario`, called by task setup) takes
  `{name, disruptions?: [Disruption]}`. A named scenario's own data may also list `disruptions`.
- `Disruption = {id, when: {after_calls: int, tool?: string}, do: {kind, ...}, text}`.
  - `after_calls` counts agent tool calls this part has answered since setup. `tool` optionally counts only
    calls of that tool.
  - It fires deterministically **before** answering the call that crosses the count, so the call that
    triggers it already sees the changed world.
  - `do.kind` values:
    - logistics: `lane_capacity {carrier, origin, destination, capacity}`; `lane_unavailable {carrier, origin, destination}`;
      `transit_delay {carrier, origin, destination, extra_days}`; `reject_next {action}` (the director rejects the next decision on that action).
    - lims: `result_fails {lot, test}` (moves the lot's status to failed or hold).
- When a disruption fires, the part puts `{kind: "disruption", text, data: {id, ...do}}` in that call's
  `events`, so Arena records it as a World event at that moment. It's part of saved state, so forks and
  replays see the same schedule (the parts restore exactly).

### B. A scenario chosen per run (the builder)
- The benchmark `describe` adds `scenario_options`, the knobs the builder offers:
  `{base: [task ids], disruptions: [{id, label, description, part, default?: bool, params: [{name, label, kind: "int"|"choice", choices?, min?, max?, default}], template: Disruption-with-"{param}"}]}`.
  The template's string values may contain `{param}` placeholders, filled from params.
- `POST /api/runs` and `POST /api/suites` take an optional
  `scenario: {disruptions: [{id, params: {name: value}}]}`. arenad expands it with the templates and appends
  the expanded `disruptions` to the setup call args of the part each one names. The expanded scenario is
  recorded in `RunStart` (`scenario` field), so replays and forks keep it.
  `GET /api/benchmark` (or the existing describe endpoint the UI uses) returns `scenario_options`.

### C. The digital twin (`world_view`)
- New optional benchmark method `view {connections}` → `{view: TwinView}`. arenad calls it after setup and
  after every step that wrote anything, a disruption or a link included. It records an event
  `{kind: "world_view", payload: {seq_of: <step seq>, view: TwinView}}` right after that step. A failing
  `view` records nothing and never fails the run.
- Generic `TwinView`, so any world (insurance later) can draw one:
  ```
  {title, clock?: string,
   areas: [{id, label, kind: "plant"|"lab"|"warehouse"|"customers"|"carriers"|"approvals"|string,
            items: [{id, label, status: string, abnormal: bool, detail?: string, qty?: number, unit?: string,
                     due?: string, link?: string}]}],
   flows: [{id, from: item id, to: item id, label, status: string, abnormal: bool}],
   kpis: [{id, label, value: string, abnormal: bool}]}
  ```
  For pharma:
  - Areas: Fill line, with work orders and the batch on hold. QC lab, with lot results. Warehouse, with lots
    and their qty, status and expiry. Customers, with open orders, due date and allocated lot. Carriers, with
    lanes, capacity, qualified status and availability. Approvals, with open and decided changes.
  - Flows: booked shipments, lot to customer.
  - KPIs: orders at risk, orders covered, violations so far, transport cost.
- `GET /api/branches/{id}/views` → `[{seq, view}]`, the branch's own views plus those it inherits before
  its fork point.

### D. UI (Arena)
- `World` view at `/world/:runId/:branchId`, linked from the run header:
  - the twin drawn as an HMI overview: areas as zones, items as tiles with glyph and word, flows as arrows,
    colour only for abnormal;
  - a scrubber over the branch's steps, synced with the timeline selection;
  - items that changed since the previous view are highlighted with what changed;
  - the disruption marked at the moment it fired;
  - **Rewind here** opens the fork panel at that step with "Swap architecture" preselected.
- **Scenario builder** in the New run and New suite dialogs, drawn from `scenario_options`, with a
  plain-words preview ("Tundra loses capacity after its first quote; the agents are not told").
- **Outcome comparison** on the compare page, above the record diff: valid plan (all goal checks), safety
  violations, orders on time, transport cost, agent cost, escalations (approval requests), and the steps
  where the two branches diverged (first divergence plus the disruptions).

### E. Scripted demo driver (examples/feature-tour → examples/summit-demo)
- Behaviours react to disruptions the way a sensible agent would. When a lane loses capacity, pick the next
  qualified lane. When the first reallocation is rejected, propose the alternative.
- `fork` mode: fork a branch at a step with another deployment, then resume it with the new behaviour from
  that point.
- A duplicate-shipment preset: two agents each book the urgent order (v1), v2 checks shipments first.
- A batch suite of about 30 seeded incidents (scenarios × disruption params × repeats).
- A one-command demo reset, and a runbook with the three-minute click path.

## Not now
Clinical trial world, real SAP connectors, a map or 3D view (the twin is a schematic), real model runs
(waiting on credits), Systems 4–7 (after the summit).
