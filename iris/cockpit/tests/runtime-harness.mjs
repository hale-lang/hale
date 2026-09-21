// Browser transport fixtures only. The optional native lane runs the existing
// Hale observer and a plain Hale application, without a DNA Record or service.
import { createServer } from 'node:http';
import { readFile, mkdtemp, mkdir, rm, readdir } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { isolatedEnvironment } from './environment.mjs';

const web = fileURLToPath(new URL('../web/', import.meta.url));
export const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

export async function httpFixture(handle) {
  const sockets = new Set();
  const server = createServer((req, res) => Promise.resolve(handle(req, res)).catch(error => {
    if (!res.headersSent) res.writeHead(500);
    res.end(error.message);
  }));
  server.on('connection', socket => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
    },
  };
}

export function observation() {
  return {
    ts: 123456789,
    processes: [
      { pid: 41, name: 'Intake <img src=x onerror="window.__runtimeInjected=true">', state: 'live', model: '0011223344556677', records: 24, overruns: 0, restarts: 0,
        loci: [{ id: 1, type: 'Inbox', parent: 0, pub: 8, dlv: 0 }, { id: 2, type: 'Worker / équipe', parent: 1, pub: 0, dlv: 8 }] },
      { pid: 42, name: 'Verifier', state: 'live', model: '8899aabbccddeeff', records: 8, overruns: 0, restarts: 0,
        loci: [{ id: 1, type: 'Verifier', parent: 0, pub: 0, dlv: 4 }] },
    ],
    topics: [{ name: 'work.ready', shape: '{id:Int}', pub: 8, dlv: 4 }],
    edges: [{ topic: 'work.ready', from: 41, to: 42, matched: 4, latSum: 4000, latMax: 1500, sends: 8, delivers: 4 }],
    events: ['Observed message — 第二版 <script>window.__runtimeInjected=true</script>'],
  };
}

export async function scriptedObserver() {
  const state = { snapshot: observation(), advance: true, requests: [], inFlight: 0, maximumInFlight: 0, status: 200, raw: null, wait: null, redirect: '' };
  const server = await httpFixture(async (req, res) => {
    state.requests.push({ method: req.method, url: req.url, headers: req.headers });
    if (req.url !== '/snapshot') { res.writeHead(404).end(); return; }
    state.inFlight += 1;
    state.maximumInFlight = Math.max(state.maximumInFlight, state.inFlight);
    try {
      if (state.wait) await state.wait;
      if (res.destroyed) return;
      if (state.redirect) { res.writeHead(302, { location: state.redirect, 'access-control-allow-origin': '*' }).end(); return; }
      res.writeHead(state.status, { 'content-type': 'application/json', 'access-control-allow-origin': '*', 'cache-control': 'no-store' });
      if (state.advance) state.snapshot.ts += 1;
      res.end(state.raw ?? JSON.stringify(state.snapshot));
    } finally { state.inFlight -= 1; }
  });
  return Object.assign(state, server);
}

export async function cockpitHost(observerOrigin = '', configuration = null) {
  const requests = [];
  const assets = new Map(await Promise.all(['index.html', 'app.js', 'runtime.js', 'application.js', 'definition-draft.js', 'organization-draft.js', 'knowledge-draft.js', 'task-administration.js', 'task-create.js', 'styles.css'].map(async name => [name, await readFile(path.join(web, name))])));
  const server = await httpFixture((req, res) => {
    requests.push(req.url);
    const headers = {
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      'referrer-policy': 'no-referrer',
      'content-security-policy': `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'${observerOrigin ? ` ${observerOrigin}` : ''}; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`,
    };
    if (req.url === '/iris/observer.json') {
      res.writeHead(200, { ...headers, 'content-type': 'application/json' });
      res.end(JSON.stringify(configuration ?? { profile: 'hale.iris.observer.v0', origin: observerOrigin }));
      return;
    }
    const name = req.url === '/' ? 'index.html' : req.url.slice(1);
    if (!assets.has(name)) { res.writeHead(404, headers).end(); return; }
    const contentType = name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html';
    res.writeHead(200, { ...headers, 'content-type': contentType });
    res.end(assets.get(name));
  });
  return { ...server, requests, url: `${server.origin}/#/runtime` };
}

async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode) return;
  const exited = new Promise(resolve => child.once('close', resolve));
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 1000);
  await exited;
  clearTimeout(timer);
}

export async function nativeObserver(observerBinary, appBinary, { application = false } = {}) {
  const root = await mkdtemp('/tmp/hale-iris-observation.');
  const env = isolatedEnvironment();
  for (const key of Object.keys(env)) if (key.startsWith('HALE_DNA_') || key.startsWith('HALE_IRIS_') || key.startsWith('LOTUS_')) delete env[key];
  env.XDG_RUNTIME_DIR = root;
  await mkdir(path.join(root, 'hale'), { mode: 0o700 });
  const probe = await httpFixture((req, res) => res.writeHead(503).end());
  const origin = probe.origin;
  await probe.close();
  const apiProbe = application ? await httpFixture((req, res) => res.writeHead(503).end()) : null;
  const applicationOrigin = apiProbe?.origin || '';
  await apiProbe?.close();
  let supervisor, log = '', stdout = '';
  const command = async (name, acknowledgement) => {
    const offset = stdout.length;
    supervisor.stdin.write(name + '\n');
    for (let attempt = 0; attempt < 100; attempt++) {
      if (stdout.slice(offset).includes(acknowledgement)) return stdout.slice(offset);
      if (supervisor.exitCode !== null || supervisor.signalCode) throw new Error('Native fixture supervisor exited: ' + log);
      await pause(25);
    }
    throw new Error('Native fixture command did not complete: ' + name + '\n' + log);
  };
  const result = {
    origin, root, applicationOrigin,
    snapshot: async () => {
      const response = await fetch(origin + '/snapshot', { signal: AbortSignal.timeout(1000) });
      if (!response.ok) throw new Error('Native observer response ' + response.status);
      return response.json();
    },
    applicationState: async () => {
      const options = { signal: AbortSignal.timeout(1500) };
      const list = await (await fetch(applicationOrigin + '/api/hale/v1/applications', options)).json();
      return (await (await fetch(applicationOrigin + '/api/hale/v1/applications/' + list.data.items[0].id + '/state', options)).json()).data;
    },
    restartApp: async () => {
      const output = await command('restart-app', 'IRIS_APP_RESTARTED');
      result.appPID = JSON.parse(output.match(/IRIS_APP_RESTARTED (\{[^\n]+\})/)[1]).app_pid;
    },
    stopObserver: () => command('stop-observer', 'IRIS_OBSERVER_STOPPED'),
    stopApp: () => command('stop-app', 'IRIS_APP_STOPPED'),
    log: () => log,
    close: async () => {
      await stop(supervisor);
      await rm(root, { recursive: true, force: true });
    },
  };
  try {
    // The emitter sweeps stale /dev/shm/hale-obs-* at startup, so an isolated
    // registration directory alone is insufficient. Both native processes
    // share a private PID/IPC/mount namespace and private /dev/shm. There is
    // deliberately no unsafe host fallback if bwrap is unavailable.
    supervisor = spawn('/usr/bin/bwrap', [
      '--unshare-user', '--unshare-ipc', '--unshare-pid', '--ro-bind', '/', '/',
      '--dev', '/dev', '--tmpfs', '/dev/shm', '--tmpfs', '/tmp', '--bind', root, root,
      '--proc', '/proc', '--die-with-parent', '--', process.execPath,
      fileURLToPath(new URL('./runtime-native-process.mjs', import.meta.url)),
      root, observerBinary, appBinary, web, new URL(origin).port, ...(application ? [new URL(applicationOrigin).port] : []),
    ], { env, cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
    result.supervisorPID = supervisor.pid;
    supervisor.stdout.on('data', chunk => { log = (log + chunk).slice(-65536); stdout = (stdout + chunk).slice(-65536); });
    supervisor.stderr.on('data', chunk => { log = (log + chunk).slice(-65536); });
    supervisor.on('error', error => { log += error.message; });
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      if (supervisor.exitCode !== null || supervisor.signalCode) throw new Error('Native observer exited: ' + log);
      const match = stdout.match(/IRIS_NATIVE_READY (\{[^\n]+\})/);
      if (match) result.appPID = JSON.parse(match[1]).app_pid;
      try {
        const snapshot = await result.snapshot();
        if ((!application || (await result.applicationState())?.runtime) && stdout.includes('fuse-hl: listening') && result.appPID && snapshot.processes.some(process => process.pid === result.appPID && process.loci.length > 0 && process.records > 0)) { ready = true; break; }
      } catch { }
      await pause(50);
    }
    if (!ready) throw new Error('Native observer readiness failed: ' + log + '\nRegistry: ' + JSON.stringify(await readdir(path.join(root, 'hale'))) + '\nApplication: ' + JSON.stringify(application ? await result.applicationState().catch(() => null) : null) + '\nSnapshot: ' + JSON.stringify(await result.snapshot().catch(() => null)));
    return result;
  } catch (error) { await result.close(); throw error; }
}
