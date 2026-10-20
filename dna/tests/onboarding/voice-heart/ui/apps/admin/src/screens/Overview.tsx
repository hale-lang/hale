import { Panel, PanelGrid, Gauge, Table, Status, Loading, Unavailable, Mono, micros, count, until, ago } from '@hale/components';
import { toneOf } from '../tone';
import { useGet, type Account, type Node, type SeatView, type UsageRecord } from '../api/client';
import type { ScreenProps } from '../App';
import { href } from '../route';
import { readout } from '../readout';

/** Headroom, health, spend, recent requests: the glance. */
export function Overview(_: ScreenProps) {
  return (
    <>
      <h1>Overview</h1>
      <Accounts />
      <Fleet />
      <PanelGrid>
        <Spend />
        <Recent />
      </PanelGrid>
    </>
  );
}

function Accounts() {
  const q = useGet('/admin/v1/accounts');
  if (q.isPending) return <Loading what="accounts" />;
  if (q.isError) return <Unavailable what="Accounts" detail={q.error.message} />;
  const accounts = q.data.data.filter((a) => a.status === 'active');
  return (
    <Panel title="Headroom" readout={readout(q, `${accounts.length} accounts`)}>
      <PanelGrid>
        {accounts.map((a) => (
          <AccountGauges key={a.id} account={a} />
        ))}
      </PanelGrid>
    </Panel>
  );
}

function AccountGauges({ account }: { account: Account }) {
  const windows = account.windows ?? [];
  return (
    <div>
      <div style={{ marginBottom: 'var(--space-2)' }}>
        <Mono>{account.id}</Mono> <span className="muted">{account.billing}</span>{' '}
        {account.exhausted_until && <Status tone="fail" label={`exhausted, resets ${until(account.exhausted_until)}`} />}
      </div>
      {windows.length === 0 ? (
        <span className="faint">no windows reported</span>
      ) : (
        <div style={{ display: 'flex', gap: 'var(--space-4)', flexWrap: 'wrap' }}>
          {windows.map((w) => (
            <Gauge key={w.name} label={w.name.replace('_', ' ')} used={w.used_percent / 100} detail={w.resets_at ? `resets ${until(w.resets_at)}` : undefined} size={72} />
          ))}
        </div>
      )}
    </div>
  );
}

type SeatRow = SeatView & { nodeStatus: Node['status'] };

function Fleet() {
  const q = useGet('/admin/v1/nodes');
  if (q.isPending) return <Loading what="nodes" />;
  if (q.isError) return <Unavailable what="Nodes" detail={q.error.message} />;
  const rows: SeatRow[] = q.data.data.flatMap((n) => n.seats.map((s) => ({ ...s, nodeStatus: n.status })));
  const nodes = q.data.data;
  return (
    <Panel title="Fleet" readout={readout(q, `${nodes.length} nodes`, `${rows.length} seats`, `${nodes.reduce((a, n) => a + (n.in_flight ?? 0), 0)} in flight`)}>
      <Table<SeatRow>
        rows={rows}
        rowKey={(r) => r.ref}
        columns={[
          { key: 'seat', header: 'Seat', render: (r) => <a href={href('nodes', r.node)}><Mono>{r.ref}</Mono></a> },
          { key: 'engine', header: 'Engine', render: (r) => r.engine },
          { key: 'account', header: 'Account', render: (r) => <Mono>{r.account}</Mono> },
          { key: 'serves', header: 'Serves', render: (r) => r.serves.map((m) => m.model).join(', ') },
          { key: 'status', header: 'State', render: (r) => <Status tone={toneOf(r.status)} label={r.status} title={r.reason ?? undefined} /> },
          { key: 'flight', header: 'In flight', num: true, render: (r) => `${r.in_flight} / ${r.concurrency}` },
          { key: 'problems', header: 'Problems', wrap: true, render: (r) => (r.problems.length ? <span className="c-wait">{r.problems.join('; ')}</span> : <span className="faint">–</span>) },
        ]}
        empty="No seats. Create a node, then assign it a seat."
      />
    </Panel>
  );
}

function Spend() {
  const q = useGet('/admin/v1/projects');
  if (q.isPending) return <Loading what="projects" />;
  if (q.isError) return <Unavailable what="Projects" detail={q.error.message} />;
  return (
    <Panel title="Spend this period" readout={readout(q, `${q.data.data.length} projects`)}>
      <Table
        rows={q.data.data}
        rowKey={(p) => p.id}
        columns={[
          { key: 'p', header: 'Project', render: (p) => <Mono>{p.id}</Mono> },
          { key: 'spent', header: 'Spent', num: true, render: (p) => (p.budget ? micros(p.budget.spent_micros, p.budget.currency) : <span className="faint">no budget</span>) },
          { key: 'of', header: 'Allowance', num: true, render: (p) => (p.budget ? micros(p.budget.allowance_micros, p.budget.currency) : '') },
          {
            key: 'pct',
            header: 'Used',
            num: true,
            render: (p) => {
              if (!p.budget) return '';
              const used = p.budget.allowance_micros ? p.budget.spent_micros / p.budget.allowance_micros : 0;
              const cls = used >= 1 ? 'c-fail' : used >= 0.8 ? 'c-wait' : undefined;
              return <span className={cls}>{Math.round(used * 100)}%</span>;
            },
          },
        ]}
      />
    </Panel>
  );
}

function Recent() {
  const q = useGet('/admin/v1/usage', { params: { query: { limit: 20 } } });
  if (q.isPending) return <Loading what="usage" />;
  if (q.isError) return <Unavailable what="Usage" detail={q.error.message} />;
  const fresh = (r: UsageRecord) => Date.now() - Date.parse(r.created_at) < 60_000;
  return (
    <Panel title="Recent requests" readout={readout(q, `${q.data.data.length}`)}>
      <Table<UsageRecord>
        rows={q.data.data}
        rowKey={(r) => r.id}
        rowClass={(r) => (r.status !== 'served' && fresh(r) ? 'afterglow' : undefined)}
        columns={[
          { key: 'when', header: 'When', render: (r) => <span className="muted">{ago(r.created_at)}</span> },
          { key: 'project', header: 'Project', render: (r) => <Mono>{r.project}</Mono> },
          { key: 'model', header: 'Model', render: (r) => r.served?.model ?? r.requested_model },
          { key: 'seat', header: 'Seat', render: (r) => <Mono>{r.served?.seat ?? '–'}</Mono> },
          { key: 'status', header: 'Status', render: (r) => <Status tone={toneOf(r.status)} label={r.status} /> },
          { key: 'tokens', header: 'In / out', num: true, render: (r) => `${count(r.input_tokens)} / ${count(r.output_tokens)}` },
          { key: 'price', header: 'Price', num: true, render: (r) => micros(r.price_micros, r.currency) },
        ]}
        empty="No requests yet."
      />
    </Panel>
  );
}
