# SPIKE findings — BLOCKER-1: non-`/30` frontend-addr routing + egress capture

**Wave**: SPIKE (PROBE only — design-unblock for REV-2 thin path; no promotion/walking-skeleton)
**Date**: 2026-06-25
**Probe code**: `spike-scratch/increment-b/` (gitignored, throwaway — `probe.sh`, `legf.rs`, `dial.rs`)
**Kernel**: `uname -r` = `7.0.0-22-generic` (dev-Lima; re-confirm on the 6.18 appliance pin in the DELIVER Tier-3 matrix per ADR-0068)
**Run for real**: yes — real netns/veth/nft + real `connect()` under Lima as root. No `--no-run`/compile-only gate.

## Assumption under test (BLOCKER-1)

The shipped Path-A datapath (ADR-0071: destination-blind egress nft-TPROXY capturing ALL workload egress TCP, non-rewriting, `orig_dst` via `getsockname`/`IP_RECVORIGDSTADDR`; per-netns `/30` routing from `veth_provisioner`) will **route and capture** a client connection whose destination is a NEW stable per-`<job>` **frontend** IPv4 addr that is **not** a per-netns `/30` host (a distinct ClusterIP-style subnet no netns owns), recovering the frontend addr as `orig_dst`.

## VERDICT: **WORKS** — thin path holds

A non-`/30` frontend addr (`10.96.0.1`) routes out the workload netns, is captured by the destination-blind egress nft-TPROXY, and is recovered verbatim as `orig_dst` — using **only routes production ALREADY installs**.

- The load-bearing route is the per-netns **`default via <gateway>`** route, emitted in production by `WorkloadVethStep::AddDefaultRoute` (`veth_provisioner.rs` → `ip -n <netns> route add default via <gateway>`). A non-`/30` dest is not on-link, so it follows the **default** route to the host-side veth gateway, ingresses the host veth, and is matched by the destination-blind `iifname <host_veth>` egress TPROXY rule (`install_outbound_tproxy`, `mtls_intercept.rs`).
- **Production already installs the needed route** — no responder slice has to add anything for routability/capture.

## Evidence (predicted = actual on all four cases)

```
STEP 1  netns route table:   default via 10.99.0.1 dev ovd-wl-probe
                             10.99.0.0/30 dev ovd-wl-probe proto kernel scope link src 10.99.0.2

STEP 3  CONTROL (on-link /30, via LINK route):
        DIAL_OK     target=10.99.0.1:9999 local=10.99.0.2:50588
        LEGF_ACCEPT orig_dst=10.99.0.1:9999          <- harness proven good

STEP 4  FRONTEND (non-/30 10.96.0.1, via DEFAULT route) — THE QUESTION:
        DIAL_OK     target=10.96.0.1:9999 local=10.99.0.2:49574
        LEGF_ACCEPT orig_dst=10.96.0.1:9999          <- captured, frontend addr recovered verbatim

STEP 5  NEGATIVE-CONTROL (default route removed):
        DIAL_FAIL   target=10.96.0.1:9999 kind=NetworkUnreachable errno=Some(101)   <- ENETUNREACH
```

Population diff (debugging.md §5): the ONLY difference between FRONTEND-captured and FRONTEND-unreachable is the per-netns default route. Capture is destination-blind, so once the packet egresses the veth it is captured regardless of destination — consistent with the ADR-0071 capture-all-egress claim and Cilium's destination-blind socket-LB egress prior art (cross-check passed; verdict is not a surprise).

## `service_map_hydrator` mesh-gate note

The XDP-LB gate (`service_map_hydrator.rs:265–347`) keys on **backend addr ∈ `WORKLOAD_SUBNET_BASE` (10.99.0.0/16)** to decide which *backends* are programmed into the SERVICE_MAP/LOCAL_BACKEND_MAP paths — it concerns backend programming, not egress destinations. A frontend addr **outside** `WORKLOAD_SUBNET_BASE` does not interact with this gate on the egress-capture path. In the standalone probe (no XDP attached) the frontend addr simply hit egress tproxy and was not blackholed.

## Design implication for REV-1a (frontend subnet pin)

No blocker. The architect can pin the REV-1a frontend subnet, subject to two constraints the ClusterIP-style candidate already satisfies:

1. The frontend subnet **MUST NOT overlap `WORKLOAD_SUBNET_BASE = 10.99.0.0/16`** — overlap would make a frontend addr on-link to some per-alloc `/30` (changing the carrying route) AND catch it in the `service_map_hydrator` membership test. `10.96.0.0/16` is disjoint — fine.
2. The frontend subnet **MUST NOT be made on-link / owned by any other host route** — it must fall through to the per-netns default route.

Out of scope for this probe (responder/resolve work, not routing/capture): *advertising* the frontend addr via DNS and *resolving* the recovered frontend `orig_dst` → backend (`MtlsResolve` re-key, REV-1b).

## Gate recommendation

**THIN PATH HOLDS — pin the REV-1a frontend subnet (candidate `10.96.0.0/16`, disjoint from `10.99.0.0/16`).** Routability + capture + `orig_dst` recovery already work through the shipped Path-A datapath; REV-2 needs no routing/capture dataplane work.

## Discipline / cleanup

- Probe in gitignored `spike-scratch/increment-b/` (detached `rustc` build, no cargo/registry, no workspace drag). Prior DNS-responder probe in `increment-a` preserved untouched. No `crates/` pollution (`git status crates/` clean). No eBPF (pure `ip`/`nft`/routing + real `connect()` + transparent listener; leg-F listener copies the production `IP_TRANSPARENT` + `getsockname` shape from `mtls_intercept.rs`).
- Cleanup verified post-run: `ovd-ns-probe` netns absent, `ovd-hv/wl-probe` veth absent, `overdrive-mtls` nft table absent, `fwmark 0x1` ip rule absent, route table 100 empty, no leftover `legf` procs. No Lima state left behind.
