# Drive it from outside

A service that is authoritative over something ends up wanting a
surface that tools can plug into: a command line, a dashboard, an
MCP host. You could write an HTTP server, a JSON codec per message
and a routing table for it. You do not have to. Everything the
program already declares on its bus *is* its API, and one line at
the deployment tier hands it out.

## One entry, no other change

Take a billing service as it is: a locus that subscribes a topic,
publishes another, and exposes a field.

```hale
type Verdict { review_id: Int; verdict: String; }
type VerdictResult { ok: Bool; note: String; }
type PriceMoved { sym: String; price: Float; }
type Ledger { balance: Int; entries: Int; }

topic Verdicts { payload: Verdict; subject: "app.verdict"; }
topic Prices { payload: PriceMoved; subject: "app.price"; }

locus Billing {
    contract { expose ledger: Ledger; }
    params { ledger: Ledger = Ledger { balance: 100, entries: 0 }; }
    bus {
        subscribe Verdicts as on_verdict;
        publish Prices;
    }
    fn on_verdict(v: Verdict) -> VerdictResult {
        self.ledger.entries = self.ledger.entries + 1;
        return VerdictResult { ok: true, note: "ratified " + to_string(v.review_id) };
    }
    run() {
        while !self.draining {
            std::time::sleep(1s);
            Prices <- PriceMoved { sym: "ABC", price: 1.5 };
        }
    }
}

main locus App {
    params { billing: Billing = Billing { }; }
    placement { billing: cooperative(pool = work) where async_io; }
    bindings {
        api: unix("/run/app.sock", bound: 64, on_full: refuse);
    }
}

fn main() {
    App { };
}
```

The `api:` entry is the whole change. It binds every topic some
locus subscribes as a **command** (`Verdicts`), every topic some
locus publishes as a **stream** (`Prices`), and every `expose` of
the main locus or of its default children as a **read**
(`billing.ledger`). The handler's return type became the reply:
`on_verdict` returns a `VerdictResult`, so a caller gets one back.
Nothing else in the source knows the socket exists, and a program
without the entry pays nothing for it.

For a program you are only trying out, skip even that line:

```sh
hale run --api /run/app.sock app.hl
```

puts the same entry on the main locus with the dev defaults. It
needs a `main locus` to put it on; a bare `fn main` program is
refused with the rule.

## Talking to it

The socket speaks one JSON object per line. A request is a `call`,
a `read` or a `watch`, with an `id` you choose so you can match the
answer:

```text
{"id": 1, "call": "Verdicts", "payload": {"review_id": 7, "verdict": "ratify"}}
{"id": 2, "read": "billing.ledger"}
{"id": 3, "watch": "Prices"}
```

and each answer carries your `id` plus the binding's own
`request_id`:

```text
{"request_id": 1, "id": 1, "ok": true, "value": {"ok": true, "note": "ratified 7"}}
{"request_id": 2, "id": 2, "ok": true, "value": {"balance": 100, "entries": 1}, "as_of": "sha256:8e5d…"}
{"request_id": 3, "id": 3, "ok": true, "attached": "Prices"}
{"stream": "Prices", "value": {"sym": "ABC", "price": 1.5}}
```

A command whose handler has no return type is answered `{"ok":
true, "accepted": true}` the moment it is dispatched: that is what
"accepted" means for this binding, and it is the same promise
`Verdicts <- v` makes in-process. From a shell, `socat` is enough
to try it:

```sh
printf '%s\n' '{"id":1,"call":"Verdicts","payload":{"review_id":7,"verdict":"ratify"}}' \
    | socat - UNIX-CONNECT:/run/app.sock
```

The generic clients (`hale call`, `hale watch`, `hale admin` and
`hale mcp --app`) read the description the compiler emits and are
the next piece; until then any program that can write a line to a
socket is a client.

## When it says no

A refusal is an answer, never a failure of the program:

```text
{"request_id": 4, "id": 4, "ok": false, "refusal": {"kind": "malformed", "reason": "wrong_type: review_id"}}
```

The kinds are `malformed` (not a JSON object, no verb, or a payload
that does not decode; the reason names the field), `unknown` (no
such topic or read), `not_a_command` (you called a stream),
`not_a_stream` (you watched a command) and `over_bound`. A payload
is decoded before dispatch, strictly: a string where an `Int` is
declared is `wrong_type`, a missing field without a default is
`missing_field`, and the handler only ever sees a value of its
declared type.

## The two knobs

An outside caller is unbounded by nature, so the entry carries a
bound and a policy and the checker insists on both. `bound: 64`
means at most sixty-four commands and reads waiting on a handler
at once; the sixty-fifth caller gets `over_bound` and the program
never sees it. `refuse` is the only policy for requests, because a
caller waiting for an answer cannot be shed silently. Watchers have
their own pair, per connection: `watch_bound: 256, on_watch_full:
drop_old` keeps the newest frames for a slow reader and reports how
many it dropped on the next frame it does get (`{"stream": "Prices",
"dropped": 3}`). Leave both out and a watcher gets `bound` frames
with `drop_old`.

## Reads are snapshots

A read never reaches across threads into a locus's field. It is
answered on the locus's own pool, as a copy taken there, and the
answer says when: `as_of` is a digest of what was answered, so two
reads that agree on it saw the same state and a later command can
say "only if it is still this". A live view is what a stream is
for; the two verbs are different on purpose.

## What is left out, and why

- A topic whose payload has a field with no JSON form yet
  (`Decimal`, `Time`, `Duration`, `Bytes`, an array, an enum, a
  locus) stays off the API, with a warning at the entry naming the
  field. Adding the entry never breaks a build.
- A topic two handlers both answer is an error at the entry: one
  reply per command.
- A `Drain<T>` batch handler is not reached through the binding
  yet; bulk requests wait on batch delivery over the cooperative
  queue.
- Who is calling, and roles, are the next pieces. Today every
  connection is trusted equally, so put the socket where only the
  right processes can open it.
