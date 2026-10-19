import test from 'node:test';
import assert from 'node:assert/strict';
import { buildGraph, entityId, edgeId, requestEdges, requestVertices, requestContext, relatedRequests, matchingRequests, graphKinds, requestFlowEdges, layoutGraph, resizeLayout } from './network-model.ts';
import { demoFleet, demoRequest, demoInFlight, demoCompletions } from './network-demo.ts';

test('graph relationships come from configuration or usage, never all-to-all guesses', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const graph = buildGraph(fleet);
  const model = entityId('model', 'sonnet');
  const account = entityId('account', fleet.nodes.find(n => n.id === 'h1').seats.find(s => s.ref === 'h1/seat-1').account);
  assert(graph.edges.some((e) => e.id === edgeId(model, account) && e.kind === 'offers'));
  assert(!graph.edges.some((e) => e.id === edgeId(model, entityId('account', 'local'))));
  const noUsage = buildGraph({ ...fleet, usage: [] });
  assert(!noUsage.edges.some((e) => e.kind === 'observed'));
  assert.equal(noUsage.vertices.filter((v) => v.kind === 'project').length, fleet.projects.length);
});

test('a refused request without an attributed seat never lights a machine or payer', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const record = { ...demoRequest(fleet, 2, 1_800_000_000_000), served: null, status: 'no_capacity' };
  const graph = buildGraph({ ...fleet, usage: [record] });
  const lit = requestEdges(record, graph);
  assert.equal(lit.size, 1);
  assert(!lit.has(edgeId(entityId('project', record.project), entityId('model', record.requested_model))));
});

test('a served event traces its own model, seat, node and account only', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const record = demoRequest(fleet, 1, 1_800_000_000_000);
  const graph = buildGraph({ ...fleet, usage: [record] });
  const lit = requestEdges(record, graph);
  assert.equal(lit.size, 4);
  assert(lit.has(edgeId(entityId('seat', record.served.seat), entityId('node', record.served.node))));
  assert(lit.has(edgeId(entityId('seat', record.served.seat), entityId('account', record.served.account))));
});

test('occupancy updates preserve topology and a deterministic bounded layout', () => {
  const a = buildGraph(demoFleet(0, 1_800_000_000_000));
  const b = buildGraph(demoFleet(7, 1_800_000_000_000));
  assert.deepEqual(a.edges, b.edges);
  const first = layoutGraph(a, 600, 530);
  assert.deepEqual(first, layoutGraph(b, 600, 530));
  for (const p of first.values()) {
    assert(Number.isFinite(p.x) && Number.isFinite(p.y));
    assert(p.x >= 65 && p.x <= 535 && p.y >= 55 && p.y <= 465);
  }
});

test('large fleets stay bounded and never retain dangling edges', () => {
  const fixture = demoFleet(0, 1_800_000_000_000);
  const graph = buildGraph({ ...fixture, projects: Array.from({ length: 200 }, (_, i) => ({ ...fixture.projects[0], id: `p-${i}` })) });
  assert.equal(graph.vertices.length, 160);
  assert(graph.total > 160);
  const ids = new Set(graph.vertices.map((v) => v.id));
  assert(graph.edges.every((e) => ids.has(e.source) && ids.has(e.target)));
});

import { filterFleet } from './network-filters.ts';
test('combined host and backend filters retain only matching seat neighborhoods', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const result = filterFleet(fleet, { host: 'h1', engine: 'claude-cli', account: 'claude-work' });
  assert.deepEqual(result.nodes.map(n => n.id), ['h1']);
  assert.deepEqual(result.accounts.map(a => a.id), ['claude-work']);
  assert(result.usage.every(r => !r.served || r.served.node === 'h1'));
  assert.equal(filterFleet(fleet, { host: 'h1', engine: 'mlx' }).nodes.length, 0);
});
test('policy defaults and idle filtering follow the seat contract', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const result = filterFleet(fleet, { dataClass: 'internal', enabled: 'enabled', activity: 'idle' });
  assert(result.nodes.length > 0);
  assert(result.nodes.flatMap(n => n.seats).every(s => s.in_flight === 0));
  assert.equal(filterFleet(fleet, { dataClass: 'secret' }).nodes.length, 0);
});

test('unified layout is independent of response ordering and supports newly reported dimensions', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const graph = buildGraph(fleet);
  const points = layoutGraph(graph, 940, 740);
  assert.deepEqual(points, layoutGraph({ ...graph, vertices: [...graph.vertices].reverse(), edges: [...graph.edges].reverse() }, 940, 740));
  const record = { ...fleet.usage[1], id: 'extra', metadata: { branch: 'feature/new' } };
  const expanded = buildGraph({ ...fleet, usage: [...fleet.usage, record] });
  const next = layoutGraph(expanded, 940, 740, new Map(), points);
  assert(next.has('context:branch:feature/new'));
  for (const p of next.values()) assert(Number.isFinite(p.x) && p.x >= 65 && p.x <= 875 && p.y >= 55 && p.y <= 675);
  const displacement = [...points].reduce((sum,[id,p]) => sum + Math.hypot(next.get(id).x-p.x,next.get(id).y-p.y),0)/points.size;
  assert(displacement < 50, `Existing network moved ${displacement}px on average`);
});
test('drag pins are honored and bounded; reset restores the settled layout', () => {
  const graph = buildGraph(demoFleet(0, 1_800_000_000_000));
  const base = layoutGraph(graph, 940, 740);
  const pins = new Map([['project:research', { x: 250, y: 200 }], ['node:h1', { x: -100, y: 900 }]]);
  const dragged = layoutGraph(graph, 940, 740, pins);
  assert.deepEqual(dragged.get('project:research'), { x: 250, y: 200 });
  assert.deepEqual(dragged.get('node:h1'), { x: 65, y: 675 });
  assert.deepEqual(layoutGraph(graph, 940, 740, new Map()), base);
});

test('a request fuses all seven participants without borrowing another request path', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const first = { ...demoRequest(fleet, 1, 1_800_000_000_000), id: 'first', project: 'research', metadata: { repository: 'repo-a' } };
  const second = { ...first, id: 'second', project: 'product', metadata: { repository: 'repo-b' }, served: { ...first.served, node: 'historical-host', seat: 'historical-host/seat' } };
  fleet.usage = [first,second];
  fleet.contexts = { first: { apiInstance: 'api-a' }, second: { apiInstance: 'api-b' } };
  const graph = buildGraph(fleet);
  const path = requestVertices(first, graph);
  assert.equal(path.size, 7);
  assert.equal(requestEdges(first, graph).size, 6);
  assert(path.has('repository:repo-a') && path.has('api:api-a'));
  assert(!path.has('repository:repo-b') && !path.has('api:api-b') && !path.has('node:historical-host'));
  assert.deepEqual(relatedRequests('project:research', fleet.usage, graph).map(r => r.id), ['first']);
  assert.equal(relatedRequests(`model:${first.requested_model}`, fleet.usage, graph).length, 2);
  assert(requestVertices(second, graph).has('node:historical-host'));
});
test('caller metadata cannot spoof API instance identity; missing context stays absent', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const record = { ...fleet.usage[0], metadata: { apiInstance: 'spoofed', api_instance: 'spoofed' }, served: null };
  const graph = buildGraph({ ...fleet, usage: [record] });
  assert.equal(requestContext(record).apiInstance, undefined);
  assert(!graph.vertices.some(v => v.kind === 'api' || v.kind === 'repository'));
  assert.deepEqual([...requestVertices(record, graph)].sort(), [`project:${record.project}`, `model:${record.requested_model}`, 'context:apiInstance:spoofed', 'context:api_instance:spoofed'].sort());
  assert.equal(requestFlowEdges(record, graph).size, 0);
});
test('repository and API filters intersect on the same request, not a shared model', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const a = { ...fleet.usage[0], id: 'a', metadata: { repository: 'repo-a' } };
  const b = { ...a, id: 'b', metadata: { repository: 'repo-b' } };
  fleet.usage = [a,b]; fleet.contexts = { a: { apiInstance: 'api-a' }, b: { apiInstance: 'api-b' } };
  assert.equal(filterFleet(fleet, { repository: 'repo-a', api: 'api-b' }).usage.length, 0);
  assert.deepEqual(filterFleet(fleet, { repository: 'repo-a', api: 'api-a' }).usage.map(r => r.id), ['a']);
});

test('resource filters exclude unattributed requests; request facets preserve historical evidence', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const record = { ...fleet.usage[0], served: null };
  fleet.usage = [record];
  assert.equal(filterFleet(fleet, { host: 'studio1' }).usage.length, 0);
  assert.equal(filterFleet(fleet, { project: record.project }).usage.length, 1);
  fleet.usage = [{ ...record, served: { model: record.requested_model, node: 'retired', seat: 'retired/seat', account: 'old' } }];
  const filtered = filterFleet(fleet, { repository: record.metadata.repository });
  assert.equal(filtered.usage.length, 1);
  assert(requestVertices(filtered.usage[0], buildGraph(filtered)).has('node:retired'));
});

test('composable selection intersects whole requests and can explicitly union them', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const a = { ...fleet.usage[1], id: 'a', project: 'research', metadata: { repository: 'a' } };
  const b = { ...a, id: 'b', project: 'product', metadata: { repository: 'b' } };
  fleet.usage = [a,b];
  const graph = buildGraph(fleet);
  assert.deepEqual(matchingRequests(['project:research','repository:a'],fleet.usage,graph).map(r=>r.id),['a']);
  assert.equal(matchingRequests(['project:research','repository:b'],fleet.usage,graph).length,0);
  assert.equal(matchingRequests(['project:research','repository:b'],fleet.usage,graph,'any').length,2);
});
test('optional metadata dimensions emerge without a type registry entry', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const a = { ...fleet.usage[1], metadata: { workflow: 'review', 'dna.attempt_id': 'a-41' } };
  const graph = buildGraph({ ...fleet, usage: [a] });
  assert(graphKinds(graph).includes('context:dna.attempt_id'));
  assert(requestVertices(a,graph).has('context:workflow:review'));
  const bare = buildGraph({ ...fleet, usage: [{ ...a, metadata: {} }] });
  assert(!bare.vertices.some(v => v.kind.startsWith('context:') || v.kind === 'repository'));
});
test('execution ends at the served model, through its account, while requested model remains context', () => {
  const fleet = demoFleet(0, 1_800_000_000_000);
  const record = { ...fleet.usage[1], served: { ...fleet.usage[1].served, model: 'provider-specific-model' } };
  const graph = buildGraph({ ...fleet, usage: [record], contexts: { [record.id]: { apiInstance: 'api-1' } } });
  const flow = graph.edges.filter(e => requestFlowEdges(record,graph).has(e.id));
  assert.equal(flow.length,5);
  assert(flow.some(e => e.source === `account:${record.served.account}` && e.target === 'model:provider-specific-model'));
  assert(!flow.some(e => e.source.startsWith('model:')));
  assert(requestVertices(record,graph).has(`model:${record.requested_model}`));
  const refused = { ...record, served: null };
  assert.equal(requestFlowEdges(refused,buildGraph({ ...fleet, usage: [refused] })).size,0);
});


test('directed force layout spreads execution toward models across the available canvas', () => {
  const graph = buildGraph(demoFleet(0, 1_800_000_000_000));
  for (const width of [1040, 1580]) {
    const points = layoutGraph(graph, width, 900);
    for (const edge of graph.edges.filter(e => e.flow || e.kind === 'hosts' || e.kind === 'offers')) {
      assert(points.get(edge.target).x - points.get(edge.source).x > 65, `Execution reverses at ${edge.id}`);
    }
    const xs = [...points.values()].map(p => p.x);
    const ys = [...points.values()].map(p => p.y);
    assert(Math.max(...xs)-Math.min(...xs) > width*.7, 'Network collapsed horizontally');
    assert(Math.max(...ys)-Math.min(...ys) > 900*.7, 'Network collapsed vertically');
    const labels = graph.vertices.filter(v => v.kind !== 'seat');
    for (let i=0; i<labels.length; i++) for (let j=i+1; j<labels.length; j++) {
      const a = points.get(labels[i].id), b = points.get(labels[j].id);
      assert(Math.abs(a.x-b.x)>145 || Math.abs(a.y-b.y)>(labels[i].kind === 'model' && labels[j].kind === 'model' ? 29 : 60), `Overlapping labels: ${labels[i].id}, ${labels[j].id}`);
    }
  }
});
test('resizing existing positions uses the enlarged canvas instead of keeping the old knot', () => {
  const graph = buildGraph(demoFleet(0, 1_800_000_000_000));
  const initial = layoutGraph(graph,1040,610);
  const resized = resizeLayout(initial,{x:1040,y:610},{x:1580,y:900});
  const actual = layoutGraph(graph,1580,900,new Map(),resized);
  const fresh = layoutGraph(graph,1580,900);
  const displacement = [...fresh].reduce((sum,[id,p]) => sum+Math.hypot(actual.get(id).x-p.x,actual.get(id).y-p.y),0)/fresh.size;
  assert(displacement < 25, `Resizing retained stale positions: ${displacement}px`);
  assert([...actual.values()].some(p => p.x>1350));
});
test('missing optional stages leave a directed and usable execution layout', () => {
  const fleet = demoFleet(0,1_800_000_000_000);
  const graph = buildGraph({...fleet, projects:[], accounts:[], usage:[], contexts:undefined});
  const points = layoutGraph(graph,1040,700);
  assert(!graph.vertices.some(v => v.kind === 'project' || v.kind === 'api' || v.kind === 'repository'));
  for (const edge of graph.edges) assert(points.get(edge.target).x > points.get(edge.source).x);
});


test('simulated work never exceeds three distinct projects and occupancy matches their routes', () => {
  let peak = 0;
  const projects = new Set(), accounts = new Set(), models = new Set();
  for (let tick = 0; tick < 240; tick++) {
    const fleet = demoFleet(tick, 1_800_000_000_000);
    const jobs = demoInFlight(fleet,tick,1_800_000_000_000);
    assert(jobs.length <= 3);
    assert.equal(new Set(jobs.map(j => j.record.project)).size,jobs.length);
    assert.equal(fleet.nodes.reduce((sum,n) => sum+n.in_flight,0),jobs.length);
    for (const node of fleet.nodes) for (const seat of node.seats) {
      assert.equal(seat.in_flight,jobs.filter(j => j.record.served.seat === seat.ref).length);
    }
    for (const job of jobs) { projects.add(job.record.project); accounts.add(job.record.served.account); models.add(job.record.served.model); }
    const completed = demoCompletions(fleet,tick,1_800_000_000_000);
    assert(completed.every(r => !jobs.some(j => j.record.id === r.id)));
    peak = Math.max(peak,jobs.length);
  }
  assert.equal(peak,3);
  assert.equal(projects.size,7);
  assert.deepEqual([...accounts].sort(),['agy','claude-personal','claude-work','local','openai','vibe']);
  assert.equal(models.size,24);
});
test('a routed request follows one project-to-API-to-host-to-seat-to-account-to-model branch', () => {
  const fleet = demoFleet(0,1_800_000_000_000), record = fleet.usage[1];
  const graph = buildGraph({...fleet,usage:[record],contexts:{[record.id]:{apiInstance:'api-1'}}});
  const flow = graph.edges.filter(e => requestFlowEdges(record,graph).has(e.id));
  const participants = [`project:${record.project}`,'api:api-1',`node:${record.served.node}`,`seat:${record.served.seat}`,`account:${record.served.account}`,`model:${record.served.model}`];
  assert.equal(flow.length,5);
  for (let i=1;i<participants.length;i++) assert(flow.some(e=>e.source===participants[i-1] && e.target===participants[i]));
  assert(!graph.edges.some(e=>e.source.startsWith('project:') && e.target.startsWith('model:')));
});


test('homelab demo places CLI accounts on h1/h2 and MLX capacity on Macs', () => {
  const fleet = demoFleet(0,1_800_000_000_000);
  for (const node of fleet.nodes) for (const seat of node.seats) {
    if (seat.account !== 'local') assert(['h1','h2'].includes(node.id));
    if (['h1','h2'].includes(node.id) && seat.account === 'local') assert(seat.serves.every(s => ['llama-3b','llama-8b','qwen-7b'].includes(s.model)));
    if (!['h1','h2'].includes(node.id)) assert.equal(seat.engine,'mlx');
  }
});
