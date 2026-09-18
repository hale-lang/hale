import { test as base, expect } from '@playwright/test';
import { mkdtemp, readFile, rm, mkdir, copyFile, writeFile, rename } from 'node:fs/promises';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createHash } from 'node:crypto';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { isolatedEnvironment } from './environment.mjs';

const execute = promisify(execFile);
const cockpit = fileURLToPath(new URL('../', import.meta.url));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function availablePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
async function stop(child) {
  if (!child?.pid || child.exitCode !== null || child.signalCode) return;
  const closed = new Promise(resolve => child.once('close', resolve));
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 1_000);
  await closed;
  clearTimeout(timer);
}

export const test = base.extend({
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await use(page);
    expect(errors, 'the cockpit raised no unhandled JavaScript errors').toEqual([]);
  },
  recordCount: [3, { option: true }],
  organization: [false, { option: true }],
  service: async ({ recordCount, organization }, use, testInfo) => {
    const root = await mkdtemp('/tmp/hale-iris-browser.');
    const env = isolatedEnvironment();
    env.XDG_CACHE_HOME = path.join(root, '.hale/cockpit-cache');
    const native = process.env.HALE_COCKPIT_RECORD_BIN;
    const api = process.env.HALE_API_BIN;
    if (!native || !api) throw new Error('Use npm test: it prepares the native Record writer and API paths.');
    let child;
    let log = '';
    const exitCleanup = () => { if (child?.exitCode === null) child.kill('SIGKILL'); };
    process.once('exit', exitCleanup);
    try {
      if (organization === 'generated') {
        await execute(env.HALE_BIN, ['dna', 'new', root], { env, timeout: 90_000, maxBuffer: 2_097_152 });
      }
      await execute(native, [root, 'seed', String(recordCount)], { env, timeout: 30_000 });
      const data = JSON.parse(await readFile(path.join(root, 'fixture.json'), 'utf8'));
      const git = async args => (await execute('git', ['-C', root, ...args], { env, timeout: 5_000 })).stdout.trim();
      const orgSource = path.join(root, 'dna/org/main.hl');
      let originalOrganization;
      const dependency = path.join(root, 'vendor/dna/assembly.hl');
      let originalDependency;
      if (organization === true || organization === 'large') {
        await mkdir(path.dirname(orgSource), { recursive: true });
        await copyFile(path.join(cockpit, 'tests/organization/main.hl'), orgSource);
        originalOrganization = await readFile(orgSource, 'utf8');
        if (organization === 'large') {
          const members = Array.from({ length: 30 }, (_, i) => `        extra_${String(i).padStart(2, '0')}: Reviewer = Reviewer { };`).join('\n');
          originalOrganization = originalOrganization.replace('        metrics: Metrics', `${members}\n        metrics: Metrics`);
          await writeFile(orgSource, originalOrganization);
        }
        // Domain ownership names intentionally do not equal compiler instance
        // paths. The UI must expose the map without inventing their binding.
        await writeFile(path.join(root, 'dna/org/owners'), 'org = acme\norg/support = partner\nacme: alice\npartner: bob\nhost = acme\n');
        await git(['add', 'dna/org']);
        await git(['commit', '-q', '-m', 'Declare browser organization fixture']);
        data.organizationHead = await git(['rev-parse', 'HEAD']);
      } else if (organization === 'generated') {
        originalOrganization = await readFile(orgSource, 'utf8');
        originalDependency = await readFile(dependency, 'utf8');
        await git(['add', '--', '.gitignore', 'hale.toml', 'hale.lock', 'main.hl', 'tests', 'dna']);
        await git(['commit', '-q', '-m', 'Commit generated DNA project']);
        data.organizationHead = await git(['rev-parse', 'HEAD']);
        if (await git(['check-ignore', 'vendor/dna/assembly.hl']) !== 'vendor/dna/assembly.hl') {
          throw new Error('Generated fixture must exercise a gitignored native DNA dependency.');
        }
      }
      let origin;
      for (let attempt = 0; attempt < 3 && !origin; attempt++) {
        const port = await availablePort();
        const candidate = `http://127.0.0.1:${port}`;
        child = spawn(api, [root, String(port), path.join(cockpit, 'web')], {
          env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
        });
        let spawnError;
        child.once('error', error => { spawnError = error; });
        const capture = chunk => { log = (log + chunk).slice(-262_144); };
        child.stdout.on('data', capture);
        child.stderr.on('data', capture);
        const deadline = Date.now() + 10_000;
        while (Date.now() < deadline && child.exitCode === null && !spawnError) {
          try {
            const response = await fetch(`${candidate}/api/hale/v1/applications`, { signal: AbortSignal.timeout(500) });
            const payload = await response.json();
            // A response from another process occupying the released probe
            // port is never accepted as this fixture's readiness.
            if (response.ok && payload.source.record_id === data.application && child.exitCode === null) {
              origin = candidate;
              break;
            }
          } catch { /* startup has not reached its listener yet */ }
          await delay(25);
        }
        if (!origin) await stop(child);
        if (spawnError) throw spawnError;
      }
      if (!origin) throw new Error(`Owned API failed readiness.\n${log}`);
      const apiPath = `/api/hale/v1/applications/${data.application}`;
      await use({
        ...data, origin, apiPath,
        mutate: action => execute(native, [root, action], { env, timeout: 15_000 }),
        changeOrganization: async ({ valid = true, commit = true } = {}) => {
          if (!organization) throw new Error('This test did not request an organization fixture.');
          await writeFile(orgSource, valid ? originalOrganization.replace('observed: Int = 0', 'observed: Int = 2') : 'main locus Broken { invalid source\n');
          if (commit) {
            await git(['add', 'dna/org/main.hl']);
            await git(['commit', '-q', '-m', 'Change organization source']);
          }
          return git(['rev-parse', 'HEAD']);
        },
        changeDependency: async mode => {
          if (organization !== 'generated') throw new Error('This test did not request generated dependencies.');
          if (mode === 'missing') await rename(path.join(root, 'vendor/dna'), path.join(root, 'vendor/dna.saved'));
          else if (mode === 'restore-missing') await rename(path.join(root, 'vendor/dna.saved'), path.join(root, 'vendor/dna'));
          else await writeFile(dependency, mode === 'invalid' ? 'locus InvalidDependency { invalid source\n' : originalDependency + (mode === 'comment' ? '\n// Browser fixture changed ignored dependency bytes.\n' : ''));
        },
        projectState: async () => ({
          refs: await git(['show-ref']), status: await git(['status', '--porcelain']),
          dependency: organization === 'generated' ? await readFile(dependency)
            .then(bytes => createHash('sha256').update(bytes).digest('hex')).catch(() => null) : null,
        }),
        refs: async () => (await execute('git', ['-C', root, 'show-ref'], { env, timeout: 5_000 })).stdout,
        url: (view = 'practices', extra = {}) => `${origin}/#/${view}?${new URLSearchParams({ app: data.application, ...extra })}`,
      });
    } finally {
      await stop(child);
      process.removeListener('exit', exitCleanup);
      if (testInfo.status !== testInfo.expectedStatus) {
        await testInfo.attach('api.log', { body: log, contentType: 'text/plain' });
      }
      await rm(root, { recursive: true, force: true });
    }
  },
});
export { expect };
export const errorBody = (code, message) => ({
  api_version: 'hale.v1', error: { code, message, retryable: code === 'record_unavailable' },
});
