// A team drawn as a Sculptured-style board: agents as cards, grouped by role when the team has
// groups, with "hands work to" arrows. A team from Sculptured keeps its own places; any other is
// laid out from how it runs (in order: left to right in rows, with an arrow to the next; at once:
// one row). Each card says, in a line, what the agent does and what it runs on.
import { MODE_LABEL, agentModelLine, providerName, providerOf } from '../providers';
import type { AgentSpec, Runner, Team } from '../types';

export const CARD_W = 216;
export const CARD_H = 112;
const GAP_X = 56;
const GAP_Y = 48;
/** Cards per row of a team that runs in order. */
const ROW = 3;

export interface BoardGroup {
  id: string;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface BoardLink {
  from: string;
  to: string;
}

/** A Sculptured design's board, kept with the team in this browser (arenad ignores it). */
export interface Board {
  groups: BoardGroup[];
  /** Each card's top-left corner, by agent id. */
  at: Record<string, { x: number; y: number }>;
  links: BoardLink[];
  /** The decisions each agent owns, in plain words (Sculptured's `authority`). */
  decides?: Record<string, string[]>;
}

export interface Card {
  agent: AgentSpec;
  x: number;
  y: number;
}

export interface BoardLayout {
  cards: Card[];
  groups: BoardGroup[];
  links: BoardLink[];
}

const capital = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** "sonnet" → "Sonnet"; a full model id stays as it is. */
export const modelWord = (m: string) => (/^[a-z]+$/.test(m) ? capital(m) : m);

/** The model an agent runs on, in words: "Sonnet", "Default model", "deepseek/… (OpenRouter)". */
export function modelOf(a: Pick<AgentSpec, 'kind' | 'model' | 'provider'>): string {
  const m = a.model?.trim();
  if (a.kind === 'open') return `${m || 'Default model'} (${providerName(providerOf(a))})`;
  // arenad starts Claude Code with Sonnet when the team names no model.
  if (!m) return a.kind === 'claude_code' ? 'Sonnet' : 'Default model';
  return modelWord(m);
}

/** "Claude Code · Sonnet", "llama3.3 · ollama (local)": where an agent runs and on which model. */
export const modelBadge = (a: Pick<AgentSpec, 'kind' | 'model' | 'provider'>) => (a.kind === 'open' ? agentModelLine(a) : `${MODE_LABEL[a.kind]} · ${modelOf(a)}`);

/** What an agent does, in one line: its prompt's first sentence that isn't "You are …", without a leading "Name:". */
export function jobLine(a: Pick<AgentSpec, 'name' | 'prompt'>, max = 96): string {
  const text = a.prompt.trim().split(/\n\s*\n/)[0].replace(/\s+/g, ' ');
  const sentences = text.match(/[^.!?]+[.!?]*/g) ?? [];
  let line = (sentences.find((s) => !/^\s*you are\b/i.test(s)) ?? sentences[0] ?? '').trim();
  const named = `${a.name.toLowerCase()}:`;
  if (line.toLowerCase().startsWith(named)) line = line.slice(named.length).trim();
  line = capital(line);
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

/** Positions for a team without a board: rows of three left to right in order, or one row when its agents run at once. */
function plainLayout(agents: AgentSpec[], runner: Runner): BoardLayout {
  const inOrder = runner === 'chain';
  const perRow = inOrder ? ROW : Math.max(1, agents.length);
  const cards = agents.map((agent, i) => ({ agent, x: (i % perRow) * (CARD_W + GAP_X), y: Math.floor(i / perRow) * (CARD_H + GAP_Y) }));
  const links = inOrder ? agents.slice(1).map((a, i) => ({ from: agents[i].id, to: a.id })) : [];
  return { cards, groups: [], links };
}

/** The board's own places, moved to start at 0; agents it doesn't place go in a row under it. */
function boardLayout(agents: AgentSpec[], board: Board): BoardLayout {
  const xs = [...board.groups.map((g) => g.x), ...Object.values(board.at).map((p) => p.x)];
  const ys = [...board.groups.map((g) => g.y), ...Object.values(board.at).map((p) => p.y)];
  const least = (v: number[]) => (v.length ? Math.min(...v) : 0);
  const [dx, dy] = [least(xs), least(ys)];
  const placed = agents.filter((a) => board.at[a.id]);
  const bottom = Math.max(0, ...placed.map((a) => board.at[a.id].y - dy + CARD_H), ...board.groups.map((g) => g.y - dy + g.h));
  let extra = 0;
  const cards = agents.map((agent) => {
    const p = board.at[agent.id];
    if (p) return { agent, x: p.x - dx, y: p.y - dy };
    return { agent, x: extra++ * (CARD_W + GAP_X), y: bottom + GAP_Y };
  });
  const ids = new Set(agents.map((a) => a.id));
  return {
    cards,
    groups: board.groups.map((g) => ({ ...g, x: g.x - dx, y: g.y - dy })),
    links: board.links.filter((l) => ids.has(l.from) && ids.has(l.to)),
  };
}

/** The board of `team` as it would run under `runner`. */
export function layoutBoard(team: Team, runner: Runner = 'chain'): BoardLayout {
  return team.board ? boardLayout(team.agents, team.board) : plainLayout(team.agents, runner);
}
