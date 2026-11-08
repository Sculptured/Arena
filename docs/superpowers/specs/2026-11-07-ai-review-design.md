# AI review: what went wrong in a run, and what matters between two runs (2026-11-07)

The user asked on 2026-11-07: "add a feature (optional, can be turned on and off) to have an LLM go through the
details of the run and identify where it went wrong, and suggestions to fix, and an LLM which shows you the
major differences when comparing runs so its not just a ton of data to go through".

## Hard rule
Real models only. The reviewer is an actual LLM; no canned, scripted or fake reviewer anywhere, tests included
(unit-test the digest, prompt assembly, parsing and storage on recorded data; test the reviewer itself live).

## 1. Run review
- Input: a compact, deterministic **digest** of the branch built by arenad (not the raw record): the task and
  ticket in brief; the team and runner; the checks (each passed/failed with its reason, safety flagged); refused
  and failed calls with their errors; disruptions; the agents' key writes (the world's write tool calls, args
  trimmed); world rows that ended different from the expected result (the compare/score output); metrics
  (time, tokens, cost); for a fork, its parent and fork point. Each item carries its step `seq`. Bounded size
  (cap and summarise long lists), so a review is cheap.
- Output (JSON, validated): `{verdict: one sentence, went_wrong: [{title, why, steps: [seq], severity:
  "safety"|"goal"|"efficiency"}], suggestions: [{change, why, kind: "prompt"|"team"|"tool"|"model"|"process",
  agent?}], went_well: [short], confidence}`. Every claim cites steps that exist in the branch; drop or flag
  any citation that doesn't.
- Passed runs get a short review too (what went well, efficiency suggestions).

## 2. Comparison review
- Input: both digests plus the comparison Arena already computes (outcome rows, first divergence, differing
  world rows, metrics deltas, suite verdicts for suites).
- Output: `{headline, differences: [{title, a, b, why_it_matters, steps_a: [seq], steps_b: [seq]}] (the few
  that matter, most important first, max ~6), which_is_better: "a"|"b"|"mixed", why}`. Suites: the same over
  the suite comparison (pass rates, checks that moved, safety), citing example runs.

## 3. On/off, model, storage
- A setting in arenad (`ARENA_REVIEW=on|off`, default on when a model is available) and in the UI (Advanced →
  "AI review", saved per browser; the UI only requests reviews when on). Off means nothing is sent to a model.
- Reviewer model: Claude Code headless (`claude -p`, JSON output, no tools) with Sonnet by default; or an API
  provider via arenad's providers when configured (`ARENA_REVIEW_MODEL`, e.g. `claude_code:sonnet`,
  `openrouter:<model>`). Record which model and the cost of each review.
- Routes: `POST /api/branches/{id}/review` (start or return the stored one), `GET` to read it;
  `POST /api/compare/review?a=&b=` and the suite equivalent. A review is stored with the branch (or the pair)
  and not recomputed unless asked (`?again=1`). Live runs: review when the run ends (auto when on).
- SDK: `branch.review()`, `arena.compare_review(a, b)`, `arena.compare_suites_review(a, b)`.

## 4. UI
- Run screen result banner and the run page: a "Review" card: the verdict, "Where it went wrong" items (each
  step clickable → selects it on the timeline/twin), "Suggestions" (with a "Rewind and try this" link that opens
  the fork panel at the cited step with the suggested prompt change pre-filled when the suggestion is a prompt
  or team change), "What went well". Shows "Reviewing…" while it runs; model and cost in small print.
- Compare page (runs and suites): a "What matters" card above the outcome table: headline and the few
  differences, each with step links for A and B.
- Calm, uncluttered, capitalised, in keeping with the simplified UI; hidden entirely when AI review is off.

## Verification
Unit tests for digest building (on recorded runs in the repo's test data), output validation and citation
checking, storage and the on/off rule; a live test (haiku) that a review returns valid JSON citing real steps.
Then real reviews (Sonnet) of: a failed pharma run, the single-agent run, and their comparison; show
screenshots.
