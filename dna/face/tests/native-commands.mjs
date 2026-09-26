// Opt-in real native Record/body/host acceptance. This script never writes an
// outcome, mocks an HTTP response, builds a binary, or republishes a request.
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
import { settle, wireLine } from './command-wire.mjs';
import { mapPeer, seatRecord } from './record-seats.mjs';

// GH #1029: dna/api/practice_review/tests/relay was removed with the
// membrane (GH #986); nothing builds a relay against the nerves yet, so
// this harness — which needs one — cannot run.
assert.fail('The native command relay lane is unported (GH #1029): dna/api/practice_review/tests/relay was removed with the membrane; rebuild it against the node before running this harness.');
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const webroot = path.join(repo, 'dna/face/web');
const required = ['API', 'BODY', 'RELAY'];
const binaries = Object.fromEntries(required.map(name => {
  const value = process.env[`HALE_NATIVE_COMMAND_${name}`];
  assert(value && path.isAbsolute(value), `HALE_NATIVE_COMMAND_${name} must name an explicit absolute native binary`);
  fs.accessSync(value, fs.constants.X_OK);
  return [name.toLowerCase(), fs.realpathSync(value)];
}));
assert.equal(process.platform, 'linux', 'This acceptance harness requires Linux process limits and process-group signals.');
const parent = process.env.HALE_NATIVE_COMMAND_EVIDENCE || os.tmpdir();
assert(path.isAbsolute(parent), 'HALE_NATIVE_COMMAND_EVIDENCE must be absolute');
fs.mkdirSync(parent, { recursive: true });
const evidence = fs.mkdtempSync(path.join(parent, 'native-commands-'));
const fixture = path.join(evidence, 'project');
fs.mkdirSync(path.join(fixture, '.hale/dna'), { recursive: true });
const policyPath = path.join(evidence, 'authority.json');
const inherited = isolatedEnvironment();
// Keep only ordinary process and isolated Git settings. No inherited bus,
// observation, model, signing, OIDC, or native service configuration is forwarded.
const childEnv = Object.fromEntries([
  'PATH', 'HOME', 'LANG', 'LC_ALL', 'TZ', 'GIT_CONFIG_NOSYSTEM',
  'GIT_CONFIG_GLOBAL', 'GIT_TERMINAL_PROMPT', 'HALE_DNA_DISCOVER',
].filter(key => inherited[key] !== undefined).map(key => [key, inherited[key]]));
const owned = new Set();
const processes = [];
const requests = [];
const receipts = {};
const cases = [];
const started = Date.now();
let application = '', origin = '', actor = 'alice';
let body, relay, api, interrupted = '';
let serial = 0;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n');
const git = (...args) => execFileSync('git', ['-C', fixture, ...args], {
  env: childEnv, encoding: 'utf8', timeout: 5000, maxBuffer: 16 * 1024 * 1024,
  stdio: ['ignore', 'pipe', 'pipe'],
});
const alive = item => item && item.child.pid && !item.startError && item.child.exitCode === null && item.child.signalCode === null;
function signal(item, name) {
  // A relay subprocess may outlive its leader. Its group stays owned until
  // cleanup, even after the leader exits, so it must still receive signals.
  if (!item || !owned.has(item) || !item.child.pid) return;
  try { process.kill(-item.child.pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; }
}
function checkProcesses() {
  if (interrupted) throw new Error(interrupted);
  if (Date.now() - started > 300_000) throw new Error('Native command acceptance exceeded its 300 second wall budget');
  for (const item of owned) {
    if (!item.stopping && !alive(item)) throw new Error(`${item.name} exited: ${item.child.exitCode ?? item.child.signalCode}\n${item.output.slice(-4000)}`);
  }
}
async function until(label, read, predicate, timeout = 25_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    checkProcesses();
    last = await read();
    if (predicate(last)) return last;
    await pause(100);
  }
  throw new Error(`${label} did not complete: ${JSON.stringify(last)}`);
}
function startNative(name, binary, args = [], env = {}) {
  const bounded = boundedNative(binary, args, { lock: false });
  const filename = `${String(++serial).padStart(2, '0')}-${name}.log`;
  const log = fs.createWriteStream(path.join(evidence, filename));
  const child = spawn(bounded.command, bounded.args, {
    cwd: fixture, env: { ...childEnv, USER: actor, LOGNAME: actor, ...env },
    detached: true, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const item = { name, child, output: '', log, filename, stopping: false, paused: false, startError: null };
  let closed;
  item.closed = new Promise(resolve => { closed = resolve; });
  owned.add(item);
  const record = { name, pid: child.pid, binary, args, log: filename, started_at: new Date().toISOString() };
  processes.push(record);
  for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => {
    log.write(chunk);
    item.output = (item.output + chunk.toString('utf8')).slice(-128 * 1024);
  });
  child.on('error', error => { item.startError = error; item.output += `\n${error.stack}\n`; record.error = error.message; });
  child.on('close', (code, termination) => {
    record.exit_code = code; record.signal = termination; record.finished_at = new Date().toISOString();
    log.end();
    closed();
  });
  return item;
}
async function stop(item, kill = false) {
  if (!item || !owned.has(item)) return;
  item.stopping = true;
  if (alive(item)) {
    // Never resume a deliberately stopped body when simulating a lost reply.
    if (kill) signal(item, 'SIGKILL');
    else { if (item.paused) signal(item, 'SIGCONT'); signal(item, 'SIGTERM'); }
    await Promise.race([item.closed, pause(1500)]);
    if (alive(item)) { signal(item, 'SIGKILL'); await Promise.race([item.closed, pause(1500)]); }
  }
  // Reap any still-running Git child in the same owned group.
  signal(item, 'SIGKILL');
  assert(!alive(item), `Owned ${item.name} process did not terminate`);
  owned.delete(item);
}
// Abrupt Node exit still only touches detached process groups this script owns.
process.on('exit', () => { for (const item of owned) signal(item, 'SIGKILL'); });
for (const name of ['SIGINT', 'SIGTERM']) process.on(name, () => { interrupted = `Interrupted by ${name}`; for (const item of owned) signal(item, 'SIGKILL'); });

async function availablePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const port = server.address().port;
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  return port;
}
function httpRequest(method, suffix, value, { discard = false } = {}) {
  checkProcesses();
  const encoded = value === undefined ? undefined : Buffer.from(JSON.stringify(value), 'utf8');
  const entry = { method, path: suffix, actor, request_id: value?.request_id, started_at: new Date().toISOString(), discarded: discard };
  requests.push(entry);
  return new Promise((resolve, reject) => {
    const request = http.request(origin + suffix, {
      method, agent: false,
      headers: encoded ? {
        Origin: origin, 'Content-Type': 'application/json', 'X-Hale-Command': '1',
        'Content-Length': encoded.length, Connection: 'close',
      } : { Connection: 'close' },
    }, response => {
      if (discard) {
        // Deliberately consume no status/receipt. Completion merely proves the
        // client released its connection; recovery below is GET-only.
        response.resume();
        response.on('end', () => { entry.completed = true; resolve(null); });
        response.on('error', reject);
        return;
      }
      const chunks = []; let bytes = 0;
      response.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > 2 * 1024 * 1024) { request.destroy(new Error('HTTP response exceeded harness bound')); return; }
        chunks.push(chunk);
      });
      response.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json;
        try { json = JSON.parse(text); } catch { reject(new Error(`Non-JSON HTTP ${response.statusCode}: ${text.slice(0, 1000)}`)); return; }
        entry.status = response.statusCode; entry.error = json.error?.code;
        resolve({ status: response.statusCode, json });
      });
      response.on('error', reject);
    });
    request.setTimeout(12_000, () => request.destroy(new Error(`${method} ${suffix} timed out`)));
    request.on('error', error => { entry.transport_error = error.message; reject(error); });
    request.end(encoded);
  });
}
const prefix = () => `/api/hale/v1/applications/${encodeURIComponent(application)}`;
function transientRead(result) {
  // This helper is used only by GET. A moving captured Record is safe to read
  // again; request_conflict, stale_subject and other semantic 409s are not.
  if (result.status === 409) return result.json.error?.code === 'snapshot_changed' && result.json.error?.retryable === true;
  return [501, 503].includes(result.status) && (
    result.json.error?.retryable === true || ['snapshot_changed', 'record_unavailable', 'commands_unavailable'].includes(result.json.error?.code)
  );
}
async function read(suffix) {
  return until('native source snapshot', () => httpRequest('GET', suffix), result => !transientRead(result));
}
async function startBody() {
  body = startNative('body', binaries.body);
  return until('body bootstrap', () => {
    // Native JSON builders may print one object across several lines.
    const matches = body.output.match(/\{[^{}]*"ready"\s*:\s*true[^{}]*\}/g) || [];
    return matches.length ? JSON.parse(matches.at(-1)) : null;
  }, value => value?.ready === true);
}
async function startRelay() {
  relay = startNative('relay', binaries.relay, []);
  await until('host relay startup', () => relay.output, text => text.includes('native command relay ready'));
}
// The API forwards commands as its own uid, which the record maps to the
// acting person. A granted actor holds the board and the reviewer seat so
// the gates open; an ungranted one is mapped but seated nowhere.
const seated = new Set();
function actAs(name, seat) {
  if (!seat || seated.has(name)) mapPeer(fixture, childEnv, name);
  else { seatRecord(fixture, childEnv, name, ['board', 'reviewer']); seated.add(name); }
}
async function startApi(name = 'alice') {
  actor = name; actAs(name, name !== 'mallory');
  const port = await availablePort(); origin = `http://127.0.0.1:${port}`;
  api = startNative(`api-${actor}`, binaries.api, [fixture, String(port), webroot], { HALE_DNA_COMMAND_POLICY: policyPath });
  await until('command API startup', async () => {
    try { return await httpRequest('GET', '/api/hale/v1/applications'); }
    catch (error) { if (error.code === 'ECONNREFUSED' || error.code === 'ECONNRESET') return null; throw error; }
  }, result => result?.status === 200 && result.json.source?.record_id === application);
  const capabilities = await read(prefix() + '/capabilities');
  assert.equal(capabilities.status, 200);
  assert.deepEqual(capabilities.json.data.principal, { mode: 'local', name });
  return capabilities.json.data;
}
async function switchActor(name) { await stop(api); return startApi(name); }
function journal() {
  const head = git('rev-parse', 'refs/dna/journal').trim();
  const text = git('show', `${head}:journal.jsonl`);
  const rows = text.trim().split('\n').filter(Boolean).map(line => {
    const row = JSON.parse(line);
    let data = null;
    try { data = JSON.parse(row.body); } catch { /* Native settled prose is not JSON. */ }
    return { ...row, data };
  });
  return { head, text, rows };
}
function facts(kind, entity, rows = journal().rows) { return rows.filter(row => row.kind === kind && (entity === undefined || row.entity === entity)); }
function only(kind, entity) {
  const rows = facts(kind, entity);
  assert.equal(rows.length, 1, `Expected one ${kind} fact for ${entity}`);
  return rows[0];
}
function admission(requestId, principal = actor) {
  return journal().rows.filter(row => {
    if (!['practice.requested', 'review.verdict'].includes(row.kind) || !row.data?.command_payload) return false;
    const command = JSON.parse(row.data.command_payload);
    return command.request_id === requestId && command.principal_name === principal;
  });
}
function practice(requestId, base, text, rationale = 'Native acceptance replacement.') {
  return {
    request_id: requestId, operation: 'dna.practice.propose', operation_version: '1',
    context: { application_id: application, position_id: 'org' },
    target: { application_id: application, kind: 'dna.practice', id: base },
    preconditions: { subject_digest: base, principal: { mode: 'local', name: actor } },
    arguments: { text, rationale },
  };
}
function verdict(requestId, proposal, comment = 'Approve this exact candidate.', value = 'approve') {
  return {
    request_id: requestId, operation: 'dna.review.verdict', operation_version: '1',
    context: { application_id: application, position_id: 'org' },
    target: { application_id: application, kind: 'dna.review', id: proposal.proposal.review_id },
    preconditions: { subject_digest: proposal.proposal.candidate_digest, principal: { mode: 'local', name: actor }, review_state: 'pending' },
    arguments: { verdict: value, comment },
  };
}
function assertReceipt(receipt, command) {
  assert.equal(receipt.request_id, command.request_id);
  assert.equal(receipt.application_id, application);
  assert.equal(receipt.operation, command.operation);
  assert.equal(receipt.operation_version, '1');
  assert.deepEqual(receipt.principal, command.preconditions.principal);
  assert.deepEqual(receipt.target, command.target);
  assert.deepEqual(receipt.context, command.context);
  assert.equal(receipt.subject_digest, command.preconditions.subject_digest);
  assert.match(receipt.command_id, /^command-[0-9a-f]{64}$/);
  assert.match(receipt.fingerprint, /^sha256:[0-9a-f]{64}$/);
}
// One line of the head's wire, and the receipt it settles to.
async function submit(command) {
  const result = await httpRequest('POST', prefix() + '/commands', wireLine(command));
  const settled = settle(result.status, result.json);
  assert.equal(settled.code, '', `POST ${command.request_id}: ${JSON.stringify(result)}`);
  assert.equal(settled.reply.application_id, application);
  assertReceipt(settled.receipt, command);
  return settled.receipt;
}
async function lookup(command, predicate) {
  const result = await until(`outcome ${command.request_id}`, async () => {
    const response = await read(prefix() + '/commands?request_id=' + encodeURIComponent(command.request_id));
    const settled = settle(response.status, response.json);
    assert.equal(settled.code, '', JSON.stringify(response));
    assert.equal(settled.reply.application_id, application);
    assertReceipt(settled.receipt, command);
    return settled.receipt;
  }, predicate);
  receipts[`${command.preconditions.principal.name}:${command.request_id}`] = result;
  return result;
}
async function created(command) {
  await submit(command);
  const receipt = await lookup(command, value => value.proposal?.state === 'created');
  assert.equal(receipt.state, 'succeeded');
  assert.equal(receipt.review.subject_digest, receipt.proposal.candidate_digest);
  const proposed = only('practice.proposed', receipt.command_id);
  assert.equal(proposed.data.digest, receipt.proposal.candidate_digest);
  assert.equal(proposed.data.review_id, receipt.proposal.review_id);
  assert.equal(proposed.data.because, command.arguments.rationale);
  const reviewRows = facts('review.requested').filter(row => row.entity === receipt.proposal.review_id || row.entity === 'review:' + receipt.proposal.review_id);
  assert.equal(reviewRows.length, 1);
  assert.equal(reviewRows[0].data.subject_digest, receipt.proposal.candidate_digest);
  assert.equal(reviewRows[0].data.knowledge_digest, receipt.proposal.candidate_digest);
  assertCandidate(receipt, command);
  return receipt;
}
function assertCandidate(receipt, command) {
  const digest = receipt.proposal.candidate_digest;
  assert.match(digest, /^sha256:[0-9a-f]{64}$/);
  const document = git('cat-file', '-p', 'refs/dna/receipts/' + digest.slice(7));
  assert.equal('sha256:' + createHash('sha256').update(document, 'utf8').digest('hex'), digest);
  const value = JSON.parse(document);
  assert.equal(value.text, command.arguments.text);
  assert.equal(value.supersedes, command.preconditions.subject_digest);
  assert.equal(value.author, 'org'); assert.equal(value.target, 'org');
  assert.equal(value.kind, 'practice'); assert.equal(value.name, 'practice/receipts');
  fs.writeFileSync(path.join(evidence, `candidate-${digest.slice(7)}.json`), document);
}
function assertAdoption(receipt, command, predecessor) {
  assert.equal(receipt.state, 'succeeded'); assert.equal(receipt.verdict.state, 'accepted');
  assert.equal(receipt.review.state, 'settled'); assert.equal(receipt.review.outcome, 'approve');
  assert.equal(receipt.activation.state, 'adopted');
  const decision = only('review.command_decided', receipt.command_id);
  assert.equal(decision.data.command_id, receipt.command_id);
  assert.equal(decision.data.accepted, true); assert.equal(decision.data.settled, true);
  assert.equal(decision.data.review_id, command.target.id);
  assert.equal(decision.data.subject_digest, command.preconditions.subject_digest);
  assert.equal(decision.data.reviewer, command.preconditions.principal.name);
  assert.equal(decision.data.authority, 'board'); assert.equal(decision.data.comment, command.arguments.comment);
  assert.equal(only('knowledge.ratified', command.preconditions.subject_digest).data.review_id, command.target.id);
  assert.equal(only('knowledge.retired', predecessor).data.by, command.preconditions.subject_digest);
}
async function approve(command, predecessor) {
  await submit(command);
  const receipt = await lookup(command, value => value.activation?.state === 'adopted');
  assertAdoption(receipt, command, predecessor);
  return receipt;
}
// A refusal: the route's own keeps its HTTP status and error envelope; the
// binding's is its refusal kind; the provider's is the CommandReply code
// under a 200.
async function expectError(method, suffix, command, status, code) {
  const response = method === 'GET' ? await read(suffix) : await httpRequest(method, suffix, wireLine(command));
  const settled = settle(response.status, response.json);
  assert.equal(response.status, status, JSON.stringify(response));
  assert.equal(settled.code, code, JSON.stringify(response));
}
async function quiesce() {
  let previous = journal().head, stable = 0;
  await until('quiescent native journal', async () => {
    await pause(100); const head = journal().head;
    stable = head === previous ? stable + 1 : 0; previous = head; return stable;
  }, count => count >= 4);
}
async function stopDelivery() {
  await quiesce();
  for (const item of [relay, body]) { signal(item, 'SIGSTOP'); item.paused = true; }
  for (const item of [relay, body]) await until(`stopped ${item.name}`, () => fs.readFileSync(`/proc/${item.child.pid}/status`, 'utf8'), text => /^State:\s+T/m.test(text));
}
function resumeDelivery() {
  for (const item of [body, relay]) { signal(item, 'SIGCONT'); item.paused = false; }
}
async function scenario(name, run) {
  const beginning = Date.now();
  await run();
  cases.push({ name, passed: true, elapsed_ms: Date.now() - beginning, record_head: journal().head });
  process.stderr.write(`PASS ${name}\n`);
}

let failure;
try {
  git('init', '--quiet');
  git('config', 'user.name', 'Native command acceptance');
  git('config', 'user.email', 'native-command-acceptance@example.invalid');
  git('config', 'dna.trust', 'local'); git('config', 'dna.principal', 'local');
  const ready = await startBody();
  application = git('rev-list', '--max-parents=0', 'refs/dna/journal').trim();
  assert.match(application, /^[0-9a-f]{40,64}$/);
  assert.equal(ready.practice_id, ready.bootstrap_digest);
  save('authority.json', {
    format: 'dna.practice-review-authority/1', application_id: application,
    grants: ['alice', 'bob'].map(name => ({ mode: 'local', name, authority: 'board', practice_propose: true, review_verdict: true, recover: true })),
  });
  await startRelay();
  const capabilities = await startApi();
  assert.equal(capabilities.writes.practice_propose, true); assert.equal(capabilities.writes.review_verdict, true);
  let active = ready.practice_id, firstProposal, firstCommand;
  const literal = 'Collect the exact receipt.\r\nKeep café / 東京 / 🧭.\r\nControl: \u0001; literal <script> & "quotes".\r\n';
  const rationale = 'Preserve CRLF\r\nUnicode 🧭 and control \u0001 exactly.';
  const comment = 'Approve this exact text.\r\nVérifié 東京 🧭 \u0001.';

  await scenario('replacement → real candidate Review → exact approval → adoption', async () => {
    firstCommand = practice('replace-1', active, literal, rationale);
    firstProposal = await created(firstCommand);
    const fact = only('practice.requested', firstProposal.command_id);
    assert.equal(fact.data.text, literal); assert.equal(fact.data.because, rationale);
    assert.equal(fact.author, 'alice'); assert.equal(fact.data.by, 'alice');
    const approval = verdict('approve-1', firstProposal, comment);
    await approve(approval, active);
    assert.equal(only('review.verdict', approval.target.id).data.comment, comment);
    active = firstProposal.proposal.candidate_digest;
  });
  await scenario('identical retry, changed content, and cross-operation request conflict', async () => {
    const retry = await submit(firstCommand);
    assert.equal(retry.command_id, firstProposal.command_id); assert.equal(retry.fingerprint, firstProposal.fingerprint);
    const changed = structuredClone(firstCommand); changed.arguments.text += 'Changed';
    await expectError('POST', prefix() + '/commands', changed, 200, 'request_conflict');
    await expectError('POST', prefix() + '/commands', verdict('replace-1', firstProposal), 200, 'request_conflict');
    assert.equal(admission('replace-1').length, 1);
  });
  let lostCommand, lostProposal;
  await scenario('discarded POST reply recovers by GET across API and body restart', async () => {
    await stopDelivery();
    lostCommand = practice('lost-proposal', active, 'Recovered after API and body restart.\r\n🧭 \u0001');
    save('lost-request.json', { application_id: application, principal: lostCommand.preconditions.principal, request_id: lostCommand.request_id });
    await httpRequest('POST', prefix() + '/commands', wireLine(lostCommand), { discard: true });
    const admitted = admission(lostCommand.request_id);
    assert.equal(admitted.length, 1); assert.equal(facts('practice.proposed', admitted[0].entity).length, 0);
    const posts = requests.filter(row => row.method === 'POST').length;
    await stop(api); await stop(relay, true); await stop(body, true);
    const restarted = await startBody(); assert.equal(restarted.practice_id, active);
    await startRelay(); await startApi();
    lostProposal = await lookup(lostCommand, value => value.proposal?.state === 'created');
    assertCandidate(lostProposal, lostCommand);
    assert.equal(requests.filter(row => row.method === 'POST').length, posts, 'Recovery must never resubmit');
    assert.equal(admission(lostCommand.request_id).length, 1);
    assert.equal(lostProposal.command_id, admitted[0].entity);
    assert.equal(lostProposal.fingerprint, admitted[0].data.command_fingerprint);
  });
  await scenario('two admitted verdicts: first accepted, second refused beside approval', async () => {
    await stopDelivery();
    const first = verdict('approve-race-first', lostProposal), second = verdict('approve-race-second', lostProposal);
    const firstRecorded = await submit(first), secondRecorded = await submit(second);
    assert.equal(firstRecorded.state, 'recorded'); assert.equal(secondRecorded.state, 'recorded');
    assert.equal(facts('review.command_decided', firstRecorded.command_id).length, 0);
    assert.equal(facts('review.command_decided', secondRecorded.command_id).length, 0);
    resumeDelivery();
    const accepted = await lookup(first, value => value.activation?.state === 'adopted');
    assertAdoption(accepted, first, active);
    const refused = await lookup(second, value => value.verdict?.state === 'refused');
    assert.equal(refused.state, 'refused'); assert.equal(refused.review.outcome, 'approve');
    assert.equal(refused.activation.state, 'unknown');
    assert.equal(only('review.command_decided', refused.command_id).data.accepted, false);
    assert(Number(only('review.command_decided', accepted.command_id).seq) < Number(only('review.command_decided', refused.command_id).seq));
    active = lostProposal.proposal.candidate_digest;
  });
  let retiredBase;
  await scenario('two replacement candidates: later approval can refuse adoption', async () => {
    retiredBase = active;
    const a = await created(practice('competing-a', active, 'Candidate A preserves the actual predecessor.'));
    const b = await created(practice('competing-b', active, 'Candidate B shares that predecessor.'));
    await approve(verdict('competing-approve-a', a), active);
    const commandB = verdict('competing-approve-b', b);
    await submit(commandB);
    const refused = await lookup(commandB, value => value.activation?.state === 'refused');
    assert.equal(refused.verdict.state, 'accepted'); assert.equal(refused.review.outcome, 'approve');
    assert.equal(refused.state, 'succeeded');
    const outcome = only('knowledge.refused', b.proposal.candidate_digest);
    assert.equal(outcome.data.review_id, b.proposal.review_id); assert.equal(outcome.data.supersedes, active);
    assert.equal(outcome.data.retired_by, a.proposal.candidate_digest);
    assert.equal(facts('knowledge.ratified', b.proposal.candidate_digest).length, 0);
    assert.equal(only('knowledge.retired', active).data.by, a.proposal.candidate_digest);
    active = a.proposal.candidate_digest;
  });
  await scenario('new request against retired subject is not admitted', async () => {
    await expectError('POST', prefix() + '/commands', practice('stale-new-key', retiredBase, 'Stale subject.'), 200, 'stale_subject');
    assert.equal(admission('stale-new-key').length, 0);
  });
  await scenario('principal isolation survives API restart with both grants', async () => {
    await switchActor('bob');
    await expectError('GET', prefix() + '/commands?request_id=replace-1', undefined, 200, 'command_not_found');
    const command = practice('replace-1', active, 'Bob has his own request identity.');
    const proposal = await created(command);
    assert.notEqual(proposal.command_id, firstProposal.command_id);
    assert.equal(admission('replace-1', 'bob').length, 1);
    assert.equal(only('practice.requested', proposal.command_id).author, 'bob');
    await switchActor('alice');
    const original = await lookup(firstCommand, value => value.proposal?.state === 'created');
    assert.equal(original.command_id, firstProposal.command_id);
  });
  await scenario('ungranted principal cannot submit or recover another receipt', async () => {
    await switchActor('mallory');
    // Seated nowhere: the slice is the two ungated commands, a proposal is
    // unknown to this caller, and the lookup is the provider's to refuse.
    const described = await httpRequest('POST', prefix() + '/commands', { describe: true });
    assert.equal(described.status, 200, JSON.stringify(described));
    assert.deepEqual(described.json.value.commands.map(entry => entry.name).sort(), ['CommandLookup', 'TaskCreate']);
    await expectError('POST', prefix() + '/commands', practice('unauthorized', active, 'No policy grant.'), 404, 'unknown');
    await expectError('GET', prefix() + '/commands?request_id=replace-1', undefined, 200, 'forbidden');
    assert.equal(admission('unauthorized').length, 0);
  });
  await scenario('encoded HTTP cap and maximum decoded fields through native adoption', async () => {
    await switchActor('alice');
    const tooLarge = practice('encoded-control-too-large', active, '\u0001'.repeat(8192), '\u0001'.repeat(2048));
    assert(Buffer.byteLength(JSON.stringify(wireLine(tooLarge)), 'utf8') > 32768);
    await expectError('POST', prefix() + '/commands', tooLarge, 413, 'command_too_large');
    assert.equal(admission(tooLarge.request_id).length, 0);
    const text = '\u0001'.repeat(3000) + 'x'.repeat(5192);
    const reason = '\u0001'.repeat(100) + 'r'.repeat(1948);
    assert.equal(Buffer.byteLength(text, 'utf8'), 8192);
    assert.equal(Buffer.byteLength(reason, 'utf8'), 2048);
    const command = practice('max-control-replacement', active, text, reason);
    assert(Buffer.byteLength(JSON.stringify(wireLine(command)), 'utf8') <= 32768);
    const proposal = await created(command);
    const fact = only('practice.requested', proposal.command_id);
    assert.equal(fact.data.text, text); assert.equal(fact.data.because, reason);
    // created() verifies the actual receipt blob's SHA-256 and decoded text.
    // The body must also carry the same rationale into its proposal answer.
    assert.equal(only('practice.proposed', proposal.command_id).data.because, reason);
    const approval = verdict('max-control-approve', proposal, '\u0001'.repeat(2048));
    assert.equal(Buffer.byteLength(approval.arguments.comment, 'utf8'), 2048);
    assert(Buffer.byteLength(JSON.stringify(wireLine(approval)), 'utf8') <= 32768);
    await approve(approval, active);
    active = proposal.proposal.candidate_digest;
  });
} catch (error) {
  failure = error;
} finally {
  for (const item of [...owned].reverse()) {
    try { await stop(item, item.paused); } catch (error) { failure ||= error; }
  }
  let snapshot;
  try {
    snapshot = journal();
    fs.writeFileSync(path.join(evidence, 'journal.jsonl'), snapshot.text);
    fs.writeFileSync(path.join(evidence, 'refs.txt'), git('show-ref'));
  } catch (error) { failure ||= error; }
  const hashes = Object.fromEntries(Object.entries(binaries).map(([name, binary]) => [name, { path: binary, sha256: createHash('sha256').update(fs.readFileSync(binary)).digest('hex') }]));
  save('requests.json', requests); save('receipts.json', receipts); save('processes.json', processes);
  const result = {
    ok: !failure, passed: cases.length, expected: 9, cases, elapsed_ms: Date.now() - started,
    application_id: application, record_head: snapshot?.head, record_rows: snapshot?.rows.length,
    evidence, fixture, policy: policyPath, journal: path.join(evidence, 'journal.jsonl'),
    binaries: hashes, post_requests: requests.filter(row => row.method === 'POST').length,
    read_requests: requests.filter(row => row.method === 'GET').length,
    discarded_replies: requests.filter(row => row.discarded).length,
    remaining_owned_processes: owned.size,
    scope: 'real local Record/body/host command acceptance; no routing-1 or multi-clone claim',
    error: failure ? { message: failure.message, stack: failure.stack } : undefined,
  };
  save('result.json', result);
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
  if (failure) process.exitCode = 1;
}
