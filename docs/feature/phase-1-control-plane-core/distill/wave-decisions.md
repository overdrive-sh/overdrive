# DISTILL Wave Decisions — phase-1-control-plane-core

**Wave**: DISTILL (acceptance-designer)
**Owner**: Quinn
**Date**: 2026-04-23
**Status**: COMPLETE — handoff-ready for DEVOPS (platform-architect) and DELIVER (software-crafter), pending peer review.

> **Amendment 2026-04-26.** `overdrive cluster init` was removed from
> Phase 1 in commit `d294fb8`. The walking-skeleton CA-minting site is
> now `overdrive serve` exclusively; the demo / litmus / strategy-C
> references below are revised in place. The `cluster init` verb
> returns in Phase 5 with the persistent CA + operator-cert ceremony
> per ADR-0010 §Amendment 2026-04-26 and GH #81. Full rationale in
> `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.

---

## Reconciliation

**Reconciliation passed — 0 contradictions.**

Procedure run per skill Wave-Decision Reconciliation:

1. **DISCUSS ↔ DESIGN** — DISCUSS UC-1 (transport pivot to REST + OpenAPI)
   is codified by ADR-0008. DISCUSS Key Decision 7 (ship `Action::HttpCall`
   in Phase 1 surface even though runtime shim lands Phase 3) matches
   ADR-0013 §3. DISCUSS Key Decision 8 (`ObservationStore` impl = reuse
   `SimObservationStore`) matches ADR-0012. DISCUSS Key Decision 6
   (`JobSpec` placeholder resolution) is executed by ADR-0011.
2. **DISCUSS ↔ whitepaper** — Whitepaper §4 names `Job`, `Node`,
   `Allocation`, `Policy`, `Investigation` — user stories US-01 ship the
   first three as aggregates; §4's "Policy / Investigation stubs" matches
   ADR-0011. Whitepaper §18 pure-reconciler contract matches ADR-0013
   trait shape.
3. **DEVOPS absence** — `docs/feature/phase-1-control-plane-core/devops/`
   does not yet exist. Per skill graceful-degradation rule, default
   environment matrix is `clean`, `with-pre-commit`, `with-stale-config`.
   DEVOPS wave may run in parallel after this handoff; none of its
   decisions can regress a DISTILL scenario (environments are additive).
4. **KPI contracts absence** — `docs/product/kpi-contracts.yaml` does not
   exist. Per skill soft gate, **warning logged**, proceeding without
   `@kpi-contract` tagged observability scenarios. KPIs K1–K7 from
   `discuss/outcome-kpis.md` drive `@kpi KN` scenario tagging below —
   that is the feature-level KPI surface; the product-level contracts
   file is a future artifact.

---

## DWD-01 — Walking Skeleton Strategy: C (Real local)

**Decision**: Strategy **C (real local)**. The walking skeleton exercises:

- Real `LocalStore` (redb) against `tempfile::TempDir`.
- Real `rcgen`-minted ephemeral CA at `overdrive serve` start
  (ADR-0010 §R1 as amended 2026-04-26 — `serve` is the sole
  Phase 1 cert-minting site).
- Real `axum` + `rustls` server bound on `https://127.0.0.1:7001`
  (ADR-0008).
- Real `reqwest`-based CLI client hitting the real server (ADR-0014).
- Real per-primitive `libsql` database file provisioned at
  `<data_dir>/reconcilers/<name>/memory.db` (ADR-0013).
- `SimObservationStore` — the **production Phase 1 implementation** per
  ADR-0012 with `GossipProfile::single_node()`. Not a mock. Not a fake.
  The wiring adapter around it IS the Phase 1 server impl.

**Strategy-C litmus** ("If I deleted the real adapter, would the WS still
pass?"):

- Delete `redb` → compile failure. WS fails. ✅
- Delete `rcgen` → no CA can be minted → `serve` fails to bootstrap
  TLS → every WS scenario fails. ✅
- Delete `axum` / `rustls` → server cannot bind. WS fails. ✅
- Delete `reqwest` → CLI has no client. WS fails. ✅
- Delete `libsql` → reconciler-runtime cannot provision per-primitive
  memory. WS fails. ✅

No `@requires_external` markers in Phase 1 — nothing in the walking
skeleton depends on a paid service, a remote registry, or an internet
connection.

**Tagging convention:**

- `@walking_skeleton @real-io @adapter-integration @driving_adapter` —
  end-to-end scenarios entering through the `overdrive` CLI subprocess.
- `@walking_skeleton @real-io @adapter-integration` — end-to-end
  scenarios entering through the HTTP library port (reqwest against the
  real server), used when the CLI is not the driving concern.
- `@library_port` — Rust public-API surface tests for aggregates,
  `IntentKey`, `Reconciler` trait contract.

## DWD-02 — KPI-to-scenario tag map

KPIs K1–K7 from `discuss/outcome-kpis.md`. Every KPI has ≥1 scenario.

| KPI | Summary | Tagged scenarios |
|---|---|---|
| K1 | Round-trip spec digest byte-identical | §1.1 WS-1, §3.1, §6.1 |
| K2 | Invalid spec rejected before IntentStore write (400) | §4.2, §4.3, §6.6 |
| K3 | `commit_index` strictly monotonic | §4.5, §4.6 |
| K4 | DST invariants for reconciler primitive | §5.7, §5.8, §5.9 |
| K5 | `cluster status` surfaces reconciler registry + broker counters | §5.6, §6.4 |
| K6 | Error paths answer "what / why / how to fix" | §3.4, §6.3, §6.5, §6.6 |
| K7 | Empty observations render explicit empty state | §4.7, §4.8, §6.2, §6.7 |

## DWD-03 — No `.feature` files; scenarios are Gherkin-in-markdown

Project rule (`.claude/rules/testing.md`) and phase-1-foundation DWD-03
(precedent). Carried forward verbatim. Every scenario in
`test-scenarios.md` is a fenced ```gherkin block; the crafter translates
each to a Rust `#[test]` / `#[tokio::test]` function under
`crates/{crate}/tests/acceptance/*.rs` or
`crates/{crate}/tests/integration/*.rs` per ADR-0005.

No `cucumber-rs`, no `pytest-bdd`, no `conftest.py`, no
`.feature`-file consumer is introduced by this feature.

## DWD-04 — Driving ports identified for this feature

Per the architecture brief §14–§23 and DESIGN `wave-decisions.md`,
the driving ports for phase-1-control-plane-core are:

1. **`cargo run --bin overdrive -- <args>` subprocess** (or the
   installed `overdrive` binary). The operator-facing CLI. All US-05
   scenarios enter through this port. Scenarios tagged `@driving_adapter`.
2. **`overdrive serve` subprocess** — starts the control-plane HTTP
   server as a child process. The walking-skeleton scenarios compose
   a `serve` subprocess with a `job submit` / `alloc status` subprocess
   against the same endpoint.
3. **`POST /v1/jobs`** + **`GET /v1/jobs/{id}`** + **`GET /v1/cluster/info`**
   + **`GET /v1/allocs`** + **`GET /v1/nodes`** HTTP endpoints via
   `reqwest` against the real axum server. Used when the CLI is not the
   driving concern (e.g. asserting the HTTP error shape directly).
4. **Library trait surface** (`Reconciler`, `IntentStore`,
   `ObservationStore`, `Job::from_spec`, `IntentKey::for_job`). Tagged
   `@library_port`; exercised from `tests/acceptance/*.rs` in the owning
   crate.

Every AC in `user-stories.md` maps to ≥1 scenario. Every CLI subcommand
named in ADR-0014 is exercised by ≥1 `@driving_adapter` scenario.
Every HTTP endpoint named in ADR-0008 is exercised by ≥1 scenario
(either via CLI or via reqwest directly).

## DWD-05 — Property-based scenarios marked `@property`

Per `testing.md` proptest mandatory call sites, the following scenarios
carry `@property` so the DELIVER crafter translates via `proptest!`
blocks (not single-example assertions):

- `Job` / `Node` / `Allocation` aggregate rkyv round-trip equality.
- `Job` / `Node` / `Allocation` aggregate serde-JSON round-trip equality.
- `IntentKey::for_job` stability over arbitrary valid `JobId`.
- `ContentHash::of` determinism over arbitrary valid `Job`.
- `LocalStore::commit_index` monotonicity across N successive submits.

Each `@property` scenario's generator must span both accepted and
rejected inputs where relevant — this is how the 40% error-path ratio
is achieved without duplicating hand-picked boundary cases.

## DWD-06 — Scaffolding posture (Mandate 7, additive)

Per phase-1-foundation ADR-0001 ("complete scaffolding in place — don't
refactor") and the parent-task instruction to *preserve* phase-1-foundation
scaffolding. Grep confirmed before any new scaffold:

- **Exists already** — do NOT overwrite:
  - 11 newtypes + `ContentHash` + `CorrelationKey` + ID plumbing in
    `crates/overdrive-core/src/id.rs`.
  - 8 trait ports (Clock, Transport, Entropy, Dataplane, Driver,
    IntentStore, ObservationStore, Llm) in
    `crates/overdrive-core/src/traits/`.
  - `LocalStore` in `crates/overdrive-store-local/`.
  - `SimObservationStore` + other `Sim*` adapters + invariant catalogue
    + DST harness in `crates/overdrive-sim/`.
  - `xtask dst`, `xtask dst_lint` wired in `xtask/src/`.
  - CLI scaffolding (clap Subcommand tree) in
    `crates/overdrive-cli/src/main.rs` — handlers are
    `tracing::warn!("command not yet wired…")` stubs; US-05 fills them in.

- **New scaffolds this wave creates** (files materialised for DELIVER to
  fill in; bodies are `panic!("Not yet implemented -- RED scaffold")`):
  - `crates/overdrive-core/src/aggregate/mod.rs` — `Job`, `Node`,
    `Allocation`, `Policy`, `Investigation`, `AggregateError`,
    `IntentKey` (per ADR-0011).
  - `crates/overdrive-core/src/reconciler.rs` — `Reconciler` trait,
    `Action` enum, `ReconcilerName` newtype, `State`, `Db`, `TargetResource`,
    `WorkflowSpec` (per ADR-0013).
  - `crates/overdrive-control-plane/Cargo.toml` — new crate,
    `crate_class = "adapter-host"` per DESIGN wave-decisions
    (renamed from `adapter-real` per ADR-0016).
  - `crates/overdrive-control-plane/src/lib.rs` — module skeleton
    (`api`, `handlers`, `tls_bootstrap`, `error`, `reconciler_runtime`,
    `eval_broker`, `libsql_provisioner`, `observation_wiring`).

- **Not created this wave** (DELIVER owns):
  - Actual DST invariant bodies for the three new invariants
    (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`,
    `ReconcilerIsPure`). The invariant enum variants ARE scaffolded so
    `cargo xtask dst --only <name>` resolves, but the evaluators panic
    until the crafter translates DST scenarios in §5.

**Scaffold inventory** (every `// SCAFFOLD: true` marker DELIVER will
eventually replace):

| File | Symbols marked `SCAFFOLD: true` |
|---|---|
| `crates/overdrive-core/src/aggregate/mod.rs` | `Job::from_spec`, `Node::new`, `Allocation::new`, `Policy` (stub), `Investigation` (stub), `AggregateError`, `IntentKey::for_job`, `IntentKey::for_node`, `IntentKey::for_allocation`, `JobSpecInput` (TOML shape), `NodeSpecInput`, `AllocationSpecInput` |
| `crates/overdrive-core/src/reconciler.rs` | `Reconciler` trait, `Action` enum, `ReconcilerName::from_str`, `ReconcilerName::new`, `State`, `Db`, `TargetResource::from_str`, `WorkflowSpec` (stub) |
| `crates/overdrive-control-plane/src/lib.rs` | `run_server`, `ServerConfig`, `bootstrap_tls`, `noop_heartbeat` reconciler (public factory) |
| `crates/overdrive-control-plane/src/api.rs` | `SubmitJobRequest`, `SubmitJobResponse`, `JobDescription`, `ClusterStatus`, `AllocStatusResponse`, `NodeList`, `ErrorBody` |
| `crates/overdrive-control-plane/src/handlers.rs` | `submit_job`, `describe_job`, `cluster_status`, `alloc_status`, `node_list` |
| `crates/overdrive-control-plane/src/error.rs` | `ControlPlaneError`, `to_response` |
| `crates/overdrive-control-plane/src/tls_bootstrap.rs` | `mint_ephemeral_ca`, `write_trust_triple`, `load_server_tls_config` |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | `ReconcilerRuntime::new`, `ReconcilerRuntime::register`, `ReconcilerRuntime::registered`, broker interface surface |
| `crates/overdrive-control-plane/src/eval_broker.rs` | `EvaluationBroker::new`, `submit`, `drain_pending`, `reap_cancelable`, `BrokerCounters` |
| `crates/overdrive-control-plane/src/libsql_provisioner.rs` | `provision_db_path`, `open_db` |
| `crates/overdrive-control-plane/src/observation_wiring.rs` | `wire_single_node_observation` |
| `crates/overdrive-core/src/lib.rs` | `pub mod aggregate;` + `pub mod reconciler;` additions (plumb the new modules into the crate root) |
| `xtask/src/dst.rs` | invariant enum variants `AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, `ReconcilerIsPure` plus their `ALL`-list entries and `as_canonical` arms — bodies panic |

Workspace-level edits (declared but not compiled by this wave):

- Add `crates/overdrive-control-plane` to `members` in the root
  `Cargo.toml`.
- Add `axum`, `axum-server`, `utoipa`, `utoipa-axum`, `libsql` entries
  to `[workspace.dependencies]` with the versions proposed in DESIGN
  wave-decisions §Tech stack.

The crafter runs `cargo check` post-scaffold. The workspace will
compile once deps resolve (or it flags a `CLARIFICATION_NEEDED` for
any version conflict).

## DWD-07 — Scenario title discipline

Every scenario title describes an observable outcome in the engineer's
or operator's vocabulary. Bad shapes rejected — carried from
phase-1-foundation DWD-07:

- Function-name framing ("`test_submit_job_returns_200`").
- Method-name framing ("`Job::from_spec returns Ok`").
- Technical-flow framing ("End-to-end submit flow through all layers").

Good shapes accepted:

- "Ana submits a job and sees the commit index echoed back".
- "Server rejects a malformed spec before any store write".
- "Reconciler primitive is alive after control-plane boot".

## DWD-08 — Story-to-scenario traceability tagging

Every scenario carries `@us-XX` naming the originating user story
(US-01 through US-05). A single scenario validating across multiple
stories carries each tag. Scenarios derived from the journey carry
`@journey:submit-a-job` alongside the covered `@us-XX` tags.

## DWD-09 — Mandatory adapter coverage

Per Mandate 6 (hexagonal boundary enforcement — adapter coverage gate),
every driven adapter new-to-this-feature has a real-I/O integration
scenario (walking skeleton OR dedicated). Coverage table:

| Adapter | New-this-feature? | Covered by |
|---|---|---|
| `LocalStore` (redb file) | Pre-existing — gains `commit_index()` accessor | §4.5 commit_index monotonic; §3.1 WS submit-then-describe |
| `SimObservationStore` wired as Phase 1 server impl (ADR-0012) | NEW wiring | §4.7 empty `alloc_status`; §4.8 empty `node_health`; §6.2 empty CLI state |
| `rcgen` ephemeral CA (ADR-0010) | NEW | §2.1 first-boot writes trust triple; §2.2 multi-SAN cert; §2.3 re-init re-mints |
| `axum` + `rustls` server (ADR-0008) | NEW | §1.1 WS-1 submit round-trip over real HTTPS; §3.5 SIGINT drain |
| `utoipa` OpenAPI schema derivation (ADR-0009) | NEW | §3.6 schema byte-identical to checked-in; §3.7 handler drift fails `openapi-check` |
| `reqwest` CLI client (ADR-0014) | NEW | §6.1 round-trip; §6.3 connection refused actionable error |
| `libsql` per-primitive memory (ADR-0013) | NEW | §5.4 isolated DB paths; §5.5 `alpha` cannot read `beta`'s DB |
| Reconciler runtime + EvaluationBroker (ADR-0013) | NEW | §5.1 registry non-empty; §5.2 broker collapses duplicates; §5.3 reaper bounds set |

Every row PRESENT. No `@requires_external` markers needed — every
adapter in Phase 1 is local.

## DWD-10 — Error-path ratio target (≥40%)

Target: at least 40% of scenarios are `@error-path` or `@property` with
invariant-red generator coverage. See `acceptance-review.md` §1 for the
raw count; effective count including `@property` boundary coverage is
≥40%.

## DWD-11 — KPI contracts soft gate

`docs/product/kpi-contracts.yaml` does not exist. Warning logged
(skill graceful-degradation rule). No `@kpi-contract` tags emitted.
Feature-level KPIs K1–K7 from `discuss/outcome-kpis.md` drive `@kpi KN`
tags; that is the current KPI surface. When product-level KPI
contracts land (post Phase 1), Sentinel (acceptance-designer-reviewer)
may re-audit and propose `@kpi-contract` additions without breaking
anything here.

## DWD-12 — Environment coverage (graceful degradation)

`docs/feature/phase-1-control-plane-core/devops/` does not exist.
Default environment matrix applied per skill:

| Environment | WS coverage |
|---|---|
| `clean` | §1.1 "freshly started control-plane on clean `tempfile::TempDir`" ✅ |
| `with-pre-commit` | N/A — phase-1-control-plane-core does not ship pre-commit hooks; nothing to conflict with |
| `with-stale-config` | §2.4 "Ana has a pre-existing `~/.overdrive/config` from a previous cluster that is no longer running" — addressed via the re-init-re-mints behaviour (ADR-0010) |

DEVOPS wave may refine; any refinement is additive and cannot block
scenarios here.

---

## Cross-wave reconciliation record

| Delta | Source | Action |
|---|---|---|
| Transport pivot gRPC → REST+OpenAPI | DISCUSS UC-1 + ADR-0008 | Scenarios written against REST endpoints + JSON bodies; zero `grpc` / `tonic` references in test-scenarios |
| `JobSpec` placeholder collision | DISCUSS Key Decision 6 + ADR-0011 | Intent-side `Job` in `overdrive-core::aggregate`; observation-side `AllocStatusRow` unchanged; US-01 scenarios reference `Job::from_spec` and never `JobSpec` |
| Phase 1 ObservationStore impl | DISCUSS Key Decision 8 + ADR-0012 | §4.7 / §4.8 scenarios read via the `SimObservationStore`-wired server; empty-row assertions are about the wiring, not the sim |
| Slice 4 ships whole | DISCUSS Key Decision 7 + ADR-0013 §7 | US-04 scenarios cover trait + broker + libSQL + DST invariants in one story; no §5.X is split-4A-only |
| Byte-identical re-submit idempotent | ADR-0015 | §4.9 asserts 200 OK on re-submit of same spec; §4.10 asserts 409 on different spec at same key |
| `utoipa` over `aide` | ADR-0009 | §3.6/§3.7 scenarios reference `cargo xtask openapi-check`, not `aide` machinery |
| Hand-rolled reqwest client over Progenitor | ADR-0014 | US-05 scenarios reference CLI behaviours, never a generated client artifact |
| Ephemeral in-process CA, no `--insecure` | ADR-0010 | §2.X scenarios cover CA bootstrap, trust-triple write, multi-SAN, re-init; grep-gate for `--insecure` in §2.5 |

No contradictions surfaced. `CLARIFICATION_NEEDED` not required.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DISTILL wave decisions for phase-1-control-plane-core. 12 DWDs + reconciliation + scaffold inventory. |
| 2026-04-26 | Amendment — `cluster init` removed from Phase 1 (commit `d294fb8`). DWD-01 strategy-C litmus revised to read `serve` as the sole CA-minting site; banner added at top pointing to ADR-0010 §Amendment 2026-04-26 and GH #81. RCA: `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`. |
