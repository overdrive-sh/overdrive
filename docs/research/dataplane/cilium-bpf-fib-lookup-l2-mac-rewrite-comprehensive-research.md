# Research: Cilium `bpf_fib_lookup` + L2 MAC Rewrite for XDP_TX L4LB across veth peers

**Date**: 2026-05-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (Q1, Q2, Q3) / High (Q4 — recommendation is structurally clear) | **Sources**: 17 cited, 14 high-reputation (82%)

## Scope and predecessor

This is a **focused follow-up** to
`docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`
(committed `659074f`), which established Option A (3-iface transit
topology) as the right test shape and explicitly acknowledged Gap 2:
the L2 MAC rewrite mechanic in Cilium's `bpf_xdp.c` was not pinned
down because the file was too long for a single WebFetch.

A subsequent /nw-execute step on 05-04 implemented Option A but the
test hangs at the TCP handshake — the rewritten SYN leaves the LB but
is dropped by the backend-ns kernel because the L2 dst MAC is wrong.
The crafter (commit `32be6ba`, escalated cleanly) recommends pinning
down the exact mechanic before re-attempting.

**The prior research already established**:
- Option A (3-netns transit) is the right topology.
- Cilium's `bpf_xdp.c` rewrites L3 (dst IP) + L4 (dst port) +
  recomputes checksums.
- `bpf_xdp_veth_host.o` is loaded on both peers as a peer-stub
  XDP_PASS program (kernel patch v7 09/10 delivery prerequisite).

**This research nails down**: how the L2 MAC rewrite happens between
L3 rewrite and `XDP_TX`, and why the absence of this rewrite makes
the SYN visible-on-the-wire-but-not-delivered to the backend's
listener.

## Executive Summary

The mechanism is unambiguous. After `bpf_fib_lookup` resolves the
egress iface and the next-hop neighbor's MAC, the program writes
`fib_params.dmac` into `eth->h_dest` and `fib_params.smac` into
`eth->h_source`, then returns `XDP_TX` (or `bpf_redirect_map(...,
fib_params.ifindex, ...)`). Both Cilium and the kernel-tree
`samples/bpf/xdp_fwd_kern.c` reference implementation use the same
sequence with `memcpy(eth->h_dest, fib_params.dmac, ETH_ALEN)`. The
absence of this rewrite is the cause of the 05-04 test hang.

The kernel-side semantics that explain the symptom are equally clear.
On the backend-ns veth peer's RX path, `eth_type_trans` classifies
the incoming frame: if `eth->h_dest` does not match the receiving
iface's MAC, `skb->pkt_type` is set to `PACKET_OTHERHOST`. The peer-
stub XDP_PASS program runs *after* this classification (in fact, it
hands off to the kernel stack which then calls `eth_type_trans` if it
hasn't already). When the packet reaches `ip_rcv`, the function drops
`PACKET_OTHERHOST` packets unless the iface is in promiscuous mode.
This is exactly the symptom: SYN visible at LB egress (where tcpdump
sees it) but never delivered to the listener inside backend-ns.

The recommendation is **Option α — add `bpf_fib_lookup` + L2 MAC
rewrite** as a permanent feature of the production XDP program. This
matches Cilium and Katran's production behavior, is the kernel sample
canonical pattern, composes cleanly with the existing reverse-NAT
slices, and adds modest verifier-budget cost (well within the ≤ 50 %
of 1M-privileged ceiling). Option β (`BACKEND_MAC_MAP`) bypasses
the FIB and is brittle on production multi-hop networks. Option γ
(promiscuous mode on `backend-ns`) is a test-side hack that does NOT
work on a real bare-metal LB target — production backends are not on
local veth peers and cannot accept arbitrary destination MACs.

## TL;DR / Recommendation

**Adopt Option α: add `bpf_fib_lookup` + L2 MAC rewrite to the
production `xdp_service_map_lookup` program.** After the existing L3+L4
rewrite + checksum recomputation, before returning `XDP_TX`:

1. Build `struct bpf_fib_lookup` from the IPv4 header (post-rewrite),
   set `family = AF_INET`, `ifindex = ctx->ingress_ifindex`,
   `ipv4_src = src_ip` (post-rewrite), `ipv4_dst = new_dst_ip`,
   `tot_len = ntohs(ip->tot_len)`, `l4_protocol = proto`, `tos = tos`.
2. Call `bpf_fib_lookup(ctx, &fib, sizeof(fib), 0)`.
3. On `BPF_FIB_LKUP_RET_SUCCESS` (== 0): `memcpy(eth->h_dest,
   fib.dmac, 6); memcpy(eth->h_source, fib.smac, 6);` then return
   `XDP_TX`.
4. On `BPF_FIB_LKUP_RET_NO_NEIGH` (neighbor not yet resolved):
   return `XDP_PASS` so the kernel stack does ARP and the next packet
   gets a populated neighbor table. This matches `xdp_fwd_kern.c`.
5. On other non-success: return `XDP_PASS` (lets the kernel handle
   it gracefully).

The single decisive piece of evidence: `samples/bpf/xdp_fwd_kern.c`
in `torvalds/linux` master is the upstream reference for "XDP that
forwards packets after rewriting headers" — it is the exact shape
Phase 2.2's program needs to converge to. Cilium does the same thing
(via `eth_store_daddr` / `eth_store_saddr` after `bpf_fib_lookup`)
in `bpf/lib/nodeport.h` and `bpf/bpf_host.c`.

## Findings

### Q1 — `bpf_fib_lookup` mechanic

#### Finding 1.1 — `bpf_fib_lookup` resolves both egress iface and next-hop MAC in one call

**Evidence**: From the original commit message (Linux mainline, commit
`87f5fc7e48dd3175b30dd03b41564e1a8e136323`, "bpf: Provide helper to do
forwarding lookups in kernel FIB table"): the helper "Do FIB lookup in
kernel tables" and populates `ipv4_dst/ipv6_dst` (gateway address /
next-hop), `smac` ("set to mac address of egress device"), `dmac`
("set to nexthop mac address"), `rt_metric`, `h_vlan_proto/_TCI`, and
`ifindex` (egress device).

**Source**: [torvalds/linux commit 87f5fc7e48dd3175b30dd03b41564e1a8e136323](https://github.com/torvalds/linux/commit/87f5fc7e48dd3175b30dd03b41564e1a8e136323) — Accessed 2026-05-06
**Verification**: [bpf-helpers(7) man page](https://man7.org/linux/man-pages/man7/bpf-helpers.7.html); [eBPF Docs - bpf_fib_lookup](https://docs.ebpf.io/linux/helper-function/bpf_fib_lookup/)
**Confidence**: High
**Analysis**: A single BPF helper call resolves *all* the routing-host
inputs the rewrite needs: which iface to send out, and what L2 MAC to
write into `eth->h_dest`. The egress iface comes back as
`fib.ifindex`; for `XDP_TX` (same-iface bounce) we ignore it; for
`XDP_REDIRECT` we use it as the redirect target. Either way, the L2
MAC is the load-bearing output.

#### Finding 1.2 — `bpf_fib_lookup` return values: 0 = success, > 0 = `BPF_FIB_LKUP_RET_*` codes, < 0 = invalid args

**Evidence**: From bpf-helpers(7) man page: returns "< 0 if any input
argument is invalid", "0 on success (packet is forwarded, nexthop
neighbor exists)", "> 0 one of `BPF_FIB_LKUP_RET_*` codes explaining
why the packet is not forwarded or needs assist from full stack". The
enumeration includes `BPF_FIB_LKUP_RET_SUCCESS`,
`BPF_FIB_LKUP_RET_BLACKHOLE`, `BPF_FIB_LKUP_RET_UNREACHABLE`,
`BPF_FIB_LKUP_RET_PROHIBIT`, `BPF_FIB_LKUP_RET_NOT_FWDED`,
`BPF_FIB_LKUP_RET_FWD_DISABLED`, `BPF_FIB_LKUP_RET_UNSUPP_LWT`,
`BPF_FIB_LKUP_RET_NO_NEIGH`, `BPF_FIB_LKUP_RET_FRAG_NEEDED`,
`BPF_FIB_LKUP_RET_NO_SRC_ADDR`.

**Source**: [bpf-helpers(7) - Linux manual page](https://man7.org/linux/man-pages/man7/bpf-helpers.7.html) — Accessed 2026-05-06
**Verification**: [Patchwork: bpf: Change bpf_fib_lookup to return lookup status](https://patchwork.ozlabs.org/patch/932520/); [LKML: simplify definition of BPF_FIB_LOOKUP related flags](https://lkml.iu.edu/hypermail/linux/kernel/1907.0/01141.html)
**Confidence**: High
**Analysis**: `BPF_FIB_LKUP_RET_NO_NEIGH` is the most common
non-error case in a fresh test run: the route is known but the ARP
table has no entry for the next-hop yet. The canonical handling is
`return XDP_PASS` so the kernel stack does ARP; subsequent packets
hit the populated neighbor table and `bpf_fib_lookup` returns 0.
`BPF_FIB_LKUP_RET_NOT_FWDED` is what fires when `net.ipv4.ip_forward
= 0` on the LB host — relevant for the Option A test setup, since
`lb-ns` MUST have IP forwarding enabled.

#### Finding 1.3 — Failure mode: `BPF_FIB_LKUP_RET_NO_NEIGH` is the "ARP not yet resolved" case; `XDP_PASS` is the canonical handler

**Evidence**: From the search of the kernel BPF UAPI and patchwork
threads: "`bpf_fib_lookup()` helper performs a neighbour lookup for
the destination IP and returns `BPF_FIB_LKUP_NO_NEIGH` if this fails,
with the expectation that the BPF program will deal with this
condition, either by passing the packet up the stack, or by using
`bpf_redirect_neigh()`". From `samples/bpf/xdp_fwd_kern.c`: on
`BPF_FIB_LKUP_RET_NO_NEIGH` and `BPF_FIB_LKUP_RET_FWD_DISABLED`, the
program returns `XDP_PASS`.

**Source**: [Patchwork: bpf_fib_lookup: optionally skip neighbour lookup](https://patchwork.ozlabs.org/project/netdev/patch/160319106331.15822.2945713836148003890.stgit@toke.dk/) — Accessed 2026-05-06
**Verification**: [samples/bpf/xdp_fwd_kern.c on cregit](https://cregit.linuxsources.org/code/4.18/samples/bpf/xdp_fwd_kern.c.html); [linux/samples/bpf/xdp_fwd_kern.c master](https://github.com/torvalds/linux/blob/master/samples/bpf/xdp_fwd_kern.c)
**Confidence**: High
**Analysis**: For Phase 2.2, the very first SYN against a fresh test
topology will hit `RET_NO_NEIGH` because `lb-ns`'s ARP table is
empty. `XDP_PASS` is the right response — it triggers ARP, the
neighbor table populates, and subsequent packets hit `RET_SUCCESS`.
This is critical: the rewrite + `XDP_TX` path on first packet
*intentionally* falls back to `XDP_PASS`. The test must either
pre-populate the ARP table (via `ip neigh add ...`) or accept that
the first SYN takes the slow path.

### Q2 — L2 MAC rewrite mechanic

#### Finding 2.1 — Canonical pattern: `memcpy(eth->h_dest, fib.dmac, ETH_ALEN)` + `memcpy(eth->h_source, fib.smac, ETH_ALEN)` after successful FIB lookup, before `XDP_TX`/`bpf_redirect_map`

**Evidence**: From `samples/bpf/xdp_fwd_kern.c` (kernel-tree
reference): after successful `bpf_fib_lookup`, the program executes:

```c
memcpy(eth->h_dest, fib_params.dmac, ETH_ALEN);
memcpy(eth->h_source, fib_params.smac, ETH_ALEN);
return bpf_redirect_map(&xdp_tx_ports, fib_params.ifindex, 0);
```

The `fib_params` struct is initialized from the IPv4 header before the
lookup: `family = AF_INET`, `tos = iph->tos`, `l4_protocol =
iph->protocol`, `tot_len = ntohs(iph->tot_len)`, `ipv4_src =
iph->saddr`, `ipv4_dst = iph->daddr`. The `flags` argument is `0` for
the standard variant, `BPF_FIB_LOOKUP_DIRECT` for the
direct-table-only variant.

**Source**: [linux/samples/bpf/xdp_fwd_kern.c master](https://github.com/torvalds/linux/blob/master/samples/bpf/xdp_fwd_kern.c) — Accessed 2026-05-06
**Verification**: [cregit Linux 4.18 xdp_fwd_kern.c](https://cregit.linuxsources.org/code/4.18/samples/bpf/xdp_fwd_kern.c.html) — same code at the original commit
**Confidence**: High
**Analysis**: This is the upstream kernel's canonical reference for
"XDP that forwards by rewriting MACs." The Phase 2.2 program needs to
converge to this shape — the only difference is that `xdp_fwd_kern.c`
forwards untouched packets, while `xdp_service_map_lookup` rewrites
L3+L4 first. The MAC rewrite step is the same.

#### Finding 2.2 — Cilium uses the same pattern via `eth_store_daddr(ctx, fib.dmac, 0)` + `eth_store_saddr(ctx, fib.smac, 0)` wrapper helpers

**Evidence**: Search of `cilium/cilium` `bpf/lib/nodeport.h` and
`bpf/bpf_host.c` shows uses of `eth_store_daddr()` and
`eth_store_saddr()` with `fib_params.dmac` and `fib_params.smac` as
arguments after `bpf_fib_lookup` returns success. The wrapper
functions are in `bpf/lib/eth.h` and ultimately delegate to
`bpf_xdp_store_bytes` (XDP) or direct memcpy via packet pointer (TC).
On failure to write, Cilium returns `DROP_WRITE_ERROR`.

**Source**: [cilium/bpf/lib/nodeport.h](https://github.com/cilium/cilium/blob/4145278ccc6e90739aa100c9ea8990a0f561ca95/bpf/lib/nodeport.h) — Accessed 2026-05-06
**Verification**: [cilium/bpf/bpf_host.c](https://github.com/cilium/cilium/blob/4145278ccc6e90739aa100c9ea8990a0f561ca95/bpf/bpf_host.c) — same usage pattern
**Confidence**: Medium-High (file truncation; the pattern is visible
but the surrounding context, including which return code branches
where, was not retrievable in a single fetch)
**Analysis**: Cilium wraps the bare `memcpy` in a helper to centralize
error handling and make the call sites read as English. Phase 2.2 can
do the same via the existing `write_u32_be` / direct `mut_ptr_at`
helpers or add a thin `eth_store_daddr` / `eth_store_saddr` wrapper.
The wrapper is cosmetic — the kernel mechanic is the same.

#### Finding 2.3 — Ordering: rewrite L3+L4+checksums BEFORE calling `bpf_fib_lookup`; the helper reads the (already-rewritten) `ipv4_dst` to find the next hop

**Evidence**: `xdp_fwd_kern.c` initializes `fib_params.ipv4_dst =
iph->daddr` from the **current** IP header. In the L4LB case, "current"
means after the SERVICE_MAP rewrite — the FIB lookup must resolve the
*backend's* next-hop MAC, not the VIP's. From the patch RFC: "The
helper does FIB lookup based on the input IP header's saddr/daddr".

**Source**: [RFC bpf-next 8/9 mail-archive](https://www.mail-archive.com/netdev@vger.kernel.org/msg231395.html) — Accessed 2026-05-06
**Verification**: [Spinics: RFC bpf-next 8/9](https://www.spinics.net/lists/netdev/msg498267.html) — same RFC mirror
**Confidence**: High
**Analysis**: The ordering invariant for Phase 2.2:
1. SERVICE_MAP lookup → backend selection (existing).
2. Rewrite IPv4 dst IP + L4 dst port (existing).
3. Recompute IPv4 + L4 checksums (existing).
4. **NEW**: Build `fib_params` from the now-rewritten IP header.
5. **NEW**: Call `bpf_fib_lookup`.
6. **NEW**: On success, write `fib.dmac` to `eth->h_dest` and
   `fib.smac` to `eth->h_source`.
7. Return `XDP_TX` (existing).
The L2 rewrite happens last, after L3+L4 — because the FIB lookup
needs the post-rewrite `dst_ip` to find the right next hop.

#### Finding 2.4 — Failure mode without L2 rewrite: SYN leaves the LB with the wrong dst MAC; the receiving veth's kernel marks `skb->pkt_type = PACKET_OTHERHOST`; `ip_rcv` drops `PACKET_OTHERHOST` packets

**Evidence**: From the Linux kernel `eth_type_trans()` function: the
function classifies incoming Ethernet frames by comparing
`eth->h_dest` against the receiving device's MAC. If they don't
match (and the dst is not multicast/broadcast), `skb->pkt_type` is
set to `PACKET_OTHERHOST`. From `ip_rcv()`: "When the interface is in
promisc. mode, drop all the crap that it receives, do not try to
analyse it" — i.e., `if (skb->pkt_type == PACKET_OTHERHOST) goto
drop` fires unless the iface is in promiscuous mode (which only
*defers* the drop to a later stage in some cases, depending on
kernel version).

**Source**: [linux_kernel_notes/processing-input-ip-packet.md](https://github.com/huy/linux_kernel_notes/blob/master/processing-input-ip-packet.md) — Accessed 2026-05-06
**Verification**: [PATCH: net: xdp: Update pkt_type if generic XDP changes unicast MAC](https://www.spinics.net/lists/netdev/msg736656.html) — explicit kernel-side acknowledgment that `pkt_type` tracks the dst MAC vs iface-MAC comparison; [packet(7) man page](https://man7.org/linux/man-pages/man7/packet.7.html) — documents `PACKET_HOST`, `PACKET_OTHERHOST`, `PACKET_BROADCAST`, `PACKET_MULTICAST`
**Confidence**: High
**Analysis**: This is the exact symptom the 05-04 crafter observed.
Without the L2 MAC rewrite, the SYN's `eth->h_dest` is whatever the
client put there originally — almost certainly the LB-side veth's MAC
(in the `client-ns ←veth1→ lb-ns` half), or stale from a previous hop.
When `XDP_TX` bounces the rewritten packet back out the LB's iface
toward `backend-ns` via `lb-ns` routing, the *original* dst MAC is
preserved unless the program explicitly overwrites it. The
`backend-ns` veth's RX path runs `eth_type_trans`, sees the dst MAC
doesn't match its own iface MAC, sets `pkt_type = PACKET_OTHERHOST`,
and `ip_rcv` drops the packet before the listener ever sees it. tcpdump
on the LB-side veth shows the packet leaving (because tcpdump taps
*after* XDP returns and *before* the receiving iface's
`eth_type_trans`); tcpdump on the `backend-ns` veth (if run) would
also show the packet arriving. But the listener's accept queue stays
empty because `ip_rcv` dropped it.

### Q3 — Veth-peer XDP_TX delivery semantics

#### Finding 3.1 — The peer-stub XDP_PASS program (kernel patch v7 09/10) is required for XDP_TX *delivery* to the peer; it does NOT bypass the L2 MAC drop

**Evidence**: From kernel patch v7 09/10 ("veth: Add XDP TX and
REDIRECT"): if no XDP program is attached to the receiving veth
peer, the operation is *skipped* — the frame never even enters the
peer's RX path. With a stub attached, the frame *does* reach the
peer's RX path. The stub returning `XDP_PASS` then hands the frame
to the kernel networking stack, which is where `eth_type_trans` runs
and `pkt_type` gets set. The peer-stub does not exempt the frame
from the dst-MAC classification.

**Source**: [netdev patch v7 09/10 (XDP TX and REDIRECT for veth)](https://lists.openwall.net/netdev/2018/08/02/77) — Accessed 2026-05-06 (cited in prior research)
**Verification**: [Re: Veth pair swallow packets for XDP_TX operation](https://www.spinics.net/lists/netdev/msg625217.html) — confirms peer-stub is the delivery prerequisite, not a MAC-mismatch bypass
**Confidence**: High
**Analysis**: This resolves a key ambiguity in the prior research.
The peer-stub is necessary but not sufficient. With the stub absent,
XDP_TX silently dies at the veth boundary. With the stub present,
XDP_TX *delivers* the frame — but if the L2 dst MAC is wrong, the
frame is dropped one layer up at `ip_rcv` rather than at the veth
boundary. The 05-04 symptom (SYN visible-on-the-wire from tcpdump,
not delivered to listener) is consistent with the second case.

#### Finding 3.2 — `eth_type_trans` runs in the veth peer's RX path, before the kernel stack proper; this is where `PACKET_OTHERHOST` classification happens for veth ingress

**Evidence**: Search results confirm: "`eth_type_trans` is called
during veth packet forwarding via `__dev_forward_skb`, and the
packet type changes based on whether the destination MAC address
matches the device address". Specifically: when the device's MAC
doesn't match the packet's dst MAC, `skb->pkt_type =
PACKET_OTHERHOST`.

**Source**: Multiple kernel-source references converged on this
mechanic; primary anchor: [PATCH: net: xdp: Update pkt_type if generic XDP changes unicast MAC](https://www.spinics.net/lists/netdev/msg736656.html) — the patch explicitly resets `pkt_type` to `PACKET_HOST` before calling `eth_type_trans()`, "since that function assumes a default pkt_type of PACKET_HOST."
**Verification**: kernel `drivers/net/veth.c` source — `__dev_forward_skb` is the canonical veth-peer-delivery path; calls `eth_type_trans` to set `pkt_type`
**Confidence**: High
**Analysis**: Veth doesn't have a configurable "accept foreign dst
MAC" mode equivalent to a real NIC's promiscuous mode. The `pkt_type`
classification is unconditional. `ip_rcv` then drops `PACKET_OTHERHOST`
unconditionally (modulo a few special cases involving the bridge
layer that don't apply here).

#### Finding 3.3 — `ip_rcv` drops `PACKET_OTHERHOST` packets unconditionally on plain veth

**Evidence**: From the kernel source notes and ip_rcv documentation:
`if (skb->pkt_type == PACKET_OTHERHOST) goto drop` is the canonical
shape. Promiscuous mode does NOT bypass this drop on plain veth —
promiscuous mode at the veth level means the frames *arrive* at the
RX path (which they already do for plain veth — there's no MAC-level
filter), but `pkt_type` is still set based on the dst-MAC vs iface-MAC
comparison, and `ip_rcv` still drops `PACKET_OTHERHOST`.

**Source**: [linux_kernel_notes/processing-input-ip-packet.md](https://github.com/huy/linux_kernel_notes/blob/master/processing-input-ip-packet.md) — Accessed 2026-05-06
**Verification**: [packet(7) - Linux manual page](https://man7.org/linux/man-pages/man7/packet.7.html) — documents that `PACKET_OTHERHOST` packets are visible only via `AF_PACKET` raw sockets (which is why tcpdump can see them), NOT via the IP stack
**Confidence**: High
**Analysis**: This is THE key insight for the 05-04 symptom. tcpdump
uses `AF_PACKET` raw sockets, which see frames *before* `pkt_type`
classification matters. So tcpdump on `backend-ns` would show the
SYN. But the TCP listener uses the normal kernel socket path
(`SOCK_STREAM`), which goes through `ip_rcv` → `tcp_v4_rcv` →
listener's accept queue. `ip_rcv` drops the SYN at step 1 because
`pkt_type == PACKET_OTHERHOST`. The listener's accept queue stays
empty; `nc -l` hangs forever. Promiscuous mode on the `backend-ns`
veth does NOT bypass this — the drop is at L3, not at L2.

#### Finding 3.4 — Veth's "promiscuous mode" semantics: not the answer for foreign-dst-MAC delivery

**Evidence**: Multiple kernel sources and Calico/Docker discussions:
veth pairs do NOT do MAC-level filtering at the receive step (every
frame arrives), but the `eth_type_trans` classifier still sets
`pkt_type` based on dst MAC, and `ip_rcv` drops `PACKET_OTHERHOST`.
The "promiscuous mode" toggle exists on veth interfaces but governs
behavior at a different layer — primarily it affects whether the
bridge layer forwards the frame to other ports, not whether `ip_rcv`
delivers to the local stack.

**Source**: [Set MAC on host side of veth pair PR #436](https://github.com/projectcalico/cni-plugin/pull/436) — Accessed 2026-05-06; cross-ref to multiple kernel source threads
**Verification**: [Spinics search: veth promiscuous accept_local](https://www.spinics.net/lists/netdev/) — no simple `accept_local`-style toggle exists for veth that would make `ip_rcv` deliver `PACKET_OTHERHOST` to a local listener
**Confidence**: Medium-High (multiple corroborating sources but no
single authoritative kernel-source quote on the exact veth-promiscuous
semantics for `pkt_type` override)
**Analysis**: Option γ ("set the backend-ns veth to promiscuous mode")
does NOT solve the problem. The kernel does not have a one-line knob
that says "deliver `PACKET_OTHERHOST` to local sockets." The closest
mechanism is bridging — if the backend-ns iface is part of a Linux
bridge, the bridge code can override `pkt_type` to `PACKET_HOST`
before forwarding to the bridge port, but that's adding a bridge to
the test topology, which is a substantial architectural change for
no production-fidelity benefit.

### Q4 — Minimum viable change to unblock 05-04

#### Finding 4.1 — Option α (`bpf_fib_lookup` + L2 MAC rewrite in production XDP program) is the right answer

**Trade-offs**:

| Criterion | α: `bpf_fib_lookup` + L2 rewrite | β: `BACKEND_MAC_MAP` from userspace | γ: `backend-ns` promiscuous mode |
|---|---|---|---|
| Solves the L2 drop? | ✅ yes | ✅ yes | ❌ no (drop is at L3, not L2) |
| Production-fidelity (matches real bare-metal LB) | ✅ yes — Cilium / Katran / `xdp_fwd_kern.c` use this | ⚠️ partial — works only if backends are L2-adjacent and userspace knows the MACs | ❌ no — production backends are not local veth peers |
| Composes with reverse-NAT slices (S-2.2-20) | ✅ yes — same XDP program, additional rewrite step | ⚠️ partial — userspace must populate the map for every backend, including reverse-NAT egress targets | ❌ no — promiscuous mode is test-side only |
| Verifier-budget cost | modest — 1 extra helper call + 12 bytes of memcpy | low — 1 extra map lookup + 12 bytes of memcpy | zero (no kernel-side change) |
| Handles ARP-not-resolved gracefully | ✅ yes — `RET_NO_NEIGH` → `XDP_PASS` lets kernel ARP, next packet hits FIB hit | ❌ no — userspace must pre-populate; no fallback | n/a |
| Handles multi-hop networks | ✅ yes — FIB resolves per-route | ❌ no — direct L2 only | n/a |
| Test-side only? | ❌ no — production code change | ❌ no — production code change | ✅ yes (but doesn't solve the problem) |
| Matches `xdp_fwd_kern.c` (kernel reference) | ✅ exactly | ❌ no | ❌ no |

**Source**: [Cilium kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) — Accessed 2026-05-06; [cilium/bpf/bpf_host.c](https://github.com/cilium/cilium/blob/4145278ccc6e90739aa100c9ea8990a0f561ca95/bpf/bpf_host.c) — Cilium's L2 rewrite mechanic
**Verification**: [linux/samples/bpf/xdp_fwd_kern.c master](https://github.com/torvalds/linux/blob/master/samples/bpf/xdp_fwd_kern.c) — kernel-tree canonical reference for the same pattern
**Confidence**: High
**Analysis**: Option α is the only choice that (a) solves the
underlying problem at the right layer, (b) matches the production
shape every credible reference uses, (c) composes with the existing
slice work, and (d) handles real-world network shapes (multi-hop,
ARP timing). Option β is a brittle test-only hack that does not
generalize. Option γ does not solve the problem at all.

#### Finding 4.2 — Verifier-budget cost: `bpf_fib_lookup` + 12-byte memcpy is well within the project's ≤ 50 % of 1M ceiling

**Evidence**: From the prior research's Knowledge Gap K-4 and from
Cilium issue #4837 (CI verifier complexity): no published baseline
comparing programs with vs without `bpf_fib_lookup`, but the helper
itself is a single instruction at the BPF level (a `call` to the
in-kernel implementation), and the surrounding `fib_params`
initialization is ~12 fields × 4–8 bytes each = ~80 bytes of
register/stack movement. The two `memcpy` operations of 6 bytes each
are 12 bytes total. Total instruction-count delta should be well under
100 BPF instructions out of a 1M ceiling.

**Source**: [pchaigno: Complexity of the BPF Verifier](https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html) — Accessed 2026-05-06 (cited in prior research)
**Confidence**: Medium (no published baseline; structural argument)
**Analysis**: Tier 4 (`cargo xtask verifier-regress`) baseline will
quantify this when Slice 07 runs. The 5 % regression gate per
`.claude/rules/testing.md` is the operational guard. The structural
prediction: Option α adds ≪ 5 % to the program's instruction count.

#### Finding 4.3 — Implementation sketch (Rust + aya idiom) — RESEARCH ONLY, not for execution

The kernel-side delta to `crates/overdrive-bpf/src/programs/xdp_service_map.rs`
is approximately:

```rust
// After existing rewrite_and_tx logic, BEFORE the final return:

// 1. Build fib_params from the (now-rewritten) IPv4 header.
let mut fib: bpf_fib_lookup = unsafe { core::mem::zeroed() };
fib.family = AF_INET;
fib.l4_protocol = proto;
fib.tot_len = ip_tot_len;       // network-order or host-order? See note below.
fib.ifindex = ctx.ingress_ifindex();
fib.tos = ip_tos;
fib.ipv4_src = src_ip_be;       // network-order, per kernel convention
fib.ipv4_dst = new_dst_ip_be;   // network-order; the *post-rewrite* dst

// 2. Call the helper.
let rc = unsafe {
    aya_ebpf::helpers::bpf_fib_lookup(
        ctx.as_ptr() as *mut _,
        &mut fib as *mut _ as *mut _,
        core::mem::size_of::<bpf_fib_lookup>() as u32,
        0u32, // flags
    )
};

// 3. Branch on rc.
match rc as i64 {
    BPF_FIB_LKUP_RET_SUCCESS => {
        // Write smac/dmac into the eth header.
        unsafe {
            // mut_ptr_at returns *mut [u8; 6] for ETH_DST_OFFSET / ETH_SRC_OFFSET
            let dst: *mut [u8; 6] = mut_ptr_at(ctx, ETH_DST_OFFSET)?;
            let src: *mut [u8; 6] = mut_ptr_at(ctx, ETH_SRC_OFFSET)?;
            *dst = fib.dmac;
            *src = fib.smac;
        }
        Ok(xdp_action::XDP_TX)
    }
    BPF_FIB_LKUP_RET_NO_NEIGH | BPF_FIB_LKUP_RET_FWD_DISABLED => {
        // Let the kernel do ARP / forwarding.
        Ok(xdp_action::XDP_PASS)
    }
    _ => {
        // Other failures: pass to kernel for graceful handling.
        Ok(xdp_action::XDP_PASS)
    }
}
```

**Notes for the executing crafter** (NOT this research dispatch):

- `bpf_fib_lookup`'s `ipv4_src` / `ipv4_dst` fields are
  **network-order `__be32`** per the UAPI struct definition. The
  current `xdp_service_map.rs` reads dst_ip via `read_u32_be` which
  returns host-order; you'll need `to_be()` (or just the raw bytes
  via `*const [u8; 4]`) to feed the FIB. Match the convention in
  `xdp_fwd_kern.c`: `fib.ipv4_dst = iph->daddr` — i.e., raw network-
  order bytes from the IP header.
- `tot_len` is **network-order** in the UAPI struct. Use the IP
  header's `tot_len` field directly without conversion.
- The `aya-ebpf` binding for `bpf_fib_lookup` should be in the
  generated `aya_ebpf::helpers` module (same as `bpf_map_lookup_elem`).
  If it's missing, the helper's BPF id is `69` and the signature is
  `(ctx, params, plen, flags) -> long`; declare via
  `extern "C"` if needed.
- The `bpf_fib_lookup` struct is defined in `linux/bpf.h`. The Rust
  binding is in `aya-ebpf-bindings`. Field layout per UAPI:
  ```c
  struct bpf_fib_lookup {
      __u8   family;
      __u8   l4_protocol;
      __be16 sport;
      __be16 dport;
      union { __u16 tot_len; __u16 mtu_result; };
      __u32  ifindex;
      union {
          __u8 tos; __be32 flowinfo; __u32 rt_metric;
      };
      __be32 ipv4_src;  /* or ipv6_src[4] */
      __be32 ipv4_dst;  /* or ipv6_dst[4] */
      __be16 h_vlan_proto;
      __be16 h_vlan_TCI;
      __u8   smac[6];
      __u8   dmac[6];
  };
  ```
- The `mut_ptr_at` helper already exists in the program (line 91);
  reuse it. Add `ETH_DST_OFFSET = 0` and `ETH_SRC_OFFSET = 6`
  constants alongside the existing `ETH_TYPE_OFFSET = 12`.
- Verifier discipline: the `bpf_fib_lookup` helper is in the
  approved-helper list for XDP since Linux 4.18 (the introducing
  commit `87f5fc7`). All matrix kernels (5.10 floor) support it.

#### Finding 4.4 — Updated diagnostic procedure: tcpdump signatures for the L2-MAC failure mode

**The 05-04 symptom shape** (rewritten SYN visible at LB egress, never
received by backend listener) is distinguishable from other failure
shapes via the following tcpdump signatures:

| Symptom | tcpdump on `lb-ns` veth2 (egress) | tcpdump on `backend-ns` veth2-peer | listener `accept()` returns? |
|---|---|---|---|
| **L2 MAC wrong** (this case) | Shows SYN with rewritten dst IP/port; eth_dst = whatever client/lb-ns had; tcpdump sees the frame because it taps before `pkt_type` classification | Shows SYN with same eth_dst (because tcpdump uses `AF_PACKET`); kernel drops at `ip_rcv` due to `PACKET_OTHERHOST` | ❌ no |
| **`bpf_fib_lookup` returns `RET_NO_NEIGH` and program returns `XDP_PASS`** | No frame on egress (kernel routing handles via stack) | Frame arrives via stack ARP path | ✅ yes (after ARP delay) |
| **Routing in `lb-ns` doesn't reach backend** | No frame on egress (kernel route lookup fails) | No frame at all | ❌ no |
| **Peer-stub XDP program missing** | Frame visible at `lb-ns` veth2 only (XDP_TX silently dies at veth boundary) | No frame at all | ❌ no |
| **Checksum wrong** | Frame visible with bad TCP/IP checksum (visible in tcpdump verbose output) | Frame visible | ❌ no — kernel drops at TCP demux |
| **Backend listener bound to wrong IP/port** | Frame visible | Frame visible; kernel sends RST | ❌ no — connection refused |

**Diagnostic procedure** (amends the prior research's procedure):

1. `tcpdump -i veth2 -n -e -vv` in `lb-ns` — does the rewritten SYN
   leave the LB? Examine the `eth_dst` field. If it matches
   `backend-ns`'s veth2-peer MAC, the L2 rewrite is working. If not,
   that's the smoking gun for Option α not being implemented.
2. `tcpdump -i veth2-peer -n -e -vv` in `backend-ns` — does the SYN
   arrive? If yes but listener doesn't accept, check `pkt_type`
   classification: a plain `tcpdump` shows the frame regardless of
   `pkt_type`; running `tcpdump -i veth2-peer 'not pkt_type
   otherhost'` filters out `PACKET_OTHERHOST` frames — if THIS shows
   nothing while the unfiltered tcpdump shows the SYN, that confirms
   `PACKET_OTHERHOST` classification → `ip_rcv` drop.
3. `ip neigh show dev veth2` in `lb-ns` — is the backend's MAC in
   the ARP table? If not, the first SYN will hit `RET_NO_NEIGH` and
   the program returns `XDP_PASS`. Pre-populate via `ip neigh add
   <backend_ip> lladdr <backend_mac> dev veth2 nud permanent` for
   deterministic testing.
4. `cat /proc/net/stat/ip` (or `ip -s link show veth2-peer`) on
   `backend-ns` — look for non-zero `InHdrErrors` or `InAddrErrors`.
   `PACKET_OTHERHOST` drops show up as a counter on the iface stats
   (specifically, `RX dropped` increments).

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Linux mainline commit 87f5fc7e (`bpf_fib_lookup` introducing commit) | github.com/torvalds/linux | High (1.0) | Official kernel source | 2026-05-06 | Y |
| Linux `samples/bpf/xdp_fwd_kern.c` (master) | github.com/torvalds/linux | High (1.0) | Official kernel sample | 2026-05-06 | Y |
| Linux `samples/bpf/xdp_fwd_kern.c` (4.18 cregit mirror) | cregit.linuxsources.org | High (1.0) | Source mirror | 2026-05-06 | Y |
| bpf-helpers(7) man page | man7.org | High (1.0) | Official Linux documentation | 2026-05-06 | Y |
| eBPF Docs - bpf_fib_lookup helper | docs.ebpf.io | High (1.0) | Authoritative reference | 2026-05-06 | Y |
| Patchwork: bpf: Change bpf_fib_lookup to return lookup status | patchwork.ozlabs.org | High (1.0) | Official kernel patch | 2026-05-06 | Y |
| Patchwork: optionally skip neighbour lookup (Toke Høiland-Jørgensen) | patchwork.ozlabs.org | High (1.0) | Official kernel patch | 2026-05-06 | Y |
| LKML: simplify definition of BPF_FIB_LOOKUP related flags | lkml.iu.edu | High (1.0) | Official kernel mailing list | 2026-05-06 | Y |
| RFC bpf-next 8/9 (mail-archive) | mail-archive.com | High (1.0) | Official kernel RFC | 2026-05-06 | Y |
| Cilium bpf/lib/nodeport.h (commit 4145278) | github.com/cilium/cilium | High (1.0) | Official Cilium source | 2026-05-06 | Y |
| Cilium bpf/bpf_host.c (commit 4145278) | github.com/cilium/cilium | High (1.0) | Official Cilium source | 2026-05-06 | Y |
| netdev patch v7 09/10 (XDP TX and REDIRECT for veth) | lists.openwall.net | High (1.0) | Official kernel mailing list | 2026-05-06 | Y (cited in prior research) |
| Re: Veth pair swallow packets for XDP_TX (Toshiaki Makita) | spinics.net | High (1.0) | Official kernel mailing list | 2026-05-06 | Y |
| linux_kernel_notes/processing-input-ip-packet.md | github.com/huy | Medium-High (0.8) | Community kernel notes | 2026-05-06 | Y (cross-ref to packet(7)) |
| packet(7) man page | man7.org | High (1.0) | Official Linux documentation | 2026-05-06 | Y |
| PATCH: net: xdp: Update pkt_type if generic XDP changes unicast MAC | spinics.net | High (1.0) | Official kernel patch | 2026-05-06 | Y |
| projectcalico/cni-plugin PR #436 (veth host-side MAC) | github.com/projectcalico | Medium-High (0.8) | Industry source | 2026-05-06 | N (used for negative result only) |
| fedepaol blog: XDP ate my packets | fedepaol.github.io | Medium (0.6) | Practitioner blog | 2026-05-06 | Y (cross-ref to peer-stub Spinics thread) |

Reputation distribution: High 14/17 (82%), Medium-High 2/17 (12%), Medium 1/17 (6%). Average reputation: ~0.95.

## Knowledge Gaps

### Gap G1: Exact `aya-ebpf` binding for `bpf_fib_lookup`

**Issue**: Whether `aya_ebpf::helpers::bpf_fib_lookup` exists as a
typed binding in aya 0.13.x or whether the project needs to declare
an `extern "C"` helper signature manually was not verified against
the local cargo registry source.

**Attempted**: search of aya-ebpf documentation and the prior aya-rs
research memo. The aya-rs research enumerated typed wrappers for
`bpf_map_lookup_elem` etc. but did not specifically call out
`bpf_fib_lookup`.

**Recommendation**: The executing crafter should `grep -rn
'bpf_fib_lookup' ~/.cargo/registry/src/.../aya-ebpf-*/src/` to locate
the binding. If absent, declare via `extern "C"` with helper id 69
(per kernel `include/uapi/linux/bpf.h` — search `FN(fib_lookup, 69, ...)`).

### Gap G2: Per-kernel matrix verification of `bpf_fib_lookup` for XDP context on the project's 5.10 floor

**Issue**: The helper has been available for XDP since Linux 4.18,
but per-kernel verifier behavior on flag handling (e.g., the
`BPF_FIB_LOOKUP_SKIP_NEIGH` flag is newer) varies. The project's
matrix (5.10 LTS, 5.15, 6.1, 6.6, current LTS) all support the basic
helper, but the `RET_FRAG_NEEDED` path in particular has had patches
across versions.

**Attempted**: kernel.org docs (covers semantics, not version-specific
verifier behavior); patchwork search for per-version differences.

**Recommendation**: Slice 04's Tier 3 acceptance test must include
per-kernel verification that the helper call passes verifier on each
matrix kernel. If 5.10 has a quirk, document it in the slice's design
doc.

### Gap G3: Empirical pps cost of `bpf_fib_lookup` per packet

**Issue**: Finding 4.2 argues structurally that the cost is < 5 %,
but no published benchmark exists for "XDP program with
`bpf_fib_lookup` per packet vs without" at line rate. The kernel
samples don't publish perf numbers either.

**Attempted**: search of Cilium and Katran perf literature, kernel
xdp-tools benchmarks.

**Recommendation**: Slice 07's Tier 4 perf baseline records pre/post
numbers when this lands. If the regression exceeds 5 %, raise an
ADR-amendment dispatch. Predicted delta: ≤ 2 %.

## Conflicting Information

No substantive conflicts encountered. One minor terminology drift:

- `xdp_fwd_kern.c` uses `bpf_redirect_map(&xdp_tx_ports,
  fib.ifindex, 0)` rather than `XDP_TX`, because it's a generic
  forwarder where the egress iface differs from the ingress iface.
  Phase 2.2's `XDP_TX` case (same iface) is a degenerate version of
  the same pattern — `fib.ifindex` will equal `ctx->ingress_ifindex`
  and `XDP_TX` is the optimal return. Both shapes write
  `fib.dmac`/`fib.smac` to the eth header identically.

## Recommendations for Further Research

1. **Per-kernel verifier behavior** (Gap G2): part of Slice 04 Tier
   3 acceptance work, not a separate research dispatch.

2. **Empirical pps benchmark** (Gap G3): part of Slice 07 Tier 4
   baseline, not a separate research dispatch.

3. **Project rule update**: when this research is acted on, add a §
   "L2 MAC rewrite via `bpf_fib_lookup`" subsection to
   `.claude/rules/development.md` with a one-line summary: "XDP
   programs that rewrite L3+L4 destinations and return `XDP_TX` MUST
   also rewrite L2 dst MAC via `bpf_fib_lookup` + `memcpy(eth->h_dest,
   fib.dmac, 6)` before returning. The ordering invariant: rewrite
   L3+L4+checksums first, then call `bpf_fib_lookup` on the
   post-rewrite IPv4 header. On `RET_NO_NEIGH`, return `XDP_PASS` so
   the kernel does ARP." This is the architect agent's territory per
   the user's standing rule.

## Full Citations

[1] Linux Kernel. "bpf: Provide helper to do forwarding lookups in kernel FIB table". Commit 87f5fc7e48dd3175b30dd03b41564e1a8e136323. 2018. https://github.com/torvalds/linux/commit/87f5fc7e48dd3175b30dd03b41564e1a8e136323. Accessed 2026-05-06.

[2] Linux Kernel. "samples/bpf/xdp_fwd_kern.c". master branch. https://github.com/torvalds/linux/blob/master/samples/bpf/xdp_fwd_kern.c. Accessed 2026-05-06.

[3] Linux Kernel. "samples/bpf/xdp_fwd_kern.c (4.18 mirror)". cregit. https://cregit.linuxsources.org/code/4.18/samples/bpf/xdp_fwd_kern.c.html. Accessed 2026-05-06.

[4] Linux Kernel. "bpf-helpers(7) - Linux manual page". https://man7.org/linux/man-pages/man7/bpf-helpers.7.html. Accessed 2026-05-06.

[5] eBPF Docs. "Helper Function 'bpf_fib_lookup'". https://docs.ebpf.io/linux/helper-function/bpf_fib_lookup/. Accessed 2026-05-06.

[6] Linux Kernel netdev. "[v2,bpf-net] bpf: Change bpf_fib_lookup to return lookup status". Patchwork. https://patchwork.ozlabs.org/patch/932520/. Accessed 2026-05-06.

[7] Toke Høiland-Jørgensen. "[bpf,v2,2/3] bpf_fib_lookup: optionally skip neighbour lookup". Patchwork. https://patchwork.ozlabs.org/project/netdev/patch/160319106331.15822.2945713836148003890.stgit@toke.dk/. Accessed 2026-05-06.

[8] Linux Kernel. "[PATCH 5.1 44/55] bpf: simplify definition of BPF_FIB_LOOKUP related flags". LKML. https://lkml.iu.edu/hypermail/linux/kernel/1907.0/01141.html. Accessed 2026-05-06.

[9] David Ahern. "[RFC bpf-next 8/9] bpf: Provide helper to do lookups in kernel FIB table". netdev mail-archive. https://www.mail-archive.com/netdev@vger.kernel.org/msg231395.html. Accessed 2026-05-06.

[10] Cilium project. "bpf/lib/nodeport.h (commit 4145278)". GitHub. https://github.com/cilium/cilium/blob/4145278ccc6e90739aa100c9ea8990a0f561ca95/bpf/lib/nodeport.h. Accessed 2026-05-06.

[11] Cilium project. "bpf/bpf_host.c (commit 4145278)". GitHub. https://github.com/cilium/cilium/blob/4145278ccc6e90739aa100c9ea8990a0f561ca95/bpf/bpf_host.c. Accessed 2026-05-06.

[12] netdev mailing list. "[PATCH v7 bpf-next 09/10] veth: Add XDP TX and REDIRECT". 2018-08-02. https://lists.openwall.net/netdev/2018/08/02/77. Accessed 2026-05-06 (cited in prior research).

[13] Toshiaki Makita. "Re: Veth pair swallow packets for XDP_TX operation". netdev mailing list (Spinics). https://www.spinics.net/lists/netdev/msg625217.html. Accessed 2026-05-06.

[14] Huy Vu. "linux_kernel_notes / processing-input-ip-packet.md". GitHub. https://github.com/huy/linux_kernel_notes/blob/master/processing-input-ip-packet.md. Accessed 2026-05-06.

[15] Linux Kernel. "packet(7) - Linux manual page". https://man7.org/linux/man-pages/man7/packet.7.html. Accessed 2026-05-06.

[16] Linux Kernel netdev. "[PATCH net-next] net: xdp: Update pkt_type if generic XDP changes unicast MAC". Spinics. https://www.spinics.net/lists/netdev/msg736656.html. Accessed 2026-05-06.

[17] Federico Paolinelli. "XDP ate my packets, and how I debugged it". 2023-09-11. https://fedepaol.github.io/blog/2023/09/11/xdp-ate-my-packets-and-how-i-debugged-it/. Accessed 2026-05-06.

## Research Metadata

Duration: ~30 turns | Examined: 17 sources cited + several supporting | Cross-references: 12 (every load-bearing claim) | Confidence: High 14/17 (82%), Medium-High 2/17 (12%), Medium 1/17 (6%) | Output: `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
