// =====================================================================
// pond.js — L2 hand-transpiled pond ecosystem.
//
// This is what `aperio-codegen-js` will eventually emit. Written by
// hand to validate the L1 runtime surface and give L3 a live program
// to visualize.
//
// Source it transpiles (notional Aperio):
//
//   @form(vec) locus Pond : projection rich, schedule pinned(core=0) { … }
//   locus Pad : projection rich, schedule cooperative { … }
//   locus Frog : schedule cooperative { … }
//   locus Blossom { … }
//   locus Cloud : schedule pinned(core=1) { … }
//
// Loads after aperio-runtime.js. Exports a single `boot(opts)` that
// constructs the world and returns refs to the top-level loci.
// =====================================================================

(function (root) {
'use strict';

const A = (typeof require !== 'undefined') ? require('./aperio-runtime.js')
                                            : root.Aperio;

// ---------------------------------------------------------------------
// Blossom — a pool-allocated, short-lived bloom event.
//   locus Blossom { params { ... } birth { ... } fn fade() }
// ---------------------------------------------------------------------

class Blossom extends A.Locus {
  // params:  pad_id, intensity, emitted_at
  // (defaults flattened by Locus ctor via Object.assign)

  birth() {
    // log(f"blossom from pad {self.pad_id}");
    // no-op in JS unless a sink is wired; observers see locus-birth.
  }

  fade() {
    this.intensity = this.intensity * 0.5;
  }
}


// ---------------------------------------------------------------------
// Frog — has its own state, publishes on the bus, has a closure on
// hunger.
//
//   locus Frog : schedule cooperative {
//     params  { id, hunger, pitch }
//     bus     { publish "frog.ribbit" of type RibbitEvent as ribbit; }
//     closure rest_satiation { hunger ~~ 0.5 within 0.4; epoch tick; }
//     birth   { "frog.ribbit" <- RibbitEvent { ... }; }
//     run     { self.hunger = self.hunger + 0.005; }
//     fn eat(bug)   { self.hunger = self.hunger - 0.2; }
//     dissolve { ... }
//   }
// ---------------------------------------------------------------------

class Frog extends A.Locus {
  // params: id, hunger, pitch
  constructor(params, parent, scheduler) {
    super(params, parent, scheduler);

    new A.Closure(this, {
      name: 'rest_satiation',
      epoch: 'tick',
      check: function () { return Math.abs(this.hunger - 0.5) <= 0.4; },
      snapshot: function () { return { hunger: this.hunger }; },
      // Persistent failure → Pad's on_failure routes 'starvation'.
      failureKind: 'starvation',
      routeAfter: 60,   // ~1s of sustained violation
    });
  }

  birth() {
    A.getBus().send('frog.ribbit', {
      frog_id: this.id,
      pitch:   this.pitch,
      when:    Date.now(),
    }, this);
  }

  run() {
    this.hunger = Math.min(this.hunger + 0.005, 1.5);
  }

  eat(bug) {
    this.hunger = Math.max(this.hunger - 0.2, 0);
  }

  dissolve() { /* log */ }
}


// ---------------------------------------------------------------------
// Pad — has a heap of Frog visitors, a vitality closure, and routes
// frog failures with restart/dissolve/bubble.
//
//   locus Pad : projection rich, schedule cooperative {
//     params   { id, vitality, last_bloom }
//     contract { expose vitality; consume sunlight; }
//     capacity { heap visitors of Frog as_parent_for Frog; }
//     closure not_drowning { vitality ~~ 1.0 within 0.5; epoch tick; }
//     fn tick()  { self.vitality = self.vitality * 0.999; }
//     fn seal()  { log(...) }
//     on_failure(child, err) { match err.kind { ... } }
//   }
// ---------------------------------------------------------------------

class Pad extends A.Locus {
  // params: id, vitality, last_bloom
  constructor(params, parent, scheduler) {
    super(params, parent, scheduler);

    // capacity { heap visitors of Frog as_parent_for Frog; }
    const self = this;
    this.visitors = new A.Heap(function (p) {
      return new Frog(p, self, self._scheduler);
    });
    this._capacity.visitors = this.visitors;

    // closure not_drowning { vitality ~~ 1.0 within 0.5; epoch tick; }
    new A.Closure(this, {
      name: 'not_drowning',
      epoch: 'tick',
      check: function () { return Math.abs(this.vitality - 1.0) <= 0.5; },
      snapshot: function () { return { vitality: this.vitality }; },
    });
  }

  tick() {
    // Pad vitality slowly tracks parent pond's water_level so
    // weather has a visible downstream effect.
    const target = this.parent ? this.parent.water_level : 1.0;
    this.vitality = this.vitality * 0.997 + Math.min(target, 1.0) * 0.003;
  }

  seal() { /* log */ }

  // on_failure(child, err) { match err.kind { … } }
  on_failure(child, err) {
    switch (err.kind) {
      case 'starvation':
        // restart(child) until self.vitality > 0.5d
        if (this.vitality > 0.5) A.restart(child);
        else                     A.dissolve(child);
        break;
      case 'drowning':
        A.dissolve(child);
        break;
      default:
        A.bubble(err);
    }
  }
}


// ---------------------------------------------------------------------
// Pond — the main coordinator. @form(vec) is illustrative here; we
// implement the slot as a Heap for now (the form lowering would be
// FormVec, but for pads-of-pads we want individually-named cells).
//
//   @form(vec) locus Pond : tier 2, projection rich, schedule pinned(core=0) {
//     params   { name, water_level, max_pads, bloom_threshold }
//     contract { expose water_level, pad_count; consume rainfall; }
//     capacity { heap pads of Pad as_parent_for Pad;
//                pool blossoms of Blossom; }
//     bus      { subscribe "weather.rain"    as on_rain    of type RainEvent;
//                subscribe "weather.drought" as on_drought of type DroughtEvent;
//                publish   "pond.bloom" of type BloomEvent as bloom_out; }
//     closure  water_persistence { water_level ~~ 1.0 within 0.2; epoch tick; … }
//     birth    { … log … initial pad spawn … }
//     run      { for pad in self.pads { pad.tick() } }
//     fn add_pad(seed) -> Pad fallible(PondFull) { … }
//     fn get_pad(i) -> Pad fallible(NoSuchPad) { … }
//     drain / dissolve / on_failure / modes …
//   }
// ---------------------------------------------------------------------

class Pond extends A.Locus {
  // params: name, water_level, max_pads, bloom_threshold
  constructor(params, parent, scheduler) {
    super(params, parent, scheduler);

    const self = this;

    // capacity slots
    this.pads = new A.Heap(function (p) {
      return new Pad(p, self, self._scheduler);
    });
    this.blossoms = new A.Pool(64, function (p) {
      return new Blossom(p, self, self._scheduler);
    });
    this._capacity.pads = this.pads;
    this._capacity.blossoms = this.blossoms;

    // bus subscriptions
    const bus = A.getBus();
    bus.subscribe('weather.rain', function (e) {
      self.water_level = Math.min(self.water_level + e.intensity * 0.08, 2.0);
    });
    bus.subscribe('weather.drought', function (e) {
      self.water_level = Math.max(self.water_level - 0.15, 0);
    });

    // closure water_persistence
    new A.Closure(this, {
      name: 'water_persistence',
      epoch: 'tick',
      check: function () { return Math.abs(this.water_level - 1.0) <= 0.4; },
      snapshot: function () { return { water_level: this.water_level }; },
    });
  }

  birth() {
    // spawn initial pads
    const n = this.max_pads || 6;
    for (let i = 0; i < n; i++) {
      this.pads.alloc({
        id: i,
        vitality: 0.7 + Math.random() * 0.3,
        last_bloom: 0,
      });
    }
  }

  accept(child) {
    // could quarantine if over cap; pads alloc themselves so this is illustrative
  }

  run() {
    const self = this;
    // for pad in self.pads { pad.tick(); … }
    for (const pad of this.pads) {
      pad.tick();

      // chance to spawn a frog when vitality is high (visual interest)
      if (pad.visitors.count < 3 && pad.vitality > 0.75 && Math.random() < 0.012) {
        pad.visitors.alloc({
          id: Math.floor(Math.random() * 10000),
          hunger: 0.1 + Math.random() * 0.3,
          pitch:  0.5 + Math.random() * 1.0,
        });
      }

      // Frogs occasionally catch a bug — calls eat() which drops hunger.
      // Without this, hunger climbs forever and starvation triggers.
      for (const frog of pad.visitors) {
        if (frog.hunger > 0.3 && Math.random() < 0.03) {
          frog.eat({});
        }
      }

      // chance to emit a blossom when vitality high enough
      if (pad.vitality >= (this.bloom_threshold || 0.7) && Math.random() < 0.02) {
        const b = this.blossoms.acquire({
          pad_id: pad.id,
          intensity: 1.0,
          emitted_at: Date.now(),
        });
        if (b) {
          A.getBus().send('pond.bloom', {
            pad_id: pad.id, intensity: b.intensity, when: Date.now()
          }, this);
          // briefly boost the pad's vitality to celebrate
          pad.last_bloom = Date.now();
        }
      }
    }

    // blossoms fade and recycle
    for (const b of this.blossoms) {
      b.fade();
      if (b.intensity < 0.1) this.blossoms.release(b);
    }

    // gradual water depletion (sunlight / evaporation)
    this.water_level = Math.max(this.water_level - 0.0003, 0);
  }

  // on_failure(child, err) { match err.kind { … } }
  on_failure(child, err) {
    switch (err.kind) {
      case 'drought':
        if (this.water_level > 0.5) A.restart(child);
        else                        A.dissolve(child);
        break;
      case 'rot':
        A.dissolve(child);
        break;
      default:
        A.bubble(err);
    }
  }

  drain() {
    for (const pad of this.pads) {
      // pad.seal();
    }
  }

  dissolve() { /* log */ }

  // Aperio modes — exposed as methods (the compiler emits one per mode).
  bulk() { return this.pads.count; }

  harmonic() {
    let total = 0, n = 0;
    for (const pad of this.pads) { total += pad.vitality; n++; }
    return n ? total / n : 0;
  }
}


// ---------------------------------------------------------------------
// Cloud — separate top-level locus that publishes weather. mood is a
// param the simulator UI flips.
//
//   locus Cloud : schedule pinned(core=1) {
//     params { mood: String = "calm"; }
//     bus    { publish "weather.rain" / "weather.drought" ... }
//     run    { match self.mood { … <- … } }
//   }
// ---------------------------------------------------------------------

class Cloud extends A.Locus {
  // params: mood
  constructor(params, parent, scheduler) {
    super(params, parent, scheduler);
    this._tickCount = 0;
  }

  run() {
    this._tickCount++;
    const bus = A.getBus();

    // Throttle: roughly once a second at 60fps
    if (this._tickCount % 60 !== 0) return;

    switch (this.mood) {
      case 'stormy':
        bus.send('weather.rain', {
          intensity: 1.5 + Math.random() * 1.5,
          when: Date.now(),
        }, this);
        break;
      case 'parched':
        bus.send('weather.drought', {
          duration: 1000,
          when: Date.now(),
        }, this);
        break;
      // 'calm' — yield
    }
  }
}


// ---------------------------------------------------------------------
// boot — construct the world.
// ---------------------------------------------------------------------

function boot(opts) {
  opts = opts || {};

  // Single scheduler + bus for the program.
  const scheduler = A.getScheduler();
  A.getBus();   // ensure bus singleton wired

  const pond = new Pond({
    name:            opts.name            || 'demo-pond',
    water_level:     opts.water_level     != null ? opts.water_level     : 1.0,
    max_pads:        opts.max_pads        != null ? opts.max_pads        : 6,
    bloom_threshold: opts.bloom_threshold != null ? opts.bloom_threshold : 0.7,
  }, null, scheduler);

  const cloud = new Cloud({
    mood: opts.mood || 'calm',
  }, null, scheduler);

  return { scheduler, pond, cloud, Aperio: A };
}


// ---------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------

const PondModule = {
  boot, Pond, Pad, Frog, Blossom, Cloud,
};

if (typeof module !== 'undefined' && module.exports) {
  module.exports = PondModule;
} else {
  root.Pond = PondModule;
}

})(typeof window !== 'undefined' ? window : this);
