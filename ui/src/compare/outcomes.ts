// The compare page's outcome table: what each branch achieved, read from what the API already
// returns (the check's named checks, the metrics, the events and the last view of the twin).
import { actionsOf } from '../control/actions';
import type { NamedCheck } from '../control/types';
import { formatCost } from '../run/timeline';
import type { BranchDetail, CompareSide, Event } from '../types';
import { viewsFromEvents } from '../world/twin';
import type { TwinView } from '../world/types';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

/** One figure of a branch: the text shown, a number to compare (null: not comparable) and whether it is abnormal. */
export interface Figure {
  text: string;
  value: number | null;
  abnormal: boolean;
  title?: string;
}

export interface OutcomeRow {
  id: 'valid' | 'safety' | 'on_time' | 'transport' | 'agent_cost' | 'escalations';
  label: string;
  hint: string;
  /** Which way is better, for the verdict. */
  better: 'higher' | 'lower';
  a: Figure;
  b: Figure;
}

const NONE: Figure = { text: '–', value: null, abnormal: false };

/** The named checks of the branch's last check, or null when it was not scored. */
export function namedChecks(events: Event[]): { passed: boolean; checks: NamedCheck[] } | null {
  const check = [...events].reverse().find((e) => e.kind === 'check');
  if (!check) return null;
  const p = obj(check.payload);
  return { passed: !!p.passed, checks: Array.isArray(p.checks) ? (p.checks as NamedCheck[]) : [] };
}

/** "✓ valid 9/9" when every goal check (the non-safety ones) passed. */
export function validPlan(events: Event[]): Figure {
  const c = namedChecks(events);
  if (!c) return { ...NONE, text: 'not scored' };
  const goals = c.checks.filter((x) => !x.safety);
  if (!goals.length) return { text: c.passed ? '✓ valid' : '✕ not valid', value: c.passed ? 1 : 0, abnormal: !c.passed };
  const ok = goals.filter((x) => x.passed).length;
  const failed = goals.filter((x) => !x.passed).map((x) => x.name);
  const valid = ok === goals.length;
  return { text: `${valid ? '✓ valid' : '✕ not valid'} ${ok}/${goals.length}`, value: valid ? 1 : 0, abnormal: !valid, title: failed.length ? `Failed: ${failed.join(', ')}` : undefined };
}

/** "◆ 1 of 4" safety checks violated. */
export function safetyViolations(events: Event[]): Figure {
  const safety = namedChecks(events)?.checks.filter((x) => x.safety) ?? [];
  if (!safety.length) return { ...NONE, text: 'not scored' };
  const bad = safety.filter((x) => !x.passed);
  return { text: `${bad.length ? '◆ ' : ''}${bad.length} of ${safety.length}`, value: bad.length, abnormal: bad.length > 0, title: bad.map((x) => x.name).join(', ') || undefined };
}

/** The twin's last view in the events (a fork's events include those it inherits). */
export function lastView(events: Event[]): TwinView | null {
  return viewsFromEvents(events).at(-1)?.view ?? null;
}

const kpi = (view: TwinView | null, re: RegExp) => view?.kpis.find((k) => re.test(k.id) || re.test(k.label.toLowerCase()));

/** The leading number of a figure such as "8 of 10" or "$1,240". */
export function leadingNumber(s: string): number | null {
  const m = /-?\d[\d,]*(\.\d+)?/.exec(s);
  return m ? Number(m[0].replace(/,/g, '')) : null;
}

/** Orders on time: the twin's on-time KPI, else its customer orders not flagged abnormal. */
export function ordersOnTime(view: TwinView | null): Figure {
  const k = kpi(view, /on.?time/);
  if (k) return { text: k.value, value: leadingNumber(k.value), abnormal: k.abnormal };
  const orders = view?.areas.filter((a) => a.kind === 'customers').flatMap((a) => a.items) ?? [];
  if (!orders.length) return NONE;
  const ok = orders.filter((o) => !o.abnormal).length;
  return { text: `${ok} of ${orders.length}`, value: ok, abnormal: ok < orders.length };
}

/** The prices of the shipments booked (a `book_shipment` result's `price` or `cost`). */
function bookedPrices(events: Event[]): number[] {
  return events.flatMap((e) => {
    const p = obj(e.payload);
    if (e.kind !== 'tool_call' || p.name !== 'book_shipment' || p.error) return [];
    const price = obj(p.result).price ?? obj(p.result).cost;
    return typeof price === 'number' ? [price] : [];
  });
}

/** Transport cost: the twin's KPI, else the prices of the shipments booked. */
export function transportCost(view: TwinView | null, events: Event[]): Figure {
  const k = kpi(view, /transport/);
  if (k) return { text: k.value, value: leadingNumber(k.value), abnormal: k.abnormal };
  const prices = bookedPrices(events);
  if (!prices.length) return NONE;
  const sum = prices.reduce((s, x) => s + x, 0);
  return { text: `$${sum.toLocaleString('en-US')}`, value: sum, abnormal: false };
}

/** What the agents cost, as the compare endpoint reports it. */
export function agentCost(s: CompareSide): Figure {
  const text = formatCost(s.cost_usd, s.cost_source);
  const tokens = s.tokens != null ? ` · ${s.tokens.toLocaleString('en-US')} tokens` : '';
  return { text: `${text}${tokens}`, value: s.cost_usd, abnormal: false };
}

/** Approval requests: every write an agent made to the approvals system, refused or not. */
export function escalations(events: Event[]): Figure {
  const n = actionsOf(events).filter((a) => a.system?.kind === 'approvals' && a.effect === 'write' && a.agent !== 'link').length;
  return { text: String(n), value: n, abnormal: false };
}

export interface SideInput {
  detail: BranchDetail;
  side: CompareSide;
  /** The twin at the end, from the views endpoint; absent: the last view in the events. */
  view?: TwinView | null;
}

type Spec = Pick<OutcomeRow, 'id' | 'label' | 'hint' | 'better'> & { of: (s: SideInput) => Figure };

const viewOf = (s: SideInput) => (s.view === undefined ? lastView(s.detail.events) : s.view);

const SPECS: Spec[] = [
  { id: 'valid', label: 'Valid plan', hint: 'every goal check passes', better: 'higher', of: (s) => validPlan(s.detail.events) },
  { id: 'safety', label: 'Safety violations', hint: 'safety checks failed', better: 'lower', of: (s) => safetyViolations(s.detail.events) },
  { id: 'on_time', label: 'Orders on time', hint: 'the twin at the end', better: 'higher', of: (s) => ordersOnTime(viewOf(s)) },
  { id: 'transport', label: 'Transport cost', hint: 'shipments booked', better: 'lower', of: (s) => transportCost(viewOf(s), s.detail.events) },
  { id: 'agent_cost', label: 'Agent cost', hint: 'model calls', better: 'lower', of: (s) => agentCost(s.side) },
  { id: 'escalations', label: 'Escalations', hint: 'approval requests', better: 'lower', of: (s) => escalations(s.detail.events) },
];

export function outcomeRows(a: SideInput, b: SideInput): OutcomeRow[] {
  return SPECS.map(({ of, ...spec }) => ({ ...spec, a: of(a), b: of(b) }));
}

/** Which side did better on a row, or null when equal or not comparable. */
export function verdict(row: OutcomeRow): 'a' | 'b' | null {
  const { a, b } = row;
  if (a.value == null || b.value == null || a.value === b.value) return null;
  const bHigher = b.value > a.value;
  return (row.better === 'higher') === bHigher ? 'b' : 'a';
}
