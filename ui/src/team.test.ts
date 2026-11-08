import { describe, expect, it } from 'vitest';
import { isUntouched, teamForRun } from './team';
import type { AgentSpec, Team } from './types';

const agent = (id: string, prompt = `be ${id}`): AgentSpec => ({ id, name: id, kind: 'claude_code', model: 'sonnet', prompt });
const single: Team = { id: 'single', name: 'Single agent', agents: [agent('operator')], runner: 'chain' };
const chain: Team = { id: 'team', name: 'Coordinated team', agents: [agent('coordinator'), agent('records')], runner: 'chain' };
const dispatch: Team = { id: 'dispatch', name: 'Dispatching team', agents: [agent('coordinator', 'lead'), agent('records')], runner: 'dispatch' };

describe('the team a browser keeps', () => {
  it('an old default kept as it came gives way to the new one; an edited, imported or picked team stays', () => {
    const others = [dispatch, chain];
    const keptOldDefault: Team = { id: 'team', name: 'Coordinated team', agents: [agent('coordinator'), agent('records')] };
    expect(isUntouched(keptOldDefault, others)).toBe(true);
    const edited = { ...keptOldDefault, agents: [agent('coordinator', 'my own words'), agent('records')] };
    expect(isUntouched(edited, others)).toBe(false);
    expect(isUntouched({ ...keptOldDefault, picked: true }, others)).toBe(false);
    expect(isUntouched({ ...keptOldDefault, id: 'mine' }, others)).toBe(false);
    expect(isUntouched(single, others)).toBe(false);
  });

  it('a run gets neither the suggested runner nor the picked mark', () => {
    const sent = teamForRun({ ...dispatch, picked: true }, 'dispatch');
    expect('runner' in sent || 'picked' in sent).toBe(false);
    expect(sent.agents.map((a) => a.id)).toEqual(['coordinator', 'records']);
  });
});
