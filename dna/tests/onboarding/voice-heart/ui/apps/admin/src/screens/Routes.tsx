import { Panel, Table, Status, Loading, Unavailable, Empty, Mono, until, when } from '@hale/components';
import { useGet } from '../api/client';
import type { ScreenProps } from '../App';
import { readout } from '../readout';

/** Per model, where the next request would go and why. */
export function Routes(_: ScreenProps) {
  const q = useGet('/admin/v1/routes');
  if (q.isPending) return <Loading what="the route table" />;
  if (q.isError) return <Unavailable what="The route table" detail={q.error.message} />;
  const t = q.data;
  return (
    <>
      <h1>Route table</h1>
      <p className="muted">
        Version {t.version}, published {when(t.at)}. A snapshot as this api instance holds it; the node accepts or refuses each request regardless.
      </p>
      {t.models.length === 0 && <Empty>No models route anywhere: no seat is up, or the catalog is empty.</Empty>}
      {t.models.map((m) => (
        <Panel key={m.model} title={<Mono>{m.model}</Mono>} readout={readout(q, `${m.seats.length} seats`, `${m.exhausted.length} exhausted`, `v${t.version}`)}>
          <Table
            rows={m.seats}
            rowKey={(s) => s.seat}
            columns={[
              { key: 'rank', header: '#', num: true, render: (s) => m.seats.indexOf(s) + 1 },
              { key: 'seat', header: 'Seat', render: (s) => <Mono>{s.seat}</Mono> },
              { key: 'account', header: 'Account', render: (s) => <Mono>{s.account}</Mono> },
              { key: 'as', header: 'Engine model', render: (s) => s.engine_model ?? <span className="faint">same</span> },
              { key: 'free', header: 'Free', num: true, render: (s) => (s.free === 0 ? <span className="c-wait">0</span> : s.free) },
              { key: 'rank_v', header: 'Rank', num: true, render: (s) => s.rank.toFixed(3) },
              { key: 'dc', header: 'Data classes', render: (s) => s.data_classes.join(', ') },
            ]}
            empty="No seat can serve this model now."
          />
          {m.exhausted.length > 0 && (
            <div style={{ marginTop: 'var(--space-3)' }}>
              {m.exhausted.map((e) => (
                <div key={e.account}>
                  <Status tone="fail" label={`${e.account} exhausted`} /> <span className="muted">{e.until ? `resets ${until(e.until)} (${when(e.until)})` : 'until it reports headroom'}</span>
                </div>
              ))}
            </div>
          )}
        </Panel>
      ))}
    </>
  );
}
