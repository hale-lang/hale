import { useEffect, useState } from 'react';
import { Panel, Table, Status, Loading, Unavailable, Missing, Kv, Mono, Form, Field, TextInput, NumberInput, Select, Button, Confirm, micros, when } from '@hale/components';
import { useGet, type CatalogModel } from '../api/client';
import { useWrite } from '../api/mutate';
import type { ScreenProps } from '../App';
import { go } from '../route';
import { readout } from '../readout';

/** The catalog: what callers may name, with its price. */
export function Catalog({ route, setInspector }: ScreenProps) {
  const selected = route.parts[1];
  const q = useGet('/admin/v1/models');
  const model = q.data?.data.find((m) => m.id === selected);
  const creating = selected === 'new';

  useEffect(() => {
    setInspector(creating ? <ModelForm /> : model ? <ModelInspector model={model} /> : null);
  }, [creating, model, setInspector]);

  if (q.isPending) return <Loading what="the catalog" />;
  if (q.isError) return <Unavailable what="The catalog" detail={q.error.message} />;
  if (selected && !creating && !model) return <Missing what={`model ${selected}`} />;

  return (
    <>
      <h1>Catalog</h1>
      <Panel title="Models" readout={readout(q, `${q.data.data.length} models`)}>
        <Table<CatalogModel>
          rows={q.data.data}
          rowKey={(m) => m.id}
          selectedKey={selected}
          onSelect={(m) => go('catalog', m.id)}
          columns={[
            { key: 'id', header: 'Model', render: (m) => <Mono>{m.id}</Mono> },
            { key: 'kind', header: 'Kind', render: (m) => `${m.kind ?? 'local'} · ${m.owned_by ?? 'local'}` },
            { key: 'ctx', header: 'Context', num: true, render: (m) => m.context_length?.toLocaleString() ?? '–' },
            { key: 'out', header: 'Max out', num: true, render: (m) => m.max_output_tokens?.toLocaleString() ?? '–' },
            { key: 'price', header: 'Price / Mtok in · out', num: true, render: (m) => (m.price ? `${micros(m.price.input_per_mtok_micros, m.price.currency)} · ${micros(m.price.output_per_mtok_micros, m.price.currency)}` : <span className="faint">free</span>) },
            { key: 'seats', header: 'Seats', num: true, render: (m) => m.seats.length },
            { key: 'avail', header: 'Available', render: (m) => (m.available ? <Status tone="quiet" label="yes" /> : <Status tone="wait" label="no seat" />) },
          ]}
          empty="No models. Add one, then a seat can serve it."
        />
        <p style={{ marginBottom: 0 }}>
          <Button onClick={() => go('catalog', 'new')} tone="primary">add model</Button>
        </p>
      </Panel>
    </>
  );
}

function ModelForm({ model }: { model?: CatalogModel }) {
  const w = useWrite();
  const [id, setId] = useState(model?.id ?? '');
  const [ownedBy, setOwnedBy] = useState(model?.owned_by ?? 'local');
  const [kind, setKind] = useState<'local' | 'hosted'>(model?.kind ?? 'local');
  const [ctx, setCtx] = useState<number | null>(model?.context_length ?? null);
  const [maxOut, setMaxOut] = useState<number | null>(model?.max_output_tokens ?? null);
  const [priceIn, setPriceIn] = useState<number | null>(model?.price ? model.price.input_per_mtok_micros / 1e6 : null);
  const [priceOut, setPriceOut] = useState<number | null>(model?.price ? model.price.output_per_mtok_micros / 1e6 : null);
  const price = priceIn != null && priceOut != null ? { input_per_mtok_micros: Math.round(priceIn * 1e6), output_per_mtok_micros: Math.round(priceOut * 1e6), currency: 'USD' } : null;
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>{model ? <Mono>{model.id}</Mono> : 'Add model'}</h2>
      <Form
        error={w.error}
        onSubmit={async () => {
          const body = { owned_by: ownedBy, kind, context_length: ctx ?? undefined, max_output_tokens: maxOut ?? undefined, price };
          const r = model
            ? await w.write('patch', '/admin/v1/models/{model}', { params: { path: { model: model.id } }, body })
            : await w.write('post', '/admin/v1/models', { body: { id, ...body } });
          if (r) go('catalog', model?.id ?? id);
        }}
        actions={
          <>
            <Button type="submit" tone="primary" disabled={w.busy}>{model ? 'save' : 'add'}</Button>
            <Button onClick={() => go('catalog', ...(model ? [model.id] : []))}>cancel</Button>
          </>
        }
      >
        {!model && (
          <Field label="id" required help="what callers name; never reused for a different model">
            <TextInput mono value={id} onChange={setId} required placeholder="claude-sonnet-5" />
          </Field>
        )}
        <Field label="kind" required>
          <Select value={kind} onChange={(v) => setKind(v as typeof kind)} options={[{ value: 'local' }, { value: 'hosted' }]} />
        </Field>
        <Field label="owned by" help="the provider, or local">
          <TextInput value={ownedBy} onChange={setOwnedBy} />
        </Field>
        <Field label="context length">
          <NumberInput value={ctx} onChange={setCtx} min={1} />
        </Field>
        <Field label="max output tokens" help="the default and maximum; reserved when a request sets none">
          <NumberInput value={maxOut} onChange={setMaxOut} min={1} />
        </Field>
        <Field label="price per million input tokens (USD)" help="empty on both for free">
          <NumberInput value={priceIn} onChange={setPriceIn} min={0} step={0.01} />
        </Field>
        <Field label="price per million output tokens (USD)">
          <NumberInput value={priceOut} onChange={setPriceOut} min={0} step={0.01} />
        </Field>
      </Form>
    </>
  );
}

function ModelInspector({ model: m }: { model: CatalogModel }) {
  const w = useWrite();
  const [editing, setEditing] = useState(false);
  useEffect(() => setEditing(false), [m]);
  if (editing) return <ModelForm model={m} />;
  return (
    <>
      <h2 style={{ marginBottom: 'var(--space-3)' }}>
        <Mono>{m.id}</Mono>
      </h2>
      <Kv
        rows={[
          { k: 'kind', v: `${m.kind ?? 'local'} · ${m.owned_by ?? 'local'}` },
          { k: 'context', v: m.context_length?.toLocaleString() ?? '–' },
          { k: 'max output', v: m.max_output_tokens?.toLocaleString() ?? '–' },
          { k: 'price', v: m.price ? `${micros(m.price.input_per_mtok_micros, m.price.currency)} in · ${micros(m.price.output_per_mtok_micros, m.price.currency)} out, per Mtok` : 'free' },
          { k: 'available', v: m.available ? 'yes' : <span className="c-wait">no seat serves it</span> },
          { k: 'seats', v: m.seats.length ? m.seats.map((s) => <div key={s}><Mono>{s}</Mono></div>) : 'none' },
          { k: 'created', v: when(m.created_at) },
        ]}
      />
      <p style={{ display: 'flex', gap: 'var(--space-2)', marginTop: 'var(--space-4)' }}>
        <Button onClick={() => setEditing(true)}>edit</Button>
        <Confirm label="remove" confirm="remove from catalog" onConfirm={async () => { const r = await w.write('delete', '/admin/v1/models/{model}', { params: { path: { model: m.id } } }); if (r) go('catalog'); }} />
      </p>
      {w.pending && <p className="c-wait">Proposed, pending review: {w.pending}</p>}
      {w.error && <p className="c-fail">{w.error}</p>}
    </>
  );
}
