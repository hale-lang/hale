import createClient, { type MaybeOptionalInit } from 'openapi-fetch';
import type { PathsWithMethod, RequiredKeysOf } from 'openapi-typescript-helpers';
import { useQuery } from '@tanstack/react-query';
import type { paths, components } from './schema';

/** The one way the UI reaches voice: spec/openapi.yaml, typed. */
export const api = createClient<paths>({ baseUrl: '' });

export type Schemas = components['schemas'];
export type Node = Schemas['Node'];
export type SeatView = Schemas['SeatView'];
export type Account = Schemas['Account'];
export type Project = Schemas['Project'];
export type UsageRecord = Schemas['UsageRecord'];
export type UsageSummary = Schemas['UsageSummary'];
export type Change = Schemas['Change'];
export type CatalogModel = Schemas['CatalogModel'];

type GetPaths = PathsWithMethod<paths, 'get'>;

// openapi-fetch's own (unexported) shape for the optional init argument.
type InitParam<Init> = RequiredKeysOf<Init> extends never ? [(Init & { [key: string]: unknown })?] : [Init & { [key: string]: unknown }];

/**
 * A typed GET as a query. The response type follows from the path, so a
 * contract change is a type error here rather than a surprise on screen.
 */
export function useGet<P extends GetPaths>(path: P, ...init: InitParam<MaybeOptionalInit<paths[P], 'get'>>) {
  return useQuery({
    queryKey: [path, init[0] ?? null],
    queryFn: async () => {
      const r = await api.GET(path, ...init);
      if (r.error !== undefined || r.data === undefined) {
        throw new Error(describe(r.error, r.response.status));
      }
      return r.data;
    },
  });
}

function describe(error: unknown, status: number): string {
  const e = error as { error?: { message?: string; code?: string } } | undefined;
  const msg = e?.error?.message ?? `${status}`;
  return e?.error?.code ? `${e.error.code}: ${msg}` : msg;
}
