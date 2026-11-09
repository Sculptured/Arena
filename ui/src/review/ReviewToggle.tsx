import { Check } from 'lucide-react';
import { DropdownMenuItem } from '../components/ui/dropdown-menu';
import { modelName } from './present';
import { setReviewOn, useReviewOn, useServerReview } from './setting';

/** ⋯ → AI review: on or off in this browser; says why when arenad has it off. */
export default function ReviewToggle() {
  const on = useReviewOn();
  const server = useServerReview();
  const off = server && !server.enabled;
  const hint = off ? `Off in arenad: ${server.why ?? 'no model'}` : server ? `Reviews runs and comparisons with ${modelName(server.model)}` : '';
  return (
    <DropdownMenuItem disabled={!!off} title={hint} onSelect={(e) => { e.preventDefault(); setReviewOn(!on); }}>
      <span className="flex-1">{off ? 'AI review (off in arenad)' : 'AI review'}</span>
      {on && !off && <Check aria-label="on" />}
    </DropdownMenuItem>
  );
}
