// The head's command wire as the face meets it (GH #1104, #1135): one line
// of the api binding's wire POSTed to …/commands, forwarded to the head's
// socket, and the binding's receipt line back. Scripted lanes build their
// answers here so every lane scripts the one shape the head speaks.

// The call each operation is sent as.
export const CALLS = {
  'dna.practice.propose': 'PracticePropose', 'dna.review.verdict': 'ReviewVerdict',
  'dna.organization.propose': 'OrganizationPropose', 'dna.task.reassign': 'TaskReassign',
  'dna.person.retire': 'PersonRetire', 'dna.task.create': 'TaskCreate',
};
// The two commands every authenticated peer may send.
export const UNGATED = ['CommandLookup', 'TaskCreate'];

// A describe line asks for the caller's slice. It reads; lanes that watch
// for writes leave it out.
export function isDescribe(request) {
  if (request.method() !== 'POST' || !new URL(request.url()).pathname.endsWith('/commands')) return false;
  try { return request.postDataJSON()?.describe === true; } catch { return false; }
}
export const isWrite = request => request.method() !== 'GET' && !isDescribe(request);

// The head's own caller, as the binding reports a forwarded line.
export const CALLER = { mode: 'unix', name: 'uid:1000', uid: 1000, gid: 1000, pid: 4242, via: 'http-session' };
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
    principal_mode: '', principal_name: '', position_id: 'org', target_kind: 'dna.practice', target_id: '',
    subject_digest: '', fingerprint: '', state: '', reason: '', proposal_state: 'pending', verdict_value: '',
    verdict_state: '', candidate_digest: '', review_id: '', review_state: 'unavailable', review_outcome: '',
    review_subject_digest: '', activation_state: 'unknown', activation_reason: '', ...flat,
    organization: { proposal_state: 'pending', source_head: '', source_digest: '', mutation_id: '', candidate_commit: '', application_state: 'pending', application_reason_code: '', restart_handoff_state: 'pending', ...organization },
    task: { state: '', from: '', to: '', event_id: '', ...task },
    person: { state: '', from: '', to: '', event_id: '', transferred: -1, ...person },
    task_create: { intent_id: '', intent_state: '', task_id: '', event_id: '', ...task_create },
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
    'dna.task.create': () => ({ request_id, record_head: p.record_head, outcome: a.outcome, to: a.to }),
  };
  if (!Object.hasOwn(payloads, operation)) throw new Error('No call carries ' + operation);
  return { call: CALLS[operation], payload: payloads[operation]() };
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
