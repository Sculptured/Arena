import { describe, expect, it } from 'vitest';
import { parseStep, withStep } from '../run/useStepParam';
import { drivenLine, stepHeadline, worldState } from './page';
import { swapChanges, forkChoiceOf, singleTeam, teamSwapProblem, ARCHITECTURES } from '../run/architecture';
import { withRules } from '../team';
import type { Event } from '../types';
import { FORK_SEQ, V1, baseView, team, v1Detail, v2Detail, views } from './fixtures';
import { BUS_LANE, roundedPath, MIN_TILE_W, PROCESS_ORDER, TILE_H, TILE_H_CHANGED, arrowBetween, fitsSideBySide, splitAreas, twinLayout } from './layout';
import { buildTrack, happenings, moveStop, nearestStop, rewindSeq, snapStop, trackAt, trackMarks } from './scrub';
import { isIdLike, diffViews, itemFacts, itemShort, normalizeViews, statusGlyph, statusMark, viewIndexAt, viewsFromEvents } from './twin';

const v1Views = views[V1];

describe('twin views', () => {
  it('reads the views from world_view events, keyed by the step each shows', () => {
    const fromEvents = viewsFromEvents(v1Detail.events);
    expect(fromEvents.map((v) => v.seq)).toEqual(v1Views.map((v) => v.seq));
    expect(fromEvents[0].seq).toBe(1);
  });

  it('orders the endpoint\'s views by the step each follows and drops what is no view', () => {
    const raw = [{ seq: 9, event_seq: 10, view: baseView() }, { seq: 1, event_seq: 2, view: baseView() }];
    expect(normalizeViews(raw).map((v) => [v.seq, v.event_seq])).toEqual([[1, 2], [9, 10]]);
    expect(normalizeViews([{ seq: 3, view: {} as never }])).toEqual([]);
    expect(viewsFromEvents(v1Detail.events)[0].event_seq).toBe(2);
  });

  it('finds the view at a step: the latest at or before it, the last without a step', () => {
    const vs = [{ seq: 1, view: baseView() }, { seq: 6, view: baseView() }, { seq: 9, view: baseView() }];
    expect(viewIndexAt(vs, 7)).toBe(1);
    expect(viewIndexAt(vs, 9)).toBe(2);
    expect(viewIndexAt(vs, null)).toBe(2);
    expect(viewIndexAt(vs, 0)).toBe(0);
    expect(viewIndexAt([], 3)).toBe(-1);
  });

  it('says what changed since the view before', () => {
    const hold = v1Views.findIndex((v, i) => i > 0 && v.view.areas[2].items.find((x) => x.id === 'CVX-2611')!.status === 'on hold');
    const d = diffViews(v1Views[hold - 1].view, v1Views[hold].view);
    expect(d.items.map((c) => c.line)).toEqual(['CVX-2611: quarantined → on hold']);
    expect(d.items[0]).toMatchObject({ id: 'CVX-2611', area: 'Warehouse', kind: 'changed', short: 'quarantined → on hold', abnormal: true });
    expect(diffViews(null, v1Views[0].view).items).toEqual([]);
  });

  it('lists new flows, new items and changed KPIs', () => {
    const i = v1Views.findIndex((v) => v.view.flows.length === 1);
    const d = diffViews(v1Views[i - 1].view, v1Views[i].view);
    expect(d.flows.map((f) => [f.from, f.to])).toEqual([['CVX-2604', 'PO-SBMC-4471']]);
    expect(d.kpis.get('orders_at_risk')).toBe('8 of 10');
    expect(d.items.find((c) => c.id === 'PO-SBMC-4471')!.short).toBe('at risk → late');
    const added = diffViews(baseView(), { ...baseView(), areas: [{ id: 'x', label: 'X', kind: 'lab', items: [{ id: 'n', label: 'NC-1', status: 'open', abnormal: false }] }] });
    expect(added.items.find((c) => c.id === 'n')!.line).toBe('NC-1: new · open');
    expect(added.items.find((c) => c.id === 'CVX-2611')!.kind).toBe('removed');
  });

  it('puts the change that matters first: status, then quantity, detail, due', () => {
    const it0 = { id: 'a', label: 'A', status: 'released', abnormal: false, qty: 4000, unit: 'syr', detail: 'x', due: '11-10' };
    expect(itemShort(it0, { ...it0, qty: 2800 })).toBe('4,000 syr → 2,800 syr');
    expect(itemShort(it0, { ...it0, detail: 'y' })).toBe('y');
    expect(itemShort(it0, { ...it0, due: '11-12' })).toBe('due 11-10 → 11-12');
    expect(itemShort(it0, { ...it0 })).toBeNull();
    expect(itemFacts(it0)).toBe('4,000 syr · due 11-10 · x');
  });

  it('gives every status a shape; colour only when abnormal', () => {
    expect(statusGlyph('released', false)).toBe('✓');
    expect(statusGlyph('not qualified', false)).toBe('○');
    expect(statusGlyph('qualified', false)).toBe('✓');
    expect(statusGlyph('on hold', true)).toBe('■');
    expect(statusGlyph('at risk', true)).toBe('⚠');
    expect(statusGlyph('expired', true)).toBe('✕');
    expect(statusGlyph('filling', false)).toBe('◐');
    expect(statusGlyph('weird', true)).toBe('⚠');
    expect(statusGlyph('weird', false)).toBe('●');
    expect(statusMark({ status: 'no capacity', abnormal: true })).toEqual({ glyph: '⚠', word: 'No capacity', abnormal: true });
  });
});

describe('twin layout', () => {
  const view = baseView();
  it('orders zones by process, approvals as the rail, unknown kinds after the known', () => {
    const shuffled = { ...view, areas: [view.areas[4], { id: 'u', label: 'Other', kind: 'claims', items: [] }, ...view.areas.slice(0, 4), view.areas[5]] };
    const { main, rail } = splitAreas(shuffled);
    expect(main.map((a) => a.kind)).toEqual([...PROCESS_ORDER, 'claims']);
    expect(rail.map((a) => a.kind)).toEqual(['approvals']);
  });

  it('lays out side by side at a wide screen, customers in two columns, zones one height', () => {
    expect(fitsSideBySide(view, 1400)).toBe(true);
    expect(fitsSideBySide(view, 700)).toBe(false);
    const l = twinLayout(view, 1400, new Set(['PO-SBMC-4471']));
    expect(l.zones.map((z) => z.area.label)).toEqual(['Fill line', 'QC lab', 'Warehouse', 'Carriers', 'Customers', 'Approvals']);
    const cust = l.zones[4];
    expect(cust.cols).toBe(2);
    expect(new Set(l.zones.map((z) => z.h)).size).toBe(1);
    expect(Math.round(l.zones.at(-1)!.x + l.zones.at(-1)!.w)).toBe(1400);
    const tiles = l.tiles.filter((t) => t.zone === 'cust');
    expect(tiles[0].h).toBe(TILE_H_CHANGED);
    expect(tiles[1].h).toBe(TILE_H_CHANGED);
    expect(tiles[2].h).toBe(TILE_H);
    expect(Math.min(...l.tiles.map((t) => t.w))).toBeGreaterThanOrEqual(MIN_TILE_W);
  });

  it('draws an arrow per flow whose ends are on screen, left to right', () => {
    const v = { ...view, flows: [{ id: 'f', from: 'CVX-2604', to: 'PO-LCH-3307', label: 'Borealis', status: 'booked', abnormal: false }, { id: 'g', from: 'nope', to: 'PO-LCH-3307', label: '', status: '', abnormal: false }] };
    const l = twinLayout(v, 1400);
    expect(l.arrows.map((a) => a.flow.id)).toEqual(['f']);
    const from = l.tiles.find((t) => t.id === 'CVX-2604')!;
    expect(l.arrows[0].x1).toBe(from.x + from.w);
    expect(l.arrows[0].d.startsWith(`M${from.x + from.w},`)).toBe(true);
    expect(l.arrows[0].label).toBeDefined();
    const back = arrowBetween(v.flows[0], { x: 500, y: 0, w: 100, h: 50 }, { x: 0, y: 0, w: 100, h: 50 });
    expect([back.x1, back.x2]).toEqual([500, 100]);
  });
});

describe('flow routing', () => {
  it('runs lot-to-order flows along a bus under the carriers, one lane each, and makes room for it', () => {
    const v = v1Views.at(-1)!.view;
    const l = twinLayout(v, 1400);
    const carriers = l.zones.find((z) => z.area.kind === 'carriers')!;
    const carrierBottom = Math.max(...l.tiles.filter((t) => t.zone === carriers.area.id).map((t) => t.y + t.h));
    const ys = l.arrows.map((a) => a.label!.y);
    expect(l.arrows).toHaveLength(4);
    expect(new Set(ys).size).toBe(4);
    expect(Math.min(...ys)).toBeGreaterThan(carrierBottom);
    expect(l.height).toBeGreaterThanOrEqual(Math.max(...ys) + BUS_LANE / 2);
    expect(l.arrows.every((a) => a.label!.x === carriers.x + 8)).toBe(true);
  });

  it('rounds the corners of an orthogonal path', () => {
    expect(roundedPath([[0, 0], [10, 0], [10, 10]], 4)).toBe('M0,0 L6,0 Q10,0 10,4 L10,10');
    expect(isIdLike('CVX-2611')).toBe(true);
    expect(isIdLike("Lakeview Children's")).toBe(false);
  });
});

describe('scrubber', () => {
  const track = buildTrack(v1Detail.events, v1Views, null);
  it('stops at each view and marks the disruption and the refusal', () => {
    expect(track.stops).toEqual(v1Views.map((v) => v.seq));
    expect(track.marks.map((m) => [m.kind, m.label])).toEqual([
      ['refusal', 'book_shipment refused'],
      ['disruption', 'Tundra Medical Courier loses capacity after its first quote.'],
    ]);
    expect(track.marks[0].seq).toBe(FORK_SEQ);
    expect(track.min).toBe(1);
    expect(trackAt(track.max, track)).toBe(1);
    expect(trackAt(5, { min: 3, max: 3 })).toBe(0);
  });

  it('marks a fork point', () => {
    expect(trackMarks(v2Detail.events, FORK_SEQ).find((m) => m.kind === 'fork')!.seq).toBe(FORK_SEQ);
  });

  it('snaps a step to its stop and moves between stops', () => {
    const stops = [1, 6, 9];
    expect(snapStop(stops, 7)).toBe(6);
    expect(snapStop(stops, null)).toBe(9);
    expect(snapStop(stops, 0)).toBe(1);
    expect(moveStop(stops, 7, 'next')).toBe(9);
    expect(moveStop(stops, 7, 'prev')).toBe(1);
    expect(moveStop(stops, 9, 'next')).toBe(9);
    expect(nearestStop(stops, 0.6, { min: 1, max: 9 })).toBe(6);
    expect(nearestStop([], 0.5, { min: 0, max: 1 })).toBeNull();
  });

  it('says what happened between two views', () => {
    const disruption = track.marks.find((m) => m.kind === 'disruption')!.seq;
    const h = happenings(v1Detail.events, team, FORK_SEQ - 1, disruption);
    expect(h.map((x) => x.kind)).toEqual(['refusal', 'write', 'write', 'write', 'disruption']);
    expect(h[0].text).toBe('Logistics: book_shipment refused: no approved cold_chain_shipment change for quote Q-0001');
    expect(h[2].text).toBe('World: decide_change');
  });

  it('rewinds to the first agent step after the view, keeping the world as shown', () => {
    const lastShared = v1Views.filter((v) => v.seq < FORK_SEQ).at(-1)!.seq;
    expect(rewindSeq(v1Detail.events, lastShared)).toBe(FORK_SEQ);
    expect(rewindSeq(v1Detail.events, 10_000)).toBeNull();
    const evs: Event[] = [{ branch_id: 'b', seq: 4, at: '', agent: 'link', kind: 'tool_call', payload: {} }, { branch_id: 'b', seq: 5, at: '', agent: null, kind: 'world_view', payload: {} }, { branch_id: 'b', seq: 6, at: '', agent: 'a', kind: 'tool_call', payload: {} }];
    expect(rewindSeq(evs, 3)).toBe(6);
  });
});

describe('swap architecture', () => {
  it('gives every agent the rules, replacing earlier ones', () => {
    const rules = ARCHITECTURES[0].rules;
    const c = swapChanges(team, rules);
    expect(Object.keys(c)).toEqual(team.agents.map((a) => a.id));
    expect(c.quality.prompt).toBe(`${team.agents[1].prompt}\n\nDeployment rules:\n${rules}`);
    expect(withRules(c.quality.prompt!, 'other')).toBe(`${team.agents[1].prompt}\n\nDeployment rules:\nother`);
    expect(withRules(c.quality.prompt!, '  ')).toBe(team.agents[1].prompt);
    expect(swapChanges(team, '')).toEqual({});
  });

  it('makes the single agent a new team of one, which only an external run can take', () => {
    const single = ARCHITECTURES.find((a) => a.id === 'single')!;
    expect(single.shape).toBe('team');
    const t = singleTeam(team, single.rules);
    expect(t.agents.map((a) => [a.id, a.kind, a.read_only])).toEqual([['operator', team.agents[0].kind, false]]);
    expect(t.agents[0].prompt).toContain("every role's work");
    const start = (runner: string): Event[] => [{ branch_id: 'b', seq: 1, at: '', agent: null, kind: 'run_start', payload: { runner } }];
    expect(teamSwapProblem(start('external'))).toBeNull();
    expect(teamSwapProblem(start('chain'))).toContain('Team v2');
  });

  it('opens on "Swap architecture" only when the address asks', () => {
    expect(forkChoiceOf('architecture')).toBe('architecture');
    expect(forkChoiceOf(null)).toBe('agents');
  });
});

describe('world view state', () => {
  const input = { views: v1Views, step: null, events: v1Detail.events, team, startSeq: null };
  it('shows the last view when the address names no step', () => {
    const s = worldState(input);
    expect(s.index).toBe(v1Views.length - 1);
    expect(s.current).toBe(v1Views.at(-1)!.seq);
  });

  it('names the step on screen', () => {
    const before = v1Views.filter((v) => v.seq < FORK_SEQ).at(-1)!.seq;
    const s = worldState({ ...input, step: before });
    expect(s.current).toBe(before);
    expect(s.headline).toBe(`Step ${before} · Logistics · cold_chain_quotes`);
    expect(worldState({ ...input, views: [] }).view).toBeNull();
  });

  it('says when a run made no model call', () => {
    expect(drivenLine([])).toBe('No model calls');
    expect(drivenLine([{ branch_id: 'b', seq: 2, at: '', agent: 'a', kind: 'model_call', payload: {} }])).toBe('');
    // A CLI agent: no model_call events, but its end reports the tokens it used.
    expect(drivenLine([{ branch_id: 'b', seq: 9, at: '', agent: 'a', kind: 'agent_end', payload: { tokens: 573562 } }])).toBe('');
    expect(drivenLine([{ branch_id: 'b', seq: 9, at: '', agent: 'a', kind: 'agent_end', payload: { tokens: 0 } }])).toBe('No model calls');
    expect(stepHeadline(v1Detail.events, team, 1)).toBe('Step 1 · after setup');
    expect(stepHeadline(v1Detail.events, team, null)).toBe('No step');
  });

  it('keeps the step in the address', () => {
    expect([parseStep('12'), parseStep(''), parseStep('x'), parseStep(null)]).toEqual([12, null, null, null]);
    expect(withStep('/world/r/b', 9)).toBe('/world/r/b?step=9');
    expect(withStep('/world/r/b', null)).toBe('/world/r/b');
  });
});
