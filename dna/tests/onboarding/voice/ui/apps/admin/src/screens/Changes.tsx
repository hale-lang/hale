import { useEffect } from 'react';
import { Panel, Table, Loading, Unavailable, Kv, Mono, when, ago } from '@hale/components';
import { useGet, type Change } from '../api/client';
import type { ScreenProps } from '../App';
import { go } from '../route';
import { readout } from '../readout';

/** The audit log: every applied admin write, before and after. */
export function Changes({ route, setInspector }: ScreenProps) {
  const selected = route.parts[1];
  const q = useGet('/admin/v1/changes', { params: { query: { limit: 100 } } });
  const change = q.data?.data.find((c) => c.id === selected);
  useEffect(() => {
    setInspector(change ? <ChangeInspector change={change} /> : null);
  }, [change, setInspector]);

  if (q.isPending) return <Loading what="changes" />;
  if (q.isError) return <Unavailable what="Changes" detail={q.error.message} />;
  return (
    <>
      <h1>Changes</h1>
      <Panel title="Applied" readout={readout(q, `${q.data.data.length} changes`)}>
        <Table<Change>
          rows={q.data.data}
          rowKey={(c) => c.id}
          selectedKey={selected}
          onSelect={(c) => go('changes', c.id)}
          columns={[
            { key: 'when', header: 'When', render: (c) => <span className="muted" title={when(c.at)}>{ago(c.at)} ago</span> },
            { key: 'action', header: 'Action', render: (c) => c.action },
            { key: 'target', header: 'Target', render: (c) => <Mono>{c.target}</Mono> },
            { key: 'who', header: 'By', render: (c) => c.principal ?? <span className="faint">no principal</span> },
          ]}
          empty="Nothing has changed yet."
        />
      </Panel>
    </>
  );
}

function ChangeInspector({ change: c }: { change: Change }) {
  const pre = (v: unknown) => (
    <pre className="mono" style={{ margin: 0, whiteSpace: 'pre-wrap', fontSize: 'var(--size-sm)' }}>
      {v == null ? '–' : JSON.stringify(v, null, 2)}
    </pre>
  );
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{c.id}</Mono>
      </h2>
      <Kv
        rows={[
          { k: 'at', v: when(c.at) },
          { k: 'action', v: c.action },
          { k: 'target', v: <Mono>{c.target}</Mono> },
          { k: 'by', v: c.principal ?? 'no principal' },
        ]}
      />
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Before</h3>
      {pre(c.before)}
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>After</h3>
      {pre(c.after)}
    </>
  );
}
