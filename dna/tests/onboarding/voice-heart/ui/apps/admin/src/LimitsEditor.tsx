import { Button, NumberInput, TextInput, Table } from '@hale/components';
import type { Schemas } from './api/client';

export type Limit = Schemas['Limit'];

/**
 * Voice's rate limits, edited as rows. Null counters are unlimited. The
 * scope is fixed by where the editor sits (a project's limits, a key's,
 * an account's); a project's editor may also carry model-scope rows.
 */
export function LimitsEditor(props: { scope: Limit['scope']; allowModel?: boolean; value: Limit[]; onChange: (v: Limit[]) => void }) {
  const rows = props.value;
  const set = (i: number, patch: Partial<Limit>) => props.onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const remove = (i: number) => props.onChange(rows.filter((_, j) => j !== i));
  const add = (scope: Limit['scope']) => props.onChange([...rows, { scope, window_seconds: 60, requests: null, input_tokens: null, output_tokens: null, concurrent: null }]);
  const num = (k: 'window_seconds' | 'requests' | 'input_tokens' | 'output_tokens' | 'concurrent', i: number, r: Limit, min = 0) => (
    <NumberInput value={r[k] ?? null} onChange={(v) => set(i, { [k]: k === 'window_seconds' ? (v ?? 60) : v })} min={min} placeholder={k === 'window_seconds' ? '60' : '∞'} />
  );
  type Row = { r: Limit; i: number };
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
      <Table<Row>
        rows={rows.map((r, i) => ({ r, i }))}
        rowKey={(x) => String(x.i)}
        columns={[
          { key: 'scope', header: 'Scope', render: ({ r, i }) => (r.scope === 'model' ? <TextInput mono value={r.model ?? ''} onChange={(v) => set(i, { model: v })} placeholder="model id" required /> : r.scope) },
          { key: 'w', header: 'Window (s)', num: true, render: ({ r, i }) => num('window_seconds', i, r, 1) },
          { key: 'req', header: 'Requests', num: true, render: ({ r, i }) => num('requests', i, r) },
          { key: 'in', header: 'In tokens', num: true, render: ({ r, i }) => num('input_tokens', i, r) },
          { key: 'out', header: 'Out tokens', num: true, render: ({ r, i }) => num('output_tokens', i, r) },
          { key: 'c', header: 'Concurrent', num: true, render: ({ r, i }) => num('concurrent', i, r) },
          { key: 'x', header: '', render: ({ i }) => <Button onClick={() => remove(i)}>remove</Button> },
        ]}
        empty="No limits: unlimited."
      />
      <span style={{ display: 'flex', gap: 'var(--space-2)' }}>
        <Button onClick={() => add(props.scope)}>add {props.scope} limit</Button>
        {props.allowModel && <Button onClick={() => add('model')}>add model limit</Button>}
      </span>
    </div>
  );
}

/** What the API accepts: the read-only fields dropped. */
export function limitsForWrite(limits: Limit[]): Limit[] {
  return limits.map(({ remaining: _r, resets_at: _t, ...rest }) => rest);
}
