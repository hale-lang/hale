import fs from 'fs';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
const [bundle, base] = process.argv.slice(2);
const spec = JSON.parse(fs.readFileSync(bundle, 'utf8'));
const ajv = new Ajv2020({ strict: false, allErrors: true }); addFormats(ajv); ajv.addKeyword('discriminator');
const calls = [
  ['get', '/healthz'], ['get', '/node/v1/status'], ['get', '/node/v1/capabilities'],
  ['get', '/node/v1/local'], ['put', '/node/v1/local', { paused: true, paused_seats: [], max_in_flight: null, reason: 'test' }],
  ['get', '/node/v1/status'],
];
let fails = 0;
for (const [m, p, body] of calls) {
  const op = spec.paths[p][m];
  const code = Object.keys(op.responses).find(c => c.startsWith('2'));
  const r = await fetch(base + p.replace('{seat}', 'static'), { method: m.toUpperCase(), body: body && JSON.stringify(body) });
  const j = await r.json();
  const f = ajv.compile((({ $schema, ...s }) => s)(op.responses[code].content['application/json'].schema));
  const ok = String(r.status) === code && f(j);
  if (!ok) fails++;
  console.log(ok ? 'ok  ' : 'FAIL', m.toUpperCase(), p, r.status, ok ? '' : JSON.stringify(f.errors?.slice(0, 3)));
}
process.exit(fails ? 1 : 0);
