/* The face: the DNA organism's browser surface. Presentation only: the service owns state and authority.
 * Every Record value is rendered as text. Only scoped command recovery metadata
 * enters browser storage; draft content, rationale and receipts never do.
 */
"use strict";

(() => {
  const API = "/api/hale/v1/applications";
  const LIMIT = 25;
  const READ_TIMEOUT_MS = 15_000;
  // Resource-specific presentation stays behind this small workspace registry;
  // request lifetime, identity, paging and auth remain shared across all reads.
  const WORKSPACES = {
    practices: {
      title: "Practices", singular: "Practice", resource: "practices", capability: "practices",
      kicker: "ORGANIZATIONAL MEMORY", description: "The agreements that guide this application, and the decisions behind them.",
      register: "Practice register", detail: "Practice detail", back: "Back to practices",
      empty: "No practices yet", emptyDescription: "No canonical practice proposals are recorded in this snapshot.",
      selection: "A practice, in context", selectionDescription: "Select a practice to read its document, provenance, and governing Review.",
      fields: ["id", "digest", "name", "kind", "text", "text_status", "author", "target", "binding_class", "provenance", "supersedes", "request_id", "review_id", "review_state", "review_settled", "review_outcome", "state", "requester", "rationale"],
      booleans: ["text_available", "ratified", "retired", "declined"],
      rowName: (item) => display(item.name, item.id), badge: practiceBadge, rowMeta: practiceMeta, inspector: practiceDetail
    },
    organization: {
      title: "Organization", singular: "Organization node", resource: "organization", capability: "organization",
      kicker: "DECLARED ORGANIZATION", description: "Explore source-declared positions, their surrounding structure, and the contracts they expose.",
      register: "Organization outline", detail: "Organization node", back: "Back to organization",
      empty: "No declared instances", emptyDescription: "This source snapshot declares no static organization instances. Declarations alone do not establish occupied or vacant positions.",
      selection: "Structure, with its source", selectionDescription: "Select a declared instance to inspect its containment, contracts, and source. Viewing a position does not grant its authority.",
      fields: ["id", "declaration", "parent_id", "thread_domain", "role", "source_file"], booleans: ["in_position_outline", "sealed"],
      sourceBound: true, rowName: (item) => item.id, badge: organizationBadge, rowMeta: organizationMeta, inspector: organizationDetail
    },
    definitions: {
      title: "Definitions", singular: "Definition revision", resource: "definitions", capability: "definitions",
      kicker: "CODE-AUTHORED WORKFLOWS", description: "Explore recursive workflows, prepare an exact revision, and validate its complete catalog.",
      register: "Definition catalog", detail: "Definition revision", back: "Back to definitions",
      empty: "No definitions in this catalog", emptyDescription: "The host supplied an available, validated catalog with no workflow definitions.",
      selection: "A workflow, in source", selectionDescription: "Select an exact revision to inspect its Steps, leaf Work specifications, child definitions, and source provenance.",
      fields: ["id", "definition_id", "revision", "title"], booleans: [],
      sourceBound: true, rowName: (item) => display(item.title, item.definition_id), badge: definitionBadge, rowMeta: definitionMeta, inspector: definitionDetail
    },
    workflows: {
      title: "Work", singular: "Execution", resource: "workflows", capability: "workflows",
      kicker: "RECORDED EXECUTION", description: "Follow admitted work through ordered Steps, child tasks, attempts, and evidenced outcomes.",
      register: "Execution register", detail: "Execution", back: "Back to executions",
      empty: "No visible executions", emptyDescription: "No visible workflow admissions or task births are recorded in this snapshot.",
      selection: "Work, with its evidence", selectionDescription: "Select an execution to explore its bound recipe and recorded transitions.",
      fields: ["id", "engine", "definition_id", "revision", "state", "reason"], booleans: [],
      rowName: item => item.id, badge: item => workflowBadge(item.state), rowMeta: workflowMeta, inspector: workflowDetail
    },
    tasks: {
      title: "Work", singular: "Task", resource: "tasks", capability: "tasks",
      kicker: "HANDED RESPONSIBILITIES", description: "See who holds each obligation, follow its history, and make an authorized reassignment.",
      register: "Handed Tasks", detail: "Task responsibility", back: "Back to handed Tasks",
      empty: "No visible handed Tasks", emptyDescription: "No readable Task with a recorded handoff is available in this snapshot.",
      selection: "A responsibility, and who holds it", selectionDescription: "Select a Task to inspect its assignee, obligation, and assignment history.",
      fields: ["id", "outcome", "state", "assignee", "obligation", "acceptance_digest", "evidence_ref", "waiting", "assignment_digest"],
      booleans: ["acceptance_bound", "evidence_required", "reassignment_supported"],
      rowName: item => item.outcome || item.id, badge: item => workflowBadge(item.state), rowMeta: item => node("div", "record-meta", item.assignee || "No assignee recorded"), inspector: taskDetail
    },
    knowledge: {
      title: "Knowledge", singular: "Knowledge item", resource: "knowledge/nodes", capability: "knowledge",
      kicker: "THE KNOWLEDGE GRAPH", description: "Explore the ideas held by this application, their stored relationships, and where they are bound.",
      register: "Knowledge register", detail: "Knowledge item", back: "Back to knowledge",
      empty: "No knowledge items available", emptyDescription: "This snapshot has no visible knowledge items in the selected context.",
      selection: "An idea, and its connections", selectionDescription: "Select an item to read its content, inspect its direct relationships, and see where it is bound.",
      fields: ["id", "kind", "text", "author", "projection_state", "revision", "name", "supersedes", "ratified_seq"], booleans: ["accepted"],
      sourceBound: true, rowName: knowledgeName, badge: knowledgeBadge, rowMeta: knowledgeMeta, inspector: knowledgeDetail
    },
    reviews: {
      title: "Reviews", singular: "Review", resource: "reviews", capability: "reviews",
      kicker: "DECISIONS & AUTHORITY", description: "Inspect the exact subject, required authority, and recorded decision.",
      register: "Review register", detail: "Review detail", back: "Back to reviews",
      empty: "No reviews yet", emptyDescription: "No Reviews are recorded in this snapshot.",
      selection: "A decision, in context", selectionDescription: "Select a Review to inspect its exact subject, authority, and recorded outcome.",
      fields: ["id", "state", "subject_digest", "required_authority", "settled", "outcome", "knowledge_digest", "text_status", "question"],
      booleans: ["text_available"], rowName: (item) => item.id, badge: reviewBadge, rowMeta: reviewMeta, inspector: reviewDetail
    }
  };
  const APPLICATION_HOST = document.documentElement.dataset.faceProfile === "application";
  const VIEWS = new Set([...Object.keys(WORKSPACES), "application", "projects"]);
  const independentView = (view) => view === "application" || view === "projects";
  // The operator-machine head answers this path; a plain Record API does not.
  const HEAD_API = "/api/hale/v1/head";
  const OUTCOMES = { approve: "Approved", reject: "Rejected", revise: "Revision requested", abstain: "Abstained" };
  const PRACTICE_STATES = { pending: "Pending", ratified: "Ratified", declined: "Declined", retired: "Retired", refused: "Refused" };
  const STATUS_REASONS = {
    missing: ["Document unavailable", "The canonical receipt document is missing from this Record."],
    redacted: ["Content redacted", "The service has withheld this content because the receipt is redacted."],
    protected: ["Content protected", "The receipt's classification does not permit this read surface to return its content."],
    source_unavailable: ["Content visibility unavailable", "This read surface cannot establish current receipt visibility. After Ledger adoption, Record metadata remains available while receipt content is withheld, including after abandonment."],
    digest_mismatch: ["Document could not be verified", "The available document does not match its recorded digest. Its content is withheld."],
    invalid_document: ["Document could not be verified", "The receipt does not have a supported, valid canonical document. Its content is withheld."]
  };
  const $ = (id) => document.getElementById(id);
  const ui = Object.fromEntries(["application", "connection-caption", "principal", "sign-out", "breadcrumb-current", "workspace-kicker", "workspace-title", "workspace-description", "refresh", "notice", "source", "content", "workspace-footer", "announcement"].map((id) => [id, $(id)]));
  let generation = 0;
  let controller = null;
  let lastStarted = 0;
  let applicationController = null;
  // The head is probed once per page, and only for views that read the
  // Record or the Projects workspace.
  let headState = null;
  let headProbed = false;
  let headRedirect = false;
  let projectsController = null;
  let definitionDraftController = null;
  let definitionDraftHost = null;
  let organizationDraftController = null;
  let organizationDraftHost = null;
  let ownershipDraftController = null;
  let ownershipDraftHost = null;
  let knowledgeDraftController = null;
  let knowledgeDraftHost = null;
  let knowledgePreview = null;
  let knowledgeComparison = "compare";
  let knowledgeSelectedEdge = "";
  // Only paths traversed through captured member references appear here.
  // Shared child definitions never acquire an invented unique parent.
  let definitionJourney = null;
  let navigationFocus = null;
  let organizationPresentation = "topology";
  let topologyScale = 1;
  let state = blankState(readRoute());

  // Command bodies and receipts remain in memory. Only a scoped recovery key
  // crosses a reload, and persisting that key must succeed before any POST.
  const COMMAND_PROFILE = "dna.practice.propose.v1";
  const PRACTICE_OPERATION = "dna.practice.propose";
  const REVIEW_OPERATION = "dna.review.verdict";
  const REVIEW_PROFILE = "dna.review.verdict.v1";
  const SOURCE_REVIEW_PROFILE = "dna.organization.review.verdict.v1";
  const ORGANIZATION_OPERATION = "dna.organization.propose";
  const TASK_OPERATION = "dna.task.reassign";
  const PERSON_OPERATION = "dna.person.retire";
  const TASK_CREATE_OPERATION = "dna.task.create";
  const ORGANIZATION_MODULE = "dna/org/main.hl";
  const ORGANIZATION_BASE = ["source_head", "module_digest", "dependency_source", "dependency_digest", "record_head"];
  const VERDICTS = { approve: "Approve", reject: "Reject", revise: "Request revision" };
  const COMMAND_STORAGE = "face.practice-recovery.v1:";
  const COMMAND_TERMINAL = new Set(["succeeded", "refused", "failed"]);
  let intervention = { scope: "", phase: "idle", draft: null, metadata: null, receipt: null, error: "", blocked: false };
  let commandGeneration = 0;
  let commandController = null;
  const KNOWLEDGE_OPERATION = "dna.knowledge.edge.link";
  const KNOWLEDGE_UNLINK = "dna.knowledge.edge.unlink";
  const KNOWLEDGE_NODES = ["dna.knowledge.node.propose", "dna.knowledge.node.revise", "dna.knowledge.node.retire"];
  const KNOWLEDGE_BINDINGS = ["dna.knowledge.binding.bind", "dna.knowledge.binding.unbind"];
  const KNOWLEDGE_REMOVAL_PAGES = 32;
  const KNOWLEDGE_STORAGE = "face.knowledge-recovery.v1:";
  let knowledgeCommand = { scope: "", metadata: null, result: null, capability: null, phase: "idle", error: "", projection: "unread", blocked: false };
  let knowledgeCommandGeneration = 0;
  let knowledgeCommandController = null;

  function closedObject(value, keys) {
    return value !== null && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key));
  }
  function unicodeText(value) {
    if (typeof value !== "string" || value.includes("\u0000")) return false;
    for (let i = 0; i < value.length; i += 1) {
      const c = value.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff) {
        const next = value.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      } else if (c >= 0xdc00 && c <= 0xdfff) return false;
    }
    return true;
  }
  function byteLength(value) { return new TextEncoder().encode(value).length; }
  function commandID(value, maximum = 256) {
    return unicodeText(value) && value.length > 0 && !/[\u0000-\u001f\u007f]/.test(value) && byteLength(value) <= maximum;
  }
  function sourceDigest(value) { return typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value); }
  function sourceCommit(value) { return typeof value === "string" && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value); }
  function organizationBase(base) {
    return closedObject(base, ORGANIZATION_BASE) && sourceCommit(base.source_head) && sourceDigest(base.module_digest) && sourceDigest(base.dependency_digest) && ["none", "committed_source", "local_vendor_snapshot"].includes(base.dependency_source) && commandID(base.record_head);
  }
  function commandProfile(capabilities, operation) {
    if (operation === TASK_CREATE_OPERATION) {
      const c = capabilities?.task_create_commands;
      const supported = closedObject(c, ["profile", "available", "authorized", "position_id", "recovery", "max_outcome_bytes", "max_identity_bytes", "max_request_bytes", "reason"]) && c.profile === "dna.task.create.v1" && typeof c.available === "boolean" && typeof c.authorized === "boolean" && c.position_id === "org" && c.recovery === "record_lifetime" && c.max_outcome_bytes === "8192" && c.max_identity_bytes === "256" && c.max_request_bytes === "32768" && unicodeText(c.reason) && byteLength(c.reason) <= 512;
      return { supported, enabled: supported && c.available && c.authorized };
    }
    if (operation === TASK_OPERATION || operation === PERSON_OPERATION) {
      const person = operation === PERSON_OPERATION;
      const c = person ? capabilities?.person_commands : capabilities?.task_commands;
      const supported = closedObject(c, ["profile", "available", "authorized", "position_id", "recovery", "max_identity_bytes", "max_request_bytes", "recipients", "reason", ...(person ? ["max_transfers"] : [])]) && c.profile === (person ? "dna.person.retire.v1" : "dna.task.reassign.v1") && (!person || c.max_transfers === "32") && typeof c.available === "boolean" && typeof c.authorized === "boolean" && c.position_id === "org" && c.recovery === "record_lifetime" && c.max_identity_bytes === "256" && c.max_request_bytes === "32768" && Array.isArray(c.recipients) && c.recipients.length <= 64 && c.recipients.every(value => commandID(value)) && new Set(c.recipients).size === c.recipients.length && unicodeText(c.reason) && byteLength(c.reason) <= 512 && (c.available && c.authorized || c.recipients.length === 0);
      return { supported, enabled: supported && c.available && c.authorized };
    }
    const review = operation === REVIEW_OPERATION;
    const c = review ? capabilities?.review_commands : capabilities?.commands;
    const limits = review ? ["max_comment_bytes"] : ["max_text_bytes", "max_rationale_bytes"];
    const supported = [PRACTICE_OPERATION, REVIEW_OPERATION].includes(operation) && closedObject(c, ["profile", "available", "authorized", "position_id", "recovery", ...limits, "reason"]) && c.profile === (review ? REVIEW_PROFILE : COMMAND_PROFILE) && typeof c.available === "boolean" && typeof c.authorized === "boolean" && c.position_id === "org" && c.recovery === "record_lifetime" && (review ? c.max_comment_bytes === "2048" : c.max_text_bytes === "8192" && c.max_rationale_bytes === "2048") && unicodeText(c.reason);
    return { supported, enabled: supported && c.available && c.authorized };
  }
  function sourceCommandProfile(capabilities, review) {
    const c = review ? capabilities?.organization_review_commands : capabilities?.organization_commands;
    const fields = review ? ["operation", "max_comment_bytes"] : ["module_path", "max_source_bytes", "max_rationale_bytes", "max_request_bytes"];
    const supported = closedObject(c, ["profile", "available", "authorized", "position_id", "recovery", "command_origin", ...fields, "reason"]) && typeof c.available === "boolean" && typeof c.authorized === "boolean" && c.position_id === "org" && c.recovery === "record_lifetime" && ["", location.origin].includes(c.command_origin) && unicodeText(c.reason) && (review ? c.profile === SOURCE_REVIEW_PROFILE && c.operation === REVIEW_OPERATION && c.max_comment_bytes === "2048" : c.profile === "dna.organization.propose.v1" && c.module_path === "dna/org/main.hl" && c.max_source_bytes === "16384" && c.max_rationale_bytes === "2048" && c.max_request_bytes === "32768");
    return { supported, enabled: supported && c.available && c.authorized && c.command_origin === location.origin };
  }
  function commandCapability(capabilities = state.capabilities, operation = PRACTICE_OPERATION, sourceReview = false) {
    const organization = operation === ORGANIZATION_OPERATION;
    const profile = organization || sourceReview && operation === REVIEW_OPERATION ? sourceCommandProfile(capabilities, sourceReview) : commandProfile(capabilities, operation);
    const writes = capabilities?.writes;
    const consistent = commandID(capabilities?.principal?.name) && ["local", "oidc"].includes(capabilities.principal.mode) && commandID(capabilities.application_id) && typeof writes?.practice_propose === "boolean" && typeof writes.review_verdict === "boolean" && typeof capabilities.read_only === "boolean" && (!(writes.practice_propose || writes.review_verdict) || capabilities.read_only === false) && (!writes.practice_propose || commandProfile(capabilities, PRACTICE_OPERATION).enabled) && (!writes.review_verdict || commandProfile(capabilities, REVIEW_OPERATION).enabled);
    const sourceConsistent = !(sourceReview || organization) || typeof writes?.organization_propose === "boolean" && typeof writes.organization_review_verdict === "boolean" && (!(writes.organization_propose || writes.organization_review_verdict) || capabilities.read_only === false) && (!writes.organization_propose || sourceCommandProfile(capabilities, false).enabled) && (!writes.organization_review_verdict || sourceCommandProfile(capabilities, true).enabled);
    const taskConsistent = operation !== TASK_OPERATION || typeof writes?.task_reassign === "boolean" && (!writes.task_reassign || capabilities.read_only === false && commandProfile(capabilities, TASK_OPERATION).enabled);
    const personConsistent = operation !== PERSON_OPERATION || typeof writes?.person_retire === "boolean" && (!writes.person_retire || capabilities.read_only === false && commandProfile(capabilities, PERSON_OPERATION).enabled);
    const createConsistent = operation !== TASK_CREATE_OPERATION || typeof writes?.task_create === "boolean" && (!writes.task_create || capabilities.read_only === false && commandProfile(capabilities, TASK_CREATE_OPERATION).enabled);
    const supported = profile.supported && consistent && sourceConsistent && taskConsistent && personConsistent && createConsistent;
    return { supported, allowed: state.phase === "ready" && supported && profile.enabled && writes[operation === TASK_CREATE_OPERATION ? "task_create" : operation === PERSON_OPERATION ? "person_retire" : operation === TASK_OPERATION ? "task_reassign" : organization ? "organization_propose" : sourceReview ? "organization_review_verdict" : operation === REVIEW_OPERATION ? "review_verdict" : "practice_propose"] === true };
  }
  function reviewCapability(review) {
    return commandCapability(state.capabilities, REVIEW_OPERATION, review?.organization_source === true);
  }
  function recoveryCapability() {
    return commandCapability(state.capabilities, intervention.metadata?.operation || PRACTICE_OPERATION, intervention.metadata?.source_review === true);
  }
  function commandScope() {
    const principal = state.capabilities?.principal;
    return ["ready", "domain-error"].includes(state.phase) && state.app && principal ? JSON.stringify([state.app.id, principal.mode, principal.name]) : "";
  }
  function recoveryKey(scope) { return COMMAND_STORAGE + encodeURIComponent(scope); }
  function clearIntervention() {
    commandGeneration += 1;
    if (commandController) commandController.abort();
    commandController = null;
    intervention = { scope: "", phase: "idle", draft: null, metadata: null, receipt: null, error: "", blocked: false };
  }
  function validRecovery(metadata) {
    const principal = state.capabilities?.principal;
    const common = ["version", "application_id", "principal", "request_id", "target_id", "subject_digest"];
    const legacy = closedObject(metadata, common) && metadata.version === 1 && metadata.subject_digest === metadata.target_id;
    const current = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind"]) && metadata.version === 2 && metadata.operation_version === "1" && metadata.position_id === "org" && (metadata.operation === PRACTICE_OPERATION ? metadata.target_kind === "dna.practice" && metadata.subject_digest === metadata.target_id : metadata.operation === REVIEW_OPERATION && metadata.target_kind === "dna.review");
    const source = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind", "source_review"]) && metadata.version === 3 && metadata.source_review === true && metadata.operation === REVIEW_OPERATION && metadata.operation_version === "1" && metadata.position_id === "org" && metadata.target_kind === "dna.review";
    const organization = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind", "base", "source_digest"]) && metadata.version === 4 && metadata.operation === ORGANIZATION_OPERATION && metadata.operation_version === "1" && metadata.position_id === "org" && metadata.target_kind === "dna.organization.module" && metadata.target_id === ORGANIZATION_MODULE && organizationBase(metadata.base) && metadata.base.module_digest === metadata.subject_digest && sourceDigest(metadata.source_digest);
    const task = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind"]) && metadata.version === 5 && metadata.operation === TASK_OPERATION && metadata.operation_version === "1" && metadata.position_id === "org" && metadata.target_kind === "dna.task" && sourceDigest(metadata.subject_digest);
    const person = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind"]) && metadata.version === 6 && metadata.operation === PERSON_OPERATION && metadata.operation_version === "1" && metadata.position_id === "org" && metadata.target_kind === "dna.person" && sourceDigest(metadata.subject_digest);
    // A raised task targets the Record; its subject is the head it was prepared against.
    const create = closedObject(metadata, [...common, "operation", "operation_version", "position_id", "target_kind"]) && metadata.version === 7 && metadata.operation === TASK_CREATE_OPERATION && metadata.operation_version === "1" && metadata.position_id === "org" && metadata.target_kind === "dna.record" && metadata.target_id === metadata.application_id;
    return (legacy || current || source || organization || task || person || create) && metadata.application_id === state.app?.id && closedObject(metadata.principal, ["mode", "name"]) && metadata.principal.mode === principal?.mode && metadata.principal.name === principal?.name && commandID(metadata.request_id, 128) && commandID(metadata.target_id) && commandID(metadata.subject_digest);
  }
  function restoreIntervention() {
    intervention.scope = commandScope();
    if (!intervention.scope) return;
    try {
      const raw = localStorage.getItem(recoveryKey(intervention.scope));
      if (raw === null) return;
      const metadata = JSON.parse(raw);
      if (!validRecovery(metadata)) throw new Error("invalid recovery metadata");
      // Legacy storage is never rewritten during recovery. Normalize only this
      // in-memory view, preserving the same slot and lock across operations.
      intervention.metadata = metadata.version === 1 ? { ...metadata, version: 2, operation: PRACTICE_OPERATION, operation_version: "1", position_id: "org", target_kind: "dna.practice" } : metadata;
      intervention.phase = "uncertain";
      intervention.error = "A saved request may already have reached the service. Check its status using the same request identity; it will not be submitted again.";
    } catch {
      intervention.blocked = true;
      intervention.error = "Recovery storage is unavailable or its saved identity cannot be verified. Submission is blocked so an unresolved request cannot be replaced.";
    }
  }
  function eligiblePractice(p) {
    return p && practiceTextAvailable(p) && p.kind === "practice" && p.state === "ratified" && p.ratified === true && p.retired === false && p.declined === false && p.author === "org" && p.target === "org" && commandID(p.id) && p.id === p.digest;
  }
  function interventionReason(p) {
    if (!commandCapability().supported) return "This connection does not provide the supported practice command profile. Practice reads remain available.";
    if (!commandCapability().allowed) return "Practice submission is unavailable for this connection or signed-in principal. Viewing an organization position does not grant authority.";
    if (intervention.blocked) return intervention.error;
    if (intervention.metadata) return "Check the saved request above before starting another proposal.";
    if (!eligiblePractice(p)) return "Revision proposals require a readable, ratified, current organization-wide practice. The service rechecks eligibility and authority before accepting a request.";
    return "Propose an organization-wide replacement. The service checks your current authority; viewing a position does not grant it.";
  }
  function validDraft(draft) {
    if (draft?.operation === TASK_CREATE_OPERATION) return commandID(draft.record_head) && draft.subject === draft.record_head && commandID(draft.target) && unicodeText(draft.outcome) && byteLength(draft.outcome) > 0 && byteLength(draft.outcome) <= 8192 && commandID(draft.to);
    if (draft?.operation === PERSON_OPERATION) return sourceDigest(draft.subject) && commandID(draft.target) && (draft.to === "" || commandID(draft.to)) && draft.target !== draft.to;
    if (draft?.operation === TASK_OPERATION) return sourceDigest(draft.subject) && commandID(draft.target) && commandID(draft.from) && commandID(draft.to) && draft.from !== draft.to;
    if (draft?.operation === REVIEW_OPERATION) return typeof draft.verdict === "string" && Object.hasOwn(VERDICTS, draft.verdict) && unicodeText(draft.comment) && byteLength(draft.comment) <= 2048;
    if (draft?.operation === ORGANIZATION_OPERATION) return organizationBase(draft.base) && draft.subject === draft.base.module_digest && draft.target === ORGANIZATION_MODULE && sourceDigest(draft.source_digest) && draft.source_digest !== draft.subject && unicodeText(draft.source_text) && byteLength(draft.source_text) > 0 && byteLength(draft.source_text) <= 16384 && unicodeText(draft.rationale) && byteLength(draft.rationale) > 0 && byteLength(draft.rationale) <= 2048;
    return draft?.operation === PRACTICE_OPERATION && unicodeText(draft.text) && byteLength(draft.text) > 0 && byteLength(draft.text) <= 8192 && unicodeText(draft.rationale) && byteLength(draft.rationale) > 0 && byteLength(draft.rationale) <= 2048 && draft.text !== draft.predecessor;
  }
  function beginIntervention(p) {
    if (!commandCapability().allowed || !eligiblePractice(p) || intervention.metadata || intervention.blocked) return;
    intervention.draft = { operation: PRACTICE_OPERATION, subject: p.digest, target: p.id, name: p.name, predecessor: p.text, text: p.text, rationale: "" };
    intervention.phase = "editing";
    intervention.error = "";
    render();
    $("proposal-text")?.focus();
  }
  // Presentation-only diff. Bound the alignment work independently of document
  // size; an oversized middle is shown as one replacement, never truncated.
  function practiceChanges(before, after) {
    const tokens = (text) => text.match(/\s+|[\p{L}\p{N}_]+|[^\s]/gu) || [];
    const a = tokens(before), b = tokens(after);
    let start = 0, end = 0;
    while (start < a.length && start < b.length && a[start] === b[start]) start++;
    while (end < a.length - start && end < b.length - start && a[a.length - end - 1] === b[b.length - end - 1]) end++;
    const left = a.slice(start, a.length - end), right = b.slice(start, b.length - end);
    const chunks = [];
    const push = (kind, text) => {
      if (!text) return;
      if (chunks.at(-1)?.kind === kind) chunks.at(-1).text += text;
      else chunks.push({ kind, text });
    };
    push("same", a.slice(0, start).join(""));
    const coarse = (left.length + 1) * (right.length + 1) > 250000;
    if (coarse) {
      push("removed", left.join("")); push("added", right.join(""));
    } else {
      const width = right.length + 1;
      const lengths = new Uint32Array((left.length + 1) * width);
      for (let i = left.length - 1; i >= 0; i--) for (let j = right.length - 1; j >= 0; j--) {
        lengths[i * width + j] = left[i] === right[j] ? 1 + lengths[(i + 1) * width + j + 1] : Math.max(lengths[(i + 1) * width + j], lengths[i * width + j + 1]);
      }
      let i = 0, j = 0;
      while (i < left.length || j < right.length) {
        if (i < left.length && j < right.length && left[i] === right[j]) { push("same", left[i++]); j++; }
        else if (i < left.length && (j === right.length || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1])) push("removed", left[i++]);
        else push("added", right[j++]);
      }
    }
    push("same", end ? a.slice(a.length - end).join("") : "");
    return { chunks, coarse };
  }
  function practiceComparison(draft) {
    const comparison = node("div", "practice-comparison");
    comparison.id = "proposal-comparison";
    comparison.tabIndex = -1;
    comparison.setAttribute("role", "group");
    comparison.setAttribute("aria-label", "Exact predecessor and proposed replacement");
    const { chunks, coarse } = practiceChanges(draft.predecessor, draft.text);
    const count = (kind) => chunks.filter((chunk) => chunk.kind === kind).reduce((n, chunk) => n + Array.from(chunk.text).length, 0);
    const summary = node("p", "comparison-summary", count("added") + " added characters · " + count("removed") + " removed characters" + (coarse ? " · Large changes grouped" : ""));
    const columns = node("div", "intervention-comparison");
    const before = node("div", "document-text comparison-before"), after = node("div", "document-text comparison-after");
    const document = (title, content, identity, className) => append(node("section", "intervention-document " + className), node("h4", "", title), content, node("p", "mono", identity));
    append(columns, document("Current ratified text", before, "Predecessor · " + draft.subject, "comparison-current"), document("Proposed replacement", after, "Draft · candidate identity assigned after submission", "comparison-proposed"));
    const paint = (highlight) => {
      changes.setAttribute("aria-pressed", String(highlight));
      exact.setAttribute("aria-pressed", String(!highlight));
      before.replaceChildren(); after.replaceChildren();
      for (const chunk of chunks) {
        if (chunk.kind !== "added") before.append(highlight && chunk.kind === "removed" ? node("del", "", chunk.text) : documentTextNode(chunk.text));
        if (chunk.kind !== "removed") after.append(highlight && chunk.kind === "added" ? node("ins", "", chunk.text) : documentTextNode(chunk.text));
      }
    };
    const changes = button("Show changes", () => paint(true), "scope-button");
    const exact = button("Read exact text", () => paint(false), "scope-button");
    const controls = append(node("div", "comparison-controls"), summary, append(node("div", "comparison-switch"), changes, exact));
    paint(true);
    return append(comparison, controls, columns);
  }
  function documentTextNode(text) { return document.createTextNode(text); }
  function renderPracticeIntervention(p) {
    const panel = node("section", "practice-intervention");
    panel.setAttribute("aria-label", "Practice intervention");
    panel.setAttribute("role", "region");
    const heading = append(node("div", "intervention-heading"), append(node("div"), node("h3", "", "Propose a revision"), node("p", "", interventionReason(p))));
    const body = node("div", "intervention-body");
    const draft = intervention.draft;
    if (draft?.operation !== PRACTICE_OPERATION || draft.subject !== p.digest || !["editing", "reviewing"].includes(intervention.phase)) {
      const start = button("Propose revision", () => beginIntervention(p));
      start.disabled = !commandCapability().allowed || !eligiblePractice(p) || Boolean(intervention.metadata) || intervention.blocked;
      body.append(start);
      return append(panel, heading, body);
    }
    body.append(node("p", "intervention-scope", "Scope · org / Organization-wide. Acting principal · " + state.capabilities.principal.name));
    if (intervention.phase === "editing") {
      const form = node("form");
      const field = (labelText, id, property, maximum, className = "") => {
        const label = node("label", "intervention-field");
        const input = node("textarea", className);
        input.id = id;
        input.name = property;
        input.setAttribute("aria-label", labelText);
        input.required = true;
        input.autocomplete = "off";
        input.spellcheck = true;
        input.value = draft[property];
        let presented = input.value;
        const count = node("p", "intervention-counter");
        count.id = id + "-count";
        input.setAttribute("aria-describedby", count.id + " proposal-draft-hint");
        const update = () => {
          // Browsers normalize CRLF in textareas. Merely opening a draft must
          // not change its captured bytes or manufacture a proposed revision.
          if (input.value !== presented) { draft[property] = input.value; presented = input.value; }
          const length = byteLength(draft[property]);
          count.textContent = length + " / " + maximum + " UTF-8 bytes";
          input.setAttribute("aria-invalid", String(!unicodeText(input.value) || length > maximum));
        };
        input.addEventListener("input", update);
        update();
        return append(label, node("span", "", labelText), input, count);
      };
      const hint = node("p", "field-hint", "Keep the text exact. Draft text and rationale stay in this page only and are cleared when the view reloads or changes. No proposal is sent until you review and submit it.");
      hint.id = "proposal-draft-hint";
      const error = node("p", "intervention-error", intervention.error);
      error.setAttribute("role", "alert");
      error.hidden = !intervention.error;
      const review = node("button", "button", "Review proposal");
      review.type = "submit";
      const cancelDraft = button("Discard draft", () => { intervention.draft = null; intervention.phase = "idle"; intervention.error = ""; render(); focusPanel("detail"); });
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        if (!validDraft(draft)) {
          intervention.error = draft.text === draft.predecessor ? "Change the proposed text before reviewing a revision." : "Enter nonempty valid text within 8192 UTF-8 bytes and a rationale within 2048 UTF-8 bytes. Nothing has been submitted.";
          error.textContent = intervention.error;
          error.hidden = false;
          return;
        }
        intervention.phase = "reviewing";
        intervention.error = "";
        render();
        $("proposal-comparison")?.focus();
      });
      append(form, field("Proposed text", "proposal-text", "text", 8192), field("Rationale", "proposal-rationale", "rationale", 2048, "intervention-rationale"), hint, error, append(node("div", "intervention-actions"), review, cancelDraft));
      body.append(form);
    } else {
      const comparison = practiceComparison(draft);
      append(body, node("p", "field-hint", "Review both exact texts before submitting. This requests a proposal; it does not approve its Review or adopt the replacement."), comparison, section("Rationale", documentText(draft.rationale)));
      const submit = button("Submit proposal", () => submitIntervention(), "button");
      submit.disabled = Boolean(intervention.reserving);
      const edit = button("Back to editing", () => { intervention.phase = "editing"; intervention.error = ""; render(); $("proposal-text")?.focus(); });
      edit.disabled = Boolean(intervention.reserving);
      const actions = append(node("div", "intervention-actions"), submit, edit);
      body.append(actions);
      if (intervention.error) {
        const error = node("p", "intervention-error", intervention.error);
        error.setAttribute("role", "alert");
        body.append(error);
      }
    }
    return append(panel, heading, body);
  }
  function pendingPracticeReview(r) {
    return r && r.is_mutation === false && r.approvers === "" && r.required_authority === "board" && r.state === "pending" && r.text_available === true && r.text_status === "available" && r.settled === "" && r.outcome === "" && commandID(r.id) && commandID(r.subject_digest) && (r.subject_digest === r.knowledge_digest && !r.knowledge_binding_digest && !r.knowledge_edge_digest || r.subject_digest === r.knowledge_binding_digest && r.knowledge_digest === "" && !r.knowledge_edge_digest || r.subject_digest === r.knowledge_edge_digest && r.knowledge_digest === "" && !r.knowledge_binding_digest);
  }
  function eligibleReview(r, candidate = state.reviewCandidate) {
    if (r?.organization_source === true) {
      const c = candidate?.organization, principal = state.capabilities?.principal?.name;
      const owners = typeof r.approvers === "string" ? r.approvers.split(" ").filter(Boolean) : [];
      const belongs = r.approvers === "" || owners.some(entry => entry.indexOf("=") > 0 && entry.slice(entry.indexOf("=") + 1).split(",").includes(principal));
      return r.is_mutation === true && r.state === "pending" && r.text_available === true && r.text_status === "available" && r.settled === "" && r.outcome === "" && commandID(r.author) && r.author !== principal && belongs && candidate?.kind === "organization_source_change" && candidate.review_id === r.id && candidate.candidate_commit === r.subject_digest && c?.application_id === state.app?.id && c.mutation_id === r.id && c.command_id === r.organization_source_request_id && c.module.digest === r.organization_source_digest && c.review.quorum.state === "available";
    }
    if (pendingPracticeReview(r) && r.knowledge_edge_digest) return candidate?.kind === "knowledge_edge_change" && candidate.review_id === r.id && candidate.candidate_digest === r.subject_digest && commandID(candidate.relationship?.application_id) && r.author !== state.capabilities?.principal?.name;
    if (pendingPracticeReview(r) && r.knowledge_binding_digest) return candidate?.kind === "knowledge_binding_change" && candidate.review_id === r.id && candidate.candidate_digest === r.subject_digest && commandID(candidate.binding?.application_id) && r.author !== state.capabilities?.principal?.name;
    return pendingPracticeReview(r) && candidate && practiceTextAvailable(candidate) && commandID(candidate.kind) && candidate.state === "pending" && candidate.ratified === false && candidate.retired === false && candidate.declined === false && commandID(candidate.author, 1024) && commandID(candidate.target, 1024) && candidate.id === r.subject_digest && candidate.digest === r.subject_digest && candidate.review_id === r.id && candidate.review_state === "pending" && candidate.review_outcome === "" && candidate.review_settled === "";
  }
  function validBindingCandidate(data, review, appId) {
    assert(closedObject(data, ["kind", "review_id", "candidate_digest", "document"]) && data.kind === "knowledge_binding_change" && data.review_id === review.id && data.candidate_digest === review.subject_digest && unicodeText(data.document) && byteLength(data.document) <= 32768);
    const b = JSON.parse(data.document);
    const keys = ["format", "application_id", "request_id", "command_fingerprint", "authority", "authority_basis", "operation", "idea_id", "binding_id", "author", "target", "class", "by", "because", "binding_basis"];
    assert(closedObject(b, keys) && keys.every(key => unicodeText(b[key])) && b.format === "dna.knowledge-binding-change/1" && b.application_id === appId && knowledgeBindingOperation(b.operation) && receiptID(b.idea_id) && receiptID(b.binding_id) && commandID(b.request_id) && commandID(b.command_fingerprint));
    assert([b.author, b.target, b.by, b.authority].every(value => commandID(value, 256)) && commandID(b.authority_basis, 2048) && byteLength(b.because) <= 2048);
    assert(["initiative", "goal", "concern"].includes(b.class) && review.author === b.by && review.target === b.target && review.binding_class === b.class, "The binding candidate's proposer and applicability do not match this exact Review.");
    return { ...data, text: data.document, binding: b };
  }
  function validRelationshipCandidate(data, appId) {
    assert(closedObject(data, ["kind", "review_id", "candidate_digest", "document"]) && data.kind === "knowledge_edge_change" && commandID(data.review_id) && receiptID(data.candidate_digest) && unicodeText(data.document) && byteLength(data.document) <= 32768);
    const e = JSON.parse(data.document), keys = ["format", "application_id", "request_id", "command_fingerprint", "authority", "authority_basis", "operation", "edge_id", "from_id", "to_id", "rel", "by", "because", "edge_basis"];
    assert(closedObject(e, keys) && keys.every(key => unicodeText(e[key])) && e.format === "dna.knowledge-edge-change/1" && e.application_id === appId && [KNOWLEDGE_OPERATION, KNOWLEDGE_UNLINK].includes(e.operation));
    assert([e.edge_id, e.from_id, e.to_id].every(receiptID) && commandID(e.request_id) && commandID(e.command_fingerprint) && commandID(e.authority, 256) && commandID(e.authority_basis, 2048) && commandID(e.by, 256) && e.rel.length > 0 && byteLength(e.rel) <= 256 && byteLength(e.because) <= 2048);
    return { ...data, text: data.document, relationship: e };
  }
  function reviewInterventionReason(r) {
    if (!reviewCapability(r).supported) return r?.organization_source ? "This connection does not provide the separate Organization decision profile. The exact source comparison remains available." : "This connection does not provide the supported Review command profile. Review reads remain available.";
    if (!reviewCapability(r).allowed) return "Decision submission is unavailable for this connection or signed-in principal. Required authority is information, not a permission grant.";
    if (intervention.blocked) return intervention.error;
    if (intervention.metadata) return "Check the saved request above and explicitly dismiss it after completion before preparing another decision.";
    if (state.reviewCandidateError) return state.reviewCandidateError;
    if (r?.organization_source && r.author === state.capabilities?.principal?.name) return "Another authorized person must decide this Organization proposal. You are its recorded proposer.";
    if (r?.organization_source && !eligibleReview(r)) return "An Organization decision requires the exact readable candidate, a pending Review and available owner quorum. The service rechecks authority and affected-owner membership before recording it.";
    if (r?.knowledge_edge_digest && r.author === state.capabilities?.principal?.name) return "Another authorized person must decide this relationship proposal. You are its recorded proposer.";
    if (r?.knowledge_binding_digest && r.author === state.capabilities?.principal?.name) return "Another authorized person must decide this binding proposal. You are its recorded proposer.";
    if (!eligibleReview(r)) return "Decisions here require a readable canonical Knowledge, binding or relationship candidate and its exact pending Review, explicitly identified as non-mutation and without owner approvers. Missing eligibility facts, settled, unlinked and protected Reviews cannot use this action.";
    return "Decide on this exact pending candidate. The service rechecks current authority, independence, subject and pending state before recording a verdict.";
  }
  function currentDraftEligible(draft) {
    if (!draft || !commandCapability(state.capabilities, draft.operation, draft.source_review === true).allowed) return false;
    if (draft.operation === TASK_CREATE_OPERATION) return state.route.view === "tasks" && state.source?.record_head === draft.record_head && state.app?.id === draft.target;
    if (draft.operation === ORGANIZATION_OPERATION) return state.route.view === "organization" && state.source?.record_head === draft.base.record_head && ["source_head", "dependency_source", "dependency_digest"].every(key => state.collection?.basis?.[key] === draft.base[key]) && organizationDraftController?.publicationMatches(draft.validation) === true;
    if (draft.operation === PERSON_OPERATION) return state.route.view === "tasks" && state.route.assignee === draft.target && state.person?.person === draft.target && state.person.subject_digest === draft.subject && state.person.state === "active" && state.person.authorized && (draft.to === "" ? state.person.tasks.length === 0 : state.person.recipients.includes(draft.to));
    if (draft.operation === TASK_OPERATION) return state.route.view === "tasks" && state.detail?.id === draft.target && state.detail.assignment_digest === draft.subject && state.detail.assignee === draft.from && state.detail.state === "handed" && state.detail.reassignment_supported === true && state.capabilities.task_commands.recipients.includes(draft.to);
    return draft.operation === REVIEW_OPERATION ? state.route.view === "reviews" && eligibleReview(state.detail) && Boolean(state.detail.organization_source) === Boolean(draft.source_review) && state.detail.id === draft.target && state.detail.subject_digest === draft.subject && state.reviewCandidate.text === draft.candidate : state.route.view === "practices" && eligiblePractice(state.detail) && state.detail.id === draft.target && state.detail.digest === draft.subject;
  }
  function organizationProposalAccess() {
    const capability = commandCapability(state.capabilities, ORGANIZATION_OPERATION);
    const reason = !capability.supported ? "This connection does not provide the Organization publishing profile. Validation and export remain available." : !capability.allowed ? "Organization publishing is unavailable for this connection or signed-in principal." : intervention.blocked ? intervention.error : intervention.metadata ? "Recover or dismiss the saved request before starting another proposal." : "Submit the exact checked source for native verification and Review.";
    return { ...capability, allowed: capability.allowed && !intervention.blocked && !intervention.metadata, reason };
  }
  async function proposeOrganization(data, rationale) {
    if (!organizationProposalAccess().allowed || !organizationDraftController?.publicationMatches(data)) return { error: "The checked source or publishing permission is no longer available. Nothing was submitted." };
    const draft = { operation: ORGANIZATION_OPERATION, target: ORGANIZATION_MODULE, subject: data.base.module_digest, base: { ...data.base }, source_text: data.module.text, source_digest: data.module.digest, rationale, validation: data };
    intervention.draft = draft; intervention.phase = "reviewing"; intervention.error = "";
    await submitIntervention();
    const submitted = Boolean(intervention.metadata), error = intervention.error;
    if (intervention.draft === draft && !submitted) { intervention.draft = null; intervention.phase = "idle"; }
    return { submitted, error: submitted ? "" : error || "The source or signed-in context changed before the request could be reserved. Nothing was submitted." };
  }
  function beginReviewIntervention(r) {
    if (!reviewCapability(r).allowed || !eligibleReview(r) || intervention.metadata || intervention.blocked) return;
    intervention.draft = { operation: REVIEW_OPERATION, subject: r.subject_digest, target: r.id, candidate: state.reviewCandidate.text, verdict: "", comment: "", ...(r.organization_source ? { source_review: true } : {}) };
    intervention.phase = "editing";
    intervention.error = "";
    render();
    $("decision-approve")?.focus();
  }
  function renderReviewIntervention(r) {
    const panel = node("section", "practice-intervention review-intervention");
    panel.setAttribute("role", "region");
    panel.setAttribute("aria-label", "Review intervention");
    const heading = append(node("div", "intervention-heading"), append(node("div"), node("h3", "", r.organization_source ? "Review the Organization change" : "Decide on the exact candidate"), node("p", "", reviewInterventionReason(r))));
    const body = node("div", "intervention-body");
    if (r.organization_source && state.reviewCandidate?.organization) body.append(window.FaceOrganizationReview.render(state.reviewCandidate));
    if (!r.organization_source && (eligibleReview(r) || (state.reviewCandidate?.binding || state.reviewCandidate?.relationship) && state.reviewCandidate.review_id === r.id && state.reviewCandidate.candidate_digest === r.subject_digest)) {
      const binding = state.reviewCandidate.binding, relationship = state.reviewCandidate.relationship;
      const candidate = node("section", "intervention-document");
      if (binding) {
        candidate.append(node("h4", "", binding.operation === "dna.knowledge.binding.unbind" ? "Remove this exact locus binding" : "Add this exact locus binding"));
        const tuple = node("dl", "fact-grid"); fact(tuple, "Knowledge item", binding.idea_id, true, true); fact(tuple, "Authoring locus", binding.author); fact(tuple, "Target locus", binding.target); fact(tuple, "Service-derived class", binding.class); fact(tuple, "Actual proposer", binding.by); candidate.append(tuple, documentText(binding.because));
        const canonical = documentText(state.reviewCandidate.document); canonical.classList.add("binding-canonical-document"); candidate.append(append(node("details"), node("summary", "", "Exact canonical binding document"), canonical));
      } else if (relationship) {
        candidate.append(node("h4", "", relationship.operation === KNOWLEDGE_UNLINK ? "Remove this exact directed relationship" : "Add this exact directed relationship"));
        const tuple = node("dl", "fact-grid"); fact(tuple, "From", relationship.from_id, true, true); fact(tuple, "Relationship", relationship.rel); fact(tuple, "To", relationship.to_id, true, true); fact(tuple, "Actual proposer", relationship.by); candidate.append(tuple, documentText(relationship.because));
        const canonical = documentText(state.reviewCandidate.document); canonical.classList.add("relationship-canonical-document"); candidate.append(append(node("details"), node("summary", "", "Exact canonical relationship document"), canonical));
      } else candidate.append(node("h4", "", "Canonical candidate text"), documentText(state.reviewCandidate.text));
      candidate.append(node("p", "mono", "Candidate digest · " + r.subject_digest));
      body.append(candidate);
    }
    const draft = intervention.draft;
    if (draft?.operation !== REVIEW_OPERATION || draft.target !== r.id || draft.subject !== r.subject_digest || !["editing", "reviewing"].includes(intervention.phase)) {
      const prepare = button("Prepare decision", () => beginReviewIntervention(r));
      prepare.disabled = !reviewCapability(r).allowed || !eligibleReview(r) || Boolean(intervention.metadata) || intervention.blocked;
      body.append(append(node("div", "intervention-actions"), prepare));
      return append(panel, heading, body);
    }
    body.append(node("p", "intervention-scope", "Scope · org / Organization-wide. Acting principal · " + state.capabilities.principal.name));
    if (intervention.phase === "editing") {
      const form = node("form");
      const choices = append(node("fieldset", "decision-choices"), node("legend", "", "Decision"));
      for (const [value, label] of Object.entries(VERDICTS)) {
        const choice = node("input");
        choice.type = "radio";
        choice.name = "verdict";
        choice.id = "decision-" + value;
        choice.value = value;
        choice.checked = draft.verdict === value;
        choice.required = true;
        choice.addEventListener("change", () => { if (choice.checked) draft.verdict = value; });
        choices.append(append(node("label", "decision-choice"), choice, node("span", "", label)));
      }
      const field = node("label", "intervention-field");
      const input = node("textarea", "intervention-rationale");
      input.id = "decision-note";
      input.name = "comment";
      input.setAttribute("aria-label", "Decision note");
      input.autocomplete = "off";
      input.value = draft.comment;
      const count = node("p", "intervention-counter");
      count.id = "decision-note-count";
      const hint = node("p", "field-hint", "The note is optional. Your choice and note stay in this page only and clear when the view reloads or changes. Nothing is sent until you review and submit the decision.");
      hint.id = "decision-draft-hint";
      input.setAttribute("aria-describedby", count.id + " " + hint.id);
      const update = () => {
        draft.comment = input.value;
        const length = byteLength(input.value);
        count.textContent = length + " / 2048 UTF-8 bytes";
        input.setAttribute("aria-invalid", String(!unicodeText(input.value) || length > 2048));
      };
      input.addEventListener("input", update);
      update();
      append(field, node("span", "", "Decision note"), input, count);
      const error = node("p", "intervention-error", intervention.error);
      error.setAttribute("role", "alert");
      error.hidden = !intervention.error;
      const review = node("button", "button", "Review decision");
      review.type = "submit";
      form.addEventListener("submit", (event) => {
        event.preventDefault();
        if (!validDraft(draft) || !currentDraftEligible(draft)) {
          intervention.error = "Choose a decision and keep the optional note within 2048 UTF-8 bytes of valid text. The exact candidate must still be available; nothing has been submitted.";
          error.textContent = intervention.error;
          error.hidden = false;
          return;
        }
        intervention.phase = "reviewing";
        intervention.error = "";
        render();
        $("decision-confirmation")?.focus();
      });
      const discard = button("Discard draft", () => { intervention.draft = null; intervention.phase = "idle"; intervention.error = ""; render(); focusPanel("detail"); });
      append(form, choices, field, hint, error, append(node("div", "intervention-actions"), review, discard));
      body.append(form);
    } else {
      const confirmation = node("div", "decision-confirmation");
      confirmation.id = "decision-confirmation";
      confirmation.tabIndex = -1;
      confirmation.setAttribute("role", "group");
      confirmation.setAttribute("aria-label", "Exact-candidate decision confirmation");
      const facts = node("dl", "fact-grid");
      fact(facts, "Decision", VERDICTS[draft.verdict]);
      fact(facts, "Review identity", draft.target, true, true);
      fact(facts, "Exact candidate", draft.subject, true, true);
      append(confirmation, facts, section("Decision note", draft.comment ? documentText(draft.comment) : node("p", "detail-note", "No note supplied.")), node("p", "field-hint", draft.source_review ? "Submitting records your decision on the exact Organization candidate above. The owner quorum, source application and observed running version are separate outcomes." : "Submitting asks the service to record this verdict on the canonical candidate shown above. A recorded verdict, Review settlement, and Knowledge activation are separate outcomes."));
      const submit = button("Submit decision", () => submitIntervention(), "button");
      const edit = button("Back to editing", () => { intervention.phase = "editing"; intervention.error = ""; render(); $("decision-" + draft.verdict)?.focus(); });
      submit.disabled = edit.disabled = Boolean(intervention.reserving);
      append(body, confirmation, append(node("div", "intervention-actions"), submit, edit));
      if (intervention.error) {
        const error = node("p", "intervention-error", intervention.error);
        error.setAttribute("role", "alert");
        body.append(error);
      }
    }
    return append(panel, heading, body);
  }
  function validOrganizationReceipt(r, metadata) {
    const o = r.organization;
    assert(closedObject(o, ["proposal_state", "source_head", "source_digest", "mutation_id", "candidate_commit", "application_state", "application_reason_code", "restart_handoff_state"]));
    assert(o.source_head === metadata.base.source_head && o.source_digest === metadata.source_digest && (o.mutation_id === "" || commandID(o.mutation_id)) && (o.candidate_commit === "" || sourceCommit(o.candidate_commit)));
    if (o.proposal_state === "created") assert(r.state === "succeeded" && commandID(o.mutation_id) && sourceCommit(o.candidate_commit));
    else {
      const states = { pending: "recorded", refused: "refused", failed: "refused", unknown: "outcome_unknown" };
      const pending = o.proposal_state === "unknown" ? "unknown" : "pending";
      assert(Object.hasOwn(states, o.proposal_state) && r.state === states[o.proposal_state] && o.candidate_commit === "" && r.review.state === "unavailable" && o.application_state === pending && o.restart_handoff_state === pending && o.application_reason_code === "");
    }
    assert(["pending", "applied", "refused", "declined", "failed", "unknown"].includes(o.application_state) && ["pending", "requested", "unknown"].includes(o.restart_handoff_state));
    assert(o.restart_handoff_state !== "requested" || o.application_state === "applied");
    assert(!["applied", "refused", "failed"].includes(o.application_state) || r.review.state === "settled" && r.review.outcome === "approve");
    assert(o.application_state !== "declined" || r.review.state === "settled" && ["reject", "revise"].includes(r.review.outcome));
    const refusals = ["invalid_command", "forbidden", "stale_subject", "organization_worktree_invalid", "organization_candidate_invalid", "organization_snapshot_unsupported", "organization_deployment_unsupported"];
    assert(o.application_state === "refused" ? refusals.includes(o.application_reason_code) : o.application_state === "failed" ? o.application_reason_code === "native_apply_failed" : o.application_reason_code === "");
  }
  function validCommandReceipt(body, metadata, method, status, expectedVerdict = null, expectedTask = null) {
    assert(closedObject(body, ["api_version", "source", "data"]) && body.api_version === "hale.v1");
    validSource(body.source, metadata.application_id);
    assert(decimal(body.source.record_revision));
    const r = body.data;
    const isVerdict = metadata.operation === REVIEW_OPERATION;
    const isOrganization = metadata.operation === ORGANIZATION_OPERATION;
    const isTask = metadata.operation === TASK_OPERATION;
    const isPerson = metadata.operation === PERSON_OPERATION;
    const isCreate = metadata.operation === TASK_CREATE_OPERATION;
    assert(closedObject(r, ["command_id", "request_id", "application_id", "operation", "operation_version", "principal", "context", "target", "subject_digest", "fingerprint", "state", "reason", isCreate ? "task_create" : isPerson ? "person" : isTask ? "task" : isOrganization ? "organization" : isVerdict ? "verdict" : "proposal", "review", "activation"]));
    assert(commandID(r.command_id) && commandID(r.fingerprint) && r.request_id === metadata.request_id && r.application_id === metadata.application_id && r.operation === metadata.operation && r.operation_version === metadata.operation_version);
    assert(closedObject(r.principal, ["mode", "name"]) && r.principal.mode === metadata.principal.mode && r.principal.name === metadata.principal.name);
    assert(closedObject(r.context, ["application_id", "position_id"]) && r.context.application_id === metadata.application_id && r.context.position_id === metadata.position_id);
    assert(closedObject(r.target, ["application_id", "kind", "id"]) && r.target.application_id === metadata.application_id && r.target.kind === metadata.target_kind && r.target.id === metadata.target_id && r.subject_digest === metadata.subject_digest);
    assert(["recorded", "admitted", "running", "succeeded", "refused", "failed", "outcome_unknown"].includes(r.state) && unicodeText(r.reason));
    if (isPerson) {
      assert(closedObject(r.person, ["state", "from", "to", "event_id", "transferred"]) && r.person.from === metadata.target_id && (r.person.to === "" || commandID(r.person.to)) && r.person.to !== r.person.from);
      assert(r.person.state === "applied" ? r.state === "succeeded" && sourceCommit(r.person.event_id) && decimal(r.person.transferred) && BigInt(r.person.transferred) <= 32n && (r.person.transferred === "0" || r.person.to !== "") : r.person.state === "unknown" && r.state === "outcome_unknown" && r.person.event_id === "" && r.person.transferred === "");
      assert(method !== "POST" || expectedTask && r.person.from === expectedTask.from && r.person.to === expectedTask.to);
    } else if (isTask) {
      assert(closedObject(r.task, ["state", "from", "to", "event_id"]) && commandID(r.task.from) && commandID(r.task.to) && r.task.from !== r.task.to);
      assert(r.task.state === "applied" ? r.state === "succeeded" && sourceCommit(r.task.event_id) : r.task.state === "unknown" && r.state === "outcome_unknown" && r.task.event_id === "");
      assert(method !== "POST" || expectedTask && r.task.from === expectedTask.from && r.task.to === expectedTask.to);
    } else if (isCreate) {
      const t = r.task_create;
      assert(closedObject(t, ["intent_id", "intent_state", "task_id", "event_id"]) && ["requested", "offered", "refused", "born", "unknown"].includes(t.intent_state));
      assert(t.intent_state === "unknown" ? r.state === "outcome_unknown" && t.intent_id === "" && t.task_id === "" && t.event_id === "" : r.state === "succeeded" && /^i[0-9a-f]{1,16}$/.test(t.intent_id) && sourceCommit(t.event_id) && (t.intent_state === "born" ? commandID(t.task_id) : t.task_id === ""));
    } else if (isOrganization) validOrganizationReceipt(r, metadata);
    else if (isVerdict) {
      assert(closedObject(r.verdict, ["value", "state"]) && typeof r.verdict.value === "string" && Object.hasOwn(VERDICTS, r.verdict.value) && ["pending", "accepted", "refused", "unknown"].includes(r.verdict.state));
      assert(method !== "POST" || r.verdict.value === expectedVerdict);
      assert(r.state !== "succeeded" || r.verdict.state === "accepted");
      assert(r.state !== "refused" || r.verdict.state === "refused");
    } else {
      assert(closedObject(r.proposal, ["state", "candidate_digest", "review_id"]) && ["pending", "created", "refused", "unknown"].includes(r.proposal.state));
      assert([r.proposal.candidate_digest, r.proposal.review_id].every((id) => id === "" || commandID(id)));
      assert(r.proposal.state !== "created" || (commandID(r.proposal.candidate_digest) && commandID(r.proposal.review_id)));
      assert(r.state !== "succeeded" || r.proposal.state === "created");
      assert(r.state !== "refused" || r.proposal.state === "refused");
    }
    assert(closedObject(r.review, ["state", "outcome", "subject_digest"]) && ["unavailable", "pending", "settled"].includes(r.review.state) && ["", "approve", "reject", "revise", "abstain"].includes(r.review.outcome));
    const exactSubject = isTask || isPerson || isCreate ? "" : isVerdict ? metadata.subject_digest : isOrganization ? r.organization.candidate_commit : r.proposal.candidate_digest;
    const created = !isTask && !isPerson && !isCreate && (isVerdict || (isOrganization ? r.organization.proposal_state === "created" : r.proposal.state === "created"));
    assert(r.review.state === "unavailable" ? r.review.outcome === "" && r.review.subject_digest === "" : created && r.review.subject_digest === exactSubject && (r.review.state === "pending" ? r.review.outcome === "" : r.review.outcome !== ""));
    assert(closedObject(r.activation, ["state", "reason"]) && ["unknown", "pending", "adopted", "refused"].includes(r.activation.state) && unicodeText(r.activation.reason));
    assert(!(metadata.source_review === true || isOrganization || isTask || isPerson || isCreate) || r.activation.state === "unknown" && r.activation.reason === "");
    assert(!(isTask || isPerson || isCreate) || r.review.state === "unavailable");
    assert(r.activation.state !== "adopted" || (created && r.review.state === "settled" && r.review.outcome === "approve"));
    assert(method === "GET" ? status === 200 : status === (COMMAND_TERMINAL.has(r.state) ? 200 : 202));
    return r;
  }
  async function commandRequest(method, metadata, payload, signal) {
    const pending = new AbortController();
    const cancelCommand = () => pending.abort();
    signal.addEventListener("abort", cancelCommand, { once: true });
    if (signal.aborted) cancelCommand();
    const timeout = setTimeout(() => pending.abort(), READ_TIMEOUT_MS);
    const path = API + "/" + encodeURIComponent(metadata.application_id) + "/commands" + (method === "GET" ? "?" + new URLSearchParams({ request_id: metadata.request_id }) : "");
    try {
      const response = await fetch(path, { method, signal: pending.signal, credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json", ...(method === "POST" ? { "Content-Type": "application/json", "X-Hale-Command": "1" } : {}) }, ...(method === "POST" ? { body: JSON.stringify(payload) } : {}) });
      if (response.status === 401) throw new ReadError(401, "unauthenticated", "Sign in again to recover this request.");
      if (response.status === 409) {
        const failure = await response.json();
        if (closedObject(failure, ["api_version", "error"]) && failure.api_version === "hale.v1" && closedObject(failure.error, ["code", "message", "retryable"]) && failure.error.code === "command_context_changed" && typeof failure.error.message === "string" && typeof failure.error.retryable === "boolean") {
          throw new ReadError(409, "command_context_changed", "The signed-in identity changed before submission.");
        }
      }
      if (!response.ok) throw new ReadError(response.status, "command_unconfirmed", "The request outcome could not be confirmed.");
      const body = await response.json();
      return { receipt: validCommandReceipt(body, metadata, method, response.status, payload?.arguments?.verdict, payload?.operation === PERSON_OPERATION ? { from: payload.target.id, to: payload.arguments.to } : payload?.operation === TASK_OPERATION ? { from: payload.preconditions.assignee, to: payload.arguments.to } : null), source: body.source };
    } finally {
      clearTimeout(timeout);
      signal.removeEventListener("abort", cancelCommand);
    }
  }
  function commandStillCurrent(token, scope, signal) {
    return token === commandGeneration && !signal.aborted && scope === commandScope() && scope === intervention.scope;
  }
  function commandAuthenticationLost(contextChanged = false) {
    const route = state.route;
    cancel();
    state = { ...blankState(route), phase: "error", error: contextChanged ? new ReadError(409, "command_context_changed", "The signed-in identity changed before submission.") : new ReadError(401, "unauthenticated", "Sign in to recover the saved request.") };
    render();
    ui.announcement.textContent = (contextChanged ? "Sign-in identity changed." : "Sign in required.") + " Draft and receipt data cleared; the request identity is retained for recovery.";
  }
  async function deliverCommand(method, metadata, payload = null) {
    if (commandController) return;
    const restoreCheckFocus = document.activeElement?.id === "command-check-status";
    const token = ++commandGeneration;
    const scope = intervention.scope;
    commandController = new AbortController();
    const signal = commandController.signal;
    const previous = intervention.receipt;
    intervention.phase = method === "POST" ? "submitting" : "recovering";
    intervention.receipt = null;
    intervention.error = "";
    render();
    let refreshTask = false;
    try {
      const result = await commandRequest(method, metadata, payload, signal);
      if (!commandStillCurrent(token, scope, signal)) return;
      if (previous) assert(result.receipt.command_id === previous.receipt.command_id && result.receipt.fingerprint === previous.receipt.fingerprint && (metadata.operation !== REVIEW_OPERATION || result.receipt.verdict.value === previous.receipt.verdict.value), "The recovered request identity changed.");
      intervention.receipt = result;
      intervention.phase = "result";
      refreshTask = metadata.operation === TASK_OPERATION && result.receipt.task.state === "applied" && state.route.view === "tasks" && state.route.id === metadata.target_id && state.source && BigInt(state.source.record_revision) < BigInt(result.source.record_revision);
      refreshTask = refreshTask || metadata.operation === PERSON_OPERATION && result.receipt.person.state === "applied" && state.route.view === "tasks" && state.route.assignee === metadata.target_id && state.source && BigInt(state.source.record_revision) < BigInt(result.source.record_revision);
      ui.announcement.textContent = metadata.operation === TASK_CREATE_OPERATION ? "New task status loaded. The organism answers separately; check again to follow the ask." : metadata.operation === PERSON_OPERATION ? "Retirement status loaded. Inspect the recorded transfer and current person state." : metadata.operation === TASK_OPERATION ? "Reassignment status loaded. The Task remains responsible for its original obligation." : metadata.source_review || metadata.operation === ORGANIZATION_OPERATION ? "Request status loaded. The decision, owner quorum, source application and running result are separate facts." : "Request status loaded. Command outcome, Review settlement and adoption are separate facts.";
    } catch (error) {
      if (!commandStillCurrent(token, scope, signal)) return;
      if (error.status === 401) { commandAuthenticationLost(); return; }
      if (error.code === "command_context_changed") { commandAuthenticationLost(true); return; }
      intervention.phase = "uncertain";
      intervention.receipt = null;
      intervention.error = error.status === 404 ? "No receipt was found yet. An earlier in-flight request could still be accepted. Keep this request identity and check again; no new request has been sent." : error.status === 403 ? "This principal cannot currently read or submit the request. Its saved identity is retained; no new request has been sent." : error.status === 503 ? "The request outcome is unavailable. Delivery may have occurred. Its identity is retained; check its status when the service is available." : "The request outcome could not be verified. Delivery may have occurred. Check the saved request; it will not be submitted again.";
      ui.announcement.textContent = "Request outcome is unconfirmed. Its identity is saved; automatic resubmission is disabled.";
    } finally {
      payload = null;
      if (token === commandGeneration) {
        commandController = null;
        if (refreshTask) { refresh(); return; }
        render();
        if (document.activeElement === document.body) {
          if (method === "POST") $("command-recovery")?.focus();
          else if (restoreCheckFocus) $("command-check-status")?.focus({ preventScroll: true });
        }
      }
    }
  }
  async function submitIntervention() {
    const draft = intervention.draft;
    if (intervention.phase !== "reviewing" || intervention.reserving || !validDraft(draft) || !currentDraftEligible(draft) || intervention.metadata || intervention.blocked) return;
    if (typeof navigator.locks?.request !== "function" || typeof crypto.randomUUID !== "function") {
      intervention.error = "This browser cannot safely reserve a recoverable request across tabs. Nothing was submitted; a secure request identity and browser locks are required.";
      render();
      return;
    }
    const scope = intervention.scope;
    const generationBeforeReservation = commandGeneration;
    const stillReviewing = () => scope === commandScope() && scope === intervention.scope && commandGeneration === generationBeforeReservation && intervention.phase === "reviewing" && intervention.draft === draft && !intervention.metadata && !intervention.blocked && validDraft(draft) && currentDraftEligible(draft);
    intervention.reserving = true;
    intervention.error = "";
    render();
    let reservation;
    try {
      reservation = await navigator.locks.request(recoveryKey(scope), { mode: "exclusive", ifAvailable: true }, (lock) => {
        if (!lock) return { error: "Another tab is reserving a request. Nothing was submitted. Check the request status in that tab before continuing." };
        // Acquiring a browser lock is asynchronous. Recheck the authenticated
        // view and the exact draft before creating or touching its identity.
        if (!stillReviewing()) return { stale: true };
        const key = recoveryKey(scope);
        if (localStorage.getItem(key) !== null) return { existing: true };
        const isVerdict = draft.operation === REVIEW_OPERATION;
        const metadata = { version: 2, application_id: state.app.id, principal: { mode: state.capabilities.principal.mode, name: state.capabilities.principal.name }, request_id: crypto.randomUUID(), operation: draft.operation, operation_version: "1", position_id: "org", target_kind: isVerdict ? "dna.review" : "dna.practice", target_id: draft.target, subject_digest: draft.subject };
        if (draft.source_review === true) { metadata.version = 3; metadata.source_review = true; }
        const isOrganization = draft.operation === ORGANIZATION_OPERATION;
        if (isOrganization) { metadata.version = 4; metadata.target_kind = "dna.organization.module"; metadata.base = { ...draft.base }; metadata.source_digest = draft.source_digest; }
        const isTask = draft.operation === TASK_OPERATION;
        if (isTask) { metadata.version = 5; metadata.target_kind = "dna.task"; }
        const isPerson = draft.operation === PERSON_OPERATION;
        if (isPerson) { metadata.version = 6; metadata.target_kind = "dna.person"; }
        const isCreate = draft.operation === TASK_CREATE_OPERATION;
        if (isCreate) { metadata.version = 7; metadata.target_kind = "dna.record"; }
        const payload = { request_id: metadata.request_id, operation: metadata.operation, operation_version: metadata.operation_version, context: { application_id: metadata.application_id, position_id: metadata.position_id }, target: { application_id: metadata.application_id, kind: metadata.target_kind, id: metadata.target_id }, preconditions: isOrganization ? { principal: { ...metadata.principal }, base: { ...metadata.base } } : { subject_digest: metadata.subject_digest, principal: { mode: metadata.principal.mode, name: metadata.principal.name }, ...(isVerdict ? { review_state: "pending" } : {}) }, arguments: isOrganization ? { source_text: draft.source_text, rationale: draft.rationale } : isVerdict ? { verdict: draft.verdict, comment: draft.comment } : { text: draft.text, rationale: draft.rationale } };
        if (isTask) { payload.preconditions.assignee = draft.from; payload.arguments = { to: draft.to }; }
        if (isPerson) payload.arguments = { to: draft.to };
        if (isCreate) { payload.preconditions = { record_head: draft.record_head, principal: { mode: metadata.principal.mode, name: metadata.principal.name } }; payload.arguments = { outcome: draft.outcome, to: draft.to }; }
        if (byteLength(JSON.stringify(payload)) > 32768) return { error: "The JSON-encoded request exceeds 32768 bytes. Shorten its text before submitting; nothing was sent or saved." };
        const serialized = JSON.stringify(metadata);
        localStorage.setItem(key, serialized);
        if (localStorage.getItem(key) !== serialized) throw new Error("recovery identity was not persisted");
        return { metadata, payload };
      });
    } catch {
      reservation = { error: "The request identity could not be safely saved. Nothing was submitted. Enable browser storage before sending a request." };
    }
    if (!stillReviewing() || reservation.stale) return;
    intervention.reserving = false;
    if (reservation.error) {
      intervention.error = reservation.error;
      render();
      return;
    }
    if (reservation.existing) {
      intervention.draft = null;
      intervention.phase = "idle";
      restoreIntervention();
      render();
      if (intervention.metadata) void checkCommandStatus();
      return;
    }
    intervention.metadata = reservation.metadata;
    intervention.draft = null;
    if (draft.operation === ORGANIZATION_OPERATION) {
      organizationDraftController?.destroy(); organizationDraftController = null; organizationDraftHost = null;
    }
    await deliverCommand("POST", reservation.metadata, reservation.payload);
    reservation = null;
  }
  async function checkCommandStatus() {
    if (!intervention.metadata || !recoveryCapability().supported || commandController || intervention.scope !== commandScope()) return;
    await deliverCommand("GET", intervention.metadata);
  }
  async function dismissCompletedRequest() {
    const result = intervention.receipt;
    const scope = intervention.scope;
    const metadata = intervention.metadata;
    const token = commandGeneration;
    if (!result || !COMMAND_TERMINAL.has(result.receipt.state) || typeof navigator.locks?.request !== "function") return;
    try {
      const removed = await navigator.locks.request(recoveryKey(scope), { mode: "exclusive", ifAvailable: true }, (lock) => {
        if (!lock) throw new Error("request reservation is in use");
        if (scope !== commandScope() || scope !== intervention.scope || token !== commandGeneration || intervention.receipt !== result) return false;
        const key = recoveryKey(scope);
        const raw = localStorage.getItem(key);
        if (!raw || JSON.parse(raw).request_id !== metadata.request_id) throw new Error("recovery identity changed");
        localStorage.removeItem(key);
        if (localStorage.getItem(key) !== null) throw new Error("recovery identity retained");
        return true;
      });
      if (!removed || scope !== commandScope() || scope !== intervention.scope || token !== commandGeneration) return;
      // The terminal receipt may describe settlement or adoption newer than
      // the inspected domain snapshot. Re-read before offering another action.
      refresh();
    } catch {
      if (scope !== commandScope() || scope !== intervention.scope || token !== commandGeneration) return;
      intervention.error = "The saved identity could not be cleared. Keep using this request status; a new request remains blocked.";
      render();
    }
  }
  function commandOutcomeMap(result, metadata) {
    if (metadata.operation === ORGANIZATION_OPERATION) return organizationOutcomeMap(result.receipt);
    const r = result.receipt, isVerdict = metadata.operation === REVIEW_OPERATION;
    const bindingReview = isVerdict && (state.detail?.knowledge_binding_digest === metadata.subject_digest || state.reviewCandidate?.kind === "knowledge_binding_change" || knowledgeCommand.result?.receipt?.binding?.candidate_digest === metadata.subject_digest);
    const relationshipReview = isVerdict && (state.detail?.knowledge_edge_digest === metadata.subject_digest || state.reviewCandidate?.kind === "knowledge_edge_change" || knowledgeCommand.result?.receipt?.relationship?.candidate_digest === metadata.subject_digest);
    const stages = [
      { key: "command", title: "Command", value: r.state, tone: r.state === "succeeded" ? "confirmed" : ["failed", "refused"].includes(r.state) ? "refused" : r.state === "outcome_unknown" ? "unknown" : "pending", explanation: "This is the outcome of this saved request. It does not establish the candidate's adoption." },
      isVerdict
        ? { key: "proposal", title: "This verdict", value: VERDICTS[r.verdict.value] + " · " + r.verdict.state, tone: r.verdict.state === "accepted" ? "confirmed" : r.verdict.state === "refused" ? "refused" : r.verdict.state === "unknown" ? "unknown" : "pending", explanation: "The service reports this request's verdict separately from the overall Review. Another person's decision cannot establish acceptance of this command." }
        : { key: "proposal", title: "Proposal", value: r.proposal.state, tone: r.proposal.state === "created" ? "confirmed" : r.proposal.state === "refused" ? "refused" : r.proposal.state === "unknown" ? "unknown" : "pending", explanation: r.proposal.state === "created" ? "The candidate and its exact Review have been created. Open either object to inspect it; creation does not activate the replacement." : "The service has not established a created candidate for this request. Check the saved request to follow its outcome." },
      { key: "review", title: isVerdict ? "Review settlement" : "Exact-candidate Review", value: r.review.state + (r.review.outcome ? " · " + OUTCOMES[r.review.outcome] : ""), tone: r.review.state === "unavailable" ? "unknown" : r.review.state === "pending" ? "pending" : r.review.outcome === "approve" ? "confirmed" : "refused", explanation: "This is the decision on the exact candidate. Approval does not establish adoption." },
      { key: "adoption", title: "Adoption", value: r.activation.state === "adopted" ? "Adopted" : r.activation.state === "refused" ? "Refused" : r.activation.state === "pending" ? "Pending — awaiting adoption" : "Unknown — adoption is not established", tone: r.activation.state === "adopted" ? "confirmed" : r.activation.state === "refused" ? "refused" : r.activation.state === "pending" ? "pending" : "unknown", explanation: r.activation.reason || (r.activation.state === "adopted" ? "The receipt establishes adoption of this candidate. The practice page has its own captured snapshot; follow the candidate to read its current state." : "Keep the request identity and check again for evidence of adoption or refusal. A successful command or approved Review cannot fill in this missing result.") }
    ];
    if (bindingReview) stages[3] = { key: "adoption", title: "Binding effect", value: "Reported by binding request", tone: "unknown", explanation: "This receipt establishes the Review decision. The original binding request separately reports whether the exact applicability change was applied, declined or refused, and whether the graph reflects it." };
    if (relationshipReview) stages[3] = { key: "adoption", title: "Relationship effect", value: "Reported by relationship request", tone: "unknown", explanation: "This receipt establishes the Review decision. The original relationship request separately reports the exact directed effect or refusal and fresh graph observation." };
    if (metadata.source_review) stages[3] = { key: "adoption", title: "Organization result", value: "Follow change status", tone: "unknown", explanation: "This receipt reports your decision and the overall Review. Open the decision Review to follow source application, launch history and separately verified running status." };
    return renderCommandStages(stages, r);
  }
  function organizationOutcomeMap(r) {
    const o = r.organization;
    const tone = value => ["succeeded", "created", "applied", "requested"].includes(value) ? "confirmed" : ["refused", "failed", "declined"].includes(value) ? "refused" : ["unknown", "outcome_unknown", "unavailable"].includes(value) ? "unknown" : "pending";
    return renderCommandStages([
      { key: "command", title: "Request", value: r.state, tone: tone(r.state), explanation: "The saved source request has its own durable identity. Recovering it reads the original request; it does not create another proposal." },
      { key: "candidate", title: "Candidate", value: o.proposal_state, tone: tone(o.proposal_state), explanation: o.proposal_state === "created" ? "Native verification produced this exact candidate and its Review. The candidate remains inspectable as its later application progresses." : "The service has not established a created candidate. Keep this request identity to follow its outcome." },
      { key: "review", title: "Review", value: r.review.state + (r.review.outcome ? " · " + OUTCOMES[r.review.outcome] : ""), tone: r.review.state === "settled" ? r.review.outcome === "approve" ? "confirmed" : "refused" : tone(r.review.state), explanation: "The exact candidate's authority and affected-owner quorum decide this Review. Approval is separate from source application." },
      { key: "application", title: "Source application", value: o.application_state, tone: tone(o.application_state), explanation: o.application_reason_code ? "The native source application reported: " + o.application_reason_code + ". Inspect the current Organization and the exact candidate before proposing another change." : o.application_state === "applied" ? "The native apply receipt establishes that this candidate reached source. That historical fact does not establish a running process." : o.application_state === "declined" ? "The native application path recorded the Review's rejection or revision request. No applied candidate is established by this request." : "This stage follows the original native source-application result. An unknown effect needs recovery, even if the proposal and Review are complete." },
      { key: "handoff", title: "Restart handoff", value: o.restart_handoff_state, tone: tone(o.restart_handoff_state), explanation: "A requested handoff means the owning host was asked to restart from the applied candidate. It does not establish a process start or healthy operation." },
      { key: "running", title: "Running version", value: "Not established by this receipt", tone: "unknown", explanation: "The running version needs a separate observation tied to this exact candidate, binary and process. A changed chart or approved Review cannot supply that evidence." }
    ], r);
  }
  function renderCommandStages(stages, r) {
    const map = node("div", "command-outcome-map");
    map.style.setProperty("--outcome-stages", String(stages.length));
    const trail = node("ol", "intervention-stage-list"); trail.setAttribute("aria-label", "Request and outcome");
    const inspector = node("div", "outcome-inspector"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected outcome");
    const selected = stages.some(stage => stage.key === intervention.outcomeStage) ? intervention.outcomeStage : (stages.find((stage) => stage.tone === "refused" || stage.tone === "pending") || stages.at(-1)).key;
    const choose = (key) => {
      intervention.outcomeStage = key;
      for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === key));
      const stage = stages.find((item) => item.key === key);
      inspector.dataset.state = stage.tone;
      inspector.replaceChildren(node("h4", "", stage.title), node("p", "outcome-value", stage.value), node("p", "", stage.explanation));
      if (key === "command" && r.reason) inspector.append(node("p", "", r.reason));
    };
    for (const [index, stage] of stages.entries()) {
      const control = button("", () => choose(stage.key), "outcome-stage");
      control.dataset.stage = stage.key; control.dataset.state = stage.tone;
      control.setAttribute("aria-label", stage.title);
      const value = node("span", "outcome-stage-value", stage.value); value.id = "outcome-" + stage.key + "-value";
      control.setAttribute("aria-describedby", value.id);
      const marker = node("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true");
      append(control, marker, node("strong", "", stage.title), value);
      trail.append(append(node("li"), control));
    }
    choose(selected);
    return append(map, trail, inspector);
  }
  function renderCommandRecovery() {
    if (!intervention.metadata && !intervention.blocked) return null;
    if (intervention.metadata?.operation === PERSON_OPERATION) return renderPersonRecovery();
    if (intervention.metadata?.operation === TASK_OPERATION) return renderTaskRecovery();
    if (intervention.metadata?.operation === TASK_CREATE_OPERATION) return renderTaskCreateRecovery();
    const panel = node("section", "panel practice-intervention intervention-recovery");
    panel.id = "command-recovery";
    panel.tabIndex = -1;
    panel.setAttribute("role", "region");
    panel.setAttribute("aria-label", "Command recovery");
    const isVerdict = intervention.metadata?.operation === REVIEW_OPERATION;
    const isOrganization = intervention.metadata?.operation === ORGANIZATION_OPERATION;
    const heading = append(node("div", "intervention-heading"), append(node("div"), node("h3", "", isOrganization ? "Organization source request" : isVerdict ? "Review decision request" : "Practice request"), node("p", "", "The saved request belongs to this application and signed-in principal. Recovery reads its status; it does not submit it again.")));
    const body = node("div", "intervention-body");
    const metadata = intervention.metadata;
    if (!metadata) return append(panel, heading, append(body, node("p", "intervention-error", intervention.error)));
    const busy = ["submitting", "recovering"].includes(intervention.phase);
    const result = intervention.receipt;
    const status = node("p", "intervention-status" + (!result ? " uncertain" : ""), busy ? intervention.phase === "submitting" ? "Submitting the saved request once. Leaving this page cannot undo an accepted request." : "Checking the saved request identity…" : result ? "Command state · " + result.receipt.state : intervention.error);
    status.setAttribute("role", "status");
    body.append(status);
    const evidence = node("details", "command-evidence");
    evidence.append(node("summary", "", "Exact request and receipt evidence"));
    const facts = node("dl", "fact-grid");
    fact(facts, "Request identity", metadata.request_id, true, true);
    fact(facts, isOrganization ? "Original source digest" : isVerdict ? "Exact candidate" : "Exact predecessor", metadata.subject_digest, true, true);
    if (isOrganization) {
      fact(facts, "Proposed source digest", metadata.source_digest, true, true);
      for (const key of ORGANIZATION_BASE) fact(facts, key.replaceAll("_", " "), metadata.base[key], true, true);
    }
    if (isVerdict) fact(facts, "Review identity", metadata.target_id, true, true);
    fact(facts, "Scope", "org / Organization-wide");
    fact(facts, "Principal", metadata.principal.mode + " · " + metadata.principal.name);
    evidence.append(facts);
    if (result) {
      const r = result.receipt;
      body.append(commandOutcomeMap(result, metadata), node("p", "detail-note", isOrganization ? "Candidate creation, Review, source application and restart handoff come from this request's recorded evidence. The running version is verified separately." : metadata.source_review ? "Your recorded decision and the full owner quorum are separate. The source request reports application and handoff; a successful launch needs its own evidence." : isVerdict ? "An accepted verdict does not establish Review settlement or adoption. Review and adoption may reflect other recorded decisions; this request's verdict is reported separately." : "Review approval does not establish adoption. These facts come from the returned receipt's Record basis."));
      const basis = node("dl", "fact-grid");
      fact(basis, "Command identity", r.command_id, true, true);
      fact(basis, "Receipt Record head", result.source.record_head, true, true);
      fact(basis, "Receipt Record revision", result.source.record_revision, false, true);
      if (isOrganization && r.organization.candidate_commit) fact(basis, "Candidate commit", r.organization.candidate_commit, true, true);
      if (!isVerdict && !isOrganization && r.proposal.candidate_digest) fact(basis, "Candidate digest", r.proposal.candidate_digest, true, true);
      if (r.review.subject_digest) fact(basis, "Review subject digest", r.review.subject_digest, true, true);
      evidence.append(basis);
      if (r.reason) body.append(node("p", "detail-note", r.reason));
      if (r.activation.reason) body.append(node("p", "detail-note", "Adoption · " + r.activation.reason));
      const links = node("div", "intervention-actions");
      if (isOrganization) {
        links.append(navigationLink("Open Organization", routeHash(workspaceRoute("organization", { app: metadata.application_id })), "graph"));
        if (r.organization.proposal_state === "created") links.append(navigationLink("Open source Review", routeHash(workspaceRoute("reviews", { app: metadata.application_id, id: r.organization.mutation_id })), "detail"));
      } else if (isVerdict) {
        const binding = state.reviewCandidate?.kind === "knowledge_binding_change" || state.detail?.knowledge_binding_digest === metadata.subject_digest || knowledgeCommand.result?.receipt?.binding?.candidate_digest === metadata.subject_digest;
        const relationship = state.reviewCandidate?.kind === "knowledge_edge_change" || state.detail?.knowledge_edge_digest === metadata.subject_digest || knowledgeCommand.result?.receipt?.relationship?.candidate_digest === metadata.subject_digest;
        const generic = state.capabilities?.reads?.knowledge === true && state.reviewCandidate?.kind !== "practice";
        if (metadata.source_review) links.append(navigationLink("Open Organization", routeHash(workspaceRoute("organization", { app: metadata.application_id })), "graph"));
        else if (!binding && !relationship) links.append(navigationLink(generic ? "Open candidate knowledge" : "Open candidate practice", routeHash(workspaceRoute(generic ? "knowledge" : "practices", { app: metadata.application_id, id: metadata.subject_digest, locus: metadata.application_id === state.app?.id ? state.route.locus : "" })), "detail"));
        const bindingItem = state.reviewCandidate?.binding?.idea_id || (knowledgeCommand.result?.receipt?.binding?.candidate_digest === metadata.subject_digest ? knowledgeCommand.result.receipt.target.id : "");
        if (binding && bindingItem) links.append(navigationLink("Open knowledge applicability", routeHash(workspaceRoute("knowledge", { app: metadata.application_id, id: bindingItem })), "graph"));
        const relationshipItem = state.reviewCandidate?.relationship?.from_id || (knowledgeCommand.result?.receipt?.relationship?.candidate_digest === metadata.subject_digest ? knowledgeCommand.result.receipt.target.id : "");
        if (relationship && relationshipItem) links.append(navigationLink("Open relationship graph", routeHash(workspaceRoute("knowledge", { app: metadata.application_id, id: relationshipItem })), "graph"));
        links.append(navigationLink("Open decision Review", routeHash(workspaceRoute("reviews", { app: metadata.application_id, id: metadata.target_id, locus: metadata.application_id === state.app?.id ? state.route.locus : "" })), "detail"));
      } else if (r.proposal.state === "created") {
        append(links, navigationLink("Open proposed practice", routeHash(workspaceRoute("practices", { app: metadata.application_id, id: r.proposal.candidate_digest, locus: metadata.application_id === state.app?.id ? state.route.locus : "" })), "detail"), navigationLink("Open proposal review", routeHash(workspaceRoute("reviews", { app: metadata.application_id, id: r.proposal.review_id, locus: metadata.application_id === state.app?.id ? state.route.locus : "" })), "detail"));
      }
      body.append(links);
    }
    const check = button("Check request status", () => checkCommandStatus());
    check.id = "command-check-status";
    check.disabled = busy || !recoveryCapability().supported;
    const actions = append(node("div", "intervention-actions"), check);
    if (result && COMMAND_TERMINAL.has(result.receipt.state)) actions.append(button("Dismiss completed request", dismissCompletedRequest));
    body.append(actions, evidence);
    if (!recoveryCapability().supported) body.append(node("p", "detail-note", "This connection does not currently advertise the matching recovery profile. The saved identity is retained."));
    if (result && intervention.error) body.append(node("p", "intervention-error", intervention.error));
    return append(panel, heading, body);
  }

  function knowledgeRecoveryKey(scope) { return KNOWLEDGE_STORAGE + encodeURIComponent(scope); }
  function knowledgeOperation(metadata) { return metadata.version === 1 ? KNOWLEDGE_OPERATION : metadata.operation; }
  function knowledgeNodeOperation(operation) { return KNOWLEDGE_NODES.includes(operation); }
  function knowledgeBindingOperation(operation) { return KNOWLEDGE_BINDINGS.includes(operation); }
  function reviewedKnowledgeOperation(operation) { return knowledgeNodeOperation(operation) || knowledgeBindingOperation(operation); }
  function nativeKnowledgeOperation(operation) { const native = "dna.knowledge." + operation; return [KNOWLEDGE_OPERATION, KNOWLEDGE_UNLINK, ...KNOWLEDGE_NODES, ...KNOWLEDGE_BINDINGS].includes(native) ? native : ""; }
  function knowledgeTargetKind(metadata) { return metadata.version === 3 ? metadata.target_kind : "dna.knowledge.node"; }
  function knowledgeCompleted(result) { return result && (reviewedKnowledgeOperation(result.receipt.operation) || result.receipt.relationship ? ["succeeded", "refused"].includes(result.receipt.state) : result.receipt.state === "recorded"); }
  function knowledgeAccessLabel() { return knowledgeCommand.capability?.enabled ? reviewedKnowledgeOperation(knowledgeCommand.capability.operation) || knowledgeCommand.capability.mode === "review" ? "Knowledge proposals enabled" : "Relationship commands enabled" : knowledgeCommand.capability || state.capabilities?.read_only ? "Read only" : "Read access"; }
  function clearKnowledgeCommand() {
    knowledgeCommandGeneration++;
    knowledgeCommandController?.abort(); knowledgeCommandController = null;
    knowledgeCommand = { scope: "", metadata: null, result: null, capability: null, phase: "idle", error: "", projection: "unread", blocked: false };
  }
  function validKnowledgeRecovery(m) {
    const p = state.capabilities?.principal;
    const keys = ["version", "application_id", "principal", "request_id", "target_id", "record_head"];
    const version = closedObject(m, keys) && m.version === 1 || closedObject(m, [...keys, "operation"]) && m.version === 2 && [KNOWLEDGE_OPERATION, KNOWLEDGE_UNLINK].includes(m.operation) || closedObject(m, [...keys, "operation", "target_kind"]) && m.version === 3 && knowledgeNodeOperation(m.operation) && m.target_kind === (m.operation === "dna.knowledge.node.propose" ? "dna.knowledge.collection" : "dna.knowledge.node") || closedObject(m, [...keys, "operation", "binding"]) && m.version === 4 && knowledgeBindingOperation(m.operation) && closedObject(m.binding, ["author", "target"]) && [m.binding.author, m.binding.target].every(value => commandID(value, 256));
    return version && m.application_id === state.app?.id
      && closedObject(m.principal, ["mode", "name"]) && m.principal.mode === p?.mode && m.principal.name === p?.name
      && commandID(m.request_id, 128) && (knowledgeTargetKind(m) === "dna.knowledge.collection" ? commandID(m.target_id, 1024) : receiptID(m.target_id)) && commandID(m.record_head);
  }
  function restoreKnowledgeCommand() {
    knowledgeCommand.scope = commandScope();
    if (!knowledgeCommand.scope) return;
    try {
      const raw = localStorage.getItem(knowledgeRecoveryKey(knowledgeCommand.scope));
      if (raw === null) return;
      const metadata = JSON.parse(raw);
      if (!validKnowledgeRecovery(metadata)) throw new Error("invalid Knowledge recovery identity");
      knowledgeCommand.metadata = metadata; knowledgeCommand.phase = "uncertain";
      knowledgeCommand.error = "A saved Knowledge request may already be recorded. Recover it by its original identity; it will not be submitted again.";
    } catch {
      knowledgeCommand.blocked = true;
      knowledgeCommand.error = "Knowledge recovery storage is unavailable or invalid. New Knowledge submissions are blocked until the saved identity can be checked.";
    }
  }
  function validKnowledgeCapability(body, appId, principal, operation) {
    assert(closedObject(body, ["api_version", "source", "data"]) && body.api_version === "hale.v1"); validSource(body.source, appId);
    const c = body.data;
    const isNode = knowledgeNodeOperation(operation), isBinding = knowledgeBindingOperation(operation);
    assert(closedObject(c, ["profile", "application_id", "principal", "position_id", "available", "authorized", "mode", "reason", "policy_basis", "recovery", isNode ? "max_text_bytes" : isBinding ? "max_locus_bytes" : "max_rel_bytes", "max_rationale_bytes", "max_request_bytes"]));
    if (c.application_id !== appId || c.principal?.mode !== principal.mode || c.principal?.name !== principal.name) throw new ReadError(409, "command_context_changed", "The signed-in identity changed. The draft cannot be submitted.");
    assert(closedObject(c.principal, ["mode", "name"]) && c.profile === operation + ".v1" && c.position_id === "org" && typeof c.available === "boolean" && typeof c.authorized === "boolean"
      && ["direct", "review", "unavailable"].includes(c.mode) && unicodeText(c.reason) && unicodeText(c.policy_basis) && c.recovery === "record_lifetime"
      && (isNode ? c.max_text_bytes === "8192" : isBinding ? c.max_locus_bytes === "256" : c.max_rel_bytes === "256") && c.max_rationale_bytes === "2048" && c.max_request_bytes === (isNode ? "98304" : "32768"));
    return { ...c, operation, enabled: c.available && c.authorized && (isNode || isBinding ? c.mode === "review" : ["direct", "review"].includes(c.mode)) };
  }
  async function readKnowledgeCapability(appId, principal, operation, signal) {
    const body = await request(API + "/" + encodeURIComponent(appId) + "/dna/knowledge/commands/capability" + (operation === KNOWLEDGE_OPERATION ? "" : "?" + new URLSearchParams({ operation })), signal);
    return { capability: validKnowledgeCapability(body, appId, principal, operation), source: body.source };
  }
  function validKnowledgeCommandReceipt(body, metadata, method, status) {
    assert(closedObject(body, ["api_version", "source", "data"]) && body.api_version === "hale.v1"); validSource(body.source, metadata.application_id);
    const r = body.data;
    const isNode = knowledgeNodeOperation(knowledgeOperation(metadata)), isBinding = knowledgeBindingOperation(knowledgeOperation(metadata)), isRelationship = !isNode && !isBinding && Object.prototype.hasOwnProperty.call(r, "relationship");
    assert(closedObject(r, ["command_id", "request_id", "application_id", "operation", "operation_version", "principal", "context", "target", "fingerprint", "state", "details_visible", "edge_id", "event_id", "sequence", "admission_head", "authority", "authority_basis", ...(isNode ? ["node"] : isBinding ? ["binding"] : isRelationship ? ["relationship"] : [])]));
    assert(commandID(r.command_id) && commandID(r.fingerprint) && r.request_id === metadata.request_id && r.application_id === metadata.application_id && r.operation === knowledgeOperation(metadata) && r.operation_version === "1");
    assert(closedObject(r.principal, ["mode", "name"]) && r.principal.mode === metadata.principal.mode && r.principal.name === metadata.principal.name);
    assert(closedObject(r.context, ["application_id", "position_id"]) && r.context.application_id === metadata.application_id && r.context.position_id === "org");
    assert(closedObject(r.target, ["application_id", "kind", "id"]) && r.target.application_id === metadata.application_id && r.target.kind === knowledgeTargetKind(metadata) && typeof r.details_visible === "boolean");
    assert(r.details_visible ? r.target.id === metadata.target_id && (isNode || isBinding ? r.edge_id === "" : receiptID(r.edge_id)) : r.target.id === "" && r.edge_id === "");
    assert([r.event_id, r.admission_head, r.authority].every(value => commandID(value, 256)) && commandID(r.authority_basis, 2048) && r.admission_head === metadata.record_head && decimal(r.sequence) && BigInt(body.source.record_revision) > BigInt(r.sequence));
    if (isNode) {
      const n = r.node;
      assert(closedObject(n, ["proposal_state", "candidate_digest", "review_id", "review_state", "review_outcome", "activation_state", "activation_reason", "reason"]));
      assert(["pending", "created", "refused", "unknown"].includes(n.proposal_state) && ["recorded", "succeeded", "refused", "outcome_unknown"].includes(r.state));
      assert(!r.details_visible || r.state === ({ pending: "recorded", created: "succeeded", refused: "refused", unknown: "outcome_unknown" })[n.proposal_state]);
      assert((n.candidate_digest === "" || receiptID(n.candidate_digest)) && (n.review_id === "" || commandID(n.review_id)) && unicodeText(n.reason) && unicodeText(n.activation_reason));
      assert(["unavailable", "pending", "settled"].includes(n.review_state) && (n.review_state === "settled" ? ["approve", "reject", "revise"].includes(n.review_outcome) : n.review_outcome === ""));
      assert(["unknown", "pending", "adopted", "refused"].includes(n.activation_state));
      if (r.details_visible && n.proposal_state === "created") assert(receiptID(n.candidate_digest) && commandID(n.review_id));
      if (!r.details_visible) assert(n.proposal_state === "unknown" && n.candidate_digest === "" && n.review_id === "" && n.review_state === "unavailable" && n.activation_state === "unknown");
      if (n.activation_state !== "unknown") assert(n.proposal_state === "created" && n.review_state === "settled" && n.review_outcome === "approve");
      assert(status === (method === "POST" ? 202 : 200));
    } else if (isBinding) {
      const b = r.binding;
      assert(closedObject(b, ["binding_id", "proposal_state", "candidate_digest", "review_id", "review_state", "review_outcome", "effect_state", "effect_reason", "reason"]));
      assert(["pending", "created", "refused", "unknown"].includes(b.proposal_state) && ["recorded", "succeeded", "refused", "outcome_unknown"].includes(r.state));
      assert(!r.details_visible || r.state === ({ pending: "recorded", created: "succeeded", refused: "refused", unknown: "outcome_unknown" })[b.proposal_state]);
      assert((b.binding_id === "" || receiptID(b.binding_id)) && (b.candidate_digest === "" || receiptID(b.candidate_digest)) && (b.review_id === "" || commandID(b.review_id)) && unicodeText(b.reason) && unicodeText(b.effect_reason));
      assert(["unavailable", "pending", "settled"].includes(b.review_state) && (b.review_state === "settled" ? ["approve", "reject", "revise"].includes(b.review_outcome) : b.review_outcome === ""));
      assert(["unknown", "pending", "bound", "unbound", "declined", "refused"].includes(b.effect_state));
      if (r.details_visible) assert(receiptID(b.binding_id));
      if (r.details_visible && b.proposal_state === "created") assert(receiptID(b.candidate_digest) && commandID(b.review_id));
      if (!r.details_visible) assert(b.binding_id === "" && b.proposal_state === "unknown" && b.candidate_digest === "" && b.review_id === "" && b.review_state === "unavailable" && b.effect_state === "unknown" && b.reason === "" && b.effect_reason === "");
      if (["bound", "unbound", "refused"].includes(b.effect_state)) assert(b.proposal_state === "created" && b.review_state === "settled" && b.review_outcome === "approve");
      if (b.effect_state === "bound") assert(r.operation === "dna.knowledge.binding.bind");
      if (b.effect_state === "unbound") assert(r.operation === "dna.knowledge.binding.unbind");
      if (b.effect_state === "declined") assert(b.proposal_state === "created" && b.review_state === "settled" && ["reject", "revise"].includes(b.review_outcome));
      assert(status === (method === "POST" ? 202 : 200));
    } else if (isRelationship) {
      const e = r.relationship;
      assert(closedObject(e, ["proposal_state", "candidate_digest", "review_id", "review_state", "review_outcome", "effect_state", "effect_reason", "reason"]));
      assert(["pending", "created", "refused", "unknown"].includes(e.proposal_state) && ["recorded", "succeeded", "refused", "outcome_unknown"].includes(r.state));
      assert(!r.details_visible || r.state === ({ pending: "recorded", created: "succeeded", refused: "refused", unknown: "outcome_unknown" })[e.proposal_state]);
      assert((e.candidate_digest === "" || receiptID(e.candidate_digest)) && (e.review_id === "" || commandID(e.review_id)) && unicodeText(e.reason) && unicodeText(e.effect_reason) && byteLength(e.reason) <= 512 && byteLength(e.effect_reason) <= 512);
      assert(["unavailable", "pending", "settled"].includes(e.review_state) && (e.review_state === "settled" ? ["approve", "reject", "revise"].includes(e.review_outcome) : e.review_outcome === ""));
      assert(["unknown", "pending", "linked", "unlinked", "declined", "refused"].includes(e.effect_state));
      if (r.details_visible && e.proposal_state === "created") assert(receiptID(e.candidate_digest) && commandID(e.review_id) && e.reason === "");
      else assert(e.candidate_digest === "" && e.review_id === "" && e.review_state === "unavailable" && e.effect_state === "unknown" && e.effect_reason === "");
      if (!r.details_visible) assert(e.proposal_state === "unknown" && e.reason === "");
      if (e.proposal_state !== "refused") assert(e.reason === "");
      if (!["declined", "refused"].includes(e.effect_state)) assert(e.effect_reason === "");
      if (["linked", "unlinked", "refused"].includes(e.effect_state)) assert(e.proposal_state === "created" && e.review_state === "settled" && e.review_outcome === "approve");
      if (e.effect_state === "linked") assert(r.operation === KNOWLEDGE_OPERATION);
      if (e.effect_state === "unlinked") assert(r.operation === KNOWLEDGE_UNLINK);
      if (e.effect_state === "declined") assert(e.proposal_state === "created" && e.review_state === "settled" && ["reject", "revise"].includes(e.review_outcome));
      assert(status === (method === "POST" ? 202 : 200));
    } else { assert(r.state === "recorded"); assert(status === (method === "POST" ? 202 : 200)); }
    return { receipt: r, source: body.source };
  }
  async function knowledgeCommandRequest(method, metadata, payload, signal) {
    const pending = new AbortController(), abort = () => pending.abort();
    signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
    const timeout = setTimeout(abort, READ_TIMEOUT_MS);
    try {
      const path = API + "/" + encodeURIComponent(metadata.application_id) + "/dna/knowledge/commands" + (method === "GET" ? "?" + new URLSearchParams({ request_id: metadata.request_id }) : "");
      const response = await fetch(path, { method, signal: pending.signal, credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json", ...(method === "POST" ? { "Content-Type": "application/json", "X-Hale-Command": "1" } : {}) }, ...(method === "POST" ? { body: JSON.stringify(payload) } : {}) });
      const body = await response.json();
      if (!response.ok) {
        assert(closedObject(body, ["api_version", "error"]) && body.api_version === "hale.v1" && closedObject(body.error, ["code", "message", "retryable"]) && unicodeText(body.error.code) && unicodeText(body.error.message) && typeof body.error.retryable === "boolean");
        throw new ReadError(response.status, body.error.code, body.error.message);
      }
      return validKnowledgeCommandReceipt(body, metadata, method, response.status);
    } finally { clearTimeout(timeout); signal.removeEventListener("abort", abort); }
  }
  function knowledgeCommandCurrent(token, scope, signal) {
    return token === knowledgeCommandGeneration && !signal.aborted && scope === knowledgeCommand.scope && scope === commandScope();
  }
  function sameKnowledgeAdmission(next, previous) {
    assert(Boolean(next.relationship) === Boolean(previous.relationship), "The saved relationship admission variant changed.");
    if (next.relationship && previous.relationship && previous.details_visible && next.details_visible && previous.relationship.proposal_state === "created" && next.relationship.proposal_state === "created") assert(next.relationship.candidate_digest === previous.relationship.candidate_digest && next.relationship.review_id === previous.relationship.review_id, "The canonical relationship proposal identity changed.");
    assert(["operation", "command_id", "fingerprint", "event_id", "sequence", "admission_head", "authority", "authority_basis"].every(key => next[key] === previous[key]), "The saved Knowledge command identity or admission changed.");
    if (previous.details_visible && next.details_visible) assert(next.edge_id === previous.edge_id, "The recorded relationship identity changed.");
    if (next.node && previous.node && previous.details_visible && next.details_visible && previous.node.proposal_state === "created" && next.node.proposal_state === "created") assert(next.node.candidate_digest === previous.node.candidate_digest && next.node.review_id === previous.node.review_id, "The canonical Knowledge proposal identity changed.");
    if (next.binding && previous.binding && previous.details_visible && next.details_visible) {
      assert(next.binding.binding_id === previous.binding.binding_id, "The exact binding identity changed.");
      if (previous.binding.proposal_state === "created" && next.binding.proposal_state === "created") assert(next.binding.candidate_digest === previous.binding.candidate_digest && next.binding.review_id === previous.binding.review_id, "The canonical binding proposal identity changed.");
    }
  }
  function restrictKnowledgeCommandView() {
    knowledgeCommand.projection = "restricted";
    if (state.route.view !== "knowledge") return;
    destroyKnowledgeDraft();
    const route = firstPageRoute(state.route);
    state = { ...blankState(route), apps: state.apps, app: state.app, capabilities: state.capabilities, workingContext: state.workingContext, phase: "domain-error", error: new ReadError(404, "knowledge_not_found", (reviewedKnowledgeOperation(knowledgeOperation(knowledgeCommand.metadata)) ? "Knowledge change details" : "Relationship details") + " are unavailable under current visibility. Previously displayed Knowledge content has been cleared.") };
    replaceRoute(route);
  }
  async function observeKnowledgeRemoval(loaded, receipt, token, scope, signal) {
    if (!loaded.relationships || loaded.detailError || loaded.collection.basis.routing_version !== "0") {
      const confirmed = await knowledgeCommandRequest("GET", knowledgeCommand.metadata, null, signal);
      if (!knowledgeCommandCurrent(token, scope, signal)) throw new DOMException("Superseded graph observation", "AbortError");
      sameKnowledgeAdmission(confirmed.receipt, receipt); knowledgeCommand.result = confirmed;
      return { projection: confirmed.receipt.details_visible ? "unavailable" : "restricted", pages: 0 };
    }
    const pending = new AbortController(), abort = () => pending.abort();
    signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
    const timeout = setTimeout(abort, READ_TIMEOUT_MS);
    const current = () => { if (!knowledgeCommandCurrent(token, scope, signal) || pending.signal.aborted) throw new DOMException("Superseded graph observation", "AbortError"); };
    let page = loaded.relationships, pages = 0;
    const cursors = new Set(), edges = new Set();
    try {
      while (true) {
        current(); pages++;
        for (const edge of page.items) {
          assert(!edges.has(edge.id), "The relationship sequence repeated an identity."); edges.add(edge.id);
          if (edge.id === receipt.edge_id) return { projection: "still_present", pages };
        }
        if (!page.page.has_more) break;
        if (pages >= KNOWLEDGE_REMOVAL_PAGES) return { projection: "partial", pages };
        const cursor = page.page.next_cursor;
        assert(!cursors.has(cursor), "The relationship sequence repeated a cursor."); cursors.add(cursor);
        const params = new URLSearchParams({ id: receipt.target.id, limit: String(LIMIT), snapshot: loaded.collection.page.snapshot, cursor });
        if (loaded.route.target) params.set("target", loaded.route.target);
        const response = await request(API + "/" + encodeURIComponent(receipt.application_id) + "/dna/knowledge/edges?" + params, pending.signal);
        current(); validSource(response.source, receipt.application_id);
        validKnowledge(response.data, response.source, loaded.route, "edges", cursor);
        assert(response.source.record_head === loaded.source.record_head && response.source.record_revision === loaded.source.record_revision && sameKnowledgeBasis(response.data.basis, loaded.collection.basis), "The relationship sequence changed while checking removal.");
        page = response.data;
      }
      // Absence is meaningful only while both endpoints remain visible under
      // the same Record policy as every page. This GET never re-submits.
      const confirmed = await knowledgeCommandRequest("GET", knowledgeCommand.metadata, null, pending.signal);
      current(); sameKnowledgeAdmission(confirmed.receipt, receipt);
      knowledgeCommand.result = confirmed;
      if (!confirmed.receipt.details_visible) return { projection: "restricted", pages };
      if (confirmed.source.record_head !== loaded.source.record_head || confirmed.source.record_revision !== loaded.source.record_revision) return { projection: "changed", pages };
      return { projection: "removed", pages, snapshot: loaded.collection.page.snapshot, record_head: loaded.source.record_head };
    } finally { clearTimeout(timeout); signal.removeEventListener("abort", abort); }
  }
  async function observeReviewedRelationship(loaded, receipt, token, scope, signal) {
    const pending = new AbortController(), abort = () => pending.abort();
    signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
    const timeout = setTimeout(abort, READ_TIMEOUT_MS);
    const current = () => { if (!knowledgeCommandCurrent(token, scope, signal) || pending.signal.aborted) throw new DOMException("Superseded relationship observation", "AbortError"); };
    const removing = receipt.operation === KNOWLEDGE_UNLINK, scanRoute = { ...loaded.route, target: "", snapshot: "" }, ids = new Set(), cursors = new Set();
    let cursor = "", source = null, basis = null, snapshot = "", pages = 0, projection = "unavailable";
    try {
      const candidateResponse = await request(API + "/" + encodeURIComponent(receipt.application_id) + "/dna/reviews/candidate?" + new URLSearchParams({ id: receipt.relationship.review_id, snapshot: loaded.source.record_head }), pending.signal); current();
      validSource(candidateResponse.source, receipt.application_id);
      assert(candidateResponse.source.record_head === loaded.source.record_head && candidateResponse.source.record_revision === loaded.source.record_revision, "The exact relationship candidate changed during graph observation.");
      const candidate = validRelationshipCandidate(candidateResponse.data, receipt.application_id), e = candidate.relationship;
      assert(candidate.review_id === receipt.relationship.review_id && candidate.candidate_digest === receipt.relationship.candidate_digest && e.request_id === receipt.command_id && e.command_fingerprint === receipt.fingerprint && e.operation === receipt.operation && e.edge_id === receipt.edge_id && [e.from_id, e.to_id].includes(receipt.target.id), "The exact relationship candidate does not match the admitted request.");
      while (pages < KNOWLEDGE_REMOVAL_PAGES) {
        const query = new URLSearchParams({ id: receipt.target.id, limit: String(LIMIT) }); if (snapshot) query.set("snapshot", snapshot); if (cursor) query.set("cursor", cursor);
        const response = await request(API + "/" + encodeURIComponent(receipt.application_id) + "/dna/knowledge/edges?" + query, pending.signal); current();
        validSource(response.source, receipt.application_id); validKnowledge(response.data, response.source, { ...scanRoute, snapshot }, "edges", cursor);
        if (!source) { source = response.source; basis = response.data.basis; snapshot = response.data.page.snapshot; }
        assert(source.record_head === response.source.record_head && source.record_revision === response.source.record_revision && sameKnowledgeBasis(basis, response.data.basis), "The relationship sequence changed during observation."); pages++;
        if (basis.routing_version !== "0" || source.record_head !== loaded.source.record_head || source.record_revision !== loaded.source.record_revision) { projection = "changed"; break; }
        let match = null;
        for (const row of response.data.items) { assert(!ids.has(row.id), "The relationship sequence repeated an identity."); ids.add(row.id); if (row.id === receipt.edge_id) match = row; }
        if (match) { projection = removing ? "still_present" : match.from_id === e.from_id && match.to_id === e.to_id && match.rel === e.rel ? "observed" : "unavailable"; break; }
        if (!response.data.page.has_more) { projection = removing ? "removed" : "not_on_page"; break; }
        cursor = response.data.page.next_cursor; assert(!cursors.has(cursor), "The relationship sequence repeated a cursor."); cursors.add(cursor); projection = "partial";
      }
      const confirmed = await knowledgeCommandRequest("GET", knowledgeCommand.metadata, null, pending.signal); current(); sameKnowledgeAdmission(confirmed.receipt, receipt); knowledgeCommand.result = confirmed;
      if (!confirmed.receipt.details_visible) return { projection: "restricted", pages };
      if (!source || confirmed.source.record_head !== source.record_head || confirmed.source.record_revision !== source.record_revision) projection = "changed";
      else if (confirmed.receipt.relationship.effect_state !== (removing ? "unlinked" : "linked")) projection = "awaiting_effect";
      return { projection, pages, snapshot, record_head: source?.record_head || "" };
    } finally { clearTimeout(timeout); signal.removeEventListener("abort", abort); }
  }
  async function refreshKnowledgeCommandGraph(token, scope, signal) {
    const r = knowledgeCommand.result?.receipt;
    if (!r || !r.relationship && r.state !== "recorded") return;
    if (r.relationship && r.relationship.effect_state !== (r.operation === KNOWLEDGE_UNLINK ? "unlinked" : "linked")) { knowledgeCommand.projection = "awaiting_effect"; if (!r.details_visible) restrictKnowledgeCommandView(); return; }
    if (!r.details_visible) { restrictKnowledgeCommandView(); return; }
    if (state.route.view !== "knowledge" || state.route.id !== r.target.id) { knowledgeCommand.projection = "unread"; return; }
    knowledgeCommand.projection = "reading"; knowledgeCommand.observation = null;
    // The command is already committed. Discard old cursors and draft overlay;
    // readback failure can only change projection status, never that admission.
    destroyKnowledgeDraft();
    const route = firstPageRoute(state.route), tokenRoute = generation;
    const context = { apps: state.apps, app: state.app, capabilities: state.capabilities, workingContext: state.workingContext };
    try {
      const loaded = await readApplication(route, tokenRoute, signal, identity => {
        const p = knowledgeCommand.metadata.principal;
        if (identity.app.id !== knowledgeCommand.metadata.application_id || identity.capabilities.principal.mode !== p.mode || identity.capabilities.principal.name !== p.name) throw new ReadError(409, "command_context_changed", "The signed-in identity changed during graph readback.");
      });
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      assert(BigInt(loaded.source.record_revision) > BigInt(r.sequence), "The graph observation precedes this recorded command.");
      state = { ...state, ...loaded, phase: "ready", inspectedAt: new Date(), error: null }; replaceRoute(loaded.route);
      if (r.relationship) {
        const observation = await observeReviewedRelationship(loaded, r, token, scope, signal);
        if (!knowledgeCommandCurrent(token, scope, signal)) return;
        knowledgeCommand.projection = observation.projection; knowledgeCommand.observation = observation;
        if (observation.projection === "restricted") restrictKnowledgeCommandView();
        if (observation.projection === "observed") knowledgeSelectedEdge = r.edge_id;
        return;
      }
      if (r.operation === KNOWLEDGE_UNLINK) {
        const observation = await observeKnowledgeRemoval(loaded, r, token, scope, signal);
        if (!knowledgeCommandCurrent(token, scope, signal)) return;
        knowledgeCommand.projection = observation.projection; knowledgeCommand.observation = observation;
        if (observation.projection === "restricted") restrictKnowledgeCommandView();
        return;
      }
      const edge = loaded.relationships?.items.find(edge => edge.id === r.edge_id);
      knowledgeCommand.projection = edge ? "observed" : loaded.detailError ? "unavailable" : "not_on_page";
      if (edge) knowledgeSelectedEdge = edge.id;
    } catch (error) {
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      if (error.status === 401 || error.code === "command_context_changed") { commandAuthenticationLost(error.code === "command_context_changed"); return; }
      if (error.status === 403) { knowledgeCommand.result = null; knowledgeCommand.phase = "uncertain"; knowledgeCommand.error = "Current authority does not permit this read. The saved request identity is retained."; }
      knowledgeCommand.projection = "unavailable";
      state = { ...blankState(route), ...context, phase: "domain-error", error }; replaceRoute(route);
    }
  }
  async function refreshKnowledgeNodeGraph(token, scope, signal) {
    const r = knowledgeCommand.result.receipt, n = r.node;
    knowledgeCommand.observation = null;
    if (!r.details_visible) { restrictKnowledgeCommandView(); return; }
    if (n.activation_state !== "adopted") { knowledgeCommand.projection = "awaiting_activation"; return; }
    if (state.route.view !== "knowledge") { knowledgeCommand.projection = "unread"; return; }
    const retiring = r.operation === "dna.knowledge.node.retire";
    const id = retiring ? r.target.id : n.candidate_digest;
    if (state.route.id !== id) { knowledgeCommand.projection = "unread"; return; }
    const route = { ...firstPageRoute(state.route), id, target: "" };
    const context = { apps: state.apps, app: state.app, capabilities: state.capabilities, workingContext: state.workingContext };
    knowledgeCommand.projection = "reading"; destroyKnowledgeDraft();
    try {
      const loaded = await readApplication(route, generation, signal, identity => {
        const p = knowledgeCommand.metadata.principal;
        if (identity.app.id !== r.application_id || identity.capabilities.principal.mode !== p.mode || identity.capabilities.principal.name !== p.name) throw new ReadError(409, "command_context_changed", "The signed-in identity changed during Knowledge readback.");
      });
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      assert(BigInt(loaded.source.record_revision) > BigInt(r.sequence), "The graph observation precedes this recorded request.");
      state = { ...state, ...loaded, phase: "ready", inspectedAt: new Date(), error: null }; replaceRoute(loaded.route);
      // A graph row cannot borrow visibility or lifecycle from an older
      // command lookup. Reconfirm the receipt at this exact graph Record head.
      const confirmed = await knowledgeCommandRequest("GET", knowledgeCommand.metadata, null, signal);
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      sameKnowledgeAdmission(confirmed.receipt, r); knowledgeCommand.result = confirmed;
      if (!confirmed.receipt.details_visible) { restrictKnowledgeCommandView(); return; }
      if (confirmed.source.record_head !== loaded.source.record_head || confirmed.source.record_revision !== loaded.source.record_revision) { knowledgeCommand.projection = "changed"; return; }
      if (confirmed.receipt.node.activation_state !== "adopted") { knowledgeCommand.projection = "awaiting_activation"; return; }
      const exact = loaded.detail?.id === id ? loaded.detail : null;
      const retired = exact?.projection_state === "retired" && !exact.accepted;
      const ratified = exact?.accepted && exact.projection_state === "ratified";
      const historicallyRatified = retired && BigInt(exact.ratified_seq) > 0n;
      knowledgeCommand.projection = exact && (retiring ? retired : ratified || historicallyRatified) ? "node_observed" : "unavailable";
      if (exact) knowledgeCommand.observation = { id: exact.id, retired, ratified: ratified || historicallyRatified, snapshot: loaded.collection.page.snapshot };
    } catch (error) {
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      if (error.status === 401 || error.code === "command_context_changed") { commandAuthenticationLost(error.code === "command_context_changed"); return; }
      if (error.status === 403) { knowledgeCommand.result = null; knowledgeCommand.phase = "uncertain"; knowledgeCommand.error = "Current authority does not permit this read. The saved request identity is retained."; }
      knowledgeCommand.projection = "unavailable";
      state = { ...blankState(route), ...context, phase: "domain-error", error }; replaceRoute(route);
    }
  }
  async function refreshKnowledgeBindingGraph(token, scope, signal) {
    const r = knowledgeCommand.result.receipt, removing = r.operation === "dna.knowledge.binding.unbind";
    knowledgeCommand.observation = null;
    if (!r.details_visible) { restrictKnowledgeCommandView(); return; }
    if (r.binding.effect_state !== (removing ? "unbound" : "bound")) { knowledgeCommand.projection = "awaiting_effect"; return; }
    if (state.route.view !== "knowledge" || state.route.id !== r.target.id) { knowledgeCommand.projection = "unread"; return; }
    const route = firstPageRoute(state.route), context = { apps: state.apps, app: state.app, capabilities: state.capabilities, workingContext: state.workingContext };
    const pending = new AbortController(), abort = () => pending.abort();
    signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
    const timeout = setTimeout(abort, READ_TIMEOUT_MS);
    const current = () => { if (!knowledgeCommandCurrent(token, scope, signal) || pending.signal.aborted) throw new DOMException("Superseded binding observation", "AbortError"); };
    knowledgeCommand.projection = "reading"; destroyKnowledgeDraft();
    try {
      const loaded = await readApplication(route, generation, pending.signal, identity => {
        const p = knowledgeCommand.metadata.principal;
        if (identity.app.id !== r.application_id || identity.capabilities.principal.mode !== p.mode || identity.capabilities.principal.name !== p.name) throw new ReadError(409, "command_context_changed", "The signed-in identity changed during binding readback.");
      }); current();
      assert(BigInt(loaded.source.record_revision) > BigInt(r.sequence), "The graph observation precedes this binding request.");
      state = { ...state, ...loaded, phase: "ready", inspectedAt: new Date(), error: null }; replaceRoute(loaded.route);
      // The visible workspace may filter by working locus. Independently scan
      // this item's unfiltered bindings; keep the operator's route unchanged.
      const scanRoute = { ...route, target: "", snapshot: "" }, ids = new Set(), cursors = new Set();
      let cursor = "", source = null, basis = null, snapshot = "", pages = 0, projection = "unavailable";
      while (pages < KNOWLEDGE_REMOVAL_PAGES) {
        const query = new URLSearchParams({ id: r.target.id, limit: String(LIMIT) });
        if (snapshot) query.set("snapshot", snapshot); if (cursor) query.set("cursor", cursor);
        const response = await request(API + "/" + encodeURIComponent(r.application_id) + "/dna/knowledge/bindings?" + query, pending.signal); current();
        validSource(response.source, r.application_id); validKnowledge(response.data, response.source, { ...scanRoute, snapshot }, "bindings", cursor);
        if (!source) { source = response.source; basis = response.data.basis; snapshot = response.data.page.snapshot; }
        assert(source.record_head === response.source.record_head && source.record_revision === response.source.record_revision && sameKnowledgeBasis(basis, response.data.basis), "The binding sequence changed during observation.");
        pages++;
        if (basis.routing_version !== "0" || source.record_head !== loaded.source.record_head || source.record_revision !== loaded.source.record_revision) { projection = "changed"; break; }
        let match = null;
        for (const row of response.data.items) {
          assert(!ids.has(row.id), "The binding sequence repeated an identity."); ids.add(row.id);
          if (row.id === r.binding.binding_id) match = row;
        }
        if (match) {
          const intent = knowledgeCommand.metadata.binding;
          projection = removing ? "still_present" : match.idea_id === r.target.id && match.author === intent.author && match.target === intent.target ? "binding_observed" : "unavailable";
          break;
        }
        if (!response.data.page.has_more) { projection = removing ? "binding_removed" : "not_present"; break; }
        cursor = response.data.page.next_cursor;
        assert(!cursors.has(cursor), "The binding sequence repeated a cursor."); cursors.add(cursor);
        projection = "partial";
      }
      const confirmed = await knowledgeCommandRequest("GET", knowledgeCommand.metadata, null, pending.signal); current();
      sameKnowledgeAdmission(confirmed.receipt, r); knowledgeCommand.result = confirmed;
      if (!confirmed.receipt.details_visible) { restrictKnowledgeCommandView(); return; }
      if (!source || confirmed.source.record_head !== source.record_head || confirmed.source.record_revision !== source.record_revision) projection = "changed";
      else if (confirmed.receipt.binding.effect_state !== (removing ? "unbound" : "bound")) projection = "awaiting_effect";
      knowledgeCommand.projection = projection;
      if (["binding_observed", "binding_removed"].includes(projection)) knowledgeCommand.observation = { pages, snapshot, record_head: source.record_head };
    } catch (error) {
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      if (error.status === 401 || error.code === "command_context_changed") { commandAuthenticationLost(error.code === "command_context_changed"); return; }
      if (error.status === 403) { knowledgeCommand.result = null; knowledgeCommand.phase = "uncertain"; knowledgeCommand.error = "Current authority does not permit this read. The saved request identity is retained."; }
      knowledgeCommand.projection = "unavailable";
      state = { ...blankState(route), ...context, phase: "domain-error", error }; replaceRoute(route);
    } finally { clearTimeout(timeout); signal.removeEventListener("abort", abort); }
  }
  async function deliverKnowledgeCommand(method, metadata, payload = null) {
    if (knowledgeCommandController) return;
    const token = ++knowledgeCommandGeneration, scope = knowledgeCommand.scope;
    knowledgeCommandController = new AbortController(); const signal = knowledgeCommandController.signal;
    const previous = knowledgeCommand.result, focusCheck = document.activeElement?.id === "knowledge-check-status";
    knowledgeCommand.phase = method === "POST" ? "submitting" : "recovering"; knowledgeCommand.error = "";
    render();
    try {
      const result = await knowledgeCommandRequest(method, metadata, payload, signal);
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      if (previous) sameKnowledgeAdmission(result.receipt, previous.receipt);
      knowledgeCommand.result = result; knowledgeCommand.phase = "result";
      if (knowledgeBindingOperation(result.receipt.operation)) await refreshKnowledgeBindingGraph(token, scope, signal);
      else if (knowledgeNodeOperation(result.receipt.operation)) await refreshKnowledgeNodeGraph(token, scope, signal);
      else if (result.receipt.relationship || result.receipt.state === "recorded") await refreshKnowledgeCommandGraph(token, scope, signal);
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      ui.announcement.textContent = reviewedKnowledgeOperation(result.receipt.operation) || result.receipt.relationship ? "Knowledge request loaded. Proposal, Review, effect and graph observation are separate facts." : result.receipt.state === "recorded" ? "Relationship recorded. Graph observation is reported separately." : "Relationship outcome is unknown. Recover the saved request; it will not be submitted again.";
    } catch (error) {
      if (!knowledgeCommandCurrent(token, scope, signal)) return;
      if (error.status === 401 || error.code === "command_context_changed") { commandAuthenticationLost(error.code === "command_context_changed"); return; }
      const refused = method === "POST" && [400, 403, 409, 413].includes(error.status) && ["invalid_command", "forbidden", "stale_subject", "command_too_large"].includes(error.code);
      knowledgeCommand.phase = refused ? "refused" : "uncertain";
      if (!previous || error.status === 403) knowledgeCommand.result = null;
      if (error.status === 403 && state.route.view === "knowledge") {
        const route = firstPageRoute(state.route);
        knowledgeCommand.capability = null;
        destroyKnowledgeDraft();
        state = { ...blankState(route), apps: state.apps, app: state.app, capabilities: state.capabilities, workingContext: state.workingContext, phase: "domain-error", error: new ReadError(403, "knowledge_recovery_forbidden", "Current authority does not permit this request. Previously displayed Knowledge content has been cleared.") };
        replaceRoute(route);
      }
      const subject = reviewedKnowledgeOperation(knowledgeOperation(metadata)) ? "Knowledge" : "relationship";
      knowledgeCommand.error = refused ? "The service refused this " + subject + " request: " + error.message : error.status === 403 ? "Current authority does not permit recovery. The saved request identity is retained." : knowledgeCommand.result ? "The current lookup is unavailable. The previously confirmed admission remains recorded; no replacement request has been sent." : "The " + subject + " outcome could not be confirmed. Keep this request identity and check again; no replacement request has been sent.";
    } finally {
      payload = null;
      if (token === knowledgeCommandGeneration) {
        const focused = document.activeElement, focusId = focused?.id, focusKey = focused?.dataset.knowledgeControl;
        const otherFocus = focused !== document.body && !$("knowledge-command-recovery")?.contains(focused);
        knowledgeCommandController = null; render();
        if (focusId && $(focusId)) $(focusId).focus({ preventScroll: true });
        else if (focusKey) ui.content.querySelector('[data-knowledge-control="' + CSS.escape(focusKey) + '"]')?.focus({ preventScroll: true });
        if (document.activeElement === document.body) {
          if (method === "POST" && !otherFocus) $("knowledge-command-recovery")?.focus();
          else if (focusCheck && !otherFocus) $("knowledge-check-status")?.focus({ preventScroll: true });
        }
      }
    }
  }
  async function checkKnowledgeCommand() {
    if (!knowledgeCommand.metadata || knowledgeCommandController || knowledgeCommand.scope !== commandScope()) return;
    await deliverKnowledgeCommand("GET", knowledgeCommand.metadata);
  }
  async function submitKnowledgeCommand(draft, current) {
    if (!current() || !nativeKnowledgeOperation(draft.operation) || knowledgeCommand.metadata || knowledgeCommand.blocked || knowledgeCommandController) throw new Error("Check the saved Knowledge request before starting another change.");
    if (typeof navigator.locks?.request !== "function" || typeof crypto.randomUUID !== "function") throw new Error("This browser cannot safely save a recoverable request across tabs. Nothing was submitted.");
    const token = knowledgeCommandGeneration, scope = commandScope(), key = knowledgeRecoveryKey(scope);
    const valid = () => current() && token === knowledgeCommandGeneration && scope === commandScope() && !knowledgeCommand.metadata && !knowledgeCommand.blocked;
    let reservation;
    try {
      reservation = await navigator.locks.request(key, { mode: "exclusive", ifAvailable: true }, lock => {
        if (!lock) throw new Error("Another tab is reserving a Knowledge request. Nothing was submitted.");
        if (!valid()) return null;
        if (localStorage.getItem(key) !== null) return { existing: true };
        const isNode = knowledgeNodeOperation(nativeKnowledgeOperation(draft.operation)), isBinding = knowledgeBindingOperation(nativeKnowledgeOperation(draft.operation));
        const targetId = draft.operation === "node.propose" ? draft.arguments.target : draft.operation === "node.revise" ? draft.arguments.supersedes : draft.operation === "node.retire" ? draft.arguments.id : state.detail.id;
        const metadata = { version: isNode ? 3 : isBinding ? 4 : 2, operation: nativeKnowledgeOperation(draft.operation), ...(isNode ? { target_kind: draft.operation === "node.propose" ? "dna.knowledge.collection" : "dna.knowledge.node" } : {}), ...(isBinding ? { binding: { author: draft.arguments.author, target: draft.arguments.target } } : {}), application_id: draft.application_id, principal: { mode: draft.prepared_by.mode, name: draft.prepared_by.name }, request_id: crypto.randomUUID(), target_id: targetId, record_head: draft.base.basis.projection_record_head };
        assert(validKnowledgeRecovery(metadata));
        const a = draft.arguments;
        const arguments_ = draft.operation === "edge.unlink" ? { edge_id: a.id, from_id: a.from_id, to_id: a.to_id, rel: a.rel, rationale: a.rationale } : draft.operation === "binding.unbind" ? { binding_id: a.id, idea_id: a.idea_id, author: a.author, target: a.target, rationale: a.rationale } : { ...a };
        const payload = { request_id: metadata.request_id, operation: metadata.operation, operation_version: "1", context: { application_id: metadata.application_id, position_id: "org" }, target: { application_id: metadata.application_id, kind: knowledgeTargetKind(metadata), id: metadata.target_id }, preconditions: { principal: metadata.principal, record_head: metadata.record_head }, arguments: arguments_ };
        if (byteLength(JSON.stringify(payload)) > (isNode ? 98304 : 32768)) throw new Error("The encoded Knowledge request exceeds " + (isNode ? "96" : "32") + " KiB. Nothing was submitted.");
        const saved = JSON.stringify(metadata); localStorage.setItem(key, saved);
        if (localStorage.getItem(key) !== saved) throw new Error("The recovery identity could not be saved. Nothing was submitted.");
        return { metadata, payload };
      });
    } catch (error) { throw new Error(error.message || "Knowledge recovery storage is unavailable. Nothing was submitted."); }
    if (!valid() || !reservation) return;
    if (reservation.existing) {
      restoreKnowledgeCommand(); destroyKnowledgeDraft(); render();
      if (knowledgeCommand.metadata) void checkKnowledgeCommand();
      return;
    }
    knowledgeCommand.scope = scope; knowledgeCommand.metadata = reservation.metadata;
    destroyKnowledgeDraft();
    await deliverKnowledgeCommand("POST", reservation.metadata, reservation.payload);
  }
  async function dismissKnowledgeCommand() {
    const { metadata, result, phase, scope } = knowledgeCommand, token = knowledgeCommandGeneration;
    if (!metadata || (!knowledgeCompleted(result) && phase !== "refused") || knowledgeCommandController || typeof navigator.locks?.request !== "function") return;
    try {
      const removed = await navigator.locks.request(knowledgeRecoveryKey(scope), { mode: "exclusive", ifAvailable: true }, lock => {
        if (!lock || token !== knowledgeCommandGeneration || scope !== commandScope()) return false;
        const key = knowledgeRecoveryKey(scope), raw = localStorage.getItem(key);
        if (!raw || JSON.parse(raw).request_id !== metadata.request_id) throw new Error("Recovery identity changed.");
        localStorage.removeItem(key); if (localStorage.getItem(key) !== null) throw new Error("Recovery identity retained."); return true;
      });
      if (removed && token === knowledgeCommandGeneration && scope === commandScope()) refresh();
    } catch { knowledgeCommand.error = "The recovery identity could not be cleared. Keep this request before starting another."; render(); }
  }
  function renderKnowledgeCommandRecovery() {
    const { metadata, result, phase, error, blocked, projection } = knowledgeCommand;
    if ((!metadata && !blocked) || knowledgeCommand.scope !== commandScope()) return null;
    if (metadata && knowledgeBindingOperation(knowledgeOperation(metadata))) return renderKnowledgeBindingRecovery();
    if (metadata && knowledgeNodeOperation(knowledgeOperation(metadata))) return renderKnowledgeNodeRecovery();
    if (result?.receipt.relationship) return renderKnowledgeRelationshipRecovery();
    const panel = node("section", "panel practice-intervention intervention-recovery"); panel.id = "knowledge-command-recovery"; panel.tabIndex = -1; panel.setAttribute("aria-label", "Knowledge relationship request"); panel.setAttribute("role", "region");
    const r = result?.receipt, busy = Boolean(knowledgeCommandController), removing = metadata && knowledgeOperation(metadata) === KNOWLEDGE_UNLINK;
    panel.append(append(node("header", "panel-heading"), node("h2", "", removing ? "Relationship removal" : "Relationship request"), badge(r?.state === "recorded" ? "Recorded" : phase === "refused" ? "Refused" : "Unconfirmed", r?.state === "recorded" ? "positive" : "neutral")));
    const body = node("div", "intervention-body");
    const status = node("p", "", phase === "submitting" ? "Submitting the saved request once…" : phase === "recovering" ? "Reading the saved request…" : r?.state === "recorded" ? removing ? "The relationship removal is committed in the Record." : "The relationship effect is committed in the Record." : error || "The service has not confirmed whether this request is recorded."); status.setAttribute("role", "status"); body.append(status);
    if (r?.state === "recorded") {
      const observed = removing ? projection === "removed" : projection === "observed";
      const explanation = observed ? removing ? "The exact relationship is absent from all " + knowledgeCommand.observation.pages + " checked relationship pages at one fresh snapshot. Both endpoints remain visible under that same Record policy." : "The exact relationship appears in a fresh graph read." : projection === "restricted" ? "Relationship details are unavailable under current visibility." : projection === "still_present" ? "The exact relationship is present in the fresh graph. Its recorded removal remains part of history." : projection === "partial" ? "The bounded check did not reach the end of the relationship sequence. Removal observation is not established." : projection === "changed" ? "The Record or visibility changed during the check. Removal observation is not established." : projection === "not_on_page" ? "The exact relationship is not on the refreshed page. Page coverage does not establish absence." : projection === "reading" ? "Reading the current graph…" : "Graph readback is unavailable or has not been checked. The recorded effect remains committed.";
      const map = node("div", "command-outcome-map"), trail = node("ol", "intervention-stage-list"), inspector = node("div", "outcome-inspector");
      trail.setAttribute("aria-label", "Relationship outcome"); trail.style.gridTemplateColumns = "repeat(2, minmax(0, 1fr))";
      inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected relationship outcome");
      const stages = [
        { key: "record", title: "Record effect", value: removing ? "Removal recorded" : "Recorded", tone: "confirmed", explanation: removing ? "The native service committed removal of one exact directed relationship at the checked Record head. Reverse relationships, other labels and endpoint items are separate identities." : "The native service committed this directed relationship at the exact checked Record head. Its receipt remains durable independently of graph projection." },
        { key: "graph", title: "Graph observation", value: observed ? removing ? "Removal observed" : "Observed in graph" : "Not established", tone: observed ? "confirmed" : "unknown", explanation }
      ];
      const choose = key => {
        knowledgeCommand.outcomeStage = key;
        for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === key));
        const stage = stages.find(stage => stage.key === key); inspector.dataset.state = stage.tone;
        inspector.replaceChildren(node("h4", "", stage.title), node("p", "outcome-value", stage.value), node("p", "", stage.explanation));
      };
      for (const [index, stage] of stages.entries()) {
        const control = button("", () => choose(stage.key), "outcome-stage"); control.id = "knowledge-outcome-" + stage.key;
        control.dataset.stage = stage.key; control.dataset.state = stage.tone; control.setAttribute("aria-label", stage.title);
        const marker = node("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true");
        control.append(marker, node("strong", "", stage.title), node("span", "outcome-stage-value", stage.value)); trail.append(append(node("li"), control));
      }
      choose(knowledgeCommand.outcomeStage || "graph"); map.append(trail, inspector);
      body.append(map);
      if (r.details_visible) body.append(navigationLink("Open relationship graph", routeHash(workspaceRoute("knowledge", { app: metadata.application_id, id: r.target.id })), "graph"));
    }
    if (error && r) body.append(node("p", "intervention-error", error));
    if (metadata) {
      const actions = node("div", "intervention-actions"), check = button("Check relationship request", checkKnowledgeCommand); check.id = "knowledge-check-status"; check.disabled = busy; actions.append(check);
      if (r?.state === "recorded" || phase === "refused") { const dismiss = button("Dismiss relationship request", dismissKnowledgeCommand); dismiss.disabled = busy; actions.append(dismiss); }
      const facts = node("dl", "fact-grid"); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Principal", metadata.principal.mode + " · " + metadata.principal.name);
      fact(facts, "Operation", knowledgeOperation(metadata), true, true);
      if (r) { fact(facts, "Command identity", r.command_id, true, true); fact(facts, "Receipt Record head", result.source.record_head, true, true); if (r.details_visible) fact(facts, "Relationship identity", r.edge_id, true, true); }
      if (projection === "removed") { fact(facts, "Checked relationship pages", knowledgeCommand.observation.pages); fact(facts, "Observation snapshot", knowledgeCommand.observation.snapshot, true, true); }
      body.append(actions, append(node("details", "command-evidence"), node("summary", "", "Exact relationship request evidence"), facts));
    }
    return append(panel, body);
  }
  function renderKnowledgeRelationshipRecovery() {
    const { metadata, result, phase, error, projection, observation } = knowledgeCommand;
    const r = result?.receipt, b = r?.relationship, removing = knowledgeOperation(metadata) === KNOWLEDGE_UNLINK, busy = Boolean(knowledgeCommandController);
    const panel = node("section", "panel practice-intervention intervention-recovery"); panel.id = "knowledge-command-recovery"; panel.tabIndex = -1; panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Knowledge relationship request");
    panel.append(append(node("header", "panel-heading"), node("h2", "", removing ? "Relationship removal proposal" : "Relationship proposal"), badge(b?.proposal_state === "created" ? "Proposal created" : b?.proposal_state === "refused" || phase === "refused" ? "Refused" : "Awaiting outcome", "neutral")));
    const body = node("div", "intervention-body");
    const status = node("p", "", phase === "submitting" ? "Submitting the saved relationship request once…" : phase === "recovering" ? "Reading the saved relationship request…" : b?.proposal_state === "created" ? "The relationship candidate and required Review were created. Approval, the relationship effect and graph observation are separate facts." : error || "Relationship proposal creation is not established. Recover the original request; it will not be submitted again."); status.setAttribute("role", "status"); body.append(status);
    if (r) {
      const observed = ["observed", "removed"].includes(projection);
      const stages = [
        { key: "proposal", title: "Proposal", value: b.proposal_state, confirmed: b.proposal_state === "created", text: b.reason || "The owning service creates an exact directed relationship candidate. Both endpoint items remain unchanged; the tuple identity alone does not establish an applied effect." },
        { key: "review", title: "Review", value: b.review_state + (b.review_outcome ? " · " + b.review_outcome : ""), confirmed: b.review_state === "settled", text: "Inspect the canonical relationship action, exact directed tuple, proposer and rationale in its required Review." },
        { key: "effect", title: "Relationship effect", value: b.effect_state, confirmed: ["linked", "unlinked"].includes(b.effect_state), text: b.effect_reason || "The native domain applies or refuses the approved change. Approval alone does not establish a relationship effect." },
        { key: "graph", title: "Graph observation", value: observed ? removing ? "Removal observed" : "Observed in graph" : "Not established", confirmed: observed, text: projection === "restricted" ? "Relationship details are unavailable under current visibility." : projection === "removed" ? "The complete unfiltered relationship sequence no longer contains this exact native relationship identity. The final receipt remains visible at the same Record head." : projection === "observed" ? "A fresh unfiltered graph read contains the exact native relationship identity and canonical ordered endpoints and relation label." : projection === "partial" ? "The bounded read did not complete the relationship sequence. Absence is not established." : projection === "still_present" ? "The exact relationship is still present in the fresh graph. The recorded removal remains separate from its current projection." : "Graph observation is unavailable or not established at one fresh source. Recorded proposal, Review and effect remain intact." }
      ];
      const map = node("div", "command-outcome-map"), trail = node("ol", "intervention-stage-list"), inspector = node("div", "outcome-inspector"); trail.setAttribute("aria-label", "Relationship change outcome"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected relationship outcome");
      const choose = key => {
        const stage = stages.find(value => value.key === key) || stages[0]; knowledgeCommand.outcomeStage = stage.key;
        for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === stage.key));
        inspector.dataset.state = stage.confirmed ? "confirmed" : "unknown"; inspector.replaceChildren(node("h4", "", stage.title), node("p", "outcome-value", stage.value), node("p", "", stage.text));
      };
      for (const [index, stage] of stages.entries()) {
        const control = button("", () => choose(stage.key), "outcome-stage"); control.id = "knowledge-outcome-" + stage.key; control.dataset.stage = stage.key; control.dataset.state = stage.confirmed ? "confirmed" : "unknown"; control.setAttribute("aria-label", stage.title);
        const marker = node("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true"); control.append(marker, node("strong", "", stage.title), node("span", "outcome-stage-value", stage.value)); trail.append(append(node("li"), control));
      }
      choose(knowledgeCommand.outcomeStage || (observed ? "graph" : b.proposal_state !== "created" ? "proposal" : b.review_state === "pending" ? "review" : "effect")); map.append(trail, inspector); body.append(map);
      if (r.details_visible) {
        const tuple = node("dl", "fact-grid"); fact(tuple, "Focused knowledge item", r.target.id, true, true); fact(tuple, "Exact relationship identity", r.edge_id, true, true); body.append(tuple);
        const links = node("div", "intervention-actions");
        if (b.review_id) links.append(navigationLink("Open exact relationship Review", routeHash(workspaceRoute("reviews", { app: metadata.application_id, id: b.review_id })), "detail"));
        links.append(navigationLink("Open relationship graph", routeHash(workspaceRoute("knowledge", { app: metadata.application_id, id: r.target.id })), "graph")); body.append(links);
      }
      if (b.reason) body.append(node("p", "detail-note", b.reason)); if (b.effect_reason) body.append(node("p", "detail-note", "Relationship effect · " + b.effect_reason)); if (error) body.append(node("p", "intervention-error", error));
    }
    const actions = node("div", "intervention-actions"), check = button("Check relationship request", checkKnowledgeCommand); check.id = "knowledge-check-status"; check.disabled = busy; actions.append(check);
    if (knowledgeCompleted(result) || phase === "refused") { const dismiss = button("Dismiss relationship request", dismissKnowledgeCommand); dismiss.disabled = busy; actions.append(dismiss); }
    body.append(actions);
    const facts = node("dl", "fact-grid"); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Operation", knowledgeOperation(metadata), true, true); fact(facts, "Principal", metadata.principal.mode + " · " + metadata.principal.name);
    if (r) { fact(facts, "Command identity", r.command_id, true, true); fact(facts, "Receipt Record head", result.source.record_head, true, true); if (r.details_visible) fact(facts, "Exact relationship identity", r.edge_id, true, true); }
    if (observation) { fact(facts, "Checked relationship pages", observation.pages); fact(facts, "Observation snapshot", observation.snapshot, true, true); }
    body.append(append(node("details", "command-evidence"), node("summary", "", "Exact relationship request evidence"), facts)); return append(panel, body);
  }
  function renderKnowledgeBindingRecovery() {
    const { metadata, result, phase, error, projection, observation } = knowledgeCommand;
    const r = result?.receipt, b = r?.binding, removing = knowledgeOperation(metadata) === "dna.knowledge.binding.unbind", busy = Boolean(knowledgeCommandController);
    const panel = node("section", "panel practice-intervention intervention-recovery"); panel.id = "knowledge-command-recovery"; panel.tabIndex = -1; panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Knowledge binding request");
    panel.append(append(node("header", "panel-heading"), node("h2", "", removing ? "Remove locus binding" : "Add locus binding"), badge(b?.proposal_state === "created" ? "Proposal created" : b?.proposal_state === "refused" || phase === "refused" ? "Refused" : "Awaiting outcome", "neutral")));
    const body = node("div", "intervention-body");
    const status = node("p", "", phase === "submitting" ? "Submitting the saved binding request once…" : phase === "recovering" ? "Reading the saved binding request…" : b?.proposal_state === "created" ? "The binding candidate and required Review were created. Approval, the binding effect and graph observation are separate facts." : error || "Binding proposal creation is not established. Recover the original request; it will not be submitted again."); status.setAttribute("role", "status"); body.append(status);
    if (r) {
      const observed = ["binding_observed", "binding_removed"].includes(projection);
      const stages = [
        { key: "proposal", title: "Proposal", value: b.proposal_state, confirmed: b.proposal_state === "created", text: b.reason || "The owning service creates an exact binding-change candidate. The original Knowledge item remains unchanged." },
        { key: "review", title: "Review", value: b.review_state + (b.review_outcome ? " · " + b.review_outcome : ""), confirmed: b.review_state === "settled", text: "Inspect the canonical binding action, exact tuple, proposer and rationale in its required Review." },
        { key: "effect", title: "Binding effect", value: b.effect_state, confirmed: ["bound", "unbound"].includes(b.effect_state), text: b.effect_reason || "The native domain applies or refuses the approved change. Approval alone does not establish a binding effect." },
        { key: "graph", title: "Graph observation", value: observed ? removing ? "Binding removal observed" : "Binding observed" : "Not established", confirmed: observed, text: projection === "restricted" ? "Binding details are unavailable under current visibility." : projection === "binding_removed" ? "The complete unfiltered binding sequence for this item no longer contains this exact native binding identity. The final receipt remains visible at the same Record head." : projection === "binding_observed" ? "A fresh unfiltered graph read contains the exact native binding identity and requested item, author and target." : projection === "partial" ? "The bounded read did not complete the binding sequence. Absence is not established." : projection === "still_present" ? "The exact binding is still present in the fresh graph. The recorded removal remains separate from its current projection." : "Graph observation is unavailable or not established at one fresh source. Recorded proposal, Review and effect remain intact." }
      ];
      const map = node("div", "command-outcome-map"), trail = node("ol", "intervention-stage-list"), inspector = node("div", "outcome-inspector"); trail.setAttribute("aria-label", "Binding change outcome"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected binding outcome");
      const choose = key => {
        const stage = stages.find(value => value.key === key) || stages[0]; knowledgeCommand.outcomeStage = stage.key;
        for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === stage.key));
        inspector.dataset.state = stage.confirmed ? "confirmed" : "unknown"; inspector.replaceChildren(node("h4", "", stage.title), node("p", "outcome-value", stage.value), node("p", "", stage.text));
      };
      for (const [index, stage] of stages.entries()) {
        const control = button("", () => choose(stage.key), "outcome-stage"); control.id = "knowledge-outcome-" + stage.key; control.dataset.stage = stage.key; control.dataset.state = stage.confirmed ? "confirmed" : "unknown"; control.setAttribute("aria-label", stage.title);
        const marker = node("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true"); control.append(marker, node("strong", "", stage.title), node("span", "outcome-stage-value", stage.value)); trail.append(append(node("li"), control));
      }
      choose(knowledgeCommand.outcomeStage || (observed ? "graph" : b.proposal_state !== "created" ? "proposal" : b.review_state === "pending" ? "review" : "effect")); map.append(trail, inspector); body.append(map);
      if (r.details_visible) {
        const tuple = node("dl", "fact-grid"); fact(tuple, "Knowledge item", r.target.id, true, true); fact(tuple, "Requested authoring locus", metadata.binding.author); fact(tuple, "Requested target locus", metadata.binding.target); body.append(tuple);
        const links = node("div", "intervention-actions");
        if (b.review_id) links.append(navigationLink("Open exact binding Review", routeHash(workspaceRoute("reviews", { app: metadata.application_id, id: b.review_id })), "detail"));
        links.append(navigationLink("Open knowledge applicability", routeHash(workspaceRoute("knowledge", { app: metadata.application_id, id: r.target.id })), "graph")); body.append(links);
      }
      if (b.reason) body.append(node("p", "detail-note", b.reason)); if (b.effect_reason) body.append(node("p", "detail-note", "Binding effect · " + b.effect_reason)); if (error) body.append(node("p", "intervention-error", error));
    }
    const actions = node("div", "intervention-actions"), check = button("Check binding request", checkKnowledgeCommand); check.id = "knowledge-check-status"; check.disabled = busy; actions.append(check);
    if (knowledgeCompleted(result) || phase === "refused") { const dismiss = button("Dismiss binding request", dismissKnowledgeCommand); dismiss.disabled = busy; actions.append(dismiss); }
    body.append(actions);
    const facts = node("dl", "fact-grid"); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Operation", knowledgeOperation(metadata), true, true); fact(facts, "Principal", metadata.principal.mode + " · " + metadata.principal.name);
    if (r) { fact(facts, "Command identity", r.command_id, true, true); fact(facts, "Receipt Record head", result.source.record_head, true, true); if (r.details_visible) fact(facts, "Exact binding identity", b.binding_id, true, true); }
    if (observation) { fact(facts, "Checked binding pages", observation.pages); fact(facts, "Observation snapshot", observation.snapshot, true, true); }
    body.append(append(node("details", "command-evidence"), node("summary", "", "Exact binding request evidence"), facts)); return append(panel, body);
  }
  function renderKnowledgeNodeRecovery() {
    const { metadata, result, phase, error, projection, observation } = knowledgeCommand;
    const r = result?.receipt, n = r?.node, operation = knowledgeOperation(metadata);
    const retiring = operation === "dna.knowledge.node.retire", revising = operation === "dna.knowledge.node.revise";
    const panel = node("section", "panel practice-intervention intervention-recovery");
    panel.id = "knowledge-command-recovery"; panel.tabIndex = -1; panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Knowledge change request");
    const title = retiring ? "Knowledge retirement" : revising ? "Knowledge revision" : "Knowledge creation";
    panel.append(append(node("header", "panel-heading"), node("h2", "", title), badge(n?.proposal_state === "created" ? "Proposal created" : n?.proposal_state === "refused" || phase === "refused" ? "Refused" : "Awaiting outcome", n?.proposal_state === "created" ? "positive" : "neutral")));
    const body = node("div", "intervention-body");
    const status = node("p", "", phase === "submitting" ? "Submitting the saved Knowledge request once…" : phase === "recovering" ? "Reading the saved Knowledge request…" : n?.proposal_state === "created" ? "The proposal and its required Review were created. Approval and activation are reported separately." : n?.proposal_state === "refused" ? "The service refused this Knowledge proposal." : error || "The service has not established proposal creation. Check the original request; it will not be submitted again.");
    status.setAttribute("role", "status"); body.append(status);
    if (r) {
      const observed = projection === "node_observed";
      const graphExplanation = projection === "restricted" ? "Knowledge change details are unavailable under current visibility." : observed ? retiring ? "A fresh graph read confirms this exact item is retired and retained in history." : observation?.retired ? "A fresh graph read confirms the adopted receipt is retained in history and is now retired." : "A fresh graph read confirms the exact adopted Knowledge item." : projection === "awaiting_activation" ? "Graph activation is not established. A created proposal or approved Review alone does not establish adoption." : "Graph readback is unavailable or has not been checked. Recorded proposal, Review and activation facts remain separate.";
      const stages = [
        { key: "proposal", title: "Proposal", value: n.proposal_state, confirmed: n.proposal_state === "created", text: n.reason || "The owning service reports whether it created the canonical candidate and required Review." },
        { key: "review", title: "Review", value: n.review_state + (n.review_outcome ? " · " + n.review_outcome : ""), confirmed: n.review_state === "settled", text: "This is the Review of the exact canonical candidate. Open that Review to inspect and decide through its existing controls." },
        { key: "activation", title: "Activation", value: retiring && n.activation_state === "adopted" ? "retired" : n.activation_state, confirmed: n.activation_state === "adopted", text: n.activation_reason || (retiring ? "The native domain decides whether to retire the exact target while preserving its history." : "The native domain decides whether the approved candidate can become active. Approval can coexist with adoption refusal.") },
        { key: "graph", title: "Graph observation", value: observed ? retiring ? "Retirement observed" : "Adoption observed" : "Not established", confirmed: observed, text: graphExplanation }
      ];
      const map = node("div", "command-outcome-map"), trail = node("ol", "intervention-stage-list"), inspector = node("div", "outcome-inspector");
      trail.setAttribute("aria-label", "Knowledge change outcome"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected Knowledge outcome");
      const choose = key => {
        const stage = stages.find(value => value.key === key) || stages[0]; knowledgeCommand.outcomeStage = stage.key;
        for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === stage.key));
        inspector.dataset.state = stage.confirmed ? "confirmed" : "unknown";
        inspector.replaceChildren(node("h4", "", stage.title), node("p", "outcome-value", stage.value), node("p", "", stage.text));
      };
      for (const [index, stage] of stages.entries()) {
        const control = button("", () => choose(stage.key), "outcome-stage"); control.id = "knowledge-outcome-" + stage.key; control.dataset.stage = stage.key; control.dataset.state = stage.confirmed ? "confirmed" : "unknown"; control.setAttribute("aria-label", stage.title);
        const marker = node("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true");
        control.append(marker, node("strong", "", stage.title), node("span", "outcome-stage-value", stage.value)); trail.append(append(node("li"), control));
      }
      choose(knowledgeCommand.outcomeStage || (observed ? "graph" : n.proposal_state !== "created" ? "proposal" : n.review_state === "pending" ? "review" : "activation")); map.append(trail, inspector); body.append(map);
      if (r.details_visible) {
        const links = node("div", "intervention-actions"), route = (view, id) => routeHash(workspaceRoute(view, { app: metadata.application_id, id }));
        if (n.review_id) links.append(navigationLink("Open exact Review", route("reviews", n.review_id), "detail"));
        if (n.candidate_digest && !retiring) links.append(navigationLink("Open candidate knowledge", route("knowledge", n.candidate_digest), "graph"));
        if (state.route.practice_action && n.candidate_digest && !retiring) links.append(navigationLink("Open candidate in Practices", route("practices", n.candidate_digest), "detail"));
        if (revising || retiring) links.append(navigationLink(retiring ? "Open knowledge history" : "Open prior knowledge", route("knowledge", metadata.target_id), "graph"));
        if (state.route.practice_action && (revising || retiring)) links.append(navigationLink("Open prior Practice", route("practices", metadata.target_id), "detail"));
        body.append(links);
      }
      if (n.reason) body.append(node("p", "detail-note", n.reason));
      if (n.activation_reason) body.append(node("p", "detail-note", "Activation · " + n.activation_reason));
      if (error) body.append(node("p", "intervention-error", error));
    }
    const busy = Boolean(knowledgeCommandController), check = button("Check knowledge request", checkKnowledgeCommand); check.id = "knowledge-check-status"; check.disabled = busy;
    const actions = append(node("div", "intervention-actions"), check);
    if (knowledgeCompleted(result) || phase === "refused") { const dismiss = button("Dismiss knowledge request", dismissKnowledgeCommand); dismiss.disabled = busy; actions.append(dismiss); }
    const facts = node("dl", "fact-grid"); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Operation", operation, true, true); fact(facts, "Principal", metadata.principal.mode + " · " + metadata.principal.name);
    if (r) { fact(facts, "Command identity", r.command_id, true, true); fact(facts, "Receipt Record head", result.source.record_head, true, true); if (r.details_visible && n.candidate_digest) fact(facts, "Canonical candidate", n.candidate_digest, true, true); if (r.details_visible && n.review_id) fact(facts, "Exact Review", n.review_id, true, true); }
    body.append(actions, append(node("details", "command-evidence"), node("summary", "", "Exact Knowledge request evidence"), facts));
    return append(panel, body);
  }
  function blankState(route) {
    return { route, phase: "loading", apps: [], app: null, capabilities: null, workingContext: null, source: null, collection: null, detail: null, detailError: null, organizationBranch: null, organizationBranchError: null, reviewCandidate: null, reviewCandidateError: "", organizationStatus: null, organizationStatusError: "", organizationImpact: null, organizationImpactError: "", practiceContext: null, person: null, personError: "", relationships: null, bindings: null, error: null, notice: "", inspectedAt: null, head: headState };
  }
  function node(tag, className, text) {
    const el = document.createElement(tag);
    if (className) el.className = className;
    if (text !== undefined && text !== null) el.textContent = String(text);
    return el;
  }
  function append(parent, ...children) {
    children.filter(Boolean).forEach((child) => parent.append(child));
    return parent;
  }
  function button(label, action, className = "button secondary") {
    const el = node("button", className, label);
    el.type = "button";
    el.addEventListener("click", action);
    return el;
  }
  function link(label, href, className = "text-link") {
    const el = node("a", className, label);
    el.href = href;
    return el;
  }
  function navigationLink(label, href, target, className = "text-link") {
    const el = link(label, href, className);
    el.addEventListener("click", (event) => {
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      if (location.hash === href) {
        event.preventDefault();
        focusPanel(target);
      } else navigationFocus = target;
    });
    return el;
  }
  function focusPanel(target) {
    if (target === "context") { $("working-locus")?.focus({ preventScroll: true }); return; }
    if (["execution", "execution-map", "attempt"].includes(target)) {
      const selected = $(target === "execution-map" ? "execution-steps" : target === "attempt" ? "execution-attempt-panel" : "execution-focus-panel") || $("record-detail-panel");
      selected?.focus({ preventScroll: true });
      selected?.scrollIntoView({ block: "nearest", behavior: "auto" });
      return;
    }
    const panel = $(target === "graph" ? "relationship-map-panel" : target === "detail" ? "record-detail-panel" : "record-list-panel");
    if (!panel) return;
    panel.focus({ preventScroll: true });
    panel.scrollIntoView({ block: "start", behavior: "auto" });
  }
  function backToList() {
    if (state.route.practice_action) return navigationLink("Back to Practices", routeHash(workspaceRoute("practices")), "list", "text-link detail-back");
    return navigationLink(WORKSPACES[state.route.view].back, routeHash({ ...state.route, id: "", offset: state.collection.page.offset, edges_cursor: "", bindings_cursor: "", snapshot: state.collection.page.snapshot }), "list", "text-link detail-back");
  }
  function short(value, width = 17) {
    return value.length > width ? value.slice(0, width) + "…" : value;
  }
  function display(value, fallback = "Not recorded") {
    return typeof value === "string" && value.length ? value : fallback;
  }
  function readRoute() {
    const raw = location.hash.slice(1);
    const cut = raw.indexOf("?");
    const view = (cut < 0 ? raw : raw.slice(0, cut)).replace(/^\//, "");
    const query = new URLSearchParams(cut < 0 ? "" : raw.slice(cut + 1));
    const offsetRaw = query.get("offset") || "0";
    const offset = /^(0|[1-9]\d{0,6})$/.test(offsetRaw) && Number(offsetRaw) <= 1000000 ? Number(offsetRaw) : 0;
    const selected = APPLICATION_HOST ? "application" : VIEWS.has(view) ? view : "practices";
    const practiceAction = selected === "knowledge" && ["create", "revise", "retire", "applicability"].includes(query.get("practice_action")) ? query.get("practice_action") : "";
    return { view: selected, app: query.get("app") || "", assignee: selected === "tasks" ? query.get("assignee") || "" : "", assignee_invalid: selected === "tasks" && query.has("assignee") && (query.getAll("assignee").length !== 1 || !commandID(query.get("assignee"))), practice_action: practiceAction, locus: query.get("locus") || "", branch: selected === "organization" ? query.get("branch") || "" : "", worknode: query.get("node") || "", workattempt: selected === "workflows" ? query.get("attempt") || "" : "", id: query.get("id") || "", offset, snapshot: query.get("snapshot") || "", scope: query.get("scope") === "all" ? "all" : "positions", cursor: query.get("cursor") || "", target: query.get("target") || "", edges_cursor: query.get("edges_cursor") || "", bindings_cursor: query.get("bindings_cursor") || "" };
  }
  function routeHash(route) {
    if (route.view === "application") return "#/application";
    if (route.view === "projects") return "#/projects";
    const query = new URLSearchParams();
    if (route.app) query.set("app", route.app);
    if (route.locus) query.set("locus", route.locus);
    if (route.id) query.set("id", route.id);
    if (route.view === "tasks" && route.assignee) query.set("assignee", route.assignee);
    if (route.view === "workflows" && route.id && route.worknode) query.set("node", route.worknode);
    if (route.view === "workflows" && route.id && route.worknode && route.workattempt) query.set("attempt", route.workattempt);
    if (route.view === "knowledge") {
      if (["create", "revise", "retire", "applicability"].includes(route.practice_action)) query.set("practice_action", route.practice_action);
      for (const key of ["cursor", "target"]) if (route[key]) query.set(key, route[key]);
      if (route.id) for (const key of ["edges_cursor", "bindings_cursor"]) if (route[key]) query.set(key, route[key]);
    } else if (route.offset) query.set("offset", String(route.offset));
    if (route.snapshot) query.set("snapshot", route.snapshot);
    if (route.view === "organization" && route.scope === "all") query.set("scope", "all");
    if (route.view === "organization" && route.branch) query.set("branch", route.branch);
    return "#/" + route.view + (query.size ? "?" + query.toString() : "");
  }
  function workspaceRoute(view, extra = {}) {
    const app = state.app?.id || state.route.app;
    const locus = state.route.locus || "";
    return { view, app, locus, target: view === "knowledge" ? locus : "", id: "", offset: 0, snapshot: "", ...extra };
  }
  function practiceAdministrationRoute(action, practice = null) {
    return routeHash(workspaceRoute("knowledge", { id: practice?.id || "", target: "", practice_action: action }));
  }
  function changeWorkingContext(locus) {
    navigationFocus = "context";
    if (state.route.practice_action) { navigate(workspaceRoute("practices", { locus })); return; }
    navigate({ ...firstPageRoute(state.route), locus, id: "", branch: "", target: state.route.view === "knowledge" ? locus : "" });
  }
  function replaceRoute(route) {
    history.replaceState(null, "", routeHash(route));
    state.route = route;
  }
  function navigate(route) {
    const hash = routeHash(route);
    if (location.hash === hash) {
      const focusTarget = navigationFocus;
      navigationFocus = null;
      loadRoute(route, "", focusTarget);
    }
    else location.hash = hash;
  }
  function relatedRoute(view, id) {
    if (view === "knowledge" && state.route.view === view) return routeHash({ ...state.route, id, practice_action: id === state.route.id ? state.route.practice_action : "", edges_cursor: "", bindings_cursor: "", snapshot: state.collection.page.snapshot });
    const offset = ["organization", "definitions"].includes(view) && state.route.view === view ? state.collection.page.offset : 0;
    return routeHash(workspaceRoute(view, { id, offset, snapshot: state.collection.page.snapshot, scope: state.route.scope, branch: view === "organization" && state.route.view === view ? state.route.branch : "" }));
  }
  function assert(condition, message = "The service returned an unsupported or incomplete response.") {
    if (!condition) throw new ReadError(0, "invalid_response", message);
  }
  class ReadError extends Error {
    constructor(status, code, message) { super(message); this.status = status; this.code = code; }
  }
  function validSource(source, app = "") {
    assert(source && typeof source.record_id === "string" && source.record_id.length && typeof source.record_head === "string" && source.record_head.length && typeof source.record_revision === "string" && /^(0|[1-9]\d*)$/.test(source.record_revision));
    assert(!app || source.record_id === app, "The service returned a different Record identity. No records are displayed.");
  }
  function validPage(data, source, workspace = null) {
    assert(data && Array.isArray(data.items) && data.page);
    const page = data.page;
    assert(Number.isSafeInteger(page.limit) && page.limit >= 1 && page.limit <= 100 && Number.isSafeInteger(page.offset) && page.offset >= 0 && page.offset <= 1000000 && Number.isSafeInteger(page.total) && page.total >= 0 && Number.isSafeInteger(page.next_offset) && page.next_offset >= -1 && page.next_offset <= 1000000 && typeof page.snapshot === "string" && page.snapshot.length);
    if (!workspace?.sourceBound) assert(page.snapshot === source.record_head);
    assert(page.next_offset === -1 || page.next_offset > page.offset, "The service returned an invalid page continuation.");
  }
  function validRows(items, view) {
    const workspace = WORKSPACES[view];
    const ids = new Set();
    for (const item of items) {
      assert(item && workspace.fields.every((key) => typeof item[key] === "string") && item.id.length && workspace.booleans.every((key) => typeof item[key] === "boolean"));
      assert(!ids.has(item.id), "The service returned duplicate object identities.");
      ids.add(item.id);
      if (view === "practices") assert(item.digest.length);
      if (view === "definitions") validDefinition(item);
      if (view === "workflows") validWorkflow(item);
      if (view === "tasks") {
        assert(window.FaceTaskAdministration, "Task administration is not loaded.");
        try { window.FaceTaskAdministration.validate(item); } catch { assert(false, "The service returned an invalid Task responsibility."); }
      }
      if (view === "knowledge") assert(receiptID(item.id) && decimal(item.revision, true) && decimal(item.ratified_seq, true) && item.source_provenance === null);
      if (view === "reviews" && item.knowledge_binding_digest !== undefined) assert(typeof item.knowledge_binding_digest === "string" && (!item.knowledge_binding_digest || receiptID(item.knowledge_binding_digest) && item.knowledge_binding_digest === item.subject_digest && item.knowledge_digest === ""));
      if (view === "reviews" && item.knowledge_edge_digest !== undefined) assert(typeof item.knowledge_edge_digest === "string" && (!item.knowledge_edge_digest || receiptID(item.knowledge_edge_digest) && item.knowledge_edge_digest === item.subject_digest && item.knowledge_digest === "" && !item.knowledge_binding_digest));
      if (view === "reviews" && item.organization_source !== undefined) {
        assert(item.organization_source === true && item.is_mutation === true && item.knowledge_digest === "" && !item.knowledge_binding_digest && !item.knowledge_edge_digest && ["organization_source_request_id", "organization_source_digest", "author"].every(key => typeof item[key] === "string"));
        if (item.text_available) assert(commandID(item.organization_source_request_id) && receiptID(item.organization_source_digest) && commandID(item.author));
        else assert(item.organization_source_request_id === "" && item.organization_source_digest === "" && item.author === "");
      }
      if (view === "organization") {
        assert(["position", "structure"].includes(item.role));
        assert(Array.isArray(item.parameters) && item.parameters.every((p) => p && typeof p.name === "string" && typeof p.type === "string"));
        assert(stringArray(item.methods) && stringArray(item.publishes));
        assert(Array.isArray(item.subscribes) && item.subscribes.every((s) => s && typeof s.topic === "string" && typeof s.handler === "string" && typeof s.shed === "string" && nullableDecimal(s.capacity)));
        assert(Array.isArray(item.supervises) && item.supervises.every((s) => s && typeof s.child === "string" && typeof s.error === "string" && stringArray(s.ops) && nullableDecimal(s.retry)));
      }
    }
  }
  function stringArray(value) { return Array.isArray(value) && value.every((item) => typeof item === "string"); }
  function nullableDecimal(value) { return value === null || (typeof value === "string" && value === value.trim() && /^(0|[1-9][0-9]*)$/.test(value)); }
  // Preserve native Int64 values as text, including revisions beyond 2^53.
  function decimal(value, signed = false) {
    if (typeof value !== "string" || !(signed ? /^(0|-?[1-9][0-9]*)$/ : /^(0|[1-9][0-9]*)$/).test(value)) return false;
    const digits = value.startsWith("-") ? value.slice(1) : value;
    const ceiling = value.startsWith("-") ? "9223372036854775808" : "9223372036854775807";
    return digits.length < ceiling.length || (digits.length === ceiling.length && digits <= ceiling);
  }
  function validDefinitionReference(value) {
    assert(value && typeof value.definition_id === "string" && /^[a-z0-9][a-z0-9-]*$/.test(value.definition_id) && decimal(value.revision, true) && value.id === value.definition_id + "@" + value.revision, "The service returned an inconsistent definition identity or revision.");
  }
  function validDefinition(item) {
    validDefinitionReference(item);
    assert(Array.isArray(item.steps) && item.steps.length && Array.isArray(item.dependents));
    item.steps.forEach((step, index) => {
      assert(step && step.index === String(index) && typeof step.store === "string" && Array.isArray(step.members) && step.members.length, "The service returned incomplete or unordered definition Steps.");
      const keys = new Set();
      for (const member of step.members) {
        assert(member && typeof member.key === "string" && /^[a-z0-9-]+$/.test(member.key) && !keys.has(member.key));
        keys.add(member.key);
        if (member.kind === "leaf") {
          const leaf = member.leaf;
          assert(member.child === null && leaf && ["objective", "context_digest", "knowledge_bindings", "output_contract", "data_class", "requires", "target"].every((key) => typeof leaf[key] === "string") && decimal(leaf.cost_ceiling, true) && decimal(leaf.attempts) && leaf.attempts !== "0");
        } else {
          assert(member.kind === "child" && member.leaf === null);
          validDefinitionReference(member.child);
        }
      }
    });
    for (const dependent of item.dependents) {
      validDefinitionReference(dependent);
      assert(decimal(dependent.step) && typeof dependent.key === "string" && /^[a-z0-9-]+$/.test(dependent.key));
    }
  }
  const DEFINITION_BASIS_FIELDS = ["catalog_digest", "format", "provenance_kind", "source_revision", "source_path", "dependency_digest"];
  const DEFINITION_LIMITS = [["max_depth", "Maximum depth"], ["max_steps", "Steps per workflow"], ["max_members", "Members per Step"], ["max_attempts", "Attempts per leaf"], ["max_works", "Total Works"]];
  function validDefinitions(data) {
    const basis = data.basis;
    assert(basis && DEFINITION_BASIS_FIELDS.every((key) => typeof basis[key] === "string") && basis.catalog_digest.length && basis.source_revision.length && basis.source_path.length && basis.format === "dna.workflow-definitions/2" && basis.provenance_kind === "trusted_host_claims" && basis.limits && DEFINITION_LIMITS.every(([key]) => decimal(basis.limits[key]) && basis.limits[key] !== "0"));
  }
  function sameDefinitionBasis(left, right) {
    return DEFINITION_BASIS_FIELDS.every((key) => left[key] === right[key]) && DEFINITION_LIMITS.every(([key]) => left.limits[key] === right.limits[key]);
  }
  const KNOWLEDGE_BASIS_FIELDS = ["record_id", "projection_record_head", "projection_record_revision", "projection_watermark", "store_generation", "routing_version", "visibility_record_head", "visibility_record_revision", "visibility_ledger_head", "visibility_ledger_revision", "reader_scope", "target", "coverage"];
  function receiptID(value) { return typeof value === "string" && /^(sha256:)?[0-9a-f]{64}$/.test(value); }
  function validKnowledge(data, source, route, kind, cursor = "") {
    assert(data && Array.isArray(data.items) && data.page && data.basis && data.coverage);
    const page = data.page, basis = data.basis;
    assert(page.limit === LIMIT && Number.isSafeInteger(page.returned) && page.returned >= 0 && page.returned <= page.limit && page.returned === data.items.length && typeof page.has_more === "boolean" && typeof page.snapshot === "string" && page.snapshot.length);
    assert(page.has_more ? page.returned > 0 && typeof page.next_cursor === "string" && page.next_cursor.length > 0 && page.next_cursor !== cursor : page.next_cursor === null, "The service returned an invalid knowledge page continuation.");
    assert(!route.snapshot || page.snapshot === route.snapshot, "The knowledge read returned a different snapshot than requested.");
    assert(KNOWLEDGE_BASIS_FIELDS.filter((key) => !key.startsWith("visibility_ledger_")).every((key) => typeof basis[key] === "string"));
    assert(["projection_record_revision", "projection_watermark", "store_generation", "routing_version", "visibility_record_revision"].every((key) => decimal(basis[key])));
    assert(basis.record_id === source.record_id && basis.projection_record_head === source.record_head && basis.visibility_record_head === source.record_head && basis.projection_record_revision === source.record_revision && basis.projection_watermark === source.record_revision && basis.visibility_record_revision === source.record_revision && basis.reader_scope.length && basis.target === (route.target || "") && basis.coverage === "native_ideas_edges_bindings", "The knowledge projection and its visibility policy do not share the requested Record and context.");
    assert(basis.routing_version === "0" ? basis.visibility_ledger_head === null && basis.visibility_ledger_revision === null : typeof basis.visibility_ledger_head === "string" && basis.visibility_ledger_head.length > 0 && decimal(basis.visibility_ledger_revision));
    assert(data.coverage.bindings === "complete" && ["runs", "definitions", "practices"].every((key) => data.coverage[key] === "unavailable"));
    if (kind === "nodes") validRows(data.items, "knowledge");
    else {
      const keys = new Set();
      const fields = kind === "edges" ? ["id", "from_id", "to_id", "rel"] : ["id", "idea_id", "target", "author", "class", "applicability"];
      for (const item of data.items) {
        assert(item && fields.every((key) => typeof item[key] === "string") && item.id.length && !keys.has(item.id));
        keys.add(item.id);
        if (kind === "edges") assert(receiptID(item.from_id) && receiptID(item.to_id) && (item.from_id === route.id || item.to_id === route.id), "The service returned a relationship outside the selected item.");
        else assert(item.idea_id === route.id && ["exact", "ancestor", "unfiltered"].includes(item.applicability));
      }
    }
  }
  function sameKnowledgeBasis(left, right) { return KNOWLEDGE_BASIS_FIELDS.every((key) => left[key] === right[key]); }
  function validOrganization(data) {
    const b = data.basis, ownership = data.ownership;
    assert(b && ["source_head", "seed", "artifact_digest", "dependency_digest", "shape_hash", "schema", "coverage"].every((key) => typeof b[key] === "string") && b.source_head.length && b.artifact_digest.length && b.dependency_digest.length === 71 && /^sha256:[0-9a-f]{64}$/.test(b.dependency_digest) && b.coverage === "static_instances");
    assert(["local_vendor_snapshot", "committed_source", "none"].includes(b.dependency_source));
    assert(typeof b.position_group_declared === "boolean" && typeof b.exact_ownership === "boolean" && ["semantics", "declaration_count", "uninstantiated_declaration_count"].every((key) => Number.isSafeInteger(b[key]) && b[key] >= 0));
    assert(ownership && ["single_owner", "shared"].includes(ownership.mode) && typeof ownership.host_owner === "string" && ownership.instance_binding === "unavailable");
    assert(Array.isArray(ownership.positions) && ownership.positions.every((p) => p && typeof p.position === "string" && typeof p.owner === "string"));
    assert(Array.isArray(ownership.memberships) && ownership.memberships.every((m) => m && typeof m.owner === "string" && stringArray(m.members)));
  }
  function sameOrganizationBasis(left, right) {
    return ["source_head", "seed", "artifact_digest", "dependency_digest", "dependency_source", "shape_hash", "schema", "coverage", "position_group_declared", "exact_ownership", "semantics", "declaration_count", "uninstantiated_declaration_count"].every((key) => left[key] === right[key]);
  }
  async function request(path, signal) {
    // This deadline covers both headers and the complete response body. It is
    // a browser read deadline, not proof that the service stopped its own work.
    const pending = new AbortController();
    let timedOut = false;
    const cancelRead = () => pending.abort();
    signal.addEventListener("abort", cancelRead, { once: true });
    if (signal.aborted) cancelRead();
    const timeout = setTimeout(() => { timedOut = true; pending.abort(); }, READ_TIMEOUT_MS);
    try {
      const response = await fetch(path, { signal: pending.signal, credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" } });
      if (response.status === 401) throw new ReadError(401, "unauthenticated", "The service needs a valid session before it can return Record data.");
      let body;
      try { body = await response.json(); }
      catch (error) {
        if (pending.signal.aborted) throw error;
        throw new ReadError(response.status, "invalid_response", "The service did not return a readable JSON response.");
      }
      if (!response.ok) throw new ReadError(response.status, body.error?.code || "request_failed", typeof body.error?.message === "string" ? body.error.message : "The read did not complete.");
      assert(body && body.api_version === "hale.v1" && body.data);
      validSource(body.source);
      return body;
    } catch (error) {
      if (signal.aborted) throw new DOMException("Superseded read", "AbortError");
      if (timedOut) throw new ReadError(0, "read_timeout", "The service did not finish this read within 15 seconds. Application data has been cleared. Retry when the service is ready.");
      if (error instanceof ReadError) throw error;
      throw new ReadError(0, "connection_failed", "The local service could not be reached. Check that it is running, then retry.");
    } finally {
      clearTimeout(timeout);
      signal.removeEventListener("abort", cancelRead);
    }
  }
  function ensureCurrent(token, signal) {
    if (token !== generation || signal.aborted) throw new DOMException("Superseded read", "AbortError");
  }
  function firstPageRoute(route) { return { ...route, offset: 0, cursor: "", edges_cursor: "", bindings_cursor: "", snapshot: "" }; }
  async function readKnowledge(base, route, token, signal) {
    if (!route.snapshot && (route.cursor || route.edges_cursor || route.bindings_cursor)) {
      route = firstPageRoute(route);
      state.notice = "This page link has no snapshot. Knowledge pagination restarted from the first page.";
    }
    async function page(kind, id, cursor, snapshot) {
      const query = new URLSearchParams({ limit: String(LIMIT) });
      if (id) query.set("id", id);
      if (route.target) query.set("target", route.target);
      if (cursor) query.set("cursor", cursor);
      if (snapshot) query.set("snapshot", snapshot);
      const response = await request(base + "/dna/knowledge/" + kind + "?" + query, signal);
      ensureCurrent(token, signal);
      validSource(response.source, route.app);
      validKnowledge(response.data, response.source, { ...route, id, snapshot }, kind, cursor);
      return response;
    }
    const response = await page("nodes", "", route.cursor, route.snapshot);
    const collection = response.data;
    route = { ...route, offset: 0, snapshot: collection.page.snapshot };
    let detail = null, detailError = null, relationships = null, bindings = null;
    if (route.id) {
      const reads = await Promise.allSettled([
        page("nodes", route.id, "", route.snapshot),
        page("edges", route.id, route.edges_cursor, route.snapshot),
        page("bindings", route.id, route.bindings_cursor, route.snapshot)
      ]);
      ensureCurrent(token, signal);
      const failures = reads.filter((read) => read.status === "rejected").map((read) => read.reason);
      // Authentication or snapshot failure clears the whole view even if a
      // parallel exact-item request also answered not found.
      const failure = failures.find((error) => error.status === 401) || failures.find((error) => error.status !== 404 || error.code === "application_not_found") || failures[0];
      if (failure) {
        if (failure.status !== 404 || failure.code === "application_not_found") throw failure;
        detailError = failure;
      } else {
        for (const read of reads) {
          const value = read.value;
          assert(value.source.record_head === response.source.record_head && value.source.record_revision === response.source.record_revision && value.data.page.snapshot === collection.page.snapshot && sameKnowledgeBasis(value.data.basis, collection.basis), "The knowledge item, relationships, and bindings do not share one source snapshot.");
        }
        const exact = reads[0].value.data;
        assert(exact.items.length === 1 && exact.items[0].id === route.id && !exact.page.has_more, "The knowledge detail does not match the exact receipt identity requested.");
        detail = exact.items[0];
        relationships = reads[1].value.data;
        bindings = reads[2].value.data;
      }
    }
    return { source: response.source, collection, detail, detailError, relationships, bindings, route };
  }
  function contextFromOrganization(response, appId) {
    validSource(response.source, appId);
    validPage(response.data, response.source, WORKSPACES.organization);
    validOrganization(response.data);
    const positions = [], names = new Set();
    for (const row of response.data.ownership.positions) {
      assert(row.position.length > 0 && row.position.length <= 1024 && !/[\u0000-\u001f\u007f]/.test(row.position) && !names.has(row.position), "The declared locus context list is ambiguous or invalid.");
      names.add(row.position); positions.push({ position: row.position, owner: row.owner });
    }
    positions.sort((a, b) => a.position.localeCompare(b.position));
    return { available: true, positions, source: response.source, sourceHead: response.data.basis.source_head };
  }
  function requireWorkingContext(route, context) {
    if (route.locus && (!context?.available || !context.positions.some(row => row.position === route.locus))) {
      throw new ReadError(404, "working_context_unavailable", "The selected locus is not available in the current declared ownership map. Choose another context or clear it.");
    }
  }
  function sameContextRecord(context, source) {
    if (context?.available && (context.source.record_id !== source.record_id || context.source.record_head !== source.record_head || context.source.record_revision !== source.record_revision)) {
      throw new ReadError(409, "snapshot_changed", "The context and workspace were read from different Record snapshots. Refresh to recheck them together.");
    }
  }
  async function readApplication(route, token, signal, authenticated) {
    const discovery = await request(API, signal);
    ensureCurrent(token, signal);
    validPage(discovery.data, discovery.source);
    const apps = discovery.data.items;
    for (const app of apps) assert(app && typeof app.id === "string" && app.id.length && app.kind === "dna" && typeof app.name === "string");
    if (!apps.length) return { apps, app: null, source: discovery.source, collection: null };
    const app = route.app ? apps.find((item) => item.id === route.app) : apps[0];
    if (!app) throw new ReadError(404, "application_not_found", "This service does not serve the application named in this link.");
    const base = API + "/" + encodeURIComponent(app.id);
    const capabilityResponse = await request(base + "/capabilities", signal);
    ensureCurrent(token, signal);
    validSource(capabilityResponse.source, app.id);
    const capabilities = capabilityResponse.data;
    const workspace = WORKSPACES[route.view];
    assert(capabilities.application_id === app.id && typeof capabilities.read_only === "boolean" && capabilities.principal && typeof capabilities.principal.name === "string" && ["local", "oidc"].includes(capabilities.principal.mode) && capabilities.reads);
    route = { ...route, app: app.id };
    // Recovery needs this freshly authenticated identity, not a successful
    // domain collection. Never include a partial collection or its contents.
    authenticated({ apps, app, capabilities, route });
    if (capabilities.reads[workspace.capability] !== true) throw new ReadError(0, "unsupported_capability", "This connection does not advertise the requested read capability.");
    let workingContext = null;
    if (route.view !== "organization") {
      if (capabilities.reads.organization === true) {
        try {
          const contextResponse = await request(base + "/dna/organization?limit=1", signal);
          ensureCurrent(token, signal);
          workingContext = contextFromOrganization(contextResponse, app.id);
        } catch (error) {
          if (error.name === "AbortError" || [401, 403, 409].includes(error.status) || error.code === "invalid_response") throw error;
          workingContext = { available: false, positions: [], error: "Declared locus contexts could not be read." };
        }
      } else if (route.locus) workingContext = { available: false, positions: [], error: "This connection does not provide declared locus contexts." };
      authenticated({ apps, app, capabilities, route, workingContext });
      requireWorkingContext(route, workingContext);
    }
    if (route.view === "knowledge") {
      if (route.locus && !route.practice_action) route = { ...route, target: route.locus };
      if (route.practice_action) route = { ...route, target: "" };
      const loaded = await readKnowledge(base, route, token, signal);
      sameContextRecord(workingContext, loaded.source);
      let practiceContext = null;
      if (route.practice_action) {
        assert(route.practice_action === "create" ? !route.id : Boolean(route.id), "Practice administration requires an exact practice, or an empty target for creation.");
        if (route.id && !loaded.detailError) {
          if (capabilities.reads.practices !== true) throw new ReadError(0, "unsupported_capability", "The exact Practice cannot be checked on this connection.");
          const response = await request(base + "/dna/practices?" + new URLSearchParams({ id: route.id, snapshot: loaded.source.record_head }), signal);
          ensureCurrent(token, signal); validSource(response.source, app.id); validPage(response.data, response.source, WORKSPACES.practices); validRows(response.data.items, "practices");
          assert(response.source.record_head === loaded.source.record_head && response.source.record_revision === loaded.source.record_revision && response.data.items.length === 1 && response.data.items[0].id === route.id, "The Practice and its applicability do not share one exact source snapshot.");
          const p = response.data.items[0], item = loaded.detail;
          if (!practiceTextAvailable(p)) throw new ReadError(403, "forbidden", "The exact Practice document is unavailable for administration.");
          assert(p.kind === "practice" && item?.kind === "practice" && ["name", "text", "author"].every(key => p[key] === item[key]), "The selected graph item does not match the exact Practice document.");
          practiceContext = p;
        }
      }
      return { apps, app, capabilities, workingContext, practiceContext, ...loaded };
    }
    if (route.offset > 0 && !route.snapshot) {
      route = { ...route, offset: 0 };
      state.notice = "This page link has no snapshot. Pagination restarted from the first page.";
    }
    if (route.view === "tasks" && (route.assignee_invalid || route.assignee && !commandID(route.assignee))) throw new ReadError(400, "invalid_assignee", "Choose one exact person from the ownership map, or return to all handed Tasks.");
    const query = new URLSearchParams({ limit: String(LIMIT), offset: String(route.offset) });
    if (route.view === "tasks" && route.assignee) query.set("assignee", route.assignee);
    if (route.snapshot) query.set("snapshot", route.snapshot);
    const collectionResponse = await request(base + "/dna/" + workspace.resource + "?" + query, signal);
    ensureCurrent(token, signal);
    validSource(collectionResponse.source, app.id);
    validPage(collectionResponse.data, collectionResponse.source, workspace);
    if (route.view === "workflows") validWorkflowBasis(collectionResponse.data);
    if (route.view === "tasks") validTaskBasis(collectionResponse.data, collectionResponse.source, route.assignee || "");
    if (route.view === "organization") validOrganization(collectionResponse.data);
    if (route.view === "definitions") {
      validDefinitions(collectionResponse.data);
      assert(!route.snapshot || collectionResponse.data.page.snapshot === route.snapshot, "The service returned a different definition catalog snapshot than requested.");
    }
    validRows(collectionResponse.data.items, route.view);
    assert(collectionResponse.data.page.offset === route.offset, "The service returned a different page than requested.");
    const collection = collectionResponse.data;
    if (route.view === "organization") {
      workingContext = contextFromOrganization(collectionResponse, app.id);
      authenticated({ apps, app, capabilities, route, workingContext });
      requireWorkingContext(route, workingContext);
    }
    sameContextRecord(workingContext, collectionResponse.source);
    route = { ...route, snapshot: collection.page.snapshot };
    let detail = null, detailError = null;
    if (route.id) {
      const detailQuery = new URLSearchParams({ id: route.id, snapshot: collection.page.snapshot });
      try {
        const response = await request(base + "/dna/" + workspace.resource + "?" + detailQuery, signal);
        ensureCurrent(token, signal);
        validSource(response.source, app.id);
        validPage(response.data, response.source, workspace);
        if (route.view === "workflows") validWorkflowBasis(response.data);
        if (route.view === "tasks") validTaskBasis(response.data, response.source);
        if (route.view === "organization") {
          validOrganization(response.data);
          assert(sameOrganizationBasis(response.data.basis, collection.basis), "The organization detail has a different source basis. No mixed version is displayed.");
        }
        if (route.view === "definitions") {
          validDefinitions(response.data);
          assert(sameDefinitionBasis(response.data.basis, collection.basis), "The definition detail has a different catalog or source basis. No mixed version is displayed.");
        }
        validRows(response.data.items, route.view);
        assert(response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision && response.data.page.snapshot === collection.page.snapshot && response.data.items.length === 1 && response.data.items[0].id === route.id, "The detail does not match the selected object and source snapshot.");
        detail = response.data.items[0];
      } catch (error) {
        if (error.status !== 404 || error.code === "application_not_found") throw error;
        detailError = error;
      }
    }
    // A branch is navigation context, independent of the selected inspector.
    // Its root may be outside the current page. Resolve only that exact object,
    // on the collection's basis; the read does not establish its full subtree.
    let organizationBranch = null, organizationBranchError = null;
    if (route.view === "organization" && route.branch) {
      organizationBranch = collection.items.find(item => item.id === route.branch) || (detail?.id === route.branch ? detail : null);
      if (!organizationBranch && route.id === route.branch && detailError) organizationBranchError = detailError;
      else if (!organizationBranch) {
        try {
          const query = new URLSearchParams({ id: route.branch, snapshot: collection.page.snapshot });
          const response = await request(base + "/dna/organization?" + query, signal);
          ensureCurrent(token, signal);
          validSource(response.source, app.id);
          validPage(response.data, response.source, workspace);
          validOrganization(response.data);
          validRows(response.data.items, "organization");
          assert(sameOrganizationBasis(response.data.basis, collection.basis) && response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision && response.data.page.snapshot === collection.page.snapshot && response.data.items.length === 1 && response.data.items[0].id === route.branch, "The organization branch does not match the requested identity and source snapshot.");
          organizationBranch = response.data.items[0];
        } catch (error) {
          if (error.status !== 404 || error.code === "application_not_found") throw error;
          organizationBranchError = error;
        }
      }
    }
    let reviewCandidate = null, reviewCandidateError = "";
    const readableBindingReview = (detail?.knowledge_binding_digest || detail?.knowledge_edge_digest) && detail.text_available === true && detail.text_status === "available";
    const readableSourceReview = detail?.organization_source === true && detail.text_available === true && detail.text_status === "available";
    if (route.view === "reviews" && (readableSourceReview || readableBindingReview || pendingPracticeReview(detail) && commandCapability(capabilities, REVIEW_OPERATION).supported)) {
      if (!readableSourceReview && !detail.knowledge_binding_digest && !detail.knowledge_edge_digest && capabilities.reads.practices !== true) reviewCandidateError = "The canonical candidate cannot be checked because this connection does not advertise practice reads. No decision can be submitted.";
      else {
        try {
          const binding = Boolean(detail.knowledge_binding_digest), edge = Boolean(detail.knowledge_edge_digest), sourceChange = readableSourceReview, typed = binding || edge || sourceChange;
          const candidateQuery = new URLSearchParams({ id: typed ? detail.id : detail.subject_digest, snapshot: collection.page.snapshot });
          const response = await request(base + (typed ? "/dna/reviews/candidate?" : "/dna/practices?") + candidateQuery, signal);
          ensureCurrent(token, signal);
          validSource(response.source, app.id);
          assert(response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision, "The canonical candidate and Review do not share one exact source snapshot.");
          let candidate;
          if (sourceChange) { candidate = await window.FaceOrganizationReview.validate(response.data, detail, app.id); ensureCurrent(token, signal); }
          else if (binding) candidate = validBindingCandidate(response.data, detail, app.id);
          else if (edge) {
            candidate = validRelationshipCandidate(response.data, app.id);
            assert(candidate.review_id === detail.id && candidate.candidate_digest === detail.subject_digest && candidate.relationship.by === detail.author, "The relationship candidate does not match this exact Review and proposer.");
          }
          else {
            validPage(response.data, response.source, WORKSPACES.practices); validRows(response.data.items, "practices");
            assert(response.data.page.snapshot === collection.page.snapshot && response.data.items.length === 1 && response.data.items[0].id === detail.subject_digest, "The candidate does not match this exact Review.");
            candidate = response.data.items[0];
          }
          if (typed || eligibleReview(detail, candidate)) reviewCandidate = candidate;
          else reviewCandidateError = "The canonical Knowledge candidate is protected, unavailable, no longer pending, or does not match this exact Review. Its text is withheld here and no decision can be submitted.";
        } catch (error) {
          // Auth, snapshot movement and malformed joins invalidate the whole
          // view. A missing or unavailable candidate only disables this action.
          if (![404, 503].includes(error.status) || error.code === "application_not_found") throw error;
          reviewCandidateError = "The exact canonical candidate could not be read at this snapshot. No decision can be submitted; refresh to check it again.";
        }
      }
    }
    let organizationStatus = null, organizationStatusError = "";
    if (route.view === "reviews" && detail?.organization_source === true) {
      try {
        const response = await request(base + "/dna/organization/source-status?" + new URLSearchParams({ id: detail.id, snapshot: collectionResponse.source.record_head }), signal);
        ensureCurrent(token, signal); validSource(response.source, app.id);
        assert(response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision, "The Organization status and Review do not share one exact source snapshot.");
        try { organizationStatus = window.FaceOrganizationStatus.validate(response.data, detail, reviewCandidate); }
        catch { assert(false, "The Organization change status does not match this Review or its exact source evidence."); }
      } catch (error) {
        if (![403, 404, 503].includes(error.status) || error.code === "application_not_found") throw error;
        organizationStatusError = error.status === 403 ? "This session cannot read the source change's application and runtime evidence." : "The source change status is unavailable in this snapshot. Its current running version is not established here.";
      }
    }
    let organizationImpact = null, organizationImpactError = "";
    if (route.view === "reviews" && detail?.organization_source === true) {
      try {
        const response = await request(base + "/dna/organization/source-impact?" + new URLSearchParams({ id: detail.id, snapshot: collectionResponse.source.record_head }), signal);
        ensureCurrent(token, signal); validSource(response.source, app.id);
        assert(response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision, "The responsibility check and Review do not share one exact source snapshot.");
        try { organizationImpact = window.FaceOrganizationImpact.validate(response.data, detail, collectionResponse.source); }
        catch { assert(false, "The responsibility check does not match this Review or its captured Record."); }
        if (organizationStatus) assert(["base_commit", "module_digest", "candidate_commit", "source_digest"].every(key => organizationImpact.source[key] === organizationStatus.source[key]), "The responsibility check and change status identify different source evidence.");
        if (reviewCandidate?.organization) assert(organizationImpact.source.base_commit === reviewCandidate.organization.base.source_head && organizationImpact.source.module_digest === reviewCandidate.organization.base.module_digest && organizationImpact.source.source_digest === reviewCandidate.organization.module.digest, "The responsibility check does not match the exact candidate source comparison.");
      } catch (error) {
        if (![403, 404, 503].includes(error.status) || error.code === "application_not_found") throw error;
        organizationImpact = null;
        organizationImpactError = error.status === 403 ? "This session cannot read the responsibility evidence for this source change." : "Responsibility evidence is unavailable at this snapshot. No conclusion about affected work can be drawn.";
      }
    }
    let person = null, personError = "";
    if (route.view === "tasks" && route.assignee && capabilities.reads.people === true) {
      try {
        const response = await request(base + "/dna/people?" + new URLSearchParams({ id: route.assignee, snapshot: collectionResponse.source.record_head }), signal);
        ensureCurrent(token, signal); validSource(response.source, app.id);
        assert(response.source.record_head === collectionResponse.source.record_head && response.source.record_revision === collectionResponse.source.record_revision);
        person = validPerson(response.data, response.source, route.assignee);
      } catch (error) {
        if (error.name === "AbortError" || [401, 409].includes(error.status)) throw error;
        personError = "The complete retirement plan is unavailable. No retirement can be prepared from this view.";
      }
    }
    return { apps, app, capabilities, workingContext, source: collectionResponse.source, collection, person, personError, detail, detailError, organizationBranch, organizationBranchError, reviewCandidate, reviewCandidateError, organizationStatus, organizationStatusError, organizationImpact, organizationImpactError, route };
  }
  function destroyIndependent() {
    applicationController?.destroy();
    applicationController = null;
    projectsController?.destroy();
    projectsController = null;
  }
  function setHead(head) {
    headState = head;
    state.head = head;
    const nav = $("nav-projects");
    if (nav) nav.hidden = APPLICATION_HOST || !head;
    if (state.route.view === "projects") renderConnection(true);
  }
  async function probeHead(signal) {
    if (headProbed || APPLICATION_HOST) return;
    const pending = new AbortController();
    const cancelRead = () => pending.abort();
    signal.addEventListener("abort", cancelRead, { once: true });
    if (signal.aborted) cancelRead();
    const timeout = setTimeout(cancelRead, READ_TIMEOUT_MS);
    let head = null;
    try {
      const response = await fetch(HEAD_API, { signal: pending.signal, credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" } });
      // Anything but a closed head envelope means no head stands behind this
      // API: the shell continues unchanged rather than raising an error.
      if (response.status === 200 && window.FaceProjects) {
        const envelope = window.FaceProjects.validate(await response.json(), "head");
        head = { principal: envelope.head.principal, data: envelope.data };
      }
    } catch (error) {
      if (signal.aborted) throw new DOMException("Superseded read", "AbortError");
      head = null;
    } finally {
      clearTimeout(timeout);
      signal.removeEventListener("abort", cancelRead);
    }
    headProbed = true;
    headRedirect = head?.data.state === "detached";
    setHead(head);
  }
  function destroyDefinitionDraft() {
    definitionDraftController?.destroy();
    definitionDraftController = null;
    definitionDraftHost = null;
  }
  function destroyOrganizationDraft() {
    organizationDraftController?.destroy();
    organizationDraftController = null;
    organizationDraftHost = null;
    ownershipDraftController?.destroy();
    ownershipDraftController = null;
    ownershipDraftHost = null;
  }
  function destroyKnowledgeDraft() {
    const previous = knowledgeDraftController;
    knowledgeDraftController = null;
    knowledgeDraftHost = null;
    knowledgePreview = null;
    knowledgeComparison = "compare";
    knowledgeSelectedEdge = "";
    previous?.destroy();
  }
  function cancel() {
    destroyKnowledgeDraft();
    destroyOrganizationDraft();
    destroyIndependent();
    destroyDefinitionDraft();
    clearIntervention();
    clearKnowledgeCommand();
    generation += 1;
    if (controller) controller.abort();
    controller = null;
  }
  async function loadRoute(route = readRoute(), initialNotice = "", focusTarget = null) {
    cancel();
    const token = generation;
    controller = new AbortController();
    const signal = controller.signal;
    lastStarted = Date.now();
    state = blankState(route);
    state.notice = initialNotice;
    if (route.view === "projects" || !independentView(route.view)) {
      render();
      try { await probeHead(signal); }
      catch (error) { if (error.name === "AbortError" || token !== generation || signal.aborted) return; }
      if (token !== generation || signal.aborted) return;
      // A detached head has no Record to read: land on Projects once.
      if (headRedirect && route.view !== "projects") {
        route = { ...route, view: "projects" };
        replaceRoute(route);
        state = blankState(route);
        state.notice = initialNotice;
      }
      headRedirect = false;
    }
    if (independentView(route.view)) {
      definitionJourney = null;
      state.phase = "ready";
      render();
      return;
    }
    render();
    // A moving snapshot gets one automatic reset. Continuing movement is an
    // explicit error with a manual retry, never an unbounded request loop.
    for (let attempt = 0; attempt < 2; attempt += 1) {
      let recoveryContext = null;
      try {
        const loaded = await readApplication(route, token, signal, (context) => {
          ensureCurrent(token, signal);
          recoveryContext = context;
        });
        ensureCurrent(token, signal);
        state = { ...state, ...loaded, phase: "ready", inspectedAt: new Date(), error: null };
        if (loaded.route) replaceRoute(loaded.route);
        reconcileDefinitionJourney();
        restoreIntervention();
        restoreKnowledgeCommand();
        render();
        if (focusTarget) focusPanel(focusTarget);
        ui.announcement.textContent = route.id ? "Selected record loaded." : "Record page loaded.";
        if (intervention.metadata && recoveryCapability().supported) void checkCommandStatus();
        if (knowledgeCommand.metadata) void checkKnowledgeCommand();
        return;
      } catch (error) {
        if (error.name === "AbortError" || token !== generation || signal.aborted) return;
        if (error.status === 409 && attempt === 0) {
          route = firstPageRoute(route);
          state = blankState(route);
          state.notice = route.view === "organization" ? "The organization source, dependencies, or Record changed. Pagination restarted from the first page." : route.view === "definitions" ? "The definition catalog, source basis, admission limits, or Record changed. Pagination restarted from the first page." : route.view === "knowledge" ? "Knowledge or its visibility policy changed. All pages restarted from the current snapshot." : "The Record changed. Pagination restarted from the first page.";
          replaceRoute(route);
          render();
          continue;
        }
        const notice = state.notice;
        // Authentication and unverifiable identity failures invalidate the
        // captured context too. Ordinary domain outages retain only discovery
        // and capabilities so GET recovery can proceed independently.
        const canRecover = recoveryContext && error instanceof ReadError && ![401, 403].includes(error.status) && !["application_not_found", "invalid_response", "unauthenticated", "forbidden", "command_context_changed"].includes(error.code);
        state = { ...blankState(route), ...(canRecover ? recoveryContext : {}), phase: canRecover ? "domain-error" : "error", error, notice };
        if (canRecover) {
          replaceRoute(state.route);
          restoreIntervention();
          restoreKnowledgeCommand();
        }
        render();
        ui.announcement.textContent = error.status === 401 ? "Sign in required. Record data cleared." : canRecover && intervention.metadata ? "Domain data could not be read and has been cleared. The saved request can still be checked independently." : "The read did not complete. Record data cleared.";
        if (canRecover && intervention.metadata && recoveryCapability().supported) void checkCommandStatus();
        if (canRecover && knowledgeCommand.metadata) void checkKnowledgeCommand();
        return;
      }
    }
  }
  function refresh() {
    const route = firstPageRoute(readRoute());
    replaceRoute(route);
    loadRoute(route);
  }
  function render() {
    // A recovery-only render may reattach the same draft DOM. Preserve the
    // editor caret; route/access changes already destroy the controller.
    const draftFocus = (definitionDraftHost?.contains(document.activeElement) || ownershipDraftHost?.contains(document.activeElement) || organizationDraftHost?.contains(document.activeElement) || knowledgeDraftHost?.contains(document.activeElement)) ? document.activeElement : null;
    const draftCaret = draftFocus && typeof draftFocus.selectionStart === "number" ? [draftFocus.selectionStart, draftFocus.selectionEnd, draftFocus.selectionDirection] : null;
    destroyIndependent();
    if (state.phase !== "ready" || state.route.view !== "definitions" || !state.detail || state.error) destroyDefinitionDraft();
    if (state.phase !== "ready" || state.route.view !== "organization" || state.error) destroyOrganizationDraft();
    if (state.phase !== "ready" || state.route.view !== "knowledge" || state.error) destroyKnowledgeDraft();
    const application = state.route.view === "application";
    const projects = state.route.view === "projects";
    const independent = application || projects;
    const practiceAdministration = state.route.view === "knowledge" && state.route.practice_action;
    const workspace = WORKSPACES[state.route.view];
    const title = application ? "Application" : projects ? "Projects" : practiceAdministration ? "Practice administration" : workspace.title;
    document.title = title + " · the face";
    document.body.dataset.view = state.route.view;
    document.body.classList.toggle("practice-operating", ["practices", "reviews"].includes(state.route.view) && Boolean(state.detail));
    ui["workspace-title"].textContent = title;
    ui["breadcrumb-current"].textContent = title;
    ui["workspace-kicker"].textContent = application ? "APPLICATION CONTROL" : projects ? "OPERATOR MACHINE" : practiceAdministration ? "PRACTICES / SCOPE & LIFECYCLE" : workspace.kicker;
    ui["workspace-description"].textContent = application ? "Understand current behavior, make a deliberate change, and follow the application's own result." : projects ? "Create, attach and operate DNA projects on this machine through the project service. Every action is the CLI verb, recorded as a durable receipt and read back before it is shown." : practiceAdministration ? "Shape a practice in context: its exact text, applicability, governing Review and retained history." : workspace.description;
    ui.refresh.hidden = independent;
    ui.content.setAttribute("aria-busy", String(state.phase === "loading"));
    ui.notice.hidden = !state.notice;
    ui.notice.textContent = state.notice;
    for (const view of VIEWS) {
      const nav = $("nav-" + view);
      if (!nav) continue;
      if (view === "projects") nav.hidden = APPLICATION_HOST || !state.head;
      else nav.hidden = APPLICATION_HOST || application ? !independentView(view) : view === "application";
      if (view === (practiceAdministration ? "practices" : state.route.view === "tasks" ? "workflows" : state.route.view)) nav.setAttribute("aria-current", "page");
      else nav.removeAttribute("aria-current");
      nav.href = routeHash(workspaceRoute(view === "workflows" && (state.route.view === "tasks" || state.capabilities?.reads?.workflows !== true && state.capabilities?.reads?.tasks === true) ? "tasks" : view));
    }
    const workUnavailable = state.capabilities?.reads?.workflows !== true && state.capabilities?.reads?.tasks !== true;
    $("nav-workflows").classList.toggle("unavailable-nav", !independent && workUnavailable);
    $("work-nav-status").hidden = independent || !workUnavailable;
    const knowledgeUnavailable = state.capabilities?.reads?.knowledge === false || (state.route.view === "knowledge" && state.error?.code === "unsupported_capability");
    const knowledgeNav = $("nav-knowledge");
    knowledgeNav.classList.toggle("unavailable-nav", knowledgeUnavailable);
    $("knowledge-nav-status").hidden = !knowledgeUnavailable;
    $("model-note").hidden = independent || !knowledgeUnavailable;
    ui.application.closest(".application-control").hidden = independent;
    document.querySelector(".brand").href = APPLICATION_HOST || application ? "#/application" : routeHash(workspaceRoute("practices"));
    if (knowledgeUnavailable) {
      knowledgeNav.setAttribute("aria-disabled", "true");
      knowledgeNav.setAttribute("aria-describedby", "model-note");
      knowledgeNav.removeAttribute("href");
      knowledgeNav.tabIndex = -1;
    } else {
      knowledgeNav.removeAttribute("aria-disabled");
      knowledgeNav.removeAttribute("aria-describedby");
      knowledgeNav.removeAttribute("tabindex");
    }
    renderConnection(independent);
    if (application) ui.principal.textContent = "Connecting to application";
    const practiceEnabled = !independent && commandCapability().allowed;
    const reviewEnabled = !independent && commandCapability(state.capabilities, REVIEW_OPERATION).allowed;
    const sourceReviewEnabled = !independent && commandCapability(state.capabilities, REVIEW_OPERATION, true).allowed;
    const organizationEnabled = !independent && commandCapability(state.capabilities, ORGANIZATION_OPERATION).allowed;
    const taskEnabled = !independent && commandCapability(state.capabilities, TASK_OPERATION).allowed;
    const personEnabled = !independent && commandCapability(state.capabilities, PERSON_OPERATION).allowed;
    const createEnabled = !independent && commandCapability(state.capabilities, TASK_CREATE_OPERATION).allowed;
    const commandEnabled = practiceEnabled || reviewEnabled || sourceReviewEnabled || organizationEnabled || taskEnabled || personEnabled || createEnabled;
    const taskActions = [createEnabled ? "New tasks" : "", personEnabled && taskEnabled ? "People & Task actions" : personEnabled ? "Retirement" : taskEnabled ? "Reassignment" : ""].filter(Boolean);
    const modeBadge = document.querySelector(".read-only");
    modeBadge.textContent = state.route.view === "tasks" ? taskActions.length ? taskActions.join(" & ") + " enabled" : "Read only" : state.route.view === "knowledge" ? knowledgeAccessLabel() : state.route.view === "organization" ? organizationEnabled ? "Organization proposals enabled" : "Read only" : state.route.view === "reviews" && state.detail?.organization_source ? sourceReviewEnabled ? "Organization decisions enabled" : "Read only" : practiceEnabled && reviewEnabled ? "Proposals & decisions enabled" : practiceEnabled ? "Practice proposals enabled" : reviewEnabled ? "Review decisions enabled" : "Read only";
    renderStrata(independent);
    renderSource();
    renderWorkingContext();
    ui["workspace-footer"].replaceChildren(node("span", "", application ? "Hale · application service" : projects ? "Hale · project service" : "Hale API · v1"), node("span", "", application ? "The application owns its controls, authority, state, and command outcomes." : projects ? "Every operation is the CLI verb, run by the head and journaled as a receipt. Effects are read back from the head before they are shown." : state.route.view === "organization" ? "Source, dependencies, and Record are pinned separately. Drafts require validation before export." : state.route.view === "definitions" ? "Definitions are code-authored. This workspace reads the host's loaded catalog." : state.route.view === "knowledge" ? "Knowledge, relationships, and bindings share one inspected snapshot." : state.route.view === "tasks" ? "Reassignment preserves each Task’s obligation and assignment history." : commandEnabled ? "Commands require explicit submission. Review settlement and adoption remain separate." : "State is read from the local Record. No changes are made here."));
    if (application) {
      const mount = node("div");
      ui.content.replaceChildren(mount);
      if (window.FaceApplication) applicationController = window.FaceApplication.mount(mount, {
        onContext: (context) => {
          if (state.route.view !== "application") return;
          ui.principal.textContent = context?.principal ? "Local · " + context.principal.name : "Not connected";
          modeBadge.textContent = context?.commandEnabled ? "Application controls enabled" : "Read only";
        }
      });
      else mount.append(stateCard("Application controls unavailable", "The application browser module could not be loaded. Reload this page to try again."));
    }
    else if (projects) {
      const mount = node("div");
      ui.content.replaceChildren(mount);
      if (state.phase === "loading") mount.append(stateCard("Reading the project service", "Checking whether an operator-machine head answers behind this API.", "◌"));
      else if (window.FaceProjects) {
        const token = generation;
        projectsController = window.FaceProjects.mount(mount, {
          head: state.head, principal: state.head?.principal || null,
          onHead(head) { if (token === generation) setHead(head); },
          onAttached(applicationId) { if (token === generation) navigate(workspaceRoute("practices", { app: applicationId, locus: "", target: "" })); },
          onInvalidate(error) {
            if (token !== generation) return;
            const message = error?.message || "The project service principal or access changed.";
            if (error?.status === 401 || error?.status === 403 || error?.code === "command_context_changed") {
              const route = state.route; cancel();
              state = { ...blankState(route), phase: "error", error: new ReadError(error.status || 409, error.code || "unauthenticated", message) }; render();
            } else loadRoute(state.route, message);
          }
        });
      }
      else mount.append(stateCard("Projects instrument unavailable", "The project browser module could not be loaded. Reload this page to try again."));
    }
    else if (state.phase === "loading") ui.content.replaceChildren(stateCard(state.route.view === "organization" ? "Reading the organization source" : state.route.view === "definitions" ? "Reading the definition catalog" : state.route.view === "knowledge" ? "Reading the knowledge graph" : "Reading the Record", state.route.view === "organization" ? "Checking this page against its committed source, captured dependencies, and Record snapshot. Previously displayed content has been cleared." : state.route.view === "definitions" ? "Checking the loaded catalog and its source basis. Previously displayed content has been cleared." : state.route.view === "knowledge" ? "Checking the visible items and their connections against one snapshot. Previously displayed content has been cleared." : "Loading this page and its source snapshot. Previously displayed content has been cleared.", "◌"));
    else if (state.error) ui.content.replaceChildren(errorCard(state.error));
    else if (!state.app) ui.content.replaceChildren(stateCard("No applications available", "This service has not returned an accessible application.", "◇", [button("Retry", refresh)]));
    else ui.content.replaceChildren(renderCatalog());
    if (!independent && ["ready", "domain-error"].includes(state.phase)) {
      if (["tasks", "workflows"].includes(state.route.view)) {
        const tabs = node("nav", "intervention-actions"); tabs.setAttribute("aria-label", "Work views");
        for (const [view, label] of [["tasks", "Handed Tasks"], ["workflows", "Executions"]]) {
          if (state.capabilities?.reads?.[view] !== true) continue;
          const tab = navigationLink(label, routeHash(workspaceRoute(view)), "list", "button secondary");
          if (view === state.route.view) tab.setAttribute("aria-current", "page"); tabs.append(tab);
        }
        ui.content.prepend(tabs);
      }
      const knowledgeRecovery = renderKnowledgeCommandRecovery();
      if (knowledgeRecovery) {
        const workspace = ui.content.querySelector(".knowledge-workspace"), map = workspace?.querySelector("#relationship-map-panel");
        if (map) map.after(knowledgeRecovery); else if (workspace) workspace.prepend(knowledgeRecovery); else ui.content.prepend(knowledgeRecovery);
      }
      const recovery = renderCommandRecovery();
      if (recovery) {
        const metadata = intervention.metadata, receipt = intervention.receipt?.receipt;
        const related = metadata && state.detail && (state.route.view === "tasks" ? metadata.operation === TASK_OPERATION && state.detail.id === metadata.target_id : state.route.view === "practices" ? state.detail.id === metadata.subject_digest || state.detail.id === receipt?.proposal?.candidate_digest : state.route.view === "reviews" && (state.detail.id === metadata.target_id || state.detail.id === receipt?.proposal?.review_id));
        const anchor = related && ui.content.querySelector(".practice-detail > .detail-rule, .review-detail > .detail-rule, .task-responsibility-detail > .detail-rule");
        if (anchor) anchor.after(recovery);
        else ui.content.prepend(recovery);
      }
    }
    if (draftFocus?.isConnected && (definitionDraftHost?.contains(draftFocus) || ownershipDraftHost?.contains(draftFocus) || organizationDraftHost?.contains(draftFocus) || knowledgeDraftHost?.contains(draftFocus))) {
      draftFocus.focus({ preventScroll: true });
      if (draftCaret) draftFocus.setSelectionRange(...draftCaret);
    }
  }
  function renderStrata(independent) {
    const strata = $("strata");
    strata.hidden = independent;
    const structure = ["organization", "definitions"].includes(state.route.view);
    const app = state.app?.id || state.route.app;
    const structural = link("01 / Structure", routeHash(workspaceRoute("organization", { app })), "stratum");
    const recorded = link("03 / Record", routeHash(workspaceRoute("practices", { app })), "stratum");
    const workView = ["tasks", "workflows"].includes(state.route.view);
    if (!independent && !workView) (structure ? structural : recorded).setAttribute("aria-current", "page");
    const workAvailable = state.capabilities?.reads?.workflows === true || state.capabilities?.reads?.tasks === true;
    const ledger = workAvailable ? link("02 / Work", routeHash(workspaceRoute(state.route.view === "tasks" || state.capabilities?.reads?.workflows !== true ? "tasks" : "workflows")), "stratum") : button("02 / Work", () => {}, "stratum");
    ledger.disabled = !workAvailable;
    if (workView) ledger.setAttribute("aria-current", "page");
    ledger.setAttribute("aria-describedby", "ledger-plane-hint");
    const hint = node("span", "strata-note", state.route.view === "tasks" ? "Task responsibility · exact assignment history" : workAvailable ? "Recorded execution · live overlay unavailable" : "Work reader unavailable");
    hint.id = "ledger-plane-hint";
    const mode = node("span", "strata-mode", structure ? "Declared structure" : state.route.view === "tasks" ? "Task responsibility" : state.route.view === "workflows" ? "Execution evidence" : "Recorded evidence");
    strata.replaceChildren(structural, ledger, recorded, hint, mode);
  }
  function renderWorkingContext() {
    const panel = $("working-context");
    if (!panel) return;
    panel.replaceChildren();
    const context = state.workingContext;
    panel.hidden = independentView(state.route.view) || !["ready", "domain-error"].includes(state.phase) || (!context && !state.route.locus);
    if (panel.hidden) return;
    const control = node("div", "working-context-control");
    const label = node("label", "eyebrow", "Working locus"); label.htmlFor = "working-locus";
    const select = node("select"); select.id = "working-locus";
    const all = node("option", "", "All declared loci"); all.value = ""; select.append(all);
    for (const row of context?.positions || []) {
      const option = node("option", "", row.position + (row.owner ? " · " + row.owner : ""));
      option.value = row.position; select.append(option);
    }
    const selected = context?.positions.find(row => row.position === state.route.locus);
    if (state.route.locus && !selected) {
      const unavailable = node("option", "", "Unavailable context"); unavailable.value = state.route.locus; select.append(unavailable);
    }
    select.value = state.route.locus || "";
    select.addEventListener("change", () => changeWorkingContext(select.value));
    select.disabled = !context?.available && !state.route.locus;
    control.append(label, select);
    const description = node("div", "working-context-description");
    const semantics = state.route.view === "knowledge" ? state.route.locus ? "Knowledge relevance follows this locus, including applicable ancestor bindings." : "Knowledge has its own relevance filter below." : state.route.view === "practices" ? "Practice pages remain unfiltered; exact target matches are marked." : state.route.view === "definitions" ? "Catalog pages remain unfiltered; matching direct leaf targets are marked." : state.route.view === "tasks" ? "Tasks retain their recorded assignees. Viewing a locus does not filter assignments or grant reassignment authority." : state.route.view === "workflows" ? "Execution pages remain unfiltered. Viewing a locus does not establish task assignment." : state.route.view === "reviews" ? "Reviews retain their recorded scope and required authority." : "The chart shows compiler instances. Their binding to these ownership scopes is unavailable.";
    description.append(node("p", "", context?.available ? semantics : context?.error || "Declared context unavailable."), node("p", "working-context-authority", "Viewing context only · your signed-in identity and command authority stay unchanged."));
    if (selected) description.append(node("p", "working-context-owner", "Declared owner · " + display(selected.owner, "Not supplied")));
    const actions = node("div", "working-context-actions");
    if (state.route.locus) {
      actions.append(button("Clear working context", () => changeWorkingContext("")));
      if (state.route.view !== "knowledge" && state.capabilities?.reads?.knowledge === true) actions.append(link("Knowledge here", routeHash(workspaceRoute("knowledge")), "button secondary"));
    }
    append(panel, control, description, actions);
    if (context?.sourceHead) {
      const evidence = node("details", "working-context-evidence");
      evidence.append(node("summary", "", "Context source"), node("p", "mono", context.sourceHead), node("p", "detail-note", "Paths and owners come from the checked source's declared ownership map. They do not establish occupied positions or permission grants.")); panel.append(evidence);
    }
  }
  function targetMatchesContext(item, view = state.route.view) {
    const locus = state.route.locus;
    if (!locus || !state.workingContext?.available) return false;
    if (view === "practices") return item.target === locus;
    if (view === "definitions") return item.steps.some(step => step.members.some(member => member.kind === "leaf" && member.leaf.target === locus));
    return false;
  }
  function renderConnection(independent) {
    const select = ui.application;
    const projects = state.route.view === "projects";
    const head = state.head;
    select.replaceChildren();
    if (state.apps.length && !independent) {
      for (const app of state.apps) {
        const option = node("option", "", display(app.name, short(app.id)));
        option.value = app.id;
        option.selected = app.id === state.app?.id;
        select.append(option);
      }
      select.disabled = false;
    } else {
      select.append(node("option", "", projects ? "Project service" : independent ? "Application service" : state.phase === "loading" ? "Connecting…" : "No Record connected"));
      select.disabled = true;
    }
    ui["connection-caption"].textContent = projects ? (head ? head.data.state === "attached" ? "Head · attached to " + head.data.active.name : "Head · no project attached" : "No project service behind this API") : independent ? "No DNA connection required" : state.app ? "DNA · local Record" : state.phase === "loading" ? "Reading the local service" : "Application data unavailable";
    const principal = state.capabilities?.principal;
    ui.principal.textContent = projects ? (head ? "Local · " + head.principal.name : "Not connected") : principal ? (principal.mode === "oidc" ? "Signed in · " : "Local · ") + principal.name : state.error?.status === 401 ? "Sign in required" : state.phase === "loading" ? "Connecting" : "Not connected";
    ui["sign-out"].hidden = !principal || principal.mode !== "oidc";
  }
  function renderSource() {
    ui.source.replaceChildren();
    ui.source.hidden = !state.source;
    if (!state.source) return;
    const source = state.source;
    const summary = node("div", "source-summary");
    append(summary,
      append(node("span", "source-label"), node("span", "source-dot"), document.createTextNode("Local Record snapshot")),
      append(node("span"), document.createTextNode("Revision "), node("strong", "mono", source.record_revision)),
      append(node("span"), document.createTextNode("Inspected "), node("strong", "", state.inspectedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })))
    );
    const details = node("details");
    details.append(node("summary", "", "Source details"));
    const identities = node("div", "source-identities");
    for (const [label, value] of [["Record", source.record_id], ["Head", source.record_head]]) append(identities, append(node("div"), node("span", "", label), node("code", "", value)));
    if (state.route.view === "definitions" && state.collection?.basis) {
      const basis = state.collection.basis;
      for (const [label, value] of [["Catalog", basis.catalog_digest], ["Format", basis.format], ["Provenance", "Trusted host claims"], ["Source revision", basis.source_revision], ["Source path", basis.source_path], ["Dependencies", display(basis.dependency_digest, "Not supplied")]]) append(identities, append(node("div"), node("span", "", label), node("code", "", value)));
      for (const [key, label] of DEFINITION_LIMITS) append(identities, append(node("div"), node("span", "", label), node("code", "", basis.limits[key])));
      summary.append(append(node("span"), document.createTextNode("Catalog "), node("strong", "mono", short(basis.catalog_digest, 17))));
      if (!state.detail) identities.append(node("p", "detail-note", "The catalog digest identifies the loaded definition bytes. The source revision, module path, and dependency digest are claims supplied by the host; this read does not independently verify those claims against source files."));
    } else if (state.route.view === "knowledge" && state.collection?.basis) {
      const basis = state.collection.basis;
      summary.append(append(node("span"), document.createTextNode("Graph generation "), node("strong", "mono", basis.store_generation)));
      for (const [label, value] of [["Projection head", basis.projection_record_head], ["Watermark", basis.projection_watermark], ["Graph generation", basis.store_generation], ["Visibility head", basis.visibility_record_head], ["Ledger head", basis.visibility_ledger_head === null ? "Record only" : basis.visibility_ledger_head], ["Ledger revision", basis.visibility_ledger_revision === null ? "Not applicable" : basis.visibility_ledger_revision], ["Routing version", basis.routing_version], ["Reader scope", basis.reader_scope], ["Target context", display(basis.target, "All loci")]]) append(identities, append(node("div"), node("span", "", label), node("code", "", value)));
      identities.append(node("p", "detail-note", "The projection and receipt visibility were checked together for this reader and target context."));
    } else if (state.route.view === "workflows" && state.collection?.basis) {
      identities.append(node("p", "detail-note", "Native wf1 projection of recorded facts. This connection reads Record routing 0; runtime identity and live execution are not inferred."));
    } else if (state.route.view === "tasks" && state.collection?.basis) {
      identities.append(node("p", "detail-note", "Task assignments and handoff requirements come from this Record snapshot. Refresh to inspect changes recorded since this read."));
    } else if (state.route.view === "organization" && state.collection?.basis) {
      const basis = state.collection.basis;
      const dependencyOrigin = { local_vendor_snapshot: "Captured local vendor files", committed_source: "Committed vendor files", none: "No vendor files" };
      for (const [label, value] of [["Source", basis.source_head], ["Seed", basis.seed], ["Dependencies", basis.dependency_digest], ["Dependency origin", dependencyOrigin[basis.dependency_source]], ["Artifact", basis.artifact_digest], ["Shape", basis.shape_hash], ["Schema", basis.schema], ["Semantics", basis.semantics]]) append(identities, append(node("div"), node("span", "", label), node("code", "", value)));
      summary.append(append(node("span"), document.createTextNode("Source "), node("strong", "mono", short(basis.source_head, 10))));
      identities.append(node("p", "detail-note", "Organization declarations come from the committed source revision. Uncommitted declaration edits are not included."));
      identities.append(node("p", "detail-note", basis.dependency_source === "local_vendor_snapshot" ? "Dependencies were captured from local vendor files, separately from the source commit. Their digest identifies the bytes used for this inspection. This read does not verify them against a dependency lockfile." : basis.dependency_source === "committed_source" ? "Dependencies came from committed vendor files. Their digest identifies the bytes used for this inspection." : "This inspection used no vendor files."));
    }
    if (!["definitions", "knowledge"].includes(state.route.view)) identities.append(node("p", "detail-note", "This is the inspected local snapshot. Remote synchronization and Ledger freshness are not established by this read."));
    details.append(identities);
    append(ui.source, summary, details);
  }
  function badge(label, tone = "") { return node("span", "badge " + tone, label); }
  function practiceBadge(practice) {
    const tones = { pending: "amber", ratified: "green", declined: "red", retired: "", refused: "red" };
    const label = PRACTICE_STATES[practice.state] || "Unknown · " + practice.state;
    return badge(practice.kind === "retirement" ? "Retirement · " + label : label, tones[practice.state] || "");
  }
  function practiceDocumentLabel(practice) {
    return practice.kind === "practice" ? "Practice document" : practice.kind === "retirement" ? "Retirement document" : "Knowledge document · " + display(practice.kind);
  }
  function reviewBadge(review) {
    return badge(display(review.state), review.state === "pending" ? "amber" : "");
  }
  function practiceMeta(item) {
    return append(node("div", "record-meta"), node("div", "", practiceDocumentLabel(item)), node("div", "", "Proposed by " + display(item.requester, "unrecorded requester")), node("code", "", short(item.digest, 27)));
  }
  function reviewMeta(item) {
    return append(node("div", "record-meta"), node("div", "", "Authority · " + display(item.required_authority)), node("div", "", item.outcome ? "Decision · " + (OUTCOMES[item.outcome] || "Unknown · " + item.outcome) : "No decision recorded"));
  }
  function organizationBadge(item) { return badge(item.role === "position" ? "Declared position" : "Structure", item.role === "position" ? "green" : ""); }
  function organizationMeta(item) {
    return append(node("div", "record-meta"), node("div", "", "Declaration · " + display(item.declaration)), node("div", "", "Static instance · " + display(item.thread_domain, "thread domain not specified")));
  }
  function definitionBadge(item) { return badge("Revision " + item.revision); }
  function definitionStepLabel(index) { return "Step " + (BigInt(index) + 1n).toString(); }
  function definitionMeta(item) {
    const members = item.steps.reduce((count, step) => count + step.members.length, 0);
    return append(node("div", "record-meta"), node("code", "", item.id), node("div", "", item.steps.length + (item.steps.length === 1 ? " Step" : " Steps") + " · " + members + (members === 1 ? " member" : " members")));
  }
  const WORK_STATES = { bound: "Bound · not admitted", admitted: "Admitted", registered: "Registered", activated: "Activated", attempt_admitted: "Attempt outstanding", attempt_done: "Attempt done · Work unsettled", attempt_failed: "Attempt failed · Work unsettled", attempt_declined: "Attempt declined · Work unsettled", attempt_timeout: "Attempt timed out · Work unsettled", done: "Done", completed: "Completed", failed: "Failed", cancelled: "Cancelled", refused: "Refused", unclassified: "No wf1 admission" };
  function workflowBadge(value) { return badge(WORK_STATES[value] || value, ["done", "completed"].includes(value) ? "green" : ["failed", "refused", "cancelled", "attempt_failed", "attempt_timeout", "attempt_declined"].includes(value) ? "amber" : ""); }
  function validWorkflowBasis(data) {
    assert(data.basis?.projection === "dna.workflow-projection/1" && data.basis.memory === "record" && data.basis.routing === "0" && data.basis.runtime_association === false, "The execution source does not match this reader's contract.");
  }
  function validTaskBasis(data, source, assignee = "") {
    const b = data.basis;
    assert(closedObject(data, ["profile", "items", "page", "basis", ...(assignee ? ["assignee"] : [])]) && (!assignee || data.assignee === assignee && data.items.every(row => row.assignee === assignee)) && data.profile === "dna.task-administration.v1" && closedObject(b, ["projection", "memory", "routing", "record_head", "record_revision"]) && b.projection === "dna.task-administration/1" && b.memory === "record" && b.routing === "0" && b.record_head === source.record_head && b.record_revision === source.record_revision, "The Task responsibilities do not match their captured Record.");
    for (const row of data.items) assert(Array.isArray(row.history) && row.history.every(event => decimal(event.sequence) && BigInt(event.sequence) < BigInt(source.record_revision)), "Task assignment history extends beyond the captured Record.");
  }
  function validPerson(value, source, person) {
    assert(closedObject(value, ["profile", "person", "state", "successor", "event_id", "subject_digest", "tasks", "transferred", "authorized", "recipients", "basis"]));
    assert(value.profile === "dna.person-administration.v1" && value.person === person && commandID(person) && ["active", "retired"].includes(value.state) && sourceDigest(value.subject_digest));
    assert((value.successor === "" || commandID(value.successor)) && (value.state === "active" ? value.event_id === "" && value.successor === "" : sourceCommit(value.event_id) && value.authorized === false));
    assert(typeof value.authorized === "boolean" && Array.isArray(value.tasks) && value.tasks.length <= 32 && value.transferred === String(value.tasks.length));
    const ids = new Set();
    for (const task of value.tasks) { assert(closedObject(task, ["id", "assignment_digest", "from"]) && commandID(task.id) && sourceDigest(task.assignment_digest) && task.from === person && !ids.has(task.id)); ids.add(task.id); }
    assert(Array.isArray(value.recipients) && value.recipients.length <= 64 && value.recipients.every(value => commandID(value)) && new Set(value.recipients).size === value.recipients.length && (value.authorized || value.recipients.length === 0));
    assert(closedObject(value.basis, ["memory", "routing", "record_head", "record_revision"]) && value.basis.memory === "record" && value.basis.routing === "0" && value.basis.record_head === source.record_head && value.basis.record_revision === source.record_revision);
    return value;
  }
  function preparePersonRetirement(plan, to) {
    const draft = { operation: PERSON_OPERATION, target: plan.person, subject: plan.subject_digest, to };
    if (state.person !== plan || !commandCapability(state.capabilities, PERSON_OPERATION).allowed || intervention.metadata || intervention.draft || intervention.blocked || !validDraft(draft) || !currentDraftEligible(draft)) throw new Error("Refresh the person's responsibilities and authority before preparing retirement.");
    intervention.draft = draft; intervention.phase = "reviewing"; intervention.error = "";
    render(); $("person-retirement-confirmation")?.focus();
  }
  function renderPersonAdministration() {
    const plan = state.person;
    const panel = node("section", "person-administration"); panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Person administration");
    const heading = append(node("header", "person-heading"), node("span", "person-monogram", Array.from(plan.person)[0]), append(node("div"), node("p", "eyebrow accent", "PEOPLE / RESPONSIBILITY"), node("h2", "", plan.person), node("p", "detail-note", plan.state === "retired" ? "Retirement recorded" : plan.tasks.length + " open responsibilities to carry forward")));
    panel.append(heading);
    if (plan.state === "retired") {
      panel.append(node("p", "person-retirement-destination", plan.successor ? "Future handoffs follow " + plan.successor : "Future handoffs have no named successor."));
      if (plan.successor) panel.append(navigationLink("Inspect " + plan.successor + "’s assignments", routeHash(workspaceRoute("tasks", { assignee: plan.successor })), "list", "text-link"));
      panel.append(node("p", "detail-note", "The record retains the person's work and history. Organization memberships and declared positions are managed separately."));
      return panel;
    }
    const destination = node("strong", "person-destination", "Choose who carries the work");
    const bridge = append(node("div", "person-transfer-bridge"), append(node("div", "person-transfer-end"), node("span", "eyebrow", "RETIRING"), node("strong", "", plan.person)), node("span", "person-transfer-arrow", "→"), append(node("div", "person-transfer-end"), node("span", "eyebrow", "SUCCESSOR"), destination));
    bridge.setAttribute("aria-label", "Responsibility transfer"); panel.append(bridge);
    const work = node("ul", "person-transfer-tasks"); work.setAttribute("aria-label", "Responsibilities included in retirement");
    for (const task of plan.tasks) {
      const captured = state.collection.items.find(row => row.id === task.id);
      work.append(append(node("li"), node("span", "person-transfer-dot", ""), navigationLink(captured?.outcome || task.id, routeHash(workspaceRoute("tasks", { assignee: plan.person, id: task.id })), "detail", "text-link"), node("span", "detail-note", "Requirements retained")));
    }
    if (plan.tasks.length) panel.append(work);
    else panel.append(node("p", "detail-note", "No open handed Tasks are included in this complete retirement plan."));
    const draft = intervention.draft;
    if (draft?.operation === PERSON_OPERATION && draft.target === plan.person) {
      destination.textContent = draft.to || "No named successor";
      const confirmation = node("section", "person-retirement-confirmation"); confirmation.id = "person-retirement-confirmation"; confirmation.tabIndex = -1;
      confirmation.setAttribute("role", "group"); confirmation.setAttribute("aria-label", "Confirm person retirement");
      confirmation.append(node("h3", "", "Retire " + plan.person + "?"), node("p", "", plan.tasks.length + " open responsibilities will move together" + (draft.to ? " to " + draft.to : "") + ". Their obligations and acceptance requirements stay attached. Future handoffs follow the recorded successor; the person stops receiving new work."));
      if (!draft.to) confirmation.append(node("p", "", "No successor is named. Future requests for this person remain unassigned."));
      const confirm = button("Confirm retirement", submitIntervention, "button primary"), cancel = button("Keep person active", () => { intervention.draft = null; intervention.phase = "idle"; intervention.error = ""; render(); });
      confirm.disabled = Boolean(intervention.reserving) || !currentDraftEligible(draft); cancel.disabled = Boolean(intervention.reserving);
      confirmation.append(append(node("div", "intervention-actions"), confirm, cancel));
      if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); confirmation.append(error); }
      panel.append(confirmation); return panel;
    }
    const allowed = commandCapability(state.capabilities, PERSON_OPERATION).allowed && plan.authorized && !intervention.metadata && !intervention.draft && !intervention.blocked;
    const form = node("form", "person-retirement-form"); form.setAttribute("aria-label", "Prepare person retirement");
    const select = node("select"); select.setAttribute("aria-label", "Retirement successor"); select.disabled = !allowed;
    const placeholder = node("option", "", "Choose a successor"); placeholder.disabled = true; placeholder.selected = true; select.append(placeholder);
    for (const name of plan.recipients.filter(name => name !== plan.person)) { const option = node("option", "", name); option.value = name; select.append(option); }
    if (!plan.tasks.length) { const option = node("option", "", "No successor · leave future requests unassigned"); option.value = ""; select.append(option); }
    const submit = node("button", "button primary", "Review retirement"); submit.type = "submit"; submit.disabled = true;
    const status = node("p", "detail-note"); status.setAttribute("role", "status");
    if (!allowed) status.textContent = intervention.metadata ? "Resolve the saved request before preparing another change." : "This session has no current retirement grant for this person.";
    select.addEventListener("change", () => { destination.textContent = select.value || "No named successor"; submit.disabled = !allowed || select.selectedIndex < 1; });
    form.addEventListener("submit", event => { event.preventDefault(); if (!allowed || select.selectedIndex < 1) return; try { preparePersonRetirement(plan, select.value); } catch (error) { status.textContent = error.message; } });
    form.append(append(node("label"), node("span", "eyebrow", "Who carries the work next?"), select), submit, status);
    panel.append(form, node("p", "detail-note", "Retirement preserves work and history. It does not remove a source membership or delete a position. The service checks the complete plan again at submission."));
    return panel;
  }
  function renderPersonRecovery() {
    const metadata = intervention.metadata, result = intervention.receipt;
    const panel = node("section", "intervention-panel"); panel.id = "command-recovery"; panel.tabIndex = -1;
    panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Person retirement request"); panel.append(node("h3", "", "Retirement · " + metadata.target_id));
    if (result) {
      const outcome = result.receipt.person;
      const observed = outcome.state === "applied" && state.person?.person === metadata.target_id && state.person.state === "retired" && state.person.event_id === outcome.event_id && state.person.successor === outcome.to;
      panel.dataset.observation = observed ? "observed" : "unavailable";
      panel.append(node("p", "outcome-value", outcome.state === "applied" ? outcome.from + " → " + (outcome.to || "No named successor") : "Retirement outcome unknown"), node("p", "", outcome.state === "applied" ? outcome.transferred + " responsibilities transferred with this retirement." : "Keep this request identity while its result is established."), node("p", "detail-note", observed ? "The current person read contains this exact retirement event." : "The current person state has not yet confirmed this exact event."));
    } else panel.append(node("p", "detail-note", intervention.phase === "submitting" ? "Recording the retirement and its transfers…" : "Recovering the saved retirement request."));
    if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); panel.append(error); }
    const check = button("Check request status", checkCommandStatus); check.id = "command-check-status"; check.disabled = Boolean(commandController) || !recoveryCapability().supported;
    const actions = append(node("div", "intervention-actions"), check, navigationLink("Inspect person", routeHash(workspaceRoute("tasks", { assignee: metadata.target_id })), "detail", "text-link"));
    if (result && COMMAND_TERMINAL.has(result.receipt.state)) actions.append(button("Dismiss completed request", dismissCompletedRequest));
    const evidence = append(node("details", "intervention-evidence"), node("summary", "", "Request evidence")), facts = node("dl", "fact-grid");
    fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Reviewed responsibility plan", metadata.subject_digest, true, true);
    if (result?.receipt.person.event_id) fact(facts, "Retirement event", result.receipt.person.event_id, true, true);
    evidence.append(facts); panel.append(actions, evidence); return panel;
  }
  function prepareTaskReassignment(item, selection) {
    if (state.phase !== "ready" || state.route.view !== "tasks" || state.detail !== item || !commandCapability(state.capabilities, TASK_OPERATION).allowed || intervention.metadata || intervention.blocked || intervention.draft || !item.reassignment_supported || item.state !== "handed" || !commandID(item.assignee) || !commandID(selection?.to) || selection.to === item.assignee || !state.capabilities.task_commands.recipients.includes(selection.to)) throw new Error("Reload this Task and check the current authority before preparing a reassignment.");
    intervention.draft = { operation: TASK_OPERATION, target: item.id, subject: item.assignment_digest, from: item.assignee, to: selection.to };
    intervention.phase = "reviewing"; intervention.error = "";
    render(); $("task-reassignment-confirmation")?.focus();
  }
  function taskDetail(item) {
    const detail = node("div", "task-responsibility-detail");
    detail.append(backToList(), node("hr", "detail-rule"));
    if (state.route.assignee && item.assignee !== state.route.assignee) {
      detail.append(node("p", "detail-note", "This Task is now recorded under " + (item.assignee || "no assignee") + ". The list still shows assignments for " + state.route.assignee + "."));
      detail.append(navigationLink(item.assignee ? "View assignments for " + item.assignee : "View all assignments", routeHash(workspaceRoute("tasks", { assignee: item.assignee })), "list", "text-link"));
    }
    const capability = commandCapability(state.capabilities, TASK_OPERATION);
    detail.append(window.FaceTaskAdministration.render(item, {
      inspectedAt: state.inspectedAt, historical: false,
      canReassign: capability.allowed && !intervention.metadata && !intervention.blocked && !intervention.draft && commandID(item.assignee),
      recipients: capability.allowed ? state.capabilities.task_commands.recipients : [],
      onPrepareReassignment: selection => prepareTaskReassignment(item, selection), onRefresh: refresh
    }));
    const draft = intervention.draft;
    if (draft?.operation === TASK_OPERATION && draft.target === item.id && draft.subject === item.assignment_digest) {
      const confirmation = node("section", "intervention-panel"); confirmation.id = "task-reassignment-confirmation"; confirmation.tabIndex = -1;
      confirmation.setAttribute("role", "group"); confirmation.setAttribute("aria-label", "Confirm Task reassignment");
      confirmation.append(node("h3", "", "Pass this responsibility"), node("p", "outcome-value", draft.from + " → " + draft.to), node("p", "", "The same Task remains open. Its obligation, bound acceptance requirements, and recorded history remain attached."));
      const confirm = button("Confirm reassignment", () => submitIntervention(), "button");
      const discard = button("Keep current assignee", () => { intervention.draft = null; intervention.phase = "idle"; intervention.error = ""; render(); });
      confirm.disabled = Boolean(intervention.reserving) || !currentDraftEligible(draft); discard.disabled = Boolean(intervention.reserving);
      confirmation.append(append(node("div", "intervention-actions"), confirm, discard));
      if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); confirmation.append(error); }
      detail.append(confirmation);
    }
    return detail;
  }
  function renderTaskRecovery() {
    const metadata = intervention.metadata, result = intervention.receipt;
    const panel = node("section", "intervention-panel"); panel.id = "command-recovery"; panel.tabIndex = -1;
    panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Task reassignment request");
    panel.append(node("h3", "", "Task reassignment"));
    if (result) {
      const r = result.receipt, task = state.route.view === "tasks" && state.detail?.id === metadata.target_id ? state.detail : null;
      const observed = r.task.state === "applied" && task?.history.some(event => event.event_id === r.task.event_id && event.kind === "task.reassigned" && event.from === r.task.from && event.to === r.task.to && event.by === metadata.principal.name);
      panel.dataset.observation = observed ? "observed" : "unavailable";
      panel.append(renderCommandStages([
        { key: "command", title: "Request", value: r.state === "succeeded" ? "Recorded" : "Unconfirmed", tone: r.state === "succeeded" ? "confirmed" : "unknown", explanation: r.state === "succeeded" ? "The native service recorded this exact reassignment once." : "Keep this request identity and check its status. No replacement request is sent automatically." },
        { key: "assignment", title: "Assignment", value: r.task.from + " → " + r.task.to, tone: r.task.state === "applied" ? "confirmed" : "unknown", explanation: "Changing the assignee preserves the Task and its original obligation. It does not record completion." },
        { key: "current", title: "Current Task", value: observed ? (task.state === "handed" ? "Open · " + (task.assignee || "unassigned") : task.state) : "Refresh to inspect", tone: observed ? "confirmed" : "unknown", explanation: observed ? "This captured Task history contains the exact reassignment event. Any later assignment or settlement remains visible in the same history." : "The command receipt and the Task read are separate evidence. Open or refresh the Task to verify its current responsibility." }
      ], r));
    } else panel.append(node("p", "detail-note", intervention.phase === "submitting" ? "Recording the reassignment…" : intervention.phase === "recovering" ? "Checking the saved request…" : "The request outcome is not yet confirmed."));
    if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); panel.append(error); }
    const actions = node("div", "intervention-actions");
    const check = button("Check request status", checkCommandStatus); check.id = "command-check-status"; check.disabled = Boolean(commandController) || !recoveryCapability().supported;
    actions.append(check, navigationLink("Open current Task", routeHash(workspaceRoute("tasks", { app: metadata.application_id, id: metadata.target_id })), "detail"));
    if (result && COMMAND_TERMINAL.has(result.receipt.state)) actions.append(button("Dismiss completed request", dismissCompletedRequest));
    panel.append(actions);
    const evidence = node("details", "intervention-evidence"); evidence.append(node("summary", "", "Request evidence"));
    const facts = node("dl", "fact-grid"); fact(facts, "Task", metadata.target_id, true, true); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Inspected Task basis", metadata.subject_digest, true, true);
    if (result) { fact(facts, "Command identity", result.receipt.command_id, true, true); fact(facts, "Record head", result.source.record_head, true, true); if (result.receipt.task.event_id) fact(facts, "Reassignment event", result.receipt.task.event_id, true, true); }
    evidence.append(facts); panel.append(evidence); return panel;
  }
  // Raising work: the ask is recorded at once; the organism's answer arrives
  // through GET lookup, re-derived from the Record on every check.
  function prepareTaskCreate(ask) {
    if (state.phase !== "ready" || state.route.view !== "tasks" || !state.app || !commandCapability(state.capabilities, TASK_CREATE_OPERATION).allowed || intervention.metadata || intervention.blocked || intervention.draft || !commandID(state.source?.record_head) || !window.FaceTaskCreate) throw new Error("Reload the handed Tasks and check the current authority before preparing a new task.");
    const exact = window.FaceTaskCreate.validate(ask);
    intervention.draft = { operation: TASK_CREATE_OPERATION, target: state.app.id, subject: state.source.record_head, record_head: state.source.record_head, outcome: exact.outcome, to: exact.to };
    intervention.phase = "reviewing"; intervention.error = "";
    render(); $("task-create-confirmation")?.focus();
  }
  function taskCreatePanel() {
    const capability = commandCapability(state.capabilities, TASK_CREATE_OPERATION);
    if (!capability.supported || !window.FaceTaskCreate) return null;
    const frame = node("div", "task-create-frame");
    frame.append(window.FaceTaskCreate.render({
      canCreate: capability.allowed && !intervention.metadata && !intervention.blocked && !intervention.draft,
      positions: state.workingContext?.available ? state.workingContext.positions : [], defaultTo: state.route.locus || "",
      reason: !capability.allowed ? "Raising work is unavailable for this connection or signed-in principal. Viewing a locus does not grant it." : intervention.metadata ? "Check the saved request before raising another task." : intervention.blocked ? intervention.error : intervention.draft ? "Confirm or discard the prepared task first." : "",
      onPrepare: ask => prepareTaskCreate(ask)
    }));
    const draft = intervention.draft;
    if (draft?.operation === TASK_CREATE_OPERATION) {
      const confirmation = node("section", "intervention-panel"); confirmation.id = "task-create-confirmation"; confirmation.tabIndex = -1;
      confirmation.setAttribute("role", "group"); confirmation.setAttribute("aria-label", "Confirm new task");
      confirmation.append(node("h3", "", "Raise this task"), node("p", "outcome-value task-literal", draft.outcome), node("p", "", "For " + (draft.to === "org" ? "the whole organization" : draft.to) + " · asked by " + state.capabilities.principal.name + ". The organism decides whether to admit it; its answer is recorded separately."));
      const confirm = button("Confirm new task", () => submitIntervention(), "button");
      const discard = button("Discard", () => { intervention.draft = null; intervention.phase = "idle"; intervention.error = ""; render(); });
      confirm.disabled = Boolean(intervention.reserving) || !currentDraftEligible(draft); discard.disabled = Boolean(intervention.reserving);
      confirmation.append(append(node("div", "intervention-actions"), confirm, discard));
      if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); confirmation.append(error); }
      frame.append(confirmation);
    }
    return frame;
  }
  function renderTaskCreateRecovery() {
    const metadata = intervention.metadata, result = intervention.receipt;
    const panel = node("section", "intervention-panel task-create-recovery"); panel.id = "command-recovery"; panel.tabIndex = -1;
    panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "New task request");
    panel.append(node("h3", "", "New task"));
    if (result) {
      const r = result.receipt, t = r.task_create;
      panel.dataset.intentState = t.intent_state;
      panel.append(renderCommandStages([
        { key: "command", title: "Request", value: r.state === "succeeded" ? "Recorded" : "Unconfirmed", tone: r.state === "succeeded" ? "confirmed" : "unknown", explanation: r.state === "succeeded" ? "The service recorded this ask once, in your name, at the Record head you prepared it against." : "Keep this request identity and check its status. No replacement request is sent automatically." },
        { key: "intent", title: "Intent", value: t.intent_state === "unknown" ? "Not established" : t.intent_id + " · " + t.intent_state, tone: t.intent_state === "refused" ? "refused" : t.intent_state === "born" || t.intent_state === "offered" ? "confirmed" : t.intent_state === "requested" ? "pending" : "unknown", explanation: t.intent_state === "requested" ? "The ask is in the Record. The host beside the organism relays it; the organism's answer is a later fact." : t.intent_state === "offered" ? "The organism admitted the ask. Its Task is minted next." : t.intent_state === "refused" ? "The organism refused this ask; its reason is in the Record. This request is complete." : t.intent_state === "born" ? "The organism admitted the ask and minted its Task." : "No intent is named for an unconfirmed ask." },
        { key: "task", title: "Task", value: t.task_id || (t.intent_state === "refused" ? "None" : "Not yet born"), tone: t.task_id ? "confirmed" : t.intent_state === "refused" ? "refused" : "pending", explanation: t.task_id ? "The Task exists. It joins the handed Tasks once the leader hands it to a person." : "Check again to follow the ask through offer and birth. A born Task is named here before it is handed." }
      ], r));
    } else panel.append(node("p", "detail-note", intervention.phase === "submitting" ? "Recording the ask…" : intervention.phase === "recovering" ? "Checking the saved request…" : "The request outcome is not yet confirmed."));
    if (intervention.error) { const error = node("p", "intervention-error", intervention.error); error.setAttribute("role", "alert"); panel.append(error); }
    const actions = node("div", "intervention-actions");
    const check = button("Check request status", checkCommandStatus); check.id = "command-check-status"; check.disabled = Boolean(commandController) || !recoveryCapability().supported;
    actions.append(check);
    if (result && COMMAND_TERMINAL.has(result.receipt.state)) actions.append(button("Dismiss completed request", dismissCompletedRequest));
    panel.append(actions);
    const evidence = node("details", "intervention-evidence"); evidence.append(node("summary", "", "Request evidence"));
    const facts = node("dl", "fact-grid"); fact(facts, "Request identity", metadata.request_id, true, true); fact(facts, "Prepared against Record head", metadata.subject_digest, true, true);
    if (result) {
      const t = result.receipt.task_create;
      fact(facts, "Command identity", result.receipt.command_id, true, true); fact(facts, "Record head", result.source.record_head, true, true);
      if (t.intent_id) fact(facts, "Intent", t.intent_id, true, true); if (t.event_id) fact(facts, "Ask event", t.event_id, true, true); if (t.task_id) fact(facts, "Task", t.task_id, true, true);
    }
    evidence.append(facts); panel.append(evidence); return panel;
  }
  function validWorkflow(item) {
    assert(["wf1", "unclassified"].includes(item.engine) && decimal(item.revision) && ["admitted", "done", "failed", "cancelled", "refused", "unclassified"].includes(item.state));
    assert(Array.isArray(item.nodes) && item.nodes.length <= 256 && Array.isArray(item.attempts) && item.attempts.length <= 512 && Array.isArray(item.history) && item.history.length <= 512);
    const ids = new Map();
    for (const n of item.nodes) {
      assert(n && ["id", "kind", "parent_id", "spawning_step", "member_key", "definition_id", "revision", "step_index", "state", "objective", "target", "context_digest", "knowledge_bindings", "output_contract", "requires", "data_class", "cost_ceiling", "allowance", "attempt_id", "attempt_no", "result", "result_ref", "pending", "failed_member", "reason", "admission_stop"].every(key => typeof n[key] === "string") && n.id && !ids.has(n.id));
      assert(["task", "step", "work"].includes(n.kind) && Object.hasOwn(WORK_STATES, n.state) && typeof n.joined === "boolean" && ["revision", "step_index", "cost_ceiling", "allowance"].every(key => decimal(n[key])) && decimal(n.attempt_no, true));
      ids.set(n.id, n);
    }
    if (ids.size) assert(ids.get(item.id)?.kind === "task" && ids.get(item.id).parent_id === "", "The execution root is missing from its recipe.");
    for (const n of item.nodes) {
      assert(n.id === item.id || ids.has(n.parent_id), "A recipe node has no declared parent.");
      const parent = ids.get(n.parent_id);
      if (parent) assert(parent.kind === (n.kind === "work" ? "step" : "task"));
      if (n.kind === "task" && n.id !== item.id) assert(ids.get(n.spawning_step)?.kind === "step" && ids.get(n.spawning_step).parent_id === n.parent_id);
      let at = n, seen = new Set();
      while (at) { assert(!seen.has(at.id), "The execution recipe contains a parent cycle."); seen.add(at.id); at = ids.get(at.parent_id); }
    }
    const attempts = new Set();
    for (const a of item.attempts) {
      assert(a && ["id", "work_id", "number", "performer_kind", "disposition", "performer", "result", "result_ref", "evidence_ref", "narrative"].every(key => typeof a[key] === "string") && a.id && !attempts.has(a.id) && ids.get(a.work_id)?.kind === "work" && decimal(a.number) && typeof a.accepted === "boolean" && ["outstanding", "done", "failed", "declined", "timeout"].includes(a.disposition));
      if (a.accepted) assert(a.disposition === "done" && ids.get(a.work_id).state === "done" && ids.get(a.work_id).attempt_id === a.id);
      attempts.add(a.id);
    }
    for (const fact of item.history) assert(fact && ["kind", "id", "node_id", "order"].every(key => typeof fact[key] === "string") && decimal(fact.order) && (ids.has(fact.node_id) || fact.node_id === item.id));
  }
  function workflowMeta(item) { return append(node("div", "record-meta"), node("div", "", item.engine === "wf1" ? item.definition_id + "@" + item.revision : "Task birth · execution engine unclassified")); }
  function workflowDestination(id) { return routeHash({ ...state.route, worknode: id, workattempt: "" }); }
  function workflowNavigate(label, id, className = "text-link") { return navigationLink(label, workflowDestination(id), "execution", className); }
  function workflowMembers(item, step) { return item.nodes.filter(n => n.kind === "work" ? n.parent_id === step.id : n.kind === "task" && n.spawning_step === step.id); }
  function workflowBarrier(step, members) {
    const barrier = node("div", "execution-barrier");
    barrier.dataset.state = step.state;
    if (step.pending) {
      barrier.append(node("p", "", "Completion not recorded for · " + step.pending));
      const links = node("div", "execution-barrier-members");
      for (const key of new Set(step.pending.split(" ").filter(Boolean))) {
        const matches = members.filter(m => m.member_key === key);
        if (matches.length === 1) {
          const action = workflowNavigate(key + " ↗", matches[0].id);
          action.setAttribute("aria-label", "Follow member " + key);
          links.append(action);
        } else links.append(node("span", "", key + " · exact member unavailable"));
      }
      barrier.append(links);
    }
    if (step.joined && step.state !== "completed") barrier.append(node("p", "", "All registered members settled done; Step completion has not been recorded."));
    if (step.state === "completed") barrier.append(node("p", "", "Committed Step completion recorded."));
    if (step.failed_member) {
      const failed = node("p", "", "Failed member · " + step.failed_member);
      const matches = members.filter(m => m.member_key === step.failed_member);
      if (matches.length === 1) failed.append(" · ", workflowNavigate("Inspect failed member", matches[0].id));
      barrier.append(failed);
    }
    if (!barrier.childElementCount) barrier.append(node("p", "", step.state === "bound" ? "Step is bound; activation has not been recorded." : "No completion barrier fact supplied."));
    return barrier;
  }
  function workflowIdentity(label, value) {
    return append(node("details", "execution-identity"), node("summary", "", label), node("code", "", value));
  }
  function workflowDetail(item) {
    const detail = node("article", "detail execution-detail");
    append(detail, backToList(), append(node("div", "detail-kicker"), node("span", "eyebrow", item.engine === "wf1" ? "Recorded execution" : "Task birth"), workflowBadge(item.state)), node("h2", "", item.definition_id ? item.definition_id + "@" + item.revision : item.id), workflowIdentity("Exact execution identity", item.id));
    if (item.reason) detail.append(documentText(item.reason));
    if (item.engine === "unclassified") { detail.append(node("p", "detail-note", "A task birth is recorded without a wf1 admission or refusal. Its execution engine and state are not established by this projection; no Steps, attempts or completion are inferred.")); return detail; }
    if (!item.nodes.length) { detail.append(node("p", "detail-note", "Admission was refused. No bound execution or started work is asserted.")); return detail; }
    const byId = new Map(item.nodes.map(n => [n.id, n]));
    const selectedId = state.route.worknode || item.id, selected = byId.get(selectedId);
    if (!selected) { detail.append(stateCard("Execution node unavailable", "This exact identity is not in the captured recipe.", "◇", [workflowNavigate("Return to execution root", item.id)], "compact")); return detail; }
    const task = selected.kind === "task" ? selected : selected.kind === "step" ? byId.get(selected.parent_id) : byId.get(byId.get(selected.parent_id).parent_id);
    const ancestors = []; let up = task;
    while (up) { ancestors.unshift(up); up = byId.get(up.parent_id); }
    const breadcrumb = node("nav", "execution-breadcrumbs"); breadcrumb.setAttribute("aria-label", "Execution ancestry");
    for (const ancestor of ancestors) {
      if (breadcrumb.childElementCount) { const separator = node("span", "", "/"); separator.setAttribute("aria-hidden", "true"); breadcrumb.append(separator); }
      const action = workflowNavigate(ancestor.definition_id + "@" + ancestor.revision, ancestor.id);
      action.setAttribute("aria-label", ancestor.definition_id + "@" + ancestor.revision + " · " + ancestor.id); action.title = ancestor.id;
      if (ancestor.id === task.id) action.setAttribute("aria-current", "location");
      breadcrumb.append(action);
    }
    detail.append(breadcrumb, append(node("header", "execution-scope-heading"), node("h3", "", task.definition_id + "@" + task.revision), workflowBadge(task.state)));
    if (state.capabilities?.reads?.definitions === true) detail.append(link("Inspect exact definition " + task.definition_id + "@" + task.revision, routeHash(workspaceRoute("definitions", { id: task.definition_id + "@" + task.revision })), "text-link"));
    if (task.spawning_step) detail.append(workflowNavigate("Return to spawning Step", task.spawning_step, "text-link execution-return"));
    if (task.admission_stop) detail.append(node("p", "notice execution-stop", "Admission stopped · " + task.admission_stop + ". Already admitted responsibilities retain their recorded outcomes below."));
    if (task.id !== item.id && task.reason) detail.append(documentText(task.reason));
    if (task.state === "bound") detail.append(node("p", "detail-note", "This child recipe was bound by its parent. The child task has not been admitted."));
    const steps = item.nodes.filter(n => n.kind === "step" && n.parent_id === task.id).sort((a,b) => BigInt(a.step_index) < BigInt(b.step_index) ? -1 : 1);
    const sequence = node("ol", "execution-steps"); sequence.setAttribute("aria-label", "Ordered execution Steps"); sequence.id = "execution-steps"; sequence.tabIndex = -1;
    for (const step of steps) {
      const isSelected = selected.id === step.id || selected.parent_id === step.id;
      const card = node("li", "execution-step" + (isSelected ? " selected" : "")); card.dataset.state = step.state; card.dataset.id = step.id;
      const stepLink = workflowNavigate("Step " + (BigInt(step.step_index) + 1n), step.id);
      if (selected.id === step.id) stepLink.setAttribute("aria-current", "true");
      append(card, append(node("header", "execution-step-heading"), stepLink, workflowBadge(step.state)));
      const members = workflowMembers(item, step);
      const list = node("ul", "execution-members");
      for (const member of members) {
        const entry = node("li", "execution-member" + (member.id === selected.id ? " selected" : ""));
        entry.dataset.id = member.id;
        const title = member.kind === "task" ? member.definition_id + "@" + member.revision : member.objective || member.member_key;
        const action = workflowNavigate("", member.id, "execution-member-link");
        action.setAttribute("aria-label", (member.kind === "task" ? "Enter child " : "Inspect work ") + member.id);
        if (member.id === selected.id) action.setAttribute("aria-current", "true");
        append(action, node("span", "eyebrow", member.kind === "task" ? "Child task · " + member.member_key : "Work · " + member.member_key), node("strong", "", title));
        append(entry, action, workflowBadge(member.state));
        if (member.target) entry.append(node("span", "execution-target", member.target));
        list.append(entry);
      }
      card.append(list);
      card.append(workflowBarrier(step, members));
      sequence.append(card);
    }
    let inspector;
    if (selected.kind === "work") inspector = workflowWork(selected, item.attempts.filter(a => a.work_id === selected.id));
    else {
      inspector = node("section", "execution-inspector");
      inspector.setAttribute("aria-label", selected.kind === "step" ? "Selected Step" : "Selected workflow");
      append(inspector, node("span", "eyebrow", selected.kind === "step" ? "Completion barrier" : "Inside this workflow"), node("h3", "", selected.kind === "step" ? definitionStepLabel(selected.step_index) : "Explore this workflow"), workflowBadge(selected.state));
      if (selected.kind === "step") {
        inspector.append(workflowBarrier(selected, workflowMembers(item, selected)), node("p", "detail-note", "A member outcome and the committed Step barrier are separate facts."));
        const members = node("ul", "execution-member-index");
        for (const member of workflowMembers(item, selected)) members.append(append(node("li"), workflowNavigate(member.member_key + " · " + (member.objective || member.definition_id), member.id), workflowBadge(member.state)));
        inspector.append(members);
      } else {
        inspector.append(node("p", "detail-note", "Follow a Step's completion barrier to see its member outcomes. Enter a child task to explore the workflow inside it."));
        const index = node("ol", "execution-step-index");
        for (const step of steps) index.append(append(node("li"), workflowNavigate(definitionStepLabel(step.step_index), step.id), workflowBadge(step.state), node("p", "", step.pending ? "Completion not recorded for · " + step.pending : step.joined ? "Members settled done" : "No joined completion recorded")));
        inspector.append(index);
      }
      if (selected.kind === "step" && selected.reason) inspector.append(documentText(selected.reason));
      inspector.append(workflowIdentity("Exact selected identity", selected.id));
    }
    inspector.id = "execution-focus-panel"; inspector.tabIndex = -1;
    const controls = node("div", "execution-context-actions");
    controls.append(button("Back to workflow", () => focusPanel("execution-map")));
    if (selected.kind === "work") controls.append(workflowNavigate("Inspect containing Step", selected.parent_id));
    inspector.prepend(controls);
    detail.append(append(node("div", "execution-stage"), sequence, inspector));
    const history = node("details", "execution-history"); history.append(node("summary", "", "Recorded transition history · " + item.history.length));
    const events = node("ol");
    for (const fact of item.history) events.append(append(node("li"), node("span", "", fact.kind), byId.has(fact.node_id) ? workflowNavigate(fact.id, fact.node_id) : node("code", "", fact.id)));
    history.append(events, node("p", "detail-note", "Native projection order. No timestamps or runtime association are supplied by these facts."));
    detail.append(history, node("p", "detail-note", "Inspection does not launch, retry, cancel or settle work. The execution service owns those operations."));
    return detail;
  }
  function workflowWork(work, attempts) {
    const panel = node("section", "execution-inspector"); panel.setAttribute("aria-label", "Work and attempt results");
    append(panel, node("span", "eyebrow", "Selected Work · " + work.member_key), node("h3", "", work.objective || work.id), workflowBadge(work.state));
    if (work.state === "done") panel.append(append(node("div", "execution-accepted"), section("Accepted Work result", documentText(work.result || "No inline result supplied.")), work.result_ref ? node("p", "mono", work.result_ref) : null));
    else panel.append(node("p", "detail-note", "No accepted Work result is recorded."));
    if (work.reason) panel.append(documentText(work.reason));
    const facts = node("dl", "fact-grid");
    for (const [label, value] of [["Target", work.target], ["Requirements", work.requires], ["Output contract", work.output_contract], ["Attempt allowance", work.allowance], ["Current attempt", work.attempt_id], ["Context digest", work.context_digest]]) fact(facts, label, display(value, "Not supplied"), true);
    const specification = append(node("details", "execution-specification"), node("summary", "", "Bound specification and exact identity"), node("code", "", work.id), facts);
    if (work.knowledge_bindings) specification.append(section("Exact bound knowledge specification", documentText(work.knowledge_bindings)), node("p", "detail-note", "The binding specification is retained as supplied. It is not reinterpreted against today's graph."));
    const selectedId = state.route.workattempt || work.attempt_id;
    const selected = attempts.find(a => a.id === selectedId);
    const history = node("div", "attempt-history"); history.setAttribute("aria-label", "Attempt history");
    history.append(node("h4", "", "Recorded attempts"));
    for (const a of attempts) {
      const card = node("article", "attempt-card" + (a.id === selected?.id ? " selected" : "")); card.dataset.accepted = String(a.accepted);
      const action = navigationLink("Attempt " + a.number + " · " + a.disposition, routeHash({ ...state.route, worknode: work.id, workattempt: a.id }), "attempt", "attempt-link");
      action.setAttribute("aria-label", "Inspect attempt " + a.id + " · Attempt " + a.number + " · " + a.disposition);
      if (a.id === selected?.id) action.setAttribute("aria-current", "true");
      append(card, node("h4")); card.firstChild.append(action);
      append(card, node("p", "", "Performer kind · " + a.performer_kind), node("p", "", a.accepted ? "Accepted by the Work" : "Not an accepted Work result"));
      history.append(card);
    }
    if (!attempts.length) history.append(node("p", "detail-note", "No attempt has been admitted for this Work."));
    const evidence = node("section", "execution-attempt"); evidence.id = "execution-attempt-panel"; evidence.tabIndex = -1; evidence.setAttribute("aria-label", "Selected attempt");
    if (selected) {
      append(evidence, node("h4", "", "Attempt " + selected.number + " · " + selected.disposition), node("p", "", selected.accepted ? "Accepted by the Work" : "Not an accepted Work result"), node("code", "", selected.id));
      if (selected.performer) evidence.append(node("p", "", "Recorded performer · " + selected.performer));
      if (selected.result) evidence.append(documentText(selected.result));
      if (selected.narrative) evidence.append(documentText(selected.narrative));
      if (selected.result_ref) evidence.append(node("p", "mono", "Result reference · " + selected.result_ref));
      if (selected.evidence_ref) evidence.append(node("p", "mono", "Evidence reference · " + selected.evidence_ref));
      if (selected.disposition === "outstanding") evidence.append(node("p", "detail-note", "An admitted attempt is outstanding. This record does not supply an actionable human obligation."));
    } else evidence.append(node("p", "detail-note", selectedId ? "Attempt unavailable. This exact identity is not among the returned attempts for this Work." : "Select a recorded attempt to inspect its result and evidence."));
    panel.append(history, evidence, specification); return panel;
  }
  function knowledgeName(item) { return display(item.name, "Unnamed knowledge item"); }
  function knowledgeBadge(item) { return badge(display(item.projection_state, "State not recorded"), item.projection_state === "ratified" ? "green" : item.projection_state === "proposed" ? "amber" : ""); }
  function knowledgeMeta(item) {
    return append(node("div", "record-meta"), node("div", "", display(item.kind, "Kind not recorded") + " · revision " + item.revision), node("code", "", short(item.id, 30)));
  }
  function recordLink(item) {
    const workspace = WORKSPACES[state.route.view];
    const href = routeHash({ ...state.route, id: item.id, worknode: "", workattempt: "", snapshot: state.collection.page.snapshot, offset: state.collection.page.offset, edges_cursor: "", bindings_cursor: "" });
    const a = navigationLink("", href, "detail", "record-link");
    a.setAttribute("aria-label", workspace.rowName(item) + (["definitions", "knowledge"].includes(state.route.view) ? " · " + item.id : ""));
    if (state.route.id === item.id) a.setAttribute("aria-current", "true");
    const matches = targetMatchesContext(item);
    a.classList.toggle("context-target-match", matches);
    const meta = workspace.rowMeta(item);
    if (matches) meta.append(badge("Exact context target", "green"));
    return append(a, append(node("div", "record-line"), node("span", "record-name", workspace.rowName(item)), workspace.badge(item)), meta);
  }
  function organizationRows() {
    const { items, basis } = state.collection;
    const positionsOnly = basis.position_group_declared && state.route.scope !== "all";
    const rows = positionsOnly ? items.filter(item => item.in_position_outline) : items;
    if (!state.route.branch) return rows;
    const root = state.organizationBranch;
    return root ? [root, ...rows.filter(item => item.id !== root.id && item.parent_id === root.id)] : [];
  }
  function organizationBranchRoute(id) {
    return routeHash({ ...state.route, branch: id, id: id || state.route.id, snapshot: state.collection.page.snapshot });
  }
  function organizationBranchContext(rows) {
    const fragment = document.createDocumentFragment();
    const path = node("nav", "organization-branch-path");
    path.setAttribute("aria-label", "Organization branch");
    path.append(navigationLink("Whole organization", organizationBranchRoute(""), "list"));
    const addCrumb = crumb => {
      const separator = node("span", "organization-branch-separator", "/");
      separator.setAttribute("aria-hidden", "true");
      path.append(separator, crumb);
    };
    const root = state.organizationBranch;
    if (!root) {
      append(fragment, path, stateCard("Organization branch not found", "The requested branch could not be read in this source snapshot: " + state.route.branch + ". Return to the whole organization to choose an available instance.", "◇", [], "compact"));
      return fragment;
    }
    const byID = new Map(state.collection.items.map(item => [item.id, item]));
    if (state.detail) byID.set(state.detail.id, state.detail);
    byID.set(root.id, root);
    const chain = [], seen = new Set();
    let current = root, boundary = "";
    while (current && !seen.has(current.id)) {
      chain.unshift(current); seen.add(current.id);
      if (current.parent_id && !byID.has(current.parent_id)) boundary = current.parent_id;
      current = byID.get(current.parent_id);
    }
    if (boundary) addCrumb(navigationLink("Parent outside this page · " + boundary, relatedRoute("organization", boundary), "detail"));
    for (const item of chain) {
      const crumb = item.id === root.id ? node("span", "", item.id) : navigationLink(item.id, organizationBranchRoute(item.id), "list");
      if (item.id === root.id) crumb.setAttribute("aria-current", "page");
      addCrumb(crumb);
    }
    if (current) path.append(node("span", "", "Unresolved parent cycle"));
    const context = node("section", "organization-branch-context");
    context.setAttribute("aria-label", "Branch context");
    const immediate = state.collection.items.filter(item => item.id !== root.id && item.parent_id === root.id);
    const complete = state.collection.page.offset === 0 && state.collection.items.length === state.collection.page.total;
    append(context, node("span", "eyebrow", "Inside this branch"), node("h3", "", root.id.slice(root.id.lastIndexOf(".") + 1)), node("p", "", (rows.length - 1) + " immediate " + (rows.length === 2 ? "child" : "children") + " on this page · " + root.declaration));
    if (!state.collection.items.some(item => item.id === root.id)) context.append(node("p", "detail-note", "Branch root read by identity; children are limited to this page."));
    if (immediate.length > rows.length - 1) context.append(node("p", "detail-note", (immediate.length - rows.length + 1) + " structural children on this page are outside the positions outline. Choose All structure to include them."));
    if (!immediate.length) context.append(node("p", "detail-note", complete ? "No immediate children are declared in this complete source snapshot." : "No immediate children on this page. Other pages may contain children of this branch."));
    context.append(node("p", "detail-note", "Source-declared containment · viewing only."));
    append(fragment, path, context);
    return fragment;
  }
  function organizationOutline() {
    const { items, page, basis } = state.collection;
    const positionsOnly = basis.position_group_declared && state.route.scope !== "all";
    const rows = organizationRows();
    const fragment = document.createDocumentFragment();
    const controls = node("div", "outline-controls");
    controls.setAttribute("role", "group");
    controls.setAttribute("aria-label", "Outline scope");
    const positionButton = button("Declared positions", () => navigate({ ...state.route, scope: "positions" }), "scope-button");
    positionButton.disabled = !basis.position_group_declared;
    positionButton.setAttribute("aria-pressed", String(positionsOnly));
    const allButton = button("All structure", () => navigate({ ...state.route, scope: "all" }), "scope-button");
    allButton.setAttribute("aria-pressed", String(!positionsOnly));
    append(controls, positionButton, allButton);
    const toolbar = node("div", "topology-toolbar");
    const presentations = node("div", "topology-tools");
    presentations.setAttribute("role", "group");
    presentations.setAttribute("aria-label", "Organization presentation");
    for (const [value, label] of [["topology", "Topology"], ["outline", "Outline"]]) {
      const toggle = button(label, () => {
        organizationPresentation = value;
        render();
        $("presentation-" + value)?.focus();
      }, "scope-button");
      toggle.id = "presentation-" + value;
      toggle.setAttribute("aria-pressed", String(organizationPresentation === value));
      presentations.append(toggle);
    }
    append(toolbar, controls, presentations);
    fragment.append(toolbar);
    if (state.route.branch) fragment.append(organizationBranchContext(rows));
    const explanation = !basis.position_group_declared ? "No positions group is declared in this source. Showing all static structure." : positionsOnly ? "Explicitly declared positions and their containment ancestors." : "All static instances, including structure outside the positions group.";
    if (!state.route.branch) fragment.append(node("p", "outline-note", explanation));
    if (page.total > items.length) fragment.append(node("p", "outline-note", "This outline covers the current page. A parent on another page is shown as a reference, not as a new root position."));
    if (state.route.branch && !state.organizationBranch) return fragment;
    if (!rows.length) {
      fragment.append(stateCard("No positions on this page", "This page contains no declared positions or their ancestors. Choose All structure or continue to another page.", "◇", [], "compact"));
      return fragment;
    }
    if (organizationPresentation === "topology") {
      fragment.append(organizationTopology(rows));
      return fragment;
    }
    const byID = new Map(rows.map((item) => [item.id, item]));
    const children = new Map();
    for (const item of rows) {
      if (byID.has(item.parent_id) && item.parent_id !== item.id) {
        if (!children.has(item.parent_id)) children.set(item.parent_id, []);
        children.get(item.parent_id).push(item);
      }
    }
    const seen = new Set();
    function branch(item) {
      seen.add(item.id);
      const li = append(node("li"), recordLink(item));
      if (item.parent_id && !byID.has(item.parent_id) && item.id !== state.route.branch) {
        const reference = navigationLink("Parent · " + item.parent_id, relatedRoute("organization", item.parent_id), "detail", "outline-parent");
        li.append(reference);
      }
      const nested = node("ul", "structure-children");
      for (const child of children.get(item.id) || []) if (!seen.has(child.id)) nested.append(branch(child));
      if (nested.childNodes.length) li.append(nested);
      return li;
    }
    const list = node("ul", "record-list structure-list");
    for (const item of rows) if (!byID.has(item.parent_id) || item.parent_id === item.id) list.append(branch(item));
    // A malformed/cyclic relationship must not hide an instance or recurse
    // forever. The remaining rows have no hierarchy asserted by this view.
    const unarranged = rows.filter((item) => !seen.has(item.id));
    if (unarranged.length) {
      fragment.append(node("p", "outline-note", "Some containment links could not be arranged. Remaining instances are listed without a hierarchy."));
      for (const item of unarranged) if (!seen.has(item.id)) list.append(append(node("li"), recordLink(item)));
    }
    fragment.append(list);
    return fragment;
  }
  function organizationTopology(rows) {
    const frame = node("section", "topology-panel" + (state.route.branch ? " topology-scoped" : ""));
    frame.setAttribute("aria-label", "Declared containment topology");
    const viewport = node("div", "topology-viewport");
    viewport.tabIndex = 0;
    viewport.setAttribute("aria-label", "Organization canvas. Scroll to explore declared containment.");
    const stage = node("div", "topology-stage");
    stage.style.zoom = String(topologyScale);
    const byID = new Map(rows.map((item) => [item.id, item]));
    const children = new Map();
    for (const item of rows) if (byID.has(item.parent_id) && item.parent_id !== item.id) {
      if (!children.has(item.parent_id)) children.set(item.parent_id, []);
      children.get(item.parent_id).push(item);
    }
    const seen = new Set();
    function membrane(item) {
      const a = navigationLink("", relatedRoute("organization", item.id), "detail", "topology-node");
      a.setAttribute("aria-label", item.id);
      a.dataset.instanceId = item.id;
      a.dataset.role = item.role;
      if (item.id === state.route.branch) a.dataset.branchRoot = "true";
      if (state.route.id === item.id) a.setAttribute("aria-current", "true");
      const label = item.id.slice(item.id.lastIndexOf(".") + 1);
      append(a, node("span", "topology-node-kind", item.role === "position" ? "Declared position" : "Structural instance"), node("strong", "topology-node-label", label), node("code", "topology-node-path", item.id), node("span", "topology-node-kind", item.declaration));
      if (state.route.branch) {
        const count = state.collection.items.filter(row => row.parent_id === item.id && row.id !== item.id).length;
        if (count) a.append(node("span", "topology-node-children", count + " immediate " + (count === 1 ? "child" : "children") + " on page"));
      }
      if (item.parent_id) {
        const parent = node("span", "sr-only", "Declared parent: " + item.parent_id);
        parent.id = "topology-parent-" + rows.indexOf(item);
        a.setAttribute("aria-describedby", parent.id);
        a.append(parent);
      }
      return a;
    }
    function branch(item, parent = "") {
      seen.add(item.id);
      const li = node("li", "topology-branch");
      if (parent) { li.dataset.edgeFrom = parent; li.dataset.edgeTo = item.id; }
      const container = node("div", "topology-node-wrap");
      if (item.parent_id && !byID.has(item.parent_id) && item.id !== state.route.branch) {
        const reference = navigationLink("Parent outside page · " + item.parent_id, relatedRoute("organization", item.parent_id), "detail", "topology-parent-reference");
        container.append(reference);
      }
      container.append(membrane(item));
      li.append(container);
      const nested = node("ul", "topology-children");
      for (const child of children.get(item.id) || []) if (!seen.has(child.id)) nested.append(branch(child, item.id));
      if (nested.childNodes.length) li.append(nested);
      return li;
    }
    const tree = node("ul", "topology-tree");
    for (const item of rows) if (!byID.has(item.parent_id)) tree.append(branch(item));
    const unresolved = rows.filter((item) => !seen.has(item.id));
    if (unresolved.length) {
      stage.append(node("p", "topology-empty", "Some containment links could not be arranged. These instances are shown without connecting lines."));
      for (const item of unresolved) tree.append(append(node("li", "topology-branch topology-unresolved"), membrane(item)));
    }
    stage.append(tree);
    viewport.append(stage);
    const tools = node("div", "topology-tools");
    const percentage = node("output", "topology-scale", Math.round(topologyScale * 100) + "%");
    percentage.setAttribute("aria-label", "Topology zoom");
    const changeScale = (scale) => {
      topologyScale = Math.max(.5, Math.min(1.5, scale));
      stage.style.zoom = String(topologyScale);
      percentage.textContent = Math.round(topologyScale * 100) + "%";
    };
    const zoomOut = button("−", () => changeScale(topologyScale - .1), "scope-button");
    zoomOut.setAttribute("aria-label", "Zoom out topology");
    const zoomIn = button("+", () => changeScale(topologyScale + .1), "scope-button");
    zoomIn.setAttribute("aria-label", "Zoom in topology");
    const fit = button("Fit", () => {
      stage.style.zoom = "1";
      changeScale(Math.min(1, (viewport.clientWidth - 32) / stage.scrollWidth));
      viewport.scrollTo({ left: 0, top: 0 });
    }, "scope-button");
    fit.setAttribute("aria-label", "Fit topology");
    append(tools, zoomOut, percentage, zoomIn, fit);
    const legend = append(node("div", "topology-legend"), node("span", "topology-legend-edge", "Declared containment"), node("span", "", state.route.branch ? "Branch root + " + (rows.length - 1) + " immediate children" : rows.length + " instances on page"), tools);
    append(frame, viewport, legend);
    return frame;
  }
  function renderCatalog() {
    const workspace = WORKSPACES[state.route.view];
    const collection = state.collection;
    const knowledge = state.route.view === "knowledge";
    const catalog = node("div", "catalog" + (state.route.view === "organization" ? " organization-catalog" : state.route.view === "definitions" ? " definitions-catalog" : state.route.view === "workflows" ? " workflows-catalog" : ""));
    if (state.route.view === "workflows" && state.detail) catalog.classList.add("execution-open");
    const listPanel = node("section", "panel");
    listPanel.id = "record-list-panel";
    listPanel.tabIndex = -1;
    listPanel.setAttribute("aria-label", workspace.register);
    append(listPanel, append(node("header", "panel-heading"), node("h2", "", workspace.register), node("span", "", knowledge ? collection.page.returned + " on page" : collection.page.total + (state.route.view === "organization" ? " instances" : state.route.view === "definitions" ? " revisions" : " recorded"))));
    if (state.route.view === "tasks" && state.route.assignee) {
      const person = append(node("section", "task-person-context"), node("p", "eyebrow", "RECORDED ASSIGNEE"), node("h3", "", state.route.assignee), node("p", "detail-note", "Visible Tasks with this recorded assignee, including retained completed work. Membership and viewing context do not grant command authority."));
      person.setAttribute("aria-label", "Assignments for " + state.route.assignee);
      person.append(navigationLink("All assignees", routeHash(workspaceRoute("tasks")), "list", "text-link"));
      if (state.person && state.detail) person.append(navigationLink("Manage retirement", routeHash(workspaceRoute("tasks", { assignee: state.route.assignee })), "detail", "text-link"));
      if (state.personError) person.append(node("p", "detail-note", state.personError));
      listPanel.append(person);
    }
    if (state.route.view === "practices") {
      if (state.capabilities.reads.knowledge === true) listPanel.append(append(node("div", "intervention-actions"), navigationLink("Create practice", practiceAdministrationRoute("create"), "detail", "button primary")));
      else listPanel.append(node("p", "detail-note", "Practice creation and applicability require this connection's model administration service."));
    }
    if (state.route.view === "tasks") { const raise = taskCreatePanel(); if (raise) listPanel.append(raise); }
    if (state.route.view === "organization" && state.route.branch) listPanel.append(organizationOutline());
    else if (!collection.items.length) listPanel.append(stateCard(workspace.empty, knowledge ? state.route.cursor ? "There are no knowledge items on this page. Return to the first page to continue." : workspace.emptyDescription : collection.page.total === 0 ? state.route.view === "tasks" && state.route.assignee ? "No visible Task has this recorded assignee in the inspected snapshot. This does not establish that the person has no other responsibilities." : workspace.emptyDescription : "There are no records on this page. Return to the first page to continue.", "◇", !knowledge && collection.page.total ? [button("First page", refresh)] : [], "compact"));
    else if (state.route.view === "organization") listPanel.append(organizationOutline());
    else {
      const list = node("ul", "record-list");
      for (const item of collection.items) {
        list.append(append(node("li"), recordLink(item)));
      }
      listPanel.append(list);
    }
    const page = collection.page;
    if (knowledge) listPanel.append(knowledgePagination(collection, "cursor", "knowledge items"));
    else {
    const pagination = node("div", "pagination");
    const previous = button("Previous page", () => navigate({ ...state.route, id: "", offset: Math.max(0, page.offset - page.limit), snapshot: page.snapshot }));
    previous.disabled = page.offset === 0;
    const next = button("Next page", () => navigate({ ...state.route, id: "", offset: page.next_offset, snapshot: page.snapshot }));
    next.disabled = page.next_offset < 0;
    const shown = state.route.view === "organization" ? organizationRows().length : collection.items.length;
    const pageLabel = state.route.view === "organization" ? shown + (state.route.branch ? " in branch" : " shown") + " · " + collection.items.length + " on page" : collection.items.length ? (page.offset + 1) + "–" + (page.offset + collection.items.length) + " of " + page.total : "0 shown";
    append(pagination, previous, node("span", "pagination-label", pageLabel), next);
    listPanel.append(pagination);
    }
    const detailPanel = node("section", "panel");
    detailPanel.id = "record-detail-panel";
    detailPanel.tabIndex = -1;
    detailPanel.setAttribute("aria-label", workspace.detail);
    if (state.detailError) detailPanel.append(errorCard(state.detailError, true));
    else if (state.detail) detailPanel.append(workspace.inspector(state.detail));
    else if (state.route.view === "tasks" && state.person) detailPanel.append(renderPersonAdministration());
    else {
      detailPanel.classList.add("selection-prompt");
      detailPanel.append(stateCard(workspace.selection, workspace.selectionDescription, "◇", [], "compact"));
    }
    if (state.route.view === "workflows" && state.detail) append(catalog, detailPanel, listPanel);
    else append(catalog, listPanel, detailPanel);
    if (knowledge) {
      const frame = node("div", "knowledge-workspace");
      const coverage = append(node("section", "panel knowledge-coverage"), node("h2", "", "Coverage of this view"), node("p", "", "Native knowledge items, stored relationships, and locus bindings are available. Run, Definition, and Practice dependencies are unavailable."));
      const editor = knowledgeEditor();
      if (state.route.practice_action) {
        const p = state.practiceContext;
        const context = append(node("section", "viewing-context"), append(node("div"), node("span", "eyebrow", "Practice context"), node("strong", "", p?.name || (state.route.practice_action === "create" ? "New practice" : "Exact practice unavailable"))), navigationLink(p ? "Back to practice" : "Back to Practices", routeHash(workspaceRoute("practices", { id: p?.id || "" })), p ? "detail" : "list"));
        context.setAttribute("aria-label", "Practice context");
        append(frame, context, state.detail ? knowledgeMap(state.detail) : null, editor, catalog, coverage);
      } else append(frame, knowledgeContext(), state.detail ? knowledgeMap(state.detail) : null, editor, catalog, coverage);
      return frame;
    }
    if (state.route.view === "organization") {
      const frame = node("div", "organization-workspace");
      const basis = collection.basis;
      const context = append(node("section", "viewing-context"), append(node("div"), node("span", "eyebrow", "Viewing context"), node("strong", "", state.detail ? state.detail.id : "Declared organization")), node("p", "", "Inspection only. Selecting a position does not change your identity or grant its authority."));
      context.setAttribute("aria-label", "Viewing context");
      append(frame, context, catalog);
      const coverage = append(node("section", "panel organization-coverage"), node("h2", "", "What this source establishes"), node("p", "", basis.declaration_count + " declarations · " + collection.page.total + " static instances · " + basis.uninstantiated_declaration_count + " declarations without static instances."), node("p", "", "Static instances describe source structure, not running occupants or vacancies. A declaration with no static instance is not presented as a vacant position."), node("p", "", "Static containment coverage · " + (basis.exact_ownership ? "exact" : "partial")));
      const host = organizationDraftHost || node("div", "organization-draft-host");
      organizationDraftHost = host;
      const ownershipHost = ownershipDraftHost || node("div", "ownership-draft-host");
      ownershipDraftHost = ownershipHost;
      frame.append(host, coverage, organizationOwnership(collection.ownership), ownershipHost);
      if (window.FaceOwnershipDraft && !ownershipDraftController) {
        const token = generation;
        ownershipDraftController = window.FaceOwnershipDraft.mount(ownershipHost, {
          applicationId: state.app.id, principal: state.capabilities.principal,
          basis, recordHead: state.source.record_head, capability: state.capabilities.ownership_drafts,
          onInvalidate(error) {
            if (token !== generation) return;
            const message = error?.message || "Ownership source or access changed. The draft was cleared.";
            if (error?.status === 401 || error?.status === 403) {
              const route = state.route; cancel();
              state = { ...blankState(route), phase: "error", error: new ReadError(error.status, "unauthenticated", message) }; render();
            } else loadRoute(firstPageRoute(state.route), message, "list");
          }
        });
      }
      if (intervention.metadata || intervention.blocked) {
        organizationDraftController?.destroy(); organizationDraftController = null;
        host.replaceChildren(node("p", "detail-note", "Recover or dismiss the saved request before preparing another Organization proposal."));
      } else if (window.FaceOrganizationDraft && !organizationDraftController) {
        const token = generation;
        organizationDraftController = window.FaceOrganizationDraft.mount(host, {
          applicationId: state.app.id, principal: state.capabilities.principal,
          basis, recordHead: state.source.record_head, capability: state.capabilities.organization_drafts, selectedId: state.detail?.id || "",
          publicationAccess() { return token === generation ? organizationProposalAccess() : { allowed: false, reason: "The inspected Organization changed. Reload before proposing source." }; },
          onPropose(data, rationale) {
            return token === generation ? proposeOrganization(data, rationale) : { error: "The inspected Organization changed. Nothing was submitted." };
          },
          validateProjection(data) {
            validRows(data.items, "organization");
            validOrganization({ ...data, ownership: collection.ownership });
          },
          onInvalidate(error) {
            if (token !== generation) return;
            const message = error?.message || "Organization source or access changed. The draft was cleared.";
            if (error?.status === 401 || error?.status === 403) {
              const route = state.route; cancel();
              state = { ...blankState(route), phase: "error", error: new ReadError(error.status, "unauthenticated", message) }; render();
            } else loadRoute(firstPageRoute(state.route), message, "list");
          }
        });
      }
      return frame;
    }
    return catalog;
  }
  function knowledgeEditor() {
    if (knowledgeCommand.metadata || knowledgeCommand.blocked) return node("p", "detail-note", "Recover or dismiss the saved Knowledge request before preparing another change.");
    if (state.route.practice_action && state.route.practice_action !== "create" && !state.practiceContext) return node("p", "detail-note", "The exact Practice and its applicability must both be readable before preparing a change.");
    if (state.practiceContext && (state.practiceContext.retired || !state.practiceContext.ratified)) return node("p", "detail-note", "This practice is not currently in force. Its readable graph and history remain inspectable; follow its Review or current successor before preparing another change.");
    const host = knowledgeDraftHost || node("div", "knowledge-draft-host");
    knowledgeDraftHost = host;
    if (window.FaceKnowledgeDraft && !knowledgeDraftController) {
      const token = generation, appId = state.app.id, principal = { ...state.capabilities.principal };
      const source = state.source, route = { ...state.route }, collection = state.collection, selected = state.detail;
      const base = API + "/" + encodeURIComponent(appId);
      knowledgeDraftController = window.FaceKnowledgeDraft.mount(host, {
        applicationId: appId, principal, basis: collection.basis, snapshot: collection.page.snapshot,
        ...(route.practice_action ? { practice: { action: route.practice_action, author: state.practiceContext?.author || route.locus || "org", target: route.practice_action === "applicability" ? route.locus || state.practiceContext?.target || "org" : state.practiceContext?.target || route.locus || "org" } } : {}),
        target: route.target, item: selected, relationships: state.relationships, bindings: state.bindings,
        visibleItems: collection.items,
        onReturn: () => route.practice_action ? navigate(workspaceRoute("practices", { id: selected?.id || "" })) : focusPanel("graph"),
        onPreview(preview) {
          if (token !== generation || host !== knowledgeDraftHost || state.phase !== "ready" || state.route.view !== "knowledge" || state.detail?.id !== selected?.id) return;
          knowledgePreview = preview.active ? preview : null;
          if (!preview.active) knowledgeComparison = "compare";
          refreshKnowledgeMap();
        },
        async onReview({ relatedId, operation, signal }) {
          async function fresh(kind, id = "", cursor = "") {
            const params = new URLSearchParams({ limit: String(LIMIT), snapshot: collection.page.snapshot });
            if (route.target) params.set("target", route.target);
            if (id) params.set("id", id);
            if (cursor) params.set("cursor", cursor);
            const response = await request(base + "/dna/knowledge/" + kind + "?" + params, signal);
            ensureCurrent(token, signal); validSource(response.source, appId);
            validKnowledge(response.data, response.source, { ...route, id, snapshot: collection.page.snapshot }, kind, cursor);
            assert(response.source.record_head === source.record_head && response.source.record_revision === source.record_revision && sameKnowledgeBasis(response.data.basis, collection.basis), "The Knowledge source or visibility changed while preparing this draft.");
            if (kind === "nodes" && id) assert(response.data.items.length === 1 && response.data.items[0].id === id, "The service did not return the exact visible item requested.");
            return response.data;
          }
          const capability = await request(base + "/capabilities", signal);
          ensureCurrent(token, signal); validSource(capability.source, appId);
          if (capability.data.application_id !== appId || capability.data.principal?.mode !== principal.mode || capability.data.principal?.name !== principal.name || capability.data.reads?.knowledge !== true || capability.source.record_head !== source.record_head) {
            throw new ReadError(409, "snapshot_changed", "The Knowledge source or signed-in identity changed. The draft was cleared.");
          }
          const reads = await Promise.allSettled([
            fresh("nodes", "", route.cursor),
            selected ? fresh("nodes", selected.id) : Promise.resolve(null),
            selected ? fresh("edges", selected.id, route.edges_cursor) : Promise.resolve(null),
            selected ? fresh("bindings", selected.id, route.bindings_cursor) : Promise.resolve(null),
            relatedId ? fresh("nodes", relatedId) : Promise.resolve(null)
          ]);
          ensureCurrent(token, signal);
          const errors = reads.filter(read => read.status === "rejected").map(read => read.reason);
          if (errors.length) throw errors.find(error => [401, 403, 409].includes(error.status)) || errors[0];
          let commandCapability = null;
          if (nativeKnowledgeOperation(operation)) {
            knowledgeCommand.capability = null;
            document.querySelector(".read-only").textContent = knowledgeAccessLabel();
            try {
              const response = await readKnowledgeCapability(appId, principal, nativeKnowledgeOperation(operation), signal);
              ensureCurrent(token, signal);
              if (response.source.record_head !== source.record_head) throw new ReadError(409, "snapshot_changed", "The Record changed while checking relationship authority. Refresh before submitting.");
              commandCapability = response.capability;
              knowledgeCommand.capability = commandCapability;
            } catch (error) {
              if (signal.aborted || error.status === 401 || error.code === "command_context_changed" || error.status === 409) throw error;
              commandCapability = { enabled: false, mode: "unavailable", reason: "Relationship submission is unavailable on this connection." };
            }
            document.querySelector(".read-only").textContent = knowledgeAccessLabel();
          }
          return { node: reads[1].value?.items[0] || null, relationships: reads[2].value, bindings: reads[3].value, related: reads[4].value?.items[0] || null, commandCapability };
        },
        onSubmit(draft, current) {
          if (token !== generation || !current() || state.detail?.id !== selected?.id || state.source.record_head !== source.record_head || !knowledgeCommand.capability?.enabled || knowledgeCommand.capability.operation !== nativeKnowledgeOperation(draft.operation)) throw new Error("The checked Knowledge source or authority is no longer current. Review the draft again.");
          return submitKnowledgeCommand(draft, () => token === generation && host === knowledgeDraftHost && current());
        },
        onInvalidate(error) {
          if (token !== generation) return;
          const message = error?.status === 404 ? "A referenced Knowledge item is unavailable in this view. The draft was cleared." : error?.message || "Knowledge source or access changed. The draft was cleared.";
          if (error?.status === 401 || error?.status === 403) {
            cancel(); state = { ...blankState(route), phase: "error", error: new ReadError(error.status, "unauthenticated", message) }; render();
          } else loadRoute(firstPageRoute(route), message, "list");
        }
      });
    }
    return host;
  }
  function knowledgePagination(data, key, label) {
    const selected = key !== "cursor";
    const change = (cursor) => {
      navigationFocus = selected ? "detail" : "list";
      navigate({ ...state.route, [key]: cursor, snapshot: data.page.snapshot, ...(selected ? {} : { id: "", practice_action: "", edges_cursor: "", bindings_cursor: "" }) });
    };
    const first = button("First page", () => change(""));
    first.disabled = !state.route[key];
    first.setAttribute("aria-label", "First page of " + label);
    const next = button("Next page", () => change(data.page.next_cursor));
    next.disabled = !data.page.has_more;
    next.setAttribute("aria-label", "Next page of " + label);
    return append(node("div", "pagination knowledge-pagination"), first, node("span", "pagination-label", data.page.returned + " shown" + (data.page.has_more ? " · more available" : "")), next);
  }
  function knowledgeContext() {
    const form = node("form", "panel knowledge-context");
    const label = node("label", "", "Relevant to locus");
    label.htmlFor = "knowledge-target";
    const input = node("input");
    input.id = "knowledge-target";
    input.type = "text";
    input.value = state.route.target || "";
    input.placeholder = "All loci";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.setAttribute("aria-describedby", "knowledge-context-hint");
    const submit = node("button", "button secondary", "Apply context");
    submit.type = "submit";
    const hint = node("p", "field-hint", "Use a native locus path to narrow relevance. Your signed-in identity stays the same.");
    hint.id = "knowledge-context-hint";
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      navigationFocus = "list";
      navigate({ ...firstPageRoute(state.route), id: "", locus: input.value.trim() === state.route.locus ? state.route.locus : "", target: input.value.trim() });
    });
    const controls = append(node("div", "input-row"), input, submit);
    if (state.route.target) controls.append(button("All loci", () => {
      navigationFocus = "list";
      navigate({ ...firstPageRoute(state.route), id: "", locus: "", target: "" });
    }));
    return append(form, label, controls, hint);
  }
  function knowledgeDetail(item) {
    const detail = node("article", "detail knowledge-detail");
    append(detail, backToList(), append(node("div", "detail-kicker"), node("span", "eyebrow", "Knowledge item"), knowledgeBadge(item)), node("h2", "", knowledgeName(item)), node("p", "detail-subtitle", display(item.kind, "Kind not recorded") + " · revision " + item.revision), node("hr", "detail-rule"), section("Content", documentText(item.text)));
    if (item.kind === "practice" && state.capabilities.reads.practices === true) detail.append(navigationLink("Open Practice", routeHash(workspaceRoute("practices", { id: item.id })), "detail", "button secondary"));
    const identity = node("dl", "fact-grid");
    fact(identity, "Receipt identity", item.id, true, true);
    fact(identity, "Recorded name", item.name, true);
    fact(identity, "Author", item.author);
    fact(identity, "Exact revision", item.revision, false, true);
    fact(identity, "Projection lifecycle", item.projection_state);
    fact(identity, "Accepted in projection", item.accepted ? "Yes" : "No");
    fact(identity, "Ratification sequence", item.ratified_seq, false, true);
    detail.append(section("Identity & lifecycle", identity));
    const provenance = append(node("div", "unavailable-content"), node("strong", "", "Original provenance unavailable"), node("p", "", "The stored projection does not retain the original receipt provenance. Its lifecycle state describes how this idea is held in the projection."));
    detail.append(section("Source provenance", provenance));
    const predecessor = node("div");
    if (item.supersedes) {
      append(predecessor, node("p", "mono", item.supersedes));
      if (receiptID(item.supersedes)) predecessor.append(navigationLink("Open predecessor", relatedRoute("knowledge", item.supersedes), "detail"));
      predecessor.append(node("p", "detail-note", "The recorded supersedes field names this predecessor."));
    } else predecessor.append(node("p", "detail-note", "Predecessor not available in this view."));
    detail.append(section("Supersedes", predecessor));
    detail.append(knowledgeRelationships(item), knowledgeBindings());
    return detail;
  }
  function refreshKnowledgeMap() {
    const previous = $("relationship-map-panel");
    if (!previous || state.phase !== "ready" || state.route.view !== "knowledge" || !state.detail) return;
    const hadFocus = previous.contains(document.activeElement);
    const focused = hadFocus ? document.activeElement.dataset.knowledgeControl : "";
    const openEvidence = new Set(Array.from(previous.querySelectorAll("details[open][data-knowledge-evidence]")).map(el => el.dataset.knowledgeEvidence));
    const scrolls = ["incoming", "outgoing"].map(direction => {
      const list = previous.querySelector(".map-lane-" + direction + " .map-connections");
      return { direction, top: list?.scrollTop || 0, left: list?.scrollLeft || 0 };
    });
    const next = knowledgeMap(state.detail);
    previous.replaceWith(next);
    for (const { direction, top, left } of scrolls) next.querySelector(".map-lane-" + direction + " .map-connections")?.scrollTo({ top, left });
    for (const evidence of next.querySelectorAll("details[data-knowledge-evidence]")) evidence.open = openEvidence.has(evidence.dataset.knowledgeEvidence);
    if (hadFocus) (Array.from(next.querySelectorAll("[data-knowledge-control]")).find(el => el.dataset.knowledgeControl === focused) || next).focus({ preventScroll: true });
  }
  function knowledgeMap(item) {
    const map = node("section", "panel relationship-map");
    map.id = "relationship-map-panel";
    map.dataset.knowledgeControl = "map";
    map.tabIndex = -1;
    map.setAttribute("aria-label", "Knowledge relationship map");
    const stored = state.relationships.items;
    const preview = knowledgePreview, comparing = preview && knowledgeComparison === "compare";
    const requested = preview?.arguments || {};
    const edges = stored.map(edge => ({ ...edge, change: comparing && preview.operation === "edge.unlink" && requested.id === edge.id ? "removed" : "current" }));
    if (comparing && preview.operation === "edge.link" && receiptID(requested.from_id) && receiptID(requested.to_id) && [requested.from_id, requested.to_id].includes(item.id)) edges.push({ from_id: requested.from_id, to_id: requested.to_id, rel: requested.rel, change: "added", draft: true });
    const names = new Map(state.collection.items.map((row) => [row.id, knowledgeName(row)]));
    names.set(item.id, knowledgeName(item));
    if (preview?.checked && preview.related && [requested.from_id, requested.to_id].includes(preview.related.id)) names.set(preview.related.id, knowledgeName(preview.related));
    const incoming = new Map(), outgoing = new Map(), self = [];
    for (const edge of edges) {
      if (edge.from_id === item.id && edge.to_id === item.id) { self.push(edge); continue; }
      const fromFocus = edge.from_id === item.id;
      const id = fromFocus ? edge.to_id : edge.from_id;
      const lane = fromFocus ? outgoing : incoming;
      if (!lane.has(id)) lane.set(id, []);
      lane.get(id).push(edge);
    }
    const heading = append(node("header", "panel-heading"), node("h2", "", "Relationship map"), node("span", "", stored.length + " relationships on page"));
    const note = node("p", "map-note", "Only relationships on this page are shown. Follow a connected item to explore its direct relationships.");
    note.id = "relationship-map-note";
    map.setAttribute("aria-describedby", note.id);
    const edgeLabel = edge => {
      const row = node("li", "map-edge");
      row.dataset.fromId = edge.from_id; row.dataset.toId = edge.to_id; row.dataset.change = edge.change;
      if (edge.draft) row.append(node("span", "map-draft-edge-label", "Requested · " + display(edge.rel, "Relationship not named")));
      else {
        row.dataset.edgeId = edge.id;
        const inspect = button(display(edge.rel, "Relationship not named"), () => { knowledgeSelectedEdge = edge.id; refreshKnowledgeMap(); }, "map-edge-control");
        inspect.setAttribute("aria-label", "Inspect relationship " + edge.id);
        inspect.setAttribute("aria-pressed", String(knowledgeSelectedEdge === edge.id));
        inspect.dataset.knowledgeControl = "edge:" + edge.id;
        row.append(inspect);
      }
      return row;
    };
    const lane = (title, groups, direction) => {
      const column = node("section", "map-lane map-lane-" + direction);
      column.setAttribute("role", "group");
      column.setAttribute("aria-label", title);
      const nativeCount = Array.from(groups.values()).filter(connections => connections.some(edge => !edge.draft)).length;
      column.append(append(node("div", "map-lane-heading"), node("h3", "", title), node("span", "", nativeCount + " items on page" + (groups.size > nativeCount ? " + draft endpoint" : ""))));
      if (!groups.size) column.append(node("p", "map-empty", "None on this page."));
      else {
        const list = node("ul", "map-connections");
        for (const [id, connections] of groups) {
          const row = node("li", "map-node");
          const native = connections.some(edge => !edge.draft);
          const name = names.get(id) || (native ? "Knowledge item" : "Unverified item");
          const destination = native ? navigationLink(name, relatedRoute("knowledge", id), "graph", "map-node-link") : node("div", "map-node-link", name);
          if (native) {
            destination.setAttribute("aria-label", "Open connected knowledge item " + id + " · " + name);
            destination.dataset.knowledgeControl = "connected:" + direction + ":" + id;
          }
          else { row.dataset.change = "added"; destination.append(node("span", "map-draft-endpoint", "Requested endpoint")); }
          row.classList.toggle("map-selected-neighbor", connections.some(edge => edge.id && edge.id === knowledgeSelectedEdge));
          destination.append(node("span", "mono map-node-id", id));
          const relationships = node("ul", "map-edge-labels");
          for (const edge of connections) relationships.append(edgeLabel(edge));
          append(row, destination, relationships);
          list.append(row);
        }
        column.append(list);
      }
      return column;
    };
    const focus = node("div", "map-focus");
    const current = append(node("div", "map-current"), node("span", "eyebrow", "Focused item"), node("h3", "", knowledgeName(item)), knowledgeBadge(item), node("p", "mono map-node-id", item.id), button("Inspect this item", () => focusPanel("detail"), "button secondary"));
    current.querySelector("button").dataset.knowledgeControl = "inspect-item";
    if (comparing && ["node.revise", "node.retire"].includes(preview.operation)) {
      current.dataset.change = preview.operation === "node.retire" ? "removed" : "changed";
      current.append(node("p", "map-candidate-caption", preview.operation === "node.retire" ? "Retirement requested" : "Revision requested · " + display(requested.name, "Untitled draft")));
    }
    if (comparing && ["binding.bind", "binding.unbind"].includes(preview.operation)) {
      const binding = node("div", "map-draft-binding");
      binding.dataset.change = preview.operation === "binding.unbind" ? "removed" : "added";
      append(binding, node("span", "eyebrow", "Requested applicability"), node("p", "", preview.operation === "binding.unbind" ? "Remove binding at" : "Bind at"), node("code", "", requested.target || "Choose a locus"));
      current.append(binding);
    }
    focus.append(current);
    if (self.length) {
      const loop = append(node("section", "map-self"), node("h3", "", "Self references"));
      loop.setAttribute("role", "group");
      loop.setAttribute("aria-label", "Self references");
      const labels = node("ul", "map-edge-labels");
      for (const edge of self) labels.append(edgeLabel(edge));
      append(loop, node("span", "map-loop-arrow", "↶"), labels);
      loop.querySelector(".map-loop-arrow").setAttribute("aria-hidden", "true");
      focus.append(loop);
    }
    const incomingArrow = node("span", "map-arrow map-arrow-in", "→");
    const outgoingArrow = node("span", "map-arrow map-arrow-out", "→");
    incomingArrow.setAttribute("aria-hidden", "true"); outgoingArrow.setAttribute("aria-hidden", "true");
    incomingArrow.classList.toggle("map-arrow-empty", !incoming.size); outgoingArrow.classList.toggle("map-arrow-empty", !outgoing.size);
    const diagram = append(node("div", "map-columns"), focus, lane("Incoming", incoming, "incoming"), incomingArrow, outgoingArrow, lane("Outgoing", outgoing, "outgoing"));
    append(map, heading, note, diagram);
    const actions = state.route.practice_action
      ? append(node("div", "map-context-actions"), node("span", "eyebrow", "Practice change"), button("Continue practice draft", () => knowledgeDraftController?.begin()))
      : append(node("div", "map-context-actions"), node("span", "eyebrow", "Change focused item"), button("Revise focused item", () => knowledgeDraftController?.begin({ operation: "node.revise" })), button("Add relationship", () => knowledgeDraftController?.begin({ operation: "edge.link" })), button("Add locus binding", () => knowledgeDraftController?.begin({ operation: "binding.bind" })));
    for (const control of actions.querySelectorAll("button")) control.disabled = Boolean(knowledgeCommand.metadata || knowledgeCommand.blocked || state.route.practice_action && (!state.practiceContext || state.practiceContext.retired || !state.practiceContext.ratified));
    Array.from(actions.querySelectorAll("button")).forEach((button, index) => { button.dataset.knowledgeControl = "action:" + index; });
    map.append(actions);
    const selected = stored.find(edge => edge.id === knowledgeSelectedEdge);
    if (selected) {
      const inspector = node("section", "map-relationship-inspector");
      inspector.setAttribute("aria-label", "Selected relationship");
      append(inspector, node("span", "eyebrow", "Stored relationship"), node("h3", "", display(selected.rel, "Relationship not named")), node("p", "map-selected-direction", (names.get(selected.from_id) || "Knowledge item") + " → " + (names.get(selected.to_id) || "Knowledge item")));
      const evidence = node("details");
      evidence.dataset.knowledgeEvidence = selected.id;
      evidence.append(node("summary", "", "Exact relationship identity"));
      evidence.querySelector("summary").dataset.knowledgeControl = "evidence:" + selected.id;
      const facts = node("dl", "fact-grid");
      fact(facts, "Relationship", selected.id, true, true); fact(facts, "From", selected.from_id, true, true); fact(facts, "To", selected.to_id, true, true);
      evidence.append(facts);
      if (!state.route.practice_action) inspector.append(button("Remove this relationship", () => knowledgeDraftController?.begin({ operation: "edge.unlink", edgeId: selected.id })));
      inspector.append(evidence);
      const remove = inspector.querySelector("button");
      if (remove) {
        remove.disabled = Boolean(knowledgeCommand.metadata || knowledgeCommand.blocked);
        remove.dataset.knowledgeControl = "remove:" + selected.id;
      }
      map.append(inspector);
    }
    if (preview) {
      const draft = node("section", "map-draft-context");
      draft.setAttribute("aria-label", "Knowledge graph draft");
      const planes = node("div", "map-comparison"); planes.setAttribute("role", "group"); planes.setAttribute("aria-label", "Knowledge comparison");
      for (const [value, label] of [["compare", "Compare draft"], ["current", "Current graph"]]) {
        const choice = button(label, () => { knowledgeComparison = value; refreshKnowledgeMap(); }, "scope-button");
        choice.dataset.knowledgeControl = "plane:" + value;
        choice.setAttribute("aria-pressed", String(knowledgeComparison === value)); planes.append(choice);
      }
      append(draft, planes, node("p", "map-draft-status", preview.checked ? "Source checked · not submitted." : "Draft only · source check required."));
      if (preview.operation === "node.propose") draft.append(node("h3", "", "New item · " + display(requested.name, "Untitled draft")), node("p", "detail-note", "No item identity or relationships have been assigned."));
      else if (preview.operation === "node.revise") draft.append(node("p", "", "Proposed revision remains attached to this exact predecessor. The service must establish any new identity and adoption."));
      else if (preview.operation === "node.retire") draft.append(node("p", "", "Retirement is requested for this item. The visible relationships remain recorded; their treatment requires the owning service."));
      else if (preview.operation === "edge.unlink") draft.append(node("p", "", "Removal requested for one exact stored relationship. Its endpoints remain unchanged."));
      else if (preview.operation === "edge.link") {
        draft.append(node("p", "", "The dashed relationship is a requested addition. Endpoint visibility is checked during review."));
        const other = requested.from_id === item.id ? requested.to_id : requested.from_id;
        if (other && !names.has(other)) draft.append(node("p", "map-draft-endpoint", "Unverified item · " + other));
        else if (preview.checked && preview.related) draft.append(node("p", "map-draft-endpoint", "Related item checked · " + knowledgeName(preview.related)));
      }
      else draft.append(node("p", "", "Requested locus applicability. Effective binding and authority require the owning service."));
      draft.append(node("p", "detail-note", "Current graph unchanged. This page is not a complete impact assessment."), button("Continue editing draft", () => knowledgeDraftController?.begin()));
      draft.lastChild.dataset.knowledgeControl = "continue-draft";
      map.insertBefore(draft, diagram);
    }
    return map;
  }
  function knowledgeRelationships(item) {
    const data = state.relationships;
    const content = node("div");
    content.append(node("p", "detail-note knowledge-section-note", "Direct relationships stored for this item. Arrows show the recorded direction."));
    if (!data.items.length) content.append(node("p", "detail-note", "No visible relationships on this page."));
    else {
      const list = node("ul", "knowledge-relationships");
      for (const edge of data.items) {
        const outgoing = edge.from_id === item.id;
        const other = outgoing ? edge.to_id : edge.from_id;
        const row = node("li", "knowledge-relationship");
        const direction = node("span", "relationship-direction", outgoing ? "This item →" : "→ This item");
        const endpoint = navigationLink(other === item.id ? "This item (self-reference)" : other, relatedRoute("knowledge", other), "detail", "text-link mono relationship-endpoint");
        append(row, node("div", "relationship-kind", display(edge.rel, "Relationship not named")), append(node("div", "relationship-path"), ...(outgoing ? [direction, endpoint] : [endpoint, direction])));
        const receipt = node("details", "relationship-receipt");
        append(receipt, node("summary", "", "Relationship identity"), node("p", "mono", edge.id));
        row.append(receipt);
        list.append(row);
      }
      content.append(list);
    }
    content.append(knowledgePagination(data, "edges_cursor", "relationships"));
    return section("Direct relationships", content);
  }
  function knowledgeBindings() {
    const data = state.bindings;
    const content = node("div");
    content.append(node("p", "detail-note knowledge-section-note", "Stored locus bindings describe where this idea applies."));
    if (!data.items.length) content.append(node("p", "detail-note", "No visible bindings on this page."));
    else {
      const list = node("ul", "knowledge-bindings");
      for (const binding of data.items) {
        const row = node("li", "knowledge-binding");
        row.dataset.bindingId = binding.id;
        const facts = node("dl", "fact-grid");
        fact(facts, "Locus", binding.target, true, true);
        fact(facts, "Binding class", binding.class);
        fact(facts, "Author", binding.author);
        fact(facts, "Applies to this context", { exact: "Exact locus", ancestor: "Ancestor locus", unfiltered: "All loci view" }[binding.applicability], true);
        row.append(facts);
        const receipt = node("details", "relationship-receipt");
        append(receipt, node("summary", "", "Binding identity"), node("p", "mono", binding.id));
        const remove = button("Remove this binding", () => knowledgeDraftController?.begin({ operation: "binding.unbind", bindingId: binding.id })); remove.disabled = Boolean(knowledgeCommand.metadata || knowledgeCommand.blocked || state.route.practice_action && state.route.practice_action !== "applicability" || state.practiceContext && (state.practiceContext.retired || !state.practiceContext.ratified));
        row.append(remove, receipt);
        list.append(row);
      }
      content.append(list);
    }
    content.append(knowledgePagination(data, "bindings_cursor", "bindings"));
    return section("Used at", content);
  }
  function reconcileDefinitionJourney() {
    if (state.route.view !== "definitions" || !state.detail || !state.collection) {
      definitionJourney = null;
      return;
    }
    const snapshot = state.collection.page.snapshot;
    const principal = JSON.stringify(state.capabilities.principal);
    if (!definitionJourney || definitionJourney.app !== state.app.id || definitionJourney.snapshot !== snapshot || definitionJourney.principal !== principal || !definitionJourney.entries.some((entry) => entry.id === state.detail.id)) {
      definitionJourney = { app: state.app.id, snapshot, principal, entries: [{ id: state.detail.id, step: "", key: "" }] };
    }
  }
  function definitionPath(item) {
    const path = node("nav", "definition-path");
    path.setAttribute("aria-label", "Traversed definition path");
    const entries = definitionJourney?.entries || [{ id: item.id, step: "", key: "" }];
    const current = entries.findIndex((entry) => entry.id === item.id);
    entries.slice(0, current + 1).forEach((entry, index) => {
      if (index) path.append(node("span", "definition-path-via", definitionStepLabel(entry.step) + " · " + entry.key + " →"));
      if (index === current) {
        const selected = node("span", "mono", entry.id);
        selected.setAttribute("aria-current", "location");
        path.append(selected);
      } else path.append(navigationLink(entry.id, relatedRoute("definitions", entry.id), "detail"));
    });
    return path;
  }
  function definitionDetail(item) {
    const detail = node("article", "detail definition-detail");
    append(detail, backToList(), definitionPath(item));
    if (!state.collection.items.some((row) => row.id === item.id)) detail.append(node("p", "detail-note", "This revision is on another page. Back to definitions returns to the catalog page you were inspecting."));
    const host = definitionDraftHost || node("div", "definition-draft-host");
    definitionDraftHost = host;
    detail.append(host);
    const basis = state.collection.basis;
    if (window.FaceDefinitionDraft && !definitionDraftController) {
      const token = generation;
      definitionDraftController = window.FaceDefinitionDraft.mount(host, {
        item, basis, applicationId: state.app.id, principal: state.capabilities.principal,
        capability: state.capabilities.definition_drafts,
        onInvalidate(error) {
          if (token !== generation) return;
          definitionJourney = null;
          const message = error?.message || "The definition source or access changed. The draft was cleared.";
          if (error?.status === 401) {
            const route = state.route;
            cancel();
            state = { ...blankState(route), phase: "error", error: new ReadError(401, "unauthenticated", message) };
            render();
          } else loadRoute(firstPageRoute(state.route), message, "detail");
        },
        onNavigateChild(id, occurrence) {
          if (token !== generation) return;
          const step = item.steps.find((value) => value.index === occurrence?.step);
          const member = step?.members.find((value) => value.key === occurrence?.key);
          if (member?.kind !== "child" || member.child.id !== id) return;
          const current = definitionJourney.entries.findIndex((entry) => entry.id === item.id);
          definitionJourney.entries = [...definitionJourney.entries.slice(0, current + 1), { id, step: step.index, key: member.key }];
          navigationFocus = "detail";
          navigate({ ...state.route, id, snapshot: state.collection.page.snapshot });
        }
      });
    } else if (!window.FaceDefinitionDraft) host.replaceChildren(stateCard("Definition workspace unavailable", "Reload the face to load the definition editor.", "◇"));
    const references = node("details", "definition-evidence");
    references.append(node("summary", "", "Direct catalog references · " + item.dependents.length));
    const dependents = node("ul", "definition-dependents");
    for (const dependent of item.dependents) {
      dependents.append(append(node("li", "linked-record"), append(node("div"), node("div", "linked-label", dependent.definition_id), node("p", "mono", "Revision " + dependent.revision), node("p", "", definitionStepLabel(dependent.step) + " · member " + dependent.key)), navigationLink("Open " + dependent.id, relatedRoute("definitions", dependent.id), "detail")));
    }
    references.append(item.dependents.length ? dependents : node("p", "detail-note", "No definitions in this catalog directly reference this exact revision."));
    references.append(node("p", "detail-note", "Existing references remain pinned to this revision. These links do not establish running work or wider system impact."));
    detail.append(references);
    const evidence = node("details", "definition-evidence");
    evidence.append(node("summary", "", "Source claims & admission limits"));
    const provenance = node("dl", "fact-grid");
    fact(provenance, "Source revision", basis.source_revision, true, true);
    fact(provenance, "Source module", basis.source_path, true, true);
    fact(provenance, "Catalog digest", basis.catalog_digest, true, true);
    fact(provenance, "Dependency digest", display(basis.dependency_digest, "Not supplied"), true, true);
    evidence.append(section("Source provenance", provenance), node("p", "detail-note", "Trusted host claims identify the loaded catalog. They do not establish source-file ownership or permission to replace that module."));
    const limits = node("dl", "fact-grid");
    DEFINITION_LIMITS.forEach(([key, label]) => fact(limits, label, basis.limits[key], false, true));
    evidence.append(section("Catalog admission limits", limits));
    detail.append(evidence);
    return detail;
  }
  function organizationDetail(item) {
    const detail = node("article", "detail");
    append(detail, backToList(), append(node("div", "detail-kicker"), node("span", "eyebrow", "Source-declared instance"), organizationBadge(item)), node("h2", "", item.id), node("p", "detail-subtitle", "Declaration · " + item.declaration), node("hr", "detail-rule"));
    if (!state.collection.items.some((row) => row.id === item.id)) detail.append(node("p", "detail-note", item.id === state.route.branch ? "This branch root was read by identity. Its children in the canvas come from the current page." : "This instance is on another page. The outline remains on the page you were inspecting; Back to organization returns to that page."));
    else if (state.collection.basis.position_group_declared && state.route.scope !== "all" && !item.in_position_outline) detail.append(node("p", "detail-note", item.id === state.route.branch ? "This structural instance remains visible as the branch root. The positions filter applies to its children." : "This instance is outside the declared-position outline. Choose All structure to include it on this page."));
    if (state.route.branch && state.organizationBranch && !organizationRows().some(row => row.id === item.id)) detail.append(node("p", "detail-note", "The inspected instance is outside this branch's visible neighborhood. Enter its branch to bring it into focus."));
    const actions = node("div", "organization-instance-actions");
    actions.append(navigationLink("Enter branch", organizationBranchRoute(item.id), "list", "button secondary"));
    if (state.capabilities.organization_drafts?.supported && item.source_file === "dna/org/main.hl") actions.append(button("Edit this instance", () => organizationDraftController?.edit(item.id), "button primary"));
    detail.append(actions);
    const facts = node("dl", "fact-grid");
    fact(facts, "Declared role", item.role === "position" ? "Member of the positions group" : "Structural instance");
    fact(facts, "Thread domain", item.thread_domain);
    fact(facts, "Sealed declaration", item.sealed ? "Yes" : "No");
    fact(facts, "Source file", item.source_file, true, true);
    detail.append(section("Declared structure", facts));
    if (item.parent_id) detail.append(section("Containment", append(node("div", "linked-record"), append(node("div"), node("div", "linked-label", "Declared parent"), node("p", "mono", item.parent_id)), navigationLink("Open parent", relatedRoute("organization", item.parent_id), "detail"))));
    else detail.append(section("Containment", node("p", "detail-note", "No parent is recorded for this static instance.")));
    detail.append(section("Parameters", declaredTable(["Name", "Type"], item.parameters.map((p) => [p.name, p.type]), "No parameters declared.")));
    detail.append(section("Methods", declaredNames(item.methods, "No methods declared.")));
    detail.append(section("Publishes", declaredNames(item.publishes, "No publications declared.")));
    detail.append(section("Subscriptions", declaredTable(["Topic", "Handler", "Capacity", "Shed"], item.subscribes.map((s) => [s.topic, s.handler, s.capacity === null ? "Not specified" : s.capacity, display(s.shed)]), "No subscriptions declared.")));
    detail.append(section("Supervision", declaredTable(["Child", "Error", "Operations", "Retry"], item.supervises.map((s) => [s.child, s.error, s.ops.join(", ") || "Not specified", s.retry === null ? "Not specified" : s.retry]), "No supervision declared.")));
    detail.append(section("Identity & authority", node("p", "detail-note", "These declarations describe the source contract. They do not identify a running occupant, grant the signed-in person an acting role, or establish effective command permissions.")));
    return detail;
  }
  function declaredNames(values, empty) {
    if (!values.length) return node("p", "detail-note", empty);
    const list = node("ul", "declared-names");
    values.forEach((value) => list.append(node("li", "mono", value)));
    return list;
  }
  function declaredTable(headers, rows, empty) {
    if (!rows.length) return node("p", "detail-note", empty);
    const table = node("table", "declaration-table");
    const head = node("tr");
    headers.forEach((label) => { const th = node("th", "", label); th.scope = "col"; head.append(th); });
    table.append(append(node("thead"), head));
    const body = node("tbody");
    rows.forEach((values) => { const row = node("tr"); values.forEach((value) => row.append(node("td", "", value))); body.append(row); });
    table.append(body);
    return table;
  }
  function organizationOwnership(ownership) {
    const panel = node("section", "panel organization-coverage");
    append(panel, node("h2", "", "Declared ownership map"), node("p", "", "Owning-party assignments are a separate model. This source does not establish a binding between these position names and the static instances above."));
    const facts = node("dl", "fact-grid");
    fact(facts, "Ownership mode", ownership.mode === "shared" ? "Shared ownership" : "Single owner");
    fact(facts, "Host owner", ownership.host_owner);
    panel.append(facts);
    const columns = node("div", "ownership-columns");
    const members = window.FaceOwnershipPeople ? window.FaceOwnershipPeople.render(ownership, {
      available: state.capabilities?.reads?.tasks === true,
      onInspectPerson: ({ name }) => navigate(workspaceRoute("tasks", { assignee: name, locus: "" }))
    }) : declaredTable(["Owner", "Members"], ownership.memberships.map((m) => [m.owner, m.members.join(", ") || "None declared"]), "No explicit memberships declared.");
    append(columns, section("Assignments", declaredTable(["Position name", "Owner"], ownership.positions.map((p) => [p.position, p.owner]), "No explicit position assignments declared.")), section("Memberships", members));
    panel.append(columns);
    if (ownership.positions.length) {
      const contexts = node("div", "ownership-context-actions");
      contexts.setAttribute("aria-label", "Declared locus contexts");
      for (const row of ownership.positions) contexts.append(button("Work from " + row.position, () => changeWorkingContext(row.position)));
      panel.append(contexts);
    }
    return panel;
  }
  function fact(grid, label, value, wide = false, mono = false) {
    append(grid, append(node("div", wide ? "wide" : ""), node("dt", "", label), node("dd", mono ? "mono" : "", display(value))));
  }
  function section(title, content) {
    return append(node("section", "detail-section"), node("h3", "", title), content);
  }
  function documentText(text, className = "") { return node("div", "document-text " + className, text || "No text recorded."); }
  function availableNotice(status) {
    const [title, description] = STATUS_REASONS[status] || ["Content unavailable", "This response does not establish that the content is available to display."];
    return append(node("div", "unavailable-content"), node("strong", "", title), node("p", "", description));
  }
  function practiceTextAvailable(p) { return p.text_available === true && p.text_status === "available"; }
  function reviewTextAvailable(r) { return (r.text_available === true && r.text_status === "available") || r.text_status === "not_applicable"; }
  function practiceDetail(p) {
    const detail = node("article", "detail practice-detail");
    const readable = practiceTextAvailable(p);
    append(detail,
      backToList(),
      append(node("div", "detail-kicker"), node("span", "eyebrow", practiceDocumentLabel(p)), practiceBadge(p)),
      node("h2", "", display(p.name, p.id)),
      node("p", "detail-subtitle", "Target · " + display(p.target) + "   /   " + display(p.binding_class, "No binding class")),
      node("hr", "detail-rule"),
      section("Canonical text", readable ? documentText(p.text) : availableNotice(p.text_status))
    );
    detail.append(renderPracticeIntervention(p));
    if (p.kind === "practice" && readable && state.capabilities.reads.knowledge === true) {
      const actions = node("div", "intervention-actions");
      actions.append(navigationLink(p.ratified && !p.retired ? "Manage applicability" : "Inspect applicability", practiceAdministrationRoute("applicability", p), "graph", "button secondary"));
      if (p.ratified && !p.retired) actions.append(navigationLink("Edit practice & scope", practiceAdministrationRoute("revise", p), "detail", "button secondary"), navigationLink("Retire practice", practiceAdministrationRoute("retire", p), "detail", "button secondary"));
      detail.append(section("Scope & lifecycle", append(node("div"), actions, node("p", "detail-note", "A binding makes this practice relevant at a locus; it does not assign work or change an application's enforcement rules. Revision and retirement preserve earlier versions and require their own Review."))));
    }
    const lifecycle = node("dl", "fact-grid");
    fact(lifecycle, p.kind === "practice" ? "Practice state" : "Document state", PRACTICE_STATES[p.state] || "Unknown · " + p.state);
    fact(lifecycle, "Review decision", p.review_outcome ? OUTCOMES[p.review_outcome] || "Unknown · " + p.review_outcome : "No decision recorded");
    const lifeSection = section("State & decision", lifecycle);
    lifeSection.append(node("p", "detail-note", p.kind === "retirement" ? "This document requests retirement of an earlier version; it is not an active practice. The target version's recorded state establishes whether retirement occurred." : "An approved Review is a recorded decision. Practice ratification and retirement are separate facts."));
    detail.append(lifeSection);
    const provenance = node("dl", "fact-grid");
    fact(provenance, "Requester", p.requester);
    fact(provenance, "Knowledge author", p.author);
    fact(provenance, "Provenance", p.provenance, true);
    detail.append(section("Provenance", provenance));
    if (readable && p.rationale) detail.append(section("Proposal rationale", documentText(p.rationale, "rationale")));
    if (p.review_id) {
      const info = append(node("div"), node("div", "linked-label", "Governing Review"), node("p", "mono", p.review_id), node("p", "", "State · " + display(p.review_state)));
      if (readable && p.review_settled) info.append(node("p", "", p.review_settled));
      detail.append(section("Related decision", append(node("div", "linked-record"), info, navigationLink("Open review", relatedRoute("reviews", p.review_id), "detail"))));
    }
    if (p.supersedes) detail.append(section("Version lineage", append(node("div", "linked-record"), append(node("div"), node("div", "linked-label", p.kind === "retirement" ? "Retirement target" : "Supersedes"), node("p", "mono", p.supersedes)), navigationLink(p.kind === "retirement" ? "View target version" : "View predecessor", relatedRoute("practices", p.supersedes), "detail"))));
    const identities = node("dl", "fact-grid");
    fact(identities, "Document digest", p.digest, true, true);
    fact(identities, "Practice identity", p.id, true, true);
    fact(identities, "Request identity", p.request_id, true, true);
    detail.append(section("Record references", identities));
    return detail;
  }
  function reviewDetail(r) {
    const detail = node("article", "detail review-detail");
    const readable = reviewTextAvailable(r);
    append(detail,
      backToList(),
      append(node("div", "detail-kicker"), node("span", "eyebrow", "Recorded Review"), reviewBadge(r)),
      node("h2", "", r.organization_source ? "Organization change" : r.id),
      node("p", "detail-subtitle", "Required authority · " + display(r.required_authority)),
      node("hr", "detail-rule"),
      section("Question under review", readable ? documentText(r.question) : availableNotice(r.text_status))
    );
    if (r.organization_source) {
      if (state.organizationStatus) {
        detail.append(window.FaceOrganizationStatus.render(state.organizationStatus, { recordHead: state.source.record_head, inspectedAt: state.inspectedAt, onRefresh: refresh }));
      } else {
        const status = append(node("section", "organization-status"), node("h3", "", "Change status unavailable"), node("p", "detail-note", state.organizationStatusError || "Application and running-process evidence could not be read."), button("Refresh change status", refresh));
        status.setAttribute("role", "region"); status.setAttribute("aria-label", "Organization change status"); detail.append(status);
      }
      detail.append(window.FaceOrganizationImpact.render(state.organizationImpact, { inspectedAt: state.inspectedAt, onRefresh: refresh, unavailableReason: state.organizationImpactError }));
    }
    detail.append(renderReviewIntervention(r));
    const decision = node("dl", "fact-grid");
    fact(decision, "Review state", r.state);
    fact(decision, "Recorded decision", r.outcome ? OUTCOMES[r.outcome] || "Unknown · " + r.outcome : "No decision recorded");
    if (readable && r.settled) fact(decision, "Settlement record", r.settled, true);
    const decisionSection = section("Decision", decision);
    decisionSection.append(node("p", "detail-note", "This decision does not establish that the subject was activated or applied. Follow the subject's own state."));
    detail.append(decisionSection);
    const subject = node("dl", "fact-grid");
    fact(subject, "Exact subject digest", r.subject_digest, true, true);
    fact(subject, "Required authority", r.required_authority);
    detail.append(section("Subject & authority", subject));
    if (r.knowledge_digest) detail.append(section("Related practice", append(node("div", "linked-record"), append(node("div"), node("div", "linked-label", "Knowledge document"), node("p", "mono", r.knowledge_digest)), navigationLink("Open practice", relatedRoute("practices", r.knowledge_digest), "detail"))));
    if (r.text_status === "not_applicable") detail.append(node("p", "detail-note", "This ordinary Review declares its question directly in Record; no canonical practice receipt governs that question."));
    return detail;
  }
  function stateCard(title, description, symbol = "◇", actions = [], className = "") {
    const card = append(node("section", "state-card " + className), node("span", "state-symbol", symbol), node("h2", "", title), node("p", "", description));
    card.firstChild.setAttribute("aria-hidden", "true");
    if (actions.length) card.append(append(node("div", "state-actions"), ...actions));
    return card;
  }
  function errorCard(error, detail = false) {
    let title = "Unable to read the application";
    let description = error.message;
    const actions = [];
    if (error.code === "command_context_changed") {
      title = "Sign-in identity changed";
      description = "The session no longer matches the principal who prepared the request. Draft, receipt and Record content have been cleared. The saved request identity is retained for its original principal. Reload or sign in again before checking it; it will not be submitted automatically.";
      actions.push(link("Sign in", "/auth/login", "button"));
    } else if (error.status === 401) {
      title = "Sign in to read this application";
      description = "Your session is missing or has expired. Application data has been cleared.";
      actions.push(link("Sign in", "/auth/login", "button"));
    } else if (error.code === "head_detached") {
      title = "No project is attached";
      description = "The project service has no attached project, so there is no Record to read here. Attach or create one in Projects.";
      actions.push(link("Open Projects", "#/projects", "button"));
    } else if (error.code === "head_api_unavailable" || error.code === "upstream_timeout") {
      title = "Project service unavailable";
      description = (error.code === "upstream_timeout" ? "The attached project's API did not answer in time." : "The attached project's API could not be reached.") + " Inspect the attached project and its API child in Projects.";
      actions.push(link("Open Projects", "#/projects", "button"));
    } else if (error.status === 404) {
      title = error.code === "application_not_found" ? "Application not found" : detail ? WORKSPACES[state.route.view].singular + " not found" : "Read endpoint not found";
      if (detail) description = state.route.view === "organization" ? "The exact instance in this link is absent from the inspected organization source. It has not been replaced with another instance." : state.route.view === "definitions" ? "The exact definition revision in this link is absent from the inspected catalog. It has not been replaced with a newer or different revision." : state.route.view === "knowledge" ? "This item is not available in the inspected knowledge view." : "The exact identifier in this link is absent from the inspected Record snapshot. It has not been replaced with another object.";
      if (error.code === "application_not_found") actions.push(button("Discover application", () => navigate({ ...firstPageRoute(state.route), app: "", id: "", branch: "", locus: "", target: "" })));
    } else if (state.route.view === "knowledge" && (error.code === "unsupported_capability" || error.status === 503)) {
      title = "Knowledge unavailable";
      if (error.code === "unsupported_capability") description = "This connection has not enabled Knowledge reads.";
    } else if (state.route.view === "definitions" && (error.code === "unsupported_capability" || error.code === "definitions_unsupported" || error.code === "definitions_unavailable" || error.code === "definition_source_unavailable" || error.code === "definition_read_limit")) {
      title = "Definition catalog unavailable";
      if (error.code === "unsupported_capability") description = "This connection does not advertise a Definitions read capability. No catalog is available to inspect.";
    } else if (state.route.view === "definitions" && error.code?.startsWith("definition_") && error.status !== 409) title = "Definition catalog could not be verified";
    else if (error.status === 503) title = error.code?.startsWith("organization_") ? "Organization source unavailable" : "Record unavailable";
    else if (error.status === 409) {
      title = state.route.view === "organization" ? "Organization source is changing" : state.route.view === "definitions" ? "Definition catalog is changing" : state.route.view === "knowledge" ? "Knowledge is changing" : "Record is changing";
      description = "The snapshot changed again after one automatic restart. Retry when the source settles; no mixed snapshot is displayed.";
    } else if (error.code === "read_timeout") title = "Service took too long";
    else if (error.code === "connection_failed") title = "Service unreachable";
    else if (error.code === "invalid_response") title = "Response could not be verified";
    else if (error.code === "unsupported_capability") title = "Read capability unavailable";
    if (error.code === "working_context_unavailable") { title = "Working context unavailable"; actions.unshift(button("Clear working context", () => changeWorkingContext(""))); }
    if (state.route.view === "knowledge" && error.status === 400 && state.route.target) actions.push(button("Clear context", () => navigate({ ...firstPageRoute(state.route), id: "", locus: "", target: "" })));
    actions.push(button("Retry", refresh));
    const card = stateCard(title, description, error.status === 401 ? "◇" : "!", actions, detail ? "compact" : "");
    if (detail) card.prepend(backToList());
    return card;
  }
  function pause() {
    if (independentView(state.route.view)) return;
    cancel();
    definitionJourney = null;
    const route = state.route;
    state = blankState(route);
    state.phase = "paused";
    renderConnection(false);
    renderSource();
    renderWorkingContext();
    ui.content.setAttribute("aria-busy", "false");
    ui.content.replaceChildren(stateCard("View paused", "Record content was cleared while this page was away. It will be rechecked when you return.", "◌"));
  }
  ui.refresh.addEventListener("click", refresh);
  document.querySelector(".skip-link").addEventListener("click", (event) => {
    event.preventDefault();
    $("main").focus();
  });
  ui.application.addEventListener("change", () => navigate({ ...firstPageRoute(state.route), app: ui.application.value, id: "", branch: "", locus: "", target: "", assignee: "", assignee_invalid: false }));
  ui["sign-out"].addEventListener("click", pause);
  window.addEventListener("hashchange", () => {
    const route = readRoute();
    const selectedChanged = route.id && (route.id !== state.route.id || route.view !== state.route.view || route.app !== state.route.app);
    const branchChanged = route.view === "organization" && route.branch !== state.route.branch;
    const executionChanged = route.view === "workflows" && route.id === state.route.id && (route.worknode !== state.route.worknode || route.workattempt !== state.route.workattempt);
    const knowledgePageChanged = route.view === "knowledge" && ["cursor", "edges_cursor", "bindings_cursor", "target"].some((key) => route[key] !== state.route[key]);
    const focusTarget = navigationFocus || (branchChanged ? "list" : selectedChanged ? "detail" : executionChanged ? route.workattempt ? "attempt" : "execution" : !route.id && state.route.id ? "list" : knowledgePageChanged ? route.id ? "detail" : "list" : null);
    navigationFocus = null;
    loadRoute(route, "", focusTarget);
  });
  window.addEventListener("pagehide", pause);
  window.addEventListener("pageshow", (event) => { if (event.persisted) refresh(); });
  document.addEventListener("visibilitychange", () => {
    if (independentView(state.route.view)) return;
    if (document.hidden) pause();
    else refresh();
  });
  window.addEventListener("focus", () => {
    if (!independentView(state.route.view) && !document.hidden && Date.now() - lastStarted > 1000) refresh();
  });
  replaceRoute(state.route);
  loadRoute(state.route);
})();
