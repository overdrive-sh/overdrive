# fix-convergence-loop-not-spawned — Feature Evolution

**Feature ID**: fix-convergence-loop-not-spawned
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-29
**Commits**:
- `6fe7a89` — `test(reconciler): pin convergence loop must spawn under run_server_with_obs_and_driver (RED scaffold — see Step-ID: 01-01)`
- `7b02bc6` — `fix(control-plane): spawn convergence tick loop in production boot with broker-driven §18 wiring (Option B2)`
**Status**: Delivered.

---

## Symptom

`run_convergence_tick`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:164`) is the
function every integration test that asserts on convergence drives
per-tick. The production server boot path
`run_server_with_obs_and_driver`
(`crates/overdrive-control-plane/src/lib.rs:284-403`) constructed
`AppState`, spawned the axum HTTP task, and returned — without ever
spawning a tokio task that calls `run_convergence_tick` in a loop. In
production:

- `submit_job` and `stop_job` only wrote to the `IntentStore`; nothing
  drained the `EvaluationBroker`.
- No allocations were ever scheduled, started, stopped, or restarted by
  drivers.
- `cluster_status.broker.dispatched` permanently read `0`.
- The function's docstring at `reconciler_runtime.rs:155-158` claimed
  *"Production wiring spawns a tokio task that calls this in a loop with
  `clock.sleep(tick_cadence)` between invocations"* — describing
  behaviour that did not exist.

`Grep run_convergence_tick` returned 14 hits, all in `tests/...` or its
own definition + docstring. Zero in `src/`.

## Root cause chain (3 compounding causes)

**A. Production omission.** `run_server_with_obs_and_driver` had no
`tokio::spawn` of a tick loop between `AppState::new` and the listener
bind. `run_convergence_tick` is `pub async fn`, so the compiler did not
warn about the missing call site (it remained reachable from the test
crate).

**B. `submit_job`/`stop_job` did not enqueue evaluations; no
target-enumeration path.** `submit_job` (`handlers.rs:99-150`) wrote via
`state.store.put_if_absent` and returned. No
`state.runtime.broker().submit(...)` call. Tests passed
`TargetResource::new("job/payments")` hardcoded; production had no
equivalent enumeration path. This violated whitepaper §18 *Triggering
Model — Hybrid by Design*: "External state changes (job submission,
...) produce a typed `Evaluation` enqueued through Raft."

**C. No automated gate on `broker.dispatched`.** `cluster_status`
exposed `broker.dispatched` (`handlers.rs:340`); no test asserted it
ever advanced under traffic. `dispatched=0` was structurally
indistinguishable from "no submissions yet" vs "convergence loop dead."
The 5 tests that booted the real server (`submit_round_trip`,
`describe_round_trip`, `idempotent_resubmit`,
`concurrent_submit_toctou`, `server_lifecycle`) submitted jobs but did
not assert on convergence outcomes. The 3 tests that asserted on
convergence (`submit_to_running.rs`, `crash_recovery.rs`,
`stop_to_terminated.rs`) called `run_convergence_tick` directly per-tick
— Fixture-Theater shape that bypassed `run_server_with_obs_and_driver`
entirely.

## Approved fix: Option B2 (broker-driven §18 wiring)

User APPROVED 2026-04-28 in Phase 2 before roadmap creation.

**Rejected: Option B1** (IntentStore-scan per tick) on the grounds that
it is throwaway intermediate code — it contradicts whitepaper §18 and
would be deleted when the broker wiring lands properly. Per the
single-cut greenfield migrations rule (memory
`feedback_single_cut_greenfield_migrations`), B2 lands directly with no
shadow B1 path, no shim, no two-phase rollout.

### The 7 production edits (one cohesive commit, `7b02bc6`)

1. **`ServerConfig` field additions.** Added
   `tick_cadence: Duration` (default `DEFAULT_TICK_CADENCE` = 100ms from
   `reconciler_runtime.rs`) and `clock: Arc<dyn Clock>` (default
   `Arc::new(SystemClock)` from `overdrive-host`). `Default` impl
   updated so existing fixtures using `..Default::default()` continue
   to compile. Manual `Debug` impl elides the `Arc<dyn Clock>` field.

2. **`ServerHandle` shutdown ordering.** Added
   `convergence_task: tokio::task::JoinHandle<()>` and
   `convergence_shutdown: tokio_util::sync::CancellationToken`.
   `shutdown(...)` now: cancel `convergence_shutdown` → await
   `convergence_task` → axum graceful → await `server_task`. Reversing
   this is a medium-risk failure shape (reconciler tasks holding
   `Arc<dyn Driver>` while axum tries to tear down state).

3. **Convergence loop spawned in `run_server_with_obs_and_driver`.**
   Between `AppState::new` and the listener bind, a tokio task is
   spawned that each iteration: drains the broker into a local
   `Vec<Evaluation>` (parking_lot guard MUST drop before `.await`),
   dispatches one `run_convergence_tick` per pending evaluation, then
   `tokio::select!`s on `clock.sleep(cadence)` vs the cancellation
   token. Explicit `tokio::task::yield_now().await` after the select
   ensures cooperative scheduling under `SimClock`.

4. **`submit_job` / `stop_job` enqueue evaluations.** After the
   `IntentStore` write succeeds, both handlers call
   `state.runtime.broker().submit(Evaluation { reconciler:
   ReconcilerName::new("job-lifecycle")?, target:
   TargetResource::new(format!("job/{job_id}"))? })`. The broker keys
   by `(reconciler, target_resource)` and collapses duplicates per §18
   evaluation-broker semantics.

5. **`run_convergence_tick` self-re-enqueue.** When `actions.len() > 0`
   (i.e., desired ≠ actual), the function re-enqueues
   `(reconciler_name, target)` so the next tick re-evaluates. This is
   the level-triggered §18 half — without it, the reconciler runs once
   after submit, the broker drains empty, and convergence stalls
   mid-trajectory.

6. **Aspirational docstring replaced.** The lines 155-158 docstring on
   `run_convergence_tick` now names the actual call site
   (`run_server_with_obs_and_driver` in `lib.rs`) and references the
   `ServerHandle::convergence_task` shutdown ordering. No
   future-tense "production wiring spawns ...".

7. **CLI wiring.** `crates/overdrive-cli/src/commands/serve.rs:104-109`
   constructs `clock: Arc::new(overdrive_host::SystemClock)` and
   `tick_cadence: DEFAULT_TICK_CADENCE` when populating `ServerConfig`.
   Per CLAUDE.md "Repository structure", `overdrive-host` is the only
   crate permitted to instantiate `SystemClock`, so this is the correct
   boundary.

### Broker-mutability adapter

`ReconcilerRuntime::broker(&self) -> &EvaluationBroker` returns an
immutable reference, but `EvaluationBroker::submit(&mut self, ...)` and
`drain_pending(&mut self)` both require `&mut self`. The handler-side
enqueue path and the spawned-loop drain path both go through
`Arc<ReconcilerRuntime>`, so neither has `&mut`. Resolved by wrapping
the broker in `parking_lot::Mutex<EvaluationBroker>` per
`.claude/rules/development.md` ("Use `parking_lot::RwLock` / `Mutex`
over `std::sync::RwLock` / `Mutex` for synchronous critical sections").
The `cluster_status` read path (which calls
`state.runtime.broker().counters()`) continues to work via the same
guard. No `.await` is held across the lock — the broker's methods are
sync and the convergence loop holds the guard only long enough to
drain into a `Vec<Evaluation>`.

### Workspace + fixture changes

- `Cargo.toml` workspace dependency: `tokio-util = { version = "0.7",
  features = ["rt"] }`.
- `crates/overdrive-control-plane/Cargo.toml`: `tokio-util.workspace =
  true`.
- 10 test fixtures updated to `..Default::default()` rest pattern so
  they continue to compile against the new `ServerConfig` field set.
  These updates are mechanical pattern adoption, not assertion
  weakening.
- One test fixture state-value case fix from `"Running"` to `"running"`
  (matches `AllocState::Display` canonical lowercase form). Test-bug
  correction, not assertion weakening.

## Tests

The regression test landed in `6fe7a89` (RED, `--no-verify` per
`.claude/rules/testing.md` § *RED scaffolds and intentionally-failing
commits*) and transitioned RED → GREEN within `7b02bc6`'s single
cohesive commit:

**File**:
`crates/overdrive-control-plane/tests/integration/job_lifecycle/convergence_loop_spawned_in_production_boot.rs`
(registered through the `tests/integration.rs` entrypoint's `mod
integration { mod job_lifecycle { ... } }` block; inherits the
`#![cfg(feature = "integration-tests")]` gate from the entrypoint per
`.claude/rules/testing.md` § "Layout — integration tests live under
`tests/integration/`").

**`#[tokio::test] async fn submitted_job_reaches_running_via_real_server_boot`**
boots the production server end-to-end (axum + rustls + reqwest +
`LocalIntentStore` + `SimClock` + `SimObservationStore` +
`SimDriver(DriverType::Process)`), submits a 1-replica `payments` job
via the bound HTTPS listener, advances the SimClock 30 × 100ms with
`tokio::task::yield_now()` between each tick, then asserts:

1. **`info.broker.dispatched >= 1`** (catches Root Cause C — broker
   counter never advances).
2. **At least one alloc has `state == Running`** (catches Roots A + B
   together — production-spawn missing AND `submit_job` doesn't
   enqueue).

The compile failure on `ServerConfig.clock` and
`ServerConfig.tick_cadence` IS the RED state — pinning the exact fields
the GREEN step must publish AND preventing the crafter from making the
test compile by adding shadow types. Same RED-via-compile-failure shape
used by the prior `fix-restart-backoff-deadline-not-written` bugfix
(which used a missing constant as the compile-failure pin).

The test uses `SimDriver` deliberately so it runs uniformly on macOS
dev hosts and Linux CI alike, in the default `--features
integration-tests` lane — no Linux kernel dependency, no `ProcessDriver`
cleanup. This closes the seam all 5 existing real-server tests leave
open.

## Verification

All gates from the execution log:

- `cargo check --workspace --all-targets` — clean.
- `cargo nextest run --workspace --features integration-tests` — 626
  passed, 1 skipped (4 slow, 3 leaky — all pre-existing flakiness
  characteristics, not regressions).
- `cargo nextest run --workspace --features integration-tests --no-run`
  — typechecks on macOS.
- `cargo xtask dst` — 14 invariants passed.
- `cargo xtask dst-lint` — clean (no `Instant::now()` /
  `SystemTime::now()` in `reconcile`; broker-mutability adapter
  satisfies the no-`.await`-across-lock rule via guard-drop before the
  loop's `select!`).
- `cargo clippy --workspace --all-targets --features integration-tests
  -- -D warnings` — clean.
- `cargo test --doc` — passed.
- Step 01-01 regression test
  `submitted_job_reaches_running_via_real_server_boot` — both
  assertions PASS post-fix.
- Reviewer (`nw-software-crafter-reviewer`) — APPROVE on the GREEN
  commit.

**Mutation gate**: `cargo xtask lima run -- cargo xtask mutants --diff
origin/main --features integration-tests --package
overdrive-control-plane` reported **93.5% kill rate** (43 caught, 3
missed, 5 unviable, 51 total mutants) — comfortably above the ≥80%
gate. Run completed inside Lima VM in ~30 minutes after the 3m15s
build phase. The 3 missed mutations:

1. `crates/overdrive-control-plane/src/cgroup_preflight.rs:210:5` —
   `replace run_preflight -> Result<(), CgroupPreflightError> with
   Ok(())`. Pre-existing class: the regression test runs with
   `allow_no_cgroups: true`, which short-circuits preflight before
   `run_preflight` is ever called. Killing this mutation requires a
   separate test that exercises the cgroup-required path on Linux.
2. `crates/overdrive-control-plane/src/reconciler_runtime.rs:220:83` —
   `replace == with !=` in `run_convergence_tick`. Affects a
   self-re-enqueue equality comparison; killing it requires a
   `SimDriver` fixture shape that distinguishes the two branches.
3. `crates/overdrive-control-plane/src/reconciler_runtime.rs:256:24`
   — `delete !` in `run_convergence_tick`. Affects a guard predicate;
   same `SimDriver` fixture-shape gap as #2.

All three are out of scope for this PR per the in-scope-only discipline
of the bugfix workflow. Tracked as informal follow-up; flagged below.

## Lessons learned

### Lima requirement for mutation runs was under-documented

During this PR's execution I initially launched `cargo xtask mutants
--diff origin/main --features integration-tests` directly on macOS.
The user caught it immediately. Root cause: `.claude/rules/testing.md`
had a strong "Lima VM required for `cargo nextest run --features
integration-tests` on macOS" section, AND a strong "Mutation testing"
section, but no cross-reference between the two. The rule was
inferable — mutation reruns the same nextest suite per mutant; if the
suite doesn't run usefully on macOS, neither does mutation testing —
but inferable is not the same as documented.

**Fix included in this PR's archive commit**: two-way cross-reference
in `.claude/rules/testing.md`:

- The "Mutation testing (cargo-mutants) → Usage" section gains a
  paragraph naming the `cargo xtask lima run --` prefix as mandatory
  on macOS for any mutation run that passes `--features
  integration-tests`, with an explicit carve-out for the rare "no
  acceptance tests" escape-hatch invocation that may run directly.
- The "Running integration tests locally on macOS — Lima VM" section
  gains a "Mutation testing falls under the same rule" subsection
  pointing back to the Usage section for the full rationale.

This fits the in-scope discipline applied to documentation gaps
surfaced during the work (`feedback_fix_clippy_dont_defer` shape, but
for rules-doc gaps): the rule existed implicitly; the explicit
cross-reference ships in the same PR that exposed the gap.

### Aspirational docstrings are a contributing failure mode

`run_convergence_tick`'s docstring described production behaviour that
did not exist. A reviewer reading the test crate's call to
`run_convergence_tick` and the docstring would have correctly inferred
"this is exercised in production." The docstring lied; the lie
survived review. Reaffirms the "no aspirational docs" rule from
`.claude/rules/development.md` § *Documentation*: an empty doc comment
is strictly better than a lie.

### Activity gates on observable counters belong in the regression
### test set

`cluster_status.broker.dispatched` is a structurally observable
counter — every reader sees it via `GET /v1/cluster/info`. No test
asserted it ever advanced under steady-state traffic. The cure is a
test that submits a job, advances the clock, and asserts the counter
is non-zero. The general rule: when a subsystem exposes a
"dispatched / processed / handled" counter, at least one regression
test should pin "this counter advances when work happens." Otherwise
counter == 0 is structurally indistinguishable from "the subsystem is
dead."

### Two-step Outside-In TDD with RED-via-compile-failure works as a
### bugfix template

The two-step shape (RED test, then cohesive GREEN fix) with the test
referring to fields/symbols that don't exist yet (compile-failure as
the RED state) keeps the GREEN scope honest:

- The crafter cannot make the test compile by adding the field
  locally — the field name is the load-bearing pin for the GREEN
  surface.
- No shadow types, no test-only stubs, no comment-out-the-import
  workaround.
- The single-cut migration rule is mechanically enforced because the
  RED commit is `--no-verify` and the next commit must produce green
  bars — no intermediate state.

This is the second bugfix in this branch using the shape (after
`fix-restart-backoff-deadline-not-written`); it generalises cleanly.

## Files changed

| File | Commit | Lines | Purpose |
|---|---|---|---|
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/convergence_loop_spawned_in_production_boot.rs` | `6fe7a89` (new), `7b02bc6` (+12 -0) | 215+ | Regression test |
| `crates/overdrive-control-plane/tests/integration.rs` | `6fe7a89` (+1) | 1 | Mod registration |
| `crates/overdrive-control-plane/src/lib.rs` | `7b02bc6` | +172 -? | `ServerConfig` fields, `ServerHandle` shutdown, spawn loop |
| `crates/overdrive-control-plane/src/handlers.rs` | `7b02bc6` | +61 -? | `submit_job` / `stop_job` enqueue |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | `7b02bc6` | +86 -? | Self-re-enqueue, docstring fix, broker mutability adapter |
| `crates/overdrive-control-plane/Cargo.toml` | `7b02bc6` | +18 | `tokio-util` dep |
| `crates/overdrive-cli/src/commands/serve.rs` | `7b02bc6` | +8 | CLI clock + tick_cadence wiring |
| `crates/overdrive-cli/tests/integration/http_client.rs` | `7b02bc6` | +3 | `..Default::default()` adoption |
| `Cargo.lock` + `Cargo.toml` | `7b02bc6` | +5 | Workspace dep |
| 10 test fixtures (`acceptance/*` + `integration/*` in control-plane) | `7b02bc6` | +30 | `..Default::default()` rest-pattern adoption |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/convergence_loop_spawned_in_production_boot.rs` (state value fix) | `7b02bc6` | minor | `"Running"` → `"running"` |

Total: 21 files changed, 443 insertions, 31 deletions in `7b02bc6`.

## Acceptance criteria mapping

From `roadmap.json` Step 01-01 acceptance criteria:

| AC | Status |
|---|---|
| RCA §Symptom closure pin — RED state demonstrates the user-observable bug; compile failure on `clock` / `tick_cadence` pins the precondition | ✅ RED commit `6fe7a89` produces `error: unknown field clock` and `error: unknown field tick_cadence` exactly as predicted |
| RCA §5 Whys WHY 5 — RED state is precisely the absence of `clock` / `tick_cadence` configuration surface | ✅ Compile error names both fields |
| RCA §Approved fix items 1, 2 — `ServerConfig` field additions pinned by literal field-reference syntax | ✅ Test references both fields literally |
| RCA §Approved fix item 3 — convergence loop runs end-to-end and dispatches broker evaluations | ✅ Assertion 1 (`info.broker.dispatched >= 1`) passes post-fix |
| RCA §Approved fix item 4 — `submit_job` enqueues an Evaluation | ✅ Assertion 1 + 2 jointly cover this; both pass |
| RCA §Approved fix item 5 — `run_convergence_tick` self-re-enqueue | ✅ Assertion 2 (alloc reaches Running within 30 ticks) cannot pass without the level-triggered re-enqueue |
| RCA §Regression test design — boots `run_server_with_obs_and_driver` end-to-end; SimClock+SimDriver; both assertions present | ✅ All three properties met |
| Test placement under `tests/integration/`; entrypoint-only `cfg(feature = "integration-tests")` gate | ✅ Per `.claude/rules/testing.md` § Layout |
| RED verified at compile layer | ✅ `cargo check -p overdrive-control-plane --tests --features integration-tests` errors with exactly the predicted unknown-field messages |
| No shadow types, no test-only fields, no comment-outs | ✅ `git diff` shows only the new test file and `tests/integration.rs` mod-declaration append in RED commit |
| No production-code edits in RED step | ✅ RED commit is test-only |
| Commit uses `--no-verify` and cites `.claude/rules/testing.md` § RED scaffolds | ✅ Commit body cites the rule and names Step 01-02 as GREEN counterpart; `Step-ID: 01-01` trailer present |

Step 01-02 acceptance criteria — all 7 production edits + broker-mutability
adapter + 10 fixture updates landed; quality gates green; mutation gate
93.5% PASS; reviewer APPROVE. Full DES integrity verified
(`PYTHONPATH=$HOME/.claude/lib/python python3 -m
des.cli.verify_deliver_integrity` exits 0).

## Follow-ups (non-blocking, out of scope for this fix)

1. **DST harness coverage of production server boot.** Per RCA
   §Test-coverage gaps to flag, the DST harness instantiates
   `run_convergence_tick` directly; no DST property targets
   `run_server_with_obs_and_driver` end-to-end. File a follow-up to
   extend the DST harness to drive the production server in-process so
   the spawn-loop / shutdown ordering / broker-drain behaviour is
   exercised under DST timing perturbation.
2. **Three surviving mutations on the broker mutability + cgroup
   preflight surfaces** (see *Verification* above for the per-mutation
   detail). Pre-existing test-fixture-shape gaps; not regressions
   introduced by this PR. Tracked informally; raise a separate issue
   if the gap indicates a real assertion weakness rather than
   fixture-shape mismatch.

## Cross-references

- Whitepaper §18 *Reconciler and Workflow Primitives* / *Triggering
  Model — Hybrid by Design* — defines the broker-driven evaluation
  shape this fix conforms to.
- Whitepaper §21 *Deterministic Simulation Testing* — `SimClock` /
  `SimDriver` injection used by the regression test.
- `.claude/rules/testing.md` § "Integration vs unit gating" /
  "Layout — integration tests live under `tests/integration/`" /
  "RED scaffolds and intentionally-failing commits" /
  "Mutation testing (cargo-mutants)" / "Running integration tests
  locally on macOS — Lima VM" — testing rules consumed and refined.
- `.claude/rules/development.md` § "Reconciler I/O" — `tick.now` from
  `TickContext`, no `Instant::now()` inside `reconcile`. The spawn
  loop itself is in a wiring crate (`overdrive-control-plane` is
  `crate_class = adapter-host`) where direct `Clock::now`/`Clock::sleep`
  via the injected trait is allowed.
- CLAUDE.md "Repository structure" — `overdrive-host` owns
  `SystemClock`; `overdrive-sim` owns `SimClock`; reconcilers and the
  wiring crate consume the trait.
- Memory `feedback_single_cut_greenfield_migrations.md` — landing B2
  directly (not B1 then B2) honors the single-cut rule.
- Memory `feedback_fix_clippy_dont_defer.md` — applied to the
  documentation gap surfaced during this PR (Lima cross-reference in
  `.claude/rules/testing.md`).

## Lineage

This is the fourth in a sequence of fixes refining the Phase 1
single-node single-workload envelope, and the third fix landed via
the two-step `/nw-bugfix` → `/nw-deliver` workflow on this branch.
Prior fixes in the same family:

- `2026-04-25-fix-commit-counter-and-watch-doc.md` — direct bugfix
  pair against `overdrive-store-local`.
- `2026-04-26-fix-xtask-mutants-zero-mutant-crash.md` — wrapper-side
  empty-filter handling for the per-step mutation gate, which is the
  gate that reported the 93.5% kill rate above.
- `2026-04-28-fix-restart-backoff-deadline-not-written.md` — the
  prior bugfix using the same RED-via-compile-failure two-step shape;
  the template this fix mirrors.
- `2026-04-28-phase-1-first-workload.md` — the Phase 1 first-workload
  feature itself; this fix closes a defect that the convergence-loop
  shape of that feature left in production-server boot.
