import type { ReactNode } from 'react';

/** The designed states: each looks like something, and says what it is. */
function State(props: { tone: 'quiet' | 'wait' | 'fail'; children: ReactNode }) {
  const cls = props.tone === 'fail' ? 'c-fail' : props.tone === 'wait' ? 'c-wait' : 'faint';
  return (
    <div
      className={cls}
      role="status"
      style={{
        padding: 'var(--space-4)',
        border: 'var(--line-hair) dashed var(--color-rule-strong)',
        fontSize: 'var(--size-sm)',
      }}
    >
      {props.children}
    </div>
  );
}

export function Loading(props: { what?: string }) {
  return <State tone="wait">Loading {props.what ?? ''}…</State>;
}

export function Unavailable(props: { what?: string; detail?: string }) {
  return (
    <State tone="fail">
      {props.what ?? 'This'} is unavailable.{props.detail ? ` ${props.detail}` : ''}
    </State>
  );
}

export function Missing(props: { what: string }) {
  return <State tone="quiet">No such {props.what}.</State>;
}

export function Empty(props: { children: ReactNode }) {
  return <State tone="quiet">{props.children}</State>;
}

/** Data that could not be refreshed: shown, but marked. */
export function Stale(props: { since: Date }) {
  return (
    <span className="c-wait" title={`Last refreshed ${props.since.toISOString()}`}>
      stale
    </span>
  );
}
