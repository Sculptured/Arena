import { MoreHorizontal } from 'lucide-react';
import { NavLink, useLocation, useNavigate } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { CORE } from '../core';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { Kbd } from '../components/ui/kbd';
import ReviewToggle from '../review/ReviewToggle';
import { cn } from '@/lib/cn';
import { type Mode, hereItems, hereOf } from './palette';
import { setThemePref, useTheme } from './theme';
import { nextPref } from './themeModel';

const NAV = [
  { to: '/', label: 'Run', end: true },
  { to: '/history', label: 'History', end: false },
];

const THEME_WORD = { system: 'System', light: 'Light', dark: 'Dark' } as const;

/** The views of the run on screen: its world view, all its details and its process view. */
function HereItems() {
  const nav = useNavigate();
  const { pathname, search } = useLocation();
  const here = hereItems(hereOf(pathname, search));
  if (!here.length) return null;
  return (
    <>
      <DropdownMenuLabel>This run</DropdownMenuLabel>
      {here.map((i) => 'go' in i.action && <DropdownMenuItem key={i.id} onSelect={() => nav((i.action as { go: string }).go)}>{i.label}</DropdownMenuItem>)}
    </>
  );
}

/** "⋯": everything that isn't on the first screen. */
function MoreMenu({ onPalette }: { onPalette: (start?: Mode) => void }) {
  const nav = useNavigate();
  const { pref } = useTheme();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="icon" className="text-muted" aria-label="More"><MoreHorizontal /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-64">
        <HereItems />
        <DropdownMenuLabel>Test and compare</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => nav('/suites')}>Suites</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'compare-runs', first: null })}>Compare two runs…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'compare-suites', first: null })}>Compare two suites…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'world' })}>World view of a run…</DropdownMenuItem>
        <DropdownMenuLabel>Team</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => nav('/team')}>Team editor</DropdownMenuItem>
        <ReviewToggle />
        <DropdownMenuLabel>App</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => onPalette()}><span className="flex-1">Command palette</span><Kbd>⌘K</Kbd></DropdownMenuItem>
        <DropdownMenuItem onSelect={(e) => { e.preventDefault(); setThemePref(nextPref(pref)); }}>
          <span className="flex-1">Theme</span><span className="text-2xs text-muted">{THEME_WORD[pref]}</span>
        </DropdownMenuItem>
        <DropdownMenuLabel className="font-normal" title="Open the UI with ?core=<url> to use another Arena core, or ?core= to go back to this one">
          Arena core: {CORE || 'this server'}
        </DropdownMenuLabel>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The wordmark, Run and History, and "⋯" for everything else. */
export default function TopBar({ onPalette }: { onPalette: (start?: Mode) => void }) {
  return (
    <div className="sticky top-0 z-40 flex h-11 shrink-0 items-center gap-2 bg-bg px-3 sm:gap-4 sm:px-5">
      <span className="flex items-center gap-1.5 pr-1 text-sm font-semibold tracking-tight">
        <span aria-hidden="true" className="grid size-5 place-items-center rounded-sm bg-fg font-mono text-2xs text-surface">A</span>
        <span className="hidden sm:inline">Arena</span>
      </span>
      <nav className="flex items-center gap-1" aria-label="Main">
        {NAV.map((n) => (
          <NavLink key={n.to} to={n.to} end={n.end}
            className={({ isActive }) => cn('rounded-md px-2.5 py-1 text-sm text-muted hover:text-fg', isActive && 'font-medium text-fg')}>
            {n.label}
          </NavLink>
        ))}
      </nav>
      <span className="flex-1" />
      <MoreMenu onPalette={onPalette} />
    </div>
  );
}
