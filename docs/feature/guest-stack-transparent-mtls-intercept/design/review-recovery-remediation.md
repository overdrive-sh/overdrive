# DESIGN Recovery Remediation Review

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review iteration | 1 |
| Role | Independent solution-architecture reviewer |
| Reviewed commit | `39e256890bfda97b1d2db368a9d462972387b74e` |
| Parent | `89604b407dab319d2c9e01bb34f3f20d3243f97e` |
| Review type | DESIGN recovery remediation |
| Scope | `feature-delta.md` and `design/wave-decisions.md` recovery amendment, checked against the accepted feature inputs, architecture SSOT/ADRs, the architecture-delta assessment, and the current production delta since approved step 02-04 |
| Excluded | Implementation changes, tests, roadmap remediation, mutation testing, and historical-review prescriptions as architecture authority |
| Verdict | **NEEDS_REVISION** |

## Executive assessment

The remediation correctly rejects the filesystem outbox, parallel lifecycle
truth, multi-process data-directory protocol, live-survivor reconstruction,
quarantine protocol, pre-start interception, generic public task primitives,
and retry-owner transfer. It also restores the accepted post-READY/pre-EXEC
ordering, makes awaited mTLS teardown explicit, constrains task ownership to the
allocation-local worker, pins the permitted public API, and gives every major
remediation-era mechanism an explicit disposition.

Two high-severity architecture defects remain. First, the selected owner
shutdown drains allocation guards while the accepted VM lifecycle deliberately
does not stop live VMs at server shutdown. That creates the exact live-guest,
guard-less interval which the boot ordering claims cannot occur. Second, the
terminal-cleanup design promises that VM Artifact Disposal converges every
remaining host residue even though its accepted executor owns only VM scope,
run-directory, and rootfs artifacts; it cannot converge mTLS worker or structural
network residue. Both defects put an implementation crafter at an unresolved
architecture choice, so the recovery is not yet executable as a closed contract.

## Scope and evidence

The review read the complete target diff and the complete amended sections, not
only the summary in `wave-decisions.md`. Evidence included:

- accepted feature issue GH #222, including its 2026-08-27 tap-in-netns scope
  amendment;
- `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
  and `distill/red-classification.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/deliver/assessment-architecture-delta-since-02-04.md`;
- architecture `brief.md` and ADR-0069, ADR-0070, ADR-0081, ADR-0083,
  ADR-0088, and ADR-0089;
- the cumulative production delta from approved step-02-04 commit `408f5feb`
  through the reviewed commit, with focused source inspection of
  `ServerHandle::shutdown`, `MtlsInterceptWorker::{stop_alloc,shutdown_owner}`,
  `Driver::release_for_exit_emission`, VM reclamation executors, and action-shim
  start/terminal paths.

The reviewed commit changes exactly two design files: 452 insertions and 28
deletions. `git diff --check 89604b407dab319d2c9e01bb34f3f20d3243f97e
39e256890bfda97b1d2db368a9d462972387b74e` completed successfully.

## Findings

### GMR-ARCH-001 — Owner shutdown removes the guard from a VM that intentionally survives server shutdown

- **Severity:** High
- **Dimension:** Security ordering; lifecycle ownership; accepted-architecture compatibility
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:863-865`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:996-1001`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1031-1054`
  - `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md:410-414`
  - `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md:1291-1294`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:841-903`
  - `crates/overdrive-control-plane/src/lib.rs:1522-1538`

The recovery retains a one-shot `shutdown_owner` which “attempts all allocation
teardowns.” The exact retained `stop_alloc` contract closes admission, joins the
allocation task tree, drains enforced handles, and drops the outbound rule guard.
The production shutdown path invokes this owner shutdown, but it does not stop
driver-owned workloads or author lifecycle state. ADR-0083 explicitly chooses
no shutdown-time VM stop because boot reclamation is the sole survivor-removal
mechanism.

Those decisions do not compose. A graceful SIGINT can complete mTLS worker
teardown while the Cloud Hypervisor process and guest remain live. The recovery
then relies on a later boot to kill that guest, but its stated safety argument
only covers the order “kill old VMM, then stale-rule sweep.” The graceful
shutdown has already removed the rule before either event. The result is a live
guest outside its allocation interception boundary for an unbounded interval
(including the case in which no replacement process is started), contradicting
F3 and the claim that no live guest can cross a guard-less window.

This is not solved by the test-only abrupt owner seam: that seam intentionally
models process loss and cannot define production graceful-shutdown semantics.
Nor may DELIVER invent shutdown-time lifecycle finalization, survivor adoption,
quarantine, or a second public operation to fill the gap.

**Required remediation:** Pin a production owner-shutdown disposition which
preserves the accepted no-shutdown-time-VM-stop decision and keeps every
surviving VM fail-closed until the next boot's kill-before-sweep boundary. The
answer must use the already accepted lifecycle/kernel mechanisms, must state
the exact ownership/drop behavior of allocation rule guards and listeners, and
must remain compatible with the exact one-shot public signatures. If no such
shape is intended, explicitly revise the affected accepted decision through
DESIGN; do not leave the choice to the 02-06 crafter.

### GMR-ARCH-002 — VM Artifact Disposal is assigned residue it cannot observe or remove

- **Severity:** High
- **Dimension:** Recovery authority; mechanism completeness; state-layer hygiene
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:893-898`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1003-1020`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1125-1138`
  - `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md:1055-1064`
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:281-317`

R4 applies to all terminal cleanup owners—awaited mTLS stop, structural
netns/slot teardown, and the driver hook—and says that after any cleanup failure
the lifecycle row may commit and “existing VM Artifact Disposal converges
remaining host residue.” The accepted Artifact Disposal executor has a narrower
authority: it kills the VM cgroup scope and discards VM run-directory/rootfs
artifacts while authoring no row. It has no mTLS worker, network provisioner,
slot allocator, listener, enforced-handle, or allocation-rule capability.

The terminal-row fence then makes a duplicate finalization a zero-effect no-op,
so a failed terminal `stop_alloc` or structural network teardown is not retried
by action replay after the row commits. A private later `stop_alloc` or owner
shutdown can retry retained mTLS handles, and a future boot's ordinary network
GC/rule sweep may remove some other residue, but neither fact makes VM Artifact
Disposal the general convergence owner asserted by R4. The current wording
therefore both overstates accepted recovery and invites DELIVER to widen the VM
reclaimer into another cross-layer cleanup subsystem—the architecture failure
this remediation is meant to remove.

**Required remediation:** Limit each recovery promise to the resources its
accepted mechanism actually owns. State separately what happens after a failed
mTLS stop, a failed structural network teardown, and failed VM artifact cleanup;
identify only already-reachable retry/boot paths, and preserve the typed error
when no accepted bounded convergence promise exists. Do not add a durable
cleanup token, parallel state machine, wider reclamation port, or hidden
terminal-row meaning to make the prose true.

## Fixed-decision compliance

| Fixed decision | Result | Evidence and assessment |
|---|---|---|
| ObservationStore terminal row is the sole durable lifecycle truth | PASS | F1 and R1 remove `terminal-effects/`, receipts, replay, `effect_key`, and lifecycle-event ports; broadcast is explicitly ephemeral and snapshot/relist recovers truth. |
| No outbox or parallel system of record | PASS | Removal is unconditional in the recovery table and exact surface disposition. No ObservationStore row is repurposed as an intent or delivery queue. |
| One process owns one data directory | PASS | F2 rejects shared-directory multi-process coordination and the reclamation lease is explicitly process-local and non-durable. |
| Exact capture-ready through awaited EXEC-release order | PASS | F3 and R6 restore C3/capture, awaited driver start/READY, Running commit, awaited intercept install and D7 baseline, then awaited release. Pre-start interception is removed. |
| Await effects by evolving the existing operation | PASS | `stop_alloc`, worker shutdown, server shutdown, CLI shutdown, and EXEC release are awaited; runtime lookup, detached cleanup, and sibling public methods are forbidden. |
| Roadmap lists are guidance | PASS | F5 permits compiler/criterion fallout while leaving the roadmap outside this DESIGN edit. |
| Review prescriptions are provenance, not architecture | PASS | The recovery expressly supersedes journal/quarantine/survivor/pre-start prescriptions and treats old review artifacts as history. |
| Cleanup/recovery promises stay within accepted mechanisms | **FAIL** | GMR-ARCH-002 assigns mTLS/network residue to the VM Artifact Disposal executor, whose accepted authority does not include it. |
| No hidden state machine or multi-instance protocol | PASS | Pending cleanup tokens, special Pending meaning, route/event records, retry state machines, and cross-process leases are removed. |
| No pre-start intercept or hidden survivor reconstruction | PASS | Both mechanisms and their tests/call sites are explicitly removed. |
| Private allocation-local task ownership only | PASS | R3 removes the public generic core module and confines registration/stop completion to one mTLS allocation; resolver/server/driver/reclamation reuse is forbidden. |
| Resume fresh 02-05, then fresh 02-06, preserving 02-04 | PASS | The recovery boundary is explicit and treats `408f5feb` as comparison-only, not reset authority. |
| Exact API with no invented surface | PASS | The four async signatures and retained/removed cross-crate surfaces are exact; private helper freedom is bounded. |
| Live VM remains protected through replacement recovery | **FAIL** | GMR-ARCH-001 leaves graceful owner shutdown free to delete the guard before the accepted boot reclaimer kills the surviving VM. |

## Mechanism-disposition coverage

| Remediation-era mechanism | Disposition coverage | Review result |
|---|---|---|
| Failed-start cleanup ownership/retry protocol | `RESHAPE` to one awaited `VmDriver::start` owner; remove pending owner/token/public cleanup carrier | Covered and coherent for VM-owned start resources |
| Pure same-attempt terminal fence | `RETAIN` | Covered; exact duplicate finalization is zero-effect and Platform Reclamation is the only same-id reopen |
| `OwnedTaskSet` / `CompletionFence` | `RESHAPE` to private allocation owner/completion | Covered; no generic cross-layer reuse survives |
| Allocation stop | `RETAIN / RESHAPE` as awaited, fallible, cancellation-safe per-allocation stop | Covered, subject to GMR-ARCH-001/002 at outer shutdown and post-terminal failure boundaries |
| Server shutdown/retry owner | `RESHAPE` to async, fallible, one-shot diagnostics only | Public retry transfer is correctly removed; production survivor safety is incomplete per GMR-ARCH-001 |
| Execution-time reclamation arbitration | `RETAIN, NARROW` process-local claim over the VM supervision map | Covered; no persistence or cross-process authority is implied |
| Live-survivor reconstruction | `REMOVE` | Covered, including plan/apply/recover call sites |
| Recovery quarantine | `REMOVE` | Covered, including kernel type, userdata, batches, APIs, and tests |
| Pre-start intercept and rollback | `REMOVE` | Covered; fresh and same-id restart return to the same post-READY gate |
| Terminal outbox/event protocol | `REMOVE` | Covered comprehensively across persistence, ports, receipts, replay, event key, and terminal hook |
| Public cleanup/error additions | `REMOVE / NARROW` | Covered; the retained duplicate-start cause and reclamation claim are bounded |
| Abrupt test-owner seam | `RETAIN` behind integration gates only | Covered; it authors no lifecycle truth and is not accepted as a substitute for boot-reclamation evidence |
| Tests and compiler fallout | `RESHAPE` | Covered by behavior/protocol classification and explicit no-mutation boundary |
| VM Artifact Disposal as a universal residue owner | No valid disposition | **Not covered honestly; GMR-ARCH-002** |

## Architecture quality summary

| Dimension | Assessment |
|---|---|
| Requirements alignment | Strong except for the two recovery/shutdown contradictions above |
| System-of-record discipline | Strong; one durable lifecycle truth and no queue semantics in ObservationStore |
| Component boundaries | Strong for task ownership and public surface; incomplete at owner shutdown and residue ownership |
| Concurrency and ordering | Strong for start, terminal-row, task-registration, and reclamation races; unsafe graceful-shutdown ordering remains |
| Failure semantics | Typed and awaited at the named APIs, but terminal cleanup overpromises convergence beyond the available capability set |
| Evolvability | Good removal of speculative protocols; exact surface constraints prevent another review-driven API accretion cycle |
| Testability | Mechanism-specific test retention/removal is explicit and preserves independent Rust/expectation boundaries |

## Independent verification

- `git diff --check` for the reviewed parent/target range: **PASS**.
- Target file scope: **PASS**; only the two authorized DESIGN artifacts changed.
- Fixed-decision trace: **12 PASS, 2 FAIL**.
- Remediation-mechanism classification: comprehensive except for the invalid
  universal Artifact Disposal ownership claim identified in GMR-ARCH-002.
- Public API audit: no unpinned recovery API is sanctioned by the amendment.
- Design/code executability audit: **FAIL** at graceful owner shutdown and
  post-terminal cleanup failure, where the prose requires behavior not supplied
  by the selected accepted mechanisms.
- No code, tests, roadmap, assessment, or mutation configuration was changed or
  executed by this review.

## Remediation gate

The original designer must remediate both high-severity findings in the DESIGN
artifacts and return the resulting commit for a fresh review iteration. The
remediation must not reintroduce an outbox, parallel lifecycle truth, durable
cleanup token, live-survivor join, quarantine protocol, pre-start intercept,
generic task primitive, public retry owner, shared-directory protocol, or new
public API. It must make graceful owner shutdown and each residue class
executable using only the selected accepted mechanisms.

## Verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 2 High, 0 Medium, 0 Low**.

DELIVER must not resume at 02-05 RED until this recovery DESIGN has been
remediated and independently approved.

---

## Iteration 2 — 2026-08-31

### Review metadata

| Field | Value |
|---|---|
| Reviewed commit | `bce1658de52ed95c0b1daecbe675aed38a141a27` |
| Compared with | Iteration-1 review commit `b2491db09bbefdb5c201abfe4d2b0b9a106e1023` |
| Review type | DESIGN recovery remediation re-review |
| Scope | Disposition of GMR-ARCH-001/002; the corrected current-state-versus-occurrence model; bounded-history, atomicity, restart/replay, LWW, duplicate/same-state, schema-evolution, and stream semantics; the mark-before-TPROXY kernel premise; and consistency of ADR-0088, ADR-0089, and `brief.md` |
| Excluded | Implementation changes, tests, roadmap/assessment edits, mutation testing, and any redesign by the reviewer |
| Verdict | **NEEDS_REVISION** |

### Executive assessment

Both Iteration-1 findings are closed. Owner shutdown now retains each active
allocation's original kernel rules while revoking userspace, and the amended
mark-before-TPROXY order is supported by the primary Linux 6.18 implementation:
the mark mutates `skb->mark`, a listenerless TPROXY sets `NFT_BREAK`, that
verdict skips the rest of the current rule without undoing prior side effects,
and IPv4 input routing builds its FIB key from the resulting skb mark. The
already-existing persistent fwmark rule/local route therefore prevents the
packet from resuming its original forwarding path. The VM Artifact Disposal
overclaim is also removed; each failure class now names only its actual live
retry, process-exit, or boot cleanup owner, and the terminal row is withheld
until action-owned mTLS and structural-network cleanup succeeds.

The user-caught lifecycle-model correction is architecturally necessary, not a
speculative outbox. A LWW current row cannot preserve the Platform Reclamation
occurrence after the permitted same-allocation Running successor wins. A
bounded immutable occurrence family in the same ObservationStore, accepted in
one transaction with current state, is the smallest mechanism in the supplied
vocabulary that preserves that fact without creating delivery receipts or a
second system of record. The 64-row recent-history boundary, oldest-first
eviction, restart persistence, LWW-loser/equal-retry no-op, and explicitly
best-effort stream projection are all sufficiently pinned.

Two new high-severity gaps keep the compound record contract from being
implementable without invention. The old generic
`ObservationStore::write(ObservationRow::AllocStatus(..))` remains a public
current-row authoring path that can bypass occurrence creation entirely. In
addition, the new operation cannot both derive an exact `from` state and retain
ADR-0048's existing observation self-healing behavior when the prior current
envelope is corrupt or from an unknown future version. The exact-surface rule
forbids the implementation crafter from selecting either missing disposition.

### Iteration-1 finding dispositions

| Finding | Disposition | Evidence and assessment |
|---|---|---|
| GMR-ARCH-001 — owner shutdown removes the guard from a surviving VM | **CLOSED** | `feature-delta.md:871,1104-1152,1220-1248` makes owner shutdown a non-terminal process-owner boundary: it seals registration, suppresses active outbound/inbound guard destructors, closes listeners and every userspace child, and leaves the original rules installed through boot kill-before-sweep. Both prerouting encoders are pinned to mark before TPROXY, with no quarantine, listener adoption, second rule, or public guard method. The kernel premise and persistent shared-routing premise were independently checked below. |
| GMR-ARCH-002 — VM Artifact Disposal is assigned mTLS/network residue | **CLOSED** | `feature-delta.md:1154-1208` withholds the terminal current row and occurrence on driver, mTLS, or structural-network cleanup failure and leaves the level-triggered action replayable. Its resource-specific table limits Artifact Disposal to VM-exclusive cgroup/run-directory/rootfs residue; mTLS and network residue name only existing retry, process-exit, boot netns-GC, and tagged-rule-sweep paths, including an explicit non-promise where no bounded convergence exists. |

### New findings

#### GMR-ARCH-003 — the generic AllocStatus writer bypasses the atomic lifecycle contract

- **Severity:** High
- **Dimension:** System-of-record integrity; API-shape completeness; atomicity
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:866`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:917-971`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:973-980`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1283-1308`
  - `crates/overdrive-core/src/traits/observation_store.rs:610-612`
  - `crates/overdrive-core/src/traits/observation_store.rs:1873-1915`
  - `crates/overdrive-store-local/src/observation_backend.rs:416-443`
  - `crates/overdrive-sim/src/adapters/observation_store.rs:253-258,616-645`

R1 adds `write_alloc_lifecycle(current, source)` and says every named
production lifecycle author must use it, but R7 only adds that method; it does
not dispose of the existing public
`write(ObservationRow::AllocStatus(Box<AllocStatusRow>))` branch. That branch
remains part of the closed `ObservationRow` input surface and both adapters
still accept it as an ordinary LWW current-row mutation. It has no
`TransitionSource`, so it cannot construct the required occurrence.

The result is two public ways to author the same durable current record. One
commits current+occurrence atomically; the other can commit and fan out current
alone. Migrating today's action-shim, exit-observer, and reclamation call sites
by convention does not close the invariant for a missed site, a future writer,
or any existing helper that receives `ObservationRow`. It also makes R1's
statement that a caller cannot commit current without the matching occurrence
false at the port boundary. This is particularly material because the generic
write still emits the accepted `AllocStatus` subscription event, so downstream
reconcilers can converge on a current state for which the promised occurrence
does not exist.

The crafter cannot resolve this by guessing. Rejecting the generic variant,
removing it from the write-input shape while retaining it as a subscription
payload, or routing it through the compound operation each has a different
public/error/source contract; the last is impossible without inventing a
`TransitionSource`.

**Required remediation:** pin the exact disposition of
`ObservationStore::write` for `ObservationRow::AllocStatus` so there is exactly
one legal lifecycle-current authoring primitive and every accepted current
winner necessarily has its derived occurrence. Preserve the existing
`SubscriptionEvent::Row(ObservationRow::AllocStatus(..))` read-side projection
if that remains required, but do not ask DELIVER to invent a source value,
error variant, split input enum, visibility rule, or compatibility bypass.
Add conformance evidence that the old public path cannot create a current-only
lifecycle winner.

#### GMR-ARCH-004 — a corrupt or future-version prior row has no legal compound-write outcome

- **Severity:** High
- **Dimension:** Schema evolution; recovery semantics; atomicity
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:939-968`
  - `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md:528-541`
  - `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md:565-573`
  - `crates/overdrive-store-local/src/observation_backend.rs:1078-1114`

The compound operation must read the prior current row, apply the existing LWW
rule, and derive `occurrence.from` from the prior row's **exact** state; `None`
is reserved for the first accepted transition. The existing local LWW writer,
however, deliberately treats an undecodable prior AllocStatus envelope—whether
malformed or an unknown future variant—as displaced by the incoming typed row.
That is ADR-0048's observation-side self-healing posture: log/skip the bad row
and allow convergence, rather than turning one observation into persistent
unavailability.

R1 does not define the compound equivalent. Returning a codec error and rolling
back forever leaves that allocation's current key poisoned and contradicts the
existing convergence behavior. Treating the undecodable prior as absent writes
`from: None`, falsely recording a first transition when durable predecessor
bytes exist. Falling back to the generic writer reopens GMR-ARCH-003 and loses
atomic occurrence creation. The V1 occurrence schema has no value that means
“predecessor existed but its state was unreadable,” and the exact-surface rule
forbids a crafter from adding one.

**Required remediation:** define the exact atomic behavior when the current
AllocStatus envelope cannot be decoded, including malformed bytes and an
unknown future envelope variant. The decision must reconcile ADR-0048's
observation degradation/self-healing policy with truthful occurrence history,
and must pin any required schema, error, or method-shape change in DESIGN.
DELIVER must not invent a sentinel `from`, silently mislabel it as `None`, or
use the current-only writer as an escape hatch.

### Lifecycle-history adversarial matrix

| Case | Result | Assessment |
|---|---|---|
| Why a second record family is needed | **PASS** | Platform Reclamation is a genuine lifecycle occurrence whose later same-id Running winner supersedes the LWW current row. A separate bounded family inside the same store directly repairs the current-state/history category error without becoming an outbox. |
| Atomic current + occurrence | **FAIL** | The selected local/sim transaction shapes are feasible, but GMR-ARCH-003 leaves a parallel current-only writer and GMR-ARCH-004 leaves one prior-decode state without a lawful transaction result. |
| LWW loser and equal retry | **PASS** | Both return `Ok(None)`, mutate neither family, and fan out nothing. The required conformance test includes equal retry, so replay of the same timestamp cannot duplicate history. |
| Dominating same-state write | **PASS** | The algorithm defines an occurrence by an accepted LWW winner, not by `from != to`; therefore a meaningful Running→Running `Stable` claim and any other dominating same-state update are retained. Exact equal-timestamp retries remain suppressed. This is coherent with the existing lifecycle-event model, which already contains same-state claims. |
| Duplicate terminal finalization | **PASS** | The terminal claim on the current row remains the action-level fence and the transition check runs before cleanup; sequential replay of the identical claim performs no cleanup, write, occurrence, hook, or broadcast. The occurrence row is not misused as the fence. |
| 65th and later occurrence | **PASS** | The transaction evicts the oldest `(counter, writer)` keys until exactly the latest 64 remain, and the tests explicitly require the 65th winner to evict only the oldest. The design labels this durable **recent** history and defers fleet-wide/configurable retention to GH #265 rather than implying an audit log. |
| Process restart / replay | **PASS**, subject to GMR-ARCH-004 | Both families live in the same redb database and commit transaction; retained occurrences survive restart. Replaying an equal accepted row produces no new entry. Ordinary valid-envelope restart behavior is closed; the malformed/future-prior case is not. |
| Stream cut after commit | **PASS** | Broadcast is expressly best-effort and occurs after the store commit. A cut can lose or duplicate a live notification; existing submit streams retain their terminal-current snapshot recovery and gain no cursor/replay/exactly-once promise. The occurrence table is therefore observation history, not a delivery queue or disguised outbox. |
| Production consumer / speculative scope | **PASS** | No HTTP/CLI query or automatic stream rebuild is invented. The typed reader is the minimum adapter/conformance surface for the durable fact the user required; broader operational-event querying/export remains GH #265. |

### Primary-kernel verification of GMR-ARCH-001

The dead-listener argument was checked against the project's pinned Linux 6.18
line, not only nftables prose:

- [`nft_tproxy.c` at v6.18](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/net/netfilter/nft_tproxy.c?h=v6.18)
  sets `regs->verdict.code = NFT_BREAK` when no transparent socket is found;
- [`nft_meta.c` at v6.18](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/net/netfilter/nft_meta.c?h=v6.18)
  implements `NFT_META_MARK` assignment as `skb->mark = value`;
- [`nf_tables_core.c` at v6.18](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/net/netfilter/nf_tables_core.c?h=v6.18)
  handles `NFT_BREAK` by resetting the expression verdict to continue and
  advancing to the next rule, without rolling back an earlier skb mutation;
- [`route.c` at v6.18](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/net/ipv4/route.c?h=v6.18)
  assigns `fl4.flowi4_mark = skb->mark` before the IPv4 input FIB lookup.

The existing production shared infrastructure is node-global add-if-missing
state and is never removed by a per-allocation guard: `fwmark 0x1 lookup 100`
plus `local 0.0.0.0/0 dev lo table 100`
(`crates/overdrive-worker/src/mtls_intercept.rs:825-876,986-1018`). In the
current one-process/one-data-directory scope, the retained rule therefore
cannot resume the original forwarding route merely because its TPROXY listener
closed. `NFT_BREAK` continues evaluation after the current rule, so the
mandated real-kernel dead-listener test remains important to catch a future
later rule that clears/replaces the mark; the present design explicitly
requires that proof.

### Architecture-SSOT amendment check

| Artifact | Result | Assessment |
|---|---|---|
| ADR-0088 | PASS | The amendment changes only the existing outbound/inbound prerouting expression order and the dead-listener consequence; topology, addressing, D7 counter identity, ownership, and live-listener behavior remain intact. |
| ADR-0089 | PASS | Provisioning and CH-attach boundaries remain unchanged. The text consistently classifies both encoders as mark-before-TPROXY and adds no second rule, quarantine, listener-adoption, or guard API. |
| `brief.md` | PASS | The ADR index rows and 2026-08-31 changelog record only the kernel rule-order amendment. They correctly leave feature-local occurrence/cleanup mechanics in the feature DESIGN artifacts and do not falsely attribute an ObservationStore schema change to ADR-0088/0089. |

### Fixed-decision and mechanism summary

| Area | Result |
|---|---|
| One ObservationStore, no filesystem outbox/receipt/replay protocol | PASS |
| LWW current state distinguished from bounded lifecycle occurrences | PASS |
| Exactly one atomic lifecycle-current authoring route | **FAIL — GMR-ARCH-003** |
| ADR-0048-compatible corrupt/future-prior handling | **FAIL — GMR-ARCH-004** |
| One process / one data directory | PASS |
| Post-READY/pre-EXEC interception order | PASS |
| No survivor reconstruction, quarantine, pre-start intercept, or public retry owner | PASS |
| Private allocation-local task ownership | PASS |
| Graceful owner shutdown keeps a surviving VM fail-closed | PASS |
| Resource-specific terminal cleanup/recovery ownership | PASS |
| Exact public-surface discipline | **FAIL** at the two ObservationStore gaps above |
| Fresh 02-05 then fresh 02-06 resume boundary | PASS |

### Independent verification

- `git diff --check b2491db09bbefdb5c201abfe4d2b0b9a106e1023 bce1658de52ed95c0b1daecbe675aed38a141a27`: **PASS**.
- Remediation scope: **PASS**; five documentation files changed, comprising the
  two recovery DESIGN artifacts and the three narrowly required architecture
  SSOT amendments.
- GMR-ARCH-001: **CLOSED** by exact shutdown ownership plus Linux 6.18 kernel
  semantics and the retained shared route.
- GMR-ARCH-002: **CLOSED** by commit-last terminal ordering and
  resource-specific recovery promises.
- Lifecycle history: the mechanism is justified and its normal-case retention,
  replay, LWW, and stream contracts are coherent; port closure and corrupt-prior
  semantics remain blocking.
- No code, tests, roadmap, assessment, DES log, or mutation configuration was
  changed or executed by this review.

### Iteration-2 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 2 High, 0 Medium, 0 Low**.

The original designer must close GMR-ARCH-003 and GMR-ARCH-004 in the
authoritative DESIGN artifacts, preserving the accepted occurrence-history,
mark-before-TPROXY, and resource-specific cleanup decisions. DELIVER must not
resume at 02-05 RED until a later independent review returns **APPROVED**.
