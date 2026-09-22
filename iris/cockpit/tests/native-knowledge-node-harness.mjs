// Real Body/relay/membrane + Review API + Knowledge service on one Git Record.
// Every proposal, Review, activation and graph projection is native.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import net from 'node:net';
import { startService, nativeCommandEnvironmentPresent } from './native-command-harness.mjs';

export const nodeEnvironmentPresent = () => nativeCommandEnvironmentPresent() && process.env.HALE_KNOWLEDGE_SERVICE_BIN?.startsWith('/');

export async function startNodeService(options = {}) {
  const binary = options.knowledge || process.env.HALE_KNOWLEDGE_SERVICE_BIN;
  assert(binary?.startsWith('/'), 'Supply absolute HALE_KNOWLEDGE_SERVICE_BIN.');
  let native, policyPath, policy, startKnowledge, privateOrigin;
  const service = await startService({
    ...options,
    async startDependencies(context) {
      const server = net.createServer();
      await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
      const port = server.address().port; await new Promise(resolve => server.close(resolve)); privateOrigin = `http://127.0.0.1:${port}`;
      policyPath = context.evidence + '/knowledge-authority.json';
      const grant = { mode: 'local', name: context.principal, authority: 'board', edge_link: 'direct', edge_unlink: 'direct', node_propose: 'review', node_revise: 'review', node_retire: 'review', node_scopes: [{ author: 'org', target: 'org' }], recover: true };
      policy = { format: 'dna.knowledge-authority/1', application_id: context.application, grants: options.grants || [grant] };
      await writeFile(policyPath, JSON.stringify(policy));
      await writeFile(context.evidence + '/knowledge-service.json', JSON.stringify({ binary, sha256: createHash('sha256').update(await readFile(binary)).digest('hex'), application: context.application, origin: privateOrigin, policy: policyPath }, null, 2));
      const readKey = 'native-node-read-' + randomUUID(), commandKey = 'native-node-command-' + randomUUID();
      startKnowledge = async () => {
        native = context.launch('knowledge', binary, [context.root, String(port)], { HALE_DNA_KNOWLEDGE_DSN: 'memory', HALE_DNA_KNOWLEDGE_READ_KEY: readKey, HALE_DNA_KNOWLEDGE_COMMAND_KEY: commandKey, HALE_DNA_KNOWLEDGE_COMMAND_POLICY: policyPath });
        await context.wait('Knowledge native startup', async () => {
          try { const response = await fetch(privateOrigin + '/identity', { signal: AbortSignal.timeout(500) }); return response.ok && (await response.json()).identity === context.application; }
          catch { return false; }
        }, Boolean);
      };
      await startKnowledge();
      return { apiEnv: { HALE_DNA_KNOWLEDGE_URL: privateOrigin, HALE_DNA_KNOWLEDGE_READ_KEY: readKey, HALE_DNA_KNOWLEDGE_COMMAND_KEY: commandKey }, restart: async () => { await context.stopProcess(native); await startKnowledge(); } };
    },
  });
  const commandPath = service.apiPath + '/dna/knowledge/commands';
  async function request(path, init = {}) {
    const response = await fetch(service.origin + path, { signal: AbortSignal.timeout(15_000), ...init });
    return { status: response.status, body: await response.json() };
  }
  return {
    ...service, privateOrigin, commandPath, request,
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
