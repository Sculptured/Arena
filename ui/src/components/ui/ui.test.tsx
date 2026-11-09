import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ColumnDef } from '@tanstack/react-table';
import { Button } from './button';
import { DataTable } from './data-table';
import { Dialog, DialogContent } from './dialog';
import { FilterChips } from './filter-chips';
import { Checkbox, Field, Input, NativeSelect, Textarea } from './input';
import { Kbd } from './kbd';
import { PageHeader, Stat } from './page';
import { Panel, PanelHeader } from './panel';
import { Split } from './split';
import { EmptyState, ErrorState, LoadingState, Notice } from './state';
import { StatusMark, Tag } from './status';
import { RowTh, Table, Td, Th } from './table';
import { Tabs, TabsContent, TabsList, TabsTrigger } from './tabs';
import { Tooltip, TooltipProvider } from './tooltip';

const html = (n: React.ReactNode) => renderToStaticMarkup(<TooltipProvider>{n}</TooltipProvider>);
const text = (h: string) => h.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();

describe('base components', () => {
  it('button: variants, a type by default, and asChild for links', () => {
    expect(html(<Button variant="primary">Go</Button>)).toMatch(/<button[^>]*class="[^"]*bg-primary[^"]*"[^>]*type="button"/);
    expect(html(<Button asChild><a href="/x">Link</a></Button>)).toMatch(/^<a [^>]*href="\/x"[^>]*>Link<\/a>$/);
  });
  it('status mark: glyph and word always; colour only when abnormal', () => {
    const ok = html(<StatusMark mark={{ glyph: '✓', word: 'passed', abnormal: false }} />);
    const bad = html(<StatusMark mark={{ glyph: '✕', word: 'failed', abnormal: true }} />);
    expect(text(ok)).toBe('✓ passed');
    expect(ok).not.toContain('text-alarm');
    expect(bad).toContain('text-alarm');
    expect(html(<Tag tone="warn">approximate</Tag>)).toContain('text-warn');
  });
  it('states say what happened and what to do next', () => {
    expect(text(html(<EmptyState title="No runs yet" hint="Start one." />))).toBe('No runs yet Start one.');
    const err = html(<ErrorState error="boom" onRetry={() => {}} />);
    expect(err).toContain('role="alert"');
    expect(text(err)).toContain('Check that arenad is running');
    expect(text(err)).toContain('Try again');
    expect(html(<LoadingState label="Loading runs…" rows={2} />)).toContain('aria-busy="true"');
    expect(text(html(<Notice onDismiss={() => {}}>moved</Notice>))).toBe('moved Dismiss');
  });
  it('panel, page header, stat and table', () => {
    expect(text(html(<Panel><PanelHeader title="Trace" meta="the turn" /></Panel>))).toBe('Trace the turn');
    expect(text(html(<PageHeader title="Runs" meta="27 runs" actions={<Button>New</Button>} />))).toBe('Runs New 27 runs');
    expect(html(<Stat label="Refused" value="1" abnormal />)).toContain('text-alarm');
    expect(html(<Table><thead><tr><Th>A</Th></tr></thead><tbody><tr><RowTh>r</RowTh><Td>1</Td></tr></tbody></Table>)).toContain('scope="row"');
  });
  it('form controls carry their label', () => {
    const f = html(<Field label="Task" hint="pick one"><NativeSelect><option>a</option></NativeSelect></Field>);
    expect(text(f)).toBe('Task a pick one');
    expect(html(<Input placeholder="x" />)).toContain('placeholder="x"');
    expect(html(<Textarea rows={2} />)).toContain('rows="2"');
    expect(html(<Checkbox checked readOnly />)).toContain('type="checkbox"');
    expect(text(html(<Kbd>⌘K</Kbd>))).toBe('⌘K');
  });
  it('filter chips: one checked, each with its count', () => {
    const h = html(<FilterChips label="Status" options={[{ id: 'all', label: 'All' }, { id: 'failed', label: 'Failed' }]} value="failed" counts={{ all: 3, failed: 0 }} onChange={() => {}} />);
    expect(h).toMatch(/aria-checked="true"[^>]*>Failed/);
    expect(text(h)).toBe('All 3 Failed 0');
  });
  it('tabs render the active panel; a closed dialog renders nothing; a tooltip keeps its trigger', () => {
    const t = html(<Tabs defaultValue="a"><TabsList><TabsTrigger value="a">A</TabsTrigger><TabsTrigger value="b">B</TabsTrigger></TabsList><TabsContent value="a">first</TabsContent><TabsContent value="b">second</TabsContent></Tabs>);
    expect(text(t)).toContain('first');
    expect(text(t)).not.toContain('second');
    expect(html(<Dialog open={false}><DialogContent title="x">body</DialogContent></Dialog>)).toBe('');
    expect(text(html(<Tooltip content="why"><button>i</button></Tooltip>))).toBe('i');
  });
  it('split: a main area, a resize handle and a side panel', () => {
    const h = html(<Split storageKey="k" main={<p>main</p>} side={<p>side</p>} />);
    expect(h).toContain('role="separator"');
    expect(text(h)).toBe('main side');
  });
  it('data table: loading, error, empty, rows, and no match', () => {
    const cols: ColumnDef<{ id: string }, unknown>[] = [{ id: 'id', header: 'Id', accessorFn: (r) => r.id }];
    const base = { columns: cols, getRowId: (r: { id: string }) => r.id, loadingLabel: 'Loading…', empty: <p>none yet</p>, label: 'T' };
    expect(text(html(<DataTable {...base} data={null} />))).toContain('Loading…');
    expect(html(<DataTable {...base} data={null} error="down" />)).toContain('role="alert"');
    expect(text(html(<DataTable {...base} data={[]} />))).toBe('none yet');
    expect(text(html(<DataTable {...base} data={[{ id: 'r1' }, { id: 'r2' }]} />))).toBe('Id r1 r2');
    expect(text(html(<DataTable {...base} data={[{ id: 'r1' }]} filter="zz" filterFn={(r, f) => r.id.includes(String(f))} />))).toContain('Nothing matches the filter.');
  });
});
