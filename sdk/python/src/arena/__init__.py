"""Arena from Python: build environments, run teams, observe, control and export.

    from arena import Arena, Agent

    arena = Arena("http://127.0.0.1:4510")          # or: with Arena.local(benchmark="retail") as arena:
    branch = arena.run("72", team=[Agent.claude_code("support", model="haiku")])
    for event in branch.stream():
        print(event.summary())
    print(branch.check())

See docs/sdk.md in the Arena repository.
"""

from .client import DEFAULT_URL, AgentEndpoint, Arena, Branch, Suite
from .errors import (
    ApiError,
    ArenaError,
    BadRequest,
    Conflict,
    ConnectionFailed,
    DiskFull,
    NotFound,
    NotSupported,
)
from .mcp import Tool, ToolResult
from .models import (
    ENDED,
    Agent,
    Benchmark,
    BranchRecord,
    Change,
    CheckResult,
    Disruption,
    Event,
    ForkInfo,
    NamedCheck,
    RunSummary,
    Scenario,
    Task,
    Team,
)

__version__ = "0.1.0"

__all__ = [
    "Arena", "Branch", "AgentEndpoint", "Suite", "DEFAULT_URL",
    "Agent", "Team", "Change", "Scenario", "Disruption", "Task", "Benchmark", "Event",
    "BranchRecord", "CheckResult", "NamedCheck", "RunSummary", "ForkInfo", "ENDED",
    "Tool", "ToolResult",
    "ArenaError", "ApiError", "BadRequest", "NotFound", "Conflict", "DiskFull",
    "ConnectionFailed", "NotSupported",
]
