import { describe, expect, it } from 'vitest';
import { coreUrl, normalizeCore, resolveCore } from './core';

describe('core URL', () => {
  it('keeps only an http(s) base without a trailing slash', () => {
    expect(normalizeCore(' http://127.0.0.1:4513/ ')).toBe('http://127.0.0.1:4513');
    expect(normalizeCore('https://arena.example.com/core/')).toBe('https://arena.example.com/core');
    expect(normalizeCore('')).toBe('');
    expect(normalizeCore(undefined)).toBe('');
    expect(normalizeCore('javascript:alert(1)')).toBeNull();
    expect(normalizeCore('not a url')).toBeNull();
  });

  it('takes ?core= first and remembers it, then the remembered one, then the build, else same origin', () => {
    expect(resolveCore('?core=http://h:1/', 'http://old:2', 'http://built:3')).toEqual({ base: 'http://h:1', remember: 'http://h:1' });
    expect(resolveCore('?core=', 'http://old:2', 'http://built:3')).toEqual({ base: '', remember: '' });
    expect(resolveCore('?core=ftp://x', 'http://old:2', undefined)).toEqual({ base: 'http://old:2', remember: null });
    expect(resolveCore('', null, 'http://built:3/')).toEqual({ base: 'http://built:3', remember: null });
    expect(resolveCore('?run=r', null, undefined)).toEqual({ base: '', remember: null });
  });

  it('puts every path on the core', () => {
    expect(coreUrl('/api/branches/b%201/stream', 'http://h:1')).toBe('http://h:1/api/branches/b%201/stream');
    expect(coreUrl('/api/health', '')).toBe('/api/health');
  });
});
