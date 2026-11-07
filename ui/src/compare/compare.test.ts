import { describe, expect, it } from 'vitest';
import type { CompareSide, Event } from '../types';
import { FORK_SEQ, comparison, v1Detail, v2Detail } from '../world/fixtures';
import { axisTicks, divergenceStrip } from './divergence';
import { escalations, lastView, leadingNumber, ordersOnTime, outcomeRows, safetyViolations, transportCost, validPlan, verdict } from './outcomes';

const ev = (seq: number, kind: Event['kind'], payload: unknown, agent: string | null = null): Event => ({ branch_id: 'b', seq, at: '', agent, kind, payload });

describe('outcome comparison', () => {
  const rows = outcomeRows({ detail: v1Detail, side: comparison.a }, { detail: v2Detail, side: comparison.b });
  const row = (id: string) => rows.find((r) => r.id === id)!;

  it('reads every outcome of both branches', () => {
    expect(rows.map((r) => [r.label, r.a.text, r.b.text])).toEqual([
      ['Valid plan', '✕ not valid 8/9', '✓ valid 9/9'],
      ['Safety violations', '◆ 1 of 4', '0 of 4'],
      ['Orders on time', '2 of 10', '4 of 10'],
      ['Transport cost', '$3,410', '$4,660'],
      ['Agent cost', 'tokens only · 0 tokens', 'tokens only · 0 tokens'],
      ['Escalations', '3', '3'],
    ]);
    expect(row('valid').a).toMatchObject({ abnormal: true, title: 'Failed: eligible_stock_allocated' });
    expect(row('safety').a.title).toBe('no_short_dated_lot_shipped');
  });

  it('says which side did better, and nothing when equal or unknown', () => {
    expect(['valid', 'safety', 'on_time', 'transport', 'escalations'].map((id) => verdict(row(id)))).toEqual(['b', 'b', 'b', 'a', null]);
    expect(verdict(row('agent_cost'))).toBeNull();
  });

  it('falls back when the run was not scored or drew no twin', () => {
    expect(validPlan([])).toMatchObject({ text: 'not scored', value: null });
    expect(validPlan([ev(2, 'check', { passed: true })])).toMatchObject({ text: '✓ valid', value: 1 });
    expect(safetyViolations([ev(2, 'check', { passed: true, checks: [{ name: 'x', passed: true }] })]).text).toBe('not scored');
    expect(ordersOnTime(null).text).toBe('–');
    expect(lastView([])).toBeNull();
  });

  it('counts orders on time from the customers when the twin has no KPI for it', () => {
    const v = lastView(v1Detail.events)!;
    const noKpi = { ...v, kpis: [] };
    expect(ordersOnTime(noKpi)).toMatchObject({ text: '2 of 10', value: 2, abnormal: true });
  });

  it('adds up booked shipments when the twin has no transport KPI', () => {
    const evs = [
      ev(2, 'tool_call', { name: 'book_shipment', result: { price: 420 }, error: null }, 'l'),
      ev(3, 'tool_call', { name: 'book_shipment', result: { price: 2150 }, error: null }, 'l'),
      ev(4, 'tool_call', { name: 'book_shipment', result: { price: 999 }, error: 'refused' }, 'l'),
    ];
    expect(transportCost(null, evs)).toMatchObject({ text: '$2,570', value: 2570 });
    expect(transportCost(null, []).text).toBe('–');
    expect(leadingNumber('$1,240.50')).toBe(1240.5);
    expect(leadingNumber('none')).toBeNull();
  });

  it('counts approval requests by agents, not the decisions links make', () => {
    expect(escalations(v1Detail.events).value).toBe(3);
  });

  it('reports a known agent cost', () => {
    const side: CompareSide = { ...comparison.a, cost_usd: 0.42, cost_source: 'openrouter', tokens: 1200 };
    const r = outcomeRows({ detail: v1Detail, side }, { detail: v2Detail, side: { ...side, cost_usd: 0.3 } }).find((x) => x.id === 'agent_cost')!;
    expect(r.a.text).toBe('$0.42 (OpenRouter) · 1,200 tokens');
    expect(verdict(r)).toBe('b');
  });
});

describe('divergence strip', () => {
  it('puts both branches on one step axis with the first divergence and their marks', () => {
    const s = divergenceStrip(v1Detail, v2Detail, comparison.first_divergent_seq);
    expect(s.first).toBe(FORK_SEQ);
    expect(s.min).toBe(3);
    expect(s.rows.map((r) => r.side)).toEqual(['A', 'B']);
    expect(s.rows[0].marks.map((m) => m.kind)).toEqual(['refusal', 'disruption']);
    expect(s.rows[1].marks.map((m) => m.kind)).toEqual(['fork', 'disruption']);
    expect(s.rows[0].end).toBe(s.max);
  });

  it('picks round ticks', () => {
    expect(axisTicks(1, 47)).toEqual([10, 20, 30, 40]);
    expect(axisTicks(0, 10, 5)).toEqual([0, 2, 4, 6, 8, 10]);
    expect(axisTicks(3, 3)).toEqual([3]);
  });
});
