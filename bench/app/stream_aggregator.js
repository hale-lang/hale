// JS equivalent of stream_aggregator.ap.
// Pub/sub aggregator over a subject-keyed router.

class Aggregator {
    constructor() {
        this.count = 0;
        this.sum = 0;
        this.minV = 999999999;
        this.maxV = 0;
    }
    onSample(s) {
        this.count = this.count + 1;
        this.sum = this.sum + s.value;
        if (s.value < this.minV) this.minV = s.value;
        if (s.value > this.maxV) this.maxV = s.value;
    }
}

const agg = new Aggregator();
const router = new Map();
router.set("bench.sample", agg.onSample.bind(agg));

const iters = 200_000;
const t0 = process.hrtime.bigint();
const handler = router.get("bench.sample");
for (let i = 0; i < iters; i++) {
    const v = (i * 31 + 7) % 1000;
    handler({ value: v });
}
const elapsed = process.hrtime.bigint() - t0;
console.log(`iters=${iters}`);
console.log(`count=${agg.count}`);
console.log(`sum=${agg.sum}`);
console.log(`elapsed_ns=${elapsed}`);
