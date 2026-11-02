# ADR-0224 — Keep gateway backend selection and packet entry in the existing cgroup-BPF Service dataplane

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decisions ratified by the user 2026-09-13. Reconstruction decisions
D1–D5 approved by the user 2026-11-01 (D3 is the basis of this decision; see
[feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).
Remediation r1 (2026-11-02) merged the former packet-entry record into this
ADR; the demand-gated TEACH half of the original packet-entry decision is
retired by D1 and was never implemented.

## Context

The gateway resolves one exact Service Frontend but must not enumerate or
choose its backends, and must not become a second backend load balancer. The
existing ancestor-attached `cgroup_connect4_service` hook already owns
same-host destination rewrite for sockets in the node's cgroup tree, but on
main it reads only `LOCAL_BACKEND_MAP` and allows every miss
([ADR-0053](adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md)). A
host-originated connect to a Service VIP therefore is not selected. VM
backends are already programmed into `SERVICE_MAP`/`BACKEND_MAP` by the
ordinary `ServiceMapHydrator` path (D1; verified on native metal kernel 7.0,
[D4 findings](../../feature/public-ingress-gateway/spike/findings-vm-service-map-programming.md)),
so selection state exists; only a selecting reader is missing.

VM backends are reached from the host through the node-shared inbound path: a
marked, loopback-sourced dial is diverted by TPROXY to leg-C, which terminates
mTLS, arms kTLS and relays to the guest over an owner-opened vsock flow
([ADR-0145](adr-0145-vsock-kernel-forwarded-guest-transport-replaces-shared-bridge.md),
[ADR-0173](adr-0173-host-platform-clients-reach-a-vm-port-through-an-owner-opened-vsock-flow.md),
[ADR-0186](adr-0186-leg-s-reaches-vm-backends-through-a-temporary-vsock-relay-until-303.md),
[ADR-0190](adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md)).
Forcing the gateway through XDP instead would need a gateway-only network
namespace packet entry and reverse-path lifecycle.

## Decision

Backend selection for gateway upstream sockets happens in a `cgroup/connect4`
program, in the one BPF object `EbpfDataplane` already loads, attached to the
node's existing attach cgroup. For a socket the dataplane adapter has
registered as a gateway upstream socket, the program selects the backend from
`SERVICE_MAP`, its Maglev inner map and `BACKEND_MAP`, rewrites the
destination to it, and records the outcome (selected `BackendId` or
`NoBackend`) as the receipt
([ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md)).

- No outer `SERVICE_MAP` entry, or no usable backend in the chained lookup, is
  a typed `NoBackend` (public 503), never a fall-through to the unchanged VIP.
  A selected backend whose dial then fails is a connect failure (public 502).
  The gateway never reports a wrong-peer success.
- Whether the arm also treats `BACKEND_MAP.healthy == 0` as `NoBackend`, or
  ignores the byte, is **pending the user decision
  [P-9](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)**
  (options: honour `healthy` so the readiness window is 503; ignore it so that
  window is 502). This ADR holds under either option.
- A socket that is not registered keeps today's behaviour byte for byte. The
  gateway supplies the exact Service Frontend and never implements userspace
  backend selection, never reads `ServiceBackendRow`, enumerates candidates or
  mutates raw maps. XDP wire ingress is unchanged.
- Packet entry: the socket carries `SO_MARK` and a `127.0.0.1` source as
  ADR-0190 requires, so the rewritten dial enters the existing leg-C path
  ([ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)).
  No new network namespace, veth, route or nft rule is added.
- Whether the registered arm is a second program attached beside
  `cgroup_connect4_service` or an added arm of that program is **not decided
  here**: it is the user decision
  [P-10](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)
  (V-GW-4 measured 246 verified instructions for the arm against 28 for the
  existing program, so folding it in needs a Tier-4 re-baseline). Both options
  satisfy this decision.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Watch backend rows and round-robin in the gateway | It creates a second selector that can disagree with BPF and can authenticate a different backend from the one carrying the request. |
| Gateway-only netns/veth into XDP, or a gateway-specific cgroup/netns/interface solely to hook the socket | The `serve` process already lies inside the attach cgroup subtree. The cgroup a socket belongs to is fixed at socket allocation from the creating task's cgroup (`cgroup_sk_alloc`, `kernel/cgroup/cgroup.c:7408-7434` called from `net/core/sock.c:2330 @ v7.0`) and `connect4` programs run against that socket cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`), so a socket created by a `serve` thread (V-GW-2: all 19 `serve` threads in `/overdrive.slice/control-plane.slice`, under the attach path `/overdrive.slice`) is covered without added network lifecycle. |
| Split selection into a tail-called program | V-GW-4 shows the arm verifies (246 instructions, far below the 1M ceiling), so a tail call buys nothing; the budget question is the baseline policy, handled by P-10. |
| Run an external proxy/load balancer | It adds another deployment and secret boundary while duplicating the existing Dataplane owner. |

## Consequences

- The gateway cannot list backend candidates or mutate raw BPF maps.
  ServiceLifecycle remains the sole Backend Eligibility publisher and
  ServiceMapHydrator/Dataplane remain the only backend-application owners.
- Unregistered cgroup traffic and XDP wire forwarding keep their current
  behaviour byte for byte.
- The verifier-regression and budget gates apply to the registered arm
  (measured 246 verified instructions on 7.0.0-38-generic); placement and any
  baseline change follow P-10.
- The marked, loopback-bound dial is correct only while every Service backend
  address lies in the guest prefix (assumption A-3, inherited from ADR-0190).
- The extended-hook comment at `crates/overdrive-dataplane/src/mtls/mod.rs:928-931`
  states the connect hook attaches to the workload subtree and not the agent's;
  V-GW-2 observed the production hooks attached at `/sys/fs/cgroup/overdrive.slice`
  with `serve` inside it. The comment is stale or refers to a different hook; it
  is corrected when that file is next touched (flagged, not a gateway change).
- Selection receipt and applied-identity publication remain separate decisions
  in ADR-0232 and ADR-0233.

## Links

- [Backend programming and selection on main](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-backend-programming-and-selection-on-main-d1-d3)
- [Gateway upstream path into leg-C](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-upstream-path-into-leg-c-d3-d5-d)
- [Contract-shaping validation items](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-contract-shaping-validation-items)
- [Canonical C4 selection path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0053](adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md), [ADR-0040](adr-0040-service-map-three-map-split-and-hash-of-maps.md), [ADR-0042](adr-0042-service-map-hydrator-reconciler.md)
- [ADR-0228](adr-0228-dataplane-selection-receipt-ownership.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md), [ADR-0241](adr-0241-dataplane-applied-backend-identity-association.md), [ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)
