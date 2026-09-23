// Real HTTP contract boundary proof. Outcomes come only from the native
// provider, host, Body, Review and Knowledge projection on an isolated Record.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { startNodeService, nodeEnvironmentPresent } from './native-knowledge-node-harness.mjs';

assert(nodeEnvironmentPresent(), 'Supply HALE_NATIVE_COMMAND_{API,BODY,RELAY,MEMBRANE} and HALE_DNA_MEMORY_DSN_OWNER.');
const parent = process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
assert(path.isAbsolute(parent), 'Evidence parent must be absolute.');
await mkdir(parent, { recursive: true });
const evidence = await mkdtemp(path.join(parent, 'native-node-http-'));
const results = [];
let service, failure;
const started = Date.now();
const bytes = value => Buffer.byteLength(value, 'utf8');
const hash = value => createHash('sha256').update(value, 'utf8').digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const save = (name, value) => writeFile(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n');
async function check(name, run) {
  const began = Date.now();
  try {
    const details = await run();
    results.push({ name, ok: true, elapsed_ms: Date.now() - began, ...details });
    console.log(JSON.stringify({ case: name, ok: true }));
  } catch (error) {
    results.push({ name, ok: false, elapsed_ms: Date.now() - began, error: error.stack || String(error) });
    throw error;
  }
}
const headers = () => ({ Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' });
const input = text => ({ kind: 'idea', name: 'exact-http-boundary', text, author: 'org', target: 'org', rationale: 'Preserve café 🧭\r\nexact evidence \u0001.' });
const admissions = requestId => service.facts('knowledge.node.requested').filter(row => {
  try { return JSON.parse(row.data.command_payload).request_id === requestId; } catch { return false; }
});
async function unchangedRefusal(command, status, code) {
  const before = service.journal().head;
  const response = await service.post(command);
  assert.equal(response.status, status, JSON.stringify(response));
  assert.equal(response.body.error?.code, code, JSON.stringify(response));
  assert.equal(service.journal().head, before, 'refused command must not append a request');
  assert.equal(admissions(command.request_id).length, 0);
  return { status, code, request_id: command.request_id };
}
const timer = setTimeout(async () => {
  console.error('Native node HTTP acceptance exceeded its 120-second wall budget.');
  try { await service?.stop(); } finally { process.exit(124); }
}, 120_000);

try {
  service = await startNodeService({ evidenceParent: evidence, rootPrefix: '/tmp/hale-face-browser.node-http.' });
  const prefix = '\u0001'.repeat(7000) + '\r\nCafé 東京 🧭';
  const text = prefix + 'x'.repeat(8192 - bytes(prefix));
  assert.equal(bytes(text), 8192);
  let command, created, adopted;

  await check('node profile advertises native Review and exact limits', async () => {
    const response = await service.capability('dna.knowledge.node.propose');
    assert.equal(response.status, 200, JSON.stringify(response));
    assert.equal(response.body.data.profile, 'dna.knowledge.node.propose.v1');
    assert.equal(response.body.data.mode, 'review');
    assert.equal(response.body.data.available, true); assert.equal(response.body.data.authorized, true);
    assert.equal(response.body.data.max_text_bytes, '8192'); assert.equal(response.body.data.max_request_bytes, '98304');
    return { profile: response.body.data.profile };
  });
  await check('8193 decoded text bytes are refused without admission', async () => {
    await service.quiesce();
    const oversized = await service.command('node.propose', input(text + 'x'), 'org', 'too-long-' + randomUUID());
    assert.equal(bytes(oversized.arguments.text), 8193); assert(bytes(JSON.stringify(oversized)) < 98304);
    return unchangedRefusal(oversized, 400, 'invalid_command');
  });
  await check('ungranted author and target scopes cannot create a node', async () => {
    const values = [];
    for (const pair of [{ author: 'org/restricted', target: 'org' }, { author: 'org', target: 'org/restricted' }]) {
      await service.quiesce();
      const forbidden = await service.command('node.propose', { ...input('Scoped evidence.'), ...pair }, pair.target, 'scope-' + randomUUID());
      values.push(await unchangedRefusal(forbidden, 403, 'forbidden'));
    }
    return { attempts: values };
  });
  await check('8192 control-heavy UTF8 bytes create an exact canonical candidate and Review', async () => {
    command = await service.command('node.propose', input(text), 'org', 'maximum-' + randomUUID());
    const encoded = bytes(JSON.stringify(command)); assert(encoded > 32768 && encoded < 98304);
    const response = await service.post(command); assert.equal(response.status, 202, JSON.stringify(response));
    created = await service.waitNode(command.request_id, receipt => receipt.node.proposal_state === 'created');
    assert.equal(created.state, 'succeeded'); assert.equal(created.node.activation_state, 'unknown');
    assert.equal(admissions(command.request_id).length, 1);
    const canonical = service.candidate(created.node.candidate_digest);
    assert.equal(canonical.text, text); assert.equal(canonical.kind, 'idea');
    assert.equal(canonical.author, 'org'); assert.equal(canonical.target, 'org');
    await service.quiesce();
    const read = await service.read(service.apiPath + '/dna/practices?' + new URLSearchParams({ id: created.node.candidate_digest }));
    assert.equal(read.status, 200, JSON.stringify(read));
    const row = read.json.data.items[0]; assert.equal(row.id, created.node.candidate_digest); assert.equal(row.digest, row.id);
    assert.equal(row.text, text); assert.equal(row.text_available, true); assert.equal(row.state, 'pending'); assert.equal(row.review_id, created.node.review_id);
    const review = await service.read(service.apiPath + '/dna/reviews?' + new URLSearchParams({ id: created.node.review_id, snapshot: read.json.source.record_head }));
    assert.equal(review.status, 200, JSON.stringify(review));
    const r = review.json.data.items[0]; assert.equal(r.subject_digest, row.id); assert.equal(r.knowledge_digest, row.id); assert.equal(r.state, 'pending');
    assert.equal(r.question, 'ratify idea `' + text + '` as a initiative for org?');
    assert.equal(r.is_mutation, false); assert.equal(r.approvers, ''); assert.equal(r.required_authority, 'board');
    await save('canonical-read.json', read.json); await save('proposal-receipt.json', created);
    return { request_id: command.request_id, candidate: row.id, text_bytes: bytes(text), text_sha256: hash(text), encoded_request_bytes: encoded };
  });
  await check('actual exact Review approval adopts and projects the same bytes', async () => {
    const requestId = 'approve-' + randomUUID();
    const verdict = {
      request_id: requestId, operation: 'dna.review.verdict', operation_version: '1',
      context: { application_id: service.application, position_id: 'org' },
      target: { application_id: service.application, kind: 'dna.review', id: created.node.review_id },
      preconditions: { principal: service.principal, subject_digest: created.node.candidate_digest, review_state: 'pending' },
      arguments: { verdict: 'approve', comment: 'Approve exact café 🧭\r\ncontrol \u0001 evidence.' },
    };
    const response = await service.request(service.apiPath + '/commands', { method: 'POST', headers: headers(), body: JSON.stringify(verdict) });
    assert([200, 202].includes(response.status), JSON.stringify(response));
    const decided = await service.waitCommand(requestId, receipt => receipt.verdict.state === 'accepted' && receipt.activation.state === 'adopted');
    adopted = await service.waitNode(command.request_id, receipt => receipt.node.activation_state === 'adopted');
    assert.equal(adopted.node.review_outcome, 'approve'); assert.equal(adopted.node.candidate_digest, created.node.candidate_digest);
    assert.equal(service.facts('knowledge.ratified', created.node.candidate_digest).length, 1);
    let graph;
    const deadline = Date.now() + 20_000;
    do {
      graph = await service.request(service.apiPath + '/dna/knowledge/nodes?' + new URLSearchParams({ id: created.node.candidate_digest }));
      if (graph.status === 200 && graph.body.data.items[0]?.accepted === true) break;
      assert([200, 404, 409, 501, 503].includes(graph.status), JSON.stringify(graph)); await delay(100);
    } while (Date.now() < deadline);
    assert.equal(graph.status, 200, JSON.stringify(graph));
    const node = graph.body.data.items[0]; assert.equal(node.id, created.node.candidate_digest); assert.equal(node.accepted, true);
    assert.equal(node.projection_state, 'ratified'); assert.equal(node.text, text); assert.equal(bytes(node.text), 8192);
    await save('adoption-receipt.json', adopted); await save('verdict-receipt.json', decided); await save('graph-read.json', graph.body);
    return { candidate: node.id, text_sha256: hash(node.text), review: created.node.review_id, graph_revision: graph.body.source.record_revision };
  });
  await check('adopted control-heavy node is a valid exact relationship endpoint', async () => {
    await service.quiesce();
    const args = { from_id: created.node.candidate_digest, to_id: service.practice, rel: 'supports exact bytes 🧭', rationale: 'Use the real adopted canonical endpoint.' };
    const edge = await service.command('edge.link', args, args.from_id, 'edge-' + randomUUID());
    const response = await service.post(edge); assert.equal(response.status, 202, JSON.stringify(response));
    assert.equal(response.body.data.state, 'recorded'); assert.equal(response.body.data.operation, 'dna.knowledge.edge.link');
    const graph = await service.read(service.apiPath + '/dna/knowledge/edges?' + new URLSearchParams({ id: args.from_id }));
    assert.equal(graph.status, 200, JSON.stringify(graph));
    const stored = graph.json.data.items.find(row => row.id === response.body.data.edge_id);
    assert(stored, 'native relationship must be observed in fresh graph');
    assert.equal(stored.from_id, args.from_id); assert.equal(stored.to_id, args.to_id); assert.equal(stored.rel, args.rel);
    await save('edge-receipt.json', response.body); await save('edge-read.json', graph.json);
    return { edge_id: stored.id, from_id: stored.from_id, to_id: stored.to_id };
  });
  await check('node and edge operations share conflict-safe request identity', async () => {
    await service.quiesce();
    const changed = await service.command('edge.link', { from_id: created.node.candidate_digest, to_id: service.practice, rel: 'different operation', rationale: 'Must conflict with original node proposal.' }, created.node.candidate_digest, command.request_id);
    const before = service.journal().head; const response = await service.post(changed);
    assert.equal(response.status, 409, JSON.stringify(response)); assert.equal(response.body.error.code, 'request_conflict');
    assert.equal(service.journal().head, before); assert.equal(admissions(command.request_id).length, 1);
    const found = await service.lookup(command.request_id); assert.equal(found.status, 200, JSON.stringify(found));
    assert.equal(found.body.data.operation, 'dna.knowledge.node.propose'); assert.equal(found.body.data.command_id, adopted.command_id);
    return { request_id: command.request_id, code: response.body.error.code };
  });
} catch (error) {
  failure = error; console.error(error.stack || error);
} finally {
  clearTimeout(timer);
  if (service) {
    try { await service.stop(); assert.deepEqual(service.processes(), []); }
    catch (error) { failure ||= error; }
  }
  const result = { ok: !failure, elapsed_ms: Date.now() - started, passed: results.filter(value => value.ok).length, total: 7, results, evidence, native_evidence: service?.evidence, remaining_processes: service?.processes() || [], error: failure?.stack || null };
  await save('result.json', result); console.log(JSON.stringify(result, null, 2));
  if (failure) process.exitCode = 1;
}
