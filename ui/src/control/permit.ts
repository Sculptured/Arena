// Interlocks: the permit a write ran under. A write that names a change (in its arguments) is
// matched with the approvals system's record of that change: who decided it, and how.
import type { Event } from '../types';

export interface Permit {
  id: string;
  approved: boolean | null;
  /** "QA head approved chg-… (deviation)"; empty while undecided. */
  decision: string;
  /** The change's action, e.g. "cold_chain_shipment". */
  action: string;
}

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

/** Every change the approvals system recorded up to `seq`, by id, with its decision when there is one. */
function changesUpTo(events: Event[], seq: number): Map<string, Permit> {
  const out = new Map<string, Permit>();
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (e.seq > seq || e.kind !== 'world_event') continue;
    const p = obj(e.payload);
    const data = obj(p.data);
    const id = typeof data.id === 'string' ? data.id : null;
    if (!id) continue;
    const prev = out.get(id) ?? { id, approved: null, decision: '', action: '' };
    if (p.kind === 'change_proposed') out.set(id, { ...prev, action: String(data.action ?? '') });
    if (p.kind === 'decided') out.set(id, { ...prev, approved: data.approved === true, decision: String(p.text ?? '') });
  }
  return out;
}

/** The string values in a tool call's arguments (one level deep). */
function argStrings(args: unknown): string[] {
  return Object.values(obj(args)).filter((v): v is string => typeof v === 'string');
}

/** The permit a tool call names, if any: a change the approvals system knows. */
export function permitOf(events: Event[], call: Event): Permit | null {
  if (call.kind !== 'tool_call') return null;
  const changes = changesUpTo(events, call.seq);
  const named = argStrings(obj(call.payload).args).find((v) => changes.has(v));
  return named ? changes.get(named)! : null;
}

/** The change id a call asked under but the approvals system never recorded (`proposal_id`, `change_id`). */
export function missingPermit(events: Event[], call: Event): string | null {
  if (call.kind !== 'tool_call' || permitOf(events, call)) return null;
  const args = obj(obj(call.payload).args);
  const id = args.proposal_id ?? args.change_id ?? args.permit;
  return typeof id === 'string' ? id : null;
}
