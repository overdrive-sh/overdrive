# DELIVER review — 02-09 Order same-ID replacement teardown

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Roadmap step | `02-09` |
| Iteration | 1 |
| Commit reviewed | `51d67a0399a4daf7cb62b0189053155b6d811c1a` (`fix(action-shim): order same-id replacement teardown`) |
| Commit trailer | `Step-Id: 02-09` |
| Review sources | `feature-delta.md` BTR-03; architecture brief BTR extension; DISTILL `S-GTI-BTR-03`; `deliver/roadmap.json` step `02-09` |
| Verdict | **NEEDS REMEDIATION** |

## Accepted contract checked

For a same-ID replacement, the action must await prior driver stops (where
only `NotFound` means absence), then mTLS teardown, then the allocation
network teardown and slot release, before replacement provision, identity
injection, and driver start. Any error at the driver, mTLS, or network cut
must stop the later stages. A post-assignment provision failure continues to
use the existing BTR-02 raw structural unwind before writing `Failed`, with
store error taking precedence over cleanup error. The accepted design
expressly excludes a `RestartNetworkDisposition`, a second restart-cleanup
protocol, detached cleanup, and new public or test-only production seams.

This is binding in the feature delta (`feature-delta.md:1534-1690`),
architecture brief (`docs/product/architecture/brief.md:10326-10353`), and
DISTILL scenario S-GTI-BTR-03 (`distill/test-scenarios.md:127-183`).

## What is correct

The changed restart action first resolves and awaits every prior driver stop,
returning a non-`NotFound` `DriverError` immediately
(`action_shim/mod.rs:2164-2174`). It then awaits the existing private
`cleanup_restart_abort` before calling replacement provision
(`:2176-2202`). That helper awaits `MtlsInterceptWorker::stop_alloc` and then
the existing allocation-keyed raw network teardown/release (`:1345-1360`;
`:1317-1333`). `stop_alloc` itself joins its enforcement work before it
returns (`mtls_intercept_worker.rs:992-1047`, `:1524-1544`), so this is an
awaited prior-protection boundary rather than detached work.

The later post-assignment failure branch still captures only the raw
structural teardown, writes the existing `Failed` disposition afterward, and
returns store error before cleanup error (`action_shim/mod.rs:2202-2249`).
Only a successful provision reaches identity injection and driver start
(`:2252-2285`). The commit introduces no public production API,
`RestartNetworkDisposition`, or second cleanup protocol.

## Findings

### F-01 — BTR-03 test performs real socket I/O in the default in-memory acceptance lane

**Severity:** High — blocking test-boundary and scenario-classification
violation.

The new scenario is unconditionally compiled into the default `acceptance`
test binary (`crates/overdrive-control-plane/tests/acceptance.rs:248`), whose
header declares that lane to be default-feature and free of real
infrastructure (`:12-14`). Its exercised `drive_same_id_replacement` fixture
creates an actual `std::net::TcpListener` through `RecordingIntercept`
(`action_shim_crash_observability.rs:2255-2295`, `:2502-2512`) and makes a real
`std::net::TcpStream::connect` to it (`:2512`). The test itself is therefore
not `@in-memory`, despite the S-GTI-BTR-03 scenario tag and the test's current
default-lane placement.

This is reachable in the current test execution path, not static suspicion:
the default command below loaded the unconditional acceptance module and ran
`same_id_restart_removes_prior_protection_before_replacement_provision`, which
called that fixture and performed the bind/connect sequence. Repository test
rules require real network activity, including socket binds and localhost
connections, to be gated behind `integration-tests`; the default lane may use
only in-memory doubles. This finding concerns the test's actual execution
boundary, not a claim of a production runtime defect.

**Required bounded remediation:** place or gate this one BTR-03 scenario and
its real-socket fixture in the existing `integration-tests` lane. Preserve its
production `dispatch_with_network_provisioner` driving port, exact
`/// CONTRACT_SHAPE: bounded-change.` declaration, and current driver/mTLS/
network/replacement trace assertions. Do not change production code, add a
public or test-only production seam, or alter the accepted replacement
protocol.

### No additional findings

Aside from its execution lane, the new test is a genuine action-shim
composition test rather than a parallel reimplementation. It drives the
supported production dispatcher with the existing network-provisioner port
(`action_shim_crash_observability.rs:2559`) and partitions non-`NotFound`
driver failure, mTLS failure, structural network failure, `NotFound`, and full
success (`:2614-2720`). Its deletion-sensitive order oracle proves all prior
driver stops precede mTLS rule removal, mTLS teardown completion precedes
network teardown/release, and those precede replacement provision, identity,
and driver start. The smallest-slot assertion proves the old allocation's slot
was released before its replacement is provisioned.

The pre-existing BTR-02 test remains the appropriate evidence for the
post-assignment failure sequence and error precedence
(`action_shim_crash_observability.rs:2141-2234`).

## Verification evidence

| Check | Result |
|---|---|
| `git show --check 51d67a0399a4daf7cb62b0189053155b6d811c1a` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 test passed in the default lane (and establishes F-01 execution reachability). |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(post_assignment_provision_failure_tears_down_before_slot_release) + test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 2 tests passed. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 test passed; the scenario remains valid when correctly feature-gated. |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --test acceptance -- -D warnings` | Pass. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/guest-stack-transparent-mtls-intercept/deliver/` | Pass — all 12 steps have complete DES traces. |
| Commit scope | Pass — only the roadmap action-shim production file and acceptance-test evidence changed. |

## Remediation disposition

Iteration 1 requires the original 02-09 crafter to resolve F-01 with the
bounded test-lane correction above. Re-review must confirm that default
feature acceptance tests no longer bind or connect sockets, while the
`integration-tests` run preserves the complete BTR-03 ordering and
error-partition evidence.

## Final verdict

**NEEDS REMEDIATION.** The production implementation conforms to the BTR-03
ordering and bounded API contract, but its required regression test is placed
in the wrong execution tier and currently violates the repository's
in-memory/default-lane rule.

---

## Iteration 2 — F-01 remediation re-review

| Field | Value |
|---|---|
| Remediation commit reviewed | `f4bb63deb4ba4fe687153d8dd1559fcf5ab9110e` (`test(action-shim): gate same-id replacement scenario`) |
| Commit trailer | `Step-Id: 02-09` |
| Verdict | **APPROVED** |

### F-01 disposition — resolved

The remediation applies `#[cfg(feature = "integration-tests")]` to the
real-socket imports and to every BTR-03-only fixture, helper, implementation,
and test (`action_shim_crash_observability.rs:40-93`, `:2244-2646`). This
includes both actual socket entry points: `RecordingIntercept`'s
`TcpListener::bind` (`:2281-2293`) and the fixture's two
`TcpStream::connect` operations (`:2536-2541`, `:2617-2625`). Therefore the
unconditional acceptance module still compiles in the default lane, but none
of the BTR-03 socket code is compiled or reachable unless
`integration-tests` is enabled.

The default-feature selection was independently compiled and selected zero
BTR-03 tests. With `integration-tests` enabled, the same selection compiled
and passed. The latter executes the unchanged production
`dispatch_with_network_provisioner` path (`:2585-2611`), keeps the exact
`/// CONTRACT_SHAPE: bounded-change.` declaration (`:2640`), and retains the
full prior-driver, mTLS, structural-network, replacement-provision, identity,
and driver-start partitions (`:2646-2707`). No production code, public API,
test-only production seam, or replacement protocol changed.

### No additional findings

The feature gate is narrowly applied only to the real-I/O BTR-03 evidence.
It neither weakens its required error cuts nor changes BTR-02's existing
post-assignment failure evidence. The accepted production ordering remains
the one reviewed in iteration 1.

### Re-review verification

| Check | Result |
|---|---|
| `git show --check f4bb63deb4ba4fe687153d8dd1559fcf5ab9110e` | Pass. |
| `cargo xtask lima run -- cargo nextest run --no-tests pass -p overdrive-control-plane --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — default-feature acceptance compiled and selected 0 BTR-03 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 BTR-03 test passed. |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --features integration-tests --test acceptance -- -D warnings` | Pass. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/guest-stack-transparent-mtls-intercept/deliver/` | Pass — all 12 steps have complete DES traces. |
| Scope audit | Pass — the remediation changes only BTR-03 test compilation gates; it adds no production or public surface. |

### Final verdict

**APPROVED.** F-01 is resolved: real socket operations are restricted to the
`integration-tests` lane, while the feature-gated test retains the complete
S-GTI-BTR-03 production ordering and failure-cut evidence.

---

## Iteration 3 — final-gate baseline regression audit

| Field | Value |
|---|---|
| Review trigger | Qualified native mutation preflight stopped at its unmutated workspace integration baseline before testing mutants. |
| Current implementation | `51d67a0399a4daf7cb62b0189053155b6d811c1a` plus the F-01 test-tier remediation `f4bb63deb4ba4fe687153d8dd1559fcf5ab9110e` |
| Verdict | **NEEDS REMEDIATION** |

### F-02 — stale integration teardown counts reject the required two-incarnation cleanup

**Severity:** High — active integration baseline regression; bounded test
fallout from BTR-03, not a production implementation defect.

The three failing tests share `drive_restart_abort`, which first creates a
live prior intercept and assigns a structural slot (`mtls_install_fail_closed.rs:1108-1149`),
then invokes the real supported `dispatch_with_network_provisioner` action
composition (`:1159-1183`). Its `RestartAbortNetwork` increments
`teardowns` for every call to the existing C3 teardown port (`:1047-1086`).
The three oracles still demand one call (`:1213-1218`, `:1229-1237`,
`:1245-1253`), but the current non-mutated production path necessarily makes
two calls.

**Reproduction:**

```text
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
  --features integration-tests --test integration \
  -E 'test(restart_provision_failure_tears_down_replacement_network_and_releases_the_slot) + test(restart_identity_failure_stops_prior_intercept_before_network_release) + test(restart_driver_start_failure_stops_prior_intercept_before_network_release)'
```

The three tests failed against the current unmutated code, each at its
`teardowns == 1` assertion with observed `2` (0 passed, 3 failed). This
independently reproduces the qualified native preflight's failure without a
mutant or proposed remedy.

**Production reachability and order:** production `run_server` owns the
convergence loop (`lib.rs:3058-3078`), which awaits each
`run_convergence_tick` (`:3370-3388`). The tick awaits
`dispatch_with_workflow_intent` (`reconciler_runtime.rs:1711-1746`), which
calls the supported production `dispatch` (`action_shim/mod.rs:1010-1042`)
and ultimately the same sequential `dispatch_single` used by the integration
test. Graceful shutdown waits for an active dispatch to finish
(`lib.rs:1410-1417`); this is not a detached-tail or cancellation-only trace.

For a reachable non-terminal `RestartAllocation`, the exact sequence is:

1. After the prior driver stop, BTR-03 awaits `cleanup_restart_abort`
   (`action_shim/mod.rs:2164-2177`). Its first `stop_alloc` removes and drains
   the live prior intercept; its raw teardown then destroys the old structural
   network and releases the old allocation slot (`:1345-1360`; worker
   `:992-1047`). This is teardown call **one**.
2. `provision_and_inject_netns` then assigns the now-free slot and invokes the
   replacement provisioner (`:1113-1148`, `:2202-2207`). The numeric slot may
   be reused, but this is a new replacement assignment after the old owner has
   been released.
3. On a provision error, the BTR-02 branch invokes the raw teardown for that
   failed replacement assignment before its Failed write (`:2211-2249`). On
   identity error, the later abort calls the same existing cleanup helper
   (`:2252-2280`); on driver-start error, it does likewise (`:2321-2333`).
   Each path tears down and releases the replacement assignment: teardown call
   **two**.

The second `stop_alloc` in the identity/driver-start paths does not tear down
the old mTLS owner twice. After the first stop removed the active intercept,
the worker finds the completed prior stop and only awaits it again when there
are no retry handles (`mtls_intercept_worker.rs:1002-1021`). The existing
`stop_alloc_calls == 1` assertions in the identity and driver-start cases
therefore remain the correct no-duplicate-owner oracle.

This is the accepted contract, not an accidental cleanup loop. BTR-03
requires old driver, mTLS, and structural ownership to be gone before
replacement work, and expressly says a replacement failure after assignment
reuses BTR-02's teardown/release/error-precedence contract
(`distill/test-scenarios.md:127-164`; `feature-delta.md:1547-1548,1591-1595`;
`brief.md:10342-10353`). Removing either call to make the stale count pass
would either begin replacement before old protection is gone or leak the
failed replacement's assigned structural resource.

**Required bounded remediation:** update only
`crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`.
For the provision, identity, and driver-start restart-abort cases, change the
structural-teardown oracle from one total call to two: first the required old
protection teardown, then the replacement assignment's failure cleanup.
Adjust the adjacent test comments/names only as needed to state that
two-incarnation meaning. Preserve the existing primary-error, Failed-row,
slot-release, and `stop_alloc_calls == 1` checks; do not change production
code, public API, ports, test-only production seams, or the BTR-03 protocol.

### No additional findings

The feature-gated S-GTI-BTR-03 dispatcher test remains green and still proves
the prior-driver → mTLS → structural teardown/release → replacement provision
→ identity → driver-start order. The baseline failure is confined to the
older integration fixture's stale aggregate count; it does not contradict the
accepted ordering or reveal a second cleanup protocol.

### Final-gate verification

| Check | Result |
|---|---|
| Qualified native mutation preflight | Unmutated workspace baseline stopped before mutant execution: the same three integration tests observed 2 instead of 1. |
| Scoped unmutated reproduction above | **Fail** — 0 passed, 3 failed at the three stale count assertions. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 BTR-03 ordering test passed. |
| Production caller/owner audit | Pass — all effects are awaited in the active convergence dispatch; no detached or cancellation-only path is involved. |

### Final verdict

**NEEDS REMEDIATION.** F-02 is an in-scope 02-09 test-oracle transition
caused by the required prior-cleanup-before-replacement ordering. The
production implementation must remain unchanged; the original 02-09 crafter
must make the bounded integration-test update above and re-run the three
baseline cases before re-review.

---

## Iteration 4 — F-02 remediation re-review

| Field | Value |
|---|---|
| Remediation commit reviewed | `22844034c879ecae5913f0d2bb133e9314063ad2` (`test(action-shim): update restart cleanup counts`) |
| Commit trailer | `Step-Id: 02-09` |
| Verdict | **APPROVED** |

### F-02 disposition — resolved

The remediation changes only the three stale integration-test oracles in
`mtls_install_fail_closed.rs`. The provision case is renamed and documents
the old-owner teardown followed by failed-replacement cleanup
(`:1209-1225`); it now asserts exactly two structural-teardown calls. The
identity and driver-start cases have the same exact two-call assertion and
matching explanation (`:1228-1269`). Their exact
`/// CONTRACT_SHAPE: bounded-change.` declarations remain intact.

The assertions are honest about the proven production path. The fixture starts
with both a live prior intercept and an owned slot (`:1113-1149`), and its
network port increments `teardowns` once for each actual teardown call
(`:1056-1086`). Production first awaits prior cleanup before replacement
provision (`action_shim/mod.rs:2164-2177`), then assigns/provisions the new
incarnation (`:1113-1148`). A failed provision invokes the BTR-02 raw cleanup
(`:2202-2249`); identity and driver-start failures invoke their existing later
cleanup paths (`:2252-2280`, `:2321-2333`). Thus two is the required total,
not a relaxed bound.

The remediation preserves the complementary evidence: all three retain the
primary result/row and slot-release checks; identity and driver-start retain
the prior-intercept-converged and exactly-one active `stop_alloc` assertions
(`mtls_install_fail_closed.rs:1233-1244`, `:1252-1269`). The worker makes the
later stop wait on its completed prior stop rather than removing an active
intercept again (`mtls_intercept_worker.rs:1002-1024`). No production code,
public API, port, test-only production seam, or BTR-03 protocol changed.

### No additional findings

The complete feature-gated S-GTI-BTR-03 ordering test remains green. Together
with the corrected integration cases, it covers both the strict prior-owner
cut and the required cleanup of a failure after replacement assignment.

### Re-review verification

| Check | Result |
|---|---|
| `git show --check 22844034c879ecae5913f0d2bb133e9314063ad2` | Pass. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test integration -E 'test(restart_provision_failure_tears_down_old_and_replacement_networks_and_releases_the_slot) + test(restart_identity_failure_stops_prior_intercept_before_network_release) + test(restart_driver_start_failure_stops_prior_intercept_before_network_release)'` | Pass — 3 tests passed. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 BTR-03 ordering test passed. |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --features integration-tests --test integration -- -D warnings` | Pass. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/guest-stack-transparent-mtls-intercept/deliver/` | Pass — all 12 steps have complete DES traces. |
| Mutation testing | Not run — explicitly prohibited by the user for this re-review. |
| Scope audit | Pass — only the three required test expectations and their descriptions changed. |

### Final verdict

**APPROVED.** F-02's stale baseline oracle is corrected without altering the
accepted production ordering or adding any surface. All 02-09 findings are
resolved.
