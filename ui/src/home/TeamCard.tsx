import { FileUp, Pencil } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import ImportDialog from '../board/ImportDialog';
import { modelBadge } from '../board/board';
import TeamBoard from '../board/TeamBoard';
import { Button } from '../components/ui/button';
import { Panel, PanelHeader } from '../components/ui/panel';
import { SidePanel } from '../components/ui/side-panel';
import { LoadingState } from '../components/ui/state';
import type { Runner, Team } from '../types';

/** The team as a read-only board, with Import and Edit; a card's prompt opens beside it. */
export default function TeamCard({ team, runner, onImported }: { team: Team | null; runner: Runner; onImported: (t: Team) => void }) {
  const [picked, setPicked] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const agent = team?.agents.find((a) => a.id === picked) ?? null;
  return (
    <Panel className="flex min-h-0 flex-col">
      <PanelHeader title="Team" meta={team?.name}>
        <Button size="sm" onClick={() => setImporting(true)}><FileUp />Import</Button>
        <Button size="sm" asChild><Link to="/team"><Pencil />Edit</Link></Button>
      </PanelHeader>
      {!team ? <LoadingState label="Loading your team…" rows={4} /> : (
        <div className="flex min-h-0 flex-1">
          <TeamBoard team={team} runner={runner} selected={picked} onSelect={setPicked} className="h-auto min-h-72 flex-1" />
          {agent && (
            <SidePanel title={agent.name} meta={`${modelBadge(agent)}${agent.read_only ? ' · Read-only' : ''}`} onClose={() => setPicked(null)} className="w-72 shrink-0 border-l">
              <p className="px-3 pt-2 text-2xs font-medium text-muted">Prompt</p>
              <p className="px-3 pb-3 text-xs leading-5 whitespace-pre-wrap">{agent.prompt || 'No prompt.'}</p>
            </SidePanel>
          )}
        </div>
      )}
      <ImportDialog open={importing} onOpenChange={setImporting} onImported={(t) => { setPicked(null); onImported(t); }} />
    </Panel>
  );
}
