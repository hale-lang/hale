import { useEffect, useState } from 'react';
import { Panel, Table, Status, Loading, Unavailable, Missing, Kv, Mono, Form, Field, TextInput, NumberInput, Select, ListInput, Checkbox, Button, Confirm, Secret, SchemaForm, ago, type JsonObject, type JsonSchema } from '@hale/components';
import { useGet, type Node, type SeatView, type Schemas } from '../api/client';
import { useWrite } from '../api/mutate';
import type { ScreenProps } from '../App';
import { href, go } from '../route';
import { readout } from '../readout';
import { toneOf } from '../tone';

type Seat = Schemas['Seat'];

/** Nodes, and one node's seats with assigned beside reported. */
export function Nodes({ route, setInspector }: ScreenProps) {
  const nodeId = route.parts[1];
  const seatId = route.parts[2];
  const q = useGet('/admin/v1/nodes');
  const node = q.data?.data.find((n) => n.id === nodeId);
  const creating = nodeId === 'new';
  const seat = node?.seats.find((s) => s.id === seatId);
  const newSeat = seatId === 'new';

  useEffect(() => {
    if (creating) setInspector(<CreateNode />);
    else if (node && newSeat) setInspector(<SeatForm node={node} />);
    else if (node && seat) setInspector(<SeatInspector node={node} seat={seat} />);
    else if (node) setInspector(<NodeInspector node={node} />);
    else setInspector(null);
  }, [creating, node, seat, newSeat, setInspector]);

  if (q.isPending) return <Loading what="nodes" />;
  if (q.isError) return <Unavailable what="Nodes" detail={q.error.message} />;
  if (nodeId && !creating && !node) return <Missing what={`node ${nodeId}`} />;

  return (
    <>
      <h1>Nodes</h1>
      <Panel title="Nodes" readout={readout(q, `${q.data.data.length} nodes`)}>
        <Table<Node>
          rows={q.data.data}
          rowKey={(n) => n.id}
          selectedKey={nodeId}
          onSelect={(n) => go('nodes', n.id)}
          columns={[
            { key: 'id', header: 'Node', render: (n) => <Mono>{n.id}</Mono> },
            { key: 'name', header: 'Name', render: (n) => n.name ?? '' },
            { key: 'status', header: 'State', render: (n) => <Status tone={toneOf(n.status)} label={n.status} /> },
            { key: 'seats', header: 'Seats', num: true, render: (n) => n.seats.length },
            { key: 'cap', header: 'In flight / cap', num: true, render: (n) => `${n.in_flight ?? 0} / ${n.effective_max_in_flight ?? '–'}` },
            { key: 'assign', header: 'Assignment', num: true, render: (n) => <AssignmentCell node={n} /> },
            { key: 'hb', header: 'Heartbeat', render: (n) => <Heartbeat at={n.last_heartbeat_at} /> },
          ]}
          empty="No nodes. Create one to get its enrollment token."
        />
        <p style={{ marginBottom: 0 }}>
          <Button onClick={() => go('nodes', 'new')} tone="primary">new node</Button>
        </p>
      </Panel>
      {node && <Seats node={node} selected={seatId} fresh={readout(q)} />}
    </>
  );
}

/** Quiet while fresh; amber once older than the interval a node is expected to keep. */
function Heartbeat({ at }: { at: string | null | undefined }) {
  if (!at) return <span className="faint">–</span>;
  const stale = Date.now() - Date.parse(at) > 60_000;
  return <span className={stale ? 'c-wait' : 'muted'} title={stale ? 'no heartbeat for over a minute' : undefined}>{ago(at)} ago</span>;
}

/** Intent beside ground truth: the latest assignment and the one applied. */
function AssignmentCell({ node }: { node: Node }) {
  const version = node.assignment_version ?? 0;
  const behind = node.applied_version != null && node.applied_version < version;
  return (
    <span className={behind ? 'c-wait' : undefined} title={behind ? 'the node has not yet applied the latest assignment' : undefined}>
      v{version}
      {node.applied_version == null ? <span className="faint"> (not applied)</span> : behind ? ` (applied v${node.applied_version})` : ''}
    </span>
  );
}

function Seats({ node, selected, fresh }: { node: Node; selected?: string; fresh: string }) {
  return (
    <Panel title={`Seats on ${node.id}`} readout={`${node.seats.length} seats · ${fresh}`}>
      <Table<SeatView>
        rows={node.seats}
        rowKey={(s) => s.id}
        selectedKey={selected}
        onSelect={(s) => go('nodes', node.id, s.id)}
        columns={[
          { key: 'id', header: 'Seat', render: (s) => <Mono>{s.id}</Mono> },
          { key: 'engine', header: 'Engine', render: (s) => s.engine },
          { key: 'account', header: 'Account', render: (s) => <Mono>{s.account}</Mono> },
          { key: 'serves', header: 'Serves', render: (s) => s.serves.map((m) => (m.as ? `${m.model} as ${m.as}` : m.model)).join(', ') },
          { key: 'assigned', header: 'Assigned', render: (s) => (s.enabled === false ? 'disabled' : `enabled · ${s.concurrency ?? 1} at once`) },
          { key: 'reported', header: 'Reported', render: (s) => <Status tone={toneOf(s.status)} label={s.status} title={s.reason ?? undefined} /> },
          { key: 'reason', header: 'Reason', wrap: true, render: (s) => s.reason ?? (s.problems.length ? <span className="c-wait">{s.problems.join('; ')}</span> : <span className="faint">–</span>) },
          { key: 'flight', header: 'In flight', num: true, render: (s) => s.in_flight },
        ]}
        empty="No seats assigned to this node."
      />
      <p style={{ marginBottom: 0 }}>
        <Button onClick={() => go('nodes', node.id, 'new')} tone="primary" disabled={!node.capabilities}>
          {node.capabilities ? 'add seat' : 'add seat (the node has not declared what it offers)'}
        </Button>
      </p>
    </Panel>
  );
}

function CreateNode() {
  const w = useWrite();
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [cap, setCap] = useState<number | null>(null);
  const [token, setToken] = useState<{ id: string; token: string } | null>(null);
  if (token) {
    return (
      <>
        <h2 style={{ marginBottom: 'var(--space-3)' }}>
          <Mono>{token.id}</Mono> created
        </h2>
        <Secret label="enrollment token" value={token.token} note="start the node with it; voice stores only a digest" />
        <p className="muted" style={{ fontSize: 'var(--size-sm)' }}>
          <code>voice-node --id {token.id} --token … --api https://your-api --capabilities capabilities.json</code>
        </p>
        <p><Button onClick={() => go('nodes', token.id)}>done</Button></p>
      </>
    );
  }
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>New node</h2>
      <Form
        error={w.error}
        onSubmit={async () => {
          const r = await w.write('post', '/admin/v1/nodes', { body: { id, name: name || undefined, max_in_flight: cap } });
          if (r && 'enrollment_token' in r) setToken({ id: r.id, token: r.enrollment_token });
          else if (r) go('nodes');
        }}
        actions={
          <>
            <Button type="submit" tone="primary" disabled={w.busy}>create and issue token</Button>
            <Button onClick={() => go('nodes')}>cancel</Button>
          </>
        }
      >
        <Field label="id" required help="the machine's name in voice; never reused">
          <TextInput mono value={id} onChange={setId} required pattern="^[a-z0-9][a-z0-9-]{0,62}$" placeholder="laptop" />
        </Field>
        <Field label="name">
          <TextInput value={name} onChange={setName} />
        </Field>
        <Field label="in-flight cap" help="the coordinator's cap; empty leaves the node's declared cap in charge">
          <NumberInput value={cap} onChange={setCap} min={1} />
        </Field>
      </Form>
    </>
  );
}

function NodeInspector({ node }: { node: Node }) {
  const w = useWrite();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(node.name ?? '');
  const [cap, setCap] = useState<number | null>(node.max_in_flight ?? null);
  const [token, setToken] = useState<string | null>(null);
  useEffect(() => {
    setEditing(false);
    setName(node.name ?? '');
    setCap(node.max_in_flight ?? null);
    setToken(null);
  }, [node]);
  const caps = node.capabilities;
  const local = node.local;
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{node.id}</Mono>
      </h2>
      {editing ? (
        <Form
          error={w.error}
          onSubmit={async () => {
            const r = await w.write('patch', '/admin/v1/nodes/{node}', { params: { path: { node: node.id } }, body: { name: name || undefined, max_in_flight: cap } });
            if (r) setEditing(false);
          }}
          actions={
            <>
              <Button type="submit" tone="primary" disabled={w.busy}>save</Button>
              <Button onClick={() => setEditing(false)}>cancel</Button>
            </>
          }
        >
          <Field label="name"><TextInput value={name} onChange={setName} /></Field>
          <Field label="in-flight cap" help={caps ? `at most the declared ${caps.max_in_flight}; empty for none` : 'empty for none'}>
            <NumberInput value={cap} onChange={setCap} min={1} max={caps?.max_in_flight} />
          </Field>
        </Form>
      ) : (
        <Kv
          rows={[
            { k: 'state', v: <Status tone={toneOf(node.status)} label={node.status} /> },
            { k: 'name', v: node.name ?? '–' },
            { k: 'version', v: node.version ?? '–' },
            { k: 'connected', v: node.connected_at ? `${ago(node.connected_at)} ago` : 'never' },
            { k: 'heartbeat', v: <Heartbeat at={node.last_heartbeat_at} /> },
            { k: 'cap', v: <CapExplained node={node} /> },
            { k: 'assignment', v: <AssignmentCell node={node} /> },
          ]}
        />
      )}
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Offers</h3>
      {caps ? (
        <Kv rows={caps.engines.map((e) => ({ k: e.id, v: <>{e.kind}{e.models ? <span className="muted"> · {e.models.join(', ')}</span> : null}</> }))} />
      ) : (
        <span className="faint">not declared yet: the node has not registered</span>
      )}
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Local controls</h3>
      {local ? (
        <Kv
          rows={[
            { k: 'paused', v: local.paused ? <Status tone="wait" label="paused" /> : 'no' },
            { k: 'paused seats', v: local.paused_seats.length ? local.paused_seats.join(', ') : 'none' },
            { k: 'local cap', v: local.max_in_flight ?? 'none' },
            { k: 'reason', v: local.reason ?? '–' },
          ]}
        />
      ) : (
        <span className="faint">not reported yet</span>
      )}
      {token && <div style={{ marginTop: 'var(--space-3)' }}><Secret label="new enrollment token" value={token} note="the old token stops working now; restart the node with this one" /></div>}
      {w.pending && <p className="c-wait">Proposed, pending review: {w.pending}</p>}
      {w.error && !editing && <p className="c-fail">{w.error}</p>}
      {node.status !== 'revoked' && (
        <p style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap', marginTop: 'var(--space-4)' }}>
          {!editing && <Button onClick={() => setEditing(true)}>edit</Button>}
          <Confirm label="rotate token" confirm="rotate: the old one stops now" tone="default" onConfirm={async () => { const r = await w.write('post', '/admin/v1/nodes/{node}/token', { params: { path: { node: node.id } } }); if (r && 'enrollment_token' in r) setToken(r.enrollment_token); }} />
          <Confirm label="revoke node" confirm="revoke: its seats stop serving" onConfirm={async () => { await w.write('delete', '/admin/v1/nodes/{node}', { params: { path: { node: node.id } } }); }} />
        </p>
      )}
      <p className="faint" style={{ fontSize: 'var(--size-xs)', marginTop: 'var(--space-4)' }}>
        <a href={href('nodes')}>all nodes</a>
      </p>
    </>
  );
}

/** The effective cap and what bounds it: declared, coordinator, local. */
function CapExplained({ node }: { node: Node }) {
  const declared = node.capabilities?.max_in_flight;
  const parts = [
    declared != null ? `declared ${declared}` : null,
    node.max_in_flight != null ? `coordinator ${node.max_in_flight}` : null,
    node.local?.max_in_flight != null ? `local ${node.local.max_in_flight}` : null,
  ].filter(Boolean);
  return (
    <>
      <b>{node.effective_max_in_flight ?? '–'}</b>
      {parts.length ? <span className="muted"> · {parts.join(' · ')}</span> : null}
    </>
  );
}

function SeatInspector({ node, seat }: { node: Node; seat: SeatView }) {
  const w = useWrite();
  const [editing, setEditing] = useState(false);
  useEffect(() => setEditing(false), [seat]);
  if (editing) return <SeatForm node={node} seat={seat} onDone={() => setEditing(false)} />;
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{seat.ref}</Mono>
      </h2>
      <Kv
        rows={[
          { k: 'reported', v: <Status tone={toneOf(seat.status)} label={seat.status} /> },
          { k: 'reason', v: seat.reason ?? '–' },
          { k: 'problems', v: seat.problems.length ? <span className="c-wait">{seat.problems.join('; ')}</span> : 'none' },
          { k: 'engine', v: seat.engine },
          { k: 'account', v: <Mono>{seat.account}</Mono> },
          { k: 'serves', v: seat.serves.map((m) => <div key={m.model}>{m.model}{m.as ? <span className="muted"> as {m.as}</span> : null}</div>) },
          { k: 'data classes', v: (seat.data_classes ?? ['internal']).join(', ') },
          { k: 'assigned', v: seat.enabled === false ? 'disabled' : `enabled · ${seat.concurrency ?? 1} at once` },
          { k: 'in flight', v: seat.in_flight },
          { k: 'config', v: <Mono>{JSON.stringify(seat.config ?? {})}</Mono> },
        ]}
      />
      {w.pending && <p className="c-wait">Proposed, pending review: {w.pending}</p>}
      {w.error && <p className="c-fail">{w.error}</p>}
      <p style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap', marginTop: 'var(--space-4)' }}>
        <Button onClick={() => setEditing(true)}>edit</Button>
        <Button onClick={() => w.write('patch', '/admin/v1/nodes/{node}/seats/{seat}', { params: { path: { node: node.id, seat: seat.id } }, body: { enabled: seat.enabled === false } })}>
          {seat.enabled === false ? 'enable' : 'disable'}
        </Button>
        <Confirm label="remove seat" confirm="remove: in-flight requests finish" onConfirm={async () => { const r = await w.write('delete', '/admin/v1/nodes/{node}/seats/{seat}', { params: { path: { node: node.id, seat: seat.id } } }); if (r) go('nodes', node.id); }} />
      </p>
    </>
  );
}

/** A seat, created or changed: the engine's own config schema drives its form. */
function SeatForm({ node, seat, onDone }: { node: Node; seat?: SeatView; onDone?: () => void }) {
  const w = useWrite();
  const accounts = useGet('/admin/v1/accounts');
  const catalog = useGet('/admin/v1/models');
  const engines = node.capabilities?.engines ?? [];
  const [id, setId] = useState(seat?.id ?? '');
  const [engine, setEngine] = useState(seat?.engine ?? engines[0]?.id ?? '');
  const [account, setAccount] = useState(seat?.account ?? '');
  const [serves, setServes] = useState<Seat['serves']>(seat?.serves ?? []);
  const [dataClasses, setDataClasses] = useState<string[]>(seat?.data_classes ?? ['internal']);
  const [concurrency, setConcurrency] = useState<number | null>(seat?.concurrency ?? 1);
  const [enabled, setEnabled] = useState(seat?.enabled ?? true);
  const [config, setConfig] = useState<JsonObject>((seat?.config as JsonObject | undefined) ?? {});
  const engineDef = engines.find((e) => e.id === engine);
  const schema = (engineDef?.config_schema ?? { type: 'object' }) as JsonSchema;
  const cancel = () => (onDone ? onDone() : go('nodes', node.id));
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>{seat ? <><Mono>{seat.ref}</Mono></> : `New seat on ${node.id}`}</h2>
      <Form
        error={w.error}
        onSubmit={async () => {
          const body: Seat = { id, engine, account, serves, data_classes: dataClasses, concurrency: concurrency ?? 1, enabled, config };
          const r = seat
            ? await w.write('patch', '/admin/v1/nodes/{node}/seats/{seat}', { params: { path: { node: node.id, seat: seat.id } }, body: { engine, account, serves, data_classes: dataClasses, concurrency: concurrency ?? 1, enabled, config } })
            : await w.write('post', '/admin/v1/nodes/{node}/seats', { params: { path: { node: node.id } }, body });
          if (r) {
            if (onDone) onDone();
            else go('nodes', node.id, id);
          }
        }}
        actions={
          <>
            <Button type="submit" tone="primary" disabled={w.busy || !engine || !account || serves.length === 0}>{seat ? 'save' : 'add seat'}</Button>
            <Button onClick={cancel}>cancel</Button>
          </>
        }
      >
        {!seat && (
          <Field label="id" required help="unique on this node">
            <TextInput mono value={id} onChange={setId} required pattern="^[a-z0-9][a-z0-9-]{0,62}$" placeholder="static" />
          </Field>
        )}
        <Field label="engine" required help="one the node offers">
          <Select value={engine} onChange={(v) => { setEngine(v); setConfig({}); }} options={engines.map((e) => ({ value: e.id, label: `${e.id} (${e.kind})` }))} required empty="" />
        </Field>
        <Field label="account" required help="what this seat spends">
          <Select value={account} onChange={setAccount} options={(accounts.data?.data ?? []).filter((a) => a.status === 'active').map((a) => ({ value: a.id, label: `${a.id} · ${a.billing}` }))} required empty="" />
        </Field>
        <Field label="serves" required help="catalog models this seat serves; `as` is the engine's own name when it differs">
          <ServesEditor value={serves} onChange={setServes} models={(catalog.data?.data ?? []).map((m) => m.id)} engineModels={engineDef?.models ?? null} />
        </Field>
        <Field label="data classes" help="what this seat may carry">
          <ListInput value={dataClasses} onChange={setDataClasses} />
        </Field>
        <Field label="concurrency" help="requests this seat serves at once">
          <NumberInput value={concurrency} onChange={setConcurrency} min={1} />
        </Field>
        <Checkbox checked={enabled} onChange={setEnabled} label="enabled" />
        {schema.properties && (
          <>
            <h3 style={{ margin: 'var(--space-2) 0 0' }}>{engine} config</h3>
            <SchemaForm schema={schema} value={config} onChange={setConfig} />
          </>
        )}
      </Form>
    </>
  );
}

function ServesEditor({ value, onChange, models, engineModels }: { value: Seat['serves']; onChange: (v: Seat['serves']) => void; models: string[]; engineModels: string[] | null }) {
  const set = (i: number, patch: Partial<Seat['serves'][number]>) => onChange(value.map((m, j) => (j === i ? { ...m, ...patch } : m)));
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-1)' }}>
      {value.map((m, i) => (
        <span key={i} style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center' }}>
          <Select value={m.model} onChange={(v) => set(i, { model: v })} options={models.map((id) => ({ value: id }))} empty="model" required />
          <span className="faint">as</span>
          {engineModels ? (
            <Select value={m.as ?? ''} onChange={(v) => set(i, { as: v || null })} options={engineModels.map((id) => ({ value: id }))} empty="same" />
          ) : (
            <TextInput mono value={m.as ?? ''} onChange={(v) => set(i, { as: v || null })} placeholder="same" />
          )}
          <Button onClick={() => onChange(value.filter((_, j) => j !== i))}>remove</Button>
        </span>
      ))}
      <span><Button onClick={() => onChange([...value, { model: models[0] ?? '', as: null }])}>add model</Button></span>
    </div>
  );
}
