// Check a captured stream (a text/event-stream body) against the contract:
// every event valid for its endpoint, `data: [DONE]` last, and the ending
// expected. `failed` is a stream whose seat failed once it had begun: on
// /v1/responses an `error` event, then `response.failed` with `voice` and its
// attempts; on /v1/chat/completions a last chunk finished `error`, with an
// `error` object, `usage: null` and `voice`.
//   usage: node sse-check.mjs <bundled.json> <path> <captured file> [failed]
import fs from 'fs';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
const [bundlePath, path, file, expect] = process.argv.slice(2);
const spec = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));
const ajv = new Ajv2020({ strict: false, allErrors: true }); addFormats(ajv); ajv.addKeyword('discriminator');
const schema = structuredClone(spec.paths[path].post.responses['200'].content['text/event-stream'].schema);
delete schema.$schema;
const f = ajv.compile(schema);
let fails = 0;
const fail = m => { fails++; console.log('FAIL', path, m); };
const events = [];
for (const block of fs.readFileSync(file, 'utf8').split('\n\n')) {
  if (!block.trim()) continue;
  const lines = block.split('\n');
  if (lines.every(l => l.startsWith(':'))) continue; // a keep-alive comment
  events.push({ event: lines.find(l => l.startsWith('event: '))?.slice(7) || '', data: lines.filter(l => l.startsWith('data: ')).map(l => l.slice(6)).join('\n') });
}
if (events.at(-1)?.data !== '[DONE]') fail('does not end with data: [DONE]');
const body = events.slice(0, -1).map(e => { const j = JSON.parse(e.data); if (!f(j)) fail((j.type || 'chunk') + ' ' + JSON.stringify(f.errors.slice(0, 2))); return j; });
const last = body.at(-1);
if (expect === 'failed') {
  if (path === '/v1/responses') {
    const err = body.at(-2);
    if (err?.type !== 'error') fail('no error event before the end');
    if (last?.type !== 'response.failed' || last.response?.status !== 'failed') fail('does not end with response.failed');
    if (!last?.response?.voice?.attempts?.length) fail('response.failed carries no voice attempts');
    if (body.some(j => j.type === 'response.completed')) fail('a failed stream says completed');
  } else {
    const c = last?.choices?.[0];
    if (c?.finish_reason !== 'error' || !last.error?.code) fail('the last chunk is not finished error with an error object');
    if (last.usage !== null) fail('usage is not null');
    if (!last.voice?.attempts?.length) fail('the last chunk carries no voice attempts');
    if (body.some(j => ['stop', 'length', 'tool_calls'].includes(j.choices?.[0]?.finish_reason))) fail('a failed stream ends as a complete one');
  }
}
console.log(path, 'captured stream:', events.length, 'events,', expect || 'served', fails ? `${fails} failures` : 'valid');
process.exit(fails ? 1 : 0);
