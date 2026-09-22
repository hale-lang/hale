// Real Knowledge command service + public API. The seed binary only creates
// prior native subjects; command admission, projection and recovery are real.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import net from 'node:net';
import { boundedNative, isolatedEnvironment } from './environment.mjs';

const webroot = fileURLToPath(new URL('../web', import.meta.url));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function port() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const value = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return value;
}
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode) return;
  const done = new Promise(resolve => child.once('close', resolve));
  try { process.kill(-child.pid, 'SIGTERM'); } catch { child.kill('SIGTERM'); }
  const timer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch { child.kill('SIGKILL'); } }, 1_000);
  await done; clearTimeout(timer);
}

export async function startKnowledgeService(options = {}) {
  const api = options.api || process.env.HALE_KNOWLEDGE_API_BIN;
  const native = options.service || process.env.HALE_KNOWLEDGE_SERVICE_BIN;
  const seed = options.seed || process.env.HALE_KNOWLEDGE_SEED_BIN;
  if (![api, native, seed].every(value => value?.startsWith('/'))) throw new Error('Supply absolute Knowledge API, service and seed binary paths.');
  const root = await mkdtemp('/tmp/hale-face-browser.knowledge-command.');
  const env = isolatedEnvironment();
  const actor = options.actor || 'alice';
  env.USER = actor;
  env.XDG_CACHE_HOME = resolve(root, '.hale/face-cache');
  const seeded = boundedNative(seed, [root, 'seed', String(options.count ?? 3)]);
  const result = spawnSync(seeded.command, seeded.args, { env, encoding: 'utf8', timeout: 40_000, maxBuffer: 2_097_152 });
  if (result.status !== 0) throw new Error(`Knowledge seed failed: ${result.stderr || result.error || result.stdout}`);
  const refs = JSON.parse(await readFile(resolve(root, 'fixture.json'), 'utf8'));
  const apiPort = options.port || await port();
  const nativePort = await port();
  const origin = `http://127.0.0.1:${apiPort}`;
  const privateOrigin = `http://127.0.0.1:${nativePort}`;
  const apiPath = `/api/hale/v1/applications/${refs.application}`;
  const policyPath = resolve(root, 'knowledge-authority.json');
  env.HALE_DNA_KNOWLEDGE_DSN = 'memory';
  env.HALE_DNA_KNOWLEDGE_URL = privateOrigin;
  env.HALE_DNA_KNOWLEDGE_READ_KEY = 'face-native-knowledge-read-fixture-key';
  env.HALE_DNA_KNOWLEDGE_COMMAND_KEY = 'face-native-knowledge-write-fixture-key';
  env.HALE_DNA_KNOWLEDGE_COMMAND_POLICY = policyPath;
  const policy = {
    format: 'dna.knowledge-authority/1', application_id: refs.application,
    grants: options.grants || [{ mode: 'local', name: actor, authority: 'knowledge-editor', edge_link: 'direct', edge_unlink: 'direct', recover: true }],
  };
  await writeFile(policyPath, JSON.stringify(policy));
  let apiChild, nativeChild;
  const logs = { api: '', service: '' };
  const history = [];
  let stopped = false;
  const emergency = () => {
    for (const child of [apiChild, nativeChild]) if (child?.exitCode === null && !child.signalCode) {
      try { process.kill(-child.pid, 'SIGKILL'); } catch { /* already stopped */ }
    }
  };
  process.once('exit', emergency);
  const processes = () => history.map(({ role, child }) => ({ role, pid: child.pid, exitCode: child.exitCode, signal: child.signalCode, live: child.exitCode === null && !child.signalCode }));
  async function launch(role, binary, args, ready) {
    const bounded = boundedNative(binary, args, { lock: false });
    const child = spawn(bounded.command, bounded.args, { env, cwd: root, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    history.push({ role, child });
    if (role === 'api') apiChild = child; else nativeChild = child;
    let failure;
    child.once('error', error => { failure = error; });
    for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { logs[role] = (logs[role] + bytes).slice(-262_144); });
    const deadline = Date.now() + 12_000;
    while (Date.now() < deadline && child.exitCode === null && !child.signalCode && !failure) {
      try { if (await ready()) return; } catch { /* wait only for this owned process */ }
      await delay(30);
    }
    throw new Error(`${role} failed readiness: ${failure || logs[role]}`);
  }
  async function start() {
    await launch('service', native, [root, String(nativePort)], async () => {
      const response = await fetch(`${privateOrigin}/identity`, { signal: AbortSignal.timeout(500) });
      return response.ok && (await response.json()).identity === refs.application;
    });
    await launch('api', api, [root, String(apiPort), options.webroot || webroot], async () => {
      const response = await fetch(`${origin}/api/hale/v1/applications`, { signal: AbortSignal.timeout(500) });
      return response.ok;
    });
  }
  async function request(path, init = {}) {
    const response = await fetch(origin + apiPath + path, { signal: AbortSignal.timeout(10_000), ...init });
    return { status: response.status, body: await response.json() };
  }
  const service = {
    root, application: refs.application, principal: { mode: 'local', name: actor }, origin, privateOrigin, apiPath,
    url(view = 'knowledge', extra = {}) {
      return `${origin}/#/${view}?${new URLSearchParams({ app: refs.application, ...(view === 'knowledge' ? { id: refs.knowledge } : {}), ...extra })}`;
    },
    refs: async () => refs, processes, logs,
    request,
    capability: operation => request('/dna/knowledge/commands/capability' + (operation ? '?' + new URLSearchParams({ operation }) : '')),
    post: command => request('/dna/knowledge/commands', { method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(command) }),
    lookup: requestId => request(`/dna/knowledge/commands?request_id=${encodeURIComponent(requestId)}`),
    async recordHead() {
      const value = await request('/capabilities');
      if (value.status !== 200) throw new Error(`Record read failed ${JSON.stringify(value)}`);
      return value.body.source.record_head;
    },
    async command(requestId, overrides = {}) {
      return {
        request_id: requestId, operation: 'dna.knowledge.edge.link', operation_version: '1',
        context: { application_id: refs.application, position_id: 'org' },
        target: { application_id: refs.application, kind: 'dna.knowledge.node', id: refs.knowledge },
        preconditions: { principal: service.principal, record_head: await service.recordHead() },
        arguments: { from_id: refs.knowledge, to_id: refs.neighbors[0].id, rel: 'supports native recovery', rationale: 'Retain the recorded relationship across restart.' },
        ...overrides,
      };
    },
    async unlinkCommand(requestId, edge, overrides = {}) {
      return service.command(requestId, {
        operation: 'dna.knowledge.edge.unlink',
        arguments: { edge_id: edge.id, from_id: edge.from_id, to_id: edge.to_id, rel: edge.rel, rationale: 'Remove exactly this recorded relationship.' },
        ...overrides,
      });
    },
    async relationships(id = refs.knowledge) {
      const rows = [], cursors = new Set(); let cursor = '', snapshot = '';
      for (let page = 0; page < 32; page++) {
        const response = await request('/dna/knowledge/edges?' + new URLSearchParams({ id, limit: '25', ...(snapshot ? { snapshot } : {}), ...(cursor ? { cursor } : {}) }));
        if (response.status !== 200) throw new Error(`Relationship read failed ${JSON.stringify(response)}`);
        rows.push(...response.body.data.items);
        if (!response.body.data.page.has_more) return rows;
        snapshot = response.body.data.page.snapshot; cursor = response.body.data.page.next_cursor;
        if (cursors.has(cursor)) throw new Error('Relationship cursor repeated.'); cursors.add(cursor);
      }
      throw new Error('Relationship evidence exceeds 32 pages.');
    },
    async mutate(action) {
      const command = boundedNative(seed, [root, action]);
      const result = spawnSync(command.command, command.args, { env, encoding: 'utf8', timeout: 40_000, maxBuffer: 2_097_152 });
      if (result.status !== 0) throw new Error(`Knowledge fixture mutation failed: ${result.stderr || result.error}`);
    },
    async restart() { await stopChild(apiChild); await stopChild(nativeChild); await start(); },
    async setGrants(grants) { await writeFile(policyPath, JSON.stringify({ ...policy, grants })); await service.restart(); },
    async stop() {
      if (stopped) return;
      stopped = true;
      await stopChild(apiChild); await stopChild(nativeChild);
      process.removeListener('exit', emergency);
    },
    async exportEvidence(directory) {
      await mkdir(directory, { recursive: true });
      await Promise.all([
        writeFile(resolve(directory, 'service.json'), JSON.stringify({ application: refs.application, root, origin, binaries: { api, native, seed }, principal: service.principal, processes: processes() }, null, 2)),
        writeFile(resolve(directory, 'api.log'), logs.api), writeFile(resolve(directory, 'knowledge.log'), logs.service),
      ]);
    },
  };
  try { await start(); return service; }
  catch (error) { await service.stop(); if (options.evidence) await service.exportEvidence(options.evidence); throw error; }
}
