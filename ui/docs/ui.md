# Arena UI: look, tokens, components, patterns

Arena's web UI is a developer and operations tool: dense, calm, and readable at a glance.

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
| Process view | `equip`, `equip-line`, `flow`, `flow-strong` | equipment shapes; read flows and write flows |

- **Type.** Inter (variable) for text and JetBrains Mono (variable) for ids, numbers and JSON, both
  bundled with `@fontsource-variable`. The scale is `text-2xs` 11 px (labels in caps), `text-xs` 12,
  `text-sm` 13 (body), `text-base` 14, `text-lg` 16 (page titles) and `text-xl` 20.
- **Spacing.** Tailwind's 4 px scale. Pages use a 12 px gutter on phones and 16 px from `sm`. Panels
  are spaced 12 px apart. Table rows are 32 px tall.
- **Radii.** `sm` 3 px for chips, `md` 4 px for controls, `lg` 6 px for panels and dialogs.
- **Theme.** System, light or dark, chosen with the top bar button or the palette and kept in
  `localStorage` (`arena.theme`).

## Components (`src/components/ui`)

These are shadcn-style components: the code is ours, built on Radix primitives where a primitive exists.

- `button`: variants `default`, `primary`, `ghost` and `danger`. `asChild` styles a link as a button.
- `panel`: `Panel` and `PanelHeader` (a title in caps, a meta note, and actions).
- `page`: `PageHeader` (title, state, actions, meta line, children) and `PageBody`. `Stat` is a label in
  caps over a mono value, with an `abnormal` option.
- `status`: `StatusMark` shows a glyph and a word, and adds colour only when `abnormal`. `Tag` is a
  neutral label, with optional `warn` or `alarm` tones.
- `state`: `EmptyState`, `ErrorState` (says what to try and has a retry), `LoadingState` (skeleton rows
  and what is loading), and `Notice`.
- `input`: `Input`, `Textarea`, `NativeSelect`, `Checkbox` and `Field` (a label above its control).
- `table`: dense table primitives. `data-table` is a generic TanStack Table with sorting, a filter
  function, ↑/↓ or j/k, Home/End and Enter, and its own loading, error, empty and no-match states.
- `filter-chips`: one-of chips with counts.
- `tabs`, `tooltip`, `popover`, `dialog` (with `DialogActions`) and `dropdown-menu`: Radix.
- `command`: cmdk, used by the palette. `toaster`: sonner.
- `split`: a main area and a side panel that can be resized by dragging or with ←/→ on the handle.
  Its width is kept per page. The two stack on narrow screens.
- `chart`: `Sparkline` and `LineChart` on uPlot, which follow the theme and the box width.

Logic lives in pure modules with tests, and components only render. For example: `lib/keys.ts`
(keyboard moves), `lib/split.ts`, `lib/chart.ts`, `app/palette.ts`, `runs/runsTable.ts`,
`run/timelineLayout.ts`, `run/waterfall.ts`, `run/runStats.ts`, `control/flowLayout.ts`,
`control/permit.ts` and `control/suiteList.ts`.

## Patterns

- **Status.** A status is always a glyph and a word: `✓ passed`, `◐ running`, `✕ failed`,
  `⚠ infra error`, `■ interrupted` or `finished`, `◆ unsafe`. Only abnormal states are coloured.
- **Action timeline** (`run/ActionTimeline.tsx`). There is one lane per agent, plus World (links and
  world events) and Check. Marks are placed on one time axis and the track can be zoomed (fit up to
  32×).
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
- **World view** (`pages/World.tsx`, `/world/:runId/:branchId`). The benchmark's digital twin
  (`world_view` events, `GET /api/branches/{id}/views`) as an HMI overview: areas as zones left to right
  in process order (`world/layout.ts`), approvals as a dashed rail, items as tiles with a glyph and a
  word, and lot-to-order flows as pipes along a bus under the zones between them, one labelled lane
  each. Tiles changed since the view before get an accent ring and a "what changed" line
  (`world/twin.ts`). The scrubber (`world/scrub.ts`) stops at each view, marks disruptions (▼) and
  refusals (▲), plays and pauses (space), and moves with ←/→. Rewind here opens the run page's fork
  panel at the next agent step with "Swap architecture" chosen (`run/architecture.ts`): Team v2 adds its
  rules to every agent's prompt; the single agent replaces the team (a fork's `team`, which only a run
  an external system drives can take). The selected
  step is `?step=` in the address, shared by the run page, the process view and the world view. On
  phones the zones stack and flows are listed.
- **Scenario builder** (`scenario/`). In the New run and New suite dialogs, from the benchmark's
  `scenario_options` (`GET /api/benchmark`): a toggle and parameters per disruption (label and
  description filled with them, as arenad fills the template), and a plain-words preview.
- **Outcome comparison** (`compare/`). At the top of the compare page: valid plan, safety
  violations, orders on time and transport cost (the twin's `orders_covered` and `transport_cost`
  KPIs at the end), agent cost (as the agents reported it, labelled reported, and illustrative for
  scripted usage) and escalations for both branches, and a divergence strip on one step axis. World
  views are not steps: the timeline, run figures, process view and divergence skip them.
- **Fixtures.** A build with `VITE_ARENA_FIXTURES=1` answers `/api` from the pharma fixtures
  (`dev/fixtureApi.ts`), for screenshots without arenad.
- **Trends.** Actions per minute for each system kind, tokens, cost and tool latency as sparklines,
  and context per model call as a chart. A cost the record does not give per call draws no line.
- **Data tables.** Lists use filter chips with counts plus a text filter, and rows open with a click
  or Enter.
- **Command palette.** Opens with ⌘K or Ctrl+K. It lists the pages; new run and new suite (which open
  their dialogs through `?new=run` and `?new=suite`); compare two runs or two suites, picked in two
  steps; the theme; and every run branch and suite.
- **Status bar.** It shows the agent CLIs found, one calm line for API mode ("0 of 3 ready: no API key
  set", with each provider, its reason and Check one click away), and saves on disk as a small
  analog bar that turns to alarm only when new runs are refused. It has the class `status-line`,
  which the capture scripts use to hide it.
- **Empty, loading and error states.** Every list and panel has its own, and each says what to do
  next.
