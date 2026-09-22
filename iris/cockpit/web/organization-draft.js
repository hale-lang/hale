/* Source-backed organization drafts. The host checks the complete captured seed. */
"use strict";
(() => {
  const PROFILE = "dna.organization.draft.v1";
  const encoder = new TextEncoder();
  const BASE = ["source_head", "dependency_digest", "dependency_source", "module_digest", "record_head"];
  const bytes = text => encoder.encode(text).length;
  const closed = (value, keys) => value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
  const same = (a, b, keys) => closed(a, keys) && keys.every(key => a[key] === b[key]);
  const el = (tag, className = "", text) => {
    const item = document.createElement(tag);
    item.className = className;
    if (text !== undefined) item.textContent = text;
    return item;
  };
  const append = (item, ...children) => { item.append(...children); return item; };
  function button(text, fn, className = "button secondary") {
    const item = el("button", className, text); item.type = "button"; item.addEventListener("click", fn); return item;
  }
  async function digest(text) {
    const hash = await crypto.subtle.digest("SHA-256", encoder.encode(text));
    return "sha256:" + [...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, "0")).join("");
  }
  function capable(c) {
    return closed(c, ["profile", "supported", "validation", "publication", "module", "max_request_bytes", "max_source_bytes", "max_instances"]) && c.profile === PROFILE && c.supported === true && c.validation === true && c.publication === false && c.module === "dna/org/main.hl" && c.max_request_bytes === "32768" && c.max_source_bytes === "16384" && c.max_instances === "256";
  }
  function ownershipCapable(c) {
    return closed(c, ["profile", "supported", "validation", "publication", "module", "max_request_bytes", "max_source_bytes", "max_entries"]) && c.profile === "dna.organization.ownership.draft.v1" && c.supported === true && c.validation === true && c.publication === false && c.module === "dna/org/owners" && c.max_request_bytes === "32768" && c.max_source_bytes === "16384" && c.max_entries === "256";
  }
  function ownershipValid(data) {
    const o = data.ownership, p = data.impact, string = x => typeof x === "string", strings = x => Array.isArray(x) && x.every(string);
    return closed(o, ["mode", "host_owner", "instance_binding", "positions", "memberships"]) && ["single_owner", "shared"].includes(o.mode) && string(o.host_owner) && o.instance_binding === "unavailable" &&
      Array.isArray(o.positions) && o.positions.length <= 256 && o.positions.every(row => closed(row, ["position", "owner"]) && string(row.position) && string(row.owner)) &&
      Array.isArray(o.memberships) && o.memberships.length <= 256 && o.memberships.every(row => closed(row, ["owner", "members"]) && string(row.owner) && strings(row.members)) &&
      closed(p, ["host_changed", "mode_changed", "live_obligations", "affected_owners", "scopes"]) && typeof p.host_changed === "boolean" && typeof p.mode_changed === "boolean" && p.live_obligations === "unavailable" &&
      Array.isArray(p.affected_owners) && p.affected_owners.length <= 512 && p.affected_owners.every(row => closed(row, ["owner", "members"]) && string(row.owner) && strings(row.members)) &&
      Array.isArray(p.scopes) && p.scopes.length <= 512 && p.scopes.every(row => closed(row, ["position", "before_owner", "after_owner"]) && string(row.position) && string(row.before_owner) && string(row.after_owner));
  }
  function ownershipForm(source, data, selectedId, onSelect, onApply) {
    const panel = el("section", "org-structure-form"); panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Ownership map editor");
    const error = el("p", "org-form-error"); error.setAttribute("role", "alert"); panel.append(error);
    const entries = []; let start = 0;
    for (const line of source.match(/[^\n]*\n|[^\n]+$/g) || []) {
      const text = line.trim(), eq = text.indexOf("="), colon = text.indexOf(":");
      if (text && !text.startsWith("#")) {
        const assignment = eq >= 0 && (colon < 0 || eq < colon), at = assignment ? eq : colon;
        entries.push({ start, end: start + line.length, name: text.slice(0, at).trim(), value: text.slice(at + 1).trim(), kind: assignment ? text.slice(0, at).trim() === "host" ? "host" : "assignment" : "membership", ending: line.endsWith("\r\n") ? "\r\n" : line.endsWith("\n") ? "\n" : "" });
      }
      start += line.length;
    }
    const safe = (value, empty = false) => (empty && !value) || (bytes(value) <= 1024 && value.length > 0 && !/[\s\u0000-\u001f\u007f#,:=]/.test(value));
    const normalized = value => value.replace(/\/+$/, "") || "/";
    const field = (label, input) => { input.setAttribute("aria-label", label); return append(el("label", "org-structured-field"), el("span", "", label), input); };
    function change(entry, text) {
      error.textContent = "";
      const candidate = entry ? source.slice(0, entry.start) + text + (text ? entry.ending : "") + source.slice(entry.end) : source + (source && !source.endsWith("\n") ? "\n" : "") + text + "\n";
      onApply(candidate, "");
    }
    for (const kind of ["assignment", "membership"]) {
      const assignments = kind === "assignment", group = append(el("fieldset"), el("legend", "", assignments ? "Scope assignment" : "Owner membership"));
      const select = el("select"), fresh = el("option", "", assignments ? "New scope" : "New membership"); fresh.value = ""; select.append(fresh);
      const rows = entries.filter(row => row.kind === kind);
      rows.forEach((row, index) => { const option = el("option", "", row.name); option.value = String(index); select.append(option); });
      const name = el("input"), value = el("input"); name.autocomplete = value.autocomplete = "off";
      const remove = button(assignments ? "Validate assignment removal" : "Validate membership removal", () => { if (select.value !== "") change(rows[Number(select.value)], ""); }); remove.disabled = true;
      select.addEventListener("change", () => { const row = select.value === "" ? null : rows[Number(select.value)]; name.value = row?.name || ""; value.value = row?.value || ""; remove.disabled = !row; error.textContent = ""; });
      group.append(field(assignments ? "Assignment to edit" : "Membership to edit", select), field(assignments ? "Scope path" : "Owner name", name), field(assignments ? "Assigned owner" : "Members", value));
      group.append(el("p", "detail-note", assignments ? "A scope inherits its nearest declared ancestor's owner. An empty owner leaves that scope unowned. Removing an assignment restores inheritance; it does not retire work." : "Members are names separated by commas or spaces. The native preview identifies affected owners and their review members; this does not grant your current session authority."));
      group.append(button(assignments ? "Validate assignment" : "Validate membership", () => {
        const entry = select.value === "" ? null : rows[Number(select.value)];
        const key = name.value.trim(), val = value.value.trim();
        if (!safe(key) || (assignments && key === "host") || rows.some(row => row !== entry && (assignments ? normalized(row.name) === normalized(key) : row.name === key))) { error.textContent = "Use a unique scope or owner name without separators or whitespace."; return; }
        if (assignments ? !safe(val, true) : !val.split(/[ ,\t]+/).filter(Boolean).every(word => safe(word))) { error.textContent = "Use individual names without assignment separators or newlines."; return; }
        change(entry, key + (assignments ? " = " : ": ") + val);
      }, "button primary"), remove); panel.append(group);
    }
    const host = entries.find(row => row.kind === "host"), hosting = append(el("fieldset"), el("legend", "", "Hosting party")), input = el("input"); input.value = data.ownership.host_owner;
    hosting.append(field("Host owner", input), el("p", "detail-note", "Changing who hosts the shared Ledger affects every declared owner's trust decision."), button("Validate hosting party", () => {
      if (!safe(input.value.trim(), true)) { error.textContent = "Use one hosting-party name without separators or whitespace."; return; }
      change(host, "host = " + input.value.trim());
    }, "button primary")); panel.append(hosting);
    return panel;
  }
  function ownershipImpact(data) {
    const panel = append(el("section"), el("h3", "", "Ownership change review"));
    panel.append(el("p", "", "Candidate mode · " + (data.ownership.mode === "shared" ? "Shared ownership" : "Single owner") + " · Host · " + (data.ownership.host_owner || "Not declared")));
    if (data.impact.mode_changed) panel.append(el("p", "org-form-error", "Ownership mode changes. With no position assignments, DNA uses single-owner admission."));
    if (data.impact.host_changed) panel.append(el("p", "detail-note", "The hosting party changes; every affected owner appears in the review list."));
    const scopes = append(el("table", "declaration-table"), append(el("thead"), append(el("tr"), el("th", "", "Declared scope"), el("th", "", "Current owner"), el("th", "", "Candidate owner")))), body = el("tbody");
    for (const row of data.impact.scopes) body.append(append(el("tr"), el("td", "mono", row.position), el("td", "", row.before_owner || "Unowned"), el("td", "", row.after_owner || "Unowned")));
    scopes.append(body); panel.append(scopes, el("p", "detail-note", "Scope rows cover names declared in either map, including inherited owners after removal. Single-owner mode is shown separately; this is not a complete inventory of active work."), el("h4", "", "Affected owner reviews"));
    const reviews = el("ul");
    for (const row of data.impact.affected_owners) reviews.append(el("li", "", row.owner + " · " + (row.members.length ? row.members.join(", ") : "No review members declared")));
    if (!reviews.childElementCount) reviews.append(el("li", "", "No owner holdings, membership or hosting changes."));
    panel.append(reviews, el("p", "detail-note", "This is the native source model's review preview. Live obligations, active assignments and permission grants are unavailable here. Publication and activation require the governed source-change service."));
    return panel;
  }
  // Locate editable source spans without reprinting handwritten Hale. The native
  // compiler remains the authority for syntax, contracts and the resulting chart.
  function sourceModel(source) {
    const tokens = [], mates = new Map(), stack = [];
    const fail = () => { throw new Error("This source shape needs the Hale source editor."); };
    let i = 0;
    while (i < source.length) {
      if (/\s/.test(source[i])) { i++; continue; }
      if (source.startsWith("//", i)) { const end = source.indexOf("\n", i); i = end < 0 ? source.length : end; continue; }
      if (source.startsWith("/*", i)) {
        i += 2; let depth = 1;
        while (i < source.length && depth) { if (source.startsWith("/*", i)) { depth++; i += 2; } else if (source.startsWith("*/", i)) { depth--; i += 2; } else i++; }
        if (depth) fail(); continue;
      }
      const start = i, quote = source[i];
      if (quote === '"' || quote === "'") {
        i++; let closed = false;
        while (i < source.length) { if (source[i] === "\\") i += 2; else if (source[i++] === quote) { closed = true; break; } }
        if (!closed) fail();
      } else if (/[A-Za-z_]/.test(source[i])) { while (i < source.length && /[A-Za-z0-9_]/.test(source[i])) i++; }
      else if (source.startsWith("::", i)) i += 2;
      else i++;
      const value = source.slice(start, i), index = tokens.length;
      tokens.push({ value, start, end: i });
      if (["{", "(", "["].includes(value)) stack.push(index);
      else if (["}", ")", "]"].includes(value)) {
        const open = stack.pop();
        if (open === undefined || "{([".indexOf(tokens[open].value) !== "})]".indexOf(value)) fail();
        mates.set(open, index);
      }
    }
    if (stack.length) fail();
    const ident = value => /^[A-Za-z_][A-Za-z0-9_]*$/.test(value || "");
    const value = index => tokens[index]?.value;
    const text = (a, b) => a < b ? source.slice(tokens[a].start, tokens[b - 1].end) : "";
    const span = (a, b) => ({ start: tokens[a].start, end: tokens[b - 1].end });
    function fields(open, delimiter) {
      const entries = []; let from = open + 1, p = from, end = mates.get(open);
      while (p < end) {
        if (mates.has(p)) { p = mates.get(p) + 1; continue; }
        if (value(p) === delimiter) { if (p > from) entries.push({ from, to: p, end: p + 1 }); from = p + 1; }
        p++;
      }
      if (from < end) entries.push({ from, to: end, end });
      return entries;
    }
    const declarations = new Map(); let positions = null;
    for (let p = 0; p < tokens.length; p++) {
      if (value(p) === "locus" && ident(value(p + 1)) && value(p + 2) === "{") {
        const open = p + 2, end = mates.get(open), name = value(p + 1);
        if (declarations.has(name)) fail();
        const declaration = { name, open, end, fields: [], params: null, main: value(p - 1) === "main" };
        for (let q = open + 1; q < end; q++) {
          if (value(q) === "params" && value(q + 1) === "{") {
            if (declaration.params !== null) fail();
            declaration.params = q + 1;
            for (const f of fields(q + 1, ";")) {
              if (!ident(value(f.from)) || value(f.from + 1) !== ":") continue;
              let equal = f.from + 2;
              while (equal < f.to && value(equal) !== "=" && !mates.has(equal)) equal++;
              const field = { ...f, ...span(f.from, f.end), name: value(f.from), nameSpan: span(f.from, f.from + 1), type: text(f.from + 2, equal).replace(/\s/g, ""), equal, expression: equal < f.to ? text(equal + 1, f.to) : "" };
              if (value(equal) === "=" && ident(value(equal + 1)) && value(equal + 2) === "{" && mates.get(equal + 2) === f.to - 1 && field.type === value(equal + 1)) {
                field.constructor = { name: value(equal + 1), open: equal + 2, end: f.to - 1, fields: fields(equal + 2, ",") };
              }
              declaration.fields.push(field);
            }
          }
          if (mates.has(q)) q = mates.get(q);
        }
        declarations.set(name, declaration); p = end;
      } else if (value(p) === "group" && value(p + 1) === "positions" && value(p + 2) === "=" && value(p + 3) === "{") {
        if (positions) fail();
        const open = p + 3, end = mates.get(open), members = fields(open, ",");
        positions = { open, end, members, mayBeEmpty: value(end + 1) === "may_be_empty", simple: members.every(f => f.to === f.from + 1 && ident(value(f.from))) };
        p = end;
      } else if (mates.has(p)) p = mates.get(p);
    }
    function binding(row, rows) {
      const declaration = declarations.get(row.declaration);
      if (!declaration || row.source_file !== "dna/org/main.hl") return { declaration: null, field: null, parent: null, affected: [] };
      const parent = rows.find(item => item.id === row.parent_id);
      const parentDeclaration = parent?.source_file === "dna/org/main.hl" ? declarations.get(parent.declaration) : null;
      const candidates = (parentDeclaration?.fields || []).filter(f => f.constructor?.name === row.declaration && row.id === parent.id + "." + f.name);
      const field = candidates.length === 1 ? candidates[0] : null;
      const affected = field ? rows.filter(item => item.declaration === parent.declaration).map(item => rows.find(child => child.parent_id === item.id && child.id === item.id + "." + field.name)).filter(Boolean) : [];
      return { declaration, field, parent: parentDeclaration, affected };
    }
    function patch(edits) {
      let result = source, previous = source.length + 1;
      for (const edit of edits.sort((a, b) => b.start - a.start)) {
        if (edit.end > previous || edit.start < 0 || edit.end < edit.start) fail();
        result = result.slice(0, edit.start) + edit.text + result.slice(edit.end); previous = edit.start;
      }
      return result;
    }
    function childInsertion(declaration, name, child, entry, removedField = null) {
      if (!ident(name) || declaration.fields.some(f => f.name === name)) throw new Error("Choose a unique Hale identifier for this child.");
      if (!declarations.has(child) || declarations.get(child).main) throw new Error("Choose a local child declaration.");
      const seen = new Set();
      function reaches(current) { if (current === declaration.name) return true; if (seen.has(current)) return false; seen.add(current); return (declarations.get(current)?.fields || []).some(f => f !== removedField && f.constructor && reaches(f.constructor.name)); }
      if (reaches(child)) throw new Error("That declaration would create recursive static containment.");
      const at = declaration.params === null ? tokens[declaration.open].end : tokens[mates.get(declaration.params)].start;
      return { start: at, end: at, text: declaration.params === null ? "\n    params { " + entry + " }\n" : "\n        " + entry + "\n    " };
    }
    function insertChild(declaration, name, child) {
      return patch([childInsertion(declaration, name, child, name + ": " + child + " = " + child + " { };")]);
    }
    function moveChild(binding, destination, name) {
      if (binding.parent === destination) throw new Error("Choose a different parent declaration. Use Instance name to rename within this declaration.");
      const f = binding.field;
      // Preserve the complete type, initializer and inline comments. A move is
      // a declaration edit; the compiler checks references and the whole chart.
      const entry = name + source.slice(f.nameSpan.end, f.end);
      const insertion = childInsertion(destination, name, f.constructor.name, entry, f);
      return patch([{ start: f.start, end: f.end, text: "" }, insertion]);
    }
    function roleEdit(name, enabled, allowEmpty) {
      if (!positions) {
        if (!enabled) return null;
        return { start: source.length, end: source.length, text: "\ngroup positions = { " + name + " };\n" };
      }
      if (!positions.simple) throw new Error("The positions group uses a pattern or qualified declaration. Edit its exact Hale source.");
      const member = positions.members.find(f => value(f.from) === name);
      if (enabled && !member) { const at = tokens[positions.end].start, last = positions.members.at(-1); return { start: at, end: at, text: (last && value(last.end - 1) !== "," ? ", " : " ") + name + " " }; }
      if (!enabled && member) {
        const index = positions.members.indexOf(member), last = index === positions.members.length - 1;
        const start = last && index > 0 && member.end === member.to ? tokens[positions.members[index - 1].to].start : tokens[member.from].start;
        const removal = { start, end: tokens[member.end - 1].end, text: "" };
        if (positions.members.length === 1 && !positions.mayBeEmpty) {
          if (!allowEmpty) throw new Error("Removing the last position requires explicitly allowing an empty positions group.");
          return [removal, { start: tokens[positions.end].end, end: tokens[positions.end].end, text: " may_be_empty" }];
        }
        return removal;
      }
      return null;
    }
    return { declarations, positions, tokens, binding, patch, insertChild, moveChild, roleEdit, ident };
  }

  function structureForm(source, data, selectedId, onSelect, onApply) {
    const panel = el("section", "org-structure-form"); panel.setAttribute("aria-label", "Structured organization editor");
    panel.setAttribute("role", "region");
    let model;
    try { model = sourceModel(source); } catch (error) { return append(panel, el("p", "detail-note", error.message)); }
    const rows = data.projection.items;
    if (!rows.length) return append(panel, el("p", "detail-note", "No checked instances to edit. Use Hale source to declare the organization root."));
    const row = rows.find(item => item.id === selectedId) || rows[0];
    function field(label, input) { input.setAttribute("aria-label", label); return append(el("label", "org-structured-field"), el("span", "", label), input); }
    const picker = el("select");
    for (const item of rows) { const option = el("option", "", item.id); option.value = item.id; picker.append(option); }
    picker.value = row.id; picker.addEventListener("change", () => onSelect(picker.value));
    panel.append(field("Instance to edit", picker), el("h3", "", row.id), el("p", "detail-note", "Declaration · " + row.declaration));
    const binding = model.binding(row, rows), declaration = binding.declaration;
    if (!declaration) return append(panel, el("p", "detail-note", "This declaration is outside the editable module or has no unambiguous local source binding. Its source remains unchanged."));
    const shared = rows.filter(item => item.declaration === row.declaration);
    const errors = el("p", "org-form-error"); errors.setAttribute("role", "alert"); panel.append(errors);
    const apply = action => {
      try { errors.textContent = ""; const result = action(); if (result.text === source) { errors.textContent = "No source changes to validate."; return; } onApply(result.text, result.selection || row.id); }
      catch (error) { errors.textContent = error.message; }
    };
    function affected(title, items) {
      const list = el("ul", "org-affected-instances"); items.forEach(item => list.append(el("li", "mono", item.id)));
      return append(el("section", "org-source-scope"), el("h4", "", title + " · " + items.length), list);
    }
    if (binding.field) {
      const group = append(el("fieldset"), el("legend", "", "Declared instance"));
      const name = el("input"); name.value = binding.field.name; name.autocomplete = "off"; name.spellcheck = false;
      group.append(field("Instance name", name), affected("This source field declares", binding.affected));
      group.append(el("p", "detail-note", "Renaming changes these instance paths. References elsewhere remain in the source and are checked by Hale."));
      group.append(button("Validate instance name", () => apply(() => {
        if (!model.ident(name.value) || binding.parent.fields.some(f => f !== binding.field && f.name === name.value)) throw new Error("Choose a unique Hale identifier for this instance.");
        return { text: model.patch([{ ...binding.field.nameSpan, text: name.value }]), selection: row.parent_id + "." + name.value };
      }), "button primary"));
      const removal = el("details"); removal.append(el("summary", "", "Remove this source field"));
      const roots = new Set(binding.affected.map(item => item.id)), removed = new Set(roots);
      let changed = true;
      while (changed) { changed = false; for (const item of rows) if (removed.has(item.parent_id) && !removed.has(item.id)) { removed.add(item.id); changed = true; } }
      removal.append(affected("Instances removed from the declared chart", rows.filter(item => removed.has(item.id))), el("p", "detail-note", "This removes source declarations only. Live obligations, ownership reassignment and retirement need the owning services; they are not resolved by this draft."));
      const confirm = el("input"); confirm.type = "checkbox";
      const remove = button("Validate source removal", () => apply(() => ({ text: model.patch([{ start: binding.field.start, end: binding.field.end, text: "" }]), selection: row.parent_id }))); remove.disabled = true;
      confirm.addEventListener("change", () => { remove.disabled = !confirm.checked; });
      removal.append(field("I reviewed every affected instance", confirm), remove); group.append(removal); panel.append(group);

      const move = append(el("fieldset"), el("legend", "", "Move to another parent"));
      const destination = el("select"), movedName = el("input"); movedName.value = binding.field.name; movedName.autocomplete = "off"; movedName.spellcheck = false;
      for (const item of rows) {
        const target = model.binding(item, rows).declaration;
        if (!target || target === binding.parent) continue;
        const option = el("option", "", item.id + " · " + target.name); option.value = item.id; destination.append(option);
      }
      const impact = el("div"), reviewed = el("input"); reviewed.type = "checkbox";
      const validateMove = button("Validate move", () => apply(() => {
        if (!reviewed.checked) throw new Error("Review the source and destination instances before moving.");
        const target = rows.find(item => item.id === destination.value);
        if (!target) throw new Error("Choose a local destination parent.");
        return { text: model.moveChild(binding, model.binding(target, rows).declaration, movedName.value), selection: target.id + "." + movedName.value };
      }), "button primary"); validateMove.disabled = true;
      function showMoveImpact() {
        reviewed.checked = false; validateMove.disabled = true;
        const target = rows.find(item => item.id === destination.value);
        const parents = target ? rows.filter(item => item.declaration === target.declaration && item.source_file === target.source_file) : [];
        impact.replaceChildren(affected("Current source instances and descendants", rows.filter(item => removed.has(item.id))), affected("Destination declaration shared by", parents));
        if (model.ident(movedName.value)) impact.append(affected("Proposed child paths before native validation", parents.map(item => ({ id: item.id + "." + movedName.value }))));
      }
      destination.addEventListener("change", showMoveImpact); movedName.addEventListener("input", showMoveImpact);
      reviewed.addEventListener("change", () => { validateMove.disabled = !reviewed.checked || !destination.options.length; });
      move.append(field("Destination parent", destination), field("Child name after move", movedName), el("p", "detail-note", "Moving edits both parent declarations. Shared parents can change the number of instances. The constructor is preserved; Hale checks the resulting chart and any remaining references. Ownership and live obligations are unchanged."), impact, field("I reviewed both parent declarations", reviewed), validateMove);
      if (!destination.options.length) move.append(el("p", "detail-note", "No other local parent declaration is available. Use Hale source to prepare a new parent."));
      showMoveImpact(); panel.append(move);
    } else panel.append(el("p", "detail-note", "The root or this construction has no editable child field. Declaration changes below still apply to its checked instances."));

    const membership = append(el("fieldset"), el("legend", "", "Position declaration"));
    const position = el("input"); position.type = "checkbox"; position.checked = row.role === "position";
    position.disabled = Boolean(model.positions && !model.positions.simple);
    membership.append(field("Include this declaration in positions", position), affected("Declaration shared by", shared), el("p", "detail-note", "Group membership applies to every instance of this declaration. It establishes no occupant, ownership assignment or command permission."));
    const allowEmpty = el("input"); allowEmpty.type = "checkbox";
    if (model.positions?.members.length === 1 && !model.positions.mayBeEmpty && row.role === "position") membership.append(field("Allow an empty positions group", allowEmpty), el("p", "detail-note", "Required to remove the last member. This explicitly changes the source rule that positions must exist."));
    const role = button("Validate position membership", () => apply(() => { const edit = model.roleEdit(declaration.name, position.checked, allowEmpty.checked); return { text: edit ? model.patch(Array.isArray(edit) ? edit : [edit]) : source }; }));
    role.disabled = position.disabled; membership.append(role);
    if (position.disabled) membership.append(el("p", "detail-note", "This group uses patterns or qualified names; edit its exact source instead."));
    panel.append(membership);

    const defaults = append(el("fieldset"), el("legend", "", "Declaration defaults")), changes = [];
    for (const f of declaration.fields) {
      let input, initial, encode;
      if (f.type === "Int" && /^-?\s*\d+$/.test(f.expression)) {
        initial = f.expression.replace(/\s/g, ""); input = el("input"); input.value = initial; input.inputMode = "numeric";
        encode = value => { if (!/^-?(0|[1-9][0-9]*)$/.test(value) || BigInt(value) < -9223372036854775808n || BigInt(value) > 9223372036854775807n) throw new Error(f.name + " must be a whole Int64 value."); return value; };
      } else if (f.type === "Bool" && ["true", "false"].includes(f.expression)) {
        initial = f.expression; input = el("select"); for (const v of ["true", "false"]) { const option = el("option", "", v); option.value = v; input.append(option); } input.value = initial; encode = value => value;
      } else if (f.type === "String" && f.expression.startsWith('"')) {
        try { initial = JSON.parse(f.expression); } catch { continue; }
        if (typeof initial !== "string") continue;
        input = el("textarea"); input.value = initial;
        encode = value => { if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value) || value !== value.toWellFormed()) throw new Error(f.name + " contains unsupported characters."); return JSON.stringify(value); };
      } else continue;
      defaults.append(field(f.name + " default (" + f.type + ")", input));
      changes.push(() => input.value === initial ? null : { start: model.tokens[f.equal + 1].start, end: model.tokens[f.to - 1].end, text: encode(input.value) });
    }
    if (changes.length) {
      defaults.append(el("p", "detail-note", "Defaults belong to " + declaration.name + " and may be overridden by individual constructors. Existing methods, contracts and explicit overrides are preserved."), affected("Instances using this declaration", shared), button("Validate declaration defaults", () => apply(() => ({ text: model.patch(changes.map(change => change()).filter(Boolean)) })), "button primary"));
      panel.append(defaults);
    }
    const children = append(el("fieldset"), el("legend", "", "Add a child"));
    const name = el("input"); name.autocomplete = "off"; name.spellcheck = false;
    const choice = el("select");
    for (const child of model.declarations.values()) if (!child.main && child.name !== declaration.name) { const option = el("option", "", child.name); option.value = child.name; choice.append(option); }
    children.append(field("New child name", name), field("Child declaration", choice), affected("Parent declaration shared by", shared), el("p", "detail-note", "A new child is constructed in each of these parent instances, using its declaration defaults."));
    const add = button("Validate new child", () => apply(() => ({ text: model.insertChild(declaration, name.value, choice.value), selection: row.id + "." + name.value })), "button primary"); add.disabled = !choice.options.length;
    children.append(add); panel.append(children);
    return panel;
  }

  function mount(host, { applicationId, principal, basis, recordHead, capability, validateProjection, onInvalidate, selectedId = "", ownership = false, publicationAccess = () => ({ allowed: false, reason: "Organization publishing is unavailable on this connection." }), onPropose = null }) {
    const profile = ownership ? "dna.organization.ownership.draft.v1" : PROFILE;
    const enabled = ownership ? ownershipCapable(capability) : capable(capability);
    let original = null, result = null, pending = false, disposed = false, version = 0, controller = null, source = "";
    let checked = null, selected = selectedId, mode = "source";
    const urls = new Set();
    const root = el("section", "panel org-draft"); root.setAttribute("aria-label", ownership ? "Ownership editing" : "Organization editing"); root.setAttribute("role", "region");
    const heading = append(el("header", "panel-heading"), append(el("div"), el("span", "eyebrow", "Organization administration"), el("h2", "", ownership ? "Prepare an ownership change" : "Prepare a source change")));
    const open = button(ownership ? "Edit ownership source" : "Edit organization source", () => load("source"));
    const guided = button(ownership ? "Edit ownership" : "Edit organization", () => load("structured"), "button primary");
    const status = el("p", "org-draft-status", enabled ? ownership ? "Edit scopes, owner membership and hosting; review the native impact before exporting source." : "Edit the declared structure and inspect the checked change before exporting or proposing it." : "Source editing is unavailable on this connection.");
    status.setAttribute("role", "status"); status.tabIndex = -1;
    const content = el("div", "org-draft-content");
    heading.append(guided, open); guided.disabled = open.disabled = !enabled;
    root.append(heading, status, content); host.replaceChildren(root);
    const endpoint = "/api/hale/v1/applications/" + encodeURIComponent(applicationId) + (ownership ? "/dna/organization/ownership/draft" : "/dna/organization/draft");
    const fail = (message, status = 0) => Object.assign(new Error(message), { status });
    function revoke() { for (const url of urls) URL.revokeObjectURL(url); urls.clear(); }
    function clear() { result = null; revoke(); }
    function invalidate(error) {
      clear(); original = checked = null; source = ""; content.replaceChildren(); onInvalidate(error);
    }
    async function request(body) {
      controller?.abort(); controller = new AbortController();
      const active = controller, timer = setTimeout(() => active.abort(), 35000);
      try {
        const response = await fetch(endpoint, { method: body ? "POST" : "GET", credentials: "same-origin", cache: "no-store", signal: active.signal,
          ...(body ? { headers: { "Content-Type": "application/json", "X-Hale-Command": "1" }, body } : {}) });
        const text = await response.text();
        if (bytes(text) > 2097152) throw fail("The service response exceeds the editor limit.");
        let value;
        try { value = JSON.parse(text); } catch { throw fail("The service did not return a valid source response."); }
        if (!response.ok) throw fail(value.error?.message || "The source request failed.", response.status);
        if (value.api_version !== "hale.v1" || value.source?.record_id !== applicationId || value.source?.record_head !== recordHead) throw fail("The application's Record changed. Reload before editing.", 409);
        const data = value.data;
        if (!closed(data, ["profile", "principal", "base", "module", ...(ownership ? ["ownership", "impact"] : ["projection"]), "validation", "publication"]) || data.profile !== profile || !same(data.principal, principal, ["mode", "name"])) throw fail("The signed-in identity or source response changed.", 409);
        if (!closed(data.base, BASE) || !BASE.every(key => typeof data.base[key] === "string" && data.base[key]) || data.base.record_head !== recordHead || ["source_head", "dependency_digest", "dependency_source"].some(key => data.base[key] !== basis[key])) throw fail("The source or dependencies changed. Reload before editing.", 409);
        if (!closed(data.module, ["path", "text", "digest"]) || data.module.path !== capability.module || typeof data.module.text !== "string" || bytes(data.module.text) > 16384 || data.module.text.includes("\u0000") || data.module.digest !== await digest(data.module.text) || data.publication !== "unavailable") throw fail("The source module or its digest could not be verified.");
        if (ownership) {
          if (!ownershipValid(data)) throw fail("The native ownership preview is invalid.");
        } else {
        if (!data.projection || !Array.isArray(data.projection.items) || data.projection.items.length > 256) throw fail("The checked chart is unavailable.");
        validateProjection(data.projection);
        if (["source_head", "dependency_digest", "dependency_source"].some(key => data.projection.basis[key] !== data.base[key])) throw fail("The checked chart has a different source basis.");
        }
        if (!body && (data.validation !== (ownership ? "parsed_source" : "checked_source") || data.base.module_digest !== data.module.digest)) throw fail("The service did not return the checked committed module.");
        if (body && (data.validation !== "valid_draft" || !same(data.base, original.base, BASE) || data.module.text !== source)) throw fail("The validation does not match this exact source draft.");
        return data;
      } catch (error) {
        if (active.signal.aborted && !disposed) throw fail("The check did not finish. You can explicitly try again; no change was applied.");
        throw error;
      } finally { clearTimeout(timer); }
    }
    async function load(nextMode = "source") {
      if (pending || !enabled) return;
      mode = nextMode;
      const token = ++version; pending = true; guided.disabled = open.disabled = true; status.textContent = "Reading the checked source…";
      try {
        const data = await request(); if (disposed || token !== version) return;
        original = checked = data; source = data.module.text; clear(); renderEditor();
        status.textContent = !ownership && publicationAccess().allowed ? "Draft in this browser · validate the exact change before preparing a proposal." : "Draft in this browser · validation and export available · publication unavailable.";
        content.querySelector(mode === "structured" ? "select" : ".org-source-editor")?.focus();
      } catch (error) { if (!disposed && token === version) { if ([401, 403, 409].includes(error.status)) invalidate(error); else status.textContent = error.message; } }
      finally { pending = false; if (!disposed) guided.disabled = open.disabled = !enabled; }
    }
    function download(text, name, type) {
      const url = URL.createObjectURL(new Blob([text], { type })); urls.add(url);
      const a = el("a"); a.href = url; a.download = name; a.click();
    }
    function body() { return JSON.stringify({ profile, application_id: applicationId, principal, base: original.base, source_text: source }); }
    function renderEditor() {
      guided.hidden = open.hidden = true;
      const toolbar = el("div", "org-draft-tools");
      const check = button(ownership ? "Validate ownership map" : "Validate organization", () => validate(), "button primary");
      const reset = button("Discard draft", () => { version++; controller?.abort(); pending = false; source = original.module.text; checked = original; clear(); mode = "source"; renderEditor(); status.textContent = "Draft reset to the captured source."; content.querySelector("textarea").focus(); });
      toolbar.append(el("span", "mono", original.module.path), check, reset);
      const layout = el("div", "org-draft-layout");
      const editor = el("textarea", "org-source-editor"); editor.value = source; editor.spellcheck = false; editor.setAttribute("aria-label", ownership ? "Ownership source" : "Organization Hale source");
      const sourcePane = append(el("details", "org-source-pane"), el("summary", "", ownership ? "Ownership source" : "Hale source"), el("p", "detail-note", "The complete module is preserved, including comments and handwritten code."), editor);
      sourcePane.open = mode === "source";
      const formPanel = append(el("details", "org-structured-panel"), el("summary", "", ownership ? "Edit ownership declarations" : "Edit declared structure"));
      formPanel.open = mode === "structured";
      const formHost = el("div"); formPanel.append(formHost);
      const review = el("section", "org-change-review"); review.setAttribute("aria-label", "Proposed source changes");
      const evidence = el("section", "org-draft-evidence"); evidence.setAttribute("aria-label", ownership ? "Ownership validation" : "Organization validation"); evidence.setAttribute("role", "region");
      layout.append(sourcePane, review); content.replaceChildren(toolbar, formPanel, layout, evidence);
      function renderForm() {
        if (!checked || checked.module.text !== source) {
          formHost.replaceChildren(el("p", "detail-note", ownership ? "Validate the current source to refresh ownership forms." : "Validate the current source to refresh the structured editor. Previous instance bindings are no longer used.")); return;
        }
        formHost.replaceChildren((ownership ? ownershipForm : structureForm)(source, checked, selected, id => { selected = id; renderForm(); formHost.querySelector("select")?.focus(); }, (candidate, nextSelection) => {
          selected = nextSelection; mode = "structured"; editor.value = candidate; update(candidate); void validate();
        }));
      }
      function update(candidate) {
        // Textareas normalize CRLF on assignment. Keep exact captured/form bytes
        // until a human edits text, and preserve a uniform CRLF source's style.
        source = typeof candidate === "string" ? candidate : original.module.text.includes("\r\n") && !original.module.text.replace(/\r\n/g, "").includes("\n") ? editor.value.replace(/\n/g, "\r\n") : editor.value;
        clear(); version++; controller?.abort(); pending = false;
        const tooLarge = bytes(source) > 16384 || bytes(body()) > 32768 || source.includes("\u0000") || source !== source.toWellFormed();
        check.disabled = tooLarge;
        status.textContent = tooLarge ? "The source must be valid Unicode within 16 KiB and the encoded request within 32 KiB." : source === original.module.text ? "Source matches the captured module. Validate to check the unchanged round trip." : "Unvalidated draft · check the complete organization before export.";
        review.replaceChildren(diff(original.module.text, source)); evidence.replaceChildren(); renderForm();
      }
      editor.addEventListener("input", update); update(source);
      async function validate() {
        if (pending || check.disabled) return;
        const token = ++version; clear(); pending = true; check.disabled = true; evidence.replaceChildren(); status.textContent = ownership ? "Checking ownership with the native DNA model…" : "Checking the captured organization with Hale…";
        try {
          const data = await request(body()); if (disposed || token !== version) return;
          result = checked = data; renderForm(); status.textContent = ownership ? "Native ownership validation passed. Review the candidate impact below." : "Native organization validation passed. The running application is unchanged.";
          if (ownership) evidence.append(ownershipImpact(data));
          else {
          const counts = impact(original.projection.items, data.projection.items);
          evidence.append(el("h3", "", "Checked chart changes"), el("p", "", counts.added.length + " added · " + counts.removed.length + " removed · " + counts.changed.length + " changed"));
          evidence.append(chart(original.projection.items, data.projection.items, counts, id => {
            selected = id; mode = "structured"; formPanel.open = true; renderForm();
            formPanel.scrollIntoView({ block: "start" }); formHost.querySelector("select")?.focus({ preventScroll: true });
          }), changeList(counts));
          evidence.append(el("p", "detail-note", "This impact covers declared instances and their contracts. Running obligations, retirement and effective authority require the owning services."));
          }
          if (!ownership && typeof onPropose === "function") evidence.append(publication(data));
          evidence.append(button(ownership ? "Download validated ownership map" : "Download validated Hale", () => { if (result === data) download(data.module.text, ownership ? "owners" : "main.hl", "text/plain;charset=utf-8"); }));
          evidence.append(button("Download validation evidence", () => { if (result === data) download(JSON.stringify(data, null, 2) + "\n", ownership ? "ownership-validation.json" : "organization-validation.json", "application/json"); }));
          const info = append(el("details"), el("summary", "", "Source basis and validation evidence"), el("pre", "org-evidence-json", JSON.stringify({ principal: data.principal, base: data.base, candidate_digest: data.module.digest, publication: data.publication }, null, 2)));
          evidence.append(info);
          if (root.contains(document.activeElement)) status.focus();
        } catch (error) {
          if (!disposed && token === version) { clear(); if ([401, 403, 409].includes(error.status)) invalidate(error); else status.textContent = error.message; }
        } finally { if (token === version) { pending = false; check.disabled = false; } }
      }
    }
    function publication(data) {
      const panel = el("section", "organization-publication"); panel.setAttribute("role", "region"); panel.setAttribute("aria-label", "Organization publication");
      const access = publicationAccess();
      panel.append(el("h3", "", "Propose this Organization change"));
      if (data.module.digest === data.base.module_digest) {
        panel.append(el("p", "detail-note", "The source is unchanged. Make and validate a change before preparing a proposal.")); return panel;
      }
      if (!access.allowed) { panel.append(el("p", "detail-note", access.reason)); return panel; }
      const content = el("div"); panel.append(content);
      let rationale = "", submitting = false;
      const current = () => !disposed && result === data && !pending && source === data.module.text;
      function edit() {
        const form = el("form"), input = el("textarea", "intervention-rationale");
        input.setAttribute("aria-label", "Rationale"); input.name = "rationale"; input.required = true; input.autocomplete = "off"; input.value = rationale;
        const count = el("p", "intervention-counter"), error = el("p", "intervention-error"); error.setAttribute("role", "alert"); error.hidden = true;
        const update = () => { rationale = input.value; count.textContent = bytes(rationale) + " / 2048 UTF-8 bytes"; input.setAttribute("aria-invalid", String(!validText(rationale, 2048))); };
        input.addEventListener("input", update); update();
        const review = el("button", "button", "Review publication"); review.type = "submit";
        form.append(append(el("label", "intervention-field"), el("span", "", "Rationale"), input), count, el("p", "field-hint", "This sends the complete checked Hale module for native verification and Review. The rationale stays in this page until you submit."), error, review);
        form.addEventListener("submit", event => {
          event.preventDefault();
          if (!current() || !publicationAccess().allowed || !validText(rationale, 2048) || bytes(rationale) === 0) {
            error.textContent = "Keep a nonempty rationale within 2048 UTF-8 bytes. The exact validation and publishing permission must still be available. Nothing was submitted."; error.hidden = false; return;
          }
          confirm();
        });
        content.replaceChildren(form);
      }
      function confirm() {
        const confirmation = el("div", "decision-confirmation"); confirmation.tabIndex = -1;
        confirmation.setAttribute("role", "group"); confirmation.setAttribute("aria-label", "Organization publication confirmation");
        const error = el("p", "intervention-error"); error.setAttribute("role", "alert"); error.hidden = true;
        const back = button("Back to rationale", () => { if (!submitting) { edit(); content.querySelector("textarea")?.focus(); } });
        const submit = button("Submit Organization proposal", async () => {
          if (submitting) return;
          if (!current() || !publicationAccess().allowed) { error.textContent = "The exact validation or publishing permission changed. Nothing was submitted."; error.hidden = false; return; }
          submitting = true; submit.disabled = back.disabled = true; confirmation.setAttribute("aria-busy", "true");
          try {
            const outcome = await onPropose(data, rationale);
            if (disposed || result !== data) return;
            if (!outcome?.submitted) { error.textContent = outcome?.error || "The request could not be reserved. Nothing was submitted."; error.hidden = false; }
          } catch {
            if (!disposed && result === data) { error.textContent = "The request could not be prepared. Check the saved request status before trying again."; error.hidden = false; }
          } finally {
            submitting = false;
            if (!disposed && result === data) { submit.disabled = back.disabled = false; confirmation.removeAttribute("aria-busy"); }
          }
        }, "button primary");
        confirmation.append(el("h4", "", "Submit the checked source"), el("p", "", "The exact source and chart comparison above are the candidate for this proposal."), el("p", "mono", "Source digest · " + data.module.digest), el("h4", "", "Rationale"), el("div", "document-text", rationale), el("p", "field-hint", "Submission creates a recoverable request. Native verification, independent Review, source application and the running result follow separately."), error, append(el("div", "intervention-actions"), submit, back));
        content.replaceChildren(confirmation); confirmation.focus();
      }
      edit(); return panel;
    }
    function diff(before, after) {
      const frame = append(el("div"), el("h3", "", "Source diff"));
      if (before === after) return append(frame, el("p", "detail-note", "No source changes."));
      const a = before.split("\n"), b = after.split("\n");
      let start = 0, end = 0;
      while (start < a.length && start < b.length && a[start] === b[start]) start++;
      while (end < a.length - start && end < b.length - start && a[a.length - end - 1] === b[b.length - end - 1]) end++;
      frame.append(el("p", "detail-note", "Replacement starts at line " + (start + 1) + ". Unchanged prefix and suffix are retained."));
      frame.append(el("pre", "org-diff-removed", a.slice(start, a.length - end).map(line => "− " + line).join("\n") || "No removed lines"));
      frame.append(el("pre", "org-diff-added", b.slice(start, b.length - end).map(line => "+ " + line).join("\n") || "No added lines"));
      return frame;
    }
    function impact(before, after) {
      const old = new Map(before.map(row => [row.id, row])), next = new Map(after.map(row => [row.id, row]));
      return { added: after.filter(row => !old.has(row.id)), removed: before.filter(row => !next.has(row.id)), changed: after.filter(row => old.has(row.id) && JSON.stringify(old.get(row.id)) !== JSON.stringify(row)) };
    }
    function chart(before, after, changes, editInstance) {
      const old = new Map(before.map(row => [row.id, row])), next = new Map(after.map(row => [row.id, row]));
      const union = new Map([...old, ...next]);
      const changed = new Set(changes.changed.map(row => row.id));
      const kind = id => !old.has(id) ? "added" : !next.has(id) ? "removed" : changed.has(id) ? "changed" : "unchanged";
      const labels = { added: "Added", removed: "Removed", changed: "Changed", unchanged: "Unchanged" };
      const children = new Map();
      for (const row of union.values()) {
        const parent = union.has(row.parent_id) ? row.parent_id : "";
        if (!children.has(parent)) children.set(parent, []);
        children.get(parent).push(row.id);
      }
      for (const siblings of children.values()) siblings.sort((a, b) => a.localeCompare(b));
      // The native projections are complete and individually checked. The union
      // is a presentation layout only; every drawn edge comes from a projection.
      const ordered = [], visited = new Set();
      const stack = (children.get("") || []).slice().reverse().map(id => [id, 0]);
      while (stack.length) {
        const [id, depth] = stack.pop();
        if (visited.has(id)) continue;
        visited.add(id); ordered.push({ id, depth });
        stack.push(...(children.get(id) || []).slice().reverse().map(child => [child, depth + 1]));
      }
      for (const id of union.keys()) if (!visited.has(id)) ordered.push({ id, depth: 0 });
      const descends = (id, ancestor, map) => {
        const seen = new Set();
        while (id && map.has(id) && !seen.has(id)) {
          if (id === ancestor) return true;
          seen.add(id); id = map.get(id).parent_id;
        }
        return false;
      };
      let plane = "compare", scope = "", renderedScope = null, drawVersion = 0, inspected = union.has(selected) ? selected : ordered[0]?.id || "";
      const frame = el("section", "org-impact"); frame.setAttribute("aria-label", "Checked candidate chart"); frame.setAttribute("role", "region");
      const controls = el("div", "org-impact-tools");
      const planeControls = el("div", "org-impact-planes"); planeControls.setAttribute("role", "group"); planeControls.setAttribute("aria-label", "Source comparison");
      const note = el("p", "detail-note", "Validated source preview · the running organization is unchanged. Renamed paths appear as removed and added identities.");
      const breadcrumb = el("nav", "org-impact-path"); breadcrumb.setAttribute("aria-label", "Impact branch");
      const count = el("p", "org-impact-count");
      const layout = el("div", "org-impact-layout");
      const viewport = el("div", "org-impact-viewport"); viewport.tabIndex = 0; viewport.setAttribute("role", "group"); viewport.setAttribute("aria-label", "Source impact canvas. Scroll to explore containment.");
      const stage = el("div", "org-impact-stage"); viewport.append(stage);
      const inspector = el("div", "org-impact-inspector"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected instance impact");
      append(layout, viewport, inspector); append(controls, planeControls, count); append(frame, controls, note, breadcrumb, layout);
      const planeButtons = new Map();
      for (const [value, label] of [["compare", "Compare changes"], ["before", "Current source"], ["after", "Checked candidate"]]) {
        const control = button(label, () => { plane = value; draw(); }, "scope-button");
        planeButtons.set(value, control); planeControls.append(control);
      }
      function visible(id) { return plane === "compare" || (plane === "before" ? old : next).has(id); }
      function inspect(id) {
        inspected = id;
        for (const control of stage.querySelectorAll("button[data-instance-id]")) control.setAttribute("aria-pressed", String(control.dataset.instanceId === id));
        inspector.replaceChildren();
        if (!union.has(id)) { inspector.append(el("p", "detail-note", "Select an instance to inspect the exact source impact.")); return; }
        const row = union.get(id);
        append(inspector, el("span", "eyebrow", labels[kind(id)]), el("h4", "", row.name || id.split(".").at(-1)), el("p", "mono org-impact-identity", id));
        if (!visible(id)) inspector.append(el("p", "detail-note", "This instance is absent from the selected source view. Its comparison remains here for reference."));
        const table = el("table", "declaration-table");
        table.append(append(el("thead"), append(el("tr"), el("th", "", "Property"), el("th", "", "Current source"), el("th", "", "Checked candidate"))));
        const body = el("tbody");
        for (const [field, label] of [["parent_id", "Within"], ["declaration", "Declaration"], ["role", "Role"]]) {
          const value = item => !item ? "Absent" : item[field] || (field === "parent_id" ? "Root" : "Not declared");
          body.append(append(el("tr"), el("th", "", label), el("td", "", value(old.get(id))), el("td", "", value(next.get(id)))));
        }
        table.append(body); inspector.append(table);
        const descendants = [...union.keys()].filter(other => other !== id && (descends(other, id, old) || descends(other, id, next)));
        if (descendants.length) {
          inspector.append(el("p", "detail-note", descendants.length + " declared descendants · " + descendants.filter(other => kind(other) !== "unchanged").length + " affected by this source change"));
          const enter = button("Enter this branch", () => { scope = id; draw(); breadcrumb.querySelector("button[aria-current]")?.focus(); });
          enter.disabled = scope === id; inspector.append(enter);
        }
        const edit = button("Edit candidate instance", () => editInstance(id)); edit.disabled = !next.has(id); inspector.append(edit);
        if (!next.has(id)) inspector.append(el("p", "detail-note", "Removed from the source candidate. Live retirement and work reassignment are not established."));
        const evidence = append(el("details"), el("summary", "", "Exact returned instance evidence"), el("pre", "org-evidence-json", JSON.stringify({ current: old.get(id) || null, candidate: next.get(id) || null }, null, 2)));
        inspector.append(evidence);
      }
      function draw() {
        const drawing = ++drawVersion;
        for (const [value, control] of planeButtons) control.setAttribute("aria-pressed", String(value === plane));
        breadcrumb.replaceChildren();
        const root = button("Whole organization", () => { scope = ""; draw(); breadcrumb.querySelector("button")?.focus(); }, "text-link");
        if (!scope) root.setAttribute("aria-current", "location"); breadcrumb.append(root);
        if (scope) {
          const ancestors = [], seen = new Set(); let id = scope;
          while (id && union.has(id) && !seen.has(id)) { seen.add(id); ancestors.unshift(id); id = union.get(id).parent_id; }
          for (const ancestor of ancestors) {
            const item = button(ancestor, () => { scope = ancestor; draw(); breadcrumb.querySelector("button[aria-current]")?.focus(); }, "text-link");
            if (ancestor === scope) item.setAttribute("aria-current", "location"); breadcrumb.append(el("span", "", "/"), item);
          }
        }
        const rows = ordered.filter(row => !scope || descends(row.id, scope, old) || descends(row.id, scope, next));
        const minimum = rows.length ? Math.min(...rows.map(row => row.depth)) : 0;
        // Union coordinates remain identical while switching source planes, so
        // a removed branch leaves a gap instead of shifting every other node.
        const included = new Set(rows.map(row => row.id)), positions = new Map();
        let leaf = 0;
        for (const row of rows) if (!(children.get(row.id) || []).some(id => included.has(id))) {
          positions.set(row.id, { x: 28 + (row.depth - minimum) * 234, y: 24 + leaf++ * 112 });
        }
        for (const row of rows.slice().reverse()) if (!positions.has(row.id)) {
          const nested = (children.get(row.id) || []).map(id => positions.get(id)).filter(Boolean);
          const y = nested.length ? (Math.min(...nested.map(p => p.y)) + Math.max(...nested.map(p => p.y))) / 2 : 24 + leaf++ * 112;
          positions.set(row.id, { x: 28 + (row.depth - minimum) * 234, y });
        }
        const width = Math.max(280, ...[...positions.values()].map(p => p.x + 224));
        const height = Math.max(180, ...[...positions.values()].map(p => p.y + 110));
        const scopeChanged = scope !== renderedScope;
        const previousScroll = [viewport.scrollLeft, viewport.scrollTop]; renderedScope = scope;
        stage.replaceChildren(); stage.style.width = width + "px"; stage.style.height = height + "px";
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg"); svg.setAttribute("width", width); svg.setAttribute("height", height); svg.setAttribute("aria-hidden", "true"); svg.classList.add("org-impact-edges");
        const oldEdges = new Set(before.filter(row => old.has(row.parent_id)).map(row => JSON.stringify([row.parent_id, row.id])));
        const nextEdges = new Set(after.filter(row => next.has(row.parent_id)).map(row => JSON.stringify([row.parent_id, row.id])));
        for (const edge of new Set([...oldEdges, ...nextEdges])) {
          if (plane === "before" && !oldEdges.has(edge) || plane === "after" && !nextEdges.has(edge)) continue;
          const [from, to] = JSON.parse(edge), a = positions.get(from), b = positions.get(to);
          if (!a || !b) continue;
          const path = document.createElementNS(svg.namespaceURI, "path");
          const x = a.x + 194, middle = x + 20;
          path.setAttribute("d", `M ${x} ${a.y + 36} H ${middle} V ${b.y + 36} H ${b.x}`);
          path.dataset.edgeFrom = from; path.dataset.edgeTo = to;
          path.dataset.change = !oldEdges.has(edge) ? "added" : !nextEdges.has(edge) ? "removed" : "unchanged";
          svg.append(path);
        }
        stage.append(svg);
        for (const { id } of rows) {
          if (!visible(id)) continue;
          const row = (plane === "before" ? old : next).get(id) || union.get(id), p = positions.get(id);
          const control = button("", () => inspect(id), "org-impact-node");
          control.dataset.instanceId = id; control.dataset.change = kind(id); control.setAttribute("aria-label", id);
          control.style.left = p.x + "px"; control.style.top = p.y + "px";
          append(control, el("span", "org-impact-node-state", labels[kind(id)]), el("strong", "", row.name || id.split(".").at(-1)), el("span", "", row.declaration));
          control.title = id + (row.parent_id ? " · within " + row.parent_id : " · root"); stage.append(control);
        }
        count.textContent = rows.filter(row => visible(row.id)).length + " instances · " + (scope || "whole organization") + " · " + (plane === "compare" ? "source changes" : plane === "before" ? "current source" : "checked candidate");
        if (scope && !rows.some(row => row.id === inspected)) inspected = scope;
        inspect(inspected);
        if (scopeChanged) {
          const point = positions.get(scope || inspected) || { x: 0, y: 0 };
          // The first draw occurs before insertion. Reveal the focused branch
          // once layout exists; a later redraw must not inherit its old callback.
          requestAnimationFrame(() => {
            if (disposed || !stage.isConnected || drawing !== drawVersion) return;
            viewport.scrollLeft = Math.max(0, point.x - 28);
            viewport.scrollTop = Math.max(0, point.y - viewport.clientHeight / 2 + 39);
          });
        } else { viewport.scrollLeft = previousScroll[0]; viewport.scrollTop = previousScroll[1]; }
      }
      draw(); return frame;
    }
    function changeList(changes) {
      const list = el("ul", "org-draft-impact");
      for (const [kind, rows] of Object.entries(changes)) for (const row of rows) list.append(el("li", "", kind + " · " + row.id + (row.parent_id ? " · within " + row.parent_id : "")));
      if (!list.childElementCount) list.append(el("li", "", "The declared chart and contracts are unchanged. Source-only changes remain visible in the diff."));
      return list;
    }
    return { publicationMatches(data) { return !ownership && !disposed && !pending && result === data && data.validation === "valid_draft" && source === data.module.text; }, edit(id) { selected = id; if (!original) void load("structured"); else { mode = "structured"; renderEditor(); content.querySelector("select")?.focus(); } root.scrollIntoView({ block: "start" }); }, destroy() { disposed = true; version++; controller?.abort(); revoke(); source = ""; original = checked = result = null; host.replaceChildren(); } };
  }
  const SOURCE_SUMMARY = ["added", "removed", "renamed", "moved", "split", "joined", "ambiguous", "contract_deltas", "effect_deltas", "certificate_deltas", "law_deltas"];
  const SOURCE_CHANGES = ["persisted", "added", "removed", "renamed", "moved", "split", "joined", "ambiguous"];
  const validText = (text, max = 4096) => typeof text === "string" && bytes(text) <= max && !text.includes("\u0000") && !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(text);
  const sourceHash = value => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value);
  const commitId = value => typeof value === "string" && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value);
  async function sourceCandidate(data, review, applicationId) {
    const check = condition => { if (!condition) throw new Error("The exact Organization candidate or its evidence could not be verified."); };
    check(closed(data, ["kind", "review_id", "candidate_commit", "document"]) && data.kind === "organization_source_change" && data.review_id === review.id && data.candidate_commit === review.subject_digest && commitId(data.candidate_commit) && validText(data.document, 1048576));
    const c = JSON.parse(data.document), ownership = c.format === "dna.organization-ownership-candidate/1";
    check(closed(c, ["format", "application_id", "command_id", "mutation_id", "base", "module", "candidate", "changed_files", "verification", "review", ...(ownership ? ["ownership_impact"] : [])]) && (ownership || c.format === "dna.organization-source-candidate/1") && c.application_id === applicationId && c.command_id === review.organization_source_request_id && c.mutation_id === review.id);
    check(closed(c.base, BASE) && commitId(c.base.source_head) && sourceHash(c.base.module_digest) && validText(c.base.record_head, 256) && validText(c.base.dependency_source, 256) && sourceHash(c.base.dependency_digest));
    check(closed(c.module, ["path", "base_text", "text", "digest"]) && c.module.path === (ownership ? "dna/org/owners" : "dna/org/main.hl") && validText(c.module.base_text, 16384) && validText(c.module.text, 16384) && sourceHash(c.module.digest) && c.module.digest === review.organization_source_digest);
    check(closed(c.candidate, ["commit", "parent", "shape"]) && c.candidate.commit === data.candidate_commit && c.candidate.parent === c.base.source_head && validText(c.candidate.shape, 256));
    check(Array.isArray(c.changed_files) && c.changed_files.length <= 1 && c.changed_files.every(path => path === c.module.path));
    check(closed(c.verification, ["evidence", "semantic_diff"]) && /^fmt=-?\d+ check=-?\d+ verify=-?\d+ test=-?\d+ diff=-?\d+ rollback=-?\d+ fleet=-?\d+$/.test(c.verification.evidence));
    const diff = c.verification.semantic_diff;
    check(closed(diff, ["receipt_digest", "document", "digest"]) && sourceHash(diff.receipt_digest) && sourceHash(diff.digest) && validText(diff.document, 262144));
    const semantic = JSON.parse(diff.document);
    check(closed(semantic, ["format", "coverage", "classification", "base_shape", "candidate_shape", "summary", "declarations"]) && semantic.format === "dna.organization-semantic-diff/1" && semantic.coverage === "classification_summary_declarations" && ["identical", "source-only", "model-shape", "contract"].includes(semantic.classification));
    check(validText(semantic.base_shape, 256) && semantic.candidate_shape === c.candidate.shape && closed(semantic.summary, SOURCE_SUMMARY) && SOURCE_SUMMARY.every(key => Number.isSafeInteger(semantic.summary[key]) && semantic.summary[key] >= 0));
    check(Array.isArray(semantic.declarations) && semantic.declarations.length <= 4096 && semantic.declarations.every(row => closed(row, ["change", "kind", "name"]) && SOURCE_CHANGES.includes(row.change) && validText(row.kind, 256) && validText(row.name, 4096)));
    check(closed(c.review, ["required_authority", "approvers", "quorum"]) && c.review.required_authority === review.required_authority && c.review.approvers === review.approvers && closed(c.review.quorum, ["state", "required_owners", "approved_owners"]) && ["available", "unavailable"].includes(c.review.quorum.state) && validText(c.review.quorum.required_owners, 8192) && validText(c.review.quorum.approved_owners, 8192));
    if (ownership) {
      const p = c.ownership_impact, name = x => validText(x, 256) && !/[\s#,:=]/u.test(x);
      check(closed(p, ["host_changed", "mode_changed", "live_obligations", "affected_owners", "scopes"]) && typeof p.host_changed === "boolean" && typeof p.mode_changed === "boolean" && p.live_obligations === "unavailable");
      check(Array.isArray(p.affected_owners) && p.affected_owners.length <= 512 && p.affected_owners.every(row => closed(row, ["owner", "members"]) && name(row.owner) && row.owner && Array.isArray(row.members) && row.members.length <= 8192 && row.members.every(member => name(member) && member)));
      check(new Set(p.affected_owners.map(row => row.owner)).size === p.affected_owners.length && p.affected_owners.map(row => row.owner + "=" + row.members.join(",")).join(" ") === c.review.approvers);
      check(Array.isArray(p.scopes) && p.scopes.length <= 512 && p.scopes.every(row => closed(row, ["position", "before_owner", "after_owner"]) && name(row.position) && row.position && name(row.before_owner) && name(row.after_owner)));
      check(new Set(p.scopes.map(row => row.position)).size === p.scopes.length);
    }
    const hashes = await Promise.all([digest(c.module.base_text), digest(c.module.text), digest(diff.document)]);
    check(hashes[0] === c.base.module_digest && hashes[1] === c.module.digest && hashes[2] === diff.digest);
    check((c.module.base_text === c.module.text) === (c.changed_files.length === 0));
    return { ...data, text: data.document, organization: c, semantic };
  }
  function sourceComparison(c) {
    const frame = el("section", "source-comparison"); frame.setAttribute("aria-label", "Exact Organization source comparison");
    const tools = el("div", "source-comparison-tools"), content = el("div", "source-comparison-columns");
    let mode = "changes";
    const before = c.module.base_text.split("\n"), after = c.module.text.split("\n");
    let start = 0, tail = 0;
    while (start < before.length && start < after.length && before[start] === after[start]) start++;
    while (tail < before.length - start && tail < after.length - start && before.at(-tail - 1) === after.at(-tail - 1)) tail++;
    const draw = () => {
      for (const control of tools.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.mode === mode));
      content.replaceChildren();
      for (const [title, lines, side] of [["Captured base", before, "before"], ["Exact candidate", after, "after"]]) {
        const pane = append(el("section", "source-comparison-pane " + side), el("h5", "", title));
        const code = el("pre", "source-comparison-code"); code.tabIndex = 0; code.setAttribute("aria-label", title + " source");
        const low = mode === "changes" ? Math.max(0, start - 2) : 0, high = mode === "changes" ? Math.min(lines.length, lines.length - tail + 2) : lines.length;
        if (c.module.base_text === c.module.text && mode === "changes") code.append(el("span", "detail-note", "Source bytes are unchanged."));
        else for (let index = low; index < high; index++) {
          const line = el("span", "source-comparison-line" + (index >= start && index < lines.length - tail ? " changed" : ""));
          const number = el("span", "source-line-number", String(index + 1)); number.setAttribute("aria-hidden", "true");
          line.append(number, el("code", "", lines[index])); code.append(line);
        }
        pane.append(code); content.append(pane);
      }
    };
    for (const [value, label] of [["changes", "Changed region"], ["all", "Full source"]]) {
      const control = button(label, () => { mode = value; draw(); }); control.dataset.mode = value; tools.append(control);
    }
    frame.append(tools, content, el("p", "detail-note", "Exact retained module bytes. The changed region includes two context lines; it is a source comparison, not a live organization view.")); draw(); return frame;
  }
  function ownershipReviewMap(c) {
    const impact = c.ownership_impact, frame = el("section", "source-change-map"); frame.setAttribute("aria-label", "Ownership scope transfers");
    const controls = el("div", "source-map-controls"), canvas = el("div", "source-map-canvas"), inspector = el("div", "source-map-inspector"); inspector.setAttribute("aria-live", "polite");
    let all = false, selected = "";
    const owner = value => value || "Unowned";
    const inspect = row => {
      selected = row.position;
      for (const control of canvas.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.scope === selected));
      inspector.replaceChildren(el("span", "eyebrow", "OWNERSHIP SCOPE"), el("h5", "", row.position));
      const journey = el("div", "ownership-transfer-journey");
      for (const [label, value] of [["Captured owner", row.before_owner], ["Proposed owner", row.after_owner]]) {
        const stop = append(el("div", "ownership-transfer-stop"), el("span", "eyebrow", label), el("strong", "", owner(value)));
        const voters = impact.affected_owners.find(item => item.owner === value);
        stop.append(el("span", "detail-note", voters ? voters.members.length ? "Review members · " + voters.members.join(", ") : "No eligible Review members declared" : "No approval required from this owner by the captured map comparison"));
        journey.append(stop);
      }
      inspector.append(journey, el("p", "detail-note", "Approval records agreement to this map. Work transfer and activation remain pending separate operational checks."));
    };
    const draw = () => {
      for (const control of controls.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.all === String(all)));
      canvas.replaceChildren(append(el("div", "source-map-center"), el("span", "source-map-orbit", "◈"), el("strong", "", "Ownership"), el("span", "", impact.affected_owners.length + " affected owners")));
      const lanes = el("div", "source-map-lanes"), rows = impact.scopes.filter(row => all || row.before_owner !== row.after_owner);
      for (const row of rows) {
        const control = button("", () => inspect(row), "source-declaration"); control.dataset.scope = row.position; control.setAttribute("aria-label", "Inspect ownership scope " + row.position);
        control.append(el("span", "source-declaration-dot"), el("strong", "", row.position), el("small", "", owner(row.before_owner) + " → " + owner(row.after_owner))); lanes.append(control);
      }
      canvas.append(lanes);
      if (rows.length) inspect(rows.find(row => row.position === selected) || rows[0]);
      else inspector.replaceChildren(el("p", "detail-note", "No scope changes owner. Membership or hosting changes can still require approval; inspect the captured Review and exact map below."));
    };
    for (const [value, label] of [[false, "Changing scopes"], [true, "All scopes"]]) { const control = button(label, () => { all = value; draw(); }); control.dataset.all = String(value); controls.append(control); }
    frame.append(controls, canvas, inspector);
    if (impact.host_changed) frame.append(el("p", "detail-note", "The hosting party changes in this proposal."));
    if (impact.mode_changed) frame.append(el("p", "detail-note", "The ownership mode changes in this proposal."));
    frame.append(el("p", "detail-note", "Captured ownership-map comparison. Live obligations and running occupants are not established by this evidence."));
    draw(); return frame;
  }
  function sourceReview(candidate) {
    const c = candidate.organization, semantic = candidate.semantic, ownership = c.format === "dna.organization-ownership-candidate/1";
    const frame = el("section", "organization-source-review"); frame.setAttribute("role", "region"); frame.setAttribute("aria-label", "Organization candidate comparison");
    const classification = ownership ? "Ownership responsibilities change" : { identical: "Artifact unchanged", "source-only": "Shape preserved · source updated", "model-shape": "Organization shape changes", contract: "Contract changes" }[semantic.classification];
    const heading = append(el("header", "source-review-heading"), el("p", "eyebrow accent", "ORGANIZATION / EXACT CANDIDATE"), el("h4", "", classification), el("p", "detail-note", "Inspect the change, then follow its Review. Approval, source application and a running process are separate results."));
    const checks = el("div", "source-verification-strip"); checks.setAttribute("aria-label", "Native verification results");
    for (const entry of c.verification.evidence.split(" ")) {
      const [name, code] = entry.split("="), item = el("span", "source-verification-result"); item.dataset.result = code === "0" ? "passed" : "finding";
      item.append(el("span", "", name), el("strong", "", code === "0" ? "Passed" : "Code " + code)); checks.append(item);
    }
    let changes = el("section", "source-change-map"); changes.setAttribute("aria-label", "Semantic declaration changes");
    const controls = el("div", "source-map-controls"), diagram = el("div", "source-map-canvas"), inspector = el("div", "source-map-inspector"); inspector.setAttribute("aria-live", "polite");
    let filter = "changed", selected = -1;
    const detail = index => {
      selected = index; const row = semantic.declarations[index];
      for (const control of diagram.querySelectorAll("button")) control.setAttribute("aria-pressed", String(Number(control.dataset.index) === index));
      inspector.replaceChildren(el("span", "eyebrow", row.kind + " · " + row.change), el("h5", "", row.name), el("p", "detail-note", row.change === "persisted" ? "The native semantic diff retains this declaration. Its source may still change." : "This declaration is identified by the native semantic diff. Compare the captured source below for its implementation."));
    };
    const draw = () => {
      for (const control of controls.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.filter === filter));
      diagram.replaceChildren();
      const visible = semantic.declarations.map((row, index) => ({ row, index })).filter(({ row }) => filter === "all" || row.change !== "persisted");
      const selectedRows = visible.slice(0, 256);
      const center = append(el("div", "source-map-center"), el("span", "source-map-orbit", "◈"), el("strong", "", "dna/org"), el("span", "", visible.length + " declarations")); diagram.append(center);
      const lanes = el("div", "source-map-lanes");
      for (const [label, kinds] of [["Entering", ["added"]], ["Within the candidate", ["persisted", "renamed", "moved", "split", "joined", "ambiguous"]], ["Leaving", ["removed"]]]) {
        const rows = selectedRows.filter(({ row }) => kinds.includes(row.change)); if (!rows.length) continue;
        const lane = append(el("div", "source-map-lane"), el("p", "eyebrow", label));
        for (const { row, index } of rows) {
          const control = button("", () => detail(index), "source-declaration"); control.dataset.index = String(index); control.dataset.change = row.change;
          control.setAttribute("aria-label", "Inspect " + row.kind + " " + row.name); control.setAttribute("aria-pressed", String(index === selected));
          control.append(el("span", "source-declaration-dot"), el("strong", "", row.name), el("small", "", row.kind + " · " + row.change)); lane.append(control);
        }
        lanes.append(lane);
      }
      diagram.append(lanes);
      if (!visible.length) diagram.append(el("p", "source-map-empty", "No declaration identities changed. Explore retained declarations or compare the exact source below."));
      if (visible.length > 256) diagram.append(el("p", "detail-note", "Showing the first 256 of " + visible.length + " declarations. Complete retained evidence is below."));
      if (selectedRows.length) detail(selectedRows.some(item => item.index === selected) ? selected : selectedRows[0].index);
      else inspector.replaceChildren(el("p", "detail-note", "The semantic classification and source comparison remain available even when no declaration identities change."));
    };
    for (const [value, label] of [["changed", "Changed declarations"], ["all", "All declarations"]]) {
      const control = button(label, () => { filter = value; draw(); }); control.dataset.filter = value; controls.append(control);
    }
    const summary = el("div", "source-semantic-summary");
    for (const [key, label] of [["contract_deltas", "contract"], ["effect_deltas", "effect"], ["law_deltas", "law"], ["certificate_deltas", "certificate"]]) summary.append(el("span", "", semantic.summary[key] + " " + label + " deltas"));
    changes.append(controls, diagram, inspector, summary, el("p", "detail-note", "This view covers native classification, summary counts and declaration identities. Live obligations and ownership transfer require separate impact evidence.")); draw();
    if (ownership) changes = ownershipReviewMap(c);
    const quorum = el("section", "source-review-quorum"); quorum.setAttribute("aria-label", "Captured Review quorum");
    const q = c.review.quorum, owners = q.required_owners.split(" ").filter(Boolean), approved = new Set(q.approved_owners.split(" ").filter(Boolean));
    quorum.append(el("h5", "", "Review authority · " + c.review.required_authority));
    if (q.state === "unavailable") quorum.append(el("p", "detail-note", "The current quorum evidence is unavailable."));
    else if (!owners.length) quorum.append(el("p", "detail-note", "One independent authorized decision is required."));
    else for (const owner of owners) { const chip = el("span", "source-owner-vote", owner + " · " + (approved.has(owner) ? "Approved" : "Awaiting approval")); chip.dataset.approved = String(approved.has(owner)); quorum.append(chip); }
    const evidence = append(el("details", "source-review-evidence"), el("summary", "", "Exact candidate and semantic evidence"), el("p", "mono", "Candidate commit · " + c.candidate.commit), el("p", "mono", "Module digest · " + c.module.digest), el("pre", "", JSON.stringify(c, null, 2)));
    return append(frame, heading, checks, changes, sourceComparison(c), quorum, evidence);
  }
  // Read evidence independently of command receipts. An authorized reviewer
  // needs only the exact Review identity, never the proposer's recovery key.
  function sourceStatus(data, review, candidate = null) {
    const check = condition => { if (!condition) throw new Error("The Organization change status does not match its exact Review and evidence."); };
    const empty = (value, keys) => keys.every(key => value[key] === "");
    const decimal = value => typeof value === "string" && /^(0|[1-9][0-9]{0,18})$/.test(value);
    const attempt = value => typeof value === "string" && /^organization-launch:[a-f0-9]{64}$/.test(value);
    const shape = value => typeof value === "string" && /^[a-f0-9]{16}$/.test(value);
    check(closed(data, ["profile", "review_id", "mutation_id", "source", "review", "application", "restart_handoff", "launch", "observation", "exit", "rollback", "current_running"]) && data.profile === "dna.organization.source-status.v1");
    check(review?.organization_source === true && data.review_id === review.id && data.mutation_id === review.id);
    const s = data.source, r = data.review, a = data.application, h = data.restart_handoff, l = data.launch, o = data.observation, x = data.exit, b = data.rollback, c = data.current_running;
    check(closed(s, ["base_commit", "module_digest", "candidate_commit", "source_digest"]) && commitId(s.base_commit) && commitId(s.candidate_commit) && sourceHash(s.module_digest) && sourceHash(s.source_digest));
    check(s.candidate_commit === review.subject_digest);
    // A bounded source comparison can be unavailable while this separately
    // authorized status read still proves the candidate's source identity.
    if (review.organization_source_digest) check(s.source_digest === review.organization_source_digest);
    if (candidate) check(candidate.organization?.base.source_head === s.base_commit && candidate.organization.base.module_digest === s.module_digest && candidate.candidate_commit === s.candidate_commit);
    check(closed(r, ["state", "outcome", "required_owners", "approved_owners"]) && ["unavailable", "pending", "settled"].includes(r.state) && validText(r.required_owners, 8192) && validText(r.approved_owners, 8192));
    check(r.state === "settled" ? ["approve", "reject", "revise"].includes(r.outcome) : r.outcome === "");
    check(r.state === "unavailable" || r.state === review.state && r.outcome === review.outcome);
    if (r.state === "unavailable") check(r.required_owners === "" && r.approved_owners === "");
    const owners = r.required_owners.split(" ").filter(Boolean), approved = r.approved_owners.split(" ").filter(Boolean);
    check(new Set(owners).size === owners.length && new Set(approved).size === approved.length && approved.every(owner => owners.includes(owner)));
    check(closed(a, ["state", "reason_code", "event_id"]) && ["pending", "applied", "refused", "declined", "failed", "unknown"].includes(a.state));
    const refusals = ["invalid_command", "forbidden", "stale_subject", "organization_worktree_invalid", "organization_candidate_invalid", "organization_snapshot_unsupported", "organization_deployment_unsupported"];
    check(a.state === "refused" ? refusals.includes(a.reason_code) : a.state === "failed" ? a.reason_code === "native_apply_failed" : a.reason_code === "");
    check(a.state === "applied" ? commitId(a.event_id) : a.event_id === "");
    check(!["applied", "refused", "failed"].includes(a.state) || r.state === "settled" && r.outcome === "approve");
    check(a.state !== "declined" || r.state === "settled" && ["reject", "revise"].includes(r.outcome));
    check(closed(h, ["state", "event_id"]) && ["pending", "requested", "unknown"].includes(h.state));
    check(h.state === "requested" ? a.state === "applied" && commitId(h.event_id) : h.event_id === "");
    const launchKeys = ["attempt_id", "request_event_id", "event_id", "binary_digest", "topology_digest", "topology_shape"];
    check(closed(l, ["state", ...launchKeys]) && ["unavailable", "requested", "launched", "unknown"].includes(l.state));
    if (["requested", "launched"].includes(l.state)) {
      check(h.state === "requested" && attempt(l.attempt_id) && commitId(l.request_event_id) && sourceHash(l.binary_digest) && sourceHash(l.topology_digest) && shape(l.topology_shape));
      check(l.state === "launched" ? commitId(l.event_id) : l.event_id === "");
    } else check(empty(l, launchKeys));
    check(closed(o, ["state", "event_id", "window_started", "window_ended"]) && ["unavailable", "healthy", "crashed", "unknown"].includes(o.state));
    if (["healthy", "crashed"].includes(o.state)) check(l.state === "launched" && commitId(o.event_id) && decimal(o.window_started) && decimal(o.window_ended) && BigInt(o.window_ended) >= BigInt(o.window_started));
    else check(empty(o, ["event_id", "window_started", "window_ended"]));
    check(closed(x, ["state", "event_id", "code"]) && ["unavailable", "exited", "unknown"].includes(x.state));
    if (x.state === "exited") check(h.state === "requested" && commitId(x.event_id) && decimal(x.code) && BigInt(x.code) <= 255n);
    else check(empty(x, ["event_id", "code"]));
    const rollbackKeys = ["request_event_id", "event_id", "base_commit"], baseLaunchKeys = ["launch_request_event_id", "launch_event_id", "binary_digest", "topology_digest"];
    check(closed(b, ["state", ...rollbackKeys, "launch_state", ...baseLaunchKeys]) && ["unavailable", "requested", "applied", "unknown"].includes(b.state) && ["unavailable", "requested", "launched", "unknown"].includes(b.launch_state));
    if (["requested", "applied"].includes(b.state)) {
      check(x.state === "exited" && commitId(b.request_event_id));
      check(b.state === "applied" ? commitId(b.event_id) && b.base_commit === s.base_commit : b.event_id === "" && b.base_commit === "");
    } else check(empty(b, rollbackKeys));
    if (["requested", "launched"].includes(b.launch_state)) {
      check(b.state === "applied" && commitId(b.launch_request_event_id) && sourceHash(b.binary_digest) && sourceHash(b.topology_digest));
      check(b.launch_state === "launched" ? commitId(b.launch_event_id) : b.launch_event_id === "");
    } else check(empty(b, baseLaunchKeys));
    const runtimeKeys = ["attempt_id", "launch_event_id", "candidate_commit", "binary_digest", "topology_digest", "topology_shape", "process_key"];
    check(closed(c, ["available", "profile", ...runtimeKeys]) && typeof c.available === "boolean" && c.profile === "dna.organization-runtime/1");
    if (c.available) check(l.state === "launched" && x.state === "unavailable" && b.state === "unavailable" && o.state !== "crashed" && c.attempt_id === l.attempt_id && c.launch_event_id === l.event_id && c.candidate_commit === s.candidate_commit && c.binary_digest === l.binary_digest && c.topology_digest === l.topology_digest && c.topology_shape === l.topology_shape && typeof c.process_key === "string" && /^[a-f0-9]{64}$/.test(c.process_key));
    else check(empty(c, runtimeKeys));
    return data;
  }
  function statusJourney(data, { recordHead, inspectedAt, onRefresh, runtimeHref } = {}) {
    const frame = el("section", "organization-status"); frame.setAttribute("role", "region"); frame.setAttribute("aria-label", "Organization change status");
    const heading = append(el("header", "source-review-heading"), el("p", "eyebrow accent", "ORGANIZATION / CHANGE JOURNEY"), el("h3", "", "From Review to running"));
    heading.append(el("p", "detail-note", "Follow this exact candidate through its decision, source change and host. Select a stage to inspect its evidence."));
    const a = data.application, h = data.restart_handoff, l = data.launch, o = data.observation, x = data.exit, b = data.rollback, c = data.current_running;
    const tone = value => ["applied", "requested", "launched", "healthy"].includes(value) ? "confirmed" : ["refused", "declined", "failed", "crashed", "exited"].includes(value) ? "refused" : ["unavailable", "unknown"].includes(value) ? "unknown" : "pending";
    const outcome = { approve: "Approved", reject: "Rejected", revise: "Revision requested" };
    const stages = [
      { key: "review", title: "Review", value: outcome[data.review.outcome] || (data.review.state === "pending" ? "Awaiting decision" : "Not established"), tone: data.review.state === "settled" ? data.review.outcome === "approve" ? "confirmed" : "refused" : tone(data.review.state), text: "The decision belongs to this exact candidate. Source application is a separate result.", facts: [["Required owners", data.review.required_owners || "No owner quorum listed"], ["Approved owners", data.review.approved_owners || "None listed"]] },
      { key: "application", title: "Source", value: a.state, tone: tone(a.state), text: a.state === "applied" ? "This candidate was applied to source. That retained fact does not establish which version is running now." : "The native application result is separate from approval. An unknown effect must be recovered before assuming it succeeded.", facts: [["Application event", a.event_id], ["Result reason", a.reason_code]] },
      { key: "handoff", title: "Handoff", value: h.state, tone: tone(h.state), text: "The owning host was asked to restart only when this handoff is recorded. A request does not prove that a process started.", facts: [["Handoff event", h.event_id]] },
      { key: "launch", title: "Launch", value: l.state, tone: tone(l.state), text: "A launch acknowledgment identifies the exact attempt and built artifacts. An unacknowledged claim can remain unknown; refreshing reads its evidence without starting another process.", facts: [["Attempt", l.attempt_id], ["Launch request", l.request_event_id], ["Launch event", l.event_id], ["Binary digest", l.binary_digest], ["Topology digest", l.topology_digest]] },
      { key: "observation", title: "Observation", value: o.state === "healthy" ? "Healthy during window" : o.state, tone: tone(o.state), text: "This is a completed observation window for the exact launch. It remains part of history after a process exits.", facts: [["Observation event", o.event_id], ["Window start (Unix seconds)", o.window_started], ["Window end (Unix seconds)", o.window_ended]] },
      { key: "running", title: "Running", value: c.available ? "Verified at this read" : "Not established", tone: c.available ? "confirmed" : "unknown", text: c.available ? "The service matched this candidate, binary and exact live process when this view was read. Runtime will look for that same process key in a new observation; refresh here to recheck the Organization association." : "The service did not establish a current process association. Healthy history, source application and missing exit evidence cannot fill in this result.", facts: [["Process key", c.process_key], ["Exact candidate", data.source.candidate_commit]] }
    ];
    if (x.state !== "unavailable" || b.state !== "unavailable" || b.launch_state !== "unavailable") stages.push({ key: "recovery", title: "Exit & rollback", value: b.state === "applied" ? "Base source restored" : x.state === "exited" ? "Process exited" : "Recovery not established", tone: b.state === "applied" ? "confirmed" : "unknown", text: "A process exit, resetting source to the original base and launching that base are separate facts. A base launch does not establish that the base process is currently healthy or running.", facts: [["Exit", x.state + (x.code ? " · code " + x.code : "")], ["Exit event", x.event_id], ["Source rollback", b.state], ["Rollback event", b.event_id], ["Restored base", b.base_commit], ["Base process launch", b.launch_state], ["Base launch event", b.launch_event_id]] });
    const map = el("div", "command-outcome-map organization-status-map"), trail = el("ol", "intervention-stage-list"), inspector = el("div", "outcome-inspector");
    map.style.setProperty("--outcome-stages", "6"); trail.setAttribute("aria-label", "Organization change journey"); inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected change stage");
    const choose = key => {
      const stage = stages.find(value => value.key === key);
      for (const control of trail.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.stage === key));
      inspector.dataset.state = stage.tone;
      inspector.replaceChildren(el("h4", "", stage.title), el("p", "outcome-value", stage.value), el("p", "", stage.text));
      const facts = el("dl", "fact-grid");
      for (const [label, value] of stage.facts) if (value) facts.append(append(el("div", "wide"), el("dt", "", label), el("dd", "mono", value)));
      if (facts.childElementCount) inspector.append(append(el("details", "source-review-evidence"), el("summary", "", "Stage evidence"), facts));
      if (key === "running" && c.available && runtimeHref) { const link = el("a", "button secondary", "Inspect exact process in Runtime"); link.href = runtimeHref; inspector.append(link); }
    };
    for (const [index, stage] of stages.entries()) {
      const control = button("", () => choose(stage.key), "outcome-stage"); control.dataset.stage = stage.key; control.dataset.state = stage.tone; control.setAttribute("aria-label", stage.title);
      const marker = el("span", "outcome-marker", index === 6 ? "↶" : String(index + 1).padStart(2, "0")); marker.setAttribute("aria-hidden", "true");
      const value = el("span", "outcome-stage-value", stage.value); value.id = "source-status-" + stage.key + "-value"; control.setAttribute("aria-describedby", value.id);
      control.append(marker, el("strong", "", stage.title), value);
      trail.append(append(el("li", stage.key === "recovery" ? "organization-status-recovery" : ""), control));
    }
    choose(stages.length > 6 ? "recovery" : c.available ? "running" : (stages.find(stage => stage.tone === "refused" || stage.tone === "pending") || stages.at(-1)).key);
    map.append(trail, inspector);
    const actions = el("div", "intervention-actions"); if (onRefresh) actions.append(button("Refresh change status", onRefresh));
    const stamp = inspectedAt instanceof Date ? inspectedAt.toLocaleTimeString() : "this read";
    actions.append(el("p", "detail-note", "Checked at " + stamp + ". Refresh captures the Review and status together."));
    const evidence = append(el("details", "source-review-evidence"), el("summary", "", "Change identity"), el("p", "mono", "Review · " + data.review_id), el("p", "mono", "Candidate · " + data.source.candidate_commit), el("p", "mono", "Record · " + recordHead));
    frame.append(heading, map, actions, evidence); return frame;
  }
  const RESPONSIBILITY_COUNTS = ["workflow_tasks", "open_workflow_tasks", "open_works", "outstanding_attempts", "legacy_tasks", "open_legacy_tasks", "handed_tasks", "unresolved_intents", "schedules", "pending_effects", "unknown_effects"];
  const RESPONSIBILITY_OPEN = RESPONSIBILITY_COUNTS.filter(key => !["workflow_tasks", "legacy_tasks"].includes(key));
  function responsibilityImpact(data, review, source) {
    const check = condition => { if (!condition) throw new Error("The responsibility check does not match this exact Organization Review and Record snapshot."); };
    const decimal = value => typeof value === "string" && /^(0|[1-9][0-9]{0,18})$/.test(value) && BigInt(value) <= 9223372036854775807n;
    check(closed(data, ["profile", "review_id", "mutation_id", "source", "basis", "state", "reason_code", "counts"]) && data.profile === "dna.organization.source-impact.v1");
    check(review?.organization_source === true && validText(data.review_id, 256) && data.review_id.length > 0 && data.review_id === review.id && data.mutation_id === review.id);
    const s = data.source, b = data.basis;
    check(closed(s, ["base_commit", "module_digest", "candidate_commit", "source_digest"]) && commitId(s.base_commit) && sourceHash(s.module_digest) && commitId(s.candidate_commit) && sourceHash(s.source_digest));
    check(s.candidate_commit === review.subject_digest);
    if (review.organization_source_digest) check(s.source_digest === review.organization_source_digest);
    check(closed(b, ["record_head", "record_revision", "memory", "scope", "position_binding", "admission_fence"]) && commitId(b.record_head) && decimal(b.record_revision) && b.memory === "record" && b.scope === "application" && b.position_binding === "unavailable" && b.admission_fence === "unavailable");
    check(source && typeof source.record_id === "string" && source.record_id.length > 0 && b.record_head === source.record_head && b.record_revision === source.record_revision);
    check(["unavailable", "obligations_observed", "none_observed"].includes(data.state));
    if (data.state === "unavailable") {
      check(["impact_read_limit", "impact_source_invalid", "impact_source_unavailable", "impact_unsupported_history"].includes(data.reason_code) && data.counts === null);
    } else {
      check(data.reason_code === (data.state === "obligations_observed" ? "impact_obligations_observed" : "impact_none_observed"));
      check(closed(data.counts, RESPONSIBILITY_COUNTS) && RESPONSIBILITY_COUNTS.every(key => decimal(data.counts[key])));
      const c = data.counts;
      check(BigInt(c.open_workflow_tasks) <= BigInt(c.workflow_tasks) && BigInt(c.open_legacy_tasks) <= BigInt(c.legacy_tasks) && BigInt(c.handed_tasks) <= BigInt(c.open_legacy_tasks));
      const observed = RESPONSIBILITY_OPEN.some(key => c[key] !== "0");
      check(data.state === (observed ? "obligations_observed" : "none_observed"));
    }
    return data;
  }
  function responsibilityCheck(data, { inspectedAt, onRefresh, unavailableReason } = {}) {
    const frame = el("section", "organization-impact"); frame.setAttribute("role", "region"); frame.setAttribute("aria-label", "Current responsibility check"); frame.dataset.state = data?.state || "unavailable";
    const heading = append(el("header", "source-review-heading"), el("p", "eyebrow accent", "ORGANIZATION / RECORDED RESPONSIBILITIES"), el("h3", "", "Current responsibility check"), el("p", "detail-note", "Application-wide evidence at this read. These responsibilities are not attributed to particular changed positions."));
    const state = data?.state || "unavailable";
    const status = el("p", "organization-impact-state", state === "obligations_observed" ? "Open responsibilities recorded" : state === "none_observed" ? "None observed in this bounded read" : "Responsibility evidence unavailable"); status.setAttribute("role", "status");
    frame.append(heading, status);
    if (state === "unavailable") {
      const reasons = {
        impact_read_limit: "The service could not establish complete evidence within this read's resource limit.",
        impact_source_invalid: "The recorded responsibility history could not be verified.",
        impact_source_unavailable: "Complete authorized responsibility evidence is unavailable.",
        impact_unsupported_history: "This read does not support all responsibility history in the application."
      };
      frame.append(el("p", "detail-note", unavailableReason || reasons[data?.reason_code] || "The current responsibility check could not be read."), el("p", "detail-note", "Counts are withheld. Unavailable evidence does not mean there are no responsibilities."));
    } else {
      const c = data.counts;
      const groups = [
        { key: "tasks", title: "Tasks", summary: c.open_workflow_tasks + " workflow · " + c.open_legacy_tasks + " legacy open", open: ["open_workflow_tasks", "open_legacy_tasks", "handed_tasks"], text: "Open Task counts distinguish workflow and legacy histories. Handed-off Tasks are included in open legacy Tasks; this read does not identify their assignees or establish that all are human tasks.", fields: [["Open workflow Tasks", "open_workflow_tasks"], ["Open legacy Tasks", "open_legacy_tasks"], ["Handed-off Tasks", "handed_tasks"], ["Recorded workflow Tasks", "workflow_tasks"], ["Recorded legacy Tasks", "legacy_tasks"]] },
        { key: "work", title: "Work", summary: c.open_works + " open · " + c.outstanding_attempts + " attempts", open: ["open_works", "outstanding_attempts"], text: "Works and their attempts sit within workflow Tasks. An outstanding attempt is not an additional independent obligation, and a recorded request alone does not establish a completed result.", fields: [["Open Works", "open_works"], ["Outstanding attempts", "outstanding_attempts"]] },
        { key: "schedules", title: "Schedules", summary: c.schedules + " recorded", open: ["schedules"], text: "Recorded schedules may cause later work. This count does not establish which process is running, whether a scheduled action will fire, or whether changing source transfers its responsibility.", fields: [["Recorded schedules", "schedules"]] },
        { key: "effects", title: "Uncertain work & effects", summary: c.unresolved_intents + " intents · " + c.pending_effects + " pending · " + c.unknown_effects + " unknown", open: ["unresolved_intents", "pending_effects", "unknown_effects"], text: "Unresolved intents and effect states must remain distinct. A missing or uncertain outcome is not evidence of success or failure; this read does not retry, cancel or resolve any effect.", fields: [["Unresolved intents", "unresolved_intents"], ["Pending effects", "pending_effects"], ["Unknown effects", "unknown_effects"]] }
      ];
      const workspace = el("div", "organization-impact-workspace"), controls = el("div", "organization-impact-categories"), inspector = el("div", "organization-impact-inspector");
      controls.setAttribute("role", "group"); controls.setAttribute("aria-label", "Responsibility categories");
      inspector.setAttribute("role", "group"); inspector.setAttribute("aria-label", "Selected responsibility category"); inspector.setAttribute("aria-live", "polite");
      const choose = key => {
        const group = groups.find(value => value.key === key);
        for (const control of controls.querySelectorAll("button")) control.setAttribute("aria-pressed", String(control.dataset.category === key));
        inspector.replaceChildren(el("p", "eyebrow", "RECORDED / APPLICATION-WIDE"), el("h4", "", group.title), el("p", "detail-note", group.text));
        const facts = el("dl", "organization-impact-counts");
        for (const [label, field] of group.fields) { const row = el("div"); row.dataset.count = field; row.append(el("dt", "", label), el("dd", "", c[field])); facts.append(row); }
        inspector.append(facts);
      };
      for (const group of groups) {
        const control = button("", () => choose(group.key), "organization-impact-category"); control.dataset.category = group.key; control.setAttribute("aria-label", group.title);
        control.append(el("strong", "", group.title), el("span", "", group.summary)); controls.append(control);
      }
      choose((groups.find(group => group.open.some(key => c[key] !== "0")) || groups[0]).key);
      workspace.append(controls, inspector); frame.append(workspace, el("p", "detail-note", "Categories overlap. Do not add them into a total obligation count."));
    }
    frame.append(el("p", "organization-impact-boundary", "This current read is not an assessment retained at decision time and does not authorize source application. Removing a source declaration does not retire responsibilities or reassign their work."));
    const actions = el("div", "intervention-actions");
    if (typeof onRefresh === "function") actions.append(button("Refresh responsibility check", onRefresh));
    const stamp = inspectedAt instanceof Date && Number.isFinite(inspectedAt.getTime()) ? inspectedAt.toLocaleTimeString() : "this read";
    actions.append(el("p", "detail-note", "Read at " + stamp + ". Refresh captures the Review and current responsibility check together.")); frame.append(actions);
    if (data) {
      const evidence = append(el("details", "source-review-evidence"), el("summary", "", "Responsibility check basis")), facts = el("dl", "fact-grid");
      for (const [label, value] of [["Review", data.review_id], ["Exact candidate", data.source.candidate_commit], ["Record head", data.basis.record_head], ["Record revision", data.basis.record_revision], ["Position attribution", "Unavailable"], ["Admission fence", "Unavailable"]]) facts.append(append(el("div", "wide"), el("dt", "", label), el("dd", "mono", value)));
      evidence.append(facts); frame.append(evidence);
    }
    return frame;
  }
  function ownershipPeople(ownership, { onInspectPerson, available = false } = {}) {
    const frame = el("section", "ownership-people");
    frame.setAttribute("role", "region"); frame.setAttribute("aria-label", "Declared members");
    frame.append(el("h3", "", "Declared members"));
    const identity = value => validText(value, 256) && value.length > 0 && !/[\u0000-\u001f\u007f]/.test(value);
    const memberships = ownership?.memberships;
    let total = 0;
    const valid = ownership?.instance_binding === "unavailable" && Array.isArray(memberships) && memberships.length <= 256 && memberships.every(row => {
      if (!closed(row, ["owner", "members"]) || !identity(row.owner) || !Array.isArray(row.members) || row.members.length > 256 || !row.members.every(identity)) return false;
      total += row.members.length; return total <= 2048;
    });
    if (!valid) {
      const notice = el("p", "detail-note", "Declared membership is unavailable in this source response."); notice.setAttribute("role", "status"); frame.append(notice); return frame;
    }
    frame.append(el("p", "detail-note", "Select a declared member to inspect Tasks recorded with that exact assignee. Membership does not establish a position occupant, Task owner or reassignment authority."));
    const enabled = available === true && typeof onInspectPerson === "function";
    if (!enabled) frame.append(el("p", "detail-note", "Recorded assignments are unavailable on this connection."));
    const groups = el("div", "ownership-people-groups");
    for (const row of memberships) {
      const owner = row.owner;
      const group = el("section", "ownership-people-group"); group.setAttribute("role", "group"); group.setAttribute("aria-label", "Declared members · " + owner);
      group.append(el("span", "eyebrow", "Declared owner"), el("h4", "", owner));
      const members = el("ul", "ownership-people-members");
      for (const name of row.members) {
        const control = button("", () => { if (enabled) onInspectPerson({ owner, name }); }, "ownership-person");
        control.disabled = !enabled; control.setAttribute("aria-label", "Inspect recorded assignments for " + name + " · declared owner " + owner);
        control.append(el("strong", "", name), el("span", "", "Inspect recorded assignments"));
        members.append(append(el("li"), control));
      }
      group.append(members.childElementCount ? members : el("p", "detail-note", "No members declared for this owner.")); groups.append(group);
    }
    frame.append(groups.childElementCount ? groups : el("p", "detail-note", "No owner memberships declared."));
    return frame;
  }
  window.IrisOwnershipPeople = { render: ownershipPeople };
  window.IrisOrganizationImpact = { validate: responsibilityImpact, render: responsibilityCheck };
  window.IrisOrganizationStatus = { validate: sourceStatus, render: statusJourney };
  window.IrisOrganizationReview = { validate: sourceCandidate, render: sourceReview };
  window.IrisOrganizationDraft = { mount };
  window.IrisOwnershipDraft = { mount: (host, options) => mount(host, { ...options, ownership: true }) };
})();
