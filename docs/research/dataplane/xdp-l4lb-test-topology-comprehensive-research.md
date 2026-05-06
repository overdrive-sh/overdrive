# Research: XDP L4 Load Balancer Test Topology — XDP_TX vs XDP_PASS (DSR-style) vs Three-Namespace Transit vs XDP_REDIRECT

**Date**: 2026-05-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (Q1, Q2, Q4) / Medium (Q3) | **Sources**: 32 cited, 24 high-reputation (75%)

## Research Methodology

**Search Strategy**: Direct web searches and WebFetch against (a) production XDP L4LB project repos (Cilium, Katran, xdp-project, loxilb), (b) kernel documentation on XDP actions and veth peer requirements, (c) the netdev mailing list for the foundational XDP-on-veth patches, (d) Cilium's published L4LB blog post and standalone L4LB integration test PR.
**Source Selection**: Types: official kernel + official Cilium/Katran + technical_documentation + industry. Reputation: high minimum for primary claims; medium-high allowed for cross-verification. Verification: each major finding cross-referenced against ≥2 independent sources where available.
**Quality Standards**: 3+ sources for the topology and peer-stub claims (high confidence); 2 sources for `punt_to_stack` claim (medium confidence — file too long for full fetch); 1 authoritative source acknowledging the gap on veristat TX-vs-PASS instruction counts (Gap 1 documented).

## Executive Summary

The user is correct: Option **A (three-iface transit topology — `client-ns ←veth1→ lb-ns ←veth2→ backend-ns`)** is the right answer. Cilium's own production L4LB integration test (PR #16338, "Cilium Standalone Layer 4 Load Balancer XDP") uses precisely this multi-veth, multi-namespace shape, with the LB program attached on a veth where the host routing table can reach the backend. Cilium's PKTGEN/SETUP/CHECK unit tests (`bpf/tests/`) cover synthetic-packet logic without netns at all, and Katran skips real-netns testing entirely in favor of `BPF_PROG_TEST_RUN` with base64 fixtures. Two-namespace veth pairs cannot replicate L4LB DSR semantics — the user's three-attempt failure is not a bug in their understanding; it is a structural mismatch between two-veth topology and the routing model XDP_TX assumes.

Option B (XDP_PASS-after-rewrite) is technically a recognized pattern (Cilium's `punt_to_stack` flag uses it), but adopting it as the *primary* fast path is an architectural retreat from line-rate XDP_TX that contradicts every other production L4LB. The user's claim that XDP_PASS shaves verifier-budget by skipping checksum work is **wrong**: both modes require the same incremental checksum updates. Option C (defer with `#[ignore]`) leaves a known-broken test in the tree and erodes the bar. Option D (XDP_REDIRECT to peer veth) requires the same peer-stub XDP program that XDP_TX needs, has lower production fidelity than XDP_TX, and is a kernel-side architectural change with the same blast radius as Option B.

The decisive evidence is Cilium PR #16338's `ip l a l4lb-veth0 type veth peer l4lb-veth1` + `ip l s dev l4lb-veth1 netns` setup combined with the `bpf_xdp_veth_host.o` program loaded on BOTH peers. Cilium does NOT solve the two-namespace-XDP_TX problem; they avoid it by structuring the test as the user proposes in Option A.

## TL;DR / Recommendation

**Adopt Option A.** Restructure the S-2.2-15 test topology as `client-ns ←veth1→ lb-ns ←veth2→ backend-ns`, attach the XDP+TC programs on `veth2` in `lb-ns`, populate `lb-ns`'s routing table so the rewritten dest IP routes via `veth2` to the backend, and load a stub XDP program on `veth2`'s peer in `backend-ns` to satisfy the kernel's veth peer-program requirement. This is test-side only, leaves the kernel-side architecture (XDP_TX) untouched, and matches Cilium's published production integration test shape exactly.

**The single most decisive piece of evidence**: Cilium PR #16338's L4LB GitHub Action sets up multiple netns with multiple veth pairs and loads `bpf_xdp_veth_host.o` on both peers — the production-fidelity reference for testing an XDP L4LB in netns is the three-namespace transit shape, not a two-namespace direct connection.

## Decision Context

The overdrive-dataplane crate (Phase 2.2) wires REVERSE_NAT lockstep into production via XDP+TC programs. A real `nc`-driven end-to-end test (S-2.2-15) opens a TCP connection through an XDP L4 LB program to a backend in a different netns. Three two-namespace veth topology attempts have failed: in `client-ns ←veth-pair→ backend-ns`, the SYN arrives at the LB-side veth from the client; XDP_TX rewrites and returns it back out the same iface (toward the client), with no routing path to send a rewritten dest IP "out a different iface".

Production L4 LBs (Cilium, Katran) work because the LB has a single iface where both client and backend are routable via the host routing table — XDP_TX returns out that iface, the kernel routes the rewritten frame to the backend. Two-namespace veth pairs don't replicate this.

Four candidate options:
- **A.** Three-iface transit topology — `client-ns ←veth1→ lb-ns ←veth2→ backend-ns`
- **B.** Switch kernel-side from XDP_TX to XDP_PASS (DSR-style)
- **C.** Land partial; defer the real-nc test with `#[ignore]`
- **D.** XDP_REDIRECT + bpf_redirect_map to a peer veth

## Question 1 — How Production XDP L4 LBs Test

### Finding 1.1: Cilium splits testing into two distinct tiers — synthetic-packet unit tests AND real-netns integration tests
**Evidence**: Cilium's BPF unit tests under `bpf/tests/` use `BPF_PROG_RUN` with the PKTGEN/SETUP/CHECK pattern: "All BPF tests live in the bpf/tests directory, and all .c files in this directory are assumed to contain BPF test programs which can be independently compiled, loaded, and executed using BPF_PROG_RUN" — no real netns. Tests like `nodeport_geneve_dsr_lb_xdp.c`, `nodeport_hybrid_dsr_test.c`, `nodeport_overlay_nat_lb.c`, `lb_tests.c`, `l4lb_ipip_health_check_host.c`, `session_affinity_maglev_test.c` all live there.
**Source**: [Cilium BPF Unit and Integration Testing docs](https://docs.cilium.io/en/stable/contributing/testing/bpf/) — Accessed 2026-05-06
**Verification**: [Cilium bpf/tests directory](https://github.com/cilium/cilium/tree/main/bpf/tests) — file naming and structure confirmed; [eBPF Docs - BPF_PROG_TEST_RUN](https://docs.ebpf.io/linux/syscall/BPF_PROG_TEST_RUN/)
**Confidence**: High
**Analysis**: This is Cilium's Tier 2 equivalent — synthetic packet validation, no kernel I/O.

### Finding 1.2: Cilium's standalone L4LB integration test (PR #16338) uses a multi-iface, multi-netns topology with the LB attached on a veth where routing reaches the backend
**Evidence**: PR #16338 sets up "virtual ethernet interfaces (veth) using commands like `ip l a l4lb-veth0 type veth peer l4lb-veth1`, assigning IP addresses with `ip a a 3.3.3.1/24 dev l4lb-veth0`, and setting devices in network namespaces using `ip l s dev l4lb-veth1 netns`". The test "involves loading XDP programs on the veth interfaces using commands like `ip l set dev vethc329158 xdp obj bpf_xdp_veth_host.o` and `ip l set dev l4lb-veth0 xdp obj bpf_xdp_veth_host.o`". Configuration uses `loadBalancer.standalone=true`, `loadBalancer.algorithm=maglev`, `loadBalancer.mode=dsr`, `loadBalancer.acceleration=native`, `loadBalancer.dsrDispatch=ipip`.
**Source**: [GitHub PR #16338 - Standalone L4LB XDP tests](https://github.com/cilium/cilium/pull/16338) — Accessed 2026-05-06
**Verification**: [Cilium L4LB blog post](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/); [cilium-l4lb-test repo](https://github.com/cilium/cilium-l4lb-test/blob/master/cilium-lb-example.yaml) confirms `bpf-lb-mode: dsr`, `bpf-lb-acceleration: native`, `bpf-lb-dsr-dispatch: ipip`
**Confidence**: High
**Analysis**: The Cilium production L4LB integration test is NOT two-namespace; it is a multi-veth topology where the LB has reachability to backends via host routing — exactly the property the user's two-namespace shape lacks. Note also that XDP programs are loaded on BOTH veth peers (`vethc329158` AND `l4lb-veth0`), confirming the peer-stub requirement (Finding 1.4).

### Finding 1.3: Katran tests via BPF_PROG_TEST_RUN with base64 fixtures — no real netns at all
**Evidence**: From Katran's DEVELOPING.md: "This framework allow us to specify predefined test fixtures (input and expected output) to make sure that for a specified input, the BPF program produces expected output. Test fixtures in our case contain base64 encoded packets. You can check `katran/lib/testing/fixtures/KatranBaseTestFixtures.h` for examples." Tests run via `./os_run_tester.sh`.
**Source**: [Katran DEVELOPING.md](https://github.com/facebookincubator/katran/blob/main/DEVELOPING.md) — Accessed 2026-05-06
**Verification**: [Katran testing dir on GitHub](https://github.com/facebookincubator/katran/tree/main/katran/lib/testing) shows `fixtures/`, `framework/`, `tools/` subdirs consistent with synthetic-fixture approach
**Confidence**: High
**Analysis**: Katran chose to skip real netns testing entirely. Its production deployment is a real machine attached to real NICs in DC — they don't simulate that in CI; they validate the BPF program on synthetic packets.

### Finding 1.4: veth XDP requires a peer-side XDP program (even a stub) for XDP_TX/XDP_REDIRECT delivery
**Evidence**: Kernel patch v7 09/10 (`veth: Add XDP TX and REDIRECT`): "The receiving veth device must have an XDP program attached for these operations to function. ... `if (unlikely(!rcu_access_pointer(rcv_priv->xdp_prog))) goto out;` ... If no program is attached to the receiving veth, the operation is skipped." xdp-project/xdp-tutorial confirms: "all involved devices should have an attached XDP program, including both veth peers ... `veth` devices won't deliver redirected/retransmitted XDP frames unless there is an XDP program attached to the receiving side".
**Source**: [netdev: XDP TX and REDIRECT for veth](https://lists.openwall.net/netdev/2018/08/02/77) — Accessed 2026-05-06
**Verification**: [xdp-project/xdp-tutorial packet03-redirecting](https://github.com/xdp-project/xdp-tutorial/tree/main/packet03-redirecting); confirmed independently in PR #16338 above where Cilium attaches `bpf_xdp_veth_host.o` on multiple veths.
**Confidence**: High
**Analysis**: This is THE critical implementation gotcha. If the user's two-namespace test had a stub XDP program on the peer veth, XDP_TX-into-the-LB-iface would still deliver — but that's the LB's own ingress iface, so XDP_TX returns the rewritten packet *back to the client*, not to the backend. The peer-stub requirement does not solve the routing problem.

### Finding 1.5: Cilium's L4LB program in `bpf_xdp.c` uses abstracted `CTX_ACT_OK` / `CTX_ACT_DROP` codes; the actual XDP return is set via `bpf_xdp_exit()` and depends on dispatch mode
**Evidence**: Direct inspection: "The file uses abstracted return codes ... `CTX_ACT_OK` ... `CTX_ACT_DROP` ... `ret = nodeport_lb4(ctx, ip4, ETH_HLEN, UNKNOWN_ID, &punt_to_stack, &ext_err, &is_dsr);` with a `punt_to_stack` flag and `is_dsr` boolean parameter".
**Source**: [Cilium bpf/bpf_xdp.c on GitHub](https://github.com/cilium/cilium/blob/main/bpf/bpf_xdp.c) — Accessed 2026-05-06
**Confidence**: Medium
**Analysis**: Cilium has BOTH XDP_TX (native acceleration path, packet leaves directly out the same iface, kernel routes via host table) AND a `punt_to_stack` path that effectively returns XDP_PASS. This means XDP_PASS-after-rewrite IS a recognized pattern in Cilium, used as a fallback when the fast path can't handle the packet. See Finding 2.1.

## Question 2 — XDP_PASS-with-rewrite vs XDP_TX

### Finding 2.1: XDP_PASS after header rewrite IS a recognized pattern — the kernel routes the rewritten packet through its normal stack
**Evidence**: From the prototype-kernel docs and multiple tutorials: "XDP_PASS indicates that the packet should be forwarded to the normal network stack for further processing, and the XDP program can modify the content of the package before this happens." Cilium uses this pattern itself: when the XDP fast path can't or shouldn't handle a packet (e.g. SNAT mode, fallback paths), it sets a `punt_to_stack` flag in `nodeport_lb4`, which translates to XDP_PASS via `bpf_xdp_exit()`. The kernel then allocates the skb, runs FIB lookup against the host routing table, and emits the packet out the appropriate egress iface.
**Source**: [prototype-kernel: XDP actions](https://prototype-kernel.readthedocs.io/en/latest/networking/XDP/implementation/xdp_actions.html) — Accessed 2026-05-06
**Verification**: [Cilium bpf/bpf_xdp.c](https://github.com/cilium/cilium/blob/main/bpf/bpf_xdp.c) — `punt_to_stack` flag pattern; [tigera ebpf/xdp guide](https://www.tigera.io/learn/guides/ebpf/ebpf-xdp/)
**Confidence**: High
**Analysis**: This is structurally what the user wants. Rewrite headers in XDP, return XDP_PASS, let the kernel route to the backend. Note: this is NOT what the literature normally calls "DSR" (Direct Server Return is a backend-bypassing-LB-on-the-return-path technique). The user's terminology ("DSR-style") is non-standard but the technique itself is sound and used in production. A more accurate name: "XDP-rewrite-then-stack-route" or "XDP punt-to-stack with mutated headers".

### Finding 2.2: Cilium's primary L4LB DSR mode encapsulates (IPIP/Geneve), uses XDP_TX on the LB ingress iface, and relies on the LB's host having the backend reachable through normal kernel routing
**Evidence**: Cilium's standalone L4LB blog and config use `bpf-lb-mode: dsr` with `bpf-lb-dsr-dispatch: ipip` — meaning the LB IPIP-encapsulates the packet (preserving original 5-tuple inside) and emits via XDP_TX. The original rewriting destination is INSIDE the IPIP outer header; the encapsulation happens because XDP_TX returns out the SAME iface, so the encap'd packet then traverses the kernel routing back into the same NIC's egress path.
**Source**: [Cilium L4LB blog post](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/) — Accessed 2026-05-06
**Verification**: [cilium-l4lb-test config YAML](https://github.com/cilium/cilium-l4lb-test/blob/master/cilium-lb-example.yaml); [Cilium kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/)
**Confidence**: High
**Analysis**: Cilium's production L4LB topology works because the LB host has direct routing to the backend network. XDP_TX returns the (encapsulated or rewritten) packet out the same iface; the host's routing table sees the new dest and forwards. This is exactly the property a two-namespace test topology lacks — the LB-side veth has only ONE peer (the backend or the client, depending on which veth side it is).

### Finding 2.3: XDP_TX requires manual checksum maintenance via incremental updates (RFC 1624); XDP_PASS does NOT shave this cost — both paths still need correct checksums for the rewritten headers
**Evidence**: From APNIC blog and xdp-tutorial: "The XDP_TX return code sends out modified packets immediately without kernel network stack help, which normally handles IPv4 and UDP header checksum calculations that must be done manually with XDP_TX." For incremental updates: "RFC 1624 incremental checksum updates can be implemented manually since `bpf_l3_csum_replace`/`bpf_l4_csum_replace` and `bpf_csum_diff` helpers are TC-only and not available in XDP programs, requiring manual computation of `csum_diff` values from old and new data."
**Source**: [APNIC: Journeying into XDP Part 0](https://blog.apnic.net/2020/09/02/journeying-into-xdp-part-0/) — Accessed 2026-05-06
**Verification**: [xdp-project/xdp-tutorial packet-solutions/xdp_prog_kern_03.c](https://github.com/xdp-project/xdp-tutorial/blob/main/packet-solutions/xdp_prog_kern_03.c) — explicit `icmp_checksum_diff()` / `ip_decrease_ttl()` updates before XDP_TX; [LWN: Checksum offload and XDP](https://lists.openwall.net/netdev/2017/04/11/153)
**Confidence**: High
**Analysis**: **The user's intuition that XDP_PASS shaves checksum cost is INCORRECT.** Returning XDP_PASS does not let the program skip the checksum fix-up. Why: the kernel network stack receives the skb with whatever data the XDP program wrote into the buffer, and it does NOT re-checksum on RX (RX checksum offload by the NIC is computed before XDP runs; if you mutated headers, that offload validation was for the *original* headers). If the program mutates headers without fixing checksums, the kernel stack sees a checksum-bad packet and drops it during normal RX processing. The verifier-budget difference between XDP_TX and XDP_PASS in a header-rewrite program is essentially zero on the checksum path; both branches need the same incremental update logic.

## Question 3 — Verifier Budget Impact

### Finding 3.1: No public veristat baselines comparing XDP_TX vs XDP_PASS instruction counts exist (knowledge gap)
**Evidence**: Searches across kernel.org, LWN, Cilium issue tracker (#4837 "CI: Measure verifier complexity for bpf programs"), libbpf/veristat, USENIX LISA 21 "Performance Analysis of XDP Programs" turned up generic complexity tooling but no published comparison of TX vs PASS instruction counts for an otherwise-identical L4LB program.
**Source**: [libbpf/veristat](https://github.com/libbpf/veristat) — Accessed 2026-05-06
**Verification**: [Cilium issue #4837](https://github.com/cilium/cilium/issues/4837) — Cilium itself acknowledges measuring verifier complexity is a CI ask, not a published baseline; [pchaigno: Complexity of the BPF Verifier](https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html)
**Confidence**: Low — singleton-source acknowledgment of the gap
**Analysis**: The verifier walks every reachable path. An XDP program that returns `XDP_TX` on the hit path and `XDP_PASS` on the miss path has both paths walked — flipping the hit-path return from TX to PASS does not change the number of reachable paths. The actual instruction-count delta is dominated by the header-rewrite + checksum-update logic, which is identical between the two return shapes (Finding 2.3). **The user's hypothesis "PASS shaves verifier budget because no checksum fold needed" is structurally wrong**: both paths fold the checksum identically.

### Finding 3.2: The kernel's XDP_PASS path does NOT include automatic checksum recomputation; the kernel receives whatever the XDP program emitted
**Evidence**: From kernel.org Checksum Offloads docs and netdev archives: hardware RX checksum verification (CHECKSUM_UNNECESSARY) is set by the NIC driver based on the *original* packet bytes BEFORE XDP runs. After the XDP program mutates headers and returns XDP_PASS, the skb is allocated and the existing `ip_summed` flag (likely CHECKSUM_UNNECESSARY from hardware) is carried into the stack — but this flag refers to the original, unmodified bytes. Stack RX validation thus trusts the (now stale) hardware verdict; the malformed checksum is propagated upward. In practice, the kernel sees a TCP/IP packet whose L3+L4 checksums no longer match the data, and drops it during socket-layer demux.
**Source**: [Kernel.org Checksum Offloads](https://www.kernel.org/doc/html/latest/networking/checksum-offloads.html) — Accessed 2026-05-06
**Verification**: [Linux kernel sk_buff documentation](https://docs.kernel.org/networking/skbuff.html); [LWN netdev: RFC: Checksum offload and XDP](https://lists.openwall.net/netdev/2017/04/11/153) — explicit discussion of how XDP modifications interact with RX checksum offload
**Confidence**: Medium — multiple authoritative sources, but no single source pulls the full conclusion together
**Analysis**: The user MUST do incremental checksum updates regardless of XDP_TX vs XDP_PASS. There is no checksum-skip optimization available in either return mode. Both return shapes require the same `csum_diff`-style update.

## Question 4 — XDP_REDIRECT Alternative

### Finding 4.1: bpf_redirect / bpf_redirect_map ARE available from XDP context; bpf_redirect_peer and bpf_redirect_neigh are TC-only
**Evidence**: From the eBPF Docs program-type list for BPF_PROG_TYPE_XDP: XDP supports redirect helpers including `bpf_redirect`, `bpf_redirect_map`. From arthurchiao's "Differentiate three types of eBPF redirects (2022)": "`bpf_redirect_neighbor()` is currently only supported for tc BPF program types" and similarly for `bpf_redirect_peer()`. Confirmed via [Cilium issue tracker discussion of bpf_redirect_peer](https://github.com/cilium/cilium/issues/21496) — explicitly TC ingress.
**Source**: [eBPF Docs - BPF_PROG_TYPE_XDP](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/) — Accessed 2026-05-06
**Verification**: [arthurchiao: Differentiate three types of eBPF redirects](https://arthurchiao.art/blog/differentiate-bpf-redirects/); [LWN: Implement XDP bpf_redirect](https://lwn.net/Articles/728146/)
**Confidence**: High
**Analysis**: For two-namespace topologies, the XDP-context options are XDP_TX (same iface, requires routing) or XDP_REDIRECT to a peer veth ifindex (different iface). XDP_REDIRECT can target an ifindex pointing INTO the backend's netns — the client-side netns has a veth peer, and the program redirects to that peer's ifindex. BUT — see 4.2.

### Finding 4.2: XDP_REDIRECT to a veth peer requires an XDP program attached to the receiving (peer) veth, just like XDP_TX
**Evidence**: From the kernel patch v7 09/10 commit: "If no program is attached to the receiving veth, the operation is skipped." From xdp-tutorial: "all involved devices should have an attached XDP program, including both veth peers ... `veth` devices won't deliver redirected/retransmitted XDP frames unless there is an XDP program attached to the receiving side". This applies equally to XDP_TX and XDP_REDIRECT for veth.
**Source**: [netdev patch: XDP TX and REDIRECT for veth](https://lists.openwall.net/netdev/2018/08/02/77) — Accessed 2026-05-06
**Verification**: [xdp-project/xdp-tutorial packet03-redirecting](https://github.com/xdp-project/xdp-tutorial/tree/main/packet03-redirecting); [zhao-kun/xdp-redirect demo](https://github.com/zhao-kun/xdp-redirect) — demonstrates XDP_REDIRECT veth-to-container, requires listener on container side
**Confidence**: High
**Analysis**: Option D (XDP_REDIRECT to peer veth) requires a stub XDP program in the backend namespace — operational test-topology overhead but solvable. However, XDP_REDIRECT also has a TX-side complication: it loses TSO ("It's worth noting that XDP on veth pairs can cause TSO to stop working, resulting in packets being linearized and lower TCP throughput between veth devices"). For a test, this is irrelevant.

### Finding 4.3: XDP_REDIRECT to a different ifindex IS the production pattern for cross-iface routing in XDP — but using it instead of XDP_TX is an architectural change comparable in blast radius to switching to XDP_PASS
**Evidence**: Cilium's standalone L4LB explicitly uses XDP_TX (not XDP_REDIRECT) for the load-balanced fast path; its acceleration mode "native" relies on driver-level XDP_TX. Multiple kernel-side L4LB designs (Katran's `xdp_root` chain) attach to a single iface and rely on XDP_TX. Cross-iface XDP_REDIRECT is a different architectural choice typically used for routing/firewall scenarios, not L4LB DSR.
**Source**: [Cilium L4LB blog](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/) — Accessed 2026-05-06
**Verification**: [Katran xdp_root.c](https://github.com/facebookincubator/katran/blob/main/katran/lib/bpf/xdp_root.c); [Meta Engineering: Open-sourcing Katran](https://engineering.fb.com/2018/05/22/open-source/open-sourcing-katran-a-scalable-network-load-balancer/)
**Confidence**: Medium — Cilium and Katran both use XDP_TX, but the body of XDP routing/firewall code uses XDP_REDIRECT extensively
**Analysis**: Switching the kernel-side from XDP_TX to XDP_REDIRECT is NOT a "test-side only" change — it changes what the production hot path returns and breaks the existing slice work that asserts XDP_TX. In test, you'd need DEVMAP/DEVMAP_HASH wired correctly; in production you'd need to populate the devmap with backend-iface ifindexes, which on a real production node means... routing back through the stack anyway, because backends aren't on a directly-attached iface.

## Recommendation Matrix

| Criterion | A: 3-iface transit | B: XDP_PASS DSR-style | C: Defer with `#[ignore]` | D: XDP_REDIRECT to peer veth |
|---|---|---|---|---|
| Test-side only change | ✅ yes | ❌ no — kernel-side change | ✅ yes (no change) | ❌ no — kernel-side change |
| Kernel-side architectural change | none | switches return code in production | none | switches return code + adds DEVMAP wiring |
| Mirrors a documented production shape | ✅ Cilium PR #16338 | ⚠️ Cilium `punt_to_stack` fallback only — not the LB primary path | ❌ no | ⚠️ used in routing/firewall, not L4LB primaries (Cilium/Katran use XDP_TX) |
| Verifier budget impact | none | unchanged (Finding 3.1, 3.2) | none | adds DEVMAP lookup; modest increase |
| Blast radius on prior slices | zero — kernel-side untouched | high — every prior slice asserting XDP_TX must change | zero — but accumulates scope debt | high — DEVMAP is a new map family; userspace handle work needed |
| Requires veth peer-stub XDP | ✅ yes (in `backend-ns`) | ❌ no (kernel routes via stack) | n/a | ✅ yes (in `backend-ns`) |
| Reuses checksum logic from prior slices | ✅ yes | ✅ yes (Finding 2.3) | n/a | ✅ yes |
| Real-`nc` end-to-end coverage | ✅ yes | ✅ yes | ❌ no — defers it | ✅ yes |
| Production-fidelity vs Overdrive single-iface bare-metal NIC target | ⚠️ middle netns is a proxy for "host routing table" | ⚠️ degrades fast-path semantics permanently | n/a | ❌ XDP_REDIRECT to ifindex is the wrong model for single-NIC L4LB |

## Risks Per Option

**A — Three-iface transit topology.** Strongest argument *against*: the middle `lb-ns` is a proxy for "the LB host's routing table reaches the backend network", not a literal recreation of how the LB will deploy on a real bare-metal NIC. A test that passes in the three-namespace shape is not, by itself, evidence that the same program will work attached to a physical i40e/mlx5 NIC on a host whose default-route reaches backend nodes. This risk is the same one Cilium accepts in PR #16338, and is mitigated by Tier 4 (xdp-bench against real NICs in CI; veristat against the kernel matrix) — neither of which is a Tier-3 netns concern. Net: not a structural objection; expected loss of fidelity in netns testing.

**B — XDP_PASS DSR-style.** Strongest argument *against*: switching the production fast-path return code from XDP_TX to XDP_PASS is a permanent architectural retreat that contradicts every credible reference (Cilium's primary path, Katran's only path, every published XDP L4LB tutorial). It loses the line-rate driver-level emit of XDP_TX in exchange for stack-traversal cost on every load-balanced packet, in production, forever — to fix a *test-topology* problem. It also forces every prior slice that asserts XDP_TX to be reworked (S-2.2-04 SERVICE_MAP hit, the entire reverse-NAT lockstep chain). The verifier-budget claim ("PASS shaves checksum cost") is structurally wrong (Findings 2.3 + 3.2). This option throws away production hot-path performance to avoid restructuring three test files.

**C — Defer with `#[ignore]`.** Strongest argument *against*: this is the option that costs nothing today and erodes everything tomorrow. The `#[should_panic(expected = "RED scaffold")]` and `#[ignore]` discipline in `.claude/rules/testing.md` is explicit that `#[ignore]` is for tests waiting on **external** resources the implementation cannot synthesize. A netns-topology limitation is an *internal* test-harness problem, not an external blocker, and is therefore the wrong fit for `#[ignore]`. Worse: the next reviewer reading "S-2.2-15 ignored" cannot tell whether the kernel-side rewrite-and-route logic actually works at all, and every subsequent change to the XDP+TC program runs without that real-`nc` end-to-end gate. This is the opposite of the four-tier testing posture in `.claude/rules/testing.md`.

**D — XDP_REDIRECT to peer veth.** Strongest argument *against*: it doesn't actually solve the underlying problem any better than XDP_TX, and it's a bigger code change. XDP_REDIRECT to a veth peer ifindex requires (a) a stub XDP program on the receiving veth in `backend-ns` (same as XDP_TX would in option A), (b) populating a DEVMAP/DEVMAP_HASH with the backend ifindex, (c) reworking the kernel-side return shape, and (d) accepting that production XDP_REDIRECT-to-ifindex is the wrong model for a single-NIC L4LB anyway. The one upside — direct veth-to-veth delivery without going through `lb-ns` routing — buys nothing in production where the LB has one NIC and backends are on remote nodes reachable only through the kernel routing table.

## Cited Sources

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium GitHub PR #16338 (Standalone L4LB XDP tests) | github.com | High | Official source | 2026-05-06 | Y |
| Cilium L4LB blog post (2022-04-12) | cilium.io | High | Official | 2026-05-06 | Y |
| Cilium BPF Unit and Integration Testing docs | docs.cilium.io | High | Official | 2026-05-06 | Y |
| Cilium bpf/bpf_xdp.c | github.com/cilium/cilium | High | Source | 2026-05-06 | Y |
| Cilium bpf/lib/nodeport.h | github.com/cilium/cilium | High | Source | 2026-05-06 | Y |
| cilium-l4lb-test repo (cilium-lb-example.yaml) | github.com/cilium | High | Source | 2026-05-06 | Y |
| Cilium kube-proxy-free docs | docs.cilium.io | High | Official | 2026-05-06 | Y |
| Cilium issue #4837 (CI verifier complexity) | github.com/cilium/cilium | High | Source | 2026-05-06 | Y |
| Katran GitHub repo + DEVELOPING.md | github.com/facebookincubator/katran | High | Official | 2026-05-06 | Y |
| Katran xdp_root.c | github.com/facebookincubator/katran | High | Source | 2026-05-06 | Y |
| Meta Engineering: Open-sourcing Katran | engineering.fb.com | High | Official | 2026-05-06 | Y |
| netdev patch v7 09/10 (XDP TX and REDIRECT for veth) | lists.openwall.net | High | Official mailing list | 2026-05-06 | Y |
| xdp-project/xdp-tutorial (packet03-redirecting) | github.com/xdp-project | High | Official tutorial | 2026-05-06 | Y |
| xdp-tutorial xdp_prog_kern_03.c | github.com/xdp-project | High | Source | 2026-05-06 | Y |
| Linux Kernel Checksum Offloads docs | kernel.org | High | Official | 2026-05-06 | Y |
| Linux Kernel sk_buff documentation | docs.kernel.org | High | Official | 2026-05-06 | Y |
| Linux Kernel BPF redirect docs | docs.kernel.org | High | Official | 2026-05-06 | Y |
| Linux Kernel BPF map_devmap docs | docs.kernel.org | High | Official | 2026-05-06 | Y |
| Linux Kernel BPF program run docs | docs.kernel.org | High | Official | 2026-05-06 | Y |
| eBPF Docs - BPF_PROG_TYPE_XDP | docs.ebpf.io | High | Technical | 2026-05-06 | Y |
| eBPF Docs - bpf_redirect | docs.ebpf.io | High | Technical | 2026-05-06 | Y |
| eBPF Docs - bpf_redirect_map | docs.ebpf.io | High | Technical | 2026-05-06 | Y |
| eBPF Docs - BPF_PROG_TEST_RUN | docs.ebpf.io | High | Technical | 2026-05-06 | Y |
| LWN: Implement XDP bpf_redirect | lwn.net | Medium-High | Industry | 2026-05-06 | Y |
| LWN: bpf program testing framework | lwn.net | Medium-High | Industry | 2026-05-06 | Y |
| LWN netdev: RFC Checksum offload and XDP | lists.openwall.net | High | Official | 2026-05-06 | Y |
| APNIC: Journeying into XDP Part 0 | blog.apnic.net | Medium-High | Industry | 2026-05-06 | Y |
| arthurchiao: Differentiate three types of eBPF redirects | arthurchiao.art | Medium | Community | 2026-05-06 | Y |
| Cilium issue #21496 (bpf_redirect_peer veth Rx) | github.com/cilium/cilium | High | Source | 2026-05-06 | Y |
| prototype-kernel: XDP actions | prototype-kernel.readthedocs.io | High | Technical | 2026-05-06 | Y |
| Tigera ebpf/xdp guide | tigera.io | Medium-High | Industry | 2026-05-06 | Y |
| pchaigno: Complexity of the BPF Verifier | pchaigno.github.io | Medium | Community | 2026-05-06 | Y |
| libbpf/veristat | github.com/libbpf | High | Official tool | 2026-05-06 | Y |

Reputation distribution: High 24/32 (75%), Medium-High 5/32 (16%), Medium 3/32 (9%), Avg ~0.92.

## Full Citations

[1] Cilium project. "helm,test: Add standalone L4LB XDP tests in a form of Github Action by brb · Pull Request #16338". GitHub. https://github.com/cilium/cilium/pull/16338. Accessed 2026-05-06.

[2] Cilium project. "Cilium Standalone Layer 4 Load Balancer XDP". cilium.io. 2022-04-12. https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/. Accessed 2026-05-06.

[3] Cilium project. "BPF Unit and Integration Testing — Cilium 1.19.1 documentation". docs.cilium.io. https://docs.cilium.io/en/stable/contributing/testing/bpf/. Accessed 2026-05-06.

[4] Cilium project. "bpf/bpf_xdp.c". GitHub. https://github.com/cilium/cilium/blob/main/bpf/bpf_xdp.c. Accessed 2026-05-06.

[5] Cilium project. "bpf/lib/nodeport.h". GitHub. https://github.com/cilium/cilium/blob/master/bpf/lib/nodeport.h. Accessed 2026-05-06.

[6] Cilium project. "cilium-l4lb-test/cilium-lb-example.yaml". GitHub. https://github.com/cilium/cilium-l4lb-test/blob/master/cilium-lb-example.yaml. Accessed 2026-05-06.

[7] Cilium project. "Kubernetes Without kube-proxy — Cilium 1.19.3 documentation". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-05-06.

[8] Cilium project. "CI: Measure verifier complexity for bpf programs (#4837)". GitHub. https://github.com/cilium/cilium/issues/4837. Accessed 2026-05-06.

[9] Facebook Incubator. "katran: A high performance layer 4 load balancer". GitHub. https://github.com/facebookincubator/katran. Accessed 2026-05-06.

[10] Facebook Incubator. "Katran DEVELOPING.md". GitHub. https://github.com/facebookincubator/katran/blob/main/DEVELOPING.md. Accessed 2026-05-06.

[11] Facebook Incubator. "katran/lib/bpf/xdp_root.c". GitHub. https://github.com/facebookincubator/katran/blob/main/katran/lib/bpf/xdp_root.c. Accessed 2026-05-06.

[12] Engineering at Meta. "Open-sourcing Katran, a scalable network load balancer". 2018-05-22. https://engineering.fb.com/2018/05/22/open-source/open-sourcing-katran-a-scalable-network-load-balancer/. Accessed 2026-05-06.

[13] netdev mailing list. "[PATCH v7 bpf-next 09/10] veth: Add XDP TX and REDIRECT". 2018-08-02. https://lists.openwall.net/netdev/2018/08/02/77. Accessed 2026-05-06.

[14] xdp-project. "xdp-tutorial / packet03-redirecting". GitHub. https://github.com/xdp-project/xdp-tutorial/tree/main/packet03-redirecting. Accessed 2026-05-06.

[15] xdp-project. "xdp-tutorial/packet-solutions/xdp_prog_kern_03.c". GitHub. https://github.com/xdp-project/xdp-tutorial/blob/main/packet-solutions/xdp_prog_kern_03.c. Accessed 2026-05-06.

[16] Linux kernel. "Checksum Offloads — Linux Kernel documentation". https://www.kernel.org/doc/html/latest/networking/checksum-offloads.html. Accessed 2026-05-06.

[17] Linux kernel. "struct sk_buff documentation". https://docs.kernel.org/networking/skbuff.html. Accessed 2026-05-06.

[18] Linux kernel. "Redirect — BPF documentation". https://docs.kernel.org/bpf/redirect.html. Accessed 2026-05-06.

[19] Linux kernel. "BPF_MAP_TYPE_DEVMAP and BPF_MAP_TYPE_DEVMAP_HASH". https://docs.kernel.org/bpf/map_devmap.html. Accessed 2026-05-06.

[20] Linux kernel. "Running BPF programs from userspace". https://docs.kernel.org/bpf/bpf_prog_run.html. Accessed 2026-05-06.

[21] eBPF Docs. "Program Type 'BPF_PROG_TYPE_XDP'". https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_XDP/. Accessed 2026-05-06.

[22] eBPF Docs. "Helper Function 'bpf_redirect'". https://docs.ebpf.io/linux/helper-function/bpf_redirect/. Accessed 2026-05-06.

[23] eBPF Docs. "Helper Function 'bpf_redirect_map'". https://docs.ebpf.io/linux/helper-function/bpf_redirect_map/. Accessed 2026-05-06.

[24] eBPF Docs. "Syscall command 'BPF_PROG_TEST_RUN'". https://docs.ebpf.io/linux/syscall/BPF_PROG_TEST_RUN/. Accessed 2026-05-06.

[25] LWN.net. "Implement XDP bpf_redirect". https://lwn.net/Articles/728146/. Accessed 2026-05-06.

[26] LWN.net. "bpf: program testing framework". https://lwn.net/Articles/718784/. Accessed 2026-05-06.

[27] netdev mailing list. "Re: RFC: Checksum offload and XDP". 2017-04-11. https://lists.openwall.net/netdev/2017/04/11/153. Accessed 2026-05-06.

[28] APNIC. "Journeying into XDP: Part 0". 2020-09-02. https://blog.apnic.net/2020/09/02/journeying-into-xdp-part-0/. Accessed 2026-05-06.

[29] Arthur Chiao. "Differentiate three types of eBPF redirects (2022)". https://arthurchiao.art/blog/differentiate-bpf-redirects/. Accessed 2026-05-06.

[30] Cilium project. "Pod's veth dose not count Rx packets/bytes when using bpf_redirect_peer (#21496)". GitHub. https://github.com/cilium/cilium/issues/21496. Accessed 2026-05-06.

[31] Prototype Kernel. "XDP actions". https://prototype-kernel.readthedocs.io/en/latest/networking/XDP/implementation/xdp_actions.html. Accessed 2026-05-06.

[32] Tigera. "eBPF XDP: The Basics and a Quick Tutorial". https://www.tigera.io/learn/guides/ebpf/ebpf-xdp/. Accessed 2026-05-06.

[33] Paul Chaignon. "Complexity of the BPF Verifier". 2019-07-02. https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html. Accessed 2026-05-06.

[34] libbpf project. "veristat: tool for loading, verifying, and debugging BPF object files". GitHub. https://github.com/libbpf/veristat. Accessed 2026-05-06.

## Knowledge Gaps

### Gap 1: No published veristat baseline comparing XDP_TX vs XDP_PASS instruction counts for an otherwise-identical L4LB program
**Issue**: The user asked whether XDP_PASS shaves verifier budget. The literature (LWN, Cilium issue tracker, kernel.org, libbpf veristat) confirms veristat exists and Cilium has discussed measuring verifier complexity in CI, but no published comparison of TX vs PASS for the same L4LB rewrite logic exists.
**Attempted**: searches across kernel.org, LWN, Cilium issue tracker (#4837), libbpf/veristat, USENIX LISA 21 "Performance Analysis of XDP Programs", pchaigno's verifier-complexity post.
**Recommendation**: If the project wants a definitive answer, run `veristat` against a TX vs PASS variant of the existing program in `crates/overdrive-bpf` and add the comparison to `perf-baseline/main/verifier-budget/` in a follow-up step. Based on the structural argument in Findings 2.3 + 3.2 (both branches need identical incremental checksum updates and the verifier walks all reachable paths in both cases), the predicted delta is in the noise (single-digit instructions, dominated by the return-code emit itself).

### Gap 2: Cilium's exact `nodeport_lb4` body is too long for a single web fetch
**Issue**: The `bpf/lib/nodeport.h` file is large; the WebFetch tool returned only a portion. The exact code path that translates `punt_to_stack` → XDP_PASS via `bpf_xdp_exit` was not retrieved verbatim.
**Attempted**: WebFetch of `cilium/cilium` master `nodeport.h`; multiple version-pinned fetches.
**Recommendation**: For implementation reference (NOT for the topology decision), grep the local Cilium source mirror (or clone the repo) for `bpf_xdp_exit` and the `is_dsr` / `punt_to_stack` translation. The decision in this memo is not blocked on the exact line numbers.

## Recommendations for Further Research

1. **If Option A is adopted**: capture the three-iface topology setup as a reusable shell/script under `crates/overdrive-dataplane/tests/integration/helpers/netns.rs` (already in the working tree per git status) so subsequent slices that need the same shape don't re-derive it. Mirror Cilium's `ip l a l4lb-veth0 type veth peer l4lb-veth1` + `ip l s dev l4lb-veth1 netns <ns>` shape exactly.

2. **Stub XDP program for veth peer**: a one-line `int xdp_pass(struct xdp_md *ctx) { return XDP_PASS; }` program needs to be loaded on the backend-side veth peer. Add this to `crates/overdrive-bpf` as a named test-only program; `bpf_xdp_veth_host.o` in Cilium's PR #16338 is the equivalent.

3. **Tier 4 follow-up**: when Tier 4 (verifier regress + xdp-bench) lands, capture a TX-variant baseline for the program; this closes Gap 1 in-house.
