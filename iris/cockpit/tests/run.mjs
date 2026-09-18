import { mkdtemp, readFile, writeFile, rm, access } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { isolatedEnvironment } from './environment.mjs';

const cockpit = fileURLToPath(new URL('../', import.meta.url));
const repo = path.resolve(cockpit, '../..');
const hale = path.resolve(process.env.HALE_BIN || path.join(repo, 'target/release/hale'));
const api = path.resolve(process.env.HALE_API_BIN || path.join(repo, 'dna/api/api'));
const env = isolatedEnvironment();
const scratch = await mkdtemp('/tmp/hale-iris-browser-build.');
let active;
function run(command, args, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    active = spawn(command, args, { cwd: cockpit, env: { ...env, ...extraEnv }, stdio: 'inherit' });
    active.once('error', reject);
    active.once('exit', (code, signal) => {
      active = undefined;
      if (code === 0) resolve();
      else reject(new Error(`${command} exited ${code ?? signal}`));
    });
  });
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => active?.kill(signal));
try {
  await access(hale);
  await access(api);
  const source = (await readFile(path.join(cockpit, 'tests/record/main.hl'), 'utf8'))
    .replace('"../../../../dna/core"', JSON.stringify(path.join(repo, 'dna/core')))
    .replace('"../../../../dna/api/tests/fixture"', JSON.stringify(path.join(repo, 'dna/api/tests/fixture')));
  const entry = path.join(scratch, 'record.hl');
  await writeFile(entry, source);
  await run(hale, ['check', entry]);
  await run(hale, ['build', entry]);
  await run(process.execPath, [path.join(cockpit, 'node_modules/@playwright/test/cli.js'),
    'test', '--config', 'tests/playwright.config.mjs', ...process.argv.slice(2)], {
    HALE_API_BIN: api, HALE_COCKPIT_RECORD_BIN: path.join(scratch, 'record'),
    HALE_API_CONTRACT_ROOT: path.join(repo, 'dna/api/contract/v1'),
  });
} catch (error) {
  console.error(error.message);
  console.error('Build dna/api first, then supply HALE_BIN and HALE_API_BIN if they are outside this checkout.');
  process.exitCode = 1;
} finally {
  await rm(scratch, { recursive: true, force: true });
}
