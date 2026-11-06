import { describe, expect, it } from 'vitest';
import { actionsOf } from './actions';
import { events, team } from './fixtures';
import { abnormal, countsLine, processView } from './process';

describe('process view', () => {
  const v = processView(events, actionsOf(events), team);

  it('shows_agents_as_units_with_their_state', () => {
    expect(v.units.map((u) => [u.id, u.state, countsLine(u)])).toEqual([
      ['lead', 'running', '0 R · 0 W'],
      ['quality', 'done', '1 R · 3 W'],
    ]);
    expect(abnormal(v.units[1])).toBe(true);
  });

  it('shows_every_part_of_the_world_as_equipment_even_untouched', () => {
    expect(v.equipment.map((e) => [e.part, e.system?.kind, e.reads, e.writes, e.refused])).toEqual([
      ['erp', 'erp', 0, 2, 1],
      ['lims', 'lims', 1, 0, 0],
      ['gate', 'approvals', 0, 1, 0],
    ]);
  });

  it('counts_each_flow_from_an_agent_to_a_system', () => {
    expect(v.flows.map((f) => [f.agent, f.part, countsLine(f), abnormal(f)])).toEqual([
      ['quality', 'lims', '1 R · 0 W', false],
      ['quality', 'erp', '0 R · 2 W', true],
      ['quality', 'gate', '0 R · 1 W', false],
    ]);
  });
});
