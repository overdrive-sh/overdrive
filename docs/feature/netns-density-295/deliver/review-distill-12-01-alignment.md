# DISTILL review — STEP 12-01 error-taxonomy alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-01` |
| Feature | `netns-density-295` |
| Roadmap step | `12-01` |
| Review date | 2026-10-24 |
| Reviewer | Fresh isolated `nw-acceptance-designer-reviewer` |
| Design authority | Commit `63b8b9b1d1971cf2847186817909f9f991997b80`; `review-design-12-01-error-variants.md` iteration 2 |
| Review scope | The bounded deletion of obsolete acceptance-test taxonomy and bridge/TAP scratch-complement surface, plus the proposed existing semantic RED handoff |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

## Review boundary and authority

This review is limited to `distill-12-01-alignment.md` and the authored
acceptance-test deletions in:

- `crates/overdrive-control-plane/src/guest_network/owner_acceptance/errors.rs`;
  and
- `crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs`.

The controlling contract is the exact post-attachment-cut
`GuestNetworkError` set at `feature-delta.md:8551-8569`, the scratch fact
deletion at `feature-delta.md:8623-8632`, STEP 12-01's deletion discipline at
`roadmap.json:1058`, and the independent DESIGN approval at
`review-design-12-01-error-variants.md:209-306`.

The shared worktree also contains the STEP 12-01 crafter's activation of 476
lines carrying `pending DELIVER` markers or their now-stale marker
documentation. Ten of those marker removals occur in the two reviewed Rust
files. They are concurrent crafter work, not DISTILL-authored test-body
changes. This review preserved them and did not restore, rewrite, or claim
ownership of them.

This review changes only this native Markdown artifact. It changes no
production source, test source, marker, design, roadmap, DES event, execution
log, model, lane, or kernel state. It performs no commit, mutation run, or
Metal job.

## Strengths

- The taxonomy correction is exact. `taxonomy_name` now has thirteen arms,
  one for each approved post-cut `GuestNetworkError` variant, remains
  exhaustive, and has no wildcard. Its one forwarder arm is
  `Forwarder { .. }`; no operation field or replacement carrier was invented.
- The two removed arms are exactly `ScratchCleanupIncomplete` and
  `StartupProbeCleanup`. DESIGN identifies both as having no post-cut
  producer, branch consumer, wire form, persisted form, or compatibility
  requirement.
- The only removed property is
  `scratch_complement_never_fabricates_zero`, together with its test-local
  constructor, the two deleted scratch types, and the now-unused `proptest`
  import. Its complete universe is the deleted bridge/TAP/TCX/map/pin/guard
  scratch complement.
- Retained typed-forwarder, listener-refusal, runtime-taxonomy, provision,
  teardown, and allocation-reclaim bodies remain byte-for-byte unchanged.
  The dirty Rust diff adds no test behavior; its only added lines are the
  rustfmt reflow of the retained import list.
- The alignment correctly rejects the 46 activated `quint_owner_flows`
  failures as a semantic RED receipt. They reach the production owner but
  panic in `HostSharedGuestNetworkOwner::flow_observation` before
  quint-connect compares the projected owner state with the model.

## Contract and deletion map

| Accepted contract | Reviewed acceptance-test disposition | Assessment |
|---|---|---|
| The post-cut `GuestNetworkError` set has exactly 13 variants | Remove only the two obsolete exhaustive-match arms; retain thirteen named arms and no wildcard | **Exact.** Adding any unapproved variant still causes a compile-time exhaustiveness failure. |
| Forwarder failures cross the owner boundary as `GuestNetworkError::Forwarder { source: ForwarderError }` | `a_forwarder_that_cannot_be_opened_or_probed_refuses_boot_typed` remains unchanged and activated | **Preserved.** The exact typed source, no-later-boot-effect complement, and released-context outcome remain asserted. |
| Bridge/TAP scratch probe and reverse cleanup leave at the attachment cut | Remove `GuestNetworkScratchComplement`, `GuestNetworkScratchCount`, the constructor, and their sole pure property | **Exact deletion.** No retained forwarder, allocation, wire, persistence, or operator outcome depends on that property. |
| Allocation cleanup and reclaim remain | All seven provision/teardown/reclaim bodies in `netns_density_guest_network.rs:911-1354` remain unchanged and activated | **Preserved.** The deletion does not remove retained allocation lifecycle evidence. |
| No new test, lane, model, fixture seam, or public API is required | No such surface was added | **Compliant.** |

Repository search finds no surviving reference to the two obsolete error
variants or the deleted scratch helper/property/types in the two reviewed test
paths. Conversely, the retained forwarder and allocation-cleanup tests remain
present. The deletion therefore follows the roadmap's last-owner discipline:
test support leaves with the production concept it alone exercised.

## RED-classification honesty

The proposed RED handoff is sound, with one required sequence made explicit.

The current shared checkout is intentionally between the two sides of an
exact public-contract cut: `taxonomy_name` matches the approved thirteen
variants, while the pre-cut production enum still declares those thirteen
plus `ScratchCleanupIncomplete` and `StartupProbeCleanup`. The resulting
non-exhaustive-match compiler error is a transient contract-alignment build
failure. It is **not** the semantic RED and must never be logged as one.

The STEP 12-01 crafter must first perform the accepted production deletion of
those two variants and their now-obsolete payload/support fallout. That edit is
legitimate preparation because it makes the public enum equal the already
approved exact set; it neither implements nor weakens the missing boot-order
behavior. Immediately after that bounded deletion, and before implementing
the owner boot effects, the crafter can run:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route)' --no-fail-fast --no-capture
```

That existing test enters through `ComposedNode::boot`, which calls the
production phase-1 probe and production `boot_server` composition. The current
phase-1 and owner boot scaffolds return `Ok(())` without opening a forwarder or
issuing a later boot effect. The test consequently reaches its existing
`expect("the forwarder opened")` oracle and fails because `OpenForwarder` is
absent. This is the smallest already-authored semantic ordering RED for the
missing STEP 12-01 behavior; no test-support or upstream specification gap is
present.

The adjacent existing
`phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one`
test remains a valid corroborating typed-error oracle. It drives both approved
phase-1 entry points, requires the exact `ForwarderError` cause, requires the
probe context to be released, and excludes later boot effects. It is not
necessary to invent or rewrite a test to replace the conformance-driver
failures.

If the focused ordering selector does not reach the named missing-
`OpenForwarder` oracle after the production enum deletion, its actual failure
must be classified before GREEN work continues. Neither the present compiler
failure nor a scaffold panic elsewhere may be substituted for that receipt.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 10/10 | This correction removes only a deleted mechanism's property; retained startup refusal, typed cause, cleanup, retry, and ordering error paths remain. |
| Given-When-Then form | 10/10 | No scenario body changes. The selected RED has one production boot stimulus and observable effect-order assertions. |
| Business/domain language | 10/10 | The alignment preserves the precise owner, forwarder context, startup probe, listener, gateway-route, and scratch-complement terms from DESIGN. |
| Coverage completeness | 10/10 | Every retained post-cut error variant stays in the exhaustive closure; only the two design-deleted variants and their sole obsolete property leave. |
| Walking-skeleton user-centricity | 10/10 | Not applicable to this focused error-taxonomy deletion; no walking skeleton changes. |
| Priority validation | 10/10 | The delta fixes only the proven mismatch between the accepted 13-variant contract and pre-cut acceptance-test references. |
| Observable behavior assertions | 10/10 | Retained tests still observe typed errors, production-owner effects, context release, lifecycle order, and cleanup complements. |
| Traceability coverage | 10/10 | The deletion traces directly from DESIGN commit `63b8b9b1` through STEP 12-01 to the two obsolete test surfaces. |
| Walking-skeleton boundary proof | 10/10 | Not applicable; the selected focused RED nevertheless enters through the production boot composition. |

All dimensions meet the approval threshold. Contract Shape declarations on
the retained bodies are unchanged; deleting the complete pure-function
scenario for a deleted type does not weaken a surviving contract.

## Test-design mandates and completeness

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/production boundary | PASS | The retained typed-error and ordering tests drive the production owner or production boot composition. No internal-only replacement test was added. |
| CM-B — domain-language abstraction | PASS | Scenario names and assertions retain the accepted guest-network vocabulary. |
| CM-C — complete focused journey | PASS | Startup probe/open, typed refusal, no-later-effect complement, context release, and boot ordering remain covered across the existing focused cells. |
| CM-D — pure logic/fixture isolation | PASS | The only removed pure helper belonged exclusively to the deleted scratch value; no fixture logic replaces it. |
| CM-E — bounded observable universe | PASS | The retained forwarder and allocation cells keep their complements; no assertion was removed from a retained body. |
| CM-F — layer-dependent PBT mode | PASS | Removing the source-local PBT with its deleted pure type leaves no PBT on a real-I/O layer and introduces no example/PBT mismatch. |
| CM-G — two-tier rich journey | PASS / N/A | No new rich journey or tier is introduced. |
| CM-H — explicit integration sad paths | PASS | Existing startup and forwarder failure cases remain explicit examples. |

For the canonical C1-C7 taxonomy, this bounded correction preserves the
retained boundaries and cardinalities (C1/C3), the phase and lifecycle
transitions (C2/C4), the closed exact error set (C6), and the interruption and
cleanup outcomes (C7). C5 has no new mode flag and is not applicable. DESIGN
has already pinned the C6 error representation exactly, so no
`SPECIFICATION_AMBIGUITY` or `AT_GAP_IN_DELIVERY_SCOPE` exists.

## Mechanical verification

| Check | Result |
|---|---|
| Exact taxonomy arm count | PASS — 13 retained arms, no wildcard, one `Forwarder` carrier. |
| Reviewed test-body additions or rewrites | PASS — none. |
| Obsolete reference search in the two owned test paths | PASS — zero matches. |
| Retained forwarder and allocation-cleanup bodies | PASS — present and unchanged. |
| Concurrent crafter activation preservation | PASS — all 476 removed `pending DELIVER` lines, including the 10 in the reviewed files, remain untouched. |
| Scoped `git diff --check` | PASS. |
| Test execution | NOT RUN — the pre-cut 15-variant production enum intentionally cannot compile against the newly exact exhaustive 13-arm match; this is not accepted as semantic RED. |
| Mutation testing | NOT RUN — correctly reserved for the single final DELIVER gate. |
| Metal execution | NOT RUN — unnecessary for this static bounded review. |

## Verdict

**APPROVED.** The DISTILL alignment removes exactly the acceptance-test
surface owned by the deleted bridge/TAP scratch mechanism, preserves every
retained test body and the crafter's marker activations, and makes the error
closure match the approved exact thirteen-variant contract. The proposed
existing boot-order test is an honest semantic RED after the crafter performs
the mandatory production enum deletion; the intervening compiler failure is
documented only as contract-shape preparation and is not misrepresented as
behavioral evidence. No upstream gap, new test, rewritten oracle, weakened
assertion, lane, model, or public API is required.

# APPROVED

---

# Supplemental review iteration 2 — fail-stop and beacon fixture alignment

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Supplemental DESIGN authority | Commit `bc749ad7f7f4ad85be83a75f5217fcb09cf5e32b`; approved fail-stop DESIGN supplement |
| Scope | Exact eight-cause acceptance table, retained allocation-scoped E23 wording, and private beacon-fixture compiler migration |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration is additive. It preserves iteration 1's evidence and approval,
then reviews the supplemental changes recorded in the updated
`distill-12-01-alignment.md`.

## Supplemental boundary and attribution

The new DISTILL-owned alignment is limited to:

- `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs`;
- documentation only in `crates/overdrive-testing/src/pids_max.rs`;
- documentation only in
  `crates/overdrive-worker/tests/integration/real_cgroup_fs/refused_thread_kill.rs`;
  and
- the private test helper and its existing consumers in
  `crates/overdrive-worker/src/vm_driver.rs`.

The current `vm_driver.rs` diff also contains active STEP 12-01 production
implementation, including transport-token composition, the production
`BeaconWriter` migration, watcher wiring, and two pending-marker removals.
Those changes belong to the original crafter. They were inspected only far
enough to verify that the private fixture targets the coordinated shape; they
are not authored, reviewed for production correctness, or attributed to
DISTILL here.

Across the worktree, the crafter's 476 removed lines containing
`pending DELIVER` remain intact. Two occur in `vm_driver.rs` in addition to the
ten noted by iteration 1. No marker removal is part of this supplemental
DISTILL body.

## Exact fail-stop set

`CAUSES` now contains the eight DESIGN-approved variants in declaration order:

1. `RecoveryDeadlineExceeded`;
2. `LeakedQuiescenceHold`;
3. `ForeignQuiescenceHold`;
4. `SupervisorReturned`;
5. `SupervisorFailed`;
6. `SupervisorPanicked`;
7. `SupervisorCancelled`; and
8. `RequestChannelClosed`.

The edit is internally complete: the array type is `[...; 8]`, the compile-time
length assertion is `CAUSES.len() == 8`, and the generated `FailStop` index
domain is `0_u8..8`. `TapQuiescenceUndetermined` and `VmKillFailed` are removed
from the table with their deleted bridge-era semantic conditions. No retained
cause is reordered, merged, renamed, or flattened.

Both live behavioral bodies are preserved:

- `every_fail_stop_cause_is_closed_and_first_request_wins` still crosses every
  retained cause with all nine retained shared components, verifies the exact
  receipt, verifies first-request-wins, consumes recovery state, and refuses a
  future EXEC claim; and
- `generated_operation_sequences_match_the_gate_model` still checks every
  generated retained cause against the same receipt, terminal-state,
  waiter-release, idempotence, and model-transition assertions.

No assertion, operation type, model state, sequence-length bound, component
domain, test function, Contract Shape declaration, or timeout changed. This is
the exact table/range contraction required by accepted DESIGN, not a new test
or a rewritten oracle.

## Retained E23 owner and observable

The two E23 edits change prose only. They now name the reachable retained
owner precisely:
`VmDriver`'s allocation-scoped `CgroupManager::cgroup_kill` over
`CgroupPath::for_alloc`.

The Tier-3 body remains byte-for-byte unchanged. It still creates one
allocation scope, exhausts the cgroup-v2 `pids.max` thread budget, invokes the
real allocation-scoped kill write, and proves that the process reaches an
`Ok(())` or typed `io::Error` result instead of panicking or aborting. Its exact
`/// CONTRACT_SHAPE: unbounded-preservation.` line, real-kernel/Lima boundary,
pending marker, setup, typed-error assertion, and cleanup remain intact.

The wording removes obsolete per-TAP and shared-network-supervisor kill
ownership without deleting or weakening the VM lifecycle evidence. It neither
redirects E23 to the deleted `VmKillCapability` nor broadens it to a whole-slice
kill.

## Private beacon fixture migration

The source-local fixture migration is necessary compiler fallout from the
already-approved `BeaconIo` port replacing direct Unix split halves. It uses
only existing surface:

- existing `SimBeaconIo` and `SimVmm`;
- existing `BeaconIo::{bind, accept}`;
- existing test-support scripting `queue_connection` and `guest_close`; and
- the move-only existing `BeaconSocket` returned by `accept`.

`test_beacon_writer` is now async because bind and accept are async. It creates
one empty simulated beacon connection, closes the guest side, accepts the exact
key, and hands the socket plus `Arc<dyn BeaconIo>` to the coordinated private
`BeaconWriter::spawn(io, socket, vmm, control)` shape. The helper returns the
same `Arc<BeaconWriter>` / `Weak<BeaconWriter>` identity pair plus the writer's
private line receiver.

`dropped_gate_sender_releases_watcher_without_stranding` passes that receiver
to the coordinated private `run_exit_watcher` shape. An empty connection whose
guest side is closed yields the same immediate no-report result as the former
closed `UnixStream` peer. The five-second strand detector, matching accepted
session, dropped gate sender, exit-event receipt, allocation identity, and
`Crashed { exit_code: None, signal: None }` assertions are unchanged.

The claim-guard test awaits the helper and retains each returned receiver while
the corresponding accepted-session identity is live. Its Starting,
pre-beacon, replacement-session, originating-session, `EndingInFlight`, and
absent-entry assertions are unchanged. The migration neither manufactures a
new state nor changes which `Arc` identity the guard compares.

No test was added or removed. No behavioral assertion, timeout, Contract Shape
declaration, scenario, lane, public API, production port, adapter, test seam,
or model action was added or weakened. `SimBeaconIo` and all scripting methods
used here pre-existed this alignment; the only new code is private fixture
composition and the compiler-required receiver threading.

## Acceptance-review gates

| Gate | Result | Supplemental evidence |
|---|---|---|
| Exact typed set / C6 closure | PASS | Eight named causes, length 8, generator range `0..8`; the two DESIGN-deleted causes are absent. |
| State and transition coverage / C2 | PASS | Generated terminal transitions and the exhaustive component/cause cross-product retain every assertion. |
| Hexagonal boundary / CM-A | PASS | E23 remains on real `CgroupManager`; the beacon fixture now uses the approved `BeaconIo` port and its existing sim adapter. |
| Business/domain language / CM-B | PASS | Documentation now preserves allocation-scoped VM-lifecycle ownership and the exact fail-stop vocabulary. |
| Complete focused journeys / CM-C | PASS | Fail-stop receipt/refusal, E23 no-panic, watcher release/emission, and accepted-session identity journeys remain complete. |
| Pure fixture isolation / CM-D | PASS | The private helper scripts adapter input; it contains no production decision logic. |
| Observable universe / CM-E | PASS | Existing exact receipts, gate state, exit event, allocation identity, and supervision-map complements are unchanged. |
| Layer/PBT discipline / CM-F | PASS | The core state-machine property remains PBT; the source-local async fixture tests remain deterministic examples. |
| Rich-journey tier / CM-G | PASS / N/A | No new journey or tier is introduced. |
| Real-I/O sad path / CM-H | PASS | E23 stays one explicit real-kernel refusal example. |

The supplemental changes therefore introduce neither
`SPECIFICATION_AMBIGUITY` nor an `AT_GAP_IN_DELIVERY_SCOPE`. Commit
`bc749ad7` pins the public exact set and supplied-owner signatures; the beacon
writer/watcher signature is private crafter-owned implementation structure,
and DISTILL's fixture follows that coordinated structure without inventing
public surface.

## Supplemental mechanical verification

| Check | Result |
|---|---|
| Exact `CAUSES` list, length, and index domain | PASS — eight / eight / `0_u8..8`. |
| Deleted-cause and stale supervisor-kill wording search in the three fail-stop/E23 paths | PASS — zero matches. |
| New test or Contract Shape declaration | PASS — none. |
| Behavioral assertion or timeout diff | PASS — none; only the compile-time cause-length assertion changes from 10 to the approved 8, and two fixture setup assertions validate the existing simulated connection. |
| Direct `UnixStream` fixture use in `vm_driver.rs` | PASS — removed; the existing `BeaconIo` port owns the move-only socket. |
| Concurrent marker activation preservation | PASS — all 476 removed `pending DELIVER` lines remain untouched. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the five completed aligned files outside `vm_driver.rs` | PASS. |
| Whole-file `rustfmt --check` for `vm_driver.rs` | CONCURRENT PRODUCTION WORK — currently reports only active crafter-owned production formatting at the writer construction and beacon-observation projection; no supplemental test hunk appears in the formatter diff. The updated alignment's earlier blanket six-file PASS is therefore stale in the moving shared worktree and is superseded by this precise result. |
| Test execution | NOT RUN — production `BeaconWriter`, `accept_ready`, startup, and watcher call sites are still being coordinated by the original crafter, and the pre-cut guest-network enum mismatch from iteration 1 remains. A transient compiler mismatch is not semantic RED. |
| Metal and mutation testing | NOT RUN — outside this bounded static review and reserved for the prescribed DELIVER gates. |

## Iteration-2 verdict

**APPROVED.** The supplemental alignment contracts the fail-stop test universe
to the exact eight approved causes, preserves every retained gate-model
assertion, keeps E23 on the allocation-scoped VM lifecycle kill path, and
migrates only private source-local beacon fixture composition onto the existing
`BeaconIo`/`SimBeaconIo` boundary. Existing behavioral assertions, the
five-second strand timeout, Contract Shape declarations, test identities, and
observable complements remain unchanged. Concurrent production work and all
476 marker activations are preserved and are not attributed to DISTILL. No new
or weakened test, API, seam, adapter, lane, model, or public contract appears.

# APPROVED — SUPPLEMENTAL ITERATION 2

---

# Supplemental review iteration 3 — `VipRange` property alignment

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 3 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Supplemental DESIGN authority | Commit `07e4553173d96926385e628c2889b17e2e1a0e75`; approved `VipRange::first_overlapping_range` DESIGN supplement |
| Scope | Strengthening the existing P4-S14 property and the coordinated private watcher-fixture argument |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration is additive. It preserves iterations 1 and 2, then reviews the
new bounded alignment in the updated `distill-12-01-alignment.md`.

## Boundary and concurrent-work attribution

The new DISTILL-owned test delta is limited to the existing source-local
P4-S14 property and the private fixture plumbing needed by existing callers:

- `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs`;
  and
- the direct `run_exit_watcher` fixture call in
  `crates/overdrive-worker/src/vm_driver.rs`.

The rest of both dirty files includes active STEP 12-01 crafter work. In
particular, the removed attachment-cut ignore markers in
`prefix_boot_acceptance.rs`, the two removed step-6 markers in `vm_driver.rs`,
and every production `BeaconState`, `BeaconWriter`, startup, accept, and watcher
change are concurrent crafter changes. This review neither authors nor accepts
those production changes. All 476 removed `pending DELIVER` lines remain
untouched and are not attributed to DISTILL.

## P4-S14 property assessment

The existing property
`probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix`
retains its identity, exact
`/// CONTRACT_SHAPE: unbounded-preservation.` declaration, source-local
production-owner boundary, and
`#[ignore = "pending DELIVER step 13"]` marker. The file still contains the
same nine test functions and twenty-four functions overall; no scenario,
property, helper category, lane, or fixture seam was added.

The property is strengthened in three contract-bearing ways:

1. **Valid reserved addresses.** Generated reservation words are mapped into
   generated configured CIDRs before constructing `VipRange`. The existing
   `VipRange::new` precondition then admits only valid non-overlapping ranges,
   in-range reservations, and positive remaining capacity. The identical
   ranges and reservation set are passed through the boot-seam and owner entry
   points. Reservations therefore participate in generated input while the
   expected overlap remains correctly defined over configured CIDRs.
2. **Operator order.** The independent oracle changes from `any(overlap)` to
   `find(overlap)` over the generated vector in its original order. When more
   than one configured CIDR intersects `GUEST_NODE_PREFIX`, the expected value
   is the first such CIDR, exactly as approved DESIGN requires.
3. **Exact typed payload.** Both phase-1 entry points must return
   `GuestNetworkError::GuestPrefixOverlapsVipRange` with
   `prefix == GUEST_NODE_PREFIX` and `vip_range == expected`. Matching an
   arbitrary variant instance is no longer sufficient.

The former behavioral universe remains intact. An overlap still requires zero
boot effects. A disjoint configuration still returns `Ok` and must satisfy the
existing phase-1 witness: one released probe context and no serving context or
later boot effect. The explicit single-range examples, VIP-before-node-address
ordering, node-address properties, owner-placement checks, fd-budget check,
and forwarder-probe property keep every prior assertion. Their only fixture
fallout is passing an empty reservation set to the now-complete helper
signature.

The source oracle is suitably independent of the approved production query:
it computes CIDR intersection from the generated values and compares the
returned typed error. It does not call
`VipRange::first_overlapping_range`, inspect `VipRange`'s private collection,
expand allocatable addresses, or use reservation-sensitive `contains` as its
expected-value source.

## Step ownership and RED classification

Commit `07e455317` pins the exact public signature
`VipRange::first_overlapping_range(&self, other: Ipv4Net) -> Option<Ipv4Net>`
and assigns that primitive plus its phase-1 caller to STEP 12-01. The
strengthened property deliberately remains pending for STEP 13, which owns the
guest-prefix startup-refusal implementation and activation of P4-S14.

The unchanged step-13 marker is therefore correct. This property must not be
activated or presented as STEP 12-01's semantic RED receipt. Iteration 1's
existing owner-composition ordering test remains the bounded STEP 12-01 RED
handoff.

## Private `BeaconState` fixture argument

The direct source-local watcher fixture now passes
`Arc::new(Mutex::new(BeaconState::default()))` to the coordinated private
`run_exit_watcher` signature. This is exact neutral compiler fallout:

- the fixture has no live beacon launch, unread connection, pending read, or
  claim for the watcher to remove;
- clearing the tested allocation from a fresh private state changes nothing;
- the production `LiveMap`, dropped gate sender, line receiver, exit channel,
  cgroup ports, `Weak<BeaconWriter>`, and all observable assertions remain the
  same; and
- the five-second strand timeout remains exactly unchanged.

`BeaconState` and the new watcher parameter are private crafter-owned
implementation structure. Passing their neutral value adds no public API,
port, adapter, test-support seam, scenario, state precondition, or assertion.

## Acceptance-review gates

| Gate | Result | Iteration-3 evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | Generated configurations retain overlapping and disjoint CIDRs, including range sizes and placements around the guest prefix. |
| C2 — state and transition | PASS | Both approved phase-1 entry points still traverse the same refusal/pass transition and observable complement. |
| C3 — cardinality | PASS | The existing `1..5` range vector remains; zero through four generated reservations now vary independently. |
| C4 — lifecycle/idempotency | PASS / N/A | No lifecycle operation changes; each generated composition is created, observed, and exited as before. |
| C5 — mode/decision table | PASS | Boot-seam and owner modes both exercise every generated case. |
| C6 — negative and typed robustness | PASS | The property now requires the exact first offending CIDR and exact guest prefix in the typed error. |
| C7 — environment/interruption | PASS / N/A | No environment, concurrency, or interruption claim changes in this source-local configuration property. |
| CM-A — production boundary | PASS | Both calls use `probe_guest_flow_startup` or the production owner's `probe_startup`; the test does not call the new query. |
| CM-B — domain language | PASS | Configured VIP range, operator order, guest prefix, phase 1, and typed refusal match the accepted vocabulary. |
| CM-C — complete focused journey | PASS | Configuration, phase-1 stimulus, typed result, and no-effect/released-context outcome remain connected. |
| CM-D — fixture isolation | PASS | Reservation generation prepares valid input only; no production overlap logic is embedded in a fixture service. |
| CM-E — observable universe | PASS | Exact error payload plus refusal complement are stronger; no prior observable was removed. |
| CM-F — layer-dependent PBT | PASS | The source-local simulated property remains PBT at the appropriate fast layer. |
| CM-G — two-tier rich journey | PASS / N/A | No new journey or acceptance tier is introduced. |
| CM-H — integration sad paths | PASS / N/A | The changed property is source-local; existing explicit integration examples are unchanged. |

The accepted signature and exact typed mapping close the earlier interface
gap. No `SPECIFICATION_AMBIGUITY` or `AT_GAP_IN_DELIVERY_SCOPE` remains.

## Mechanical verification

| Check | Result |
|---|---|
| Existing property identity and step-13 marker | PASS — unchanged and still pending STEP 13. |
| Test/function counts | PASS — 9 tests and 24 functions before and after. |
| Exact first-overlap oracle | PASS — operator-ordered `find`, exact prefix and exact `vip_range` payload. |
| Reserved-address input | PASS — generated within configured ranges and admitted only through valid `VipRange::new`. |
| Both phase-1 entry points | PASS — `BootSeam` and `Owner` receive the same ranges and reservations. |
| Prior pass/refusal complements | PASS — disjoint witness and no-boot-effect refusal assertions retained. |
| New public/test-support surface | PASS — none. |
| Coordinated private watcher argument | PASS — fresh empty `BeaconState`; no behavioral assertion or timeout change. |
| Concurrent marker preservation | PASS — all 476 removed `pending DELIVER` lines remain untouched; the P4-S14 step-13 marker remains present. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the six non-overlapping aligned Rust files | PASS. |
| Whole-file `vm_driver.rs` formatting | CONCURRENT PRODUCTION WORK — formatter output remains confined to crafter-owned production blocks; the reviewed watcher-fixture argument is already in formatter shape. |
| Test execution | NOT RUN — P4-S14 remains correctly ignored for STEP 13 and the shared tree is in coordinated STEP 12-01 production work. No compile or scaffold failure is treated as semantic RED. |
| Metal and mutation testing | NOT RUN — outside this bounded static DISTILL review. |

## Iteration-3 verdict

**APPROVED.** The existing P4-S14 property is strengthened to cover valid
reserved addresses, operator-supplied CIDR order, and the exact first
overlapping typed payload through both phase-1 entry points while preserving
every prior assertion, complement, Contract Shape declaration, function
identity, and its STEP 13 marker. The fresh private `BeaconState` argument is
neutral compiler fallout for the coordinated watcher signature. No test,
scenario, assertion, timeout, API, seam, adapter, lane, model, or public
contract is added or weakened, and concurrent production and marker changes
remain separately attributed.

# APPROVED — SUPPLEMENTAL ITERATION 3

---

# Supplemental review iteration 4 — `ProbeForwarder` fixture auto-release

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 4 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Fixture-only auto-release of the expected phase-1 `ProbeForwarder` effect |
| Verdict | **CHANGES_REQUIRED** |
| Open findings | 1 blocker |

This iteration reviews the new three-fixture alignment recorded in the updated
`distill-12-01-alignment.md`. It changes only this review artifact.

## Correctly bounded changes

The changes in the three named fixtures are individually correct:

- `owner_acceptance/support.rs::Composition::with_prefix` opts its fresh seam
  into `ProbeForwarder` auto-release. Every owner-acceptance cell and
  conformance driver using that composition can complete an expected startup
  probe without changing the seam's global defaults.
- `owner_acceptance/legs_dial.rs::compose` makes the same local choice for its
  separately constructed composition.
- `prefix_boot_acceptance.rs::Serve::start` makes the same local choice for
  both phase-1 entry points.

Each call is placed immediately after `SimGuestFlowIo::new`. No production
port, test-support API, seam method, model action, scenario, test body, or
assertion was added. The diff of `flow_io/sim.rs` is empty: in particular,
`SimGuestFlowIo::new` still excludes `ProbeForwarder` from its default
auto-release set, so stepped drivers retain explicit control.

The existing seam semantics also preserve failure honesty. `auto_release`
adds only the effect kind to the local `auto` set; `script_failure` continues
to enqueue the next exact `SimFailure`, and `gate` pops that script when the
auto-released effect is issued. The phase-1 `DeviceAbsent`, `AbiMismatch`,
`Backpressure`, and `Cleanup` cases therefore still reach their exact
`ForwarderError` assertions.

No assertion changed with this supplement. The existing issued-effect order,
one probe context, release on success or failure, zero serving-context open,
no later boot effect, and positive phase-1 witness assertions remain present
at `prefix_boot_acceptance.rs:535-635`. The six LEGS-DIAL marker removals and
other concurrent activation/production changes remain crafter-owned and are
not attributed to DISTILL.

## Finding

### F-04-01 — Two existing expected-pass phase-1 fixtures still park forever

- **Severity:** Blocker.
- **Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — the approved effect and fixture seam
  are fully specified; this is incomplete fixture alignment, not a DESIGN
  ambiguity.
- **Claim:** The three added opt-ins do not cover every existing fixture that
  awaits the production phase-1 forwarder probe. Two reachable fixtures still
  construct a fresh `SimGuestFlowIo` without auto-releasing
  `ProbeForwarder`, then await phase 1 with no driver able to release the held
  effect.

#### Reachability evidence

1. `shared_network_test_ports.rs:948-961` constructs the `ComposedNode` boot
   seam with `SimGuestFlowIo::new` and immediately awaits `boot_composed`.
   `boot_composed` calls `probe_guest_flow_boot` at `:998-1003`, which reaches
   the production `probe_guest_flow_startup -> GuestFlowIo::probe_forwarder`
   path. The fixture has no `ProbeForwarder` auto-release or concurrent release
   driver. Its existing `fail_next_forwarder_open` path merely queues a
   `script_failure` at `:955-956`; a script is consumed only after the effect
   is auto-released, so both the pass and scripted-failure boots park.
2. `action_shim/guest_activation_acceptance.rs:219-240` constructs another
   fresh `SimGuestFlowIo`, builds the production owner over it, and directly
   awaits `owner.probe_startup()` as an expected pass. It likewise has no
   `ProbeForwarder` auto-release and no release task, so the fixture parks
   before reaching its existing activation assertions.

This is not a theoretical schedule. `SimGuestFlowIo::probe_forwarder` awaits
`gate(ProbeForwarder)` (`flow_io/sim.rs:1872-1874`). `gate` records and holds
any kind absent from the local `auto` set (`:1447-1467`). The constructor's
default set at `:602-621` does not include `ProbeForwarder`. Neither reachable
fixture calls `release` or `auto_release` for it.

The first omission also affects the already-approved RED/ordering path from
iteration 1: `a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route`
uses `ComposedNode::boot`. Once phase 1 is implemented, that composition must
complete its probe before the existing `OpenForwarder` ordering oracle can be
reached. Leaving the effect held would replace semantic evidence with a hung
fixture.

#### Required bounded remediation

Add the same fixture-local
`io.auto_release(SimEffectKind::ProbeForwarder)` immediately after the fresh
seam is constructed in:

- `shared_network_test_ports.rs::ComposedNode::boot`; and
- `action_shim/guest_activation_acceptance.rs::ShimFixture::build`.

Add only the imports mechanically required by those calls. Do not change
`SimGuestFlowIo::new`, the seam API, any test body, marker, assertion, timeout,
model action, lane, or production behavior. In `ComposedNode`, retain
`fail_next_forwarder_open` and its current `script_failure` so exact typed
failure cases auto-release with the queued cause.

## Acceptance-review gates

| Gate | Result | Evidence |
|---|---|---|
| CM-A — production boundary | PASS for the three changed fixtures | They still drive the production owner/phase-1 entry points over the existing seam. |
| CM-B — domain language | PASS | The fixture choice names the exact `ProbeForwarder` operation. |
| CM-C — complete focused journey | **FAIL overall** | Two existing phase-1 journeys cannot leave the stimulus and reach their typed or positive outcome. |
| CM-D — fixture isolation | PASS for the mechanism | Local `auto_release` is existing test-support scripting; no production logic is duplicated. |
| CM-E — observable universe | PASS for changed fixtures | Ordering, context release, no-later-effect, and positive witness assertions are unchanged. |
| C2 — state/transition | **FAIL overall** | The omitted fixtures remain stuck at the new transition instead of reaching success or typed refusal. |
| C6 — typed negative path | **FAIL for `ComposedNode` scripted failures** | The queued exact failure cannot be consumed while the effect remains held. |

The mechanism is correct and scoped, but its application is incomplete. A
blocker remains until every existing expected-pass phase-1 composition can
complete without globally relaxing the seam.

## Mechanical verification

| Check | Result |
|---|---|
| Added test/scenario/API/seam/model action | PASS — none. |
| `SimGuestFlowIo::new` global default | PASS — unchanged; `ProbeForwarder` remains locally controlled. |
| Existing assertions and Contract Shape declarations | PASS — unchanged. |
| Changed-fixture failure scripting | PASS — scripts remain exact and are consumed on local auto-release. |
| Existing expected-pass fixture inventory | **FAIL** — two reachable constructors remain unaligned. |
| Test execution | NOT RUN — static gate semantics prove the park; the shared tree remains under active production work. |
| Metal and mutation testing | NOT RUN — outside this bounded review. |

## Iteration-4 verdict

**CHANGES_REQUIRED.** The three authored fixture opt-ins are correct, preserve
all existing ordering, context-release, no-later-effect, positive-witness, and
typed-failure assertions, and avoid a global seam relaxation. The alignment is
not complete: `ComposedNode::boot` and `ShimFixture::build` reach the same
production `ProbeForwarder` effect on fresh seams that still hold it forever.
Add the same local opt-in at those two constructors, with no other change, then
return this iteration for re-review.

# CHANGES_REQUIRED — SUPPLEMENTAL ITERATION 4

---

# Supplemental review iteration 5 — F-04-01 closure

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 5 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Bounded remediation of F-04-01 only |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration preserves the iteration-4 rejection as history and reviews only
the two requested fixture-local additions.

## Finding disposition

| Finding | Iteration 4 | Iteration 5 evidence | Disposition |
|---|---|---|---|
| F-04-01 — two expected-pass phase-1 fixtures hold `ProbeForwarder` forever | Blocker | Both omitted fresh seams now opt into local `ProbeForwarder` auto-release immediately after construction | **CLOSED** |

## Closure evidence

`shared_network_test_ports.rs::ComposedNode::boot` now calls
`io.auto_release(SimEffectKind::ProbeForwarder)` directly after
`SimGuestFlowIo::new` and before the optional
`script_failure(ProbeForwarder, failure)`. This order is exact:

- the ordinary boot probe completes with its natural result;
- `fail_next_forwarder_open` still queues the exact typed failure;
- the auto-released effect consumes that queued failure rather than parking;
- the production boot-seam phase can reach its existing refusal or later
  ordering assertions; and
- no global seam default changes.

`action_shim/guest_activation_acceptance.rs::ShimFixture::build` now performs
the same local opt-in directly after its fresh seam is constructed and before
the production owner is built. Its expected successful `owner.probe_startup()`
can therefore complete before the existing activation journey begins.

The five documented fixture sites now cover every current source-local fresh
`SimGuestFlowIo` composition that awaits the production phase-1 probe:

1. `owner_acceptance/support.rs::Composition::with_prefix`;
2. `owner_acceptance/legs_dial.rs::compose`;
3. `prefix_boot_acceptance.rs::Serve::start`;
4. `shared_network_test_ports.rs::ComposedNode::boot`; and
5. `action_shim/guest_activation_acceptance.rs::ShimFixture::build`.

Other direct `SimGuestFlowIo::new` sites do not await phase 1: they exercise the
seam itself, adapter equivalence, or composition lifetime. They require no
fixture opt-in.

## Preservation check

The remediation adds only the two `auto_release` calls and the one import
needed by `ShimFixture`. It changes no test function, scenario, assertion,
timeout, Contract Shape declaration, pending marker, effect-order oracle,
context-release oracle, no-later-effect complement, positive witness, model
action, lane, production API, or seam API.

`SimGuestFlowIo::new` remains unchanged and still does not globally
auto-release `ProbeForwarder`; stepped conformance drivers retain control.
`ComposedNode` retains its existing queued `script_failure` call in the same
position relative to boot, so failure payloads and their exact typed
projections are preserved.

The marker removals and supplied-owner caller fallout visible in the same two
dirty files remain concurrent crafter work. They are not part of this
remediation and are not attributed to DISTILL.

## Mechanical verification

| Check | Result |
|---|---|
| Both F-04-01 fixture sites locally auto-release `ProbeForwarder` | PASS. |
| `ComposedNode` auto-release precedes queued failure scripting | PASS. |
| `SimGuestFlowIo::new` or seam implementation diff | PASS — none. |
| Assertion, timeout, Contract Shape, or marker change from remediation | PASS — none. |
| New test/API/seam/model/fixture type | PASS — none. |
| Updated handoff inventory | PASS — names all five aligned fixtures and preserves exact failure/pass semantics. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the two remediated files | PASS. |
| Test execution | NOT RUN — the shared tree remains in active coordinated STEP 12-01 production work; no compiler or scaffold failure is used as semantic evidence. |
| Metal and mutation testing | NOT RUN — outside this bounded re-review. |

## Iteration-5 verdict

**APPROVED.** F-04-01 is closed. Every existing source-local composition that
awaits the production phase-1 forwarder probe now makes the same fixture-local
auto-release choice, while the global sim default and stepped-driver control
remain unchanged. Expected pass paths no longer park; queued failures retain
their exact typed cause; and all ordering, context-release, no-later-effect,
positive-witness, assertion, timeout, marker, and Contract Shape evidence is
preserved. No new test surface or production contract was introduced.

# APPROVED — SUPPLEMENTAL ITERATION 5

---

# Supplemental review iteration 6 — simulated half-close projection

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 6 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Exact reproduced `SimFlowForwarder` half-close projection correction |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the bounded test-binding correction documented in the
updated handoff. It changes only this review artifact.

## Reproduced semantic contradiction

The existing owner-acceptance test
`the_guests_half_close_reaches_the_destination_after_its_bytes` reproduced the
defect at seed 61. After the guest sent `abc` and half-closed its direction,
the forwarder facts established all of the following simultaneously:

- the destination had received `abc` in order;
- exactly one direction had `source_end == Clean`;
- that direction had `target_shut == true`;
- the opposite direction had no source end;
- the pair remained live and retained its quota place;
- no owner clear-linger or close effect had occurred; and
- no completed or aborted end record had been consumed.

At that same boundary, `peer_end == Some(Eof)` contradicted the accepted
oracle that a one-direction `SHUT_WR` is not a terminal whole-socket event.
The failure is reachable through the production owner over the named sim
arbiter; it is not a theoretical trace or a fabricated state.

## Correction assessment

The correction in
`crates/overdrive-sim/src/adapters/flow_forwarder.rs::target_half_close`
removes only:

```rust
state.peer_end.get_or_insert(SimFarEnd::Eof);
```

It retains `state.sent_shutdown = Some(SimShutdown::Send)`. This is the exact
semantic split:

- `sent_shutdown == Send` records the directional `SHUT_WR` / vsock `0x2`
  delivered after accepted bytes; and
- `peer_end` remains the far side's terminal whole-descriptor outcome,
  projected by the owner's later clean close as `Eof` or by abort/release as
  `Reset`.

No state variant, socket field, method, port, adapter seam, public API, owner
operation, or production mechanism is introduced. The edit is subtractive in
the existing `overdrive-sim` test binding and changes no production crate.

## Oracle preservation and strengthening

The two existing standalone `SimFlowForwarder` assertions are changed from a
premature terminal EOF to an exact absence assertion:

1. after the first clean source end, while the opposite direction and pair are
   still live; and
2. after the standalone forwarder has emitted its one `Completed` record and
   unhooked the pair, but before any owner has closed either descriptor.

These `None` assertions strengthen ownership isolation: the test now fails if
the forwarder fabricates a terminal descriptor event at either directional or
record-completion time.

Every surrounding behavioral assertion remains unchanged:

- bytes cross both directions exactly and in order;
- the first direction records `source_end = Clean`, `target_shut = true`, and
  leaves the opposite direction open;
- the pair emits `ForwardOutcome::Completed` only after both source
  directions end cleanly;
- the target shutdown remains `Send` / `0x2` in both directions;
- moved-byte counts remain exact;
- the completed record remains unique under trailing `0x3` and reset input;
- inventory still reaches zero live pairs with one completion and zero queued
  ends;
- both sockets are unhooked when the pair completes;
- the production owner still clears linger before closing and then observes
  terminal `FarEndEvent::Eof`; and
- abort, context release, and open-direction close paths still project reset /
  `SHUTDOWN Both` and retain their existing assertions.

The owner-acceptance body that exposed the defect is unchanged. Its pending
marker removal is concurrent STEP 12-01 activation and is not part of this
DISTILL correction.

## Acceptance-review gates

| Gate | Result | Iteration-6 evidence |
|---|---|---|
| CM-A — production boundary | PASS | The reproducer drives the production owner; the corrected arbiter remains the named `FlowForwarder` test binding. |
| CM-B — domain language | PASS | Directional half-close, opposite direction, completed record, owner close, EOF, and reset remain distinct. |
| CM-C — complete focused journey | PASS | Bytes, directional end, live-pair retention, second end, completion, owner close, and terminal far-end outcome remain covered. |
| CM-D — fixture isolation | PASS | No business decision moved into a fixture; one false sim projection is removed. |
| CM-E — observable universe | PASS | The new absence assertions close the premature-terminal-event complement while preserving every other socket/record/inventory fact. |
| C2 — state and transition | PASS | One clean direction stays live; two clean directions complete; owner close terminates; abort resets. |
| C4 — lifecycle/idempotency | PASS | Completion, unhook, unique record, trailing-end no-op, and later owner cleanup remain asserted. |
| C6 — negative robustness | PASS | Abort/reset and context-release outcomes are unchanged and remain distinct from clean directional shutdown. |

## Mechanical verification

| Check | Result |
|---|---|
| Arbiter implementation delta | PASS — one obsolete `peer_end` write removed; `sent_shutdown = Send` retained. |
| Existing test surface | PASS — no test or test body added or removed. |
| Assertion delta | PASS — exactly two existing `peer_end` assertions now require `None`; all other assertions remain. |
| Directional byte/order and opposite-direction liveness | PASS — unchanged assertions. |
| Two-direction completion, both `0x2` deliveries, inventory and unhook | PASS — unchanged implementation and assertions. |
| Owner-close terminal EOF | PASS — `close_from_owner(false)` still sets `peer_end = Eof`, and the owner acceptance assertion remains. |
| Abort/release terminal reset | PASS — abort and context-release code and assertions are unchanged. |
| New state/API/seam/mechanism | PASS — none. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the two corrected sim files | PASS. |
| Test execution | NOT RUN by this reviewer — the handoff records the focused seed-61 reproduction; the shared worktree remains under active coordinated STEP 12-01 production work. |
| Metal and mutation testing | NOT RUN — outside this bounded static review. |

## Iteration-6 verdict

**APPROVED.** The reproduced failure proved that the sim projected one
directional half-close both as `sent_shutdown = Send` and as a terminal EOF.
Removing only the terminal projection restores the accepted ownership and
state meanings without weakening behavior: byte order, opposite-direction
liveness, two-direction completion, both directional shutdowns, record
uniqueness, inventory, unhook, later owner-close EOF, and abort/reset outcomes
remain covered. No new test, seam, API, state, model action, or mechanism was
introduced.

# APPROVED — SUPPLEMENTAL ITERATION 6

---

# Supplemental review iteration 7 — pre-attach causes and unclassified ends

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 7 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Two reproduced, bounded corrections in existing simulation test support |
| Verdict | **CHANGES_REQUIRED** |
| Open findings | 1 blocker |

This iteration reviews the corrections documented for owner-acceptance seeds
65, 66, and 68. It changes only this review artifact.

## Pre-attach destination-cause lookup

The `SimGuestFlowIo::peer_of(flow)` correction is valid for the existing DG-3
ordered scenarios. It retains the attached mapping as the first and
authoritative path. Before attach, it now:

1. requires that the requested flow already has its existing `FlowSockets`
   entry;
2. considers only connected `HandleState::Peer` values with no attached flow;
3. returns the candidate only when exactly one exists; and
4. fails the fixture for zero or multiple candidates instead of selecting the
   newest handle or guessing across concurrent flows.

That matches seeds 66 and 68: each has exactly one request whose peer connect
has completed while attach is still pending. The destination reset, or the
destination bytes followed by FIN, therefore reaches the actual sole connected
unattached peer already owned by the scenario. Attach then remains the point
where the existing mapping learns the flow identity.

No correlation map, field, method, handle, seam, API, or model state was added.
The existing bodies and their held-connect order, destination stimuli,
attach-refusal/attach-time classification, exact byte order, typed abort,
clean half-close, owner release, and control-message assertions are unchanged.
The fail-closed zero/multiple behavior prevents this bounded helper from
silently masking ambiguity.

## Unclassified end correction — valid empty-queue path

The `SimFlowForwarder` correction correctly recognizes the reproduced seed-65
state when the attached source is a vsock stream in `Closing`, has no recorded
class, and has an empty receive queue. The precedence remains correct:

- `sk_err` maps first to `SocketError`;
- an explicitly recorded abort maps next to `PeerAbort`;
- an explicitly recorded clean class is excluded from `Unclassified` and
  reaches the existing clean path; and
- only an unclassified `Closing` vsock with no queued bytes maps to
  `AbortCause::Unclassified { side }`.

`set_vsock_state` schedules only an already-installed pair hook. A pre-attach
socket still has no hook and retains its existing attach-refusal semantics.
For seed 65's empty queue, the scheduled run produces the existing one
`Unclassified` end record; the owner releases the flow, resets its peer, emits
one `guest_flow.unclassified_end` warning with the four existing identity
fields, and the forwarder inventory increments `unclassified_eof`. No existing
assertion or body changed.

## Finding

### F-07-01 — Queued bytes drain, but the promised unclassified abort is never re-evaluated

- **Severity:** Blocker.
- **Kind:** `AT_GAP_IN_DELIVERY_SCOPE` — the accepted ordering is explicit;
  this is an incomplete implementation of the existing sim state transition,
  not a DESIGN ambiguity.
- **Claim:** The updated handoff states that queued bytes are forwarded before
  an unclassified EOF is classified. The current run loop forwards the queued
  bytes but becomes `Blocked` after the last byte, so it never returns to
  `source_abort` to emit the promised `Unclassified` record.

#### Reachable trace

1. An attached vsock has one or more bytes in `stream_rx`, no recorded class,
   and enters `SimVsockState::Closing`.
2. `set_vsock_state` schedules its existing hook once
   (`flow_forwarder.rs:1442-1450`).
3. `run_stream_direction` checks `source_abort` before reading
   (`:629-644`). The new unclassified branch declines while `stream_rx` is not
   empty (`:606-613`), correctly preserving byte delivery.
4. The run accepts and consumes one byte. It returns `Again` only while more
   bytes remain, the target accepted less than offered, or
   `clean_source_end` is true (`:645-657`).
5. On the final queued byte, no byte remains, the one-byte offer was fully
   accepted, and no clean class exists. The result is `Blocked`. The one hook
   schedule has been spent; no later event queues another run.

The pair therefore remains live after all queued bytes have drained, with no
`Unclassified` end record, owner release, peer reset, warning, or inventory
increment. Seed 65 does not expose this because its receive queue starts empty,
but the correction's stated queued-byte contract is not satisfied.

#### Required bounded remediation

Preserve the current error/recorded-abort/recorded-clean precedence and byte
delivery, but ensure the same scheduled run sequence re-evaluates the source
after its final queued byte is accepted. Once the queue is empty and the
source remains `Closing` without a recorded class, it must produce exactly the
existing `AbortCause::Unclassified { side }` outcome.

Do not add a test, state field, API, seam, event kind, correlation mechanism,
or production behavior. Do not classify before queued bytes have been
accepted. Keep the existing seed-65 body and all owner/alarm/inventory
assertions unchanged.

## Acceptance-review gates

| Gate | Result | Evidence |
|---|---|---|
| CM-A — production boundary | PASS | Seeds 65/66/68 remain existing production-owner scenarios over the named sim arbiters. |
| CM-B — domain language | PASS | Connected peer, attach, queued bytes, recorded class, unclassified end, owner release, and alarm remain distinct. |
| CM-C — complete focused journey | **FAIL for queued unclassified input** | The transition stops after byte delivery and never reaches end record, owner release, or alarm. |
| CM-D — fixture isolation | PASS | Both corrections reuse existing state and helpers; no production logic or new seam is introduced. |
| CM-E — observable universe | **FAIL for queued unclassified input** | Bytes are observed, but the required end/alarm/inventory complement is absent. |
| C2 — state and transition | **FAIL** | `Closing + unclassified + queued -> drained -> abort` stops in a live state. |
| C6 — negative robustness | PASS for empty queue, **FAIL for queued bytes** | Exact `Unclassified` is produced only when no bytes were present at the first run. |

The pre-attach lookup and empty-queue unclassified path are accepted. Approval
is blocked only by F-07-01.

## Mechanical verification

| Check | Result |
|---|---|
| Existing scenario/test/body count | PASS — no additions or removals. |
| Pre-attach attached-first / sole-candidate / zero-multiple fail-closed behavior | PASS. |
| New peer identity/correlation state | PASS — none. |
| Socket-error and recorded-abort precedence | PASS — unchanged and ahead of `Unclassified`. |
| Recorded-clean classification | PASS — excluded from `Unclassified`; existing clean path retained. |
| Empty-queue seed-65 end/alarm/inventory path | PASS by static path and handoff reproduction. |
| Queued-byte drain followed by unclassified end | **FAIL** — final-byte run returns `Blocked` with no reschedule. |
| Existing assertion, marker, timeout, Contract Shape delta | PASS — none attributable to these corrections. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the three corrected files | PASS. |
| Test execution | NOT RUN by this reviewer — the handoff supplies the bounded reproductions; static transition inspection proves F-07-01. |
| Metal and mutation testing | NOT RUN — outside this bounded review. |

## Iteration-7 verdict

**CHANGES_REQUIRED.** The pre-attach sole-peer lookup is bounded, fail-closed
under ambiguity, and sufficient for the existing ordered DG-3 scenarios. The
empty-queue unclassified correction preserves error/abort/clean precedence and
reaches the existing owner, reset, warning, and inventory oracles. The
queued-byte form remains incomplete: it drains the bytes and then parks the
pair instead of re-evaluating the now-empty unclassified `Closing` source.
Resolve F-07-01 within the existing run machinery and return for re-review.

# CHANGES_REQUIRED — SUPPLEMENTAL ITERATION 7

---

# Supplemental review iteration 8 — F-07-01 re-review and context history

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 8 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | F-07-01 remediation and the existing recovery context-history oracle |
| Verdict | **CHANGES_REQUIRED** |
| Open findings | 1 blocker |

This iteration reviews both items named in the updated handoff. It changes only
this review artifact.

## Finding disposition

| Finding | Iteration 7 | Iteration 8 evidence | Disposition |
|---|---|---|---|
| F-07-01 — queued unclassified bytes drain without a final classification run | Blocker | The worktree still has the iteration-7 run condition unchanged; no final-byte reschedule or equivalent re-evaluation is present | **OPEN** |

## F-07-01 remains open

The handoff states that queued bytes now trigger another existing run after the
last byte is accepted. The reviewed source does not contain that remediation.
`run_stream_direction` still uses this unchanged continuation condition at
`flow_forwarder.rs:651-657`:

```rust
if accepted < bytes.len()
    || source_socket.queued() > 0
    || clean_source_end(&source_socket)
{
    DirectionRun::Again
} else {
    DirectionRun::Blocked
}
```

For an unclassified `Closing` vsock, `clean_source_end` is false. On the final
accepted byte, `accepted == bytes.len()` and `queued() == 0`, so this still
returns `Blocked`. `set_vsock_state` supplied the only hook schedule. No later
event queues another run, and the now-empty source never reaches
`source_abort` again.

The diff of `flow_forwarder.rs` remains the same as iteration 7 for this path:
it adds the empty-queue `Unclassified` branch and schedules the installed hook
from `set_vsock_state`, but makes no change after byte consumption. The claimed
queued-byte correction is present in the handoff prose only.

F-07-01 therefore remains blocking with the same bounded remediation: after
the last queued byte is accepted, the existing run sequence must re-evaluate a
still-`Closing`, classless source and produce the existing
`AbortCause::Unclassified { side }`. Preserve error/recorded-abort/clean
precedence, deliver every queued byte first, and add no test, state, API, seam,
or mechanism.

## Recovery context-history oracle

The independent recovery-oracle adjustment is correct and approved. The
existing
`the_forwarder_recovery_ends_streams_then_reopens_the_context` body now names
the complete QPROJ-8 history instead of asserting the stale count of two:

1. `contexts[0]` is the released, empty phase-1 probe context;
2. `contexts[1]` is the released, empty original serving context; and
3. `contexts[2]` is the open, initially empty replacement serving context.

The exact count of three is required by the real sequence on the same
`SimForwarderDevice`: phase 1 opens and releases its own context, boot opens
the first serving context, and recovery closes that context before opening its
replacement. The assertion keeps the full ordered table. It does not filter or
drop the probe row, clear device history, or create another device to retain a
stale count.

The separate phase-1 witness remains unchanged and still requires exactly one
released empty row immediately after phase 1. The owner observation remains
serving-generation-only: the probe context never advances
`forwarder_generation`; recovery still advances from the original serving
generation to its next value.

All other recovery assertions remain unchanged: inventory failure identifies
the `Forwarder` component, the first hold aborts the stream, close-old precedes
open-new, the old context is empty, the replacement begins empty, the audit is
clean, restore reopens, and a new dial pairs under the new serving generation.
The marker removal in the same file is concurrent crafter activation and is
not attributed to this alignment.

## Mechanical verification

| Check | Result |
|---|---|
| F-07-01 final-byte re-evaluation in source | **FAIL — absent.** |
| Socket-error / recorded-abort / clean precedence | PASS — unchanged. |
| Recovery context count and ordered row meanings | PASS — exact three-row history asserted. |
| Phase-1 one-row witness | PASS — unchanged. |
| Serving generation semantics | PASS — probe excluded; recovery advances once. |
| Recovery ordering, stream end, audit, restore, and new dial assertions | PASS — unchanged. |
| New test/API/seam/state/mechanism | PASS — none. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the two reviewed files | PASS. |
| Test execution | NOT RUN by this reviewer — the source-level transition still proves F-07-01; the handoff records active crafter reruns. |
| Metal and mutation testing | NOT RUN — outside this bounded review. |

## Iteration-8 verdict

**CHANGES_REQUIRED.** The exact three-row recovery history is an honest
strengthening and preserves phase-1 isolation, serving generation, recovery
ordering, leak detection, and every downstream oracle. F-07-01 is not closed:
the claimed final-byte reschedule is absent from the reviewed source, whose
unchanged condition still returns `Blocked` after the last queued byte. Land
the bounded run-sequence remediation, then return the finding for re-review.

# CHANGES_REQUIRED — SUPPLEMENTAL ITERATION 8

---

# Supplemental review iteration 9 — F-07-01 closure and split VMM exit

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 9 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | F-07-01 source remediation, bounded pre-attach scheduling, split VMM-exit fixture ordering, and context-oracle disposition |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration preserves iterations 7 and 8 as the rejection history, then
reviews the source now present in the shared worktree. It changes only this
review artifact.

## Finding disposition

| Finding | Iteration 8 | Iteration 9 evidence | Disposition |
|---|---|---|---|
| F-07-01 — queued unclassified bytes drain without a final classification run | Open blocker | The final accepted byte now returns `Again` when the source is the classless, empty, `Closing` vsock; the next existing run emits `Unclassified` | **CLOSED** |

## F-07-01 closure

The corrected implementation factors the existing predicate into
`unclassified_source_end`. It is true only for a vsock stream that is
`Closing`, has no recorded class, and has no queued receive bytes.

The run sequence is now complete:

1. `set_vsock_state(Closing)` schedules the already-installed pair hook.
2. `source_abort` preserves precedence: socket error first, recorded abort
   second, then classless/empty `Closing` as `Unclassified`.
3. While queued bytes remain, the predicate is false and the normal stream
   path accepts and consumes bytes in order.
4. After the final accepted byte, the continuation condition observes
   `unclassified_source_end == true` and returns `DirectionRun::Again`.
5. The next existing run reaches `source_abort` with an empty queue and emits
   exactly `AbortCause::Unclassified { side }`.

The remediation adds no event, timer, state, queue, API, seam, test, or model
action. It does not classify before byte delivery. A target that accepts zero
still blocks normally; a later target-resume schedule continues the existing
run machinery. If a socket error or recorded class appears before the next
run, the existing precedence remains authoritative.

The seed-65 owner body remains unchanged and continues to require release,
peer reset, one `guest_flow.unclassified_end` warning with CID/flow/side/
generation, and one `unclassified_eof` increment. F-07-01 is closed.

## Pre-attach peer scheduling

The refined `peer_of(flow)` guard also matches the reproduced seeds 66 and 68
without fabricating identity:

- an attached mapping remains authoritative;
- before attach, the requested flow must already have its existing vsock
  entry;
- any still-held `ConnectPeer` effect blocks the stimulus;
- after the held connect is released, its effect is eligible even if the owner
  future has not yet been polled to write the peer address into test-state
  bookkeeping; and
- exactly one unattached peer must exist. Zero or multiple candidates fail the
  fixture.

Thus the existing DG-3 ordered schedule can apply the destination reset, or
bytes followed by FIN, in the interval after connect completion and before
attach. The helper does not choose among parallel peers, seed a flow mapping,
or add correlation state. Attach remains the operation that records the peer's
flow identity. All seed-66 attach-refusal/typed-abort assertions and seed-68
byte-order/clean-half-close assertions remain unchanged.

## Split VMM-exit fixture ordering

The seed-60 correction is also bounded and necessary. The existing split
fixture represents two independently ordered observations of one VMM exit:

1. `vm_stop_before_callbacks` makes the device-release socket error and state
   readable, but does not run the forwarder callbacks; and
2. `finish_sweep` reports those errors, allowing the existing watch end record
   to enter the owner's queue.

Previously the first step also auto-released accepted-connection loss. During
the intentional gap, the owner could observe control-session loss and abort the
watch itself, replacing the test's existing “record in flight resolves first”
schedule with a different valid owner-abort schedule.

The first step now sets the existing `peer_gone` fact but keeps the existing
`peer_gone_released` bit false. `finish_sweep` still runs every existing socket
callback. Drivers that need accepted-connection loss release it through the
existing `release_peer_gone` operation. No state, method, event, API, seam, or
schedule mechanism is added.

The combined `vm_stop` convenience path preserves its former auto-delivery
contract: it performs both sweep steps and then, only when auto-delivery is
enabled, releases accepted-connection loss and wakes owner tasks. Existing
Quint drivers with explicit ordering keep their `release_peer_gone` control.

This makes the existing seed-60 schedule reachable without inventing a
production state. The kernel/device callback and accepted connection read are
independent observations whose owner processing may occur in either order. The
body's unchanged assertion that the stream waits for its in-flight watch record
is therefore evidence for one real ordering, while other existing cells retain
the session-loss-first ordering.

## Recovery context oracle disposition

Iteration 8's approval of the recovery context-history correction remains
unchanged. The existing seed-61 recovery oracle names all three rows on the
same device: released empty phase-1 probe, released empty original serving
context, and open empty replacement serving context. It does not filter
history. The separate phase-1 witness remains one released row, and the owner
generation still counts only serving contexts.

## Acceptance-review gates

| Gate | Result | Iteration-9 evidence |
|---|---|---|
| CM-A — production boundary | PASS | Seeds 60/65/66/68 retain their existing production-owner paths over the named sim arbiters. |
| CM-B — domain language | PASS | Connect completion, attach identity, callback, accepted-connection loss, queued bytes, end record, and owner release remain distinct. |
| CM-C — complete focused journeys | PASS | Every stimulus can now reach its unchanged typed/ordering/output oracle. |
| CM-D — fixture isolation | PASS | Corrections alter only existing test-binding scheduling and projection; no production decision is duplicated. |
| CM-E — observable universe | PASS | Byte order, exact causes, owner release, alarms, inventory, record-in-flight, and full context history remain asserted. |
| C2 — state and transition | PASS | Final-byte drain reaches unclassified abort; split VMM-exit transitions remain independently orderable. |
| C4 — lifecycle/idempotency | PASS | Combined VMM stop preserves prior convenience behavior; split steps preserve explicit driver control. |
| C6 — negative robustness | PASS | Zero/multiple pre-attach peers fail closed; exact socket/abort/unclassified precedence is retained. |

## Mechanical verification

| Check | Result |
|---|---|
| F-07-01 final-byte re-evaluation | PASS — `unclassified_source_end` participates in the post-consume `Again` condition. |
| Error / recorded-abort / recorded-clean precedence | PASS — unchanged and ordered ahead of `Unclassified`. |
| Bytes before unclassified end | PASS — the predicate requires an empty source queue. |
| Pre-attach held-connect and identity guard | PASS — held effect rejected; sole unattached peer required; zero/multiple fail. |
| Split VMM callback vs accepted-loss ordering | PASS — existing release bit/operation reused; no new state or seam. |
| Combined `vm_stop` auto-delivery | PASS — restored after `finish_sweep` when enabled. |
| Seed-61 three-context oracle | PASS — prior approval incorporated unchanged. |
| Existing test/body/assertion/timeout/Contract Shape surface | PASS — no new body or weakened assertion attributable to these corrections. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the three reviewed files | PASS. |
| Test execution | NOT RUN by this reviewer — the handoff records active focused reruns; this review is bounded to source and oracle integrity. |
| Metal and mutation testing | NOT RUN — outside this bounded review. |

## Iteration-9 verdict

**APPROVED.** F-07-01 is closed by an actual final-byte re-evaluation in the
existing run machinery. The bounded pre-attach lookup refuses held or ambiguous
peers without inventing identity, and the split VMM-exit fixture now preserves
the authored watch-record-first schedule while the combined helper retains
auto-delivery. The exact three-context recovery oracle remains approved. No
test body, assertion, timeout, state, API, seam, event, model action, or
production mechanism was added or weakened.

# APPROVED — SUPPLEMENTAL ITERATION 9

---

# Supplemental review iteration 10 — QPROJ-6 multi-recovery property

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 10 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Strengthening the existing generated EXEC-gate property to the approved QPROJ-6 multi-recovery contract |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the bounded update to
`crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs`. It changes
only this review artifact.

## Contract mapping

Approved update-table row 12 changes the public recovery surface from one
optional recovery to an ordered vector of every active component recovery.
The current public contract additionally pins:

- `begin_recovery(component)` starts the first recovery from Open, adds a
  distinct component while already Recovering, and rejects a duplicate;
- `complete_attempt(remaining)` records one completed repair-and-full-audit
  cycle for components active at the cycle's start, removes recovered
  components, does not charge a component that joined during that cycle, and
  never invents a recovery merely because `remaining` names one;
- an empty tracked set reopens the gate;
- `recovery_progress()` returns every tracked component in component `Ord`
  with its exact attempt count; and
- fail-stop reports the deterministic latest tracked component and its count,
  then closes the gate with first-request-wins behavior.

The updated generated property covers each contract point through the public
`GuestNetworkExecSupervisor` and `GuestNetworkExecGate` capabilities.

## Generated reference model assessment

The existing property remains one test and one generated operation family. Its
reference state expands from a single component/count to:

- an ordered component-to-attempt map;
- the set of components that joined since the previous completed cycle; and
- the latest component required for the public fail-stop receipt.

This is an independent value model of the public transition contract. It does
not read or instantiate private `RecoveringState`, `RecoverySnapshot`, gate
state, clocks, notifications, waiter lists, or active-claim counters. The
generated commands call only public methods, and the oracle compares only
public return values, progress rows, claim behavior, and fail-stop receipts.
Using an ordered map and a joined set is the direct mathematical representation
of QPROJ-6's public rules, rather than an assertion over implementation fields.

`CompleteAttempt` now carries a nine-bit mask. The generator domain
`0 .. 2^9` contains all 512 possible
`BTreeSet<SharedGuestNetworkComponent>` values, including empty, singleton,
many, every tracked subset, and sets containing untracked components. The
reference model retains only already tracked members. Consequently a passing
implementation cannot create a recovery from an arbitrary `remaining` member.

The attempt rule is exact:

- a component active before the cycle increments once if retained;
- a component added while Recovering starts at zero and is marked joined, so
  the current cycle does not increment it;
- recovered components leave;
- the joined set clears after completion; and
- later completed cycles increment every retained component then active at
  their start.

After every generated operation the property compares the complete vector,
not a selected row: equal length, component identity in `Ord` order, and exact
attempt count for each entry. Non-Recovering states require an empty vector.

## Transition and fail-stop preservation

The `BeginRecovery` generator now accepts a distinct component during
Recovering and rejects a duplicate. The existing deterministic illegal-event
cell is aligned to the same rule: adding `Dns` during a `Forwarder` recovery is
legal, while adding `Forwarder` again is rejected. BootClosed, duplicate Open,
completion outside Recovering, and every post-FailStop operation remain
refused.

Fail-stop remains closed and deterministic. For every generated state it
checks the exact cause, selected component, and that component's attempt count.
A second fail-stop returns no receipt. The existing exhaustive component ×
eight-cause example remains unchanged and still checks first-request-wins,
empty progress, and refusal of future claims.

Claim semantics are not weakened:

- Open grants and tracks claims;
- BootClosed and Recovering park claims and the property verifies the waiter
  is unfinished;
- reopening releases parked claims;
- FailStop wakes parked claims with `None` and refuses future claims; and
- the existing deterministic cell still proves a pre-detection claim is not
  revoked by FailStop.

## Acceptance-review gates

| Gate | Result | Iteration-10 evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | Empty/full masks, singleton and many component sets, duplicates, and state boundaries are in the generated domain. |
| C2 — state and transition | PASS | BootClosed/Open/multi-Recovering/FailStop, distinct joins, duplicate refusal, completion, reopen, and terminal behavior are modeled. |
| C3 — cardinality | PASS | All 512 remaining-component sets cover zero, one, and many across nine components. |
| C4 — lifecycle/idempotency | PASS | Duplicate begin and repeated fail-stop are refused; cycles add, retain, remove, reopen, or terminate without resurrecting state. |
| C5 — decision table | PASS | Every component and every retained fail-stop cause remains generated or exhaustively crossed. |
| C6 — negative robustness | PASS | Untracked `remaining` members do not become recoveries; illegal state operations are rejected. |
| C7 — concurrency/interruption | PASS for this layer | Parked claims span recovery joins/completions and are released or refused only by the public gate transition. |
| CM-A — public boundary | PASS | The property calls only the paired public gate/supervisor capabilities. |
| CM-B — domain language | PASS | Component recovery, completed attempt, remaining set, progress, claim, and fail-stop retain exact domain meanings. |
| CM-C — complete focused journey | PASS | Generated sequences cover detection through join/repair/read-back/reopen or fail-stop and observable claim outcomes. |
| CM-D — pure reference model | PASS | The model is value-only and side-effect free; runtime effects are confined to public calls. |
| CM-E — observable universe | PASS | Full progress vectors, receipts, operation acceptance, and claim completion are compared after every operation. |
| CM-F — layer-dependent PBT | PASS | Stateful/generative exploration remains in the fast in-process core acceptance layer. |

## Mechanical verification

| Check | Result |
|---|---|
| Existing test count | PASS — six tests before and after; no new scenario or body. |
| Remaining-set domain | PASS — masks `0..512`, exactly all subsets of nine components. |
| Distinct join / duplicate refusal | PASS — generated model and existing deterministic cell agree. |
| Joined-cycle attempt semantics | PASS — joined components remain zero for the current completion and increment on later retained cycles. |
| Untracked remaining members | PASS — never inserted into the reference recovery map. |
| Ordered full-vector comparison | PASS — length, ordered component, and attempts checked after every operation. |
| Latest-component fail-stop receipt | PASS — exact component, attempts, cause, and first-request-wins checked. |
| Closed-claim behavior | PASS — wait/refuse/release behavior retained for every gate state. |
| New API/seam/fixture/production state | PASS — none. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` | PASS. |
| Test execution | NOT RUN by this reviewer — the shared worktree remains under active coordinated STEP 12-01 work; review is bounded to contract and oracle integrity. |
| Metal and mutation testing | NOT RUN — outside this bounded review. |

## Iteration-10 verdict

**APPROVED.** The existing generated property now covers the approved QPROJ-6
multi-recovery contract over every component-set input while remaining an
independent public-boundary reference model. It checks distinct joins,
duplicate rejection, joined-cycle attempt accounting, absence of invented
recoveries, ordered complete progress, deterministic fail-stop receipts,
first-request-wins, and fail-closed claims. No test, API, seam, fixture, or
production state was added, and no earlier gate invariant was weakened.

# APPROVED — SUPPLEMENTAL ITERATION 10

---

# Supplemental review iteration 11 — netlink route-interface fixtures

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 11 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| DESIGN authority | Commit `0035249d0aac16c1ae8ef8e9590492337dac4eb0`; approved GW-LOCAL/D26 netlink supplement |
| Scope | Existing fixture/compiler alignment to `FibRouteType` and typed `NetlinkError` |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the DISTILL-owned existing-fixture fallout
documented in the updated handoff. The active `Client`, host adapter,
`GuestFlowIo` trait, owner runtime, and netlink production implementation are
crafter-owned concurrent changes and are outside this acceptance review.

## Exact contract alignment

Approved DESIGN introduces the public semantic route type
`overdrive_netlink::FibRouteType` and preserves `NetlinkError` through the
route lookup boundary. The control-plane trait keeps the accepted crate-private
name only as a type alias:

```rust
pub(crate) use overdrive_netlink::FibRouteType as HostRouteType;
```

There is therefore one representation, not a second enum. Existing fixture
code now imports and scripts `FibRouteType` directly. The substitutions are
mechanical and preserve each exact value: `Unicast`, `Local`, `Broadcast`, and
`Anycast`; unconfigured sim destinations still default to `Unicast`.

`SimGuestFlowIo::route_type` now returns the trait's typed netlink result. Its
existing failure vocabulary has the exact bounded mappings authorized by the
supplement:

| Existing stimulus | Exact result |
|---|---|
| no scripted failure | configured `FibRouteType`, else `FibRouteType::Unicast` |
| `SimFailure::Io(kind)` | `NetlinkError::Connect { source: io::Error::from(kind) }` through the existing typed constructor |
| `SimFailure::Netlink` | `NetlinkError::FibLookupMissing { destination }` with the exact queried address |

No generic string, `FlowIoError`, arbitrary route operation, or invented error
variant replaces the approved cause. Both errors remain non-idempotent at this
boundary and reach the owner's already-approved fail-closed
`FlowRefusal::HostInternal` branch.

## Existing D26 behavior and cleanup

The existing
`tcp_flows_to_host_internal_or_excluded_destinations_are_refused_by_class`
body is unchanged apart from the enum path used by its fixture. It still
drives static exclusions plus local, broadcast, and anycast FIB classes and
asserts:

- the exact `Refused { refusal: HostInternal }` outcome for the route classes;
- the already-accepted guest record socket is closed by the owner;
- the full allocation observation returns to the pre-stimulus value;
- the live control session remains the same;
- unrelated flows and state remain unchanged through that full observation;
  and
- zero `ConnectPeer` effects occur for every refused destination.

The datagram refusal cells retain the analogous association cleanup, quota
release, and no-retained-flow outcomes. The migration does not turn a route
lookup failure into a node failure, leak an accepted socket, open a peer before
classification, or broaden the host-internal rule.

## GW-LOCAL separation

The route-query alignment does not alter the distinct R5-11 mutation contract.
Existing boot cells still assert the exact
`GuestNetworkOperation::LocalRouteConverge` mapping, forwarder/listener/route
ordering, idempotent keep/replace/add behavior, route persistence across stale
sweep, and failed-write non-effect. The sim's existing `GatewayRoute +
SimFailure::Netlink` mapping remains `GuestNetworkError::Netlink`; only the
independent `RouteType` effect uses `FibLookupMissing`.

That separation prevents a D26 read failure from being confused with a
GW-LOCAL route-write failure and preserves both typed owners.

## Scope and non-expansion

The fixture diff changes imports and values only in the existing owner policy,
datagram, activation, shared-node, and Quint compositions. No assertion,
scenario, test function, marker, timeout, Contract Shape declaration, effect
count, cleanup expectation, failure-injection method, adapter seam, or model
action was added or weakened.

The reviewed sim already had `set_route_type`, `RouteType`, `SimFailure::Io`,
and `SimFailure::Netlink`; their signatures/results are aligned to the approved
typed contract. No new test-support capability exists. Production methods
`Client::converge_ipv4_host_local_route`, `Client::ipv4_fib_route_type`, host
mapping, trait implementation, and owner branching remain separately owned by
the crafter and receive no approval from this iteration.

## Acceptance-review gates

| Gate | Result | Iteration-11 evidence |
|---|---|---|
| CM-A — production boundary | PASS | Existing D26/GW-LOCAL scenarios still drive the production owner; only driven-port sim values change type. |
| CM-B — domain language | PASS | FIB route type, host-internal refusal, route convergence, accepted guest socket, and peer connect remain distinct. |
| CM-C — complete focused journey | PASS | D26 input through refusal/cleanup and GW-LOCAL boot through route outcome remain intact. |
| CM-D — fixture isolation | PASS | Existing sim stimuli configure typed adapter results; no policy logic moves into fixtures. |
| CM-E — observable universe | PASS | Refusal, socket cleanup, session/unrelated state, peer-effect count, route state, and ordering assertions are unchanged. |
| C2 — state/transition | PASS | Route read failure remains flow-local; route-write failure remains boot refusal with prior route preserved. |
| C5 — decision table | PASS | Local, broadcast, anycast, unicast/default, and static classes retain their existing cells. |
| C6 — typed negative robustness | PASS | Existing I/O and no-message stimuli now produce exact approved `NetlinkError` variants without flattening. |

## Mechanical verification

| Check | Result |
|---|---|
| Private duplicate route enum | PASS — removed; `HostRouteType` is only the approved alias. |
| Fixture route values | PASS — direct `FibRouteType` with no semantic substitutions. |
| Sim route default | PASS — `Unicast`, unchanged. |
| I/O failure mapping | PASS — typed `NetlinkError::Connect` with the original `io::Error`. |
| Empty successful lookup mapping | PASS — exact `FibLookupMissing { destination }`. |
| D26 refusal, cleanup, session/unrelated-state and zero-peer-connect assertions | PASS — unchanged. |
| R5-11 route assertions and error ownership | PASS — unchanged and separate from `RouteType`. |
| New test/API/seam/failure stimulus | PASS — none. |
| Concurrent production changes | PRESERVED — not attributed to or approved as DISTILL work. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the six aligned fixture files | PASS. |
| Test execution | NOT RUN by this reviewer — the shared tree remains under active coordinated STEP 12-01 production work. |
| Metal and mutation testing | NOT RUN — outside this bounded static review. |

## Iteration-11 verdict

**APPROVED.** Existing fixtures now use the single approved `FibRouteType`
representation, and the sim route read preserves the exact typed netlink
causes: `Connect` for its existing I/O stimulus and
`FibLookupMissing { destination }` for its existing source-less netlink
stimulus. D26 remains fail-closed and flow-local with complete cleanup and zero
peer connects; GW-LOCAL ordering, idempotence, and failed-write invariants are
unchanged. No error, test, seam, or observable contract was invented or
weakened, and concurrent crafter production work remains separately owned.

# APPROVED — SUPPLEMENTAL ITERATION 11

---

# Supplemental review iteration 12 — QHOLD leaked-count fixtures

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 12 |
| Review date | 2026-10-24 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| DESIGN authority | Commit `e9c915e191b9c145e466beee9780507057244bcf`; approved QHOLD audit-interface supplement |
| Scope | Existing test/simulation implementor alignment for `SharedGuestNetworkOwner::leaked_quiescence_holds` |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the DISTILL-owned compiler fallout in existing test
and simulation owner implementations. The production trait declaration,
`HostSharedGuestNetworkOwner`, supervisor audit ordering, and fail-stop branch
remain crafter-owned concurrent work and receive no production approval here.

## Exact interface gate

The reviewed trait surface matches approved DESIGN exactly:

```rust
#[must_use]
fn leaked_quiescence_holds(&self) -> u32;
```

It is synchronous, infallible, read-only, and has no default implementation.
The absence of a default is load-bearing: every concrete, simulated, or
supplied owner must expose its own ledger identity at compile time. No owner can
silently inherit zero and hide a leak.

Repository inventory finds the expected ten implementations and the required
method on all ten: three direct ledger owners and seven delegating adapters.

## Direct owner identity

The two DISTILL-owned direct implementations read the exact existing ledger
that already owns their quiescence lifecycle:

- `TestSharedOwner` returns `self.ledger.leaked()`. Its existing `quiesce` and
  `restore` methods use that same `self.ledger`.
- `SimSharedGuestNetworkOwner` returns
  `self.quiescence.ledger.leaked()`. Its existing hold issue/redeem path and
  inherent leaked-count accessor use that same ledger.

Neither implementation constructs a second `QuiescenceLedger`, stores a
parallel counter, returns a fixture constant, reads a journal, or derives the
answer from hold count. The returned value therefore starts at zero with that
owner, rises only when one of that ledger's holds is dropped unredeemed, and
cannot mask a leaked hold owned by the instance.

## Delegating owner adapters

All seven existing wrappers implement the new method as one direct delegation
to `self.inner.leaked_quiescence_holds()`:

- `ActivationFaultOwner`;
- the mTLS journal owner;
- `RecordingGuestNetwork`;
- the activation-order owner;
- the retiring-cleanup journal owner;
- `LeaseLedger`; and
- `RecordingOwner`.

These adapters already delegate startup, convergence, audit, quiescence, and
restore to the same inner owner. The new delegation therefore preserves one
ledger identity across the complete owner lifecycle. None introduces a ledger,
cache, fallback, journal-derived count, or wrapper-local source of truth.

## Existing QHOLD evidence

The existing R5-32 leaked-hold body remains unchanged. It drives the composed
production owner, whose own `quiesce()` issues the hold that the fixture drops.
Its exact observable witness remains:

- the listener recovery's hold plus the leaked hold produce hold count `2`;
- leaked count is exactly `1`;
- admission remains `Quiesced` and no flow survives;
- restoring the listener recovery leaves holds/leaked/admission exactly
  `(1, 1, Quiesced)`;
- the episode is gone, a dial remains refused, and the leaked hold cannot
  reopen forwarding;
- the next audit tick produces the existing typed
  `LeakedQuiescenceHold` fail-stop; and
- restart constructs a fresh owner with zero holds/leaks and forwarding
  converges open.

The test-local `TestSharedOwner` is used by the separate supplied-owner
composition. Its new direct-ledger method is compiler-aligned for that path;
the R5-32 leak journey itself remains the composed host-owner witness. This
attribution distinction does not change coverage or require a new scenario.

Existing foreign-hold evidence also remains unchanged: a hold from another
ledger is refused, leaks only at its issuing ledger, and does not increment the
owner's reported leak count. That complement protects owner-ledger identity
from being widened to “any hold this owner observed.”

## Scope and preservation

The alignment adds only required trait implementations. It changes no test
body, scenario, assertion, timeout, Contract Shape declaration, hold issuance,
restore behavior, ledger construction, audit stimulus, fail-stop cause, model
variable, lane, or seam. Marker removals and production supervisor/owner edits
visible in the shared worktree remain concurrent crafter changes and are not
attributed to DISTILL.

## Acceptance-review gates

| Gate | Result | Iteration-12 evidence |
|---|---|---|
| CM-A — owner boundary | PASS | The supervisor-facing trait reads the same owner that issues/redeems holds; fixtures do not downcast or use a full diagnostic snapshot. |
| CM-B — domain language | PASS | Owner ledger, issued hold, redeemed hold, leaked hold, cumulative count, audit tick, and fail-stop remain distinct. |
| CM-C — complete focused journey | PASS | Existing R5-32 covers issue, leak, retained quiescence, recovery restore, audit detection, fail-stop, and fresh-owner restart. |
| CM-D — fixture isolation | PASS | Direct owners read existing ledgers; wrappers delegate without business logic. |
| CM-E — observable universe | PASS | Exact holds, leaks, admission, flows, episodes, dial refusal, typed shutdown, and restart complement remain asserted. |
| C2 — state/transition | PASS | Zero → leaked cumulative count → terminal audit outcome → fresh owner zero remains covered. |
| C4 — lifecycle/idempotency | PASS | Leak never redeems or decrements; restart, not a hidden reset, creates the new zero-valued owner. |
| C6 — negative robustness | PASS | No default-zero path, secondary ledger, or wrapper cache can silently suppress the typed leak outcome. |

## Mechanical verification

| Check | Result |
|---|---|
| Exact trait signature and `#[must_use]` | PASS. |
| Default trait implementation | PASS — none. |
| Concrete implementation inventory | PASS — all 10 owners implement the method. |
| Direct test/sim owner mappings | PASS — each reads its existing issuing ledger. |
| Delegating adapter mappings | PASS — seven direct inner-owner delegations. |
| New ledger/cache/constant/journal-derived count | PASS — none. |
| R5-32 leak witness and assertions | PASS — unchanged. |
| Foreign-ledger complement | PASS — unchanged and still owner-specific. |
| New test/scenario/seam/model/lane | PASS — none. |
| Concurrent production changes | PRESERVED — not attributed to or approved by DISTILL. |
| Scoped `git diff --check` | PASS. |
| Focused `rustfmt --check` on the nine aligned test/simulation files | PASS. |
| Test execution | NOT RUN by this reviewer — the shared worktree remains under active coordinated STEP 12-01 production work. |
| Metal and mutation testing | NOT RUN — outside this bounded static review. |

## Iteration-12 verdict

**APPROVED.** The test and simulation owners satisfy the exact no-default
QHOLD interface without introducing an alternate ledger or hiding leaks.
Direct owners read the ledger that already issues their holds; all seven
wrappers delegate to the same inner owner. The existing R5-32 and foreign-hold
oracles remain unchanged and preserve cumulative leak detection, quiescence,
typed fail-stop, and fresh-owner restart behavior. No assertion, test surface,
seam, or lifecycle contract was weakened.

# APPROVED — SUPPLEMENTAL ITERATION 12

---

# Supplemental review iteration 13 — DG-4 typed beacon-bind refusal

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-13` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Accepted DESIGN | `99ab1fb4fed1c5a3e3a91fbf6343251e790df2c3` |
| Scope | Existing `a_failed_beacon_bind_refuses_boot` acceptance cell and its DG-4 handoff entry |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the bounded DISTILL alignment of the existing
failed-beacon-bind cell. The production `VmmBootError` declaration,
`compose_vm_driver` mapping, `run_server` disposition, marker activation, and
all other concurrent STEP 12-01 changes remain crafter-owned and receive no
production approval here.

## Exact typed-error oracle

The added assertion matches the complete approved caller-visible chain:

```rust
ControlPlaneError::VmmBoot(VmmBootError::BeaconBind {
    source: BeaconIntakeError::Bind { port, source },
})
```

Its guard requires `port` to equal `config.vsock_ports.beacon()` from the same
`ServerConfig` moved into `run_server`, and requires the retained `io::Error`
to have the injected `ErrorKind::AddrInUse`. The cell therefore distinguishes
the new semantic refusal from every existing `VmmBootError` variant and proves
that neither the configured `VsockListenPort` nor the underlying bind cause is
flattened at either error boundary.

Capturing `config` and `expected_port` before the existing `refused_boot`
entry is test-local observation only. It adds no production API, seam, error
variant, conversion, fixture state, or alternate execution path. The two new
imports name the exact already-approved public types used by the nested match.

## Existing observable universe is preserved

The cell keeps every prior assertion and adds the typed-chain assertion before
them:

- `!node.beacon.is_bound()` still proves the simulated bind failure published
  no listener;
- `events.refusal_reasons()` must still equal the one-element vector
  `["vm_driver.beacon_bind"]`, preserving both reason and exact event
  cardinality;
- `SimSharedTable::Absent` still proves boot did not reach the later shared
  mTLS table write; and
- the unchanged `refused_boot` helper still rejects an `Ok(ServerHandle)` and
  shuts an unexpected handle down before panicking, so the cell cannot pass if
  the server is published despite the fault.

No assertion, complement, timeout, cleanup witness, or Contract Shape
declaration was removed or relaxed. The function remains the same existing
P4-S13/DG-4 scenario over the production `run_server` composition and the
existing `SimBeaconIo::fail_next_bind` stimulus; there is no new test body,
fixture, lane, model state, or recovery mechanism.

## Capability and exhaustive-match boundary

This file's existing `Node::adapters` fixture composes the VM capability as
`Required`, and this bounded alignment does not claim otherwise. Accepted
DESIGN separately requires the production composition to hard-refuse the same
post-probe beacon-bind error for both `Optional` and `Required`. The roadmap's
DISTILL instruction is narrower and explicit: align this existing failed-bind
scenario to the nested typed cause while preserving its refusal and cleanup
outcome, without adding a scenario. The strengthened Required-composition cell
satisfies that instruction; the two-capability branch implementation remains
crafter and DELIVER-review evidence.

Repository search finds no acceptance-test exhaustive match over
`VmmBootError`. Existing tests select individual variants, while the HTTP
conversion handles the enclosing `ControlPlaneError::VmmBoot` family. No
additional DISTILL match arm, rewritten cell, or invented complement is
needed for compiler alignment.

## Attribution and deletion discipline

The DISTILL-owned change in this file is limited to the two type imports, the
same-config expected-port capture, and the nested typed-error assertion. The
removal of the existing `pending DELIVER attachment-cut step (6)` ignore
marker is the coordinated crafter activation and is preserved without being
attributed to DISTILL. Other marker activations and every production change in
the shared worktree are likewise outside this review.

The handoff accurately records the strengthened oracle, retained cleanup and
event assertions, unchanged server-publication complement, absence of a new
scenario, and absence of other exhaustive-match fallout. It does not convert
the concurrent production mapping into DISTILL authorship.

## Acceptance-review gates

| Gate | Result | Iteration-13 evidence |
|---|---|---|
| Exact typed-error contract | PASS | The nested outer variant, boot variant, intake variant, configured port, and `AddrInUse` cause are all matched. |
| Existing journey preservation | PASS | Same `run_server` entry, `SimBeaconIo` stimulus, Required capability fixture, and refusal helper. |
| Observable universe | PASS | Typed refusal, nothing bound, exactly one bind-specific reason, no later shared-table write, and no published server are retained together. |
| Negative robustness | PASS | Probe variants, generic boot errors, wrong ports, and different I/O kinds cannot satisfy the new match. |
| Test-surface discipline | PASS | One existing body is strengthened; no new test, fixture seam, capability seam, lane, model state, retry, or cleanup mechanism. |
| Compiler fallout | PASS | Only the exact approved error types are imported; no other acceptance exhaustive match requires a new arm. |
| Contract weakening | PASS | No assertion, exact cardinality, cleanup complement, timeout, or Contract Shape declaration is weakened. |
| Concurrent work preservation | PASS | Crafter marker activation and production edits are preserved and excluded from DISTILL attribution. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped `git diff --check` | PASS for the reviewed test, handoff, and review artifact. |
| Focused `rustfmt --edition 2024 --check` | PASS for `shared_guest_network_startup.rs`. |
| Test execution | NOT RUN by this reviewer — the shared worktree remains under active coordinated STEP 12-01 production work. |
| Metal and mutation testing | NOT RUN — outside this bounded static review. |

## Iteration-13 verdict

**APPROVED.** The existing DG-4 bind-refusal cell now requires the exact
approved typed chain and preserves the configured port and injected
`AddrInUse` cause. Its existing no-listener, exactly-one-event,
no-later-write, and no-published-server witnesses remain intact. The alignment
adds no scenario, API, seam, fixture behavior, retry, cleanup mechanism, or
exhaustive-match fallout, and it does not weaken any existing assertion.

# APPROVED — SUPPLEMENTAL ITERATION 13

---

# Supplemental review iteration 14 — in-guest R5-54 runner boundary

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-14` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Authorization | User-approved bounded reuse of the existing GuestKit staging command and guest-suite runner |
| Scope | `xtask/src/guest_kit_prepare.rs`, `crates/overdrive-testing/bin/fwd_guest_suite.rs`, and the corresponding DISTILL handoff entry |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the test-support extension before its native Metal
execution. It does not approve concurrent production implementation, the
coordinated removal of the seven DELIVER ignore markers, or any other dirty
work in the shared STEP 12-01 worktree.

## Prepared artifact identity

`IN_GUEST_LANES` gains exactly one descriptor:

```rust
InGuestLane {
    package: "overdrive-dataplane",
    bin: "integration",
    target: InGuestTarget::Test,
    features: "integration-tests",
}
```

The unchanged shared builder therefore invokes the semantic equivalent of
`cargo test --no-run --package overdrive-dataplane --test integration
--features integration-tests --message-format json`, selects the compiler
artifact whose target name is `integration` and whose profile is a test, and
copies that executable to the existing contract path
`target/guest-kit/in-guest/overdrive-dataplane--integration`.

This is the existing `tests/integration.rs` libtest target. The package
declares `integration-tests` and no `kvm-tests` feature, so the feature set is
exact. The extension does not create another test target, wrap or copy test
source, change a test body, or add a Cargo feature. The only current diff in
`mtls_inbound_guest_relay.rs` is the coordinated crafter removal of the seven
STEP 12-01 ignore markers; all seven test bodies and their Contract Shape
declarations remain unchanged.

## Existing runner composition

The new `mtls-relay` group stays inside the existing
`overdrive-fwd-guest-suite` binary and its fixed dispatch table. It obtains the
prepared dataplane integration binary through the existing
`InGuestTests::prepared` path and uses the existing `GuestRun` image, boot,
execution, result, and cleanup flow. It adds no generic package/filter CLI,
runner binary, fixture trait, public API, or execution framework.

The group uses `FWD_GUEST_BOUNDARY.raw(13)`. Static inventory shows slots
0–6 owned by `guest_boundary.rs`, slots 7–12 by the pre-existing guest-suite
groups, and slot 13 previously unused. The band width is 16, so slot 13 stays
inside the already-registered, disjoint band and requires no CID registry or
lease change.

The shared image path inserts the selected libtest executable at
`IN_GUEST_TESTS_PATH` with `with_shared_libraries: true`. `GuestKit::image`
therefore stages the binary's absolute ELF interpreter and `ldd` library
closure exactly as it does for the existing in-guest suites. The group stages
the stock module for `kit.kernel_release` through the existing
`host_module_file(…, "net/tls/tls", …)` path at
`/lib/overdrive-testing/modules/tls.ko`.

`GuestRun::exec` preserves the established module order: the scripted peer's
`run-with-modules` command loads the staged `vsock_loopback.ko` first and
`tls.ko` second, then execs the staged libtest binary. The scripted PID 1
already mounts `/proc`, `/sys`, and `/dev` and brings loopback up before it
accepts that command. The R5-54 fixture mints its PKI in-process, so the group
needs no additional secret, certificate, network namespace, host listener,
or guest tool.

## Exact selection and result oracle

The runner passes exactly:

```text
--test-threads=1 --nocapture integration::mtls_inbound_guest_relay::
```

The module-qualified libtest filter selects the existing R5-54 module and no
sibling integration module. Source inventory finds exactly seven live
`#[tokio::test]` bodies under that module and zero remaining `#[ignore]`
attributes. The existing `GuestRun::exec` oracle requires process exit zero
and the exact successful summary prefix `test result: ok. 7 passed; 0 failed;`.
An empty filter result, ignored body, failed body, extra newly selected body,
or nonzero process exit therefore fails closed.

The group retains `CELL_BOUND` and serial libtest execution. It does not edit
the seven scenarios, their assertions, their real `HostMtlsEnforcement`
driving port, their scripted `GuestStreamDial`, or their byte ordering,
authentication, typed failure, directional close/reset, and teardown
witnesses.

## Kernel and cleanup boundary

The host process prepares the kernel image and matching module files and
launches the existing Cloud Hypervisor fixture. The libtest executable runs
inside the disposable guest. Its `127.0.0.1` listeners, kTLS `TCP_ULP`
attachments, socket resets, and test threads therefore belong to the guest
kernel rather than the physical Metal host kernel.

On success, the unchanged `GuestRun::finish` asks the scripted peer to power
off and waits up to 30 seconds for the VMM to exit. On early return or failed
cleanup, `GuestKitVm::drop` retains the existing terminate/reap fallback.
`GuestRun`'s work directory removes the image and run artifacts, while the
separate module work directory removes any decompressed `tls.ko` staging.
The extension introduces no host-kernel kTLS operation, persistent guest,
detached process, new cleanup owner, or second lifecycle.

## Scope and ownership

The reviewed diff adds one artifact descriptor and one fixed runner group,
plus documentation and a shared constant for the already-used TLS module
path. The small `run_ulp` reformat substitutes that constant while preserving
its package, binary, slot 10, module, exact test filter, expected count,
timeout, and cleanup behavior.

No existing group, package feature, test body, assertion, lane definition,
nextest group, Metal launcher, public production API, test seam, or scenario
is added or weakened. The user-approved extension only makes the already
authored seven-test R5-54 module executable behind the existing disposable
GuestKit boundary.

## Mechanical verification

| Check | Result |
|---|---|
| Scoped `git diff --check` | PASS for both support files, the handoff, and this review artifact. |
| Focused `rustfmt --edition 2024 --check` | PASS for both support files. |
| Dataplane staged target | PASS — package `overdrive-dataplane`, test target `integration`, feature `integration-tests`. |
| Selected source inventory | PASS — exactly 7 Tokio tests and 0 ignores in `mtls_inbound_guest_relay.rs`. |
| CID inventory | PASS — slot 13 is unused and within the existing 16-slot `FWD_GUEST_BOUNDARY`. |
| Guest kernel prerequisites | PASS by static path inspection — loopback setup, `vsock_loopback.ko`, matching `tls.ko`, ELF interpreter, and shared libraries use existing GuestKit paths. |
| Test/build execution | NOT RUN — this review precedes the explicitly requested native execution gate. |
| Metal and mutation testing | NOT RUN by this reviewer. |

## Approved post-review commands

```text
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=mtls-relay cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

## Iteration-14 verdict

**APPROVED.** The extension stages the unchanged existing dataplane
integration libtest target with its exact declared feature and executes only
the seven existing R5-54 cells inside a disposable GuestKit guest. The
module-qualified filter and exact pass-count oracle fail closed. Existing
loopback, kTLS module, dynamic-library, CID, timeout, shutdown, VMM-reap, and
work-directory ownership remain intact. No test, scenario, lane, framework,
public API, production seam, or physical-host kTLS execution is introduced.

# APPROVED — SUPPLEMENTAL ITERATION 14

---

# Supplemental review iteration 15 — retired Sim attachment state

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-15` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Three bounded simulation/compiler alignments for the accepted attachment cut |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the aligned Sim adapters and the corresponding
handoff entry. Production deletion and implementation elsewhere in the shared
STEP 12-01 worktree remain crafter-owned. The user explicitly directed that
the early R18 `intercept_mark_guard` deletion be left as-is; this review does
not require the field to be restored merely to reproduce the earlier 13-04
timing.

## Exact provisioner-interface alignment

The accepted cross-crate interface is now exactly:

```rust
fn provision(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError>;
```

Both reviewed existing implementations match that one-plan interface:

- the shared invariant-catalogue `NoopNetworkProvisioner`; and
- the provision-failure invariant's `SimPartialFailureNetwork`.

Their `VmTapPlan` imports and parameters are deleted. No replacement argument,
derived TAP plan, overload, adapter method, or test-only public surface was
introduced. `NoopNetworkProvisioner` retains its exact no-I/O `Ok(())`
behavior, while `SimPartialFailureNetwork` continues to observe and fail
through the production `WorkloadNetworkProvisioner` boundary.

The catalogue prose is narrowed from a complete VM plan to the retained
workload-netns plan, and the registered invariant description removes only
the retired TAP noun. Its safety, liveness, cleanup, and smallest-free-slot
claims remain intact.

## Retained provision-failure universe

The invariant's logical artifact set now contains exactly the five retained
allocation-owned structures available from `WorkloadNetnsPlan` and the
production workload-netns path:

- the allocation namespace;
- the host veth end;
- the workload veth end;
- `/etc/netns/<owner>` resolver state; and
- the workload subnet's transit route.

`GuestTap`, `GuestRoute`, and `ReturnRoute` are removed from both the enum and
the artifact constructor because their only input was the deleted
`VmTapPlan`. No retained `WorkloadNetnsPlan` field or cleanup-owned artifact
was dropped.

With a five-element complete set, the seeded failing call still constructs a
non-empty proper subset: rotation remains seed-derived and
`partial_count = 1 + draw % (len - 1)` remains in the range 1–4. Updating the
checker from `created.len() >= 8` to `created.len() >= 5` preserves the same
strict precondition over the smaller exact universe; it does not permit an
empty or complete-set failure fixture.

All retained outcome oracles are byte-for-byte unchanged:

- the failed allocation receives smallest-free slot 1 after the blocker holds
  slot 0;
- the existing typed provision failure converges to durable
  `WorkloadNetnsProvisionFailed { stage: "netns_provision", detail:
  "workload netns provisioning failed" }`;
- exactly one matching lifecycle occurrence is recorded and the failed
  allocation never reaches the VM driver;
- structural teardown removes exactly the created retained-artifact set;
- the failure-boundary snapshot contains zero allocation-owned residue;
- ordering remains failure → teardown start → clean teardown completion →
  successor provision;
- the successor reuses slot 1, reaches `Running`, and reaches the driver; and
- the existing negative control removes one real cleanup observation and must
  still turn the invariant red on the exact-set mismatch.

The only assertion adjustment is the complete-universe cardinality from eight
to five. It is necessary to prevent every legitimate 1–4 element partial set
from being measured against the deleted eight-element universe. No retained
typed cause, order, cleanup complement, zero-residue check, liveness bound,
slot identity, successor witness, or mutation-sensitive negative control is
relaxed.

## Sim intercept projection

`SimMtlsIntercept::state_snapshot` now constructs the current
`InterceptState` shape with the retained fields only:

- the exact recorded `InterceptPostcondition` program;
- `policy_route: true`; and
- the cloned dynamic member set.

The deleted `intercept_mark_guard: true` initializer was compiler fallout from
removing that field from `InterceptState`; no Sim state, branch, stimulus,
program identity, policy-route projection, member projection, listener,
fault slot, or convergence behavior changes. Per the user's explicit “leave
it be” direction, the earlier-than-13-04 deletion timing is accepted and is
not treated as a request to recreate an already-retired field.

## Scope and preservation

The reviewed Rust diff is deletion-led: 14 insertions and 34 deletions across
the three files, with the insertions limited to signature/prose reshaping and
the exact five-element retained vector. It adds no test, invariant, scenario,
fixture action, error, public API, model variable, lane, or mechanism.

Concurrent production changes, marker activations, and other compiler fallout
remain outside this review and are neither attributed to DISTILL nor approved
here. The handoff accurately describes the three owned alignments and does not
claim that obsolete TAP/guest-route outcomes remain part of the post-cut
contract.

## Acceptance-review gates

| Gate | Result | Iteration-15 evidence |
|---|---|---|
| Exact interface | PASS | Both Sim provisioners implement the accepted one-plan signature and carry no `VmTapPlan`. |
| Retained universe | PASS | Namespace, both veth ends, resolver directory, and transit route remain; only TAP-derived artifacts are removed. |
| Proper-subset stimulus | PASS | Five-element universe with selected count 1–4 remains non-empty and incomplete for every seed. |
| Typed failure and durability | PASS | Exact durable cause, single occurrence, and no driver start are unchanged. |
| Cleanup and ordering | PASS | Exact created/removed equality, zero residue, failure-before-cleanup order, and negative control are unchanged. |
| Slot reuse and liveness | PASS | Blocker slot 0, failed/successor slot 1 identity, successor `Running`, and driver-start witness are unchanged. |
| Sim intercept state | PASS | Program, policy route, and members remain projected; only the removed field initializer disappears. |
| Contract weakening | PASS | No retained outcome or oracle is relaxed; cardinality follows the exact post-cut artifact universe. |
| User timing direction | PASS | Early R18 deletion is left in place as explicitly directed. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped `git diff --check` | PASS for the three reviewed Rust files, handoff, and review artifact. |
| Focused `rustfmt --edition 2024 --check` | PASS for all three reviewed Rust files. |
| Retired-symbol search in reviewed Sim paths | PASS — no `VmTapPlan`, retired artifact variant, or `intercept_mark_guard` reference remains. |
| Retained artifact inventory | PASS — exactly namespace, host veth, workload veth, resolver directory, and transit route. |
| Test execution | NOT RUN — outside this bounded coordinated static review. |
| Metal and mutation testing | NOT RUN. |

## Iteration-15 verdict

**APPROVED.** The two existing Sim provisioners match the exact post-cut
one-plan interface. The provision-failure invariant deletes only the three
`VmTapPlan`-derived artifacts, preserves the five actual retained structures,
and keeps its strict non-empty proper-subset, typed failure, exact teardown,
zero-residue, ordering, negative-control, and slot-reuse oracles. The Sim mTLS
projection removes only the user-accepted retired R18 field initializer while
preserving program, policy-route, and member state.

# APPROVED — SUPPLEMENTAL ITERATION 15

---

# Supplemental review iteration 16 — remaining fixture/compiler alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-16` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Bounded retained-fixture alignment, inventoried TAP/TCX test deletion, existing guest-init oracles, and three pre-converged Sim fixture owners |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the authored fixture/test-support changes and deletion
inventory only. Concurrent production implementation, the private
`AppState::new` port capture, `ready_message` implementation, marker
activations, and other compiler fallout remain crafter-owned and receive no
production approval here.

## One-plan fixture port

`RecordingNetnsProvisioner` in the existing guest-activation acceptance
module now implements the accepted interface:

```rust
fn provision(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError>;
```

Its obsolete `VmTapPlan` import and second argument are removed without a
replacement seam. The fixture still increments the same counter and returns
the same result. Its retained feature-disabled contrast still requires one
exec/process-netns provision call, no guest-transport lease or CID, one driver
start with `transport: None`, and a `Running` row. No guest activation,
ordering, EXEC-release, late-activation, or typed-refusal assertion changes as
part of this signature alignment.

Other changes visible in that shared file, including marker activation,
forwarder-probe fixture release, and typed FIB route alignment, are separately
coordinated work already reviewed in earlier iterations and are not
reattributed here.

## Retained address inverse and TAP-only deletion

The source-local module in `veth_provisioner.rs` retains
`assigned_workload_address_inverse_accepts_only_exact_second_usable_hosts`
with its exact pure-function Contract Shape. It still covers slot 0, a middle
slot, and `NET_SLOT_MAX`; accepts each retained workload-netns plan's second
usable address; rejects the subnet network and host-side address; and rejects
addresses outside or above the retained assigned-workload domain.

The removed assertions from that body were only for the deleted guest `/30`
and its TAP gateway. The four deleted properties likewise had no post-cut
subject:

- TAP repair-step projection over `ObservedVmTap` and `VmTapStep`;
- slot-derived persistent TAP naming;
- the guest-side `/30` carve and guest address; and
- the slot-derived VM NIC MAC.

Their fixtures, arbitraries, and `proptest` import disappear with
`VmTapPlan`, `derive_vm_tap_plan`, and the TAP convergence types. Retained
workload-netns inverse coverage is preserved rather than deleting the mixed
module wholesale. No remaining netns/veth address assertion was removed or
rewritten.

## Exact TCX integration deletion

The deleted integration files are exactly the two whole-file entries named by
the accepted attachment-cut inventory:

- `guest_tcx_egress_lifecycle.rs`; and
- `guest_tcx_inventory.rs`.

The former exclusively drove the retired `GuestTcxProgram` egress attachment,
TAP endpoint, link pin, query, detach, and ninth classifier counter. The latter
owned the retired scratch TAP/pin-root fixture and the TCX programs, endpoint
and counter maps, entries, links, pins, inventory identities, adoption, and
schema/ownership checks. Both imported the deleted
`overdrive_dataplane::guest_tcx` module and supplied no assertion for a
retained dataplane path.

Their two declarations and only their associated comments are removed from
`tests/integration.rs`; every retained integration module remains declared,
including `mtls_inbound_guest_relay`. Source search within the current
`overdrive-dataplane` Rust tree finds no surviving `guest_tcx` module import,
qualified call, or integration declaration. The test deletions therefore
follow the production classifier's last-caller cut instead of hiding or
ignoring an unbuildable obsolete test.

## Mechanical import and display alignment

The reviewed supplied-owner fixture no longer needs `PathBuf` or
`SimCgroupFs` after the crafter-owned `boot_server` cgroup parameter removal;
only those stale imports are removed in this alignment. Other visible changes
in `shared_network_test_ports.rs` belong to the earlier QHOLD, route-type,
probe-release, and production-signature coordination and are outside this
iteration's attribution.

The existing guest-owner-thread cell scripts
`std::io::ErrorKind::Interrupted`. `SimGuestOwnerIo` materializes that as an
`io::Error`, and `GuestOwnerFatal::EventWait` renders its source through
`Display`. The standard rendered text is `operation interrupted`; the former
`Interrupted` literal was the enum's debug spelling. Updating this one string
keeps the exact `waiting for guest transport events failed:` prefix and all
substantive oracles: the owner starts on its own thread before READY, reaches
its first seam call, has no control session at READY, and delivers the fatal
result to the handler.

## READY beacon-port oracle

The existing READY cell correctly keeps `port=50234`. Its fixture token names
`beacon=50234`, while `BeaconMessage::Ready` defines `port` as the vsock port
the guest dialed. Replacing that expected value with the historical default
1234 would weaken GUEST-PORTS and accept a real production defect.

The acceptance body and its exact output assertion are unchanged. The
crafter-owned `ready_message` fix now reads `token.ports.beacon`; this review
uses it only to confirm that the preserved test oracle follows the approved
contract and does not treat the production fix as DISTILL authorship.

## Pre-converged Sim owner fixtures

Three fixture composition sites now call `converge_shared().await` on their
one existing owner before passing that same `Arc` to `AppState::new`:

- `ActivationOwner` for the three activation-order failure paths;
- `JournalOwner` for the retiring-cleanup failure path; and
- the existing process-wide `SimSharedGuestNetworkOwner` for the service-kind
  VM terminal/readiness failure path.

This is the boot precondition production establishes before action dispatch.
On the Sim owner, successful convergence records `ForwarderOpen`, sets its
existing converged flag, and makes `flow_ports()` report the existing default
stream/control ports. The fixtures neither write a private `OnceLock`, invent
port values, add a setter, nor construct a second owner. The exact converged
owner is the object subsequently held by `AppState` and used by every driven
convergence tick.

Every new convergence call is awaited and uses `expect` or the fixture's
existing seeded setup failure path. A convergence error therefore fails the
harness visibly before the scenario starts; it cannot be converted to a
successful behavioral verdict. The call does not open the EXEC gate, activate
an allocation, provision a lease, or author a terminal/readiness row. In the
activation fixture its extra owner operation is used only in failure
diagnostics; in the retirement fixture the decorator's behavioral journal
records allocation provision calls, not shared convergence; and the
service-kind body does not use owner-call count as an oracle.

The five existing scenario bodies retain their original behavioral
assertions: the three recovery/latch/fail-stop activation paths, the
failed-removal/address-retention-and-reuse path, and the terminal/readiness
path. Their marker removals are crafter activations, not DISTILL body changes.
The pre-convergence therefore supplies an actual post-boot owner state needed
by the private constructor fallout without manufacturing the failure outcome
under test or concealing a production failure.

## Scope and compiler boundary

This supplemental approval does not claim a clean workspace-wide compile
while the shared crafter worktree is still active. It approves the listed
DISTILL-owned alignments and exact inventoried deletions. Any additional
production or test caller fallout from deleted interfaces remains the
crafter's compiler-resolved responsibility and is outside this static
acceptance review.

## Acceptance-review gates

| Gate | Result | Iteration-16 evidence |
|---|---|---|
| Exact provisioner contract | PASS | The reviewed fixture has the one-plan signature and unchanged call-count behavior. |
| Retained inverse | PASS | Workload second-usable acceptance, boundary slots, and retained negative addresses remain asserted. |
| TAP-only deletion | PASS | Four properties and guest-half assertions have only deleted TAP/guest-NIC subjects. |
| TCX deletion discipline | PASS | Both exact inventoried files and their declarations are deleted after their production module; no dataplane Rust caller remains. |
| Guest fatal display | PASS | Expected text follows `io::Error` Display while retaining operation, timing, no-session, and fatal-handler oracles. |
| READY port contract | PASS | The unchanged 50234 assertion is the token's dialed beacon port, never the legacy default. |
| Fixture state legitimacy | PASS | Each same owner is converged through its real port before constructor capture; no state or port is fabricated. |
| Failure visibility | PASS | Owner convergence is awaited and fail-fast; none of the five scenario failures can be swallowed. |
| Existing behavioral oracles | PASS | Bodies and assertions remain unchanged; only coordinated ignore markers are activated. |
| Public/test surface | PASS | No setter, constructor parameter, public field, new test, lane, model action, or fixture owner is added. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped `git diff --check` | PASS for the reviewed live files, handoff, and review artifact. |
| Focused `rustfmt --edition 2024 --check` | PASS for the eight reviewed live Rust files. |
| Dataplane retired-symbol/declaration search | PASS — no `guest_tcx` Rust reference remains under `overdrive-dataplane`. |
| Supplied-owner stale-import search | PASS — no `PathBuf` or `SimCgroupFs` remains in `shared_network_test_ports.rs`. |
| Deleted file inventory | PASS — exactly the two accepted TCX integration files. |
| Test execution | NOT RUN — outside this bounded coordinated static review. |
| Metal and mutation testing | NOT RUN. |

## Iteration-16 verdict

**APPROVED.** The remaining authored alignments match the accepted cut. The
one-plan fixture preserves its call-count and activation oracles; the retained
workload-address inverse remains live while four TAP-only properties are
deleted with their subjects; and the two exact TCX integration files are
removed with their last dataplane classifier caller. The guest fatal Display
expectation and READY port oracle now express their actual approved semantics.
Converging each existing Sim owner before `AppState::new` reproduces the real
boot precondition, fails visibly on error, and neither fabricates ports nor
manufactures any of the five behavioral outcomes.

# APPROVED — SUPPLEMENTAL ITERATION 16

---

# Supplemental review iteration 17 — guest-relay crypto-provider build support

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-17` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Workspace `ktls` inheritance and the existing `overdrive-dataplane` dev dependency used by the approved in-guest relay runner |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the bounded Cargo feature correction that lets the
already-approved GuestKit runner reach the unchanged R5-54 test contracts.
Other root-manifest and lockfile changes visible in the shared STEP 12-01
worktree are concurrent crafter work and receive no approval here.

## Reproduced failure class and necessity

The first approved guest execution reached the staged
`overdrive-dataplane --test integration` binary and selected all seven existing
relay cells. Each stopped before its behavioral oracle when Rustls attempted
to choose a process-wide `CryptoProvider`: the test graph had both `ring` and
AWS-LC provider features compiled.

The conflict came from one exact edge. The workspace's direct Rustls
dependency already has defaults disabled and explicitly selects `ring`, while
the existing dev-only `ktls` dependency previously inherited ktls 6's default
features. The installed `ktls 6.0.2` manifest declares:

```text
default = ["aws_lc_rs", "tls12"]
aws_lc_rs = ["rustls/aws_lc_rs", "tokio-rustls/aws_lc_rs"]
ring = ["rustls/ring", "tokio-rustls/ring"]
```

Thus the pre-correction test binary compiled the workspace-selected `ring`
provider and the ktls-default AWS-LC provider together. That ambiguity is
test-build plumbing failure before handshake, dial, relay, byte-order,
half-close, reset, or teardown behavior; changing a scenario oracle or
installing a provider inside each body would conceal the actual dependency
cause.

## Exact Cargo inheritance

The correction has the minimum two-level Cargo shape:

```toml
# workspace dependency
ktls = { version = "6", default-features = false }

# overdrive-dataplane dev-dependency
ktls = { workspace = true, default-features = false, features = ["ring"] }
```

Disabling defaults at the workspace declaration is necessary because a member
cannot reliably subtract default features already enabled by the inherited
workspace dependency. The member then selects the provider that the workspace
already uses. Repeating `default-features = false` at the use site keeps the
dev-only edge explicit and prevents a future inheritance edit from silently
reintroducing ktls defaults.

The version requirement remains `6`; the lockfile still resolves
`ktls 6.0.2`. No package, version, source, or checksum changes for `ktls`,
Rustls, Tokio Rustls, `ring`, or AWS-LC are introduced by this correction.
Cargo features are not recorded in the lockfile, so no ktls-related lockfile
diff is expected. Unrelated lockfile changes belong to concurrent production
dependency work.

## Consumer and graph boundary

Workspace-manifest inventory finds one `ktls` consumer:
`overdrive-dataplane`, under `[dev-dependencies]`. Its production kTLS arm is
the existing private raw `setsockopt` implementation and does not link the
`ktls` crate. Changing the workspace dependency defaults therefore changes no
production dependency edge or runtime provider selection.

The resolved feature graph for
`overdrive-dataplane --features integration-tests` shows:

```text
overdrive-dataplane [dev-dependencies]
└── ktls feature "ring"
    ├── ktls v6.0.2
    ├── rustls feature "ring"
    └── tokio-rustls feature "ring"
        └── rustls feature "ring"
```

The provider-filtered graph contains no `aws_lc_rs` or `aws-lc-rs` feature and
no AWS-LC package edge for this package. The direct production Rustls
declaration remains unchanged with `default-features = false` and
`features = ["ring", "tls12"]`; the correction neither adds a runtime
`install_default` call nor changes production TLS behavior.

No other workspace manifest consumes `ktls`, so there is no second dev target
whose provider choice, test binary, or feature contract can regress from this
change.

## Test and support preservation

The seven `mtls_inbound_guest_relay` bodies, their Rustls configurations,
their test-local kTLS peer, Contract Shape declarations, module-qualified
runner filter, exact `7 passed; 0 failed` oracle, GuestKit image, kernel
modules, and cleanup ownership remain unchanged. The correction adds no test,
fixture hook, runtime provider, dependency, feature exposed by an Overdrive
crate, lane, runner group, or production API.

This is compiler/build support for the user-approved existing-test execution
boundary. It requires no further product or architecture decision and does not
expand the reviewed runner's file or scenario scope.

## Acceptance-review gates

| Gate | Result | Iteration-17 evidence |
|---|---|---|
| Failure causality | PASS | The pre-oracle panic follows the simultaneous `ring` and ktls-default AWS-LC provider features. |
| Workspace inheritance | PASS | Defaults are disabled at the inherited declaration and at the single use site. |
| Provider identity | PASS | The dev edge explicitly selects the workspace's existing `ring` provider. |
| Version/dependency discipline | PASS | `ktls` remains 6.0.2; no dependency, package, version, source, or checksum is added or changed. |
| Production graph | PASS | `ktls` remains a dataplane dev dependency; the production raw kTLS path and direct Rustls declaration are unchanged. |
| Other consumers | PASS | No other workspace manifest consumes `ktls`. |
| Runtime mechanism | PASS | No provider-install code, test hook, body edit, API, or runtime branch is added. |
| Existing acceptance surface | PASS | Same seven tests, filter, exact count, GuestKit boundary, and cleanup contract. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped `git diff --check` | PASS for both reviewed manifests, handoff, and review artifact. |
| TOML parse and exact-value assertions | PASS for the workspace and dataplane `ktls` declarations. |
| `cargo tree --locked -p overdrive-dataplane --features integration-tests -e features -i ktls` | PASS — one dev-only consumer with `ktls feature "ring"`. |
| Provider-filtered feature tree | PASS — `ktls`, Rustls, and Tokio Rustls select `ring`; no AWS-LC provider appears. |
| ktls-related lockfile diff | PASS — none. |
| Build/test execution | NOT RUN — no additional test job was needed for this bounded manifest review. |
| Metal and mutation testing | NOT RUN. |

## Iteration-17 verdict

**APPROVED.** Disabling ktls defaults at the workspace inheritance point and
selecting `ring` on the sole dev-only consumer removes the exact dual-provider
cause that stopped the seven relay cells before their contracts. It preserves
the ktls version, direct Rustls configuration, production dependency graph,
runtime provider behavior, test bodies, runner, and every acceptance oracle.

# APPROVED — SUPPLEMENTAL ITERATION 17

---

# Supplemental review iteration 18 — TLS 1.3 wrong-CA client oracle assessment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-18` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing R5-54 wrong-CA cell's client-side refusal oracle; no body or production edit in this assessment |
| Disposition | **APPROVED — bounded existing-oracle alignment** |
| Open findings | 0 blockers to the bounded alignment |

The current body remains unchanged pending the approved alignment below. This
assessment does not approve deleting the client observation, accepting a
timeout, delivering any application plaintext, weakening the typed server
refusal, or adding production alert machinery.

## Accepted authentication outcome

The accepted contracts pin the server-owned security boundary:

- an inbound client certificate that does not chain to
  `IdentityRead::current_bundle()` is refused fail-closed as
  `MtlsEnforcementError::PeerVerificationFailed`;
- authentication completes before `GuestStreamDial::dial`, so the refused
  client causes zero dial calls, creates no leg S, consumes no guest-flow
  quota, and delivers zero plaintext to the guest application; and
- the existing inbound guardrail independently requires the wrong-CA and
  no-certificate refusals to be cause-distinct and the server workload to
  receive zero bytes.

Neither R5-54, LEGS-DIAL, nor the mTLS guardrail pins the client helper's local
`ClientConnection::is_handshaking()` transition to remain true until the
server has verified the client's certificate. They also do not require a TLS
alert to be flushed to the peer before `enforce` returns its typed refusal.

The reproduced cell already satisfies every server-owned outcome: the result
is `PeerVerificationFailed`, `dial.calls()` is empty, and `dial.events()` is
empty. The sole failing assertion is the extra timing proxy
`!join(client)`, where the client thread returns `Client::connect(...).is_ok()`.

## TLS 1.3 protocol proof

Rustls 0.23.43's TLS 1.3 client state machine explains the observation without
relaxing authentication:

1. After validating the server `Finished`, client `ExpectFinished` constructs
   its `Certificate`, `CertificateVerify`, and `Finished` flight.
2. It queues that flight, installs application traffic state with
   `start_traffic`, and transitions directly to `ExpectTraffic`.
   `ClientConnection::is_handshaking()` is therefore false at the client once
   its final flight is queued; TLS 1.3 has no second server `Finished` after
   client authentication for this helper to await.
3. Only after receiving that client flight does the server's
   `ExpectCertificate` call `verify_client_cert`. A wrong-CA result queues a
   fatal certificate alert and returns the verification error.

The test's blocking `Client::connect` loop returns `Ok(Client)` as soon as its
local Rustls state is no longer handshaking. It can therefore return before
the server reads and rejects the exact final flight it just wrote. Treating
that local transition as proof the server accepted the peer is incorrect for
this TLS 1.3 mutual-auth ordering.

## Actual production path

`HostMtlsEnforcement::establish_guest` runs `server_handshake` before the
guest dial. `drive_handshake_server` reads the client's final flight and maps
the Rustls `process_new_packets` failure to the approved
`PeerVerificationFailed`. The `?` then returns out of the blocking handshake
before `establish_guest` reaches `guest_dial.dial`.

Rustls has queued a fatal alert through `send_cert_verify_error_alert`, but the
current production error path does not call `write_tls` after that error. The
owned leg C drops as the failed establishment unwinds, so the client observes
eventual TLS/TCP termination, commonly EOF/reset, rather than necessarily
receiving the alert during `Client::connect`.

That behavior is fail-closed: no guest dial or relay exists, and no
application plaintext crosses the authenticated boundary. Flushing the alert
would be a new production behavior, not a correction required by any accepted
outcome. It is therefore not required for STEP 12-01 and must not be invented
to preserve the invalid boolean timing proxy.

## Approved bounded oracle alignment

The existing wrong-CA body may replace only `!join(client)` with an eventual
client-refusal observation in the same client thread:

- if `Client::connect` returns `Err`, the client is already refused;
- if it returns `Ok`, retain that `Client`, perform a bounded read, retry an
  interrupted read, and require the connection to reach EOF or a terminal
  TLS/TCP error before the existing `WAIT` bound;
- in either case, require zero client-visible application plaintext; a timeout
  or `WouldBlock` terminal result is a failure; and
- retain unchanged the exact server-side `PeerVerificationFailed`, zero dial,
  and zero leg-S-event assertions.

The existing `Client::read_up_to(1)` shape is sufficient because it returns
both the collected plaintext and the terminal result. The alignment needs no
new test, production API, fixture seam, connection mode, or scenario. An EOF,
an authenticated fatal alert, and a TCP reset are equivalent client-side
terminal observations for the accepted server refusal only when the collected
application bytes remain exactly empty and the server-owned oracles above all
hold.

The body and its rustdoc must stop claiming that `Client::connect` itself must
return `Err`; they should state that the server refuses peer verification and
the client observes no application data before the connection ends. This
corrects the observation point. It does not change zero to some, must to may,
or never to sometimes for any accepted security outcome.

## Security and weakening audit

| Concern | Result | Evidence |
|---|---|---|
| Typed server refusal | PRESERVED | Exact `PeerVerificationFailed` match remains mandatory. |
| Authentication before dial | PRESERVED | Dial and leg-S journals must remain empty. |
| Guest plaintext | PRESERVED | No leg S exists, so no client byte reaches the guest application. |
| Client plaintext | STRENGTHENED | The replacement explicitly requires zero received application bytes before terminal. |
| Eventual refusal | PRESERVED | Connect error or bounded terminal read; timeout/continued connection is rejected. |
| Alert delivery | NOT AN ACCEPTED REQUIREMENT | Rustls may queue it, but current production closes without flushing; no contract pins alert receipt. |
| Scenario/API surface | PRESERVED | Same wrong-CA stimulus and body; no production or test seam. |

## Mechanical verification

| Check | Result |
|---|---|
| Current test body | CONFIRMED unchanged except its coordinated DELIVER marker activation. |
| Rustls client TLS 1.3 state order | CONFIRMED from local `rustls-0.23.43/src/client/tls13.rs`: client final flight, `start_traffic`, `ExpectTraffic`. |
| Rustls server verification order | CONFIRMED from local `server/tls13.rs`: server `Finished` precedes `ExpectCertificate::verify_client_cert`. |
| Production refusal path | CONFIRMED: verification error maps typed and exits before `guest_dial.dial`. |
| Alert flushing | CONFIRMED absent on the production error path; queued Rustls alert is not written before leg drop. |
| Scoped `git diff --check` | PASS for the unchanged cell, handoff, and review artifact. |
| Test/Metal/mutation execution | NOT RUN by this static assessment. |

## Iteration-18 disposition

**APPROVED — BOUNDED EXISTING-ORACLE ALIGNMENT.** The `!join(client)` boolean
is not a valid TLS 1.3 server-authentication oracle because the client reaches
local traffic state after sending its final certificate flight, before the
server verifies that flight. Replace that one timing proxy with a bounded
zero-plaintext terminal observation and retain the typed server refusal plus
zero dial and zero leg-S events. Production alert flushing is not required.

# APPROVED — SUPPLEMENTAL ITERATION 18

---

# Supplemental review iteration 19 — broad retained-test compiler fallout

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-19` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Remaining existing test provisioners, deleted VM-TAP-only C3 cells, retained address cleanup, typed activation fixture, retired state projections, and two reproduced Sim fixture outcomes |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the bounded existing-test and fixture fallout
described below. Production interface deletion, private constructor changes,
production worker behavior, marker activations, and other shared-worktree
changes remain crafter-owned.

## Complete one-plan test-support migration

The additional twenty control-plane fixture implementations and three Sim
spike implementations now match the accepted interface:

```rust
fn provision(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError>;
```

Their `VmTapPlan` imports and second parameters are deleted. No implementation
adds a replacement argument, constructs a synthetic guest plan, changes its
return value, or adds a compatibility overload. Existing no-op adapters remain
no-op; counting and failure adapters retain their counters, ordered trace
entries, and typed errors; teardown methods and teardown journals are
unchanged. The sole fixture that retained provision inputs changes its journal
from `(WorkloadNetnsPlan, VmTapPlan)` to the exact surviving
`WorkloadNetnsPlan` and otherwise preserves the same observation point.

Together with the provisioners reviewed in iterations 15 and 16 and the two
crafter-owned production adapters, repository inventory finds 28 current
`WorkloadNetworkProvisioner` implementations and zero Rust reference to
`VmTapPlan`. This is compiler-required caller fallout for one accepted
signature deletion, not a behavior migration or new test surface.

## VM-TAP-only C3 deletion discipline

`alloc_netns_lifecycle.rs` removes exactly the two scenarios whose complete
subject was the retired VM TAP branch:

- restart replacement/convergence of the persistent TAP, guest `/30`, TAP
  gateway, namespace forwarding, return route, and TAP identity; and
- fail-closed handling when the slot-derived TAP name was occupied by an
  incompatible link.

Their TAP observation and restart-seeding helpers disappear with them. The
single wrapper in `veth_provision_idempotent.rs` that invoked the first deleted
scenario is removed as required; it supplied no independent assertion.

The file retains its independent workload-netns outcomes: slot/provision
failure becomes the exact durable `WorkloadNetnsProvisionFailed` row without
leaking a slot; `FinalizeFailed { Stable }` preserves a live Running
allocation and its slot; and a genuine failed terminal still tears down and
releases it. These retained bodies, their typed causes, and their positive and
negative teardown gates remain unchanged. Removing the VM-only workload-netns
and TAP scenario follows the accepted post-cut ownership rule that VM guest
networking belongs to `GuestNetworkPlan`, while the legacy
`WorkloadNetnsPlan` port remains for the exec/process namespace path.

## Retained prior-row cleanup oracle

`terminal_netns_cleanup_from_observed_addr.rs` still seeds a prior Running row
with the exact retained transit `workload_addr`, reconstructs its
`WorkloadNetnsPlan`, dispatches `FinalizeFailed`, requires one exact teardown
plan, and requires the final row to be `Failed`.

The deleted second table row used only `derive_vm_tap_plan(...).guest_addr`.
That address and its inverse ceased to exist with the per-VM TAP plan. Removing
that row does not narrow the surviving cleanup contract: the one retained
address representation still crosses the same production dispatcher and
teardown observation.

## Typed activation-refusal fixture

The mTLS fail-closed fixture retains
`GuestNetworkError::PostconditionMismatch` with exact operation
`ForwardingActivate`. Its retired `LinkUp` fact is replaced by the live
post-cut activation precondition:

```rust
expected: ControlSession { alloc, cid, live: true }
observed: ControlSession { alloc, cid, live: false }
```

Both facts use the allocation and CID from the same `GuestNetworkPlan`. This
is the source-honest retained reason activation cannot proceed; it neither
changes the error family nor broadens the refusal. Existing refusal journaling,
durable failure stage, teardown, lease cleanup, and unrelated-allocation
oracles remain unchanged.

## Retired state projections and imports

The remaining worker/control-plane test doubles construct the current
`InterceptState` from its retained program, policy route, and dynamic members.
The test-local `intercept_mark_guard` storage, initialization, projection, and
successful-converge assignment are deleted with the field. Per the user's
explicit direction to leave the early R18 deletion as-is, this compiler
alignment does not restore an already-retired observation or invent a
replacement bit.

The compiler-fallout files also remove only unused `PathBuf` imports left by
deleted cgroup/VM-TAP fixture arguments. No retained path value or filesystem
oracle disappears through those import cleanups.

Current Rust-source search finds none of `VmTapPlan`, `derive_vm_tap_plan`,
`vm_tap_converge_steps`, `ObservedVmTap`, `VmTapStep`,
`GuestNetworkFact::LinkUp`, or `intercept_mark_guard`.

## Shared mTLS owner fixture precondition

The service-kind VM terminal fixture already composed the same shared mTLS
worker later passed to `AppState`, but had not called its production boot
precondition. It now awaits `start_shared_owner()` before constructing
`AppState` and before the first convergence tick, beside the existing guest
owner convergence.

This is not manufactured state. The call uses the real existing worker port,
is awaited, and fails the fixture visibly on a typed error. It does not author
an allocation row, a probe result, a backend state, or the test's terminal
event. The exact started worker instance is then passed into `AppState`; no
second worker, listener seam, setter, or synthetic success is introduced.
The existing first allocation `Running` oracle and every later terminal,
readiness, no-restart, and dead-backend-ineligibility assertion remain
unchanged.

## Canonical next-fit oracle

The reproduced retiring-cleanup seed proved every accepted cleanup outcome
but falsified the former final assertion that release rewinds the pool cursor.
The pool contract and implementation are explicit: assignment searches from
`next_fit`, advances the cursor after the selected offset, and release removes
the held offset without changing the cursor.

The revised existing-body oracle snapshots the provision journal before the
post-release start, derives the next bounded address from the most recent
assigned address, and requires both:

- the new lease's address; and
- the address injected into the driver's transport assignment

to equal that exact next-fit address. The bounded schedule cannot exhaust or
wrap the `/16`, so `last + 1` is the exact cursor result for this scenario.

All cleanup and no-reuse evidence remains unchanged: the predecessor becomes
Retiring before the refused removal, other starts exercise the held window,
none receives its address while held, the successful retry removes its member
before one exact lease release, and a post-release start succeeds. The test no
longer claims immediate reuse. The release still proves the predecessor's
offset is free for a later cursor wrap; this particular bounded seed does not
claim to execute that wrap.

Correcting the contradicted immediate-reuse expectation is not a weakening of
an accepted outcome. It replaces an unapproved allocator rule with the exact
accepted M-9 next-fit result while preserving the higher-order retire,
cleanup-before-release, no-reuse-while-held, exact-once release, and transport
injection invariants.

## Acceptance-review gates

| Gate | Result | Iteration-19 evidence |
|---|---|---|
| Provisioner signature | PASS | Every Rust implementation has the one-plan interface; retired symbols are absent. |
| Fixture behavior | PASS | No-op, count, typed failure, trace, and teardown behavior remain unchanged. |
| VM-TAP deletion | PASS | Only two VM-TAP-specific C3 scenarios, their helpers, and one wrapper are removed. |
| Retained workload-netns coverage | PASS | Provision failure and both `FinalizeFailed` gate directions remain. |
| Prior-row cleanup | PASS | Exact retained transit-address recovery and teardown remain; only the deleted guest-address row is gone. |
| Typed activation failure | PASS | Same `ForwardingActivate` mismatch with exact retained session identity and liveness. |
| R18 timing direction | PASS | Retired guard projections remain deleted as explicitly directed. |
| Shared-owner start | PASS | Same worker, real awaited port, fail-fast result, unchanged Running/terminal oracles. |
| Next-fit correctness | PASS | Lease and driver spec must equal the exact cursor continuation; no immediate-rewind assumption. |
| Security/lifecycle preservation | PASS | No retained failure, cleanup, release, activation, terminal, or unrelated-state oracle is weakened. |
| Surface discipline | PASS | No test, seam, API, setter, compatibility type, model action, or lane is added. |

## Mechanical verification

| Check | Result |
|---|---|
| Retired-symbol source scan | PASS — all seven named retired symbols are absent from Rust sources. |
| Provisioner implementation inventory | PASS — 28 current implementations, all compiler-aligned to the one-plan trait. |
| Scoped `git diff --check` | PASS for the reviewed test-support and handoff paths. |
| Focused `rustfmt --edition 2024 --check` | PASS for the reviewed live fixture and test files. |
| Test execution | NOT RUN by this bounded static reviewer; reproduced seed outcomes are recorded in the handoff. |
| Metal and mutation testing | NOT RUN. |

## Iteration-19 verdict

**APPROVED.** The broad compiler fallout removes only the deleted VM-TAP/R18
surface and preserves every retained test behavior. The service-kind fixture
now establishes the real shared-owner boot precondition without fabricating a
result. The retiring-cleanup body now asserts canonical next-fit placement
while retaining the complete held/no-reuse, cleanup-before-release, and exact
release evidence.

# APPROVED — SUPPLEMENTAL ITERATION 19

---

# Supplemental review iteration 20 — implemented wrong-CA terminal oracle

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-20` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Final source check of the iteration-18-approved R5-54 wrong-CA oracle alignment |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the implemented client-observation change in the
existing wrong-CA body and its handoff text. The coordinated marker activation
and all production changes remain crafter-owned.

## Exact implementation

The client thread now returns one uniform pair:

- `Client::connect` error → `(empty plaintext, that terminal error)`; or
- successful local TLS 1.3 connect → the result of the existing
  `Client::read_up_to(1)` loop.

The successful-connect branch retries only an empty read interrupted with
`ErrorKind::Interrupted`. It does not discard plaintext: an interrupted result
that already accumulated a byte leaves the loop and fails the subsequent
exact-empty assertion. The one-byte bound is sufficient to falsify the zero-
plaintext contract on the first application byte without waiting for a larger
response.

After the server enforcement result is observed, the body joins the client and
requires `plaintext.is_empty()`. A read-side `Ok(())` means EOF. An error is
accepted as terminal only after explicitly rejecting `WouldBlock`, `TimedOut`,
and a terminal `Interrupted`; the socket retains the existing `WAIT` read
timeout. Consequently a live connection that merely reaches its timeout
cannot be mislabeled as refusal, and an unconsumed retryable interruption
cannot satisfy the terminal oracle.

## Preserved security outcome

The exact server-owned assertions remain unchanged and mandatory:

- `HostMtlsEnforcement::enforce` must return
  `MtlsEnforcementError::PeerVerificationFailed`;
- `dial.calls()` must be empty; and
- `dial.events()` must be empty, proving no leg S existed.

The updated client observation adds the explicit zero-application-plaintext
check and requires bounded termination after an initially successful local
TLS 1.3 state transition. It does not accept a successful continuing
connection, timeout, retryable terminal state, or any application byte. A
connect-time error and a connect-success-then-EOF/TLS/TCP-error remain the two
protocol-equivalent client observations approved in iteration 18.

The rustdoc now states the observable outcome precisely: peer verification
refuses the client, it receives no application plaintext before the connection
ends, and no guest dial occurs. It no longer claims that the client's local
`connect` helper must itself return an error before the server processes the
client certificate flight.

## Scope and weakening audit

| Concern | Result | Iteration-20 evidence |
|---|---|---|
| Typed server refusal | PRESERVED | Exact `PeerVerificationFailed` match unchanged. |
| Authentication-before-dial | PRESERVED | Exact zero-call and zero-event assertions unchanged. |
| Guest application isolation | PRESERVED | No leg S can exist. |
| Client application plaintext | PASS | Exact empty vector is required on every client outcome. |
| Eventual termination | PASS | EOF or non-retryable TLS/TCP error only. |
| Hidden timeout acceptance | PASS | `WouldBlock` and `TimedOut` explicitly fail. |
| Interrupted I/O | PASS | Empty `Interrupted` retries; a terminal `Interrupted` is rejected. |
| Alert requirement | UNCHANGED | EOF and terminal error remain equivalent; production alert flushing is not required. |
| Test/API surface | PRESERVED | Same body and stimulus; no helper, scenario, seam, API, or production branch. |
| Contract weakening | PASS | No zero→some, must→may, or never→sometimes change to an accepted security outcome. |

## Mechanical verification

| Check | Result |
|---|---|
| Source diff inspection | PASS — only the wrong-CA rustdoc and client terminal oracle change beyond coordinated marker activation. |
| `rustfmt --edition 2024 --check` | PASS for `mtls_inbound_guest_relay.rs`. |
| Scoped `git diff --check` | PASS for the test, handoff, and review artifact. |
| Test/Metal/mutation execution | NOT RUN by this bounded final source check. |

## Iteration-20 verdict

**APPROVED.** The implementation exactly realizes the iteration-18
disposition: connect failure is terminal refusal; connect success must be
followed by zero application plaintext and EOF or a non-retryable terminal
TLS/TCP error. Timeout, `WouldBlock`, and terminal `Interrupted` cannot pass.
The typed server refusal and zero dial/leg-S evidence remain unchanged.

# APPROVED — SUPPLEMENTAL ITERATION 20

---

# Supplemental review iteration 21 — broad-gate fixture and generator domains

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-21` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Source-local admission-refusal fixture boot baseline and existing health-check property input domains |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only two DISTILL-owned test-support corrections exposed
by the broad test-target gate. The worker guest-probe branch and the production
activation-loop liveness fixes remain crafter-owned. Their existing test bodies
and oracles are not changed or waived here.

## Admission fixture boot baseline

The source-local `AdmissionFixture` now runs the same required owner boot
precondition as the production composition:

```rust
owner.converge_shared().await.expect("the fixture owner converges before AppState");
```

It invokes that port on the fixture's one existing `TestSharedOwner`, awaits
the typed result, and then passes the same `Arc` into `AppState::new`. It does
not write the private flow-port slot, inject port values, construct a second
owner, add a setter, or bypass convergence. A failed convergence fails fixture
construction before either admission scenario starts.

`TestSharedOwner::converge_shared` records one boot call and publishes its
existing test-owner flow ports. The fixture captures the owner journal length
immediately after that call as `owner_call_baseline`. This keeps the boot
precondition explicit without counting it as an allocation effect.

The baseline is applied consistently at every observation boundary:

- lease-event stamps subtract it from the current owner journal length;
- the cap-refusal no-effect assertion requires the journal length to remain
  exactly at the baseline; and
- the restart-cleanup assertion skips exactly the baseline before comparing
  its allocation call sequence.

The tracing subscriber is installed after fixture construction, so the boot
call cannot fabricate a lease event. Recording the baseline before
`AppState::new` also prevents masking any later constructor or dispatch call:
every journaled owner operation after successful boot remains visible above
the baseline.

The original admission oracles remain exact. A cap-refused successor has no
owner allocation effect, driver start, row, lifecycle occurrence, lifecycle
event, lease, retirement, or release. A refused restart still retires its
predecessor, performs exactly one successful teardown, releases afterward,
and gives the successor no driver start, row, occurrence, or event. Relative
event stamps remain `retire@0`, `release@1`, preserving the ordering witness
around the single teardown call.

## Listener and probe port domains

The three positive properties whose generated port is installed as a Service
listener now use one valid-listener strategy:

```rust
prop_oneof![
    1_u16..GUEST_TCP_INTAKE_PORT,
    (GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS)..=u16::MAX,
]
```

With the accepted constants, those ranges are exactly `1..=60999` and
`62025..=65535`. They exclude the full guest-transport reservation
`61000..=62024`, including both endpoints, while retaining every valid
nonzero listener port below and above it. The affected properties are the
positive HTTP parse, inferred-default startup probe, and explicit-empty
startup opt-out cases; each previously failed for an unrelated reserved
listener before reaching the behavior named by the property.

The general `port_strategy()` still spans `1..=65535`. Every property whose
generated port is only a probe target continues to use it. Probe targets may
legitimately name the reserved host-listener numbers; no guest-listener rule
has been incorrectly projected onto the probe domain.

Dedicated `vsock_deploy_validation` coverage is unchanged. It still proves
refusal iff a declared listener is inside the reservation for either
protocol, exact typed payload/message at representative points, acceptance at
60999 and 62025, and the mixed ordinary/reserved-listener refusal. The
positive parser properties therefore avoid invalid fixtures without removing
any reservation boundary or interior coverage.

## Probe-index property domain

The positional-index property retains its original generated ranges:

- startup: `1..=3`;
- readiness: `0..=3`; and
- liveness: `0..=3`.

It now discards only a generated total above `GUEST_PROBE_SLOTS`. Because the
maximum is 3 + 3 + 3 and the cap is 8, the sole excluded tuple is `(3, 3, 3)`.
Every valid count value for every role remains reachable, including index 2 in
each role and every total through eight. The assertions still require exact
vector lengths, each descriptor's per-role positional index, the correct role,
and global `(role, idx)` uniqueness.

The unchanged dedicated probe-cap properties retain the invalid domain. They
accept eight, require exact `ProbeCountExceeded { count: 9, max: 8 }` across
multiple nine-probe shapes and both ingress representations, and prove by
property that refusal occurs iff the declared-plus-inferred total exceeds the
cap. Excluding the invalid nine-probe tuple from a positive parse property
therefore removes an incoherent precondition without weakening count-refusal
coverage.

## Crafter-owned production failures

The six worker guest-probe failures remain evidence for the missing approved
production branch: guest-prefix TCP/HTTP probes must use the existing
`GuestStreamDial` with `DialPurpose::Probe`. Their test bodies still require
the same dial identity, TCP/HTTP behavior, refusal/drop outcomes, eight-probe
capacity, and zero ordinary-prober calls. No test input or assertion is
changed to accept the current direct-prober behavior.

The three activation timeouts likewise remain production liveness failures.
Their concurrent fixtures and control-session stimuli are unchanged. The
current production retry loop can keep both awaited operations immediately
ready after `ControlSessionPending` and starve the current-thread fixture that
would open the session. The crafter owns the internal yield/liveness fix; no
timeout, zero/once assertion, teardown witness, VMM-death outcome, or
feature-disabled contrast was altered to make the busy loop pass.

## Acceptance-review gates

| Gate | Result | Iteration-21 evidence |
|---|---|---|
| Owner identity | PASS | The exact existing owner is converged and passed to `AppState`. |
| Failure visibility | PASS | Convergence is awaited and fail-fast; no default or synthetic ports. |
| Boot/allocation separation | PASS | Explicit baseline excludes only the one pre-dispatch boot call. |
| Hidden extra calls | PASS | Any owner call after the baseline remains visible in journal length, relative event stamps, and sliced journal. |
| Admission zero-effect oracle | PASS | Driver, row, occurrence, lifecycle, lease, retirement, release, and allocation-owner effects remain forbidden. |
| Restart ordering | PASS | Retire → exactly one teardown → release remains exact. |
| Listener generator | PASS | Exactly the complete valid nonzero domain outside 61000..=62024. |
| Probe generator | PASS | Full nonzero port domain retained for probe-only inputs. |
| Probe-index domain | PASS | Only invalid 3/3/3 is discarded; every valid per-role count remains reachable. |
| Negative/boundary coverage | PASS | Dedicated listener-reservation and probe-cap refusal suites are unchanged. |
| Production defects | PRESERVED | Worker probe and activation-liveness test bodies remain unchanged and failing until crafter fixes. |
| Surface discipline | PASS | No new test, seam, API, fixture owner, port constant, model action, or lane. |

## Mechanical verification

| Check | Result |
|---|---|
| Constant arithmetic | PASS — listener domains are 1..=60999 and 62025..=65535; probe domain is 1..=65535. |
| Generated-count audit | PASS — only 3 + 3 + 3 exceeds the cap of 8. |
| Dedicated complement inventory | PASS — reserved-listener and excess-probe properties remain unchanged. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed Rust files. |
| Scoped `git diff --check` | PASS for both reviewed files, handoff, and review artifact. |
| Test execution | NOT RUN by this bounded static reviewer; the broad-gate outcomes are recorded in the handoff. |
| Metal and mutation testing | NOT RUN. |

## Iteration-21 verdict

**APPROVED.** The admission fixture now establishes the real owner boot
precondition and explicitly removes only that boot call from allocation-effect
accounting. The property generators now exercise their exact valid domains,
while the dedicated reserved-listener and excess-probe properties preserve
the complete invalid-domain coverage. No production defect is accepted by a
test change.

# APPROVED — SUPPLEMENTAL ITERATION 21

---

# Supplemental review iteration 22 — paused activation fixture barrier

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-22` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Five existing pre-session paused-time barriers in `guest_activation_acceptance.rs` |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the DISTILL-owned synchronization used by the
existing guest-activation scenarios. The production activation-loop yield is
crafter-owned and is not attributed to this fixture change.

## Barrier necessity and contract

The former five pre-session `SimGuestFlowIo::settled()` calls were idle
signals implemented through paused Tokio time. They could complete only after
the executor had no runnable task. Once the production activation retry
correctly yielded between `ControlSessionPending` results, that retry remained
runnable while it waited for the guest control session. The old barrier could
therefore prevent the fixture from ever supplying the existing `ControlOpen`
stimulus. It no longer represented the scenario's intended pending-state
witness.

The private `activation_pending` helper observes the exact pending state from
ports already held by `ShimFixture`. Within at most 256 scheduler polls it
requires all of the following at once:

- the pool holds the allocation's lease and exposes its CID and launch token;
- the recording VM driver has started that allocation;
- the owner's observation contains that CID in `Provisioned`;
- the CID has no control session; and
- the recording driver has not released EXEC for that allocation.

An unsuccessful poll performs one `yield_now`; reaching the bound panics with
the lease, recorded starts, and EXEC releases. The helper does not advance
paused time, sleep for a duration, increase a timeout, queue an effect, or
mutate owner state. The 256-poll bound therefore gives deterministic failure
for a missing precondition while allowing the concurrent current-thread task
to progress.

Only after this conjunction holds does each existing body call
`open_control_session`, which queues the same encoded `ControlOpen` on the
same control listener and then awaits actual Sim I/O completion. The
post-session `settled()` in `open_control_session`, the flow-request settle,
the VM-stop settle, and the boot/probe settle remain unchanged. The edit thus
distinguishes scheduler coordination from completion of an actual scripted
I/O effect.

## Scenario and ordering audit

The five replacements cover the original pre-session boundaries only: the
initial and post-restore boundaries in the quiesced activation scenario, plus
one boundary each in the post-teardown refusal, VMM-death/stale-launch, and
guest-transport-enabled paths. No scenario, stimulus, timeout, assertion, or
Contract Shape declaration was added or removed by this alignment.

The quiesced scenario first proves the leased CID is `Provisioned` and EXEC is
unreleased while the hold exists. After restoring the hold, it reaches the
same session-absent pending state again and now explicitly requires the CID
and launch token to equal the original pair before supplying `ControlOpen`.
This strengthens the existing continuity witness and preserves the order:
lease and VM start, quiesced pending state, restore, session-pending state,
control open, Active, then exactly one EXEC release.

The other three paths likewise cannot supply `ControlOpen` before the lease,
VM start, and owner CID exist, and cannot pass the barrier after EXEC has been
released. Their existing typed refusal, cleanup, zero/once release,
VMM-death, stale-launch, and feature-disabled contrast assertions remain
unchanged.

## Acceptance-review gates

| Gate | Result | Iteration-22 evidence |
|---|---|---|
| Intended pending witness | PASS | Lease + matching driver start + `Provisioned` CID + absent session + unreleased EXEC are required together. |
| Stimulus ordering | PASS | Existing `ControlOpen` is queued only after the pending witness. |
| Quiesced continuity | PASS | CID and launch token are now asserted stable across restore before session open. |
| Bounded polling | PASS | Exactly 256 scheduler polls, then diagnostic panic; no unbounded loop. |
| Timing independence | PASS | `yield_now` drives scheduling only; no duration, paused-time advance, or timeout relaxation. |
| Actual I/O completion | PRESERVED | Existing post-stimulus and post-effect `settled()` calls remain in place. |
| Behavioral assertions | PRESERVED | Active/Provisioned, EXEC zero/once, refusal, cleanup, and stale-launch oracles are unchanged. |
| Surface discipline | PASS | Private fixture helper and constant only; no production API, test seam, model action, scenario, or lane. |
| Ownership | PASS | Fixture synchronization is DISTILL-owned; the production retry yield remains crafter-owned. |

## Mechanical verification

| Check | Result |
|---|---|
| Replacement inventory | PASS — exactly five pre-session idle barriers use the bounded pending-state observation. |
| Later settle inventory | PASS — actual Sim I/O and VM-stop completion barriers remain unchanged. |
| `rustfmt --edition 2024 --check` | PASS for `guest_activation_acceptance.rs`. |
| Scoped `git diff --check` | PASS for the test, handoff, and review artifact. |
| Test/Metal/mutation execution | NOT RUN by this bounded static review. |

## Iteration-22 verdict

**APPROVED.** The bounded private fixture poll observes the real state the
five existing scenarios require before opening the control session. It
preserves stimulus order and every behavioral oracle, strengthens the
quiesced CID/token continuity witness, and introduces no timing concession,
production surface, test seam, or new scenario.

# APPROVED — SUPPLEMENTAL ITERATION 22

---

# Supplemental review iteration 23 — positive activation fixture cleanup

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-23` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing `StopAllocation` cleanup appended to the two positive activation cells |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only fixture end-of-scenario cleanup. Both positive
cells had reached their intended Running/Active outcome and passed every
behavioral assertion, but then dropped the fixture while its live control
session still owned one Sim record handle. The unchanged seam destructor
correctly reported that owned handle.

## Owned-handle and teardown path

The surviving handle is expected live state at the positive outcome, not an
unattributed fixture artifact. `open_control_session` queues the existing
control connection; the owner's control listener accepts it as a
`RecordSocket`; `spawn_control` moves that socket into the allocation's
tracked control task; and the decoded `ControlOpen` records the session on the
CID. While the allocation remains Active, that task and record socket must
remain live.

Each affected body now dispatches the existing product action
`StopAllocation { terminal: None }` after its last outcome assertion. The
normal stop path:

1. invokes the recorded VM driver's awaited `stop`;
2. retires the allocation's guest-network lease;
3. calls `HostSharedGuestNetworkOwner::teardown` through the existing
   `GuestNetworkProvisioner` port;
4. marks the CID `Retiring`, cancels its session and tracked tasks, and awaits
   those tasks;
5. lets the control task run its existing `io.close(socket)` path;
6. verifies that no session, unread connection, guest flow, dial, datagram,
   tracked task, or pair remains before removing the CID; and
7. releases the pool lease only after teardown succeeds.

The fixture awaits the full dispatch and requires `Ok`. This is the same
owner and lifecycle path used for an ordinary allocation stop; it neither
drops the record socket directly nor reaches into Sim state.

## Oracle and leak audit

In `exec_is_released_only_after_the_allocation_is_active`, the existing
assertions still first require an Active CID and exactly one EXEC release. In
`an_allocation_without_a_guest_transport_runs_as_today`, the existing
assertions still first require the disabled allocation's current behavior and
exactly one EXEC release for each allocation, with the composed allocation
released only after activation. Cleanup occurs after those final assertions,
so the tests cannot obtain their claimed positive outcome by stopping early.

The enabled allocation is the only half of the contrast that owns a guest
control-session record handle. The feature-disabled allocation has no guest
lease, CID, activation, or control socket, so no guest-network teardown is
needed to discharge the seam invariant for that half.

The cleanup does not conceal a production leak:

- a driver or owner teardown error fails the added `expect`;
- owner teardown returns success only after its exact per-CID inventory is
  empty;
- pool release remains ordered after successful owner teardown; and
- `SimGuestFlowIo::drop` still counts every remaining seam handle and panics
  if any was not aborted, closed, or consumed.

Consequently, a falsely successful teardown that left the record handle live
would still trigger the same destructor oracle. The change supplies the
missing normal lifecycle event instead of suppressing that oracle or clearing
the seam out of band.

## Acceptance-review gates

| Gate | Result | Iteration-23 evidence |
|---|---|---|
| Positive outcome | PRESERVED | All Active/Running and exact EXEC assertions execute before cleanup. |
| Owned-handle identity | PASS | The live handle is the Active CID's tracked control-session `RecordSocket`. |
| Production lifecycle | PASS | Existing `StopAllocation` drives driver stop, owner teardown, task join, CID removal, then lease release. |
| Cleanup completion | PASS | Each dispatch is awaited and must return `Ok`. |
| Leak visibility | PRESERVED | The Sim destructor's nonzero-handle panic is unchanged and remains the final complement oracle. |
| Feature-disabled contrast | PRESERVED | Its no-lease/no-CID/no-activation behavior and assertions are unchanged. |
| Assertion strength | PASS | No assertion, timeout, typed outcome, or ordering requirement is changed. |
| Surface discipline | PASS | No new test, helper, seam, API, model action, or cleanup mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Source diff inspection | PASS — one awaited existing stop dispatch follows the last assertion in each of the two positive cells. |
| Normal teardown path inspection | PASS — tracked control task cancellation/await closes the record socket and teardown checks absence before release. |
| `rustfmt --edition 2024 --check` | PASS for `guest_activation_acceptance.rs`. |
| Scoped `git diff --check` | PASS for the test, handoff, and review artifact. |
| Test/Metal/mutation execution | NOT RUN by this bounded static review. |

## Iteration-23 verdict

**APPROVED.** Both positive cells now end their intentionally live allocation
through the existing awaited `StopAllocation` lifecycle after proving every
named outcome. This closes the owner-held control-session record normally,
while teardown failures, residual owner state, and residual seam handles
remain observable. No production leak is masked and no behavioral oracle is
weakened.

# APPROVED — SUPPLEMENTAL ITERATION 23

---

# Supplemental review iteration 24 — `fwd_pair` serving-context projection

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-24` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing `FwdPairDriver` projection of serving contexts over retained phase-1 history |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the bounded conformance-driver correction prompted by
the exact initial replay at seed `0xea0ef1a4`. At Trace 1, Step 0, the model
correctly expected the first serving generation to have `ctx = open` and
`oldCtx = none`; the former projection instead treated phase 1's released
probe context as that serving context's predecessor.

## Projection identity and history discipline

The Sim device table intentionally records every context in open order. Boot
therefore leaves one released probe context followed by the serving owner's
first open context. `ForwarderGeneration`, however, is explicitly scoped to
the serving owner: `FIRST` is its first serving context and each successful
owner recovery advances it once. Its raw value is consequently the exact
number of serving contexts opened by this owner, independent of the phase-1
probe row.

The projection now computes the serving suffix by subtracting the owner's
generation count from the complete device history. It accepts that partition
only when the prefix has exactly one row. It then validates the retained phase
row in place: it must be released, have zero live pairs, zero live watches,
and no queued ends. An absent phase row, an additional prefix row, a serving
generation larger than the table, or any phase-1 retained resource is an
explicit conformance error.

This is an identity-based partition, not a value filter. The device table is
not cleared or rewritten, and released rows are not generally discarded. In
particular, every released serving predecessor remains in the suffix and is
still available to `oldCtx` and generation-indexed queued-end projection.

## Generation and recovery audit

The corrected accounting is exact at each reachable owner state:

- after initial boot, owner generation 1 selects the sole serving row, so
  `ctx = open`, `oldCtx = none`, and model generation is 0;
- during a recovery after the old serving context is closed and the new open
  is held, generation remains 1, the sole serving row is released, and no
  predecessor is invented;
- after the replacement opens successfully, owner generation 2 selects both
  serving rows, so the current row is open, its predecessor is released, and
  model generation is 1; and
- on owner death, the current serving row remains in history as released;
  `ctx` and `oldCtx` continue to come from the serving suffix while the owner
  projection is correctly absent.

`project_owner` now receives that same suffix. A pair or watch's
`ForwarderGeneration` remains mapped by `generation - 1`, which makes owner
generation 1 address serving row 0 and generation 2 address serving row 1.
Queued-end counts, current-generation liveness, the pair/watch identity, and
consumed-record precedence therefore stay aligned to the context that
actually issued the ID. Phase 1 can no longer shift those lookups by one, and
an old serving context's queued record remains observable after recovery.

The recovery action, owner observation, Sim state, model variables, action
map, trace recordings, and named-run bodies are unchanged. The concurrent
removal of pending DELIVER markers is crafter activation and is not part of
this DISTILL projection correction.

## Acceptance-review gates

| Gate | Result | Iteration-24 evidence |
|---|---|---|
| Initial model state | PASS | Generation 1 projects only the first serving context: open current, no predecessor, model generation 0. |
| Phase-1 history | PASS | The retained row stays in the arbiter and must be exactly one released, resource-empty context. |
| Extra/missing rows | PASS | Prefix count other than one and generation/table underflow fail explicitly. |
| Serving predecessor | PASS | Released serving contexts remain in the suffix and continue to drive `oldCtx`. |
| Pair/watch generation | PASS | Raw owner generation maps to suffix index `generation - 1` on every serving generation. |
| Queued ends | PASS | Lookup uses the exact serving context that issued the pair/watch ID, including released predecessors. |
| Leak visibility | PASS | Live pair, live watch, or queued end on the phase row is a conformance failure, not filtered history. |
| Recovery semantics | PRESERVED | Close/held-open/reopen and owner-death paths are unchanged. |
| Model integrity | PRESERVED | No Quint source, trace, action, expected state, or rerecord changed. |
| Surface discipline | PASS | Private projection logic only; no production API, seam, state, or mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Source and model inspection | PASS — projection-only change matches the model's owner-scoped generation and predecessor semantics. |
| Generation/index arithmetic | PASS — serving count 1/2 maps to model generation 0/1 and row indices 0/1. |
| `rustfmt --edition 2024 --check` | PASS for `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and review artifact. |
| Conformance/model rerecord | NOT RUN by this bounded static review. |
| Metal and mutation testing | NOT RUN. |

## Iteration-24 verdict

**APPROVED.** The driver now separates the one retained phase-1 probe row
from the exact owner-serving suffix without hiding either history or leaked
resources. Initial, recovery, replacement, and owner-death projections retain
exact context and generation accounting, and no model behavior or production
surface changes.

# APPROVED — SUPPLEMENTAL ITERATION 24

---

# Supplemental review iteration 25 — beacon fixture lock-guard lifetimes

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-25` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Five lock-observation scopes in the existing beacon `ClaimGuard` ownership test |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the lexical lifetime correction for five
`parking_lot::MutexGuard` observations in
`claim_guard_requires_the_originating_live_session_and_preserves_other_entries`.
The surrounding Beacon I/O migration and production implementation are
crafter-owned and are not attributed to this DISTILL correction.

## Guard lifetime and ordering audit

Each affected observation now lives in its own lexical block. The guarded
map, lookup, match, cardinality assertion, and diagnostic text are unchanged.
The block ends before the test drops any beacon/witness handle and before the
next asynchronous `test_beacon_writer().await` call. This gives the compiler
and `await_holding_lock` lint an unambiguous lifetime boundary without a lint
allowance.

The five guarded observations remain in their original order and retain their
exact meanings:

1. a rejected Starting claim leaves the sole `Starting` entry intact;
2. a rejected pre-beacon Live claim leaves the sole `Live { beacon: None }`
   entry intact;
3. a mismatched watcher leaves the sole replacement Live entry pointing to
   the exact replacement `Arc<BeaconWriter>`;
4. a matching accepted session changes the sole entry to
   `EndingInFlight`; and
5. a late watcher leaves an existing sole `EndingInFlight` entry intact.

The explicit ownership operations surrounding those observations are also
unchanged. `starting_guard`, `pre_beacon_guard`, `replacement_guard`, and
`ending_guard` are dropped before their observations; `original_guard` is
dropped after the test first observes its successful transition. The explicit
beacon and witness drops remain after the corresponding map observation.
The final absent-allocation case still performs its same empty-map assertion
after dropping its failed-claim guard and has no later await.

Lexical scope therefore changes only the static lifetime recognized for each
read guard. It does not move an assertion across a `ClaimGuard` transition,
release a beacon earlier than before, add scheduling, or permit concurrent
mutation during an observation.

## Acceptance-review gates

| Gate | Result | Iteration-25 evidence |
|---|---|---|
| Five lint sites | PASS | Exactly five map guards that precede later fixture awaits now end lexically. |
| Assertions | PRESERVED | Cardinality, variant, beacon absence, and `Arc::ptr_eq` checks are byte-for-byte unchanged. |
| Claim ordering | PRESERVED | Failed/successful `try_begin_ending`, guard drops, and map observations remain in the same sequence. |
| Beacon ownership | PRESERVED | Explicit beacon and replacement-witness drops retain their positions after each observation. |
| Async behavior | PRESERVED | No await, task, timeout, yield, channel action, or scheduler stimulus was added or reordered. |
| Lint discipline | PASS | Lexical lifetimes resolve the guard-across-await analysis without `allow`/`expect`. |
| Surface discipline | PASS | No new test, fixture helper, seam, API, state, or production mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Bounded source diff inspection | PASS — only braces replace explicit map-guard drops at the five reviewed sites. |
| `rustfmt --edition 2024 --check` | PASS for `crates/overdrive-worker/src/vm_driver.rs`. |
| Scoped `git diff --check` | PASS for `vm_driver.rs`, the handoff, and this review artifact. |
| Local macOS Clippy | NOT CLAIMED — the Linux-only dependency blocks this target before `overdrive-worker` is checked. |
| Required executable confirmation | PENDING crafter's coordinated Metal Clippy rerun; no Metal job was started by this reviewer. |
| Mutation testing | NOT RUN. |

## Iteration-25 verdict

**APPROVED.** The five lexical blocks end only the temporary map-read guards
before subsequent fixture awaits. All ownership transitions, assertions,
explicit handle drops, and scenario ordering are preserved, with no lint
suppression or test/API/seam change. The crafter's Metal Clippy rerun remains
the executable confirmation.

# APPROVED — SUPPLEMENTAL ITERATION 25

---

# Supplemental review iteration 26 — independent socket-error fact and exact guard destruction

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-26` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | `SimSocket::set_sk_err` fact correction and five beacon fixture guard-drop refinements |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews two independent bounded follow-ups. The first corrects
one existing Sim socket stimulus so a pending socket error no longer invents
a received vsock reset. The second retains iteration 25's lexical scopes and
adds an explicit destruction point immediately after the final assertion for
each of the five map guards.

## `SimSocket::set_sk_err` semantic audit

`set_sk_err` now changes only `sk_err`. This matches the method's existing
two-step contract: it stores a pending socket error without reporting it;
`report_error` later invokes the installed error callback and produces the
typed `SocketError` end. A pending error and a peer SHUTDOWN are independent
kernel facts, so the method no longer writes `received_shutdown = Both` merely
because the socket kind is vsock.

This resolves the exact `VmmExit` step-1 mismatch. The orphan sweep's first
step, `vm_stop_before_callbacks`, stores `ECONNRESET` on each affected host
vsock while leaving the socket established and its received peer-shutdown
flags empty. The model can therefore observe `sk_err` with no `{R,S}` packet.
`finish_sweep` remains the second step and calls `report_error`, which queues
the existing `Aborted { SocketError { ... } }` outcome.

Every repository caller was checked against the narrower fact:

- `vm_stop_before_callbacks` requires the error-before-callback window and no
  fabricated peer packet;
- `dial_vsock_error` represents a vsock I/O error, then immediately reports
  that error; neither operation semantically receives a peer reset;
- the direct swept-half acceptance cells use `sk_err` for HalfEnded refusal,
  local-read error, and later callback classification;
- control-plane destination errors and both host/guest datagram conformance
  stimuli use TCP or UDP halves, for which the former implementation never
  changed `received_shutdown`; and
- attach refusal, watch reporting, holder reads, source-abort classification,
  and send failure all read `sk_err` directly and retain their existing typed
  behavior.

Actual reset stimuli remain separate and unchanged. A vsock `peer_reset`
still calls `peer_shutdown(Both)`, records the peer bits, classifies an abort
when hooked, and sets the closing state when its receive queue is empty.
Guest-reset and destination-reset fixture paths continue to use
`peer_reset`. The correction therefore removes only the false equivalence
between an independent socket error and an incoming reset; it does not weaken
any real reset or peer-abort behavior.

No test body, Quint state, trace, expected outcome, public signature, or seam
was added or changed by this correction.

## Beacon guard refinement

The five beacon ownership observations remain enclosed in the lexical blocks
approved in iteration 25. Each block now also calls `drop(map_guard)`
immediately after that guard's final assertion. This satisfies the Metal
`significant_drop_tightening` requirement at the exact last use while the
closing brace independently proves that no guard can cross a subsequent
`test_beacon_writer().await`.

All five assertions and surrounding ownership operations remain in the same
order. The failed/successful `ClaimGuard` transitions occur before the map
observations; the explicit beacon/witness drops occur afterward; and the
matching accepted-session guard is still dropped only after its
`EndingInFlight` state is observed. No lint allowance, await, timeout, task,
fixture API, or scenario was added.

## Acceptance-review gates

| Gate | Result | Iteration-26 evidence |
|---|---|---|
| Independent error fact | PASS | `set_sk_err` writes only the pending errno. |
| VMM-exit step 1 | PASS | Error is readable with no received SHUTDOWN before callback reporting. |
| VMM-exit step 2 | PRESERVED | `finish_sweep` still reports the error and queues the exact `SocketError` outcome. |
| Attach/watch behavior | PRESERVED | Refusal, local read, reporting, and abort classification read `sk_err` directly. |
| Actual reset behavior | PRESERVED | Vsock `peer_reset` still delegates to `peer_shutdown(Both)` and retains peer-abort semantics. |
| Shared callers | PASS | All call sites either require error-only semantics or operate on TCP/UDP, whose behavior is unchanged. |
| Guard last use | PASS | Each of five map guards is explicitly dropped immediately after its final assertion. |
| Guard across await | PASS | Each explicit drop remains inside a lexical block that ends before the next helper await. |
| Beacon oracles | PRESERVED | Map assertions, claim transitions, and beacon/witness drop order are unchanged. |
| Surface discipline | PASS | No signature, new test, seam, lint allowance, model action, or production mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| `set_sk_err` caller inventory | PASS — all seven repository call sites were inspected. |
| Reset-path inventory | PASS — real vsock resets continue through `peer_reset` → `peer_shutdown(Both)`. |
| Beacon guard inventory | PASS — five lexical guards each have an explicit drop after their final unchanged assertion. |
| `rustfmt --edition 2024 --check` | PASS for `flow_forwarder.rs` and `vm_driver.rs`. |
| Scoped `git diff --check` | PASS for both Rust files, the handoff, and this review artifact. |
| Test/conformance/Metal execution | NOT RUN by this bounded static review. |
| Mutation testing | NOT RUN. |

## Iteration-26 verdict

**APPROVED.** A pending Sim socket error is now projected as its own fact,
while actual peer resets retain their full SHUTDOWN/abort semantics and every
shared caller continues to observe its intended error path. The five beacon
guards now have both lexical and explicit last-use destruction without any
oracle or scheduling change.

# APPROVED — SUPPLEMENTAL ITERATION 26

---

# Supplemental review iteration 27 — `HolderDrop` across the dial-to-stream handoff

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-27` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing `FwdPairDriver::holder_drop` stimulus ownership before and after `Paired` |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the bounded driver correction exposed by the replay
sequence `InitWith("watch") → VmmExit → HolderDrop`. The model, production
dial contract, test bodies, and expected projections are unchanged.

## Ownership handoff audit

The conformance fixture has two mutually exclusive representations of the
same caller-owned dial endpoint:

- before the guest sends `Paired`, `self.dial` owns the spawned task that is
  polling the production `OwnerDial::dial` future; and
- after `Paired`, `attach_h` awaits that task, takes `self.dial`, and stores
  the returned `GuestStream` in `self.stream`.

The former `holder_drop` implementation only took and dropped
`self.stream`. In a pre-handoff trace, `self.stream` was necessarily `None`
and the still-owned `self.dial` task remained live, so the modeled holder-drop
action became a fixture no-op.

The driver now takes and drops the returned `GuestStream` when one exists.
Otherwise it takes the in-flight dial `JoinHandle` and calls `abort`, then
runs the same existing settle. The branches are ordered around the real
handoff and cannot both act: `attach_h` consumes the join handle before it
installs the stream.

Aborting the fixture's join handle is the exact way to model dropping the
pre-handoff dial future. Merely dropping a Tokio `JoinHandle` would detach the
task and preserve the caller operation, contrary to `HolderDrop`. Task abort
drops the polled production `dial` future; its existing `DialCancelGuard`
cancels the token owned by `run_dial`; and `run_dial` performs its already
authored error cleanup: it marks cancellation after a written request, sends
the existing guest `Abort`, aborts the forwarder watch with `Detached`, closes
`V_h`, and removes the dial entry. The following settle lets that existing
cleanup reach the same projection boundary as every other driver stimulus.

This path is already independently exercised by
`a_dropped_dial_future_ends_the_dial_like_the_pairing_deadline`, which uses the
same spawned-dial `abort` plus settle sequence and requires the closed dial,
guest Abort, empty dial table, released quota place, zero live watches, and
inert late reply.

## Model and ordering audit

`HolderDrop` models the external holder abandoning the watch-backed endpoint
and requires the live watch to end `Detached` and `V_h` to close. That owner
exists before the API future resolves as well as after it returns a
`GuestStream`; the handoff changes only which fixture field owns the operation.
The new pre-handoff branch therefore supplies the already-modeled action
rather than inventing a cancellation state.

In the reproduced sequence, `VmmExit` step 1 has made `sk_err` readable but
has not run `sk_error_report` or completed the pending dial. `self.dial`
remains the actual owner and `self.stream` remains absent. Aborting that task
selects the production cancellation contract and its deterministic cleanup;
it does not fabricate a guest reply, pair, stream, end callback, or production
state unreachable from caller cancellation.

Post-handoff behavior is unchanged: `GuestStream::drop` still aborts the
watch and closes its owned `V_h`. If neither field exists, the helper retains
its prior no-effect fallback; valid model traces enable `HolderDrop` only
while the host endpoint is alive, so no new invalid action becomes playable.

## Acceptance-review gates

| Gate | Result | Iteration-27 evidence |
|---|---|---|
| Pre-handoff owner | PASS | `self.dial` is the spawned task polling the caller's production dial future. |
| Post-handoff owner | PASS | `attach_h` consumes `self.dial` before assigning the returned `GuestStream` to `self.stream`. |
| Holder-drop effect | PASS | Stream drop applies after handoff; task abort drops the dial future before handoff. |
| Production cancellation | PASS | Existing `DialCancelGuard` and `run_dial` perform watch abort, Detached record, guest Abort, socket close, and dial removal. |
| Reproduced ordering | PASS | VMM-exit step 1 leaves the dial pending, so the new branch acts on the real remaining owner. |
| Independent acceptance evidence | PASS | Existing dropped-dial-future cell already pins the same abort/settle cleanup contract. |
| Model integrity | PRESERVED | `HolderDrop`, trace recordings, expected projections, and Quint sources are unchanged. |
| Scenario/oracle integrity | PRESERVED | No test body, assertion, named run, timeout, or expected outcome changed. |
| Surface discipline | PASS | Private driver stimulus only; no production/API/seam/state/mechanism change. |

## Mechanical verification

| Check | Result |
|---|---|
| Ownership-state inspection | PASS — `self.dial` and `self.stream` are mutually exclusive across `attach_h`. |
| Cancellation-path inspection | PASS — abort drops the outer dial future and activates existing token-driven `run_dial` cleanup. |
| `rustfmt --edition 2024 --check` | PASS for `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and this review artifact. |
| Conformance/model rerecord | NOT RUN by this bounded static review. |
| Metal and mutation testing | NOT RUN. |

## Iteration-27 verdict

**APPROVED.** `HolderDrop` now acts on the fixture field that actually owns
the dial endpoint at either side of the `Paired` handoff. The pre-handoff task
abort is the established caller-cancellation path, exercises the existing
production cleanup contract, and neither manufactures a state nor changes the
model, production surface, scenario, or oracle.

# APPROVED — SUPPLEMENTAL ITERATION 27

---

# Supplemental review iteration 28 — completed pre-handoff dial cancellation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-28` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Completion boundary for the existing pre-handoff `HolderDrop` task abort |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

The current-tree Metal replay invalidated iteration 27's assumption that
`JoinHandle::abort` followed immediately by the ordinary settle was sufficient
to complete the modeled `HolderDrop` action. The same seed still projected a
live watch: the runtime had not yet polled and destroyed the aborted outer
dial task, so its `DialCancelGuard` had not cancelled `run_dial`. This
iteration records that failed attempt and reviews the corrected completion
boundary.

## Cancellation completion audit

The pre-handoff branch now performs these exact steps:

```rust
dial.abort();
drop(self.rt.block_on(dial));
self.settle();
```

`abort` requests cancellation of the fixture-owned outer task that is polling
the production `OwnerDial::dial` future. Awaiting the same `JoinHandle` is the
task API's completion boundary: it guarantees that the outer future has been
dropped before the driver projects the action. Dropping that future drops its
armed `DialCancelGuard`, which cancels the token observed by the separately
spawned `run_dial` task. The unchanged settle then allows `run_dial`'s existing
token-driven cleanup to finish: guest Abort after the written request, watch
abort with `Detached`, `V_h` close, dial-table removal, and quota release.

There is no circular wait. The joined handle belongs to the outer caller
future; it waits on `result_rx`, while its abort destroys that wait and the
guard. The inner `run_dial` task is not joined at this point and therefore
cannot be a prerequisite for completion of the aborted outer handle. It is
driven afterward by the existing settle boundary once the cancellation token
has definitely been set.

The result drop also closes the race in which the outer task completed just
before cancellation won. In that case the join returns its nested production
result. If it contains a `GuestStream`, dropping the join result immediately
drops that stream and invokes the same post-handoff holder-drop behavior. If
it contains an error, the production dial cleanup has already completed.
Thus every interleaving releases the actual caller-owned object exactly once;
none can detach a completed stream or leave the pre-handoff future running.

## Model and fixture discipline

The added await is completion of the exact cancellation action already chosen
by `HolderDrop`. It is not a timed sleep, yield count, observation poll, or
new Sim effect. The model treats holder drop and its watch/socket consequences
as one step, so the driver must complete cancellation before returning the
step for projection. Waiting for the task that owns the future is the minimal
ownership boundary that makes that atomic action honest.

Post-handoff behavior remains the existing direct `GuestStream` drop. The
pre-handoff branch still uses only the already-owned join handle and the
production `DialCancelGuard`/`run_dial` cleanup. No guest reply, callback,
record, stream, or pair is manufactured by the fixture. Quint source,
recorded traces, named-run bodies, production code, APIs, and seams remain
unchanged.

## Iteration-27 remediation disposition

| Prior disposition | Reproduced evidence | Iteration-28 disposition |
|---|---|---|
| Abort plus settle was considered sufficient to drop the outer dial future. | The same Metal seed still observed the live watch because the aborted task had not yet been polled and destroyed. | **SUPERSEDED** — abort and await the same handle, discard its result, then settle the already-triggered inner cleanup. |

## Acceptance-review gates

| Gate | Result | Iteration-28 evidence |
|---|---|---|
| Failed attempt retained | PASS | The handoff records the abort-plus-settle replay failure and why cancellation had not started. |
| Outer future destruction | PASS | Awaiting the aborted handle completes destruction of the future and its `DialCancelGuard`. |
| Inner cleanup ordering | PASS | Token cancellation is guaranteed before the unchanged settle drives `run_dial` cleanup. |
| Concurrent completion | PASS | A successfully returned `GuestStream` is dropped through the join result; an error already owns no stream. |
| Deadlock freedom | PASS | The awaited outer handle does not await the inner task after abort; inner cleanup runs during settle. |
| Atomic model action | PASS | Cancellation completion precedes projection, matching `HolderDrop`'s one-step effects. |
| No fabricated barrier | PASS | Join completion is the owned task's lifecycle boundary, with no clock, timeout, yield count, or state polling. |
| Model/oracle integrity | PRESERVED | No Quint source, trace, body, assertion, or expected state changed. |
| Surface discipline | PASS | Private driver control only; no production/API/seam/state/mechanism change. |

## Mechanical verification

| Check | Result |
|---|---|
| Cancellation ownership inspection | PASS — the awaited handle owns the outer dial future whose guard signals the inner task. |
| Concurrent-result inspection | PASS — dropping `Result<Result<GuestStream, _>, JoinError>` releases any returned stream. |
| `rustfmt --edition 2024 --check` | PASS for `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and this review artifact. |
| Metal replay | NOT RUN by this reviewer; the failed abort-only replay is recorded in the handoff and the crafter owns the corrected rerun. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-28 verdict

**APPROVED.** Awaiting the aborted fixture task is the exact completion
boundary needed to destroy the pre-handoff production dial future and arm its
existing cancellation cleanup before projection. The subsequent settle drives
that cleanup without a cycle or synthetic timing barrier, and a concurrent
successful result is safely reduced to the same `GuestStream` drop. Iteration
27's abort-only disposition is superseded.

# APPROVED — SUPPLEMENTAL ITERATION 28

---

# Supplemental review iteration 29 — holder-close socket fact

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-29` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing Sim dial-holder close paths and their shared `SimSocket` effect |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration corrects the causal reading of the post-iteration-28 replay.
The first non-verbose report repeated the whole-state mismatch, but the
removed one-run diagnostic proved that abort plus join completion had worked:
the dial branch completed, the outer join was cancelled, the forwarder held
zero pairs and zero watches, and exactly one end record was queued. The next
verbose replay showed the owner projection already exact: ended,
`Detached`, queued, and `nrec = 1`. The sole remaining mismatch was the
holder-owned socket fact: the model required sent `{R,S}` and `sshut = true`,
while the shared `SimSocket` recorded neither.

## Missing socket effect

Both accepted holder-close paths already recorded the logical
`VsockShutdownOp::Close` but omitted the corresponding effect on the same
shared socket that QPROJ-8 reads:

- before handoff, `run_dial` cleanup retains a dialed `RecordSocket`; its
  existing `GuestFlowIo::close` reaches `end_handle`; and
- after handoff, `into_guest_stream` consumes that record and moves its
  `SimSocket` into `SimGuestStreamIo`; dropping the returned `GuestStream`
  reaches `SimGuestStreamIo::drop`.

`end_handle` now calls `record.vsock.close_from_owner(abortive)` for a dialed
record whose flow is known. `SimGuestStreamIo::drop` now clones its retained
socket identity and calls `close_from_owner(false)`. Each call sits beside
the path's unchanged logical Close append and dial-closed bookkeeping.

For a vsock stream, the existing `close_from_owner` operation sets the local
owner-closed fact and the socket's sent SHUTDOWN to `Both`. It does not change
received peer-shutdown flags, `sk_err`, recorded end class, peer-reset facts,
or vsock state. The preceding orphan-sweep correction therefore remains
intact: after `VmmExit → HolderDrop`, the socket retains its independent
`sk_err`, retains no received peer bits, and now additionally records the
owner's outgoing close.

## Ownership, ordering, and single-close audit

The two new call sites are mutually exclusive for one dial endpoint.
`into_guest_stream` removes the `RecordSocket` handle from the seam and moves
its `SimSocket` into the stream pipe. Therefore a pre-handoff error/cancel
ends the record through `end_handle`, while a successfully handed-off stream
can only close through its own drop. The concurrent-completion case from
iteration 28 also follows the latter path because dropping the joined result
drops the returned stream.

The established operation order is unchanged. Pre-handoff `run_dial`
cleanup aborts the watch first, then closes the retained socket. Post-handoff
stream drop aborts a still-live watch first, then closes the retained socket.
The existing `Detached` record is consequently produced before the outgoing
close, matching the authored cancellation and holder-drop contracts. A watch
already ended by the owner is not aborted again; the stream drop still closes
its socket exactly as before, now with the socket fact recorded.

The physical write is idempotent if another independent owner-lifecycle event
has already set sent SHUTDOWN to `Both`, but it does not add a second logical
Close entry. Accepted-record, peer-socket, pair-socket, context-release, and
process-exit paths are unchanged. The shared effect therefore fills the
missing fact only where the dial holder itself closes `V_h`; it does not
broaden close behavior to adjacent ownership paths.

## No weakening or masking

This change follows the completed owner cleanup rather than replacing it.
The forwarder inventory and queued-end facts remain exact, and the driver
still fails if watch cancellation, `Detached` classification, dial removal,
or record cardinality is wrong. The new socket call supplies the separately
projected sent-side fact; it cannot manufacture owner cleanup, a received
reset, or the pending socket error.

The temporary diagnostic is absent from the source. No additional wait,
yield, timeout, model transition, expected state, test body, public API, seam,
or production mechanism was added.

## Acceptance-review gates

| Gate | Result | Iteration-29 evidence |
|---|---|---|
| Causal isolation | PASS | Completed cancellation already showed zero pairs/watches and one queued end; only sent socket state differed. |
| Owner projection | PRESERVED | Ended/Detached/queued/`nrec = 1` remains the independent owner oracle. |
| Pre-handoff ownership | PASS | The dialed record retains the exact `SimSocket` until `end_handle` consumes it. |
| Post-handoff ownership | PASS | `into_guest_stream` moves that socket into the stream pipe consumed by stream drop. |
| Single close path | PASS | Record consumption and stream handoff are exclusive; one holder path owns the socket at a time. |
| Effect ordering | PASS | Existing watch abort precedes existing logical and newly recorded socket close on both paths. |
| Sent-side effect | PASS | Vsock owner close records sent `Both`, making `sshut` true. |
| Independent receive/error facts | PRESERVED | No received SHUTDOWN, `sk_err`, reset, class, or state mutation is added. |
| Shared callers | PASS | All dial-record closes and returned-stream drops already promised/logged Close; adjacent handle types and owner-death paths are unchanged. |
| Diagnostics and timing | PASS | Throwaway diagnostics are removed; no wait, timeout, yield count, or polling was added. |
| Surface discipline | PASS | Existing private adapter operations only; no API, seam, test, model, or production change. |

## Mechanical verification

| Check | Result |
|---|---|
| Socket ownership inspection | PASS — the record owns the socket before `into_guest_stream`; the stream pipe owns it afterward. |
| Close-log inspection | PASS — each path retains its single existing `VsockShutdownOp::Close` append. |
| Socket mutation inspection | PASS — vsock `close_from_owner` changes owner-closed and sent SHUTDOWN only. |
| `rustfmt --edition 2024 --check` | PASS for `flow_io/sim.rs` and `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for both reviewed Rust files, the handoff, and this review artifact. |
| Metal/conformance execution | NOT RUN by this reviewer; the handoff records the focused diagnostic and verbose replay evidence. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-29 verdict

**APPROVED.** The completed cancellation path was already correct; the
remaining divergence was the Sim adapter's missing outgoing close on the
shared holder socket. Both mutually exclusive ownership paths now apply that
existing socket operation while preserving the exact Detached record,
single logical close, independent received/error/reset facts, model, and
production surface.

# APPROVED — SUPPLEMENTAL ITERATION 29

---

# Supplemental review iteration 30 — CID `Launch` provision-release completion

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-30` |
| Date | 2026-10-24 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing CID-lease conformance driver's first `Launch` settle predicate |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the one-predicate correction exposed by the activated
CID-lease gate. Removed read-only diagnostics showed that the ordinary launch
had no Sim VMM device holder, no host PID, no allocation row, and an unfinished
tick. The sole observed fact was the original `Held::Provision` entry that the
driver had just granted but the dispatch had not yet consumed.

## Stimulus versus effect completion

`HeldProvisioner::release(Held::Provision, alloc)` records a grant and wakes
the held call. It intentionally does not remove the call from `waiting`.
Removal happens inside `HeldProvisioner::hold` only when the woken production
task is polled, consumes the matching grant, removes the waiting entry, and
returns from `provision`.

The former first-Launch predicate accepted any `held_for(a)`. Because
`settle` evaluates its condition before advancing the serve runtime, it saw
the just-granted Provision entry immediately and returned without polling the
dispatch. The subsequent absence of a matching VMM was then mislabeled as the
model's device-holder clash even though no launch attempt had occurred.

The predicate now accepts only one of the real post-stimulus boundaries:

- `launch_vmm(alloc, cid)` observes a VMM of the exact allocation whose PID is
  recorded by the host and is the actual holder of that CID's device;
- the exact workload tick has finished; or
- the workload's dispatch has progressed to the existing
  `Held::Teardown` boundary.

An existing `Held::Provision` entry no longer completes the wait. The loop
therefore advances the serve runtime until the production task consumes the
grant and performs the launch path. This distinguishes granting the fixture
effect from completion of the production effect without adding a sleep
duration, new observation, or synthetic state.

## Full `Launch` action audit

The remainder of `Launch` is unchanged and retains the exact owner path:

1. resolve the model workload's recorded placement allocation;
2. read that allocation's lease CID and launch token from the production
   `GuestAddressPool` observation;
3. release that exact allocation's held Provision call;
4. wait for the exact VMM/device fact, tick completion, or post-launch
   teardown boundary;
5. on a successful VMM creation, queue READY to the bound beacon port with
   that CID and launch token;
6. release the accepted beacon read;
7. await dispatch completion or the existing predecessor/successor teardown
   boundary; and
8. reap a completed tick through its existing checked outcome.

The correction changes only step 4's rejection of a stale pre-launch pacing
entry. Lease ownership, allocation relabelling, device-holder identity, host
PID matching, READY stimulus, token/port identity, row projection, and
post-launch cleanup remain unchanged.

## Hazard and model audit

The actual clash branch remains intact. `launch_vmm` requires both the
allocation's host PID and that PID's ownership of the leased CID device. A
VMM whose device open is refused cannot satisfy it; once its dispatch reaches
the existing finished/teardown boundary, the same HazardOnly clash result is
returned. The fixture has no alternate scripted start failure in this action,
so the corrected wait does not hide a distinct production outcome.

Conversely, a released Provision entry alone cannot prove either success or
clash and is no longer accepted as either. A task that makes no progress now
reaches the existing bounded `Unsettled` failure rather than being converted
to a false hazard. A successful ordinary launch must expose the real
allocation/PID/CID holder before READY is supplied.

The model's `Launch` action, free-device success, held-device clash, row and
lease projections, named runs, test bodies, and expected states are unchanged.
The concurrent marker activation is crafter-owned. Removed diagnostics are
absent from the source.

## Acceptance-review gates

| Gate | Result | Iteration-30 evidence |
|---|---|---|
| Exact release target | PRESERVED | The driver still grants `Held::Provision` for the placement allocation only. |
| Grant/return distinction | PASS | A granted Provision remains waiting until the polled production task consumes it. |
| Successful launch fact | PASS | Completion requires exact allocation PID plus exact leased-CID device ownership. |
| Finished dispatch | PRESERVED | The exact workload tick remains a completion boundary. |
| Post-launch cleanup | PRESERVED | Only existing `Held::Teardown` remains an alternate held boundary. |
| Real clash | PRESERVED | Device-open refusal still yields no matching holder and reaches the unchanged hazard branch. |
| Stalled effect | PASS | Lack of VMM, finish, or teardown reaches bounded `Unsettled`, not false clash. |
| READY ordering | PRESERVED | READY is still queued only after the real VMM/device holder is observed. |
| Model/oracle integrity | PRESERVED | No Quint source, trace, test body, projection, or expected outcome changed. |
| Surface discipline | PASS | Predicate-only private driver change; no production/API/seam/state/mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Held-provisioner inspection | PASS — grant and waiting-entry removal are separate, scheduler-ordered operations. |
| Launch-path inspection | PASS — all owner, lease, VMM, beacon, row, and teardown logic after the wait is unchanged. |
| Hazard-path inspection | PASS — exact PID/device-holder observation and clash classification remain unchanged. |
| `rustfmt --edition 2024 --check` | PASS for `quint_cid_lease.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and this review artifact. |
| CID conformance/Metal execution | NOT RUN by this bounded static reviewer; the handoff records the diagnostic reproduction. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-30 verdict

**APPROVED.** The Launch wait now distinguishes release of the fixture's
Provision gate from the production dispatch's completion of that effect. It
can proceed only to a real allocation/PID/CID holder, finished tick, or
post-launch teardown, preserving the actual clash branch and every model and
READY oracle without masking a stalled dispatch.

# APPROVED — SUPPLEMENTAL ITERATION 30

---

# Supplemental review iteration 31 — record-socket owner-close projection

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-31` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing Sim `RecordSocket` owner-close effect for accepted and dialed records |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

A new verbose `fwd_pair` replay isolated two separate facts in
`InitWith("stream") → VmmExit → TcpRst("h") → CtxRelease("h")`. The
guest-opened flow was still `Connecting`, its `ConnectPeer` effect was held,
and its accepted `V_h` had never attached to the forwarder context. The
production Forwarder-repair branch did not cancel and await the existing flow
task, so the unattached owner-held record survived the repair. That is a
DELIVER implementation gap owned by the original crafter. DISTILL does not
alter the driver stimulus, model, or oracle to compensate for it.

The replay also proved that iteration 29's Sim alignment was too narrowly
placed. An accepted pre-attach `RecordSocket` and a dialed pre-handoff
`RecordSocket` are the same ownership class at the `GuestFlowIo` boundary:
both own a live host `V_h`, and consuming either through the existing
owner-close/abort port must project the close on its shared `SimSocket`.

## Record ownership and close effect

`RecordState` always retains the `SimSocket` created for its accepted or
dialed vsock connection. `end_owned` consumes the opaque `RecordSocket`
handle exactly once, removes its `HandleState::Record` from the seam, and
passes that state to `end_handle`. The socket is therefore still owned by the
record at this boundary regardless of `RecordOrigin` or whether a flow ID has
already been decoded/written.

`record.vsock.close_from_owner(abortive)` now runs unconditionally in the
`HandleState::Record` arm. Origin-specific bookkeeping remains after it:

- accepted records with a connection key retain the existing
  `closed_by_owner` evidence;
- dialed records with a known flow retain their existing dial-closed flag and
  single `VsockShutdownOp::Close` log; and
- records without an attributed flow gain no invented flow identity or
  logical message.

Successful ownership transfers bypass `end_handle`. `attach` consumes the
accepted record and moves the same socket into the pair's vsock half;
`into_guest_stream` consumes the dialed record and moves it into the stream
pipe. Consequently, the new record-arm call cannot close an attached pair or
a handed-off stream. Those later owners retain their existing lifecycle and,
for a returned stream, the separately approved stream-drop close.

## Socket-fact and single-close audit

For a vsock, the existing `close_from_owner` operation records only local
owner closure and sent SHUTDOWN `Both`. It does not write received peer bits,
`sk_err`, recorded end class, peer-reset state, or vsock connection state.
Close and abort of a record therefore have the same transport-visible `0x3`
effect, while the `abortive` distinction remains available to the existing
TCP peer-handle branch where reset versus EOF matters.

There is one record close per owned handle. `end_owned` first removes the
handle, so a second close cannot reach the same record. A successful attach or
stream handoff removes the record without calling `end_handle`, transferring
the socket to exactly one later owner. Process exit retains its separate
context/handle teardown semantics. No additional logical Close is appended by
the new socket operation.

All existing record-close callers already claim this ownership effect:
control-session close, malformed/stale/refused accepted flows, cancellation
of a pre-attach flow task, attach-refusal cleanup, pending-dial failure or
cancellation, and the direct seam contract cells. The widened placement makes
their shared socket fact match the close they already execute; it does not add
a new caller or alternate closure path.

## Production-gap separation

The Sim change cannot make the reproduced recovery trace pass by itself.
`close_from_owner` runs only when production actually invokes
`GuestFlowIo::close` or `abort` on the record. The current
`converge_shared` Forwarder repair does neither for the held Connecting task,
so the surviving `V_h` and flow entry remain visible to the unchanged driver
oracle.

The accepted production correction remains explicit: recovery step 1 must
cancel and await the existing flow tasks before closing/reopening the
forwarder. The ordinary `tcp_connect` cancellation cleanup then closes its
still-owned record, the Sim projects outgoing `{R,S}`, and the owner
projection can reach ended/`byRel`. DISTILL adds no synthetic abort, forced
close, recovery hook, polling, or test-side state mutation.

## Iteration-29 disposition

| Prior scope | New reproduced evidence | Iteration-31 disposition |
|---|---|---|
| The `end_handle` socket effect was restricted to a dialed record with a known flow. | Recovery owns an accepted, attributed, pre-attach `RecordSocket` whose ordinary cancellation close must expose the same sent-side socket fact. | **SUPERSEDED IN PLACEMENT** — every consumed record applies its owner-close effect; dial-only logs and bookkeeping remain origin-specific. |

Iteration 29's stream-drop path, independent receive/error/reset facts, and
single logical dial Close remain approved and unchanged.

## Acceptance-review gates

| Gate | Result | Iteration-31 evidence |
|---|---|---|
| Exact record owner | PASS | `RecordState` owns the shared accepted/dialed `SimSocket` until `end_owned` consumes the handle. |
| All record origins | PASS | Accepted control/flow and dialed records receive the same real owner-close socket effect. |
| Pre-attach projection | PASS | A cancelled Connecting flow's retained `V_h` now records outgoing `Both` when production closes it. |
| Ownership transfer | PRESERVED | Successful `attach` and `into_guest_stream` consume the record without `end_handle` and do not close early. |
| Single close | PASS | Handle removal precedes `end_handle`; later owners are mutually exclusive with the record owner. |
| Origin bookkeeping | PRESERVED | Accepted `closed_by_owner` and dial flow/Close logs retain their exact conditions and cardinality. |
| Independent socket facts | PRESERVED | No received SHUTDOWN, error, reset, recorded class, or connection-state mutation is added. |
| Alternate closure paths | PRESERVED | Pair, peer, stream, context-release, and process-exit ownership paths are unchanged. |
| Production defect visibility | PASS | The Sim effect occurs only after a real close/abort; missing repair cancellation still fails the existing trace. |
| Model/oracle integrity | PRESERVED | No Quint source, trace, action, projection expectation, test body, or timeout changed. |
| Diagnostics | PASS | Temporary read-only diagnostics are absent from the source. |
| Surface discipline | PASS | Existing private adapter operation only; no API, seam, state, or recovery mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Record lifecycle inspection | PASS — close/abort consumes the record; attach/stream handoff transfers it without close. |
| Caller inventory | PASS — every existing Record close already represents owner disposal of its vsock descriptor. |
| Socket mutation inspection | PASS — vsock owner close sets owner-closed and sent `Both` only. |
| `rustfmt --edition 2024 --check` | PASS for `flow_io/sim.rs`. |
| Scoped `git diff --check` | PASS for the Sim adapter, handoff, and this review artifact. |
| Metal/conformance execution | NOT RUN by this bounded static reviewer; the handoff records the focused replay evidence. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-31 verdict

**APPROVED.** Every consumed record socket now projects the owner close on
the exact shared `V_h` it owns, while successful handoffs, single-close
discipline, origin-specific logs, and independent peer/error/reset facts stay
intact. The change does not conceal the reproduced production recovery gap;
the original crafter still must cancel and await the held flow task before its
ordinary close can occur.

# APPROVED — SUPPLEMENTAL ITERATION 31

---

# Supplemental review iteration 32 — CID composition shared mTLS boot precondition

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-32` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing CID-lease composition's node-shared mTLS owner startup |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

After the corrected Launch settle reached the real VM and READY path, seed
`0xbfb027be` stopped at the existing teardown boundary. Removed diagnostics
showed an already-committed `Running { reason: Started }` row, no live VMM or
device holder, and teardown waiting for that same allocation. This was the
action shim's real post-start fail-closed path: its existing
`MtlsInterceptWorker` had never had its node-shared owner started, so
`mtls_lifecycle.start_alloc` correctly failed after driver start, stopped the
VMM, and initiated cleanup.

## Composition identity and boot ordering

The CID composition now constructs the existing test mTLS worker exactly
once, starts its existing node-shared owner, checks that startup returns
`Ok`, and moves that same `Arc<MtlsInterceptWorker>` into `AppState::new`:

```rust
let mtls_worker = crate::shared_network_test_ports::mtls_worker();
mtls_worker.start_shared_owner().await.expect(...);
let mut state = AppState::new(..., mtls_worker, ...);
```

There is no second helper call, replacement worker, wrapper, or copied state.
The allocation lifecycle invoked by the action shim therefore addresses the
same worker instance whose shared listener/intercept owner was started.

The startup occurs on the composition's retained current-thread runtime,
beside the existing `TestSharedOwner::converge_shared` boot prerequisite and
before any `AppState` tick is spawned. This matches the production boot
ordering relevant to allocation dispatch: the guest-network owner is
converged and the node-shared mTLS owner is healthy before an allocation can
reach `start_alloc`. Moving the already-created Arc into `AppState` preserves
the worker tasks on the same runtime for the serve process's lifetime.

## Real lifecycle and cleanup audit

This fixture correction does not replace the allocation lifecycle with a
successful stub. `AppState` retains the real `MtlsInterceptWorker`, so a
successful Launch still follows the complete accepted path:

1. the production VM driver starts the VMM and observes READY;
2. the action shim commits the Running observation;
3. `mtls_lifecycle.start_alloc` installs the allocation under the already-live
   shared owner;
4. guest-network activation and EXEC release continue through their existing
   gates; and
5. stop, restart, failure, and terminal cleanup call the same worker's real
   `stop_alloc` path.

The setup satisfies only the production boot precondition that was missing.
Allocation-specific install failures still return through the unchanged
typed fail-closed branch, which stops the VMM and performs mTLS, guest-network,
and lease cleanup. Likewise, a real CID device-holder clash still occurs in
`VmDriver::start` before READY and is observed by the unchanged
allocation/PID/device-holder oracle. Neither outcome is overridden or
translated by the fixture.

Starting the worker also increases fidelity of later model actions: a
successful allocation now owns real per-allocation interception state, and
subsequent Stop/Release/restart paths must clean that state through the same
worker. The change therefore exposes lifecycle cleanup defects rather than
pre-clearing or bypassing them.

## No production-bug masking

The former failure was a valid response to an invalid fixture composition,
not evidence that production should tolerate an unstarted shared owner.
Production boot already refuses startup if `start_shared_owner` fails and
never dispatches allocations in that state. The corrected fixture now enforces
the same rule with `expect`; a shared-owner startup failure cannot be ignored
or converted into a passing Launch.

Dedicated fail-closed mTLS tests retain their explicitly unstarted or failing
workers and their existing oracles. This edit is confined to the positive
CID conformance composition, whose Launch model requires the booted node to
accept an ordinary allocation. No production branch, error mapping, model
state, test body, or expected outcome changed. All temporary CID diagnostics
are absent from the source.

## Acceptance-review gates

| Gate | Result | Iteration-32 evidence |
|---|---|---|
| Single worker identity | PASS | One helper result is started and moved directly into `AppState`; no second worker is constructed. |
| Boot ordering | PASS | Guest-owner convergence and shared mTLS startup complete before the first allocation tick. |
| Startup failure visibility | PASS | `start_shared_owner` is awaited and must return `Ok`; failure stops fixture construction. |
| Real allocation install | PRESERVED | Action shim still calls the real worker's `start_alloc` after Running. |
| Real cleanup | PRESERVED | Stop/failure/restart paths retain the same worker and existing `stop_alloc` lifecycle. |
| Device clash | PRESERVED | VM driver and exact PID/CID device-holder observation are unchanged. |
| Fail-closed behavior | PRESERVED | Any genuine per-allocation mTLS install failure still stops the VMM and enters ordinary cleanup. |
| Production parity | PASS | The fixture now establishes the node-shared owner prerequisite production boot requires. |
| Model/oracle integrity | PRESERVED | No Quint source, action, trace, body, projection, or expected state changed. |
| Diagnostics | PASS | Temporary read-only diagnostics are removed. |
| Surface discipline | PASS | Existing private fixture composition only; no API, seam, type, state, or production mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Construction inspection | PASS — the same local `mtls_worker` binding is started and passed to `AppState::new`. |
| Action-shim path inspection | PASS — post-start `start_alloc` and fail-closed cleanup are unchanged. |
| `rustfmt --edition 2024 --check` | PASS for `quint_cid_lease.rs`. |
| Scoped `git diff --check` | PASS for the CID driver, handoff, and this review artifact. |
| CID conformance/Metal execution | NOT RUN by this bounded static reviewer; the handoff records the seed and exact diagnostic evidence. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-32 verdict

**APPROVED.** The CID composition now starts the exact mTLS worker instance
that `AppState` later drives, restoring the production boot prerequisite
without bypassing the real allocation install, device-clash, or fail-closed
cleanup paths. The change adds no outcome override, public surface, test seam,
model transition, or oracle weakening.

# APPROVED — SUPPLEMENTAL ITERATION 32

---

# Supplemental review iteration 33 — Forwarder recovery protocol composition

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-33` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing `fwd_pair` `CtxRelease(h)` / `Reopen(h)` composition of the production recovery protocol |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration withdraws the production-gap premise recorded in iteration 31.
The raw `HostSharedGuestNetworkOwner::converge_shared` Forwarder branch is not
the production recovery entry point and must not duplicate flow cleanup. The
runtime shared-network supervisor first acquires a retained recovery hold
through `owner.quiesce()`. That existing owner operation latches admission,
clears flow ownership, cancels and awaits every flow task, aborts forwarder
work, drains the existing cleanup paths, and closes unattached record sockets.
Only after that completed boundary does the supervisor call
`converge_shared`.

The prior proposal to add cancellation/await logic inside
`converge_shared` is therefore **WITHDRAWN**. No such production remediation
is authorized or required by this review.

## Driver protocol alignment

The conformance driver previously scripted the audit failure and called
`audit_shared` → `converge_shared` directly. That bypassed the accepted
ADR-0124/ADR-0169 composition and produced the false surviving-Connecting-flow
premise. The corrected driver now uses the existing owner primitives in the
same effect order as the production supervisor.

`CtxRelease(h)` on the recovery path now:

1. scripts the existing Forwarder inventory I/O failure;
2. calls `audit_shared` and requires the finding to damage/mark the Forwarder;
3. refuses a duplicate driver recovery hold;
4. calls the production owner's `quiesce` and retains the returned move-only
   `QuiescenceHold` in private driver state;
5. holds the existing `OpenForwarder` effect;
6. spawns the existing `converge_shared` repair; and
7. settles only after quiescence cleanup has completed and repair is parked at
   the held replacement open.

Because `quiesce` is awaited before repair starts, a pre-attach Connecting
task cannot survive this action boundary. Its existing cancellation cleanup
closes the accepted record socket; iteration 31's Sim projection then exposes
that real owner close on the shared `V_h`. Attached pairs, watches, pending
dials, and handed-off streams use the same existing quiescence semantics.

`Reopen(h)` now:

1. releases the held replacement open;
2. settles and joins the exact retained repair task;
3. requires the repaired production owner to pass `audit_shared`;
4. takes the exact `QuiescenceHold` acquired by the matching CtxRelease;
5. restores that hold through the owner; and
6. settles the resulting admission/task state before projection.

The hold is never reconstructed from a count or substituted token. A failed
repair or post-repair audit retains the hold fail-closed, matching the
supervisor's retry episode. A successful repair consumes and restores it
exactly once. A foreign-hold restoration is surfaced as a failed driver step,
as production would escalate it, rather than being accepted.

## Model action boundaries

The model deliberately presents recovery as two observable actions.
`CtxRelease(h)` represents the completed pre-repair cleanup plus release of
the old context, with the replacement open still pending. The corrected
driver returns from that action only after `quiesce` has completed and the
repair task is parked at `OpenForwarder`, so the projection cannot observe an
intermediate live flow or an already-open replacement.

`Reopen(h)` represents the replacement context becoming healthy. The driver
does not return until open, inventory, repair completion, clean audit, and
restoration of the recovery's own hold have completed. These are lifecycle
completion boundaries, with no sleep, yield count, state injection, or
synthetic abort.

The existing D-5 split remains unchanged. If the current context inventory
contains a live pair or watch, `CtxRelease(h)` still models final host-owner
death and never enters in-process recovery. With no live context object, the
recovery path may still own unattached or already-ended flow-table resources;
the production quiescence boundary resolves those resources before context
replacement, exactly as the model's `recoveryClose` outcome requires.

## Production-composition equivalence

The production supervisor also records EXEC recovery state, applies clock
bounds, retries failed attempts, and audits every shared component. Those are
supervisor coordination outside `fwd_pair`'s one-owner state. For the modeled
successful Forwarder attempt, the driver uses the same production owner
instance and the exact owner-side sequence: observed audit failure,
`quiesce`, retained hold, `converge_shared`, clean audit, restore that hold.
No alternate owner, no-op port, copied ledger, or private cleanup routine is
introduced.

Requiring the post-repair owner audit does not filter a valid model state: the
model's `Reopen` means the replacement context is healthy. Any remaining
Forwarder inconsistency is a conformance failure, not another valid Reopen
outcome. The duplicate-hold guard likewise rejects only a fixture protocol
violation; the model cannot issue another CtxRelease while its context is
already released awaiting Reopen.

## Iteration-31 production-gap disposition

| Prior claim | Correct production evidence | Iteration-33 disposition |
|---|---|---|
| `converge_shared` lacked required flow-task cancellation and should be changed in DELIVER. | The runtime supervisor acquires `owner.quiesce()` first; that existing operation cancels/awaits flow tasks and closes records before calling `converge_shared`. | **WITHDRAWN** — no cleanup belongs in `converge_shared`; the driver now composes the existing supervisor protocol. |

Iteration 31's bounded Sim `RecordSocket` owner-close projection remains
approved: it records the close produced by the existing quiescence cleanup.
Only its asserted missing-production-cleanup premise is superseded.

## Acceptance-review gates

| Gate | Result | Iteration-33 evidence |
|---|---|---|
| Audit-first recovery | PASS | Existing inventory failure is observed and marks Forwarder before hold acquisition. |
| Production quiescence | PASS | Exact owner `quiesce` completes flow cancellation, task joins, forwarder aborts, and socket cleanup before repair. |
| Move-only hold identity | PASS | The returned hold is retained privately and the same value is consumed by restore. |
| CtxRelease boundary | PASS | Old context is released and replacement open remains held after completed cleanup. |
| Reopen boundary | PASS | Repair join, clean owner audit, exact-hold restore, and settle complete before projection. |
| Failed-attempt posture | PASS | Repair/audit failure retains the hold fail-closed rather than reopening admission. |
| D-5 owner death | PRESERVED | A live context pair/watch still selects final owner death, not recovery. |
| Unattached resources | PASS | No-live-context recovery still cleans flow-table tasks through production quiescence. |
| No duplicate cleanup | PASS | `converge_shared` and production code are unchanged; the false remediation is withdrawn. |
| No hold leak on success | PASS | One acquired recovery hold is taken and restored exactly once. |
| Valid model states | PRESERVED | Duplicate recovery and unhealthy Reopen are protocol failures, not modeled valid outcomes. |
| Model/oracle integrity | PRESERVED | No Quint source, action, trace, expected state, or projection changed. |
| Surface discipline | PASS | Private driver state and existing ports only; no production API, seam, type, or recovery mechanism. |

## Mechanical verification

| Check | Result |
|---|---|
| Production supervisor inspection | PASS — recovery acquisition calls owner quiescence before the repair attempt and restores the retained hold only after a healthy attempt. |
| Owner quiescence inspection | PASS — first hold clears tables, cancels and awaits flow tasks, aborts forwarder work, and closes owned descriptors. |
| Driver boundary inspection | PASS — CtxRelease parks at replacement open; Reopen joins, audits, restores, and settles. |
| `rustfmt --edition 2024 --check` | PASS for `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and this review artifact. |
| Metal/conformance execution | NOT RUN by this bounded static reviewer; the handoff records the premise correction and reproduced trace. |
| Model rerecord and mutation testing | NOT RUN. |

## Iteration-33 verdict

**APPROVED.** The driver now composes the production Forwarder recovery's
actual owner-side protocol across the model's two atomic boundaries: audit,
completed quiescence and retained hold before repair; then repaired context,
clean audit and restoration of that exact hold. The prior production-gap
claim is withdrawn, and no duplicate cleanup, invented recovery mechanism,
resource leak, model change, or oracle weakening is introduced.

# APPROVED — SUPPLEMENTAL ITERATION 33

---

# Supplemental review iteration 34 — exact refusal/error projection and disabled-action formal traces

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-34` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL/formal-model reviewer |
| Scope | Two bounded driver-source alignments and the unimplemented `cid_lease` disabled-action trace proposal |
| Source verdict | **APPROVED** |
| Formal proposal verdict | **APPROVED FOR BOUNDED IMPLEMENTATION AND EVIDENCE RE-RECORD** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration separates three independent findings: the exact typed outcome
of the modeled pool-refused restart, retention of the real pre-attach TCP
socket fact after owner cleanup, and a Quint evaluator/ITF incompatibility in
six disabled-action named runs. The first two changes are implemented in the
current Rust source. The formal-model proposal remains unimplemented and is
reviewed here before any model or evidence mutation.

## Source review A — exact pool-refusal tick outcome

`AssignRefused(a)` models one production convergence tick whose successor
admission is refused by a full CID pool and whose predecessor cleanup then
parks at teardown. That transition deliberately leaves no placement
allocation for `a`. When the following modeled `Release(a)` grants the
predecessor teardown and the tick completes, the production runtime preserves
the admission refusal as its terminal tick result.

The CID driver now uses the existing `placement.is_none()` distinction only
at Release to select a dedicated reap path. That path accepts exactly:

```text
ConvergenceError::Shim(
  ShimError::GuestNetwork(
    GuestNetworkError::PoolExhausted { .. }
  )
)
```

It accepts no other guest-network admission error, generic shim error,
hydration/persistence error, successful tick, missing tick, or panicked task.
`Ok(())` fails as an action precondition mismatch; every different typed
error fails as a production error. The wildcard is confined to the approved
`PoolExhausted` payload fields and does not widen the semantic variant.

The special reap runs only after the existing modeled refusal removed the
placement and Release completed the held predecessor cleanup. Ordinary
placement, stop, finalize, clash, and successful-restart releases retain the
existing `reap_tick`, which still requires `Ok`. The production refusal is
therefore consumed only where the model already includes that refusal and its
ordered cleanup; it is not globally swallowed.

The defensive unfinished-task arm cannot admit an incomplete effect at this
call site: Release's preceding settle returns to the reap branch only when no
new held teardown remains and the exact workload tick is finished. A missing
or different task state still fails.

## Source review B — exact retained TCP socket fact

In the reproduced pre-attach trace, `TcpRst("h")` selected the owner's one
unattached TCP peer and set that exact `SimSocket`'s real `sk_err`. Production
quiescence then correctly removed the live peer handle. The model retains the
terminal TCP error, but the prior projection fell back to no socket and
reported `tcpErr(h) = 0`.

The driver now stores an `Option<SimSocket>` clone of the exact socket chosen
by `peer_socket`. Attached flows continue to prefer the seam's
flow-correlated peer. Before attach, the existing single-peer fallback is
selected and the same object is retained. After owner cleanup removes the
handle, projection reads `sk_err` from that retained shared object.

No errno, label, or terminal state is copied into driver-owned scalar state.
`TcpRst` mutates the retained Sim socket itself, and QPROJ-8 later reads that
socket's actual `sk_err`. Retaining an `Arc`-backed socket observation does not
retain a `SeamHandle`, flow task, forwarder pair, owner-table row, or cleanup
right, so it cannot keep production ownership alive or inhibit cleanup.
Attached correlation remains first priority, and the former live-unattached
fallback remains last; the new retained identity only bridges the exact
pre-attach socket across its owner's completed cleanup.

## Primary evidence for the formal incompatibility

Exactly six live `lease_conf_ok` Rust named tests end their Quint run with a
deliberately disabled transition:

- `ReleaseWhileTheVmmRunsIsRefused` — disabled `Release(1)`;
- `ServeCrashOrphanIsReclaimedBeforeAnyAssign` — disabled `Assign(1)`;
- `StoppedWorkloadStaysStoppedAcrossServeCrash` — disabled `Assign(1)`;
- `FailureAtCeilingExhaustsTheBudget` — disabled `Assign(1)`;
- `ServeCrashDuringFinalizeFinalizesAfterRestart` — disabled `Assign(1)`; and
- `StopOfAnEndedWorkloadReclaimsItsLease` — disabled `Assign(1)`.

The current Quint 0.32 evaluator correctly reports all six named runs as
holding. For `quint test`, however, `.then(Action).fail()` emits an additional
terminal empty ITF record `{}` for the failed transition. This is not a model
state produced by an action and carries no `conf_step`.

The workspace pins `quint-connect = 0.1.2`. Its local source
`driver/step.rs::extract_from_sum_type` unconditionally calls
`find_record_in_path` for the configured nondeterminism path before any driver
step. `CidLeaseDriver` configures
`lease_conf_ok::cid_lease::conf_step`, so the empty record fails path
extraction before the Rust driver can run or project production state. There
is no driver action, refusal, or implementation mismatch behind these six
failures.

Current checked-in formal evidence is internally current before the proposed
edit: both the file and `evidence/summary.json` record
`cid_lease.qnt = 8f9bf5ef20f14596f8b0582ae95f302673eb396d3899cfac45141a97c8cd0cbb`
and
`checks.toml = 6915260626ab7238a55da721f0dbe26c262da6e00c20d76c214551b3096470a4`.
The incompatibility comes from the actual terminal emission and the pinned
parser behavior, rather than stale model evidence.

## Formal equivalence of the proposed rewrite

The proposal is semantics-preserving provided the implementation factors the
existing guards exactly and changes no transition effect.

The pure Assign predicate must be exactly equivalent to the current action's
enabling conjunction:

```quint
def assignEnabled(a: int): bool = all {
  not(BUG_ASSIGN_SPLIT),
  up,
  reclaimed or BUG_ASSIGN_BEFORE_RECLAIM,
  freeOffs != Set(),
  not(tickBlocked(a)),
  val x = al.get(a)
  x.st == "wait" or (x.st == "ended" and x.retry and x.off >= 0),
}
```

`Assign(a)` may replace those guard clauses with `assignEnabled(a)` but must
retain its exact `x`, `fromEnded`, next-fit choice, primed assignments,
restart-attempt update, and `conf_step` write.

The pure Release predicate must preserve the current ordered branch guards:

```quint
def releaseEnabled(a: int): bool = all {
  up,
  val x = al.get(a)
  if (x.st == "retiring" or
      (BUG_RELEASE_BEFORE_VMM and x.st == "running" and x.pst != "retiring"))
    devAt(x.off) != a or BUG_RELEASE_BEFORE_VMM
  else if (BUG_TERMINAL_BEFORE_RELEASE and x.st == "failed" and x.off >= 0)
    true
  else
    all { x.pst == "retiring", devAt(x.pred) != a },
}
```

`Release(a)` may call `releaseEnabled(a)` but must retain the same branch
selection, every primed assignment, finalization bookkeeping, and
`conf_step` write. The predicates must not add inferred conditions such as
`off >= 0`, lease membership, `Safety`, or a stronger predecessor shape to a
branch that does not currently guard on them.

Under those exact definitions, `assignEnabled(a)` iff `Assign(a)` has a
transition and `releaseEnabled(a)` iff `Release(a)` has a transition. Their
effect expressions are total whenever the existing guards hold: Assign's
next-fit is guarded by a nonempty free set, and Release's `devAt` is already
defined outside the model offset set. Factoring introduces no state variable,
nondeterminism, temporal assumption, fairness change, or additional action.
The `step` relation and every TLC/Apalache reachable behavior remain
identical.

Each of the six named runs may then replace its terminal
`.then(Action).fail()` with an expectation on the unchanged current state:

```quint
.expect(not(releaseEnabled(1)))
```

or:

```quint
.expect(not(assignEnabled(1)))
```

The formal claim is the same: the named action has no successor from that
state. Unlike `.fail()`, the expectation asks Quint to check a pure predicate
and does not emit a failed-transition placeholder. The last full state and
its previous action's `conf_step` remain available to quint-connect, which
replays only transitions that actually occurred.

This does not weaken executable coverage. The current `.fail()` step never
reaches the Rust driver; it already proves only formal action disablement.
After the rewrite, the evaluator proves that exact disablement through the
factored action guard, while quint-connect replays and compares the complete
reachable prefix. Test names, prefixes, scenarios, assertions, action names,
and model outcomes remain intact.

The model-only `CrashIsNotEnabledInThe325Window` `.fail()` run is outside
these six Rust conformance targets and outside this proposal; it must remain
unchanged. No other `.fail()` run, model, or conformance input is authorized
for editing.

## Bounded implementation and evidence authorization

**APPROVED FOR IMPLEMENTATION** with this exact scope:

1. edit only `specs/quint/guest-flow-owner/cid_lease.qnt` to add the two pure
   predicates, make `Assign`/`Release` use their exact guards, and rewrite the
   six named-run terminals listed above;
2. do not change `checks.toml`, Rust driver/test bodies, action/variant names,
   `conf_step`, constants, variables, transitions, fairness, hazards, or the
   model-only Crash guard run;
3. run the six existing Rust `#[quint_test]` cells and confirm their emitted
   traces contain full states only and execute no disabled Assign/Release
   driver step;
4. run the repository's formal typecheck;
5. regenerate the complete guest-flow-owner evidence with the established
   three-job command:

   ```text
   cargo xtask quint check --subsystem guest-flow-owner --record --jobs 3 --mem-budget 12g
   ```

6. verify the regenerated record:

   ```text
   cargo xtask quint verify-evidence --subsystem guest-flow-owner
   ```

7. require the generated `evidence/summary.json` and `summary.md` to contain
   the actual new SHA-256 for `cid_lease.qnt`, retain the unchanged actual
   `checks.toml` hash, and report every configured outcome matching `expect`.

Evidence hashes and summaries must be generated by the evidence command, not
hand-edited. A focused evaluator pass alone does not replace the full
three-job re-record because the changed formal input invalidates the subsystem
record.

## Acceptance and formal-review gates

| Gate | Result | Iteration-34 evidence |
|---|---|---|
| Pool refusal representation | PASS | Only the exact `ConvergenceError::Shim(ShimError::GuestNetwork(PoolExhausted))` envelope is accepted. |
| Error complement | PASS | Success, missing task, panic, and every other typed error fail. |
| Refusal scope | PASS | Special reap is confined to the placement-none `AssignRefused` Release path. |
| Pre-attach socket identity | PASS | Driver retains the exact selected `SimSocket`, not an errno copy or replacement socket. |
| Cleanup independence | PASS | Retained socket observation holds no seam handle, owner row, task, or pair. |
| Attached precedence | PRESERVED | Flow-correlated peer remains the projection's first choice. |
| Current formal evidence | PASS | Current source and recorded `cid_lease.qnt`/`checks.toml` hashes agree before the proposal. |
| Failure locus | PASS | Empty failed-transition ITF record lacks `conf_step`; quint-connect fails before driver execution. |
| Assign predicate equivalence | PASS | Proposed predicate is exactly the existing Assign guard and adds no inferred restriction. |
| Release predicate equivalence | PASS | Proposed predicate preserves current branch priority and exact branch guards. |
| Transition relation | PRESERVED | Action effects, variables, action names, fairness, and `conf_step` remain unchanged. |
| Disabled-action claim | PRESERVED | `not(enabled)` is equivalent to absence of the action transition at each named prefix. |
| Cell inventory | PRESERVED | All six Rust named cells and their full prefixes remain; no trace state is filtered or dropped. |
| Adjacent formal runs | PRESERVED | Model-only Crash `.fail()` and all other specs/runs stay untouched. |
| Evidence discipline | REQUIRED | Full jobs=3 re-record, actual input hashes, and verify-evidence are mandatory after implementation. |

## Mechanical verification

| Check | Result |
|---|---|
| Rust source diff inspection | PASS for exact refusal and retained-socket changes. |
| quint-connect 0.1.2 local source inspection | PASS — configured sum-type path extraction is unconditional for every ITF state. |
| Current input hash comparison | PASS — `cid_lease.qnt` and `checks.toml` match the checked-in evidence record. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed Rust files. |
| Scoped `git diff --check` | PASS for reviewed Rust files, handoff, and this review artifact. |
| Formal model/source edits | NOT MADE by this reviewer; proposal remains pending implementation. |
| Test/formal/Metal/mutation jobs | NOT RUN by this bounded reviewer. |

## Iteration-34 verdict

**APPROVED.** The two implemented source alignments preserve exact typed
error and kernel-fact evidence without swallowing adjacent failures or
copying synthetic state. The disabled-action Quint rewrite is formally
equivalent when implemented with the exact predicates and six-run scope above,
so bounded implementation plus the mandatory jobs=3 full evidence re-record
and hash verification is approved.

# APPROVED — SUPPLEMENTAL ITERATION 34

---

# Supplemental review iteration 35 — obsolete netlink inventory deletion and implemented Quint guard factoring

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-35` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL/formal-model reviewer |
| Scope | Netlink test inventory made obsolete by the accepted TAP/bridge cut and the implemented `cid_lease` disabled-action rewrite |
| Source verdict | **APPROVED** |
| Formal execution status | **MANDATORY NEXT GATE — not yet run** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews the actual source package before the formal evidence
job. It contains two bounded changes: removal of tests whose complete subject
was deleted by the accepted netlink cut, and the exact Quint implementation
approved in iteration 34. No formal execution or evidence regeneration is
claimed; the attempted Lima typecheck lost its SSH connection before Quint
started.

## Netlink deletion inventory

### Source-local unit fallout

`client.rs` no longer imports or builds the deleted TAP/link-identity decoder
types. Exactly two pure-function tests left with those decoders:

- `persistent_tap_classifier_requires_tap_type_and_persistence`; and
- `persistent_tap_and_bridge_projection_preserves_every_observable_identity_field`.

Both construct the removed `TapLinkState`, `PersistentTapIdentity`,
`ObservedLinkIdentity`, and `ObservedLinkKind` surfaces and exercise no
retained route, rule, or FIB behavior. Their private typed-netlink builders and
imports were their last callers and are deleted with them.

The retained `client.rs` tests still exercise the local-route key and all
fwmark/table matching combinations, including header and attribute tables,
same-rule conjunction, vanilla-rule exclusion, and wrong-table refusal. The
new retained production FIB/local-route APIs and their supporting route types
remain compiled by the module; nothing in the deleted TAP/link-identity tests
provided neutral coverage for them.

`ethtool.rs` deletes the UAPI-constant test for the removed debug-message-mask
protocol. Its constants and decoder were removed with that production API, so
the test has no retained subject. The TX-checksum suite remains: command and
nested-header wire pins, the property over encoded off requests, exact
changeable-target selection, active-state observation, endian readers, and
nested-attribute behavior are unchanged.

### Real-kernel integration inventory

The deleted `tests/integration.rs` entrypoint declared only four modules, all
of which are removed:

- `bridge_guard_lifecycle` drives only the deleted nft bridge guard table,
  chain, set, rules, membership, and inverse cleanup operations;
- `managed_link_address` drives only deleted bridge creation/adoption,
  semantic link identity, and persistent-TAP construction;
- `scratch_tap` is test support for those removed TAP/link modules and has no
  independent cell; and
- `tap_debug_msg_mask` drives only deleted persistent-TAP and debug-mask
  readback surfaces.

There was no route, rule, FIB lookup, local-route, nft-neutral, or TX-offload
cell in that integration entrypoint. Deleting the entrypoint therefore removes
no retained integration module by omission. Retained topology-neutral unit
coverage continues in `client.rs`, `ethtool.rs`, `nft.rs`, `error.rs`, and
`setns.rs`.

Repository search under `overdrive-netlink` now finds no live reference to the
deleted TAP/link identity, bridge guard, persistent-TAP, debug-mask types,
functions, module declarations, or integration helpers. No compatibility shim
or dead import remains.

## Implemented Quint predicate equivalence

The current `cid_lease.qnt` adds only two pure derived predicates:
`canAssign(a)` and `canRelease(a)`. Their names differ from iteration 34's
illustrative names but their semantics match the approved definitions.

`canAssign(a)` copies every prior Assign guard:

- `not(BUG_ASSIGN_SPLIT)`;
- serve up;
- reclaimed, except the existing assign-before-reclaim hazard;
- a nonempty free-offset set;
- no blocked predecessor teardown; and
- the exact `wait` or retryable-ended source-state condition.

`Assign(a)` now invokes `canAssign(a)` and retains the same `x`,
`fromEnded`, next-fit selection, lease/cursor update, predecessor ownership,
attempt update, unchanged variables, and `conf_step` assignment. The repeated
local bindings do not change evaluation: every invocation ranges over
`ALLOCS`, where `al.get(a)` is total.

`canRelease(a)` preserves the former ordered branch guards exactly:

1. serve must be up;
2. placement-retiring, or the existing release-before-VMM hazard branch,
   requires the same device-holder condition;
3. the terminal-before-release hazard branch has no additional guard; and
4. the predecessor branch requires `pst == "retiring"` and the same device
   condition on `pred`.

`Release(a)` invokes that predicate, selects the same branch with the same
condition, and retains every state update and `conf_step` write. Moving the
branch guards into the pure predicate adds no invariant-derived restriction,
changes no branch priority, and does not make any former disabled action
enabled.

The state machine is consequently unchanged: no variable, action, constant,
main, transition effect, nondeterministic choice, fairness expression,
invariant, hazard, or conformance-step variant changed.

## Six named disabled-action witnesses

The six live Rust conformance runs approved in iteration 34 now replace only
their final failed transition with a pure expectation:

- one `expect(not(canRelease(1)))`; and
- five `expect(not(canAssign(1)))`.

Every prefix, intermediate `Safety`/outcome assertion, run name, conformance
main, and final refusal claim is unchanged. The predicate is the exact guard
of the named action, so each expectation proves the same absence of a
successor transition as `.then(Action).fail()` without asking Quint to emit an
empty failed-transition ITF record. No state is filtered and no replacement
action is emitted for the Rust driver.

The only remaining `.fail()` in `cid_lease.qnt` is
`CrashIsNotEnabledInThe325Window`. It is the explicitly model-only pending
#325 witness and has no live `#[quint_test]`; it remains byte-for-byte in
place. `checks.toml`, all Rust driver/test bodies, and every other formal file
are unchanged.

## Evidence state and mandatory next gate

The implemented `cid_lease.qnt` currently hashes to:

```text
c527f987c4ded33b99049afb313206004e503a4dc47d0a04d51de07058498730
```

`checks.toml` remains:

```text
6915260626ab7238a55da721f0dbe26c262da6e00c20d76c214551b3096470a4
```

The checked-in evidence still records the former `cid_lease.qnt` hash
`8f9bf5ef20f14596f8b0582ae95f302673eb396d3899cfac45141a97c8cd0cbb`,
jobs=3, and 279 passing checks. That stale hash is expected at this pre-job
review boundary and is not accepted as final evidence.

The source is approved to proceed through these required gates:

1. Quint typecheck;
2. the six existing Rust `#[quint_test]` cells, proving full ITF states reach
   quint-connect and no disabled Assign/Release driver step is manufactured;
3. full formal re-record:

   ```text
   cargo xtask quint check --subsystem guest-flow-owner --record --jobs 3 --mem-budget 12g
   ```

4. evidence verification:

   ```text
   cargo xtask quint verify-evidence --subsystem guest-flow-owner
   ```

The generated `summary.json` and `summary.md` must record the actual current
`cid_lease.qnt` hash above, retain the unchanged `checks.toml` hash, preserve
the 279-check inventory, and show every outcome matching `expect`. Any further
model edit requires review of its new actual hash; the evidence files must be
tool-generated rather than hand-edited.

## Review gates

| Gate | Result | Iteration-35 evidence |
|---|---|---|
| Netlink last-caller deletion | PASS | Deleted unit helpers/tests have no retained caller or subject. |
| Integration entrypoint inventory | PASS | It declared only the four removed TAP/bridge/debug modules. |
| Retained netlink coverage | PRESERVED | Route/rule/local/FIB, TX offload, nft-neutral, error, and setns unit suites remain. |
| Deleted-reference search | PASS | No removed TAP/link/bridge-guard/debug-mask symbol or module declaration remains under the crate. |
| Assign guard equivalence | PASS | `canAssign` is the exact prior guard; all effects stay in `Assign`. |
| Release guard equivalence | PASS | `canRelease` preserves branch order and every prior guard; all effects stay in `Release`. |
| Formal state machine | PRESERVED | No variable, action effect, state relation, fairness, invariant, main, or hazard changed. |
| Six refusal witnesses | PRESERVED | Same prefixes and disabled-action claims use pure guard expectations. |
| Pending #325 witness | PRESERVED | The sole remaining model-only `.fail()` is unchanged. |
| Trace filtering | PASS | No empty-state filter, dropped state, or fabricated action was added. |
| Formal execution claim | PASS | None claimed; Lima failed before Quint started. |
| Evidence freshness | PENDING REQUIRED GATE | Current source hash differs from the pre-job record; jobs=3 re-record and verification must update it. |

## Mechanical verification

| Check | Result |
|---|---|
| Quint diff inspection | PASS — 25 insertions/12 deletions confined to pure guard factoring and six terminal expectations. |
| Remaining `.fail()` inventory | PASS — only the pending-#325 model-only Crash witness remains. |
| Formal/evidence file scope | PASS — only `cid_lease.qnt` changed; `checks.toml` and evidence are untouched before the job. |
| `rustfmt --edition 2024 --check` | PASS for the reviewed netlink Rust files. |
| Scoped `git diff --check` | PASS for netlink, `cid_lease.qnt`, handoff, and this review artifact. |
| Quint typecheck/tests/record/verify | NOT RUN by this reviewer. |
| Metal and mutation testing | NOT RUN. |

## Iteration-35 verdict

**APPROVED.** The netlink deletions remove only tests and fixtures whose TAP,
bridge-guard, link-identity, or debug-mask subject was deleted, while retained
route/rule/FIB and TX-offload coverage remains. The implemented Quint helpers
are exact pure factorizations of the prior guards, and the six named runs
preserve their refusal outcomes without emitting failed-transition records.
The source is approved for the mandatory typecheck, six-cell replay, jobs=3
formal re-record, and evidence/hash verification.

# APPROVED — SUPPLEMENTAL ITERATION 35

---

# Supplemental review iteration 36 — dead-owner serving-generation snapshot

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-36` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Existing `fwd_pair` projection after final host-owner death |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

A live-watch trace selected the D-5 final-owner-death branch of
`CtxRelease(h)`. The driver correctly ended the Sim process and omitted owner
state, but the subsequent projection still called `owner.flow_observation()`
to derive the number of serving context rows. That observation synchronously
reads `GuestFlowIo::inventory`; after `exit_process`, such owner I/O is
invalid and the fixture panicked before comparing its retained kernel and
socket facts.

## Snapshot source and lifetime

`owner_death` now reads the production owner's actual
`flow_observation().forwarder_generation` immediately before invoking
`exit_process` and stores that copy in private driver state. There is no await
or scheduling boundary between completion of the observation and process
exit: the current-thread owner cannot open or replace another context in that
interval. The copied `ForwarderGeneration` is therefore the exact generation
of the context history that the ensuing process exit releases.

The field starts as `None` in every fresh driver and is written only by
`owner_death`. The normal `init_with` replacement constructs a fresh driver,
so a snapshot cannot cross traces or owner instances. If dead-owner
projection somehow occurs without that pre-exit capture, it fails explicitly
instead of substituting `FIRST`, a context-table length, or a model value.

## Live/dead I/O boundary

Projection now selects the generation source by owner lifetime:

- while the owner lives, it retains the existing current
  `owner.flow_observation()` read; and
- after final owner death, it uses only the captured generation.

The dead branch performs no owner port call. It continues to read the Sim
device's retained context table and the retained peer/vsock socket objects,
which are kernel/seam facts deliberately preserved after process exit. It
uses the snapshot only to split that device history into the exact one-row
phase-1 prefix and the owner-serving suffix. Existing prefix cardinality and
resource-empty checks remain active, so the snapshot cannot hide an extra
context row or phase-1 resource.

`project_owner` remains unreachable when `owner_dead`: the projected owner is
still `None`. No CID table, flow phase, generation counter, attach counter,
inventory result, or consumed record is reconstructed post-mortem.

## Sudden-death path audit

Every modeled sudden host-owner death reaches the single `owner_death`
function: `ctx_release_h` selects it only when the current forwarder inventory
contains a live pair or watch. That function captures the generation, exits
the seam/process, drops the owner runtime and its tasks, marks the owner dead,
and retains only the existing socket/context observations. The model then
makes host death final, and `alive` continues to reject every later owner
action.

The in-process Forwarder recovery branch does not set `owner_dead` and does
not use the snapshot. Its live projection continues through the current owner
observation across CtxRelease/Reopen. End-of-test `Composition::drop` also
needs no snapshot because no model state is projected after fixture teardown.
Thus the field covers all and only projected final-death paths.

The snapshot remains correct after any actual serving generation: generation
1 selects the first serving row; a later generation selects its complete
serving history. `exit_process` releases the current row without removing the
device history, so the existing `ctx`/`oldCtx` computation still observes the
released current context and any real serving predecessor.

## Oracle and surface audit

The model's owner-death outcome is unchanged: `owner = None`, current context
released, no later owner step, and retained socket/end facts compared exactly.
The snapshot is the same QPROJ-8 generation value used while live; it is not
an expected value copied from the model and does not synthesize a context,
inventory, socket effect, or owner state.

No production code, public API, Sim seam, Quint source, action, scenario,
projection type, or expected outcome changed.

## Review gates

| Gate | Result | Iteration-36 evidence |
|---|---|---|
| Actual source | PASS | Snapshot comes from the live production owner's accepted observation port. |
| Capture ordering | PASS | Observation completes immediately before `exit_process`, with no scheduling point between them. |
| Dead-owner I/O boundary | PASS | Post-mortem projection makes no owner port or inventory call. |
| Live-owner behavior | PRESERVED | Live projections still read the current owner observation. |
| Context history | PRESERVED | Snapshot only partitions retained phase/serving rows; all row/resource validations remain. |
| Owner absence | PRESERVED | Owner projection remains exactly `None` after final death. |
| Socket/end evidence | PRESERVED | Retained TCP/vsock facts and context release outcomes are unchanged. |
| All projected death paths | PASS | The sole D-5 owner-death function captures before every modeled final exit. |
| Recovery path | PRESERVED | In-process CtxRelease/Reopen remains live and does not consult the snapshot. |
| No synthetic fallback | PASS | Missing snapshot is an explicit error, never a default generation. |
| Trace isolation | PASS | Fresh driver construction resets the optional snapshot. |
| Surface discipline | PASS | Private driver field only; no production/API/seam/model change. |

## Mechanical verification

| Check | Result |
|---|---|
| Owner-death call inventory | PASS — one modeled final-death function and one call site. |
| Projection branch inspection | PASS — only the live branch calls `flow_observation`; `project_owner` remains live-only. |
| `rustfmt --edition 2024 --check` | PASS for `quint_fwd_pair.rs`. |
| Scoped `git diff --check` | PASS for the driver, handoff, and this review artifact. |
| Test/Metal/formal/mutation execution | NOT RUN by this bounded static reviewer. |

## Iteration-36 verdict

**APPROVED.** Final owner death now captures the real serving generation while
the owner port is still valid and uses that value only for post-mortem context
history partitioning. Live projection, retained socket/context oracles,
owner absence, and all recovery/model behavior remain unchanged, with no
synthetic expected state or post-exit owner I/O.

# APPROVED — SUPPLEMENTAL ITERATION 36

---

# Supplemental review iteration 37 — Quint state-reading predicate qualifier

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-37` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL/formal-model reviewer |
| Scope | Compiler-required qualifier correction for `canAssign` and `canRelease` |
| Verdict | **APPROVED** |
| Formal execution status | **MANDATORY NEXT GATE — corrected source not yet executed** |
| Open findings | 0 critical, 0 high, 0 medium |

The first Metal Quint typecheck reached the newly factored predicates and
rejected their initial `pure def` declarations with QNT200. Both predicate
bodies read model state (`al`, `up`, `reclaimed`, `freeOffs`, `dev`, and
derived state readers). Quint's effect discipline therefore requires ordinary
`def`; `pure def` is reserved for definitions with no state reads.

## Exact correction

The source changes only these two qualifiers:

```quint
def canAssign(a: int): bool = ...
def canRelease(a: int): bool = ...
```

Their parameters, return types, expressions, branch order, and references are
unchanged. `Assign` and `Release` still use them as the same exact guards, and
the six named refusal witnesses still evaluate the same
`not(canAssign(1))` / `not(canRelease(1))` predicates.

An ordinary Quint `def` may read the current state but introduces no state
transition. These definitions contain no primed variable, action call,
nondeterministic choice, temporal operator, or mutation. Their Boolean value
at a state is identical to the value of the rejected `pure def` spelling; the
qualifier only makes the declared effect match the expression's actual state
reads.

The correction therefore leaves every property approved in iterations 34–35
intact:

- the Assign and Release guards remain exact factorizations;
- action effects and `conf_step` writes remain unchanged;
- the six named prefixes and disabled-action outcomes remain unchanged;
- the sole model-only pending-#325 `.fail()` remains unchanged; and
- no trace filtering, state variable, action, main, invariant, fairness rule,
  hazard, Rust driver, or check inventory changes.

## Failed-run and evidence disposition

The QNT200 run is retained as compiler evidence for the qualifier correction;
it is not a semantic model result. It stopped at typecheck, before the six-cell
replay, evaluator checks, evidence record, or verification. No evidence file
was changed.

The corrected `cid_lease.qnt` now hashes to:

```text
c0e3b2b077067ae1fff8c1e61f055bdc18c384a64af4d2f7592747e556464481
```

This supersedes iteration 35's pre-correction source hash. The unchanged
`checks.toml` hash remains
`6915260626ab7238a55da721f0dbe26c262da6e00c20d76c214551b3096470a4`.
The checked-in evidence still carries the old pre-proposal model hash and must
not be treated as current.

After this source approval, the mandatory sequence remains:

1. rerun Quint typecheck and require QNT200 to be absent;
2. run the six live Rust `#[quint_test]` refusal cells;
3. regenerate all guest-flow-owner evidence with jobs=3; and
4. run subsystem `verify-evidence`, requiring the generated summaries to
   record the actual corrected hash above and all 279 expected outcomes.

## Review gates

| Gate | Result | Iteration-37 evidence |
|---|---|---|
| QNT200 cause | PASS | Both helpers read current model state, which `pure def` forbids. |
| Qualifier scope | PASS | Only `pure def` → `def` changed for the two helpers. |
| Predicate bodies | PRESERVED | Every approved guard clause and branch is unchanged. |
| Action relation | PRESERVED | Assign/Release effects and enabled-state sets are unchanged. |
| Named witnesses | PRESERVED | Six `expect(not(predicate))` terminals are unchanged. |
| Side effects | PASS | Ordinary definitions read state but contain no primed write or action. |
| Adjacent model | PRESERVED | Variables, mains, invariants, fairness, hazards, #325 witness, and check inventory are untouched. |
| Evidence honesty | PASS | Failed typecheck is recorded; no later job or evidence freshness is claimed. |
| Evidence freshness | PENDING REQUIRED GATE | Re-record must use the corrected actual source hash. |

## Mechanical verification

| Check | Result |
|---|---|
| Source inspection | PASS — two state-reading `def` declarations, no `pure def` remains for these helpers. |
| Current SHA-256 | PASS — `c0e3b2b077067ae1fff8c1e61f055bdc18c384a64af4d2f7592747e556464481`. |
| Scoped `git diff --check` | PASS for `cid_lease.qnt`, handoff, and this review artifact. |
| Corrected Quint typecheck/replay/record/verify | NOT RUN by this reviewer. |
| Metal and mutation testing | NOT RUN. |

## Iteration-37 verdict

**APPROVED.** Changing the two state-reading helper declarations from
`pure def` to ordinary `def` is the exact compiler-required correction. Their
values, action guards, six refusal witnesses, transition relation, and formal
outcomes remain unchanged. Corrected typecheck, six-cell replay, jobs=3
re-record, and evidence verification remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 37

---

# Supplemental review iteration 38 — completed formal evidence disposition

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-38` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL/formal-artifact reviewer |
| Scope | Post-implementation Quint typecheck, six-cell replay, full guest-flow-owner record, and evidence verification |
| Verdict | **APPROVED — FORMAL GATE COMPLETE** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration closes the execution gates left pending by iterations 35 and
37. It reviews the recorded commands and the synced evidence artifacts; it
does not rerun any formal job.

## Executed command sequence

The approved source was exercised through these four Metal commands in order:

```text
cargo xtask metal run -- cargo xtask quint typecheck
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(quint_cid_lease_failure_at_ceiling_exhausts_the_budget) | test(quint_cid_lease_release_while_the_vmm_runs_is_refused) | test(quint_cid_lease_serve_crash_during_finalize_finalizes_after_restart) | test(quint_cid_lease_serve_crash_orphan_is_reclaimed_before_any_assign) | test(quint_cid_lease_stop_of_an_ended_workload_reclaims_its_lease) | test(quint_cid_lease_stopped_workload_stays_stopped_across_serve_crash)' --no-fail-fast --no-capture
cargo xtask metal run -- cargo xtask quint check --subsystem guest-flow-owner --jobs 3 --record
cargo xtask metal run -- cargo xtask quint verify-evidence --subsystem guest-flow-owner
```

The corrected model typechecked all seven specifications. The six formerly
empty-terminal CID cells passed 6/6, proving the guard expectations emit
replayable full states and do not manufacture a disabled Assign or Release
driver step.

The complete three-job formal record passed 279/279 under run id
`2043112-1792964018.889`. The subsequent subsystem `verify-evidence` passed.
No partial, focused, or prior run is substituted for this complete record.

## Summary and input integrity

Independent parsing of `evidence/summary.json` establishes:

- schema 1 and subsystem `guest-flow-owner`;
- jobs = 3;
- 279 check records with 279 unique names;
- all 279 have `pass = true`;
- all 279 have `outcome == expect`;
- all 24 recorded input hashes match the current local input bytes; and
- every referenced trace, condensed trace, and failure-log path exists.

The two contract-shaping input hashes are exact:

```text
cid_lease.qnt  c0e3b2b077067ae1fff8c1e61f055bdc18c384a64af4d2f7592747e556464481
checks.toml    6915260626ab7238a55da721f0dbe26c262da6e00c20d76c214551b3096470a4
```

The generated summary artifacts hash to:

```text
summary.json  9e1e6cb36e818f54c805feaf77dfee64e5cc1c72ae11afe6531f470e1836db76
summary.md    57e754363dad80508025c5b0109592ed809c134d7c612b852f375f9defec9038
```

`summary.md` independently renders the same run id, jobs count, 279/279
result, tool versions, and both input hashes. The evidence is therefore tied
to the exact approved state-reading-`def` source, rather than the rejected
`pure def` intermediate or the pre-proposal model.

## Trace receipt and timing audit

All 70 checks whose expected outcome is `violation` carry at least one trace
receipt, and every referenced receipt exists. No counterexample or hazard
outcome is represented only by a missing path.

The one reported timing warning is informational, not a missing receipt or
timeout. `udp-slots-hazard-late-paired-by-slot-route` completed in 181.93
seconds within its 900-second bound, returned its expected `violation`, has
`pass = true`, and retains both:

- `traces/udp-slots-hazard-late-paired-by-slot-route.itf.json`; and
- `traces/udp-slots-hazard-late-paired-by-slot-route.txt`.

Its result therefore remains valid despite exceeding the CI timing advisory.

## Artifact scope and ownership repair

The formal package changes exactly one input plus 80 generated evidence files:

- `cid_lease.qnt`;
- `evidence/summary.json` and `evidence/summary.md`; and
- 78 generated trace artifacts.

`checks.toml` and every other formal input are unchanged. The recorder's
remote evidence subtree was initially root-owned, which prevented sync-back.
The repair changed ownership only for
`/home/ubuntu/overdrive/specs/quint/guest-flow-owner/evidence` and then synced
that exact subtree. The local evidence tree is consistently owned by
`marcus:staff`; the generated run id and artifact hashes match the completed
record. No source outside the approved `cid_lease.qnt` input was introduced by
that ownership repair.

The broad trace-file churn is expected from a full 279-check record: it is
generated evidence across the subsystem, while the summary's complete input
hash map and outcome inventory bind every artifact to the approved inputs.

## Completion gates

| Gate | Result | Iteration-38 evidence |
|---|---|---|
| Quint typecheck | PASS | Seven specifications typechecked after the `def` correction. |
| Six disabled-action cells | PASS | 6/6 existing CID `#[quint_test]` cells passed. |
| Full record | PASS | Run `2043112-1792964018.889`, jobs=3, 279/279. |
| Evidence verification | PASS | Subsystem `verify-evidence` completed successfully. |
| Outcome integrity | PASS | 279 unique checks; every `pass` true and every outcome equals expect. |
| Input integrity | PASS | All 24 recorded input hashes match current local files. |
| Approved CID source | PASS | Recorded hash is exactly `c0e3b2…4481`. |
| Check inventory | PRESERVED | `checks.toml` hash remains exactly `691526…0a4`. |
| Trace receipts | PASS | All 70 expected violations have existing evidence receipts. |
| Timing advisory | NON-BLOCKING | Slow check completed inside timeout with expected violation and both receipts. |
| Artifact scope | PASS | One approved input plus 80 generated evidence files; no other formal source changed. |
| Ownership repair | PASS | Limited to the remote evidence subtree; synced local evidence ownership is consistent. |
| Reviewer rerun | NOT RUN | This disposition verifies completed artifacts without launching another formal job. |

## Mechanical verification

| Check | Result |
|---|---|
| SHA-256 recomputation | PASS for CID input, checks inventory, and both summaries. |
| JSON structural audit | PASS for run metadata, unique checks, outcomes, input hashes, and receipt paths. |
| Changed-file inventory | PASS — 80 evidence files and the one approved model input. |
| Local evidence ownership | PASS — 142 evidence files report `marcus:staff`. |
| Scoped `git diff --check` | PASS for the formal subtree, handoff, and this review artifact. |

## Iteration-38 verdict

**APPROVED — FORMAL GATE COMPLETE.** The corrected `cid_lease` source
typechecks, all six disabled-action conformance cells pass, the full jobs=3
record matches all 279 expected outcomes, and the verified evidence hashes
match every current input. The timing advisory has complete violation
receipts, and the ownership repair was confined to syncing the generated
evidence subtree.

# APPROVED — SUPPLEMENTAL ITERATION 38

---

# Supplemental review iteration 39 — retained TX-offload test UAPI tag

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-39` |
| Date | 2026-10-25 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Test-local `ETHTOOL_A_BITSET_BIT_VALUE` tag used by the retained FEATURES_SET payload parser |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

The accepted debug-mask deletion removed a formerly module-level constant that
one retained TX-offload test parser also used. The production encoder still
intentionally omits `ETHTOOL_A_BITSET_BIT_VALUE`; the test must know that UAPI
tag in order to prove the omission. The current correction defines only:

```rust
const ETHTOOL_A_BITSET_BIT_VALUE: u16 = 3;
```

inside the existing `#[cfg(test)] mod tests`.

## UAPI identity

The value is exact. The vendored Linux UAPI declares the nested bit attributes
in this order:

```text
UNSPEC = 0, INDEX = 1, NAME = 2, VALUE = 3
```

The workspace's `ethtool 0.2.9` dependency independently pins the same three
named constants as INDEX 1, NAME 2, VALUE 3. The test-local value therefore
matches both the vendored kernel header and the decoder used by the retained
feature family.

## Scope and oracle audit

The constant is compiled only with the source-local test module. Production
encoding still emits each requested feature as a nested `BITS_BIT` carrying a
`BIT_NAME` and deliberately no `BIT_VALUE`, which requests value zero/off.

The existing `parse_wanted_bits` body is unchanged. It walks
FEATURES_WANTED → BITSET_BITS → BITS_BIT, reads the existing name attribute,
and reports whether tag 3 is present. The existing property body is also
unchanged: it requires every requested name exactly once and requires
`has_value == false` for every bit. The correction therefore restores the
compiler input for the retained no-BIT_VALUE oracle without adding or
weakening an assertion.

No debug-message-mask constant, command, decoder, production API, integration
test, or TAP surface is restored. Source search finds no
`ETHTOOL_MSG_DEBUG_GET`, `ETHTOOL_A_DEBUG_*`, `ETHTOOL_A_BITSET_VALUE`,
`debug_msg_mask`, or `debug_msg_masks` in the netlink crate. The only live
`ETHTOOL_A_BITSET_BIT_VALUE` definition is the test-local tag and its retained
parser use.

## Review gates

| Gate | Result | Iteration-39 evidence |
|---|---|---|
| Linux UAPI value | PASS | Vendored enum ordinal and `ethtool 0.2.9` both define BIT_VALUE as 3. |
| Compilation scope | PASS | Constant is inside `#[cfg(test)] mod tests`; no production symbol is added. |
| Retained subject | PASS | FEATURES_SET encoder and TX-offload payload property remain live. |
| Parser body | PRESERVED | Existing nested-attribute walk is unchanged. |
| Assertion body | PRESERVED | Existing exact-name and no-BIT_VALUE assertions are unchanged. |
| Debug-mask deletion | PRESERVED | No removed debug command, decoder, API, test, or TAP surface returns. |
| Surface discipline | PASS | One private test constant only; no API, seam, scenario, or production behavior. |

## Mechanical verification

| Check | Result |
|---|---|
| Vendored UAPI inspection | PASS — VALUE follows UNSPEC/INDEX/NAME and equals 3. |
| Dependency decoder inspection | PASS — `ethtool 0.2.9` pins VALUE to 3. |
| Removed-debug reference search | PASS within `overdrive-netlink`. |
| `rustfmt --edition 2024 --check` | PASS for `ethtool.rs`. |
| Scoped `git diff --check` | PASS for `ethtool.rs`, handoff, and this review artifact. |
| Test/Metal/mutation execution | NOT RUN by this bounded static reviewer. |

## Iteration-39 verdict

**APPROVED.** The fixture-only constant restores the exact Linux UAPI tag
needed by the retained TX-offload payload parser. The no-BIT_VALUE property is
unchanged, and no deleted TAP debug-mask production or test surface is
reintroduced.

# APPROVED — SUPPLEMENTAL ITERATION 39

---

# Supplemental review iteration 40 — supplied-owner entrypoint caller fallout

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-40` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Three existing fixture callers of the approved five-argument supplied-owner entrypoint |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

The approved supplied-owner contract now has exactly five inputs:

```text
(config, boot, driver, shared_guest_network, guest_network_exec)
```

The retired sixth `vm_cgroups: CgroupManager` input belonged to the deleted
supervisor-wide VM-kill capability. This iteration reviews only the mechanical
caller fallout in the Sim boot-order invariant, the VM-reclamation invariant,
and the reusable direct-handler conformance harness.

## Exact caller mapping

Each reviewed call removes only its trailing construction of:

```rust
CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SimCgroupFs::new()))
```

The five retained arguments are the same values in the same order:

- the existing `ServerConfig`;
- the already-composed `BootAdapters` bundle;
- the same driver or driver trait object;
- the same supplied shared guest-network owner; and
- the same `GuestNetworkExecWiring` instance.

No caller substitutes a default owner, reconstructs wiring, changes driver
identity, or selects another server entrypoint. The source now compiles
against the exact accepted signature rather than preserving a compatibility
overload.

## Fixture and lifecycle preservation

The removed `CgroupManager` was a neutral argument constructed solely at the
call expression. Its instance was no longer part of the accepted supplied
owner boundary and no fixture read it back or asserted on it.

The real fixture cgroup adapters remain in their existing ownership path:

- `netns_density_boot_order` still builds its host boot adapters with
  `SimCgroupFs`, and still uses `PathBuf` for VMM clone-path evidence;
- `vm_reclamation` still builds the host bundle with `SimCgroupFs`, retains
  its `PathBuf` clone/staging facts, and drives the same VM host/reclamation
  state; and
- `DirectHandlerHarness` still receives the boot bundle produced with the
  simulated host ports, including its simulated cgroup filesystem.

Thus per-allocation VM start, stop, artifact disposal, and reclamation retain
their existing cgroup lifecycle. Only the deleted supervisor capability's
standalone duplicate argument disappears.

The two imports removed are exactly the now-unused `CgroupManager` imports in
the two Sim invariant modules. `SimCgroupFs` and `PathBuf` remain imported and
used at their independent VM-lifecycle sites. The conformance harness used
fully qualified names for the removed construction, so it needs no import
deletion.

## Assertions and documentation

All boot-order, sweep-order, EXEC-gate, VMM replacement, lease, row, timeout,
shutdown, and cleanup assertions remain unchanged. The alignment neither
removes a lifecycle observation nor changes a seed, adapter, wait, or expected
outcome.

The conformance harness prose now names the current five-argument function and
correctly attributes simulated host behavior to the boot bundle. It does not
change the harness boundary or add implementation detail to a product flow.

The concurrent activation of the VM-reclamation test marker is crafter-owned
DELIVER work and is not attributed to this mechanical argument alignment; its
body and oracles are unchanged.

## Review gates

| Gate | Result | Iteration-40 evidence |
|---|---|---|
| Signature compliance | PASS | All three reviewed calls pass the exact five approved arguments. |
| Argument identity | PRESERVED | Config, boot, driver, owner, and wiring values are unchanged. |
| Removed parameter | PASS | Only the retired trailing `CgroupManager` construction is removed. |
| Compatibility surface | PASS | No overload, dummy parameter, or replacement seam is introduced. |
| Cgroup lifecycle | PRESERVED | Each fixture's boot/VM path still owns its existing `SimCgroupFs` adapter. |
| Path evidence | PRESERVED | Independent `PathBuf` uses remain in both Sim invariants and the harness. |
| Imports | PASS | Only two argument-only `CgroupManager` imports are removed. |
| Assertions | PRESERVED | No test body oracle, seed, wait, or lifecycle observation changes. |
| Harness prose | PASS | Documentation matches the current signature and boot-bundle ownership. |
| Attribution | PASS | Concurrent marker activation remains crafter-owned. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped source diff | PASS — three call expressions, two unused imports, and harness signature prose. |
| Retained adapter search | PASS — `SimCgroupFs` and `PathBuf` remain at VM-lifecycle/boot-bundle sites. |
| `rustfmt --edition 2024 --check` | PASS for all three reviewed files. |
| Scoped `git diff --check` | PASS for the three files, handoff, and this review artifact. |
| Test/Metal/mutation execution | NOT RUN by this bounded mechanical review. |

## Iteration-40 verdict

**APPROVED.** The three fixture callers now match the exact five-argument
supplied-owner contract. Removing the obsolete standalone cgroup manager does
not alter driver, owner, wiring, boot, cgroup lifecycle, reclamation, or any
test oracle, and no replacement surface is introduced.

# APPROVED — SUPPLEMENTAL ITERATION 40

---

# Supplemental review iteration 41 — final five-argument caller fallout

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-41` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Five existing VM-lifecycle latency spike callers and one resulting unused integration-test import |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This supplement reviews the final neutral caller fallout from the accepted
five-argument `run_server_with_obs_and_driver` contract. It covers only the
five call expressions in `vm_lifecycle_latency_283_spike.rs` and removal of the
now-unused `provision_workload_netns` import in `alloc_netns_lifecycle.rs`.
Concurrent marker activations, attachment-cut fixture changes, and obsolete
TAP scenario deletions remain outside this iteration and are not attributed to
this caller alignment.

## Exact caller alignment

Each of the five spike callsites removes only the retired trailing construction
of:

```rust
CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SimCgroupFs::new()))
```

Every call retains the same five values in the same order:

```text
(config, boot, driver, shared_guest_network, guest_network_exec)
```

The callsites still await the returned server handle and unwrap the same
result. They do not select another entrypoint, insert a compatibility default,
replace an owner, reconstruct the existing wiring, or add a test seam.

A repository-wide syntax-aware inventory found 19 actual Rust invocations of
`run_server_with_obs_and_driver`; every invocation now passes exactly five
arguments. The public function definition also exposes exactly the accepted
five inputs. No six-argument invocation, overload, or dummy replacement
remains.

## VM-lifecycle evidence preservation

The spike still constructs each `BootAdapters` bundle through `sim_boot`, and
`sim_boot` still passes `Arc::new(SimCgroupFs::new())` to
`NodeAdapters::host`. The simulated cgroup filesystem therefore remains on the
real boot and VM-lifecycle ownership path. Removing the retired standalone
server argument does not remove cgroup behavior from the fixture.

The five call edits leave the existing configurations, clocks, observation
stores, drivers, supplied shared guest-network owners, execution wiring,
latency stimuli, seeds, waits, capacity boundaries, event capture, cleanup,
and assertions unchanged. The concurrent removal of pending DELIVER markers is
crafter-owned activation work; this mechanical review does not claim or
reassess it.

## Integration import cleanup

The `alloc_netns_lifecycle.rs` edit removes only
`provision_workload_netns` from its import list after the accepted obsolete TAP
cells and their last direct provisioning caller were deleted. The retained
structural workload-netns tests still use `WorkloadNetnsPlan`,
`derive_workload_netns_plan`, `responder_addr_for_slot`, and
`teardown_workload_netns`. Their provisioning-double behavior, failure-row
oracles, stable-success ownership, terminal cleanup, and slot release remain
unchanged.

The larger attachment-cut diff in that file was reviewed in earlier
supplements. This iteration treats it only as context establishing that the
removed import has no remaining caller; it does not reattribute those scenario
deletions to this compiler cleanup.

## Review gates

| Gate | Result | Iteration-41 evidence |
|---|---|---|
| Signature compliance | PASS | All five spike calls pass the exact accepted five arguments. |
| Repository caller inventory | PASS | All 19 actual Rust invocations have arity five. |
| Argument identity and order | PRESERVED | Config, boot, driver, supplied owner, and wiring remain unchanged at every reviewed call. |
| Compatibility surface | PASS | No overload, dummy argument, alternate entrypoint, API, or seam is introduced. |
| Sim cgroup lifecycle | PRESERVED | `sim_boot` still installs `SimCgroupFs` in `NodeAdapters::host`. |
| Spike behavior | PRESERVED | Seeds, stimuli, observations, waits, assertions, cleanup, and outcomes are unchanged by the call edits. |
| Import deletion | PASS | `provision_workload_netns` has no retained caller in `alloc_netns_lifecycle.rs`; all retained netns symbols remain. |
| Attribution | PASS | Concurrent marker activations and obsolete TAP-cell deletions are excluded from this iteration. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped source diff | PASS — five trailing argument constructions and one unused import only within this iteration. |
| Global actual-invocation arity inventory | PASS — 19 of 19 calls have five arguments. |
| Retained adapter search | PASS — `SimCgroupFs` remains in the spike's `NodeAdapters::host` boot bundle. |
| Retained netns symbol search | PASS — plan derivation, responder address, and teardown remain used in `alloc_netns_lifecycle.rs`. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed source files. |
| Scoped `git diff --check` | PASS for both source files, the handoff, and this review artifact. |
| Test/Metal/formal/mutation execution | NOT RUN by this bounded mechanical review. |

## Iteration-41 verdict

**APPROVED.** The five VM-lifecycle latency spike callers now match the exact
five-argument supplied-owner entrypoint without changing any retained input or
oracle. Simulated cgroup behavior remains in the boot bundle, the integration
import removal is last-caller compiler hygiene, and no replacement API or test
surface is introduced.

# APPROVED — SUPPLEMENTAL ITERATION 41

---

# Supplemental review iteration 42 — final fixture-lint alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-42` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Five mechanical lint corrections in existing DISTILL fixture and test-support code |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

This supplement reviews only the final lint corrections recorded in the
handoff: one fulfilled-line-count expectation removal, two private binding
renames in the Forwarder conformance driver, one equivalent reverse iterator
operation, one blank-line deletion, and one rustdoc code-span correction. No
production contract, test scenario, assertion, ordering step, timeout, or
public surface is part of this alignment.

## Exact mechanical changes

### Boot-order helper expectation

`netns_density_boot_order::run_seed` no longer carries the stale
`#[expect(clippy::too_many_lines)]` attribute after earlier caller alignment
shortened the helper below the lint threshold. The function body, seeded
scenario, boot composition, recorded calls, sweep observations, and every
oracle remain unchanged by this removal. No `allow` replaces the fulfilled
expectation.

### Forwarder driver binding names

`FwdPairDriver::reopen_h` now gives its two distinct owned values distinct
private names:

- `open_effect` is the held `OpenForwarder` effect identifier released through
  the Sim I/O arbiter;
- `quiescence_hold` is the counted `QuiescenceHold` consumed by
  `owner.restore`.

Each binding is used exactly once at the same operation as before. The rename
does not change the repair join, clean audit, hold restoration, settlement, or
error propagation order, and it avoids describing two different ownership
roles with the same `held` spelling.

### Retiring-cleanup iterator

The existing next-fit oracle still filters the ordered provision journal for
its last assigned address. Its `slice::Iter` followed by `filter_map` is a
`DoubleEndedIterator`; replacing `.last()` with `.next_back()` therefore
selects the same final matching address while reading it from the back. The
filter, address increment, expected value, subsequent placement, and equality
assertion are unchanged. It neither skips a journal row nor relaxes the exact
next-fit outcome.

### Admission-refusal body and ethtool rustdoc

The admission-refusal fixture removes one blank line between subscribing to
the lifecycle channel and dispatching the existing restart action. This brings
the lint-counted body back to its threshold without moving or removing any
statement. Cleanup, owner-journal, driver, row, lifecycle, pool, and event
assertions remain in the same order.

The test-local ethtool constant's rustdoc renders `FEATURES_SET` as a code span.
Only Markdown delimiters changed; the UAPI constant, encoder property, and
no-`BIT_VALUE` oracle remain intact. No doc-markdown allowance was added.

## Review gates

| Gate | Result | Iteration-42 evidence |
|---|---|---|
| Lint-expectation discipline | PASS | The stale boot-order `expect` is deleted and no replacement allowance is introduced. |
| Private-name semantics | PRESERVED | The effect identifier and quiescence hold retain their original types, consumers, and order. |
| Iterator equivalence | PASS | `filter_map` remains unchanged and `next_back()` returns the same last matching journal address. |
| Admission-refusal behavior | PRESERVED | Only whitespace is removed; statements and all refusal/cleanup assertions remain ordered identically. |
| Rustdoc correction | PASS | `FEATURES_SET` is code-formatted with no source or oracle change. |
| Contract shape | PRESERVED | No test, fixture API, production API, model, seam, or scenario is added or weakened. |
| Attribution | PASS | Concurrent production work and test activations are excluded from this lint-only review. |

## Mechanical verification

| Check | Result |
|---|---|
| Targeted source inspection | PASS — each correction is confined to its stated attribute, private binding, iterator terminal, whitespace, or rustdoc token. |
| Binding-use inspection | PASS — `open_effect` and `quiescence_hold` each have one definition and one matching consumer. |
| Retained-oracle inspection | PASS — the retiring-cleanup filter, address mapping, and exact comparison remain present. |
| `rustfmt --edition 2024 --check` | PASS for all five reviewed Rust files. |
| Scoped `git diff --check` | PASS for the five files, handoff, and this review artifact. |
| Test/Clippy/Metal/formal/mutation execution | NOT RUN by this bounded static reviewer. |

## Iteration-42 verdict

**APPROVED.** All five corrections are mechanical lint alignment. They remove
no assertion or behavior, preserve the exact iterator and ownership outcomes,
add no lint suppression or surface, and leave concurrent implementation work
outside this review.

# APPROVED — SUPPLEMENTAL ITERATION 42

---

# Supplemental review iteration 43 — admission-refusal drain extraction

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-43` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL reviewer |
| Scope | Private fixture-helper extraction from one existing admission-refusal test body |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

The actual Metal Clippy result after iteration 42 still measured
`a_refused_restart_successor_still_cleans_up_its_predecessor_once` at 101 lines
against the 100-line threshold. That reproduced result supersedes iteration
42's narrower claim that deleting one blank line was sufficient. The earlier
blank-line edit was behaviorally neutral, but it did not close the lint.

This iteration reviews the implemented correction: the test's existing
lifecycle receiver drain is extracted into one private fixture helper,
`drain_lifecycle_allocations`. No lint allowance, test, production API, model,
port, or seam is added.

## Extraction equivalence

The helper receives the same mutable `Receiver<LifecycleEvent>` at the same
point in the test and returns the same ordered `Vec<AllocationId>` formerly
built inline. Its transition table is unchanged:

| Receiver outcome | Before extraction | After extraction |
|---|---|---|
| `Ok(event)` | Append `event.alloc_id` and continue | Append `event.alloc_id` and continue |
| `Empty` | Stop draining | Stop draining |
| `Closed` | Stop draining | Stop draining |
| `Lagged(missed)` | Panic with the exact missed-count diagnostic | Panic with the exact missed-count diagnostic |

The helper does not await, filter, sort, deduplicate, retry, or consume an
additional observation after termination. Receive order therefore remains the
broadcast channel's order. The caller retains the same `published` binding and
the same subsequent assertion that the refused successor allocation appears in
no lifecycle event.

All earlier assertions stay before the drain: exact typed refusal, sole
predecessor teardown, retire/teardown/release event order, no successor driver
start, no successor row, and no successor lifecycle occurrence. All pool
occupancy, lease absence, and held-set assertions stay after it. The extraction
does not move an observable operation across dispatch or any oracle.

## Lint and scope result

After rustfmt, the test spans 95 physical source lines from its function
declaration through its closing brace. The extracted helper is private to the
existing `admission_refusal_acceptance` test module and has one caller. It is a
fixture readability boundary over an already-authored observation loop, not a
new scenario or test-support API.

## Review gates

| Gate | Result | Iteration-43 evidence |
|---|---|---|
| Reproduced premise | PASS | The handoff records the actual Metal 101/100 result; iteration 42's threshold claim is superseded. |
| Receiver semantics | PRESERVED | `Ok`, `Empty`, `Closed`, and `Lagged` retain their exact handling. |
| Event order | PRESERVED | Events are appended in receive order with no filtering or reordering. |
| Refused-allocation oracle | PRESERVED | The caller still asserts the returned allocation list excludes the successor. |
| Surrounding assertions | PRESERVED | Typed refusal, cleanup order, rows, occurrences, occupancy, leases, and held-set checks are unchanged. |
| Lint discipline | PASS | The formatted test is 95 physical lines; no lint allow or expect is added. |
| Surface discipline | PASS | The one-caller helper is private fixture code; no new test, production API, model, port, or seam appears. |

## Mechanical verification

| Check | Result |
|---|---|
| Targeted source inspection | PASS — the former inline match arms and the helper's match arms are identical in outcome and order. |
| Formatted physical-line count | PASS — 95 lines including declaration and closing brace. |
| Helper caller inventory | PASS — one existing test calls the private helper. |
| `rustfmt --edition 2024 --check` | PASS for `action_shim/mod.rs`. |
| Scoped `git diff --check` | PASS for `action_shim/mod.rs`, the handoff, and this review artifact. |
| Test/Clippy/Metal/formal/mutation execution | NOT RUN by this bounded static reviewer. |

## Iteration-43 verdict

**APPROVED.** The private extraction closes the source-size condition without
changing lifecycle receiver behavior or any surrounding oracle. Iteration
42's blank-line sufficiency claim is superseded by the recorded Metal result;
there is no remaining review finding on the implemented helper correction.

# APPROVED — SUPPLEMENTAL ITERATION 43

---

# Supplemental review iteration 44 — `fwd_pair` abort/handoff source review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-44` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL/formal source reviewer |
| Scope | Approved T-5/T-6 `strmHeld` model replacement, canonical README mapping, and private host conformance driver |
| Verdict | **CHANGES_REQUESTED — blocking specification/source mismatch** |
| Open findings | 0 critical, 1 high, 0 medium |

This is a source review before any replacement formal execution. It compares
the current `fwd_pair.qnt`, `guest-flow-owner/README.md`, and private
`quint_fwd_pair.rs` driver with the exact contract approved in
`design-12-01-fwd-abort-model.md` and independent DESIGN/formal review
iteration 2. It does not approve evidence, roadmap validation, or execution.

## Correctly implemented parts

The ownership cut itself matches the approved design:

- `strmHeld` is present in `St`, `cur`, `commit`, and `InitWith`; ordinary
  record updates preserve it.
- `attachAndHandoff` sets it only for watch-mode guest attach when the host
  watch was live in the action pre-state and the existing attach result is
  `ok`.
- the host driver projects the fact at top level from
  `self.stream.is_some()`, including after owner death, without a shadow phase
  or inferred dial-table flag;
- dialer `Write`, `Fin`, `Read`, `HolderEnd`, and `ReadEof` require the holder
  fact, and `holder_end` separately refuses to touch the Sim socket without an
  actual stream;
- pending-dial and held-stream `Abort(h)` have the approved distinct socket,
  `strmAbort`, and guest-control effects;
- `GuestAbort(g)` is enabled by the pending message and guest context, selects
  its effect from the actual guest pair phase, and always clears the message;
- `recoveryClose` creates an owner-aborted stream only when a stream is held;
  and
- `HolderDrop` retains one action with the approved held-stream and live
  pending-dial branches, clears both current-holder facts only on object drop,
  and is disabled by the model when neither a stream nor live watch exists.

The latest driver correction also preserves the late-attach order after
pending-dial `HolderDrop`: the pending `JoinHandle` is aborted and reaped by
the drop action, a later guest attach still sends its real local control reply,
and the absence of a dial handle is accepted only when the pre-reply watch was
not live. No stream or phase is fabricated.

The formal inventory is otherwise bounded. Relative to the prior model there
are still 38 actions, 39 named runs, 9 temporal formulas, 36 `ConfStep`
variants, and 11 modules in the same order. `checks.toml` and
`distill/test-scenarios.md` are unchanged. The Rust additions remain inside a
`cfg(all(test, feature = "integration-tests"))` private module and add no
public or production surface.

## F44-01 — blocking `SPECIFICATION_AMBIGUITY`: a live watch's refused guest attach has incompatible post-states

**Severity:** High / blocking  
**Status:** Open  
**Affected source:** `fwd_pair.qnt::attachAndHandoff`,
`FwdPairDriver::attach_g`, and the approved refused-attach handoff clause

The approved reconciliation says that a refused watch-mode guest attach does
not hand out a `GuestStream`, and the current driver now correctly sends the
real `ControlMessage::Refused` instead of manufacturing `Paired`. However, the
model and driver do not agree on the rest of that same action's host post-state.

A concrete existing action sequence is sufficient:

```text
InitWith("watch") -> TcpRst("g", false) -> Attach("g")
```

This is enabled in the current design instance. `TcpRst("g", false)` leaves
the guest pair unattached with a pending TCP error while the host watch and
dial are live. `Attach("g")` then takes the existing `HalfEnded` refusal path.

In the model, `attachEff(cur, "g", ...)` changes only the guest side:
`attRes(g) = halfEnded`, `pr(g) = refused`, and `V_g` closes.
`attachAndHandoff` correctly leaves `strmHeld = false`, but it leaves the host
watch live, its record absent, and `V_h` open.

The private driver instead sends:

```rust
ControlMessage::Refused {
    flow,
    refusal: FlowRefusal::ApplicationRefused {
        errno: KernelErrno::ECONNREFUSED,
    },
}
```

and settles the real owner. Production `run_dial` returns
`GuestDialError::Refused { errno }`; its existing error cleanup aborts the
live watch, queues its `Detached` record, closes owner-held `V_h`, removes the
dial, and sends no reciprocal `Abort { flow }` for this guest-originated
refusal. The driver then consumes the actual error. Its next projection is
therefore host `pr(h) = ended`, one queued record, `V_h.sent = {R,S}`, and no
held stream. That cannot equal the model's live-watch/open-socket post-state.

This is not a hypothetical schedule or an execution-timing guess. The action
is admitted by the current guards, and the production cleanup is the direct
`ControlMessage::Refused` branch followed by `run_dial`'s unconditional error
cleanup. It is also not fixed by accepting any real dial `Err`: the mismatch is
in the projected host watch, record, and socket facts, not only the unprojected
dial error variant.

The approved design pins refusal as a no-handoff case but does not pin these
host-side effects inside the existing atomic `Attach("g")` transition. The
implementation cannot honestly preserve both the current model post-state and
the actual Refused control path. Adding the missing effects directly in
DISTILL would extend the exact reviewed formal transition contract.

### Required disposition

Return this bounded branch to DESIGN to pin, within the existing action set,
the exact `Attach("g")` post-state when guest attach is refused while the host
watch is live. The decision must cover the watch record/cause, `V_h` shutdown,
pending dial result, and absence of a reciprocal guest `Abort` message. It must
then be independently reviewed and folded into the model, README/canonical
mapping, and private driver. No new action, run, scenario, public API, seam, or
production change is implied or authorized by this finding.

Formal execution must not begin from the current source package: a typecheck
would not prove the transition correspondence, and randomized conformance can
reach the contradictory state.

## Other contract and scope checks

| Gate | Result | Iteration-44 evidence |
|---|---|---|
| Ownership fact | PASS | Top-level model fact maps only to the actual move-only stream object's presence. |
| Pending/held abort split | PASS | Pending cancellation closes owner-held `V_h`; held-stream abort leaves holder-owned `V_h` untouched. |
| Holder-only enabledness | PASS | Every approved stream call/observation requires `strmHeld`. |
| Late Paired after cancellation | PASS | Local guest attach remains possible and no host stream is manufactured. |
| Guest Abort phase selection | PASS | Host-holder ownership does not decide guest-pair ownership; every delivery clears the message. |
| Recovery | PASS | Holder-free recovery adds no stream fact, outcome, message, or socket effect. |
| HolderDrop | PASS | Actual stream drop and live pending-dial cancellation remain distinct; terminal handle disposal adds no model step. |
| Invariants | PASS except F44-01 reachability | `strmAbort => strmHeld`, held-stream recovery, and existing T-5/T-6 temporal obligations remain present. |
| Inventory | PASS | No action, run, temporal formula, `ConfStep` variant, module, check, or scenario was added or removed. |
| Surface | PASS | No public/production API, port, state, error, or seam changed. |
| Refused guest attach | **BLOCKED** | Model leaves the host watch live; the real Refused path ends it, queues a record, and closes `V_h`. |

## Mechanical verification

| Check | Result |
|---|---|
| Formal inventory comparison | PASS — action/run/temporal/variant/module names and order unchanged. |
| Check/scenario diff | PASS — `checks.toml` and `distill/test-scenarios.md` unchanged. |
| `rustfmt --edition 2024 --check` | PASS for the private host driver. |
| Scoped `git diff --check` | PASS for the spec, README, driver, handoff, and this review artifact. |
| Typecheck/exact-seed/named/randomized/job-3 record/verify-evidence | NOT RUN; blocked pending F44-01 design closure and a corrected source re-review. |

## Iteration-44 verdict

**CHANGES_REQUESTED.** The approved `strmHeld` ownership split and nearly all
of its existing-action closure are implemented exactly, with no inventory or
surface expansion. F44-01 blocks source approval because an existing,
reachable refused guest attach projects mutually incompatible host states in
the model and real driver. The full mandatory formal/evidence sequence remains
pending and must not start until the exact branch is designed, independently
approved, implemented, and re-reviewed.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 44

---

# Supplemental review iteration 45 — F44-01 refused-attach remediation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-45` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL/formal source reviewer |
| Scope | F44-01 remediation plus complete re-review of the approved `strmHeld` source package |
| Verdict | **APPROVED — SOURCE ONLY** |
| F44-01 disposition | **CLOSED** |
| Open findings | 0 critical, 0 high, 0 medium |

The authoritative feature delta resolves F44-01 as composition of two already
accepted contracts rather than a new design choice: EVMOD-ATTACH-END requires
the guest's ended-half response and pre-pair close, while the existing dial
outcome contract requires every `Refused` error to abort its watch, close
owner-held `V_h`, remove the dial, release its places, and return the typed
error. A guest-originated reciprocal `Refused` does not ask the host to send a
second `Abort { flow }`. This exact existing-contract disposition is now
implemented and documented.

## F44-01 remediation review

For watch-mode `Attach("g")`, `attachAndHandoff` now has three exhaustive
ownership outcomes selected from the action pre-state and existing attach
result:

| Pre-state/result | Model and driver outcome |
|---|---|
| Host watch live; guest attach `ok` | The real dial must return `Ok(GuestStream)` and `strmHeld` becomes true. |
| Host watch live; guest attach `halfEnded` | The real dial must return `GuestDialError::Refused { errno: ECONNREFUSED }`; the host watch ends with one queued `Detached` record, owner-held `V_h` closes, and no stream or reciprocal guest `Abort` appears. |
| Host watch already ended | A successful or refused late reply cannot create a stream; the dial's already-fixed error is preserved, or an already-reaped pending handle remains absent. |

The model's refused-live-watch branch starts from the existing guest refusal
post-state, then applies the existing host watch abort and socket-close
effects in the same `Attach`/`AttachHook` action. Its exact host facts are:

```text
pr(h) = ended / Detached / queued once
vs(h).sent = {R,S}
vs(h).sshut = true
strmHeld = false
strmAbort = false
hres = none
guestAbortSent = false
```

The guest side retains its existing `halfEnded`, `refused`, zero-record,
linger-close, and `V_g` shutdown outcome. The branch does not increment a new
fault or create another protocol step: the ended half was produced by the
existing fault action, and `run_dial` performs its Refused cleanup before the
same dial call returns.

The private driver takes `watch_live` before sending the reply, matching the
model's pre-state predicate. It sends the real `ApplicationRefused` control
message with `ECONNREFUSED`, settles the real owner cleanup, and accepts the
live refusal only when the actual dial future returns that exact typed error.
An unexpected error, missing live future, or returned stream is a conformance
failure. For a late reply after the host dial already ended, it accepts only
the prior fixed error or the already-authorized absence of a reaped pending
handle; it never synthesizes a stream or overwrites the earlier host result.

The reproduced F44-01 trace therefore has one post-state on both sides:

```text
InitWith("watch") -> TcpRst("g", false) -> Attach("g")
```

The guest refusal closes its half, the host receives typed Refused, its live
watch ends once with `Detached`, and its owner-held `V_h` closes. F44-01's
live-watch/open-socket model divergence is removed.

## Complete source-package re-review

All previously approved ownership-cut mappings remain exact:

- `strmHeld` is initialized and committed as an ordinary state fact and is
  projected only from `self.stream.is_some()` at the top-level host boundary,
  including after owner death.
- Pending-dial `Abort(h)` closes owner-held `V_h`, keeps `strmAbort` false,
  and retains the existing typed-error path; held-stream `Abort(h)` keeps
  holder-owned `V_h` untouched and sets `strmAbort`.
- Every watch-mode holder call or observation requires `strmHeld`; the driver
  refuses `HolderEnd` without the actual stream object.
- Late guest attach remains enabled after host cancellation and can create the
  local guest pair without recreating a host holder.
- `GuestAbort(g)` follows only the guest pair phase, aborting a live pair even
  without a host stream and discarding for watch-mode unattached, ended, or
  refused state while always clearing the message.
- Holder-free recovery creates no stream, message, outcome, or socket effect;
  held-stream recovery preserves `strmHeld` and an already reported `hres`
  while setting the existing owner-abort fact.
- `HolderDrop` still distinguishes actual stream drop from live pending-dial
  cancellation; it sets the stream-drop ghost only for the former, clears the
  two current-holder facts together, and has no model transition after both
  holder and watch are absent.
- `WatchAbortLeavesHolderSocket`, `strmAbort => strmHeld`,
  `NoLiveWatchAfterDrop`, `RecoveryEndsHeldStream`, and the existing T-5/T-6
  temporal obligations remain unchanged in strength.

There is no hidden inventory or surface expansion. The model still contains
38 actions, 39 named runs, 9 temporal formulas, 36 `ConfStep` variants, and 11
modules in the same order as the prior input. `checks.toml` and
`distill/test-scenarios.md` remain byte-unchanged. The one additional effect is
inside the existing atomic `Attach`/`AttachHook` transition. The Rust driver
remains private test support under
`cfg(all(test, feature = "integration-tests"))`; it adds no production/public
type, method, error, parameter, port, or seam.

## Review gates

| Gate | Result | Iteration-45 evidence |
|---|---|---|
| F44-01 existing-contract basis | PASS | Feature-delta disposition composes the already-pinned ended-half response and dial-error cleanup; no new choice is introduced. |
| Refused model post-state | PASS | Guest refusal and host `Detached`/queued/closed-`V_h` cleanup occur in the same existing attach action. |
| Refused driver result | PASS | A live refusal requires exact `GuestDialError::Refused { errno: ECONNREFUSED }`. |
| Reciprocal message rule | PASS | Refused cleanup leaves `guestAbortSent = false`, matching `run_dial`. |
| Successful handoff | PASS | Only a pre-state live watch plus successful attach plus actual returned stream creates `strmHeld`. |
| Late fixed result | PASS | Late reply preserves the earlier error or an already-reaped handle; it cannot produce a stream. |
| Pending/held abort split | PASS | R5-53 and R5-63/WATCH-ABORT remain distinct and exact. |
| Holder-only actions | PASS | All existing stream calls and observations require the actual holder fact. |
| Guest-pair terminal behavior | PASS | Both late-message orders remain enabled; actual guest phase selects abort versus discard. |
| Recovery and drop | PASS | Holder-free paths fabricate no holder state; actual holder paths retain T-5/T-6 obligations. |
| Inventory | PASS | No action, run, temporal, variant, module, check, scenario, or trace filter changed beyond existing action bodies. |
| Public/production surface | PASS | No API, error, port, mechanism, or production implementation changed. |

## Mechanical verification

| Check | Result |
|---|---|
| Formal inventory comparison | PASS — counts, names, and order unchanged. |
| Check/scenario diff | PASS — `checks.toml` and `distill/test-scenarios.md` unchanged. |
| Canonical/README/source correspondence | PASS — all three describe the same Refused cleanup and no reciprocal Abort. |
| `rustfmt --edition 2024 --check` | PASS for the private host driver. |
| Scoped `git diff --check` | PASS for the spec, README, driver, canonical delta, handoff, and this review artifact. |
| Typecheck/exact-seed/named/randomized/job-3 record/verify-evidence | NOT RUN by this source reviewer; all remain mandatory after this approval. |

## Evidence and roadmap disposition

This approval closes the source-review gate only. The historical 279-check
record predates the corrected formal input and is not current evidence for the
handoff contract. Before any final evidence or roadmap approval, the
orchestrator must still complete, in the required order:

1. full guest-flow-owner typecheck;
2. exact seed `0xea0ef1a4` replay without a filtered action or fabricated
   state;
3. every unchanged named run and randomized host/guest conformance gate;
4. the complete guest-flow-owner record with exactly three jobs;
5. `verify-evidence` and exact input/summary hash inspection; and
6. independent review of the replacement evidence.

## Iteration-45 verdict

**APPROVED — SOURCE ONLY.** F44-01 is closed. The Refused branch now composes
the exact existing guest refusal and host dial cleanup in the existing attach
transition, checks the real typed result, and leaves no reciprocal Abort or
fabricated stream. The complete `strmHeld` source package matches the accepted
T-5/T-6 contracts without weakening an invariant or adding an action,
scenario, filter, API, seam, state mechanism, or production change. Formal
execution and evidence approval remain outstanding mandatory gates.

# APPROVED — SUPPLEMENTAL ITERATION 45 (SOURCE ONLY)

---

# Supplemental review iteration 46 — dead-owner guest-attach driver boundary

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-46` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL/formal source reviewer |
| Scope | Trace-18 private-driver correction and compiler-required Quint local rename |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The corrected exact-seed replay advanced to Trace 18:

```text
InitWith("watch")
  -> CtxRelease("h")
  -> Write("gh")
  -> Attach("g")
```

`CtxRelease("h")` selects the existing host-owner-death branch because the
host watch is live. The real `exit_process` destroys the host process's
control-session record, dial task, watch, context, and other owned handles.
The model makes subsequent host-owner actions impossible, while still allowing
the guest's local write and attach before the guest observes the lost session.
The former driver tried to deliver that local guest reply through the destroyed
host record and panicked before comparing the modeled state.

## Driver correction

`attach_g` now separates the guest-local stimulus from delivery to a living
host owner:

- `owner_dead == false` keeps the full existing path: inspect the actual watch,
  send the real `Paired` or `Refused` reply through the accepted control
  record, settle production handling, and validate the real dial result;
- `owner_dead == true` performs no host control I/O and does not read a host
  owner projection, but still applies the existing local guest attach or
  refusal state transition.

The branch uses the existing `owner_dead` process-lifetime fact set by the
modeled `CtxRelease("h")` action. It does not introduce a second session flag,
reconstruct the destroyed record, infer death from an action candidate, or
filter `Attach("g")` from the trace.

The `watch_live` expression short-circuits on `owner_dead`, so
`forwarder_inventory()` is not called through the dead host seam. `flow` is
the already-retained fixture identity, and the final local guest transition
uses only the driver's existing guest stimulus state. The ordinary settle
after that local transition preserves retained Sim socket facts and returns
early from owner-id discovery because the owner is dead.

For a living owner, iteration 45's exact checks are unchanged. A successful
live handoff must return the actual `GuestStream`; a live refusal must return
`GuestDialError::Refused { errno: ECONNREFUSED }`; a late reply after a living
owner already ended the dial preserves its prior fixed error; and a missing
future is accepted only where no live handoff or live refusal is expected.

The correction therefore adds no synthetic host state. After owner death the
host projection remains absent, the released context and retained socket facts
remain the sole host evidence, and `strmHeld` remains the actual
`self.stream.is_some()` fact. Only the model-authorized guest-local state is
advanced.

## Quint local-name correction

The first source execution exposed QNT101 because the helper-local name `next`
shadowed Quint's built-in `next`. The local is now named `attached`. Its
initializer is still the same `attachEff(t, s, ps)` result, and every guard,
spread update, refusal cleanup, and return references that same value. This is
an alpha rename only; no state, transition, enabledness condition, or outcome
changed.

The author-reported typecheck advanced past this identifier error and exposed
Trace 18. That partial execution is reproduction evidence for this correction,
not completion of the formal gate.

## Review gates

| Gate | Result | Iteration-46 evidence |
|---|---|---|
| Owner-death boundary | PASS | No host control or owner observation is attempted after `owner_dead`. |
| Guest-local transition | PRESERVED | Existing local attach/refusal still occurs before observed session loss. |
| No action filtering | PASS | `Attach("g")` remains enabled and applied; only impossible delivery to the destroyed host record is omitted. |
| No host recreation | PASS | The driver neither replaces nor reopens the destroyed connection, owner, dial, watch, or context. |
| Living successful handoff | PRESERVED | Real `Ok(GuestStream)` remains required. |
| Living Refused cleanup | PRESERVED | Exact typed Refused result and host cleanup from iteration 45 remain required. |
| Living late result | PRESERVED | Existing fixed-error and reaped-handle rules are unchanged. |
| Projection boundary | PASS | Host owner remains absent after death; retained S/C facts and actual stream presence remain the only compared host facts. |
| Quint rename | PASS | `next` to `attached` changes only the private local identifier. |
| Inventory and surface | PASS | No model guard, action, run, scenario, check, state fact, API, seam, or production mechanism is added or removed. |

## Mechanical verification

| Check | Result |
|---|---|
| Formal inventory comparison | PASS — 38 actions, 39 runs, 9 temporals, 36 variants, and 11 modules remain in the same order. |
| Reserved-local search | PASS — no `val next =` remains in `fwd_pair.qnt`. |
| Check/scenario diff | PASS — `checks.toml` and `distill/test-scenarios.md` unchanged. |
| `rustfmt --edition 2024 --check` | PASS for the private host driver. |
| Scoped `git diff --check` | PASS for the spec, driver, README/canonical mapping, handoff, and this review artifact. |
| Full typecheck/exact-seed/named/randomized/job-3 record/verify-evidence | PENDING; not run by this source reviewer. |

## Iteration-46 verdict

**APPROVED — SOURCE ONLY.** Trace 18 now preserves the existing boundary:
guest-local attachment may advance briefly after host owner death, while no
message is delivered through a destroyed host record and no host state is
recreated or synthesized. Living success, refusal, and late-error behavior is
unchanged. The Quint edit is an alpha rename required by its compiler. All
formal runs, replacement evidence, hash verification, and evidence review
remain outstanding mandatory gates.

# APPROVED — SUPPLEMENTAL ITERATION 46 (SOURCE ONLY)

---

# Supplemental review iteration 47 — Trace-14 recovery source review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-47` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL/formal source reviewer |
| Scope | DESIGN review-iteration-3 holder-free `recoveryClose` replacement and canonical fold |
| Verdict | **CHANGES_REQUESTED — exact selector is broader than approved** |
| Open findings | 0 critical, 1 high, 0 medium |

The new `recoveryClose` effect is correct for reproduced Trace 14. It follows
the retained non-watch queued-record branch and held-stream branch, uses the
existing `vs(h).sent` fact to detect incomplete cleanup, applies idempotent
`shutAll(h)`, keeps `strmAbort = false` and `hres = none`, preserves an
existing guest-Abort fact, and adds that fact only when cleanup was pending
and the VM remained live. Because it spreads the prior state, it leaves the
ended watch's record, cause, count, and classification untouched.

One exact selector clause from the approved review is missing, however. The
source and canonical fold select every watch-mode state with `strmHeld =
false` and an ended watch. DESIGN review iteration 3 approves this cleanup
only while the dialer endpoint is still alive; it expressly states that
`ep(h) = dead` after `HolderDrop` excludes the branch.

## F47-01 — blocking design divergence: holder-free recovery omits the alive-dialer guard

**Severity:** High / blocking  
**Status:** Open  
**Affected source:** `fwd_pair.qnt::recoveryClose`, the README recovery
mapping, and the canonical `CtxRelease`/T-5 fold

The implemented selector is:

```quint
watchSide(t, "h") and not(t.strmHeld) and p.ph == "ended"
```

The exact independently approved selector also requires:

```quint
t.ep.get("h") == "alive"
```

This is semantically material even when `V_h` is already closed. A current
`pair_conf` sequence can extend the existing watched-stream path as follows:

```text
WatchedStreamSeesGuestResetAsAbort
  -> HolderDrop
  -> CtxRelease("h")
```

Before `HolderDrop`, the real stream has reported `GuestAbort`, so `hres =
GuestAbort`. Dropping the actual stream closes `V_h`, clears `strmHeld` and
`strmAbort`, sets `ep(h) = dead`, and deliberately preserves the already
reported `hres`. With no live watch, the later context release selects
Forwarder recovery. Production has no pending dial or stream cleanup left and
the private driver retains `self.hres = GuestAbort`.

The current broader model branch still runs because it does not test
`ep(h)`. `cleanupPending` is false and `shutAll` is idempotent, but the branch
unconditionally overwrites `hres` with `none`. The next conformance projection
therefore disagrees with the real driver and erases an accepted previously
reported stream outcome. The same broad selector also contradicts review
iteration 3's explicit statement that a completed `HolderDrop` cannot be
treated as pending-dial cleanup.

This path is production-reachable and admitted by `pair_conf`'s fault budget:
a held stream can report its terminal result and be dropped, followed later by
an independent Forwarder recovery. It is not an invented candidate state.

### Required bounded remediation

Add the already-approved alive-dialer predicate to the new holder-free branch:

```quint
else if (
  watchSide(t, "h")
    and not(t.strmHeld)
    and p.ph == "ended"
    and t.ep.get("h") == "alive"
)
```

Update the README and canonical fold to state the same selector. No DESIGN
choice, driver change, state, action, run, scenario, hazard, API, seam, or
production mechanism is required. The approved Trace-14 state has
`ep(h) = alive`, so this correction retains its exact close. The retained
held-stream and non-watch branches already have correct ordering and must stay
unchanged.

Formal execution must remain blocked until the selector and canonical text
match the approved review-iteration-3 contract and receive source re-review.

## Other source checks

| Gate | Result | Iteration-47 evidence |
|---|---|---|
| Trace-14 effect | PASS | Pending owner-held `V_h` closes idempotently; no message is emitted after VM loss. |
| Branch order | PASS | Non-watch queued-record handling remains first and held-stream recovery remains before the new holder-free branch. |
| Record preservation | PASS | `pr(h)` record, cause, queue state, and count are spread unchanged. |
| Held-stream recovery | PRESERVED | It sets `strmAbort`, keeps `strmHeld`, preserves `hres`, and does not close holder-owned `V_h`. |
| Existing guest Abort | PRESERVED | The holder-free update ORs the existing fact and adds it only for pending cleanup with `vmUp`. |
| Idempotent close | PASS | `cleanupPending` reads `sent != both`; `shutAll` retains the existing no-op-on-closed behavior. |
| HolderDrop exclusion | **BLOCKED** | Missing `ep(h) = alive` admits the completed-holder path and clears its reported `hres`. |
| Inventory and surface | PASS | No state, action, run, temporal, variant, module, check, scenario, API, seam, or mechanism was added. |

## Mechanical verification

| Check | Result |
|---|---|
| Formal inventory comparison | PASS — 38 actions, 39 runs, 9 temporals, 36 variants, and 11 modules remain in the same order. |
| Approved-selector inspection | **FAIL** — holder-free ended-watch branch has no `ep(h) = alive` guard. |
| `rustfmt --edition 2024 --check` | PASS for the unchanged private host driver. |
| Scoped `git diff --check` | PASS for the spec, README, canonical/design artifacts, handoff, and this review artifact. |
| Both exact seeds/unfiltered-40/typecheck/job-3 record/verify-evidence | NOT RUN; blocked pending F47-01 remediation and re-review. |

## Iteration-47 verdict

**CHANGES_REQUESTED.** The Trace-14 effect and its ordering are correct, but
the holder-free branch is broader than DESIGN review iteration 3 approved.
Without the alive-dialer guard it reclassifies a completed actual-stream drop
as pending-dial cleanup and erases a retained `hres`. F47-01 is the only open
finding. All formal execution and replacement-evidence gates remain pending.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 47

---

# Supplemental review iteration 48 — F47-01 alive-dialer selector correction

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-48` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL/formal source reviewer |
| Scope | Exact F47-01 selector correction in holder-free `recoveryClose` |
| Verdict | **APPROVED — SOURCE ONLY** |
| F47-01 disposition | **CLOSED** |
| Open findings | 0 critical, 0 high, 0 medium |

The holder-free recovery branch now has the complete selector independently
approved in DESIGN review iteration 3:

```quint
watchSide(t, "h")
  and not(t.strmHeld)
  and p.ph == "ended"
  and t.ep.get("h") == "alive"
```

This is the only transition-semantic change since iteration 47. The retained
non-watch queued-record branch remains first, and the retained held-stream
branch remains second. The new holder-free branch therefore applies only after
those meanings have been excluded and only while the dialer endpoint still
represents a pending dial rather than a dropped former holder.

## Exact boundary review

The reproduced Trace-14 pre-state still satisfies the branch: its watch is
ended, no `GuestStream` was handed out, and the dialer endpoint is alive while
the pending dial still owns open `V_h`. Recovery records whether socket cleanup
was pending, applies existing idempotent `shutAll(h)`, preserves the queued
`SocketError` record and count, keeps `strmAbort = false` and `hres = none`,
and emits no guest Abort because the VM is gone.

A completed actual-stream `HolderDrop` no longer enters this branch. That
transition sets `ep(h) = dead`, closes `V_h`, clears `strmHeld` and
`strmAbort`, and preserves any already-reported `hres`. A later Forwarder
recovery now falls through without treating the former holder as pending dial
cleanup, so the driver and model retain the same `HalfClose`, `GuestAbort`, or
`OwnerAbort` result.

The README now names this alive-endpoint boundary and the dead-endpoint
`HolderDrop` complement explicitly. The authoritative feature delta's
“owner-held `V_h`” wording is read with the exact review-iteration-3 selector:
an endpoint made dead by actual holder drop is no longer the owner-held
pending-dial socket addressed by this recovery clause.

## Effect and scope preservation

Every approved effect remains unchanged:

- `cleanupPending` is still exactly `vs(h).sent != {R,S}`;
- `shutAll` still makes an incomplete close exact and an existing close a
  no-op;
- `pr(h)` remains spread unchanged, preserving outcome, cause, queued/read
  status, record count, release classification, and context generation;
- `strmHeld` remains false, `strmAbort` remains false, and `hres` remains none
  only in the pending-dial branch;
- `guestAbortSent` preserves an existing message and adds one only when the
  close was newly required while `vmUp` remained true;
- VM-gone cleanup adds no guest message;
- held-stream recovery still sets `strmAbort`, preserves `strmHeld` and any
  prior `hres`, and leaves holder-owned `V_h` untouched; and
- non-watch queued-record cleanup remains byte-for-byte in its earlier branch.

No model state, action, run, temporal property, `ConfStep` variant, module,
check, scenario, hazard, driver code, public API, seam, or production mechanism
was added or removed.

## Review gates

| Gate | Result | Iteration-48 evidence |
|---|---|---|
| Approved selector | PASS | Watch mode, no held stream, ended watch, and alive dialer are all required. |
| Trace-14 selection | PASS | The raw pre-state has `ep(h) = alive`, so required pending cleanup remains selected. |
| Completed HolderDrop exclusion | PASS | `ep(h) = dead` falls through and preserves the reported stream result. |
| Branch order | PASS | Non-watch and held-stream meanings retain precedence. |
| Record preservation | PASS | The holder-free branch does not rewrite or consume `pr(h)`. |
| Idempotent socket close | PASS | `cleanupPending` and `shutAll` are unchanged. |
| Guest message condition | PASS | Existing OR, cleanup-pending, and `vmUp` conditions are unchanged. |
| Held-stream outcome | PRESERVED | `V_h`, `strmHeld`, `strmAbort`, and existing `hres` retain the approved U-R1 behavior. |
| Inventory and surface | PASS | No action, run, scenario, check, state, driver, API, seam, or mechanism changed. |

## Mechanical verification

| Check | Result |
|---|---|
| Exact-selector search | PASS — the approved four-clause selector appears once. |
| Formal inventory comparison | PASS — 38 actions, 39 runs, 9 temporals, 36 variants, and 11 modules remain in the same order. |
| `rustfmt --edition 2024 --check` | PASS for the unchanged private host driver. |
| Scoped `git diff --check` | PASS for the spec, README/canonical mapping, handoff, and this review artifact. |
| Both exact seeds/unfiltered-40/typecheck/job-3 record/verify-evidence | PENDING; not run by this source reviewer. |

## Iteration-48 verdict

**APPROVED — SOURCE ONLY.** F47-01 is closed. Holder-free recovery now applies
only to the alive pending-dial endpoint approved by DESIGN iteration 3. Trace
14 retains its exact cleanup, completed holder drops retain their reported
stream outcome, and no other transition or surface changed. Both exact seeds,
the unchanged 40-cell conformance gate, full job-3 record, evidence hashes,
and independent proof review remain mandatory and pending.

# APPROVED — SUPPLEMENTAL ITERATION 48 (SOURCE ONLY)

---

# Supplemental review iteration 49 — Trace-47 `TakeEnd` cleanup source review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-49` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL/formal source reviewer |
| Scope | DESIGN review-iteration-4 `TakeEnd(h)` pending-dial cleanup amendment |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The current `TakeEnd` implementation matches the exact Trace-47 amendment
approved in DESIGN review iteration 4. It keeps the existing record-read guard,
performs the existing queued-to-read transition first, then applies the
already-approved pending-dial cleanup only for the precise pre-state owner and
ownership boundary.

## Exact selector and ordering

The cleanup selector is:

```quint
s == "h"
  and watchSide(cur, "h")
  and not(cur.strmHeld)
  and cur.ep.get("h") == "alive"
```

`canTakeEnd(cur, s)` remains the action's first guard. It already requires the
record to be queued in the current open context at the flow's current
context-generation. `t1` then changes only `pr(s).rec` from `queued` to `read`.
The pending-dial cleanup is applied to that post-read state while its ownership
selector and `cleanupPending` decision use the action pre-state, exactly as the
approved transition requires.

This ordering preserves both owners of the effect:

1. the forwarder/owner record consumer fixes and records the terminal outcome;
2. cancellation of the holder-free `AwaitingReply` dial closes the `V_h` that
   the dial still owns.

The transition therefore cannot close a socket before the terminal record is
observed, and it cannot rewrite the record from the cleanup result.

## Effects and exclusions

Within the selected host-watch branch:

- `cleanupPending` is the pre-state predicate `vs(h).sent != {R,S}`;
- `shutAll(t1, "h")` is idempotent and changes only the existing socket-close
  complement when needed;
- `strmAbort` remains false and `hres` remains none because no stream was handed
  to a caller;
- `guestAbortSent` preserves an existing fact and becomes newly true only when
  close cleanup was pending and `vmUp` remained true; and
- the post-read `pr(h)` retains its exact `out`, `cause`, `errno`, `nrec`,
  closure/release facts, and context generation.

All approved exclusions are present:

- `strmHeld = true` skips cleanup, so consuming a held stream's watch record
  does not close holder-owned `V_h`;
- `ep(h) = dead` skips cleanup, so completed holder or pending-dial disposal is
  not repeated and any reported `hres` remains intact;
- non-watch host records retain their pre-existing same-dispatch `closeEff`;
- guest-side `TakeEnd` retains the existing record-read-only effect;
- a record in a released or older context remains ineligible through the
  unchanged `canTakeEnd` guard; and
- recovery and `HolderDrop` branches are not modified.

The final commit expression applies `closeEff` only to the same host non-watch
case as before. The watch amendment and ordinary pair cleanup therefore cannot
both run in one action.

## Contract and scope audit

Trace 47's exact state is selected: the host watch is ended with one queued
`SocketError` record, the actual stream is absent, the dialer endpoint is
alive, and owner-held `V_h` has not completed its local close. The new action
reads the record, preserves `SocketError`/errno/count, closes `V_h`, and emits
no guest message because the VM is gone. This matches production
`process_end` followed by the cancelled `run_dial` cleanup.

The same selector is cause-neutral, as approved. Every terminal record that
ends a holder-free `AwaitingReply` dial requires the common socket cleanup;
the typed dial outcome remains owned by `owner_flows` and production
`process_end`. No model error or cause is invented here.

No state, action, named run, temporal property, `ConfStep` variant, module,
check, scenario, hazard, driver, API, seam, or production mechanism changed.
The README, feature-delta action map/T-5 clause, and handoff state the same
record-first ordering and held/dead/guest/non-watch exclusions.

## Review gates

| Gate | Result | Iteration-49 evidence |
|---|---|---|
| Existing action guard | PRESERVED | `canTakeEnd` remains first and unchanged. |
| Record-first order | PASS | `rec = read` is written before cleanup is applied. |
| Exact selector | PASS | Host + watch + no held stream + alive dialer are all required from the pre-state. |
| Cleanup predicate | PASS | Pre-state `vs(h).sent != both` alone decides whether close/message cleanup is new. |
| Record outcome | PRESERVED | Outcome, cause, errno, and record count pass through unchanged. |
| Held stream | PRESERVED | No close or holder-result rewrite occurs. |
| Dead endpoint | PRESERVED | No completed disposal is repeated; prior `hres` remains. |
| Non-watch host pair | PRESERVED | Existing `closeEff` remains the sole final branch. |
| Guest record | PRESERVED | Existing read-only behavior remains. |
| Recovery/HolderDrop | PRESERVED | Guards and effects are unchanged. |
| Inventory and surface | PASS | No formal inventory, driver, public surface, seam, or mechanism changed. |

## Mechanical verification

| Check | Result |
|---|---|
| Formal inventory comparison | PASS — 38 actions, 39 runs, 9 temporals, 36 variants, and 11 modules remain in the same order. |
| Check/scenario inventory | PASS — no check or scenario change accompanies the existing-action amendment. |
| `rustfmt --edition 2024 --check` | PASS for the unchanged private host driver. |
| Scoped `git diff --check` | PASS for the spec, README/canonical/design artifacts, handoff, and this review artifact. |
| Both exact seeds/unfiltered-40/typecheck/job-3 record/verify-evidence | PENDING; not run by this source reviewer. |

## Iteration-49 verdict

**APPROVED — SOURCE ONLY.** The Trace-47 amendment uses the exact approved
pre-state selector, reads the terminal record before completing pending-dial
socket cleanup, and preserves its outcome, cause, errno, and count. Held
streams, dead endpoints, guest records, non-watch pairs, recovery, and
`HolderDrop` remain unchanged. Both exact-seed replays, the unchanged 40-cell
conformance gate, full job-3 record, evidence hashes, and independent proof
review remain mandatory and pending.

# APPROVED — SUPPLEMENTAL ITERATION 49 (SOURCE ONLY)

---

# Supplemental review iteration 50 — broad-gate fixture alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-50` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL source reviewer |
| Scope | Four bounded existing-test/support corrections from the current broad default gate |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

This iteration reviews only the bounded DISTILL-owned alignments identified by
the broad-gate failure grouping: valid positive-property input domains, six
fixture composition roots, fail-stop request ownership, and composed-process
teardown. It does not approve or waive any remaining production, conformance,
capacity, or execution-environment failure.

## Positive property input domains

The eight observed positive-property failures generated services that the
accepted ingress contract correctly refuses. The updated strategies now keep
those properties inside the valid-service domain:

- listener strategies generate every nonzero TCP/UDP port outside
  `guest_reserved_ports()`; both valid intervals remain represented;
- probe-target-only strategies retain the full nonzero port domain because the
  guest reservation constrains listeners, not probe destinations; and
- three-role probe-vector strategies retain their existing per-role ranges but
  require the combined count to be at most `GUEST_PROBE_SLOTS`.

For the positional-index property, the prior role ranges have maximum total
`3 + 3 + 3`; the filter therefore removes only that invalid nine-probe
combination. It retains zero-role cases, more than one probe per role, every
valid role-count shape, role-local index order, and global `(role, idx)`
uniqueness. The persistence strategies likewise retain arbitrary probe
mechanics/fields and every valid empty/non-empty per-role shape up to the same
production cap.

Listener protocol generation, list order, service multiplicity, and
`(port, protocol)` distinctness are unchanged. The dedicated deploy-validation
properties remain byte-unchanged and continue to prove the complete reserved
listener interval, both immediate valid boundaries, acceptance at eight total
probes, and typed refusal above the cap. No negative case is converted into a
positive oracle or silently dropped from the suite.

## Six `AppState` fixture builders

Six async fixture builders previously created a fresh shared guest-network
owner and immediately passed another fresh or unconverged instance to the
fixture convenience constructor. Production boot converges the owner before
constructing the state that dispatches allocation actions, and `AppState::new`
can only capture ports the supplied owner already reports.

Each builder now:

1. creates or receives its existing fixture owner;
2. awaits that same owner's existing `converge_shared()` operation;
3. passes the same `Arc` instance to `AppState::new`; and
4. leaves all action stimuli and assertions after construction unchanged.

This is verified for the netns-density seam, runtime-convergence helper,
service-workload helper, injected-clock helper, lifecycle-backoff helper, and
mTLS fail-closed seam. No port is injected, guessed, defaulted, copied from a
second owner, or exposed through a new seam. In the journaled fixtures the
per-action trace begins after construction, so boot convergence is not
misclassified as an allocation effect. The eighteen former failures can now
reach their existing behavioral oracles without altering those oracles.

## Fail-stop request ownership

`ComposedNode::fail_stop_requested` is a nonblocking observation of the
existing `ServerHandle::shutdown_requested` future. Once ready, that future
consumes the supervisor's request channel or completed task result. The former
fixture discarded the returned value and later awaited the same one-shot
boundary again in `end_after_fail_stop`, causing four timeouts.

The fixture now caches the actual returned `ServeShutdownRequest`:

- repeated nonblocking observations clone the cached value;
- `end_after_fail_stop` consumes that exact cached request;
- if no request was observed, `end_after_fail_stop` retains the original
  blocking await; and
- boot and unrelated crash boundaries clear stale cache state.

The cache does not infer a cause, manufacture a request, repoll a consumed
one-shot, change supervisor state, or alter timeout/retry/recovery budgets. It
is private fixture ownership of an already-observed value; the production
`ServerHandle` API and supervisor protocol are unchanged.

## Composed-process teardown

The six quiescence failures occurred at runner process boundaries rather than
at a modeled owner invariant. One driver instance is reused across traces and
dropped after the last trace; replacing or dropping a live node without an
explicit process end made the Sim seam report process-owned handles. The old
explicit crash order also exited the seam before aborting server tasks, so a
surviving task could call an already-exited adapter.

The shared private boundary now ends an abrupt composed process in this order:

1. drop the fixture's retained owner and dial references;
2. consume `ServerHandle::kill_for_test`, which performs its existing
   immediate abort-and-join fence for all server-owned tasks; and
3. call `SimGuestFlowIo::exit_process` only after no task can issue another
   owner operation.

`QuiescenceDriver` invokes the same boundary before replacing a live node,
before entering `InitDown`, and on ordinary final drop. Explicit `Crash` and
fail-stop use the same task-before-seam order. Panic unwinding performs no
second asynchronous teardown, matching the Sim seam's existing rule that it
does not raise a second leak panic while the thread is already panicking.

This does not suppress the seam's ordinary owner-leak oracle. A seam dropped
without a declared process exit still counts open handles and panics. These
paths explicitly model abrupt process termination, where process-scoped tasks
and descriptors must die; `exit_process` remains the existing kernel/context
effect of that boundary. The change neither gracefully drains workloads nor
turns a live process leak into a successful steady state.

## Scope and unresolved groups

The four corrections add no production API, owner method, test, scenario,
timeout, retry, lane, or synthetic port/state seam. Concurrent marker
activations and production implementation changes remain crafter-owned and are
not attributed to this review.

The other broad-gate groups remain unresolved by this package: full-scale
`quint_flow_quota` initialization, the `quint_udp_slots` observation gap,
owner-flow semantic divergences, worker stop/beacon failures, the Sim
far-end-oracle question, supervisor cadence, and production fixes. In
particular, all 55 real-cgroup failures still require their approved disposable
cgroup/Cloud Hypervisor execution boundary. This source approval neither
waives those failures nor treats the physical-host run as green.

## Review gates

| Gate | Result | Iteration-50 evidence |
|---|---|---|
| Positive PBT domain | PASS | Only services rejected by accepted listener/probe ingress constraints are excluded. |
| Probe coverage | PRESERVED | Per-role ranges, zero/non-empty cases, arbitrary descriptors, ordering, and distinct role indices remain. |
| Listener coverage | PRESERVED | Both protocols and every valid nonzero port remain; reserved-port sad paths are unchanged. |
| AppState owner identity | PASS | Each builder converges and passes the same owner instance. |
| Port source | PASS | Ports come only from the converged owner's existing `flow_ports`; none is injected or guessed. |
| Fixture action oracles | PRESERVED | Convergence occurs before construction/trace baselines; action bodies and assertions are unchanged by this alignment. |
| Fail-stop request | PASS | Cache contains the actual first observed request and is consumed once. |
| Timeout/recovery policy | PRESERVED | No wait duration, budget, retry, or recovery rule changes. |
| Process teardown order | PASS | External refs drop, server tasks abort/join, then the seam exits. |
| Leak detection | PRESERVED | Non-process-exit seam drops still enforce the existing handle-leak panic. |
| Surface discipline | PASS | No production/public API, owner protocol, seam, test, scenario, or lane is added. |
| Remaining failures | NOT WAIVED | Production/conformance groups and 55 physical-boundary cgroup failures remain open. |

## Mechanical verification

| Check | Result |
|---|---|
| Targeted source inspection | PASS — six strategy files, six builder files, and two private composed-fixture files match the bounded classifications. |
| Sad-path diff | PASS — `vsock_deploy_validation.rs` is unchanged. |
| Same-owner inspection | PASS — every reviewed builder passes the exact owner it converged. |
| `rustfmt --edition 2024 --check` | PASS for all fourteen reviewed Rust files. |
| Scoped `git diff --check` | PASS for all reviewed files, the handoff, and this review artifact. |
| Broad/focused/Metal/formal/mutation execution | NOT RUN by this bounded source reviewer. |

## Iteration-50 verdict

**APPROVED — SOURCE ONLY.** The four corrections align positive properties
with valid ingress, compose six fixtures with their actual converged owner,
retain the first observed fail-stop request, and end composed processes before
their seam. They preserve all behavioral assertions, leak detection, timeout
policy, valid-input coverage, and production boundaries. Remaining broad-gate
and physical-cgroup failures are neither changed nor waived.

# APPROVED — SUPPLEMENTAL ITERATION 50 (SOURCE ONLY)

---

# Supplemental review iteration 51 — guest EXIT authority fixture ordering

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-51` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL source reviewer |
| Scope | Existing VM EXIT-authority fixture stimulus and receipt boundary |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The former fixture terminated the VMM and only afterward queued `EXIT 7` on the
guest session. That ordering cannot occur on the real boundary: after process
death the guest cannot write, and `SimBeaconIo::read_line` correctly reports
`ConnectionReset` before returning any newly queued line once the accepted
connection's VMM is dead. The failure was therefore a fixture stimulus defect,
not evidence against the EXIT-authority implementation.

## Corrected ordering

The existing scenario now orders the same effects as the real lifecycle:

1. start the VMM and accept the guest beacon session;
2. release the existing Running-confirmed gate;
3. write `EXIT 7` while the VMM is alive;
4. observe the production BeaconWriter consume that line and return to its
   next blocked read; and
5. only then terminate the VMM.

The final assertion remains exactly:

```text
ExitKind::Crashed { exit_code: Some(7), signal: None }
```

It therefore continues to prove that the accepted guest report is
authoritative over the VMM's later, conflicting teardown outcome. Allocation
identity and exactly-one exit-event reception remain unchanged.

## Receipt observation

The fixture uses the existing `SimBeaconIo::parked()` read surface rather than
a scheduler delay or queue mutation:

- immediately after `GuestSession::write("EXIT 7")`, `parked()` must be false,
  proving an in-flight beacon read is deliverable from the queued line;
- the bounded loop waits until `parked()` becomes true again, which requires
  the line to have left the Sim inbound queue and the BeaconWriter to have
  returned to a read with no deliverable input; and
- the subsequent unchanged exit-event assertion proves the consumed line
  reached the exit-classification path as `EXIT 7`.

The loop's `yield_now` does not define completion and has no fixed count.
Completion is solely the existing parked-state observation; the one-second
timeout only fails a fixture whose production writer never consumes and
reparks. No `SimClock` tick, sleep-based ordering claim, direct inbound-queue
edit, or adapter behavior change is introduced.

Because `SimBeaconIo::try_read` checks a dead accepted VMM before returning a
buffered line, moving termination after the observed receipt is necessary and
sufficient. It does not make the adapter permissive or conceal the prior
post-death reset.

## Scope and oracle preservation

Only the existing test's stimulus order and comments changed. The production
BeaconWriter, exit watcher, bounded guest-report drain, VMM adapter,
`SimBeaconIo`, `GuestSession`, published beacon protocol, and public/test
support APIs are unchanged. The test still uses the same allocation, handle,
session, Running gate, real `Vmm::terminate` port call, exit receiver, and
typed EXIT-7 oracle.

Concurrent activation-marker removals elsewhere in the file are crafter-owned
and excluded from this bounded review. No test, scenario, seam, adapter state,
yield-count rule, or queue-injection helper is added.

## Review gates

| Gate | Result | Iteration-51 evidence |
|---|---|---|
| Realizable stimulus | PASS | Guest writes while its VMM and accepted connection are alive. |
| Receipt boundary | PASS | Deliverable-read false-to-true parked transition precedes VMM termination. |
| Effect completion | PASS | The production BeaconWriter consumes the line and returns to a blocked read before teardown. |
| No timing proxy | PASS | State observation decides completion; timeout only bounds failure and no fixed yield count is used. |
| Adapter semantics | PRESERVED | Post-death reads still reset before buffered delivery. |
| EXIT authority | PRESERVED | Exact `EXIT 7` classification remains the final oracle. |
| Later VMM outcome | PRESERVED | Real VMM termination still follows the guest report and cannot overwrite it. |
| Surface discipline | PASS | No production/test-support API, seam, adapter behavior, helper, or scenario is added. |

## Mechanical verification

| Check | Result |
|---|---|
| Scoped body diff | PASS — write/receipt moved before the existing terminate call; final assertions are unchanged. |
| `rustfmt --edition 2024 --check` | PASS for `vm_driver_stop_totality.rs`. |
| Scoped `git diff --check` | PASS for the fixture, handoff, and this review artifact. |
| Focused/broad/Metal/mutation execution | NOT RUN by this bounded source reviewer. |

## Iteration-51 verdict

**APPROVED — SOURCE ONLY.** The fixture now proves a realizable lifecycle:
the living guest's EXIT report is consumed through the production BeaconWriter
before VMM teardown, and the unchanged exit event remains authoritative over
the later VMM outcome. The correction adds no timing-count oracle, queue
injection, adapter relaxation, seam, API, or test.

# APPROVED — SUPPLEMENTAL ITERATION 51 (SOURCE ONLY)

---

# Supplemental review iteration 52 — stale-launch same-CID fixture domain

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-52` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL source reviewer |
| Scope | Two existing queued previous/dead-launch owner-acceptance fixtures |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The two existing bodies describe connections queued by a previous launch of
the same CID. Their assertions distinguish attribution at first-record read:
the current CID row exists, but the record carries the prior launch token and
must be refused as stale. The former default `/16` fixture did not establish
that precondition. Its next-fit cursor naturally assigned the replacement a
different CID after release, so the old connection was correctly classified as
unknown and closed at accept before the body attempted its scripted read.

## Corrected fixture domain

A private helper now boots the same production owner composition through the
existing `Composition::with_prefix` path with:

```text
prefix  = 100.95.0.0/30
gateway = 100.95.0.1
```

After excluding the network, broadcast, and gateway addresses, the production
`GuestAddressPool` has exactly one usable workload offset. The sequence is
therefore natural production behavior:

1. the old allocation receives the sole address and derived CID plus a launch
   token from the existing seeded `SimEntropy` port;
2. its connection is queued while auto-delivery is disabled;
3. the VMM stops, the lease retires, owner teardown completes, and the pool
   releases that offset;
4. the replacement allocation is admitted through ordinary next-fit, wrapping
   to the same sole address/CID; and
5. the new lease draws the next launch token from the same entropy sequence.

No CID, pool cursor, lease row, owner table, connection key, or launch token is
written by the fixture. CID reuse and token renewal both come from the existing
production pool.

## Oracle preservation

The scripted accept/read order and all assertions remain unchanged. Because
the replacement row now owns the same CID, the old queued connection reaches
its first-record read rather than the unknown-CID accept close. Its old token
then differs from the replacement lease's fresh token, retaining the exact
ATTR-TOKEN refusal boundary.

The prior-launch session still cannot take the claim, while the current
launch's session does. In the dead-launch matrix, the old session, TCP flow,
datagram flow, and empty connection still close at read; no control message,
registration, destination connect, or attach crosses launches; the current
session remains the sole holder and activation still succeeds.

The correction does not relax the owner's canonical unknown-CID early-close
behavior. It places only these two stale-token scenarios in the same-CID domain
their prose and read-stage assertions already require. Unknown-CID handling
remains production behavior, and the separate direct foreign-token refusal
remains existing coverage; no owner implementation or Sim classification
changed.

## Scope audit

`one_cid_composition` is private to the source-local acceptance module and has
exactly two callers. It reuses existing composition, boot, pool, entropy,
forwarder, listener, and routing paths. The only body edit is replacing
`Composition::booted(seed)` with that helper.

No wait, release order, auto-delivery setting, socket stimulus, assertion,
typed cause, API, seam, adapter behavior, test, or scenario changed. Concurrent
marker activation elsewhere in the file is crafter-owned and excluded from
this review.

## Review gates

| Gate | Result | Iteration-52 evidence |
|---|---|---|
| Pool capacity | PASS | `/30` with gateway `.1` leaves one production-usable offset. |
| CID reuse | PASS | Release plus ordinary next-fit wraps naturally to the sole derived CID. |
| Launch renewal | PASS | A second `assign` draws the next token from existing seeded entropy. |
| Old connection identity | PRESERVED | Queued keys and first records are created before teardown from the old lease. |
| New owner state | PRESERVED | Replacement lease/session are created through normal owner ports. |
| Read-stage stale refusal | PRESERVED | Same CID reaches read; old token retains ATTR-TOKEN mismatch. |
| Unknown-CID policy | PRESERVED | No production accept classification changes or test-only override is introduced. |
| Assertions and timing | PRESERVED | All releases, settles, observations, and oracles remain unchanged. |
| Surface discipline | PASS | Private two-caller helper only; no API, seam, state injection, test, or scenario. |

## Mechanical verification

| Check | Result |
|---|---|
| Helper caller inventory | PASS — exactly the two previous/dead-launch bodies use it. |
| Scoped body diff | PASS — composition construction is the only behavioral fixture change. |
| `rustfmt --edition 2024 --check` | PASS for `admission.rs` and the existing composition support. |
| Scoped `git diff --check` | PASS for both source files, the handoff, and this review artifact. |
| Focused/broad/Metal/mutation execution | NOT RUN by this bounded source reviewer. |

## Iteration-52 verdict

**APPROVED — SOURCE ONLY.** The two fixtures now obtain same-CID reuse and a
fresh launch token through the production pool's natural one-offset behavior.
Their existing read-stage stale-launch and no-cross-launch assertions retain
full force, while unknown-CID early close remains unchanged. No identifier,
state, timing, API, seam, adapter, oracle, or scenario is manufactured.

# APPROVED — SUPPLEMENTAL ITERATION 52 (SOURCE ONLY)

---

# Supplemental review iteration 53 — stale-launch admission-order source review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-53` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL source reviewer |
| Scope | Remaining stale-launch owner-acceptance fixture ordering |
| Verdict | **CHANGES_REQUESTED — effect-completion boundary missing** |
| Open findings | 0 critical, 1 high, 0 medium |

The reordered body has the correct intended protocol sequence. All old
connections remain queued before teardown; the one-offset pool provides the
same CID with a fresh launch token; the stale old control session is delivered
first; the current launch's real `ControlOpen` must then establish its claim;
only after that may the old flow, datagram, and empty connections be admitted
for their first-record outcomes. No production gate or assertion needs to
change.

The current source, however, releases each control record and immediately
releases the next accept without settling the owner in between. The comments
describe completed reads and a live current claim, but the fixture has only
made both asynchronous effects runnable.

## F53-01 — blocking fixture-order defect: current-session claim is not completed before stale-flow accept

**Severity:** High / blocking  
**Status:** Open  
**Affected source:**
`a_dead_launchs_queued_connections_are_closed_at_their_read`

The relevant sequence is currently:

```rust
c.io.release_read(old_session, None);

let new_session = c.open_session(&new, 2048);
c.io.release_accept(new_session);
c.settle().await;
c.io.release_read(new_session, None);

for key in [old_flow, old_datagram, empty] {
    c.io.release_accept(key);
    c.settle().await;
    c.io.release_read(key, None);
}
```

`SimGuestFlowIo::release_read` only moves buffered bytes to the accepted
record's ready queue and wakes tasks. It does not execute the production owner
read or install a control-session claim synchronously. On the first loop
iteration, the current session read and the old flow accept are both ready at
the same `settle`. D-UP-19 intentionally pins no accept order. The old flow can
therefore reach the production admission check before the current session
claim exists and be correctly closed by the no-session boundary at accept,
recreating the invalid later `release_read` the edit is intended to remove.

The old stale session's read is likewise released without an owner-settle
boundary before the current session accept. Its stale token still prevents a
claim, but the stated “old session read first, then current launch claims”
ordering is not actually established by the fixture.

### Required bounded remediation

Use the existing idle/effect-completion boundary after each control record is
released:

```rust
c.io.release_read(old_session, None);
c.settle().await;

// accept current session
...
c.io.release_read(new_session, None);
c.settle().await;

// only now accept stale flow/datagram/empty
```

This adds no scheduler count, sleep, clock advance, state injection, assertion,
API, or seam. `Composition::settle` is the existing fixture observation that
every owner task is parked with nothing undelivered. It makes the authored
protocol order concrete: stale session refused with no claim, current
`ControlOpen` read and claim installed, then stale data connections admitted
and refused at their reads.

## Other source checks

| Gate | Result | Iteration-53 evidence |
|---|---|---|
| Same-CID domain | PASS | Existing one-offset production pool naturally reuses the CID and draws a new launch token. |
| Queued-before-teardown | PASS | Old session, flow, datagram, and empty connections are all created before old-owner teardown. |
| Old session first | BLOCKED on completion | Accept is first, but its released read is not settled before current-session admission. |
| Current session claim before stale flows | **FAIL** | Current read and first stale-flow accept become runnable before the same settle. |
| Production no-session gate | PRESERVED | The finding requires a fixture barrier; no admission behavior is weakened. |
| Assertions | PRESERVED | Closed-at-read, zero message/connect/attach, current claim, and activation oracles are unchanged. |
| Surface discipline | PASS | No production/API/seam/model/test addition is present or required. |

## Mechanical verification

| Check | Result |
|---|---|
| Source ordering inspection | FAIL — no settle follows either control-session `release_read`. |
| `release_read` semantics | CONFIRMED — it makes bytes ready and wakes; owner processing remains asynchronous. |
| `settle` semantics | CONFIRMED — existing helper parks all owner tasks with nothing undelivered. |
| `rustfmt --edition 2024 --check` | PASS for `admission.rs`. |
| Scoped `git diff --check` | PASS for the source, handoff, and this review artifact. |
| Focused/broad/Metal/mutation execution | NOT RUN; blocked pending F53-01 remediation and re-review. |

## Iteration-53 verdict

**CHANGES_REQUESTED.** The intended order and unchanged oracles are correct,
but the current source does not complete the current control-session read and
claim before admitting the old data connections. F53-01 requires only two
existing `settle` boundaries. No production contract, gate, state, API, seam,
timing policy, or assertion change is authorized.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 53

---

# Supplemental review iteration 54 — F53-01 claim-completion remediation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-54` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source reviewer |
| Scope | F53-01 control-session effect-completion boundary |
| Verdict | **APPROVED — SOURCE ONLY** |
| F53-01 disposition | **CLOSED** |
| Open findings | 0 critical, 0 high, 0 medium |

The fixture now completes and observes the current launch's real
`ControlOpen` claim before it releases any stale flow, datagram, or empty
connection into the accept path.

## Exact ordering review

The resulting production order is concrete:

1. accept the old queued control connection;
2. release its stale `ControlOpen` record;
3. accept the new launch's control connection and settle the owner;
4. release the new `ControlOpen` record;
5. settle again and read the production owner observation;
6. require `cids[new.cid].session == Some(new_session)`; and
7. only then begin the existing accept → settle → read sequence for each old
   data connection.

The old stale-session read does not need a separate immediate settle in this
arrangement. It is already ready before the current session accept is
released. The following existing settle runs all owner tasks to idle while the
new session's record is still unreleased, so the old stale read must complete
and cannot race a current-launch claim. The new connection can be accepted in
that settle, but it cannot claim until its `ControlOpen` bytes are explicitly
released afterward.

After that release, the new settle is the required effect-completion boundary.
The added intermediate owner observation is a production-state witness, not a
fixture flag: it proves the current same-CID row actually reports the new
session claim before any stale data connection can reach admission.

## Contract and oracle preservation

The production no-session gate remains unchanged and retains its own coverage.
This fixture does not bypass it; it establishes the live-session precondition
needed to exercise ATTR-TOKEN at first-record read. All old connections remain
queued before teardown and retain their original keys and records. Same-CID
reuse and the replacement launch token still come from the one-offset
production pool and seeded entropy.

Each stale flow/datagram/empty connection retains its existing accept,
settle, and record-read stimuli. The final assertions are unchanged: all old
connections close at read, the new session holds the claim, no stale flow or
datagram registers, no control message crosses launches, no peer connect or
attach occurs, and activation succeeds. The intermediate claim assertion
strengthens only the causal precondition already required by those outcomes.

No yield count, sleep, clock tick, queue mutation, forced identifier, state
injection, production gate change, API, seam, helper, test, or scenario is
added.

## Review gates

| Gate | Result | Iteration-54 evidence |
|---|---|---|
| Old stale session | PASS | Its read is ready before the settle in which the new record remains unreleased. |
| Current claim completion | PASS | Read release is followed by idle settle and production owner observation. |
| Claim witness | PASS | Owner reports exactly `Some(new_session)` before any stale data accept. |
| Stale data admission | PASS | Every old connection is accepted only after the current claim exists. |
| ATTR-TOKEN boundary | PRESERVED | Old same-CID records retain their prior launch token and reach read-stage refusal. |
| No-session gate | PRESERVED | Production admission behavior is unchanged rather than bypassed. |
| Final oracles | PRESERVED | Closed/zero-effect/current-claim/activation assertions remain unchanged. |
| Surface discipline | PASS | Existing settle and owner observation only; no API, seam, state, test, or scenario. |

## Mechanical verification

| Check | Result |
|---|---|
| Source ordering inspection | PASS — current claim settle and witness precede the first stale accept release. |
| Old-read completion inspection | PASS — new record remains unreleased during the settle that completes the old read. |
| `rustfmt --edition 2024 --check` | PASS for `admission.rs`. |
| Scoped `git diff --check` | PASS for the source, handoff, and this review artifact. |
| Focused/broad/Metal/mutation execution | NOT RUN by this bounded source reviewer. |

## Iteration-54 verdict

**APPROVED — SOURCE ONLY.** F53-01 is closed. The owner now demonstrably holds
the current launch's real control-session claim before any old data connection
is admitted, while the stale old session completes first and every existing
ATTR-TOKEN, zero-effect, claim, and activation oracle remains intact. No
production behavior or test-support surface changed.

# APPROVED — SUPPLEMENTAL ITERATION 54 (SOURCE ONLY)

---

# Supplemental review iteration 55 — quiescence disabled-flow witness

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-55` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL/formal source reviewer |
| Scope | Existing `NoFlowOpensWhileCountedHoldRemains` terminal witness |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The named run previously ended with `.then(OpenFlow).fail()`. Quint represented
the deliberately disabled action as an empty terminal `{}`, so quint-connect
failed while extracting the required recorded `conf_step` before the driver
could replay anything. The same tool incompatibility was already established
for the CID named runs; an empty terminal state is not the semantic witness the
recording conformance main needs.

## Exact guard equivalence

The original `OpenFlow` enabledness clauses were:

```quint
up, not(latch), flowsOpen == 0
```

The factored state-reading definition is exactly:

```quint
def canOpenFlow: bool = up and not(latch) and flowsOpen == 0
```

`OpenFlow` now uses `canOpenFlow` as its sole guard, followed by the same
primed assignments and the same recorded `OpenFlow` step. The definition is a
regular `def`, as required for a predicate that reads state, and contains no
primed write, nondeterminism, derived approximation, or extra condition.

The named run retains the complete
`TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore` prefix.
Its terminal state has `up = true`, `cnt = 1`, `latch = true`, and
`flowsOpen = 0`. It now ends with:

```quint
.expect(not(canOpenFlow))
```

That assertion is logically identical to requiring the existing action to be
disabled. The prior `.fail()` added no transition when correct; replacing it
with a state predicate therefore preserves the entire state/action sequence,
the counted-hold refusal meaning, the earlier Safety assertion, and the
terminal `conf_step` from the last real action while avoiding an empty ITF
state.

## Scope and evidence disposition

No state variable, action, action effect, enabled transition, run name,
scenario, temporal property, hazard, check, driver, API, seam, or production
behavior changed. The README describes the same counted-hold witness and the
tool-format reason for its representation.

The separate randomized seed `0x8703c2cd` postmortem projection gap remains
unedited pending its exact trace. This correction neither filters that trace
nor claims to resolve it.

Because `quiescence.qnt` is a formal input, this source approval does not make
the historical record current. The complete guest-flow-owner job-3 record,
`verify-evidence`, input/summary hashes, named/randomized conformance, and
independent evidence review remain mandatory after all source changes are
approved.

## Review gates

| Gate | Result | Iteration-55 evidence |
|---|---|---|
| Guard terms | IDENTICAL | `up`, `not(latch)`, and `flowsOpen == 0` are the complete old and new guards. |
| Action use | PASS | `OpenFlow` uses the factored guard and retains every effect/record assignment. |
| Disabled witness | EQUIVALENT | Failed action application is replaced by exact negated enabledness in the same state. |
| State sequence | PRESERVED | The named-run prefix and terminal model state are unchanged. |
| Outcome meaning | PRESERVED | A counted hold still proves that no new flow may open. |
| Formal inventory | PRESERVED | 19 actions, 11 runs, 6 temporals, 14 variants, and 5 modules retain their names and order. |
| Unrelated random gap | UNCHANGED | Seed `0x8703c2cd` remains pending exact replay/disposition. |
| Surface discipline | PASS | No test, driver, API, seam, state, action, scenario, or filter is added. |

## Mechanical verification

| Check | Result |
|---|---|
| Guard text comparison | PASS — conjunction terms are identical. |
| Formal inventory comparison | PASS — names, counts, and order unchanged. |
| Check inventory diff | PASS — `checks.toml` unchanged. |
| Scoped `git diff --check` | PASS for the model, README, handoff, and this review artifact. |
| Typecheck/named/randomized/job-3 record/verify-evidence | NOT RUN by this source reviewer; mandatory and pending. |

## Iteration-55 verdict

**APPROVED — SOURCE ONLY.** The named run now proves the same exact disabled
`OpenFlow` guard without emitting an empty terminal ITF state. Action
enabledness, effects, sequence, refusal meaning, inventory, and every product
boundary remain unchanged. Replacement formal evidence is still required.

# APPROVED — SUPPLEMENTAL ITERATION 55 (SOURCE ONLY)

---

# Supplemental review iteration 56 — private per-process runtime ownership plan

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-56` |
| Date | 2026-10-26 |
| Reviewer | Independent DISTILL ownership/source-plan reviewer |
| Scope | Two-file private runtime-ownership plan for abrupt quiescence conformance processes |
| Verdict | **APPROVED — PLAN ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The reproduced postmortem call has a precise private-fixture cause. A
`ComposedNode` process owns detached owner-runtime child tasks, but the current
`QuiescenceDriver` reuses one Tokio runtime across modeled process deaths.
`ServerHandle::kill_for_test` fences the server-owned top-level tasks; it has no
handle for every detached accept, control-session, and flow child. Calling
`SimGuestFlowIo::exit_process` while that shared runtime remains live therefore
allows an old child to run against an already-exited seam. A real process
death instead destroys the runtime and every child together.

The proposed refactor gives each modeled process incarnation one private
runtime while retaining the existing node-global fixture state. It is the
smallest correction that matches that ownership boundary without adding a
production drain or a task-control seam.

## Phase-A process ownership

`ComposedNode` may split its existing abrupt stop into an internal phase A
that:

1. takes the current `Process` out of the node;
2. clears/captures the existing cached fail-stop request as appropriate;
3. scripts the same retained-table crash fact;
4. drops the process's retained owner, dial, DNS/CID, and other external
   fixture references;
5. consumes and awaits the existing `ServerHandle::kill_for_test` immediate
   abort/join fence;
6. records the existing `NodeStop`; and
7. returns the process's existing `Arc<SimGuestFlowIo>` without calling
   `exit_process`.

This phase neither gracefully tears down the owner nor creates another handle.
The only remaining owners of process-bound futures are detached tasks on the
current runtime, which is exactly the ownership the next phase removes.

Ambient-runtime acceptance callers retain their current behavior by invoking
phase A and immediately applying the same seam `exit_process` themselves. The
new phase-A entrypoint is private test support; it is not a product or Sim
port.

## Quiescence-driver runtime rotation

The `QuiescenceDriver`'s existing current-thread, paused-time Runtime becomes
the runtime of one process incarnation. On modeled `Crash`, fail-stop, live
trace replacement, and ordinary final drop, the driver performs:

```text
phase A on current runtime
-> return to synchronous Driver code
-> replace and drop the old Runtime
-> io.exit_process()
```

Dropping the current-thread runtime cancels and drops every remaining detached
child. Because only the phase-A future can execute while `block_on` owns that
single thread, no child is concurrently running when `block_on` returns. The
seam exits only after runtime destruction, so no task remains able to call it.

Runtime drop occurs outside a Tokio async context. `Driver::step`, the helper
methods, and `Drop` are synchronous; their asynchronous work is completed by
the existing private `Runtime::block_on` before `mem::replace`/drop. This avoids
Tokio's forbidden “drop a runtime from an asynchronous context” shape.

A fresh current-thread runtime is installed for a later `RestartWith`. The
node retains its existing `SimClock`, `SimMtlsIntercept`,
`GuestAddressPool`, durable directories, boot counter, and refused-boot
observation seam. Tokio time is again paused for the fixture idle signal;
production time continues to come from the retained injected `SimClock`.

## Failed boot and panic boundaries

A typed boot refusal ends that attempted process incarnation. The driver must
rotate its runtime immediately after the boot future returns and before any
subsequent driver action or settle. The current refused-boot path performs no
await after its existing seam exit; immediate current-thread runtime
destruction therefore cancels every child made ready by that exit before the
driver can poll it again. The retained `refused_io` Arc remains available for
its existing issued-effect observation. An implementation that adds another
await/poll between refused seam exit and runtime rotation would violate this
approved plan.

During panic unwinding the driver's custom `Drop` performs no `block_on` or
other blocking cleanup. Rust field destruction drops the Runtime before the
subsequent node/process fields, cancelling runtime-owned tasks first. The Sim
adapter's existing `thread::panicking()` guard remains the diagnostic-safe
fallback; no assertion or adapter behavior is suppressed by new code.

## Caller and retained-handle audit

| Owner/reference | Required plan disposition |
|---|---|
| `Process.owner`, `Process.dial`, DNS/CID fixture references | Dropped in phase A before the server fence. |
| `ServerHandle` top-level server/supervisor tasks | Consumed by existing `kill_for_test` and awaited. |
| Detached owner accept/session/flow children | Owned only by the process Runtime; cancelled by Runtime drop. |
| `Arc<SimGuestFlowIo>` | Returned from phase A and exited only after Runtime drop. |
| Node clock/table/pool/directories/boot count | Remain on `ComposedNode` across restart. |
| Refused-boot seam | Retained for existing effect inspection after immediate runtime rotation. |
| New process tasks | Spawn only on the fresh Runtime installed for that restart. |

No old process future or owner reference may cross into the fresh runtime.
Conversely, node-global facts do not move into the old runtime's destruction.

## Contract and scope audit

Compared model state is unchanged. `Crash` and fail-stop still produce only
the existing down state, stop cause, zero process-scoped owner state, and
retained table. Restart still reopens the same durable/node-global fixture
facts. Ordinary live actions keep their current production ports and
projections.

The plan changes exactly two test-support files. It adds no production drain,
owner task hook, Tokio handle to product code, Sim method, public API, model
state/action/projection, wait, yield count, sleep, assertion suppression, or
cleanup outcome. Ambient callers keep their existing visible crash/fail-stop
semantics.

The independent Trace-2 `OpenFlow`/damaged-FlowListeners formal contract gap at
seed `0x8703c2cd` remains unedited and unfiltered. This plan addresses only
Trace 1's process-runtime ownership.

## Plan gates

| Gate | Result | Iteration-56 evidence |
|---|---|---|
| Failure ownership | PASS | Surviving children are owned by the reused private runtime, not by a missing production API. |
| Abrupt semantics | PASS | Existing immediate `kill_for_test` remains; no graceful owner teardown is introduced. |
| Child-task completion | PASS | Runtime destruction cancels detached children that no retained JoinHandle can reach. |
| Seam ordering | PASS | Runtime is gone before the phase-A seam exits. |
| Tokio drop context | PASS | Rotation occurs in synchronous driver code after `block_on` returns. |
| Restart isolation | PASS | Fresh process tasks use a new paused current-thread Runtime. |
| Node-global continuity | PASS | Clock, table, pool, directories, and boot count remain outside process runtime ownership. |
| Normal drop | PASS | Same phase A → runtime drop → seam exit ordering is available. |
| Panic drop | PASS | No blocking custom cleanup; field order drops runtime before node/process state. |
| Failed boot | PASS with implementation constraint | Rotate immediately after the refusal future returns; no intervening poll after the existing seam exit. |
| Model/projection | PRESERVED | No state, action, outcome, or observation mapping changes. |
| Surface discipline | PASS | Two private test-support files; no production or Sim API/seam. |

## Required implementation proof

Implementation approval will require source evidence that phase A consumes the
whole `Process`, that old Runtime drop occurs outside every `block_on` and
before seam exit, that normal/panic/failed-boot paths follow the reviewed
order, and that ambient callers still exit exactly once. The exact
`0x8703c2cd` Trace-1 replay must then show no postmortem Sim call or leaked
handle. Full named/randomized/formal evidence remains a later gate.

## Iteration-56 verdict

**APPROVED — PLAN ONLY.** Per-process ownership by the existing private Tokio
runtime is the bounded OS-equivalent correction for Trace 1. Phase A fences
known top-level tasks and relinquishes fixture references; dropping the old
runtime removes otherwise-unreachable children; only then may the seam exit.
The plan preserves node-global state, abrupt semantics, model projections,
ambient caller behavior, and every production boundary. Source implementation
and exact-seed proof remain mandatory before this correction is complete.

# APPROVED — SUPPLEMENTAL ITERATION 56 (PLAN ONLY)

---

# Supplemental review iteration 57 — per-process runtime source review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-57` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL ownership/source reviewer |
| Scope | Two-file implementation of approved plan iteration 56 |
| Verdict | **CHANGES_REQUESTED — one failed-boot path does not rotate** |
| Open findings | 0 critical, 1 high, 0 medium |

The implementation correctly realizes nearly all of the approved ownership
plan. `ComposedNode` now exposes phase-A crash and fail-stop endings that remove
the `Process`, drop retained owner/dial references, consume the existing server
abort/join fence, record the same stop cause, and return the old seam without
exiting it. Ambient helpers immediately exit that seam and retain their public
behavior.

`QuiescenceDriver` uses one constructor for paused current-thread runtimes.
Crash, fail-stop, live trace replacement, and normal final drop all run phase A
on the current runtime, return to synchronous driver code, replace and drop the
old runtime, then call `exit_process` on the retained seam. Runtime destruction
is therefore outside every `block_on`, and detached owner children cannot run
after seam exit. Panic unwinding skips blocking custom cleanup and field order
drops the runtime before the node/process fields.

## F57-01 — blocking ownership gap: `InitWith` boot refusal propagates without runtime rotation

**Severity:** High / blocking  
**Status:** Open  
**Affected source:** `QuiescenceDriver::init_with`

Plan iteration 56 requires every typed boot refusal to end and rotate that
attempted process runtime. `restart_with` implements this correctly by saving
`booted.is_err()`, constructing the action result, and calling
`replace_process_runtime(None)` before returning.

`init_with` still performs the boot inside an async block with `?`:

```rust
self.rt.block_on(async {
    node.boot().await.map_err(|refusal| DriverError::BootDiverged { ... })?;
    ...
    Ok(())
})?;
```

If a boot expected to succeed returns a typed refusal, the outer `?`
propagates `BootDiverged` before `replace_process_runtime` is called. The local
node drops, but the driver retains the same runtime that hosted the refused
incarnation and any detached owner children it created. Runtime destruction
eventually occurs if the whole driver is dropped, but it is not the immediate
failed-process boundary the approved plan requires, and the error can return
to the runner while that ended runtime is still live.

This is a concrete current caller path, not a hypothetical extra API: every
`InitWith` trace invokes it, and an unexpected production boot refusal is
exactly the `BootDiverged` branch the method already represents.

### Required bounded remediation

Capture the `block_on` result before applying `?`. When it is `Err`, rotate the
ended runtime synchronously and only then propagate the same error. The refused
boot's existing `ComposedNode::boot` path has already exited and retained its
seam, so this path must use `replace_process_runtime(None)` exactly as
`restart_with` does; it must not add another seam exit.

No model outcome, error mapping, timeout, API, seam, task hook, or assertion
changes. Successful `InitWith` continues to install the newly booted local
node on its current runtime.

## Other ownership-path dispositions

| Path | Result | Iteration-57 evidence |
|---|---|---|
| Phase-A crash | PASS | Whole `Process` consumed; external owner/dial refs dropped; server fence awaited; seam returned live. |
| Ambient crash | PASS | Same phase A followed by one immediate seam exit. |
| Phase-A fail-stop | PASS | Actual cached/awaited request consumed once; server fenced; seam returned. |
| Ambient fail-stop | PASS | Same phase A followed by one exit and unchanged request return. |
| Driver Crash | PASS | Phase A → synchronous old-runtime drop → seam exit. |
| Driver fail-stop | PASS | Validated request, phase A, runtime drop, seam exit; existing cause unchanged. |
| Trace replacement / `InitDown` | PASS | Live process is detached and its runtime destroyed before node replacement. |
| Normal driver drop | PASS | Same ownership order, with no old child crossing field destruction. |
| Panic unwind | PASS | No `block_on`; runtime field drops before node/process fields. |
| `RestartWith` refused boot | PASS | All typed refusal results rotate before return. |
| `InitWith` refused boot | **FAIL** | `?` returns before runtime rotation. |
| Successful restart | PASS | Fresh runtime hosts the new process; node-global clock/table/pool/dirs persist. |

## Scope and mechanical checks

No model state, action, projection, stop cause, production API/trait, Sim API,
graceful teardown, assertion suppression, yield, sleep, or timing policy
changed. The new methods remain in test-only support, and their caller
inventory is limited to ambient wrappers plus `QuiescenceDriver`.

| Check | Result |
|---|---|
| Runtime-drop context inspection | PASS — every implemented replacement is synchronous after `block_on`. |
| Whole-Process ownership inspection | PASS — phase A consumes owner, dial, DNS/CID fixture values, and `ServerHandle`. |
| Seam-exit ordering | PASS on implemented phase-A callers. |
| Failed-boot path audit | **FAIL** only for `init_with`; `restart_with` passes. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed files. |
| Scoped `git diff --check` | PASS for both source files, handoff, and this review artifact. |
| Exact-seed/focused/formal execution | NOT RUN; blocked pending F57-01 remediation and re-review. |

## Iteration-57 verdict

**CHANGES_REQUESTED.** Runtime ownership, phase-A fencing, normal/panic drop,
crash, fail-stop, trace reset, ambient callers, and `RestartWith` refusal all
match approved plan 56. F57-01 is the only open finding: `InitWith` can return a
boot refusal while retaining the ended incarnation's runtime. Rotate before
propagating that unchanged error, then return for source re-review and exact
seed proof.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 57

---

# Supplemental review iteration 58 — refused-boot runtime remediation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-58` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL ownership/source reviewer |
| Scope | F57-01 remediation in the two approved private test-support files |
| Verdict | **APPROVED — source correction; exact-seed proof remains mandatory** |
| Open findings | 0 critical, 0 high, 0 medium |

F57-01 is resolved. `ComposedNode::boot_with_deferred_refusal_exit` runs the
same boot composition and preserves the same successful `Process`. On refusal
it records the same `NodeStop::Refused` and retained `refused_io`, but returns
the exact `ControlPlaneError` with the process seam still live. The existing
`boot` method is now a compatibility wrapper: it invokes the deferred helper,
exits the returned seam exactly once on refusal, and returns the same typed
error. Its signature, success behavior, refusal state, and ambient caller
semantics are unchanged. Caller inventory confirms that only
`QuiescenceDriver` uses the deferred form directly.

`QuiescenceDriver::init_with` now captures the deferred result before error
propagation. On refusal it synchronously replaces and drops the old paused
runtime, exits the returned seam, and only then constructs the same
`BootDiverged { expected: "succeed", observed: "refused: ..." }`. On success
it opens the optional initial flow on that same runtime and installs the same
new node. No successful-process future or state crosses a runtime boundary.

`restart_with` applies the same ordering to every typed refusal. It first
preserves the exact old classification: the scripted
`MtlsBoot(SharedOwner)` refusal is accepted only when `write_ok` is false,
other refusals remain `BootDiverged`, and an unexpected success remains
`BootDiverged`. If and only if boot refused, it then drops the ended runtime
and exits that refusal's actual seam before returning the already-computed
result. A successful restart retains its live runtime and `Process`.

## Ownership and error-path audit

| Path or invariant | Result | Iteration-58 evidence |
|---|---|---|
| Ambient successful boot | PASS | Wrapper delegates to the same composition and retains the same `Process`. |
| Ambient refused boot | PASS | Wrapper exits the returned actual seam once and returns the identical typed refusal. |
| `InitWith` refusal | PASS | Old runtime is dropped outside `block_on`, then the actual refused seam exits, then unchanged `BootDiverged` propagates. |
| `RestartWith(false)` expected refusal | PASS | Exact `MtlsBoot(SharedOwner)` match remains the only accepted refusal; cleanup precedes `Ok`. |
| Unexpected restart refusal | PASS | Exact typed error is retained for the unchanged diagnostic mapping; cleanup precedes propagation. |
| Unexpected restart success | PASS | No refused seam exists and the live process remains on its runtime until normal driver teardown. |
| Successful `InitWith` initial flow | PASS | Flow opens only after successful boot and before installing the node, as before. |
| Runtime-drop context | PASS | Both refusal rotations occur in synchronous driver code after `block_on` returns. |
| Seam identity and ordering | PASS | Cleanup uses the `Arc<SimGuestFlowIo>` returned by that exact failed boot; runtime destruction precedes `exit_process`. |
| Panic unwind | PASS | No blocking cleanup was added to unwinding; the driver's existing panic-safe drop and field destruction order remain intact. |
| Crash/fail-stop paths | PASS | F57-approved phase-A implementations are unchanged by this remediation. |
| Projection and errors | PASS | No model state/action/projection, stop cause, typed refusal, or `BootDiverged` wording changed. |
| Surface discipline | PASS | The helper is confined to the existing test-only support module; no production or Sim API/trait/seam changed. |

The retained `refused_io` clone observes the same exited seam after the
returned clone is released, matching the former ambient `boot` behavior and
preserving refusal-effect inspection. Runtime replacement creates no second
exit and performs no wait, yield, sleep, graceful owner teardown, or result
suppression.

## Verification

| Check | Result |
|---|---|
| Deferred-helper caller inventory | PASS — direct callers are only the ambient `boot` wrapper and the two `QuiescenceDriver` boot paths. |
| Refusal classification inspection | PASS — exact existing `ControlPlaneError` matching and `BootDiverged` mappings are preserved. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed source files. |
| Scoped `git diff --check` | PASS for both source files, the handoff, and this review artifact before this append. |
| Exact-seed/focused/formal execution | NOT RUN — source approval precedes the mandatory exact-seed proof and later full evidence run. |

## F57-01 disposition

**RESOLVED.** Both refused-boot entry points now destroy the exact ended
process runtime before exiting its actual seam and before returning their
unchanged result. Ambient callers retain immediate refused-seam exit and the
same public test-support behavior.

## Iteration-58 verdict

**APPROVED — SOURCE CORRECTION.** The remediation closes the sole iteration-57
ownership gap without changing successful boot, typed-error classification,
model behavior, production surface, Sim surface, or ambient fixture semantics.
The exact `0x8703c2cd` Trace-1 replay remains mandatory before runtime-ownership
completion can be claimed; this source verdict does not claim execution.

# APPROVED — SUPPLEMENTAL ITERATION 58 (SOURCE ONLY)

---

# Supplemental review iteration 59 — `flow_quota` serving-context projection

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-59` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source reviewer |
| Scope | Existing `QuotaDriver::project` context-history alignment only |
| Verdict | **APPROVED — source only** |
| Open findings | 0 critical, 0 high, 0 medium |

The correction matches the already approved QPROJ-8 context identity used by
the `fwd_pair` driver. `SimFlowForwarder::contexts()` retains every device
context in open order, including the released phase-1 probe row. The owner's
actual `forwarder_generation`, by contrast, counts only contexts opened by
the serving owner. Subtracting that generation count from the complete table
therefore identifies the exact serving suffix without clearing, copying, or
filtering history by row contents.

The projection rejects every dishonest partition. The owner generation may
not exceed the device row count; the non-serving prefix must contain exactly
one row; and that phase-1 row must be closed with zero live pairs, zero live
watches, and an empty queued-end list. An absent or extra prefix row and every
retained phase-1 resource remain explicit conformance failures.

The existing pair identity is preserved as
`(ForwarderGeneration, ForwardId)`. A held flow's production
`GuestFlowPhase::Paired` generation now indexes `generation - 1` into the
serving suffix, so generation 1 resolves the first serving row rather than
the phase-1 probe. The existing record-loss oracle remains exact: a held pair
must be either present in that generation's queued ends or present in the
forwarder's live pair state; otherwise projection still fails with
`record was produced and lost`.

`endq` is likewise built from every serving row in open order, with the
existing generation reconstruction and exact pair-to-place lookup. An unknown
queued ID still fails instead of being discarded. Released serving history
would remain in the suffix, while the one phase-1 row remains visible and
validated outside it.

## Reproduced-cause alignment

The recorded seed `0x80618105` reaches `init -> Admit(1) -> HalfClose(1) ->
Abort(1)`. Pair 65,535 is queued in serving generation 1 while the owner has
consumed zero end records. The former absolute-table lookup selected row 0,
the released phase-1 context, and falsely reported record loss. The corrected
lookup selects serving-suffix row 0, which is complete-table row 1, and
therefore projects the actual queued pair and `endq = [1]`. It does not derive
that outcome from expected model state or copy the diagnostic facts into the
driver.

## Review gates

| Gate | Result | Iteration-59 evidence |
|---|---|---|
| Context identity | PASS | Owner generation selects the exact suffix of serving rows in the retained device history. |
| Phase-1 complement | PASS | Exactly one released row with zero pairs, watches, and queued ends is mandatory. |
| Pair generation | PASS | Existing production generation and pair ID map to serving index `generation - 1`. |
| Queue order | PASS | `endq` traverses serving rows and each row's `queued_ends` in their existing order. |
| Record-loss oracle | PRESERVED | Neither a missing live pair nor an unknown queued ID is accepted or filtered. |
| History | PRESERVED | Released serving rows remain addressable; no context is deleted, rewritten, or synthesized. |
| Tests and model | PRESERVED | No Quint source, action, state, scenario, test body, or bound changed. |
| Surface discipline | PASS | Projection-only test-support logic; no production API, Sim API, seam, or mechanism. |
| Diagnostics | PASS | The temporary seed diagnostics are absent from the reviewed driver. |

The concurrent activation of the eight existing `flow_quota` cells and the
crafter's separate Sim summary optimization are outside this bounded DISTILL
projection disposition. This review does not attribute or re-review those
changes.

## Verification

| Check | Result |
|---|---|
| Source and context-contract inspection | PASS. |
| Generation/index arithmetic | PASS — owner generation 1 maps to the first serving row while complete row 0 remains the phase witness. |
| `rustfmt --edition 2024 --check` | PASS for `quint_flow_quota.rs`. |
| Scoped `git diff --check` | PASS for the driver and handoff before this append. |
| Focused seed/full conformance execution | NOT RUN — this is the requested source-only review. |

## Iteration-59 verdict

**APPROVED — SOURCE ONLY.** The projection now reads pair state and queued end
records from the exact serving-context suffix named by the actual owner
generation. It retains the full device history, strictly validates phase 1,
and preserves every pair identity, queue-order, unknown-record, and
record-loss oracle without changing a test body or model contract.

# APPROVED — SUPPLEMENTAL ITERATION 59 (SOURCE ONLY)

---

# Supplemental review iteration 60 — `flow_quota` disabled-action witness

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-60` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal-source reviewer |
| Scope | `canReadEnd` guard factoring and the existing out-of-order `ReadEnd` witness |
| Verdict | **APPROVED — source only; execution evidence remains pending** |
| Open findings | 0 critical, 0 high, 0 medium |

The new state-reading `canReadEnd(s)` definition is exactly the former
`ReadEnd(s)` guard:

```quint
slot.get(s).st == "ended" and endq.length() > 0 and endq.head() == s
```

`ReadEnd` now invokes that predicate and retains every state update unchanged:
it frees the named slot, removes the queue head, preserves the ghost, and
records the same conformance action. No enabled or disabled transition changes.

The conjunction also retains the original safe evaluation order. The queue
length is checked before `head()`, using the same established Quint guard idiom
already present elsewhere in these specifications. Consequently an empty
queue evaluates the predicate false without requiring a head, while a
non-empty queue permits exactly the prior head-identity comparison.

## Witness equivalence

`EndRecordsAreReadInQueueOrder` reaches the unchanged state in which both
slots are ended and `endq == [2, 1]`. In that state:

- `slot.get(1).st == "ended"` is true;
- `endq.length() > 0` is true; and
- `endq.head() == 1` is false because the head is 2.

The old terminal `.then(ReadEnd(1)).fail()` witnessed that the action guard is
false. The replacement `.expect(not(canReadEnd(1)))` asserts the same false
guard in the identical pre-transition state. There was no successful
`ReadEnd(1)` transition in the old run, so the successful action sequence and
all model states remain identical. The replacement only prevents Quint from
emitting the deliberately disabled action's empty terminal `{}`, which
quint-connect 0.1.2 cannot decode.

## Review gates

| Gate | Result | Iteration-60 evidence |
|---|---|---|
| Guard identity | PASS | All three former conjuncts occur once, in the same order, in `canReadEnd`. |
| Action enabledness | PRESERVED | `ReadEnd` delegates to that exact predicate. |
| Empty-queue semantics | PRESERVED | Length precedes `head()`, so the predicate remains safely false for an empty queue. |
| Queue-order witness | PRESERVED | The run still proves slot 1 is unreadable while queue head 2 precedes it. |
| Terminal state | PRESERVED | The assertion is evaluated before any disabled transition, at the same `[2, 1]` state. |
| Successful transition sequence | PRESERVED | The removed `.fail()` contributed no successful state transition. |
| State and outcomes | PRESERVED | Slot, queue, ghost, conformance-step, invariants, and refusal meaning are unchanged. |
| Formal input scope | PASS | No main, config, action set, scenario name, driver, filter, or bound changed. |

The scoped README sentence accurately records the compatibility reason and
the unchanged guard outcome. Other concurrent README edits concern separate
`fwd_pair` work and are outside this iteration.

## Verification

| Check | Result |
|---|---|
| Source equivalence inspection | PASS. |
| Remaining `flow_quota.qnt` `.fail()` inventory | PASS — none remain. |
| Scoped `git diff --check` | PASS for the specification, README, and handoff before this append. |
| Quint typecheck and exact replays | NOT RUN — mandatory after source approval. |
| Job-3 record and evidence verification | NOT RUN — mandatory after final approved source changes. |

## Iteration-60 verdict

**APPROVED — SOURCE ONLY.** `canReadEnd` is an exact state-reading factor of
the action guard, including non-empty/head evaluation order. The named run
retains the same out-of-order refusal witness and terminal model state while
avoiding only quint-connect's empty-state incompatibility. Typecheck, exact
replays, the final job-3 record, and evidence verification remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 60 (SOURCE ONLY)

---

# Supplemental review iteration 61 — approved decision source package

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-61` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source reviewer |
| Scope | Actual formal, conformance, acceptance and existing-GuestKit source for the seven user-approved decisions |
| Canonical inputs | The four approved decision artifacts and their independent fold reviews, the feature-delta authority row, and `CHCGROUP55RUNNER1`'s exact manifest |
| Verdict | **CHANGES_REQUESTED — four bounded source defects** |
| Open findings | 0 critical, 3 high, 1 medium |

The package correctly implements most of the approved fold. BFIFO1 adds one
model-only FIFO, preserves it through every lifecycle action, checks exact
pending membership and distinctness in `BeaconQueueExact`, includes it in
`allVars`, and removes the worker driver's private accept-order shadow. One
real accept plus the existing post-state projection remains the identity
oracle. ACTPRIORITY1 changes only `ActivatePending`'s guard, and
QUIESCENCEOPEN1 changes only the existing state-reading `canOpenFlow` guard.
The added R5-26 assertions correctly require zero admitted guest-flow state,
zero held places, and zero attaches after the real Quiesced refusal.

The existing-GuestKit cgroup extension also matches its approved packaging.
It adds exactly the three staged libtest entries; boots a fresh guest for each
binary; creates and mounts private cgroup v2 inside that guest; checks the
`cpu`, `memory`, `io`, and `pids` controller tokens before libtest; runs every
approved filter single-threaded with its own required-pass oracle; requires
the manifest total `3 + 45 + 7 = 55`; and powers each successful guest off
before starting the next. It creates no physical-host cgroup or module effect,
changes no selected test body, and adds no runner framework or product API.
The previously reviewed mTLS relay group remains separate from this cgroup
disposition.

Four defects block approval of the combined source package.

## F61-01 — PABORTERR1 changes the wrong transition and leaves the approved transition unchanged

**Severity:** High / blocking  
**Status:** Open  
**Affected source:** `specs/quint/guest-flow-owner/owner_flows.qnt`, public
`GuestDialError` documentation

The approved typed mapping is action-specific. Only `HostRecvAbort(h)` may
change from `Unreachable:io` to `Unreachable:ECONNRESET` when the matching
host dial is still open and its result remains pending. The accepted contract
explicitly retains `HostAbortAccept` as `Unreachable:io` because that separate
action represents another pre-pair host-side `V_h` I/O failure with no exact
modeled errno.

The actual source does the inverse:

- `HostAbortAccept` now assigns `Unreachable:ECONNRESET`; and
- `HostRecvAbort` still assigns `Unreachable:io`.

The strengthened `AcceptedVgErrorAbortsWithAbort` expectation therefore asks
for `ECONNRESET` after an action that still produces `io`, while the unrelated
host-I/O path has been reclassified without authority. This violates both the
exact semantic selector and the requirement to preserve every other dial
result.

The approved public error-documentation update is also absent.
`crates/overdrive-core/src/guest_transport/dial.rs` still describes only a
host vsock connect/I/O failure as `Unreachable { errno }`; it does not record
that a payload-free guest `Abort` received while awaiting `Paired` maps to the
fixed `KernelErrno::ECONNRESET`. The typed-error contract gate requires that
mapping to be explicit at the public interface even though the enum shape is
unchanged.

### Required bounded remediation

Restore `HostAbortAccept` to `dlEndOne(h, "Unreachable:io")`; change only
`HostRecvAbort` to `dlEndOne(h, "Unreachable:ECONNRESET")`; retain the existing
run strengthening; and add the approved exact mapping to the existing public
`GuestDialError::Unreachable` documentation. Add no variant, action, state,
wire payload, driver normalization, or production behavior.

## F61-02 — the Quiesced conformance handler releases bytes before a record handle exists

**Severity:** High / blocking  
**Status:** Open  
**Affected source:** `OwnerFlowsDriver::host_accept_flow`

The driver configures `SimGuestFlowIo` with `set_auto_deliver(false)`.
`release_accept(key)` only marks the queued connection releasable and wakes the
accept future; it does not synchronously turn that queue entry into an accepted
`RecordSocket`. `release_read(key, None)` searches only accepted record handles
and panics when the connection remains queued.

The new Quiesced branch calls those two methods consecutively before its first
`settle`. It therefore reaches the test-support panic
`is not an accepted, unattached connection` instead of driving the production
refusal reader or observing `Refused { Quiesced }`. This is deterministic on
the exact `HostAcceptFlow` path, independent of the pending production
implementation.

### Required bounded remediation

Release the accept and settle until the production accept task has created and
parked its record read; then release the already-buffered complete request and
settle again. Both fixture phases remain inside the same conformance-driver
method and therefore the same formal `HostAcceptFlow` action. Keep the real
outbox delta, exact single `Refused { flow, Quiesced }`, and close/state
comparison; add no new action, barrier API, result shadow, or scenario.

## F61-03 — the approved admission resource, priority and lifecycle checks are missing

**Severity:** High / blocking  
**Status:** Open  
**Affected boundary:** ADMISSIONREFUSAL1 acceptance/source-local evidence

The approved contract requires the existing R5-26/admission cell **and private
owner tests** to prove four classes before implementation acceptance:

1. stable Quiesced refusal with zero admitted state/place;
2. malformed, unsupported and stale-attribution priority;
3. cancellation and no response after teardown, session loss/replacement, and
   root shutdown; and
4. the exact private refusal-reader identity/count, 65,536 cap, atomic
   saturation, lossless completion rearm, and root-shutdown priority.

The package adds only class 1's three state assertions. No new or strengthened
private/source-local check covers classes 2–4. The existing fd-budget cells
also accept any `required > 1_024`; they do not pin the approved exact
`GuestNetworkError::FdBudget.required == 376_832`, so the former 65,536 value,
the 311,296 pre-decision baseline, and the approved target all satisfy the
current oracle.

This is a completeness defect in DISTILL evidence, not authority for DISTILL
to edit the crafter-owned reader/cap/fd production implementation. Add the
approved private/source-local checks and strengthen an existing fd-budget
oracle to the exact target. Preserve the existing scenario-id inventory and
public seams; the canonical decision deliberately assigns these checks to
private test support.

## F61-04 — the README retains a contradictory unqualified D-UP-19 summary

**Severity:** Medium / blocking documentation consistency  
**Status:** Open  
**Affected source:** `specs/quint/guest-flow-owner/README.md`

The detailed `owner_flows` section correctly records BFIFO1 as the bounded
beacon exception. The README's opening contract inventory still says D-UP-19
means “no accept order” and “any pending connection is accepted” without that
exception. That statement is false for the newly guarded
`HostAcceptBeacon(b)` and contradicts the accepted replacement text in the
same file.

Update the opening summary to retain arbitrary accept order only for
order-insensitive owner models and name the BFIFO1 beacon exception. Do not
broaden FIFO modeling to another listener or change reclamation ordering.

## Approved subset audit

| Decision/boundary | Result | Iteration-61 evidence |
|---|---|---|
| `FLOWQUOTA240` | PASS | Exact one-test 60-second × 4 override; eight samples and body unchanged. |
| `BFIFO1` model state/actions | PASS | Empty/append/head-tail/preserve effects are complete; no generation rewrite or non-head removal. |
| `BeaconQueueExact` / `allVars` | PASS | Domain, distinctness, pending iff membership, safety inclusion and fairness tuple are exact. |
| Beacon conformance identity | PASS | Private `accept_order` is removed; actual accepted identity remains observable through existing projection. |
| `ACTPRIORITY1` | PASS | Only `ActivatePending` gains `not(quiesced)`; Quiesced has priority. |
| `QUIESCENCEOPEN1` | PASS | `canOpenFlow` additionally requires undamaged fixed C1/FlowListeners. |
| `ADMISSIONREFUSAL1` model action | PASS | No new formal state/action/run; Quiesced still closes in existing `HostAcceptFlow`. |
| R5-26 zero-state strengthening | PASS | Exact refusal, close, empty `guest_flows`, zero `held_flows`, zero attaches. |
| `PABORTERR1` | **FAIL** | Result changed on `HostAbortAccept`; required `HostRecvAbort` remains generic; public mapping doc absent. |
| Quiesced driver stimulus | **FAIL** | Read release occurs before accept effect completion and panics. |
| Admission evidence completeness | **FAIL** | Required private priority/lifecycle/cap/rearm checks and exact fd oracle are absent. |
| `CHCGROUP55RUNNER1` staging | PASS | Exactly three new cgroup binaries use existing staging paths/features. |
| `CHCGROUP55RUNNER1` guest boundary | PASS | Three fresh guests, guest-only cgroup2 mount, exact controller preflight and cleanup. |
| `CHCGROUP55RUNNER1` manifest | PASS | Exact approved filters and per-filter counts total 55; extras or failures cannot silently pass. |
| New surface/lane/scenario | PASS for approved subset | No product API, formal action, runner framework, or new lane; only approved private packaging. |

## Mechanical verification

| Check | Result |
|---|---|
| Canonical proposal/fold comparison | PASS for the approved subset; failures above are direct divergences from the accepted text. |
| BFIFO lifecycle assignment inventory | PASS — every existing action either initializes, mutates, or preserves `bq` as approved. |
| Cgroup filter/pass-count comparison | PASS — constants match the canonical 3/45/7 table exactly. |
| `rustfmt --edition 2024 --check` | PASS for the reviewed Rust source files. |
| Scoped `git diff --check` | PASS before this append. |
| Quint typecheck, exact seeds, named/randomized targets | NOT RUN; blocked on source remediation. |
| Cgroup 55-body guest execution | NOT RUN; blocked on source review. |
| Three-job formal record / evidence verification | NOT RUN; mandatory after final approved source. |

## Iteration-61 verdict

**CHANGES_REQUESTED.** BFIFO1, ACTPRIORITY1, QUIESCENCEOPEN1,
FLOWQUOTA240, the R5-26 zero-state assertions, and the exact cgroup GuestKit
packaging pass source review. PABORTERR1 is applied to the wrong action, the
Quiesced handler deterministically releases a read before accept completion,
the accepted admission resource/priority/lifecycle evidence is incomplete,
and the README retains one contradictory D-UP-19 summary. Remediate only those
bounded defects and return for source re-review. No execution or evidence
record is authorized by this verdict.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 61

---

# Supplemental review iteration 62 — iteration-61 remediation and private-test proposal

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-62` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/test-design reviewer |
| Scope | F61-01/02/03/04 remediation plus the unimplemented one-body private ADMISSIONREFUSAL1 proposal |
| Verdict | **CHANGES_REQUESTED — corrected source passes; private test proposal does not yet prove production rearm** |
| Open findings | 0 critical, 1 high, 0 medium |

## Corrected-source disposition

### F61-01 — resolved

The formal typed-result mapping now matches PABORTERR1 exactly:

- `HostAbortAccept` is restored to generic `Unreachable:io`;
- only `HostRecvAbort` fixes the still-pending dial result to
  `Unreachable:ECONNRESET`;
- the existing `AcceptedVgErrorAbortsWithAbort` run requires that exact result;
  and
- the existing public `GuestDialError::Unreachable` documentation records the
  fixed mapping for a payload-free pre-Paired guest `Abort`, while retaining
  host-observed errnos for host operations.

No action identity, wire payload, variant, cleanup effect, driver
normalization, or production behavior changed.

### F61-02 — resolved

The Quiesced `HostAcceptFlow` driver path now releases accept, settles until
the production accept task owns an accepted record handle and has parked its
read, releases the complete buffered request, and settles again. Those two
fixture phases remain inside the one conformance action. The unchanged outbox
delta still requires exactly one production-emitted
`Refused { flow, Quiesced }`; no result is injected or shadowed.

### F61-04 — resolved

The README's opening D-UP-19 inventory now states the order-insensitive
default, names BFIFO1's bounded beacon FIFO exception, and separately retains
unordered boot reclamation. It agrees with the detailed `owner_flows` section
and the approved replacement text.

### F61-03 — existing-body portion resolved

The current existing bodies now cover the parts they can reach without
changing subjects:

- R5-26 requires the stable exact Quiesced refusal, owner close, empty
  `guest_flows`, zero `held_flows`, and zero attaches;
- the stale-token body enters quiescence first and still requires a silent
  `StaleLaunch`, proving attribution before the snapshotted refusal;
- the existing malformed body runs while Quiesced and requires both exact
  `Malformed` and `Unsupported` messages for attributable requests, proving
  decode classification before the snapshotted refusal; and
- the source-local startup body pins
  `FdBudget { required: 376_832, available: 1_024 }` exactly.

The original scenarios and their retained assertions remain present. Adding
the related unsupported row to the existing decode-priority body is a dense
parameter-equivalent strengthening, not a new scenario id or lane.

## F62-01 — proposed cap/rearm section performs the wake it is meant to verify

**Severity:** High / blocking  
**Status:** Open  
**Affected proposal:**
`owner_runtime::admission_refusal_readers_are_bounded_rearmed_and_cancelled_without_cross_lifecycle_response`

The proposed single private body is correctly located and scoped. A
source-local current-thread paused Tokio test can access the existing private
owner state, `FlowTask` classification, stream accept loop, cancellation
tokens and `Notify` without adding a public seam. One parameterized body is
also the smallest reasonable count for the remaining approved behaviors.

Its cap/rearm stimulus is not yet an honest oracle, however. The proposal
says the test will remove a synthetic tracked entry through
`OwnerState::remove_flow_task` **and publish `refusal_reader_capacity` itself**.
Those are the exact two production effects whose completion/rearm contract the
test must verify. A production implementation that removes an entry without
decrementing through the counted method, or decrements without publishing the
wake, can still pass that section because the test supplies both effects.

This is not hypothetical. In the current in-progress production source,
`finish_flow_task` removes directly from `entry.flow_tasks` and contains no
capacity notification. A cap/rearm test that manually calls the counted
removal and `notify` would make that broken ordinary-completion path appear
correct. It would test the private primitives in isolation while missing their
required production composition.

The proposed identity claim is also under-specified. It creates only positive
`owner_launch = Some`, `flow = None` fillers. It would not reject a mutation
that also counts an ordinary pre-decode admitted task (`None`, `None`) or a
decoded datagram task (`None`/`Some`, `Some(flow)`). Exact reader identity
requires positive and negative table cases.

Finally, “release the request at the same boundary” is not an executable
ordering for the session-loss case as written. Making the Sim session EOF
ready does not yet make the refusal reader's cancellation token ready; the
control-session task must first process that EOF. Releasing request bytes
before that owner step can legitimately let the read win. A race oracle must
establish that lifecycle cancellation and read are both ready before the
reader is next polled, or avoid claiming a simultaneous-race proof.

### Required proposal remediation

Keep one body and the private/no-seam boundary, but make these exact changes
before implementation authority is claimed:

1. Synthetic `task = None` entries may compress the immutable 65,536
   precondition, but they must be filler only. At the cap, free capacity by
   completing or cancelling an **actual production refusal-reader task**
   through its real owner path. The queued second connection must be accepted
   only because that production completion both updates the count and
   publishes the existing wake. The test must never invoke `Notify` to make
   this rearm pass.
2. Add a local classification table covering at least the positive
   `(owner_launch = Some, flow = None)` case and the ordinary pre-decode and
   decoded-flow negative shapes. Assert exact count deltas on insert,
   replacement and removal so neither undercount nor overcount survives.
3. Use the real teardown, session-loss/replacement and root-shutdown owners for
   the three lifecycle cases. Require task completion, count zero, owner close,
   zero admitted state/place and no old/replacement-session response.
4. Where biased cancellation is asserted, first prove the task cancellation
   token is ready through the lifecycle owner, then make the held record read
   ready before allowing the reader to poll. Do not treat an unprocessed Sim
   EOF as completed session-loss cancellation.
5. Test shutdown priority separately with both the existing root token and a
   capacity wake ready; here a test-issued notification is appropriate because
   the subject is the `biased` wait selection, not production publication.
6. Remove every filler through the counted state method during cleanup, but do
   not cite that cleanup as evidence for ordinary task-completion rearm.

These corrections require no new production helper, API, Sim seam, action,
scenario id or lane. They keep the exact proposed Contract Shape declaration.

## Authority disposition

No additional user decision is required merely to author the corrected one
body. ADMISSIONREFUSAL1 was explicitly approved with the recorded validation
cost: extend the existing admission cell **and private owner tests**, without a
new scenario id or public seam, for priority, lifecycle cancellation, and the
private reader-count/backpressure boundary. Its independent design review also
states that the exact private-test count follows that approved decision.

The proposed body stays within that authority. The current blocker is its
technical oracle, not missing product or test-surface authority. If remediation
introduces a new seam, public hook, scenario id, lane, or different production
mechanism, that expansion would require a new decision; none is necessary for
the bounded correction above.

## Verification

| Check | Result |
|---|---|
| F61-01 formal/result/doc mapping | PASS. |
| F61-02 accepted-record ordering | PASS by source inspection. |
| F61-04 README reconciliation | PASS. |
| Existing admission priority assertions | PASS by source inspection. |
| Exact source-local fd-budget oracle | PASS — `376_832`. |
| Proposed private test boundary/count | PASS — one body, private helpers/cases, no scenario id or seam. |
| Proposed production rearm oracle | **FAIL** — test manually supplies counted removal and notification. |
| Proposed exact identity coverage | **FAIL** — negative task shapes are not asserted. |
| Proposed simultaneous cancellation ordering | **FAIL** — session EOF readiness is not cancellation readiness. |
| `rustfmt --edition 2024 --check` | PASS for the corrected reviewed Rust files. |
| Scoped `git diff --check` | PASS before this append. |
| Test/formal/Metal execution | NOT RUN; blocked pending proposal remediation and authored-source re-review. |

## Iteration-62 verdict

**CHANGES_REQUESTED.** F61-01, F61-02 and F61-04 are closed, and the existing
admission bodies now honestly cover stable refusal, attribution/decode
priority and the exact fd budget. F61-03 remains open only for the proposed
private cap/rearm and lifecycle body. Its scope and one-body shape are valid,
but its current rearm stimulus performs the production wake itself and its
identity/race oracles are incomplete. Correct the proposal as bounded above;
do not seek another user approval for validation already included in the
approved ADMISSIONREFUSAL1 contract.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 62

---

# Supplemental review iteration 63 — actual private admission validation body

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-63` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/test-design reviewer |
| Scope | The one authorized private admission validation body and its test-local helpers |
| Verdict | **CHANGES_REQUESTED — one negative classification fixture is not a real production task shape** |
| Open findings | 0 critical, 1 high, 0 medium |

The authored body closes the central iteration-62 defects. Its cap/rearm
proof now uses production composition rather than supplying the result:

- test-only filler entries take the actual counter to 65,535;
- the first real Quiesced `spawn_flow` task reaches exactly 65,536 under the
  owner-state lock;
- the second released connection remains in the Sim accept queue;
- completing the first actual reader runs the production
  `finish_flow_task`, counted removal, and capacity notification;
- only that completion permits the second connection to become an accepted,
  classified, read-pending refusal reader; and
- with the second request and root cancellation ready together, the reader's
  biased cancellation closes without response, and the accept loop's biased
  shutdown leaves the third connection unaccepted despite the production
  completion notification.

Manual synthetic removal happens only after root shutdown, when no capacity
waiter exists, and is used solely for cleanup. It is no longer cited as rearm
evidence. The teardown and session-EOF/replacement cases also use fresh
production compositions and actual owner ports; each proves reader-count
removal, owner close, and absence of a response on either the old or
replacement session. The body carries the exact Contract Shape line and adds
no production helper, public hook, Sim seam, model action, scenario id, lane,
or timeout override.

## F63-01 — the purported decoded-datagram negative uses an unreachable owner-launch shape

**Severity:** High / blocking test honesty  
**Status:** Open  
**Affected helper:** `seed_refusal_readers_to_one_below_cap`

The exact production task partition is already pinned and implemented:

| Production `flow_tasks` class | `owner_launch` | `flow` | Refusal reader |
|---|---:|---:|---:|
| Ordinary admitted stream before decode | `None` | `None` | No |
| Ordinary decoded datagram | `None` | `Some(flow)` | No |
| Non-admitted refusal reader | `Some(snapshot)` | `None` for its lifetime | Yes |

The new helper correctly inserts the ordinary stream as `(None, None)`, but
its entry named `datagram` is `(Some(lease.launch), Some(flow))`. No production
path creates that combination: ordinary admitted tasks retain
`owner_launch = None` when `set_datagram_awaiting_paired` sets their `flow`,
while refusal readers never acquire a `flow`.

This makes the negative case a fabricated truth-table row rather than the
promised actual decoded-datagram state. It directly tests the private Boolean
expression against an unreachable combination and would allow the review to
claim production-shape coverage that the body did not execute.

### Required bounded remediation

Construct the decoded-datagram negative with
`owner_launch: None, flow: Some(guest_flow(99))`, matching the actual admitted
task transition. Retain the ordinary `(None, None)` and positive
`(Some(launch), None)` rows, exact count assertions, real cap/rearm path, root
priority, lifecycle cases, cleanup and one-body count. Do not add another test
or production seam merely to cover an unreachable fourth truth-table row.

## Review gates

| Gate | Result | Iteration-63 evidence |
|---|---|---|
| User/test-body authority | PASS | ADMISSIONREFUSAL1 explicitly authorizes private owner validation without a new scenario id or public seam. |
| One-body density | PASS | Cap/rearm/root priority and lifecycle cases share one parameterized body. |
| Positive reader identity | PASS | Actual refusal entries use `Some(snapshot)` / `None`. |
| Ordinary stream exclusion | PASS | Actual pre-decode shape `None` / `None` does not increment. |
| Decoded datagram exclusion | **FAIL** | Fixture uses `Some(snapshot)` / `Some(flow)`, not production's `None` / `Some(flow)`. |
| Exact cap | PASS | Actual first reader reaches 65,536; count never projects above it. |
| Production completion/rearm | PASS | First actual task completion is the sole wake that accepts the second. |
| Root shutdown priority | PASS | Ready cancellation beats ready read and subsequent capacity wake; third remains queued. |
| Teardown lifecycle | PASS | Real teardown cancels/awaits, removes count, closes, and emits no response. |
| Session replacement lifecycle | PASS | Real EOF cancellation completes before replacement; no response crosses session identity. |
| Synthetic cleanup isolation | PASS | Counted removal occurs only after shutdown and is not rearm evidence. |
| Contract Shape / surface | PASS | Exact marker; test-local helpers only; no API/seam/action/lane/timeout change. |

## Verification

| Check | Result |
|---|---|
| Source-path and owner-state inspection | PASS except F63-01. |
| Production `FlowTask` construction/mutation inventory | PASS — confirms the three reachable shapes above. |
| `rustfmt --edition 2024 --check` | PASS for `owner_acceptance/admission.rs`. |
| Scoped `git diff --check` | PASS before this append. |
| Compile, focused body and exact/formal replays | NOT RUN; mandatory after source approval. |

## Iteration-63 verdict

**CHANGES_REQUESTED.** The actual body now honestly proves real
completion-driven capacity rearm, root-shutdown priority and the two lifecycle
cancellation paths. Approval is blocked only by the decoded-datagram negative,
which fabricates `owner_launch = Some` instead of using the production
`None`/`Some(flow)` shape. Correct that one fixture row and return for bounded
source re-review; no additional user decision is required.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 63

---

# Supplemental review iteration 64 — decoded-datagram fixture remediation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-64` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/test-design reviewer |
| Scope | F63-01 tuple remediation only |
| Verdict | **APPROVED — source only; execution remains mandatory** |
| Open findings | 0 critical, 0 high, 0 medium |

F63-01 is resolved. The private classification setup now uses the three exact
reachable production `flow_tasks` shapes:

| Task class | `owner_launch` | `flow` | Count effect |
|---|---:|---:|---:|
| Ordinary pre-decode admitted stream | `None` | `None` | No increment |
| Ordinary decoded datagram | `None` | `Some(guest_flow(99))` | No increment |
| Non-admitted refusal reader | `Some(lease.launch)` | `None` | Increment |

Both negative entries leave the seeded count at 65,535, while the real first
refusal reader remains the sole transition to 65,536. The tuple fix changes no
cap/rearm stimulus, actual task completion, production notification, root
shutdown priority, lifecycle case, response assertion, cleanup, helper
surface, Contract Shape, scenario id, lane, API, seam, or timeout.

## Iteration-63 finding disposition

| Finding | Disposition | Evidence |
|---|---|---|
| F63-01 | **RESOLVED** | The decoded-datagram negative is now the actual `None` / `Some(flow)` production shape. |

## Verification

| Check | Result |
|---|---|
| Exact tuple/source comparison | PASS. |
| Retained count assertion | PASS — both negative inserts leave `GUEST_FLOW_MAX_PAIRS - 1`. |
| No unrelated private-body change | PASS by scoped source inspection. |
| `rustfmt --edition 2024 --check` | PASS for `owner_acceptance/admission.rs`. |
| Scoped `git diff --check` | PASS before this append. |
| Compile and focused body execution | NOT RUN — mandatory after source approval. |
| Exact named/randomized/formal replay | NOT RUN — remains part of the later evidence gate. |

## Iteration-64 verdict

**APPROVED — SOURCE ONLY.** The one private ADMISSIONREFUSAL1 body now uses
only reachable task classifications and retains its honest production-driven
cap, rearm, shutdown-priority and lifecycle oracles. Actual compilation and
execution of the focused body, followed by the required exact and full formal
evidence, remain mandatory before completion is claimed.

# APPROVED — SUPPLEMENTAL ITERATION 64 (SOURCE ONLY)

---

# Supplemental review iteration 65 — concurrent refusal response-order oracle

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-65` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/test-design reviewer |
| Scope | Existing malformed/unsupported refusal body's response collection oracle |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |

The adjusted oracle matches the approved concurrent refusal-reader contract.
The two sockets are accepted into independent tasks and neither request is
allowed to head-of-line block the other. No contract orders their control
responses relative to each other, so requiring the opening order
`Malformed(flow 1), Unsupported(flow 2)` was an invalid cross-reader ordering
constraint. The reproduced completion order
`Unsupported(flow 2), Malformed(flow 1)` is therefore valid.

The replacement does not weaken message identity, payload, or cardinality:

- the outbox must contain exactly two messages;
- exactly one must equal
  `Refused { flow: flow 1, refusal: Malformed }`; and
- exactly one must equal
  `Refused { flow: flow 2, refusal: Unsupported }`.

Together those assertions reject every missing, duplicate, extra, wrong-flow,
wrong-variant, or wrong-payload result. They merely accept either permutation
of the two exact messages. The body does not sort or normalize production
output, and it adds no wait, scheduler control, ordering mechanism, API, seam,
scenario, lane, or timeout.

The retained assertions still require both sockets closed, zero registered
datagrams, zero held places, zero peer opens, and successful hold restoration.
Malformed/Unsupported decode priority over the snapshotted Quiesced refusal is
therefore unchanged.

## Verification

| Check | Result |
|---|---|
| Exact cardinality | PASS — `len() == 2`. |
| Malformed identity/payload | PASS — exact count one for flow 1. |
| Unsupported identity/payload | PASS — exact count one for flow 2. |
| Cross-reader order | Correctly unconstrained. |
| Zero-effect/state oracles | PRESERVED. |
| Contract Shape | PRESERVED — `unbounded-preservation`. |
| `rustfmt --edition 2024 --check` | PASS for `owner_acceptance/admission.rs`. |
| Scoped `git diff --check` | PASS before this append. |
| Independent execution | Not rerun by this review; the supplied focused Metal receipt is recorded in the handoff. |

## Iteration-65 verdict

**APPROVED.** The existing body now checks the exact unordered multiset of two
concurrent refusal responses. It removes only an unapproved cross-reader
completion order and preserves every accepted priority, identity, payload,
cardinality, close, zero-state and zero-effect outcome.

# APPROVED — SUPPLEMENTAL ITERATION 65

---

# Supplemental review iteration 66 — RELEASEPROJ1 source alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-66` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Private `owner_flows` allocation/generation projection, cleanup-tombstone validation, and README fold |
| Authority | User-approved `RELEASEPROJ1`; independently approved canonical fold |
| Verdict | **APPROVED — source only** |
| Open findings | 0 critical, 0 high, 0 medium |

The source implements the accepted QPROJ-2-first projection without changing
the formal model or production runtime. `OwnerFlowsDriver::project` reads the
real pool snapshot before assigning `alloc` or its compared generation and
implements all four approved cases:

1. an absent pool lease leaves `alloc = Absent` and `lease_gen = None`;
2. a Retiring pool lease must retain the driver's exact CID and launch, and
   projects `Retiring` with that pool launch;
3. an Admitted pool lease must retain the driver's exact CID and launch and
   requires a matching QPROJ-3 allocation/CID/launch row in `Provisioned` or
   `Active`; and
4. a missing or mismatched row, or `Retiring` owner phase under an Admitted
   lease, fails conformance instead of being normalized.

The comparison-side `OwnerFlowsState` continues to omit generation only when
the model's `alloc` is `Absent`, so pool absence and formal `Release` remain
congruent. No owner row can manufacture a post-release lease or generation,
and no driver action history supplies either fact.

## Cleanup-tombstone audit

Whenever an owner row remains while the pool lease is absent or Retiring, the
new private validator requires the exact approved fail-closed cleanup shape:

- public and private allocation, CID, launch and `Retiring` phase match the
  driver's old lease;
- public and private session authority is absent, including sender and
  cancellation ownership;
- public `unread` equals private `unread`;
- public and private guest flows, flow tasks/ids, dials, reply/cancel/request
  auxiliaries, datagrams and held-flow count are empty;
- no private pair or dial watch belongs to the CID;
- `control_tasks` has exactly the same ordered key set as `unread`; and
- every retained control task keeps `owner_launch = Some(old launch)`.

The validator deliberately ignores private non-resource result caches, as the
accepted contract requires. Those caches cannot authorize admission or retain
a flow place. The production source independently removes a Retiring row when
its final accepted-unread task finishes; while one remains, public `unread`
continues to project the existing session as `reading`.

This validation does not hide serving residue. A retained flow, task, flow ID,
dial auxiliary, datagram, pair, watch, held place, session authority, identity
mismatch, or non-Retiring row is an error. Conversely, an owner row already
removed while the pool still holds a Retiring lease remains valid because
QPROJ-2 alone owns the lease lifecycle fact.

## Same-CID generation safety

An Admitted replacement lease must match the new public owner row exactly.
Old accepted-unread keys remain visible through that row's `unread`, so the
existing session projection remains `reading`; the driver does not relabel or
copy their generation. Current production transfers the old `control_tasks`
without changing `owner_launch`, and the existing late-read/finish path checks
the accepted launch before claiming, clearing a session, removing a task, or
removing a Retiring row. The unchanged stale-launch and teardown/read actions
therefore remain the behavioral safeguard for the replacement boundary.

No model state, action, run, guard, check, trace filter, product behavior,
release callback, owner, public observation, API, wire shape, scenario, or
lane was added. The README accurately records pool authority, the strict
cleanup row, continuing `reading` projection, and generation-safe reuse.

## Review gates

| Gate | Result | Iteration-66 evidence |
|---|---|---|
| Pool absence | PASS | Projects `Absent` with no compared generation, including when the exact cleanup row remains. |
| Retiring pool lease | PASS | Exact CID/launch required; pool launch supplies the compared generation. |
| Admitted pool lease | PASS | Exact CID/allocation/launch and `Provisioned`/`Active` owner row required. |
| Invalid Admitted combinations | PASS | Missing, mismatched, or owner-`Retiring` rows fail closed. |
| Tombstone identity/phase | PASS | Public/private row must equal the old allocation, CID, launch and `Retiring` phase. |
| Session authority | PASS | Public session plus private session/sender/cancel must all be absent. |
| Forwarding residue | PASS | Flows, tasks, ids, dials/auxiliaries, datagrams, pairs, watches and held count must be zero. |
| Accepted-unread ownership | PASS | Public/private unread agree; control-task keys equal unread and retain the old launch. |
| Reading projection | PRESERVED | Still comes from the actual public unread set. |
| Same-CID replacement | PRESERVED | New lease/row identity is exact; old task identity and late-finish safeguards remain production-owned. |
| Formal/runtime/API surface | PRESERVED | Projection/documentation only. |

The concurrent GuestKit module-staging dependency order remains packaging
inside the already approved disposable-guest runner. It introduces no host
kernel operation or RELEASEPROJ1 mechanism and is outside this projection
change.

## Verification

| Check | Result |
|---|---|
| Canonical lines `9589–9637` and `10432–10433` comparison | PASS. |
| Approved design and fold-review comparison | PASS. |
| Current private owner lifecycle/late-finish source inspection | PASS. |
| README contract alignment | PASS. |
| `rustfmt --edition 2024 --check` | PASS for `quint_owner_flows.rs`. |
| Scoped `git diff --check` | PASS before this append. |
| Exact seed `0xf23a4188` and unchanged owner-flow targets | NOT RUN — mandatory after source approval. |
| Three-job record, `verify-evidence`, independent proof review | NOT RUN — mandatory final evidence gates. |

## Iteration-66 verdict

**APPROVED — SOURCE ONLY.** RELEASEPROJ1 now derives lease presence and
compared generation exclusively from QPROJ-2, admits a retained QPROJ-3 row
only as the exact fail-closed accepted-unread cleanup tombstone, and preserves
the existing `reading` and same-CID generation-safety behavior. Exact-seed,
unchanged-target, full-record and proof evidence remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 66 (SOURCE ONLY)

---

# Supplemental review iteration 67 — ordinary non-watch owner-abort closure

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-67` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Existing `fwd_pair.ownerAbort` non-watch branch and strengthened existing run |
| Authority | Existing accepted owner-abort/descriptor-close contract; no new decision required |
| Verdict | **APPROVED — source only** |
| Open findings | 0 critical, 0 high, 0 medium |

The Trace-51 correction matches the already accepted ordinary owner-abort
contract. The reproduced action is a live non-watch pair whose production
owner runs `abort_all`, queues the `Detached` record, then closes both owned
descriptors. QPROJ-8 consequently reports the owner row absent, shared V_h
closed, sent shutdown `{R,S}`, and `sshut = true`. Only a watch owned by a
`GuestStream` keeps V_h untouched.

The formal non-watch branch now performs:

```quint
val t1 = abortSide(fault(t), s, "Detached", false)
shutAll(setPr(t1, s, { ...t1.pr.get(s), closed: true }), s)
```

This adds exactly the missing `pr.closed = true` fact. The outer `shutAll`
does not add a second socket effect on any reachable non-watch `Abort` branch:

- `Abort(s)` requires `live(cur, s)` and `sideUp(cur, s)`;
- for every non-watch live pair, `abortSide` reaches its non-watch branch;
- that branch always ends by calling `shutAll(t4, s)`, for stream and datagram
  modes and for both sides;
- `shutAll` either preserves an already-`both` socket or calls `send(...,
  both)`, after which `sent == both` and `sshut == true`; and
- the second `shutAll` therefore takes its no-op arm after `setPr` changes
  only the pair's `closed` field.

It emits no duplicate control packet, changes no queue, direction, endpoint,
record outcome/cause/count, shutdown flag, ghost, fault count, or guest Abort.
The redundant close complement is an idempotent expression of the same
normative descriptor close, not a new effect beyond the DESIGN author's
narrow `closed` correction.

Both watch branches retain their distinct ownership rules. A pre-handoff
pending dial still closes owner-held V_h through its already approved branch;
a held `GuestStream` still queues the watch record while leaving holder-owned
V_h untouched and setting `strmAbort`. `WatchAbortLeavesHolderSocket` and the
watch-only no-close clauses are unchanged.

The existing `OwnerAbortResetsBothFarEnds` run is strengthened in place to
require the already-produced immediate facts: `closed`, `sent == both`, and
`sshut`. It retains `Detached`, one queued record and the far-end reset
sequence. No run, scenario, action, state, API, seam, production behavior, or
new mechanism was added. The README now states the same ordinary-pair versus
watch ownership distinction.

## Review gates

| Gate | Result | Iteration-67 evidence |
|---|---|---|
| Reproduced Trace 51 mapping | PASS | Non-watch host owner abort requires the actual descriptor-close fact. |
| Record order/class | PRESERVED | `abortSide` still queues one `Detached` record before the same-step close. |
| `pr.closed` | PASS | Set true only on the non-watch owner-abort branch. |
| Outer `shutAll` effects | IDEMPOTENT | Inner `abortSide` has already produced `sent == both` and `sshut`. |
| Far-end resets | PRESERVED | Existing `abortSide` reset/cut effects remain unchanged. |
| Watch pre-handoff branch | PRESERVED | Existing owner-held close behavior unchanged. |
| Watch held-stream branch | PRESERVED | Holder-owned V_h remains untouched. |
| Existing run oracle | STRENGTHENED | Adds exact closed/sent/sshut facts without a new run. |
| Formal/runtime surface | PRESERVED | No state/action/scenario/API/seam/production edit. |

## Verification

| Check | Result |
|---|---|
| Canonical owner-abort clauses `5886–5891`, watch clauses `4877–4896` / `5155–5166` | PASS. |
| `abortSide` / `shutAll` reachability and idempotence inspection | PASS for every non-watch mode and side. |
| Existing run and README comparison | PASS. |
| Scoped `git diff --check` | PASS before this append. |
| Quint typecheck, exact seed and full FwdPair targets | NOT RUN — mandatory after source approval. |
| Three-job record and evidence verification | NOT RUN — mandatory final gates. |

## Iteration-67 verdict

**APPROVED — SOURCE ONLY.** The ordinary non-watch owner abort now records
the descriptor closure required by the existing contract. Its outer
`shutAll` is a no-op after `abortSide` on every reachable non-watch branch, so
the correction introduces no unauthorized socket effect and leaves both
watch ownership branches unchanged. Exact-seed and full formal evidence remain
mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 67 (SOURCE ONLY)

---

# Supplemental review iteration 68 — CONTROLOPENRESET1 source alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-68` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Existing `owner_flows.HostReadOpen(c)` branch partition only |
| Authority | User-approved `CONTROLOPENRESET1`; canonical fold and technical review complete |
| Verdict | **APPROVED — source only** |
| Open findings | 0 critical, 0 high, 0 medium |

The formal source implements the exact approved stable boundary for a queued
current-launch `ControlOpen` whose reset was already attached at VMM death.
`HostReadOpen(c)` retains its guard and check order, then partitions the
existing result exactly:

1. invalid live-token comparison closes this connection without changing the
   current holder;
2. an already-held claim closes this connection and retains that holder;
3. the `BUG_SESSION_SPLIT_CLAIM` hazard retains its existing `checked`
   intermediate with no claim;
4. a valid/free connection whose writer is up becomes `live` and takes claim
   `c`; and
5. a valid/free connection whose writer is down applies the existing total
   `sessionLoss(c)` effects in this same action.

Moving `keepSessRest` into the first four branches is mechanically necessary
and semantically neutral. Each branch assigns the same `sess`, `claim`, and
`heldByOther` values as before, while `keepSessRest` preserves allocation,
generation, VMM state, quiescence, flows, host/guest dial state, beacon state,
activation ghosts, and the guest-held session. No branch receives an extra
message or cleanup effect.

The writer-down branch intentionally does not call `keepSessRest`, because its
approved result is the already-defined session-loss composition:

- close session `c`;
- retain claim 0 (and clear it if the helper is reached from a held session);
- apply `BUG_NO_ABORT_ON_LOSS` exactly as the existing fault variant defines;
- otherwise abort registered guest flows and host dials with the existing
  `NoSession` result;
- clear the existing in-flight paired/refused/guest-abort/host-abort message
  sets; and
- preserve all unrelated lifecycle, guest, beacon, ghost, and result-cache
  facts through the existing helper actions.

At this branch's valid/free pre-state, those total effects equal the
production sequence that briefly claims the valid open, immediately reads the
already-ready reset, and completes `session_lost` before the driver's settle
boundary. The model exposes only the stable `closed` / claim-0 state; it adds
no premature token rejection and no barrier around the transient claim.

`VmStop` remains unchanged: a session already live becomes `dead`, while a
connection already in `reading` at VMM death closes there. The separately
driven `HostDeadSessionLoss` remains unchanged and necessary for the former.
K-A2-prime queued-child retention, stale launch isolation, held-claim
protection, the split-claim and no-abort fault variants, and all existing
checks/runs remain intact.

## Review gates

| Gate | Result | Iteration-68 evidence |
|---|---|---|
| `HostReadOpen` guard | PRESERVED | Still requires `sess(c).st == reading`. |
| Token precedence | PRESERVED | Invalid token closes before claim or writer checks. |
| Held-claim precedence | PRESERVED | Existing holder survives; new connection closes. |
| Split-claim hazard | PRESERVED | Fault branch remains before writer liveness and keeps its exact state. |
| Valid writer-up branch | PRESERVED | `live` with claim `c`. |
| Valid writer-down branch | PASS | Reuses total `sessionLoss(c)`, ending `closed` with claim 0. |
| NoSession cleanup | PASS | Existing flow/dial/message effects are composed unchanged. |
| Existing fault variants | PRESERVED | `BUG_NO_ABORT_ON_LOSS` and split-claim semantics remain in their existing helpers/branches. |
| Live-session VMM death | PRESERVED | `VmStop -> HostDeadSessionLoss` remains separate. |
| Already-reading VMM death | PRESERVED | Existing `VmStop` close branch unchanged. |
| Formal/runtime/API surface | PRESERVED | No state/action/run/check/driver/Sim/production/API/wire change. |

The README action map already records this exact partition and needs no
additional source change.

## Verification

| Check | Result |
|---|---|
| Canonical authority/fold lines `754`, `10459`, `10475` comparison | PASS. |
| Approved design and independent technical review comparison | PASS. |
| Full branch-assignment and helper-effect audit | PASS. |
| Scoped `git diff --check` | PASS before this append. |
| Exact compile-time seed `0x6539f0dd` | NOT RUN — mandatory after source approval. |
| All 46 unchanged owner-flow targets | NOT RUN — mandatory. |
| Final three-job record / `verify-evidence` / hash and proof review | NOT RUN — mandatory final gates. |

## Iteration-68 verdict

**APPROVED — SOURCE ONLY.** CONTROLOPENRESET1 changes only the existing
valid/free writer-down `HostReadOpen` result to the production-observable
stable session-loss state. Invalid-token, held-claim, split-claim, writer-up,
live-session death and already-reading death semantics remain unchanged, and
the existing total cleanup introduces no widened message or typed-result
effect. Exact-seed and full replacement evidence remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 68 (SOURCE ONLY)

---

# Supplemental review iteration 69 — generation-isolated VMM stop and abort-callback run

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-69` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/conformance reviewer |
| Scope | Two private conformance-driver corrections: owner-flow `VmStop` unread delivery and FwdPair abort-class `VCtl(gh)` completion |
| Verdict | **APPROVED — source only** |
| Open findings | 0 critical, 0 high, 0 medium |

## Owner-flow `VmStop` generation isolation

The `VmStop` handler now snapshots the exact stopped lease from `vm_gen`, then
retains the existing CID-wide Sim VMM-death stimulus. That distinction is
correct: device death is CID-wide at the socket seam, while delivery of an
accepted connection's peer-gone callback belongs to the process launch that
owned that accepted control task.

The handler reads public QPROJ-3 `unread` and maps every key to the actual
private `control_tasks[key].owner_launch`. A missing private task is an
explicit conformance failure. It releases `peer_gone` only when that stored
launch equals the stopped lease's launch.

This preserves every required generation case:

- a current-launch connection already accepted/unread at `VmStop` receives
  its peer-gone callback and follows the existing already-reading close path;
- a current-launch connection still queued is marked reset CID-wide and
  retains its buffered opening record for the later accept/read path;
- an old-launch queued connection retains its earlier buffered record/reset;
- an old-launch accepted-unread task transferred into the same-CID replacement
  row keeps its old `owner_launch` and is not falsely released by the later
  launch's stop; and
- a previously live session remains outside `unread` and continues through
  the separately driven `HostDeadSessionLoss` boundary.

Registered guest flows and in-flight dials keep their existing VMM-sweep
handling. No driver generation is guessed from the current row, no public
observation is rewritten, and the retained old task still closes stale when
its already-held read is released.

## FwdPair abort-class `VCtl(gh)` completion

The new `observed_pair` helper is a read-only QPROJ-3 identity lookup. While
the owner lives it returns the actual `(ForwarderGeneration, ForwardId)` only
for a public stream/datagram `Paired` phase. Unattached, non-paired, absent-row
and owner-death states return no pair. `pair_id` now delegates to the same
lookup while retaining its stricter attached-pair error for explicit Hop/run
actions.

For guest-to-host `Packet::Close` and `Packet::Rst`, `vctl_gh` snapshots that
live identity before applying the existing socket stimulus. The Sim socket's
abort callback records the abort class and queues the pair's existing source-A
run. The driver then executes exactly one `run_forwarder_once(pair,
PairSide::A)` and fails if the identified live pair did not queue it. That run
is the existing EVMOD-END effect carried by formal `VCtl`: it consumes no
invented state, uses no expected model ID, and removes the pair's other queued
run when the abort ends the pair.

Branch scope is exact:

- `Packet::Shut` remains callback/classification only; existing `HopEnd`
  actions carry the clean end;
- an unattached or already absent owner pair records socket facts without a
  fabricated run, preserving attach-time classification and idempotence;
- watch mode returns no pair because watch end recording occurs directly in
  the watch callback;
- after owner death no owner observation or host pair run is attempted; and
- global `auto_run` remains disabled, so every other modeled Hop/run action
  keeps its existing explicit granularity.

The helper returns generation alongside ID for actual identity and diagnostics.
If a stale-generation owner row ever names a pair outside the current context,
the existing run call returns false and the conformance action fails rather
than silently applying another context's result.

## Review gates

| Gate | Result | Iteration-69 evidence |
|---|---|---|
| Stopped launch source | PASS | Actual `self.vm_gen` lease is cloned before the stop. |
| CID-wide VMM death | PRESERVED | Existing Sim death marking remains unchanged. |
| Accepted-unread identity | PASS | Public unread must have a private control task; actual `owner_launch` decides release. |
| Old-launch retained reader | PRESERVED | Later same-CID stop does not release its read; old queued reset remains. |
| Current-launch unread | PRESERVED | Matching task receives peer-gone exactly once through existing method. |
| Queued connection semantics | PRESERVED | No task/unread lookup is invented before accept. |
| Live-session loss boundary | PRESERVED | `HostDeadSessionLoss` remains separate. |
| Pair identity | PASS | Actual QPROJ-3 generation/id, never driver/model bookkeeping. |
| Abort callback completion | PASS | Exactly one already-queued source-A run is required and executed. |
| Missing queued run | FAIL-CLOSED | Conformance step errors instead of synthesizing an end. |
| Clean half-close | PRESERVED | `Packet::Shut` does not consume its later HopEnd. |
| Unattached/ended/dead/watch branches | PRESERVED | No live public pair means no run; existing socket/callback behavior remains. |
| Global run policy | PRESERVED | `set_auto_run(false)` and all other explicit run actions are unchanged. |
| Surface discipline | PASS | Private driver logic only; no model/oracle/API/seam/production change. |

## Verification

| Check | Result |
|---|---|
| Owner task/launch and Sim read-release source inspection | PASS. |
| QPROJ-3 pair generation/id and Sim callback/run inspection | PASS. |
| `rustfmt --edition 2024 --check` | PASS for both reviewed driver files. |
| Scoped `git diff --check` | PASS before this append. |
| Exact owner-flow seed `0xab174c14` and all 46 targets | NOT RUN — mandatory after source approval. |
| Exact FwdPair Trace 63 seed and all 40 targets | NOT RUN — mandatory after source approval. |
| Final record/hash/proof evidence | NOT RUN — mandatory final gates. |

## Iteration-69 verdict

**APPROVED — SOURCE ONLY.** `VmStop` now releases accepted-connection loss
only for unread tasks owned by the stopped launch while preserving CID-wide
device death and old-generation buffered-reset semantics. Abort-class
`VCtl(gh)` now carries out exactly the real pair run queued by the existing
callback, using actual owner generation/ID and failing on a missing run.
Neither correction changes a model oracle, public seam, production path, or
unrelated action. Exact seeds and complete owner-flow/FwdPair evidence remain
mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 69 (SOURCE ONLY)

---

# Supplemental review iteration 70 — trusted physical-launch fixture plan

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-70` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | Plan only: generation-exact physical VMM identity in `SimGuestFlowIo` and owner-flow `VmStop` |
| Verdict | **APPROVED — PLAN ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The rerun invalidates iteration 69's task-owner filter without changing the
accepted model or production contract. `control_tasks[key].owner_launch`
records the owner row against which an accepted reader was attributed. A
launch-1 child queued before death and accepted after same-CID launch-2
provisioning therefore correctly has task owner launch 2, even though its
physical peer, buffered bytes, and queued reset belong to launch 1. It cannot
identify which VMM a later device-death stimulus should affect.

Parsing the buffered `ControlOpen` cannot repair that distinction. The record
is untrusted and may be stale, malformed, unsupported, empty, or carry a
foreign token while the physical socket still belongs to the fixture lease
that opened it. Physical launch is a transport-fixture fact and must be bound
at connection creation.

## Approved private test-support shape

The proposed alignment is complete and bounded:

1. `QueuedConn` receives an optional private trusted physical launch from the
   fixture creator, separate from record bytes.
2. Accept copies that value unchanged into `RecordState` and into the shared
   Sim vsock registry entry created for that accepted child. Attach/handoff
   therefore retains the same physical identity on the socket the forwarder
   observes.
3. Production-composed fixture helpers derive the tag from their real
   `Lease`/`GuestNetworkPlan`, never from the encoded request:
   `open_session_with`, `open_tcp_flow_with`, `open_datagram`,
   `open_raw_flow`, and the equivalent action-shim/shared-network creators.
   The helpers that accept a deliberately foreign or malformed payload still
   pass the actual lease launch separately.
4. Low-level socket/adapter tests that have no lease pass no tag and continue
   to use the existing CID-wide `vm_stop(cid)` semantics.
5. The launch-aware `vm_stop_before_callbacks` takes the stopped trusted
   launch at its three actual direct callers: owner-flow conformance, FwdPair
   conformance, and the existing VMM-exit stream acceptance cell.
6. The selective helper marks queued and accepted inbound resources, plus
   their registered Sim vsocks, only when the physical tag equals the stopped
   launch. Dial-created V_h resources remain current-device resources: they
   have no queued inbound tag and cannot remain live through the owner
   teardown/replacement boundary.
7. The existing low-level `vm_stop(cid)` retains a separate CID-wide internal
   path, including auto-delivery and the full sweep. No low-level test loses
   its current semantics.
8. `OwnerFlowsDriver::vm_stop` removes the disproved task-owner filter and
   releases accepted loss only for
   `QPROJ-3 unread ∩ SimSocketObservation.peer_gone`. Because the launch-aware
   Sim step authored `peer_gone` from trusted physical identity, this is the
   actual arbiter rather than a payload or driver-history guess.

## Caller and lifecycle audit

The current direct creation/caller inventory closes the shape:

- owner-acceptance `Composition` owns the real `Lease` for every session,
  TCP, datagram and raw/malformed creator;
- action-shim helpers can read the actual allocation lease independently of
  the payload token they deliberately vary;
- shared-network helpers own the active plan/launch and pass it through their
  existing `deliver` boundary;
- `accept_path_tier3` and `flow_io::sim` self-tests are intentionally
  low-level and remain untagged/CID-wide;
- the only launch-aware stop callers all own the actual stopped lease; and
- dialed host connections are created through `connect_guest`, not an inbound
  queued record. Existing teardown ends every live dial/pair/watch before a
  same-CID replacement; stale registry clones are unhooked and carry no
  surviving forwarding authority.

The identity lifetime is sufficient. An inbound tag moves with the queued
child into its accepted record and shared vsock; it is independent of when the
owner accepts, attributes, attaches, closes, or transfers the task row. A
launch-1 child accepted under launch 2 therefore remains physically launch 1.
The launch-2 stop leaves it alone; its already-held launch-1 record/reset later
drives the existing stale-token and session-loss close. A current-launch
accepted reader is marked `peer_gone` and released through the existing
already-reading path.

No production state, product API, public test API, model variable/action,
scenario, lane, payload field, or parsing heuristic is introduced. The
private test-support signature/struct changes are DISTILL's structural HOW
under the workspace interface rule and do not require DESIGN remediation.

## Implementation constraints for source review

| Constraint | Required source evidence |
|---|---|
| Trusted source | Every high-level tag comes from `Lease.launch` or the equivalent plan assignment, never decoded bytes. |
| Wrong-token/raw independence | Foreign-token, malformed, unsupported and empty-record creators retain the real lease tag. |
| Queue-to-record continuity | Accept copies the tag without recomputation. |
| Socket continuity | The accepted record's shared SimSocket registry entry carries the same tag through attach. |
| Low-level compatibility | Untagged creators use only the preserved CID-wide stop path. |
| Launch-aware callers | Exactly the three current callers pass their actual stopped lease launch. |
| Dialed V_h | Selective stop still reaches current dialed records/vsocks; no old live dial survives replacement. |
| Owner-flow release | Uses actual launch-filtered `peer_gone` intersection; no task-owner or payload comparison remains. |
| Auto-delivery | Existing CID-wide `vm_stop` still releases every CID-wide accepted loss when enabled. |
| Sweep order | First-step error visibility and existing per-flow/full callback methods remain unchanged. |
| Cleanup/resource lifetime | Closing/unhooking behavior is unchanged; tags add identity only and own no task/socket. |

## Verification status

| Check | Result |
|---|---|
| Reproduced premise correction | PASS — accepted-row launch is not physical VMM identity. |
| Current creator/caller inventory | PASS. |
| Wrong-token/malformed boundary | PASS by plan — trusted tag remains separate from bytes. |
| Same-CID generation isolation | PASS by plan. |
| Public/model/production surface | PRESERVED. |
| Source implementation | NOT YET REVIEWED. |
| Exact negative seed and full owner-flow targets | NOT RUN — mandatory after source approval. |

## Iteration-70 verdict

**APPROVED — PLAN ONLY.** Trusted physical launch belongs at the existing
test-fixture connection-creation boundary and must follow the queued child,
accepted record, and shared SimSocket independently of untrusted bytes and
owner-row attribution. The plan preserves CID-wide low-level behavior and
uses launch-aware filtering only where the stopped lease is actually known.
Source implementation, exact negative replay, all unchanged owner-flow
targets, and final evidence review remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 70 (PLAN ONLY)

---

# Supplemental review iteration 71 — trusted physical-launch source

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-71` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | Source only: iteration-70 trusted physical-launch fixture alignment across nine existing Rust test-support files |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

The implementation matches the approved iteration-70 plan. It supersedes the
owner-flow half of iteration 69's disproved task-owner filter without changing
the accepted formal model, production contract, or test scenarios.

## Trusted identity path

`SimGuestFlowIo::queue_connection` now accepts private optional
`physical_launch` metadata. `QueuedConn` stores that value separately from the
untrusted first-record bytes. Accept moves the value through both relevant
objects without decoding or recomputing it:

- the new host vsock is registered as `(peer_cid, physical_launch, socket)`;
  and
- the accepted `RecordState` receives the same `physical_launch`.

The identity therefore follows the physical queued child across delayed
accept, owner-row replacement, record reading and attach. A launch-1 child
accepted after launch-2 provisioning remains tagged launch 1 even though its
new accepted task belongs to the launch-2 owner row.

The high-level creators use trusted fixture state:

| Creator | Physical identity source | Payload independence |
|---|---|---|
| `Composition::open_session_with` | `lease.launch` | The encoded `ControlOpen.launch` remains the independently supplied argument. |
| `Composition::open_tcp_flow_with` | `lease.launch` | The encoded `FlowRequest.launch` remains the independently supplied argument. |
| `Composition::open_datagram` | `lease.launch` | Uses the lease for both because this helper has no foreign-token parameter. |
| `Composition::open_raw_flow` | `lease.launch` | Arbitrary, empty, malformed or unsupported bytes are never parsed for identity. |
| Action-shim connection helpers | The allocation launch returned by the fixture lease | The call sites obtain it from the actual pending/active allocation. |
| Shared-network `deliver` | `active_cid()`'s active plan launch | It asserts the connection CID is that same active fixture VM before queueing. |

The existing negative callers establish the source distinction directly:
`open_tcp_flow_with(..., foreign_token())` and
`open_session_with(..., foreign_token())` retain `lease.launch` as the
physical tag, while malformed, unsupported and empty records enter through
`open_raw_flow` with that same lease tag. No wire token or decoded request is
trusted as physical ownership.

## Stop and release semantics

The internal stop matcher has two explicit modes:

- `vm_stop(cid)` passes no launch filter and keeps the low-level CID-wide
  behavior. It marks every queued/accepted inbound child of the CID, every
  dialled host record and every registered host vsock, runs the full sweep,
  and retains auto-delivery of CID-wide accepted loss.
- `vm_stop_before_callbacks(cid, stopped_launch)` uses exact launch equality
  for queued and accepted inbound records. Registered inbound vsocks use the
  same tag. Dial-created records and identity-free host-dial vsocks remain the
  current-device case and are still selected.

There are exactly three launch-aware callers, and each already owns the
stopped lease: owner-flow conformance passes `stopped_lease.launch`, FwdPair
passes `self.lease().launch`, and the existing VMM-exit acceptance fixture
passes `lease.launch`. All direct low-level `queue_connection` callers in
`flow_io::sim` self-tests and `accept_path_tier3` pass `None`; none reaches the
launch-aware helper, and every low-level VMM-stop exercise retains
`vm_stop(cid)`. No untagged inbound creator reaches a launch-aware stop.

The `None` host-dial case does not create an old-launch escape. Host dials are
created by `connect_guest`, outside the inbound queue, and every live
dial/pair/watch is ended during owner teardown before same-CID replacement.
Finished pair/watch sockets are unhooked by the existing forwarder cleanup;
registry clones that remain cannot produce a later forwarding callback.

`OwnerFlowsDriver::vm_stop` now snapshots the actual stopped lease, applies
the launch-aware Sim stop, and releases only keys in the intersection of the
existing QPROJ-3 `unread` set and the Sim socket observation's actual
`peer_gone` set. The disproved `control_tasks[key].owner_launch` comparison is
gone. Consequently:

- a current-launch unread connection is marked and released through its
  already-running reader;
- an old-launch unread connection is not marked by the replacement VMM's
  stop, so its previously queued reset and stale bytes remain the only cause
  of its eventual close; and
- registered flows, dials, live-session loss and their existing callback
  actions keep their prior paths.

## Scope and surface audit

The reviewed delta adds only private fixture identity and bounded call-site
fallout. It adds no public or production API, production state, payload field,
formal variable/action/run, projection oracle, scenario, lane, wait, yield, or
test body. The tag owns no task or socket and changes no close/unhook behavior.
Unrelated concurrent source changes in the same dirty files remain outside
this iteration and are not attributed to DISTILL alignment 71.

## Verification

| Check | Result |
|---|---|
| Queue-to-record-to-vsock identity continuity | PASS. |
| High-level creator and wrong-token/raw caller audit | PASS. |
| Direct `queue_connection` caller inventory | PASS — high-level creators tag from fixture state; low-level callers pass `None`. |
| Direct launch-aware stop caller inventory | PASS — exactly three callers, each using its actual stopped lease. |
| Same-CID replacement and current-launch release analysis | PASS. |
| Identity-free dial lifecycle and unhook analysis | PASS. |
| Public/model/production surface | PRESERVED. |
| `rustfmt --edition 2024 --check` on all nine reviewed Rust files | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Exact owner-flow seed `0xab174c14` and all 46 targets | NOT RUN — mandatory after source approval. |
| Exact FwdPair seeds and all 40 targets | NOT RUN — mandatory after source approval. |
| Final formal record/hash/proof evidence | NOT RUN — mandatory final gates. |

## Iteration-71 verdict

**APPROVED — SOURCE ONLY.** The implementation obtains physical VMM identity
from trusted fixture ownership, carries it unchanged through queue, accept and
the shared host vsock, and applies generation-exact VMM-death effects without
consulting untrusted bytes or accepted-row attribution. CID-wide low-level
behavior and the current-device host-dial case remain complete. The owner-flow
driver now releases only the unread records that the launch-aware socket
arbiter actually marked. Exact negative replay, complete owner-flow/FwdPair
targets, and final formal evidence remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 71 (SOURCE ONLY)

---

# Supplemental review iteration 72 — FwdPair datagram pre-attach hold point

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-72` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | Source only: Trace-65 `InitWith(dgram)` / pre-attach UDP stimulus / `Attach(h)` hold-point alignment |
| Verdict | **CHANGES_REQUESTED — source behavior is correct; action-map prose contradicts the new hold point** |
| Open findings | 0 critical, 0 high, 1 medium |

## Behavioral source assessment

The two behavioral substitutions are exact. `InitWith(dgram)` holds the
existing `SimEffectKind::ConnectUdp` before `open_datagram` starts the real
owner task. The task therefore completes the existing request acceptance,
record read, static-policy and host-internal checks, successful `route_type`,
and `open_peer` before parking in `connect_udp`. The opened UDP `SimSocket` is
then the sole unattached peer exposed by `latest_peer_socket`; the driver's
existing `peer_socket` caches that actual socket and never fabricates a flow
association or model state.

This is the first existing asynchronous boundary at which all model-permitted
pre-attach host-peer stimuli are playable:

- `UdpBlock(gh)` and `UdpUnblock(gh)` change the actual socket's far-reader
  state;
- `SendDg(hg, ...)` queues the destination's actual pre-attach datagram on
  that socket;
- `UdpErr(h)` writes the actual UDP socket error and retains the existing
  `FwdErr(h)` report step; and
- host-side projection reads the same cached socket before attach and the
  same `peer_of_opt(flow)` socket after attach.

The Sim UDP socket already represents a connected socket object when
`open_peer` creates it; the held `ConnectUdp` effect separately owns the
adapter's destination/local/peer-address completion. Consequently these
environment facts survive the release without a copied state. In particular,
`connect_udp` sets the real peer/local address without clearing a prior
`sk_err`; datagram attach intentionally does not reject a UDP half merely for
that pending error, and the later existing error-report action classifies it.

`Attach(h)` now releases exactly `ConnectUdp`. The resumed production task
then performs, in order, the existing UDP connect completion, synchronous
`peer_addr` validation, synchronous `attach`, owner-table transition and
`Paired` write before `settle` returns. A connect failure retains the existing
`DestinationUnreachable` mapping and peer abort and cannot fall through to
peer validation or attach. Cancellation while the connect is held retains the
production `tokio::select!` branch that aborts the opened peer. No driver
shadow, forced socket fact, extra polling point, yield, action, guard, state or
scenario was added.

The other initialization branches are unchanged: stream mode still holds and
releases `ConnectPeer`, while watch mode creates its real dial/watch. All
post-attach datagram paths retain their existing primitive and error order.

## F72-01 — source action map still describes the obsolete datagram boundary

**Severity: medium — blocking documentation/source contradiction.**

The driver-level action map is part of the conformance mapping, but its
`InitWith("dgram")` row still says that Init "opens and connects H" and that
the "kernel-peer check is held." Actual source now stops before connect
completion: route lookup and `open_peer` have completed, while `ConnectUdp`
itself is held. The peer-address check is not independently held; it runs only
after `Attach(h)` releases connect. The adjacent generic `Attach("h")` row
does not repair the contradiction because it says a held "connect / check" is
answered without identifying the datagram branch.

This wording would tell the next reviewer that pre-attach stimuli operate on
a completed production UDP connect, which is precisely the boundary this
change moved. Align the two existing action-map rows with the implemented
order, for example:

- `InitWith("dgram")`: the request is read, route lookup succeeds, H is
  opened, and H's `ConnectUdp` is held; and
- `Attach("h")`: stream releases `ConnectPeer`; datagram releases
  `ConnectUdp`, after which peer validation and synchronous attach complete.

This is a documentation-only correction in the already-edited private driver.
It requires no test, model, production, API or seam change.

## Verification

| Check | Result |
|---|---|
| Attributable behavioral diff | PASS — only `RouteType` to `ConnectUdp` in datagram Init and Attach release selection. |
| Production `datagram_associate` order | PASS — route → open UDP peer → connect → peer-address validation → synchronous attach. |
| Pre-attach socket identity/state | PASS — one actual opened unattached `SimSocket`, cached and reused after attach. |
| Connect/cancel/error fallthrough | PASS. |
| Stream/watch branches | UNCHANGED. |
| Model/action/state/scenario/public surface | UNCHANGED. |
| `rustfmt --edition 2024 --check` on `quint_fwd_pair.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Exact seed `0xea0ef1a4` and all 40 FwdPair targets | NOT RUN — mandatory after source approval. |

## Iteration-72 verdict

**CHANGES_REQUESTED.** The new hold point is behaviorally correct and carries
every model-permitted pre-attach UDP fact through the actual opened peer and
the production connect/validate/attach sequence. F72-01 is the sole blocker:
the source action map still states that datagram Init completed connect and
held the later peer check. Correct those two rows to describe the actual
`ConnectUdp` boundary, then re-review before the exact seed and full 40-target
run.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 72 (SOURCE ONLY)

---

# Supplemental review iteration 73 — F72-01 disposition

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-73` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | F72-01 only: `quint_fwd_pair` action-map comment correction |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## F72-01 — resolved

The two existing action-map rows now describe the implemented boundary
exactly:

- `InitWith("dgram")` reads the request, completes route lookup, opens H and
  parks at the held `ConnectUdp`; and
- `Attach("h")` distinguishes the stream and datagram branches, releasing
  `ConnectPeer` for stream or `ConnectUdp` for datagram, then records connected
  peer validation, synchronous attach and the owner's `Paired` write.

The obsolete claims that datagram Init had already connected H and that a
later kernel-peer check itself was held are gone. The correction changes only
these rustdoc table cells. It does not alter a function, model, test body,
scenario, API, seam or runtime behavior.

## Verification

| Check | Result |
|---|---|
| F72-01 requested wording | PASS. |
| Comment-to-source order | PASS — route/open → held connect → release/connect → peer validation → synchronous attach → `Paired`. |
| Behavioral source | UNCHANGED from the iteration-72 approved assessment. |
| `rustfmt --edition 2024 --check` on `quint_fwd_pair.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Exact seed `0xea0ef1a4` and all 40 FwdPair targets | NOT RUN — mandatory after source approval. |

## Iteration-73 verdict

**APPROVED — SOURCE ONLY.** F72-01 is closed. The action map now matches the
already-approved datagram hold point and production completion order exactly.
The exact seed and full 40-target FwdPair run remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 73 (SOURCE ONLY)

---

# Supplemental review iteration 74 — FwdPair `FwdErr(h)` first run

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-74` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | Source only: sweep/UDP callback completion inside existing `FwdErr(h)` |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Accepted action boundary

The formal `FwdErr(h)` action is terminal for a live pair with a pending
socket error: it records `SocketError`, ends the pair, queues one record and
applies the pair-socket abort effects. In the Sim adapter the error callback
does not itself perform that terminal transition. `record_socket_end` stores
the abort cause and queues both source-A and source-B runs; with the driver's
existing global `auto_run = false`, one queued direction must run before the
accepted formal action is complete.

The correction carries out exactly that existing first run for the two
callback-producing branches. It does not add a new action or change the
model:

1. `fwd_err_h` preserves the existing priority: an unperformed VMM sweep
   callback wins over a pending UDP callback.
2. For a stream/datagram pair it snapshots the actual QPROJ-3
   `(ForwarderGeneration, ForwardId)` before the callback. This is the same
   already-reviewed owner identity used by abort-class `VCtl`; no driver or
   expected-model identity is consulted.
3. It invokes the existing `finish_sweep_of(flow)` or UDP peer
   `report_error`, each of which installs the real callback result and queues
   the pair's two runs.
4. It executes exactly the already-queued source-A run and fails the
   conformance action if that identified live pair has no such runnable work.
   An abort cause is pair-wide, so this first run ends the pair and existing
   `finish_item` removes the other queued direction while producing exactly
   one end record.

The direct non-callback branch still calls `run_h(PairSide::B, "FwdErr")`
once and returns through its existing settle path. There is no second run or
callback there.

## Branch and identity audit

- **VMM sweep plus pending UDP error:** the sweep branch remains first. It
  reports V_h's reset, runs the accepted terminal pair work once, and leaves
  the independently resident UDP error fact unchanged, matching Trace 67.
- **UDP error alone:** `UdpErr(h)` remains nonterminal. The connected UDP
  socket, including a zero-length resident datagram, remains intact until
  `FwdErr(h)` reports the error and performs the first queued run.
- **Stream sweep:** the same V_h callback/queued-run sequence applies to the
  stream pair, matching the reproduced Trace 56 class.
- **Watch:** `pair_to_run` is intentionally absent. A watch error callback
  ends the watch and queues its record directly; no pair run exists or is
  invented.
- **Unattached or already-ended flow:** QPROJ-3 exposes no paired identity,
  so the driver performs no fabricated run. The formal guard does not emit a
  pair `FwdErr` from those states.
- **Dead owner:** `observed_pair` returns no identity without consulting a
  vanished owner. The model makes host-owner death final, so no later host
  action is admitted.
- **Generation safety:** the snapshot names the actual generation as well as
  the ID. `run_forwarder_once` can execute only in the current open context;
  a stale or missing run therefore fails rather than being replaced with a
  guessed ID or synthetic end.

The callback still queues both sides, but the first source-A run consumes the
pair-wide stored abort and removes all remaining runs for that ID. The change
therefore cannot double-report the callback, run both directions, or produce a
second record. It changes no projection, model oracle, Sim adapter,
production code, public API, state, action, scenario or test body.

## Verification

| Check | Result |
|---|---|
| Accepted `FwdErr` action mapping | PASS — callback plus one terminal first run. |
| Actual QPROJ-3 generation/ID snapshot | PASS. |
| Sweep-over-UDP priority | PRESERVED. |
| UDP-alone nonterminal stimulus | PRESERVED. |
| Stream/datagram callback run | PASS — exactly queued source A, fail-closed when absent. |
| Direct source-B run branch | UNCHANGED. |
| Watch/unattached/ended/dead-owner branches | PASS — no fabricated pair work. |
| One-record and no-double-run behavior | PASS by `record_socket_end` / `run_pair` / `finish_item` inspection. |
| `rustfmt --edition 2024 --check` on `quint_fwd_pair.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Exact seeds `0xea0ef1a4`, Trace 56 seed, and all 40 FwdPair targets | NOT RUN — mandatory after source approval. |

## Iteration-74 verdict

**APPROVED — SOURCE ONLY.** Both callback branches now complete the existing
formal `FwdErr(h)` action by running exactly one piece of work already queued
for the actual live pair. Callback priority, UDP residency, direct-run and
watch semantics remain unchanged, and absent/non-live owner states receive no
synthetic transition. Both canonical exact seeds and all 40 FwdPair targets
remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 74 (SOURCE ONLY)

---

# Supplemental review iteration 75 — `UdpErr` association ownership guard

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-75` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: existing-contract `UdpErr(s)` enabledness in `fwd_pair.qnt`, with README/design reconciliation |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced ownership cut

Trace 69 reaches `InitWith(dgram) → CtxRelease(h) → VmmExit → UdpErr(h)`.
At Init, the real host task has opened H and is parked at `ConnectUdp`.
`CtxRelease(h)` follows the no-live-pair recovery branch: quiescence
cancellation wins the held connect's biased select, aborts H, joins the flow
task, removes the owner row and closes the unattached descriptors before the
context closes. `VmmExit` affects guest/vsock state and does not recreate H.
The subsequent model `UdpErr(h)` therefore has no association-owned connected
UDP descriptor on which production can observe `SO_ERROR`.

The old guard admitted that action solely from datagram mode, side liveness,
the unset `terr` bit and the fault budget. That admitted a stimulus after its
named production object was gone. Retaining an observation-only `SimSocket`
clone or treating the driver action as a no-op would manufacture the missing
owner resource or conceal an invalid formal transition.

## Exact guard audit

The new state-reading definition is exact:

```quint
def ownsUdpSocket(t: St, s: str): bool = and {
  t.mode == "dgram", t.ctx.get(s) == "open",
  t.pr.get(s).ph.in(Set("unattached", "live")), not(t.pr.get(s).closed),
}
```

`UdpErr(s)` replaces only its former `mode == "dgram"` term with this
predicate. It retains `sideUp(cur, s)`, `not(cur.terr.get(s))` and
`canFault(cur)` byte for byte, and its state update remains the same
`terr(s) = true` plus the existing fault count and `ConfStep::UdpErr` record.
No action, variable, outcome, scenario or transition effect was added or
removed.

The permitted states remain complete for both sides:

- **Host H, unattached:** after route/open and while `ConnectUdp` is held, H
  is the real current association socket; its pending error can be present at
  attach and the existing first run classifies it.
- **Host H, live:** the attached pair still owns H, so a transmit/ICMP error
  remains eligible and the existing `FwdErr(h)` ends the pair.
- **Guest S, unattached:** the guest owner has connected its resident slot S
  for the current association before `Paired`; an error remains eligible.
- **Guest S, live:** the attached guest pair still owns that connected S, so
  the symmetric error remains eligible.

The excluded states do not remove a valid association error:

- `ctx != open` means the owning forwarder context was released or the guest
  process/VM is gone;
- `refused` and `gone` have no current association pair;
- canonical source never enters `attaching` (`AttachRead`/`AttachHook` exist
  only under the `checkThenHook` hazard, whose configured instance is stream,
  not datagram); and
- `closed` records descriptor closure and cannot name a live connected
  association.

## Ended-pair and resident-slot proof

The `ended` exclusion does not assume that every owner fd disappears at the
instant the pair ends. A live pair's callback or first run fixes the terminal
outcome/cause, removes the pair from the forwarder and unhooks both socket
halves before the owner consumes the queued end record. Host descriptors and
the guest slot may remain owner-held during that record window, but a later
socket error cannot schedule another association run or replace the fixed
terminal cause. It is therefore not a second `UdpErr → FwdErr` transition.

On the guest side, slot S is intentionally resident after an association
ends. When the owner processes that association's end it executes the
existing order: discard S's receive queue, disconnect S, clear the association
and attached identities, and keep the slot owner unless the slot itself is
being released. Residency is not connection ownership. A later application
datagram starts a new association and reconnects the same slot; that new
association again has phase `unattached` and then `live`, so the predicate
admits its own `UdpErr` normally. The guard neither strands the resident slot
nor suppresses reassociation.

The README and accepted design text state these same ownership and terminal
boundaries. The change is an existing-contract guard correction, not a new
failure policy or a weakening of error behavior. `UdpBlock`, `UdpUnblock`,
`SendDg` and their guards are unchanged; no adjacent conclusion is inferred
without a reproduced mismatch.

## Verification

| Check | Result |
|---|---|
| Trace-69 host ownership path | PASS — recovery cancels/aborts the held H and closes its descriptors before the rejected action. |
| Host `unattached` / `live` valid states | PRESERVED. |
| Guest `unattached` / `live` valid states | PRESERVED. |
| Guest slot residency, disconnect and reassociation | PRESERVED. |
| Queued-record window and fixed terminal cause | PASS — ended pair is unhooked even before deferred owner close. |
| Existing `sideUp`, `!terr` and fault-bound terms | PRESERVED. |
| State/action/run/scenario inventory | UNCHANGED. |
| README and accepted design mapping | CONSISTENT. |
| Workspace `git diff --check` before this append | PASS. |
| Exact canonical seeds, all 40 FwdPair targets and final job-3 evidence | NOT RUN — mandatory after source approval. |

## Iteration-75 verdict

**APPROVED — SOURCE ONLY.** `UdpErr(s)` is now enabled exactly while side `s`
owns the current association's connected UDP socket: datagram mode, open
context, unattached or live phase, and descriptors not closed. The symmetric
host and guest states remain reachable, the guest's resident slot still
disconnects and may reassociate, and a queued terminal record cannot acquire a
second cause after the pair was unhooked. The guard removes only the
reproduced ownerless transition and leaves every other UDP stimulus unchanged.
Exact seeds, all 40 targets and final formal evidence remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 75 (SOURCE ONLY)


---

# Supplemental review iteration 76 — `UdpBlock` target-socket ownership

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-76` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: existing-contract `UdpBlock(d)` / `UdpUnblock(d)` enabledness in `fwd_pair.qnt` and README |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Object and direction mapping

The reproduced Trace-69 successor reaches the recovery-closed host state and
then emits `UdpBlock(gh)`. Production has no H there: recovery cancelled the
held `ConnectUdp`, aborted and removed its peer handle, joined the flow task
and released the context. The host driver correctly cannot obtain a peer
socket.

`UdpBlock(d)` is the target UDP socket's send returning `EAGAIN` because
that socket cannot currently deliver to its far reader. The drivers express
the stimulus on the actual target socket with `peer_stop_reading`;
`UdpUnblock(d)` is the resume transition on that same socket. It is not a
free-standing far reader variable after the local target socket or its
association ownership is gone.

The target mapping is exact in both directions:

- for `d = gh`, `dstOf(d) = h` and the target is host UDP peer H. Before
  host attach this is the real peer opened before the held `ConnectUdp`;
  while live it is the same socket installed in the host pair;
- for `d = hg`, `dstOf(d) = g` and the target is guest slot socket S.
  Before guest attach S is already connected for the current association;
  while live it is the UDP half installed in the guest pair.

The host conformance driver applies the `gh` stimulus to its actual
`peer_socket`; the guest driver applies the `hg` stimulus to its actual
slot `outer`. Actions for the opposite side remain stimulus-only in each
driver, as the established two-driver action map requires.

## Exact guard change

Both actions now reuse the independently approved association-ownership
predicate at their target side:

```quint
action UdpBlock(d: str): bool = all {
  ownsUdpSocket(cur, dstOf(d)), CFG != "dlive", not(cur.dg.get(d).blk),
  // unchanged effect
}

action UdpUnblock(d: str): bool = all {
  ownsUdpSocket(cur, dstOf(d)), cur.dg.get(d).blk,
  // unchanged effect
}
```

Thus the only replacement is `mode == "dgram"` with the exact target's
existing socket ownership. `UdpBlock` retains the `dlive` exclusion and
`!blk`; `UdpUnblock` retains `blk`. Both still change only the same model
`dg(d).blk` fact. `UdpErr` remains as approved in iteration 75.

The guard preserves every valid target state. Host H and guest S are eligible
throughout their respective pre-attach and live association phases. A target
whose other side has already ended remains eligible if that target itself
still owns its live association socket. The guard does not require data,
reader activity, a particular queue occupancy or a projected driver flag, so
it does not filter an independent reader choice while the association exists.

Once the target pair ends, its terminal outcome is fixed, both socket hooks
are removed and no later reader-resume can wake a send for that association.
An owner fd may remain until the queued record is consumed; exclusion at
`ph = ended` rests on the lost association hook and fixed terminal result,
not on an assumption that the fd was already dropped. The later owner close
drops H or, on the guest, discards S's queue and disconnects the resident slot.

Guest slot residency is preserved. A kept slot has no connected association
after that cleanup, but a new application datagram starts a new association,
reconnects S and returns the target to `unattached` and then `live`. That
new association may take its own `UdpBlock`/`UdpUnblock` actions. No stale
`blk` fact or pending-input shadow is carried as ownership of the ended
association.

`SendDg` is unchanged. Its source-queue/drop accounting, empty datagrams and
all existing datagram payload effects retain their previous guards and
semantics; no unreproduced adjacent restriction was introduced.

## Verification

| Check | Result |
|---|---|
| Trace-69 successor host-object absence | PASS. |
| `dstOf(gh) = h` / actual H mapping | PASS. |
| `dstOf(hg) = g` / actual S mapping | PASS. |
| Host and guest pre-attach eligibility | PRESERVED. |
| Host and guest live eligibility | PRESERVED. |
| Queued-end descriptor window | PASS — exclusion follows unhook/fixed terminal state, not premature fd disappearance. |
| Resident slot disconnect and later reassociation | PRESERVED. |
| Existing `CFG != dlive`, `!blk` and `blk` terms | PRESERVED. |
| `UdpErr`, `SendDg`, queues, drops and empty datagrams | UNCHANGED. |
| State/action/run/scenario/API inventory | UNCHANGED. |
| README ownership explanation | CONSISTENT. |
| Workspace `git diff --check` before this append | PASS. |
| Exact seeds, all 40 FwdPair targets and final proof/evidence | NOT RUN — mandatory after source approval. |

## Iteration-76 verdict

**APPROVED — SOURCE ONLY.** `UdpBlock(d)` and `UdpUnblock(d)` now require
the actual association-owned target UDP socket at `dstOf(d)`. The correction
removes only block/resume stimuli after that target's association resource is
gone, while retaining both directions' pre-attach, live and later
reassociation states. It adds no shadow, mechanism or contract change and
does not alter `UdpErr`, `SendDg` or datagram queue/drop behavior. Exact
seeds, all 40 targets and final formal evidence remain mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 76 (SOURCE ONLY)

---

# Supplemental review iteration 77 — host `TcpRst` callback first run

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-77` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL test-support/source reviewer |
| Scope | Source only: host-side `TcpRst` callback completion and its exact-seed evidence |
| Verdict | **CHANGES_REQUESTED — behavior is correct; the handoff's claimed exact trace omits the causative `Fin(hg)` step** |
| Open findings | 0 critical, 0 high, 1 medium |

## Behavioral source assessment

The host-side correction matches the accepted `TcpRst(h, after_fin)` action.
Before the existing reset stimulus, the driver snapshots the actual QPROJ-3
`(ForwarderGeneration, ForwardId)` exposed by the production owner. It then
resets the actual retained/attached TCP peer, invokes the existing cause
boundary and executes exactly one already-queued source-B run for that pair.
If QPROJ-3 named a live pair but the callback queued no B run, the action
fails rather than inventing a result or selecting a different ID.

Side B is exact: host attach installs V_h as A and D as B. TCP `peer_reset()`
derives its errno from D's actual `SimTcpState` and invokes D's installed
`sk_error_report`. The callback stores `SocketError { side: B, errno }` and
queues A and B. The selected B run sees the pair-wide stored abort before
inspecting that direction's existing `source_end`, so a reset after the
source FIN remains terminal `SocketError`. Existing `finish_item` removes
the other queued run and produces exactly one record.

The driver does not derive or hard-code the errno from `after_fin`. Before
the FIN D is `Established` and the actual socket sets `ECONNRESET`; after
the FIN it is `CLOSE_WAIT` and the actual socket sets `EPIPE`. The model
still checks `after_fin` against its source-writer FIN, and the post-step
QPROJ-8 `tcpErr` and record errno expose any model/socket disagreement.

Branch scope remains bounded:

- an unattached or already-ended H flow has no QPROJ-3 pair identity, so its
  actual peer reset is retained without a forced pair run;
- a live H pair must supply the callback's queued B run;
- side G remains the existing guest stimulus path;
- direct `Hop*` runs, byte queues and accepted bytes are unchanged; and
- callback completion cannot double-run because the first abort run removes
  both queued directions for the ended ID.

No Sim adapter, production path, model transition, projection, API, seam,
state, action, run, scenario or test body changed.

## F77-01 — the handoff's “exact” trace omits `Fin(hg)`

**Severity: medium — blocking evidence-sequence defect.**

The handoff says exact seed `0xee26ec37` reaches:

`InitWith(stream) → ShutRd(hg) → CloseUnattached(g) → Attach(h) → HopEnd0(hg) → TcpRst(h, after_fin=true)`.

The cited raw log contains an additional first transition:

`InitWith(stream) → Fin(hg) → ShutRd(hg) → CloseUnattached(g) → Attach(h) → HopEnd0(hg) → TcpRst(h, after_fin=true)`.

That omitted `Fin(hg)` is causative evidence. It sets `wfin`; the later
`HopEnd0(hg)` carries the source FIN and leaves D in `CLOSE_WAIT`, which is
why the real reset produces `EPIPE`. Without it, the displayed trace does
not explain its own `after_fin=true` parameter or EPIPE outcome.

Correct only the existing handoff sequence to include `Fin(hg)`. No source,
model, test or design change is required.

## Verification

| Check | Result |
|---|---|
| Actual QPROJ-3 generation/ID snapshot before reset | PASS. |
| Actual side-B callback and queued-run identity | PASS. |
| Callback abort checked before existing clean source end | PASS. |
| Before-FIN `ECONNRESET` / after-FIN `EPIPE` | PASS — socket-derived, no hard-coded errno. |
| No-pair branch | PRESERVED — actual reset only, no synthetic run. |
| Guest branch and direct Hop runs | UNCHANGED. |
| One record / no double run | PASS by callback, `run_pair` and `finish_item` inspection. |
| `rustfmt --edition 2024 --check` on `quint_fwd_pair.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Handoff exact trace | FAIL — missing `Fin(hg)` before `ShutRd(hg)`. |
| Both canonical seeds and all 40 FwdPair targets | NOT RUN — mandatory after source approval. |

## Iteration-77 verdict

**CHANGES_REQUESTED.** The implementation correctly completes host
`TcpRst` with exactly one real callback-queued side-B run, preserves actual
socket-derived reset errno and leaves no-pair/guest/direct-run behavior
unchanged. F77-01 is the sole blocker: the handoff's exact trace must include
the causative `Fin(hg)` step shown in its cited raw log. Re-review that
documentation-only correction before the mandatory exact seeds and
40-target run.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 77 (SOURCE ONLY)

---

# Supplemental review iteration 78 — destination-reset endpoint semantics

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-78` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL semantic/source reviewer |
| Scope | Semantic disposition only: QPROJ-7 `far_end` after a destination-originated TCP reset; F77-01 disposition |
| Verdict | **APPROVED — SEMANTIC DISPOSITION; TEST SOURCE/EXECUTION PENDING** |
| Open findings | 0 design gaps, 0 semantic blockers |

## F77-01 — resolved

The handoff's exact `0xee26ec37` trace now includes the omitted causative
`Fin(hg)` before `ShutRd(hg)`. It therefore explains `wfin`, the later
`HopEnd0(hg)`, D's `CLOSE_WAIT` state and the actual `EPIPE` selected by
`peer_reset`. The documentation-only finding is closed; iteration 77's
behavioral source approval stands.

## Competing interpretations

### Hypothesis A — `far_end` is observed output at the external peer

Under this reading, `destination_resets(flow)` is input stimulus from the
external destination into local D. It sets D's local `sk_err` and invokes
D's `sk_error_report`. The forwarder records `SocketError { side: B,
errno: ECONNRESET }`, aborts the pair, and shuts V_h both ways so the
opposite guest application observes a reset. The destination that sent the
RST receives no second owner-originated terminal event; its QPROJ-7
`far_end` entry is therefore absent.

### Hypothesis B — `far_end` also records the inbound reset stimulus

Under this reading, the seam would insert `Reset` merely because the
destination supplied an RST, even though no local socket operation sent one
back. That is an abstract action-history fact. It is not the result returned
by `SimSocket::peer_end`, and it would make one QPROJ-7 field mix outbound
far-peer observations with inbound stimulus.

## Normative and production proof

The full contract resolves the apparent wording tension in favor of
Hypothesis A.

1. QPROJ-7 defines `SimOwnerSocketObservation.far_end` as each outer far
   end's first terminal event and pins the source to the Sim socket arbiter,
   paired with Tier-3. QPROJ-1 permits projected state from that arbiter or
   an explicitly named call result, never from driver action history.
2. The corresponding read surface obtains attached TCP entries from
   `SimSocket::peer_end`. That accessor is the far peer's observed terminal
   event. The symmetric guest seam says its `far_end` contains events the
   outer far end **saw**, merging owner-close effects with `peer_end`.
3. `close_from_owner` pins the polarity directly: it records the event a
   still-live far peer can observe, and a TCP peer that already reset the
   local half cannot observe a later synthetic local reset.
4. R5-45's broad sentence that a reset makes every far end observe reset is
   made endpoint-exact by its named conformance runs:
   `RemoteResetReachesGuestAppAsReset` and
   `GuestAppResetReachesRemoteAsReset`. P-69 reports the same matrix: a
   remote RST resets G; a guest-app RST resets R. The initiating endpoint's
   own RST is the stimulus that terminated it, not a second reset received
   from the forwarder.
5. The production path agrees. An incoming destination RST moves D to the
   TCP error/closed path and calls `sk_error_report`; the module records the
   side-B socket error. Pair abort later calls `tcp_abort` on both TCP
   halves, but Linux `tcp_abort` returns `-ENOENT` without sending an active
   reset when that D socket is already `TCP_CLOSE`. The vsock half receives
   `SHUT_RDWR`, which is the required reset propagation to the opposite
   guest far end.

Thus “every far end” is an outcome invariant across the two endpoint-specific
propagation directions, not authority to report a reset echo at the endpoint
that injected the reset. The contract is complete; no DESIGN ambiguity or
user-approved weakening is required.

## Minimal fix disposition

The minimal contract-preserving correction is the existing-test oracle
alignment, not an adapter projection:

- require the exact end record
  `SocketError { side: B, errno: ECONNRESET }`;
- require `SimShutdown::Both` on the shared V_h, proving the reset reaches
  the opposite guest far end; and
- require no QPROJ-7 `far_end` row for the destination that initiated the
  reset at this observation point.

Do not insert an abstract `Reset` into `local_far_end` or mutate
`SimSocket::peer_end` for the inbound stimulus. Either would conflate cause
with observed output and contradict the pinned no-echo rule. If a later
owner-close assertion exposes a synthetic destination echo, the correct
adapter direction is to preserve the already-reset peer's absence, not to
redefine `far_end` as action history.

The proposed test edits strengthen endpoint precision: the former generic
`SocketError` match becomes exact side and errno, and the opposite-end
shutdown is newly asserted. Changing the initiating destination's expected
row from `Reset` to absent therefore does not weaken R5-45.

## Verification

| Check | Result |
|---|---|
| Focused RED evidence `6b04e060-2c6a-4ec1-8f00-063d401f8040` | CONFIRMED — old test expected `Reset`, actual QPROJ-7 read had no row. |
| Exact end record | REQUIRED — side B, `ECONNRESET`. |
| Opposite guest far-end reset | REQUIRED — V_h `sent_shutdown = Both`. |
| Initiating destination output | REQUIRED — no echoed QPROJ-7 event. |
| Production TCP counterpart | PASS — already-reset/TCP_CLOSE D receives no second active reset; V_h propagates abort. |
| R5-45 / P-69 / named-run reconciliation | PASS — reset propagates to the other endpoint in each direction. |
| DESIGN ambiguity | NONE. |
| F77-01 | CLOSED. |
| Workspace `git diff --check` before this append | PASS. |
| Current unclaimed test diff `rustfmt --check` | NOT READY — rustfmt would condense the exact `SocketError` pattern. |
| Focused rerun and broader gates | NOT RUN — test source remains unclaimed and execution is pending. |

## Iteration-78 verdict

**APPROVED — SEMANTIC DISPOSITION.** QPROJ-7 `far_end` is an observed
terminal output at the external peer, not an abstract record of the peer's
inbound reset stimulus. The correct seed-8 oracle is exact side-B
`ECONNRESET`, V_h `SHUT_RDWR`/Both for the opposite guest far end, and no
echoed event at the initiating destination. The contract is complete and
the test-oracle correction is the smallest non-weakening fix. No adapter
projection, API, seam or DESIGN change is authorized. The current unclaimed
test diff still requires rustfmt, ownership and executable verification.

# APPROVED — SUPPLEMENTAL ITERATION 78 (SEMANTIC DISPOSITION)

---

# Supplemental review iteration 79 — `SendDg` source-socket ownership

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-79` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: existing-contract `SendDg(d, sz)` enabledness in `fwd_pair.qnt` and README |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced source-object mismatch

Trace 28 reaches `InitWith(dgram) → VmmExit → CtxRelease(h) → Reopen(h)
→ SendDg(hg, 2)`. At that point the VM and guest side are gone; host
recovery cancelled/closed the old association, released its context and
opened an empty replacement. The reopened owner has no H peer socket, so
the host driver correctly cannot apply `peer_send`.

The former guard required only datagram mode, source `sideUp`, the send-count
bound and a valid size. It could therefore append a model `dq0` entry after
the association-owned local source socket had disappeared. That entry had
no production queue or driver object and was not an external packet
observation.

## Canonical source mapping

`SendDg(d, sz)` is a send by the source application/destination into the
source UDP socket's receive queue. The direction mapping is load-bearing:

- `d = gh` has `srcOf(d) = g`: the guest application sends into its
  association-owned slot socket S; and
- `d = hg` has `srcOf(d) = h`: the remote destination sends into the
  association-owned host UDP socket H.

This differs from `UdpBlock`/`UdpUnblock`, which describe the **target** UDP
send socket and correctly use `dstOf(d)`. The new guard uses
`ownsUdpSocket(cur, srcOf(d))`, matching the actual object the existing host
and guest drivers call (`peer_socket` for H, slot `outer` for S). Neither
driver receives a bookkeeping substitute or an expected model value.

## Exact guard preservation

Only the former `mode == "dgram"` term is replaced:

```quint
action SendDg(d: str, sz: int): bool = all {
  ownsUdpSocket(cur, srcOf(d)), sideUp(cur, srcOf(d)),
  cur.dg.get(d).n < dnmax(d), SIZES.contains(sz),
  // existing queue/drop/count/order effect
}
```

Every other condition and effect is unchanged:

- source `sideUp` remains explicit;
- `dnmax(d)` still bounds sends per direction;
- `SIZES` still includes the zero-length datagram;
- `n` and `sizes` still advance once for every send attempt;
- a queue below WU appends the next datagram id to `dq0`; and
- a full source queue records exactly that id in `sdrop`, preserving
  order, count and loss attribution.

No payload, frame, target queue or forwarder-run rule changed.

## Both sides and lifecycle

The new predicate preserves all valid sends:

- pre-attach H exists after `open_peer` and while `ConnectUdp` is held;
- pre-attach S is already connected for the guest association before
  `Paired`; and
- live H and S remain the source sockets installed in their respective
  pairs.

It also preserves asymmetric lifecycle windows. If the other side is gone
while this source still owns an unattached/live socket in an open context,
this source remains eligible. The guard does not require the destination
side to be live, a target reader to accept, or a forwarder run to be
possible.

After this source pair ends, the pair is unhooked and its terminal outcome
is fixed. An owner descriptor may remain pending end-record consumption,
but datagrams arriving there are not new datagrams of the ended model
association and cannot be forwarded by it. H is subsequently closed. On
the guest, S's queue is discarded and the resident slot is disconnected;
the next application datagram starts a new association and reconnects S.
That new association returns to `unattached`/`live` and can take its own
`SendDg` steps. The guard therefore preserves reassociation without carrying
an old association's `dq0`, count or drop history as a pending-input shadow.

`DRead` remains intentionally unchanged: datagrams already delivered into a
far receiver's queue are independent of whether the local association
socket still exists and may still be read. `UdpErr`, `UdpBlock` and
`UdpUnblock` remain as approved in iterations 75–76.

## Verification

| Check | Result |
|---|---|
| Trace-28 missing-H reproduction | CONFIRMED by cited exact seed evidence. |
| `srcOf(gh) = g` / guest S object | PASS. |
| `srcOf(hg) = h` / host H object | PASS. |
| Both pre-attach source states | PRESERVED. |
| Both live source states | PRESERVED. |
| Other-side-gone but source-owned state | PRESERVED. |
| Resident slot disconnect and later reassociation | PRESERVED. |
| `sideUp`, `dnmax`, size-zero domain, WU capacity, ordering and drops | PRESERVED. |
| `DRead`, `UdpErr`, `UdpBlock` and `UdpUnblock` | UNCHANGED. |
| Driver/model/API/seam/state/action/scenario inventory | UNCHANGED. |
| README explanation | CONSISTENT. |
| Workspace `git diff --check` before this append | PASS. |
| Exact seeds, all 40 FwdPair targets and final job-3 proof/evidence | NOT RUN — mandatory after source approval. |

## Iteration-79 verdict

**APPROVED — SOURCE ONLY.** `SendDg(d, sz)` now requires the
association-owned source UDP socket at `srcOf(d)`, while retaining every
existing liveness, size, count, queue-capacity, ordering and drop rule. The
guard removes only datagrams attributed to a source object that no longer
exists for that association; it preserves both directions' pre-attach, live
and later reassociation behavior and does not filter independent far-receiver
reads. Exact seeds, all 40 targets and final formal evidence remain
mandatory.

# APPROVED — SUPPLEMENTAL ITERATION 79 (SOURCE ONLY)

---

# Supplemental review iteration 80 — default-union fixture package

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-80` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/formal reviewer |
| Scope | Four bounded default-union alignments: trusted launch delivery, quiescence foreign fail-stop inventory, next-fit acceptance oracle, runtime AppState owner precondition |
| Verdict | **CHANGES_REQUESTED — two exact blockers** |
| Open findings | 0 critical, 2 high, 0 medium |

## Approved portions

### Trusted physical launch in the composed shared-network fixture

The `deliver` helper now receives trusted physical launch identity from its
creator rather than discovering it from `ActiveCid` or decoding record
bytes. Its complete direct-caller inventory is two calls:

- `provision_with_session` passes `plan.assignment().launch()` before
  `ActiveCid` is registered; and
- `open_guest_flow` passes the already-active plan launch used to construct
  that flow.

Both callers also construct `GuestConnKey.peer_cid` from the same plan.
Control-open and flow-request payloads remain independent protocol bytes;
the queue's physical tag is never parsed from them. This closes the
pre-`ActiveCid` panic without weakening launch attribution or adding
production/test API beyond the existing private fixture signature.

### Next-fit acceptance oracle

The renamed acceptance cell now matches the actual M-9 next-fit sequence.
The predecessor takes `.2`; the unrelated live allocation takes `.3` and
advances the cursor; releasing `.2` frees it without rewinding the cursor;
the successor therefore takes `.4` and the corresponding next CID.

All effect-first teardown evidence remains: the failed teardown retains the
predecessor lease, the unrelated allocation cannot take it, retry performs
teardown before `LeaseReleased`, and no removal repeats. The pool's existing
operation-sequence property independently exercises release followed by
next-fit wrap/reuse, so correcting this cell's invalid immediate-reuse
oracle does not remove released-offset reusability coverage.

### R5-58 timeout classification

No source change masks the overlapping-recovery timeout. The one-component
seed completed; the two-component seed remains an unresolved
production-supervisor/fixture scheduling investigation. No timeout increase,
scenario split or early repair completion is accepted by this review.

## F80-01 — `FailStop` action guard breaks an accepted formal run

**Severity: high — blocking conformance-coverage regression.**

Adding `CFG != "conf"` inside the canonical `FailStop(c)` action does leave
`quiesce_ok`/`quiesce_live` model checking, the temporal
`ForeignHoldEndsInFailStop`, its negative-control hazard and the external
FHOLD-CHECK supervisor cell intact. It does not, however, preserve the
accepted formal inventory.

`checks.toml` still pins the evaluator check
`quiescence-run-ok-foreign-hold-restore-changes-nothing-then-fail-stops` on
main `quiesce_conf`, run
`ForeignHoldRestoreChangesNothingThenFailStops`, with `expect = "holds"`.
That run calls `ForeignRestore` and then calls `FailStop` directly. With
`CFG == "conf"`, the new action guard makes its exact foreign-only
`FailStop` transition disabled. The check can no longer hold.

The driver documentation is correct that the Rust conformance driver cannot
drive this supervisor-only transition and therefore has no `#[quint_test]`
for that run. That does not erase the separate formal evaluator check.
Removing or invalidating the run would reduce accepted FHOLD-CHECK coverage
to make randomized driver traces green.

Keep the canonical `FailStop` action and its direct named run unchanged.
Exclude the foreign-only transition only from the recording main's
randomized `step` relation that quint-connect samples, while retaining
leak/deadline fail-stop in that relation. Model-checking mains, the direct
formal run, hazards and the supervisor cell must all remain executable.
This is a formal-driver routing correction, not permission to delete the
check or weaken `ForeignHoldEndsInFailStop`.

## F80-02 — the reported failing runtime fixture still constructs an unconverged owner

**Severity: high — blocking source omission.**

The reproduced failing body is
`runtime_reconcile_is_idempotent_across_simulated_control_plane_restart`.
Its current source still calls `AppState::new` with a fresh
`crate::acceptance::serve_ports::owner()` and never invokes
`converge_shared` on that instance. It therefore retains the exact
`flow_ports()` panic reported by the 2,124-body run.

Two other construction sites in `runtime_convergence_loop.rs` now create one
`guest_owner`, converge it, and pass that same `Arc` to `AppState::new`.
Those changes are mechanically correct, but they do not alter the named
failing body at lines around its own AppState construction.

Apply the same bounded pattern inside
`runtime_reconcile_is_idempotent_across_simulated_control_plane_restart`:
construct one fixture owner, await its existing `converge_shared`, and pass
that exact instance to `AppState::new`. Do not construct a second owner or
manufacture flow ports.

## Verification

| Check | Result |
|---|---|
| Shared-network `deliver` caller inventory | PASS — exactly plan launch before ActiveCid and active-plan launch for flows. |
| Payload/physical-launch separation | PASS. |
| Next-fit `.2` / `.3` / `.4` sequence | PASS. |
| Teardown and release-last assertions | PRESERVED. |
| Released-offset reuse coverage | PRESERVED by the existing pool operation-sequence property. |
| `quiesce_live` temporal and hazards | PRESERVED. |
| `quiesce_conf` foreign named evaluator run | FAIL — `CFG != "conf"` disables its required direct `FailStop`. |
| FHOLD-CHECK production supervisor cell | PRESERVED but does not replace the broken formal run. |
| Named runtime-restart fixture | FAIL by source inspection — still passes an unconverged fresh owner. |
| R5-58 overlap timeout | OPEN — correctly not waived or edited. |
| `rustfmt --edition 2024 --check` on the three reviewed Rust files | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Targeted rerun | NOT RUN — blocked on F80-01 and F80-02 remediation. |

## Iteration-80 verdict

**CHANGES_REQUESTED.** The trusted-launch fixture correction and the `.4`
next-fit oracle are approved, and the R5-58 timeout remains honestly open.
F80-01 must preserve the accepted foreign-hold evaluator run while filtering
only undrivable randomized conformance steps. F80-02 must converge and pass
the same owner in the actual named runtime-restart body. Targeted execution
must wait for both exact source corrections.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 80

---

# Supplemental review iteration 81 — F80 remediation and private driver identities

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-81` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source/formal reviewer |
| Scope | F80 remediation; FwdPair actual-live identity; UDP-slot held-effect identity; retained trusted-launch and next-fit corrections |
| Verdict | **CHANGES_REQUESTED — one bounded scope/source discrepancy** |
| Open findings | 0 critical, 0 high, 1 medium |

## F80-01 — resolved

The canonical `FailStop(c)` action is restored verbatim. It still enables
budget-spent, leaked-hold and foreign-hold fail-stop and therefore preserves
the direct evaluator run
`ForeignHoldRestoreChangesNothingThenFailStops`, the
`ForeignHoldEndsInFailStop` temporal, both foreign hazards and the
FHOLD-CHECK production supervisor cell.

The new `conformanceFailStopDrivable(c)` definition gates only the
`recStep` selector sampled by randomized `quiesce_conf` traces. On
`CFG != "conf"` it is unconditionally true, so model-checking and hazard
instances retain their full transition relation. On `CFG == "conf"`, it
admits budget-spent and leaked-hold fail-stop, including those states when
`foreign` is also true, and excludes only a fail-stop enabled solely by the
driver-owned foreign flag. Direct named-run calls to `FailStop` bypass that
selector and remain executable.

This is the exact accepted distinction between canonical model behavior and
the subset the Rust conformance driver can drive. No formal outcome or
FHOLD-CHECK evidence was removed.

## F80-02 — resolved

`runtime_reconcile_is_idempotent_across_simulated_control_plane_restart` now
constructs `restart_guest_owner`, awaits `converge_shared` on it before
`AppState::new`, and passes that same `Arc` into the state. It neither
constructs a second owner nor supplies manufactured flow ports. This closes
the exact reproduced `flow_ports()` precondition panic.

## FwdPair actual-live pair identity — approved

`observed_pair` continues to obtain `(generation, ForwardId)` from the
production QPROJ-3 row. It now treats that identity as runnable only when
both of these actual arbiter facts hold:

- the row generation equals the owner's current `forwarder_generation`; and
- `forwarder_pair_state(id)` reports a live pair in the current Sim context.

The generation comparison occurs before the current-context ID lookup, so an
old context's reused numeric ID cannot be mistaken for a new live pair.
After callback completion, an owner may intentionally retain its `Paired`
row until it consumes the queued record; the Sim pair is already removed and
unhooked, so returning no runnable identity is exact for late `VCtl`,
`FwdErr` and `TcpRst` stimuli.

This filter affects action execution only. The cached `(generation, id)` and
the production row remain available to projection: queued ends are still
looked up in the exact context-history row, record counts and causes remain,
and owner flow/quota retention is unchanged. Current/old context state and
phase-one complements remain projected separately. The synchronous Sim
context release makes a released context with a live pair unreachable; an
unexpected open old context remains visible through `oldCtx`, rather than
being aliased into the new ID namespace.

## UDP-slot route-effect identity — approved

The route target is only an IP and cannot distinguish simultaneous
associations to the same destination. The new stimulus-only ticket map uses
the private seam's actual `SimEffectId`:

1. `GuestReady` snapshots all currently held `RouteType` IDs.
2. After the real owner opens association `i` and settles at its route
   lookup, the driver requires exactly one new held `RouteType` effect.
3. It verifies that effect's actual target is the association destination
   and stores `i → SimEffectId` only for later stimulus delivery.
4. `HostPair(i)` and `HostRefuse(i)` remove the exact ticket, re-read the
   held effect by ID, revalidate kind and target, and release that ID.

The reproduced two-association/same-IP case supplies the negative proof:
both effects have the same target but distinct seam-issued IDs. There is no
IP-first match, model ordinal fallback, production task identity, projected
state, public API or new seam. Missing, duplicated, consumed or retargeted
tickets fail closed.

## Retained iteration-80 approvals

The creator-supplied trusted physical launch and `.4` next-fit acceptance
oracle remain correct and unchanged. The R5-58 two-component overlap
timeout remains open and is not reclassified, split or granted a larger
budget.

## F81-01 — two unrelated passing fixtures still retain the claimed reverted setup edit

**Severity: medium — blocking bounded-scope discrepancy.**

The handoff says the wrong other runtime-function edit was reverted and that
only the reproduced
`runtime_reconcile_is_idempotent_across_simulated_control_plane_restart`
body receives the new convergence precondition. Current source still adds
the same owner construction/convergence/pass-through sequence in two other
places:

- `build_converged_state`; and
- `eval_dispatch_runs_only_the_named_reconciler`.

The 2,124-body reproduction named only the restart-idempotence body as the
runtime-convergence failure; these other fixtures passed and no reproduced
failure or accepted contract requires changing their setup. Their edits are
mechanically safe but remain adjacent fixture hardening outside this exact
remediation and contradict the claimed source scope.

Revert those two unrelated convergence insertions, retaining the exact
`restart_guest_owner` correction in the failing body. If separate
production-path evidence proves either passing fixture requires convergence,
record that evidence as its own bounded alignment rather than silently
including it here.

## Verification

| Check | Result |
|---|---|
| Canonical `FailStop` direct action/run | PRESERVED. |
| Randomized `quiesce_conf` foreign-only selector | PASS — only the undrivable branch is excluded. |
| Non-conf model checking, temporal, hazards and FHOLD-CHECK | PRESERVED. |
| Intended runtime-restart owner instance | PASS — one converged owner passed unchanged. |
| FwdPair current generation + live Sim pair | PASS. |
| Old generation / numeric-ID collision | FAIL-CLOSED by generation-first short circuit. |
| Queued end/history/quota projection | PRESERVED. |
| UDP route ticket creation | PASS — exactly one new actual held ID and target. |
| UDP route ticket consumption | PASS — exact ID re-read/revalidated/released. |
| Trusted launch and next-fit `.4` | PRESERVED. |
| R5-58 timeout | OPEN — not waived. |
| Reviewed Rust files `rustfmt --edition 2024 --check` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Claimed unrelated runtime edit reversion | FAIL — two extra insertions remain. |
| Exact failed seeds, 17 targeted bodies, default union and final formal evidence | NOT RUN — mandatory after source approval. |

## Iteration-81 verdict

**CHANGES_REQUESTED.** F80-01 and F80-02 are technically closed. The
actual-live pair filter and exact UDP held-effect ticket are approved, as are
the retained trusted-launch and next-fit corrections. F81-01 is the sole
remaining blocker: remove the two unrelated owner-convergence insertions
that the handoff says were reverted, while keeping the exact correction in
the reproduced restart-idempotence fixture. Targeted execution remains
mandatory after that bounded source cleanup.

# CHANGES REQUESTED — SUPPLEMENTAL ITERATION 81

---

# Supplemental review iteration 82 — F81-01 provenance disposition

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-82` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL source reviewer |
| Scope | Mechanical disposition of F81-01 only |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Provenance correction

F81-01 incorrectly grouped two source sites as unrelated edits.
Supplemental review iteration 50 had already independently approved
`build_converged_state` as the runtime-convergence helper among the six
`AppState` fixture builders that must converge and pass the same owner
instance. Retaining that convergence is required by the earlier reviewed
fixture package; reverting it would undo an accepted alignment.

The genuinely accidental insertion was
`eval_dispatch_runs_only_the_named_reconciler`. Current source has removed
only that insertion. The passing body again supplies its original fresh
fixture owner with no added convergence call.

The newly reproduced
`runtime_reconcile_is_idempotent_across_simulated_control_plane_restart`
body retains its exact correction: it creates `restart_guest_owner`,
converges that instance, and passes the same `Arc` to `AppState::new`.

The resulting three-site disposition is exact:

- `build_converged_state`: convergence retained under iteration-50
  authority;
- `eval_dispatch_runs_only_the_named_reconciler`: accidental convergence
  reverted; and
- restart-idempotence fixture: convergence retained under F80-02
  remediation.

No other source behavior changed in this remediation.

## Verification

| Check | Result |
|---|---|
| Iteration-50 runtime-helper provenance | CONFIRMED. |
| Accidental eval-dispatch insertion | REMOVED. |
| Target restart-idempotence owner | PRESERVED — same converged instance passed to `AppState::new`. |
| `rustfmt --edition 2024 --check` on `runtime_convergence_loop.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Execution | NOT RUN — exact failed seeds, targeted bodies, default union and final evidence remain mandatory. |

## Iteration-82 verdict

**APPROVED — SOURCE ONLY.** F81-01 is closed. Its
`build_converged_state` half is withdrawn because iteration 50 already
approved that exact required helper alignment; its eval-dispatch half is
closed by the precise reversion. All technical approvals from iteration 81
stand. Mandatory targeted and broad execution remains pending.

# APPROVED — SUPPLEMENTAL ITERATION 82 (SOURCE ONLY)

---

# Supplemental review iteration 83 — cancellation-safe held DNS audit count

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-83` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL fixture/source reviewer |
| Scope | Source only: private `TestGuestDns` pending-audit counter under normal completion and future cancellation |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced cancellation path

The retained R5-58 seed `4354685564936845354` overlaps two component
episodes. A DNS audit is held at the shared semaphore while the production
one-second `with_clock_bound` expires. That bound cancels and drops the
parked audit future.

The old fixture incremented `pending` before the semaphore await and
decremented only after successful acquisition. Cancellation therefore
destroyed the future before the decrement, leaving `pending = 1` with no
audit future capable of consuming a permit. The existing
`release_audits` loop observed that stale count forever. This is a fixture
accounting defect; production already owns detecting audits and repair
attempts in separate task sets and does not require serializing the two
episodes.

## RAII source audit

`PendingAudit::new` performs the increment at the exact former point, after
the fixture confirms audits are held and immediately before awaiting the
semaphore. Its `Drop` performs the matching decrement.

The two exits are balanced exactly once:

- **normal permit acquisition:** the await returns, the code explicitly
  drops `pending` at the former post-acquisition decrement point, then
  forgets the acquired permit and continues the scripted audit; and
- **cancellation:** dropping the parked future drops its live
  `PendingAudit`, decrementing before any later `release_audits` count read.

There is no await between successful acquisition and the explicit guard
drop, so cancellation cannot interleave a second decrement in that
sequence. After explicit drop the guard no longer exists. The constructor
increments once, Rust drops a live value once, and the debug underflow
assertion therefore protects a fixture defect without defining runtime
behavior.

The guard deliberately covers only the period in which the call is pending
at `TestAuditGate::permits`. It does not claim that the whole DNS audit has
completed. On the normal path the counter retires before the scripted
`audit_refused` outcome exactly as it did previously; call ordering and the
meaning of `pending_audits()` are unchanged.

## Contract preservation

The correction changes no production audit, supervisor task ownership,
recovery episode, attempt count, clock bound, audit period, timeout,
test body or assertion. It does not auto-release a held audit or complete
the first recovery before the second begins. The existing one-period
detection bound and concurrent-episode oracle remain intact.

No public API, seam, scenario or state projection is added. `PendingAudit`
is private lifetime bookkeeping inside the existing test gate.

## Verification

| Check | Result |
|---|---|
| Increment position | PRESERVED — immediately before the held semaphore await. |
| Normal decrement position | PRESERVED — immediately after acquisition, before scripted audit outcome. |
| Cancellation decrement | PASS — future drop owns the guard drop. |
| Single decrement | PASS — explicit drop consumes the guard; no later guard value remains. |
| Underflow behavior | FAIL-CLOSED in debug; constructor/drop ownership is balanced. |
| Audit/repair task concurrency | UNCHANGED. |
| Audit period, call bound and recovery ordering | UNCHANGED. |
| R5-58 assertions and scenario | UNCHANGED. |
| Public/production surface | UNCHANGED. |
| `rustfmt --edition 2024 --check` on `shared_network_test_ports.rs` | PASS. |
| Workspace `git diff --check` before this append | PASS. |
| Exact R5-58 seed and full property | NOT RUN — mandatory after source approval; a later semantic failure would require separate evidence. |

## Iteration-83 verdict

**APPROVED — SOURCE ONLY.** The private RAII guard retires the held-audit
counter at the same point on successful acquisition and on cancellation of
the parked future. It cannot double-decrement, does not claim whole-audit
completion, and leaves the production one-period/concurrent-recovery
contract unchanged. Exact R5-58 execution remains mandatory and is not
predicted green by this source verdict.

# APPROVED — SUPPLEMENTAL ITERATION 83 (SOURCE ONLY)

---

# Supplemental review iteration 84 — watched-holder local-read nondeterminism plan

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-84` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL fixture/model reviewer |
| Scope | Plan only: private FwdPair holder-read fixture handling for the native-valid EOF/`ENOTCONN` choice after context-release `SHUT_RDWR` |
| Verdict | **APPROVED — PLAN ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Native and formal contract

The two existing named runs intentionally select different raw holder syscall
results from the same abstract post-release state:

- `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected` selects
  `HolderEnd("err")`; and
- `ReleasedWatchWithNoRecordReadsAsAbort` selects `HolderEnd("eof")`.

Both selections are canonical. P-70 measured a holder read after the
forwarder's `SHUT_RDWR` as `ENOTCONN` in 10/400 `0x3` cases, 4/400 close
cases and 1,916/4,096 parallel cases, with the remaining reads returning
zero. The accepted F11/`holderReadable` contract consequently admits exactly
`{"eof", "err"}` once the watched host vsock has sent the receive-shutdown
bit. `HolderEnd(k)` is already the environment action that chooses one member
of that set; it is not a projection of owner state.

The Sim socket currently collapses both native outcomes to `WouldBlock` in
this exact state. Its `local_read` observes queued bytes first, then `sk_err`,
received shutdown, a `Closing` vsock, and an unconnected setup. A released
watch instead has an empty connected `VsockStream`, remains `Established`,
has no received shutdown and no `sk_err`, and has only its locally sent
`Both` shutdown fact. Returning either accepted native result from that
validated collapsed state supplies an I/O-environment choice that the
existing model already owns; it does not alter a modeled state transition.

## Proposed private fixture boundary

The proposed control-plane-private `OwnerShutdownRead::{Eof, NotConnected}`
and helper are valid DISTILL test-support structure. They add no public or
cross-crate interface and require no further DESIGN or user authority under
the workspace's explicit test-support ownership rule.

The helper remains faithful only with the stated fail-closed order:

1. require the actual socket to be a `VsockStream` in `Established` state;
2. require its actual sent shutdown to be exactly `Both`;
3. require empty stream input, no received shutdown and no `sk_err`, so
   queued data, peer EOF and reset/error precedence cannot be bypassed;
4. invoke the existing `local_read(max_len)` exactly once and require the
   collapsed result to be `WouldBlock`; and
5. only then translate the existing `HolderEnd(k)` choice to `Eof` or exact
   `ENOTCONN`, without changing the socket or storing a shadow result.

These predicates prove that the context release has created the native
ambiguous read boundary. They also exclude guest half-close, device-sweep
error, nonempty input, unconnected setup and closing-vsock paths, whose real
`local_read` results remain authoritative. The helper must be selected only
for this owner/context-shutdown shape; every other `HolderEnd` continues to
call the existing socket read directly. The existing requirement that the
driver holds the actual `GuestStream` remains unchanged.

The final equality with `k` is not independent outcome evidence in this one
branch: `k` is the formal environment input used to choose a member of the
native-valid result set. The evidence is the exact socket-state validation
plus the native P-70 result set. The subsequent unchanged production
`GuestStream` operation remains the business oracle: either raw EOF or
`ENOTCONN` must still resolve from the owner-ended watch as
`ConnectionAborted`/`OwnerAbort`. The helper therefore cannot bypass or
predict the production resolver.

## Contract preservation

The plan adds no formal state, action, run, scenario, projection, production
port, public API, Sim adapter method, stored script or socket mutation. It
does not make an unproven state reachable; native P-70 already proves both
results for this state. It narrows result selection to the exact existing
`HolderEnd` environment choice and fails if the actual socket gains any fact
whose precedence would make the result deterministic.

## Verification required after implementation

| Check | Required result |
|---|---|
| Helper visibility | Private to the existing control-plane FwdPair driver/test support. |
| Actual read count | Exactly one `local_read(max_len)` invocation in the ambiguous helper path. |
| Socket prerequisites | Exact kind/state/sent-shutdown/empty-input/no-received-shutdown/no-`sk_err` checks before selection. |
| Socket state | No mutation and no stored outcome shadow. |
| Other read paths | Continue using the existing real `local_read` result. |
| Not-connected result | Exact Linux `ENOTCONN` (`107`), not a generic or copied error. |
| Named proof | Both existing context-release watch runs pass with their distinct `err` and `eof` selections. |
| Business outcome | Both retain the unchanged `OwnerAbort`/guest-reset assertions. |
| Broader execution | Exact FwdPair seed and full 40-cell gate remain mandatory. |

## Iteration-84 verdict

**APPROVED — PLAN ONLY.** The private helper is a bounded fixture model of a
native nondeterministic syscall result at an already-proven socket boundary.
It neither scripts owner state nor weakens the production `GuestStream`
outcome. Implementation approval depends on the exact fail-closed
preconditions above and proof from both existing named runs; no execution is
claimed here.

# APPROVED — SUPPLEMENTAL ITERATION 84 (PLAN ONLY)

---

# Supplemental review iteration 85 — watch-record publication ordering and final-owner witness

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-85` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal-source reviewer |
| Scope | Source only: `watchOutcome`, the existing in-flight-record named run, and empty-terminal compatibility for the existing final-owner witness |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Watch-record authority

The canonical `GuestStream` contract requires the owner to read an end record
before publishing its classification. A callback-queued record is still in
flight: production `next_end` must consume it before `process_end` publishes
the watch outcome, and the Sim records the same boundary in `ends_taken`.

The previous `watchOutcome` accepted both `queued` and `read` as authoritative.
The source correction changes only that predicate to `rec == "read"`.
`queued` and `none` now both return `wait`; the earlier owner-abort
`strmAbort` and context-release `byRel` precedence remains byte-for-byte ahead
of the record branch. Half-close versus abort classification still uses the
unchanged recorded `out` value after the owner read.

The existing
`VmmExitWithWatchRecordInFlightResolvesFromRecord` run now exposes every
boundary in order:

- after `FwdErr(h)`, the actual record is `queued`, the existing wake fact is
  present, `watchOutcome` is still `wait`, and `ReadEof` is disabled;
- the existing `TakeEnd(h)` action changes the same record to `read`;
- only then is `GuestAbort` authoritative and `ReadEof` enabled; and
- the unchanged final `ReadEof` assertions still require reset, never EOF,
  byte preservation and `hres == "GuestAbort"`.

This adds no peek, automatic end delivery, action, state, run or scenario. It
strengthens the named oracle at the exact production publication boundary and
does not filter any allowed callback ordering.

## Empty-terminal witness compatibility

`canReopen(t, s)` copies the former `Reopen` guard exactly: host side,
generation zero, host owner not dead and context released. `Reopen` now calls
that predicate with no changed update or fault-variant behavior.

`HostOwnerDeathIsFinal` reaches the same post-`CtxRelease(h)` state and checks
`not(canReopen(cur, "h"))` there instead of executing disabled `Reopen` via
`.fail()`. The old form produced an empty terminal ITF state before
quint-connect could extract `conf_step`; the new witness proves the identical
disabled-action fact in the same state. It does not remove an action, accept a
reopen, or change the final-owner outcome.

## Verification

| Check | Result |
|---|---|
| Canonical read-before-publish order | PASS — only `rec == "read"` classifies. |
| Queued record behavior | PASS — remains `wait`, with `ReadEof` disabled. |
| Owner-abort/context-release precedence | UNCHANGED. |
| Outcome and typed classification | UNCHANGED — half-close/abort mapping remains exact. |
| Existing named run | STRENGTHENED in place; no new run or scenario. |
| `Reopen` guard terms | EXACT COPY in `canReopen`. |
| `Reopen` action update | UNCHANGED. |
| Final-owner state and disabled witness | PRESERVED without an empty ITF terminal state. |
| Model/action/run inventory | UNCHANGED. |
| Execution | NOT RUN — exact named runs, all 40 FwdPair cells and final jobs-3 record/verify evidence remain mandatory. |

## Iteration-85 verdict

**APPROVED — SOURCE ONLY.** The model now preserves the production boundary
between a callback-queued record and an owner-read record, and the existing
named run proves both sides before retaining its original business outcome.
The factored `Reopen` guard is equivalent to the former action guard and makes
the existing final-owner witness recordable without changing its state or
meaning. Execution evidence remains pending.

# APPROVED — SUPPLEMENTAL ITERATION 85 (SOURCE ONLY)

---

# Supplemental review iteration 86 — watched-holder local-read nondeterminism source

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-86` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL fixture/source reviewer |
| Scope | Source only: private `OwnerShutdownRead` and `SimGuestFlowIo::owner_shutdown_read`, plus the existing FwdPair `HolderEnd(k)` caller |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Exact collapsed-state validation

`owner_shutdown_read` selects a native-valid result only after reading the
actual shared socket and requiring every fact that distinguishes the P-70
collapsed state:

- `SimSocketKind::VsockStream`;
- local `sent_shutdown == Some(Both)`;
- empty actual stream input;
- no received shutdown;
- no `sk_err`;
- `SimVsockState::Established`; and
- an actual `local_read(max)` result of `WouldBlock`.

The final underlying read also proves the socket has connected setup: with
the preceding facts, an unconnected setup would return `ENOTCONN`, not
`WouldBlock`. It therefore does not need a new setup accessor. Received
peer shutdown, orphan-sweep error, `Closing` state and queued bytes retain
their existing precedence and cannot enter the selector.

The exact local-close fact is not inferred from the trace. The helper requires
the shared V_h itself to have sent `Both`. In the only caller,
`holder_end` also requires the existing held `GuestStream`; a holder drop has
already cleared that object, an owner abort of a held watch leaves V_h
untouched, guest ends are received facts, and device death supplies `sk_err`.
Thus the eligible current-driver path is the real context-release shutdown of
the still-holder-owned watched socket. No expected model state manufactures
that fact.

## One-read audit

Exactly one `SimSocket::local_read` occurs on every reachable caller branch:

- when all collapsed-state predicates hold, the helper performs the one read,
  asserts `WouldBlock`, and returns `Some(selected)`; `unwrap_or_else` is lazy,
  so the fallback does not execute;
- when a predicate does not hold, the helper returns `None` before any local
  read, and the fallback performs the one ordinary read; and
- the caller has already required a V_h from the same retained `flow_sockets`
  entry, so a missing helper lookup cannot create a second read.

If queued bytes exist, the empty-input predicate rejects the helper before it
can drain them and the ordinary read consumes them once. If a change races
with the read after validation, the helper asserts on the non-`WouldBlock`
result and never falls through to a second read. The valid selected path has
empty input, so the underlying read changes no socket queue or state. The
helper stores no choice, observation or shadow result.

The sole current source caller passes `usize::MAX`; repository search finds no
second caller. A zero-length read therefore cannot be interpreted as a
terminal EOF by this bounded driver path. The helper's `max` parameter is the
same nonzero length used by the ordinary fallback, and no production or test
API exposes it to a modeled action. Adding a speculative zero-length guard for
an absent caller is not required by this source package.

## Result and business-outcome separation

`OwnerShutdownRead::Eof` produces only `SimLocalRead::Eof`.
`NotConnected` produces exact Linux `ENOTCONN` (`KernelErrno(107)`), with the
nonzero constructor checked locally. The existing `k` action input selects
between those two results only after the socket has proven the native
ambiguous state. In all other error paths, including the orphan sweep's
`ECONNRESET`, the real `local_read` result is retained.

`holder_end` still maps only the existing `eof | err` vocabulary, rejects any
other action input, validates the returned class against `k`, and then follows
the unchanged production `GuestStream` resolution step. The selector neither
reads nor sets the watch outcome. The existing named runs must still prove
that both raw results resolve to `OwnerAbort` and that the guest-side reset and
byte oracles remain intact.

## Surface and scope audit

`OwnerShutdownRead` and the helper are `pub(crate)` inside the existing
control-plane Sim I/O test-support module. No item is exported from the crate,
and `overdrive-sim::SimSocket` gains no method or result-script state. The
formal model, projection, actions, named runs, production owner, public API
and production `GuestStream` resolver are unchanged. This is the exact
private fixture HOW approved in iteration 84.

## Verification

| Check | Result |
|---|---|
| Actual V_h close fact | PASS — exact sent `Both` on the shared socket. |
| Queue/error/end precedence | PASS — empty input, no received shutdown, no `sk_err`, `Established`. |
| Connected setup | PASS — underlying `WouldBlock` excludes unconnected setup. |
| Underlying read count | PASS — exactly one on eligible and fallback branches. |
| Queued-byte preservation | PASS — helper rejects before reading; fallback reads once. |
| Selected error | PASS — exact `ENOTCONN` 107. |
| Zero-length terminal risk | UNREACHABLE — sole caller supplies `usize::MAX`. |
| Stored state/socket mutation | NONE on the eligible empty-input path. |
| Noncollapsed reads | UNCHANGED through `SimSocket::local_read`. |
| Public/cross-crate surface | UNCHANGED. |
| Reviewed Rust `rustfmt --edition 2024 --check` | PASS. |
| Reviewed Rust `git diff --check` | PASS. |
| Execution | NOT RUN — both existing named runs, exact seed and full 40-cell FwdPair gate remain mandatory. |

## Iteration-86 verdict

**APPROVED — SOURCE ONLY.** The implementation matches the iteration-84 plan:
it selects one native P-70 result only after the actual socket proves the
collapsed context-release state, performs exactly one underlying read, and
leaves every deterministic read and the production business resolver
unchanged. The two named-run and full-gate execution receipts remain required.

# APPROVED — SUPPLEMENTAL ITERATION 86 (SOURCE ONLY)

---

# Supplemental review iteration 87 — watch record-read fallout in the existing reset run

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-87` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal-source reviewer |
| Scope | Source only: existing `WatchedStreamSeesGuestResetAsAbort` run and its inherited `RecoveryEndsHeldStreamWhoseWatchEnded` continuation |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced fallout and exact correction

The focused six-watch execution passed four cells and exposed the same
iteration-85 record-publication boundary in the two cells built from
`WatchedStreamSeesGuestResetAsAbort`. After the guest reset and host callback,
the existing state has a `PeerAbort` record queued and the wake fact set, but
the owner has not read that record. The corrected `watchOutcome` therefore
returns `wait`, exactly as the accepted `GuestStream` contract requires.

The existing base run now records that order without changing its scenario:

1. the reset and `VCtl(gh)` retain the exact `PeerAbort`, queued-record and
   wake assertions and additionally prove `watchOutcome == "wait"` and
   `ReadEof(gh)` disabled;
2. the existing physical `HolderEnd("eof")` observation remains and again
   proves that a raw holder EOF does not classify the stream while the record
   is merely queued;
3. the existing `TakeEnd("h")` action advances that same record to `read` and
   proves `GuestAbort` authoritative with `ReadEof(gh)` enabled; and
4. the original final `ReadEof(gh)` assertions remain byte-for-byte in
   meaning: reset, never EOF, the delivered byte retained and
   `hres == "GuestAbort"`.

This is compiler/evaluator fallout from the approved `rec == "read"`
contract. It neither creates an owner read nor peeks at an outcome: the
formal run uses the already-existing `TakeEnd` action at the point where the
production owner consumes the record.

## Inherited recovery run

`RecoveryEndsHeldStreamWhoseWatchEnded` intentionally begins from the complete
base run. It now inherits a real owner-read watch record and the already
reported `GuestAbort`, matching its premise that the watch ended and the
stream reported that end before recovery.

Its own steps and assertions are unchanged. `CtxRelease(h)` still executes
the existing recovery-close behavior for an ended held stream, sets the
owner-end fact without replacing the previously reported `GuestAbort`, and
`Reopen(h)` retains the generation and outcome oracles. No extra end read is
introduced in the recovery continuation.

## Complete watch-run dependency audit

The other watch-mode `ReadEof` paths need no analogous change:

- `VmmExitWithWatchRecordInFlightResolvesFromRecord` already performs and
  proves `TakeEnd(h)` between the queued callback record and `ReadEof`;
- `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted` resolves through the
  higher-precedence `strmAbort` owner fact, so its intentionally queued
  detached record is not the classification source; and
- `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected` and
  `ReleasedWatchWithNoRecordReadsAsAbort` resolve through the higher-precedence
  `byRel` context-release fact and have no end record to read.

The remaining watch runs do not classify a `GuestStream` end, and non-watch
`ReadEof` actions use direction state rather than `watchOutcome`. The source
therefore adds no unnecessary owner-read steps and filters no valid record
ordering.

## Scope and verification

| Check | Result |
|---|---|
| Accepted `rec == "read"` meaning | PRESERVED. |
| Queued-record state | Explicitly proves wait and disabled `ReadEof`. |
| Raw holder EOF | PRESERVED and shown non-authoritative. |
| Owner record read | Existing `TakeEnd(h)` action at the production boundary. |
| Final typed outcome | PRESERVED — `GuestAbort` / `ConnectionReset`, never EOF. |
| Byte oracle | PRESERVED — `got == [1]`. |
| Recovery continuation | UNCHANGED after the corrected base state. |
| Owner-abort/context-release precedence | UNCHANGED; no unnecessary read added. |
| Actions, runs, scenarios and model state | UNCHANGED. |
| Driver, Sim and production source | UNCHANGED. |
| Reviewed model/README `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — the two corrected named runs, full 40-cell FwdPair gate and final jobs-3 record/verify evidence remain mandatory. |

## Iteration-87 verdict

**APPROVED — SOURCE ONLY.** The existing base watch-reset run now crosses the
same owner-read boundary already approved in iteration 85 before it classifies
the `GuestStream` end. Its inherited recovery run starts from the correct
read-and-reported state, and every other watch end remains on its existing
record-read or higher-precedence owner/context path. No contract or scenario
is weakened.

# APPROVED — SUPPLEMENTAL ITERATION 87 (SOURCE ONLY)

---

# Supplemental review iteration 88 — leaked-hold recovery priority and whole-lifetime owner journal

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-88` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL model/test-source reviewer |
| Scope | Source only: `quiescence.qnt` recovery-start guard/README and the existing next-fit acceptance body's complete owner-call journal |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Leaked-hold fail-stop priority

Seed `0x94db98ed` reaches the exact false transition after the final successful
restart:

```text
ForeignRestore -> OpenFlow -> LeakHold -> Damage(C1) -> RecQuiesce(C1)
```

At that last step the model has `up`, idle damaged C1 and `leaked == 1`, so
the former `RecQuiesce` guard started a new recovery and incremented the hold
count. The production supervisor does not do so. At every due periodic audit,
`run_shared_network_supervisor` first reads the same owner's
`leaked_quiescence_holds()`. A nonzero value aborts audit/attempt tasks,
publishes the typed `LeakedQuiescenceHold` fail-stop request and leaves the
loop before the audit-spawn branch. The failing driver trace advances to a
new due audit only after the leak, so production's lack of a new recovery is
the exact priority outcome rather than missing fixture progress.

`canRecQuiesce(c)` copies the former three guard terms (`up`, component idle,
component damaged) and adds only `leaked == 0 or bug("leakIgnored")`.
`RecQuiesce` uses that predicate with an unchanged update and action record.
The normal model therefore removes precisely the recovery start that the
production leak-first supervisor cannot take at that audit boundary.

The model deliberately treats `RecQuiesce` as the atomic due-audit detection
and hold acquisition; it has no separate audit-in-flight state. The added
term does not claim that a recovery already started before the leak vanishes.
`RecRepair`, `RepFail`, `RecRedo` and `RecRestore` retain their guards and
updates. The existing
`DroppedHoldKeepsForwardingQuiescedThenFailStops` run still leaks during an
already-held recovery, completes that recovery's repair/audit/restore, remains
quiesced by the leaked hold and then fail-stops.

## Hazard and foreign-hold preservation

The explicit `leakIgnored` fault changes both relevant pieces of the accepted
counterfactual consistently:

- it bypasses the new `canRecQuiesce` ledger term, retaining recovery-start
  behavior for the faulty supervisor; and
- it keeps the leak cause disabled in `FailStop`, so
  `LeakedHoldEndsInFailStop` still has its required violation.

No invariant, temporal property, hazard instance or check is removed. The
canonical `FailStop` action and its leaked-hold branch remain unchanged on
normal instances.

`ForeignRestore`, `foreign`, the `ForeignQuiescenceHold` action/property and
the previously reviewed conformance-driver exclusion are untouched. A
foreign-only result therefore retains its direct evaluator/model-checking
coverage. When a real leaked hold is also present, the production/model
leaked-hold priority remains available; the change does not reinterpret a
foreign hold as a recovery.

## Whole-lifetime owner journal

The existing
`teardown_failure_holds_the_lease_until_retry_completes_then_next_fit_continues`
fixture now performs its previously approved production-equivalent startup
convergence before `AppState::new`. The exact same
`SimSharedGuestNetworkOwner` instance is stored in the fixture and supplied as
the action owner's port. Its successful `converge_shared()` records one
`ForwarderOpen` before any allocation action.

The final assertion calls `fixture.sim_owner.calls()` directly. That method
clones the complete adapter journal; there is no mark, `since`, filter, clear
or suffix selection. Adding the leading `ForwarderOpen` is therefore required
to keep the equality honest. The remaining sequence is unchanged and exact:

1. predecessor attach/activate;
2. failed predecessor teardown abort;
3. unrelated allocation attach/activate;
4. successful predecessor retry abort; and
5. successor attach/activate.

Any repeated startup convergence, missing activation, extra teardown, changed
retry order or additional owner call still fails the equality. Per-operation
`StepTrace::mark` assertions continue to start after fixture construction, as
intended; they prove the bounded workload effects while this final oracle
proves the full owner lifetime.

The test's lease-retired/held/release-last assertions, nonrepeated element
removal, driver-stop results, exact next-fit `.4` address, matching next CID
and all start/activation outcomes are untouched. The added startup entry
neither hides a journal prefix nor relaxes any allocation outcome.

## Verification

| Check | Result |
|---|---|
| Production leak check | PASS — before due-audit spawn and recovery discovery. |
| `canRecQuiesce` original terms | PRESERVED exactly. |
| New normal guard | PASS — only nonzero owner-ledger leak disables a new recovery start. |
| In-flight recovery completion | PRESERVED — repair, redo, restore and failure actions unchanged. |
| `leakIgnored` counterexample | PRESERVED — recovery start bypass plus fail-stop omission. |
| Foreign-hold coverage | UNCHANGED. |
| Model state/actions/scenarios | No additions or deletions; one exact enabledness correction. |
| Fixture owner identity | PASS — startup and workload calls use the same owner instance. |
| Journal scope | PASS — complete `calls()` vector, with no slicing or filtering. |
| Workload call order and cardinality | PRESERVED after one exact leading `ForwarderOpen`. |
| Lease/teardown/retry/next-fit oracles | UNCHANGED. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact seed, targeted acceptance body and the coordinated broader gates remain mandatory. |

## Iteration-88 verdict

**APPROVED — SOURCE ONLY.** `canRecQuiesce` now matches the production
supervisor's leak-first due-audit boundary while retaining already-started
recovery progress and the explicit `leakIgnored` hazard. The acceptance body
honestly includes its approved same-owner startup convergence in the complete
owner journal and preserves every workload and lifecycle oracle.

# APPROVED — SUPPLEMENTAL ITERATION 88 (SOURCE ONLY)

---

# Supplemental review iteration 89 — active-session precondition for quiescence `OpenFlow`

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-89` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL conformance-source reviewer |
| Scope | Source only: existing Quiescence driver `InitWith`/successful `RestartWith` composition precondition |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced model/fixture mismatch

The continued seed `0x94db98ed` reaches:

```text
InitWith(0) -> Damage(C2) -> OpenFlow
```

`flowsOpen == 0` states that the modeled Active allocation holds no guest
flow, dial or datagram association. It does not mean the allocation or its
live control session is absent. `OpenFlow` represents a new guest-opened flow
on that already Active allocation.

The old fixture created both the allocation and its control session lazily in
`open_guest_flow`. `Damage(C2)` had already ended the control-listener accept
task, so the late `ControlOpen` could not establish the claim. Production
`activate` correctly returned `ControlSessionPending`. That failure proves a
fixture precondition mismatch: it does not prove that a live control session
stops working when its listener later dies.

## Exact production-composition precondition

After a successful composed boot, `InitWith` now calls the existing
`ComposedNode::ensure_active_cid` before any trace action can damage a
component. The helper establishes the precondition entirely through existing
production paths:

1. assigns a real plan from the node's `GuestAddressPool`;
2. calls the composed owner's production `provision` port;
3. delivers a real `ControlOpen` carrying the plan launch on the actual
   control listener, with the trusted physical launch supplied separately;
4. lets the owner accept and read that record; and
5. calls production `activate` and requires exact `ForwardingActivation::Activated`.

Only after that succeeds does `InitWith(1)` call the unchanged
`open_guest_flow`; `InitWith(0)` stops with the live allocation/session and no
modeled flow. The driver stores no synthetic owner phase, launch, claim or
projection value.

A successful `RestartWith(true)` performs the same precondition on the new
process only after the real composed boot returned `Ok`. A scripted
`RestartWith(false)` typed boot refusal returns and destroys that refused
process without allocating, provisioning, opening a control session or
activating. Unexpected boot success/refusal still takes the existing
`BootDiverged` paths.

## Projection and listener semantics

The Quiescence projection continues to compute `flowsOpen` solely as the sum
of `guest_flows`, `dials` and datagram associations in the real owner
observation. Allocation presence and the control session are deliberately not
counted. The new baseline therefore projects exact zero for `InitWith(0)` and
after a successful restart, while `InitWith(1)` still projects exactly one
after the additional real flow is paired.

`Damage(C2)` remains the real failure of the control-listener accept task. The
correction does not restart it, queue a replacement connection or bypass
session admission. It proves the intended production distinction: losing the
listener prevents a new control session, while the already accepted and read
session remains authoritative and can carry `Paired` for a later flow.
`Damage(C1)` continues to prevent `OpenFlow` through the separately approved
model guard and the dead stream-listener path.

All refusal assertions remain strict. A missing or unread control session,
wrong launch, failed provision or non-`Activated` result fails fixture setup;
no `ControlSessionPending` result is accepted. Boot refusal creates no hidden
allocation from which a later projection could read state.

## Scope and verification

| Check | Result |
|---|---|
| Model meaning of `flowsOpen` | PRESERVED — flow/dial/datagram count only. |
| Allocation/session precondition | Real pool, provision, `ControlOpen` and activate path. |
| Launch identity | Real plan launch plus existing trusted physical-launch input. |
| `InitWith(0)` projection | EXACTLY 0 modeled flows. |
| `InitWith(1)` projection | Baseline session plus exactly one real guest flow. |
| Successful restart | Same new-process active-session precondition. |
| Refused restart | No allocation/session setup. |
| C2 failure semantics | Listener dies; existing session remains usable. |
| C1 failure semantics | Unchanged; no flow admission through a dead stream listener. |
| Model actions, guards and scenarios | UNCHANGED. |
| New seam/API or injected state | NONE. |
| `rustfmt --edition 2024 --check` on the driver | PASS. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact seed, affected named runs and the broader Quiescence/formal gates remain mandatory. |

## Iteration-89 verdict

**APPROVED — SOURCE ONLY.** The correction establishes the allocation and
control-session precondition that `OpenFlow` already assumes, using the real
pool/owner/control/activation composition before listener damage can occur.
It leaves `flowsOpen` at exact zero until a modeled flow opens, preserves
listener-failure behavior and boot refusal, and adds no state or interface.

# APPROVED — SUPPLEMENTAL ITERATION 89 (SOURCE ONLY)

---

# Supplemental review iteration 90 — full-audit finding during an in-flight recovery

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-90` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: existing Quiescence `RecRedo`/`RecRestore` actions aligned with the production full-component post-repair audit and QPROJ-6 multi-recovery observation |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced production transition

Seed `0x94db98ed`, Trace 65, reaches:

```text
InitWith(1) -> Damage(C2) -> ForeignRestore -> RecQuiesce(C2)
-> LeakHold -> Damage(C1) -> RecRepair(C2) -> RecRestore(C2)
```

Immediately before the final modeled action, C2 owns a recovery hold and has
completed its repair, the separate defect hold is leaked, and C1 is damaged
but has no recovery. The former model treated `RecRestore(C2)` as an audit of
C2 alone and removed its hold. Production performs a full shared-network
audit after every successful repair.

The concrete path is fixed in current production:

1. `run_shared_network_recovery_attempt(C2)` marks C2 `Repaired` after its
   repair succeeds;
2. `audit_shared_network_components` audits the shared components and returns
   the minimum `SharedGuestNetworkComponent` finding by the enum's fixed
   `Ord`;
3. C1 is `FlowListeners`, ordered before C2 `ControlListener`, so the result
   is C1;
4. `healthy` for C2 is false because the finding is not later than C2;
5. the supervisor processes that result by calling the existing
   `acquire_shared_network_recovery_hold` for idle C1 before completing the
   round;
6. `begin_recovery(C1)` adds the distinct recovery, and the owner's real
   `quiesce()` adds one hold;
7. C2 is not in the successful set and therefore is not restored; and
8. `complete_attempt({C1, C2})` increments C2 once while the newly joined C1
   retains attempt zero, then QPROJ-6 exposes both as `Held`.

With the independent leaked hold, the exact observable result is therefore
`cnt == 3`, C1 and C2 both held, and C2 attempt one. This is the implemented
and already accepted multi-recovery/QPROJ-6 contract, not a new recovery
mechanism. The leaked-hold check from iteration 88 runs at the start of a new
periodic audit; it does not cancel an already-running recovery's mandatory
post-repair audit or result processing.

## Formal mapping

The new state-reading definitions encode only facts already present in the
model:

- `auditFinding` selects damaged C1 before damaged C2, matching the production
  component order restricted to this model's two components;
- `findingBlocks(c, finding)` is exactly production's
  `not(finding.is_none_or(|finding| finding.component > component))`; and
- `startsFinding` is true only for a real finding whose recovery is currently
  absent (`rec == "idle"`).

No model variable, action, phase or scenario is added.

For `RecRedo(c)`, a finding at or before c keeps c's existing hold, changes
its phase back to held and increments its attempt once. When the first finding
is a different idle component, the same action also gives that component one
new held recovery and increments the hold count once. The new component's
attempt remains zero, matching `joined_since_attempt`; an already active
finding receives no duplicate hold.

For `RecRestore(c)`, no finding at or before c means c's own ordered audit is
clean. A later finding may still start its idle recovery before c restores.
The modeled count changes by `+1` for that new hold and `-1` for c's restored
hold, exactly matching the production result processing and round completion.
If the later component already has a recovery, neither production nor the
model takes a second hold.

The actions fold the existing post-repair audit, any newly found recovery,
round completion and restore into the existing conformance step. That is the
same bounded action abstraction the driver already uses when it releases the
post-repair audit and settles the supervisor; it does not fabricate a driver
observation or filter a trace.

## Invariant and scenario preservation

`CountCoversRecoveries` remains exact in every added branch: the count rises
only when an idle finding becomes held and falls only when c is restored.
The independent leaked count is never consumed. A leak during an already
started recovery therefore remains quiesced while the full audit may reveal
another component, as production does.

The named overlap sequence is preserved. When C1 restores while C2 already
has a held recovery, C2 is the later finding but `startsFinding` is false;
there is no duplicate hold, C1's hold is removed, and the count remains one.
The own-component post-repair failure run is also preserved: a C2 finding
during C2's audit returns C2 to held and increments C2 once, with no new
recovery.

The `boolLatch` hazard retains its teeth. Its `RecRestore` branch still forces
the count to zero and clears held recoveries, including a newly discovered
one, and records early reopening whenever another unrepaired or newly started
recovery exists. Normal instances continue to use counted holds.

`ForeignRestore`, the foreign-hold checks, `canRecQuiesce`, repair-error
handling, fail-stop, fairness declarations, evaluator runs and check inventory
are unchanged. The correction expands the effects of the existing audit
actions only where the production full audit already reports another modeled
component.

## Contract-authority assessment

No new contract decision is required. G-V5 already requires one hold and one
budget per damaged component, overlapping recoveries, a full audit after a
repair, and no reopening until every holding recovery restores. The approved
QPROJ-6/update-table contract already permits `begin_recovery` to add a
distinct component while Recovering and `complete_attempt` to preserve a
newly joined component at attempt zero. Current production implements the
exact ordering above. The formal correction exposes that existing behavior;
it does not weaken “clean audit,” because a later first finding proves the
current earlier component's ordered audit was clean, while a same/earlier
finding keeps it in recovery.

## Verification

| Check | Result |
|---|---|
| Production audit scope | PASS — full shared-component audit after repair. |
| Finding order | PASS — C1/FlowListeners precedes C2/ControlListener. |
| C2 result with C1 finding | PASS — unhealthy; C2 hold retained and attempt incremented. |
| New C1 recovery | PASS — existing `begin_recovery` + real `quiesce`, attempt zero. |
| Later finding during C1 restore | PASS — new hold before C1 restore, net count unchanged. |
| Existing finding recovery | PASS — no duplicate hold. |
| Leaked hold | PRESERVED and never redeemed by these actions. |
| Own-component redo | PRESERVED. |
| Named two-recovery overlap | PRESERVED without a second hold. |
| `boolLatch` hazard | PRESERVED. |
| Model state/action/run/check inventory | UNCHANGED. |
| Driver, Sim and production source | UNCHANGED. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact seed, named/evaluator Quiescence coverage, hazard checks and final formal evidence remain mandatory. |

## Iteration-90 verdict

**APPROVED — SOURCE ONLY.** The model now maps the production full-audit
result exactly: an earlier finding keeps the repaired component's recovery,
counts its failed attempt and starts one distinct idle recovery under one new
hold; a later finding permits the current restore while starting the later
recovery. Existing leak, overlap, own-component and hazard behavior remains
intact, with no new contract or mechanism.

# APPROVED — SUPPLEMENTAL ITERATION 90 (SOURCE ONLY)

---

# Supplemental review iteration 91 — Quint `RecRedo` binder-scope correction

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-91` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal-source reviewer |
| Scope | Syntax only: `RecRedo` reuse of its existing new-finding predicate after Quint rejected the local binder scope |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Compiler evidence

Metal run 27 reached the formal typecheck before any semantic replay.
`quiescence.qnt` and its hazard import reported the same `QNT404` defect:
`starts` was not in scope at the later `cnt'` and `g'.overlapped`
assignments. The ensuing ten evaluator failures were typecheck fallout; no
model state or production comparison ran.

Quint's sequential `val` scope made the original `starts` binder available to
the immediately nested expression but not to those later comma-separated
action conjuncts. The adjacent `retried` binder remains valid because it is
used only by the immediately following `rec'` assignment. `RecRestore` has a
different nested binder chain and was accepted by the same typecheck; it is
unchanged.

## Semantic equivalence

The correction removes only the rejected `RecRedo` binder and inlines the
identical predicate at its three use sites:

```text
startsFinding(auditFinding) and auditFinding != c
```

Those sites remain exactly:

- whether `rec'` also changes the different idle finding to `held`;
- whether `cnt'` adds that finding's one new hold; and
- whether the overlap ghost is raised.

`startsFinding` and `auditFinding` are deterministic reads of the same
unprimed pre-state throughout one atomic action. Re-evaluating the expression
cannot observe an intervening update. The `auditFinding != c` exclusion is
present at every site, so an own-component redo still adds no second hold.

Every guard and update approved in iteration 90 remains otherwise
byte-identical: the current component returns to `held`, its attempt increments
once, latch stays true, flow/leak/foreign/environment facts remain fixed, and
the action records the same `RecRedo(c)` step. `RecRestore`, all named runs,
hazards, state and action inventories are unchanged.

## Verification

| Check | Result |
|---|---|
| Rejected binder | REMOVED from `RecRedo`. |
| Inlined predicate | Exact former expression at all three use sites. |
| Pre-state evaluation | IDENTICAL and deterministic. |
| Own-component duplicate-hold exclusion | PRESERVED at every site. |
| Other guards/effects | UNCHANGED. |
| `RecRestore` | UNCHANGED. |
| Model actions, state, runs and checks | UNCHANGED. |
| Reviewed source `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — typecheck, exact seed, ten evaluators, hazards and final formal evidence must be rerun. |

## Iteration-91 verdict

**APPROVED — SOURCE ONLY.** The inlining is the exact semantics-preserving
repair for the reproduced Quint binder-scope error. It changes no enabledness,
state transition, invariant or contract. The failed typecheck and dependent
evaluator results supply no semantic evidence; the complete formal sequence
must restart from typecheck.

# APPROVED — SUPPLEMENTAL ITERATION 91 (SOURCE ONLY)

---

# Supplemental review iteration 92 — ordered periodic-audit recovery selection

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-92` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: `nextDamagedIdle` and the existing `RecQuiesce(c)` enabledness |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced ordering mismatch

After the iteration-91 syntax correction, Metal typecheck passed and all nine
enabled named Quiescence evaluators passed. The retained randomized seed then
reached Trace 86:

```text
InitDown -> RestartWith(false) -> RestartWith(true) -> LeakHold -> Crash
-> RestartWith(true) -> Damage(C1) -> Damage(C2) -> ForeignRestore
-> RecQuiesce(C2)
```

The crash and successful restart clear the earlier process-scoped leak; both
C1 and C2 are damaged and neither has a recovery. The production full audit
uses `SharedGuestNetworkComponent`'s declaration/`Ord` order. C1 maps to
`FlowListeners`, which precedes C2 `ControlListener`, so the audit reports C1
and the supervisor starts C1's recovery. The driver correctly found no C2
recovery for the trace's requested action.

The former guard tested only the requested component's local damaged/idle
facts. It therefore admitted C2 even though production had selected the
earlier C1. This was a formal enabledness omission, not a driver failure.

## Exact recovery-start abstraction

`nextDamagedIdle` derives the next recovery-starting component from existing
state only:

- damaged idle C1 first;
- otherwise damaged idle C2; and
- otherwise `none`.

`canRecQuiesce(c)` now requires `c == nextDamagedIdle`, while retaining `up`
and the exact reviewed leak gate. Equality with `nextDamagedIdle` implies both
of the former per-c conditions (`dmg[c]` and `rec[c] == idle`), so no original
precondition was lost.

The function models recovery starts, not every audit tick. Production's audit
returns its first damaged component. When that component already has a
recovery, the supervisor updates that existing recovery and takes no second
hold; such a tick is a stutter in this action vocabulary. In the composed
driver, a listener repair is parked with that component in
`repairs_in_flight`; the owner audit deliberately excludes it, allowing the
next damaged idle component to be the finding on a following recovery-starting
tick. After a failed attempt, the driver's one-period advance also lets the
shorter retry begin and park before the held full audit is released. Thus
skipping `rec != idle` is the correct abstraction for an observable
`RecQuiesce` transition and does not claim that one audit scans past an active
finding in the same supervisor step.

This distinction preserves the existing production-backed overlap run:
after C1 owns its hold and its repair is in flight, a following audit can
start damaged idle C2. If both are idle, as in Trace 86, only C1 can start.

## Contract and hazard preservation

No accepted outcome is narrowed. The contract already fixes component order,
one recovery/hold per damaged component, and permits distinct recoveries to
overlap. The correction removes only action choices that cannot be the next
production recovery start from the same abstract state.

`ForeignRestore` changes no damage or recovery fact, so it neither reorders
nor enables a component. Its direct evaluator and foreign-hold hazard remain
unchanged.

The `leakIgnored` hazard still bypasses the leak gate and admits the ordered
recovery start; normal leaked state still fail-stops before a new periodic
audit starts a recovery. `boolLatch` retains the two-recovery path because a
non-idle C1 is skipped when C2 later starts. The existing named overlap,
post-repair finding, repair-error and fail-stop actions and assertions are
unchanged.

The change adds no variable, action, scenario, driver condition, projection,
API or production behavior. The driver continues to request the exact model
action and fails if production does not create that recovery; the model now
chooses the same ordered component rather than filtering or fabricating a
driver result.

## Verification

| Check | Result |
|---|---|
| Both idle and damaged | C1 selected before C2. |
| C1 active, C2 idle/damaged | C2 remains an eligible later recovery start. |
| Duplicate recovery/hold | FORBIDDEN — non-idle component is never selected. |
| Original damaged/idle terms | IMPLIED exactly by `c == nextDamagedIdle`. |
| Leak-first guard | PRESERVED. |
| `ForeignRestore` | UNCHANGED and ordering-neutral. |
| Named overlapping recoveries | PRESERVED. |
| `leakIgnored` and `boolLatch` hazards | PRESERVED. |
| Model state/action/run/check inventory | UNCHANGED. |
| Driver, Sim and production source | UNCHANGED. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact seed, named/evaluator suite, hazards and final formal evidence remain mandatory. |

## Iteration-92 verdict

**APPROVED — SOURCE ONLY.** `nextDamagedIdle` makes `RecQuiesce` select the
same first recovery-starting component as the production audit/supervisor
path. It rejects the impossible later choice when both recoveries are idle,
retains the existing overlapping-recovery path when an earlier component is
already active, and changes no contract or execution mechanism.

# APPROVED — SUPPLEMENTAL ITERATION 92 (SOURCE ONLY)

---

# Supplemental review iteration 93 — randomized conformance routing at a parked post-repair audit

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-93` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL conformance-boundary reviewer |
| Scope | Source only: `quiesce_conf` randomized `recStep` routing while an episode is `repaired` |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced boundary

The full 2,124-body default union passed 2,123 bodies. Its sole failure was
randomized Quiescence seed `0x82af4af4`, whose relevant suffix is:

```text
RecQuiesce(C1) -> RecRepair(C1) -> RecQuiesce(C2)
```

After `RecRepair(C1)`, production and QPROJ-6 correctly expose C1 as
`Repaired`: the repair returned `Ok`, and its full post-repair audit is
parked. C2 is damaged and idle. The model's next `RecQuiesce(C2)` expected an
independently completed periodic audit, adding C2's hold while leaving C1
repaired (`cnt == 2`).

The existing driver cannot produce that isolated observation from this
history. Both the already-started post-repair audit and the later periodic
audit call the same process responder through the same DNS audit gate.
`ComposedNode::release_audits` releases every pending waiter. Production then
processes C1's older post-repair result: the full audit finds later C2,
acquires C2's recovery hold, classifies C1 healthy, restores C1's hold and
completes the attempt. The stable projection is therefore C1 idle, C2 held,
`cnt == 1`, exactly as observed.

This is not merely an assertion-order artifact. The production full-audit
function serializes its DNS audit under the DNS mutex. In this action history,
C1's post-repair audit acquired that boundary before the later periodic audit.
The existing `release_audit` primitive can release one semaphore waiter, but
that waiter is the older post-repair audit; it cannot select the later audit
by purpose. Releasing one or all therefore reaches the same combined
supervisor outcome. Isolating the later audit would require a new tagged or
per-purpose gate, which is a new test seam and is neither necessary nor
authorized.

## Coverage-preserving routing

`conformanceRecQuiesceDrivable` applies only to randomized `recStep` selection
when `CFG == "conf"`. It routes `RecQuiesce` away whenever any episode is
`repaired`, because that phase means the existing driver has a parked
post-repair audit whose release must be observed first. The condition does
not change the canonical `RecQuiesce` action or its guard.

The production result is not dropped. From the same formal state,
`auditFinding` makes the appropriate existing action executable:

- `RecRestore(C1)` when the first finding is later C2 starts C2 and restores
  C1, producing the observed C1-idle/C2-held/count-one state; or
- `RecRedo(C1)` when the finding is at or before C1 keeps C1's hold and counts
  its failed attempt, starting a different idle finding when required.

Those actions were aligned with the full-audit result in iterations 90–91.
The randomized conformance main therefore retains executable coverage of the
combined outcome through the action the driver can honestly carry out. It no
longer asks the same gate to expose an intermediate ordering unavailable from
that trace history.

## No hidden formal weakening

The routing predicate is explicit in source and README. It does not alter:

- direct named-run action chains, which invoke canonical actions rather than
  randomized `recStep` selection;
- `quiesce_ok`, `quiesce_live`, temporal checks or any hazard instance,
  because their `CFG` is not `conf`;
- the canonical concurrent `RecQuiesce` transition retained for formal
  state-space exploration;
- the named held-overlap path, where C1 is `held`, not `repaired`, when C2's
  recovery starts; or
- repair, audit, restore, leak, foreign-hold, fail-stop, state, invariant or
  projection definitions.

The restriction is also no broader than the fixture limitation. A QPROJ-6
`Repaired` episode exists only from repair success until its post-repair audit
result is processed. In this fixture that audit is precisely the pending
shared-gate operation. Once no episode is repaired, randomized
`RecQuiesce` selection is unchanged.

Production can admit other real-time interleavings in which a periodic audit
was already ahead of a post-repair audit. The canonical model continues to
cover those. This specific driver has no honest way to create that ordering
after it has already completed the `RecRepair` action; adding a seam to do so
would manufacture scheduling control. Retaining the impossible randomized
step would compare different production boundaries, not increase valid
conformance coverage.

## Verification

| Check | Result |
|---|---|
| Raw failure classification | Boundary mismatch, not accepted production failure. |
| Existing one-audit release | Cannot select the later periodic waiter. |
| Combined production outcome | Covered by existing `RecRedo` / `RecRestore`. |
| Randomized `quiesce_conf` scope | Only `recStep` selection while an episode is `repaired`. |
| Canonical action/guard | UNCHANGED. |
| Named runs | UNCHANGED, including held overlap. |
| Non-conf model checking and hazards | UNCHANGED. |
| State, action, scenario and check inventory | UNCHANGED. |
| Driver, fixture, Sim and production source | UNCHANGED. |
| New seam/API | NONE. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact seed, named/randomized Quiescence suite, hazards and final formal evidence remain mandatory. |

## Iteration-93 verdict

**APPROVED — SOURCE ONLY.** The explicit selector removes only a randomized
action ordering that the approved conformance driver cannot isolate once a
post-repair audit is parked. Its stable production outcome remains exercised
through `RecRedo`/`RecRestore`, while canonical, named and hazard coverage stay
intact. This is honest boundary routing rather than a contract relaxation or
a green-only suppression.

# APPROVED — SUPPLEMENTAL ITERATION 93 (SOURCE ONLY)

---

# Supplemental review iteration 94 — recovery-round membership blocker classification

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-94` |
| Date | 2026-10-26 |
| Reviewer | Same independent DISTILL conformance-boundary reviewer |
| Scope | Proof/classification only: randomized seed `0x2355dc58`, production attempt-round ownership, QPROJ-6 sufficiency and existing fixture capabilities |
| Verdict | **BLOCKED — TESTABILITY REPRESENTATION DECISION REQUIRED** |
| Open findings | 0 critical, 1 high, 0 medium |

## F94-01 — QPROJ-6 cannot distinguish a current-round recovery from a joined recovery

**Severity: high — blocks honest randomized conformance completion.**

### Reproduced path

The retained seed reaches the following exact trace prefix:

```text
InitWith(0) -> Damage(C1) -> Damage(C2)
-> RecQuiesce(C1) -> RecQuiesce(C2) -> RepFail(C2)
```

After the first `RecQuiesce`, the production supervisor has inserted C1's
recovery, entered its attempt round and parked C1's repair at the existing
held `BindListener` effect. The second `RecQuiesce` is valid: the owner audit
excludes C1 while its repair is in flight, finds damaged idle C2, and the
supervisor acquires C2's distinct hold.

Production does not add C2 to the already-running attempt set. In
`run_shared_network_supervisor`, a round is spawned only when both
`attempts.is_empty()` and `round_pending.is_empty()`; it iterates the recovery
map at that instant and inserts those components into `round_pending`
([`lib.rs`:1702–1725]). C2 joins after C1's task and pending entry already
exist. The EXEC gate records it in `joined_since_attempt`
([`guest_network.rs`:267–299]), so at the eventual
`complete_attempt` it remains attempt zero while components that participated
in the round increment ([`guest_network.rs`:325–346]).

At the attempted `RepFail(C2)`, the private fixture truth is exact:
`ComposedNode::held_repair(C1)` is present and `held_repair(C2)` is absent
([`shared_network_test_ports.rs`:1230–1254]). The driver correctly advances
the retry once, still finds no C2 repair, and returns
`NotEnabled { action: "RepFail", why: "ControlListener's repair is not parked..." }`
([`quint_quiescence.rs`:332–350]). The failure is therefore a real
conformance-state ambiguity, not a missing settle, permit or repair outcome.

### Insufficient compared state

QPROJ-6 exposes for each recovery only:

- `phase: Held | Repaired`; and
- `failed_attempts`.

The Quiescence projection maps those to `rec(c)` and `att(c)`
([`lib.rs`:1151–1173], [`quint_quiescence.rs`:567–591]). Immediately before
`RepFail(C2)`, both C1 and C2 are `Held` with attempt zero. Neither the public
EXEC progress vector nor `RecoveryEpisode` says which component belongs to
the current attempt round or which joined after it began.

This information cannot be reconstructed from component order. The reverse
schedule is reachable through the same production entry points:

```text
Damage(C2) -> RecQuiesce(C2) -> Damage(C1) -> RecQuiesce(C1)
```

C2 then owns the parked current attempt and C1 is the later joiner, despite
C1 sorting first. The most recent `conf_step` is also insufficient after any
intervening damage, leak or foreign-restore stimulus overwrites it.

### Why bounded shortcuts are invalid

- **Filter `RepFail` whenever recoveries overlap:** removes accepted R5-58
  coverage of per-component repair failure during overlapping recoveries.
- **Choose a component by enum order:** contradicted by the reachable reverse
  schedule.
- **Treat every Held recovery as attempting:** reproduces the current false
  action and asks production to fail a repair task that does not exist.
- **Complete or fail C1 inside `RepFail(C2)`:** changes C1's projected phase or
  attempt count before the requested action and cannot match the formal
  post-state.
- **Auto-start C2 while C1's task is active:** changes production's accepted
  one-round-at-a-time scheduling.
- **Reuse `joined_since_attempt` through `recovery_progress`:** impossible;
  that set is intentionally private and its members are not represented in
  the returned vector.

The existing held-effect support proves current task membership but cannot by
itself constrain Quint's next action. Using it requires a corresponding
formal/conformance representation that distinguishes attempting from joined.
Conversely, exposing the supervisor's local `round_pending` would create a
new test observation boundary. Neither change is authorized by the current
model or QPROJ-6 contract.

### Minimal proven gap

An exact representation of **attempt-round membership** is missing at the
formal/driver boundary. The required representation must let randomized
conformance distinguish, for otherwise identical `Held`/attempt-zero
episodes, which component has a currently drivable repair result. It must
preserve all of the following accepted facts:

- a distinct component may join while another attempt is parked;
- the joiner owns its hold immediately but is not charged in that round;
- the next round may include every remaining recovery;
- either component may be the original participant; and
- repair failure remains independently exercisable for each component.

Selecting whether this is represented as model-only conformance state,
existing private held-effect evidence mapped into a new compared/driver fact,
or an explicit private supervisor observation is a new testability/model
choice. This review does not authorize or invent that choice.

## Existing coverage and source disposition

The production implementation is correct for the reproduced schedule. The
named overlap runs remain valid because they assert hold/count behavior and
do not demand a result from the joined component before its round. The
single-component repair-failure run remains valid but does not cover the
missing overlap-plus-failure cross-product required by randomized R5-58.

No current source change should be accepted to suppress this trace. The
iteration-93 audit routing correction addresses a different, fully observable
boundary and remains approved. `quiescence.qnt`, the driver and production
source are intentionally unchanged for F94-01.

## Verification

| Check | Result |
|---|---|
| Seed `0x2355dc58` reachability | PROVEN through composed production supervisor. |
| C1 current attempt | PROVEN by parked C1 `BindListener` repair. |
| C2 joined recovery | PROVEN by held episode/hold with no C2 repair effect. |
| QPROJ-6 phase/attempt values | IDENTICAL for C1 and C2: Held, zero. |
| Reverse membership order | REACHABLE from C2-first damage/recovery then C1 join. |
| Existing support | Can observe parked repair, but model has no membership representation. |
| Honest filter using current state | NONE. |
| Production defect | NONE in this bounded finding. |
| Source mutation | NONE. |
| Metal/formal execution by reviewer | NONE. |

## Iteration-94 verdict

**BLOCKED — TESTABILITY REPRESENTATION DECISION REQUIRED.** The failure is
genuine and the current state is insufficient: two QPROJ-6-identical
recoveries have different legal next repair actions because only one belongs
to the active attempt round. Existing private support can prove that fact but
cannot make the formal generator select it without an additional approved
representation. Filtering, component-order inference and implicit completion
would each weaken or falsify accepted overlap/failure coverage.

# BLOCKED — SUPPLEMENTAL ITERATION 94

---

# Supplemental review iteration 95 — RECROUND1 model-only attempt-round implementation

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-95` |
| Date | 2026-10-27 |
| Reviewer | Same independent DISTILL formal/source reviewer |
| Scope | Source only: user-approved RECROUND1 canonical fold in `quiescence.qnt`, plus documentation-only driver/README/handoff alignment |
| Authority | User reply **“proceed”**; `review-design-12-01-recovery-round.md` iteration 2 **APPROVED — CANONICAL FOLD** |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Iteration-94 blocker disposition

RECROUND1 supplies the exact representation iteration 94 proved missing,
without extending QPROJ-6 or any executable interface. `roundState(c)` is
model-only scheduler state with the four approved values:

- `outside`: no result in the active round, including a mid-round joiner;
- `pending`: active-round result not yet returned;
- `keep`: unhealthy result collected behind another pending member; and
- `restore`: healthy result collected behind another pending member.

The reproduced seed `0x2355dc58` now maps exactly. C1's first
`RecQuiesce` sees no round and `beginRound` marks C1 `pending`. C2's later
`RecQuiesce` preserves that active round and leaves C2 `outside`. The false
`RepFail(C2)` is therefore disabled by `resultPending(C2)`, while C1's real
parked repair remains drivable. If C1 fails, its single-member barrier keeps
C1, increments only C1, preserves outside C2 at attempt zero, then starts the
next round with both held recoveries `pending`. C2 can fail independently in
that next round. Reverse start order follows the same state rules without an
enum-order guess.

F94-01 is closed by this approved representation. No filtering, implicit
participant completion or production scheduling change was substituted for
the missing state.

## State and lifecycle audit

The implementation adds one model variable and one private result record.
`roundOutside`, `inRound`, `resultPending` and `beginRound` have the exact
approved meanings. `beginRound` marks every then-`held` recovery `pending`
and every other component `outside`, matching the supervisor's next-round
snapshot after the prior barrier.

Every action handles `roundState` explicitly:

| Action class | Source behavior | Assessment |
|---|---|---|
| `InitDown`, `InitWith` | Set `roundOutside`. | Exact fresh-process state. |
| `Damage`, `OpenFlow`, `LeakHold`, `ForeignRestore` | Preserve the map. | Exact environment/defect stimuli. |
| `RestartWith` | Preserve the stopped all-`outside` map. | Exact; every process-ending action already cleared it. |
| `Crash`, `FailStop` | Clear to `roundOutside`. | Exact process-scope reset. |
| `RecQuiesce(c)` | Starts `beginRound(nextRec)` only with no round; otherwise preserves the round and leaves the new holder outside. | Exact participant/joiner split. |
| `RecRepair(c)` | Requires pending/held and leaves pending while phase becomes repaired. | Exact pending post-repair audit. |
| `RepFail`, `RecRedo`, `RecRestore` | Collect `keep`/`restore` and invoke the common barrier helper. | Exact last-result linearization. |

There is no action path that leaves an old process's round state across crash
or fail-stop, starts a duplicate round for a joiner, or publishes a result for
an outside component.

## Barrier audit

`roundComplete` requires at least one collected result and no pending member.
Before that condition, `publishRound` preserves projected phase, attempts,
count and latch while retaining the collected status. This matches production
`round_results` waiting for `round_pending` to empty.

At the barrier, the implementation performs the approved per-member effects:

- every `keep` becomes `held` and increments its own attempt exactly once;
- every `restore` becomes `idle`, resets its attempt to zero and consumes one
  hold;
- every `outside` joiner retains phase, attempt and hold;
- count decreases by exactly the number of restore members; and
- remaining held recoveries enter the next round through `beginRound`.

Single-member outcomes are unchanged. A lone failed repair or failed audit
returns to held with one added attempt and immediately begins its next retry
round. A lone healthy recovery becomes idle and reopens only when its restored
hold takes the count to zero. A result action that discovers a different idle
component takes its new hold but leaves that recovery outside the completing
round, so it is neither restored nor charged.

Multi-member publication is correctly delayed. An early `keep` or `restore`
does not change compared attempts, phases or count while another member is
pending. The last result publishes all collected outcomes together, then
starts the next round from exactly the held survivors. This preserves the
EXEC gate's no-charge `joined_since_attempt` rule and production's successful
restore barrier.

## Hazard and invariant preservation

The existing `boolLatch` fault remains load-bearing at the result barrier.
When a barrier contains a restore, the fault clears holding recoveries and
the count as the former boolean-latch defect did; `reopenEarly` is raised when
a kept or outside recovery still required quiescence. Normal instances retain
counted holds and restore-at-zero behavior.

`RoundStateIsRecovery` exactly enforces the approved representation:

- every component value is in the four-value set;
- every non-outside member is a holding recovery;
- `restore` is possible only for a repaired phase; and
- a collected `keep`/`restore` state is observable only while some result is
  still pending, because the last-result action publishes the barrier.

The invariant joins `Safety` without removing any prior invariant. Leak,
foreign-restore, quiescence, admission and table guarantees remain unchanged.
All non-recovery stimuli preserve the scheduler state, and process-ending
actions clear it.

## Liveness and fail-stop audit

`waitingOnRound(c)` applies only to a holding recovery that is outside or has
a collected result while another component remains pending. Adding it to the
canonical `FailStop(c)` guard preserves the accepted per-component deadline:
a joiner or collected result cannot wait forever solely because another task
never returns.

`OwnFair(c)` adds `roundState` to the tuples of the result and fail-stop
actions whose enabledness reads it. The existing temporal outcomes and their
per-component fairness structure remain unchanged. The recording conformance
selector deliberately does not make this elapsed-deadline case executable;
its handler, clock behavior and existing spent-attempt/leak cases are
unchanged, as RECROUND1 requires.

The earlier conformance-only periodic-audit routing remains scoped exactly as
before. RECROUND1 adds no filter to a result action and restores the required
overlap-plus-failure coverage through actual round membership.

## Named overlap and action inventory

The existing overlap run now pins both key states without adding a run:

- after C1 then C2 quiesce, C1 is `pending` and C2 `outside`; and
- after C1's healthy single-member barrier, C2 is `pending` in the next
  round while its hold remains.

All existing action names, `ConfStep` variants, named runs, hazards and check
entries are unchanged. The single-component repair-failure and post-repair
failure runs retain their original compared outcomes. No accepted overlap,
failure, restore or fail-stop case is removed.

## Projection, fixture and interface audit

`roundState` appears in Rust only in explanatory comments. It is absent from
`QuiescenceState`, both state decoders, `State::from_driver`, driver fields and
handler logic. QPROJ-6 remains `RecoveryEpisode { phase, failed_attempts }`;
no recovery phase, EXEC observation, supervisor accessor or fixture method is
added. The existing `held_repair(c)` check remains the driving-point proof for
selected repair-result actions and still fails rather than fabricating an
unavailable task.

No production Rust, public/internal API, typed error, wire/storage format,
test body, scenario, lane or execution command changes under RECROUND1. The
Rust change is documentation-only for this decision; concurrent previously
reviewed driver corrections remain separately owned by their iterations.

## Verification

| Check | Result |
|---|---|
| User/design authority | PASS — exact RECROUND1 approved and canonically folded. |
| Seed `0x2355dc58` false C2 result | DISABLED by C2 `outside`; C1 remains pending. |
| Reverse participant order | REPRESENTED without component-order inference. |
| First round / later joiner | PASS. |
| Single-member failure/restore | Semantically unchanged. |
| Multi-member result barrier | PASS — no early projected attempt/restore/count effect. |
| Outside joiner charge | ZERO in the completing round. |
| Next-round membership | All and only held survivors become pending. |
| `boolLatch` hazard | PRESERVED at the barrier. |
| Representation invariant | Exact four-value/holding/restore/pending clauses present. |
| Per-component liveness | `waitingOnRound` plus fairness tuples match approved contract. |
| Action/run/hazard/check inventory | UNCHANGED. |
| QPROJ-6, driver, fixture, API and errors | UNCHANGED. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — typecheck, exact seed, all Quiescence targets/hazards, default union, conformance lint and final jobs-3 record/verify evidence remain mandatory. |

## Iteration-95 verdict

**APPROVED — SOURCE ONLY.** RECROUND1 is implemented exactly as approved. The
model now distinguishes pending participants, collected results and outside
joiners; publishes multi-member effects only at the existing production
barrier; retains single-member outcomes, counted holds, hazards and
per-component liveness; and adds no projection, seam, API, error or executable
scenario. Iteration-94's representation blocker is closed. Replacement proof
remains pending.

# APPROVED — SUPPLEMENTAL ITERATION 95 (SOURCE ONLY)

---

# Supplemental review iteration 96 — retained SimSocket arbiter marker placement

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-96` |
| Date | 2026-10-27 |
| Reviewer | Same independent DISTILL conformance-source reviewer |
| Scope | Source only: one `conformance-lint: sim-arbiter-ok` marker immediately above the existing `self.host_peer.clone()` projection read |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Lint finding and marker necessity

The conformance lint correctly reported `sim-in-projection` at the
`self.host_peer.clone()` fallback in `PairStateH::from_driver`. The existing
QPROJ-8 marker above the preceding flow-correlated `peer_of_opt` read did not
cover it: the lint exemption is intentionally line-local and accepts a marker
only on the Sim read's own line or the immediately preceding line.

The added marker is placed immediately above the exact `.or_else(||
self.host_peer.clone())` read. It has a nonempty, specific reason naming:

- QPROJ-8 `tcpErr`;
- the retained clone of the same actual host-peer `SimSocket`;
- the pre-attach stimulus boundary; and
- Tier-3 P-69 reset classification and P-71 destination-reset-before-attach.

It therefore satisfies the lint's narrow exemption contract. It is not a
module/function allowance, attribute suppression, wildcard reason or marker
on an unrelated owner read. Only the one source line is exempted.

## Authentic arbiter and shared identity

`SimSocket` cloning shares `Arc<Mutex<SocketState>>`; it does not copy the
socket facts. `host_peer` is populated only by `peer_socket`, which selects an
actual socket in this order:

1. the flow-correlated socket returned by `SimGuestFlowIo::peer_of_opt` after
   attach;
2. the previously retained exact socket; or
3. the existing live unattached-peer fallback.

The chosen object is stored as `socket.clone()`, so later `peer_reset`, FIN,
queue, shutdown and `sk_err` mutations affect the same shared arbiter state
that projection reads. No errno, label, expected result or formal state is
copied into driver bookkeeping.

The projection preserves the same priority. A current flow-correlated peer is
preferred; `host_peer` is used only when that lookup no longer supplies the
socket; the live unattached fallback remains last. The retained clone cannot
substitute another flow's newest socket or override an attached identity.

When production owner cleanup consumes its peer handle, the cached
`SimSocket` is only an observation handle to shared socket state. It is not a
`PeerSocket`/`OwnedSocket`, seam handle, flow task, pair id, context reference
or cleanup capability. Keeping the Arc therefore preserves the terminal
kernel fact for QPROJ-8 without keeping production ownership or resources
alive.

## Contract and native pairing

The feature-delta classifies `tcpErr(s)` as S: the TCP half's actual `sk_err`
in the `SimFlowForwarder` socket model. P-69 supplies the real-kernel reset
classification and hazard-16 errno shapes. P-71 supplies the destination
reset before attach and attach-time end behavior. Those are the exact facts
the marker names.

The previously approved pre-attach trace depends on this identity: a real
destination reset sets `sk_err` on the owner's actual unattached TCP socket;
owner cleanup removes the live handle; the model retains the terminal error;
and projection must read that same socket rather than synthesize zero. The
retained shared socket was independently approved in the earlier source
review. This marker documents that existing arbiter classification; it does
not create or broaden it.

## Scope and verification

| Check | Result |
|---|---|
| Marker position | Immediately above `self.host_peer.clone()`. |
| Marker reason | Specific QPROJ-8 fact and Tier-3 twins; nonempty. |
| Suppression scope | One Sim read line only. |
| Socket identity | Same Arc-backed `SimSocket` chosen by the real stimulus path. |
| Attached correlation | Still first priority. |
| Live unattached fallback | Retained as last priority. |
| Copied errno/model state | NONE. |
| Production ownership retained | NONE. |
| QPROJ-8 and native coverage | PRESERVED — P-69/P-71. |
| Runtime/test/model behavior | UNCHANGED. |
| Blanket lint allowance | NONE. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — coordinated `conformance-lint` and remaining replacement gates must rerun. |

## Iteration-96 verdict

**APPROVED — SOURCE ONLY.** The marker is necessary at the exact retained
socket read and accurately classifies it as the approved QPROJ-8 Sim arbiter.
It covers one line, documents the real shared identity and Tier-3 pairing, and
changes no projection value, ownership, model, fixture or production behavior.

# APPROVED — SUPPLEMENTAL ITERATION 96 (SOURCE ONLY)

---

# Supplemental review iteration 97 — first FwdPair hazard-wrapper heap split

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-97` |
| Date | 2026-10-27 |
| Reviewer | Same independent DISTILL formal-packaging reviewer |
| Scope | Source only: split the two-instance `fwd_pair_hazards.qnt` wrapper into two one-instance wrappers and repoint one existing check |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## Reproduced tooling boundary

Record run `2580067-1793100798.247` contains 279 checks: 271 passed and eight
ended `ToolError` after two attempts. The eight failures are four pairs of
checks whose wrapper files each import two FwdPair instances. Their failure
logs show `java.lang.OutOfMemoryError: Java heap space` from the Apalache
compile server while Typesafe Config tokenized the generated module, before a
first pass or TLC invariant run. Each attempt retained the existing fixed
2,048 MiB server heap and existing timeouts.

The one-instance FwdPair wrapper controls completed under the same run and
heap. The independent startup diagnosis likewise found the single-instance
shape viable without a heap override. This is therefore a compilation-unit
packaging limit. It supplies no evidence that either hazard property holds or
fails, and it does not authorize a different backend, bound, timeout or
memory allowance.

## Exact instantiation equivalence

The original wrapper contained two module declarations. Source comparison
confirms that each declaration is byte-identical after the split:

```quint
module pair_bugConsumeOnShortSend {
  import fwd_pair(OFF = Set("consumeOnShortSend"), CFG = "stall", REC = false).* from "../fwd_pair"
}

module pair_bugEofAsFin {
  import fwd_pair(OFF = Set("eofAsFin"), CFG = "ok", REC = false).* from "../fwd_pair"
}
```

`fwd_pair_hazards.qnt` retains the first declaration. New append-only
`fwd_pair_hazards_10.qnt` contains the second. Only explanatory wrapper
comments changed to describe one import per compilation unit. The imported
model, `OFF` rule, `CFG`, `REC`, module name and relative import are unchanged.

No definition from one wrapper can influence the other: each module was
already a separate instantiation of `fwd_pair`, and the checks address one
module by exact name. Moving the declaration changes only which source file
the compile server parses with it.

## Check-manifest equivalence

The manifest diff contains exactly one field change:

```text
fwd-pair-hazard-eof-as-fin-peer-loss.spec:
  hazard/fwd_pair_hazards.qnt
  -> hazard/fwd_pair_hazards_10.qnt
```

Every semantic and execution field is unchanged:

- check id/name: `fwd-pair-hazard-eof-as-fin-peer-loss`;
- main: `pair_bugEofAsFin`;
- property: invariant `NoEofOnPeerLoss`;
- backend and bound: TLC, exhaustive;
- expected outcome: `violation`;
- CI membership: true; and
- inherited timeout, retry and resource policy: unchanged.

The consume-on-short-send check changes no manifest field and continues to
address `pair_bugConsumeOnShortSend` in the original wrapper with invariant
`UnitsConserved`, TLC exhaustive, expected `violation`.

Both HEAD and current `checks.toml` contain exactly 279 `[[check]]` entries.
No check id, check, scenario, property, constant, action, model state, lane or
expected violation was added, deleted or weakened.

## Resource and evidence discipline

The source adds no heap flag, environment variable, jobs setting, worker
count, timeout, retry, backend or runner path. The fixed compile-server
resource boundary remains the boundary under test. Splitting the wrapper
reduces only how many expanded model instances one Apalache startup parses.

The current evidence summary correctly remains a failed 271/279 receipt from
the pre-split run and still names the old EOF wrapper path. It is not silently
edited into passing evidence. The exact consume-on-short-send falsification
must run after this approval; only a real expected `UnitsConserved` violation
validates the first packaging change. The remaining three two-instance
wrappers and the final jobs-3 record/verify evidence remain pending separate
source review and execution.

## Scope and verification

| Check | Result |
|---|---|
| Consume instance text | BYTE-IDENTICAL. |
| EOF-as-FIN instance text | BYTE-IDENTICAL. |
| Module names / `OFF` / `CFG` / `REC` | UNCHANGED. |
| EOF check manifest | Only `spec` path changed. |
| Consume check manifest | UNCHANGED. |
| Property/backend/bound/expected outcome | UNCHANGED. |
| Timeout/retry/resource policy | UNCHANGED. |
| Check inventory | 279 before and after. |
| Model/action/scenario semantics | UNCHANGED. |
| New execution lane or memory override | NONE. |
| Existing evidence | Still records 271 pass / 8 startup tool errors; replacement pending. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — exact falsification and later full record/verify gates remain mandatory. |

## Iteration-97 verdict

**APPROVED — SOURCE ONLY.** The change is a semantics-neutral compilation-unit
split. It keeps both FwdPair instances and both checks exact, repoints only the
EOF check's wrapper path, preserves all resource and expected-violation
settings, and leaves the 279-check inventory unchanged. Approval does not
convert the eight startup failures into proof; the prescribed falsification
and replacement evidence remain required.

# APPROVED — SUPPLEMENTAL ITERATION 97 (SOURCE ONLY)

---

# Supplemental review iteration 98 — remaining FwdPair hazard-wrapper heap splits

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-DISTILL-ALIGNMENT-REVIEW-98` |
| Date | 2026-10-27 |
| Reviewer | Same independent DISTILL formal-packaging reviewer |
| Scope | Source only: split the remaining three two-instance FwdPair hazard wrappers and repoint exactly their moved checks |
| Verdict | **APPROVED — SOURCE ONLY** |
| Open findings | 0 critical, 0 high, 0 medium |

## First-split falsification gate

The two iteration-97 jobs-1, non-recording commands both completed under the
unchanged TLC compile-server heap and timeout policy:

- `fwd-pair-hazard-consume-on-short-send`: expected `violation`, observed
  `violation`, one attempt, 42.7 seconds, exit 0; and
- `fwd-pair-hazard-eof-as-fin-peer-loss`: expected `violation`, observed
  `violation`, one attempt, 43.5 seconds, exit 0.

Both receipts report the same 2,048 MiB TLC heap, four workers, 900-second
timeout and ordinary retry policy, with no OOM. They establish that splitting
the compilation unit preserved each moved/retained hazard's teeth and removed
the reproduced startup failure without a resource override. They authorize
the identical mechanical treatment of the remaining three failed pairs; they
do not replace the final recorded evidence.

## Exact wrapper equivalence

Source comparison against HEAD confirms all six retained/moved module
declarations are byte-identical:

| Original wrapper | Retained module | New wrapper | Moved module |
|---|---|---|---|
| `fwd_pair_hazards_2.qnt` | `pair_bugCheckThenHook` | `fwd_pair_hazards_11.qnt` | `pair_bugEndedAttachGracefulClose` |
| `fwd_pair_hazards_3.qnt` | `pair_bugReleaseKeepsPairs` | `fwd_pair_hazards_12.qnt` | `pair_bugLossyEndRecord` |
| `fwd_pair_hazards_5.qnt` | `pair_bugWatchAbortShutsVh` | `fwd_pair_hazards_13.qnt` | `pair_bugDropWithoutAbort` |

Each declaration retains its exact imported `fwd_pair`, `OFF` rule, `CFG`,
`REC = false`, module name and relative source path:

- `endedAttachGracefulClose`, `CFG = "ok"`;
- `lossyEndRecord`, `CFG = "ok"`; and
- `dropWithoutAbort`, `CFG = "watch"`.

The retained instances likewise keep `checkThenHook/ok`,
`releaseKeepsPairs/ok` and `watchAbortShutsVh/watch` exactly. Only wrapper
comments were normalized to describe one import per compilation unit.

Repository inspection now finds exactly one module declaration in every
`fwd_pair_hazards*.qnt` wrapper. No imported instance was duplicated or lost.

## Check-manifest equivalence

Across iterations 97 and 98, the complete `checks.toml` diff has exactly four
changes, all to existing `spec` fields:

| Existing check | New wrapper |
|---|---|
| `fwd-pair-hazard-eof-as-fin-peer-loss` | `hazard/fwd_pair_hazards_10.qnt` |
| `fwd-pair-hazard-ended-attach-graceful-close` | `hazard/fwd_pair_hazards_11.qnt` |
| `fwd-pair-hazard-lossy-end-record` | `hazard/fwd_pair_hazards_12.qnt` |
| `fwd-pair-hazard-drop-without-abort` | `hazard/fwd_pair_hazards_13.qnt` |

For the three iteration-98 checks, names, mains, invariant properties,
backends, exhaustive bounds, expected outcomes and CI flags remain exact:

- `pair_bugEndedAttachGracefulClose` / `EndedHalfNeverEndsClean` / TLC /
  exhaustive / expected `violation`;
- `pair_bugLossyEndRecord` / `OneEndRecordPerPair` / TLC / exhaustive /
  expected `violation`; and
- `pair_bugDropWithoutAbort` / `NoLiveWatchAfterDrop` / TLC / exhaustive /
  expected `violation`.

Their retained sibling checks change no manifest field. The total check count
is 279 in both HEAD and current source. No constant, bound, timeout, retry,
worker count, resource budget, backend, expected verdict, check identity or CI
membership changes.

## Semantic and execution scope

The new files are append-only packaging wrappers. They contain no state,
action, property, scenario, run, hazard rule or tool configuration beyond the
unchanged imported module declaration. Check selection still resolves the
same main and property; TLC explores the same instance and must find the same
counterexample.

No new test, check, execution lane, runner command, heap flag or evidence
format is introduced. The current 271/279 evidence remains the failed
pre-split receipt until the complete jobs-3 record is rerun. The three moved
checks still require real expected violations in replacement evidence; source
equivalence does not pre-approve their outcomes.

## Verification

| Check | Result |
|---|---|
| First retained/moved pair falsification | PASS at expected violations under unchanged heap. |
| Six iteration-98 module declarations | BYTE-IDENTICAL to their originals. |
| `OFF` / `CFG` / `REC` / import path | UNCHANGED. |
| All FwdPair hazard wrappers | Exactly one imported instance each. |
| Manifest changes across 97+98 | Exactly four existing `spec` paths. |
| Names/mains/properties/backends/bounds | UNCHANGED. |
| Expected violations / CI / timeout | UNCHANGED. |
| Check inventory | 279 before and after. |
| Model/action/scenario/check semantics | UNCHANGED. |
| New lane/tool/resource setting | NONE. |
| Reviewed files `git diff --check` | PASS. |
| Execution | NOT RUN by this reviewer — full jobs-3 record and evidence verification remain mandatory. |

## Iteration-98 verdict

**APPROVED — SOURCE ONLY.** The remaining three changes repeat the proven
semantics-neutral wrapper split exactly. Every retained and moved instance is
byte-identical, only the three corresponding existing check paths change,
all wrappers now compile one instance, and the 279-check semantic/resource
contract is unchanged. Replacement recorded evidence remains pending.

# APPROVED — SUPPLEMENTAL ITERATION 98 (SOURCE ONLY)
