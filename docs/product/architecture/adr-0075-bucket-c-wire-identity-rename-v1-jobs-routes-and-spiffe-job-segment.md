# ADR-0075 — Bucket C wire/identity rename: `/v1/jobs*` HTTP routes → `/v1/workloads*`, and the SPIFFE SVID path segment `/job/` → `/workload/`

## Status

Accepted. 2026-07-11. Decision-makers: user (ratified both decisions
before this record — research §5 Q2 "REFACTOR approved" and Q3
"REFACTOR approved", 2026-07-01); Morgan (recording the decisions +
pinning the implementation contract). Mode: propose (recording a
user-ratified decision). Tags: phase-2, terminology-migration,
application-arch, wire-contract, workload-identity, single-cut.

This ADR records the **last step** of the job→workload terminology
migration (research:
`docs/research/refactoring/job-vs-workload-terminology-comprehensive-research.md`,
Bucket C). Buckets A (legitimate `WorkloadKind::Job` kind tokens — KEEP)
and B (internal behavior-preserving renames) are already landed. Bucket C
is the two remaining tokens that cross a boundary a client or an issued
SVID observes:

- **C1** — the operator HTTP route family `/v1/jobs*`.
- **C2** — the SPIFFE SVID path segment `/job/` (a fixed grammar segment
  applied to **every** workload kind, NOT the kind discriminator).

Both are recorded here rather than in two ADRs because they are one
ratified decision-family (finish the Nomad→Kubernetes umbrella-noun
migration for the wire/identity surface), share one Context (single-cut
greenfield, single in-tree HTTP client, ephemeral kernel-held SVIDs = no
data migration), and land in one cut. Their prior-ADR entanglements
differ (C1 → ADR-0008/ADR-0014; C2 → ADR-0067/ADR-0072), so each decision
carries its own cross-link/amendment block below.

**Amends ADR-0008** (REST + OpenAPI transport — the endpoint table) and
**ADR-0014** (CLI HTTP client + shared types — the client route strings /
shared-type prose). The route *family* moves from `/v1/jobs*` to
`/v1/workloads*`; the transport, framework, TLS, HTTP-version, and
`/v1`-prefix decisions those ADRs pin are otherwise unchanged. See
*Cross-links → C1* for the exact amendment markers.

**Cross-checks ADR-0067** (workload identity manager / `SvidLifecycle` /
`SpiffeId::for_allocation`) and **ADR-0072** (dial-by-name responder). The
outcome of the cross-check (recorded in *Cross-links → C2*): **neither ADR
pins `/job/` as load-bearing beyond what `SpiffeId::for_allocation` (the
sole producer) and `workload_of` (the sole parser) own** — so C2 is a
producer + parser + fixtures rename in one cut, with no authz/policy/data
lockstep beyond the single shared parser. This ADR **does not supersede**
either — it renames one grammar segment they both reference.

## Context

### The migration this closes

Overdrive's operator surface was modelled on **HashiCorp Nomad**, where
"job" is the umbrella noun for any submitted unit. When the
workload-kind-discriminator feature (ADR-0047) introduced
`WorkloadKind::{Service, Job, Schedule}`, the model shifted to the
**Kubernetes** shape — "workload" is the umbrella and "Job" is one
run-to-completion kind among several. The `WorkloadId` rename
(`17f633e2`) and the intent-key rename to `workloads/<id>` (ADR-0050
OQ-5) advanced that migration; Buckets A/B finished the internal and
kind-token work. **Two Nomad-era generic "job" tokens survive on
boundaries a client or an issued cert observes** — the C1 route family and
the C2 SVID path segment. Leaving them permanently bakes the Nomad-era
umbrella noun into the public HTTP surface and into every workload's
cryptographic identity string, keeping the ubiquitous language ambiguous
("job-the-kind or job-the-unit?") at exactly the two most visible places.

### Why each is decision-gated (and why the cut is clean)

Both tokens cross a boundary, so neither is a free internal refactor — but
both are single-cut clean under the project's greenfield-migration rule:

- **C1 is an HTTP wire contract.** Any external client hitting `/v1/jobs`
  breaks. But the **only in-tree client** is `overdrive-cli`
  (`http_client.rs`) — verified single consumer per ADR-0014's shared-type
  design (the CLI imports the server's request/response types directly; no
  generated SDK, no second client). Server + client move in one commit.
  Per research §5 Q2, **no external HTTP consumer is in Phase-2 scope** —
  so the single-cut greenfield rule permits a clean rename with no dual
  `/v1/jobs`-and-`/v1/workloads` compat window.

- **C2 is an issued-certificate identity string.** Every SVID's SPIFFE ID
  literally contains `/job/`. But mTLS is **kernel-mediated and SVIDs are
  ephemeral** — workloads hold no SVID material, the held `SvidMaterial`
  lives in-process in `IdentityMgr` and is re-minted on every
  control-plane restart (ADR-0067 D1 "restart recovery"; root `CLAUDE.md`
  § "Workload identity model"). The durable `issued_certificates` audit
  row carries only facts (`spiffe_id, serial, …`), no cert bytes and no
  key — it cannot reconstruct a usable SVID and is not consumed as a
  live credential. **There is therefore no durable cert store to
  migrate.** The change is: the SVID string producers (two live sites —
  see the *Implementation contract* correction below), one parser, and
  the test/doc fixtures — all in one cut.

  **Knowledge gap CLOSED (recorded).** The research §5 Q3 flagged a gap:
  does any Rego/policy file or dataplane match key on the literal `/job/`
  segment (which would have to move in lockstep)? The answer, verified for
  this ADR: **no.** There are no `.rego`/policy files keying on `/job/`,
  and no dataplane match on the literal segment. The **only**
  authz-adjacent consumer of the segment is the single parser `workload_of`
  (`name_index.rs:130`), which is imported and reused by **both** the DNS
  dial-by-name index **and** the mTLS resolve adapter
  (`mtls_resolve_adapter.rs:208`). One parser, two consumers, one match arm
  to change.

### Non-scope — tokens that contain "job" but are NOT C1/C2 (KEEP)

Recorded so the crafter does not over-reach. These are Bucket A kind
tokens or the ratified-KEEP `--job` CLI flag; **none change**:

- `WorkloadKind::Job` wire token `"job"` / the `[job]` TOML section body /
  the `b'j'` discriminator byte (`workload_spec.rs:376,388,919`), and the
  `kind: "job"` serde discriminator in the polymorphic describe response.
  These are the **kind** discriminator (Bucket A). Renaming them would
  erase the kind distinction ADR-0047 introduced.
- The `?job=<id>` query param on `GET /v1/allocs` and its server-side
  `field = Some("job")` validation name (`http_client.rs:315,329`;
  `handlers.rs` validation surface). These back the **ratified-KEEP
  `--job` CLI flag** (research §5 Q1 — CLI verb KEEP). KEEP. *(Note the
  route these hang off — `GET /v1/allocs` — is itself untouched by C1;
  only the `/v1/jobs*` family moves.)*

### Constraints (locked)

- **Single-cut greenfield.** No dual-route compat, no `/job/`-and-
  `/workload/` both-accepted grammar, no shim, no `pub use` alias, no
  deprecation window (`CLAUDE.md` § single-cut greenfield migrations;
  `feedback_single_cut_greenfield_migrations.md`). Removed is removed.
- **Implement to the design — never invent API surface** (`CLAUDE.md`).
  This is a pure rename: the crafter changes the enumerated string
  literals and their fixtures. No new route, no new SVID grammar segment,
  no new parser arm beyond the single `Some("job")` → `Some("workload")`
  swap, no signature change.
- **rkyv discipline (C2 boundary check).** `SpiffeId` is `rkyv::Archive`
  (`id.rs:252`), but its archived layout is `{ canonical: String,
  path_start: usize }` — the archived value is the **whole canonical
  string**, not a positional encoding of path segments. Changing the
  *content* of the string (`/job/` → `/workload/`) does **not** change the
  archived *layout* and is therefore **not** an rkyv schema-evolution
  event. No envelope bump, no golden-bytes fixture. (Any durable
  `issued_certificates` rows from a prior boot carry the old `/job/`
  string; per the ephemeral-SVID + greenfield model there is no requirement
  to read them back as live identities, and the greenfield upgrade path is
  "delete the on-disk redb file" — `.claude/rules/development.md` §
  "Migration policy is greenfield single-cut".)

## Decision

**Rename both remaining Bucket-C wire/identity tokens, single-cut, no
shim:**

- **C1** — the operator HTTP route family and its committed OpenAPI spec
  move from `/v1/jobs*` to `/v1/workloads*`:
  - `POST /v1/jobs`               → `POST /v1/workloads`
  - `GET /v1/jobs/{id}`           → `GET /v1/workloads/{id}`
  - `POST /v1/jobs/{id}/stop`     → `POST /v1/workloads/{id}/stop`
  - `POST /v1/jobs/{id}/restart`  → `POST /v1/workloads/{id}/restart`

  The `GET /v1/allocs`, `GET /v1/nodes`, `GET /v1/cluster/info` routes are
  **unaffected**. The OpenAPI `tag` is already `"workloads"` (renamed in
  Bucket B) — no tag change.

- **C2** — the SPIFFE SVID path segment moves from `/job/` to
  `/workload/` for **every** kind:
  - `spiffe://overdrive.local/job/{workload}/alloc/{alloc}`
    → `spiffe://overdrive.local/workload/{workload}/alloc/{alloc}`

The *Implementation contract* section below is the binding, design-
sensitive surface. Per `CLAUDE.md` § "Implement to the design — never
invent API surface", the crafter renames exactly the string literals,
route strings, and fixture literals pinned there — nothing else.

## Alternatives considered

### Alternative A — Keep both Nomad-era tokens (do nothing)

Leave `/v1/jobs*` and `/job/` as-is; treat the surviving generic "job" as
harmless legacy.

**Rejected (and the user ratified rejecting it).** It permanently bakes
the Nomad-era umbrella noun into the two most visible surfaces — the
public HTTP contract and every workload's cryptographic identity — leaving
the ubiquitous language ambiguous exactly where a reader is most likely to
hit it (a `curl` against `/v1/jobs`, an SVID string in a log line). The
whole point of Buckets A/B was to reserve "job" for `WorkloadKind::Job`;
stopping short of C leaves the migration visibly half-done on the wire.

### Alternative B — Dual-accept both old and new (compat window)

Serve `/v1/jobs*` **and** `/v1/workloads*`; accept both `/job/` and
`/workload/` in the parser during a deprecation window.

**Rejected — violates single-cut greenfield.** The project rule is
explicit: no dual old/new paths, no deprecation windows, no
feature-flagged compat (`CLAUDE.md` § single-cut greenfield;
`feedback_single_cut_greenfield_migrations.md`). Dual-accept is only worth
its cost when an external consumer must be migrated across a window —
research §5 Q2 confirms there is no external HTTP consumer in scope, and
the C2 parser has exactly one internal caller-shape (`workload_of`, two
consumers). A both-accepted SVID grammar would additionally weaken the
identity contract (two valid encodings of one identity) for zero benefit.

### Alternative C — Split into two ADRs (routes vs identity grammar)

Author ADR-0075 for C1 (routes) and ADR-0076 for C2 (SVID grammar).

**Rejected — one cohesive record is the better fit.** Both are the same
ratified decision-family (finish the umbrella-noun migration on the
wire/identity surface), share one Context (single-cut, single in-tree
client, ephemeral SVIDs → no migration), and land in one cut. The only
argument for splitting is the differing prior-ADR entanglement (C1 →
0008/0014; C2 → 0067/0072) — but that is handled cleanly by two delineated
decision sections with their own cross-link blocks, exactly as ADR-0074
recorded a decision + coupled sub-decision in one record. Splitting would
duplicate the shared Context and fragment the "last step of the migration"
narrative across two files.

### Alternative D (C2-specific) — Rename only the producer, teach the parser to strip a variable segment

Change `for_allocation` to emit `/workload/` but keep `workload_of` matching
on *any* segment name (position-based, not literal `"job"`/`"workload"`).

**Rejected.** It would make the parser silently accept malformed or
mismatched SVID grammars, eroding the exact-match contract that makes the
identity string a reliable key. The single-cut fix — producer emits
`/workload/`, parser matches `Some("workload")` — keeps the grammar
literal and exact on both ends, which is what an identity contract
requires. Position-based parsing is the "make it lenient to dodge a
lockstep" anti-pattern; the lockstep here is one match arm.

## Implementation contract

Pin these exactly. The crafter implements to this shape and does not
re-decide any of it. **Both C1 and C2 land in the same PR**, single-cut,
no shim — there is no intermediate commit where a route or the SVID
grammar is served under both names.

### C1 — `/v1/jobs*` → `/v1/workloads*` (routes + spec + client + tests)

Rename the four route strings from `/v1/jobs...` to `/v1/workloads...`
across the surfaces below. The handler *function* names
(`submit_workload` / `describe_workload` / `stop_workload` /
`restart_workload`) and the OpenAPI `tag = "workloads"` are **already
correct** (Bucket B) — do NOT touch them; only the path literals move.

1. **Server router** — `crates/overdrive-control-plane/src/lib.rs:2330-2333`.
   axum `:id` param syntax. Change the four `.route("/v1/jobs...", ...)`
   strings to `/v1/workloads...`:
   ```rust
   .route("/v1/workloads",              post(handlers::submit_workload))
   .route("/v1/workloads/:id",          get(handlers::describe_workload))
   .route("/v1/workloads/:id/stop",     post(handlers::stop_workload))
   .route("/v1/workloads/:id/restart",  post(handlers::restart_workload))
   ```
   The three untouched routes (`/v1/allocs`, `/v1/nodes`,
   `/v1/cluster/info`) stay verbatim.

2. **`#[utoipa::path]` attrs** — `crates/overdrive-control-plane/src/handlers.rs`
   at `path = "/v1/jobs..."` on the four handlers: `:196` (submit),
   `:649` (describe), `:784` (stop), `:883` (restart). utoipa `{id}`
   syntax. Change each `path = "/v1/jobs..."` to `/v1/workloads...`. Also
   update the **prose** in the same file that names the routes: the
   module-doc route table at `handlers.rs:7-8`, the fn docstrings at
   `:160`, `:185`, `:628`, `:759-760`, `:856`, and the **AIP-136 stop
   note** at `:762-768`. The AIP-136 note carries over **verbatim except
   the noun** — `/v1/jobs/{id}:stop` → `/v1/workloads/{id}:stop`,
   `/v1/jobs/:id:stop` → `/v1/workloads/:id:stop`,
   `/v1/jobs/{id}/stop` → `/v1/workloads/{id}/stop`; the AIP-136-vs-
   subsegment rationale (axum matchit treats `:` as the param prefix) is
   unchanged. The 404-contract doc at `:776-779` and idempotency doc at
   `:770-773` reword "job" prose to "workload" where they mean the unit.

3. **Committed OpenAPI spec** — `api/openapi.yaml`. Regenerate via
   `cargo openapi-gen` (do NOT hand-edit the paths). The spec derives from
   the utoipa attrs (step 2) and the shared types, so regeneration moves
   the path keys at `~:85` (`/v1/jobs`), `~:150` (`/v1/jobs/{id}`), `~:201`
   (`/v1/jobs/{id}/restart`), `~:264` (`/v1/jobs/{id}/stop`) to
   `/v1/workloads*` and updates every prose `POST /v1/jobs` / `GET
   /v1/jobs/{id}` description string that flows from the fn docstrings.
   **Verify** the regenerated spec after: it must contain
   `/v1/workloads`, `/v1/workloads/{id}`, `/v1/workloads/{id}/stop`,
   `/v1/workloads/{id}/restart` and **no** remaining `/v1/jobs` path key.
   *(Note: one description at `~:1985` already says `POST /v1/workloads` —
   a stale-forward reference that regeneration reconciles with the rest.)*

4. **In-tree HTTP client** — `crates/overdrive-cli/src/http_client.rs`.
   The **only** client (ADR-0014 single-consumer shared-type design), so
   it moves in the same cut. Change the route strings and their docstrings
   at `:183` / `:212` (`POST /v1/jobs` submit + streaming), `:265`
   (`POST /v1/jobs/{id}/stop`), `:279` (`POST /v1/jobs/{id}/restart`),
   `:296` (`GET /v1/jobs/{id}` describe) to `/v1/workloads...`. **Do NOT**
   touch the `?job=` query-param append at `:329` or the `GET
   /v1/allocs?job=` doc at `:315` — that is the ratified-KEEP `--job` flag
   surface (Non-scope). Only the `/v1/jobs*` path family moves.

5. **The `openapi_gate.rs` hardcoded expected-path assertion** —
   `crates/overdrive-control-plane/tests/integration/openapi_gate.rs:50`.
   The `expected` array is `["/v1/jobs", "/v1/jobs/{id}", "/v1/allocs",
   "/v1/nodes", "/v1/cluster/info"]`. Change the two job entries to
   `"/v1/workloads"` and `"/v1/workloads/{id}"`. Leave the other three.

6. **Tests referencing `/v1/jobs` URLs (~15 files).** Every integration/
   acceptance test that hard-codes a `/v1/jobs` request URL moves in
   lockstep. The named set from the research:
   `submit_round_trip`, `describe_round_trip`, `dns_responder_*` (walking
   skeleton + ping-pong), `server_lifecycle`, `idempotent_resubmit`,
   `concurrent_submit_toctou`, `canonical_address_inbound_walking_skeleton`,
   `backend_discovery_bridge`, `service_submit_dispatch_wiring`,
   `streaming_submit`, `job_stop_intent_key`, `job_stop_idempotent`,
   `convergence_loop_spawned_in_production_boot`, plus `openapi_gate` (step
   5). **Canonical enumeration before renaming** (the research did not read
   all sites): run
   `rg -n '/v1/jobs' crates/` and change every request-URL string literal
   to `/v1/workloads*`. **Note the test-file/function NAMES containing
   "job" are Bucket-B classification, not C1** — e.g.
   `job_stop_intent_key` / `job_stop_idempotent` name the *unit*
   generically and were already handled (or handled) under Bucket B; C1
   changes only the **URL string literals inside** them, not the fn names.
   Do not rename any test fn as part of C1.

### C2 — SPIFFE `/job/` → `/workload/` (producers + single parser + fixtures + docs)

The load-bearing lockstep. Change **both live producers** and the **one
parser** together; every fixture literal follows mechanically. If a
producer emits `/workload/` while any fixture still emits `/job/`, that
fixture's SVID no longer resolves through `workload_of` (or mismatches the
producer) — so the fixtures MUST move in the same cut.

1. **Canonical PRODUCER** — `crates/overdrive-core/src/id.rs:322`,
   `SpiffeId::for_allocation`. Change the `format!` literal:
   ```rust
   // from:
   format!("spiffe://overdrive.local/job/{}/alloc/{}", workload.as_str(), alloc.as_str())
   // to:
   format!("spiffe://overdrive.local/workload/{}/alloc/{}", workload.as_str(), alloc.as_str())
   ```
   This is the ADR-0067 D5 canonical constructor.
   **`SpiffeId::new` does NOT hardcode or assert `/job/`** (verified:
   `id.rs:268-282` validates only scheme + non-empty trust-domain +
   non-empty path — it never inspects the `/job/` segment). So there is no
   validator arm to change. Update the docstring path illustrations on
   `for_allocation` (`id.rs:302,311`, the `unreachable!` message
   `:325-328`) and the `SpiffeId` type doc example (`id.rs:238`) from
   `/job/...` to `/workload/...`.

1b. **SECOND live PRODUCER — a hand-rolled duplicate that ADR-0067 D5's
   consolidation missed** — `crates/overdrive-control-plane/src/reconciler_runtime.rs:2938-2943`.
   This is a **production convergence path** (backend-identity derivation
   for the dataplane backend set, NOT a `#[cfg(test)]` block) that
   hand-rolls the same string via `SpiffeId::new(&format!("spiffe://
   overdrive.local/job/{}/alloc/{}", workload_id, row.alloc_id))?` instead
   of calling `for_allocation`. It MUST move in the same cut — otherwise a
   live backend SVID keeps emitting `/job/` while the renamed parser
   expects `/workload/`, silently breaking backend resolution at runtime
   with a green compile. **Two correct fixes, crafter's choice, but the
   result must be one grammar:**
   - **Preferred** — replace the hand-rolled `format!` with a call to
     `SpiffeId::for_allocation(&workload_id, &row.alloc_id)`, completing the
     ADR-0067 D5 single-producer consolidation this site escaped. This is
     the honest fix (one producer, per D5's intent) and removes the drift
     permanently. Note the `?`/`map_err(ConvergenceError::TargetShape)` at
     `:2943` becomes unnecessary — `for_allocation` is infallible (`-> Self`).
   - **Minimal** — change the literal in place (`/job/` → `/workload/`),
     matching step 1. Acceptable for a pure single-cut rename, but leaves
     the duplicate producer; prefer the D5 consolidation.
   Also update the stale comment at `:2936` referencing the
   `mint_alloc_identity` SPIFFE shape. **This is API-preserving either way**
   — no new public surface; `for_allocation` already exists (ADR-0067 D5).
   *(This site is why the enumeration below is `rg`-driven, not a fixed
   file list: ADR-0067 D5 claimed a single producer, but the live tree has
   two. Trust the grep, not the prior "single producer" claim.)*

2. **Single canonical PARSER** — `crates/overdrive-control-plane/src/dns_responder/name_index.rs:137`,
   inside `workload_of`. Change the single match arm:
   ```rust
   // from:
   Some("job") => break segments.next()?,
   // to:
   Some("workload") => break segments.next()?,
   ```
   Update the same fn's doc/inline comments that describe the
   `/job/<job>/alloc/<alloc>` shape (`name_index.rs:127,131-133`) to
   `/workload/<workload>/alloc/<alloc>`. **This is the entire logic
   lockstep** — `mtls_resolve_adapter.rs:208` imports and reuses this exact
   `workload_of` (verified), so DNS dial-by-name AND mTLS resolve share
   this one parser. Update any `/job/`-shape prose in
   `mtls_resolve_adapter.rs` comments to `/workload/`. There is **no second
   parser** to change.

3. **Test / fixture literals — all move in lockstep.** Every
   `spiffe://overdrive.local/job/…/alloc/…` literal across the workspace
   becomes `/workload/`. **Canonical enumeration** (broader than the
   research's representative list — ~72 files carry the literal): run
   ```
   rg -l 'spiffe://overdrive\.local/job/' crates/
   ```
   and change every match. The named high-signal anchors the research
   pinned (all in this set, none exhaustive):
   - `overdrive-core`: `src/id.rs` (tests + docs), `src/traits/ca.rs`,
     `src/reconcilers/svid_lifecycle.rs`,
     `src/reconcilers/service_map_hydrator.rs`,
     `src/dataplane/fingerprint.rs`, and the `tests/acceptance/*` +
     `tests/*.rs` fixtures under it.
   - `overdrive-control-plane`: `src/identity_mgr.rs`,
     `src/reconciler_runtime.rs`, `src/dns_responder/name_index.rs`,
     `src/mtls_resolve_adapter.rs`, and the `tests/integration/*` +
     `tests/acceptance/*` fixtures (DNS, mtls, svid, service-lifecycle,
     listener-fact, identity-mgr).
   - `overdrive-sim`: the invariant fixtures
     (`src/invariants/*.rs`) and `tests/acceptance/*` (identity-read,
     mtls-enforcement equivalence).
   - `overdrive-host`: `tests/integration/rcgen_ca_chain_verify.rs`.
   - `overdrive-dataplane`, `overdrive-worker`, `overdrive-cli`: the
     `tests/**` reverse-nat / e2e / alloc-status fixtures.

   The invariant to hold: after the change, `rg
   'spiffe://overdrive\.local/job/' crates/` returns **zero** hits, and
   `rg 'spiffe://overdrive\.local/workload/' crates/` returns the full
   fixture set. A single surviving `/job/` fixture breaks resolution
   against the renamed producer/parser.

4. **Doc/prose path illustrations** — `/job/<job>/alloc/<alloc>` shape
   references become `/workload/<workload>/alloc/<alloc>` in `id.rs`,
   `name_index.rs:131`, and `mtls_resolve_adapter.rs` comments (covered by
   steps 1-2 above). Also fix any ADR-adjacent doc illustration only if it
   is a live code-shape claim; ADR *records* keep their historical text
   (see Cross-links).

5. **Explicitly NOT changed under C2:**
   - `TargetResource::new("job/<workload_id>")` broker keys (ADR-0067 D5b;
     `workload_lifecycle.rs`, `exit_observer.rs`). This is an **internal
     reconciler broker-routing key**, not the SVID path segment and not a
     wire/identity boundary — it is Bucket-B-adjacent internal naming and
     is **out of scope for this ADR**. Do not touch it here (renaming it is
     a separate internal-naming concern, not a wire/identity change).
   - The `Some("job")` → `Some("workload")` change is the parser's literal
     match; do NOT also make the parser position-based or lenient
     (Alternative D, rejected).

### Single-cut greenfield (both C1 + C2)

No shim, no dual-route, no both-accepted grammar, no `pub use` alias, no
deprecation window. The router change, the utoipa/spec regen, the client
change, the C1 test-URL updates, the C2 producer + parser + fixture
changes, and all doc/prose updates land in the **same PR**. There is no
intermediate commit where a route or the SVID grammar is reachable under
both the old and new name. Removed is removed (`CLAUDE.md` § single-cut
greenfield; `feedback_single_cut_greenfield_migrations.md`).

## Consequences

### Positive

- **Ubiquitous language finished on the wire/identity surface.** "job"
  now means `WorkloadKind::Job` everywhere, including the public HTTP
  contract and every SVID identity string. A reader hitting `/v1/workloads`
  or seeing `spiffe://overdrive.local/workload/…` no longer has to
  disambiguate "job-the-kind vs job-the-unit."
- **The migration is complete.** Buckets A + B + C are landed; no
  Nomad-era generic "job" survives on any observed boundary. The
  `rg 'spiffe://overdrive\.local/job/'` and `rg '/v1/jobs' crates/`
  invariants (both → zero) become standing regression guards.
- **One SSOT per boundary preserved.** C1 moves one route family with the
  server and its single in-tree client in lockstep (no drift possible —
  shared types, ADR-0014). C2 moves both SVID-string producers + one
  parser; the two parser consumers (DNS + mTLS) update through the single
  shared `workload_of`. (The preferred fix folds the second producer into
  the ADR-0067 D5 `for_allocation` constructor, completing the
  single-producer consolidation D5 intended — see contract C2 step 1b.)

### Negative

- **Breaking for any hypothetical external HTTP client** hitting
  `/v1/jobs*`. **None is in scope** (research §5 Q2 — no external consumer
  in Phase 2; the only in-tree client moves in the same cut). If an
  external consumer is ever added *before* this lands, it must target
  `/v1/workloads*` from day one — there is no compat window by design.
- **Every newly-issued SVID's identity string changes** from `/job/` to
  `/workload/`. Because SVIDs are ephemeral + kernel-held and re-minted on
  restart (ADR-0067 D1; no durable cert store), no live credential is
  invalidated by a data migration — the next issuance simply emits the new
  string. Any prior-boot `issued_certificates` **audit rows** carry the
  old `/job/` string as a historical fact; they are audit records, not live
  identities, and the greenfield upgrade path (delete the on-disk redb
  file) already governs stale durable state. No cross-version SVID
  reconciliation is required.
- **Two `rg`-verified invariants must stay green.** The single-cut nature
  means a *partial* landing (producer renamed, one fixture missed) breaks
  SVID resolution in tests. The contract's zero-hit `rg` checks (C2 step 3)
  are the crafter's structural guard against a partial cut.

### Quality-attribute impact

- **Maintainability — analyzability**: positive. One noun for the umbrella
  concept across the wire and identity surfaces; no context-dependent
  disambiguation when reading a route, an SVID, or a fixture.
- **Compatibility — interoperability**: neutral-to-slightly-negative in
  the abstract (a public route family changes), but **neutral in practice**
  — the OpenAPI spec is regenerated so any future generated SDK derives
  the new paths, and there is no external consumer to break.
- **Security — integrity / authenticity**: neutral. The SVID grammar stays
  exact-match on both producer and parser (Alternative D rejected); the
  identity contract's strength is unchanged, only the segment spelling
  moves. Kernel-mediated mTLS enforcement is untouched — `workload_of` (the
  one authz-adjacent consumer) changes one literal, and both its consumers
  (DNS index, mTLS resolve) update through it.
- **Reliability — recoverability**: neutral. Restart recovery (ADR-0067
  D1) re-mints SVIDs with the new grammar; no persisted identity to
  reconcile across the rename.

## Cross-links

### C1 — HTTP route family (amends ADR-0008 / ADR-0014)

- **ADR-0008** (REST + OpenAPI transport). Its *Decision* endpoint table
  and its 2026-04-26 (ADR-0020) amendment name the routes as `POST
  /v1/jobs`, `GET /v1/jobs/{id}`. **Amended by this ADR**: the workload
  route family is `POST /v1/workloads`, `GET /v1/workloads/{id}`, `POST
  /v1/workloads/{id}/stop`, `POST /v1/workloads/{id}/restart`. The
  transport (axum/hyper/rustls), HTTP-version posture (ALPN `h2,
  http/1.1`), and `/v1` prefix decisions are **unchanged** — only the noun
  in the path moves. ADR-0008's response-shape amendments (ADR-0020
  `spec_digest` / `IdempotencyOutcome`) are unaffected. *(A reciprocal
  amendment marker in ADR-0008's Status block, pointing at this ADR, is the
  architect's follow-up edit — the endpoint table there is now historical
  for the `/v1/jobs*` spelling.)*
- **ADR-0014** (CLI HTTP client + shared types). Its client-method prose
  references `POST /v1/jobs` etc. **Amended by this ADR** for the route
  spelling; its core decision (hand-rolled `reqwest` client, shared Rust
  request/response types, single in-tree consumer) is **unchanged and is
  the reason C1 is a clean single cut** (one client, moved in lockstep).
- **ADR-0027** (stop semantics) and **ADR-0073** (restart) — the `/stop`
  and `/restart` route *shapes* they reference move with the family; their
  *semantics* (idempotency, generation) are untouched.
- **ADR-0009** (OpenAPI derivation) — the spec is a report of the utoipa
  attrs; regeneration (contract C1 step 3) is the ADR-0009 mechanism, not a
  new one.

### C2 — SPIFFE SVID path segment (cross-checks ADR-0067 / ADR-0072)

- **ADR-0067** (workload identity manager / `SvidLifecycle` /
  `SpiffeId::for_allocation`). D5 defines `for_allocation` as the intended
  **single producer** of the allocation SVID string and records that it
  consolidated the two prior private helpers (`mint_alloc_identity`,
  `mint_identity`). **Correction recorded by this ADR: the consolidation
  was incomplete** — `reconciler_runtime.rs:2938-2943` still hand-rolls the
  same string on a live convergence path (contract C2 step 1b). So there
  are **two** live producers to change, and the crafter enumerates them via
  `rg`, not by trusting the "single producer" claim (contract C2 step 1b's
  note). **Cross-check outcome: ADR-0067 does NOT pin `/job/` as
  load-bearing beyond the producer(s)+parser.** Its D5b
  `TargetResource::new("job/<workload_id>")`
  is an internal **broker-routing key**, not the SVID grammar (explicitly
  out of scope, contract C2 step 5). This ADR renames the segment
  `for_allocation` emits; ADR-0067's identity-holder/reader/dropper design
  is otherwise unchanged. **Not superseded** — the ADR-0067 record keeps
  its historical `/job/` illustrations as written (an ADR is immutable;
  the *code* it describes changes here). The architect may add a
  forward-pointer note to ADR-0067 D5 referencing this rename.
- **ADR-0072** (dial-by-name responder). DDN-2 references the SVID path
  `spiffe://overdrive.local/job/<WorkloadId>/alloc/<id>` and derives the
  DNS `<job>` label from it via the shared parser. **Cross-check outcome:
  ADR-0072 does NOT pin `/job/` beyond the shared `workload_of` parser** —
  DDN-2's mapping is "group the rows by their SVID workload segment," which
  is exactly what the renamed parser does. The emitted DNS name
  (`<name>.svc.overdrive.local`) **never contained the literal "job"**, so
  the DNS wire is unaffected; only the segment the shared parser matches
  moves. **Not superseded.** The architect may add a forward-pointer note
  to ADR-0072 DDN-2.
- **ADR-0063** (built-in CA — `Ca` port, `SvidMaterial`). The mint path is
  unaffected; `issue_svid` signs whatever SPIFFE string `for_allocation`
  produced. No CA-side change.
- **root `CLAUDE.md`** § "Workload identity model — workloads hold NOTHING;
  the kernel does mTLS" — the ephemeral-SVID / no-durable-cert-store
  premise that makes C2 a producer+parser+fixtures cut with no data
  migration.

### Shared

- **Research** —
  `docs/research/refactoring/job-vs-workload-terminology-comprehensive-research.md`
  (Bucket C section + §5 Q2/Q3 ratified decisions + the closed knowledge
  gap on `/job/`-keyed authz).
- **ADR-0047** (workload-kind discriminator) — defines `WorkloadKind::Job`
  and the `[job]` / `b'j'` kind tokens that are **Bucket A (KEEP)**, i.e.
  the Non-scope keep-list in Context.
- **ADR-0050** (OQ-5, intent-key rename to `workloads/<id>`) — the prior
  single-cut Bucket-C-shaped rename this ADR follows the pattern of.
- **`CLAUDE.md`** § single-cut greenfield migrations, § "Implement to the
  design — never invent API surface"; `feedback_single_cut_greenfield_migrations.md`
  — the rules the pinned contract satisfies.
