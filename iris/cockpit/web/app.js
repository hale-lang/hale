/* Iris browser cockpit. Presentation only: the service owns state and authority.
 * Every Record value is rendered as text. No Record data enters browser storage.
 */
"use strict";

(() => {
  const API = "/api/hale/v1/applications";
  const LIMIT = 25;
  const VIEWS = new Set(["practices", "reviews", "runtime"]);
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
    return navigationLink(state.route.view === "practices" ? "Back to practices" : "Back to reviews", routeHash({ ...state.route, id: "", offset: state.collection.page.offset, snapshot: state.collection.page.snapshot }), "list", "text-link detail-back");
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
    return { view: VIEWS.has(view) ? view : "practices", app: query.get("app") || "", id: query.get("id") || "", offset, snapshot: query.get("snapshot") || "" };
  }
  function routeHash(route) {
    if (route.view === "runtime") return "#/runtime";
    const query = new URLSearchParams();
    if (route.app) query.set("app", route.app);
    if (route.id) query.set("id", route.id);
    if (route.offset) query.set("offset", String(route.offset));
    if (route.snapshot) query.set("snapshot", route.snapshot);
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
    return routeHash({ view, app: state.app.id, id, offset: 0, snapshot: state.source.record_head });
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
  function validPage(data, source) {
    assert(data && Array.isArray(data.items) && data.page);
    const page = data.page;
    assert(Number.isSafeInteger(page.limit) && page.limit >= 1 && page.limit <= 100 && Number.isSafeInteger(page.offset) && page.offset >= 0 && page.offset <= 1000000 && Number.isSafeInteger(page.total) && page.total >= 0 && Number.isSafeInteger(page.next_offset) && page.next_offset >= -1 && page.next_offset <= 1000000 && page.snapshot === source.record_head);
    assert(page.next_offset === -1 || page.next_offset > page.offset, "The service returned an invalid page continuation.");
  }
  function validRows(items, view) {
    const fields = view === "practices"
      ? ["id", "digest", "name", "kind", "text", "text_status", "author", "target", "binding_class", "provenance", "supersedes", "request_id", "review_id", "review_state", "review_settled", "review_outcome", "state", "requester", "rationale"]
      : ["id", "state", "subject_digest", "required_authority", "settled", "outcome", "knowledge_digest", "text_status", "question"];
    for (const item of items) {
      assert(item && fields.every((key) => typeof item[key] === "string") && item.id.length && typeof item.text_available === "boolean");
      if (view === "practices") assert(["ratified", "retired", "declined"].every((key) => typeof item[key] === "boolean") && item.digest.length);
    }
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
    assert(capabilities.application_id === app.id && capabilities.read_only === true && capabilities.principal && typeof capabilities.principal.name === "string" && ["local", "oidc"].includes(capabilities.principal.mode) && capabilities.reads);
    if (capabilities.reads[route.view] !== true) throw new ReadError(0, "unsupported_capability", "This connection does not advertise the requested read capability.");
    route = { ...route, app: app.id };
    if (route.offset > 0 && !route.snapshot) {
      route = { ...route, offset: 0 };
      state.notice = "This page link has no snapshot. Pagination restarted from the first page.";
    }
    const query = new URLSearchParams({ limit: String(LIMIT), offset: String(route.offset) });
    if (route.snapshot) query.set("snapshot", route.snapshot);
    const collectionResponse = await request(base + "/dna/" + route.view + "?" + query, signal);
    ensureCurrent(token, signal);
    validSource(collectionResponse.source, app.id);
    validPage(collectionResponse.data, collectionResponse.source);
    validRows(collectionResponse.data.items, route.view);
    assert(collectionResponse.data.page.offset === route.offset, "The service returned a different page than requested.");
    const collection = collectionResponse.data;
    route = { ...route, snapshot: collection.page.snapshot };
    let detail = null, detailError = null;
    if (route.id) {
      const detailQuery = new URLSearchParams({ id: route.id, snapshot: collection.page.snapshot });
      try {
        const response = await request(base + "/dna/" + route.view + "?" + detailQuery, signal);
        ensureCurrent(token, signal);
        validSource(response.source, app.id);
        validPage(response.data, response.source);
        validRows(response.data.items, route.view);
        assert(response.source.record_head === collectionResponse.source.record_head && response.data.items.length === 1 && response.data.items[0].id === route.id, "The detail does not match the selected object and Record snapshot.");
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
          state.notice = "The Record changed. Pagination restarted from the first page.";
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
    const title = runtime ? "Runtime" : state.route.view === "reviews" ? "Reviews" : "Practices";
    document.title = title + " · Iris";
    ui["workspace-title"].textContent = title;
    ui["breadcrumb-current"].textContent = title;
    ui["workspace-kicker"].textContent = runtime ? "THE RUNNING SYSTEM" : state.route.view === "reviews" ? "DECISIONS & AUTHORITY" : "ORGANIZATIONAL MEMORY";
    ui["workspace-description"].textContent = runtime ? "Observe any running Hale system, independently of its application model." : state.route.view === "reviews" ? "Inspect the exact subject, required authority, and recorded decision." : "The agreements that guide this application, and the decisions behind them.";
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
    ui["workspace-footer"].replaceChildren(node("span", "", runtime ? "Hale · runtime observer" : "Hale API · v1"), node("span", "", runtime ? "Runtime evidence and application state have separate sources." : "State is read from the local Record. No changes are made here."));
    if (runtime) ui.content.replaceChildren(renderRuntime());
    else if (state.phase === "loading") ui.content.replaceChildren(stateCard("Reading the Record", "Loading this page and its source snapshot. Previously displayed content has been cleared.", "◌"));
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
  function renderCatalog() {
    const practice = state.route.view === "practices";
    const collection = state.collection;
    const catalog = node("div", "catalog");
    const listPanel = node("section", "panel");
    listPanel.id = "record-list-panel";
    listPanel.tabIndex = -1;
    listPanel.setAttribute("aria-label", practice ? "Practice register" : "Review register");
    append(listPanel, append(node("header", "panel-heading"), node("h2", "", practice ? "Practice register" : "Review register"), node("span", "", collection.page.total + " recorded")));
    if (!collection.items.length) listPanel.append(stateCard(practice ? "No practices yet" : "No reviews yet", collection.page.total === 0 ? (practice ? "No canonical practice proposals are recorded in this snapshot." : "No Reviews are recorded in this snapshot.") : "There are no records on this page. Return to the first page to continue.", "◇", collection.page.total ? [button("First page", refresh)] : [], "compact"));
    else {
      const list = node("ul", "record-list");
      for (const item of collection.items) {
        const href = routeHash({ ...state.route, id: item.id, snapshot: collection.page.snapshot, offset: collection.page.offset });
        const a = navigationLink("", href, "detail", "record-link");
        a.setAttribute("aria-label", practice ? display(item.name, item.id) : item.id);
        if (state.route.id === item.id) a.setAttribute("aria-current", "true");
        append(a, append(node("div", "record-line"), node("span", "record-name", practice ? display(item.name, item.id) : item.id), practice ? practiceBadge(item) : reviewBadge(item)));
        const meta = node("div", "record-meta");
        if (practice) {
          meta.append(node("div", "", "Proposed by " + display(item.requester, "unrecorded requester")));
          meta.append(node("code", "", short(item.digest, 27)));
        } else {
          meta.append(node("div", "", "Authority · " + display(item.required_authority)));
          meta.append(node("div", "", item.outcome ? "Decision · " + (OUTCOMES[item.outcome] || "Unknown · " + item.outcome) : "No decision recorded"));
        }
        a.append(meta);
        list.append(append(node("li"), a));
      }
      listPanel.append(list);
    }
    const page = collection.page;
    const pagination = node("div", "pagination");
    const previous = button("Previous page", () => navigate({ ...state.route, id: "", offset: Math.max(0, page.offset - page.limit), snapshot: page.snapshot }));
    previous.disabled = page.offset === 0;
    const next = button("Next page", () => navigate({ ...state.route, id: "", offset: page.next_offset, snapshot: page.snapshot }));
    next.disabled = page.next_offset < 0;
    append(pagination, previous, node("span", "pagination-label", collection.items.length ? (page.offset + 1) + "–" + (page.offset + collection.items.length) + " of " + page.total : "0 shown"), next);
    listPanel.append(pagination);
    const detailPanel = node("section", "panel");
    detailPanel.id = "record-detail-panel";
    detailPanel.tabIndex = -1;
    detailPanel.setAttribute("aria-label", practice ? "Practice detail" : "Review detail");
    if (state.detailError) detailPanel.append(errorCard(state.detailError, true));
    else if (state.detail) detailPanel.append(practice ? practiceDetail(state.detail) : reviewDetail(state.detail));
    else {
      detailPanel.classList.add("selection-prompt");
      detailPanel.append(stateCard(practice ? "A practice, in context" : "A decision, in context", practice ? "Select a practice to read its document, provenance, and governing Review." : "Select a Review to inspect its exact subject, authority, and recorded outcome.", practice ? "▤" : "◇", [], "compact"));
    }
    append(catalog, listPanel, detailPanel);
    return catalog;
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
      description = "Your session is missing or has expired. Record data has been cleared. Runtime observation remains available independently.";
      actions.push(link("Sign in", "/auth/login", "button"));
    } else if (error.status === 404) {
      title = error.code === "application_not_found" ? "Application not found" : detail ? (state.route.view === "practices" ? "Practice not found" : "Review not found") : "Read endpoint not found";
      if (detail) description = "The exact identifier in this link is absent from the inspected Record snapshot. It has not been replaced with another object.";
      if (error.code === "application_not_found") actions.push(button("Discover application", () => navigate({ ...state.route, app: "", id: "", offset: 0, snapshot: "" })));
    } else if (error.status === 503) title = "Record unavailable";
    else if (error.status === 409) {
      title = "Record is changing";
      description = "The snapshot changed again after one automatic restart. Retry when the Record settles; no mixed snapshot is displayed.";
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
