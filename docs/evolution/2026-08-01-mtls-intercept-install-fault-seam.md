# mtls-intercept-install-fault-seam — Feature Evolution

**Feature ID**: `mtls-intercept-install-fault-seam`
**Branch**: `marcus-sa/mtls-fault-injection-test-infra`
**Duration**: 2026-08-01 — research → DESIGN → DISTILL → DELIVER → finalize, one day,
11 commits.
**Status**: Delivered — 6/6 DELIVER roadmap steps complete (`01-01`, `02-01`,
`03-01`, `04-01`, `04-02`, `05-01`) across 5 phases; 13/13 DISTILL scenarios
covered, zero orphans. `des-verify-integrity` reports *"All 6 steps have complete
DES traces"*. Workspace suite **2467 passed / 22 skipped**; clippy and fmt clean.
Final unscoped per-PR mutation gate — the invocation CI runs —
`total=4 caught=4 missed=0 kill_rate=100.0% status=pass`, and the
`fail_closed_on_mtls_install -> Ok(())` mutant that motivated the issue is among
the four **caught**.
**ADRs**: [ADR-0076](../product/architecture/adr-0076-mtls-intercept-port-fault-injectable-privileged-install-surface.md)
(rev 5 — the feature's design record: the `MtlsIntercept` port, and § Decision 7
the one production fix). Extends ADR-0071; does **not** extend its
`wire → probe → use` invariant — this port deliberately carries no `probe()`
(§ Decision 4).
**Closes**: GH [#250](https://github.com/overdrive-sh/overdrive/issues/250)
(fault-injection seam for the `fail_closed_on_mtls_install` MISSED mutant). The
`.cargo/mutants.toml` `exclude_re` suppression is deleted and the mutant is
caught.

---

## What shipped

A driven port over the **privileged intercept-install surface** —
`MtlsIntercept` (`crates/overdrive-worker/src/mtls_intercept_port.rs`), three sync
methods (`bind_transparent` / `install_outbound` / `install_inbound`) wrapping the
raw `libc::socket` + `setsockopt(IP_TRANSPARENT)` bind and the two `nft`/`ip`
shell-outs `MtlsInterceptWorker::start_alloc` previously called as concrete free
functions. It ships with a host adapter (`HostMtlsIntercept`, three one-line
delegations), a fault-scripting simulation adapter (`SimMtlsIntercept` in
`overdrive-sim`), and an `InterceptGuard` marker trait so the port can hand back
RAII guards a test double can satisfy without a real `nft` `Drop`.

The point of it is narrow and worth stating up front: it makes the fail-closed
handler's **call-site ordering** exercisable. On an intercept-install failure the
`StartAllocation` / `RestartAllocation` arms must `return` **before**
`driver.release_for_exit_emission(handle)`, so a now-`Failed` allocation never
releases its exit watcher. Nothing in the tree could force `start_alloc` to fail
on demand — `mtls_worker` was a concrete `Option<&Arc<MtlsInterceptWorker>>`, and
under root the real install *succeeds* — so that security-relevant ordering was
wholly untested.

And one production fix, which nobody planned for: the superseding `Failed` row
that the fail-closed handler writes **did not win the LWW merge** and was
silently dropped, leaving an allocation durably recorded `Running` with no
interception installed. That is the exact surface the feature exists to defend,
and the feature's own first end-to-end test found it. See *"The port paid for
itself from a direction nobody predicted"* below.

Five phases:

### Phase 01 — kill the mutant at zero production cost, and delete both suppressions

**01-01** (`ce44a5e3`) authored S-MIF-01/02/03 as a default-lane
`#[cfg(test)] mod fail_closed_mtls_tests` inside
`crates/overdrive-control-plane/src/action_shim/mod.rs`, calling the
module-private `fail_closed_on_mtls_install` directly with all eight arguments
constructed in-process. In the **same commit** it deleted both suppressions: the
`.cargo/mutants.toml` `exclude_re` entry `"fail_closed_on_mtls_install"` with its
24-line justification comment, and the source-site `// mutants: skip` block. **No
production code changed.** S-MIF-01 is parameterised over six `cause` shapes and
carries ten assertions (A-1…A-10), including the forward-carried
`alloc_id`/`workload_id`/`node_id`/`kind`/`started_at` fields and
`TransitionSource` — added at DESIGN review iteration 1 because forward-carry drop
is a named bug class in this repo (GH #248).

This step is deliberately unmergeable with 02-01 (DFS-6): bundling it with the
port would have made the mutation gate's green depend on the port's correctness,
and would have destroyed the feature's central auditable claim — that the mutant
died at **zero** production cost.

### Phase 02 — extract the port as a mandatory 4th worker dependency

**02-01** (`2e02ddbf`) landed `mtls_intercept_port.rs` character-exact against
`architecture.md` § 4.1: the `InterceptGuard` marker trait, `impl InterceptGuard
for TproxyInterceptGuard`, the three-method `MtlsIntercept` trait with its
four-section rustdoc contract and a `# NO probe()` block, and `HostMtlsIntercept`.
`MtlsInterceptWorker::new` gains a **mandatory** 4th `Arc<dyn MtlsIntercept>`
parameter — not a builder override, not an `Option`, so a call site that forgets
it fails to compile. `AllocIntercept`'s two guard fields widen to
`Box<dyn InterceptGuard>`. All 1 production + 9 non-production call sites across 7
files pass `Arc::new(HostMtlsIntercept::new())`, preserving today's behaviour
byte-for-byte.

Behaviour-preserving refactor; **no scenario goes green here**, by design. The
gate is the compiler plus the existing Tier-3 suite staying green.

### Phase 03 — the fault-scripting simulation adapter

**03-01** (`1fe5ed76`) created
`crates/overdrive-sim/src/adapters/mtls_intercept.rs`: a `Clone`
`SimInterceptFault` descriptor expressed in the **real** error shapes the substrate
produces (`TransparentListener { errno }` / `TproxyInstall { reason }`, not a
generic boolean "fail now"), `SimMtlsIntercept` with **three independent**
fault slots, and a private `InertGuard`. Faults are **standing** (fire on every
call while armed), deliberately diverging from `SimMtlsResolve`'s consume-on-use
shape: a missing `CAP_NET_ADMIN` or an absent `nft` binary fails *every* call, and
`start_alloc` calls `bind_transparent` twice, so consume-on-use would make "which
leg failed" an artifact of ordering rather than a test's explicit choice. Added
the `overdrive-sim → overdrive-worker` `[dependencies]` edge (not a new edge
*class* — the transitive path already existed through `overdrive-control-plane`).
S-MIF-06/07/08/13 land default-lane and I/O-free: every fault arm short-circuits
before any syscall.

### Phase 04 — the call-site ordering test, then the production defect it found

**04-01** (`2deeaa49`) authored S-MIF-04 (`@keystone`, `Action::StartAllocation`)
and S-MIF-05 (`Action::RestartAllocation`) in
`crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs` —
Lima + root, `is_root()`-gated, driving the **real** `action_shim::dispatch`
through the production guard with a real netns provision. Litmus **L-4/L-5**
(moving the `release_for_exit_emission` block above the mTLS guard's `return`, one
arm at a time) turned assertion **A-6'** red on each arm independently and were
reverted — the port's sole justification, discharged.

The step also reproduced a real production defect, and **could not go green**
against it. See below. It landed with assertion A-1' `#[ignore]`d against a
reproduced diagnosis.

**04-02** (`e116b7c1`) — **the one production fix in this feature.** A new
module-private `superseding_timestamp(tick, superseded)` returning
`max(tick+1, superseded.counter+1)`, and `build_alloc_status_row`'s
`tick: &TickContext` parameter replaced by a **required**
`updated_at: LogicalTimestamp`. A-1' is un-ignored on both arms; S-MIF-01's
assertion A-2 is strengthened because its 01-01 fixture had used an artificial
different-counter shape that masked the defect.

### Phase 05 — host↔sim `Ok`-arm equivalence

**05-01** (`65fdbeaf`) authored S-MIF-09/10/11/12 in
`crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs`, each
parameterised over the `{HostMtlsIntercept, SimMtlsIntercept}` **adapter axis** —
an implementation axis, not a generative input space — driving both through the
same sequence and asserting the same observable contract: a non-zero assigned
port, distinct ports per call, installs returning `Ok(guard)`, guards dropping
cleanly including over already-released state. All four executed under root on
kernel `7.0.0-28-generic` for both adapters with zero skips.

The **fault arms are not equivalence-testable**, and the design says so rather
than claiming otherwise: the host adapter cannot be made to fail on demand, which
is the exact reason the port exists. They are pinned by the trait contract plus
the sim contract test (T3).

**b983ea06** — a post-05-01 adversarial-review remediation, worth recording
because it is the same discipline applied to the feature's own docs. Two
docstrings asserted coverage that did not exist (S-MIF-08 justified not re-driving
`bind_transparent` after `clear_faults()` by naming two tests that never call
`clear_faults` at all; the T4 contract table marked S-MIF-12's
non-duplication clause "asserted" when the test asserts only `Ok` + no-panic).
Both now state the gap and name the regression that would survive. A third fix
made `assert_err_shape` check the faulted `addr` the contract pins, which it had
been discarding.

---

## The issue's premise was wrong twice, and finding that out reshaped the work

GH #250 asked for a fault-injection seam so a killer test could be written for
`fail_closed_on_mtls_install`, whose whole-body `-> Ok(())` mutant was MISSED and
suppressed. DESIGN verified two findings against source, and each removed one of
the justifications the issue and the research had assumed. They are recorded as
**DFS-0a** and **DFS-0b** ahead of every other decision, because every other
decision depends on them.

**DFS-0a — the mutant was killable today, default-lane, at zero production
cost.** `fail_closed_on_mtls_install` is an `async fn` with no `pub` at
`action_shim/mod.rs:413`, and that file's own `#[cfg(test)] mod tests` already
reaches parent items via `use super::{…}`. All eight arguments are constructible
in-process with no I/O. The issue's stated blocker — that
`#[non_exhaustive]` on `MtlsInterceptInstallError` prevented constructing a cause
cross-crate — was **misdiagnosed**: the attribute is enum-level only, with no
per-variant `#[non_exhaustive]` and every variant public with public field types,
so per the Rust Reference it blocks exhaustive *matching*, not *construction*. The
"accept the gap / keep a permanent justified exclusion" branch — the branch the
Cilium precedent argues for — was therefore moot. There was no gap to accept.

**DFS-0b — a default-lane end-to-end test was impossible regardless, port or no
port.** `provision_and_inject_netns` is gated on the *same* `mtls_worker.is_some()`
flag and runs strictly **upstream** of the mTLS install. Setting
`mtls_worker: Some(..)` therefore forces real `ip netns` / veth shell-outs;
without root the allocation is driven `Failed` by the *sibling*
`fail_closed_on_netns_provision` handler and `worker.start_alloc` is never
reached. No port design choice changes that.

So the port was **not** justified by mutant-killability, and **not** by lane
placement. Its surviving justification is one leg, narrower than the issue's, and
the ADR states it without hedging: the port makes the **call-site ordering
property testable at all**. A helper-level test structurally cannot reach whether
`start_alloc`'s `Err` propagates to the helper, nor whether the `return` sits
before the `release_for_exit_emission`. That is what T2/A-6' asserts, and litmus
L-4/L-5 are what prove it asserts it.

Two evidence caveats were recorded so no reader mistakes the design for an appeal
to authority, and neither is cited anywhere as justification. Cockburn gives **no
criterion** for port granularity — *"largely a matter of taste"*, with *"no
particular damage in choosing the 'wrong' number"*. The mutation-testing
literature is **silent** on changing production code for killability, because the
field holds the program fixed and varies the suite. The mutation result is used
only as evidence that an unasserted specification-level behaviour exists.

The Cilium counter-precedent (`//go:build privileged_tests`, no kernel-surface
abstraction) is the nearest peer project and is **acknowledged and overridden on a
narrow, checkable ground**: its model exercises privileged code *for success* and
offers no mechanism to make a privileged call fail on demand. Under root the real
install succeeds; there is no Cilium-shaped way to reach the fail-closed handler
at all. The two approaches are complements, and this repo already runs both.

---

## The port paid for itself from a direction nobody predicted

Step 04-01's assertion **A-1'** was the first test in the codebase able to observe
the fail-closed path through the real `action_shim::dispatch`. On first execution
it reproduced a **genuine production defect**.

`fail_closed_on_mtls_install` built its superseding `Failed` row from the same
`tick` and the same `node_id` as the `Running` row it must supersede. Three legs,
each verified in source:

1. `timestamp_for(tick, writer)` returns
   `LogicalTimestamp { counter: tick.tick.saturating_add(1), writer }` — the
   counter derives **only** from the tick, with no per-write sequence.
2. The `Running` row and the superseding `Failed` row are constructed from the
   same `tick` in the same dispatch frame, and the `Failed` row copies
   `running_row.node_id` as its writer. Both carry a byte-identical
   `(counter, writer)`.
3. `LogicalTimestamp::dominates` returns `false` on `Equal` with the same writer —
   correctly, because a row with an equal timestamp genuinely is not newer.

So `failed_row.dominates(running_row) == false`, and the `Failed` row was
**silently dropped by both `ObservationStore` adapters** — the sim's
`apply_alloc_status` and the production `overdrive-store-local`
`apply_alloc_status_lww` that `run_server` wires. Both return `Ok(())`, so the lost
write is indistinguishable from success at the call site.

**Operator-visible consequence**: an mTLS install failure left the allocation
**durably recorded `Running` with no interception installed**. The driver *was*
stopped and the `LifecycleEvent` *was* emitted, so no workload kept running
uninstrumented — but the durable record lied, and that record is the exact surface
this feature exists to defend. The feature's own headline claim was unmet in
production.

**Why it survived until 04-01.** Step 01-01's helper-level test seeded the
`Running` row with `SEEDED_RUNNING_COUNTER = 0` and invoked the helper with
`TICK = 7`, so its assertion A-2 ("strictly greater counter") held in the fixture
and never in production. The artificial different-counter shape masked the defect.
Step 04-02 corrected the fixture, which moves the regression guard into the
**default lane** — litmus L-6 shows the corrected T1 going red on the reintroduced
defect, so the guard no longer depends on a Lima+root run.

The honest framing, recorded in ADR-0076 § Decision 7: this is the port's value
arriving from a direction the design did not anticipate. The justification was
call-site-ordering testability; what the call-site test actually caught first was
a durable-state defect.

### The fix generalises rather than patching one site

`superseding_timestamp(tick, superseded)` derives the counter from the row being
superseded — `max(tick+1, superseded.counter+1)` — and `build_alloc_status_row`'s
`tick` parameter becomes a **required** `updated_at: LogicalTimestamp`, so every
writer decides its stamp explicitly and the next missed site is a compile error.
Five non-supersede call sites pass byte-identical values; one supersede site passes
`superseding_timestamp(tick, running_row)`.

Three properties are load-bearing. `max`, not a bare `prior + 1`, keeps `tick`
live (arity unchanged, no unused-parameter lint), single-sources the tick base from
`timestamp_for` so the two cannot drift, and stays correct if a future supersede
site's prior row is behind the current tick. A **required parameter, not a
post-build patch** is the same discipline `build_alloc_status_row` already applies
to `workload_addr` for the identical bug class (GH #248) — mutating
`row.updated_at` after building is precisely the shape that comment rejects. And
the swap makes every writer's LWW stamp an explicit, reviewable decision at the
call site.

`LogicalTimestamp::dominates` was **not** changed. The comparator is correct and
its idempotency case (a re-delivered gossip row is a no-op) depends on the `false`;
the bug was in the counter the shim assigned.

**Blast-radius audit** (ADR-0076 § 7c) checked every candidate same-tick supersede
site: only the two mTLS fail-closed arms are affected, and both are fixed by the
one shared helper. `fail_closed_on_netns_provision` was **verified**, not assumed,
clean — it fires at the pre-`Running` provision seam and builds a fresh row with
nothing to supersede. The exit observer was already immune, having always derived
its successor row's counter from the prior row, and is the precedent the fix
follows.

Honest limit, stated in the ADR: this makes the bug class **visible and
deliberate, not impossible**. A future writer at a supersede site can still pass
`timestamp_for(...)`. The structural defense is the required parameter plus the
rustdoc on both helpers; there is no type that forbids the wrong choice.

---

## Key design decisions (from `design/wave-decisions.md` / ADR-0076)

| # | Decision | Why |
|---|---|---|
| DFS-0a / DFS-0b | Both of the issue's premises are refuted before any decision is taken | The mutant is killable today at zero production cost; a default-lane E2E test is impossible without a second port. Recorded first because every other answer depends on them. |
| DFS-1 | Extract `MtlsIntercept` as a 4th mandatory `Arc<dyn …>` on `MtlsInterceptWorker::new`; `action_shim` signatures unchanged. **Justified by one thing: the call-site-ordering test.** | Rejected: "T1 only, skip the port" — a helper-level test cannot assert that `start_alloc`'s `Err` reaches the helper nor that the `return` precedes the release; and it permanently forecloses the port because the mutant, the only forcing function, would already be dead. Rejected: a port at the `action_shim` boundary — abstracts the wrong thing (the un-ownable surface is `libc`/`nft`, not code we own). Rejected: a `#[cfg(feature)]` fault field — the default lane compiles *without* the feature, so the seam is invisible to the test it exists to enable. |
| OQ-1 | **3 methods**, not one `install(spec)` | A single method moves `start_alloc`'s ordering and partial-teardown discipline into the adapter, so a test double would *replace* logic we own. Three methods put the boundary exactly at the un-ownable surface and keep the worker's ordering exercised. |
| OQ-2 | **Sync**, no `#[async_trait]`; `Arc<dyn>`, not a generic parameter | Every underlying primitive is a blocking syscall or a blocking `Command`; `start_alloc` is `pub fn`; nothing awaits store I/O. A generic would propagate virally through the worker, `AppState`, and every dispatch call site for one vtable dispatch on a path that already spawns `nft`. |
| OQ-3 | Trait + host adapter in **`overdrive-worker`**; sim adapter in `overdrive-sim` | Stated as a **trade-off**, not an impossibility (the first draft over-claimed and review corrected it). The structural half: relocating `TproxyInterceptGuard` into `core` would put a real-`nft` `Drop` on a `crate_class = "core"` compile path. The judgement half: core placement would require minting a duplicate core-side error type. Precedent exists — `SimViewStore` implements a port declared in `overdrive-control-plane`. |
| OQ-4 | **NO `probe()`, no boot gate — STRUCK** at user direction (rev 4) | A production behaviour change (`overdrive serve` refusing to start where it previously started), out of #250's scope, buying better boot-time *diagnosis* rather than a new safety property — a capability-less node already fails every deploy at the upstream netns seam. The trait carries a `# NO probe()` rustdoc block naming ADR-0076 § Decision 4 as the decision a future crafter would have to supersede. |
| OQ-5 | Nothing blocks construction; **no escape hatch is added** | The five private `const fn` constructors on `MtlsInterceptInstallError` stay private — the worker remains its only constructor. Rejected: a `pub fn install_failed(...)` (unnecessary, and widens a security-critical error type's construction surface) and a `#[doc(hidden)] pub fn __for_test(...)` (public API pretending not to be). |
| OQ-6 | Cover **both** dispatch arms with two test functions; do not collapse the duplication | What is duplicated is a 6-line guard whose body already delegates to the shared helper; the two blocks close over different locals. Extraction would need a control-flow-signal return type. The second arm defends against a future divergent edit to one block. |
| DFS-3 | `InterceptGuard` marker trait + `Box<dyn InterceptGuard>` | A sim adapter must not return a real `TproxyInterceptGuard` — its `Drop` shells out to `nft`. Rejected: an `inert()` constructor on the concrete guard, and an `Option` guard field in production — both are production shaped by simulation. A guard's entire contract *is* its `Drop`, so a method-less trait is the accurate shape. |
| DFS-4 | Sim faults are **standing**, not consume-on-use | A missing capability or absent binary fails every call; standing faults also remove call-order dependence, since `start_alloc` binds twice. |
| DFS-5 | The sim's `Ok` arm binds a **real, plain** loopback listener — and this is documented as pushing any test that drives it into the integration lane | There is no way to fabricate a `TcpListener` without a syscall. The consequence is scoped, not hidden: T1 never touches the sim, and T3 drives only fault arms, which short-circuit before any syscall. |
| DFS-6 | T1 + both suppression deletions land as **one step, first**, before the port | Independently valuable, independently gated, needs no production change. Bundling would make the gate's green depend on the port's correctness. |
| DFS-7 | The trait contract states **only what both adapters can honour**; substrate specifics live on `HostMtlsIntercept`'s own rustdoc | The first draft stated `IP_TRANSPARENT`, "exactly ONE nft rule", and guard-`Drop`-removes-a-rule as *trait* postconditions — making the contract unimplementable by half its sanctioned implementors. Post-split, the trait contract and T4's asserted set coincide exactly, and the honest gap shrinks to the fault arms alone. |
| DFS-8 | Lima+root integration tests **participate in the mutation gate** | The canonical CI invocation is Lima-wrapped and runs as root by default. Default-lane placement is a **wall-clock** property, not a coverage one — which is why DFS-0b is a cost, not a blocker, and why no second port is needed here. |
| DFS-9 | The superseding `Failed` row derives its LWW counter from the row it supersedes — **and the "no production behaviour change" claim is withdrawn, not qualified** | Rejected: changing `dominates` to let an equal timestamp win (would break LWW idempotency and make acceptance order-dependent across gossip replay). Rejected: installing the intercept *before* writing the `Running` row (deletes an observable durable transition; leaves a spawned driver with no row if the `Failed` write then fails). Rejected: a per-write `AtomicU64` in the shim (threads mutable state through `dispatch` and is restart-unsafe). Rejected: fabricating a second `TickContext` (a lie about which tick the write belongs to). Rejected: patching `row.updated_at` after building (the shape `build_alloc_status_row`'s own comment rejects). |

The design passed **two review iterations**. Iteration 1 returned
`rejected_pending_revisions` — 0 critical, 5 high — a mechanical rejection on the
HIGH count, not a judgement against the approach; both load-bearing findings
(DFS-0a, DFS-0b) were independently verified against source and confirmed. The
five HIGHs were: the boot probe's justification not surviving verification; an
"exhaustive" call-site table that omitted one row; T1 asserting none of the
forward-carried fields (the #248 bug class); the mutation obligation being
narrower than the gate actually scores; and a trait contract stating
postconditions the sanctioned sim adapter could not honour. Iteration 2 returned
`approved` — 0 critical, 1 high — with the remaining findings closed in the
following revision. Rev 4 was user direction (narrow the justification to one leg;
strike the probe), and rev 5 was the production defect.

---

## Steps completed (from `execution-log.json`)

| Step | Phase ledger | Outcome |
|---|---|---|
| 01-01 | RED / GREEN / COMMIT | all PASS |
| 02-01 | RED / GREEN / COMMIT | all PASS |
| 03-01 | RED / GREEN / COMMIT | all PASS |
| 04-01 | RED PASS → **GREEN FAIL** → COMMIT SKIPPED (`BLOCKED_BY_DEPENDENCY`) → COMMIT PASS | landed with A-1' `#[ignore]`d against a reproduced diagnosis |
| 04-02 | RED / GREEN / COMMIT | all PASS — the production fix; A-1' un-ignored on both arms |
| 05-01 | RED / GREEN / COMMIT | all PASS |

### 04-01's ledger has no superseding `GREEN/PASS`, and that is deliberate

The sequence reads `GREEN/EXECUTED/FAIL → COMMIT/SKIPPED/BLOCKED_BY_DEPENDENCY →
COMMIT/EXECUTED/PASS`. GREEN genuinely failed, because A-1' could not pass against
the then-undiagnosed production defect, and the step landed with A-1' `#[ignore]`d
against a *reproduced* diagnosis rather than a guess. The `COMMIT/SKIPPED` entry
carries the full diagnosis inline, including that both litmus L-4 and L-5 were
confirmed RED and reverted and that `action_shim/mod.rs` was verified byte-clean
against HEAD before anything was committed.

**An adversarial review asked for a retroactive `GREEN/PASS` to be injected. It
was declined as misreporting.** The ledger records what happened;
`des-verify-integrity` accepts the sequence as complete, and the reasoning is
preserved in `deliver/litmus-evidence.md` so a later reader does not mistake the
gap for an omission.

---

## Falsification evidence

`execution-log.json`'s schema (`{sid, p, s, d, t}`) cannot hold prose, so the
observations the roadmap demanded — *"record the observed failing test name and
assertion"* — live in `deliver/litmus-evidence.md`. The governing rule is from
DISTILL: **a green suite with no litmus recorded is an unfalsified pass and must
be rejected.** Every revert used the `Edit` tool; `git checkout --` is blocked by
the destructive-git-ops hook and was never used.

Seven litmus edits, each applied → observed RED → reverted:

| # | Edit | Turned RED |
|---|---|---|
| L-1 | whole body of `fail_closed_on_mtls_install` → `Ok(())` | all three T1 tests, plain assertion failures (`left: Running, right: Failed`), not scaffold panics |
| L-2 | `let _ = driver.stop(handle).await;` → `driver.stop(handle).await?;` | S-MIF-02 only |
| L-3 | `obs.write(..).await?;` → `let _ = obs.write(..).await;` | S-MIF-03 only |
| L-4 | move the `release_for_exit_emission` block above the `StartAllocation` arm's mTLS-guard `return` | S-MIF-04 A-6' — `got releases [AllocationId("mif-start")]` |
| L-5 | the same move in the `RestartAllocation` arm | S-MIF-05 A-6' — `got releases [AllocationId("mif-restart")]` |
| L-6 | `superseding_timestamp`: `superseded.updated_at.counter.saturating_add(1)` → `superseded.updated_at.counter` | both corrected T1 tests, all six cases — **default lane** |
| L-7 | `fail_closed_on_mtls_install`'s 5th argument back to `timestamp_for(tick, running_row.node_id.clone())` | S-MIF-04 and S-MIF-05 A-1', both arms — `left: 1, right: 2` rows, the survivor being `state: Running` |

Three of these carry more signal than a bare RED:

- **L-1 is independently corroborated mechanically** — cargo-mutants generates the
  same `FnValue → 'Ok(())'` replacement and reports it `CaughtMutant`, applied by
  the tool against an independently built binary.
- **L-4/L-5 discharge the port's sole justification.** A-6' dies on the reordering
  on both arms independently, and **survives step 01-01's T1 entirely** — a
  helper-level test structurally cannot reach a call-site ordering property.
- **L-7 is decisive about A-1' specifically.** Under the reintroduced defect the
  *other two* tests (A-6'/A-8'/A-9') still **passed**. That is direct proof A-1'
  is the only assertion observing supersession, and that un-ignoring it was
  justified.

Step 01-01 also carried a structural proof that no revert drifted: after all
applications and reverts, `git diff -U0` on `action_shim/mod.rs` returned exactly
the 10 lines of the deleted `// mutants: skip` block and nothing else.

---

## Two mutation results are recorded unflatteringly, on purpose

The mutation gate produced three figures across the feature, and two of them say
less than the number suggests. Both are recorded as such rather than laundered
into a kill rate.

**Step 01-01 — 1 mutant, caught, reported VACUOUS.** A 100% rate over a
one-mutant set is not a coverage claim, and DISTILL required the actual mutant set
to be enumerated off the guest `outcomes.json` **before** any kill-rate figure was
stated. (The diff-scoped `--file` shape yields `No mutants to filter` here, because
01-01's diff is entirely `#[cfg(test)]`; the whole-file
`--workspace --package --file` shape is required.) Assertions A-8/A-9/A-10 rest on
the #248 forward-carry bug class, not on mutation coverage.

**Step 03-01 — `Found 0 mutants to test`.** Not a vacuous diff-shape artifact but a
**real exclusion**: `.cargo/mutants.toml` Rule 7 excludes
`crates/overdrive-sim/src/adapters/**` wholesale. **This was not reported as a kill
rate.** Four load-bearing mutants were closed by manual flip-proof instead —
dropping the armed errno, `.take()` instead of `.clone()` on the standing-fault
read, `clear_faults` omitting the inbound slot, and `script_inbound_fault` writing
an aliased slot — 4/4 killed, 239/239 green on the post-revert re-run. The
`.take()` mutant doubles as the **positive control for the I/O-free claim**: when
it made the `Ok` arm reachable on the second call, the test failed carrying a
literally-bound socket in its panic payload, which is direct proof the unmutated
code never reaches the bind.

**Step 04-02 — 3 mutants, 3 caught, 100%, and the figure says nothing about the
fix.** The three are whole-body replacements on `fail_closed_on_mtls_install`,
`fail_closed_on_netns_provision`, and `dispatch_single`. **`superseding_timestamp`
generated ZERO mutants**: `LogicalTimestamp` derives no `Default` (whole-body
operator unviable), `saturating_add(literal)` yields nothing in this repo, and
`.max(..)` is a method call rather than a mutable binary operator. The fix's
defense rests **entirely** on litmus L-6 and L-7. No `exclude_re` entry was added.

**The final unscoped per-PR gate** — the invocation CI actually runs —
returned `total=4 caught=4 missed=0 timeout=0 unviable=0 kill_rate=100.0%
status=pass` over `fail_closed_on_mtls_install`, `fail_closed_on_netns_provision`,
`dispatch_single`, and `MtlsInterceptWorker::start_alloc`. cargo-mutants now
*generates* the `fail_closed_on_mtls_install -> Ok(())` mutant — the `exclude_re`
deletion held, it is no longer suppressed — and it is **caught**. Zero missed
mutants, so no new `exclude_re` entry was added for `HostMtlsIntercept`'s
delegations either.

---

## Struck, out of scope, and recorded-but-not-fixed

Nothing in this feature carries a hand-wavy forward pointer. Each item below is
either struck outright with no promised slice, or closed against an issue number
verified with `gh issue view --comments`. **No GitHub issue was created.**

- **The boot `CAP_NET_ADMIN` probe — STRUCK** (ADR-0076 § Decision 4,
  `architecture.md` § 8.2). Removed entirely: the trait method, the host impl, the
  `InterceptError::Probe` variant, the `MtlsBootError::InterceptProbe` variant, the
  `run_server` gate, the sim's probe scripting, and the verification-catalogue
  graduation it would have produced. Recorded as out of scope with **no forward
  pointer, no promised future slice, and no issue number** — the "drop the deferral
  language" option. It is listed in the design's *Rejected in full* section so it is
  not re-proposed as an obvious omission, and the trait's rustdoc carries a
  `# NO probe()` block naming the decision that would have to be superseded.
  A consequence knowingly left in place: a capability-less node still refuses every
  deploy under `WorkloadNetnsProvisionFailed` rather than a cause naming the
  missing capability.
- **The `WorkloadNetns` port** that would make the end-to-end test default-lane —
  out of scope, closed against **GH [#197](https://github.com/overdrive-sh/overdrive/issues/197)**
  (veth → first-class network reconciler), whose **Scope item 1** is verbatim that
  port. Two independent reasons not to build it here: it would land #197's port
  without the reconciler it exists to serve and prejudge that design; and per DFS-8
  it buys **no gate coverage** this design lacks, only wall-clock and local
  ergonomics.
- **A host-adapter fault arm (checklist C6a)** — deliberately not authored.
  Widening T4 beyond its `Ok`-arm equivalence scope is a DESIGN call, not a DISTILL
  one; it was surfaced as a reviewer observation under T4 instead.
- **Three systemic LWW findings** surfaced by the blast-radius audit (ADR-0076
  § 7d). None is caused by this feature, none is fixed by it, and **none is a
  deferral with a promised slice** — they are recorded as observed facts, with no
  forward pointer and no issue:
  1. **Cross-restart counter regression** (the largest). `tick_n` is a local
     initialised to `0` in `spawn_convergence_loop`, incremented ~864,000/day at the
     100 ms cadence, and never seeded from anything persistent — while `alloc_status`
     rows *are* durable across restart. Post-restart writes for a pre-existing alloc
     are therefore dropped by LWW until the tick counter catches up.
     **Verified from source; NOT reproduced at runtime** — no existing restart test
     writes a post-restart row and asserts it wins.
  2. **Next-tick tie residual.** A same-tick supersede consumes counter `tick+2`, so
     an ordinary write on the immediately following tick ties. Pre-existing in
     identical shape for the exit observer, whose module doc already documents it as
     accepted. § 7b's fix does not close it.
  3. **Other tick-derived row types.** `ServiceBackendRow` is written by two
     reconcilers both using `tick.tick + 1` and keyed on `service_id` alone
     (reachability not audited); `NodeHealthRow` uses wall-clock seconds, so two
     heartbeats in one second collide — benign at current intervals.

  Findings 1 and 2 share one remedy — monotone-against-prior at every write site —
  which is a larger decision than this feature. DFS-9's required `updated_at`
  parameter is the **enabling precondition** for it, not a down payment on it.

Two further items were recorded as observations rather than scope. The
`Option<&AllocationHandle>` parameter on `fail_closed_on_mtls_install` is wider
than production needs (`state == Running` ⟺ `handle == Some`), but narrowing it is
a production signature change and this feature edits that helper's signature not at
all. And `install_outbound(host_veth: &str, …)` keeps a raw `&str` where the other
methods take typed parameters, because § 4.1 pins it verbatim and inventing API
surface is forbidden.

---

## Lessons learned

- **Ground the premise before designing the fix — and re-ground it at every
  wave.** Two of the three justifications the issue assumed did not survive source
  verification. Had DESIGN inherited them, the feature would have shipped a port
  advertised as doing something it demonstrably does not (kill the mutant — the
  mutant was already killable) and as enabling something it cannot (a default-lane
  E2E test). The honest response was to narrow the justification to one leg and say
  so once, without hedging. This is the same discipline CLAUDE.md § "Ground the
  premise" names, applied to a justification rather than to a defended state.
- **A test that can reach production for the first time will find what nothing
  else could.** A-1' was designed to observe supersession as a supporting assertion;
  what it actually did was reproduce a durable-state defect that had shipped, gone
  unnoticed, and defeated the exact security control the feature existed to defend.
  The port's justification and the port's payoff came from different directions, and
  the record says so rather than retrofitting the story.
- **A fixture that differs from production in one field can mask a whole defect
  class.** 01-01's helper test seeded a different counter than production produces,
  so its "strictly greater counter" assertion held in the fixture and never in the
  system. Correcting the fixture was as valuable as the production fix — it moved
  the regression guard from a Lima+root lane into the default lane.
- **Report the mutation number the tool actually justifies.** Three gates in this
  feature reported figures; two of them (a one-mutant 100%, and a 100% over a
  function that generated no mutants at all) say nothing about what they appear to
  cover. Recording them as vacuous, naming the mutants verbatim before stating any
  rate, and closing the real gaps by manual flip-proof is the only version of the
  gate that means anything. cargo-mutants' blind spots — `spawn_blocking` bodies,
  `saturating_add(literal)`, method calls where a binary operator was expected — are
  a known property of the tool, not a property of the code.
- **Do not repair a ledger to look tidy.** 04-01's `GREEN/FAIL → COMMIT/SKIPPED →
  COMMIT/PASS` is what happened. Injecting a retroactive `GREEN/PASS` would have
  made the audit trail agree with a story nobody lived, and the DES log is a
  contract.
- **Fix the trait contract, not the adapter, when adapters diverge.** Review
  iteration 1 caught the trait stating substrate postconditions (`IP_TRANSPARENT`,
  "exactly ONE nft rule") that the sanctioned sim adapter could not honour. Four
  rustdoc sections were present, so the rule was formally satisfied and
  substantively violated. Splitting the contract made T4's asserted set coincide
  exactly with what the trait claims, and shrank the honest gap to the fault arms —
  which is what the design had always claimed it was.

---

## Migrated permanent artifacts

- **Design record**: [ADR-0076](../product/architecture/adr-0076-mtls-intercept-port-fault-injectable-privileged-install-surface.md)
  (rev 5 — already permanent). It is the design SSOT for this feature; there is no
  `feature-delta.md`, because the feature entered from the issue and research
  directly into DESIGN.
- **Acceptance scenarios**: `docs/scenarios/mtls-intercept-install-fault-seam/test-scenarios.md`
  (migrated from the feature workspace — the 13 S-MIF scenarios with their Universe
  declarations, assertion tables, and the mutation-gate traceability section).
- **Research**: `docs/research/testing/fault-injection-seam-fail-closed-paths-research.md`
  (already permanent — 29 cited sources; the source of the Cilium counter-precedent,
  the Cockburn and mutation-literature negative results, and the Saltzer & Schroeder
  / Yuan et al. fail-safe-defaults asymmetry the seam cost is justified against).
- **History**: the feature workspace
  `docs/feature/mtls-intercept-install-fault-seam/` is **preserved** — the intake
  issue, the DESIGN `architecture.md` (verbatim API surface, including § 4.8's
  character-exact production fix) and `wave-decisions.md`, the DISTILL
  `test-scenarios.md` and `red-classification.md`, and the DELIVER `roadmap.json`,
  `execution-log.json` and `litmus-evidence.md`. This evolution doc is the summary;
  the workspace is the full record, and `litmus-evidence.md` in particular is the
  only durable home for the falsification observations the phase ledger cannot hold.
