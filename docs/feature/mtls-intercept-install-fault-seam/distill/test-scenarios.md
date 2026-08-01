# Test scenarios — `mtls-intercept-install-fault-seam`

**Wave**: DISTILL | **Mode**: PROPOSE | **Designer**: Quinn (nw-acceptance-designer) | **Date**: 2026-08-01 | **Feature**: GH #250

> **Executable acceptance specification.** This document is the
> GIVEN/WHEN/THEN **SSOT** for the feature — **no `.feature` files** (per
> `.claude/rules/testing.md` § "No `.feature` files anywhere"). DELIVER
> translates each scenario into a Rust `#[test]` / `#[tokio::test]` body.
> Gherkin below is specification prose; nothing parses it.
>
> **Contract this distills**: `design/architecture.md` (§ 4.1 verbatim API
> surface, § 5.1–5.4 the four test families, § 6 the mutation-gate contract,
> § 9 the DELIVER ordering constraint), `design/wave-decisions.md`
> (DFS-0a/0b, DFS-1…DFS-8, OQ-1…OQ-9), **ADR-0076 rev 4**. The design is
> **ACCEPTED and BINDING**; DISTILL picks **no** new signature (CLAUDE.md
> § "Implement to the design — never invent API surface").
>
> **Lang**: Rust (`[lang-mode] rust`, `Cargo.toml` marker).
> **Policy**: `inherit` — `docs/architecture/atdd-infrastructure-policy.md`
> exists; the four MIF rows are appended there by this wave.
> **Port bootstrap**: no `tests/common/state_delta.rs` — the Rust
> universe-guard mapping is recorded in the policy file's Mandate-8 note and
> is satisfied natively by exact-set / exact-field assertions (see
> § "Universe discipline" below).

---

## Wave-Decision Reconciliation HARD GATE — PASS (0 contradictions)

Ran before any scenario was written (`nw-distill` § "Wave-Decision
Reconciliation HARD GATE").

Files read:

- `+ docs/feature/mtls-intercept-install-fault-seam/design/architecture.md`
- `+ docs/feature/mtls-intercept-install-fault-seam/design/wave-decisions.md`
- `+ docs/product/architecture/adr-0076-mtls-intercept-port-fault-injectable-privileged-install-surface.md` (rev 4)
- `+ docs/feature/mtls-intercept-install-fault-seam/issue-250.md`
- `+ docs/research/testing/fault-injection-seam-fail-closed-paths-research.md` (§ RQ3 Findings 3.1–3.3)
- `+ docs/architecture/atdd-infrastructure-policy.md`
- `- docs/feature/mtls-intercept-install-fault-seam/discuss/` (**not found** → WARN)
- `- docs/feature/mtls-intercept-install-fault-seam/devops/` (**not found** → WARN)

**Graceful degradation applied.** No `discuss/` → acceptance criteria are
derived from DESIGN + the originating issue; story-to-scenario traceability is
skipped (there are no user stories — this is a bug/test-infrastructure feature
sourced from GH #250, and per `nw-distill` § "Acceptance Criteria" that is the
sanctioned bug-fix AC shape: *"When {trigger}, {modified_code_path} produces
{correct_outcome} instead of {current_broken_behavior}"*). No `devops/` →
default environment matrix, mapped onto the project's four-tier model below.
DESIGN is **present**, so the driving-port gate does not block.

**Cross-wave contradiction check.** Only one wave-decisions file exists, so
there is no DISCUSS↔DESIGN↔DEVOPS triangle to contradict. The one
premise-level disagreement in the record is **already reconciled inside
DESIGN** and is reproduced here so DELIVER does not re-litigate it:

| Claim | Source | Status |
|---|---|---|
| *"`mtls_worker` is a concrete type … To inject an install failure in a **default-lane** `dispatch_single` test, the worker needs a fault-injection seam"* | `issue-250.md` § "Why it isn't trivially testable today" + § Scope bullet 2 | **CORRECTED by DESIGN, not contradicted-and-ignored.** DFS-0a: the helper is module-private and every argument is default-lane constructible, so the mutant is killable today at zero production cost. DFS-0b: a `dispatch`-level test can **never** be default-lane, port or no port, because `provision_and_inject_netns` is gated on the same `mtls_worker.is_some()` flag and shells out to real `ip netns`. Both were independently verified at DESIGN review iteration 1 and CONFIRMED. |
| *"Remove the `.cargo/mutants.toml` `exclude_re` exclusion … once the killer test lands"* | `issue-250.md` § Scope bullet 3 | **CONSISTENT** with OQ-9 / § 6 — strengthened: the source-site `// mutants: skip` block is deleted in the same commit, and the obligation is 100 % of the function's mutants, not just the whole-body one. |
| Boot `CAP_NET_ADMIN` `probe()` (revs 1–3 of the ADR) | ADR-0076 revs 1–3 | **STRUCK at rev 4** (§ 8.2 / OQ-4). architecture.md, wave-decisions.md and ADR-0076 rev 4 all agree: no `probe()`, no boot gate, **no production behaviour change**. No residue found in any of the three. |

Reconciliation **PASS — 0 contradictions**.

---

## Scope + strategy

**Scope**: a fault-injection seam for the *existing* transparent-mTLS
intercept-install fail-closed path, plus the tests that seam makes possible.
The feature ships **no production behaviour change** (ADR-0076 § Decision 4)
— `run_server` gains no gate, no new failure mode, and no new
`health.startup.refused` reason.

**What the tests must establish**, in the design's own order of justification:

1. **The specification of `fail_closed_on_mtls_install` is asserted at all**
   (T1). This is what un-suppresses the mutation gate. It needs **no**
   production change (DFS-0a).
2. **The call-site ordering property is asserted at all** (T2, A-6'): on an
   install failure the `StartAllocation` / `RestartAllocation` arms `return`
   **before** `driver.release_for_exit_emission(handle)`
   (`action_shim/mod.rs:1307` before `:1319`; `:1507` before `:1519`), so a
   now-`Failed` alloc never releases its exit watcher. **This is the ONE leg
   that justifies the port** (DFS-1). Nothing in the tree can force
   `start_alloc` to fail on demand today.
3. **The new sim adapter honours its own scripted-fault contract** (T3).
4. **Host and sim do not diverge on the `Ok` arms of the new trait** (T4) —
   the DST equivalence-test obligation
   (`.claude/rules/development.md` § "The DST equivalence test is the
   structural guard").

**Strategy** (tiers per `.claude/rules/testing.md`):

- **Tier 1 — in-crate unit, default lane** (`action_shim/mod.rs`
  `#[cfg(test)] mod fail_closed_mtls_tests`): S-MIF-01/02/03. Zero I/O — no
  socket, no netns, no subprocess, no tempdir. Runs under bare
  `cargo nextest run`, and therefore under `cargo xtask mutants` on macOS
  **without** Lima.
- **Tier 1 — sim-adapter contract, default lane**
  (`overdrive-sim/src/adapters/mtls_intercept.rs` `#[cfg(test)] mod tests`):
  S-MIF-06/07/08/13. Fault arms only, so they short-circuit **before** any
  syscall (DFS-5) and the default lane stays I/O-free.
- **Integration — Lima + root, `is_root()`-gated** (control-plane
  `tests/integration/mtls_install_fail_closed.rs`): S-MIF-04/05. Real netns +
  veth provisioning runs upstream (DFS-0b), so root is structural, not
  incidental.
- **Integration — Lima + root** (worker
  `tests/integration/mtls_intercept_equivalence.rs`): S-MIF-09/10/11/12. The
  sim's `bind_transparent` `Ok` arm binds a **real, plain** loopback socket
  (DFS-5), so even the sim half is integration-lane.
- **No Tier 2** — this feature adds no kernel-side eBPF program; there is no
  `BPF_PROG_TEST_RUN` target.
- **No Tier 4** — no new BPF instruction budget, no XDP throughput surface.

**Driving ports exercised**: `action_shim::dispatch` (the production entry
point for both `Action::StartAllocation` and `Action::RestartAllocation`) —
S-MIF-04/05. The `MtlsIntercept` trait is the **driven** port under
substitution; `HostMtlsIntercept` / `SimMtlsIntercept` are its two adapters.

**Error-path coverage**: **11 / 13 ≈ 85 %**. The feature *is* an error path;
the only non-error scenarios are the two `Ok`-arm equivalence cases
(S-MIF-09/10) — and even S-MIF-11/12 exercise guard-drop-after-release edges.
The ≥ 40 % target is met with margin.

### Universe discipline (Mandate 8, Rust mapping)

Every scenario below declares a **Universe**: the set of *port-exposed*
observables it promises to track. Per the policy file's Mandate-8 note the
Rust equivalent of `assert_state_delta(before, after, universe, expected)` is
a native exact-field / exact-set assertion over that declared set — an
unexpected mutation of a declared observable fails the assertion exactly as a
`strict=True` state-delta would. **No universe entry is a private field.**
Concretely the universes here are: rows read back out of `ObservationStore`,
`Driver` trait-method call records, `LifecycleEvent`s received on the
`broadcast` bus, the `Result` returned by the unit under test,
`NetSlotAllocator::snapshot()`, and `TcpListener::local_addr()`.

### PBT / parametrize density (Mandate 9)

| Layer | Mode | Applied to |
|---|---|---|
| Tier 1 (default lane, layers 1–2) | **parametrize** (`rstest` cases, or a table-driven loop over a `const` slice — DELIVER picks the idiom already used in the touched module) | S-MIF-01 (6 cause shapes), S-MIF-06 (4 sanctioned pairings), S-MIF-13 (3 arm-one-fault directions) |
| Tier 1 (default lane) | example-only | S-MIF-02, S-MIF-03, S-MIF-07, S-MIF-08 — each pins a single named invariant with one meaningful input |
| Integration (layers 3+) | **example-only, sad paths enumerated** (Mandate 11) | S-MIF-04/05 (two named arms), S-MIF-09/10/11/12 (parametrised over the *adapter*, `{Host, Sim}` — an adapter axis, not a generative input space) |

**No `proptest` / generative PBT anywhere in this feature.** The argument
spaces are small closed enumerations (5 error variants → 4 stage strings; 2
fault descriptors; 3 sim slots; 2 adapters; 2 dispatch arms). Manufacturing a
generator over a 6-element closed set would be parametrisation theatre.
`.claude/rules/testing.md` § "Property-based testing" puts the proptest
trigger at *"a pure function's argument space exceeds a dozen hand-picked
cases"* — none here does.

---

## Environment mapping (no feature-local `devops/`)

The generic default matrix applies as a fallback; the project's test-tier
model is the real environment taxonomy. Each default-matrix environment is
mapped or explicitly waived.

| Default-matrix env | Mapping / waiver | Rationale |
|---|---|---|
| `clean` | **mapped** → S-MIF-04/05 (fresh netns + `NetnsGuard` pre-sweep, fresh `SimObservationStore`, fresh `NetSlotAllocator`), S-MIF-09..12 (fresh Lima boot) | the feature's real "clean" environment is a swept Lima VM + a fresh in-process composition, not an installer clean-install |
| `with-pre-commit` | **waived** | the feature touches no git-hook surface |
| `with-stale-config` | **waived** | the feature reads no config file and performs no migration |

Real (tier-based) environments the scenarios exercise:

| Tier env | Scenarios | Mechanism |
|---|---|---|
| Tier-1 in-process (default lane) | S-MIF-01, -02, -03 | direct call of the module-private `fail_closed_on_mtls_install`; `SimObservationStore::single_peer`, `broadcast::channel(16)`, a test-local `RecordingDriver`. Zero I/O. |
| Tier-1 sim-contract (default lane) | S-MIF-06, -07, -08, -13 | `SimMtlsIntercept` fault arms only — short-circuit before any syscall |
| Integration — Lima + root (`is_root()`-gated) | S-MIF-04, -05 | `cargo xtask lima run --` + real `action_shim::dispatch` + real `ip netns`/veth + `SimDriver`-backed `RecordingDriver` + `SimMtlsIntercept` armed |
| Integration — Lima + root | S-MIF-09, -10, -11, -12 | `cargo xtask lima run --` + real `libc::socket`/`setsockopt`/`nft` through `HostMtlsIntercept`, and a real plain loopback bind through `SimMtlsIntercept` |

**Leak hygiene is mandatory, not optional** for S-MIF-04/05/09..12:
`NetnsGuard`-style RAII cleanup **plus** the explicit pre-sweep at each use
site (`alloc_netns_lifecycle.rs:168-176`, `:371-372`), and `nft`-rule teardown
via guard `Drop` for the Host adapter. This repo has a documented cross-run
leak-hazard class for exactly this shape (`.claude/rules/testing.md` § leaked
workload cgroups; `.claude/rules/debugging.md` § leftover XDP attachments). A
test without the guard poisons every subsequent Lima run.

---

## Scenario index

| ID | Title | Tags | Lane / tier | Target file | Mutation target |
|---|---|---|---|---|---|
| S-MIF-01 | An intercept-install failure drives the allocation Failed, stops the driver, and never releases the exit gate | `@in-memory` `@error_path` `@parametrized` `@security` | 1 / default | `crates/overdrive-control-plane/src/action_shim/mod.rs` (`#[cfg(test)] mod fail_closed_mtls_tests`) | **YES — the suppressed whole-body `-> Ok(())` mutant** |
| S-MIF-02 | A driver stop that errors does not prevent the Failed row | `@in-memory` `@error_path` | 1 / default | same | yes (`let _ =` → `?` on `driver.stop`) |
| S-MIF-03 | An observation-store write rejection surfaces as an error and emits no lifecycle event | `@in-memory` `@error_path` | 1 / default | same | yes (`obs.write(..).await?` → swallow; emit-before-write reorder) |
| S-MIF-04 | A failed intercept install on a fresh allocation never releases its exit watcher | `@keystone` `@driving_port` `@real-io` `@error_path` `@security` | int / Lima+root | `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs` | **YES — the `StartAllocation` call-site ordering (`:1307` before `:1319`)** |
| S-MIF-05 | A failed intercept install on a restarted allocation never releases its exit watcher | `@driving_port` `@real-io` `@error_path` `@security` | int / Lima+root | same | **YES — the `RestartAllocation` call-site ordering (`:1507` before `:1519`)** |
| S-MIF-06 | An armed intercept fault surfaces as the real error the substrate produces | `@in-memory` `@error_path` `@parametrized` | 1 / default | `crates/overdrive-sim/src/adapters/mtls_intercept.rs` (`#[cfg(test)] mod tests`) | yes (fault→error materialisation) |
| S-MIF-07 | An armed intercept fault is standing: it fires on every subsequent call | `@in-memory` `@error_path` | 1 / default | same | yes (`.take()` consume-on-use regression, DFS-4) |
| S-MIF-08 | Clearing the faults disarms all three install steps, and clearing an unarmed double is a no-op | `@in-memory` `@error_path` | 1 / default | same | yes (partial `clear_faults`) |
| S-MIF-09 | A bound intercept listener reports the concrete port the kernel assigned | `@real-io` `@adapter-integration` `@equivalence` | int / Lima+root | `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs` | yes (port-0 passthrough) |
| S-MIF-10 | Two intercept listeners never share a port | `@real-io` `@adapter-integration` `@equivalence` | int / Lima+root | same | yes (cached/singleton listener) |
| S-MIF-11 | An install hands back a guard that releases cleanly | `@real-io` `@adapter-integration` `@equivalence` | int / Lima+root | same | yes (guard `Drop` panic/error) |
| S-MIF-12 | Re-installing the same capture converges instead of duplicating, and both guards release cleanly | `@real-io` `@adapter-integration` `@equivalence` `@idempotency` | int / Lima+root | same | yes (non-idempotent install; double-release panic) |
| S-MIF-13 | Arming one install step leaves the others on their success arms | `@in-memory` `@orthogonality` | 1 / default | `crates/overdrive-sim/src/adapters/mtls_intercept.rs` | yes (shared/aliased fault slot) |

**13 scenarios.** Family mapping: T1 → S-MIF-01/02/03; T2 → S-MIF-04/05;
T3 → S-MIF-06/07/08 (+ S-MIF-13, DISTILL-added, see § "DISTILL additions");
T4 → S-MIF-09/10/11 (+ S-MIF-12, DISTILL-added).

### No `@walking_skeleton` — deliberate, and why

There is **no scenario tagged `@walking_skeleton`** in this feature, and the
absence is a decision rather than an omission. `nw-test-design-mandates`
§ "Walking Skeleton Litmus Test" item 4 requires that *"a non-technical
stakeholder can confirm 'yes, that is what users need'"*. This feature
delivers **no user-observable outcome**: ADR-0076 § Decision 4 and
architecture.md § 7 both record **no production behaviour change** — no new
operator surface, no new failure mode, no new log reason. Its "demo" is a
mutation-gate delta, which is not a user goal.

S-MIF-04 is therefore designated the feature **`@keystone`**: the single
scenario that drives the production composition (`action_shim::dispatch` with
`mtls_worker: Some(..)`, real netns provisioning, the real production guard at
`:1305-1318`) end-to-end. It carries `@keystone @driving_port @real-io`, not
`@walking_skeleton`.

### Mandate 1 (hexagonal boundary) — the T1 tension, reconciled

T1 (S-MIF-01/02/03) calls a **module-private helper directly**, which is not a
driving port. That is a deliberate, bounded departure, and it is safe only
because of the pairing below — DELIVER must not collapse it:

- **T2 (S-MIF-04/05) is the port-to-port test.** It drives the public
  `action_shim::dispatch` and proves the helper *is reached* from the
  production guard with a `Failed` row carrying
  `MtlsInterceptInstallFailed { stage, .. }`. That closes the TBU
  ("Tested But Unwired") risk Mandate 1 exists to prevent.
- **T1 is a focused sub-port test** in the sense of
  `nw-test-design-mandates` § "Focused Scenarios" — it covers the row/event
  contract breadth cheaply, at the boundary DFS-0a proves is reachable, and
  it is the test the *scoped mutation gate* (`--file
  crates/overdrive-control-plane/src/action_shim/mod.rs`) actually scores.
- **Residual, stated plainly**: assertions A-3 (for five of the six cause
  shapes), A-4, A-5, A-7, A-8, A-9 and A-10 are asserted **only** at helper
  level. T2 pins the port-level counterpart for the row-supersession and the
  `stage` string of the one `leg_f_bind` case (A-1'). Duplicating the full
  ten at Lima+root wall-clock was rejected as waste; the design pins T2's
  assertion set at exactly four (§ 5.2 / OQ-7) and DISTILL does not widen it.

---

## T1 — default-lane helper contract (the mutant killer)

> **Home**: `crates/overdrive-control-plane/src/action_shim/mod.rs`, a NEW
> sibling module `#[cfg(test)] mod fail_closed_mtls_tests` with **its own**
> module doc. The pre-existing `#[cfg(test)] mod tests` at `:1840` has a `//!`
> doc scoped to `persist_workflow_intents` and is **left untouched**
> (architecture.md § 5.1).
>
> **Lane**: default. Zero I/O — no socket, no netns, no subprocess, no
> tempdir. Runs under bare `cargo nextest run`, and therefore under
> `cargo xtask mutants` on macOS without Lima.
>
> **Reachability (DFS-0a, verified)**: the helper is an `async fn` with no
> `pub` at `:413`; the file's own test module already reaches parent items via
> `use super::{…}` at `:1869`. All eight arguments are default-lane
> constructible with no I/O — `cause` cross-crate as
> `MtlsInterceptInstallError::LegFBind(InterceptError::TransparentListener { addr, source: std::io::Error::from_raw_os_error(libc::EPERM) })`.
> **No new escape hatch, no `#[doc(hidden)]`, no new public constructor**
> (OQ-5). Enum-level `#[non_exhaustive]` blocks exhaustive *matching*, not
> *construction*.
>
> **`RecordingDriver`** (test-local, the `InertDriver` precedent at
> `tests/acceptance/finalize_failed_forward_carries_workload_addr.rs:74`) holds
> `stops: Mutex<Vec<AllocationId>>` and `releases: Mutex<Vec<AllocationId>>`,
> and additionally records `on_alloc_running` calls so the same double serves
> T2. `Driver::release_for_exit_emission` and `Driver::on_alloc_running` are
> defaulted no-ops on the trait (`overdrive-core/src/traits/driver.rs:416`,
> `:498`) — the double overrides both to record.
>
> **C2a — SUT state machine, documented in the module doc** (mandatory):
>
> ```text
>   Pending --driver.start Ok--> Running(row written, watcher parked)
>       |                            |
>       |                            +-- start_alloc Ok --> release gate, on_alloc_running
>       |                            |
>       |                            +-- start_alloc Err --> [fail_closed_on_mtls_install]
>       |                                                       stop driver (best effort)
>       |                                                       write superseding Failed row
>       |                                                       emit LifecycleEvent
>       |                                                       return WITHOUT releasing gate
>       +-- driver.start StartRejected --> Failed (handle None, gate never armed;
>                                          the mTLS guard is unreachable — it sits
>                                          inside `if state == AllocState::Running`)
> ```
>
> The `Failed` terminal state has no outgoing edge inside this helper: a second
> install failure for an already-`Failed` alloc is **structurally unreachable**
> (the guard fires at most once per dispatch), which is the C2b
> illegal-event-from-terminal-state rationale recorded in § "Self-completeness
> audit".

### S-MIF-01 — An intercept-install failure drives the allocation Failed, stops the driver, and never releases the exit gate

```gherkin
@in-memory @error_path @parametrized @security
Scenario: A failed intercept install fails the allocation closed
  Given an allocation that reached Running and had its Running row recorded
  And the intercept install for that allocation refused at a known install stage
  When the fail-closed handler is invoked with that refusal
  Then the handler reports the dispatch itself succeeded
  And the recorded state of the allocation supersedes Running with Failed
  And the Failed record names the install stage that refused and carries the refusal detail
  And the Failed record claims no per-instance address and no terminal verdict
  And the allocation's process was stopped exactly once
  And the allocation's exit watcher was never released
  And exactly one lifecycle transition was announced, from the prior state to Failed
  And the Failed record carries the allocation's identity, workload, node and kind unchanged
  And the Failed record carries the moment the allocation started running, not an absence
  And the announced transition is attributed to the reconciler
```

**The ten assertions — reproduced VERBATIM from `architecture.md` § 5.1
(OQ-7). The third column is "the regression this assertion defends", NOT a
catalogue of mutants cargo-mutants will generate** (architecture.md § 5.1
blockquote; see § "Mutation-gate traceability" below).

| # | Assertion | Regression it defends |
| --- | --- | --- |
| A-1 | Returns `Ok(())` | body → `Err(...)` |
| A-2 | The obs store holds a SUPERSEDING row for the alloc with `state == AllocState::Failed` and a strictly greater `updated_at.counter` than the seeded `Running` row | whole-body → `Ok(())`; `AllocState::Failed` → any other state |
| A-3 | That row's `reason` is `TransitionReason::MtlsInterceptInstallFailed { stage, detail }` with `stage == "leg_f_bind"` and `detail == cause.to_string()` | `reason: None`; a wrong `stage()` mapping; a swapped `stage`/`detail` |
| A-4 | That row's `workload_addr` is `None` and `terminal` is `None` | the `None,` / `None` wirings |
| A-5 | `RecordingDriver::stops` contains the alloc exactly once | deletion of the `driver.stop(handle)` call |
| A-6 | `RecordingDriver::releases` is EMPTY | insertion of a `release_for_exit_emission` call inside the helper |
| A-7 | Exactly one `LifecycleEvent` is received on the bus, with `to == AllocStateWire::Failed` and `from == prior_state` | deletion of `emit_event`; a swapped `from`/`to` |
| **A-8** | The `Failed` row's `alloc_id`, `workload_id`, `node_id`, and `kind` are byte-equal to the seeded `Running` row's | `running_row.workload_id` → `Default`; a swapped `workload_id`/`node_id`; `kind` → any other `WorkloadKind` |
| **A-9** | The `Failed` row's `started_at` is byte-equal to the `Running` row's `Some(..)` — **NOT `None`** | `running_row.started_at` → `None` (the forward-carry drop, #248's shape) |
| **A-10** | The emitted event's `source == TransitionSource::Reconciler` | `TransitionSource::Reconciler` → any other source |

> A-3's literal `stage == "leg_f_bind"` is the design's example for the
> `LegFBind` case. Under the parameterisation below the assertion reads
> "`stage` equals the string pinned for **this** case", against the closed
> four-value vocabulary — which is exactly what architecture.md § 5.1
> "Parameterisation" prescribes. Every other assertion is case-invariant.

**Parameterisation — 6 cases** (the design pins 4; cases 5–6 are a
DISTILL-added superset — see § "DISTILL additions"). All six shapes are
constructible from **existing public variants with public field types**; no
API is invented.

| Case | `cause` | Expected `stage` |
|---|---|---|
| 1 (design) | `MtlsInterceptInstallError::OutboundTproxyInstall(InterceptError::TproxyInstall { .. })` | `"outbound_tproxy_install"` |
| 2 (design) | `MtlsInterceptInstallError::LegFBind(InterceptError::TransparentListener { addr, source: io::Error::from_raw_os_error(libc::EPERM) })` | `"leg_f_bind"` |
| 3 (design) | `MtlsInterceptInstallError::Inbound(InterceptError::TransparentListener { .. })` | `"leg_c_transparent_listener"` |
| 4 (design) | `MtlsInterceptInstallError::Inbound(InterceptError::TproxyInstall { .. })` | `"inbound_tproxy"` |
| 5 (**DISTILL-added**) | `MtlsInterceptInstallError::LegFLocalAddr { source }` | `"leg_f_bind"` (alias arm) |
| 6 (**DISTILL-added**) | `MtlsInterceptInstallError::LegCLocalAddr { source }` | `"leg_c_transparent_listener"` (alias arm) |

Cases 5–6 pin the two **alias arms** of `MtlsInterceptInstallError::stage()`
(`mtls_intercept_worker.rs`), which map the `local_addr`/`getsockname`
capture failures onto the *same* stage strings as their bind siblings. Without
them the closed-vocabulary contract is asserted for 4 of the 6 constructible
shapes and a future edit that split the alias arms would go unnoticed. They
cost one table row each.

- **Universe** (port-exposed): the returned `Result<(), ShimError>`; every
  `AllocStatusRow` readable from the `SimObservationStore` for this alloc
  (`state`, `updated_at.counter`, `reason`, `detail`, `workload_addr`,
  `terminal`, `alloc_id`, `workload_id`, `node_id`, `kind`, `started_at`);
  `RecordingDriver::{stops, releases}`; every `LifecycleEvent` received on the
  `broadcast::Receiver` (`from`, `to`, `source`). Nothing private is read.
- **Why every assertion pins the specification, not the implementation**
  (research Finding 3.2 — Google's productive/unproductive discriminator):
  the specification is *"an install failure drives the alloc `Failed`, stops
  the driver, and never releases the gate"*. None of A-1…A-10 asserts on a
  private field, on an internal call count of a collaborator we own, or on a
  method-decomposition shape. The counter-archetype Google names —
  `new ArrayList(64)` → `new ArrayList(16)` — has no analogue here.
- **A-8/A-9/A-10 rest on the #248 forward-carry bug class, NOT on mutation
  coverage** (architecture.md § 5.1 blockquote / § 6.5). The helper
  forward-carries five fields out of the `Running` row into
  `build_alloc_status_row` (`:440-450`) and stamps a `TransitionSource`
  (`:457`); forward-carry drop is a *named* bug class in this repo (it is why
  `workload_addr` became a required parameter — GH #248 / dial-by-name 02-02).
- **Expected RED**: **NOT a scaffold.** See `red-classification.md`
  § "T1 is authored GREEN — its RED is a mutation litmus".

### S-MIF-02 — A driver stop that errors does not prevent the Failed row

```gherkin
@in-memory @error_path
Scenario: A workload that already vanished still fails its allocation closed
  Given an allocation that reached Running and had its Running row recorded
  And the intercept install for that allocation refused
  And the workload process has already exited, so stopping it reports it is gone
  When the fail-closed handler is invoked with that refusal
  Then the handler still reports the dispatch itself succeeded
  And the recorded state of the allocation still supersedes Running with Failed
  And the allocation's exit watcher was still never released
```

- **Universe**: the returned `Result<(), ShimError>`; the latest
  `AllocStatusRow` for the alloc (`state`, `reason`); `RecordingDriver::releases`.
- **Grounded premise** (CLAUDE.md § "Ground the premise"): this is
  production-reachable, not a test-only state. A workload that exits between
  `driver.start` returning and the intercept install completing yields
  `DriverError::NotFound` from `driver.stop`. The helper's own comment names
  it: *"Best-effort: a `NotFound` (already gone) is tolerated, mirroring the
  `RestartAllocation` stop half."*
- **The regression it defends is security-critical**: changing
  `let _ = driver.stop(handle).await;` to `driver.stop(handle).await?;` makes
  a vanished workload abort the handler **before** the `Failed` row is
  written — the alloc stays recorded `Running` with no mTLS interception
  installed, which is exactly the un-alarmed exclusion-mechanism failure
  Saltzer & Schroeder describe (architecture.md § 1, research Finding 5.2).
- **Mutation target**: cargo-mutants' `Result`-returning-call handling and any
  future `?` edit on the stop. Reported honestly: whether the tool generates a
  mutant here must be read off `cargo mutants --list`, per § 6.5.
- **Expected RED**: **NOT a scaffold** — authored GREEN; falsified by the
  `let _ =` → `?` litmus in `red-classification.md`.

### S-MIF-03 — An observation-store write rejection surfaces as an error and emits no lifecycle event

```gherkin
@in-memory @error_path
Scenario: A rejected recording of the failure is reported, not swallowed
  Given an allocation that reached Running and had its Running row recorded
  And the recorder will reject the next write
  And the intercept install for that allocation refused
  When the fail-closed handler is invoked with that refusal
  Then the handler reports the dispatch failed, naming the recorder as the cause
  And no lifecycle transition was announced
  And the allocation's exit watcher was still never released
```

- **Universe**: the returned `Result<(), ShimError>` (the `Err` variant must
  be `ShimError::Observation`, the closed contract at `:598` /
  `action_shim/mod.rs:1761`); the count of `LifecycleEvent`s received on the
  bus (must be **zero**); `RecordingDriver::releases` (must be empty).
- **Mechanism**: `SimObservationStore::inject_write_failure(...)` — existing
  infrastructure (`overdrive-sim/src/adapters/observation_store.rs:459`),
  FIFO-consumed, canonical consumer
  `tests/integration/workload_lifecycle/crash_recovery_obs_write_rejected.rs`.
  **No new test double is authored.**
- **Grounded premise**: `obs.write(...).await?` is the helper's one fallible
  step and the rejection path is real (the same store surface the exit
  observer's bounded-retry path already exercises against a real rejection).
- **The regression it defends**: the write-then-emit **ordering**. An edit
  that emits the lifecycle event before (or regardless of) the write would
  announce a `Failed` transition that no durable row backs — an observer
  divergence. C7b (interruption / partial commit) is covered here.
- **Expected RED**: **NOT a scaffold** — authored GREEN; falsified by the
  swallow-the-write litmus in `red-classification.md`.

---

## T2 — integration-lane call-site ordering (the port's ONE justification)

> **Home**: `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`
> (NEW), declared from the existing `tests/integration.rs` entrypoint inside
> the inline `mod integration { … }` block (the `tests/*.rs`-is-a-crate-root
> trick, `.claude/rules/testing.md` § "Integration vs unit gating").
>
> **Lane**: integration (`--features integration-tests`), **Lima + root**,
> `is_root()`-gated — the test **skips**, it does not fail, on an unprivileged
> host (`alloc_netns_lifecycle.rs:100-103`). This is forced by the netns seam
> (DFS-0b), not by the port: `provision_and_inject_netns` short-circuits
> **only** on `mtls_worker.is_none()` (`mod.rs:838`), so arming the mTLS seam
> unavoidably reaches `provision_workload_netns(&plan)` (`:855`) and real
> `ip netns` shell-outs. **Per DFS-8 this is a wall-clock cost, not a coverage
> cost** — `cargo xtask lima run -- cargo xtask mutants … --features integration-tests`
> runs as root by default, so these scenarios kill call-site mutants in the
> real CI gate.
>
> **Fixture — reuse `alloc_netns_lifecycle.rs`, do not re-invent** (§ 5.2):
> `is_root()` early-return; `NetnsGuard` RAII cleanup (`:168-176`) **plus** the
> explicit pre-sweep at each use site (`:371-372`, `:595-596`, `:673-674`) —
> **mandatory**, a T2 without it poisons every subsequent Lima run.
>
> **Worker construction — a TEST-LOCAL helper with this EXACT signature**
> (§ 5.2; the sibling's own `build_worker()` takes no arguments and returns
> only `Arc<MtlsInterceptWorker>`, leaving no handle to arm a fault on):
>
> ```rust
> fn build_worker(
>     intercept: Arc<dyn overdrive_worker::mtls_intercept_port::MtlsIntercept>,
> ) -> Arc<MtlsInterceptWorker>
> ```
>
> Its **body is copied verbatim from `alloc_netns_lifecycle.rs:110-118`** — the
> same `SimIdentityRead` / `SimMtlsEnforcement` / `SimMtlsResolve` / `SimClock`
> construction with their real required arguments — passing `intercept` as the
> 4th argument. Do **not** hand-write the sim constructors from memory.
>
> **Arming the fault — the exact call order** (§ 5.2; the `Arc::clone` before
> the cast is load-bearing so the test retains a typed `Arc<SimMtlsIntercept>`
> after the worker holds its erased copy):
>
> ```rust
> let intercept = Arc::new(SimMtlsIntercept::new());
> intercept.script_bind_fault(SimInterceptFault::TransparentListener {
>     errno: libc::EPERM,
> });
> let worker = build_worker(Arc::clone(&intercept) as Arc<dyn MtlsIntercept>);
> // … then dispatch with `mtls_worker: Some(&worker)`
> ```
>
> Everything else is the existing fixture: real `action_shim::dispatch`,
> `SimDriver` (so `Driver::start` succeeds without spawning a workload),
> `SimObservationStore`, a fresh `NetSlotAllocator`. The netns provision runs
> for real (root), `Driver::start` succeeds, the `Running` row commits, and
> then — and only then — the scripted install fault fires.
>
> **Driver**: the same `RecordingDriver` shape as T1, here **wrapping**
> `SimDriver` and recording `stop` / `release_for_exit_emission` /
> `on_alloc_running`, so A-6' and A-8' are observable **without adding any
> accessor to `overdrive-sim`**.
>
> **C2a module doc**: the same alloc state machine drawn under T1, plus the
> netns/slot lifecycle edge that A-9' characterises.
>
> **Both arms are two test functions and are NOT collapsed** (OQ-6). What is
> duplicated in production is a 6-line `if let … { return …; }` guard whose
> body already delegates entirely to the shared helper, and the two blocks
> close over different locals (`workload_id`/`node_id` vs `prior_row.*`).
> Extracting it would need a control-flow-signal return type — indirection for
> negative gain. Two test cases are the cheaper, more direct defense.

### S-MIF-04 — A failed intercept install on a fresh allocation never releases its exit watcher

```gherkin
@keystone @driving_port @real-io @error_path @security
Scenario: A fresh allocation whose intercept refuses is failed closed and keeps its watcher
  Given a node that provisions real network isolation for each allocation
  And an intercept surface that refuses to bind the outbound leg because the privilege is missing
  When the operator's start action for a new allocation is dispatched
  Then the allocation is first recorded Running, then superseded by Failed
  And the Failed record names the outbound-leg bind as the refusing stage
  And the allocation's exit watcher was never released
  And the allocation was never announced as running to its driver
  And the allocation's network slot is still held after the failure
```

**Assertions — the call-site properties T1 cannot reach** (verbatim from
`architecture.md` § 5.2; the design's own labels are kept so DELIVER can
trace them):

| # | Assertion | Why it needs T2 |
| --- | --- | --- |
| A-1' | The latest row for the alloc is `Failed` with `MtlsInterceptInstallFailed { stage: "leg_f_bind", .. }`, superseding a `Running` row that WAS written first | proves `start_alloc`'s `Err` actually reaches the helper through the production guard |
| A-6' | `release_for_exit_emission` was **NEVER** called for the alloc | the gate-non-release is a property of the CALL SITE's `return` placement (`mod.rs:1307` before `:1319`), not of the helper. A reordering that releases first survives T1 entirely. **This is the security-critical assertion and the one most easily omitted.** |
| A-8' | `driver.on_alloc_running` was never called for the alloc | same ordering property, second observable |
| A-9' | After the fail-closed dispatch, `net_slot_allocator.snapshot()` still contains the alloc — the slot is **retained**, released later by the terminal action's teardown seam | a *characterisation* assertion of today's behaviour: the fail-closed path returns before `teardown_and_release_netns` (`:887`), and T2 is the first test in the codebase able to observe it |

- **Universe** (port-exposed): every `AllocStatusRow` readable from the
  `SimObservationStore` for this alloc, in write order (so "Running first,
  then superseded by Failed" is observable, not inferred);
  `RecordingDriver::{releases, on_alloc_running_calls}`;
  `NetSlotAllocator::snapshot()` keyed by `AllocationId`. `snapshot()` is a
  documented public read-only observer
  (`veth_provisioner.rs`, *"A point-in-time clone for read-only observers"*)
  — not a private field.
- **A-9' is a characterisation, deliberately.** The retained netns is what the
  later `StopAllocation`/terminal arm tears down; changing that is out of
  #250's scope. Pinning it means a future change to the resource lifecycle on
  this path is a deliberate, visible edit rather than a silent one.
- **`@keystone`, not `@walking_skeleton`** — see § "No `@walking_skeleton`".
- **Mutation target**: the `StartAllocation` guard block at `:1304-1318` and
  its `return`-before-`:1319` placement. Per DFS-8 this runs inside the
  canonical CI mutation invocation.
- **Expected RED**: `MISSING_FUNCTIONALITY` — `SimMtlsIntercept`,
  `SimInterceptFault` and `mtls_intercept_port::MtlsIntercept` do not exist
  yet. Scaffolded `#[should_panic(expected = "RED scaffold")]`; GREEN in
  DELIVER step 04.

### S-MIF-05 — A failed intercept install on a restarted allocation never releases its exit watcher

```gherkin
@driving_port @real-io @error_path @security
Scenario: A restarted allocation whose intercept refuses is failed closed and keeps its watcher
  Given an allocation that was previously recorded Running
  And an intercept surface that refuses to bind the outbound leg because the privilege is missing
  When the operator's restart action for that allocation is dispatched
  Then the allocation is first recorded Running again, then superseded by Failed
  And the Failed record names the outbound-leg bind as the refusing stage
  And the allocation's exit watcher was never released
  And the allocation was never announced as running to its driver
  And the allocation's network slot is still held after the failure
```

- **Assertions**: A-1', A-6', A-8', A-9' — **identical set**, evaluated against
  the `RestartAllocation` arm (`mod.rs:1504-1518`, `return` at `:1507` before
  the release at `:1519`).
- **Fixture delta**: seeded with a prior `Running` row so
  `find_prior_alloc_row` resolves `(workload_id, node_id)` (§ 5.2).
- **Universe**: same as S-MIF-04.
- **What this case adds over S-MIF-04**: the two production blocks are
  byte-identical *today*. This case defends against a **future divergent edit
  to one block** — which OQ-6 names as "the real risk", distinct from the
  single shared-helper mutant that either arm would kill.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 04.

---

## T3 — default-lane sim-adapter contract

> **Home**: `crates/overdrive-sim/src/adapters/mtls_intercept.rs`
> `#[cfg(test)] mod tests` (the sibling precedent is `mtls_resolve.rs:168`).
>
> **Lane**: default. **Fault arms only** — an armed fault short-circuits
> **before** any syscall, so these perform ZERO I/O (architecture.md § 4.6,
> DFS-5). The `Ok` arm of `bind_transparent` binds a real plain loopback
> socket and is therefore **never** exercised here; it belongs to T4.
>
> **Sanctioned pairings only** (architecture.md § 4.6 "Out-of-contract fault
> pairings"): `bind_transparent` ⇔ `TransparentListener`, and `install_*` ⇔
> either variant (the `Inbound` arm legitimately carries both, per
> `MtlsInterceptInstallError::stage`). Arming any other pairing is a test
> defect, not a supported scenario, and **no scenario below arms one**.
>
> *Rev 4: the probe-scripting cases are gone with the probe (§ 0a).*

### S-MIF-06 — An armed intercept fault surfaces as the real error the substrate produces

```gherkin
@in-memory @error_path @parametrized
Scenario: A scripted refusal is reported in the shape the real substrate reports it
  Given a simulated intercept surface with a refusal armed on one install step
  When that install step is attempted
  Then it reports exactly the refusal the real substrate would report for that cause
  And the reported cause carries the operating-system code or failing-command description that was armed
```

**Parameterisation — the 4 sanctioned pairings:**

| Case | Method | Armed `SimInterceptFault` | Expected `InterceptError` |
|---|---|---|---|
| 1 | `bind_transparent` | `TransparentListener { errno: libc::EPERM }` | `InterceptError::TransparentListener` whose `source.raw_os_error() == Some(libc::EPERM)` |
| 2 | `install_outbound` | `TproxyInstall { reason }` | `InterceptError::TproxyInstall` carrying that exact `reason` |
| 3 | `install_inbound` | `TproxyInstall { reason }` | `InterceptError::TproxyInstall` carrying that exact `reason` |
| 4 | `install_inbound` | `TransparentListener { errno: libc::ENOPROTOOPT }` | `InterceptError::TransparentListener` whose `source.raw_os_error() == Some(libc::ENOPROTOOPT)` (the `Inbound` arm legitimately carries both) |

- **Universe** (port-exposed): the `Result` returned by the trait method — its
  `Err` variant discriminant, and for `TransparentListener` the
  `source.raw_os_error()`, for `TproxyInstall` the `reason` string. Nothing
  reads a `SimMtlsIntercept` private field; the scripting helpers are the only
  writes and the trait method is the only read.
- **Realism criterion** (research Finding 5.3, DFS-4): the scripted faults are
  expressed in the **real** error shapes the production substrate produces
  (`errno` / a failing-command `reason`), not a generic boolean "fail now".
  `libc::EPERM` is the missing-`CAP_NET_ADMIN` shape; `libc::ENOPROTOOPT` is
  the kernel-without-`IP_TRANSPARENT` shape; `libc::EADDRINUSE` is the third
  documented shape and is exercised by S-MIF-07's re-fire case.
- **C6b (each declared error triggered) and C6c (closed set)**: these four
  cases exhaust the two variants `SimInterceptFault` can materialise, which
  are exactly the two variants architecture.md § 4.2 says the sim scripts —
  `InterceptError` gains no variant and `mtls_intercept.rs` is not edited at
  all by this feature.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 03.

### S-MIF-07 — An armed intercept fault is standing: it fires on every subsequent call

```gherkin
@in-memory @error_path
Scenario: A missing privilege refuses every attempt, not just the first
  Given a simulated intercept surface with a refusal armed on the listener bind
  When the listener bind is attempted twice in succession
  Then both attempts are refused with the same cause
```

- **Universe**: the two `Result`s returned by the two consecutive
  `bind_transparent` calls — both `Err`, both `InterceptError::TransparentListener`,
  both carrying the armed `raw_os_error()`.
- **This pins DFS-4 — the STANDING (not consume-on-use) fault lifetime** — and
  it is the one place that decision is falsifiable. The divergence from
  `SimMtlsResolve`'s consume-on-use `.take()` shape is deliberate: a poisoned
  store handle is transient, whereas a missing `CAP_NET_ADMIN` or an absent
  `nft` binary fails EVERY call. Standing faults also remove call-order
  dependence — `start_alloc` calls `bind_transparent` **twice** (leg-F then
  leg-C), and a consume-on-use fault would make *"which leg failed"* an
  artifact of ordering rather than the test's explicit choice.
- **Mutation target (mandatory)**: a mutation (or a future edit) that
  `.take()`s the armed fault instead of cloning it makes the second call take
  the `Ok` arm — which, for `bind_transparent`, would additionally drag a real
  socket bind into the default lane. Killed here.
- **Two calls, not `n`**: two is the minimum that distinguishes standing from
  one-shot, and it is the exact cardinality the production caller exhibits
  (`start_alloc` binds leg-F then leg-C). A larger `n` would be
  parametrisation theatre.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 03.

### S-MIF-08 — Clearing the faults disarms all three install steps, and clearing an unarmed double is a no-op

```gherkin
@in-memory @error_path
Scenario: Clearing the refusals restores every install step, and clearing twice is harmless
  Given a simulated intercept surface with a refusal armed on each of the three install steps
  When the refusals are cleared
  Then both install steps succeed and hand back a guard
  And clearing the refusals a second time changes nothing and does not fail
```

- **Universe**: the `Result`s returned by `install_outbound` and
  `install_inbound` after `clear_faults()` (both `Ok`); the same two `Result`s
  after a second `clear_faults()` (still `Ok`).
- **Deliberate omission**: `bind_transparent` is **not** re-driven after the
  clear — its `Ok` arm binds a real plain loopback socket (DFS-5) and would
  push this scenario into the integration lane. That its bind slot is
  cleared is covered indirectly by S-MIF-13 (arming exactly one slot leaves
  the others disarmed) and directly at integration lane by S-MIF-09, which
  drives the sim's `bind_transparent` `Ok` arm with nothing armed.
- **C2b — illegal-event-from-disarmed-state**: the second `clear_faults()` on
  an already-disarmed double is the sim fault-state-machine's
  illegal-event-from-each-state case, and it is asserted to be a benign no-op
  rather than a panic or a state flip.
- **Mutation target**: a `clear_faults` that clears only one or two of the
  three slots (a copy-paste omission) — killed by the two-install assertion.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 03.

### S-MIF-13 — Arming one install step leaves the others on their success arms  *(DISTILL-added)*

```gherkin
@in-memory @orthogonality
Scenario: A refusal armed on one install step does not leak to the others
  Given a simulated intercept surface with a refusal armed on exactly one install step
  When each of the two rule-installing steps is attempted
  Then the step whose refusal was armed is refused
  And the step whose refusal was not armed succeeds and hands back a guard
```

**Parameterisation — the three I/O-free directions:**

| Case | Armed slot | `install_outbound` | `install_inbound` |
|---|---|---|---|
| 1 | `bind_fault` | `Ok(guard)` | `Ok(guard)` |
| 2 | `outbound_fault` | `Err` | `Ok(guard)` |
| 3 | `inbound_fault` | `Ok(guard)` | `Err` |

- **Universe**: the two `Result`s from `install_outbound` / `install_inbound`
  per case.
- **C5b — flag orthogonality.** `SimMtlsIntercept` holds **three independent**
  `Mutex<Option<SimInterceptFault>>` slots. An implementation that shared one
  slot across all three methods, or that had a copy-paste bug pointing two
  scripting helpers at the same field, would still pass S-MIF-06/07/08 — every
  one of those arms exactly one slot at a time and reads back the same method.
  This scenario is the only thing that separates the slots.
- **Named coverage gap (small, deliberate)**: the fourth direction — arming an
  *install* fault and confirming `bind_transparent` still takes its `Ok` arm —
  requires a real socket bind (DFS-5) and is therefore **not** default-lane.
  It is not authored. Listed in § "Self-completeness audit" under C5b.
- **Mutation target**: a shared/aliased fault slot; a `script_inbound_fault`
  that writes `outbound_fault`.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 03.

---

## T4 — integration-lane host↔sim equivalence (`Ok` arms), and its honest limit

> **Home**: `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs`
> (NEW), declared from the existing `tests/integration.rs` entrypoint inside
> its inline `mod integration { … }` block.
>
> **Lane**: integration (`--features integration-tests`), **Lima + root**.
> `HostMtlsIntercept` needs `CAP_NET_ADMIN` for `IP_TRANSPARENT` and real
> `nft`; the sim's `bind_transparent` `Ok` arm binds a real plain loopback
> socket (DFS-5). Both halves are therefore integration-lane. `is_root()`
> early-return → **skip**, do not fail.
>
> **Shape**: each scenario is parametrised over the **adapter axis**
> `{HostMtlsIntercept, SimMtlsIntercept}` — an implementation axis, not a
> generative input space (Mandate 11: example-only at layer 3+). It drives
> both through the **same** call sequence and asserts the same observable
> contract, which is the `.claude/rules/development.md`
> § "The DST equivalence test is the structural guard" obligation.
>
> **The asserted set IS the trait contract, modulo one deliberately
> unobservable clause** (§ 5.4). This is what the § 4.1 contract split bought:
> before the split the trait stated postconditions (`IP_TRANSPARENT`,
> "exactly ONE nft rule appended", "guard `Drop` removes the rule") the
> sanctioned sim adapter **could not honour**, so T4 would have had to assert a
> *weaker* set than the contract stated. Post-split, contract and assertion
> coincide.
>
> | Contract clause | Asserted by |
> |---|---|
> | `bind_transparent` returns a bound, listening listener; `local_addr()` port is NON-ZERO when `addr` carried port 0 | S-MIF-09 |
> | Each call returns a DISTINCT listener | S-MIF-10 |
> | `install_*` returns a guard owning exactly what the call acquired; `Drop` never panics | S-MIF-11 |
> | A re-install for a target already carrying an identical capture is idempotent-by-convergence, and `Drop` never panics even for a guard whose state was already released out-of-band | S-MIF-12 |
> | *"the capture is in effect **against this adapter's OWN substrate**"* | **NOT asserted — deliberately.** Unobservable through any trait accessor, so no adapter can diverge on it *observably*. Honoured and asserted **per-adapter**: for `HostMtlsIntercept` by the existing Tier-3 suite (`start_alloc_installs_both_tproxy.rs`, `bidirectional_walking_skeleton.rs`, which observe real `nft` state and real intercepted traffic); for the sim, vacuously. Listed here rather than silently dropped. |
>
> **Substrate specifics are `HostMtlsIntercept`'s own obligations, NOT T4's**
> (§ 4.1): `IP_TRANSPARENT` + `IP_FREEBIND` on the socket, the
> one-`nft`-rule-per-install realisation, removal by handle on `Drop`, the
> shared-routing-infra convergence. The **existing** Tier-3 suite asserts them.
>
> **Leak hygiene**: every acquired guard is dropped inside the test; the Host
> adapter's `Drop` removes its `nft` rule by handle. A `nft`-ruleset pre-sweep
> at the use site matches the `NetnsGuard` discipline in T2.

### S-MIF-09 — A bound intercept listener reports the concrete port the kernel assigned

```gherkin
@real-io @adapter-integration @equivalence
Scenario: An agent-chosen ephemeral leg reports a usable port, whichever intercept surface is in use
  Given an intercept surface
  When a listener is bound at the loopback address with the port left to the kernel
  Then the listener is bound and listening
  And it reports a concrete loopback address whose port is not zero
```

- **Parameterised over**: `{HostMtlsIntercept::new(), SimMtlsIntercept::new()}`
  (sim with **no** fault armed).
- **Universe**: the `Result` from `bind_transparent(127.0.0.1:0)`;
  `listener.local_addr()` — its address family (IPv4) and its port.
- **C1a — the minimum/zero input**: port `0` is the production shape for both
  legs (`start_alloc` binds `SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)` twice)
  and the only behaviourally-distinguished value in the `u16` domain.
- **Mutation target**: an adapter that passes the requested port through
  verbatim (so a port-0 request reports port 0) — which would silently corrupt
  the TPROXY redirect target, exactly the D-MTLS-18 fail-closed concern
  `LegFLocalAddr` / `LegCLocalAddr` exist for.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 05.

### S-MIF-10 — Two intercept listeners never share a port

```gherkin
@real-io @adapter-integration @equivalence
Scenario: The two intercept legs get two different ports, whichever intercept surface is in use
  Given an intercept surface
  When two listeners are bound in succession at the loopback address with the port left to the kernel
  Then the two listeners report two different ports
```

- **Parameterised over**: `{Host, Sim}`.
- **Universe**: the two `local_addr()` ports.
- **Why this matters**: `start_alloc` calls `bind_transparent` **twice** —
  leg-F then leg-C — and then installs one TPROXY rule per leg pointing at
  each leg's reported port. An adapter that cached or memoised a single
  listener would collapse both legs onto one socket and cross-wire the
  intercept. This is the `Ok`-arm contract clause *"Each call returns a
  DISTINCT listener"*.
- **Mutation target**: a cached/singleton listener; a `OnceLock`-shaped
  memoisation.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 05.

### S-MIF-11 — An install hands back a guard that releases cleanly

```gherkin
@real-io @adapter-integration @equivalence
Scenario: Both intercept installs hand back a guard that releases without incident
  Given an intercept surface and a live listener bound for each intercept leg
  When the outbound capture and the inbound capture are installed
  Then each install hands back a guard
  And releasing each guard neither fails nor panics
```

- **Parameterised over**: `{Host, Sim}`.
- **Universe**: the two `Result<Box<dyn InterceptGuard>>` values (both `Ok`);
  the absence of a panic across both `Drop`s (the test completing is the
  observable — `InterceptGuard`'s contract is *entirely* its `Drop`, so there
  is no accessor to read).
- **Fixture note**: the Host case needs a real host-side veth for
  `install_outbound`'s `iifname` and a canonical per-workload address for
  `install_inbound`'s `ip daddr` — reuse the `overdrive-testing` netns/veth
  fixtures already used by the worker's Tier-3 suite rather than hand-rolling
  `ip link add`.
- **Mutation target**: a guard `Drop` that panics or that propagates an `nft`
  removal error.
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 05.

### S-MIF-12 — Re-installing the same capture converges instead of duplicating, and both guards release cleanly  *(DISTILL-added)*

```gherkin
@real-io @adapter-integration @equivalence @idempotency
Scenario: Installing the same capture twice converges, and both guards release cleanly
  Given an intercept surface and a live listener bound for the outbound leg
  When the outbound capture for the same interface and leg is installed twice
  Then both installs hand back a guard
  And releasing both guards in turn neither fails nor panics
```

- **Parameterised over**: `{Host, Sim}`.
- **Universe**: the two `Result<Box<dyn InterceptGuard>>` values (both `Ok`);
  the absence of a panic across both `Drop`s, **including the second `Drop`,
  whose underlying state the first `Drop` already released**.
- **This is a pure contract-clause assertion, adding no new API.** It pins two
  clauses architecture.md § 4.1 states explicitly and that no other scenario
  reaches:
  1. `install_outbound`'s edge case — *"A re-install for a veth already
     carrying an identical capture is idempotent-by-convergence; it does not
     create a duplicate."*
  2. `InterceptGuard`'s invariant — *"Dropping never panics and never errors,
     including for a guard whose underlying state was already released
     out-of-band."*
- **What it does NOT assert**: *"exactly one `nft` rule exists"*. That is
  substrate, unobservable through the trait, and belongs to
  `HostMtlsIntercept`'s own Tier-3 obligations (§ 4.1). Asserting it here
  would re-introduce the § 4.1 contract defect DFS-7 fixed.
- **C4a (apply twice) + C4b (inverse op without prerequisite)**: the second
  `Drop` releasing already-released state IS the inverse-without-prerequisite
  case.
- **Mutation target**: a non-idempotent install that appends a duplicate rule
  (observable here as the second `Drop` erroring on a handle the first already
  removed, or as a `Drop` panic).
- **Expected RED**: `MISSING_FUNCTIONALITY`; GREEN in DELIVER step 05.

### The fault-arm limit, stated plainly

The **fault** arms are *not* equivalence-testable for the fault classes the
sim scripts (`EPERM` on `setsockopt`, an absent `nft` binary): the host
adapter cannot be made to exhibit them on demand — *that inability is the
entire reason this port exists* (research Conflict 2). Those arms are pinned
by the trait's rustdoc contract plus T3; the host adapter's fault arms are
exercised, unscripted, by real operational failures. The gap is *smaller*
than it looks: each `HostMtlsIntercept` method is a **one-line delegation**
with no logic of its own to diverge.

> **DISTILL observation for the reviewer (non-blocking, NOT acted on).**
> The § 5.4 sentence *"the host adapter cannot be made to fail on demand"* is
> true of the scripted fault classes but is **broader than the evidence**: a
> **non-existent interface name** passed to `install_outbound` makes real
> `nft` reject the rule, so a *limited* host fault arm IS forceable and would
> yield `InterceptError::TproxyInstall`. Adding that case would partially
> narrow the § 5.4 gap and would close checklist item **C6a** (malformed
> input). **DISTILL has deliberately NOT authored it**, because T4's scope is
> pinned to the `Ok`-arm equivalence set (§ 5.4 / OQ-8) and widening it is a
> DESIGN decision, not a DISTILL one (CLAUDE.md § "Implement to the design").
> Surfaced here so it is a visible choice rather than a silent omission.

---

## Mutation-gate traceability (OQ-9 / § 6)

### The suppression deletions (DELIVER step 01, same commit as T1)

1. `.cargo/mutants.toml` `exclude_re` entry `"fail_closed_on_mtls_install"`
   (`:592-615`, with its 24-line justification comment) — **DELETED**.
2. The source-site `// mutants: skip` comment block at
   `action_shim/mod.rs:403-412` — **DELETED**. It documents a suppression that
   will no longer exist; leaving it is an aspirational-doc violation. (It was
   never itself a suppression: a bare comment suppresses nothing.)

### Scenario → mutant mapping

| Scenario | Mutant(s) it is expected to kill | Confidence |
|---|---|---|
| **S-MIF-01** | The **one** whole-body mutant `replace fail_closed_on_mtls_install -> Result<(), ShimError> with Ok(())` — the mutant `exclude_re` currently suppresses. **Any single one of its 6 cases kills it**; the other 5 defend the specification. | **HIGH** — this is the recorded MISSED mutant from the dial-by-name 02-02 review. |
| S-MIF-01 (cases 1–6), S-MIF-02, S-MIF-03 | **Every other mutant cargo-mutants generates inside the function**, whatever that set turns out to be. The `exclude_re` entry is a bare **function-name anchor** — its own comment says *"the whole helper is uncovered, not just the whole-body mutant"* — so deleting it un-suppresses all of them, while `cargo xtask mutants` scores **kill rate across the diff window**. The contract is **100 % of the function's mutants**, not ≥ 80 %. | **MEDIUM** — set unknown until enumerated; see the mandatory `--list` step below. |
| S-MIF-04 | Mutants in the `StartAllocation` guard block (`mod.rs:1304-1318`) and any reordering that moves `release_for_exit_emission` (`:1319`) ahead of the fail-closed `return` (`:1307`). | MEDIUM |
| S-MIF-05 | The same in the `RestartAllocation` block (`:1504-1518`, `:1507` before `:1519`). | MEDIUM |
| S-MIF-06/07/08/13 | Mutants in `SimMtlsIntercept`'s three trait-method bodies and the four scripting helpers (fault→error materialisation, `.take()`-vs-clone, partial `clear_faults`, aliased slot). | HIGH — small pure functions, mutation-friendly. |
| S-MIF-09/10/11/12 | Mutants in `HostMtlsIntercept`'s three one-line delegations and `SimMtlsIntercept`'s `Ok` arms. | LOW — these are I/O shims reachable only from Tier-3; see the `exclude_re` guidance below. |

### Reconciling "two call-site arms" with OQ-6 — there is exactly ONE helper mutant

The dispatch framing *"both call-site arms = two mutants"* needs correcting,
and OQ-6 already states the correction:

- `fail_closed_on_mtls_install` is **ONE function**, so the `exclude_re`
  deletion un-suppresses **one** whole-body `-> Ok(())` mutant, not two.
  cargo-mutants mutates a function **definition**, never a call site.
- The two `StartAllocation` / `RestartAllocation` blocks are two **invocation
  sites**, not two helper mutants. OQ-6: *"the suppressed mutant is **one**
  whole-body mutant on the shared helper, killable from either arm; the second
  arm defends against a future divergent edit to one block, which is the real
  risk."*
- Mutants generated for the *enclosing* `dispatch_single` are a different and
  much larger target, governed by the ordinary diff-scoped ≥ 80 % gate — not
  by this feature's 100 %-of-the-helper contract.
- **`MtlsInterceptInstallError::stage()` lives in `overdrive-worker`**, outside
  the scoped `--file crates/overdrive-control-plane/src/action_shim/mod.rs`
  run. S-MIF-01's six-case stage parameterisation therefore defends the
  **specification** (the closed four-value vocabulary), **not** that scoped
  gate. Stated so no one reads the 6 cases as a mutation claim.

### Mandatory: enumerate the ACTUAL mutant set BEFORE claiming 100 % (§ 6.5)

Before asserting the contract is met, run `cargo mutants --list` scoped to
`crates/overdrive-control-plane/src/action_shim/mod.rs` and **record the
mutants generated inside `fail_closed_on_mtls_install` on the DELIVER step**.

cargo-mutants does **not** insert statements and does **not** substitute call
arguments, and this helper contains **no binary operators** — so the generated
set may be **just the whole-body mutant**, in which case a 100 % function-scoped
kill rate is **vacuous and must be reported as such**. This repo has a recorded
instance of exactly that trap (a file-scoped gate reading 100 % while
generating zero mutants for the load-bearing arm). **Assertions A-8/A-9/A-10
therefore rest on the #248 forward-carry bug class, which stands regardless of
what the tool generates — NOT on mutation coverage.**

### The scoped re-run

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/action_shim/mod.rs
```

Read the **guest** `target/xtask/mutants-summary.json`, **not** the stale host
artifact (macOS Lima writes the summary into the guest target dir).

### T1 alone must suffice — and why

Because of the **DELIVER ordering** (DFS-6 / § 9): T1 and both suppression
deletions land as **step 01**, *before* the port exists and therefore before
T2 exists, so on that commit T1 is the only test defending the function.

It is **NOT** because a Lima-gated test fails to count toward the gate — per
DFS-8 it does count (`cargo xtask lima run` runs as root by default, and the
Lima prefix is *mandatory* for any mutation run carrying
`--features integration-tests` precisely because otherwise the
`#[cfg(target_os = "linux")]` surface is unreachable and *"the kill-rate gate
becomes meaningless"*). **Default-lane placement is a wall-clock property, not
a coverage one.**

If the crafter finds any mutant in the function survives with only T1, that is
a **design defect to surface as a blocker** — not a licence to re-add the
suppression, and not a licence to lean on T2 to cover step 01.

### No pre-emptive new `exclude_re` entry

None is added for any symbol this design introduces (§ 6.7).
`HostMtlsIntercept`'s three one-line delegations are I/O shims reachable only
from Tier-3. **If — and only if — the diff-scoped gate actually reports them
missed**, add one entry with the standard justification comment naming **T4
(S-MIF-09..12)** as the exercising test, matching the sibling
`sweep_one_chain` / `list_named_chain` entries.

---

## Adapter coverage table (Mandate 6)

Every driven adapter the feature adds or exercises → at least one `@real-io`
scenario. **No empty rows.**

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `MtlsIntercept` → `HostMtlsIntercept` (real `libc::socket`/`setsockopt(IP_TRANSPARENT, IP_FREEBIND)`, real `nft`/`ip`) — **NEW** | **YES** | S-MIF-09/10/11/12 (Lima + root, real syscalls + real `nft`). Substrate specifics (the setopts, exactly-one-rule, removal-by-handle) remain asserted by the **existing** Tier-3 suite `start_alloc_installs_both_tproxy.rs` / `bidirectional_walking_skeleton.rs`, unchanged by this feature. |
| `MtlsIntercept` → `SimMtlsIntercept` (fault-scripted double) — **NEW** | **YES** (its `Ok` arm binds a real plain loopback socket, DFS-5) | S-MIF-09/10/11/12 (`Ok` arms, real bind); S-MIF-06/07/08/13 (fault arms, default lane, zero I/O) |
| `ObservationStore` → `SimObservationStore` (row supersession + injected write rejection) | **YES** (through the production dispatch path) | S-MIF-04/05 (real `action_shim::dispatch` writes the `Running` then the `Failed` row). Focused in-process: S-MIF-01/02/03 (**not** `@real-io`). |
| `Driver` → `SimDriver` wrapped by `RecordingDriver` (`stop` / `release_for_exit_emission` / `on_alloc_running`) | **YES** (through the production dispatch path) | S-MIF-04/05. Focused in-process: S-MIF-01/02 (**not** `@real-io`). |
| `veth_provisioner::provision_workload_netns` / `teardown_workload_netns` + `NetSlotAllocator` (real `ip netns` / veth) — **reused, unchanged** | **YES** (reused) | S-MIF-04/05 run it for real under root; A-9' observes the retained slot via `NetSlotAllocator::snapshot()` |
| `MtlsEnforcement` / `MtlsResolve` / `Clock` (the worker's other three ports) — **reused, unchanged** | n/a — no new surface; wired as the existing `alloc_netns_lifecycle.rs` fixture wires them | S-MIF-04/05 (`SimMtlsEnforcement` / `SimMtlsResolve` / `SimClock`) |

`MtlsInterceptInstallError::stage()` and `InterceptGuard` are **not** adapters
(no I/O, no port trait boundary of their own) — they are the Tier-1 seams
S-MIF-01 and S-MIF-11/12 pin.

## Driving-adapter verification (Mandate / RCA-P1)

| Driving entry point named in DESIGN | Real-protocol scenario |
|---|---|
| `action_shim::dispatch` → `Action::StartAllocation` (`mod.rs:1304-1318`) | **S-MIF-04** — real `dispatch`, real netns, `mtls_worker: Some(..)` |
| `action_shim::dispatch` → `Action::RestartAllocation` (`mod.rs:1504-1518`) | **S-MIF-05** — same |
| `run_server` (composition root, § 4.4 — **wire only, no gate**) | **No new scenario, deliberately.** § 4.4 adds *only* the construction of `HostMtlsIntercept` and its passage as the worker's 4th argument; the probe gate is struck (§ 8.2), so `run_server` gains **no gate, no `tracing::warn!`, no early return, and no new failure mode**. There is no new boot behaviour to assert. The wiring itself is **compiler-enforced** (a mandatory 4th `new()` parameter — a call site that "forgets" the port fails to compile, § 7 "Enforcement tooling"), and it is exercised at runtime by every existing Tier-3 test that boots `run_server` with `compose_mtls` true. |
| `overdrive serve` / `overdrive deploy` (CLI verbs) | **Not exercised, and correctly so.** The feature is a test seam; ADR-0076 § Decision 4 records **no production behaviour change**, so there is no operator-observable delta for a CLI scenario to assert. Asserting one would be a change-detector over unchanged behaviour. |

**No uncovered DESIGN entry point.** The helper-level scenarios
(S-MIF-01/02/03) do **not** substitute for S-MIF-04/05 — see § "Mandate 1 …
reconciled".

---

## DISTILL additions beyond the design's pinned four families

Three additions. **None invents API surface** — each uses only symbols
`architecture.md` § 4.1 / § 4.6 pins verbatim, or existing public variants
with public field types. Each is listed so a reviewer can strike it
individually without touching the design-pinned set.

| Addition | What it adds | Symbols used | Why |
|---|---|---|---|
| **S-MIF-01 cases 5–6** | Two extra `cause` parameterisation rows (`LegFLocalAddr`, `LegCLocalAddr`) | existing public variants of `MtlsInterceptInstallError`, both carrying a public `std::io::Error` | Pins the two **alias arms** of `stage()`. The design's four cases cover all four *stage strings* but only 4 of the 6 *constructible shapes*; an edit splitting the alias arms would go unnoticed. Cost: two table rows. |
| **S-MIF-12** | Idempotent re-install + double guard-`Drop` | `install_outbound`, `InterceptGuard` — both § 4.1 verbatim | Pins two contract clauses § 4.1 states explicitly and that no design-pinned scenario reaches: install idempotency-by-convergence, and *"Dropping never panics … including for a guard whose underlying state was already released out-of-band."* Closes checklist **C4a + C4b**. |
| **S-MIF-13** | Fault-slot orthogonality across the three sim slots | `script_bind_fault` / `script_outbound_fault` / `script_inbound_fault`, `install_outbound`, `install_inbound` — all § 4.6 verbatim | `SimMtlsIntercept` holds **three independent** `Mutex<Option<SimInterceptFault>>` slots; a shared/aliased slot or a copy-paste bug in a scripting helper passes S-MIF-06/07/08 unnoticed. Closes checklist **C5b**. Default lane, zero I/O. |

**S-MIF-02 and S-MIF-03 are NOT additions in this sense** — they are within
the design's T1 family (§ 5.1 "construct the eight arguments directly and call
`fail_closed_on_mtls_install(...)`"), varying two of the eight arguments the
design's own D-1 table enumerates (`driver`, `obs`). Both defend
production-reachable states (see each scenario's *Grounded premise* note).

### Deliberately NOT authored, with the grounding rationale

| Candidate | Why not |
|---|---|
| `handle: None` passed to `fail_closed_on_mtls_install` | **NOT production-reachable.** The mTLS guard sits inside `if state == AllocState::Running { … }` (`mod.rs:1279`), and `state == Running` ⟺ `handle_opt == Some` (both are set from the same `driver.start` `Ok` arm, `:1200-1207`). A test asserting behaviour for `handle: None` would defend a state only a test can produce — precisely the GH #248 / ADR-0074 anti-pattern CLAUDE.md § "Ground the premise" exists to prevent. Recorded as a checklist rationale, not a scenario. *(Adjacent observation, non-blocking: the `Option<&AllocationHandle>` parameter is therefore wider than production needs. Narrowing it is a production signature change and is **out of #250's scope** — the design edits this helper not at all. No deferral language, no issue.)* |
| A host-adapter **fault** case (`install_outbound` with a non-existent interface name) | Would widen T4 beyond its design-pinned `Ok`-arm equivalence scope (§ 5.4 / OQ-8). Surfaced as a reviewer observation under T4 instead. |
| The fourth orthogonality direction (arm an install fault → `bind_transparent` still `Ok`) | Requires a real socket bind (DFS-5) ⇒ not default-lane. Named as a gap under S-MIF-13 and in the audit. |
| A CLI / `overdrive serve` boot scenario | No production behaviour change to assert (§ "Driving-adapter verification"). |

---

## Verification-catalogue graduation — NONE (re-checked, not inherited)

`.claude/rules/verification.md` graduates **operator-surface (`O`) and
end-to-end (`E`)** scenarios, and qualitative expectations no `assert!` holds,
into `verification/expectations/`.

architecture.md § 9 records **none**, on the grounds that the only
operator-surface expectation the design produced was the boot-probe refusal,
struck at § 8.2. DISTILL **re-ran the check against the authored set rather
than inheriting the conclusion**, and confirms it:

| Scenario | Surface | Graduates? |
|---|---|---|
| S-MIF-01/02/03 | in-process helper contract | No — in-process logic; Tier 1 owns it |
| S-MIF-04/05 | `action_shim::dispatch` — an **internal** driving port, not an operator surface. No CLI output, no exit code, no log line, no file, no rendered artifact changes. | No |
| S-MIF-06/07/08/13 | sim-adapter contract | No — a test double's own contract is not an operator surface |
| S-MIF-09/10/11/12 | adapter-contract equivalence | No — adapter-contract behaviour the four tiers already own |

**No `O`- or `E`-surface expectation graduates from this feature.** The rule
explicitly warns that duplicating tier-owned in-process logic into the
catalogue dilutes its signal. The decisive fact is the same one § 8.2 rests
on: **this feature makes no production behaviour change**, so there is nothing
operator-observable to expect.

---

## Mandate-12 (SSOT via types + services) — Rust mapping

The Python-pilot machinery (`domain_types.py`, `parsers.parse` step
decorators, the step-reuse ratio) has **no Rust analogue** and is not
introduced (policy file § Polyglot note). The four mechanical criteria map as:

| Criterion | Rust mapping | Status |
|---|---|---|
| 1 — domain types module exists | The type system **is** the module: `SimInterceptFault` (typed fault descriptor, § 4.6), `InterceptError`, `MtlsInterceptInstallError`, `InterceptGuard`, `AllocationId`, `NetSlot`, `AllocState`, `TransitionReason`, `TransitionSource`, `WorkloadKind` — all pre-existing or design-pinned production types, **not** a test-only shadow module. | **MET** |
| 2 — composition methods consume typed parameters | `script_bind_fault(SimInterceptFault)` / `script_outbound_fault(…)` / `script_inbound_fault(…)` take the typed descriptor, not a `bool` or a `&str`; `bind_transparent(SocketAddrV4)`, `install_inbound(SocketAddrV4, u16)`. **One raw `&str` remains** — `install_outbound(host_veth: &str, …)`, pinned verbatim by § 4.1. Not changed (never invent API surface); recorded as an observation. | **MET, one pinned exception** |
| 3 — no business logic in step bodies | The Rust analogue: each scenario constructs its fixture, invokes **one** unit under test, and asserts through port observables. No scenario re-implements the helper's row-building, the `stage()` mapping, or the guard's `Drop`. | **MET by construction** |
| 4 — step-reuse ratio (informational) | **N/A** — there are no Gherkin step decorators in Rust; the Gherkin above is specification-only prose. The reuse analogue is the shared `RecordingDriver` shape (T1 + T2) and the shared `build_worker(intercept)` helper. No ratio is computed; none is a gate. | **N/A, documented** |

---

## Self-completeness audit (`nw-at-completeness-check`, 15-item mechanical checklist)

**Domain extensions**: none. No
`docs/feature/mtls-intercept-install-fault-seam/distill/at-completeness-extensions.yaml`
is authored — the `nwave-installer` overlay (IP/privacy + filesystem shape) has
no bearing on a kernel-intercept fault seam. Checklist stays at 15 items.

| Item | Verdict | Evidence / rationale |
|---|---|---|
| **C1a** — ≥1 AT exercises empty/zero/minimum-size input | **PASS** | S-MIF-09 binds `127.0.0.1:`**`0`** — the zero/minimum value of the port domain, and the production shape for both legs. |
| **C1b** — ≥1 AT on each partition boundary (max-1, max, max+1) | **PASS (N/A, documented)** | The port's numeric domain is a `u16` port. It has **no behaviourally-distinct maximum**: the kernel treats 65535 exactly as 8080. Its only distinguished value is `0` (kernel-assign), covered by C1a and asserted non-zero on return. There is no partition boundary to sit on. |
| **C2a** — SUT state machine documented in AT module docstring | **PASS** | Mandated in the T1 and T2 module docs (drawn under T1: `Pending → Running → [fail-closed] → Failed`, plus the `StartRejected` branch that never reaches the guard). |
| **C2b** — for each state, ≥1 AT for illegal-event-from-that-state | **PASS** | Sim fault machine: `clear_faults` from the **disarmed** state is asserted a benign no-op (S-MIF-08). Alloc machine: the `Failed` terminal has no outgoing edge inside the helper — a second install failure for an already-`Failed` alloc is **structurally unreachable** (the guard fires at most once per dispatch, inside `if state == Running`). Documented, not skipped. |
| **C3** — parametrize/PBT over n ∈ {0, 1, many} for each collection input | **PASS (N/A, documented)** | Neither the helper nor any `MtlsIntercept` method takes a collection-shaped input. The one N-vs-0 cardinality in the neighbourhood — *N* inbound captures for *N* declared Service ports, **zero** for a Job-kind workload — is explicitly **the CALLER's decision, not this method's** (§ 4.1), lives in `start_alloc`, is unchanged by this feature, and is covered by the existing Tier-3 `start_alloc_installs_both_tproxy.rs`. |
| **C4a** — each mutating op has an "apply twice" AT | **PASS** | S-MIF-12 installs the same capture twice (idempotency-by-convergence); S-MIF-07 fires the same armed fault twice (correct **non**-idempotency of a standing fault — it does not decay). |
| **C4b** — ≥1 AT for inverse op without prerequisite | **PASS** | S-MIF-12's **second** guard `Drop` releases state the first `Drop` already released — the release-without-prerequisite case the `InterceptGuard` contract names verbatim. |
| **C5a** — each mode flag: every materially-distinct combination exercised | **PASS** | Three flag axes, each exhausted: the `stage()` cause vocabulary (S-MIF-01, **6/6** constructible shapes → 4/4 stage strings); the two dispatch arms (S-MIF-04 + S-MIF-05, 2/2); the three sim fault slots × two fault descriptors (S-MIF-06, 4 sanctioned pairings — the unsanctioned pairings are a documented test defect, § 4.6, and are deliberately not exercised). |
| **C5b** — ≥1 AT asserting flag orthogonality | **PASS (partial, gap named)** | S-MIF-13 asserts three of the four directions (arm-bind → both installs `Ok`; arm-outbound → inbound `Ok`; arm-inbound → outbound `Ok`). **Gap**: arm-an-install → `bind_transparent` still `Ok` needs a real socket bind (DFS-5) ⇒ not default-lane; not authored. |
| **C6a** — each input param: ≥1 AT with a malformed value | **FAIL** | **No scenario passes a malformed value to a port method.** The reachable candidate — a non-existent interface name to `install_outbound`, which real `nft` rejects — would widen T4 beyond its design-pinned `Ok`-arm scope (§ 5.4 / OQ-8). Deliberately not authored; surfaced as a reviewer observation under T4. See § "Gap register" below. |
| **C6b** — each declared error in contract: ≥1 AT triggers exactly that error | **PASS** | `InterceptError::TransparentListener` + `InterceptError::TproxyInstall` → S-MIF-06 (both, by exact variant + payload). All 5 `MtlsInterceptInstallError` variants → S-MIF-01's 6 cases. `ShimError::Observation` → S-MIF-03. |
| **C6c** — ≥1 AT asserts a closed error set | **PASS** | S-MIF-01 asserts `stage` ∈ the **closed four-value vocabulary** across all six constructible cause shapes — no fifth string can escape. |
| **C7a** — ≥1 AT under a degraded-resource condition | **PASS** | Missing `CAP_NET_ADMIN` (`libc::EPERM`) is the modelled degraded condition, driven through the whole production dispatch in S-MIF-04/05 and through the adapter in S-MIF-06; kernel-without-`IP_TRANSPARENT` (`ENOPROTOOPT`) and address-in-use (`EADDRINUSE`) are the other two documented shapes. |
| **C7b** — ≥1 AT for interruption mid-operation (partial commit) | **PASS** | The fail-closed path **is** an interruption mid-operation: `driver.start` succeeded, the `Running` row committed, then the install refused. S-MIF-04/05 assert the supersession (A-1') **and** the retained net slot (A-9' — the partial-commit characterisation). S-MIF-03 covers the helper's own write rejecting mid-handler. |
| **C7c** — if concurrent-safe by claim: ≥1 multi-actor AT | **PASS (N/A, documented)** | Neither adapter carries a concurrency-sensitive invariant: `HostMtlsIntercept` is a stateless unit struct; `SimMtlsIntercept` holds three **independent** `parking_lot::Mutex<Option<…>>` slots with no cross-slot invariant and no check-and-act across them. The per-alloc concurrency of `start_alloc` is **unchanged** by this feature, and `.claude/rules/testing.md` places concurrency invariants at Tier 1 DST, to which this feature adds nothing. |

### Verdict — **COMPLETE** (14 / 15 passing; threshold ≥ 13)

Computed mechanically, not judged. **Read with the caveat that four items pass
on documented-N/A or partial grounds** (C1b, C3, C7c — genuinely inapplicable;
C5b — partial with a named gap), so a reviewer re-grading any of them to FAIL
lands at 13/15 (still COMPLETE) or 12/15 (ACCEPTABLE_WITH_DOCUMENTED_GAPS).
The transparency is the point.

### Gap register

| Gap | Category | Kind | Severity | Disposition |
|---|---|---|---|---|
| No malformed-input AT on any `MtlsIntercept` method | C6a | `AT_GAP_IN_DELIVERY_SCOPE` | **MEDIUM** | **Deliberately not filled.** Filling it means widening T4's design-pinned `Ok`-arm equivalence scope (§ 5.4 / OQ-8) — a DESIGN decision, not a DISTILL one. Surfaced to the reviewer under T4 with the concrete candidate (`install_outbound("no-such-veth", port)` against the Host adapter). |
| Fourth orthogonality direction (arm-install → `bind_transparent` still `Ok`) | C5b | `AT_GAP_IN_DELIVERY_SCOPE` | **LOW** | Not filled: needs a real socket bind (DFS-5) ⇒ integration-lane cost for a low-value direction; the three cheap directions already separate the slots. |
| Host↔sim equivalence on the **fault** arms | (outside the 15-item set) | design-recorded limit | **LOW** | Not fillable — the host adapter cannot be made to exhibit the scripted fault classes on demand; *that inability is the reason the port exists* (§ 5.4, research Conflict 2). Mitigated: each host method is a one-line delegation with no logic to diverge. |

**Zero `SPECIFICATION_AMBIGUITY` findings.** Categories C2 (state machine),
C5 (mode-flag inventory), C6 (error contract) and C7 (env / interruption
matrix) each have their upstream artifact present and sufficient:

- **C2** — the alloc state machine is specified by the production source and
  by architecture.md § 1's call chain; no DISCUSS re-entry needed.
- **C5** — the flag inventory is enumerated in `architecture.md` § 4.6
  (`SimInterceptFault` variants, the three slots, the sanctioned pairings) and
  in `mtls_intercept_worker.rs::stage()`'s closed vocabulary.
- **C6** — the typed error set per port method is pinned per-method in
  § 4.1's `# Edge cases` rustdoc blocks.
- **C7** — the environment matrix is the project's four-tier model
  (`.claude/rules/testing.md`), which supersedes a feature-local `devops/`
  directory here; the degraded-resource conditions are enumerated in
  `architecture.md` § 1's named-fault table.

**No upstream routing is emitted.** No `CLARIFICATION_NEEDED` is returned.

### Completeness audit log (falsifier-gate telemetry, plan v3 § 6.7)

```
(mtls-intercept-install-fault-seam, C1, 0, —)
(mtls-intercept-install-fault-seam, C2, 0, —)
(mtls-intercept-install-fault-seam, C3, 0, —)
(mtls-intercept-install-fault-seam, C4, 0, —)
(mtls-intercept-install-fault-seam, C5, 1, LOW)
(mtls-intercept-install-fault-seam, C6, 1, MEDIUM)
(mtls-intercept-install-fault-seam, C7, 0, —)
```

---

## DISTILL wave-decisions (recorded inline — no separate file)

| # | Decision | Rationale |
|---|---|---|
| **Q-1** | **13 scenarios across the design's four families**, no fifth family. | The design pins T1–T4; DISTILL splits them into one-behaviour-per-scenario units and adds three items (§ "DISTILL additions") that use only pinned symbols. |
| **Q-2** | **No `@walking_skeleton`.** S-MIF-04 is the `@keystone`. | The feature has **no user-observable outcome** (ADR-0076 § Decision 4), so the litmus test's item 4 cannot be satisfied by any scenario. Tagging one anyway would be dishonest. |
| **Q-3** | **T1's Mandate-1 departure is bounded by T2**, not waived. | T2 is the port-to-port test that closes the TBU risk; T1 is a focused sub-port test whose reachability DFS-0a verifies and which the *scoped mutation gate* is what actually scores. Residual assertions asserted only at helper level are enumerated, not hidden. |
| **Q-4** | **No `proptest` / generative PBT anywhere.** Parametrisation only, over closed enumerations. | Every argument space here is a closed set of ≤ 6 elements; `.claude/rules/testing.md` puts the proptest trigger at *"exceeds a dozen hand-picked cases"*. Manufacturing a generator would be parametrisation theatre (Mandate 9 / `nw-test-optimization` paradigm-match). |
| **Q-5** | **T1 is authored GREEN, not RED-scaffolded.** | Its production code already exists and already works (DFS-0a). Its falsification is a **mutation litmus**, specified in `red-classification.md`. Attaching `#[should_panic(expected = "RED scaffold")]` to a test whose behaviour is already implemented would be a false RED. |
| **Q-6** | **Verification-catalogue graduation: NONE** — re-checked against the authored set, not inherited. | No `O`- or `E`-surface scenario exists, because the feature changes no operator-observable behaviour. |
| **Q-7** | **The `Option<&AllocationHandle>` width and the `install_outbound(&str)` raw-string parameter are recorded as observations, not changed.** | Both are design-pinned or out of #250's scope. CLAUDE.md § "Implement to the design"; no deferral language, no issue number, no forward pointer. |
| **Q-8** | **Four `atdd-infrastructure-policy.md` rows appended** (policy `inherit`, accretion). | The `MtlsIntercept` port and its two adapters were absent from the policy; the skill requires appending the missing rows before generating scenarios. |
