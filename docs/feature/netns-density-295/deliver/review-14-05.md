# DELIVER review — STEP 14-05 existing 64-VM correctness unit

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-05` |
| Review type | Fresh isolated post-commit implementation review |
| Parent | `a2cd9691de4da949389cfe884a044ca9cc60367d` |
| Implementation range | `ba85e05c321f4f7882a7b9724a887d29c64123a9` through `8bc5ed027d541e1d9f2a597f74072d7109b93798` |
| Review iteration | 1 historical; 2 remediation re-review |
| Final verdict | **APPROVED** |
| Open findings | 0 |

This review applies the current `AGENTS.md`, the applicable project rules, the
software-crafter review dimensions and TDD enforcement skills, the complete
`.context/netns-density-295/dispatch-14-05.md`, the feature delta, accepted
ADR-0098 and the roadmap contract for P6-S15 / V-9 correctness / R5-25 /
R5-55 / NO-SURVIVE. The repository's native Markdown DELIVER convention is
used for this artifact.

The user-owned dirty `AGENTS.md` was preserved. The reviewer changed no
production source, test source, DES event, design, roadmap or host state. This
artifact is the only reviewer-owned file.

## Authority and review boundary

The accepted step activates exactly these two existing bodies in
`crates/overdrive-cli/tests/integration/vsock_density.rs`:

1. `integration::vsock_density::every_vm_at_density_round_trips_through_its_own_attachment`
2. `integration::vsock_density::serve_down_at_density_is_fail_closed_and_the_next_boot_reclaims_every_vm`

The first must prove 64 distinct live workload addresses, one TCP and one UDP
round trip per guest application, exact once-only peer observations, and the
node source address. The second must prove immediate fail-closed refusal with
`ECONNREFUSED` or `ENETUNREACH` while `serve` is up and down, no delivery to a
live host listener, unsupervised VMM destruction on the next boot, and
replacement of every persisted Job on the reclaimed CID set. U-2 excludes
latency, throughput, memory, percentile and density measurements; those belong
to GH #326.

The implementation range also contains the necessary existing GuestKit
selection/resource support and the bounded production fixes that the native
RED exposed. The independently reviewed support handoff is
`docs/feature/netns-density-295/deliver/vm-cohort-support.md`, with its
three-iteration approval in
`docs/feature/netns-density-295/deliver/review-vm-cohort-support.md`.

## `praise:` strengths

`praise:` The two exact bodies remain the sole accepted identities. The
focused runner selects one full libtest identity at a time, requires one pass
with zero ignored/runtime-SKIP bodies, reports `canonical_119_credit=0`, and
keeps the existing `cgroup-default` / `mtls-relay` manifests unchanged.

`praise:` The corrected address oracle follows ADR-0098. The private
`observe_live_workload_address` helper reads only public `Running` rows and
fails immediately if `Terminated` or `Failed` is observed first. It does not
recover or reconstruct a terminal address, so a crashed allocation cannot be
turned into address-cardinality success.

`praise:` The native receipts demonstrate the required disposable boundary.
The outer guest boots Linux 7.0.0-38 through the existing production
`CloudHypervisorVmm` GuestKit path; the 64 nested workload VMMs and their
kernel/network/cgroup effects run inside that guest. The final focused runs
record 64 cleanup completions for the round trip and 122 for the recovery
body, with zero canonical-manifest credit and no physical-host test receipt.

## Diff and exact contract audit

The complete range contains eight files:

| File | Reviewed change | Contract disposition |
|---|---|---|
| `crates/overdrive-cli/tests/integration/vsock_density.rs` | Removes the two owned pending markers; adds live-row address observation and keeps the existing body outcomes | Required test activation and approved ADR-0098 oracle correction |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | Adds exact direct selection for the two existing identities, private static resource sizing, and zero-credit result reporting | Existing GuestKit support; no public surface |
| `crates/overdrive-init/src/main.rs` | Applies one-second `SO_LINGER` to the EXIT beacon and closes the beacon before poweroff | Private lifecycle correction |
| `crates/overdrive-worker/src/vm_driver.rs` | Gives the beacon reader priority, drains the final report/EOF after VMM exit, and joins writer stop before emitting the exit event | Private async shutdown/drain correction |
| `crates/overdrive-control-plane/src/guest_network.rs` | Counts queued end records as occupied during forwarder audit | Production control-plane audit correction; subject to F-01 below |
| `docs/feature/netns-density-295/deliver/execution-log.json` | Preserves prior blocked attempts and records the actual final RED/GREEN/COMMIT events | DES audit record |
| `docs/feature/netns-density-295/deliver/vm-cohort-support.md` | Existing GuestKit support handoff | Independently reviewed support artifact |
| `docs/feature/netns-density-295/deliver/review-vm-cohort-support.md` | Existing independent support review | Approved through iteration 3 |

No public method, type, enum variant, trait, parameter, port, composition-root
signature, wire field or persisted shape was added. `DescribeArgs` / `describe`
are existing operator-facing APIs. The new runner types and helpers are
private test support. `GuestNetworkFact`, `GuestNetworkError` and
`InitError` retain their existing typed representations; the new socket option
failure uses the existing private `InitError::Io` carrier and does not invent a
semantic public variant.

The production handoff order is coherent: PID 1 writes `EXIT`, applies the
linger policy, drops the beacon socket, and only then powers off. The worker
drain races the guest report against the VMM signal, gives the writer a
bounded cooperative drain before the final line/EOF read, then requests and
awaits writer stop before clearing beacon state, cleaning driver artifacts and
emitting `ExitEvent`. The owner audit change is private and leaves the
existing typed `PostconditionMismatch` / bounded-recovery path intact.

## Acceptance and test integrity

There are two distinct acceptance behaviors and two active acceptance bodies;
the legacy unit-test budget is therefore four, with two actual bodies. The
step is explicitly exempt from the property-based unit-test default because
it is a fixed 64-VM native composition proof. No new unit test, scenario, lane,
model, seam or expectation runner was introduced.

The test diff removes only the two pending markers and applies the reviewed
live-address correction. The previous terminal-row address cardinality read
was invalid under ADR-0098; the replacement requires the same 64 addresses
while rows are `Running`, then retains all terminal, TCP/UDP, source-address,
fail-closed, PID, CID and cleanup assertions. No assertion was weakened,
deleted or skipped. Both live bodies retain:

```text
/// CONTRACT_SHAPE: bounded-change.
```

Both bodies enter through the production deploy/serve composition and observe
public rows, peer traffic, host TCP behavior and VMM process/CID state. They do
not mock the SUT or recreate production logic in a fixture.

## Native evidence ledger

The retained ledger is
`.context/netns-density-295/vm-cohort/raw/native-ledger.json`; the complete
raw transcripts are in the same directory. The final evidence is:

| Body | Durable run | Result | Raw evidence |
|---|---|---|---|
| Round trip | `48a2a4804cdb008122887ddc` | exit 0; `1 passed; 0 failed; 0 ignored`; `64` `VM cleanup calls finished`; `64` VMM reaps; `canonical_119_credit=0` | `raw/final-roundtrip.log` |
| Serve-down/reclaim | `bf76f2e6bf9a6f83dd3fe1c0` | exit 0; `1 passed; 0 failed; 0 ignored`; `122` cleanup completions; `122` VMM reaps; `canonical_119_credit=0` | `raw/final-recovery.log` |

Both final runs report the exact source/staged executable pair:

```text
CARGO_REPORTED_SOURCE_ELF=/home/ubuntu/overdrive/target/vm-cohort-native/debug/deps/integration-644109f42e418098
STAGED_ELF=target/vm-cohort-native/guest-kit/in-guest/overdrive-cli--integration
SHA256=abe4ef403f6ae03549a9aa01e645b7b5bfe787d414acc4d7573b1a069a72a547
```

The transcripts show `test -x`, `cmp -s`, matching sizes/modes and matching
SHA-256 values before either focused body starts. The focused output names both
exact identities, `source_manifest_bodies=2`, `outer_memory_mib=12821`,
`outer_vcpus=8`, and zero canonical credit. The static 12,821 MiB envelope is
compile-time checked from the production `MemoryPlan` for 64 × 128 MiB nested
workloads; it is capacity configuration, not a measured memory result.

The historical RED receipts are preserved and excluded from credit. They
include the initial `WorkloadCrashedImmediately`/missing host EXIT-report
failures and the later `Forwarder` audit mismatch diagnostic. Temporary drain,
poll/EINTR, console and guest-shutdown experiments are explicitly described as
diagnostic and are not presented as final evidence. No host-native nextest,
Lima, mutation test, KASAN, host process kill, host module/network cleanup or
canonical-manifest credit was used.

## DES and commit audit

The DES log preserves the earlier blocked attempts and the final actual phase
events. The final phase sequence is:

| Phase | Result | UTC timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-11-01T19:40:24Z` |
| GREEN | EXECUTED / PASS | `2026-11-01T19:40:56Z` |
| COMMIT | EXECUTED / FAIL, then EXECUTED / PASS | `2026-11-01T19:42:41Z`, `2026-11-01T19:44:01Z` |

The implementation commit `ba85e05c321f4f7882a7b9724a887d29c64123a9` and the
DES follow-up `8bc5ed027d541e1d9f2a597f74072d7109b93798` retain Marcus's author,
the exact `Step-Id: 14-05` trailer and exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer. `git diff --check` passes
for the complete range. The current worktree's only pre-existing dirty file is
`AGENTS.md`.

## Finding

### F-01 — Blocker — the queued-end forwarder audit remedy lacks an exact seeded Sim failure

**Kind:** `EVIDENCE/CONTROL-PLANE-ORDERING`

The production delta at
`crates/overdrive-control-plane/src/guest_network.rs:1339-1359` changes the
forwarder audit invariant from comparing live pairs/watches to comparing
`live_pairs + live_watches + queued_ends` against the owner's pair/watch
places. The comment and the native RED make the intended case clear: an end
record can leave the forwarder's live count while the owner still retains its
pair until the end consumer reads the record.

The real production path is reachable. In the retained diagnostic
`raw/current-console-diagnostic.log` (run `2ea0b8587dc5175ab3d83a9e`), the
periodic owner audit reports:

```text
expected=Forwarder { generation: ForwarderGeneration(1), live_pairs: 1, live_watches: 0 }
observed=Some(Forwarder { generation: ForwarderGeneration(1), live_pairs: 0, live_watches: 0 })
```

The owner then enters Forwarder recovery and reaches its bounded fail-stop.
That is valid native RED evidence. The two final native PASS receipts prove the
stakeholder bodies complete under the final source.

The required control-plane evidence gate is still open for this particular
remedy. No retained current-step or parent-to-current evidence runs an
existing `overdrive-sim`/conformance invariant that drives the queued-end
handoff itself and fails against the approved parent before this change. The
retained R5-58 seed `11400714819323198485` in
`.context/netns-density-295/14-01/raw/shared-audit-overlap-red/regression-2.log`
is a different owner-snapshot race: it creates a Probe watch between the
owner snapshot and inventory read (`expected live_watches=1`,
`observed live_watches=0`). That invariant was already repaired and approved in
the parent history by the current-owner recheck; it does not exercise
`queued_ends`, nor does it prove that the new addition is the necessary fix for
the current `live_pairs=1/live_pairs=0` production mismatch. The final native
GREEN cannot substitute for the seeded Sim gate.

This is an evidence/necessity blocker, not a claim that the arithmetic is
wrong. The private addition is plausible and its native failure is real, but
the repository rule requires the control-plane ordering remedy to be falsified
and revalidated at the existing Sim boundary before acceptance. No new seam,
test model, public API, lane or architectural mechanism is authorized by this
finding.

**Required bounded remediation:** retain a seeded run of an existing
`overdrive-sim`/conformance invariant that reaches the exact queued-end
handoff through the current production owner path, fails against parent
`a2cd9691de4da949389cfe884a044ca9cc60367d`, prints its seed and complete
state/order, and passes with `ba85e05c3`'s source. Include the full raw command,
source identity, exit status and cleanup record. If no existing Sim invariant
can represent the queued-end state, stop treating this as an accepted
control-plane concurrency remedy and return that testability/design gap for
separate disposition; do not add a new seam or weaken the audit contract just
to make the evidence pass.

**Disposition:** OPEN — blocks approval. No production or test remediation was
performed by this reviewer.

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact public API shape | PASS | No public or cross-crate surface changed. |
| Typed semantic errors | PASS | Existing `InitError::Io`, `GuestNetworkError::PostconditionMismatch` and OS errno oracle remain in use. |
| Acceptance completeness | PASS subject to F-01 evidence | Both existing bodies assert all named U-2 correctness outcomes. |
| Test integrity | PASS | Marker removal and reviewed live-row oracle correction only; no weakened assertion. |
| Testing theater / external validity | PASS | Production serve/deploy composition, public describe, peer, kernel TCP and VMM/CID observations. |
| GuestKit boundary | PASS | Nested effects stay inside the existing disposable production CH GuestKit. |
| Lifecycle / async completion | PASS | EXIT close-before-poweroff, reader-priority drain, awaited writer stop and existing cleanup ownership are coherent and natively exercised. |
| Control-plane necessity evidence | **BLOCKER** | F-01: the exact queued-end invariant is not seeded and falsified in Sim. |
| Scope / architecture | PASS | No new lane, scenario, benchmark, API, seam, model or persistence mechanism. |

## Verification record

- Read `AGENTS.md`, the applicable `.claude/rules/**/*.md`,
  `nw-sc-review-dimensions`, `nw-tdd-review-enforcement` and
  `nw-tdd-methodology`.
- Compared the complete parent-to-HEAD diff and verified the exact eight-file
  scope, commit authors, Step-Id trailers and Codex trailers.
- Audited both exact identities, the live-address correction, contract-shape
  declarations, direct-selection accounting, static resource assertions and
  final raw/native ledger receipts.
- Verified final focused results, source/staged ELF equality evidence, 64/122
  cleanup completions, no canonical credit and the disposable GuestKit console
  boundary.
- Verified the retained production RED audit mismatch and the historical seeded
  R5-58 Sim RED; distinguished the earlier watch-race invariant from the new
  queued-end accounting branch.
- Did not run mutation testing, KASAN, Lima, host-native nextest, privileged
  host cleanup, a duplicate native run or any unchanged formal input.

## Remediation disposition

F-01 must be addressed by the original step crafter and returned to this same
reviewer. The native final receipts remain historical evidence and do not need
rerun unless the source or selected boundary changes. No next roadmap step may
start while this review is `NEEDS_REVISION`.

## Verdict

**NEEDS_REVISION.** The accepted test bodies, GuestKit boundary, API shape,
typed error usage, lifecycle fixes and final native outcomes are otherwise
strong and design-aligned. Approval is blocked solely because the new
control-plane queued-end audit remedy has not been independently falsified and
validated through the required existing seeded Sim invariant.

## Review iteration 2 — queued-end seeded regression remediation

### Iteration 2 metadata

| Field | Value |
|---|---|
| Review type | Focused remediation re-review by the same independent DELIVER reviewer |
| Prior finding | F-01 — queued-end forwarder audit remedy lacked an exact seeded Sim failure |
| Current HEAD | `846d8ec02659084215e1ded5b17a4cb466aa2ea4` |
| Remediation commits | `9dc7bb9ba3269460c7b13dfefdf72e64067eb3c7`, `68b0f7b10c4b1579f04bb5a03588f726dd6db11e`, `846d8ec02659084215e1ded5b17a4cb466aa2ea4` |
| User approval | Exact one-regression proposal approved with `yes` |
| DISTILL authored-body review | `docs/feature/netns-density-295/deliver/review-queued-end-audit-regression-proposal.md` — APPROVED |
| Iteration 2 verdict | **APPROVED** |
| Open findings | 0 |

This re-review covers only F-01 remediation. It does not reopen the accepted
64-VM native bodies, their production fixes, the GuestKit support review, or
any prior boundary/API/error/lifecycle disposition. The new regression is the
single user-approved source-local Sim body; no production source, public API,
seam, model, lane, native VM path or existing test body was added or changed.

### F-01 remediation evidence

The new identity is exactly:

```text
shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery
```

The authored-body review confirms the executable diff is confined to
`crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs`:
one contract-table row, one import of the existing private `SimPairKind`, and
one body carrying `/// CONTRACT_SHAPE: bounded-change.` The source hash is
`726a3944991ca3070592b59e82e17ce0d1a13162a722a425b1fae63f891e2ca1`.
The body reuses the existing `seeds()` surface and the existing seed
`11400714819323198485`; it adds no environment variable, new fixture method,
seam, model action or API.

The exact action trace is identical in the parent and restored worktrees:

1. Advance 989 ms within the audit period.
2. Boot one active allocation and pair one guest-opened TCP flow.
3. Arm the existing post-snapshot/pre-inventory audit checkpoint.
4. Let the periodic audit capture one owner pair and pause.
5. Reset the actual Sim forwarder pair while `auto_ends=false`, leaving one
   unread end record queued.
6. Release the periodic audit and settle the supervisor.
7. Release the queued end to the real production owner consumer and settle.
8. Await `ComposedNode::crash()` cleanup before evaluating retained facts.

The test's tracked state is the exact one-place universe:

| Point | Owner pair places | Forwarder inventory | Required outcome |
|---|---:|---|---|
| Before end | 1 | `live_pairs=1, live_watches=0, queued_ends=0` | Open, no recovery |
| End queued/unread | 1 | `live_pairs=0, live_watches=0, queued_ends=1` | One occupied place, no recovery |
| After end consumed | 0 | `live_pairs=0, live_watches=0, queued_ends=0` | Converged, no recovery |

This directly proves the necessity of `queued_ends`: parent source
`3c1b9411d8d54abb2c610b3e66ce743177c0b7042f60b7cc474a88bc9a741173`
compares only live counts, while restored source
`d7498bf52b685404485efcc5664c8485ffac1bc6121c91ecf3c1c361232e4308f`
includes the queued record in both snapshot and current-owner comparisons.
The restored hash is recorded by the ledger as
`d7498bf52b685404485efcc5664c8485ffac1bc6121c91ec3f1c361232e4308f`.

The parent-fail and restored-pass receipts are complete:

| Worktree | Run | Result | Counts | Cleanup |
|---|---|---|---|---|
| Parent + identical regression body | `2c362955672f8c2357a11a60` | exit 100; typed false Forwarder recovery | selected 1, passed 0, failed 1, ignored 0, runtime-skip 0 | `node.crash().await` completed before retained assertions |
| Restored production source + identical body | `fbbd698701ac86754a28e70d` | exit 0; `1 passed; 0 failed; 0 ignored; 0 runtime-SKIP` | selected 1, passed 1, failed 0, ignored 0, runtime-skip 0 | `node.crash().await` completed before retained assertions |

The parent receipt prints the exact seed and action sequence, then observes
the false recovery (`RecoveryEpisode::Held`, one hold, `Quiesced`) before the
assertion fails. The restored receipt prints the identical sequence and
passes both the no-recovery after-audit tuple and the zero-after-consumption
tuple. Preparation failures are separately retained in the ledger (stale BPF
artifact and unprivileged target write attempts); they did not receive RED
credit. The actual parent/restored receipts use task-owned target directories
and the identical BPF object SHA
`5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a`.

The new regression is distinct from the historical watch race. The old R5-58
seed in iteration 1 creates a Probe watch between owner snapshot and inventory
read. This remediation creates a TCP pair, ends it into `queued_ends=1`,
holds the owner's pair until the existing end consumer reads the record, and
proves the exact arithmetic branch introduced by F-01. The parent failure and
restored pass therefore establish both reachability and necessity of the
queued-end accounting.

### API, ownership and boundary re-check

The remediation adds no public/cross-crate API, typed error, wire field,
persistence shape, production mechanism, Sim seam or model action. It enters
through the production-composed `ComposedNode` owner/supervisor and uses only
the existing driven `SimGuestFlowIo` controls (`set_auto_ends`,
`destination_resets`, `release_end`, inventory and checkpoint). The body does
not call `audit_shared` directly, seed owner state, or recreate the production
arithmetic.

The expected parent failure is the existing typed
`GuestNetworkError::PostconditionMismatch` path surfaced as a Forwarder
recovery episode. Cleanup is awaited before assertions, so a panic cannot be
mistaken for semantic RED and no process/task tree remains live. The restored
source remains the original production correction; no source change was made
to pass the new body beyond the already reviewed `queued_ends` arithmetic.

### Iteration 2 disposition

| Finding | Evidence | Disposition |
|---|---|---|
| F-01 | Exact seeded queued-end body, parent exit 100 with false recovery, restored exit 0, identical action trace/source test hash, complete cleanup and zero runtime skips | **CLOSED** |

No new findings were introduced. The source-gate receipt
`1603799a4a13e83bf98fd6e1` passed formatting, scoped check and scoped Clippy for
the authored body; no native, mutation, KASAN, Lima or host effect was used.

## Cumulative final verdict

**APPROVED.** Iteration 2 closes F-01 with the user-approved, independently
reviewed seeded Sim regression. The queued-end arithmetic is now proven through
the existing production owner/supervisor boundary to fail on the parent and
pass on the restored source, with exact source identity, seed/action trace,
typed failure, cleanup and skip accounting. All iteration-1 acceptance,
GuestKit, API, typed-error, lifecycle, scope and native evidence assessments
remain approved and unchanged.
