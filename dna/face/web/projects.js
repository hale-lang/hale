/* Projects workspace over the operator-machine head (dna/api/project_service).
 * Every operation is the CLI verb the head runs; this module submits closed
 * requests, follows durable receipts and reads the head again before it shows
 * any effect. No secret value, draft or receipt enters browser storage: one
 * scoped recovery slot holds only the request identity until it settles. */
"use strict";
(() => {
  const HEAD = "/api/hale/v1/head";
  const PROFILE = "dna.head.v1";
  const STORAGE = "iris.projects-recovery.v1:";
  const FETCH_TIMEOUT_MS = 35000;
  const POLL_MS = 2000;
  const MAX_REQUEST_BYTES = 32768;
  const STATES = ["recorded", "admitted", "refused", "running", "succeeded", "failed", "outcome_unknown"];
  const TERMINAL = new Set(["refused", "succeeded", "failed", "outcome_unknown"]);
  const HEAD_SCOPED = new Set(["dna.project.create", "dna.project.init", "dna.project.attach", "dna.project.forget"]);
  const ATTACHING = new Set(["dna.project.create", "dna.project.init", "dna.project.attach"]);
  const encoder = new TextEncoder();
  const bytes = value => encoder.encode(value).length;
  const closed = (value, keys) => value !== null && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
  const text = (value, max = 65536) => typeof value === "string" && bytes(value) <= max && !value.includes("\u0000") && !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(value);
  const safe = (value, max = 256) => text(value, max) && !/[\u0000-\u001f\u007f]/.test(value) && !value.startsWith("-");
  const int = value => Number.isSafeInteger(value);
  const bool = value => typeof value === "boolean";
  const strings = (value, max = 256) => Array.isArray(value) && value.length <= max && value.every(item => text(item, 4096));
  const hex40 = value => typeof value === "string" && /^[a-f0-9]{40}$/.test(value);
  const principalOK = value => closed(value, ["mode", "name"]) && value.mode === "local" && safe(value.name) && value.name.length > 0;
  const fail = message => { throw new Error(message || "The project service returned an unsupported or incomplete response."); };
  const check = condition => { if (!condition) fail(); };

  // The head envelope is closed at every level: an unexpected key anywhere is a
  // different service, not extra information. It is never the Record envelope.
  function validEnvelope(value) {
    check(closed(value, ["api_version", "head", "data"]) && value.api_version === "hale.v1");
    check(closed(value.head, ["profile", "principal", "active"]) && value.head.profile === PROFILE && principalOK(value.head.principal));
    check(value.head.active === "" || hex40(value.head.active));
  }
  function validBody(value) {
    check(closed(value, ["state", "pid", "since", "command_id", "membrane_up", "mode", "exit_code"]));
    check(["stopped", "running", "exited", "external"].includes(value.state) && int(value.pid) && int(value.since) && text(value.command_id, 256));
    check(bool(value.membrane_up) && ["run", "dev", ""].includes(value.mode) && int(value.exit_code));
  }
  function validHeadData(data, active) {
    check(closed(data, ["state", "active", "state_dir", "sources_dir", "projects_dir", "children", "credentials", "busy", "operations"]));
    check(["detached", "attached"].includes(data.state) && text(data.state_dir, 4096) && text(data.sources_dir, 4096) && text(data.projects_dir, 4096) && text(data.busy, 256));
    if (data.state === "attached") {
      check(closed(data.active, ["application_id", "root", "name", "api"]) && hex40(data.active.application_id) && text(data.active.root, 4096) && text(data.active.name, 256));
      check(closed(data.active.api, ["port", "state", "pid"]) && int(data.active.api.port) && ["starting", "ready", "down"].includes(data.active.api.state) && int(data.active.api.pid));
      check(active === data.active.application_id);
    } else check(data.active === null && active === "");
    check(closed(data.children, ["body"]));
    validBody(data.children.body);
    check(closed(data.credentials, ["needed", "file_sources", "env_present"]) && strings(data.credentials.needed) && strings(data.credentials.file_sources) && strings(data.credentials.env_present));
    check(Array.isArray(data.operations) && data.operations.length <= 64);
    const names = new Set();
    for (const operation of data.operations) {
      check(closed(operation, ["name", "version", "available", "reason_code"]) && /^dna\.[a-z][a-z_.]*$/.test(operation.name) && operation.version === "1" && bool(operation.available) && text(operation.reason_code, 256) && !names.has(operation.name));
      names.add(operation.name);
    }
  }
  function validReceipt(r) {
    check(closed(r, ["command_id", "request_id", "operation", "operation_version", "principal", "context", "target", "fingerprint", "state", "reason_code", "reason", "submitted_at", "updated_at", "run", "outcome"]));
    check(/^command-[a-f0-9]+$/.test(r.command_id) && safe(r.request_id, 128) && r.request_id.length > 0 && /^dna\.[a-z][a-z_.]*$/.test(r.operation) && r.operation_version === "1" && principalOK(r.principal));
    check(closed(r.context, ["head", "application_id"]) && r.context.head === "local" && (r.context.application_id === "" || hex40(r.context.application_id)));
    check(closed(r.target, ["kind", "id"]) && ((r.target.kind === "dna.head" && r.target.id === "local") || (r.target.kind === "dna.project" && hex40(r.target.id))));
    check(/^sha256:[a-f0-9]{64}$/.test(r.fingerprint) && STATES.includes(r.state) && text(r.reason_code, 256) && text(r.reason, 131072) && int(r.submitted_at) && int(r.updated_at));
    if (r.run !== null) check(closed(r.run, ["kind", "pid", "deadline", "external", "exit_code", "log"]) && ["run", "body", "inline"].includes(r.run.kind) && int(r.run.pid) && int(r.run.deadline) && bool(r.run.external) && int(r.run.exit_code) && r.run.log === "/head/logs?run=" + r.command_id);
    check(r.outcome !== null && typeof r.outcome === "object" && !Array.isArray(r.outcome) && bytes(JSON.stringify(r.outcome)) <= 1048576);
  }
  function validProject(item) {
    const summary = ["application_id", "name", "root", "attached_at", "active", "record", "body", "forge", "recent"];
    const detail = closed(item, [...summary, "authority", "connections", "handoffs", "secrets", "receipts"]);
    check(detail || closed(item, summary));
    check(hex40(item.application_id) && text(item.name, 256) && text(item.root, 4096) && int(item.attached_at) && bool(item.active));
    if (item.record !== null) check(closed(item.record, ["head", "revision"]) && hex40(item.record.head) && (int(item.record.revision) || (typeof item.record.revision === "string" && /^(0|[1-9]\d*)$/.test(item.record.revision))));
    validBody(item.body);
    check(closed(item.forge, ["github", "board"]) && text(item.forge.github, 256) && strings(item.forge.board, 64));
    check(Array.isArray(item.recent) && item.recent.length <= 10);
    for (const entry of item.recent) check(closed(entry, ["command_id", "request_id", "operation", "state", "updated_at"]) && /^command-[a-f0-9]+$/.test(entry.command_id) && safe(entry.request_id, 128) && /^dna\.[a-z][a-z_.]*$/.test(entry.operation) && STATES.includes(entry.state) && int(entry.updated_at));
    if (!detail) return;
    check(closed(item.authority, ["command", "task", "organization"]) && bool(item.authority.command) && bool(item.authority.task) && bool(item.authority.organization));
    check(Array.isArray(item.connections) && item.connections.length <= 256 && Array.isArray(item.handoffs) && item.handoffs.length <= 1024 && strings(item.secrets, 256) && Array.isArray(item.receipts) && item.receipts.length <= 50);
    for (const c of item.connections) check(closed(c, ["name", "url", "position", "purpose", "classes", "state"]) && text(c.name, 256) && text(c.url, 2048) && text(c.position, 256) && text(c.purpose, 4096) && strings(c.classes, 64) && text(c.state, 64));
    for (const h of item.handoffs) check(closed(h, ["id", "connection", "kind", "subject", "state"]) && text(h.id, 256) && text(h.connection, 256) && text(h.kind, 64) && text(h.subject, 4096) && text(h.state, 64));
    for (const receipt of item.receipts) validReceipt(receipt);
  }
  function validate(value, kind = "head") {
    validEnvelope(value);
    const data = value.data;
    if (kind === "head") validHeadData(data, value.head.active);
    else if (kind === "projects") {
      check(closed(data, ["items"]) && Array.isArray(data.items) && data.items.length <= 256);
      const ids = new Set(); let active = 0;
      for (const item of data.items) { validProject(item); check(!ids.has(item.application_id)); ids.add(item.application_id); if (item.active) active += 1; }
      check(active <= 1);
    } else if (kind === "command") validReceipt(data);
    else if (kind === "log") check(closed(data, ["name", "offset", "next_offset", "text", "complete"]) && text(data.name, 256) && int(data.offset) && data.offset >= 0 && int(data.next_offset) && data.next_offset >= data.offset && text(data.text, 65536) && bool(data.complete));
    else fail("unknown response kind");
    return structuredClone(value);
  }

  // Closed argument allowlists in fingerprint order (brief §2.7), with the
  // grammar each verb enforces so an invalid value never leaves the browser.
  const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/, SECRET = /^[A-Z_][A-Z0-9_]*$/, REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/, LOGIN = /^[A-Za-z0-9-]{1,39}$/, TARGET = /^[^@\s/]+@[^@\s/]+$/;
  const path = value => safe(value, 4096) && value.startsWith("/");
  const optional = (test) => value => value === "" || test(value);
  const word = value => safe(value) && value.length > 0 && !/\s/.test(value);
  const FORMS = [
    { operation: "dna.project.create", title: "Create project", submit: "Create project", fields: [
      { key: "parent", label: "Parent directory", kind: "text", prefill: head => head.projects_dir, valid: path, hint: "absolute directory; the project is created under it" },
      { key: "name", label: "Project name", kind: "text", valid: value => NAME.test(value), hint: "lowercase letters, digits, dot, underscore or dash; up to 64" },
      { key: "profile", label: "Profile", kind: "select", options: [["local", "local"], ["remote-body", "remote body"]] },
      { key: "remote", label: "Record remote (optional)", kind: "text", valid: optional(word) },
      { key: "body", label: "Body host (optional)", kind: "text", valid: optional(value => TARGET.test(value)), hint: "user@host" },
      { key: "discover", label: "Discover the toolchain while creating", kind: "checkbox" }] },
    { operation: "dna.project.init", title: "Initialize project", submit: "Initialize project", fields: [
      { key: "root", label: "Project root", kind: "text", valid: path, hint: "absolute path of an existing checkout without a Record" }] },
    { operation: "dna.project.attach", title: "Attach project", submit: "Attach project", fields: [
      { key: "root", label: "Project root", kind: "text", valid: path, hint: "absolute path of a Git toplevel holding refs/dna/journal" }] },
    { operation: "dna.project.detach", title: "Detach project", submit: "Detach project", project: true, fields: [] },
    { operation: "dna.project.sync", title: "Sync record", submit: "Sync record", project: true, fields: [] },
    { operation: "dna.project.publish", title: "Publish genome", submit: "Publish genome", project: true, fields: [
      { key: "remote", label: "Remote URL (optional when origin exists)", kind: "text", valid: optional(word) },
      { key: "message", label: "Commit message", kind: "textarea", valid: value => text(value, 512) && value.length > 0 && !/[\u0000-\u0008\u000b-\u001f\u007f]/.test(value) && !value.startsWith("-"), hint: "1 to 512 bytes" }] },
    { operation: "dna.forge.configure", title: "Configure forge", submit: "Configure forge", project: true, fields: [
      { key: "github", label: "GitHub repository (owner/repo, empty to unset)", kind: "text", valid: value => value === "" || REPO.test(value) },
      { key: "board", label: "Board logins (comma separated)", kind: "list", valid: value => value.every(login => LOGIN.test(login)) }] },
    { operation: "dna.forge.sync", title: "Sync forge", submit: "Sync forge", project: true, fields: [] },
    { operation: "dna.body.local.start", title: "Start local body", submit: "Start local body", project: true, fields: [
      { key: "mode", label: "Mode", kind: "select", options: [["run", "run"], ["dev", "dev"]] },
      { key: "owner", label: "Owner (optional)", kind: "text", valid: optional(word) }] },
    { operation: "dna.body.local.stop", title: "Stop local body", submit: "Stop local body", project: true, fields: [] },
    { operation: "dna.body.provision", title: "Provision body", submit: "Provision body", preview: "Preview provisioning", project: true, fields: [
      { key: "target", label: "Target (user@host)", kind: "text", valid: value => TARGET.test(value) },
      { key: "dsn", label: "Knowledge DSN (optional)", kind: "text", valid: optional(word) },
      { key: "dir", label: "Remote directory (optional)", kind: "text", valid: optional(path) },
      { key: "dry_run", kind: "submitter" }] },
    { operation: "dna.body.start", title: "Start remote body", submit: "Start remote body", project: true, fields: [{ key: "body", label: "Body (optional user@host)", kind: "text", valid: optional(value => TARGET.test(value)) }] },
    { operation: "dna.body.stop", title: "Stop remote body", submit: "Stop remote body", project: true, fields: [{ key: "body", label: "Body (optional user@host)", kind: "text", valid: optional(value => TARGET.test(value)) }] },
    { operation: "dna.body.logs", title: "Remote body logs", submit: "Read remote body logs", project: true, fields: [{ key: "body", label: "Body (optional user@host)", kind: "text", valid: optional(value => TARGET.test(value)) }] },
    { operation: "dna.secret.set", title: "Set secret", submit: "Set secret", project: true, secret: true, fields: [
      { key: "name", label: "Secret name", kind: "secret-name", valid: value => SECRET.test(value) && bytes(value) <= 256 },
      { key: "source", label: "Source", kind: "secret-source" },
      { key: "body", label: "Body (optional user@host)", kind: "text", valid: optional(value => TARGET.test(value)) }] },
    { operation: "dna.secret.rotate", title: "Rotate secret", submit: "Rotate secret", project: true, secret: true, fields: [
      { key: "name", label: "Secret name", kind: "secret-name", valid: value => SECRET.test(value) && bytes(value) <= 256 },
      { key: "source", label: "Source", kind: "secret-source" },
      { key: "body", label: "Body (optional user@host)", kind: "text", valid: optional(value => TARGET.test(value)) }] },
    { operation: "dna.models.probe", title: "Probe models", submit: "Probe models", project: true, fields: [
      { key: "confirm_spend", label: "I confirm this probe may spend model credits", kind: "checkbox", gate: true }] },
    { operation: "dna.connection.propose", title: "Propose connection", submit: "Propose connection", project: true, fields: [
      { key: "record_url", label: "Peer Record URL", kind: "text", valid: word },
      { key: "name", label: "Connection name", kind: "text", valid: word },
      { key: "position", label: "Position (our side)", kind: "text", valid: word },
      { key: "purpose", label: "Purpose", kind: "text", valid: value => safe(value, 4096) && value.length > 0 },
      { key: "classes", label: "Classes (comma separated)", kind: "list", valid: value => value.every(word) }] },
    { operation: "dna.connection.close", title: "Close connection", submit: "Close connection", project: true, fields: [
      { key: "name", label: "Connection name", kind: "text", valid: word },
      { key: "why", label: "Why", kind: "text", valid: value => safe(value, 4096) && value.length > 0 }] },
    { operation: "dna.handoff.publish", title: "Publish handoff", submit: "Publish handoff", project: true, fields: [
      { key: "connection", label: "Connection", kind: "text", valid: word },
      { key: "kind", label: "Kind", kind: "select", options: [["task", "task"], ["receipt", "receipt"]] },
      { key: "subject", label: "Subject", kind: "text", valid: value => safe(value, 4096) && value.length > 0 },
      { key: "note", label: "Note (optional)", kind: "text", valid: optional(value => safe(value, 4096)) }] },
    { operation: "dna.handoff.accept", title: "Accept handoff", submit: "Accept handoff", project: true, fields: [
      { key: "id", label: "Handoff id", kind: "text", valid: word },
      { key: "note", label: "Note (optional)", kind: "text", valid: optional(value => safe(value, 4096)) }] },
    { operation: "dna.handoff.sync", title: "Sync handoffs", submit: "Sync handoffs", project: true, fields: [] }
  ];
  const STAGES = { queued: "Queued in this browser", recorded: "Recorded", admitted: "Admitted", refused: "Refused", running: "Running", succeeded: "Succeeded", failed: "Failed", outcome_unknown: "Outcome unknown" };

  const el = (tag, className = "", value) => { const node = document.createElement(tag); if (className) node.className = className; if (value !== undefined) node.textContent = String(value); return node; };
  const append = (node, ...children) => { node.append(...children.filter(Boolean)); return node; };
  const button = (label, click, className = "button secondary") => { const node = el("button", className, label); node.type = "button"; node.addEventListener("click", click); return node; };
  const fact = (list, label, value, mono = false) => list.append(append(el("div"), el("dt", "", label), el("dd", mono ? "mono" : "", value)));
  const since = value => value > 0 ? new Date(value * 1000).toLocaleString() : "";
  const recoveryKey = principal => STORAGE + encodeURIComponent(JSON.stringify([principal.mode, principal.name]));

  let current = null;
  function mount(host, { head = null, principal = null, onHead = () => {}, onAttached = () => {}, onInvalidate = () => {} } = {}) {
    current?.destroy();
    let disposed = false, version = 0, timer = null, refreshing = false;
    let state = head ? { principal: head.principal, data: head.data } : null, whom = principal || head?.principal || null;
    let projects = [], detail = null, unavailable = "", logs = { name: "", text: "", next_offset: 0, complete: true, error: "" };
    let request = null, formsKey = "", stage = "";
    const root = el("section", "projects"); root.setAttribute("role", "region"); root.setAttribute("aria-label", "Projects workspace");
    const heading = append(el("header", "panel-heading projects-heading"), append(el("div"), el("span", "eyebrow", "Project service"), el("h2", "", "Projects on this machine")));
    const status = el("p", "projects-status", "Reading the project service…"); status.setAttribute("role", "status"); status.tabIndex = -1;
    const refreshButton = button("Refresh projects", () => refresh());
    heading.append(refreshButton);
    const requestPanel = el("section", "projects-request intervention-recovery"); requestPanel.id = "projects-request"; requestPanel.setAttribute("role", "region"); requestPanel.setAttribute("aria-label", "Project request"); requestPanel.hidden = true;
    const summary = el("dl", "fact-grid projects-summary");
    const registry = el("section", "panel projects-registry"); registry.setAttribute("aria-label", "Project registry");
    const forms = el("div", "projects-forms");
    const logPanel = el("section", "panel projects-logs"); logPanel.setAttribute("aria-label", "Head logs");
    root.append(heading, status, requestPanel, summary, registry, forms, logPanel);
    host.replaceChildren(root);

    const failWith = (message, status = 0, code = "") => Object.assign(new Error(message), { status, code });
    async function exchange(method, url, body = null) {
      const active = new AbortController(); const timer = setTimeout(() => active.abort(), FETCH_TIMEOUT_MS);
      try {
        const response = await fetch(url, { method, credentials: "same-origin", cache: "no-store", signal: active.signal, headers: body ? { Accept: "application/json", "Content-Type": "application/json", "X-Hale-Command": "1" } : { Accept: "application/json" }, ...(body ? { body: JSON.stringify(body) } : {}) });
        const raw = await response.text();
        if (bytes(raw) > 2097152) throw failWith("The project service response exceeds the browser limit.", response.status);
        let value;
        try { value = JSON.parse(raw); } catch { throw failWith("The project service did not return a readable JSON response.", response.status, "invalid_response"); }
        if (!response.ok) {
          const error = value?.error;
          throw failWith(typeof error?.message === "string" ? error.message : "The request did not complete.", response.status, typeof error?.code === "string" ? error.code : "request_failed");
        }
        return { status: response.status, value };
      } catch (error) {
        if (active.signal.aborted && !disposed) throw failWith("The project service did not answer within 35 seconds.", 0, "read_timeout");
        if (error.status === undefined) throw failWith("The project service could not be reached.", 0, "connection_failed");
        throw error;
      } finally { clearTimeout(timer); }
    }
    function invalidating(error) { return error.status === 401 || error.status === 403 || error.code === "command_context_changed"; }
    async function readHead() {
      const { value } = await exchange("GET", HEAD);
      const envelope = validate(value, "head");
      if (whom && (envelope.head.principal.mode !== whom.mode || envelope.head.principal.name !== whom.name)) throw failWith("The project service principal changed. Reload before continuing.", 409, "command_context_changed");
      whom = envelope.head.principal;
      state = { principal: envelope.head.principal, data: envelope.data };
      onHead(state);
      return state.data;
    }
    async function readProjects(id = "") {
      const { value } = await exchange("GET", HEAD + "/projects" + (id ? "?id=" + encodeURIComponent(id) : ""));
      const items = validate(value, "projects").data.items;
      if (id) { if (items.length !== 1 || items[0].application_id !== id) fail("The project service returned a different project."); return items[0]; }
      return items;
    }
    async function refresh() {
      if (disposed || refreshing) return;
      refreshing = true; refreshButton.disabled = true; const token = ++version;
      try {
        const data = await readHead(); if (disposed || token !== version) return;
        projects = await readProjects(); if (disposed || token !== version) return;
        detail = data.state === "attached" ? await readProjects(data.active.application_id) : null; if (disposed || token !== version) return;
        unavailable = "";
        if (!request) restore();
      } catch (error) {
        if (disposed || token !== version) return;
        if (invalidating(error)) { onInvalidate(error); return; }
        state = state && error.status !== 404 ? state : null; projects = []; detail = null;
        unavailable = error.status === 404 ? "no_head" : error.message;
      } finally { if (!disposed) { refreshing = false; refreshButton.disabled = false; if (token === version) { render(); schedule(); } } }
    }
    function operation(name) { return state?.data.operations.find(entry => entry.name === name) || null; }
    function slotKey() { return whom ? recoveryKey(whom) : ""; }
    function readSlot() {
      const key = slotKey(); if (!key) return null;
      try {
        const raw = localStorage.getItem(key); if (raw === null) return null;
        const slot = JSON.parse(raw);
        if (!closed(slot, ["version", "request_id", "operation", "target"]) || slot.version !== 1 || !safe(slot.request_id, 128) || !slot.request_id.length || !/^dna\.[a-z][a-z_.]*$/.test(slot.operation) || !closed(slot.target, ["kind", "id"])) throw new Error("invalid slot");
        return slot;
      } catch { return { blocked: true }; }
    }
    function clearSlot(requestId) {
      const key = slotKey(); if (!key) return;
      try { const raw = localStorage.getItem(key); if (raw !== null && JSON.parse(raw).request_id === requestId) localStorage.removeItem(key); } catch { /* storage unavailable: nothing to clear */ }
    }
    function restore() {
      const slot = readSlot(); if (!slot) return;
      if (slot.blocked) { request = { phase: "blocked", error: "The saved request identity cannot be verified. Submission is blocked so an unresolved request cannot be replaced.", receipt: null, payload: null, observation: "none" }; return; }
      request = { phase: "recovering", payload: { request_id: slot.request_id, operation: slot.operation, target: slot.target, arguments: {} }, receipt: null, error: "", observation: "pending", restored: true }; stage = "";
      void lookup();
    }
    async function lookup() {
      if (!request || disposed) return;
      const id = request.payload.request_id;
      try {
        const { value } = await exchange("GET", HEAD + "/commands?request_id=" + encodeURIComponent(id));
        if (disposed || !request || request.payload.request_id !== id) return;
        const receipt = validate(value, "command").data;
        if (receipt.request_id !== id || receipt.operation !== request.payload.operation) fail("The recovered request identity changed.");
        accept(receipt);
      } catch (error) {
        if (disposed || !request || request.payload.request_id !== id) return;
        if (invalidating(error)) { onInvalidate(error); return; }
        request.phase = "uncertain";
        request.error = error.status === 404 ? "No receipt was found for the saved request. It may still be arriving; check again before discarding its identity. Nothing is resubmitted." : error.status === 503 || error.status === 504 ? "The project service is unavailable. The saved request keeps its identity; check its status when the service answers." : error.message;
        render(); schedule();
      }
    }
    function accept(receipt) {
      if (request.receipt?.state !== receipt.state) stage = "";
      request.receipt = receipt; request.error = ""; request.polls = 0;
      request.phase = TERMINAL.has(receipt.state) ? "terminal" : "following";
      if (request.inspected) { request.observation = "none"; render(); schedule(); return; }
      render();
      if (TERMINAL.has(receipt.state)) void observe(); else schedule();
    }
    // An effect is shown only after the head is read again and reports it.
    // A terminal receipt alone never sets the observed attribute.
    async function observe() {
      const r = request.receipt, args = request.payload.arguments || {};
      try {
        const data = await readHead(); if (disposed || request?.receipt !== r) return;
        projects = await readProjects(); if (disposed || request?.receipt !== r) return;
        detail = data.state === "attached" ? await readProjects(data.active.application_id) : null; if (disposed || request?.receipt !== r) return;
        unavailable = "";
        request.observation = r.state === "succeeded" ? (effect(r, args, data) ? "observed" : "unobserved") : "none";
      } catch (error) {
        if (disposed || request?.receipt !== r) return;
        if (invalidating(error)) { onInvalidate(error); return; }
        request.observation = "unread"; request.error = "The head could not be read again: " + error.message;
      }
      clearSlot(r.request_id);
      render(); schedule();
      if (request.observation === "observed" && ATTACHING.has(r.operation) && hex40(r.outcome.application_id)) onAttached(r.outcome.application_id);
    }
    function effect(r, args, data) {
      const o = r.outcome, project = detail, recordMoved = () => !o.record || typeof o.record !== "object" || (project?.record?.head === o.record.head_after);
      switch (r.operation) {
        case "dna.project.create": case "dna.project.init": case "dna.project.attach": return data.state === "attached" && data.active.application_id === o.application_id && projects.some(item => item.application_id === o.application_id && item.active);
        case "dna.project.detach": return data.state === "detached";
        case "dna.project.forget": return !projects.some(item => item.application_id === args.application_id);
        case "dna.forge.configure": return Boolean(project) && project.forge.github === o.github && JSON.stringify(project.forge.board) === JSON.stringify(o.board);
        case "dna.body.local.start": return data.children.body.state === "running";
        case "dna.body.local.stop": return data.children.body.state !== "running";
        case "dna.secret.set": case "dna.secret.rotate": return Boolean(project) && project.secrets.includes(o.name) && recordMoved();
        case "dna.connection.propose": return Boolean(project) && project.connections.some(c => c.name === o.name) && recordMoved();
        case "dna.connection.close": return Boolean(project) && !project.connections.some(c => c.name === o.name && c.state !== "closed") && recordMoved();
        default: return Boolean(project) && recordMoved();
      }
    }
    // Non-terminal receipts and a starting API child are followed every two
    // seconds; an unconfirmed submission is looked up a bounded number of
    // times, then waits for an explicit check. Nothing here resubmits.
    function schedule() {
      if (timer) { clearTimeout(timer); timer = null; }
      if (disposed || document.hidden) return;
      const following = request?.phase === "following" || (request?.phase === "uncertain" && !request.receipt && (request.polls || 0) < 15);
      const starting = state?.data.state === "attached" && state.data.active.api.state === "starting";
      if (!following && !starting) return;
      timer = setTimeout(() => {
        timer = null; if (disposed) return;
        if (following) { request.polls = (request.polls || 0) + 1; void lookup(); }
        if (starting) void readHead().then(() => { if (!disposed) { render(); schedule(); } }).catch(() => { if (!disposed) schedule(); });
      }, POLL_MS);
    }
    async function submit(form, values, submitter) {
      if (disposed || !state || request || refreshing) return;
      const op = operation(form.operation);
      if (!op?.available) { showProblems(form, [["", "The head reports this operation unavailable" + (op?.reason_code ? " (" + op.reason_code + ")" : "") + "."]]); return; }
      if (state.data.busy) { showProblems(form, [["", "The head is running " + state.data.busy + "; wait for it to settle."]]); return; }
      const problems = [];
      const args = {};
      for (const field of form.fields) {
        if (field.kind === "submitter") { args[field.key] = submitter === "preview"; continue; }
        const value = values[field.key];
        if (field.kind === "secret-source") {
          if (!value) { problems.push([field.key, "Choose a secret source: a 0600 file under the sources directory or an exported variable."]); continue; }
          const [kind, name] = [value.slice(0, value.indexOf(":")), value.slice(value.indexOf(":") + 1)];
          const known = kind === "file" ? state.data.credentials.file_sources : kind === "env" ? state.data.credentials.env_present : [];
          if (!known.includes(name)) { problems.push([field.key, "The source is not one the head advertises."]); continue; }
          args.source = { kind, name }; continue;
        }
        if (field.kind === "checkbox") { args[field.key] = value === true; if (field.gate && value !== true) problems.push([field.key, "Confirm the spend before probing."]); continue; }
        if (field.valid && !field.valid(value)) { problems.push([field.key, field.hint ? "Expected " + field.hint + "." : "This value is not accepted."]); continue; }
        args[field.key] = value;
      }
      if (problems.length) { showProblems(form, problems); return; }
      showProblems(form, []);
      const headScoped = HEAD_SCOPED.has(form.operation), applicationId = state.data.active?.application_id || "";
      if (!headScoped && !applicationId) { showProblems(form, [["", "No project is attached."]]); return; }
      if (typeof navigator.locks?.request !== "function" || typeof crypto.randomUUID !== "function") { showProblems(form, [["", "This browser cannot safely reserve a recoverable request. Nothing was submitted."]]); return; }
      const payload = { request_id: "head-" + crypto.randomUUID(), operation: form.operation, operation_version: "1", context: { head: "local", application_id: headScoped ? "" : applicationId }, target: headScoped ? { kind: "dna.head", id: "local" } : { kind: "dna.project", id: applicationId }, preconditions: { principal: { mode: whom.mode, name: whom.name } }, arguments: args };
      if (bytes(JSON.stringify(payload)) > MAX_REQUEST_BYTES) { showProblems(form, [["", "The JSON-encoded request exceeds 32768 bytes. Nothing was sent or saved."]]); return; }
      const key = slotKey(); let reservation;
      try {
        reservation = await navigator.locks.request(key, { mode: "exclusive", ifAvailable: true }, lock => {
          if (!lock) return { error: "Another tab is reserving a request. Nothing was submitted; check that tab first." };
          if (localStorage.getItem(key) !== null) return { existing: true };
          const slot = JSON.stringify({ version: 1, request_id: payload.request_id, operation: payload.operation, target: payload.target });
          localStorage.setItem(key, slot);
          if (localStorage.getItem(key) !== slot) throw new Error("recovery identity was not persisted");
          return { ok: true };
        });
      } catch { reservation = { error: "The request identity could not be saved. Nothing was submitted; enable browser storage first." }; }
      if (disposed) return;
      if (reservation.error) { showProblems(form, [["", reservation.error]]); return; }
      if (reservation.existing) { restore(); render(); return; }
      request = { phase: "queued", payload, receipt: null, error: "", observation: "pending" }; stage = "";
      render(); requestPanel.focus?.();
      try {
        const { value } = await exchange("POST", HEAD + "/commands", payload);
        if (disposed || request?.payload !== payload) return;
        const receipt = validate(value, "command").data;
        if (receipt.request_id !== payload.request_id || receipt.operation !== payload.operation) fail("The receipt does not describe this request.");
        accept(receipt);
      } catch (error) {
        if (disposed || request?.payload !== payload) return;
        if (invalidating(error)) { onInvalidate(error); return; }
        if ([400, 409, 413, 415].includes(error.status)) {
          // Refused before it was recorded: the head holds nothing to recover.
          clearSlot(payload.request_id); request = null; render();
          showProblems(form, [["", "The project service refused the request (" + error.code + "): " + error.message]]);
          return;
        }
        request.phase = "uncertain"; request.error = "The request outcome is unavailable (" + error.code + "). Delivery may have occurred; its identity is retained and checked again, never resubmitted.";
        render(); schedule();
      }
    }
    function showProblems(form, problems) {
      const node = forms.querySelector('form[data-operation="' + form.operation + '"]');
      if (!node) { if (problems.length) { status.textContent = problems.map(([, message]) => message).join(" "); status.focus({ preventScroll: true }); } return; }
      const list = node.querySelector(".projects-problems"); list.replaceChildren();
      for (const input of node.querySelectorAll("[aria-invalid]")) input.removeAttribute("aria-invalid");
      for (const [key, message] of problems) {
        list.append(el("li", "", message));
        const input = key && node.querySelector('[name="' + key + '"]'); if (input) input.setAttribute("aria-invalid", "true");
      }
      list.hidden = !problems.length;
    }
    async function discard() {
      if (!request || request.phase !== "uncertain" || request.receipt) return;
      clearSlot(request.payload.request_id); request = null; render();
    }
    function dismiss() { if (!request || (!request.inspected && !["terminal", "blocked"].includes(request.phase))) return; if (request.receipt && !request.inspected) clearSlot(request.receipt.request_id); request = null; render(); }
    async function readLog(query, offset = 0) {
      const token = version;
      try {
        const { value } = await exchange("GET", HEAD + "/logs?" + query + (offset ? "&offset=" + offset : ""));
        if (disposed || token !== version) return;
        const data = validate(value, "log").data;
        logs = { name: data.name, query, text: (offset && logs.query === query ? logs.text : "") + data.text, next_offset: data.next_offset, complete: data.complete, error: "" };
      } catch (error) { if (disposed || token !== version) return; if (invalidating(error)) { onInvalidate(error); return; } logs = { ...logs, query, error: error.message }; }
      renderLogs();
    }

    function render() {
      renderStatus(); renderRequest(); renderSummary(); renderRegistry(); renderForms(); renderLogs();
    }
    function renderStatus() {
      if (!state) { status.textContent = unavailable === "no_head" ? "No project service is running behind this API. Start the face through start.sh to manage projects here; Record reads stay available." : unavailable ? "Project service unavailable: " + unavailable : "Reading the project service…"; return; }
      const d = state.data;
      status.textContent = (d.state === "attached" ? "Attached to " + d.active.name + " · API " + d.active.api.state : "No project attached") + " · principal " + state.principal.name + (d.busy ? " · running " + d.busy : "") + (unavailable ? " · last read failed: " + unavailable : "");
    }
    function renderSummary() {
      summary.replaceChildren(); summary.hidden = !state; if (!state) return;
      const d = state.data;
      fact(summary, "Head state", d.state);
      fact(summary, "Active project", d.state === "attached" ? d.active.name + " · " + d.active.root : "none");
      fact(summary, "Body", d.children.body.state + (d.children.body.mode ? " · " + d.children.body.mode : "") + (d.children.body.state === "running" ? (d.children.body.membrane_up ? " · membrane up" : " · membrane not up") : "") + (d.children.body.state === "exited" ? " · exit " + d.children.body.exit_code : ""));
      fact(summary, "State directory", d.state_dir, true);
      fact(summary, "Secret sources", d.sources_dir + (d.credentials.file_sources.length ? " · " + d.credentials.file_sources.join(", ") : " · none"), true);
      fact(summary, "Credentials needed", d.credentials.needed.length ? d.credentials.needed.map(name => name + (d.credentials.env_present.includes(name) ? " (exported)" : "")).join(", ") : "none declared");
      fact(summary, "Busy", d.busy || "no", d.busy.length > 0);
    }
    function renderRegistry() {
      registry.replaceChildren(); registry.hidden = !state; if (!state) return;
      registry.append(append(el("header", "panel-heading"), el("h2", "", "Registered projects"), el("span", "", projects.length + " project(s)")));
      if (!projects.length) { registry.append(el("p", "detail-note projects-empty", "No project is registered on this machine yet. Create one, initialize a checkout, or attach an existing project below.")); return; }
      const list = el("ul", "record-list");
      for (const item of projects) {
        const li = el("li"); li.dataset.applicationId = item.application_id;
        const row = el("div", "projects-project");
        const line = append(el("div", "record-line"), el("strong", "record-name", item.name), append(el("span", "badge " + (item.active ? "green" : "")), document.createTextNode(item.active ? "active" : "registered")));
        const meta = el("p", "record-meta");
        meta.append(el("code", "", item.application_id), document.createTextNode(" · " + item.root + (item.record ? " · record " + item.record.head.slice(0, 12) + " · revision " + item.record.revision : " · record unread") + " · body " + item.body.state + (item.forge.github ? " · forge " + item.forge.github : "")));
        row.append(line, meta);
        if (item.recent.length) {
          const recent = el("ul", "projects-recent"); recent.setAttribute("aria-label", "Recent receipts for " + item.name);
          for (const entry of item.recent) recent.append(append(el("li"), el("code", "", entry.operation), document.createTextNode(" · " + entry.state + " · "), button("Inspect " + entry.request_id, () => { if (request) return; request = { phase: "recovering", payload: { request_id: entry.request_id, operation: entry.operation, target: { kind: "", id: "" }, arguments: {} }, receipt: null, error: "", observation: "none", inspected: true }; render(); void lookup(); }, "text-link projects-inspect")));
          row.append(recent);
        }
        const actions = el("div", "intervention-actions");
        if (item.active) actions.append(button("Detach project", () => submit(FORMS.find(form => form.operation === "dna.project.detach"), {}, "")));
        else {
          actions.append(button("Attach " + item.name, () => submit(FORMS.find(form => form.operation === "dna.project.attach"), { root: item.root }, "")));
          actions.append(button("Forget " + item.name, () => submit({ operation: "dna.project.forget", fields: [{ key: "application_id", valid: hex40 }] }, { application_id: item.application_id }, "")));
        }
        for (const control of actions.querySelectorAll("button")) control.disabled = Boolean(request) || Boolean(state.data.busy);
        row.append(actions);
        li.append(row); list.append(li);
      }
      registry.append(list);
      if (detail) {
        const panel = el("section", "projects-detail"); panel.setAttribute("aria-label", "Active project detail");
        const facts = el("dl", "fact-grid");
        fact(facts, "Authority files", ["command", "task", "organization"].filter(kind => detail.authority[kind]).join(", ") || "none decodable");
        fact(facts, "Secrets set", detail.secrets.length ? detail.secrets.join(", ") : "none");
        fact(facts, "Connections", detail.connections.length ? detail.connections.map(c => c.name + " (" + c.state + ")").join(", ") : "none");
        fact(facts, "Handoffs", detail.handoffs.length ? detail.handoffs.map(h => h.id + " · " + h.kind + " · " + h.state).join(", ") : "none");
        panel.append(el("h3", "", "Active project"), facts);
        registry.append(panel);
      }
    }
    // Forms are rebuilt only when the head's shape changes, so typed values
    // survive receipt polling; availability is synchronized on every render.
    function renderForms() {
      const d = state?.data;
      const key = d ? JSON.stringify([d.state, d.active?.application_id || "", d.operations, d.credentials]) : "none";
      if (key !== formsKey) {
        formsKey = key; forms.replaceChildren();
        if (d) {
          const onboarding = el("section", "panel projects-onboarding"); onboarding.setAttribute("aria-label", "Onboarding");
          onboarding.append(append(el("header", "panel-heading"), el("h2", "", d.state === "attached" ? "Another project" : "Bring a project here"), el("span", "", "create · init · attach")));
          const operate = el("section", "panel projects-operations"); operate.setAttribute("aria-label", "Active project operations");
          operate.append(append(el("header", "panel-heading"), el("h2", "", d.state === "attached" ? "Operate " + d.active.name : "Operations"), el("span", "", d.state === "attached" ? "every action is the CLI verb" : "attach a project to operate it")));
          for (const form of FORMS) {
            if (form.operation === "dna.project.detach" || form.operation === "dna.project.forget") continue;
            (HEAD_SCOPED.has(form.operation) ? onboarding : operate).append(renderForm(form, d));
          }
          forms.append(onboarding, operate);
        }
      }
      const blocked = !d ? "" : request ? "Resolve the request above before submitting another." : d.busy ? "The head is running " + d.busy + "; wait for it to settle." : "";
      for (const node of forms.querySelectorAll("form")) node.sync(blocked);
    }
    function renderForm(form, d) {
      const op = operation(form.operation);
      const node = el("form", "projects-form"); node.dataset.operation = form.operation; node.setAttribute("aria-label", form.title); node.noValidate = true;
      const fieldset = el("fieldset"); fieldset.append(el("legend", "", form.title));
      const reason = !op ? "not offered by this head" : op.available ? "" : op.reason_code || "unavailable";
      const note = el("p", "detail-note projects-reason"); note.hidden = true; fieldset.append(note);
      let gate = null;
      for (const field of form.fields) {
        if (field.kind === "submitter") continue;
        const id = "projects-" + form.operation.replaceAll(".", "-") + "-" + field.key;
        // The label is a sibling, never the control's parent, so the control's
        // accessible name is the label text alone and not its current value.
        const wrapper = el("div", "projects-field"), label = el("label"); label.htmlFor = id;
        let input;
        if (field.kind === "select") { input = el("select"); for (const [value, caption] of field.options) { const option = el("option", "", caption); option.value = value; input.append(option); } }
        else if (field.kind === "checkbox") { input = el("input"); input.type = "checkbox"; if (field.gate) gate = input; }
        else if (field.kind === "textarea") { input = el("textarea"); input.rows = 3; }
        else if (field.kind === "secret-name") {
          input = el("select"); const custom = el("option", "", "Type a name below"); custom.value = ""; input.append(custom);
          for (const name of d.credentials.needed) { const option = el("option", "", name); option.value = name; input.append(option); }
        } else if (field.kind === "secret-source") {
          input = el("select");
          for (const name of d.credentials.file_sources) { const option = el("option", "", "file · " + name); option.value = "file:" + name; input.append(option); }
          for (const name of d.credentials.env_present) { const option = el("option", "", "environment · " + name); option.value = "env:" + name; input.append(option); }
          if (!input.options.length) { const none = el("option", "", "No sources advertised"); none.value = ""; input.append(none); input.disabled = true; }
        } else { input = el("input"); input.type = "text"; input.autocomplete = "off"; input.spellcheck = false; if (field.prefill) input.value = field.prefill(d); }
        input.id = id; input.name = field.key;
        if (field.kind === "checkbox") { wrapper.className = "projects-field projects-check"; label.textContent = field.label; wrapper.append(input, label); }
        else { label.textContent = field.label + (field.hint ? " · " + field.hint : ""); wrapper.append(label, input); }
        fieldset.append(wrapper);
        if (field.kind === "secret-name") {
          const customId = id + "-custom", customWrapper = el("div", "projects-field"), customLabel = el("label", "", "Custom secret name · upper case letters, digits and underscore"); customLabel.htmlFor = customId;
          const customInput = el("input"); customInput.type = "text"; customInput.id = customId; customInput.name = "name_custom"; customInput.autocomplete = "off"; customInput.spellcheck = false;
          customWrapper.append(customLabel, customInput); fieldset.append(customWrapper);
        }
      }
      if (form.secret) fieldset.append(el("p", "detail-note", "The value never enters the browser: the head pipes the named file or exported variable into the CLI verb."));
      const problems = el("ul", "projects-problems intervention-error"); problems.setAttribute("role", "alert"); problems.hidden = true;
      const actions = el("div", "intervention-actions");
      const submitButton = el("button", "button primary", form.submit); submitButton.type = "submit"; submitButton.value = "submit";
      actions.append(submitButton);
      if (form.preview) { const previewButton = el("button", "button secondary", form.preview); previewButton.type = "submit"; previewButton.value = "preview"; actions.prepend(previewButton); }
      let blocked = "";
      const sync = () => {
        const message = reason ? "Unavailable: " + reason + "." : blocked;
        note.textContent = message; note.hidden = !message;
        for (const control of actions.querySelectorAll("button")) control.disabled = Boolean(message) || (gate ? !gate.checked : false);
      };
      node.sync = value => { blocked = value; sync(); };
      if (gate) gate.addEventListener("change", sync);
      sync();
      fieldset.append(problems, actions);
      node.append(fieldset);
      node.addEventListener("submit", event => {
        event.preventDefault();
        const values = {};
        for (const field of form.fields) {
          if (field.kind === "submitter") continue;
          const input = node.elements.namedItem(field.key);
          if (field.kind === "checkbox") values[field.key] = input.checked;
          else if (field.kind === "list") values[field.key] = input.value.split(",").map(item => item.trim()).filter(Boolean);
          else if (field.kind === "secret-name") { const custom = node.elements.namedItem("name_custom").value.trim(); values[field.key] = custom || input.value; }
          else values[field.key] = input.value;
        }
        void submit(form, values, event.submitter?.value || "submit");
      });
      return node;
    }
    function renderRequest() {
      requestPanel.replaceChildren(); requestPanel.hidden = !request; requestPanel.tabIndex = -1;
      if (!request) { requestPanel.removeAttribute("data-observation"); requestPanel.removeAttribute("data-state"); return; }
      const r = request.receipt, payload = request.payload;
      requestPanel.dataset.state = r ? r.state : request.phase;
      requestPanel.dataset.observation = request.observation;
      const head = append(el("div", "intervention-heading"), append(el("div"), el("h3", "", request.inspected ? "Inspected receipt" : request.restored ? "Saved request" : "Request"), el("p", "", payload.operation + " · " + payload.request_id)));
      requestPanel.append(head);
      const body = el("div", "intervention-body");
      if (request.phase === "blocked") { body.append(el("p", "intervention-status uncertain", request.error), append(el("div", "intervention-actions"), button("Dismiss", dismiss))); requestPanel.append(body); return; }
      const stages = [];
      const tone = value => ["succeeded", "admitted", "recorded", "queued"].includes(value) ? "confirmed" : ["refused", "failed"].includes(value) ? "refused" : value === "outcome_unknown" ? "unknown" : "pending";
      const reached = key => { const order = ["queued", "recorded", "admitted", "running", "succeeded"]; const at = r ? (r.state === "refused" ? "admitted" : ["failed", "outcome_unknown"].includes(r.state) ? "succeeded" : r.state) : "queued"; return order.indexOf(key) <= order.indexOf(at); };
      stages.push({ key: "queued", title: "Queued", value: STAGES.queued, tone: "confirmed", explanation: "The request identity was reserved in this browser before anything was sent. It is the only thing the browser remembers." });
      stages.push({ key: "recorded", title: "Recorded", value: reached("recorded") ? "Recorded by the head" : "Not yet recorded", tone: reached("recorded") ? "confirmed" : "pending", explanation: "The head journals the request before validating it, so a replay with the same identity returns this receipt instead of running again." });
      const admission = !r ? "pending" : r.state === "refused" ? "refused" : reached("admitted") ? "admitted" : "pending";
      stages.push({ key: "admission", title: "Admission", value: admission === "refused" ? "Refused · " + (r.reason_code || "refused") : admission === "admitted" ? "Admitted" : "Awaiting admission", tone: tone(admission), explanation: admission === "refused" ? (r.reason || "The head refused this request before running anything.") : "Validation and availability are decided by the head under the same rules as the CLI." });
      const running = r && r.run ? r.run.kind + (r.run.pid > 0 ? " · pid " + r.run.pid : "") + (r.run.deadline ? " · deadline " + since(r.run.deadline) : "") : "";
      stages.push({ key: "running", title: "Running", value: r?.state === "running" ? "Running · " + running : r && reached("running") && r.state !== "refused" ? (r.run ? "Ran as " + running : "Ran inline") : "Not started", tone: r?.state === "running" ? "pending" : r && reached("running") && r.state !== "refused" ? "confirmed" : admission === "refused" ? "refused" : "pending", explanation: r?.run?.external ? "This verb reaches outside the machine. A missed deadline yields an unknown outcome, never a claimed success." : "The head runs the CLI verb detached, so restarting the head does not interrupt it." });
      const outcome = !r ? "pending" : TERMINAL.has(r.state) ? r.state : "pending";
      stages.push({ key: "outcome", title: "Outcome", value: outcome === "pending" ? "Not settled" : STAGES[outcome] + (r.run && r.run.exit_code >= 0 && outcome !== "refused" ? " · exit " + r.run.exit_code : ""), tone: tone(outcome), explanation: outcome === "outcome_unknown" ? "The run ended without a recorded exit, or its deadline passed on an external verb. Inspect the run log and the project's Record before submitting again; nothing is retried automatically." : outcome === "succeeded" ? (request.observation === "observed" ? "The head was read again and reports the effect." : request.observation === "unobserved" ? "The receipt says succeeded, but a fresh read of the head does not yet show the effect." : request.observation === "unread" ? "The receipt says succeeded; the head could not be read again to confirm the effect." : "Reading the head again to confirm the effect.") : outcome === "failed" ? (r.reason || "The verb exited non-zero.") : outcome === "refused" ? "Nothing ran." : "The receipt is followed every two seconds until it settles." });
      const map = el("div", "command-outcome-map"); map.style.setProperty("--outcome-stages", String(stages.length));
      const trail = el("ol", "intervention-stage-list"); trail.setAttribute("aria-label", "Request lifecycle");
      const inspector = el("div", "outcome-inspector"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected stage");
      // An explicit choice sticks; otherwise the inspector follows the receipt
      // to the first stage that is refused, unknown or still pending.
      const selected = stages.some(item => item.key === stage) ? stage : (stages.find(item => item.tone === "refused" || item.tone === "unknown" || item.tone === "pending") || stages.at(-1)).key;
      const choose = (key, explicit = false) => {
        if (explicit) stage = key;
        for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === key));
        const item = stages.find(entry => entry.key === key);
        inspector.dataset.state = item.tone;
        inspector.replaceChildren(el("h4", "", item.title), el("p", "outcome-value", item.value), el("p", "", item.explanation));
      };
      for (const [index, item] of stages.entries()) {
        const control = button("", () => choose(item.key, true), "outcome-stage"); control.dataset.stage = item.key; control.dataset.state = item.tone; control.setAttribute("aria-label", item.title);
        const marker = el("span", "outcome-marker", String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true");
        append(control, marker, el("strong", "", item.title), el("span", "outcome-stage-value", item.value));
        trail.append(append(el("li"), control));
      }
      choose(selected);
      body.append(append(map, trail, inspector));
      if (request.error) body.append(el("p", "intervention-status uncertain", request.error));
      if (r) {
        const facts = el("dl", "fact-grid projects-receipt");
        fact(facts, "Command", r.command_id, true); fact(facts, "Request", r.request_id, true); fact(facts, "Operation", r.operation); fact(facts, "Target", r.target.kind + " · " + r.target.id, true);
        fact(facts, "State", r.state + (r.reason_code ? " · " + r.reason_code : "")); fact(facts, "Updated", since(r.updated_at));
        if (r.state === "refused") fact(facts, "Reason", r.reason || "not given");
        if (r.run) { fact(facts, "Run", r.run.kind + " · pid " + r.run.pid + " · exit " + r.run.exit_code + (r.run.external ? " · external" : "")); const logLink = el("a", "text-link", "Run log"); logLink.href = "/api/hale/v1" + r.run.log; logLink.dataset.log = r.run.log; append(facts, append(el("div"), el("dt", "", "Log"), append(el("dd"), logLink))); }
        body.append(facts);
        if (r.reason && r.state !== "refused") body.append(el("pre", "projects-log projects-reason-text", r.reason));
        body.append(renderOutcome(r.outcome));
        const actions = el("div", "intervention-actions");
        if (r.run && r.run.kind === "run") actions.append(button("Show run log", () => readLog("run=" + encodeURIComponent(r.command_id))));
        if (TERMINAL.has(r.state) || request.inspected) actions.append(button("Dismiss", dismiss));
        if (!TERMINAL.has(r.state)) actions.append(button("Check status", () => lookup()));
        body.append(actions);
      } else if (request.phase === "uncertain") {
        const actions = el("div", "intervention-actions");
        actions.append(button("Check status", () => lookup()));
        actions.append(button("Discard unconfirmed identity", discard));
        body.append(actions);
      } else body.append(el("p", "intervention-status", request.phase === "recovering" ? "Looking the saved request up by its identity. No new request is sent." : "Submitting the request to the head…"));
      requestPanel.append(body);
    }
    function renderOutcome(outcome) {
      const section = el("section", "projects-outcome"); section.setAttribute("aria-label", "Outcome");
      const keys = Object.keys(outcome); if (!keys.length) return section;
      const facts = el("dl", "fact-grid");
      for (const key of keys) {
        const value = outcome[key];
        if (key === "record" && value && typeof value === "object") { fact(facts, "Record before", String(value.head_before ?? ""), true); fact(facts, "Record after", String(value.head_after ?? ""), true); fact(facts, "Rows appended", Array.isArray(value.rows) ? value.rows.map(row => row.kind + " · " + row.entity).join(", ") || "none" : "unknown"); continue; }
        if (["preview", "text", "table", "summary", "sync_summary"].includes(key) && typeof value === "string") { section.append(el("h4", "", key), el("pre", "projects-log", value)); continue; }
        fact(facts, key, typeof value === "object" ? JSON.stringify(value) : String(value), key.endsWith("_id") || key === "pushed_ref");
      }
      section.prepend(facts);
      return section;
    }
    function renderLogs() {
      logPanel.replaceChildren(); logPanel.hidden = !state; if (!state) return;
      logPanel.append(append(el("header", "panel-heading"), el("h2", "", "Head logs"), el("span", "", "api · body · runs")));
      const actions = el("div", "intervention-actions");
      for (const child of ["api", "body"]) actions.append(button("Show " + child + " log", () => readLog("child=" + child)));
      logPanel.append(actions);
      if (logs.error) logPanel.append(el("p", "intervention-error", logs.error));
      if (logs.name) {
        const pre = el("pre", "projects-log", logs.text || "(empty)"); pre.setAttribute("aria-label", "Log " + logs.name);
        logPanel.append(el("p", "detail-note", "Log " + logs.name + (logs.complete ? " · complete" : " · more available")), pre);
        if (!logs.complete) logPanel.append(button("Load more", () => readLog(logs.query, logs.next_offset)));
      }
    }
    const visibility = () => { if (document.hidden) { if (timer) { clearTimeout(timer); timer = null; } } else schedule(); };
    document.addEventListener("visibilitychange", visibility);
    render();
    void refresh();
    const controllerObject = { destroy() { if (disposed) return; disposed = true; version += 1; if (timer) clearTimeout(timer); document.removeEventListener("visibilitychange", visibility); host.replaceChildren(); if (current === controllerObject) current = null; } };
    current = controllerObject;
    return controllerObject;
  }
  function destroy() { current?.destroy(); current = null; }
  window.IrisProjects = Object.freeze({ validate, mount, destroy, FORMS: Object.freeze(FORMS.map(form => Object.freeze({ operation: form.operation, title: form.title, fields: form.fields.map(field => field.key) }))) });
})();
