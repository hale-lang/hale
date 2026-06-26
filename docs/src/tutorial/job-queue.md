# Build a job queue

In about thirty minutes, you'll build a small **job queue** and watch it
descend the four altitudes — from a throwaway script to a service split
across processes — changing almost nothing but `main` at the very end. The
first three stages run in the browser at the
[playground](https://hale-lang.github.io/hale/play/) (no install); to follow
along locally, drop each program in a `.hl` file and `hale run` it.

We'll keep the "work" trivial — squaring a number stands in for whatever a
real job does — so the shape of the program stays in focus.

## 1. A job, and the work

Start with the data and the work, as a plain script. A `type` is pure data;
a `fn` does something with it.

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

This is Hale as a small, clean scripting language — no ceremony, no runtime
to think about. One job, processed.

## 2. A queue that holds the jobs

A queue needs to *hold* jobs. In Hale a collection is a locus with a `@form`
annotation — no `Vec<T>` to import or parameterize. `@form(vec)` synthesizes
`push`, `get`, `pop`, `len`, and `is_empty` on the locus; `get`/`pop` are
fallible (out of range), so you address them at the call site with `or`.

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

    let mut i = 0;
    let n = q.len();
    while i < n {
        let j: Job = q.get(i) or Job { id: 0, work: 0 };
        println("job ", j.id, " -> ", process(j));
        i = i + 1;
    }
}
```

```text
queued: 3
job 1 -> 49
job 2 -> 9
job 3 -> 81
```

This is the everyday altitude — loci as plain objects that hold state and
expose behavior. Still a single program, run start to finish.

## 3. Make it a service: the typed bus

A real queue doesn't drain itself in a loop — work *arrives*, and workers
react. That's the typed message bus. Declare the channels as `topic`s, and
wire loci to them: a `Worker` subscribes to `Jobs`, does the work, and
publishes a `Result`; a `Reporter` subscribes to `Results`; a `Submitter`
publishes jobs.

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

Notice what you *didn't* write. The `Submitter` never calls the `Worker` —
it publishes to a topic, and whoever subscribes gets the message. There's no
mutex, no channel type to choose, no `async`/`await` colouring a single
function. This is the concurrent-services altitude, and the cardinality is
emergent: add a second `Worker { };` in `main` and both receive jobs — the
topic is many-to-many.

So far the bus has been running in-process (the default transport — an
in-memory queue). The loci don't know or care. That's the seam we pull on
next.

## 4. Deploy it: change only `main`

The loci above never mention threads or transports. You wire *those* in
`main` — `placement { }` says where loci run, and `bindings { }` says how
each topic travels. None of the `Worker` / `Reporter` / `Submitter` code
changes; you give them a new `main` per deployment.

To run the worker as its own process — listening for jobs over a Unix
socket, on its own cooperative pool — that's a `main` locus:

```hale
// worker.hl — the worker as its own binary. Import the shared Job/Result
// types, the Jobs/Results topics, and the Worker/Reporter loci from §3;
// only this `main` is new.
main locus WorkerNode {
    params {
        worker:   Worker   = Worker { };
        reporter: Reporter = Reporter { };
    }
    placement {
        worker: cooperative(pool = jobs);   // its own pool / OS thread
    }
    bindings {
        Jobs: unix("/run/jobs.sock", role: listen);
    }
}
```

The job *source* becomes a second binary whose `main` instantiates the
`Submitter` and binds the same topic with `role: connect`
(`Jobs: unix("/run/jobs.sock", role: connect);`). Same `Jobs` topic, same
typed payload — now crossing a process boundary instead of an in-memory
queue. Swap `unix(...)` for `udp://host:port` or a broker adapter and the
loci still don't change; only `main` does. (Add a `codec(...)` on the
binding to put JSON or protobuf on the wire so a non-Hale peer can read it.)

For the full multi-binary picture — sharing the loci across files, picking
transports, and supervising the workers — see
[Across binaries](../services/multi-binary.md) and
[Concurrency & placement](../services/concurrency.md).

## What you built

The same `Job` / `Worker` / topic definitions carried you from a script to a
distributed service. Each altitude added exactly what it needed and nothing
more:

| Altitude | What appeared |
|---|---|
| **Script** | `type`, `fn` — data and the work |
| **Everyday** | a `@form(vec)` locus that holds the jobs |
| **Concurrent** | `topic`s + the bus; workers react instead of being called |
| **Systems** | `main` chooses placement and transports — the loci untouched |

That last row is the point: a Hale program is a *design* of loci and topics;
where and how it runs is a binding you change in one place. From here, the
[concurrent services](../services/lifecycle.md) chapters go deeper on
lifecycle, failure, and supervision — or open the
[playground](https://hale-lang.github.io/hale/play/) and run the bus version
in your browser.
