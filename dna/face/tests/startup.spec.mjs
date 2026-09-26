import { test as base, expect } from '@playwright/test';
import { mkdtemp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { isolatedEnvironment, boundedNative } from './environment.mjs';

const execute = promisify(execFile), face = fileURLToPath(new URL('../', import.meta.url));
const launcher = path.join(face, 'start.sh');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function port() {
  const server = net.createServer(); await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const number = server.address().port; await new Promise(resolve => server.close(resolve)); return number;
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  const closed = new Promise(resolve => child.once('close', resolve));
  child.kill('SIGTERM'); const timeout = setTimeout(() => child.kill('SIGKILL'), 5000);
  await closed; clearTimeout(timeout);
}
const test = base.extend({
  project: async ({}, use) => {
    const scratch = await mkdtemp('/tmp/hale-face-startup.'), root = path.join(scratch, 'fresh project $literal');
    const env = isolatedEnvironment(); env.HALE_BIN = process.env.HALE_BIN;
    if (!env.HALE_BIN || !process.env.HALE_API_BIN) throw new Error('Run through the native browser test launcher or provide HALE_BIN and HALE_API_BIN.');
    const generated = boundedNative(env.HALE_BIN, ['dna', 'new', root], { build: true });
    const children = [], stateDirs = [];
    const stopRecordedChildren = async () => {
      for (const dir of stateDirs) {
        let names = [];
        try { names = await readdir(path.join(dir, 'children')); } catch { continue; }
        for (const name of names.filter(n => n.endsWith('.pid'))) {
          try { const pid = Number((await readFile(path.join(dir, 'children', name), 'utf8')).trim()); if (pid > 0) process.kill(pid, 'SIGTERM'); } catch { }
        }
      }
      await sleep(500);
      for (const dir of stateDirs) {
        let names = [];
        try { names = await readdir(path.join(dir, 'children')); } catch { continue; }
        for (const name of names.filter(n => n.endsWith('.pid'))) {
          try { const pid = Number((await readFile(path.join(dir, 'children', name), 'utf8')).trim()); if (pid > 0) process.kill(pid, 'SIGKILL'); } catch { }
        }
      }
    };
    try {
      await execute(generated.command, generated.args, { env, timeout: 300000, maxBuffer: 2097152 });
      const git = args => execute('git', ['-C', root, ...args], { env, timeout: 5000 }).then(r => r.stdout.trim());
      // Organization inspection is explicitly committed-source inspection.
      await git(['add', '--', '.gitignore', 'hale.toml', 'hale.lock', 'main.hl', 'tests', 'dna']);
      await git(['-c', 'user.name=Face startup test', '-c', 'user.email=face@example.invalid', 'commit', '-q', '-m', 'Capture generated source']);
      const state = async () => ({ refs: await git(['show-ref']), status: await git(['status', '--porcelain']), source: await readFile(path.join(root, 'dna/org/main.hl'), 'utf8') });
      const start = async ({ build = false, drafts = false, extraEnv = {} } = {}) => {
        const chosenPort = await port(), origin = `http://127.0.0.1:${chosenPort}`;
        // The head's API child listens on its own port and outlives the head
        // by design: each launch gets a port of its own, and the teardown
        // stops every child the head recorded.
        const apiPort = await port();
        const options = [root, '--port', String(chosenPort), '--api-port', String(apiPort), ...(drafts ? ['--source-drafts'] : [])];
        const childEnv = { ...env, ...extraEnv }; delete childEnv.HALE_API_BIN; delete childEnv.HALE_HEAD_BIN;
        // The head keeps a registry and receipts under the state directory:
        // every launch here gets its own, never the operator's.
        childEnv.XDG_STATE_HOME = path.join(scratch, 'state'); childEnv.XDG_CONFIG_HOME = path.join(scratch, 'config');
        childEnv.HALE_DNA_HEAD_STATE = path.join(scratch, 'head-state-' + String(chosenPort)); stateDirs.push(childEnv.HALE_DNA_HEAD_STATE);
        // The launcher builds the head unless handed one: a case that supplies
        // the API also supplies a built head when the environment names one,
        // and otherwise takes the build budget for the head it will build.
        const prebuiltHead = process.env.HALE_NATIVE_HEAD_BIN || '';
        if (!build) { options.push('--api', process.env.HALE_API_BIN); if (prebuiltHead) options.push('--head', prebuiltHead); }
        else { childEnv.TMPDIR = path.join(scratch, 'build temporary'); await mkdir(childEnv.TMPDIR); }
        const builds = build || !prebuiltHead;
        // Bound the real compiler/API tree without holding the native build lock
        // for the HTTP process lifetime. No application body is run or observed.
        const bounded = boundedNative(launcher, options, { build: builds, lock: false });
        const child = spawn(bounded.command, bounded.args, { env: childEnv, cwd: scratch, stdio: ['ignore', 'pipe', 'pipe'] });
        children.push(child); let log = ''; let failure;
        child.on('error', error => { failure = error; });
        child.stdout.on('data', chunk => { log = (log + chunk).slice(-262144); }); child.stderr.on('data', chunk => { log = (log + chunk).slice(-262144); });
        // A launcher that builds the head or the API tree needs the build budget's wall time.
        const deadline = Date.now() + (builds ? 600000 : 45000);
        while (Date.now() < deadline && child.exitCode === null && !failure) {
          try {
            const response = await fetch(origin + '/api/hale/v1/applications', { signal: AbortSignal.timeout(1000) });
            const body = await response.json();
            if (response.ok && body.source.record_id === await git(['rev-list', '--max-parents=0', 'refs/dna/journal'])) {
              // The launch token the head minted into its state directory
              // (GH #989), and the URL it printed with it.
              const token = (await readFile(path.join(childEnv.HALE_DNA_HEAD_STATE, 'head.token'), 'utf8')).trim();
              return { child, origin, token, shell: `${origin}/?token=${token}`, application: body.source.record_id, log: () => log, tmpdir: childEnv.TMPDIR };
            }
          } catch { }
          await sleep(50);
        }
        throw failure || new Error('Face launcher did not become ready: ' + log);
      };
      await use({ root, env, scratch, state, start });
    } finally { for (const child of children) await stop(child); await stopRecordedChildren(); await rm(scratch, { recursive: true, force: true }); }
  },
});
test.setTimeout(900000);

test('One-command startup builds the native face for a fresh DNA project without changing it', async ({ page, project }, testInfo) => {
  const before = await project.state(); const service = await project.start({ build: true, drafts: true });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  const organization = page.waitForResponse(r => r.url().includes('/dna/organization?') && !new URL(r.url()).searchParams.has('id'));
  // The shell opens only with the launch token: bare, it is refused; at the
  // URL the head printed, it opens and sets the session cookie.
  const bare = await page.request.get(service.origin + '/');
  expect(bare.status()).toBe(401);
  expect(service.log()).toContain('/?token=' + service.token);
  expect((await page.goto(service.shell + '#/organization')).status()).toBe(200);
  expect((await page.request.get(service.origin + '/')).status(), 'the cookie the URL set opens the shell again').toBe(200);
  const response = await organization; expect(response.status(), await response.text()).toBe(200);
  const data = await response.json();
  expect(data.data.basis.dependency_source).toBe('local_vendor_snapshot');
  await expect(page.getByRole('region', { name: 'Declared containment topology', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Edit organization', exact: true })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Edit ownership', exact: true })).toBeEnabled();
  const capabilities = await page.request.get(`${service.origin}/api/hale/v1/applications/${service.application}/capabilities`);
  // Record commands are the head socket's gated topics (GH #1104 piece 5):
  // capabilities name that socket and the HTTP route that forwards one wire
  // line to it, and carry no command profile; HTTP itself writes nothing.
  const caps = await capabilities.json(); expect(caps.data.reads.definitions).toBe(false); expect(caps.data.read_only).toBe(true);
  expect(caps.data.writes).toBeUndefined(); expect(caps.data.commands).toBeUndefined();
  expect(caps.data.api.transport).toBe('unix'); expect(caps.data.api.socket).toMatch(new RegExp('/' + service.application.slice(0, 12) + '\\.sock$'));
  expect(caps.data.api.http).toBe(`/api/hale/v1/applications/${service.application}/commands`);
  expect(await project.state()).toEqual(before); expect(errors).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('fresh-project-face.png') });
  await writeFile(testInfo.outputPath('startup-capabilities.json'), JSON.stringify(caps, null, 2));
  await stop(service.child); expect(await readdir(service.tmpdir)).toEqual([]);
});

test('An existing native API starts separately and one face stopping leaves the other running', async ({ page, project }) => {
  const before = await project.state(), first = await project.start(), second = await project.start();
  await page.goto(first.shell + '#/practices');
  await expect(page.getByRole('heading', { name: 'Practices', exact: true })).toBeVisible();
  await stop(second.child);
  const alive = await page.request.get(first.origin + '/api/hale/v1/applications');
  expect(alive.status()).toBe(200); expect((await alive.json()).source.record_id).toBe(first.application);
  expect(first.child.exitCode).toBeNull(); expect(await project.state()).toEqual(before);
});

test('Startup refuses malformed memory wiring without exposing credentials or changing the project', async ({ project }) => {
  const before = await project.state(), credential = 'private-startup-test-credential-do-not-log';
  let failure;
  try { await execute(launcher, [project.root, '--api', process.env.HALE_API_BIN], { env: { ...project.env, HALE_DNA_MEMORY_DSN_HEAD: credential }, timeout: 5000 }); } catch (error) { failure = error; }
  expect(failure?.code).toBe(2); expect(failure.stderr).toContain('HALE_DNA_MEMORY_DSN_HEAD to be a postgres:// DSN');
  expect(failure.stdout + failure.stderr).not.toContain(credential); expect(await project.state()).toEqual(before);
});
