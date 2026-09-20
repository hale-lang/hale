/* Native handed-Task inspection and reassignment preparation.
 * This module performs no requests, writes or persistence. */
"use strict";
(() => {
  const encoder = new TextEncoder();
  const FIELDS = ["id", "outcome", "state", "assignee", "obligation", "acceptance_digest", "acceptance_bound", "evidence_required", "evidence_ref", "waiting", "assignment_digest", "reassignment_supported", "history"];
  const HISTORY = ["event_id", "sequence", "kind", "from", "to", "by"];
  const closed = (value, fields) => value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === fields.length && fields.every(key => Object.hasOwn(value, key));
  const text = (value, max) => typeof value === "string" && encoder.encode(value).length <= max && !value.includes("\u0000") && !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(value);
  const person = value => text(value, 256) && value.length > 0 && !/[\u0000-\u001f\u007f]/.test(value);
  const sha = value => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value);
  const eventId = value => typeof value === "string" && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value);
  const decimal = value => typeof value === "string" && /^(0|[1-9][0-9]{0,18})$/.test(value) && BigInt(value) <= 9223372036854775807n;
  function validate(row) {
    const check = condition => { if (!condition) throw new Error("The handed Task and its assignment history could not be verified."); };
    check(closed(row, FIELDS) && person(row.id) && text(row.outcome, 8192) && text(row.waiting, 2048) && text(row.obligation, 256));
    check(["handed", "done", "failed", "cancelled", "refused", "decided", "declined", "timeout", "escalated", "transfer_requested", "transfer_accepted"].includes(row.state) && (row.assignee === "" || person(row.assignee)));
    check(typeof row.acceptance_bound === "boolean" && typeof row.evidence_required === "boolean" && typeof row.reassignment_supported === "boolean" && text(row.acceptance_digest, 256) && text(row.evidence_ref, 256) && sha(row.assignment_digest));
    check(row.acceptance_bound || row.acceptance_digest === "");
    check(Array.isArray(row.history) && row.history.length > 0 && row.history.length <= 128);
    const ids = new Set(); let sequence = -1n, assignee = "";
    for (const [index, entry] of row.history.entries()) {
      check(closed(entry, HISTORY) && eventId(entry.event_id) && !ids.has(entry.event_id) && decimal(entry.sequence) && BigInt(entry.sequence) > sequence);
      check(["task.handed", "task.reassigned"].includes(entry.kind) && (entry.from === "" || person(entry.from)) && (entry.to === "" || person(entry.to)) && (entry.by === "" || person(entry.by)));
      check(entry.from === assignee && (index !== 0 || entry.kind === "task.handed") && (entry.kind !== "task.reassigned" || person(entry.to) && person(entry.by)));
      ids.add(entry.event_id); sequence = BigInt(entry.sequence); assignee = entry.to;
    }
    check(assignee === row.assignee && (!row.reassignment_supported || row.state === "handed"));
    return { ...row, history: row.history.map(entry => ({ ...entry })) };
  }
  const el = (tag, className = "", value) => { const node = document.createElement(tag); node.className = className; if (value !== undefined) node.textContent = value; return node; };
  const append = (node, ...children) => { node.append(...children); return node; };
  const button = (label, click, className = "button secondary") => { const node = el("button", className, label); node.type = "button"; node.addEventListener("click", click); return node; };
  const fact = (list, label, value, className = "") => list.append(append(el("div"), el("dt", "", label), el("dd", className, value)));
  function render(input, { onPrepareReassignment, onRefresh, canReassign = false, historical = false, inspectedAt, recipients = [] } = {}) {
    const row = validate(input);
    const supplied = Array.isArray(recipients) && recipients.length <= 64 && recipients.every(person) && new Set(recipients).size === recipients.length;
    const people = supplied ? recipients.filter(value => value !== row.assignee) : [];
    const allowed = canReassign === true && row.reassignment_supported && historical === false && row.state === "handed" && people.length > 0 && typeof onPrepareReassignment === "function";
    const root = el("section", "task-administration"); root.setAttribute("role", "region"); root.setAttribute("aria-label", "Handed Task administration"); root.dataset.state = row.state;
    const header = append(el("header", "task-administration-heading"), el("p", "eyebrow accent", "WORK / HUMAN RESPONSIBILITY"), el("h2", "", row.outcome || "Handed Task"));
    const assignment = append(el("div", "task-current-assignment"), el("span", "eyebrow", "Recorded assignee"), el("strong", "", row.assignee || "Unassigned"), el("span", "task-assignment-state", row.state === "handed" ? "Handed · completion not recorded" : "Recorded Task state · " + row.state));
    header.append(assignment);
    if (historical) header.append(el("p", "task-administration-notice", "Historical snapshot · reassignment is unavailable. Refresh to inspect the current Task."));
    if (row.waiting) header.append(append(el("div", "task-administration-waiting"), el("h3", "", "Why this Task is waiting"), el("p", "task-literal", row.waiting)));
    root.append(header);

    const trail = el("ol", "task-assignment-trail"); trail.setAttribute("aria-label", "Assignment trail");
    const selected = el("section", "task-assignment-inspector"); selected.setAttribute("role", "group"); selected.setAttribute("aria-label", "Selected assignment"); selected.setAttribute("aria-live", "polite");
    const choose = index => {
      const entry = row.history[index];
      for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(Number(control.dataset.index) === index));
      const heading = el("h3", "", entry.kind === "task.handed" ? entry.to ? "Handed to " + entry.to : "Handed · no assignee recorded" : (entry.from || "Unassigned") + " → " + entry.to);
      const facts = el("dl", "task-assignment-facts");
      fact(facts, "Recorded by", entry.by || "Not supplied"); fact(facts, "Recorded sequence", entry.sequence);
      const identity = append(el("details", "task-assignment-evidence"), el("summary", "", "Exact assignment event"), el("code", "", entry.event_id));
      selected.replaceChildren(el("p", "eyebrow", index === row.history.length - 1 ? "CURRENT ASSIGNMENT" : "EARLIER ASSIGNMENT"), heading, facts, el("p", "detail-note", entry.kind === "task.handed" ? entry.to ? "The Task was handed to this person. Handoff is not completion of their responsibility." : "The Task was recorded as handed without an assignee. Completion of its responsibility is not established." : "Reassignment changes who holds the same Task. Its obligation and bound acceptance requirements are retained."), identity);
    };
    for (const [index, entry] of row.history.entries()) {
      const control = button("", () => choose(index), "task-assignment-stop"); control.dataset.index = String(index);
      control.setAttribute("aria-label", (entry.kind === "task.handed" ? "Handoff to " + (entry.to || "unassigned") : "Reassignment from " + (entry.from || "unassigned") + " to " + entry.to) + " · sequence " + entry.sequence);
      control.append(el("span", "task-assignment-marker", String(index + 1).padStart(2, "0")), append(el("span"), el("strong", "", entry.to || "Unassigned"), el("small", "", entry.kind === "task.handed" ? index === 0 ? "Initial handoff" : "Handoff" : "From " + (entry.from || "unassigned"))));
      trail.append(append(el("li"), control));
    }
    choose(row.history.length - 1);
    const journey = append(el("div", "task-assignment-journey"), trail, selected);
    root.append(journey);

    const requirements = append(el("section", "task-acceptance"), el("h3", "", "What this Task must satisfy"));
    const facts = el("dl", "task-assignment-facts");
    fact(facts, "Obligation", row.obligation || "Not supplied");
    fact(facts, "Acceptance at handoff", row.acceptance_bound ? row.acceptance_digest ? "Exact practice bound" : "No practice was in force" : "Not recorded by this handoff");
    fact(facts, "Evidence requirement", row.evidence_required ? "Required" : "No evidence requirement recorded");
    if (row.acceptance_digest) fact(facts, "Bound acceptance digest", row.acceptance_digest, "mono");
    fact(facts, "Linked evidence", row.evidence_ref || "No evidence linked", row.evidence_ref ? "mono" : "");
    requirements.append(facts, el("p", "detail-note", "These requirements belong to this Task's recorded handoff. Reassignment does not replace them with today's practice or settle the Task."));
    root.append(requirements);

    const form = el("form", "task-reassignment-form"); form.setAttribute("aria-label", "Prepare Task reassignment");
    form.append(el("h3", "", "Reassign this Task"), el("p", "detail-note", "Choose from the people supplied by this session's native Task capability. The service checks the current assignment and recipient again when you submit."));
    const select = el("select"); select.setAttribute("aria-label", "New assignee"); select.required = true;
    const placeholder = el("option", "", "Choose a person"); placeholder.value = ""; select.append(placeholder);
    for (const value of people) { const option = el("option", "", value); option.value = value; select.append(option); }
    select.disabled = !allowed;
    const label = append(el("label", "task-recipient-field"), el("span", "", "New assignee"), select);
    const intent = el("p", "task-reassignment-intent"); intent.hidden = true;
    const submit = el("button", "button primary", "Review reassignment"); submit.type = "submit"; submit.disabled = true;
    const status = el("p", "task-reassignment-status"); status.setAttribute("role", "status");
    if (!allowed) status.textContent = historical ? "This snapshot is historical; refresh before preparing a reassignment." : row.state !== "handed" ? "Only a currently handed Task can be reassigned here." : !row.reassignment_supported ? "The native reader does not support reassignment for this Task." : !canReassign ? "Reassignment is not available to this session." : "No eligible replacement person is available from the native capability.";
    let pending = false, prepared = false;
    function selection() {
      const valid = allowed && people.includes(select.value) && !pending && !prepared;
      submit.disabled = !valid; intent.hidden = !valid;
      intent.textContent = valid ? (row.assignee || "Unassigned") + " → " + select.value + " · same Task, same requirements" : "";
    }
    select.addEventListener("change", () => { if (!pending && !prepared && allowed) { status.textContent = ""; selection(); } });
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (!allowed || pending || prepared || !people.includes(select.value)) return;
      const to = select.value;
      pending = true; select.disabled = true; submit.disabled = true; status.textContent = "Preparing the exact reassignment for confirmation…";
      try {
        const result = await onPrepareReassignment({ to });
        if (result?.error) throw new Error(typeof result.error === "string" ? result.error : "The reassignment could not be prepared.");
        prepared = true; status.textContent = "Reassignment prepared for confirmation. The recorded assignee has not changed.";
      } catch (error) {
        status.textContent = typeof error?.message === "string" ? error.message : "The reassignment could not be prepared. Nothing was submitted by this editor.";
      } finally { pending = false; select.disabled = !allowed || prepared; selection(); }
    });
    form.append(label, intent, submit, status); root.append(form);
    const footer = el("div", "task-administration-footer");
    if (typeof onRefresh === "function") footer.append(button("Refresh Task", onRefresh));
    const stamp = inspectedAt instanceof Date && Number.isFinite(inspectedAt.getTime()) ? inspectedAt.toLocaleTimeString() : "this read";
    footer.append(el("p", "detail-note", "Inspected at " + stamp + ". Assignment history records who held this Task; it does not change ownership scopes, remove positions or retire a person."));
    const identity = append(el("details", "task-assignment-evidence"), el("summary", "", "Task and assignment identity"), el("code", "", row.id), el("code", "", row.assignment_digest));
    root.append(footer, identity);
    return root;
  }
  window.IrisTaskAdministration = { validate, render };
})();
