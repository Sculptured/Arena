# Examples

Run from `sdk/python` with `uv run python examples/<name>.py`. Each connects to the arenad at `ARENA_URL`, or
starts one (`Arena.local`) from `ARENA_BIN_DIR` (default `core/target/release`) on the Postgres at `ARENA_PG_URL`
with the benchmark `ARENA_BENCHMARK` (default `retail`); see `common.py`. Use `ARENA_URL` when one example
should see another's runs (a started arenad stops when its script ends).

Agents are always real models: Claude Code (`claude` signed in) by default, or an API provider whose key is set
in arenad's environment. An example that needs what isn't there says so and stops. They cost credits; the
defaults use the cheapest Claude model (`haiku`).

| Example | What it shows | Model calls |
|---|---|---|
| `quickstart.py [task] [model]` | Run the benchmark's team of Claude Code agents, stream the timeline live, print the checks and metrics | yes |
| `bring_your_own_agent.py [task] [claude-code\|openai\|openrouter\|anthropic] [model]` | An external run your code drives: Claude Code on the agent's MCP URL, or a minimal tool loop over the gateway | yes |
| `counterfactual_fork.py [branch] [model]` | Fork at the first write with a changed prompt, the world restored; compare the branches | yes |
| `suite.py [tasks] [repeats] [model] [compare_with]` | A suite: pass rates, safety, spread; compare two suites | yes |
| `export_dataset.py <branch\|suite> [out]` | Export a run (with its forks) or a suite as JSONL and read it back | no |
| `step_env.py [task] [max_steps]` | `ArenaParallelEnv` with a model as the policy (Claude API if `ANTHROPIC_API_KEY`, else Claude Code); then "reset to step N" with a fork | yes |

What these printed when last run (retail task 72, `haiku`, 2026-11-07): quickstart passed (20 events, 134k
tokens, $0.008 reported); bring-your-own-agent with Claude Code made 5 read calls and failed the task; the fork of
the quickstart run at its first write moved to the resolver's start (a CLI session restarts from the agent's start)
and passed again; a one-run suite failed (pass rate 0.0); the step-loop policy made 6 read calls, ended, and was
scored failed. Models vary; that is what Arena measures.
