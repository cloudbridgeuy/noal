-- The ticket tracker noal's first agentic surface reads from.
--
-- Three kinds of ticket share one table. `parent_id` is the only hierarchy
-- rule: a subtask points at a task, a task at an epic. The database does not
-- enforce the kind of the parent; the MVP tests rendering, not integrity.

create type ticket_kind as enum ('epic', 'task', 'subtask');
create type ticket_status as enum ('todo', 'in_progress', 'done');

create table ticket (
  id          serial primary key,
  kind        ticket_kind   not null,
  parent_id   integer references ticket(id),
  title       text          not null,
  description text          not null default '',
  status      ticket_status not null default 'todo',
  tags        text[]        not null default '{}',
  created_at  timestamptz   not null default now()
);

create table comment (
  id         serial primary key,
  ticket_id  integer     not null references ticket(id),
  author     text        not null,
  body       text        not null,
  created_at timestamptz not null default now()
);

create index comment_ticket_id on comment (ticket_id);
create index ticket_parent_id on ticket (parent_id);
