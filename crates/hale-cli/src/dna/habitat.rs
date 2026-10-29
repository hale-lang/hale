//! `hale habitat` — the acts of the machine's habitat (GH #1653).
//!
//! The local habitat is long-lived: one compose project per machine,
//! named for the habitat, holding a service per allocation — an
//! organism's memory, nerves and senses' store, named for the organism's
//! record identity and the role. Organisms are its tenants, reused from
//! run to run, and no command that works on an organism (`dev`, `run`,
//! `head`, `show`, `work`, the host's verbs) provisions or migrates
//! anything as a side effect: each reports what is missing and names the
//! act that supplies it. This module holds those acts. `migrate` brings
//! the organism's allocations up and moves memory's schema and the
//! nerves' stream to this toolchain's; `up`, `status` and `down` follow
//! with the habitat's genome.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use super::{host_run, local_habitat_dir, local_habitat_project, manifest_pin, memory_migrate, nerves_migrate, project, MemoryPlan, NervesPlan};

#[path = "sapling.rs"]
mod sapling;

#[path = "forget.rs"]
mod forget;

#[path = "remote.rs"]
mod remote;

#[path = "readings.rs"]
mod readings;

#[path = "reflexes.rs"]
mod reflexes;

#[path = "hosted.rs"]
mod hosted;

#[path = "placements.rs"]
mod placements;

static HEAD_FLAG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static IN_HEAD_ENDPOINT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const HABITAT_DOCKERFILE: &str = include_str!("../../../../Dockerfile");
const HABITAT_PIN: &str = env!("HALE_HABITAT_PIN");
/// The toolchain's repository: a habitat image is built from a commit of it.
pub(super) const HALE_GIT: &str = "https://github.com/hale-lang/hale.git";

pub(super) fn set_head_flag(url: String) {
    let _ = HEAD_FLAG.set(url);
}

pub(super) fn head_flag() -> Option<String> {
    HEAD_FLAG.get().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty())
}

pub(super) fn head_flag_or_local_url() -> Option<String> {
    if let Some(u) = head_flag() {
        return Some(u);
    }
    if let Some(u) = std::env::var("HALE_DNA_HEAD_URL").ok().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()) {
        return Some(u);
    }
    std::fs::read_to_string(head_url_file()).ok().map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn default_habitat_image_tag(pin: &str) -> String {
    format!("hale-habitat:{}", &pin[..12.min(pin.len())])
}

pub(crate) fn default_habitat_image() -> String {
    default_habitat_image_tag(HABITAT_PIN)
}

/// Whether the habitat's Board trusts the nursery `source`
/// (`habitat.nursery`, `hale habitat nursery add`).
pub(super) fn trusts_nursery(source: &str) -> bool {
    sapling::trusted_nurseries().iter().any(|s| s == source)
}

/// The verb's help: stdout and exit 0 for `--help`, stderr and exit 2
/// for a usage error (GH #817).
pub(crate) fn usage_text() -> &'static str {
    r#"usage: hale habitat [--head <url>] up [--no-body] [--profile local] [--backends own|operator] [--name <name>] [--image <tag>]
       hale habitat up --host <name> [--user <user>] [--port <n>] [--name <name>] [--image <tag>] [--backends own|operator]
       hale habitat image build [<checkout>]
       hale habitat [--head <url>] graft <dir> [--attach] [--no-library] [--no-pack]
       hale habitat graft --resume <path on the habitat's machine> [flags]
                          [--profile local|remote-body [--remote <url>] [--body <user@host>]]
                          [--with <pack> [--set <param>=<value>]… [--as <who>]]
       hale habitat [--head <url>] graft <sapling> [--host <name>] [--own <part>[=<host>]]… [--reuse <part>]… [--plan] [--no-library] [--no-pack] [--set <param>=<value>]… [--no-head]
       hale habitat [--head <url>] place <key>/<role> <host>
       hale habitat [--head <url>] model-map <organism> <mode>.<size> <model> [--endpoint <url>] [--key-vault <slot>] [--tools]
       hale habitat [--head <url>] model-probe <organism> <mode>.<size>
       hale habitat [--head <url>] apply [<host>] [--fresh]
       hale habitat [--head <url>] nursery add <dir|git-url> | refresh | list | ratify <sapling>
       hale habitat [--head <url>] forge <organism> forgejo|github|file|none [--repo <owner/name>] [--key-vault <slot>] [--no-head]
       hale habitat handover [organism]
       hale habitat [--head <url>] host add <name> [--user <user>] [--port <n>]
       hale habitat [--head <url>] host probe <name>
       hale habitat [--head <url>] host remove <name> [--plan]
       hale habitat [--head <url>] organism remove <name> [--plan]
       hale habitat organism <name> dna <verb> [args…]
       hale habitat [--head <url>] brief
       hale habitat [--head <url>] status
       hale habitat down [--forget [--plan]]
       hale habitat migrate [organism]
       hale habitat sweep <root> | --test-vaults [<root>]
       hale habitat resolve <name>

The local habitat holds every organism's infrastructure on this machine:
one docker compose project, named for the habitat, with a service per
allocation (memory-<id>, nerves-<id>, senses-<id>, <id> the organism's
record identity), under ${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local.
It is long-lived: nothing tears it down when work ends, and two checkouts
of one organism are the same tenant.

up                   birth the habitat the first time, from the genome the
                     toolchain ships (its positions the nursery's `habitat`
                     pack, its law the `standard` pack and its document's
                     `law` items), the one organism born without a parent;
                     then bring every recorded allocation up through the
                     compose provider, migrating any whose recorded schema
                     version is behind this toolchain's; render its own
                     `body` and `face` services into its compose project
                     (built from the embedded Dockerfile at `hale.pin`, or
                     `--image <tag>` / `HALE_HABITAT_IMAGE=<tag>`), bring
                     them up with `docker compose up -d`, wait on their
                     probes, and record its placement (the host `local` with
                     what it offers, the profile's policy, its head and face
                     on that host, rows of its record) and the head's
                     address (on its `habitat.profile` row and in
                     `head.url`); --no-body births it and brings its
                     backing allocations up and starts no body or face (what
                     a test's habitat needs to graft into). Idempotent. Its
                     backends are chosen at its birth and kept: `own` (the
                     default) provisions memory, the nerves and the senses'
                     store as its project's allocations; `operator` adopts
                     the Postgres and the NATS HALE_DNA_MEMORY_DSN_OWNER and
                     HALE_DNA_NATS_URL_OWNER name, and provisions nothing
                     of its own. --name names the habitat at its birth (a
                     DNS label: lowercase letters, digits and `-`, at most
                     63; `local` when none is given), recorded on its
                     `habitat.profile` row and kept: `status`, the head's
                     Commands::index and the operator face show it. The
                     hosted profile is not built.
up --host <name>     run the habitat on another machine, one with docker and
                     nothing else: reach it over SSH as the person's own
                     login (--user and --port, else the SSH config's;
                     HALE_HABITAT_SSH_CONFIG names a config file for
                     `ssh -F`), put the toolchain image there (there
                     already, sent from this machine's docker, or pulled
                     as the published image of this toolchain's version
                     when it answers as this toolchain does; nothing is
                     compiled there), run `up` there in that image, wait
                     on its head and face at http://<name>:<port>, and keep
                     a pointer here (the name, the host, the head's URL and
                     launch token, the record's identity) in place of a
                     habitat. The face and head bind the address <name>
                     resolves to here. Every verb here reaches the head
                     through the pointer; `status` asks it, `down` runs
                     there over SSH (`--forget` planned and confirmed
                     there by the name typed at this terminal, the pointer
                     removed last), `graft <dir>` runs there over SSH (a
                     directory here shipped there as the habitat's copy,
                     the organism's home, in a directory of its own made
                     fresh there; `graft --resume <path>` goes on with a
                     graft that stopped, in the directory it kept there,
                     nothing shipped), `organism <name> dna <verb>`
                     runs an organism's own verbs there, and `migrate` and
                     `handover`, which act on a genome here, are refused.
                     Run again, it finds the same habitat there by its
                     record's identity and changes nothing.
image build [<dir>]  build the habitat toolchain image once from the local
                     checkout at <dir> (`.` by default) as context and print
                     its tag.
--head <url>         reach the habitat's head at <url> (`HALE_DNA_HEAD_URL`
                     names the same thing; precedence: `--head`,
                     `HALE_DNA_HEAD_URL`, then the recorded address).
graft <dir>          birth an organism as the habitat's child, the one way an
                     organism other than the habitat is born: its record's
                     genesis names the habitat's record as its parent, the
                     roles its genome needs (`[dna] needs` in its hale.toml:
                     memory and nerves always, senses unless left out;
                     undeclared, as a foreign heart is, it is exactly the
                     organism's own, never a heart's; a Hale heart may
                     declare `database`, a tenant database of its memory
                     server handed to it as HALE_DNA_DATABASE_DSN) are
                     allocated to it and recorded in the habitat's record,
                     and the `standard` pack is proposed to its Board
                     (--no-pack leaves it for `hale dna pack apply`).
                     <dir> is a new application's directory, or with
                     --attach an application (its workspace's manifest is
                     the organism's) or a repository that is there
                     already. A graft that stopped part way goes on
                     where it stopped when run again; a record born of
                     anything but this habitat is refused. A new
                     application takes --profile remote-body --remote
                     <url> [--body <user@host>]: the record shared over
                     that remote, the body on that server. --with <pack>
                     proposes a pack from the toolchain's nursery as
                     well, planned on the holes the birth proposed (`hale
                     dna pack apply <pack> --on-holes`, its --set and
                     --as passed on), so one `review all approve`
                     ratifies the starting organization whole.
graft <sapling>      graft a sapling the habitat pins, by name, with no
                     checkout: its pinned revision is fetched into the
                     habitat's store (<habitat>/saplings/<name>), its digest
                     checked (a mismatch refuses, nothing grafted), the
                     repository there grafted as the habitat's child, the
                     packs it adopts proposed to its Board (--set values go
                     to them), and the sapling, revision and digest recorded
                     on the graft's row. A sapling the toolchain carries
                     (`hale`) is no fetch: its tree is made the organism's
                     own repository there, with no remote, and its digest
                     checked the same way. A bare name only; ./<name> is a
                     directory.
--no-head            (graft) the act itself: a sapling's graft is otherwise
                     the head's Commands::graft while the head answers.
--host <name>        (graft) place the new organism's application, its own
                     services (a compose.yaml at its root), on that host,
                     brought up there by its provider.
--own <part>[=<host>]
--reuse <part>       (graft) how each body part is filled. The organism
                     plane (memory, nerves, senses) reuses the habitat's,
                     on its machine, unless --own gives it a server of its
                     own (on <host>, else on --host). The application plane
                     (database, broker) is the application's own when its
                     manifest says it brings it ([dna] brings), else the
                     habitat provisions it; --reuse has the habitat
                     provide it, --own leaves it to the application.
--plan               (graft) print the plan, part by part, and graft
                     nothing. At a terminal a graft prints its plan and
                     asks before it acts.
model-map <organism> <category> <model>
                     point a grafted organism's model map at an endpoint
                     (the head's Commands::set_model_map: `hale dna models
                     category … --endpoint <url> [--key-vault <slot>]` run
                     in its directory); a key only by its vault slot's name.
model-probe <organism> <category>
                     ask the model that category names, where the map says
                     it is asked, once (Commands::probe_model).
place <alloc> <host> place an allocation (`<key>/<role>`: memory, nerves,
                     senses, services) on a host (the head's
                     Commands::place_allocation); nothing applies yet.
apply [<host>]       apply the allocations (of one host, or all) where they
                     are placed (Commands::apply_allocations): each one that
                     moved is withdrawn where it ran and brought up where it
                     is placed, by that host's provider, and the pulse after
                     is printed. A moved memory starts empty on its new
                     host: a memory that holds its organism's record moves
                     only with --fresh, and the habitat's own never.
nursery add <source> the Board trusts a nursery, a directory or a git
                     repository (the head's Commands::add_nursery), and what
                     it offers is read into the habitat's record. `refresh`
                     reads every nursery again (`up` does too): a sapling
                     first seen is pinned, a newer revision of one pinned is
                     filed for the Board and never taken; `ratify <sapling>`
                     pins the revision filed (nothing grafted moves); `list`
                     shows the nurseries and saplings. The toolchain's own
                     nursery is always one.
forge <organism> <provider>
                     allocate the organism's forge (`habitat` for the
                     habitat's own): forgejo (the habitat's own Forgejo,
                     the local profile's default: a user and a repository
                     there, its token a vault slot), github with --repo
                     owner/name and --key-vault, the vault slot of its
                     token (`forge-token` by default; never the token),
                     file (a forge of files, the fixtures') or none (a bare
                     remote; Reviews decided in the face or CLI). The head's
                     Commands::allocate_forge while the head answers; the
                     act itself with --no-head. `hale dna forge sync`
                     syncs with what is allocated. A graft takes
                     --forge <provider> over the habitat's policy.
handover [organism]  what the habitat hands the organism's heart, one
                     NAME=value a line, a secret only by its vault slot's
                     name: HALE_DNA_APPLICATION_ID, HALE_DNA_HEAD_URL,
                     HALE_DNA_OIDC_ISSUER, HALE_DNA_OIDC_CLIENT and
                     HALE_DNA_OIDC_CLIENT_VAULT, and for each heart role
                     its genome declares, HALE_DNA_DATABASE_DSN or
                     HALE_DNA_BROKER_URL, _USER, _VAULT, _SUBJECTS,
                     _NODE_USER and _NODE_VAULT. `hale dna dev` hands the
                     heart the same.
host add <name>      add a machine on the LAN, by the name it resolves under,
                     as a host the habitat places on over SSH (port 22
                     unless --port, as --user or this user): prints the
                     host key first seen there (the one trusted after it)
                     and the habitat's public key. With no --user, the
                     habitat logs in as an account of its own there,
                     hale-<id>: over your own SSH login (as up --host
                     reaches a machine), with your sudo, it makes that
                     account (no sudo of its own; in the docker group,
                     which is root-equivalent on the host), puts the key in
                     it and probes the host ready. With --user, the key goes
                     into that account. When your login does not reach the
                     machine, what to do there is printed, and the host is
                     pending-key.
host probe <name>    probe it again over SSH: what it offers (docker
                     compose, the user's systemd, its OS and architecture);
                     a pending-key host whose key is now authorized is
                     ready. A host that answers with another host key is
                     refused.
host remove <name>   remove it, planned first: the plan lists each allocation
                     placed or applied there and what stops or is deleted
                     on that host, the record's rows about it, and the line
                     its authorized_keys took, which this CLI then removes
                     over your own SSH login. The services placed there
                     (organism place) are taken off first, each by its
                     own uninstall over your SSH login, with what placing
                     installed there. When the plan stops or deletes anything there,
                     nothing is touched until the host's name is typed back
                     at a terminal; --plan prints the plan and exits.
                     The host verbs are the Board's commands of the
                     habitat's head (Commands::add_host, ::probe_host,
                     ::plan_remove_host, ::remove_host): its body must be
                     up.
organism remove <name>
                     remove an organism the habitat grafted, planned first:
                     its record and memory, its genome repository (the
                     habitat's clone of a sapling; a checkout of the
                     person's is kept), its Forgejo repository and account,
                     its allocations by host, its compose services and
                     projects, its vault slots (by name) and its files
                     here, and its rows in the habitat's record marked
                     removed. Nothing is touched until its name is typed
                     back at a terminal; --plan prints the plan and exits.
                     The head's Commands::plan_remove_organism and
                     ::remove_organism. The habitat itself is removed by
                     `down --forget`.
organism <name> dna <verb> [args…]
                     `hale dna <verb>` in the grafted organism's own
                     directory (`review`, `task create`, `work run`,
                     `show`, `secret set`, …): through a pointer
                     (`up --host`), on the habitat's machine over SSH, at
                     this terminal when there is one, after the pointer's
                     record is checked there; stdin goes through, so a
                     value piped to `secret set` arrives.
brief                the Board's hat as a prompt, with the habitat's live
                     state: its hosts, its allocations and their pulse, its
                     organisms, its saplings, and what waits for the Board.
                     The head's read Commands::brief, asked of the head
                     as the person this account maps to; an agent pairing
                     with that person asks the same row over MCP (the
                     endpoint `status` prints). It reads the record
                     alone, never the vault: no secret value is in it.
status               the habitat, its genome and body, its face and, while
                     the face runs, its head's MCP endpoint, the services
                     placed on its hosts (organism place) under their
                     hosts with their readings endpoints, and each
                     allocation: its organism, whether each role's service
                     runs, the schema version memory and the nerves record
                     against this toolchain's, and how a heart's declared
                     migration last ran.
down                 stop the habitat's body, its face and every service of
                     its compose project (`compose down`); its volumes are
                     kept, and `up` brings them back. --forget removes the
                     habitat whole, planned
                     first: its record and memory, each grafted organism
                     (its record, genome repository, forge repository and
                     allocations by host), the compose projects and their
                     volumes, its vault slots (by name), its Forgejo and
                     the accounts there, its hosts' entries and what it
                     runs on each over SSH, each line with its size or
                     count and whether it is recoverable. Nothing is
                     touched until the habitat's name is typed back at a
                     terminal; there is no other way to confirm, and
                     without a terminal --forget refuses. --plan prints the
                     plan and exits. A forget that fails part way says what
                     was done and what was not, and run again plans from
                     what remains. The line each host's authorized_keys
                     took is noted, never removed.
migrate [organism]   bring the organism's allocations up and migrate them:
                     memory's schema and the nerves' stream move to this
                     toolchain's, and the head's DSN is kept in the
                     checkout's .hale/dna/memory.env. With
                     HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER
                     set, that server of your own is migrated instead.
                     Idempotent. The organism defaults to the current
                     directory. Each role's version is recorded in the
                     allocation's allocation.json.

resolve <name>       the first IPv4 address <name> resolves to on this
                     machine: what a compose project placed on that host is
                     handed as HALE_DNA_HOST_LISTEN. Exit 1 when it resolves
                     to none.
sweep <root>         take down every compose project run from under <root>,
                     a test's habitat, or named by a directory under it (a
                     host's `.hale/habitat/<project>`, a habitat born
                     there) and run from nowhere else: its containers,
                     networks and volumes, confirmed gone by the
                     project's label. The
                     projects are named in <root>/.dna-compose-projects
                     until they are, so a teardown that did not finish is
                     retried. Never the developer's habitat (under $HOME),
                     nor the one XDG_DATA_HOME names outside <root>. Exit 1
                     names what survived; nothing is done without docker.
                     --test-vaults sweeps the test vaults of processes
                     that are gone (<tmp>/hale-test-vaults-<uid>, or
                     <root>): each one's habitat comes down before the
                     vault goes, and a habitat whose vault went first is
                     found by its compose working directory.

Nothing else migrates or provisions: `hale dna dev`, `run`, `head`, `show`
and `work` run on what is up and migrated, and name this command when it
is not.
"#
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let mut cleaned = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--head" {
            let Some(url) = args.get(i + 1).filter(|u| !u.is_empty()) else {
                eprintln!("hale habitat: `--head` takes a URL\n{}", usage_text());
                return ExitCode::from(2);
            };
            let _ = HEAD_FLAG.set(url.trim_end_matches('/').to_string());
            i += 2;
            continue;
        }
        if let Some(url) = args[i].strip_prefix("--head=") {
            if url.is_empty() {
                eprintln!("hale habitat: `--head` takes a URL\n{}", usage_text());
                return ExitCode::from(2);
            }
            let _ = HEAD_FLAG.set(url.trim_end_matches('/').to_string());
            i += 1;
            continue;
        }
        cleaned.push(args[i].clone());
        i += 1;
    }
    let args = &cleaned[..];
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{}", usage_text());
            ExitCode::SUCCESS
        }
        Some(verb @ ("migrate" | "handover")) if remote::pointer().is_some() => remote::refuse_local(verb).unwrap_or(ExitCode::from(2)),
        // a directory (one here, or a path) is grafted on the habitat's
        // machine over SSH; a bare name is a sapling, the head's graft
        Some("graft") if args.get(1).map(String::as_str) == Some("--resume") => match remote::pointer() {
            Some(p) => remote::graft_resume(&p, &args[2..]),
            None => {
                eprintln!("hale habitat graft --resume: only through a pointer to a habitat on another machine (`hale habitat up --host`); here, run the graft again in the organism's directory");
                ExitCode::from(2)
            }
        },
        Some("graft") if args.get(1).is_some_and(|a| Path::new(a).is_dir() || a.contains('/')) && remote::pointer().is_some() => match remote::pointer() {
            Some(p) => remote::graft(&p, &args[1..]),
            None => ExitCode::from(2),
        },
        // a bare name through a pointer is a sapling: the head grafts it
        Some("graft") if args.get(1).is_some_and(|a| !a.starts_with('-')) && remote::pointer().is_some() => match remote::pointer() {
            Some(p) => remote::graft_sapling(&p, &args[1..]),
            None => ExitCode::from(2),
        },
        Some("organism") if args.get(2).map(String::as_str) == Some("dna") => match remote::pointer() {
            Some(p) => remote::organism_dna(&p, &args[1..]),
            None => organism_dna(&args[1..]),
        },
        Some("migrate") => migrate(args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))),
        Some("up") => up(&args[1..]),
        Some("image") => image_cmd(&args[1..]),
        Some("graft") => graft(&args[1..]),
        Some("handover") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match project(&dir).and_then(|(root, _)| handover(&root, true)) {
                Ok(lines) => {
                    for (k, v) in lines {
                        println!("{k}={v}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hale habitat handover: {e}");
                    ExitCode::from(1)
                }
            }
        }
        Some("nursery") => sapling::nursery_cmd(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("organism") => organism(&args[1..]),
        Some("brief") if args.len() == 1 => brief(),
        Some("place") => sapling::place_cmd(&args[1..]),
        Some("model-map") => sapling::model_map_cmd(&args[1..], false),
        Some("model-probe") => sapling::model_map_cmd(&args[1..], true),
        Some("apply") => sapling::apply_cmd(&args[1..]),
        Some("forge") => sapling::forge_cmd(&args[1..]),
        Some("status") if args.len() == 1 => match remote::pointer() {
            Some(p) => remote::status(&p),
            None => status(),
        },
        Some("down") => match remote::pointer() {
            Some(p) => remote::down(&p, &args[1..]),
            None => down(&args[1..]),
        },
        Some("resolve") if args.len() == 2 => match first_ipv4(&args[1]) {
            Ok(ip) => {
                println!("{ip}");
                ExitCode::SUCCESS
            }
            Err(why) => {
                eprintln!("hale habitat resolve: {why}");
                ExitCode::from(1)
            }
        },
        Some("sweep") if args.get(1).map(String::as_str) == Some("--test-vaults") && args.len() <= 3 => {
            let root = args.get(2).map(PathBuf::from).unwrap_or_else(crate::verbs::test::test_vault_root);
            match crate::verbs::test::sweep_dead_test_vaults(&root) {
                Ok(()) => ExitCode::SUCCESS,
                Err(why) => {
                    eprintln!("hale habitat sweep: {why}");
                    ExitCode::from(1)
                }
            }
        }
        Some("sweep") if args.len() == 2 => match down_under(Path::new(&args[1])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("hale habitat sweep: {why}");
                ExitCode::from(1)
            }
        },
        _ => {
            eprint!("{}", usage_text());
            ExitCode::from(2)
        }
    }
}

/// `hale habitat image build [<checkout>]`: build the habitat toolchain
/// image once from the local checkout as context.
fn image_cmd(args: &[String]) -> ExitCode {
    let usage = "usage: hale habitat image build [<checkout>]";
    let dir = match (args.first().map(String::as_str), args.get(1), args.len()) {
        (Some("build"), None, 1) => PathBuf::from("."),
        (Some("build"), Some(d), 2) if !d.starts_with('-') => PathBuf::from(d),
        _ => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let dockerfile = dir.join("Dockerfile");
    if !dockerfile.is_file() {
        eprintln!("hale habitat image build: no Dockerfile in {}", dir.display());
        return ExitCode::from(1);
    }
    let tag = std::env::var("HALE_HABITAT_IMAGE").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(default_habitat_image);
    let status = Command::new("docker")
        .args(["build", "-t", &tag, "-f"])
        .arg(&dockerfile)
        .arg(&dir)
        .stdout(std::process::Stdio::from(std::io::stderr()))
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("{tag}");
            ExitCode::SUCCESS
        }
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat image build: docker: {e}");
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------
// hosts: a thin client of the habitat head's commands (phase 4)
// ---------------------------------------------------------------

pub(crate) fn head_url_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("head.url")
}

/// The habitat head's HTTP address: `--head <url>`, then `HALE_DNA_HEAD_URL`,
/// then the local habitat's `head.url` state file, then the `habitat.profile`
/// row in its record, then `dna.head.url` + 1.
pub(crate) fn head_endpoint() -> Option<String> {
    if let Some(u) = head_flag_or_local_url() {
        return Some(u);
    }
    let genome = genome_dir();
    if !IN_HEAD_ENDPOINT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let found = rows_of(&genome)
            .iter()
            .rev()
            .find(|r| r["kind"] == "habitat.profile")
            .and_then(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()))
            .and_then(|b| b["head"].as_str().map(|s| s.trim().trim_end_matches('/').to_string()))
            .filter(|s| !s.is_empty());
        IN_HEAD_ENDPOINT.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(u) = found {
            return Some(u);
        }
    }
    head_url_at(&genome).as_deref().and_then(loopback_port).map(|base| format!("http://{}:{}", habitat_address(), base + 1))
}

/// The link that signs the person in to the habitat's face: the face's URL
/// with the head's launch token, which opens `/` once and sets the session
/// cookie (`dna/api/project_service/head.hl`). "" when no token is kept here.
pub(super) fn sign_in_link(face: &str) -> String {
    let token = head_token();
    if token.is_empty() || face.is_empty() {
        return String::new();
    }
    format!("{}/?token={token}", face.trim_end_matches('/'))
}

/// What a birth or a sapling's graft proposes is the packaging the person
/// chose by that act, so the act ratifies it in their name: `hale dna review
/// all approve --no-wait`, every pending group (purpose, the practices it
/// ships with, positions, …) decided at once, its verdicts executed when
/// the body runs. The organism starts governed instead of waiting on its
/// own defaults; anything proposed after is the Board's to review. A
/// directory grafted as a new application is not packaging anyone vetted,
/// and keeps its proposals. A line saying what was done, or how to finish.
pub(super) fn ratify_packaged(root: &Path, act: &str, what: &str) -> String {
    let comment = format!("ratified at {act}: {what}, the packaging the act chose");
    let ran = std::env::current_exe().map_err(|e| e.to_string()).and_then(|me| {
        Command::new(me)
            .current_dir(root)
            .args(["dna", "review", "all", "approve", "--no-wait", "--comment", &comment])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())
    });
    match ran {
        Ok(o) if o.status.success() => format!("ratified {what}, at {act}"),
        Ok(o) => format!(
            "{:<8}{what} proposed but not ratified ({}): `hale dna review all approve` in {} ratifies it",
            "note",
            String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("no reason given"),
            root.display()
        ),
        Err(e) => format!("{:<8}{what} proposed but not ratified ({e}): `hale dna review all approve` in {} ratifies it", "note", root.display()),
    }
}

pub(crate) fn head_token() -> String {
    std::fs::read_to_string(face_state_dir().join("head.token"))
        .or_else(|_| std::fs::read_to_string(genome_dir().join(".hale/dna/head.token")))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Whether the habitat's head is reachable (over an explicit
/// `HALE_DNA_RPC_SOCKET` or at `head_endpoint()`).
pub(super) fn head_is_up() -> bool {
    if let Some(given) = std::env::var("HALE_DNA_RPC_SOCKET").ok().filter(|v| !v.is_empty()) {
        return Path::new(&given).exists();
    }
    let Some(url) = head_endpoint() else { return false };
    if let Some(port) = loopback_port(&url) {
        return std::net::TcpStream::connect_timeout(&listen_socket(port), std::time::Duration::from_millis(500)).is_ok();
    }
    true
}

/// One call of the habitat head's surface over HTTP (`POST <endpoint>/call/<member>`).
pub(super) fn http_surface_call(endpoint: &str, member: &str, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = format!("{}/call/{member}", endpoint.trim_end_matches('/'));
    let body_str = payload.to_string();
    let token = head_token();
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-w", "\n%{http_code}", "-X", "POST", "-H", "Content-Type: application/json", "-H", "Hale-Surface-Digest: "]);
    if !token.is_empty() {
        cmd.args(["-H", &format!("Authorization: Bearer {token}")]);
    }
    cmd.args(["--data-binary", &body_str, &url]);
    let out = cmd.output().map_err(|e| format!("the habitat's head did not answer: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code_str) = text.rsplit_once('\n').unwrap_or((&text, "0"));
    let status: u16 = code_str.trim().parse().unwrap_or(0);
    if status == 0 {
        return Err("the habitat's head is not up (`hale habitat up` starts it)".to_string());
    }
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    if status == 200 {
        if parsed.get("value").is_some() && parsed.get("id").is_some() {
            return Ok(parsed["value"].clone());
        }
        return Ok(parsed);
    }
    let why = parsed["refusal"]["reason"]
        .as_str()
        .or_else(|| parsed["error"]["message"].as_str())
        .or_else(|| parsed["error"].as_str())
        .unwrap_or_else(|| body.trim())
        .to_string();
    Err(format!("the habitat's head refused {member}: {why}"))
}

fn host(args: &[String]) -> ExitCode {
    let usage = "usage: hale habitat host add <name> [--user <user>] [--port <n>] | probe <name> | remove <name> [--plan]";
    if args.first().map(String::as_str) == Some("remove") {
        return match (args.get(1), args.get(2).map(String::as_str), args.len()) {
            (Some(n), None, 2) if !n.starts_with('-') => host_remove(n, false),
            (Some(n), Some("--plan"), 3) if !n.starts_with('-') => host_remove(n, true),
            _ => {
                eprintln!("{usage}");
                ExitCode::from(2)
            }
        };
    }
    let (verb, name) = match (args.first().map(String::as_str), args.get(1)) {
        (Some(v @ ("add" | "probe")), Some(n)) if !n.starts_with('-') => (v, n.clone()),
        _ => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let mut payload = serde_json::json!({ "name": name });
    let mut i = 2;
    while i < args.len() {
        match (verb, args[i].as_str(), args.get(i + 1)) {
            ("add", "--user", Some(u)) => payload["user"] = serde_json::json!(u),
            ("add", "--port", Some(p)) => match p.parse::<u16>() {
                Ok(p) if p > 0 => payload["port"] = serde_json::json!(p),
                _ => {
                    eprintln!("hale habitat host: `--port {p}` is not a port");
                    return ExitCode::from(2);
                }
            },
            (_, other, _) => {
                eprintln!("hale habitat host {verb}: `{other}` is not a flag of `host {verb}`\n{usage}");
                return ExitCode::from(2);
            }
        }
        i += 2;
    }
    // with no --user, the habitat logs in as an account of its own there,
    // made below over the person's login, never as the person
    let own_user = verb == "add" && payload.get("user").is_none();
    if own_user {
        match habitat_identity() {
            Some(id) => payload["user"] = serde_json::json!(remote::habitat_user(&id)),
            None => {
                eprintln!("hale habitat host add {name}: the habitat has no record identity yet (`hale habitat up` births it)");
                return ExitCode::from(1);
            }
        }
    }
    let login_port = payload["port"].as_u64().and_then(|n| u16::try_from(n).ok());
    let member = if verb == "add" { "Commands::add_host" } else { "Commands::probe_host" };
    let v = match host_call(member, payload) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let v = &v;
    let h: serde_json::Value = v["host"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let field = |k: &str| h[k].as_str().unwrap_or("").to_string();
    if v["ok"] != serde_json::json!(true) {
        eprintln!("hale habitat host {verb} {name}: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
        return ExitCode::from(1);
    }
    let offers = if field("offers").is_empty() { "nothing yet".to_string() } else { field("offers") };
    println!("{:<8}{name} ({}, {}@{}:{}): {}", "host", field("provider"), field("user"), field("address"), h["port"].as_i64().unwrap_or(0), field("state"));
    // a ready host's probe checks its exporter too, over the person's own
    // login: installed when it is missing, replaced when it runs with other
    // flags than this toolchain's
    if verb == "probe" && field("state") == "ready" && !readings::node_exporter_off() {
        let login = if remote::is_habitat_user(&field("user")) { None } else { Some(field("user")) };
        let port = h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
        progress(&format!("checking the node and container exporters on {name}, over your SSH login"));
        for l in readings::install_over(&name, login.as_deref(), port) {
            println!("{l}");
        }
    }
    println!("{:<8}{offers}{}", "offers", if field("os").is_empty() { String::new() } else { format!(" ({} {})", field("os"), field("arch")) });
    if verb == "add" {
        println!("{:<8}{} {} (trusted from now on: another key is refused)", "hostkey", field("fingerprint"), field("host_key").split(' ').next().unwrap_or(""));
        println!("{:<8}{}", "key", v["public_key"].as_str().unwrap_or(""));
        if field("state") != "ready" {
            // over the person's own login, as `up --host` reaches a machine:
            // the key appended there, then the host probed again
            let port = h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
            let line = v["authorize"].as_str().unwrap_or("");
            let done = if own_user {
                println!("making the habitat's own account {} on {name} over your SSH login (no sudo of its own; on Linux in the docker group, which is root-equivalent on that host; on macOS a hidden account with no password); sudo may ask for your password:", field("user"));
                remote::provision_user(&name, login_port, &field("user"), line)
            } else {
                remote::authorize_key(&name, &field("user"), port, line)
            };
            match done {
                Ok(()) => {
                    println!("authorized the habitat's key in {}@{name}:~/.ssh/authorized_keys, over your SSH login", field("user"));
                    match host_call("Commands::probe_host", serde_json::json!({ "name": name })) {
                        Ok(p) => {
                            let ph: serde_json::Value = p["host"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
                            let pf = |k: &str| ph[k].as_str().unwrap_or("").to_string();
                            println!("{:<8}{name}: {}{}", "probed", pf("state"), if pf("offers").is_empty() { String::new() } else { format!(", offers {}", pf("offers")) });
                            if pf("state") != "ready" {
                                println!("{:<8}{}", "note", p["why"].as_str().unwrap_or("not ready yet: `hale habitat host probe` checks it again"));
                            }
                            if !readings::node_exporter_off() {
                                // the machine's readings, over the same login the account was made over
                                let login = if own_user { None } else { Some(field("user")) };
                                progress(&format!("installing the node and container exporters on {name}, over your SSH login"));
                                for l in readings::install_over(&name, login.as_deref(), if own_user { login_port } else { port }) {
                                    println!("{l}");
                                }
                            }
                        }
                        Err(code) => return code,
                    }
                }
                Err(why) => {
                    println!("{name} was not set up over your SSH login ({why}); on {name}, {}append this line to ~/.ssh/authorized_keys of {}, then `hale habitat host probe {name}`:", if own_user { format!("make the user {} (in the docker group), ", field("user")) } else { String::new() }, field("user"));
                    println!("{line}");
                }
            }
        }
    } else if let Some(why) = v["why"].as_str().filter(|w| !w.is_empty()) {
        println!("{:<8}{why}", "note");
    }
    ExitCode::SUCCESS
}

/// `hale habitat organism remove <name> [--plan]`: the organism's removal,
/// planned (`Commands::plan_remove_organism`), printed, and confirmed by its
/// name typed back at a terminal, which `Commands::remove_organism` checks
/// before it executes the plan item by item. `--plan` prints it and exits.
/// The directory of the organism the habitat grafted as `name` (or keyed
/// `name`), from its `habitat.grafted` row and its allocation's cache.
fn organism_root(name: &str) -> Result<PathBuf, String> {
    let keys = live_grafts_named(&rows_of(&genome_dir()), name);
    let key = match keys.as_slice() {
        [k] => k.clone(),
        [] => return Err(format!("the habitat grafted no organism `{name}`")),
        _ => return Err(format!("the habitat grafted more than one organism named `{name}`: name it by its key")),
    };
    let cache = PathBuf::from(local_habitat_dir()).join("allocations").join(&key).join("allocation.json");
    let root = std::fs::read_to_string(&cache).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v["root"].as_str().map(PathBuf::from));
    match root {
        Some(r) if r.is_dir() => Ok(r),
        Some(r) => Err(format!("the organism `{name}`'s directory {} is not on this machine", r.display())),
        None => Err(format!("the organism `{name}` ({key}) names no directory in {}", cache.display())),
    }
}

/// The keys of the organisms the habitat holds grafted under `name` (or
/// keyed `name`): each entity's latest `habitat.grafted` row, an organism
/// whose latest row says it was removed left out (as `grafted_organism`
/// reads them, dna/core/teardown.hl).
fn live_grafts_named(rows: &[serde_json::Value], name: &str) -> Vec<String> {
    let mut latest: std::collections::BTreeMap<String, serde_json::Value> = std::collections::BTreeMap::new();
    for r in rows.iter().filter(|r| r["kind"] == "habitat.grafted") {
        if let Some(e) = r["entity"].as_str() {
            let body = r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).unwrap_or_default();
            latest.insert(e.to_string(), body);
        }
    }
    latest.into_iter().filter(|(_, b)| b["state"].as_str() != Some("removed")).filter(|(k, b)| k == name || b["name"].as_str() == Some(name)).map(|(k, _)| k).collect()
}

/// `hale habitat organism <name> dna <verb> [args…]`: `hale dna <verb>` in
/// the organism's own directory, as it runs there (through a pointer, on
/// the habitat's machine: remote::organism_dna).
fn organism_dna(args: &[String]) -> ExitCode {
    let (Some(name), Some(verb)) = (args.first(), args.get(2)) else {
        eprintln!("usage: hale habitat organism <name> dna <verb> [args…]");
        return ExitCode::from(2);
    };
    let _ = verb;
    let root = match organism_root(name) {
        Ok(r) => r,
        Err(why) => {
            eprintln!("hale habitat organism {name}: {why}");
            return ExitCode::from(1);
        }
    };
    let me = match std::env::current_exe() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("hale habitat organism {name}: {e}");
            return ExitCode::from(1);
        }
    };
    match Command::new(me).arg("dna").args(&args[2..]).current_dir(&root).status() {
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat organism {name}: {e}");
            ExitCode::from(1)
        }
    }
}

fn organism(args: &[String]) -> ExitCode {
    use std::io::{BufRead, IsTerminal, Write};
    match args.first().map(String::as_str) {
        Some("place") => return hosted::place(&args[1..], true),
        Some("unplace") => return hosted::place(&args[1..], false),
        _ => {}
    }
    let usage = "usage: hale habitat organism remove <name> [--plan] | place|unplace <name> <command> --host <host> [--id <id>] [--plan]";
    let (name, plan_only) = match (args.first().map(String::as_str), args.get(1), args.get(2).map(String::as_str), args.len()) {
        (Some("remove"), Some(n), None, 2) if !n.starts_with('-') => (n.clone(), false),
        (Some("remove"), Some(n), Some("--plan"), 3) if !n.starts_with('-') => (n.clone(), true),
        _ => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    if !plan_only && !std::io::stdin().is_terminal() {
        eprintln!("hale habitat organism remove {name}: stdin is not a terminal; removing an organism deletes its record and everything the habitat holds for it, and is confirmed only by typing its name at a terminal (`--plan` prints the plan)");
        return ExitCode::from(2);
    }
    let planned = match host_call("Commands::plan_remove_organism", serde_json::json!({ "name": name })) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if planned["ok"] != serde_json::json!(true) {
        eprintln!("hale habitat organism remove {name}: {} ({})", planned["why"].as_str().unwrap_or(""), planned["code"].as_str().unwrap_or(""));
        return ExitCode::from(1);
    }
    let items: Vec<serde_json::Value> = planned["plan"].as_str().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    let confirm = planned["confirm"].as_str().unwrap_or("").to_string();
    println!("the plan to remove the organism {confirm} (nothing is touched until its name is typed back):");
    for (group, heading) in [("record", "its record and memory"), ("genome", "its genome repository"), ("forge", "its Forgejo repository and account"), ("allocations", "its allocations"), ("compose", "its compose services and projects"), ("vault", "its vault slots (names only)"), ("remote", "on the hosts, over SSH")] {
        let of: Vec<&serde_json::Value> = items.iter().filter(|i| i["group"] == group).collect();
        if of.is_empty() {
            continue;
        }
        println!();
        println!("{heading}");
        for i in of {
            println!("  - {}", plan_line(i));
        }
    }
    // the services placed for it on the hosts: taken off first, from here
    let (current, history) = placements::from_head().unwrap_or_default();
    let placed: Vec<placements::Placement> = current.into_iter().filter(|p| p.organism == confirm).collect();
    if !placed.is_empty() {
        println!();
        println!("its services placed on the hosts, from this machine");
        for l in placements::plan_lines(&placed) {
            println!("  - {l}");
        }
    }
    if plan_only {
        return ExitCode::SUCCESS;
    }
    print!("\ntype the organism's name ({confirm}) to remove it and all of the above: ");
    let _ = std::io::stdout().flush();
    let mut typed = String::new();
    let _ = std::io::stdin().lock().read_line(&mut typed);
    if !placed.is_empty() {
        if typed.trim() != confirm {
            eprintln!("hale habitat organism remove {name}: `{}` is not the organism's name, and nothing was removed (confirm_mismatch)", typed.trim());
            return ExitCode::from(1);
        }
        for l in placements::take_off(&placed, &history) {
            println!("{l}");
        }
    }
    let v = match host_call("Commands::remove_organism", serde_json::json!({ "name": name, "confirm": typed.trim(), "digest": planned["digest"] })) {
        Ok(v) => v,
        Err(code) => return code,
    };
    print_report(&v);
    if v["ok"] != serde_json::json!(true) {
        eprintln!("hale habitat organism remove {name}: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
        return ExitCode::from(1);
    }
    println!("the organism {confirm} is removed");
    ExitCode::SUCCESS
}

/// A plan's item as a line: what it is, its size or count, and whether it
/// is recoverable.
fn plan_line(i: &serde_json::Value) -> String {
    let field = |k: &str| i[k].as_str().unwrap_or("").to_string();
    let tail = match (field("measure").as_str(), field("recoverable").as_str()) {
        ("" | "-", "" | "-") => String::new(),
        ("" | "-", r) => format!(" (recoverable: {r})"),
        (m, "" | "-") => format!(" ({m})"),
        (m, r) => format!(" ({m}; recoverable: {r})"),
    };
    format!("{}{tail}", field("what"))
}

/// A removal's report, an item a line: done, failed (and why) or not done.
fn print_report(v: &serde_json::Value) {
    let report: Vec<serde_json::Value> = v["report"].as_str().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    for r in &report {
        let shown = match r["state"].as_str().unwrap_or("") {
            "done" => "done",
            "failed" => "FAILED",
            _ => "not done",
        };
        let why = r["why"].as_str().filter(|w| !w.is_empty()).map(|w| format!(": {w}")).unwrap_or_default();
        println!("{shown:<10}{}{why}", r["what"].as_str().unwrap_or(""));
    }
}

/// `member` of the habitat head's commands called over its surface with
/// `payload`: the value it answered, or the exit status after saying why
/// there is none.
fn host_call(member: &str, payload: serde_json::Value) -> Result<serde_json::Value, ExitCode> {
    match sapling::head_call(member, payload) {
        Ok(v) => Ok(v),
        Err(why) => {
            eprintln!("hale habitat host: {why}");
            Err(ExitCode::from(1))
        }
    }
}

/// `hale habitat host remove <name> [--plan]`: the removal's plan
/// (`Commands::plan_remove_host`) printed and, when it stops or deletes
/// anything on the host, confirmed by the host's name typed back at a
/// terminal, which `Commands::remove_host` checks before it executes the
/// plan item by item. `--plan` prints it and exits.
fn host_remove(name: &str, plan_only: bool) -> ExitCode {
    use std::io::{BufRead, IsTerminal, Write};
    let planned = match host_call("Commands::plan_remove_host", serde_json::json!({ "name": name })) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if planned["ok"] != serde_json::json!(true) {
        eprintln!("hale habitat host remove {name}: {} ({})", planned["why"].as_str().unwrap_or(""), planned["code"].as_str().unwrap_or(""));
        return ExitCode::from(1);
    }
    let items: Vec<serde_json::Value> = planned["plan"].as_str().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    // the services placed there: taken off first, from this machine
    let (current, history) = placements::from_head().unwrap_or_default();
    let placed: Vec<placements::Placement> = current.into_iter().filter(|p| p.host == name).collect();
    println!("the plan to remove the host {name}:");
    for l in placements::plan_lines(&placed) {
        println!("  - {l}");
    }
    for i in &items {
        println!("  - {}", plan_line(i));
    }
    if plan_only {
        return ExitCode::SUCCESS;
    }
    // a service placed there is taken off only on the host's name too
    let confirm = planned["confirm"].as_str().filter(|c| !c.is_empty()).map(str::to_string).unwrap_or_else(|| if placed.is_empty() { String::new() } else { name.to_string() });
    let mut typed = String::new();
    if !confirm.is_empty() {
        if !std::io::stdin().is_terminal() {
            eprintln!("hale habitat host remove {name}: stdin is not a terminal; removing the host stops and deletes what the habitat runs there, and is confirmed only by typing the host's name at a terminal (`--plan` prints the plan)");
            return ExitCode::from(2);
        }
        print!("\ntype the host's name ({name}) to remove it and all of the above: ");
        let _ = std::io::stdout().flush();
        let _ = std::io::stdin().lock().read_line(&mut typed);
    }
    if !placed.is_empty() {
        if typed.trim() != name {
            eprintln!("hale habitat host remove {name}: `{}` is not the host's name, and nothing was removed (confirm_mismatch)", typed.trim());
            return ExitCode::from(1);
        }
        for l in placements::take_off(&placed, &history) {
            println!("{l}");
        }
    }
    let v = match host_call("Commands::remove_host", serde_json::json!({ "name": name, "confirm": typed.trim(), "digest": planned["digest"] })) {
        Ok(v) => v,
        Err(code) => return code,
    };
    print_report(&v);
    if v["ok"] != serde_json::json!(true) {
        eprintln!("hale habitat host remove {name}: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
        return ExitCode::from(1);
    }
    let h: serde_json::Value = v["host"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let field = |k: &str| h[k].as_str().unwrap_or("").to_string();
    println!("{:<8}{name} ({}, {}@{}:{}): {}", "host", field("provider"), field("user"), field("address"), h["port"].as_i64().unwrap_or(0), field("state"));
    // the line its authorized_keys took, which the habitat cannot remove,
    // taken off over the person's own login
    let port = h["port"].as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
    if !readings::node_exporter_off() {
        for l in readings::remove_over(name, None, port) {
            println!("{l}");
        }
    }
    // the habitat's directories left empty there
    let tidied = placements::tidy_host(name, port, &[], &[]);
    if !tidied.is_empty() {
        println!("{tidied}");
    }
    if remote::is_habitat_user(&field("user")) {
        // the habitat's own account there, its home and key with it
        match remote::remove_user(name, port, &field("user")) {
            Ok(()) => println!("the habitat's account {} is removed from {name}, over your SSH login", field("user")),
            Err(why) => println!("the habitat's account {} is left on {name} ({why}): remove it there (Linux `sudo userdel -r {}`, macOS `sudo dscl . -delete /Users/{}`)", field("user"), field("user"), field("user")),
        }
    } else {
        for i in items.iter().filter(|i| i["group"] == "hosts" && i["host"] == name && i["args"].as_str().is_some_and(|a| !a.is_empty())) {
            let line = i["args"].as_str().unwrap_or("");
            let comment = line.rsplit(' ').next().unwrap_or("");
            match remote::unauthorize_key(name, &field("user"), port, comment) {
                Ok(()) => println!("the habitat's key is out of {}@{name}:~/.ssh/authorized_keys, over your SSH login", field("user")),
                Err(why) => println!("the habitat's key is left on {name} ({why}): remove the line ending `{comment}` from {}'s ~/.ssh/authorized_keys there", field("user")),
            }
        }
    }
    ExitCode::SUCCESS
}

fn migrate(dir: PathBuf) -> ExitCode {
    let root = match project(&dir) {
        Ok((root, _)) => root,
        Err(e) => {
            eprintln!("hale habitat migrate: {e}");
            return ExitCode::from(2);
        }
    };
    match migrate_lines(&root) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat migrate: {e}");
            ExitCode::from(1)
        }
    }
}

/// The organism at `root`'s allocations brought up and migrated: what was
/// done, a line each, or why it could not be.
fn migrate_lines(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    match memory_migrate(root) {
        Ok(MemoryPlan::Roles { .. }) => out.push(format!("memory  migrated; the head's DSN is kept in {}", root.join(".hale/dna/memory.env").display())),
        Ok(MemoryPlan::NoDatabase(why)) => return Err(format!("memory: {why}")),
        Err(e) => return Err(format!("memory: {e}")),
    }
    match nerves_migrate(root) {
        Ok(NervesPlan::Roles(_)) => out.push("nerves  migrated; the organization's stream is in place".to_string()),
        Ok(NervesPlan::NoServer(why)) => return Err(format!("nerves: {why}")),
        Err(e) => return Err(format!("nerves: {e}")),
    }
    // the heart's roles, when the genome declares them and its application
    // does not own them
    if provisioned_needs(root).iter().any(|r| r == "database") {
        out.push(provision_database(root, if own_servers() { "operator" } else { "own" })?);
    }
    if provisioned_needs(root).iter().any(|r| r == "broker") {
        out.push(provision_broker(root)?);
    }
    out.extend(run_heart_migrations(root)?);
    // the senses' store is the habitat's when memory and the nerves are:
    // an environment that names servers of its own names its own store.
    // Nothing reads it but the reflexes, so one that cannot come up is said
    if !own_servers() {
        out.push(senses_line(root));
    }
    Ok(out)
}

/// The senses' store of the organism at `root` brought up: what `up` and
/// `migrate` say of it. One that cannot come up is said, and nothing stops.
fn senses_line(root: &Path) -> String {
    match host_run("senses-up", root, &[]) {
        Ok(o) if !o.starts_with("none: ") => format!("senses  up ({})", o.trim().trim_start_matches("HALE_DNA_SENSES_URL=")),
        Ok(o) => format!("senses  not up: {}", o.trim().strip_prefix("none: ").unwrap_or(o.trim())),
        Err(e) => format!("senses  not up: {e}"),
    }
}

/// Whether this environment names memory or nerves of its own.
fn own_servers() -> bool {
    ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()))
}

// ---------------------------------------------------------------
// the allocation record (GH #1653)
// ---------------------------------------------------------------

/// The habitat's record identity: its pointer's when it runs on another
/// machine, else its genome's.
fn habitat_identity() -> Option<String> {
    let id = match remote::pointer() {
        Some(p) => p["identity"].as_str().map(str::to_string),
        None => manifest_pin(&genome_dir().join("hale.toml")).ok().flatten(),
    };
    id.filter(|i| i.len() >= 12)
}

/// The directory of the organism at `root`'s allocations, `None` when it
/// pins no record identity.
fn allocation_dir(root: &Path) -> Option<PathBuf> {
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten()?;
    (id.len() >= 12).then(|| PathBuf::from(local_habitat_dir()).join("allocations").join(&id[..12]))
}

/// GH #1653: an allocation is a row in the habitat's own record, as every
/// provisioning act is: `habitat.allocated <id>/<role>` the first time an
/// organism's role is provisioned (its record identity, the role, the
/// service that is its tenant in the habitat's project, the vault slots it
/// holds by name, never a value; never a checkout's path: an allocation is
/// the organism's, whichever of its checkouts migrated it) and
/// `habitat.migrated <id>/<role>` each time its schema version moves. The
/// rows are written into the habitat's record as soon as it exists: an
/// allocation made before the habitat is born (its own, at bootstrap, or
/// an organism's migrated first) waits in the cache and is written at
/// birth. `allocations/<id>/allocation.json` is that cache, derived from
/// the rows; `status` reads the rows when the habitat's memory is up. An
/// allocation on an operator's own server (the habitat's `operator`
/// backends) is recorded the same way, its service `operator`.
pub(super) fn record_allocation(root: &Path, role: &str, version: u64, slots: &str, operator: bool) -> Result<(), String> {
    let Some(dir) = allocation_dir(root) else { return Ok(()) };
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().unwrap_or_default();
    let path = dir.join("allocation.json");
    let mut row: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    row["organism"] = serde_json::json!(id);
    row["root"] = serde_json::json!(root.display().to_string());
    if !row["roles"].is_object() {
        row["roles"] = serde_json::json!({});
    }
    // a record the habitat's memory keeps, and an organization the
    // habitat's nerves hold, name the habitat's server
    let service = ((role == "memory" || role == "nerves") && part_root(root, role) != root).then(|| allocation_service_of(&genome_dir(), role)).flatten();
    next_rows_as(&mut row, &id, role, version, slots, &local_habitat_project(), operator, service.as_deref());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_row(&path, &row)?;
    // into the habitat's record now, when it exists; else at its birth
    let _ = flush_pending();
    Ok(())
}

/// The cache `row` after the organism `id`'s `role` was migrated to
/// `version`: the rows the habitat's record is owed added to its pending
/// ones — `habitat.allocated` the first time, `habitat.migrated` whenever
/// the version moved — keyed by the organism and the role alone. A heart
/// role has no version of the habitat's: its schema is the heart's (a
/// declared migration runs the heart's own tool, `run_heart_migrations`),
/// so its allocation is recorded once and carries none.
///
/// The allocation names `service_as` (a tenant's record on the habitat's
/// memory names the habitat's server) when given.
#[allow(clippy::too_many_arguments)]
fn next_rows_as(row: &mut serde_json::Value, id: &str, role: &str, version: u64, slots: &str, project: &str, operator: bool, service_as: Option<&str>) {
    let heart = PROVISIONED_HEART_ROLES.contains(&role);
    let had = if heart { row["roles"][role].is_object().then_some(version) } else { row["roles"][role]["schema_version"].as_u64() };
    let recorded = row["roles"][role]["recorded"].as_bool().unwrap_or(false);
    let mut pending: Vec<serde_json::Value> = row["pending"].as_array().cloned().unwrap_or_default();
    // an allocation on the operator's own server is no service of the
    // habitat's project: it is named `operator`
    let service = if operator { "operator".to_string() } else { service_as.map(str::to_string).unwrap_or_else(|| format!("{role}-{}", &id[..12])) };
    let entity = format!("{}/{role}", &id[..12]);
    let slots: Vec<&str> = slots.split_whitespace().collect();
    if had.is_none() && !recorded {
        // every allocation names its provider and its host: the `compose`
        // provider's on the host `local`, as the `local` profile's policy
        // gives each role (`habitat.policy`), or the operator's own server,
        // adopted, on no host of the habitat's
        // (phase 4: or the host the allocation was placed on, as its
        // placement cache says, dna/core/placement.hl)
        let placed = placed_cache(&id[..12], role);
        let (provider, host, project) = if operator {
            ("operator".to_string(), String::new(), project.to_string())
        } else if let Some(p) = &placed {
            (p["provider"].as_str().unwrap_or("ssh").to_string(), p["host"].as_str().unwrap_or("").to_string(), p["project"].as_str().unwrap_or(project).to_string())
        } else {
            ("compose".to_string(), "local".to_string(), project.to_string())
        };
        pending.push(serde_json::json!({ "kind": "habitat.allocated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "service": service, "project": project, "provider": provider, "host": host, "vault": slots }).to_string() }));
    }
    if !heart && had != Some(version) {
        pending.push(serde_json::json!({ "kind": "habitat.migrated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "from": had, "schema_version": version }).to_string() }));
    }
    // a migration that names no slots keeps the ones the allocation holds
    let kept: Vec<serde_json::Value> = row["roles"][role]["vault"].as_array().cloned().unwrap_or_default();
    let vault = if slots.is_empty() { serde_json::json!(kept) } else { serde_json::json!(slots) };
    row["roles"][role] = if heart {
        serde_json::json!({ "service": service, "vault": vault, "recorded": recorded || had.is_some() })
    } else {
        serde_json::json!({ "service": service, "schema_version": version, "vault": vault, "recorded": recorded || had.is_some() })
    };
    row["pending"] = serde_json::json!(pending);
}

/// Where the allocation `<key>/<role>` was placed off this machine, as its
/// placement cache (`allocations/<key>/placed.<role>`, dna/core/placement.hl)
/// keeps it; `None` on this machine. A heart's database is placed with the
/// memory server, its broker with the nerves'.
pub(super) fn placed_cache(key: &str, role: &str) -> Option<serde_json::Value> {
    let role = match role {
        "database" => "memory",
        "broker" => "nerves",
        r => r,
    };
    let text = std::fs::read_to_string(PathBuf::from(local_habitat_dir()).join("allocations").join(key).join(format!("placed.{role}"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// Phase 4: the host a graft places the new organism on (`graft <sapling>
/// --host <name>`), taken by its birth as soon as its identity is pinned.
pub(super) static GRAFT_HOST: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();

/// How a graft fills one of the organism's body parts: `Reuse`, what the
/// habitat already offers, or `Own`, a part of the organism's own, on a
/// host (`None`: the organism-plane part on the graft's `--host`, an
/// application-plane part in the application's own compose).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Part {
    Reuse,
    Own(Option<String>),
}

/// The organism plane: what every organism has, and by default reuses
/// from the habitat (on its machine).
pub(super) const ORGANISM_PARTS: [&str; 3] = ["memory", "nerves", "senses"];
/// The application plane's infrastructure a heart may ask the habitat for:
/// by default the application's own (its compose), provisioned by the
/// habitat only when the graft says `--reuse`.
pub(super) const APPLICATION_PARTS: [&str; 2] = ["database", "broker"];

/// The parts a graft chose (`--own <part>[=<host>]`, `--reuse <part>`);
/// a part it names none for takes its plane's default (`part_choice`).
pub(super) static GRAFT_PARTS: std::sync::OnceLock<BTreeMap<String, Part>> = std::sync::OnceLock::new();
/// `graft --plan`: the plan printed, nothing grafted.
pub(super) static GRAFT_PLAN_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the application being grafted brings itself (`[dna] brings`).
pub(super) static GRAFT_BRINGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// How the graft fills `role`: its choice, else the default: the
/// organism plane reuses the habitat's, and an application-plane part is
/// the application's own when it brings it, else the habitat's.
pub(super) fn part_choice(role: &str) -> Part {
    GRAFT_PARTS.get().and_then(|m| m.get(role).cloned()).unwrap_or_else(|| default_part(role, GRAFT_BRINGS.get().map(Vec::as_slice).unwrap_or(&[])))
}

pub(super) fn default_part(role: &str, brings: &[String]) -> Part {
    if APPLICATION_PARTS.contains(&role) && brings.iter().any(|b| b == role) { Part::Own(None) } else { Part::Reuse }
}

/// `--own <part>[=<host>]` and `--reuse <part>`, read into a choice per part:
/// a part the graft does not take, or a dedicated server for an
/// application-plane part (not built), refused by name.
pub(super) fn parse_part(flag: &str, value: &str, into: &mut BTreeMap<String, Part>) -> Result<(), String> {
    let (part, host) = match value.split_once('=') {
        Some((p, h)) if !h.is_empty() => (p, Some(h.to_string())),
        Some((p, _)) => (p, None),
        None => (value, None),
    };
    if !ORGANISM_PARTS.contains(&part) && !APPLICATION_PARTS.contains(&part) {
        return Err(format!("`{flag} {value}`: a part is one of {}, {}", ORGANISM_PARTS.join(", "), APPLICATION_PARTS.join(", ")));
    }
    let choice = match flag {
        "--reuse" if host.is_some() => return Err(format!("`--reuse {value}`: a reused part runs where the habitat's does; `--own {part}=<host>` places one of its own")),
        "--reuse" => Part::Reuse,
        _ if APPLICATION_PARTS.contains(&part) && host.is_some() => {
            return Err(format!("`--own {value}`: a {part} server of the organism's own on a host of its own is not built yet; `--own {part}` (the application's own, in its compose) or `--reuse {part}`"));
        }
        _ => Part::Own(host),
    };
    if into.insert(part.to_string(), choice).is_some() {
        return Err(format!("`{part}` is chosen twice"));
    }
    Ok(())
}

/// The root whose memory server keeps the record at `root`: its own, or the
/// habitat's genome when its graft made it a tenant of the habitat's memory
/// (`parts.json`: memory = reuse), as `dna::memory_root` resolves it.
pub(crate) fn memory_root(root: &Path) -> PathBuf {
    part_root(root, "memory")
}

/// The root whose server holds `part` of the organism at `root` (memory,
/// or nerves: an account of the habitat's server), as `dna::part_root`.
fn part_root(root: &Path, part: &str) -> PathBuf {
    let Some(dir) = allocation_dir(root) else { return root.to_path_buf() };
    let reused = std::fs::read_to_string(dir.join("parts.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).is_some_and(|v| v[part] == "reuse");
    let genome = genome_dir();
    let mine = manifest_pin(&root.join("hale.toml")).ok().flatten();
    let habitat = manifest_pin(&genome.join("hale.toml")).ok().flatten();
    if !reused || habitat.is_none() || mine == habitat { root.to_path_buf() } else { genome }
}

/// `<role>-<key>`, the compose service of the organism at `root`'s role.
fn allocation_service_of(root: &Path, role: &str) -> Option<String> {
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten()?;
    (id.len() >= 12).then(|| format!("{role}-{}", &id[..12]))
}

/// The habitat's identity, as its face published it (`<habitat>/identity.env`,
/// dna/face/start.sh under `HALE_DNA_IDENTITY=publish`): the issuer every
/// organism it grafts signs in through. `None` before its face has run.
pub(super) fn shared_issuer() -> Option<String> {
    let text = std::fs::read_to_string(PathBuf::from(local_habitat_dir()).join("identity.env")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("HALE_DNA_OIDC_ISSUER=")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The name a person knows the habitat's machine by: its declared address
/// under `up --host`, else this machine.
fn habitat_machine() -> String {
    let at = habitat_address();
    if at == "127.0.0.1" || at == "localhost" || at.is_empty() { "this machine".to_string() } else { at }
}

/// The plan of what the graft makes, part by part, for an organism that
/// needs `needs` and runs compose services of its own (`has_app`): what each
/// part reuses or owns, and where.
pub(super) fn graft_plan(needs: &[String], has_app: bool) -> Vec<String> {
    let here = habitat_machine();
    let host = GRAFT_HOST.get().and_then(|h| h["host"].as_str()).map(str::to_string).unwrap_or_else(|| here.clone());
    let brings = GRAFT_BRINGS.get().cloned().unwrap_or_default();
    plan_lines(needs, has_app, &here, &host, &brings, shared_issuer().as_deref(), &part_choice)
}

/// One line of a graft's plan: the part, the choice, what it means, and the
/// flag that flips it (columns by width, never by spaces in the text).
fn plan_row(part: &str, choice: &str, what: &str, flip: &str) -> String {
    let flip = if flip.is_empty() { String::new() } else { format!("  ({flip})") };
    format!("  {part:<9} {choice:<6} {what}{flip}")
}

fn plan_lines(needs: &[String], has_app: bool, here: &str, host: &str, brings: &[String], shared: Option<&str>, part_choice: &dyn Fn(&str) -> Part) -> Vec<String> {
    let (here, host) = (here.to_string(), host.to_string());
    let mut out = vec!["organism plane".to_string()];
    for role in ORGANISM_PARTS {
        if role == "senses" && !needs.iter().any(|r| r == "senses") {
            continue;
        }
        out.push(match part_choice(role) {
            Part::Reuse if role == "memory" => plan_row(role, "reuse", &format!("a schema of its own on the habitat's Postgres ({here})"), "--own memory=<host>: a server of its own"),
            Part::Reuse if role == "nerves" => plan_row(role, "reuse", &format!("an account of its own on the habitat's NATS ({here})"), "--own nerves=<host>: a server of its own"),
            Part::Reuse => plan_row(role, "reuse", &format!("a server of its own on the habitat's machine ({here}); sharing the habitat's is not built"), &format!("--own {role}=<host>: elsewhere")),
            Part::Own(h) => plan_row(role, "own", &format!("a server of its own on {}", h.unwrap_or_else(|| host.clone())), ""),
        });
    }
    out.push(match shared {
        Some(issuer) => plan_row("identity", "reuse", &format!("the habitat's sign-in ({issuer}): one account, every organism"), ""),
        None => plan_row("identity", "own", "its own sign-in stub, beside its head (the habitat's face has published none)", ""),
    });
    out.push(plan_row("forge", "reuse", "the habitat's forge, by its policy (--forge to choose)", ""));
    out.push("application plane".to_string());
    out.push(if has_app { format!("  {:<9} its compose services on {host}", "services") } else { format!("  {:<9} none (no compose file at its root)", "services") });
    for role in APPLICATION_PARTS {
        if !needs.iter().any(|r| r == role) {
            continue;
        }
        let server = if role == "database" { "Postgres" } else { "NATS" };
        out.push(match part_choice(role) {
            Part::Own(_) if !brings.iter().any(|b| b == role) => plan_row(role, "own", &format!("the application's own {server}: it does not say it brings one (`[dna] brings`), so its compose must provide it"), ""),
            Part::Own(_) => plan_row(role, "own", &format!("the application's own {server}, in its compose"), &format!("--reuse {role}: one the habitat provisions and hands over")),
            Part::Reuse => plan_row(role, "reuse", &format!("provisioned by the habitat on the organism's {} server, handed over as HALE_DNA_{}_*", if role == "database" { "memory" } else { "nerves" }, role.to_uppercase()), ""),
        });
    }
    out
}

/// The organism keyed `key` placed as the graft chose, before anything of
/// it is brought up: each organism-plane part the graft owns on a host is
/// kept as placed there, so its birth brings it up there and derives its
/// address from there; a reused part stays on the habitat's machine. The
/// choice for each application-plane part is kept beside the allocations
/// (`parts.json`), which every later act that provisions reads
/// (`app_owns`). Nothing when the graft placed nothing and chose nothing.
pub(super) fn place_for_graft(key: &str) -> Result<Option<String>, String> {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(key);
    // the allocation's directory holds its passwords' files: yours alone,
    // as the secrets' provisioning makes it, whichever makes it first
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        if !dir.is_dir() {
            std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut placed = Vec::new();
    for role in ORGANISM_PARTS {
        let h = match part_choice(role) {
            Part::Reuse => continue,
            Part::Own(Some(name)) => match sapling::graft_host(&name)? {
                Some(h) => h,
                None => continue,
            },
            Part::Own(None) => match GRAFT_HOST.get() {
                Some(h) => h.clone(),
                None => continue,
            },
        };
        let path = dir.join(format!("placed.{role}"));
        std::fs::write(&path, format!("{h}\n")).map_err(|e| format!("write {}: {e}", path.display()))?;
        placed.push(format!("{role} on {}", h["host"].as_str().unwrap_or("")));
    }
    let parts: serde_json::Map<String, serde_json::Value> = ORGANISM_PARTS.iter().chain(APPLICATION_PARTS.iter()).map(|r| (r.to_string(), serde_json::json!(if part_choice(r) == Part::Reuse { "reuse" } else { "own" }))).collect();
    write_row(&dir.join("parts.json"), &serde_json::Value::Object(parts))?;
    Ok((!placed.is_empty()).then(|| format!("{:<8}its {}", "placed", placed.join(", "))))
}

/// Whether the application at `root` owns its `role` (an application-plane
/// part: its compose provides it), as its graft chose: the habitat then
/// provisions none. An organism grafted before choices were kept reuses.
pub(super) fn app_owns(root: &Path, role: &str) -> bool {
    let kept = allocation_dir(root).and_then(|dir| std::fs::read_to_string(dir.join("parts.json")).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    match kept {
        Some(v) => v[role] == "own",
        // grafted before choices were kept: what it brings
        None => brings_of(std::fs::read_to_string(root.join("hale.toml")).ok().as_deref()).unwrap_or_default().iter().any(|b| b == role),
    }
}

/// The roles the habitat provisions for the organism at `root`: what it
/// needs, less the application-plane parts its application owns.
pub(super) fn provisioned_needs(root: &Path) -> Vec<String> {
    needs_at(root).into_iter().filter(|r| !app_owns(root, r)).collect()
}
/// The forge provider a graft names for its organism (`--forge`), over the
/// habitat's policy.
pub(super) static GRAFT_FORGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// One run of a heart's declared migration, a `habitat.migrated
/// <id>/<role>` row owed to the habitat's record: the command, its exit
/// status and its output (its last bytes, bounded), whatever it did.
fn record_migration_run(root: &Path, role: &str, command: &[String], status: &str, output: &str) -> Result<(), String> {
    let Some(dir) = allocation_dir(root) else { return Ok(()) };
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().unwrap_or_default();
    if id.len() < 12 {
        return Ok(());
    }
    let path = dir.join("allocation.json");
    let mut row: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    let mut pending: Vec<serde_json::Value> = row["pending"].as_array().cloned().unwrap_or_default();
    const KEPT: usize = 4096;
    let tail = if output.len() > KEPT { let mut at = output.len() - KEPT; while !output.is_char_boundary(at) { at += 1; } &output[at..] } else { output };
    pending.push(serde_json::json!({ "kind": "habitat.migrated", "entity": format!("{}/{role}", &id[..12]), "body": serde_json::json!({ "organism": id, "role": role, "command": command, "status": status, "output": tail }).to_string() }));
    row["pending"] = serde_json::json!(pending);
    if row["roles"][role].is_object() {
        row["roles"][role]["migration"] = serde_json::json!(status);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_row(&path, &row)?;
    let _ = flush_pending();
    Ok(())
}

fn write_row(path: &Path, row: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(row).map_err(|e| e.to_string())? + "\n";
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// The allocation rows every cache holds that the habitat's record does
/// not yet, written into it: what the habitat's birth does for the
/// allocations made before it (its own, at bootstrap), and every later
/// migration for its own. Nothing while the habitat is not born.
fn flush_pending() -> Result<usize, String> {
    let genome = genome_dir();
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_none() || !genome.join(".hale/dna/memory.env").is_file() || !super::record_exists(&genome).unwrap_or(false) {
        return Ok(0);
    }
    let mut lines = String::new();
    let mut touched = Vec::new();
    for (key, row) in allocations() {
        let Some(p) = row["pending"].as_array() else { continue };
        if p.is_empty() {
            continue;
        }
        for r in p {
            lines.push_str(&r.to_string());
            lines.push('\n');
        }
        touched.push(key);
    }
    if lines.is_empty() {
        return Ok(0);
    }
    let file = PathBuf::from(local_habitat_dir()).join(".allocation-rows.jsonl");
    std::fs::write(&file, &lines).map_err(|e| format!("write {}: {e}", file.display()))?;
    let landed = host_run("record-seed", &genome, &[file.display().to_string(), String::new()]);
    let _ = std::fs::remove_file(&file);
    landed?;
    let base = PathBuf::from(local_habitat_dir()).join("allocations");
    for key in &touched {
        let path = base.join(key).join("allocation.json");
        let Some(mut row) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        row["pending"] = serde_json::json!([]);
        if let Some(roles) = row["roles"].as_object_mut() {
            for (_, r) in roles.iter_mut() {
                r["recorded"] = serde_json::json!(true);
            }
        }
        write_row(&path, &row)?;
    }
    Ok(lines.lines().count())
}

/// The allocations as the habitat's record states them: per `<id>/<role>`,
/// the organism, the service and the latest schema version, with the
/// checkout this machine last migrated it from beside it from the local
/// cache (never the record's);
/// `None` when the record cannot be read (the habitat not born, or its
/// memory not up), when the cache is all there is.
fn recorded_allocations() -> Option<Vec<(String, serde_json::Value)>> {
    let genome = genome_dir();
    manifest_pin(&genome.join("hale.toml")).ok().flatten()?;
    let rows = host_run("record-rows", &genome, &[]).ok()?;
    let mut by: std::collections::BTreeMap<String, serde_json::Value> = std::collections::BTreeMap::new();
    for line in rows.lines() {
        let Ok(r) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let kind = r["kind"].as_str().unwrap_or("");
        if kind != "habitat.allocated" && kind != "habitat.migrated" {
            continue;
        }
        let Some((key, role)) = r["entity"].as_str().and_then(|e| e.split_once('/')) else { continue };
        let body: serde_json::Value = r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_else(|| r["body"].clone());
        // an organism removed: its allocations are the record's history alone
        if kind == "habitat.allocated" && body["state"] == "removed" {
            if let Some(row) = by.get_mut(key) {
                if let Some(roles) = row["roles"].as_object_mut() {
                    roles.remove(role);
                }
                if row["roles"].as_object().is_some_and(|r| r.is_empty()) {
                    by.remove(key);
                }
            }
            continue;
        }
        let row = by.entry(key.to_string()).or_insert_with(|| serde_json::json!({ "roles": {} }));
        if kind == "habitat.allocated" {
            row["organism"] = body["organism"].clone();
            row["roles"][role]["service"] = body["service"].clone();
            row["roles"][role]["vault"] = body["vault"].clone();
        } else if body["status"].is_string() {
            // a heart's migration run: its status, never a version
            row["roles"][role]["migration"] = body["status"].clone();
        } else {
            row["roles"][role]["schema_version"] = body["schema_version"].clone();
        }
    }
    let cache: std::collections::BTreeMap<String, serde_json::Value> = allocations().into_iter().collect();
    for (key, row) in by.iter_mut() {
        if let Some(root) = cache.get(key).and_then(|c| c["root"].as_str()) {
            row["root"] = serde_json::json!(root);
        }
    }
    Some(by.into_iter().collect())
}

/// Every allocation row in the habitat, by its directory's key.
fn allocations() -> Vec<(String, serde_json::Value)> {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations");
    let mut out: Vec<(String, serde_json::Value)> = std::fs::read_dir(&dir)
        .map(|es| {
            es.flatten()
                .filter_map(|e| {
                    let key = e.file_name().to_string_lossy().into_owned();
                    let row = std::fs::read_to_string(e.path().join("allocation.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null);
                    Some((key, row))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The versions this toolchain migrates each role to (`memory`, `nerves`),
/// asked of the host at `root`.
fn toolchain_versions(root: &Path) -> Vec<(String, u64)> {
    host_run("schema-versions", root, &[])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('=').and_then(|(k, v)| v.trim().parse().ok().map(|v| (k.to_string(), v))))
        .collect()
}

/// The roles of `row` recorded behind `versions`, as `role a -> b`.
fn behind(row: &serde_json::Value, versions: &[(String, u64)]) -> Vec<String> {
    versions
        .iter()
        .filter_map(|(role, want)| {
            let had = row["roles"][role.as_str()]["schema_version"].as_u64()?;
            (had < *want).then(|| format!("{role} {had} -> {want}"))
        })
        .collect()
}

/// GH #1653: the habitat's upgrade of the organism at `root`: each of its
/// allocation's roles recorded behind this toolchain's version is migrated
/// (an operator's own servers move with `upgrade`'s owner's DSN instead).
/// What was done, a line each; nothing for an organism with no allocation.
pub(super) fn upgrade_allocation(root: &Path) -> Result<Vec<String>, String> {
    let Some(dir) = allocation_dir(root) else { return Ok(Vec::new()) };
    let Some(row) = std::fs::read_to_string(dir.join("allocation.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return Ok(Vec::new()) };
    let moved = behind(&row, &toolchain_versions(root));
    // a heart's declared migrations run at every upgrade: its tool keeps
    // its own ledger and decides whether anything moved
    if moved.is_empty() {
        return run_heart_migrations(root);
    }
    let mut out = vec![format!("habitat the allocation records versions behind this toolchain's ({}): migrating it", moved.join(", "))];
    out.extend(migrate_lines(root)?);
    Ok(out)
}

/// GH #1653: a heart's own migration for a heart role it declares,
/// `[dna.migrations.<role>] command = [...]` in its genome's manifest.
/// Declaring one is the heart's choice: the habitat then runs it, with that
/// role's handover in its environment, at graft, by `hale habitat migrate
/// <organism>` and by `hale dna upgrade`, every time (the heart's tool is
/// idempotent and keeps its own ledger; the habitat keeps no version of
/// it). A heart that declares none owns its schema however it likes.
pub(super) struct HeartMigration {
    pub(super) role: String,
    command: Vec<String>,
}

/// The heart migrations the genome at `root` declares, each for a heart
/// role it needs; refused by name when one is malformed, names a key
/// other than `command`, or is for a role the genome does not declare.
pub(super) fn heart_migrations(root: &Path) -> Result<Vec<HeartMigration>, String> {
    let Ok(text) = std::fs::read_to_string(root.join("hale.toml")) else { return Ok(Vec::new()) };
    let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
    let Some(m) = table.get("dna").and_then(|d| d.get("migrations")) else { return Ok(Vec::new()) };
    let m = m.as_table().ok_or("[dna.migrations] is not a table of heart roles")?;
    let needs = needs_at(root);
    let mut out = Vec::new();
    for (role, spec) in m {
        if !PROVISIONED_HEART_ROLES.contains(&role.as_str()) || !needs.iter().any(|r| r == role) {
            return Err(format!("[dna.migrations.{role}]: a heart migration is for a heart role the genome needs ({}), and `{role}` is not one", PROVISIONED_HEART_ROLES.join(", ")));
        }
        let spec = spec.as_table().ok_or_else(|| format!("[dna.migrations.{role}] is not a table"))?;
        if let Some(other) = spec.keys().find(|k| *k != "command") {
            return Err(format!("[dna.migrations.{role}]: `{other}` is not a key of a heart migration; it is its `command` alone, run at every graft, `hale habitat migrate` and `hale dna upgrade` (the tool keeps its own ledger)"));
        }
        let command: Vec<String> = spec.get("command").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if command.is_empty() {
            return Err(format!("[dna.migrations.{role}] needs `command`, the argv that migrates it"));
        }
        // a role its application owns is migrated by the application (its
        // compose), never by the habitat
        if !app_owns(root, role) {
            out.push(HeartMigration { role: role.clone(), command });
        }
    }
    Ok(out)
}

/// Each declared heart migration run, with the role's handover in its
/// environment, in the organism's directory; every run is a
/// `habitat.migrated` row with the command's exit status and output, and
/// one that fails fails the act (the next act runs it again). What was
/// done, a line each.
pub(super) fn run_heart_migrations(root: &Path) -> Result<Vec<String>, String> {
    let migrations = heart_migrations(root)?;
    if migrations.is_empty() {
        return Ok(Vec::new());
    }
    // a migration is a habitat act: the role's allocation is brought up
    // (and waited for) as `hale habitat migrate` brings it up, never
    // assumed up; an upgrade that just restarted the nerves for new
    // passwords waits for them here
    let handed = handover(root, false)?;
    let mut out = Vec::new();
    for m in migrations {
        let mut cmd = Command::new(&m.command[0]);
        cmd.args(&m.command[1..]).current_dir(root);
        for (k, v) in &handed {
            cmd.env(k, v);
        }
        let shown = m.command.join(" ");
        let ran = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                record_migration_run(root, &m.role, &m.command, "not run", &e.to_string())?;
                return Err(format!("the heart's {} migration (`{shown}`) could not run: {e}", m.role));
            }
        };
        let output = format!("{}{}", String::from_utf8_lossy(&ran.stdout), String::from_utf8_lossy(&ran.stderr));
        let status = ran.status.code().map(|c| format!("exit {c}")).unwrap_or_else(|| ran.status.to_string());
        record_migration_run(root, &m.role, &m.command, &status, &output)?;
        if !ran.status.success() {
            return Err(format!("the heart's {} migration (`{shown}`) failed ({status}): {}", m.role, String::from_utf8_lossy(&ran.stderr).trim()));
        }
        out.push(format!("{:<8}the heart's {} migration ran (`{shown}`, {status})", "heart", m.role));
    }
    Ok(out)
}

// ---------------------------------------------------------------
// up, status, down (GH #1653)
// ---------------------------------------------------------------

fn genome_dir() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("genome")
}

/// How long `up` waits for a face that is still building the head's
/// programs before it answers.
const FACE_BUILD_WAIT: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// The seed a face's log says it is building now (`hale: building …: <seed> @ …`
/// as its last such line, with nothing after it but more of the same), or
/// `None` when its last line is anything else.
fn building(tail: &str) -> Option<String> {
    let last = tail.lines().map(str::trim).filter(|l| !l.is_empty()).last()?;
    let rest = last.strip_prefix("hale: building ")?;
    let seed = rest.split_once(": ").map(|(_, s)| s).unwrap_or(rest);
    Some(seed.split(" @ ").next().unwrap_or(seed).trim().to_string())
}

/// The last `n` lines `service` of the habitat's compose project wrote
/// (its output and its errors), "" when docker answers none.
fn service_log_tail(service: &str, n: usize) -> String {
    let project = local_habitat_project();
    let Docker::Answered(ids) = docker(&["ps", "-aq", "--filter", &format!("label=com.docker.compose.project={project}"), "--filter", &format!("label=com.docker.compose.service={service}")]) else {
        return String::new();
    };
    let Some(id) = ids.lines().map(str::trim).find(|l| !l.is_empty()) else { return String::new() };
    match Command::new("docker").args(["logs", "--tail", &n.to_string(), id]).output() {
        Ok(o) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim_end().to_string(),
        Err(_) => String::new(),
    }
}

/// The state of `service` in the habitat's compose project when running.
fn service_status(service: &str) -> Option<String> {
    let project = local_habitat_project();
    let Docker::Answered(out) = docker(&[
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--filter",
        &format!("label=com.docker.compose.service={service}"),
        "--filter",
        "status=running",
        "--format",
        "{{.Status}}",
    ]) else {
        return None;
    };
    let status = out.lines().next().unwrap_or("").trim();
    (!status.is_empty()).then(|| status.to_string())
}

/// The habitat's body as it runs in the habitat's compose project, or `None`.
fn body_state() -> Option<String> {
    service_status("body").map(|s| format!("up (compose service body, {s})"))
}

fn backends_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("backends")
}

/// The name the habitat was born with, kept beside it as its backends are.
fn name_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("name")
}

/// The name a habitat has when its birth named none.
const DEFAULT_HABITAT_NAME: &str = "local";

/// The habitat's name as the labels on its containers carry it: the one it
/// was born with here, else the one this machine's pointer names for a
/// habitat on another machine (as `dna::habitat_name_here`).
pub(super) fn habitat_label() -> String {
    std::fs::read_to_string(name_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| remote::pointer().and_then(|p| p["name"].as_str().map(str::to_string)).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| DEFAULT_HABITAT_NAME.to_string())
}

/// A compose service's `labels:` block for a part `role` of the organism
/// `organism`, as `dna::compose_labels` writes one: the habitat, the
/// organism and the part, which the container exporter carries onto every
/// reading.
pub(super) fn compose_labels(organism: &str, role: &str) -> String {
    let q = |v: &str| serde_json::to_string(v).unwrap_or_default();
    format!("    labels:\n      hale.habitat: {}\n      hale.organism: {}\n      hale.role: {}\n", q(&habitat_label()), q(organism), q(role))
}

/// Whether `name` is a DNS label: 1 to 63 lowercase letters, digits and
/// `-`, neither first nor last a `-`.
fn habitat_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !name.starts_with('-') && !name.ends_with('-')
}

/// The habitat's name for this run of `up`: the one it was born with (kept
/// beside it, and on its `habitat.profile` row), or for a habitat not yet
/// born the one asked (`local` by default). A habitat keeps its name: one
/// asked of a habitat born with another is refused.
fn name_of(asked: Option<&str>) -> Result<String, String> {
    if let Some(a) = asked {
        if !habitat_name_ok(a) {
            return Err(format!("`--name {a}`: a habitat's name is a DNS label: 1 to 63 lowercase letters, digits and `-`, neither first nor last a `-`"));
        }
    }
    let born_any = genome_dir().join("hale.toml").is_file() || backends_file().exists();
    let kept = std::fs::read_to_string(name_file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let born = kept.or_else(|| born_any.then(|| DEFAULT_HABITAT_NAME.to_string()));
    match (born, asked) {
        (Some(b), Some(a)) if b != a => Err(format!("the habitat is named `{b}`: a habitat keeps the name it was born with")),
        (Some(b), _) => Ok(b),
        (None, Some(a)) => Ok(a.to_string()),
        (None, None) => Ok(DEFAULT_HABITAT_NAME.to_string()),
    }
}

/// The backends this run of `up` uses: the ones the habitat was born on
/// (kept beside it, and as its `habitat.profile` row), or for a habitat not
/// yet born the ones asked (`own` by default); refused when the asking or
/// the environment disagrees with them.
fn backends_of(asked: Option<&str>) -> Result<String, String> {
    if let Some(a) = asked {
        if a != "own" && a != "operator" {
            return Err(format!("`--backends {a}`: the backends are `own` (the habitat's allocations under docker compose) or `operator` (the Postgres and NATS HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER name)"));
        }
    }
    let born = std::fs::read_to_string(backends_file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let backends = match (&born, asked) {
        (Some(b), Some(a)) if b != a => return Err(format!("the habitat was born on `{b}` backends; they are chosen at its birth, not at `up`")),
        (Some(b), _) => b.clone(),
        (None, Some(a)) => a.to_string(),
        (None, None) => "own".to_string(),
    };
    let named = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    if backends == "own" && own_servers() {
        return Err("the habitat runs on its own allocations; unset HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER, or birth it with `--backends operator` to adopt them".to_string());
    }
    if backends == "operator" && !(named("HALE_DNA_MEMORY_DSN_OWNER") && named("HALE_DNA_NATS_URL_OWNER")) {
        return Err("the habitat's backends are the operator's: name them in HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER".to_string());
    }
    Ok(backends)
}

fn up(args: &[String]) -> ExitCode {
    let mut no_body = false;
    let mut profile = "local".to_string();
    let mut asked: Option<String> = None;
    let mut asked_name: Option<String> = None;
    let mut asked_image: Option<String> = None;
    let mut on_host: Option<String> = None;
    let mut ssh_user: Option<String> = None;
    let mut ssh_port: Option<u16> = None;
    // what `up` on the habitat's machine is handed of these flags
    let mut passed: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-body" => {
                no_body = true;
                passed.push("--no-body".to_string());
            }
            "--host" if i + 1 < args.len() => {
                on_host = Some(args[i + 1].clone());
                i += 1;
            }
            "--user" if i + 1 < args.len() => {
                ssh_user = Some(args[i + 1].clone());
                i += 1;
            }
            "--port" if i + 1 < args.len() => {
                match args[i + 1].parse::<u16>() {
                    Ok(p) if p > 0 => ssh_port = Some(p),
                    _ => {
                        eprintln!("hale habitat up: `--port {}` is not a port", args[i + 1]);
                        return ExitCode::from(2);
                    }
                }
                i += 1;
            }
            "--backends" if i + 1 < args.len() => {
                asked = Some(args[i + 1].clone());
                passed.push("--backends".to_string());
                passed.push(args[i + 1].clone());
                i += 1;
            }
            "--profile" if i + 1 < args.len() => {
                profile = args[i + 1].clone();
                i += 1;
            }
            "--name" if i + 1 < args.len() => {
                asked_name = Some(args[i + 1].clone());
                i += 1;
            }
            "--image" if i + 1 < args.len() => {
                asked_image = Some(args[i + 1].clone());
                i += 1;
            }
            other => {
                eprintln!("hale habitat up: `{other}` is not a flag of `up`\n{}", usage_text());
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    if on_host.is_none() && (ssh_user.is_some() || ssh_port.is_some()) {
        eprintln!("hale habitat up: `--user` and `--port` are the SSH login of `--host <name>`\n{}", usage_text());
        return ExitCode::from(2);
    }
    if let Some(host) = on_host {
        if no_body || profile != "local" {
            eprintln!("hale habitat up --host: a habitat on another machine is the `local` profile with its body and face (no `--no-body`)");
            return ExitCode::from(2);
        }
        let kept = remote::pointer();
        let name = match asked_name.clone().or_else(|| kept.as_ref().and_then(|p| p["name"].as_str().map(str::to_string))) {
            Some(n) if habitat_name_ok(&n) => n,
            Some(n) => {
                eprintln!("hale habitat up: `{n}` is not a habitat name (a DNS label: lowercase letters, digits and `-`, at most 63)");
                return ExitCode::from(2);
            }
            None => DEFAULT_HABITAT_NAME.to_string(),
        };
        return remote::up(&host, ssh_user.as_deref(), ssh_port, &name, asked_image.as_deref(), &passed);
    }
    if let Some(p) = remote::pointer() {
        return remote::refuse_local_up(&p);
    }
    if profile != "local" {
        eprintln!("hale habitat up: the `{profile}` profile is not built: only `local` (docker compose on this machine) is; the hosted one runs on the backends a platform already runs (GH #1653)");
        return ExitCode::from(2);
    }
    // GH #1653: the habitat's backends are chosen at its birth and kept:
    // `own`, allocations of its own project under docker compose, or
    // `operator`, the Postgres and the NATS an operator names
    // (HALE_DNA_MEMORY_DSN_OWNER, HALE_DNA_NATS_URL_OWNER), adopted rather
    // than provisioned. Either way the environment must agree with it: a
    // server named for an `own` habitat would migrate one place and run
    // another, and an `operator` habitat with none named has no backends
    let backends = match backends_of(asked.as_deref()) {
        Ok(b) => b,
        Err(why) => {
            eprintln!("hale habitat up: {why}");
            return ExitCode::from(2);
        }
    };
    let name = match name_of(asked_name.as_deref()) {
        Ok(n) => n,
        Err(why) => {
            eprintln!("hale habitat up: {why}");
            return ExitCode::from(2);
        }
    };
    match bring_up(&backends, &name, !no_body) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
    }
    let genome = genome_dir();
    // GH #1653: born and its allocations up, its body not started: what a
    // test's habitat needs to graft into
    if no_body {
        if let Err(e) = place(&genome, &backends) {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
        println!("{:<8}not started (--no-body): `hale habitat up` starts it", "body");
        return ExitCode::SUCCESS;
    }
    match start_body_and_face(&genome, &backends, &name, asked_image.as_deref()) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
    }
    if let Err(e) = place(&genome, &backends) {
        eprintln!("hale habitat up: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

pub(super) fn progress(msg: &str) {
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) || PROGRESS_TO_LOG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("... {msg}");
    }
}

/// A graft the head started (`--no-head`) writes its progress into its log,
/// which `Commands::graft_log` pages to the person following it.
static PROGRESS_TO_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The habitat's placement, through its providers (phase 4): the host
/// the `compose` provider produces (`local`, with what it offers), the
/// `local` profile's policy (a provider per role, on these backends) and
/// the habitat's own head and face placed on that host, each a row of its
/// record written when the record does not already say it (`habitat-place`,
/// dna/host/placement.hl). What it recorded, a line each, printed.
fn place(genome: &Path, backends: &str) -> Result<(), String> {
    progress("recording placement and refreshing nurseries");
    let said = host_run("habitat-place", genome, &[backends.to_string()]).map_err(|e| format!("the habitat's placement was not recorded: {e}"))?;
    print!("{said}");
    // what its nurseries offer, read into its record (a newer revision of a
    // pinned sapling filed for the Board, never taken)
    match sapling::refresh() {
        Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
        Err(e) => println!("{:<8}the nurseries were not read: {e}", "note"),
    }
    Ok(())
}

/// The habitat born the first time, its own memory and nerves migrated
/// (its schema is the habitat's to move, at bootstrap and at a toolchain's
/// upgrade), and every tenant's recorded allocation brought up. A tenant is
/// never migrated here: `up` runs under ongoing work, and an organism's
/// allocation moves at its upgrade or with `hale habitat migrate`.
fn bring_up(backends: &str, name: &str, with_body: bool) -> Result<Vec<String>, String> {
    let genome = genome_dir();
    let mut out = vec![format!("habitat local at {} (compose project {}; {backends} backends)", local_habitat_dir(), local_habitat_project())];
    // born whole once its identity is pinned and no birth is pending: a
    // birth that stopped part way left its steps in `birth.pending`, and is
    // resumed from them (memory that is merely down is never mistaken for
    // a birth that did not happen)
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_some() && !birth_pending().exists() {
        progress("migrating the habitat's memory and nerves");
        out.extend(migrate_lines(&genome)?);
    } else {
        std::fs::create_dir_all(local_habitat_dir()).map_err(|e| format!("create {}: {e}", local_habitat_dir()))?;
        if !backends_file().exists() {
            std::fs::write(backends_file(), format!("{backends}\n")).map_err(|e| format!("write {}: {e}", backends_file().display()))?;
        }
        if !name_file().exists() {
            std::fs::write(name_file(), format!("{name}\n")).map_err(|e| format!("write {}: {e}", name_file().display()))?;
        }
        out.extend(birth(&genome, backends, name, with_body)?);
        // its senses' store, as a migration brings it up: what the reflexes
        // read and the habitat's Grafana charts
        if !own_servers() {
            out.push(senses_line(&genome));
        }
    }
    let own = allocation_dir(&genome);
    let versions = toolchain_versions(&genome);
    let rows = recorded_allocations().unwrap_or_else(allocations);
    for (key, row) in rows {
        let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(&key);
        if Some(&dir) == own.as_ref() {
            continue;
        }
        if !dir.join("compose.yaml").is_file() || !dir.join("ports.yaml").is_file() {
            continue;
        }
        // the tenant's allocations are the compose provider's to apply
        // (`habitat-apply`, dna/host/placement.hl)
        if let Err(why) = host_run("habitat-apply", &genome, &[dir.display().to_string()]) {
            out.push(format!("{:<8}allocation {key}: did not come up: {why}", "note"));
            continue;
        }
        let moved = behind(&row, &versions);
        let root = row["root"].as_str().unwrap_or("<organism>");
        out.push(format!("{:<8}allocation {key} ({root}){}", "up", if moved.is_empty() { String::new() } else { format!("; behind this toolchain ({}): `hale habitat migrate {root}` or `hale dna upgrade` migrates it", moved.join(", ")) }));
    }
    Ok(out)
}

/// The steps of a birth not yet whole, one a line as each is done
/// (`genesis`, `profile`, `habitat`, `standard`): written when a birth
/// begins and removed when it is whole.
fn birth_pending() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("birth.pending")
}

fn birth_done(step: &str) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(birth_pending()).map_err(|e| format!("write {}: {e}", birth_pending().display()))?;
    writeln!(f, "{step}").map_err(|e| format!("write {}: {e}", birth_pending().display()))
}

/// GH #1653: the habitat's birth, the one with no parent: its genome
/// written from the toolchain's, made a repository, born as an organism
/// (its record in its own memory allocation) and given its positions and
/// its law as the Board's to ratify. Each step is taken only when it is
/// not done already, so a birth that stopped part way (memory or docker
/// failing after the identity was pinned) is resumed by the next `up`: a
/// pin whose record never reached memory is a birth that did not happen,
/// and is drawn again.
fn birth(genome: &Path, backends: &str, name: &str, with_body: bool) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(genome).map_err(|e| format!("create {}: {e}", genome.display()))?;
    for f in hale_dna::HABITAT_FILES {
        let rel = f.path.strip_prefix("dna/habitat/").unwrap_or(f.path);
        let path = genome.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        if !path.exists() {
            std::fs::write(&path, f.content).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
    }
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(genome).args(["-c", "user.name=habitat", "-c", "user.email=habitat@localhost"]).args(args).output().map_err(|e| e.to_string());
    if !genome.join(".git").exists() {
        git(&["init", "-q", "-b", "main"])?;
    }
    git(&["add", "-A"])?;
    let _ = git(&["commit", "-q", "-m", "the habitat's genome"])?;
    let mut out = Vec::new();
    if !birth_pending().exists() {
        std::fs::write(birth_pending(), "").map_err(|e| format!("write {}: {e}", birth_pending().display()))?;
    }
    let done = std::fs::read_to_string(birth_pending()).unwrap_or_default();
    let have: Vec<&str> = done.lines().collect();
    // whether the record was born is memory's to say, never a local
    // marker's: a genesis memory holds is resumed, never drawn again
    let manifest = genome.join("hale.toml");
    progress("birthing the habitat's genome, memory, and nerves");
    match genesis_state(genome)? {
        Genesis::Unpinned => {
            out.push(format!("{:<8}the habitat at {} (the one organism with no parent)", "born", genome.display()));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
        Genesis::Present => {
            // born: the rest of its init, each step of which keeps what
            // is done, against the record memory holds
            out.push(format!("{:<8}the habitat's record is in memory; its birth goes on where it stopped", "resume"));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
        Genesis::Never(drawn) => {
            // memory answered that this identity's record was never born
            // (or its memory was never provisioned): the identity is drawn
            // again, and what was made for it, holding no record, goes
            forget_allocation(&drawn[..12.min(drawn.len())]);
            unpin(&manifest)?;
            out.push(format!("{:<8}the habitat's earlier birth stopped before its record reached memory: born again", "resume"));
            out.push(format!("{:<8}the habitat at {} (the one organism with no parent)", "born", genome.display()));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
    }
    // the bootstrap's allocations, the habitat's own memory and nerves,
    // made before its record existed: their rows, now it does
    let n = flush_pending()?;
    if n > 0 {
        out.push(format!("recorded {n} allocation row(s) in the habitat's record"));
    }
    if !have.contains(&"profile") {
        // its profile: the backends it was born on, its name, and when its
        // body and face come up with it, its face's and head's addresses
        let mut body = serde_json::json!({ "profile": "local", "backends": backends, "project": local_habitat_project(), "name": name });
        if with_body {
            let base = head_block(genome, &granted_bases(genome))?;
            let face_url = format!("http://{}:{base}", habitat_address());
            let head_url = format!("http://{}:{}", habitat_address(), base + 1);
            std::fs::write(head_url_file(), format!("{head_url}\n")).map_err(|e| format!("write {}: {e}", head_url_file().display()))?;
            body["face"] = serde_json::json!(face_url);
            body["head"] = serde_json::json!(head_url);
        }
        let profile = PathBuf::from(local_habitat_dir()).join(".profile-row.jsonl");
        let row = serde_json::json!({ "kind": "habitat.profile", "entity": "local", "body": body.to_string() });
        std::fs::write(&profile, format!("{row}\n")).map_err(|e| format!("write {}: {e}", profile.display()))?;
        let landed = host_run("record-seed", genome, &[profile.display().to_string(), String::new()]);
        let _ = std::fs::remove_file(&profile);
        landed?;
        birth_done("profile")?;
    }
    // each pack is proposed by its own `hale dna pack apply`, a process of
    // its own: the verb hands itself over to the host (exec), and the
    // habitat's birth goes on after it
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut proposed = 0;
    // a pack whose batch the record holds was proposed (and, at birth,
    // ratified): memory decides, never the local marker, which a stopped
    // birth may have lost
    let recorded_rows = rows_of(genome);
    for (pack, args) in [("habitat", vec!["apply", "habitat", "--on-holes"]), ("standard", vec!["apply", "standard"])] {
        let recorded = recorded_rows.iter().any(|r| r["kind"] == "batch.proposed" && r.to_string().contains(&format!("{pack}@")));
        if have.contains(&pack) || recorded {
            continue;
        }
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push(genome.display().to_string());
        let ok = Command::new(&me).arg("dna").arg("pack").args(&a).status().is_ok_and(|s| s.success());
        if ok {
            proposed += 1;
            birth_done(pack)?;
        } else {
            return Err(format!("the `{pack}` pack was not proposed to the habitat's Board (`hale habitat up` tries again)"));
        }
    }
    if proposed > 0 {
        out.push(ratify_packaged(&genome, "the habitat's birth", "the toolchain's foundational positions and law"));
    }
    // whole
    let _ = std::fs::remove_file(birth_pending());
    Ok(out)
}

/// Whether the habitat's record was born, as memory says.
enum Genesis {
    /// no identity pinned yet
    Unpinned,
    /// memory holds the pinned identity's genesis
    Present,
    /// memory answered that it holds no genesis for the pinned identity,
    /// or the identity's memory allocation was never provisioned (no
    /// volume of it exists): what was made for it holds no record
    Never(String),
}

/// GH #1653: the pinned identity's genesis, asked of memory. Memory that is
/// not up is brought up (the habitat's own, a habitat act) and asked again;
/// memory that still cannot be reached is a refusal, and nothing is
/// destroyed on a guess.
fn genesis_state(genome: &Path) -> Result<Genesis, String> {
    let Some(id) = manifest_pin(&genome.join("hale.toml")).ok().flatten() else { return Ok(Genesis::Unpinned) };
    let key = id[..12.min(id.len())].to_string();
    let asked = super::record_exists(genome);
    if let Ok(present) = asked {
        return Ok(if present { Genesis::Present } else { Genesis::Never(id) });
    }
    // memory could not say: the habitat's own allocation that never had a
    // volume never held a record
    if !own_servers() && !memory_volume_exists(&key) {
        return Ok(Genesis::Never(id));
    }
    let up = memory_migrate(genome);
    match (up, super::record_exists(genome)) {
        (_, Ok(true)) => Ok(Genesis::Present),
        (_, Ok(false)) => Ok(Genesis::Never(id)),
        (Err(e), Err(_)) | (_, Err(e)) => Err(format!("the habitat's memory cannot be reached to say whether its record ({key}…) was born, and nothing is undone on a guess: {e}")),
    }
}

/// Whether the memory volume of the allocation keyed `key` may exist in
/// the habitat's project: false only when docker answers that it has no
/// such volume; anything else (no docker, no daemon, an error) is taken as
/// "it may", so nothing is drawn again on a guess.
pub(super) fn memory_volume_exists(key: &str) -> bool {
    let name = format!("{}_memory-{key}", local_habitat_project());
    match Command::new("docker").args(["volume", "inspect", &name]).output() {
        Ok(o) if !o.status.success() => !String::from_utf8_lossy(&o.stderr).to_lowercase().contains("no such volume"),
        _ => true,
    }
}

/// The allocation keyed `key` taken down whole (its services and volumes)
/// and its cache removed: what a birth that never reached memory leaves.
/// Only ever called once memory has answered that the identity holds no
/// record (`genesis_state`).
fn forget_allocation(key: &str) {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(key);
    if dir.join("compose.yaml").is_file() && dir.join("ports.yaml").is_file() {
        let _ = Command::new("env")
            .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
            .arg(local_habitat_project())
            .arg("--project-directory")
            .arg(&dir)
            .arg("-f")
            .arg(dir.join("compose.yaml"))
            .arg("-f")
            .arg(dir.join("ports.yaml"))
            .args(["down", "-v"])
            .output();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The record identity taken out of the manifest's `dna` table, every
/// other line kept.
fn unpin(manifest: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{} does not parse: {e}", manifest.display()))?;
    if let Some(t) = doc.get_mut("dna").and_then(|d| d.as_table_like_mut()) {
        t.remove("record");
    }
    std::fs::write(manifest, doc.to_string()).map_err(|e| format!("write {}: {e}", manifest.display()))
}

// ---------------------------------------------------------------
// the faces: the habitat's own, and each organism's address
// ---------------------------------------------------------------

/// How many consecutive loopback ports one organism's head takes: the
/// face (the port a browser opens), the head behind it, the API child, the
/// stub provider and the API child's commands, in that order from the
/// block's base. `hale dna head` reads them back from the organism's
/// `dna.head.url`, so the address the habitat records is the address the
/// head serves on.
pub(crate) const HEAD_BLOCK: u16 = 8;

/// The first port a block is looked for from; each habitat starts its
/// search at its own offset, so two habitats on one machine (a test's
/// beside the developer's) do not meet.
const HEAD_BLOCK_FROM: u16 = 8800;

pub(crate) fn head_url_at(root: &Path) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(root).args(["config", "--get", "dna.head.url"]).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// The port of a head URL on this machine (the loopback, or the address the
/// habitat is recorded at), `None` for any other address.
pub(crate) fn loopback_port(url: &str) -> Option<u16> {
    let recorded = format!("http://{}:", habitat_address());
    let rest = url.strip_prefix("http://127.0.0.1:").or_else(|| url.strip_prefix("http://localhost:")).or_else(|| url.strip_prefix(recorded.as_str()))?;
    let port: u16 = rest.trim_end_matches('/').parse().ok()?;
    (port >= 1 && u32::from(port) + u32::from(HEAD_BLOCK) <= 65535).then_some(port)
}

/// The fixed ports a body's senses scrape endpoints listen on, every
/// interface, beside the habitat's head and face (`dna/core/senses.hl`:
/// spine, node, head). A head block is never placed over them: on the
/// host's network the body holds them, and a face given one could not bind.
const SENSES_PORTS: [u16; 3] = [9464, 9465, 9466];

fn overlaps_senses(base: u16) -> bool {
    SENSES_PORTS.iter().any(|p| *p >= base && u32::from(*p) < u32::from(base) + u32::from(HEAD_BLOCK))
}

fn block_free(base: u16) -> bool {
    (0..5).all(|i| std::net::TcpListener::bind(("127.0.0.1", base + i)).is_ok())
}

/// The base port of the head block of the organism at `root`: the one its
/// `dna.head.url` names, or, when it names none, the first block from the
/// habitat's starting point that no block in `taken` overlaps and whose
/// ports are free, recorded there.
fn head_block(root: &Path, taken: &[u16]) -> Result<u16, String> {
    if let Some(port) = head_url_at(root).as_deref().and_then(loopback_port) {
        return Ok(port);
    }
    let h = openssl::sha::sha256(local_habitat_dir().as_bytes());
    let blocks = (65000 - u32::from(HEAD_BLOCK_FROM)) / u32::from(HEAD_BLOCK);
    let start = u32::from(u16::from_be_bytes([h[0], h[1]])) % 200;
    for i in 0..blocks {
        let base = HEAD_BLOCK_FROM + (((start + i) % blocks) as u16) * HEAD_BLOCK;
        if taken.iter().any(|t| t.abs_diff(base) < HEAD_BLOCK) || overlaps_senses(base) || !block_free(base) {
            continue;
        }
        let ok = Command::new("git").arg("-C").arg(root).args(["config", "--local", "dna.head.url", &format!("http://127.0.0.1:{base}")]).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("could not record the head's address in {}'s git config", root.display()));
        }
        return Ok(base);
    }
    Err("no block of free loopback ports for a head".to_string())
}

/// The bases of the head blocks the habitat's grafts hold, from their rows.
fn granted_bases(genome: &Path) -> Vec<u16> {
    rows_of(genome).iter().filter(|r| r["kind"] == "habitat.grafted").filter_map(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())).filter_map(|b| b["face"].as_str().and_then(loopback_port)).collect()
}

fn face_state_dir() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("face-state")
}

/// The habitat's face as it runs in the habitat's compose project, or `None`.
fn face_running() -> Option<String> {
    service_status("face").map(|s| format!("compose service face, {s}"))
}

fn docker_socket_path() -> String {
    if cfg!(target_os = "macos") {
        return "/var/run/docker.sock".to_string();
    }
    if let Ok(h) = std::env::var("DOCKER_HOST") {
        if let Some(s) = h.trim().strip_prefix("unix://") {
            return s.to_string();
        }
    }
    if let Ok(o) = Command::new("docker").args(["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"]).output() {
        if o.status.success() {
            if let Some(s) = String::from_utf8_lossy(&o.stdout).trim().strip_prefix("unix://") {
                return s.to_string();
            }
        }
    }
    "/var/run/docker.sock".to_string()
}

/// Ensures the habitat's `habitat.profile` row records the head and face
/// addresses (`head` and `face`), writing an updated row if it was born
/// before they were assigned (`--no-body`).
fn ensure_profile_head(genome: &Path, backends: &str, name: &str, face_url: &str, head_url: &str) -> Result<(), String> {
    let latest = rows_of(genome)
        .into_iter()
        .rev()
        .find(|r| r["kind"] == "habitat.profile")
        .and_then(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()));
    if latest.as_ref().is_some_and(|b| b["head"].as_str() == Some(head_url) && b["face"].as_str() == Some(face_url)) {
        return Ok(());
    }
    let profile = PathBuf::from(local_habitat_dir()).join(".profile-row.jsonl");
    let row = serde_json::json!({
        "kind": "habitat.profile",
        "entity": "local",
        "body": serde_json::json!({
            "profile": "local",
            "backends": backends,
            "project": local_habitat_project(),
            "name": name,
            "face": face_url,
            "head": head_url,
        }).to_string()
    });
    std::fs::write(&profile, format!("{row}\n")).map_err(|e| format!("write {}: {e}", profile.display()))?;
    let landed = host_run("record-seed", genome, &[profile.display().to_string(), String::new()]);
    let _ = std::fs::remove_file(&profile);
    landed?;
    Ok(())
}

/// The first IPv4 address `name` resolves to on this machine, through the
/// toolchain's own resolver (the platform's getaddrinfo, no external
/// tool): a host's listen address (`HALE_DNA_HOST_LISTEN`,
/// `hale habitat resolve`), or why there is none.
pub(crate) fn first_ipv4(name: &str) -> Result<String, String> {
    ipv4_of(name, &resolved(name)?)
}

/// Every address `name` resolves to on this machine, through the
/// toolchain's own resolver (the platform's getaddrinfo), in its order.
pub(crate) fn resolved(name: &str) -> Result<Vec<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs;
    Ok((name, 0u16).to_socket_addrs().map_err(|e| format!("{name} does not resolve here: {e}"))?.collect())
}

/// The first IPv4 address among `addrs`, `name`'s: a host's listen
/// address as a project placed there is told it.
pub(crate) fn ipv4_of(name: &str, addrs: &[std::net::SocketAddr]) -> Result<String, String> {
    addrs.iter().find(|a| a.is_ipv4()).map(|a| a.ip().to_string()).ok_or_else(|| format!("{name} resolves to no IPv4 address here"))
}

/// The first IPv4 address among `addrs`, else the first address of any
/// family: where a habitat on another machine binds its face and head.
pub(crate) fn ipv4_or_first(name: &str, addrs: &[std::net::SocketAddr]) -> Result<String, String> {
    addrs.iter().find(|a| a.is_ipv4()).or_else(|| addrs.first()).map(|a| a.ip().to_string()).ok_or_else(|| format!("{name} resolves to no address here"))
}

/// The address the habitat declares its person-facing listeners on: the
/// loopback for a local habitat; the host's interface for a habitat on
/// another machine, which `up --host` hands every container it runs there
/// (`HALE_DNA_HABITAT_LISTEN`). The one knob for it; the face service's
/// bind (`HALE_DNA_LISTEN_HOST`), its probes and, on macOS, its published
/// ports follow it.
fn habitat_listen_host() -> String {
    std::env::var(remote::LISTEN_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| "127.0.0.1".to_string())
}

/// The host the habitat's face and head are recorded at (their URLs, on
/// its `habitat.profile` row and in `head.url`): the loopback for a local
/// habitat, the host's name for one on another machine
/// (`HALE_DNA_HABITAT_ADDRESS`, which `up --host` hands its containers).
/// The habitat's own machine as its readings name it: the address it is
/// recorded at, or `local` on the loopback (as the head's `host_label`).
fn host_label_here() -> String {
    let at = habitat_address();
    if at == "127.0.0.1" || at == "localhost" { "local".to_string() } else { at }
}

fn habitat_address() -> String {
    std::env::var(remote::ADDRESS_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| "127.0.0.1".to_string())
}

/// `ghcr.io/hale-lang/hale:<this version>` pulled and tagged `tag`, when it
/// runs exactly this toolchain (`hale --version` alike: a build of another
/// commit under the same version is not this one): the published image's
/// name, or `None` (none published, unreachable, or another build), when
/// compose builds the image instead.
fn pull_published(tag: &str) -> Option<String> {
    let published = format!("ghcr.io/hale-lang/hale:{}", env!("CARGO_PKG_VERSION"));
    let ours = Command::new(std::env::current_exe().ok()?).arg("--version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;
    progress(&format!("pulling {published}, this release's image"));
    if !matches!(docker(&["pull", "-q", &published]), Docker::Answered(_)) {
        return None;
    }
    let theirs = match docker(&["run", "--rm", "--entrypoint", "hale", &published, "--version"]) {
        Docker::Answered(v) => v.trim().to_string(),
        _ => return None,
    };
    if theirs != ours {
        progress(&format!("{published} is another build of this version; building the image instead"));
        return None;
    }
    matches!(docker(&["tag", &published, tag]), Docker::Answered(_)).then_some(published)
}

/// The socket address the habitat's listener on `port` is probed at.
fn listen_socket(port: u16) -> std::net::SocketAddr {
    let ip: std::net::IpAddr = habitat_listen_host().parse().unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
    std::net::SocketAddr::new(ip, port)
}

/// Renders the habitat's `body` and `face` services into its allocation's
/// `compose.yaml` and `ports.yaml` (alongside the embedded `Dockerfile` and
/// `hale.pin`), brings them up with `docker compose up -d`, and waits on
/// their probes.
fn start_body_and_face(genome: &Path, backends: &str, name: &str, asked_image: Option<&str>) -> Result<Vec<String>, String> {
    let base = head_block(genome, &granted_bases(genome))?;
    let face_url = format!("http://{}:{base}", habitat_address());
    let head_url = format!("http://{}:{}", habitat_address(), base + 1);
    std::fs::create_dir_all(local_habitat_dir()).map_err(|e| format!("create {}: {e}", local_habitat_dir()))?;
    std::fs::write(head_url_file(), format!("{head_url}\n")).map_err(|e| format!("write {}: {e}", head_url_file().display()))?;
    ensure_profile_head(genome, backends, name, &face_url, &head_url)?;
    std::fs::create_dir_all(face_state_dir()).map_err(|e| format!("create {}: {e}", face_state_dir().display()))?;

    let dir = allocation_dir(genome).ok_or("the habitat's genome pins no record identity")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let pin = HABITAT_PIN.trim();
    std::fs::write(dir.join("Dockerfile"), HABITAT_DOCKERFILE).map_err(|e| format!("write {}: {e}", dir.join("Dockerfile").display()))?;
    std::fs::write(dir.join("hale.pin"), format!("{pin}\n")).map_err(|e| format!("write {}: {e}", dir.join("hale.pin").display()))?;

    let env_image = std::env::var("HALE_HABITAT_IMAGE").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let override_image = asked_image.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).or(env_image);
    let image = override_image.clone().unwrap_or_else(|| default_habitat_image_tag(pin));
    // the published image of this release, when this machine has no image
    // of this toolchain yet: pulled, checked to run this very toolchain,
    // and tagged as the one compose would otherwise build (a build compiles
    // the compiler and the head's programs, several GB at its peak, more
    // than a small machine's docker gives)
    let mut pulled = None;
    if override_image.is_none() && matches!(docker(&["image", "inspect", "-f", "{{.Id}}", &image]), Docker::Failed(_)) {
        pulled = pull_published(&image);
    }
    let build_spec = if override_image.is_some() || pulled.is_some() {
        String::new()
    } else {
        format!("    build:\n      context: \"{HALE_GIT}#{pin}\"\n")
    };

    let hab_dir = local_habitat_dir();
    let vault = super::vault_local_dir().unwrap_or_else(|| PathBuf::from(&hab_dir).join("vault"));
    if !vault.exists() {
        let _ = std::fs::create_dir_all(&vault);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o700));
        }
    }
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| {
        PathBuf::from(&hab_dir).parent().and_then(|p| p.parent()).map(|p| p.display().to_string()).unwrap_or_else(|| hab_dir.clone())
    });
    let user = std::env::var("USER").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "hale".to_string());
    let sock = docker_socket_path();

    let mut mounts = vec![format!("      - \"{hab_dir}:{hab_dir}\"")];
    let vault_str = vault.display().to_string();
    if vault_str != hab_dir && !vault_str.starts_with(&format!("{hab_dir}/")) {
        mounts.push(format!("      - \"{vault_str}:{vault_str}\""));
    }
    mounts.push(format!("      - \"{sock}:/var/run/docker.sock\""));
    let mounts_yaml = mounts.join("\n");

    let mut env_file_spec = String::new();
    if backends == "operator" {
        let env_path = PathBuf::from(&hab_dir).join("backends.env");
        let text = ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"]
            .iter()
            .map(|k| format!("{k}={}\n", std::env::var(k).unwrap_or_default()))
            .collect::<String>();
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&env_path).map_err(|e| format!("write {}: {e}", env_path.display()))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("write {}: {e}", env_path.display()))?;
        env_file_spec = format!("    env_file:\n      - \"{}\"\n", env_path.display());
    }

    let is_macos = cfg!(target_os = "macos");
    let declared = habitat_listen_host();
    // The address the face's person-facing listeners (the face, the head,
    // the OIDC stub) bind, `HALE_DNA_LISTEN_HOST`. On the host's network
    // (Linux) it is the habitat's declared address itself. On the
    // project's network (macOS) they bind every interface of the
    // container, which is safe only because compose publishes their ports
    // on the declared address alone (`ports.yaml` below), never on the
    // host's other interfaces.
    let face_listen = if is_macos { "0.0.0.0".to_string() } else { declared.clone() };
    // The host the face, head and identity stub are recorded under, which
    // the head answers as and the stub names its issuer and endpoints by.
    let declared_at = habitat_address();
    let net_or_hosts = if is_macos {
        "    extra_hosts:\n      - \"host.docker.internal:host-gateway\"\n".to_string()
    } else {
        "    network_mode: \"host\"\n".to_string()
    };
    let user_spec = if is_macos {
        String::new()
    } else {
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        use std::os::unix::fs::MetadataExt;
        let sock_gid = std::fs::metadata(&sock).map(|m| m.gid()).unwrap_or(gid);
        format!("    user: \"{uid}:{gid}\"\n    group_add:\n      - \"{sock_gid}\"\n")
    };
    // Where these services reach a port published on this machine's
    // loopback (`dna::loopback_host`): on the host's network that is
    // 127.0.0.1 itself; on the project's network (macOS) it is the host.
    let loopback = if is_macos {
        "      HALE_DNA_LOOPBACK_HOST: \"host.docker.internal\"\n"
    } else {
        ""
    };
    let mut forwarded_env = String::new();
    if let Docker::Answered(ver) = docker(&["version", "--format", "{{.Server.APIVersion}}"]) {
        let ver = ver.trim();
        if !ver.is_empty() && !ver.contains(['"', '\n', '\r']) {
            forwarded_env.push_str(&format!("      DOCKER_API_VERSION: \"{ver}\"\n"));
        }
    }
    for k in ["HALE_DNA_DISCOVER", "HALE_DNA_TRUSTED_LOCAL", "HALE_DNA_WAIT_SCALE", "HALE_DNA_FORGE", "LOTUS_DRAIN_GRACE_MS", "HALE_DNA_SUITE_TAG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() && !v.contains(['"', '\n', '\r']) {
                forwarded_env.push_str(&format!("      {k}: \"{v}\"\n"));
            }
        }
    }

    // Grafana, behind the face: on the face's loopback at the block's
    // GRAFANA_OFFSET, trusting the header only the two of them know
    // (none without a senses' store of the habitat's own to chart: an
    // operator's backends hold none)
    let key = dir.file_name().map(|k| k.to_string_lossy().into_owned()).unwrap_or_default();
    let senses = readings::senses_port(&dir, &key);
    let grafana = match senses {
        Some(senses) if !readings::grafana_off() => {
            let reach = if is_macos { "host.docker.internal" } else { "127.0.0.1" };
            readings::write_provisioning(Path::new(&hab_dir), &format!("http://{reach}:{senses}"))?;
            Some((base + readings::GRAFANA_OFFSET, readings::grafana_header(Path::new(&hab_dir))?))
        }
        _ => None,
    };
    let grafana_note = (grafana.is_none() && senses.is_none() && !readings::grafana_off()).then(|| format!("{:<8}none: the habitat has no senses' store of its own to chart ({} publishes none)", "grafana", dir.join("ports.yaml").display()));
    // on demand (unless HALE_DNA_GRAFANA=on), the face starts and stops it
    // with the compose files of this directory
    let on_demand = readings::grafana_runs() == readings::GrafanaRuns::OnDemand;
    let compose_dir = dir.display().to_string();
    let grafana_env = grafana.as_ref().map(|(port, header)| readings::grafana_face_env(*port, header, on_demand.then_some(compose_dir.as_str()))).unwrap_or_default();
    // what the senses' store scrapes of the machines: this one, until the
    // head writes the hosts it adds beside it
    readings::seed_targets(&dir, &host_label_here())?;
    // and what it watches for: the habitat's reflexes, its defaults and its
    // genome's, read by its store from now on (restarted when they moved)
    let reflexes_line = if senses.is_some() {
        let manifest = std::fs::read_to_string(genome.join("hale.toml")).unwrap_or_default();
        // (and the job of the machines' containers, beside its machines')
        let moved = reflexes::write_habitat_rules(&dir, &key, &manifest)? | readings::ensure_containers_job(&dir)? | readings::ensure_services_job(&dir)?;
        if moved {
            let _ = Command::new("env")
                .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
                .arg(local_habitat_project())
                .arg("--project-directory")
                .arg(&dir)
                .arg("-f")
                .arg(dir.join("compose.yaml"))
                .arg("-f")
                .arg(dir.join("ports.yaml"))
                .args(["restart", &format!("senses-{key}")])
                .output();
        }
        let names: Vec<String> = reflexes::habitat_defaults().into_iter().map(|r| r.name).chain(reflexes::declared(&manifest).unwrap_or_default().into_iter().map(|r| r.name)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        Some(format!("reflexes {} watched by its senses' store ({}; the operator face's alerts)", names.len(), names.join(", ")))
    } else {
        None
    };

    let drain_secs = drain_window().as_secs();
    let g = genome.display();
    let fstate = face_state_dir().display().to_string();
    let mut services_block = String::from("# habitat-body-and-face\n  body:\n");
    services_block.push_str(&format!("    image: \"{image}\"\n"));
    services_block.push_str(&compose_labels("habitat", "body"));
    services_block.push_str(&build_spec);
    services_block.push_str("    init: true\n");
    services_block.push_str(&user_spec);
    services_block.push_str(&net_or_hosts);
    services_block.push_str(&format!("    working_dir: \"{g}\"\n"));
    services_block.push_str(&env_file_spec);
    services_block.push_str(&format!("    environment:\n      HOME: \"{hab_dir}\"\n      USER: \"{user}\"\n      XDG_DATA_HOME: \"{data}\"\n      HALE_DNA_HABITAT: \"{hab_dir}\"\n      HALE_VAULT_DIR: \"{vault_str}\"\n      HALE_DNA_COMPOSE: \"off\"\n      HALE_DNA_HABITAT_ADDRESS: \"{declared_at}\"\n"));
    services_block.push_str(loopback);
    services_block.push_str(&forwarded_env);
    // The body drains onto the habitat's own memory and nerves (its lease
    // goes back in memory) when it stops: it depends on them, so compose
    // stops it, within its grace, before them whatever stops the project
    // (`compose down`, a body `hale habitat down` did not find running).
    let key = dir.file_name().map(|k| k.to_string_lossy().into_owned()).unwrap_or_default();
    let present = std::fs::read_to_string(dir.join("compose.yaml")).unwrap_or_default();
    let backends_of_body: Vec<String> = ["memory", "nerves"].iter().map(|r| format!("{r}-{key}")).filter(|svc| !key.is_empty() && present.contains(&format!("\n  {svc}:\n"))).collect();
    let depends = if backends_of_body.is_empty() { String::new() } else { format!("    depends_on:\n{}", backends_of_body.iter().map(|svc| format!("      - \"{svc}\"\n")).collect::<String>()) };
    services_block.push_str(&format!("    volumes:\n{mounts_yaml}\n    command: [\"dna\", \"dev\", \"{g}\", \"--no-iris\"]\n    restart: on-failure\n    stop_grace_period: {drain_secs}s\n{depends}  face:\n    image: \"{image}\"\n"));
    services_block.push_str(&compose_labels("habitat", "face"));
    services_block.push_str(&build_spec);
    services_block.push_str("    init: true\n");
    services_block.push_str(&user_spec);
    services_block.push_str(&net_or_hosts);
    services_block.push_str(&format!("    working_dir: \"{g}\"\n"));
    services_block.push_str(&env_file_spec);
    services_block.push_str(&format!("    environment:\n      HOME: \"{hab_dir}\"\n      USER: \"{user}\"\n      XDG_DATA_HOME: \"{data}\"\n      HALE_DNA_HABITAT: \"{hab_dir}\"\n      HALE_VAULT_DIR: \"{vault_str}\"\n      HALE_DNA_HEAD_STATE: \"{fstate}\"\n      HALE_DNA_FACE_HOME: \"/app/\"\n      HALE_DNA_IDENTITY: \"publish\"\n      HALE_DNA_LISTEN_HOST: \"{face_listen}\"\n      HALE_DNA_HABITAT_ADDRESS: \"{declared_at}\"\n"));
    services_block.push_str(loopback);
    services_block.push_str(&forwarded_env);
    services_block.push_str(&grafana_env);
    services_block.push_str(&format!("    volumes:\n{mounts_yaml}\n    command: [\"dna\", \"head\", \"{g}\"]\n    restart: on-failure\n"));
    if let Some((port, header)) = &grafana {
        services_block.push_str(&readings::grafana_service(&hab_dir, *port, &face_url, header, &user_spec));
    }

    let compose_path = dir.join("compose.yaml");
    let existing_compose = std::fs::read_to_string(&compose_path).unwrap_or_else(|_| "services:\n".to_string());
    let base_compose = if let Some((before, after)) = existing_compose.split_once("# habitat-body-and-face\n") {
        if let Some(vol_idx) = after.find("\nvolumes:\n") {
            format!("{before}{}", &after[vol_idx + 1..])
        } else {
            before.to_string()
        }
    } else {
        existing_compose
    };
    let new_compose = if let Some((services_part, volumes_part)) = base_compose.split_once("\nvolumes:\n") {
        format!("{}\n{services_block}volumes:\n{volumes_part}", services_part.trim_end())
    } else {
        format!("{}\n{services_block}", base_compose.trim_end())
    };
    std::fs::write(&compose_path, new_compose).map_err(|e| format!("write {}: {e}", compose_path.display()))?;

    let ports_path = dir.join("ports.yaml");
    let existing_ports = std::fs::read_to_string(&ports_path).unwrap_or_else(|_| "services:\n".to_string());
    let base_ports = existing_ports.split("# habitat-body-and-face\n").next().unwrap_or(&existing_ports).trim_end().to_string();
    let new_ports = if is_macos {
        let mut p = format!("{base_ports}\n# habitat-body-and-face\n  face:\n    ports:\n");
        for offset in (0..HEAD_BLOCK).filter(|o| *o != readings::GRAFANA_OFFSET) {
            let port = base + offset;
            p.push_str(&format!("      - \"{declared}:{port}:{port}\"\n"));
        }
        p
    } else if base_ports.trim() == "services:" {
        "services: {}\n".to_string()
    } else {
        format!("{base_ports}\n")
    };
    std::fs::write(&ports_path, new_ports).map_err(|e| format!("write {}: {e}", ports_path.display()))?;

    progress("starting body and face compose services");
    let mut cmd = Command::new("env");
    cmd.args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
        .arg(local_habitat_project())
        .arg("--project-directory")
        .arg(&dir)
        .arg("-f")
        .arg(&compose_path)
        .arg("-f")
        .arg(&ports_path)
        // their backends are the habitat's allocations, brought up by
        // their provider: `depends_on` orders the stop, never this start
        .args(["up", "-d", "--no-deps", "body", "face"])
        .args(grafana.as_ref().filter(|_| !on_demand).map(|_| "grafana"));
    // a Grafana an earlier `up` started, when this one runs none or runs it
    // on demand (made again below, joined to this face's network: a
    // container joined to a face this `up` replaced would never start). The
    // project holds the organisms' allocations too: never --remove-orphans.
    if grafana.is_none() || on_demand {
        if let Docker::Answered(ids) = docker(&["ps", "-aq", "--filter", &format!("label=com.docker.compose.project={}", local_habitat_project()), "--filter", "label=com.docker.compose.service=grafana"]) {
            let ids: Vec<&str> = ids.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if !ids.is_empty() {
                let mut a = vec!["rm", "-f"];
                a.extend(ids);
                let _ = docker(&a);
            }
        }
    }
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        cmd.stderr(std::process::Stdio::inherit());
    }
    let ran = cmd.output().map_err(|e| format!("docker compose: {e}"))?;
    if !ran.status.success() {
        return Err(format!("docker compose up -d body face: {}", String::from_utf8_lossy(&ran.stderr).trim()));
    }
    // an on-demand Grafana is created (its image pulled) and not started:
    // the face starts it at its first use (`compose start`, which makes
    // nothing). Without --no-deps, so compose joins it to the face's
    // network: with it, compose (2.6) leaves `service:face` unresolved, and
    // the container never starts. The face it depends on is up already.
    if grafana.is_some() && on_demand {
        let ran = Command::new("env")
            .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
            .arg(local_habitat_project())
            .arg("--project-directory")
            .arg(&dir)
            .arg("-f")
            .arg(&compose_path)
            .arg("-f")
            .arg(&ports_path)
            .args(["up", "--no-start", "grafana"])
            .output()
            .map_err(|e| format!("docker compose: {e}"))?;
        if !ran.status.success() {
            return Err(format!("docker compose up --no-start grafana: {}", String::from_utf8_lossy(&ran.stderr).trim()));
        }
    }

    progress("waiting for the habitat's body and face probes");
    // three minutes to answer once the face has what it runs; a face still
    // building the head's programs (a first start, or one after the
    // toolchain changed) says so in its log, and is waited for up to
    // FACE_BUILD_WAIT
    let begun = std::time::Instant::now();
    let mut deadline = begun + std::time::Duration::from_secs(180);
    let mut said_building = String::new();
    let mut body_ok = false;
    let mut face_ok = false;
    while std::time::Instant::now() < deadline {
        body_ok = body_state().is_some();
        face_ok = face_running().is_some()
            && std::net::TcpStream::connect_timeout(
                &listen_socket(base),
                std::time::Duration::from_millis(250),
            )
            .is_ok()
            && head_is_up();
        if body_ok && face_ok {
            break;
        }
        if let Some(seed) = building(&service_log_tail("face", 5)) {
            if seed != said_building {
                progress(&format!("the face is building the head's {seed}"));
                said_building = seed;
            }
            deadline = deadline.max((std::time::Instant::now() + std::time::Duration::from_secs(180)).min(begun + FACE_BUILD_WAIT));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if !body_ok {
        return Err(format!("the habitat's body service did not stay up; its last lines:\n{}", service_log_tail("body", 30)));
    }
    if !face_ok {
        return Err(format!("the habitat's face and head did not answer on {face_url} and {head_url} within {}s; the face's last lines:\n{}", begun.elapsed().as_secs(), service_log_tail("face", 30)));
    }
    let mut lines = vec![format!("{:<8}up (compose service face, {face_url}/)", "face"), format!("{:<8}up (compose service body)", "body")];
    let link = sign_in_link(&face_url);
    if !link.is_empty() {
        lines.push(format!("{:<8}{link}", "sign in"));
    }
    if grafana.is_some() {
        let when = if on_demand { format!("; started at its first use, stopped after {} minutes unused", readings::GRAFANA_IDLE_MINUTES) } else { String::new() };
        lines.push(format!("{:<8}{face_url}{} (compose service grafana, behind the face's sign-in{when})", "grafana", readings::GRAFANA_PATH));
    }
    lines.extend(grafana_note);
    lines.extend(reflexes_line);
    lines.extend(readings::install_here(&host_label_here()));
    Ok(lines)
}

/// Phase 4: the habitat's face (and the head behind it) restarted, after
/// its own memory or nerves moved.
fn restart_face() -> Result<String, String> {
    let genome = genome_dir();
    let dir = allocation_dir(&genome).ok_or("the habitat's genome pins no record identity")?;
    let ran = Command::new("env")
        .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
        .arg(local_habitat_project())
        .arg("--project-directory")
        .arg(&dir)
        .arg("-f")
        .arg(dir.join("compose.yaml"))
        .arg("-f")
        .arg(dir.join("ports.yaml"))
        .args(["restart", "face"])
        .output()
        .map_err(|e| format!("docker compose restart face: {e}"))?;
    if !ran.status.success() {
        return Err(format!("docker compose restart face: {}", String::from_utf8_lossy(&ran.stderr).trim()));
    }
    Ok(format!("{:<8}restarted (compose service face)", "face"))
}

/// `hale habitat brief`: the head's `Commands::brief`, printed.
fn brief() -> ExitCode {
    match sapling::head_call("Commands::brief", serde_json::json!({ "limit": 200 })) {
        Ok(v) if v["ok"] == serde_json::json!(true) => {
            print!("{}", v["prompt"].as_str().unwrap_or(""));
            ExitCode::SUCCESS
        }
        Ok(v) => {
            eprintln!("hale habitat brief: the head answered no brief ({})", v["why"].as_str().unwrap_or(""));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("hale habitat brief: {e}");
            ExitCode::from(1)
        }
    }
}

/// The port the habitat's head serves its surface over MCP on, as its API
/// child reported it (`mcp=<port>` in the face state's `children/api.ports`).
fn mcp_port() -> Option<u16> {
    let text = std::fs::read_to_string(face_state_dir().join("children").join("api.ports")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("mcp=")).and_then(|p| p.trim().parse().ok()).filter(|p| *p > 0)
}

fn status() -> ExitCode {
    let genome = genome_dir();
    println!("habitat local at {} (compose project {})", local_habitat_dir(), local_habitat_project());
    // its name, as its record holds it (the `habitat.profile` row)
    let recorded = rows_of(&genome).iter().rev().find(|r| r["kind"] == "habitat.profile").and_then(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())).and_then(|b| b["name"].as_str().map(str::to_string));
    let kept = std::fs::read_to_string(name_file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    println!("{:<8}{}", "name", recorded.or(kept).unwrap_or_else(|| DEFAULT_HABITAT_NAME.to_string()));
    match manifest_pin(&genome.join("hale.toml")).ok().flatten() {
        Some(id) => println!("genome  {} (record {}…)", genome.display(), &id[..12.min(id.len())]),
        None => println!("genome  not born: `hale habitat up` births it"),
    }
    println!("{:<8}{}", "body", body_state().unwrap_or_else(|| "down: `hale habitat up` starts it".to_string()));
    match head_url_at(&genome).as_deref().and_then(loopback_port) {
        Some(port) => {
            println!("{:<8}http://{}:{port}/ ({})", "face", habitat_address(), face_running().map(|s| format!("up, {s}")).unwrap_or_else(|| "down: `hale habitat up` starts it".to_string()));
            let link = sign_in_link(&format!("http://{}:{port}", habitat_address()));
            if !readings::grafana_off() {
                println!("{:<8}http://{}:{port}{}", "grafana", habitat_address(), readings::GRAFANA_PATH);
            }
            if !link.is_empty() && face_running().is_some() {
                println!("{:<8}{link}", "sign in");
            }
        }
        None => println!("{:<8}not assigned: `hale habitat up` assigns its address", "face"),
    }
    if face_running().is_some() {
        if let Some(port) = mcp_port() {
            println!("{:<8}mcp://127.0.0.1:{port} (the head's commands over MCP: `hale mcp --app mcp://127.0.0.1:{port}`, the person's ID token in HALE_API_TOKEN)", "mcp");
        }
    }
    // the services placed on its hosts, under their hosts, as its record
    // holds them (none said when the record is not readable)
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_some() {
        if let Ok(text) = host_run("record-rows", &genome, &[]) {
            let rows: Vec<serde_json::Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
            for l in placements::status_lines(&placements::fold(&rows).0) {
                println!("{l}");
            }
        }
    }
    let (rows, from) = match recorded_allocations() {
        Some(rows) => (rows, "the habitat's record"),
        None => (allocations(), "the allocations' cache (the habitat's record is not readable: its memory is not up)"),
    };
    println!("allocations, as {from}:");
    if rows.is_empty() {
        println!("allocations: none");
        return ExitCode::SUCCESS;
    }
    let running = running_services();
    let versions = rows.iter().find_map(|(_, r)| r["root"].as_str().map(PathBuf::from).filter(|p| p.join("hale.toml").is_file())).map(|r| toolchain_versions(&r)).unwrap_or_default();
    for (key, row) in rows {
        println!("allocation {key} ({})", row["root"].as_str().unwrap_or("no organism recorded"));
        for role in ["memory", "nerves", "senses", "database"] {
            if role == "database" && !row["roles"]["database"].is_object() {
                continue;
            }
            // the heart's database is a tenant of the memory server, and a
            // record the habitat's memory keeps names the habitat's service
            let own = format!("memory-{key}");
            let memory_service = row["roles"]["memory"]["service"].as_str().filter(|s| s.starts_with("memory-")).unwrap_or(&own).to_string();
            let tenant = memory_service != own;
            let own_nerves = format!("nerves-{key}");
            let nerves_service = row["roles"]["nerves"]["service"].as_str().filter(|s| s.starts_with("nerves-")).unwrap_or(&own_nerves).to_string();
            let nerves_tenant = nerves_service != own_nerves;
            let service = if role == "database" || role == "memory" { memory_service.clone() } else if role == "nerves" { nerves_service.clone() } else { format!("{role}-{key}") };
            let on_operator = row["roles"][role]["service"].as_str() == Some("operator");
            let state = if on_operator { "on the operator's server" } else if running.iter().any(|s| s == &service) { "running" } else { "stopped" };
            let recorded = row["roles"][role]["schema_version"].as_u64();
            let want = versions.iter().find(|(r, _)| r == role).map(|(_, v)| *v);
            // a heart role's schema is the heart's: its last migration run
            let migration = row["roles"][role]["migration"].as_str().map(|m| format!(", its migration last ran with {m}")).unwrap_or_default();
            let version = match (recorded, want) {
                (Some(had), Some(want)) if had < want => format!(", schema {had}, behind this toolchain's {want}: `hale habitat migrate {}` (or `hale dna upgrade` there) migrates it", row["root"].as_str().unwrap_or("<organism>")),
                (Some(had), Some(_)) => format!(", schema {had} (this toolchain's)"),
                (Some(had), None) => format!(", schema {had}"),
                (None, _) => String::new(),
            };
            let whose = if tenant && role == "memory" {
                ", the habitat's: a schema of its own there"
            } else if tenant && role == "database" {
                ", the habitat's: a database of its own there"
            } else if nerves_tenant && role == "nerves" {
                ", the habitat's: an account of its own there"
            } else {
                ""
            };
            println!("  {role:<7} {service} {state}{whose}{version}{migration}");
        }
    }
    ExitCode::SUCCESS
}

/// The running services of the habitat's project.
fn running_services() -> Vec<String> {
    match docker(&["ps", "--filter", &format!("label=com.docker.compose.project={}", local_habitat_project()), "--filter", "status=running", "--format", "{{.Label \"com.docker.compose.service\"}}"]) {
        Docker::Answered(out) => out.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// How long the habitat's body may take to drain once told to stop: the
/// host's drain grace (the organization's and its lease's, up to
/// `LOTUS_DRAIN_GRACE_MS`, the node default otherwise) and a margin. Its
/// backends are stopped only after it has gone, so the drain still reaches
/// them.
fn drain_window() -> std::time::Duration {
    let grace: u64 = std::env::var("LOTUS_DRAIN_GRACE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| super::NODE_DRAIN_GRACE_MS.parse().unwrap_or(30000));
    std::time::Duration::from_millis(grace + 15000)
}

/// `hale habitat down`: the habitat's compose project taken down (`docker
/// compose down`), leaving only its volumes; `up` brings it back.
/// `--forget` removes it whole, planned first (forget.rs).
fn down(args: &[String]) -> ExitCode {
    // run there through a pointer (`up --host`): the habitat here must be
    // the record the pointer names, or nothing is stopped or removed
    if let Some(expected) = std::env::var(remote::EXPECT_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        let found = manifest_pin(&genome_dir().join("hale.toml")).ok().flatten().unwrap_or_default();
        if found != expected {
            eprintln!(
                "hale habitat down: the habitat here holds the record {}, not the {} the pointer named; nothing is touched",
                if found.is_empty() { "none".to_string() } else { format!("{}…", &found[..12.min(found.len())]) },
                format!("{}…", &expected[..12.min(expected.len())])
            );
            return ExitCode::from(1);
        }
    }
    match args {
        [] => {}
        [f] if f == "--forget" => return forget::forget(false),
        [f, p] | [p, f] if f == "--forget" && p == "--plan" => return forget::forget(true),
        _ => {
            eprintln!("usage: hale habitat down [--forget [--plan]]");
            return ExitCode::from(2);
        }
    }
    let mut said = Vec::new();
    if let Err(why) = stop_processes(&mut said) {
        eprintln!("hale habitat down: {why}");
        return ExitCode::from(1);
    }
    let project = local_habitat_project();
    let ids = match docker(&["ps", "-a", "-q", "--filter", &format!("label=com.docker.compose.project={project}")]) {
        Docker::Answered(out) => out.lines().map(str::to_string).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        Docker::Absent => Vec::new(),
        Docker::Failed(why) => {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
    };
    if !ids.is_empty() || !said.is_empty() {
        if let Docker::Failed(why) = docker(&["compose", "-p", &project, "down", "--remove-orphans"]) {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
        if !ids.is_empty() {
            said.push(format!("stopped {} service(s) of {project}", ids.len()));
        }
    }
    if said.is_empty() {
        println!("the habitat is already down");
    } else {
        for l in said {
            println!("{l}");
        }
        println!("its data is kept: `hale habitat up` brings it back");
    }
    ExitCode::SUCCESS
}

/// The habitat's face and body services stopped first: its face, then its
/// body (drained onto its backends while they are still running). What was
/// stopped is said in `said`.
fn stop_processes(said: &mut Vec<String>) -> Result<(), String> {
    let project = local_habitat_project();
    for (service, timeout, msg) in [
        ("grafana", None, "stopped the habitat's Grafana (compose service grafana)"),
        ("face", None, "stopped the habitat's face (compose service face)"),
        ("body", Some(drain_window().as_secs().to_string()), "stopped the habitat's body (compose service body), drained"),
    ] {
        let Docker::Answered(out) = docker(&[
            "ps",
            "-q",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--filter",
            &format!("label=com.docker.compose.service={service}"),
            // a body between restarts is stopped (and drained) here too,
            // never left for `compose down` to stop beside its backends
            "--filter",
            "status=running",
            "--filter",
            "status=restarting",
        ]) else {
            continue;
        };
        let ids: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if ids.is_empty() {
            continue;
        }
        let mut args = vec!["stop"];
        if let Some(ref t) = timeout {
            args.push("-t");
            args.push(t.as_str());
        }
        args.extend(ids);
        if let Docker::Failed(why) = docker(&args) {
            return Err(why);
        }
        said.push(msg.to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------
// the teardown of a test's habitat (GH #1653)
// ---------------------------------------------------------------

/// The file under a root naming the compose projects found rooted in it,
/// one per line, kept until their teardown is confirmed: a volume carries
/// its project's name but not the directory it ran from, so a project
/// whose containers went and whose volumes did not is found again by name.
/// `dna::scratch_compose_down` keeps the same file.
pub(crate) const COMPOSE_FILE: &str = ".dna-compose-projects";

/// What docker answered: its output, nothing to ask (no CLI or no daemon:
/// nothing compose started can be running there), or why it failed.
pub(crate) enum Docker {
    Answered(String),
    Absent,
    Failed(String),
}

fn docker(args: &[&str]) -> Docker {
    match Command::new("docker").args(args).output() {
        Err(_) => Docker::Absent,
        Ok(o) if o.status.success() => Docker::Answered(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            if err.contains("Cannot connect to the Docker daemon") || err.contains("Is the docker daemon running") {
                Docker::Absent
            } else {
                Docker::Failed(format!("docker {}: {}", args.join(" "), err.trim()))
            }
        }
    }
}

/// The compose project of the local habitat that lives at `dir`, as
/// `dna::habitat_project_at` names it.
pub(crate) fn habitat_project_at(dir: &str) -> String {
    let h = openssl::sha::sha256(dir.as_bytes());
    format!("hale-habitat-local-{}", h[..4].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The projects of docker's `listed` (`<project>|<working dir>` lines)
/// rooted in `root`, and the ones `recorded` names, as
/// `dna::scratch_projects_in` finds them: never the developer's own
/// habitat (under `$HOME`), nor the one this process's `XDG_DATA_HOME`
/// names when it lives outside `root`. The machine's long-lived habitat is
/// no teardown's, whatever a file under a root says.
pub(crate) fn projects_rooted_in(listed: &str, root: &Path, recorded: &str) -> Vec<String> {
    let text = root.display().to_string();
    let under = format!("{text}/");
    let home = std::env::var("HOME").unwrap_or_default();
    let mine = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
    let here = local_habitat_dir();
    let current = if here == text || here.starts_with(&under) { None } else { Some(habitat_project_at(&here)) };
    let mut projects: Vec<String> = listed
        .lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(p, wd)| !p.is_empty() && (*wd == text || wd.starts_with(&under)))
        .map(|(p, _)| p.to_string())
        .chain(recorded.lines().filter(|l| !l.is_empty()).map(str::to_string))
        .filter(|p| *p != mine && Some(p) != current.as_ref())
        .collect();
    projects.sort();
    projects.dedup();
    projects
}

/// The compose projects `root` owns by what lies under it, as
/// `dna::scratch_owned_projects` finds them, whether or not a container of
/// theirs is left to carry a working directory (a project whose containers
/// went keeps its network and its volumes): each directory an `ssh` host
/// was handed a render in (`<home>/.hale/habitat/<project>/…`), and each
/// habitat born there (`…/hale/habitat/local`, its project
/// [`habitat_project_at`]). Never a project one of whose containers (in
/// docker's `listed`) ran from outside `root`.
pub(crate) fn owned_projects(root: &Path, listed: &str) -> Vec<String> {
    let text = root.display().to_string();
    let under = format!("{text}/");
    let outside: Vec<&str> = listed.lines().filter_map(|l| l.split_once('|')).filter(|(p, wd)| !p.is_empty() && *wd != text && !wd.starts_with(&under)).map(|(p, _)| p).collect();
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let parent = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let grand = dir.parent().and_then(|g| g.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if parent == "habitat" && grand == ".hale" {
                found.push(name);
            } else if name == "local" && parent == "habitat" && grand == "hale" {
                found.push(habitat_project_at(&path.display().to_string()));
            } else {
                stack.push(path);
            }
        }
    }
    found.retain(|p| !outside.contains(&p.as_str()));
    found.sort();
    found.dedup();
    found
}

/// Take down every compose project rooted in `root` — its containers,
/// networks and volumes (`docker compose -p <project> down -v`, which needs
/// no compose file) — and confirm none of it is left, by the project's
/// label on containers and on volumes. The projects are named in
/// `root`'s [`COMPOSE_FILE`] before anything is taken down and the file
/// goes only once nothing is left, so a teardown that did not finish (a
/// volume docker would not remove) is found again and retried by the next.
/// `Err` names what failed or survived; the caller keeps `root`. The one
/// teardown `hale test`, the CLI tests' reap and
/// `scripts/compose-down-under.sh` share (`hale habitat sweep`).
/// docker's compose containers, `<project>|<working dir>` a line.
const COMPOSE_LISTING: [&str; 6] = ["ps", "-a", "--filter", "label=com.docker.compose.project", "--format", "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}"];

/// Every compose container's project and working directory, as
/// [`down_under`] reads them; "" without docker.
pub(crate) fn compose_projects_listed() -> Result<String, String> {
    match docker(&COMPOSE_LISTING) {
        Docker::Absent => Ok(String::new()),
        Docker::Failed(why) => Err(why),
        Docker::Answered(out) => Ok(out),
    }
}

pub(crate) fn down_under(root: &Path) -> Result<(), String> {
    down_under_with(root, &mut |args| docker(args))
}

pub(crate) fn down_under_with(root: &Path, docker: &mut dyn FnMut(&[&str]) -> Docker) -> Result<(), String> {
    let file = root.join(COMPOSE_FILE);
    let recorded = std::fs::read_to_string(&file).unwrap_or_default();
    let listed = match docker(&COMPOSE_LISTING) {
        Docker::Absent => return Ok(()),
        Docker::Failed(why) => return Err(why),
        Docker::Answered(out) => out,
    };
    let owned = owned_projects(root, &listed).join("\n");
    let projects = projects_rooted_in(&listed, root, &format!("{recorded}\n{owned}"));
    if projects.is_empty() {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let _ = std::fs::create_dir_all(root);
    std::fs::write(&file, projects.iter().map(|p| format!("{p}\n")).collect::<String>()).map_err(|e| format!("write {}: {e}", file.display()))?;
    let mut left = Vec::new();
    for project in &projects {
        if let Docker::Failed(why) = docker(&["compose", "-p", project, "down", "-v", "--remove-orphans"]) {
            left.push(why);
        }
        let label = format!("label=com.docker.compose.project={project}");
        for (what, args) in [("containers", ["ps", "-a", "-q", "--filter", label.as_str()].as_slice()), ("volumes", ["volume", "ls", "-q", "--filter", label.as_str()].as_slice()), ("networks", ["network", "ls", "-q", "--filter", label.as_str()].as_slice())] {
            match docker(args) {
                Docker::Answered(out) if !out.trim().is_empty() => left.push(format!("compose project {project} still has {what} after its teardown")),
                Docker::Failed(why) => left.push(why),
                _ => {}
            }
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_file(&file);
        Ok(())
    } else {
        Err(left.join("\n"))
    }
}

// ---------------------------------------------------------------
// graft (GH #1653)
// ---------------------------------------------------------------

/// The organism's own roles, the ones the habitat provisions for the
/// organism itself (its record is memory's, its organization reads the
/// nerves, its body's readings go to the senses), and of them the ones it
/// always gets.
const ROLES: [&str; 3] = ["memory", "nerves", "senses"];
const ALWAYS: [&str; 2] = ["memory", "nerves"];
/// The roles a heart may need of its own: never given by default (a
/// foreign heart's infrastructure is its own, and none of it is the
/// habitat's to touch), only when a Hale heart's genome declares one. The
/// local profile provisions `database` (a tenant database of the
/// organism's memory server) and `broker` (a user of its nerves server
/// with a subject space and an inbox of its own); the rest are refused
/// until a heart needs them.
const HEART_ROLES: [&str; 4] = ["database", "broker", "cache", "object-store"];
const PROVISIONED_HEART_ROLES: [&str; 2] = ["database", "broker"];

/// GH #1653: the roles the genome at `dir` needs from the habitat, as its
/// manifest declares them (`[dna] needs = ["memory", "nerves", …]`): the
/// first half of graft, satisfied by its second. None declared (no
/// manifest, or a foreign heart's, which declares nothing) is exactly the
/// organism's own roles, never a heart's: the habitat provides nothing for
/// a heart that did not ask. A heart role is declared by a Hale heart that
/// needs one, and is refused for now (this habitat provisions none); a
/// role it does not know, or a list without memory or the nerves, is
/// refused by name.
pub(super) fn needs_of(manifest_text: Option<&str>) -> Result<Vec<String>, String> {
    let declared = match manifest_text {
        None => None,
        Some(text) => {
            let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
            match table.get("dna").and_then(|d| d.get("needs")) {
                None => None,
                Some(toml::Value::Array(a)) => Some(a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("[dna] needs holds {v}, not a role's name"))).collect::<Result<Vec<_>, _>>()?),
                Some(other) => return Err(format!("[dna] needs is {other}, not a list of roles")),
            }
        }
    };
    let Some(mut needs) = declared else { return Ok(ROLES.iter().map(|r| r.to_string()).collect()) };
    if let Some(r) = needs.iter().find(|r| HEART_ROLES.contains(&r.as_str()) && !PROVISIONED_HEART_ROLES.contains(&r.as_str())) {
        return Err(format!("[dna] needs names the heart role `{r}`, which this habitat does not provision yet (it provisions the organism's own, {}, and the heart roles {})", ROLES.join(", "), PROVISIONED_HEART_ROLES.join(", ")));
    }
    if let Some(r) = needs.iter().find(|r| !ROLES.contains(&r.as_str()) && !PROVISIONED_HEART_ROLES.contains(&r.as_str())) {
        return Err(format!("[dna] needs names `{r}`, which the habitat does not provision (it provisions the organism's own roles, {}, and knows the heart roles {})", ROLES.join(", "), HEART_ROLES.join(", ")));
    }
    for must in ALWAYS {
        if !needs.iter().any(|r| r == must) {
            return Err(format!("[dna] needs leaves out `{must}`, which every organism needs: its record is memory's and its organization reads the nerves"));
        }
    }
    needs.sort_by_key(|r| ROLES.iter().chain(PROVISIONED_HEART_ROLES.iter()).position(|x| x == r));
    needs.dedup();
    Ok(needs)
}

/// The application-plane parts the application brings itself (`[dna] brings`,
/// a subset of its needs: its compose provides them, so by default the
/// habitat provisions none, and `graft --reuse <part>` has it provide one).
pub(super) fn brings_of(manifest_text: Option<&str>) -> Result<Vec<String>, String> {
    let Some(text) = manifest_text else { return Ok(Vec::new()) };
    let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
    let brings: Vec<String> = match table.get("dna").and_then(|d| d.get("brings")) {
        None => return Ok(Vec::new()),
        Some(toml::Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("[dna] brings holds {v}, not a part's name"))).collect::<Result<Vec<_>, _>>()?,
        Some(other) => return Err(format!("[dna] brings is {other}, not a list of parts")),
    };
    let needs = needs_of(Some(text))?;
    if let Some(b) = brings.iter().find(|b| !APPLICATION_PARTS.contains(&b.as_str()) || !needs.iter().any(|n| n == *b)) {
        return Err(format!("[dna] brings `{b}`: what an application brings is one of its needs among {}", APPLICATION_PARTS.join(", ")));
    }
    Ok(brings)
}

fn graft(args: &[String]) -> ExitCode {
    let mut dir: Option<PathBuf> = None;
    let mut attach = false;
    let mut library = true;
    let mut pack = true;
    let mut with: Option<String> = None;
    let mut who: Option<String> = None;
    let mut sets = Vec::new();
    let mut profile: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut body: Option<String> = None;
    let mut on_host: Option<String> = None;
    let mut no_head = false;
    let mut parts: BTreeMap<String, Part> = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--host" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `--host` takes a host's name\n{}", usage_text());
                    return ExitCode::from(2);
                };
                on_host = Some(v.clone());
                i += 1;
            }
            "--no-head" => {
                no_head = true;
                PROGRESS_TO_LOG.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            "--plan" => GRAFT_PLAN_ONLY.store(true, std::sync::atomic::Ordering::Relaxed),
            "--own" | "--reuse" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `{a}` takes a part (memory, nerves, senses, database, broker)\n{}", usage_text());
                    return ExitCode::from(2);
                };
                if let Err(e) = parse_part(a, v, &mut parts) {
                    eprintln!("hale habitat graft: {e}");
                    return ExitCode::from(2);
                }
                i += 1;
            }
            "--forge" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `--forge` takes a provider (forgejo, github, file, none)\n{}", usage_text());
                    return ExitCode::from(2);
                };
                let _ = GRAFT_FORGE.set(v.clone());
                i += 1;
            }
            "--attach" => attach = true,
            "--no-library" => library = false,
            "--no-pack" => pack = false,
            "--profile" | "--remote" | "--body" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `{a}` takes a value\n{}", usage_text());
                    return ExitCode::from(2);
                };
                match a {
                    "--profile" => profile = Some(v.clone()),
                    "--remote" => remote = Some(v.clone()),
                    _ => body = Some(v.clone()),
                }
                i += 1;
            }
            "--with" | "--set" | "--as" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `{a}` takes a value\n{}", usage_text());
                    return ExitCode::from(2);
                };
                match a {
                    "--with" => with = Some(v.clone()),
                    "--set" => sets.push(v.clone()),
                    _ => who = Some(v.clone()),
                }
                i += 1;
            }
            f if f.starts_with("--") => {
                eprintln!("hale habitat graft: `{f}` is not a flag of `graft`\n{}", usage_text());
                return ExitCode::from(2);
            }
            d => {
                if dir.replace(PathBuf::from(d)).is_some() {
                    eprintln!("hale habitat graft: one directory\n{}", usage_text());
                    return ExitCode::from(2);
                }
            }
        }
        i += 1;
    }
    let Some(dir) = dir else {
        eprintln!("hale habitat graft: which directory?\n{}", usage_text());
        return ExitCode::from(2);
    };
    let chose_parts = !parts.is_empty() || GRAFT_PLAN_ONLY.load(std::sync::atomic::Ordering::Relaxed);
    let _ = GRAFT_PARTS.set(parts);
    // a bare name the habitat pins as a sapling: grafted from its nursery,
    // with no checkout (its --set values go to the packs it adopts)
    // phase 4: a graft's placement: the host its organism's allocations are
    // placed on (every allocation names its provider: the host's)
    if let Some(name) = &on_host {
        match sapling::graft_host(name) {
            Ok(Some(h)) => {
                let _ = GRAFT_HOST.set(h);
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("hale habitat graft: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if !attach && with.is_none() && who.is_none() && profile.is_none() && remote.is_none() && body.is_none() && sapling::names_a_sapling(&dir) {
        // a client of the head's `Commands::graft` while the head is up (the
        // operator face's act too); the act itself with `--no-head`, or
        // when no head answers
        // the head's act takes the plan as it stands and asks no one: a graft
        // that chose its parts, or runs at a person's terminal, acts itself
        if !no_head && !chose_parts && !std::io::IsTerminal::is_terminal(&std::io::stdin()) && sets.is_empty() && pack {
            if let Some(v) = sapling::head_graft(&dir.to_string_lossy(), on_host.as_deref().unwrap_or(""), library) {
                if v["ok"] == serde_json::json!(true) {
                    print!("{}", v["said"].as_str().unwrap_or(""));
                    return ExitCode::SUCCESS;
                }
                eprint!("{}", v["said"].as_str().unwrap_or(""));
                eprintln!("hale habitat graft: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
                return ExitCode::from(1);
            }
        }
        return match sapling::graft_sapling(&dir.to_string_lossy(), library, pack, &sets) {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hale habitat graft: {e}");
                ExitCode::from(1)
            }
        };
    }
    if with.is_none() && (!sets.is_empty() || who.is_some()) {
        eprintln!("hale habitat graft: `--set` and `--as` go with a pack; name the pack with `--with <pack>`");
        return ExitCode::from(2);
    }
    if attach && (profile.is_some() || remote.is_some() || body.is_some()) {
        eprintln!("hale habitat graft: `--profile`, `--remote` and `--body` shape a new application; an attached one keeps its own (`hale dna profile` detects it)");
        return ExitCode::from(2);
    }
    let shape = Shape { profile: profile.as_deref(), remote: remote.as_deref(), body: body.as_deref() };
    match graft_lines(&dir, attach, library, pack, &shape, None) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            eprintln!("hale habitat graft: {e}");
            return ExitCode::from(1);
        }
    }
    // the pack planned on the holes the birth proposed (a process of its
    // own: the verb execs the host); its refusal is the graft's exit
    let Some(pack) = with else { return ExitCode::SUCCESS };
    let mut pack_args = vec!["dna".to_string(), "pack".to_string(), "apply".to_string(), pack, "--on-holes".to_string()];
    for v in sets {
        pack_args.push("--set".to_string());
        pack_args.push(v);
    }
    if let Some(who) = who {
        pack_args.push("--as".to_string());
        pack_args.push(who);
    }
    pack_args.push(dir.to_string_lossy().to_string());
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let status = std::env::current_exe().and_then(|me| Command::new(me).args(&pack_args).status());
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat graft: the `--with` pack: {e}");
            ExitCode::from(1)
        }
    }
}

/// GH #1653: an organism born as the habitat's child: its needs read from
/// its genome, its record's genesis naming the habitat's record, each role
/// it needs allocated to it, and the graft and its needs a row of the
/// habitat's record. The habitat must be born, and the environment must
/// agree with its backends.
/// A new application's profile pieces (`--profile`, `--remote`, `--body`).
struct Shape<'a> {
    profile: Option<&'a str>,
    remote: Option<&'a str>,
    body: Option<&'a str>,
}

fn graft_lines(dir: &Path, attach: bool, library: bool, pack: bool, shape: &Shape, sapling: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    let genome = genome_dir();
    let Some(parent) = manifest_pin(&genome.join("hale.toml")).ok().flatten() else {
        return Err("the habitat is not born: `hale habitat up` births it, and every other organism is grafted from it".to_string());
    };
    if !super::record_exists(&genome).unwrap_or(false) {
        return Err("the habitat's record cannot be read (its memory is not up): `hale habitat up` brings it up".to_string());
    }
    let backends = backends_of(None)?;
    if attach && !dir.is_dir() {
        return Err(format!("{} is not there to attach (graft without --attach births a new application there)", dir.display()));
    }
    if !attach && std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!("{} exists and is not empty: `--attach` grafts the application or repository that is there", dir.display()));
    }
    // the organism's root, as its birth resolves it: an attached
    // application's workspace (its manifest, its pin and its needs are the
    // workspace's), a repository's own directory, or the new application
    let root = if attach { super::organism_root(dir)? } else { dir.to_path_buf() };
    let manifest = std::fs::read_to_string(root.join("hale.toml")).ok();
    // a genome that pins a record already is this habitat's child whose
    // graft stopped before its end, or ended: the graft goes on where it
    // stopped, each step keeping what is done. A record born of anything
    // else is never grafted, and one memory does not hold is refused by
    // the birth itself, naming it
    let pinned = manifest.as_deref().and_then(manifest_pin_in_text);
    if let Some(id) = &pinned {
        let short = &id[..12.min(id.len())];
        // memory decides, never a guess: the genesis is read, after the
        // local setup the birth would make first when it cannot be
        let mut said = pinned_genesis(&root);
        if let PinnedGenesis::Unread(_) = said {
            let _ = super::memory_migrate(&root);
            said = pinned_genesis(&root);
        }
        match said {
            PinnedGenesis::Parent(of) if of == parent => {}
            PinnedGenesis::Parent(of) => {
                let whose = if of.is_empty() { "no parent".to_string() } else { format!("the record {}…", &of[..12.min(of.len())]) };
                return Err(format!("{} pins the record {short}…, born of {whose}, not of this habitat: an organism is grafted once, at its birth", root.display()));
            }
            // memory holds no such record: the birth refuses the pin,
            // naming it
            PinnedGenesis::Absent => {}
            PinnedGenesis::Unread(why) => return Err(format!("{} pins the record {short}…, whose genesis could not be read, so its parent is not known: {why}", root.display())),
        }
    }
    let needs = needs_of(manifest.as_deref())?;
    let _ = GRAFT_BRINGS.set(brings_of(manifest.as_deref())?);
    // the plan, part by part, before anything is made: `--plan` prints it
    // and stops; at a terminal the person confirms it (a graft the head
    // started, with no terminal, takes it as planned); a graft that goes on
    // where it stopped was planned when it began
    if pinned.is_none() {
        let has_app = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"].iter().any(|f| root.join(f).is_file());
        let plan = graft_plan(&needs, has_app);
        let what = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let on = GRAFT_HOST.get().and_then(|h| h["host"].as_str()).map(|h| format!(" onto {h}")).unwrap_or_default();
        if GRAFT_PLAN_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
            println!("graft {what}{on}\n");
            for l in &plan {
                println!("{l}");
            }
            std::process::exit(0);
        }
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            use std::io::{BufRead, Write};
            println!("graft {what}{on}\n");
            for l in &plan {
                println!("{l}");
            }
            print!("\ngraft with these? [y/N] ");
            let _ = std::io::stdout().flush();
            let mut said = String::new();
            let _ = std::io::stdin().lock().read_line(&mut said);
            if !matches!(said.trim(), "y" | "Y" | "yes") {
                eprintln!("nothing grafted (`--own <part>[=<host>]` and `--reuse <part>` change the plan; `--plan` prints it)");
                std::process::exit(1);
            }
        }
    }
    let mut out = vec![format!("{:<8}{} as the habitat's child (parent {}…; needs {}){}", "graft", root.display(), &parent[..12], needs.join(", "), if pinned.is_some() { "; the graft goes on where it stopped" } else { "" })];
    out.extend(if attach { super::init_under(dir, library, super::Lineage::Child(&parent))? } else { super::new_project_under(dir, shape.profile, shape.remote, shape.body, library, &parent)? });
    let root = if attach { root } else { project(dir)?.0 };
    let child = manifest_pin(&root.join("hale.toml")).ok().flatten().ok_or("the child pins no record after its birth")?;
    // birth allocated memory and the nerves; the rest of what it needs here
    if needs.iter().any(|r| r == "senses") && backends == "own" {
        match host_run("senses-up", &root, &[]) {
            Ok(o) if !o.starts_with("none: ") => {
                record_allocation(&root, "senses", 0, "", false)?;
                out.push(format!("senses  allocated ({})", o.trim().trim_start_matches("HALE_DNA_SENSES_URL=")));
            }
            Ok(o) => out.push(format!("{:<8}senses not up: {}", "note", o.trim().strip_prefix("none: ").unwrap_or(o.trim()))),
            Err(e) => out.push(format!("{:<8}senses not up: {e}", "note")),
        }
    }
    // the application-plane parts the graft has the habitat provide; one
    // the application owns is its compose's, and said so
    for role in APPLICATION_PARTS {
        if needs.iter().any(|r| r == role) && part_choice(role) != Part::Reuse {
            out.push(format!("{role:<8} the application's own, in its compose (the habitat provisions none: `--reuse {role}` has it provide one)"));
        }
    }
    if needs.iter().any(|r| r == "database") && part_choice("database") == Part::Reuse {
        out.push(provision_database(&root, &backends)?);
    }
    if needs.iter().any(|r| r == "broker") && part_choice("broker") == Part::Reuse {
        if backends == "operator" {
            return Err("the `broker` heart role is a user of the organism's own nerves server, which the habitat configures; a habitat on the operator's backends does not configure the operator's NATS".to_string());
        }
        out.push(provision_broker(&root)?);
    }
    // the heart's own migrations, with its roles in place
    out.extend(run_heart_migrations(&root)?);
    // the attach handover: the service client the heart signs in as,
    // mapped in the child's record and its secret drawn, so nothing of it
    // is written by hand
    out.extend(map_service_client(&root)?);
    // the graft is complete when the habitat's record holds its row: one
    // row, however many times the graft goes on
    if rows_of(&genome).iter().any(|r| r["kind"] == "habitat.grafted" && r["entity"] == &child[..12]) {
        out.push(format!("{:<8}the graft of {}… (the habitat's record holds it)", "kept", &child[..12]));
    } else {
        // the organism's address: its head block, taken beside every block
        // the habitat has handed out, and kept in its own `dna.head.url`
        let mut taken = granted_bases(&genome);
        taken.extend(head_url_at(&genome).as_deref().and_then(loopback_port));
        let base = head_block(&root, &taken)?;
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mut body = serde_json::json!({ "child": child, "parent": parent, "needs": needs, "backends": backends, "name": name, "face": format!("http://{}:{base}", habitat_address()), "head": format!("http://{}:{}", habitat_address(), base + 1) });
        // its declared purpose (dna/org/purpose.hl), for the habitat's index
        body["purpose"] = serde_json::json!(declared_purpose(&root));
        // a sapling's graft says which sapling, revision and digest it is
        if let Some(l) = sapling {
            for k in ["sapling", "revision", "digest", "nursery"] {
                body[k] = l[k].clone();
            }
        }
        let row = serde_json::json!({ "kind": "habitat.grafted", "entity": &child[..12], "body": body.to_string() });
        let file = PathBuf::from(local_habitat_dir()).join(".graft-row.jsonl");
        std::fs::write(&file, format!("{row}\n")).map_err(|e| format!("write {}: {e}", file.display()))?;
        let landed = host_run("record-seed", &genome, &[file.display().to_string(), String::new()]);
        let _ = std::fs::remove_file(&file);
        landed?;
        out.push(format!("recorded the graft of {}… in the habitat's record", &child[..12]));
    }
    // its forge and its identity, roles the habitat allocates as it
    // allocates its memory: the forge the policy (or `--forge`) names, the
    // identity the habitat's own sign-in at the organism's issuer; outside
    // the graft row's branch, so a retry finishes what a failure left
    out.extend(sapling::allocate_roles(&child, &root)?);
    // its codebases, as its manifest lists them: attached in its record
    // (`codebase.attached`, phase 3's attach), nothing cloned until a leg
    // works one; each once, however many times the graft goes on
    out.extend(attach_codebases(&root)?);
    // the standard practices, the law every organism is born under, for
    // the child's Board (a process of its own: the verb execs the host),
    // proposed once
    if !pack {
        out.push(format!("{:<8}the `standard` pack (--no-pack: `hale dna pack apply standard` proposes it)", "skipped"));
        return Ok(out);
    }
    let proposed = |r: &serde_json::Value| {
        r["kind"] == "batch.proposed" && r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).is_some_and(|b| b["pack"].as_str().is_some_and(|p| p.starts_with("standard@")))
    };
    if pinned.is_some() && rows_of(&root).iter().any(proposed) {
        out.push(format!("{:<8}the `standard` pack (proposed to the child's Board already)", "kept"));
        return Ok(out);
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let ok = Command::new(&me).args(["dna", "pack", "apply", "standard"]).arg(&root).status().is_ok_and(|s| s.success());
    out.push(if ok { "proposed the `standard` pack to the child's Board".to_string() } else { format!("{:<8}the `standard` pack was not proposed: `hale dna pack apply standard {}` proposes it", "note", root.display()) });
    Ok(out)
}

/// The purpose the organism at `root` declares (`PURPOSE` in its
/// `dna/org/purpose.hl`), "" when it declares none.
fn declared_purpose(root: &Path) -> String {
    let text = std::fs::read_to_string(root.join("dna/org/purpose.hl")).unwrap_or_default();
    let Some(at) = text.find("const PURPOSE: String = \"") else { return String::new() };
    let rest = &text[at + "const PURPOSE: String = \"".len()..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// What memory says of the record the organism at `root` pins (GH #1653):
/// the parent its genesis names ("" for none), that it holds no such
/// record, or why it could not be read.
enum PinnedGenesis {
    Parent(String),
    Absent,
    Unread(String),
}

fn pinned_genesis(root: &Path) -> PinnedGenesis {
    // the clone's kept head DSN, else one derived from memory that is
    // there (nothing brought up, nothing written)
    let head = super::kept_head_dsn(root).ok().flatten().or_else(|| {
        host_run("memory-migrate", root, &["--derive".to_string()]).ok().and_then(|o| o.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_HEAD=")).map(str::to_string))
    });
    let ask = |verb: &str| -> Result<String, String> {
        let (mut cmd, _) = super::host_command(verb, root)?;
        if let Some(h) = &head {
            cmd.env("HALE_DNA_MEMORY_DSN_HEAD", h);
        }
        let out = cmd.output().map_err(|e| format!("hale dna {verb}: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() { format!("hale dna {verb} failed") } else { err });
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    };
    match ask("record-head") {
        Err(why) => return PinnedGenesis::Unread(why),
        Ok(h) if h.trim() == "none" => return PinnedGenesis::Absent,
        Ok(_) => {}
    }
    let rows = match ask("record-rows") {
        Ok(r) => r,
        Err(why) => return PinnedGenesis::Unread(why),
    };
    let genesis = rows.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).find(|r| r["kind"] == "record.genesis");
    match genesis {
        Some(g) => {
            let body: serde_json::Value = g["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_default();
            PinnedGenesis::Parent(body["parent"].as_str().unwrap_or("").to_string())
        }
        None => PinnedGenesis::Unread("the record has no genesis row".to_string()),
    }
}

/// The rows of the record of the organism at `root`, none when it cannot
/// be read.
fn rows_of(root: &Path) -> Vec<serde_json::Value> {
    host_run("record-rows", root, &[]).map(|rows| rows.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// A codebase a genome's manifest lists (`[[dna.codebase]]` in its
/// hale.toml): the fields phase 3's `hale dna codebase attach` takes.
#[derive(Debug, PartialEq)]
struct ManifestCodebase {
    name: String,
    repo: String,
    branch: String,
    build: String,
    test: String,
    toolchain: String,
}

/// The codebases the manifest text lists, in its order.
fn manifest_codebases(text: &str) -> Result<Vec<ManifestCodebase>, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
    let Some(list) = table.get("dna").and_then(|d| d.get("codebase")) else { return Ok(Vec::new()) };
    let list = list.as_array().ok_or("[dna] codebase is not a list of tables (`[[dna.codebase]]`)")?;
    list.iter()
        .map(|c| {
            let field = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cb = ManifestCodebase { name: field("name"), repo: field("repo"), branch: field("branch"), build: field("build"), test: field("test"), toolchain: field("toolchain") };
            if cb.name.is_empty() || cb.repo.is_empty() {
                return Err(format!("a [[dna.codebase]] names its `name` and its `repo` ({c})"));
            }
            Ok(cb)
        })
        .collect()
}

/// The organism at `root` given the codebases its manifest lists: each one
/// its record does not hold yet attached by the host's `codebase attach`
/// (a `codebase.attached` row in its record, as the habitat's act). What
/// was attached, a line each.
fn attach_codebases(root: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(root.join("hale.toml")).unwrap_or_default();
    let listed = manifest_codebases(&text)?;
    if listed.is_empty() {
        return Ok(Vec::new());
    }
    let held: Vec<String> = rows_of(root).iter().filter(|r| r["kind"] == "codebase.attached").filter_map(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).and_then(|b| b["name"].as_str().map(str::to_string))).collect();
    let mut out = Vec::new();
    for c in listed {
        if held.contains(&c.name) {
            out.push(format!("{:<8}the codebase `{}` (its record holds it)", "kept", c.name));
            continue;
        }
        let mut args = vec!["attach".to_string(), c.name.clone(), "--repo".into(), c.repo.clone(), "--build".into(), c.build.clone(), "--test".into(), c.test.clone(), "--as".into(), "habitat".into()];
        if !c.branch.is_empty() {
            args.extend(["--branch".to_string(), c.branch.clone()]);
        }
        if !c.toolchain.is_empty() {
            args.extend(["--toolchain".to_string(), c.toolchain.clone()]);
        }
        host_run("codebase", root, &args).map_err(|e| format!("the codebase `{}` was not attached: {e}", c.name))?;
        out.push(format!("attached the codebase `{}` ({}, {}), as its manifest lists it", c.name, c.repo, if c.branch.is_empty() { "main" } else { &c.branch }));
    }
    Ok(out)
}

/// GH #1653: the `database` heart role provisioned for the organism at
/// `root`, a tenant database of the memory server its record is on (its
/// own, or the habitat's when that keeps the record), and recorded as its
/// allocation; the heart is handed `HALE_DNA_DATABASE_DSN`, which names
/// its vault slot.
pub(super) fn provision_database(root: &Path, backends: &str) -> Result<String, String> {
    let o = host_run("database-provision", root, &[])?;
    if let Some(why) = o.trim().strip_prefix("none: ") {
        return Err(format!("database: {why}"));
    }
    let slots = o.lines().find_map(|l| l.strip_prefix("allocation.vault_slots=")).unwrap_or("");
    record_allocation(root, "database", 0, slots, backends == "operator")?;
    let whose = if memory_root(root) == root { "the organism's" } else { "the habitat's" };
    Ok(format!("database a tenant database of {whose} memory server, for its heart (HALE_DNA_DATABASE_DSN, its password in the vault slot {slots})"))
}

/// GH #1653: the `broker` heart role, recorded as the organism's
/// allocation and then provisioned from that row: its two users, their
/// subject space and their passwords, written into its nerves server's
/// configuration with its other secrets.
fn provision_broker(root: &Path) -> Result<String, String> {
    // the names the broker is handed by, before its row is written: the
    // role is named, as the habitat has not recorded it yet
    let named = handover_with(root, true, "broker")?;
    let name = |k: &str| named.iter().find_map(|(n, v)| (n == k).then(|| v.clone())).unwrap_or_default();
    let vault = name("HALE_DNA_BROKER_VAULT");
    let node_vault = name("HALE_DNA_BROKER_NODE_VAULT");
    if vault.is_empty() || node_vault.is_empty() {
        return Err("the broker's vault slots are not named by the handover".to_string());
    }
    // the habitat's decision is its row: every reader of the organism's
    // heart roles (its secrets, its nerves server's users, its handover)
    // reads the row, never the manifest again
    record_allocation(root, "broker", 0, &format!("{vault} {node_vault}"), false)?;
    // its two users and their passwords, drawn with the organism's other
    // secrets now the allocation names the role; the server rereads them
    let drawn = host_run("secrets-provision", root, &[])?;
    if let Some(why) = drawn.trim().strip_prefix("none: ") {
        return Err(format!("the broker's users were not provisioned: {why}"));
    }
    let lines = handover(root, false)?;
    let get = |k: &str| lines.iter().find_map(|(n, v)| (n == k).then(|| v.clone())).unwrap_or_default();
    Ok(format!("{:<8}{} and its nodes' {} on the organism's nerves server, confined to {} (HALE_DNA_BROKER_*, their passwords in the vault slots {vault} and {node_vault})", "broker", get("HALE_DNA_BROKER_USER"), get("HALE_DNA_BROKER_NODE_USER"), get("HALE_DNA_BROKER_SUBJECTS")))
}

/// The handover the habitat gives the heart of the organism at `root`,
/// `(name, value)`; `derive` provisions nothing.
pub(super) fn handover(root: &Path, derive: bool) -> Result<Vec<(String, String)>, String> {
    handover_with(root, derive, "")
}

/// [`handover`], with heart roles named beside the ones the habitat
/// allocated (`with`, words): a graft names the role it is provisioning
/// before the role's row is written.
fn handover_with(root: &Path, derive: bool, with: &str) -> Result<Vec<(String, String)>, String> {
    let mut args: Vec<String> = if derive { vec!["--derive".to_string()] } else { Vec::new() };
    if !with.is_empty() {
        args.push("--with".to_string());
        args.push(with.to_string());
    }
    let o = host_run("heart-handover", root, &args)?;
    if let Some(why) = o.trim().strip_prefix("none: ") {
        return Err(why.to_string());
    }
    Ok(o.lines().filter(|l| l.starts_with("HALE_DNA_")).filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect())
}

/// The heart's service client mapped in the child's record (`git config
/// --local --add dna.oidc.service <client>=<client>`, once) and its secret
/// drawn into the vault slot the handover names.
fn map_service_client(root: &Path) -> Result<Vec<String>, String> {
    let lines = handover(root, true).unwrap_or_default();
    let Some(client) = lines.iter().find_map(|(k, v)| (k == "HALE_DNA_OIDC_CLIENT").then(|| v.clone())) else { return Ok(Vec::new()) };
    let mapping = format!("{client}={client}");
    let have = Command::new("git").arg("-C").arg(root).args(["config", "--local", "--get-all", "dna.oidc.service"]).output().map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == mapping)).unwrap_or(false);
    if !have {
        let ok = Command::new("git").arg("-C").arg(root).args(["config", "--local", "--add", "dna.oidc.service", &mapping]).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("could not map the service client `{client}` in {}", root.display()));
        }
    }
    // a habitat that shares its identity learns the client: its provider
    // reads the list at each request (dna/oidc, HALE_DNA_OIDC_SERVICES_FILE)
    if shared_issuer().is_some() {
        let list = PathBuf::from(local_habitat_dir()).join("identity.services");
        let known = std::fs::read_to_string(&list).unwrap_or_default();
        if !known.split_whitespace().any(|c| c == client) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&list).map_err(|e| format!("{}: {e}", list.display()))?;
            writeln!(f, "{client}").map_err(|e| format!("{}: {e}", list.display()))?;
        }
    }
    let drawn = super::organism_secrets(root, false);
    Ok(vec![format!("{:<8}the heart's service client `{client}` (dna.oidc.service {mapping}; its secret the vault slot oidc-service-{client})", "oidc"), drawn].into_iter().filter(|l| !l.is_empty()).collect())
}

/// The roles the organism at `root` needs, as its manifest declares them.
pub(super) fn needs_at(root: &Path) -> Vec<String> {
    needs_of(std::fs::read_to_string(root.join("hale.toml")).ok().as_deref()).unwrap_or_else(|_| ROLES.iter().map(|r| r.to_string()).collect())
}

fn manifest_pin_in_text(text: &str) -> Option<String> {
    let table: toml::Table = text.parse().ok()?;
    table.get("dna").and_then(|d| d.get("record")).and_then(|v| v.as_str()).map(str::to_string)
}


#[cfg(test)]
mod tests {
    use super::building;

    #[test]
    fn a_face_still_building_says_which_seed() {
        let log = "hale: building the head: dna/api @ /h/.cache/hale/iris/ab12\nhale: building the head: dna/reflexes @ /h/.cache/hale/iris/ab12\n";
        assert_eq!(building(log).as_deref(), Some("dna/reflexes"));
        assert_eq!(building("hale: building the head: dna/api @ /x\nhead: serving on 127.0.0.1:9313\n"), None);
        assert_eq!(building(""), None);
    }

    use super::*;

    /// A host's listen address is its name's first IPv4 address, through
    /// the toolchain's own resolver; a name that resolves to none says so.
    /// The two rules over one resolution: a project's listen address is
    /// IPv4 only; a remote habitat's listener prefers IPv4 and falls back
    /// to the first address.
    /// A graft's plan, part by part: the organism plane reuses the
    /// habitat's by default, an application-plane part is the application's
    /// own only when it brings it, and each flag flips one part, placed.
    #[test]
    fn a_graft_plans_each_part_and_each_flag_flips_one() {
        let mut m = BTreeMap::new();
        parse_part("--own", "memory=box-1", &mut m).unwrap();
        parse_part("--reuse", "database", &mut m).unwrap();
        assert_eq!(m.get("memory"), Some(&Part::Own(Some("box-1".into()))));
        assert_eq!(m.get("database"), Some(&Part::Reuse));
        assert!(parse_part("--own", "memory", &mut m).unwrap_err().contains("chosen twice"));
        assert!(parse_part("--reuse", "nerves=box-1", &mut BTreeMap::new()).is_err(), "a reused part runs where the habitat's does");
        assert!(parse_part("--own", "database=box-1", &mut BTreeMap::new()).unwrap_err().contains("not built yet"));
        assert!(parse_part("--own", "redis", &mut BTreeMap::new()).is_err());
        let brings = vec!["database".to_string(), "broker".to_string()];
        assert_eq!(default_part("database", &brings), Part::Own(None));
        assert_eq!(default_part("database", &[]), Part::Reuse, "an application that brings none is provided one");
        assert_eq!(default_part("memory", &brings), Part::Reuse);
        let needs: Vec<String> = ["memory", "nerves", "database", "broker"].iter().map(|s| s.to_string()).collect();
        let lines = plan_lines(&needs, true, "rack-a.lan", "box-1", &brings, Some("http://rack-a.lan:8803"), &|r: &str| m.get(r).cloned().unwrap_or_else(|| default_part(r, &brings)));
        let text = lines.join("\n");
        assert!(text.contains(&plan_row("memory", "own", "a server of its own on box-1", "")), "{text}");
        assert!(text.contains(&plan_row("nerves", "reuse", "an account of its own on the habitat's NATS (rack-a.lan)", "--own nerves=<host>: a server of its own")), "{text}");
        let defaults = plan_lines(&needs, true, "rack-a.lan", "box-1", &brings, None, &|r: &str| default_part(r, &brings)).join("\n");
        assert!(defaults.contains(&plan_row("memory", "reuse", "a schema of its own on the habitat's Postgres (rack-a.lan)", "--own memory=<host>: a server of its own")), "{defaults}");
        assert!(defaults.contains(&plan_row("identity", "own", "its own sign-in stub", "")[..]), "no published identity: {defaults}");
        assert!(text.contains("services  its compose services on box-1"), "{text}");
        assert!(text.contains(&plan_row("database", "reuse", "provisioned by the habitat", "")), "{text}");
        assert!(text.contains(&plan_row("broker", "own", "the application's own NATS, in its compose", "")), "{text}");
        assert!(!text.contains("senses"), "a part it does not need is not planned: {text}");
        assert!(text.contains(&plan_row("identity", "reuse", "the habitat's sign-in (http://rack-a.lan:8803)", "")), "{text}");
    }

    /// What an application brings is among its application-plane needs.
    #[test]
    fn an_application_brings_only_what_it_needs() {
        let ok = "[dna]\nneeds = [\"memory\", \"nerves\", \"database\", \"broker\"]\nbrings = [\"database\", \"broker\"]\n";
        assert_eq!(brings_of(Some(ok)).unwrap(), ["database", "broker"]);
        assert_eq!(brings_of(Some("[dna]\nneeds = [\"memory\", \"nerves\"]\n")).unwrap(), Vec::<String>::new());
        assert!(brings_of(Some("[dna]\nneeds = [\"memory\", \"nerves\"]\nbrings = [\"database\"]\n")).is_err(), "not a need");
        assert!(brings_of(Some("[dna]\nneeds = [\"memory\", \"nerves\"]\nbrings = [\"memory\"]\n")).is_err(), "not an application-plane part");
    }

    #[test]
    fn the_listen_rules_read_one_resolution_two_ways() {
        let v6: std::net::SocketAddr = "[2001:db8::1]:0".parse().unwrap();
        let v4: std::net::SocketAddr = "192.0.2.7:0".parse().unwrap();
        assert_eq!(ipv4_of("h", &[v6, v4]).as_deref(), Ok("192.0.2.7"));
        assert_eq!(ipv4_or_first("h", &[v6, v4]).as_deref(), Ok("192.0.2.7"));
        assert!(ipv4_of("h", &[v6]).is_err());
        assert_eq!(ipv4_or_first("h", &[v6]).as_deref(), Ok("2001:db8::1"));
        assert!(ipv4_of("h", &[]).is_err());
        assert!(ipv4_or_first("h", &[]).is_err());
    }

    #[test]
    fn a_hosts_listen_address_is_its_first_ipv4() {
        assert_eq!(first_ipv4("127.0.0.1").as_deref(), Ok("127.0.0.1"));
        assert_eq!(first_ipv4("localhost").as_deref(), Ok("127.0.0.1"));
        assert!(first_ipv4("no-such-host.invalid").is_err());
        assert!(first_ipv4("::1").is_err());
    }

    /// A head block never covers the senses' fixed scrape ports.
    #[test]
    fn a_head_block_is_never_placed_over_the_senses_ports() {
        assert!(overlaps_senses(9464) && overlaps_senses(9459) && overlaps_senses(9466));
        assert!(!overlaps_senses(9456) && !overlaps_senses(9467) && !overlaps_senses(8800));
    }

    /// An organism removed and another grafted under its name: the name
    /// is the live one's, never ambiguous.
    #[test]
    fn a_removed_organisms_name_is_the_live_ones() {
        let row = |e: &str, body: serde_json::Value| serde_json::json!({ "kind": "habitat.grafted", "entity": e, "body": body.to_string() });
        let rows = vec![
            row("aaaaaaaaaaaa", serde_json::json!({ "name": "shipme" })),
            row("aaaaaaaaaaaa", serde_json::json!({ "name": "shipme", "state": "removed" })),
            row("bbbbbbbbbbbb", serde_json::json!({ "name": "shipme" })),
            serde_json::json!({ "kind": "habitat.allocated", "entity": "cccccccccccc/memory", "body": "{}" }),
        ];
        assert_eq!(live_grafts_named(&rows, "shipme"), vec!["bbbbbbbbbbbb".to_string()]);
        assert!(live_grafts_named(&rows, "aaaaaaaaaaaa").is_empty());
        assert_eq!(live_grafts_named(&rows[..1], "shipme"), vec!["aaaaaaaaaaaa".to_string()]);
    }

    /// D8: a genome's manifest lists its codebases with phase 3's attach
    /// fields; one without a name or a repository is refused.
    #[test]
    fn a_manifest_lists_its_codebases() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dna/nursery/saplings/hale/hale.toml")).unwrap();
        let listed = manifest_codebases(&text).unwrap();
        let names: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["hale", "voice"]);
        assert!(listed.iter().all(|c| !c.repo.is_empty() && !c.build.is_empty() && !c.test.is_empty() && !c.toolchain.is_empty()), "{listed:?}");
        // voice's seeds, built and tested as its CI does
        assert_eq!(listed[1].build, "hale build api && hale build brain && hale build node");
        assert_eq!(listed[1].test, "hale test api && hale test store && hale test brain && hale test node");
        assert!(manifest_codebases("[dna]\nneeds = [\"memory\", \"nerves\"]\n").unwrap().is_empty());
        assert!(manifest_codebases("[[dna.codebase]]\nname = \"x\"\n").is_err());
    }

    /// D8: a habitat's name is a DNS label.
    #[test]
    fn a_habitat_name_is_a_dns_label() {
        for ok in ["local", "homelab-test", "h", "a1-b2", &"x".repeat(63)] {
            assert!(habitat_name_ok(ok), "{ok}");
        }
        for bad in ["", "Homelab", "home_lab", "-lab", "lab-", "home.lab", "home lab", &"x".repeat(64)] {
            assert!(!habitat_name_ok(bad), "{bad}");
        }
    }

    /// GH #1653: a heart migration is declared for a heart role the
    /// genome needs, as its `command` alone.
    #[test]
    fn heart_migrations_are_declared_per_role() {
        let d = root("migrations");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = [\"api\", \"migrate\"]\n").unwrap();
        let m = heart_migrations(&d).unwrap();
        assert_eq!((m.len(), m[0].role.as_str(), m[0].command.clone()), (1, "database", vec!["api".to_string(), "migrate".to_string()]));
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\"]\n\n[dna.migrations.database]\ncommand = [\"api\"]\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("`database` is not one"), "only for a role the genome needs");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = []\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("needs `command`"), "a command");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = [\"api\"]\nschema_version = 5\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("`schema_version` is not a key"), "the habitat keeps no version of the heart's schema");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n").unwrap();
        assert!(heart_migrations(&d).unwrap().is_empty(), "a heart that declares none has nothing run");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// GH #1653: a heart role's allocation is recorded once and carries no
    /// version; the organism's own roles keep theirs.
    #[test]
    fn a_heart_role_carries_no_version() {
        let mut row = serde_json::json!({});
        let id = "0123456789abcdef0123456789abcdef";
        next_rows_as(&mut row, id, "database", 0, "postgres-heart_x", "p", false, None);
        next_rows_as(&mut row, id, "database", 0, "postgres-heart_x", "p", false, None);
        let pending = row["pending"].as_array().unwrap();
        assert_eq!(pending.len(), 1, "allocated once, and no migration row of the habitat's: {pending:?}");
        assert_eq!(pending[0]["kind"], "habitat.allocated");
        assert!(row["roles"]["database"]["schema_version"].is_null());
        next_rows_as(&mut row, id, "memory", 3, "", "p", false, None);
        assert_eq!(row["roles"]["memory"]["schema_version"], 3);
    }

    /// GH #1653: needs by role, the first half of graft.
    #[test]
    fn needs_are_declared_by_role() {
        // undeclared, foreign or not: exactly the organism's own roles, no
        // heart role
        for undeclared in [None, Some("[deps]\n")] {
            let got = needs_of(undeclared).unwrap();
            assert_eq!(got, ROLES, "an undeclared genome gets the organism's own roles");
            assert!(got.iter().all(|r| !HEART_ROLES.contains(&r.as_str())), "and no heart role: {got:?}");
        }
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"database\", \"memory\", \"nerves\"]\n")).unwrap(), ["memory", "nerves", "database"], "a Hale heart declares the database it needs");
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"memory\", \"nerves\", \"broker\"]\n")).unwrap(), ["memory", "nerves", "broker"], "and the broker it needs");
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\", \"nerves\", \"cache\"]\n")).unwrap_err().contains("the heart role `cache`, which this habitat does not provision yet"), "the other heart roles are refused by name");
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"nerves\", \"memory\"]\n")).unwrap(), ["memory", "nerves"], "senses left out");
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\", \"redis\"]\n")).unwrap_err().contains("`redis`, which the habitat does not provision"));
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\"]\n")).unwrap_err().contains("leaves out `nerves`"));
        assert!(needs_of(Some("[dna]\nneeds = \"memory\"\n")).unwrap_err().contains("not a list of roles"));
    }

    /// GH #1653: an allocation's rows are the organism's and the role's,
    /// never a checkout's: allocated once, migrated whenever the version
    /// moves, nothing when it does not; and a recorded version behind the
    /// toolchain's is named.
    #[test]
    fn allocation_rows_follow_the_version() {
        let id = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let mut row = serde_json::json!({ "root": "/a/checkout" });
        next_rows_as(&mut row, id, "memory", 9, "postgres-owner-1234567890ab", "hale-habitat-local-x", false, None);
        let kinds: Vec<&str> = row["pending"].as_array().unwrap().iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["habitat.allocated", "habitat.migrated"]);
        let allocated = row["pending"][0]["body"].as_str().unwrap();
        assert!(allocated.contains("\"organism\":\"1234567890abcdef") && allocated.contains("\"vault\":[\"postgres-owner-1234567890ab\"]") && !allocated.contains("checkout"), "{allocated}");
        assert_eq!(row["pending"][0]["entity"], "1234567890ab/memory");
        // phase 4: every allocation names its provider and its host
        assert!(allocated.contains("\"provider\":\"compose\"") && allocated.contains("\"host\":\"local\""), "{allocated}");
        let mut adopted = serde_json::json!({});
        next_rows_as(&mut adopted, id, "nerves", 2, "", "p", true, None);
        let on_operator = adopted["pending"][0]["body"].as_str().unwrap();
        assert!(on_operator.contains("\"provider\":\"operator\"") && on_operator.contains("\"host\":\"\"") && on_operator.contains("\"service\":\"operator\""), "{on_operator}");
        // flushed, then migrated again at the same version: nothing owed
        row["pending"] = serde_json::json!([]);
        row["roles"]["memory"]["recorded"] = serde_json::json!(true);
        next_rows_as(&mut row, id, "memory", 9, "", "p", false, None);
        assert!(row["pending"].as_array().unwrap().is_empty());
        // the version moves: one migrated row, from the old version
        next_rows_as(&mut row, id, "memory", 10, "", "p", false, None);
        let moved = &row["pending"][0];
        assert_eq!(moved["kind"], "habitat.migrated");
        assert!(moved["body"].as_str().unwrap().contains("\"from\":9") && moved["body"].as_str().unwrap().contains("\"schema_version\":10"));
        // behind: the recorded version against the toolchain's
        row["roles"]["nerves"] = serde_json::json!({ "schema_version": 1 });
        assert_eq!(behind(&row, &[("memory".into(), 11), ("nerves".into(), 1)]), vec!["memory 10 -> 11".to_string()]);
    }

    fn root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hale_habitat_sweep_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A teardown whose volume did not go keeps the project named under the
    /// root and fails; the next finds it by that name alone (its containers
    /// are gone, so no label points at the root any more), confirms it gone
    /// and only then lets the name go.
    #[test]
    fn a_volume_that_survives_is_retried_by_name() {
        let d = root("retry");
        let p = habitat_project_at(&format!("{}/data/hale/habitat/local", d.display()));
        let listing = format!("{p}|{}/data/hale/habitat/local/allocations/abc\n", d.display());
        let mut volume_left = true;
        let mut first = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(listing.clone()),
            ["compose", ..] => Docker::Answered(String::new()),
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { format!("{p}_memory-abc\n") } else { String::new() }),
            ["network", ..] => Docker::Answered(String::new()),
            _ => Docker::Failed("unexpected".into()),
        };
        let err = down_under_with(&d, &mut first).unwrap_err();
        assert!(err.contains("still has volumes"), "{err}");
        assert_eq!(std::fs::read_to_string(d.join(COMPOSE_FILE)).unwrap(), format!("{p}\n"), "the project stays named");
        // the containers went: nothing labels the root any more, and the
        // volume is still there, so the next teardown fails again, by name
        let mut downs = Vec::new();
        let mut again = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["compose", "-p", project, ..] => {
                downs.push(project.to_string());
                Docker::Answered(String::new())
            }
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(format!("{p}_memory-abc\n")),
            ["network", ..] => Docker::Answered(String::new()),
            _ => Docker::Failed("unexpected".into()),
        };
        assert!(down_under_with(&d, &mut again).is_err());
        assert_eq!(downs, vec![p.clone()], "the recorded project is taken down again");
        assert!(d.join(COMPOSE_FILE).exists());
        // once the volume goes, the name goes
        volume_left = false;
        let mut done = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { "x\n".into() } else { String::new() }),
            _ => Docker::Answered(String::new()),
        };
        down_under_with(&d, &mut done).unwrap();
        assert!(!d.join(COMPOSE_FILE).exists(), "confirmed gone, the record goes");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A root owns what lies under it with no container left to say so:
    /// the projects an `ssh` host was handed renders for under its home
    /// there, and a habitat born in it; never a project that also runs
    /// from outside the root. Taken down whole, its network with it.
    #[test]
    fn a_root_owns_the_projects_its_directories_name() {
        let d = root("owned");
        std::fs::create_dir_all(d.join("home/.hale/habitat/hale-habitat-local-aa-localhost/abc")).unwrap();
        std::fs::create_dir_all(d.join("home/.hale/habitat/hale-habitat-local-aa-localhost-abc/sprout/conf")).unwrap();
        std::fs::create_dir_all(d.join("home/.hale/habitat/elsewhere/x")).unwrap();
        let born = format!("{}/home/.local/share/hale/habitat/local", d.display());
        std::fs::create_dir_all(format!("{born}/genome")).unwrap();
        let listed = "elsewhere|/home/someone/.local/share/hale/habitat/local/allocations/x\n";
        assert_eq!(owned_projects(&d, listed), {
            let mut v = vec!["hale-habitat-local-aa-localhost".to_string(), "hale-habitat-local-aa-localhost-abc".to_string(), habitat_project_at(&born)];
            v.sort();
            v
        });
        let mut downs = Vec::new();
        let mut network_left = true;
        let mut dock = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(listed.to_string()),
            ["compose", "-p", project, ..] => {
                downs.push(project.to_string());
                network_left = false;
                Docker::Answered(String::new())
            }
            ["network", ..] => Docker::Answered(if network_left { "n\n".into() } else { String::new() }),
            _ => Docker::Answered(String::new()),
        };
        down_under_with(&d, &mut dock).unwrap();
        assert_eq!(downs.len(), 3, "each owned project is taken down: {downs:?}");
        assert!(!downs.contains(&"elsewhere".to_string()), "never one that runs from outside the root");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The developer's habitat is never a teardown's, even named in a
    /// root's file.
    #[test]
    fn the_developers_habitat_is_never_taken_down() {
        let d = root("developer");
        let home = std::env::var("HOME").unwrap_or_default();
        let developers = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
        std::fs::write(d.join(COMPOSE_FILE), format!("{developers}\n")).unwrap();
        let mut touched = false;
        let mut dock = |args: &[&str]| {
            if args[0] == "compose" {
                touched = true;
            }
            Docker::Answered(if args[0] == "ps" && args.len() > 3 && args[2] == "--filter" && args[3] == "label=com.docker.compose.project" { format!("{developers}|{home}/.local/share/hale/habitat/local\n") } else { String::new() })
        };
        down_under_with(&d, &mut dock).unwrap();
        assert!(!touched, "nothing was taken down");
        let _ = std::fs::remove_dir_all(&d);
    }
}
