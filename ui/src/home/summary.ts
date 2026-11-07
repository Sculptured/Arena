// The Run screen's words: the world and task in plain words, the one line on what will run, and
// what is missing before a team can run (a CLI not installed, a key not set). Never a stand-in.
import { modelOf } from '../board/board';
import { MODE_LABEL, providerOf } from '../providers';
import type { AgentKind, Health, TaskSummary, Team } from '../types';

const capital = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** "6 agents · Claude Code · Sonnet": how many agents, where they run, on which models. */
export function runLine(team: Team): string {
  const n = team.agents.length;
  const modes = [...new Set(team.agents.map((a) => MODE_LABEL[a.kind]))];
  const models = [...new Set(team.agents.map(modelOf))];
  return [`${n} agent${n === 1 ? '' : 's'}`, modes.join(', '), models.join(', ')].join(' · ');
}

/** Why API-mode agents on `id` can't run: a provider not configured, or not ready. */
function providerProblem(id: string, h: Health): string | null {
  const p = h.providers?.find((s) => s.id === id);
  if (!p) return `The model provider ${id} isn't configured.`;
  const unknown = p.kind === 'local' && !p.checked;
  return p.available || unknown ? null : `${p.label} isn't ready: ${capital(p.why ?? 'unavailable')}.`;
}

const CLI: [AgentKind, keyof Health, string][] = [
  ['claude_code', 'claude', "Claude Code isn't installed: put the claude command on this machine's PATH and sign in."],
  ['codex', 'codex', "Codex isn't installed: put the codex command on this machine's PATH and sign in."],
];

/** What keeps the team from running here, one sentence each; empty when nothing does. */
export function missingFor(team: Team, h: Health | null): string[] {
  if (!h) return [];
  const kinds = new Set(team.agents.map((a) => a.kind));
  const cli = CLI.filter(([kind, found]) => kinds.has(kind) && !h[found]).map(([, , why]) => why);
  const providers = [...new Set(team.agents.filter((a) => a.kind === 'open').map(providerOf))].map((id) => providerProblem(id, h));
  const external = kinds.has('external') ? ['This team is driven by an external system: start it from that system, or change the agents to Claude Code or Codex.'] : [];
  return [...cli, ...providers.filter((p): p is string => !!p), ...external];
}

/** A task's name in plain words: its one-line reason, up to the first colon. */
export function taskName(t: Pick<TaskSummary, 'reason'>): string {
  const head = t.reason.split(':')[0].trim();
  return capital(head || t.reason);
}

/** "CVX-2611: Grade A environmental-monitoring excursion": the ticket's first heading. */
export function ticketTitle(ticket: string): string {
  const line = ticket.split('\n').find((l) => l.trim().startsWith('#'));
  return line ? line.replace(/^#+\s*/, '').trim() : '';
}

/** A world part as a word: short ones are acronyms (ERP, CRM, LIMS), others capitalised. */
export const partWord = (part: string) => (part.length <= 4 ? part.toUpperCase() : capital(part));

/** "Restoring the world: ERP, LIMS, Logistics, Approvals, CRM…" */
export function restoringLine(parts: string[]): string {
  return parts.length ? `Restoring the world: ${parts.map(partWord).join(', ')}…` : 'Restoring the world…';
}

/** The world's name: the twin's own title up to its first " · " or ": " (its organisation), else the benchmark's name. */
export function worldName(benchmark: string, twinTitle?: string | null): string {
  const org = twinTitle?.split(/ · |: /)[0]?.trim();
  return org || capital(benchmark.replace(/[-_]+/g, ' '));
}
