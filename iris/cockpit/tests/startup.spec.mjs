import { test as base, expect } from '@playwright/test';
import { mkdtemp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { isolatedEnvironment, boundedNative } from './environment.mjs';

const execute = promisify(execFile), cockpit = fileURLToPath(new URL('../', import.meta.url));
const launcher = path.join(cockpit, 'start.sh');
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
    const scratch = await mkdtemp('/tmp/hale-iris-startup.'), root = path.join(scratch, 'fresh project $literal');
    const env = isolatedEnvironment(); env.HALE_BIN = process.env.HALE_BIN;
    if (!env.HALE_BIN || !process.env.HALE_API_BIN) throw new Error('Run through the native browser test launcher or provide HALE_BIN and HALE_API_BIN.');
    const generated = boundedNative(env.HALE_BIN, ['dna', 'new', root], { build: true });
    const children = [];
    try {
      await execute(generated.command, generated.args, { env, timeout: 300000, maxBuffer: 2097152 });
      const git = args => execute('git', ['-C', root, ...args], { env, timeout: 5000 }).then(r => r.stdout.trim());
      // Organization inspection is explicitly committed-source inspection.
      await git(['add', '--', '.gitignore', 'hale.toml', 'hale.lock', 'main.hl', 'tests', 'dna']);
      await git(['-c', 'user.name=Iris startup test', '-c', 'user.email=iris@example.invalid', 'commit', '-q', '-m', 'Capture generated source']);
      const state = async () => ({ refs: await git(['show-ref']), status: await git(['status', '--porcelain']), source: await readFile(path.join(root, 'dna/org/main.hl'), 'utf8') });
      const start = async ({ build = false, drafts = false, extraEnv = {} } = {}) => {
        const chosenPort = await port(), origin = `http://127.0.0.1:${chosenPort}`;
        const options = [root, '--port', String(chosenPort), ...(drafts ? ['--source-drafts'] : [])];
        const childEnv = { ...env, ...extraEnv }; delete childEnv.HALE_API_BIN;
        if (!build) options.push('--api', process.env.HALE_API_BIN);
        else { childEnv.TMPDIR = path.join(scratch, 'build temporary'); await mkdir(childEnv.TMPDIR); }
        // Bound the real compiler/API tree without holding the native build lock
        // for the HTTP process lifetime. No application body is run or observed.
        const bounded = boundedNative(launcher, options, { build, lock: false });
        const child = spawn(bounded.command, bounded.args, { env: childEnv, cwd: scratch, stdio: ['ignore', 'pipe', 'pipe'] });
        children.push(child); let log = ''; let failure;
        child.on('error', error => { failure = error; });
        child.stdout.on('data', chunk => { log = (log + chunk).slice(-262144); }); child.stderr.on('data', chunk => { log = (log + chunk).slice(-262144); });
        // A launcher that builds the API tree needs the build budget's wall time.
        const deadline = Date.now() + (build ? 600000 : 45000);
        while (Date.now() < deadline && child.exitCode === null && !failure) {
          try {
            const response = await fetch(origin + '/api/hale/v1/applications', { signal: AbortSignal.timeout(1000) });
            const body = await response.json();
            if (response.ok && body.source.record_id === await git(['rev-list', '--max-parents=0', 'refs/dna/journal'])) return { child, origin, application: body.source.record_id, log: () => log, tmpdir: childEnv.TMPDIR };
          } catch { }
          await sleep(50);
        }
        throw failure || new Error('Iris launcher did not become ready: ' + log);
      };
      await use({ root, env, scratch, state, start });
    } finally { for (const child of children) await stop(child); await rm(scratch, { recursive: true, force: true }); }
  },
});
test.setTimeout(900000);

test('One-command startup builds native Iris for a fresh DNA project without changing it', async ({ page, project }, testInfo) => {
  const before = await project.state(); const service = await project.start({ build: true, drafts: true });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  const organization = page.waitForResponse(r => r.url().includes('/dna/organization?') && !new URL(r.url()).searchParams.has('id'));
  await page.goto(service.origin + '/#/organization');
  const response = await organization; expect(response.status(), await response.text()).toBe(200);
  const data = await response.json();
  expect(data.data.basis.dependency_source).toBe('local_vendor_snapshot');
  await expect(page.getByRole('region', { name: 'Declared containment topology', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Edit organization', exact: true })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Edit ownership', exact: true })).toBeEnabled();
  const capabilities = await page.request.get(`${service.origin}/api/hale/v1/applications/${service.application}/capabilities`);
  const caps = await capabilities.json(); expect(caps.data.reads.definitions).toBe(false); expect(caps.data.read_only).toBe(true);
  expect(await project.state()).toEqual(before); expect(errors).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('fresh-project-iris.png') });
  await writeFile(testInfo.outputPath('startup-capabilities.json'), JSON.stringify(caps, null, 2));
  await stop(service.child); expect(await readdir(service.tmpdir)).toEqual([]);
});

test('An existing native API starts separately and one cockpit stopping leaves the other running', async ({ page, project }) => {
  const before = await project.state(), first = await project.start(), second = await project.start();
  await page.goto(first.origin + '/#/practices');
  await expect(page.getByRole('heading', { name: 'Practices', exact: true })).toBeVisible();
  await stop(second.child);
  const alive = await page.request.get(first.origin + '/api/hale/v1/applications');
  expect(alive.status()).toBe(200); expect((await alive.json()).source.record_id).toBe(first.application);
  expect(first.child.exitCode).toBeNull(); expect(await project.state()).toEqual(before);
});

test('Startup refuses incomplete service wiring without exposing credentials or changing the project', async ({ project }) => {
  const before = await project.state(), credential = 'private-startup-test-credential-do-not-log';
  let failure;
  try { await execute(launcher, [project.root, '--api', process.env.HALE_API_BIN], { env: { ...project.env, HALE_DNA_KNOWLEDGE_READ_KEY: credential }, timeout: 5000 }); } catch (error) { failure = error; }
  expect(failure?.code).toBe(2); expect(failure.stderr).toContain('require both');
  expect(failure.stdout + failure.stderr).not.toContain(credential); expect(await project.state()).toEqual(before);
});
