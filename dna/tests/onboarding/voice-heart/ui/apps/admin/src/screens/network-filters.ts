import { requestContext, type Fleet } from './network-model.ts';

export const FACETS = [
  ['host', 'Host'], ['seat', 'Seat'], ['engine', 'Backend / engine'], ['account', 'Account'],
  ['model', 'Model'], ['project', 'Project'], ['repository', 'Repository'], ['api', 'API instance'], ['hostState', 'Host state'], ['seatState', 'Seat state'],
  ['provider', 'Provider'], ['billing', 'Billing'], ['accountState', 'Account state'], ['dataClass', 'Data class'],
  ['activity', 'Seat activity'], ['enabled', 'Seat enabled'], ['problems', 'Seat problems'],
] as const;
export type Facet = typeof FACETS[number][0];
export type Filters = Partial<Record<Facet, string>> & { search?: string };
export function filterOptions(fleet: Fleet): Record<Facet, string[]> {
  const seats = fleet.nodes.flatMap(n => n.seats);
  const unique = (values: (string | undefined | null)[]) => [...new Set(values.filter((v): v is string => !!v))].sort();
  return {
    host: unique(fleet.nodes.map(n => n.id)), seat: unique(seats.map(s => s.ref)), engine: unique(seats.map(s => s.engine)),
    account: unique(fleet.accounts.map(a => a.id)), model: unique([...seats.flatMap(s => s.serves.map(m => m.model)), ...fleet.usage.map(r => r.requested_model)]),
    repository: unique(fleet.usage.map(r => requestContext(r, fleet.contexts).repository)), api: unique(fleet.usage.map(r => requestContext(r, fleet.contexts).apiInstance)),
    project: unique(fleet.projects.map(p => p.id)), hostState: unique(fleet.nodes.map(n => n.status)), seatState: unique(seats.map(s => s.status)),
    provider: unique(fleet.accounts.map(a => a.provider ?? 'local')), billing: unique(fleet.accounts.map(a => a.billing ?? 'none')),
    accountState: unique(fleet.accounts.map(a => a.exhausted_until ? 'exhausted' : a.status)), dataClass: unique(seats.flatMap(s => s.data_classes ?? ['internal'])),
    activity: ['in flight', 'idle'], enabled: ['enabled', 'disabled'], problems: ['has problems', 'no problems'],
  };
}
/** Intersect facets at the seat, retaining only its declared/observed neighborhood. */
export function filterFleet(fleet: Fleet, f: Filters): Fleet {
  if (!Object.values(f).some(Boolean)) return fleet;
  const matches = (key: Facet, value: string) => !f[key] || f[key] === value;
  const scopedUsage = fleet.usage.filter(r => matches('project', r.project) && matches('model', r.requested_model) && matches('repository', requestContext(r, fleet.contexts).repository ?? '') && matches('api', requestContext(r, fleet.contexts).apiInstance ?? ''));
  const requestScoped = !!(f.project || f.repository || f.api);
  const observedSeats = new Set(scopedUsage.flatMap(r => r.served ? [r.served.seat] : []));
  const nodes = fleet.nodes.map(n => ({ ...n, seats: n.seats.filter(s => {
    const a = fleet.accounts.find(a => a.id === s.account);
    return matches('host', n.id) && matches('hostState', n.status) && matches('seat', s.ref) && matches('engine', s.engine)
      && matches('account', s.account) && matches('seatState', s.status) && matches('provider', a?.provider ?? 'local')
      && matches('billing', a?.billing ?? 'none') && matches('accountState', a?.exhausted_until ? 'exhausted' : a?.status ?? 'unknown')
      && matches('activity', s.in_flight > 0 ? 'in flight' : 'idle') && matches('enabled', s.enabled === false ? 'disabled' : 'enabled')
      && matches('problems', s.problems.length ? 'has problems' : 'no problems')
      && (!f.dataClass || (s.data_classes ?? ['internal']).includes(f.dataClass))
      && (!f.model || s.serves.some(m => m.model === f.model))
      && (!requestScoped || observedSeats.has(s.ref))
      && (!f.search || [n.id, n.name, s.ref, s.engine, s.account, a?.name, ...s.serves.flatMap(m => [m.model, m.as])].join(' ').toLowerCase().includes(f.search.toLowerCase()));
  }) })).filter(n => n.seats.length);
  const refs = new Set(nodes.flatMap(n => n.seats.map(s => s.ref)));
  const accounts = new Set(nodes.flatMap(n => n.seats.map(s => s.account)));
  const resourceScoped = Object.entries(f).some(([key,value]) => value && !['project','repository','api','model'].includes(key));
  const usage = scopedUsage.filter(r => !resourceScoped || !!r.served && refs.has(r.served.seat));
  const projects = new Set(usage.map(r => r.project));
  return { contexts: fleet.contexts, nodes, accounts: fleet.accounts.filter(a => accounts.has(a.id)), projects: fleet.projects.filter(p => projects.has(p.id)), usage };
}
