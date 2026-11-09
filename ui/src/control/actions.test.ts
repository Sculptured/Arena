import { describe, expect, it } from 'vitest';
import { actionWord, actionsOf, classify, kindWord, outcomeOf, systemLine, worldInfo } from './actions';
import { ev, events, runStart } from './fixtures';

describe('actions', () => {
  it('classifies_each_call_by_the_system_its_part_is_and_the_tools_effect', () => {
    const a = actionsOf(events);
    expect(a.map((x) => [x.tool, kindWord(x.system), x.effect, x.outcome])).toEqual([
      ['get_result', 'LIMS', 'read', 'ok'],
      ['open_deviation', 'ERP', 'write', 'ok'],
      ['request_approval', 'approvals', 'write', 'ok'],
      ['hold_batch', 'ERP', 'write', 'refused'],
    ]);
    expect(a[1].durationMs).toBe(1500);
    expect(a[2].durationMs).toBeNull();
    expect(actionWord(a[0])).toBe('LIMS read');
  });

  it('a_run_recorded_before_world_info_is_unknown_not_guessed', () => {
    const old = ev(1, 0, null, 'run_start', { task_id: 'x', world: [{ part: 'erp', integration: 'erpnext' }] });
    const info = worldInfo([old]);
    expect(info.known).toBe(false);
    const a = classify(ev(2, 1, 'a', 'tool_call', { name: 'open_deviation', part: 'erp' }), info)!;
    expect([a.system, a.effect, actionWord(a)]).toEqual([null, 'unknown', 'Unknown call']);
  });

  it('a_benchmark_tool_takes_its_part_from_the_call_and_errors_are_not_refusals', () => {
    const info = worldInfo([runStart]);
    const a = classify(ev(2, 1, 'a', 'tool_call', { name: 'note', part: 'lims', error: 'timeout' }), info)!;
    expect([kindWord(a.system), a.effect, a.outcome]).toEqual(['LIMS', 'read', 'error']);
    expect(outcomeOf('Write tools are not available to this agent: its team made it read-only')).toBe('refused');
    expect(outcomeOf(null)).toBe('ok');
  });

  it('labels_stand_ins_with_what_they_stand_in_for_and_simulations_as_simulated', () => {
    const info = worldInfo(events);
    expect(systemLine(info.systems.lims)).toBe('Shakedown LIMS, stands in for LabWare, simulated');
    expect(systemLine(info.systems.erp)).toBe('ERPNext, stands in for SAP S/4HANA');
    expect(systemLine(null)).toBe('system not stated');
  });
});
