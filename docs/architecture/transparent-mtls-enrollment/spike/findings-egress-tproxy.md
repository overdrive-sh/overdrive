# SPIKE findings — Path A EGRESS nft-TPROXY (Probe B / increment-b)

Feature: **transparent-mtls-enrollment** (GH #236), Q1.
Probe: validate **Path A's egress interception** on the real kernel — the
active-side mirror of the already-PROVEN inbound nft-TPROXY half.

## Environment

- `uname -r` = **`7.0.0-22-generic`** (aarch64, Lima `overdrive` VM, run as root)
- `nft` = nftables **v1.1.6**, `ip` = iproute2; kernel TPROXY + `IP_TRANSPARENT`
  + `getsockname` all present (pinned appliance is 6.18 LTS — ADR-0068).
- Spike code: `spike-scratch/increment-b/` (gitignored, self-contained Rust +
  `ip`/`nft` CLI, **NO eBPF**, **zero `crates/` touch**). `increment-a/`
  preserved untouched.

## Binary verdict: **WORKS** — Path A egress validated

A workload in its own netns `connect()`s a real remote peer; the packet egresses
the workload netns, ingresses the host-side veth, where **nft-TPROXY in
PREROUTING fires**, redirects to the agent's **leg-F `IP_TRANSPARENT` listener**,
and **`getsockname()` recovers the original destination** — with **NO
pre-programmed per-destination map** and **NO orig-dst loss**. The F5 recursion
exemption holds. Symmetric with the proven inbound half (`install_inbound_tproxy`),
reusing the exact nft / `ip rule` / local-route / `IP_TRANSPARENT` recipe applied
to egress.

## H / P / F outcome

- **Hypothesis:** workload-netns egress, captured at host-side-veth ingress via
  nft-TPROXY, delivers to leg-F and `getsockname` returns the dialed `(ip,port)`.
  → **CONFIRMED.**
- **Predicted:** `getsockname()` == real-peer `(ip,port)` (NOT leg-F). → **MATCHED.**
- **Falsified if:** TPROXY doesn't fire for netns egress / `getsockname` returns
  leg-F / routing re-captures the agent's leg-B dial / egress needs OUTPUT-path
  TPROXY. → **None occurred.**

## Topology

```
┌─ netns nsW ─────────┐         ┌─ host netns ───────────────────────────┐
│  workload client    │         │  vethH 10.99.0.1/24                     │
│  vethW 10.99.0.2/24 │◀═veth══▶│   ↑ PREROUTING (priority mangle):       │
│  default via .1     │         │   │  nft-TPROXY                         │
│  connect(10.200.0.1:18777)    │   │   meta mark 0x2 accept   ← F5 head  │
└─────────────────────┘         │   │   ip daddr 10.200.0.1 dport 18777   │
                                 │   │    tproxy to 127.0.0.1:28777        │
                                 │   ▼    meta mark set 0x1                │
                                 │  ip rule fwmark 0x1 lookup 100          │
                                 │  ip route local 0.0.0.0/0 dev lo t100   │
                                 │  leg-F IP_TRANSPARENT 127.0.0.1:28777   │
                                 │  real-backend listener 10.200.0.1:18777 │
                                 └─────────────────────────────────────────┘
```

Real-backend `10.200.0.1:18777` lives on host `lo`; the workload routes to it via
the gateway, so its egress genuinely *ingresses* the host-side veth and hits
PREROUTING — the inbound topology mirrored to egress (not loopback-to-self).

## Evidence — main run (real `getsockname` output, verbatim)

Kernel state after install:

```
table ip overdrive-mtls-spikeb {
	chain prerouting {
		type filter hook prerouting priority mangle; policy accept;
		meta mark 0x00000002 accept
		ip daddr 10.200.0.1 tcp dport 18777 tproxy to 127.0.0.1:28777 meta mark set 0x00000001 accept
	}
}
0:	from all lookup local
32765:	from all fwmark 0x1 lookup 100
32766:	from all lookup main
32767:	from all lookup default
local default dev lo scope host    # table 100
```

Run result:

```
[step 5] client output: [exit=Some(0)] stdout=connected; echo="ECHO-FROM-LEGF" stderr=
[step 6] leg-F accepted a connection (peer=10.99.0.2:58998)
[step 6] getsockname(leg-F accepted) = 10.200.0.1:18777
[step 6] expected real-backend       = 10.200.0.1:18777
[step 6] ✔ RECOVERED orig-dst == dialed real-backend (NO map, NO orig-dst loss)
[step 7] ✔ NO LOOP: agent's marked leg-B dial reached the REAL-BACKEND listener
         directly (peer=10.200.0.1:34260); F5 exemption prevented re-capture into leg-F.
=== VERDICT: WORKS — Path A egress validated ===
```

Reproduced on a 2nd run (fresh ephemeral ports `33426` / `50010`), identical verdict.

- **Recovered vs expected:** `getsockname` = `10.200.0.1:18777` == dialed
  real-backend `10.200.0.1:18777`. **NOT** leg-F (`127.0.0.1:28777`). Accepted
  socket peer is `10.99.0.2:…` (the workload's veth address) — confirming the
  connection originated in the workload netns and was redirected, not a host
  shortcut.

## Did nft-TPROXY fire for netns egress at the veth ingress? — YES, proven by control

Per `debugging.md` §11 / §5 (compare populations). A **with-TPROXY vs
without-TPROXY** control isolates "fired" from "passed through":

- **Without any TPROXY rule** (control, identical netns+veth topology): the
  workload's `connect()` reached the **real backend directly** — `BACKEND-GOT-IT`,
  `exit=0`.
- **With the TPROXY rule** (main run): the same dial was instead accepted by
  **leg-F**, and `getsockname` recovered the orig-dst.

The redirect is genuinely nft-TPROXY at PREROUTING on the host-side veth.
**Egress does NOT need OUTPUT-path TPROXY** — PREROUTING on the ingress veth is
where workload-netns egress surfaces, which is why the inbound recipe mirrors
cleanly. (Matches Cilium: it catches "egress" because the workload is in a netns
behind a veth.)

## F5 / no-loop result — exemption load-bearing (both directions proven)

- **Positive (main run + dedicated control):** agent dials backend with
  `SO_MARK=0x2`; chain-head `meta mark 0x2 accept` short-circuits before the
  per-dest tproxy rule. `AGENT-DIAL reached: b'REAL-BACKEND-REPLY\n'` and
  `LEGF-CORRECTLY-IDLE (TimeoutError)` — dial reached the real backend, leg-F
  idle. **No loop.**
- **Negative control (exemption removed):** the same marked dial WAS captured by
  its own tproxy rule — `LEGF-ACCEPTED orig_dst=('10.200.0.7', 18700)`. **The
  loop hazard is real**; the exemption prevents it.

Sub-finding: the agent's dial originates in the **host netns**, yet WITHOUT the
exemption it is *still* captured by PREROUTING — the `ip route local … table 100`
re-injection puts host-originated packets to the backend daddr/dport through
PREROUTING too. So the F5 exemption is **genuinely required**: the agent's
leg-B/leg-S dial MUST carry the leg-dial mark, exactly as the production inbound
half already does.

## Edge cases / kernel surprises

- **Host-side routing hygiene is required, not a TPROXY concession.** The spike
  sets `ip_forward=1` and relaxes `rp_filter` on the ingress veth / `all` / `lo`.
  Without forwarding the host won't route to the lo-bound backend; without
  rp_filter relaxation the asymmetric ingress (in via veth, local-table reinject
  via lo) is dropped — a false "no fire." Standard L4-redirect host settings (the
  production veth-provisioner already owns analogous invariants, e.g. `tx off`).
- **nft renders the mark zero-padded** (`meta mark 0x00000002`) as the production
  dump-parse helpers already expect; egress install must reuse the same
  canonical-rendering dedup logic.
- **`priority mangle` PREROUTING is the correct hook** for egress capture (same as
  inbound). No separate egress hook / OUTPUT chain needed.
- No flakiness; teardown left zero residue (verified each run: no nft table, no
  `ip rule`, no netns, no lo address, no table-100 route).

## Design implications (for DESIGN)

1. **Path A egress holds as designed.** Routing shape needs NO adjustment:
   PREROUTING nft-TPROXY on the per-workload host-side veth → fwmark → `ip rule`
   → local route table → `IP_TRANSPARENT` leg-F → `getsockname`. Literal mirror
   of `install_inbound_tproxy`; egress + ingress unify on one proven mechanism.
2. **Load-bearing prerequisite = the topology change** (the wave-decisions
   headline): the workload MUST be in its own netns+veth. In host-netns single
   node v1 there is no veth ingress for egress to surface at. The `ExecDriver`
   `setns(CLONE_NEWNET)` seam is the enabler.
3. **F5 leg-dial mark is mandatory on the agent's orig-dst dial** — `SO_MARK`
   with the leg-dial mark so the chain-head exemption short-circuits it; identical
   to inbound `MTLS_LEG_S_DIAL_MARK`, non-optional (proven by the negative control).
4. **Per-workload-veth host routing settings** (`ip_forward`, `rp_filter` on the
   veth) join `tx off` as **platform-owned converge-on-boot invariants** on the
   veth-provisioner path (the #234 shared-routing-infra family). Surface in DESIGN.
5. **Single shared fwmark + single nft `prerouting` chain serve both directions
   and all destinations** — TPROXY preserves daddr, agent recovers orig-dst
   per-flow via `getsockname`; nothing per-destination in the routing layer.
   Egress install is one more per-dest rule on the same shared chain (could even
   share the inbound chain).
6. **No new BPF surface.** Confirms the wave-decisions choice over the
   research-doc cookie-stash *for the egress-capture step*: nft-TPROXY is
   sufficient and validated. (The research doc's cookie-stash is a different trade
   — avoids netns routing infra at the cost of new BPF. This probe shows the
   routing infra is tractable and already-proven-shaped, removing the "Path A
   egress is unvalidated" risk. DESIGN may still weigh the two.)

## Gate recommendation

**PROMOTE Path A egress to DESIGN → DELIVER.** The one unvalidated assumption from
wave-decisions.md ("Path A's egress nft-TPROXY on a per-workload veth +
`getsockname` recovery is UNVALIDATED on our exact topology") is now **validated
on the real kernel** with a live falsification path (with/without-TPROXY control)
and a proven F5 exemption (positive + negative controls). No routing-shape
adjustment required. Carry the four converge-on-boot host prerequisites
(netns+veth, `ip_forward`, `rp_filter`, leg-dial `SO_MARK`) into DESIGN as
explicit named prerequisites.
