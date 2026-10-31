import { ago } from '@hale/components';

/** A panel's readout: what it counts, then how fresh: `2 SEATS · 3S`. */
export function readout(q: { dataUpdatedAt: number; isFetching: boolean }, ...parts: (string | number | null | undefined)[]): string {
  const fresh = q.isFetching ? '…' : q.dataUpdatedAt ? ago(new Date(q.dataUpdatedAt).toISOString()) : '–';
  return [...parts.filter((p) => p !== null && p !== undefined && p !== ''), fresh].join(' · ');
}
