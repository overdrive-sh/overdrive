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
