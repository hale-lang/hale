// Real Task-only composition: native Dna.ask handoff seed, then the actual
// configured API. No Body/Host, fabricated command outcomes, or graph service.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { boundedNative, isolatedEnvironment, launchToken } from './environment.mjs';
import { seatRecord, unseatRecord } from './record-seats.mjs';
import { settle, wireLine } from './command-wire.mjs';

export const nativeTaskEnvironmentPresent = () => Boolean(process.env.HALE_NATIVE_TASK_API && process.env.HALE_NATIVE_TASK_SEED);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const webroot = fileURLToPath(new URL('../web/', import.meta.url));
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');

export async function startTaskService({ evidenceParent = process.env.HALE_NATIVE_TASK_EVIDENCE || os.tmpdir(), actor = 'riley', taskPolicy } = {}) {
  assert.equal(typeof taskPolicy, 'function', 'Supply the exact native Task policy factory');
  const binaries = Object.fromEntries(['API', 'SEED'].map(key => {
    const binary = process.env[`HALE_NATIVE_TASK_${key}`]; assert(binary && path.isAbsolute(binary)); fs.accessSync(binary, fs.constants.X_OK);
    return [key.toLowerCase(), fs.realpathSync(binary)];
  }));
  fs.mkdirSync(evidenceParent, { recursive: true });
  const evidence = fs.mkdtempSync(path.join(evidenceParent, 'native-task-'));
  const root = path.join(evidence, 'project'); fs.mkdirSync(path.join(root, '.hale/dna'), { recursive: true });
  const inherited = isolatedEnvironment();
  const env = Object.fromEntries(['PATH', 'HOME', 'LANG', 'LC_ALL', 'TZ', 'GIT_CONFIG_NOSYSTEM', 'GIT_CONFIG_GLOBAL', 'GIT_TERMINAL_PROMPT', 'HALE_DNA_DISCOVER'].filter(k => inherited[k] !== undefined).map(k => [k, inherited[k]]));
  Object.assign(env, { USER: actor, LOGNAME: actor });
  const run = (command, args, options = {}) => {
    const result = spawnSync(command, args, { cwd: root, env, encoding: 'utf8', timeout: 15_000, maxBuffer: 4 * 1024 * 1024, ...options });
    assert.equal(result.status, 0, `${command}: ${result.stderr || result.stdout || result.error?.message}`);
    return result.stdout;
  };
  const git = (...args) => run('git', ['-C', root, ...args], { timeout: 5000 });
  git('init', '--quiet'); git('config', 'user.name', 'Native Task acceptance'); git('config', 'user.email', 'native-task@example.invalid');
  git('config', 'dna.principal', 'local'); git('config', 'dna.trust', 'local');
  const seed = boundedNative(binaries.seed, [root, evidence, 'seed']);
  const seedOutput = run(seed.command, seed.args); fs.writeFileSync(path.join(evidence, 'seed.log'), seedOutput);
  assert(seedOutput.includes('READY actual native Task handoffs'));
  // The actor holds the board: reassignment and retirement are the owner's
  // gated commands on the head's socket, which the face reaches forwarded.
  seatRecord(root, env, actor, ['board']);
  const fixture = JSON.parse(fs.readFileSync(path.join(evidence, 'task-fixture.json')));
  const application = fixture.application_id; let first;
  const authorityFile = path.join(evidence, 'authority.json'), taskFile = path.join(evidence, 'task-policy.json');
  fs.writeFileSync(authorityFile, JSON.stringify({ format: 'dna.practice-review-authority/1', application_id: application, grants: [{ mode: 'local', name: actor, authority: 'board', practice_propose: false, review_verdict: false, recover: true }] }));
  fs.writeFileSync(taskFile, JSON.stringify(taskPolicy(application, actor)));
  const listener = net.createServer(); await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
  const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  const origin = `http://127.0.0.1:${port}`, prefix = `/api/hale/v1/applications/${application}`;
  const processes = [], requests = []; let api, stopped = false, sequence = 0;
  // Each API start mints a launch token (GH #989): every POST carries it, and
  // every page the lane drives gets its session cookie again.
  let token = ''; const pages = new Set();
  const authorize = async page => { const opened = await page.request.get(`${origin}/?token=${token}`); assert.equal(opened.status(), 200, 'The launch token did not open the shell'); };
  const healthy = () => { assert(!stopped); if (api && !api.stopping) assert(api.child.exitCode === null && api.child.signalCode === null && !api.error, `Native Task API exited: ${api.output}`); };
  const send = (method, suffix, body) => new Promise((resolve, reject) => {
    healthy(); const data = body === undefined ? null : JSON.stringify(body); const entry = { method, path: suffix }; requests.push(entry);
    const request = http.request(origin + suffix, { method, agent: false, headers: data === null ? {} : { Origin: origin, 'X-Hale-Command': '1', 'X-Hale-Token': token, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data) } }, response => {
      let size = 0; const chunks = [];
      response.on('data', chunk => { size += chunk.length; if (size > 2 * 1024 * 1024) request.destroy(new Error('Response bound exceeded')); else chunks.push(chunk); });
      response.on('end', () => { try { const json = JSON.parse(Buffer.concat(chunks).toString('utf8')); entry.status = response.statusCode; entry.code = json.error?.code; resolve({ status: response.statusCode, json }); } catch (error) { reject(error); } }); response.on('error', reject);
    });
    request.on('error', reject); request.setTimeout(5000, () => request.destroy(new Error('Native Task request timeout'))); if (data !== null) request.write(data); request.end();
  });
  const read = suffix => send('GET', suffix);
  function journal() { const head = git('rev-parse', 'refs/dna/journal').trim(), text = git('show', `${head}:journal.jsonl`); return { head, text, rows: text.trim().split('\n').map(JSON.parse) }; }
  const signal = (item, name) => { if (!item?.child.pid) return; try { process.kill(-item.child.pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; } };
  const killOnExit = () => signal(api, 'SIGKILL'); process.on('exit', killOnExit);
  async function stopAPI() {
    if (!api) return; api.stopping = true;
    if (api.child.exitCode === null && api.child.signalCode === null) { signal(api, 'SIGTERM'); await Promise.race([api.closed, delay(1000)]); }
    if (api.child.exitCode === null && api.child.signalCode === null) { signal(api, 'SIGKILL'); await Promise.race([api.closed, delay(1000)]); }
    assert(api.child.exitCode !== null || api.child.signalCode !== null, 'Task API did not stop'); api = null;
  }
  async function startAPI() {
    assert(!api); const bounded = boundedNative(binaries.api, [root, String(port), webroot], { lock: false });
    const filename = `${++sequence}-api.log`, log = fs.createWriteStream(path.join(evidence, filename));
    const child = spawn(bounded.command, bounded.args, { cwd: root, env: { ...env, HALE_DNA_COMMAND_POLICY: authorityFile, HALE_DNA_TASK_POLICY: taskFile }, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const record = { binary: binaries.api, pid: child.pid, log: filename }; processes.push(record);
    const item = { child, output: '', stopping: false, error: null }; item.closed = new Promise(resolve => { child.once('close', (code, termination) => { record.code = code; record.signal = termination; log.end(); resolve(); }); });
    child.once('error', error => { item.error = error; record.error = error.message; });
    for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { log.write(chunk); item.output = (item.output + chunk).slice(-8192); }); api = item;
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) { try { const result = await read(prefix + '/capabilities'); if (result.status === 200) { assert.equal(result.json.data.principal.name, actor); token = await launchToken(root); for (const page of pages) await authorize(page); return result.json.data; } } catch (error) { if (!['ECONNREFUSED', 'ECONNRESET'].includes(error.code)) throw error; } await delay(50); }
    throw new Error('Native Task API startup timed out: ' + item.output);
  }
  function exportEvidence() {
    const state = journal(); fs.writeFileSync(path.join(evidence, 'journal.jsonl'), state.text);
    fs.writeFileSync(path.join(evidence, 'processes.json'), JSON.stringify({ processes, remaining: api ? [api.child.pid] : [] }, null, 2));
    fs.writeFileSync(path.join(evidence, 'requests.json'), JSON.stringify(requests, null, 2));
    fs.writeFileSync(path.join(evidence, 'service.json'), JSON.stringify({ application, origin, root, task: first?.id || fixture.first_id, policy: taskFile, binaries: Object.fromEntries(Object.entries(binaries).map(([name, binary]) => [name, { path: binary, sha256: digest(binary) }])) }, null, 2));
  }
  async function stop() { if (stopped) return; try { await stopAPI(); exportEvidence(); } finally { stopped = true; process.removeListener('exit', killOnExit); } }
  try {
    const capabilities = await startAPI();
    const initial = await read(prefix + "/dna/tasks?id=" + encodeURIComponent(fixture.first_id)); assert.equal(initial.status, 200, JSON.stringify(initial)); first = initial.json.data.items[0]; assert(first?.state === "handed");
    fs.writeFileSync(path.join(evidence, "task-response.json"), JSON.stringify(initial.json, null, 2)); exportEvidence();
    return { application, origin, root, evidence, prefix, task: first.id, initial: first, principal: { mode: 'local', name: actor }, capabilities, read,
      // One line of the head's wire (an envelope-shaped command is sent as
      // its call), settled to the receipt or the refusal it earned.
      post: async command => { const result = await send('POST', prefix + '/commands', command.call ? command : wireLine(command)); return { ...result, ...settle(result.status, result.json) }; },
      lookup: async request_id => { const result = await read(prefix + '/commands?' + new URLSearchParams({ request_id })); return { ...result, ...settle(result.status, result.json) }; },
      unseat: () => unseatRecord(root, env, actor, ['board']),
      command: (row, to, request_id) => ({ request_id, operation: 'dna.task.reassign', operation_version: '1', context: { application_id: application, position_id: 'org' }, target: { application_id: application, kind: 'dna.task', id: row.id }, preconditions: { subject_digest: row.assignment_digest, principal: { mode: 'local', name: actor }, assignee: row.assignee }, arguments: { to } }),
      current: async (id = first.id) => { const response = await read(prefix + '/dna/tasks?id=' + encodeURIComponent(id)); assert.equal(response.status, 200, JSON.stringify(response)); return response.json.data.items[0]; },
      url: () => origin + '/#/tasks?' + new URLSearchParams({ app: application, id: first.id }),
      // A page this lane drives: the session cookie now and after every restart.
      attach: async page => { pages.add(page); await authorize(page); },
      restart: async () => { await stopAPI(); return startAPI(); }, stop, exportEvidence, journal, requests,
    };
  } catch (error) { await stop().catch(() => {}); throw error; }
}
