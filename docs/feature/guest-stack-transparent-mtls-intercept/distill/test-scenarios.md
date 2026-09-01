# DISTILL test scenarios — bounded lifecycle/network correction

**Feature:** `guest-stack-transparent-mtls-intercept` (GH #222)

**Original design base:** `465b96c39083984a1d2d470caff918a723b9301f`

**Current amendment:** ADR-0089 §7 and feature-delta
`BTR-3 allocation-lifecycle port amendment` (2026-09-01)

**Scope:** BTR-1, BTR-2, and BTR-3 only.

This is specification prose; executable `.feature` files are forbidden. Rust
acceptance tests and registered `overdrive-sim` invariants are the executable
handoff. BTR-1 and BTR-2 are implemented. BTR-3 has one intentional RED Rust
scaffold for the newly approved lifecycle-port invariant; production API and
adapter implementation remain DELIVER work.

## Reconciliation and scope fence

The accepted BTR-3 amendment supersedes only the old claim that no new port or
Sim seam is needed. The exact production addition is the two-method async
`MtlsInterceptLifecycle` port owned by `action_shim`, its implementation for
the existing `Arc<MtlsInterceptWorker>`, and the socket-free
`SimMtlsInterceptLifecycle`. The lower-level three-method `MtlsIntercept` port,
existing errors, singular worker ownership, and BTR-1/BTR-2 behavior remain
unchanged.

No scenario may add a retry/probe/inspection method, new error or enum variant,
second owner, wrapper, constructor, cancellation protocol, outbox, receipt,
generation fence, expanded boot-GC state machine, or
`RestartNetworkDisposition`. Tests observe existing port results, snapshots,
events, rows, and allocator ownership only.

## Coverage map

| ID | Canonical executable evidence | Exact responsibility |
|---|---|---|
| S-GTI-BTR-01 | registered Tier-1 `terminal-contention-converges`; focused `stop_allocation_second_lww_rejection_completes_without_event` | The invariant owns the production Stop/exit-observer equal-timestamp LWW loss and one bounded rebase. The focused table owns the 0/1/2-proposal boundary, exact-terminal no-op, and typed read/write errors. |
| S-GTI-BTR-02 | registered Tier-1 `vm-provision-failure-cleans-network-and-reuses-slot`; focused `post_assignment_provision_failure_tears_down_before_slot_release` | The invariant owns seeded partial logical-network creation, successful exact-complement cleanup, durable Failed classification, and smallest-free slot reuse. The focused table owns both action arms, teardown-failure slot retention, store-error precedence, and pre-assignment exhaustion. |
| S-GTI-BTR-03 | Tier-1 `same-id-restart-removes-prior-protection-before-replacement-provision` after DELIVER registration; RED scaffold `same_id_restart_removes_prior_protection_before_replacement_provision`; integration sibling `same_id_restart_real_worker_closes_prior_listener_and_drops_guard_before_stop_completion` | The invariant owns deterministic cross-port ordering, failure partitions, convergence, seed reproduction, and negative control. The integration sibling owns only real worker/listener/guard cleanup facts. |

The two implemented invariants are not generalized beyond the scopes in this
table. Focused examples remain only where they prove different edge/error
contracts. Tier 3 is limited to real sockets, kernel/netns/veth/TAP/nftables,
cgroups, processes, redb, and their cleanup complements; it does not repeat a
Tier-1 cross-port oracle.

## S-GTI-BTR-01 — terminal contention is bounded

```gherkin
@driving_port @in-memory @error @contract-shape:bounded-change
Scenario: Stop converges after the production exit observer wins an LWW race
  Given a running VM is supervised by SimDriver
    And StopAllocation reaches its first compound terminal proposal
  When the real exit observer accepts an equal-timestamp driver occurrence first
  Then the parked Stop proposal is the actual LWW loser
    And one fresh read and rebased proposal reaches the requested terminal
    And the rejected proposal appends no occurrence or lifecycle event
    And the driver route and supervision ownership are released
```

`terminal-contention-converges` drives the real Stop and exit-observer
composition against `SimDriver` and `SimObservationStore`. Its pure checker
pins the current row, exact accepted/rejected attempt sequence, occurrences,
events, one-rebase/two-proposal bound, route removal, observer drain, and
supervision release. Its negative control deletes the observed LWW loss and
must fail.

The focused action-shim table remains complementary:

| Partition | Required result |
|---|---|
| first proposal accepted | one accepted tail |
| first loser, second accepted | one rebase, then accepted tail |
| current row already equals requested terminal | no proposal/event; release and route removal |
| two losers | success after exactly two proposals; no fabricated event or third proposal |
| point-read or compound-write error | existing typed error and no partial commit |

## S-GTI-BTR-02 — post-assignment provision failure is unwound

```gherkin
@driving_port @in-memory @error @cleanup @contract-shape:bounded-change
Scenario: A VM provision failure removes exactly its assigned logical network
  Given slot 0 is held by a blocker
    And the target receives slot 1
    And the seeded provisioner creates a non-empty proper artifact subset
  When production StartAllocation receives the existing typed provision error
  Then structural teardown removes exactly the created target-owned artifacts
    And the target is durably Failed without reaching Driver::start
    And a successor receives the released smallest-free slot 1
```

`vm-provision-failure-cleans-network-and-reuses-slot` owns that successful
unwind trajectory. Its safety/liveness/convergence checker uses only observed
provisioner events, allocator ownership, rows, occurrences, and driver calls;
the negative control deletes one `ArtifactRemoved` fact and must fail.

The focused action-shim table retains only additional error precedence and
boundary complements:

| Partition | Required result |
|---|---|
| slot exhaustion before assignment | no provision teardown; existing failure disposition |
| assigned, provision fails, teardown succeeds | teardown precedes Failed write; slot released |
| assigned, provision fails, teardown fails | slot retained; original provision cause remains durable; existing cleanup error returns |
| cleanup and Failed write both fail | both attempted; existing observation/store error wins |

## S-GTI-BTR-03 — same-ID restart is prior-protection-first

```gherkin
@driving_port @in-memory @error @ordering @contract-shape:bounded-change
Scenario: A same-ID VM restart replaces lifecycle ownership only after teardown
  Given a successful real StartAllocation dispatch established exactly one Live lifecycle owner
    And that same allocation owns driver supervision and a structural slot
  When dispatch_with_network_provisioner drives RestartAllocation for the same id
  Then prior Driver::stop completes
    Before prior MtlsInterceptLifecycle::stop_alloc completes
    Before old structural-network teardown and slot release complete
    Before replacement provision starts
    Before replacement identity is present at Driver::start
    Before replacement Driver::start completes
    Before replacement MtlsInterceptLifecycle::start_alloc completes
```

The initial `Live` state must come from a successful production
`StartAllocation` dispatch. Preloading the Sim lifecycle map is forbidden
because it bypasses the ownership acquisition under test.

The registered invariant must cover these seeded partitions:

| Partition | Safety at the cut | Bounded convergence |
|---|---|---|
| clean | the complete order above; no stale prior owner | same ID ends exactly `Live`, with one replacement driver start and one replacement lifecycle `StartCompleted` |
| transient lifecycle-stop failure | no structural teardown or replacement event; old slot retained; lifecycle snapshot is exactly `TeardownPending` | one additional dispatch retries the same stop transition and converges to the same ID `Live` |
| transient structural-network teardown failure | lifecycle ownership is absent, old slot remains held, and no replacement event occurs | one additional dispatch completes teardown and converges to the same ID `Live` |

The same seeded evidence must also exercise all three replacement-stage
failure cuts. These checks are deliberately narrow so they do not duplicate
the focused error/cleanup contracts owned by the affected port:

| Replacement failure cut | BTR-3 lifecycle assertion | Assertion owned elsewhere |
|---|---|---|
| network provision | lifecycle remains absent; no identity, driver-start, or lifecycle-start event follows | S-GTI-BTR-02 owns exact structural unwind, slot disposition, Failed cause, and store-error precedence |
| identity acquisition | lifecycle remains absent; no driver-start or lifecycle-start event follows | existing identity failure tests own identity cleanup and typed error projection |
| driver start | lifecycle remains absent; no lifecycle-start event follows | existing driver-start failure tests own driver error classification and post-assignment cleanup |

Every failure returns the existing typed `ShimError`, prints the seed in the
invariant report, and admits no later replacement event. The checker must
reject stale or partial ownership, more than one replacement driver/lifecycle
start, a different allocation ID, or convergence beyond the one-additional-
dispatch bound. Exact reproduction is:

```text
cargo dst --seed 424242 --only same-id-restart-removes-prior-protection-before-replacement-provision
```

The deletion-sensitive negative control must make the pure checker fail after
either (a) removing/reordering lifecycle-stop completion in an otherwise
healthy trace or (b) treating `TeardownPending` as absence. It does not mutate
production or teach the Sim adapter the action-shim algorithm.

Prior driver non-`NotFound` failure remains a focused existing-error
complement. The three replacement-stage cuts extend only the lifecycle
absence/no-later-event oracle; they reuse the existing network, identity, and
driver error/cleanup evidence instead of reproducing it.

## Test architecture and placement

| Boundary | Treatment |
|---|---|
| action-shim dispatcher | real in-process `dispatch` / `dispatch_with_network_provisioner` |
| observation, driver, allocator, network | existing Sim/production ports; logical state only in Tier 1 |
| lifecycle | socket-free `SimMtlsInterceptLifecycle` in Tier 1; no worker, low-level intercept, listener, address, task, or guard |
| BTR-3 host effects | existing concrete `MtlsInterceptWorker` integration fixture, asserting only real listener closure and guard-drop/stop completion |

BTR-1 and BTR-2 evaluator bodies live under
`crates/overdrive-sim/src/invariants/`; their fixed-seed harness pins live under
`crates/overdrive-sim/tests/acceptance/`. The BTR-3 RED scaffold lives beside
those pins. DELIVER replaces it with the real invariant evaluator, adds the
socket-free adapter, and registers the canonical enum/`cargo dst --only` name
only when the approved production trait exists. DISTILL does not invent that
production API early.

## RED handoff

Only BTR-3 is RED in this amendment. Its Rust function uses the repository's
exact `#[should_panic(expected = "RED scaffold")]` convention. DELIVER must:

1. introduce only ADR-0089 §7's exact trait and socket-free adapter;
2. activate the real Start-then-Restart seeded body and remove the marker;
3. register the canonical invariant in the default catalogue and harness;
4. implement the pure checker and deletion-sensitive negative control; and
5. preserve the independent real-worker integration evidence without adding
   cross-port assertions to it.

No mutation run, reviewer, expectation, example, roadmap, or production
implementation is part of this DISTILL amendment.

## AT-completeness audit

| Category | Verdict | Evidence |
|---|---|---|
| C1/C3 boundaries and cardinality | PASS | BTR-1 pins 0/1/2 proposals; BTR-2 pins pre/post assignment; BTR-3 pins exactly one prior owner and one replacement start. |
| C2 state and order | PASS | Each scenario names the legal transition order; BTR-3 rejects every later event after either teardown failure cut. |
| C4 idempotency/inverse | PASS | Exact-terminal Stop is a no-op; absent lifecycle stop is idempotent; failed lifecycle stop remains `TeardownPending` and retryable. |
| C5 modes | N/A | No mode or feature-flag axis is introduced. |
| C6 errors | PASS | Existing observation, provision, teardown, identity, driver, lifecycle-install, and lifecycle-stop error families remain closed; all three replacement-stage failure cuts are pinned without duplicating their cleanup suites. |
| C7 degradation/concurrency | PASS | BTR-1 drives the production contention; BTR-2/BTR-3 cover each approved degraded ownership boundary with bounded convergence. |

**Result: complete for the approved BTR-1..3 scope; BTR-3 remains an
intentional RED handoff until DELIVER implements ADR-0089 §7.**
