# Research: Job vs Workload Terminology Disentanglement — Refactor Audit of the Overdrive Codebase

**Date**: 2026-07-01 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (internal), Medium (external direction) | **Sources**: this repository (primary) + external naming-convention references (secondary)

> **This is a codebase-archaeology + categorization research artifact. It changes no code.** Every claim about the code is cited as `path:line`. External references appear only in §2 to justify the "which term should win" direction.

---

## Executive Summary

Overdrive is mid-way through a **naming-model migration** that it started but never finished. The operator surface was originally modeled on **HashiCorp Nomad**, where "job" is the umbrella term for any submitted unit [1]. When the `workload-kind-discriminator` feature (ADR-0047) introduced kinds — `WorkloadKind::{ Service, Job, Schedule }` (`crates/overdrive-core/src/aggregate/workload_spec.rs:328-336`) — the model shifted to the **Kubernetes** shape, where "workload" is the umbrella and "Job" is one specific *run-to-completion* kind among several [2]. The `WorkloadId` rename (commit `17f633e2`) and the intent-key rename to `workloads/<id>` (ADR-0050 OQ-5, `aggregate/mod.rs:1181`) advanced the migration, but a large volume of legacy generic "job" survives. A `\bjob\b|\bJob\b` scan matches **2,621 times across 250 files** in `crates/`; the task is not to rename all of them but to **separate the three classes** the surviving "job" now belongs to.

**Core finding — three distinct classes of surviving "job":**
- **Bucket A — LEGITIMATE (keep):** "job" that means `WorkloadKind::Job` (the run-to-completion kind) — `JobSpec`, the `[job]` TOML body, the `b'j'` discriminator, and — **correcting the task's own hint** — the per-kind streaming surfaces `JobSubmitEvent` (`streaming.rs:580`), `deploy_streaming_job` (`deploy.rs:536`), and the kind-contrast test `submit_job_inserts_no_listener_facts` (`listener_facts.rs:1344`), each of which has a Service sibling and encodes run-to-completion semantics.
- **Bucket B — STALE generic (safe internal rename, single-cut):** "job" that means "the submitted unit, any kind" — a stale-doc residue (`jobs/<id>` in comments where the code already says `workloads/<id>`), test names (`serde_round_trips_job_id`), `JobLifecycle` prose, the `--job` CLI flag, `<job>` DNS variable naming, the scheduler `job: &Job` param, and the central `Job = JobV1` type-alias collision.
- **Bucket C — WIRE / IDENTITY / PERSISTED (decision-gated):** "job" a client, an issued SVID, or a DNS resolver observes — the `/v1/jobs*` HTTP route family + OpenAPI `tag="jobs"`, and the SPIFFE `/job/` path segment applied to **every** kind (`SpiffeId::for_allocation`, `id.rs:320-322`). The persisted intent-key prefixes are **already** `workloads/` — that Bucket-C item was resolved single-cut and only its docs lag.

**Headline recommendation:** **finish the Nomad→Kubernetes migration.** Reserve "job" strictly for `WorkloadKind::Job`; move every generic "job" to "workload". Sequence it: Phase 1 = stale-doc fixes (zero risk, discharges an existing doc-drift debt), Phase 2 = internal identifier renames (compiler-checked, per-test classification for `submit_job_*`), Phase 3 = the `Job`/`JobV1` type-alias decision, Phase 4 = the two genuine wire/identity decisions (`/v1/jobs` route, `/job/` SVID path), each gated behind an explicit user decision and an architect-authored ADR. **The dominant risk is a blind `job→workload` sweep** — it would erase the real kind distinctions in Bucket A (the task hint itself mis-classified three such sites).

---

## 1. Research Methodology

**Search strategy**: `Grep` enumeration of `\bjob\b|\bJob\b`, `job_id`, `submit_job`, `job_stop`, `/v1/jobs`, `SpiffeId::for_allocation`, `<job>`, `for_workload_kind`, `WorkloadKind`, `JobSpec`, `deploy_streaming_job`, `JobSubmitEvent`, `JobCommand`, `--job` across `crates/**/*.rs`, plus targeted reads of the verified anchor sites. ADR cross-checks: ADR-0047 (workload-kind discriminator), ADR-0014 (job API shapes), ADR-0067/0072 (SPIFFE / dial-by-name), ADR-0073 (restart).

**Source selection**: PRIMARY = this repo (crates/, ADRs, CLAUDE.md). SECONDARY = 2–3 external naming-convention authorities (Nomad, Kubernetes, Fowler/DDD ubiquitous-language) for direction only.

**Quality standard**: every code claim carries a `path:line` anchor read directly (not narrated from memory). Bucket A/B/C judgments are verified against what each site actually asserts (job-KIND behavior vs any-workload behavior).

---

## 2. The Intended Vocabulary (Ubiquitous Language)

**Target vocabulary (one term per concept):**

- **"workload"** = the generic umbrella noun for *a submitted unit the platform runs* — regardless of kind. It is the noun for the ID (`WorkloadId`), the aggregate, the lifecycle reconciler (`WorkloadLifecycle`), the intent-key prefix (`workloads/<id>`), and every any-kind operation (submit / stop / restart / describe).
- **"job"** = reserved *strictly* for `WorkloadKind::Job` — the **run-to-completion** kind (the `[job]` TOML body, `JobSpec`, `JobSubmitEvent`, the `b'j'` discriminator). "Schedule" is a cron-fired job; "Service" is the long-running supervised kind.

The legacy naming exists because Overdrive's operator surface was modeled on **HashiCorp Nomad**, where **"job" IS the umbrella** — "A job represents a desired state and provides the set of tasks that should be run" [1], and the submit verb is `nomad job run` (the codebase comment at `crates/overdrive-cli/src/cli.rs:56` cites `nomad job run --detach` directly). When Overdrive introduced kinds (ADR-0047), it moved to the **Kubernetes model**, where **"workload" is the umbrella** — "A workload is an application running on Kubernetes" — and **Job is one specific kind among several** (Deployment, StatefulSet, **Job**, **CronJob**), where "Job … define[s] a task that runs to completion and then stops" and "CronJob … run[s] the same Job … according to a schedule" [2]. Overdrive's `WorkloadKind::{Service, Job, Schedule}` is a near-exact mirror of Kubernetes's umbrella-plus-kinds shape (Schedule ≙ CronJob, Job ≙ Job, Service ≙ a long-running Deployment-shaped kind).

The disentanglement is therefore not a matter of taste — it is finishing a **half-completed migration from the Nomad naming model to the Kubernetes naming model**. The domain-driven-design principle at stake is **ubiquitous language**: one unambiguous term per concept, so "job" never has to be disambiguated by context [3, evergreen]. Every surviving generic "job" is a place where the ubiquitous language is still ambiguous — the reader must ask "job-the-kind or job-the-unit?" — which is exactly the confusion this refactor removes.

> **Direction (headline):** finish the Nomad→Kubernetes naming migration. Keep "job" **only** for `WorkloadKind::Job`; move every generic "job" to "workload". Wire/identity tokens (`/v1/jobs`, SPIFFE `/job/`) are the last, decision-gated step.

---

## 3. Three-Bucket Categorized Inventory

**How to read the buckets.** Each row is a *pattern* (a symbol name, a route string, a doc phrasing), not one of the 2,621 raw hits. `path:line` anchors are representative; where a pattern spans many sites the count is noted. The Bucket A/B judgment for each row was verified against **what the site actually asserts** — job-KIND behavior (`WorkloadKind::Job`) belongs in A; any-workload-generic behavior belongs in B.

### Bucket A — LEGITIMATE `job` (job-the-kind; KEEP)

These name `WorkloadKind::Job` — the run-to-completion kind — or its run-to-completion semantics. Renaming them to "workload" would be *wrong*: it would erase the kind distinction the `workload-kind-discriminator` feature deliberately introduced (ADR-0047).

| # | Pattern / symbol | Anchor | Why it stays |
|---|---|---|---|
| A1 | `WorkloadKind::Job` enum variant | `crates/overdrive-core/src/aggregate/workload_spec.rs:333` | The kind itself. `wire_str()=="job"` (`:376`), `discriminator_byte()==b'j'` (`:350`). |
| A2 | `JobSpec` struct (validated `[job]` body, no `replicas`) | `crates/overdrive-core/src/aggregate/workload_spec.rs:576` | Job-kind body; "run-to-completion per ADR-0047 §1". |
| A3 | `ScheduleSpec.job_inner: JobSpec` | `crates/overdrive-core/src/aggregate/workload_spec.rs:598` | A schedule *is* a cron-wrapped job; the inner body is legitimately a job. |
| A4 | `WorkloadSpec::Job(JobSpec)` / `WorkloadSpecInput::Job(JobSpec)` arms | `workload_spec.rs:626`, `:649` | Tagged-enum kind arm. |
| A5 | `[job]` TOML section (operator surface for a run-to-completion workload) | `workload_spec.rs:332` (doc), ADR-0047 | The operator writes `[job]` to declare a Job. Wire-facing kind token. |
| A6 | `Job` aggregate (rkyv-persisted run-to-completion body) | `crates/overdrive-control-plane/src/handlers.rs:23` (import), `aggregate/mod.rs` | The persisted Job aggregate. |
| A7 | `from_wire_str`/`wire_str` "job" mapping; `from_discriminator_byte` `b'j'` | `workload_spec.rs:376`, `:388`, `:362` | Wire tokens for the Job kind — protocol-fixed, correct. |
| A8 | `AllocCommand::Status` **Job-kind** render arm (`Job '<name>' (kind: Job)`, per-attempt table) | `crates/overdrive-cli/src/render.rs` (`render_kind_aware_body`), see crate CLAUDE.md | Renders the Job-kind view specifically. |
| A9 | `JobSubmitEvent` (per-kind streaming event enum) + `build_workload_stream`/`build_workload_accepted` Job arm | `crates/overdrive-control-plane/src/streaming.rs:580` (enum), `:549-570` (doc: "Job streaming sub-path", run-to-completion semantics); dispatch `handlers.rs:558,565` | **Corrects the task hint.** `JobSubmitEvent` is Job-kind-specific — it has a sibling `ServiceSubmitEvent` (`streaming.rs:559,991`; `deploy.rs:669`). Its variant set (`Running` is informational-not-terminal, `AttemptFailed`, exit-bearing `Succeeded/Failed`) encodes run-to-completion semantics the type system makes structurally distinct from Service. KEEP. |
| A10 | `deploy_streaming_job` (CLI) | `crates/overdrive-cli/src/commands/deploy.rs:536`, dispatched from `deploy_streaming` `:515` alongside `deploy_streaming_service` `:517` | **Corrects the task hint.** Per-kind pair with `deploy_streaming_service` (`:598`). Consumes `JobSubmitEvent`; Service consumes `ServiceSubmitEvent`. Job-kind-specific. KEEP. |
| A11 | `submit_job_inserts_no_listener_facts` test | `crates/overdrive-control-plane/src/listener_facts.rs:1344` | **Corrects the task hint.** Asserts a **Job-KIND** invariant: "A Job submit allocates no VIP … edge upsert is Service-only" (`:1341-1352`), builds `wire_job("batch")`. This is a kind-contrast test — KEEP the `job` in the name. |
| A12 | `WorkloadIntentV1::Job(JobV1)` / `WorkloadSpec::Job` persistence variant | `crates/overdrive-core/src/aggregate/mod.rs:389` ("Run-to-completion workload") | The rkyv-persisted Job-kind variant. Discriminant-fixed. KEEP. |
| A13 | `JOB_PROBES_GUIDANCE` / job-kind probe-rejection messaging (if present) | _verify by grep_ `JOB_PROBES` — **NOT FOUND** in this pass; treat as N/A unless a later grep surfaces it | Would be Job-kind-specific if present (Job rejects readiness probes). See Knowledge Gaps. |
| A14 | `job_kind_*` / `kind: Job` / `WorkloadKind::Job` assertion tests | grep `kind: Job`, `WorkloadKind::Job`, `wire_job(` in tests | Tests of the kind discriminator itself — must keep "job". |

### Bucket B — STALE generic `job` → should be `workload` (safe internal rename; single-cut)

These use "job" to mean **the submitted unit, generically, regardless of kind**. The rename to "workload" is behavior-preserving (identifiers, test names, doc comments — no wire/on-disk change). Single-cut per the project's greenfield-migration rule (no deprecation shims).

| # | Pattern / symbol | Anchor(s) | Recommended new name |
|---|---|---|---|
| B1 | Module doc says intent keys are `jobs/<id>` — but the **actual key string is already `workloads/<id>`** | `crates/overdrive-core/src/aggregate/mod.rs:1`, `:11`, `:1166`, `:1294` | Fix doc to `workloads/<id>`. **Pure doc residue** — the code was already migrated single-cut (ADR-0050 OQ-5, `for_workload` at `:1181`). This is a "behavior-change must mark stale adjacent docs" violation left behind by that migration. |
| B2 | `serde_round_trips_job_id` test name — constructs a `WorkloadId` | `crates/overdrive-core/src/id.rs:1424` | `serde_round_trips_workload_id` |
| B3 | `JobLifecycle` doc/prose residue (reconciler is `WorkloadLifecycle`) | `crates/overdrive-core/src/transition_reason.rs:544` ("`BackoffExhausted` JobLifecycle pathway"); grep `JobLifecycle` | `WorkloadLifecycle` in prose |
| B4 | `AllocCommand::Status { job: String }` — the `--job` flag whose own doc says "Canonical `WorkloadId`" | `crates/overdrive-cli/src/cli.rs:121-125` | Field/flag → `workload` / `--workload` (any-kind describe). See §5 Q for the CLI-verb decision. |
| B5 | `AllocCommand::Status` doc "Read canonical `spec_digest` for a job" / "Named after ADR-0014's `GET /v1/jobs/{id}`" | `crates/overdrive-cli/src/cli.rs:117-120` | Reword to "for a workload". |
| B6 | `post_http_invalid_job_id` and any `submit_job_*` / `job_stop_*` test names that exercise **any-kind** (not Job-kind-contrast) behavior | grep `submit_job`, `job_stop`, `invalid_job_id` across `tests/` — **classify each individually**: A11 (`submit_job_inserts_no_listener_facts`) is Job-KIND and stays; a test named for "the submitted unit" generically renames | `submit_workload_*` / `workload_stop_*` / `invalid_workload_id` — **only** for the generic ones. **Do NOT blind-rename** — each `submit_job_*` must be read: does it assert a Job-kind contrast (→ A) or any-workload behavior (→ B)? |
| B7 | Generic "job" in rustdoc/comments across `src/` meaning "the submitted unit" | pervasive; e.g. handler comment `resource = jobs/<id>` at `handlers.rs:807` (note `:868`/`:907` already say `workloads/<id>` — **inconsistent within one file**) | Reword to "workload" / `workloads/<id>` |
| B8 | `for_schedule` doc references `` [`Self::for_job`] `` — a method that **no longer exists** (replaced by `for_workload`) | `crates/overdrive-core/src/aggregate/mod.rs:1238` | Fix dangling doc-link to `for_workload`. |
| B9 | OpenAPI tag description "Job lifecycle endpoints"; `streaming.rs` module doc "Streaming submit loop for `POST /v1/jobs`" | `crates/overdrive-control-plane/src/api.rs:495`; `crates/overdrive-control-plane/src/streaming.rs:1` | Reword description to "Workload lifecycle endpoints". (The tag *string* `"jobs"` is the wire-facing Bucket C item C1; the *description text* is free B prose.) |
| B10 | `JobSubmitEvent::Accepted.intent_key` doc "Canonical `jobs/<id>` IntentKey string form" — the real `IntentKey` is `workloads/<id>` | `crates/overdrive-control-plane/src/streaming.rs:587` | Fix doc to `workloads/<id>`. Same stale-doc class as B1 (the *value* on the wire is already `workloads/<id>`; only the doc lies). |
| B11 | `dns_responder/name_index.rs` `<job>` variable/label naming (means "the workload") — reads the literal `/job/` SPIFFE segment (C2), groups rows by `<job>` | `crates/overdrive-control-plane/src/dns_responder/name_index.rs:2,10,49,52,110-114` (69 total `job` hits, all naming) | Rename `<job>` → `<workload>`/`<name>` in identifiers & docs. **No wire change** — the literal `/job/` it *parses* is the SPIFFE path (C2); this file only names its variables after it. |
| B12 | Scheduler `job: &Job` parameter + "the job's resource envelope" prose — places **any** kind, param named after the Job aggregate | `crates/overdrive-scheduler/src/lib.rs:73,53,63-64,92-118,180-185` | Rename param `job → workload` and prose "job" → "workload". **Coupled to the B13 type-naming decision** — the *type* `Job` (= `JobV1`) is the run-to-completion payload, so `workload: &Job` reads oddly until B13 resolves. |
| B13 | **`pub type Job = JobV1`** — the type alias `Job` doubles as (a) the Job-**kind** run-to-completion payload (A12) AND (b) the generic "aggregate" noun the module doc calls "the legacy `Job` aggregate" | `crates/overdrive-core/src/aggregate/mod.rs:140` (alias), `:36` ("legacy `Job` aggregate"), `:100-140` (aggregate doc), `handlers.rs:23` (import) | **This is the central naming collision.** `JobV1` IS the run-to-completion payload (correct, A12). But `Job` as an *un-suffixed umbrella noun* invites every generic call site (scheduler, handlers) to read it as "the workload aggregate". **Recommend**: keep `JobV1`/`WorkloadIntentV1::Job` (kind payload), and audit each *un-suffixed `Job`* call site — those meaning "the run-to-completion payload" keep it; those meaning "the submitted unit generically" want the umbrella type (`WorkloadIntent`) or a rename. Needs a design decision (see §5 Q5). |

### Bucket C — WIRE / IDENTITY / PERSISTED `job` (rename has protocol or on-disk cost; needs an explicit decision)

These "job" tokens cross a boundary a client, an issued cert, a DNS resolver, or an on-disk file observes. A rename is **not** a free internal refactor; each needs an explicit decision (and likely an ADR), weighed against the project's single-cut greenfield rule.

| # | Pattern | Anchor(s) | Boundary / compat cost | Single-cut clean? |
|---|---|---|---|---|
| C1 | HTTP route family `/v1/jobs`, `/v1/jobs/{id}`, `/v1/jobs/{id}/stop`, `/v1/jobs/{id}/restart` + OpenAPI `tag="jobs"` | routes: `crates/overdrive-control-plane/src/lib.rs:2330-2333`; `#[utoipa::path] path="/v1/jobs..."` `handlers.rs:196,649,784,883`; tag `handlers.rs:209,658,794,893` + `api.rs:495`; client `crates/overdrive-cli/src/http_client.rs` (grep `/v1/jobs`) | **HTTP wire contract.** Any external client hitting `/v1/jobs` breaks. But the ONLY in-tree client is `overdrive-cli` (`http_client.rs`) — server + client move together in one cut. | **Likely yes** (Phase-1 greenfield, single in-tree client) — but it is a *public API surface* → warrants an ADR + user decision, not a silent rename. |
| C2 | SPIFFE SVID path segment `/job/` applied to **every** kind (services included): `SpiffeId::for_allocation` → `spiffe://overdrive.local/job/{workload}/alloc/{alloc}` | `crates/overdrive-core/src/id.rs:320-322`; doc examples `:238`, `:302`, `:311`, `:327`; grep `/job/` | **Issued-certificate identity.** Every SVID's SPIFFE ID literally contains `/job/`. Changing to `/workload/` changes every issued cert's identity string; any policy/authz keyed on the `/job/` path segment (Rego, intended-peer pinning) must move in lockstep. mTLS is kernel-mediated and certs are ephemeral/reissued (workloads hold no SVID — root CLAUDE.md), so no *durable* cert store to migrate. | **Plausibly yes** given ephemeral SVIDs + kernel-mediated mTLS, BUT it touches the identity grammar → **must** be an ADR decision. Cross-check ADR-0067/0072. |
| C3 | Mesh DNS grammar: `MeshServiceName` names its label `<job>`; `<job>.svc.overdrive.local`; `DNS_LABEL_OCTET_MAX` doc "bounds a `MeshServiceName`'s `<job>`" | `crates/overdrive-core/src/id.rs:798-859`; grep `<job>` | **Naming only at the label position** — the *label value* is the workload id, not a literal "job"; the grammar suffix is `.svc.overdrive.local` (no literal "job" on the wire). So this is mostly **doc/identifier naming** (`<job>` → `<workload>` or `<name>`) with **no wire change** — the DNS name emitted never contains the literal string "job". | **Yes — effectively Bucket B** for the wire (no literal "job" emitted); only the *doc/label-variable naming* changes. Kept in C for visibility because it sits on the DNS-identity surface. |
| C4 | Persisted intent-key prefixes | `aggregate/mod.rs:1181` (`workloads/<id>`), `:1191`, `:1210`, `:1232` | **Already migrated to `workloads/`** single-cut (ADR-0050 OQ-5). Only the *docs* still say `jobs/` (→ B1). No remaining persisted "job" prefix for the workload aggregate. | N/A — done. `schedules/<id>` (`:1248`) is its own kind prefix (fine). |
| C5 | rkyv-archived field names / discriminator carrying "job" (`workload_kind` byte `b'j'`, `SubmitWorkloadRequest.workload_kind` wire field) | `workload_spec.rs:350`, `api.rs` `workload_kind` | The `b'j'` byte and `"job"` string are the **Job-kind discriminator** (Bucket A) — correct, not stale. No generic-"job" archived field found. | N/A — these are kind tokens (A7), not generic residue. |

---

## 4. Proposed Clean Refactor Plan (Phased)

Ordered so **mechanical, zero-risk** work lands first and **wire/identity** changes are gated behind explicit decisions. All internal renames are single-cut greenfield (no deprecation shims, no dual old/new paths — per project rule).

### Phase 1 — Stale-doc & dangling-reference fixes (zero behavior change, zero API change)

**Scope:** Bucket B rows B1, B7, B8, B9, B10 — comments and rustdoc only. No identifier changes.
**Affected crates:** `overdrive-core` (`aggregate/mod.rs` module doc + `for_schedule`/`as_str`/`IntentKey` docs), `overdrive-control-plane` (`streaming.rs`, `api.rs` tag description, `handlers.rs` `resource = jobs/<id>` comment).
**Risk:** none. These are docs that already contradict the code (the code says `workloads/<id>`; the docs say `jobs/<id>`).
**Interaction with rules:** this phase *discharges* an existing "behavior-change must mark stale adjacent docs" debt left by the ADR-0050 OQ-5 migration — the migration changed the key strings but left the surrounding prose stale. No aspirational-docs concern (we are removing lies, not adding claims).
**Payoff:** removes the most actively-misleading residue (docs asserting a key format the code abandoned) for near-zero cost.

### Phase 2 — Internal identifier renames (behavior-preserving, no wire/on-disk change)

**Scope:** Bucket B rows B2, B3, B4, B5, B11, B12 — test names, struct field / CLI-flag identifiers, `<job>` variable naming, scheduler param.
**Affected crates:** `overdrive-core` (`id.rs` test name, `transition_reason.rs` prose), `overdrive-cli` (`AllocCommand::Status { job }` field + `--job` flag + docs — see §5 Q1), `overdrive-control-plane` (`dns_responder/name_index.rs` `<job>` naming), `overdrive-scheduler` (`job: &Job` param).
**Risk:** low. Pure rename; the compiler catches every call site. The `--job` flag rename (B4) is the one *operator-visible* item — it changes a CLI flag string, so it is technically a small CLI-surface change (fold into the Q1 decision, not a silent rename).
**Interaction with rules:** single-cut — rename identifier + every caller in one commit; delete nothing that still has a referent. `submit_job_*` test names (B6) must be **classified individually** first (A11 stays; generic ones rename) — do not blind-rename.

### Phase 3 — Structural type-naming decision (`Job = JobV1` collision)

**Scope:** Bucket B row B13 (+ its coupled B12 scheduler param).
**Affected crates:** `overdrive-core` (`aggregate/mod.rs` `pub type Job = JobV1`), every consumer of the un-suffixed `Job` alias (`handlers.rs`, `overdrive-scheduler`, tests).
**Risk:** medium — touches a widely-imported public type alias. Not a wire change, but a broad internal API-surface change.
**Interaction with rkyv rules:** `JobV1` / `WorkloadIntentV1::Job` are the **persisted** names — leave the rkyv variant identifiers alone (renaming a variant would be a schema-evolution event per `.claude/rules/development.md` § "rkyv schema evolution"). This phase is only about the *un-suffixed alias `Job`* and its generic call sites, not the versioned payload type. **Gate behind Q5.**

### Phase 4 — Wire / identity decisions (each its own gated item)

Each of these is a **separate decision** with a distinct compat cost; none is a blind rename. Present each to the user; on approval, each likely warrants an ADR (route/identity grammar are architectural surfaces).

| Item | Bucket | Compat cost | Single-cut viability |
|---|---|---|---|
| `/v1/jobs*` HTTP routes + OpenAPI `tag="jobs"` → `/v1/workloads*` | C1 | Breaks any external HTTP client; only in-tree client is `overdrive-cli` (moves in lockstep) | Clean cut *iff* no external client is in scope for Phase 1 (greenfield). Needs Q2 + ADR. |
| SPIFFE SVID path `/job/` → `/workload/` (all kinds) | C2 | Changes every issued SVID's identity string; any authz/policy keyed on `/job/` segment moves in lockstep; SVIDs are ephemeral + kernel-held (no durable cert store to migrate) | Plausibly clean (ephemeral certs), but touches identity grammar → needs Q3 + ADR; cross-check ADR-0067/0072. |
| `MeshServiceName` `<job>` label naming | C3 (≈B) | **No wire change** (the emitted DNS name never contains literal "job"); doc/identifier-only | Fold into Phase 2 once confirmed no literal "job" is emitted; kept visible here because it sits on the DNS surface. |

**Interaction with rules:** each wire item is a *public surface* change → route it through the architect agent for the ADR, per project convention (design artifacts go through the architect, not inline). Do **not** create GitHub issues for these — surface as questions (§5) and let the user decide.

---

## 5. Risks, Open Questions, and Decisions the User Must Make

Each is an explicit question **for the user** — not a recommendation to act, and (per project rule) **not** a prompt to create a GitHub issue. No issue numbers are invented.

### ✅ Ratified decisions (2026-07-01)

The user ratified the following. This subsection is the decision record; the per-question detail below is retained as rationale.

- **Q1 — CLI verb: KEEP.** `overdrive job list|stop` and the `--job` flag stay. These CLI tokens drop out of the Bucket-B rename set (B4 keeps the flag/verb identifiers; only free-prose "for a job" wording is reworded). *Residual sub-fork to confirm: keep `job list|stop` acting on **all** kinds (verbatim status quo), or re-scope `job` to Job-kind-only and add a generic `workload list|stop`.*
- **Q2 — `/v1/jobs*` → `/v1/workloads*`: REFACTOR (approved).** Wire change → architect-authored ADR first (updates/supersedes ADR-0008/0014), then single-cut server + in-tree client (`overdrive-cli`) in one commit.
- **Q3 — SPIFFE `/job/` → `/workload/`: REFACTOR (approved).** Identity-grammar change on every kind → architect-authored ADR first (cross-check ADR-0067/0072), then single-cut. Grep policy / `mtls_resolve` / dataplane surfaces for any `/job/`-keyed authz that must move in lockstep (per Knowledge Gaps).
- **Q4 — `Job = JobV1` alias: AUDIT.** Audit every un-suffixed `Job` call site; generic → umbrella type, kind-specific → keep. rkyv variant names (`JobV1`, `WorkloadIntentV1::Job`) unchanged.
- **Cadence — all three buckets, one at a time.** Execution order: **Bucket B** (internal, no ADR) → **Bucket C** (wire/identity, ADR-gated), with **Bucket A** as the invariant to protect throughout.

**Q1 — CLI verb: `overdrive job list|stop` and `--job`.**
Today `overdrive job list|stop` (`crates/overdrive-cli/src/cli.rs:64-66,98-102`) operate on **any** workload kind, while `overdrive workload restart` (`:68-70,104-108`) is the newer correct naming. And `overdrive alloc status --job <id>` (`:121-125`) has a `--job` flag whose own doc says "Canonical `WorkloadId`". Options:
  (a) Rename `job list|stop` → `workload list|stop` and `--job` → `--workload` (consolidate all any-kind ops under `workload`); OR
  (b) Keep `job` as a *filter-by-kind* subcommand (`overdrive job list` = "list Job-kind workloads only") and add generic `workload list|stop`.
Note (a) is the cleaner ubiquitous-language outcome but is an **operator-visible CLI change**; (b) preserves `job` but gives it a *new, narrower* meaning. **Decision needed before Phase 2** (B4/B5 depend on it).

**Q2 — HTTP route family `/v1/jobs*` → `/v1/workloads*`?** (C1)
Breaking for any external HTTP client; the only in-tree client (`overdrive-cli/src/http_client.rs:200,231,276,293,302`) moves in the same cut. Is any *external* consumer of `/v1/jobs` in scope for Phase 1? If not, the single-cut greenfield rule permits a clean rename — but it is a **public API surface** and should land via an ADR (ADR-0008/0014 are the current records). **User decision + architect-authored ADR.**

**Q3 — SPIFFE SVID path `/job/` → `/workload/`?** (C2)
`SpiffeId::for_allocation` hardcodes `spiffe://overdrive.local/job/<workload>/alloc/<alloc>` (`crates/overdrive-core/src/id.rs:320-322`) for **every** kind, services included. Changing to `/workload/` changes every issued SVID's identity string. Because mTLS is kernel-mediated and SVIDs are ephemeral/reissued (workloads hold no SVID material — root `CLAUDE.md`), there is likely **no durable cert store to migrate** — but any Rego/authz/intended-peer logic keyed on the `/job/` path segment must move in lockstep, and the change touches the **identity grammar**. Cross-check ADR-0067 / ADR-0072. **User decision + architect-authored ADR.** *(If deferred, note: leaving `/job/` is internally consistent but permanently bakes the Nomad-era term into every workload's identity.)*

**Q4 — `MeshServiceName` `<job>` label naming.** (C3)
The mesh DNS grammar names its label `<job>` in docs/identifiers (`crates/overdrive-core/src/id.rs:798-859`; `dns_responder/name_index.rs`), but a `MeshServiceName` names a **service/workload**, and the emitted DNS name (`<name>.svc.overdrive.local`) **never contains the literal string "job"**. Confirm there is truly no wire dependency, then this is a Phase-2 identifier/doc rename (`<job>` → `<workload>` or `<name>`). **Low-stakes; confirm-then-rename.**

**Q5 — The `Job = JobV1` type-alias collision.** (B13)
`pub type Job = JobV1` (`crates/overdrive-core/src/aggregate/mod.rs:140`) is simultaneously the **run-to-completion payload** (legitimate, A12) and the noun the module doc calls "the legacy `Job` aggregate" (`:36`) that generic call sites (scheduler `job: &Job`, handlers) read as "the workload aggregate". Do we:
  (a) Keep `JobV1` as the kind payload and audit every un-suffixed `Job` call site (generic ones → `WorkloadIntent`/umbrella type; kind-specific ones keep `Job`); OR
  (b) Accept the collision as low-cost and only fix docs?
The rkyv variant names (`JobV1`, `WorkloadIntentV1::Job`) stay regardless (renaming is a schema-evolution event). **Decision needed before Phase 3.**

### Cross-cutting risks

- **Blind-rename hazard (the central risk).** The task's own hint mis-classified three sites as stale-generic that are in fact Job-kind-specific: `JobSubmitEvent` (A9), `deploy_streaming_job` (A10), `submit_job_inserts_no_listener_facts` (A11). **A mechanical `job→workload` sweep would erase real kind distinctions.** Every rename must be verified against what the site asserts (kind-contrast → keep; any-workload → rename). This is why Phase 2's `submit_job_*` classification is per-test.
- **Doc-vs-code drift is already live.** `jobs/<id>` in docs vs `workloads/<id>` in code (B1, B10) means readers are *already* being misled today — Phase 1 is worth doing on its own merits, independent of the wire decisions.
- **rkyv discriminant discipline.** Any temptation to rename `WorkloadIntentV1::Job` / `JobV1` must be resisted — it is a positional-layout schema-evolution event, not a rename (per `.claude/rules/development.md` § "rkyv schema evolution" / "Version-bump procedure").
- **Architect-gated surfaces.** Route grammar (Q2) and identity grammar (Q3) are architectural; per project convention their ADRs are authored by the architect agent, not inline.

---

## Knowledge Gaps

- **`JOB_PROBES_GUIDANCE` (A13)** — the task hint listed it; a targeted grep for `JOB_PROBES` in `crates/**/*.rs` did **not** surface it in this pass. It may exist under a different symbol name (probe-rejection for Job kind). If present, it is Job-kind-specific (Bucket A). *Attempted:* `Grep JOB_PROBES`. *Recommendation:* a follow-up grep for `probe`+`Job`/`readiness`+`reject` before Phase 2.
- **Exhaustive per-test classification of `submit_job_*` / `job_stop_*` / `*_job_id` test names** — this audit classified the representative and highest-signal cases and established the *rule* (kind-contrast → A; any-workload → B), but did not read all ~2,621 hits. *Recommendation:* Phase 2 opens with a mechanical enumeration (`grep -rn 'fn .*job' crates/**/tests`) and a per-test A/B tag before any rename.
- **External authz/policy dependence on the SPIFFE `/job/` segment** — whether any Rego policy, intended-peer-pinning rule, or dataplane match keys on the literal `/job/` path segment (which would have to move in lockstep with Q3). *Attempted:* not exhaustively traced in this pass. *Recommendation:* grep policy + `mtls_resolve` surfaces for `/job/` before deciding Q3.

## Conflicting Information

**The task-prompt anchor list vs. the code — three mis-classifications (resolved in favor of the code).** The dispatch hint tentatively grouped `deploy_streaming_job`/`JobSubmitEvent` and `submit_job_*` test names under "Bucket B (assess — some are Job-kind-specific)". Direct reading resolves them to **Bucket A**: `JobSubmitEvent` has a sibling `ServiceSubmitEvent` (`streaming.rs:559,991`), `deploy_streaming_job` pairs with `deploy_streaming_service` (`deploy.rs:515-517`), and `submit_job_inserts_no_listener_facts` asserts a Job-vs-Service kind contrast (`listener_facts.rs:1341-1352`). The code is authoritative; the hint correctly *flagged* these for assessment, and the assessment lands them in A.

## Source Analysis & Citation Coverage

**Citation coverage:** every code claim carries a `path:line` anchor read directly during this session (not narrated from memory). Anchor verification reads performed: `workload_spec.rs` (WorkloadKind, JobSpec, WorkloadSpec), `id.rs` (WorkloadId, SpiffeId::for_allocation, MeshServiceName, `serde_round_trips_job_id`), `aggregate/mod.rs` (IntentKey `for_workload*`, `Job=JobV1`, `WorkloadIntentV1`, module docs), `handlers.rs` + `api.rs` + `lib.rs` (route family, tags, handler names), `http_client.rs` (client route strings), `streaming.rs` (JobSubmitEvent + siblings), `deploy.rs` (deploy_streaming pair), `cli.rs` (Job/Workload command split, `--job`), `listener_facts.rs` (submit_job test), `scheduler/src/lib.rs` (`job: &Job`), `dns_responder/name_index.rs` (`<job>` grammar), `transition_reason.rs` (JobLifecycle prose). Plus Grep enumeration of `/job/` (37+ hits, all SPIFFE-path or test), `/v1/jobs`, `JobLifecycle`, `JobSubmitEvent`, `deploy_streaming_*`, `ServiceSubmitEvent`.

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| This repository (`crates/`, ADRs, CLAUDE.md) | local | High (1.0) — primary authoritative | source code / SSOT | 2026-07-01 | Y (anchors read directly) |
| Nomad architecture docs | developer.hashicorp.com | High (1.0) — official | technical docs | 2026-07-01 | Y (matches `cli.rs:56` comment) |
| Kubernetes Workloads concept | kubernetes.io | High (1.0) — official | technical docs | 2026-07-01 | Y (matches `WorkloadKind` shape) |
| DDD "Ubiquitous Language" (Evans/Fowler) | martinfowler.com | Medium-High (0.8) | methodology (evergreen) | 2026-07-01 | Direction-only, single ref |

Reputation: High 3 (75%), Medium-High 1 (25%), Avg ≈ 0.95. **Confidence: High** for the code inventory (primary source read directly); **Medium-High** for the external "which term should win" direction (two High-tier official corroborating sources + one evergreen methodology principle).

## Sources

[1] HashiCorp. "Nomad Architecture". developer.hashicorp.com. https://developer.hashicorp.com/nomad/docs/concepts/architecture. Accessed 2026-07-01. — "A job represents a desired state and provides the set of tasks that should be run." (Nomad = the umbrella-"job" model; the source of Overdrive's legacy naming; cf. `crates/overdrive-cli/src/cli.rs:56` citing `nomad job run --detach`.)

[2] The Kubernetes Authors. "Workloads". kubernetes.io. https://kubernetes.io/docs/concepts/workloads/. Accessed 2026-07-01. — "A workload is an application running on Kubernetes." Workload resource kinds include Deployment, StatefulSet, **Job** ("a task that runs to completion, just once"), and **CronJob** ("run the same Job … according to a schedule"). (Kubernetes = the umbrella-"workload" model; Overdrive's `WorkloadKind::{Service, Job, Schedule}` mirror.)

[3] Fowler, Martin. "UbiquitousLanguage" (Eric Evans, *Domain-Driven Design*). martinfowler.com. https://martinfowler.com/bliki/UbiquitousLanguage.html. [Foundational; concept remains current] — one rigorous, shared term per concept, so a word never needs context to disambiguate. Cited direction-only.

### Internal anchor index (representative, per bucket)

- **Bucket A:** `workload_spec.rs:333` (WorkloadKind::Job), `:350` (b'j'), `:376` (wire_str), `:576` (JobSpec), `:598` (job_inner); `streaming.rs:580` (JobSubmitEvent); `deploy.rs:536` (deploy_streaming_job); `listener_facts.rs:1344` (submit_job test); `aggregate/mod.rs:389` (WorkloadIntentV1::Job).
- **Bucket B:** `aggregate/mod.rs:1,11,1166,1294,1238` (stale `jobs/<id>` docs + dangling `for_job` link); `id.rs:1424` (serde_round_trips_job_id); `transition_reason.rs:544` (JobLifecycle prose); `cli.rs:117-125` (`--job` flag+docs); `streaming.rs:587` (stale intent_key doc); `dns_responder/name_index.rs:110-114` (`<job>` naming); `scheduler/src/lib.rs:73` (`job: &Job`); `aggregate/mod.rs:140,36` (`Job = JobV1` collision).
- **Bucket C:** `lib.rs:2330-2333` + `handlers.rs:196,649,784,883` + `api.rs:495` + `http_client.rs:200,231,276,293,302` (`/v1/jobs*` route family + tag); `id.rs:320-322` (SPIFFE `/job/`); `id.rs:798-859` (MeshServiceName `<job>`, naming-only).

## Research Metadata

Duration: single session (~turns 1-30) | Files examined: 15+ read directly, 250 scanned via Grep | Code claims cited: 40+ with `path:line` | External sources: 3 (2 High-tier official, 1 evergreen) | Cross-refs: 3 mis-classifications corrected against code | Confidence: High (internal inventory), Medium-High (external direction) | Output: `docs/research/refactoring/job-vs-workload-terminology-comprehensive-research.md`
