// Real reviewed relationship composition; no authored command outcomes.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { startNodeService, nodeEnvironmentPresent } from './native-knowledge-node-harness.mjs';
export { nodeEnvironmentPresent as edgeReviewEnvironmentPresent };
export const edgeReviewGrant = { mode: 'local', name: 'alice', authority: 'board', edge_link: 'review', edge_unlink: 'review', node_propose: 'review', node_revise: 'review', node_retire: 'review', node_scopes: [{ author: 'org', target: 'org' }], recover: true };

export async function startEdgeReviewService(options = {}) {
  const service = await startNodeService({ ...options, grants: options.grants || [edgeReviewGrant] });
  async function asActor(actor) { await service.stopAPI(); await service.startAPI(actor); }
  try {
    const policy = JSON.parse(await readFile(service.policy, 'utf8'));
    policy.grants.push({ mode: 'local', name: 'bob', authority: 'board', practice_propose: false, review_verdict: true, recover: true });
    await writeFile(service.policy, JSON.stringify(policy)); await asActor('alice');
  } catch (error) { await service.stop(); throw error; }
  async function waitRelationship(id, predicate) {
    const deadline = Date.now() + 20_000; let last;
    while (Date.now() < deadline) {
      last = await service.lookup(id);
      if (last.status === 200 && predicate(last.body.data)) return last.body.data;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Native relationship outcome timed out: ' + JSON.stringify(last));
  }
  async function decide(candidate, reviewId, verdict = 'approve') {
    await service.quiesce(); await asActor('bob');
    const command = { request_id: randomUUID(), operation: 'dna.review.verdict', operation_version: '1', context: { application_id: service.application, position_id: 'org' }, target: { application_id: service.application, kind: 'dna.review', id: reviewId }, preconditions: { subject_digest: candidate, principal: { mode: 'local', name: 'bob' }, review_state: 'pending' }, arguments: { verdict, comment: 'Decide independently on the exact directed candidate.' } };
    const result = await service.request(service.apiPath + '/commands', { method: 'POST', headers: { Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(command) });
    assert.equal(result.status, 202, JSON.stringify(result)); await service.waitCommand(command.request_id, receipt => receipt.verdict.state === 'accepted'); await asActor('alice');
  }
  async function createItem() {
    const command = await service.command('node.propose', { kind: 'idea', name: 'Relationship endpoint', text: 'The second exact endpoint remains independent of its relationships.', author: 'org', target: 'org', rationale: 'Prepare a real second endpoint.' }, 'org');
    assert.equal((await service.post(command)).status, 202);
    const created = await service.waitNode(command.request_id, r => r.node.proposal_state === 'created'); await decide(created.node.candidate_digest, created.node.review_id);
    await service.waitNode(command.request_id, r => r.node.activation_state === 'adopted'); await service.quiesce(); return created.node.candidate_digest;
  }
  async function edges(idea = service.practice) {
    await service.quiesce();
    const rows = []; let cursor = '', snapshot = '';
    do {
      const query = new URLSearchParams({ id: idea, limit: '25' }); if (cursor) query.set('cursor', cursor); if (snapshot) query.set('snapshot', snapshot);
      const result = await service.request(service.apiPath + '/dna/knowledge/edges?' + query); assert.equal(result.status, 200, JSON.stringify(result));
      const data = result.body.data; if (snapshot) assert.equal(data.page.snapshot, snapshot); snapshot = data.page.snapshot; cursor = data.page.next_cursor; rows.push(...data.items);
    } while (cursor);
    return rows;
  }
  return { ...service, asActor, decide, createItem, waitRelationship, edges };
}
