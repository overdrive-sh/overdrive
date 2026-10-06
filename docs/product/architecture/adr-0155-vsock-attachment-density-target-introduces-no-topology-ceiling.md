# ADR-0155 — 16,384 VM attachments remains a measurement target; the vsock attachment adds no fixed per-node ceiling below it

## Status

**Proposed — decision D11 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

When accepted, this ADR **supersedes ADR-0117** (initial shared-bridge density
target). ADR-0117's premise that attachments attach reproducibly is falsified
for the bridge (E18). Its framing of 16,384 as a placeholder, not a guarantee,
and its rejection of 32,768 and 100,000 still hold and are carried forward.

## Context

The user ruled on 2026-09-24 that 16,384 is a fixed placeholder with no
capacity basis; ADR-0121's admission cap is the same placeholder. Real
capacity is #299 (heterogeneous resource accounting) and #261 (CPU and
memory). E18 showed the shared bridge imposes a kernel ceiling of about 1,023
ports.

Under ADR-0145 an attachment is one vhost-vsock device with its CID, its
kernel vhost worker task, one CH device thread, its control session, and its
intake listeners. The benchmark measured 16,384 Aya owners (16,466 kernel
tasks; 2.794 GiB unreclaimable slab for UDP owners) in a private kernel with
synthetic drivers. The CH fork adds one vhost worker task and one parked
device thread per VM. Kernel vhost-vsock admits any 32-bit CID except reserved
values and duplicates; the lease derives the CID (ADR-0156).

## Decision

1. **16,384 stays the measurement target and the admission placeholder**
   (ADR-0121 as amended). It is not a capacity guarantee on any hardware.
2. **The attachment topology must not impose a fixed limit below the target:**
   - no per-node port namespace, netdevice table, or per-VMM connection table
     bounds attachments;
   - forwarding map capacity is sized from the target and pinned in the
     feature delta;
   - flow-level exhaustion is a per-flow typed refusal, never an attachment
     ceiling.
3. **The CAP-295-A receipts are redefined for the vsock attachment** and
   re-captured through production `serve` + `deploy` on qualified metal (V-9).
   The bridge-era receipts (T1-BASE / T1-PORT4) do not carry over.

## Alternatives considered

- **Lower the target to what the bridge allowed.** Reintroduces a topology
  ceiling. Rejected.
- **Raise the target to 32,768 or 100,000.** Already rejected by ADR-0117 on
  its own grounds; nothing new supports it. Rejected.
- **Drop the target.** Leaves the topology unbounded by any measured
  population. Rejected.

## Consequences

- The attachment cost model is restated as CID, device, two tasks and
  sockets, with no netdevices.
- Concurrent flows are a separate, workload-driven dimension (ADR-0147) with
  their own pinned capacity, sized at four concurrent flows per attachment at
  the target. Exhausting it refuses individual flows, never attachments.
- Hardware CPU and memory limits remain real (#261). Below the target, a
  measured hardware limit is a finding, not a design ceiling.
