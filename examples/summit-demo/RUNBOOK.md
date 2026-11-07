# Summit demo runbook: disrupt, run, fail, rewind, change, compare

Three minutes, Precision in Pharma Supply Chain, Boston. The world is Larchmont Biologics with batch
CVX-2611 on hold. **Every agent is scripted; usage and costs are illustrative; no model is called.**
Every page says so ("Scripted agents, illustrative usage · no model calls").

## Before you go on (10 minutes ahead)

```bash
cd /home/alexsky2/MLSys/Sculptured/arena-demo
examples/summit-demo/reset.sh          # restarts the demo arenad, deletes rehearsals, records the World run
cd examples/summit-demo && python3 drive.py watch    # leave it running in its own terminal
```

`reset.sh` prints the **World beat** URL. Open two browser tabs at 1440×900 or larger:

- **Tab A:** the World beat URL (`/world/<run>/<branch>?step=1`).
- **Tab B:** `http://127.0.0.1:4513/` (Runs).

Also keep the batch comparison URL at hand: `http://127.0.0.1:4513/suites/compare/<v1 suite>/<v2 suite>`
(the ids are in "Batch results" below, and `drive.py reset` lists the batch suites).

Two waits are real and you talk over them: starting a run restores the whole world (ERPNext, Twenty,
LIMS, carriers, approvals) from its base save, about 50 s in the rehearsal; a fork restores it from the
step's saves, about 17 s. The paced v1 run takes about 40 s once driven, the rerun from the fork about
45 s.

## The click path

| Time | Beat | Click | Say |
|---|---|---|---|
| 0:00 | **World** | Tab A. Point at the zones left to right. | "This is Larchmont's digital twin: the fill line with CVX-2611 on hold, the QC lab, lots in the warehouse with their status and expiry, the open orders with due dates, the cold-chain lanes, and the approvals. Live, interconnected state from a real ERP, CRM and approvals system, not a dashboard." |
| 0:20 | **Disrupt** | Tab B → **New run**. Task stays `cvx-2611-em-excursion`. **Agents run** → *by an external system*. Tick **Usual courier loses capacity** (defaults: after 1 quote, 0 left). Read the preview line. **Start run**. | "Any team's agents connect to Arena. I schedule a disruption inside the simulated carrier: the usual courier runs out of capacity after its first quote. The agents are not told." |
| 0:30 | *(restore, ~50 s)* | While "Starting…" shows, switch to Tab A and press **▶** on the scrubber. | "Arena restores the entire world exactly as it was saved, every part, so every run and every rerun starts from the same state." |
| 1:20 | **Run** | Tab B is now the run page. Click **World**. The twin and the timeline change step by step (fast, then slow over the first shipment). | "Team v1 handles the excursion: deviation, hold, reallocation, then shipping." |
| 1:35 | **Fail** | **▲ refused** appears at the first booking; a moment later **Safety violations so far** turns red and St. Brennan's order reads *short-dated lot*. Then **▼** marks the courier running out. | "v1 tried to book the shipment before it was approved. Arena refused it. Then it shipped a short-dated lot to St. Brennan: the twin flags it. And the courier filled up: the next order went to the next qualified lane." |
| 2:00 | **Rewind** | When the run says *failed*: click the track just left of the **▲** (or use **←/→**) until the headline reads *Step … · Logistics and customer service · cold_chain_quotes*, the quote right before the refused booking. Click **Rewind here**. The fork panel opens with **Swap architecture → Team v2** chosen. | "I scrub back to the moment before the bad decision. Rewind here forks the world at exactly that state." |
| 2:08 | **Change** | Click **Change and rerun from here (6 changes)**. While "Forking…" shows (~17 s): | "Same world, same disruption, a different architecture: v2 books only after approval and checks shelf life. Everything before this step is reused." |
| 2:25 | **Rerun** | The new branch opens; click **World**: v2 proposes, gets approval, books CVX-2604, violations stay 0. | |
| 2:30 | **Batch** | While the rerun finishes (~45 s), open the batch comparison URL. | "And this isn't an anecdote: thirty seeded incidents per architecture, every task under three disruptions, twice. Pass rate 50% against 100%, a safety violation in half of v1's runs and none of v2's. These are rates." |
| 2:50 | **Compare** | Back on the v2 run page: **Compare with…** → the failed parent. Read the **Outcomes** table. | "Outcomes, not transcripts: valid plan 8 of 9 against 9 of 9, one safety violation against none, orders on time, transport cost, agent cost as reported, escalations, and where the two diverged." |

### Closing line (20 s)

"Your agents, any framework, connect to Arena and meet the production world: real ERP, CRM, LIMS
and approvals, with disruptions they aren't told about. Arena refuses what isn't allowed, shows the
consequence in the twin, lets you rewind to the exact moment and try another design, and turns it
into rates across hundreds of incidents, before anything touches production. These agents were
scripted for today; yours won't be."

## If something misbehaves

| Symptom | Do this |
|---|---|
| **Start run** spins past 90 s, or errors | Use the recorded pair instead: open "Recorded fallback" below (World of the v1 run, then its compare). Say "here is the run I recorded this morning". |
| The run page stays at its first steps (nothing moves) | The driver isn't watching: in its terminal run `python3 drive.py watch`. It picks the run up within a second. |
| **Rewind here** is greyed out | The run is still running: wait for *failed*. At the last view there is nothing to rerun: press ← to the quote before the ▲. |
| A fork sits still for a few seconds after it opens | Normal: the driver waits for the fork's world to go live, then resumes it. |
| **Change and rerun** errors (fingerprint, refused restore) | Fork from the recorded fallback v1 branch instead (same click path), or open the recorded compare. |
| The fork doesn't move after it opens | Same as above: the driver isn't watching. If it printed `diverged`, you forked after the first booking: rewind to a view before the ▲ instead. |
| The UI looks stale or broken | Reload the page. If arenad died: `examples/summit-demo/serve.sh` (keeps all recordings), then `drive.py watch`. |
| Everything is wrong | `examples/summit-demo/reset.sh`, then `drive.py watch`, then start from the World beat. Allow 3 minutes. |
| Want the single agent instead of v2 | In the fork panel choose **Architecture → Single agent** (only for runs an external system drives). The driver resumes it as one agent. |

## What each command does

- `reset.sh` — `serve.sh` (stop the demo arenad, `arenad base`, start it detached, log in
  `/home/alexsky2/sculptured-work-local/arenad-demo.log`), then `drive.py reset` (delete runs named
  "Summit live/world/duplicate" and unnamed external runs outside any suite; the batch suites stay),
  then `drive.py world` (record the World beat's run: Team v2, no disruption).
- `drive.py watch` — drives every run waiting for an external system, by its team: one agent is the
  single agent, prompts with "Deployment rules" are Team v2, else Team v1. A fork resumes from its
  fork point: the calls before it are answered from the record.
- `drive.py live` — the same live run from the terminal (paced; `--fast` for none).
- `drive.py fork --branch <id> --at <step> --deployment v2|single` — Rewind here from the terminal.
- `drive.py duplicate --deployment v1|v2` — the duplicate-shipment preset (`urgent-escalation`).
- `drive.py batch --deployment v1|v2` — a batch suite: 5 tasks × 3 disruptions × 2 repeats.
- `capture.mjs` — this click path through the UI with Playwright, screenshotting each beat.

## Recorded fallback and batch results

- **Recorded fallback** (from the rehearsal through the UI, kept by `reset.sh`):
  v1 run `http://127.0.0.1:4513/world/caad0e0e307945edb61b2b5e44fc9e8b/d23ebed967fe43059bc4a2557e96e9a0?step=122`
  (the refusal), the v2 fork `/world/caad0e0e307945edb61b2b5e44fc9e8b/12637733b7fb4121b588abbc92ac20aa`,
  and their comparison `/compare/d23ebed967fe43059bc4a2557e96e9a0/12637733b7fb4121b588abbc92ac20aa`.
  The run is named "Summit fallback: …", so `reset.sh` keeps it.
- **Batch comparison**: `/suites/compare/1a1c528b27664aeca4d95529fb02f6d4/58af69e1eef3487794d35c60999f94d2`
  (v1: 50% pass, a violation in 15 of 30 runs; v2: 100%, none).
