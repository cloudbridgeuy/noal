# Schema

PostgreSQL. Two tables. Read-only.

## ticket

| column      | type                                        | meaning |
| ----------- | ------------------------------------------- | ------- |
| id          | integer, primary key                        | |
| kind        | enum: 'epic', 'task', 'subtask'             | what level of work this is |
| parent_id   | integer, nullable, references ticket(id)    | a subtask's parent is a task; a task's parent is an epic; an epic has none |
| title       | text                                        | |
| description | text, may be empty                          | |
| status      | enum: 'todo', 'in_progress', 'done'         | |
| tags        | text[]                                      | free-form labels such as 'llm', 'postgres', 'frontend' |
| created_at  | timestamptz                                 | |

## comment

| column     | type                                  | meaning |
| ---------- | ------------------------------------- | ------- |
| id         | integer, primary key                  | |
| ticket_id  | integer, references ticket(id)        | |
| author     | text                                  | a person's handle, or 'noal' for the system |
| body       | text                                  | |
| created_at | timestamptz                           | |

# Rules

- The hierarchy is subtask → task → epic, through `parent_id`. The database
  does not enforce which kind a parent has to be; a well-formed backlog keeps
  a subtask's parent a task and a task's parent an epic, but nothing in the
  schema requires it. To reach an epic from a subtask, join `ticket` twice.
- Compare enums as strings: `status = 'done'`, `kind = 'task'`.
- Tags are an array: `'llm' = any(tags)` tests membership.
- To attach comments to a ticket in one row, aggregate them:
  `(select coalesce(json_agg(json_build_object('author', c.author, 'body', c.body, 'created_at', c.created_at) order by c.created_at), '[]') from comment c where c.ticket_id = t.id) as comments`
- Always alias every output column with a plain snake_case name.
- Return at most 200 rows.

# Example

Request: "tasks still open under the Render MVP epic, with their comments"

```sql
select t.id, t.title, t.status, t.tags,
       (select coalesce(json_agg(json_build_object('author', c.author, 'body', c.body) order by c.created_at), '[]')
          from comment c where c.ticket_id = t.id) as comments
  from ticket t
  join ticket e on e.id = t.parent_id
 where t.kind = 'task' and e.title = 'Render MVP' and t.status <> 'done'
 order by t.created_at
```
