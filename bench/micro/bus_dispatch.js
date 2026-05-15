// JS equivalent of bus_dispatch.ap.
// Subject-keyed handler dispatch via Map<string, fn>. Mirrors
// Aperio's bus router lookup; NOT EventEmitter (which adds
// listener-array iteration overhead) because Aperio's bus is
// single-subscriber here.

class Aggregator {
    constructor() { this.count = 0; }
    onTick(t) { this.count = this.count + 1; }
}

const agg = new Aggregator();
const router = new Map();
router.set("bench.tick", agg.onTick.bind(agg));

const iters = 10_000;
const t0 = process.hrtime.bigint();
for (let i = 0; i < iters; i++) {
    const handler = router.get("bench.tick");
    handler({ n: i });
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`count=${agg.count}`);
console.log(`elapsed_ns=${elapsed}`);
