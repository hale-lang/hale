import type { Account, Node, Schemas, UsageRecord } from '../api/client';

export type CoreKind = 'project' | 'repository' | 'model' | 'api' | 'node' | 'seat' | 'account';
export type Kind = CoreKind | `context:${string}`;
export const KINDS: CoreKind[] = ['project', 'repository', 'model', 'api', 'node', 'seat', 'account'];
export const kindName = (kind: Kind): string => kind.startsWith('context:') ? kind.slice(8) : ({ project: 'Project', repository: 'Repository', model: 'Model', api: 'API instance', node: 'Execution host', seat: 'Seat', account: 'Account' }[kind as CoreKind]);
export const graphKinds = (graph: Graph): Kind[] => [...new Set(graph.vertices.map(v => v.kind))].sort((a, b) => kindName(a).localeCompare(kindName(b)));

export type RequestContext = { repository?: string; apiInstance?: string };
export type Fleet = { nodes: Node[]; accounts: Account[]; projects: Schemas['Project'][]; usage: UsageRecord[]; contexts?: Record<string, RequestContext> };
export type Vertex = { id: string; name: string; kind: Kind; state: string; active: number; details: [string, string][]; href?: string; parent?: string };
export type Edge = { id: string; source: string; target: string; kind: 'offers' | 'hosts' | 'pays' | 'observed'; flow?: boolean };
export type RequestPath = { vertices: string[]; edges: string[]; flowEdges: string[]; context: RequestContext };
export type Graph = { vertices: Vertex[]; edges: Edge[]; total: number; requests: Map<string, RequestPath> };
export type Point = { x: number; y: number };
export const entityId = (kind: Kind, name: string) => `${kind}:${name}`;
export const edgeId = (a: string, b: string) => [a, b].sort().join('|');
const link = (...parts: string[]) => '#/' + parts.map(encodeURIComponent).join('/');

/** Repository is an optional caller label. API identity must come from trusted observation, never caller metadata. */
export function requestContext(record: UsageRecord, contexts?: Fleet['contexts']): RequestContext {
  return { repository: record.metadata.repository?.trim() || undefined, ...contexts?.[record.id] };
}

/** Each request is a hyperedge: a correlated set of participants, rendered as links. */
export function buildGraph(fleet: Fleet): Graph {
  const vertices = new Map<string, Vertex>();
  const edges = new Map<string, Edge>();
  const requests = new Map<string, RequestPath>();
  const add = (v: Vertex) => vertices.set(v.id, v);
  const connect = (source: string, target: string, kind: Edge['kind'], flow = false) => {
    if (vertices.has(source) && vertices.has(target)) {
      const id = edgeId(source, target);
      // Configuration retains its style; the request membership is tracked separately.
      if (!edges.has(id)) edges.set(id, { id, source, target, kind, flow });
      else if (flow) edges.set(id, { ...edges.get(id)!, source, target, flow: true });
      return id;
    }
    return undefined;
  };
  for (const p of fleet.projects) add({ id: entityId('project', p.id), name: p.id, kind: 'project', state: p.status, active: 0, href: link('projects', p.id), details: [['Name', p.name], ['State', p.status]] });
  for (const a of fleet.accounts) add({ id: entityId('account', a.id), name: a.id, kind: 'account', state: a.exhausted_until ? 'exhausted' : a.status, active: 0, href: link('accounts', a.id), details: [['Billing', a.billing ?? 'none'], ['State', a.status], ...a.windows.map((w): [string, string] => [w.name.replaceAll('_', ' '), `${w.used_percent}% used · reported ${w.observed_at}`])] });
  for (const n of fleet.nodes) {
    add({ id: entityId('node', n.id), name: n.id, kind: 'node', state: n.status, active: n.in_flight ?? 0, href: link('nodes', n.id), details: [['Name', n.name ?? n.id], ['State', n.status], ['In flight / cap', `${n.in_flight ?? '—'} / ${n.effective_max_in_flight ?? '—'}`], ['Last heartbeat', n.last_heartbeat_at ?? 'not reported'], ['Assignment / applied', `${n.assignment_version ?? '—'} / ${n.applied_version ?? '—'}`]] });
    for (const s of n.seats) {
      add({ id: entityId('seat', s.ref), name: s.id, kind: 'seat', parent: entityId('node', n.id), state: s.status, active: s.in_flight, href: link('nodes', n.id, s.id), details: [['Seat', s.ref], ['Engine', s.engine], ['Account', s.account], ['State', s.status], ['In flight / cap', `${s.in_flight} / ${s.concurrency ?? 1}`], ['Reason', s.reason ?? (s.problems.join('; ') || 'none reported')]] });
      for (const m of s.serves) {
        const id = entityId('model', m.model);
        if (!vertices.has(id)) add({ id, name: m.model, kind: 'model', state: 'configured', active: 0, href: link('catalog', m.model), details: [['Catalog model', m.model], ['Meaning', 'Configured on a seat; not evidence of loaded weights.']] });
        // Account/model links summarize the models offered by its seats.
        connect(vertices.has(entityId('account', s.account)) ? entityId('account', s.account) : entityId('seat', s.ref), id, 'offers');
      }
      connect(entityId('node', n.id), entityId('seat', s.ref), 'hosts');
      connect(entityId('seat', s.ref), entityId('account', s.account), 'pays');
    }
  }
  for (const r of fleet.usage) {
    const context = requestContext(r, fleet.contexts);
    const members: string[] = [];
    const links: string[] = [];
    const flowLinks: string[] = [];
    const member = (kind: Kind, name: string, meaning = 'Seen in a request; current configuration is not reported.') => {
      const id = entityId(kind, name);
      if (!vertices.has(id)) add({ id, name, kind, state: 'observed', active: 0, details: [[kindName(kind), name], ['Evidence', meaning]] });
      members.push(id);
      return id;
    };
    const join = (a: string, b: string, flow = false) => { const id = connect(a, b, 'observed', flow); if (id) { links.push(id); if (flow) flowLinks.push(id); } };
    const project = member('project', r.project);
    member('model', r.requested_model);
    if (context.repository) join(project, member('repository', context.repository, 'Caller-supplied metadata.repository; a label, not verified repository ownership.'));
    for (const [key, value] of Object.entries(r.metadata)) {
      if (key === 'repository' || typeof value !== 'string' || !value.trim()) continue;
      join(project, member(`context:${key}`, value, `Caller-supplied metadata.${key}; request context, not verified infrastructure identity.`));
    }
    const api = context.apiInstance ? member('api', context.apiInstance, 'Request observation. Demo instances are synthetic; API identity is not currently exposed by UsageRecord.') : null;
    if (api) join(project, api, true);
    if (r.served) {
      const seat = member('seat', r.served.seat);
      const host = member('node', r.served.node);
      const account = member('account', r.served.account);
      const servedModel = member('model', r.served.model, 'Model reported by the serving engine; the terminal inference target.');
      join(host, seat, true); join(seat, account, true); join(account, servedModel, true);
      if (api) join(api, host, true);
    }
    requests.set(r.id, { vertices: [...new Set(members)], edges: [...new Set(links)], flowEdges: flowLinks, context });
  }
  const visible = [...vertices.values()].slice(0, 160);
  const ids = new Set(visible.map(v => v.id));
  return { vertices: visible, edges: [...edges.values()].filter(e => ids.has(e.source) && ids.has(e.target)), total: vertices.size, requests };
}

/** No transitive traversal: sharing a model/account cannot splice two requests together. */
export function requestEdges(record: UsageRecord, graph: Graph): Set<string> {
  const existing = new Set(graph.edges.map(e => e.id));
  return new Set((graph.requests.get(record.id)?.edges ?? []).filter(id => existing.has(id)));
}
export function requestFlowEdges(record: UsageRecord, graph: Graph): Set<string> {
  const existing = new Set(graph.edges.map(e => e.id));
  return new Set((graph.requests.get(record.id)?.flowEdges ?? []).filter(id => existing.has(id)));
}

export function requestVertices(record: UsageRecord, graph: Graph): Set<string> {
  const existing = new Set(graph.vertices.map(v => v.id));
  return new Set((graph.requests.get(record.id)?.vertices ?? []).filter(id => existing.has(id)));
}
export function relatedRequests(id: string, records: UsageRecord[], graph: Graph): UsageRecord[] {
  return records.filter(r => graph.requests.get(r.id)?.vertices.includes(id));
}
export function matchingRequests(ids: string[], records: UsageRecord[], graph: Graph, mode: 'all' | 'any' = 'all'): UsageRecord[] {
  if (!ids.length) return records;
  return records.filter(r => {
    const members = graph.requests.get(r.id)?.vertices ?? [];
    return mode === 'all' ? ids.every(id => members.includes(id)) : ids.some(id => members.includes(id));
  });
}

const hash = (value: string) => [...value].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

/** One field with a soft execution direction. Context never shortens the execution spine. */
export function layoutGraph(graph: Graph, width: number, height: number, pins: ReadonlyMap<string, Point> = new Map(), previous?: ReadonlyMap<string, Point>): Map<string, Point> {
  const vertices = [...graph.vertices].sort((a, b) => a.id.localeCompare(b.id));
  const edges = [...graph.edges].sort((a, b) => a.id.localeCompare(b.id));
  const byId = new Map(vertices.map(v => [v.id, v]));
  const neighbors = new Map(vertices.map(v => [v.id, [] as string[]]));
  for (const e of edges) { neighbors.get(e.source)?.push(e.target); neighbors.get(e.target)?.push(e.source); }
  const constrain = (p: Point) => ({ x: Math.max(65, Math.min(width - 65, p.x)), y: Math.max(55, Math.min(height - 65, p.y)) });
  const context = vertices.filter(v => v.kind === 'repository' || v.kind.startsWith('context:'));
  const stages = (['project', 'api', 'node', 'seat', 'account', 'model'] as Kind[]).filter(kind => vertices.some(v => v.kind === kind));
  const anchors = new Map<string, Point>();
  const span = width - 170;
  const start = 85 + (context.length ? span * .17 : 0);
  const modelStart = width - 190;
  const top = 95, bottom = Math.max(top, height - 90);
  const spreadY = (index: number, length: number) => length === 1 ? (top + bottom) / 2 : top + index * (bottom - top) / (length - 1);
  // Shared model neighborhoods order hosts before their seats, reducing crossings.
  const modelIds = vertices.filter(v => v.kind === 'model').map(v => v.id);
  const modelOrder = (v: Vertex): number => {
    const seats = neighbors.get(v.id)!.filter(id => byId.get(id)?.kind === 'seat');
    const accounts = seats.flatMap(id => neighbors.get(id)!).filter(id => byId.get(id)?.kind === 'account');
    const models = [...new Set([...seats, ...accounts].flatMap(id => neighbors.get(id)!).filter(id => byId.get(id)?.kind === 'model'))];
    return models.length ? models.reduce((sum, id) => sum + modelIds.indexOf(id), 0) / models.length : modelIds.length;
  };
  for (const [stage, kind] of stages.entries()) {
    const group = vertices.filter(v => v.kind === kind);
    if (kind === 'node') group.sort((a,b) => modelOrder(a) - modelOrder(b) || a.id.localeCompare(b.id));
    if (kind === 'model' || kind === 'account') {
      const meanY = (v: Vertex) => {
        const related = neighbors.get(v.id)!.flatMap(id => anchors.has(id) ? [anchors.get(id)!.y] : []);
        return related.length ? related.reduce((sum,y) => sum+y,0)/related.length : height/2;
      };
      group.sort((a,b) => meanY(a)-meanY(b) || a.id.localeCompare(b.id));
    }
    const x = stages.length === 1 ? width / 2 : start + stage * (modelStart - start) / (stages.length - 1);
    group.forEach((v, index) => {
      const hostId = v.parent ?? neighbors.get(v.id)!.find(id => byId.get(id)?.kind === 'node');
      const host = kind === 'seat' && hostId ? anchors.get(hostId) : undefined;
      const siblings = hostId ? group.filter(s => s.parent === hostId || neighbors.get(s.id)!.includes(hostId)) : [];
      const offset = host ? (siblings.indexOf(v) - (siblings.length - 1) / 2) * 46 : 0;
      const weights = group.map(item => 1 + neighbors.get(item.id)!.filter(id => byId.get(id)?.kind === 'seat').length);
      const hostY = top + (weights.slice(0,index).reduce((sum,n) => sum+n,0) + weights[index]! / 2) / weights.reduce((sum,n) => sum+n,0) * (bottom-top);
      const y = kind === 'model' ? group.length === 1 ? height / 2 : top + index * (height - 160) / (group.length - 1) : host ? host.y + offset : kind === 'node' ? hostY : spreadY(index, group.length);
      anchors.set(v.id, { x, y });
    });
  }
  const contextY = (v: Vertex) => {
    const related = neighbors.get(v.id)!.flatMap(id => anchors.has(id) ? [anchors.get(id)!.y] : []);
    return related.length ? related.reduce((sum,y) => sum+y,0)/related.length : height/2;
  };
  context.sort((a,b) => contextY(a)-contextY(b) || a.id.localeCompare(b.id));
  context.forEach((v,index) => anchors.set(v.id, { x: 85, y: spreadY(index, context.length) }));
  const points = new Map(vertices.map(v => {
    const anchor = anchors.get(v.id)!, saved = previous?.get(v.id);
    const initial = saved ? { x: saved.x * .1 + anchor.x * .9, y: saved.y * .1 + anchor.y * .9 } : anchor;
    return [v.id, constrain(pins.get(v.id) ?? initial)];
  }));
  for (let step = 0; step < 220; step++) {
    const force = new Map(vertices.map(v => [v.id, { x: 0, y: 0 }]));
    for (let i = 0; i < vertices.length; i++) for (let j = i + 1; j < vertices.length; j++) {
      const a = vertices[i]!, b = vertices[j]!, pa = points.get(a.id)!, pb = points.get(b.id)!;
      const dx = pa.x - pb.x || .1, dy = pa.y - pb.y || .1;
      const distance = Math.max(1, Math.hypot(dx, dy));
      const repulsion = Math.min(6, 1700 / (distance * distance));
      let fx = dx / distance * repulsion, fy = dy / distance * repulsion;
      // Reserve both the glyph and its label, including labels revealed by selection.
      const overlapX = (a.kind === 'seat' && b.kind === 'seat' ? 112 : 158) - Math.abs(dx);
      const overlapY = (a.kind === 'model' && b.kind === 'model' ? 30 : a.kind === 'seat' && b.kind === 'seat' ? 46 : 82) - Math.abs(dy);
      if (overlapX > 0 && overlapY > 0) {
        if (overlapX < overlapY) fx += Math.sign(dx) * overlapX * .9;
        else fy += Math.sign(dy) * overlapY * .9;
      }
      force.get(a.id)!.x += fx; force.get(a.id)!.y += fy;
      force.get(b.id)!.x -= fx; force.get(b.id)!.y -= fy;
    }
    for (const e of edges) {
      const a = points.get(e.source)!, b = points.get(e.target)!;
      const aa = anchors.get(e.source)!, ba = anchors.get(e.target)!;
      const structural = e.kind === 'hosts' || e.kind === 'offers' || e.flow;
      const strength = (structural ? .065 : .012) / Math.sqrt(Math.max(neighbors.get(e.source)!.length, neighbors.get(e.target)!.length));
      // Springs retain their natural directed span instead of collapsing long context links.
      const fx = ((b.x - a.x) - (ba.x - aa.x)) * strength - (structural ? Math.max(0, 85 - (b.x-a.x)) * .4 : 0);
      const fy = ((b.y - a.y) - (e.kind === 'hosts' ? ba.y - aa.y : 0)) * strength;
      force.get(e.source)!.x += fx; force.get(e.source)!.y += fy;
      force.get(e.target)!.x -= fx; force.get(e.target)!.y -= fy;
    }
    for (const v of vertices) {
      if (pins.has(v.id)) continue;
      const p = points.get(v.id)!, f = force.get(v.id)!, anchor = anchors.get(v.id)!;
      const saved = previous?.get(v.id);
      f.x += (anchor.x - p.x) * .12 + ((saved?.x ?? p.x) - p.x) * .008;
      f.y += (anchor.y - p.y) * .1 + ((saved?.y ?? p.y) - p.y) * .008;
      const cooling = 1 - step / 300;
      const next = constrain({ x: p.x + Math.max(-12,Math.min(12,f.x)) * cooling, y: p.y + Math.max(-12,Math.min(12,f.y)) * cooling });
      // Neural layers retain their alignment; only explicit pins leave a layer.
      p.x = constrain(anchor).x;
      p.y = v.kind === 'model' || v.kind === 'account' ? constrain(anchor).y : next.y;
    }
  }
  return points;
}

/** Preserve the mental map proportionally when the canvas changes size. */
export function resizeLayout(points: ReadonlyMap<string, Point>, from: Point, to: Point): Map<string, Point> {
  return new Map([...points].map(([id,p]) => [id, { x: p.x * to.x / from.x, y: p.y * to.y / from.y }]));
}

export function edgePath(a: Point, b: Point, id: string, orthogonal = false): string {
  if (orthogonal) return `M${a.x},${a.y} H${(a.x+b.x)/2} V${b.y} H${b.x}`;
  const dx = b.x - a.x, dy = b.y - a.y, length = Math.max(1, Math.hypot(dx, dy));
  const bend = (hash(id) % 41) - 20;
  return `M${a.x},${a.y} Q${(a.x + b.x) / 2 - dy / length * bend},${(a.y + b.y) / 2 + dx / length * bend} ${b.x},${b.y}`;
}
