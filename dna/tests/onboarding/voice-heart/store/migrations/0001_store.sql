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
