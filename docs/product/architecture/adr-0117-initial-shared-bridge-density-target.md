# ADR-0117 — Size the shared bridge for a fixed placeholder population of 16,384 guest network attachments per node

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records D-295-3 with CAP-295-A; ADR-0125 records
PORT-295-C.

**Amended 2026-09-24 by user ruling (with D-295-R7), accepted with the #295
correctness-recovery replacement DESIGN on the same date.** This decision is
operative in code committed at HEAD `db3af700` on the #295 feature branch (the
cap constant, `scheduler.rs:50`; not merged to `main`). As accepted on
2026-09-16 it was titled "Initial measured shared-bridge contract is 16,384
guest network attachments per node" and read: *"Set the initial shared-bridge
measured network-attachment contract to 16,384 simultaneously active guest
attachments per node. Completion requires controlled native measurement of
TAP/bridge/TCX/map/guard/address/shared-listener and shared-set state at that
population."* The user ruled that 16,384 is an arbitrary threshold set without a
capacity basis: what a node can hold depends on the resources the box has. The
Decision below states the amended contract; the number is unchanged, but it is
now the current fixed placeholder cap, not a measured, meaningful, or promised
density, and not a completion target. Derived
or configurable per-node guest-network capacity is
[GH #299](https://github.com/overdrive-sh/overdrive/issues/299); real
node-wide CPU and memory accounting is
[GH #261](https://github.com/overdrive-sh/overdrive/issues/261). Every held
attachment, retiring ones included, counts against the cap (ADR-0133). No
end-to-end VM density is claimed: 16,384 VM deployments fit the baseline node's
resource model only because of the workload-local CPU/memory accounting defect
recorded in GH #261 (proof-findings §3.2 finding 2, D-295-R9).

## Context

The removed `NetSlot` model caps networked allocations at 4,096. GH #112's
100,000 unikernel report motivates re-examination but is not an Overdrive
capacity measurement and does not define the workload size, connection count,
VMM process model, or recovery budget. Parts A-D prove bounded two-guest
mechanisms only, not end-to-end capacity.

## Decision

Size the shared-bridge design for a fixed **placeholder** population of 16,384
held guest attachments per node: the address prefix, map sizes, and measurement
profiles below are derived from it, and the admission cap enforces it
(ADR-0121, ADR-0132, ADR-0133). The number is not a density claim or promise.
The T1-BASE and T1-PORT4 profiles measure TAP/bridge/TCX/map/guard/address/
shared-listener and shared-set state at that population to size runtime bounds
(audit, quiescence, and restore latency) and to report the attachment-state
cost; Parts A–D's bounded evidence does not supply those numbers. CAP-295-A
explicitly excludes any simultaneous application-flow, enforcement-handle,
throughput, FD, pump-thread, or stack population. This is also not a claim that
any server can supply 16,384 VMMs, vCPUs, or guest RAM.

At that target the approved D-295-2 direction carries 16,384 pinned TCX links,
endpoint-map entries, and managed-TAP guard members. Part C measured one shared
program at 296 verified instructions and 4,096 bytes memlock; a 16-entry
preallocated endpoint map at 3,840 bytes; an eight-entry counter map at 368
bytes; 50.811 ms one-time load/verifier; and 7.624/7.943 ms attach+pin per TAP.
Linear extrapolation would make 16,384 serial cold attaches roughly 127.5 seconds,
so attach concurrency/churn and per-link memory are mandatory measurements,
not assumed capacity.

Use a node `/16` guest prefix, reserving network, broadcast, and gateway, so the
pool has 65,533 usable addresses, about four times the placeholder population. D-295-5
contributes exactly two listener FDs/tasks. ADR-0125 fixes the #295 nft rule
count at a constant, independent of allocations and listeners, while dynamic
element cardinality remains N managed-IP + N outbound-source elements plus M
distinct TCP destination-port elements. The
`8N+6M` logical-key formula covers only IP-family sets; the bridge
`managed_taps` set adds N IFNAMSIZ-bounded names and their own memory/churn.
Valid Service port cardinality remains unbounded. The feature delta defines two
reproducible receipts at N=16,384: M=0 and four TCP ports per attachment
(M=65,536). Higher M remains valid product input outside #295's receipt.

Controlled native benchmarks at that population measure attachment
creation/removal, nft element update rate, bridge/FDB memory,
capability/allowed-port index cost, TCX/map/pin state, and attachment recovery.
Their results size runtime bounds and are reported as cost; they do not turn the
placeholder into a delivered density. Part D settles listener/task cardinality
but its point RSS/timings and same-node functional evidence are not a
benchmark.

## Alternatives considered

### 32,768 guest network attachments

Retains `/16` headroom but approximately doubles TAP, map, set-element, and
recovery pressure while rule/listener/task cardinality stays constant. A full 32,768-active
predecessor/successor overlap needs 65,536 addresses, three more than the
65,533 usable addresses in `/16`. Rejected as the initial #295 contract.

### 100,000 guest network attachments

Requires at least `/15`, a larger endpoint map than the approved 65,536-entry
shape, 100,000 managed-IP elements, 100,000 outbound elements, M inbound
elements, and roughly 800 GiB
even at 8 MiB guest RAM. Rule/listener/task cardinality remains constant, but
Parts A–D prove none of those attachment resources. Rejected as #295's target.

### No sizing number at all

Rejected. Without a number the architecture cannot choose address headroom,
map/set sizing, recovery inventory, or the population at which runtime bounds
are measured.

### A capacity derived from the node's resources

Not decided here. It is the right basis for a node's real capacity, and it is
[GH #299](https://github.com/overdrive-sh/overdrive/issues/299) together with
[GH #261](https://github.com/overdrive-sh/overdrive/issues/261). Until then the
fixed placeholder is the only bound.

## Consequences

Positive: a fixed sizing basis with explicit assumptions and headroom, and a
bound on the kernel attachment state one node holds. Negative: the bound is
arbitrary and the same on every node; runtime bounds sized at this population
must be re-measured if the placeholder changes. A 60 s
cleanup would require at least 273 complete allocation cleanups/s plus `(2N+M)/60`
IP-set removals/s plus `N/60` bridge-set removals/s, with no current evidence for
that bound. Connection
pump scaling is neither promised nor partially redesigned; it is tracked by
[GH #300](https://github.com/overdrive-sh/overdrive/issues/300). Admission
enforcement is independently recorded in ADR-0121.
