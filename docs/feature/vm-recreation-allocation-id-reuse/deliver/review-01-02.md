# DELIVER Review — Driver-neutral allocation replacement, step 01-02

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-02` — Complete successor-first exact-ID ownership |
| Implementation commit | `c830b55453b529ef40bbb44a170e823639490ed6` |
| Fixture commit | `699570332a5361469a6230e4be6cd998bdcdcd85` |
| Acceptance reconciliation | `72cb4fea03ac778d671a9904585e73bf162cc183` |
| Review remediation | `b295d973de2167124f8cf6be0bea64660cbac078` |
| Reviewer | Fresh isolated DELIVER reviewer, GPT-5.6 Luna/max |
| Review date | 2026-09-13 |
| Iteration | 2 |
| Final verdict | **APPROVED** |

## Authority and scope

This review covers the step-01-02 implementation and the qualified-metal
fixture adjustment against the user-ratified P-105-1 through P-105-7 contract
in the canonical [feature delta](../feature-delta.md), ADR-0105, ADR-0106,
ADR-0108, ADR-0109, and the approved two-step roadmap. The exact action shape
is unchanged: `RestartAllocation.alloc_id` is the accepted predecessor and
`RestartAllocation.spec.alloc` is one distinct successor.

The review also audits the 18 newly activated pending bodies, the active
S-284-SIM-P8 preservation control, all affected Sim/integration/acceptance
surfaces, DES traces, attribution, and the qualified-metal/strace boundary.
No production or test source was edited by this reviewer. The pre-existing
dirty `AGENTS.md`, execution log, and untracked step-01-01 review artifact were
preserved.

## Executive verdict

The focused driver-neutral Sim and Service evidence is green, the active
control-plane suites now exercise the fresh-successor contract, and the
retained qualified-metal receipt records all six native tests passing with the
required raw strace capture. The changed action-shim path follows the ratified
successor-first ordering. The reconciliation and remediation closed the prior
same-ID specification drift and live-comment/evidence findings without adding
compatibility behavior, public API, or a new mechanism.

## Design-contract checklist

| Contract | Result | Evidence |
|---|---|---|
| Predecessor/successor identity relation | PASS | `action_shim/mod.rs:2323-2349` reads/fences `alloc_id`, captures predecessor facts, and keeps `successor_alloc_id = spec.alloc`; the five focused driver-neutral tests assert distinct exact IDs for Exec and VM. |
| Terminal handoff | PASS in the changed shim | The shim returns without effects unless the fenced predecessor is `Failed` or `Terminated` (`action_shim/mod.rs:2328-2335`); `Draining` cannot enter the effect path. The stale callers identified in F-01 still seed `Running` rows. |
| Successor-first completion | PASS in focused Sim evidence | Provision, successor identity, driver start, publication/unwind, accepted-Running index/hooks and successor mTLS all complete before `finish_restart` begins exact-old cleanup (`action_shim/mod.rs:2351-2762`). SIM-01 and SIM-02 observe this with blocked cleanup and exact IDs. |
| Durable reservation before effects | PASS in the production runtime path | `run_convergence_tick_inner` persists the returned View before dispatch (`reconciler_runtime.rs:1485-1500`). SIM-05 uses registered `WorkloadLifecycle`, `RecordingRedbViewStore`, close/reopen, and `persistence_at_start` to prove the reservation survives and is seen before each successor start. |
| Publication and history | PASS in focused Sim evidence | Fresh Running/Failed rows are built with `prior = None` (`action_shim/mod.rs:2566-2585`); SIM-03, SIM-04 and SIM-05 preserve the predecessor row/history, require successor zero/`None` history, and reject a second in-shim proposal. |
| Result precedence and cleanup short-circuiting | PASS in focused Sim evidence | `finish_restart` implements all four successor/cleanup result cells (`action_shim/mod.rs:1489-1528`); the 16-cell SIM-02 matrix observes driver → mTLS → network short-circuiting and successor-primary both-fail tracing. |
| Service, late-exit and stream schedules | PASS for the mapped Sim bodies | SIM-P1/P2/P3, SIM-P4/P5 and SIM-P6/P7 pass through the registered Service, exit-observer and stream compositions. SIM-P8 is the active policy preservation control and passes separately. |
| Driver neutrality and public API shape | PASS | The production diff retains `Action::RestartAllocation { alloc_id, spec, kind }`; driver matching in the replacement path only projects the supplied payload. No public method, type, variant, field, port, store, schema or driver-policy surface was added. |
| Qualified-metal host effects | PASS | `evidence-01-02-metal.md` retains the native non-virtualized x86_64 KVM six-test receipt: exit 0, 6/6 passed, and the exact retained raw strace path for the predecessor-artifact test. |
| Mutation testing | PASS — not run | Individual-step and final mutation runs are explicitly excluded by the user/roadmap; no exclusions were edited. |

The only production source expansion beyond the roadmap's named shim file is
the nine-line `ServiceLifecycle` policy adjustment in
`overdrive-reconcilers/src/service_lifecycle.rs`. It is tightly related to
the accepted predecessor-scoped terminal veto/fresh-successor policy and adds
no boundary or public surface. Its mapped Service tests pass.

## Activated scenario and Contract Shape audit

The step removes exactly 18 pending markers. The active S-284-SIM-P8
preservation body remains unmarked, as required. Thus 19 step-related bodies
are executed when the active preservation control is included:

| Scenario group | Bodies | Result |
|---|---:|---|
| S-284-SIM-01..05 | 5 | 5/5 pass |
| S-284-SIM-P1..P3 | 3 | 3/3 pass |
| S-284-SIM-P4..P5 | 2 | 2/2 pass |
| S-284-SIM-P6..P7 | 2 | 2/2 pass |
| S-284-SIM-P8 active preservation control | 1 | 1/1 pass |
| S-284-METAL-P1/P2 | 6 | 6/6 pass in retained native receipt; raw strace retained |

All 18 newly activated bodies and the active P8 body carry their exact
`/// CONTRACT_SHAPE: bounded-change.` or
`/// CONTRACT_SHAPE: unbounded-preservation.` declaration. No activated body
retains a pending `#[ignore]` marker. The mapped bodies enter through the
public reconciler/runtime or action-shim driving surfaces, use Sim/host
adapters only at driven boundaries, and do not spawn the built Overdrive
binary or emit expectation evidence.

The step has 19 observable mapped bodies including P8, giving a conservative
test budget of `19 × 2 = 38`; 19 bodies are within budget. The matrix cells in
SIM-02 are variations of the one mapped precedence behavior, not additional
test methods. No zero-assertion, tautological, fully mocked-SUT, circular
oracle, or implementation-only test was found. The P8 body is intentionally a
ServiceLifecycle policy control, not a replacement for the registered
production-owner SIM-05 path. The six qualified-metal bodies were executed by
the retained native receipt rather than silently converted to host-conditional
or Sim-only evidence.

## Fixture and test-integrity audit

Commit `c830b554` removes only the 18 authorized pending markers, changes the
replacement action-shim control flow, and makes the bounded Service policy
adjustment. The test bodies, seeds, assertions, declared Contract Shapes and
Sim fixtures remain intact. Commit `69957033` makes one qualified-metal
fixture correction:

- the existing 25 ms logical-tick delay is changed to one second so the
  production `ProbeRunner`'s separate `SystemClock` can author its existing
  one-second probe interval; and
- predecessor artifact presence is asserted at its valid `Running` boundary,
  before the terminal transition may close its vsock, while final exact-family
  and PID absence assertions remain in place.

The before/after diff shows no weakened expected value, removed behavioral
assertion, altered seed, fabricated lifecycle row, or new production seam.
The correction is bounded by the existing ten logical-second loop and
30-second reclamation cadence. The strace fixture attaches before the server
and Cloud Hypervisor children are created, follows child syscalls, retains a
mode-0700 raw capture, and checks distinct beacon binds plus old-ID cleanup
after successor creation. Static fixture review passes. Commit `72cb4fea`
reconciles the active control-plane bodies to terminal predecessors and fresh
successors: assertions now preserve predecessor rows/history while selecting
the numeric-current successor, and no production API or compatibility branch
was added. Commit `b295d973` records the actual native receipt and corrects the
one live boot comment to name predecessor `alloc_id`, successor `spec.alloc`,
and exact-old cleanup order.

## DES, commit, and mechanical evidence

| Check | Result | Evidence |
|---|---|---|
| Roadmap validation | PASS | `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json` → `VALID: 1 phases, 2 steps`. |
| DES integrity | PASS | `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/vm-recreation-allocation-id-reuse/deliver/` → `All 2 steps have complete DES traces`. |
| Step-01-02 phase order | PASS mechanically | `execution-log.json` records RED `FAIL` at `18:26:57Z`, GREEN `PASS` at `18:58:55Z`, COMMIT `PASS` at `18:59:51Z`, fixture-only remediation GREEN/COMMIT at `19:09:56Z`/`19:10:10Z`, and the acceptance reconciliation/remediation GREEN/COMMIT at `20:16:39Z`/`20:16:51Z`. |
| Implementation attribution | PASS | `c830b554`, `69957033`, `72cb4fea`, and `b295d973` retain Marcus Schack Abildskov as author/committer and carry exactly one `Co-Authored-By: Codex <codex@openai.com>` with no Claude/Anthropic attribution; the three step-delivery commits `c830b554`, `69957033`, and `b295d973` also carry `Step-Id: 01-02`. |
| Scope/whitespace | PASS | `git diff --check 69957033..HEAD` and `git diff --check c830b554..HEAD` are clean. The reconciliation changes only the affected active test specifications; the final production change is the bounded live-comment correction. |
| Mutation testing | PASS — not run | Explicitly prohibited for this step and not used to justify any result. |

## Verification evidence

All executable checks below were routed through the required Lima runner unless
otherwise noted.

| Command/scope | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test driver_neutral_allocation_replacement --no-capture --no-fail-fast` | **PASS — 5/5** |
| Service projection selection in `overdrive-sim::integration` | **PASS — 3/3** (`liveness_restart_budget_and_finalization_keep_projection_handoffs`, `failed_withdrawal_drains_terminal_and_repairs_after_view_reload`, `startup_failure_withdraws_then_fresh_successor_starts_unobserved`) |
| `service_kind_vm_terminal_invariant::terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility` | **PASS — 1/1**, active SIM-P8 control |
| `allocation_restart_write_spike` | **PASS — 2/2** |
| `e09_v2_failure_stream_overlap_spike` | **PASS — 2/2** |
| Full `overdrive-sim` integration binary | **PASS — 36/36** |
| Full `overdrive-sim` acceptance binary | **PASS — 97/97** (one unrelated test was reported skipped by nextest) |
| Full `overdrive-reconcilers` acceptance binary | **PASS — 18/18** |
| Full `overdrive-core` acceptance binary | **PASS — 557/557** |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests,kvm-tests` | **PASS** |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests,kvm-tests -- -D warnings` | **PASS** |
| `cargo xtask lima run -- cargo fmt --all -- --check` | **PASS** |
| `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint` | **PASS** |
| Full `overdrive-control-plane` acceptance binary | **PASS — 360/360** with 3 nextest-skipped tests; the prior two stale runtime View assertions now select the numeric-current successor. |
| Full `overdrive-control-plane` library tests | **PASS — 226/226** with `--features integration-tests`; the prior two boot-reclamation assertions now require fresh successor reservations and preserve predecessor history. |
| Full `overdrive-control-plane` integration binary | **PASS — 214/214**; mTLS, C3, crash-recovery, and two-cycle bodies now drive terminal predecessors and distinct successors. Nextest reports one pre-existing `LEAK` for unrelated `terminal_propagation::non_terminal_transitions_emit_none`; that file is unchanged by this step and the leak is not part of the accepted replacement contract. |
| Qualified native-metal six-test selection | **PASS — 6/6, exit 0** in retained `evidence-01-02-metal.md` on native non-virtualized x86_64 KVM; the receipt cites raw strace at `/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789326561586357993-1787813/strace.raw`. |

## Findings

### F-01 — Iteration 1 BLOCKER: active control-plane tests encoded the superseded same-ID Restart contract

**Reachability proof.** The changed production entry point is the public
`action_shim::dispatch` path into `dispatch_single`, whose Restart arm now
fences the observation row and requires `Failed|Terminated`
(`crates/overdrive-control-plane/src/action_shim/mod.rs:2323-2335`). The
registered production owner path is
`run_convergence_tick_inner` → View fsync → `dispatch_with_network_provisioner`
(`reconciler_runtime.rs:1443-1500,1526-1571`). The corrected
`WorkloadLifecycle` producer returns predecessor `alloc_id` plus fresh
`spec.alloc` (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1021-1062`).
These are real owner/caller paths, not hypothetical futures.

The iteration-1 full Lima runs reproduced the failures:

| Failing active body | Exact stale input/assertion | Observed failure |
|---|---|---|
| Seven `mtls_install_fail_closed` Restart cases | `seed_running_row` creates `Running` at `:596-624`; Restart actions reuse that ID at `:681-685`, `:830-835`, and `:1354-1359`, while assertions require driver start/old cleanup (`:736-756`, `:889-917`, `:1404-1478`). The new terminal handoff correctly returns before any effect. | 7 failures in the 214-test integration run; examples: `restart_allocation_install_failure_never_releases_the_exit_watcher` got `starts=[]`, and `restart_running_write_rejection_tears_down_network_and_releases_slot` got `Ok(())`. |
| `veth_provision_idempotent::c3_restart_replaces_and_converges_vm_network_plan` | Starts a `Running` predecessor at `:705-724`, then dispatches same-ID Restart at `:748-763`; it still expects predecessor teardown before replacement at `:764-767`. | Fails at `alloc_netns_lifecycle.rs:764`: “restart must tear down the prior structural netns before replacement provision.” |
| `workload_lifecycle::crash_recovery::killed_workload_is_restarted_with_fresh_alloc_id` | The module documents Phase-1 same-ID recovery at `:1-24` and searches the old key for `restart_count >= 1` at `:232-253`. The corrected action publishes a fresh key with zero per-key history. | Times out in the 214-test integration run without the expected old-key recovered row. |
| `workload_lifecycle::crash_observability_two_cycles::two_crash_cycles_count_two_restarts_and_describe_the_second_terminal` | Both recovery actions use `alloc_id == spec.alloc` at `:243-257` and `:310-326`; the old test expects cross-cycle `restart_count`/`last_terminated` on one key. | Times out at `crash_observability_two_cycles.rs:146`; the same-ID path can stop the just-started execution during exact-old cleanup rather than provide the new contract. |
| `runtime_convergence_loop::stop_after_failed_alloc_drains_broker` and `runtime_reconcile_is_idempotent_across_simulated_control_plane_restart` | Warm-up reads `restart_counts`/`last_failure_seen_at` only at `alloc-…-0` (`:612-615`, `:907-910`), while the fresh candidate is now `alloc-…-1`. | Both fail their bounded warm-up assertions (`:620-625`, `:914-918`). |
| `vm_reclamation_boot` library tests | The boot/re-drive assertions at `:365-415` and `:533-580` require no second same-ID action. The current production reconciler returns a higher fresh successor, visibly shown in the failure as `alloc-…-2`. | Both fail in the 221-test library run at `:411` and `:573`. |

This is specification/test drift caused by the globally changed Restart
meaning, not permission to restore same-ID behavior. The accepted design
explicitly says old rows/history remain at the predecessor key and every
successor is fresh; the unchanged bodies therefore cannot remain active
acceptance evidence. The `vm_reclamation_boot` pair is carryover from the
01-01 identity cut and must be reconciled with that step's original owner;
the mTLS/C3/runtime/crash bodies are dependent control-plane evidence that
must be reconciled before this step can pass.

**Remediation disposition:** **RESOLVED** by `72cb4fea`. The six affected
test surfaces now seed `Failed`/`Terminated` predecessors, pass distinct
successor specs, assert predecessor immutability and fresh per-key history,
and select the numeric-current candidate where the runtime may have advanced
the successor. The reconciliation changed only the contradictory test
assumptions; it added no action variant, same-ID compatibility branch, new
error, or test-only production seam. The full control-plane acceptance,
library, and integration suites now pass at 360/360, 226/226, and 214/214.

### F-02 — Iteration 1 BLOCKER: no current-commit qualified native-metal 6/6 evidence with strace

The six mapped qualified-metal bodies are active and compile, and the static
fixture audit confirms real Cloud Hypervisor, cgroupfs/filesystem, SVID/mTLS,
process, socket, rootfs and strace observables. At iteration 1 this workspace
had no authorized native target: `OVERDRIVE_METAL_TARGET` was unset, the host
was `arm64`, `/dev/kvm` was unavailable, and no current receipt was retained.

At iteration 1, the only on-disk six-test claim was the superseded
`deliver/superseded-vm-only/review-01-03.md`, which names commit `d4db67f6`.
That commit's `workload_lifecycle.rs` still returned VM `StartAllocation`
(`:1047-1073`), whereas current `c830b554` implements the driver-neutral
predecessor-to-fresh-successor `RestartAllocation`. Under the verification
rule that historical evidence proves only its recorded SHA, that narrated
old-run receipt cannot prove c830b554/69957033.

**Remediation disposition:** **RESOLVED** by the retained receipt
`docs/feature/vm-recreation-allocation-id-reuse/deliver/evidence-01-02-metal.md`
recorded in `b295d973`. It contains the exact six-test `cargo xtask metal run
--` selection, native non-virtualized x86_64 KVM substrate, exit status `0`,
actual stdout showing `6 tests run: 6 passed`, and the GH #284 raw strace path
`/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789326561586357993-1787813/strace.raw`.
The receipt also records distinct predecessor/successor artifact paths and
the final exact-old cleanup complements. It is accepted as the retained
actual native receipt for this re-review; no Lima result is substituted.

### F-03 — Iteration 1 MEDIUM: live production boot comment said same-ID Restart

At iteration 1, `crates/overdrive-control-plane/src/lib.rs:2873-2878` said
that ordinary “same-id RestartAllocation” installs a fresh rule after boot.
The current production contract uses a predecessor `alloc_id` and a distinct
successor `spec.alloc`; the comment was on the active boot path, not an
explicitly historical ADR. It could mislead future changes back toward the
rejected same-ID identity model.

**Remediation disposition:** **RESOLVED** by the eight-line bounded comment
change in `b295d973`. `lib.rs` now names predecessor `alloc_id`, fresh
successor `spec.alloc`, successor-first completion, and exact-old
driver→mTLS→structural-network cleanup. No historical ADRs, API, owner, or
mechanism were changed.

## Test integrity and external-validity verdict

The activation diff removes only authorized pending markers. The 69957033
fixture change strengthens timing/ownership observability at the valid running
boundary and preserves final cleanup complements. The focused Sim tests drive
the production action shim/runtime with complete in-memory driven adapters;
the qualified-metal tests use real local Cloud Hypervisor and host effects;
none imports or links an Overdrive crate from a black-box expectation runner.
No test was weakened, silently skipped, replaced with a placeholder, or made
green by a fabricated successor row. The 72cb4fea reconciliation preserves
the original port-level behavior and rewrites only the stale identity
assumptions; its full suites are green. The retained native receipt contains
actual command output and an exit status, rather than a narrated expectation.

## Iteration history

| Iteration | Reviewed target | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | `c830b554` + `69957033` | **CHANGES_REQUESTED** | F-01 — active same-ID control-plane specifications fail; F-02 — current native-metal 6/6/strace receipt missing; F-03 — stale live boot comment | Return to the step's original crafter/acceptance-author path. Re-run all affected suites and current native-metal evidence; no roadmap advancement. |
| 2 | `c830b554` + `69957033` + `72cb4fea` + `b295d973` | **APPROVED** | No remaining findings; F-01/F-02/F-03 resolved | Acceptance reconciliation, retained native receipt, and bounded comment correction verified. |

## Final verdict

**APPROVED.** The successor-first action-shim implementation, active
control-plane suites, focused Sim evidence, and retained qualified-metal
receipt satisfy the ratified contract. F-01 was resolved by bounded
acceptance-specification reconciliation, F-02 by the current native 6/6
receipt with cited raw strace, and F-03 by the bounded live-comment correction.
No same-ID compatibility behavior, new public/private policy port, action,
store, schema, retry, lock, network, cleanup, or driver-specific mechanism was
introduced. Mutation testing remains intentionally unrun. The roadmap may
advance only through the orchestrator's normal next-step sequencing.
