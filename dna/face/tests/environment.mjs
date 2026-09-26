// Memory's three DSNs never reach a spawned process by inheritance (GH
// #985). The owner's goes only to a fixture program that migrates or drops
// a record (`memoryOwner`, below), the head's only to an API a harness
// starts, and the spine's to nothing a browser fixture starts.
export const MEMORY_DSNS = ['HALE_DNA_MEMORY_DSN_OWNER', 'HALE_DNA_MEMORY_DSN_SPINE', 'HALE_DNA_MEMORY_DSN_HEAD'];
// The nerves' URLs likewise (GH #1029): the owner's goes only to the host a
// harness starts (`hale dna dev` creates the stream and hands the host the
// spine's), never to a browser fixture by inheritance.
export const NERVES_URLS = ['HALE_DNA_NATS_URL_OWNER', 'HALE_DNA_NATS_URL_SPINE', 'HALE_DNA_NATS_URL_HEAD', 'HALE_DNA_NATS_URL_APP', 'HALE_DNA_NATS_ORG'];

export function isolatedEnvironment() {
  const env = { ...process.env };
  for (const key of [
    'GIT_DIR', 'GIT_COMMON_DIR', 'GIT_CONFIG', 'GIT_NAMESPACE', 'GIT_WORK_TREE',
    'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
    'GIT_CONFIG_COUNT', 'GIT_CONFIG_PARAMETERS', ...MEMORY_DSNS, ...NERVES_URLS,
    'HALE_DNA_KNOWLEDGE_COMMAND_POLICY', 'HALE_DNA_EVIDENCE_KEY', 'HALE_DNA_OIDC_SECRET',
    'HALE_DNA_OWNER', 'HALE_DNA_LEASE', 'HALE_DNA_LEASE_TOKEN', 'HALE_DNA_TAPE', 'HALE_DNA_ONESHOT',
    'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'LOTUS_OBS', 'HALE_DNA_ORG_DRAFTS',
  ]) delete env[key];
  return Object.assign(env, {
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_TERMINAL_PROMPT: '0', HALE_DNA_DISCOVER: 'off',
  });
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
  const memory = build ? 2_147_483_648 : 536_870_912;
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
