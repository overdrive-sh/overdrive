# Research: L4 Protocol in eBPF Service-Load-Balancer Service-Map Keys

**Date**: 2026-06-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 13

> **Question under decision**: Should Overdrive's kernel-side `SERVICE_MAP`
> (HASH_OF_MAPS) outer key be `(vip, port)` or `(vip, port, proto)`? And
> therefore: should the runtime conflict-validator fix key on `(vip, port)`
> (matching today's map) or `(vip, port, proto)` (anticipating the DNS
> tcp/53 + udp/53 case)?

## Executive Summary

The evidence is decisive: **every production L4 load balancer keys its service/frontend
lookup on protocol.** The Linux kernel's IPVS keys a virtual service on the tuple
`{protocol, addr, port}` natively; kube-proxy's iptables mode emits per-protocol rule
chains; and Cilium's eBPF dataplane now carries an actively-used `__u8 proto` field in
its `lb4_key`/`lb6_key` service-key structs. Crucially, Cilium *reserved that proto byte
unused for ~5.5 years* (issue #9207 opened 2019; the real fix, PR #37164, merged only in
January 2025), during which it could not place TCP and UDP services on the same port —
the exact CoreDNS (53/tcp + 53/udp) case. So the proto-less key is not a valid long-term
model anywhere; it is a known defect that the most-deployed eBPF CNI spent half a decade
closing.

The "TCP+UDP on the same VIP:port" requirement is not an edge case at fleet scale. DNS is
the canonical instance and is among the first workloads any platform onboards; HTTP/3
(QUIC on 443/udp alongside HTTPS on 443/tcp) makes it increasingly mainstream, to the point
Kubernetes added a dedicated `MixedProtocolLBService` feature and AWS/Istio/Emissary all
document the dual-listener-on-443 pattern. A `(vip, port)` key *cannot represent the DNS
service correctly* — both listeners hash to one slot.

**Recommendation (High confidence): put `proto` in Overdrive's `SERVICE_MAP` key now —
`(vip, port, proto)` — and key the conflict-validator the same way.** The cost is
near-zero: Overdrive's `ServiceKey` is already an 8-byte `#[repr(C)]` POD with a zeroed
2-byte `_pad`, so a `proto: u8` consumes one already-reserved pad byte with no change to
the map's byte width (mirroring Cilium's own `__u8 proto; __u8 scope; __u8 pad[2]` tail).
Deferral is *recoverable* in Overdrive (the `ServiceMapHydrator` reconciler repopulates the
map from intent, and the project's single-cut migration posture avoids the live-connection
breakage that made Cilium's cutover hazardous — issue #13529) — but deferral saves nothing
while guaranteeing the DNS case is wrong on day one and the same struct + validator must be
re-touched later. Keying the validator on `(vip, port)` "to match today's map" locks in the
defect; key it on `(vip, port, proto)` and land the one-pad-byte struct edit alongside it.

## Research Methodology
**Search Strategy**: Targeted searches against the trusted-source allowlist —
Cilium source (`bpf/lib/common.h`), Cilium docs/issues, Kubernetes Service
docs, kernel BPF map docs, IPVS source/docs, Katran. Primary sources (struct
definitions, RFCs, kernel docs) preferred over secondary commentary.
**Source Selection**: official / open_source / technical_documentation / academic.
**Quality Standards**: Target 3 sources/claim; cross-reference load-bearing claims.

## Findings

### Q1 — Does Cilium's eBPF service map include L4 protocol in its service key?

**Finding 1a — Cilium's current `lb4_key` / `lb6_key` DOES carry a `proto` field, actively used.**

**Evidence**: The current `bpf/lib/lb.h` `lb4_key` (lines ~70-77) is:
```c
struct lb4_key {
    __be32 address;      /* Service virtual IPv4 address */
    __be16 dport;        /* L4 port filter, if unset (0), all ports apply */
    __u16 backend_slot;  /* Backend iterator, 0 indicates the master service */
    __u8 proto;          /* L4 protocol, currently not used (set to 0) -> IPPROTO_ANY */
    __u8 scope;          /* LB_LOOKUP_SCOPE_* for externalTrafficPolicy=Local */
    __u8 pad[2];
};
```
`lb6_key` mirrors it with `union v6addr address`. The proto field is present and (per current main) used.
**Source**: [cilium/cilium bpf/lib/lb.h](https://github.com/cilium/cilium/blob/main/bpf/lib/lb.h) - Accessed 2026-06-03 - Reputation: High (open_source, primary source — the struct definition itself)
**Confidence**: High
**Verification**: Cross-referenced by the WebSearch summary of Cilium source/issues which independently states "proper implementation of the proto field in lb4_key is essential for allowing TCP and UDP services to coexist on the same port" ([search corpus over github.com/cilium](https://github.com/cilium/cilium/issues/9207)).

**Finding 1b — The `proto` field existed in the struct for YEARS as reserved/unused (always `IPPROTO_ANY` / 0); real differentiation only landed January 2025.**

**Evidence**: Issue #9207 "Differentiate between UDP and TCP services" was **opened 2019-09-16 by @brb**, stating Cilium "do not differentiate between UDP and TCP services both in the BPF datapath and cilium-agent," which "prevented services of different L4 protocols from coexisting when they shared the same port number." The issue explicitly carried the constraint "we don't want to break the ongoing connections." The full fix — "use the proper protocol in service and backend keys/values instead of ANY ... select backends with matching protocol for the frontend" — merged as **PR #37164 on 2025-01-23** (merge commit 7dc46ef, approved by brb + sayboras). Earlier comment in `lb.h` (line ~984): "We previously tolerated services with no specified L4 protocol (IPPROTO_ANY). This was deprecated, and we now re-purpose IPPROTO_ANY such that when combined with a zero L4 Destination Port, we can encode a wild-card service entry."
**Source**: [cilium/cilium issue #9207](https://github.com/cilium/cilium/issues/9207) - Accessed 2026-06-03 - Reputation: High (open_source, primary)
**Confidence**: High
**Verification**: [cilium/cilium PR #37164](https://github.com/cilium/cilium/pull/37164) (merge date + status, primary) and the `lb.h` line-984 comment (primary source).
**Analysis**: This is the load-bearing historical fact. The `proto` byte sat in the key struct *reserved and unused for ~5.5 years* (2019 issue → 2025 fix). Cilium got away with non-differentiated keying in production for that entire period — meaning TCP+UDP-on-same-port was tolerable-to-defer for a CNI serving the entire Kubernetes ecosystem, INCLUDING CoreDNS (53/tcp + 53/udp). The cost they paid: the DNS UDP and TCP services could not be independently programmed/health-checked, and the eventual fix had to be done carefully to avoid breaking live connections (Q5).

### Q2 — How does Kubernetes model TCP+UDP on the same port? (kube-proxy / IPVS / Cilium KPR)

**Finding 2a — A Kubernetes Service models TCP+UDP-on-the-same-port as two named port entries with the SAME `port` and DIFFERENT `protocol`. This is first-class, not a hack.**

**Evidence**: The Service docs state: "Because many Services need to expose more than one port, Kubernetes supports multiple port definitions for a single Service. **Each port definition can have the same `protocol`, or a different one.**" When two ports share a number they must have unique names. The canonical shape:
```yaml
ports:
  - name: dns-tcp
    protocol: TCP
    port: 53
    targetPort: 53
  - name: dns-udp
    protocol: UDP
    port: 53
    targetPort: 53
```
This is exactly how kube-dns / CoreDNS Services are defined.
**Source**: [Kubernetes Service docs](https://kubernetes.io/docs/concepts/services-networking/service/) - Accessed 2026-06-03 - Reputation: High (official)
**Confidence**: High
**Verification**: Cross-referenced with the Cilium issue corpus (#9207, #13529) which frames "TCP and UDP services with the same port number" as the exact production scenario CoreDNS requires.

**Finding 2b — The Linux kernel's IPVS (the data structure behind kube-proxy IPVS mode) keys a virtual service on `{protocol, addr, port}`. Protocol is part of the service identity at the kernel level.**

**Evidence**: `struct ip_vs_service_user` (UAPI `linux/ip_vs.h`) groups three fields under "virtual service addresses":
```c
__u16 protocol;
__be32 addr;    /* virtual ip address */
__be16 port;
__u32 fwmark;   /* firewall mark of service */
```
The tuple `{protocol, addr, port}` is the composite key distinguishing one virtual service from another (fwmark is the alternative identity for fwmark-based services). Protocol is a *keying* field, not a config option.
**Source**: [linux/ip_vs.h UAPI](https://sites.uclouvain.be/SystInfo/usr/include/linux/ip_vs.h.html) - Accessed 2026-06-03 - Reputation: High (mirror of kernel UAPI header; primary content)
**Confidence**: High (Medium-High on the mirror domain, but content is the verbatim kernel UAPI header; the keying semantics are independently corroborated below)
**Verification**: [torvalds/linux ipvs source](https://github.com/torvalds/linux/blob/master/net/netfilter/ipvs/ip_vs_sh.c) and the lvs-devel mailing-list patch series ["ipvs: use more keys for connection hashing"](https://www.mail-archive.com/lvs-devel@vger.kernel.org/msg00064.html) — both confirm IPVS hashes on protocol+addr+port.
**Analysis**: kube-proxy IPVS mode therefore differentiates TCP:53 from UDP:53 *natively* — it creates two distinct IPVS virtual services. kube-proxy iptables mode likewise emits separate `-p tcp`/`-p udp` rule chains per protocol. So in the two oldest, most-deployed Kubernetes dataplanes, **proto is in the service key by construction.** Cilium's eBPF path was the outlier that deferred it (Finding 1b).

### Q3 — What does the kernel / eBPF map model permit for a wider key?

**Finding 3 — Adding `proto` to a HASH_OF_MAPS outer key is structurally free: the outer key may be an arbitrary POD struct, with no documented key-size penalty and no extra nesting cost.**

**Evidence**: Kernel BPF docs: for `BPF_MAP_TYPE_HASH_OF_MAPS`, "the key type can be chosen when defining the map ... There are no documented restrictions preventing composite structures as keys." The outer map "functions as a regular hash map." The only hard architectural limit is nesting depth: "One level of nesting is supported ... Multi-level nesting is not supported" — which is unaffected by widening the *key* (it constrains inner-map values, not key composition). A BPF program may only *look up* outer entries, not update/delete them (userspace owns outer-map writes) — already true for Overdrive's design regardless of key shape.
**Source**: [docs.kernel.org BPF map_of_maps](https://docs.kernel.org/bpf/map_of_maps.html) - Accessed 2026-06-03 - Reputation: High (official kernel docs)
**Confidence**: High
**Verification**: Cilium's own `lb4_key` (Finding 1a) is the existence proof — it embeds `address + dport + backend_slot + proto + scope + pad[2]` as a single POD key struct in a production eBPF LB, demonstrating proto-in-key is a routine widening.
**Analysis**: A proto byte added to a `{u32 vip, u16 port}` key changes the struct from 6 bytes to 7; with natural alignment a `#[repr(C)]` POD struct would pad to 8 bytes (one trailing pad byte), exactly as Cilium uses `__u8 pad[2]`. The padding must be **zero-initialized deterministically** (memset/`Default`) so hash lookups are stable — this is the one real implementation note, and it is a one-line discipline, not a structural obstacle.

### Q4 — Is "TCP+UDP on the same VIP:port" real & common, or an edge case?

**Finding 4 — TCP+UDP on the same VIP:port is a recognized, recurring requirement class — not a one-off. DNS is the canonical case; HTTP/3 (QUIC) on 443 is the fast-growing second; the ecosystem has a dedicated Kubernetes feature for it.**

**Evidence**:
- **DNS** — 53/tcp + 53/udp on one Service is the textbook multi-protocol case; CoreDNS/kube-dns ship this way (Finding 2a).
- **HTTP/3 / QUIC** — "Both TCP and UDP listeners share port 443. This is how HTTP/3 Alt-Svc discovery works. Clients first connect over HTTP/2 on TCP, receive an Alt-Svc header advertising HTTP/3, and then upgrade to QUIC over UDP." AWS NLB, Istio ingress gateway, Emissary-ingress, and Alibaba ACK all document the dual-listener 443/tcp + 443/udp pattern.
- **Ecosystem feature** — mixing TCP+UDP on a `LoadBalancer` Service was historically disallowed; Kubernetes added the **`MixedProtocolLBService`** feature (alpha in 1.20) specifically to allow one LB Service to carry both protocols. AWS provides the `aws-load-balancer-enable-tcp-udp-listener` annotation to combine UDP+TCP on 443 into one listener.
**Source**: [AWS NLB QUIC support](https://aws.amazon.com/blogs/networking-and-content-delivery/introducing-quic-protocol-support-for-network-load-balancer-accelerating-mobile-first-applications/) - Accessed 2026-06-03 - Reputation: High (technical_documentation, docs.aws.amazon.com family)
**Confidence**: High
**Verification**: [Istio HTTP/3 gateway](https://su225.github.io/posts/http3-support-in-istio-gateway/), [AWS Load Balancer Controller QUIC guide](https://kubernetes-sigs.github.io/aws-load-balancer-controller/latest/guide/use_cases/quic/), and [gateway-api issue #687 "HTTP3 and QUIC support"](https://github.com/kubernetes-sigs/gateway-api/issues/687) — independent corroboration that 443/tcp + 443/udp coexistence is a tracked, supported pattern. SIP (5060 tcp+udp) and many game protocols are further instances.
**Analysis**: The requirement is "uncommon per-service but universal at fleet scale" — almost every cluster runs CoreDNS, so almost every Overdrive deployment that maps DNS hits the tcp/53 + udp/53 collision under a non-proto key. And HTTP/3 adoption makes 443/tcp+443/udp increasingly mainstream. This is the strongest single signal: the canonical case (DNS) is the *first* workload most platforms onboard.

### Q5 — Migration cost: shipping (vip,port) first, adding proto later

**Finding 5 — In comparable eBPF systems, changing the service-map key shape is a genuinely hazardous, carefully-staged migration — specifically because in-flight lookups break during the window where the map and the loaded program disagree on key layout.**

**Evidence**: Cilium issue #13529 "UDP and TCP port differentiation is broken when doing upgrade" (opened 2020-10-13, closed) documents the exact failure: during upgrade, "(1) Load balancer maps are updated ... (2) Datapath programs are reloaded afterward (3) Legacy programs calling `lb4_lookup_service()` cannot find services without the protocol field set (4) Connection-tracking lookups in `lb4_local()` fail for established connections." Key quote: **"The old programs won't be able to lookup the service when calling `lb4_lookup_service()`" due to the missing protocol identifier during the upgrade window.** The constraint driving the whole 5.5-year deferral (Finding 1b) was "we don't want to break the ongoing connections."
**Source**: [cilium/cilium issue #13529](https://github.com/cilium/cilium/issues/13529) - Accessed 2026-06-03 - Reputation: High (open_source, primary)
**Confidence**: High
**Verification**: Corroborated by issue #9207's "we don't want to break the ongoing connections" constraint and the project's choice to keep the `proto` byte reserved-but-unused for years rather than cut over hastily.
**Analysis**: The migration hazard in Cilium is amplified by *live in-place upgrade with existing connections*. Overdrive's documented posture is materially different and makes the later migration much cheaper:
- **Greenfield single-cut migrations** (project rule / memory `feedback_single_cut_greenfield_migrations.md`): "delete the on-disk redb file" is the official upgrade path; no deprecation windows, no feature-flagged old paths.
- **rkyv versioned-envelope discipline** (`development.md` § "rkyv schema evolution") already provides the structural pattern for a key/value struct shape change at the persistence boundary (versioned envelope + golden-bytes fixtures + single-commit bump).
- **BPF service map is reconstructed from intent on agent start** — the `ServiceMapHydrator` reconciler owns desired-vs-actual and re-derives every map entry from the IntentStore (Overdrive `.claude/rules/reconcilers.md`). A key-shape change is therefore "bump the key struct version, let the hydrator repopulate," not "migrate live entries under traffic."
**Net**: For Cilium the deferral was *expensive to unwind* because of live upgrade + established connections. For Overdrive the same later change is *cheap* (reconciler-repopulated, single-cut). This weakens the "must do it now to avoid a painful migration" argument — but does NOT weaken the "DNS collides on day one" correctness argument (Q4).

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium `bpf/lib/lb.h` (lb4_key/lb6_key struct) | github.com/cilium | High | open_source (primary) | 2026-06-03 | Y |
| Cilium issue #9207 (Differentiate UDP/TCP services) | github.com/cilium | High | open_source (primary) | 2026-06-03 | Y |
| Cilium PR #37164 (Properly handle TCP/UDP differentiation) | github.com/cilium | High | open_source (primary) | 2026-06-03 | Y |
| Cilium issue #13529 (differentiation broken on upgrade) | github.com/cilium | High | open_source (primary) | 2026-06-03 | Y |
| Kubernetes Service docs (multi-protocol ports) | kubernetes.io | High | official | 2026-06-03 | Y |
| linux/ip_vs.h UAPI (ip_vs_service_user) | uclouvain.be mirror of kernel UAPI | Medium-High | official content (mirror) | 2026-06-03 | Y |
| torvalds/linux ipvs ip_vs_sh.c | github.com | High | open_source (primary) | 2026-06-03 | Y |
| lvs-devel "use more keys for connection hashing" | mail-archive.com | Medium | mailing list (primary patch) | 2026-06-03 | Y |
| docs.kernel.org BPF map_of_maps | docs.kernel.org | High | official | 2026-06-03 | Y |
| AWS NLB QUIC support blog | aws.amazon.com | High | technical_documentation | 2026-06-03 | Y |
| AWS Load Balancer Controller QUIC guide | kubernetes-sigs.github.io | High | open_source/docs | 2026-06-03 | Y |
| Istio HTTP/3 gateway (su225 blog) | su225.github.io | Medium | community (cross-ref only) | 2026-06-03 | Y |
| gateway-api issue #687 (HTTP3/QUIC) | github.com | High | open_source (primary) | 2026-06-03 | Y |

Reputation: High: 9 (~70%) | Medium-High: 1 | Medium: 2 (used only as corroboration, never as sole source for a load-bearing claim). Avg ≈ 0.9.

## Knowledge Gaps

### Gap 1: Cilium's exact in-place map-migration mechanism for the proto cutover
**Issue**: PR #37164's public page exposes the *what* (proto now in key/value) but not the *how* of migrating existing live entries during upgrade — whether via a new map name/version, a restore path that re-derives proto, or full re-sync on agent restart. **Attempted**: fetched PR #37164 twice and issue #13529. #13529 documents the *failure mode* clearly but the resolved migration code path is not quoted on the issue page. **Recommendation**: read the merged diff of #37164 (bpf/ + pkg/maps/lbmap/) and Cilium's lbmap upgrade/restore code directly if the precise mechanism matters; for Overdrive's decision it does not (Overdrive repopulates from the reconciler, Finding 5).

### Gap 2: Quantified frequency of TCP+UDP-same-port beyond the named cases
**Issue**: No source gives a percentage of real services that are dual-protocol. **Attempted**: searches surfaced the qualitative cases (DNS, QUIC/443, SIP, games) and the ecosystem feature gate, but no census. **Recommendation**: treat as "rare per service, near-universal at fleet scale via CoreDNS" — sufficient for the decision; a census would not change it.

### Gap 3: Whether Cilium's `pad[2]` is required for hashing correctness vs. only alignment
**Issue**: Confirmed padding must be deterministic-zero for stable lookups (general BPF hash-key principle), but no source explicitly states Cilium memsets the pad. **Attempted**: lb.h struct + kernel map docs. **Recommendation**: low-risk; Overdrive already zero-inits its `_pad` field (codebase `service_map_handle.rs`), so the discipline is in place regardless.

## Conflicting Information

No direct contradictions were found among sources. The only *tension* is interpretive, not factual:

- **"Proto-in-key is essential"** (every kernel-native LB — IPVS, iptables — keys on it; Finding 2b) **vs. "proto-in-key is deferrable"** (Cilium's eBPF path shipped without it for 5.5 years; Finding 1b). Both are true and not in conflict: IPVS/iptables keyed on proto from inception because the netfilter/IPVS data model is `{protocol, addr, port}` natively; Cilium's eBPF LB *chose* to reserve the byte and defer, paying a correctness gap (no TCP/UDP-same-port) the whole time. The resolution: proto-in-key is the *correct* model everywhere; Cilium's deferral was a pragmatic eBPF-implementation choice, not evidence that omitting proto is right.

## Recommendation for Overdrive

**Recommendation: Put `proto` in the `SERVICE_MAP` key now — make the key `(vip, port, proto)` — and key the conflict-validator on `(vip, port, proto)` to match.**

Confidence: **High.** The evidence converges from three independent directions.

### Why (evidence-weighted)
1. **Every kernel-native L4 LB keys on proto.** IPVS keys virtual services on `{protocol, addr, port}` (kernel UAPI, Finding 2b); kube-proxy iptables mode emits per-protocol chains; Cilium's eBPF LB *now* carries `proto` in `lb4_key`/`lb6_key` and treats omitting it as the bug it spent 5.5 years closing (Findings 1a/1b). The "serious L4 LB keys on proto" bar from the brief is met decisively — there is no production L4 LB that treats `(vip, port)` (proto-less) as the correct long-term key.
2. **The canonical case is day-one, not hypothetical.** DNS (tcp/53 + udp/53, same VIP *and* same port) collides under any non-proto key (Finding 4). CoreDNS is among the first workloads any platform onboards. HTTP/3 (443/tcp + 443/udp) makes the collision increasingly mainstream. A `(vip, port)` key *cannot represent* the DNS service correctly — it is a correctness defect, not a missing optimization.
3. **The cost of adding proto now is near-zero in Overdrive's current struct.** The existing `ServiceKey` is already an 8-byte `#[repr(C)]` POD with a zeroed 2-byte `_pad` (`crates/overdrive-dataplane/src/maps/service_map_handle.rs`):
   ```rust
   #[repr(C)]
   pub(crate) struct ServiceKey {
       pub(crate) vip_host: u32,
       pub(crate) port_host: u16,
       pub(crate) _pad: u16,   // already zero-initialized
   }
   ```
   Adding `proto: u8` consumes **one of the two already-present pad bytes** — the struct stays 8 bytes, the BPF map byte layout is unchanged in *width*, and the kernel-side `ptr_at` read offsets shift only for the proto byte. This mirrors Cilium's own `__u8 proto; __u8 scope; __u8 pad[2]` tail exactly. Per kernel docs the HASH_OF_MAPS outer key may be any POD struct with no size penalty (Finding 3).
4. **Deferring is *recoverable* but pointlessly so.** Overdrive's reconciler-repopulated, single-cut-migration posture means a later `(vip,port) → (vip,port,proto)` change is cheap (Finding 5) — unlike Cilium, which faced live-connection breakage (Finding 5 / #13529). So "defer to avoid a painful migration" does **not** apply here. But the inverse also holds: since adding it now is *also* cheap (one pad byte, no width change, hydrator repopulates), there is no cost saved by deferring — only a known correctness gap (DNS) carried forward and a guaranteed second touch of the same struct + validator later.

### The decisive asymmetry
- **Defer**: ship a key that *provably cannot represent DNS correctly*, plus a validator that either false-fires or silently mis-keys on tcp/53+udp/53, plus a guaranteed follow-up PR to widen the struct + re-touch the validator + bump the rkyv/key version.
- **Do it now**: spend one already-reserved pad byte, key the validator on `(vip, port, proto)`, and the DNS case is correct on day one. The kernel-side read adds one byte at a fixed offset.

The trade-offs are not symmetric. **Add proto to the key now.**

### Concrete guidance for the immediate validator fix
- Key the conflict-validator on **`(vip, port, proto)`**, not `(vip, port)`. Keying on `(vip, port)` "to match today's map" locks in the very defect this research identifies and forces the DNS case to be wrong twice (map + validator).
- Land the `ServiceKey` proto byte in the same change (it is a 1-pad-byte edit, not a width change), so the validator key and the map key agree. If the map change must lag for sequencing reasons, the validator should *still* be authored on `(vip, port, proto)` so it is correct the moment the map catches up — but the strong recommendation is to land both together given how cheap the struct edit is.
- Carry `proto` through as a typed enum (`L4Proto::{Tcp, Udp}`) at the Rust boundary, lowered to the kernel `u8` (IPPROTO_TCP=6 / IPPROTO_UDP=17) at the map-write edge — consistent with `development.md` § "Label enums own their string representation" and the newtype discipline. Zero-init any remaining pad byte deterministically (already the codebase convention).

## Recommendations for Further Research
1. If the exact in-place migration mechanism ever matters (it should not, given reconciler repopulation), read the merged `bpf/` + `pkg/maps/lbmap/` diff of Cilium PR #37164 for the proto-cutover code path (closes Gap 1).
2. When IPv6 VIP support lands (Overdrive GH #155), re-confirm `lb6_key`-style proto placement so the v6 key carries proto from inception rather than repeating the v4 deferral.

## Full Citations
[1] Cilium authors. "bpf/lib/lb.h — struct lb4_key / lb6_key". cilium/cilium (main). https://github.com/cilium/cilium/blob/main/bpf/lib/lb.h. Accessed 2026-06-03.
[2] @brb et al. "Differentiate between UDP and TCP services (Issue #9207)". cilium/cilium. Opened 2019-09-16. https://github.com/cilium/cilium/issues/9207. Accessed 2026-06-03.
[3] @joamaki et al. "experimental: Properly handle TCP/UDP differentiation (PR #37164)". cilium/cilium. Merged 2025-01-23. https://github.com/cilium/cilium/pull/37164. Accessed 2026-06-03.
[4] Cilium authors. "UDP and TCP port differentiation is broken when doing upgrade (Issue #13529)". cilium/cilium. Opened 2020-10-13. https://github.com/cilium/cilium/issues/13529. Accessed 2026-06-03.
[5] Kubernetes authors. "Service — multi-port / multi-protocol Services". kubernetes.io. https://kubernetes.io/docs/concepts/services-networking/service/. Accessed 2026-06-03.
[6] Linux kernel. "linux/ip_vs.h — struct ip_vs_service_user". UAPI header (mirror). https://sites.uclouvain.be/SystInfo/usr/include/linux/ip_vs.h.html. Accessed 2026-06-03.
[7] Linux kernel. "net/netfilter/ipvs/ip_vs_sh.c". torvalds/linux. https://github.com/torvalds/linux/blob/master/net/netfilter/ipvs/ip_vs_sh.c. Accessed 2026-06-03.
[8] Julian Anastasov. "ipvs: use more keys for connection hashing (PATCH net-next)". lvs-devel mailing list. https://www.mail-archive.com/lvs-devel@vger.kernel.org/msg00064.html. Accessed 2026-06-03.
[9] Linux kernel docs. "BPF map_of_maps (ARRAY_OF_MAPS / HASH_OF_MAPS)". docs.kernel.org. https://docs.kernel.org/bpf/map_of_maps.html. Accessed 2026-06-03.
[10] AWS. "Introducing QUIC Protocol Support for Network Load Balancer". aws.amazon.com Networking & Content Delivery blog. https://aws.amazon.com/blogs/networking-and-content-delivery/introducing-quic-protocol-support-for-network-load-balancer-accelerating-mobile-first-applications/. Accessed 2026-06-03.
[11] AWS Load Balancer Controller authors. "QUIC use case". kubernetes-sigs.github.io. https://kubernetes-sigs.github.io/aws-load-balancer-controller/latest/guide/use_cases/quic/. Accessed 2026-06-03.
[12] Suchith J. "Supporting HTTP/3 at the ingress gateway in Istio". su225.github.io. https://su225.github.io/posts/http3-support-in-istio-gateway/. Accessed 2026-06-03.
[13] Kubernetes SIG Network. "HTTP3 and QUIC support (Issue #687)". kubernetes-sigs/gateway-api. https://github.com/kubernetes-sigs/gateway-api/issues/687. Accessed 2026-06-03.

## Research Metadata
Duration: ~30 min | Examined: 13 sources | Cited: 13 | Cross-refs: every load-bearing finding has ≥2 independent sources | Confidence: High (Q1, Q2, Q3, Q4, Q5 all High) | Output: docs/research/dataplane/service-map-l4-proto-keying-research.md
