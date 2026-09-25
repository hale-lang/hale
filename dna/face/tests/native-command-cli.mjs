// Real source-built CLI acceptance. Only the actual Hale Body creates the
// initial Practice/Review outcomes; this script never writes domain facts.
// Run only in the serialized native gate with the supplied bounded binaries.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import { boundedNative, isolatedEnvironment } from './environment.mjs';

const runFile = promisify(execFile);
const required = name => {
  assert(process.env[name], `${name} must name the supplied native binary`);
  return resolve(process.env[name]);
};
const hostBinary = required('HALE_NATIVE_COMMAND_HOST');
const bodyBinary = required('HALE_NATIVE_COMMAND_BODY');
const apiBinary = process.env.HALE_NATIVE_COMMAND_API && resolve(process.env.HALE_NATIVE_COMMAND_API);
const actor = process.env.USER;
assert(actor && !/[\x00-\x1f\x7f]/u.test(actor), 'an explicit nonempty current USER identity is required');
const evidenceBase = resolve(process.env.HALE_NATIVE_COMMAND_EVIDENCE || join(tmpdir(), 'hale-native-command-evidence'));
await mkdir(evidenceBase, { recursive: true });
const evidence = await mkdtemp(join(evidenceBase, 'cli-'));
const root = join(evidence, 'project');
await mkdir(join(root, '.hale', 'dna'), { recursive: true });
const policyPath = join(evidence, 'authority.json');
const baseEnv = { ...isolatedEnvironment(), USER: actor, HALE_DNA_COMMAND_POLICY: policyPath };
const children = new Set();
const checks = [];
let ordinal = 0;
let summary = { passed: false, evidence, root, actor, cross_api_cli: false, checks };
const sleep = ms => new Promise(resolveSleep => setTimeout(resolveSleep, ms));
const hash = value => createHash('sha256').update(value, 'utf8').digest('hex');
function quote(value) {
  return '"' + [...value].map(c => c === '"' || c === '\\' ? '\\' + c : c.codePointAt(0) < 32 ? '\\u' + c.codePointAt(0).toString(16).padStart(4, '0') : c).join('') + '"';
}
const exactObject = object => '{' + Object.entries(object).map(([key, value]) => quote(key) + ':' + quote(value)).join(',') + '}';
const git = async (...args) => (await runFile('git', ['-C', root, ...args], { env: baseEnv, timeout: 10_000, maxBuffer: 20 * 1024 * 1024 })).stdout.trimEnd();
const refs = () => git('for-each-ref', '--format=%(refname) %(objectname)');
const journal = async () => (await git('show', 'refs/dna/journal:journal.jsonl')).split('\n').filter(Boolean).map(row => JSON.parse(row));
const bodyOf = row => JSON.parse(row.body);
const head = () => git('rev-parse', 'refs/dna/journal');

function start(label, binary, args = []) {
  const bounded = boundedNative(binary, args, { lock: false });
  const child = spawn(bounded.command, bounded.args, { cwd: root, env: baseEnv, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
  child.output = ''; child.errors = ''; child.startError = null; child.label = label;
  child.stdout.on('data', chunk => { child.output = (child.output + chunk).slice(-2 * 1024 * 1024); });
  child.stderr.on('data', chunk => { child.errors = (child.errors + chunk).slice(-2 * 1024 * 1024); });
  child.once('error', error => { child.startError = error; });
  children.add(child);
  return child;
}
async function stop(child) {
  if (!child) return;
  const alive = child.exitCode === null && !child.signalCode && !child.startError;
  if (alive) {
    const closed = new Promise(resolveClose => child.once('close', resolveClose));
    const kill = signal => { try { process.platform === 'win32' ? child.kill(signal) : process.kill(-child.pid, signal); } catch {} };
    kill('SIGTERM');
    const timer = setTimeout(() => kill('SIGKILL'), 1500);
    await closed; clearTimeout(timer);
  }
  await writeFile(join(evidence, child.label + '.log'), child.output + '\nSTDERR\n' + child.errors);
  children.delete(child);
}
async function until(label, fn, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await fn();
    if (value) return value;
    await sleep(50);
  }
  throw new Error(label + ' timed out');
}
async function cli(label, verb, args, { success = true, code = '', env = {} } = {}) {
  const bounded = boundedNative(hostBinary, [verb, root, '.', '-', '-', ...args], { lock: false });
  let result;
  try {
    result = { ...(await runFile(bounded.command, bounded.args, { cwd: root, env: { ...baseEnv, ...env }, timeout: 15_000, maxBuffer: 2 * 1024 * 1024 })), code: 0 };
  } catch (error) {
    result = { code: error.code, stdout: error.stdout || '', stderr: error.stderr || '', signal: error.signal, killed: error.killed };
  }
  await writeFile(join(evidence, `${String(++ordinal).padStart(2, '0')}-${label}.json`), JSON.stringify({ verb, args, ...result }, null, 2));
  assert(!result.killed && !result.signal, `${label}: native command exceeded its bound`);
  assert.equal(typeof result.code, 'number', `${label}: native command could not execute`);
  assert.equal(result.code === 0, success, `${label}: ${result.stderr || result.stdout}`);
  if (code) assert((result.stdout + result.stderr).includes(code), `${label}: expected ${code}, got ${result.stdout + result.stderr}`);
  return result;
}
async function unchanged(label, operation) {
  const before = await refs(); const value = await operation();
  assert.equal(await refs(), before, `${label}: must not change any Git ref`);
  checks.push(label); return value;
}
async function freePort() {
  const server = createServer();
  await new Promise((resolveListen, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolveListen); });
  const port = server.address().port;
  await new Promise(resolveClose => server.close(resolveClose));
  return port;
}

try {
  await git('init', '-q', '-b', 'main');
  await git('config', 'user.name', 'Native CLI acceptance');
  await git('config', 'user.email', 'native-cli@example.invalid');
  await git('config', 'commit.gpgsign', 'false');
  await git('config', 'dna.trust', 'local');
  const body = start('bootstrap-body', bodyBinary);
  const ready = await until('native Body bootstrap', () => {
    if (body.startError || body.exitCode !== null || body.signalCode) throw new Error(body.startError?.message || body.errors || 'Body exited before bootstrap');
    const text = body.output.match(/\{\s*"ready"\s*:\s*true[\s\S]*?\}/u)?.[0];
    return text ? JSON.parse(text) : null;
  });
  assert.equal(ready.practice_id, ready.bootstrap_digest, 'actual native bootstrap Practice must be active');
  await stop(body);
  const application = await git('rev-list', '--max-parents=0', 'refs/dna/journal');
  summary.application_id = application;
  const baselineRows = await journal();
  const bootstrap = baselineRows.find(row => row.kind === 'practice.proposed' && row.entity === 'native-command-bootstrap');
  assert(bootstrap, 'bootstrap outcome must come from the actual Body');
  const reviewId = bodyOf(bootstrap).review_id;
  assert(reviewId && reviewId !== ready.bootstrap_digest, 'native Review identity remains separate from Practice digest');

  const policy = (changes = {}) => ({ format: 'dna.practice-review-authority/1', application_id: application,
    grants: [{ mode: 'local', name: actor, authority: 'board', practice_propose: true, review_verdict: true, recover: true, ...changes }] });
  const installPolicy = async changes => { const document = JSON.stringify(policy(changes), null, 2) + '\n'; await writeFile(policyPath, document); return document; };
  const policyDocument = await installPolicy();
  const requestId = 'cli-exact-request';
  const text = '  Exact replacement 🧭\r\nDeuxième ligne / 第二行\t"receipt" \\  ';
  const rationale = '  Keep the source bytes.\r\nCLI native acceptance.  ';
  const replacement = (id = requestId, value = text) => ['propose', 'practice/receipts', '--supersedes', ready.bootstrap_digest, '--text', value, '--because', rationale, ...(id ? ['--request-id', id] : [])];
  const lookup = id => ['lookup', '--request-id', id];
  const verdict = (id = 'cli-policy-verdict', authority = '') => [reviewId, 'approve', '--digest', ready.bootstrap_digest, '--request-id', id, '--no-wait', ...(authority ? ['--authority', authority] : [])];

  await unchanged('explicit-policy-required', () => cli('missing-policy', 'practice', replacement(), { success: false, code: 'HALE_DNA_COMMAND_POLICY', env: { HALE_DNA_COMMAND_POLICY: '' } }));
  await unchanged('as-cannot-replace-actor', () => cli('wrong-as', 'practice', [...replacement(), '--as', actor + '-other'], { success: false, code: 'command_context_changed' }));
  await unchanged('authority-cannot-grant-role', () => cli('wrong-authority', 'verdict', verdict('cli-policy-verdict', 'reviewer'), { success: false, code: 'forbidden' }));
  await unchanged('unmapped-local-principal-denied', () => cli('unmapped-actor', 'practice', replacement(), { success: false, code: 'forbidden', env: { USER: actor + '-unmapped' } }));
  await installPolicy({ practice_propose: false });
  await unchanged('current-write-policy-enforced', () => cli('write-denied', 'practice', replacement(), { success: false, code: 'forbidden' }));
  await writeFile(policyPath, '{"format":"dna.practice-review-authority/1"}');
  await unchanged('malformed-policy-fails-closed', () => cli('malformed-policy', 'practice', replacement(), { success: false, code: 'invalid authority policy' }));
  await writeFile(policyPath, JSON.stringify({ ...policy(), application_id: application + '-other' }));
  await unchanged('wrong-application-policy-denied', () => cli('wrong-policy-application', 'practice', replacement(), { success: false, code: 'invalid authority policy' }));
  await installPolicy();

  const predecessorHead = await head();
  const admitted = await cli('no-body-admission', 'practice', replacement());
  assert(admitted.stderr.includes('hale dna command request_id: ' + requestId), 'request key is printed before native admission');
  assert(admitted.stdout.includes('dna.practice.propose: recorded'), 'without a Body, submission must report recorded rather than fabricated progression');
  const key = exactObject({ format: 'dna.governance-command-key/1', application_id: application, principal_mode: 'local', principal_name: actor, request_id: requestId });
  const commandId = 'command-' + hash(key);
  const typed = { format: 'dna.governance-command/1', application_id: application, principal_mode: 'local', principal_name: actor,
    request_id: requestId, operation: 'dna.practice.propose', operation_version: '1', position_id: 'org', target_kind: 'dna.practice',
    target_id: ready.bootstrap_digest, subject_digest: ready.bootstrap_digest, text, rationale, expected_review_state: '', verdict: '', comment: '' };
  const canonical = exactObject(typed);
  const rows = await journal();
  const commands = rows.filter(row => row.kind === 'practice.requested' && row.entity === commandId);
  assert.equal(commands.length, 1, 'one real native admission fact');
  const fact = bodyOf(commands[0]);
  assert.equal(commands[0].author, actor);
  assert.equal(fact.command_id, commandId);
  assert.equal(fact.request_id, commandId, 'domain trigger correlates through the durable command identity');
  assert.equal(fact.command_payload, canonical, 'native canonical payload preserves exact text and context');
  assert.equal(fact.command_fingerprint, 'sha256:' + hash(canonical));
  assert.equal(fact.command_record_head, predecessorHead, 'admission binds its exact checked predecessor');
  assert.equal(fact.command_authority_basis, 'dna.practice-review-authority/1:sha256:' + hash(policyDocument) + ':trust=local');
  assert.equal(fact.text, text); assert.equal(fact.because, rationale); assert.equal(fact.by, actor);
  assert.equal(rows.length, baselineRows.length + 1, 'CLI admission creates no second marker or fake outcome');
  checks.push('no-body-native-admission', 'exact-canonical-identity-and-text');

  const replay = await unchanged('same-id-retry-is-read-only', () => cli('exact-retry', 'practice', replacement()));
  assert(replay.stdout.includes('command_id: ' + commandId));
  await unchanged('different-content-conflicts', () => cli('content-conflict', 'practice', replacement(requestId, text + '!'), { success: false, code: 'request_conflict' }));
  await unchanged('cross-operation-id-conflicts', () => cli('operation-conflict', 'verdict', verdict(requestId), { success: false, code: 'request_conflict' }));
  const recovered = await unchanged('lookup-is-read-only', () => cli('lookup', 'practice', lookup(requestId)));
  assert(recovered.stdout.includes('command_id: ' + commandId));
  await unchanged('missing-lookup-is-read-only', () => cli('missing-lookup', 'practice', lookup('no-such-command'), { success: false, code: 'command_not_found' }));
  await installPolicy({ recover: false });
  await unchanged('current-recovery-policy-enforced', () => cli('recovery-denied', 'practice', lookup(requestId), { success: false, code: 'forbidden' }));
  await installPolicy({ practice_propose: false, review_verdict: false });
  await unchanged('read-only-principal-can-recover', () => cli('recovery-only', 'practice', lookup(requestId)));
  await installPolicy();
  const generatedA = await cli('generated-a', 'practice', replacement(''));
  const generatedB = await cli('generated-b', 'practice', replacement(''));
  const generatedId = value => value.stdout.match(/^request_id: (cli-\d+-\d+-\d+)$/mu)?.[1];
  assert(generatedId(generatedA) && generatedId(generatedB));
  assert.notEqual(generatedId(generatedA), generatedId(generatedB), 'omitted request IDs must not collide across processes');
  checks.push('generated-identities-distinct');

  if (apiBinary) {
    const port = await freePort(); const origin = `http://127.0.0.1:${port}`;
    const api = start('api', apiBinary, [root, String(port)]);
    const endpoint = `${origin}/api/hale/v1/applications/${application}/commands`;
    await until('native command API', async () => {
      if (api.startError || api.exitCode !== null || api.signalCode) throw new Error(api.startError?.message || api.errors || 'API exited');
      try { return (await fetch(`${origin}/api/hale/v1/applications`, { signal: AbortSignal.timeout(1000) })).ok; } catch { return false; }
    });
    const get = async id => {
      const response = await fetch(endpoint + '?' + new URLSearchParams({ request_id: id }), { signal: AbortSignal.timeout(10_000) });
      const result = await response.json(); assert.equal(response.status, 200, JSON.stringify(result)); return result;
    };
    const apiReceipt = await unchanged('api-recovers-cli-command', () => get(requestId));
    assert.equal(apiReceipt.data.command_id, commandId); assert.equal(apiReceipt.data.fingerprint, fact.command_fingerprint);
    assert.equal(apiReceipt.data.principal.mode, 'local'); assert.equal(apiReceipt.data.principal.name, actor);
    await writeFile(join(evidence, 'api-recovered-cli.json'), JSON.stringify(apiReceipt, null, 2));
    const apiRequest = { request_id: 'api-to-cli-request', operation: 'dna.practice.propose', operation_version: '1',
      context: { application_id: application, position_id: 'org' }, target: { application_id: application, kind: 'dna.practice', id: ready.bootstrap_digest },
      preconditions: { subject_digest: ready.bootstrap_digest, principal: { mode: 'local', name: actor } }, arguments: { text, rationale } };
    const response = await fetch(endpoint, { method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1' }, body: JSON.stringify(apiRequest), signal: AbortSignal.timeout(10_000) });
    const apiResult = await response.json(); assert.equal(response.status, 202, JSON.stringify(apiResult));
    assert.equal(apiResult.data.state, 'recorded');
    const cliReceipt = await unchanged('cli-recovers-api-command', () => cli('recover-api-command', 'practice', lookup(apiRequest.request_id)));
    assert(cliReceipt.stdout.includes('command_id: ' + apiResult.data.command_id));
    await unchanged('cli-replays-api-command-equivalently', () => cli('replay-api-command', 'practice', replacement(apiRequest.request_id)));
    const apiFact = (await journal()).find(row => row.kind === 'practice.requested' && row.entity === apiResult.data.command_id);
    assert(apiFact); assert.equal(bodyOf(apiFact).command_fingerprint, apiResult.data.fingerprint);
    assert.equal(bodyOf(apiFact).command_payload, exactObject({ ...typed, request_id: apiRequest.request_id }));
    await writeFile(join(evidence, 'api-created-command.json'), JSON.stringify(apiResult, null, 2));
    await stop(api); summary.cross_api_cli = true;
  }
  summary = { ...summary, passed: true, command_id: commandId, fingerprint: fact.command_fingerprint, checks };
} catch (error) {
  summary = { ...summary, error: error.stack || String(error), checks };
  process.exitCode = 1;
} finally {
  for (const child of [...children]) await stop(child);
  await writeFile(join(evidence, 'summary.json'), JSON.stringify(summary, null, 2));
  console.log(JSON.stringify(summary, null, 2));
}
