// Binding acceptance uses the real same-Record Body, Review API and graph service.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { startNodeService, nodeEnvironmentPresent } from './native-knowledge-node-harness.mjs';

export { nodeEnvironmentPresent as bindingEnvironmentPresent };
export const bindingTargets = ['org/support', 'org/support/urgent', 'org/elsewhere', ...Array.from({ length: 27 }, (_, i) => 'org/page/' + String(i).padStart(2, '0'))];
export const bindingGrant = {
  mode: 'local', name: 'alice', authority: 'board', edge_link: 'direct', edge_unlink: 'direct',
  node_propose: 'review', node_revise: 'review', node_retire: 'review',
  node_scopes: [{ author: 'org', target: 'org' }, { author: 'org', target: 'org/elsewhere' }],
  binding_bind: 'review', binding_unbind: 'review', binding_scopes: bindingTargets.map(target => ({ author: 'org', target })), recover: true,
};

export async function startBindingService(options = {}) {
  const service = await startNodeService({ ...options, grants: options.grants || [bindingGrant] });
  async function asActor(actor) { await service.stopAPI(); await service.startAPI(actor); }
  // The actual proposer cannot independently decide the binding Review.
  // Grant a second real API principal; changing USER alone grants nothing.
  try {
    const policy = JSON.parse(await readFile(service.policy, 'utf8'));
    policy.grants.push({ mode: 'local', name: 'bob', authority: 'board', practice_propose: false, review_verdict: true, recover: true });
    await writeFile(service.policy, JSON.stringify(policy)); await asActor('alice');
  } catch (error) { await service.stop(); throw error; }
  async function waitBinding(requestId, predicate) {
    const deadline = Date.now() + 20_000; let last;
    while (Date.now() < deadline) {
      last = await service.lookup(requestId);
      if (last.status === 200 && predicate(last.body.data)) return last.body.data;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Native binding outcome timed out: ' + JSON.stringify(last));
  }
  async function decide(candidate, reviewId, verdict = 'approve') {
    await service.quiesce(); await asActor('bob');
    const command = { request_id: randomUUID(), operation: 'dna.review.verdict', operation_version: '1', context: { application_id: service.application, position_id: 'org' }, target: { application_id: service.application, kind: 'dna.review', id: reviewId }, preconditions: { subject_digest: candidate, principal: { mode: 'local', name: 'bob' }, review_state: 'pending' }, arguments: { verdict, comment: 'Decide independently on the exact native fixture candidate.' } };
    const response = await service.request(service.apiPath + '/commands', { method: 'POST', headers: { Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(command) });
    assert.equal(response.status, 202, JSON.stringify(response)); await service.waitCommand(command.request_id, value => value.verdict.state === 'accepted'); await asActor('alice');
    return command;
  }
  async function createItem() {
    await service.quiesce();
    const command = await service.command('node.propose', { kind: 'idea', name: 'Binding anchor', text: 'Applicability changes preserve this exact historical idea.', author: 'org', target: 'org/elsewhere', rationale: 'Start outside the support branch.' }, 'org/elsewhere');
    assert.equal((await service.post(command)).status, 202);
    const created = await service.waitNode(command.request_id, value => value.node.proposal_state === 'created');
    await decide(created.node.candidate_digest, created.node.review_id);
    await service.waitNode(command.request_id, value => value.node.activation_state === 'adopted'); await service.quiesce();
    return created.node.candidate_digest;
  }
  async function applyBinding(idea, target, operation = 'binding.bind', bindingId = '') {
    await service.quiesce();
    const args = { idea_id: idea, author: 'org', target, rationale: 'Prepare real applicability for the browser scenario.' };
    if (operation === 'binding.unbind') args.binding_id = bindingId;
    const command = await service.command(operation, args, idea), response = await service.post(command);
    assert.equal(response.status, 202, JSON.stringify(response));
    const created = await waitBinding(command.request_id, value => value.binding.proposal_state === 'created');
    await decide(created.binding.candidate_digest, created.binding.review_id);
    const settled = await waitBinding(command.request_id, value => value.binding.effect_state === (operation === 'binding.bind' ? 'bound' : 'unbound')); await service.quiesce();
    return { command, receipt: settled };
  }
  async function bindings(idea, target = '') {
    const items = []; let cursor = '', snapshot = '';
    do {
      const query = new URLSearchParams({ id: idea, limit: '25' }); if (target) query.set('target', target); if (cursor) query.set('cursor', cursor); if (snapshot) query.set('snapshot', snapshot);
      const response = await service.request(service.apiPath + '/dna/knowledge/bindings?' + query); assert.equal(response.status, 200, JSON.stringify(response));
      const data = response.body.data; if (snapshot) assert.equal(data.page.snapshot, snapshot); snapshot = data.page.snapshot; items.push(...data.items); cursor = data.page.next_cursor;
    } while (cursor);
    return items;
  }
  return { ...service, asActor, waitBinding, decide, createItem, applyBinding, bindings };
}
