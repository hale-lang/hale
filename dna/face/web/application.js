/* Generic application administration. The registered app owns state and effects. */
"use strict";
(() => {
  const API = "/api/hale/v1/applications";
  const PROFILE = "hale.application.v1";
  const STORAGE = "iris.application-recovery.v1:";
  const TIMEOUT = 5000, RESPONSE_LIMIT = 65536, POLL_INTERVAL = 2000;
  const encoder = new TextEncoder();
  const node = (tag, className = "", text) => {
    const item = document.createElement(tag);
    if (className) item.className = className;
    if (text !== undefined) item.textContent = String(text);
    return item;
  };
  const append = (parent, ...children) => { children.filter(Boolean).forEach(child => parent.append(child)); return parent; };
  const button = (label, action, className = "button secondary") => {
    const item = node("button", className, label);
    item.type = "button";
    item.addEventListener("click", action);
    return item;
  };
  const closed = (value, keys) => value !== null && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
  const bytes = value => encoder.encode(value).length;
  function literal(value, maximum = 512) {
    if (typeof value !== "string" || value.includes("\u0000") || bytes(value) > maximum) return false;
    for (let i = 0; i < value.length; i += 1) {
      const code = value.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff) {
        const following = value.charCodeAt(++i);
        if (!(following >= 0xdc00 && following <= 0xdfff)) return false;
      } else if (code >= 0xdc00 && code <= 0xdfff) return false;
    }
    return true;
  }
  const id = value => typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(value);
  const digestID = value => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
  const fingerprint = value => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value);
  const revision = value => typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value) && (value.length < 19 || (value.length === 19 && value <= "9223372036854775807"));
  const actor = value => closed(value, ["mode", "name"]) && value.mode === "trusted_local" && id(value.name);
  const sameActor = (left, right) => left?.mode === right?.mode && left?.name === right?.name;
  class Failure extends Error {
    constructor(code, status = 0) { super(code); this.code = code; this.status = status; }
  }
  const assert = condition => { if (!condition) throw new Failure("invalid_response"); };
  function application(value) {
    assert(closed(value, ["id", "kind", "name", "incarnation_id"]) && digestID(value.id) && value.kind === "hale" && literal(value.name) && digestID(value.incarnation_id));
    return value;
  }
  function envelope(body, principal = null, refusal = false) {
    assert(closed(body, refusal ? ["api_version", "profile", "principal", "data", "error"] : ["api_version", "profile", "principal", "data"]) && body.api_version === "hale.v1" && body.profile === PROFILE && actor(body.principal));
    if (principal && !sameActor(body.principal, principal)) throw new Failure("identity_changed");
    return body.data;
  }
  function publicError(body) {
    if (!closed(body, ["api_version", "profile", "error"]) || body.api_version !== "hale.v1" || body.profile !== PROFILE || !closed(body.error, ["code", "message"]) || !id(body.error.code) || !literal(body.error.message)) throw new Failure("invalid_response");
    return body.error.code;
  }
  function capabilities(data, appID) {
    assert(closed(data, ["application_id", "operations"]) && data.application_id === appID && Array.isArray(data.operations) && data.operations.length <= 1);
    for (const operation of data.operations) {
      assert(closed(operation, ["id", "version", "label", "target", "supported", "available", "authorized", "input", "preconditions", "success_meaning", "recovery"]));
      assert(id(operation.id) && operation.version === "1" && literal(operation.label) && literal(operation.success_meaning));
      assert(closed(operation.target, ["kind", "id"]) && operation.target.kind === "application.control" && id(operation.target.id));
      assert([operation.supported, operation.available, operation.authorized].every(value => typeof value === "boolean"));
      assert(closed(operation.input, ["profile", "label", "choices"]) && operation.input.profile === "enum-value.v1" && literal(operation.input.label) && operation.input.label.length > 0 && Array.isArray(operation.input.choices) && operation.input.choices.length > 0 && operation.input.choices.length <= 64);
      const choices = new Set();
      for (const choice of operation.input.choices) {
        assert(closed(choice, ["value", "label"]) && id(choice.value) && literal(choice.label) && !choices.has(choice.value));
        choices.add(choice.value);
      }
      assert(Array.isArray(operation.preconditions) && operation.preconditions.length === 3 && ["incarnation_id", "revision", "principal"].every(key => operation.preconditions.includes(key)));
      assert(closed(operation.recovery, ["lookup", "retention"]) && operation.recovery.lookup === "request_id" && operation.recovery.retention === "application_store_lifetime");
    }
    return data;
  }
  function appState(data, expectedID) {
    assert(closed(data, ["application", "online", "control", "activity", ...(Object.hasOwn(data || {}, "runtime") ? ["runtime"] : [])]));
    if (data.runtime !== undefined) assert(closed(data.runtime, ["profile", "process_key", "pid"]) && data.runtime.profile === "hale.iris.process.v1" && digestID(data.runtime.process_key) && revision(data.runtime.pid) && data.runtime.pid !== "0" && data.online === true);
    application(data.application);
    if (data.application.id !== expectedID) throw new Failure("identity_changed");
    assert(typeof data.online === "boolean");
    const control = data.control;
    assert(closed(control, ["kind", "id", "label", "revision", "value", "effective_revision", "effective_value"]) && control.kind === "application.control" && id(control.id) && literal(control.label) && revision(control.revision) && id(control.value));
    assert((control.effective_revision === "" && control.effective_value === "") || (revision(control.effective_revision) && id(control.effective_value)));
    assert(closed(data.activity, ["label", "count", "observation_tick"]) && literal(data.activity.label) && revision(data.activity.count) && revision(data.activity.observation_tick));
    return data;
  }
  function payloadFor(metadata, value) {
    return { request_id: metadata.request_id, operation: metadata.operation, operation_version: metadata.operation_version, context: { application_id: metadata.application_id }, target: { application_id: metadata.application_id, kind: metadata.target_kind, id: metadata.target_id }, preconditions: { incarnation_id: metadata.incarnation_id, revision: metadata.revision, principal: { mode: metadata.principal.mode, name: metadata.principal.name } }, arguments: { value } };
  }
  async function binding(payload) {
    const frame = [PROFILE, payload.preconditions.principal.mode, payload.preconditions.principal.name, payload.context.application_id, payload.request_id, payload.operation, payload.operation_version, payload.target.kind, payload.target.id, payload.preconditions.incarnation_id, payload.preconditions.revision, payload.arguments.value].join("\n");
    const result = await crypto.subtle.digest("SHA-256", encoder.encode(frame));
    return "sha256:" + Array.from(new Uint8Array(result), byte => byte.toString(16).padStart(2, "0")).join("");
  }
  function validMetadata(value, context) {
    return closed(value, ["version", "profile", "application_id", "principal", "request_id", "operation", "operation_version", "target_kind", "target_id", "incarnation_id", "revision", "request_binding_sha256"]) && value.version === 1 && value.profile === PROFILE && value.application_id === context.app.id && actor(value.principal) && sameActor(value.principal, context.principal) && id(value.request_id) && id(value.operation) && value.operation_version === "1" && value.target_kind === "application.control" && id(value.target_id) && digestID(value.incarnation_id) && revision(value.revision) && fingerprint(value.request_binding_sha256);
  }
  async function validateReceipt(data, metadata) {
    assert(closed(data, ["receipt"]));
    const receipt = data.receipt;
    assert(closed(receipt, ["command_id", "request_id", "application_id", "principal", "operation", "operation_version", "target", "preconditions", "arguments", "fingerprint", "state", "reason", "result"]));
    assert(id(receipt.command_id) && receipt.request_id === metadata.request_id && receipt.application_id === metadata.application_id && actor(receipt.principal) && sameActor(receipt.principal, metadata.principal) && receipt.operation === metadata.operation && receipt.operation_version === metadata.operation_version && fingerprint(receipt.fingerprint));
    assert(closed(receipt.target, ["application_id", "kind", "id"]) && receipt.target.application_id === metadata.application_id && receipt.target.kind === metadata.target_kind && receipt.target.id === metadata.target_id);
    assert(closed(receipt.preconditions, ["incarnation_id", "revision"]) && receipt.preconditions.incarnation_id === metadata.incarnation_id && receipt.preconditions.revision === metadata.revision);
    assert(closed(receipt.arguments, ["value"]) && id(receipt.arguments.value));
    assert(["succeeded", "refused"].includes(receipt.state) && literal(receipt.reason) && (receipt.state === "succeeded" ? receipt.reason === "" : receipt.reason !== ""));
    assert(closed(receipt.result, ["changed", "revision", "value"]) && typeof receipt.result.changed === "boolean" && revision(receipt.result.revision) && id(receipt.result.value));
    if (receipt.state === "succeeded") {
      assert(receipt.result.value === receipt.arguments.value);
      assert(receipt.result.revision === (BigInt(metadata.revision) + (receipt.result.changed ? 1n : 0n)).toString());
    } else assert(receipt.result.changed === false);
    const reconstructed = await binding(payloadFor(metadata, receipt.arguments.value));
    if (reconstructed !== metadata.request_binding_sha256 || reconstructed !== receipt.fingerprint) throw new Failure("request_binding_mismatch");
    return receipt;
  }
  function observedBinding(data, principal) {
    return data?.online && data.runtime ? { application: data.application, principal, runtime: data.runtime } : null;
  }
  async function runtimeBinding(read) {
    const discovered = await read(API);
    const data = envelope(discovered);
    assert(closed(data, ["items"]) && Array.isArray(data.items) && data.items.length <= 1);
    if (!data.items.length) return null;
    const app = application(data.items[0]);
    const state = appState(envelope(await read(API + "/" + app.id + "/state"), discovered.principal), app.id);
    if (state.application.incarnation_id !== app.incarnation_id) throw new Failure("identity_changed");
    return observedBinding(state, discovered.principal);
  }
  function mount(container, { onRuntime, onContext, expectedRuntime = null } = {}) {
    let disposed = false, generation = 0, timer = null, stateReading = false, busy = false;
    const controllers = new Set();
    let context = null, currentState = null, stateError = "", screenError = "", loading = true;
    let draft = null, stage = "idle", metadata = null, receipt = null, recoveryError = "", storageBlocked = false, absent = false, notice = "";
    const root = node("section", "application-instrument");
    root.setAttribute("role", "region");
    root.setAttribute("aria-label", "Application administration");
    const header = node("div", "application-header");
    const main = node("div", "application-main");
    const evidence = node("div", "application-evidence");
    const intervention = node("section", "panel application-intervention");
    intervention.setAttribute("aria-label", "Application change");
    intervention.setAttribute("role", "region");
    const recovery = node("section", "panel application-recovery");
    recovery.setAttribute("aria-label", "Command recovery");
    recovery.setAttribute("role", "region");
    append(main, evidence, intervention);
    append(root, header, recovery, main);
    container.replaceChildren(root);
    const valid = token => !disposed && token === generation && !document.hidden;
    const scope = () => context ? JSON.stringify([context.app.id, context.principal.mode, context.principal.name]) : "";
    const storageKey = currentScope => STORAGE + encodeURIComponent(currentScope);
    const operation = () => context?.capabilities?.operations[0] || null;
    function enabled() {
      const op = operation();
      return Boolean(context && currentState && currentState.online && op?.supported && op.available && op.authorized && op.target.kind === currentState.control.kind && op.target.id === currentState.control.id);
    }
    function announceContext() {
      onContext?.(context ? { principal: context.principal, name: context.app.name, commandEnabled: enabled() } : null);
    }
    function abortAll() {
      generation += 1;
      clearTimeout(timer);
      timer = null;
      controllers.forEach(controller => controller.abort());
      controllers.clear();
      stateReading = false;
      busy = false;
    }
    function clearPrivate() {
      context = currentState = draft = metadata = receipt = null;
      stage = "idle";
      stateError = recoveryError = notice = "";
      absent = storageBlocked = false;
      announceContext();
    }
    function accessLost() {
      abortAll();
      clearPrivate();
      loading = false;
      screenError = "Application identity or access changed. Current state, draft and receipt were cleared. Saved request identities remain with their original application and principal. Reload the application connection to verify access; no request will be resubmitted.";
      render();
    }
    function mustClear(error) {
      return [401, 403].includes(error.status) || ["identity_changed", "command_context_changed", "principal_changed", "application_identity_changed", "application_not_found"].includes(error.code);
    }
    async function request(path, token, method = "GET", payload = null) {
      const controller = new AbortController();
      controllers.add(controller);
      const timeout = setTimeout(() => controller.abort(), TIMEOUT);
      let reader;
      try {
        const response = await fetch(path, { method, signal: controller.signal, credentials: "same-origin", referrerPolicy: "no-referrer", redirect: "error", cache: "no-store", headers: { Accept: "application/json", ...(method === "POST" ? { "Content-Type": "application/json", "X-Iris-Command": "1" } : {}) }, ...(method === "POST" ? { body: JSON.stringify(payload) } : {}) });
        if ([401, 403].includes(response.status)) throw new Failure("access_denied", response.status);
        const length = response.headers.get("content-length");
        if (length && /^\d+$/.test(length) && Number(length) > RESPONSE_LIMIT) throw new Failure("response_limit");
        if (!response.body?.getReader) throw new Failure("response_unavailable");
        reader = response.body.getReader();
        const decoder = new TextDecoder("utf-8", { fatal: true });
        let size = 0;
        const pieces = [];
        while (true) {
          const part = await reader.read();
          if (part.done) break;
          size += part.value.byteLength;
          if (size > RESPONSE_LIMIT) throw new Failure("response_limit");
          pieces.push(decoder.decode(part.value, { stream: true }));
        }
        pieces.push(decoder.decode());
        if (!valid(token)) throw new Failure("stale_read");
        const body = JSON.parse(pieces.join(""));
        if (response.status === 409 && method === "POST" && Object.hasOwn(body || {}, "data")) {
          envelope(body, context?.principal, true);
          assert(closed(body.error, ["code", "message"]) && body.error.code === "command_refused" && literal(body.error.message));
          return { body, status: response.status };
        }
        if (response.status !== 200) throw new Failure(publicError(body), response.status);
        return { body, status: response.status };
      } finally {
        clearTimeout(timeout);
        controller.abort();
        if (reader) { await reader.cancel().catch(() => {}); reader.releaseLock(); }
        controllers.delete(controller);
      }
    }
    function restore() {
      try {
        const saved = localStorage.getItem(storageKey(scope()));
        if (saved === null) return;
        const value = JSON.parse(saved);
        if (!validMetadata(value, context)) throw new Error("invalid metadata");
        metadata = value;
        recoveryError = "A saved request may already have been delivered. Recovery reads its original identity; it never submits it again.";
      } catch {
        storageBlocked = true;
        recoveryError = "Recovery storage is unavailable or contains an unverifiable reservation. Submission is blocked so an unresolved request cannot be replaced.";
      }
    }
    async function readState(token) {
      if (!valid(token) || !context || stateReading) return;
      stateReading = true;
      const previous = currentState;
      try {
        const response = await request(API + "/" + context.app.id + "/state", token);
        if (!valid(token)) return;
        const next = appState(envelope(response.body, context.principal), context.app.id);
        if (draft && (next.application.incarnation_id !== draft.incarnation_id || next.control.revision !== draft.revision || next.control.id !== draft.target_id || !next.online)) {
          draft = null;
          stage = "idle";
          notice = "Application configuration, incarnation or availability changed. The prepared draft was cleared; inspect the fresh state before preparing another change.";
        }
        if (expectedRuntime && (next.application.id !== expectedRuntime.application.id || next.application.incarnation_id !== expectedRuntime.application.incarnation_id || !sameActor(context.principal, expectedRuntime.principal))) {
          notice = "The application or access changed since Runtime was inspected. These controls describe the newly verified application state; prepare any change again.";
          expectedRuntime = null;
        }
        currentState = next;
        stateError = "";
        announceContext();
        if (!previous || previous.runtime?.process_key !== next.runtime?.process_key) renderHeader();
        renderEvidence();
        if (!previous || previous.control.id !== next.control.id || previous.control.value !== next.control.value || previous.control.revision !== next.control.revision || previous.application.incarnation_id !== next.application.incarnation_id || previous.online !== next.online) renderIntervention();
      } catch (error) {
        if (!valid(token)) return;
        if (mustClear(error)) { accessLost(); return; }
        currentState = draft = null;
        stage = "idle";
        stateError = "Current application state is unavailable and has been cleared. A saved request can still be checked independently.";
        announceContext();
        renderEvidence();
        renderIntervention();
      } finally {
        if (valid(token)) {
          stateReading = false;
          clearTimeout(timer);
          timer = setTimeout(() => { timer = null; void readState(token); }, POLL_INTERVAL);
        }
      }
    }
    async function load() {
      abortAll();
      clearPrivate();
      loading = true;
      screenError = "";
      render();
      const token = generation;
      try {
        const discovered = await request(API, token);
        const data = envelope(discovered.body);
        assert(closed(data, ["items"]) && Array.isArray(data.items) && data.items.length <= 1);
        if (!valid(token)) return;
        if (!data.items.length) { loading = false; screenError = "No application is registered with this administration service."; render(); return; }
        const app = application(data.items[0]);
        const principal = discovered.body.principal;
        const supported = await request(API + "/" + app.id + "/capabilities", token);
        const caps = capabilities(envelope(supported.body, principal), app.id);
        if (!valid(token)) return;
        context = { app, principal, capabilities: caps };
        loading = false;
        restore();
        announceContext();
        render();
        if (metadata) void deliver("GET");
        await readState(token);
      } catch (error) {
        if (!valid(token)) return;
        if (mustClear(error)) { accessLost(); return; }
        clearPrivate();
        loading = false;
        screenError = "The registered application connection could not be verified. No state or command access is available; reload to check again.";
        render();
      }
    }
    function begin() {
      const op = operation();
      if (!enabled() || busy || metadata || storageBlocked) return;
      draft = { operation: op.id, operation_version: op.version, target_id: op.target.id, incarnation_id: currentState.application.incarnation_id, revision: currentState.control.revision, before: currentState.control.value, value: null };
      stage = "editing";
      notice = "";
      renderIntervention();
      root.querySelector(".application-choice")?.focus();
    }
    function validDraft() {
      const op = operation();
      return Boolean(draft && enabled() && !metadata && !storageBlocked && draft.operation === op.id && draft.operation_version === op.version && draft.target_id === op.target.id && draft.incarnation_id === currentState.application.incarnation_id && draft.revision === currentState.control.revision && op.input.choices.some(choice => choice.value === draft.value));
    }
    async function submit() {
      if (stage !== "reviewing" || busy || !validDraft()) return;
      if (typeof navigator.locks?.request !== "function" || typeof crypto.randomUUID !== "function" || !crypto.subtle?.digest) {
        notice = "This browser cannot safely reserve a recoverable request. Browser locks, secure request identities and cryptographic binding are required; nothing was submitted.";
        renderIntervention();
        return;
      }
      const token = generation, originalScope = scope(), prepared = draft;
      const stillPrepared = () => valid(token) && scope() === originalScope && draft === prepared && stage === "reviewing" && validDraft();
      let reserving = true;
      busy = true;
      renderIntervention();
      try {
        const reservation = await navigator.locks.request(storageKey(originalScope), { mode: "exclusive", ifAvailable: true }, async lock => {
          if (!lock) return { error: "Another tab is reserving a request. Nothing was submitted; check its request before continuing." };
          if (!stillPrepared()) return { stale: true };
          const key = storageKey(originalScope);
          if (localStorage.getItem(key) !== null) return { existing: true };
          const value = { version: 1, profile: PROFILE, application_id: context.app.id, principal: { ...context.principal }, request_id: crypto.randomUUID(), operation: prepared.operation, operation_version: prepared.operation_version, target_kind: "application.control", target_id: prepared.target_id, incarnation_id: prepared.incarnation_id, revision: prepared.revision, request_binding_sha256: "" };
          const payload = payloadFor(value, prepared.value);
          if (bytes(JSON.stringify(payload)) > 8192) return { error: "The encoded request exceeds 8192 bytes. Nothing was saved or submitted." };
          value.request_binding_sha256 = await binding(payload);
          if (!stillPrepared()) return { stale: true };
          const serialized = JSON.stringify(value);
          localStorage.setItem(key, serialized);
          if (localStorage.getItem(key) !== serialized) throw new Error("reservation not retained");
          return { metadata: value, payload };
        });
        if (!stillPrepared() || reservation.stale) {
          if (valid(token) && scope() === originalScope && reservation.metadata) {
            metadata = reservation.metadata;
            draft = null;
            stage = "idle";
            busy = false;
            reserving = false;
            render();
            void deliver("GET");
          }
          return;
        }
        busy = false;
        if (reservation.error) { notice = reservation.error; renderIntervention(); return; }
        if (reservation.existing) { draft = null; stage = "idle"; restore(); render(); reserving = false; if (metadata) void deliver("GET"); return; }
        metadata = reservation.metadata;
        draft = null;
        stage = "idle";
        reserving = false;
        await deliver("POST", reservation.payload);
      } catch {
        if (!valid(token) || scope() !== originalScope) return;
        busy = false;
        notice = "The recovery reservation could not be safely established. Nothing was submitted. Check browser storage before continuing.";
        renderIntervention();
      } finally {
        if (reserving && valid(token) && scope() === originalScope) { busy = false; renderIntervention(); }
      }
    }
    async function deliver(method, payload = null) {
      if (busy || !context || !metadata) return;
      const token = generation, originalScope = scope(), saved = metadata, previous = receipt;
      busy = true;
      receipt = null;
      absent = false;
      recoveryError = "";
      renderRecovery();
      renderIntervention();
      try {
        const path = API + "/" + saved.application_id + "/commands" + (method === "GET" ? "?" + new URLSearchParams({ request_id: saved.request_id }) : "");
        const response = await request(path, token, method, payload);
        if (!valid(token) || originalScope !== scope() || metadata !== saved) return;
        const data = envelope(response.body, context.principal, response.status === 409);
        const result = await validateReceipt(data, saved);
        if (!valid(token) || originalScope !== scope() || metadata !== saved) return;
        assert(method === "GET" ? response.status === 200 : response.status === (result.state === "refused" ? 409 : 200));
        if (previous) assert(previous.command_id === result.command_id && previous.fingerprint === result.fingerprint);
        receipt = result;
        void readState(token);
      } catch (error) {
        if (!valid(token) || originalScope !== scope() || metadata !== saved) return;
        if (mustClear(error)) { accessLost(); return; }
        absent = method === "GET" && error.status === 404 && error.code === "command_not_found";
        recoveryError = absent ? "The service authoritatively reports no receipt for this request at lookup time. This does not cancel an earlier in-flight delivery. You may explicitly release the reservation after considering that risk; no replacement request is created automatically." : error.code === "request_conflict" || error.code === "request_binding_mismatch" ? "This request identity conflicts with another payload or its receipt binding does not match. No receipt from that payload is displayed. Keep the reservation and resolve the conflict; no replacement request has been created." : "The request outcome could not be verified. Delivery may have occurred. Its reservation is retained; check the same request instead of submitting again.";
      } finally {
        payload = null;
        if (valid(token) && originalScope === scope() && metadata === saved) { busy = false; renderRecovery(); renderIntervention(); }
      }
    }
    async function release() {
      if (busy || !metadata || (!receipt && !absent) || typeof navigator.locks?.request !== "function") return;
      const token = generation, originalScope = scope(), saved = metadata, confirmed = receipt;
      busy = true;
      renderRecovery();
      try {
        const removed = await navigator.locks.request(storageKey(originalScope), { mode: "exclusive", ifAvailable: true }, lock => {
          if (!lock) throw new Error("reservation busy");
          if (!valid(token) || scope() !== originalScope || metadata !== saved || receipt !== confirmed) return false;
          const key = storageKey(originalScope), raw = localStorage.getItem(key);
          if (!raw || !validMetadata(JSON.parse(raw), context) || JSON.parse(raw).request_id !== saved.request_id || JSON.parse(raw).request_binding_sha256 !== saved.request_binding_sha256) throw new Error("reservation changed");
          localStorage.removeItem(key);
          if (localStorage.getItem(key) !== null) throw new Error("reservation retained");
          return true;
        });
        if (removed && valid(token) && scope() === originalScope) await load();
      } catch {
        if (valid(token) && scope() === originalScope) recoveryError = "The reservation could not be released. Its identity remains saved; no new request can replace it.";
      } finally {
        if (valid(token) && scope() === originalScope) { busy = false; renderRecovery(); }
      }
    }
    function facts(entries) {
      const list = node("dl", "fact-grid");
      entries.forEach(([label, value]) => list.append(append(node("div"), node("dt", "", label), node("dd", "", value))));
      return list;
    }
    function renderHeader() {
      header.replaceChildren();
      const title = append(node("div"), node("p", "eyebrow", "APPLICATION-OWNED CONTROL"), node("h2", "", context?.app.name || "Application connection"));
      const controls = append(node("div", "application-actions"), button("Refresh application", () => load()), button(currentState?.runtime ? "Inspect this running application" : "Open runtime observer", () => onRuntime?.(observedBinding(currentState, context?.principal))));
      header.append(title, controls);
      header.append(node("p", "application-boundary", "Change desired configuration, then observe what the application has applied."));
    }
    function renderEvidence() {
      const evidenceOpen = Boolean(evidence.querySelector("details")?.open);
      const evidenceFocused = document.activeElement === evidence.querySelector("summary");
      evidence.replaceChildren();
      if (loading || screenError || !context) {
        evidence.append(append(node("section", "panel application-empty"), node("h3", "", loading ? "Reading application connection" : "Application unavailable"), node("p", "", screenError || "Checking the registered application and authenticated capabilities.")));
        return;
      }
      if (!currentState) {
        evidence.append(append(node("section", "panel application-empty"), node("h3", "", "Current state unavailable"), node("p", "", stateError || "Reading the application-owned state. Saved request recovery is independent.")));
        return;
      }
      const data = currentState;
      const card = (title, className) => {
        const value = node("section", "panel application-state-card " + className);
        value.setAttribute("role", "region");
        value.setAttribute("aria-label", title);
        value.append(node("p", "eyebrow", title));
        return value;
      };
      const desired = card("Desired configuration", "application-desired");
      append(desired, node("h3", "", data.control.value || "Empty value"), facts([["Control", data.control.label], ["Desired revision", data.control.revision]]), node("p", "application-boundary", "Committed application configuration. Commit success does not prove that the app has observed or applied it."));
      const effective = card("App-effective configuration", "application-effective");
      append(effective, node("h3", "", data.control.effective_revision === "" ? "Unavailable" : data.control.effective_value || "Empty value"), facts([["Effective revision", data.control.effective_revision || "Unavailable"], ["Reported availability", data.online ? "Online · fresh app heartbeat" : "Offline · no fresh app heartbeat"]]), node("p", "application-boundary", data.online ? "App-reported configuration from the captured state read." : "The effective fields are the last recorded app observation. They do not establish a current live effect."));
      const activity = card("Application activity", "application-activity");
      append(activity, node("h3", "", data.activity.count), facts([["Activity", data.activity.label], ["Observation tick", data.activity.observation_tick]]), node("p", "application-boundary", "The tick is an opaque monotonic sample position, not wall-clock time or a derived rate."));
      evidence.append(desired, effective, activity, append(node("details", "application-source"), node("summary", "", "Application identity and evidence"), facts([["Application identity", data.application.id], ["Current incarnation", data.application.incarnation_id], ["Control identity", data.control.id], ["Principal", context.principal.mode + " / " + context.principal.name]]), node("p", "application-boundary", "Desired configuration, app-effective configuration and activity come from one captured application-owned read. Command receipts are separate immutable decisions. Runtime association is reported explicitly when this app is observed. It provides navigation evidence; control authority still belongs to the application.")));
      evidence.querySelector("details").open = evidenceOpen;
      if (evidenceFocused) evidence.querySelector("summary").focus({ preventScroll: true });
    }
    function renderIntervention() {
      intervention.replaceChildren();
      intervention.hidden = !context;
      if (!context) return;
      const op = operation();
      append(intervention, node("p", "eyebrow", "DELIBERATE CHANGE"), node("h3", "", op?.label || "Application commands unavailable"), node("p", "application-boundary", op?.success_meaning || "No supported command profile was advertised."));
      if (metadata || storageBlocked) {
        intervention.append(node("p", "application-warning", "Resolve the saved request reservation before preparing another change."));
        return;
      }
      if (!draft) {
        const prepare = button("Prepare change", begin);
        prepare.disabled = !enabled() || busy;
        intervention.append(prepare);
        if (!enabled()) intervention.append(node("p", "application-warning", "A current online app state and a supported, available, authorized operation for this control are required. Read access alone does not grant command authority."));
      } else if (stage === "editing") {
        const form = node("form");
        const label = node("label", "application-field", op.input.label);
        const input = node("select", "application-choice");
        input.setAttribute("aria-label", op.input.label);
        input.required = true;
        input.append(node("option", "", "Choose a value"));
        input.firstChild.value = "";
        op.input.choices.forEach((choice, index) => { const option = node("option", "", choice.label); option.value = String(index); option.selected = draft.value === choice.value; input.append(option); });
        input.addEventListener("change", () => { if (draft) draft.value = input.value === "" ? null : op.input.choices[Number(input.value)].value; });
        label.append(input);
        const review = node("button", "button", "Review change");
        review.type = "submit";
        form.addEventListener("submit", event => {
          event.preventDefault();
          if (!validDraft()) return;
          stage = "reviewing";
          renderIntervention();
          root.querySelector(".application-confirmation")?.focus();
        });
        append(form, label, node("p", "application-boundary", "The draft stays in this page only. Review the exact change before submitting."), append(node("div", "application-actions"), review, button("Discard draft", () => { draft = null; stage = "idle"; renderIntervention(); })));
        intervention.append(form);
      } else {
        const confirmation = node("div", "application-confirmation");
        confirmation.tabIndex = -1;
        confirmation.setAttribute("role", "group");
        confirmation.setAttribute("aria-label", "Application change confirmation");
        append(confirmation, facts([["Current desired value", draft.before || "Empty value"], ["Requested value", draft.value || "Empty value"], ["Expected revision", draft.revision], ["Expected incarnation", draft.incarnation_id], ["Acting principal", context.principal.name]]), node("p", "application-boundary", "This asks the application to commit the desired setting after checking the captured incarnation, revision and principal. App-effective state is observed separately."));
        const submitButton = button("Submit change", () => submit(), "button");
        const edit = button("Back to editing", () => { stage = "editing"; renderIntervention(); root.querySelector(".application-choice")?.focus(); });
        submitButton.disabled = edit.disabled = busy;
        append(intervention, confirmation, append(node("div", "application-actions"), submitButton, edit));
      }
      if (notice) { const note = node("p", "application-warning", notice); note.setAttribute("role", "status"); intervention.append(note); }
    }
    function renderRecovery() {
      recovery.replaceChildren();
      recovery.hidden = !metadata && !storageBlocked;
      recovery.dataset.state = receipt?.state || (absent ? "absent" : "unconfirmed");
      if (recovery.hidden) return;
      append(recovery, node("p", "eyebrow", "SAVED REQUEST / NO AUTOMATIC RESUBMISSION"), node("h3", "", "Command recovery"));
      const status = node("p", "application-command-status", busy ? "Checking or delivering the reserved request. Leaving this page cannot cancel a command already accepted by the app." : receipt ? "Command " + receipt.state : absent ? "No retained receipt found" : "Request outcome unconfirmed");
      status.setAttribute("role", "status");
      recovery.append(status);
      if (metadata) recovery.append(facts([["Request identity", metadata.request_id], ["Operation", metadata.operation], ["Target", metadata.target_id], ["Reserved principal", metadata.principal.name], ["Expected incarnation", metadata.incarnation_id], ["Expected revision", metadata.revision]]));
      if (receipt) {
        const result = receipt.result;
        recovery.append(node("h4", "application-result-heading", receipt.state === "succeeded" ? "Desired configuration committed" : "Application refused this change"));
        append(recovery, facts([["Command identity", receipt.command_id], ["Requested value", receipt.arguments.value || "Empty value"], ["Result revision", result.revision], ["Result desired value", result.value || "Empty value"], ["Changed", result.changed ? "Yes" : "No"]]), node("p", "application-boundary", receipt.state === "succeeded" ? "Desired configuration committed. This receipt does not establish that the running app has applied the setting; check App-effective configuration separately." : "The app retained a refusal. This request did not change the desired configuration."));
        if (receipt.reason) recovery.append(node("p", "application-warning", receipt.reason));
      }
      if (recoveryError) recovery.append(node("p", "application-warning", recoveryError));
      if (metadata) {
        const lookup = button("Check request status", () => deliver("GET"));
        lookup.disabled = busy;
        const controls = append(node("div", "application-actions"), lookup);
        if (receipt || absent) { const clear = button("Release request reservation", () => release()); clear.disabled = busy; controls.append(clear); }
        recovery.append(controls);
      }
    }
    function render() { if (!disposed) { renderHeader(); renderEvidence(); renderIntervention(); renderRecovery(); } }
    function pause() { abortAll(); clearPrivate(); loading = false; screenError = "Application data was cleared while this page was away. Access will be checked again before restoring the connection."; render(); }
    const visibility = () => { if (document.hidden) pause(); else void load(); };
    const pageShow = event => { if (event.persisted && !document.hidden) void load(); };
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", pause);
    window.addEventListener("pageshow", pageShow);
    if (!document.hidden) void load(); else pause();
    return { destroy() { disposed = true; abortAll(); clearPrivate(); document.removeEventListener("visibilitychange", visibility); window.removeEventListener("pagehide", pause); window.removeEventListener("pageshow", pageShow); container.replaceChildren(); } };
  }
  window.IrisApplication = Object.freeze({ mount, runtimeBinding });
})();
