import { ChevronDown, Monitor, Moon, Search, Sun } from 'lucide-react';
import { NavLink, useLocation, useNavigate } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { Kbd } from '../components/ui/kbd';
import { Tooltip } from '../components/ui/tooltip';
import { cn } from '@/lib/cn';
import { type Mode, hereItems, hereOf } from './palette';
import { setThemePref, useTheme } from './theme';
import { nextPref } from './themeModel';

const NAV = [
  { to: '/', label: 'Run', end: true },
  { to: '/history', label: 'History', end: false },
];

function ThemeButton() {
  const { pref } = useTheme();
  const Icon = pref === 'dark' ? Moon : pref === 'light' ? Sun : Monitor;
  return (
    <Tooltip content={`Theme: ${pref}. Click for ${nextPref(pref)}.`}>
      <Button variant="ghost" size="icon" aria-label={`Theme: ${pref}`} onClick={() => setThemePref(nextPref(pref))}><Icon /></Button>
    </Tooltip>
  );
}

/** Everything that isn't on the first screen: suites, comparisons, the team editor, and the deeper views of the run on screen. */
function AdvancedMenu({ onPalette }: { onPalette: (start?: Mode) => void }) {
  const nav = useNavigate();
  const { pathname, search } = useLocation();
  const here = hereItems(hereOf(pathname, search));
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" className="text-muted">Advanced<ChevronDown /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-72">
        {here.length > 0 && <DropdownMenuLabel>This run</DropdownMenuLabel>}
        {here.map((i) => 'go' in i.action && <DropdownMenuItem key={i.id} onSelect={() => nav((i.action as { go: string }).go)}>{i.label}</DropdownMenuItem>)}
        <DropdownMenuLabel>Test and compare</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => nav('/suites')}>Suites</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'compare-runs', first: null })}>Compare two runs…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'compare-suites', first: null })}>Compare two suites…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPalette({ kind: 'world' })}>World view of a run…</DropdownMenuItem>
        <DropdownMenuLabel>Team</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => nav('/team')}>Team editor</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The wordmark, Run and History, the Advanced menu, the palette and the theme. */
export default function TopBar({ onPalette }: { onPalette: (start?: Mode) => void }) {
  return (
    <div className="sticky top-0 z-40 flex h-10 shrink-0 items-center gap-1 border-b border-line bg-surface px-2 sm:gap-3 sm:px-3">
      <span className="flex items-center gap-1.5 pr-1 text-sm font-semibold tracking-tight">
        <span aria-hidden="true" className="grid size-5 place-items-center rounded-sm bg-fg font-mono text-2xs text-surface">A</span>
        <span className="hidden sm:inline">Arena</span>
      </span>
      <nav className="flex items-center gap-0.5" aria-label="Main">
        {NAV.map((n) => (
          <NavLink key={n.to} to={n.to} end={n.end}
            className={({ isActive }) => cn('rounded-md px-2 py-1 text-sm text-muted hover:bg-hover hover:text-fg', isActive && 'bg-hover font-medium text-fg')}>
            {n.label}
          </NavLink>
        ))}
      </nav>
      <span className="flex-1" />
      <AdvancedMenu onPalette={onPalette} />
      <Button variant="ghost" className="w-9 justify-start px-2 text-muted sm:w-auto" onClick={() => onPalette()} aria-label="Open the command palette">
        <Search /><Kbd className="hidden sm:inline-flex">⌘K</Kbd>
      </Button>
      <ThemeButton />
    </div>
  );
}
