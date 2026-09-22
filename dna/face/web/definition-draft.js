/* Exact workflow revisions: inspect, draft, validate, export. Never publish. */
"use strict";
(() => {
  const PROFILE = "dna.definition.draft.v1";
  const REQUEST_LIMIT = 32768, TEXT_LIMIT = 8192, RESPONSE_LIMIT = 2097152, DEADLINE = 15000;
  const MAX_INT = 9223372036854775807n;
  const encoder = new TextEncoder();
  const BASIS_FIELDS = ["catalog_digest", "format", "provenance_kind", "source_revision", "source_path", "dependency_digest"];
  const LIMITS = ["max_depth", "max_steps", "max_members", "max_attempts", "max_works"];
  const LEAF_FIELDS = [["objective", "Objective"], ["context_digest", "Context digest"], ["knowledge_bindings", "Knowledge bindings"], ["output_contract", "Output contract"], ["data_class", "Data class"], ["requires", "Requires"], ["target", "Target"], ["cost_ceiling", "Cost ceiling"], ["attempts", "Attempts"]];
  const clone = value => JSON.parse(JSON.stringify(value));
  const closed = (value, keys) => value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
  const bytes = value => encoder.encode(value).length;
  const digest = value => typeof value === "string" && /^sha256:[0-9a-f]{64}$/.test(value);
  const identifier = value => typeof value === "string" && /^[a-z0-9][a-z0-9-]*$/.test(value);
  const memberKey = value => typeof value === "string" && /^[a-z0-9-]+$/.test(value);
  function text(value, bound = RESPONSE_LIMIT) {
    if (typeof value !== "string" || value.includes("\u0000") || bytes(value) > bound) return false;
    for (let i = 0; i < value.length; i += 1) {
      const code = value.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff) {
        const next = value.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      } else if (code >= 0xdc00 && code <= 0xdfff) return false;
    }
    return true;
  }
  function integer(value, signed = false) {
    if (typeof value !== "string" || !(signed ? /^(0|-?[1-9][0-9]*)$/ : /^(0|[1-9][0-9]*)$/).test(value)) return false;
    const digits = value.startsWith("-") ? value.slice(1) : value;
    const ceiling = value.startsWith("-") ? "9223372036854775808" : "9223372036854775807";
    return digits.length < ceiling.length || (digits.length === ceiling.length && digits <= ceiling);
  }
  const samePrincipal = (a, b) => closed(a, ["mode", "name"]) && ["local", "oidc"].includes(a.mode) && text(a.name, 4096) && a.mode === b.mode && a.name === b.name;
  function sameBasis(a, b) {
    return closed(a, [...BASIS_FIELDS, "limits"]) && BASIS_FIELDS.every(key => typeof a[key] === "string" && a[key] === b[key]) && closed(a.limits, LIMITS) && LIMITS.every(key => integer(a.limits[key]) && a.limits[key] === b.limits[key]);
  }
  function capable(value) {
    return closed(value, ["profile", "supported", "validation", "publication", "max_request_bytes", "max_base_bytes", "max_candidate_bytes", "max_text_field_bytes", "max_catalog_text_bytes"]) && value.profile === PROFILE && value.supported === true && value.validation === true && value.publication === false && value.max_request_bytes === "32768" && value.max_base_bytes === "65536" && value.max_candidate_bytes === "98304" && value.max_text_field_bytes === "8192" && value.max_catalog_text_bytes === "16384";
  }
  async function sha256(value) {
    const result = await crypto.subtle.digest("SHA-256", encoder.encode(value));
    return "sha256:" + Array.from(new Uint8Array(result), byte => byte.toString(16).padStart(2, "0")).join("");
  }
  const el = (tag, className = "", value) => {
    const result = document.createElement(tag);
    if (className) result.className = className;
    if (value !== undefined) result.textContent = String(value);
    return result;
  };
  const append = (parent, ...children) => { children.filter(Boolean).forEach(child => parent.append(child)); return parent; };
  function button(label, action, className = "button secondary") {
    const result = el("button", className, label);
    result.type = "button";
    result.addEventListener("click", action);
    return result;
  }
  function facts(entries) {
    const list = el("dl", "dd-facts");
    entries.forEach(([label, value]) => list.append(append(el("div"), el("dt", "", label), el("dd", "", value))));
    return list;
  }
  function mount(container, { item, basis, applicationId, principal, capability, onInvalidate, onNavigateChild } = {}) {
    const base = clone(item), capturedBasis = clone(basis), actor = clone(principal);
    let disposed = false, invalidated = false, generation = 0, controller = null, pending = false;
    let draft = null, result = null, selection = { step: 0, member: 0 }, representation = "structured", message = "", transportError = "";
    const urls = new Set();
    const available = capable(capability);
    const root = el("section", "definition-workspace");
    root.setAttribute("role", "region");
    root.setAttribute("aria-label", "Definition workspace");
    const header = el("header", "dd-header");
    const presentation = el("div", "dd-presentation");
    const layout = el("div", "dd-layout");
    const canvas = el("section", "dd-canvas");
    canvas.setAttribute("role", "region");
    canvas.setAttribute("aria-label", "Definition canvas");
    const inspector = el("aside", "dd-inspector");
    inspector.setAttribute("aria-label", "Member inspector");
    inspector.tabIndex = -1;
    const jsonPanel = el("section", "dd-json-panel");
    jsonPanel.setAttribute("aria-label", "Exact definition JSON");
    const validation = el("section", "dd-validation");
    validation.setAttribute("role", "region");
    validation.setAttribute("aria-label", "Definition validation");
    append(layout, canvas, inspector);
    append(root, header, presentation, layout, jsonPanel, validation);
    container.replaceChildren(root);
    const current = () => draft || base;
    const isCurrent = token => !disposed && !invalidated && token === generation;
    const candidate = () => ({ id: draft.id, definition_id: draft.definition_id, revision: draft.revision, title: draft.title, steps: draft.steps });
    const requestBody = value => ({ profile: PROFILE, application_id: applicationId, principal: actor, base: { id: base.id, basis: capturedBasis }, candidate: value });
    function stop() {
      generation += 1;
      controller?.abort();
      controller = null;
      pending = false;
      for (const url of urls) URL.revokeObjectURL(url);
      urls.clear();
    }
    function clear(reason) {
      stop();
      invalidated = true;
      draft = result = null;
      root.replaceChildren(append(el("div", "dd-invalidated"), el("h2", "", "Definition context changed"), el("p", "", reason.message)));
      onInvalidate?.(reason);
    }
    function invalidateResult() {
      const hadResult = Boolean(result);
      stop();
      result = null;
      transportError = "";
      message = hadResult ? "The candidate changed. The previous native result and generated source were cleared." : "";
    }
    function localChecks() {
      if (!draft) return [];
      const errors = [], add = value => { if (errors.length < 12) errors.push(value); };
      if (draft.definition_id !== base.definition_id || draft.revision !== (BigInt(base.revision) + 1n).toString() || draft.id !== draft.definition_id + "@" + draft.revision || !integer(draft.revision, true)) add("The candidate must retain this definition identity and use the next exact Int64 revision.");
      if (!text(draft.title, TEXT_LIMIT)) add("Title must be valid Unicode without NUL and no more than 8192 UTF-8 bytes.");
      if (![draft.id, draft.definition_id].every(value => text(value, TEXT_LIMIT))) add("Definition identities must be no more than 8192 UTF-8 bytes.");
      if (!draft.steps.length) add("A workflow requires at least one Step.");
      if (BigInt(draft.steps.length) > BigInt(capturedBasis.limits.max_steps)) add("The draft exceeds the captured Steps-per-workflow limit.");
      let members = 0;
      draft.steps.forEach((step, stepIndex) => {
        const name = "Step " + (stepIndex + 1);
        if (step.index !== String(stepIndex)) add(name + " has an inconsistent index.");
        if (!step.members.length) add(name + " requires at least one member.");
        if (BigInt(step.members.length) > BigInt(capturedBasis.limits.max_members)) add(name + " exceeds the captured member limit.");
        const keys = new Set();
        for (const member of step.members) {
          members += 1;
          if (!memberKey(member.key) || bytes(member.key) > 256 || keys.has(member.key)) add(name + " needs distinct member keys using lowercase letters, digits or hyphens, at most 256 UTF-8 bytes.");
          keys.add(member.key);
          if (member.kind === "leaf") {
            for (const [key, label] of LEAF_FIELDS.slice(0, 7)) if (!text(member.leaf[key], TEXT_LIMIT)) add(name + " · " + label + " must be valid Unicode without NUL and no more than 8192 UTF-8 bytes.");
            if (!integer(member.leaf.cost_ceiling, true)) add(name + " cost ceiling must be an exact signed Int64 decimal.");
            if (!integer(member.leaf.attempts) || member.leaf.attempts === "0" || BigInt(member.leaf.attempts) > BigInt(capturedBasis.limits.max_attempts)) add(name + " attempts must be within 1 and the captured maximum.");
          } else if (member.kind === "child") {
            if (!identifier(member.child.definition_id) || bytes(member.child.definition_id) > 256 || !integer(member.child.revision, true) || member.child.id !== member.child.definition_id + "@" + member.child.revision) add(name + " requires an exact child definition ID and signed Int64 revision.");
          } else add(name + " has an unsupported member kind.");
        }
      });
      if (members > 1024) add("The draft exceeds the native reader's member budget.");
      if (bytes(JSON.stringify(requestBody(candidate()))) > REQUEST_LIMIT) add("The complete validation request exceeds 32768 UTF-8 bytes. Reduce the candidate before validating.");
      return errors;
    }
    function edit(change, structural = false) {
      if (!draft || invalidated || disposed) return;
      change();
      draft.steps.forEach((step, index) => { step.index = String(index); });
      invalidateResult();
      renderCanvas();
      renderValidation();
      if (structural) { renderInspector(); renderHeader(); }
      renderRepresentation();
    }
    function focusInspector() {
      inspector.focus({ preventScroll: true });
      if (window.matchMedia("(max-width: 760px)").matches) inspector.scrollIntoView({ block: "start", behavior: "auto" });
    }
    function start() {
      if (draft || invalidated || !integer(base.revision, true) || BigInt(base.revision) === MAX_INT) return;
      stop();
      const revision = (BigInt(base.revision) + 1n).toString();
      draft = { id: base.definition_id + "@" + revision, definition_id: base.definition_id, revision, title: base.title, steps: clone(base.steps) };
      selection = { step: 0, member: 0 };
      result = null; message = transportError = ""; representation = "structured";
      render();
      header.querySelector("input")?.focus();
    }
    function discard() {
      stop(); draft = result = null; message = transportError = ""; representation = "structured";
      selection = { step: 0, member: 0 };
      render();
      header.querySelector("button")?.focus();
    }
    function addStep() {
      if (!draft || draft.steps.length >= Math.min(Number(capturedBasis.limits.max_steps), 256)) return;
      edit(() => { draft.steps.push({ index: String(draft.steps.length), members: [] }); selection = { step: draft.steps.length - 1, member: -1 }; }, true);
      canvas.querySelector('[data-step="' + selection.step + '"]')?.focus();
    }
    function addMember(stepIndex, kind) {
      const step = draft?.steps[stepIndex];
      if (!step || step.members.length >= Math.min(Number(capturedBasis.limits.max_members), 512)) return;
      let index = 1;
      while (step.members.some(member => member.key === kind + "-" + index)) index += 1;
      const member = kind === "leaf" ? { key: "leaf-" + index, kind, leaf: { objective: "", context_digest: "", knowledge_bindings: "", output_contract: "", data_class: "internal", requires: "", target: "", cost_ceiling: "0", attempts: "1" }, child: null } : { key: "child-" + index, kind, leaf: null, child: { id: "@1", definition_id: "", revision: "1" } };
      edit(() => { step.members.push(member); selection = { step: stepIndex, member: step.members.length - 1 }; }, true);
      focusInspector();
      inspector.querySelector("input")?.focus({ preventScroll: true });
    }
    function moveStep(index, delta) {
      if (!draft || index + delta < 0 || index + delta >= draft.steps.length) return;
      edit(() => {
        const step = draft.steps.splice(index, 1)[0]; draft.steps.splice(index + delta, 0, step);
        if (selection.step === index) selection.step += delta;
        else if (selection.step === index + delta) selection.step = index;
      }, true);
      canvas.querySelector('[data-step="' + (index + delta) + '"]')?.focus();
    }
    function removeStep(index) {
      edit(() => { draft.steps.splice(index, 1); selection = { step: Math.min(index, draft.steps.length - 1), member: 0 }; }, true);
      canvas.querySelector('[data-step="' + selection.step + '"]')?.focus();
    }
    function removeMember() {
      edit(() => { const step = draft.steps[selection.step]; step.members.splice(selection.member, 1); selection.member = Math.min(selection.member, step.members.length - 1); }, true);
      focusInspector();
    }
    function renderHeader() {
      header.replaceChildren();
      const identity = append(el("div", "dd-identity"), el("p", "eyebrow", draft ? "LOCAL CANDIDATE / NOT PUBLISHED" : "CODE-AUTHORED WORKFLOW"), el("h2", "", current().title || current().definition_id), el("p", "mono dd-exact-id", current().id));
      const controls = el("div", "dd-actions");
      if (draft) controls.append(button("Discard draft", discard));
      else { const startButton = button("Draft next revision", start, "button"); startButton.disabled = BigInt(base.revision) === MAX_INT; controls.append(startButton); }
      header.append(identity, controls);
      if (draft) {
        const label = el("label", "dd-title-field", "Definition title");
        const input = el("input"); input.type = "text"; input.value = draft.title; input.maxLength = REQUEST_LIMIT;
        input.addEventListener("input", () => { edit(() => { draft.title = input.value; }); identity.querySelector("h2").textContent = draft.title || draft.definition_id; });
        label.append(input); header.append(label);
        header.append(el("p", "dd-boundary", "Drafting " + draft.id + " from " + base.id + ". The loaded revision and its callers remain unchanged."));
      } else if (BigInt(base.revision) === MAX_INT) header.append(el("p", "dd-warning", "This revision is already the Int64 maximum; no N+1 draft can be represented."));
    }
    function renderRepresentation() {
      presentation.replaceChildren();
      presentation.setAttribute("role", "group"); presentation.setAttribute("aria-label", "Definition representation");
      for (const [value, label] of [["structured", "Structured"], ["json", "Draft JSON"]]) {
        const control = button(label, () => { representation = value; renderRepresentation(); presentation.querySelector('[aria-pressed="true"]')?.focus(); }, "scope-button");
        control.setAttribute("aria-pressed", String(representation === value)); presentation.append(control);
      }
      presentation.append(el("p", "dd-boundary", "Every member in a Step must complete before the next Step begins."));
      layout.hidden = representation !== "structured"; jsonPanel.hidden = representation !== "json";
      jsonPanel.replaceChildren();
      if (representation === "json") {
        const value = draft ? candidate() : { id: base.id, definition_id: base.definition_id, revision: base.revision, title: base.title, steps: base.steps };
        append(jsonPanel, el("p", "dd-boundary", (draft ? "Exact local candidate data." : "Exact selected revision data; no draft exists.") + " This read-only JSON representation is not the original Hale source module."), el("pre", "dd-code", JSON.stringify(value, null, 2)));
      }
    }
    function renderCanvas() {
      canvas.replaceChildren();
      const value = current();
      canvas.append(append(el("div", "dd-canvas-heading"), el("span", "eyebrow", draft ? "CANDIDATE STRUCTURE" : "DECLARED RECIPE"), el("span", "mono", value.steps.length + " ordered Steps")));
      const steps = el("ol", "dd-step-list");
      steps.setAttribute("aria-label", "Ordered Steps");
      value.steps.forEach((step, stepIndex) => {
        const band = el("li", "dd-step-band"); band.dataset.step = String(stepIndex); band.tabIndex = -1;
        const heading = append(el("div", "dd-step-heading"), el("h4", "", "Step " + (stepIndex + 1)), el("span", "", step.members.length + " required members"));
        band.append(heading);
        if (draft) {
          const actions = el("div", "dd-step-actions");
          for (const [delta, word] of [[-1, "earlier"], [1, "later"]]) {
            const control = button("Move " + word, () => moveStep(stepIndex, delta), "dd-small-control");
            control.setAttribute("aria-label", "Move Step " + (stepIndex + 1) + " " + word);
            control.disabled = stepIndex + delta < 0 || stepIndex + delta >= value.steps.length; actions.append(control);
          }
          const remove = button("Remove Step", () => removeStep(stepIndex), "dd-small-control dd-destructive"); remove.setAttribute("aria-label", "Remove Step " + (stepIndex + 1)); actions.append(remove); band.append(actions);
        }
        const members = el("ul", "dd-member-nodes");
        step.members.forEach((member, memberIndex) => {
          const control = button("", () => { selection = { step: stepIndex, member: memberIndex }; renderCanvas(); renderInspector(); focusInspector(); }, "dd-member-node");
          control.setAttribute("aria-label", "Inspect member " + (member.key || "unnamed") + " in Step " + (stepIndex + 1));
          control.setAttribute("aria-pressed", String(selection.step === stepIndex && selection.member === memberIndex));
          append(control, el("span", "eyebrow", member.kind === "leaf" ? "LEAF WORK" : "CHILD DEFINITION"), el("strong", "", member.key || "Unnamed member"), el("span", "dd-node-summary", member.kind === "leaf" ? member.leaf.objective.slice(0, 140) || "Objective not supplied" : member.child.id));
          members.append(append(el("li"), control));
        });
        band.append(members);
        if (!step.members.length) band.append(el("p", "dd-empty", "This Step has no required members. Add a leaf or an exact child reference."));
        if (draft) {
          const actions = el("div", "dd-add-members");
          for (const kind of ["leaf", "child"]) {
            const control = button("Add " + kind, () => addMember(stepIndex, kind), "dd-small-control"); control.setAttribute("aria-label", "Add " + kind + " to Step " + (stepIndex + 1));
            control.disabled = step.members.length >= Math.min(Number(capturedBasis.limits.max_members), 512); actions.append(control);
          }
          band.append(actions);
        }
        steps.append(band);
      });
      canvas.append(steps);
      if (!value.steps.length) canvas.append(el("p", "dd-empty", "No Steps. A valid workflow needs at least one nonempty Step."));
      if (draft) { const add = button("Add Step", addStep); add.disabled = value.steps.length >= Math.min(Number(capturedBasis.limits.max_steps), 256); canvas.append(append(el("div", "dd-add-step"), add)); }
    }
    function field(labelText, value, change, multiline = false) {
      const label = el("label", "dd-field", labelText);
      const input = el(multiline ? "textarea" : "input");
      if (!multiline) input.type = "text";
      else input.rows = labelText === "Objective" || labelText === "Output contract" ? 5 : 3;
      input.value = value; input.maxLength = REQUEST_LIMIT;
      input.spellcheck = ["Objective", "Output contract"].includes(labelText);
      input.addEventListener("input", () => edit(() => change(input.value)));
      label.append(input); return label;
    }
    function renderInspector() {
      inspector.replaceChildren();
      const member = current().steps[selection.step]?.members[selection.member];
      append(inspector, el("p", "eyebrow", draft ? "EDIT REQUIRED MEMBER" : "INSPECT REQUIRED MEMBER"), el("h3", "", member ? member.key || "Unnamed member" : "Member inspector"));
      if (!member) { inspector.append(el("p", "dd-boundary", "Choose a member on the canvas to inspect its exact specification.")); return; }
      inspector.append(el("p", "dd-inspector-location", "Step " + (selection.step + 1) + " · " + (member.kind === "leaf" ? "Leaf Work" : "Child definition")));
      if (draft) inspector.append(field("Member key", member.key, value => { member.key = value; inspector.querySelector("h3").textContent = value || "Unnamed member"; }));
      if (member.kind === "leaf") {
        if (draft) for (const [key, label] of LEAF_FIELDS) inspector.append(field(label, member.leaf[key], value => { member.leaf[key] = value; }, ["objective", "context_digest", "knowledge_bindings", "output_contract"].includes(key)));
        else inspector.append(facts(LEAF_FIELDS.map(([key, label]) => [label, member.leaf[key] || "Not supplied"])));
      } else {
        if (draft) {
          inspector.append(field("Child definition ID", member.child.definition_id, value => { member.child.definition_id = value; member.child.id = value + "@" + member.child.revision; }));
          inspector.append(field("Child revision", member.child.revision, value => { member.child.revision = value; member.child.id = member.child.definition_id + "@" + value; }));
          inspector.append(el("p", "dd-boundary", "Native validation resolves this exact revision in the captured catalog. Discard the draft before navigating to a child; draft text is not persisted."));
        } else {
          inspector.append(facts([["Child definition", member.child.definition_id], ["Exact revision", member.child.revision]]));
          if (onNavigateChild) inspector.append(button("Open child " + member.child.id, () => onNavigateChild(member.child.id, { step: current().steps[selection.step].index, key: member.key })));
        }
      }
      if (draft) inspector.append(button("Remove member", removeMember, "button secondary dd-remove-member"));
    }
    function renderValidation() {
      const retainFocus = validation.contains(document.activeElement);
      const wasOpen = Boolean(validation.querySelector("details")?.open);
      validation.replaceChildren();
      validation.hidden = !draft;
      if (!draft) return;
      const errors = localChecks();
      const status = el("p", "dd-validation-status", pending ? "Native validation in progress" : result ? result.validation.valid ? "Native catalog validation passed" : "Native catalog validation refused" : errors.length ? "Local checks need attention" : "Local checks passed · native validation not established");
      status.setAttribute("role", "status");
      status.tabIndex = -1;
      validation.append(status);
      const controls = el("div", "dd-actions");
      const validate = button("Validate draft", () => validateDraft(), "button");
      validate.disabled = pending || !available || errors.length > 0 || !crypto.subtle?.digest;
      controls.append(validate); validation.append(controls);
      if (!available) validation.append(el("p", "dd-boundary", "Native validation is unavailable on this connection. The in-memory draft does not change the loaded catalog."));
      if (available) validation.append(el("p", "dd-boundary", "Each text field is limited to 8192 UTF-8 bytes. Native validation also checks a 16384-byte text budget for the full captured candidate catalog, including base definitions not loaded on this page."));
      if (!crypto.subtle?.digest) validation.append(el("p", "dd-warning", "A secure browser context with SHA-256 support is required to bind validation and generated source."));
      if (message) validation.append(el("p", "dd-boundary", message));
      if (transportError) validation.append(el("p", "dd-warning", transportError));
      if (errors.length) { const list = el("ul", "dd-checks"); errors.forEach(error => list.append(el("li", "", error))); validation.append(list); }
      if (result) {
        const native = result.validation;
        if (!native.valid) validation.append(el("p", "dd-warning", native.code + ": " + native.message));
        else {
          const download = button("Download generated Hale", downloadArtifact);
          controls.append(download);
          validation.append(el("p", "dd-boundary", "The native catalog checks passed under the captured limits. This does not establish leaf feasibility, context access, publication or execution."));
        }
        const details = el("details", "dd-validation-evidence");
        details.open = wasOpen;
        details.append(el("summary", "", native.valid ? "Generated Hale and validation evidence" : "Validation evidence"));
        details.append(facts([["Base revision", result.base_id], ["Candidate digest", result.candidate_digest], ["Candidate catalog", native.catalog_digest || "No valid candidate catalog"], ["Publication", "Unavailable — not published"]]));
        details.append(el("p", "dd-boundary", "All existing revisions are preserved. " + result.impact.direct_dependents.length + " direct catalog references remain pinned to " + base.id + ". Runtime, Knowledge and Practice impact is not established."));
        if (native.valid) {
          details.append(facts([["Generated artifact", result.artifact.name], ["Artifact digest", result.artifact.digest], ["Export scope", "Full captured candidate catalog"]]));
          details.append(el("p", "dd-boundary", "Generated registration fragment, not the original source module. It requires the containing seed's explicit dna core import and a fresh catalog. It does not overwrite the host-claimed source path."));
          const source = el("pre", "dd-code", result.artifact.text); source.tabIndex = 0; source.setAttribute("aria-label", "Generated Hale fragment"); details.append(source);
        }
        validation.append(details);
      }
      validation.append(el("p", "dd-publication", "Local draft only · publication unavailable"));
      if (retainFocus) status.focus({ preventScroll: true });
    }
    function matchesDependents(values) {
      if (!Array.isArray(values) || values.length !== base.dependents.length || values.length > 1024) return false;
      const key = value => JSON.stringify([value.id, value.definition_id, value.revision, value.step, value.key]);
      const expected = base.dependents.map(key).sort();
      return values.every(value => closed(value, ["id", "definition_id", "revision", "step", "key"]) && identifier(value.definition_id) && integer(value.revision, true) && value.id === value.definition_id + "@" + value.revision && integer(value.step) && memberKey(value.key)) && values.map(key).sort().every((value, index) => value === expected[index]);
    }
    async function checkedResult(body, expectedDigest) {
      const fail = () => { throw new Error("The service returned an unverifiable validation result. No result or generated source is retained."); };
      if (!closed(body, ["api_version", "source", "data"]) || body.api_version !== "hale.v1" || !closed(body.source, ["record_id", "record_head", "record_revision"]) || !text(body.source.record_head, 512) || !body.source.record_head || !integer(body.source.record_revision)) fail();
      const data = body.data;
      if (!closed(data, ["profile", "principal", "base_id", "basis", "candidate_digest", "validation", "impact", "artifact", "publication"]) || data.profile !== PROFILE || !digest(data.candidate_digest) || data.candidate_digest !== expectedDigest || data.publication !== "unavailable") fail();
      if (body.source.record_id !== applicationId || !samePrincipal(data.principal, actor) || data.base_id !== base.id || !sameBasis(data.basis, capturedBasis)) {
        const error = new Error("The validation response changed application, principal or captured source context. The draft and generated source were cleared.");
        error.contextChanged = true;
        throw error;
      }
      const native = data.validation;
      if (!closed(native, ["valid", "code", "message", "catalog_digest"]) || typeof native.valid !== "boolean" || !text(native.code, 256) || !text(native.message, 8192)) fail();
      if (!closed(data.impact, ["direct_dependents", "existing_revisions_preserved"]) || data.impact.existing_revisions_preserved !== true || !matchesDependents(data.impact.direct_dependents)) fail();
      if (native.valid) {
        if (native.code !== "" || native.message !== "" || !digest(native.catalog_digest)) fail();
        const artifact = data.artifact;
        if (!closed(artifact, ["profile", "name", "digest", "text", "scope"]) || artifact.profile !== "hale.workflow-registration.v1" || artifact.name !== "workflows.generated.hl" || artifact.scope !== "captured_catalog" || !text(artifact.text) || !artifact.text || !digest(artifact.digest) || await sha256(artifact.text) !== artifact.digest) fail();
      } else if (!native.code || !native.message || native.catalog_digest !== "" || data.artifact !== null) fail();
      return data;
    }
    async function validateDraft() {
      if (!draft || pending || !available || localChecks().length || !crypto.subtle?.digest || disposed || invalidated) return;
      stop();
      const token = generation, candidateFrame = JSON.stringify(candidate());
      const payload = requestBody(JSON.parse(candidateFrame));
      result = null; transportError = message = ""; pending = true; renderValidation();
      const active = new AbortController(); controller = active;
      const timeout = setTimeout(() => active.abort(), DEADLINE);
      let reader;
      try {
        const expected = await sha256(candidateFrame);
        if (!isCurrent(token) || active.signal.aborted) return;
        const response = await fetch("/api/hale/v1/applications/" + encodeURIComponent(applicationId) + "/dna/definitions/draft", { method: "POST", credentials: "same-origin", referrerPolicy: "no-referrer", redirect: "error", cache: "no-store", signal: active.signal, headers: { "Content-Type": "application/json", Accept: "application/json", "X-Hale-Command": "1" }, body: JSON.stringify(payload) });
        if (!isCurrent(token)) return;
        if ([401, 403, 409].includes(response.status)) { clear({ status: response.status, code: response.status === 401 ? "unauthenticated" : response.status === 403 ? "forbidden" : "snapshot_changed", message: "Access, identity or the captured catalog changed. The draft and generated source were cleared. Refresh the Definitions workspace before continuing." }); return; }
        const length = response.headers.get("content-length");
        if (length && /^\d+$/.test(length) && Number(length) > RESPONSE_LIMIT) throw new Error("Validation response exceeded the browser's bounded read limit.");
        if (!response.body?.getReader) throw new Error("Validation response could not be read.");
        reader = response.body.getReader();
        const decoder = new TextDecoder("utf-8", { fatal: true });
        let size = 0; const parts = [];
        while (true) {
          const part = await reader.read(); if (part.done) break;
          size += part.value.byteLength;
          if (size > RESPONSE_LIMIT) throw new Error("Validation response exceeded the browser's bounded read limit.");
          parts.push(decoder.decode(part.value, { stream: true }));
        }
        parts.push(decoder.decode());
        if (!isCurrent(token)) return;
        const body = JSON.parse(parts.join(""));
        if (response.status !== 200) {
          if (!closed(body, ["api_version", "error"]) || body.api_version !== "hale.v1" || !closed(body.error, ["code", "message", "retryable"]) || !text(body.error.code, 256) || !text(body.error.message, 8192) || typeof body.error.retryable !== "boolean") throw new Error("The service returned an unverifiable error. No validation result was retained.");
          if (response.status === 404) { clear({ status: 404, code: "definition_not_found", message: "The exact base revision is no longer available. The draft and generated source were cleared." }); return; }
          throw new Error(response.status === 503 ? "Native validation is unavailable or exceeds its declared read budget. This does not mean the candidate is semantically invalid. Retry explicitly after checking the connection or reducing the draft." : "The service could not validate this request. No native result was established; check the candidate and retry explicitly.");
        }
        const verified = await checkedResult(body, expected);
        if (active.signal.aborted) throw new DOMException("Validation deadline elapsed", "AbortError");
        if (!isCurrent(token) || candidateFrame !== JSON.stringify(candidate())) return;
        result = verified;
      } catch (error) {
        if (isCurrent(token) && error.contextChanged) { clear({ status: 409, code: "definition_context_changed", message: error.message }); return; }
        if (isCurrent(token)) transportError = error.name === "AbortError" ? "Native validation timed out. No result was retained and no request will be retried automatically." : error instanceof SyntaxError || error instanceof TypeError ? "The native validation response could not be verified. No result or generated source was retained. Retry explicitly if appropriate." : error.message;
      } finally {
        clearTimeout(timeout); active.abort();
        if (reader) { await reader.cancel().catch(() => {}); reader.releaseLock(); }
        if (isCurrent(token)) { controller = null; pending = false; renderValidation(); }
      }
    }
    function downloadArtifact() {
      if (!draft || !result?.validation.valid || pending || disposed || invalidated) return;
      const blob = new Blob([result.artifact.text], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob); urls.add(url);
      const link = el("a"); link.href = url; link.download = result.artifact.name; link.hidden = true;
      root.append(link); link.click(); link.remove();
      setTimeout(() => { URL.revokeObjectURL(url); urls.delete(url); }, 1000);
    }
    function render() { renderHeader(); renderRepresentation(); renderCanvas(); renderInspector(); renderValidation(); }
    render();
    return { destroy() { if (disposed) return; disposed = true; stop(); draft = result = null; container.replaceChildren(); } };
  }
  window.IrisDefinitionDraft = Object.freeze({ mount });
})();
