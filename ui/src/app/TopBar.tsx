import { MoreHorizontal } from 'lucide-react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { CORE } from '../core';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { Kbd } from '../components/ui/kbd';
import ReviewToggle from '../review/ReviewToggle';
import { cn } from '@/lib/cn';
import type { Mode } from './palette';
import { setThemePref, useTheme } from './theme';
import { nextPref } from './themeModel';

/** Each section of the app and the addresses that belong to it. */
const NAV = [
  { to: '/', label: 'Experiments', match: /^\/($|experiments)/ },
  { to: '/runs', label: 'Runs', match: /^\/(run|runs|control|world|compare)(\/|$)/ },
];

const THEME_WORD = { system: 'System', light: 'Light', dark: 'Dark' } as const;

/** "⋯": everything that isn't on the first screen. */
function MoreMenu({ onPalette }: { onPalette: (start?: Mode) => void }) {
  const nav = useNavigate();
  const { pref } = useTheme();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="icon" className="text-muted" aria-label="More"><MoreHorizontal /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-64">
        <DropdownMenuLabel>Tools</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => nav('/run')}>Run once</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'compare-runs', first: null })}>Compare two runs…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'world' })}>World view of a run…</DropdownMenuItem>
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

/** The wordmark, Experiments and Runs, and "⋯" for everything else. */
export default function TopBar({ onPalette }: { onPalette: (start?: Mode) => void }) {
  const { pathname } = useLocation();
  return (
    <div className="sticky top-0 z-40 flex h-12 shrink-0 items-center gap-2 border-b border-line bg-bg/85 px-4 backdrop-blur sm:gap-6 sm:px-6">
      <Link to="/" className="flex items-center gap-2 text-sm font-semibold tracking-tight">
        <span aria-hidden="true" className="grid size-5 place-items-center rounded-[5px] bg-fg font-mono text-2xs text-bg">A</span>
        <span className="hidden sm:inline">Arena</span>
      </Link>
      <nav className="flex items-center gap-1" aria-label="Main">
        {NAV.map((n) => {
          const on = n.match.test(pathname);
          return (
            <Link key={n.to} to={n.to} aria-current={on ? 'page' : undefined}
              className={cn('rounded-md px-2.5 py-1 text-sm transition-colors', on ? 'bg-sunken font-medium text-fg' : 'text-muted hover:text-fg')}>
              {n.label}
            </Link>
          );
        })}
      </nav>
      <span className="flex-1" />
      <MoreMenu onPalette={onPalette} />
    </div>
  );
}
