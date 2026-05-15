// JS equivalent of pipeline_3stage.ap.
// Subject-keyed Map router, two hops.

class Sink {
    constructor() { this.count = 0; this.sum = 0; }
    onFiltered(f) {
        this.count = this.count + 1;
        this.sum = this.sum + f.value;
    }
}

class Filter {
    constructor(router) {
        this.passed = 0;
        this.router = router;
    }
    onEvent(e) {
        if (e.value % 2 === 0) {
            this.passed = this.passed + 1;
            this.router.get("filtered")({ value: e.value });
        }
    }
}

class Source {
    constructor(count, router) {
        this.count = count;
        this.router = router;
    }
    run() {
        const emit = this.router.get("event");
        for (let i = 0; i < this.count; i++) {
            emit({ value: i });
        }
    }
}

const n = 50000;
const sink = new Sink();
const filterRouter = new Map();
filterRouter.set("filtered", sink.onFiltered.bind(sink));
const filter = new Filter(filterRouter);
const sourceRouter = new Map();
sourceRouter.set("event", filter.onEvent.bind(filter));
const source = new Source(n, sourceRouter);

const t0 = process.hrtime.bigint();
source.run();
const elapsed = process.hrtime.bigint() - t0;
console.log(`n=${n}`);
console.log(`filter_passed=${filter.passed}`);
console.log(`sink_count=${sink.count}`);
console.log(`sink_sum=${sink.sum}`);
console.log(`elapsed_ns=${elapsed}`);
