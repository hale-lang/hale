use crate::dna;
use super::fleet::fleet_usage;
use super::model::model_usage;
use crate::topology_graph;
pub(crate) fn usage() {
    eprintln!("hale — Hale language CLI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("    hale lex   <file.hl>          tokenize and print tokens");
    eprintln!("    hale parse <file.hl>          parse and print the AST");
    eprintln!("    hale init  [dir]              bootstrap a project (hale.toml, main.hl, first test)");
    eprintln!("    hale inputs <seed>            every file a build of the seed reads (imports followed)");
    eprintln!("    hale check <file.hl | dir>    parse + typecheck");
    eprintln!("        [--dump-topology[=<path>]] [--check-topology[-shape] <path>]");
    eprintln!("        [--dump-effects-manifest] [--json] [--workspace]");
    eprintln!("        (`hale check --help` for all)");
    eprintln!("    hale verify <file.hl | dir>   check + FAIL on any advisory (discipline gate)");
    eprintln!("    hale topology graph <artifact> render a --dump-topology artifact (svg|mermaid|dot; experimental)");
    eprintln!("    hale model dump <file.hl | dir> derive + print the canonical ApplicationModel (internal; experimental)");
    eprintln!("    hale model diff <a> <b>       semantic diff of two --dump-topology artifacts (--json|--text)");
    eprintln!("    hale fleet check|dump|sign    compose topology artifacts across binaries (also attest|keygen)");
    eprintln!("    hale run   <file.hl | dir>    compile + run as a native binary");
    eprintln!("        [--observe] [--dev] [--target-cpu <v>] [--link <lib>] [--csrc <file.c>]");
    eprintln!("        (flags before the target; after it is the program's own argv)");
    eprintln!("    hale build <file.hl | dir>    parse + typecheck + emit native binary");
    eprintln!("    hale replay <rec> <file.hl>   re-run a LOTUS_OBS_RECORD recording");
    eprintln!("        [--diff: report first divergence, fail on any; per-category coverage on a match]");
    eprintln!("        [--json (with --diff): that verdict + coverage, machine-readable]");
    eprintln!("        [--at <n> | --at <consumer-id>:<ordinal>: SIGSTOP at that consume]");
    eprintln!("        [--feed: inject the recorded ingress tape into (possibly changed) code]");
    eprintln!("        [--allow-unmatched-feed: accept a partially-fed tape]");
    eprintln!("        [--allow-live-effects] [--allow-unverified-model] [--allow-truncated]");
    eprintln!("    hale iris  [port] [artifact]  the embedded observer: attach to LOTUS_OBS=1 processes, serve :8787");
    eprintln!("    hale iris inspect <artifact>  artifact-side inspector (drift / declared-but-silent / law)");
    eprintln!("    hale run --observe <target>   run with LOTUS_OBS=1 and an iris session beside it");
    eprintln!("    hale dna init|new|upgrade     attach the DNA to an application (vendor/dna, dna/, seeded Journal)");
    eprintln!("    hale node  <name>             express a fleet plan's instances on this machine, from the record");
    eprintln!("    hale targets                  the targets this compiler can name, and which it can build");
    eprintln!("    hale test  [file | dir]       compile + run *_test.hl (default: cwd)");
    eprintln!("        [-run <substr>] [--json]");
    eprintln!("    hale bench [file | dir]       run *_bench.hl bench_* fns (default: cwd)");
    eprintln!("        [-run <substr>] [--json]");
    eprintln!("    hale fmt   [file | dir] ...   canonical formatter (default: cwd)");
    eprintln!("        [--check] [--diff] [--stdin]");
    eprintln!("    hale doc   [file | dir]       render the seed's API reference (/// doc comments)");
    eprintln!("        [--json] [-o <path>] [--stdlib: the std:: surface]");
    eprintln!("    hale fetch [repo-root]        fetch git deps from hale.toml into vendor/");
    eprintln!("    hale lsp                      stdio Language Server (diagnostics)");
    eprintln!("    hale mcp                      stdio Model Context Protocol server (agent tools)");
    eprintln!("        [--app <endpoint>: a served exposure's members as tools, read from its description]");
    eprintln!();
    eprintln!("    hale describe <endpoint|file> a served exposure's description: members, streams, schemas");
    eprintln!("        [--token <t>] [-o <path>]");
    eprintln!("    hale call  <endpoint> <member> send a member (with a JSON payload), print the answer");
    eprintln!("        [<json>] [--token <t>]");
    eprintln!("    hale watch <ws://hub> <topic>  subscribe to a stream, print frames as they arrive");
    eprintln!("    hale admin <endpoint>         a local page over the description, calling through the endpoint");
    eprintln!("        [--port <n>] [--token <t>]");
    eprintln!("    hale api export --surface <S> a surface's bundle: description, OpenAPI, JSON Schema, MCP, digest");
    eprintln!("        [--out <dir> | --check <dir>] [file | dir]");
    eprintln!("    hale api describe <endpoint>  a running program's description for the caller: unix:<path> or http://host:port");
    eprintln!("        [--json] [--bearer <t>]");
    eprintln!("    hale api call <endpoint> <member> drive one member of a running program: --json <payload> or --<field> <value>");
    eprintln!("        [--bearer <t>] [--id <id>] [--digest <d>] [--raw]");
    eprintln!();
    eprintln!("    hale --version               print the version, and the embedded DNA source's digest");
    eprintln!("    hale --help                  print this help");
    eprintln!("    hale <command> --help        that command's flags, input shape and output");
}

/// GH #817: one subcommand's usage — what `--help` / `-h` as its
/// first argument prints, before the command parses anything.
///
/// Every command used to answer that question differently, because
/// none of them was answering it: `hale build --help` read the flag
/// as its target (`not a file or directory: --help`), `hale init
/// --help` would have scaffolded a project into a directory of that
/// name, `hale lsp` / `hale mcp` started a server on stdio and
/// waited, and the flag-parsing commands called it an unknown flag
/// (exit 2). Help is the one part of the surface every command has,
/// so it is written in one place.
///
/// Commands whose text already exists somewhere else call it rather
/// than repeat it: a second copy of a flag list is a copy that goes
/// stale.
///
/// Returns `false` for a command this does not answer — `iris` and
/// `dna` parse `--help` themselves further in, and the parser that
/// owns the flags owns their description. A `false` falls through
/// to ordinary dispatch.
pub(crate) fn subcommand_help(cmd: &str) -> bool {
    let text: &str = match cmd {
        // `check` and `verify` have answered `--help` since the
        // v0.15.0 devex review; the pre-pass routes to the same text
        // so there is one description of the flag set.
        "check" | "verify" => {
            check_usage(cmd == "verify");
            return true;
        }
        "fleet" => {
            print!("{}", fleet_usage());
            return true;
        }
        "model" => {
            print!("{}", model_usage());
            return true;
        }
        "topology" => {
            print!("{}", topology_graph::usage_text());
            return true;
        }
        "node" => {
            print!("{}", dna::node_usage());
            return true;
        }
        "lex" => "\
hale lex <file.hl>            tokenize and print the token stream

One file: every token on its own line, with its line:col. Nothing is
parsed, `import` is not followed, and there are no flags.
",
        "parse" => "\
hale parse <file.hl>          parse and print the AST

One file: the parsed `Program`, pretty-printed. Nothing is
typechecked and `import` is not followed — `hale check` is the
command that reads the whole import graph. Takes no flags.
",
        "init" => "\
hale init [dir]               bootstrap a project (default: the current directory)

Writes a `hale.toml` skeleton, a hello-world `main.hl`, a first
`tests/*_test.hl` so `hale test` works from minute one, and a
`.gitignore` covering the build artifact and `vendor/`.

Strictly non-destructive: a file that already exists is reported and
left exactly as it was, so `init` is safe to re-run in a
half-scaffolded directory. Takes no flags.
",
        "inputs" => "\
hale inputs <seed-dir | file.hl>   every file a build of the seed reads

One canonical path per line: the seed's own `.hl` files and every
`.hl` of every directory they import, transitively, resolved exactly
as the compiler resolves them. For anything that must know what a
build depends on without guessing from git — an untracked, ignored
or oddly named source file beside the reviewed ones is compiled all
the same. Takes no flags.
",
        "targets" => "\
hale targets                  the targets this compiler can name

One paragraph per target: its canonical triple, what it is for, and
its tier — naming a target and being able to BUILD it are different
capabilities, so each says which of the two it is. The host's own
target is marked `(host)`. `--list-targets` is the same command.
Takes no flags; `hale build --target <triple>` is what selects one.
",
        "fetch" => "\
hale fetch [repo-root]        fetch the git deps in hale.toml into vendor/

Each `[deps]` entry is cloned into `vendor/<name>/` and its resolved
SHA pinned in `hale.lock`. Default root: the current directory.
Takes no flags.
",
        "run" => "\
hale run <file.hl | dir> [program args...]   compile + run as a native binary

Compiles the target exactly as `hale build` does and execs the
result from a temporary path — there is no interpreter. The target
is one `.hl` file, whose `import` directives are followed, or one
directory, whose `.hl` files are one seed.

The first argument that is not a flag IS the target, so `hale run`'s
own flags go BEFORE it. Everything after the target is the PROGRAM's
argv: `std::env::arg` sees what a built binary run directly would
see.

  --observe                        run the program with LOTUS_OBS=1
                                   and an iris session beside it,
                                   for its lifetime

The build options are `hale build`'s, with the same meanings, and
they are part of the execution identity a recording carries: a run
recorded under `--dev` replays only under `hale replay --dev`.

  --target-cpu <native|baseline>   backend CPU tuning
  --dev                            LLVM O1 instead of the O3 default
  --link <name>                    link a system library (repeatable)
  --csrc <file.c>                  compile and link a C source
                                   (repeatable)
  --env <name>                     the deployment target: adopt the
                                   constitution [environments.<name>]
                                   binds
  --target <native>                `run` execs what it builds, so a
                                   target this host cannot execute
                                   (wasm32) is refused; build it
                                   with `hale build --target`
",
        "build" => "\
hale build <file.hl | dir> [flags]   parse + typecheck + emit a native binary

The target is one `.hl` file, whose `import` directives are
followed, or one directory, whose `.hl` files are one seed and one
binary.

The binary lands beside the target unless `-o` says where:

    hale build app.hl    ->  ./app           the basename, minus .hl
    hale build myapp/    ->  myapp/myapp     the directory's own name,
                                             inside it
    hale build myapp/ -o out/bin/myapp
                         ->  out/bin/myapp   exactly that path, its
                                             directories made; for a
                                             wasm build the .wasm is
                                             that path and its loader
                                             (.mjs) sits beside it

Flags may stand on either side of the target — the first argument
that is not a flag is the target, as in `hale check`:

    hale build --dev app.hl   ==   hale build app.hl --dev

  --target <native|wasm32|triple>  which backend emits the artifact
                                   (`hale targets` lists every target
                                   this compiler can name, and says
                                   which it can build). Without it, a
                                   `target wasm { }` declaration
                                   selects wasm32; with it, a
                                   declaration it contradicts is
                                   refused
  --target-cpu <native|baseline>   `native` tunes to this host, best
                                   speed and not portable; `baseline`
                                   pins a portable x86-64-v3 for an
                                   artifact that travels
  -o, --out <path>                 write the artifact to <path> instead
                                   of beside the target (`build` only:
                                   `run` and `replay` execute theirs)
  --dev                            LLVM O1 instead of the O3 default:
                                   build latency over run speed
  --link <name>                    link a system library (repeatable)
  --csrc <file.c>                  compile and link a C source
                                   (repeatable)
  --env <name>                     the deployment target: adopt the
                                   constitution [environments.<name>]
                                   binds
  --wrap-main                      synthesize the wasm @export entry
                                   from `fn main` (--target wasm32)
  --locality-report                the per-locus working-set table,
                                   on stderr; the build proceeds
  --target-cache <l1|l2|l3>        evaluate each locus against that
                                   cache tier's budget
  --strict                         with --target-cache: an over-budget
                                   locus is a build error, not a
                                   warning
",
        "replay" => "\
hale replay <recording> <program.hl>   re-run a LOTUS_OBS_RECORD recording

The recording is the journal a program wrote under
`LOTUS_OBS_RECORD=<path>`; the program is the code to re-execute it
against. Admission is fail-closed: a recording from different build
inputs, from a different model, or without a clean finalize is
refused unless the matching flag accepts the gap.

  --diff                    report the first divergence from the
                            recording and fail on any; on a match,
                            per-category coverage
  --json                    with --diff: that verdict and coverage,
                            machine-readable
  --at <n>                  SIGSTOP at the n'th consume
  --at <consumer-id>:<n>    ... at that consumer's n'th consume,
                            stable across multi-consumer runs
  --feed                    inject the recorded ingress tape into
                            (possibly changed) code — there is no
                            recorded schedule to compare against, so
                            not with --diff or --at
  --allow-unmatched-feed    with --feed: accept a partially-fed tape
  --allow-live-effects      re-execute although this program can
                            reach the live world (syscall and ffi
                            writes repeat)
  --allow-unverified-model  accept a recording whose execution
                            identity is missing or does not match
                            this compile
  --allow-truncated         replay the recorded prefix of a
                            crash-truncated recording

`replay` recompiles the program, so it takes `hale build`'s options
too — and must be given the ones the recording was made under, since
they are part of the execution identity admission checks:
`--target-cpu`, `--dev`, `--link`, `--csrc`, `--target`.
",
        "test" => "\
hale test [file | dir]        compile + run every `*_test.hl` (default: cwd)

Each `_test.hl` file is compiled and run as its own binary, and its
exit status is the verdict. Finding nothing to run is success, not
an error.

Files compile and run in parallel, one worker per available core by
default. The report is the same whatever the job count: results are
printed in sorted file order once every file is done, and each test's
own output stays with its own file.

  -run <substr>   only files whose path contains <substr>
                  (`--run`, `-run=<substr>` and `--run=<substr>` too)
  -j, --jobs <N>  compile and run at most N files at once (`-jN`,
                  `--jobs=N` too); `-j 1` runs them one after another.
                  HALE_TEST_JOBS=N sets the default; the flag wins
  --json          the results as JSON on stdout
",
        "bench" => "\
hale bench [file | dir]       run every `*_bench.hl`'s bench_* fns (default: cwd)

Self-calibrating: each parameterless `bench_*` fn is run until the
harness has a stable measurement, and reports ns/op and allocs/op. A
`*_bench.hl` file may not declare `fn main`.

  -run <substr>   only benches whose NAME contains <substr>
  --json          the results as JSON on stdout
",
        "fmt" => "\
hale fmt [file | dir ...]     canonical formatter, in place (default: cwd)

Go-style and zero config. Directories are recursed; `vendor/` and
dot-dirs are skipped. A file that does not lex is reported and
skipped — the formatter never rewrites a file it cannot fully
tokenize.

  --check    do not write: list the files that would change and exit
             1 (the CI gate)
  --diff     do not write: print the before/after for those files
  --stdin    format stdin to stdout (editor integration)
",
        "doc" => "\
hale doc [file | dir]         render the seed's API reference (`///` comments)

Markdown on stdout, one entry per documented declaration of the
seed's `.hl` files.

  --json        the same items as JSON instead of Markdown
  -o <path>     write to <path> instead of stdout (`--out` too)
  --stdlib      document the `std::` surface instead of a seed
",
        "lsp" => "\
hale lsp                      stdio Language Server (diagnostics)

Speaks LSP over stdin and stdout. No target and no flags: the seed
to check is derived per document from the client's `textDocument`
URIs. Not meant to be run by hand — point an editor at `hale lsp`.
",
        "mcp" => "\
hale mcp [--app <endpoint>]   stdio Model Context Protocol server (agent tools)

Speaks MCP over stdin and stdout, exposing the toolchain to a host
without a shell. No target. Its tools self-exec this binary, so the
compiler an agent drives is the one it is talking to.

`--app <endpoint>` serves a RUNNING program instead: every member of
the exposure's description (for the caller the endpoint names, `--token`
for a bearer) is a tool, its request schema the tool's input schema,
read from the description the exposure serves; a call names the digest
read. An `mcp://host:port` endpoint is an `mcp::Rpc` listener, whose own
`tools/list` is forwarded. `claude mcp add app -- hale mcp --app
/run/app.sock` is the whole setup.
",
        "api" => crate::verbs::api::api_usage(),
        "describe" => "\
hale describe <endpoint | file.hl | dir> [--token <t>] [-o <path>]

The description of a served exposure (spec/api.md § The description):
its identity and digest, its listener, the caller it established and the
roles that caller holds, the members it may call with their schemas, the
streams it may subscribe to, the outcome encoding and the notes. An
endpoint is a socket path, `http://host:port` (the caller is named by
`--token` or HALE_API_TOKEN, a bearer) or `ws://host:port` (a hub), and
the document is the bytes the exposure serves. From a program it is
`hale check --api`: the inventory of every surface and exposure, or with
`--exposure NAME --caller P [--holds R,...]` one exposure's description,
or with `--surface NAME --openapi | --json-schema | --mcp | --proto` one surface's
projection, all from the rows without running the program.
",
        "call" => "\
hale call <endpoint> <member> [<json payload>] [--token <t>] [--receipt]

Calls one member of a served exposure and prints the response. The
description is read first: a member the caller may not call is not
listed, and the call names the digest read, so a program that changed
refuses it (`digest_mismatch`). A handler error or a refusal is printed
on stderr with its kind and reason (a refusal for a role names what the
row requires) and the exit code is 1. `--receipt` prints the answer as
the exposure wrote it: the reply line over a socket (request_id, the
echoed id, the caller), `{status, body}` over HTTP.
",
        "watch" => "\
hale watch <ws://host:port> <topic> [--token <t>]

Subscribes to a stream of a hub and prints every frame as one JSON line
(`subscribed`, then `event`s with their `seq`) until the hub closes the
connection. A refusal, or a subscription that expires or is revoked, is
printed on stderr and the exit code is 1.
",
        "admin" => "\
hale admin <endpoint> [--port <n>] [--token <t>]

Serves a page on 127.0.0.1 (port 7473 by default) over a served
exposure's description: a form per member, a live tail per stream, every
action one request to the endpoint. Nothing is configured; the
description is the page.
",
        _ => return false,
    };
    print!("{}", text);
    true
}

pub(crate) fn check_usage(verify: bool) {
    let cmd = if verify { "verify" } else { "check" };
    let what = if verify {
        "typecheck + analyze; EVERY finding, advisory included, fails \
         the run"
    } else {
        "typecheck + analyze a seed"
    };
    println!("hale {} [flags] <file | dir>    {}", cmd, what);
    println!();
    println!("A directory is ONE seed: the `.hl` files directly inside");
    println!("it are checked together, without recursing. Flags may");
    println!("appear before or after the target.");
    println!();
    println!("  --workspace                    check EVERY seed under the");
    println!("                                 target, each independently.");
    println!("                                 Skips vendor/, target/ and");
    println!("                                 dot-dirs. Every seed runs even");
    println!("                                 if an earlier one fails. It does");
    println!("                                 NOT connect seeds to each other.");
    println!("  --env <name>                   also adopt the constitution that");
    println!("                                 `[environments.<name>]` in hale.toml");
    println!("                                 requires. One entrypoint deployed to");
    println!("                                 two environments is checked twice.");
    println!("  --matrix                       check every (entrypoint, environment)");
    println!("                                 pair the manifest declares. An");
    println!("                                 entrypoint listed in NO environment");
    println!("                                 is an error, not a skip. Cannot be");
    println!("                                 combined with --env, --workspace, or");
    println!("                                 any per-evaluation artifact flag.");
    println!("  --target <native|wasm32|triple>  check for that target, as");
    println!("                                 `hale build --target` builds for it.");
    println!("                                 Without it, a `target wasm {{ }}`");
    println!("                                 declaration selects wasm32, else");
    println!("                                 the host.");
    println!("  --link <lib>                    a library the build would link, held");
    println!("                                 to the target as `hale build` holds it");
    println!("                                 (wasm32 links none); so is every");
    println!("                                 imported package's `[ffi] link`.");
    println!();
    println!("Claims and topology (spec/verification.md):");
    println!("  --dump-topology[=<path>]        emit the topology artifact");
    println!("                                 (JSON; stdout when bare).");
    println!("                                 Observational: it does not");
    println!("                                 change the exit status.");
    println!("  --check-topology <path>         gate on an EXACT artifact");
    println!("                                 snapshot — law, model and");
    println!("                                 provenance. Source motion");
    println!("                                 trips it.");
    println!("  --check-topology-shape <path>   gate on the model identity");
    println!("                                 (`shape_hash`) alone. Immune");
    println!("                                 to comments moving and to");
    println!("                                 claim renames.");
    println!();
    println!("The API surface (spec/api.md):");
    println!("  --api                           print the inventory: every");
    println!("                                 surface with its digest and rows,");
    println!("                                 every exposure and hub (JSON).");
    println!("  --api --exposure <name> --caller <principal> [--holds <role,...>]");
    println!("                                 print one exposure's description");
    println!("                                 for a caller holding those roles");
    println!("                                 under its role source. <principal>");
    println!("                                 is a name or the JSON principal.");
    println!("  --api --surface <name> --openapi | --json-schema | --mcp | --proto");
    println!("                                 print that form of one surface.");
    println!();
    println!("Effects and budgets:");
    println!("  --dump-effects-manifest         emit the effects manifest");
    println!("  --check-effects-manifest <path> gate on a manifest baseline");
    println!("  --dump-resource-budget          emit the resource budget");
    println!("  --check-resource-budget <path>  gate on a budget baseline");
    println!("  --dump-alloc-summary            emit the allocation summary");
    println!();
    println!("Advisories:");
    println!("  --warn-resource-leak            enable the resource-leak lint");
    println!("  --strict-secret                 fail-closed `@secret` containment check");
    println!("  --sealable                      report which loci could be `@sealed`");
    println!("  --flows                         report which locus types are flows, and the");
    println!("                                 `release(c: T)` clause(s) that make each one");
    println!("  --units                         report every quantity's denomination, each");
    println!("                                 narrowing and the policy that discharges it");
    println!("                                 (stdout; one object with --json)");
    println!("  --no-warn-unbounded-alloc       silence the unbounded-alloc lint");
    println!("  --allow-unowned-subscriber      permit a subscriber with no owner");
    println!("  --json                          machine-readable diagnostics");
}
