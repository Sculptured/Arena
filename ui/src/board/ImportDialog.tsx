import { FileUp } from 'lucide-react';
import { useRef, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '../components/ui/button';
import { Dialog, DialogContent } from '../components/ui/dialog';
import { Textarea } from '../components/ui/input';
import { ErrorState } from '../components/ui/state';
import { cn } from '@/lib/cn';
import type { Team } from '../types';
import { importTeam } from './importTeam';

/** Import a team: a Sculptured design or an Arena team, picked, dropped or pasted. */
export default function ImportDialog({ open, onOpenChange, onImported }: { open: boolean; onOpenChange: (o: boolean) => void; onImported: (t: Team) => void }) {
  const [error, setError] = useState<string | null>(null);
  const [text, setText] = useState('');
  const [over, setOver] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const take = (raw: string) => {
    const r = importTeam(raw);
    if ('error' in r) { setError(r.error); return; }
    setError(null); setText('');
    onImported(r.team);
    onOpenChange(false);
    toast(`Imported ${r.team.name}`, { description: `${r.team.agents.length} agents from ${r.from === 'Sculptured' ? 'a Sculptured design' : 'a Shakedown team'}.` });
  };
  const read = (f: File | undefined) => { if (f) f.text().then(take, (e) => setError(String(e))); };
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) setError(null); onOpenChange(o); }}>
      <DialogContent title="Import a team" description="A Sculptured design file (.json) or a Shakedown team. Claude agents run as Claude Code, Codex agents as Codex.">
        <div className="flex flex-col gap-3 overflow-auto p-4">
          <button type="button" onClick={() => file.current?.click()}
            onDragOver={(e) => { e.preventDefault(); setOver(true); }} onDragLeave={() => setOver(false)}
            onDrop={(e) => { e.preventDefault(); setOver(false); read(e.dataTransfer.files[0]); }}
            className={cn('flex flex-col items-center gap-1.5 rounded-lg border border-dashed px-4 py-6 text-sm text-muted hover:bg-hover', over ? 'border-accent bg-accent-soft' : 'border-line-strong')}>
            <FileUp className="size-5" aria-hidden="true" />
            <span className="font-medium text-fg">Choose a file, or drop it here</span>
            <span className="text-xs">Sculptured saves designs like examples/teams/math-workbench.json</span>
          </button>
          <input ref={file} type="file" accept=".json,application/json" className="hidden" onChange={(e) => { read(e.target.files?.[0]); e.target.value = ''; }} />
          <Textarea rows={4} value={text} onChange={(e) => setText(e.target.value)} placeholder="Or paste the file's text here" aria-label="The team file's text" className="font-mono text-2xs" />
          {error && <ErrorState error={error} hint="Your team was not changed." />}
        </div>
        <div className="flex justify-end gap-2 border-t border-line px-4 py-2.5">
          <Button onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button variant="primary" onClick={() => take(text)} disabled={!text.trim()}>Import</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
