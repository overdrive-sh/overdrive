# Evolution — guest-stack-transparent-mtls-intercept (GH #222)

**Finalization record:** 2026-09-01 · **Wave arc:** SPIKE → DISTILL → DESIGN →
DELIVER · **Delivery trace:** all 13 roadmap steps have complete DES
RED/GREEN/COMMIT traces and an APPROVED step review.

> **Final-gate status:** delivery and roadmap-review evidence are complete, but
> this is not a claim that the final DELIVER gate has passed. E07 is captured
> and still awaits the required independent evidence review; the existing final
> mutation report records no quality signal because its unmutated baseline
> failed before a mutant was evaluated. No mutation testing was run during this
> finalization pass.

## Feature summary

This feature makes the existing transparent-mTLS egress path usable by a real
Cloud Hypervisor guest. A VM Job is attached to a platform-owned TAP in its
workload netns, receives a routed guest `/30` before it emits READY, and opens
ordinary sockets. The host-veth nft-TPROXY rule captures a mesh dial and
routes it through the existing kernel-mediated mTLS enforcement path; the
workload never receives certificate material. The resulting vertical slice is
driven through `overdrive serve` and `overdrive deploy`, including the checked-
in VM-Job-to-Exec-Service E07 journey.

## Business context

Overdrive's mTLS model is sidecarless and identity-unaware at the workload
boundary: the platform, rather than application code, supplies identity and
transparently mediates TLS. MicroVM guests require a distinct guest-network
wire because the guest TCP socket is not a host `struct sock`; host cgroup
socket hooks cannot observe it. The completed work therefore supplies the
routed TAP topology and the production lifecycle ordering needed to capture
the guest's first eligible mesh flow without admitting a cleartext escape.

## Key decisions

- The spike promoted the proven routed two-`/30` topology: TAP and guest `/30`
  live in the workload netns, with a host return route and `ip_forward`; L2
  bridge and unnumbered alternatives were rejected as unproven.
- `workload_addr` for a VM is its guest address, carried by the existing
  allocation-status field. Guest setup is a platform-owned cmdline token
  processed silently before READY; setup failure is fail-closed.
- TAP convergence, routing, and guest-address injection remain in the C3
  provisioning seam. Cloud Hypervisor enters the netns through the existing
  wrapper and uses `--net tap=...`; no new daemon, crate, or external API was
  introduced for the guest wire.
- The VM install gate applies on both fresh start and same-allocation restart.
  The exact D7 rule uses a single anonymous counter and strict full-program,
  generation, multipart, notification, and capture checks rather than a
  handle-only or partial projection.
- The pre-READY lifecycle is deterministic: network setup completes before
  READY; READY precedes intercept installation; awaited EXEC release follows
  intercept success. A pre-READY VMM exit becomes a Failed Job result without
  a restart; post-READY exit 78 remains an ordinary Job result.
- Recovery uses the existing `ObservationStore` current state plus bounded
  lifecycle occurrences, resource-specific awaited cleanup, and the existing
  same-ID platform-reclamation route. The rejected alternatives include an
  outbox, parallel persistence boundary, generalized retry owner, and a
  survivor/quarantine protocol.
- The final BTR-3 amendment adds only the accepted internal allocation-
  lifecycle port: the concrete mTLS worker remains the production owner while
  the socket-free Sim adapter proves the seeded replacement ordering invariant.

## Work completed

| Step | Delivered outcome |
|---|---|
| 01-01 | Deterministic `VmTapPlan` derivation for TAP names, guest `/30` addresses, and guest MACs. |
| 01-02 | Idempotent TAP-in-netns provisioning, route ownership, and structural teardown. |
| 01-03 | Cloud Hypervisor TAP attachment and pre-EXEC guest addressing/bootstrap. |
| 02-01 | VM admission to the existing outbound mTLS install path and production metal walking skeleton. |
| 02-02 | Awaited Beacon EXEC release after intercept installation. |
| 02-03 | Native-metal qualification, pre-READY closure, and truthful pre-READY terminal classification. |
| 02-04 | Exact D7 rule/capture accounting and the black-box E07 product journey. |
| 02-05 | Fresh-install and pre-READY failure closure, diagnostic selection, and cleanup truthfulness. |
| 02-06 | Same-ID reclamation, failed reinstall, exact target teardown, and repeat-stop behavior. |
| 02-07 | Two-proposal bound for Stop/exit-observer LWW contention. |
| 02-08 | Post-assignment provision-failure teardown before slot release. |
| 02-09 | Prior-protection-first same-ID replacement teardown ordering. |
| 02-10 | Production lifecycle-port composition and the fixed-seed BTR-3 simulation invariant. |

## Verification and lasting artifacts

- `deliver/execution-log.json` passes `des-verify-integrity`: all 13 steps
  have complete DES traces. Step reviews `review-01-01.md` through
  `review-02-10.md` end in **APPROVED**. The final roadmap amendment review is
  also **APPROVED** and is reflected in the synchronized roadmap status.
- The feature's sole executed-evidence catalogue entry remains at its canonical
  repository-root location, [E07](../../verification/expectations/E07-vm-job-calls-exec-service/).
  Its captured native-metal evidence records the built product reaching the
  public Service `Running` and Job `Succeeded` results after the byte-exact
  reply. Its status remains `captured — independent review pending`; it is not
  represented as satisfied.
- No temporary-workspace file matched the architecture-design, ADR,
  walking-skeleton, or UX-journey migration destinations. The governing ADRs
  are already permanent: [ADR-0088](../product/architecture/adr-0088-guest-stack-routed-tap-netns-topology-and-addressing.md)
  and [ADR-0089](../product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md).

## Lessons learned

1. A microVM networking claim needs a real guest behind a real virtio-net TAP;
   a host netns is not a valid substitute because it has a host-visible TCP
   socket.
2. READY must mean guest platform initialization has completed. Deferring the
   network setup past READY turned terminal classification into a scheduling
   race and could release EXEC before the guest was safely initialized.
3. For a security-sensitive nft observation, a rule handle or selected packet
   sample is not enough. The proof needs one generation, a complete normalized
   program, loss detection, and exact counter/capture equality.
4. Recovery remediation stayed tractable only by proving the reachable owner
   path and reusing its existing persistence and cleanup boundaries. The
   unproven outbox, quarantine, and generalized shutdown proposals were
   rejected rather than implemented speculatively.
5. Delivery completion and release-quality completion are distinct evidence
   layers. Complete step traces and approved implementation reviews do not
   substitute for independent black-box evidence review or a successful final
   mutation-quality signal.

## Issues encountered

- The original pre-READY/EXEC ordering admitted an ambiguous guest EXIT race;
  the deterministic pre-READY network barrier closed it.
- Same-ID teardown and terminal contention required bounded, production-entry
  regressions to distinguish real lifecycle faults from theoretical async
  cancellation traces.
- The current final mutation report remains a release blocker: the unmutated
  baseline timed out before testing any mutant, so its `0/0` result is not a
  pass. This record preserves that status rather than reclassifying it.

## Finalization scope

Phases A and B created this evolution record and synchronized the stale roadmap
delivery/roadmap-review statuses. E07 remains in the canonical
`verification/expectations/` catalogue; no duplicate evolution archive is
kept. Phase C cleanup has not run: the feature workspace, all reviews,
execution log, and user-owned untracked review artifact remain in place pending
explicit cleanup approval. No commit or push was made by this finalization
pass.
