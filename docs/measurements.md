# Measurements with the retail example benchmark

These numbers were measured with the bundled retail example benchmark (`benchmarks/retail`, tau2-bench retail
on one `postgres` part). They describe that benchmark on one machine, not the arena in general. The live
Claude Code and Codex runs were made before the move to integrations and have not been repeated.

## Snapshot copy time

Median 152 ms over 10 snapshots in the Task 4 test; three runs gave 152, 152 and 221 ms. Each snapshot is a
full copy of the 21 MB retail database. The machine was the development laptop.

## Live runs on task 27

2026-10-30, through the HTTP API, one run each. "A" is the default team (Claude Code to Claude Code, resolver
prompt unchanged, model `sonnet`). Seconds, tokens and cost are the compare output. For run A and run B they
cover the whole branch (triage and resolver). For a fork they cover only the steps the fork redid (here the
resolver), because the steps before the fork point are copied, not rerun.

| Run | Status | Seconds | Tokens | Cost |
|---|---|---|---|---|
| A: default team (Task 13) | passed | 23.574 | 104427 | $0.0996256 (api_equivalent_estimate) |
| A fork: resolver prompt changed, forked at step 12 (Task 13) | passed | 8.911 | 36036 | $0.0296177 (api_equivalent_estimate) |
| B: triage Claude Code, resolver Codex (new run, model unset) | passed | 50.708 | 166583 | $0.0407897 (api_equivalent_estimate, triage only, from its agent_end; Codex has no dollar figure, so the compare output shows no dollars for B) |
| C1: fork of A at step 12, resolver kind changed to `codex`, model left as `sonnet` | infra_error | 1.725 | 0 | none |
| C2: same fork, resolver kind `codex` and model `gpt-6.1-sol` | passed | 29.159 | 106955 | none (Codex reports tokens only) |
| API mode (OpenRouter) | not run | | | no `OPENROUTER_API_KEY` set; `open_available` was false |

Notes:
- The two fork rows (and C1/C2) cover the resolver only. For comparison, A's resolver alone was 8.796 s,
  45932 tokens, $0.0508491 (from the agent_end event).
- B and C2 passed the database check (no differing rows against the gold result). The Codex resolver was
  slower than the Claude resolver (34.6 s in B and 28.8 s in C2, against 8.8 s in A; Claude's 8.9 s fork with
  a longer prompt was similar) and used more tokens (107721 in B and 106955 in C2, against 45932). Codex token
  counts include cached input, so they are not comparable one to one with Claude's. One run each, so this is
  an observation, not a benchmark.
- C1 was measured before the fix that makes a kind-only fork reset the model (a kind-only fork now resets the
  model to the new mode's default, so C1 would not fail this way). It failed because Codex with a ChatGPT
  account does not support the model `sonnet`: `The 'sonnet' model is not supported when using Codex with a
  ChatGPT account.` It is not a usage limit. The fork response was
  `{"start_seq":12,"moved_from":null,"reason":null}`, and the branch ended `infra_error`. C2 is the one retry,
  with the model set to the one in the Codex config used.
- Compare, A against C2: `first_divergent_seq` 13, `differing_rows` empty. A against B: `first_divergent_seq`
  13, `differing_rows` empty. A against the Task 13 prompt fork: `first_divergent_seq` 16, `differing_rows`
  empty. A against C1: `first_divergent_seq` 13, one differing row, `orders/#W7181492` (A made the exchange,
  the failed branch did not).
- None of the forks moved (`moved_from` and `reason` were `null`): step 12 is already the start of the
  resolver.
