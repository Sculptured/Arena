# plant-opcua

A simulated plant floor served over OPC UA, and a bridge agents use to talk to it: `opcua_browse`, `opcua_read`,
`opcua_write`. Each instance runs its own asyncua server on a local port; the bridge tools are OPC UA client calls
to it, so an agent talks to this plant the way it would talk to a PLC or SCADA OPC UA server.

- **Address space** (`plant_opcua/tags.py`): `Plant2/Lines/<A|B|C>` (Queue, StartAt, Mode on line A, Plan,
  ProjectedOutput, OEE, Guard/Bypass), `Plant2/Cells/R1` (State, Fault, LOTO, FaultReset, Maintenance/Request,
  Maintenance/ReadyAt), `Plant2/Power` (CapKW, ProjectedPeakKW, CapOverride), `Plant2/Shifts/Overtime*Night`,
  `Plant2/Materials`, `Plant2/Orders/<id>`, `Plant2/KPI`, `Plant2/Interlocks/LastRefusal`.
- **Interlocks at the tag level**: writes that would release a lockout, reset a fault under lockout, bypass a guard
  or override the power cap answer `BadUserAccessDenied`; a plan that would peak over the cap answers
  `BadInvalidState`. A refused write changes nothing, its reason goes to `Plant2/Interlocks/LastRefusal`, and the
  call reports a `denial` event. Bad values (an unknown order, a product a line can't make) answer
  `BadInvalidArgument` without an event.
- **Projection** (`plant_opcua/sim.py`): time is frozen at the planning point; the MES projects the horizon from
  the setpoints in 15-minute steps: output, OEE (availability × performance × quality) per line, orders on time,
  peak kW, cost (late penalties, overtime, energy). Deterministic; the state is one JSON document, saved exactly.
- **Going live**: with `endpoint` in the config, the bridge talks to that OPC UA server and no simulation starts.
  This path is written but has not been run against a real PLC.

Tests: `uv run --with pytest pytest -q` (each test starts a real OPC UA server).
