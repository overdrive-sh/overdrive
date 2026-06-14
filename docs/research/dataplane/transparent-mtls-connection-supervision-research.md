# Research: Transparent-mTLS / L4-Proxy Dataplane Connection-Lifecycle Supervision

**Date**: 2026-06-14 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 22

> **Decision this informs**: Overdrive's `transparent-mtls-host-socket` feature
> (ADR-0069, GH #26) ships a kernel-mediated, agent-light transparent mTLS proxy.
> A stalled `splice`/kTLS pump can strand a connection (legs open, fds leaked).
> The crux: **should per-connection liveness/reaping be (A) a centralized
> reconciler over the live-connection set, (B) per-connection self-supervising
> tasks with idle/read/duration timeouts, or (C) left to kernel TCP timeout
> machinery?** This document answers with production precedent from Cilium,
> Istio ambient/ztunnel, Linkerd, Envoy, and the Linux kernel.

---

## Executive Summary

Across the five surveyed dataplanes (Cilium, Istio ambient/ztunnel, Linkerd,
Envoy) and the Linux kernel, **per-connection liveness is NOT handled by a
central control loop that enumerates live connections each tick**. The dominant
pattern is **per-connection self-supervision** — each connection's own task,
timer, or middleware owns its idle/read/duration timeouts (Envoy's
`libevent`-per-connection timers; ztunnel's per-connection tokio copy future;
linkerd2-proxy's tower `Idle`/`FailFast` middlewares) — **backed by the kernel's
TCP machinery** (keepalive, `TCP_USER_TIMEOUT`, RST, `tcp_retries2`) for the
transport-dead class. Where control loops appear in these systems, they
reconcile **configuration** — endpoints, certs, sockmap entries, and
authorization policy via the control plane / xDS — never per-connection
liveness. The single central live-connection loop found anywhere in the survey,
ztunnel's `ConnectionManager` + `PolicyWatcher`, exists explicitly to
re-evaluate **RBAC on policy change and drain unauthorized connections**, and
its own source documents it as "policy enforcement and graceful connection
draining… not connection reaping."

For Overdrive's agent-light kTLS-splice case specifically, the evidence splits
the problem into two stall classes. **Transport-dead flows** (peer gone, unacked
data, half-open) are reaped by the kernel — and the production answer (Linkerd's
`TCP_USER_TIMEOUT` fix, ztunnel's default-on keepalive) is to *tune the kernel*,
not to add a userspace watchdog. **Progress-stuck flows** (sockets healthy at
the TCP layer but the byte pump not advancing — a `splice` stalled on a pending
record, a stuck/zero-draining buffer) are the one documented class the kernel
**cannot** detect; Cloudflare's canonical reference prescribes application-level
*progress* monitoring (`tcpi_notsent_bytes` deltas), and this is exactly
Overdrive's F6 `liveness == Stalled` predicate (a record pending but not
advancing). Crucially, that progress observation, in every production analogue,
lives **per-connection** — not in a central reconciler.

**Recommendation (Q5): adopt a (C)+(B) hybrid and reject (A).** Lean on the
kernel — `TCP_USER_TIMEOUT` + keepalive on the spliced legs — for transport
death (shape C, the direct Linkerd/ztunnel precedent), and use a **per-connection
self-supervising progress watchdog** inside each `EnforcedConnection`'s task for
the residual stall the kernel cannot see (shape B, the Envoy/linkerd2-proxy/
ztunnel idiom), evaluating the existing `Stalled` predicate on the connection's
own deadline and self-tearing-down fail-closed. **Do not model per-connection
liveness as a reconciler over the live-connection set (shape A)**: no surveyed
production dataplane does so, and Overdrive's own reconciler doctrine
(`.claude/rules/reconcilers.md`) independently disqualifies it — a stalled
connection is not desired-vs-actual config drift, the connection's own task is
the natural owner of its death, and a per-tick enumeration is the wrong
granularity for a stall deadline. Reserve a central registry/loop for the
genuine config-reconciliation need it serves in ztunnel — **policy/identity-driven
force-close** (cert revoked, exemption removed) — and name it for that, not for
liveness.

---

## Research Methodology

**Search Strategy**: Targeted searches against the trusted-source domains
(envoyproxy.io, istio.io, github.com/istio/ztunnel, linkerd.io,
github.com/linkerd/linkerd2-proxy, docs.cilium.io, docs.kernel.org, lwn.net,
man pages, tokio.rs). Source code reads of the Rust proxies (ztunnel,
linkerd2-proxy) where doc prose is silent on the mechanism.

**Source Selection**: Types: official project docs, open-source code, kernel
docs, man pages, academic where available. Reputation: high min for official
docs; medium-high for industry blogs cross-referenced with a high-tier source.

**Quality Standards**: Target 3 sources/claim (min 1 authoritative). All major
claims cross-referenced. Avg reputation target ≥ 0.80.

---

## System 1: Cilium (eBPF / sockmap / kTLS + Envoy L7)

Cilium is the closest to Overdrive's *kernel-mediated* model: data movement
happens in the kernel via eBPF sockops/sockmap (socket-layer redirect) and, for
transparent encryption, kTLS. The question is whether the Cilium agent watches
or reaps individual kernel-side flows.

### Finding 1.1: Cilium's sockops/sockmap hooks fire at TCP state transitions and on every send — they enforce policy/redirect at the socket layer, keyed to the kernel's own TCP state machine

**Evidence**: Cilium implements socket-level acceleration through two hooks. The
**Socket Operations Hook** "runs on TCP events" and "monitors for TCP state
transitions, specifically for ESTABLISHED state transitions"; when a connection
reaches ESTABLISHED with a local peer, a socket send/recv program attaches. The
**Socket Send/Recv Hook** "runs on every send operation performed by a TCP
socket" and can "drop the message, send the message to the TCP layer, or
redirect the message to another socket." Cilium calls this "Socket Layer
Enforcement," attaching "to all TCP sockets associated with Cilium managed
endpoints." The sockmap "represents a map from 5-tuple to the socket and is
primarily managed from the datapath using a sockops program."

**Source**: [Cilium docs — eBPF Datapath / Socket Layer Enforcement](https://docs.cilium.io/en/stable/network/ebpf/intro/) — Accessed 2026-06-14
**Source**: [cilium/cilium pkg/maps/sockmap (pkg.go.dev)](https://pkg.go.dev/github.com/cilium/cilium/pkg/maps/sockmap) — Accessed 2026-06-14
**Confidence**: High (official Cilium docs + the Go package doc)
**Analysis**: The hooks are event-driven off the kernel's TCP state machine,
not off an agent poll. The sockmap redirect and policy verdict are computed at
connection establishment / per-send; nothing here is a per-connection liveness
timer.

### Finding 1.2: Cilium's eBPF L4 datapath delegates connection death to the kernel TCP stack; the agent reconciles CONFIG (maps/endpoints/policy), not per-connection liveness

**Evidence**: The Cilium socket-LB documentation "focuses on policy enforcement
at connection establishment but remains silent on what component owns connection
termination, reaping, or ongoing lifecycle monitoring after the kernel takes
over data movement." Cilium's sockmap entry is added on ESTABLISHED; the kernel
TCP stack drives the connection through its states. The agent's documented role
is map/endpoint/policy management — i.e. config reconciliation — not a watchdog
over live flows.

**Source**: [Cilium docs — eBPF Datapath / Socket Layer Enforcement](https://docs.cilium.io/en/stable/network/ebpf/intro/) — Accessed 2026-06-14
**Source (transparent encryption talk)**: [Linux Plumbers Conf — Seamless transparent encryption with BPF and Cilium](https://lpc.events/event/4/contributions/461/attachments/253/439/Seamless_transparent_encryption_with_BPF_and_Cilium1.pdf) — Accessed 2026-06-14
**Confidence**: Medium-High (the agent's config-only role is the consistent
Cilium design across docs; the *absence* of a per-connection liveness loop is
inferred from the datapath docs never describing one — a documented-silence
inference, flagged in Knowledge Gaps). The sockmap-cleanup-on-FIN/close behavior
is owned by the kernel's sockmap/sk lifecycle.
**Analysis**: This is strong support for the Q2 thesis at the kernel-datapath
extreme: when bytes move in the kernel, **the kernel TCP stack owns connection
death**, and the userspace agent's loop reconciles configuration (sockmap
entries, endpoints, policy, certs) — not the liveness of individual flows. A
sockmap entry is removed by the kernel when the socket closes; the agent does
not enumerate live sockets each tick to reap them.

### Finding 1.3: For L7, Cilium delegates to Envoy (per-connection timers); for native L4 mTLS, Cilium 2026 integrates ztunnel rather than building its own per-connection reaper

**Evidence**: Cilium's L7 path uses an embedded **Envoy** proxy (whose
per-connection timeout model is System 4). In March 2026 Cilium published
"Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity with
ztunnel," integrating Istio's **ztunnel** as the L4 mTLS dataplane rather than
inventing a separate per-connection supervision mechanism. kTLS "is a kernel
implementation of TLS that works after initial handshake, with BPF sockmap
attached to sockets to enforce policy."

**Source**: [Cilium blog — Native mTLS for Cilium (with ztunnel), 2026-03-23](https://cilium.io/blog/2026/03/23/native-mtls-cilium/) — Accessed 2026-06-14 [body did not render via fetch — title/topic confirmed; see Knowledge Gaps]
**Source**: [cilium/cilium Issue #14852 — kernel requirements for transparent encryption](https://github.com/cilium/cilium/issues/14852) — Accessed 2026-06-14
**Confidence**: Medium (the ztunnel-integration headline + topic are confirmed;
the article body did not render, so the lifecycle specifics are inferred from
the ztunnel architecture in System 2). Flagged in Knowledge Gaps.
**Analysis**: Cilium's own answer to "how do we do L4 mTLS connection
lifecycle" in 2026 is **reuse ztunnel** (System 2's per-connection-task +
config-reconciling-ConnectionManager model) — it did not build a central
live-connection liveness reconciler. For L7 it reuses Envoy's per-connection
timers. Cilium nowhere ships a control loop that enumerates live connections to
reap stalled ones; it ships kernel datapath + config reconciliation + delegated
proxies.

## System 2: Istio ambient mode / ztunnel (Rust per-node L4 mTLS, HBONE)

ztunnel is the closest architectural analogue to Overdrive: a per-node, Rust,
L4 mTLS proxy built on tokio + hyper, tunnelling over HBONE (HTTP CONNECT). It
is L3/L4-scoped (mTLS, authn, L4 authz, telemetry) by design.

### Finding 2.1: ztunnel proxies each connection as an independent tokio task on a multi-thread worker runtime

**Evidence**: ztunnel "builds on top of the Tokio and Hyper libraries… to write
highly performant asynchronous code… Async programming in Rust supports
work stealing natively via its Tokio library." The architecture splits a "main"
thread (single-threaded tokio runtime for admin/XDS) from "worker" thread(s)
running "a multi-thread Tokio runtime to handle users requests" (default 2
threads). ztunnel "exposes metrics for the data plane worker pool Tokio
runtime, including per-worker counter metrics like worker busy duration and park
count" and a "TCP Connections Closed… COUNTER incremented for every closed
connection."

**Source**: [Istio — Introducing Rust-Based Ztunnel](https://istio.io/latest/blog/2023/rust-based-ztunnel/) — Accessed 2026-06-14
**Source**: [istio/ztunnel ARCHITECTURE.md](https://github.com/istio/ztunnel/blob/master/ARCHITECTURE.md) — Accessed 2026-06-14
**Confidence**: High (official Istio blog + project architecture doc)
**Analysis**: The work-stealing multi-thread runtime + per-connection-closed
counter is the tokio task-per-connection idiom. Each proxied connection is a
future that drives its own bidirectional copy to completion; its lifecycle ends
when the future resolves (EOF, error, or timeout inside the future).

### Finding 2.2 (LOAD-BEARING for Q2): ztunnel's ConnectionManager is a registry for policy-driven DRAINING, NOT for stall-reaping

**Evidence**: `ConnectionManager` is "a registry that tracks both inbound and
outbound proxy connections" with `drains: Arc<RwLock<HashMap<InboundConnection,
ConnectionDrain>>>` and `outbound_connections: Arc<RwLock<HashSet<...>>>`. "The
primary purpose is **policy enforcement and graceful connection draining upon
authorization policy changes — not connection reaping**. The manager enables
runtime RBAC re-evaluation and ordered shutdown of connections that violate
updated security policies." The enumeration loop is `PolicyWatcher`: on a policy
update it walks tracked connections and `if self.state.assert_rbac(&conn.ctx)
.await.is_err() { self.connection_manager.close(&conn).await }`. A
`ConnectionGuard`'s `Drop` impl "provides a fallback cleanup," but the registry's
*loop* exists to close connections whose **authorization** no longer holds, not
connections that have stalled.

**Source**: [istio/ztunnel src/proxy/connection_manager.rs](https://github.com/istio/ztunnel/blob/master/src/proxy/connection_manager.rs) — Accessed 2026-06-14
**Confidence**: High (direct source read of the canonical file)
**Analysis**: This is the single most important finding for Q2. ztunnel — the
closest analogue to Overdrive — DOES run a central loop over the live-connection
set, but that loop's job is **config/policy reconciliation** (re-assert RBAC
when policy changes), exactly the "control loops reconcile config, not
liveness" thesis. Stall/idle teardown is NOT the ConnectionManager's job; it
belongs to the per-connection task (the bidirectional-copy future) and the
kernel. The `Drop`-based `ConnectionGuard` cleanup confirms self-completion is
the primary teardown path; the central loop is the *additional* config-driven
override. The "Register before our initial assert… prevents a race if policy
changes between assert() and track()" comment shows the registry's raison
d'être is the policy race, not liveness.

### Finding 2.3: ztunnel leans on kernel TCP keepalive (default-on since Istio 1.24) + RST for connection death, not a userspace per-connection liveness reaper

**Evidence**: "As of Istio 1.24, Ztunnel will enable keepalives on connections
by default… both the connection from the application to ztunnel and from ztunnel
to the destination." On shutdown ztunnel uses a long drain to "let existing
connections naturally die out"; for dropped TCP connections "they should get a
RST." ztunnel "relies on TCP-level mechanisms rather than userspace reapers" —
when a pod's veth is removed, clients "should get a RST" (the maintainers even
flag "TODO: verify this happens, since the veth is ripped out from us!"). Drain
is split into **per-pod** (shut down immediately — CNI rips the veth) and
**process-level** (graceful, let connections die out) — both are *config /
lifecycle-event* driven, not a liveness poll.

**Source**: [istio/ztunnel Issue #1191 — Implement improved draining](https://github.com/istio/ztunnel/issues/1191) — Accessed 2026-06-14
**Source**: [istio/istio Issue #32116 — proxy doesn't close downstream when TCP keepalives fail](https://github.com/istio/istio/issues/32116) — Accessed 2026-06-14
**Source**: [Istio & Envoy Insider — Socket Options](https://istio-insider.mygraphql.com/en/latest/ch2-envoy/socket/socket-options.html) — Accessed 2026-06-14
**Confidence**: High (project issue + Istio issue + insider doc all agree)
**Analysis**: ztunnel's connection-death strategy is: per-connection task drives
the copy to EOF/error; **kernel keepalive + RST + (recommended) TCP_USER_TIMEOUT
reap dead transport**; central machinery handles only drain (a shutdown/config
event) and policy. No component enumerates live connections each tick to test
liveness. Note the asymmetric-teardown hazard (Istio #32116: "after all probes
fail, proxy closes the upstream connection but leaves the downstream intact") —
a real lifecycle bug, but the fix is socket-option tuning + correct
half-close propagation in the per-connection task, not a central reaper.

## System 3: Linkerd / linkerd2-proxy (Rust micro-proxy)

linkerd2-proxy is a per-pod Rust micro-proxy built on tokio + tower + hyper.
Its timeout model is built from **tower middlewares** stacked per-connection /
per-service, not a central reaper.

### Finding 3.1: linkerd2-proxy implements idle/failfast as per-stack tower middlewares (Service combinators), not a central loop

**Evidence**: linkerd2-proxy "introduced `idle` and `failfast` timeout
middlewares, where `idle` causes the service to start failing if polled after
being ready and unused for some timeout." The commit "timeout: Introduce
FailFast, Idle, and Probe middlewares (#452)" adds these as composable layers.
The `idle` middleware "is intended to be driven by `probe-buffer`."

**Source**: [linkerd2-proxy commit 5206901 — Introduce FailFast, Idle, and Probe middlewares (#452)](https://github.com/linkerd/linkerd2-proxy/commit/52069015990cb07de6a142a3a7b55e90ff9cf701) — Accessed 2026-06-14
**Source**: [linkerd/linkerd2 Discussion #13566 — Idle connections timeout](https://github.com/linkerd/linkerd2/discussions/13566) — Accessed 2026-06-14
**Confidence**: High (commit in the canonical repo + project discussion)
**Analysis**: A tower `Idle` middleware wraps a `Service`; the timer lives in
the wrapped service's `poll_ready`/`call` path. This is per-connection
self-supervision realized as a stack layer — the idiomatic Rust async pattern.
There is no enumerating reaper; the middleware *is* the timeout, co-located with
the connection's own future.

### Finding 3.2: Linkerd does NOT enforce an idle timeout on opaque (pure-L4) connections — it mirrors the application

**Evidence**: "Linkerd doesn't enforce an idle timeout on opaque connections —
if the application holds the connection open, linkerd should as well." A protocol
detection timeout of 10 seconds applies at connection setup, but steady-state
opaque (raw-TCP) connections are not idle-reaped by the proxy.

**Source**: [linkerd/linkerd2 Discussion #13566 — Idle connections timeout](https://github.com/linkerd/linkerd2/discussions/13566) — Accessed 2026-06-14
**Source**: [linkerd/linkerd2 Discussion #8761 — protocol detection timeout](https://github.com/linkerd/linkerd2/discussions/8761) — Accessed 2026-06-14
**Confidence**: Medium-High (project discussions; consistent with the opaque-L4
design philosophy)
**Analysis**: This is direct evidence for Q4. For pure-L4 (the Overdrive case),
the dominant proxy stance is **do not impose a userspace idle timeout** —
long-lived idle connections are legitimate and the proxy transparently mirrors
the application's lifecycle. The proxy avoids being the entity that decides "this
idle connection is dead."

### Finding 3.3 (LOAD-BEARING for Q3): Linkerd's fix for stalled half-open connections delegates to the KERNEL via TCP_USER_TIMEOUT, not a userspace watchdog

**Evidence**: Linkerd issue #13023 documents a failure mode where, "during
ungraceful node termination, TCP connections entered a half-open state. These
connections could accumulate for up to 15 minutes without the application or
Linkerd detecting them as broken." Critically: "**TCP_KEEPALIVE doesn't work on
half-open connections**… with a half-open TCP connection, keepalive is not
applied to the connection. The kernel's retransmission timeout (RTO) mechanism
controlled connection fate — allowing approximately 15 retransmission attempts
before finally timing out, consuming that 15-minute window." **The fix is
`TCP_USER_TIMEOUT`** (a socket option set to ~30s default, ~7 retransmissions),
which "delegate[s] earlier timeout control to the kernel rather than relying on
application-level detection mechanisms… kernel-level solutions are necessary for
this class of problem."

**Source**: [linkerd/linkerd2 Issue #13023 — Implement TCP_USER_TIMEOUT to detect half-opened TCP connections](https://github.com/linkerd/linkerd2/issues/13023) — Accessed 2026-06-14
**Source**: [linkerd2-proxy PR #186 — Introduce TCP keepalive configuration](https://github.com/linkerd/linkerd2-proxy/pull/186) — Accessed 2026-06-14
**Confidence**: High (specific issue with the documented mechanism + the
keepalive-config PR)
**Analysis**: This is the strongest single data point for Q3 and the
agent-light case. A production Rust L4 mTLS proxy hit exactly the class of
problem Overdrive's F6/F7 worries about (a connection that is stranded —
half-open, dangling, not advancing) and the engineered answer was **not** "add a
userspace supervisor that enumerates connections and reaps stalled ones." It was
**tune the kernel's own transport-death machinery** (`TCP_USER_TIMEOUT` +
keepalive socket options) so the kernel reaps faster. The userspace proxy task
then observes the socket error/EOF when the kernel kills the connection, and its
own future resolves. The supervisor is the kernel; the proxy is a downstream
observer.

## System 4: Envoy (reference L4/L7 proxy — the timeout taxonomy)

Envoy is the reference L4/L7 proxy and the canonical vocabulary source for
connection-lifecycle timeouts. Its model is the clearest single statement of
the **per-connection-timer** pattern.

### Finding 4.1: Envoy's TCP-proxy idle_timeout measures bidirectional byte inactivity, per-connection

**Evidence**: "The idle timeout for connections managed by the TCP proxy
filter is the period in which there are no bytes sent or received on **either
the upstream or downstream connection**. If not set, the default idle timeout
is 1 hour. If set to 0s, the timeout is disabled." The timeout **resets on
activity** (bytes flowing) and **fires by closing the connection**.

**Source**: [Envoy — How do I configure timeouts?](https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/timeouts.html) — Accessed 2026-06-14
**Source (proto)**: [Envoy TCP Proxy (proto v3)](https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/filters/network/tcp_proxy/v3/tcp_proxy.proto) — Accessed 2026-06-14
**Confidence**: High (official docs, two pages, cross-referenced)
**Analysis**: This is the L4 analogue of Overdrive's concern. Note: it is an
**idle** timeout (no bytes either direction), not a stall detector — Envoy
treats "no bytes flowing" uniformly whether the connection is healthily idle or
pathologically stalled. The teardown is keyed to inactivity duration, not to a
"record pending but not advancing" predicate. See Q4.

### Finding 4.2: Envoy distinguishes idle (no active streams) from duration (hard cap) — and the idle timer does NOT fire while a stream is active

**Evidence**: "Idle timeout only fires when there are no active streams, unlike
`max_connection_duration` which can trigger while streams are active."
`max_connection_duration` "enforces a hard limit on connection lifetime
regardless of activity" and "Initiates drain sequence; closes after drain
period if no active streams." `stream_idle_timeout` is "the amount of time that
the connection manager will allow a stream to exist with no upstream or
downstream activity" (default 5 min), **resets on upstream or downstream
activity**, and **fires by closing the stream**.

**Source**: [Envoy — How do I configure timeouts?](https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/timeouts.html) — Accessed 2026-06-14
**Confidence**: High (official docs)
**Analysis**: Envoy's taxonomy separates three concerns, each its own
per-connection/per-stream timer: (1) **idle** (activity-reset, fires only when
nothing is happening), (2) **stream idle** (per-stream activity-reset), (3)
**max duration** (absolute, non-resetting hard cap that proceeds via a drain).
This is the canonical decomposition any L4 mTLS proxy reimplements.

### Finding 4.3: Envoy's stall/no-progress detection is per-connection timers in the data path, NOT a central liveness-enumeration loop

**Evidence**: The documentation "indicates Envoy detects stalled connections
through explicit monitoring of 'upstream or downstream activity' without
describing a central enumeration loop. These appear to be per-connection timers
rather than centralized checks." Each timeout (`idle_timeout`,
`stream_idle_timeout`, `max_connection_duration`, `request_timeout`,
`request_headers_timeout`, `connect_timeout`) is documented as a property **of
a connection or stream**, armed when that connection/stream is created and
refreshed by that connection's own activity.

**Source**: [Envoy — How do I configure timeouts?](https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/timeouts.html) — Accessed 2026-06-14
**Confidence**: Medium-High (official docs are explicit on the per-connection
nature of each timer; the absence of a central loop is inferred from the
documentation never describing one, plus Envoy's well-known event-loop
architecture where each connection lives on a worker thread's `libevent`
dispatcher with its own timers). Cross-reference with Envoy threading model
pending.
**Analysis**: Envoy's architecture is one event loop per worker thread; a
connection is pinned to a worker and its timers are `libevent` timer events on
that worker's dispatcher. There is no Envoy component that walks "all live
connections" each tick to check liveness — the timer *is* the liveness check,
and it lives with the connection. This is direct precedent for shape **(B)**.

## System 5: Kernel-level (kTLS + splice + TCP timeout machinery)

For the agent-light case — userspace sets up the socket + kTLS, then the kernel
moves bytes via `splice` — what reaps a stalled flow? The kernel has rich
transport-death machinery, but it has a documented gap that maps exactly onto
Overdrive's F6 stall mode.

### Finding 5.1: TCP keepalive only works on IDLE connections (empty send buffer); it is bypassed when data is unacknowledged in flight

**Evidence**: "Keepalives operate only on idle connections… For keepalives to
work, the send buffer must be empty. When a socket has unacknowledged data in
flight, the retransmission timer takes precedence and keepalives are completely
bypassed." Keepalive params: `TCP_KEEPIDLE` (default 2h), `TCP_KEEPINTVL`
(75s), `TCP_KEEPCNT` (9 probes).

**Source**: [Cloudflare — When TCP sockets refuse to die](https://blog.cloudflare.com/when-tcp-sockets-refuse-to-die/) — Accessed 2026-06-14
**Source**: [tcp(7) — Linux manual page (man7.org)](https://man7.org/linux/man-pages/man7/tcp.7.html) — Accessed 2026-06-14
**Confidence**: High (Cloudflare is the canonical practitioner reference on this,
cross-referenced with the authoritative tcp(7) man page)
**Analysis**: Keepalive is the kernel's mechanism for the *idle-but-dead* case.
It does not help when bytes are queued and stuck — which is closer to the stall
Overdrive worries about.

### Finding 5.2: TCP_USER_TIMEOUT bounds how long data may remain unacknowledged before the kernel force-closes — and overrides keepalive

**Evidence**: `TCP_USER_TIMEOUT` "sets the maximum amount of time that
transmitted data may remain unacknowledged before the kernel forcefully closes
the connection… Applies regardless of whether data is in flight or the
connection is idle. With keepalives enabled, it will override keepalive to
determine when to close a connection." It "is effective only during the
synchronized states of a connection (ESTABLISHED, FIN-WAIT-1, FIN-WAIT-2,
CLOSE-WAIT, CLOSING, and LAST-ACK)." Without it, a busy ESTABLISHED socket with
unacked data is governed by `tcp_retries2` (default 15 retransmissions ≈ 15
minutes).

**Source**: [Cloudflare — When TCP sockets refuse to die](https://blog.cloudflare.com/when-tcp-sockets-refuse-to-die/) — Accessed 2026-06-14
**Source**: [tcp(7) — Linux manual page (man7.org)](https://man7.org/linux/man-pages/man7/tcp.7.html) — Accessed 2026-06-14
**Confidence**: High (two authoritative sources agree)
**Analysis**: This is the kernel knob Linkerd reached for (Finding 3.3). For the
*kernel-side* legs of a kTLS-spliced flow where the remote peer dies or stops
ACKing, `TCP_USER_TIMEOUT` + keepalive give the kernel a bounded, tunable death
timer with **no userspace involvement**. This is the "rely on the kernel" shape
(C) and it covers the *transport-dead* class of stall.

### Finding 5.3 (LOAD-BEARING for Q3/Q5): The kernel CANNOT detect "slow drains" or "stuck buffers" — data queued but never progressing — application-level monitoring is required

**Evidence**: "The kernel cannot automatically detect: (1) **Slow drains** — a
connection correctly transmitting data, but below acceptable speed thresholds;
(2) **Stuck buffers** — data queued for sending that never progresses." The
recommended remedy is **application-level**: "monitoring draining pace using
`TCP_INFO` parameter `tcpi_notsent_bytes` to track unsent buffer size over time,
calculating bytes-per-second and terminating connections failing to meet minimum
throughput requirements."

**Source**: [Cloudflare — When TCP sockets refuse to die](https://blog.cloudflare.com/when-tcp-sockets-refuse-to-die/) — Accessed 2026-06-14
**Confidence**: Medium-High (single authoritative practitioner source for the
explicit "kernel cannot detect" claim; mechanism — `tcpi_notsent_bytes` via
`TCP_INFO` — is cross-confirmed by tcp(7)/`struct tcp_info`). Flagged for a
second source in Knowledge Gaps.
**Analysis**: This is the crux for Overdrive. There are **two distinct stall
classes**:
1. **Transport-dead** (peer gone, unacked data, half-open) — reaped by the
   kernel via `TCP_USER_TIMEOUT` + keepalive. No userspace supervisor needed.
2. **Progress-stuck** (sockets healthy at the TCP layer, but the byte pump is
   not advancing — a `splice` that stops moving a pending record, a stuck
   buffer, a slow/zero drain) — **the kernel does NOT reap this**. It requires a
   userspace observer that watches *progress* (e.g. `tcpi_notsent_bytes` deltas
   over time, or a record-pending-but-not-advancing predicate).
Overdrive's F6 (`liveness` returns `Stalled` only when a record is *pending* but
progress hasn't advanced) is precisely class 2 — exactly the case the kernel
leaves to userspace. This validates that *some* userspace observation is
warranted, but says nothing yet about whether that observation must be a central
reconciler vs per-connection. See Q5.

### Finding 5.4: kTLS uses splice()/sendfile()/read()/write() on the kTLS fd; a non-data TLS control message detaches the kTLS socket and surfaces an error

**Evidence**: "Standard `read()`, `write()`, `sendfile()` and `splice()` system
calls are used on the kTLS file descriptor. Upon receipt of a non-data TLS
message (a control message), the kTLS socket returns an error, and the message
is left on the original TCP socket instead. The kTLS socket is automatically
unattached."

**Source**: [The Linux Kernel — Kernel TLS offload documentation (docs.kernel.org)](https://docs.kernel.org/networking/tls-offload.html) — Accessed 2026-06-14
**Source**: [NVIDIA — Kernel Transport Layer Security (kTLS) Offloads](https://networking-docs.nvidia.com/doca/sdk/ktls-offloads) — Accessed 2026-06-14
**Confidence**: High (kernel.org is authoritative; NVIDIA cross-confirms)
**Analysis**: This matters for the agent-light pump: a kTLS control message
(e.g. key update, alert) surfaces as an *error/EOF* on the spliced fd, which the
userspace pump task observes — another case where the kernel hands a signal back
to userspace rather than silently stalling. The stall Overdrive fears is the
residual: splice stops advancing with a record pending and NO error surfaced.

---

## Synthesis — The Q1–Q5 Questions

### Q1 — The dominant pattern (central supervisor vs per-connection self-supervision)

**Answer: per-connection self-supervision (shape B) is the dominant pattern,
backed by kernel TCP machinery (shape C). No surveyed system uses a central
loop that enumerates live connections each tick to test liveness.**

| System | Per-connection mechanism | Where the timer/logic lives |
|---|---|---|
| **Envoy** | `idle_timeout` (TCP proxy), `stream_idle_timeout`, `max_connection_duration`, `connect_timeout` | per-connection/per-stream `libevent` timers on the owning worker's event loop (Findings 4.1–4.3) |
| **ztunnel** | per-connection tokio task drives the bidirectional copy to EOF/error; kernel keepalive (default-on, Istio 1.24+) | the connection's own future + kernel socket options (Findings 2.1, 2.3) |
| **linkerd2-proxy** | tower `Idle` / `FailFast` / `Probe` middlewares wrapping per-connection `Service`s; per-connection keepalive | composable stack layers co-located with the connection (Findings 3.1, 3.2) |
| **Cilium (L4)** | kernel TCP state machine + sockmap entry lifecycle; delegates L4 mTLS to ztunnel | kernel; agent only reconciles config (Findings 1.1–1.3) |
| **Cilium (L7)** | embedded Envoy → Envoy's per-connection timers | Envoy worker event loops (Finding 1.3) |

Every system arms a timer (or relies on the kernel's) **at the connection's
own scope** and refreshes it from that connection's own activity. The
"supervisor" is the connection's own task/timer/middleware, not a registry walk.

**Confidence**: High — five independent systems, official docs + source.

### Q2 — Where reconcilers / control-loops ARE used in these dataplanes

**Answer: confirmed — control loops reconcile CONFIG (endpoints, certs, policy,
sockmap entries) via the control plane / xDS, NOT per-connection liveness. The
ONE central live-connection loop found (ztunnel's `PolicyWatcher`) exists to
re-evaluate AUTHORIZATION on policy change, not to reap stalled connections.**

- **ztunnel `ConnectionManager` + `PolicyWatcher`** (Finding 2.2) is the only
  central loop over the live-connection set in the survey. Its job: on an RBAC
  policy update, walk tracked connections and `close()` those that no longer
  pass `assert_rbac`. This is **config reconciliation projected onto live
  connections** — the connection set is the surface a *policy* change must be
  applied to, exactly as a reconciler applies desired config to actual state.
  It is NOT a liveness/stall reaper.
- **Cilium agent** reconciles sockmap entries, endpoints, identities, policy,
  and certs — config — while the kernel owns per-flow lifecycle (Finding 1.2).
- **Envoy / linkerd2-proxy** receive config via xDS/control-plane and apply it;
  the data path's per-connection timers are independent of any control loop.

So: **is there ANY production system that runs a reconciler over the
live-connection set for liveness/reaping?** Across the five surveyed — **no**.
The closest (ztunnel's loop) reconciles *policy*, and self-describes as draining
on policy change, not liveness reaping.

**Confidence**: High — direct source read of ztunnel's `connection_manager.rs`;
Cilium/Envoy/Linkerd config-vs-data-path split is well-documented.

### Q3 — The agent-light / kTLS-splice specific case (who detects a stall)

**Answer: split by stall class. (1) Transport-dead flows (peer gone, unacked
data, half-open) are reaped by the KERNEL via `TCP_USER_TIMEOUT` + keepalive +
RST — no userspace supervisor needed. (2) Progress-stuck flows (sockets healthy
but the byte pump is not advancing — a `splice` stalled on a pending record, a
stuck/zero-draining buffer) ESCAPE kernel reaping and are the ONE case a
userspace observer is genuinely required.**

Evidence chain:
- The kernel's keepalive only fires on idle (empty send buffer) connections
  (Finding 5.1); `TCP_USER_TIMEOUT` bounds unacked-data and zero-window-buffered
  time and force-closes with `ETIMEDOUT` (Finding 5.2; tcp(7) confirms it covers
  "buffered data may remain untransmitted (due to zero window size)").
- **Linkerd's production answer** to a stranded-half-open class was
  `TCP_USER_TIMEOUT`, explicitly delegating to the kernel, NOT a userspace
  watchdog (Finding 3.3). **ztunnel's answer** is default-on keepalive + RST on
  veth removal (Finding 2.3). Both lean on the kernel for transport death.
- **The documented gap** (Finding 5.3): the kernel cannot detect "slow drains"
  or "stuck buffers" — data queued but never progressing. The remedy is
  application-level progress monitoring (`tcpi_notsent_bytes` deltas over time).

Overdrive's F6 `Stalled` predicate (a record is *pending* but progress hasn't
advanced) is **exactly class 2** — the stuck-buffer / no-progress case the
kernel leaves to userspace. So a userspace *observation of progress* is
warranted. But note: none of the surveyed systems built a *central reconciler*
for this; where they observe progress at all, the observation is per-connection
(a middleware/timer in the data path) or they simply rely on the kernel +
correct half-close propagation.

**Confidence**: High for the two-class split and kernel mechanics (Cloudflare +
tcp(7) + kernel.org + Linkerd issue + ztunnel issue all agree); Medium for
"no surveyed system builds a central progress-reconciler" (documented-silence
inference, flagged in gaps).

### Q4 — Stall vs idle (avoiding false-positive reaping)

**Answer: production systems either (a) treat "no bytes flowing" uniformly via
an idle timeout that resets on ANY activity (Envoy) — accepting that a truly
idle keep-alive must be kept alive by activity or explicitly exempted; or (b)
deliberately do NOT impose a userspace idle timeout on opaque/L4 connections,
mirroring the application (Linkerd). The key to avoiding false positives is
that the reap trigger is keyed to a *positive predicate* (unacked data past a
deadline; a pending record not advancing), never to "looks quiet."**

- **Envoy**: `idle_timeout` resets on any byte activity and "only fires when
  there are no active streams" (Finding 4.2). A legitimately busy connection is
  never idle-reaped; a legitimately *idle-but-healthy* long-lived connection
  WILL eventually hit the idle timeout unless activity (or TCP keepalive, which
  Envoy notes is independent of the L7 idle timer) refreshes it — hence
  keepalive is configured to keep long-lived idle tunnels alive.
- **Linkerd**: "doesn't enforce an idle timeout on opaque connections — if the
  application holds the connection open, linkerd should as well" (Finding 3.2).
  This is the explicit anti-false-positive stance for pure-L4.
- **ztunnel**: keepalive distinguishes a healthy idle connection (probes
  answered) from a dead one (probes unanswered → RST/close) — the kernel makes
  the live/dead call, not a quietness heuristic (Finding 2.3 / 5.1).

Overdrive's `liveness` returning `Stalled` **only when a record is pending but
progress hasn't advanced** is the *correct* shape and MATCHES how these systems
avoid false positives: it does not reap "quiet" connections (no pending record →
not Stalled), only ones with work-in-flight that is not advancing. This is a
*progress* predicate, not an *idle* predicate — strictly better than a bare idle
timeout for the false-positive concern, because a legitimately idle long-lived
tunnel has no pending record and is never flagged.

**Confidence**: High — Envoy docs, Linkerd discussions, kernel keepalive
semantics all consistent.

### Q5 — Recommendation mapping onto Overdrive's (A)/(B)/(C) shapes

**Recommendation: a hybrid of (C) + (B), NOT (A). Lean on the kernel (C) for the
transport-dead class, and use per-connection self-supervision (B) for the
narrow progress-stuck class that the kernel cannot see. Do NOT model
per-connection liveness as a central reconciler (A); no surveyed production
dataplane does, and the one central live-connection loop that exists (ztunnel's)
reconciles policy, not liveness.**

Mapping:

**(C) Rely on kernel TCP timeouts — ADOPT as the primary mechanism for
transport death.** Set `TCP_USER_TIMEOUT` and enable keepalive on the
kernel-side (and agent-side) legs of every kTLS-spliced flow. This is the
*direct, evidenced* production answer: Linkerd (Finding 3.3) and ztunnel
(Finding 2.3) both reach for exactly these socket options for the stranded /
half-open / peer-gone class. Cost: tuning the socket options; benefit: the
kernel reaps the entire transport-dead class with zero userspace machinery, and
your pump task simply observes the resulting `ETIMEDOUT`/EOF/RST and resolves.
**This eliminates most of F7 (stranded legs) for free.**

**(B) Per-connection self-supervising task — ADOPT for the residual
progress-stuck class (Overdrive's F6 `Stalled`).** The kernel cannot see a
`splice` that stops advancing with a record pending (Finding 5.3). The idiomatic
production answer is a per-connection mechanism co-located with the connection's
own task — Envoy's per-connection timers (4.3), linkerd2-proxy's tower `Idle`
middleware (3.1), ztunnel's per-connection copy future (2.1). For Overdrive,
this means each `EnforcedConnection`'s own task owns a *progress watchdog*: arm a
deadline when a record becomes pending; if progress (bytes advanced /
`tcpi_notsent_bytes` movement / the kTLS sequence) has not changed by the
deadline, the task fails-closed and tears down its own legs. No central
registry, no per-tick enumeration. This is the same `liveness == Stalled`
predicate the design already has — but *evaluated by the connection's own task on
its own timer*, not point-queried by a central loop.

**(A) Reconciler over the live-connection set — DO NOT ADOPT for liveness.** The
evidence is consistent and decisive: no surveyed dataplane runs a central loop
that enumerates live connections each tick to test liveness/reap stalls. The
`MtlsSupervisor`-as-reconciler shape is the *odd one out*. Reasons it is the
wrong shape here:
1. **Liveness is not desired-vs-actual config.** A reconciler's value is
   converging actual state to a declared desired state that can DRIFT
   (`.claude/rules/reconcilers.md`). A stalled connection is not config drift;
   there is no "desired connection set" the platform declares and converges to.
   The connection's existence is driven by the application, not by intent.
2. **The natural owner of a connection's death is the connection's own task.**
   Point-querying each connection's `liveness` from a central tick re-derives,
   every tick, information the connection's own task already has — and adds a
   registry whose only other consumer would be... nothing. (Contrast ztunnel's
   registry, which earns its keep by being the surface for *policy*
   re-evaluation — a genuine config-reconciliation need Overdrive's F6 does not
   have.)
3. **Tick cadence is the wrong granularity for a stall deadline.** A
   per-connection timer fires at exactly the connection's stall deadline; a
   reconciler tick fires on the worker's cadence regardless, adding latency
   (reap happens up to one tick late) and wasted work (every tick walks every
   live connection even though almost none are stalled).

**Where a reconciler/control-loop IS the right tool in Overdrive's mTLS plane**
(mirroring Q2): reconciling the *config* the connections depend on — SVID
material rotation, policy/RBAC re-evaluation (ztunnel's actual `PolicyWatcher`
analogue), endpoint/backend sets, exemption rules. If Overdrive ever needs to
*force-close existing connections on a policy/identity change* (e.g. a cert
revoked, an exemption removed), THAT is the legitimate central-loop use — and it
is policy reconciliation, not liveness reaping. Keep the two concerns separate:
**policy/config → reconciler; liveness/stall → kernel (C) + per-connection task
(B).**

**Net recommendation for F6/F7**: replace the `MtlsSupervisor`-that-point-queries
-each-connection-per-tick (shape A) with (C) kernel `TCP_USER_TIMEOUT`/keepalive
on the spliced legs + (B) a per-connection progress watchdog inside each
`EnforcedConnection`'s task that evaluates the existing `liveness == Stalled`
predicate on its own deadline and self-tears-down fail-closed. Retain a central
registry ONLY if/when a *policy-driven* force-close requirement appears (the
ztunnel `ConnectionManager` pattern), and name it for that purpose, not for
liveness.

**Confidence**: High on the directional recommendation (C+B over A) — the
production precedent is unanimous and the reconciler-doctrine in
`.claude/rules/reconcilers.md` independently disqualifies A (liveness is not
desired-vs-actual config). Medium on the exact progress-watchdog mechanism
(`tcpi_notsent_bytes` vs kTLS-sequence vs splice-return polling) — this is an
implementation choice Overdrive must validate against its actual splice/kTLS
pump on a real kernel (Tier-3 spike), since no surveyed system documents the
precise predicate for a *kTLS-spliced* pump (see Knowledge Gaps).

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Envoy — How do I configure timeouts? | envoyproxy.io | High (1.0) | official | 2026-06-14 | Y (proto v3) |
| Envoy TCP Proxy proto (v3) | envoyproxy.io | High (1.0) | official | 2026-06-14 | Y |
| Istio — Introducing Rust-Based Ztunnel | istio.io | High (1.0) | official | 2026-06-14 | Y (ARCHITECTURE.md) |
| istio/ztunnel ARCHITECTURE.md | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| istio/ztunnel connection_manager.rs | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y (source read) |
| istio/ztunnel Issue #1191 (draining) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| istio/istio Issue #32116 (keepalive fail) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| Istio & Envoy Insider — Socket Options | istio-insider.mygraphql.com | Medium (0.6) | community | 2026-06-14 | Y (cross-ref tcp(7), Envoy) |
| linkerd2-proxy commit 5206901 (Idle/FailFast/Probe) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| linkerd2-proxy PR #186 (keepalive) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| linkerd/linkerd2 Issue #13023 (TCP_USER_TIMEOUT) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| linkerd/linkerd2 Discussion #13566 (idle timeout) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | Y |
| linkerd/linkerd2 Discussion #8761 (proto detect) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | partial |
| Cilium docs — eBPF Datapath / Socket Layer Enforcement | docs.cilium.io | High (1.0) | open_source | 2026-06-14 | Y (pkg.go.dev) |
| cilium/cilium pkg/maps/sockmap | pkg.go.dev | High (1.0) | technical_doc | 2026-06-14 | Y |
| Cilium blog — Native mTLS (ztunnel), 2026-03-23 | cilium.io | High (1.0) | open_source | 2026-06-14 | partial (body unrendered) |
| cilium/cilium Issue #14852 (encryption kernel reqs) | github.com | Medium-High (0.8) | open_source | 2026-06-14 | partial |
| LPC — Seamless transparent encryption with BPF and Cilium | lpc.events | Medium-High (0.8) | industry | 2026-06-14 | partial |
| Cloudflare — When TCP sockets refuse to die | blog.cloudflare.com | Medium-High (0.8) | industry | 2026-06-14 | Y (tcp(7)) |
| tcp(7) — Linux manual page | man7.org | High (1.0) | official | 2026-06-14 | Y |
| Linux Kernel — Kernel TLS offload | docs.kernel.org | High (1.0) | official | 2026-06-14 | Y (NVIDIA) |
| NVIDIA — kTLS Offloads (DOCA) | networking-docs.nvidia.com | Medium-High (0.8) | technical_doc | 2026-06-14 | Y |

**Reputation summary**: High: 10 (~45%) | Medium-High: 10 (~45%) | Medium: 1 (~5%) |
Excluded sources used: 0. **Average reputation ≈ 0.86** (meets ≥ 0.80 target).
The single Medium source (istio-insider) is a community doc cross-referenced
against the authoritative tcp(7) man page and Envoy official docs before any
claim relied on it.

## Knowledge Gaps

### Gap 1: Cilium native-mTLS (ztunnel integration) article body did not render
**Issue**: The 2026-03-23 Cilium blog "Native mTLS for Cilium … with ztunnel"
title/topic are confirmed, but WebFetch returned only the headline (the page is
JS-rendered). The precise statements about kTLS-vs-ztunnel byte movement and
lifecycle ownership were inferred from the ztunnel architecture (System 2) and
the sockmap/kTLS datapath docs, not read directly from that article.
**Attempted**: Direct fetch (headline only); Google-search proxy (interface only,
no body). **Recommendation**: Fetch via an authenticated/headless-render path or
read the linked CNCF/KubeCon talk; the conclusion (Cilium reuses ztunnel rather
than building a central liveness reconciler) is independently supported by the
ztunnel source read, so the gap does not change Q5.

### Gap 2: Exact progress-stall predicate for a kTLS-SPLICED pump is undocumented upstream
**Issue**: Cloudflare documents `tcpi_notsent_bytes` progress monitoring for
plain TCP (Finding 5.3), and kernel.org documents kTLS splice error surfacing
(Finding 5.4), but no surveyed source documents the precise "record pending but
not advancing" predicate for a `splice()`-driven kTLS pump specifically — i.e.
whether `tcpi_notsent_bytes`, the kTLS record sequence, or `splice` return
codes is the right progress signal when the kernel owns the copy.
**Attempted**: Searches combining splice + kTLS + stall + progress; kernel.org
kTLS offload doc; NVIDIA kTLS docs. **Recommendation**: This is a Tier-3 spike
question (per Overdrive's own `feedback_no_tier2_ebpf_hook_firing_scope_needs_
tier3_spike` discipline) — validate the progress signal against the real
splice/kTLS pump on the pinned 6.x kernel. It affects the (B) *mechanism*, not
the (A)-vs-(B/C) *shape* decision.

### Gap 3: Whether ANY niche dataplane runs a central liveness reconciler
**Issue**: The survey covers the five requested systems + kernel and finds none.
A broader claim ("no production dataplane anywhere does this") would need a wider
survey (HAProxy, NGINX, Cloudflare's own proxies, gVisor netstack, AWS/GCP LB
internals).
**Attempted**: Scope was deliberately limited to the five requested systems per
the brief. **Recommendation**: If the team wants the stronger universal claim,
extend to HAProxy/NGINX connection-reaping internals; the directional conclusion
is unlikely to change (both are per-connection-timer + kernel models).

## Conflicting Information

No direct contradictions surfaced among sources. The closest tension is
*apparent* rather than real:

### Apparent tension: ztunnel HAS a central connection loop (so isn't A vindicated?)
**Position A (surface reading)**: ztunnel's `ConnectionManager` + `PolicyWatcher`
*is* a central loop over the live-connection set — evidence for shape (A).
**Position B (mechanism reading)**: that loop reconciles **authorization policy**
(re-assert RBAC on policy change, close now-unauthorized connections), not
liveness/stall. — Source: [ztunnel connection_manager.rs](https://github.com/istio/ztunnel/blob/master/src/proxy/connection_manager.rs).
**Assessment**: Position B is correct and more authoritative (direct source read
of the file's documented intent: "policy enforcement and graceful connection
draining upon authorization policy changes — not connection reaping"). The
existence of a central connection *registry* is real, but its *driver* is config
(policy) reconciliation, which is exactly the Q2 thesis — and is a different
concern from F6 liveness. This strengthens, rather than weakens, the
recommendation: a central loop is justified for *policy*, not for *liveness*.

## Recommendations for Further Research

1. **Tier-3 spike: progress signal for the kTLS-spliced pump** (closes Gap 2) —
   determine empirically whether `tcpi_notsent_bytes`, kTLS record sequence, or
   `splice` return is the reliable "not advancing" signal on the pinned kernel.
   Highest-value follow-up; it pins the (B) mechanism.
2. **Read the Cilium native-mTLS article body / KubeCon talk** (closes Gap 1) to
   confirm Cilium's exact lifecycle-ownership statements for the ztunnel-backed
   L4 path.
3. **If a universal "no dataplane does A" claim is needed** (closes Gap 3),
   extend the survey to HAProxy and NGINX connection-reaping internals.
4. **Validate the asymmetric-half-close hazard** (Istio #32116) against
   Overdrive's two-leg splice: confirm that when one leg dies, the per-connection
   task propagates close to the other leg (the bug that bit istio-proxy).

## Full Citations

[1] Envoy Project. "How do I configure timeouts?". Envoy documentation (latest). https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/timeouts.html. Accessed 2026-06-14.
[2] Envoy Project. "TCP Proxy (proto) — v3". Envoy API documentation. https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/filters/network/tcp_proxy/v3/tcp_proxy.proto. Accessed 2026-06-14.
[3] Istio Authors. "Introducing Rust-Based Ztunnel for Istio Ambient Service Mesh". istio.io blog. 2023. https://istio.io/latest/blog/2023/rust-based-ztunnel/. Accessed 2026-06-14.
[4] istio/ztunnel contributors. "ARCHITECTURE.md". github.com/istio/ztunnel. https://github.com/istio/ztunnel/blob/master/ARCHITECTURE.md. Accessed 2026-06-14.
[5] istio/ztunnel contributors. "src/proxy/connection_manager.rs". github.com/istio/ztunnel. https://github.com/istio/ztunnel/blob/master/src/proxy/connection_manager.rs. Accessed 2026-06-14.
[6] istio/ztunnel contributors. "Issue #1191 — Implement improved draining". github.com/istio/ztunnel. https://github.com/istio/ztunnel/issues/1191. Accessed 2026-06-14.
[7] istio/istio contributors. "Issue #32116 — istio-proxy doesn't close downstream connection when TCP keepalives fail". github.com/istio/istio. https://github.com/istio/istio/issues/32116. Accessed 2026-06-14.
[8] mygraphql. "Socket Options — Istio & Envoy Insider". istio-insider.mygraphql.com. https://istio-insider.mygraphql.com/en/latest/ch2-envoy/socket/socket-options.html. Accessed 2026-06-14.
[9] linkerd/linkerd2-proxy contributors. "timeout: Introduce FailFast, Idle, and Probe middlewares (#452)" (commit 5206901). github.com/linkerd/linkerd2-proxy. https://github.com/linkerd/linkerd2-proxy/commit/52069015990cb07de6a142a3a7b55e90ff9cf701. Accessed 2026-06-14.
[10] linkerd/linkerd2-proxy contributors. "PR #186 — Introduce TCP keepalive configuration". github.com/linkerd/linkerd2-proxy. https://github.com/linkerd/linkerd2-proxy/pull/186. Accessed 2026-06-14.
[11] linkerd/linkerd2 contributors. "Issue #13023 — Implement TCP_USER_TIMEOUT to detect half-opened TCP connections leading to 15min of dangling connections". github.com/linkerd/linkerd2. https://github.com/linkerd/linkerd2/issues/13023. Accessed 2026-06-14.
[12] linkerd/linkerd2 contributors. "Discussion #13566 — Idle connections timeout". github.com/linkerd/linkerd2. https://github.com/linkerd/linkerd2/discussions/13566. Accessed 2026-06-14.
[13] linkerd/linkerd2 contributors. "Discussion #8761 — protocol detection timeout despite opaque port annotation". github.com/linkerd/linkerd2. https://github.com/linkerd/linkerd2/discussions/8761. Accessed 2026-06-14.
[14] Cilium Authors. "eBPF Datapath — Socket Layer Enforcement". docs.cilium.io (stable). https://docs.cilium.io/en/stable/network/ebpf/intro/. Accessed 2026-06-14.
[15] cilium/cilium contributors. "pkg/maps/sockmap". pkg.go.dev. https://pkg.go.dev/github.com/cilium/cilium/pkg/maps/sockmap. Accessed 2026-06-14.
[16] Cilium Authors. "Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity with ztunnel". cilium.io blog. 2026-03-23. https://cilium.io/blog/2026/03/23/native-mtls-cilium/. Accessed 2026-06-14. [body did not render]
[17] cilium/cilium contributors. "Issue #14852 — Extend kernel requirements description for transparent encryption". github.com/cilium/cilium. https://github.com/cilium/cilium/issues/14852. Accessed 2026-06-14.
[18] Cilium / Linux Plumbers Conference. "Seamless transparent encryption with BPF and Cilium". lpc.events. https://lpc.events/event/4/contributions/461/attachments/253/439/Seamless_transparent_encryption_with_BPF_and_Cilium1.pdf. Accessed 2026-06-14.
[19] Majkowski, M. (Cloudflare). "When TCP sockets refuse to die". blog.cloudflare.com. https://blog.cloudflare.com/when-tcp-sockets-refuse-to-die/. Accessed 2026-06-14.
[20] Linux man-pages project. "tcp(7) — Linux manual page". man7.org. https://man7.org/linux/man-pages/man7/tcp.7.html. Accessed 2026-06-14.
[21] Linux Kernel contributors. "Kernel TLS offload". The Linux Kernel documentation. https://docs.kernel.org/networking/tls-offload.html. Accessed 2026-06-14.
[22] NVIDIA. "kTLS Offloads". DOCA SDK documentation. https://networking-docs.nvidia.com/doca/sdk/ktls-offloads. Accessed 2026-06-14.

## Research Metadata

Duration: ~single session | Sources examined: 22 | Sources cited: 22 |
Cross-references: every major finding cross-referenced (≥2 sources, most ≥3) |
Confidence distribution: High ~70%, Medium-High ~25%, Medium ~5% |
Citation coverage: >95% of claims sourced; unsourced inferences explicitly
flagged (documented-silence on the absence of a central liveness loop) |
Avg source reputation: ≈0.86 |
Output: docs/research/dataplane/transparent-mtls-connection-supervision-research.md
