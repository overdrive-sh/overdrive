# ADR-0133 — A retiring guest attachment counts against the node cap until its cleanup finishes

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R7. The counting policy, and the rule that no slot is reserved
for a replacement, are **user rulings of 2026-09-24 (rulings 1 and 5)**.
Proposed 2026-09-23 and revised 2026-09-24; reviewed by independent DESIGN
review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. It refines what ADR-0132's linearization point
counts.

## Context

A crash replacement starts a successor while the dead predecessor still holds
its lease. Retry-retaining cleanup (ADR-0135) can hold a retiring lease across
repeated cleanup failures. While a lease is retiring, its attachment's kernel
state (TAP, TCX link, bridge-guard member, intercept elements) still exists.

The 16,384 cap is a fixed placeholder, not a measured or promised density. It
was set without a capacity basis. What a node can really hold depends on its
resources: derived or configurable per-node guest-network capacity is
[GH #299](https://github.com/overdrive-sh/overdrive/issues/299), and real
node-wide CPU and memory accounting is
[GH #261](https://github.com/overdrive-sh/overdrive/issues/261). Whatever the
cap's value, the question here is whether an attachment whose cleanup has not
finished still occupies room.

Mature orchestrators disagree. The Kubernetes kubelet counts a terminating pod
in admission until it is fully terminated, including sandbox and network
teardown, and calls the one exception a bug (#104824). Nomad stops counting an
allocation at desired-stop, even while it still runs, and a port collision on
restart is on record. Upstream Kubernetes is moving toward counting terminating
pods (KEP-3939 `podReplacementPolicy: Failed`, KEP-3973). (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 5.1, 5.2 and 5.3, and Conflict 2.)

## Decision

A lease is **Admitted** from assignment until its allocation's cleanup begins.
It is then **Retiring** until its cleanup finishes and the lease is released. It
never returns to Admitted. Both states count against the cap: a replacement
starts only when there is room for it.

Cleanup begins at these points:

- after proven VMM quiescence on stop;
- at the start of genuine-terminal finalize cleanup;
- at the start of a restart predecessor's cleanup attempt;
- at the start of row-neutral reclaim (ADR-0136).

Replacement follows from the count:

- **Below the cap,** the successor is admitted first and the predecessor is
  cleaned up afterwards. Successor creation does not wait for predecessor
  cleanup (ADR-0106, user-ratified 2026-09-13). Both leases count during the
  overlap, which fits because the node has room.
- **At the cap,** the successor cannot be admitted while the predecessor's
  lease counts. ADR-0106 already anticipated that a held predecessor slot can
  make the successor meet exhaustion. The predecessor's cleanup therefore runs
  first (ADR-0136) and releases its lease, and only then is the successor
  admitted: recreate ordering. No slot is reserved for the successor; it gets
  no priority claim on the freed slot. Its own evaluation re-runs immediately
  after the predecessor's reclaim, so it normally takes the freed slot, but a
  concurrent placement may take it first, and then the successor waits like any
  placement.

Retiring accumulation is operator-visible. A stuck cleanup keeps its lease
Retiring and so visibly holds a slot:

- every admission refusal reports the held and retiring counts;
- every retirement, release, and failed cleanup emits a structured event
  naming the allocation;
- the operator's allocation view shows each allocation whose cleanup has not
  finished as cleanup-pending, never Running (ADR-0141).

## Alternatives considered

### Retiring leases do not count

Rejected by the user ruling. It admits a replacement when there is no room for
it, while the predecessor's kernel state still exists. It sits between Nomad and
the kubelet: stricter than Nomad because compute is proven gone, but looser than
the kubelet because the network residue no longer counts. Its companion, an
atomic predecessor-to-successor slot handover at retirement, frees nothing once
retiring leases count.

### Clean up the predecessor first at the cap (recreate ordering)

Adopted as the at-cap consequence of this decision. It is Kubernetes'
Deployment `Recreate` shape and KEP-3939's replace-after-full-termination
policy.

### Always clean up the predecessor first, even below the cap

Rejected. It delays every replacement behind cleanup when the node has room.
The user rejected exactly that availability gate when ratifying ADR-0106 on
2026-09-13, and no capacity reason applies below the cap.

### Reserve the released slot for the successor

Rejected by the user ruling of 2026-09-24: a replacement starts only when there
is room and gets no priority claim on a freed slot. A reservation would also
need a third lease state keyed to a workload and a handover operation, plus
release of the reservation when the workload is stopped or deleted. KEP-3939's
replace-after-termination policy reserves nothing, and the successor's immediate
re-evaluation already makes it the likely taker.

### Count only Running rows

Rejected. It excludes in-flight admissions, which is exactly the undercount the
proof reproduced.

## Consequences

Positive: the held attachment population never exceeds the cap, including
during replacement and cleanup failure. A stuck cleanup is visible as a held
slot rather than hidden capacity.

Negative:

- At the cap, replacement latency includes the predecessor's cleanup.
- A persistently failing cleanup keeps a slot for as long as it fails; its
  retry follows ADR-0136's backoff.
- A leased allocation with no row, which arises only when the first row write
  and the same arm's cleanup both fail, holds a slot until the next process
  boot (ADR-0136).
- Every cleanup path must mark retirement at its defined point.
