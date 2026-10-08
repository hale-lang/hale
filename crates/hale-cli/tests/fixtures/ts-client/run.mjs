// GH #1417 (R8a): the generated TypeScript client speaks the contract's wire.
//
//   node crates/hale-cli/tests/fixtures/ts-client/run.mjs
//
// Environment: HALE_BIN (default target/release/hale) and TSC (default `tsc`).
// What it does, in order:
//
//   1. `hale api client --surface Public --lang ts` of the witness
//      (tests/api-contract/program.hl) into a scratch directory;
//   2. compiles it with `tsc --strict`, together with scenarios.ts, which uses
//      the client the way a consumer does (the tagged union narrows, an
//      exhaustive switch compiles, a payload is typed);
//   3. builds the replay server (tests/hale/api/client/replay) with the same
//      `hale`, serves the contract's recordings over HTTP and the hub's frames
//      over a WebSocket, and runs every outcome and every way a stream ends
//      through the compiled client on node's `fetch` and `WebSocket`.
//
// The replay server compares each request the client wrote to the recorded
// one (method, path, headers and body byte for byte, the credential of a hub
// upgrade) and writes a report; the client's own reading of each recorded
// reply is compared here. Exit 0 and one line when all hold.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, copyFileSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../../../..");
const hale = path.resolve(root, process.env.HALE_BIN ?? "target/release/hale");
const tsc = process.env.TSC ?? "tsc";
const wire = path.join(root, "tests/api-contract/wire");
const env = { ...process.env, HALE_SKIP_STALE_CHECK: "1" };

let checks = 0;
function same(actual, expected, what) {
  checks += 1;
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) {
    console.error(`FAIL ${what}\n  expected: ${e}\n  actual:   ${a}`);
    process.exit(1);
  }
}

function run(cmd, args, what) {
  const r = spawnSync(cmd, args, { encoding: "utf8", env, cwd: root });
  if (r.status !== 0) {
    console.error(`FAIL ${what}: ${cmd} ${args.join(" ")}\n${r.stdout}\n${r.stderr}`);
    process.exit(1);
  }
  return r;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
    s.on("error", reject);
  });
}

async function waitFor(file, what) {
  for (let i = 0; i < 500; i++) {
    if (existsSync(file)) return;
    await new Promise((r) => setTimeout(r, 20));
  }
  console.error(`FAIL ${what}: ${file} never appeared`);
  process.exit(1);
}

const scratch = mkdtempSync(path.join(tmpdir(), "hale-ts-client-"));
const children = [];
process.on("exit", () => {
  for (const c of children) c.kill("SIGKILL");
  rmSync(scratch, { recursive: true, force: true });
});

// 1 and 2: generate, and compile under --strict
const src = path.join(scratch, "src");
mkdirSync(src, { recursive: true });
run(hale, ["api", "client", "--surface", "Public", "--lang", "ts", "--out", path.join(src, "client.ts"), path.join(root, "tests/api-contract/program.hl")], "generate the client");
run(hale, ["api", "client", "--surface", "Public", "--lang", "ts", "--check", path.join(src, "client.ts"), path.join(root, "tests/api-contract/program.hl")], "the generated client is current");
copyFileSync(path.join(here, "scenarios.ts"), path.join(src, "scenarios.ts"));
const js = path.join(scratch, "js");
mkdirSync(js, { recursive: true });
writeFileSync(path.join(js, "package.json"), '{"type":"module"}\n');
run(tsc, ["--strict", "--noUnusedLocals", "--noImplicitReturns", "--target", "es2022", "--lib", "es2022,dom", "--module", "es2022", "--moduleResolution", "bundler", "--outDir", js, path.join(src, "client.ts"), path.join(src, "scenarios.ts")], "tsc --strict");

// 3: the replay server
const replaySrc = path.join(scratch, "replay");
mkdirSync(replaySrc, { recursive: true });
copyFileSync(path.join(root, "tests/hale/api/client/replay/main.hl"), path.join(replaySrc, "main.hl"));
run(hale, ["build", replaySrc], "build the replay server");
const replay = path.join(replaySrc, "replay");

function serve(args, report) {
  const child = spawn(replay, args, { stdio: "ignore", env });
  children.push(child);
  return child;
}

const scenarios = await import(pathToFileURL(path.join(js, "scenarios.js")).href);

same(scenarios.digest(), "fnv1a64:a8930d6e7998e986", "the client names Public's digest");

// HTTP: every outcome of wire/http
{
  const port = await freePort();
  const report = path.join(scratch, "http.report");
  serve(["http", wire, String(port), report, "result", "handler_error", "server_error", "refusal_digest_mismatch", "refusal_full", "~refusal_malformed", "refusal_shutting_down", "refusal_unauthenticated", "refusal_unauthorized", "refusal_unavailable"]);
  await waitFor(report + ".ready", "the HTTP replay server");
  const got = await scenarios.http(`http://127.0.0.1:${port}`);
  same(got, [
    "result:41:125000",
    "handler_error:unknown_order:no order 999",
    "server_error",
    "refusal:digest_mismatch:the request was generated against fnv1a64:0123456789abcdef::fnv1a64:a8930d6e7998e986",
    "refusal:full:the exposure holds its bound of requests accepted and not yet answered::",
    "refusal:malformed:missing_field: qty::",
    "refusal:shutting_down:the exposure is stopping::",
    "refusal:unauthenticated:no such token::",
    "refusal:unauthorized:Orders::cancel requires trader:trader:",
    "refusal:unavailable:the receiver of Orders::place is unavailable; the call did not run::",
  ], "the outcomes of wire/http");
  await waitFor(report, "the HTTP report");
  same(readFileSync(report, "utf8"), "ok result\nok handler_error\nok server_error\nok refusal_digest_mismatch\nok refusal_full\nok refusal_malformed\nok refusal_shutting_down\nok refusal_unauthenticated\nok refusal_unauthorized\nok refusal_unavailable\n", "every request the client wrote is the recorded one");
  // nothing listening is a thrown TransportError, never an outcome
  same(await scenarios.httpLost(`http://127.0.0.1:${await freePort()}`), "lost", "a connection that is not answered is thrown");
}

// the hub: each way a subscription ends
{
  const port = await freePort();
  const report = path.join(scratch, "ws.report");
  serve(["ws", String(port), report, "closed", "revoked", "expired", "refused", "abort", "late"]);
  await waitFor(report + ".ready", "the hub replay server");
  const endpoint = `ws://127.0.0.1:${port}`;
  const event = "event:1:41:10:12500";
  same(await scenarios.stream(endpoint), ["subscribed", event, "event:3:42:5:12400", "closed"], "closed ends it, a gap in seq kept");
  same(await scenarios.stream(endpoint), ["subscribed", event, "revoked"], "revoked ends it");
  same(await scenarios.stream(endpoint), ["subscribed", event, "expired"], "expired ends it");
  same(await scenarios.stream(endpoint), ["refusal:unauthorized:Fills requires operator:operator:"], "a refusal names the role");
  same(await scenarios.stream(endpoint), ["subscribed", event, "lost"], "no closed frame is a thrown TransportError");
  same(await scenarios.stream(endpoint), ["subscribed", event, "closed"], "an event that comes late comes");
  await waitFor(report, "the hub report");
  same(readFileSync(report, "utf8"), "ok closed\nok revoked\nok expired\nok refused\nok abort\nok late\n", "every subscribe frame and credential the client wrote is the contract's");
}

console.log(`ts client: ${checks} checks passed`);
