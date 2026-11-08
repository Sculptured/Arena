"""utility: a simulated electric utility with the site's meter and its demand-response events."""
from arena_part import serve

from .part import Utility

serve(Utility())
