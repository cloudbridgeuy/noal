# Design — Personalized Application Generator over Authorized Data

**Status:** Design baseline, v2 (reconciled with the scaffold)
**Date:** 2026-08-22
**Scope:** Prototype on AdventureWorks (Postgres port), hosted on Neon, built on
the existing `noal` scaffold: Rust + Axum on Cloudflare Workers, htmx, Cedar
authorization.

Sections marked **[exists]** describe code that is already in the repository.
Sections marked **[changed]** differ from the v1 handoff because of what the
scaffold already decided. Everything else is unchanged design intent.

---

## 1. One-paragraph summary

A single-endpoint application that takes a natural-language request, retrieves
authorized data from a tenant's Postgres database, and generates a
user-specific HTML + htmx interface for that data. Authorization is authored
and decided in Cedar and compiled into SQL predicates; Postgres enforces a
thin, role-independent floor. The product is not text-to-SQL; it is a
**personalized application generator operating over live, authorized data**.
The database query is the data-gathering phase; the main output is a
temporary, user-specific application surface.

The application runs as a Cloudflare Worker. That is already built and proved
(`spikes/workers-postgres/README.md`), and it constrains several choices below.

---

## 2. Goals and non-goals

### Goals

- Different users asking the same question receive **deliberately different
  interfaces** — driven by operational role, explicit preferences, learned
  behavior, device and request intent. Drift between users is the product.
- The LLM composes the whole content region, not a component choice.
- Authorization is correct by construction: the model cannot request what the
  policy forbids, and the database cannot return it even if the model tries.
- Every number shown is computed by the system, never by the model.
- The system converges toward determinism: once a (request, role, profile)
  has been served, it is cached and replayed without model involvement.

### Non-goals (v1)

- Reusing any application logic from the source ERP. AdventureWorks provides
  schema and data only.
- Model-authored DML. All writes are named actions.
- Self-serve policy editing UI.
- Production-grade tenant isolation (see §5).
- Leaving Cloudflare Workers. The runtime is fixed; the design bends to it.

---

## 3. Architecture as it exists **[exists]**

The repository is a Cargo workspace with one rule: **functional core,
imperative shell** (`CLAUDE.md`).

| Crate           | Target                   | Holds today                                              | Will hold                                                        |
| --------------- | ------------------------ | -------------------------------------------------------- | ---------------------------------------------------------------- |
| `crates/core`   | native                   | Session sealing, WorkOS callback parsing, cookie strings | DataPlan, PlanEdit, UI tree, Cedar residual → SQL, plan compiler |
| `crates/view`   | native                   | `maud` chrome (`layout::page`) and static pages          | UI-tree → HTML compiler, fixed chrome with ask input             |
| `crates/worker` | `wasm32-unknown-unknown` | axum router, Hyperdrive socket, WorkOS calls, clock, RNG | LLM calls, per-request transaction, token signing, caches        |
| `xtask`         | native                   | Lint gate, migration runner                              | Template/tenant migration fan-out, policy round-trip check       |

Consequences for this design:

- Every stage of the pipeline that is a **transformation of values** —
  validation, compilation, translation, rendering of a tree — lives in `core`
  or `view` and is tested with `cargo test`, natively, with no database and no
  Wasm toolchain.
- Every stage that **touches the world** — the LLM, Postgres, the clock — lives
  in `worker`. `state::now()` and `entropy::` are the only clock and RNG.
  Signing tokens (§4.4) needs randomness and time; the shell passes them in.
- `cargo check --workspace` fails by design. `cargo xtask lint` checks both
  targets. Any new dependency must build for `wasm32-unknown-unknown` if it is
  reachable from `worker`, and must build natively if it is in `core`/`view`.

### 3.1 What the scaffold already provides **[exists]**

- **Identity.** WorkOS AuthKit sign-in (`/auth/login`, `/auth/callback`,
  `/auth/logout`). `SessionClaims` carries `user_id` (WorkOS `sub`),
  `session_id`, `email`, `organization_id: Option<String>`, the WorkOS access
  and refresh tokens, and `expires_at`.
- **Session.** Claims sealed with ChaCha20-Poly1305 into one cookie. Unsealing
  is authentication; `unseal` returns `Expired` instead of a stale claim.
  Reading the cookie happens only in `extract::SignedIn` and
  `extract::Visitor`.
- **Database.** One Hyperdrive binding `DB`, one `tokio-postgres` connection
  per request over `worker::Socket` with `StartTls`. Hyperdrive holds the pool.
  `simple_query` is preferred where Hyperdrive caching may be off.
- **Migrations.** Numbered files in `migrations/`, applied by
  `cargo xtask migrate` inside one transaction per file with a
  `schema_migrations` ledger.
- **Failure model.** `Failure::message` is what the browser sees;
  `Failure::detail` is what the log sees. `Config` and `SessionKey` redact in
  `Debug`.
- **Lint gate.** `unwrap`/`expect` denied, `missing_docs` denied, 1000-line
  file cap, `too_many_arguments` allow banned.

---

## 4. Request pipeline

```
Natural-language request + SignedIn claims + acting context
    │
    ▼
[1] Authorize & build catalog      worker: load principal slice  → core: Cedar → catalog
    │
    ▼
[2] Plan                           worker: LLM call              → core: validate DataPlan
    │
    ▼
[3] Compile & execute              core: DataPlan + residual → SQL  → worker: Postgres (RLS floor)
    │
    ▼
[4] Structured result + shape      core: rows, types, aggregates, sample
    │
    ▼
[5] Render                         worker: LLM call              → core: validate UI tree
    │
    ▼
[6] Compile UI                     view: tree + data → HTML/htmx fragment; worker signs action tokens
    │
    ▼
Fragment swapped into fixed chrome (view::layout)
```

Stages 2 and 5 are the only LLM calls. Both are cached (§10). Stage 3 is
deterministic. The column on the right names the crate: the shell gathers and
calls; the core decides.

### 4.1 Inputs to rendering

| Context                | Determines                                                    |
| ---------------------- | ------------------------------------------------------------- |
| User request           | What the user is trying to accomplish now                     |
| Result characteristics | What presentation fits this data (shape, size, types)         |
| Application context    | Available actions, terminology, design language               |
| Application role       | What the user may access (authorization — applied by stage 3) |
| Operational role       | What is relevant to their work                                 |
| Personal profile       | How information should be presented                           |

Authorization and personalization are **separate inputs**. Personalization
never participates in authorization.

### 4.2 LLM calls from a Worker **[changed]**

Outbound HTTPS uses `worker::Fetch`, whose future is not `Send`. The scaffold
confines `send_wrapper::SendWrapper` to `routes::auth::post_json`. The LLM
client must follow the same rule: one `llm::complete` function in `worker`
that owns the wrapper around the fetch, and nothing else. Handlers call it;
they do not wrap anything themselves.

Streaming the render stage (SSE, `hx-ext="sse"`) is **unverified** on Workers.
It is listed in §14 and is not part of phase 1.

---

## 5. Three languages

The system is defined by three small, versioned languages. Each is a set of
`serde` types in `noal_core` with a derived JSON Schema; model output is
produced with structured/constrained output so invalid shapes cannot be
emitted, and is then **validated again** by the core before use. The schema is
a hint to the model; the core's validator is the authority.

### 5.1 DataPlan (read path)

A typed query description the compiler turns into SQL.

- References only catalog objects (tables/views/columns the acting principal
  may see).
- Explicit column projections only. `SELECT *` is not expressible.
- Joins restricted to declared relationships in the catalog.
- Aggregations, filters, sorts, limit.
- Function allowlist.

The compiler works from an AST it owns. It never accepts model-written SQL.

**[changed]** `pg_query` (libpg_query, C) does not build for
`wasm32-unknown-unknown`. Admitting model-written SQL through an AST allowlist
is therefore **not available** on this runtime. DataPlan is the only read path.
The open decision from v1 §13 is closed.

### 5.2 PlanEdit (interaction path)

A structured transformation applied to an existing validated DataPlan:
`add_filter`, `remove_filter`, `sort_by`, `drill_into`, `set_limit`. Follow-up
interactions are plan edits, not new questions. No LLM round-trip,
deterministic, and the resulting UI can be a patch to the existing tree.

### 5.3 UI tree (render path)

A safe DOM abstract syntax tree the server compiles to HTML + htmx.

```json
{
  "type": "document",
  "children": [
    { "type": "heading", "level": 1, "text": "Five customers require attention" },
    { "type": "paragraph", "tone": "summary",
      "text": "Total outstanding is {{sum(customers.outstanding_amount)|currency}}." },
    { "type": "repeat", "source": "customers",
      "layout": { "type": "stack", "density": "comfortable" },
      "template": {
        "type": "section",
        "children": [
          { "type": "heading", "level": 2, "binding": "customer_name" },
          { "type": "field", "label": "Outstanding", "binding": "outstanding_amount", "format": "currency" },
          { "type": "action", "label": "Open customer", "action": "open_customer",
            "arguments": { "customer_id": { "binding": "customer_id" } } }
        ]
      }
    }
  ]
}
```

Node families: document/section/heading/paragraph, table/list/cards/stack/grid,
field/badge/metric, chart, disclosure/tabs, form/filter, action, repeat,
conditional.

**Compile-time rules** (all in `noal_core`, all unit-tested):

- Every `binding` must exist in the result's type signature; `format` must
  match the column type.
- **Narrative text may not contain numeric literals.** Values are expressed as
  computed bindings (`sum`, `avg`, `count`, `pct_change`, `top(n)`, `min`,
  `max`) evaluated by the compiler. The model chooses what to say; the system
  supplies the numbers. Numbers written as words ("five") are not caught by
  this rule; see §14.
- Actions must exist in the action catalog for the acting principal; arguments
  bound to rows must reference values present in the result set.
- The tree compiles through `maud` in `noal_view`. `maud` escapes every string
  by default, so there is no HTML sanitization step: the tree is the
  allowlist, and the only way to emit a tag is for the compiler to have a match
  arm for it. The response carries a strict CSP (no inline scripts).

### 5.4 Actions and htmx

The model selects actions by name; it cannot invent endpoints. The compiler
emits:

```html
<button hx-post="/app" hx-target="#main-result" hx-swap="innerHTML"
        name="interaction" value="<signed-action-token>">Show only high-risk customers</button>
```

The token carries: plan ID, action name, arguments, principal, expiry,
signature. The server validates the signature and re-authorizes the action
before execution. The model controls interaction design; the application
controls meaning and authorization.

**[changed]** The scaffold already has one symmetric key (`SESSION_KEY`) and an
AEAD seal/unseal pair in `core::session`. Action tokens use the **same
mechanism with a separate key** (`ACTION_KEY`) rather than Ed25519: it is
already in the Wasm build, it is already tested, and no party outside noal
ever needs to verify a token. Expiry is checked against a `Timestamp` passed
in by the shell, as `session::unseal` does. Ed25519 is kept as an option if a
token ever has to be verified by another service.

---

## 6. Tenancy on Neon **[changed]**

### 6.1 Boundary

| Phase      | Tenant boundary                                                    | Notes                                                   |
| ---------- | ------------------------------------------------------------------ | ------------------------------------------------------- |
| Prototype  | Child branch of an immutable `template` branch in one Neon project | Copy-on-write, dedicated endpoint per branch            |
| Production | Dedicated Neon project per tenant                                  | Stronger isolation, independent quotas, restore window  |

The application sees only:

```rust
struct TenantDatabase {
    tenant_id: Uuid,
    connection: TenantConnection,      // see below
    schema_version: u32,
    isolation: IsolationModel,         // SharedProjectBranch | DedicatedProject
}
```

**Caveats to state honestly:** branch restore windows are project-scoped;
long-lived branches outside the restore window lose most copy-on-write savings;
branch allowances depend on plan. Do not promise independent backup policy per
tenant in the prototype.

### 6.2 Connecting from a Worker **[changed]**

The scaffold connects through **one** Hyperdrive binding (`DB` in
`wrangler.jsonc`). A Hyperdrive binding is a fixed connection string chosen at
deploy time. It cannot be selected per request from a registry row. This
conflicts with "resolve `TenantDatabase` → acquire pooled connection" in v1.

Options, in order of preference:

1. **Prototype: one tenant, one binding.** The `DB` binding points at one
   tenant branch. Phases 1–7 need nothing else. The `TenantDatabase` type
   exists in the code but has exactly one value.
2. **Named bindings per tenant.** `wrangler.jsonc` lists `DB_<slug>` for every
   tenant; the router maps tenant → binding name. Provisioning a tenant is a
   redeploy. Workable for tens of tenants, not hundreds.
3. **Direct connection without Hyperdrive.** `worker::Socket` to the tenant's
   endpoint with the connection string read from a secret store. Loses
   Hyperdrive's pooling and TLS termination; pays a full Postgres handshake per
   request. Use Neon's pooled endpoint to soften it.
4. **Workers for Platforms / dispatch namespace**, one Worker deployment per
   tenant with its own binding. Cleanest isolation, most infrastructure.

Phase 8 chooses between 2, 3 and 4. Until then the design assumes 1, and the
`TenantConnection` enum is the seam:

```rust
enum TenantConnection {
    Hyperdrive { binding: String },      // options 1 and 2
    Direct { secret_ref: SecretRef },    // option 3
}
```

**Invariant (unchanged):** the client never supplies a database name, project
ID, binding name or connection string.

### 6.3 Control plane

Separate Neon project, `platform` database, reached through its own Hyperdrive
binding (`PLATFORM`):

```sql
create table platform.tenant (
  id                    uuid primary key,
  slug                  text unique not null,
  isolation             text not null,          -- 'branch' | 'project'
  neon_project_id       text not null,
  neon_branch_id        text,
  connection_kind       text not null,          -- 'hyperdrive' | 'direct'
  connection_ref        text not null,          -- binding name or secret ref
  schema_version        int  not null,
  target_schema_version int  not null,
  provisioning_status   text not null,
  migration_status      text,
  last_migration_error  text,
  last_verified_at      timestamptz,
  created_at            timestamptz not null default now()
);

create table platform.membership (
  tenant_id       uuid not null references platform.tenant(id),
  workos_user_id  text not null,                -- SessionClaims.user_id
  workos_org_id   text,                         -- SessionClaims.organization_id
  app_user_id     uuid not null,                -- app.users.id inside the tenant DB
  primary key (tenant_id, workos_user_id)
);
```

**[changed]** v1 left the path from identity to tenant user unwritten. It is:
`SessionClaims.user_id` → `platform.membership` → `(tenant_id, app_user_id)`.
The WorkOS organization is a hint for which tenant to offer, not an
authorization fact on its own.

### 6.4 Template

The template is a **versioned logical artifact** in the repo, applied to the
`template` branch and to every tenant on migration:

```
template/
├── migrations/            ordered, idempotent; same format as ./migrations
├── seed/
│   ├── reference-data.sql
│   ├── roles.sql
│   └── demo-data.sql      AdventureWorks-derived, demo variant only
├── policies/              Cedar policies + schema
├── catalog/               semantic column descriptions
├── tests/                 pgTAP persona matrix
└── template-version
```

Two variants: `erp-schema` (empty operational DB) and `erp-demo` (schema +
AdventureWorks data + personas).

**[changed]** `cargo xtask migrate` already applies numbered files from
`migrations/` with a ledger. It gains a `--dir` flag so the same runner applies
`template/migrations/` to a tenant, and a `--tenant <slug>` mode in phase 8
that reads the control plane. The existing `./migrations` directory stays for
the `platform` database.

### 6.5 Provisioning

1. Create branch (or project) via Neon API.
2. Create database and service roles.
3. Apply migrations over the **unpooled** endpoint, from `xtask` (native, not
   from the Worker).
4. Load reference data; load demo data for demo variant.
5. Create initial tenant administrator (production) or personas (demo).
6. Run verification suite (§11).
7. Record versions in registry; mark ready.

### 6.6 Migration rollout

Every tenant evolves independently. Control plane tracks `current/target
schema_version`; an `xtask` fan-out applies migrations and re-runs
verification. Order for any new exposed table: create → enable + force RLS →
policies → indexes for predicate columns → grants → tests.

### 6.7 Routing

```
SignedIn (cookie unsealed) → membership lookup (PLATFORM)
  → resolve TenantDatabase → open connection (per §6.2)
  → begin transaction with user context → execute → commit
```

Expect a few hundred ms cold start on a scaled-to-zero tenant; the chrome
shows a skeleton.

---

## 7. Authorization

### 7.1 Principle

**Cedar decides. The compiler filters. Postgres guarantees invariants.**

Authorization is authored once, in Cedar. The same evaluation produces three
outputs: the schema catalog visible to the planner, the SQL predicates appended
to every query, and the set of actions available in the UI. Postgres holds a
small, role-independent floor so that the worst compiler bug exposes
role-appropriate tables to an authenticated user — never another tenant, never
PII, never unauthenticated data.

### 7.2 Cedar on the Worker **[changed]**

`cedar-policy` must compile for `wasm32-unknown-unknown` and its partial
evaluation feature is experimental. Neither is proved. **Phase 2 begins with a
spike** in `spikes/cedar-wasm/`, in the style of the Postgres spike, that
answers: does the crate build for the target; what does it add to the bundle
(the scaffold is 338 KB gzipped of a 3 MB limit); does partial evaluation
produce usable residuals for the policy shapes in §7.3.

If the crate does not fit, the fallback is to run evaluation **natively in
`xtask` at template build time**: every (role, action) residual is precomputed,
translated to SQL, and shipped as data. Per-request work is then substitution
of principal attributes into a prepared predicate. This keeps "authored once in
Cedar" and changes nothing in §7.4; it only moves *when* evaluation happens.

Either way, evaluation and translation are pure and live in `noal_core`.

### 7.3 Cedar model

- **Principals:** `User`, with parent groups for roles
  (`Role::"salesperson"`, `Role::"sales_manager"`, …). Attributes assembled per
  request from `app.users` and scope tables: `territories`, `customers`,
  `departments`, `employee_id`, `capabilities`.
- **Resources:** `Order`, `Customer`, `Store`, `Product`, `Employee`,
  `EmployeePrivate`, reporting views. Attributes mirror columns. No resource
  hierarchy for v1 — containment is expressed via attributes.
- **Actions:** read actions per resource type (`Action::"read_order"`),
  aggregate actions for reporting surfaces, and named write actions
  (`Action::"approve_order"`, `Action::"dismiss_customer"`).
- **Policies:** ABAC rules.

```cedar
permit(principal in Role::"salesperson", action == Action::"read_order", resource)
when { resource.territory in principal.territories };

permit(principal in Role::"sales_manager", action == Action::"read_order", resource)
when { resource.territory in principal.territories };

permit(principal in Role::"auditor", action == Action::"read_order", resource);

forbid(principal, action, resource is EmployeePrivate)
unless { principal.capabilities.contains("pii") };
```

**No resource IDs in the policy store.** Per-record grants live in scope tables
and surface as principal attributes or resource attributes. Policy templates
are not used in v1.

### 7.4 Read path: partial evaluation → SQL

1. Build the principal entity slice from the tenant DB (one query, shell).
2. Evaluate each read action with the resource unknown. Obtain residual
   policies over `resource.<attr>` (core).
3. Translate residuals to SQL predicates: `==` → `=`, `in` / `contains` →
   `= ANY(...)`, `&&`/`||` → `AND`/`OR`, resource-attribute set membership →
   `EXISTS` against the scope table, `forbid` residuals → `AND NOT (...)`
   (core).
4. Append predicates to every relation the DataPlan touches (core).

Any policy whose residual the translator cannot express is a **build-time
failure**, caught by the round-trip test (§11.2) in `xtask`.

**[added]** Because the Postgres floor (§7.6) gives no row isolation on its
own, the compiler carries an invariant test: for every relation in a compiled
plan, the emitted SQL contains that relation's predicate. A plan that reaches
a relation with no predicate attached does not compile. This is a unit test in
`noal_core`, not a database test.

### 7.5 Write path: full evaluation

Writes are named actions, never model-generated DML.

| Shape              | Mechanism                                                                                                                          |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| Single-row action  | Lock row (`SELECT … FOR UPDATE`), evaluate on pre-image, apply, evaluate on post-image, commit or rollback                          |
| Bulk action        | Residual → `WHERE` restricts target set; DML with `RETURNING`; evaluate every post-image; any violation rolls back                 |
| Field-level control| `context.changed_fields` passed as a set; compiler knows the set statically per action                                            |

Every write inserts an audit row: request ID, principal, action, resource,
post-image hash, and the permitting policy IDs from Cedar diagnostics.

### 7.6 Postgres floor

```sql
-- Service roles
create role schema_owner nologin;
create role app_query  login nosuperuser nobypassrls;   -- select on exposed surface
create role app_writer login nosuperuser nobypassrls;   -- DML only on tables with actions
create role migrator   login nosuperuser;               -- unpooled endpoint only

-- User context (fail closed)
create schema app;
create function app.uid() returns uuid language sql stable set search_path = '' as $$
  select nullif(pg_catalog.current_setting('app.user_id', true), '')::uuid
$$;

-- On every exposed table
alter table sales.salesorderheader enable row level security;
alter table sales.salesorderheader force row level security;
create policy require_context on sales.salesorderheader as restrictive
  for all to app_query, app_writer
  using (app.uid() is not null) with check (app.uid() is not null);
create policy allow_authenticated on sales.salesorderheader
  for select to app_query using (true);
grant select on sales.salesorderheader to app_query;

-- Sensitive tables: physically split, coarse capability gate
create policy require_pii on humanresources.employee_private as restrictive
  for select to app_query
  using (pg_catalog.current_setting('app.capabilities', true) like '%pii%');
```

What the floor does **not** do: it does not know what a territory is, there is
no `SET ROLE` per request, there are no role-specific policy functions. Column
security is achieved by physically splitting sensitive columns into tables that
`app_query` can only reach with the capability gate.

Reporting views for aggregate-only access are **definer** views
(`security_invoker = false`, owned by `schema_owner`). This deliberately
bypasses the base-table floor. Document this as intentional.

**[changed]** The Worker connects as `app_query` or `app_writer`, chosen by
which Hyperdrive binding (or secret) it uses. Two bindings per tenant
(`DB` read, `DB_WRITE` write) keep the read path physically unable to write.

### 7.7 Per-request transaction **[changed]**

Hyperdrive pools in transaction mode, as PgBouncer does, so all context is
transaction-local:

```sql
begin read only;                                    -- read path; writes use begin
select set_config('app.user_id', $1, true);
select set_config('app.capabilities', $2, true);
select set_config('app.request_id', $3, true);
set local statement_timeout = '5s';
set local lock_timeout = '500ms';
-- compiled, parameterized query with Cedar predicates appended
commit;
```

Session-level state is never relied upon. The scaffold opens one connection
per request and drops it; that matches this model exactly.

`set_config` with parameters is a prepared statement. The scaffold notes that
Hyperdrive with caching **off** cannot serve prepared statements. The
transaction preamble therefore uses `simple_query` with values escaped by the
core (`quote_literal` semantics, unit-tested), and the main query uses `query`
with parameters only when caching is known to be on. §14 lists this as an
item to verify against real Neon.

The 5 s statement timeout sits inside the Workers request budget; the Worker's
own deadline must be longer than the sum of both LLM calls and the query.

### 7.8 Planner catalog

Derived per (tenant schema version, principal role set, semantic catalog
version). Contains only relations and columns the Cedar policies can permit
for that principal, with descriptions from `template/catalog/`. The individual
user ID is not part of the catalog key; it belongs in result-cache keys.

---

## 8. Personalization

```sql
create schema personalization;
create table personalization.user_profile (
  user_id uuid primary key references app.users(id),
  version bigint not null default 1,
  profile jsonb not null default '{}'
);
```

Profile contents: information density, preferred list style, narrative detail,
evidence visibility, default sorting, chart preference, device context,
accessibility needs, locale, domain preferences, and learned signals.

Rules:

- Explicit preferences override inferred preferences.
- Profile version and authorization version are independent.
- htmx interactions are logged as signals; a background job (a Cloudflare
  Queue consumer or Cron Trigger in the same Worker) updates inferred
  preferences.
- **Saved interfaces:** a user can pin a generated surface. A saved interface
  = plan + tree + profile snapshot, re-executed with fresh data.

### 8.1 Fixed chrome **[exists]**

`noal_view::layout::page` already renders the chrome: `<head>` with pinned
htmx, masthead with sign-in/sign-out driven by `Viewer`. It gains the ask
input, the acting-role switcher, and the `#main-result` container. Only the
content region varies, and only the content region is ever returned to an
htmx swap.

---

## 9. Model interaction and safety

- **Planning prompt** receives the catalog, allowed actions, and the request.
  Never raw data.
- **Render prompt** receives the result type signature, row count,
  aggregates, and a bounded sample (full set only when ≤ N rows). Data is
  passed in a delimited structured block, never as prose.
- Injection through cell values is mitigated by: structured data framing,
  binding validation, and no numeric literals in narrative.
- The model's provider key is a Worker secret read by `Config::from_env`, with
  the same redacting `Debug` as `workos_api_key`.
- Streaming is deferred (§4.2).

---

## 10. Caching **[changed]**

A Worker isolate holds no state between requests. Every cache below is
external: **Cloudflare KV** for the model-output caches (plan, UI template),
whose values are small and read far more than written, and the tenant database
or KV with a short TTL for results. Cache keys are computed in `noal_core`;
reads and writes are in `worker`.

| Cache       | Key                                                  | Contents                       | Store |
| ----------- | ---------------------------------------------------- | ------------------------------ | ----- |
| Catalog     | tenant schema version + role set + catalog version   | visible schema for the planner | KV    |
| Plan        | normalized request + catalog key                     | validated DataPlan             | KV    |
| Predicates  | principal attribute hash + action                    | translated Cedar residuals     | KV    |
| UI template | plan ID + profile version + result shape             | validated UI tree              | KV    |
| Result      | plan ID + predicates hash + data version             | rows (short TTL)               | KV    |

**Request normalization** (needed for the plan key to hit): lowercase,
collapse whitespace, strip punctuation, and — in a later phase — an embedding
nearest-neighbour lookup over past requests. v1 ships the lexical
normalization only and measures the hit rate.

Once plan and tree are cached, a repeat request is a Postgres query and a
template fill.

---

## 11. Testing and verification

### 11.1 Persona matrix

Demo personas are real AdventureWorks people. For each (persona × exposed
relation) the expected row count is fixed and asserted via pgTAP through the
full path (Cedar → predicates → Postgres). Runs at provisioning and on every
template version, from `xtask` against a real database.

### 11.2 Policy round-trip

Every Cedar policy is partially evaluated and translated; any untranslatable
residual fails the build. Runs in `cargo xtask lint`.

### 11.3 Compiler tests (native, `cargo test`)

- DataPlan rejects: unknown objects, `*`, disallowed functions, undeclared
  joins.
- Every compiled relation carries its predicate (§7.4).
- UI tree rejects: unknown bindings, mismatched formats, numeric literals in
  narrative, actions outside the catalog, arguments not present in the result.

### 11.4 Postgres floor tests (pgTAP)

No context → zero rows; `app_query` cannot reach `employee_private` without
capability; `app_writer` has DML only on action tables.

### 11.5 LLM harness

Fixed dataset × personas × ~20 requests. Assertions: plan validity, binding
coverage, no invented fields, no numeric literals in narrative, action
arguments bound to real rows, render under size budget. Variability between
personas is measured, not suppressed. Runs natively from `xtask` with recorded
model responses so the gate is deterministic; a live run is opt-in.

---

## 12. Technology **[changed]**

| Concern             | v1 choice                    | Now                                                                   |
| ------------------- | ---------------------------- | --------------------------------------------------------------------- |
| Runtime             | Server process               | **Cloudflare Workers**, Wasm (exists)                                 |
| Language / web      | Rust, Axum, htmx             | Same (exists)                                                         |
| Identity            | —                            | **WorkOS AuthKit**, sealed cookie (exists)                            |
| DB access           | sqlx                         | **`tokio-postgres` over `worker::Socket` through Hyperdrive** (exists)|
| Authorization       | Cedar, partial eval pinned   | Same, pending Wasm spike; native precompute as fallback (§7.2)        |
| Schemas / validation| serde + `jsonschema`         | serde types in `noal_core` are the validator; `schemars` emits the schema for the model |
| HTML                | maud or askama + sanitizer   | **`maud`** (exists); tree compiler is the allowlist, no sanitizer     |
| Signing             | Ed25519                      | AEAD seal with a dedicated key, reusing `core::session` (§5.4)        |
| SQL admission       | `pg_query`                   | **Not available on Wasm.** DataPlan only.                             |
| Caches              | unspecified                  | Cloudflare KV (§10)                                                   |
| Background work     | unspecified                  | Cloudflare Queues / Cron Triggers                                     |
| Tests               | pgTAP, Rust integration      | `cargo test` for core/view; pgTAP and harness from `xtask`            |
| Base schema         | AdventureWorks Postgres port | Same, with PII/pay split and `reporting` schema                       |

Every new crate reachable from `worker` is checked with
`cargo check -p noal_worker --target wasm32-unknown-unknown` before it is
adopted.

---

## 13. Phased plan **[changed]**

0. **Scaffold.** Done. Workers runtime, Hyperdrive, WorkOS session, chrome,
   migrations, lint gate, Postgres spike.
1. **Render-only spike.** Hard-coded result sets, real LLM through one
   `llm::complete`, UI tree v0 in `core`, compiler in `view`, ask input and
   `#main-result` in the chrome. Measure how variability feels across three
   personas. Needs no database change.
2. **Cedar Wasm spike**, then **template repo**: AdventureWorks on the
   `template` branch, splits, `app`/`personalization`/`reporting` schemas,
   floor policies, pgTAP matrix. `xtask migrate --dir`.
3. **Cedar integration.** Schema, policies, entity slicing, residual
   translator, round-trip test, predicate-coverage test.
4. **DataPlan + compiler.** Catalog derivation, planning prompt, execution
   inside the per-request transaction, result shape.
5. **End-to-end read path with KV caching.** Streaming if §14 clears it.
6. **PlanEdit interactions and saved interfaces.** Action tokens.
7. **Write actions with audit.** `DB_WRITE` binding.
8. **Tenant connection model** (§6.2), control plane, provisioning via Neon
   API, migration fan-out.

---

## 14. Open decisions

Carried from v1:

- Acting-role switch in the UI for presentation purposes, given Cedar
  composition handles multi-role users.
- Reporting surfaces: live views vs. materialized, and refresh cadence.
- Full-result threshold for the render prompt (N rows).
- Whether `pg_session_jwt` replaces `app.user_id` in production.

Closed by the scaffold:

- ~~DataPlan only, or a `pg_query`-admitted SQL subset.~~ DataPlan only.

New, from reconciliation:

- **Tenant connection model** (§6.2): bindings per tenant, direct sockets, or
  Workers for Platforms. Decide in phase 8.
- **Cedar on Wasm** (§7.2): in-Worker evaluation or native precompute. Decide
  after the phase 2 spike.
- **SSE streaming from a Worker** for the render stage. Unverified.
- **Hyperdrive caching and prepared statements** in the transaction preamble
  (§7.7). Verify against real Neon; the spike never did.
- **Numbers as words** in narrative. Accept, or extend the rule with a
  number-word list.
- **Request normalization** beyond lexical (§10). Measure first.

---

## 15. Invariants (non-negotiable)

1. Tenant is selected before connecting; user is established transactionally
   after.
2. No client input ever names a database, project, role, binding, or
   connection.
3. Generated SQL is read-only; writes are named, sealed, audited actions.
4. Authorization is authored only in Cedar; Postgres enforces only
   role-independent invariants.
5. The planner sees only what policy can permit; the database is a backstop,
   never the first line.
6. Narrative text contains no model-generated numbers.
7. All request context is transaction-local.
8. Personalization never participates in authorization.
9. `noal_core` and `noal_view` stay pure: every rule in this document that
   validates, compiles or translates is testable with `cargo test` and no
   database.
10. Unsealing the session cookie is authentication. No handler reads the
    cookie except through `extract::SignedIn` / `extract::Visitor`.
