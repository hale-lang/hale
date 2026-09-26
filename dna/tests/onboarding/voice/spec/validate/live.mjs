// Call every operation on a running stub and validate status, content type,
// headers and body against the bundled spec.  usage: node live.mjs <bundled.json> <base-url>
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
for (const [p, item] of Object.entries(spec.paths)) for (const [m, op] of Object.entries(item)) {
  if (m === 'parameters') continue;
  const code = Object.keys(op.responses).find(c => c.startsWith('2'));
  const url = base + p.replace(/\{(\w+)\}/g, (_, k) => sample[k]);
  const content = op.responses[code].content;
  if (!content) {
    n++;
    const body = ['post', 'put'].includes(m) ? JSON.stringify({ in_flight: 0, seats: [], assignment_version: 1, paused: false, paused_seats: [], max_in_flight: 4, engines: [], seat: 's', windows: [] }) : undefined;
    const r = await fetch(url, { method: m.toUpperCase(), body, headers: body ? { 'content-type': 'application/json' } : {} });
    if (String(r.status) !== code) fail(`${op.operationId} ${m.toUpperCase()} ${p}`, `status ${r.status} != ${code}`);
    continue;
  }
  const variants = content['application/json'] ? (content['text/event-stream'] ? [false, true] : [false]) : [true];
  const inference = !!content['application/json'];
  for (const stream of variants) {
    n++;
    const label = `${op.operationId}${stream ? ' (stream)' : ''} ${m.toUpperCase()} ${url.slice(base.length)}`;
    const body = ['post', 'put', 'patch'].includes(m) ? JSON.stringify(stream ? { model: 'echo-1', input: 'hi', stream: true } : { model: 'echo-1', input: 'hi' }) : undefined;
    const r = await fetch(url, { method: m.toUpperCase(), body, headers: body ? { 'content-type': 'application/json' } : {} });
    const text = await r.text();
    if (String(r.status) !== code) { fail(label, `status ${r.status} != ${code}: ${text.slice(0, 120)}`); continue; }
    const ct = r.headers.get('content-type') || '';
    const want = stream ? 'text/event-stream' : 'application/json';
    if (!ct.startsWith(want)) fail(label, `content-type ${ct} != ${want}`);
    if (p.startsWith('/v1/') && !r.headers.get('x-request-id')) fail(label, 'no X-Request-Id');
    if (!stream) {
      const f = ajv.compile(strip(content[want].schema));
      if (!f(JSON.parse(text))) fail(label, JSON.stringify(f.errors.slice(0, 3)));
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
console.log(`${n} live calls, ${fails} failures`);
process.exit(fails ? 1 : 0);
