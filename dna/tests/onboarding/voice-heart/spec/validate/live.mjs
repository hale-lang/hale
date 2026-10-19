// Call every operation on a running api and validate status, content type,
// headers and body against the bundled spec.  usage: node live.mjs <bundled.json> <base-url>
//
// The api starts empty, so the sample world is made first, through the
// admin plane, and every call presents the key it yields. Operations that
// create get fresh ids, and the ones that delete run last, so each call
// finds what it names.
import fs from 'fs';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
const [bundlePath, base] = process.argv.slice(2);
const spec = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));
const ajv = new Ajv2020({ strict: false, allErrors: true }); addFormats(ajv); ajv.addKeyword('discriminator');
const sample = { project: 'work', key: 'k_ci', model: 'echo-1', account: 'claude-work', node: 'laptop', seat: 'static', request_id: 'resp_01j8z3q4v7m2k9x0' };
let fails = 0, n = 0;
const fail = (l, m) => { fails++; console.log('FAIL', l, m); };
const strip = s => { const c = structuredClone(s); delete c.$schema; return c; };

// the sample world: a model, an account, a project with its budget and a
// key under it, and a node with a seat
const setup = async (path, body) => {
  const r = await fetch(base + path, { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } });
  const text = await r.text();
  if (r.status !== 201) { console.log('FAIL setup', path, r.status, text.slice(0, 200)); process.exit(1); }
  return JSON.parse(text);
};
await setup('/admin/v1/models', { id: 'echo-1', context_length: 8192, max_output_tokens: 1024, price: { input_per_mtok_micros: 3000000, output_per_mtok_micros: 15000000, currency: 'USD' } });
await setup('/admin/v1/accounts', { id: 'claude-work', name: 'Claude (work)', provider: 'anthropic', billing: 'subscription' });
await setup('/admin/v1/projects', { id: 'work', name: 'Work', owners: ['operator@example.com'], data_classes: ['public', 'internal'], budget: { allowance_micros: 50000000, period: 'month' } });
const key = await setup('/admin/v1/projects/work/keys', { name: 'ci' });
const nodeToken = (await setup('/admin/v1/nodes', { id: 'laptop', name: 'Laptop' })).enrollment_token;
await setup('/admin/v1/nodes/laptop/seats', { id: 'static', engine: 'static', account: 'claude-work', serves: [{ model: 'echo-1' }], config: { text: 'hello from voice' } });
sample.key = key.id;
const auth = { authorization: `Bearer ${key.secret}` };

// the bodies the real handlers take; anything else gets the generic one
const bodies = {
  createCatalogModel: { id: 'live-1', kind: 'hosted', owned_by: 'anthropic', price: null },
  updateCatalogModel: { max_output_tokens: 2048 },
  createProject: { id: 'live', name: 'Live' },
  updateProject: { name: 'Work' },
  createKey: { name: 'live', expires_at: '2099-01-01T00:00:00Z' },
  updateKey: { name: 'ci' },
  setBudget: { allowance_micros: 75000000, period: 'month' },
  setLimits: { limits: [{ scope: 'project', window_seconds: 60, requests: 600 }, { scope: 'model', model: 'echo-1', window_seconds: 3600, requests: 200 }] },
  createAccount: { id: 'static', name: 'Static' },
  updateAccount: { name: 'Claude (work)', billing: 'subscription' },
  createNode: { id: 'desktop', max_in_flight: 4 },
  updateNode: { max_in_flight: 8 },
  createSeat: { id: 'live', engine: 'static', account: 'claude-work', serves: [{ model: 'echo-1', as: null }], data_classes: ['public', 'internal'], concurrency: 2, config: { text: 'hi', chunk_chars: 4 } },
  updateSeat: { concurrency: 4 },
  createChatCompletion: { model: 'echo-1', messages: [{ role: 'user', content: 'hi' }] },
};

// the node plane: the node's enrollment token, and the bodies it sends
// (it registers first, as the spec lists it)
let nodeAuth = { authorization: `Bearer ${nodeToken}` };
const capabilities = { max_in_flight: 4, engines: [{ id: 'static', kind: 'static', models: null, config_schema: { type: 'object', required: ['text'] } }] };
const local = { paused: false, paused_seats: [], max_in_flight: null, reason: null };
const nodeBodies = {
  registerNode: { protocol: 2, version: 'live', capabilities, local },
  nodeHeartbeat: { in_flight: 0, seats: [{ id: 'static', status: 'up', in_flight: 0 }] },
  reportSeatStates: { assignment_version: 1, seats: [{ id: 'static', status: 'up', in_flight: 0 }] },
  reportCapabilities: capabilities,
  reportLocalControls: local,
  reportQuota: { seat: 'static', windows: [] },
  nodeDraining: { reason: 'live check' },
};

// what an endpoint answers instead of its success, while it cannot succeed:
// inference needs a node to serve it, streamed or not (a stream opens only
// once a seat answers, so a refusal is answered whole)
const refusals = {
  createResponse: { json: [503, 'no_eligible_seat'], stream: [503, 'no_eligible_seat'] },
  createChatCompletion: { json: [503, 'no_eligible_seat'], stream: [503, 'no_eligible_seat'] },
  // no brain works here, so no route table reaches the api
  getRoutes: { json: [503, 'route_table_unavailable'] },
};

const calls = [], last = [];
for (const [p, item] of Object.entries(spec.paths)) for (const [m, op] of Object.entries(item)) {
  if (m === 'parameters') continue;
  (m === 'delete' ? last : calls).push([p, m, op]);
}
for (const [p, m, op] of [...calls, ...last]) {
  const code = Object.keys(op.responses).find(c => c.startsWith('2'));
  const url = base + p.replace(/\{(\w+)\}/g, (_, k) => sample[k]);
  const content = op.responses[code].content;
  const who = p.startsWith('/node/') ? nodeAuth : auth;
  if (!content) {
    n++;
    const body = ['post', 'put'].includes(m) ? JSON.stringify(nodeBodies[op.operationId] ?? {}) : undefined;
    const r = await fetch(url, { method: m.toUpperCase(), body, headers: body ? { 'content-type': 'application/json', ...who } : who });
    if (String(r.status) !== code) fail(`${op.operationId} ${m.toUpperCase()} ${p}`, `status ${r.status} != ${code}`);
    continue;
  }
  const variants = content['application/json'] ? (content['text/event-stream'] ? [false, true] : [false]) : [true];
  const inference = !!content['application/json'];
  for (const stream of variants) {
    n++;
    const label = `${op.operationId}${stream ? ' (stream)' : ''} ${m.toUpperCase()} ${url.slice(base.length)}`;
    const generic = stream ? { model: 'echo-1', input: 'hi', stream: true } : { model: 'echo-1', input: 'hi' };
    const own = bodies[op.operationId] ?? nodeBodies[op.operationId];
    const payload = own ? (stream ? { ...own, stream: true } : own) : generic;
    const body = ['post', 'put', 'patch'].includes(m) ? JSON.stringify(payload) : undefined;
    const r = await fetch(url, { method: m.toUpperCase(), body, headers: body ? { 'content-type': 'application/json', ...who } : who });
    const text = await r.text();
    const refusal = refusals[op.operationId]?.[stream ? 'stream' : 'json'];
    if (refusal) {
      const [status, errCode] = refusal;
      if (r.status !== status) { fail(label, `status ${r.status} != ${status}: ${text.slice(0, 120)}`); continue; }
      const f = ajv.compile(strip(op.responses[String(status)].content['application/json'].schema));
      const j = JSON.parse(text);
      if (!f(j)) fail(label, JSON.stringify(f.errors.slice(0, 3)));
      if (j?.error?.code !== errCode) fail(label, `code ${j?.error?.code} != ${errCode}`);
      continue;
    }
    if (String(r.status) !== code) { fail(label, `status ${r.status} != ${code}: ${text.slice(0, 120)}`); continue; }
    const ct = r.headers.get('content-type') || '';
    const want = stream ? 'text/event-stream' : 'application/json';
    if (!ct.startsWith(want)) fail(label, `content-type ${ct} != ${want}`);
    if (p.startsWith('/v1/') && !r.headers.get('x-request-id')) fail(label, 'no X-Request-Id');
    if (!stream) {
      const f = ajv.compile(strip(content[want].schema));
      const j = JSON.parse(text);
      if (!f(j)) fail(label, JSON.stringify(f.errors.slice(0, 3)));
      // a rotation retires the token the node plane presents
      if (op.operationId === 'rotateNodeToken') nodeAuth = { authorization: `Bearer ${j.enrollment_token}` };
    } else {
      const f = ajv.compile(strip(content[want].schema));
      const datas = text.split('\n\n').map(b => b.split('\n').find(l => l.startsWith('data: '))).filter(Boolean).map(l => l.slice(6));
      if (inference && datas.at(-1) !== '[DONE]') fail(label, 'no [DONE]');
      for (const d of (inference ? datas.slice(0, -1) : datas)) if (!f(JSON.parse(d))) fail(label, JSON.stringify(f.errors.slice(0, 3)));
    }
  }
}
const nf = await fetch(base + '/nope'); n++;
const nfb = await nf.json().catch(() => null);
if (nf.status !== 404 || nfb?.error?.code !== 'not_found') fail('unknown route', `${nf.status} ${JSON.stringify(nfb)}`);
// a caller-plane read without a key is refused in the contract's envelope
const anon = await fetch(base + '/v1/models'); n++;
const anonb = await anon.json().catch(() => null);
if (anon.status !== 401 || anonb?.error?.code !== 'invalid_key') fail('no key', `${anon.status} ${JSON.stringify(anonb)}`);
console.log(`${n} live calls, ${fails} failures`);
process.exit(fails ? 1 : 0);
