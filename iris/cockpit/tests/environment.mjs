export function isolatedEnvironment() {
  const env = { ...process.env };
  for (const key of [
    'GIT_DIR', 'GIT_COMMON_DIR', 'GIT_CONFIG', 'GIT_NAMESPACE', 'GIT_WORK_TREE',
    'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES',
    'GIT_CONFIG_COUNT', 'GIT_CONFIG_PARAMETERS', 'HALE_DNA_KNOWLEDGE_DSN',
    'HALE_DNA_KNOWLEDGE_URL', 'HALE_DNA_KNOWLEDGE_READ_KEY', 'HALE_DNA_EVIDENCE_KEY', 'HALE_DNA_OIDC_SECRET',
    'HALE_DNA_OWNER', 'HALE_DNA_LEASE', 'HALE_DNA_LEASE_TOKEN', 'HALE_DNA_TAPE', 'HALE_DNA_ONESHOT',
    'OPENAI_API_KEY', 'ANTHROPIC_API_KEY',
  ]) delete env[key];
  return Object.assign(env, {
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_TERMINAL_PROMPT: '0', HALE_DNA_DISCOVER: 'off',
  });
}

// A native allocation regression must fail its own process, not exhaust the
// desktop/CI host before a wall-clock timeout fires. These are per-process
// Linux limits; the browser worker remains separate (V8 reserves large VA).
export function boundedNative(command, args, { build = false, lock = true } = {}) {
  if (process.platform !== 'linux') return { command, args };
  const memory = build ? 2_147_483_648 : 536_870_912;
  const bounded = { command: '/usr/bin/prlimit', args: [
    `--as=${memory}:${memory}`, '--core=0:0', '--cpu=30:30', '--', command, ...args,
  ] };
  // Serialize bounded native commands, but never retain this lock for an
  // HTTP service's lifetime: its browser client may need a fixture mutation.
  return lock ? { command: '/usr/bin/flock', args: [
    '--exclusive', '--no-fork', '--wait', '120', '/tmp/iris-native-validation.lock', bounded.command, ...bounded.args,
  ] } : bounded;
}
