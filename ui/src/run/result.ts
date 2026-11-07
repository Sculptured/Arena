// The result banner of a finished run: Passed or Failed (or Stopped, or an error) and one plain
// sentence on why: the first safety violation, else the first failed check, in words.
import type { BranchDetail, CheckPayload } from '../types';
import { type Flow, flowOf } from './flow';

export interface Result {
  flow: Flow;
  title: string;
  why: string;
}

/** "batch_held" → "Batch held". */
export function humanize(name: string): string {
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : name;
}

/** `s` as a sentence: capitalised, with a full stop. */
const sentence = (s: string) => {
  const t = s.trim();
  return `${t.charAt(0).toUpperCase()}${t.slice(1)}${/[.!?]$/.test(t) ? '' : '.'}`;
};

function checkWhy(p: Partial<CheckPayload>, passed: boolean): string {
  const list = Array.isArray(p.checks) ? p.checks : [];
  if (passed) return list.length ? `All ${list.length} checks passed.` : 'The world matches the expected result.';
  const bad = list.find((c) => c.safety && !c.passed) ?? list.find((c) => !c.passed);
  if (bad) {
    const what = `${bad.safety ? 'Safety violation: ' : ''}${humanize(bad.name)}`;
    return sentence(bad.detail ? `${what}: ${bad.detail}` : what);
  }
  const rows = p.differing_rows?.length ?? 0;
  return rows ? `${rows} ${rows === 1 ? 'record differs' : 'records differ'} from the expected result.` : 'The world does not match the expected result.';
}

/** The banner of a branch that ended; `null` while it runs. */
export function resultOf(d: Pick<BranchDetail, 'status' | 'events' | 'error'>): Result | null {
  const flow = flowOf(d);
  const check = [...d.events].reverse().find((e) => e.kind === 'check')?.payload as Partial<CheckPayload> | undefined;
  switch (flow) {
    case 'passed': return { flow, title: 'Passed', why: checkWhy(check ?? {}, true) };
    case 'failed': return { flow, title: 'Failed', why: check ? checkWhy(check, false) : sentence(d.error ?? 'An agent failed') };
    case 'stopped': return { flow, title: 'Stopped', why: 'You stopped the run before it finished, so it was not scored.' };
    case 'error': return { flow, title: 'Error', why: sentence(d.error ?? 'Something in the world stopped working') };
    case 'interrupted': return { flow, title: 'Interrupted', why: 'Arena restarted while the run was going, so it was not scored.' };
    default: return null;
  }
}
