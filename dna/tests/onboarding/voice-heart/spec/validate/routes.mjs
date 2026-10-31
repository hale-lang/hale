// Read a running api's route table and validate it against the bundled
// spec (internal.yaml's RouteTable, through getRoutes).
//   usage: node routes.mjs <bundled.json> <base-url> <seat> [model]
// Exits 0 when the table is valid and `seat` routes `model` (echo-1).
import fs from 'fs';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
const [bundlePath, base, seat, model = 'echo-1'] = process.argv.slice(2);
const spec = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));
const ajv = new Ajv2020({ strict: false, allErrors: true }); addFormats(ajv);
const schema = structuredClone(spec.paths['/admin/v1/routes'].get.responses['200'].content['application/json'].schema);
delete schema.$schema;
const r = await fetch(base + '/admin/v1/routes');
const text = await r.text();
if (r.status !== 200) { console.log('routes', r.status, text.slice(0, 200)); process.exit(1); }
const table = JSON.parse(text);
const f = ajv.compile(schema);
if (!f(table)) { console.log('FAIL routes', JSON.stringify(f.errors.slice(0, 3))); process.exit(1); }
const routes = table.models.find(m => m.model === model);
const found = routes?.seats.find(s => s.seat === seat);
if (!found) { console.log('FAIL', seat, 'does not route', model, JSON.stringify(routes)); process.exit(1); }
console.log('route table version', table.version, ':', model, 'on', seat, 'free', found.free);
