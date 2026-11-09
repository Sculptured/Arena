import { ChevronDown, FileUp, Pencil, PlugZap, Users } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import ImportDialog from '../board/ImportDialog';
import { type ConnectFrom, connectedTeams } from '../connect/connect';
import { agentModelLine } from '../providers';
import TeamBoard from '../board/TeamBoard';
import { Button } from '../components/ui/button';
import { SidePanel } from '../components/ui/side-panel';
import { LoadingState } from '../components/ui/state';
import { Dialog, DialogContent } from '../components/ui/dialog';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { RUNNER_WORDS } from './runners';
import { cn } from '@/lib/cn';
import type { Runner, Team } from '../types';

/** The board with the picked card's prompt over its right side. */
function BoardView({ team, runner, onExpand, className }: { team: Team; runner: Runner; onExpand?: () => void; className?: string }) {
  const [picked, setPicked] = useState<string | null>(null);
  const agent = team.agents.find((a) => a.id === picked) ?? null;
  return (
    <div className={cn('relative min-h-0 overflow-hidden', className)}>
      <TeamBoard team={team} runner={runner} selected={picked} onSelect={setPicked} onExpand={onExpand} className="h-full" />
      {agent && (
        <SidePanel title={agent.name} meta={`${agentModelLine(agent)}${agent.read_only ? ' · Read-only' : ''}`} onClose={() => setPicked(null)}
          className="absolute inset-y-0 right-0 z-10 w-80 border-l shadow-pop">
          <p className="px-3 pt-2 text-2xs font-medium text-muted">Prompt</p>
          <p className="px-3 pb-3 text-xs leading-5 whitespace-pre-wrap">{agent.prompt || 'No prompt.'}</p>
        </SidePanel>
      )}
    </div>
  );
}

/** One suggested team in the picker: its name, its size and how it runs. */
function suggestionLine(t: Team): string {
  const n = t.agents.length;
  const size = n === 1 ? 'one agent' : `${n} agents`;
  return t.runner && t.runner !== 'chain' ? `${size} · ${RUNNER_WORDS[t.runner]}` : size;
}

function PickItem({ t, line, current, onPick }: { t: Team; line: string; current: Team | null; onPick: (t: Team) => void }) {
  return (
    <DropdownMenuItem onSelect={() => onPick({ ...t, picked: true })} className={current?.id === t.id ? 'bg-accent-soft' : ''}>
      <span className="flex min-w-0 flex-col">
        <span className="truncate text-sm">{t.name}</span>
        <span className="truncate text-2xs text-muted">{line}</span>
      </span>
    </DropdownMenuItem>
  );
}

/** The benchmark's suggested teams and your connected agents, one click away; picking one replaces the team (and its runner). */
function TeamPicker({ teams, connected, current, onPick }: { teams: Team[]; connected: Team[]; current: Team | null; onPick: (t: Team) => void }) {
  const suggested = teams.length >= 2 ? teams : [];
  if (!suggested.length && !connected.length) return null;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button size="sm"><Users />{suggested.length ? 'Suggested' : 'Connected'}<ChevronDown /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-72">
        {suggested.length > 0 && <DropdownMenuLabel>The benchmark's teams</DropdownMenuLabel>}
        {suggested.map((t) => <PickItem key={t.id} t={t} line={suggestionLine(t)} current={current} onPick={onPick} />)}
        {connected.length > 0 && <DropdownMenuLabel className={suggested.length ? 'mt-1 border-t border-line pt-2' : ''}>Your connected agents</DropdownMenuLabel>}
        {connected.map((t) => <PickItem key={t.id} t={t} line={agentModelLine(t.agents[0])} current={current} onPick={onPick} />)}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const FROM_RUN: ConnectFrom = { from: '/run' };

/** The team's buttons, under Options: the suggested teams (and connected agents), Import, Connect and Edit. */
export function TeamActions({ team, suggested = [], onImported }: { team: Team | null; suggested?: Team[]; onImported: (t: Team) => void }) {
  const [importing, setImporting] = useState(false);
  const [connected] = useState(connectedTeams);
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <TeamPicker teams={suggested} connected={connected} current={team} onPick={onImported} />
      <Button size="sm" onClick={() => setImporting(true)}><FileUp />Import</Button>
      <Button size="sm" asChild><Link to="/connect" state={FROM_RUN}><PlugZap />Connect</Link></Button>
      <Button size="sm" asChild><Link to="/team"><Pencil />Edit</Link></Button>
      <ImportDialog open={importing} onOpenChange={setImporting} onImported={onImported} />
    </div>
  );
}

/** The team as a read-only board (Expand opens it full screen); a card's prompt opens beside it. */
export default function TeamCard({ team, runner }: { team: Team | null; runner: Runner }) {
  const [expanded, setExpanded] = useState(false);
  if (!team) return <LoadingState label="Loading your team…" rows={4} />;
  return (
    <div className="flex min-h-0 flex-col" title={team.name}>
      <BoardView key={team.id} team={team} runner={runner} onExpand={() => setExpanded(true)} className="h-72 rounded-lg bg-surface" />
      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent title={team.name} className="top-4 left-4 h-[calc(100vh-32px)] max-h-none w-[calc(100vw-32px)] translate-x-0">
          <BoardView team={team} runner={runner} className="flex-1" />
        </DialogContent>
      </Dialog>
    </div>
  );
}
