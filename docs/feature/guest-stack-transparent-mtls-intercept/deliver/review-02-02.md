# Adversarial review — step 02-02

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `02-02` — born-captured ordering invariant (Q9)
- **Reviewer:** `nw-software-crafter-reviewer` (same step-specific adversarial reviewer)
- **Review ID:** `code_rev_20260828_201649_iteration_5`
- **Iteration:** 5
- **Reviewed commit:** `d1d8f76fd5fee2725489dd9bb263005bfd182257`
- **Parent:** `144a973ce93e750b2cab264ef7e0225f3c66a5ea`
- **Subject:** `test(guest-stack-transparent-mtls-intercept): close cancellation event stream`
- **Trailer:** `Step-Id: 02-02`
- **Final verdict:** **APPROVED**

## Executive summary

Iteration 5 closes the last defect. After the cancellation path validates held termination, EOF/no EXEC, one exact guest-authored event, and the final supervision delta, it releases supervision, drops the last local `VmDriver`, and requires the actual exit receiver to return `None`. The one-second timeout is now only a failing safety bound: a leaked sender produces `Err(Elapsed)`, while any duplicate produces `Ok(Some(_))`; only structural channel closure succeeds. The awaited aborted release task and completed watcher leave no hidden `VmDriver`/event-sender owner, so the complete event-stream complement is closed rather than sampled.

The remediation changes only the focused worker acceptance test and DES log. The controlled termination proof, complete guest-session EOF/no-EXEC assertion, supervision proof, exact Contract Shape declarations, production ordering, awaited release API, D6 placement, exact guest wire, kernel timestamps, TLS/kTLS proof, and frozen beacon protocol all remain intact. Formatting, affected-package check/clippy, and all four focused worker tests pass in Lima. There are zero blocker, critical, high, or medium defects; step 02-02 is approved.

## Iteration history

| Iteration | Commit | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `6c09d07a2a9171bd887529e4c6ab090d6a8e0119` | **NEEDS_REVISION** | 5 blockers | All open; return to the original step-02-02 crafter, then re-review with this reviewer. |
| 2 | `93539c74f977646b1699a1cd71c492ec4d9c757a` | **NEEDS_REVISION** | 2 blockers | D2 and D5 resolved; D1 and D3 remain partially open; D4's mechanical declarations are resolved but its cancellation universe still omits the gate consequence described by D1. |
| 3 | `afbd2414785111d0a33ef0d16aa80869503ade22` | **NEEDS_REVISION** | 1 blocker | D3 is resolved and D1's production ownership/order is resolved; D1/D4 remain open only because the forced-cancellation temporal oracle can pass with gate-before-termination. |
| 4 | `144a973ce93e750b2cab264ef7e0225f3c66a5ea` | **NEEDS_REVISION** | 1 blocker | D1's termination-before-event oracle is resolved. D4 remains open because the exactly-one event complement accepts an elapsed 100 ms sample instead of proving stream completion. |
| 5 | `d1d8f76fd5fee2725489dd9bb263005bfd182257` | **APPROVED** | 0 | D4 is resolved by structural sender closure and mandatory `Ok(None)`; all prior remediations remain effective. |

## Contract Shape Compliance — iteration 1

**Overall: FAIL**

| Check | Status | Evidence |
|---|---|---|
| Per-test `CONTRACT_SHAPE` declaration | PASS mechanically | Both transitioned tests contain a declaration. |
| Exact Outcome Anchor | **FAIL** | S-GTI-02 says `Outcome anchor: DISCUSS Elevator Pitch.` with an extra period. `start_defers_exec_message_until_the_running_gate_is_released` substitutes `Outcome anchor: S-GTI-02 — ...` instead of the exact required line. |
| Banned test-name regex | PASS | Neither step-touched test matches `^test_.*(returns_\d+|exit_code|calls_.*_once|status_code|http_\d+)`. |
| Unbounded-preservation mechanism | **FAIL** | S-GTI-02 snapshots only loopback peer-port traffic and selected tracing fields, not the complete relevant wire/order universe. The driver test enumerates one 100 ms silence sample and one expected line; it has no complete before/after snapshot or audit of the unbounded session/order surface. |
| Bounded-change complement | Not applicable | Neither step-touched test declares `bounded-change`. |
| Layer choice and external validity | **FAIL** | The metal scenario uses production entry points, but its first-connect and ordering oracles are blind/circular as detailed in D2 and D3. |

The mandated checker `src/des/cli/check_contract_shape_declarations.py` is not present in this checkout or the installed nWave tree. The mechanical result above is from a direct diff-scoped source audit.

## Mechanical evidence — iteration 1

### Commit scope — PASS

- Stat: **5 files changed, 374 insertions, 106 deletions**.
- Production changes are confined to `vm_driver.rs` and the two existing action-shim Running arms. The metal scenario, focused driver acceptance case, and DES log are tightly related verification/fallout.
- `git diff --check 3a7e92f1..6c09d07a` passes.
- No `Cargo.toml`, OpenAPI, beacon enum, beacon parser/formatter, or mutation-exclusion file changes are in the commit.
- The current dirty `roadmap.json`, `AGENTS.md`, prior review artifacts, and concurrent user-owned `.claude/rules/rust.md` edit are outside the reviewed commit and were preserved. This review adds only this required review artifact.

### Q9 protocol and D6 placement audit

| Claim | Result | Evidence |
|---|---|---|
| Existing guest-initiated session retained | PASS | `VmDriver::start` stashes the accepted `OwnedWriteHalf` in `BeaconSession`; no new connect path exists. |
| `BeaconMessage` Published Language unchanged | PASS | Parent-to-commit diff for `crates/overdrive-core/src/vm/beacon.rs` is empty. |
| No new public `Driver` method | PASS | Parent-to-commit diff for `crates/overdrive-core/src/traits/driver.rs` is empty. D1 explains why the existing method must instead become async. |
| No EXEC inside `VmDriver::start` beacon-win arm | PASS | The arm stores `pending_exec` and returns without writing it. |
| Fresh D6 install site unchanged | PASS | `action_shim/mod.rs:1727-1755` calls the one existing `start_alloc(&spec)` before the release hook. |
| Restart D6 install site unchanged | PASS | `action_shim/mod.rs:2034-2060` is symmetric and contains the second/only other `start_alloc(&spec)` call. |
| Install error sends no EXEC | PASS structurally | Both arms return through `fail_closed_on_mtls_install` before the release hook; `VmDriver::stop` wins or closes the held session without calling `write_exec`. |
| Duplicate release | PASS for current state extraction | `Option::take` makes the pending message and exit gate single-owner; later calls are no-ops. |
| Cancellation/stop/release lifetime | **FAIL** | D1 and D5. The detached release outlives caller cancellation, and a stalled release can obstruct stop before its deadline. |

### DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T18:01:10Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T18:14:08Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T18:14:19Z` |

The canonical phases are present, successful, and chronological. The commit has the required step trailer and was authored/committed at the COMMIT timestamp.

### Test budget — PASS

| Roadmap behaviors | Budget (`2 × behaviors`) | Step-touched live tests | Status |
|---:|---:|---:|---|
| 4 | 8 | 2 | PASS |

The two tests are the transitioned S-GTI-02 metal scaffold and the transitioned driver EXEC-write acceptance test. Helper types/functions are not counted as behavioral tests.

### Test-integrity diff — FAIL

The old driver assertion that `start` writes EXEC before returning was correctly transitioned because the approved requirement reverses that behavior; this is not prohibited weakening. However, the replacement driver test exercises a test-only direct release and does not cover the action-shim/install boundary, cancellation, or concurrent stop. More importantly, the metal test's new positive assertions are ineffective against the regressions they claim to kill: proxy traffic can satisfy the non-empty SYN check while the relevant guest wire is unobserved, and implementation-authored events can remain ordered while the actual EXEC write moves. That is testing theater, not a mere coverage preference.

## Blocking findings — iteration 1

### D1 — `release_for_exit_emission` hides async security I/O behind a detached task

- **Severity:** Blocker
- **Dimension:** Async API contract, structured concurrency, cancellation safety, and Q9 happens-before ownership
- **Locations:**
  - `crates/overdrive-core/src/traits/driver.rs:751-763`
  - `crates/overdrive-worker/src/vm_driver.rs:1279-1356`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1750-1756`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:2057-2061`
  - `crates/overdrive-control-plane/src/worker/exit_observer.rs:259-263`

**Evidence:** `Driver` already uses `#[async_trait]`, and both action-shim release sites are in async functions. Nevertheless, the commit retains a synchronous `fn release_for_exit_emission`, takes `pending_exec` and the exit gate out of supervised state, calls `Handle::try_current`, and detaches `runtime.spawn(async move { session.write_exec(...).await; ... })`. The caller therefore observes only task submission. It immediately proceeds to `on_alloc_running`, lifecycle emission, and successful dispatch while the actual EXEC write may not have started.

This changes ownership in ways the Q9 contract cannot accept. Cancelling the action-shim future after the synchronous call does not cancel the detached release. Runtime shutdown can drop the spawned future after state/gate ownership has already been removed. Socket failure is handled later in an unrelated task and cannot be observed at the orchestration boundary. The no-runtime branch releases the exit gate but neither sends EXEC nor closes/terminates the still-supervised VM, leaving a live guest waiting forever. The trait rustdoc still describes only a synchronous exit-event gate and does not document the new socket effect or completion semantics.

The governing Rust rule now states this exact pattern is forbidden: async effects must use async APIs; a synchronous method must not discover a runtime and detach a task merely because its new body needs `.await`.

**Required remediation:** Change the existing trait hook to `async fn release_for_exit_emission(...)` rather than inventing a second public method. Update every implementation, double, and call site; await it at both action-shim Running arms and at the degraded observer path. `VmDriver` must perform/resolve the real beacon release in that structured future and release the exit gate only after the write has completed or its fail-closed error handling has completed. Remove `Handle::try_current` and the detached spawn. Update the public trait contract to describe the VM side effect, completion, idempotency, cancellation, and error behavior. Add a test in which the release future is deliberately held: the action shim must not proceed/return until actual release completes, and cancellation must not leave an independently running EXEC sender.

### D2 — S-GTI-02's no-clear-SYN oracle does not observe or correlate the guest's first escape path

- **Severity:** Blocker
- **Dimension:** Security-test honesty, exact-wire correlation, and first-connect external validity
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:75`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:259-327`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:330-380`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:820`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1212-1222`

**Evidence:** The only AF_PACKET capture is bound to `LOOPBACK_IFACE = "lo"`. The guest's pre-intercept packet arrives through its TAP and allocation host-veth; the successful proxy's leg-B/leg-C traffic is what traverses loopback. A clear guest SYN can therefore use the path the test is supposed to reject without appearing in this capture. `syns_to_peer > 0` does not make the negative oracle non-empty: it counts every initial SYN to the shared port on loopback, including the expected proxy TLS connection. `guest_originated_syns_to_peer == 0` can remain trivially true because that guest-originated wire is not captured.

The counters also aggregate every tuple touching port 18951 over the whole scenario. They do not identify the guest command's first mesh attempt, do not correlate a SYN to the exact kTLS socket tuple, and do not delimit pre-install from post-install traffic. A later correct proxy connection can supply the positive SYN/TLS evidence while an earlier clear attempt on the TAP/host-veth goes unseen. The green 18.878 s metal run demonstrates execution, not the missing observation.

**Required remediation:** Capture the actual guest egress/escape boundary (host-veth or another independently justified exact wire) from before VM release, and correlate the guest source/destination tuple and first SYN to the same allocation and mesh destination. Prove the negative over the complete relevant frame universe, not via a shared-port aggregate; a different proxy socket must not make the oracle non-empty. Preserve the existing exact bidirectional TLS/kTLS proof as the post-intercept leg proof, but do not use it as a substitute for the pre-install guest-wire assertion.

### D3 — Install-before-release is proved only by two implementation-authored trace events

- **Severity:** Blocker
- **Dimension:** Circular oracle and mutation sensitivity
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:415-463`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1177-1211`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1743-1755`
  - `crates/overdrive-worker/src/vm_driver.rs:1331-1336`

**Evidence:** The test installs a tracing subscriber, finds `mtls.intercept.install.success` and `vm.beacon.exec.released`, and compares their vector positions. Both facts are emitted by the implementation under review at hand-selected sites. A regression can write or duplicate EXEC before install and leave the release log at its current post-install site; the test still passes. Likewise, moving log placement can make the test fail without changing the socket ordering. No independent observable identifies the actual EXEC bytes on the guest-initiated session or ties the first guest action to the installed-rule boundary.

This is especially material because D1 already makes the caller/task boundary asynchronous: the event assertion validates the detached task's logging convention, not the action shim's awaited completion contract. A green log sequence is not proof of structured happens-before.

**Required remediation:** Replace the event-only ordering oracle with an independent observation of the real boundary: actual intercept/rule readiness and actual EXEC release (or the causally first guest action) on the production path, correlated to the same allocation/session/tuple. Do not invoke the release hook directly from the metal test, hand-install networking, or create a test-only release channel. A focused action-shim test may supplement the metal proof, but it must drive the real async trait call and cannot replace the external S-GTI-02 observation.

### D4 — The transitioned tests fail mandatory Outcome Anchor and unbounded-preservation rules

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance
- **Locations:**
  - `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:573-625`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1166-1240`

**Evidence:** The driver test uses `Outcome anchor: S-GTI-02 — ...`, not the exact mandatory `Outcome anchor: DISCUSS Elevator Pitch`. S-GTI-02 adds a period to that exact anchor. Both tests declare `unbounded-preservation`. The driver test then checks only a 100 ms no-line sample and one expected parsed line; it neither snapshots a complete before/after observable universe nor audits the complement across cancellation, stop, duplicate release, write error, session drop, or runtime shutdown. The metal test's incomplete loopback/event projection is D2/D3 and cannot substantiate its unbounded claim.

**Required remediation:** Add the exact Outcome Anchor line to both test docstrings. Reclassify the focused driver test if its honest contract is narrower, or implement a genuine unbounded-preservation snapshot/audit over the declared session/order universe. For S-GTI-02, fix D2/D3 and explicitly define the complete relevant wire/order universe and its preservation proof. Do not retain an unbounded label over enumerated observations.

### D5 — The new session mutex can make `VmDriver::stop` non-total before its deadline begins

- **Severity:** Blocker
- **Dimension:** Deadlock/liveness, stop totality, and unbounded I/O under lock
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:443-459`
  - `crates/overdrive-worker/src/vm_driver.rs:469-477`
  - `crates/overdrive-worker/src/vm_driver.rs:1201-1211`
  - `crates/overdrive-worker/src/vm_driver.rs:1287-1330`
  - `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:754-791`

**Evidence:** Release acquires `beacon.lock().await` and holds the `tokio::sync::MutexGuard` across `write_all` and `flush`. `stop` transitions supervision to `EndingInFlight`, then awaits the same mutex before it can set `shutdown_requested`, write SHUTDOWN, start `VM_SHUTDOWN_REQUEST_DEADLINE`, or call `Vmm::terminate`. Beacon argv has no demonstrated bound below the finite Unix-socket send buffer. A guest that sent READY but does not read, combined with a sufficiently large EXEC line, can therefore stall the release while holding the mutex and strand stop indefinitely before the nominal two-second bound starts.

The existing S-VM-76 sequence-b test does not exercise the new race. It sends no EXEC, leaves an initially empty send buffer, and tests only the later simulated grace sleep after a small SHUTDOWN write. The transitioned focused test uses a tiny argv and actively drains the peer. Neither can fail when release owns the lock and the peer is non-reading. This violates both the repository rule against locks across `.await` and AC-18's requirement that stop be total at every point in the start path.

**Required remediation:** Make EXEC/SHUTDOWN serialization cancellation-aware without holding a lock across unbounded socket I/O. A stop request must be able to prevent/cancel a not-yet-complete EXEC release and reach bounded termination; write/flush and lock acquisition cannot sit outside the stop bound. Extend S-VM-76 with a concurrent release-versus-stop case using a non-reading guest and enough data/backpressure to force the wait. Assert stop reaches `Vmm::terminate` within the simulated/bounded contract, no complete EXEC line reaches the guest after stop wins, and the exit gate/session are not stranded.

## External validity — iteration 1

**Status: FAIL**

The scenario boots a real KVM guest through the production `serve`/deploy path, uses the production guest-initiated beacon, production resolver, production intercept worker, and real nft/kTLS datapath. The guest program's first workload action is a by-name mesh dial. Those are strong foundations. External validity nevertheless fails because the two step-defining assertions are not external: the clear-SYN claim watches the wrong/aggregated boundary, and the order claim watches only implementation-authored trace events.

## Verification — iteration 1

| Verification | Result |
|---|---|
| `git diff --check 3a7e92f1..6c09d07a` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused Lima deferred-EXEC driver test | PASS — 1 passed, 196 skipped |
| Metal S-GTI-02 | PASS — 1 passed, 242 skipped in 18.878 s |
| Post-metal residue | PASS — no guest netns; no guest TAP/veth beyond the two shared `ovd-veth-*` links; no Cloud Hypervisor process; `table ip overdrive-mtls` contains only the two shared mark-exemption rules and no per-allocation TPROXY rule |
| Unfiltered Lima workspace suite | Expected known failure — 2,806 selected; `openapi_check_subprocess_exits_0_against_checked_in_yaml` reports the pre-existing `workload_addr` drift |
| OpenAPI-excluded Lima workspace suite | NOT CLEAN in two independent reviewer attempts — 2,805 selected; one unrelated dataplane concurrency/kernel test failed on each attempt. Each failing test passed immediately when rerun alone. No changed file participates in either test. |
| Beacon enum/parser parent-to-commit diff | PASS — empty |
| Public Driver trait parent-to-commit diff | PASS — empty; no invented method, but D1 requires the existing hook to become async |
| OpenAPI/Cargo manifest parent-to-commit diff | PASS — empty; the excluded OpenAPI drift predates this step |
| D6 fresh/restart source audit | PASS — exactly two `start_alloc(&spec)` sites and each precedes its release call |
| Contract Shape audit | FAIL — D4 |
| Mutation testing | NOT RUN — prohibited during an individual roadmap step |

The first OpenAPI-excluded run reached 2,760 passes before `guardrails_fail_closed_limits_supervision_exemption_authn_boundary` saw `ConnectionRefused`; its isolated rerun passed. A second full attempt at `-j 4` reached 2,376 passes before `agent_handshake_presents_held_svid_server_role` saw a transient kTLS probe refusal; its isolated rerun passed. These are recorded accurately rather than represented as a reviewer-observed 2,805-pass run. They are not attributed to this commit, but the independent full-suite claim was not reproduced.

## Quality gates — iteration 1

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | Roadmap activates S-GTI-02 and the metal test executes. |
| G2 — valid RED | PASS mechanically | RED precedes GREEN and COMMIT in the DES log. |
| G3 — assertion failure | FAIL substantively | The green assertions do not kill wrong-wire or event/log decoupling regressions; D2/D3. |
| G4 — no domain mocks/test-only wiring | PASS for setup | The metal path uses production setup; its defect is the oracle, not hand wiring. |
| G5 — business language | FAIL | Mandatory exact Outcome Anchors are absent; D4. |
| G6 — all executed tests green | FAIL | Focused lanes are green; independent full-suite attempts encountered two unrelated transient failures. |
| G7 — green before commit | PASS | DES GREEN precedes COMMIT. |
| G8 — test budget | PASS | 2 ≤ 8. |
| G9 — no prohibited test weakening | FAIL | Replacement metal assertions are ineffective against their stated regression; D2/D3. |

## RPP and design-quality scan — iteration 1

- **RPP levels scanned:** L1-L4 across the trait contract, async ownership, action-shim orchestration, beacon session state, stop path, and metal oracle.
- **Cascade stopped at:** L4 because the defects cross API, orchestration, I/O lifetime, and acceptance-proof boundaries.
- **Type/API result:** Reusing the existing trait hook is the correct public-surface direction, but keeping its stale synchronous signature after adding async I/O is not. The bounded fallout of making the existing hook async is required.
- **Error/cancellation result:** Fail-closed install error is structurally sound before release. Caller cancellation and stalled release are not sound because the I/O is detached and lock-held.
- **Protocol result:** Beacon PL remains unchanged; no new message or host-initiated connection exists.
- **Documentation result:** The public trait contract is stale about the VM socket effect and completion semantics.

## Iteration-1 defect counts

| Severity | Count |
|---|---:|
| Blocker | 5 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **5** |

## Remediation disposition — iteration 1

| Finding | Status | Owner/action |
|---|---|---|
| D1 — detached synchronous release | OPEN | Original step-02-02 crafter: make the existing hook async, await it everywhere, document/test structured completion and cancellation. |
| D2 — blind/aggregated SYN oracle | OPEN | Original step-02-02 crafter: observe and correlate the exact guest escape wire/first tuple. |
| D3 — trace-only order oracle | OPEN | Original step-02-02 crafter: add an independent actual install/release observation without test-only release wiring. |
| D4 — Contract Shape failure | OPEN | Original step-02-02 crafter: correct exact anchors and honest preservation mechanisms/classification. |
| D5 — stop-totality lock/writer race | OPEN | Original step-02-02 crafter: make release/stop cancellation-aware and add the missing forced-backpressure race acceptance case. |

No finding is waived or deferred. Under this repository's no-iteration-cap rule, the original crafter must remediate this same step and this reviewer must re-review until the verdict is `APPROVED`.

## Iteration-1 final verdict

**NEEDS_REVISION**

Step 02-02 must not advance. D1-D5 remain open. A green metal execution and mechanically correct DES/commit scope do not replace a structurally awaited release, total stop behavior, or a non-circular exact-wire proof of S-GTI-02.

---

## Iteration 2

### Remediation reviewed

- **Review ID:** `code_rev_20260828_212528_iteration_2`
- **Remediation commit:** `93539c74f977646b1699a1cd71c492ec4d9c757a`
- **Parent:** `6c09d07a2a9171bd887529e4c6ab090d6a8e0119`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): own async EXEC release`
- **Trailer:** `Step-Id: 02-02`
- **Verdict:** **NEEDS_REVISION**

### Iteration-1 disposition

| Finding | Status | Iteration-2 evidence |
|---|---|---|
| D1 — detached synchronous release | **PARTIALLY RESOLVED / OPEN** | The existing hook is async, all production call sites await it, runtime lookup/detached release are gone, write errors are handled in the awaited path, and the held-hook/action-shim test is real. Cancellation still drops the taken gate sender before the writer completes fail-closed socket closure; see D1 below. |
| D2 — blind/aggregated SYN oracle | **RESOLVED** | Capture starts before VM deploy on all interfaces, retains the actual `sockaddr_ll.sll_ifindex`, then selects the live allocation host-veth and exact `(guest source, mesh destination, port)` tuple. The positive first-SYN and plaintext-on-host-veth assertions make this boundary non-vacuous. |
| D3 — trace-only order oracle | **PARTIALLY RESOLVED / OPEN** | Trace-event ordering is removed. Actual nft rule state and the causally first guest SYN are observed independently. The packet's timestamp is nevertheless assigned at userspace dequeue, allowing a queued pre-rule frame to be mislabeled post-ready; see D3 below. |
| D4 — Contract Shape failure | **PARTIALLY RESOLVED** | Exact declarations and anchors pass, the driver test is honestly reclassified `bounded-change`, and S-GTI-02 audits the complete exact-tuple collection. The cancellation test's declared universe omits the exit-event gate consequence central to the hook; this is part of open D1 rather than a third independent finding. |
| D5 — stop-totality lock/writer race | **RESOLVED** | No lock crosses socket I/O. Stop signals outside the bounded command queue, starts its deadline before waiting on the writer, aborts at the bound, and the 16 MiB/non-reading-peer stop and cancellation cases execute green with no complete EXEC line. |

### Contract Shape Compliance — iteration 2

**Overall: FAIL because the cancellation universe omits the gate; all mechanical checks pass.**

| Check | Status | Evidence |
|---|---|---|
| Per-test `CONTRACT_SHAPE` declaration | PASS mechanically | The five step-owned live tests carry exact declarations: S-GTI-02 is `unbounded-preservation`; the focused defer, stop-backpressure, cancellation-backpressure, and action-shim-await tests are `bounded-change`. |
| Exact Outcome Anchor | PASS mechanically | All five use the exact line `Outcome anchor: DISCUSS Elevator Pitch` with no trailing punctuation. |
| Banned test-name regex | PASS | No step-owned name matches `^test_.*(returns_\d+|exit_code|calls_.*_once|status_code|http_\d+)`. |
| S-GTI-02 unbounded-preservation mechanism | PASS apart from D3's time source | The audit quantifies over every captured segment on the exact host-veth/guest/mesh tuple and every peer-port stream, proves the universe non-empty, and retains independent nft/TLS/kTLS evidence. It no longer enumerates unrelated loopback SYNs or trace fields. |
| Bounded-change declared delta/complement | **FAIL for cancellation only** | The focused release and stop-race tests declare and close their stated byte/liveness/supervision complements. `cancelling_backpressured_release_cannot_leave_an_exec_sender_running` omits the gate sender/receiver even though the method's public contract names it and cancellation drops it. |
| Layer choice and external validity | **FAIL** | Production entry points, real KVM, real nft and exact real wire are used. D3 leaves one false-positive ordering window in the independent observation. |

The mandated checker `src/des/cli/check_contract_shape_declarations.py` is still absent from both this checkout and the installed nWave tree. The mechanical checks above are a direct diff-scoped source audit.

### Mechanical evidence — iteration 2

#### Commit scope

- Remediation stat: **14 files changed, 921 insertions, 256 deletions**.
- The broader scope is bounded fallout from making the existing trait method async, the production writer/cancellation design, exact kernel/wire observation, and focused regression tests. The new `overdrive-netlink` dev dependency is used only by S-GTI-02's independent kernel-rule decoder.
- `git diff --check 6c09d07a..93539c74` passes.
- Commit subject and `Step-Id: 02-02` trailer are correct.
- No mutation exclusions, OpenAPI schema, or beacon protocol file changed.
- Dirty user-owned `.claude/rules/rust.md`, `roadmap.json`, `AGENTS.md`, and existing review artifacts remain outside the remediation commit and were preserved.

#### Q9 protocol and D6 placement

| Claim | Result | Evidence |
|---|---|---|
| Existing guest-initiated session retained | PASS | `accept_ready` still splits the connection the guest opened; `BeaconWriter::spawn` receives that `OwnedWriteHalf`. No host connect path exists in `vm_driver.rs` or the action shim. |
| `BeaconMessage` Published Language unchanged | PASS | Original step parent through remediation has an empty diff for `crates/overdrive-core/src/vm/beacon.rs`. |
| No second public release method | PASS | The existing `release_for_exit_emission` changed from sync to async; no additional hook was added. |
| No EXEC in `VmDriver::start` | PASS | The beacon-win arm stores `pending_exec`; the only actual write is owned by the post-install async release path. |
| Fresh/restart D6 sites | PASS | Exactly two `start_alloc(&spec)` sites remain, at the two existing Running arms, and each precedes one awaited release. |
| Install error sends no EXEC | PASS | Both errors return through the existing fail-closed helper before the release hook; stop signals/closes the retained writer and terminates the VMM. |
| Duplicate release | PASS | `pending_exec.take()` and `gate_sender.take()` retain single ownership; a duplicate call has no command or gate to release. |
| Stop deadline and backpressure | PASS | `request_stop` is synchronous/out-of-band; the two-second deadline is created before waiting for the writer task and bounds/aborts it. |
| Gate-after-release/fail-closed | **FAIL** | D1: cancellation drops the outer future's local `gate_sender` while the writer only then observes its cancellation signal and closes the socket. |

#### DES phase order

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T18:45:07Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T19:12:19Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T19:12:45Z` |

The remediation cycle is complete, successful, chronological, and lands at the COMMIT timestamp.

#### Test budget and integrity

| Roadmap behaviors | Budget (`2 × behaviors`) | Step-owned live tests after remediation | Status |
|---:|---:|---:|---|
| 4 | 8 | 5 | PASS |

The five tests are S-GTI-02, the focused deferred-release test, two forced-backpressure race tests, and the action-shim awaited/cancellation-ownership test. Async signature-only edits to pre-existing SimDriver/call-site tests are compiler fallout, not newly authored behaviors.

No existing assertion was weakened or deleted. The metal oracle was substantially strengthened from trace/shared-loopback observation to actual exact-boundary observation, and the driver transition adds duplicate-release, cancellation, stop, and complete-session assertions. G9 therefore passes. D3 is a remaining false-positive path in a new oracle, not a concealed requirement reduction.

### Blocking findings — iteration 2

#### D1 — cancellation opens the exit-event gate before fail-closed socket completion

- **Severity:** Blocker
- **Dimension:** Async cancellation safety, public contract accuracy, and release/gate happens-before ownership
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:445-465`
  - `crates/overdrive-worker/src/vm_driver.rs:487-519`
  - `crates/overdrive-worker/src/vm_driver.rs:575-595`
  - `crates/overdrive-worker/src/vm_driver.rs:1442-1495`
  - `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:912-985`

**Evidence:** The async hook takes `gate_sender` out of `LiveVm` before awaiting `beacon.release_exec`. The cancellation guard is inside that nested future and only sends a watch signal on drop. When the action-shim/release task is cancelled, dropping the outer future also drops its local `gate_sender`; the exit watcher's oneshot resolves immediately through the documented sender-drop path. The writer task processes the watch signal later, at its next poll, acknowledges `Cancelled`, and only then returns and drops the sole socket write half. There is no happens-before edge from fail-closed closure to gate opening on this path.

The writer also sends its `Stopped`/`Cancelled` acknowledgement at line 590 before the `return` at line 595 drops `write_half`. Although there is no `.await` between those statements, the oneshot receiver can run on another Tokio worker as soon as it is woken. Thus even a non-cancelled stop race does not structurally guarantee the source comment at lines 1491-1493: the acknowledgement is an intent to close, not proof the fail-closed socket effect completed.

The new cancellation test correctly forces real backpressure and waits for EOF, but it never observes the gate. It can pass while the hook's other named consequence opens early, which is exactly the gap in its declared bounded-change universe.

**Required remediation:** Keep gate ownership with the write outcome across cancellation. One viable shape is to move the gate sender into the queued command and return it to the awaiting hook with the acknowledgement on normal completion; if the acknowledgement receiver was cancelled, the writer must explicitly close/drop the write half first and only then drop/send the gate. On write failure with a live caller, return the gate to the hook so VMM fail-closed termination still completes before the hook sends it. Extend the forced-cancellation case to observe the actual gate receiver/exit-event boundary and prove it remains closed until session EOF or other fail-closed completion.

#### D3 — userspace dequeue timestamps can relabel a pre-rule SYN as post-ready

- **Severity:** Blocker
- **Dimension:** Security-oracle temporal validity and mutation sensitivity
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:306-389`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:462-490`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:493-528`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1308-1323`

**Evidence:** `CapturedFrame.observed_at` is assigned with `Instant::now()` after `recvfrom` dequeues a frame from the AF_PACKET socket. The socket is bound before deploy, so a regressed pre-install guest SYN is retained by the kernel even when the capture thread is descheduled. If the nft polling task observes the installed rule before the capture thread drains that queued SYN, the later `Instant::now()` makes the old frame appear newer than `InterceptReadiness.observed_at`. Both `pre_ready.is_empty()` and `first_syn.observed_at >= readiness.observed_at` then pass even though the packet entered the escape boundary before the rule was live.

This is no longer a wrong-interface or circular trace oracle: the allocation/ifindex/tuple and kernel rule are genuine, the metal run is non-vacuous, and the successful flow proves the rule acts. The remaining defect is specifically that receive scheduling is substituted for packet event time at the one ordering comparison that carries S-GTI-02.

**Required remediation:** Timestamp packet arrival at the kernel boundary, not at userspace dequeue, and compare it against a conservative post-query rule-readiness barrier on the same clock domain. For example, enable a kernel packet timestamp ancillary record and retain it per frame; frames whose ordering cannot be proven must be conservatively classified pre-ready. A capture-thread handshake that drains and classifies all already-queued frames as pre-ready before acknowledging the barrier is also acceptable if it cannot create a false positive. Keep the exact host-veth/guest/mesh tuple, non-empty plaintext guest-boundary proof, and peer TLS/kTLS assertions unchanged.

### External validity — iteration 2

**Status: FAIL**

The test executes the complete real `serve` + deploy + Cloud Hypervisor + guest-init + by-name first-dial path on the metal box. It independently observes the real nft ruleset, exact host-veth traffic, peer TLS records, and kTLS socket state. D2's former wrong-boundary problem is resolved. External validity remains blocked only because the ordering comparison can reorder an already-queued packet at userspace dequeue (D3); the observed objects are real, but their compared times are not the kernel event order.

### Verification — iteration 2

| Verification | Result |
|---|---|
| `git diff --check 6c09d07a..93539c74` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused worker release/gate/backpressure tests | PASS — 4 passed, 195 skipped in 0.516 s |
| Focused control-plane awaited-release/fail-closed tests | PASS — 3 passed, 776 skipped in 0.339 s |
| Metal S-GTI-02 | PASS — 1 passed, 242 skipped in 18.837 s |
| Post-metal residue | PASS — no guest netns; no guest TAP/allocation veth beyond the two shared `ovd-veth-*` links; no Cloud Hypervisor process; nft table contains only the two shared mark-exemption rules |
| OpenAPI check | Expected known pre-existing failure — first divergence near `/v1/workloads/{id}/stop` (`workload_addr` vs `workload_id`); remediation changes no OpenAPI file or API response type |
| Beacon enum/parser diff | PASS — empty from original step parent through remediation |
| D6 fresh/restart audit | PASS — exactly two existing `start_alloc(&spec)` sites, both before awaited release |
| Contract Shape audit | FAIL semantically — D1's cancellation test omits the gate; exact declarations/anchors pass |
| Mutation testing | NOT RUN — repository DELIVER rules prohibit mutation testing during an individual roadmap step |

### Quality gates — iteration 2

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | S-GTI-02 is live and runs on its required metal surface. |
| G2 — valid RED | PASS | Remediation RED is executed/pass and precedes GREEN. |
| G3 — assertion failure quality | **FAIL** | D3 permits a release-before-install packet to be dequeued/stamped after readiness. |
| G4 — no domain mocks/test-only release | PASS | Metal uses production setup; focused doubles are at the Driver port and no test-only release path enters metal. |
| G5 — business language | PASS | Exact anchors and domain-oriented names pass. |
| G6 — in-scope executed tests green | PASS | All focused and metal tests pass; the known OpenAPI drift is unchanged and outside this step. |
| G7 — green before commit | PASS | GREEN precedes COMMIT in the remediation DES cycle. |
| G8 — test budget | PASS | 5 ≤ 8. |
| G9 — no prohibited test weakening | PASS | Assertions are additive/strengthened; no test is deleted, skipped, or relaxed. |

### Test integrity and RPP scan — iteration 2

- **Test modification detected:** No prohibited modification.
- **Testing theater detected:** No fully mocked, tautological, assertion-free, or always-green test. D3 is a narrower temporal false-positive defect in an otherwise real, non-vacuous external oracle.
- **Escalation:** Not applicable; no requirement was changed.
- **RPP levels scanned:** L1-L4. No independent refactoring smell is reported; the remaining findings are correctness/contract defects. The single-writer abstraction is justified by the EXEC/SHUTDOWN concurrency requirement and has multiple concrete consumers (release, stop, drop).
- **External validity:** FAIL solely on D3.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 2 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **2** |

### Iteration-2 remediation disposition

| Finding | Status | Owner/action |
|---|---|---|
| D1 — gate opens before cancellation fail-closed completes | OPEN | Original step-02-02 crafter: carry gate ownership through the writer outcome and add a gate-aware forced-cancellation assertion. |
| D2 — exact guest escape wire | RESOLVED | No further action; retain exact ifindex/source/destination correlation. |
| D3 — dequeue-time ordering false positive | OPEN | Original step-02-02 crafter: use kernel event time or a conservative capture barrier that cannot relabel queued pre-ready frames. |
| D4 — declarations/preservation | PARTIALLY RESOLVED | Exact syntax and S-GTI-02 audit pass; close the cancellation universe together with D1. |
| D5 — stop totality under backpressure | RESOLVED | No further action; retain out-of-band stop and both forced-backpressure cases. |

No finding is waived or deferred. The same crafter/reviewer pair must continue the repository's uncapped remediation cycle until zero defects remain.

### Iteration-2 final verdict

**NEEDS_REVISION**

Step 02-02 must not advance. The remediation is materially stronger and D2/D5 are closed, but the gate/fail-closed cancellation order and the packet-event time oracle remain blocking security-contract defects.

---

## Iteration 3

### Remediation reviewed

- **Review ID:** `code_rev_20260828_195611_iteration_3`
- **Remediation commit:** `afbd2414785111d0a33ef0d16aa80869503ade22`
- **Parent:** `93539c74f977646b1699a1cd71c492ec4d9c757a`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): order gate and kernel evidence`
- **Trailer:** `Step-Id: 02-02`
- **Verdict:** **NEEDS_REVISION**

### Iteration-2 disposition

| Finding | Status | Iteration-3 evidence |
|---|---|---|
| D1 — gate opens before cancellation fail-closed completes | **PRODUCTION RESOLVED / TEST ORACLE OPEN** | `gate_sender` is moved into `ExecWriteCommand` without an intervening await. On cancellation, the writer closes the sole write half, awaits fail-closed VMM termination, and only then acknowledges; the cancelled acknowledgement receiver drops the gate at that safe boundary. Normal write failure returns the gate to the live hook, which awaits VMM termination before sending it. The forced-cancellation test now consumes the actual exit receiver, but its termination-before-event assertion is not causally discriminating; see D1 below. |
| D2 — blind/aggregated SYN oracle | **RESOLVED / RETAINED** | S-GTI-02 still filters the capture to the actual allocation host-veth ifindex, exact guest source address, and exact mesh destination tuple; it retains the non-empty first-SYN/plaintext boundary and peer TLS/kTLS proof. |
| D3 — dequeue-time ordering false positive | **RESOLVED** | The capture socket enables `SO_TIMESTAMPNS`; `recvmsg` decodes `SCM_TIMESTAMPNS`, validates the timespec, and retains the kernel event time. Readiness is sampled with `CLOCK_REALTIME` only after the successful typed nft query and ifindex resolution. Missing, invalid, truncated, or equal timestamp evidence is conservatively classified pre-ready. |
| D4 — declarations/preservation | **PARTIALLY RESOLVED / OPEN** | Exact declarations and anchors pass. S-GTI-02's unbounded audit is now temporally sound. The cancellation test names the gate/event/termination surface, but its post-event liveness sample does not prove the declared order; this is the same remaining D1 defect. |
| D5 — stop totality under backpressure | **RESOLVED / RETAINED** | The out-of-band stop signal, bounded writer ownership, socket-close-before-ack order, and both 16 MiB forced-backpressure cases remain. |

### Contract Shape Compliance — iteration 3

**Overall: FAIL because the cancellation test does not falsify gate-before-termination; mechanical declarations pass.**

| Check | Status | Evidence |
|---|---|---|
| Per-test `CONTRACT_SHAPE` declaration | PASS mechanically | All five step-owned live tests retain the exact required declaration. |
| Exact Outcome Anchor | PASS mechanically | All five retain the exact `Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-owned test name matches the banned implementation-detail pattern. |
| S-GTI-02 unbounded-preservation mechanism | PASS | The test audits every captured exact-tuple segment and every peer-port stream, requires genuine kernel timestamps for the first SYN, treats unknown ordering as pre-ready, and retains independent nft/TLS/kTLS/non-vacuity evidence. |
| Bounded-change declared delta/complement | **FAIL for cancellation only** | The cancellation docstring now declares the real exit-event receiver and termination order, but the assertions cannot distinguish event-send-before-termination from event-send-after-termination when `terminate` completes before scheduled observers run. |
| Layer choice and external validity | PASS for Q9 metal; FAIL for cancellation regression | S-GTI-02 uses the complete production KVM/nft/wire path. The focused cancellation test drives the real VM driver and exit receiver, but needs a controlled pending termination boundary to prove its temporal claim. |

The repository-mandated Contract Shape checker remains absent from this checkout and the installed nWave tree. The mechanical result is therefore a direct diff-scoped source audit; semantic validation is the adversarial analysis above.

### Mechanical evidence — iteration 3

#### Commit scope

- Remediation stat: **5 files changed, 374 insertions, 102 deletions**.
- The scope is tightly related: writer/gate ownership, its public trait contract, the forced-cancellation acceptance case, the kernel-timestamp metal oracle, and the DES log.
- `git diff --check 93539c74..afbd2414` and `cargo fmt --all -- --check` pass.
- Commit subject and `Step-Id: 02-02` trailer are correct; the commit parent is exact.
- No action-shim production file, mutation exclusion, OpenAPI schema, Cargo manifest, or beacon protocol file changes are in this remediation.
- Dirty user-owned `.claude/rules/rust.md`, `roadmap.json`, `AGENTS.md`, and existing review artifacts were preserved. This reviewer changed only this review artifact.

#### Q9 protocol, release ownership, and D6 placement

| Claim | Result | Evidence |
|---|---|---|
| Existing guest-initiated session retained | PASS | `accept_ready` still supplies the guest-opened `OwnedWriteHalf` to `BeaconWriter`; no host-to-guest connect exists. |
| `BeaconMessage` Published Language unchanged | PASS | The original step parent through this remediation has an empty diff for `crates/overdrive-core/src/vm/beacon.rs`. |
| Existing public release hook only | PASS | No second hook exists; all production calls of `release_for_exit_emission` are awaited. No runtime lookup or detached release spawn exists. |
| No EXEC in `VmDriver::start` | PASS | The beacon-win arm stores the existing `BeaconMessage::Exec`; only the post-install release hook submits it to the session writer. |
| Fresh/restart D6 sites | PASS | Exactly two `start_alloc(&spec)` sites remain in the existing Running arms, each before the awaited release. |
| Install error sends no EXEC | PASS | Both install-error arms return through fail-closed stop before the release hook. |
| Gate transfer at hook cancellation | PASS structurally | `try_send` transfers the gate with no await; the writer closes the socket, completes cancellation termination, and then drops the gate through the failed acknowledgement. |
| Acknowledgement after socket close | PASS | Every non-written active outcome calls `close_socket()` before `acknowledge_active`. Task-state drop also closes the socket before dropping active, queued, or emergency gates. |
| Live-caller write failure | PASS structurally | `WriteFailed` returns the still-owned gate; the hook awaits `Vmm::terminate` before sending it. |
| Stop totality | PASS | Stop still signals outside the queue, starts its two-second deadline before joining the writer, and aborts/escalates at the bound. |

#### DES phase order

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T19:34:54Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T19:45:59Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T19:46:04Z` |

The third step-specific cycle is complete, successful, chronological, and precedes the commit timestamp (`2026-08-28T19:46:11Z`).

#### Test budget and integrity

| Roadmap behaviors | Budget (`2 × behaviors`) | Step-owned live tests | Status |
|---:|---:|---:|---|
| 4 | 8 | 5 | PASS |

No existing assertion was weakened, removed, skipped, or replaced with a tautology. The metal test changes are strict strengthening: kernel event time replaces dequeue time and unknown/equal evidence fails conservatively. The cancellation test adds the real exit receiver and a liveness assertion, but D1 explains why that assertion is not sufficient to prove its stated temporal order.

### Blocking finding — iteration 3

#### D1 — the forced-cancellation test samples termination after the event instead of proving termination-before-event

- **Severity:** Blocker
- **Dimension:** Temporal oracle validity, Contract Shape completeness, and mutation sensitivity
- **Locations:**
  - `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:912-1015`
  - especially `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:983-992`
  - production order under test: `crates/overdrive-worker/src/vm_driver.rs:698-710`

**Evidence:** The test correctly arranges a guest-authored exit report, proves the actual exit receiver remains gated before cancellation, cancels a genuinely backpressured release, and then receives the real `ExitEvent`. It does not, however, observe the interval while VMM termination is still incomplete. It first awaits `exit_rx.recv()` and only afterward checks `!sim.is_live(pid)`.

That ordering of observations cannot prove the ordering of effects. A mutant that moves `state.acknowledge_active(outcome)` immediately before `state.vmm.terminate(...).await` opens the gate first. `SimVmm::terminate` can then complete in the same writer poll before Tokio schedules the exit watcher or the test task. By the time the test receives the already-authorized event and samples liveness, the VMM is dead, so lines 983-992 still pass despite the forbidden gate-before-termination edge. The test therefore does not close the temporal complement promised by its `bounded-change` declaration and does not kill the exact regression iteration 2 required it to prove.

**Required remediation:** Keep the production ordering, but make the existing forced-cancellation test causally discriminating. Wrap `SimVmm` in a test `Vmm` decorator whose `terminate` signals entry and then awaits a reviewer-visible release latch. After aborting the release and observing `terminate` entered, assert that the actual `exit_rx` remains empty for the complete interval while termination is held. Release the latch, await termination completion, then require exactly one expected event and the existing EOF/no-EXEC/supervision complement. This preserves the five-test budget and fails deterministically if the gate is dropped or acknowledged before termination completes.

### External validity — iteration 3

**Q9 metal status: PASS. Overall review status: FAIL on the cancellation regression oracle.**

The metal scenario boots the real KVM guest via production `serve`/deploy, runs the guest binary whose first workload network action resolves and dials the mesh name, observes the actual nft ruleset, compares genuine kernel packet timestamps on the exact host-veth/guest/mesh tuple, and retains exact peer TLS 1.3/kTLS evidence. Iteration-2 D3's queued-frame false positive is closed. The only remaining defect is the focused async termination/gate proof described above.

### Verification — iteration 3

| Verification | Result |
|---|---|
| `git diff --check 93539c74..afbd2414` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused worker release/gate/backpressure tests | PASS — 4 passed, 79 skipped in 0.509 s |
| Focused control-plane awaited-release/fail-closed tests | PASS — 3 passed, 202 skipped in 0.338 s |
| Metal S-GTI-02 | PASS — 1 passed, 141 skipped in 31.703 s |
| Post-metal residue | PASS — no guest netns; only the two shared `ovd-veth-*` links; no Cloud Hypervisor process; nft table contains only the two shared mark-exemption rules |
| Lima OpenAPI check | Expected known pre-existing failure — first divergence near `/v1/workloads/{id}/stop` (`workload_addr` live vs `workload_id` on disk); this remediation changes no API/OpenAPI file |
| Beacon enum/parser diff | PASS — empty from original step parent through remediation |
| D6 fresh/restart audit | PASS — exactly two existing `start_alloc(&spec)` sites, both before awaited release |
| Mutation testing | NOT RUN — repository DELIVER rules prohibit mutation testing during an individual roadmap step |

### Quality gates — iteration 3

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | S-GTI-02 remains live on its required metal surface. |
| G2 — valid RED | PASS | The iteration-3 RED event is executed/pass and precedes GREEN. |
| G3 — assertion failure quality | **FAIL** | D1's test does not deterministically fail for gate-before-termination. |
| G4 — no domain mocks/test-only release | PASS | Metal uses production composition; the focused VMM double is a driven-port integration double and the production release path is used. |
| G5 — business language | PASS | Exact anchors and behavior-oriented test names pass. |
| G6 — in-scope executed tests green | PASS mechanically | All affected/focused/metal tests pass; green does not cure D1's false-positive ordering window. |
| G7 — green before commit | PASS | GREEN precedes COMMIT and the commit timestamp. |
| G8 — test budget | PASS | 5 ≤ 8. |
| G9 — no prohibited test weakening | PASS | All test changes are additive or stricter. |

### Test integrity and RPP scan — iteration 3

- **Test modification detected:** No prohibited modification.
- **Testing theater detected:** One scheduling-dependent temporal oracle, D1. It is not fully mocked or assertion-free, but its primary termination-before-event assertion survives the forbidden reorder.
- **Escalation:** Not applicable; the repository overrides the skill's two-iteration cap and requires continued remediation.
- **RPP levels scanned:** L1-L4. No independent smell is reported. The extra writer state is justified by single ownership, stop/cancellation, and gate-drop ordering.
- **External validity:** PASS for the metal Q9 criterion; FAIL for the cancellation-order regression contract.

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **1** |

### Iteration-3 remediation disposition

| Finding | Status | Owner/action |
|---|---|---|
| D1 — gate/fail-closed cancellation ownership | **PRODUCTION RESOLVED / TEST OPEN** | Original step-02-02 crafter: hold `Vmm::terminate` pending in the forced-cancellation test and prove the actual exit receiver remains gated throughout that interval. |
| D2 — exact guest escape wire | RESOLVED | Retain exact ifindex/source/destination correlation and non-vacuity. |
| D3 — dequeue-time ordering false positive | RESOLVED | Retain `SO_TIMESTAMPNS`/`SCM_TIMESTAMPNS`, same-clock post-query barrier, and conservative unknown/equal handling. |
| D4 — declarations/preservation | **OPEN only with D1** | Mechanical syntax is correct; make the cancellation test's declared temporal complement causally observable. |
| D5 — stop totality under backpressure | RESOLVED | Retain out-of-band stop, deadline ownership, and forced backpressure coverage. |

No finding is waived or deferred. The same crafter/reviewer pair must continue the uncapped remediation cycle until the reviewer returns `APPROVED`.

### Iteration-3 final verdict

**NEEDS_REVISION**

Step 02-02 must not advance. The kernel-time oracle and production gate ownership/order are corrected, but the mandated forced-cancellation regression test can still pass when the exit gate opens before VMM termination; one deterministic temporal-oracle remediation remains.

---

## Iteration 4

### Remediation reviewed

- **Review ID:** `code_rev_20260828_200852_iteration_4`
- **Remediation commit:** `144a973ce93e750b2cab264ef7e0225f3c66a5ea`
- **Parent:** `afbd2414785111d0a33ef0d16aa80869503ade22`
- **Subject:** `test(guest-stack-transparent-mtls-intercept): prove cancellation gate order`
- **Trailer:** `Step-Id: 02-02`
- **Verdict:** **NEEDS_REVISION**

### Iteration-3 disposition

| Finding | Status | Iteration-4 evidence |
|---|---|---|
| D1 — cancellation gate/termination ordering | **RESOLVED** | `HoldsFirstTermination` intercepts the real driven `Vmm::terminate` call, signals entry, holds the future pending, delegates to `SimVmm`, and signals completion after delegation returns. The test consumes the actual driver exit receiver and proves it remains empty while the VMM is still live and termination is held. Releasing the latch precedes completion, dead-VMM observation, and the expected exit event. The explicit gate-before-termination reorder therefore fails during the held interval. |
| D2 — exact guest escape wire | **RESOLVED / RETAINED** | No production or metal-test file changed; the exact host-veth ifindex, guest source, mesh destination, plaintext boundary, and peer TLS/kTLS proof from iteration 3 remain effective. |
| D3 — dequeue-time ordering false positive | **RESOLVED / RETAINED** | No metal-test change; genuine `SCM_TIMESTAMPNS`, same-domain post-query readiness, and conservative missing/equal handling remain effective. |
| D4 — declarations/preservation | **PARTIALLY RESOLVED / OPEN** | The exact anchor/declaration and termination-held complement are honest. The newly added exactly-one event assertion accepts a timeout, leaving an unbounded event-stream tail; see D4 below. |
| D5 — stop totality under backpressure | **RESOLVED / RETAINED** | No production change; the out-of-band stop/deadline/writer behavior and forced-backpressure coverage remain intact. |

### Contract Shape Compliance — iteration 4

**Overall: FAIL only on the exactly-one event complement. Exact mechanical declarations pass.**

| Check | Status | Evidence |
|---|---|---|
| Per-test `CONTRACT_SHAPE` declaration | PASS | All five step-owned live tests retain exact declarations. |
| Exact Outcome Anchor | PASS | All five retain the exact `Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-owned test matches the banned implementation-detail pattern. |
| Termination-before-event bounded change | PASS | The latch defines a causally controlled interval: termination has entered, the real VMM remains live, and the actual exit receiver remains empty until the test releases termination. |
| Exactly-one event complement | **FAIL** | `matches!(after_expected_event, Err(_) | Ok(None))` treats a 100 ms timeout as equivalent to structural stream completion. |
| S-GTI-02 unbounded preservation | PASS by unchanged applicable evidence | The production/metal surfaces are identical to iteration 3, whose genuine metal run exercised exact kernel-time/wire/TLS/kTLS evidence. |

The repository-mandated Contract Shape checker remains absent from the checkout and installed nWave tree. Mechanical checks are direct source audits; semantic failure is the incomplete event-stream complement above.

### Mechanical evidence — iteration 4

#### Commit scope

- Remediation stat: **2 files changed, 126 insertions, 8 deletions**.
- The only code change is the focused worker acceptance test and its driven-port latch helper; the other change appends the DES events.
- There is **no production-code change** in this remediation.
- `git diff --check afbd2414..144a973c` and `cargo fmt --all -- --check` pass.
- Commit parent, subject, and `Step-Id: 02-02` trailer are exact.
- No Cargo manifest, action shim, worker production source, OpenAPI, mutation exclusion, or beacon protocol file changed.
- Dirty user-owned `.claude/rules/rust.md`, `roadmap.json`, `AGENTS.md`, and prior review artifacts were preserved. This reviewer changed only this review artifact.

#### Effective production and protocol audit

| Claim | Result | Evidence |
|---|---|---|
| Cancellation gate ownership/order | PASS | Effective `run_beacon_writer` still closes the socket, awaits cancellation termination, and only then acknowledges/drops the command-owned gate. |
| Live-caller write failure | PASS | The effective hook still awaits VMM termination before sending its returned gate. |
| Existing awaited API only | PASS | Every production release call remains awaited; no second hook, runtime lookup, detached release, or hidden async path exists. |
| Guest-initiated direction | PASS | The accepted guest-opened beacon session remains the only EXEC transport; no host-to-guest connect exists. |
| Frozen Published Language | PASS | The diff from the original step parent through iteration 4 remains empty for `crates/overdrive-core/src/vm/beacon.rs`. |
| D6 fresh/restart placement | PASS | Exactly two `start_alloc(&spec)` calls remain in the existing Running arms, each before the awaited release. |
| Install-error fail-closed behavior | PASS | The install-error arms still stop/return before the release hook. |
| D2/D3 metal oracle | PASS unchanged | Exact wire correlation and kernel timestamp logic are untouched by this remediation. |
| D5 stop totality | PASS unchanged | Writer ownership, out-of-band stop, deadline start, and escalation code are untouched. |

#### DES phase order

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T20:02:14Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T20:04:27Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T20:04:32Z` |

The fourth cycle is complete, successful, chronological, and precedes the commit timestamp (`2026-08-28T20:04:37Z`). The controlled latch makes the RED reorder causally discriminating: if acknowledgement/gate drop occurs before the pending termination, the already-ready exit watcher delivers during the held interval and the empty-receiver assertion fails.

#### Test budget and integrity

| Roadmap behaviors | Budget (`2 × behaviors`) | Step-owned live tests | Status |
|---:|---:|---:|---|
| 4 | 8 | 5 | PASS |

No test was added, removed, skipped, or weakened. The existing cancellation test was strengthened with actual termination-entry/completion signals, live-state observation, and a held-interval negative assertion. A test-only remediation is appropriate here because iteration 3 found the production ordering correct and only its regression oracle incomplete; the explicit bad reorder makes the test RED without fixture-authored business behavior.

### Blocking finding — iteration 4

#### D4 — the exactly-one event assertion accepts an unobserved tail

- **Severity:** Blocker
- **Dimension:** Contract Shape completeness and testing-theater resistance
- **Location:** `crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs:1078-1089`

**Evidence:** The test receives and validates one `ExitEvent`, then calls `timeout(100 ms, exit_rx.recv())`. Its assertion accepts both `Ok(None)` and `Err(_)`. `Ok(None)` is structural proof that the stream is closed with no second event; `Err(_)` proves only that no second event arrived during one sampled interval. Because the driver still owns the channel sender at this point, the correct run normally succeeds through the timeout arm, not the stream-completion arm.

A second event delayed beyond 100 ms therefore passes. This contradicts both the docstring's declared “exactly one guest-authored exit event” delta and the review requirement to receive exactly one event without an unbounded timing loophole. The test has converted an unbounded complement into one enumerated time sample.

**Required remediation:** After validating EOF/no-EXEC and the final supervision delta, release supervision, drop the last `VmDriver` owner (and any other known sender owner), then await the actual exit channel to close. Use a timeout only as a failing safety bound: require `Ok(None)` and reject `Err(elapsed)` or `Ok(Some(second))`. This proves the complete stream contains exactly the one validated event without adding another test or exceeding the budget.

### External validity — iteration 4

**Q9 metal status: PASS by applicable unchanged evidence. Overall review status: FAIL on D4.**

The iteration-3 metal run remains directly applicable because iteration 4 changes only `vm_driver_stop_totality.rs` and the DES log. No production, CLI metal scenario, nft decoder, capture code, protocol, or configuration changed. That genuine run passed S-GTI-02 with the real KVM/guest-init/first-by-name-dial/nft/AF_PACKET/TLS/kTLS path and left clean residue. The current focused test also drives the real VM driver and actual exit receiver; only its post-event stream-completion proof remains incomplete.

### Verification — iteration 4

| Verification | Result |
|---|---|
| `git diff --check afbd2414..144a973c` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima worker `cargo check --all-targets --features integration-tests` | PASS |
| Lima worker `cargo clippy --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused worker release/gate/backpressure tests | PASS — 4 passed, 79 skipped in 0.562 s |
| Iteration-3 genuine metal S-GTI-02 | APPLICABLE PASS — production and metal scenario unchanged; 1 passed, 141 skipped in 31.703 s |
| Iteration-3 post-metal residue | APPLICABLE PASS — no relevant implementation/configuration change |
| OpenAPI | Not rerun; known pre-existing drift is untouched and this remediation changes no API/OpenAPI source |
| Beacon enum/parser diff | PASS — empty from original step parent through iteration 4 |
| D6 fresh/restart audit | PASS — exactly two existing sites, both before awaited release |
| Mutation testing | NOT RUN — repository DELIVER rules prohibit it during an individual roadmap step |

### Quality gates — iteration 4

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | The same focused cancellation behavior remains selected; S-GTI-02 remains live on metal. |
| G2 — valid RED | PASS | The fourth RED event is executed/pass; the latch structure deterministically exposes the explicit bad reorder. |
| G3 — assertion failure quality | **FAIL only for exact-one tail** | Gate-before-termination is now killed, but a delayed duplicate event can survive the 100 ms complement sample. |
| G4 — no domain mocks/test-only release | PASS | `HoldsFirstTermination` is a driven-port VMM decorator; the production VM driver, writer, gate, and exit receiver are exercised unchanged. |
| G5 — business language | PASS | Exact declarations and behavior-oriented names pass. |
| G6 — in-scope tests green | PASS mechanically | All focused tests pass; green does not close D4's unobserved tail. |
| G7 — green before commit | PASS | GREEN precedes COMMIT and the commit timestamp. |
| G8 — test budget | PASS | 5 ≤ 8. |
| G9 — no prohibited test weakening | PASS | Changes are additive/strengthening only. |

### Test integrity and RPP scan — iteration 4

- **Test modification detected:** No prohibited modification.
- **Testing theater detected:** One incomplete complement, D4. The core gate-order assertion is now causally sound; only the exactly-one claim can still pass with a delayed duplicate.
- **Escalation:** Not applicable; repository rules require continued uncapped remediation.
- **RPP levels scanned:** L1-L4. No independent smell is reported. The small test-only decorator is justified by the temporal driven-port boundary and has no production footprint.
- **External validity:** PASS for Q9 and termination-before-event; FAIL only for exact event-stream cardinality.

### Iteration-4 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **1** |

### Iteration-4 remediation disposition

| Finding | Status | Owner/action |
|---|---|---|
| D1 — cancellation gate/termination ordering | RESOLVED | Retain the controlled termination latch and actual exit-receiver held-interval assertion. |
| D2 — exact guest escape wire | RESOLVED | Retain exact tuple/non-vacuity/TLS/kTLS evidence. |
| D3 — kernel packet-event ordering | RESOLVED | Retain genuine timestamps and conservative barrier semantics. |
| D4 — exactly-one event complement | OPEN | Original step-02-02 crafter: close the sender universe and require receiver EOF; do not accept timeout as success. |
| D5 — stop totality | RESOLVED | Retain out-of-band bounded stop coverage. |

No finding is waived or deferred. Continue the same step-specific remediation/re-review cycle until zero defects remain.

### Iteration-4 final verdict

**NEEDS_REVISION**

Step 02-02 must not advance. Termination-before-event is now causally proven and all production/metal contracts remain sound, but the test's exactly-one event complement still accepts an unobserved tail after 100 ms.

---

## Iteration 5

### Remediation reviewed

- **Review ID:** `code_rev_20260828_201649_iteration_5`
- **Remediation commit:** `d1d8f76fd5fee2725489dd9bb263005bfd182257`
- **Parent:** `144a973ce93e750b2cab264ef7e0225f3c66a5ea`
- **Subject:** `test(guest-stack-transparent-mtls-intercept): close cancellation event stream`
- **Trailer:** `Step-Id: 02-02`
- **Verdict:** **APPROVED**

### Iteration-4 disposition

| Finding | Status | Iteration-5 evidence |
|---|---|---|
| D1 — cancellation gate/termination ordering | **RESOLVED / RETAINED** | The controlled driven-port termination latch is unchanged. The test still observes actual termination entry, keeps `SimVmm` live while termination is held, requires the actual exit receiver to remain empty throughout that interval, then releases and awaits real termination completion before accepting the event. |
| D2 — exact guest escape wire | **RESOLVED / RETAINED** | No production or metal-test file changed. The exact host-veth ifindex, guest source, mesh destination, plaintext-boundary non-vacuity, and peer TLS/kTLS evidence from iteration 3 remain effective. |
| D3 — kernel packet-event ordering | **RESOLVED / RETAINED** | No metal-test file changed. Genuine `SO_TIMESTAMPNS`/`SCM_TIMESTAMPNS`, the `CLOCK_REALTIME` post-query barrier, and conservative missing/equal timestamp handling remain effective. |
| D4 — exactly-one event complement | **RESOLVED** | The permissive `Err(_) | Ok(None)` 100 ms sample is removed. After EOF/no EXEC and `EndingInFlight -> empty` supervision cleanup, the test drops the final local driver owner and calls `require_closed_exit_stream`; only `Ok(None)` succeeds. A timeout/leaked sender or any second event fails. |
| D5 — stop totality under backpressure | **RESOLVED / RETAINED** | No production change. Out-of-band stop, deadline ownership, writer cancellation, and forced-backpressure coverage remain intact and pass in the focused Lima run. |

### Contract Shape Compliance — iteration 5

**Overall: PASS.**

| Check | Status | Evidence |
|---|---|---|
| Per-test `CONTRACT_SHAPE` declaration | PASS | All five step-owned live tests retain their exact declarations; the remediated cancellation property remains `/// CONTRACT_SHAPE: bounded-change.`. |
| Exact Outcome Anchor | PASS | All five retain the exact `/// Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-owned test uses the prohibited implementation-detail naming shape. |
| Termination-before-event bounded change | PASS | The explicit latch closes the complete controlled interval from real termination entry through completion; the actual event receiver is empty while the VMM remains live. |
| Exactly-one event complement | PASS | The receiver must reach structural EOF after known owners are closed. `Err(Elapsed)` and `Ok(Some(second_event))` both fail; only `Ok(None)` passes. |
| Complete guest-session complement | PASS | The guest is read through EOF and every complete line is parsed to prove no `BeaconMessage::Exec` exists after cancellation. |
| Supervision complement | PASS | The test observes `EndingInFlight`, calls `release_supervision`, and requires the complete live-allocation snapshot to be empty before closing the event universe. |
| S-GTI-02 unbounded preservation | PASS by unchanged applicable evidence | The production and metal surfaces are identical to iteration 3, whose genuine metal run exercised the exact kernel-time/wire/TLS/kTLS evidence. |

The repository-mandated Contract Shape checker remains absent from the checkout and installed nWave tree. Mechanical declarations were therefore audited directly in source; the semantic complement is now structurally closed.

### Mechanical evidence — iteration 5

#### Commit scope

- Remediation stat: **2 files changed, 34 insertions, 7 deletions**.
- Code delta: `vm_driver_stop_totality.rs`, **13 insertions and 7 deletions**. The only behavior change is to the focused acceptance oracle plus its test helper.
- Log delta: `execution-log.json`, **21 insertions** for the fifth RED/GREEN/COMMIT cycle.
- There is **no production, metal-test, Cargo manifest, OpenAPI, mutation-exclusion, action-shim, or beacon-protocol change** in the remediation.
- `git diff --check 144a973c..d1d8f76f` and `cargo fmt --all -- --check` pass.
- Parent, subject, and `Step-Id: 02-02` trailer are exact.
- Dirty user-owned `.claude/rules/rust.md`, `roadmap.json`, `AGENTS.md`, and prior review artifacts were preserved. This reviewer changed only this required review artifact.

#### Event sender/owner closure audit

| Owner/path | Closure proof | Result |
|---|---|---|
| Release-task `VmDriver` clone | `release_task.abort()` is followed by awaiting the join error and requiring cancellation, so `driver_for_release` is dropped before the final event assertion. | PASS |
| Local `VmDriver` sender | After EOF/no EXEC and final supervision cleanup, explicit `drop(driver)` removes the driver's `exit_tx`. | PASS |
| Exit-watcher sender clone | `run_exit_watcher` owns the only per-allocation cloned sender. It sends the one validated event and returns; receiver EOF proves that task-owned sender has actually dropped. | PASS |
| Writer/termination path | The beacon writer and driven `Vmm` decorator do not own an exit sender. Their lifecycle is nevertheless closed by cancellation transfer, observed termination completion, and guest EOF. | PASS |
| Actual receiver outcome | `require_closed_exit_stream` calls the real `mpsc::Receiver::recv` behind a one-second failing safety bound and accepts only `Ok(None)`. | PASS |
| Duplicate/delayed event | An already queued or subsequently sent duplicate yields `Ok(Some(_))`; a producer that remains live without sending yields `Err(Elapsed)`. Both violate the matcher and fail. | PASS |

This is a structural proof, not a silence sample. Once `recv()` returns `None`, Tokio's channel contract establishes that every sender has been dropped and no later event can arrive.

#### Effective production, protocol, and D6 audit

| Claim | Result | Evidence |
|---|---|---|
| Cancellation gate ownership/order | PASS | Effective `run_beacon_writer` still closes the socket, awaits fail-closed VMM termination, and only then acknowledges/drops the command-owned gate. |
| Live-caller write failure | PASS | The effective release hook still awaits VMM termination before sending its returned gate on write failure. |
| Existing awaited API only | PASS | Every production release call remains awaited; no second hook, runtime lookup, detached release, or hidden async path was introduced. |
| Guest-initiated direction | PASS | The accepted guest-opened beacon session remains the only EXEC transport; no host-to-guest connection exists. |
| Frozen Published Language | PASS | `crates/overdrive-core/src/vm/beacon.rs` has an empty diff from the original step parent through iteration 5. |
| D6 fresh/restart placement | PASS | Exactly two `start_alloc(&spec)` calls remain in `action_shim/mod.rs` (fresh at line 1730 and restart at line 2037), each before the awaited release at lines 1755 and 2060. |
| Install-error fail closed | PASS | Both install-error arms still stop/return before release. |
| D2/D3 metal oracle | PASS unchanged | Exact tuple correlation, non-vacuity, TLS/kTLS proof, and genuine kernel timestamps are untouched. |
| D5 stop totality | PASS unchanged | Writer ownership, out-of-band stop, deadline start, escalation, and their forced-backpressure test are untouched. |

#### DES phase order

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T20:12:27Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T20:13:46Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T20:13:46Z` |

The fifth cycle is complete, successful, chronological, and precedes the commit timestamp (`2026-08-28T20:13:52Z`). RED is genuine for the prior defect: the driver itself owns an `exit_tx`, so requiring receiver EOF while retaining `driver` necessarily times out and fails. GREEN adds explicit closure of that owner after every lifecycle assertion. Removing `drop(driver)` from the committed form recreates the deterministic failure; no timing or implementation-authored business result is needed to make RED discriminate.

#### Test budget and integrity

| Roadmap behaviors | Budget (`2 × behaviors`) | Step-owned live tests | Status |
|---:|---:|---:|---|
| 4 | 8 | 5 | PASS |

- No test was added, removed, skipped, renamed, or narrowed.
- The weak post-event assertion was replaced, not merely supplemented: timeout changed from an accepted outcome into an assertion failure.
- Exact event fields (`alloc`, `CleanExit`), termination ordering, EOF/no EXEC, and full supervision state remain asserted before structural stream closure.
- The small helper centralizes one test assertion and introduces no fixture-authored domain behavior.
- Test-only remediation is appropriate because iteration 4 established that production behavior was correct and only the completeness of its regression oracle remained defective.

### Findings — iteration 5

No blocker, critical, high, medium, or low findings.

### External validity — iteration 5

**PASS.** The iteration-3 metal evidence remains applicable because iterations 4 and 5 change only the focused worker acceptance test and DES log. No production, CLI metal scenario, nft decoder, AF_PACKET capture, protocol, or configuration surface changed. That genuine run passed S-GTI-02 through real KVM boot, guest init, first by-name mesh dial, installed nft rules, exact host-veth guest/mesh tuple capture, kernel packet timestamps, peer TLS 1.3/kTLS verification, and clean post-test residue. Iteration 5 independently closes the focused cancellation event universe through the actual `VmDriver` receiver.

### Verification — iteration 5

| Verification | Result |
|---|---|
| `git diff --check 144a973c..d1d8f76f` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima worker `cargo check --all-targets --features integration-tests` | PASS |
| Lima worker `cargo clippy --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused worker release/gate/backpressure tests | PASS — 4 passed, 79 skipped in 0.508 s |
| Iteration-3 genuine metal S-GTI-02 | APPLICABLE PASS — production and metal scenario unchanged; 1 passed, 141 skipped in 31.703 s |
| Iteration-3 post-metal residue | APPLICABLE PASS — no relevant implementation/configuration change |
| OpenAPI | Not rerun; known pre-existing drift is untouched and this remediation changes no API/OpenAPI source |
| Beacon enum/parser diff | PASS — empty from original step parent through iteration 5 |
| D6 fresh/restart audit | PASS — exactly two existing sites, both before awaited release |
| Mutation testing | NOT RUN — repository DELIVER rules prohibit mutation testing during an individual roadmap step |

### Quality gates — iteration 5

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | The focused cancellation behavior remains selected; S-GTI-02 remains live on metal. |
| G2 — valid RED | PASS | The fifth RED event is executed/pass and precedes GREEN; retaining the driver's sender makes mandatory receiver EOF fail deterministically. |
| G3 — assertion failure quality | PASS | Leaked ownership, timeout, duplicate event, premature gate release, absent expected event, residual EXEC, and incorrect supervision each have distinct failing assertions. |
| G4 — no domain mocks/test-only release | PASS | `HoldsFirstTermination` is a driven-port VMM decorator; production VM driver, writer, gate, sender topology, and actual receiver are exercised. |
| G5 — business language | PASS | Exact declarations and behavior-oriented test names pass. |
| G6 — in-scope tests green | PASS | All four focused tests pass in Lima. |
| G7 — green before commit | PASS | GREEN/COMMIT precede the commit timestamp. |
| G8 — test budget | PASS | 5 ≤ 8. |
| G9 — no prohibited test weakening | PASS | The only test change strictly strengthens the post-event complement. |

### Test integrity and RPP scan — iteration 5

- **Test modification detected:** No prohibited modification. The prior permissive timeout branch was deleted and replaced by exact structural closure.
- **Testing theater detected:** None. The receiver is the production driver's actual event channel; closure is determined by real sender ownership, not a fake event list or sampled quiet interval.
- **Escalation:** Not applicable. The uncapped repository cycle reached zero defects.
- **RPP levels scanned:** L1-L4. No independent smell remains. The helper is a small, behavior-specific assertion abstraction and the explicit driver drop is the lifecycle action required to close the observable universe.
- **External validity:** PASS for both the genuine Q9 metal boundary and the focused cancellation/termination/event-order regression contract.

### Iteration-5 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **0** |

### Iteration-5 remediation disposition

| Finding | Status | Disposition |
|---|---|---|
| D1 — cancellation gate/termination ordering | RESOLVED | Controlled held-termination proof retained. |
| D2 — exact guest escape wire | RESOLVED | Exact tuple/non-vacuity/TLS/kTLS evidence retained. |
| D3 — kernel packet-event ordering | RESOLVED | Genuine timestamps and conservative barrier semantics retained. |
| D4 — exactly-one event complement | RESOLVED | All sender owners are closed and the actual receiver must return `None`; timeout and duplicate event fail. |
| D5 — stop totality | RESOLVED | Out-of-band bounded stop behavior and coverage retained. |

No finding is waived, deferred, or left open.

### Iteration-5 final verdict

**APPROVED**

Step 02-02 may advance. The final event cardinality claim is structurally complete, all earlier correctness and external-validity remediations remain effective, the affected verification is green, and the review has zero defects at every severity.
