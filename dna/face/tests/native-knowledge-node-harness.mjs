// The real host, organization and Review API on one Git Record, with
// Knowledge in memory. Every proposal, Review and activation is native.
//
// There is no Knowledge service (GH #985). The API admits Knowledge commands
// in-process under HALE_DNA_KNOWLEDGE_COMMAND_POLICY and reads the graph from
// memory under the head's role (HALE_DNA_MEMORY_DSN_HEAD) alone; the host
// (`hale dna dev`, the spine here) projects the Record into memory on its
// tick and hands the harness the head's DSN (GH #1029).
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { startService, nativeCommandEnvironmentPresent } from './native-command-harness.mjs';
import { memoryOwner } from './environment.mjs';

export const nodeEnvironmentPresent = () => nativeCommandEnvironmentPresent() && Boolean(memoryOwner());

export async function startNodeService(options = {}) {
  assert(memoryOwner(), 'Knowledge reads memory: set HALE_DNA_MEMORY_DSN_OWNER.');
  let policyPath, policy;
  const service = await startService({
    ...options,
    async startDependencies(context) {
      policyPath = context.evidence + '/knowledge-authority.json';
      const grant = { mode: 'local', name: context.principal, authority: 'board', edge_link: 'direct', edge_unlink: 'direct', node_propose: 'review', node_revise: 'review', node_retire: 'review', node_scopes: [{ author: 'org', target: 'org' }], recover: true };
      policy = { format: 'dna.knowledge-authority/1', application_id: context.application, grants: options.grants || [grant] };
      await writeFile(policyPath, JSON.stringify(policy));
      // The head's DSN is the host's to give: `startService` sets it on the API.
      return { apiEnv: { HALE_DNA_KNOWLEDGE_COMMAND_POLICY: policyPath } };
    },
  });
  const commandPath = service.apiPath + '/dna/knowledge/commands';
  // One connection per request: the API is restarted on the same port
  // whenever the actor or the grants change, and a pooled keep-alive
  // socket to the old process would fail the next fetch. No retry: a read
  // that needs the projection at the Record head follows `quiesce()`, and a
  // dead head surfaces as its own connection error at once.
  async function request(path, init = {}) {
    const response = await fetch(service.origin + path, { signal: AbortSignal.timeout(15_000), ...init, headers: { Connection: 'close', ...(init.headers || {}) } });
    return { status: response.status, body: await response.json() };
  }
  return {
    ...service, commandPath, request,
    async setGrants(grants) { await writeFile(policyPath, JSON.stringify({ ...policy, grants })); await service.restart(); },
    capability: operation => request(commandPath + '/capability?' + new URLSearchParams({ operation })),
    lookup: requestId => request(commandPath + '?' + new URLSearchParams({ request_id: requestId })),
    async command(operation, arguments_, target, requestId = randomUUID()) {
      const source = await service.read(service.apiPath + '/capabilities');
      return { request_id: requestId, operation: 'dna.knowledge.' + operation, operation_version: '1', context: { application_id: service.application, position_id: 'org' }, target: { application_id: service.application, kind: operation === 'node.propose' ? 'dna.knowledge.collection' : 'dna.knowledge.node', id: target }, preconditions: { principal: service.principal, record_head: source.json.source.record_head }, arguments: arguments_ };
    },
    post: command => request(commandPath, { method: 'POST', headers: { Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(command) }),
    async waitNode(requestId, predicate) {
      const deadline = Date.now() + 20_000; let last;
      while (Date.now() < deadline) {
        last = await request(commandPath + '?' + new URLSearchParams({ request_id: requestId }));
        if (last.status === 200 && predicate(last.body.data)) return last.body.data;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      throw new Error('Native Knowledge outcome timed out: ' + JSON.stringify(last));
    },
  };
}
