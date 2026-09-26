// Validate canned responses against voice's OpenAPI (bundled, dereferenced).
// usage: node validate.mjs <bundled.json> <canned-dir> [live-base-url]
import fs from 'fs';
import path from 'path';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

const [bundlePath, dir, live] = process.argv.slice(2);
const spec = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));
const ajv = new Ajv2020({ strict: false, allErrors: true, validateFormats: true });
addFormats(ajv);
ajv.addKeyword('discriminator');

let failures = 0, checked = 0;
const report = (label, ok, errs) => {
  checked++;
  if (!ok) { failures++; console.log(`FAIL ${label}\n  ` + JSON.stringify(errs.slice(0, 6), null, 0).replace(/},{/g, '},\n  {')); }
};
const strip = s => { const c = structuredClone(s); delete c.$schema; return c; };

function ops() {
  const out = [];
  for (const [p, item] of Object.entries(spec.paths))
    for (const [m, op] of Object.entries(item)) {
      if (m === 'parameters') continue;
      const code = Object.keys(op.responses).find(c => c.startsWith('2'));
      out.push({ path: p, method: m.toUpperCase(), id: op.operationId, code: Number(code), resp: op.responses[code] });
    }
  return out;
}

function checkJson(label, schema, text) {
  let v; try { v = JSON.parse(text); } catch (e) { report(label, false, [{ parse: e.message }]); return; }
  const f = ajv.compile(strip(schema));
  report(label, f(v), f.errors || []);
}

function checkSse(label, schema, text, chat) {
  const f = ajv.compile(strip(schema));
  const datas = text.split('\n\n').map(b => b.trim()).filter(Boolean).map(b => {
    const d = b.split('\n').find(l => l.startsWith('data: '));
    return d ? d.slice(6) : null;
  }).filter(d => d !== null);
  const inference = !label.startsWith('streamAdminEvents');
  if (inference && datas[datas.length - 1] !== '[DONE]') report(label + ' [DONE]', false, [{ msg: 'stream does not end with data: [DONE]' }]);
  let seq = 0;
  for (const d of (inference ? datas.slice(0, -1) : datas)) {
    let v; try { v = JSON.parse(d); } catch (e) { report(label, false, [{ parse: e.message, d }]); continue; }
    report(`${label} #${seq} ${v.type || v.object}`, f(v), f.errors || []);
    if (!chat && inference && v.sequence_number !== seq) report(label, false, [{ msg: `sequence_number ${v.sequence_number} != ${seq}` }]);
    seq++;
  }
}

const load = name => fs.readFileSync(path.join(dir, name), 'utf8');
for (const o of ops()) {
  const ct = o.resp.content || {};
  if (ct['application/json']) checkJson(`${o.id} (${o.method} ${o.path})`, ct['application/json'].schema, load(`${o.id}.json`));
  if (ct['text/event-stream']) {
    const chat = o.id === 'createChatCompletion';
    checkSse(`${o.id} stream`, ct['text/event-stream'].schema, load(`${o.id}.stream.sse`), chat);
  }
}
console.log(`${checked} checks, ${failures} failures`);
process.exit(failures ? 1 : 0);
