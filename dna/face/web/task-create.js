/* Raising work: the "New task" form, the face's `hale dna ask`.
 * This module performs no requests, writes or persistence. */
"use strict";
(() => {
  const encoder = new TextEncoder();
  const MAX_OUTCOME_BYTES = 8192, MAX_POSITION_BYTES = 256;
  const ORGANIZATION = "org";
  const text = value => typeof value === "string" && !value.includes("\u0000") && !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(value);
  const bytes = value => encoder.encode(value).length;
  const position = value => text(value) && value.length > 0 && !/[\u0000-\u001f\u007f]/.test(value) && bytes(value) <= MAX_POSITION_BYTES;
  // The ask as the service admits it: an outcome for a position. Nothing
  // else is caller-supplied; the intent id, the asker and the Task are the
  // service's and the organism's.
  function validate(input) {
    const check = condition => { if (!condition) throw new Error("The new task could not be verified: say what should happen, for one declared locus."); };
    check(input && typeof input === "object" && !Array.isArray(input) && Object.keys(input).length === 2 && Object.hasOwn(input, "outcome") && Object.hasOwn(input, "to"));
    check(text(input.outcome) && bytes(input.outcome) > 0 && bytes(input.outcome) <= MAX_OUTCOME_BYTES && position(input.to));
    return { outcome: input.outcome, to: input.to };
  }
  const el = (tag, className = "", value) => { const node = document.createElement(tag); node.className = className; if (value !== undefined) node.textContent = value; return node; };
  const append = (node, ...children) => { node.append(...children); return node; };
  // `positions` are the working-context loci ({ position, owner }); the whole
  // organization is always offered first, as `hale dna ask` without `--to`.
  function render({ onPrepare, canCreate = false, positions = [], defaultTo = "", reason = "" } = {}) {
    const supplied = Array.isArray(positions) && positions.every(row => row && position(row.position) && typeof row.owner === "string") && new Set(positions.map(row => row.position)).size === positions.length;
    const loci = supplied ? positions.filter(row => row.position !== ORGANIZATION) : [];
    // The choices are the supplied loci, not whatever the DOM holds.
    const choices = [ORGANIZATION, ...loci.map(row => row.position)];
    const allowed = canCreate === true && typeof onPrepare === "function";
    const root = el("section", "task-create"); root.setAttribute("role", "region"); root.setAttribute("aria-label", "New task"); root.dataset.state = allowed ? "ready" : "unavailable";
    root.append(el("p", "eyebrow accent", "WORK / RAISE"), el("h2", "", "New task"), el("p", "detail-note", "Say what should happen. The organism decides whether to admit it and mints the Task; this form records the ask in your name."));
    const form = el("form", "task-create-form"); form.setAttribute("aria-label", "Prepare a new task");
    const outcome = el("textarea"); outcome.setAttribute("aria-label", "What should happen"); outcome.required = true; outcome.rows = 4; outcome.maxLength = MAX_OUTCOME_BYTES; outcome.placeholder = "What should happen";
    const outcomeField = append(el("label", "task-create-field"), el("span", "", "What should happen"), outcome);
    const counter = el("p", "task-create-counter"); counter.setAttribute("aria-live", "polite");
    const select = el("select"); select.setAttribute("aria-label", "For locus"); select.required = true;
    const whole = el("option", "", "Whole organization · " + ORGANIZATION); whole.value = ORGANIZATION; select.append(whole);
    for (const row of loci) { const option = el("option", "", row.position + (row.owner ? " · " + row.owner : "")); option.value = row.position; select.append(option); }
    if (position(defaultTo) && choices.includes(defaultTo)) select.value = defaultTo;
    const toField = append(el("label", "task-create-field"), el("span", "", "For locus"), select);
    const submit = el("button", "button primary", "Review new task"); submit.type = "submit"; submit.disabled = true;
    const status = el("p", "task-create-status"); status.setAttribute("role", "status");
    if (!allowed) status.textContent = reason || "Raising work is not available to this session.";
    outcome.disabled = !allowed; select.disabled = !allowed;
    let pending = false, prepared = false;
    function refresh() {
      const size = bytes(outcome.value);
      counter.textContent = size + " / " + MAX_OUTCOME_BYTES + " bytes";
      const valid = allowed && !pending && !prepared && text(outcome.value) && size > 0 && size <= MAX_OUTCOME_BYTES && choices.includes(select.value);
      outcome.setAttribute("aria-invalid", String(size > MAX_OUTCOME_BYTES));
      submit.disabled = !valid;
    }
    outcome.addEventListener("input", () => { if (!pending && !prepared && allowed) { status.textContent = ""; refresh(); } });
    select.addEventListener("change", () => { if (!pending && !prepared && allowed) { status.textContent = ""; refresh(); } });
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (!allowed || pending || prepared) return;
      let ask;
      try { ask = validate({ outcome: outcome.value, to: select.value }); } catch (error) { status.textContent = error.message; return; }
      if (!choices.includes(ask.to)) { status.textContent = "Choose a declared locus."; return; }
      pending = true; refresh(); outcome.disabled = true; select.disabled = true; status.textContent = "Preparing the exact ask for confirmation…";
      try {
        const result = await onPrepare(ask);
        if (result?.error) throw new Error(typeof result.error === "string" ? result.error : "The new task could not be prepared.");
        prepared = true; status.textContent = "New task prepared for confirmation. Nothing has been recorded yet.";
      } catch (error) {
        status.textContent = typeof error?.message === "string" ? error.message : "The new task could not be prepared. Nothing was submitted by this form.";
      } finally { pending = false; outcome.disabled = !allowed || prepared; select.disabled = !allowed || prepared; refresh(); }
    });
    refresh();
    form.append(outcomeField, counter, toField, submit, status); root.append(form);
    root.append(el("p", "detail-note", "The service checks your signed-in identity and the current Record head when you confirm. Whether the locus is this organization's to admit is the organism's answer, recorded separately."));
    return root;
  }
  window.IrisTaskCreate = { validate, render };
})();
