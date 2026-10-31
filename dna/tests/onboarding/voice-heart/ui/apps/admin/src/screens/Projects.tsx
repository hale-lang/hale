import { useEffect, useState } from 'react';
import { Panel, Table, Status, Loading, Unavailable, Missing, Kv, Mono, Form, Field, TextInput, NumberInput, Select, ListInput, Button, Confirm, Secret, micros, when, ago } from '@hale/components';
import { useGet, type Project, type Schemas } from '../api/client';
import { useWrite } from '../api/mutate';
import type { ScreenProps } from '../App';
import { go } from '../route';
import { readout } from '../readout';
import { toneOf } from '../tone';
import { LimitsEditor, limitsForWrite, type Limit } from '../LimitsEditor';
import { describeLimit } from './Accounts';

type Key = Schemas['Key'];

/** Projects: who is asking, what they may spend, and their keys. */
export function Projects({ route, setInspector }: ScreenProps) {
  const selected = route.parts[1];
  const q = useGet('/admin/v1/projects', { params: { query: { include_archived: true } } });
  const project = q.data?.data.find((p) => p.id === selected);
  const creating = selected === 'new';

  useEffect(() => {
    setInspector(creating ? <CreateProject /> : project ? <ProjectInspector project={project} /> : null);
  }, [creating, project, setInspector]);

  if (q.isPending) return <Loading what="projects" />;
  if (q.isError) return <Unavailable what="Projects" detail={q.error.message} />;
  if (selected && !creating && !project) return <Missing what={`project ${selected}`} />;

  return (
    <>
      <h1>Projects</h1>
      <Panel title="Projects" readout={readout(q, `${q.data.data.length} projects`)}>
        <Table<Project>
          rows={q.data.data}
          rowKey={(p) => p.id}
          selectedKey={selected}
          onSelect={(p) => go('projects', p.id)}
          columns={[
            { key: 'id', header: 'Project', render: (p) => <Mono>{p.id}</Mono> },
            { key: 'name', header: 'Name', render: (p) => p.name },
            { key: 'status', header: 'State', render: (p) => <Status tone={toneOf(p.status)} label={p.status} /> },
            { key: 'keys', header: 'Keys', num: true, render: (p) => p.key_count ?? 0 },
            { key: 'accounts', header: 'Accounts', render: (p) => (p.accounts ? p.accounts.join(', ') : <span className="muted">any</span>) },
            { key: 'spent', header: 'Spent', num: true, render: (p) => (p.budget ? micros(p.budget.spent_micros, p.budget.currency) : <span className="faint">no budget</span>) },
            { key: 'budget', header: 'Allowance', num: true, render: (p) => (p.budget ? `${micros(p.budget.allowance_micros, p.budget.currency)} / ${p.budget.period}` : '') },
          ]}
          empty="No projects. Create one, then keys under it."
        />
        <p style={{ marginBottom: 0 }}>
          <Button onClick={() => go('projects', 'new')} tone="primary">new project</Button>
        </p>
      </Panel>
      {project && <Keys project={project} />}
    </>
  );
}

function CreateProject() {
  const w = useWrite();
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [dataClasses, setDataClasses] = useState<string[]>(['internal']);
  const [accounts, setAccounts] = useState<string[]>([]);
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>New project</h2>
      <Form
        error={w.error}
        onSubmit={async () => {
          const r = await w.write('post', '/admin/v1/projects', { body: { id, name, data_classes: dataClasses, accounts: accounts.length ? accounts : null } });
          if (r) go('projects', id);
        }}
        actions={
          <>
            <Button type="submit" tone="primary" disabled={w.busy}>create</Button>
            <Button onClick={() => go('projects')}>cancel</Button>
          </>
        }
      >
        <Field label="id" required help="lowercase, digits and dashes; never reused">
          <TextInput mono value={id} onChange={setId} required pattern="^[a-z0-9][a-z0-9-]{0,62}$" placeholder="work" />
        </Field>
        <Field label="name" required>
          <TextInput value={name} onChange={setName} required />
        </Field>
        <Field label="data classes" help="what this project may send; comma-separated">
          <ListInput value={dataClasses} onChange={setDataClasses} />
        </Field>
        <Field label="accounts" help="what it may spend; empty for any. Client work names the work account here.">
          <ListInput mono value={accounts} onChange={setAccounts} placeholder="claude-work" />
        </Field>
      </Form>
    </>
  );
}

function ProjectInspector({ project: p }: { project: Project }) {
  const w = useWrite();
  const [mode, setMode] = useState<'view' | 'budget' | 'limits'>('view');
  const [allowance, setAllowance] = useState<number | null>(p.budget ? p.budget.allowance_micros / 1e6 : null);
  const [period, setPeriod] = useState<'day' | 'week' | 'month' | 'none'>(p.budget?.period ?? 'month');
  const [limits, setLimits] = useState<Limit[]>(p.limits ?? []);
  useEffect(() => {
    setMode('view');
    setAllowance(p.budget ? p.budget.allowance_micros / 1e6 : null);
    setPeriod(p.budget?.period ?? 'month');
    setLimits(p.limits ?? []);
  }, [p]);
  const b = p.budget;
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{p.id}</Mono>
      </h2>
      <Kv
        rows={[
          { k: 'state', v: <Status tone={toneOf(p.status)} label={p.status} /> },
          { k: 'name', v: p.name },
          { k: 'owners', v: p.owners.join(', ') },
          { k: 'data classes', v: p.data_classes.join(', ') },
          { k: 'models', v: p.models ? p.models.join(', ') : 'the whole catalog' },
          { k: 'accounts', v: p.accounts ? p.accounts.join(', ') : 'any' },
          { k: 'position', v: p.position ?? '–' },
          { k: 'created', v: when(p.created_at) },
        ]}
      />
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Budget</h3>
      {mode === 'budget' ? (
        <Form
          error={w.error}
          onSubmit={async () => {
            const r = await w.write('put', '/admin/v1/projects/{project}/budget', { params: { path: { project: p.id } }, body: { allowance_micros: Math.round((allowance ?? 0) * 1e6), period, currency: 'USD' } });
            if (r) setMode('view');
          }}
          actions={
            <>
              <Button type="submit" tone="primary" disabled={w.busy}>set budget</Button>
              <Button onClick={() => setMode('view')}>cancel</Button>
            </>
          }
        >
          <Field label="allowance (USD)" required help="a hard cap on spend per period">
            <NumberInput value={allowance} onChange={setAllowance} min={0} step={0.01} required />
          </Field>
          <Field label="period" required>
            <Select value={period} onChange={(v) => setPeriod(v as typeof period)} options={[{ value: 'day' }, { value: 'week' }, { value: 'month' }, { value: 'none', label: 'none (lifetime cap)' }]} />
          </Field>
        </Form>
      ) : (
        <>
          {b ? (
            <Kv
              rows={[
                { k: 'allowance', v: `${micros(b.allowance_micros, b.currency)} per ${b.period}` },
                { k: 'spent', v: <span className={b.allowance_micros && b.spent_micros / b.allowance_micros >= 0.8 ? 'c-wait' : undefined}>{micros(b.spent_micros, b.currency)}</span> },
                { k: 'reserved', v: micros(b.reserved_micros, b.currency) },
                { k: 'remaining', v: <span className={b.remaining_micros <= 0 ? 'c-fail' : undefined}>{micros(b.remaining_micros, b.currency)}</span> },
                { k: 'window', v: `${when(b.window_start)} → ${b.window_end ? when(b.window_end) : 'no end'}` },
              ]}
            />
          ) : (
            <span className="faint">no budget: unlimited spend</span>
          )}
          <p><Button onClick={() => setMode('budget')}>{b ? 'change budget' : 'set budget'}</Button></p>
        </>
      )}
      <h3 style={{ margin: 'var(--space-4) 0 var(--space-2)' }}>Limits</h3>
      {mode === 'limits' ? (
        <Form
          error={w.error}
          onSubmit={async () => {
            const r = await w.write('put', '/admin/v1/projects/{project}/limits', { params: { path: { project: p.id } }, body: { limits: limitsForWrite(limits) } });
            if (r) setMode('view');
          }}
          actions={
            <>
              <Button type="submit" tone="primary" disabled={w.busy}>save limits</Button>
              <Button onClick={() => setMode('view')}>cancel</Button>
            </>
          }
        >
          <LimitsEditor scope="project" allowModel value={limits} onChange={setLimits} />
        </Form>
      ) : (
        <>
          {(p.limits ?? []).length === 0 ? <span className="faint">none</span> : (p.limits ?? []).map((l, i) => <div key={i}>{describeLimit(l)}</div>)}
          <p><Button onClick={() => setMode('limits')}>edit limits</Button></p>
        </>
      )}
      {w.pending && <p className="c-wait">Proposed, pending review: {w.pending}</p>}
      {p.status === 'active' && (
        <p style={{ marginTop: 'var(--space-4)' }}>
          <Confirm label="archive project" confirm="archive: its keys are revoked" onConfirm={async () => { await w.write('delete', '/admin/v1/projects/{project}', { params: { path: { project: p.id } } }); }} />
        </p>
      )}
      {w.error && mode === 'view' && <p className="c-fail">{w.error}</p>}
    </>
  );
}

function Keys({ project }: { project: Project }) {
  const q = useGet('/admin/v1/projects/{project}/keys', { params: { path: { project: project.id } } });
  const w = useWrite();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState('');
  const [accounts, setAccounts] = useState<string[]>([]);
  const [secret, setSecret] = useState<{ id: string; secret: string } | null>(null);
  return (
    <Panel title={`Keys under ${project.id}`} readout={readout(q, q.data ? `${q.data.data.length} keys` : null)}>
      {q.isPending ? (
        <Loading what="keys" />
      ) : q.isError ? (
        <Unavailable what="Keys" detail={q.error.message} />
      ) : (
        <Table<Key>
          rows={q.data.data}
          rowKey={(k) => k.id}
          rowClass={(k) => (k.id === secret?.id ? 'afterglow' : undefined)}
          columns={[
            { key: 'id', header: 'Key', render: (k) => <Mono>{k.id}</Mono> },
            { key: 'name', header: 'Name', render: (k) => k.name },
            { key: 'prefix', header: 'Prefix', render: (k) => <Mono>{k.prefix ?? '–'}…</Mono> },
            { key: 'accounts', header: 'Accounts', render: (k) => (k.accounts ? k.accounts.join(', ') : <span className="muted">the project's</span>) },
            { key: 'status', header: 'State', render: (k) => <Status tone={toneOf(k.status)} label={k.status} /> },
            { key: 'used', header: 'Last used', render: (k) => <span className="muted">{k.last_used_at ? `${ago(k.last_used_at)} ago` : 'never'}</span> },
            {
              key: 'x',
              header: '',
              render: (k) =>
                k.status === 'active' ? (
                  <Confirm label="revoke" confirm="revoke now" onConfirm={async () => { await w.write('delete', '/admin/v1/projects/{project}/keys/{key}', { params: { path: { project: project.id, key: k.id } } }); }} />
                ) : null,
            },
          ]}
          empty="No keys. One per client is the intended use."
        />
      )}
      {secret && <div style={{ marginTop: 'var(--space-3)' }}><Secret label={`secret of ${secret.id}`} value={secret.secret} note="voice stores only a digest; this is the only time it is shown" /></div>}
      {creating ? (
        <div style={{ marginTop: 'var(--space-3)', maxWidth: 480 }}>
          <Form
            error={w.error}
            onSubmit={async () => {
              const r = await w.write('post', '/admin/v1/projects/{project}/keys', { params: { path: { project: project.id } }, body: { name, accounts: accounts.length ? accounts : null } });
              if (r && 'secret' in r) {
                setSecret({ id: r.id, secret: r.secret });
                setCreating(false);
                setName('');
                setAccounts([]);
              }
            }}
            actions={
              <>
                <Button type="submit" tone="primary" disabled={w.busy}>create key</Button>
                <Button onClick={() => setCreating(false)}>cancel</Button>
              </>
            }
          >
            <Field label="name" required help="name the client this key is for; usage is broken down by key">
              <TextInput value={name} onChange={setName} required placeholder="ci" />
            </Field>
            <Field label="accounts" help="restrict to these accounts, within the project's; empty for the project's">
              <ListInput mono value={accounts} onChange={setAccounts} placeholder="claude-work" />
            </Field>
          </Form>
        </div>
      ) : (
        <p style={{ marginBottom: 0 }}>
          <Button onClick={() => { setSecret(null); setCreating(true); }} tone="primary">new key</Button>
        </p>
      )}
      {w.error && !creating && <p className="c-fail">{w.error}</p>}
    </Panel>
  );
}
