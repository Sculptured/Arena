import type { Mark } from './suite';

/** A status as glyph + word; colour only when it is abnormal. */
export function StatusMark({ mark, className = '' }: { mark: Mark; className?: string }) {
  return (
    <span className={`hmi-mark${mark.abnormal ? ' abnormal' : ''} ${className}`.trim()}>
      <span className="hmi-glyph" aria-hidden="true">{mark.glyph}</span>
      {mark.word}
    </span>
  );
}

/** A labelled figure of an overview: the label above, the value below. */
export function Figure({ label, value, abnormal = false, title }: { label: string; value: string; abnormal?: boolean; title?: string }) {
  return (
    <div className={`hmi-figure${abnormal ? ' abnormal' : ''}`} title={title}>
      <span className="label">{label}</span>
      <span className="hmi-value">{value}</span>
    </div>
  );
}
