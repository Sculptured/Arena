import { Check } from 'lucide-react';
import { DropdownMenuItem, DropdownMenuLabel } from '../components/ui/dropdown-menu';
import { modelName } from './present';
import { setReviewOn, useReviewOn, useServerReview } from './setting';

/** ⋯ → AI review: on or off in this browser; says why when arenad has it off. */
export default function ReviewToggle() {
  const on = useReviewOn();
  const server = useServerReview();
  const off = server && !server.enabled;
  const hint = off ? `Off in arenad: ${server.why ?? 'no model'}` : server ? `Reviews runs and comparisons with ${modelName(server.model)}` : '';
  return (
    <>
      <DropdownMenuLabel>AI review</DropdownMenuLabel>
      <DropdownMenuItem disabled={!!off} title={hint} onSelect={(e) => { e.preventDefault(); setReviewOn(!on); }}>
        <Check className={on && !off ? 'opacity-100' : 'opacity-0'} />
        <span className="flex-1">{off ? 'AI review is off in arenad' : on ? 'AI review on' : 'AI review off'}</span>
        {!off && <span className="text-2xs text-faint">{on ? 'click to turn off' : 'click to turn on'}</span>}
      </DropdownMenuItem>
    </>
  );
}
