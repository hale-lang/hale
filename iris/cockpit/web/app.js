/* Iris browser cockpit. Presentation only: the service owns state and authority.
 * Every Record value is rendered as text. No Record data enters browser storage.
 */
"use strict";

(() => {
  const API = "/api/hale/v1/applications";
  const LIMIT = 25;
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
  const VIEWS = new Set([...Object.keys(WORKSPACES), "runtime"]);
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
  let observerURL = "";
  let navigationFocus = null;
  let state = blankState(readRoute());

  function blankState(route) {
    return { route, phase: "loading", apps: [], app: null, capabilities: null, source: null, collection: null, detail: null, detailError: null, error: null, notice: "", inspectedAt: null };
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
    const panel = $(target === "detail" ? "record-detail-panel" : "record-list-panel");
    if (!panel) return;
    panel.focus({ preventScroll: true });
    panel.scrollIntoView({ block: "start", behavior: "auto" });
  }
  function backToList() {
    return navigationLink(WORKSPACES[state.route.view].back, routeHash({ ...state.route, id: "", offset: state.collection.page.offset, snapshot: state.collection.page.snapshot }), "list", "text-link detail-back");
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
    return { view: VIEWS.has(view) ? view : "practices", app: query.get("app") || "", id: query.get("id") || "", offset, snapshot: query.get("snapshot") || "", scope: query.get("scope") === "all" ? "all" : "positions" };
  }
  function routeHash(route) {
    if (route.view === "runtime") return "#/runtime";
    const query = new URLSearchParams();
    if (route.app) query.set("app", route.app);
    if (route.id) query.set("id", route.id);
    if (route.offset) query.set("offset", String(route.offset));
    if (route.snapshot) query.set("snapshot", route.snapshot);
    if (route.view === "organization" && route.scope === "all") query.set("scope", "all");
    return "#/" + route.view + (query.size ? "?" + query.toString() : "");
  }
  function replaceRoute(route) {
    history.replaceState(null, "", routeHash(route));
    state.route = route;
  }
  function navigate(route) {
    const hash = routeHash(route);
    if (location.hash === hash) loadRoute(route);
    else location.hash = hash;
  }
  function relatedRoute(view, id) {
    const offset = view === "organization" && state.route.view === view ? state.collection.page.offset : 0;
    return routeHash({ view, app: state.app.id, id, offset, snapshot: state.collection.page.snapshot, scope: state.route.scope });
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
    let response;
    try { response = await fetch(path, { signal, credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" } }); }
    catch (error) {
      if (error.name === "AbortError") throw error;
      throw new ReadError(0, "connection_failed", "The local service could not be reached. Check that it is running, then retry.");
    }
    if (response.status === 401) throw new ReadError(401, "unauthenticated", "The service needs a valid session before it can return Record data.");
    let body;
    try { body = await response.json(); }
    catch { throw new ReadError(response.status, "invalid_response", "The service did not return a readable JSON response."); }
    if (!response.ok) throw new ReadError(response.status, body.error?.code || "request_failed", typeof body.error?.message === "string" ? body.error.message : "The read did not complete.");
    assert(body && body.api_version === "hale.v1" && body.data);
    validSource(body.source);
    return body;
  }
  function ensureCurrent(token, signal) {
    if (token !== generation || signal.aborted) throw new DOMException("Superseded read", "AbortError");
  }
  async function readApplication(route, token, signal) {
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
    assert(capabilities.application_id === app.id && capabilities.read_only === true && capabilities.principal && typeof capabilities.principal.name === "string" && ["local", "oidc"].includes(capabilities.principal.mode) && capabilities.reads);
    if (capabilities.reads[workspace.capability] !== true) throw new ReadError(0, "unsupported_capability", "This connection does not advertise the requested read capability.");
    route = { ...route, app: app.id };
    if (route.offset > 0 && !route.snapshot) {
      route = { ...route, offset: 0 };
      state.notice = "This page link has no snapshot. Pagination restarted from the first page.";
    }
    const query = new URLSearchParams({ limit: String(LIMIT), offset: String(route.offset) });
    if (route.snapshot) query.set("snapshot", route.snapshot);
    const collectionResponse = await request(base + "/dna/" + workspace.resource + "?" + query, signal);
    ensureCurrent(token, signal);
    validSource(collectionResponse.source, app.id);
    validPage(collectionResponse.data, collectionResponse.source, workspace);
    if (route.view === "organization") validOrganization(collectionResponse.data);
    validRows(collectionResponse.data.items, route.view);
    assert(collectionResponse.data.page.offset === route.offset, "The service returned a different page than requested.");
    const collection = collectionResponse.data;
    route = { ...route, snapshot: collection.page.snapshot };
    let detail = null, detailError = null;
    if (route.id) {
      const detailQuery = new URLSearchParams({ id: route.id, snapshot: collection.page.snapshot });
      try {
        const response = await request(base + "/dna/" + workspace.resource + "?" + detailQuery, signal);
        ensureCurrent(token, signal);
        validSource(response.source, app.id);
        validPage(response.data, response.source, workspace);
        if (route.view === "organization") {
          validOrganization(response.data);
          assert(sameOrganizationBasis(response.data.basis, collection.basis), "The organization detail has a different source basis. No mixed version is displayed.");
        }
        validRows(response.data.items, route.view);
        assert(response.source.record_head === collectionResponse.source.record_head && response.data.page.snapshot === collection.page.snapshot && response.data.items.length === 1 && response.data.items[0].id === route.id, "The detail does not match the selected object and source snapshot.");
        detail = response.data.items[0];
      } catch (error) {
        if (error.status !== 404 || error.code === "application_not_found") throw error;
        detailError = error;
      }
    }
    return { apps, app, capabilities, source: collectionResponse.source, collection, detail, detailError, route };
  }
  function cancel() {
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
    if (route.view === "runtime") {
      state.phase = "ready";
      render();
      return;
    }
    render();
    // A moving snapshot gets one automatic reset. Continuing movement is an
    // explicit error with a manual retry, never an unbounded request loop.
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        const loaded = await readApplication(route, token, signal);
        ensureCurrent(token, signal);
        state = { ...state, ...loaded, phase: "ready", inspectedAt: new Date(), error: null };
        if (loaded.route) replaceRoute(loaded.route);
        render();
        if (focusTarget) focusPanel(focusTarget);
        ui.announcement.textContent = route.id ? "Selected record loaded." : "Record page loaded.";
        return;
      } catch (error) {
        if (error.name === "AbortError" || token !== generation || signal.aborted) return;
        if (error.status === 409 && attempt === 0) {
          route = { ...route, offset: 0, snapshot: "" };
          state = blankState(route);
          state.notice = route.view === "organization" ? "The organization source, dependencies, or Record changed. Pagination restarted from the first page." : "The Record changed. Pagination restarted from the first page.";
          replaceRoute(route);
          render();
          continue;
        }
        const notice = state.notice;
        state = { ...blankState(route), phase: "error", error, notice };
        render();
        ui.announcement.textContent = error.status === 401 ? "Sign in required. Record data cleared." : "The read did not complete. Record data cleared.";
        return;
      }
    }
  }
  function refresh() {
    const route = { ...readRoute(), offset: 0, snapshot: "" };
    replaceRoute(route);
    loadRoute(route);
  }
  function render() {
    const runtime = state.route.view === "runtime";
    const workspace = WORKSPACES[state.route.view];
    const title = runtime ? "Runtime" : workspace.title;
    document.title = title + " · Iris";
    ui["workspace-title"].textContent = title;
    ui["breadcrumb-current"].textContent = title;
    ui["workspace-kicker"].textContent = runtime ? "THE RUNNING SYSTEM" : workspace.kicker;
    ui["workspace-description"].textContent = runtime ? "Observe any running Hale system, independently of its application model." : workspace.description;
    ui.refresh.hidden = runtime;
    ui.content.setAttribute("aria-busy", String(state.phase === "loading"));
    ui.notice.hidden = !state.notice;
    ui.notice.textContent = state.notice;
    for (const view of VIEWS) {
      const nav = $("nav-" + view);
      if (view === state.route.view) nav.setAttribute("aria-current", "page");
      else nav.removeAttribute("aria-current");
      nav.href = routeHash({ view, app: state.app?.id || state.route.app, id: "", offset: 0, snapshot: "" });
    }
    renderConnection(runtime);
    renderSource();
    ui["workspace-footer"].replaceChildren(node("span", "", runtime ? "Hale · runtime observer" : "Hale API · v1"), node("span", "", runtime ? "Runtime evidence and application state have separate sources." : state.route.view === "organization" ? "Source, dependencies, and Record are pinned separately. No changes are made here." : "State is read from the local Record. No changes are made here."));
    if (runtime) ui.content.replaceChildren(renderRuntime());
    else if (state.phase === "loading") ui.content.replaceChildren(stateCard(state.route.view === "organization" ? "Reading the organization source" : "Reading the Record", state.route.view === "organization" ? "Checking this page against its committed source, captured dependencies, and Record snapshot. Previously displayed content has been cleared." : "Loading this page and its source snapshot. Previously displayed content has been cleared.", "◌"));
    else if (state.error) ui.content.replaceChildren(errorCard(state.error));
    else if (!state.app) ui.content.replaceChildren(stateCard("No applications available", "This service has not returned an accessible application.", "◇", [button("Retry", refresh)]));
    else ui.content.replaceChildren(renderCatalog());
  }
  function renderConnection(runtime) {
    const select = ui.application;
    select.replaceChildren();
    if (state.apps.length && !runtime) {
      for (const app of state.apps) {
        const option = node("option", "", display(app.name, short(app.id)));
        option.value = app.id;
        option.selected = app.id === state.app?.id;
        select.append(option);
      }
      select.disabled = false;
    } else {
      select.append(node("option", "", runtime ? "Runtime connection" : state.phase === "loading" ? "Connecting…" : "No Record connected"));
      select.disabled = true;
    }
    ui["connection-caption"].textContent = runtime ? "No DNA connection required" : state.app ? "DNA · local Record" : state.phase === "loading" ? "Reading the local service" : "Application data unavailable";
    const principal = state.capabilities?.principal;
    ui.principal.textContent = runtime ? "Independent observer" : principal ? (principal.mode === "oidc" ? "Signed in · " : "Local · ") + principal.name : state.error?.status === 401 ? "Sign in required" : state.phase === "loading" ? "Connecting" : "Not connected";
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
    if (state.collection?.basis) {
      const basis = state.collection.basis;
      const dependencyOrigin = { local_vendor_snapshot: "Captured local vendor files", committed_source: "Committed vendor files", none: "No vendor files" };
      for (const [label, value] of [["Source", basis.source_head], ["Seed", basis.seed], ["Dependencies", basis.dependency_digest], ["Dependency origin", dependencyOrigin[basis.dependency_source]], ["Artifact", basis.artifact_digest], ["Shape", basis.shape_hash], ["Schema", basis.schema], ["Semantics", basis.semantics]]) append(identities, append(node("div"), node("span", "", label), node("code", "", value)));
      summary.append(append(node("span"), document.createTextNode("Source "), node("strong", "mono", short(basis.source_head, 10))));
      identities.append(node("p", "detail-note", "Organization declarations come from the committed source revision. Uncommitted declaration edits are not included."));
      identities.append(node("p", "detail-note", basis.dependency_source === "local_vendor_snapshot" ? "Dependencies were captured from local vendor files, separately from the source commit. Their digest identifies the bytes used for this inspection. This read does not verify them against a dependency lockfile." : basis.dependency_source === "committed_source" ? "Dependencies came from committed vendor files. Their digest identifies the bytes used for this inspection." : "This inspection used no vendor files."));
    }
    identities.append(node("p", "detail-note", "This is the inspected local snapshot. Remote synchronization and Ledger freshness are not established by this read."));
    details.append(identities);
    append(ui.source, summary, details);
  }
  function badge(label, tone = "") { return node("span", "badge " + tone, label); }
  function practiceBadge(practice) {
    const tones = { pending: "amber", ratified: "green", declined: "red", retired: "", refused: "red" };
    return badge(PRACTICE_STATES[practice.state] || "Unknown · " + practice.state, tones[practice.state] || "");
  }
  function reviewBadge(review) {
    return badge(display(review.state), review.state === "pending" ? "amber" : "");
  }
  function practiceMeta(item) {
    return append(node("div", "record-meta"), node("div", "", "Proposed by " + display(item.requester, "unrecorded requester")), node("code", "", short(item.digest, 27)));
  }
  function reviewMeta(item) {
    return append(node("div", "record-meta"), node("div", "", "Authority · " + display(item.required_authority)), node("div", "", item.outcome ? "Decision · " + (OUTCOMES[item.outcome] || "Unknown · " + item.outcome) : "No decision recorded"));
  }
  function organizationBadge(item) { return badge(item.role === "position" ? "Declared position" : "Structure", item.role === "position" ? "green" : ""); }
  function organizationMeta(item) {
    return append(node("div", "record-meta"), node("div", "", "Declaration · " + display(item.declaration)), node("div", "", "Static instance · " + display(item.thread_domain, "thread domain not specified")));
  }
  function recordLink(item) {
    const workspace = WORKSPACES[state.route.view];
    const href = routeHash({ ...state.route, id: item.id, snapshot: state.collection.page.snapshot, offset: state.collection.page.offset });
    const a = navigationLink("", href, "detail", "record-link");
    a.setAttribute("aria-label", workspace.rowName(item));
    if (state.route.id === item.id) a.setAttribute("aria-current", "true");
    return append(a, append(node("div", "record-line"), node("span", "record-name", workspace.rowName(item)), workspace.badge(item)), workspace.rowMeta(item));
  }
  function organizationOutline() {
    const { items, page, basis } = state.collection;
    const positionsOnly = basis.position_group_declared && state.route.scope !== "all";
    const rows = positionsOnly ? items.filter((item) => item.in_position_outline) : items;
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
    fragment.append(controls);
    const explanation = !basis.position_group_declared ? "No positions group is declared in this source. Showing all static structure." : positionsOnly ? "Explicitly declared positions and their containment ancestors." : "All static instances, including structure outside the positions group.";
    fragment.append(node("p", "outline-note", explanation));
    if (page.total > items.length) fragment.append(node("p", "outline-note", "This outline covers the current page. A parent on another page is shown as a reference, not as a new root position."));
    if (!rows.length) {
      fragment.append(stateCard("No positions on this page", "This page contains no declared positions or their ancestors. Choose All structure or continue to another page.", "◇", [], "compact"));
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
      if (item.parent_id && !byID.has(item.parent_id)) {
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
  function renderCatalog() {
    const workspace = WORKSPACES[state.route.view];
    const collection = state.collection;
    const catalog = node("div", "catalog");
    const listPanel = node("section", "panel");
    listPanel.id = "record-list-panel";
    listPanel.tabIndex = -1;
    listPanel.setAttribute("aria-label", workspace.register);
    append(listPanel, append(node("header", "panel-heading"), node("h2", "", workspace.register), node("span", "", collection.page.total + (state.route.view === "organization" ? " instances" : " recorded"))));
    if (!collection.items.length) listPanel.append(stateCard(workspace.empty, collection.page.total === 0 ? workspace.emptyDescription : "There are no records on this page. Return to the first page to continue.", "◇", collection.page.total ? [button("First page", refresh)] : [], "compact"));
    else if (state.route.view === "organization") listPanel.append(organizationOutline());
    else {
      const list = node("ul", "record-list");
      for (const item of collection.items) {
        list.append(append(node("li"), recordLink(item)));
      }
      listPanel.append(list);
    }
    const page = collection.page;
    const pagination = node("div", "pagination");
    const previous = button("Previous page", () => navigate({ ...state.route, id: "", offset: Math.max(0, page.offset - page.limit), snapshot: page.snapshot }));
    previous.disabled = page.offset === 0;
    const next = button("Next page", () => navigate({ ...state.route, id: "", offset: page.next_offset, snapshot: page.snapshot }));
    next.disabled = page.next_offset < 0;
    const shown = state.route.view === "organization" && collection.basis.position_group_declared && state.route.scope !== "all" ? collection.items.filter((item) => item.in_position_outline).length : collection.items.length;
    const pageLabel = state.route.view === "organization" ? shown + " shown · " + collection.items.length + " on page" : collection.items.length ? (page.offset + 1) + "–" + (page.offset + collection.items.length) + " of " + page.total : "0 shown";
    append(pagination, previous, node("span", "pagination-label", pageLabel), next);
    listPanel.append(pagination);
    const detailPanel = node("section", "panel");
    detailPanel.id = "record-detail-panel";
    detailPanel.tabIndex = -1;
    detailPanel.setAttribute("aria-label", workspace.detail);
    if (state.detailError) detailPanel.append(errorCard(state.detailError, true));
    else if (state.detail) detailPanel.append(workspace.inspector(state.detail));
    else {
      detailPanel.classList.add("selection-prompt");
      detailPanel.append(stateCard(workspace.selection, workspace.selectionDescription, "◇", [], "compact"));
    }
    append(catalog, listPanel, detailPanel);
    if (state.route.view === "organization") {
      const frame = node("div", "organization-workspace");
      const basis = collection.basis;
      const context = append(node("section", "viewing-context"), append(node("div"), node("span", "eyebrow", "Viewing context"), node("strong", "", state.detail ? state.detail.id : "Declared organization")), node("p", "", "Inspection only. Selecting a position does not change your identity or grant its authority."));
      context.setAttribute("aria-label", "Viewing context");
      append(frame, context, catalog);
      const coverage = append(node("section", "panel organization-coverage"), node("h2", "", "What this source establishes"), node("p", "", basis.declaration_count + " declarations · " + collection.page.total + " static instances · " + basis.uninstantiated_declaration_count + " declarations without static instances."), node("p", "", "Static instances describe source structure, not running occupants or vacancies. A declaration with no static instance is not presented as a vacant position."), node("p", "", "Static containment coverage · " + (basis.exact_ownership ? "exact" : "partial")));
      frame.append(coverage, organizationOwnership(collection.ownership));
      return frame;
    }
    return catalog;
  }
  function organizationDetail(item) {
    const detail = node("article", "detail");
    append(detail, backToList(), append(node("div", "detail-kicker"), node("span", "eyebrow", "Source-declared instance"), organizationBadge(item)), node("h2", "", item.id), node("p", "detail-subtitle", "Declaration · " + item.declaration), node("hr", "detail-rule"));
    if (!state.collection.items.some((row) => row.id === item.id)) detail.append(node("p", "detail-note", "This instance is on another page. The outline remains on the page you were inspecting; Back to organization returns to that page."));
    else if (state.collection.basis.position_group_declared && state.route.scope !== "all" && !item.in_position_outline) detail.append(node("p", "detail-note", "This instance is outside the declared-position outline. Choose All structure to include it on this page."));
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
    append(columns, section("Assignments", declaredTable(["Position name", "Owner"], ownership.positions.map((p) => [p.position, p.owner]), "No explicit position assignments declared.")), section("Memberships", declaredTable(["Owner", "Members"], ownership.memberships.map((m) => [m.owner, m.members.join(", ") || "None declared"]), "No explicit memberships declared.")));
    panel.append(columns);
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
    const detail = node("article", "detail");
    const readable = practiceTextAvailable(p);
    append(detail,
      backToList(),
      append(node("div", "detail-kicker"), node("span", "eyebrow", "Practice document"), practiceBadge(p)),
      node("h2", "", display(p.name, p.id)),
      node("p", "detail-subtitle", "Target · " + display(p.target) + "   /   " + display(p.binding_class, "No binding class")),
      node("hr", "detail-rule"),
      section("Canonical text", readable ? documentText(p.text) : availableNotice(p.text_status))
    );
    const lifecycle = node("dl", "fact-grid");
    fact(lifecycle, "Practice state", PRACTICE_STATES[p.state] || "Unknown · " + p.state);
    fact(lifecycle, "Review decision", p.review_outcome ? OUTCOMES[p.review_outcome] || "Unknown · " + p.review_outcome : "No decision recorded");
    const lifeSection = section("State & decision", lifecycle);
    lifeSection.append(node("p", "detail-note", "An approved Review is a recorded decision. Practice ratification and retirement are separate facts."));
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
    if (p.supersedes) detail.append(section("Version lineage", append(node("div", "linked-record"), append(node("div"), node("div", "linked-label", "Supersedes"), node("p", "mono", p.supersedes)), navigationLink("View predecessor", relatedRoute("practices", p.supersedes), "detail"))));
    const identities = node("dl", "fact-grid");
    fact(identities, "Document digest", p.digest, true, true);
    fact(identities, "Practice identity", p.id, true, true);
    fact(identities, "Request identity", p.request_id, true, true);
    detail.append(section("Record references", identities));
    return detail;
  }
  function reviewDetail(r) {
    const detail = node("article", "detail");
    const readable = reviewTextAvailable(r);
    append(detail,
      backToList(),
      append(node("div", "detail-kicker"), node("span", "eyebrow", "Recorded Review"), reviewBadge(r)),
      node("h2", "", r.id),
      node("p", "detail-subtitle", "Required authority · " + display(r.required_authority)),
      node("hr", "detail-rule"),
      section("Question under review", readable ? documentText(r.question) : availableNotice(r.text_status))
    );
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
    if (error.status === 401) {
      title = "Sign in to read this application";
      description = "Your session is missing or has expired. Application data has been cleared. Runtime observation remains available independently.";
      actions.push(link("Sign in", "/auth/login", "button"));
    } else if (error.status === 404) {
      title = error.code === "application_not_found" ? "Application not found" : detail ? WORKSPACES[state.route.view].singular + " not found" : "Read endpoint not found";
      if (detail) description = state.route.view === "organization" ? "The exact instance in this link is absent from the inspected organization source. It has not been replaced with another instance." : "The exact identifier in this link is absent from the inspected Record snapshot. It has not been replaced with another object.";
      if (error.code === "application_not_found") actions.push(button("Discover application", () => navigate({ ...state.route, app: "", id: "", offset: 0, snapshot: "" })));
    } else if (error.status === 503) title = error.code?.startsWith("organization_") ? "Organization source unavailable" : "Record unavailable";
    else if (error.status === 409) {
      title = state.route.view === "organization" ? "Organization source is changing" : "Record is changing";
      description = "The snapshot changed again after one automatic restart. Retry when the source settles; no mixed snapshot is displayed.";
    } else if (error.code === "connection_failed") title = "Service unreachable";
    else if (error.code === "invalid_response") title = "Response could not be verified";
    else if (error.code === "unsupported_capability") title = "Read capability unavailable";
    actions.push(button("Retry", refresh));
    if (!detail) actions.push(link("Open Runtime", "#/runtime"));
    const card = stateCard(title, description, error.status === 401 ? "◇" : "!", actions, detail ? "compact" : "");
    if (detail) card.prepend(backToList());
    return card;
  }
  function safeObserverURL(value) {
    const url = new URL(value);
    if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error("unsafe observer URL");
    return url.href;
  }
  function renderRuntime() {
    const layout = node("div", "runtime-layout");
    const hero = node("section", "panel runtime-hero");
    const glyph = node("div", "runtime-glyph");
    glyph.setAttribute("aria-hidden", "true");
    append(glyph, node("span", "runtime-node"), node("span", "runtime-edge"), node("span", "runtime-node center"), node("span", "runtime-edge"), node("span", "runtime-node"));
    append(hero, glyph, node("h2", "", "A window into the running system."), node("p", "", "Open an existing Hale observer to inspect processes, loci, message flow, and witnessed behavior. Any Hale application can be observed; a DNA Record is not required."));
    const form = node("form", "observer-form");
    const label = node("label", "", "Observer URL");
    label.htmlFor = "observer-url";
    const input = node("input");
    input.id = "observer-url";
    input.type = "url";
    input.required = true;
    input.value = observerURL || "http://127.0.0.1:8787/";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.setAttribute("aria-describedby", "observer-hint observer-error");
    const submit = node("button", "button", "Connect observer");
    submit.type = "submit";
    const hint = node("p", "field-hint", "Use the URL of your existing observer. This prepares a link; it does not start a process or test the connection.");
    hint.id = "observer-hint";
    const error = node("p", "field-error");
    error.id = "observer-error";
    error.setAttribute("role", "alert");
    error.hidden = true;
    const ready = node("div", "observer-ready");
    ready.hidden = !observerURL;
    function showLink(url) {
      const open = link("Open runtime observer", url, "button secondary");
      open.target = "_blank";
      open.rel = "noopener noreferrer";
      ready.replaceChildren(open, node("p", "", "Opens in a separate tab. Runtime evidence is not joined to this Record, and this cockpit does not verify the observer's coverage or freshness."));
      ready.hidden = false;
    }
    if (observerURL) showLink(observerURL);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      try {
        observerURL = safeObserverURL(input.value.trim());
        error.hidden = true;
        input.removeAttribute("aria-invalid");
        showLink(observerURL);
      } catch {
        observerURL = "";
        ready.replaceChildren();
        ready.hidden = true;
        error.textContent = "Enter an absolute http:// or https:// observer URL without credentials.";
        error.hidden = false;
        input.setAttribute("aria-invalid", "true");
      }
    });
    append(form, label, append(node("div", "input-row"), input, submit), hint, error, ready);
    hero.append(form);
    const aside = node("section", "panel runtime-aside");
    aside.append(node("h2", "", "The existing Iris instrument"));
    for (const [title, text] of [["Structure", "Explore the observed locus tower and how running processes connect."], ["Flow", "Inspect message paths, throughput, queue depth, latency, and loss where observations are available."], ["Evidence", "Compare declared structure with witnessed behavior. Missing observations are not proof of completion."]]) aside.append(append(node("div", "instrument"), node("h3", "", title), node("p", "", text)));
    append(layout, hero, aside);
    return layout;
  }
  function pause() {
    if (state.route.view === "runtime") return;
    cancel();
    const route = state.route;
    state = blankState(route);
    state.phase = "paused";
    renderConnection(false);
    renderSource();
    ui.content.setAttribute("aria-busy", "false");
    ui.content.replaceChildren(stateCard("View paused", "Record content was cleared while this page was away. It will be rechecked when you return.", "◌"));
  }
  ui.refresh.addEventListener("click", refresh);
  document.querySelector(".skip-link").addEventListener("click", (event) => {
    event.preventDefault();
    $("main").focus();
  });
  ui.application.addEventListener("change", () => navigate({ ...state.route, app: ui.application.value, id: "", offset: 0, snapshot: "" }));
  ui["sign-out"].addEventListener("click", pause);
  window.addEventListener("hashchange", () => {
    const route = readRoute();
    const selectedChanged = route.id && (route.id !== state.route.id || route.view !== state.route.view || route.app !== state.route.app);
    const focusTarget = navigationFocus || (selectedChanged ? "detail" : !route.id && state.route.id ? "list" : null);
    navigationFocus = null;
    loadRoute(route, "", focusTarget);
  });
  window.addEventListener("pagehide", pause);
  window.addEventListener("pageshow", (event) => { if (event.persisted) refresh(); });
  document.addEventListener("visibilitychange", () => {
    if (state.route.view === "runtime") return;
    if (document.hidden) pause();
    else refresh();
  });
  window.addEventListener("focus", () => {
    if (state.route.view !== "runtime" && !document.hidden && Date.now() - lastStarted > 1000) refresh();
  });
  replaceRoute(state.route);
  loadRoute(state.route);
})();
