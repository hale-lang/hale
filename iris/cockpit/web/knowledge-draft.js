/* Prepare Knowledge changes against visible, immutable service snapshots.
 * Supported relationship commands are admitted by the native service. */
"use strict";
(() => {
  const encoder = new TextEncoder();
  const bytes = value => encoder.encode(value).length;
  const clone = value => JSON.parse(JSON.stringify(value));
  const actions = [
    ["node.propose", "New knowledge item"], ["node.revise", "Revise this item"],
    ["node.retire", "Retire this item"], ["edge.link", "Add relationship"],
    ["edge.unlink", "Remove relationship"], ["binding.bind", "Add locus binding"],
    ["binding.unbind", "Remove locus binding"]
  ];
  const descriptions = {
    "node.propose": "Prepare a new assertion. Acceptance and applicability require the owning service's proposal and review path.",
    "node.revise": "Prepare a replacement for this exact item. The current item and its evidence stay intact until a replacement is adopted.",
    "node.retire": "Request retirement of this exact item. Its evidence remains part of history; this draft does not remove it from active context.",
    "edge.link": "Describe a directed relationship between two visible items. The service must decide whether this edit is supported and admissible.",
    "edge.unlink": "Remove one exact stored relationship from this page. Its direction, label and identity are checked separately from other relationships and endpoint items.",
    "binding.bind": "Request that this item apply at a locus. The service determines the binding class and checks author/target relationships and authority.",
    "binding.unbind": "Select an exact visible locus binding. Removing applicability is distinct from retiring the item."
  };
  const el = (tag, className = "", text) => {
    const element = document.createElement(tag); element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const append = (element, ...children) => { element.append(...children); return element; };
  function button(label, action, className = "button secondary") {
    const b = el("button", className, label); b.type = "button"; b.addEventListener("click", action); return b;
  }
  function field(label, value = "", multiline = false) {
    const control = el(multiline ? "textarea" : "input", "kd-input");
    if (!multiline) control.type = "text";
    control.value = value; control.setAttribute("aria-label", label);
    return { row: append(el("label", "kd-field"), el("span", "", label), control), control };
  }
  function selector(label, values, selected) {
    const control = el("select", "kd-input"); control.setAttribute("aria-label", label);
    for (const [value, text] of values) { const option = el("option", "", text); option.value = value; option.selected = value === selected; control.append(option); }
    return { row: append(el("label", "kd-field"), el("span", "", label), control), control };
  }
  function mount(host, { applicationId, principal, basis, snapshot, target, item, relationships, bindings, visibleItems = [], practice, onReview, onSubmit, onInvalidate, onPreview, onReturn }) {
    const captured = clone({ principal, basis, snapshot, target, item, relationships, bindings, visibleItems });
    const practiceContext = practice !== undefined && practice !== null;
    const practiceAction = practiceContext ? practice.action : "";
    const practiceOperations = new Map([["create", "node.propose"], ["revise", "node.revise"], ["retire", "node.retire"], ["applicability", "binding.bind"]]);
    const practiceProblem = !practiceContext ? "" : !practiceOperations.has(practiceAction) || typeof practice.author !== "string" || (practice.target !== undefined && typeof practice.target !== "string") ? "This practice action is unavailable. Choose a supported action with requested loci."
      : practiceAction === "create" ? captured.item ? "Practice creation requires a new-item context. Return to Practices to prepare a new practice." : ""
      : captured.item?.kind !== "practice" ? "This action requires an exact captured practice. The selected item is not available as a practice." : "";
    const practiceLabels = new Map([["node.propose", "New practice"], ["node.revise", "Revise this practice"], ["node.retire", "Retire this practice"], ["binding.bind", "Add practice applicability"], ["binding.unbind", "Remove practice applicability"]]);
    const editorActions = practiceContext ? actions.filter(([action]) => practiceLabels.has(action)).map(([action]) => [action, practiceLabels.get(action)]) : actions;
    const availableActions = practiceProblem ? [] : editorActions.filter(([action]) => practiceContext
      ? practiceAction === "create" ? action === "node.propose" : practiceAction === "applicability" ? action.startsWith("binding.") : action === "node.revise" || action === "node.retire"
      : captured.item || action === "node.propose");
    const availableItems = captured.visibleItems.filter((row, index, rows) => row.id !== captured.item?.id && rows.findIndex(other => other.id === row.id) === index);
    let operation = practiceContext ? practiceOperations.get(practiceAction) || "" : item ? "node.revise" : "node.propose", draft = null, result = null, verifiedRelated = null, active = false, pending = false, disposed = false, generation = 0, controller = null;
    const urls = new Set();
    const root = el("section", "panel knowledge-draft"); root.setAttribute("aria-label", practiceContext ? "Practice change editor" : "Knowledge change editor"); root.setAttribute("role", "region");
    root.tabIndex = -1;
    const title = el("h2", "", practiceContext ? "Prepare a practice change" : "Prepare a knowledge change"); title.tabIndex = -1;
    const heading = append(el("header", "panel-heading"), append(el("div"), el("span", "eyebrow", practiceContext ? "Practice administration" : "Knowledge administration"), title));
    const open = button(practiceContext ? "Prepare practice change" : "Prepare knowledge change", () => begin());
    const status = el("p", "kd-status", practiceProblem || "Draft a change and inspect its visible relationships and applicability before handing it to the owning service."); status.setAttribute("role", "status"); status.tabIndex = -1;
    const content = el("div", "kd-content"); heading.append(open); root.append(heading, status, content); host.replaceChildren(root);
    function invalidateResult() {
      generation++; controller?.abort(); controller = null; pending = false; result = null; verifiedRelated = null;
      for (const url of urls) URL.revokeObjectURL(url); urls.clear();
    }
    function emitPreview() {
      if (typeof onPreview !== "function") return;
      const hasDraft = active && draft !== null;
      onPreview({ active: hasDraft, operation: hasDraft ? operation : "", arguments: hasDraft ? clone(argumentsOf()) : null,
        valid: hasDraft && !issue(), checked: hasDraft && result !== null, related: hasDraft && result && verifiedRelated ? clone(verifiedRelated) : null });
    }
    function focusEditor() {
      title.focus({ preventScroll: true }); root.scrollIntoView({ block: "start", behavior: "auto" });
    }
    function begin({ operation: requestedOperation, edgeId, bindingId, relatedId, direction } = {}) {
      if (disposed) return false;
      if (practiceProblem) { status.textContent = practiceProblem; focusEditor(); return false; }
      if (active) {
        if (requestedOperation !== undefined || edgeId !== undefined || bindingId !== undefined || relatedId !== undefined || direction !== undefined) status.textContent = practiceContext ? "Finish or discard the current practice draft before starting another change." : "Finish or discard the current knowledge draft before starting another change.";
        focusEditor(); return false;
      }
      const nextOperation = requestedOperation ?? operation;
      const seed = {};
      if (!availableActions.some(([action]) => action === nextOperation)) {
        status.textContent = "That change is unavailable for this captured item."; focusEditor(); return false;
      }
      if (nextOperation === "edge.unlink" && edgeId !== undefined) {
        if (!captured.relationships?.items.some(row => row.id === edgeId)) {
          status.textContent = "Choose an exact relationship from the current page."; focusEditor(); return false;
        }
        seed.edge_id = edgeId;
      }
      if (nextOperation === "binding.unbind" && bindingId !== undefined) {
        if (!captured.bindings?.items.some(row => row.id === bindingId)) {
          status.textContent = "Choose an exact binding from the current page."; focusEditor(); return false;
        }
        seed.binding_id = bindingId;
      }
      if (nextOperation === "edge.link") {
        if (relatedId !== undefined) {
          if (!availableItems.some(row => row.id === relatedId)) {
            status.textContent = "Choose a knowledge item from the current collection, or enter its exact identity in a new relationship draft."; focusEditor(); return false;
          }
          seed.to_id = relatedId;
        }
        if (direction !== undefined) {
          if (!["incoming", "outgoing"].includes(direction)) {
            status.textContent = "Choose an incoming or outgoing relationship."; focusEditor(); return false;
          }
          seed.direction = direction;
        }
      }
      operation = nextOperation; active = true; renderForm(seed); focusEditor(); return true;
    }
    function defaultDraft() {
      const current = captured.item;
      return {
        name: operation === "node.revise" ? current.name : "",
        kind: practiceContext ? "practice" : operation === "node.revise" ? current.kind : "idea",
        text: operation === "node.revise" ? current.text : "",
        author: practiceContext && operation !== "node.revise" ? practice.author : current?.author || "",
        target: practiceContext && practice.target !== undefined ? practice.target : practiceContext && operation === "node.revise" ? current.target || "" : captured.target || "", rationale: "",
        to_id: "", direction: "outgoing", rel: "", edge_id: captured.relationships?.items[0]?.id || "",
        binding_id: captured.bindings?.items[0]?.id || ""
      };
    }
    function argumentsOf() {
      if (operation === "node.propose") return { kind: draft.kind, name: draft.name, text: draft.text, author: draft.author, target: draft.target, rationale: draft.rationale };
      if (operation === "node.revise") return { supersedes: captured.item.id, kind: draft.kind, name: draft.name, text: draft.text, author: draft.author, target: draft.target, rationale: draft.rationale };
      if (operation === "node.retire") return { id: captured.item.id, rationale: draft.rationale };
      if (operation === "edge.link") return { from_id: draft.direction === "outgoing" ? captured.item.id : draft.to_id, to_id: draft.direction === "outgoing" ? draft.to_id : captured.item.id, rel: draft.rel, rationale: draft.rationale };
      if (operation === "edge.unlink") return { ...captured.relationships.items.find(row => row.id === draft.edge_id), rationale: draft.rationale };
      if (operation === "binding.bind") return { idea_id: captured.item.id, author: draft.author, target: draft.target, rationale: draft.rationale };
      return { ...captured.bindings.items.find(row => row.id === draft.binding_id), rationale: draft.rationale };
    }
    function issue() {
      if (practiceContext && draft.kind !== "practice") return "This editor retains the practice kind.";
      if (!draft.rationale.trim()) return "Explain why this change is needed.";
      if (Object.values(draft).some(value => typeof value !== "string" || value.includes("\u0000") || value !== value.toWellFormed() || bytes(value) > 8192)) return "Fields must contain valid text within 8 KiB each.";
      if (["node.propose", "node.revise", "binding.bind"].includes(operation) && (!draft.author.trim() || !draft.target.trim())) return "Name the requested authoring locus and target locus.";
      if (["node.propose", "node.revise"].includes(operation) && (!draft.name.trim() || !draft.text.trim())) return practiceContext ? "Give the practice a name and text." : "Give the item a name and text.";
      if (["node.propose", "node.revise"].includes(operation) && (bytes(draft.kind) > 128 || [draft.name, draft.author, draft.target].some(value => bytes(value) > 256 || /[\u0000-\u001f\u007f]/.test(value)))) return "Keep names and loci within 256 UTF-8 bytes, without control characters.";
      if (operation === "edge.link" && (!/^(sha256:)?[a-f0-9]{64}$/.test(draft.to_id) || !draft.rel.trim())) return "Choose another visible item by its exact identity and name the relationship.";
      if (operation === "edge.link" && (bytes(draft.rel) > 256 || bytes(draft.rationale) > 2048)) return "Relationships allow 256 bytes for the label and 2048 bytes for the reason.";
      if (operation === "edge.unlink" && !captured.relationships?.items.some(row => row.id === draft.edge_id)) return "This page has no relationship selected for removal.";
      if (operation === "edge.unlink" && (bytes(argumentsOf().rel) > 256 || bytes(draft.rationale) > 2048)) return "Relationship removal allows 256 bytes for the stored label and 2048 bytes for the reason.";
      if (operation.startsWith("node.") && bytes(draft.rationale) > 2048) return "Keep the reason within 2048 UTF-8 bytes.";
      if (operation.startsWith("binding.") && (bytes(draft.rationale) > 2048 || [argumentsOf().author || "", argumentsOf().target || ""].some(value => !value.trim() || bytes(value) > 256 || /[\u0000-\u001f\u007f]/.test(value)))) return "Bindings allow loci within 256 UTF-8 bytes without control characters, and a reason within 2048 bytes.";
      if (operation === "binding.unbind" && !captured.bindings?.items.some(row => row.id === draft.binding_id)) return "This page has no binding selected for removal.";
      if (bytes(JSON.stringify(argumentsOf())) > (operation.startsWith("node.") ? 98304 : 32768)) return "This draft exceeds the encoded request size limit.";
      return "";
    }
    function renderForm(seed = {}) {
      invalidateResult(); draft = { ...defaultDraft(), ...seed }; open.hidden = true;
      const toolbar = el("div", "kd-tools");
      const choices = selector("Change kind", availableActions, operation);
      choices.control.addEventListener("change", () => { operation = choices.control.value; renderForm(); content.querySelector("select").focus(); });
      toolbar.append(choices.row, button(practiceContext ? "Discard practice draft" : "Discard knowledge draft", () => { invalidateResult(); draft = null; active = false; content.replaceChildren(); open.hidden = false; status.textContent = practiceContext ? "Draft discarded. The practice and its applicability are unchanged." : "Draft discarded. The Knowledge graph is unchanged."; emitPreview(); open.focus(); }));
      if ((captured.item || practiceContext) && typeof onReturn === "function") toolbar.append(button(practiceContext ? "Back to practice" : "Back to relationship map", () => onReturn()));
      const practiceDescriptions = {
        "node.propose": "Prepare a new practice. The owning service checks the requested loci and requires a native Review before adoption.",
        "node.revise": "Prepare a replacement for this exact practice. Its current revision and historical evidence stay intact. Extra applicability bindings are not automatically carried to the successor; inspect the successor's applicability after adoption.",
        "node.retire": "Request retirement of this exact practice while retaining its history. Retirement and removing one applicability binding are separate changes.",
        "binding.bind": "Request that this practice apply at a locus. Requested authoring and target loci do not grant authority; the service checks the actual principal and binding direction.",
        "binding.unbind": "Remove one exact returned practice applicability binding. Its identity, authoring locus and target remain distinct from other bindings and from practice retirement."
      };
      const description = el("p", "detail-note", practiceContext ? practiceDescriptions[operation] : descriptions[operation]);
      const layout = el("div", "kd-layout"), form = el("form", "kd-form"), preview = el("section", "kd-preview"); preview.setAttribute("aria-label", practiceContext ? "Practice draft preview" : "Knowledge draft preview");
      form.addEventListener("submit", event => event.preventDefault());
      const bind = (key, label, multiline = false) => {
        const f = field(label, draft[key], multiline);
        f.control.addEventListener("input", () => { draft[key] = f.control.value; update(); }); form.append(f.row); return f.control;
      };
      if (["node.propose", "node.revise"].includes(operation)) {
        const types = ["idea", "concept", "practice", "task_concept"].map(type => [type, type.replaceAll("_", " ")]);
        if (!types.some(([type]) => type === draft.kind)) types.push([draft.kind, draft.kind]);
        const kind = selector(practiceContext ? "Practice kind" : "Knowledge kind", practiceContext ? [["practice", "practice"]] : types, draft.kind); form.append(kind.row);
        kind.control.disabled = practiceContext;
        if (!practiceContext) kind.control.addEventListener("change", () => { draft.kind = kind.control.value; update(); });
        bind("name", practiceContext ? "Practice name" : "Knowledge name"); bind("text", practiceContext ? "Practice text" : "Knowledge text", true);
      }
      if (["node.propose", "node.revise", "binding.bind"].includes(operation)) {
        bind("author", "Requested authoring locus"); bind("target", practiceContext ? "Requested target locus" : "Target locus");
        form.append(el("p", "detail-note", "A requested locus is not authority to act. The service must check your identity, ownership and binding direction."));
      }
      if (operation === "edge.link") {
        const direction = selector("Relationship direction", [["outgoing", "This item → other item"], ["incoming", "Other item → this item"]], draft.direction);
        direction.control.addEventListener("change", () => { draft.direction = direction.control.value; update(); }); form.append(direction.row);
        const visible = selector("Choose visible knowledge item", [["", "Enter an exact identity below"], ...availableItems.map(row => [row.id, (row.name || "Knowledge item") + " · " + row.id])], draft.to_id);
        form.append(visible.row);
        const identity = field("Other knowledge identity", draft.to_id); form.append(identity.row);
        visible.control.addEventListener("change", () => { draft.to_id = visible.control.value; identity.control.value = draft.to_id; update(); });
        identity.control.addEventListener("input", () => {
          draft.to_id = identity.control.value; visible.control.value = availableItems.some(row => row.id === draft.to_id) ? draft.to_id : ""; update();
        });
        bind("rel", "Relationship label");
      }
      if (operation === "edge.unlink" || operation === "binding.unbind") {
        const edge = operation === "edge.unlink", rows = edge ? captured.relationships.items : captured.bindings.items;
        const selected = selector(edge ? "Relationship to remove" : "Binding to remove", rows.map(row => [row.id, edge ? row.from_id + " → " + row.to_id + " · " + row.rel : row.target + " · " + row.class + " · " + row.author]), edge ? draft.edge_id : draft.binding_id);
        selected.control.addEventListener("change", () => { draft[edge ? "edge_id" : "binding_id"] = selected.control.value; update(); }); form.append(selected.row);
      }
      bind("rationale", practiceContext ? "Reason for practice change" : "Reason for knowledge change", true);
      const review = button(practiceContext ? "Review practice draft" : "Review knowledge draft", () => reviewDraft(), "button primary");
      const evidence = el("section", "kd-evidence"); evidence.setAttribute("role", "region"); evidence.setAttribute("aria-label", practiceContext ? "Practice change review" : "Knowledge change review");
      layout.append(form, preview); content.replaceChildren(toolbar, description, layout, review, evidence);
      function update() {
        invalidateResult(); review.disabled = Boolean(issue()); evidence.replaceChildren();
        status.textContent = issue() || "Draft ready for review. No change has been submitted.";
        preview.replaceChildren(el("h3", "", "Proposed change"), el("p", "eyebrow", editorActions.find(([action]) => action === operation)[1]));
        if (captured.item) preview.append(el("p", "mono kd-identity", captured.item.id));
        if (operation === "node.revise") preview.append(el("h4", "", "Current text"), el("pre", "kd-before", captured.item.text), el("h4", "", "Proposed text"), el("pre", "kd-after", draft.text));
        else if (operation === "node.propose") preview.append(el("h4", "", draft.name || "Untitled draft"), el("pre", "kd-after", draft.text));
        else if (operation === "edge.link") {
          const a = argumentsOf(); preview.append(append(el("div", "kd-connection"), el("span", "mono", a.from_id || "Other item"), el("strong", "", "→ " + (a.rel || "relationship") + " →"), el("span", "mono", a.to_id || "Other item")));
        } else if (operation === "binding.bind") preview.append(append(el("div", "kd-connection"), el("span", "", draft.author || "Authoring locus"), el("strong", "", "applies at"), el("span", "", draft.target || "Target locus")), el("p", "detail-note", "Binding class: determined by the service."));
        else if (operation === "node.retire") preview.append(el("p", "", "Request retirement while retaining historical evidence."));
        else preview.append(el("pre", "kd-before", JSON.stringify(argumentsOf(), null, 2)));
        preview.append(el("p", "detail-note", practiceContext ? "The current practice and its applicability are unchanged. Review uses the same source and visibility snapshot." : "Current graph unchanged. Review uses the same source and visibility snapshot."));
        emitPreview();
      }
      async function reviewDraft() {
        if (pending || issue()) return;
        invalidateResult(); evidence.replaceChildren(); const token = generation; pending = true; controller = new AbortController(); review.disabled = true;
        const reviewedOperation = operation, reviewedArguments = argumentsOf();
        status.textContent = "Checking the visible source and affected relationships…";
        emitPreview();
        try {
          const fresh = await onReview({ relatedId: operation === "edge.link" ? draft.to_id : "", operation: reviewedOperation, signal: controller.signal });
          if (disposed || token !== generation) return;
          if (operation !== reviewedOperation || JSON.stringify(argumentsOf()) !== JSON.stringify(reviewedArguments)) return;
          result = { profile: "iris.knowledge.change-draft.v1", application_id: applicationId, prepared_by: captured.principal, base: { snapshot: captured.snapshot, basis: captured.basis }, operation: reviewedOperation, arguments: reviewedArguments, source_checked: true, submitted: false };
          const supported = actions.some(([action]) => action === reviewedOperation) && fresh.commandCapability?.enabled && typeof onSubmit === "function";
          const bindingChange = reviewedOperation.startsWith("binding."), nodeChange = reviewedOperation.startsWith("node."), reviewedChange = bindingChange || nodeChange || fresh.commandCapability?.mode === "review";
          const removing = reviewedOperation === "edge.unlink";
          status.textContent = "Draft reviewed against the current visible snapshot. " + (supported ? reviewedChange ? "The service permits a proposal with a required native Review." : "The service permits direct relationship submission." : "Submission is unavailable on this connection.");
          evidence.replaceChildren(el("h3", "", "Review the change"));
          if (fresh.related) evidence.append(el("p", "", "Related item verified · " + (fresh.related.name || fresh.related.id)), el("p", "mono kd-identity", fresh.related.id));
          evidence.append(impact(fresh));
          evidence.append(el("p", "kd-service-status", supported ? bindingChange ? "Submission creates an exact binding-change candidate for native Review. Approval, the actual binding effect and graph observation remain separate. Removal observation checks the complete unfiltered binding sequence for this item, while preserving its other applicability." : nodeChange ? "Submission asks the owning service to create the exact canonical proposal and its required Review. Approval, activation and graph observation remain separate; the current item and history are preserved." : reviewedChange ? "Submission creates a canonical directed relationship proposal and a required native Review. Approval, the exact relationship effect and fresh graph observation remain separate. The graph stays unchanged until the native effect is applied." : removing ? "This command records removal of the selected exact relationship at the checked Record head. Reverse relationships, other labels and endpoint items remain separate. Graph absence requires a complete visible page sequence at one fresh snapshot." : "This command records the exact directed relationship at the checked Record head. Graph projection and observation follow separately; the graph generation is read provenance, not a transaction precondition." : fresh.commandCapability?.mode === "review" && !reviewedChange ? "The policy requires a native Review, which this connection does not yet support. Nothing can be submitted through a direct fallback." : "Awaiting an owning-service edit operation. No proposal, Review, adoption or graph mutation has been recorded."));
          const submit = button(practiceContext ? "Submit practice change" : "Submit knowledge change", async () => {
            const checked = result;
            if (!supported || !checked || token !== generation || pending || disposed) return;
            pending = true; submit.disabled = true; review.disabled = true;
            status.textContent = "Saving the request identity before submission…";
            try {
              await onSubmit(clone(checked), () => !disposed && token === generation && result === checked && !issue());
            } catch (error) {
              if (!disposed && token === generation) status.textContent = error.message || "The request could not be submitted.";
            } finally {
              if (!disposed && token === generation) { pending = false; submit.disabled = false; review.disabled = Boolean(issue()); }
            }
          }); submit.disabled = !supported;
          const download = button(practiceContext ? "Download practice draft" : "Download knowledge draft", () => {
            if (!result || token !== generation) return;
            const url = URL.createObjectURL(new Blob([JSON.stringify(result, null, 2) + "\n"], { type: "application/json" })); urls.add(url);
            const a = el("a"); a.href = url; a.download = practiceContext ? "practice-change.draft.json" : "knowledge-change.draft.json"; a.click();
          });
          evidence.append(submit, download, append(el("details"), el("summary", "", "Draft identity and source evidence"), el("pre", "kd-evidence-json", JSON.stringify(result, null, 2))));
          verifiedRelated = fresh.related ? clone(fresh.related) : null; emitPreview();
          if (root.contains(document.activeElement)) status.focus();
        } catch (error) {
          if (disposed || token !== generation) return;
          invalidateResult(); evidence.replaceChildren();
          if ([401, 403, 404, 409].includes(error.status)) { draft = null; active = false; content.replaceChildren(); emitPreview(); onInvalidate(error); }
          else { review.disabled = Boolean(issue()); status.textContent = error.message || "The service could not check this snapshot. The draft has not been submitted."; emitPreview(); }
        } finally { if (token === generation) { pending = false; review.disabled = Boolean(issue()); } }
      }
      update();
    }
    function impact(fresh) {
      const region = el("div", "kd-impact");
      region.append(el("h4", "", "Visible applicability and relationships"));
      if (!captured.item) { region.append(el("p", "", practiceContext ? "New practice: the service must establish its identity, provenance and applicability when a proposal is admitted." : "New item: the service must establish its identity, provenance and applicability when a proposal is admitted.")); return region; }
      for (const [label, page] of [["Locus bindings", fresh.bindings], ["Direct relationships", fresh.relationships]]) {
        const group = el("section"); group.append(el("h4", "", label + " · " + page.items.length + " on this page"));
        const list = el("ul");
        for (const row of page.items) list.append(el("li", "", label === "Locus bindings" ? row.target + " · " + row.class + " · " + row.author : row.from_id + " → " + row.to_id + " · " + row.rel));
        group.append(list, el("p", "detail-note", page.page.has_more ? "More visible items exist. This page is not a complete impact assessment." : "End of this page sequence; earlier pages may exist.")); region.append(group);
      }
      region.append(el("p", "detail-note", "Run, Definition and Practice dependencies are unavailable. Retirement, supersession and binding changes need the service's full impact and authority checks."));
      return region;
    }
    if (practiceProblem) open.hidden = true;
    else if (practiceContext) begin();
    return { begin, destroy() { if (disposed) return; disposed = true; invalidateResult(); draft = null; active = false; host.replaceChildren(); emitPreview(); } };
  }
  window.IrisKnowledgeDraft = { mount };
})();
