// A record kept in memory, as the browser fixtures read and write it (GH
// #1590). The record is memory's: its rows are `record_rows` in the record's
// own schema (`dna_` and the first 56 characters of its identity, the pin in
// the project's hale.toml), and the Ledger's are `ledger_rows` beside them.
// A fixture reads them, and appends a record row in a person's name through
// memory's insert function, as the owner (HALE_DNA_MEMORY_DSN_OWNER, the
// runner's own: it never reaches a process a fixture starts). Values reach
// psql as variables, never spliced into the statement.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { memoryOwner } from './environment.mjs';

/** The record's identity: the `record = "<hex>"` pin in the project's hale.toml. */
export function recordIdentity(root) {
  const text = fs.readFileSync(path.join(root, 'hale.toml'), 'utf8');
  const pin = /^\s*record\s*=\s*"([0-9a-f]{64})"\s*$/m.exec(text);
  if (!pin) throw new Error(`${root}/hale.toml pins no record`);
  return pin[1];
}

/** The schema memory keeps a record in (dna/core/memory_store.hl `schema_for`). */
export const recordSchema = id => 'dna_' + id.slice(0, 56).toLowerCase();

// One statement in the record's schema as the owner; `vars` are psql
// variables the statement names as :'name'.
function psql(root, sql, vars = {}) {
  const owner = memoryOwner();
  if (!owner) throw new Error('The record is memory\'s: set HALE_DNA_MEMORY_DSN_OWNER');
  const args = [owner, '-tAq', '-v', 'ON_ERROR_STOP=1', '-v', `sch=${recordSchema(recordIdentity(root))}`];
  for (const [name, value] of Object.entries(vars)) args.push('-v', `${name}=${value}`);
  return execFileSync('psql', args, { input: `SET search_path TO :"sch", public;\n${sql}\n`, encoding: 'utf8', timeout: 15_000, maxBuffer: 64 * 1024 * 1024 });
}

// A row as the record's own line format writes it (`dna::row_line`), so a
// fixture's text matches what `hale dna record-rows` prints.
const line = row => `{"seq": ${row.seq}, "kind": ${JSON.stringify(row.kind)}, "entity": ${JSON.stringify(row.entity)}, "body": ${JSON.stringify(row.body)}, "author": ${JSON.stringify(row.author)}}`;

function rowsOf(root, table) {
  const out = psql(root, `SELECT json_build_object('seq', seq, 'kind', kind, 'entity', entity, 'body', body, 'author', author, 'digest', digest, 'prev', prev)::text FROM ${table} ORDER BY seq;`);
  return out.split('\n').filter(Boolean).map(text => JSON.parse(text));
}

/** The record: its head (the last row's digest), its text in the line format, and its rows (with `digest`). */
export function recordState(root) {
  const rows = rowsOf(root, 'record_rows');
  return { head: rows.length ? rows.at(-1).digest : 'genesis', text: rows.map(line).join('\n') + (rows.length ? '\n' : ''), rows };
}

/** The Ledger's rows (Task, attempt, receipt and lease rows are the Ledger's). */
export const ledgerRows = root => rowsOf(root, 'ledger_rows');

/** A body memory keeps unsealed under `digest` (`sha256:<hex>`), or null. */
export function receiptBody(root, digest) {
  const out = psql(root, `SELECT coalesce(receipt_read(:'digest')::text, '');`, { digest: digest.startsWith('sha256:') ? digest : 'sha256:' + digest }).trim();
  return out ? JSON.parse(out).text : null;
}

/** One record row in `author`'s name, at the record's tail, through memory's insert function. */
export function appendRecordRow(root, kind, entity, body, author = 'dna') {
  const out = psql(root, `SELECT record_append(-1, coalesce((SELECT digest FROM record_rows ORDER BY seq DESC LIMIT 1), 'genesis'), :'kind', :'entity', :'body', :'author', '', '');`, { kind, entity, body, author }).trim();
  const answer = JSON.parse(out);
  if (answer.error !== '') throw new Error(`memory refused ${kind} ${entity}: ${out}`);
  return answer.digest;
}
