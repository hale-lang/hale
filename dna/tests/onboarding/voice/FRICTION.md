# Friction

What building voice in Hale ran into: the cause and the shape that works.
Read it before the next task; append when something fights back. Versions
are the `hale` the entry was found with.

## A stateful handler literal given to `Router.add` in a builder fn dangles

**hale 0.21.0**, found at hale `3d9ea6ef`, still there at `bb0a2c55`.
Filed as [hale-lang/hale#1048](https://github.com/hale-lang/hale/issues/1048).

A route handler locus with a `String` param, constructed inline as the
argument to `Router.add` inside a function that builds and returns the
router, loses that string once the function returns: the handler later reads
garbage, and a server built this way crashed under load. `hale check` is
silent. The docs' `build_router()` example only uses a param-less handler, so
it does not show it.

```hale
locus Echo {
    params { s: String = ""; }
    fn handle(ctx: std::http::Context) -> std::http::Response {
        return std::http::Response { status: 200, body: "[" + self.s + "]" };
    }
}

fn build(dir: String) -> std::http::Router {
    let r = std::http::Router { };
    r.add("GET", "/x", Echo { s: dir + "/canned" });
    return r;
}
// build("some/dir").dispatch(...) answers "[some/dir<garbage>]"
```

It is not only builder functions: a `register(r, dir)` helper that fills a
caller's router dangles the same way, and so does a locus that owns a
`Router` param and fills it in `birth()`. Only a router filled and used in
one function is safe, which leaves no place to put a route table that a
server and its tests share.

**Working shape:** one handler locus for every endpoint, the styleguide's
one-locus-many-endpoints shape: `std::http::build_context(req)` and an
`if std::http::is_route(...)` ladder in its own `handle`, let-bound where the
`Server` is created (`api/endpoints.hl`). Shared state lives in its `params`.

## Triple-quoted and raw strings are in the grammar's comment, not the lexer

**hale 0.21.0.** `spec/grammar.ebnf` notes `STRING_LIT` as
`"..." with escapes; r"..." raw; """..."""`, but `"""{"a": 1}"""` lexes as
an empty string followed by another literal. Only `"..."` with `\"` escapes
exists.

**Working shape:** JSON bodies live in files and are read at request time
(`api/canned/`), not embedded in source.

## `std::http` has no reason phrase for most statuses

**hale 0.21.0.** `__status_phrase` covers 101, 200, 301, 400, 404, 413 and
500. Everything else goes out as `Status`, e.g. `HTTP/1.1 201 Status`.
Clients ignore reason phrases, so nothing breaks, but it reads wrong in a
trace.

**Working shape:** none needed. Worth adding 201, 202, 204, 401, 403, 409,
429, 502, 503 and 504 upstream.
