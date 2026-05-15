// JS equivalent of tree_fanout.ap.

class Worker {
    constructor(id, batchSize) {
        this.id = id;
        this.batchSize = batchSize;
    }
    compute() {
        let sum = 0;
        for (let i = 0; i < this.batchSize; i++) {
            sum = sum + i;
        }
        return sum;
    }
}

class Coordinator {
    constructor(numWorkers, itemsEach) {
        this.numWorkers = numWorkers;
        this.itemsEach = itemsEach;
        this.total = 0;
    }
    accept(w) {
        this.total = this.total + w.compute();
    }
    run() {
        for (let i = 0; i < this.numWorkers; i++) {
            const w = new Worker(i, this.itemsEach);
            this.accept(w);
        }
    }
}

const k = 20;
const m = 2000;
const t0 = process.hrtime.bigint();
const c = new Coordinator(k, m);
c.run();
const elapsed = process.hrtime.bigint() - t0;
console.log(`k=${k}`);
console.log(`m=${m}`);
console.log(`total_ops=${k * m}`);
console.log(`total=${c.total}`);
console.log(`elapsed_ns=${elapsed}`);
