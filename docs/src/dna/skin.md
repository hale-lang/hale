# The Skin

The **skin** is the organism's credential, identity, and failure-containment boundary (`dna/host/secrets.hl`, `dna/core/principal.hl`, `dna/oidc/stub.hl`). It provisions and validates secrets in a local or remote **Vault**, isolates PostgreSQL and NATS credentials per component, authenticates HTTP callers to the head via **OIDC**, enforces compile-time secret confinement with `std::secret::Credential`, and records absorbed runtime invariant failures as `violation.recorded` rows. Every component fails closed if a required secret is missing, and secret values are never written to committed files, record rows, URLs, CLI arguments, or environment variables.

```sh
hale dna secrets                        # check status of all drawn and user-supplied secrets
hale dna secret set OPENAI_API_KEY      # populate a secret slot from stdin
hale dna secret rotate OPENAI_API_KEY   # rotate an existing secret slot from stdin
```

## The Vault

Secrets are stored as raw byte entries in a vault, addressed by slot name:

- **Local filesystem vault (default)**: Stored in `HALE_VAULT_DIR`, falling back to `$XDG_CACHE_HOME/hale/vault` or `~/.cache/hale/vault`. Each secret is a mode `0600` file named after its slot; empty files are treated as missing. Because a local vault directory is readable by any process running under the same OS user, component isolation relies on passing each process only the specific vault slot name it is authorized to resolve.
- **External Vault server**: Configured by setting `HALE_VAULT_ADDR` and a per-host token in `HALE_VAULT_TOKEN`. External vaults are provisioned out of band; DNA startup verifies that all required slots exist and reports any missing entries.

### Secret Slots & Drawn Credentials

Run `hale dna secrets` to list every secret required by the organism and whether it is present in the vault (secret values are never displayed):

```text
$ hale dna secrets
the organism's secrets (the local vault, …):
  present  postgres-dna_35bf…_spine  memory: the spine's role
  present  postgres-dna_35bf…_head  memory: the head's role
  present  postgres-owner-35bf…  memory: its allocation's superuser in the local habitat
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

Every vault entry matches one of the following naming patterns (where `<identity>` is `[dna] record`, `<org>` is `dna_<identity>`, and `<id>` is the first 12 hex digits of `<identity>`):

| Vault Slot Name | Provisioning Kind | Owner / Purpose |
| --- | --- | --- |
| `postgres-dna_<identity>_spine`, `…_head` | Drawn | PostgreSQL `spine` and `head` database roles |
| `postgres-owner-<id>` | Drawn | Superuser password for the local habitat's `memory-<id>` container |
| `nats-<org>-<role>` (`owner`, `spine`, `head`, `reflexes`) | Drawn | NATS JetStream role accounts |
| `nats-<org>-app-<name>` | Drawn | NATS account for attached application `<name>` |
| `oidc-client-<client>` (`dna-local` by default) | Drawn (loopback issuer) or Slot (remote issuer) | OIDC client secret for `hale dna head` |
| `oidc-service-<service>` (per `dna.oidc.service`) | Drawn | Machine-to-machine service client secret |
| `forge-token` | Slot | GitHub / forge API token (`FORGE_TOKEN`) |
| `model-<NAME>` (per `dna/org/models.hl` or `dna/org/work.hl`) | Slot | Hosted LLM provider API key |
| `secret-<org>-<name>` (per `[dna.secrets.<name>]` in `hale.toml`) | Drawn or Slot, as declared | A secret the seed declares for its own programs and host commands ([Declared Secrets](#declared-secrets)) |
| `secret-<org>-<name>--<label>` (per member a person fills) | Slot (a family) | A member of a secret family the seed declares |

- **Drawn secrets**: Generated automatically by `hale habitat graft` and `hale dna upgrade` (32 random hex characters from `/dev/urandom`) whenever missing. NATS passwords are rotated on every `graft` and `upgrade`, updating `<habitat>/allocations/<id>/nats.secrets.conf` and `postgres.secrets` (mode `0600`). `hale dna run` and `hale dna dev` refuse to start if any drawn secret is missing:

```text
hale dna run: the organism's secrets are not all in the vault: … (`hale dna upgrade` draws them; `hale dna secrets` lists them); nothing starts without them
```

- **User-supplied slots**: Empty entries that an operator populates manually (`FORGE_TOKEN`, `OIDC_CLIENT_SECRET` for non-loopback OIDC issuers, and each `model-<NAME>` key referenced in `dna/org/models.hl` or `dna/org/work.hl`). Missing slots are flagged on `hale dna board` (missing model keys are reported by the active [body host](./heart.md#a-body-on-a-server) via `body.credential_missing`).
- **Shared-record owner roles**: Per-owner head roles (`postgres-<org>_head_<owner>`) are provisioned dynamically by `hale dna memory migrate` when `HALE_DNA_OWNER_KEYS` is set ([Memory and the Record](./memory.md)).

### Filling a Slot

Populate or rotate a user-supplied slot with `hale dna secret set` or `hale dna secret rotate`:

```sh
hale dna secret set <NAME> [--body <user@host>]
hale dna secret rotate <NAME> [--body <user@host>]
```

- **Valid `<NAME>` identifiers**: `FORGE_TOKEN`, `OIDC_CLIENT_SECRET`, any credential key declared via `HostedCredential { key: "…" }` in `dna/org/models.hl` or `dna/org/work.hl`, or the name of a slot the seed declares in `[dna.secrets]` ([Declared Secrets](#declared-secrets)). Unrecognized names are rejected.
- **Stdin-only input**: The secret value is read from a single line on stdin. Passing `NAME=value` on the command line is rejected to prevent secrets from leaking into shell history or `ps` output.

```text
$ hale dna secret set OPENAI_API_KEY
value for OPENAI_API_KEY (this machine), on one line:
secret set: OPENAI_API_KEY is in its slot of the vault, …/model-OPENAI_API_KEY (secret.rotated OPENAI_API_KEY; the value is nowhere in the record). A part reads it from the vault where it uses it
```

- **Remote host bodies (`--body`)**: When `--body <user@host>` is passed (or `dna.body` is configured in Git config via `hale dna body provision`), the secret is piped over SSH stdin directly into the remote host's vault.
- **Audit logging**: Both `set` and `rotate` append `secret.rotated <NAME>` (recording target host and author, never the secret value) to the record. `secret rotate` fails if the slot has never been set.
- **No environment fallback**: Model keys and `FORGE_TOKEN` are read exclusively from the vault. DNA never reads model keys from shell environment variables and never uses ambient `gh` CLI credentials when `forge-token` is unset.

### Declared Secrets

A seed declares the secrets its own programs and host commands read in its `hale.toml`, and the habitat keeps each in the organism's vault and hands it to its readers:

```toml
[dna.secrets.operator-token]
kind    = "drawn"                 # the habitat draws it (32 hex), at graft and upgrade
readers = ["api", "host.node"]    # programs by directory name; host commands as host.<command>

[dna.secrets.upstream-key]
kind    = "slot"                  # a person fills it: hale dna secret set upstream-key
readers = ["api"]

[dna.secrets.dashboard]
kind    = "drawn"
person  = "read"                  # hale dna secret show dashboard reveals it

[dna.secrets.model-token]
kind    = "slot"
family  = true                    # several values, each filled under a label
readers = ["worker"]
```

- **Names and keys**: a name is lowercase letters, digits and `-` (a letter first, at most 32 characters, no `-` last or doubled). `kind` is `drawn` or `slot`; `person` is `read`, `write` (a person sets it and never reads it back; a slot's default) or `none` (a drawn secret's default). A drawn secret a person would write, a slot no person may touch, an unknown key, and a `host.<command>` reader with no `[dna.host.<command>]` are all refused. `hale dna declared-secrets` prints the declaration as the toolchain reads it.
- **Its vault entry** is `secret-<org>-<name>`. A drawn one is drawn when missing, by `hale habitat graft` and `hale dna upgrade`, and **never rotated by them** (unlike the NATS passwords): `hale dna secret rotate <name>` draws it anew. `hale dna secret set` on a drawn secret is refused. A slot is listed MISSING by `hale dna secrets` until `hale dna secret set <name>` fills it.
- **Show**: `hale dna secret show <name>` prints the value of a secret declared `person = "read"`, and refuses any other, saying why. Through a pointer, run it (and `set`) as `hale habitat organism <organism> dna secret show <name>`: it runs on the habitat's machine, where the vault is.
- **Families**: `family = true` on a slot holds as many values as the deployment needs (several model tokens on one host, say), each a member a person fills under a label: `hale dna secret set model-token --member work`, `rotate … --member work`, `remove … --member work`, and `show … --member work` for a family a person reads. A label is 1 to 63 characters, a lowercase letter or a digit and then lowercase letters, digits and `-` (`^[a-z0-9][a-z0-9-]{0,62}$`). Each member is the entry `secret-<org>-<name>--<label>`. A family named without `--member` is refused, and so is `--member` on a secret that is no family. `--as` keeps its meaning on these verbs too: who acts, as the record's row names it. `hale dna secrets` lists the family once with its members, and as MISSING while it has none.
- **Handed to programs**: where the organism's services run (the habitat's apply, on this machine or an SSH host, and `hale dna dev`/`run` for a repository), the habitat copies each secret a program reads into a vault directory of the organism's own there (mode 700, each entry 600; `<habitat>/allocations/<id>/vault` here, `~/.hale/habitat/<project>/.vault` on a host, never in a checkout) and hands compose `HALE_VAULT_DIR` (that directory) and `HALE_SECRET_<NAME>` (the entry's name; a family's, each member's entry, one a line). The compose file passes them to the services that read them and mounts the directory read-only at the same path:

```yaml
services:
  api:
    environment:
      HALE_VAULT_DIR: ${HALE_VAULT_DIR}
      HALE_SECRET_OPERATOR_TOKEN: ${HALE_SECRET_OPERATOR_TOKEN}
    volumes:
      - ${HALE_VAULT_DIR}:${HALE_VAULT_DIR}:ro
```

  A program reads it as `std::secret::Credential { vault: std::env::var("HALE_SECRET_OPERATOR_TOKEN") }`. Compose's environment is the project's, so the grant to the declared readers is the compose file's to keep; a secret no program reads is never handed.
- **Handed to host commands**: `hale habitat organism place|unplace` hands a command each secret naming it among its readers, for the run alone ([The Habitat](../habitat.md)).
- **Superseded**: a host command's `operator_token` and `{operator_token_file}` input are refused; declare the token as a drawn secret with `host.<command>` among its readers.

Automated workflow rotation (`secret-rotate@1`) is present in the workflow catalog but rejected at admission because workflows are not permitted to write to `vault`:

```text
secret-rotate@1  a secret rotated
  0. rotate  vault
  1. recorded  record
  refused at admission: workflow secret-rotate@1 step 0 (rotate) writes the vault (GH #989), which is not built yet
```

## Roles and Accounts

Each DNA component receives credentials only for the specific database role and NATS account it requires:

| Component | PostgreSQL Role (`memory`) | NATS Account (`nerves`) |
| --- | --- | --- |
| **Spine** (host supervisor & `dna/org`) | `dna_<identity>_spine` (read/write record & ledger tables) | `spine` (publish/consume `<org>.dna.>`, consume `heart` durable, publish `<org>.head.>`) |
| **Head** (`hale dna head`, CLI) | `dna_<identity>_head` (read tables, acquire/fence claims, append to ledger) | `head` (subscribe-only; cannot publish) |
| **Migrations** (`hale habitat migrate`) | `owner` DSN (DDL schema migrations; never held by running daemons) | `owner` (create/update/delete JetStream stream) |
| **Application** (`heart`) | None | `app-<name>` (publish-only on `<org>.app.<name>.>`) |
| **Reflexes** (`dna/reflexes`) | None | `reflexes` (publish-only on `<org>.app.reflexes.>`) |

PostgreSQL DSNs reference their password via `&vault=postgres-<role>`, and NATS connections take `nats://host:port` alongside a `std::secret::Credential` pointing to the vault slot. See [Memory and the Record](./memory.md), [The Nerves](./nerves.md), and [The Heart](./heart.md#its-broker-account).

## OIDC Authentication for the Head

Every HTTP session on `hale dna head` is authenticated against an OpenID Connect (OIDC) provider and mapped to an organizational member name in Git config:

```sh
git config dna.principal oidc
git config dna.oidc.issuer https://id.example.com
git config dna.oidc.client dna-head
git config dna.oidc.redirect https://dna.example.com/auth/callback
git config --add dna.oidc.member "<subject>=alice"
git config dna.oidc.board alice
hale dna secret set OIDC_CLIENT_SECRET
```

- **Client secret**: Stored in the vault at `oidc-client-<client>` and revealed inline when `hale dna head` exchanges an authorization code for a token.
- **Authorization**: Unmapped OIDC subjects are denied a session. Members listed in `dna.oidc.board` receive Board authority. Starting `hale dna head` without a configured principal source fails immediately.
- **Local OIDC stub (`dna-local`)**: When running locally, `hale dna head` (or `dna/face/start.sh`) launches a loopback OIDC stub (`dna/oidc`) using the drawn `oidc-client-dna-local` secret and pins the ephemeral signing key in `dna.oidc.key`.
- **Service tokens**: Non-interactive clients authenticate using service tokens mapped via `git config --local --add dna.oidc.service "<subject>=<service>"`, backed by the drawn `oidc-service-<service>` vault secret. See [The Head and the Face](./head.md).

## Sealed Credentials

In Hale code, credentials are never stored in `String` fields or variables. Instead, components hold a `std::secret::Credential { vault: … }`—a `@sealed` locus that resolves the vault entry on demand and reveals its contents only inside the expression that writes to the network wire.

The compiler enforces that `.reveal_text()` is consumed immediately in the same statement by a wire write, a whole-value equality check, or a `@secret` parameter; it cannot be bound to a `let` variable, stored in a struct/locus field, returned from a function, or branched on ([Verification: Secrets](../verification.md#5-secret-confinement-sealed--stdsecret)).

For example, `HostedCredential` in `dna/core/models.hl` reveals a model API key directly inside the outbound HTTP request header:

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

If `.ready()` returns `false` (the vault slot is empty), the model router skips that backend before attempting a network connection. Both pond's NATS client (`CONNECT` frame) and `hale dna head` (OIDC token exchange) use the same inline `std::secret::Credential` pattern.

> [!NOTE]
> Two external boundaries currently require non-sealed delivery: PostgreSQL role creation during `hale dna memory migrate` passes role passwords in owner DDL statements, and `gh` CLI invocations receive `forge-token` via `GH_TOKEN` in the child process environment from an immediately unlinked temporary file.

## Closures and Violations

When a runtime invariant closure ([When Things Fail](../services/failure.md#declaring-invariants-closure)) fails in a component that cannot recover in place, the owning supervisor absorbs the failure, writes a `violation.recorded` row (`{kind, subject, owner, detail}`) to the record, and executes a clean recovery action:

| Violation Kind | Subject | Owner | Trigger & Recovery Action |
| --- | --- | --- | --- |
| `adapter_undeliverable` | `nerves` | Node holder (`user@host:<clone>`) | A NATS JetStream publish remained unacknowledged past its delivery window. The node stops `dna/org` and the application, releases the body lease (`body.released`, `why: nerves`), and exits `75` so systemd restarts it with a fresh connection. |
| `lease_unsettled` | Attempt ID | `spine` | A worker leg's attempt lease expired before an outcome was submitted. Logged once per expired lease, after which the attempt is re-dispatched. |
| `pulse_stopped` | `heart` | Spine holder | Pulling from the `heart` durable consumer failed while the NATS connection remained open. Logged once per outage, after which the host recreates the `heart` durable and resumes pulling. |

Because every request row is persisted to PostgreSQL before NATS delivery, restarting on a violation never loses state: the restarted node re-relays all unanswered requests.

`hale dna board` displays the total violation count and the 5 most recent `violation.recorded` rows:

```text
violations: <n> recorded, each absorbed by its owner (the latest last)
  <kind> <subject> (<owner>): <detail>
```

## Troubleshooting Common Errors

| Symptom | Cause | Resolution |
| --- | --- | --- |
| `… nothing starts without them` | One or more drawn secrets are missing from the vault | Run `hale dna upgrade` to regenerate missing drawn secrets |
| `secret set: … is no slot of the organism's …` | The secret name does not match any declared slot | Run `hale dna secrets` to list valid slot names |
| `secret rotate: <NAME> was never set` | Attempted to rotate an empty slot | Use `hale dna secret set <NAME>` for initial population |
| `no forge token: the vault holds none …` | The `forge-token` slot is empty | Run `hale dna secret set FORGE_TOKEN` |
| `hale dna head` refuses to start | No `dna.principal` configured, or loopback OIDC issuer lacks pinned `dna.oidc.key` | Configure OIDC settings in `git config` as shown above ([The Head and the Face](./head.md)) |
