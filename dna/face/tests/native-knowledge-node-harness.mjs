// The real host, organization and Review API on one Git Record, with
// Knowledge in memory. Every proposal, Review and activation is native.
//
// There is no Knowledge service (GH #985). The API admits Knowledge commands
// in-process under HALE_DNA_KNOWLEDGE_COMMAND_POLICY and reads the graph from
// memory under the head's role (HALE_DNA_MEMORY_DSN_HEAD) alone; the host
// (`hale dna dev`, the spine here) projects the Record into memory on its
// tick and hands the harness the head's DSN (GH #1029).
//
// Knowledge changes are the head's gated topics (GH #1129): each is one
// line of the api wire POSTed to …/commands under the launch token, and
// its receipt a KnowledgeReply (a refusal is its `code`). The acting person
// already holds the board and reviewer seats the base harness gives, which
// opens the `position` gate; the policy decides what they may change.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { startService, nativeCommandEnvironmentPresent } from './native-command-harness.mjs';
import { memoryOwner } from './environment.mjs';
import { wireLine, knowledgeLookupLine, settleKnowledge } from './command-wire.mjs';

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
  const commandPath = service.apiPath + '/commands';
  const commandHeaders = { Origin: service.origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' };
  // A mutation carries this launch's token, as a tool that read its file does.
  // One connection per request: the API is restarted on the same port
  // whenever the actor or the grants change, and a pooled keep-alive
  // socket to the old process would fail the next fetch. No retry: a read
  // that needs the projection at the Record head follows `quiesce()`, and a
  // dead head surfaces as its own connection error at once.
  async function request(path, init = {}) {
    const headers = { Connection: 'close', ...(init.method && init.method !== 'GET' ? { 'X-Hale-Token': service.token() } : {}), ...(init.headers || {}) };
    const response = await fetch(service.origin + path, { signal: AbortSignal.timeout(15_000), ...init, headers });
    return { status: response.status, body: await response.json() };
  }
  // One forwarded line, settled: the receipt in the old terms, or the code.
  async function forward(line) {
    const response = await request(commandPath, { method: 'POST', headers: commandHeaders, body: JSON.stringify(line) });
    return { ...settleKnowledge(response.status, response.body), line };
  }
  return {
    ...service, commandPath, request, forward,
    async setGrants(grants) { await writeFile(policyPath, JSON.stringify({ ...policy, grants })); await service.restart(); },
    // What the session may send: the calls its describe line lists.
    async slice() {
      const described = await request(commandPath, { method: 'POST', headers: commandHeaders, body: '{"describe":true}' });
      assert.equal(described.status, 200, JSON.stringify(described)); assert.equal(described.body.ok, true, JSON.stringify(described));
      return described.body.value.commands.map(entry => entry.name);
    },
    lookup: requestId => forward(knowledgeLookupLine(requestId)),
    async command(operation, arguments_, target, requestId = randomUUID()) {
      const source = await service.read(service.apiPath + '/capabilities');
      return { request_id: requestId, operation: 'dna.knowledge.' + operation, operation_version: '1', context: { application_id: service.application, position_id: 'org' }, target: { application_id: service.application, kind: operation === 'node.propose' ? 'dna.knowledge.collection' : 'dna.knowledge.node', id: target }, preconditions: { principal: service.principal, record_head: source.json.source.record_head }, arguments: arguments_ };
    },
    post: command => forward(wireLine(command)),
    async waitNode(requestId, predicate) {
      const deadline = Date.now() + 20_000; let last;
      while (Date.now() < deadline) {
        last = await forward(knowledgeLookupLine(requestId));
        if (last.code === '' && predicate(last.receipt)) return last.receipt;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      throw new Error('Native Knowledge outcome timed out: ' + JSON.stringify(last));
    },
  };
}
