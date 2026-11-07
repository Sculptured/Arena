// The digital twin's model: the branch's views in step order, the view at a step, what changed
// since the view before, and each item's status as a glyph and a word.
import type { Mark } from '@/lib/mark';
import type { Event } from '../types';
import type { BranchView, TwinFlow, TwinItem, TwinView, WorldViewPayload } from './types';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

const isView = (v: unknown): v is TwinView => Array.isArray(obj(v).areas);

/** A `world_view` event's payload (also accepted as a world event of kind `world_view`), or null. */
export function viewPayload(e: Event): WorldViewPayload | null {
  const p = obj(e.payload);
  const inner = e.kind === 'world_event' && p.kind === 'world_view' ? obj(p.data) : p;
  if (e.kind !== 'world_view' && !(e.kind === 'world_event' && p.kind === 'world_view')) return null;
  if (!isView(inner.view)) return null;
  return { seq_of: typeof inner.seq_of === 'number' ? inner.seq_of : e.seq, view: inner.view };
}

/** True for an event that is a view of the world, not a step. */
export const isViewEvent = (e: Event) => viewPayload(e) !== null;

/** The views a branch's own events carry, keyed by the step each shows. */
export function viewsFromEvents(events: Event[]): BranchView[] {
  const out: BranchView[] = [];
  for (const e of events) {
    const p = viewPayload(e);
    if (p) out.push({ seq: p.seq_of, view: p.view });
  }
  return ordered(out);
}

/**
 * The views in step order, one per step: an entry keyed by its `world_view` event's own seq is
 * moved to the step it shows, so the view at a step is the latest one at or before it.
 */
export function normalizeViews(raw: BranchView[], events: Event[]): BranchView[] {
  const stepOf = new Map<number, number>();
  for (const e of events) {
    const p = viewPayload(e);
    if (p && p.seq_of !== e.seq) stepOf.set(e.seq, p.seq_of);
  }
  return ordered(raw.filter((v) => isView(v.view)).map((v) => ({ seq: stepOf.get(v.seq) ?? v.seq, view: v.view })));
}

function ordered(list: BranchView[]): BranchView[] {
  const bySeq = new Map<number, BranchView>();
  for (const v of list) bySeq.set(v.seq, v);
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

/** The index of the view at `step` (the latest at or before it); the last view when no step; -1 when none. */
export function viewIndexAt(views: BranchView[], step: number | null): number {
  if (!views.length) return -1;
  if (step == null) return views.length - 1;
  let found = -1;
  views.forEach((v, i) => { if (v.seq <= step) found = i; });
  return found < 0 ? 0 : found;
}

/** Every item of a view by id, with the area it is in. */
export function itemsById(view: TwinView | null): Map<string, TwinItem & { area: string }> {
  const out = new Map<string, TwinItem & { area: string }>();
  for (const a of view?.areas ?? []) for (const it of a.items) out.set(it.id, { ...it, area: a.label });
  return out;
}

export interface ItemChange {
  id: string;
  area: string;
  kind: 'added' | 'removed' | 'changed';
  /** "CVX-2611: quarantined → on hold". */
  line: string;
  /** The short form a tile carries: "quarantined → on hold". */
  short: string;
  abnormal: boolean;
}

export interface ViewChanges {
  items: ItemChange[];
  /** Flows that appeared, e.g. a booked shipment. */
  flows: TwinFlow[];
  /** KPIs whose value changed, by id: the value before. */
  kpis: Map<string, string>;
}

const fmtQty = (it: TwinItem) => `${it.qty?.toLocaleString('en-US')}${it.unit ? ` ${it.unit}` : ''}`;

const qtyText = (it: TwinItem) => (it.qty != null ? fmtQty(it) : '–');

/** What changed in one item, in a few words; null when nothing a person would notice did. */
export function itemShort(before: TwinItem, after: TwinItem): string | null {
  if (before.status !== after.status) return `${before.status} → ${after.status}`;
  if (before.qty !== after.qty && after.qty != null) return `${qtyText(before)} → ${fmtQty(after)}`;
  return textChange(before, after);
}

/** A change in an item's detail or due date. */
function textChange(before: TwinItem, after: TwinItem): string | null {
  const detail = after.detail ?? '';
  if ((before.detail ?? '') !== detail) return detail || 'detail cleared';
  if ((before.due ?? '') !== (after.due ?? '')) return `due ${before.due ?? '–'} → ${after.due ?? '–'}`;
  return null;
}

function itemChanges(prev: TwinView, next: TwinView): ItemChange[] {
  const before = itemsById(prev);
  const after = itemsById(next);
  const out: ItemChange[] = [];
  for (const [id, it] of after) {
    const was = before.get(id);
    const short = was ? itemShort(was, it) : `new · ${it.status}`;
    if (short) out.push({ id, area: it.area, kind: was ? 'changed' : 'added', short, line: `${it.label}: ${short}`, abnormal: it.abnormal });
  }
  for (const [id, it] of before) {
    if (!after.has(id)) out.push({ id, area: it.area, kind: 'removed', short: 'gone', line: `${it.label}: gone`, abnormal: false });
  }
  return out;
}

/** What changed from the view before to this one; nothing for the first view. */
export function diffViews(prev: TwinView | null, next: TwinView | null): ViewChanges {
  if (!prev || !next) return { items: [], flows: [], kpis: new Map() };
  const oldFlows = new Set(prev.flows.map((f) => f.id));
  const oldKpis = new Map(prev.kpis.map((k) => [k.id, k.value]));
  const kpis = new Map<string, string>();
  for (const k of next.kpis) if (oldKpis.has(k.id) && oldKpis.get(k.id) !== k.value) kpis.set(k.id, oldKpis.get(k.id)!);
  return { items: itemChanges(prev, next), flows: next.flows.filter((f) => !oldFlows.has(f.id)), kpis };
}

/** Status words to shapes, most specific first (ISA-101: the shape says the state, colour only abnormal). */
const GLYPHS: [RegExp, string][] = [
  [/\b(no capacity|unavailable|at risk|excursion|late|short.dated|delayed|over limit)\b/, '⚠'],
  [/\b(fail|failed|rejected|refused|expired|cancel+ed|violat)/, '✕'],
  [/\b(on hold|hold|held|stopped|blocked|quarantin)/, '■'],
  [/\b(filling|running|testing|in progress|incubating|proposed|pending|open|quoted|awaiting|in transit|picking)\b/, '◐'],
  [/\b(not qualified|unqualified|planned|draft|scheduled|idle|waiting|not started|new)\b/, '○'],
  [/\b(released|approved|qualified|complete|completed|booked|delivered|covered|passed|on time|notified|available|done|shipped|closed)\b/, '✓'],
];

/** The glyph of a status word; an unknown abnormal status reads ⚠, an unknown normal one ●. */
export function statusGlyph(status: string, abnormal: boolean): string {
  const s = status.toLowerCase();
  const hit = GLYPHS.find(([re]) => re.test(s));
  if (hit) return hit[1];
  return abnormal ? '⚠' : '●';
}

/** An item's (or flow's) status as a mark. */
export function statusMark(x: { status: string; abnormal: boolean }): Mark {
  return { glyph: statusGlyph(x.status, x.abnormal), word: x.status, abnormal: x.abnormal };
}

/** The facts under a tile, each kept whole when it wraps: its quantity, due date and the parts of its detail. */
export function itemFactParts(it: TwinItem): string[] {
  const detail = (it.detail ?? '').split(' · ').map((d) => d.trim());
  return [it.qty != null ? fmtQty(it) : '', it.due ? `due ${it.due}` : '', ...detail].filter(Boolean);
}

/** The line under a tile, as one string. */
export const itemFacts = (it: TwinItem): string => itemFactParts(it).join(' · ');

/** True for a label that is an id ("CVX-2611", "EM-26-11873"): set in mono; names and sentences are not. */
export const isIdLike = (label: string) => /^[A-Z0-9][A-Z0-9._/-]*$/.test(label.trim());
