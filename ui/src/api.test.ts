import { afterEach, describe, expect, it, vi } from 'vitest';
import { stream } from './api';
import type { Event } from './types';

class FakeEventSource {
  static last: FakeEventSource;
  listeners = new Map<string, (e: MessageEvent) => void>();
  closed = false;
  constructor(public url: string) {
    FakeEventSource.last = this;
  }
  addEventListener(name: string, fn: (e: MessageEvent) => void) {
    this.listeners.set(name, fn);
  }
  close() {
    this.closed = true;
  }
  emit(name: string, data: unknown) {
    this.listeners.get(name)?.({ data: JSON.stringify(data) } as MessageEvent);
  }
}

afterEach(() => vi.unstubAllGlobals());

describe('api', () => {
  it('stream_parses_events', () => {
    vi.stubGlobal('EventSource', FakeEventSource);
    const events: Event[] = [];
    const statuses: { status: string; error: string | null }[] = [];
    const stop = stream('b1', (e) => events.push(e), (s) => statuses.push(s));
    const es = FakeEventSource.last;
    expect(es.url).toBe('/api/branches/b1/stream');

    const ev: Event = {
      branch_id: 'b1',
      seq: 3,
      at: '2026-10-30T00:00:00Z',
      agent: 'a1',
      kind: 'tool_call',
      payload: { name: 'find_user' },
    };
    es.emit('event', ev);
    es.emit('status', { status: 'passed', error: null });
    es.listeners.get('event')?.({ data: 'not json' } as MessageEvent); // ignored, no throw

    expect(events).toEqual([ev]);
    expect(statuses).toEqual([{ status: 'passed', error: null }]);
    stop();
    expect(es.closed).toBe(true);
  });
});
