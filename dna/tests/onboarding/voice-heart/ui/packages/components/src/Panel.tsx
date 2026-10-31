import type { ReactNode } from 'react';
import s from './Panel.module.css';

/**
 * A bounded region of the canvas: a hairline plate with cut corners, a
 * heading, and a readout of what it counts and how fresh it is.
 */
export function Panel(props: { title: ReactNode; readout?: ReactNode; children: ReactNode }) {
  return (
    <section className={s.panel}>
      <div className={s.inner}>
        <div className={s.head}>
          <h3>{props.title}</h3>
          {props.readout && <span className="readout">{props.readout}</span>}
        </div>
        {props.children}
      </div>
    </section>
  );
}

export function PanelGrid(props: { children: ReactNode }) {
  return <div className={s.grid}>{props.children}</div>;
}

/** Typed properties of the selected thing, complete identifiers unbroken. */
export function Kv(props: { rows: { k: string; v: ReactNode }[] }) {
  return (
    <dl className={s.kv}>
      {props.rows.map((r) => (
        <div key={r.k} style={{ display: 'contents' }}>
          <dt>{r.k}</dt>
          <dd>{r.v}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Mono(props: { children: ReactNode }) {
  return <span className="mono">{props.children}</span>;
}
