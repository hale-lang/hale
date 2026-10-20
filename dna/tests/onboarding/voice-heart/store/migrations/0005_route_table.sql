create table route_table (
  id      boolean primary key default true check (id),
  version bigint not null,
  at      timestamptz not null,
  body    jsonb not null
);
