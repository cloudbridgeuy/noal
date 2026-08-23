-- noal's own backlog, so the demo is judged without reading the seed.
-- IDs are explicit because rows reference each other.

insert into ticket (id, kind, parent_id, title, description, status, tags, created_at) values
  -- Epics
  (1, 'epic', null, 'Render MVP', 'An agent writes a SQL query and a Tera template; the server fills it with live rows.', 'in_progress', '{mvp,agent}', now() - interval '20 days'),
  (2, 'epic', null, 'Ticket schema', 'A small ticket tracker with epics, tasks, and subtasks for the MVP to read.', 'done', '{data,postgres}', now() - interval '19 days'),
  (3, 'epic', null, 'AI Gateway', 'Route every model call through Cloudflare AI Gateway for logs and model switching.', 'todo', '{infra,llm}', now() - interval '10 days'),

  -- Tasks under Render MVP (1)
  (10, 'task', 1, 'Ask endpoint', 'POST /ask takes a request and returns a fragment.', 'in_progress', '{backend,axum}', now() - interval '18 days'),
  (11, 'task', 1, 'Plan prompt', 'The model returns SQL plus the shape of the result.', 'done', '{llm,prompt}', now() - interval '17 days'),
  (12, 'task', 1, 'Render prompt', 'The model returns a Tera template from the shape alone.', 'in_progress', '{llm,prompt}', now() - interval '16 days'),
  (13, 'task', 1, 'Retry with feedback', 'Each stage retries once with the error appended.', 'todo', '{llm,resilience}', now() - interval '12 days'),
  (14, 'task', 1, 'Debug overlay', 'A hidden panel shows SQL, shape, template, attempts, and timings.', 'todo', '{frontend,htmx}', now() - interval '11 days'),

  -- Tasks under Ticket schema (2)
  (20, 'task', 2, 'Write migration', 'ticket and comment tables with enums for kind and status.', 'done', '{postgres}', now() - interval '19 days'),
  (21, 'task', 2, 'Seed backlog', 'About thirty tickets describing noal itself.', 'done', '{postgres,seed}', now() - interval '18 days'),
  (22, 'task', 2, 'Catalog document', 'A hand-written description of the schema for the planner.', 'in_progress', '{llm,docs}', now() - interval '15 days'),

  -- Tasks under AI Gateway (3)
  (30, 'task', 3, 'Create gateway', 'Create the noal gateway in the Cloudflare dashboard.', 'todo', '{infra}', now() - interval '9 days'),
  (31, 'task', 3, 'Point rig at the gateway', 'Set LLM_BASE_URL and the cf-aig-authorization header.', 'todo', '{backend,llm}', now() - interval '8 days'),
  (32, 'task', 3, 'Compare models on the plan stage', 'Try a cheaper model for planning and measure failures.', 'todo', '{llm,research}', now() - interval '5 days'),

  -- Subtasks
  (100, 'subtask', 10, 'Form on the home page', 'One input, hx-post to /ask, swaps itself.', 'done', '{frontend,htmx}', now() - interval '17 days'),
  (101, 'subtask', 10, 'Wrap SQL in json_agg', 'One text column back over simple_query.', 'done', '{postgres}', now() - interval '16 days'),
  (102, 'subtask', 10, 'Run query and render concurrently', 'Join the database future and the render future.', 'in_progress', '{backend}', now() - interval '14 days'),
  (103, 'subtask', 10, 'Read-only transaction', 'begin read only so a model UPDATE fails.', 'todo', '{postgres,safety}', now() - interval '13 days'),
  (110, 'subtask', 11, 'Shape enum', 'text, integer, number, boolean, timestamp, text_list, object_list.', 'done', '{types}', now() - interval '17 days'),
  (111, 'subtask', 11, 'JSON schema for structured output', 'Derive with schemars and pass to rig.', 'done', '{llm}', now() - interval '16 days'),
  (120, 'subtask', 12, 'Strip code fences', 'The model sometimes wraps the template in backticks.', 'in_progress', '{llm,parsing}', now() - interval '12 days'),
  (121, 'subtask', 12, 'Forbid script tags in the prompt', 'Ask for plain HTML and htmx attributes only.', 'todo', '{llm,prompt}', now() - interval '11 days'),
  (130, 'subtask', 13, 'Append Postgres error to plan prompt', '', 'todo', '{llm}', now() - interval '10 days'),
  (131, 'subtask', 13, 'Append Tera error to render prompt', '', 'todo', '{llm}', now() - interval '10 days'),
  (140, 'subtask', 14, 'Backtick toggles the panel', '', 'todo', '{frontend}', now() - interval '9 days'),
  (141, 'subtask', 14, 'Corner button for trackpads', '', 'todo', '{frontend}', now() - interval '9 days'),
  (220, 'subtask', 22, 'Describe the hierarchy rule', 'Subtask to task to epic, in words the model follows.', 'in_progress', '{docs}', now() - interval '14 days'),
  (221, 'subtask', 22, 'List enum values', 'So the model filters on status correctly.', 'done', '{docs}', now() - interval '14 days'),
  (300, 'subtask', 30, 'Pick a gateway name', 'noal.', 'todo', '{infra}', now() - interval '8 days'),
  (310, 'subtask', 31, 'Store provider keys in the gateway', 'BYOK so the Worker holds one token.', 'todo', '{infra,secrets}', now() - interval '7 days'),
  (320, 'subtask', 32, 'Build a fixed prompt set', 'Ten sample requests, run against each candidate model, to compare plan failure rates.', 'todo', '{llm,research}', now() - interval '4 days');

insert into comment (ticket_id, author, body, created_at) values
  (1, 'guzman', 'The idea under test is whether a constrained template gives good interfaces.', now() - interval '19 days'),
  (1, 'noal', 'First fragment rendered end to end.', now() - interval '3 days'),
  (10, 'guzman', 'Keep the handler thin. The loop policy belongs in core.', now() - interval '17 days'),
  (10, 'noal', 'Blocked on the concurrent join until SendWrapper was applied to the rig call.', now() - interval '13 days'),
  (11, 'guzman', 'Structured output comes back as a text block with JSON inside.', now() - interval '16 days'),
  (11, 'noal', 'Shape enum is flat on purpose; Anthropic structured output dislikes recursion.', now() - interval '15 days'),
  (12, 'guzman', 'The model must never see rows. Only the shape.', now() - interval '16 days'),
  (12, 'noal', 'Strict undefined variables catch most binding mistakes.', now() - interval '12 days'),
  (13, 'guzman', 'One retry per stage. More than that hides bad prompts.', now() - interval '12 days'),
  (14, 'guzman', 'Like react-query devtools: hidden, a key binding, a corner button.', now() - interval '11 days'),
  (14, 'noal', 'Payload travels inside the fragment as a JSON script tag.', now() - interval '10 days'),
  (20, 'noal', 'Enums for kind and status; text[] for tags.', now() - interval '19 days'),
  (21, 'guzman', 'Dogfood. The seed is our own backlog.', now() - interval '18 days'),
  (22, 'noal', 'Catalog is compiled in with include_str.', now() - interval '15 days'),
  (3, 'guzman', 'Gateway is a managed service, not a Worker. One URL change.', now() - interval '10 days'),
  (30, 'noal', 'Needs the account id for the base URL.', now() - interval '9 days'),
  (31, 'noal', 'rig ClientBuilder has http_headers for the gateway token.', now() - interval '8 days'),
  (32, 'guzman', 'Measure plan failures per model before switching.', now() - interval '5 days'),
  (100, 'noal', 'hx-swap outerHTML on the form itself.', now() - interval '17 days'),
  (101, 'noal', 'coalesce(json_agg(t), ''[]'') so an empty result is still JSON.', now() - interval '16 days'),
  (102, 'guzman', 'The render call is the slow one; overlap it with the query.', now() - interval '14 days'),
  (103, 'guzman', 'No security yet, but read only costs nothing.', now() - interval '13 days'),
  (110, 'noal', 'object_list carries nested fields for comments.', now() - interval '17 days'),
  (120, 'noal', 'Strip leading and trailing fences, keep everything else.', now() - interval '12 days'),
  (121, 'guzman', 'No CSP yet, so the prompt is the only guard.', now() - interval '11 days'),
  (130, 'noal', 'Postgres errors name the column; that is enough for a fix.', now() - interval '10 days'),
  (140, 'guzman', 'Backtick, like a game console.', now() - interval '9 days'),
  (220, 'noal', 'Say it twice: once as a rule, once as an example query.', now() - interval '14 days'),
  (300, 'guzman', 'noal.', now() - interval '8 days'),
  (310, 'noal', 'Then ANTHROPIC_API_KEY can leave the Worker.', now() - interval '7 days');

select setval('ticket_id_seq', (select max(id) from ticket));
