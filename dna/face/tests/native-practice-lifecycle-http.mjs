// Compact real Practice lifecycle over the existing native Knowledge provider.
// No command/outcome interception, seeded domain outcomes, pagination or builds.
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { startBindingService, bindingEnvironmentPresent } from './native-knowledge-binding-harness.mjs';

assert(bindingEnvironmentPresent(), 'Supply matching API, Body, relay and Knowledge service binaries.');
const parent = process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
assert(path.isAbsolute(parent)); await mkdir(parent, { recursive: true });
const evidence = await mkdtemp(path.join(parent, 'native-practice-lifecycle-'));
const results = [], started = Date.now(); let service, failure;
const save = (name, data) => writeFile(path.join(evidence, name), JSON.stringify(data, null, 2) + '\n');
const grant = { mode: 'local', name: 'alice', authority: 'board', edge_link: 'direct', edge_unlink: 'direct',
  node_propose: 'review', node_revise: 'review', node_retire: 'review',
  node_scopes: [{ author: 'org', target: 'org/elsewhere' }],
  binding_bind: 'review', binding_unbind: 'review', binding_scopes: [{ author: 'org', target: 'org/support' }], recover: true };
const name = 'practice/receipt-compass';
const originalText = 'Collect the exact receipt — café 東京 🧭.\r\nKeep <literal> provenance.';
const revisedText = 'Collect and verify the exact receipt.\r\nRetain the original source.';
async function check(name, fn) {
  const at = Date.now();
  try { const detail = await fn(); results.push({ name, ok: true, elapsed_ms: Date.now() - at, ...detail }); console.log(JSON.stringify({ name, ok: true })); }
  catch (error) { results.push({ name, ok: false, error: error.stack }); throw error; }
}
async function practice(id, expectedState, expectedKind, text, command, receipt) {
  const result = await service.read(service.apiPath + '/dna/practices?' + new URLSearchParams({ id }));
  assert.equal(result.status, 200, JSON.stringify(result)); const row = result.json.data.items[0];
  assert.equal(row.id, id); assert.equal(row.kind, expectedKind); assert.equal(row.state, expectedState);
  assert.equal(row.text_available, true); assert.equal(row.text, text); assert.equal(row.name, name);
  assert.equal(row.requester, 'alice'); assert.equal(row.rationale, command.arguments.rationale);
  assert.equal(row.request_id, receipt.command_id, 'catalog retains exact scoped native request identity');
  await save(`practice-${expectedKind}-${expectedState}-${id.slice(7, 19)}.json`, result.json);
  return row;
}
async function graph(id, target = '') {
  const query = new URLSearchParams({ limit: '25' }); if (target) query.set('target', target); else query.set('id', id);
  const result = await service.request(service.apiPath + '/dna/knowledge/nodes?' + query);
  assert.equal(result.status, 200, JSON.stringify(result)); assert.equal(result.body.data.page.next_cursor, null, 'fixture fits one native page');
  return result.body.data.items.filter(row => row.id === id);
}
async function propose(operation, text, old = '') {
  await service.quiesce();
  const args = operation === 'node.retire' ? { id: old, rationale: text } : {
    kind: 'practice', name, text, author: 'org', target: 'org/elsewhere', rationale: operation === 'node.propose' ? 'Create a scoped Practice with exact human provenance.' : 'Revise this exact predecessor; retain its history.',
    ...(old ? { supersedes: old } : {}),
  };
  const command = await service.command(operation, args, old || 'org/elsewhere');
  const response = await service.post(command); assert.equal(response.status, 202, JSON.stringify(response));
  const receipt = await service.waitNode(command.request_id, value => value.node.proposal_state === 'created');
  await service.quiesce(); await save(`${operation}-command.json`, command); await save(`${operation}-created.json`, receipt);
  return { command, receipt, id: receipt.node.candidate_digest, review: receipt.node.review_id };
}
async function approve(proposal) {
  await service.decide(proposal.id, proposal.review);
  const settled = await service.waitNode(proposal.command.request_id, value => value.node.activation_state === 'adopted');
  await service.quiesce(); await save(`${proposal.command.operation}-adopted.json`, settled); return settled;
}
const timer = setTimeout(async () => { console.error('Compact Practice lifecycle exceeded 90 seconds.'); try { await service?.stop(); } finally { process.exit(124); } }, 90_000);
try {
  service = await startBindingService({ grants: [grant], evidenceParent: evidence, rootPrefix: '/tmp/hale-face-browser.practice-lifecycle.' });
  let initial, revision, retirement, extra;
  await check('create kind Practice through native node admission and independent exact Review', async () => {
    initial = await propose('node.propose', originalText);
    await practice(initial.id, 'pending', 'practice', originalText, initial.command, initial.receipt);
    assert.equal(service.candidate(initial.id).kind, 'practice');
    assert.equal((await graph(initial.id))[0].accepted, false);
    await approve(initial);
    await practice(initial.id, 'ratified', 'practice', originalText, initial.command, initial.receipt);
    const row = (await graph(initial.id))[0]; assert.equal(row.kind, 'practice'); assert.equal(row.accepted, true); assert.equal(row.text, originalText);
    assert.equal((await graph(initial.id, 'org/support/urgent')).length, 0, 'original binding is outside the support branch');
    return { id: initial.id, review: initial.review };
  });
  await check('review one extra applicability tuple and observe inherited scope', async () => {
    extra = await service.applyBinding(initial.id, 'org/support');
    const rows = await service.bindings(initial.id); assert.equal(rows.length, 2);
    assert(rows.some(row => row.target === 'org/elsewhere')); assert(rows.some(row => row.id === extra.receipt.binding.binding_id && row.target === 'org/support'));
    assert.equal((await graph(initial.id, 'org/support/urgent'))[0].id, initial.id);
    await save('initial-practice-bindings.json', rows);
    return { binding_id: extra.receipt.binding.binding_id, native_effect: extra.receipt.binding.effect_state };
  });
  await check('revise exact Practice predecessor without silently copying extra applicability', async () => {
    revision = await propose('node.revise', revisedText, initial.id); assert.equal(service.candidate(revision.id).supersedes, initial.id);
    await approve(revision);
    await practice(initial.id, 'retired', 'practice', originalText, initial.command, initial.receipt);
    await practice(revision.id, 'ratified', 'practice', revisedText, revision.command, revision.receipt);
    assert.equal((await graph(initial.id))[0].accepted, false); assert.equal((await graph(revision.id))[0].accepted, true);
    assert.equal((await graph(initial.id, 'org/support/urgent')).length, 0);
    assert.equal((await graph(revision.id, 'org/support/urgent')).length, 0, 'extra predecessor binding does not move to successor');
    const oldBindings = await service.bindings(initial.id), newBindings = await service.bindings(revision.id);
    assert.equal(oldBindings.length, 2, 'retired predecessor retains exact historical applicability tuples');
    assert.deepEqual(newBindings.map(row => row.target), ['org/elsewhere']);
    assert.equal(service.candidate(initial.id).text, originalText); await save('revision-bindings.json', { predecessor: oldBindings, successor: newBindings });
    return { predecessor: initial.id, successor: revision.id };
  });
  await check('review retirement of successor while retaining exact Practice and retirement history', async () => {
    const rationale = 'Retire this Practice after the supported process changed.';
    retirement = await propose('node.retire', rationale, revision.id);
    const candidate = service.candidate(retirement.id); assert.equal(candidate.kind, 'retirement'); assert.equal(candidate.name, name); assert.equal(candidate.supersedes, revision.id); assert.equal(candidate.text, rationale);
    assert.equal((await graph(revision.id))[0].accepted, true, 'proposal alone cannot retire a Practice');
    await approve(retirement);
    await practice(revision.id, 'retired', 'practice', revisedText, revision.command, revision.receipt);
    await practice(retirement.id, 'ratified', 'retirement', rationale, retirement.command, retirement.receipt);
    assert.equal((await graph(revision.id))[0].accepted, false);
    assert.equal((await graph(revision.id, 'org/elsewhere')).length, 0);
    assert.equal((await graph(retirement.id))[0].accepted, false, 'ratified retirement document is not a new accepted graph Idea');
    assert.equal(service.candidate(initial.id).text, originalText); assert.equal(service.candidate(revision.id).text, revisedText);
    const native = service.facts('knowledge.retired', revision.id); assert.equal(native.length, 1); assert.equal(native[0].data.by, retirement.id);
    const catalog = await service.read(service.apiPath + '/dna/practices?limit=25'); assert.equal(catalog.status, 200);
    const sameName = catalog.json.data.items.filter(row => row.name === name); assert.equal(sameName.length, 3);
    assert.equal(sameName.filter(row => row.kind === 'practice' && row.state === 'ratified').length, 0);
    assert(service.journal().rows.length < 100, 'deliberately small lifecycle; no sustained/multipage fixture');
    await save('final-catalog.json', catalog.json); return { retirement: retirement.id, retired: revision.id, native_rows: service.journal().rows.length };
  });
} catch (error) { failure = error; console.error(error.stack || error); }
finally {
  clearTimeout(timer);
  if (service) try { await service.stop(); assert.deepEqual(service.processes(), []); } catch (error) { failure ||= error; }
  const result = { ok: !failure, elapsed_ms: Date.now() - started, results, evidence, native_evidence: service?.evidence, remaining_processes: service?.processes() || [], error: failure?.stack || null,
    scope: 'Four compact real HTTP/Body/Review/graph cases; no browser interaction or long-running Knowledge-service stability claim.' };
  await save('result.json', result); console.log(JSON.stringify(result, null, 2)); if (failure) process.exitCode = 1;
}
