# Research: How DNS forwarders surface broken or missing upstream configuration

**Date**: 2026-11-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium | **Sources**: 17

## Executive Summary

Question under study: the Overdrive node DNS responder (feature `guest-public-dns-resolution`, #382, ADR-0358) forwards guest non-mesh queries to the resolvers in the host `/etc/resolv.conf`, re-read per forwarded query. The draft refused boot when the file exists but is unreadable. Issue #376 argues startup checks on an immutable appliance verify things that cannot change and that first real use reports the failure anyway. If there is no startup check, how would an operator know?

Documented behaviour of the surveyed systems splits in two. Systems that read the resolver file once at startup (CoreDNS `forward`; Docker's daemon when building container resolv.conf) fail or degrade at config-parse time. Systems that poll or re-read at runtime (dnsmasq, systemd-resolved) log a warning and keep running; none of the surveyed runtime-reading systems exit on a bad file. For a dead upstream, all surveyed forwarders degrade per query (CoreDNS returns SERVFAIL, systemd-resolved fails the lookup) and expose it via counters, logs or a status command rather than by exiting.

The platform-level pattern (Kubernetes probes and node conditions; Google SRE monitoring guidance) is: do not restart or refuse for a dependency problem. Report it through a readiness/condition-style signal plus metrics, and alert on user-visible symptoms. This document presents option shapes mapped to Overdrive's two failure cases and does not choose among them.

Caveat: Overdrive-specific facts (ADR-0358 text, issue #376 text) were taken from the task statement, not re-read; no `gh` access in this run.

## Research Methodology
**Search Strategy**: Official project docs fetched directly; upstream source files (CoreDNS, dnsmasq, moby) read via raw GitHub for behaviours docs omit. **Source Selection**: official docs and upstream source = High; third-party search results not cited as facts. **Verification**: single authoritative source per system (acceptable per project rule); cross-system claims (readiness semantics) have 2 sources. Fetch tool summarises pages with a small model; quotes below are as returned by that tool, so exact wording should be re-checked on the cited page before being reproduced in an ADR.

## Findings

### Finding 1: CoreDNS `forward` reads resolv.conf once at startup and fails config load on an unusable file
**Documented**: `forward` "If not specified, the system resolver (`/etc/resolv.conf`) is used" (resolver option context); the docs say nothing on missing/unreadable/empty file. Source: [CoreDNS forward plugin](https://coredns.io/plugins/forward/).
**Source code (primary)**: `HostPortOrFile` calls `dns.ClientConfigFromFile`; a missing or unreadable file returns `not an IP address or file %q: %w`; a file with no `nameserver` lines yields `ErrNoNameservers` ("no nameservers found"). Source: [plugin/pkg/parse/host.go](https://raw.githubusercontent.com/coredns/coredns/master/plugin/pkg/parse/host.go). Setup rejects zero upstreams with "no valid upstream addresses found" ([plugin/forward/setup.go](https://raw.githubusercontent.com/coredns/coredns/master/plugin/forward/setup.go); the path from file-read to that error is not shown in the excerpt read).
**Inference**: setup-time errors in CoreDNS abort Corefile load, i.e. startup refusal; this is inferred from Go setup-function convention, not stated on the pages read. The `reload` plugin watches the Corefile and imports only (SHA512 checksum), not resolv.conf, and keeps the old config with an error log if the new one fails ([reload](https://coredns.io/plugins/reload/)). So CoreDNS = validate once at startup, no live re-read of resolv.conf.
**Confidence**: Medium (source read, but startup-abort inferred).

### Finding 2: CoreDNS dead upstreams: health checks, SERVFAIL, "broken" metric
**Evidence**: health check "at a 0.5s interval for as long as the upstream reports unhealthy"; any non-network-error reply (even REFUSED/SERVFAIL) counts healthy; `max_fails` default 2 ("0" disables marking down); "When all upstreams are down it assumes health checking as a mechanism has failed and will try to connect to a random upstream"; `failfast_all_unhealthy_upstreams` returns SERVFAIL immediately. Metrics: `coredns_forward_healthcheck_broken_total` (all-unhealthy case), `coredns_proxy_healthcheck_failures_total{proxy_name="forward"}`, `coredns_proxy_request_duration_seconds`; older `coredns_forward_healthcheck_failures_total` deprecated. Source: [forward](https://coredns.io/plugins/forward/).
**Code**: ServeDNS returns `dns.RcodeServerFailure` with `ErrNoHealthy` or the last upstream error; the excerpt has no log call on that path; observability is the `healthcheckBrokenCount` metric and optional dnstap. Source: [plugin/forward/forward.go](https://raw.githubusercontent.com/coredns/coredns/master/plugin/forward/forward.go).
**Confidence**: High (docs + code agree).

### Finding 3: CoreDNS readiness and loop detection
**Evidence**: `ready` serves `/ready` (default `:8181`), 200 "OK", 503 listing not-ready plugins; modes `until-ready` (default) and `continuously` ([ready](https://coredns.io/plugins/ready/)). `loop` sends a probe query for up to 30 s at startup and halts the process on detecting a forwarding loop; docs warn `forward` over a resolv.conf with local addresses can loop ([loop](https://coredns.io/plugins/loop/)). The docs read do not say `ready` reflects upstream health; the forward docs list no readiness hook. **Inference**: forward upstream health is not tied to `/ready`.
**Confidence**: Medium.

### Finding 4: dnsmasq re-reads resolv.conf by polling; bad/empty file = warn once and retry
**Documented**: `--resolv-file` replaces /etc/resolv.conf as the upstream source; `--no-poll` "Don't poll /etc/resolv.conf for changes"; with several files "the file with the currently latest modification time is the one used"; `--clear-on-reload` clears cache on re-read ([dnsmasq man](https://dnsmasq.org/docs/dnsmasq-man.html)). The page text read did not cover the no-servers case (truncated).
**Source (primary)**: `poll_resolv()` logs `reading %s` (LOG_INFO) on success; on a changed file with no usable servers logs once `no servers found in %s, will retry` (LOG_WARNING), sets mtime to 0 so the next poll retries; `stat` failure logs `failed to access %s: %s`. None exits. Source: [dnsmasq.c (mirror imp/dnsmasq)](https://raw.githubusercontent.com/imp/dnsmasq/master/src/dnsmasq.c). The tool reported only a missing resolv-file *directory* as fatal at startup (`EVENT_RESOLV_ERR`).
**Gap**: client-visible reply when no servers exist (REFUSED/SERVFAIL) not verified here.
**Confidence**: Medium (primary source via a third-party mirror).

### Finding 5: systemd-resolved: fallback servers; fails the query if no server; reload on SIGHUP
**Evidence**: queries going to no configured server use "one of the compiled-in fallback DNS servers"; if none suitable, "the unicast DNS query fails, as no suitable DNS servers can be determined"; SIGHUP reloads config; SIGUSR1 dumps caches and learned server-feature state to the system logs; it "will continuously talk to the same server ... until an error occurs" then switches to the next ([systemd-resolved(8)](https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html)). `FallbackDNS=` is used only when no other DNS information exists, else a compiled-in list; `DNS=` falls back to `/etc/resolv.conf` if it exists and lists servers ([resolved.conf(5)](https://www.freedesktop.org/software/systemd/man/latest/resolved.conf.html)). The pages read do not describe `resolvectl status` output or journal messages (gap).
**Confidence**: Medium.

### Finding 6: Docker: host resolv.conf read when the container resolv.conf is built; public-DNS fallback is source-documented, not doc-documented
**Evidence**: "The embedded DNS server forwards external DNS lookups to the DNS servers configured on the host" (embedded address 127.0.0.11); "--dns=127.0.0.1 refers to the container's own loopback address" ([Docker networking overview](https://docs.docker.com/engine/network/)). Fallback in source: `defaultIPv4NSs` = 8.8.8.8, 8.8.4.4, "Fallback nameservers, to use if none can be obtained from the host or command line options"; loopback filtering in `TransformForLegacyNw` logs at Info "No non-localhost DNS nameservers are left in resolv.conf. Using default external servers"; `Load(path)` returns the open error for a missing file rather than substituting defaults ([moby internal/resolvconf](https://raw.githubusercontent.com/moby/moby/master/daemon/libnetwork/internal/resolvconf/resolvconf.go)). Docker's public pages read do not mention 8.8.8.8 (the bridge-network page and the networking overview both lack it); a web search surfaced third-party pages and an open docs issue asking for the fallback to be documented, not cited as fact here.
**Inference**: the substitution is a silent-ish degrade (Info log, comment line in the generated file), and sends container queries to a third party the operator did not choose. For Overdrive this is a property to avoid or deliberately accept.
**Confidence**: Medium-High (source).

### Finding 7: Unbound exposes counters and an infra cache; startup behaviour not verified
**Evidence**: `unbound-control status` (exit 3 if not running), `stats`/`stats_noreset`, `num.answer.rcode.*`, `num.answer.bogus`, `threadX.requestlist.exceeded`, `dump_infra` (EDNS/ping/lameness per address), `flush_infra` ([unbound-control](https://nlnetlabs.nl/documentation/unbound/unbound-control/)). `unbound-checkconf` validates config offline, exits 1 on error ([unbound-checkconf](https://nlnetlabs.nl/documentation/unbound/unbound-checkconf/)). Unbound is mostly a recursive resolver, not a forwarder over resolv.conf by default; whether the daemon refuses on invalid config is not stated in the pages read.
**Confidence**: Low-Medium.

### Finding 8: Consul recursors: static list, per-query timeout, no documented startup validation
**Evidence**: `recursors` = upstream servers for names outside Consul's domain; `recursor_strategy` default `sequential`; `recursor_timeout` default 2s; docs read state no startup validation and no behaviour for unset `recursors` ([Consul DNS config](https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/dns)).
**Confidence**: Low (negative evidence only).

### Finding 9: Kubernetes readiness vs liveness vs startup; node conditions
**Evidence**: readiness failure removes the Pod IP from matching EndpointSlices; liveness failure restarts the container; startup probe gates the others. The concepts page: "When your app has a strict dependency on back-end services, you can implement both a liveness and a readiness probe. The liveness probe passes when the app itself is healthy, but the readiness probe additionally checks that each required back-end service is available"; incorrect liveness probes "can lead to cascading failures" ([Kubernetes probes](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/)). Node `conditions` (`Ready`, `DiskPressure`, `MemoryPressure`, `PIDPressure`, `NetworkUnavailable`) are the documented channel for node-level problems, and the control plane turns them into taints ([Node status](https://kubernetes.io/docs/reference/node/node-status/)).
**Confidence**: High for probe semantics (2 pages: probes concept page; the task page's liveness text agrees); Medium for the node-condition analogy (inference).

### Finding 10: SRE guidance: alert on symptoms; logs are white-box detail
**Evidence**: four golden signals (latency, traffic, errors, saturation); "what's broken, and why?"; focus on symptoms, worry about causes only if "very definite, very imminent"; logs pair with dashboards for historical correlation; pages for urgent issues ([Google SRE book ch. 6](https://sre.google/sre-book/monitoring-distributed-systems/)). The pages read give no direct "startup validation vs runtime health" or "fail fast vs degrade" guidance (gap).
**Confidence**: Medium.

## Comparison Table

| System | Startup validation of upstream config | Runtime failure behaviour to client | Signals exposed | Config reload model | Source |
|---|---|---|---|---|---|
| CoreDNS `forward` | Parses file at config load; missing/unreadable -> error; no nameserver lines -> "no nameservers found"; setup error aborts load (abort inferred) | All upstreams down: tries a random one (default) or SERVFAIL immediately with `failfast_all_unhealthy_upstreams`; final return SERVFAIL | Metrics: `coredns_forward_healthcheck_broken_total`, `coredns_proxy_healthcheck_failures_total`, request duration; `/ready` exists but not documented as upstream-aware; no log call on no-healthy path | Read once at startup; `reload` watches Corefile+imports only | [forward](https://coredns.io/plugins/forward/), [host.go](https://raw.githubusercontent.com/coredns/coredns/master/plugin/pkg/parse/host.go), [forward.go](https://raw.githubusercontent.com/coredns/coredns/master/plugin/forward/forward.go), [reload](https://coredns.io/plugins/reload/) |
| CoreDNS `loop` | Probe at startup; halts process on detected loop | n/a | Process exit + log | Startup only | [loop](https://coredns.io/plugins/loop/) |
| dnsmasq | Warns, does not exit (empty file: "no servers found in %s, will retry"); only missing resolv-file dir is fatal | Not verified here | Syslog: `reading %s`, `no servers found ... will retry` (once per failure episode), `failed to access`; SIGUSR1 stats | Polls file mtime (disable with `--no-poll`); optional cache clear | [man](https://dnsmasq.org/docs/dnsmasq-man.html), [dnsmasq.c](https://raw.githubusercontent.com/imp/dnsmasq/master/src/dnsmasq.c) |
| systemd-resolved | None that refuses; uses compiled-in fallback servers if none | If no suitable server: query fails; on server error switches to next | `resolvectl` (output not verified), SIGUSR1 dump to journal | SIGHUP reload; cache flushed on network change | [resolved](https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html), [resolved.conf](https://www.freedesktop.org/software/systemd/man/latest/resolved.conf.html) |
| Docker embedded DNS | File read when container resolv.conf is generated; missing file -> error from `Load`; loopback-only host list -> substitute 8.8.8.8/8.8.4.4 | Forwards to host servers (or the fallback) | Daemon Info log on fallback; warning comment inside generated resolv.conf | At container (re)configuration, not per query | [Docker docs](https://docs.docker.com/engine/network/), [moby source](https://raw.githubusercontent.com/moby/moby/master/daemon/libnetwork/internal/resolvconf/resolvconf.go) |
| Unbound | Offline `unbound-checkconf`; daemon refuse-on-bad-config not verified | Not verified | `unbound-control status/stats/dump_infra`, rcode and requestlist counters | Not verified | [control](https://nlnetlabs.nl/documentation/unbound/unbound-control/), [checkconf](https://nlnetlabs.nl/documentation/unbound/unbound-checkconf/) |
| Consul recursors | None documented | `recursor_timeout` 2s per upstream; failover not documented | Not documented on page read | Static config | [Consul](https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/dns) |
| Kubernetes NodeLocal DNSCache / kube-dns | Not found (page fetch truncated before health/metrics sections) | Not found | CoreDNS metrics per node (stated) | Not found | [NodeLocal](https://kubernetes.io/docs/tasks/administer-cluster/nodelocaldns/) |
| Nomad, Fly | Not researched / no source found | | | | gap |

## Synthesis of dominant patterns

1. **Read-once forwarders validate loudly at startup; read-per-use or polling resolvers do not refuse.** CoreDNS (read once) errors at config load; dnsmasq (polls) warns once and retries; systemd-resolved substitutes fallbacks or fails the query. No surveyed system that re-reads at runtime exits on a bad file. Overdrive's per-query re-read is the polling/runtime-read family, and its refusal-on-boot draft was the odd one out (inference from this table).
2. **A dead upstream degrades per query, with a protocol error to the client.** CoreDNS returns SERVFAIL; systemd-resolved fails the lookup; none exit.
3. **Signals are counters plus logs, not liveness failures.** CoreDNS `..._healthcheck_broken_total` and `proxy_healthcheck_failures_total`; dnsmasq syslog lines deduplicated to one per failure episode; Unbound counters and a status CLI. Log-once-per-episode (dnsmasq `warned` flag) avoids per-query log floods.
4. **Silent substitution is the documented trap.** Docker's public-DNS fallback is visible only as an Info log and is absent from the doc pages read; CoreDNS's random-upstream behaviour when all are marked down is similar "keep trying" degradation.
5. **Platform convention separates readiness from liveness**: dependency problems go into readiness/conditions (traffic removal, visibility), not liveness (restart) or refusal, and SRE guidance favours symptom alerts over cause alerts; logs serve diagnosis.

## Options mapped onto Overdrive's cases (no decision)

Cases: (A) `/etc/resolv.conf` unreadable or no nameserver lines; (B) all listed upstreams dead or timing out. Guest-visible reply is SERVFAIL in the CoreDNS/systemd-resolved precedent for both; on the appliance the file is part of the fixed image, so (A) reflects an image defect or runtime damage, not operator error (task premise).

| Option | Case A | Case B | Evidence for | Evidence against / cost |
|---|---|---|---|---|
| O1. Boot refusal (current draft) | `serve` exits | not applicable | CoreDNS fails config load on the same condition | No surveyed per-use re-reading system does this; takes the node's mesh and mTLS down for a public-DNS defect (inference: DNS responder is one component of `serve`); #376 argues the check verifies an image invariant |
| O2. Logs only | Warn once per failure episode (dnsmasq pattern); guest gets SERVFAIL | Same, throttled | Cheapest; matches dnsmasq | No operator shell on appliance, so logs must be shipped or readable via a CLI to be actionable; Google SRE page names logs as diagnosis, not alerting |
| O3. Counter/metric | Counter of forwarded queries failed by cause (config unreadable, no nameservers, upstream timeout) | Same, plus a "all upstreams failed" counter | CoreDNS `healthcheck_broken_total`, Unbound `num.*` counters; symptom alerting per SRE | Needs a metrics consumer; counters do not say "currently broken" without rate logic |
| O4. Status field readable by the operator CLI | Last-forward outcome and last config-read error in a node status record | Same, with last-success time | `unbound-control status`, `resolvectl`, Node `conditions` as the node-problem channel | New surface; per the repo rules, new public API needs a design decision and "agents propose" approval |
| O5. Health condition (degraded, not refused) | Condition `DnsForwardingDegraded` true with reason | Same | Kubernetes readiness: dependency checks live in readiness, liveness stays about the app itself; node conditions | Overdrive has no pinned condition surface in this research; do not tie to restart or refusal |
| O6. Silent public-DNS substitution (Docker-style) | Use fixed public resolvers | n/a | Docker precedent | Sends guest queries to a third party the operator did not choose; hides the defect (Info log only) |

Combinations present in the precedents: dnsmasq = O2; CoreDNS = O3 (plus O1 for the read-once case); Kubernetes = O5 (+O3 from the control plane).

## Knowledge Gaps

- **Gap 1**: CoreDNS startup abort on forward setup error is inferred; `classifyToAddrs`/`expandAndDedup` bodies and the Corefile-load abort were not read. Recommendation: read `plugin/forward/setup.go` fully and CoreDNS `caddy`/`plugin` startup, or run a probe.
- **Gap 2**: dnsmasq client reply when it has no servers; Unbound startup behaviour with bad config and forwarder (`forward-zone`) semantics; `resolvectl status` and journal wording; Consul behaviour with unset `recursors` and failover; kube-dns / NodeLocal DNSCache health and failure behaviour (page truncated); Nomad and Fly (no sources searched).
- **Gap 3**: Docker's fallback is absent from the Docker doc pages fetched; established only from source. A search result suggested a docs issue exists; not verified.
- **Gap 4**: No source read on "startup validation vs runtime health" (SRE book reliability/launch chapters, "fail fast"); only the monitoring chapter was read.
- **Gap 5**: The Overdrive ADR-0358 and issue #376 were not re-read (no shell/`gh`); statements about them come from the task prompt.
- **Gap 6**: Fetches summarised by a small model; verbatim quotes should be re-verified before use in an ADR.

## Conflicting Information
None material. Minor tension: CoreDNS `health_check` counts REFUSED/SERVFAIL replies as healthy, so a resolver that answers every query with SERVFAIL stays "healthy" ([forward](https://coredns.io/plugins/forward/)), meaning health-check signals can understate user-visible failure; SRE symptom-based guidance covers this.

## Recommendations for Further Research
1. If O3/O4 is pursued, read how Overdrive's existing node status and observation rows already expose component health, before proposing a new surface.
2. Fill Gap 1 and Gap 2 if the CoreDNS/dnsmasq precedent is to be quoted in an ADR.
3. Measure the real cost of O1 on the appliance (what else `serve` refusing takes down) before comparing.

## Full Citations
[1] CoreDNS. "forward". https://coredns.io/plugins/forward/. Accessed 2026-11-25.
[2] CoreDNS. "ready". https://coredns.io/plugins/ready/. Accessed 2026-11-25.
[3] CoreDNS. "reload". https://coredns.io/plugins/reload/. Accessed 2026-11-25.
[4] CoreDNS. "loop". https://coredns.io/plugins/loop/. Accessed 2026-11-25.
[5] CoreDNS source. plugin/pkg/parse/host.go, plugin/forward/setup.go, plugin/forward/forward.go (master). https://raw.githubusercontent.com/coredns/coredns/master/plugin/forward/forward.go and siblings. Accessed 2026-11-25.
[6] Kelley, S. dnsmasq manual. https://dnsmasq.org/docs/dnsmasq-man.html. Accessed 2026-11-25.
[7] dnsmasq source mirror (imp/dnsmasq). src/dnsmasq.c. https://raw.githubusercontent.com/imp/dnsmasq/master/src/dnsmasq.c. Accessed 2026-11-25.
[8] systemd. systemd-resolved.service(8). https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html. Accessed 2026-11-25.
[9] systemd. resolved.conf(5). https://www.freedesktop.org/software/systemd/man/latest/resolved.conf.html. Accessed 2026-11-25.
[10] Docker. "Networking overview". https://docs.docker.com/engine/network/. Accessed 2026-11-25.
[11] moby source. daemon/libnetwork/internal/resolvconf/resolvconf.go. https://raw.githubusercontent.com/moby/moby/master/daemon/libnetwork/internal/resolvconf/resolvconf.go. Accessed 2026-11-25.
[12] NLnet Labs. unbound-control; unbound-checkconf. https://nlnetlabs.nl/documentation/unbound/unbound-control/ ; https://nlnetlabs.nl/documentation/unbound/unbound-checkconf/. Accessed 2026-11-25.
[13] HashiCorp. Consul DNS configuration. https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/dns. Accessed 2026-11-25.
[14] Kubernetes. "Liveness, Readiness, and Startup Probes". https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/. Accessed 2026-11-25.
[15] Kubernetes. "Node Status". https://kubernetes.io/docs/reference/node/node-status/. Accessed 2026-11-25.
[16] Kubernetes. "Using NodeLocal DNSCache". https://kubernetes.io/docs/tasks/administer-cluster/nodelocaldns/. Accessed 2026-11-25.
[17] Google. "Monitoring Distributed Systems", SRE book ch. 6. https://sre.google/sre-book/monitoring-distributed-systems/. Accessed 2026-11-25.

## Research Metadata
Examined: ~25 fetches/searches | Cited: 17 | Cross-refs: probes (2), CoreDNS docs+source | Confidence: High 25%, Medium 55%, Low 20% | Output: docs/research/dataplane/dns-forwarder-upstream-config-failure-surfacing-research.md
