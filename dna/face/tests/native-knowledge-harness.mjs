// Real Knowledge commands + public API over memory. The seed binary only
// creates prior native subjects and stands in for the spine; command
// admission, projection and recovery are real.
//
// There is no Knowledge service (GH #985). The API admits a Knowledge command
// into the Record in-process, under the explicit authority policy, and reads
// the graph from memory under the head's role alone. The spine projects the
// Record into memory on its tick; this composition has no spine, so the seed
// binary's `project` action (migrate, then one projection under the spine's
// role) is run in its place after every admission and API restart. `drop`
// removes the Record's schema and roles when the fixture stops.
//
// Knowledge changes are the head's gated topics (GH #1129): a line of the
// api wire POSTed to …/commands, forwarded to the head's own socket. The
// head's uid is mapped to the actor, who holds a seat (the `position`
// gate); the policy grants that person. Every POST carries the launch token
// the head minted (GH #989), and a page the lane opens is given the session
// cookie again after each restart.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import net from 'node:net';
import { boundedNative, isolatedEnvironment, memoryOwner, launchToken } from './environment.mjs';
import { seatRecord } from './record-seats.mjs';
import { wireLine, knowledgeLookupLine, settleKnowledge } from './command-wire.mjs';

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

export const knowledgeEnvironmentPresent = () => ['API', 'SEED'].every(name => process.env[`HALE_KNOWLEDGE_${name}_BIN`]?.startsWith('/'));

export async function startKnowledgeService(options = {}) {
  const api = options.api || process.env.HALE_KNOWLEDGE_API_BIN;
  const seed = options.seed || process.env.HALE_KNOWLEDGE_SEED_BIN;
  if (![api, seed].every(value => value?.startsWith('/'))) throw new Error('Supply absolute Knowledge API and seed binary paths.');
  const owner = memoryOwner();
  if (!owner) throw new Error('Knowledge reads memory: set HALE_DNA_MEMORY_DSN_OWNER to a Postgres the seed binary may migrate a Record into.');
  const root = await mkdtemp('/tmp/hale-face-browser.knowledge-command.');
  const env = isolatedEnvironment();
  const actor = options.actor || 'alice';
  env.USER = actor;
  env.XDG_CACHE_HOME = resolve(root, '.hale/face-cache');
  // The owner's DSN reaches the seed binary only; the API is a head.
  const seedEnv = { ...env, HALE_DNA_MEMORY_DSN_OWNER: owner };
  const logs = { api: '', memory: '' };
  let migrated = false, dropped = false;
  const runSeed = (args, label) => {
    const command = boundedNative(seed, [root, ...args]);
    const result = spawnSync(command.command, command.args, { env: seedEnv, encoding: 'utf8', timeout: 40_000, maxBuffer: 2_097_152 });
    logs.memory = (logs.memory + (result.stderr || '')).slice(-262_144);
    if (result.status !== 0) throw new Error(`${label} failed: ${result.stderr || result.error || result.stdout}`);
    return result.stdout;
  };
  runSeed(['seed', String(options.count ?? 3)], 'Knowledge seed');
  // The actor holds a seat: a Knowledge change is gated `position`.
  seatRecord(root, env, actor, options.seats || ['editor']);
  const refs = JSON.parse(await readFile(resolve(root, 'fixture.json'), 'utf8'));
  const apiPort = options.port || await port();
  const origin = `http://127.0.0.1:${apiPort}`;
  const apiPath = `/api/hale/v1/applications/${refs.application}`;
  const policyPath = resolve(root, 'knowledge-authority.json');
  env.HALE_DNA_KNOWLEDGE_COMMAND_POLICY = policyPath;
  const policy = {
    format: 'dna.knowledge-authority/1', application_id: refs.application,
    grants: options.grants || [{ mode: 'local', name: actor, authority: 'knowledge-editor', edge_link: 'direct', edge_unlink: 'direct', recover: true }],
  };
  await writeFile(policyPath, JSON.stringify(policy));
  // One projection at a time: a browser POST and a Node-side command may
  // both ask for the spine's tick.
  let ticking = Promise.resolve();
  function tick() {
    const next = ticking.then(() => {
      migrated = true;
      const printed = runSeed(['project'], 'Knowledge projection').trim().split('\n').pop();
      if (!/^postgres(ql)?:\/\//.test(printed)) throw new Error('The seed binary printed no head DSN.');
      env.HALE_DNA_MEMORY_DSN_HEAD = printed;
    });
    ticking = next.catch(() => {});
    return next;
  }
  function drop() {
    if (!migrated || dropped) return;
    dropped = true;
    runSeed(['drop'], 'Knowledge memory drop');
  }
  let apiChild, token = '';
  // Pages the lane opened: each gets the session cookie of every launch.
  const pages = new Set();
  const history = [];
  let stopped = false;
  const emergency = () => {
    for (const child of [apiChild]) if (child?.exitCode === null && !child.signalCode) {
      try { process.kill(-child.pid, 'SIGKILL'); } catch { /* already stopped */ }
    }
    try { drop(); } catch { /* the process is exiting */ }
  };
  process.once('exit', emergency);
  const processes = () => history.map(({ role, child }) => ({ role, pid: child.pid, exitCode: child.exitCode, signal: child.signalCode, live: child.exitCode === null && !child.signalCode }));
  async function launch(role, binary, args, ready) {
    const bounded = boundedNative(binary, args, { lock: false });
    const child = spawn(bounded.command, bounded.args, { env, cwd: root, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    history.push({ role, child });
    apiChild = child;
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
    await tick();
    await launch('api', api, [root, String(apiPort), options.webroot || webroot], async () => {
      const response = await fetch(`${origin}/api/hale/v1/applications`, { signal: AbortSignal.timeout(500) });
      return response.ok;
    });
    token = await launchToken(root);
    for (const page of pages) await authorize(page);
  }
  // The URL the head printed, opened once: it sets the session cookie.
  async function authorize(page) {
    const opened = await page.request.get(`${origin}/?token=${token}`);
    if (opened.status() !== 200) throw new Error('The launch token did not open the shell: ' + opened.status());
  }
  async function request(path, init = {}) {
    const headers = { ...(init.method && init.method !== 'GET' ? { 'X-Hale-Token': token } : {}), ...(init.headers || {}) };
    const response = await fetch(origin + apiPath + path, { signal: AbortSignal.timeout(10_000), ...init, headers });
    return { status: response.status, body: await response.json() };
  }
  // One line of the api wire, forwarded; its receipt settled.
  async function forward(line) {
    const response = await request('/commands', { method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(line) });
    return { ...settleKnowledge(response.status, response.body), line };
  }
  const service = {
    root, application: refs.application, principal: { mode: 'local', name: actor }, origin, apiPath,
    url(view = 'knowledge', extra = {}) {
      return `${origin}/#/${view}?${new URLSearchParams({ app: refs.application, ...(view === 'knowledge' ? { id: refs.knowledge } : {}), ...extra })}`;
    },
    // A page this lane drives: given the session cookie now and after every
    // restart.
    async attach(page) { pages.add(page); await authorize(page); },
    get token() { return token; },
    refs: async () => refs, processes, logs,
    request, forward,
    // The spine's tick: the Record as it stands, projected into memory.
    tick,
    // The session's slice: the calls the head's describe line lists for it.
    async slice() {
      const described = await request('/commands', { method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: '{"describe":true}' });
      if (described.status !== 200 || !described.body.ok) throw new Error('describe failed ' + JSON.stringify(described));
      return described.body.value.commands.map(entry => entry.name);
    },
    // An admission moves the Record; the spine's tick follows it.
    async post(command) {
      const settled = await forward(wireLine(command));
      await tick();
      return settled;
    },
    lookup: requestId => forward(knowledgeLookupLine(requestId)),
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
    // Each mutation projects again, as the spine's next tick would.
    async mutate(action) { await ticking; runSeed([action], 'Knowledge fixture mutation'); },
    async restart() { await stopChild(apiChild); await start(); },
    async setGrants(grants) { await writeFile(policyPath, JSON.stringify({ ...policy, grants })); await service.restart(); },
    async stop() {
      if (stopped) return;
      stopped = true;
      await stopChild(apiChild);
      await ticking;
      try { drop(); } finally { process.removeListener('exit', emergency); }
    },
    async exportEvidence(directory) {
      await mkdir(directory, { recursive: true });
      await Promise.all([
        writeFile(resolve(directory, 'service.json'), JSON.stringify({ application: refs.application, root, origin, binaries: { api, seed }, principal: service.principal, processes: processes() }, null, 2)),
        writeFile(resolve(directory, 'api.log'), logs.api), writeFile(resolve(directory, 'memory.log'), logs.memory),
      ]);
    },
  };
  try { await start(); return service; }
  catch (error) {
    try { await service.stop(); } catch (failure) { error.message += `\n${failure.message}`; }
    if (options.evidence) await service.exportEvidence(options.evidence);
    throw error;
  }
}
