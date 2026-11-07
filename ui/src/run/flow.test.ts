import { describe, expect, it } from 'vitest';
import type { Event } from '../types';
import { clockText, flowMark, flowOf, pausedIn, pausesOf } from './flow';
import { humanize, resultOf } from './result';

const ev = (seq: number, kind: Event['kind'], at: string, payload: unknown = {}): Event => ({ branch_id: 'b', seq, at, agent: null, kind, payload });
const t = (s: number) => new Date(Date.UTC(2026, 10, 7, 12, 0, s)).toISOString();

describe('run flow', () => {
  it('derives paused from the last pause or resume, unless the server says', () => {
    const events = [ev(1, 'run_start', t(0)), ev(2, 'pause', t(5)), ev(3, 'resume', t(9)), ev(4, 'pause', t(12))];
    expect(pausedIn(events)).toBe(true);
    expect(pausedIn(events.slice(0, 3))).toBe(false);
    expect(flowOf({ status: 'running', events })).toBe('paused');
    expect(flowOf({ status: 'running', events, paused: false })).toBe('running');
    expect(flowOf(null)).toBe('starting');
    expect(flowOf({ status: 'infra_error', events: [] })).toBe('error');
    expect(flowOf({ status: 'stopped', events })).toBe('stopped');
    expect(flowMark('paused').word).toBe('Paused');
    expect(flowMark('failed').abnormal).toBe(true);
  });

  it('lists each pause, an open one ending now', () => {
    const events = [ev(4, 'pause', t(12)), ev(2, 'pause', t(5)), ev(3, 'resume', t(9))];
    const now = Date.parse(t(20));
    expect(pausesOf(events, now)).toEqual([
      { start: Date.parse(t(5)), end: Date.parse(t(9)), open: false },
      { start: Date.parse(t(12)), end: now, open: true },
    ]);
    expect(pausesOf([], now)).toEqual([]);
  });

  it('writes elapsed time short', () => {
    expect([clockText(4.6), clockText(65), clockText(3725), clockText(null)]).toEqual(['4 s', '1 min 05 s', '1 h 02 min', '–']);
  });
});

describe('result banner', () => {
  const check = (payload: unknown) => [ev(9, 'check', t(30), payload)];
  it('names the first safety violation before any other failed check', () => {
    const checks = [
      { name: 'batch_held', passed: false, detail: 'CVX-2611 is not on hold' },
      { name: 'no_expired_lot_shipped', passed: false, safety: true, detail: 'lot CVX-2550 expired' },
    ];
    expect(resultOf({ status: 'failed', events: check({ passed: false, checks }), error: null })).toEqual({
      flow: 'failed', title: 'Failed', why: 'Safety violation: No expired lot shipped: lot CVX-2550 expired.',
    });
    const goal = [{ name: 'batch_held', passed: false, detail: 'CVX-2611 is not on hold' }];
    expect(resultOf({ status: 'failed', events: check({ passed: false, checks: goal }), error: null })?.why).toBe('Batch held: CVX-2611 is not on hold.');
  });

  it('says why in one sentence for every ending, and nothing while it runs', () => {
    const all = [{ name: 'a', passed: true }, { name: 'b', passed: true }];
    expect(resultOf({ status: 'passed', events: check({ passed: true, checks: all }), error: null })?.why).toBe('All 2 checks passed.');
    expect(resultOf({ status: 'failed', events: check({ passed: false, differing_rows: ['x/1'] }), error: null })?.why).toBe('1 record differs from the expected result.');
    expect(resultOf({ status: 'failed', events: [], error: 'the model gave up' })?.why).toBe('The model gave up.');
    expect(resultOf({ status: 'stopped', events: [], error: null })?.title).toBe('Stopped');
    expect(resultOf({ status: 'infra_error', events: [], error: 'erp stopped' })).toEqual({ flow: 'error', title: 'Error', why: 'Erp stopped.' });
    expect(resultOf({ status: 'running', events: [], error: null })).toBeNull();
    expect(humanize('no-order_shipped_twice')).toBe('No order shipped twice');
  });
});
