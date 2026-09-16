# SPIKE Decisions — netns-density-295

## Assumption Tested

- After Exec removal, two real Cloud Hypervisor microVMs on a node-local
  shared bridge can communicate through the actual production zero-copy mTLS
  enforcement core without depending on a per-workload network namespace,
  veth pair, `/30` carve, `NetSlot`, or `host_veth`.

## Probe Verdict

- **WORKS.** Part A proved the shared-bridge/TAP kernel mechanism. Part B
  reused the production `HostMtlsEnforcement`, workload-identity,
  mesh-resolution, and per-VM cgroup components on native metal and proved
  TLS 1.3 kTLS TX/RX plus zero-copy `splice(2)` with no direct shared-L2
  bypass. See `findings.md`.

## Promotion Decision

- **DISCARD from promotion; hand off to DESIGN.** Approved by the user on
  2026-09-16. The scratch implementation is not promoted directly into
  production. By explicit user instruction, the throwaway sources and raw
  evidence are retained and committed instead of deleted so DESIGN can audit
  the executed proof.

## Design Implications

- The microVM dataplane does not require the existing per-workload
  netns/veth/`/30`/`NetSlot`/`host_veth` mechanism.
- Preserve the topology-neutral production core: platform-held workload
  identities, mesh resolution, TLS 1.3, kTLS TX/RX, zero-copy splice, and the
  per-VM cgroup resource/lifecycle boundary.
- Replace the current netns-shaped VMM network attachment and
  `NetSlotAllocator`-backed DNS construction with a shared bridge/TAP design.
- Do not infer that per-allocation leg-F/leg-C listeners can be consolidated;
  the spike did not test that architectural choice.

## Constraints Discovered

- The proof is same-node. Cross-host routing, target density, throughput, and
  final production API shape remain DESIGN questions.
- The working interception path uses per-TAP bridge classification and local
  delivery before IP TPROXY; bare bridge `broute` was insufficient in the
  observed substrate.
- Per-VM cgroups remain necessary for host CPU weighting, reserve-padded
  memory limits, VMM ownership, OOM attribution, and total cleanup.
