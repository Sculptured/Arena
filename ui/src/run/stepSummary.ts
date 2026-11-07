// A step in one or two plain sentences, for the step drawer: who did what, on which system, and
// how it went (refused or failed, and why). The raw record stays behind "Details".
import { type Action, actionWord, classify, kindWord, worldInfo } from '../control/actions';
import { formatDuration } from '../control/lanes';
import type { Event, Team } from '../types';
import { clockText } from './flow';

export interface StepSummary {
  title: string;
  text: string;
  abnormal: boolean;
}

type Payload = Record<string, unknown>;
const obj = (v: unknown): Payload => (v && typeof v === 'object' ? (v as Payload) : {});
const clip = (s: string, n = 180) => (s.length > n ? `${s.slice(0, n - 1).trimEnd()}…` : s);
const quote = (s: unknown) => `“${clip(String(s ?? '').replace(/\s+/g, ' ').trim())}”`;

function nameIn(team: Team, id: string | null): string {
  if (id == null || id === 'link') return 'The world';
  return team.agents.find((a) => a.id === id)?.name ?? id;
}

const VERB = { write: 'wrote to', read: 'read', unknown: 'used' } as const;

/** "It worked, in 120 ms.", "It was refused: …", "It failed: …". */
function outcomeLine(a: Action): string {
  if (a.outcome === 'refused') return `It was refused: ${clip(a.error ?? '')}`;
  if (a.outcome === 'error') return `It failed: ${clip(a.error ?? '')}`;
  return a.durationMs == null ? 'It worked.' : `It worked, in ${formatDuration(a.durationMs)}.`;
}

function toolStep(e: Event, events: Event[], who: string): StepSummary {
  const a = classify(e, worldInfo(events))!;
  if (a.system?.kind === 'messages') return { title: `${who} · Messages`, text: `${who} read its messages.`, abnormal: false };
  const system = a.system ? `the ${kindWord(a.system)}` : 'a tool';
  const text = `${who} ${VERB[a.effect]} ${system} (${a.tool}). ${outcomeLine(a)}`;
  return { title: `${who} · ${a.system ? actionWord(a) : a.tool}`, text, abnormal: a.outcome !== 'ok' };
}

function modelStep(p: Payload, who: string): StepSummary {
  const model = obj(p.request).model ?? p.model;
  const usage = obj(p.usage);
  const tin = usage.prompt_tokens ?? p.input_tokens;
  const tout = usage.completion_tokens ?? p.output_tokens;
  const tokens = typeof tin === 'number' && typeof tout === 'number' ? ` ${tin.toLocaleString()} tokens in, ${tout.toLocaleString()} out.` : '';
  return { title: `${who} · Model call`, text: `${who} asked its model${typeof model === 'string' && model ? ` (${model})` : ''}.${tokens}`, abnormal: false };
}

function checkStep(p: Payload): StepSummary {
  const checks = Array.isArray(p.checks) ? (p.checks as { passed: boolean }[]) : [];
  const ok = checks.filter((c) => c.passed).length;
  const text = checks.length ? `The run was scored: ${ok} of ${checks.length} checks passed.` : `The run was scored: it ${p.passed ? 'matches' : 'does not match'} the expected result.`;
  return { title: p.passed ? 'Check passed' : 'Check failed', text, abnormal: !p.passed };
}

type Words = (p: Payload, who: string, to: (v: unknown) => string, e: Event, events: Event[]) => StepSummary;

function endStep(p: Payload, who: string): StepSummary {
  const took = typeof p.seconds === 'number' ? ` after ${clockText(p.seconds)}` : '';
  return p.ok === false
    ? { title: `${who} · Failed`, text: `${who} stopped with an error${took}: ${clip(String(p.error ?? 'no reason given'))}`, abnormal: true }
    : { title: `${who} · Done`, text: `${who} finished${took}.`, abnormal: false };
}

const WORDS: Partial<Record<Event['kind'], Words>> = {
  tool_call: (_p, who, _to, e, events) => toolStep(e, events, who),
  model_call: (p, who) => modelStep(p, who),
  message: (p, who, to) => ({ title: `${who} · Message`, text: `${who} sent a message to ${to(p.to)}: ${quote(p.text)}`, abnormal: false }),
  handoff: (p, who, to) => ({ title: `${who} · Handoff`, text: `${who} left a note for ${to(p.to)}: ${quote(p.note)}`, abnormal: false }),
  agent_start: (_p, who) => ({ title: `${who} · Start`, text: `${who} started.`, abnormal: false }),
  agent_end: endStep,
  check: (p) => checkStep(p),
  world_event: (p) => ({ title: `World · ${String(p.kind ?? 'event')}`, text: String(p.text ?? 'Something happened in the world.'), abnormal: false }),
  pause: () => ({ title: 'Paused', text: 'The run was paused: no agent could act until it resumed.', abnormal: false }),
  resume: (p) => ({ title: 'Resumed', text: `The run went on after ${clockText(Number(p.paused_ms ?? 0) / 1000)} paused.`, abnormal: false }),
  run_start: () => ({ title: 'Start', text: 'The run started and the task was set up in the world.', abnormal: false }),
};

/** The step `e` of a branch with `events`, run by `team`, in words. */
export function stepSummary(e: Event, events: Event[], team: Team): StepSummary {
  const to = (v: unknown) => (v == null ? 'the agents after it' : v === '*' ? 'everyone' : nameIn(team, String(v)));
  const words = WORDS[e.kind];
  return words ? words(obj(e.payload), nameIn(team, e.agent), to, e, events) : { title: 'World view', text: "The world's view after a step.", abnormal: false };
}
