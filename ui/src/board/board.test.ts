import { describe, expect, it } from 'vitest';
import type { AgentSpec, Team } from '../types';
import { agentModelLine } from '../providers';
import { CARD_H, CARD_W, jobLine, layoutBoard } from './board';
import { importTeam, workOrder } from './importTeam';

const agent = (id: string, more: Partial<AgentSpec> = {}): AgentSpec => ({ id, name: id, kind: 'claude_code', model: null, prompt: '', ...more });
const team = (n: number): Team => ({ id: 't', name: 'T', agents: Array.from({ length: n }, (_, i) => agent(`a${i}`)) });

/** A Sculptured design shaped like `examples/teams/math-workbench.json`, cut down. */
const design = {
  id: 'math-workbench', name: 'Math · interactive workbench', revision: 0,
  models: { claude: 'opus', codex: null },
  groups: [
    { id: 'make', label: 'Design & build', x: 72, y: 180, w: 512, h: 276, models: { claude: 'haiku', codex: null } },
    { id: 'review', label: 'Reviews', x: 600, y: 60, w: 248, h: 516 },
  ],
  agents: [
    { id: 'check', group_id: 'review', name: 'Math verifier', kind: 'codex', job: 'Math verifier: test the computations.', folder: null, x: 624, y: 112,
      advanced: { model: null, permission: 'unrestricted', resume: 'resume-if-possible', data_access: { reads: ['site/'], writes: [] }, authority: { domains: ['Release go/no-go'] } } },
    { id: 'build', group_id: 'make', name: 'Workbench builder', kind: 'claude', job: 'Workbench builder: implement it.', folder: null, x: 360, y: 232,
      advanced: { model: 'sonnet', permission: 'unrestricted', resume: 'resume-if-possible' } },
    { id: 'design', group_id: 'make', name: 'Workbench designer', kind: 'claude', job: 'Design the workbench.\n\nMore.', folder: null, x: 96, y: 232,
      advanced: { model: null, permission: 'unrestricted', resume: 'resume-if-possible' } },
  ],
  connections: [
    { id: 'c1', from: 'design', to: 'build', kind: 'hands-work-to' },
    { id: 'c2', from: 'build', to: 'check', kind: 'hands-work-to' },
    { id: 'c3', from: 'check', to: 'design', kind: 'watches' },
  ],
};

describe('team board layout', () => {
  it('lays a team that runs in order left to right in rows of three, each handing to the next', () => {
    const b = layoutBoard(team(4), 'chain');
    expect(b.cards.map((c) => [c.agent.id, c.x, c.y])).toEqual([
      ['a0', 0, 0], ['a1', CARD_W + 56, 0], ['a2', 2 * (CARD_W + 56), 0], ['a3', 0, CARD_H + 48],
    ]);
    expect(b.links).toEqual([{ from: 'a0', to: 'a1' }, { from: 'a1', to: 'a2' }, { from: 'a2', to: 'a3' }]);
    expect(b.groups).toEqual([]);
  });

  it('puts agents that run at once in one row with no arrows', () => {
    const b = layoutBoard(team(5), 'concurrent');
    expect(new Set(b.cards.map((c) => c.y))).toEqual(new Set([0]));
    expect(b.links).toEqual([]);
  });

  it("keeps a Sculptured board's places, moved to start at the corner", () => {
    const imported = importTeam(JSON.stringify(design));
    if ('error' in imported) throw new Error(imported.error);
    const b = layoutBoard(imported.team);
    const at = Object.fromEntries(b.cards.map((c) => [c.agent.id, [c.x, c.y]]));
    expect(at).toEqual({ design: [24, 172], build: [288, 172], check: [552, 52] });
    expect(b.groups.map((g) => [g.id, g.x, g.y])).toEqual([['make', 0, 120], ['review', 528, 0]]);
    expect(b.links).toEqual([{ from: 'design', to: 'build' }, { from: 'build', to: 'check' }]);
  });

  it('says what each agent does and runs on in one line', () => {
    expect(jobLine({ name: 'Quality', prompt: 'You are the Quality lead at Larchmont. Open the deviation and hold the batch. Then hand off.' }))
      .toBe('Open the deviation and hold the batch.');
    expect(jobLine({ name: 'Workbench builder', prompt: 'Workbench builder: implement it.' })).toBe('Implement it.');
    expect(jobLine({ name: 'x', prompt: 'a'.repeat(200) }, 20)).toHaveLength(20);
    expect(agentModelLine(agent('a', { model: 'haiku' }))).toBe('Claude Code · Haiku');
    expect(agentModelLine(agent('a'))).toBe('Claude Code · Sonnet');
    expect(agentModelLine(agent('a', { kind: 'codex' }))).toBe('Codex · Default model');
  });
});

describe('import', () => {
  it('maps a Sculptured design to real Claude Code and Codex agents, in work order', () => {
    const r = importTeam(JSON.stringify(design));
    if ('error' in r) throw new Error(r.error);
    expect(r.from).toBe('Sculptured');
    expect(r.team.name).toBe('Math · interactive workbench');
    expect(r.team.agents.map((a) => [a.id, a.kind, a.model, a.read_only])).toEqual([
      ['design', 'claude_code', 'haiku', false], // the group's model
      ['build', 'claude_code', 'sonnet', false], // its own
      ['check', 'codex', null, true], // no Codex default anywhere; writes nothing: read-only
    ]);
    expect(r.team.agents[2].prompt).toBe('Math verifier: test the computations.');
    expect(r.team.board?.decides).toEqual({ check: ['Release go/no-go'] });
  });

  it('refuses kinds Arena cannot run, by name, instead of swapping them', () => {
    const d = { ...design, agents: [{ ...design.agents[0], kind: 'opencode', name: 'Coder' }] };
    const r = importTeam(JSON.stringify(d));
    expect('error' in r && r.error).toContain("Arena can't run OpenCode agents yet: Coder");
  });

  it('takes an Arena team as it is, and says what is wrong with anything else', () => {
    const t = team(2);
    expect(importTeam(JSON.stringify(t))).toEqual({ team: t, from: 'Arena' });
    expect(importTeam('{nope')).toEqual({ error: expect.stringContaining("isn't a JSON file") });
    expect(importTeam('{"id":"x","name":"x","agents":[]}')).toEqual({ error: 'There are no agents in this file.' });
    expect(importTeam(JSON.stringify({ ...t, agents: [{ id: 'a', name: 'A', kind: 'scripted', prompt: '' }] })))
      .toEqual({ error: expect.stringContaining('needs an id, a name, a kind') });
    expect(importTeam('{"madl":"draft","agents":[{}]}')).toEqual({ error: expect.stringContaining('MADL') });
  });

  it('orders agents after those that hand them work, file order otherwise, and survives a cycle', () => {
    const a = (id: string) => ({ id });
    expect(workOrder([a('c'), a('b'), a('a')], [{ from: 'a', to: 'b' }, { from: 'b', to: 'c' }]).map((x) => x.id)).toEqual(['a', 'b', 'c']);
    expect(workOrder([a('x'), a('y')], [{ from: 'x', to: 'y' }, { from: 'y', to: 'x' }]).map((x) => x.id)).toEqual(['x', 'y']);
  });
});
