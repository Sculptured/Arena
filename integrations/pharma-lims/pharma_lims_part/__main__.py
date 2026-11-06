"""pharma-lims: a simulated LIMS and QA head as an arena-part/1 integration."""
from arena_part import serve

from .part import PharmaLims

serve(PharmaLims())
