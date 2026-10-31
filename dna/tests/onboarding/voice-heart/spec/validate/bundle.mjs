// Hold the api's exported surface (api/bundle/Voice.openapi.json) to the
// contract (spec/openapi.yaml, bundled): each row type against the
// contract schema it stands for, field by field. Every difference is a
// gap, classed by what closes it; the classed gaps are committed in
// api/gaps.json, and a run fails when the gaps found and the gaps
// committed differ, either way: a new difference, or a closed one still
// listed.
//
// It fails closed: every exported component is mapped to the contract,
// reached inside a mapped one, or on NOT_COMPARED with a reviewed reason;
// a contract shape with no single type is a gap of its own (`open`); a
// scalar of another type is classed only by a reviewed rule; enum values
// are compared when both sides have them; and `required` is checked both
// ways (a field an answer requires is one voice always sends). A row's
// optional field (a presence pair, `anyOf` of its schema and null) is the
// contract's nullable field, or one it may leave out.
//
//   usage: node bundle.mjs <bundled-contract.json> <Voice.openapi.json> <gaps.json> [--write]
import fs from 'fs';

const [contractPath, bundlePath, gapsPath, mode] = process.argv.slice(2);
const contract = JSON.parse(fs.readFileSync(contractPath, 'utf8'));
const bundle = JSON.parse(fs.readFileSync(bundlePath, 'utf8'));

// What closes each kind of gap.
const WAITS_ON = {
  optional: 'hale-lang/hale#1600: optional fields (a value or null)',
  enum: 'hale step 0: enums',
  map: 'voice: a free-form JSON object the row carries as text: std::json::Value carries it (hale v0.26.0); none today',
  wrapper: 'hale: a bounded field assigned whole (FRICTION.md); the list rides in a record',
  'json-text': 'voice: a typed object the row carries as free-form JSON (std::json::Value)',
  path: 'voice: a path or query parameter carried in the row\'s request',
  'map-values': 'voice: a map whose values one side says and the other does not, so they are not compared',
  wire: 'voice: the REST wire\'s shape differs from the row (written by api/wire.hl)',
  required: 'voice: the row gives the field a default, so the surface does not require it; the REST reader does (api/wire.hl)',
  open: 'voice: the contract admits several shapes there (oneOf/anyOf, or none named); the row carries one, as the REST reader does',
};

// Each row type, and the contract schema it stands for: a component name,
// or METHOD path for an inline response.
const MAP = {
  Health: 'Health',
  CatalogModel: 'CatalogModel',
  CatalogModelCreate: 'CatalogModelCreate',
  CatalogModelUpdate: 'CatalogModelUpdate',
  CatalogList: 'GET /admin/v1/models',
  Model: 'Model',
  ModelList: 'ModelList',
  Capabilities: 'Capabilities',
  Project: 'Project',
  ProjectCreate: 'ProjectCreate',
  ProjectUpdate: 'ProjectUpdate',
  ProjectList: 'GET /admin/v1/projects',
  Key: 'KeyWithSecret',
  KeyCreate: 'KeyCreate',
  KeyUpdate: 'KeyUpdate',
  KeyList: 'GET /admin/v1/projects/{project}/keys',
  Budget: 'Budget',
  'BudgetSet': 'BudgetSet',
  LimitSet: 'LimitSet',
  'Limit': 'Limit',
  'ModelPrice': 'ModelPrice',
  Account: 'Account',
  AccountCreate: 'AccountCreate',
  AccountUpdate: 'AccountUpdate',
  AccountList: 'GET /admin/v1/accounts',
  Node: 'NodeWithToken',
  NodeCreate: 'NodeCreate',
  NodeUpdate: 'NodeUpdate',
  NodeList: 'GET /admin/v1/nodes',
  SeatView: 'SeatView',
  SeatCreate: 'Seat',
  SeatUpdate: 'SeatUpdate',
  SeatList: 'GET /admin/v1/nodes/{node}/seats',
  'SeatModel': 'SeatModel',
  Change: 'Change',
  ChangeList: 'GET /admin/v1/changes',
  Attempt: 'Attempt',
  VoiceExtension: 'VoiceResponseExtension',
  ServedBy: 'Served',
  Served: 'Response',
  InferenceRequest: 'CreateResponseRequest',
  Registration: 'Registration',
  Registered: 'Registered',
  Assignment: 'Assignment',
  AssignedSeat: 'Seat',
  SeatState: 'SeatState',
  SeatStates: 'SeatStates',
  Heartbeat: 'Heartbeat',
  QuotaReport: 'QuotaReport',
  Draining: 'Draining',
  ApiError: 'ErrorResponse.error',
  BudgetRequest: 'BudgetSet',
  LimitsRequest: 'LimitSet',
};

// Exported components compared with nothing, each with why, and the only
// fields its reason allows it: a component that grows a field its reason
// does not cover (a body field on a path record) fails the run.
const NOT_COMPARED = {
  AccountRef: ['the path parameter `account`; no body', ['account']],
  KeyRef: ['the path parameters `project` and `key`; no body', ['project', 'key']],
  ModelRef: ['the path parameter `model`; no body', ['model']],
  NodeRef: ['the path parameter `node`; no body', ['node']],
  ProjectRef: ['the path parameter `project`; no body', ['project']],
  SeatRef: ['the path parameters `node` and `seat`; no body', ['node', 'seat']],
  AccountQuery: ['the query parameter `include_archived`; no body', ['include_archived']],
  ProjectQuery: ['the query parameter `include_archived`; no body', ['include_archived']],
  'ChangeQuery': ['the query parameters of `GET /admin/v1/changes` (`target`, `from`, `to`, `limit`, `cursor`, with `has_from` and `has_to` for a `from` or `to` given); no body', ['target', 'has_from', 'from', 'has_to', 'to', 'limit', 'cursor']],
  Check: ['an element of a map of values (`Health.checks`): it crosses as its `status` alone under its `name`, so its schema is never on the wire; the map\'s value is compared as `Health.checks{}`', ['name', 'status']],
  'hale.Refusal': ['hale\'s own refusal of a malformed rpc (std::api), not part of voice\'s contract', ['refusal']],
};

// Fields of a row's request that are the path's or the query's.
const PARAMS = new Set(['model', 'project', 'key', 'account', 'node', 'seat', 'include_archived']);
// Records that stand for a list: the list rides in `items`. A row's list
// is a `bounded` field of its own; these are left only inside a record
// that stands for a value or null (`Restriction.names`).
const WRAPPERS = new Set(['Names', 'Limits', 'SeatModels', 'SeatViews', 'ModelIds', 'AppliedLimits', 'Problems', 'Attempts', 'AssignedSeats', 'SeatStateList']);
// Records that stand for a value or null.
const NULLABLES = new Set(['Restriction']);
// Rows only voice writes: answers. A field such a row requires and the
// contract does not is one voice always sends, which the contract allows;
// on a request it would be one the row refuses to go without.
const ANSWERS = new Set(['Health', 'CatalogModel', 'CatalogList', 'Model', 'ModelList', 'Capabilities', 'Project', 'ProjectList', 'Key', 'KeyList', 'Budget', 'LimitSet', 'Account', 'AccountList', 'Node', 'NodeList', 'SeatView', 'SeatList', 'Change', 'ChangeList', 'Attempt', 'VoiceExtension', 'ServedBy', 'Served', 'Registered', 'Assignment', 'AssignedSeat', 'ApiError']);

const bschemas = bundle.components.schemas;
const cschemas = contract.components.schemas;
const gaps = [];
const gap = (at, kind) => { if (!gaps.some(g => g.at === at)) gaps.push({ at, kind }); };
// the exported components a comparison reached
const reached = new Set();

function bresolve(s) {
  while (s && s.$ref) {
    const n = s.$ref.replace('#/components/schemas/', '');
    reached.add(n);
    s = bschemas[n];
  }
  return s || {};
}
function bname(s) { return s && s.$ref ? s.$ref.replace('#/components/schemas/', '') : ''; }

// A contract schema with allOf merged, and whether it admits null.
function cflat(s) {
  if (!s) return { s: {}, nullable: false };
  let nullable = false;
  if (Array.isArray(s.type) && s.type.includes('null')) { nullable = true; s = { ...s, type: s.type.filter(t => t !== 'null')[0] }; }
  // `oneOf` or `anyOf` of one shape and `null` is that shape, nullable
  for (const key of ['oneOf', 'anyOf']) {
    if (!s[key]) continue;
    const rest = s[key].filter(o => o.type !== 'null');
    if (rest.length < s[key].length) nullable = true;
    if (rest.length === 1) { const r = cflat(rest[0]); return { s: r.s, nullable: nullable || r.nullable }; }
  }
  if (s.allOf) {
    const merged = { type: 'object', properties: {}, required: [] };
    for (const part of s.allOf) {
      const p = cflat(part).s;
      Object.assign(merged.properties, p.properties || {});
      merged.required.push(...(p.required || []));
    }
    return { s: merged, nullable };
  }
  return { s, nullable };
}
const ctype = s => s.type || (s.properties ? 'object' : s.items ? 'array' : undefined);

// A row's optional field (a presence pair): its value's schema, or null.
function bflat(b) {
  const s = b && b.$ref ? b : b || {};
  if (Array.isArray(s.anyOf)) {
    const rest = s.anyOf.filter(o => o.type !== 'null');
    if (rest.length === 1 && rest.length < s.anyOf.length) return { b: rest[0], nullable: true };
  }
  return { b, nullable: false };
}

function compare(at, c, bIn) {
  const { s: cs, nullable } = cflat(c);
  // a row's field may be left out or null when the contract's may not be
  // null: absence is what the contract allows there, and a null reads as it
  const { b, nullable: bnullable } = bflat(bIn);
  const name = bname(b);
  if (name) reached.add(name);
  if (NULLABLES.has(name)) { gap(at, 'optional'); return; }
  if (WRAPPERS.has(name)) {
    if (ctype(cs) === 'array') gap(at, 'wrapper'); else gap(at, 'wire');
    return;
  }
  const bs = bresolve(b);
  if (nullable && !bnullable) gap(at, 'optional');
  const ct = ctype(cs), bt = bs.type;
  if (cs.enum && bs.enum) {
    const a = [...cs.enum].sort().join(','), z = [...bs.enum].sort().join(',');
    if (a !== z) gap(at + ' (enum values)', 'enum-values');
  } else if (cs.enum && bt === 'string') { gap(at + ' (enum)', 'enum'); return; }
  // a narrowing: the row refuses values the contract allows
  else if (bs.enum && !cs.enum) { gap(at + ' (narrowed to an enum)', 'type'); return; }
  // free-form JSON (`std::json::Value`, `x-hale-any`): the contract's
  // free-form object is what it carries, byte for byte; a contract that
  // names the object's fields is a shape the row does not check
  if (bs['x-hale-any'] === true) {
    if (ct === 'object' && (!cs.properties || cs.additionalProperties)) return;
    gap(at, 'json-text');
    return;
  }
  // a contract shape with no single type: several, or none named
  if (ct === undefined) { gap(at, 'open'); return; }
  if (ct === 'object' && bt === 'string') {
    const freeform = !cs.properties || cs.additionalProperties;
    gap(at, freeform ? 'map' : 'json-text');
    return;
  }
  if (ct === 'array' && bt === 'string') { gap(at, 'json-text'); return; }
  if (ct === 'object' && bt === 'object') {
    // a map's values: compared when both sides say them, a gap of its own
    // when only one does (a boolean says no schema)
    const ca = cs.additionalProperties, ba = bs.additionalProperties;
    const cmap = !!ca && typeof ca === 'object', bmap = !!ba && typeof ba === 'object';
    if (cmap && bmap) compare(at + '{}', ca, ba);
    else if (cmap !== bmap) gap(at + '{}', 'map-values');
    fields(at, cs, bs);
    return;
  }
  if (ct === 'array' && bt === 'array') { compare(at + '[]', cs.items, bs.items); return; }
  // an integer where the contract says number is a narrowing too
  if (ct === 'number' && bt === 'integer') { gap(at + ' (narrowed to an integer)', 'type'); return; }
  if (ct !== bt) gap(at, 'type');
}

function fields(at, cs, bs) {
  const cp = cs.properties || {}, bp = bs.properties || {};
  const breq = new Set(bs.required || []);
  const creq = new Set(cs.required || []);
  for (const k of Object.keys(cp)) {
    // a read-only field is the server's to write: compared where the row
    // has it (an answer), skipped where it has not (a request)
    if (cp[k].readOnly && !(k in bp)) continue;
    if (!(k in bp)) gap(`${at}.${k}`, 'missing');
    else {
      if (creq.has(k) && !breq.has(k)) gap(`${at}.${k} (required)`, 'required');
      // a field the row requires and the contract does not: the row
      // would refuse what the contract allows
      if (breq.has(k) && !creq.has(k) && !ANSWERS.has(at.split(/[.[ ]/)[0])) gap(`${at}.${k} (required by the row)`, 'over-required');
      compare(`${at}.${k}`, cp[k], bp[k]);
    }
  }
  for (const k of Object.keys(bp)) {
    if (k in cp) continue;
    if (PARAMS.has(k)) gap(`${at}.${k}`, 'path');
    else gap(`${at}.${k}`, 'extra');
  }
}

for (const [row, target] of Object.entries(MAP)) {
  const b = bschemas[row];
  if (!b) { gap(row, 'unmapped'); continue; }
  let c;
  if (target.includes(' ')) {
    const [method, path] = target.split(' ');
    c = contract.paths[path][method.toLowerCase()].responses['200'].content['application/json'].schema;
  } else if (target.includes('.')) {
    // a component's property: `ErrorResponse.error`
    const [comp, prop] = target.split('.');
    c = cschemas[comp] && cflat(cschemas[comp]).s.properties?.[prop];
  } else c = cschemas[target];
  if (!c) { gap(row + ' -> ' + target, 'unmapped'); continue; }
  compare(row, c, { $ref: '#/components/schemas/' + row });
}
for (const n of Object.keys(bschemas)) if (n.includes('::')) gap('component ' + n, 'name');
// every exported component is compared, or says why not
const unaccounted = Object.keys(bschemas).filter(n => !(n in MAP) && !reached.has(n) && !(n in NOT_COMPARED) && !WRAPPERS.has(n) && !NULLABLES.has(n));
const stale = Object.keys(NOT_COMPARED).filter(n => !(n in bschemas) || n in MAP || reached.has(n));
// held to its reason: no field beyond the ones it names
const beyond = Object.entries(NOT_COMPARED).flatMap(([n, [, only]]) =>
  n in bschemas ? Object.keys(bresolve(bschemas[n]).properties || {}).filter(k => !only.includes(k)).map(k => `${n}.${k}`) : []);

// A field the row has and the contract does not, or the reverse, classed
// by why: the first rule whose pattern the gap's place matches.
const RULES = [
  [/\.(used|revoked|archived|connected|beating|ends|budgeted|anchored)$/, 'optional', 'the null of the field beside it'],
  [/\.(created_by|principal|exhausted_until|output_tokens_before_failure|quantization)$/, 'optional', 'a field the api leaves null or absent for now'],
  [/List\.data\[\]\.(secret|enrollment_token)$/, 'wire', 'the list shares the type of the create answer; the wire leaves the secret out'],
  [/^(Model|ModelList)\b/, 'wire', 'the caller\'s model nests voice\'s fields under `voice`, and lists say `object`'],
  [/^(Budget|(ProjectCreate|Project|ProjectList\.data\[\]|Capabilities|ApiError\.voice)\.budget)\./, 'wire', 'a budget is its BudgetSet flattened into it on the wire'],
  [/^Registered\.nats/, 'wire', '`nats` is an object of url and subject on the wire'],
  [/(VoiceExtension|Served\.voice)\.(wall_ms|timing)$/, 'wire', 'the timing is an object of wall, queue and load on the wire'],
  [/^Health\.(checks|store)$/, 'map', '`checks` is a map of dependency to state'],
  [/^Capabilities\.(project_id|project_name|key_id|key_name|project|key)$/, 'wire', 'the project and the key are objects of id and name on the wire'],
  [/^ApiError\.(kind|type)$/, 'wire', 'the row\'s `kind` is the envelope\'s `type`, renamed by the wire (error_json)'],
  [/^ApiError\.(status|retry_after)$/, 'wire', 'the HTTP status the error answers with and its Retry-After header, not fields of the envelope'],
];
// Places whose every gap is the wire's, whatever the field: the standard
// request and response, which pass through as JSON text. `Served.voice`
// is the typed VoiceExtension, and is compared.
const WHOLE = [
  [/^(Served\.(?!voice\b)|InferenceRequest\.)/, 'the standard request and response pass through as JSON text (`body`, `response`)'],
];
// Kinds a rule may class; any other that no rule classes fails.
const RULED = new Set(['missing', 'extra', 'type', 'over-required']);
function reclass(at, kind) {
  // a shape the contract leaves open says why, wherever it is
  if (kind === 'open') {
    for (const [re, k, why, only] of RULES) if (only === 'open' && re.test(at)) return { kind: k, why };
    return { kind };
  }
  // a value or null is the optional gap wherever it is
  if (kind === 'optional') return { kind };
  for (const [re, why] of WHOLE) if (re.test(at)) return { kind: 'wire', why };
  if (!RULED.has(kind)) return { kind };
  for (const [re, k, why, only] of RULES) if ((!only || only === kind) && re.test(at)) return { kind: k, why };
  return { kind };
}
const committed = fs.existsSync(gapsPath) ? JSON.parse(fs.readFileSync(gapsPath, 'utf8')) : { gaps: [] };
const reasons = new Map(committed.gaps.map(g => [g.at, g]));
const found = gaps.map(g => {
  const r = reclass(g.at, g.kind);
  return { at: g.at, found: g.kind, kind: r.kind, waits_on: WAITS_ON[r.kind] || 'unclassed: say what closes it', ...(r.why ? { why: r.why } : {}) };
}).sort((a, b) => a.at.localeCompare(b.at));

if (mode === '--write') {
  const counts = {};
  for (const g of found) counts[g.kind] = (counts[g.kind] || 0) + 1;
  fs.writeFileSync(gapsPath, JSON.stringify({ note: 'Written by spec/validate/bundle.mjs --write; every gap between the exported Voice surface and spec/openapi.yaml, classed by what closes it.', counts, gaps: found }, null, 2) + '\n');
  console.log(`${found.length} gaps written`, counts);
  process.exit(0);
}

let fails = 0;
const committedAt = new Set(committed.gaps.map(g => g.at));
const foundAt = new Set(found.map(g => g.at));
for (const g of found) {
  if (!committedAt.has(g.at)) { fails++; console.log('NEW GAP', g.at, g.found); }
  else if (reasons.get(g.at).found !== g.found) { fails++; console.log('CHANGED', g.at, reasons.get(g.at).found, '->', g.found); }
}
for (const g of committed.gaps) if (!foundAt.has(g.at)) { fails++; console.log('CLOSED, still listed', g.at, g.kind); }
for (const g of found) if (!(g.kind in WAITS_ON)) { fails++; console.log('UNCLASSED', g.at, g.kind); }
for (const n of unaccounted) { fails++; console.log('NOT COMPARED, and no reason given', n); }
for (const n of stale) { fails++; console.log('NOT_COMPARED lists what is compared or gone', n); }
for (const f of beyond) { fails++; console.log('NOT_COMPARED, but a field its reason does not cover', f); }
const counts = {};
for (const g of found) counts[g.kind] = (counts[g.kind] || 0) + 1;
console.log(`${found.length} gaps, all committed and classed:`, fails === 0, counts);
process.exit(fails ? 1 : 0);
