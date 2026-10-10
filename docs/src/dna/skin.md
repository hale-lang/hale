# The skin

The **skin** is the organism's trust boundary: the vault, the roles and
accounts each part holds, the identity provider in front of the head,
and the closures that name a violation. Its one job is that every part
holds exactly the credential it uses and nothing else. It is owned in
one place: one function lists the organism's secrets and one provisions
them (`dna/host/secrets.hl`); every other part only consumes a secret,
by its vault name. It fails closed: a missing secret refuses a start,
and nothing falls back to a default. It never puts a value in a
committed file, the record, a URL, an argument or an environment
variable.

## The vault

A secret lives in the vault, one entry per name, holding the secret's
exact bytes.

- **The local vault** is a directory: `HALE_VAULT_DIR`, else
  `$XDG_CACHE_HOME/hale/vault`, else `~/.cache/hale/vault`. Each entry
  is a file, mode 600. An empty entry is not held.
- **A real vault** is named by `HALE_VAULT_ADDR`, reached with the
  per-host token in `HALE_VAULT_TOKEN`. It is provisioned out of band:
  the bootstrap only checks it, and says what it lacks.

The local vault is one directory per user, not a boundary between the
parts one user runs: any of them could read any entry by name. What
keeps a part to its own credential is what it is handed. An
application gets its own account's vault name and none of the others'.

### The slots

`hale dna secrets` lists every secret the organism requires and whether
the vault holds it. It never prints a value.

```text
$ hale dna secrets
the organism's secrets (the local vault, …):
  present  postgres-dna_35bf…_spine  memory: the spine's role
  present  postgres-dna_35bf…_head  memory: the head's role
  present  postgres-owner-refproj  memory: the compose database's superuser (dna/postgres.secrets)
  present  nats-dna_35bf…-owner  the nerves: the owner's account
  present  nats-dna_35bf…-spine  the nerves: the spine's account
  present  nats-dna_35bf…-head  the nerves: the head's account
  present  nats-dna_35bf…-reflexes  the nerves: the reflexes's account
  present  nats-dna_35bf…-app-refproj  the nerves: the application `refproj`'s account
  present  oidc-client-dna-local  the skin: the head's OIDC client secret (the local stub's)
  MISSING  forge-token  the forge's token — a person supplies it: `hale dna secret set FORGE_TOKEN`
  MISSING  model-OPENAI_API_KEY  the model's key OPENAI_API_KEY — a person supplies it: `hale dna secret set OPENAI_API_KEY`
2 missing
```

Each name is one of these (`<identity>` is the record's, `<org>` its
`dna_<identity>` token, `<seed>` the compose project's name):

| vault name | kind | whose |
| --- | --- | --- |
| `postgres-dna_<identity>_spine`, `…_head` | drawn | memory's two roles |
| `postgres-owner-<seed>` | drawn | the compose database's superuser, when the seed has `dna/compose.yaml` |
| `nats-<org>-<role>`, for `owner`, `spine`, `head`, `reflexes` | drawn | the nerves' accounts |
| `nats-<org>-app-<name>` | drawn | the attached application's broker account, while the record attaches it |
| `oidc-client-<client>` (`dna-local` when `dna.oidc.client` is unset) | drawn for an issuer on the loopback, else a slot | the head's OIDC client |
| `oidc-service-<service>`, per `dna.oidc.service` | drawn | a service client |
| `forge-token` | slot | the forge's token |
| `model-<NAME>`, per credential `dna/org/models.hl` or `dna/org/work.hl` names | slot | a model's key |

**Drawn** means the organism owns the value: `hale dna init` and `hale
dna upgrade` draw it from urandom (32 hex characters) into the vault
when the vault lacks it. The nerves' passwords are drawn anew on every
`init` and `upgrade`. The servers compose runs read theirs from files
written from the same draw, `dna/nats.secrets.conf` and
`dna/postgres.secrets`: mode 600, ignored by git, and refused when git
tracks the file or would. `dna/nats.conf` is tracked and holds no
password; it includes the secrets file.

**A slot** is an empty entry a person fills. There are three kinds:
the forge's token, each model key the catalog or the legs'
`work.hl` names, and the head's
client secret when the issuer is not on this machine.

Per-member secrets are provisioned at that member's admission and are
not on this list: an owner's head role on a shared record
(`postgres-<org>_head_<owner>`, which `hale dna memory migrate` makes
under `HALE_DNA_OWNER_KEYS`; see [Memory and the record](./memory.md)).

The board shows a line for each missing secret, except a model key:
the vault that matters for a model key is the one on the machine that
calls the model, and [the body](./heart.md#a-body-on-a-server) reports
that one itself. `hale dna run` and `dev` refuse to start while a drawn
secret is missing, naming each:

```text
hale dna run: the organism's secrets are not all in the vault: … (`hale dna upgrade` draws them; `hale dna secrets` lists them); nothing starts without them
```

### Filling a slot

```sh
hale dna secret set <NAME> [--body <user@host>]
hale dna secret rotate <NAME> [--body <user@host>]
```

`<NAME>` is `FORGE_TOKEN`, `OIDC_CLIENT_SECRET`, or a credential
`dna/org/models.hl` or `dna/org/work.hl` names (`HostedCredential {
key: "…" }`; `hale dna secrets` lists a `work.hl` key as the legs'
model key); any other
name is refused, with the list. The value comes from stdin, one line,
never from the command line: `NAME=value` as an argument is refused,
because it would sit in every shell history and process list.

```text
$ hale dna secret set OPENAI_API_KEY
value for OPENAI_API_KEY (this machine), on one line:
secret set: OPENAI_API_KEY is in its slot of the vault, …/model-OPENAI_API_KEY (secret.rotated OPENAI_API_KEY; the value is nowhere in the record). A part reads it from the vault where it uses it
```

With `--body` (or `dna.body` set by `hale dna body provision`), the
value travels over ssh's stdin into that machine's vault. The record
gets one row, `secret.rotated <NAME>` with where and by whom, and never
the value. `secret rotate` is the same, and refuses a name that was
never set.

A model key and the forge's token are read from the vault alone. No
environment variable stands in for either. With no forge token, nothing
is done at the forge, and never in the name of whoever `gh` happens to
be logged in as.

The workflow catalog has a `secret-rotate` definition, but the vault is
not yet a store a workflow step may write, so it is refused at
admission:

```text
secret-rotate@1  a secret rotated
  0. rotate  vault
  1. recorded  record
  refused at admission: workflow secret-rotate@1 step 0 (rotate) writes the vault (GH #989), which is not built yet
```

## Roles and accounts

Each part holds its own credential for each private service, and the
service's grants are the boundary.

| part | memory (Postgres) | nerves (NATS) |
| --- | --- | --- |
| the spine: a body's host and its organization | `dna_<identity>_spine`: reads and writes the record's tables | `spine`: publishes and reads the organization's facts, reads the applications' events, tells the heads what landed |
| a head | `dna_<identity>_head` (or an owner's): reads, takes or fences a lease | `head`: subscribes, publishes nothing |
| the migrations | the owner's DSN, held by no running part | `owner`: creates the stream |
| the application | none | `app-<name>`: publishes on its own subjects alone |
| the reflexes | none | `reflexes`: publishes its firings alone |

A DSN names its role's vault entry (`&vault=postgres-<role>`) and a
NATS URL is `nats://host:port` alone: the driver or the client presents
the password when it connects. [Memory and the record](./memory.md)
and [The nerves](./nerves.md) have each grant in full; [The
heart](./heart.md#its-broker-account) has the application's account and
`hale dna application remove`, which revokes it.

## OIDC for the head

Every HTTP caller of a head is an identity provider's subject. Configure
the record for your provider and map the subjects you trust to member
names:

```text
git config dna.principal oidc
git config dna.oidc.issuer https://id.example.com
git config dna.oidc.client dna-head
git config dna.oidc.redirect https://dna.example.com/auth/callback
git config --add dna.oidc.member "<subject>=alice"
git config dna.oidc.board alice
hale dna secret set OIDC_CLIENT_SECRET
```

The client secret your provider issued goes into the vault as
`oidc-client-<client>`, and the head reads it there on the one line
that builds its token request. An unmapped subject gets no session; a
member `dna.oidc.board` names acts with the Board's authority. A head
started with no principal source refuses to start.

Local mode is OIDC too: `dna/face/start.sh` starts a stub provider
(`dna/oidc`) on the loopback, under a key made for that launch and the
`oidc-client-dna-local` secret `init` drew, and signs you in as
yourself. An issuer on the loopback is anyone's who can bind its port,
so its key is pinned (`dna.oidc.key`). A program with no person behind
it gets a service token instead, mapped with `git config --local --add
dna.oidc.service "<subject>=<service>"`, its secret the drawn
`oidc-service-<service>`.

Sessions, tokens, service reads and the face are [the head's](./head.md).

## Sealed credentials

A part never holds a credential as a `String`. It holds a
`std::secret::Credential { vault: … }`, a sealed locus that resolves the
entry on every privileged call, and reveals it only in the statement
that writes it onto the wire. The compiler holds the reveal to that one
use: consumed in the same statement by a wire write, a whole-value
comparison or a `@secret` parameter, never bound, stored, returned or
branched on ([Verification: secrets](../verification.md#secrets-confine-classify-claim)).

A model key is the example. The catalog's hosted backends carry a
`HostedCredential`, which names the key and reveals it in the request
header, from `dna/core/models.hl`:

```hale,fragment
fn model_key_slot(key: String) -> String { return "model-" + key; }

@sealed locus HostedCredential {
    params { key: String = "OPENAI_API_KEY"; scheme: String = "bearer"; }
    // the vault's name for it, read where it is presented and nowhere else
    fn slot() -> String { return model_key_slot(self.key); }
    fn ready() -> Bool { return std::secret::Credential { vault: self.slot() }.ready(); }
    // …
    @effects(is: { secret_use, external_model })
    fn post_json(url: String, body: String, extra: String) -> std::http::ClientResponse fallible(std::http::HttpError) {
        let u = std::http::parse_url(url) or raise;
        return std::http::request(std::http::ClientRequest {
            method: "POST",
            url: u,
            headers: "Content-Type: application/json\r\n" + extra
                + (if self.scheme == "x-api-key" { "x-api-key: " } else { "Authorization: Bearer " })
                + std::secret::Credential { vault: self.slot() }.reveal_text(),
            body: std::bytes::from_string(body)
        }) or raise;
    }
}
```

A backend whose credential is not `ready()` is not a permitted backend:
the router refuses it cleanly instead of the wire failing. The NATS
client reveals a role's password on the line that writes `CONNECT`, and
the head its client secret in its token request, the same way.

Two credentials are not sealed yet. The migration sets each memory
role's password in the SQL it runs as the owner, and the forge's token
reaches `gh`, which takes it no other way, as `GH_TOKEN` in that one
child's environment, through a file unlinked at once.

## Closures and violations

Some failures are not a part's to fix in place. A closure names the
invariant ([When things fail](../services/failure.md#declaring-an-invariant-closure)),
the part that owns it absorbs the violation, records it, and goes on.
The record keeps each as a `violation.recorded` row, `{kind, subject,
owner, detail}`. No topic carries one. There are three:

| kind | subject | owner | what happened, and what the owner does |
| --- | --- | --- | --- |
| `adapter_undeliverable` | `nerves` | the node's holder | a publish the stream did not acknowledge in its window; the node stops, gives the body lease back and exits 75 for its unit to start it again with a fresh connection |
| `lease_unsettled` | the attempt | `spine` | a leg's lease expired with no outcome; recorded once per lease, and the attempt is asked again |
| `pulse_stopped` | `heart` | the spine's holder | the heart's pull on its durable was refused while the connection held; once per outage, and the durable is made again |

Because every request is a row first, a violation loses no fact: the
next node relays every request still unanswered.

`hale dna board` shows how many there are and the latest five:

```text
violations: <n> recorded, each absorbed by its owner (the latest last)
  <kind> <subject> (<owner>): <detail>
```

## How it breaks

| symptom | cause | what to do |
| --- | --- | --- |
| `… nothing starts without them` | a drawn secret is missing from the vault | `hale dna upgrade` |
| `secret set: … is no slot of the organism's …` | the name is not a slot | `hale dna secrets` lists the slots |
| `secret rotate: <NAME> was never set` | rotate before set | `hale dna secret set <NAME>` |
| `refused: dna/nats.secrets.conf is tracked by git …` | a secrets file is tracked | `git rm --cached` it; `hale dna upgrade` adds it to `.gitignore` |
| `no forge token: the vault holds none …` | the forge's slot is empty | `hale dna secret set FORGE_TOKEN` |
| a head that will not start | no principal source, or a loopback issuer without its pinned key | configure OIDC as above ([The head](./head.md)) |
