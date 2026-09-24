# ADR-0137 — The intercept owner converges dynamic intercept members to empty during fresh-process boot

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R12. Proposed 2026-09-23; reviewed by independent DESIGN review
rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. Makes executable ADR-0125's accepted
requirement that boot clear stale elements and read back empty sets before
listener-target convergence. It assigns that clear an owner, a port, and an
ordering. Exact signatures live only in the #295 feature delta.

## Context

After process loss, the shared IP intercept program keeps its dynamic members:
managed guest addresses, outbound sources, and inbound destination tuples. The
proof `serve_killed_restart_boot_clear` restarted in killed mode through the real
composition root:

- VMM reclamation ran first.
- Startup was then refused with `shared mTLS rule/set convergence failed`. The
  worker's fresh-process start observes the shared program, and that
  observation rejects non-empty sets.
- No production code called the lower boot-clear adapter, and the three stale
  members were never deleted.

Rebuilding owned dataplane state before admitting new work is established.
kube-proxy's nftables proxier periodically flushes and rebuilds its own chains
and forces a full resync after any failure; Cilium dumps its endpoint map at
restart and deletes every entry not tied to a live workload. Cilium's default is
a hitless restore of live workloads, which #295 does not need because boot
reclaims every VMM and adopts none: with no surviving workload, the expected
member set is empty. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 7.1 and 7.2.)

## Decision

In the fresh-process branch, the intercept owner (`MtlsInterceptWorker`)
converges all three dynamic sets to empty through the intercept port. Before it
may do so, the composition root must have established four preconditions:

- VMM reclamation;
- the shared-switch stale sweep;
- a zero managed-TAP read-back;
- EXEC BootClosed.

The owner requires the empty read-back before it observes the constant
program's identity, binds fresh listeners, replaces owned targets, reads
back, and opens admission. The resulting boot order is:

1. VMM reclamation.
2. Switch sweep and zero managed-TAP read-back.
3. Production shared-switch convergence.
4. Dynamic-member clear and empty read-back.
5. Constant-program identity, fresh bind, target replacement, the policy route
   and intercept-mark guard table, and full read-back.
6. DNS.
7. Supervisor.
8. Open admission.

Clear failure refuses startup with a distinct typed cause and leaves the gate
BootClosed. The same port operation, given the registry's expected members,
performs runtime member repair (feature delta, decision R15).

## Alternatives considered

### The composition root calls the netlink clear directly

Rejected. It bypasses the intercept port and its sim implementation, and splits
ownership of intercept state between the composition root and the worker.

### The shared-switch owner's stale sweep clears the IP sets

Rejected. IP-family intercept membership belongs to the intercept owner.
Clearing it from the switch couples two owners' state and duplicates the
constant-program identity check.

## Consequences

Positive: a node that lost its process recovers without operator action. Boot
has one ordered owner for intercept residue.

Negative: the intercept port gains a members-convergence operation. The boot
clear is legal only in a fresh process with no element tokens, and the worker's
fresh-process branch enforces that.
