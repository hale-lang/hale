// Actual Host.run lifecycle, actual native source/Review/apply setup, and actual
// observer process join. The only faults are named git command replies in the
// disposable repository; no source, Review, application, or launch fact is seeded.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';

assert.equal(process.env.HALE_HOST_LAUNCH_NAMESPACE, 'private', 'run through the private PID/IPC/mount namespace wrapper');
const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../../..');
const evidence = process.env.HALE_HOST_LAUNCH_EVIDENCE;
const setupBin = process.env.HALE_HOST_LAUNCH_SETUP_BIN;
const hostBin = process.env.HALE_HOST_LAUNCH_BIN;
const membraneBin = process.env.HALE_HOST_LAUNCH_MEMBRANE_BIN;
const observerBin = process.env.HALE_HOST_LAUNCH_OBSERVER_BIN;
const apiBin = process.env.HALE_HOST_LAUNCH_API_BIN;
for (const value of [evidence, setupBin, hostBin, membraneBin, observerBin]) assert.ok(value, 'all explicit native artifact paths are required');
await fs.mkdir(evidence, { recursive: true });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = data => 'sha256:' + crypto.createHash('sha256').update(data).digest('hex');
const modes = process.argv.slice(2);
assert.ok(modes.length > 0, 'name the bounded cases to run');

function command(bin, args, cwd, env) {
  const r = spawnSync('/usr/bin/timeout', ['45s', '/usr/bin/prlimit', '--as=536870912:536870912', '--core=0:0', '--cpu=30:30', '--', bin, ...args], { cwd, env, encoding: 'utf8', maxBuffer: 2 * 1024 * 1024 });
  assert.equal(r.status, 0, `${path.basename(bin)} ${args[0]} failed: ${r.stderr}\n${r.stdout}`);
  return r.stdout;
}
function lastJSON(output) { return JSON.parse(output.trim().split('\n').findLast(line => line.startsWith('{'))); }
async function poll(action, label, ms = 25000) {
  const end = Date.now() + ms; let value;
  while (Date.now() < end) { value = await action(); if (value) return value; await delay(250); }
  throw new Error(`${label} timed out; last=${JSON.stringify(value)}`);
}
async function port() { const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p; }
async function run(mode) {
  assert.ok(['healthy', 'launch-ack', 'claim-reply', 'rollback-ack', 'rollback-launch-ack'].includes(mode), 'bounded known case');
  const root = await fs.mkdtemp(path.join(evidence, `${mode}-`));
  const work = path.join(root, 'project'); const control = path.join(root, 'control'); const runtime = path.join(root, 'runtime');
  await fs.mkdir(path.join(work, 'dna/org'), { recursive: true }); await fs.mkdir(path.join(work, 'app'), { recursive: true });
  await fs.mkdir(path.join(work, 'vendor/core'), { recursive: true }); await fs.mkdir(control); await fs.mkdir(runtime); await fs.mkdir(path.join(runtime, 'hale'), { mode: 0o700 });
  const baseSource = await fs.readFile(path.join(here, 'org/main.hl'), 'utf8');
  await fs.writeFile(path.join(work, 'dna/org/main.hl'), baseSource);
  await fs.writeFile(path.join(work, 'app/main.hl'), 'main locus App { run() { } }\nfn main() { App { }; }\n');
  await fs.writeFile(path.join(work, '.gitignore'), '/.hale/\n/vendor/\n/dna/org/org\n/app/app\n');
  for (const name of await fs.readdir(path.join(repo, 'dna/core'))) if (name.endsWith('.hl')) await fs.copyFile(path.join(repo, 'dna/core', name), path.join(work, 'vendor/core', name));
  const env = { ...process.env, HALE_BIN: process.env.HALE_BIN || '/home/riley/.local/bin/hale', HALE_DNA_MEMBRANE: membraneBin, HALE_DNA_DISCOVER: 'off', XDG_RUNTIME_DIR: runtime, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_TERMINAL_PROMPT: '0' };
  for (const key of Object.keys(env)) if (/^(?:GIT_(?:DIR|COMMON_DIR|WORK_TREE|INDEX_FILE|CONFIG_COUNT|CONFIG_PARAMETERS|NAMESPACE)|HALE_DNA_(?:KNOWLEDGE|OWNER|LEASE|BODY|TAPE)|LOTUS_OBS|OPENAI_API_KEY|ANTHROPIC_API_KEY)/.test(key)) delete env[key];
  command('/usr/bin/git', ['init', '-q', '-b', 'main'], work, env);
  command('/usr/bin/git', ['add', '-A'], work, env);
  command('/usr/bin/git', ['-c', 'user.name=LaunchFixture', '-c', 'user.email=launch@fixture', 'commit', '-qm', 'base'], work, env);
  const setup = command(setupBin, ['prepare', work], work, env); await fs.writeFile(path.join(root, 'setup.log'), setup); const meta = lastJSON(setup); await fs.writeFile(path.join(root, 'meta.json'), JSON.stringify(meta, null, 2));
  const binDir = path.join(root, 'bin'); await fs.mkdir(binDir);
  const buildQueue = path.join(control, 'builds'); await fs.mkdir(buildQueue);
  // Test infrastructure preserves the existing separate bounds: actual host
  // and application processes stay at 512 MiB; exact compiler build workers
  // have 2 GiB. Product invocation/arguments and compiler outputs stay native.
  const compilerWrapper = `#!/bin/sh\nif [ "$1" != build ]; then exec "$LAUNCH_REAL_COMPILER" "$@"; fi\nrequest=$(mktemp -d "$LAUNCH_BUILD_QUEUE/request.XXXXXX") || exit 1\nprintf '%s\\n' "$2" > "$request/seed"\ntouch "$request/ready"\ntries=0\nwhile [ ! -f "$request/status" ] && [ "$tries" -lt 2250 ]; do sleep 0.02; tries=$((tries + 1)); done\n[ -f "$request/status" ] || exit 124\ncat "$request/stdout"\ncat "$request/stderr" >&2\nexit "$(cat "$request/status")"\n`;
  await fs.writeFile(path.join(binDir, 'hale'), compilerWrapper, { mode: 0o755 });
  const buildChildren = new Set(); let servicing = false;
  const buildTimer = setInterval(async () => {
    if (servicing) return; servicing = true;
    try {
      for (const name of await fs.readdir(buildQueue)) {
        const request = path.join(buildQueue, name);
        if (!(await fs.stat(path.join(request, 'ready')).catch(() => null)) || await fs.stat(path.join(request, 'started')).catch(() => null)) continue;
        await fs.writeFile(path.join(request, 'started'), 'actual compiler worker');
        const seed = (await fs.readFile(path.join(request, 'seed'), 'utf8')).trimEnd();
        assert.ok(seed === path.join(work, 'dna/org') || (seed.startsWith(path.join(work, '.hale/dna/worktrees') + '/') && seed.endsWith('/dna/org')), 'exact native Organization seed only');
        const output = await fs.open(path.join(request, 'stdout'), 'w'); const error = await fs.open(path.join(request, 'stderr'), 'w');
        const build = spawn('/usr/bin/timeout', ['45s', '/usr/bin/prlimit', '--as=2147483648:2147483648', '--core=0:0', '--cpu=30:30', '--', env.HALE_BIN, 'build', seed], { cwd: work, env, stdio: ['ignore', output.fd, error.fd], detached: true });
        buildChildren.add(build);
        const status = await new Promise(resolve => { build.once('error', () => resolve(125)); build.once('close', (code, signal) => resolve(code ?? 128)); });
        buildChildren.delete(build); await output.close(); await error.close();
        await fs.writeFile(path.join(request, 'worker.json'), JSON.stringify({ compiler: env.HALE_BIN, seed, pid: build.pid, exitCode: build.exitCode, signalCode: build.signalCode, status }, null, 2));
        await fs.writeFile(path.join(request, 'status.tmp'), String(status)); await fs.rename(path.join(request, 'status.tmp'), path.join(request, 'status'));
      }
    } catch (error) { console.error('compiler worker:', error); }
    finally { servicing = false; }
  }, 50);
  // /usr/bin/git still performs every successful write. Before-commit blocking
  // and one lost update-ref reply are scoped to this exact fixture checkout.
  const wrapper = `#!/bin/sh\nif [ "$1" = -C ] && [ "$2" = "$LAUNCH_TEST_REPO" ]; then\n  if [ "$3" = update-ref ] && [ "$4" = refs/dna/journal ] && [ "$LAUNCH_TEST_MODE" = claim-reply ]; then\n    subject=$(/usr/bin/git -C "$2" show -s --format=%s "$5" 2>/dev/null)\n    case "$subject" in 'organization.source.launch_requested '*)\n      if mkdir "$LAUNCH_TEST_CONTROL/once" 2>/dev/null; then\n        /usr/bin/git "$@"; status=$?\n        [ "$status" -eq 0 ] || exit "$status"\n        echo lost-claim-reply >> "$LAUNCH_TEST_CONTROL/fault.log"\n        exit 1\n      fi;;\n    esac\n  fi\n  if [ ! -f "$LAUNCH_TEST_CONTROL/release" ]; then\n    for value in "$@"; do\n      case "$LAUNCH_TEST_MODE:$value" in\n        'launch-ack:organization.source.launched '*|'rollback-ack:mutation.rolled_back '*|'rollback-launch-ack:organization.source.rollback_launched '*)\n          echo withheld-ack >> "$LAUNCH_TEST_CONTROL/fault.log"; exit 1;;\n      esac\n    done\n  fi\nfi\nexec /usr/bin/git "$@"\n`;
  await fs.writeFile(path.join(binDir, 'git'), wrapper, { mode: 0o755 });
  const hostEnv = { ...env, HALE_BIN: path.join(binDir, 'hale'), LAUNCH_REAL_COMPILER: env.HALE_BIN, LAUNCH_BUILD_QUEUE: buildQueue, PATH: binDir + ':' + env.PATH, LAUNCH_TEST_REPO: work, LAUNCH_TEST_CONTROL: control, LAUNCH_TEST_MODE: mode };
  if (mode.startsWith('rollback')) await fs.writeFile(path.join(work, '.hale/dna/fixture-crash'), 'exit the candidate only');
  const children = []; const descriptors = [];
  async function start(name, bin, args, childEnv = hostEnv) {
    const log = await fs.open(path.join(root, name + '.log'), 'w'); descriptors.push(log);
    const child = spawn('/usr/bin/prlimit', ['--as=536870912:536870912', '--core=0:0', '--cpu=30:30', '--', bin, ...args], { cwd: work, env: childEnv, stdio: ['ignore', log.fd, log.fd], detached: true });
    children.push({ name, child }); return child;
  }
  const journal = () => {
    const r = spawnSync('/usr/bin/git', ['show', 'refs/dna/journal:journal.jsonl'], { cwd: work, encoding: 'utf8', maxBuffer: 2 * 1024 * 1024 });
    assert.equal(r.status, 0, 'read native Record'); return r.stdout.trim().split('\n').filter(Boolean).map(JSON.parse);
  };
  const read = () => lastJSON(command(setupBin, ['runtime', work, meta.mutation_id], work, env));
  const count = kind => journal().filter(row => row.kind === kind).length;
  const pids = async () => {
    const result = []; const dir = path.join(work, '.hale/dna');
    for (const name of ['org.pid', 'fence.pid', 'iris.pid']) { const text = await fs.readFile(path.join(dir, name), 'utf8').catch(() => ''); if (+text > 0) result.push(+text); }
    const attempts = path.join(dir, 'organization-launches');
    for (const entry of await fs.readdir(attempts).catch(() => [])) for (const name of ['org.pid', 'rollback.pid']) { const text = await fs.readFile(path.join(attempts, entry, name), 'utf8').catch(() => ''); if (+text > 0) result.push(+text); }
    return [...new Set(result)];
  };
  const host = await start('host', hostBin, ['run', work, 'app', '', '', '--no-iris', '--observe', '1']);
  try {
    await poll(async () => { if (count('organization.source.launch_requested') === 1) return true; if (host.exitCode !== null) throw new Error('Host exited before claim: ' + await fs.readFile(path.join(root, 'host.log'), 'utf8')); return false; }, 'durable launch claim');
    if (mode === 'claim-reply') {
      await poll(() => host.exitCode !== null, 'unknown claim host exit'); assert.equal(host.exitCode, 3);
      assert.equal(count('organization.source.launched'), 0); assert.equal(await fs.readFile(path.join(work, '.hale/dna/org.pid'), 'utf8').catch(() => ''), '');
      const state = read(); assert.equal(state.attempt, true); assert.equal(state.available, false);
      assert.match(await fs.readFile(path.join(control, 'fault.log'), 'utf8'), /lost-claim-reply/);
      await fs.writeFile(path.join(root, 'state.json'), JSON.stringify(state, null, 2));
    } else {
      let before;
      if (mode !== 'healthy') {
        await poll(async () => (await fs.readFile(path.join(control, 'fault.log'), 'utf8').catch(() => '')).includes('withheld-ack'), 'named acknowledgment interruption');
        before = await pids(); const state = read();
        assert.equal(state.available, false, 'claim alone never supplies current process association');
        if (mode === 'launch-ack') { assert.equal(state.launched, false); assert.equal(count('organization.source.launch_requested'), 1); }
        if (mode === 'rollback-ack') { assert.equal(command('/usr/bin/git', ['rev-parse', 'HEAD'], work, env).trim(), meta.source_head); assert.equal(count('mutation.rolled_back'), 0); }
        if (mode === 'rollback-launch-ack') { assert.equal(count('mutation.rolled_back'), 1); assert.equal(state.rollback, false); }
        assert.equal(host.exitCode, null, 'same host remains available for exact acknowledgment recovery');
        await fs.writeFile(path.join(control, 'release'), 'release named fault');
      }
      if (mode.startsWith('rollback')) {
        await poll(() => count('organization.source.rollback_launched') === 1, 'same owned rollback launch acknowledged');
        const state = read(); assert.equal(state.rollback, true); assert.equal(state.available, false); assert.equal(state.exited, true);
        assert.equal(count('organization.source.rollback_requested'), 1); assert.equal(count('organization.source.rollback_launch_requested'), 1); assert.equal(count('mutation.rolled_back'), 1);
        assert.equal(command('/usr/bin/git', ['rev-parse', 'HEAD'], work, env).trim(), meta.source_head);
        if (mode === 'rollback-launch-ack') assert.ok(before.includes(+(await fs.readFile(path.join(work, '.hale/dna/org.pid'), 'utf8'))), 'lost rollback ack recovers the same existing process');
        await fs.writeFile(path.join(root, 'state.json'), JSON.stringify(state, null, 2));
      } else {
        await poll(() => journal().some(row => row.kind === 'expression.observed' && row.entity === meta.mutation_id && row.body.startsWith('healthy ')), 'actual Body healthy acknowledgment');
        const state = await poll(() => { const value = read(); return value.available ? value : false; }, 'fresh stable native association'); assert.equal(state.available, true); assert.equal(state.observed, true); assert.equal(state.outcome, 'healthy'); assert.equal(state.current_process_key, state.process_key);
        assert.equal(state.candidate_commit, meta.candidate_commit); assert.equal(count('organization.source.launched'), 1); assert.equal(count('organization.source.launch_requested'), 1);
        if (mode === 'launch-ack') assert.ok(before.includes(state.pid), 'lost launch ack recovers the same existing process');
        const dir = path.join(work, '.hale/dna/organization-launches', crypto.createHash('sha256').update(state.attempt_id).digest('hex'));
        assert.equal(sha(await fs.readFile(path.join(dir, 'program'))), state.binary_digest); assert.equal(sha(await fs.readFile(path.join(dir, 'topology.json'))), state.topology_digest);
        const observerPort = await port(); await start('observer', observerBin, [String(observerPort), path.join(repo, 'iris/render/web'), path.join(dir, 'topology.json')], env);
        const snapshot = await poll(async () => { try { const value = await (await fetch(`http://127.0.0.1:${observerPort}/snapshot`, { signal: AbortSignal.timeout(1000) })).json(); return value.processes?.some(p => p.pid === state.pid && p.process_key === state.process_key) ? value : false; } catch { return false; } }, 'actual observer and current process join', 12000);
        await fs.writeFile(path.join(root, 'snapshot.json'), JSON.stringify(snapshot, null, 2)); await fs.writeFile(path.join(root, 'state.json'), JSON.stringify(state, null, 2));
        // An actual composed API shares the same native reader in this exact
        // PID/IPC/filesystem namespace. The grants authorize evidence reads only.
        let statusRead;
        if (apiBin) {
          const principal = 'runtime-reader';
          const commandPolicy = path.join(root, 'authority.json'); const sourcePolicy = path.join(root, 'source-authority.json');
          await fs.writeFile(commandPolicy, JSON.stringify({ format: 'dna.practice-review-authority/1', application_id: meta.application_id,
            grants: [{ mode: 'local', name: principal, authority: 'board', practice_propose: false, review_verdict: false, recover: true }] }));
          await fs.writeFile(sourcePolicy, JSON.stringify({ format: 'dna.organization-authority/1', application_id: meta.application_id,
            grants: [{ mode: 'local', name: principal, authority: 'board', organization_propose: false, organization_review: false, recover: true }] }));
          const apiPort = await port();
          await start('api', apiBin, [work, String(apiPort)], { ...env, USER: principal, HALE_DNA_COMMAND_POLICY: commandPolicy, HALE_DNA_ORGANIZATION_POLICY: sourcePolicy });
          statusRead = async () => {
            const head = command('/usr/bin/git', ['rev-parse', 'refs/dna/journal'], work, env).trim();
            try {
              const response = await fetch(`http://127.0.0.1:${apiPort}/api/hale/v1/applications/${meta.application_id}/dna/organization/source-status?` + new URLSearchParams({ id: meta.mutation_id, snapshot: head }), { signal: AbortSignal.timeout(3000) });
              if (response.status === 409) return null;
              const value = await response.json(); assert.equal(response.status, 200, JSON.stringify(value)); return value;
            } catch (error) { if (error instanceof TypeError || error.name === 'TimeoutError') return null; throw error; }
          };
          const current = await poll(async () => { const v = await statusRead(); return v?.data.current_running.available ? v : false; }, 'actual API current-running association', 12000);
          assert.equal(current.data.current_running.process_key, state.process_key);
          assert.ok(snapshot.processes.some(p => p.pid === state.pid && p.process_key === current.data.current_running.process_key), 'public opaque key joins exact native observer process');
          for (const field of ['attempt_id', 'launch_event_id', 'candidate_commit', 'binary_digest', 'topology_digest', 'topology_shape']) assert.equal(current.data.current_running[field], state[field], `exact API ${field}`);
          assert.equal(current.data.application.state, 'applied'); assert.equal(current.data.observation.state, 'healthy');
          assert.equal(Object.hasOwn(current.data.current_running, 'pid'), false, 'public status does not expose PID');
          await fs.writeFile(path.join(root, 'api-current-response.json'), JSON.stringify(current, null, 2));
        }
        // Historical health persists after the exact owned child ends. A fresh
        // read must not turn that history into current running evidence.
        process.kill(state.pid, 'SIGTERM'); await poll(() => !read().available, 'ended process association unavailable', 10000);
        assert.equal(count('organization.source.launch_observed'), 1);
        if (statusRead) {
          const ended = await poll(async () => { const v = await statusRead(); return v && !v.data.current_running.available ? v : false; }, 'actual API ended-process association unavailable', 10000);
          assert.equal(ended.data.current_running.process_key, ''); assert.equal(ended.data.observation.state, 'healthy');
          assert.equal(ended.data.launch.state, 'launched'); assert.equal(ended.data.current_running.attempt_id, '');
          await fs.writeFile(path.join(root, 'api-ended-response.json'), JSON.stringify(ended, null, 2));
        }
      }
    }
    await fs.writeFile(path.join(root, 'journal.json'), JSON.stringify(journal(), null, 2));
    console.log(JSON.stringify({ case: mode, passed: true, root }));
  } finally {
    clearInterval(buildTimer);
    for (const child of buildChildren) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
    const owned = await pids();
    for (const { child } of children) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
    for (const pid of owned) { try { process.kill(pid, 'SIGTERM'); } catch {} }
    await delay(150);
    for (const { child } of children) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
    for (const pid of owned) { try { process.kill(pid, 'SIGKILL'); } catch {} }
    await fs.writeFile(path.join(root, 'processes.json'), JSON.stringify(children.map(({ name, child }) => ({ name, pid: child.pid, exitCode: child.exitCode, signalCode: child.signalCode })), null, 2));
    for (const handle of descriptors) await handle.close();
  }
}
for (const mode of modes) await run(mode);
