"""plant-floor: the plant-floor benchmark (docs/protocol.md, "Benchmark calls")."""
from arena_part import serve_benchmark

from .bench import PlantFloor

serve_benchmark(PlantFloor())
