> **Superseded in part.** The option "REFUSED" (O1) and the `10.99.0.0/16` mapping below were written before the design settled; accepted ADR-0359 chose the dnsmasq-shape blocked reply and guards `10.98.0.0/16`, the service VIP pool and `100.95.0.0/16`, not `10.99.0.0/16`.

# Research: How DNS forwarders and platform DNS handle upstream answers that point into private or platform-owned address space

**Date**: 2026-11-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium (High for dnsmasq/Unbound, Low-Medium for platform systems where the finding is "not documented") | **Sources**: 20

Scope: input to GH #382 / feature `guest-public-dns-resolution`, ADR-0359 (GPD-4: replace a whole forwarded reply with REFUSED when an A record or SVCB/HTTPS ipv4hint falls in an Overdrive-owned range). This document presents evidence and options. It does not make the design decision.

## Executive Summary

Two mature DNS components implement answer filtering by address: dnsmasq (`--stop-dns-rebind`) and Unbound (`private-address`). Both are about the same hazard, an upstream name resolving into address space the resolver's client treats as private. Neither is aimed at a platform's own service range specifically. dnsmasq defaults to off in the daemon and on in OpenWrt; Unbound defaults to an empty list. dnsmasq clears the whole reply (answer, authority and additional counts set to 0) on a hit. Unbound removes only the offending A/AAAA/SVCB/HTTPS records, and deliberately does not return SERVFAIL. Neither returns REFUSED. dnsmasq checks only A/AAAA in the answer section, following CNAME chains to their targets. Unbound checks all sections and also checks SVCB/HTTPS ipv4hint/ipv6hint.

Every platform with a "traffic to range X is intercepted" model that we could document (Istio DNS proxy, Consul transparent proxy, Fly 6PN, Tailscale) chose the range so that public DNS cannot plausibly return it (Class E 240.240.0.0/16, 240.0.0.0/4, ULA fdaa::/16, CGNAT 100.64.0.0/10). We found no documentation of any of them filtering upstream answers in the intercepted range. Kubernetes CoreDNS (`forward . /etc/resolv.conf`), Docker's embedded DNS, systemd-resolved, Cilium's DNS proxy and Route 53 Resolver document no such filter. The defence in the platforms we could read is range choice, not answer filtering. The one counter-signal: Consul's 240.0.0.0/4 choice has a vendor-acknowledged external conflict case, so range choice reduces the hazard but does not remove it.

## Research Methodology

**Search Strategy**: vendor manuals and source files fetched from upstream repos (dnsmasq `forward.c`/`rfc1035.c`, Unbound `iter_scrub.c`/`iter_priv.c`); vendor docs for platform systems; IANA registry. **Source Selection**: official docs and upstream source preferred. **Verification**: the dnsmasq and Unbound behaviours are each confirmed by manual plus source. Platform "no filter" findings are absence-of-documentation, not proof of absence (see Knowledge Gaps). Fetched content was treated as untrusted data; two fetches (openwrt.org) returned a bot-protection page and were replaced by a search summary. Where a fetch tool summarised a page, line numbers were unavailable; locations are given by function.

## Findings

### F1. dnsmasq: reject-and-clear on private-range answers; default off in the daemon, on in OpenWrt
**Evidence**: Manual: `--stop-dns-rebind` "Reject (and log) addresses from upstream nameservers which are in the private ranges." `--rebind-localhost-ok` "Exempt 127.0.0.0/8 and ::1 from rebinding checks", noting that range "is returned by realtime black hole servers". `--rebind-domain-ok` "Do not detect and block dns-rebind on queries to these domains." [1]
Source, `process_reply()` in `src/forward.c`: on `extract_addresses()` returning non-zero, `header->ancount`, `nscount` and `arcount` are set to 0; on return value 1 it logs "possible DNS-rebind attack detected" and sets EDE `EDE_BLOCKED`; only return value 2 (malformed) sets SERVFAIL. The RCODE is otherwise whatever upstream sent. [2]
Source, `extract_addresses()` in `src/rfc1035.c`: loops over `ancount` only (answer section); checks A (`F_IPV4`) and AAAA (`F_IPV6`) via `private_net()`/`private_net6()`; CNAMEs are not checked themselves but followed to the target; returns 1 "if we reject an address because it look like part of dns-rebinding attack". [3]
`private_net()` ranges: 0.0.0.0/8 and 127/8 (when localhost banned), 10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16, 192.0.2/24, 198.51.100/24, 203.0.113/24, 255.255.255.255; `private_net6()`: IPv4-mapped private, link-local, site-local, fd00::/8, 2001:db8::/32. [3]
**Action on hit**: whole reply emptied (all three section counts zeroed), NOERROR retained from upstream (so the client sees an empty NOERROR/NODATA-shaped reply), EDE "Blocked" added if the client used EDNS. Inference: client-visible result is "name exists, no addresses".
**Confidence**: High (manual + two source files, same project though; independent corroboration is F3). Note [1], [2], [3] are one project, so they count as one independent source family.
**Not documented**: DNSSEC interaction. The `secure` flag is not used by the rebind decision in the code we read [3]; `cache_secure` is cleared on a hit [2].

### F2. Unbound: strip matching records, never SERVFAIL, nothing on by default
**Evidence**: `private-address`: "These are addresses on your private network, and are not allowed to be returned for public internet names. Any occurrence of such addresses are removed from DNS answers." "The option removes resource records of types A, AAAA, SVCB and HTTPS that match the filter." The ipv4hint and ipv6hint svcparams are also checked. "Additionally, the DNSSEC validator may mark the answers bogus." "No private addresses are enabled by default." `private-domain` allows a domain and subdomains to contain private addresses. [4]
Source, `scrub_sanitize()` in `iterator/iter_scrub.c`: the check is applied to every rrset in the message (answer, authority and additional); comment: "do not set servfail since this leads to too many drops of other people using rfc1918 space" and "do not remove entire rrset, unless all records in it are bad". [5]
Source, `iterator/iter_priv.c`: no built-in range list, the tree comes from config; names with a private-domain match are exempt; SVCB/HTTPS handled by walking SvcParams and examining only ipv4hint/ipv6hint keys; no CNAME handling. [6]
**Action on hit**: the offending A/AAAA/SVCB/HTTPS records (not the reply) are removed; rcode unchanged. The upstream rationale for avoiding SERVFAIL is explicit in the source comment.
**Confidence**: High (manual + two source files; one project).

### F3. OpenWrt and Pi-hole: dnsmasq's switch, on by default on OpenWrt
**Evidence**: OpenWrt `rebind_protection` defaults to 1 and maps to `--stop-dns-rebind`; `rebind_localhost` maps to `--rebind-localhost-ok`; `rebind_domain` maps to `--rebind-domain-ok`. [7] (via search summary only; the OpenWrt wiki fetch returned a bot-wall page. The summary notes one older mirror gives a different `rebind_localhost` default.) Pi-hole's FTL DNS resolver page documents no rebind setting. [8]
**Confidence**: Medium (OpenWrt default from a search summary; Pi-hole is absence of documentation).

### F4. Kubernetes/CoreDNS, Docker, systemd-resolved: forward without documented address filtering
**Evidence**: Kubernetes default Corefile: non-cluster queries go to `forward . /etc/resolv.conf`; the page documents no answer filtering. [9] The CoreDNS `forward` plugin page documents options (`failover`, `next`, `except`) that act on rcodes and names, not addresses; no rebind plugin is mentioned. [10] Kubernetes ClusterIP docs describe the service range as a configured range with unique IPs per Service and say nothing on public answers landing in it. [11] Docker: "The embedded DNS server forwards external DNS lookups to the DNS servers configured on the host"; address 127.0.0.11; no filtering documented. [12] systemd-resolved man page documents no rebinding protection; it documents routing of non-matching queries to default-route link servers. [13]
**Confidence**: Medium. These are absences in the documentation fetched, not verified by reading source. CoreDNS's `kubernetes` plugin page was truncated at the tail in one fetch.

### F5. Istio DNS proxy: Class E VIPs chosen to avoid real services; upstream answers not documented as filtered
**Evidence**: "Istio will automatically allocate non-routable VIPs (from the Class E subnet)"; "These addresses will be picked from the `240.240.0.0/16` reserved IP address range to avoid conflicting with real services." Unhandled queries: "the request is forwarded upstream following the standard `/etc/resolv.conf` DNS configuration." The page does not mention filtering upstream answers in 240.240.0.0/16. [14] IANA marks 240.0.0.0/4 as not forwardable and not globally reachable. [15]
**Confidence**: Medium-High on the range-choice claim, Low on "no filter exists" (absence).

### F6. Consul transparent proxy: 240.0.0.0/4 virtual IPs, with a vendor-acknowledged conflict
**Evidence**: Consul assigns transparent-proxy service virtual IPs from 240.0.0.0/4 and resolves them at `<service>.virtual.consul`; a HashiCorp support article states the range is hardcoded and can conflict with external services using addresses in it, with workarounds of avoiding the range outside Consul or using explicit upstreams; a configurability request is hashicorp/consul#22595. [16] (search summaries only; the support article itself redirected and was not read.)
**Analysis**: this is the one documented case where a "choose a range public DNS won't return" defence was found to leak in practice, and the remedy offered is operator avoidance, not an answer filter.
**Confidence**: Low-Medium (secondary summary).

### F7. Fly.io: platform DNS at fdaa::3 over a ULA prefix; resolves arbitrary public names
**Evidence**: "The custom Fly.io DNS server is always available at the IPv6 address `fdaa::3`"; "we overwrite `/etc/resolv.conf`"; "This DNS server can resolve arbitrary DNS queries, so you can look up `google.com` with it." The 6PN prefix is `fdaa::/16`, described by Fly as ULA space "analogous to the 10-net space in IPv4". [17][18] Fly documents no answer filtering for fdaa::/16, and does not state why a ULA was chosen (the ULA properties are from the RFC concept, our inference).
**Confidence**: Medium.

### F8. Tailscale: CGNAT 100.64.0.0/10 chosen to avoid conflict with common private ranges
**Evidence**: Tailscale IPs come from "special-use IPv4 addresses from the `100.64.0.0/10` subnet", "reserved for ISP networks and routing equipment rather than private networks", chosen so they "don't conflict with IP addresses from subnets commonly used for private networks"; the page warns of conflicts with "other VPNs that use the same address space". [19] MagicDNS forwards non-MagicDNS queries to configured nameservers; filtering and rebind protection are not documented. [20] IANA: 100.64.0.0/10 is Forwardable True, Globally Reachable False (same as 10/8 on those two flags). [15]
**Confidence**: Medium-High for range choice, Low for "no filter".

### F9. AWS and Cilium: filter by name, not by answer address
**Evidence**: Route 53 Resolver DNS Firewall filters on domain names; the related Network Firewall "Resolved IP Validation" compares outbound connection destinations with addresses the Resolver returned, which is an egress check rather than an answer filter. [21] (search summaries.) Cilium's DNS proxy docs state the security model assumes intercepted DNS responses come from trusted cluster DNS servers, and warn that intercepting less-trusted resolvers may let them influence learned toFQDNs IPs; no private-IP filter documented. [22] (search summary.)
**Confidence**: Low-Medium.

## Comparison table

| System | Filters forwarded answers by address? | Default on? | Ranges | Action on hit | Sections / types checked | Source |
|---|---|---|---|---|---|---|
| dnsmasq `--stop-dns-rebind` | Yes | No (daemon); Yes on OpenWrt | Fixed built-in: RFC1918, 100.64/10, link-local, doc nets, loopback (opt-out), fd00::/8, v4-mapped | Whole reply emptied (an/ns/ar counts = 0), upstream rcode kept (NOERROR), EDE Blocked, logged | Answer section only; A and AAAA; CNAME followed, not checked; SVCB/HTTPS not checked | [1][2][3] |
| Unbound `private-address` | Yes | No (empty list) | Operator-configured; no built-in list | Only matching RRs removed (rrset dropped if all bad); rcode unchanged; deliberately no SERVFAIL; DNSSEC validator may mark bogus | All sections; A, AAAA, SVCB, HTTPS (ipv4hint/ipv6hint); CNAME not checked; `private-domain` exempts | [4][5][6] |
| OpenWrt (dnsmasq) | Yes | Yes (`rebind_protection=1`) | As dnsmasq | As dnsmasq | As dnsmasq | [7] |
| Pi-hole FTL | Not documented | n/a | n/a | n/a | n/a | [8] |
| CoreDNS / Kubernetes DNS (`forward . /etc/resolv.conf`) | Not documented | n/a | Service CIDR example 10.96.0.0/16 | n/a | n/a | [9][10][11] |
| systemd-resolved | Not documented | n/a | n/a | n/a | n/a | [13] |
| Docker embedded DNS (127.0.0.11) | Not documented | n/a | n/a | n/a | n/a | [12] |
| Istio DNS proxy | Not documented; defence is range choice | n/a | 240.240.0.0/16 (Class E, not globally reachable) | n/a | n/a | [14][15] |
| Consul transparent proxy | Not documented; range choice (with known conflict case) | n/a | 240.0.0.0/4 | n/a | n/a | [16] |
| Fly.io (fdaa::3) | Not documented; range choice | n/a | fdaa::/16 ULA | n/a | n/a | [17][18] |
| Tailscale MagicDNS | Not documented; range choice | n/a | 100.64.0.0/10 | n/a | n/a | [19][20] |
| AWS Route 53 Resolver | Name filter only (DNS Firewall); separate egress IP validation | Opt-in | n/a | block/alert on query name | query names | [21] |
| Cilium DNS proxy | Not documented; trust-model caveat | n/a | n/a | n/a | n/a | [22] |
| Firecracker / Kata / gVisor | Nothing found | n/a | n/a | n/a | n/a | Knowledge Gap 3 |

## Synthesis: dominant patterns

1. **Pattern A, reply-level or record-level filtering of "private" answers** (dnsmasq, Unbound, OpenWrt). Aimed at DNS rebinding of the client's own LAN. Ranges are generic private space (dnsmasq built-in; Unbound operator-set). Action is silent mutation, not REFUSED: dnsmasq empties the reply and keeps the rcode, Unbound removes records and refuses to SERVFAIL. Both document an exemption mechanism for split-horizon (`rebind-domain-ok`, `private-domain`).
2. **Pattern B, range choice as the defence** (Istio 240.240/16, Consul 240/4, Fly fdaa::/16, Tailscale 100.64/10). Platform-owned intercepted ranges sit in space public DNS should not return. We found no documented answer filter on top. Consul's own support guidance shows the choice can still collide with external services.
3. **Pattern C, no address filtering at all** (CoreDNS forward, Docker, systemd-resolved, Cilium, Route 53 DNS Firewall): trust the upstream and rely on network policy or name rules.
4. **Coverage varies and gaps are real**: only Unbound checks SVCB/HTTPS hints and non-answer sections; dnsmasq checks answer-section A/AAAA only; nobody documented checking CNAME targets as addresses (CNAMEs resolve to A records in the same answer section anyway, which both tools do check).
5. **No system found uses REFUSED for this**, and none filters a platform's own service/VIP range specifically (Kubernetes service CIDR included). GPD-4's "replace whole reply with REFUSED" and "owned-range list" therefore have no direct precedent; the closest is dnsmasq's whole-reply clearing. This is an observation, not a recommendation.

## Mapping onto Overdrive's four candidate ranges

Facts from ADR-0359 as read: owned set = 10.98.0.0/16 (mesh frontend), 10.99.0.0/16 (workload), 100.95.0.0/16 (guest prefix), service VIP pool. The ADR itself says the node treats addresses in these as frontend, platform service, or guest, and that real upstreams legitimately serve other private ranges (split-horizon). The remainder is inference.

| Range | Falls in a generic "private" set? | Does public DNS plausibly return it? | Is there an aliasing failure if it does? | Covered by another mechanism? (inference) |
|---|---|---|---|---|
| 10.98.0.0/16 (frontend) | Yes, RFC1918: dnsmasq/OpenWrt block it, Unbound only if configured | Not by honest public names, but split-horizon and attacker-controlled names (the rebinding threat model) can return any 10.x | Yes, real: guest TCP to it is steered as mesh traffic, and the ADR names this as the hazard | Only by the filter, or by a dnsmasq-style blanket-private rule which AC-7 rejects |
| 10.99.0.0/16 (workload subnet) | Yes, RFC1918 | Same as above | Real if guest traffic to it is treated as workload traffic by the platform; whether routed or steered is not established in this research | Unknown; needs the owning dataplane ADR, not researched here |
| 100.95.0.0/16 (guest prefix) | Yes, inside CGNAT 100.64/10: dnsmasq blocks all of 100.64/10; Tailscale occupies the same block | Rare in public DNS but not reserved against it: IANA lists 100.64/10 as forwardable, not globally reachable. Documented overlap with Tailscale/ISP-CGNAT use | Real as the ADR states (guest addresses), strength depends on how guest-to-guest traffic is handled | Range choice (Pattern B) is the only alternative and has a documented conflict class (Tailscale warning, Consul precedent) |
| Service VIP pool | Depends on the configured pool (not read in this research) | Depends on pool | Real per the ADR (guest TCP to a VIP resolves as mesh, ADR-0164) | If the pool were moved to Class E (Istio/Consul practice) or another not-globally-reachable block, filtering becomes defence in depth rather than the only barrier; this is an option, not a recommendation |

Options evidence supports (not a decision):
- **O1 Filter owned ranges, whole-reply REFUSED** (GPD-4 as written). Precedent for whole-reply neutralisation: dnsmasq. No precedent for REFUSED; client-visible effect of REFUSED vs dnsmasq's empty NOERROR is not documented in any source read.
- **O2 Filter owned ranges, strip offending records** (Unbound behaviour). Precedent explicitly avoids SERVFAIL; stripping leaves partial answers and under DNSSEC the validator "may mark the answers bogus" [4].
- **O3 Range choice only** (Pattern B). Zero per-query cost, but the only documented example with a known conflict (Consul) is the closest analogue, and 10.98/10.99 are RFC1918, the space real split-horizon upstreams use.
- **O4 Combine O1/O2 with a re-homed range** for the pool/prefix where the allocation can still move. Requires owning allocations to be changeable; not evidenced here.

## Source Analysis

| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| [1] dnsmasq man page | thekelleys.org.uk | High (upstream) | official | 2026-11-25 | Y ([2][3]) |
| [2] dnsmasq forward.c | raw.githubusercontent.com/imp/dnsmasq (mirror) | Medium-High | source | 2026-11-25 | Y ([3]) |
| [3] dnsmasq rfc1035.c | same | Medium-High | source | 2026-11-25 | Y |
| [4] unbound.conf | unbound.docs.nlnetlabs.nl | High | official | 2026-11-25 | Y ([5][6]) |
| [5] iter_scrub.c | raw.githubusercontent.com/NLnetLabs/unbound | High | source | 2026-11-25 | Y |
| [6] iter_priv.c | same | High | source | 2026-11-25 | Y |
| [7] OpenWrt DHCP/DNS (search summary) | openwrt.org | Medium | official, indirect | 2026-11-25 | N |
| [8] Pi-hole FTL resolver | docs.pi-hole.net | Medium-High | official | 2026-11-25 | N |
| [9] K8s custom DNS | kubernetes.io | High | official | 2026-11-25 | Y ([10]) |
| [10] CoreDNS forward | coredns.io | High | official | 2026-11-25 | Y |
| [11] K8s ClusterIP allocation | kubernetes.io | High | official | 2026-11-25 | N |
| [12] Docker networking | docs.docker.com | High | official | 2026-11-25 | N |
| [13] systemd-resolved | freedesktop.org | High | official | 2026-11-25 | N |
| [14] Istio DNS proxy | istio.io | High | official | 2026-11-25 | N |
| [15] IANA IPv4 special registry | iana.org | High | official | 2026-11-25 | N |
| [16] Consul tproxy / HashiCorp support (search summary) | developer.hashicorp.com, support.hashicorp.com | Medium | official, indirect | 2026-11-25 | N |
| [17] Fly private networking | docs.fly.io | High | official | 2026-11-25 | Y ([18]) |
| [18] Fly 6PN blog/community (search summary) | fly.io | Medium-High | official blog | 2026-11-25 | Y |
| [19] Tailscale 100.x | tailscale.com | Medium-High | vendor | 2026-11-25 | N |
| [20] Tailscale MagicDNS | tailscale.com | Medium-High | vendor | 2026-11-25 | N |
| [21] AWS Route 53 Resolver docs/blog (search summary) | docs.aws.amazon.com | High | official, indirect | 2026-11-25 | N |
| [22] Cilium L7 policy / DNS docs (search summary) | docs.cilium.io | High | official, indirect | 2026-11-25 | N |

Reputation: High ~65%, Medium-high ~25%, Medium ~10%.

## Knowledge Gaps

1. **Platform "no filtering" claims are absence-of-documentation.** Attempted: docs for CoreDNS, Kubernetes, Docker, systemd-resolved, Istio, Fly, Tailscale, Cilium, AWS. Not checked: source code of istiod/pilot-agent DNS proxy, CoreDNS plugin list (external `rebind`-style plugins), tailscaled's DNS forwarder, Fly's resolver (closed source). Recommendation: read Istio `pilot/pkg/dns`/agent DNS proxy and tailscaled `net/dns/resolver` before stating that no one filters.
2. **Client-visible behaviour of REFUSED vs empty NOERROR vs NXDOMAIN on a blocked answer** (resolver retry/fallback to the next nameserver, glibc/musl/systemd-resolved behaviour) was not sourced. Docker documents that its embedded DNS stops at success or NXDOMAIN [12], which is a hint that rcode choice changes retry behaviour but is not a statement about guest stub resolvers.
3. **Firecracker, Kata Containers, gVisor**: nothing relevant found (these do not ship a DNS forwarder; searched only by reasoning on their scope, no dedicated fetch). Recommend recording as "not applicable".
4. **DNSSEC implications** are documented only by Unbound's one-sentence note [4] (validator may mark answers bogus) and dnsmasq's clearing of `cache_secure` [2]. How a validating guest stub treats an emptied or REFUSED reply was not sourced.
5. **Consul 240.0.0.0/4 conflict** and **OpenWrt default** rely on search summaries; the support article redirected to a login page.
6. **Overdrive's VIP pool range and whether 10.99.0.0/16 is steered or merely routed** were not read from the repository; the mapping table marks these as unknown.
7. Linkerd was not researched (it does not intercept DNS answers or allocate a VIP range as far as we know, unverified).

## Conflicting Information

- OpenWrt `rebind_localhost` default: wiki mirror says 1, an older mirror says 0 [7]. Not material here.
- No other conflicts found.

## Recommendations for Further Research

1. Read Istio's DNS proxy and tailscaled's resolver source for any answer-range check (closes Gap 1).
2. Test, on the Overdrive guest images, glibc/musl behaviour on REFUSED vs empty NOERROR vs NXDOMAIN (closes Gap 2); this is a spike, not a desk study.
3. Confirm from the owning ADRs how guest TCP to 10.99.0.0/16, 100.95.0.0/16 and the VIP pool is handled, to settle the "real aliasing failure" column.

## Full Citations

[1] Kelley, S. "dnsmasq man page". https://thekelleys.org.uk/dnsmasq/docs/dnsmasq-man.html. Accessed 2026-11-25.
[2] dnsmasq `src/forward.c`, `process_reply()`. https://raw.githubusercontent.com/imp/dnsmasq/master/src/forward.c. Accessed 2026-11-25.
[3] dnsmasq `src/rfc1035.c`, `extract_addresses()`, `private_net()`, `private_net6()`. https://raw.githubusercontent.com/imp/dnsmasq/master/src/rfc1035.c. Accessed 2026-11-25.
[4] NLnet Labs. "unbound.conf". https://unbound.docs.nlnetlabs.nl/en/latest/manpages/unbound.conf.html. Accessed 2026-11-25.
[5] NLnet Labs. `iterator/iter_scrub.c`. https://raw.githubusercontent.com/NLnetLabs/unbound/master/iterator/iter_scrub.c. Accessed 2026-11-25.
[6] NLnet Labs. `iterator/iter_priv.c`. https://raw.githubusercontent.com/NLnetLabs/unbound/master/iterator/iter_priv.c. Accessed 2026-11-25.
[7] OpenWrt. "DHCP and DNS configuration /etc/config/dhcp". https://openwrt.org/docs/guide-user/base-system/dhcp (accessed via search summary 2026-11-25).
[8] Pi-hole. "FTLDNS DNS resolver". https://docs.pi-hole.net/ftldns/dns-resolver/. Accessed 2026-11-25.
[9] Kubernetes. "Customizing DNS Service". https://kubernetes.io/docs/tasks/administer-cluster/dns-custom-nameservers/. Accessed 2026-11-25.
[10] CoreDNS. "forward plugin". https://coredns.io/plugins/forward/. Accessed 2026-11-25.
[11] Kubernetes. "Service ClusterIP allocation". https://kubernetes.io/docs/concepts/services-networking/cluster-ip-allocation/. Accessed 2026-11-25.
[12] Docker. "Networking overview". https://docs.docker.com/engine/network/. Accessed 2026-11-25.
[13] systemd. "systemd-resolved.service". https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html. Accessed 2026-11-25.
[14] Istio. "DNS Proxying". https://istio.io/latest/docs/ops/configuration/traffic-management/dns-proxy/. Accessed 2026-11-25.
[15] IANA. "IANA IPv4 Special-Purpose Address Registry". https://www.iana.org/assignments/iana-ipv4-special-registry/iana-ipv4-special-registry.xhtml. Accessed 2026-11-25.
[16] HashiCorp. "Enable transparent proxy mode on VMs"; "Virtual IP in the range 240.0.0.0/4 may conflict with external services IP addresses in the same range". https://developer.hashicorp.com/consul/docs/connect/proxy/transparent-proxy/vm ; https://support.hashicorp.com/hc/en-us/articles/44335990566163-Virtual-IP-in-the-range-240-0-0-0-4-may-conflict-with-external-services-IP-addresses-in-the-same-range (search summary only). Accessed 2026-11-25.
[17] Fly.io. "Private networking". https://docs.fly.io/networking/private-networking. Accessed 2026-11-25.
[18] Fly.io. "Incoming! 6PN Private Networks". https://fly.io/blog/incoming-6pn-private-networks/ (search summary). Accessed 2026-11-25.
[19] Tailscale. "What are these 100.x.y.z addresses?". https://tailscale.com/kb/1015/100.x-addresses. Accessed 2026-11-25.
[20] Tailscale. "MagicDNS". https://tailscale.com/kb/1081/magicdns. Accessed 2026-11-25.
[21] AWS. "How Route 53 Resolver DNS Firewall works"; "Introducing Route 53 Resolved IP Validation for AWS Network Firewall". https://docs.aws.amazon.com/Route53/latest/DeveloperGuide/resolver-dns-firewall-overview.html ; https://aws.amazon.com/blogs/security/introducing-route-53-resolved-ip-validation-for-aws-network-firewall/ (search summary). Accessed 2026-11-25.
[22] Cilium. "Layer 7 Policies" / "Locking Down External Access with DNS-Based Policies". https://docs.cilium.io/en/latest/security/policy/layer7/ ; https://docs.cilium.io/en/stable/security/dns/ (search summary). Accessed 2026-11-25.
[Repo] `docs/product/architecture/adr-0359-a-forwarded-reply-carrying-an-overdrive-owned-address-is-emptied-whole-and-marked-blocked.md` (Context and Alternatives, lines 5-29).

## Research Metadata
Output: docs/research/dataplane/dns-forwarder-answer-filtering-owned-ranges-research.md | Confidence: High 35%, Medium 45%, Low 20% | Tool failures: openwrt.org (bot wall, x2), HashiCorp support (redirect), AWS DNS Firewall rule-group page (empty content), Kubernetes DNS concepts page (truncated); replaced by search summaries and noted above.
