# The store

Postgres holds everything durable, and the api and the brain both read and
write it, so its tables and the transactions over them are a contract
between them. This document is that contract: the state laws the README
states in prose, written as SQL. The migrations in the shared store seed
are its executable form and must match it.

What is *not* here: the route table (a snapshot the brain publishes on
NATS, `internal.yaml`), prompts and responses (never stored), and anything
a node keeps locally.

Conventions: ids are the API's slugs and opaque strings (`text`); money is
integer micros; times are `timestamptz`; JSON the API defines is stored as
`jsonb` in the API's own shape, so a row reads back as the resource.

## Configuration

Written only by the admin plane, each write one transaction that also
appends to `change`.

```sql
create table catalog_model (
  id                text primary key,
  owned_by          text not null default 'local',
  kind              text not null check (kind in ('local', 'hosted')),
  context_length    integer,
  max_output_tokens integer,
  price             jsonb,             -- ModelPrice, or null for free
  created_at        timestamptz not null default now(),
  removed_at        timestamptz
);

create table account (
  id          text primary key,
  name        text not null,
  provider    text,
  billing     text not null check (billing in ('subscription', 'metered', 'none')),
  status      text not null default 'active' check (status in ('active', 'archived')),
  created_at  timestamptz not null default now(),
  archived_at timestamptz
);

create table project (
  id           text primary key,
  name         text not null,
  status       text not null default 'active' check (status in ('active', 'archived')),
  owners       text[] not null,
  data_classes text[] not null default '{internal}',
  models       text[],                 -- null: the whole catalog
  accounts     text[],                 -- null: any
  position     text,
  created_at   timestamptz not null default now(),
  archived_at  timestamptz
);

create table budget (
  project          text primary key references project,
  allowance_micros bigint not null check (allowance_micros >= 0),
  currency         text not null default 'USD',
  period           text not null check (period in ('day', 'week', 'month', 'none')),
  period_anchor    timestamptz not null
);

-- One rate limit. Exactly one of project, key, model_id, account is set,
-- matching scope.
create table "limit" (
  id             bigserial primary key,
  scope          text not null check (scope in ('project', 'key', 'model', 'account')),
  project        text references project,
  key            text,
  model_id       text,
  account        text references account,
  window_seconds integer not null check (window_seconds >= 1),
  requests       integer,
  input_tokens   bigint,
  output_tokens  bigint,
  concurrent     integer
);

create table api_key (
  id            text primary key,
  project       text not null references project,
  name          text not null,
  prefix        text not null,
  secret_digest bytea not null,        -- the secret is never stored
  models        text[],
  accounts      text[],
  status        text not null default 'active' check (status in ('active', 'revoked', 'expired')),
  created_at    timestamptz not null default now(),
  created_by    text,
  expires_at    timestamptz,
  last_used_at  timestamptz,
  revoked_at    timestamptz
);
alter table "limit" add foreign key (key) references api_key;

create table node (
  id                 text primary key,
  name               text,
  token_digest       bytea not null,   -- the enrollment token is never stored
  max_in_flight      integer check (max_in_flight >= 1),  -- the coordinator's cap, or null
  status             text not null default 'pending' check (status in ('pending', 'up', 'down', 'revoked')),
  assignment_version integer not null default 0,          -- bumped by any change to the node or its seats
  created_at         timestamptz not null default now(),
  revoked_at         timestamptz
);

create table seat (
  node         text not null references node,
  id           text not null,
  engine       text not null,
  account      text not null references account,
  serves       jsonb not null,         -- [SeatModel]
  data_classes text[] not null default '{internal}',
  concurrency  integer not null default 1 check (concurrency >= 1),
  enabled      boolean not null default true,
  config       jsonb not null default '{}',
  primary key (node, id)
);

create table change (
  id        bigserial primary key,
  at        timestamptz not null default now(),
  principal text,
  action    text not null check (action in ('create', 'update', 'delete', 'archive', 'revoke', 'rotate')),
  target    text not null,
  before    jsonb,
  after     jsonb
);

create table policy_change (
  id       text primary key,
  status   text not null check (status in ('pending_review', 'applied', 'rejected')),
  target   text not null,
  proposed jsonb not null,
  review   text
);
```

## The fleet as reported

Written by the node plane and by api instances about themselves. Read by
the brain to compute the route table, and by the admin plane to show it.

```sql
create table node_state (
  node              text primary key references node,
  version           text,              -- the node software's version
  capabilities      jsonb not null,    -- Capabilities, as declared
  local             jsonb not null,    -- LocalControls, as reported
  registered_at     timestamptz not null,
  last_heartbeat_at timestamptz,
  in_flight         integer not null default 0,
  applied_version   integer            -- the assignment the node last reported on
);

create table seat_state (
  node        text not null,
  seat        text not null,
  status      text not null check (status in ('starting', 'up', 'down', 'disabled', 'paused', 'refused')),
  reason      text,
  in_flight   integer not null default 0,
  reported_at timestamptz not null,
  primary key (node, seat)
);

create table quota_window (
  account     text not null references account,
  name        text not null,           -- five_hour, seven_day, ...
  used_percent numeric not null,
  resets_at   timestamptz,
  observed_at timestamptz not null,
  seat        text,                    -- <node>/<seat> that reported it
  primary key (account, name)
);

create table api_instance (
  id                text primary key,
  started_at        timestamptz not null default now(),
  last_heartbeat_at timestamptz not null default now()
);
```

A node is `up` while its heartbeat is younger than three intervals, `down`
after; the brain applies that, and marks `abandoned` any request an
instance held when its own heartbeat lapsed (below).

## Counters and reservations

One row per limit per window, one row per budget per period, and one row
per request in flight. Every change is a conditional update, so two
instances never over-commit.

```sql
create table limit_state (
  limit_id      bigint not null references "limit",
  window_start  timestamptz not null,
  requests      integer not null default 0,
  input_tokens  bigint not null default 0,
  output_tokens bigint not null default 0,
  concurrent    integer not null default 0,
  primary key (limit_id, window_start)
);

create table budget_state (
  project         text not null references project,
  period_start    timestamptz not null,
  spent_micros    bigint not null default 0,
  reserved_micros bigint not null default 0,
  primary key (project, period_start)
);

create table request (
  id                text primary key,
  created_at        timestamptz not null default now(),
  instance          text not null references api_instance,
  project           text not null,
  key               text not null,
  requested_model   text not null,
  requested_account text,
  endpoint          text not null,
  data_class        text not null,
  reserved_input    bigint not null,
  reserved_output   bigint not null,
  reserved_micros   bigint not null,
  price             jsonb,             -- the model's price when reserved; settlement uses it
  limits            bigint[] not null, -- the limit ids reserved against
  period_start      timestamptz not null,
  status            text not null default 'in_flight' check (status in ('in_flight', 'settled', 'abandoned')),
  metadata          jsonb not null default '{}'
);
create index on request (instance) where status = 'in_flight';
```

## The ledger

Append-only. One row per request, in the API's `UsageRecord` shape, so a
row is the resource.

```sql
create table usage (
  id           text primary key,      -- the request id
  created_at   timestamptz not null,
  project      text not null,
  key          text not null,
  account      text,                   -- the account that served, if any
  requested_model text not null,
  status       text not null,
  served       jsonb,                  -- Served, or null
  input_tokens bigint not null default 0,
  cached_input_tokens bigint not null default 0,
  cache_write_input_tokens bigint not null default 0,
  output_tokens bigint not null default 0,
  reasoning_tokens bigint not null default 0,
  price_micros bigint not null default 0,
  currency     text not null default 'USD',
  record       jsonb not null          -- the whole UsageRecord, as the API returns it
);
create index on usage (project, created_at desc);
create index on usage (account, created_at desc);
```

Summaries (`GET /admin/v1/usage/summary`) are aggregates over `usage`, and
the admin event stream's `usage` events are its rows as they are inserted.

## The transactions

These are the state laws. Each is one transaction, and each conditional
`update` is what makes a limit hold across instances: a row that does not
change means the request does not fit.

### Reserve

Before serving. `$est_in` is the estimated input, `$max_out` the requested
maximum output (the model's default when the request sets none), `$cost`
their price in micros at the model's current price, `$limits` every limit
that applies (the project's, the key's, the model's, and those of the
accounts the request may spend).

```sql
begin;

-- each applicable limit, in its current window; a window row is created on
-- first use
insert into limit_state (limit_id, window_start)
  select id, date_bin(make_interval(secs => window_seconds), now(), 'epoch')
  from "limit" where id = any($limits)
on conflict do nothing;

update limit_state s
   set requests = s.requests + 1,
       input_tokens = s.input_tokens + $est_in,
       output_tokens = s.output_tokens + $max_out,
       concurrent = s.concurrent + 1
  from "limit" l
 where s.limit_id = l.id and l.id = any($limits)
   and s.window_start = date_bin(make_interval(secs => l.window_seconds), now(), 'epoch')
   and (l.requests      is null or s.requests + 1 <= l.requests)
   and (l.input_tokens  is null or s.input_tokens + $est_in <= l.input_tokens)
   and (l.output_tokens is null or s.output_tokens + $max_out <= l.output_tokens)
   and (l.concurrent    is null or s.concurrent + 1 <= l.concurrent);
-- if fewer rows changed than limits apply: rollback; 429 naming the first
-- limit that did not fit

insert into budget_state (project, period_start) values ($project, $period_start)
on conflict do nothing;

update budget_state s
   set reserved_micros = s.reserved_micros + $cost
  from budget b
 where s.project = b.project and s.project = $project and s.period_start = $period_start
   and s.spent_micros + s.reserved_micros + $cost <= b.allowance_micros;
-- if no row changed: rollback; 429 budget_exhausted

insert into request (id, instance, project, key, requested_model, requested_account,
                     endpoint, data_class, reserved_input, reserved_output,
                     reserved_micros, price, limits, period_start, metadata)
values ($id, $instance, $project, $key, $model, $account, $endpoint, $data_class,
        $est_in, $max_out, $cost, $price, $limits, $period_start, $metadata);

commit;
```

A rollback holds nothing: the window and period rows may remain, at their
previous counts.

### Settle

After the node's `result`, or after every attempt failed. `$in`, `$out`
and `$micros` are the actual usage and its price at the reserved price;
for a refused or failed request they are zero.

```sql
begin;

update limit_state s
   set input_tokens = s.input_tokens - r.reserved_input + $in,
       output_tokens = s.output_tokens - r.reserved_output + $out,
       concurrent = s.concurrent - 1
  from request r, "limit" l
 where r.id = $id and l.id = any(r.limits) and s.limit_id = l.id
   and s.window_start = date_bin(make_interval(secs => l.window_seconds), r.created_at, 'epoch');

update budget_state s
   set reserved_micros = s.reserved_micros - r.reserved_micros,
       spent_micros = s.spent_micros + $micros
  from request r
 where r.id = $id and s.project = r.project and s.period_start = r.period_start;

insert into usage (id, created_at, project, key, account, requested_model, status, served,
                   input_tokens, cached_input_tokens, cache_write_input_tokens,
                   output_tokens, reasoning_tokens, price_micros, currency, record)
values (...);

update request set status = 'settled' where id = $id;

commit;
```

`requests` in the window is not decremented: a request counts against the
window it was made in whether or not it succeeded, which is what a request
limit means.

### Heartbeat

Every api instance, every interval:

```sql
update api_instance set last_heartbeat_at = now() where id = $instance;
```

The node plane's heartbeat and reports write `node_state`, `seat_state`
and `quota_window` with plain upserts, each its own small transaction.

### Reclaim

The brain, each tick. An instance whose heartbeat is older than three
intervals is dead; every request it held is settled with zero usage as
`abandoned`, and the instance row goes.

```sql
begin;
-- for each request r where r.instance = $dead and r.status = 'in_flight':
--   the Settle transaction with $in = $out = $micros = 0 and
--   usage.status = 'abandoned', then
update request set status = 'abandoned' where id = $id;
-- then
delete from api_instance where id = $dead;
commit;
```

Node liveness is the same rule applied to `node_state.last_heartbeat_at`,
and the brain writes `node.status`.

### Assignment

Any admin write to a node or its seats, in the same transaction as the
change, bumps `node.assignment_version`. The api publishes the node's new
assignment on `voice.node.<id>.assign`; a node that is offline gets it
when it registers.

## The brain's read

Each tick, the brain computes the route table from `seat`, `seat_state`,
`node`, `node_state`, `quota_window`, `account`, `"limit"` and
`limit_state` (account-scope limits), and `catalog_model`: a seat routes
when it is enabled, reported `up`, on a node that is `up`, for an account
that is active and has headroom under its windows and limits. It writes
nothing back for this; the table goes to NATS. Its one write is reclaim.

## Migrations

The schema above is migration 1 of the shared store seed, registered with
[`pond/migrations`](https://github.com/hale-lang/pond/tree/main/migrations)
and run as the one-shot `migrate` step the api and the brain wait on. A
change to this document is a new migration, never an edit to an applied
one.
