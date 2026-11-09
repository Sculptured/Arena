import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { missingFor, runLine, runnerProblem } from '../home/summary';
import { agentModelLine, agentModelTip, modelProblem, providerOptions } from '../providers';
import { teamForRun } from '../team';
import type { AgentSpec, Health, Team } from '../types';
import { checkAnswer } from '../dev/fixtureApi';
import {
  CONNECTED_KEY, type CheckAnswer, checkBody, connectedAgent, connectedLabel, connectedTeam, connectedTeams, emptyForm, formProblem, parseHeaders, readConnected, saveConnected,
} from './connect';

const http = (more: Partial<AgentSpec> = {}): AgentSpec => ({
  id: 'planner', name: 'Plant planner', kind: 'http', model: null, prompt: '', url: 'https://agent.example.com/arena',
  headers: { Authorization: { secret: '3f9a' } }, ...more,
});
const bedrock = (more: Partial<AgentSpec> = {}): AgentSpec => ({
  id: 'bedrock-agent', name: 'Haiku on Bedrock', kind: 'open', provider: 'bedrock:us-east-1', model: 'anthropic.claude-3-5-haiku-20241022-v1:0', prompt: '', ...more,
});
const team = (...agents: AgentSpec[]): Team => ({ id: 't', name: 'T', agents });
const noModels: Health = { open_available: false, claude: false, codex: false, providers: [] };

describe('the headers box', () => {
  it('reads one Name: value per line, skipping blank lines and keeping colons in values', () => {
    expect(parseHeaders('')).toEqual({ headers: {} });
    expect(parseHeaders('Authorization: Bearer abc\n\n  X-Api-Key:  k:1  \r\n')).toEqual({ headers: { Authorization: 'Bearer abc', 'X-Api-Key': 'k:1' } });
  });
  it('names the first line that is not a header, and a header given twice', () => {
    expect(parseHeaders('Authorization Bearer abc')).toEqual({ error: "Header line 1 isn't “Name: value”." });
    expect(parseHeaders('A: 1\nBad Name: 2')).toEqual({ error: "Header line 2 isn't “Name: value”." });
    expect(parseHeaders('A:')).toEqual({ error: "Header line 1 isn't “Name: value”." });
    expect(parseHeaders('a: 1\nA: 2')).toEqual({ error: 'The header A is there twice.' });
  });
});

describe('the form and the check it sends', () => {
  it('needs an http(s) URL, or a Bedrock model and region', () => {
    expect(formProblem(emptyForm())).toBe("Give the agent's URL.");
    expect(formProblem({ ...emptyForm(), url: 'agent.example.com' })).toBe('The URL should start with https:// or http://.');
    expect(formProblem({ ...emptyForm(), url: 'ftp://agent.example.com' })).toBe('The URL should start with https:// or http://.');
    expect(formProblem({ ...emptyForm(), url: 'http://localhost:8800', headers: 'nope' })).toBe("Header line 1 isn't “Name: value”.");
    expect(formProblem(emptyForm('bedrock'))).toBe('Name a Bedrock model.');
    expect(formProblem({ ...emptyForm('bedrock'), model: 'm', region: 'mars' })).toBe('Name an AWS region, like us-east-1.');
    expect(emptyForm('bedrock').region).toBe('us-east-1');
  });
  it('builds the body arenad takes', () => {
    expect(checkBody(emptyForm())).toBeNull();
    expect(checkBody({ ...emptyForm(), url: ' http://localhost:8800/agent ' })).toEqual({ kind: 'http', url: 'http://localhost:8800/agent' });
    expect(checkBody({ ...emptyForm(), url: 'https://a.example.com', headers: 'Authorization: Bearer x' }))
      .toEqual({ kind: 'http', url: 'https://a.example.com', headers: { Authorization: 'Bearer x' } });
    expect(checkBody({ ...emptyForm('bedrock'), model: ' anthropic.claude-x ', region: 'eu-west-1' }))
      .toEqual({ kind: 'bedrock', model_id: 'anthropic.claude-x', region: 'eu-west-1' });
  });
});

describe('the team made of a check answer', () => {
  it('is one agent, named by what it called itself, run one after another', () => {
    const answer: CheckAnswer = { ok: true, agent: http({ name: '' }), name: 'Plant planner', ms: 142 };
    const a = connectedAgent(answer);
    expect(a.name).toBe('Plant planner');
    expect(connectedTeam(a)).toEqual({ id: 'connected-agent-example-com', name: 'Plant planner', agents: [a], runner: 'chain' });
  });
  it('falls back to the host, or the Bedrock model', () => {
    expect(connectedAgent({ ok: true, agent: http({ name: '' }), ms: 1 }).name).toBe('agent.example.com');
    expect(connectedAgent({ ok: true, agent: bedrock({ name: '' }), ms: 1 }).name).toBe('anthropic.claude-3-5-haiku-20241022-v1:0');
  });
  it('never carries a header value, only the references arenad answered with', () => {
    const leaky = http({ headers: { Authorization: 'Bearer sk-live', 'X-Key': { secret: 'ab12' } } });
    expect(connectedAgent({ ok: true, agent: leaky, ms: 1 }).headers).toEqual({ 'X-Key': { secret: 'ab12' } });
  });
  it('goes to arenad as it came back: the URL and the secret references', () => {
    const run = teamForRun(connectedTeam(http()), 'chain');
    expect(run.agents[0]).toMatchObject({ kind: 'http', url: 'https://agent.example.com/arena', headers: { Authorization: { secret: '3f9a' } } });
    expect(run).not.toHaveProperty('runner');
  });
});

describe('the connected agents this browser keeps', () => {
  let store: Record<string, string>;
  beforeEach(() => {
    store = {};
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => { store[k] = v; },
      removeItem: (k: string) => { delete store[k]; },
    });
  });
  afterEach(() => vi.unstubAllGlobals());

  it('keeps the agent newest first, replacing an earlier connection to the same agent', () => {
    saveConnected(http({ headers: { Authorization: { secret: 'old' } } }));
    saveConnected(bedrock());
    saveConnected(http({ headers: { Authorization: { secret: 'new' } } }));
    const list = readConnected();
    expect(list.map((a) => a.kind)).toEqual(['http', 'open']);
    expect(list[0].headers).toEqual({ Authorization: { secret: 'new' } });
    const teams = connectedTeams();
    expect(teams.map((t) => connectedLabel(t, teams))).toEqual(['Plant planner (connected)', 'Haiku on Bedrock (connected)']);
    saveConnected(http({ url: 'http://localhost:8800/agent' }));
    const more = connectedTeams();
    expect(more.map((t) => connectedLabel(t, more))).toEqual(['Plant planner (connected, localhost:8800)', 'Plant planner (connected, agent.example.com)', 'Haiku on Bedrock (connected)']);
  });
  it('never stores a header value', () => {
    saveConnected(http({ headers: { Authorization: 'Bearer sk-live-123', 'X-Ref': { secret: 'r1' } } }));
    expect(store[CONNECTED_KEY]).not.toContain('sk-live-123');
    expect(readConnected()[0].headers).toEqual({ 'X-Ref': { secret: 'r1' } });
  });
  it('reads nothing from a broken or foreign entry, and survives storage that throws', () => {
    store[CONNECTED_KEY] = '{not json';
    expect(readConnected()).toEqual([]);
    store[CONNECTED_KEY] = JSON.stringify([{ id: 'x', name: 'Claude', kind: 'claude_code', model: null, prompt: '' }, http()]);
    expect(readConnected().map((a) => a.id)).toEqual(['planner']);
    vi.stubGlobal('localStorage', { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } });
    expect(readConnected()).toEqual([]);
    expect(saveConnected(http())).toHaveLength(1);
  });
});

describe('connected agents elsewhere in the UI', () => {
  it('are labelled by where they run', () => {
    expect(agentModelLine(http())).toBe('HTTP agent · agent.example.com');
    expect(agentModelLine({ kind: 'http', model: null, provider: null })).toBe('HTTP agent · Its own model');
    expect(agentModelTip(http())).toBe('https://agent.example.com/arena');
    expect(agentModelLine(bedrock())).toBe('Bedrock · anthropic.claude-3-5-haiku-20241022-v1:0');
    expect(agentModelTip(bedrock({ provider: 'bedrock:eu-west-1' }))).toBe('Bedrock in eu-west-1');
    expect(runLine(team(http()))).toBe('1 agent · HTTP agent · agent.example.com');
    expect(runLine(team(bedrock()))).toBe('1 agent · Bedrock · anthropic.claude-3-5-haiku-20241022-v1:0');
  });
  it('are never blocked for a missing CLI or an unlisted provider', () => {
    expect(missingFor(team(http()), noModels)).toEqual([]);
    expect(missingFor(team(bedrock()), noModels)).toEqual([]);
    expect(missingFor(team(bedrock(), { ...http(), id: 'b', kind: 'claude_code', model: null }), noModels)).toEqual([expect.stringContaining("Claude Code isn't installed")]);
    expect(modelProblem('bedrock:us-east-1', 'anthropic.claude-x', [])).toBeNull();
    expect(modelProblem('bedrock:us-east-1', '', [])).toBe('Name a model for Bedrock');
    expect(providerOptions([{ id: 'openrouter', kind: 'openrouter', label: 'OpenRouter', available: true, checked: true }], 'bedrock:us-east-1').at(-1))
      .toEqual({ id: 'bedrock:us-east-1', text: 'Bedrock (us-east-1)', disabled: false });
  });
  it('an HTTP agent cannot run with a lead dispatching', () => {
    expect(runnerProblem(team(http()), 'dispatch')).toContain("HTTP agent can't run with a lead dispatching");
    expect(runnerProblem(team(http()), 'chain')).toBeNull();
    expect(runnerProblem(team(bedrock()), 'dispatch')).toBeNull();
  });
});

describe('the fixture check', () => {
  it('connects, or fails plainly for a URL or model with "fail" in it', () => {
    expect(checkAnswer({ kind: 'http', url: 'http://localhost:8800', headers: { Authorization: 'Bearer x' } }))
      .toMatchObject({ ok: true, name: 'Plant planner', agent: { kind: 'http', headers: { Authorization: { secret: expect.any(String) } } } });
    expect(JSON.stringify(checkAnswer({ kind: 'http', url: 'http://localhost:8800', headers: { Authorization: 'Bearer x' } }))).not.toContain('Bearer x');
    expect(checkAnswer({ kind: 'http', url: 'https://fail.example.com' })).toMatchObject({ ok: false });
    expect(checkAnswer({ kind: 'bedrock', model_id: 'm', region: 'us-east-1' })).toMatchObject({ ok: true, agent: { provider: 'bedrock:us-east-1', model: 'm' } });
  });
});
