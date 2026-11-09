import { ArrowRight, Factory, GitCompare, Layers, ListTree, Monitor, Play } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { runs as fetchRuns } from '../api';
import { experiments as fetchExperiments } from '../experiments/api';
import type { ListedExperiment } from '../experiments/types';
import type { RunSummary } from '../types';
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from '../components/ui/command';
import { Dialog, DialogContent } from '../components/ui/dialog';
import { Kbd } from '../components/ui/kbd';
import { hasModifier } from '@/lib/keys';
import { type Action, type Item, type Mode, hereOf, itemsFor, placeholderOf } from './palette';
import { setThemePref, useTheme } from './theme';
import { nextPref } from './themeModel';

/** ⌘K / Ctrl+K anywhere toggles the palette. */
export function usePaletteShortcut(toggle: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey) && !e.altKey) { e.preventDefault(); toggle(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [toggle]);
}

/** Runs and experiments, fetched each time the palette opens. */
function useLists(open: boolean) {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [experiments, setExperiments] = useState<ListedExperiment[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    fetchRuns().then(setRuns).catch((e) => setError(String(e.message ?? e)));
    fetchExperiments().then(setExperiments).catch((e) => setError(String(e.message ?? e)));
  }, [open]);
  return { runs, experiments, error };
}

function iconOf(item: Item) {
  if (item.id.startsWith('new-')) return <Play aria-hidden="true" />;
  if (item.id.startsWith('world') || item.group === 'World view of') return <Factory aria-hidden="true" />;
  if (item.id.startsWith('cmp-') || item.group === 'Compare with') return <GitCompare aria-hidden="true" />;
  if (item.group === 'Go to') return <ArrowRight aria-hidden="true" />;
  if (item.id === 'theme') return <Monitor aria-hidden="true" />;
  return item.id.startsWith('experiment-') ? <Layers aria-hidden="true" /> : <ListTree aria-hidden="true" />;
}

function groupsOf(items: Item[]): [string, Item[]][] {
  const out = new Map<string, Item[]>();
  for (const i of items) out.set(i.group, [...(out.get(i.group) ?? []), i]);
  return [...out];
}

function Footer({ nested }: { nested: boolean }) {
  return (
    <div className="flex items-center gap-3 border-t border-line px-3 py-1.5 text-2xs text-muted">
      <span><Kbd>↑</Kbd> <Kbd>↓</Kbd> move</span><span><Kbd>↵</Kbd> open</span>
      {nested && <span><Kbd>⌫</Kbd> back</span>}<span className="ml-auto"><Kbd>esc</Kbd> close</span>
    </div>
  );
}

/** The mode (opened for one thing, such as the ⋯ menu's "Compare two runs…", it starts there) and what it lists. */
function useItems(open: boolean, start?: Mode) {
  const [mode, setMode] = useState<Mode>({ kind: 'root' });
  const lists = useLists(open);
  const { pathname, search } = useLocation();
  useEffect(() => { if (open && start) setMode(start); }, [open, start]);
  const items = useMemo(() => itemsFor(mode, lists.runs, lists.experiments, hereOf(pathname, search)), [mode, lists.runs, lists.experiments, pathname, search]);
  return { mode, setMode, items, lists };
}

/** The palette: go to an experiment, a run or a page; start one; compare two runs. */
export default function CommandPalette({ open, onOpenChange, start }: { open: boolean; onOpenChange: (o: boolean) => void; start?: Mode }) {
  const nav = useNavigate();
  const theme = useTheme();
  const [query, setQuery] = useState('');
  const { mode, setMode, items, lists } = useItems(open, start);
  const close = (o: boolean) => { onOpenChange(o); if (!o) { setMode({ kind: 'root' }); setQuery(''); } };
  const run = (a: Action) => {
    if ('mode' in a) { setMode(a.mode); setQuery(''); return; }
    if ('theme' in a) setThemePref(nextPref(theme.pref));
    else nav(a.go);
    close(false);
  };
  const back = (e: React.KeyboardEvent) => { if (e.key === 'Backspace' && !query && mode.kind !== 'root' && !hasModifier(e)) setMode({ kind: 'root' }); };
  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent title="Command palette" bare className="top-[10vh]">
        <Command loop onKeyDown={back}>
          <CommandInput value={query} onValueChange={setQuery} placeholder={placeholderOf(mode)} />
          <CommandList>
            <CommandEmpty>{lists.error ? `Couldn't load runs and experiments: ${lists.error}` : 'Nothing matches. Try a task name, an id or a status.'}</CommandEmpty>
            {groupsOf(items).map(([group, list]) => (
              <CommandGroup key={group} heading={group}>
                {list.map((i) => (
                  <CommandItem key={i.id} value={`${i.label} ${i.id}`} keywords={i.keywords} onSelect={() => run(i.action)}>
                    {iconOf(i)}<span className="min-w-0 flex-1 truncate">{i.label}</span>
                    {i.hint && <span className="shrink-0 font-mono text-2xs text-muted">{i.hint}</span>}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
          <Footer nested={mode.kind !== 'root'} />
        </Command>
      </DialogContent>
    </Dialog>
  );
}
