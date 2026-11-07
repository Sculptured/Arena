import { describe, expect, it } from 'vitest';
import type { AgentSpec, Health, Team } from '../types';
import { missingFor, partWord, restoringLine, runLine, taskName, ticketTitle, worldName } from './summary';

const agent = (id: string, more: Partial<AgentSpec> = {}): AgentSpec => ({ id, name: id, kind: 'claude_code', model: 'sonnet', prompt: '', ...more });
const team = (...agents: AgentSpec[]): Team => ({ id: 't', name: 'T', agents });
const health = (more: Partial<Health> = {}): Health => ({ open_available: false, claude: true, codex: true, ...more });

describe('run screen words', () => {
  it('says what will run in one line', () => {
    expect(runLine(team(...['a', 'b', 'c', 'd', 'e', 'f'].map((id) => agent(id))))).toBe('6 agents · Claude Code · Sonnet');
    expect(runLine(team(agent('a', { model: 'haiku' }), agent('b', { kind: 'codex', model: null })))).toBe('2 agents · Claude Code, Codex · Haiku, Default model');
    expect(runLine(team(agent('a', { model: null })))).toBe('1 agent · Claude Code · Sonnet');
  });

  it('says exactly what is missing, and offers no stand-in', () => {
    expect(missingFor(team(agent('a')), health())).toEqual([]);
    expect(missingFor(team(agent('a')), health({ claude: false }))).toEqual([expect.stringContaining("Claude Code isn't installed")]);
    expect(missingFor(team(agent('a', { kind: 'codex' })), health({ codex: false }))[0]).toContain("Codex isn't installed");
    const providers = [{ id: 'openrouter', kind: 'openrouter' as const, label: 'OpenRouter', available: false, checked: false, why: 'OPENROUTER_API_KEY is not set' }];
    expect(missingFor(team(agent('a', { kind: 'open', model: null })), health({ providers }))).toEqual(["OpenRouter isn't ready: OPENROUTER_API_KEY is not set."]);
    expect(missingFor(team(agent('a')), null)).toEqual([]);
  });

  it('names tasks and the world in plain words', () => {
    expect(taskName({ reason: 'A Grade A sample fails during the fill of CVX-2611: hold it, find stock' })).toBe('A Grade A sample fails during the fill of CVX-2611');
    expect(taskName({ reason: 'the same incident, but the courier has no capacity' })).toBe('The same incident, but the courier has no capacity');
    expect(ticketTitle('# CVX-2611: Grade A excursion\n\nLarchmont…')).toBe('CVX-2611: Grade A excursion');
    expect(worldName('pharma', 'Larchmont Biologics · Corvexa 40 mg PFS')).toBe('Larchmont Biologics');
    expect(worldName('pharma-plant')).toBe('Pharma plant');
    expect([partWord('erp'), partWord('lims'), partWord('logistics')]).toEqual(['ERP', 'LIMS', 'Logistics']);
    expect(restoringLine(['erp', 'crm'])).toBe('Restoring the world: ERP, CRM…');
  });
});
