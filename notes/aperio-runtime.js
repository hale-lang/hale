// =====================================================================
// aperio-runtime.js — L1 runtime library for the in-browser JS target.
//
// The Aperio→JS codegen (future L4) emits calls into these primitives.
// The browser simulator (L3) subscribes to the observability hooks.
// The hand-transpiled pond (L2) validates this surface.
//
// Loads via <script src="aperio-runtime.js"> (sets window.Aperio) or
// `require()` in Node.
//
// ---------------------------------------------------------------------
// Library-level decisions:
//   • scheduling      single requestAnimationFrame loop, four phases
//                     per frame, in order:
//                       (a) fire pending lifecycle transitions
//                           (births / accepts / drains / dissolves)
//                       (b) deliver queued bus messages
//                       (c) run closure checks
//                       (d) pump run() bodies
//                     Lifecycle first means newly-born loci see their
//                     birth() run before any run() / closure check.
//   • bus delivery    queued at send time, drained at top of next tick;
//                     one program-wide bus singleton via getBus().
//   • closure cadence honors the user's `epoch` annotation per closure.
//   • fallible ABI    tagged union { ok: true, value } / { ok: false, err };
//                     `or raise` throws ClosureViolation; recovery
//                     primitives catch.
//
// All four are library decisions — swappable here without touching the
// Aperio source or the codegen contract.
// =====================================================================

(function (root) {
'use strict';

// ---------------------------------------------------------------------
// FALLIBLE
// ---------------------------------------------------------------------

function ok(value)  { return { ok: true,  value: value }; }
function fail(err)  { return { ok: false, err: err   }; }

class ClosureViolation extends Error {
  constructor(payload) {
    super('closure violation');
    this.payload = payload;
  }
}


// ---------------------------------------------------------------------
// SCHEDULER
// ---------------------------------------------------------------------

class Scheduler {
  constructor() {
    this._tick      = 0;
    this._loci      = new Set();
    this._buses     = new Set();
    this._closures  = [];     // [{ locus, closure }]
    this._lifecyQ   = [];     // [{ kind, locus, ... }]
    this._listeners = {};
    this._raf       = null;
    this._speed     = 1.0;    // sim ticks per render frame (1.0 ≈ 60/s)
    this._accum     = 0;      // fractional-tick accumulator
    this._wallStart = (typeof performance !== 'undefined') ? performance.now() : Date.now();
  }

  /** Set how many sim ticks elapse per render frame.
   *  1.0 = 60 ticks/sec (full speed). 0 = paused but still rendering. */
  setSpeed(s) { this._speed = Math.max(0, Number(s) || 0); }
  get speed() { return this._speed; }

  start() {
    if (this._raf !== null) return;
    // Bootstrap synchronously: run lifecycle queue until quiet so the
    // world is populated before the first painted frame.
    let bootSafety = 0;
    while (this._lifecyQ.length > 0 && bootSafety++ < 100) {
      this._phaseLifecycle();
    }
    this._accum = 0;
    const loop = () => {
      // Advance sim by (speed) ticks-worth this frame.
      this._accum += this._speed;
      let safety = 0;
      while (this._accum >= 1 && safety++ < 50) {
        this._accum -= 1;
        this._runOneTick();
      }
      // 'frame' fires every render frame regardless of sim cadence.
      this.emit('frame', { tick: this._tick, speed: this._speed });
      this._raf = (typeof requestAnimationFrame !== 'undefined')
        ? requestAnimationFrame(loop)
        : setTimeout(loop, 16);
    };
    this._raf = (typeof requestAnimationFrame !== 'undefined')
      ? requestAnimationFrame(loop)
      : setTimeout(loop, 16);
  }

  stop() {
    if (this._raf === null) return;
    if (typeof cancelAnimationFrame !== 'undefined') cancelAnimationFrame(this._raf);
    else clearTimeout(this._raf);
    this._raf = null;
  }

  /** Run one tick synchronously. Used when paused, also emits 'frame'
   *  so observers (the canvas) can re-render. */
  step() {
    this._runOneTick();
    this.emit('frame', { tick: this._tick, speed: this._speed });
  }

  _runOneTick() {
    this._tick++;
    try { this._phaseLifecycle(); } catch (e) { this._uncaught(e); }
    try { this._phaseBus(); }       catch (e) { this._uncaught(e); }
    try { this._phaseClosure(); }   catch (e) { this._uncaught(e); }
    try { this._phaseRun(); }       catch (e) { this._uncaught(e); }
    this.emit('tick', { number: this._tick });
  }

  get tick() { return this._tick; }
  get running() { return this._raf !== null; }

  on(event, handler) {
    if (!this._listeners[event]) this._listeners[event] = new Set();
    this._listeners[event].add(handler);
    return () => this.off(event, handler);
  }

  off(event, handler) {
    if (this._listeners[event]) this._listeners[event].delete(handler);
  }

  emit(event, payload) {
    const set = this._listeners[event];
    if (!set) return;
    for (const h of set) {
      try { h(payload); } catch (e) { /* observer errors don't break scheduler */ }
    }
  }

  // ---- INTERNAL ----

  _registerLocus(l) {
    this._loci.add(l);
    this._queueLifecycle({ kind: 'birth', locus: l });
    if (l.parent) {
      this._queueLifecycle({ kind: 'accept', parent: l.parent, child: l });
    }
  }

  _unregisterLocus(l) { this._loci.delete(l); }

  _registerBus(b) { this._buses.add(b); }

  _registerClosure(locus, closure) {
    this._closures.push({ locus, closure });
  }

  _queueLifecycle(entry) { this._lifecyQ.push(entry); }

  _phaseBus() {
    for (const bus of this._buses) bus._drain();
  }

  _phaseClosure() {
    const now = (typeof performance !== 'undefined') ? performance.now() : Date.now();
    for (const { locus, closure } of this._closures) {
      if (locus._dissolved || locus._quarantined) continue;
      const ep = closure.opts.epoch;
      if (ep === 'tick') {
        closure.check();
      } else if (ep === 'duration') {
        const interval = closure.opts.interval || 1000;
        if (now - closure._lastChecked >= interval) {
          closure.check();
          closure._lastChecked = now;
        }
      }
      // 'explicit' — user-driven only
    }
  }

  _phaseRun() {
    for (const locus of this._loci) {
      if (locus._dissolved || locus._quarantined || locus._draining || !locus._born) continue;
      try {
        locus.run();
      } catch (e) {
        if (e instanceof ClosureViolation) {
          this._handleFailure(locus, locus, e.payload);
        } else {
          this._uncaught(e);
        }
      }
    }
  }

  _phaseLifecycle() {
    const q = this._lifecyQ;
    this._lifecyQ = [];
    for (const entry of q) {
      switch (entry.kind) {
        case 'birth': {
          if (entry.locus._dissolved) break;
          try {
            entry.locus.birth();
            entry.locus._born = true;
            this.emit('locus-birth', { locus: entry.locus });
          } catch (e) {
            if (e instanceof ClosureViolation) {
              this._handleFailure(entry.locus, entry.locus, e.payload);
            } else this._uncaught(e);
          }
          break;
        }
        case 'accept': {
          if (entry.parent._dissolved) break;
          try {
            entry.parent.accept(entry.child);
            this.emit('locus-accept', { parent: entry.parent, child: entry.child });
          } catch (e) { this._uncaught(e); }
          break;
        }
        case 'drain': {
          if (entry.locus._dissolved) break;
          try {
            entry.locus.drain();
            entry.locus._draining = true;
            this.emit('locus-drain', { locus: entry.locus });
          } catch (e) { this._uncaught(e); }
          break;
        }
        case 'dissolve': {
          this._dissolveCascade(entry.locus);
          break;
        }
        case 'failure': {
          this._handleFailure(entry.locus, entry.child, entry.err);
          break;
        }
      }
    }
  }

  _dissolveCascade(locus) {
    if (locus._dissolved) return;
    // Dissolve children depth-first.
    for (const slotName in locus._capacity) {
      const slot = locus._capacity[slotName];
      if (slot && typeof slot[Symbol.iterator] === 'function') {
        const children = Array.from(slot);
        for (const c of children) {
          if (c instanceof Locus) this._dissolveCascade(c);
        }
      }
    }
    try { locus.dissolve(); } catch (e) { /* ignore during cascade */ }
    locus._dissolved = true;
    this._loci.delete(locus);
    this.emit('locus-dissolve', { locus });
  }

  _handleFailure(locus, child, err) {
    const errObj = (err && typeof err === 'object' && 'kind' in err)
      ? err
      : { kind: 'unknown', payload: err };
    this.emit('recovery', { kind: 'failure', child, locus, err: errObj });
    try {
      locus.on_failure(child, errObj);
    } catch (e) {
      if (e instanceof ClosureViolation) {
        if (locus.parent) {
          this._queueLifecycle({
            kind: 'failure', locus: locus.parent, child: locus, err: e.payload
          });
        } else {
          this.emit('recovery', { kind: 'uncaught', err: e.payload });
        }
      } else this._uncaught(e);
    }
  }

  _uncaught(e) {
    this.emit('recovery', { kind: 'uncaught', err: e });
    if (typeof console !== 'undefined') console.error('aperio scheduler uncaught:', e);
  }
}


// ---------------------------------------------------------------------
// LOCUS
// ---------------------------------------------------------------------

class Locus {
  constructor(params, parent, scheduler) {
    this.params      = params || {};
    Object.assign(this, this.params);
    this.parent      = parent || null;
    this._scheduler  = scheduler || (parent && parent._scheduler) || null;
    this._closures   = [];
    this._capacity   = {};
    this._dissolved  = false;
    this._draining   = false;
    this._quarantined = false;
    this._born       = false;
    if (this._scheduler) this._scheduler._registerLocus(this);
  }

  // Lifecycle hooks (override).
  birth()                       { /* override */ }
  accept(child)                 { /* override */ }
  run()                         { /* override */ }
  drain()                       { /* override */ }
  dissolve()                    { /* override */ }
  on_failure(child, err)        { Aperio.bubble(err); }
}


// ---------------------------------------------------------------------
// RECOVERY PRIMITIVES — free functions on Aperio.*
// ---------------------------------------------------------------------
// Recovery primitives in Aperio source (e.g. `restart(child)`,
// `dissolve(child)`, `bubble(err)`) compile to calls on these free
// functions, not to methods on Locus. Keeps them disambiguated from
// the lifecycle hooks with the same names (`dissolve { ... }` etc.).

function _doRestart(child) {
  if (!child) return;
  try { child.dissolve(); } catch (e) {}
  child._draining = false;
  child._quarantined = false;
  child._dissolved = false;
  child._born = false;
  if (child._scheduler) {
    child._scheduler._loci.add(child);
    child._scheduler._queueLifecycle({ kind: 'birth', locus: child });
  }
}

function _doDissolve(child) {
  if (!child || child._dissolved) return;
  if (child._scheduler) {
    child._scheduler._queueLifecycle({ kind: 'dissolve', locus: child });
  }
}

function _doQuarantine(child) { if (child) child._quarantined = true; }

function _doDrain(child) {
  if (!child) return;
  if (child._scheduler) {
    child._scheduler._queueLifecycle({ kind: 'drain', locus: child });
  }
}

function _doBubble(err) { throw new ClosureViolation(err); }

function _doReorganize(child) { /* deferred — design pending */ }


// ---------------------------------------------------------------------
// BUS — program-wide singleton; loci all use the same bus instance.
// ---------------------------------------------------------------------

class Bus {
  constructor(scheduler) {
    this._scheduler = scheduler;
    this._subs      = {};
    this._queue     = [];
    if (scheduler) scheduler._registerBus(this);
  }

  subscribe(subject, handler) {
    if (!this._subs[subject]) this._subs[subject] = new Set();
    this._subs[subject].add(handler);
    return () => this._subs[subject] && this._subs[subject].delete(handler);
  }

  publish(subject) { /* declarative — runtime no-op */ }

  send(subject, payload, from) {
    this._queue.push({ subject, payload, from: from || null });
    if (this._scheduler) {
      this._scheduler.emit('bus-send', { subject, payload, from: from || null });
    }
  }

  _drain() {
    if (this._queue.length === 0) return;
    const q = this._queue;
    this._queue = [];
    for (const msg of q) {
      const handlers = this._subs[msg.subject];
      if (!handlers) continue;
      for (const h of handlers) {
        try { h(msg.payload, msg); } catch (e) {
          if (this._scheduler) this._scheduler._uncaught(e);
        }
        if (this._scheduler) {
          this._scheduler.emit('bus-deliver', {
            subject: msg.subject, payload: msg.payload, subscriber: h
          });
        }
      }
    }
  }
}


// ---------------------------------------------------------------------
// CAPACITY SLOTS — POOL & HEAP
// ---------------------------------------------------------------------

class Pool {
  constructor(cap, factory) {
    this._cap = cap;
    this._factory = factory;
    this._cells = [];
    this._freeIdx = [];
    this._liveMask = [];   // parallel to _cells; true = live
  }

  acquire(...args) {
    if (this._freeIdx.length > 0) {
      const idx = this._freeIdx.pop();
      this._liveMask[idx] = true;
      const cell = this._cells[idx];
      // F.22 contract: cell is NOT cleared. For locus cells we
      // re-apply params from args[0] so birth sees fresh state.
      if (cell instanceof Locus && args[0]) {
        Object.assign(cell, args[0]);
        cell._dissolved = false;
        cell._draining = false;
        cell._quarantined = false;
        cell._born = false;
        if (cell._scheduler) {
          cell._scheduler._loci.add(cell);
          cell._scheduler._queueLifecycle({ kind: 'birth', locus: cell });
        }
      }
      return cell;
    }
    if (this._cells.length >= this._cap) return null;
    const cell = this._factory(...args);
    this._cells.push(cell);
    this._liveMask.push(true);
    return cell;
  }

  release(cell) {
    const idx = this._cells.indexOf(cell);
    if (idx < 0 || !this._liveMask[idx]) return;
    this._liveMask[idx] = false;
    this._freeIdx.push(idx);
    if (cell instanceof Locus) {
      try { cell.dissolve(); } catch (e) {}
      cell._dissolved = true;
      if (cell._scheduler) cell._scheduler._loci.delete(cell);
    }
  }

  get count() { return this._cells.length - this._freeIdx.length; }
  get cap()   { return this._cap; }

  *[Symbol.iterator]() {
    for (let i = 0; i < this._cells.length; i++) {
      if (this._liveMask[i]) yield this._cells[i];
    }
  }
}


class Heap {
  constructor(factory) {
    this._factory = factory;
    this._cells = new Set();
  }

  alloc(...args) {
    const cell = this._factory(...args);
    this._cells.add(cell);
    return cell;
  }

  free(cell) {
    if (!this._cells.has(cell)) return;
    this._cells.delete(cell);
    if (cell instanceof Locus) {
      if (cell._scheduler) {
        cell._scheduler._queueLifecycle({ kind: 'dissolve', locus: cell });
      }
    }
  }

  get count()         { return this._cells.size; }
  [Symbol.iterator]() { return this._cells.values(); }
}


// ---------------------------------------------------------------------
// FORMS — @form(vec) and (future) others.
// ---------------------------------------------------------------------

class FormVec {
  constructor() { this._items = []; }

  push(x) { this._items.push(x); }

  get(i) {
    if (i < 0 || i >= this._items.length) {
      return fail({ kind: 'out-of-bounds', index: i, len: this._items.length });
    }
    return ok(this._items[i]);
  }

  pop() {
    if (this._items.length === 0) {
      return fail({ kind: 'empty', index: -1, len: 0 });
    }
    return ok(this._items.pop());
  }

  len()      { return this._items.length; }
  is_empty() { return this._items.length === 0; }

  [Symbol.iterator]() { return this._items.values(); }
}

const Form = {
  vec()         { return new FormVec(); },
};


// ---------------------------------------------------------------------
// CLOSURE
// ---------------------------------------------------------------------

class Closure {
  constructor(locus, opts) {
    // opts (new fields):
    //   failureKind:  string  — what err.kind to route as (default: opts.name)
    //   routeAfter:   number  — consecutive failure ticks before routing
    //                            to parent.on_failure (default 60 ≈ 1s)
    //                            set 0 to disable routing entirely
    this.locus = locus;
    this.opts = opts;
    this._lastChecked = 0;
    this._lastResult = null;
    this._consecFails = 0;
    this._routedThisStreak = false;
    if (locus && locus._scheduler) {
      locus._scheduler._registerClosure(locus, this);
    }
  }

  check() {
    let passed;
    try {
      passed = !!this.opts.check.call(this.locus);
    } catch (e) {
      passed = false;
    }
    this._lastResult = passed;
    const s = this.locus._scheduler;
    if (passed) {
      this._consecFails = 0;
      this._routedThisStreak = false;
      if (s) s.emit('closure-pass', { locus: this.locus, closure: this });
    } else {
      this._consecFails++;
      const snap = this.opts.snapshot
        ? (function () { try { return this.opts.snapshot.call(this.locus); } catch (_) { return null; } }).call(this)
        : null;
      if (s) s.emit('closure-fail', { locus: this.locus, closure: this, snapshot: snap });

      // Persistent failure → bubble as a failure event into the parent's
      // on_failure routing. Fires once per streak.
      const threshold = this.opts.routeAfter != null ? this.opts.routeAfter : 60;
      if (threshold > 0 && !this._routedThisStreak && this._consecFails >= threshold) {
        this._routedThisStreak = true;
        const parent = this.locus.parent;
        if (parent && s) {
          s._queueLifecycle({
            kind: 'failure',
            locus: parent,
            child: this.locus,
            err: {
              kind: this.opts.failureKind || this.opts.name,
              payload: snap,
            },
          });
        }
      }
    }
    return passed;
  }
}


// ---------------------------------------------------------------------
// PERSPECTIVE
// ---------------------------------------------------------------------

class Perspective {
  constructor(opts) { this.opts = opts; }

  is_stable() {
    try { return !!this.opts.stableWhen.call(this.opts.locus, this.opts.locus); }
    catch (e) { return false; }
  }

  serialize() {
    if (!this.is_stable()) return null;
    try { return this.opts.capture.call(this.opts.locus, this.opts.locus); }
    catch (e) { return null; }
  }
}


// ---------------------------------------------------------------------
// EXPORT
// ---------------------------------------------------------------------

const Aperio = {
  Scheduler, Locus, Bus, Pool, Heap,
  Form, FormVec, Closure, Perspective,
  ok, fail, ClosureViolation,
  restart:          _doRestart,
  restart_in_place: _doRestart,
  dissolve:         _doDissolve,
  quarantine:       _doQuarantine,
  drain:            _doDrain,
  bubble:           _doBubble,
  reorganize:       _doReorganize,
};

let _defaultScheduler = null;
let _defaultBus = null;

Aperio.getScheduler = function () {
  if (!_defaultScheduler) _defaultScheduler = new Scheduler();
  return _defaultScheduler;
};

Aperio.getBus = function () {
  if (!_defaultBus) _defaultBus = new Bus(Aperio.getScheduler());
  return _defaultBus;
};

Aperio._reset = function () {
  // For tests: reset singletons.
  if (_defaultScheduler) _defaultScheduler.stop();
  _defaultScheduler = null;
  _defaultBus = null;
};

if (typeof module !== 'undefined' && module.exports) {
  module.exports = Aperio;
} else {
  root.Aperio = Aperio;
}

})(typeof window !== 'undefined' ? window : this);
