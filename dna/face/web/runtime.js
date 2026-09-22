/* Native Hale observer v0. Observation only; no DNA identity or control surface. */
"use strict";
(() => {
  const PROFILE = "hale.iris.observer.v0";
  const MAX_BODY = 2 * 1024 * 1024;
  const TIMEOUT = 3000;
  const INTERVAL = 1000;
  const UNSAFE_NUMBER = Symbol("unsafe observer number");
  const el = (tag, className = "", text) => {
    const value = document.createElement(tag);
    if (className) value.className = className;
    if (text !== undefined) value.textContent = String(text);
    return value;
  };
  const append = (parent, ...children) => { children.filter(Boolean).forEach(child => parent.append(child)); return parent; };
  const button = (label, action, className = "button secondary") => {
    const value = el("button", className, label);
    value.type = "button";
    value.addEventListener("click", action);
    return value;
  };
  const require = (condition) => { if (!condition) throw new Error("unsupported_snapshot"); };
  const object = value => value !== null && typeof value === "object" && !Array.isArray(value);
  const text = value => typeof value === "string" && value.length <= 65536;
  const identity = value => Number.isSafeInteger(value) && value >= 0;
  const count = value => {
    if (value?.[UNSAFE_NUMBER]) return null;
    require(typeof value === "number");
    return Number.isSafeInteger(value) && value >= 0 ? value : null;
  };
  const measurement = value => value === null ? "Unavailable" : String(value);
  const boundedArray = (value, limit) => { require(Array.isArray(value) && value.length <= limit); return value; };
  const topicKey = topic => JSON.stringify([topic.shape, topic.name]);
  function safeURL(value) {
    if (typeof value !== "string" || /[\u0000-\u0020\u007f]/.test(value)) throw new Error("unsafe_url");
    const url = new URL(value);
    if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error("unsafe_url");
    return url;
  }
  function validateSnapshot(raw) {
    require(object(raw));
    const processIDs = new Set();
    const processes = boundedArray(raw.processes, 16).map(process => {
      require(object(process) && identity(process.pid) && process.pid > 0 && !processIDs.has(process.pid) && text(process.name) && text(process.model) && ["live", "dead"].includes(process.state));
      require(process.process_key === undefined || process.process_key === "" || (typeof process.process_key === "string" && /^[a-f0-9]{64}$/.test(process.process_key)));
      processIDs.add(process.pid);
      const locusIDs = new Set();
      const loci = boundedArray(process.loci, 2048).map(locus => {
        require(object(locus) && identity(locus.id) && identity(locus.parent) && !locusIDs.has(locus.id) && text(locus.type));
        locusIDs.add(locus.id);
        return { id: locus.id, parent: locus.parent, type: locus.type, pub: count(locus.pub), dlv: count(locus.dlv) };
      });
      const byID = new Map(loci.map(locus => [locus.id, locus]));
      // Parent zero is the native root sentinel. Missing nonzero parents are
      // visible as unresolved; cycles cannot supply truthful containment.
      const complete = new Set();
      for (const locus of loci) {
        const path = new Set();
        let current = locus;
        while (current && !complete.has(current.id)) {
          require(!path.has(current.id));
          path.add(current.id);
          current = current.parent === 0 ? null : byID.get(current.parent);
        }
        path.forEach(id => complete.add(id));
      }
      return { processKey: process.process_key || "", pid: process.pid, name: process.name, model: process.model, state: process.state, records: count(process.records), overruns: count(process.overruns), restarts: count(process.restarts), loci };
    });
    const topicIDs = new Set();
    const topics = boundedArray(raw.topics, 1024).map(topic => {
      require(object(topic) && text(topic.name) && topic.name.length > 0 && text(topic.shape) && !topicIDs.has(topicKey(topic)));
      topicIDs.add(topicKey(topic));
      return { name: topic.name, shape: topic.shape, pub: count(topic.pub), dlv: count(topic.dlv) };
    });
    const edges = boundedArray(raw.edges, 1024).map(edge => {
      require(object(edge) && text(edge.topic) && edge.topic.length > 0 && identity(edge.from) && edge.from > 0 && identity(edge.to) && edge.to > 0);
      // Native edges omit shape, so equal exposed tuples can be distinct
      // shaped topics. Keep rows separately, without a persistent edge ID.
      return { topic: edge.topic, from: edge.from, to: edge.to, matched: count(edge.matched), latSum: count(edge.latSum), latMax: count(edge.latMax), sends: count(edge.sends), delivers: count(edge.delivers) };
    });
    const events = boundedArray(raw.events, 64).map(event => { require(text(event)); return event; });
    return { ts: count(raw.ts), processes, topics, edges, events };
  }
  async function readJSON(url, signal, maximum) {
    const response = await fetch(url, { method: "GET", credentials: "omit", referrerPolicy: "no-referrer", redirect: "error", cache: "no-store", signal, headers: { Accept: "application/json" } });
    if (!response.ok) throw new Error("observer_unavailable");
    const length = response.headers.get("content-length");
    if (length && /^\d+$/.test(length) && Number(length) > maximum) throw new Error("body_limit");
    if (!response.body?.getReader) throw new Error("stream_unavailable");
    const reader = response.body.getReader();
    const decoder = new TextDecoder("utf-8", { fatal: true });
    let size = 0;
    const pieces = [];
    try {
      while (true) {
        const part = await reader.read();
        if (part.done) break;
        size += part.value.byteLength;
        if (size > maximum) throw new Error("body_limit");
        pieces.push(decoder.decode(part.value, { stream: true }));
      }
      pieces.push(decoder.decode());
      return JSON.parse(pieces.join(""), (key, value, context) => {
        if (typeof value !== "number") return value;
        // JSON.parse can round fractional/exponent or oversized tokens into
        // apparently safe integers. Verify the original native integer token.
        if (typeof context?.source !== "string") throw new Error("numeric_source_unavailable");
        const exact = /^(0|[1-9][0-9]*)$/.test(context.source) && (context.source.length < 16 || (context.source.length === 16 && context.source <= "9007199254740991"));
        return exact ? value : { [UNSAFE_NUMBER]: true };
      });
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
  }
  function mount(container, { onBack, backLabel = "Back to DNA inspection", applicationHost = false, expectedApplication = null, onApplication, expectedProcessKey = null } = {}) {
    let disposed = false, sequence = 0, active = null, timer = null;
    let configLoaded = false, configured = "", connected = false;
    let snapshot = null, selected = null, phase = "configuring", view = "containment";
    let viewScope = null, scopeNotice = "";
    let message = "Reading this cockpit's trusted observer configuration…";
    let lastTimestamp = null, lastAdvance = 0;
    let diagnosticsOpen = false;
    let boundApplication = null, applicationPin = expectedApplication, associationLost = false;
    let associationMessage = "Connect to match the application's identity with an observed process.";
    // An authorized source-status link may carry a process identity as a
    // viewing hint. It is independent of the application-control association.
    const processHintRequested = expectedProcessKey !== null && expectedProcessKey !== undefined && expectedProcessKey !== "";
    const processHint = typeof expectedProcessKey === "string" && /^[a-f0-9]{64}$/.test(expectedProcessKey) ? expectedProcessKey : "";
    let hintedPID = null, hintApplied = false, hintState = "waiting", hintSignature = "";
    const linkedProcess = el("section", "runtime-application-association");
    linkedProcess.setAttribute("role", "region"); linkedProcess.setAttribute("aria-label", "Linked process inspection");
    linkedProcess.setAttribute("aria-live", "polite"); linkedProcess.hidden = !processHintRequested;
    function reconcileProcessHint(next, nextPhase) {
      if (!processHint) return;
      const candidates = next.processes.filter(process => process.processKey === processHint && process.state === "live");
      const match = nextPhase === "live" && candidates.length === 1 ? candidates[0] : null;
      if (hintedPID !== null && hintedPID !== match?.pid) {
        const selectedPID = selected?.kind === "process" ? selected.key : selected?.kind === "locus" ? selected.pid : null;
        if (selectedPID === hintedPID) selected = null;
        if (viewScope?.pid === hintedPID) { viewScope = null; shown.clear(); }
        scopeNotice = "The linked process is no longer verified in a fresh observation. Its viewing focus was cleared.";
      }
      hintedPID = match?.pid ?? null;
      hintState = !["live", "empty"].includes(nextPhase) ? "stale" : candidates.length > 1 ? "ambiguous" : match ? "matched" : next.processes.some(process => process.processKey === processHint && process.state === "dead") ? "ended" : "missing";
      if (match && !hintApplied) {
        // Initial navigation selects once, without moving keyboard focus or
        // scrolling on a poll. Further exploration stays under user control.
        hintApplied = true; selected = { kind: "process", key: match.pid };
        viewScope = { pid: match.pid, locus: null }; scopeNotice = "";
      }
    }
    function renderProcessHint() {
      if (!processHintRequested) return;
      const state = !processHint ? "invalid" : !connected ? phase === "unavailable" ? "unavailable" : "waiting" : hintState;
      const signature = JSON.stringify([state, hintedPID]); if (signature === hintSignature) return; hintSignature = signature;
      const messages = {
        invalid: "The linked process identity is invalid. No process was selected.",
        waiting: "Connect to the configured observer to locate the exact linked process.",
        unavailable: "Observation is unavailable. The linked process and its viewing focus are no longer verified.",
        stale: "The observation is not fresh. The linked process and its viewing focus are no longer verified.",
        missing: "The exact linked process is not present in this observation. A matching name or reused PID does not replace it.",
        ended: "The observer reports that the linked process has ended. Its viewing focus was cleared.",
        ambiguous: "More than one live process reports the linked identity. No process can be selected from this link.",
        matched: "Exact linked process observed · PID " + hintedPID,
      };
      linkedProcess.dataset.state = state;
      linkedProcess.replaceChildren(el("h3", "", "Linked process"), el("p", "", messages[state]), el("p", "runtime-boundary", "This link selects observation only. It does not establish the Organization's current running version or grant application controls."));
      if (state === "matched") linkedProcess.append(button("Focus linked process", () => {
        const matches = phase === "live" && snapshot ? snapshot.processes.filter(process => process.state === "live" && process.processKey === processHint) : [];
        if (matches.length !== 1 || matches[0].pid !== hintedPID) return;
        selected = { kind: "process", key: hintedPID }; enterScope(hintedPID);
      }));
    }
    const sameBinding = (left, right) => left?.application.id === right?.application.id && left?.application.incarnation_id === right?.application.incarnation_id && left?.principal.mode === right?.principal.mode && left?.principal.name === right?.principal.name && left?.runtime.process_key === right?.runtime.process_key && left?.runtime.pid === right?.runtime.pid;
    const matched = process => boundApplication && process?.state === "live" && process.processKey === boundApplication.runtime.process_key && String(process.pid) === boundApplication.runtime.pid;
    function controlButton() {
      return button("Open application controls", () => { if (boundApplication && connected && phase === "live") onApplication?.(boundApplication); });
    }
    const association = el("section", "runtime-application-association");
    association.setAttribute("aria-label", "Application runtime association");
    association.setAttribute("aria-live", "polite");
    association.hidden = !applicationHost;
    async function associate(next, signal) {
      if (!applicationHost || associationLost) return;
      boundApplication = null;
      try {
        const candidate = await window.IrisApplication.runtimeBinding(path => readJSON(path, signal, 65536));
        if (!candidate) { associationMessage = "No current application process association. Observation remains available."; return; }
        if (applicationPin && !sameBinding(applicationPin, candidate)) {
          associationLost = true;
          associationMessage = "The application or access changed. Return to Application to verify its current incarnation before inspecting it again.";
          return;
        }
        applicationPin = candidate;
        const matches = next.processes.filter(process => process.state === "live" && process.processKey === candidate.runtime.process_key && String(process.pid) === candidate.runtime.pid);
        if (matches.length !== 1 || next.ts === null) { associationMessage = "The application's current process is not verified in this observation."; return; }
        boundApplication = candidate;
        associationMessage = candidate.application.name + " · matched to process " + candidate.runtime.pid;
      } catch { associationMessage = "Application identity could not be verified. Its control link has been cleared."; }
    }
    let associationSignature = "";
    function renderAssociation() {
      const signature = JSON.stringify([associationMessage, phase === "live", boundApplication?.runtime.process_key]);
      if (signature === associationSignature) return;
      associationSignature = signature;
      association.replaceChildren(el("p", "", associationMessage));
      if (boundApplication && phase === "live") {
        append(association, button("Focus application process", () => { const process = snapshot?.processes.find(matched); if (process) { selected = { kind: "process", key: process.pid }; enterScope(process.pid); } }), controlButton());
      }
    }
    const shown = new Map();
    let shownRoutes = 100, shownTopics = 100;
    const root = el("section", "runtime-instrument");
    root.setAttribute("aria-label", "Runtime observer");
    root.setAttribute("role", "region");
    const connection = el("section", "panel runtime-connection");
    const heading = append(el("div", "runtime-connection-heading"), append(el("div"), el("p", "eyebrow", "NATIVE OBSERVER / V0"), el("h2", "", "Read the running system")), button(backLabel, () => onBack?.()));
    const settings = el("details", "runtime-connection-settings");
    settings.open = true;
    settings.append(el("summary", "", "Connection settings"));
    const form = el("form", "observer-form");
    const label = el("label", "", "Observer URL");
    label.htmlFor = "runtime-observer-url";
    const input = el("input");
    input.id = label.htmlFor;
    input.type = "url";
    input.required = true;
    input.autocomplete = "off";
    input.spellcheck = false;
    input.value = "http://127.0.0.1:8787/";
    const connect = el("button", "button", "Connect observer");
    connect.type = "submit";
    const disconnect = button("Disconnect observer", () => stop("disconnected", "Disconnected. Observation and selection have been cleared."));
    const refresh = button("Refresh observation", () => { if (connected && !active) { clearTimeout(timer); timer = null; void poll(); } });
    const hint = el("p", "field-hint");
    hint.id = "runtime-observer-hint";
    input.setAttribute("aria-describedby", hint.id);
    const external = el("div", "observer-ready");
    external.hidden = true;
    const status = el("section", "runtime-observer-status");
    status.setAttribute("role", "region");
    status.setAttribute("aria-label", "Runtime observer status");
    status.setAttribute("aria-live", "polite");
    const statusTitle = el("strong"), statusMessage = el("p");
    append(status, statusTitle, statusMessage);
    const boundary = el("p", "runtime-boundary", "Read-only observation. Process and locus IDs belong to this connection; they are not application incarnations or DNA identities. Application controls are reached only through an explicit, currently verified process association.");
    append(form, label, append(el("div", "input-row"), input, connect), hint, external);
    settings.append(form);
    const connectionControls = append(el("div", "runtime-connection-controls"), settings, append(el("div", "runtime-connection-actions"), disconnect, refresh));
    append(connection, heading, connectionControls, status, boundary);
    const observations = el("div", "runtime-observations");
    append(root, connection, linkedProcess, association, observations);
    container.replaceChildren(root);

    const current = token => !disposed && token === sequence && !document.hidden;
    function clearConnection() {
      sequence += 1;
      connected = false;
      clearTimeout(timer);
      timer = null;
      if (active) active.abort();
      active = null;
      snapshot = null;
      boundApplication = null;
      hintedPID = null; hintApplied = false; hintState = "waiting";
      associationMessage = "Observation is disconnected. No application process is verified.";
      selected = null;
      viewScope = null;
      scopeNotice = "";
      lastTimestamp = null;
      lastAdvance = 0;
      diagnosticsOpen = false;
      shown.clear();
      shownRoutes = shownTopics = 100;
    }
    function stop(nextPhase, explanation) {
      clearConnection();
      settings.open = true;
      phase = nextPhase;
      message = explanation;
      render();
    }
    function renderStatus() {
      renderAssociation();
      renderProcessHint();
      root.dataset.connected = String(connected);
      const titles = { configuring: "Checking configuration", disconnected: "Observer disconnected", connecting: "Connecting to observer", live: "Observer live", empty: "Observer live — no processes", unchanged: "Observer timestamp unchanged", unverified: "Observer responding — freshness unavailable", unavailable: "Observer unavailable", paused: "Observation paused" };
      status.dataset.state = phase;
      statusTitle.textContent = titles[phase];
      statusMessage.textContent = message;
      connect.disabled = !configLoaded || connected || document.hidden;
      disconnect.disabled = !connected;
      refresh.disabled = !connected || Boolean(active);
      hint.textContent = configured ? "Configured live origin · " + configured + ". Connect explicitly to read its /snapshot. Other URLs prepare an external link only." : "Live observation is not configured. A safe URL prepares an external link only; it does not fetch a snapshot or start a process.";
    }
    function inspect(kind, key, pid) {
      selected = { kind, key, pid };
      renderObservations();
      const inspector = root.querySelector(".runtime-inspector");
      inspector?.focus({ preventScroll: true });
      inspector?.scrollIntoView({ block: window.matchMedia("(max-width: 980px)").matches ? "start" : "nearest", behavior: "auto" });
    }
    function selectionButton(label, kind, key, pid, className) {
      const result = button(label, () => inspect(kind, key, pid), className);
      result.dataset.runtimeFocus = JSON.stringify([kind, key, pid ?? null]);
      result.setAttribute("aria-pressed", String(selected?.kind === kind && selected.key === key && selected.pid === pid));
      return result;
    }
    function enterScope(pid = null, locus = null) {
      if (!snapshot) return;
      if (pid !== null) {
        const process = snapshot.processes.find(item => item.pid === pid);
        if (!process || (locus !== null && !process.loci.some(item => item.id === locus))) return;
      }
      viewScope = pid === null ? null : { pid, locus };
      scopeNotice = "";
      if (view === "routes") view = "containment";
      renderObservations();
      const heading = root.querySelector(".runtime-breadcrumbs");
      heading?.focus({ preventScroll: true });
      heading?.scrollIntoView({ block: window.matchMedia("(max-width: 980px)").matches ? "start" : "nearest", behavior: "auto" });
    }
    function enterButton(pid, locus = null) {
      const label = locus === null ? "Enter process " + pid : "Enter locus " + locus + " in process " + pid;
      const control = button(locus === null ? "Enter process →" : "Enter locus →", () => enterScope(pid, locus), "runtime-enter");
      control.setAttribute("aria-label", label);
      control.dataset.runtimeFocus = JSON.stringify(["enter", pid, locus]);
      return control;
    }
    function reconcileScope(next) {
      const previous = new Map((snapshot?.processes || []).map(process => [process.pid, process]));
      const currentProcesses = new Map(next.processes.map(process => [process.pid, process]));
      const restarted = new Set(next.processes.filter(process => previous.has(process.pid) && (previous.get(process.pid).restarts !== process.restarts || previous.get(process.pid).processKey !== process.processKey)).map(process => process.pid));
      const selectedPID = selected?.kind === "process" ? selected.key : selected?.kind === "locus" ? selected.pid : null;
      if (selectedPID !== null && (!currentProcesses.has(selectedPID) || restarted.has(selectedPID))) {
        selected = null;
        if (restarted.has(selectedPID)) scopeNotice = "The restart counter for process " + selectedPID + " changed. Inspector selection was cleared; identifiers do not establish an incarnation.";
      }
      if (!viewScope) return;
      const process = currentProcesses.get(viewScope.pid);
      if (!process) {
        scopeNotice = "The focused process " + viewScope.pid + " is no longer observed. Returned to the observed fleet.";
        viewScope = null;
      } else if (restarted.has(viewScope.pid)) {
        scopeNotice = "The restart counter for process " + viewScope.pid + " changed. Viewing scope and inspector selection were cleared; returned to the observed fleet.";
        viewScope = null;
        selected = null;
        shown.clear();
      } else if (viewScope.locus !== null && !process.loci.some(locus => locus.id === viewScope.locus)) {
        scopeNotice = "The focused locus " + viewScope.locus + " is no longer observed. Returned to process " + process.pid + ".";
        viewScope = { pid: process.pid, locus: null };
      }
    }
    function scopeBreadcrumbs() {
      const nav = el("nav", "runtime-breadcrumbs");
      nav.setAttribute("aria-label", "Runtime containment path");
      nav.tabIndex = -1;
      nav.dataset.runtimeFocus = "containment-path";
      const crumb = (label, ariaLabel, action, current) => {
        const control = button(label, action, "runtime-crumb");
        control.setAttribute("aria-label", ariaLabel);
        control.dataset.runtimeFocus = "crumb:" + ariaLabel;
        if (current) control.setAttribute("aria-current", "location");
        nav.append(control);
      };
      crumb("Observed fleet", "Observed fleet", () => enterScope(), !viewScope);
      if (!viewScope) return nav;
      const process = snapshot.processes.find(item => item.pid === viewScope.pid);
      if (!process) return nav;
      crumb("Process " + process.pid, "Process " + process.pid, () => enterScope(process.pid), viewScope.locus === null);
      if (viewScope.locus === null) return nav;
      const byID = new Map(process.loci.map(locus => [locus.id, locus]));
      const ancestors = [];
      let locus = byID.get(viewScope.locus), unresolved = null;
      while (locus) {
        ancestors.unshift(locus);
        if (locus.parent === 0) break;
        if (!byID.has(locus.parent)) { unresolved = locus.parent; break; }
        locus = byID.get(locus.parent);
      }
      if (unresolved !== null) nav.append(el("span", "runtime-crumb-unresolved", "Parent #" + unresolved + " unobserved"));
      if (ancestors.length > 32) nav.append(el("span", "runtime-crumb-unresolved", ancestors.length - 32 + " earlier observed ancestors omitted"));
      for (const ancestor of ancestors.slice(-32)) crumb((ancestor.type || "Unnamed locus") + " · #" + ancestor.id, "Locus " + ancestor.id + " in process " + process.pid, () => enterScope(process.pid, ancestor.id), ancestor.id === viewScope.locus);
      return nav;
    }
    function locusControls(process, locus, className = "") {
      const group = el("div", "runtime-locus-controls " + className);
      const item = selectionButton("", "locus", locus.id, process.pid, "runtime-locus");
      item.setAttribute("aria-label", "Inspect locus " + locus.id);
      append(item, el("strong", "", locus.type || "Unnamed locus"), el("span", "mono", "#" + locus.id + (locus.parent === 0 ? " · Root" : " · Parent #" + locus.parent + (process.loci.some(candidate => candidate.id === locus.parent) ? "" : " unobserved"))));
      append(group, item, enterButton(process.pid, locus.id));
      return group;
    }
    function renderFocusedScope(listMode) {
      const process = snapshot.processes.find(item => item.pid === viewScope.pid);
      const locus = viewScope.locus === null ? null : process.loci.find(item => item.id === viewScope.locus);
      const frame = el("section", "runtime-scope-membrane" + (listMode ? " runtime-scope-list" : ""));
      const header = el("header", "runtime-scope-header");
      header.tabIndex = -1;
      header.dataset.runtimeFocus = "scope-heading";
      const title = append(el("div"), el("p", "eyebrow", locus ? "INSIDE OBSERVED LOCUS" : "INSIDE OBSERVED PROCESS"), el("h2", "", locus ? (locus.type || "Unnamed locus") + " · #" + locus.id : processName(process.pid)));
      append(header, title, el("p", "runtime-scope-identity mono", "Process " + process.pid + (locus ? " / Locus " + locus.id : " · " + process.state)));
      const inspectCurrent = selectionButton(locus ? "Inspect this locus" : "Inspect this process", locus ? "locus" : "process", locus ? locus.id : process.pid, locus ? process.pid : undefined, "button secondary");
      inspectCurrent.setAttribute("aria-label", locus ? "Inspect locus " + locus.id : "Inspect process " + process.pid);
      header.append(inspectCurrent);
      frame.append(header, el("h3", "runtime-contents-heading", "Immediate observed contents"));
      const children = locus ? process.loci.filter(item => item.parent !== 0 && item.parent === locus.id) : process.loci.filter(item => item.parent === 0);
      const key = JSON.stringify(["scope", process.pid, viewScope.locus]);
      const maximum = shown.get(key) || 40;
      const list = el("ul", "runtime-immediate-contents");
      list.setAttribute("aria-label", "Immediate observed contents");
      for (const child of children.slice(0, maximum)) {
        const row = el("li");
        row.dataset.locusId = String(child.id);
        row.dataset.parentId = String(child.parent);
        row.append(locusControls(process, child));
        list.append(row);
      }
      frame.append(list);
      if (!children.length) frame.append(el("p", "runtime-empty-note", locus ? "No immediate children are observed for this locus in the current snapshot." : "No loci with the native root marker are observed in this process."));
      else frame.append(el("p", "runtime-coverage", "Showing " + Math.min(maximum, children.length) + " of " + children.length + " immediate observed " + (locus ? "children" : "root loci") + ". Deeper descendants appear only when entered."));
      if (children.length > maximum) {
        const more = button("Show more immediate contents", () => { shown.set(key, maximum + 80); renderObservations(); }, "text-link runtime-more");
        more.dataset.runtimeFocus = "more-scope:" + key;
        frame.append(more);
      }
      if (!locus) {
        const byID = new Set(process.loci.map(item => item.id));
        const unresolved = process.loci.filter(item => item.parent !== 0 && !byID.has(item.parent));
        if (unresolved.length) {
          frame.append(el("h3", "runtime-contents-heading runtime-unresolved-heading", "Unresolved parent references"), el("p", "runtime-coverage", "These loci are observed in this process, but their parents are absent. They are not asserted to be root loci."));
          const other = el("ul", "runtime-immediate-contents runtime-unresolved-contents");
          other.setAttribute("aria-label", "Loci with unobserved parents");
          for (const child of unresolved.slice(0, maximum)) other.append(append(el("li"), locusControls(process, child)));
          frame.append(other);
          if (unresolved.length > maximum) {
            const more = button("Show more unresolved loci", () => { shown.set(key, maximum + 80); renderObservations(); }, "text-link runtime-more");
            more.dataset.runtimeFocus = "more-unresolved:" + key;
            frame.append(el("p", "runtime-coverage", "Showing " + maximum + " of " + unresolved.length + " loci with unobserved parents."), more);
          }
        }
      }
      frame.append(el("p", "runtime-scope-boundary", "Viewing observed containment changes this canvas only. Inspector selection remains separate; no acting position or authority is selected."));
      return frame;
    }
    function facts(entries) {
      const list = el("dl", "fact-grid");
      entries.forEach(([name, value]) => list.append(append(el("div"), el("dt", "", name), el("dd", "", value))));
      return list;
    }
    function processName(pid) {
      const process = snapshot.processes.find(item => item.pid === pid);
      return process ? (process.name || "Unnamed process") + " · PID " + pid : "Unobserved PID " + pid;
    }
    function processTree(process, listMode) {
      const area = el("ul", listMode ? "runtime-locus-list" : "runtime-locus-tree");
      area.dataset.runtimeScroll = String(process.pid);
      area.setAttribute("aria-label", "Loci in process " + process.pid);
      const byID = new Map(process.loci.map(locus => [locus.id, locus]));
      const children = new Map();
      const roots = [];
      for (const locus of process.loci) {
        if (locus.parent !== 0 && byID.has(locus.parent)) {
          if (!children.has(locus.parent)) children.set(locus.parent, []);
          children.get(locus.parent).push(locus);
        } else roots.push(locus);
      }
      const maximum = shown.get(process.pid) || 40;
      let visible = 0;
      const pending = roots.map(locus => ({ locus, parent: area })).reverse();
      while (pending.length && visible < maximum) {
        const { locus, parent } = pending.pop();
        const row = el("li");
        row.append(locusControls(process, locus));
        parent.append(row);
        visible += 1;
        const descendants = children.get(locus.id) || [];
        let branch = area;
        if (!listMode && descendants.length) { branch = el("ul"); row.append(branch); }
        for (let i = descendants.length - 1; i >= 0; i -= 1) pending.push({ locus: descendants[i], parent: branch });
      }
      return { area, visible, maximum };
    }
    function renderContainment(listMode) {
      const fleet = el("div", listMode ? "runtime-fleet runtime-fleet-list" : "runtime-fleet");
      for (const process of snapshot.processes) {
        const card = el("section", "runtime-process " + (process.state === "dead" ? "is-dead" : ""));
        const select = selectionButton("", "process", process.pid, undefined, "runtime-process-select");
        select.setAttribute("aria-label", "Inspect process " + process.pid);
        append(select, el("span", "runtime-process-state", process.state === "live" ? "LIVE" : "DEAD"), el("strong", "", process.name || "Unnamed process"), el("span", "mono", "PID " + process.pid + " · " + process.loci.length + " observed loci"));
        card.append(select, enterButton(process.pid));
        if (matched(process)) card.append(el("p", "runtime-coverage", "Application · " + boundApplication.application.name));
        const tree = processTree(process, listMode);
        card.append(tree.area);
        if (!process.loci.length) card.append(el("p", "runtime-empty-note", "No live loci observed in this process."));
        if (tree.visible < process.loci.length) {
          const more = button("Show more loci in process " + process.pid, () => { shown.set(process.pid, tree.maximum + 80); renderObservations(); }, "text-link runtime-more");
          more.dataset.runtimeFocus = "more-loci:" + process.pid;
          card.append(el("p", "runtime-coverage", "Showing " + tree.visible + " of " + process.loci.length + " observed loci."), more);
        }
        fleet.append(card);
      }
      return fleet;
    }
    function renderRoutes() {
      const routes = el("div", "runtime-routes");
      routes.append(el("p", "runtime-coverage", "Observed process-to-process routes. These endpoints do not identify individual loci. Arrows follow from → to; same-process routes return to the same process."));
      const edges = el("ul", "runtime-route-list");
      edges.setAttribute("aria-label", "Topic routes");
      for (const [index, edge] of snapshot.edges.slice(0, shownRoutes).entries()) {
        const row = selectionButton("", "edge", index, undefined, "runtime-route");
        row.setAttribute("aria-label", "Inspect route " + edge.topic + " from " + edge.from + " to " + edge.to);
        const matches = snapshot.topics.filter(topic => topic.name === edge.topic).length;
        const shape = matches > 1 ? "Topic shape ambiguous" : matches === 1 ? "Observed topic name" : "Topic details unobserved";
        append(row, el("span", "", processName(edge.from)), append(el("span", "runtime-route-topic"), el("span", "", "→ " + edge.topic + " →"), el("small", "", (edge.from === edge.to ? "Same-process route · " : "") + shape)), el("span", "", processName(edge.to)));
        edges.append(append(el("li"), row));
      }
      routes.append(edges);
      if (!snapshot.edges.length) routes.append(el("p", "runtime-empty-note", "No routes observed in this snapshot."));
      if (shownRoutes < snapshot.edges.length) {
        const more = button("Show more routes", () => { shownRoutes += 100; renderObservations(); });
        more.dataset.runtimeFocus = "more-routes";
        routes.append(more);
      }
      const topics = el("ul", "runtime-topic-list");
      topics.setAttribute("aria-label", "Observed topics");
      snapshot.topics.slice(0, shownTopics).forEach(topic => {
        const item = selectionButton(topic.name + " · " + (topic.shape || "Empty shape"), "topic", topicKey(topic), undefined, "runtime-topic");
        item.setAttribute("aria-label", "Inspect topic " + topic.name + " with shape " + topic.shape);
        topics.append(append(el("li"), item));
      });
      append(routes, el("h3", "runtime-subheading", "Observed topics"), topics);
      if (!snapshot.topics.length) routes.append(el("p", "runtime-empty-note", "No topics observed in this snapshot."));
      if (shownTopics < snapshot.topics.length) {
        const more = button("Show more topics", () => { shownTopics += 100; renderObservations(); });
        more.dataset.runtimeFocus = "more-topics";
        routes.append(more);
      }
      return routes;
    }
    function renderInspector() {
      const inspector = el("aside", "panel runtime-inspector");
      inspector.tabIndex = -1;
      inspector.dataset.runtimeFocus = "inspector";
      inspector.setAttribute("aria-label", "Runtime inspector");
      append(inspector, el("p", "eyebrow", "OBSERVATION DETAIL"), el("h2", "", "Runtime inspector"));
      const process = selected?.kind === "process" ? snapshot.processes.find(item => item.pid === selected.key) : snapshot.processes.find(item => item.pid === selected?.pid);
      if (selected?.kind === "process" && process) {
        append(inspector, el("h3", "", process.name || "Unnamed process"), facts([["Connection-local PID", process.pid], ["State", process.state], ["Model", process.model || "Not reported"], ["Observed loci", process.loci.length], ["Records", measurement(process.records)], ["Observer ring overruns", measurement(process.overruns)], ["Restarts", measurement(process.restarts)]]), el("p", "runtime-boundary", "Overruns count this collector's overwritten-ring advances, not application or network loss."));
      } else if (selected?.kind === "locus" && process?.loci.some(item => item.id === selected.key)) {
        const locus = process.loci.find(item => item.id === selected.key);
        append(inspector, el("h3", "", locus.type || "Unnamed locus"), facts([["Connection-local locus", locus.id], ["Process PID", process.pid], ["Parent locus", locus.parent === 0 ? "Root" : locus.parent + (process.loci.some(item => item.id === locus.parent) ? "" : " · unobserved")], ["Published", measurement(locus.pub)], ["Delivered", measurement(locus.dlv)]]));
      } else if (selected?.kind === "topic" && snapshot.topics.some(item => topicKey(item) === selected.key)) {
        const topic = snapshot.topics.find(item => topicKey(item) === selected.key);
        append(inspector, el("h3", "", topic.name), facts([["Published", measurement(topic.pub)], ["Delivered", measurement(topic.dlv)]]), el("h4", "", "Reported shape"), el("pre", "runtime-shape", topic.shape || "Not reported"));
      } else if (selected?.kind === "edge" && snapshot.edges[selected.key]) {
        const edge = snapshot.edges[selected.key];
        const matches = snapshot.topics.filter(topic => topic.name === edge.topic).length;
        append(inspector, el("h3", "", edge.topic), facts([["From", processName(edge.from)], ["To", processName(edge.to)], ["Topic shape", matches > 1 ? "Ambiguous — route omits shape" : "Not carried by this route"], ["Matched observations", measurement(edge.matched)], ["Source topic sends", measurement(edge.sends)], ["Destination topic deliveries", measurement(edge.delivers)], ["Latency sum (native units)", measurement(edge.latSum)], ["Latency maximum (native units)", measurement(edge.latMax)]]), el("p", "runtime-boundary", "Sends and deliveries are whole endpoint-topic totals, not pair-specific loss or successful application effects. Route selection clears on the next observation because native rows have no unambiguous persistent identity."));
      } else {
        selected = null;
        inspector.append(el("p", "runtime-empty-note", "Select a process, locus, topic or route to inspect the values reported by this connection."));
      }
      if (matched(process) && phase === "live") inspector.append(el("p", "runtime-coverage", "Verified application process · " + boundApplication.application.name), controlButton());
      inspector.append(el("p", "runtime-boundary", "Counters are cumulative native observations, not rates. Unsafe numeric measurements are unavailable. Connection-local IDs may be reused after restarts; application incarnation is linked only when separately verified."));
      return inspector;
    }
    function renderObservations() {
      const focus = document.activeElement?.dataset?.runtimeFocus;
      const scrolls = new Map(Array.from(observations.querySelectorAll("[data-runtime-scroll]")).map(item => [item.dataset.runtimeScroll, [item.scrollTop, item.scrollLeft]]));
      const previousEvents = observations.querySelector(".runtime-events");
      if (previousEvents && snapshot) diagnosticsOpen = previousEvents.open;
      observations.replaceChildren();
      if (!snapshot) {
        observations.append(append(el("div", "runtime-idle"), el("div", "runtime-idle-orbit", "◌"), el("h2", "", phase === "unavailable" ? "Observation unavailable" : phase === "connecting" ? "Waiting for the first snapshot" : "A separate window into execution"), el("p", "", "Connect to the configured native observer to inspect its fleet, containment and routes. No application Record is required.")));
        return;
      }
      const summary = el("dl", "runtime-metrics");
      [["Last observation", measurement(snapshot.ts)], ["Processes", snapshot.processes.length], ["Topics", snapshot.topics.length], ["Routes", snapshot.edges.length]].forEach(([name, value]) => summary.append(append(el("div"), el("dt", "", name), el("dd", "", value))));
      const controls = el("div", "runtime-view-controls");
      controls.setAttribute("aria-label", "Observation presentation");
      for (const [value, label] of [["containment", "Process containment"], ["routes", "Topic routes"], ["list", "Accessible list"]]) {
        const control = button(label, () => { view = value; renderObservations(); }, "scope-button");
        control.dataset.runtimeFocus = "view:" + value;
        control.setAttribute("aria-pressed", String(view === value));
        controls.append(control);
      }
      const canvas = el("section", "panel runtime-canvas");
      canvas.setAttribute("aria-label", view === "routes" ? "Observed routing" : "Observed processes");
      canvas.dataset.scopeKind = viewScope ? viewScope.locus === null ? "process" : "locus" : "fleet";
      canvas.dataset.scopePid = viewScope ? String(viewScope.pid) : "";
      canvas.dataset.scopeLocus = viewScope?.locus !== null && viewScope?.locus !== undefined ? String(viewScope.locus) : "";
      const scopeStatus = el("p", "runtime-scope-status" + (scopeNotice ? " changed" : ""), scopeNotice || (view === "routes" ? "Topic routes cover the observed fleet. Your containment viewing scope is retained when you return." : viewScope ? "Viewing immediate observed contents. Enter changes the canvas; Inspect changes the separate inspector." : "Observed fleet overview. Enter a process or locus to focus its immediate observed contents."));
      scopeStatus.setAttribute("role", "status");
      scopeStatus.setAttribute("aria-label", "Runtime viewing scope");
      append(canvas, controls, scopeBreadcrumbs(), scopeStatus, view === "routes" ? renderRoutes() : viewScope ? renderFocusedScope(view === "list") : snapshot.processes.length ? renderContainment(view === "list") : el("p", "runtime-empty-note", "The observer answered successfully with no processes. This is an empty observation, not a connection failure."));
      const layout = append(el("div", "runtime-observation-layout"), canvas, renderInspector());
      const events = el("details", "panel runtime-events");
      events.open = diagnosticsOpen;
      const eventHeading = el("summary", "", "Observer diagnostics · " + snapshot.events.length);
      eventHeading.dataset.runtimeFocus = "diagnostics";
      append(events, eventHeading, el("p", "runtime-coverage", "A short diagnostic tail, not durable or replayable Record events. Missing routes or events do not establish an absence of traffic."));
      const eventList = el("ol");
      snapshot.events.forEach(event => eventList.append(el("li", "mono", event)));
      events.append(snapshot.events.length ? eventList : el("p", "runtime-empty-note", "No events reported in this snapshot."));
      append(observations, summary, layout, events);
      observations.querySelectorAll("[data-runtime-scroll]").forEach(item => {
        const position = scrolls.get(item.dataset.runtimeScroll);
        if (position) { item.scrollTop = position[0]; item.scrollLeft = position[1]; }
      });
      if (focus) {
        const restored = Array.from(observations.querySelectorAll("[data-runtime-focus]")).find(item => item.dataset.runtimeFocus === focus);
        if (restored) restored.focus({ preventScroll: true });
        else if (scopeNotice) observations.querySelector(".runtime-scope-header, .runtime-breadcrumbs")?.focus({ preventScroll: true });
      }
    }
    function render() { if (!disposed) { renderStatus(); renderObservations(); } }
    async function poll() {
      if (!connected || active || disposed || document.hidden) return;
      const token = sequence;
      const request = new AbortController();
      active = request;
      const timeout = setTimeout(() => request.abort(), TIMEOUT);
      renderStatus();
      try {
        const next = validateSnapshot(await readJSON(configured + "/snapshot", request.signal, MAX_BODY));
        if (!current(token) || !connected) return;
        if (next.ts !== null && lastTimestamp !== null && next.ts < lastTimestamp) throw new Error("timestamp_regressed");
        const advanced = next.ts !== null && (lastTimestamp === null || next.ts > lastTimestamp);
        if (advanced) { lastTimestamp = next.ts; lastAdvance = performance.now(); }
        if (next.ts !== null && !advanced && performance.now() - lastAdvance >= TIMEOUT) throw new Error("timestamp_stalled");
        await associate(next, request.signal);
        if (!current(token) || !connected) return;
        reconcileScope(next);
        if (selected?.kind === "edge") selected = null;
        phase = next.ts === null ? "unverified" : !advanced ? "unchanged" : next.processes.length ? "live" : "empty";
        reconcileProcessHint(next, phase);
        snapshot = next;
        message = next.ts === null ? "The observer answered, but its timestamp cannot be represented exactly. Freshness is unavailable; measurements are not rounded." : !advanced ? "The source timestamp has not advanced. Repeated HTTP responses are not new evidence; observation clears after three seconds without progress." : "Native snapshot received. Polling while this view is visible. Last observation is observer monotonic nanoseconds, not wall-clock time.";
        render();
      } catch (error) {
        if (!current(token)) return;
        stop("unavailable", error.message === "body_limit" ? "The observer response exceeded the 2 MiB limit. Observation and selection were cleared; reconnect explicitly." : error.message === "timestamp_stalled" || error.message === "timestamp_regressed" ? "The observer timestamp stopped advancing or moved backwards. Observation and selection were cleared; reconnect explicitly." : error.message === "unsupported_snapshot" ? "The observer returned an unsupported or inconsistent snapshot. Observation and selection were cleared; reconnect explicitly." : error.message === "numeric_source_unavailable" ? "This browser cannot verify the original numeric values in observer JSON. Live observation is unavailable." : "The observer could not be read within the bounded request. Observation and selection were cleared; reconnect explicitly.");
      } finally {
        clearTimeout(timeout);
        request.abort();
        if (active === request) active = null;
        if (current(token) && connected) { renderStatus(); timer = setTimeout(() => { timer = null; void poll(); }, INTERVAL); }
      }
    }
    async function configure() {
      if (disposed || document.hidden || active) return;
      const token = sequence;
      const request = new AbortController();
      active = request;
      const timeout = setTimeout(() => request.abort(), TIMEOUT);
      try {
        const config = await readJSON("/iris/observer.json", request.signal, 4096);
        if (!current(token)) return;
        require(object(config) && Object.keys(config).length === 2 && config.profile === PROFILE && typeof config.origin === "string");
        if (config.origin) {
          const url = safeURL(config.origin);
          require(url.pathname === "/" && !url.search && !url.hash);
          configured = url.origin;
          input.value = configured;
        }
        phase = "disconnected";
        message = configured ? "A trusted observer origin is configured. Select Connect observer to begin read-only observation." : "No live observer origin is configured. You can prepare a safe external observer link.";
      } catch {
        if (!current(token)) return;
        configured = "";
        phase = "disconnected";
        message = "Live observer configuration is unavailable. A safe external link remains available; no observer snapshot was requested.";
      } finally {
        clearTimeout(timeout);
        request.abort();
        if (active === request) active = null;
        if (current(token)) { configLoaded = true; render(); }
      }
    }
    form.addEventListener("submit", event => {
      event.preventDefault();
      if (!configLoaded || disposed || document.hidden) return;
      clearConnection();
      external.replaceChildren();
      external.hidden = true;
      try {
        const url = safeURL(input.value.trim());
        input.removeAttribute("aria-invalid");
        if (configured && url.origin === configured && url.pathname === "/" && !url.search && !url.hash) {
          connected = true;
          // Collapse only for this explicit connection action. Polling never
          // changes the user's later choice to open or close these settings.
          settings.open = false;
          phase = "connecting";
          message = "Connecting to the configured native observer. No prior observation is retained.";
          render();
          disconnect.focus({ preventScroll: true });
          void poll();
        } else {
          const open = el("a", "button secondary", "Open runtime observer");
          open.href = url.href;
          open.target = "_blank";
          open.rel = "noopener noreferrer";
          append(external, open, el("p", "", "External link only. This cockpit has not connected to or verified this observer; no snapshot was fetched."));
          external.hidden = false;
          phase = "disconnected";
          message = "External observer link prepared. Live observation remains disconnected.";
          render();
        }
      } catch {
        input.setAttribute("aria-invalid", "true");
        phase = "disconnected";
        message = "Enter an absolute http:// or https:// observer URL without credentials. Nothing was connected.";
        render();
      }
    });
    input.addEventListener("input", () => {
      external.replaceChildren();
      external.hidden = true;
      if (connected || snapshot) stop("disconnected", "The connection address changed. Observation and selection were cleared; connect explicitly.");
    });
    const visibility = () => {
      if (document.hidden) stop("paused", "Observation stopped while this page was hidden. Reconnect explicitly when you return.");
      else if (!configLoaded) { phase = "configuring"; message = "Reading trusted observer configuration…"; render(); void configure(); }
      else { phase = "disconnected"; message = "Observation remains disconnected after leaving the page. Select Connect observer to resume."; render(); }
    };
    document.addEventListener("visibilitychange", visibility);
    const pageHide = () => stop("paused", "Observation stopped when this page was left. Reconnect explicitly to resume.");
    window.addEventListener("pagehide", pageHide);
    render();
    void configure();
    return { destroy() { disposed = true; clearConnection(); document.removeEventListener("visibilitychange", visibility); window.removeEventListener("pagehide", pageHide); container.replaceChildren(); } };
  }
  window.IrisRuntime = Object.freeze({ mount });
})();
