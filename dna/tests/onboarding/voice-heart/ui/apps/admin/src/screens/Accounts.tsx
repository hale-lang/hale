import { useEffect, useState } from 'react';
import { Panel, Table, Status, Loading, Unavailable, Missing, Kv, Mono, Gauge, Form, Field, TextInput, Select, Button, Confirm, until, when } from '@hale/components';
import { useGet, type Account } from '../api/client';
import { useWrite } from '../api/mutate';
import type { ScreenProps } from '../App';
import { go } from '../route';
import { readout } from '../readout';
import { toneOf } from '../tone';
import { LimitsEditor, limitsForWrite, type Limit } from '../LimitsEditor';

/** Accounts: what pays, what has limits, and how much is left. */
export function Accounts({ route, setInspector }: ScreenProps) {
  const selected = route.parts[1];
  const q = useGet('/admin/v1/accounts', { params: { query: { include_archived: true } } });
  const account = q.data?.data.find((a) => a.id === selected);
  const creating = selected === 'new';

  useEffect(() => {
    setInspector(creating ? <CreateAccount /> : account ? <AccountInspector account={account} /> : null);
  }, [creating, account, setInspector]);

  if (q.isPending) return <Loading what="accounts" />;
  if (q.isError) return <Unavailable what="Accounts" detail={q.error.message} />;
  if (selected && !creating && !account) return <Missing what={`account ${selected}`} />;

  return (
    <>
      <h1>Accounts</h1>
      <Panel title="Accounts" readout={readout(q, `${q.data.data.length} accounts`)}>
        <Table<Account>
          rows={q.data.data}
          rowKey={(a) => a.id}
          selectedKey={selected}
          onSelect={(a) => go('accounts', a.id)}
          columns={[
            { key: 'id', header: 'Account', render: (a) => <Mono>{a.id}</Mono> },
            { key: 'name', header: 'Name', render: (a) => a.name },
            { key: 'provider', header: 'Provider', render: (a) => a.provider ?? <span className="faint">local</span> },
            { key: 'billing', header: 'Billing', render: (a) => a.billing },
            { key: 'status', header: 'State', render: (a) => (a.exhausted_until ? <Status tone="fail" label="exhausted" title={`until ${when(a.exhausted_until)}`} /> : <Status tone={toneOf(a.status)} label={a.status} />) },
            { key: 'windows', header: 'Windows', render: (a) => (a.windows.length ? a.windows.map((w) => `${w.name} ${Math.round(w.used_percent)}%`).join(' · ') : <span className="faint">none reported</span>) },
            { key: 'seats', header: 'Seats', num: true, render: (a) => a.seats.length },
          ]}
          empty="No accounts. Create one, then seats can spend it."
        />
        <p style={{ marginBottom: 0 }}>
          <Button onClick={() => go('accounts', 'new')} tone="primary">new account</Button>
        </p>
      </Panel>
    </>
  );
}

function CreateAccount() {
  const w = useWrite();
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [provider, setProvider] = useState('');
  const [billing, setBilling] = useState<'subscription' | 'metered' | 'none'>('none');
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>New account</h2>
      <Form
        error={w.error}
        onSubmit={async () => {
          const r = await w.write('post', '/admin/v1/accounts', { body: { id, name, provider: provider || null, billing } });
          if (r) go('accounts', id);
        }}
        actions={
          <>
            <Button type="submit" tone="primary" disabled={w.busy}>create</Button>
            <Button onClick={() => go('accounts')}>cancel</Button>
          </>
        }
      >
        <Field label="id" required help="lowercase, digits and dashes; never reused">
          <TextInput mono value={id} onChange={setId} required pattern="^[a-z0-9][a-z0-9-]{0,62}$" placeholder="claude-work" />
        </Field>
        <Field label="name" required>
          <TextInput value={name} onChange={setName} required />
        </Field>
        <Field label="provider" help="who the account is with; empty for a local engine">
          <TextInput value={provider} onChange={setProvider} placeholder="anthropic" />
        </Field>
        <Field label="billing" required>
          <Select value={billing} onChange={(v) => setBilling(v as typeof billing)} options={[{ value: 'none', label: 'none (local engine)' }, { value: 'subscription' }, { value: 'metered' }]} />
        </Field>
      </Form>
    </>
  );
}

function AccountInspector({ account: a }: { account: Account }) {
  const w = useWrite();
  const [limits, setLimits] = useState<Limit[]>(a.limits ?? []);
  const [editing, setEditing] = useState(false);
  useEffect(() => setLimits(a.limits ?? []), [a]);
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{a.id}</Mono>
      </h2>
      <Kv
        rows={[
          { k: 'state', v: <Status tone={toneOf(a.status)} label={a.status} /> },
          { k: 'name', v: a.name },
          { k: 'provider', v: a.provider ?? 'local' },
          { k: 'billing', v: a.billing },
          { k: 'exhausted', v: a.exhausted_until ? <span className="c-fail">until {when(a.exhausted_until)} ({until(a.exhausted_until)})</span> : 'no' },
          { k: 'seats', v: a.seats.length ? a.seats.map((s) => <div key={s}><Mono>{s}</Mono></div>) : 'none' },
          { k: 'created', v: when(a.created_at) },
        ]}
      />
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Windows</h3>
      {a.windows.length === 0 ? (
        <span className="faint">none reported yet</span>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
          {a.windows.map((win) => (
            <Gauge key={win.name} label={win.name.replace('_', ' ')} used={win.used_percent / 100} detail={`${win.resets_at ? `resets ${until(win.resets_at)}` : ''} · seen by ${win.seat ?? '–'}`} size={56} />
          ))}
        </div>
      )}
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Limits</h3>
      {editing ? (
        <Form
          error={w.error}
          onSubmit={async () => {
            const r = await w.write('patch', '/admin/v1/accounts/{account}', { params: { path: { account: a.id } }, body: { limits: limitsForWrite(limits) } });
            if (r) setEditing(false);
          }}
          actions={
            <>
              <Button type="submit" tone="primary" disabled={w.busy}>save limits</Button>
              <Button onClick={() => { setLimits(a.limits ?? []); setEditing(false); }}>cancel</Button>
            </>
          }
        >
          <LimitsEditor scope="account" value={limits.map((l) => ({ ...l, account: a.id }))} onChange={setLimits} />
        </Form>
      ) : (
        <>
          {(a.limits ?? []).length === 0 ? <span className="faint">none: only what the provider allows</span> : (a.limits ?? []).map((l, i) => <div key={i}>{describeLimit(l)}</div>)}
          <p><Button onClick={() => setEditing(true)}>edit limits</Button></p>
        </>
      )}
      {w.pending && <p className="c-wait">Proposed, pending review: {w.pending}</p>}
      {a.status === 'active' && (
        <p style={{ marginTop: 'var(--space-4)' }}>
          <Confirm label="archive account" confirm="archive: its seats stop serving" onConfirm={async () => { await w.write('delete', '/admin/v1/accounts/{account}', { params: { path: { account: a.id } } }); }} />
        </p>
      )}
      {w.error && !editing && <p className="c-fail">{w.error}</p>}
    </>
  );
}

export function describeLimit(l: Limit): string {
  const parts = [l.requests != null ? `${l.requests} req` : null, l.input_tokens != null ? `${l.input_tokens} in` : null, l.output_tokens != null ? `${l.output_tokens} out` : null, l.concurrent != null ? `${l.concurrent} at once` : null].filter(Boolean);
  return `${l.scope}${l.model ? ` ${l.model}` : ''}: ${parts.join(', ') || 'unlimited'} per ${l.window_seconds}s`;
}
