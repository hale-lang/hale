import type { ReactNode } from 'react';
import s from './Shell.module.css';

/** The instrument's frame: context rail, canvas, inspector, evidence strip. */
export function Shell(props: {
  rail: ReactNode;
  inspector?: ReactNode;
  evidence: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className={s.shell}>
      <nav className={s.rail} aria-label="Context">
        {props.rail}
      </nav>
      <main className={s.canvas}>
        <div className={s.canvasInner}>{props.children}</div>
      </main>
      <aside className={s.inspector} aria-label="Inspector">
        {props.inspector ?? <span className="faint">Select something to inspect it.</span>}
      </aside>
      <footer className={s.evidence} aria-label="Evidence">
        {props.evidence}
      </footer>
    </div>
  );
}

export function RailHeader(props: { app: string; sub?: string }) {
  return (
    <div>
      <div className={s.app}>{props.app}</div>
      {props.sub && <div className={s.appSub}>{props.sub}</div>}
    </div>
  );
}

export function RailNav(props: { items: { href: string; label: string; active?: boolean }[] }) {
  return (
    <div className={s.nav}>
      {props.items.map((it) => (
        <a
          key={it.href}
          href={it.href}
          className={`${s.navLink} ${it.active ? s.navActive : ''}`}
          aria-current={it.active ? 'page' : undefined}
        >
          {it.label}
        </a>
      ))}
    </div>
  );
}

export function RailPrincipal(props: { children: ReactNode }) {
  return <div className={s.principal}>{props.children}</div>;
}

/** Source, revision and freshness of what the canvas shows. */
export function Evidence(props: { items: { label: string; value: ReactNode }[] }) {
  return (
    <>
      {props.items.map((it) => (
        <span key={it.label}>
          {it.label} <b>{it.value}</b>
        </span>
      ))}
    </>
  );
}
