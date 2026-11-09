# Arena UI: look, tokens, components, patterns

Arena's web UI is extra sparse: each screen shows only what a first-time user needs to do its one
main thing, with one primary action. Everything else is one click away under **⋯**, **More**,
**Details**, **Options** or ⌘K. No helper paragraphs on screens (a tooltip at most), no ids, no
secondary metadata lines, few borders, plenty of whitespace.

## Screens

- **Top bar** (`app/TopBar.tsx`): the wordmark, **Experiments** and **Runs**, and one **⋯** with Run
  once, Compare two runs, World view of a run, the Team editor, AI review on or off, the command
  palette, the theme and which Arena core is in use.
- **Experiments** (`/`, `experiments/ExperimentsPage.tsx`): newest first, each row the experiment's
  name, its scenario in words, one small bar per setup (passed of runs asked) and when. Tick two or
  more (the box shows on hover) to **Compare** their setups side by side. Empty: one line on what an
  experiment is and **New experiment**.
- **New experiment** (`/experiments/new`): the name inline as the page title; **Scenario** (the
  task by plain name, then disruptions as a picker); **Agent setups**, 1 to 4 cards (name, team,
  model for Claude Code agents, rules added to every prompt; a dashed card adds one, copying the
  last); **Runs per setup** (1, 3, 5, 10; default 5); a bottom bar with "2 setups × 5 runs = 10 runs",
  what blocks starting, and **Run experiment**.
- **Experiment** (`/experiments/:id`, `/experiments/compare?ids=`): the name, status and scenario
  line; **Stop** while it runs. One column per setup (`experiments/Results.tsx`, model in
  `experiments/model.ts`): the pass rate large with its 95% Wilson range as a band, a square per run
  (solid passed, ✕ failed, ! not scored, pulsing running, dashed not run yet, a red dot for a safety
  violation; each opens its run), then checks passed, safety violations, denied actions, flags for a
  human, cost and time per run, the best in bold. Under it the checks that some run failed, side by
  side as bars (all of them one click away). Two setups that ended get the AI comparison line above.
- **Runs** (`/runs`, `pages/Runs.tsx`): name, team, result, when; reruns under their run as "↳
  Rerun". A row opens on Run once's live view; its menu has All details, Process view and Delete.
- **Run once** (`/run`, `pages/Home.tsx`, `home/`): the world's name, a task picker, the team board
  and one big **Run**; **Options** holds the disruptions, the runner and the team's Suggested, Import
  and Edit. After **Run** the same screen is the live run (`?run=&branch=`, `home/LiveView.tsx`):
  **Pause** / **Resume**, **Stop**, status and time, **Timeline | Digital Twin**; a step opens a slim
  drawer; at the end the result in one sentence, **Details** and the run's **⋯**.
- **Run page** (`/runs/:run/:branch`): the task, its status and the run's **⋯**; time, cost, check,
  safety and any refusal; the review line; the timeline and event log with the step panel beside
  them; the trace once a step is picked; **More about this run**.
- **A run's ⋯** (`run/RunMenu.tsx`, on the run page, the world view, the process view and the live
  result): its other views, Compare with another run, its reruns, and under **Advanced** the only way
  to rewind: "Rewind to step N and rerun…" opens the fork panel in the step panel (`?fork=1`).
- **Compare runs** (`/compare/:a/:b`), **World view** (`/world/…`), **Process view** (`/control/…`)
  and the **Team editor** (`/team`) are one click away under ⋯ and ⌘K.

**Which core.** The UI talks to the Arena core (arenad) it is served from. A build can name another
with `VITE_ARENA_URL=<url>`, and the address can switch with `?core=<url>` (this browser remembers it;
`?core=` with nothing goes back to the serving core); every `/api` request and the event stream go
there (`src/core.ts`), and the ⋯ menu says which core is in use. A UI served from another
origin needs the core started with `ARENA_CORS_ORIGIN=<the UI's origin>`.

Pause and Stop are arenad's (`POST /api/branches/{id}/pause`, `/resume`, `/stop`): while paused, the
branch's tool calls and gateway model calls wait and the world clock stops; `pause` and `resume` are
events, drawn on the timeline as a shaded band.

## References

- **High-performance HMI (ISA-101).** Grey surfaces, simple shapes for equipment. Colour appears only
  for abnormal states, and always comes with a glyph and a word. The process view and every status mark
  follow this.
- **Honeycomb's agent timeline.** One chronological sequence with a swim lane per agent and handoffs
  drawn between lanes. Clicking a step opens its detail. The run view's action timeline follows this.
- **Trace waterfalls** (Grafana, Elastic APM, OpenTelemetry). Time runs left to right, spans nest, and
  gaps mean waiting or unseen work. The run view's trace of the selected turn follows this.
- **Developer tools** (Linear, Vercel). Dense, calm type, a command palette, keyboard navigation, and
  empty states that say what to do next.

## Tokens

Tokens are defined once, in `src/styles/index.css`, as CSS variables for each theme (light by default,
dark under `[data-theme='dark']`). They are exposed to Tailwind v4 through `@theme`. Components use the
Tailwind names and never raw colours.

| Group | Tailwind names | Use |
|---|---|---|
| Surfaces | `bg`, `surface`, `raised`, `sunken`, `hover` | canvas, panels, controls, wells and code, row hover |
| Lines | `line`, `line-strong` | hairlines, strong borders and lane bars |
| Text | `fg`, `muted`, `faint` | body, secondary, tertiary |
| Accent (the only one) | `accent`, `accent-fg`, `accent-soft` | focus, selection, the primary action |
| Abnormal only | `alarm`, `alarm-soft`, `warn`, `warn-soft` | refused, failed, unsafe; approximate restore |
| Result ink | `ok` | a passed run's square, a bar of passes (an ink, never a colour) |
| Process view | `equip`, `equip-line`, `flow`, `flow-strong` | equipment shapes; read flows and write flows |

- **Type.** Inter (variable) for text and JetBrains Mono (variable) for ids, numbers and JSON, both
  bundled with `@fontsource-variable`; figures use Inter's tabular digits, mono is for ids, code and
  the timeline axis. The scale is `text-2xs` 11 px (labels), `text-xs` 12, `text-sm` 13 (body),
  `text-base` 14, `text-lg` 16, `text-xl` 20 (page titles) and `text-2xl` 28 (a result's headline).
- **Spacing.** Tailwind's 4 px scale. Pages use a 16 px gutter on phones and 24 px from `sm`, and one
  of four widths (`PageHeader`/`PageBody` `width`: narrow 768, default 1024, wide 1280, full). Panels
  are spaced 16 px apart. Table rows are 32 px tall.
- **Radii.** `sm` 3 px for chips, `md` 4 px for controls, `lg` 6 px for panels and dialogs.
- **Theme.** System, light or dark, chosen with the top bar button or the palette and kept in
  `localStorage` (`arena.theme`).

## Components (`src/components/ui`)

These are shadcn-style components: the code is ours, built on Radix primitives where a primitive exists.

- `button`: variants `default`, `primary`, `ghost` and `danger`. `asChild` styles a link as a button.
- `panel`: `Panel` and `PanelHeader` (a title, a meta note, and actions).
- `side-panel`: `SidePanel`, a slim panel beside the main area (a step, an agent's prompt).
- `page`: `PageHeader` (title, state, actions, children; no border or panel) and `PageBody`. `Stat` is a label
  over a tabular value, with an `abnormal` option.
- `status`: `StatusMark` shows a glyph and a word, and adds colour only when `abnormal`. `Tag` is a
  neutral label, with optional `warn` or `alarm` tones.
- `state`: `EmptyState`, `ErrorState` (says what to try and has a retry), `LoadingState` (skeleton rows
  and what is loading), and `Notice`.
- `input`: `Input`, `Textarea`, `NativeSelect`, `Checkbox` and `Field` (a label above its control).
- `table`: dense table primitives. `data-table` is a generic TanStack Table with sorting, a filter
  function, ↑/↓ or j/k, Home/End and Enter, and its own loading, error, empty and no-match states.
- `filter-chips`: one-of chips with counts. `segmented`: a few exclusive choices side by side.
- `tabs`, `tooltip`, `popover`, `dialog` (with `DialogActions`) and `dropdown-menu`: Radix.
- `command`: cmdk, used by the palette. `toaster`: sonner.
- `split`: a main area and a side panel that can be resized by dragging or with ←/→ on the handle.
  Its width is kept per page. The two stack on narrow screens.
- `chart`: `Sparkline` and `LineChart` on uPlot, which follow the theme and the box width.

Logic lives in pure modules with tests, and components only render. For example: `lib/keys.ts`
(keyboard moves), `lib/split.ts`, `lib/chart.ts`, `app/palette.ts`, `runs/runsTable.ts`,
`run/timelineLayout.ts`, `run/zoom.ts`, `run/flow.ts`, `run/result.ts`, `run/stepSummary.ts`,
`run/waterfall.ts`, `run/runStats.ts`, `board/board.ts`, `board/importTeam.ts`, `home/summary.ts`, `control/flowLayout.ts`,
`control/permit.ts`, `experiments/model.ts`, `lib/format.ts` and `lib/usePolled.ts` (one way to poll).

## Patterns

- **Words.** Labels start with a capital, no all-caps; values start with a capital (Passed, Failed, Read,
  Write, Refused, One after another, Claude Code). No ids, hashes, step numbers or image tags on the
  Experiments, Runs or Run once: they live behind Details and on the run page. Explanations are tooltips,
  not sentences on the screen.
- **Status.** A status is always a glyph and a word: `✓ Passed`, `◐ Running`, `‖ Paused`, `✕ Failed`,
  `⚠ Infra error`, `■ Interrupted`, `■ Stopped`, `◆ unsafe`. Only abnormal states are coloured.
- **Action timeline** (`run/ActionTimeline.tsx`). There is one lane per agent, plus World (links and
  world events) and Check. Marks are placed on one time axis. The mouse moves it (`run/zoom.ts`):
  the wheel scrolls sideways when there is more to see (else the page scrolls), Ctrl/⌘ + wheel or a
  trackpad pinch zooms around the cursor (fit up to 64×), and a drag on empty space pans. A live run
  follows its newest step until you scroll or zoom away; **Follow** brings it back. Pauses are shaded
  bands. There is no toolbar row: a faint **⋯** over the ruler's right end holds what the marks
  mean, Model calls on or off, and −, Fit and + zoom. Lanes show the agent's name; its model is the
  tooltip.
  - Shapes: hollow circle for a read, solid square for a write (heavier), diamond for approvals, and
    an alarm-coloured triangle for a refusal or error. A model call is a thin tick.
  - Labels: an action carries its system kind ("ERP") where the label fits. The selected mark shows
    the full "ERP write".
  - Messages and handoffs are drawn as arrows between lanes.
  - ←/→, ↑/↓, j/k and h/l move through the steps.
  - The Event log tab lists the same steps as rows.
- **Trace waterfall** (`run/Waterfall.tsx`). It shows the selected step's turn: the agent's model call
  and every step up to its next model call. Each bar ends when its step was recorded and starts that
  step's `duration_ms` earlier. Arena's own work in a step (gate check, recording, save) is listed
  under it.
- **Inspector.** This is the step in full: the system it touched and its effect, the permit the write
  ran under (or the change it named that approvals never recorded), the arguments, reads and writes,
  the world rows at that step, and the result or error. Change and rerun opens in the same panel.
- **Process view** (`control/ProcessDiagram.tsx`, React Flow). Agents are units on the left and the
  world's systems are equipment on the right. Each kind of system has a simple shape: a cylinder for
  data stores, a box for applications, a diamond for approvals, a hexagon for APIs and a truck for
  logistics.
  - Flows: writes are heavier and solid, reads-only flows are thin and dashed. An abnormal flow is
    alarm-coloured and labelled ("⚠ 1 refused").
  - On phones the diagram scrolls sideways.
- **World view** (`pages/World.tsx`, `/world/:runId/:branchId`). The twin's title and the run's
  status (the task and disruption are the title's tooltip), the KPIs as plain figures, the scrubber,
  and the benchmark's digital twin
  (`world_view` events, `GET /api/branches/{id}/views`) as an HMI overview: areas as zones left to right
  in process order (`world/layout.ts`; no item counts or alarm badges on the zones, the tiles say
  it), approvals as a dashed rail, items as tiles with a glyph and a word, and lot-to-order flows as
  pipes along a bus under the zones between them, one labelled lane each (ids in the tooltip). Tiles changed since the view before get an accent ring and a "what changed" line
  (`world/twin.ts`). The scrubber (`world/scrub.ts`) stops at each view, marks disruptions (▼) and
  refusals (▲; what each is in its tooltip), plays and pauses (space), and moves with ←/→. The run's ⋯ → Advanced opens the run page's fork
  panel at the next agent step; "Swap architecture" (`run/architecture.ts`): Team v2 adds its
  rules to every agent's prompt; the single agent replaces the team (a fork's `team`, which only a run
  an external system drives can take). The selected
  step is `?step=` in the address, shared by the run page, the process view and the world view. On
  phones the zones stack and flows are listed.
- **Team board** (`board/`, React Flow). Any team as a read-only Sculptured-style board: agents as
  cards (name, a one-line job from the prompt, model badge, Read-only, the decisions it owns), group
  zones and "hands work to" arrows from a Sculptured design; any other team laid out from its runner
  (in order: rows of three left to right with an arrow to the next; at once: one row;
  `board/board.ts`). It opens readable: a board that only fits below 80% opens at 80% from its top-left corner. The wheel pans, Ctrl/⌘ + wheel or a pinch zooms, a drag pans; −, Fit and + zoom by button and **Expand** opens it full screen (these show on hover). A card shows its prompt beside the board. **Import** (`board/importTeam.ts`) takes a
  Sculptured design (`TeamDraft`: `claude` becomes Claude Code, `codex` Codex; the model from the
  agent, its group or the team; places, groups and hands-work-to connections kept; agents put in work
  order; an agent whose `data_access.writes` is empty is read-only) or an Arena team, picked, dropped
  or pasted. A kind Arena can't run (OpenCode) is refused by name. The board is kept with the team in
  this browser and never sent to arenad. **Edit** opens the team editor (`/team`).
- **Scenario builder** (`scenario/`). On Run once and New experiment as a picker, from the benchmark's
  `scenario_options` (`GET /api/benchmark`): a toggle and parameters per disruption (label and
  description filled with them, as arenad fills the template), and a plain-words preview.
- **Outcome comparison** (`compare/`). Under What matters on the compare page: valid plan, safety
  violations, orders on time and transport cost (the twin's `orders_covered` and `transport_cost`
  KPIs at the end), agent cost (as the agents reported it, labelled reported, and illustrative for
  scripted usage) and escalations for both branches (what each row means is its tooltip); the
  divergence strip on one step axis is under Details. World
  views are not steps: the timeline, run figures, process view and divergence skip them.
- **Fixtures.** A build with `VITE_ARENA_FIXTURES=1` answers `/api` from the pharma fixtures
  (`dev/fixtureApi.ts`), for screenshots without arenad.
- **Trends.** Actions per minute for each system kind, tokens, cost and tool latency as sparklines,
  and context per model call as a chart. A cost the record does not give per call draws no line.
- **Data tables.** Rows open with a click or Enter; ⌘K finds any run or experiment.
- **Command palette.** Opens with ⌘K or Ctrl+K. It lists the pages (Experiments, Runs, Team
  editor); the deeper views of the run on screen; New experiment and Run once; compare two runs,
  picked in two steps (a run's ⋯ opens it at the second); the theme; and every experiment and run
  branch.
- **Status bar.** Only what needs attention, otherwise nothing: no model to run agents with (no agent
  CLI and no API key, with each provider, its reason and Check one click away), or saves 90% full or
  more, as a small analog bar that turns to alarm when new runs are refused. It has the class
  `status-line`, which the capture scripts use to hide it.
- **Empty, loading and error states.** Every list and panel has its own, and each says what to do
  next.
