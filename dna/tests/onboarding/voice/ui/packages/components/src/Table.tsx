import type { ReactNode } from 'react';
import s from './Table.module.css';

export type Column<T> = {
  key: string;
  header: ReactNode;
  /** Right-aligned with tabular numerals. */
  num?: boolean;
  /** Prose that may wrap; cells do not wrap by default, so ids stay whole. */
  wrap?: boolean;
  render: (row: T) => ReactNode;
};

/** Dense, precise lookup. Numbers are the content; they line up. */
export function Table<T>(props: {
  columns: Column<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  selectedKey?: string;
  onSelect?: (row: T) => void;
  /** Class per row, e.g. an afterglow for a fresh failure. */
  rowClass?: (row: T) => string | undefined;
  empty?: ReactNode;
}) {
  const selectable = !!props.onSelect;
  return (
    <div className={s.wrap}>
    <table className={`${s.table} ${selectable ? s.selectable : ''}`}>
      <thead>
        <tr>
          {props.columns.map((c) => (
            <th key={c.key} className={c.num ? s.num : undefined}>
              {c.header}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {props.rows.length === 0 && (
          <tr>
            <td colSpan={props.columns.length} className={s.empty}>
              {props.empty ?? 'Nothing here.'}
            </td>
          </tr>
        )}
        {props.rows.map((row) => {
          const k = props.rowKey(row);
          const cls = [props.rowClass?.(row), k === props.selectedKey ? s.selected : undefined]
            .filter(Boolean)
            .join(' ');
          return (
            <tr
              key={k}
              className={cls || undefined}
              onClick={props.onSelect ? () => props.onSelect!(row) : undefined}
              tabIndex={selectable ? 0 : undefined}
              onKeyDown={
                props.onSelect
                  ? (e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        props.onSelect!(row);
                      }
                    }
                  : undefined
              }
              aria-selected={selectable ? k === props.selectedKey : undefined}
            >
              {props.columns.map((c) => (
                <td key={c.key} className={c.num ? s.num : c.wrap ? s.wrapCell : undefined}>
                  {c.render(row)}
                </td>
              ))}
            </tr>
          );
        })}
      </tbody>
    </table>
    </div>
  );
}
