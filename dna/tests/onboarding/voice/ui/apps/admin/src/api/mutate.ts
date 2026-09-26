import { useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import type { MaybeOptionalInit, FetchResponse } from 'openapi-fetch';
import type { PathsWithMethod, RequiredKeysOf } from 'openapi-typescript-helpers';
import { api } from './client';
import type { paths } from './schema';

type Method = 'post' | 'put' | 'patch' | 'delete';
type InitParam<Init> = RequiredKeysOf<Init> extends never ? [(Init & { [key: string]: unknown })?] : [Init & { [key: string]: unknown }];
/** The success body of a write, from the contract. */
type Data<P extends keyof paths, M extends Method> = Extract<FetchResponse<paths[P][M] & Record<string | number, any>, MaybeOptionalInit<paths[P], M>, 'application/json'>, { error?: never }>['data'];

/**
 * A typed write. Every rule is the api's; the UI only reports what it
 * answered. After any write, everything shown is refetched: the api is the
 * only source of what changed. `pending` is a 202: proposed, not applied.
 */
export function useWrite() {
  const qc = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<string | null>(null);

  async function write<M extends Method, P extends PathsWithMethod<paths, M>>(method: M, path: P, ...init: InitParam<MaybeOptionalInit<paths[P], M>>): Promise<Data<P, M> | null> {
    setBusy(true);
    setError(null);
    setPending(null);
    try {
      const fn = { post: api.POST, put: api.PUT, patch: api.PATCH, delete: api.DELETE }[method] as (p: P, ...i: typeof init) => Promise<{ data?: unknown; error?: unknown; response: Response }>;
      const r = await fn(path, ...init);
      if (r.error !== undefined) {
        const e = r.error as { error?: { message?: string; code?: string } };
        setError(e?.error?.code ? `${e.error.code}: ${e.error.message ?? ''}` : (e?.error?.message ?? `${r.response.status}`));
        return null;
      }
      if (r.response.status === 202) {
        const pc = r.data as { id?: string; review?: string | null } | undefined;
        setPending(pc?.review ?? pc?.id ?? 'pending review');
      }
      await qc.invalidateQueries();
      return (r.data ?? ({} as Data<P, M>)) as Data<P, M>;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }

  return { write, error, busy, pending, clear: () => setError(null) };
}
