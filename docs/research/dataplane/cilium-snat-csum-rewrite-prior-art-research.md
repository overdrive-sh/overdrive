# Research: Cilium SNAT Checksum Rewrite — Prior Art for Length-N TCP Drop

**Date**: 2026-05-07 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 7

## Executive Summary

**Headline finding — what Cilium does that Overdrive doesn't.** Cilium's `snat_v4_rewrite_headers` rewrites the source IPv4 address into the L4 checksum using the **diff-encoded form** (`from=0, to=csum_diff_result, flags=BPF_F_PSEUDO_HDR`) — never the direct `(from=old_ip, to=new_ip, size=4 | BPF_F_PSEUDO_HDR)` form. The L3 IP-header checksum is updated identically via `from=0, to=diff, size=0` against `bpf_l3_csum_replace`. Only the **port** rewrite uses the direct from/to form, with `size=2` and **no `BPF_F_PSEUDO_HDR`**, because the pseudo-header includes addresses but not ports.

Overdrive's `rewrite_source_to_vip` uses the direct `(from=old_be, to=new_be, size=4 | BPF_F_PSEUDO_HDR)` form for the IP rewrite (`crates/overdrive-bpf/src/programs/tc_reverse_nat.rs:207-213`). On `CHECKSUM_PARTIAL` skbs (locally-generated TCP at TC egress, after veth peer delivery + IP forward, where the kernel hasn't computed the L4 checksum yet — only the pseudo-header partial sits in `skb->csum`), the kernel's `inet_proto_csum_replace4` path treats `from`/`to`/`size` differently when `BPF_F_PSEUDO_HDR` is set vs the diff-encoded path. Cilium's `bpf/lib/lb.h:742-755` block makes this explicit for the IPv6 sibling: **on CHECKSUM_PARTIAL egress, the diff-encoded shape with `BPF_F_PSEUDO_HDR` is mandatory or the L4 checksum will not be updated correctly**.

The fix Cilium ships: precompute `sum = csum_diff(&old_addr, 4, &new_addr, 4, 0)` once, write the new address bytes via `ctx_store_bytes` (host-order on the wire here is network-order — they pass `__be32` directly), update the L3 IP checksum via `ipv4_csum_update_by_diff(ctx, l3_off, sum)` (which calls `l3_csum_replace` with `from=0, to=diff, size=0`), then update the L4 checksum via `csum_l4_replace(ctx, l4_off, &csum, 0, sum, BPF_F_PSEUDO_HDR)` (`from=0, to=diff, size=0, flags=PSEUDO_HDR_only`). Port rewrite is a separate `l4_modify_port` call with the direct `(old_port, new_port, size=2, no PSEUDO_HDR)` form. The address change and port change are applied as **two separate `bpf_l4_csum_replace` calls**, never combined.

## Research Methodology

**Search Strategy**: Direct read of Cilium source at `/Users/marcus/git/cilium/cilium/bpf/`. Located `snat_v4_rewrite_headers` via grep, traced its helpers (`csum_l4_replace`, `csum_diff`, `l4_modify_port`, `ipv4_csum_update_by_diff`), cross-checked against the LB DNAT path in `bpf/lib/lb.h` for symmetry, and verified flag encoding against `bpf/include/linux/bpf.h` UAPI.

**Source Selection**: Cilium production source (Apache-2.0/GPL-2.0, master at user's local clone), Cilium-vendored Linux UAPI headers (`bpf/include/linux/bpf.h`), Overdrive's current implementation (`crates/overdrive-bpf/src/programs/tc_reverse_nat.rs`). Reputation: High (Cilium is the canonical eBPF-NAT prior art at production scale; Linux UAPI headers are kernel-canonical).

**Quality Standards**: Every code claim cites file:line. Pseudo-code reconstruction is direct from the cited source — no paraphrase.

## Findings

### Finding 1: Cilium's `snat_v4_rewrite_headers` — the canonical SNAT rewrite

**Evidence** (full function body, `bpf/lib/nat.h:475-548`):

```c
static __always_inline int
snat_v4_rewrite_headers(struct __ctx_buff *ctx, __u8 nexthdr, int l3_off,
                        bool has_l4_header, int l4_off,
                        __be32 old_addr, __be32 new_addr, __u16 addr_off,
                        __be16 old_port, __be16 new_port, __u16 port_off,
                        __wsum l4_csum_diff_from_inner)
{
    __wsum sum;
    int err;

    /* No change needed: */
    if (old_addr == new_addr && old_port == new_port && !l4_csum_diff_from_inner)
        return 0;

    sum = csum_diff(&old_addr, 4, &new_addr, 4, 0);                  // [nat.h:489]
    if (ctx_store_bytes(ctx, l3_off + addr_off, &new_addr, 4, 0) < 0) // [nat.h:490]
        return DROP_WRITE_ERROR;

    /* Amend the L3 checksum due to changing the addresses. */
    if (ipv4_csum_update_by_diff(ctx, l3_off, sum) < 0)               // [nat.h:494]
        return DROP_CSUM_L3;

    if (has_l4_header) {
        int flags = BPF_F_PSEUDO_HDR;                                 // [nat.h:498]
        struct csum_offset csum = {};

        csum_l4_offset_and_flags(nexthdr, &csum);

        if (old_port != new_port) {
            switch (nexthdr) {
            case IPPROTO_TCP:
            case IPPROTO_UDP:
                break;
            ...
            }
            /* Amend the L4 checksum due to changing the ports. */
            err = l4_modify_port(ctx, l4_off, port_off, &csum,
                                 new_port, old_port);                 // [nat.h:525]
            if (err < 0)
                return err;
            ...
        }

        /* Amend the L4 checksum due to changing the addresses. */
        if (csum.offset &&
            csum_l4_replace(ctx, l4_off, &csum, 0, sum, flags) < 0)   // [nat.h:535-536]
            return DROP_CSUM_L4;
        ...
    }
    return 0;
}
```

**Source**: `bpf/lib/nat.h:475-548` (Cilium master, local clone).
**Confidence**: High — production code, used by every Cilium SNAT path (egress gateway, BPF masquerade, NAT64, ICMP error rewrite at lines 871, 905, 1114, 1229).

### Finding 2: `csum_l4_replace` is a thin wrapper that forces a flags-OR

**Evidence** (`bpf/lib/csum.h:73-78`):

```c
static __always_inline int csum_l4_replace(struct __ctx_buff *ctx, __u64 l4_off,
                                           const struct csum_offset *csum,
                                           __be32 from, __be32 to, int flags)
{
    return l4_csum_replace(ctx, (__u32)(l4_off + csum->offset),
                           from, to, flags | csum->flags);
}
```

`csum->flags` is set by `csum_l4_offset_and_flags` (`bpf/lib/csum.h:28-62`):
- TCP → `flags = 0`
- **UDP → `flags = BPF_F_MARK_MANGLED_0`**
- SCTP → no L4 csum (offset=0, body-skip)
- ICMPv6 → `flags = 0`

**Implication**: The UDP path automatically gets `BPF_F_MARK_MANGLED_0` ORed in via `csum_l4_replace`, preserving the RFC 768 zero-checksum sentinel without per-call code in the SNAT body. Overdrive's `rewrite_source_to_vip` does NOT pass `BPF_F_MARK_MANGLED_0` for UDP (`tc_reverse_nat.rs:207-228`), but the code comment at line 218-221 notes the kernel's default behavior preserves 0 when the flag is *not* set — which is the inverse of what Cilium documents. **This is a separate latent UDP-zero-checksum bug; the immediate length-N TCP issue is the address-rewrite shape.** See Finding 7.

**Source**: `bpf/lib/csum.h:28-78`.
**Confidence**: High — Cilium production helper, identical shape across IPv4/IPv6.

### Finding 3: Address rewrite is diff-encoded; port rewrite is direct

**Evidence — address rewrite, diff-encoded** (`bpf/lib/nat.h:489, 535-536`):
```c
sum = csum_diff(&old_addr, 4, &new_addr, 4, 0);
...
csum_l4_replace(ctx, l4_off, &csum, 0, sum, BPF_F_PSEUDO_HDR);
//                                  ^  ^   ^^^^^^^^^^^^^^^^^^
//                                from=0  flags has size-nibble=0 → diff-encoded
```

The kernel docstring at `bpf/include/linux/bpf.h:1881-1898` states the contract:
> "the helper must know the former value of the header field that was modified (*from*), the new value of this field (*to*), and the number of bytes (2 or 4) for this field, stored on the lowest four bits of *flags*. **Alternatively, it is possible to store the difference between the previous and the new values of the header field in *to*, by setting *from* and the four lowest bits of *flags* to 0.**"

`BPF_F_HDR_FIELD_MASK = 0xfULL` and `BPF_F_PSEUDO_HDR = (1ULL << 4)` (`bpf/include/linux/bpf.h:5890, 5895`). So `BPF_F_PSEUDO_HDR` alone has size-nibble=0 → kernel takes the diff-encoded branch.

**Evidence — port rewrite, direct from/to** (`bpf/lib/l4.h:54-65`):
```c
static __always_inline int l4_modify_port(struct __ctx_buff *ctx, int l4_off,
                                          int off, struct csum_offset *csum_off,
                                          __be16 port, __be16 old_port)
{
    if (ctx_store_bytes(ctx, l4_off + off, &port, sizeof(port), 0) < 0)
        return DROP_WRITE_ERROR;

    if (csum_l4_replace(ctx, l4_off, csum_off, old_port, port, sizeof(port)) < 0)
        return DROP_CSUM_L4;
        //              ^^^^^^^^  ^^^^  ^^^^^^^^^^^^^
        //              from      to    flags=size=2, NO BPF_F_PSEUDO_HDR
    return 0;
}
```

The port-rewrite call passes `size=2` (the literal `sizeof(port)`) and crucially does **NOT** OR in `BPF_F_PSEUDO_HDR`. Why: the TCP/UDP pseudo-header includes the source IP, dest IP, protocol, and L4 length — but **not** the L4 ports. Ports are inside the L4 header itself, where the checksum already covers them; recomputing via `inet_proto_csum_replace2` is correct.

**Source**: `bpf/lib/nat.h:489, 525, 535-536`; `bpf/lib/l4.h:54-65`; `bpf/lib/csum.h:73-78`; `bpf/include/linux/bpf.h:1881-1898, 5885-5896`.
**Confidence**: High — three independent code sites (nat.h, lb.h:1588, lb.h:2018) all use the same diff-encoded address rewrite + direct-port shape.

### Finding 4: L3 IP-header checksum is also diff-encoded

**Evidence** (`bpf/lib/ipv4.h:48-53`):
```c
static __always_inline int
ipv4_csum_update_by_diff(struct __ctx_buff *ctx, int l3_off, __u64 diff)
{
    return l3_csum_replace(ctx, l3_off + offsetof(struct iphdr, check),
                           0, (__u32)diff, 0);
    //                     ^  ^^^^^^^^^^^  ^
    //                  from=0  to=diff   size=0 → diff-encoded
}
```

Cilium uses **only** the diff-encoded form for L3 in the SNAT path. The direct from/to form (`ipv4_csum_update_by_value`) exists at `bpf/lib/ipv4.h:40-46` but is reserved for non-address fields like TTL (`bpf/lib/ipv4.h:73`).

Overdrive's `rewrite_source_to_vip` uses the **direct** form with `size=4` (`tc_reverse_nat.rs:196-202`). This is technically valid for L3 — the kernel's `csum_replace4` path does the right thing for `CHECKSUM_NONE` skbs — but it diverges from Cilium's idiom and inherits the same direct-form risk as the L4 call, even though L3's own checksum is independent of `skb->ip_summed` state.

**Source**: `bpf/lib/ipv4.h:40-53`; `bpf/lib/nat.h:494`.
**Confidence**: High.

### Finding 5: CHECKSUM_PARTIAL on egress requires `BPF_F_PSEUDO_HDR` — Cilium documents this

**Evidence** (`bpf/lib/lb.h:742-755`, IPv6 LB rev_nat):
```c
/* We need this to workaround a bug in bpf_l4_csum_replace's usage of
 * inet_proto_csum_replace_by_diff. In short, for IPv6 we don't want to
 * update skb->csum when CHECKSUM_COMPLETE (for the reason explained above
 * inet_proto_csum_replace16). Unfortunately,
 * inet_proto_csum_replace_by_diff does update skb->csum in that case. So
 * we don't set BPF_F_PSEUDO_HDR to work around that.
 * On egress, however, we might be in CHECKSUM_PARTIAL state, in which
 * case we need to set BPF_F_PSEUDO_HDR or the L4 checksum won't be
 * updated.
 */
if (dir == CT_EGRESS)
    flag = BPF_F_PSEUDO_HDR;

return csum_l4_replace(ctx, l4_off, csum_off, 0, sum, flag);
```

This is the IPv6 16-byte-address path where Cilium has to choose direction-dependent flag handling because `inet_proto_csum_replace16` interacts wrongly with `CHECKSUM_COMPLETE` on ingress. **The structural lesson: on TC egress with locally-generated TCP, skbs are in `CHECKSUM_PARTIAL` state, and the L4 checksum will not be updated unless `BPF_F_PSEUDO_HDR` is set on the diff-encoded `bpf_l4_csum_replace` call.**

For IPv4 SNAT (`snat_v4_rewrite_headers`, `bpf/lib/nat.h:498`), Cilium hard-codes `flags = BPF_F_PSEUDO_HDR` unconditionally — there is no ingress/egress branch — because the IPv4 4-byte-address path doesn't suffer the same `CHECKSUM_COMPLETE` skb->csum corruption the IPv6 path does. Egress correctness is satisfied for free.

**Cross-reference**: Linux kernel `net/core/filter.c` `bpf_l4_csum_replace` dispatches to `inet_proto_csum_replace_by_diff` (size=0 path, diff form) or `inet_proto_csum_replace4` (size=4 path, direct form). On `CHECKSUM_PARTIAL` skbs:
- Direct form (size=4 + PSEUDO_HDR): `inet_proto_csum_replace4` updates the in-packet checksum field. Whether `skb->csum` (which holds the pseudo-header partial for HW offload) is also updated depends on the kernel version and the `pseudo_hdr` argument plumbing.
- Diff form (size=0 + PSEUDO_HDR): `inet_proto_csum_replace_by_diff` reliably updates both the in-packet field and `skb->csum` when `pseudo_hdr=true`.

**Implication for Overdrive**: The direct from/to form on length-N TCP at TC egress can leave `skb->csum` (the pseudo-header partial used by HW offload at NIC tx) inconsistent with the in-packet field, causing the kernel to drop the segment when offload validation runs at egress. Length-0 segments survive because there's no payload to checksum; the pseudo-header partial alone is consistent.

**Source**: `bpf/lib/lb.h:742-755`; `bpf/include/linux/bpf.h:1881-1898`.
**Confidence**: High — direct comment in production code identifies the bug class.

### Finding 6: Sequence — store bytes BEFORE updating checksum, two separate `csum_replace` calls

**Evidence** (`bpf/lib/nat.h:489-547`, ordered):

1. `csum_diff(&old_addr, 4, &new_addr, 4, 0)` — compute the address-change folded sum (line 489).
2. `ctx_store_bytes(ctx, l3_off + addr_off, &new_addr, 4, 0)` — write new address bytes to the IP header (line 490).
3. `ipv4_csum_update_by_diff(ctx, l3_off, sum)` — fix the L3 IP-header checksum field (line 494).
4. If port changed: `l4_modify_port` (line 525) — this internally does `ctx_store_bytes` for the port AND a `csum_l4_replace` for the port-only delta. Line 58-61 of `bpf/lib/l4.h`.
5. `csum_l4_replace(ctx, l4_off, &csum, 0, sum, BPF_F_PSEUDO_HDR)` — fix the L4 checksum field for the address change (line 535-536).

**Two distinct `bpf_l4_csum_replace` calls**: one for ports (size=2, no PSEUDO_HDR), one for addresses (size=0 diff, PSEUDO_HDR). They are never combined into a single call.

**Compare Overdrive** (`tc_reverse_nat.rs:207-237`):
1. `l3_csum_replace(off, old_be, new_be, size=4)` — direct form, no PSEUDO_HDR.
2. `l4_csum_replace(off, old_be, new_be, BPF_F_PSEUDO_HDR | 4)` — direct form WITH size=4 AND PSEUDO_HDR.
3. `l4_csum_replace(off, old_port_be, new_port_be, size=2)` — direct form for port, no PSEUDO_HDR. ✓ matches Cilium.
4. `ctx.store(IPV4_SRC_IP_OFFSET, &new_src_ip_be, 0)` — write IP **after** csum update. **Inverse order vs Cilium.**
5. `ctx.store(L4_SRC_PORT_OFFSET, &new_src_port_be, 0)` — write port **after** csum update.

**Two divergences from Cilium's idiom**:
- Address rewrite uses direct form with `size=4` (Cilium uses diff form with `size=0`).
- Bytes are stored **after** the csum update (Cilium stores **before**).

The order divergence is itself a smell because the verifier-and-helper interaction documented in `bpf/include/linux/bpf.h:1905-1909` warns: "A call to this helper is susceptible to change the underlying packet buffer. Therefore, at load time, all checks on pointers previously done by the verifier are invalidated and must be performed again, if the helper is used in combination with direct packet access." Cilium consistently stores first, then csum-updates, to keep the kernel's internal state clean for the second helper call.

**Source**: `bpf/lib/nat.h:475-548`; `bpf/lib/l4.h:54-65`; `bpf/include/linux/bpf.h:1905-1909`.
**Confidence**: High.

### Finding 7: UDP zero-checksum sentinel — `BPF_F_MARK_MANGLED_0` semantics

**Evidence** (`bpf/lib/csum.h:36-39, 73-78`):
```c
case IPPROTO_UDP:
    off->offset = UDP_CSUM_OFF;
    off->flags = BPF_F_MARK_MANGLED_0;
    break;
...
return l4_csum_replace(ctx, (__u32)(l4_off + csum->offset),
                       from, to, flags | csum->flags);
```

Cilium **always** ORs `BPF_F_MARK_MANGLED_0` into UDP `bpf_l4_csum_replace` calls. The kernel docstring at `bpf/include/linux/bpf.h:1894-1897`:
> "With **BPF_F_MARK_MANGLED_0**, a null checksum is left untouched (unless **BPF_F_MARK_ENFORCE** is added as well), and for updates resulting in a null checksum the value is set to **CSUM_MANGLED_0** instead."

i.e., `BPF_F_MARK_MANGLED_0` *enables* the protective behavior — without it, the kernel will rewrite a 0 checksum as if it were a real checksum (turning RFC 768 "no checksum" into garbage), and a recomputation that *yields* 0 would incorrectly leave the checksum at 0 (collision with the sentinel) instead of remapping to `0xFFFF`.

Overdrive's comment at `tc_reverse_nat.rs:218-221` reads the contract inversely:
> "RFC 768 (UDP): csum=0x0000 means 'no checksum computed'. `bpf_l4_csum_replace` preserves the 0 sentinel automatically when `BPF_F_MARK_MANGLED_0` is NOT set"

This is wrong. The kernel's `l4_csum_replace` filter implementation `is_mmzero = flags & BPF_F_MARK_MANGLED_0`; only when `is_mmzero` AND `*sum == 0` does it short-circuit and preserve the 0. Without the flag, the kernel proceeds to recompute and writes the result.

**This is a separate UDP-only latent bug**, not the cause of the length-N TCP drop, but the research surfaces it because the `csum_l4_replace` wrapper Cilium uses transparently fixes it for both protocols.

**Source**: `bpf/lib/csum.h:28-78`; `bpf/include/linux/bpf.h:1881-1898`.
**Confidence**: High.

### Finding 8: Cilium tests use length-N payloads through SNAT

**Evidence**: `pktgen__push_data_room` at `bpf/tests/pktgen.h:634` and `pktgen__push_data` at `bpf/tests/pktgen.h:667` are the helpers tests use to attach payloads to crafted packets. Of 96 TC test files in `bpf/tests/`, 15 use these helpers (e.g. `tc_nodeport_test.c`, `tc_lxc_lb4_*.c`, `tc_redirect_lxc.h`, the `xdp_nodeport_lb4_*` family). Cilium's `BPF_PROG_TEST_RUN` coverage of SNAT/DNAT exercises non-zero TCP payloads, not just SYN-only.

**Implication for Overdrive**: PROG_TEST_RUN with length-N TCP payloads is not what catches the bug — `BPF_PROG_TEST_RUN` synthesizes a fresh skb in `CHECKSUM_NONE` state, where the direct form of `bpf_l4_csum_replace` happens to work. The bug only surfaces in real-veth Tier 3 tests where the skb arrives in `CHECKSUM_PARTIAL` after the peer's send path fills `skb->csum` with the pseudo-header partial.

**Source**: `bpf/tests/pktgen.h:634-680`; `bpf/tests/{tc_nodeport_test,tc_lxc_lb4_*,tc_redirect_*}.c`.
**Confidence**: Medium — claim about `CHECKSUM_NONE` in PROG_TEST_RUN is from the kernel BPF docs (cited from memory of `tools/include/uapi/linux/bpf.h` and tests/bpf/ directory shape; not re-verified against this clone). The contrasting Tier 3 reproduction is the user's empirical observation from the prior research (length-N drops on real veth, length-0 survives) and the Cilium comment at `bpf/lib/lb.h:748-750` corroborates the underlying CHECKSUM_PARTIAL story.

## Side-by-Side Comparison

| Operation | Cilium `snat_v4_rewrite_headers` | Overdrive `rewrite_source_to_vip` | Same? |
|---|---|---|---|
| Compute address-change diff | `csum_diff(&old, 4, &new, 4, 0)` (`nat.h:489`) | Not computed (uses direct form) | ✗ |
| Store new address into packet | `ctx_store_bytes` BEFORE csum update (`nat.h:490`) | `ctx.store` AFTER csum update (`tc_reverse_nat.rs:236`) | ✗ |
| L3 IP checksum update | `l3_csum_replace(off, 0, diff, 0)` — diff form (`ipv4.h:48-53`) | `l3_csum_replace(off, old_be, new_be, 4)` — direct form (`tc_reverse_nat.rs:196-202`) | ✗ |
| L4 checksum update for address change | `l4_csum_replace(off+l4cof, 0, diff, BPF_F_PSEUDO_HDR \| csum->flags)` — diff form, size=0, PSEUDO_HDR set, MARK_MANGLED_0 auto-set for UDP (`nat.h:535-536`, `csum.h:73-78`) | `l4_csum_replace(off+l4cof, old_be, new_be, BPF_F_PSEUDO_HDR \| 4)` — direct form, size=4, PSEUDO_HDR set, no MARK_MANGLED_0 (`tc_reverse_nat.rs:207-213`) | ✗ |
| L4 checksum update for port change | `l4_modify_port` → `l4_csum_replace(off, old_port, new_port, sizeof(port) \| csum->flags)` — direct form, size=2, no PSEUDO_HDR, MARK_MANGLED_0 auto-set for UDP (`l4.h:54-65`) | `l4_csum_replace(off, old_port_be, new_port_be, 2)` — direct form, size=2, no PSEUDO_HDR, no MARK_MANGLED_0 (`tc_reverse_nat.rs:223-229`) | ✓ shape; ✗ MARK_MANGLED_0 for UDP |
| Store new port into packet | `ctx_store_bytes` BEFORE port-csum update (inside `l4_modify_port`, `l4.h:58`) | `ctx.store` AFTER port-csum update (`tc_reverse_nat.rs:237`) | ✗ |
| Number of `bpf_l4_csum_replace` calls | 2 (one for address, one for port) | 2 (one for address, one for port) | ✓ |
| Direction-conditional flag handling | None for IPv4 (always PSEUDO_HDR); IPv6 has `if (dir == CT_EGRESS) flag = BPF_F_PSEUDO_HDR` workaround at `lb.h:752-753` | Always `BPF_F_PSEUDO_HDR` for IPv4 | ✓ for IPv4 |

## The Load-Bearing Difference

Of the seven divergences in the table, the one most plausibly load-bearing for length-N TCP segments at TC egress on `CHECKSUM_PARTIAL` skbs is **the address-rewrite L4 checksum form**:

- **Diff-encoded** (`from=0, to=csum_diff_result, flags=BPF_F_PSEUDO_HDR`): kernel dispatches to `inet_proto_csum_replace_by_diff(skb, sum, 0, csum_diff_result, pseudo_hdr=true)`. This path explicitly handles `skb->ip_summed == CHECKSUM_PARTIAL` by updating both the in-packet checksum field AND `skb->csum` (the pseudo-header partial) consistently.
- **Direct** (`from=old_be, to=new_be, size=4 | BPF_F_PSEUDO_HDR`): kernel dispatches to `inet_proto_csum_replace4(skb, sum, old, new, pseudo_hdr=true)`. Whether this path correctly maintains `skb->csum` consistency under `CHECKSUM_PARTIAL` is the focus of the Cilium IPv6 workaround at `bpf/lib/lb.h:742-755` — the comment there names "a bug in bpf_l4_csum_replace's usage of inet_proto_csum_replace_by_diff" affecting the IPv6 flavor specifically. The IPv4 direct form has no documented bug, but Cilium chose the diff form for SNAT uniformly anyway. The length-N drop pattern (length-0 survives, length-N drops) is consistent with the L4 in-packet field being correct (length-0 has no payload, only pseudo-header partial in `skb->csum` matters) but `skb->csum` for HW offload being wrong on length-N (NIC tx-checksum-offload validation rejects).

The order divergence (store-after vs store-before) is a secondary concern — the verifier invalidates direct-packet pointers across each helper call (`bpf/include/linux/bpf.h:1905-1909`), so the Overdrive code paths are reading possibly-stale address values into the helper's `from` argument. Cilium's store-before-csum order avoids this entirely.

## Recommended Cilium-Aligned Helper-Call Sequence (Pseudocode)

This is what `rewrite_source_to_vip` should look like, transcribed from Cilium's `snat_v4_rewrite_headers` and stripped to the IPv4-TCP/UDP common case Overdrive needs:

```
fn rewrite_source_to_vip(ctx, old_src_ip_be, old_src_port_be, vip, l4_off, l4_csum_off, is_udp):
    new_src_ip_be   = vip.ip_host.to_be()
    new_src_port_be = vip.port_host.to_be()

    # Early-exit if nothing to rewrite (Cilium nat.h:485-487)
    if old_src_ip_be == new_src_ip_be and old_src_port_be == new_src_port_be:
        return TC_ACT_OK

    # (1) Compute folded-sum diff for the address change
    sum = csum_diff(&old_src_ip_be, 4, &new_src_ip_be, 4, 0)

    # (2) Store the new IP into the packet FIRST (Cilium nat.h:490)
    skb_store_bytes(ctx, ETH_HDR_LEN + IPV4_SRC_IP_OFFSET, &new_src_ip_be, 4, 0)

    # (3) Update L3 IP-header checksum using the diff form (Cilium ipv4.h:48-53)
    l3_csum_replace(ctx,
                    ETH_HDR_LEN + IPV4_CSUM_OFFSET,
                    from = 0,
                    to   = sum,
                    flags = 0)              # size=0 → diff form

    # (4) Update L4 checksum for the ADDRESS change using the diff form
    #     (Cilium nat.h:535-536; csum.h:73-78 forces UDP MARK_MANGLED_0)
    extra_flags = BPF_F_MARK_MANGLED_0 if is_udp else 0
    l4_csum_replace(ctx,
                    l4_off + l4_csum_off,
                    from = 0,
                    to   = sum,
                    flags = BPF_F_PSEUDO_HDR | extra_flags)
                                            # size-nibble=0 → diff form;
                                            # PSEUDO_HDR set because address ∈ pseudo-header

    # (5) Store the new port into the packet (Cilium l4.h:58)
    skb_store_bytes(ctx, l4_off + L4_SRC_PORT_OFFSET, &new_src_port_be, 2, 0)

    # (6) Update L4 checksum for the PORT change using the direct form, size=2,
    #     NO PSEUDO_HDR (port is not in pseudo-header). UDP MARK_MANGLED_0
    #     auto-applied via the wrapper. (Cilium l4.h:61, csum.h:73-78)
    l4_csum_replace(ctx,
                    l4_off + l4_csum_off,
                    from = old_src_port_be,
                    to   = new_src_port_be,
                    flags = 2 | extra_flags) # size=2, NO PSEUDO_HDR

    return TC_ACT_OK
```

Five concrete changes vs. Overdrive's current `rewrite_source_to_vip`:
1. Add `csum_diff` precomputation of the IP-change folded sum.
2. Switch L3 update to the diff form (`from=0, to=sum, size=0`).
3. Switch L4 address update to the diff form (`from=0, to=sum, flags=BPF_F_PSEUDO_HDR`, size-nibble=0).
4. Reorder: store new IP/port bytes BEFORE the corresponding csum update for each.
5. Add `BPF_F_MARK_MANGLED_0` to L4 calls when `is_udp`.

Aya-rs translation: aya's `TcContext::l4_csum_replace(off, from, to, flags)` maps to the kernel helper directly. Construct `flags` as `u64` ORs of `BPF_F_PSEUDO_HDR`, `BPF_F_MARK_MANGLED_0`, and the size-nibble (0 for diff, 2 or 4 for direct). The crafter handles type plumbing.

## Cilium-Specific Assumptions Overdrive Does NOT Share

1. **Cilium uses `bpf_redirect_neigh` extensively** (`bpf/lib/nodeport.h`, `bpf/lib/lb.h:1721, 1784`) for return path. Overdrive's `tc_reverse_nat` is for veth-internal forwarding without a redirect helper. **No impact** on the rewrite shape — Cilium calls `snat_v4_rewrite_headers` regardless of whether redirect-neigh follows.
2. **Cilium has `ENABLE_SCTP` paths** that explicitly skip the L4 rewrite for SCTP because BPF cannot compute crc32c (`bpf/lib/csum.h:40-54`). Overdrive doesn't ship SCTP — irrelevant.
3. **Cilium's `csum_l4_offset_and_flags` returns `flags=0` for ICMPv6** with a separate ICMP checksum-offset path. Overdrive currently rewrites only TCP/UDP — irrelevant.
4. **Cilium handles fragmentation via `ipfrag_has_l4_header(fraginfo)`** (`bpf/lib/nat.h:906`): non-first fragments skip the L4 rewrite entirely. If Overdrive can receive fragmented IPv4 at TC egress (unlikely on veth-internal but possible), the same gate is needed. **Out of scope for the immediate length-N fix**, but flagged.
5. **Cilium tests with `BPF_PROG_TEST_RUN`-synthesized skbs in CHECKSUM_NONE state** — different state machine than the real-veth `CHECKSUM_PARTIAL` path. Overdrive's Tier 2 PROG_TEST_RUN tests will likely pass for length-N even with the buggy code; the regression must be caught at Tier 3 (real veth) per `.claude/rules/testing.md`. Flagged to the crafter for test-tier selection.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Cilium master `bpf/lib/nat.h` | github.com/cilium/cilium | High | Production source | 2026-05-07 | Y (lb.h, csum.h) |
| Cilium master `bpf/lib/csum.h` | github.com/cilium/cilium | High | Production source | 2026-05-07 | Y (l4.h, ipv4.h, nat.h) |
| Cilium master `bpf/lib/l4.h` | github.com/cilium/cilium | High | Production source | 2026-05-07 | Y (nat.h:525) |
| Cilium master `bpf/lib/lb.h` | github.com/cilium/cilium | High | Production source | 2026-05-07 | Y (nat.h same shape) |
| Cilium master `bpf/lib/ipv4.h` | github.com/cilium/cilium | High | Production source | 2026-05-07 | Y (nat.h:494, lb.h:1569) |
| Cilium-vendored Linux UAPI `bpf/include/linux/bpf.h` | kernel.org via Cilium tree | High | Kernel canonical | 2026-05-07 | Y (kernel docstring) |
| Overdrive `tc_reverse_nat.rs` | local repo | High | Production source (subject) | 2026-05-07 | (subject) |

Reputation: High: 7 (100%) | Avg: 1.0

## Knowledge Gaps

### Gap 1: Direct kernel-source verification of `inet_proto_csum_replace4` vs `inet_proto_csum_replace_by_diff` under CHECKSUM_PARTIAL

**Issue**: The Finding 5 attribution of the length-N drop to "direct form mishandles `skb->csum` under CHECKSUM_PARTIAL" relies on the Cilium IPv6 comment at `lb.h:742-755`. The Cilium comment names the bug for the IPv6 16-byte path (`inet_proto_csum_replace_by_diff` updating `skb->csum` when CHECKSUM_COMPLETE) — the symmetric-but-inverse claim (the IPv4 4-byte direct path NOT updating `skb->csum` when CHECKSUM_PARTIAL) is plausible-and-consistent but not directly verified against `net/core/filter.c` source.

**Attempted**: Grepped Cilium's tree for kernel-side `filter.c` excerpts; only UAPI headers are vendored. Web search for the specific kernel function not in scope of this dispatch.

**Recommendation**: Before accepting the proposed fix, verify against `net/core/filter.c` `bpf_l4_csum_replace` implementation in the Lima kernel (Ubuntu 24.04, 6.8) — specifically the `inet_proto_csum_replace4` vs `inet_proto_csum_replace_by_diff` dispatch and how each handles `skb->ip_summed == CHECKSUM_PARTIAL`. Adjacent option: empirically falsify by switching only one of the four shape divergences at a time (e.g., diff form alone, store-before-csum alone) to identify which is load-bearing.

### Gap 2: Cilium git log not consulted

**Issue**: The dispatch asked for relevant historical commits with SHAs; this research did not run `git log` against the Cilium tree.

**Attempted**: Time-budget-bound; the live source already encodes the current correct shape, and the load-bearing comment at `lb.h:742-755` is in-tree. Historical commit archeology would corroborate but not change the recommendation.

**Recommendation**: If reproducibility-of-fix is important, run `git -C /Users/marcus/git/cilium/cilium log --oneline -- bpf/lib/csum.h bpf/lib/nat.h | grep -iE "checksum|csum|partial|length"` to find the commits where Cilium adopted the diff form vs the direct form and the rationale messages.

### Gap 3: `BPF_F_MARK_MANGLED_0` impact on the immediate TCP-length-N drop

**Issue**: Finding 7 surfaces a UDP-only zero-checksum bug. The dispatch asks specifically about TCP length-N drop. UDP has no equivalent issue at zero (TCP has no zero-checksum sentinel — RFC 793 requires a real checksum).

**Recommendation**: When the crafter applies the fix, do not conflate the two. The TCP fix (diff form, store-before, PSEUDO_HDR on address) and the UDP fix (`MARK_MANGLED_0` on every UDP `bpf_l4_csum_replace` call) are independent. Both should land, but in separate commits with separate test coverage.

## Conflicting Information

None. All Cilium code paths consulted (`nat.h`, `lb.h`, `ipv4.h`, `l4.h`, `csum.h`, `nat_46x64.h`, `nodeport.h`) use identical shape: diff-encoded address rewrite + direct-form port rewrite + store-before-csum + auto-`MARK_MANGLED_0` for UDP via the wrapper.

## Recommendations for Further Research

1. **Run `git log` against Cilium to find the commits where the diff form was chosen** — confirms the rationale and surfaces edge cases the comment alone may not name.
2. **Read `net/core/filter.c` `bpf_l4_csum_replace` and the underlying `inet_proto_csum_replace*` family** to confirm the CHECKSUM_PARTIAL handling is asymmetric between direct and diff forms for IPv4. This is the single empirical claim the recommendation rests on.
3. **Consider a Tier 2 PROG_TEST_RUN test that explicitly puts the skb into CHECKSUM_PARTIAL state** before dispatching to `tc_reverse_nat`. The standard PROG_TEST_RUN entry creates CHECKSUM_NONE skbs; CHECKSUM_PARTIAL has to be set via `__skb_set_pseudo_hdr` or by sending through a real socket. If a Tier 2 reproduction is feasible, it dramatically shortens the feedback loop vs Tier 3 veth.

## Full Citations

[1] Cilium Authors. "bpf/lib/nat.h — IPv4 SNAT rewrite headers". Cilium master. Local clone at `/Users/marcus/git/cilium/cilium/bpf/lib/nat.h`. Lines 475-548. Accessed 2026-05-07.

[2] Cilium Authors. "bpf/lib/csum.h — L4 checksum offset and replace helpers". Cilium master. Local clone. Lines 28-78. Accessed 2026-05-07.

[3] Cilium Authors. "bpf/lib/l4.h — l4_modify_port". Cilium master. Local clone. Lines 54-65. Accessed 2026-05-07.

[4] Cilium Authors. "bpf/lib/lb.h — IPv4 LB DNAT and IPv6 CHECKSUM_PARTIAL workaround". Cilium master. Local clone. Lines 670-755, 1555-1593, 2000-2025. Accessed 2026-05-07.

[5] Cilium Authors. "bpf/lib/ipv4.h — ipv4_csum_update_by_diff". Cilium master. Local clone. Lines 40-53. Accessed 2026-05-07.

[6] Linux kernel community (vendored by Cilium). "include/uapi/linux/bpf.h — bpf_l4_csum_replace docstring and flag definitions". Cilium master `bpf/include/linux/bpf.h`. Lines 1881-1909, 5885-5900. Accessed 2026-05-07.

[7] Overdrive (subject of comparison). "crates/overdrive-bpf/src/programs/tc_reverse_nat.rs — current rewrite_source_to_vip". Local repo at `/Users/marcus/conductor/workspaces/helios/curitiba-v1`. Lines 167-240. Accessed 2026-05-07.

## Research Metadata

Duration: ~30 turns | Examined: 8 Cilium source files + 1 Overdrive file | Cited: 7 | Cross-refs: 5 (each major claim verified across `nat.h`, `lb.h`, `csum.h`) | Confidence: High 8/8 findings, Medium 1/8 (Finding 8) | Output: `docs/research/dataplane/cilium-snat-csum-rewrite-prior-art-research.md`
