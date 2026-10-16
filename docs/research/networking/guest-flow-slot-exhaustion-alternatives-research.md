# Research: Guest flow-slot exhaustion — is there a fundamentally better design than PSO-1 / PSO-2?

**Date**: 2026-10-15 | **Researcher**: nw-researcher (Nova) | **Confidence**: High on kernel facts (read in `vendors/linux` @ v7.0); Medium on the recommendation (three of its premises are unmeasured and each has a cheap spike, § Validation) | **Sources**: 31

Scope: GH #295 (`netns-density-295`). Research only. No design, ADR, spec, code or test file was edited. The kernel baseline is the vendored tree at tag `v7.0` (`vendors/linux`); every kernel claim below is cited `path:line @ v7.0` and was read in that tree in this session. Labels used throughout: **[FACT]** read in source or doc, **[MEASURED]** from an existing spike, **[INFERENCE]** my reasoning from facts, **[UNVERIFIED]** not checked.

## Executive Summary

**The headache has a root cause that PSO-1 and PSO-2 do not touch.** The guest slot pool exists because forwarding goes through `sk_skb` redirects, whose intermediate queue (`psock->ingress_skb`) is asynchronous, has no byte limit, and is flushed by a worker that needs the target socket alive (`skmsg.c:957-993`, `:695-743`). A flow that has "ended" can therefore still owe bytes, and that debt needs a per-flow cell until it is paid. The two proposals manage *who gets a slot*; the better question is *why a released flow still needs one*, and the answer splits in two.

1. **Most of cause 2 does not reach probes, by the design's own text and the kernel source [INFERENCE, two cheap checks open].** A draining direction completes as soon as the *target socket accepts* the last byte (`sent >= goal`), and a TCP target accepts bytes without the application reading until about half its send buffer is in flight (`skmsg.c:650`, `include/net/sock.h:2631-2634`) plus the peer's receive window (defaults 16 KiB send, 128 KiB receive, [kernel sysctl docs](https://docs.kernel.org/networking/ip-sysctl.html)). A probe request is tiny, so its drain completes with no application read. Separately, a dropped `GuestStream` closes with SHUTDOWN `0x3` while a direction is open, which the guest classifies as an abort with *no drain* (`feature-delta.md:2936-2941`, `:3313-3321`, `:4192-4194`), and the VM probe table never half-closes a probe stream (`:4105-4108`). PSO-1's cause 2 for probes therefore needs a probe runner that half-closes before dropping and an application whose unread tail exceeds the kernel buffers, and neither is specified. The only remaining probe cause is the guest owner's dispatch order (cause 1), which "ends before claims" removes without a wire change.

2. **For general flows the real disagreement is two accounting domains, and the transport already carries the signal that unifies them.** The host releases quota at `GuestStream` drop (`feature-delta.md:2940-2941`). The guest releases its slot when its own drain and close complete. If the host instead released the quota (and node slot) when it observes the guest's final SHUTDOWN `0x3` — the packet `virtio_transport_close` already sends when the guest owner closes V_g (`net/vmw_vsock/virtio_transport_common.c:1294-1295 @ v7.0`) and that the host classifier already classifies as "end of a completed flow" (`feature-delta.md:3318-3321`) — the invariant becomes **guest-held ⊆ host-held ∪ ends-not-yet-dispatched**. The pool then provably suffices, nobody is refused, and there is **no wire change**.

3. **Vsock credit cannot replace the guest slot, and the slow-reader-is-backpressure claim is only half true.** With sockmap, `virtio_transport_read_skb` returns vsock credit *before* the verdict runs (`virtio_transport_common.c:1776-1781 @ v7.0`), unlike the ordinary read path that credits on consumption (`:634`, `:653-655`). A TCP source stays bounded because its skb stays charged to the source socket until the target sends it [INFERENCE from `tcp_bpf.c:15-31`, `skmsg.c:957-993`]; a **vsock source is unbounded in kernel memory** [FACT: no limit on `ingress_skb`; runtime growth UNMEASURED]. This is a resource-safety gap (guest memory in the V_g→app direction, host memory in the V_h→destination direction) that no slot count bounds. A talk at LSF/MM/BPF 2026 reports the same family of defect as "TCP backpressure completely broken" in sockmap redirection ([Malla, 2026](https://bpfconf.ebpf.io/bpfconf2026/bpfconf2026_material/Backpressure%20Support%20for%20Sockmap%20based%20Redirection.pdf)).

**Recommendation (ranked in § Candidates).** (1) Run two cheap spikes that ground the premise (small-tail drain with a non-reading application; the existing K-PS2 spike). (2) Adopt **candidate A** (host retains quota and node slot until the guest's close is observed) **plus candidate B** (pin "ends before claims" at the guest owner), and drop PSO-1 and PSO-2's typed refusal. (3) Open a separate contract-shaping item for the unbounded vsock-source queue, which is the larger risk. Candidate A changes the meaning of `GuestStream` drop, so it needs the user's explicit approval (`.claude/rules/design.md` § "Surface material design decisions").

## Research Methodology

**Search strategy.** Read the problem in `feature-delta.md` (status bullets, § Core vocabulary, owner flow rules, `GuestStream` contract, probe runner, PSO block), ADR-0183, the existing drain research and spike T1-T6 findings; read the vendored Linux v7.0 sources for vsock, skmsg, tcp close/orphan; read Cilium at `/Users/marcus/git/cilium/cilium`; fetched upstream docs and source for Kubernetes, Nomad, Consul, Envoy, nginx, HAProxy, socat, Firecracker and the RFC.

**Source selection.** Kernel source and RFC (High, 1.0); Kubernetes/Firecracker/Cilium source (High); vendor docs for Envoy, nginx, Nomad, socat (Medium-High, 0.8); conference slides by a vendor engineer and an HAProxy git mirror (Medium, 0.6, cross-referenced).

**Quality standard.** Kernel claims: one authoritative source (the source line) is sufficient. Design-direction claims: marked INFERENCE and tied to a spike that would falsify them.

**Not done.** No code ran: this agent had no shell. Nothing here is a new measurement. Kata, gVisor and Consul's concurrency limits were not found or read (§ Knowledge Gaps).

## The problem as the design states it

- A guest TCP flow holds one drain-cell slot of `GUEST_FLOW_SLOTS = 4,096 + 4,096 + 8 = 8,200` (`feature-delta.md:1195-1211`). The claim "a guest TCP flow never finds its drain-cell slots full" is stated at `:1216-1226` and flagged unproven.
- A slot is released when both directions complete (`:3360-3363`) or on abort in the dispatch that handles the end (`:3364-3375`). A drain has no deadline (`:3341-3359`; ADR-0183 `:130-135`, rejected deadline `:170-171`).
- The host releases quota and node slot at `GuestStream` drop; a drop while a direction is open sends `0x3` and the guest aborts; after both directions ended cleanly the guest treats the `0x3` as the end of a completed flow (`:2936-2941`).
- PSO-1 (`:4210-4326`): per-probe fixed slot, atomic takeover, kind-4 offset-12 wire change, `DialPurpose::Probe { slot }`, new Quint module. PSO-2 (`:4384-4469`): ends-before-claims, typed `ENOBUFS` refusal, rests on K-PS2 (spike V-PSO2(a) not yet run).

## Findings

### F1 — What a draining flow actually waits for, and when that needs the application to read

**Evidence [FACT]:** the egress redirect appends the skb to the *target's* `ingress_skb` and schedules the target's worker (`net/core/skmsg.c:989-990 @ v7.0`). The worker sends only `if (sock_writeable(psock->sk))` (`:650-652`), else re-arms after one jiffy (`:719-729`). `sock_writeable` is `wmem_alloc < sk_sndbuf >> 1` (`include/net/sock.h:2631-2634 @ v7.0`). The direction is complete when the cell's `sent` reaches `goal` (`feature-delta.md:3341-3359`), i.e. when the last byte has entered the target's send path.

**Analysis [INFERENCE]:** a TCP target takes bytes into its send queue (up to about half of `sk_sndbuf` unacknowledged) and loopback delivers into the application's receive window with no `read()` by the application. Defaults are 16 KiB initial send buffer and 128 KiB receive buffer ([kernel sysctl docs](https://docs.kernel.org/networking/ip-sysctl.html)); the intake child has a 4 MiB receive buffer (`feature-delta.md:1188`). So an unread tail of roughly 10^5 bytes or less completes its drain without the application reading. Only a tail above that pins the slot. **[MEASURED, partial]:** spike T1 shows 2 kB flows with no pause reach `sent < goal` at first `RDHUP` in about 5 % of host directions and complete immediately after (`spike/findings-drain-t1-t6.md:54-58`); the paused-reader cells used 1 MiB tails only (`:49-52`). A small-tail, never-reading cell was not run.

**Confidence:** High on the code; Medium on the conclusion until V-1 (below) runs.

### F2 — A probe flow does not reach R2-DRAIN on the design's own text

**Evidence [FACT]:** TCP-connect probe: "the stream is dropped at once (the guest application sees its accepted connection reset)" (`feature-delta.md:4107`). `Drop` while either direction is open sends `0x3` and "the guest aborts the application connection with a reset" (`:2936-2938`; kernel side `virtio_transport_common.c:1294-1295 @ v7.0`). The classifier maps `0x3` while any direction is open to `EndAbort` (`:3317-3321`), and "an aborted flow has no drain to wait for" (`:4153-4154`, `:4192-4194`). The HTTP probe reads a response and times out or drops (`:4108`).

**Analysis [INFERENCE]:** cause 2 for a probe needs the runner to call `shutdown_write` on its stream (clean `0x2`) *and* the guest's own direction to have ended *and* an unread tail above the F1 threshold. The probe table describes none of these. This is the premise-grounding question of `.claude/rules/development.md` § "Ground the premise": name the production path that produces the state. The design names none for probes. **If the runner never half-closes, PSO-1's takeover defends a state no production path produces.**

**Confidence:** Medium. The one open point is whether the HTTP client path (not yet written; DELIVER is stopped) half-closes.

### F3 — Vsock credit is returned before the sockmap target has accepted anything

**Evidence [FACT]:**
- Ordinary read: `virtio_transport_stream_do_dequeue` decrements `rx_bytes`, advances `fwd_cnt` and sends a credit update only when the *reader* dequeues (`virtio_transport_common.c:634`, `:653-655 @ v7.0`).
- Sockmap read: `virtio_transport_read_skb` does `virtio_transport_dec_rx_pkt(vvs, pkt_len, pkt_len)` (`:1776`), then `virtio_transport_send_credit_update(vsk)` (`:1779`), *then* `return recv_actor(sk, skb)` (`:1781`), which runs the verdict and queues the skb at the target (`skmsg.c:1236-1266`, `:1076-1078`).
- The sender's budget is `min(peer_buf_alloc, buf_alloc) − (tx_cnt − peer_fwd_cnt)` (`:825-832`, `:889-903`); `peer_fwd_cnt` comes from every received header (`:1519-1523`). Default `buf_alloc` is 256 KiB (`net/vmw_vsock/af_vsock.c:199-201 @ v7.0`), settable with `SO_VM_SOCKETS_BUFFER_SIZE` (`:2030-2033`), ceiling `0xFFFFFFFF` (`include/linux/virtio_vsock.h:139`). A receiver whose `buf_used + len > buf_alloc` silently drops the packet (`:447`, `:1378-1382`).
- No limit applies when the verdict queues at the target: `sk_psock_skb_redirect` ends with `skb_queue_tail(&psock_other->ingress_skb, skb)` and no accounting check (`skmsg.c:989`). The ingress path, by contrast, checks `sk_rmem_alloc > sk_rcvbuf` (`:535`).

**Analysis [INFERENCE]:** with a vsock source, "credit returned" means "the verdict took it", not "the target accepted it". Credit therefore **cannot replace the guest slot**, and cannot make the host's quota the source of truth, and it does not bound memory. The V_g→application direction on the guest and the V_h→destination direction on the host both have this shape. A TCP source differs: `tcp_eat_skb` advances `copied_seq` and may send a window update but does not orphan the skb (`net/ipv4/tcp_bpf.c:15-31`), and `sk_psock_skb_redirect` does not orphan it either, so the skb stays charged to the source's `sk_rmem_alloc` until the worker frees it and the source's advertised window stays closed [INFERENCE; not measured]. This matches the Azure talk's description of `sk_skb`: "Rely on backpressure from TCP stack on source socket buffer fill up".

**Confidence:** High on the code; the memory-growth consequence is UNMEASURED (V-3).

**Why it matters for the question asked:** the slot pool bounds the *number* of draining flows, not their *bytes*. A guest application that never reads while the host sender keeps sending can grow guest kernel memory, and a destination that never reads while a guest sends can grow *host* kernel memory, with no slot count in the way. The ADR-0183 statement "a slow reader is backpressure, not a fault" holds for TCP-source directions only.

### F4 — Releasing a flow's resources at end-of-flow works only for bytes already in the socket's send queue

**Evidence [FACT]:**
- `close()` of a sockmap'd socket runs `sock_map_close`: `sk_psock_stop`, `cancel_delayed_work_sync(&psock->work)`, `sk_psock_put`, then `saved_close` (`net/core/sock_map.c:1674-1706 @ v7.0`). The psock's destroy zaps `ingress_skb` (`skmsg.c:832-841`, `:863-886`). **Bytes still in the psock queue are dropped on close.**
- A plain `tcp_close` with queued send data does not drop it: it sends the FIN after the data (`tcp_close_state`/`tcp_send_fin`, `net/ipv4/tcp.c:3229-3260 @ v7.0`), orphans the socket (`:3264-3274`), and the kernel keeps delivering and probing a zero window (RFC 9293 §3.8.6.1: "the sender MUST NOT abort the connection when the probe timer expires", [RFC 9293](https://www.rfc-editor.org/rfc/rfc9293.html)). Unread *receive* data at close triggers an RST (`tcp.c:3196-3224`).
- The kernel bounds the pool of such orphans globally, not per flow: `tcp_max_orphans` — "If this number is exceeded orphaned connections are reset immediately" ([kernel docs](https://docs.kernel.org/networking/ip-sysctl.html)); `tcp_check_oom` (`tcp.c:3147-3173`), `tcp_out_of_resources` (`tcp_timer.c:83-142`), and for an orphan under a zero window `tcp_probe_timer` (`tcp_timer.c:395-437`). An orphan in FIN_WAIT2 is aborted after `tcp_fin_timeout` (60 s default, same docs).

**Analysis [INFERENCE]:** candidate (a) of the task ("release the slot at the flow end and let the kernel socket buffer hold the tail, orphan-like") is **already true once `sent ≥ goal`**: the tail is then in the target's send queue and the owner could `close()` it and let the kernel finish. Before `sent ≥ goal` the tail is in `ingress_skb`, which `close()` destroys, so an earlier release would truncate. After the drain completes, what pins the slot is the *other* direction still being open, which has nothing to do with unread data. So (a) removes nothing from the current design for the tail; its real content is the kernel's *policy*: a global orphan cap with reset on pressure (F6).

### F5 — vsock half-close and unread data

**Evidence [FACT]** (`virtio_transport_common.c @ v7.0`): a received SHUTDOWN ORs `RCV`/`SEND` into `peer_shutdown` (`:1443-1447`); when both are set and the socket has no unread data, the *guest kernel itself* replies RST and closes the transport (`:1448-1452`), independent of the owner. With unread data it does neither, and the data stays readable. `close()` sends SHUTDOWN `0x3` unless the peer already sent `0x3`, in which case it replies RST (`:1288-1295`). A listener whose accept queue is full answers RST (`:1544-1547`), which is the kernel's own bound on pending connection requests. `shutdown(SHUT_WR)` sends `SEND` only (`feature-delta.md:2929-2931`; T5 measured `0x2` vs `0x3`, `spike/findings-drain-t1-t6.md:14`).

**Analysis:** the host cannot learn the guest *owner's* slot release from a host-initiated `0x3`, because the guest kernel answers it before the owner acts. It *can* learn it from the guest's `0x3` when the guest owner closes V_g after a clean exchange, because no kernel auto-close precedes it: with only `SEND` received, `peer_shutdown != SHUTDOWN_MASK` (`:1448`), so V_g stays open until the owner closes it. This is the signal candidate A uses.

### F6 — Prior art bounds a non-reading peer by deadline or by byte cap and reset, or by real end-to-end backpressure; none waits forever with a held slot

| System | Mechanism | Source |
|---|---|---|
| socat | After one side's EOF the other's write side is shut down, then "waits <timeout> seconds before terminating. Default is 0.5 seconds" (`-t`); `-T` inactivity timeout | [socat man page](https://manpages.debian.org/testing/socat/socat.1.en.html) |
| Envoy | Listener `per_connection_buffer_limit_bytes` soft limit, default 1 MiB, with read-disable watermarks; `per_connection_buffer_high_watermark_timeout` closes a connection above the high watermark for too long; HTTP `delayed_close_timeout` default 1000 ms after local close | [Envoy flow-control FAQ and API search](https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/flow_control), [HCM proto](https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/filters/network/http_connection_manager/v3/http_connection_manager.proto) |
| nginx | `lingering_time` 30 s, `lingering_timeout` 5 s, `send_timeout` 60 s; `reset_timedout_connection` to "avoid keeping an already closed socket with filled buffers in a FIN_WAIT1 state for a long time" | [nginx core module](https://nginx.org/en/docs/http/ngx_http_core_module.html) |
| HAProxy | `timeout client-fin` / `server-fin`: inactivity timeout for half-closed sessions, added because half-closed sessions pile up in FIN_WAIT with no way for haproxy to remove them | [HAProxy commit "implement half-closed timeouts"](https://git01.mediatek.com/plugins/gitiles/haproxy/+/05cdd9655db32bfd6cb9c503f3474390e346903b) (mirror; Medium) |
| Firecracker vsock | Userspace copy; TX buffered per connection `CONN_TX_BUF_SIZE`; kill timer `CONN_SHUTDOWN_TIMEOUT_MS` armed on shutdown; credit follows what the host stream consumed | [connection.rs](https://raw.githubusercontent.com/firecracker-microvm/firecracker/main/src/vmm/src/devices/virtio/vsock/csm/connection.rs) |
| Linux TCP | Orphans held until the peer stops answering probes (no per-flow deadline, RFC 9293), capped globally by `tcp_max_orphans` and memory with RST | F4 |

**Conflicting positions** are recorded in § Conflicting Information. The relevant observation is that Linux TCP itself uses no per-flow deadline for a *live* peer; it bounds the *pool* and resets under pressure.

### F7 — Cilium keeps no per-flow drain state because it never forwards bytes in BPF

**Evidence [FACT]** (`/Users/marcus/git/cilium/cilium`, HEAD): there are no `sk_msg` or `sockops` programs in `bpf/`; the only matches are the UAPI header copy (`bpf/include/linux/bpf.h:316,975,2808,3020`). The Azure talk lists Cilium under sockmap users as "(A long time ago)" and says "not many users for redirection in prod".
- Socket load balancing (`bpf/bpf_sock.c`) keeps one reverse-NAT entry per socket cookie in an **LRU hash**, `LB4_REVERSE_NAT_SK_MAP_SIZE 262144` (`bpf/node_config.h:82`, `bpf/lib/sock.h:36-42`), written at connect (`bpf_sock.c:145-171`), deleted by `cgroup/sock_release` (`:173-185`, `:1287-1310`), and checked for staleness on use: a stale entry is deleted and the lookup fails (`:595-614`). The state is a cache, so eviction is safe.
- Backend removal terminates sockets by abort, not drain: `bpf_sock_destroy` from `iter/tcp` and `iter/udp` programs (`bpf/bpf_sock_term.c:39-170`).
- L7 redirect is packet level: `sk_assign` of the proxy socket at tc ingress (`bpf/lib/proxy.h:30`, `:59`), and the bytes then flow through Envoy's userspace buffers (F6).

**Analysis [INFERENCE]:** Cilium's answers to "non-reading peer" are an evictable cache, abort on teardown, and a userspace proxy with watermarks. Overdrive's cells must be exact and non-evicting (an LRU eviction stalled 1 of 10,000 flows, `feature-delta.md:264`), so Cilium's cheap trick is unavailable. The prior drain research (`vsock-sockmap-half-close-and-drain-research.md:336-352`) reached the same conclusion for the older Cilium `sk_msg` acceleration.

### F8 — Probe isolation: other orchestrators do not draw probe connections from a pool shared with workload flows, and discard errors

**Evidence [FACT]:** the kubelet TCP probe dials from the kubelet's own sockets and closes at once: `DoTCPProbe` dials, returns `Failure` on error, otherwise `conn.Close()` and `Success` ([tcp.go](https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/probe/tcp/tcp.go)). Each probe has one goroutine ("Each worker has a go-routine associated with it which runs the probe loop"), and "Prober error, throw away the result" ([worker.go](https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/kubelet/prober/worker.go)). Nomad script checks "run inside the task" and need long timeouts because namespace setup can take seconds ([Nomad check docs](https://developer.hashicorp.com/nomad/docs/job-specification/check)). Consul's TCP check default timeout is 10 s ([Consul docs, search result](https://developer.hashicorp.com/consul/docs/services/usage/checks)); I found no documented limit on concurrent checks.

**Analysis [INFERENCE]:** "no slot for a probe" cannot occur there, because there is no slot. A probe is one fresh connection whose only shared resource is the application's accept queue. The kubelet's "throw away the result" is the same decision as Overdrive's PROBE-NOSAMPLE (`feature-delta.md:4110-4132`). Overdrive meets the problem only because the guest forwards by slot-indexed cells, and the guest cannot tell a probe flow from a relay flow at all today: kind 4 carries `0` at offset 12 (`:4231-4237`).

### F9 — Sockmap backpressure is a known-open upstream problem, and not for vsock

**Evidence [FACT, third party]:** Malla (Azure Container Networking), LSF/MM/BPF 2026: "Why Not? TCP Backpressure completely broken; Misc. Bugs; Performance issues with small messages"; a demo shows classic sockmap receiver memory growing past 10 MB in 20 s against a 64 KB `sk_rcvbuf`, flat with the proposed `BPF_F_PERMANENT`; the proposal covers `sk_msg` and says for `sk_skb`: "Rely on backpressure from TCP stack on source socket buffer fill up". The slide of 2025-26 fixes includes "bpf, vsock: Fixes related to sockmap/sockhash redirection (SK_DROP on unsupported af_vsock; rx_bytes/msg_count updates on read_skb)" ([slides](https://bpfconf.ebpf.io/bpfconf2026/bpfconf2026_material/Backpressure%20Support%20for%20Sockmap%20based%20Redirection.pdf)).

**Analysis [INFERENCE]:** the demonstrated path is `sk_msg` ingress, not Overdrive's `sk_skb` egress, so it corroborates the family but is not a measurement of ours. No upstream fix for vsock-source `sk_skb` backpressure is visible; none should be assumed on the pinned 7.0 kernel.

## Candidates

Every candidate states mechanism, evidence, what it removes, risks, and whether it needs a wire change. Ranking criteria: simplicity, correctness, no fallback or heuristic (`.claude/rules/design.md` § "Design the correct mechanism").

### A — The host retains quota and node slot until it observes the guest's close (task item c)

- **Mechanism.** `GuestStream` drop after the host's own direction ended cleanly (`0x2`) no longer closes V_h at once. The owner keeps V_h registered (a fd and the slot counters) until the guest's final SHUTDOWN `0x3` arrives (F5), then releases quota and node slot. A drop with a direction open still aborts at once (`0x3`), unchanged. Guest-opened flows use the same rule at the host's completion.
- **Evidence.** F5 (no kernel auto-close precedes the guest owner's close when only `SEND` was received); `feature-delta.md:3318-3321` (the host classifier already distinguishes a `0x3` after both directions ended); `:3341-3363` (the guest owner closes V_g when both directions complete); ADR-0147 already counts draining flows at the host (`:3541-3550`).
- **Removes.** Class (iii) flows of PSO-2 by construction (no guest flow the host released and that is still draining), PSO-1's probe slots, takeover, `DialPurpose::Probe { slot }`, `ProbeSlot`, `probe_slots.qnt`, and PSO-2's refusal path (`general_slots.qnt` shrinks to the ordering invariant). The claim at `:1216-1226` becomes provable: guest-held ⊆ host-held ∪ ends-not-yet-dispatched.
- **Risks.** (1) It changes `GuestStream` drop semantics (`:2936-2941`) and the `flow_quota` model (`DrainingFlowHoldsItsSlot` extends to the guest's drain). (2) A quota-exempt probe that half-closes would linger and, over many runs, accumulate node slots; so a probe stream's drop must always abort (a structural rule on `DialPurpose::Probe` streams). (3) A guest that never closes holds the allocation's own quota slots, which is the stated bulkhead (`:3384-3386`).
- **Wire change: none.** **Cost: moderate** (host owner state, Quint, one ADR-0147 sentence).
- **Does not remove** cause 1 for aborts (the guest kernel answers `0x3` before the owner dispatches), which is B.

### B — Pin "ends before claims" at the guest owner (PSO-2 first half)

- **Mechanism.** Before deciding no slot is free, the owner consumes the flow-event ring, reads the loss counters and collects pending terminal readiness (`feature-delta.md:4393-4406`).
- **Evidence.** K-PS2 by source (`:4161-4179`): vhost queues host→guest packets FIFO (`drivers/vhost/vsock.c:125`, `:296 @ v7.0`) and the guest processes its RX ring in order. Not run on a microVM (V-PSO2(a)).
- **Removes** cause 1 for probes and general flows with no wire change and no new surface.
- **Risks.** Rests on K-PS2; a falsification returns it to DESIGN (`:4440-4459`).
- **Wire change: none. Cost: low.**

### C — PSO-1 as proposed

Wire change (offset 12), new core newtype and error, changed public enum, new Quint module and hazards (`:4348-4382`). Bounds only the probe slots. By F2 it defends a state no specified probe path produces, and A + B already bound probes. **Rank: last of the live candidates.**

### D — Probes take a path with no drain cells

- **D1, a userspace relay in the guest owner for probe flows.** Probe flows need no half-close fidelity and carry little data, so a small bounded copy loop (two fds, fixed buffers, natural backpressure) has no cells, no epoch, no slot index, and cannot be pinned. It needs the guest to know a flow is a probe, which today it cannot (F8): **a wire change** (one flag, simpler than PSO-1's slot number and takeover). `design.md` asks for a stated reason before choosing userspace over BPF; "low volume, no drain obligation" is that reason. **Cost: moderate; second data path in the owner.**
- **D2, run the probe inside the guest (Nomad's model, F8).** Removes probe flows from the pool entirely. Needs a result message (**wire change**), a guest-side HTTP client, and trusts the guest to report its own health; a hung guest then reports nothing. It changes approved probe decisions (PD-1 to PD-9, `feature-delta.md:49-64`). **Cost: high.**

### E — Release at end of flow, kernel holds the tail (task item a)

As analysed in F4 it is already what happens once `sent ≥ goal`; before that it truncates. Not a change to adopt, but the explanation of why A cannot be replaced by an earlier release.

### F — Bound the drain by kernel buffer size (task item b)

- **F1, credit on free (kernel patch).** Defer the vsock credit update to the skb's release, so a vsock source is throttled by the target's real consumption (it would bound `ingress_skb` at `buf_alloc` per flow). Correct, and the only candidate that restores end-to-end backpressure; it patches `virtio_transport_read_skb` on the pinned kernel (`virtio_transport_common.c:1776-1781`). Needs a user ruling on carrying a kernel patch (ADR-0068 pins a kernel; `CLAUDE.md` allows changing the Cloud Hypervisor fork, not the kernel). **Wire change: none. Cost: high.**
- **F2, a byte cap with abort.** The verdict already counts `fwd`; the final hop counts `sent`. `fwd − sent > CAP` could return `SK_DROP`, which the A3 path already reports as `RedirectRefused` and aborts the flow (`feature-delta.md:3367-3369`). No new record kind. It bounds memory but truncates a legitimate transfer whose reader is merely slow, since the sender cannot be throttled; tails between the kernel buffers and `CAP` still pin slots. A resource policy, not a fix for slots. **Wire change: none. Cost: low-moderate.**

### G — Evict the oldest host-released draining flow when the pool is full

The Linux orphan model (F4): the guest knows a flow is host-released when it classified the final `0x3` after both directions ended. Evicting it needs no wire change and never refuses a new flow. It truncates an unread tail the host already considers delivered, which violates the approved R2-DRAIN no-truncation property; it is a policy choice (who pays: the new flow under PSO-2, or the stale tail), so it needs the user's decision. **Rank below A.**

### H — Cells in socket-local storage instead of an indexed pool

`bpf_sk_storage_get` is available to `sock_ops` and tracing programs but **not** to `sk_skb` programs (`net/core/filter.c:8638-8676 @ v7.0`, versus `:8556-8580`), so the verdict cannot reach it. It would delete the slot index, epochs (A2) and `GUEST_FLOW_SLOTS`, but not the bound (fds and memory become the bound) and it invalidates the measured MMAPABLE-array configuration (T2). **Not recommended.**

### I — Userspace relay for all flows

Restores backpressure and removes the drain entirely (the ztunnel/Firecracker model, F6). `splice(2)` on vsock falls back to a copy (`net/socket.c:1129-1138 @ v7.0`; `af_vsock.c` has no `splice_read`). It abandons ADR-0147's kernel-forwarded pairs and the density target. **Not recommended.**

### Ranking

| Rank | Candidate | Simplicity | Correctness | No fallback / heuristic | Wire change |
|---|---|---|---|---|---|
| 1 | **A + B** (host retains until guest close; ends before claims) | High: deletes PSO-1 and the refusal path | Invariant provable; B rests on K-PS2 | Yes: atomic accounting at one owner | None |
| 2 | B alone, no A, with typed refusal for class (iii) (PSO-2 as written) | High | Pool can still fill with a broken VM's own tails; refusal is the outcome | Typed refusal, no heuristic | None |
| 3 | D1 (guest relay for probes) | Medium | Probes cannot be pinned | Yes | One flag |
| 4 | F2 (byte cap with abort) as an add-on to 1 | Medium | Bounds memory; truncates slow readers | A policy constant | None |
| 5 | G (evict orphans) | Medium | Violates approved no-truncation | Heuristic | None |
| 6 | C (PSO-1) | Low | Bounds probes only; premise unproven | Yes | Yes |
| 7 | D2, F1, H, I | Low | Correct but large | Yes | D2: yes; others: no |

## Recommendation

1. **Ground the premise first (V-1, V-2 below).** If a small unread tail drains with a non-reading application, and the probe runner never half-closes, PSO-1 has no production trigger.
2. **Adopt A + B.** State the invariant "guest-held ⊆ host-held ∪ ends-not-yet-dispatched". Make a `Probe` stream's drop an abort by contract. Delete PSO-1; do not add PSO-2's refusal (the pool cannot fill by construction; an unreachable `None` is `unreachable!`, per `.claude/rules/rust.md`).
3. **Open a separate item for the unbounded vsock-source queue (F3)** with V-3 first. Candidate F2 is the cheap bound; F1 is the correct one.
4. **User decisions needed (numbered for `design.md`):** (D-1) `GuestStream` drop releases quota at the guest's close, not at drop (changes `feature-delta.md:2936-2941` and ADR-0147's release point); (D-2) a `Probe` stream's drop always aborts; (D-3) PSO-1 and PSO-2's refusal are withdrawn if V-1 and V-2 hold; (D-4) whether F1 or F2 bounds the vsock-source queue. No GitHub issue is proposed or created here.

## Validation (each is a spike or a source read; none is run here)

| Id | Question | Method | Falsified if |
|---|---|---|---|
| V-1 | Does a ≤128 KiB tail drain to `sent ≥ goal` with an application that never reads? | Existing drain harness (`spike/findings-drain-t1-t6.md`), reader paused forever, tails 1 KiB / 16 KiB / 128 KiB / 1 MiB, clean half-close, Linux 7.0, microVM | A ≤16 KiB tail pins the slot |
| V-2 | K-PS2: the end of a released flow precedes the next request's bytes at the guest | The design's V-PSO2(a) (`feature-delta.md:4440-4459`) | One later request readable before an earlier end record or loss count |
| V-3 | Does a vsock-source direction grow kernel memory without bound when its target never reads? | Guest sends 1 GiB to a non-reading destination, host sends to a non-reading guest application; record `Slab`/`SUnreclaim` and `fwd − sent` on 7.0 | Memory flat near `buf_alloc` |
| V-4 | Does the guest owner's `close(V_g)` after a clean exchange produce a host-classified `0x3` with no auto-close before it? | One microVM batch of the integrated owner | The host sees a reset or nothing |

## Source Analysis

| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| Linux v7.0 vsock, skmsg, sock_map, tcp, filter, socket sources (`vendors/linux`) | local vendored | High | primary source | 2026-10-15 | Y (prior research doc, spike T1-T6) |
| feature-delta, ADR-0183, drain research, spike findings | local repo | High (SSOT) | internal | 2026-10-15 | Y |
| kernel sysctl docs | docs.kernel.org | High | official | 2026-10-15 | Y (tcp.c) |
| RFC 9293 | rfc-editor.org | High | standard | 2026-10-15 | Y |
| Kubernetes `tcp.go`, `worker.go` | raw.githubusercontent.com | High | primary source | 2026-10-15 | N |
| Firecracker `connection.rs` | raw.githubusercontent.com | High | primary source | 2026-10-15 | Y (prior doc F6) |
| Cilium source (`bpf/`) | local checkout | High | primary source | 2026-10-15 | Y (talk) |
| Envoy docs (flow control, HCM) | envoyproxy.io | Medium-High | official | 2026-10-15 | Y (two pages) |
| nginx core module docs | nginx.org | Medium-High | official | 2026-10-15 | N |
| socat man page | manpages.debian.org | Medium-High | official | 2026-10-15 | N |
| Nomad check docs | developer.hashicorp.com | Medium-High | official | 2026-10-15 | N |
| Consul checks docs (search result only) | developer.hashicorp.com | Medium-High | official | 2026-10-15 | N |
| HAProxy half-closed timeout commit | git01.mediatek.com (mirror) | Medium | VCS mirror | 2026-10-15 | N |
| Malla, LSF/MM/BPF 2026 slides | bpfconf.ebpf.io | Medium-High | conference | 2026-10-15 | Partial (matches skmsg.c reading) |

Reputation: High 8 (57 %), Medium-high 5 (36 %), Medium 1 (7 %). Average about 0.92.

## Knowledge Gaps

### Gap 1: Runtime memory growth of the vsock-source queue
**Issue:** F3's consequence is source reading only. **Attempted:** searched upstream for a measurement of `sk_skb` egress with a vsock source; found only the `sk_msg` demo. **Recommendation:** V-3.

### Gap 2: Whether the HTTP probe path half-closes
**Issue:** the runner for the HTTP probe is not written; F2's premise depends on it. **Attempted:** read `feature-delta.md:4105-4132`. **Recommendation:** pin "a probe stream's drop aborts" in the contract (D-2) or run V-1 with a half-closing probe.

### Gap 3: Kata, gVisor, Consul concurrency
**Issue:** no Kata agent or gVisor netstack source read; no Consul concurrent-check limit found. **Attempted:** Consul docs search (timeouts only). **Recommendation:** low value; all are userspace copy paths or per-check goroutines.

### Gap 4: Envoy byte-limit semantics beyond docs; HAProxy primary doc
**Issue:** Envoy limits were read from docs search output, not source; HAProxy via a git mirror. **Recommendation:** none needed for the conclusions.

### Gap 5: Ubuntu 7.0.0-29 vs upstream v7.0
**Issue:** carried over from the prior research: stable backports are not enumerated. None of the cited functions is known to be touched.

## Conflicting Information

### Conflict 1: A drain with no deadline vs deadlines in prior art
**Position A:** "A slow reader is backpressure, not a fault"; a deadline "turns a slow reader into truncation or a spurious abort" — ADR-0183 `:130-135`, `:170-171`; RFC 9293 §3.8.6.1 forbids aborting a connection whose peer answers zero-window probes. Reputation 1.0.
**Position B:** socat (0.5 s), Envoy (high-watermark timeout, 1000 ms delayed close), nginx (30-60 s), HAProxy (client-fin), Firecracker (kill timer) all bound a non-reading peer by time or size. Reputation 0.6-1.0.
**Assessment:** both are authoritative at their own layer. The difference is the layer: TCP needs no deadline because the sender is throttled and the kernel caps the orphan *pool*; the proxies bound a userspace buffer they cannot otherwise throttle. A sockmap with a vsock source is the second case (F3), which favours a byte bound (F2) over a time bound, and a *pool-wide* bound over a per-flow one.

## Recommendations for Further Research

1. Run V-1 and V-2 now (both reuse existing harnesses); they decide whether PSO-1 has a trigger.
2. Run V-3 before DELIVER; it may change a contract (a byte bound or a kernel patch).
3. Have the architect re-state `GuestStream` drop and the `flow_quota` model under candidate A, and re-run the `flow_quota` and `guest-flow-owner` Quint checks, once D-1 to D-3 are decided.

## Full Citations

[1] Linux kernel v7.0 (`vendors/linux`), `net/vmw_vsock/virtio_transport_common.c` (lines cited in text), `net/vmw_vsock/af_vsock.c:199-201,2030-2033`, `include/linux/virtio_vsock.h:139`, `drivers/vhost/vsock.c:125,296,555`.
[2] Linux v7.0, `net/core/skmsg.c:535,646-656,670-747,817-886,957-993,1076-1078,1236-1266`; `net/core/sock_map.c:1674-1706`; `net/ipv4/tcp_bpf.c:15-31`; `include/net/sock.h:2631-2641`; `net/core/filter.c:8556-8580,8638-8676`; `net/socket.c:1129-1138`.
[3] Linux v7.0, `net/ipv4/tcp.c:3147-3345`; `net/ipv4/tcp_timer.c:83-142,395-437`.
[4] Linux kernel documentation. "IP Sysctl". https://docs.kernel.org/networking/ip-sysctl.html. Accessed 2026-10-15.
[5] Eddy, W. "RFC 9293: Transmission Control Protocol (TCP)". IETF. 2022. https://www.rfc-editor.org/rfc/rfc9293.html. Accessed 2026-10-15.
[6] Malla, H. "Backpressure for Sockmap based Redirection". LSF/MM/BPF 2026, Zagreb. https://bpfconf.ebpf.io/bpfconf2026/bpfconf2026_material/Backpressure%20Support%20for%20Sockmap%20based%20Redirection.pdf. Accessed 2026-10-15.
[7] Cilium, local checkout `/Users/marcus/git/cilium/cilium`: `bpf/bpf_sock.c`, `bpf/bpf_sock_term.c`, `bpf/lib/sock.h`, `bpf/lib/proxy.h`, `bpf/node_config.h`. Accessed 2026-10-15.
[8] Kubernetes. `pkg/probe/tcp/tcp.go`, `pkg/kubelet/prober/worker.go`. https://raw.githubusercontent.com/kubernetes/kubernetes/master/pkg/probe/tcp/tcp.go. Accessed 2026-10-15.
[9] HashiCorp. "Nomad check block". https://developer.hashicorp.com/nomad/docs/job-specification/check. Accessed 2026-10-15.
[10] HashiCorp. "Define health checks". Consul. https://developer.hashicorp.com/consul/docs/services/usage/checks. Accessed 2026-10-15.
[11] Envoy. "How do I configure flow control?". https://www.envoyproxy.io/docs/envoy/latest/faq/configuration/flow_control; "HTTP connection manager (proto)". https://www.envoyproxy.io/docs/envoy/latest/api-v3/extensions/filters/network/http_connection_manager/v3/http_connection_manager.proto. Accessed 2026-10-15.
[12] nginx. "Module ngx_http_core_module". https://nginx.org/en/docs/http/ngx_http_core_module.html. Accessed 2026-10-15.
[13] Rieger, G. "socat manual page". https://manpages.debian.org/testing/socat/socat.1.en.html. Accessed 2026-10-15.
[14] HAProxy. "MEDIUM: session: implement half-closed timeouts (client-fin and server-fin)". https://git01.mediatek.com/plugins/gitiles/haproxy/+/05cdd9655db32bfd6cb9c503f3474390e346903b. Accessed 2026-10-15.
[15] Firecracker. `src/vmm/src/devices/virtio/vsock/csm/connection.rs`. https://raw.githubusercontent.com/firecracker-microvm/firecracker/main/src/vmm/src/devices/virtio/vsock/csm/connection.rs. Accessed 2026-10-15.
[16] Overdrive. `docs/feature/netns-density-295/feature-delta.md` (lines cited in text); `docs/product/architecture/adr-0183-a-half-close-drain-completes-on-a-kernel-record-without-a-deadline.md:130-171`; `docs/research/networking/vsock-sockmap-half-close-and-drain-research.md:166-190,249-287,336-352`; `docs/feature/netns-density-295/spike/findings-drain-t1-t6.md:9-19,49-101`.

## Research Metadata

Duration: about 60 min | Examined: about 45 | Cited: 31 (counting each kernel file group and local artifact) | Cross-refs: 9 | Confidence: High 55 %, Medium 40 %, Low 5 % | Tool failures: no shell (no spike or code run); two WebFetch failures (socat.html self-signed certificate, LKML lore 403), replaced by the Debian man page and the Azure slides; Envoy page 404, replaced by search result | Output: `docs/research/networking/guest-flow-slot-exhaustion-alternatives-research.md`
