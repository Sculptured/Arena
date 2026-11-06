// A small recorded run for the control views' tests: a coordinator asks quality to open a
// deviation; quality reads the LIMS, writes the QMS, asks for an approval, and has one write refused.
import type { Event, EventKind, Team } from '../types';

export const team: Team = {
  id: 't', name: 'T',
  agents: [
    { id: 'lead', name: 'Coordinator', kind: 'external', model: null, prompt: '' },
    { id: 'quality', name: 'Quality', kind: 'external', model: null, prompt: '' },
  ],
};

const t0 = Date.parse('2026-11-05T12:00:00Z');
export const at = (s: number) => new Date(t0 + s * 1000).toISOString();

export const ev = (seq: number, s: number, agent: string | null, kind: EventKind, payload: unknown = {}): Event =>
  ({ branch_id: 'b', seq, at: at(s), agent, kind, payload });

export const runStart = ev(1, 0, null, 'run_start', {
  task_id: 'base',
  world: [{ part: 'erp', integration: 'erpnext' }, { part: 'lims', integration: 'lims' }, { part: 'gate', integration: 'approvals' }],
  systems: {
    erp: { kind: 'erp', product: 'ERPNext', stands_in_for: 'SAP S/4HANA', simulated: false },
    lims: { kind: 'lims', product: 'Arena LIMS', stands_in_for: 'LabWare', simulated: true },
    gate: { kind: 'approvals', product: 'Sculptured approvals', simulated: false },
  },
  tools: {
    get_result: { effect: 'read', part: 'lims' },
    open_deviation: { effect: 'write', part: 'erp' },
    hold_batch: { effect: 'write', part: 'erp' },
    request_approval: { effect: 'write', part: 'gate' },
    note: { effect: 'read' },
  },
});

export const events: Event[] = [
  runStart,
  ev(2, 1, 'lead', 'agent_start'),
  ev(3, 2, 'lead', 'model_call', { usage: { prompt_tokens: 1000, completion_tokens: 50, total_tokens: 1050 }, cost_usd: 0.01 }),
  ev(4, 3, 'lead', 'message', { to: 'quality', text: 'open a deviation for CVX-2611' }),
  ev(5, 4, 'quality', 'agent_start'),
  ev(6, 10, 'quality', 'tool_call', { name: 'get_result', part: 'lims', error: null, duration_ms: 120 }),
  ev(7, 30, 'quality', 'tool_call', { name: 'open_deviation', part: 'erp', error: null, duration_ms: 1500 }),
  ev(8, 70, 'quality', 'tool_call', { name: 'request_approval', part: 'gate', error: null }),
  ev(9, 75, 'quality', 'tool_call', { name: 'hold_batch', part: 'erp', error: 'hold_batch refused: no approval for CVX-2611' }),
  ev(10, 80, 'quality', 'model_call', { usage: { prompt_tokens: 3000, completion_tokens: 100, total_tokens: 3100 }, cost_usd: null }),
  ev(11, 90, 'quality', 'agent_end', { ok: true }),
  ev(12, 95, null, 'check', { passed: false }),
];
