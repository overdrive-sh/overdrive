# Research: DNS Positive TTL vs. Active Cache Invalidation for the Dial-by-Name Responder (ADR-0072)

**Date**: 2026-06-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16 cited

## Executive Summary

The user asks whether Overdrive's dial-by-name responder can actively "de-cache" a name when its backend allocation cycles, instead of relying on a short positive A-record TTL (`A_RECORD_TTL_SECS = 1`). The short answer is **no — not across the standard client boundary** — but the deeper finding is that **the question is largely moot on the path that matters**, and there is a strictly better architectural shape available.

On invalidation: standard DNS gives an authoritative server exactly one lever over a downstream cache — the TTL countdown (RFC 1034/1035). There is no purge/push primitive that reaches a glibc client. DNS NOTIFY (RFC 1996) is authoritative-to-authoritative zone-transfer signalling, not client-cache invalidation. DNS Push Notifications (RFC 8765 over DNS Stateful Operations, RFC 8490) is the only standard that models server→client change push, but it requires a long-lived TLS/TCP DSO *subscription* that bare glibc `getaddrinfo` does not speak and that stub resolvers do not implement. So "de-cache on alloc cycle" is not expressible toward the workload's resolver.

On what actually caches: **bare glibc has no DNS cache** — each `getaddrinfo`/`getent` re-resolves via NSS, so the responder is hit every time and always returns current truth; there is nothing to invalidate and the positive TTL is inert. If nscd is present it caches `hosts` and *honors the DNS-returned TTL*; if systemd-resolved is on the path (it is present in the appliance image) it caches and honors TTL but **does not cache TTL=0 at all** ("Not caching zero TTL cache entry" in `resolved-dns-cache.c`), with no minimum clamp inflating a small TTL and a 2 h maximum clamp that is irrelevant here. Hence TTL=0 and TTL=1 *are* meaningfully different on the resolvers in Overdrive's path: TTL=0 means re-resolve every time; TTL=1 leaves a ~1 s stale-addr window on any caching intermediary.

**Recommendation**: The most robust shape is to stop answering a volatile per-instance backend addr and instead answer a **stable canonical / per-workload address**, letting the `MtlsResolve` eBPF intercept select the live backend at connect time — the convergent industry pattern (Kubernetes ClusterIP + kube-proxy, Cilium connect-time socket-level eBPF LB, CoreDNS's relaxed 5 s stable-A TTL, Istio/Linkerd sidecar endpoint selection). That makes positive-TTL staleness moot by construction and defends against application-level caches Overdrive cannot control. **That is an ADR-0072 answer-contract change (architect decision — flagged, not decided).** Within the *current* contract, **TTL=0 is the least-surprising value** (RFC-1035 "re-resolve every time", honored exactly by systemd-resolved and nscd, never worse than TTL=1, strictly better wherever a cache exists). Keeping TTL=1 is defensible only as superstition about TTL=0-hostile resolvers, for which there is no evidence in Overdrive's path.

## Research Methodology

**Search Strategy**: IETF RFCs (rfc-editor.org / datatracker.ietf.org), Linux man-pages (man7.org), glibc docs (sourceware.org), systemd-resolved docs (freedesktop.org / systemd.io), Kubernetes/Cilium/CoreDNS/Istio/Linkerd docs, lwn.net.
**Source Selection**: Types: official (RFC, kernel, man-pages), open_source (systemd, glibc, Cilium, CoreDNS, Istio, Linkerd, k8s) | Reputation: high min | Verification: cross-reference each major claim against >=2-3 independent sources, prefer an RFC + an implementation doc per claim.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced.

## The Four Sub-Questions (verdicts up front)

| # | Sub-question | Verdict |
|---|--------------|---------|
| 1 | Server-initiated cache invalidation across the standard client boundary | **No.** TTL countdown is the only standard lever. NOTIFY = authoritative-to-authoritative zone signalling. DNS Push (RFC 8765/8490) models server→client push but needs a long-lived TLS/DSO subscription that bare glibc does not speak. "De-cache on alloc cycle" is not expressible toward the glibc client. |
| 2 | What actually caches in the glibc `getaddrinfo` path | **Bare glibc: nothing** — every call re-resolves; responder is the live source of truth, TTL moot. **nscd**: caches `hosts`, honors the DNS TTL. **systemd-resolved** (in the appliance image): caches, honors TTL, but does NOT cache TTL=0. App-level caches: out of our control. |
| 3 | TTL=0 vs TTL=1 semantics across the relevant resolvers | **Meaningfully different.** systemd-resolved never caches TTL=0 ("Not caching zero TTL cache entry"); TTL=1 is cached ~1 s. nscd honors the DNS TTL identically. No minimum-clamp inflates a small TTL. **TTL=0 expresses "re-resolve every time" exactly; TTL=1 leaves a ~1 s stale window.** |
| 4 | Stable-canonical-address alternative (DNS VIP + dataplane endpoint selection) | **Industry-standard and the more robust shape.** K8s ClusterIP + kube-proxy, Cilium connect-time socket eBPF LB, CoreDNS (stable A, relaxed 5 s TTL), Istio/Linkerd all return a STABLE address and let the dataplane pick the live backend. Overdrive's `MtlsResolve` intercept is the direct analogue. Making the A answer stable makes positive-TTL staleness moot by construction. **(Answer-contract change → architect decision; flagged below.)** |

## Findings

### Sub-question 1 — Server-initiated cache invalidation across the standard client boundary

#### Finding 1.1: The DNS caching model is TTL-only; TTL is the *only* standard lever an authoritative server has over downstream cache lifetime
**Evidence**: "The meaning of the TTL field is a time limit on how long an RR can be kept in a cache." ... "Since resolvers are responsible for discarding old RRs whose TTL has expired, most implementations convert the interval specified in arriving RRs to some sort of absolute time when the RR is stored in the cache." RFC 1035 §3.2.1 defines TTL as "a 32 bit signed integer that specifies the time interval that the resource record may be cached before the source of the information should again be consulted."
**Source**: [RFC 1034 — Domain Names: Concepts and Facilities](https://www.rfc-editor.org/rfc/rfc1034) — Accessed 2026-06-25
**Confidence**: High
**Verification**: [RFC 1035 §3.2.1 / §4.1.3](https://www.rfc-editor.org/rfc/rfc1035) (TTL field definition); RFC 1034's stated design priority "Access to information is more critical than instantaneous updates or guarantees of consistency."
**Analysis**: The original DNS design deliberately chose *eventual* consistency via passive TTL expiry over *active* invalidation. The authoritative server sets a number; the downstream cache counts down from it. There is no standard "drop this now" callback. This is the foundational constraint behind the entire question.

#### Finding 1.2: There is NO standard mechanism for an authoritative server to actively revoke/purge a cached record before its TTL expires
**Evidence**: "RFC 1034 contains no provision for an authoritative server to actively revoke or invalidate cached records before TTL expiration. The standard relies entirely on passive TTL-based expiration."
**Source**: [RFC 1034](https://www.rfc-editor.org/rfc/rfc1034) — Accessed 2026-06-25
**Confidence**: High
**Verification**: RFC 1996 (NOTIFY — does not target client caches, see 1.3); RFC 8765 (Push — not honored by stub resolvers, see 1.4). The absence is confirmed by the existence of these two later, narrowly-scoped proposals that try to address asynchronous change in *specific* contexts and still do not provide a general client-cache purge.
**Analysis**: "De-cache a name on alloc cycle" is not expressible in standard DNS toward an arbitrary downstream resolver/client. The server can only make the TTL small.

#### Finding 1.3: DNS NOTIFY (RFC 1996) is authoritative-to-authoritative zone-change signalling — it is NOT client cache invalidation
**Evidence**: NOTIFY allows "a master server [to advise] a set of slave servers that the master's data has been changed and that a query should be initiated to discover the new data." It "operates exclusively between authoritative nameservers" and "has no relevance to recursive resolvers or end-client DNS caching."
**Source**: [RFC 1996 — A Mechanism for Prompt Notification of Zone Changes (DNS NOTIFY)](https://www.rfc-editor.org/rfc/rfc1996) — Accessed 2026-06-25
**Confidence**: High
**Verification**: The Notify Set defaults to "all servers named in the NS RRset" (i.e. secondary authoritative servers), and the mechanism triggers AXFR/IXFR zone transfers — concepts that exist only between authoritative servers, never toward a stub resolver or `getaddrinfo` client.
**Analysis**: NOTIFY is a common red herring when reasoning about "pushing" DNS changes. It speeds up *zone replication* between a primary and its secondaries; it does nothing to a workload's local resolver cache. Not applicable to ADR-0072's client boundary.

#### Finding 1.4: DNS Push Notifications (RFC 8765, over DNS Stateful Operations RFC 8490) DO model server→client change push — but require a long-lived TLS/TCP session and are NOT supported by stub resolvers or the glibc `getaddrinfo` path
**Evidence**: RFC 8765 abstract: "there exists no mechanism for a client to be asynchronously notified when these changes occur." It defines change notifications using TTL sentinels — additions (TTL 0–2,147,483,647), removals (TTL 0xFFFFFFFF), bulk removals (TTL 0xFFFFFFFE). It "mandates long-lived TCP/TLS connections" and states "DNS Push Notification clients MUST use DNS Stateful Operations running over TLS over TCP." Intended clients are recursive resolvers performing subscriptions on the client's behalf and specialized DNS-Based Service Discovery applications — "Not standard stub resolvers via getaddrinfo." "Ordinary glibc getaddrinfo implementations do not support DNS Push — it requires specialized client software."
**Source**: [RFC 8765 — DNS Push Notifications](https://www.rfc-editor.org/rfc/rfc8765) — Accessed 2026-06-25
**Confidence**: High (RFC text is authoritative; "not widely deployed in stub resolvers" cross-referenced below in Source Analysis)
**Verification**: RFC 8765 depends on RFC 8490 (DNS Stateful Operations), itself a Proposed Standard; the subscription/keepalive session model (`DSO Keepalive`) is fundamentally incompatible with the connectionless, per-query `res_query`/`getaddrinfo` UDP path glibc uses by default.
**Analysis**: Push notifications *conceptually* give a server the "de-cache / here-is-new-data" primitive the user is asking for — but **only across a stateful subscription the client must opt into**, and the client here is bare glibc behind an injected `resolv.conf`, which has no DNS Push client. So even the one standard that models push does not reach Overdrive's client boundary.

#### Sub-question 1 VERDICT
**"De-cache a name on alloc cycle" is NOT achievable across the standard client boundary that ADR-0072 targets.** Standard DNS offers exactly one lever to an authoritative server over a downstream cache: the TTL countdown (RFC 1034/1035). NOTIFY (RFC 1996) is authoritative-to-authoritative zone signalling, irrelevant to client caches. DNS Push (RFC 8765/8490) is the only standard that models server→client change push, but it requires a long-lived TLS/TCP DSO subscription that bare glibc `getaddrinfo` does not speak and is not deployed in stub resolvers. There is no purge/invalidate primitive that reaches a workload's local resolver. The only server-side controls are: (a) a short/zero positive TTL, or (b) re-shaping the answer so churn never invalidates it (sub-question 4).

---

### Sub-question 2 — What actually caches in the glibc `getaddrinfo` path

#### Finding 2.1: Bare glibc has NO built-in DNS/`getaddrinfo` cache — each call re-resolves via NSS
**Evidence**: The `getaddrinfo(3)` man page "contains no discussion of caching behavior, cache expiration, or cache management... The documentation offers no evidence that getaddrinfo implements internal caching of DNS results." Cross-reference: "The glibc resolver does not cache queries... Since glibc itself doesn't have built-in DNS caching, nscd ... is a system daemon that caches Name Service Switch (NSS) lookups." The biriukov.dev walkthrough confirms `getaddrinfo()` per-call (1) attempts the nscd socket — which "usually doesn't exist by default", (2) reads `/etc/nsswitch.conf`, (3) queries NSS modules in order, (4) sorts per RFC 6724; caching "is not inherent to glibc itself but rather provided by external services."
**Source**: [getaddrinfo(3) — Linux man-pages](https://man7.org/linux/man-pages/man3/getaddrinfo.3.html) — Accessed 2026-06-25
**Confidence**: High
**Verification**: [biriukov.dev — getaddrinfo() from glibc](https://biriukov.dev/docs/resolver-dual-stack-application/4-getaddrinfo-from-glibc/) (Accessed 2026-06-25); web-survey cross-reference citing jvns.ca and jameshfisher.com ("glibc resolver does not cache queries").
**Analysis**: This is the load-bearing fact for ADR-0072. With no caching layer between the workload app and the responder, **every `getaddrinfo`/`getent` call is a fresh query to the responder**, which always returns current truth from the `service_backends` rows. There is no client-side cache holding a stale addr to invalidate, so the positive TTL is largely inert *for bare glibc*.

#### Finding 2.2: nscd caches the `hosts` database and HONORS the DNS-returned TTL (ignores its own `positive-time-to-live`)
**Evidence**: nscd "provides caching for accesses of the passwd, group, hosts, services and netgroup databases." Critically, `nscd.conf(5)` states: "Note that for some name services (including specifically DNS) the TTL returned from the name service is used and this attribute [positive-time-to-live] is ignored." — appearing twice in the manual.
**Source**: [nscd.conf(5) — Linux man-pages](https://man7.org/linux/man-pages/man5/nscd.conf.5.html) — Accessed 2026-06-25
**Confidence**: High
**Verification**: [nscd(8) — Linux man-pages](https://man7.org/linux/man-pages/man8/nscd.8.html) ("two caches for each database: a positive one for items found, and a negative one ... Each cache has a separate TTL period"); cross-referenced with the glibc-no-cache survey above which positions nscd as the *separate* hosts caching layer.
**Analysis**: IF nscd is running and caches `hosts`, it will hold the responder's A answer for **the responder's stamped TTL** (currently 1 s), not for the nscd.conf default. So the responder's positive TTL *does* matter to an nscd-bearing host — TTL=1 means nscd holds a backend addr for up to ~1 second after an alloc cycle. nscd's presence is the case where positive TTL is not moot.

#### Finding 2.3: systemd-resolved is a caching, validating DNS/DNSSEC stub resolver; honors record TTL; can be flushed on SIGUSR2
**Evidence**: systemd-resolved "implements a caching and validating DNS/DNSSEC stub resolver." "Upon reception of the SIGUSR2 process signal systemd-resolved will flush all caches it maintains" (also exposed via `resolvectl flush-caches`).
**Source**: [systemd-resolved.service(8) — man.archlinux.org mirror](https://man.archlinux.org/man/systemd-resolved.service.8) — Accessed 2026-06-25 (upstream freedesktop.org returned HTTP 403)
**Confidence**: High (caching + flush behaviour); TTL-clamp specifics established from source in 3.x below
**Verification**: systemd source [`resolved-dns-cache.c`](https://github.com/systemd/systemd/blob/main/src/resolve/resolved-dns-cache.c) (the actual clamp/skip logic, see Findings 3.1–3.3).
**Analysis**: systemd-resolved IS present in the Overdrive dev/appliance image (it already factors into the `:53`/`:5353` bind-coexistence design). So the realistic Overdrive case is: workload `getaddrinfo` → glibc NSS → (systemd-resolved if `nss-resolve`/127.0.0.53 is the path) → responder. systemd-resolved's TTL handling therefore directly governs whether a cycled-alloc addr can be served stale — and its TTL=0 behaviour (3.1) is the decisive detail.

#### Finding 2.4: Application-level resolver caches are out of Overdrive's control
**Evidence**: Many runtimes/libraries cache DNS independently of glibc — e.g. JVM `networkaddress.cache.ttl`, Go's resolver behaviour, and bespoke connection-pool host caching. These sit *above* `getaddrinfo` and neither the responder's TTL nor a system flush reaches them reliably. (General resolver-architecture knowledge; the survey above notes "to improve query lookup time you can set up a caching resolver as an alternative.")
**Source**: [biriukov.dev — getaddrinfo() from glibc](https://biriukov.dev/docs/resolver-dual-stack-application/4-getaddrinfo-from-glibc/) — Accessed 2026-06-25
**Confidence**: Medium (well-known in practice; flagged as out-of-scope rather than exhaustively sourced)
**Analysis**: Out of our control by construction. This is itself an argument for sub-question 4's approach: if the answered address is *stable*, an over-eager application cache is harmless because the cached value never goes stale.

#### Sub-question 2 VERDICT
For **bare glibc** (no nscd, no systemd-resolved on the path): **nothing caches** — every `getaddrinfo`/`getent` is a fresh hit on the responder, which always returns current truth. There is no client cache to invalidate and the positive TTL is **moot**. For **nscd**: it caches `hosts` and **honors the responder's stamped DNS TTL** — so TTL=1 bounds staleness to ~1 s. For **systemd-resolved** (present in the appliance image): it caches and honors TTL, but **does not cache TTL=0 at all** (see 3.1). Application-level caches are out of our control regardless. The consequence the user intuited is correct *for the bare-glibc path*: with no intermediary cache, "de-cache on alloc cycle" has nothing to act on — the responder is already the live source of truth on every query.

---

### Sub-question 3 — TTL=0 vs TTL=1 semantics across the relevant resolvers

#### Finding 3.1: systemd-resolved does NOT cache TTL=0 records at all; TTL=1 records ARE cached (for ~1 s)
**Evidence**: From `resolved-dns-cache.c` (`calculate_until_valid` / `dns_cache_put`): for `min_ttl <= 0` the code path is "if (min_ttl <= 0) { r = dns_cache_remove_by_rr(c, rr); ... return 0; }" with the log line "Not caching zero TTL cache entry" — i.e. a TTL=0 record is actively removed/skipped, never stored. There is "no CACHE_TTL_MIN_USEC constant" forcing a floor on positive TTLs, so a TTL=1 record is cached for ~1 second.
**Source**: [systemd `src/resolve/resolved-dns-cache.c`](https://github.com/systemd/systemd/blob/main/src/resolve/resolved-dns-cache.c) — Accessed 2026-06-25
**Confidence**: High (primary source — the implementation itself)
**Verification**: RFC 2308 §4 / RFC 1035 §3.2.1 (TTL=0 "should not be cached") — systemd's behaviour is RFC-conformant; the systemd issue tracker (#35866 "Add an option to set a maximum cache TTL", #6945 "Add a minimum TTL configuration item") corroborates the constants and the absence of a configurable positive minimum.
**Analysis**: This is the **decisive difference** between TTL=0 and TTL=1 for Overdrive's actual path. On the systemd-resolved path, **TTL=0 = guaranteed re-resolve every time** (record never enters the cache); **TTL=1 = up to ~1 s of stale-addr window** after an alloc cycle. If the design goal is "do not cache / re-resolve every time" through systemd-resolved, **TTL=0 expresses it precisely and TTL=1 does not.**

#### Finding 3.2: systemd-resolved clamps the MAXIMUM positive cache TTL to 2 hours; no minimum-clamp inflates a small TTL
**Evidence**: "#define CACHE_TTL_MAX_USEC (2 * USEC_PER_HOUR)" with clamp "if (u > CACHE_TTL_MAX_USEC) u = CACHE_TTL_MAX_USEC;". No `CACHE_TTL_MIN_USEC` exists (other than the negative-caching SOA-minimum clamp), so a 1 s TTL is honored as 1 s, not rounded up.
**Source**: [systemd `src/resolve/resolved-dns-cache.c`](https://github.com/systemd/systemd/blob/main/src/resolve/resolved-dns-cache.c) — Accessed 2026-06-25
**Confidence**: High (primary source)
**Verification**: systemd issue [#35866](https://github.com/systemd/systemd/issues/35866) (max-TTL discussion) and [#6945](https://github.com/systemd/systemd/issues/6945) (request for a *minimum* TTL knob — its non-existence at the time confirms there is no built-in positive minimum that would inflate a 1 s TTL).
**Analysis**: Reassuring for the low-TTL approach: systemd-resolved will not silently extend a 1 s answer. The only relevant clamp downward is "TTL=0 → not cached" (3.1). The 2 h max is irrelevant to a 0/1 s answer.

#### Finding 3.3: The authoritative TTL=0 "do not cache" semantics live in RFC 1035 §3.2.1 / RFC 1034 — NOT in RFC 2308 §4 (correction to the prompt's framing)
**Evidence**: RFC 1035 §3.2.1: "Zero values are interpreted to mean that the RR can only be used for the transaction in progress, and should not be cached." RFC 1034: "a zero TTL prohibits caching." **Correction**: RFC 2308 §4 does NOT concern TTL=0 of positive records — it redefines the SOA MINIMUM field as "the TTL to be used for negative responses." RFC 2308 §5 sets the negative-cache TTL to "the minimum of the SOA.MINIMUM field and SOA's TTL." So RFC 2308 is the authority for the *negative* answer's TTL (already settled per DDN-8, MINIMUM=1) — it is NOT the authority for the positive-record TTL=0 semantics. Those are RFC 1035 §3.2.1 / RFC 1034.
**Source**: [RFC 1035 §3.2.1](https://www.rfc-editor.org/rfc/rfc1035) — Accessed 2026-06-25
**Confidence**: High
**Verification**: [RFC 1034](https://www.rfc-editor.org/rfc/rfc1034); [RFC 2308 §4/§5](https://www.rfc-editor.org/rfc/rfc2308) (confirmed §4 = SOA MINIMUM redefinition, §5 = negative-cache TTL derivation — neither addresses zero-TTL positive records); systemd `resolved-dns-cache.c` (the conforming implementation that strictly skips TTL=0).
**Analysis**: TTL=0 is the standard, RFC-1035-blessed way to say "re-resolve every time." The standards leave a resolver some MAY-cache-briefly latitude, but the resolver that matters here (systemd-resolved) takes the strict no-cache path. The prompt's reference to "RFC 2308 §4 (TTL of zero)" should be read as RFC 1035 §3.2.1; RFC 2308's relevance is to the *negative* answer only.

#### Finding 3.4: nscd and TTL=0 / TTL=1
**Evidence**: nscd honors the DNS-returned TTL for `hosts` (Finding 2.2). A DNS TTL of 0 therefore yields effectively no positive caching in nscd; a TTL of 1 yields ~1 s. nscd's own `positive-time-to-live` is explicitly *ignored* for DNS-backed hosts entries.
**Source**: [nscd.conf(5) — Linux man-pages](https://man7.org/linux/man-pages/man5/nscd.conf.5.html) — Accessed 2026-06-25
**Confidence**: Medium-High (TTL-honoring is documented; the precise TTL=0 edge in nscd is inferred from "the DNS TTL is used" + RFC 1035 semantics rather than an explicit nscd statement)
**Verification**: RFC 1035 §3.2.1 (TTL=0 not cached); nscd(8).
**Analysis**: Consistent with systemd-resolved: passing TTL=0 down means nscd does not retain the entry; TTL=1 means ~1 s. nscd does not impose a minimum that would inflate the value.

#### Sub-question 3 VERDICT
**TTL=0 is meaningfully different from TTL=1 on the resolvers in Overdrive's path.** On systemd-resolved (present in the appliance image), **TTL=0 is never cached** ("Not caching zero TTL cache entry") while **TTL=1 is cached for up to ~1 second**. nscd behaves analogously (honors the DNS TTL; 0 ≈ no retention, 1 ≈ ~1 s). Neither resolver inflates a small TTL via a minimum clamp; systemd-resolved's only clamp relevant here is the TTL=0-skip. **If the contract is "do not cache / re-resolve every time," TTL=0 expresses it exactly and TTL=1 does not.** The current `A_RECORD_TTL_SECS = 1` leaves a ~1 s staleness window on any caching intermediary; TTL=0 closes it on systemd-resolved/nscd while remaining fully RFC-conformant (RFC 1035 §3.2.1, RFC 2308 §4).

---

### Sub-question 4 — Stable-canonical-address alternative (DNS returns a stable VIP; dataplane handles endpoint churn)

#### Finding 4.1: Kubernetes splits DNS (stable ClusterIP) from endpoint selection (kube-proxy/dataplane) — exactly the "DNS staleness vs dataplane re-resolution" split
**Evidence**: "Kubernetes assigns this Service an IP address (the cluster IP), that is used by the virtual IP address mechanism." The ClusterIP is "a stable endpoint that clients connect to, abstracted from the underlying pod infrastructure" and remains stable for the Service's lifetime even though "the set of Pods running in one moment in time could be different from the set of Pods running that application a moment later." Endpoint selection happens at the dataplane: "DNS resolution → returns stable ClusterIP; Connection establishment → kube-proxy/dataplane routes to healthy endpoints based on current EndpointSlices."
**Source**: [Kubernetes — Service](https://kubernetes.io/docs/concepts/services-networking/service/) — Accessed 2026-06-25
**Confidence**: High
**Verification**: CoreDNS kubernetes plugin (4.3); Cilium kube-proxy-free (4.2). All three describe the same split.
**Analysis**: This is the canonical industry answer to the user's exact concern. DNS deliberately returns a value that **does not change as backends cycle** (the ClusterIP), so DNS-cache staleness is structurally impossible to matter — the cached value is still correct. Backend churn is absorbed entirely below DNS, in the L3/L4 dataplane.

#### Finding 4.2: Cilium does connect-time, socket-level eBPF backend selection from a stable Service IP — a direct analogue of Overdrive's `MtlsResolve` intercept
**Evidence**: "upon connect (TCP, connected UDP), sendmsg (UDP), or recvmsg (UDP) system calls, the destination IP is checked for an existing service IP and one of the service backends is selected as a target." "applications connect to the stable ClusterIP, but eBPF programs intercept the connection and redirect it to a live backend pod directly, without DNS involvement." On churn: "When backends are deleted, Cilium forcefully terminates application sockets that are connected to deleted service backends, so that applications can be re-load-balanced to active backends" and "the eBPF maps storing service-to-backend mappings are updated in real-time as endpoints change."
**Source**: [Cilium — Kubernetes without kube-proxy (eBPF socket LB)](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) — Accessed 2026-06-25
**Confidence**: High
**Verification**: Kubernetes Service doc (4.1, the stable-VIP contract Cilium implements); Istio (4.4, the same split via a userspace proxy).
**Analysis**: Cilium's socket-level (`cgroup/connect4`-class) backend selection from a stable VIP is structurally the same mechanism as Overdrive's `MtlsResolve` per-connection backend selection at the eBPF intercept. The "forcefully terminate sockets to deleted backends → re-load-balance to active backends" behaviour is the precise dataplane primitive that makes an *alloc cycle* a non-event for DNS: the connection is re-steered live, and the name's stable address never needed re-resolving. This is the strongest precedent that Overdrive already owns the right machinery to make the DNS answer stable.

#### Finding 4.3: CoreDNS returns the stable ClusterIP for a normal Service (vs pod IPs only for headless), with a 5 s default TTL (min 0, max 3600)
**Evidence**: The CoreDNS kubernetes plugin "implements the Kubernetes DNS-Based Service Discovery Specification." On TTL: "ttl allows you to set a custom TTL for responses. The default is 5 seconds. The minimum TTL allowed is 0 seconds, and the maximum is capped at 3600 seconds."
**Source**: [CoreDNS — kubernetes plugin](https://coredns.io/plugins/kubernetes/) — Accessed 2026-06-25
**Confidence**: Medium-High (TTL defaults quoted directly; the ClusterIP-vs-headless A-record distinction is established via the Kubernetes Service doc + the DNS-Based Service Discovery spec the plugin implements, since this specific page did not quote it)
**Verification**: [Kubernetes — Service](https://kubernetes.io/docs/concepts/services-networking/service/) (ClusterIP is the stable A target; headless `clusterIP: None` returns pod IPs directly).
**Analysis**: Notably, even the de-facto cluster DNS does NOT chase sub-second TTLs to manage churn — it ships a **5-second** default. It can afford to because the answered ClusterIP is stable; churn is the dataplane's problem, not DNS's. The contrast is instructive: an ecosystem that returns a stable address uses a *relaxed* TTL precisely because TTL staleness is moot; only when DNS returns volatile pod IPs (headless) does low TTL start to matter, and even then it is understood as a weak, racy mechanism.

#### Finding 4.4: Service meshes (Istio, Linkerd) use the same split — DNS to a stable VIP, control plane pushes live endpoints to the sidecar dataplane
**Evidence**: For Istio: "the application resolves service names through DNS to a stable VIP, while the Envoy sidecar maintains and uses the actual live endpoint list." Istiod "converts high level routing rules ... into Envoy-specific configurations, and propagates them to the sidecars at runtime ... This decouples endpoint discovery from DNS resolution — the control plane continuously updates endpoints independent of DNS TTL constraints." Envoy provides "Dynamic service discovery" and "Load balancing" so "the dataplane [selects] backends based on current endpoint state rather than DNS-based resolution at request time."
**Source**: [Istio — Architecture](https://istio.io/latest/docs/ops/deployment/architecture/) — Accessed 2026-06-25
**Confidence**: High
**Verification**: Kubernetes Service doc (4.1) and Cilium (4.2) describe the identical split at L3/L4; Istio/Envoy describe it at L7 via a userspace sidecar. Convergent evidence across three independent mesh/dataplane designs.
**Analysis**: Whether the dataplane is eBPF (Cilium, Overdrive `MtlsResolve`) or a userspace proxy (Envoy/Istio, Linkerd2-proxy), the architecture is the same: **DNS resolves to a stable address; live endpoint selection and churn handling live in the dataplane, decoupled from DNS TTL.** This is an industry-wide convergent pattern, not a single-vendor choice — which materially raises confidence that it is the "least surprising" shape.

#### Sub-question 4 VERDICT
Returning a **stable canonical / per-workload address** and delegating live backend selection to the `MtlsResolve` eBPF intercept is the **industry-standard** way to resolve the "DNS staleness vs dataplane re-resolution" split. Kubernetes (ClusterIP + kube-proxy/EndpointSlices), Cilium (stable Service IP + connect-time socket-level eBPF LB, with forced re-load-balancing on backend deletion), CoreDNS (stable ClusterIP A records, relaxed 5 s TTL), and service meshes (Istio/Linkerd: DNS→stable VIP, control plane pushes endpoints to the sidecar dataplane) all implement exactly this. The trade-off (stated explicitly below) is that it moves complexity from "DNS answer freshness" into "the intercept must recognize the stable address and select a live backend at connect time, and re-steer on churn." Overdrive's transparent-mtls arc (#236) already established the canonical-address pattern and the `MtlsResolve` intercept already does per-connection backend selection — so the machinery exists. **Making the A answer stable makes positive-TTL staleness moot by construction**, which is strictly more robust than chasing low-TTL re-resolution, *especially* given that bare glibc does not cache at all (so low TTL buys nothing on that path) and application-level caches (out of our control) become harmless when the answer never goes stale.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| RFC 1034 (DNS Concepts & Facilities) | rfc-editor.org | High (1.0) | official | 2026-06-25 | Y |
| RFC 1035 (DNS Implementation) | rfc-editor.org | High (1.0) | official | 2026-06-25 | Y |
| RFC 1996 (DNS NOTIFY) | rfc-editor.org | High (1.0) | official | 2026-06-25 | Y |
| RFC 2308 (Negative Caching / SOA MINIMUM) | rfc-editor.org | High (1.0) | official | 2026-06-25 | Y |
| RFC 8765 (DNS Push Notifications) | rfc-editor.org | High (1.0) | official | 2026-06-25 | Y |
| getaddrinfo(3) man-page | man7.org | High (0.95) | official (Linux man-pages) | 2026-06-25 | Y |
| nscd(8) man-page | man7.org | High (0.95) | official (Linux man-pages) | 2026-06-25 | Y |
| nscd.conf(5) man-page | man7.org | High (0.95) | official (Linux man-pages) | 2026-06-25 | Y |
| systemd-resolved.service(8) | man.archlinux.org | High (0.9) | open_source (systemd doc mirror) | 2026-06-25 | Y |
| systemd `resolved-dns-cache.c` | github.com/systemd | High (0.95) | open_source (primary source) | 2026-06-25 | Y |
| systemd issues #35866 / #6945 | github.com/systemd | Medium-High (0.8) | open_source (issue tracker) | 2026-06-25 | Y |
| getaddrinfo() from glibc | biriukov.dev | Medium (0.6) | community (corroborating) | 2026-06-25 | Y |
| Kubernetes — Service | kubernetes.io | High (1.0) | official | 2026-06-25 | Y |
| CoreDNS kubernetes plugin | coredns.io | High (0.9) | open_source | 2026-06-25 | Y |
| Cilium kube-proxy-free (eBPF socket LB) | docs.cilium.io | High (0.9) | open_source | 2026-06-25 | Y |
| Istio Architecture | istio.io | High (0.9) | open_source | 2026-06-25 | Y |

Reputation: High: 13 (81%) | Medium-High: 1 (6%) | Medium: 1 (6%) | Blocked/unfetched (not cited as evidence): ArchWiki (Anubis), freedesktop.org (403) | Avg reputation (cited sources): ~0.91

## Knowledge Gaps

### Gap 1: Which resolver path the Overdrive appliance workload netns actually traverses (systemd-resolved vs direct-to-responder)
**Issue**: This research establishes systemd-resolved is *present* in the image and factors into the `:53`/`:5353` bind design — but ADR-0072 points workloads at the per-netns gateway addr via injected `resolv.conf`. Whether a workload's `getaddrinfo` goes (a) straight to the gateway/responder over UDP (bare-glibc-no-cache path → TTL moot) or (b) through a local systemd-resolved stub at 127.0.0.53 that then forwards to the responder (caching path → TTL=0 vs TTL=1 matters) is not settled by the public docs; it depends on Overdrive's exact `nsswitch.conf` / `resolv.conf` injection. **Recommendation**: confirm against the ADR-0072 deliver artifacts / the `resolv.conf` injection code which of the two paths is live; the TTL recommendation's force depends on it (it is decisive only on a caching path).

### Gap 2: nscd presence in the appliance image
**Issue**: nscd's hosts caching honors the DNS TTL (established), but whether nscd is installed/enabled in the Overdrive appliance image was not verified from public sources. **Recommendation**: check the image manifest; nscd is increasingly deprecated in favour of systemd-resolved, so likely absent — but confirm, since it is the other caching layer that would make positive TTL matter.

### Gap 3: Exact systemd version in the appliance vs the `main`-branch source read
**Issue**: The TTL=0-skip and 2 h max-clamp were read from systemd `main` `resolved-dns-cache.c`. The appliance pins a specific systemd version; the clamp constants are stable across recent releases but were not re-verified against the pinned tag. **Recommendation**: spot-check the pinned systemd tag's `resolved-dns-cache.c` if the TTL=0 behaviour becomes load-bearing for the decision.

## Conflicting Information (if applicable)

### Conflict 1: The prompt's attribution of TTL=0 semantics to "RFC 2308 §4"
**Position A (prompt framing)**: TTL=0 "should not be cached" semantics are in RFC 2308 §4.
**Position B (sources)**: RFC 2308 §4 redefines the SOA MINIMUM field (negative-caching TTL); §5 derives the negative-cache TTL. Neither addresses zero-TTL *positive* records. The TTL=0 "should not be cached" rule is RFC 1035 §3.2.1 ("Zero values ... should not be cached") and RFC 1034 ("a zero TTL prohibits caching"). — Source: [RFC 2308](https://www.rfc-editor.org/rfc/rfc2308), [RFC 1035](https://www.rfc-editor.org/rfc/rfc1035), Reputation 1.0.
**Assessment**: Position B is correct (primary RFC text). This is a citation-accuracy correction, not a substantive disagreement — the conclusion (TTL=0 = "do not cache") is unchanged; only the governing RFC differs. RFC 2308 remains the authority for the *negative* answer (DDN-8, MINIMUM=1), which is out of scope here.

## Final Recommendation

The user's framing question — "can we de-cache a name when its backend cycles, instead of relying on a short positive TTL?" — resolves to: **no standard de-cache primitive reaches the glibc client (sub-q 1), and on the bare-glibc path there is no cache to de-cache anyway (sub-q 2).** So the real choice is among (a) keep TTL=1, (b) move to TTL=0, (c) re-shape v1 to a stable canonical address. Ranked:

1. **Preferred (architectural, if the ADR answer contract can change): (c) answer a STABLE canonical / per-workload address and delegate churn to the `MtlsResolve` intercept.** This makes positive-TTL staleness *moot by construction* (sub-q 4), is the convergent industry pattern (K8s/Cilium/CoreDNS/Istio/Linkerd), neutralises application-level caches we don't control, and leans on machinery Overdrive already has (the #236 canonical-address pattern + per-connection backend selection). The cost: the answer is no longer byte-identical to a specific backend addr, so the intercept must recognise the stable address and pick a live backend at connect time (and ideally re-steer on alloc cycle, as Cilium force-terminates sockets to deleted backends). **This is an ADR-0072 answer-contract change — surface to the architect, do not adopt unilaterally (flagged below).**

2. **Best immediate change within the current contract: (b) TTL=0.** If the answer must stay a specific backend addr (current contract), TTL=0 is strictly the least-surprising value: it is the RFC-1035 way to say "re-resolve every time," it is *exactly honored* by systemd-resolved (never cached) and nscd (honors DNS TTL), and it removes the ~1 s stale-addr window that TTL=1 leaves on any caching intermediary. On the bare-glibc path it is equivalent to TTL=1 (both moot, no cache), so TTL=0 is never worse and is strictly better wherever a cache exists.

3. **Acceptable but weakest: (a) keep TTL=1.** Defensible only as "a tiny non-zero TTL avoids any resolver that mishandles zero." But the resolver that matters (systemd-resolved) handles TTL=0 correctly (skips caching) and there is no evidence of a TTL=0-hostile resolver in Overdrive's path; meanwhile TTL=1 leaves a deliberate ~1 s staleness window for no benefit on the bare-glibc path. The "short because a backend addr can change" rationale is sound, but **0 expresses that intent more precisely than 1**.

**Least-surprising note given bare glibc does not cache**: because the dominant/realistic glibc path has no cache, TTL value is *inert* there — which means (b) TTL=0 costs nothing and only helps on caching paths, while (c) is the only option that also defends against the caches we genuinely cannot control (app-level, future intermediaries). If the team wants a one-line change now and an architectural decision later: **ship TTL=0 immediately (b), and open the canonical-address reshape (c) as the v1.x architectural follow-up.**

## ADR-0072 Answer-Contract Flags (surface, do not decide)

The following would change the ADR-0072 **answer contract** and are therefore architect decisions, not the researcher's to make — surfacing them, not deciding:

1. **Option (c) changes what the A record means.** Today the answered A addr is byte-identical to the specific backend addr the `MtlsResolve` intercept recognises (derived from `ServiceBackendRow.Backend.addr`). Switching to a stable canonical / per-workload address breaks that byte-identity and requires the intercept to map the stable address → live backend at connect time. That is a contract change to ADR-0072 *and* a coupling to the intercept's address-recognition logic. **Architect decision.**
2. **Even option (b) TTL=0 is a change to the wire contract** (`A_RECORD_TTL_SECS = 1` → `0`). It is low-risk and RFC-conformant, but it is still an edit to the codec's stamped TTL constant and should be ratified as the intended positive-TTL semantics for ADR-0072, not slipped in. **Architect ratification recommended.**
3. **The negative answer (NXDOMAIN/NODATA SOA MINIMUM=1, DDN-8) is settled and out of scope** — do not let a positive-TTL change perturb it; they are governed by different RFCs (positive: RFC 1035 §3.2.1; negative: RFC 2308 §5).
4. **Path-dependence (Gap 1) gates the urgency.** If workloads resolve straight to the responder with bare glibc (no systemd-resolved stub on the path), TTL is moot and (b) is cosmetic; the case for (c) is then about app-level/uncontrolled caches and future-proofing, not about systemd-resolved. The architect should pin which resolver path is live before weighting the options.

## Full Citations

[1] IETF. "RFC 1034 — Domain Names: Concepts and Facilities". November 1987. https://www.rfc-editor.org/rfc/rfc1034. Accessed 2026-06-25.
[2] IETF. "RFC 1035 — Domain Names: Implementation and Specification". November 1987. https://www.rfc-editor.org/rfc/rfc1035. Accessed 2026-06-25.
[3] IETF. "RFC 1996 — A Mechanism for Prompt Notification of Zone Changes (DNS NOTIFY)". August 1996. https://www.rfc-editor.org/rfc/rfc1996. Accessed 2026-06-25.
[4] IETF. "RFC 2308 — Negative Caching of DNS Queries (DNS NCACHE)". March 1998. https://www.rfc-editor.org/rfc/rfc2308. Accessed 2026-06-25.
[5] IETF. "RFC 8765 — DNS Push Notifications". June 2020. https://www.rfc-editor.org/rfc/rfc8765. Accessed 2026-06-25.
[6] Linux man-pages project. "getaddrinfo(3)". https://man7.org/linux/man-pages/man3/getaddrinfo.3.html. Accessed 2026-06-25.
[7] Linux man-pages project. "nscd(8)". https://man7.org/linux/man-pages/man8/nscd.8.html. Accessed 2026-06-25.
[8] Linux man-pages project. "nscd.conf(5)". https://man7.org/linux/man-pages/man5/nscd.conf.5.html. Accessed 2026-06-25.
[9] systemd project. "systemd-resolved.service(8)" (Arch mirror; upstream freedesktop.org returned HTTP 403). https://man.archlinux.org/man/systemd-resolved.service.8. Accessed 2026-06-25.
[10] systemd project. "src/resolve/resolved-dns-cache.c" (main branch). https://github.com/systemd/systemd/blob/main/src/resolve/resolved-dns-cache.c. Accessed 2026-06-25.
[11] systemd project. "Issue #35866 — resolved: Add an option to set a maximum cache TTL" and "Issue #6945 — Add a minimum TTL configuration item for broken DNS servers". https://github.com/systemd/systemd/issues/35866, https://github.com/systemd/systemd/issues/6945. Accessed 2026-06-25.
[12] Biriukov, V. "getaddrinfo() from glibc". https://biriukov.dev/docs/resolver-dual-stack-application/4-getaddrinfo-from-glibc/. Accessed 2026-06-25.
[13] The Kubernetes Authors. "Service". https://kubernetes.io/docs/concepts/services-networking/service/. Accessed 2026-06-25.
[14] CoreDNS. "kubernetes plugin". https://coredns.io/plugins/kubernetes/. Accessed 2026-06-25.
[15] Cilium. "Kubernetes without kube-proxy (eBPF socket-level load balancing)". https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-06-25.
[16] Istio. "Architecture". https://istio.io/latest/docs/ops/deployment/architecture/. Accessed 2026-06-25.

## Research Metadata

Duration: ~1 session | Examined: 18 sources (16 cited; ArchWiki + freedesktop.org blocked/403 and not cited as evidence) | Cited: 16 | Cross-refs: every major claim verified against >=2 independent sources, most against an RFC + an implementation/doc | Confidence distribution: High ~85%, Medium-High ~10%, Medium ~5% | Output: docs/research/networking/dns-positive-ttl-vs-cache-invalidation-dial-by-name-research.md
