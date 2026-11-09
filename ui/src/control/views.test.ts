import { describe, expect, it } from 'vitest';
import type { Event } from '../types';
import { actionsOf } from './actions';
import { ev, events, runStart, team } from './fixtures';
import { equipShape, flowGraph, flowLabel, flowStroke } from './flowLayout';
import { missingPermit, permitOf } from './permit';
import { processView } from './process';

describe('flow graph', () => {
  const view = processView(events, actionsOf(events), team);
  const g = flowGraph(view);
  it('puts agents on the left and systems on the right, one edge per flow', () => {
    expect(g.nodes.filter((n) => n.type === 'unit').map((n) => n.id)).toEqual(['u:lead', 'u:quality']);
    expect(g.nodes.filter((n) => n.type === 'equipment').every((n) => n.position.x > 0)).toBe(true);
    expect(g.edges.map((e) => e.id)).toEqual(['quality->lims', 'quality->erp', 'quality->gate']);
  });
  it('draws writes heavier than reads and abnormal flows in the alarm colour with words', () => {
    const [lims, erp] = view.flows;
    expect(flowStroke(lims)).toEqual({ width: 1.25, dashed: true, abnormal: false });
    expect(flowStroke(erp).width).toBeGreaterThan(2);
    expect(flowLabel(erp)).toBe('⚠ 1 refused');
    expect(flowLabel(lims)).toBe('');
    expect(g.edges[1].style?.stroke).toBe('var(--alarm)');
  });
  it('gives each kind of system a simple shape', () => {
    expect(['database', 'lims', 'approvals', 'erp', 'mcp', 'logistics', undefined].map(equipShape)).toEqual(['cylinder', 'cylinder', 'diamond', 'box', 'hexagon', 'truck', 'box']);
  });
});

describe('world links in the process view', () => {
  it('are a unit run by arena, and arena\'s inbox reads are messages, not unknown calls', () => {
    const evs: Event[] = [runStart, ev(2, 1, 'link', 'tool_call', { name: 'qa_review', part: 'lims' }), ev(3, 2, 'quality', 'tool_call', { name: 'inbox' })];
    const v = processView(evs, actionsOf(evs), team);
    expect(v.units.find((u) => u.id === 'link')).toMatchObject({ name: 'World links', state: 'automatic', other: 1 });
    expect(actionsOf(evs)[1]).toMatchObject({ effect: 'read', system: { kind: 'messages' } });
  });
});

describe('permits', () => {
  const proposed = ev(20, 1, null, 'world_event', { kind: 'change_proposed', part: 'gate', data: { id: 'chg-1', action: 'batch_hold' } });
  const decided = ev(21, 2, null, 'world_event', { kind: 'decided', part: 'gate', text: 'QA head approved chg-1 (batch_hold)', data: { id: 'chg-1', approved: true } });
  const write = ev(22, 3, 'quality', 'tool_call', { name: 'hold_batch', args: { batch: 'CVX', proposal_id: 'chg-1' } });
  const unknown = ev(23, 4, 'quality', 'tool_call', { name: 'hold_batch', args: { proposal_id: 'nope' }, error: 'refused' });
  const evs = [runStart, proposed, decided, write, unknown];
  it('a write names the change it runs under, with who decided it', () => {
    expect(permitOf(evs, write)).toEqual({ id: 'chg-1', approved: true, decision: 'QA head approved chg-1 (batch_hold)', action: 'batch_hold' });
    expect(missingPermit(evs, write)).toBeNull();
  });
  it('a change approvals never recorded is a missing permit; decisions after the call do not count', () => {
    expect(missingPermit(evs, unknown)).toBe('nope');
    expect(permitOf([runStart, write, { ...decided, seq: 30 }], write)).toBeNull();
    expect(permitOf([runStart, proposed, write], write)).toEqual({ id: 'chg-1', approved: null, decision: '', action: 'batch_hold' });
  });
});
