import { Field, TextInput, NumberInput, Select, Checkbox } from './Form';

/**
 * A form rendered from a JSON Schema object: strings, numbers, integers,
 * booleans and enums, with required marks, defaults and descriptions.
 * Enough for an engine's seat config; nested objects show as JSON text.
 */
export type JsonSchema = {
  type?: string | string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
  enum?: (string | number)[];
  description?: string;
  default?: unknown;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  title?: string;
};

export type JsonObject = Record<string, unknown>;

export function SchemaForm(props: { schema: JsonSchema; value: JsonObject; onChange: (v: JsonObject) => void }) {
  const props_ = props.schema.properties ?? {};
  const required = new Set(props.schema.required ?? []);
  const set = (k: string, v: unknown) => props.onChange({ ...props.value, [k]: v });
  return (
    <>
      {Object.entries(props_).map(([key, sub]) => {
        const type = Array.isArray(sub.type) ? sub.type.find((t) => t !== 'null') : sub.type;
        const label = sub.title ?? key;
        const help = [sub.description, sub.default !== undefined ? `default ${JSON.stringify(sub.default)}` : null].filter(Boolean).join(' · ') || undefined;
        const current = props.value[key];
        if (sub.enum) {
          return (
            <Field key={key} label={label} required={required.has(key)} help={help}>
              <Select value={current == null ? '' : String(current)} onChange={(v) => set(key, v === '' ? undefined : coerce(v, type))} options={sub.enum.map((e) => ({ value: String(e) }))} empty="" required={required.has(key)} />
            </Field>
          );
        }
        if (type === 'boolean') {
          return (
            <Field key={key} label={label} help={help}>
              <Checkbox checked={Boolean(current ?? sub.default ?? false)} onChange={(v) => set(key, v)} label={current == null && sub.default !== undefined ? `(default: ${String(sub.default)})` : ''} />
            </Field>
          );
        }
        if (type === 'integer' || type === 'number') {
          return (
            <Field key={key} label={label} required={required.has(key)} help={help}>
              <NumberInput value={typeof current === 'number' ? current : null} onChange={(v) => set(key, v ?? undefined)} min={sub.minimum} max={sub.maximum} step={type === 'integer' ? 1 : undefined} placeholder={sub.default !== undefined ? String(sub.default) : undefined} required={required.has(key)} />
            </Field>
          );
        }
        if (type === 'object' || type === 'array') {
          return (
            <Field key={key} label={`${label} (JSON)`} required={required.has(key)} help={help}>
              <TextInput mono value={current === undefined ? '' : JSON.stringify(current)} onChange={(v) => set(key, parseJson(v))} required={required.has(key)} />
            </Field>
          );
        }
        return (
          <Field key={key} label={label} required={required.has(key)} help={help}>
            <TextInput value={typeof current === 'string' ? current : ''} onChange={(v) => set(key, v === '' ? undefined : v)} required={required.has(key)} placeholder={sub.default !== undefined ? String(sub.default) : undefined} />
          </Field>
        );
      })}
    </>
  );
}

function coerce(v: string, type: string | undefined): unknown {
  if (type === 'integer' || type === 'number') return Number(v);
  return v;
}

function parseJson(v: string): unknown {
  if (v.trim() === '') return undefined;
  try {
    return JSON.parse(v);
  } catch {
    return v;
  }
}
