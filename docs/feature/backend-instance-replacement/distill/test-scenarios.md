# Test scenarios — `backend-instance-replacement`

**Wave**: DISTILL | **Mode**: PROPOSE | **Designer**: Quinn (nw-acceptance-designer) | **Date**: 2026-06-30 | **Feature**: GH #249

> **Executable acceptance specification.** This document is the
> GIVEN/WHEN/THEN **SSOT** for the feature — **no `.feature` files** (per
> `.claude/rules/testing.md` § "No `.feature` files anywhere"). DELIVER's RED
> phase translates each scenario into a Rust `#[test]` / `#[tokio::test]` body
> or a proptest, scaffolded RED (`#[should_panic(expected = "RED scaffold")]` on
> the test, `todo!("RED scaffold: …")` on production), per the Scaffold MANIFEST
> in `feature-delta.md` § "Wave: DISTILL / [REF] Scaffold MANIFEST". **No file is
> written under `crates/` this wave.**
>
> **Contract this distills**: **ADR-0073** (`overdrive workload restart <id>` +
> the desired-run generation precursor + the current-instance-scoped veto) and
> the `## Wave: DESIGN` section of `feature-delta.md`. `[D1]` is CLOSED. The six
> pinned signatures + the R1–R5 reconciler state machine in ADR-0073 are the
> contract; DISTILL picks NO new signature (CLAUDE.md "never invent API surface").
>
> **Lang**: Rust (`[lang-mode] rust`). **Policy**: `inherit`
> (`docs/architecture/atdd-infrastructure-policy.md` exists; the BIR rows appended
> below).

## Wave-Decision Reconciliation HARD GATE — PASS (0 contradictions)

Ran before scenario writing (per `nw-distill` § "Wave-Decision Reconciliation
HARD GATE"). Files read: the DISCUSS `[REF]` sections + DISCUSS Wave-Decisions in
`feature-delta.md`; `design/wave-decisions.md`; ADR-0073. **No `devops/`
directory** → WARN, default environment matrix used (the Tier-3 obligation is
folded into the DESIGN/oracle gate; not a blocker).

Checked each DISCUSS decision against DESIGN:

| DISCUSS decision / invariant | DESIGN (ADR-0073) | Verdict |
|---|---|---|
| Mechanism = explicit lifecycle verb; `deploy` stays pure-declare | Verb `overdrive workload restart <id>`; `deploy` never bumps generation (DDD-7) | **CONSISTENT** |
| Inv. 1 — NEW AllocationId + NEW `workload_addr` | `mint_alloc_id(attempt = allocs_vec.len())` mints `payments-1`, new `/30` | **CONSISTENT** |
| Inv. 2 — `jobs/<id>` intent stays declared | handler never deletes `workloads/<id>`; only bumps gen + deletes `/stop` | **CONSISTENT** |
| Inv. 3 — operator-stop sentinel cleared so a fresh instance is provisioned | `Delete{stop_key}` in the txn **AND** the generation gate (clearing alone is necessary-but-not-sufficient — the observed Operator row persists; DDD-6) | **CONSISTENT** (DESIGN *deepens* the mechanism the DISCUSS gap table already flagged as insufficient-on-its-own — a refinement, not a contradiction) |
| Inv. 4 — `F`-binding byte-stable across the cycle | `FrontendAddrAllocator` idempotent `assign` untouched | **CONSISTENT** |
| Inv. 5 — honest 404 on no-such-workload | `ControlPlaneError::NotFound { resource: "workloads/<id>" }` (DDD-8) | **CONSISTENT** |
| Inv. 6 — `overdrive deploy` remains pure-declare | DDD-7 (Bug-3 preserved) | **CONSISTENT** |

DESIGN's own back-propagation note (`design/wave-decisions.md` § "DISCUSS
assumptions changed"): **"None."** Reconciliation **PASS**.

---

## Scope + strategy

**Scope**: the production `overdrive workload restart <id>` verb + the
generation precursor + the current-instance-scoped reconciler veto, proven
end-to-end against the **three #249-deferred Tier-3 oracle ATs** (already
authored, `#[ignore]`'d). Covers **US-BIR-1** (new instance, intent retained),
**US-BIR-2** (stable `F` across the cycle, in-flight churn fail-fast), the
restart **cardinality** contract (atomic monotonic bump + level-triggered
coalescing), and the post-iteration-3 **regression** (a fresh-alloc crash after
restart must NOT wedge on a superseded operator-stop row).

**Strategy** (tiers per `.claude/rules/testing.md`):

- **Tier 1 — store-acceptance** (real `redb` via `LocalIntentStore`, gated
  `integration-tests`): the `TxnOp::IncrementU64` atomic-monotonic contract
  (`S-BIR-TXN-*`).
- **Tier 1 — pure reconciler decision logic** (`reconcile()` is pure-sync,
  default lane, `overdrive-core/tests/acceptance/`): the generation-gated
  placement, coalescing/sequential cardinality, the current-instance-scoped
  veto + R1-crash regression, the `current_alloc` helper (`S-BIR-RESTART-*`,
  `S-BIR-COALESCE-PLACE`, `S-BIR-COALESCE-NO-REPLAY`, `S-BIR-SEQUENTIAL`,
  `S-BIR-STOP-ONCE`, `S-BIR-REGRESSION-*`, `S-BIR-BUG3-PRESERVED`,
  `S-BIR-CURRENT-ALLOC`).
- **Tier 1/2 — handler** (`overdrive-control-plane/tests/acceptance/`, mirrors
  `job_stop_*`): the 404 posture, the atomic bump+clear txn shape, the cosmetic
  outcome label (`S-BIR-HANDLER-*`).
- **Integration — CLI driving adapter** (`overdrive-cli/tests/integration/`, gated
  `integration-tests`, **direct handler-call** against an in-process `run_server` —
  NO subprocess, per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
  subprocess"): `overdrive workload restart` typed `RestartOutput`
  (`S-BIR-CLI-RESTART-SUCCESS`) + the unknown-id `CliError` mapped to a non-zero
  exit (`S-BIR-CLI-RESTART-UNKNOWN`).
- **Tier 3 — the oracle** (real-kernel Lima, gated, ALREADY AUTHORED, un-ignored
  by slice-04): `S-DBN-WS-STABLE`, `S-DBN-CHURN`, `S-DBN-NXDOMAIN-02-RECOVERY`.
- **No Tier 2** — there is no kernel-side eBPF program new to this feature; the
  in-flight churn surface is the reused Tier-3 intercept worker. No
  `BPF_PROG_TEST_RUN` target.

**Driving ports** (entry points exercised): `overdrive workload restart <id>`
(`POST /v1/jobs/:id/restart` → `restart_workload`) — the NEW production entry
point; `overdrive serve` (`run_server`); `overdrive deploy` (`POST /v1/jobs`);
`overdrive alloc status` (the new-AllocationId observable); `getaddrinfo`/`getent`
(the Tier-3 name-path signal — the oracle).

**Error-path coverage**: **14 / 24 ≈ 58 %** (≥40 % target met — 404, the unknown-id
CLI path, corrupt-row decode, the absent-key edge, the two coalescing concurrency
edges, the two crash-regression failure modes, the Bug-3 must-refuse guard,
`current_alloc` numeric-vs-lexical, the no-duplicate-stop guard, the churn
fail-fast, the NXDOMAIN-while-stopped recovery).

## Environment mapping (no feature-local `devops/` — tier model adopted)

There is **no `docs/feature/backend-instance-replacement/devops/` directory** →
the generic-skill default matrix (`clean` / `with-pre-commit` / `with-stale-config`)
applies as a fallback. Per the precedent of earlier Overdrive features, **the
codebase's test-tier model is the real environment taxonomy** for this
control-plane feature — the default matrix's config-installer environments are
mostly *waived* (this feature touches no pre-commit hook and no config-migration
surface). Each default-matrix environment is mapped or explicitly waived, and
the real (tier-based) environments are named:

| Default-matrix env | Mapping / waiver | Rationale |
|---|---|---|
| `clean` | **mapped** → `S-BIR-CLI-RESTART-SUCCESS`/`-UNKNOWN` (fresh `tmp_path` trust triple + a fresh in-process `run_server` on an ephemeral port — direct handler-call, no subprocess), the three `S-DBN-*` oracle ATs (fresh netns + pinned-6.18 Lima) | the feature's real "clean" environment is a fresh tempdir + a freshly-booted in-process server / pinned-kernel Lima boot, not an installer clean-install |
| `with-pre-commit` | **waived** | the feature touches no pre-commit hook / git-hook surface |
| `with-stale-config` | **waived** | the feature has no config-migration or stale-config-upgrade surface (the generation key is greenfield; single-cut greenfield migration per project policy) |

Real (tier-based) environments the scenarios actually exercise:

| Tier env | Scenarios | Mechanism |
|---|---|---|
| Tier-1 in-process (default lane) | `S-BIR-RESTART-*`, `S-BIR-STOP-ONCE`, `S-BIR-COALESCE-*`, `S-BIR-SEQUENTIAL`, `S-BIR-REGRESSION-*`, `S-BIR-BUG3-PRESERVED`, `S-BIR-CURRENT-ALLOC`, `S-BIR-HANDLER-*` | pure `reconcile()` / in-process axum handler; no real I/O |
| Tier-1 store-acceptance (gated `integration-tests`, real redb) | `S-BIR-TXN-01..04` | `LocalIntentStore` over a `TempDir` |
| Integration — CLI handler (gated `integration-tests`, in-process) | `S-BIR-CLI-RESTART-SUCCESS`/`-UNKNOWN` | **direct handler-call** `commands::workload::restart(RestartArgs{ id, config_path })` against a fresh in-process `run_server` on an ephemeral port (trust triple written by `overdrive serve`); typed `RestartOutput` / `CliError` — NO subprocess, NO `CARGO_BIN_EXE_overdrive` (`crates/overdrive-cli/CLAUDE.md`) |
| Tier-3 real-kernel (gated, pinned-6.18 Lima as root — the MERGE GATE) | `S-DBN-WS-STABLE`, `S-DBN-CHURN`, `S-DBN-NXDOMAIN-02-RECOVERY` | `cargo xtask lima run --` + real `run_server` + netns + `getent`/connect |

---

## Scenario index

| ID | Title | Tags | Tier | Story / AC | Mutation target |
|---|---|---|---|---|---|
| S-BIR-TXN-01 | Single restart bump-and-clear advances generation 0→1 | `@store` `@real-io` | 1-store | US-BIR-1 AC4 / DDD-9 / K-BIR-1 | — |
| S-BIR-TXN-02 | N concurrent bumps leave generation == N (monotonic, no lost bump) | `@store` `@real-io` `@property` `@concurrency` | 1-store | US-BIR-1 AC4 / DDD-9 / K-BIR-1 | **yes** (`+1`→`+0`) |
| S-BIR-TXN-03 | Absent generation key reads 0 and bumps to 1; absent stop delete is a no-op | `@store` `@real-io` `@error_path` | 1-store | US-BIR-1 AC4 / DDD-9 | yes (absent→default) |
| S-BIR-TXN-04 | A corrupt / short generation row decodes to 0 and bumps to 1 | `@store` `@real-io` `@error_path` | 1-store | US-BIR-1 AC4 / DDD-9 | yes (defensive decode) |
| S-BIR-RESTART-STOPPED | A stopped workload's restart places a fresh instance, intent retained | `@reconciler` `@in-memory` `@kpi` | 1 | US-BIR-1 AC2/3/4 / K-BIR-1 | yes (gate predicate) |
| S-BIR-RESTART-RUNNING-STOP | A running workload's restart stops the current instance (no stamp) | `@reconciler` `@in-memory` | 1 | US-BIR-1 AC2 (R2) | yes (R2 stop, no stamp) |
| S-BIR-RESTART-RUNNING-PLACE | Once the old instance is Terminated, the restart places a fresh one and stamps | `@reconciler` `@in-memory` | 1 | US-BIR-1 AC2/3 (R3) | yes (R3 place + stamp) |
| S-BIR-STOP-ONCE | A running-origin restart emits exactly one stop across the draining ticks | `@reconciler` `@in-memory` `@error_path` | 1 | US-BIR-1 AC2 (R2→R5) | yes (duplicate-stop) |
| S-BIR-COALESCE-PLACE | Two pre-placement restarts place exactly one fresh instance, stamp to latest generation | `@reconciler` `@in-memory` `@concurrency` | 1 | DDD-10 / K-BIR-1 | yes (stamp `=desired`) |
| S-BIR-COALESCE-NO-REPLAY | After `observed == desired`, a follow-up reconcile emits no second instance | `@reconciler` `@in-memory` `@concurrency` | 1 | DDD-10 / K-BIR-1 | yes (stamp `=desired`) |
| S-BIR-SEQUENTIAL | A restart issued after the prior placement re-enters the cycle | `@reconciler` `@in-memory` | 1 | DDD-10 / K-BIR-1 | yes (`<` boundary) |
| S-BIR-REGRESSION-STOPPED | A fresh instance that crashes after a stopped-origin restart is crash-restarted, not wedged | `@reconciler` `@in-memory` `@error_path` `@regression` | 1 | US-BIR-1 / DDD-13 / R1-crash | **yes** (`any`→`current_alloc`) |
| S-BIR-REGRESSION-RUNNING | A fresh instance that crashes after a running-origin restart is crash-restarted, not wedged | `@reconciler` `@in-memory` `@error_path` `@regression` | 1 | US-BIR-1 / DDD-13 / R1-crash | **yes** (`any`→`current_alloc`) |
| S-BIR-BUG3-PRESERVED | A same-spec deploy does NOT resurrect an operator-stopped workload | `@reconciler` `@in-memory` `@error_path` `@regression` | 1 | DDD-7 / Bug-3 | **yes** (scoped veto must fire) |
| S-BIR-CURRENT-ALLOC | The current instance is the numerically-highest alloc suffix, not the lexical max | `@reconciler` `@in-memory` `@property` | 1 | DDD-13 | **yes** (numeric vs lexical) |
| S-BIR-HANDLER-404 | A restart on a non-existent workload is rejected with a 404, no allocation created | `@driving_adapter` `@error_path` | 1/2 | US-BIR-1 AC5 (404) | yes (404 posture) |
| S-BIR-HANDLER-TXN | A restart commits one atomic bump+clear txn and retains the intent row | `@driving_adapter` | 1/2 | US-BIR-1 AC3/4 | yes (txn op set) |
| S-BIR-HANDLER-OUTCOME-RESUMED | A restart on a stopped workload reports outcome `resumed` | `@driving_adapter` | 1/2 | DDD-11 | yes (label classification) |
| S-BIR-HANDLER-OUTCOME-RESTARTED | A restart on a running workload reports outcome `restarted` | `@driving_adapter` | 1/2 | DDD-11 | yes (label classification) |
| S-BIR-CLI-RESTART-SUCCESS | `overdrive workload restart <id>` returns the new instance | `@driving_adapter` `@real-io` | int (in-process) | US-BIR-1 | yes (RestartOutput / dispatch) |
| S-BIR-CLI-RESTART-UNKNOWN | `overdrive workload restart <unknown>` errors with not-found | `@driving_adapter` `@real-io` `@error_path` | int (in-process) | US-BIR-1 AC5 (404) | yes (CliError → exit code) |
| S-DBN-WS-STABLE | The name re-resolves the same `F` across the cycle and the next connect lands the new backend | `@real-io` `@frontend` `@churn` `@kpi` `@oracle` | 3 | US-BIR-1 + US-BIR-2 / K-BIR-1/2 | (oracle — already authored) |
| S-DBN-CHURN | An in-flight connection fails fast on backend churn; the next dial lands the new backend | `@real-io` `@churn` `@error_path` `@kpi` `@oracle` | 3 | US-BIR-2 / K-BIR-3 | (oracle — already authored) |
| S-DBN-NXDOMAIN-02-RECOVERY | A recovered workload re-resolves the same stable `F` (withhold-not-release) | `@real-io` `@error_path` `@frontend` `@kpi` `@oracle` | 3 | US-BIR-1 + US-BIR-2 / K-BIR-2 | (oracle — already authored) |

---

## Tier 1 — store-acceptance (`TxnOp::IncrementU64`, real redb)

> Home: `crates/overdrive-store-local/tests/acceptance/txn_increment_u64.rs`
> (NEW; gated `integration-tests`; real `LocalIntentStore` over a `TempDir`).
> Sibling precedent: `put_if_absent.rs`, `local_store_basic_ops.rs`.
> Contract: ADR-0073 § "The six pinned signatures" item 4 + the `TxnOp::IncrementU64`
> trait behavior contract (`development.md` § "Trait definitions specify behavior").
> The generation value codec is `u64::to_be_bytes` / `from_be_bytes`; absent or
> short ⇒ read as 0.

### S-BIR-TXN-01 — Single restart bump-and-clear advances generation 0→1

```gherkin
@store @real-io
Scenario: One atomic txn bumps the generation and clears the stop sentinel
  Given a LocalIntentStore with no "workloads/payments/generation" key
  And a present "workloads/payments/stop" sentinel
  When a single txn[IncrementU64{gen_key}, Delete{stop_key}] commits
  Then the txn returns Committed
  And get("workloads/payments/generation") decodes (big-endian u64) to exactly 1
  And get("workloads/payments/stop") returns None
```

- **Universe** (observable, port-exposed): `store.get(gen_key)` decoded BE u64;
  `store.get(stop_key)`; the `TxnOutcome`.
- **Notes**: the increment is read-modify-write **inside** the same write txn;
  `LocalIntentStore::txn` returns `Committed` unconditionally (no `Conflict`
  retry — DDD-9). The atomicity claim is that no observer sees the gen bumped
  without the stop cleared, or vice versa.
- **Expected RED**: `MISSING_FUNCTIONALITY` — `TxnOp::IncrementU64` and its
  `redb_backend` match arm do not exist yet.

### S-BIR-TXN-02 — N concurrent bumps leave generation == N (monotonic, no lost bump)

```gherkin
@store @real-io @property @concurrency
Scenario: Concurrent restart bumps never lose an increment
  Given a LocalIntentStore with no "workloads/payments/generation" key
  When N concurrent tasks each commit txn[IncrementU64{gen_key}, Delete{stop_key}]
  Then every txn returns Committed
  And the final get(gen_key) decodes to exactly N
  And the value never observed going backwards
```

- **Universe**: final `store.get(gen_key)` decoded BE u64 == N (e.g. N = 16 or
  32); the per-task `TxnOutcome`.
- **This is the load-bearing concurrency proof** (ADR-0073 item 4). redb
  serialises writers, so each read-modify-write sees the prior committed value.
- **Mutation target (mandatory)**: a mutation swapping the inner `+1 → +0` (or
  dropping the saturating add, or reading a stale snapshot instead of the live
  row) MUST be killed by this test — `.claude/rules/testing.md` § "Mutation
  testing" ("Hash determinism paths" / store primitives).
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-TXN-03 — Absent generation key reads 0 and bumps to 1; absent stop delete is a no-op

```gherkin
@store @real-io @error_path
Scenario: A first restart on a never-bumped workload starts the generation at 1
  Given a LocalIntentStore with neither a generation key nor a stop sentinel
  When a txn[IncrementU64{gen_key}, Delete{stop_key}] commits
  Then get(gen_key) decodes to 1
  And the Delete of an already-absent stop_key is a no-op (Committed, not an error)
```

- **Universe**: `store.get(gen_key)` == 1; `TxnOutcome == Committed`.
- **Edge**: the absent-key read MUST default to 0 (then +1), and `Delete` of an
  absent key is idempotent (the running-origin restart deletes a `/stop` that
  was never written).
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-TXN-04 — A corrupt / short generation row decodes to 0 and bumps to 1

```gherkin
@store @real-io @error_path
Scenario: A malformed generation row is treated as 0, not a panic
  Given a LocalIntentStore whose "workloads/payments/generation" key holds a 3-byte value
  When a txn[IncrementU64{gen_key}] commits
  Then the read defends against the short slice (decodes to 0)
  And get(gen_key) decodes to 1
```

- **Universe**: `store.get(gen_key)` == 1 (NOT a panic, NOT garbage).
- **Edge** (per `development.md` § "Safe byte-slice access"): the BE-u64 decode
  must use a length-checked accessor (`<[u8;8]>::try_from(..).map(u64::from_be_bytes)`
  with a `0` fallback on a non-8-byte slice), never `bytes[0..8]` indexing.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

---

## Tier 1 — reconciler decision logic (pure `reconcile()`)

> Home: `crates/overdrive-core/tests/acceptance/workload_lifecycle_restart.rs`
> (NEW; default lane). Sibling precedent:
> `workload_lifecycle_reconcile_branches.rs`, `workload_lifecycle_terminal_decision.rs`,
> `workload_lifecycle_backoff.rs`. `reconcile()` is pure-sync
> (`workload_lifecycle.rs:120`); these scenarios construct `(desired, actual,
> view, tick)` and assert on the returned `(Vec<Action>, NextView)` — NO real
> I/O, NO clock read. The added inputs: `desired.generation: u64`,
> `view.observed_generation: u64` (`#[serde(default)]`). `restart_pending =
> view.observed_generation < desired.generation`. The veto is the
> **current-instance-scoped** form (ADR-0073 § 5 / DDD-6/DDD-13):
> `!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)`
> — NOT `allocs_vec.iter().any(is_operator_stopped)` (the line-520 form being
> replaced).

### S-BIR-RESTART-STOPPED — A stopped workload's restart places a fresh instance, intent retained (US-BIR-1, R4)

```gherkin
@reconciler @in-memory @kpi
Scenario: Stopped-origin restart places a fresh instance
  Given a declared workload "payments" whose only alloc is "payments-0", Terminated{by: Operator}
  And the desired generation is 1 and the observed generation is 0 (restart_pending)
  When the reconciler reconciles
  Then it emits StartAllocation for a fresh alloc "payments-1" (A1 ≠ A2, new /30)
  And the next View stamps observed_generation = 1
  And the "workloads/payments" intent is untouched (no Delete in the action set)
```

- **Universe** (port-exposed observables on the returned tuple): the `Vec<Action>`
  contains exactly one `StartAllocation` whose minted id ≠ `payments-0`; the
  returned `NextView.observed_generation == 1`; no `Action` withdraws intent.
- **R4** of the ADR-0073 table (operator-stopped origin, no intervening stop).
  `mint_alloc_id(attempt = allocs_vec.len())` mints `payments-1`.
- **Mutation target**: a mutation that drops the `restart_pending` gate (so the
  veto fires unconditionally on the `payments-0/Operator` row) leaves the
  workload stopped — killed here.
- **Expected RED**: `MISSING_FUNCTIONALITY` (the `generation`/`observed_generation`
  fields + the gate do not exist).

> **R2→R3 split (review-distill High / GWT one-action).** The running-origin
> restart is a two-tick trajectory (R2 stop, then R3 place once Terminated). To
> keep one behaviour per scenario, it is split into S-BIR-RESTART-RUNNING-STOP
> (the R2 stop tick) and S-BIR-RESTART-RUNNING-PLACE (the R3 placement tick).
> Each has a single `When`. S-BIR-STOP-ONCE (below) covers the R5 no-duplicate-stop.

### S-BIR-RESTART-RUNNING-STOP — A running workload's restart stops the current instance, no stamp (US-BIR-1, R2)

```gherkin
@reconciler @in-memory
Scenario: Running-origin restart stops the current instance first
  Given a declared workload "coinflip" whose alloc "coinflip-0" is Running
  And the desired generation is 1 and the observed generation is 0 (restart_pending)
  When the reconciler reconciles
  Then it emits exactly one StopAllocation for "coinflip-0" with terminal Stopped{by: Operator}
  And the next View does NOT stamp observed_generation (still 0 — the fresh instance has not been placed)
```

- **Universe**: action set == `[StopAllocation{coinflip-0, Stopped{Operator}}]`;
  `NextView.observed_generation == 0` (unchanged).
- **R2**. The stamp must NOT happen on the stop tick — stamping here would re-arm
  the veto before the fresh instance exists (the load-bearing ordering, ADR-0073
  § 5).
- **Mutation target**: a mutation that stamps `observed_generation = desired` on
  the stop tick (R2) strands the workload Terminated — killed by the
  no-stamp assertion here (paired with S-BIR-RESTART-RUNNING-PLACE's placement).
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-RESTART-RUNNING-PLACE — Once the old instance is Terminated, the restart places a fresh one and stamps (US-BIR-1, R3)

```gherkin
@reconciler @in-memory
Scenario: Running-origin restart places the fresh instance once the old one is Terminated
  Given a running-origin restart already stopped "coinflip-0" (now Terminated{by: Operator}, no Running alloc remains)
  And the desired generation is 1 and the observed generation is 0 (restart_pending)
  When the reconciler reconciles
  Then it emits StartAllocation for a fresh "coinflip-1" (A1 ≠ A2, new /30)
  And the next View stamps observed_generation = 1
```

- **Universe**: action set == `[StartAllocation{coinflip-1}]`;
  `NextView.observed_generation == 1`.
- **R3**. The placement tick is the only tick that stamps. Together with
  S-BIR-RESTART-RUNNING-STOP this is the R2→R3 stop-then-place sequencing.
- **Mutation target**: a mutation that fails to place (or that stamps without
  placing) — killed here.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-STOP-ONCE — A running-origin restart emits exactly one stop across the draining ticks (R2→R5)

```gherkin
@reconciler @in-memory @error_path
Scenario: No duplicate StopAllocation while the old instance drains
  Given a running-origin restart that emitted StopAllocation for "coinflip-0" on tick 1
  When the reconciler reconciles again while "coinflip-0" is still draining (not yet Terminated)
  Then it emits NO second StopAllocation (the prior stop is in flight)
  And observed_generation is still unstamped
```

- **Universe**: tick-2 action set contains zero `StopAllocation` for the
  still-draining alloc; `NextView.observed_generation` unchanged.
- **R5**. The no-duplicate-stop requirement is made explicit so this focused
  state-machine test pins it; the broker `(reconciler, target)` keying +
  in-flight-action collapse already debounce, but the test guards the contract.
- **Mutation target**: a mutation that re-emits `StopAllocation` every tick while
  draining (thrashing) — killed here.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

> **Coalescing split (review-distill rev3 High / GWT one-action).** The
> level-triggered coalescing contract is a two-tick property (place once for the
> latest generation, then NO replay on the follow-up tick). To keep one driving
> `reconcile()` action per scenario it is split into S-BIR-COALESCE-PLACE (the
> single placement that stamps `observed = desired`) and S-BIR-COALESCE-NO-REPLAY
> (a follow-up reconcile emits no second placement). Together they kill the
> `observed + 1` mutation: under `observed + 1` the placement would leave
> `observed (1) < desired (2)`, so S-BIR-COALESCE-NO-REPLAY would see a second
> `StartAllocation` and fail.

### S-BIR-COALESCE-PLACE — Two pre-placement restarts place exactly one fresh instance and stamp to the latest generation (level-triggered, DDD-10)

```gherkin
@reconciler @in-memory @concurrency
Scenario: Concurrent (pre-placement) restarts place one instance for the latest generation
  Given a stopped-origin workload "payments" with observed_generation 0
  And two restarts landed before any placement, advancing desired_generation to 2
  When the reconciler reconciles
  Then it places exactly ONE fresh instance for the latest generation
  And it stamps observed_generation = desired_generation (= 2), NOT observed + 1
```

- **Universe**: the action set contains exactly ONE `StartAllocation`;
  `NextView.observed_generation == 2` (= desired, NOT 1).
- **The level-triggered contract** (ADR-0073 § "Idempotency posture"). The stamp
  is `observed = desired` (NOT `observed + 1`), which is what makes the machine
  coalesce by construction — two pre-placement bumps collapse into one fresh
  instance for the latest generation.
- **Mutation target**: a mutation changing the stamp to `observed + 1` leaves
  `observed (1) < desired (2)` after the placement — caught here (the stamp
  assertion) and again by S-BIR-COALESCE-NO-REPLAY (the follow-up re-place).
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-COALESCE-NO-REPLAY — After the coalesced placement stamps `observed == desired`, a follow-up reconcile emits no second instance (DDD-10)

```gherkin
@reconciler @in-memory @concurrency
Scenario: A coalesced placement does not replay on the next tick
  Given a coalesced placement already stamped observed_generation == desired_generation (= 2) with "payments-1" placed
  When the reconciler reconciles again
  Then restart_pending is false (observed == desired) and it emits NO further StartAllocation
  And the generation never goes backwards (the reconciler never wedges)
```

- **Universe**: the follow-up action set contains zero `StartAllocation`;
  `NextView.observed_generation == 2` (unchanged, never decremented).
- The other half of the coalescing contract: once `observed == desired` the
  machine does not re-place the "skipped" generation (no edge-triggered replay
  queue). Distinct from S-BIR-SEQUENTIAL, where a *new* restart advances `desired`
  beyond `observed` and re-entry is correct.
- **Mutation target**: a mutation that stamps `observed + 1` (so `observed (1) <
  desired (2)`) would emit a second `StartAllocation` here — killed.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-SEQUENTIAL — Two sequential restarts each cycle the workload (DDD-10)

```gherkin
@reconciler @in-memory
Scenario: A restart issued after the prior placement re-enters the cycle
  Given the prior restart placed "payments-1" (Running) and stamped observed_generation = 1
  And a second restart has since advanced desired_generation to 2 (observed 1 < desired 2)
  When the reconciler reconciles
  Then restart_pending is true again, so it re-enters the cycle and emits StopAllocation for the current "payments-1"
  And it does NOT stamp observed_generation on this tick (the fresh "payments-2" has not been placed)
```

- **Universe**: action set == `[StopAllocation{payments-1, Stopped{Operator}}]`;
  `NextView.observed_generation == 1` (unchanged — the second cycle's R2 stop tick).
- Single driving action (`reconcile()`); the second-restart-advanced-generation
  is `Given` context. Pins the sequential-vs-concurrent distinction: a restart
  issued **after** the prior placement stamped `observed` makes `restart_pending`
  true again (`observed 1 < desired 2`) and **re-enters** the cycle — whereas in
  S-BIR-COALESCE-NO-REPLAY `observed == desired` after the single placement, so no
  re-entry.
  The fresh "payments-2" placement is then the (already-covered)
  S-BIR-RESTART-RUNNING-PLACE shape on a later tick; this scenario pins the
  re-entry *decision*, not the re-proof of stop→place.
- **Mutation target**: a mutation flipping the `observed_generation < desired.generation`
  comparison to `<=` or `==` breaks the re-entry (no second `StopAllocation`) —
  killed here.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-REGRESSION-STOPPED — A fresh instance that crashes after a stopped-origin restart is crash-restarted, not wedged (R1-crash, DDD-13)

```gherkin
@reconciler @in-memory @error_path @regression
Scenario: A post-restart crash converges via crash-restart, not the stale veto
  Given a stopped-origin restart placed "payments-1" which reached Running, then CRASHED (terminal Failed / Terminated with a crash reason, NOT Stopped{Operator})
  And the superseded "payments-0", Terminated{by: Operator} row is retained
  And observed_generation == desired_generation (restart_pending is false)
  When the reconciler reconciles
  Then it crash-restarts the fresh instance (emits RestartAllocation for "payments-1" / a new Running converges)
  And it does NOT return an empty action set wedged on the stale "payments-0" / Operator row
```

- **Universe**: the action set contains the crash-restart action for the current
  (crashed) instance; it is NOT empty (the buggy `any(...)` veto returned
  `(Vec::new(), …)` here, wedging forever).
- **R1-crash**. `current_alloc(&allocs_vec)` is the crashed `payments-1` (a crash
  reason, not Operator), so the scoped veto does NOT fire and the Run branch falls
  through to the existing `is_restartable`/backoff branch.
- **Mutation target (mandatory — `.claude/rules/testing.md` § "Reconciler logic")**:
  a mutation reverting the veto to `allocs_vec.iter().any(is_operator_stopped)`,
  or dropping the `current_alloc(...)` scoping, MUST be killed by this case.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-REGRESSION-RUNNING — A fresh instance that crashes after a running-origin restart is crash-restarted, not wedged (R1-crash, DDD-13)

```gherkin
@reconciler @in-memory @error_path @regression
Scenario: A post-restart crash (running origin) converges, not wedges
  Given a running-origin restart cycled "coinflip-0" → fresh "coinflip-1" reached Running, then CRASHED (a crash reason, NOT Stopped{Operator})
  And the now-superseded "coinflip-0", Terminated{by: Operator} row is retained
  And restart_pending is false
  When the reconciler reconciles
  Then it crash-restarts "coinflip-1", not wedged on the superseded "coinflip-0" / Operator row
```

- **Universe**: same shape as S-BIR-REGRESSION-STOPPED, running origin.
- The two regression cases (stopped + running origin) are the two halves the
  iteration-3 fix pins forever; both are mandatory mutation targets for the same
  `any(...) → current_alloc(...)` mutation.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-BUG3-PRESERVED — A same-spec deploy does NOT resurrect an operator-stopped workload (DDD-7)

```gherkin
@reconciler @in-memory @error_path @regression
Scenario: The scoped veto still fires on a CURRENT operator-stop
  Given a declared workload "payments" whose current alloc "payments-0" is Terminated{by: Operator}
  And a same-spec deploy that did NOT bump the generation (observed == desired, restart_pending is false)
  When the reconciler reconciles
  Then the current-instance-scoped veto fires (current_alloc is the operator-stopped "payments-0")
  And no fresh instance is placed (the workload stays stopped)
```

- **Universe**: the action set places no `StartAllocation`; the workload remains
  Terminated.
- The **other half** of the scoped-veto property: the veto must STILL fire when
  the *current* instance is operator-stopped (scoping narrows *which* row arms the
  veto, it never weakens the veto). Bug-3 (`fix-exec-driver-exit-watcher`) is
  preserved.
- **Mutation target**: a mutation that makes the scoped veto never fire (so a
  re-deploy resurrects a stopped workload) — killed here.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-CURRENT-ALLOC — The current instance is the numerically-highest alloc suffix, not the lexical max (DDD-13)

```gherkin
@reconciler @in-memory @property
Scenario: current_alloc picks the latest-placed instance by numeric attempt index
  Given alloc rows "payments-0" … "payments-10" with mixed terminal/running states
  When current_alloc(&rows) is called
  Then it returns the row whose mint_alloc_id attempt suffix is numerically maximal (… "payments-10")
  And NOT the lexical max ("payments-9" sorts after "payments-10" lexically)
```

- **Universe**: `current_alloc(&[&AllocStatusRow])` returns the row with the
  numerically-highest parsed `mint_alloc_id` suffix.
- The grounding fact (`design/wave-decisions.md` § "DISCUSS assumptions changed"):
  `AllocationId` is `Ord` on the raw string, so `BTreeMap`/`.values()` order is
  **LEXICAL** (`alloc-payments-10 < alloc-payments-2`). The helper MUST parse the
  numeric suffix. The never-delete invariant makes attempt indices strictly
  increasing, so the numeric max is unambiguously the current instance.
- **Mutation target (mandatory)**: a mutation using `.values().last()` /
  lexical-max instead of numeric-max picks the wrong "current" instance — killed
  here. This is the helper the whole scoped veto rides on.
- **Expected RED**: `MISSING_FUNCTIONALITY` (`current_alloc` does not exist).

---

## Tier 1/2 — handler (`restart_workload`)

> Home: `crates/overdrive-control-plane/tests/acceptance/restart_workload_unknown.rs`
> + `restart_workload_intent_key.rs` + `restart_workload_outcome.rs` (NEW).
> Sibling precedent: `job_stop_unknown.rs`, `job_stop_intent_key.rs`,
> `job_stop_idempotent.rs`. The handler mirrors `stop_workload`:
> parse → get(`for_workload`) [+ get(`for_workload_stop`) for the label] else 404
> → `txn[IncrementU64{gen}, Delete{stop}]` → enqueue `job-lifecycle` eval → 200.

### S-BIR-HANDLER-404 — A restart on a non-existent workload is rejected with a 404, no allocation created (US-BIR-1 AC5)

```gherkin
@driving_adapter @error_path
Scenario: Restart on an unknown id is an honest 404
  Given no "workloads/nonexistent" aggregate exists
  When restart_workload is invoked for "nonexistent"
  Then it returns ControlPlaneError::NotFound { resource: "workloads/nonexistent" }
  And no IntentStore txn is committed (no generation bump, no sentinel delete)
  And no job-lifecycle evaluation is enqueued
```

- **Universe**: the `Result` is `Err(NotFound{resource})`; the store records zero
  `txn` calls; the eval broker records zero enqueues (assert via a counting /
  fault-injecting `IntentStore` double + the broker).
- Same posture as `stop_workload` (`job_stop_unknown.rs`).
- **Expected RED**: `MISSING_FUNCTIONALITY` (handler does not exist).

### S-BIR-HANDLER-TXN — A restart commits one atomic bump+clear txn and retains the intent row (US-BIR-1 AC3/4)

```gherkin
@driving_adapter
Scenario: Restart bumps the generation and clears the stop sentinel atomically, intent retained
  Given a declared "workloads/payments" aggregate (and an optional "/stop" sentinel)
  When restart_workload is invoked for "payments"
  Then it commits exactly one IntentStore::txn carrying [IncrementU64{for_workload_generation(payments)}, Delete{for_workload_stop(payments)}]
  And "workloads/payments" remains present after the call (intent retained, distinct from #211)
  And a job-lifecycle evaluation is enqueued
  And it returns 200 with { workload_id: "payments", outcome }
```

- **Universe**: the captured `txn` op set is exactly `[IncrementU64{gen_key},
  Delete{stop_key}]` (one commit); `store.get(for_workload(payments))` is `Some`
  after; the broker recorded one enqueue; HTTP status 200.
- Asserts the atomic bump+clear shape (DDD-9) at the handler seam and the
  intent-retained invariant.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

> **Resumed/Restarted split (review-distill High / GWT one-action).** The label
> classification has two independent invocations (stopped → Resumed, running →
> Restarted); split into one behaviour per scenario. The label is **cosmetic** —
> placement is the reconciler's generation gate, not the label; each scenario
> pins the classification source (the check-exists `/stop` read, before the bump
> txn) and that the label does not drive behaviour.

### S-BIR-HANDLER-OUTCOME-RESUMED — A restart on a stopped workload reports outcome `resumed` (DDD-11)

```gherkin
@driving_adapter
Scenario: A restart on a stopped workload reports resumed
  Given a declared workload "payments" whose "/stop" sentinel IS present at the read
  When restart_workload is invoked for "payments"
  Then the response outcome is Resumed
```

- **Universe**: `RestartWorkloadResponse.outcome == Resumed` (classified from the
  `/stop` presence at the check-exists read, before the bump txn).
- **Mutation target**: a mutation inverting the present⇒Resumed classification.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

### S-BIR-HANDLER-OUTCOME-RESTARTED — A restart on a running workload reports outcome `restarted` (DDD-11)

```gherkin
@driving_adapter
Scenario: A restart on a running workload reports restarted
  Given a declared workload "coinflip" whose "/stop" sentinel is ABSENT at the read
  When restart_workload is invoked for "coinflip"
  Then the response outcome is Restarted
```

- **Universe**: `RestartWorkloadResponse.outcome == Restarted` (classified from
  the absent `/stop` at the check-exists read).
- **Mutation target**: a mutation inverting the absent⇒Restarted classification.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

---

## Integration — CLI driving adapter (in-process; NO subprocess)

> Home: `crates/overdrive-cli/tests/integration/workload_restart.rs` (NEW; gated
> `integration-tests`; **direct handler-call**, NOT a subprocess). Sibling
> precedent: `deploy.rs`, `endpoint_from_config.rs` (the canonical shape),
> `alloc_status.rs` — all **direct-call** integration tests
> (`overdrive_cli::commands::deploy::deploy(args).await`,
> `commands::alloc::status(args).await`), NOT subprocesses. Per
> `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess" (a
> **firm rule** — "we have rejected the `Command::spawn` pattern for this crate"):
> the test calls `commands::workload::restart(RestartArgs{ id, config_path })`
> against a fresh in-process `run_server` on an ephemeral port (the trust triple
> `overdrive serve` writes names the live endpoint), and asserts on the typed
> `Result<RestartOutput, CliError>`. **NO `Command::spawn`, NO
> `CARGO_BIN_EXE_overdrive`.** This still satisfies `nw-distill` § "Driving
> Adapter Verification" — the CLI *driving adapter* (clap parse → handler →
> `ApiClient::restart_workload` → `POST /v1/jobs/:id/restart`) is exercised
> through its real handler entry point, which is how every other CLI verb in this
> crate is driving-adapter-tested. The binary `main.rs` dispatch + exit-code
> mapping is the thin reviewed `mutants::skip` dispatcher (same posture as
> deploy/stop/alloc); the end-to-end production path for the verb is closed at the
> HTTP route by the Tier-3 oracle ATs (which drive `POST /v1/jobs/:id/restart`).

> **Success/unknown split (review-distill High / GWT one-action).** The CLI
> proof has a happy path and an error path; split into one behaviour per
> scenario. **Neither is tagged `@walking_skeleton`** (review-distill Medium):
> the feature's end-to-end walking skeleton is the *reused* dial-by-name Tier-3
> oracle (S-DBN-WS-STABLE), not this CLI adapter proof — these are the new CLI
> *driving-adapter* proofs (Mandate / RCA-P1), so they carry `@driving_adapter
> @real-io` only.

### S-BIR-CLI-RESTART-SUCCESS — `overdrive workload restart <id>` returns the new instance

```gherkin
@driving_adapter @real-io
Scenario: The restart verb dispatches and reports the outcome
  Given an in-process control plane (run_server on an ephemeral port) with a declared workload "payments"
  And a trust triple on disk naming that endpoint (written by overdrive serve)
  When the operator invokes the restart handler `commands::workload::restart(RestartArgs{ id: "payments", config_path })`
  Then it returns Ok(RestartOutput { workload_id: "payments", outcome })
  And the outcome ∈ { restarted, resumed }
```

- **Universe** (port-exposed): the typed `Result<RestartOutput, CliError>` — `Ok`
  carrying `RestartOutput { workload_id, outcome }` (success).
- **Mechanism**: a **direct handler-call** (NOT a subprocess), the
  `endpoint_from_config.rs` / `deploy.rs` shape — start `run_server` on an
  ephemeral port, write the trust triple, invoke
  `overdrive_cli::commands::workload::restart(args).await`, assert on the typed
  `RestartOutput`. **NO `Command::spawn`, NO `CARGO_BIN_EXE_overdrive`** (firm rule,
  `crates/overdrive-cli/CLAUDE.md`).
- Proves the CLI *driving adapter* (handler → `ApiClient::restart_workload` →
  `POST /v1/jobs/:id/restart`) parses args, resolves the endpoint from the trust
  triple, and POSTs the route. Pipeline-level handler-internal tests do NOT replace
  this (RCA `docs/analysis/rca-user-port-gap.md`). The binary `main.rs` dispatch is
  the thin reviewed `mutants::skip` dispatcher; the verb's end-to-end production
  path is closed at the HTTP route by the Tier-3 oracle ATs.
- **Mutation target**: a mutation in the CLI handler dispatch / `RestartOutput`
  construction.
- **Expected RED**: `MISSING_FUNCTIONALITY` (`WorkloadCommand::Restart`,
  `commands::workload::restart`, and `ApiClient::restart_workload` do not exist).

### S-BIR-CLI-RESTART-UNKNOWN — `overdrive workload restart <unknown>` errors with not-found

```gherkin
@driving_adapter @real-io @error_path
Scenario: The restart verb maps an unknown id to an honest not-found error
  Given an in-process control plane (run_server on an ephemeral port) with no declared workload "nonexistent"
  And a trust triple on disk naming that endpoint
  When the operator invokes `commands::workload::restart(RestartArgs{ id: "nonexistent", config_path })`
  Then it returns Err(CliError::HttpStatus { status: 404, error: "not_found", .. })
  And render::cli_error_to_exit_code maps that CliError to a non-zero exit code
```

- **Universe** (port-exposed): the typed `Result<RestartOutput, CliError>` — `Err`
  carrying the not-found `CliError` (`HttpStatus { status: 404, error: "not_found" }`);
  the non-zero exit code `render::cli_error_to_exit_code(&err)` returns.
- **Mechanism**: a **direct handler-call** (NOT a subprocess) — invoke
  `commands::workload::restart(args).await`, assert on the typed `CliError` variant,
  and assert `render::cli_error_to_exit_code(&err)` is non-zero. **NO
  `Command::spawn`** (firm rule, `crates/overdrive-cli/CLAUDE.md`). The exit-code
  mapping is asserted at the `render::cli_error_to_exit_code` seam the binary
  `main.rs` uses, not via a real process exit.
- Proves the CLI maps the handler 404 to an honest typed error → a non-zero exit,
  not a silent success.
- **Mutation target**: a mutation that swallows the 404 / maps it to a zero exit code.
- **Expected RED**: `MISSING_FUNCTIONALITY`.

---

## Tier 3 — the oracle (ALREADY AUTHORED, `#[ignore]`'d; slice-04 un-ignores)

> These three ATs are the feature's **terminal quality gate** and DoD. They are
> NOT authored by this DISTILL wave — they already exist, deferred to #249.
> slice-04 (DELIVER terminal slice) **removes the `#[ignore = "…#249…"]` strings**
> (removed, not rewritten — no stale forward-pointer) and **swaps the
> `stop_and_converge` + same-spec-redeploy cycle/recovery for the production
> `overdrive workload restart <id>` action**; the assertions are unchanged. They
> are the SSOT proof for US-BIR-1 + US-BIR-2 on the production path. **No
> test-only intent-key clear / hand-installed replacement** (CLAUDE.md
> vertical-slice rule).

### S-DBN-WS-STABLE — The name re-resolves the same `F` across the cycle and the next connect lands the new backend

- **File**: `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs:1685`
  (`answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend`).
- **Proves**: US-BIR-1 (`alloc_b1 ≠ alloc_b2` — new AllocationId) + US-BIR-2
  (`f1_again == f1` byte-stable, post-cycle dial lands `B2`; the inter-agent
  leg-B↔leg-C hop carries TLS 1.3 `application_data` records, the `lo:SERVICE_PORT`
  `0x17` oracle). KPIs K-BIR-1, K-BIR-2.

```gherkin
@real-io @frontend @churn @kpi @oracle
Scenario: byte-stable-across-cycle oracle passes un-ignored
  Given "server" is Running behind stable frontend F1 with backend B1, and a connect to F1 lands B1 byte-exact
  When the operator runs `overdrive workload restart server` and a new instance B2 reaches Running
  Then getaddrinfo("server.svc.overdrive.local") re-resolves the same F1 byte-for-byte
  And the next connect to F1 lands the new backend B2 with a byte-exact round-trip
  And F1 was always a stable frontend ∈ 10.98.0.0/16, never a per-instance backend addr ∈ 10.99.0.0/16
  And the inter-agent leg-B↔leg-C hop carries TLS 1.3 application_data records (0x17), zero cleartext
```

- **Expected RED in DELIVER**: the AT is `#[ignore]`'d today; with the verb
  landed it must GREEN on the pinned-6.18 Tier-3 matrix. (It is NOT a
  `MISSING_FUNCTIONALITY` scaffold — it is an existing AT un-blocked.)

### S-DBN-CHURN — An in-flight connection fails fast on backend churn; the next dial lands the new backend

- **File**: `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs:1855`
  (`in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend`).
- **Proves**: US-BIR-2 churn boundary. KPI K-BIR-3.

```gherkin
@real-io @churn @error_path @kpi @oracle
Scenario: backend-churn fail-fast oracle passes un-ignored
  Given a client holds an open in-flight connection through F1 to backend B1
  When the operator runs `overdrive workload restart server` mid-connection
  Then the in-flight connection fails fast (reset/error/EOF) within CHURN_BOUND (TCP_USER_TIMEOUT), never an indefinite hang
  And a subsequent fresh connect to F1 lands the new live backend B2 (byte-exact)
  And no sock_destroy is used (#61 scope) — the terminating-proxy fail-fast posture only
```

- **Expected RED in DELIVER**: un-ignore → GREEN on the Tier-3 matrix.

### S-DBN-NXDOMAIN-02-RECOVERY — A recovered workload re-resolves the same stable `F` (withhold-not-release)

- **File**: `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs:1068`
  (`recovered_job_after_stop_resolves_to_the_same_stable_frontend`).
- **Proves**: US-BIR-1 + US-BIR-2 — the stopped `<job>` resolves NXDOMAIN while
  stopped, then re-resolves the SAME `F` once recovered via the replace action
  (the withhold-not-release Tier-3 `getent` recovery observable). KPI K-BIR-2.
  (The allocator F-retention invariant itself is already Tier-1 mutation-gated at
  01-04 / S-DBN-FRONTEND-03 / S-DBN-IDX-02; only this Tier-3 `getent` recovery
  observable is #249-blocked.)

```gherkin
@real-io @error_path @frontend @kpi @oracle
Scenario: withhold-not-release recovery oracle passes un-ignored
  Given "server" was stopped via POST /v1/jobs/server/stop (its name resolves NXDOMAIN while stopped)
  When the operator recovers the SAME "server" to Running-AND-HEALTHY via `overdrive workload restart server`
  Then getent re-resolves the SAME stable F1 byte-for-byte (the allocator withheld, did not release, F across the stop)
```

- **Expected RED in DELIVER**: un-ignore → GREEN on the Tier-3 matrix.

---

## Adapter coverage table (Mandate 6)

Every driven adapter the feature adds or exercises → at least one `@real-io`
scenario. **No empty rows.**

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `IntentStore::txn` + NEW `TxnOp::IncrementU64` (`LocalIntentStore`, real redb) | YES | **`@real-io`**: S-BIR-TXN-01..04 (real `LocalIntentStore` over redb). In-process focused: S-BIR-HANDLER-TXN (the op-set assertion via a counting double). |
| `IntentStore::get` / `delete` (check-exists 404 + label read) | YES | **`@real-io`**: S-BIR-CLI-RESTART-SUCCESS/-UNKNOWN (get/delete runs through the production route → real `LocalIntentStore` in an in-process `run_server`, via a **direct CLI handler-call** — no subprocess). In-process focused: S-BIR-HANDLER-404 / -OUTCOME-RESUMED / -OUTCOME-RESTARTED (`@driving_adapter`, counting/fault double — NOT `@real-io`). |
| `restart_workload` HTTP handler + `POST /v1/jobs/:id/restart` route | YES | **`@real-io`**: S-BIR-CLI-RESTART-SUCCESS/-UNKNOWN (direct CLI handler-call → `ApiClient::restart_workload` → the production route on an in-process `run_server`). In-process focused: S-BIR-HANDLER-* (direct handler call). End-to-end on the real-kernel path: the Tier-3 oracle ATs drive the route. |
| `overdrive workload restart` CLI verb + `ApiClient::restart_workload` | YES | S-BIR-CLI-RESTART-SUCCESS/-UNKNOWN (direct handler-call against in-process `run_server` — no subprocess) |
| `WorkloadLifecycle` reconciler (generation gate, scoped veto, placement stamp) | YES (Tier-1 pure + Tier-3 oracle) | S-BIR-RESTART-*, S-BIR-COALESCE-*/SEQUENTIAL, S-BIR-REGRESSION-*, S-BIR-BUG3, S-DBN-WS-STABLE |
| `FrontendAddrAllocator` idempotent `assign` (reused; must-not-regress guardrail) | YES (reused) | S-DBN-WS-STABLE, S-DBN-NXDOMAIN-02-RECOVERY |
| re-keyed `MtlsResolve` (per-connect live-backend translation; reused) | YES (reused) | S-DBN-WS-STABLE, S-DBN-CHURN |
| intercept worker `TCP_USER_TIMEOUT`/keepalive legs (in-flight churn; reused) | YES (reused) | S-DBN-CHURN |
| `getaddrinfo`/`getent` (name-path signal; reused) | YES (reused) | S-DBN-WS-STABLE, S-DBN-NXDOMAIN-02-RECOVERY |
| `ObservationStore` (read-only here — reconciler reads `actual.allocations`) | n/a (read-only; no new write surface) | (asserted via the reconciler `actual` inputs) |

The `current_alloc` pure helper and the BE-u64 codec are NOT adapters (no port
trait) — they are the Tier-1 proptest/unit seams (S-BIR-CURRENT-ALLOC,
S-BIR-TXN-04).

> **`@real-io` accounting (rev2, review-distill 2026-06-30 Finding-3; rev4
> mechanism correction 2026-06-30).** The `@real-io` proof for each `IntentStore`
> path is named explicitly above: the `txn`/`IncrementU64` path is real-redb at
> Tier-1 store-acceptance (S-BIR-TXN-*); the `get`/`delete`/route path is
> real-`LocalIntentStore` via the production route, exercised by the CLI
> **direct-handler-call** integration tests (S-BIR-CLI-RESTART-*) against an
> in-process `run_server` — NOT a subprocess (`crates/overdrive-cli/CLAUDE.md` §
> "Integration tests — no subprocess"). The `@driving_adapter` handler scenarios
> (S-BIR-HANDLER-*) are **focused in-process coverage** over a counting/fault
> `IntentStore` double, NOT `@real-io` — they are not counted as the Mandate-6
> real-I/O proof; the CLI direct-call + store-acceptance rows are. The verb's
> real-kernel end-to-end path is the Tier-3 oracle (driving the route).

---

## Driving-adapter verification (Mandate / RCA-P1)

DESIGN entry points → at least one scenario via the real protocol:

| Driving entry point | Real-protocol scenario |
|---|---|
| `overdrive workload restart <id>` (CLI handler — direct call, in-process `run_server`) | S-BIR-CLI-RESTART-SUCCESS (typed `RestartOutput`); S-BIR-CLI-RESTART-UNKNOWN (`CliError` not-found → non-zero exit) |
| `POST /v1/jobs/:id/restart` (HTTP) | S-BIR-HANDLER-404 / S-BIR-HANDLER-TXN / S-BIR-HANDLER-OUTCOME-RESUMED/RESTARTED; S-BIR-CLI-RESTART-* end-to-end |
| `overdrive serve` (`run_server`) | S-DBN-WS-STABLE / S-DBN-CHURN / S-DBN-NXDOMAIN-02-RECOVERY (the oracle drives `run_server_with_obs_and_driver`) |
| `overdrive deploy` (`POST /v1/jobs`) | reused unchanged — the oracle ATs deploy through it |
| `overdrive alloc status` (the new-AllocationId observable) | S-DBN-WS-STABLE (`alloc_b1 ≠ alloc_b2` via the alloc-status surface) |
| `getaddrinfo`/`getent` (name path) | S-DBN-WS-STABLE / S-DBN-NXDOMAIN-02-RECOVERY |

No uncovered DESIGN entry point. The handler-internal in-process scenarios
(S-BIR-HANDLER-*, counting/fault double) do NOT substitute for the CLI
driving-adapter scenarios (S-BIR-CLI-RESTART-SUCCESS/-UNKNOWN), which exercise the
real CLI handler → `ApiClient` → production route path via a direct handler-call
against an in-process `run_server` (no subprocess, per
`crates/overdrive-cli/CLAUDE.md`).

---

## Pre-DELIVER fail-for-the-right-reason gate

Per ADR-025 D2, the fail-for-the-right-reason gate becomes DELIVER's RED-phase
entry/exit gate. **DISTILL does NOT run the classification** — the production
surface (the `TxnOp::IncrementU64` variant, the handler, the CLI verb, the
generation fields, `current_alloc`) does not exist yet, so a compile-able RED
scaffold cannot be authored without writing into `crates/` (the Scaffold MANIFEST
SCOPE DECISION). DELIVER's RED phase materialises the scaffolds and runs the
gate; the expected RED reason per scenario is in `red-classification.md`.

For the **three oracle ATs**: they are already authored and `#[ignore]`'d, so
their "RED" is the `#[ignore]` skip, not a `MISSING_FUNCTIONALITY` panic. slice-04
removes the ignore and swaps the cycle mechanism; the gate for those is "GREEN on
the pinned-6.18 Tier-3 matrix driving the production verb."
