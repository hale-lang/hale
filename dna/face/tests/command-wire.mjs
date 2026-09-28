import net from 'node:net';

// The head's command wire as the face meets it (GH #1104, #1135): one line
// of the api binding's wire POSTed to …/commands, the binding's own HTTP
// transport under the session's bearer, and its receipt line back — a
// lookup included, as a `CommandLookup` call. Scripted lanes build their
// answers here so every lane scripts the one shape the head speaks.

// The call each operation is sent as.
export const CALLS = {
  'dna.practice.propose': 'PracticePropose', 'dna.review.verdict': 'ReviewVerdict',
  'dna.organization.propose': 'OrganizationPropose', 'dna.task.reassign': 'TaskReassign',
  'dna.person.retire': 'PersonRetire', 'dna.task.create': 'TaskCreate',
};
// The head's Knowledge topics (dna/api/knowledge_commands.hl), and their
// recovery call.
export const KNOWLEDGE_CALLS = {
  'dna.knowledge.edge.link': 'KnowledgeEdgeLink', 'dna.knowledge.edge.unlink': 'KnowledgeEdgeUnlink',
  'dna.knowledge.node.propose': 'KnowledgeNodePropose', 'dna.knowledge.node.revise': 'KnowledgeNodeRevise', 'dna.knowledge.node.retire': 'KnowledgeNodeRetire',
  'dna.knowledge.binding.bind': 'KnowledgeBindingBind', 'dna.knowledge.binding.unbind': 'KnowledgeBindingUnbind',
};
export const KNOWLEDGE_LOOKUP = 'KnowledgeLookup';
// The commands every authenticated peer may send.
export const UNGATED = ['CommandLookup', 'TaskCreate', KNOWLEDGE_LOOKUP];
// The call a line names ('' for a describe or a GET).
export function callOf(request) {
  if (request.method() !== 'POST' || !new URL(request.url()).pathname.endsWith('/commands')) return '';
  try { return request.postDataJSON()?.call || ''; } catch { return ''; }
}
export const isKnowledgeCall = request => Object.values(KNOWLEDGE_CALLS).includes(callOf(request));
export const isKnowledgeLookup = request => callOf(request) === KNOWLEDGE_LOOKUP;
// A recovery: the request id a `CommandLookup` line asks after ('' for
// any other request).
export function lookupOf(request) {
  if (callOf(request) !== 'CommandLookup') return '';
  try { return request.postDataJSON()?.payload?.request_id || ''; } catch { return ''; }
}

// A describe line asks for the caller's slice, and a lookup recovers a
// receipt. Both read; lanes that watch for writes leave them out.
export function isDescribe(request) {
  if (request.method() !== 'POST' || !new URL(request.url()).pathname.endsWith('/commands')) return false;
  try { return request.postDataJSON()?.describe === true; } catch { return false; }
}
export const isWrite = request => request.method() !== 'GET' && !isDescribe(request) && callOf(request) !== 'CommandLookup';

// The session's caller, as the binding reports a line over HTTP: the
// bearer's principal (a trusted-local session's is the head's account).
export const CALLER = { mode: 'bearer', name: 'uid:1000', uid: 1000, gid: -1, pid: -1, via: 'http' };
let sequence = 0;
export function receiptLine(value, { role } = {}) {
  return { request_id: ++sequence, ok: true, value, caller: CALLER, ...(role ? { role } : {}) };
}
export function refusalLine(kind, reason = kind) {
  return { request_id: ++sequence, ok: false, refusal: { kind, reason }, caller: CALLER };
}
export const REFUSAL_STATUS = { unauthenticated: 401, unauthorized: 403, unknown: 404, over_bound: 503 };
export const refusalStatus = kind => REFUSAL_STATUS[kind] || 400;

// The describe value: only the command entries matter to the face.
export function describeLine(names) {
  return receiptLine({
    hale_api: 1, app: 'Head', notes: {},
    commands: names.map(name => ({ name, subject: 'dna.commands.' + name, payload: name, reply: 'CommandReply', keyed_by: null, role: null })),
    reads: [], streams: [], schemas: {},
  });
}

// A CommandReceipt with every typed branch present, as the head writes it.
export function commandReceipt(fields = {}) {
  const { organization = {}, task = {}, person = {}, task_create = {}, attempt = {}, ...flat } = fields;
  return {
    command_id: '', request_id: '', application_id: '', operation: 'dna.practice.propose', operation_version: '1',
    principal_mode: '', principal_name: '', principal_positions: '', position_id: 'org', target_kind: 'dna.practice', target_id: '',
    subject_digest: '', fingerprint: '', state: '', reason: '', proposal_state: 'pending', verdict_value: '',
    verdict_state: '', candidate_digest: '', review_id: '', review_state: 'unavailable', review_outcome: '',
    review_subject_digest: '', activation_state: 'unknown', activation_reason: '', ...flat,
    organization: { proposal_state: 'pending', source_head: '', source_digest: '', mutation_id: '', candidate_commit: '', application_state: 'pending', application_reason_code: '', restart_handoff_state: 'pending', ...organization },
    task: { state: '', from: '', to: '', event_id: '', ...task },
    person: { state: '', from: '', to: '', event_id: '', transferred: -1, ...person },
    task_create: { intent_id: '', intent_state: '', task_id: '', event_id: '', kind: '', ...task_create },
    attempt: { state: '', attempt_id: '', work_id: '', task_id: '', performer_kind: '', holder: '', token: -1, until: 0, disposition: '', reason: '', event_id: '', ...attempt },
  };
}
// The provider's answer around a receipt: `source` is a Record envelope's
// source ({record_id, record_head, record_revision}).
export function commandReply(receipt, source, { ok = true, code = '' } = {}) {
  return { ok, code, application_id: ok ? source.record_id : '', head: ok ? source.record_head : '', revision: ok ? Number(source.record_revision) : 0, receipt };
}
// A provider refusal: `ok:false` with the code, the receipt empty.
export const refusedReply = code => ({ ok: false, code, application_id: '', head: '', revision: 0, receipt: commandReceipt() });

// The route's own error envelope (no socket, OIDC session, bad headers).
export const routeError = (code, message = code) => ({ api_version: 'hale.v1', error: { code, message, retryable: code === 'commands_unavailable' } });

// The wire line for a command written in the old HTTP envelope's terms
// (operation, target, preconditions, arguments): the harnesses that build
// commands that way send exactly what the face sends.
export function wireLine(command) {
  const { request_id, operation, target = {}, preconditions: p = {}, arguments: a = {} } = command;
  const payloads = {
    'dna.practice.propose': () => ({ request_id, subject_digest: p.subject_digest, text: a.text, rationale: a.rationale }),
    'dna.review.verdict': () => ({ request_id, review_id: target.id, subject_digest: p.subject_digest, verdict: a.verdict, comment: a.comment }),
    'dna.organization.propose': () => ({ request_id, ...p.base, source_text: a.source_text, rationale: a.rationale }),
    'dna.task.reassign': () => ({ request_id, task_id: target.id, assignment_digest: p.subject_digest, assignee: p.assignee, to: a.to }),
    'dna.person.retire': () => ({ request_id, person: target.id, subject_digest: p.subject_digest, to: a.to }),
    'dna.task.create': () => ({ request_id, record_head: p.record_head, outcome: a.outcome, to: a.to, ...(a.kind ? { kind: a.kind } : {}) }),
  };
  // A Knowledge change: the head derives every target but an edge's.
  if (Object.hasOwn(KNOWLEDGE_CALLS, operation)) {
    const edge = operation.startsWith('dna.knowledge.edge.');
    return { call: KNOWLEDGE_CALLS[operation], payload: { request_id, record_head: p.record_head, ...(edge ? { target_id: target.id } : {}), ...a } };
  }
  if (!Object.hasOwn(payloads, operation)) throw new Error('No call carries ' + operation);
  return { call: CALLS[operation], payload: payloads[operation]() };
}
export const knowledgeLookupLine = requestId => ({ call: KNOWLEDGE_LOOKUP, payload: { request_id: requestId } });
// A KnowledgeReply's typed receipt in the old HTTP receipt's terms
// (principal, context, target, a decimal sequence, only the operation's own
// outcome branch), so a lane keeps asserting the Record facts it did.
export function knowledgeReceiptView(r) {
  const node = r.operation.startsWith('dna.knowledge.node.'), binding = r.operation.startsWith('dna.knowledge.binding.');
  return {
    command_id: r.command_id, request_id: r.request_id, application_id: r.application_id, operation: r.operation, operation_version: r.operation_version,
    principal: { mode: r.principal_mode, name: r.principal_name }, context: { application_id: r.application_id, position_id: r.position_id },
    target: { application_id: r.application_id, kind: r.operation === 'dna.knowledge.node.propose' ? 'dna.knowledge.collection' : 'dna.knowledge.node', id: r.target_id },
    fingerprint: r.fingerprint, state: r.state, details_visible: r.details_visible, edge_id: r.edge_id, event_id: r.event_id, sequence: String(r.sequence),
    admission_head: r.admission_head, authority: r.authority, authority_basis: r.authority_basis,
    ...(node ? { node: r.node } : binding ? { binding: r.binding } : r.reviewed ? { relationship: r.relationship } : {}),
  };
}
// A forwarded Knowledge exchange, settled: the binding's refusal kind, the
// provider's refusal code, or the receipt in the old terms.
export function settleKnowledge(status, json) {
  if (!json || typeof json.ok !== 'boolean') return { status, code: json?.error?.code || 'unanswered' };
  if (!json.ok) return { status, code: json.refusal.kind, refusal: json.refusal };
  if (!json.value.ok) return { status, code: json.value.code };
  return { status, code: '', reply: json.value, receipt: knowledgeReceiptView(json.value.receipt), source: { record_id: json.value.application_id, record_head: json.value.head, record_revision: String(json.value.revision) } };
}
// A receipt line's CommandReply receipt, grouped the way the old HTTP
// receipt was (principal, target, proposal, verdict, review, activation), so
// a harness can keep asserting on the Record facts it always did.
export function receiptView(r) {
  return {
    command_id: r.command_id, request_id: r.request_id, application_id: r.application_id, operation: r.operation, operation_version: r.operation_version,
    principal: { mode: r.principal_mode, name: r.principal_name }, context: { application_id: r.application_id, position_id: r.position_id },
    target: { application_id: r.application_id, kind: r.target_kind, id: r.target_id }, subject_digest: r.subject_digest, fingerprint: r.fingerprint,
    state: r.state, reason: r.reason,
    proposal: { state: r.proposal_state, candidate_digest: r.candidate_digest, review_id: r.review_id }, verdict: { value: r.verdict_value, state: r.verdict_state },
    review: { state: r.review_state, outcome: r.review_outcome, subject_digest: r.review_subject_digest }, activation: { state: r.activation_state, reason: r.activation_reason },
    organization: r.organization, task: r.task, task_create: r.task_create, attempt: r.attempt,
    person: { ...r.person, transferred: r.person.transferred >= 0 ? String(r.person.transferred) : '' },
  };
}
// What a forwarded exchange settled to: the binding's refusal kind, the
// provider's refusal code, or the receipt.
export function settle(status, json) {
  if (!json || typeof json.ok !== 'boolean') return { status, code: json?.error?.code || 'unanswered' };
  if (!json.ok) return { status, code: json.refusal.kind, refusal: json.refusal };
  if (!json.value.ok) return { status, code: json.value.code };
  return { status, code: '', reply: json.value, receipt: receiptView(json.value.receipt) };
}

// A loopback port for a head whose successor is free too: the head's api
// binding serves its HTTP transport one port past the reads (GH #1135),
// and a port it cannot hold stops the head at start.
export async function headPort() {
  const listen = port => new Promise(resolve => {
    const server = net.createServer();
    server.once('error', () => resolve(null));
    server.listen(port, '127.0.0.1', () => resolve(server));
  });
  const close = server => new Promise(resolve => server.close(resolve));
  for (let attempt = 0; attempt < 64; attempt += 1) {
    const first = await listen(0);
    if (!first) continue;
    const port = first.address().port;
    const next = port < 65535 ? await listen(port + 1) : null;
    await close(first);
    if (next) { await close(next); return port; }
  }
  throw new Error('no loopback port with a free successor for the head');
}
