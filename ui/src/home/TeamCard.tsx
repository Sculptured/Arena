import { FileUp, Pencil } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import ImportDialog from '../board/ImportDialog';
import { agentModelLine } from '../providers';
import TeamBoard from '../board/TeamBoard';
import { Button } from '../components/ui/button';
import { Panel, PanelHeader } from '../components/ui/panel';
import { SidePanel } from '../components/ui/side-panel';
import { LoadingState } from '../components/ui/state';
import { Dialog, DialogContent } from '../components/ui/dialog';
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

/** The team as a read-only board, with Import, Edit and Expand (the board full screen); a card's prompt opens beside it. */
export default function TeamCard({ team, runner, onImported }: { team: Team | null; runner: Runner; onImported: (t: Team) => void }) {
  const [importing, setImporting] = useState(false);
  const [expanded, setExpanded] = useState(false);
  return (
    <Panel className="flex min-h-0 flex-col">
      <PanelHeader title="Team" meta={team?.name}>
        <Button size="sm" onClick={() => setImporting(true)}><FileUp />Import</Button>
        <Button size="sm" asChild><Link to="/team"><Pencil />Edit</Link></Button>
      </PanelHeader>
      {!team ? <LoadingState label="Loading your team…" rows={4} />
        : <BoardView key={team.id} team={team} runner={runner} onExpand={() => setExpanded(true)} className="h-80 flex-1 rounded-b-lg" />}
      {team && (
        <Dialog open={expanded} onOpenChange={setExpanded}>
          <DialogContent title={team.name} description={`${team.agents.length} agents · scroll to move, Ctrl + scroll or pinch to zoom, drag to pan`}
            className="top-4 left-4 h-[calc(100vh-32px)] max-h-none w-[calc(100vw-32px)] translate-x-0">
            <BoardView team={team} runner={runner} className="flex-1" />
          </DialogContent>
        </Dialog>
      )}
      <ImportDialog open={importing} onOpenChange={setImporting} onImported={onImported} />
    </Panel>
  );
}
