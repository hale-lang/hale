/**
 * A state as a glyph and a word. Every state has a shape and text, so
 * color is never the only signal, and color appears only when something
 * is happening, waiting or wrong.
 */
export type Tone = 'quiet' | 'active' | 'wait' | 'fail' | 'ok';

const GLYPH: Record<Tone, string> = { quiet: '·', active: '●', wait: '◐', fail: '✕', ok: '✓' };
const CLASS: Record<Tone, string> = { quiet: 'muted', active: 'c-active', wait: 'c-wait', fail: 'c-fail', ok: 'c-ok' };

export function Status(props: { tone: Tone; label: string; title?: string }) {
  return (
    <span className={CLASS[props.tone]} title={props.title} style={{ whiteSpace: 'nowrap' }}>
      <span aria-hidden="true" style={{ display: 'inline-block', width: '1.1em' }}>
        {GLYPH[props.tone]}
      </span>
      {props.label}
    </span>
  );
}
