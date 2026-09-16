# ADR-0117 — Initial measured shared-bridge contract is 16,384 guest network attachments per node

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records D-295-3 with CAP-295-A; ADR-0125 records
PORT-295-C.

## Context

The removed `NetSlot` model caps networked allocations at 4,096. GH #112's
100,000 unikernel report motivates re-examination but is not an Overdrive
capacity measurement and does not define the workload size, connection count,
VMM process model, or recovery budget. Parts A-D prove bounded two-guest
mechanisms only, not end-to-end capacity.

## Decision

Set the initial shared-bridge **measured network-attachment contract** to 16,384
simultaneously active guest attachments per node. Completion requires controlled
native measurement of TAP/bridge/TCX/map/guard/address/shared-listener and
shared-set state at that population; Parts A–D's bounded evidence does not
satisfy it. CAP-295-A explicitly excludes any simultaneous application-flow,
enforcement-handle, throughput, FD, pump-thread, or stack population. This is
also not a claim that every server can supply 16,384 VMMs, vCPUs, or guest RAM.

At that target the approved D-295-2 direction carries 16,384 pinned TCX links,
endpoint-map entries, and managed-TAP guard members. Part C measured one shared
program at 296 verified instructions and 4,096 bytes memlock; a 16-entry
preallocated endpoint map at 3,840 bytes; an eight-entry counter map at 368
bytes; 50.811 ms one-time load/verifier; and 7.624/7.943 ms attach+pin per TAP.
Linear extrapolation would make 16,384 serial cold attaches roughly 127.5 seconds,
so attach concurrency/churn and per-link memory are mandatory measurements,
not assumed capacity.

Use a node `/16` guest prefix, reserving network, broadcast, and gateway, so the
pool has 65,533 usable addresses and about four times T1 headroom. D-295-5
contributes exactly two listener FDs/tasks. ADR-0125 fixes nft rule count at
eleven #295 rules while dynamic element cardinality remains N managed-IP + N
outbound-source elements plus M distinct TCP destination-port elements. The
`8N+6M` logical-key formula covers only IP-family sets; the bridge
`managed_taps` set adds N IFNAMSIZ-bounded names and their own memory/churn.
Valid Service port cardinality remains unbounded. The feature delta defines two
reproducible receipts at N=16,384: M=0 and four TCP ports per attachment
(M=65,536). Higher M remains valid product input outside #295's receipt.

The target becomes a delivered claim only after controlled native benchmarks
measure attachment creation/removal, nft element update rate, bridge/FDB memory,
capability/allowed-port index cost, TCX/map/pin state, and attachment recovery.
Part D settles listener/task cardinality but its point RSS/timings and same-node
functional evidence are not a benchmark.

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

### No quantified target

Rejected. Without a number the architecture cannot choose address headroom,
map/set sizing, recovery inventory, or decide whether attachment measurements
pass.

## Consequences

Positive: a falsifiable fourfold improvement target with explicit assumptions
and headroom. Negative: implementation cannot claim completion from functional
tests alone; it needs attachment/set benchmark methodology and results. A 60 s
cleanup would require at least 273 complete allocation cleanups/s plus `(2N+M)/60`
IP-set removals/s plus `N/60` bridge-set removals/s, with no current evidence for
that bound. Connection
pump scaling is neither promised nor partially redesigned; it is tracked by
[GH #300](https://github.com/overdrive-sh/overdrive/issues/300). Admission
enforcement is independently recorded in ADR-0121.
