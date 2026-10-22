import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';

// Memory's three DSNs never reach a spawned process by inheritance (GH
// #985). The owner's goes only to a fixture program that migrates or drops
// a record (`memoryOwner`, below), the head's only to an API a harness
// starts, and the spine's to nothing a browser fixture starts.
export const MEMORY_DSNS = ['HALE_DNA_MEMORY_DSN_OWNER', 'HALE_DNA_MEMORY_DSN_SPINE', 'HALE_DNA_MEMORY_DSN_HEAD'];
// The nerves' URLs likewise (GH #1029): the owner's goes only to the host a
// harness starts (`hale habitat migrate` creates the stream, and `hale dna
// dev` hands the host the spine's), never to a browser fixture by
// inheritance.
export const NERVES_URLS = ['HALE_DNA_NATS_URL_OWNER', 'HALE_DNA_NATS_URL_SPINE', 'HALE_DNA_NATS_URL_HEAD', 'HALE_DNA_NATS_URL_APP', 'HALE_DNA_NATS_USER_APP', 'HALE_DNA_NATS_VAULT_APP', 'HALE_DNA_NATS_VAULT_REFLEXES', 'HALE_DNA_NATS_ORG'];

// Every environment a fixture is handed carries the job's vault
// (`HALE_VAULT_DIR`, mode 700, one for every worker of the run, named for
// the runner process that started them) and no real vault: a graft
// provisions the organism's secrets, and a fixture's never land in the
// developer's vault or read a secret held there. The job's habitat is
// beside it (GH #1653): the local habitat is long-lived and one per
// owner's servers, so every fixture grafts from the same one, with the
// same vault holding its record's passwords. The directory is left for the
// runner's temp sweep; a worker removing it would pull it from the others.
function jobDir() {
  const dir = path.join(os.tmpdir(), `hale-face-job-${process.env.HALE_FACE_JOB || process.ppid}`);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.chmodSync(dir, 0o700);
  return dir;
}

export function isolatedEnvironment() {
  const env = { ...process.env };
  delete env.HALE_VAULT_ADDR;
  const job = jobDir();
  env.HALE_VAULT_DIR = path.join(job, 'vault');
  fs.mkdirSync(env.HALE_VAULT_DIR, { recursive: true, mode: 0o700 });
  // GH #1653: and a local habitat beside it, so an organism a fixture
  // births is never a tenant of the developer's habitat
  env.XDG_DATA_HOME = path.join(job, 'data');
  // named outright: a HALE_DNA_HABITAT the developer exported wins over
  // XDG_DATA_HOME, and is never a fixture's
  env.HALE_DNA_HABITAT = path.join(env.XDG_DATA_HOME, 'hale/habitat/local');
  for (const key of [
    'GIT_DIR', 'GIT_COMMON_DIR', 'GIT_CONFIG', 'GIT_NAMESPACE', 'GIT_WORK_TREE',
    'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
    'GIT_CONFIG_COUNT', 'GIT_CONFIG_PARAMETERS', ...MEMORY_DSNS, ...NERVES_URLS,
    'HALE_DNA_KNOWLEDGE_COMMAND_POLICY', 'HALE_DNA_EVIDENCE_KEY',
    'HALE_DNA_OWNER', 'HALE_DNA_LEASE', 'HALE_DNA_LEASE_TOKEN', 'HALE_DNA_TAPE', 'HALE_DNA_ONESHOT',
    'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'LOTUS_OBS', 'HALE_DNA_ORG_DRAFTS',
  ]) delete env[key];
  // GH #989: a fixture's heads run the trusted-local session, which is a
  // fixture's mode alone; a project's run under OIDC
  env.HALE_DNA_TRUSTED_LOCAL = '1';
  return Object.assign(env, {
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_TERMINAL_PROMPT: '0', HALE_DNA_DISCOVER: 'off',
  });
}

// GH #1653: every organism but the habitat is grafted from a habitat. The
// job's is the one every environment names (`XDG_DATA_HOME`, in the job's
// directory), born once with no body, by whichever worker first needs it
// (`ensureHabitat`, behind a lock the others wait on): on the operator's
// backends when the runner names an owner's DSN and NATS (CI's: no
// compose), else its own. `graftEnvironment` is `env` with the owner's
// servers a graft agrees with, and does only that; `graft` is the argv of
// the act. A fixture's project is grafted with `--no-pack`: the standard
// pack is a Board's to ratify, not a face case's.
export function graftEnvironment(env) {
  const graft = { ...env };
  if (memoryOwner() && nervesOwner()) {
    graft.HALE_DNA_MEMORY_DSN_OWNER = memoryOwner();
    graft.HALE_DNA_NATS_URL_OWNER = nervesOwner();
  } else {
    delete graft.HALE_DNA_MEMORY_DSN_OWNER;
    delete graft.HALE_DNA_NATS_URL_OWNER;
  }
  return graft;
}
const habitatUp = () => ['habitat', 'up', '--no-body', '--backends', memoryOwner() && nervesOwner() ? 'operator' : 'own'];
export const graft = dir => ['habitat', 'graft', dir, '--no-pack'];

// The job's habitat is up: born by the first caller, awaited by the rest.
// Synchronous, so a fixture's graft can follow it without a race.
export function ensureHabitat(hale, env) {
  const dir = path.dirname(env.XDG_DATA_HOME);
  const ready = path.join(dir, 'habitat.ready'), lock = path.join(dir, 'habitat.lock');
  const nap = ms => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
  for (let waited = 0; !fs.existsSync(ready); waited += 500) {
    let mine = false;
    try { fs.mkdirSync(lock); mine = true; } catch { /* another worker is born it */ }
    if (mine) {
      try {
        const bounded = boundedNative(hale, habitatUp(), { build: true, lock: false });
        execFileSync(bounded.command, bounded.args, { env: graftEnvironment(env), encoding: 'utf8', timeout: 300_000, maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
        fs.writeFileSync(ready, '');
      } finally { fs.rmSync(lock, { recursive: true, force: true }); }
      return;
    }
    if (waited > 360_000) throw new Error('the job\'s habitat was not born by the worker that holds its lock');
    nap(500);
  }
}

// The owner's DSN the Knowledge fixtures migrate a record with, from the
// runner's own environment; "" when memory is not configured.
export const memoryOwner = () => process.env.HALE_DNA_MEMORY_DSN_OWNER || '';

// The owner's NATS URL the native command harness creates a record's
// stream with (GH #1029), from the runner's own environment; "" when the
// nerves are not configured. A plain `nats-server -js` (CI runs one) or
// docker's `nats:2 -js`.
export const nervesOwner = () => process.env.HALE_DNA_NATS_URL_OWNER || '';

// A native allocation regression must fail its own process, not exhaust the
// desktop/CI host before a wall-clock timeout fires. These are per-process
// Linux limits; the browser worker remains separate (V8 reserves large VA).
export function boundedNative(command, args, { build = false, lock = true } = {}) {
  if (process.platform !== 'linux') return { command, args };
  // The service reads the organization's topology of the vendored DNA (3.7 MB
  // and growing with the DNA's loci); loading it takes about 300 MiB of arena
  // at peak, which the old 512 MiB bound left too little headroom for.
  // A build's bound is its address space, which runs above its resident
  // peak: the composed head (dna/api/practice_review) peaks near 2 GiB
  // resident since its surface is served over MCP as well, and LLVM's
  // allocation failure under a 2 GiB cap aborts the compiler outright.
  const memory = build ? 3_221_225_472 : 805_306_368;
  // A build compiles the toolchain's host or a whole API tree, about a
  // CPU-minute from a cold cache on a fast machine and several on a CI
  // runner; a service or a fixture process keeps the short budget.
  const cpu = build ? 900 : 30;
  const bounded = { command: '/usr/bin/prlimit', args: [
    `--as=${memory}:${memory}`, '--core=0:0', `--cpu=${cpu}:${cpu}`, '--', command, ...args,
  ] };
  // Serialize bounded native commands, but never retain this lock for an
  // HTTP service's lifetime: its browser client may need a fixture mutation.
  return lock ? { command: '/usr/bin/flock', args: [
    '--exclusive', '--no-fork', '--wait', '120', '/tmp/face-native-validation.lock', bounded.command, ...bounded.args,
  ] } : bounded;
}

// The launch token a local head minted into `dir` (GH #989): the record's
// `.hale/dna` for an api head, the state directory for the project head.
export async function launchToken(root, dir = '.hale/dna') {
  const { readFile } = await import('node:fs/promises');
  const { join } = await import('node:path');
  const token = (await readFile(join(root, dir, 'head.token'), 'utf8')).trim();
  if (!/^[0-9a-f]{32}$/.test(token)) throw new Error('The head minted no launch token under ' + join(root, dir));
  return token;
}
