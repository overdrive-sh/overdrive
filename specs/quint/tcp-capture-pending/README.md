# Quint specification: TCP capture pending-place protocol

This subsystem model-checks the bounded shared-place protocol added by the
user-approved `TCP-CORRELATION` correction for STEP 11-06. The accepted design
is in `docs/feature/netns-density-295/feature-delta.md` § *[REF] TCP capture
correlation*; its independent folded-contract review is
`docs/feature/netns-density-295/deliver/design-review-11-06-correlation.md`
iteration 9.

Run:

```text
cargo xtask lima run -- cargo xtask quint typecheck
cargo xtask lima run -- cargo xtask quint check --subsystem tcp-capture-pending --jobs 3
```

Recorded evidence is in `evidence/summary.md`. It is written only by the
runner's `--record` mode and checked with:

```text
cargo xtask lima run -- cargo xtask quint verify-evidence --subsystem tcp-capture-pending
```

## Abstraction and bound

The production owner has one exact count from acceptance of each
`CapturedConn` through successful `attach_outbound` or consuming `abort` /
`close`. That count includes Fast Open children held internally while their
child-correlation row is `Pending`, and its production bound is
`GUEST_TCP_PENDING_MAX = 4,096`.

The model symmetry-reduces that count to two abstract places. Four one-shot
abstract children are sufficient to fill both places, attempt and reset an
overflow child, release one held place, and admit another child onto the freed
place. The bound is deliberately about simultaneous ownership, so two places
cover the relevant interleavings; it does not claim performance or numerical
verification of 4,096 kernel objects.

Each child has exactly one adapter phase:

- `free`: it holds no shared pending place;
- `correlation_pending`: the adapter accepted it and is waiting for its
  child-correlation row to become `Ready`; or
- `emitted`: the existing capture event was emitted, and its `CapturedConn`
  still owns the same place until attach, abort or close consumes it.

The kernel-facing correlation row is abstracted to `missing`, `pending`,
`ready` or `refused`. Acceptance at a free place charges exactly once. A
`pending -> ready -> emitted` sequence preserves the accepted place. A ready
row is consumed once. Missing/refused correlation and child close reset the
child and release the place without an event, owner flow or control traffic.
Attach, abort and close release an emitted handle's place. Overflow resets
only when both places are held, and the correlation collection has no separate
capacity refusal below that bound.

`ownerFlow` and `controlTraffic` are model-only outcome markers. They record
that a successfully attached emitted handle may proceed into the existing
downstream owner lifecycle. Their only purpose here is to state the negative
contract: a rejected capture cannot create either outcome. The model does not
specify the downstream flow/control protocol.

## Invariants

| Invariant | Approved rule |
|---|---|
| `OnePhasePerChild` | A child is in one of `free`, `correlation_pending` or `emitted`, never two phases. |
| `OneSharedCharge` | Every live Pending/emitted child owns one unique place; Pending-to-emitted preserves that place; the two-place bound is never exceeded. |
| `OverflowOnlyAtBound` | The overflow-reset outcome occurs only when all shared pending places are held. |
| `NoCorrelationCapacityRefusalBelowBound` | Correlation storage introduces no admission refusal while a shared pending place is free. |
| `ReadyEmitsOnce` | A child produces at most one existing capture event; downstream outputs require that event. |
| `RejectionCreatesNoOwnerOutput` | Missing/refused/closed/overflow capture produces no event, owner flow or control traffic. |
| `AdmissionAndReleaseAccounting` | Every acceptance is charged or overflow-reset; every release returns the child to `free` with no place. |
| `Safety` | Conjunction of all rules above. |

The single hazard variant removes the combined decision's no-earlier-capacity
rule by allowing a correlated child to be refused while a place is free.
Exhaustive TLC must find the resulting `Safety` violation, and the normal
ready-at-accept critical run must fail on the weakened instance.

## Critical scenarios

| Scenario | Named run |
|---|---|
| Fast Open Pending becomes Ready, emits once and transfers the same charge through attach | `PendingToReadyTransfersTheSameCharge` |
| Ready at accept emits once; abort releases the place and a later child reuses it | `ReadyAtAcceptEmitsOnceThenAbortReusesThePlace` |
| Missing, refused and child-close paths release without event/flow/control | `MissingRefusedAndChildCloseReleaseWithoutOutputs` |
| Two children fill the bound, overflow resets, and a later release permits another admission | `OverflowResetsOnlyAtTheBoundThenAReleaseMakesRoom` |
| Attach, abort and close each release an emitted handle's place | `AttachAbortAndCloseEachReleaseTheirPlace` |

## Proof boundary and production evidence

This model proves the finite abstract protocol for all interleavings in its
two-place instance. It does not establish that the Rust/Aya implementation
refines the model automatically. Per the user's bounded approval, this
subsystem adds no `quint-connect` driver, production accessor, Sim seam, test
lane or runtime API.

The independent implementation layer is the existing one-cell Tier-3 guest
test in
`crates/overdrive-init/src/guest_flow/real_kernel.rs`:
`tcp_correlation_preserves_exact_identity_and_cleans_transients_for_every_handshake_path`,
invoked by the existing slot-11 wrapper in
`crates/overdrive-init/tests/integration/in_guest_lane.rs`. That production
composition exercises real `Connect4`, final-ACK callbacks,
`KernelGuestOwnerIo`, actual data-in-SYN Fast Open, the adapter-held Pending
place, exact one-time emission, the 4,096 boundary, consuming release, direct
uncorrelated reset and map cleanup. The DISTILL review and evidence mapping
are recorded in
`docs/feature/netns-density-295/deliver/distill-review-11-06-correlation.md`.

## Not modelled

- TCP option bytes, endian encoding, SYN/non-SYN placement or retransmission;
- BPF program types, callback order, map representation, BTF, pinning, file
  descriptors or Aya loading;
- socket syscalls, accept queues, listen/TCP Fast Open backlog arithmetic,
  syncookies or kernel cleanup mechanics;
- application cookie/original-destination value correctness;
- owner event, flow or control wire formats and their downstream lifecycle;
- performance, wake delivery, the one-second audit deadline, or liveness in
  the absence of an environment transition; and
- any persistent, runtime-adopted or cross-boot state.

Those are design assumptions or production effects discharged by the accepted
design's Linux-source/spike evidence and the exact Tier-3 cell above. Passing
this model alone is not evidence that those mechanisms are correct.
