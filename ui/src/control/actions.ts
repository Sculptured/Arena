// Which system each step touched and how: read from the run's own record (`run_start.systems`,
// `run_start.tools`, each tool call's `part`), never guessed. Runs recorded before Arena kept this
// say "unknown".
import { savedParts } from '../restore';
import type { Event } from '../types';
import type { Effect, SystemInfo, ToolInfo } from './types';

export interface WorldInfo {
  /** Per part, what kind of system it is (parts whose integration does not say are absent). */
  systems: Record<string, SystemInfo>;
  tools: Record<string, ToolInfo>;
  /** The parts of the world, in order (from `run_start.world`). */
  parts: string[];
  /** False for a run recorded before Arena kept tool effects. */
  known: boolean;
}

export type Outcome = 'ok' | 'error' | 'refused';

/** One tool call of an agent (or of the world's links), classified. */
export interface Action {
  seq: number;
  /** Milliseconds since the epoch; NaN when the record has no time. */
  at: number;
  agent: string;
  tool: string;
  part: string | null;
  system: SystemInfo | null;
  effect: Effect | 'unknown';
  outcome: Outcome;
  error: string | null;
  /** How long the tool ran in the world; null when not recorded. */
  durationMs: number | null;
  inherited: boolean;
  /** The world was saved after this call (a rerun can start from it). */
  saved: boolean;
}

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

export function worldInfo(events: Event[]): WorldInfo {
  const start = obj(events.find((e) => e.kind === 'run_start')?.payload);
  const world = Array.isArray(start.world) ? start.world : [];
  return {
    systems: obj(start.systems) as Record<string, SystemInfo>,
    tools: obj(start.tools) as Record<string, ToolInfo>,
    parts: world.map((p) => String(obj(p).part ?? '')).filter(Boolean),
    known: 'tools' in start,
  };
}

/** Errors that are a refusal (a gate or a decision right held the call back), not a failure. */
const REFUSAL = /\brefused\b|not available to this agent/i;

export function outcomeOf(error: unknown): Outcome {
  if (error == null || error === '') return 'ok';
  return REFUSAL.test(String(error)) ? 'refused' : 'error';
}

/** The part a call ran on: the one it recorded, else the tool's fixed part, else none. */
function partOf(p: Record<string, unknown>, known: ToolInfo | undefined): string | null {
  if (typeof p.part === 'string') return p.part;
  return known?.part ?? null;
}

/** Arena's own talk tool for reading messages (`exec/talk.rs`): not part of the world. */
const INBOX = 'inbox';
const MESSAGES: SystemInfo = { kind: 'messages', product: "Arena's messages between agents" };

/** What the run recorded about a tool; arena's inbox reads messages. */
function toolInfo(info: WorldInfo, tool: string): ToolInfo | undefined {
  return info.tools[tool] ?? (tool === INBOX ? { effect: 'read' } : undefined);
}

function systemOf(info: WorldInfo, part: string | null, tool: string): SystemInfo | null {
  if (part != null) return info.systems[part] ?? null;
  return tool === INBOX ? MESSAGES : null;
}

const numberOr = (v: unknown): number | null => (typeof v === 'number' ? v : null);

/** A tool call event as an action; null for any other event. */
export function classify(e: Event, info: WorldInfo, startSeq: number | null = null): Action | null {
  if (e.kind !== 'tool_call') return null;
  const p = obj(e.payload);
  const tool = String(p.name ?? 'tool');
  const known = toolInfo(info, tool);
  const part = partOf(p, known);
  const error = p.error == null ? null : String(p.error);
  return {
    seq: e.seq,
    at: Date.parse(e.at),
    agent: e.agent ?? 'world',
    tool,
    part,
    system: systemOf(info, part, tool),
    effect: known?.effect ?? 'unknown',
    outcome: outcomeOf(error),
    error,
    durationMs: numberOr(p.duration_ms),
    inherited: startSeq != null && e.seq < startSeq,
    saved: savedParts(p).length > 0,
  };
}

export function actionsOf(events: Event[], startSeq: number | null = null): Action[] {
  const info = worldInfo(events);
  return events.map((e) => classify(e, info, startSeq)).filter((a): a is Action => a !== null);
}

/** The short word for a system kind ("ERP", "LIMS", "database"); "unknown" when the part does not say. */
export function kindWord(system: SystemInfo | null): string {
  if (!system) return 'unknown';
  const k = system.kind;
  return /^(erp|crm|mes|qms|lims|api|mcp)$/.test(k) ? k.toUpperCase() : k;
}

/** "ERPNext, stands in for SAP S/4HANA, simulated": what runs, what it stands for, and whether it is a simulation. */
export function systemLine(system: SystemInfo | null): string {
  if (!system) return 'system not stated';
  const bits = [system.product];
  if (system.stands_in_for) bits.push(`stands in for ${system.stands_in_for}`);
  if (system.simulated) bits.push('simulated');
  return bits.join(', ');
}

/** "ERP write", "database read", "unknown call": the system kind and the effect, as a mark is labelled. */
export function actionWord(a: Pick<Action, 'system' | 'effect'>): string {
  const effect = a.effect === 'unknown' ? 'call' : a.effect;
  return `${kindWord(a.system)} ${effect}`;
}
