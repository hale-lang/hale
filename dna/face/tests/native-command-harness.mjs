// Reusable real native service fixture, also usable by a local preview owner.
// No Playwright dependency, mocked wire data or authored outcome facts.
//
// GH #1029: the composition is the real one. A project is scaffolded with
// `hale dna new`, the acceptance Body (dna/api/practice_review/tests/body)
// becomes its organization (`dna/org`), and `hale dna dev` is the host:
// it migrates the record's memory (from the owner's DSN) and its nerves
// (from the owner's NATS URL), builds and runs the organization, relays the
// record's request rows onto the nerves and projects memory on its tick.
// The API is the composed head (`dna/api/practice_review`), reading memory
// under the head's role. When the service stops, the record's memory is
// dropped and its stream too.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { boundedNative, isolatedEnvironment, memoryOwner, nervesOwner } from './environment.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const webrootDefault = fileURLToPath(new URL('../web/', import.meta.url));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

/**
 * The service fixture's own budget, apart from its case's: the first
 * service on a project path builds the organization cold (about 80 s on a
 * CI runner), which no case budget should pay. A spec gives its `service`
 * fixture `{ timeout: serviceFixtureTimeout }`.
 */
export const serviceFixtureTimeout = 300_000;

/** What a native command lane needs: the toolchain (the host), the composed API, the memory fixture, and both owners. */
export const nativeCommandEnvironmentPresent = () =>
  ['HALE_BIN', 'HALE_NATIVE_COMMAND_API', 'HALE_FACE_MEMORY_BIN'].every(name => Boolean(process.env[name])) && Boolean(memoryOwner()) && Boolean(nervesOwner());

/** The acceptance Body as a project's organization: its imports point at this checkout. */
export function organizationSource() {
  const body = path.join(repo, 'dna/api/practice_review/tests/body/main.hl');
  return fs.readFileSync(body, 'utf8')
    .replace('"../../../../core/pond/realtime/nats"', JSON.stringify(path.join(repo, 'dna/core/pond/realtime/nats')))
    .replace('"../../../../operations"', JSON.stringify(path.join(repo, 'dna/operations')))
    .replace('"../../../../core"', JSON.stringify(path.join(repo, 'dna/core')));
}

/** A project for one lane: scaffolded, the Body its organization, committed (the host runs the genome at HEAD). */
export function scaffoldProject({ hale, root, env, actor }) {
  const parent = path.dirname(root), name = path.basename(root);
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(parent, { recursive: true });
  const bounded = boundedNative(hale, ['dna', 'new', name], { build: true, lock: false });
  execFileSync(bounded.command, bounded.args, { cwd: parent, env, encoding: 'utf8', timeout: 120_000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  const org = path.join(root, 'dna/org');
  for (const entry of fs.readdirSync(org)) fs.rmSync(path.join(org, entry), { recursive: true, force: true });
  fs.writeFileSync(path.join(org, 'main.hl'), organizationSource());
  const git = (...args) => execFileSync('git', ['-C', root, ...args], { env, encoding: 'utf8', timeout: 10_000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  git('config', 'user.name', 'Native browser acceptance'); git('config', 'user.email', 'native-browser@example.invalid');
  git('config', 'dna.principal', 'local'); git('config', 'dna.trust', 'local');
  git('add', '-A'); git('commit', '--quiet', '--allow-empty', '-m', `native acceptance project for ${actor}`);
}

/** The host is up: the organization reads the nerves, and the genome it runs is named. */
export const hostReady = text => text.includes('the organization reads its facts from the nerves') && /genome [0-9a-f]{6,}/.test(text);

/** The organization's bootstrap line from its log under the host, or null. */
export function organizationReady(root) {
  let text;
  try { text = fs.readFileSync(path.join(root, '.hale/dna/org.log'), 'utf8'); } catch { return null; }
  const matches = text.match(/\{[^{}]*"ready"\s*:\s*true[^{}]*\}/g) || [];
  return matches.length ? JSON.parse(matches.at(-1)) : null;
}

/** The pids the host wrote for what it started (the organization, the expression). */
export function hostChildren(root) {
  const pids = [];
  for (const name of ['org', 'app']) {
    try { const pid = Number(fs.readFileSync(path.join(root, `.hale/dna/${name}.pid`), 'utf8').trim()); if (pid > 0) pids.push({ name, pid }); } catch { /* not started */ }
  }
  return pids;
}

export const running = pid => { try { process.kill(pid, 0); return true; } catch (error) { return error.code === 'EPERM'; } };

/** `hale dna memory migrate` under the owner: the head's DSN for the API (idempotent; `dev` applied the schema already). */
export function headMemory({ hale, root, env, owner }) {
  const bounded = boundedNative(hale, ['dna', 'memory', 'migrate', root], { lock: false });
  const out = execFileSync(bounded.command, bounded.args, { cwd: root, env: { ...env, HALE_DNA_MEMORY_DSN_OWNER: owner }, encoding: 'utf8', timeout: 40_000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  const head = out.split('\n').find(line => line.startsWith('HALE_DNA_MEMORY_DSN_HEAD='))?.slice('HALE_DNA_MEMORY_DSN_HEAD='.length);
  assert(head, `hale dna memory migrate printed no head DSN:\n${out}`);
  return head;
}

/** Caller owns stop(). Keeping the returned service alive supports a preview. */
export async function startService(options = {}) {
  assert.equal(process.platform, 'linux', 'Native command fixtures require Linux process groups and resource limits.');
  const absolute = (name, value) => {
    assert(value && path.isAbsolute(value), `Supply an absolute ${name}`);
    fs.accessSync(value, fs.constants.X_OK);
    return fs.realpathSync(value);
  };
  const hale = absolute('HALE_BIN (the toolchain: hale dna dev is the host)', options.binaries?.hale || process.env.HALE_BIN);
  const apiBinary = absolute('HALE_NATIVE_COMMAND_API binary (dna/api/practice_review)', options.binaries?.api || process.env.HALE_NATIVE_COMMAND_API);
  const memoryBinary = absolute('HALE_FACE_MEMORY_BIN (dna/face/tests/memory, built by run.mjs)', options.binaries?.memory || process.env.HALE_FACE_MEMORY_BIN);
  const owner = options.memoryOwner || memoryOwner();
  assert(owner, 'The record lives in memory: set HALE_DNA_MEMORY_DSN_OWNER to a Postgres the host may migrate into.');
  const nerves = options.nervesOwner || nervesOwner();
  assert(nerves, 'The organization reads the nerves: set HALE_DNA_NATS_URL_OWNER to a NATS server running JetStream (nats-server -js).');
  const parent = options.evidenceParent || process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
  assert(path.isAbsolute(parent), 'Native evidence parent must be absolute');
  fs.mkdirSync(parent, { recursive: true });
  const evidence = fs.mkdtempSync(path.join(parent, 'native-command-browser-'));
  // One project path per run and parallel slot: the host's build cache keys
  // the organization's build on the seed's path and contents, so every
  // service after the first gets the organization built in seconds rather
  // than a minute. Not per worker process: Playwright replaces the worker
  // after a failed case, and a path keyed on its pid made every case after
  // a failure build cold again. The runner (the worker's parent) and the
  // slot index both survive that replacement; two slots never share a path.
  if (options.rootPrefix) {
    assert(path.isAbsolute(options.rootPrefix), 'Native rootPrefix must be absolute');
    fs.mkdirSync(path.dirname(options.rootPrefix), { recursive: true });
  }
  const slot = `${process.ppid}-${process.env.TEST_PARALLEL_INDEX ?? process.pid}`;
  const root = options.rootPrefix ? path.join(fs.mkdtempSync(options.rootPrefix), 'project') : path.join(os.tmpdir(), `hale-face-browser.native-${slot}`, 'project');
  const policy = path.join(evidence, 'authority.json');
  const isolated = isolatedEnvironment();
  const env = Object.fromEntries([
    'PATH', 'HOME', 'LANG', 'LC_ALL', 'TZ', 'GIT_CONFIG_NOSYSTEM',
    'GIT_CONFIG_GLOBAL', 'GIT_TERMINAL_PROMPT', 'HALE_DNA_DISCOVER',
  ].filter(key => isolated[key] !== undefined).map(key => [key, isolated[key]]));
  const principal = options.actor || 'alice';
  assert(principal && !/[\x00-\x1f\x7f]/u.test(principal), 'Expected a nonempty control-free local principal');
  // Preview-only startup configuration is explicit, never inherited. HALE_BIN
  // selects the trusted source validator; XDG_CACHE_HOME isolates its cache.
  const apiEnv = { ...options.apiEnv };
  // Knowledge is read from memory as a head and commanded in-process (GH
  // #985): the head's DSN and the command policy, never the owner's or the
  // spine's DSN.
  const apiEnvironmentKeys = ['HALE_DNA_ORG_DRAFTS', 'HALE_DNA_MEMORY_DSN_HEAD', 'HALE_DNA_KNOWLEDGE_COMMAND_POLICY', 'HALE_BIN', 'XDG_CACHE_HOME'];
  for (const key of Object.keys(apiEnv)) assert(apiEnvironmentKeys.includes(key), `Unsupported explicit API environment setting: ${key}`);
  const owned = new Set(), processLog = [], requestLog = [];
  let sequence = 0, host, api, dependencies, application = '', practice = '', origin = '', stopped = false, headDsn = '';
  let currentActor = principal;
  const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n');
  const git = (...args) => execFileSync('git', ['-C', root, ...args], { env, encoding: 'utf8', timeout: 5000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  const alive = item => item?.child.pid && !item.error && item.child.exitCode === null && item.child.signalCode === null;
  const signal = (item, name) => {
    if (!item?.child.pid || !owned.has(item)) return;
    try { process.kill(-item.child.pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  };
  const signalPid = (pid, name) => { try { process.kill(pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; } };
  function healthy() {
    assert(!stopped, 'Native service is stopped');
    for (const item of owned) if (!item.stopping && !alive(item)) throw new Error(`${item.name} exited (${item.child.exitCode ?? item.child.signalCode}): ${item.output.slice(-4000)}`);
  }
  async function wait(label, read, predicate, timeout = 20_000) {
    const deadline = Date.now() + timeout;
    let last;
    while (Date.now() < deadline) {
      healthy(); last = await read();
      if (predicate(last)) return last;
      await delay(100);
    }
    throw new Error(`${label} timed out: ${JSON.stringify(last)}`);
  }
  function launch(name, binary, args = [], extra = {}, { build = false } = {}) {
    const bounded = boundedNative(binary, args, { lock: false, build });
    const filename = `${String(++sequence).padStart(2, '0')}-${name}.log`;
    const log = fs.createWriteStream(path.join(evidence, filename));
    const child = spawn(bounded.command, bounded.args, { cwd: root, env: { ...env, USER: currentActor, LOGNAME: currentActor, ...extra }, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const item = { name, child, output: '', error: null, paused: false, stopping: false };
    let closed; item.closed = new Promise(resolve => { closed = resolve; });
    owned.add(item);
    const record = { name, binary, pid: child.pid, args, log: filename }; processLog.push(record);
    for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { log.write(chunk); item.output = (item.output + chunk).slice(-128 * 1024); });
    child.on('error', error => { item.error = error; record.error = error.message; });
    child.on('close', (code, termination) => { record.code = code; record.signal = termination; log.end(); closed(); });
    return item;
  }
  async function stopProcess(item, kill = false) {
    if (!item || !owned.has(item)) return;
    item.stopping = true;
    if (alive(item)) {
      if (kill) signal(item, 'SIGKILL');
      else { if (item.paused) signal(item, 'SIGCONT'); signal(item, 'SIGTERM'); }
      await Promise.race([item.closed, delay(item === host ? 8000 : 1500)]);
      if (alive(item)) { signal(item, 'SIGKILL'); await Promise.race([item.closed, delay(1500)]); }
    }
    signal(item, 'SIGKILL'); // Belt and braces: any child surviving its leader.
    assert(!alive(item), `${item.name} did not terminate`);
    owned.delete(item);
  }
  // The host starts the organization and the expression in sessions of
  // their own; a host killed outright leaves them, so they are reaped here.
  async function reapHostChildren() {
    for (const { pid } of hostChildren(root)) { signalPid(pid, 'SIGCONT'); signalPid(pid, 'SIGKILL'); }
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline && hostChildren(root).some(({ pid }) => running(pid))) await delay(100);
  }
  const killOnExit = () => { for (const item of owned) signal(item, 'SIGKILL'); for (const { pid } of hostChildren(root)) signalPid(pid, 'SIGKILL'); };
  process.on('exit', killOnExit);
  function journal() {
    const head = git('rev-parse', 'refs/dna/journal').trim();
    const text = git('show', `${head}:journal.jsonl`);
    return { head, text, rows: text.trim().split('\n').filter(Boolean).map(line => {
      const row = JSON.parse(line); let data = null;
      try { data = JSON.parse(row.body); } catch { /* Native settlement is prose. */ }
      return { ...row, data };
    }) };
  }
  function get(suffix) {
    healthy();
    return new Promise((resolve, reject) => {
      const entry = { method: 'GET', path: suffix }; requestLog.push(entry);
      const request = http.get(origin + suffix, { agent: false }, response => {
        const chunks = []; let size = 0;
        response.on('data', chunk => { size += chunk.length; if (size > 2 * 1024 * 1024) request.destroy(new Error('Native read exceeded response bound')); else chunks.push(chunk); });
        response.on('end', () => {
          try { const json = JSON.parse(Buffer.concat(chunks).toString('utf8')); entry.status = response.statusCode; entry.code = json.error?.code; resolve({ status: response.statusCode, json }); }
          catch (error) { reject(error); }
        });
        response.on('error', reject);
      });
      request.setTimeout(10_000, () => request.destroy(new Error(`Native GET timed out: ${suffix}`)));
      request.on('error', reject);
    });
  }
  const transient = result => result.status === 409
    ? result.json.error?.code === 'snapshot_changed' && result.json.error?.retryable === true
    : [501, 503].includes(result.status) && (result.json.error?.retryable === true || ['record_unavailable', 'commands_unavailable'].includes(result.json.error?.code));
  const read = suffix => wait('native source read', () => get(suffix), result => !transient(result));
  const apiPath = () => `/api/hale/v1/applications/${encodeURIComponent(application)}`;
  // Quiet is a stable Record the head can read. The host projects memory on
  // its tick, and until it has projected the Record's head the head answers
  // a Knowledge read 503 `knowledge_projection_unavailable` (retryable). A
  // browser read does not retry that; it shows "Knowledge unavailable", so a
  // page opened on a stable Record the projection has not reached stays
  // there. With the host paused or no API up there is nothing to wait for.
  // Stable means unchanged for longer than one host tick (HOST_TICK, 1 s):
  // a request row the host has yet to relay moves the Record again on the
  // next tick, and a shorter window fits between two.
  async function stableHead() {
    let head = journal().head, since = Date.now();
    await wait('stable Record', async () => { await delay(100); const next = journal().head; if (next !== head) { head = next; since = Date.now(); } return Date.now() - since; }, quiet => quiet >= 1500);
    return head;
  }
  const projecting = result => result.status === 503 && result.json.error?.code === 'knowledge_projection_unavailable';
  async function quiesce() {
    for (let round = 0; ; round++) {
      const head = await stableHead();
      if (!alive(api) || !alive(host) || host.paused) return;
      await wait('Knowledge projection at the Record head', () => get(apiPath() + '/dna/knowledge/nodes?limit=1'), result => !projecting(result), 30_000);
      if (journal().head === head || round >= 4) return;
    }
  }
  // `hale dna dev` in place of the Body and the relay: it migrates memory and
  // the nerves from the owners' URLs (which reach no other process), builds
  // and runs the organization, and relays the record's requests on its tick.
  async function startHost() {
    assert(!alive(host), 'Stop the current host before starting another');
    host = launch('host', hale, ['dna', 'dev', '.', '--no-iris'], {
      HALE_BIN: hale, HALE_DNA_MEMORY_DSN_OWNER: owner, HALE_DNA_NATS_URL_OWNER: nerves,
      // The host's build cache: one per home, shared across the services this worker starts.
      ...(process.env.XDG_CACHE_HOME ? { XDG_CACHE_HOME: process.env.XDG_CACHE_HOME } : {}),
    }, { build: true });
    await wait('host startup', () => host.output, hostReady, 240_000);
    const ready = await wait('organization bootstrap', () => organizationReady(root), result => result?.ready === true, 60_000);
    headDsn = headMemory({ hale, root, env, owner });
    return ready;
  }
  async function stopHost(kill = false) {
    await stopProcess(host, kill);
    await reapHostChildren();
  }
  async function startAPI(actor = currentActor) {
    assert(!alive(api), 'Stop the current API before starting another');
    currentActor = actor;
    api = launch(`api-${actor}`, apiBinary, [root, new URL(origin).port, options.webroot || webrootDefault], { ...apiEnv, HALE_DNA_MEMORY_DSN_HEAD: headDsn, HALE_DNA_COMMAND_POLICY: policy });
    await wait('API startup', async () => {
      try { return await get('/api/hale/v1/applications'); }
      catch (error) { if (['ECONNREFUSED', 'ECONNRESET'].includes(error.code)) return null; throw error; }
    }, response => response?.status === 200 && response.json.source?.record_id === application);
    const capabilities = await read(apiPath() + '/capabilities');
    assert.equal(capabilities.status, 200); assert.deepEqual(capabilities.json.data.principal, { mode: 'local', name: actor });
    return capabilities.json.data;
  }
  // Delivery paused: the host (the relay) and the organization stopped
  // where they stand, so a request admitted meanwhile has no outcome yet.
  async function pauseDelivery() {
    await quiesce();
    const children = hostChildren(root).filter(({ name }) => name === 'org');
    for (const { pid } of children) signalPid(pid, 'SIGSTOP');
    signal(host, 'SIGSTOP'); host.paused = true;
    const stoppedState = pid => { try { return /^State:\s+T/m.test(fs.readFileSync(`/proc/${pid}/status`, 'utf8')); } catch { return false; } };
    for (const { pid } of [{ pid: host.child.pid }, ...children]) await wait(`paused ${pid}`, () => stoppedState(pid), Boolean);
  }
  function resumeDelivery() {
    for (const { pid } of hostChildren(root).filter(({ name }) => name === 'org')) signalPid(pid, 'SIGCONT');
    signal(host, 'SIGCONT'); host.paused = false;
  }
  async function restart() {
    await stopProcess(api); await stopHost(host?.paused);
    await dependencies?.restart?.();
    await startHost(); await quiesce(); await startAPI();
  }
  function exportEvidence() {
    const snapshot = journal();
    fs.writeFileSync(path.join(evidence, 'journal.jsonl'), snapshot.text);
    fs.writeFileSync(path.join(evidence, 'refs.txt'), git('show-ref'));
    save('processes.json', processLog); save('reads.json', requestLog);
    save('service.json', { application, practice, origin, root, evidence, processes: processLog, remaining_owned_processes: owned.size,
      binaries: Object.fromEntries(Object.entries({ hale, api: apiBinary, memory: memoryBinary }).map(([name, binary]) => [name, { path: binary, sha256: createHash('sha256').update(fs.readFileSync(binary)).digest('hex') }])) });
    return evidence;
  }
  // The record's memory and its stream, gone with the service.
  function dropMemory() {
    const failures = [];
    for (const [label, binary, args, extra] of [
      ['memory drop', memoryBinary, [root, 'drop'], { HALE_DNA_MEMORY_DSN_OWNER: owner }],
      ['nerves drop', hale, ['dna', 'nerves', 'drop', root], { HALE_DNA_NATS_URL_OWNER: nerves }],
    ]) {
      try {
        const bounded = boundedNative(binary, args, { lock: false });
        execFileSync(bounded.command, bounded.args, { cwd: root, env: { ...env, ...extra }, encoding: 'utf8', timeout: 40_000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
      } catch (error) { failures.push(new Error(`${label} failed: ${error.stderr || error.message}`)); }
    }
    return failures[0];
  }
  async function stop() {
    if (stopped) return;
    let failure;
    for (const item of [...owned].reverse()) try { await stopProcess(item, item.paused); } catch (error) { failure ||= error; }
    try { await reapHostChildren(); } catch (error) { failure ||= error; }
    try { exportEvidence(); } catch (error) { failure ||= error; }
    if (application) failure ||= dropMemory();
    stopped = true;
    if (!owned.size) process.removeListener('exit', killOnExit);
    if (failure) throw failure;
  }
  try {
    scaffoldProject({ hale, root, env: { ...env, USER: principal, LOGNAME: principal }, actor: principal });
    application = git('rev-list', '--max-parents=0', 'refs/dna/journal').trim();
    assert.match(application, /^[0-9a-f]{40,64}$/);
    // Explicit preview composition may prepare source/read fixtures here.
    // The acceptance tests omit this hook; the real organization alone bootstraps them.
    if (options.prepareProject) await options.prepareProject({ root, env: { ...env, USER: principal, LOGNAME: principal }, git });
    const ready = await startHost(); practice = ready.practice_id;
    assert.equal(practice, ready.bootstrap_digest);
    save('authority.json', { format: 'dna.practice-review-authority/1', application_id: application, grants: [{ mode: 'local', name: principal, authority: 'board', practice_propose: true, review_verdict: true, recover: true }] });
    let port = options.port;
    if (port === undefined) {
      const server = net.createServer();
      await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
      port = server.address().port; await new Promise(resolve => server.close(resolve));
    }
    assert(Number.isInteger(port) && port > 0 && port < 65536, 'Expected a valid loopback port');
    origin = `http://127.0.0.1:${port}`;
    // Optional same-Record services join this fixture's bounded process owner.
    // Native browser composition supplies exact binaries and policy explicitly.
    if (options.startDependencies) {
      dependencies = await options.startDependencies({ root, application, principal, origin, evidence, env: { ...env, USER: principal, LOGNAME: principal }, launch, wait, stopProcess, headDsn });
      for (const [key, value] of Object.entries(dependencies?.apiEnv || {})) {
        assert(apiEnvironmentKeys.includes(key), `Unsupported explicit dependency API environment: ${key}`); apiEnv[key] = value;
      }
    }
    await quiesce(); await startAPI();
    const service = {
      application, app: application, practice, origin, root, evidence, policy, principal: { mode: 'local', name: principal }, headDsn: () => headDsn,
      apiPath: apiPath(), text: 'Collect the exact receipt.\nKeep its provenance.',
      url: (view = 'practices', extra = {}) => `${origin}/#/${view}?${new URLSearchParams({ app: application, ...extra })}`,
      read, journal, quiesce, startHost, stopHost, startAPI, pauseDelivery, resumeDelivery, restart, stop, exportEvidence,
      // The Body and the relay are one process now, the host; the old names
      // still name what they name.
      startBody: () => startHost(), startRelay: async () => { assert(alive(host), 'The host is the relay: start it'); },
      stopBody: () => stopHost(host?.paused), stopRelay: () => stopHost(host?.paused),
      processes: () => [...[...owned].map(item => ({ name: item.name, pid: item.child.pid, alive: Boolean(alive(item)), paused: item.paused })), ...hostChildren(root).filter(({ pid }) => running(pid)).map(({ name, pid }) => ({ name, pid, alive: true, paused: host?.paused ?? false }))],
      stopAPI: () => stopProcess(api),
      facts: (kind, entity) => journal().rows.filter(row => row.kind === kind && (entity === undefined || row.entity === entity)),
      admissions: requestId => journal().rows.filter(row => ['practice.requested', 'review.verdict'].includes(row.kind) && row.data?.command_payload && JSON.parse(row.data.command_payload).request_id === requestId),
      candidate: digest => {
        assert.match(digest, /^sha256:[0-9a-f]{64}$/);
        const raw = git('cat-file', '-p', 'refs/dna/receipts/' + digest.slice(7));
        assert.equal('sha256:' + createHash('sha256').update(raw, 'utf8').digest('hex'), digest);
        fs.writeFileSync(path.join(evidence, `candidate-${digest.slice(7)}.json`), raw);
        return JSON.parse(raw);
      },
      waitCommand: async (requestId, predicate) => wait(`command ${requestId}`, async () => {
        const response = await read(apiPath() + '/commands?' + new URLSearchParams({ request_id: requestId }));
        assert.equal(response.status, 200, JSON.stringify(response));
        assert.equal(response.json.source?.record_id, application);
        assert.equal(response.json.data.request_id, requestId);
        return response.json.data;
      }, predicate),
    };
    exportEvidence();
    return service;
  } catch (error) {
    try { await stop(); } catch { /* Preserve startup failure; process logs remain. */ }
    throw new Error(`${error.message}\nNative evidence: ${evidence}`, { cause: error });
  }
}
