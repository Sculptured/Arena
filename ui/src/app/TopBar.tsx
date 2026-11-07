import { Monitor, Moon, Search, Sun } from 'lucide-react';
import { NavLink } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { Kbd } from '../components/ui/kbd';
import { Tooltip } from '../components/ui/tooltip';
import { cn } from '@/lib/cn';
import { setThemePref, useTheme } from './theme';
import { nextPref } from './themeModel';

const NAV = [
  { to: '/', label: 'Runs', end: true },
  { to: '/suites', label: 'Suites', end: false },
  { to: '/team', label: 'Team', end: false },
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

/** The wordmark, the three places, the palette and the theme. */
export default function TopBar({ onPalette }: { onPalette: () => void }) {
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
      <Button className="w-9 justify-start px-2 text-muted sm:w-64" onClick={onPalette} aria-label="Open the command palette">
        <Search /><span className="hidden flex-1 text-left font-normal sm:inline">Go to, new run, compare…</span>
        <Kbd className="hidden sm:inline-flex">⌘K</Kbd>
      </Button>
      <ThemeButton />
    </div>
  );
}
