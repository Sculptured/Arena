"""factory-01: the Automation Fair benchmark (docs/protocol.md, "Benchmark calls")."""
from arena_part import serve_benchmark

from .bench import Factory01

serve_benchmark(Factory01())
