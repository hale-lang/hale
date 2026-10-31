import { useState, type FormEvent, type ReactNode } from 'react';
import s from './Form.module.css';

/** A form: fields, an error, and a row of actions. Square, hairline, quiet. */
export function Form(props: { onSubmit: () => void | Promise<void>; children: ReactNode; error?: string | null; actions: ReactNode }) {
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void props.onSubmit();
  };
  return (
    <form className={s.form} onSubmit={submit}>
      {props.children}
      {props.error && (
        <div className={s.error} role="alert">
          {props.error}
        </div>
      )}
      <div className={s.row}>{props.actions}</div>
    </form>
  );
}

export function Field(props: { label: string; required?: boolean; help?: string; children: ReactNode }) {
  return (
    <label className={s.field}>
      <span className={s.label}>
        {props.label}
        {props.required && <span className={s.required}> *</span>}
      </span>
      {props.children}
      {props.help && <span className={s.help}>{props.help}</span>}
    </label>
  );
}

export function TextInput(props: { value: string; onChange: (v: string) => void; mono?: boolean; placeholder?: string; required?: boolean; pattern?: string; type?: 'text' | 'url' | 'datetime-local' }) {
  return (
    <input
      className={`${s.input} ${props.mono ? s.mono : ''}`}
      type={props.type ?? 'text'}
      value={props.value}
      onChange={(e) => props.onChange(e.target.value)}
      placeholder={props.placeholder}
      required={props.required}
      pattern={props.pattern}
    />
  );
}

export function NumberInput(props: { value: number | null; onChange: (v: number | null) => void; min?: number; max?: number; step?: number; placeholder?: string; required?: boolean }) {
  return (
    <input
      className={`${s.input} ${s.mono}`}
      type="number"
      value={props.value ?? ''}
      onChange={(e) => props.onChange(e.target.value === '' ? null : Number(e.target.value))}
      min={props.min}
      max={props.max}
      step={props.step}
      placeholder={props.placeholder}
      required={props.required}
    />
  );
}

export function Select(props: { value: string; onChange: (v: string) => void; options: { value: string; label?: string }[]; required?: boolean; empty?: string }) {
  return (
    <select className={s.input} value={props.value} onChange={(e) => props.onChange(e.target.value)} required={props.required}>
      {props.empty !== undefined && <option value="">{props.empty}</option>}
      {props.options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label ?? o.value}
        </option>
      ))}
    </select>
  );
}

export function Checkbox(props: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className={s.check}>
      <input type="checkbox" checked={props.checked} onChange={(e) => props.onChange(e.target.checked)} />
      <span>{props.label}</span>
    </label>
  );
}

/** A list of strings edited as one line, comma-separated. */
export function ListInput(props: { value: string[]; onChange: (v: string[]) => void; placeholder?: string; mono?: boolean }) {
  const [text, setText] = useState(props.value.join(', '));
  return (
    <input
      className={`${s.input} ${props.mono ? s.mono : ''}`}
      value={text}
      placeholder={props.placeholder}
      onChange={(e) => {
        setText(e.target.value);
        props.onChange(
          e.target.value
            .split(',')
            .map((x) => x.trim())
            .filter(Boolean),
        );
      }}
    />
  );
}

export function Button(props: { children: ReactNode; onClick?: () => void; type?: 'button' | 'submit'; tone?: 'default' | 'primary' | 'danger'; disabled?: boolean }) {
  const cls = props.tone === 'danger' ? s.danger : props.tone === 'primary' ? s.primary : undefined;
  return (
    <button type={props.type ?? 'button'} className={cls} onClick={props.onClick} disabled={props.disabled}>
      {props.children}
    </button>
  );
}

/**
 * A two-step action, inline: the first press arms it, the second does it.
 * No modal; the inspector is the only second surface.
 */
export function Confirm(props: { label: string; confirm: string; onConfirm: () => void | Promise<void>; tone?: 'default' | 'danger'; disabled?: boolean }) {
  const [armed, setArmed] = useState(false);
  if (!armed) {
    return (
      <Button onClick={() => setArmed(true)} tone={props.tone} disabled={props.disabled}>
        {props.label}
      </Button>
    );
  }
  return (
    <span className={s.row}>
      <Button
        tone={props.tone ?? 'danger'}
        onClick={async () => {
          await props.onConfirm();
          setArmed(false);
        }}
      >
        {props.confirm}
      </Button>
      <Button onClick={() => setArmed(false)}>cancel</Button>
    </span>
  );
}

/** A secret the API returns once. Shown once here too; never stored. */
export function Secret(props: { label: string; value: string; note?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={s.secret} role="status">
      <span className={s.label}>
        <span className="c-wait">{props.label}</span> · shown once
      </span>
      <code className={s.secretValue}>{props.value}</code>
      <span className={s.row}>
        <Button
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(props.value);
              setCopied(true);
            } catch {
              setCopied(false);
            }
          }}
        >
          {copied ? 'copied' : 'copy'}
        </Button>
        {props.note && <span className={s.help}>{props.note}</span>}
      </span>
    </div>
  );
}
