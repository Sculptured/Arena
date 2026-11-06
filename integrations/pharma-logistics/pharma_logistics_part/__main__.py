"""pharma-logistics: simulated cold-chain carriers, customer notices and a supply chain director."""
from arena_part import serve

from .part import PharmaLogistics

serve(PharmaLogistics())
