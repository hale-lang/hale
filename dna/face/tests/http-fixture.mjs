// Browser transport fixtures only: a loopback HTTP server, and a static host
// that serves the face's ten assets with the head's headers and nothing else.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const web = fileURLToPath(new URL('../web/', import.meta.url));

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

export async function faceHost() {
  const requests = [];
  const assets = new Map(await Promise.all(['index.html', 'app.js', 'application.js', 'definition-draft.js', 'organization-draft.js', 'knowledge-draft.js', 'task-administration.js', 'projects.js', 'task-create.js', 'styles.css'].map(async name => [name, await readFile(path.join(web, name))])));
  const server = await httpFixture((req, res) => {
    requests.push(req.url);
    const headers = {
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      'referrer-policy': 'no-referrer',
      'content-security-policy': "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    };
    const name = req.url === '/' ? 'index.html' : req.url.slice(1);
    if (!assets.has(name)) { res.writeHead(404, headers).end(); return; }
    const contentType = name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html';
    res.writeHead(200, { ...headers, 'content-type': contentType });
    res.end(assets.get(name));
  });
  return { ...server, requests };
}
