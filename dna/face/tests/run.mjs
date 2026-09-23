import { mkdtemp, readFile, writeFile, rm, access, stat } from 'node:fs/promises';
import { constants } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { isolatedEnvironment, boundedNative } from './environment.mjs';

const face = fileURLToPath(new URL('../', import.meta.url));
const repo = path.resolve(face, '../..');
const hale = path.resolve(process.env.HALE_BIN || path.join(repo, 'target/release/hale'));
const api = path.resolve(process.env.HALE_API_BIN || path.join(repo, 'dna/api/api'));
const catalog = process.env.HALE_FACE_CATALOG_BIN || '';
const knowledge = process.env.HALE_FACE_KNOWLEDGE_BIN || '';
const commands = process.env.HALE_FACE_COMMAND_BIN || '';
const workflows = process.env.HALE_FACE_WORKFLOWS_BIN || '';
const env = isolatedEnvironment();
const scratch = await mkdtemp('/tmp/hale-face-browser-build.');
let active;
function run(command, args, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    const bounded = command === hale ? boundedNative(command, args, { build: true }) : { command, args };
    active = spawn(bounded.command, bounded.args, { cwd: face, env: { ...env, ...extraEnv }, stdio: 'inherit' });
    active.once('error', reject);
    active.once('exit', (code, signal) => {
      active = undefined;
      if (code === 0) resolve();
      else reject(new Error(`${command} exited ${code ?? signal}`));
    });
  });
}
async function validateProvider(binary, variable, label) {
  if (!binary) {
    console.log(`${label} browser integration skipped: ${variable} was not supplied.`);
    return;
  }
  if (!path.isAbsolute(binary)) throw new Error(`${variable} must be an absolute path to the ${label} integration fixture binary.`);
  if (!(await stat(binary)).isFile()) throw new Error(`${variable} must name a file.`);
  await access(binary, constants.R_OK | constants.X_OK);
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => active?.kill(signal));
try {
  await validateProvider(catalog, 'HALE_FACE_CATALOG_BIN', 'Definitions');
  await validateProvider(knowledge, 'HALE_FACE_KNOWLEDGE_BIN', 'Knowledge');
  await validateProvider(commands, 'HALE_FACE_COMMAND_BIN', 'Scripted command adapter');
  await validateProvider(workflows, 'HALE_FACE_WORKFLOWS_BIN', 'Recorded workflows');
  await access(hale);
  await access(api);
  const source = (await readFile(path.join(face, 'tests/record/main.hl'), 'utf8'))
    .replace('"../../../../dna/core"', JSON.stringify(path.join(repo, 'dna/core')))
    .replace('"../../../../dna/api/tests/fixture"', JSON.stringify(path.join(repo, 'dna/api/tests/fixture')));
  const entry = path.join(scratch, 'record.hl');
  await writeFile(entry, source);
  await run(hale, ['check', entry]);
  await run(hale, ['build', entry]);
  await run(process.execPath, [path.join(face, 'node_modules/@playwright/test/cli.js'),
    'test', '--config', 'tests/playwright.config.mjs', ...process.argv.slice(2)], {
    HALE_BIN: hale, HALE_API_BIN: api, HALE_FACE_RECORD_BIN: path.join(scratch, 'record'),
    ...(catalog ? { HALE_FACE_CATALOG_BIN: catalog } : {}),
    ...(knowledge ? { HALE_FACE_KNOWLEDGE_BIN: knowledge } : {}),
    ...(workflows ? { HALE_FACE_WORKFLOWS_BIN: workflows } : {}),
    ...(commands ? { HALE_FACE_COMMAND_BIN: commands } : {}),
    HALE_API_CONTRACT_ROOT: path.join(repo, 'dna/api/contract/v1'),
  });
} catch (error) {
  console.error(error.message);
  console.error('Build dna/api first, then supply HALE_BIN and HALE_API_BIN if they are outside this checkout.');
  process.exitCode = 1;
} finally {
  await rm(scratch, { recursive: true, force: true });
}
