# Research: Same-node mesh dials to VM addresses with no host-local prefix delivery and no default route

**Date**: 2026-10-13 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (kernel mechanism, read from source) / Medium (recommendation, pending the Linux 7.0 spike in section 6) | **Sources**: 8 distinct kernel files, 9 Overdrive documents, Cilium source, 4 web pages

Feature: GH #295 (netns-density-295). Kernel citations are `vendors/linux` at tag `v7.0`, written `path:line @ v7.0`. Nothing in this document was executed. Every "verified" item below is a source reading or a quote from the existing probe `docs/feature/netns-density-295/spike/findings-prefix-route-lo.md`. Every inference is labelled.

## Executive Summary

`connect()` on a TCP or connected-UDP socket fails with `ENETUNREACH` because the kernel does a full route lookup (`fib_lookup`, with policy rules) before any packet exists. That lookup uses the **socket's** `sk_mark`, bound device, source address and uid (`include/net/route.h:332 @ v7.0`). The nft OUTPUT mark and fwmark rule apply only later, to a packet, via `ip_route_me_harder` (`net/netfilter/nft_chain_route.c:41-48`, `net/ipv4/netfilter.c:21-53 @ v7.0`). So the failure is not a netfilter or TPROXY defect: the divert is a second route lookup that never gets reached when the first one fails.

The design round's ADR-0188 supplies the missing first-lookup route with a main-table `unicast <prefix> dev lo src 127.0.0.1`. That works, but it makes every prefix address a local-delivery route for every host process (`route.c:2595-2596`, `:1666-1668 @ v7.0`), which is the thing the user's ruling removed. ADR-0188 rejected the obvious alternative, setting the mark at connect time, for two reasons. Both reasons are shown below to be weaker than the ADR states:

1. "Source becomes the destination" (`route.c:2848-2853 @ v7.0`): true only when the socket has no source. Binding the leg-B socket to `127.0.0.1` before `connect()` skips that branch (`route.c:2849`). The table-100 route can also carry a preferred source (`fib_valid_prefsrc`, `fib_semantics.c:1324-1346 @ v7.0`).
2. "Marking every leg-B would send remote dials to `lo`": true only if the mark is unconditional. The dialer knows the destination; it marks only dials whose destination is inside the guest prefix.

**Recommendation (Medium confidence, spike required):** leg-B sets `SO_MARK = MTLS_TPROXY_MARK` and `bind(127.0.0.1)` before `connect()`, only when the peer address lies inside `GUEST_NODE_PREFIX`. No route covers the prefix. The existing `ip rule fwmark -> table 100 (local default dev lo)` supplies the first-lookup route, and the existing output tuple-set divert, TPROXY and constant reject/drop rules are unchanged. The guest prefix then has no route that delivers locally to an unmarked packet, so INB-1 holds literally and ADR-0189 (INB-1-READ) is unnecessary. ADR-0188 is replaced rather than amended.

A structurally simpler long-term option exists (leg-B dials the node's own leg-C listener directly, with the original destination passed in-process) but it changes leg-C's contract; see section 4, alternative C-prime.

## Research Methodology

**Search Strategy**: read the ADRs/feature-delta/probe named in the brief; read the Linux 7.0 source for each step of `connect()`; read Cilium source for socket-LB and proxy routing; web search for prior art.
**Source Selection**: Types: vendored kernel source (authoritative), project ADRs and probe transcript, Cilium source, kernel/tool docs | Reputation: high / medium-high | Verification: kernel claims cross-checked against the probe transcript where the probe exercised the same path.
**Quality Standards**: kernel claims are single-source (the source itself is authoritative); design conclusions are labelled inference unless the probe covers them.

## 1. Verified facts: where `ENETUNREACH` comes from, and what applies before or after the route lookup

| # | Fact | Evidence |
|---|---|---|
| K1 | The cgroup `connect4` hook runs in `pre_connect`, **before** `tcp_v4_connect` and therefore before its route lookup. It can rewrite `user_ip4`/`user_port`, and the rewritten address is what is routed. | `net/ipv4/af_inet.c:592-593, :679-680`; `net/ipv4/tcp_ipv4.c:206-218` (`tcp_v4_pre_connect`), `:254-256` (`ip_route_connect` on `usin->sin_addr`) `@ v7.0` |
| K2 | `tcp_v4_connect` calls `ip_route_connect(...)` and returns its error to the caller; `-ENETUNREACH` also bumps `IPSTATS_MIB_OUTNOROUTES`. Connected UDP does the same in `__ip4_datagram_connect`. | `tcp_ipv4.c:254-262`; `net/ipv4/datagram.c:49-57 @ v7.0` |
| K3 | The lookup key is built from the **socket**: `flowi4_init_output(fl4, oif, READ_ONCE(sk->sk_mark), tos, scope, proto, flags, dst, src, dport, sport, sk_uid(sk))`. `IP_TRANSPARENT` only adds `FLOWI_FLAG_ANYSRC` (relaxes source validation). | `include/net/route.h:319-335 @ v7.0` |
| K4 | If the socket has no destination or no source, `ip_route_connect` first does a source-choosing lookup (`__ip_route_output_key`); with both set it goes straight to `ip_route_output_flow`. | `route.h:337-356 @ v7.0` |
| K5 | `fib_lookup` runs the policy rules, matching `fwmark` against `flowi_mark` (`(rule->mark ^ fl->flowi_mark) & rule->mark_mask`). If no rule yields a route the error is `-ESRCH`, reported as `-ENETUNREACH`. | `net/core/fib_rules.c:292`; `net/ipv4/fib_rules.c:104-105`; `route.c:2814 @ v7.0` |
| K6 | **A bound output device makes the lookup ignore routes through other devices** (`flowi4_oif != nhc_oif` -> nexthop not good). When the lookup then fails and `oif` is set, `ip_route_output_key_hash_rcu` assumes the destination is on-link via that device and builds a route (`res->type = RTN_UNICAST`, `saddr = inet_select_addr(dev_out, 0, RT_SCOPE_LINK)` if unset). | `net/ipv4/fib_trie.c:1413-1414`; `route.c:2774-2801, 2814-2843 @ v7.0` |
| K7 | If the lookup returns `RTN_LOCAL` (what `local default dev lo` in table 100 returns): `saddr` = route `fib_prefsrc` if set, **else the destination**, only `if (!fl4->saddr)`; output device is loopback; route flagged `RTCF_LOCAL`. | `route.c:2848-2868 @ v7.0` |
| K8 | A preferred source is accepted on a `local` route if it resolves as a local address (127.0.0.1 does; the check also consults the local table). | `net/ipv4/fib_semantics.c:1324-1346 @ v7.0` (**inference**: that `ip route ... table 100 src 127.0.0.1` is accepted for a `/0` local route; untested) |
| K9 | Any route whose output device is loopback gets `RTCF_LOCAL`; a local route's `dst.input` is `ip_local_deliver`. This is why ADR-0188's `dev lo` unicast route is a local-delivery route for every host-originated packet. | `route.c:2595-2596`, `:1666-1668 @ v7.0` |
| K10 | A loopback source on a non-loopback output device returns `-EINVAL` unless `route_localnet` is set on that device. So `bind(127.0.0.1)` plus an unmarked connect that resolves to an uplink fails `EINVAL` (closed), not a leak. | `route.c:2578-2582 @ v7.0` |
| K11 | The socket mark reaches the packet: `skb->mark = READ_ONCE(sk->sk_mark)` in `ip_queue_xmit`. `__sock_set_mark` resets the socket dst cache, so setting the mark before `connect()` is clean. `SO_MARK` needs `CAP_NET_RAW` or `CAP_NET_ADMIN`; leg-S already sets it (`mtls/mod.rs:659-688`). | `net/ipv4/ip_output.c:532`; `net/core/sock.c:1003-1009, :1520-1528 @ v7.0` |
| K12 | The nft `type route` OUTPUT chain re-routes only if saddr, daddr, mark or tos changed; with a socket already marked 1 the divert's `meta mark set 1` changes nothing. `ip_route_me_harder` re-looks-up with `fl4.flowi4_mark = skb->mark`, `flowi4_oif = sk->sk_bound_dev_if`, and the packet's own source. | `net/netfilter/nft_chain_route.c:31-48`; `net/ipv4/netfilter.c:21-53 @ v7.0` |
| K13 | `cgroup/sock_create` programs may **write** `bpf_sock.mark` (and `bound_dev_if`, `priority`); `cgroup/connect4` programs may call `bpf_setsockopt(SOL_SOCKET, SO_MARK)` (accepted in `sol_socket_sockopt`) and `bpf_bind`. `sock_ops` `TCP_CONNECT_CB` fires inside `tcp_connect`, after the route is chosen. | `net/core/filter.c:8219-8238, :5302-5320, :8191-8198, :8961-8975`; `net/ipv4/tcp_output.c:4302 @ v7.0` |
| K14 | Blackhole/unreachable/prohibit routes make `connect()` fail at the lookup with `EINVAL`/`EHOSTUNREACH`/`EACCES`; `throw` gives `EAGAIN`. They cannot produce a connect that proceeds. | `net/ipv4/fib_semantics.c:113-128 @ v7.0` |
| K15 | Probe (Linux 7.0.0-38, Lima): without a route, `connect()` returns `ENETUNREACH` at once; with the `dev lo src 127.0.0.1` route the diverted dial reaches leg-C with `peer=127.0.0.1`, `local=100.95.0.7:8080`; the same route lets a UDP datagram reach a host wildcard `:53` socket and TCP SYNs reach a host listener when the constant rules are removed. | `findings-prefix-route-lo.md` cases 1, 2, 5 |

Confidence: High. K1-K7, K9-K14 are direct source readings; K15 is an executed transcript; K8 is the only item that is source-reading plus inference.

## 2. Findings that change the design round's reasoning

### Finding 1: the first lookup is where policy routing must select, so the selector must be the dialer, not the destination

**Evidence**: K3, K5. Any rule selecting by destination (`to <prefix>`, or a prefix route in main) is applied to every process, so the prefix becomes locally deliverable for everyone (K9). Only selectors identifying the **dialer** (fwmark, bound `oif`, uid range) keep the prefix non-local for everyone else.
**Confidence**: High (fib rule matching on mark: `fib_rules.c:292`). The statement that uid and `oif` are rule selectors is general iproute2 knowledge, not source-verified here.
**Analysis**: this is the structural reason the lo route is the wrong shape: it encodes "this prefix is local" as a fact about the destination, when the real fact is "this one socket is a mesh dialer".

### Finding 2: ADR-0188's two stated reasons for rejecting connect-time marking do not hold

- **Source**: K7 says the source becomes the destination only `if (!fl4->saddr)` (`route.c:2849`). With `bind(127.0.0.1)` the socket supplies a source, and `__ip_dev_find` validation passes because 127.0.0.1 is a local address (`route.c:2766-2770`). Alternatively the table-100 route can carry `src 127.0.0.1` (K8).
- **Remote dials**: marking is conditional on the destination being in the guest prefix; remote (non-prefix) dials stay unmarked.
- **Evidence the end state is identical**: after the existing divert, `ip_route_me_harder` looks up with saddr = the socket's chosen source and mark = 1 (K12), selecting the same table-100 route that a pre-marked connect selects. The post-divert packet in the ADR-0188 probe and the packet in the marked-connect design are therefore the same (**inference**, S1 verifies).
**Confidence**: High on the kernel statements, Medium on end-state equivalence.

### Finding 3: a bound output device alone also removes the route requirement (and needs no mark)

**Evidence**: K6. A socket bound to `lo` ignores any default route through the uplink (the nexthop's `nhc_oif != lo`), then falls into the "assume on-link" branch and builds a loopback route. `ip_route_me_harder` keeps `oif = sk_bound_dev_if` (K12), and table 100's `local default dev lo` has `nhc_oif = lo`, so the later divert lookup still succeeds.
**Confidence**: Medium (source-derived, unprobed). The on-link fallback is a long-standing kernel hack ("WHY? DW", `route.c:2823`); its stability on 7.0 is a source fact, its stability in future kernels is not Overdrive's concern because the kernel is pinned (ADR-0068).
**Analysis**: this is a second working mechanism (alternative A2 below), but it relies on a legacy fallback rather than the explicit policy table, so it ranks below mark-at-connect.

## 3. What the existing design needs from a correct mechanism

Constraints, from the brief, ADR-0172/0173/0186/0188/0189 and the user rulings (`.context/over-engineering-audit-review.md`, "Finding 1" 2026-10-11):

- no route, in any table reachable by an unmarked host process, delivers locally for any guest-prefix address (INB-1 literally, not at the socket);
- no dependency on a default route;
- leg-B's source stays outside the prefix (ADR-0172 Context: replies into the prefix are refused by the output reject);
- leg-C still learns the original destination from the TPROXY-preserved local address (ADR-0125, probe case 2);
- the output admission by tuple set (`inbound_destinations`, ADR-0125 rule 7) keeps refusing unknown ports;
- no new Overdrive-written node route (each is another thing `serve` must converge and refuse on conflict).

Not verified: how a **remote** node's VM is addressed from a client node's leg-B. `GUEST_NODE_PREFIX` is a single constant `100.95.0.0/16` (`feature-delta.md:6150`) and ADR-0188 says the route "carries no traffic to another node". If every node uses the same prefix, a prefix-destined dial is same-node by construction. If node prefixes are distinct and fabric-routed, "destination in this node's prefix" still identifies same-node dials. Either way the discriminator "peer in `GUEST_NODE_PREFIX`" selects exactly the dials that need the mark. This premise should be confirmed in DESIGN (section 7, Gap 1).

## 4. Alternatives evaluated

| Alt | Mechanism | Fixes first-lookup ENETUNREACH? | Prefix stays non-local for unmarked/other processes? | Verdict |
|---|---|---|---|---|
| **A** | `SO_MARK` + `bind(127.0.0.1)` on leg-B for prefix-destined dials only; existing table 100 + divert + TPROXY unchanged | Yes: rule `fwmark 1 -> table 100` yields `local default dev lo` at connect (K3, K5, K7) | Yes: unmarked lookup finds no route (ENETUNREACH) or, with a default route, the output reject fires (K10 gives EINVAL if the source is bound); no main-table route | **Recommended**, pending S1-S4 |
| A2 | `SO_BINDTODEVICE`/`SO_BINDTOIFINDEX` = `lo` + `bind(127.0.0.1)` on leg-B | Yes via the on-link fallback (K6) | Yes | Viable fallback; rests on a legacy fallback branch; rank 2 |
| A3 | Same as A but marking in BPF: `cgroup/sock_create` sets `bpf_sock.mark`, or `cgroup/connect4` calls `bpf_setsockopt(SO_MARK)` for prefix destinations (K13) | Yes | Yes | Equivalent effect to A with a BPF program that has no Tier-2 backstop (`cgroup_sock_addr` returns `ENOTSUPP` under `BPF_PROG_TEST_RUN`, `.claude/rules/development.md`); leg-B is not in a workload cgroup so a new attachment point and filter are needed. A userspace `setsockopt` in the dialer that owns the decision is simpler and testable. Rejected. |
| B | `cgroup/connect4` rewrites the prefix destination to the leg-C/owner address (Cilium socket-LB shape) | Yes (K1: rewrite precedes the lookup) | Yes | Loses the original destination: leg-C recovers it from the TPROXY local address. Recovering it after a rewrite needs a map keyed on something the accepted socket also shows; the source port is not allocated until after `connect4` (autobind follows `pre_connect`, `af_inet.c:592-598`) unless `bpf_bind` picks it (K13). Large new BPF surface with no Tier-2 test. Rejected as over-engineered relative to A. |
| C-prime | leg-B dials the node's **own** leg-C listener directly (loopback), and the original destination travels in-process (registry keyed by leg-B's source port), not in the TPROXY address | Yes: destination `127.0.0.1:<leg-C>` is local, no policy routing needed | Yes, and strongest: the OUTPUT divert, fwmark use for leg-B, and table-100 dependence for same-node dials disappear; the output reject can match the whole prefix | Structurally the cleanest, but changes leg-C's accept contract (a second entry path distinguished from TPROXY'd peers), needs an in-process trust argument for the registry, and assumes leg-B and leg-C share a process (**unverified**). Larger blast radius (ADR-0120, 0125, 0172, R5-43). Candidate for a later simplification, not the fix for this finding. |
| D1 | Main-table route `unicast <prefix> dev lo src 127.0.0.1` (ADR-0188) | Yes | **No**: local delivery to every host socket for non-diverted packets (K9, K15 case 5); the constant rules become load-bearing | Rejected by the user's ruling |
| D2 | Same route on a dummy device | Yes | Packets are freed at the dummy's transmit; loopback source needs `route_localnet` or a node-address/gateway source | Rejected in ADR-0188 for sound reasons (source coupling, one more netdevice); still the fallback if the user insists on a prefix route |
| D3 | `blackhole`/`unreachable`/`prohibit`/`throw` | **No**: the connect itself fails (K14) | n/a | Rejected |
| D4 | Policy rule keyed on destination (`ip rule add to <prefix> lookup 100`) | Yes | **No**: same local delivery for everyone, and for remote packets arriving on the uplink | Rejected |
| D5 | Rule keyed on dialer identity (`uidrange`, `oif lo`) instead of mark | Possible | Yes | Equivalent to A in effect; mark is already the vocabulary of the divert (`MTLS_TPROXY_MARK`) and a uidrange would cover all agent sockets. Not preferred. |
| E1 | `sk_lookup` / sockmap redirect at the leg-B side | No: `sk_lookup` runs on the input path after a route exists | n/a | Removed by the user's ruling (the steering stack). Rejected. |
| E2 | `IP_TRANSPARENT` / `IP_FREEBIND` on leg-B | No: they relax source validation only (K3) | n/a | Does not address a missing route. Rejected. |
| F | Rely on the node's default route | Only when one exists | n/a | A fallback around an unobserved fact (`.claude/rules/design.md`); rejected. |

Note on alternative A and the remaining local-delivery question: marked packets are delivered locally by table 100, but the output chain still sends a marked packet to the reject rule unless its tuple is in `inbound_destinations` (the divert rule accepts only set members), and prerouting TPROXY/drop covers the rest. A marked packet can therefore only ever reach leg-C. This is the same barrier that protects TPROXY'd remote traffic today (**inference** from ADR-0125 rule order; S3 verifies).

## 5. Prior art

### Cilium (source at `/Users/marcus/git/cilium/cilium`; `ORIG_HEAD` `dac977e678570c810f07eadeccd0493efc6f0796`, the checkout's `HEAD` ref was not resolvable without git, so treat the commit as approximate)

- **Socket-LB avoids the route problem by rewriting before the lookup.** `cil_sock4_connect` (`bpf/bpf_sock.c:458-474`) calls `__sock4_xlate_fwd`, which looks up the service by `ctx->user_ip4`/port (`:299-324`), picks a backend and sets `ctx->user_ip4 = backend->address; ctx_set_port(ctx, backend->port)` (`:452-453`). This is K1 in practice: the kernel then routes the backend address, so a ClusterIP never needs a route. It records a reverse-NAT entry keyed by socket cookie (`sock4_update_revnat`, `:145-158`, `:446`) so `getpeername`/`recvmsg` can translate back. Cilium has no ENETUNREACH handling for its own dials (a search for `ENETUNREACH` in the tree finds only vendored errno tables and unrelated handlers).
- **Socket mark as a connect-time signal, then reset.** `sock_is_health_check` reads `SO_MARK` in `connect4` (`:94-104`) and `sock_reset_health_check_marker` clears it afterwards so it "does not leak into the tcx datapath" (`:106-120`). A precedent that the socket mark is an accepted, in-tree way for a dialer to tell the datapath what it is.
- **Proxy routing is the same table-with-`local default dev lo` shape Overdrive already has.** `installToProxyRoutesIPv4` installs `RTN_LOCAL 0.0.0.0/0 dev lo` in `RouteTableToProxy` (2004) and a rule keyed on the magic mark (`pkg/proxy/routes.go:25-37, :164-185`; table ids `pkg/datapath/linux/linux_defaults/linux_defaults.go:18-22`). Source is left unset there (`Src: AddrFrom4([4]byte{})`); Overdrive's leg-B must supply a source by `bind` or `src`, because its dialer is a host socket rather than an ingress packet.
- **Not found:** how Cilium keeps pod/service CIDRs non-local on the host for locally originated traffic without socket-LB. The per-packet path was not read in this pass. This does not change the recommendation (Gap 2).

### Web

- The tproxy usage pattern, `ip route add local default dev lo table 100` + `ip rule add fwmark ... lookup 100`, is standard ([kernel tproxy doc](https://docs.kernel.org/6.6/networking/tproxy.html), [sshuttle tproxy page](https://sshuttle.readthedocs.io/en/latest/tproxy.html)). The sshuttle page states "The routes for the outgoing packets must already exist": the same class of failure for locally generated traffic. A 2006 LKML thread titled "Strange network routing behaviour when routing locally generated packets" is the older statement of K3/K5 (the search snippet said a local process cannot send to a destination reachable only through a table selected by `ip rule fwmark`; the page itself was not opened).
- Istio ambient/ztunnel redirects in the pod network namespace with TPROXY marks and Geneve tunnels ([Jimmy Song](https://jimmysong.io/en/blog/ambient-mesh-l4-traffic-path/)): pods have real routable addresses, so no analogous "no covering route" case arises.
- Firecracker/Kata use vsock as a control or data channel with host-side AF_UNIX multiplexing ([Firecracker vsock doc](https://github.com/roypat/firecracker/blob/main/docs/vsock.md), [Kata-dev thread](https://lists.katacontainers.io/pipermail/kata-dev/2019-December/001149.html)); no source found that gives a NIC-less VM an IPv4 workload address with transparent same-node mesh interception.
- gVisor and Cloudflare were not researched beyond the searches above (no relevant hit surfaced); see Gap 2.

Conclusion on prior art (**inference**): none of the references provides a design for "a destination address with no covering route and no local delivery"; the nearest in-tree analogues are socket-LB (rewrite before the lookup) and mark-selected `local default dev lo` tables (Cilium proxy, tproxy docs). Alternative A is the second pattern applied at the dialer, with the mark set from the dialer's own knowledge of the destination.

## 6. Recommendation

**Adopt alternative A: leg-B marks and source-binds same-node VM dials; no prefix route anywhere.**

Mechanism:
1. In leg-B's dial for a peer inside `GUEST_NODE_PREFIX`: create the socket, set `SO_MARK = MTLS_TPROXY_MARK` (0x1), `bind(127.0.0.1:0)` (with `IP_BIND_ADDRESS_NO_PORT` to avoid early port reservation), then `connect`. Dials to any other peer are untouched.
2. The connect-time lookup matches `fwmark 1 -> table 100` (priority before main, as the existing divert already requires) and returns `local default dev lo` (K5, K7); the source is the bound 127.0.0.1, so ADR-0172's "replies never address the prefix" holds.
3. The output chain, TPROXY, constant reject/drop rules and leg-C are unchanged; the output tuple-set rule continues to admit only active VM tuples.
4. Remove the PREFIX-ROUTE-LO boot step, the route protocol tag and the guest-prefix route conflict refusal. INB-1 holds at the route table, with no INB-1-READ decision.

Why this is the correct mechanism rather than the easiest: the failure is "the dialer's first route lookup does not know it is a mesh dial". The mechanism that carries that knowledge to the lookup is the lookup's own input, the socket's mark and source (K3), set by the component that owns the decision. It adds no node state, no new Overdrive-written route, no netdevice and no BPF program, and it keeps the prefix unrouted for every other process (fail-closed: an unmarked dial gets `ENETUNREACH` with no default route, or the existing reject with one; a bind-without-mark dial gets `EINVAL`, K10).

Second choice: alternative A2 (bind to `lo`) if S1 shows an unexpected interaction between the socket mark and the table-100 route. Longer term, alternative C-prime removes the same-node TPROXY hop entirely, but only through a leg-C contract change that belongs in its own DESIGN decision.

### Spike on Linux 7.0 (contract-shaping per `.claude/rules/design.md`; run in Lima, as root; Python or Rust probes in `spike-scratch/`, not `crates/`)

Environment per ADR-0188's probe (namespace with `lo`, a dummy uplink `up0`, `ip rule add fwmark 0x1 lookup 100`, `ip route add local 0.0.0.0/0 dev lo table 100`, the production nft shape: output `route` chain divert on a tuple set, prerouting TPROXY, constant reject/drop). Record `uname -r`.

| Id | Hypothesis | Prediction | Falsification |
|---|---|---|---|
| S1 | Marked + bound(127.0.0.1) connect to a diverted tuple with **no default route and no prefix route** reaches the TPROXY listener | Round trip; listener sees `peer=127.0.0.1`, `local=100.95.0.7:8080`; reject/drop counters 0; `ip route get 100.95.0.7 mark 1 from 127.0.0.1` shows `dev lo` | `ENETUNREACH`, `EINVAL`, a different peer/local pair, or a non-zero reject counter |
| S2 | Same with a default route via `up0` present | Same result; zero IPv4 packets on `up0` (capture) | Any IPv4 on `up0`, or a route via the uplink |
| S3 | Marked socket to a prefix tuple **not** in the set (unknown port, unknown address) | `ECONNREFUSED` from the output reject; no host socket receives it (run with a wildcard `:8080` TCP and `:53` UDP listener present) | Delivery to the wildcard listener, or a timeout |
| S4 | Unmarked controls: (a) no default route; (b) with default route; (c) marked but not bound; (d) bound but not marked | (a) `ENETUNREACH`; (b) `ECONNREFUSED` from the reject; (c) `ip route get` shows source = destination (confirms ADR-0188's claim, and shows whether `src 127.0.0.1` on the table-100 route changes it, K8); (d) `EINVAL` (K10) or `ENETUNREACH` | Any unmarked connect delivering to a host socket |
| S5 | Non-prefix dial through leg-B's code path remains unmarked and routes via the uplink | Normal route via `up0` | Any non-prefix dial landing on `lo` |
| S6 | Alternative A2 (bind to `lo`, no mark), run with and without a default route | Connect succeeds in both (K6); then the divert reaches leg-C | `ENETUNREACH` or a route through `up0` |
| S7 | Production `ip rule` order: dump the rules `serve` installs and confirm the fwmark-1 rule precedes main | Priority below main's 32766 | A rule order where main's default route wins for mark 1 |
| S8 | `serve`-level Tier 3: one microVM, `overdrive serve` + `overdrive deploy`, same-node mesh client on a node with **no default route**, plus `ip route show table all` showing no prefix route | Mesh connection to the VM succeeds; no covering route exists | Any covering route, or failure |

S1-S4 also replace R5-43's reliance on a default route in test hosts (ADR-0188 Context: "Test hosts have a default route, so tests do not expose the dependency").

## 7. What it changes in the design

- **ADR-0188 (PREFIX-ROUTE-LO)**: withdrawn before acceptance; replaced by a new ADR for "leg-B marks and source-binds same-node VM dials". Per `.claude/rules/design.md` ("Do not manufacture amendment history before implementation") the replacement is written in present tense, not as an amendment chain. Needs user approval as a material decision.
- **ADR-0189 (INB-1-READ)**: unnecessary; INB-1 holds literally (no local or unicast route covers the prefix).
- **ADR-0172**: remove the Context paragraphs that depend on PREFIX-ROUTE-LO; the constant reject/drop rules stay and revert to protecting against non-diverted host and remote traffic. Not load-bearing against a prefix local-delivery route.
- **ADR-0125**: remove the proposed 2026-10-13 amendment note. Rule 7 (output divert) is unchanged; its tuple-set admission now also gates marked sockets.
- **ADR-0152 (GW-LOCAL)**: independent. A `gateway:53` association socket needs a route to the gateway for the same ENETUNREACH reason, so the gateway's `local` /32 stays a separate decision; the case-4a source mismatch disappears with the lo route.
- **ADR-0173 / ADR-0186**: unchanged (host platform clients still use the owner's vsock flow).
- **feature-delta**: delete the prefix-route boot step in the owner/provisioner section, `GUEST_PREFIX_ROUTE_PROTOCOL` (247), the typed guest-prefix route conflict, and the V-PREFIX-ROUTE entry; add the leg-B dial contract (mark value, `bind(127.0.0.1)`, the guest-prefix predicate) and V-LEGB-MARK (S1-S8). `HostMtlsEnforcement` needs the guest prefix as a required constructor input (not defaulted, `.claude/rules/development.md` port-trait rule). `dial_leg`'s signature is private; the observable contract (marked, bound, only for prefix dials) is what DESIGN pins.
- **Lifecycle Gate Ownership**: no gate added or moved; record `Not applicable` with this evidence.
- **Formal protocol model**: no concurrent protocol changes; `Not applicable`.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Linux v7.0 source (`vendors/linux`): `tcp_ipv4.c`, `route.h`, `route.c`, `fib_rules.c` (net/core + net/ipv4), `fib_semantics.c`, `fib_trie.c`, `datagram.c`, `af_inet.c`, `sock.c`, `filter.c`, `ip_output.c`, `nft_chain_route.c`, `netfilter.c` | vendored | High | Primary source | 2026-10-13 | Y (probe transcript for the K15 subset) |
| `findings-prefix-route-lo.md` (Linux 7.0.0-38 transcript) | local | High | Executed evidence | 2026-10-13 | Y (agrees with K5, K7, K9) |
| ADR-0125, 0152, 0172, 0173, 0188, 0189; feature-delta; audit-review rulings | local | High | Design record | 2026-10-13 | Partly (design claims checked against source) |
| Cilium `bpf/bpf_sock.c`, `pkg/proxy/routes.go`, `linux_defaults.go` | github (local checkout) | High | Production source | 2026-10-13 | N (single project) |
| kernel.org tproxy doc, sshuttle tproxy page | docs.kernel.org / readthedocs | High / Medium-High | Documentation | 2026-10-13 | Y (agree with each other) |
| Jimmy Song ambient-mesh L4 path, Firecracker vsock doc, Kata-dev thread | blogs / github / mailing list | Medium | Industry | 2026-10-13 | N |

Reputation: High 5 (about 62%) | Medium-high 1 | Medium 2 | Avg about 0.88.

## Knowledge Gaps

### Gap 1: how a remote node's VM is addressed from a client node's leg-B
**Issue**: whether `GUEST_NODE_PREFIX` (a single constant `100.95.0.0/16`) is per-node distinct and fabric-routed, or identical on every node. This decides whether "destination in the local guest prefix" is exactly the same-node case. **Attempted**: ADR-0120, 0172, 0185, 0188 and feature-delta greps; the peer-address resolution path was not located in the files read. **Recommendation**: DESIGN confirms the premise before pinning the discriminator; S5 covers the non-prefix side only.

### Gap 2: prior art for a NIC-less VM with a transparent same-node mesh dial, and Cilium's host-side non-local pod/service CIDR handling
**Issue**: no source found for either. **Attempted**: web searches for Istio ambient/ztunnel, Firecracker/Kata vsock, tproxy `ENETUNREACH` discussions; Cilium `ENETUNREACH` grep; gVisor and Cloudflare were not specifically searched. **Recommendation**: low value; the recommendation rests on the kernel source and the spike, not prior art.

### Gap 3: end-state equivalence of a pre-marked connect and the post-divert packet
**Issue**: the claim that both select the same table-100 route with the same source is inferred from `ip_route_me_harder`'s inputs (K12). **Attempted**: source reading only. **Recommendation**: S1.

### Gap 4: whether leg-B and leg-C share a process (needed to cost alternative C-prime)
**Issue**: not verified. **Attempted**: crate greps showed both live in the worker/dataplane mTLS code; process boundaries were not traced. **Recommendation**: confirm only if C-prime is pursued.

## Conflicting Information

### Conflict 1: is connect-time marking viable?
**Position A**: not viable; the source becomes the destination and remote dials would be marked: ADR-0188 "Alternatives considered". Source: local ADR, Evidence: `route.c:2848-2853`.
**Position B**: viable; the source is avoided by binding 127.0.0.1 (or a table-100 `src`), and marking is conditional on the destination. Source: kernel source, Evidence: `route.c:2849` (`if (!fl4->saddr)`), `route.c:2766-2770`, `fib_semantics.c:1324-1346`.
**Assessment**: ADR-0188's line citations are correct; its conclusion holds only for an unbound, unconditionally marked socket. B follows from the same source lines. S4(c) will confirm both halves.

## Recommendations for Further Research

1. Run S1-S8 on Linux 7.0 before the replacement ADR is surfaced for approval (contract-shaping, `.claude/rules/design.md`).
2. If S1 or S2 fails, run S6 (bind to `lo`) as the substitute, then reconsider D2 (dummy device) as in ADR-0188.
3. If the user wants the same-node TPROXY hop gone, scope C-prime as a separate DESIGN decision.

## Full Citations

[1] Linux kernel v7.0, `vendors/linux`: files listed in the Source Analysis table. Accessed 2026-10-13.
[2] Overdrive. `docs/feature/netns-density-295/spike/findings-prefix-route-lo.md`. Accessed 2026-10-13.
[3] Overdrive. ADR-0125, 0152, 0172, 0173, 0185, 0188, 0189; `docs/feature/netns-density-295/feature-delta.md` (:395-422, :6140-6158, :6400-6428, :7535-7560); `.context/over-engineering-audit-review.md`. Accessed 2026-10-13.
[4] Overdrive. `crates/overdrive-dataplane/src/mtls/mod.rs:643-688` (`dial_leg`, `dial_leg_s`); `crates/overdrive-netlink/src/nft.rs:218-239, :693-719`. Accessed 2026-10-13.
[5] Cilium. `bpf/bpf_sock.c`, `pkg/proxy/routes.go`, `pkg/datapath/linux/linux_defaults/linux_defaults.go`. Local checkout `/Users/marcus/git/cilium/cilium`. Accessed 2026-10-13.
[6] Linux kernel docs. "Transparent proxy support". https://docs.kernel.org/6.6/networking/tproxy.html. Accessed 2026-10-13.
[7] sshuttle. "TPROXY method". https://sshuttle.readthedocs.io/en/latest/tproxy.html. Accessed 2026-10-13.
[8] Jimmy Song. "Transparent Traffic Intercepting and Routing in the L4 Network of Istio Ambient Mesh". https://jimmysong.io/en/blog/ambient-mesh-l4-traffic-path/. Accessed 2026-10-13.
[9] Firecracker. "vsock". https://github.com/roypat/firecracker/blob/main/docs/vsock.md; Kata-dev. "vsock & network namespaces in Kata". https://lists.katacontainers.io/pipermail/kata-dev/2019-December/001149.html. Accessed 2026-10-13.
