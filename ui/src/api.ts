import type {
  Branch, BranchDetail, Change, CompareResponse, Event, ForkResponse, Health, NewRunResponse,
  ProviderStatus, RestoreMode, RestorePlan, RowsResponse, RunSummary, StatusEvent, TaskSummary, Team,
} from './types';

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  const text = await res.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    /* non-JSON body */
  }
  if (!res.ok) {
    const msg = (body as { error?: string } | null)?.error ?? (text || res.statusText);
    throw new ApiError(res.status, msg);
  }
  return body as T;
}

const post = <T>(path: string, body: unknown) =>
  call<T>(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

export const deleteRun = (id: string) =>
  call<{ dropped: number }>(`/api/runs/${encodeURIComponent(id)}`, { method: 'DELETE' });
export const health = () => call<Health>('/api/health');
/** Every model provider; asks local servers (and OpenAI/Anthropic when keyed) for their models, 3 s each. */
export const providers = () => call<ProviderStatus[]>('/api/providers');
export const tasks = () => call<TaskSummary[]>('/api/tasks');
export const defaultTeam = () => call<Team>('/api/team/default');
export const startRun = (task_id: string, team: Team) => post<NewRunResponse>('/api/runs', { task_id, team });
export const runs = () => call<RunSummary[]>('/api/runs');
export const branch = (id: string) => call<BranchDetail>(`/api/branches/${encodeURIComponent(id)}`);
export const rows = (id: string, seq: number, keys: string[]) =>
  call<RowsResponse>(`/api/branches/${encodeURIComponent(id)}/rows?seq=${seq}&keys=${encodeURIComponent(keys.join(','))}`);
export const fork = (id: string, at_seq: number, changes: Record<string, Change> = {}, restore: RestoreMode = 'exact') =>
  post<ForkResponse>(`/api/branches/${encodeURIComponent(id)}/fork`, { at_seq, changes, restore });
/** How each part of a fork at `at_seq` would be restored; a 409 names the part that can't be. */
export const restorePlan = (id: string, at_seq: number, restore: RestoreMode) =>
  call<RestorePlan>(`/api/branches/${encodeURIComponent(id)}/restore-plan?at_seq=${at_seq}&restore=${restore}`);
export const compare = (a: string, b: string) =>
  call<CompareResponse>(`/api/compare?a=${encodeURIComponent(a)}&b=${encodeURIComponent(b)}`);

/** Subscribe to a branch's SSE feed. Returns a function that closes it. */
export function stream(
  id: string,
  onEvent: (e: Event) => void,
  onStatus: (s: StatusEvent) => void,
): () => void {
  const es = new EventSource(`/api/branches/${encodeURIComponent(id)}/stream`);
  const listen = <T>(name: string, fn: (v: T) => void) =>
    es.addEventListener(name, (m) => {
      try {
        fn(JSON.parse((m as MessageEvent).data) as T);
      } catch {
        /* malformed frame: skip */
      }
    });
  listen<Event>('event', onEvent);
  listen<StatusEvent>('status', onStatus);
  return () => es.close();
}

export type { Branch };
