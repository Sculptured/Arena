"""line-sim: a discrete-event simulation study of a plant (arena-part/1)."""
from arena_part import serve

from .part import LineSim

serve(LineSim())
