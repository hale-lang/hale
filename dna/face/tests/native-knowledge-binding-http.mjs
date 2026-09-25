// Real native HTTP acceptance. All candidate, Review and binding outcomes are
// authored by the native provider/Body; Git is inspected only for assertions.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { startBindingService, bindingEnvironmentPresent } from './native-knowledge-binding-harness.mjs';

assert(bindingEnvironmentPresent(), 'Supply matching native API, Body, relay and Knowledge service binaries.');
const parent = process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
assert(path.isAbsolute(parent)); await mkdir(parent, { recursive: true });
const evidence = await mkdtemp(path.join(parent, 'native-binding-http-'));
const started = Date.now(), results = [];
let service, failure, command, created, bound;
const bytes = value => Buffer.byteLength(value, 'utf8');
const digest = value => 'sha256:' + createHash('sha256').update(value, 'utf8').digest('hex');
const save = (name, value) => writeFile(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n');
async function check(name, run) {
  const began = Date.now();
  try { const details = await run(); results.push({ name, ok: true, elapsed_ms: Date.now() - began, ...details }); console.log(JSON.stringify({ case: name, ok: true })); }
  catch (error) { results.push({ name, ok: false, elapsed_ms: Date.now() - began, error: error.stack || String(error) }); throw error; }
}
const args = (target, rationale = 'Change exact applicability.') => ({ idea_id: service.practice, author: 'org', target, rationale });
const admissions = key => service.facts('knowledge.binding.requested').filter(row => { try { return JSON.parse(row.data.command_payload).request_id === key; } catch { return false; } });
const headers = () => ({ Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' });
async function asActor(actor) { await service.stopAPI(); await service.startAPI(actor); }
async function read(path) { const response = await service.read(path); return { status: response.status, body: response.json }; }
async function refusal(value, status, code) {
  const before = service.journal().head, response = await service.post(value);
  assert.equal(response.status, status, JSON.stringify(response)); assert.equal(response.body.error?.code, code, JSON.stringify(response));
  assert.equal(service.journal().head, before); assert.equal(admissions(value.request_id).length, 0);
  return { request_id: value.request_id, status, code };
}
async function candidate(receipt) {
  await service.quiesce();
  const source = await read(service.apiPath + '/capabilities'); assert.equal(source.status, 200);
  const head = source.body.source.record_head;
  const response = await read(service.apiPath + '/dna/reviews/candidate?' + new URLSearchParams({ id: receipt.binding.review_id, snapshot: head }));
  assert.equal(response.status, 200, JSON.stringify(response));
  const data = response.body.data; assert.equal(data.kind, 'knowledge_binding_change'); assert.equal(data.review_id, receipt.binding.review_id);
  assert.equal(data.candidate_digest, receipt.binding.candidate_digest); assert.equal(digest(data.document), data.candidate_digest);
  const canonical = JSON.parse(data.document); assert.deepEqual(canonical, service.candidate(data.candidate_digest));
  const review = await read(service.apiPath + '/dna/reviews?' + new URLSearchParams({ id: data.review_id, snapshot: head }));
  assert.equal(review.status, 200, JSON.stringify(review)); const row = review.body.data.items[0];
  assert.equal(row.subject_digest, data.candidate_digest); assert.equal(row.knowledge_binding_digest, data.candidate_digest); assert.equal(row.knowledge_digest, '');
  assert.equal(row.author, canonical.by); assert.equal(row.target, canonical.target); assert.equal(row.binding_class, canonical.class);
  assert.equal(row.text_available, true); assert.equal(row.is_mutation, false); assert.equal(row.approvers, ''); assert.equal(row.required_authority, 'board');
  return { response: response.body, document: canonical, review: row };
}
function verdict(receipt, actor) {
  return { request_id: randomUUID(), operation: 'dna.review.verdict', operation_version: '1', context: { application_id: service.application, position_id: 'org' }, target: { application_id: service.application, kind: 'dna.review', id: receipt.binding.review_id }, preconditions: { subject_digest: receipt.binding.candidate_digest, principal: { mode: 'local', name: actor }, review_state: 'pending' }, arguments: { verdict: 'approve', comment: 'Independent exact tuple review — café 🧭\r\n\u0001.' } };
}
async function decide(receipt) {
  await service.quiesce(); await asActor('bob');
  const command = verdict(receipt, 'bob');
  const response = await service.request(service.apiPath + '/commands', { method: 'POST', headers: headers(), body: JSON.stringify(command) });
  assert.equal(response.status, 202, JSON.stringify(response));
  const decided = await service.waitCommand(command.request_id, value => value.verdict.state === 'accepted');
  assert.equal(decided.activation.state, 'unknown', 'Review receipt must not claim node adoption for a binding change');
  await service.quiesce(); await asActor('alice'); return decided;
}
const timer = setTimeout(async () => { console.error('Binding HTTP acceptance exceeded 120 seconds.'); try { await service?.stop(); } finally { process.exit(124); } }, 120_000);
try {
  service = await startBindingService({ evidenceParent: evidence, rootPrefix: '/tmp/hale-face-browser.binding-http.' });
  const policy = JSON.parse(await readFile(service.policy, 'utf8'));
  if (!policy.grants.some(grant => grant.name === 'bob')) policy.grants.push({ mode: 'local', name: 'bob', authority: 'board', practice_propose: false, review_verdict: true, recover: true });
  await writeFile(service.policy, JSON.stringify(policy));
  const original = service.candidate(service.practice);
  const prefix = 'Café 東京 🧭\r\n'; const rationale = prefix + '\u0001'.repeat(2048 - bytes(prefix)); assert.equal(bytes(rationale), 2048);

  await check('binding profiles advertise reviewed tuple operations and exact limits', async () => {
    for (const operation of ['dna.knowledge.binding.bind', 'dna.knowledge.binding.unbind']) {
      const response = await service.capability(operation); assert.equal(response.status, 200, JSON.stringify(response));
      assert.equal(response.body.data.profile, operation + '.v1'); assert.equal(response.body.data.mode, 'review');
      assert.equal(response.body.data.available, true); assert.equal(response.body.data.authorized, true);
      assert.equal(response.body.data.max_locus_bytes, '256'); assert.equal(response.body.data.max_rationale_bytes, '2048'); assert.equal(response.body.data.max_request_bytes, '32768');
    }
    const capabilities = await read(service.apiPath + '/capabilities'); assert.equal(capabilities.body.data.read_only, false);
  });
  await check('ungranted tuple and 2049-byte rationale refuse without admission', async () => {
    await service.quiesce();
    const denied = await service.command('binding.bind', args('org/restricted'), service.practice);
    const oversized = await service.command('binding.bind', args('org/support', rationale + 'x'), service.practice);
    return { refusals: [await refusal(denied, 403, 'forbidden'), await refusal(oversized, 400, 'invalid_command')] };
  });
  await check('2048 escaped UTF8 rationale creates an exact canonical binding Review', async () => {
    command = await service.command('binding.bind', args('org/support', rationale), service.practice);
    assert(bytes(JSON.stringify(command)) < 32768);
    const response = await service.post(command); assert.equal(response.status, 202, JSON.stringify(response));
    created = await service.waitBinding(command.request_id, value => value.binding.proposal_state === 'created');
    const exact = await candidate(created); assert.equal(exact.document.because, rationale); assert.equal(exact.document.idea_id, service.practice);
    assert.equal(exact.document.by, 'alice'); assert.equal(exact.document.author, 'org'); assert.equal(exact.document.target, 'org/support');
    assert.equal(exact.review.state, 'pending'); assert.equal(admissions(command.request_id).length, 1);
    const self = await service.request(service.apiPath + '/commands', { method: 'POST', headers: headers(), body: JSON.stringify(verdict(created, 'alice')) });
    assert.equal(self.status, 403, JSON.stringify(self)); assert.equal(self.body.error.code, 'forbidden');
    await save('candidate-read.json', exact.response); await save('created-receipt.json', created);
    return { request_id: command.request_id, candidate: created.binding.candidate_digest, rationale_bytes: bytes(rationale), rationale_digest: digest(rationale), encoded_request_bytes: bytes(JSON.stringify(command)) };
  });
  await check('independent exact Review approval binds and preserves historical candidate', async () => {
    await decide(created); bound = await service.waitBinding(command.request_id, value => value.binding.effect_state === 'bound'); await service.quiesce();
    const exact = await candidate(bound); assert.equal(exact.review.state, 'settled'); assert.equal(exact.document.because, rationale);
    const rows = await service.bindings(service.practice, 'org/support'); const row = rows.find(value => value.id === bound.binding.binding_id);
    assert(row, 'real graph must observe the exact approved tuple'); assert.equal(row.idea_id, service.practice); assert.equal(row.author, 'org'); assert.equal(row.target, 'org/support');
    assert.deepEqual(service.candidate(service.practice), original);
    await save('bound-receipt.json', bound); await save('bindings-after-bind.json', rows);
    return { binding_id: row.id, candidate: bound.binding.candidate_digest };
  });
  await check('same key recovers and cross-operation reuse conflicts', async () => {
    await service.quiesce(); const before = service.journal().head;
    const retry = await service.post(command); assert.equal(retry.status, 202, JSON.stringify(retry)); assert.equal(retry.body.data.command_id, bound.command_id); assert.equal(service.journal().head, before);
    const changed = await service.command('binding.unbind', { ...args('org/support'), binding_id: bound.binding.binding_id }, service.practice, command.request_id);
    const conflict = await service.post(changed); assert.equal(conflict.status, 409, JSON.stringify(conflict)); assert.equal(conflict.body.error.code, 'request_conflict');
    assert.equal(service.journal().head, before); assert.equal(admissions(command.request_id).length, 1);
  });
  await check('independent approved unbind removes only its exact tuple', async () => {
    const remove = await service.command('binding.unbind', { ...args('org/support'), binding_id: bound.binding.binding_id }, service.practice);
    const response = await service.post(remove); assert.equal(response.status, 202, JSON.stringify(response));
    const proposed = await service.waitBinding(remove.request_id, value => value.binding.proposal_state === 'created'); await candidate(proposed); await decide(proposed);
    const removed = await service.waitBinding(remove.request_id, value => value.binding.effect_state === 'unbound'); await service.quiesce();
    const rows = await service.bindings(service.practice); assert(!rows.some(row => row.id === bound.binding.binding_id));
    assert(rows.some(row => row.target === 'org' && row.idea_id === service.practice), 'original native binding remains');
    assert.deepEqual(service.candidate(service.practice), original); await save('unbound-receipt.json', removed); await save('bindings-after-unbind.json', rows);
    return { binding_id: removed.binding.binding_id, request_id: remove.request_id };
  });
  await check('stale Record precondition refuses without replacement request', async () => {
    await service.pauseDelivery();
    const stale = await service.command('binding.bind', args('org/support/urgent'), service.practice);
    const advance = await service.command('binding.bind', args('org/elsewhere'), service.practice);
    const response = await service.post(advance); assert.equal(response.status, 202, JSON.stringify(response));
    const result = await refusal(stale, 409, 'stale_subject'); service.resumeDelivery(); return result;
  });
} catch (error) { failure = error; console.error(error.stack || error); }
finally {
  clearTimeout(timer);
  if (service) { try { await service.stop(); assert.deepEqual(service.processes(), []); } catch (error) { failure ||= error; } }
  const result = { ok: !failure, elapsed_ms: Date.now() - started, passed: results.filter(value => value.ok).length, total: 7, results, evidence, native_evidence: service?.evidence, remaining_processes: service?.processes() || [], error: failure?.stack || null, protection_coverage: 'Protected candidate404 and Review scope masking are covered by real Git/Body binding_review_api_test, not repeated by this HTTP harness.' };
  await save('result.json', result); console.log(JSON.stringify(result, null, 2)); if (failure) process.exitCode = 1;
}
