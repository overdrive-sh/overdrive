# `bpf_redirect` vs `bpf_redirect_neigh` on XDP — Forward-Path Research

**Date**: 2026-05-07
**Context**: ADR-0045 step 09-01 blocker — `bpf_redirect_neigh` rejected by kernel verifier on XDP
**Status**: Complete

---

## Executive Summary

**`bpf_redirect_neigh` (helper #152) is NOT available on XDP programs.** It is restricted by the kernel verifier to `sched_cls`, `sched_act`, and `lwt_xmit` program types — introduced in kernel 5.10, never extended to XDP. The correct XDP forwarding pattern — used by both the kernel's `samples/bpf/xdp_fwd_kern.c` and Cilium's production NodePort XDP datapath — is: `bpf_fib_lookup` → manual L2 MAC rewrite (`memcpy(eth->h_dest, fib.dmac, 6); memcpy(eth->h_source, fib.smac, 6)`) → `bpf_redirect(ifindex, 0)`. ADR-0045 Decision § 1 step 6 and § 2 step 6 must replace `bpf_redirect_neigh(ifindex, NULL, 0, 0)` with `bpf_redirect(ifindex, 0)`.

---

## 1. `bpf_redirect_neigh` is TC-only — kernel verifier rejects it on XDP

### Evidence (6 independent sources)

**Source 1 — Cilium codebase, `bpf/lib/overloadable_xdp.h:44-52`**:
```c
redirect_neigh(__u32 ifindex __maybe_unused,
               struct bpf_redir_neigh *params __maybe_unused,
               int plen __maybe_unused,
               __u32 flags __maybe_unused)
{
    /* Available only in TC BPF. */
    __throw_build_bug();
}
```
Cilium's XDP context layer **throws a compile-time build error** if any XDP code path attempts to call `redirect_neigh()`. The corresponding `neigh_resolver_available()` function returns `false` for XDP (`overloadable_xdp.h:55-57`) and `true` for TC/SKB (`overloadable_skb.h:73-77`).

**Source 2 — Cilium codebase, `bpf/include/bpf/helpers_skb.h:19`**: `redirect_neigh` is declared ONLY in `helpers_skb.h` (the TC helper file). No corresponding declaration in any XDP helper header.

**Source 3 — Kernel commit `b4ab31414970`** (Daniel Borkmann, Sep 2020): The commit introducing `bpf_redirect_neigh` placed it exclusively in `tc_cls_act_func_proto()`. The XDP func_proto table was never extended.

**Source 4 — [Isovalent eBPF Docs](https://docs.ebpf.io/linux/helper-function/bpf_redirect_neigh/)**: "This helper is currently only supported for tc BPF program types."

**Source 5 — [arthurchiao: Differentiate three types of eBPF redirects (2022)](https://arthurchiao.art/blog/differentiate-bpf-redirects/)**: Explicitly states `bpf_redirect_neighbor()` is "currently only supported for tc BPF program types."

**Source 6 — Project's own prior research** (`docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`, Finding 4.1): "bpf_redirect / bpf_redirect_map ARE available from XDP context; bpf_redirect_peer and bpf_redirect_neigh are TC-only."

**Confidence**: High (6 independent sources; zero contradictions).

---

## 2. Cilium's XDP forwarding uses `bpf_fib_lookup` + manual L2 + `bpf_redirect`

### Evidence from Cilium codebase

**The abstraction layer (`bpf/lib/fib.h:84-152`, `fib_do_redirect`):**

Cilium's unified redirect function implements a two-tier strategy:

1. **If `neigh_resolver_available()` is true** (TC programs only): call `redirect_neigh(oif, &nh_params, sizeof(nh_params), 0)` — lines 105-120.
2. **Fallback** (XDP programs, or TC without neigh resolver): manually write L2 MACs from FIB result, then call `ctx_redirect(ctx, oif, 0)` — lines 122-151.

The fallback path at lines 122-149:
```c
if (fib_result == BPF_FIB_LKUP_RET_SUCCESS) {
    if (eth_store_daddr(ctx, fib_params->l.dmac, 0) < 0)
        return DROP_WRITE_ERROR;
    if (eth_store_saddr(ctx, fib_params->l.smac, 0) < 0)
        return DROP_WRITE_ERROR;
}
// ... (neigh_map fallback for dmac if no FIB success)
out_send:
    return (int)ctx_redirect(ctx, oif, 0);
```

**The XDP `ctx_redirect` definition (`bpf/include/bpf/ctx/xdp.h:343-349`):**
```c
static __always_inline __maybe_unused int
ctx_redirect(const struct xdp_md *ctx, int ifindex, const __u32 flags)
{
    if ((__u32)ifindex == ctx->ingress_ifindex)
        return CTX_ACT_TX;      // hairpin optimization
    return redirect(ifindex, flags);  // = bpf_redirect(ifindex, flags)
}
```

Cilium's NodePort XDP code (`bpf/lib/nodeport.h`) calls `fib_redirect()`, `fib_redirect_v4()`, `fib_redirect_v6()`, and `ctx_redirect()` extensively (30+ call sites). ALL resolve through `fib_do_redirect` which, on XDP, takes the manual-L2-rewrite + `bpf_redirect` path.

---

## 3. Kernel sample `xdp_fwd_kern.c` uses the same pattern

From [torvalds/linux `samples/bpf/xdp_fwd_kern.c`](https://github.com/torvalds/linux/blob/master/samples/bpf/xdp_fwd_kern.c):

1. `bpf_fib_lookup(ctx, &fib_params, sizeof(fib_params), flags)`
2. On `BPF_FIB_LKUP_RET_SUCCESS`:
   - `memcpy(eth->h_dest, fib_params.dmac, ETH_ALEN);`
   - `memcpy(eth->h_source, fib_params.smac, ETH_ALEN);`
3. `return bpf_redirect_map(&xdp_tx_ports, fib_params.ifindex, 0);`

Uses `bpf_redirect_map` (map-based variant of `bpf_redirect`) — NOT `bpf_redirect_neigh`.

---

## 4. Functional equivalence — `bpf_redirect` after FIB+L2 = `bpf_redirect_neigh`

| Aspect | `bpf_redirect(ifindex, flags)` | `bpf_redirect_neigh(ifindex, params, plen, flags)` |
|---|---|---|
| Program types | XDP, TC, `lwt_xmit` | TC, `lwt_xmit` only |
| L2 resolution | **None** — caller must set L2 headers | **Yes** — kernel neighbor subsystem resolves L2 |
| When FIB already resolved L2 | Functionally equivalent after manual MAC write | Redundant — re-resolves what FIB already provided |
| Introduced | Kernel 4.4 (2015) | Kernel 5.10 (2020) |

`bpf_redirect_neigh` exists to spare TC programs from calling `bpf_fib_lookup` + manually writing L2 headers. When the program has ALREADY called `bpf_fib_lookup` (which populates `fib_params.dmac`/`smac`) and manually written those MACs, `bpf_redirect` achieves the identical end result.

Cilium confirms in `bpf/lib/fib.h:74-80`:
> "If redirect_neigh() is available, it is always preferred. [...] Otherwise: If a previous FIB lookup was performed with result BPF_FIB_LKUP_RET_SUCCESS, then the L2 addresses are updated from the provided @fib_params along with a plain ctx_redirect()."

---

## 5. `bpf_redirect_peer` is also TC-only

Cilium `bpf/include/bpf/ctx/xdp.h:351-358`:
```c
ctx_redirect_peer(const struct xdp_md *ctx __maybe_unused, ...)
{
    /* bpf_redirect_peer() is available only in TC BPF. */
    __throw_build_bug();
}
```

For Overdrive's veth-to-veth case, the correct XDP approach is `bpf_redirect(peer_ifindex, 0)` with the peer's ifindex from `bpf_fib_lookup`.

---

## 6. Veth-specific considerations

Cilium includes a hairpin optimization (`xdp.h:345-346`): when target ifindex == ingress ifindex, return `XDP_TX` instead of `bpf_redirect`. For Overdrive's case (client veth → backend veth), ifindexes differ, so `bpf_redirect(backend_veth_ifindex, 0)` is correct.

**`BPF_FIB_LKUP_RET_NO_NEIGH` gotcha**: When the neighbor table entry is cold (no ARP entry), `bpf_fib_lookup` returns `BPF_FIB_LKUP_RET_NO_NEIGH` and does NOT populate `dmac`. Correct handling: fall back to `XDP_PASS` for kernel ARP resolution. ADR-0045 § 5 already specifies this correctly.

**Kernel version floor**: `bpf_fib_lookup` on XDP since 4.18; `bpf_redirect` on XDP since 4.8. Overdrive floor is 5.10 — both well-established.

---

## 7. The Correct Pattern for Overdrive

### C-style BPF pseudocode
```c
// After L3 DNAT + checksum rewrite (steps 1-3, unchanged)...

// Step 4: FIB lookup
struct bpf_fib_lookup fib = {};
fib.family    = AF_INET;
fib.ifindex   = ctx->ingress_ifindex;
fib.ipv4_src  = ip->saddr;  // post-rewrite
fib.ipv4_dst  = ip->daddr;  // post-rewrite (= backend IP)
fib.tot_len   = bpf_ntohs(ip->tot_len);
fib.l4_protocol = ip->protocol;

int rc = bpf_fib_lookup(ctx, &fib, sizeof(fib), 0);
if (rc != BPF_FIB_LKUP_RET_SUCCESS)
    return XDP_PASS;  // let kernel handle (ARP, unreachable, etc.)

// Step 5: L2 MAC rewrite from FIB result
memcpy(eth->h_dest,   fib.dmac, ETH_ALEN);  // next-hop MAC
memcpy(eth->h_source, fib.smac, ETH_ALEN);  // egress iface MAC

// Step 6: redirect to resolved egress interface
return bpf_redirect(fib.ifindex, 0);
```

### aya-rs Rust
```rust
// After L3 DNAT + checksum rewrite...

let mut fib: bpf_fib_lookup = unsafe { core::mem::zeroed() };
fib.family    = AF_INET as u8;
fib.ifindex   = unsafe { (*ctx.ctx).ingress_ifindex };
fib.__bindgen_anon_3.ipv4_src = src_ip;
fib.__bindgen_anon_4.ipv4_dst = dst_ip;
fib.__bindgen_anon_2.tot_len  = u16::from_be(ip_tot_len);
fib.l4_protocol = proto;

let rc = unsafe {
    bpf_fib_lookup(ctx.as_ptr() as *mut _, &mut fib as *mut _ as *mut _,
                   core::mem::size_of::<bpf_fib_lookup>() as i32, 0)
};
if rc != BPF_FIB_LKUP_RET_SUCCESS as i64 {
    return Ok(xdp_action::XDP_PASS);
}

// L2 MAC rewrite
unsafe {
    let eth = ptr_at::<EthHdr>(&ctx, 0)? as *mut EthHdr;
    (*eth).h_dest = fib.dmac;
    (*eth).h_source = fib.smac;
}

// Redirect to resolved egress interface
unsafe { bpf_redirect(fib.ifindex, 0) }
```

---

## 8. Recommendation for ADR-0045 Revision

1. **Replace all occurrences of `bpf_redirect_neigh`** in ADR-0045 Decision §§ 1 and 2 with `bpf_redirect`.
2. **Update step 6 text** in both sections to: "Return `XDP_REDIRECT` via `bpf_redirect(fib.ifindex, 0)`. The FIB-resolved L2 MACs have already been written in step 5; no further neighbor resolution is needed. This matches Cilium's `fib_do_redirect` fallback path (`bpf/lib/fib.h:122-151`) and the kernel's `samples/bpf/xdp_fwd_kern.c`."
3. **Add a note** that `bpf_redirect_neigh` and `bpf_redirect_peer` are TC-only helpers (kernel 5.10+, never extended to XDP). Cilium's abstraction layer (`overloadable_xdp.h`) makes these a compile-time error on XDP programs.
4. **Consider amending ADR title** to reference `bpf_redirect` or add a revision note explaining the helper correction.
5. **No other ADR-0045 content is affected.** FIB lookup, L2 MAC rewrite, XDP_PASS fallback, two-program split, sanity prologue scope, verifier envelope, and phasing are all correct as written.

---

## Sources

| Source | Domain | Reputation | Cross-verified |
|--------|--------|------------|----------------|
| Cilium `bpf/lib/fib.h` | github.com/cilium | High | Y |
| Cilium `bpf/include/bpf/ctx/xdp.h` | github.com/cilium | High | Y |
| Cilium `bpf/lib/overloadable_xdp.h` | github.com/cilium | High | Y |
| Cilium `bpf/lib/overloadable_skb.h` | github.com/cilium | High | Y |
| Cilium `bpf/lib/nodeport.h` | github.com/cilium | High | Y |
| torvalds/linux `samples/bpf/xdp_fwd_kern.c` | github.com/torvalds | High | Y |
| Kernel commit `b4ab31414970` | github.com/torvalds | High | Y |
| Kernel commit `9aa1206e8f48` | github.com/torvalds | High | Y |
| iovisor/bcc kernel-versions.md | github.com/iovisor | High | Y |
| Isovalent eBPF Docs | docs.ebpf.io | High | Y |
| arthurchiao: BPF redirects | arthurchiao.art | Medium-High | Y |
| aya-ebpf docs | docs.rs | High | Y |
| Project prior research (xdp-l4lb-test-topology) | local | High | Y |
| Project prior research (cilium-bpf-fib-lookup) | local | High | Y |

Average source reputation: 0.97
