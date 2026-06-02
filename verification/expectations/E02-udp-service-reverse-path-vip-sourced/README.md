# E02 — a deployed UDP service's reply is sourced from the VIP, not the backend IP

**Surface:** E (end-to-end) · **KPI:** K1 (UDP reverse-path success) · **Status:** `pending`

<!-- Status rationale (rewritten 2026-06-03 after RCA
docs/analysis/root-cause-analysis-convergence-dataplane-gap.md). The earlier
rationale ("serve can't boot — EbpfDataplane attaches XDP to lo and the attach
fails") is OBSOLETE: the single-node-dataplane-wiring fix (ADR-0061) makes serve
boot, provision the veth pair, attach the two XDP programs, converge a deployed
Service, and run the backend. That is no longer the blocker.

The ACCURATE blocker is topological, and it is structural, not transient:
E02's sub-claims 2+3 are the REMOTE-backend reverse-NAT path. On single-node
localhost, the RCA proved every backend resolves to `host_ipv4` and is therefore
classified LOCAL — it programs `LOCAL_BACKEND_MAP` and is steered via the
`cgroup_connect4` hook, NOT `SERVICE_MAP` / `REVERSE_NAT_MAP` (the remote
XDP-redirect path). `REVERSE_NAT_MAP` being empty on single-node localhost is
EXPECTED, not a defect — there is no reverse-NAT source-rewrite on the local
path because the client connects to the local backend directly. The VIP-sourced
reply (sub-claim 3) is a property of the remote path only. So E02 cannot be
captured black-box in the single-node dev VM: the surface it inspects does not
exist there. Black-box capture requires a multi-node / non-host-backend
environment (the shape `reverse_nat_udp_e2e.rs` builds with `ThreeIfaceTopology`).

Stays `pending`. The Tier-3 test
`crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs` — which
exercises the REMOTE path with a non-host backend IP and passes — is the
`what, forever` witness for sub-claims 2+3 (Stabilize doctrine, verification.md).
This expectation is the design-time `why` that test guards. -->


## Expectation

A single-UDP-listener service deployed end-to-end through a real control
plane and a real kernel completes a UDP round-trip whose **reply is sourced
from the VIP (`10.96.0.10:5353`), not the backend IP**. The reverse path
rewrites the connectionless UDP datagram's source the same way the TCP path
does — closing the #163 defect class where a UDP backend reply leaked its
own address.

This is the **full K1 proof** of the S-04-A walking skeleton: deploy →
`REVERSE_NAT_MAP` carries the `(backend_ip, 5353, udp)`→VIP key → the wire
capture shows the VIP source. It is the qualitative, human-readable `why`
for the regression alarm that already exists as the passing Tier-3 test
`crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs` — per
the Stabilize doctrine in `.claude/rules/verification.md`, that test is the
`what, forever`; this expectation is the design-time conversation it
guards.

**Topology scope (RCA finding).** The reverse-NAT VIP-source behavior is a
**remote-backend** property. Single-node localhost steers a deployed Service
through the *local* path (`LOCAL_BACKEND_MAP` + `cgroup_connect4`), which does
not source-rewrite to a VIP — so this expectation is not reproducible black-box
on a single host. It is the design-time `why` for the remote datapath, witnessed
forever by `reverse_nat_udp_e2e.rs`.

- Anchor: S-04-A (`docs/feature/udp-service-support/distill/test-scenarios.md` — reverse path carries the VIP source; `bpftool` dump + wire capture)
- Anchor: K1 — UDP reverse-path success (`docs/feature/udp-service-support/feature-delta.md` § Outcome KPIs, 0%→100%; test-scenarios.md K-mapping: "K1 → S-04-A (wire capture source==VIP + `bpftool` dump)")
- Anchor: roadmap 01-03 (`docs/feature/udp-service-support/deliver/roadmap.json` — "Tier 3 walking skeleton: single UDP listener forward+reverse e2e via real `overdrive deploy` subprocess")
- Anchor: ADR-0060 (`docs/product/architecture/adr-0060-service-frontend-update-service-signature.md` — both adapters derive REVERSE_NAT keys from the same typed `ServiceFrontend`; proto=udp flows to the kernel rewrite)
- Anchor: ADR-0061 (`docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md` — the boot fix that makes serve converge + run the backend; establishes that single-node uses the LOCAL path)
- Anchor: US-04 (`docs/feature/udp-service-support/feature-delta.md` § Walking skeleton)
- Evidence: RCA `docs/analysis/root-cause-analysis-convergence-dataplane-gap.md` (single-node uses `LOCAL_BACKEND_MAP`; remote-path maps empty by design; TCP and UDP both program the local map on a free port)

## Verification

Precondition: this is a Tier-3 **remote-path** surface. The `REVERSE_NAT_MAP`
dump and the VIP-sourced wire capture both require a **non-host backend** behind
the VIP — i.e. the real `overdrive-testing` `ThreeIfaceTopology` netns/veth rig
(client / lb / backend in separate namespaces), run as root inside Lima. A
single-node deploy in the dev VM steers through the LOCAL path
(`LOCAL_BACKEND_MAP` + `cgroup_connect4`) and never populates `REVERSE_NAT_MAP`,
so the black-box runner cannot capture sub-claims 2+3 here. The runner does NOT
rebuild `ThreeIfaceTopology` (that re-implements a test tier — forbidden by
verification.md — and risks leaked XDP/cgroups on the shared VM) and does NOT
link `reverse_nat_udp_e2e.rs` as evidence (it links crates — a rejected fifth
tier). It captures what it can CP-independently (a `bpftool map dump`) and leaves
the topology-bound sub-claims `pending`.

Sub-claims:

1. The deploy exits `0` and prints `Accepted.` (the precondition — proven
   black-box by **O03**, now `satisfied`; not re-proven here).
2. `bpftool map dump REVERSE_NAT_MAP` shows the `(backend_ip, 5353, udp)`
   key mapping to the VIP — the remote-path D (dataplane/kernel) sub-claim.
3. The wire capture on the client veth shows the backend's reply **sourced
   from the VIP (`10.96.0.10:5353`)**, never the backend IP (the #163 defect
   guard) — the remote-path E sub-claim.

`satisfied` requires sub-claims 2 and 3 on a Lima run with a **remote-backend
topology** up, SHA + seed pinned in `evidence/verification.yaml`. That topology
does not exist in the single-node dev VM, so the realistic paths to `satisfied`
are (a) a future multi-node / remote-backend capture environment, or (b)
accepting `reverse_nat_udp_e2e.rs` as the standing witness per the Stabilize
doctrine. Until then E02 is the design-time `why`, `pending` for black-box
capture.

## Evidence

The controlling evidence is the RCA
`docs/analysis/root-cause-analysis-convergence-dataplane-gap.md` (probes at SHA
`e9cec107`), which established the topology scope empirically:

- A single-node UDP Service **converges and runs its backend** (post-ADR-0061):
  `alloc status` → `Allocations: 1`, `socat UDP4-LISTEN` in
  `alloc-dns-resolver-0.scope`.
- `REVERSE_NAT_MAP` and `SERVICE_MAP` dump **`Found 0 elements`** — EXPECTED:
  the local backend programs `LOCAL_BACKEND_MAP` (via `cgroup_connect4`), not
  the remote XDP-redirect maps. (A control showed a backend on a free port
  programs `LOCAL_BACKEND_MAP` for both UDP and TCP.)
- So the reverse-NAT VIP-source path E02 inspects is a remote-backend property
  not exercised single-node; the empty `REVERSE_NAT_MAP` is not a defect.

The earlier `evidence/` files (`reverse_nat_map_dump_preflight`,
`preflight_cluster`, SHA `ea3c86f6`) reflect a superseded capture whose stated
cause (serve can't boot) is now obsolete; the RCA is authoritative.

Per-sub-claim verdict:

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | deploy exits `0` + `Accepted.` (precondition) | `satisfied` (via O03) | proven black-box by O03; serve boots + accepts post-ADR-0061. |
| 2 | `REVERSE_NAT_MAP` shows `(backend_ip,5353,udp)`→VIP | `pending` (remote-path) | single-node steers via `LOCAL_BACKEND_MAP`/`cgroup_connect4`; `REVERSE_NAT_MAP` is empty by design without a non-host backend. Needs a multi-node topology. |
| 3 | wire reply sourced from VIP `10.96.0.10:5353`, not backend IP | `pending` (remote-path) | the VIP source-rewrite is a remote-path property; not exercised single-node. The wire capture needs `ThreeIfaceTopology` (a test tier the runner won't rebuild). |

The `what, forever` witness for sub-claims 2+3 (and the #163 VIP-source guard)
is `crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs`, which
runs the REMOTE path with a non-host backend IP and passes. This expectation is
the design-time `why` that test guards (Stabilize doctrine,
`.claude/rules/verification.md`).

To capture black-box: stand up a remote-backend topology (separate backend
netns / non-host backend IP) with serve + dataplane in a Lima-routed
environment, then re-run `harness/run-expectation.sh E02`. The single-node dev
VM cannot host it.
