-- Saved windows.
--
-- One row per successful ask: everything reopening the view needs, plus the
-- parentage that draws the palette's tree. `parent_id` points at another
-- window owned by the same rule as the row itself; the database does not
-- enforce same-ownership, because reads are always scoped by `user_id` and a
-- stray pointer can only ever attach one of your rows under another of them.
--
-- `name` is nullable and written by nothing yet; renaming arrives later.
-- `created_at` is database-assigned so sibling order has one clock.
--
-- `window` is a reserved word in SQL, so the table name is always quoted.
--
-- Never edit an applied migration: the ledger records that it ran, not what
-- it said, so a change to an applied file is a change that no environment
-- will ever pick up.

create table "window" (
  id         uuid        primary key,
  user_id    text        not null,
  parent_id  uuid        references "window"(id),
  request    text        not null,
  sql        text        not null,
  shape      jsonb       not null,
  template   text        not null,
  name       text,
  created_at timestamptz not null default now()
);

create index window_user_created on "window" (user_id, created_at);
create index window_parent_id on "window" (parent_id);
