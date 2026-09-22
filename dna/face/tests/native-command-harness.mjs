// Reusable real native service fixture, also usable by a local preview owner.
// No Playwright dependency, builds, mocked wire data or authored outcome facts.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { boundedNative, isolatedEnvironment } from './environment.mjs';

const names = ['API', 'BODY', 'RELAY', 'MEMBRANE'];
export const nativeCommandEnvironmentPresent = () => names.every(name => Boolean(process.env[`HALE_NATIVE_COMMAND_${name}`]));
const webrootDefault = fileURLToPath(new URL('../web/', import.meta.url));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Caller owns stop(). Keeping the returned service alive supports a preview. */
export async function startService(options = {}) {
  assert.equal(process.platform, 'linux', 'Native command fixtures require Linux process groups and resource limits.');
  const binaries = Object.fromEntries(names.map(name => {
    const value = options.binaries?.[name.toLowerCase()] || process.env[`HALE_NATIVE_COMMAND_${name}`];
    assert(value && path.isAbsolute(value), `Supply an absolute HALE_NATIVE_COMMAND_${name} binary`);
    fs.accessSync(value, fs.constants.X_OK);
    return [name.toLowerCase(), fs.realpathSync(value)];
  }));
  const parent = options.evidenceParent || process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
  assert(path.isAbsolute(parent), 'Native evidence parent must be absolute');
  fs.mkdirSync(parent, { recursive: true });
  const evidence = fs.mkdtempSync(path.join(parent, 'native-command-browser-'));
  if (options.rootPrefix) {
    assert(path.isAbsolute(options.rootPrefix), 'Native rootPrefix must be absolute');
    fs.mkdirSync(path.dirname(options.rootPrefix), { recursive: true });
  }
  const root = options.rootPrefix ? fs.mkdtempSync(options.rootPrefix) : path.join(evidence, 'project');
  fs.mkdirSync(path.join(root, '.hale/dna'), { recursive: true });
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
  const apiEnvironmentKeys = ['HALE_IRIS_ORG_DRAFTS', 'HALE_DNA_KNOWLEDGE_URL', 'HALE_DNA_KNOWLEDGE_READ_KEY', 'HALE_DNA_KNOWLEDGE_COMMAND_KEY', 'HALE_BIN', 'XDG_CACHE_HOME'];
  for (const key of Object.keys(apiEnv)) assert(apiEnvironmentKeys.includes(key), `Unsupported explicit API environment setting: ${key}`);
  const owned = new Set(), processLog = [], requestLog = [];
  let sequence = 0, body, relay, api, dependencies, application = '', practice = '', origin = '', stopped = false;
  let currentActor = principal;
  const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n');
  const git = (...args) => execFileSync('git', ['-C', root, ...args], { env, encoding: 'utf8', timeout: 5000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  const alive = item => item?.child.pid && !item.error && item.child.exitCode === null && item.child.signalCode === null;
  const signal = (item, name) => {
    if (!item?.child.pid || !owned.has(item)) return;
    try { process.kill(-item.child.pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  };
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
  function launch(name, binary, args = [], extra = {}) {
    const bounded = boundedNative(binary, args, { lock: false });
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
      await Promise.race([item.closed, delay(1500)]);
      if (alive(item)) { signal(item, 'SIGKILL'); await Promise.race([item.closed, delay(1500)]); }
    }
    signal(item, 'SIGKILL'); // Includes a membrane child surviving its leader.
    assert(!alive(item), `${item.name} did not terminate`);
    owned.delete(item);
  }
  const killOnExit = () => { for (const item of owned) signal(item, 'SIGKILL'); };
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
  async function quiesce() {
    let head = journal().head, same = 0;
    await wait('stable Record', async () => { await delay(100); const next = journal().head; same = next === head ? same + 1 : 0; head = next; return same; }, count => count >= 3);
  }
  async function startBody() {
    assert(!alive(body), 'Stop the current body before starting another');
    body = launch('body', binaries.body);
    const ready = await wait('native bootstrap', () => {
      const matches = body.output.match(/\{[^{}]*"ready"\s*:\s*true[^{}]*\}/g) || [];
      return matches.length ? JSON.parse(matches.at(-1)) : null;
    }, result => result?.ready === true);
    return ready;
  }
  async function startRelay() {
    assert(!alive(relay), 'Stop the current relay before starting another');
    relay = launch('relay', binaries.relay, [], { HALE_DNA_MEMBRANE: binaries.membrane });
    await wait('relay startup', () => relay.output, text => text.includes('native command relay ready'));
  }
  async function startAPI(actor = currentActor) {
    assert(!alive(api), 'Stop the current API before starting another');
    currentActor = actor;
    api = launch(`api-${actor}`, binaries.api, [root, new URL(origin).port, options.webroot || webrootDefault], { ...apiEnv, HALE_DNA_COMMAND_POLICY: policy });
    await wait('API startup', async () => {
      try { return await get('/api/hale/v1/applications'); }
      catch (error) { if (['ECONNREFUSED', 'ECONNRESET'].includes(error.code)) return null; throw error; }
    }, response => response?.status === 200 && response.json.source?.record_id === application);
    const capabilities = await read(apiPath() + '/capabilities');
    assert.equal(capabilities.status, 200); assert.deepEqual(capabilities.json.data.principal, { mode: 'local', name: actor });
    return capabilities.json.data;
  }
  async function pauseDelivery() {
    await quiesce();
    for (const item of [relay, body]) { signal(item, 'SIGSTOP'); item.paused = true; }
    for (const item of [relay, body]) await wait(`paused ${item.name}`, () => fs.readFileSync(`/proc/${item.child.pid}/status`, 'utf8'), text => /^State:\s+T/m.test(text));
  }
  function resumeDelivery() { for (const item of [body, relay]) { signal(item, 'SIGCONT'); item.paused = false; } }
  async function restart() {
    await stopProcess(api); await stopProcess(relay, relay.paused); await stopProcess(body, body.paused);
    await dependencies?.restart?.();
    await startBody(); await startRelay(); await quiesce(); await startAPI();
  }
  function exportEvidence() {
    const snapshot = journal();
    fs.writeFileSync(path.join(evidence, 'journal.jsonl'), snapshot.text);
    fs.writeFileSync(path.join(evidence, 'refs.txt'), git('show-ref'));
    save('processes.json', processLog); save('reads.json', requestLog);
    save('service.json', { application, practice, origin, root, evidence, processes: processLog, remaining_owned_processes: owned.size,
      binaries: Object.fromEntries(Object.entries(binaries).map(([name, binary]) => [name, { path: binary, sha256: createHash('sha256').update(fs.readFileSync(binary)).digest('hex') }])) });
    return evidence;
  }
  async function stop() {
    if (stopped) return;
    let failure;
    for (const item of [...owned].reverse()) try { await stopProcess(item, item.paused); } catch (error) { failure ||= error; }
    try { exportEvidence(); } catch (error) { failure ||= error; }
    stopped = true;
    if (!owned.size) process.removeListener('exit', killOnExit);
    if (failure) throw failure;
  }
  try {
    git('init', '--quiet'); git('config', 'user.name', 'Native browser acceptance'); git('config', 'user.email', 'native-browser@example.invalid');
    git('config', 'dna.principal', 'local'); git('config', 'dna.trust', 'local');
    // Explicit preview composition may prepare source/read fixtures here.
    // The acceptance tests omit this hook; the real body alone bootstraps them.
    if (options.prepareProject) await options.prepareProject({ root, env: { ...env, USER: principal, LOGNAME: principal }, git });
    const ready = await startBody(); practice = ready.practice_id;
    application = git('rev-list', '--max-parents=0', 'refs/dna/journal').trim();
    assert.match(application, /^[0-9a-f]{40,64}$/); assert.equal(practice, ready.bootstrap_digest);
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
      dependencies = await options.startDependencies({ root, application, principal, origin, evidence, env: { ...env, USER: principal, LOGNAME: principal }, launch, wait, stopProcess });
      for (const [key, value] of Object.entries(dependencies?.apiEnv || {})) {
        assert(apiEnvironmentKeys.includes(key), `Unsupported explicit dependency API environment: ${key}`); apiEnv[key] = value;
      }
    }
    await startRelay(); await quiesce(); await startAPI();
    const service = {
      application, app: application, practice, origin, root, evidence, policy, principal: { mode: 'local', name: principal },
      apiPath: apiPath(), text: 'Collect the exact receipt.\nKeep its provenance.',
      url: (view = 'practices', extra = {}) => `${origin}/#/${view}?${new URLSearchParams({ app: application, ...extra })}`,
      read, journal, quiesce, startBody, startRelay, startAPI, pauseDelivery, resumeDelivery, restart, stop, exportEvidence,
      processes: () => [...owned].map(item => ({ name: item.name, pid: item.child.pid, alive: Boolean(alive(item)), paused: item.paused })),
      stopAPI: () => stopProcess(api), stopBody: () => stopProcess(body, body.paused), stopRelay: () => stopProcess(relay, relay.paused),
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
