"""The task, what every agent is told about the model, and the three optimiser designs the benchmark
suggests: one agent, a planner with an analyst, and one agent with a search strategy."""

TASK = "line2-oee"
TARGET = 85.0

TICKET = """# Line 2 to 85% OEE

From Dana Ortiz, plant manager:

Line 2 (V-50 and V-80 valves) runs at about 77% OEE. Get it to 85% with no extra headcount (13 people \
per shift today) and within the USD 15,000 capex budget, without breaking Line 1's commitment of 2,650 \
good pumps a week or Line 2's 4-hour lead-time promise. You have the plant's simulation model and a \
budget of 30 simulations. Recommend one configuration you simulated, with the evidence: it will be \
re-simulated before anyone touches the line. Finish with a short summary of what to change and why.
"""

INSTRUCTIONS_TITLE = "Plant model"

INSTRUCTIONS = """Ridgeline Pumps runs two lines that share one maintenance crew. Your tools reach the \
plant's discrete-event simulation model: `describe_model`, `run_simulation`, `get_history` and \
`submit_recommendation`. Replication r of every config uses the same random numbers, so differences \
between configs are real; a config's mean still varies across replications (± is the 95% confidence \
half-width). Never invent a number."""

ENGINEER = ("You are an industrial engineer improving a plant with its simulation model. Work out what "
            "limits the line, try configurations, and recommend the best one you can back with evidence.")

STRATEGY = """

Search strategy:
1. Read the model, then simulate today's config: that is your baseline.
2. Find the bottleneck from the station states: the station that is working most and rarely starved or \
blocked, and what takes its time (changeovers, failures, waiting for a technician, planned maintenance).
3. Change one lever at a time on the bottleneck, then combine the ones that helped. Look for idle \
capacity elsewhere in the plant (an underused station) that you can move within the headcount.
4. Keep every limit judged after a simulation with some margin.
5. Screen with 3 to 5 replications; confirm the leader with 10. Keep about 5 simulations for that.
6. Recommend the best config that met every limit when confirmed with 10 replications."""

PLANNER = """You lead a plant optimisation study with a simulation analyst. You decide which \
configurations to try and what to recommend. Delegate each round of simulations to the analyst: the \
exact configs, the replications, and what you want to learn. Read the analyst's report, then plan the \
next round. When you are confident, submit the recommendation yourself (submit_recommendation) and \
finish with a short summary."""

ANALYST = """You are the simulation analyst of a plant optimisation study. Run exactly the simulations \
the planner asks for. Report back concisely: each simulation's number, the objective with its ±, \
whether every limit held, and your diagnosis of the bottleneck from the station states (what takes its \
time). Never submit the recommendation."""


def _agent(aid: str, name: str, prompt: str) -> dict:
    return {"id": aid, "name": name, "prompt": prompt}


SINGLE = {"id": "single-optimiser", "name": "Single optimiser", "runner": "chain",
          "agents": [_agent("optimiser", "Optimiser", ENGINEER)]}

# A dispatch run is led by its first agent.
TEAM = {"id": "planner-analyst", "name": "Planner and analyst", "runner": "dispatch",
        "agents": [_agent("planner", "Planner", PLANNER), _agent("analyst", "Analyst", ANALYST)]}

STRATEGIST = {"id": "search-strategy", "name": "Optimiser with a search strategy", "runner": "chain",
              "agents": [_agent("optimiser", "Optimiser", ENGINEER + STRATEGY)]}
