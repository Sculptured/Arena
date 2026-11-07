# Arena UI: dumb simple to start, depth one click away (2026-11-07)

Approved by the user in chat on 2026-11-07 ("go with the UI"). Their words: "make it dumb simple to start",
"way way way less crowded", "as simple as possible for any dummy to use", "things should be capitalized",
"running it from a single view should be as simple as a click run and see things populating the timeline and
stuff in real time, and you can pause", "making all the advanced features a little hidden so knowledgeable
people can use them but normal people aren't intimidated". The timeline is good and stays.

## Hard rule: real models only
No scripted, demo, stub or fake agents anywhere: not as a default, a fallback, a fixture team, or a test
double. Every agent run uses an actual LLM: Claude Code or Codex through their logged-in CLIs (both available
on this machine), or an API provider when its key is set. If no model is available, the UI says exactly what
is missing (for example "Claude Code isn't installed" or "Set OPENROUTER_API_KEY") and offers no stand-in.
Model variance between runs is expected and is what Arena measures.

## 1. One main screen: Run (the home page, `/`)
Before a run:
- Left: **World**: the benchmark's name and task in plain words ("Larchmont Biologics · Batch CVX-2611 on
  hold"), a small thumbnail of the digital twin (its first view, or the area names when there's none), a task
  picker (plain-language task names, ids hidden), and **Add a disruption** (the existing scenario builder,
  collapsed to a picker; parameters behind "Adjust").
- Right: **Team**: the team as a visual board (section 3), read-only, with **Import** and **Edit**.
- One big primary **Run** button. Under it, one line on what will run ("6 agents · Claude Code · Sonnet").
  The runner choice (one after another, all at once, external system) lives under Advanced.

After **Run**, the same screen becomes the live view (no navigation, no dialog):
- A toolbar: **Pause** / **Resume**, **Stop**, elapsed time, a status chip (Starting, Running, Paused,
  Passed, Failed, Stopped).
- **Timeline | Digital Twin** switch above the main area; the timeline fills in real time as steps happen,
  the twin follows the latest step (with its scrubber when the run is done or paused).
- Starting the world takes a while: show a calm progress state ("Restoring the world: ERP, CRM, LIMS…"), not
  a blank screen.
- When the run ends: a result banner, **Passed** or **Failed** with one plain sentence on why (the first failed
  check or safety violation, in words), and **Rewind** (opens the fork flow at a chosen step) and **See all
  details** (the full run page).
- Clicking a step opens a slim side drawer: what happened in one or two sentences (agent, tool, result,
  refused or not and why). Raw arguments, permits, reads/writes, timings: behind a "Details" disclosure.

Pause needs the backend (it doesn't exist yet): `POST /api/branches/{id}/pause` and `/resume`. While paused,
arenad holds new tool calls and gateway model calls for that branch (they wait, they don't fail) and the world
clock stops; Resume releases them. `GET /api/branches/{id}` reports `paused: true`. Record pause and resume as
events so the timeline shows the gap. Works for arenad's own runners (CLI and API mode) and external runs. Put
its tests in a new test file (another branch is rewriting the existing arenad test files).

## 2. Timeline: mouse-based zoom and scroll
Keep everything it does now (lanes, marks, buttons, keys). Add: wheel scrolls; Ctrl/⌘ + wheel and trackpad
pinch zoom around the cursor; click-drag on empty space pans. Smooth, no page scroll hijack outside the
timeline. The legend collapses to a "?" popover. In a live run the timeline auto-follows the newest step until
the user scrolls or zooms away; a "Follow" button brings it back.

## 3. Team: a read-only Sculptured-style board, importable
- Draw any team as a Sculptured board: agents as cards (name, a one-line job, model badge, read-only badge,
  tool/decision-right badges), grouped by role when groups exist, with "hands work to" arrows. A team that
  isn't from Sculptured is laid out the same way (chain order left to right, or one row for "all at once").
  Visual, not a list. Clicking a card shows its prompt (read-only) in a side panel.
- **Import**: a Sculptured design file (the JSON in `sculptured-control-plane/examples/teams/*.json`: `{id,
  name, groups, agents[{id, group_id, name, kind, job, folder, x, y, advanced{model, permission, resume}}],
  connections, revision}`; map `kind` claude → Claude Code, codex → Codex; use x/y and connections for the
  board), an Arena team JSON, or paste/drag the file. Optionally from a running Sculptured by URL if its API
  makes that simple; otherwise file only.
- **Edit** opens the existing editor (moved off the main screen).

## 4. Less crowded, everywhere
- Capitalise labels and values: Passed, Failed, Read, Write, Refused, One after another, By an external
  system, Claude Code. Sentence case for labels; no ALL-CAPS paragraphs.
- Fewer words per screen: no ids, hashes, raw seq numbers or image tags on the main surfaces (move them into
  Details / Advanced). Numbers only where they matter.
- Navigation: **Run** and **History** (the runs list, simplified: name, team, result, when). The status bar
  shows only what needs attention (a model not available, saves nearly full).
- Keep the ISA-101 restraint: greys, colour only for abnormal states.

## 5. Advanced, a little hidden
Suites, Compare, Process view, Event log, Trace, Deployment estimate, restore modes, runner choice, scenario
parameters, branch lists, world rows: all still there and reachable, from an **Advanced** menu in the top bar,
from "See all details" on a run, and from ⌘K. Nothing is deleted; it just isn't on the first screen.

## Not now
Agent connections for any deployed system (Bedrock, HTTP, etc.: parked, see the backlog), new benchmarks,
Sculptured-side changes.
