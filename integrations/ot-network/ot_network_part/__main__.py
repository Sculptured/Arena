"""ot-network: a simulated OT network of a plant, with zones, conduits and the IT/OT gateway's remote sessions."""
from arena_part import serve

from .part import OtNetwork

serve(OtNetwork())
