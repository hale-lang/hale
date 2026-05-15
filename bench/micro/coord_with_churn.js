// JS equivalent of coord_with_churn.ap.
// K=20 to match Aperio's accept() ceiling.

class Worker {
    constructor(n) { this.n = n; }
}

class Coord {
    constructor(batch) { this.batch = batch; }
    onAccept(w) {}
    run() {
        for (let i = 0; i < this.batch; i++) {
            const w = new Worker(i);
            this.onAccept(w);
        }
    }
}

const k = 20;
const t0 = process.hrtime.bigint();
const c = new Coord(k);
c.run();
const elapsed = process.hrtime.bigint() - t0;
console.log(`k=${k}`);
console.log(`elapsed_ns=${elapsed}`);
