"""factory-01-erp: Factory-01's simulated ERP (stock, reservations, schedule, suppliers, purchasing)."""
from arena_part import serve

from .part import Factory01Erp

serve(Factory01Erp())
