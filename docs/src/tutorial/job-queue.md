# Tutorial: Build a Job Queue

This tutorial walks through building a job queue in four stages—starting from a 12-line script and evolving it into a multi-process service—while keeping the core business logic unchanged.

| Stage | Constructs Introduced | Execution Model |
|---|---|---|
| **1. Script** | `type`, `fn`, `fn main()` | Single-threaded run-to-exit script |
| **2. In-Memory Queue** | `locus`, `@form(vec)`, `capacity` | Stateful arena-backed collection |
| **3. Reactive Bus** | `topic`, `bus { subscribe; publish; }`, `<-` | Decoupled pub/sub components |
| **4. Multi-Process** | `main locus`, `placement`, `bindings` | Separate OS processes over a Unix socket |

Stages 1–3 run directly in the browser on the [Hale Playground](https://play.hale-lang.org/) or locally via `hale run`.

---

## Stage 1: Pure Data and a Function

Start with a `type` (a pure value record) and a `fn` that processes it:

```hale
type Job { id: Int; work: Int; }

fn process(j: Job) -> Int {
    return j.work * j.work;
}

fn main() {
    let j: Job = Job { id: 1, work: 7 };
    println("job ", j.id, " -> ", process(j));
}
```

```text
job 1 -> 49
```

---

## Stage 2: Storing Jobs in a `@form(vec)` Locus

In Hale, dynamic collections are loci annotated with `@form(...)` rather than generic `Vec<T>` types. `@form(vec)` generates `push`, `get`, `pop`, `len`, `is_empty`, `clear`, and `truncate` over the `heap` slot declared in `capacity { }`:

```hale
type Job { id: Int; work: Int; }

@form(vec)
locus Queue {
    capacity { heap jobs of Job; }
}

fn process(j: Job) -> Int { return j.work * j.work; }

fn main() {
    let q = Queue { };
    q.push(Job { id: 1, work: 7 });
    q.push(Job { id: 2, work: 3 });
    q.push(Job { id: 3, work: 9 });
    println("queued: ", q.len());

    for j in q.items {
        println("job ", j.id, " -> ", process(j));
    }
}
```

```text
queued: 3
job 1 -> 49
job 2 -> 9
job 3 -> 81
```

Because `q` is bound with `let q = Queue { };`, its arena lives until the end of `main()` and frees all queued `Job` elements in a single operation at scope exit.

---

## Stage 3: Decoupling Producers and Workers with the Typed Bus

Instead of polling a list in `main()`, real services react as work arrives. Declare typed channels with `topic` and connect loci with `bus { }` blocks:

- `Submitter` publishes `Job` payloads to `Jobs` using `Jobs <- payload;`.
- `Worker` subscribes to `Jobs`, computes the result, and publishes to `Results`.
- `Reporter` subscribes to `Results` and logs completed work.

```hale
type Job    { id: Int; work: Int; }
type Result { id: Int; out: Int; }

topic Jobs    { payload: Job; }
topic Results { payload: Result; }

locus Worker {
    bus {
        subscribe Jobs as on_job;
        publish   Results;
    }
    fn on_job(j: Job) {
        let out: Int = j.work * j.work;
        Results <- Result { id: j.id, out: out };
    }
}

locus Reporter {
    bus { subscribe Results as on_result; }
    fn on_result(r: Result) { println("job ", r.id, " done -> ", r.out); }
}

locus Submitter {
    bus { publish Jobs; }
    birth() {
        Jobs <- Job { id: 1, work: 7 };
        Jobs <- Job { id: 2, work: 3 };
        Jobs <- Job { id: 3, work: 9 };
    }
}

fn main() {
    Worker { };
    Reporter { };
    Submitter { };
}
```

```text
job 1 done -> 49
job 2 done -> 9
job 3 done -> 81
```

> **Try it live:** Open the [Job Queue example in the playground](https://hale-lang.org/play/?example=jobqueue).

Notice two rules in `fn main()`:
1. **Subscribers are instantiated before publishers:** `Worker` and `Reporter` are born before `Submitter`'s `birth()` hook publishes messages (`hale check` warns if a publisher fires before its subscribers exist).
2. **Zero locks or `async`/`await`:** `Submitter` never holds a reference to `Worker`. Adding a second `Worker { };` in `main` automatically fans messages out to both subscribers.

---

## Stage 4: Splitting Across Processes Without Touching Locus Code

Notice that `Worker`, `Reporter`, and `Submitter` contain no thread or socket code. Threading (`placement`) and network/IPC transports (`bindings`) are configured exclusively in `main locus`.

To run `Worker` and `Reporter` as a standalone daemon listening on a Unix domain socket (`/tmp/jobs.sock`) on a dedicated worker thread pool, write a `main locus`:

```hale,fragment
// worker_node.hl — imports Job, Result, Jobs, Results, Worker, Reporter from stage 3
main locus WorkerNode {
    params {
        worker:   Worker   = Worker { };
        reporter: Reporter = Reporter { };
    }
    placement {
        worker: cooperative(pool = jobs);   // Dedicated OS thread pool
    }
    bindings {
        Jobs: unix("/tmp/jobs.sock", role: listen);
    }
    run() {
        while !self.draining { std::time::sleep(1s); }   // Keep daemon alive until SIGINT/SIGTERM
    }
}

fn main() { WorkerNode { }; }
```

A separate producer binary imports `Jobs` and `Submitter` and binds `Jobs: unix("/tmp/jobs.sock", role: connect);` in its own `main locus`. The `Jobs` topic now crosses an OS process boundary over `/tmp/jobs.sock` with zero edits to `Worker`, `Reporter`, or `Submitter`.

### Where to Go Next

- **[Part I: Values, Types & Variables](../basics/values.md)** — Learn the core language syntax and type system from the ground up.
- **[Part II: Locus Lifecycle](../services/lifecycle.md)** and **[Concurrency & Thread Placement](../services/concurrency.md)** — Deep dive into `birth`/`run`/`drain`/`dissolve` and thread pools.
- **[Part II: Multi-Binary Systems & Fleets](../services/multi-binary.md)** — Compose multiple binaries into a verified `fleet locus`.
