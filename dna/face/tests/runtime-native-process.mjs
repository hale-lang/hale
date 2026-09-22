// Internal test supervisor, invoked only inside the private bwrap namespace.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { boundedNative } from './environment.mjs';

const [root, observerBinary, appBinary, web, port, applicationPort] = process.argv.slice(2);
const children = [];
const intentionalStops = new Set();
const start = (binary, args, extra = {}) => {
  const bounded = boundedNative(binary, args, { lock: false });
  const child = spawn(bounded.command, bounded.args, { cwd: root, env: { ...process.env, ...extra }, stdio: ['ignore', 'pipe', 'pipe'] });
  child.output = '';
  child.stdout.on('data', chunk => { child.output = (child.output + chunk).slice(-65536); process.stdout.write(chunk); });
  child.stderr.on('data', chunk => process.stderr.write(chunk));
  child.once('error', error => { console.error(error.message); void shutdown(1); });
  child.once('exit', (code, signal) => {
    if (!closing && !intentionalStops.has(child)) {
      console.error(`Native fixture exited unexpectedly: ${code ?? signal}`);
      void shutdown(1);
    }
  });
  children.push(child);
  return child;
};
const stop = async child => {
  if (!child || child.exitCode !== null || child.signalCode) return;
  intentionalStops.add(child);
  const closed = new Promise(resolve => child.once('close', resolve));
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 1000);
  await closed;
  clearTimeout(timer);
};
let closing = false;
async function shutdown(code = 0) {
  if (closing) return;
  closing = true;
  for (const child of children) await stop(child);
  process.exit(code);
}
process.on('SIGTERM', () => void shutdown());
process.on('SIGINT', () => void shutdown());
process.on('exit', () => { for (const child of children) if (child.exitCode === null) child.kill('SIGKILL'); });

const observer = start(observerBinary, [port, web]);
let app;
async function startApp() {
  app = start(appBinary, applicationPort ? ['run', root + '/application.sqlite', 'operator'] : [], { LOTUS_OBS: '1' });
  if (applicationPort) {
    const deadline = Date.now() + 5000;
    while (!/application_id=[a-f0-9]{64} incarnation_id=[a-f0-9]{64}/.test(app.output)) {
      if (Date.now() > deadline || app.exitCode !== null || app.signalCode) throw new Error('Application did not start.');
      await new Promise(resolve => setTimeout(resolve, 25));
    }
  }
}
await startApp();
if (applicationPort) start(appBinary, ['serve', root + '/application.sqlite', applicationPort, web, 'operator'], { HALE_IRIS_OBSERVER_ORIGIN: 'http://127.0.0.1:' + port });
console.log('IRIS_NATIVE_READY ' + JSON.stringify({ app_pid: app.pid }));
const input = createInterface({ input: process.stdin });
input.on('line', async line => {
  if (line === 'stop-observer') { await stop(observer); console.log('IRIS_OBSERVER_STOPPED'); }
  if (line === 'restart-app') { await stop(app); await startApp(); console.log('IRIS_APP_RESTARTED ' + JSON.stringify({ app_pid: app.pid })); }
  if (line === 'stop-app') { await stop(app); console.log('IRIS_APP_STOPPED'); }
  if (line === 'stop') await shutdown();
});
input.on('close', () => void shutdown());
