# E11 current Stable observation — proposed ADR-0101 revision 6 amendment

**Status:** Proposed; independent DESIGN review is required before DELIVER
step 03-01 resumes. This document is not an implementation approval and does
not claim that E11 is green.

**Author:** Codex acting as solution architect.

**Authority:** User-authorized focused DESIGN amendment for the proven E11
operator-observation gap. ADR-0101 revisions 3 and 4 retain their independent
approval provenance. The proposed revision 5 readiness-wake amendment remains
its own decision and review gate; this amendment does not accept, rewrite, or
replace it.

**Implementation boundary:** No production code, tests, examples, expectation
evidence, DES event, roadmap approval, execution-history edit, or commit is
authored by this DESIGN amendment.

The canonical roadmap remains unchanged. Its 03-01 acceptance criteria already
require the same allocation to remain `Running` and `Stable` while readiness
withdraws and restores traffic. The feature-delta and wave-decision entries
below record this proposed prerequisite without changing roadmap validation
metadata or historical execution outcomes.

## Scope and revalidated evidence

The fresh native-metal E11 attempt at `2026-09-09T16:45:19Z` passed the
traffic, readiness, lifecycle, restart and cleanup predicates that do not
require a post-transition Stable field:

- readiness `Pass -> Fail -> Pass` was observed in `338 ms`, `195 ms`, and
  `160 ms` in
  `verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/readiness-recovery.tsv:2-4`;
- the same allocation `alloc-service-vm-readiness-recovery-0` remained
  `Running` with `0` restarts in all three public descriptions at
  `verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:36-38,72-74,108-110`;
- the peer results were exact reply, unreachable/no exact reply, and exact
  reply; and
- the complete zero teardown delta appears at
  `verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:148-151`.

The same transcript exposes the remaining evidence gap. `Stable` appears only
in the initial submit stream at
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:22-32`.
The during and after descriptions at
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:69-88`
and
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:105-124`
show the readiness transitions and
`Running`, but no current Stable observation. The retained
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/readiness-recovery.tsv:1-4`
header has no Stable/current-terminal column.
The independent evidence audit in
`docs/feature/service-kind-vm-workloads/deliver/review-e11-evidence.md` and
the recapture disposition in
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/recapture-blocker.md`
therefore correctly leave E11 short of its full contract.

This is a public-surface omission, not a missing lifecycle fact. The current
production path is:

```text
ServiceLifecycle startup Pass
  -> Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }) }
  -> action-shim alloc-status write
  -> AllocStatusRowV3.terminal (durable current-row claim)
  -> GET /v1/allocs?job=<workload-id>
  -> current AllocStatusRowBody conversion drops terminal
  -> workload describe has no current Stable line

readiness Fail/Pass
  -> ProbeResultRow and existing ServiceLifecycle wake
  -> ServiceBackendRow healthy false/true
  -> no allocation-row rewrite, restart, or terminal-history append
```

The exact source facts are:

| Production entry point / owner path | Verified source evidence | Fact established |
|---|---|---|
| Startup Stable decision | `crates/overdrive-reconcilers/src/service_lifecycle.rs:576-598` | `ServiceLifecycle` emits the existing `Action::FinalizeFailed` with `Some(TerminalCondition::Stable { settled_in_ms, witness })` for a Running allocation whose startup contract passed. |
| Durable row publication | `crates/overdrive-control-plane/src/action_shim/mod.rs:1520-1750`, especially `:1680-1748`; `crates/overdrive-core/src/traits/observation_store.rs:1412-1420` | The action shim writes the action's typed terminal claim onto the existing `AllocStatusRow.terminal`; `AllocStatusRowV3` already stores that field. The Stable arm keeps the allocation Running and retires only startup supervision. |
| Readiness transition owner | `crates/overdrive-reconcilers/src/service_lifecycle.rs:1084-1178`; `:1181-1216` | Readiness recomputes only `ServiceBackendRow.backends[*].healthy`; it does not change the allocation row's `state`, `terminal`, `restart_count`, or lifecycle occurrences. |
| Existing proof that Stable survives the flap | `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:594-603,644-652,693-703` | Seeded production-owner-path composition observes baseline `TerminalCondition::Stable`, then asserts the same terminal claim after readiness Fail and Pass, with Running, zero restarts and unchanged occurrence history. |
| Snapshot HTTP route | `crates/overdrive-control-plane/src/handlers.rs:1219-1257`; `crates/overdrive-cli/src/http_client.rs:322-339` | The existing `GET /v1/allocs?job=<id>` response carries the current rows consumed by the CLI; no second read or route is needed. |
| Current wire projection omission | `crates/overdrive-control-plane/src/api.rs:376-424`; `crates/overdrive-control-plane/src/handlers.rs:95-173` | `AllocStatusRowBody` has no current `terminal` field, and `From<AllocStatusRow>` projects `exit_code` from the claim but otherwise drops the current typed claim. The nested `last_terminated.terminal` is a different, prior-survivor fact. |
| Current operator renderer omission | `crates/overdrive-cli/src/render.rs:1095-1145` | The Service body renders `Alloc / State / Restarts / Since`, cause details and `last terminated` details, but no current Stable claim. |
| Current command shape | `crates/overdrive-cli/src/cli.rs:94-109`; `crates/overdrive-cli/src/commands/workload.rs:114-207` | `workload describe` already performs the one snapshot read and has no output-mode or replay argument. |
| Submit stream boundary | `crates/overdrive-control-plane/src/streaming.rs:886-985` | The existing Service submit stream emits the submit's Accepted/terminal result and closes; it is not a post-submit snapshot or replay surface. |

This is a complete production-owner path in the default `serve` composition.
It does not require a fabricated row, a test-only state, a second allocation,
or a scheduler hypothesis. The current durable fact exists; only its
operator projection is absent.

## Decision

Amend ADR-0101 with one additive projection on the existing allocation
snapshot and one presence-guarded line in the existing Service
`workload describe` renderer:

1. Append one field named exactly `terminal` to the existing public
   `AllocStatusRowBody`. Its value is the current row's already-persisted
   `AllocStatusRow.terminal`, projected with the existing
   `TerminalCondition` type. It is not a new condition type, a boolean, a
   lifecycle state, or a derived readiness label.
2. Keep `GET /v1/allocs?job=<id>`, `WorkloadCommand::Describe { id }`,
   `DescribeArgs`, `WorkloadDescribeOutput`, and the existing HTTP client
   method unchanged. The handler performs the mechanical field copy while
   projecting each row.
3. In the existing `WorkloadKind::Service` rendering arm, emit exactly
   `    terminal: Stable` for a row whose current typed claim is
   `Some(TerminalCondition::Stable { .. })`. Emit no such line for `None` or
   any other `TerminalCondition`, and do not add a table column. Job and
   Schedule rendering is unchanged.

This is the only revision-6 public field. It is a read-only projection of an
existing durable input and does not change who decides Stable, who decides
readiness, or who owns terminal/restart/cleanup behavior.

ADR-0084's existing `AllocStatus` row-backed observation and interest
vocabulary is unchanged by revision 6. The proposed revision-5
`ProbeResult` wake is the only amendment that changes that in-process wake
vocabulary; this amendment consumes the resulting existing allocation
snapshot and adds no observation event or interest.

## Exact API and rendering contract

### Existing HTTP snapshot row

In `crates/overdrive-control-plane/src/api.rs`, append this field after the
existing `last_terminated` field; do not insert it between existing fields or
rename any existing field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub terminal: Option<overdrive_core::transition_reason::TerminalCondition>,
```

The complete `AllocStatusRowBody` type remains the existing type and gains no
other field, parameter, method, enum or wrapper. The `terminal` JSON key is
omitted when the option is `None`; when present, its JSON is the unchanged
serde projection of the existing `TerminalCondition` enum. For example, a
Stable row carries the existing tagged shape (with its full
`settled_in_ms` and `witness` payload), not a second hand-written Stable
object:

```json
"terminal": {
  "kind": "stable",
  "data": {
    "settled_in_ms": 1234,
    "witness": {
      "probe_idx": 0,
      "role": "startup",
      "mechanic_summary": "tcp 0.0.0.0:18081",
      "inferred": false
    }
  }
}
```

`rows[i].terminal` means the terminal-condition claim carried by the current
row. `rows[i].last_terminated.terminal`, when present, continues to mean the
terminal claim from the prior terminal observation that this allocation
survived. The two fields must not be joined, substituted, or renamed.

In `crates/overdrive-control-plane/src/handlers.rs`, the row conversion gains
exactly this mechanical assignment in its existing `Self { ... }` literal:

```rust
terminal: row.terminal,
```

No branch on `AllocState`, no readiness lookup, no recomputation from
`ProbeResultRow`, and no terminal-kind conversion is permitted. The existing
`exit_code` projection remains unchanged. `AllocStatusResponse` and its
top-level replica fields remain unchanged; Stable is allocation-scoped, so no
top-level aggregate is introduced.

### Existing CLI snapshot rendering

The current Service table retains exactly its existing columns and order:

```text
Alloc                    State        Restarts   Since
```

Immediately after each Service allocation table row, before the existing
cause and `last terminated` detail blocks, the renderer emits exactly one
line when and only when the row's current `terminal` is
`Some(TerminalCondition::Stable { .. })`:

```text
    terminal: Stable
```

The renderer does not print `settled_in_ms`, `witness`, `Custom.detail`, or a
Debug representation. The machine-readable HTTP field remains the exact
typed source for those payloads; the plain operator output exposes the one
Stable fact E11 needs. A current non-Stable claim is represented by existing
state/reason/cause surfaces and produces no new line. The existing streaming
`format_service_stable_summary` remains unchanged and is not reused: it is a
submit-event summary with workload-level context, not an allocation snapshot.

The line is presence-guarded and source-direct. It must not be inferred from
`AllocStateWire::Running`, readiness `Pass`, `probe_results`, initial stream
history, or `last_terminated`. In production, the lifecycle invariant pairs
`TerminalCondition::Stable` with `AllocState::Running`; the renderer reports
the typed claim and leaves any invariant diagnosis to the existing state and
typed API consumers.

## Lifecycle Gate Ownership

The amendment adds an observation projection only. It does not add a gate or
move an owner.

| Signal / state | Owner and promise | Changed by this amendment | Explicitly unaffected |
|---|---|---|---|
| Allocation `Running` | Existing action shim and selected driver; an accepted Running row means the driver/Beacon start contract succeeded | Nothing | VM process, Stable, readiness, liveness and restart |
| Service `Stable` | Existing `ServiceLifecycle` startup branch; the typed claim is published on the current allocation row | Only the read projection of the already-published claim | Startup probes, `Running`, readiness, liveness, terminal dominance, restart and cleanup |
| `Backend.healthy` | Existing `ServiceLifecycle` readiness branch; it projects the current readiness policy and terminal veto into the complete `ServiceBackendRow` | Nothing | Stable claim, allocation state, restart count, lifecycle history and consumer ownership |
| Current operator terminal observation | Existing `GET /v1/allocs` handler and `workload describe` renderer | Append `AllocStatusRowBody.terminal` and render `terminal: Stable` for Service rows | No lifecycle decision, persistence write, broker wake, cadence, consumer acknowledgement or terminal transition |
| Liveness termination | Existing `ServiceLifecycle` liveness detector | Nothing | `WorkloadLifecycle` restart/finalization authority and probe supervision |
| Restart decision/action | `WorkloadLifecycle` remains sole restart authority | Nothing | Restart budget, allocation identity, VM exit watcher and cleanup |

**G-STABLE-OBS declaration:** adding, removing, or temporarily omitting this
operator projection cannot gate or redefine `Stable`, `Running`, readiness
eligibility, terminal dominance, restart/finalization, or cleanup. It can only
change what the already-authoritative current row says at the HTTP/CLI
observation boundary.

Readiness Fail and Pass retain the existing ordering and ownership:
`ProbeResultRow` persistence, existing wake/evaluation, ServiceLifecycle
backend projection, and asynchronous backend consumers. They do not rewrite
the allocation's current `terminal`, and this amendment adds no event,
subscription interest, broker operation, or persistence mutation. A genuine
later terminal/restart row may replace the current claim under the existing
LWW lifecycle path; a snapshot then reports that current row rather than
claiming Stable from history.

## Ordering, cancellation and retry boundaries

| Boundary | Required behavior |
|---|---|
| Lifecycle publication → row | Existing `ServiceLifecycle` and action-shim ordering remains authoritative. A Stable claim is visible here only after the existing row write accepts it. No second Stable write or acknowledgement is introduced. |
| Readiness Fail/Pass → current row | The existing readiness path changes only `ServiceBackendRow.healthy`; it does not rewrite `AllocStatusRow.terminal`. The current Stable claim therefore remains available during both readiness transitions while the allocation remains Running. |
| Row → HTTP snapshot | `GET /v1/allocs?job=<id>` reads the same allocation snapshot as before. The conversion copies `row.terminal` without an additional store read, join, retry, or ordering barrier. |
| HTTP snapshot → CLI | `workload describe` keeps its one existing async request and existing error behavior. No detached task, polling loop inside the CLI, retry policy, or stream replay is added. The E11 example may continue its existing bounded external describe sampling. |
| Concurrent row update | A describe call may observe the current LWW row available at its read. If a genuine terminal or replacement row wins before the read, the response reports that row; this amendment adds no stronger snapshot or monotonicity guarantee. |
| Cancellation/shutdown | No new future or task is introduced. Existing server cancellation, request cancellation, convergence draining, VM probe cancellation and terminal cleanup remain unchanged. A cancelled describe request has the existing transport outcome. |
| Failed read/serialization | Existing typed handler/transport/body-decode failures remain failures. `None` means the current row has no terminal claim; it must not be used as a substitute for an unreadable row. |
| Consumer ordering | Mesh, DNS and dataplane consumers still consume the existing `ServiceBackendRow` asynchronously. The operator projection does not wait for, acknowledge, or infer consumer application. |

The field is therefore an observation of one point-in-time current row, not a
history query and not a claim that Stable remains true after the allocation has
entered a genuine terminal state. E11's before/during/after calls must capture
the three Running rows and their direct Stable claims.

## Compatibility, privacy and non-E11 boundaries

This is an additive DTO change on an existing greenfield HTTP response:

- the route, HTTP method, query, command verb, command argument and response
  envelope are unchanged;
- the field is appended to preserve existing serialized field order;
- `skip_serializing_if = "Option::is_none"` keeps rows without a claim
  byte-identical on the JSON output, while `serde(default)` lets a new client
  read an older response;
- the existing serde derives ignore the additive key for an older client
  struct, and the existing `TerminalCondition`/`ToSchema` implementation is
  reused; no separate OpenAPI enum or version negotiation is created;
- `AllocStatusRowV3`, its rkyv envelope, redb table, LWW stamp, lifecycle
  occurrence history and `last_terminated` semantics are unchanged; no data
  migration is needed; and
- Job and Schedule consumers receive an additive optional field on the shared
  row DTO but their render branches, verdict logic, tables and lifecycle
  behavior do not change. Only a Service row with a current Stable claim gets
  the new plain-text line.

The existing control-plane authentication and authorization boundary protects
the snapshot route. The field introduces no certificate, key, token or new
network endpoint. `TerminalCondition::Stable` already exists on the submit
stream and inside `LastTerminatedBody`; reusing its exact typed payload adds no
new category of credential or secret. The witness's mechanic summary is
already operator-facing probe metadata. The opaque `Custom.detail` payload is
not newly invented or reinterpreted; it follows the existing typed terminal
contract. The implementation review must nevertheless verify that the
existing endpoint's privacy policy remains sufficient for its already-exposed
terminal payloads, and must not add logging of the full payload or a new
redaction convention.

The following are explicitly out of scope:

- `stable: bool`, `is_stable`, `current_stable`, or any derived duplicate of
  `TerminalCondition::Stable`;
- a new `StableObservation`, `TerminalKind`, enum variant, trait, method,
  endpoint, CLI option, JSON mode, stream replay, history query or ledger-only
  label;
- changing `AllocStateWire`, making Stable a lifecycle-terminal state,
  changing readiness thresholds/counters, startup policy, terminal dominance,
  liveness, restart/finalization, VM lifecycle, probe supervision or cleanup;
- changing `ObservationRow`, `ObservationRowKind`, the revision-5 wake
  contract (or ADR-0084's existing allocation-row vocabulary), broker target resolution, cadence, persistence, consumer retry,
  or mesh/DNS/dataplane acknowledgement; and
- adding a top-level replica Stable aggregate, cross-allocation consistency
  promise, new store read, cache, persistence journal or recovery protocol.

## Alternatives and disposition

| Alternative | Disposition |
|---|---|
| Append current `terminal: Option<TerminalCondition>` and render Stable-only (selected) | Reuses the existing durable row field, enum, HTTP snapshot, command and renderer. It directly exposes the missing fact with one additive field and no new owner or protocol. |
| Add `stable: bool` or `is_stable: bool` | Rejected. It is a derived second truth, cannot preserve the existing `settled_in_ms`/witness claim, and would invite a readiness-to-Stable inference. The existing typed variant is the source of truth. |
| Add `current_terminal` with the same type | Rejected. It duplicates the established core field name and creates two names for the same current-row claim. The precise distinction is carried by nesting: row `terminal` versus `last_terminated.terminal`. |
| Create a `StableObservation` wire type or new enum | Rejected. `TerminalCondition::Stable` already carries the exact accepted payload and derives serde/ToSchema. A wrapper would invent public surface and create byte-equality drift. |
| Reuse `last_terminated.terminal` | Rejected. That field explicitly describes the prior terminal observation the allocation survived; it is absent on the first Stable row and has the opposite temporal meaning. |
| Treat readiness `Pass` as Stable | Rejected. Readiness is a post-Stable eligibility input; a readiness Pass cannot prove startup Stable and a readiness Fail must not erase Stable. |
| Add a new describe route/flag/JSON mode or replay the submit stream | Rejected. The existing snapshot route already reads the authoritative row, while a new mode or replay would add API/protocol surface and still need the same row projection. |
| Derive Stable from the evidence ledger or initial submit transcript | Rejected. That would fabricate an operator observation outside the production boundary and would not prove post-transition state. |
| Add a top-level response Stable aggregate | Rejected. Stable is allocation-scoped; replica aggregation would require an unapproved policy for mixed allocation claims and would be a new semantic surface. |

## Verification and proof obligations

No implementation may advance from this amendment on design prose alone. After
independent DESIGN approval, the original step-03-01 crafter owns the bounded
implementation and its implementation review. The crafter must not add any
API beyond this contract.

### In-process contract evidence

The implementation review must prove through the existing production
composition and typed adapters:

1. `AllocStatusRowBody` conversion copies a Stable claim byte-for-byte,
   including `settled_in_ms` and every `ProbeWitness` field, and copies `None`
   as `None`.
2. JSON serialization omits `terminal` for `None` and emits the exact existing
   tagged `TerminalCondition` shape for Stable; deserialization accepts both
   the new field and an old payload without it.
3. The Service renderer preserves the existing table and emits exactly one
   `    terminal: Stable` line per current Stable Service row, with no line for
   `None` or another terminal variant. Job and Schedule output remains
   unchanged.
4. A production-owner-path test or existing composition assertion proves that
   readiness Fail and Pass leave the same current `AllocStatusRow.terminal`,
   `AllocState::Running`, allocation ID, restart count and lifecycle history.
   Tests must not seed a replacement Stable row merely to exercise the DTO.
5. Tests remain in-process; they do not spawn the built Overdrive binary or
   use the evidence runner. Any source-local pure-function property carries
   the exact required rustdoc declaration
   `/// CONTRACT_SHAPE: pure-function.`; other tests declare their bounded
   shape according to the repository rules.

### Fresh native E11 recapture

After implementation and its review, rerun the checked-in
`readiness-recovery` example through the built default-feature binary on
native metal. The existing run must still prove the traffic, readiness,
Running/zero-restart and zero-cleanup predicates. In addition, the public
transcript and extracted ledger must carry the current Stable observation from
each of the three existing `workload describe` calls, not from the initial
submit stream.

The amended ledger header is pinned exactly as:

```text
phase	readiness	terminal	observed_at_ms	detected_at_ms	transition_latency_ms	client_started_at_ms	client_elapsed_ms	lifecycle	restarts	peer_result
```

For `before`, `during`, and `after`, the `terminal` cell must be exactly
`Stable`, extracted from the corresponding public line
`terminal: Stable`; it must not be a constant copied from the initial stream,
an internal row read, or a readiness-derived boolean. The existing lifecycle
cell must remain `Running`, restart cell `0`, peer-result cells must retain
exact-reply / unreachable-no-exact-reply / exact-reply, both transition
latencies must remain at most 2,000 ms, and the zero teardown delta must remain
present.

The existing describe polling success predicate must require the direct
`terminal: Stable` line in the same captured response as the expected
readiness outcome before recording that phase's observation timestamp. A
separate later request, an initial stream line, or a value carried forward from
the previous phase does not satisfy the phase row.

An independent evidence auditor must inspect the fresh transcript, ledger,
metadata, provenance, and cleanup evidence and record the result in the
existing evidence-audit convention at
`docs/feature/service-kind-vm-workloads/deliver/review-e11-evidence.md`.
The earlier NEEDS_RECAPTURE disposition is historical evidence, not an
approval of this amendment.

## Independent design-review gate

This amendment is complete only as a proposed decision until an independent
DESIGN review verifies the revalidated production reachability, exact field
name/type/serde behavior, exact handler copy, exact renderer line, current
versus prior terminal semantics, lifecycle ownership, cancellation/retry/
ordering boundaries, compatibility/privacy implications, rejected
alternatives, and the bounded in-process plus native E11 obligations.

The mandatory independent review artifact path is:

`docs/feature/service-kind-vm-workloads/design/review-amendment-e11-stable-observation.md`

The review must be recorded as native Markdown with its findings, evidence,
verification, remediation dispositions and verdict. A chat verdict alone is
not sufficient. DELIVER step 03-01 remains blocked until this amendment's
review, and the separate revision-5 readiness-wake review, return `APPROVED`.
The review must not broaden this amendment into a new persistence, broker,
cadence, recovery, lifecycle or consumer architecture. No production,
test, harness, evidence or execution-history edit is claimed by DESIGN.
