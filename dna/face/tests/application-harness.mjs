// Real plain-Hale application + native API. No DNA Record or scripted provider.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { isolatedEnvironment, boundedNative } from './environment.mjs';

export const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
export const binary = process.env.HALE_COCKPIT_APPLICATION_BIN;
const web = fileURLToPath(new URL('../web/', import.meta.url));
async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode) return;
  const closed = new Promise(resolve => child.once('close', resolve));
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 1000);
  await closed;
  clearTimeout(timer);
}
export async function applicationFixture() {
  if (!binary || !path.isAbsolute(binary)) throw new Error('HALE_COCKPIT_APPLICATION_BIN must name the built native intake-control example.');
  const origin = `http://127.0.0.1:${await freePort()}`;
  const root = await mkdtemp('/tmp/hale-iris-application.');
  const db = path.join(root, 'application.sqlite');
  const env = isolatedEnvironment();
  for (const key of Object.keys(env)) if (/^(HALE_DNA_|HALE_IRIS_|LOTUS_)/.test(key)) delete env[key];
  // Observation stays off: no LOTUS_ variable reaches the application.
  const children = new Set();
  let log = '', app, api, identity;
  let principal = 'operator';
  const cleanup = () => { for (const child of children) if (child.exitCode === null) child.kill('SIGKILL'); };
  process.once('exit', cleanup);
  const start = args => {
    const bounded = boundedNative(binary, args, { lock: false });
    const child = spawn(bounded.command, bounded.args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
    children.add(child);
    child.output = '';
    const capture = chunk => { child.output = (child.output + chunk).slice(-65536); log = (log + chunk).slice(-131072); };
    child.stdout.on('data', capture); child.stderr.on('data', capture);
    child.on('error', error => { child.failure = error; });
    return child;
  };
  async function waitFor(child, predicate) {
    const deadline = Date.now() + 7000;
    while (Date.now() < deadline) {
      if (child.failure) throw child.failure;
      if (child.exitCode !== null || child.signalCode) throw new Error('Native application exited: ' + log);
      if (await predicate()) return;
      await pause(25);
    }
    throw new Error('Native application readiness timed out: ' + log);
  }
  const fixture = {
    root, db, origin,
    get appPID() { return app?.pid; },
    get apiPID() { return api?.pid; },
    get identity() { return identity; },
    get principal() { return principal; },
    get prefix() { return '/api/hale/v1/applications/' + identity.application_id; },
    log: () => log,
    request: async (suffix, options = {}) => {
      const { method = 'GET', body, headers = {} } = options;
      const response = await fetch(origin + fixture.prefix + suffix, {
        method, headers: { ...(method === 'POST' ? { Origin: origin, 'Content-Type': 'application/json', 'X-Iris-Command': '1' } : {}), ...headers },
        ...(body !== undefined ? { body: typeof body === 'string' ? body : JSON.stringify(body) } : {}),
        redirect: 'error', signal: AbortSignal.timeout(5000),
      });
      return { status: response.status, body: await response.json() };
    },
    state: async () => {
      const response = await fixture.request('/state');
      if (response.status !== 200) throw new Error(JSON.stringify(response));
      return response.body.data;
    },
    command: (state, value = 'paused', requestId = crypto.randomUUID()) => ({
      request_id: requestId, operation: 'example.intake.set_mode', operation_version: '1',
      context: { application_id: identity.application_id },
      target: { application_id: identity.application_id, kind: 'application.control', id: 'intake' },
      preconditions: { incarnation_id: state.application.incarnation_id, revision: state.control.revision, principal: { mode: 'trusted_local', name: principal } },
      arguments: { value },
    }),
    submit: command => fixture.request('/commands', { method: 'POST', body: command }),
    lookup: id => fixture.request('/commands?request_id=' + encodeURIComponent(id)),
    startApp: async () => {
      const next = start(['run', db, 'operator']);
      await waitFor(next, () => /application_id=([a-f0-9]{64}) incarnation_id=([a-f0-9]{64})/.test(next.output));
      const match = next.output.match(/application_id=([a-f0-9]{64}) incarnation_id=([a-f0-9]{64})/);
      identity = { application_id: match[1], incarnation_id: match[2] };
      app = next;
      return next;
    },
    stopApp: () => stop(app),
    stopApi: () => stop(api),
    startApi: async (actor = principal) => {
      principal = actor;
      api = start(['serve', db, new URL(origin).port, web, actor]);
      await waitFor(api, async () => {
        if (!api.output.includes('iris-application:listening')) return false;
        try { return (await fixture.request('/state')).status === 200; } catch { return false; }
      });
    },
    close: async () => {
      for (const child of children) await stop(child);
      process.removeListener('exit', cleanup);
      await rm(root, { recursive: true, force: true });
    },
  };
  try {
    await fixture.startApp(); await fixture.startApi();
    return fixture;
  } catch (error) { await fixture.close(); throw error; }
}
