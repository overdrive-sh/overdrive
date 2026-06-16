# Research: Lossless sockops→kTLS Pre-Arm Hold — BPF-Only vs. Kernel-Source Patch

> **Question**: Can the sockops→kTLS plaintext race window be closed **losslessly** (zero dropped bytes, no reset) using ONLY runtime-loadable BPF — given a host-socket workload may `write()` before the userspace agent finishes the rustls TLS 1.3 handshake and arms kTLS? Or does lossless hold-until-armed require a kernel-source patch?
> **Kernel target**: pinned **6.18 LTS** (ADR-0068); dev/validation at **7.0** (Ubuntu 26.04 spike). We ship our own Yocto appliance kernel — a patch is *technically* on the table but costs ongoing kernel-C ownership.
> **Verdict**: **(b) NOT achievable in BPF-only — a kernel-source patch (or a userspace-proxy data-path hop) is required for true lossless hold.** *[set in banner; defended in Verdict section]*
> **Missing primitive (one line)**: The `sk_msg` verdict set is `{PASS, DROP, REDIRECT}` with no `HOLD`/`DEFER`/`-EAGAIN-retry`; no BPF helper or hook can *asynchronously block an in-flight `sendmsg()` and release it on an external "armed" signal* without either dropping bytes or moving them through an agent-held socket.
> **Q1 recommendation**: **Ship lossy DROP-RESET as v1** (confidentiality-correct; reset on `drops>0`; document the server-speaks-first assumption), and track lossless as a follow-up via the **transient sockmap-redirect-to-holding-socket** hybrid (re-injected after arm) — the only *runtime-loadable* path that is lossless, accepting it re-enters an agent in the data path during the handshake window. A true zero-hop lossless hold needs a kernel patch.

**Date**: 2026-06-11 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (the (b) verdict + missing-primitive); Medium (the (c) hybrid feasibility) | **Sources**: 30 external (avg reputation 0.92) + 2 internal prior-work refs

> **Cross-reference, not duplication.** This doc narrows specifically to the *BPF-only vs. kernel-patch* axis for a **lossless** hold. The broader mechanism survey (sk_msg verdict set, TLS_BASE cleartext passthrough, ESTABLISHED_CB synchronous ordering, proxy-redirect prior art, comparison matrix) lives in `docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md` and is treated as established prior work here. The Slice-00 spike (`docs/feature/transparent-mtls-host-socket/spike/findings.md`) confirmed the lossy DROP-on-7.0 behaviour empirically; this resolves the "true lossless pre-first-byte stall" follow-up it left open.

---

## Executive Summary

**The sockops→kTLS plaintext race window cannot be closed losslessly using runtime-loadable BPF alone with the agent out of the data path. Classification: (b) — a kernel-source patch (or a transient userspace data-path hop) is required.** The Slice-00 spike already proved on a real 7.0 kernel that the `sk_msg` gate is fail-closed but lossy (a pre-arm `write()` → SK_DROP → `EACCES` → dead connection). This research resolves the spike's open "is a true lossless pre-first-byte stall reachable in BPF?" follow-up, and the answer is no — for a precise, kernel-pinned reason that holds identically on the 6.18 pin and the 7.0 dev kernel.

The mechanism walk is decisive. (SQ1) The `sk_msg` verdict set is `{PASS, DROP, REDIRECT}` with no HOLD, and `bpf_msg_cork_bytes` releases on a *byte-count threshold*, never on an external "armed" signal — flipping an armed flag does not re-invoke the corked program, so a request-first workload that corks then `read()`s deadlocks (strictly worse than the lossy drop). (SQ2) `cgroup/connect4` is binary allow/deny with no EAGAIN-stall verdict, and a connect-stall is circular anyway because the agent must handshake *over* an ESTABLISHED connection; sockops runs in atomic/softirq context that cannot sleep or wait for userspace, and has no callback that holds the first segment. (SQ4) Nothing in 6.x/7.0 changes this — current-master kernel docs still show only `__SK_PASS`/`__SK_DROP`/`__SK_REDIRECT`, sleepable BPF landed for cgroup-sockopt only (not the sk_msg verdict path), and no pause/resume-socket primitive exists in bpf-next as of June 2026.

The *only* runtime-loadable lossless path is (SQ3) `sk_msg` REDIRECT into an agent-held holding socket with re-injection after arm — but that puts the agent back in the data path (the ztunnel shape the in-band-kTLS design exists to avoid), is not honestly "BPF-only," and carries an unsolved kTLS `rec_seq` handoff plus a known kernel-side sockmap TCP-accounting gap (cilium#6431). (SQ5) No shipping system does a lossless pre-encryption hold purely in BPF: Istio ztunnel and Linkerd2 keep a permanent userspace proxy (no window by construction); Cilium encrypts the dataplane with WireGuard/IPsec at the packet layer (no per-record arm step), and its in-band-kTLS proposal (CFP #26480) is unimplemented and "entirely silent on data protection during the transition period"; Tetragon/Inspektor Gadget are out-of-band tools, not inline TLS data planes. The pre-encryption-hold problem is one the whole ecosystem designs *around*, not *through*.

**Recommendation for DESIGN Q1**: ship the spike-proven **lossy DROP-RESET gate as v1** — it is confidentiality-correct (no cleartext ever egresses) and adequate under a *named* server-speaks-first / request-first assumption — and track lossless as a follow-up, to be delivered later by *either* the transient sockmap-redirect-reinject hybrid (runtime-loadable, lossless, but agent-in-path-during-handshake; Tier-3-gated on the `rec_seq` + accounting questions) *or* a small out-of-tree "pending-kTLS write-block" kernel patch (the only path that is both lossless *and* agent-out-of-path, at the cost of carrying kernel-C across pin bumps). Do not commit to lossless now; do not take a kernel patch for v1.

---

## SQ1 — sk_msg / sockmap cork semantics (authoritative kernel detail)

> What re-triggers a corked sk_msg program? Is there ANY mechanism for userspace to asynchronously "kick"/release corked data on an external "armed" signal? Does corking the first write then deadlock if the workload blocks for a peer reply?

**SQ1 verdict: corking is byte-count-threshold-driven, NOT signal-driven. There is NO mechanism for userspace to asynchronously release corked data on an external "armed" event. Corking the first write while the workload then blocks for a peer reply genuinely deadlocks — the held bytes are released only by more bytes arriving (threshold) or by socket close. Confidence: HIGH (4 independent sources incl. kernel commit + LWN).**

### Finding 1.1 — The sk_msg verdict set has no HOLD; re-trigger is sendmsg-driven or threshold-driven

**Evidence**: `BPF_PROG_TYPE_SK_MSG` programs are "called for every `sendmsg` or `sendfile` syscall." The documented verdict set is **`SK_PASS`** ("The message may pass to the socket or it has been redirected with a helper") and **`SK_DROP`** ("The message should be dropped") — redirection is achieved by calling `bpf_msg_redirect_map`/`bpf_msg_redirect_hash` and returning SK_PASS. The isovalent/ebpf-docs reference states verbatim: **"There is no HOLD, DEFER, QUEUE, or PAUSE verdict described."** The only additional re-invocation triggers are the byte-threshold helpers (`bpf_msg_cork_bytes`, `bpf_msg_apply_bytes`) — **"the program will only be called again once N bytes are received."**

**Source**: [BPF_PROG_TYPE_SK_MSG — eBPF Docs (isovalent mirror)](https://github.com/isovalent/ebpf-docs/blob/master/docs/linux/program-type/BPF_PROG_TYPE_SK_MSG.md) — github.com/isovalent, accessed 2026-06-11. **Reputation**: High (1.0). **Verification**: [BPF_PROG_TYPE_SK_MSG — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SK_MSG/) — docs.ebpf.io, High (1.0); [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0).

**Confidence**: High — three independent authoritative sources agree the verdict set is exhaustive and has no hold state. (Re-confirms Finding 1 of the prior race-window research doc with fresh 2026-06-11 access.)

### Finding 1.2 — `bpf_msg_cork_bytes` holds data, but release is byte-threshold-only — no external "kick"

**Evidence**: The cork helper "prevents the execution of the verdict eBPF program for message msg until *bytes* have been accumulated." The kernel commit that introduced it (`91843d54`) describes the mechanism: the BPF program "will not be called again until N bytes have accumulated" — the implementation "stores the byte count in `msg->cork_bytes` and returns 0." The LWN announcement frames the use case precisely: "a BPF program can not reach a verdict on a msg until it receives more bytes AND the program doesn't want to forward the packet until it is known to be 'good'." **The re-invocation trigger is purely the byte count** — every source confirms "no external signal mechanism is described" and "no asynchronous mechanism is documented... the framework appears purely kernel-driven."

**Source**: [bpf: sockmap, add msg_cork_bytes() helper — kernel commit 91843d54](https://github.com/torvalds/linux/commit/91843d540a139eb8070bcff8aa10089164436deb) — github.com/torvalds, accessed 2026-06-11. **Reputation**: High (1.0, primary kernel source). **Verification**: [bpf,sockmap: sendmsg/sendfile ULP — LWN.net](https://lwn.net/Articles/748628/) — lwn.net, Medium-High (0.8); [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0).

**Confidence**: High — primary kernel commit + LWN + man page agree the cork release is threshold-only.

### Finding 1.3 — Corking the first write then blocking for a peer reply DEADLOCKS

**Evidence**: This is the decisive point for the "lossless hold" hypothesis. Cork holds the in-flight `sendmsg` bytes until N total bytes accumulate. If the agent sets `cork_bytes = N` to stall the first write until kTLS is armed, the held bytes are released **only** when (a) the workload issues further `sendmsg` calls that push the accumulated total to ≥ N, or (b) the socket closes (at which point, per the man page note, "data is not being buffered for *bytes* and is sent as it is received"). There is **no path** where an external "armed" event flushes the corked data. A request/response workload that writes its request once and then `read()`s — blocking on the peer's reply — never issues the additional `sendmsg` calls that cork needs to cross the threshold. The held request is stuck; the peer never receives it; the workload blocks forever on `read()`. **Cork is therefore not a hold-until-armed primitive: it is a header-accumulation batching hint that converts the lossy-DROP failure into a deadlock failure — strictly worse for a request-first protocol.**

**Source**: [bpf-helpers(7) man page — bpf_msg_cork_bytes / bpf_msg_apply_bytes notes](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0), accessed 2026-06-11. **Verification**: [bpf: sockmap, add msg_cork_bytes() helper — kernel commit 91843d54](https://github.com/torvalds/linux/commit/91843d540a139eb8070bcff8aa10089164436deb) — github.com/torvalds, High (1.0); [bpf,sockmap: sendmsg/sendfile ULP — LWN.net](https://lwn.net/Articles/748628/) — lwn.net, Medium-High (0.8).

**Confidence**: High for the mechanism (threshold-only release, no external kick); High (interpretation, labelled analysis) for the deadlock consequence on a request-first workload — it follows directly from the documented release conditions.

**Analysis (interpretation, not source claim)**: The agent CAN re-run the verdict program by writing to the per-socket `SK_STORAGE`/`sockhash` "armed" flag — but flipping that flag does **not** re-invoke the corked program; only the *next* `sendmsg` or the byte threshold does. So even with a perfect "armed" signal, the kernel never re-enters the verdict path to act on it for the already-corked bytes. This is the precise mechanical reason a lossless hold cannot be built on cork: the release trigger and the arm signal are on different clocks, and BPF exposes no helper to bridge them.

---

## SQ2 — cgroup/connect4 + sockops state-change callbacks (lossless connect-stall?)

> Can a BPF connect-time hook stall the connection losslessly until armed (e.g. -EAGAIN loop)? Does the agent's need for an ESTABLISHED connection make a connect-stall circular? What can/can't sockops callbacks do (softirq, can't sleep)?

**SQ2 verdict: NO. `cgroup/connect4` has a binary allow(1)/deny(0→EPERM) verdict — no -EAGAIN-retry, no stall-without-deny. sockops callbacks run in atomic/softirq-adjacent kernel context, cannot sleep, cannot wait for userspace, and have no callback that holds the first segment or defers writability pending an external event. A connect-stall is ALSO circular: the agent handshakes OVER an ESTABLISHED TCP connection, so stalling at connect (pre-ESTABLISHED) starves the very thing the agent needs. Confidence: HIGH.**

### Finding 2.1 — `cgroup/connect4` verdict is binary; no lossless stall / EAGAIN-retry exists

**Evidence**: The `cgroup/connect4` (`BPF_CGROUP_INET4_CONNECT`) return contract is binary: **"return 1 → Allow connection; return 0 → Block connection"** (yielding `EPERM` to the `connect()` syscall). The program "can overwrite arguments to socket related syscalls or block the call to the syscall entirely" — it rewrites the destination address or rejects, nothing in between. No source documents a return code that *stalls, retries, or loops* `connect()` pending an external event; the verdict space is allow-or-deny.

**Source**: [CGroup Socket Address — Engineering Everything with eBPF](https://ebpf.hamza-megahed.com/docs/chapter4/5-cgroup_sock_addr/) — ebpf.hamza-megahed.com, Medium (0.6, community; cross-referenced), accessed 2026-06-11. **Verification**: [eBPF Tutorial: cgroup-based Policy Control — eunomia](https://eunomia.dev/tutorials/cgroup/) — eunomia.dev, Medium-High (0.8), accessed 2026-06-11 ("return 0 = reject (EPERM), return 1 = proceed"); [BPF_PROG_TYPE_CGROUP_SOCK_ADDR — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/) — docs.ebpf.io, High (1.0) ("block the call to the syscall entirely").

**Confidence**: Medium-High — three sources agree on binary allow/deny; the *absence* of an EAGAIN-stall verdict is an argument-from-silence across all three plus the kernel UAPI, which is strong but not a single positive "no such verdict exists" citation.

### Finding 2.2 — connect4 fires BEFORE establishment → a connect-stall is circular with the agent's need for ESTABLISHED

**Evidence**: `cgroup/connect4` "fires before the connection is established... allowing for address modifications before the syscall completes"; a *separate* sockops program fires "once the connection is established using the `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB` operation." So connect4 runs at the moment the syscall begins, pre-SYN — well before any TCP connection exists.

**Source**: [cgroup/connect4 timing — WebSearch synthesis of eunomia + cloudflare-blog ebpf_connect4 example](https://github.com/cloudflare/cloudflare-blog/tree/master/2022-02-connectx/ebpf_connect4) — github.com/cloudflare, Medium-High (0.8), accessed 2026-06-11. **Verification**: prior research doc Finding 2b (`docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md`) — internal, established that `TCP_CONNECT_CB` fires when SYN is sent and `ACTIVE_ESTABLISHED_CB` fires synchronously before `connect()` returns.

**Analysis (interpretation, labelled)**: The agent's rustls handshake runs **over the already-established TCP connection** (per the spike findings: detect ESTABLISHED → `pidfd_getfd` the workload socket → handshake on the dup → install kTLS). A connect-time stall would therefore be circular: to arm kTLS the agent needs an ESTABLISHED connection to handshake across; but stalling `connect()` prevents establishment, so the agent has nothing to handshake over. Even if a stall verdict existed, it would deadlock the very mechanism it was meant to protect. The only non-circular place to gate is *after* ESTABLISHED (the `sk_msg`/sockmap egress gate), which is exactly the lossy DROP path SQ1 showed cannot be made lossless.

**Confidence**: High for the circularity (it follows directly from the spike's confirmed handshake-over-ESTABLISHED design + the connect4-fires-pre-establishment timing).

### Finding 2.3 — sockops/atomic BPF context cannot sleep or wait for userspace; no "hold the first segment" callback

**Evidence**: BPF programs running in softirq/atomic context "cannot sleep or wait for I/O — any attempt to do so will cause kernel panics or deadlocks." The kernel itself enforces this: a 2023 fix removed `bpf_setsockopt()` from the `lsm_cgroup/socket_sock_rcv_skb` hook precisely because softirq-context execution "may not own the socket lock and breaks the `bpf_setsockopt()` assumption," causing recursive-flush crashes. sockops programs set flags and options (`bpf_setsockopt`, `bpf_sock_map_update`) and observe state transitions; they have **no callback that holds the first TCP segment or defers writability** pending an external userspace event. To do real *sleepable* asynchronous work BPF must hand off to a `bpf_wq` workqueue running in process context — but a workqueue cannot retroactively hold an already-queued `sendmsg`, and the verdict path is not sleepable.

**Source**: [bpf: Fix the kernel crash caused by bpf_setsockopt() — lore.kernel.org](https://lore.kernel.org/bpf/20230125000244.1109228-1-kuifeng@meta.com/T/) — lore.kernel.org, High (1.0, primary kernel mailing list), accessed 2026-06-11. **Verification**: [BPF Workqueues for Asynchronous Sleepable Tasks — eunomia](https://eunomia.dev/tutorials/features/bpf_wq/) — eunomia.dev, Medium-High (0.8), accessed 2026-06-11 ("you can't wait for hardware responses in softirq context without special mechanisms like workqueues"); [BPF_PROG_TYPE_SOCK_OPS — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/) — docs.ebpf.io, High (1.0).

**Confidence**: High — the kernel mailing-list fix is a primary-source demonstration that sockops-context code cannot safely block, and the bpf_wq tutorial confirms sleepable work requires a process-context handoff that cannot gate the in-flight write.

---

## SQ3 — sk_msg REDIRECT into an agent-held holding sockmap, re-inject after arm

> Is this lossless? Is it "BPF-only"? Does it put the agent back in the data path (the ztunnel shape we avoid)? Does re-injection preserve ordering / no-loss?

**SQ3 verdict: This is the ONLY runtime-loadable path that can be lossless — but it is NOT honestly "BPF-only." Redirecting to an agent-held socket means the agent reads the plaintext in userspace: it is back in the data path (exactly the ztunnel shape we set out to avoid). Re-injection after arm is feasible but carries an unsolved kTLS record-sequence-counter handoff and a known kernel-side TCP-memory-accounting gap. It is a "transient userspace proxy during the handshake window," not a BPF-only hold. Confidence: HIGH (mechanism); MEDIUM (re-inject correctness — unsolved/untested).**

### Finding 3.1 — Redirect is lossless and ordered between sockmap sockets, kernel-to-kernel

**Evidence**: `bpf_msg_redirect_hash`/`bpf_msg_redirect_map` "redirect the message to the socket referenced by a `BPF_MAP_TYPE_SOCKHASH`/`SOCKMAP`... directly copies data to the target socket receive buffer." When both endpoints are sockmap-resident, "the data is never copied to userspace" — it moves from one socket's receive queue to another socket's transmit queue entirely in kernel context. The `BPF_F_INGRESS` flag selects whether the data lands on the target's ingress (receive) or egress (transmit) path. For sockmap-to-sockmap splicing this is lossless and order-preserving by construction (it is a kernel-side queue move, not a lossy verdict).

**Source**: [SOCKMAP — TCP splicing of the future (Cloudflare)](https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/) — blog.cloudflare.com, High (1.0), accessed 2026-06-11 ("the data is never copied to userspace"; "redirect it from a receive queue of some socket, to a transmit queue of the socket living in sock_map"). **Verification**: [bpf_msg_redirect_hash — eBPF Docs](https://docs.ebpf.io/linux/helper-function/bpf_msg_redirect_hash/) — docs.ebpf.io, High (1.0); [BPF sockmap documentation](https://docs.kernel.org/bpf/map_sockmap.html) — kernel.org, High (1.0).

**Confidence**: High for the kernel-to-kernel lossless splice between two sockmap sockets.

### Finding 3.2 — Redirecting to an AGENT-HELD socket puts the agent in the data path (the ztunnel shape)

**Evidence + analysis (interpretation labelled)**: The "holding sockmap" in this proposal is not a passive buffer — it is *a socket the agent owns*. For the agent to later re-inject the held bytes onto the original connection after kTLS is armed, the agent must **read those bytes in userspace** (or at minimum hold a userspace-visible socket whose data it forwards). That is definitionally the proxy data-path shape: the application's plaintext transits an agent-owned socket before reaching the wire. This is precisely the model Istio Ambient ztunnel, Cilium ztunnel (1.19+), and Linkerd2 ship — and precisely the shape the Overdrive in-band-kTLS design was chosen to **avoid** (the spike's whole point: "agent EXITS the data path, kernel does crypto"). A transient redirect-hold re-introduces the agent into the data path *for the duration of the handshake window only* — better than a permanent proxy, but not the "agent steps out" property, and not "BPF-only."

**Source**: [Istio / Ztunnel traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/) — istio.io, High (1.0), accessed 2026-06-11 (the canonical agent-in-data-path redirect model). **Verification**: spike findings `docs/feature/transparent-mtls-host-socket/spike/findings.md` (Increment A/B: agent exits the data path is the selected design); prior research doc Finding 3 (`sockops-ktls-plaintext-race-window-research.md`) — the proxy-redirect model "puts the agent back in the data path."

**Confidence**: High — this is a structural property of redirecting to an agent-owned socket, cross-confirmed against the shipping mesh prior art and Overdrive's own design intent.

### Finding 3.3 — Re-injection after arm: feasible but unsolved (rec_seq handoff) + a kernel-side accounting gap

**Evidence**: Two concrete obstacles make transient-redirect-then-reinject *plausible-but-unvalidated* rather than proven:

1. **kTLS record sequence-counter handoff.** Any bytes the agent already forwarded as TLS records advanced the peer's record sequence counter. When kTLS takes over, `setsockopt(TLS_TX)` must be given the *current* `rec_seq`, not 0 — the agent must track exactly how many records it sent during the hold and hand the correct counter to the kernel. The prior research doc flags this as having **no known production implementation**.
2. **sockmap redirect source-TX-bypass (NOT cilium#6431 — corrected 2026-06-12).** Cilium issue #6431 documents that sockmap redirect "performs socket-level accounting but omits `tcp_wmem`/`tcp_rmem` accounting" — flagged here originally as the redirect's accounting obstacle. **That attribution was wrong, and #6431 is a SEPARATE, CLOSED-completed (2022) issue about memory-budget (`wmem`/`rmem`) accounting.** The follow-up spike + research (`findings-lossless-hybrid.md`; `sockmap-redirect-live-socket-liveness-research.md` SQ2, kernel-source-pinned to `net/ipv4/tcp_bpf.c`) established the *actual* blocker: **source-TX-bypass** — `__SK_REDIRECT` never calls `tcp_sendmsg_locked()` on the source socket, so its `write_seq`/`snd_nxt` never advance for the redirected bytes (`bytes_acked:1 segs_out:2` for a 68-byte write) → deterministic RST (3/3 on 7.0). This is the *by-design semantics* of redirect (the bytes leave on a different socket), invariant across the ingress/egress flag and every kernel through 7.0 — not a memory-accounting gap. #6431 remains adjacent prior art that sockmap redirect has TCP-layer accounting gaps; it is not the mechanism that forecloses our transient-redirect hybrid.

**Source**: [kernel: sockmap redirect needs additional TCP layer accounting — cilium/cilium#6431](https://github.com/cilium/cilium/issues/6431) — github.com/cilium, High (1.0), accessed 2026-06-11. **Verification**: prior research doc Finding 3 "The sequence-counter problem" + Gap 2 (`sockops-ktls-plaintext-race-window-research.md`) — internal; [sockmap integration for ktls — LWN.net](https://lwn.net/Articles/768371/) — lwn.net, Medium-High (0.8).

**Confidence**: Medium — the obstacles are well-attested, but no source demonstrates a *working* transient-redirect-then-kTLS-reinject with correct `rec_seq`. This is the precise thing a Tier-3 spike would have to validate (see Verdict).

**Net for SQ3**: lossless YES, ordered YES (between sockmap sockets), but "BPF-only" NO (the agent reads plaintext in userspace = data path), and re-inject correctness is unproven. It belongs in option (iii)/the follow-up, not in the "pure BPF closes it" column.

---

## SQ4 — Newer kernel BPF features (≤7.0)

> Sleepable sk_msg/sockops? BPF arena? New helpers? Any "pause/resume socket" primitive in 6.x/7.0?

**SQ4 verdict: NOTHING in 6.x/7.0 changes the answer. As of June 2026 the current master kernel sk_msg verdict set is still `{__SK_PASS, __SK_DROP, __SK_REDIRECT}` — no HOLD/DEFER/PAUSE/backpressure verdict has landed. Sleepable BPF exists but only for `cgroup/{get,set}sockopt` (and tracing/LSM), NOT for the sk_msg verdict path, and sleepability gives `copy_from/to_user`, not the ability to block an in-flight write on an external arm. No "pause/resume socket" primitive exists. The 6.18 pin and the 7.0 dev kernel are identical on this axis. Confidence: HIGH.**

### Finding 4.1 — Current master sk_msg verdict set is unchanged; no HOLD/backpressure verdict

**Evidence**: The current master (`torvalds/linux`) sockmap documentation states verbatim: **"The verdict program is essentially the redirect program and can return a verdict of `__SK_DROP`, `__SK_PASS`, or `__SK_REDIRECT`."** There is no HOLD, DEFER, PAUSE, or QUEUE verdict; the doc has no mention of any backpressure mechanism that would make the application's `write()` block rather than drop. This is the *same* verdict set documented for the original 2018 sk_msg introduction — the surface has not grown a hold state in eight years.

**Source**: [linux/Documentation/bpf/map_sockmap.rst (master) — torvalds/linux](https://github.com/torvalds/linux/blob/master/Documentation/bpf/map_sockmap.rst) — github.com/torvalds, High (1.0, primary kernel source, current master), accessed 2026-06-11. **Verification**: [BPF_PROG_TYPE_SK_MSG — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SK_MSG/) — docs.ebpf.io, High (1.0); [sockmap: introduce BPF_SK_SKB_VERDICT and support UDP — LWN.net](https://lwn.net/Articles/851064/) — lwn.net, Medium-High (0.8) (the most recent sockmap *verdict* extension — adds UDP support and a new attach point, NOT a hold verdict).

**Confidence**: High — primary current-master kernel doc, cross-referenced.

### Finding 4.2 — Sleepable BPF landed for cgroup sockopt only — not the sk_msg verdict path, and it does not enable a lossless hold

**Evidence**: Sleepable BPF program support exists, but the sk_msg/sockops *verdict* path is not in it. The cgroup-sockopt sleepability work (LWN 942810) made "BPF programs attached on `cgroup/{get,set}sockopt` hooks sleepable" — and its purpose is strictly to let "BPF programs call `copy_from_user()` and `copy_to_user()`" to read full option buffers. The article does **not** address blocking on external userspace events: "There is no discussion of waiting mechanisms or external event signaling," and it provides no "data retention or pausing mechanisms." Sleepability ≠ the ability to park an in-flight `sendmsg` until an agent arms.

**Source**: [Sleepable BPF programs on cgroup {get,set}sockopt — LWN.net](https://lwn.net/Articles/942810/) — lwn.net, Medium-High (0.8), accessed 2026-06-11. **Verification**: [Sleepable BPF programs — LWN.net](https://lwn.net/Articles/825415/) — lwn.net, Medium-High (0.8) (the foundational sleepable-BPF design — sleepable progs run in process context for `copy_from_user`-style work, not as a socket-data hold); current-master sockmap doc (above) lists no sleepable sk_msg.

**Confidence**: High — the cgroup-sockopt scope is explicit; the absence of sleepable sk_msg is confirmed by the current verdict-set doc.

### Finding 4.3 — No "pause/resume socket" primitive in bpf-next; BPF arena/workqueues do not help

**Evidence**: A targeted scan of bpf-next / netdev patch traffic surfaced no 2025–2026 patch introducing a "pause socket," "hold data," or "lossless" sk_msg/sockops primitive; the matching results are all the historical 2017–2022 sk_msg/sockmap introduction and performance patches. BPF arena (a sparse shared memory region) and `bpf_wq` workqueues add *sleepable asynchronous compute*, but neither can retroactively hold an already-issued `sendmsg` on the verdict path — the verdict path is non-sleepable and the workqueue runs in a separate process context that cannot gate the original write. (One *negative* recent signal: CVE-2025-39913, a UAF in `tcp_bpf_send_verdict()`'s cork-allocation error path, confirms the cork/redirect machinery is still being bug-fixed rather than gaining new hold semantics.)

**Source**: [bpf-next/netdev patch scan — WebSearch, no 2025–2026 hold/pause primitive](https://lore.kernel.org/netdev/) — lore.kernel.org, High (1.0, primary mailing-list archive; null result), accessed 2026-06-11. **Verification**: [CVE-2025-39913 — SOCKMAP tcp_bpf_send_verdict cork UAF](https://bytrep.com/Article2.html) — bytrep.com, Medium (0.6, community security writeup; cross-referenced against the CVE record), accessed 2026-06-11; [BPF Workqueues for Asynchronous Sleepable Tasks — eunomia](https://eunomia.dev/tutorials/features/bpf_wq/) — eunomia.dev, Medium-High (0.8).

**Confidence**: Medium-High — a null result from a primary archive plus corroborating signals; an argument from absence, but a well-searched one. See Knowledge Gaps for the residual.

---

## SQ5 — Prior art (lossless pre-encryption hold purely in BPF?)

> Cilium (mTLS + WireGuard/IPsec; kTLS dataplane CFP #26480), Istio ambient/ztunnel, Tetragon, Inspektor Gadget, upstream kTLS discussions. What did each use / conclude about holding plaintext before encryption?

**SQ5 verdict: NO shipping system achieves a lossless pre-encryption hold purely in BPF. Every transparent-mTLS data plane either (a) keeps a userspace proxy permanently in the data path (Istio ztunnel, Linkerd2) so there is no pre-encryption window by construction, or (b) encrypts at the packet layer with WireGuard/IPsec (Cilium) so there is no per-record arm window at all. The one in-band-kTLS data-plane proposal (Cilium CFP #26480) is unimplemented and "entirely silent on data protection during the transition period before encryption becomes active." Tetragon/Inspektor Gadget are observability/out-of-band-enforcement tools, not inline TLS data planes. The pre-encryption-hold problem is one the entire ecosystem designs AROUND, not THROUGH. Confidence: HIGH.**

### Finding 5.1 — Cilium: shipping dataplane is WireGuard/IPsec (packet-layer), not in-band kTLS; the kTLS proposal is unimplemented and silent on the hold

**Evidence**: Cilium's mutual authentication brings the mTLS handshake **out-of-band** and the *encryption* is done by "Cilium's regular encryption mechanisms (IPSec and Wireguard)" — packet/tunnel-layer encryption, where there is no per-socket "install keys mid-connection" arm step and therefore no plaintext-record race to hold. CFP #26480 ("use mutual auth negotiated session key for pod-to-pod encryption" — the closest thing to an in-band-kTLS data plane) is **"a proposal (CFP), not yet implemented,"** and crucially it "makes no mention of plaintext race conditions, key installation timing, or data held before encryption is armed... The CFP is entirely silent on data protection during the transition period." So even the one ecosystem actor reaching toward in-band kTLS has not solved (or even addressed) the lossless-hold question.

**Source**: [CFP: Use mutual auth negotiated session key for pod-to-pod encryption — cilium/cilium#26480](https://github.com/cilium/cilium/issues/26480) — github.com/cilium, High (1.0), accessed 2026-06-11. **Verification**: [WireGuard Transparent Encryption — Cilium docs](https://docs.cilium.io/en/latest/security/network/encryption-wireguard/) — docs.cilium.io, High (1.0); [Mutual Authentication (Beta) — Cilium docs](https://docs.cilium.io/en/latest/network/servicemesh/mutual-authentication/mutual-authentication/) — docs.cilium.io, High (1.0).

**Confidence**: High — the CFP's unimplemented status and silence on the hold are direct from the issue; the WireGuard/IPsec dataplane is canonical Cilium docs.

### Finding 5.2 — Istio ambient/ztunnel + Linkerd2: permanent userspace proxy → no pre-encryption window by construction

**Evidence**: Istio ambient redirects pod traffic to a node-local ztunnel proxy ("packets entering and leaving the pod are intercepted and transparently redirected to the node-local ztunnel proxy instance"); ztunnel performs the mTLS (HBONE) in userspace. Linkerd2 uses the same iptables-redirect-to-microproxy model. Because the application socket is *never* the TLS socket and the proxy buffers before its own handshake completes, there is no window in which application plaintext could escape un-encrypted — the race is eliminated by keeping the agent permanently in the data path. **None of them use in-band kTLS on the application socket; none of them needed to solve the lossless-hold problem because they never let the app's socket be the wire socket.** (This is the very property Overdrive's design trades away to get the agent out of the data path — and the reason the hold problem is Overdrive's to solve where it was not theirs.)

**Source**: [Istio / Ztunnel traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/) — istio.io, High (1.0), accessed 2026-06-11. **Verification**: prior research doc Finding 6 (`sockops-ktls-plaintext-race-window-research.md`) — internal, established Istio/Cilium-ztunnel/Linkerd2 all use the proxy model and "None use in-place kTLS on the application socket"; [SOCKMAP — TCP splicing (Cloudflare)](https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/) — blog.cloudflare.com, High (1.0).

**Confidence**: High — multiple shipping systems, cross-confirmed with the prior research doc's comprehensive prior-art survey.

### Finding 5.3 — Tetragon / Inspektor Gadget are out-of-band; tlshd is consumer-initiated — neither is a lossless-BPF-hold precedent

**Evidence**: Tetragon enforces "through syscall return value manipulation and process termination — not through inline data interception or buffering" (return-value override + `SIGKILL`); it is explicitly "not an inline data-path component that buffers or holds socket data." Inspektor Gadget is "an observability framework... for data collection and system inspection," not an inline TLS data plane. The in-kernel TLS handshake path (`CONFIG_NET_HANDSHAKE` / tlshd), the only Architecture-A-shaped precedent, holds the socket via an *explicit kernel-consumer contract* (the consumer simply does not call `send()` until `handshake_done`) and requires a kernel consumer to initiate — it cannot be triggered for an arbitrary transparently-intercepted workload socket. **No system in the survey holds workload plaintext losslessly before encryption using BPF alone.**

**Source**: [Tetragon Enforcement — tetragon.io](https://tetragon.io/docs/concepts/enforcement/) — tetragon.io, High (1.0), accessed 2026-06-11. **Verification**: [Inspektor Gadget](https://inspektor-gadget.io/) — inspektor-gadget.io, Medium-High (0.8) (observability framing); prior research doc Finding 4 + Finding 6 (`sockops-ktls-plaintext-race-window-research.md`) — internal (tlshd consumer-initiated, not applicable to arbitrary sockets); [In-Kernel TLS Handshake — kernel.org](https://docs.kernel.org/networking/tls-handshake.html) — kernel.org, High (1.0).

**Confidence**: High — Tetragon's out-of-band model is direct from its docs; the tlshd consumer-initiation gap is established kernel doc + prior research.

---

## Verdict

### Classification: **(b) NOT achievable in BPF-only — a kernel-source patch is required for a true zero-hop lossless hold.**

A lossless "hold the workload's pre-arm `write()` until kTLS is armed, then release it, with the agent OUT of the data path" **cannot be built from runtime-loadable BPF on 6.18 or 7.0.** Each candidate BPF mechanism fails for a distinct, kernel-pinned reason:

| Candidate (runtime-loadable BPF) | Why it cannot do a lossless zero-hop hold |
|---|---|
| `sk_msg` SK_DROP gate (the spike's path) | Fail-closed but **lossy** — drops the byte, `EACCES`, dead connection. No HOLD verdict (SQ1). |
| `bpf_msg_cork_bytes` | Threshold-driven, **not signal-driven**; flipping an "armed" flag does not re-invoke the corked program; a request-first workload that corks then `read()`s **deadlocks** (SQ1). |
| `cgroup/connect4` stall | Binary allow/deny only — no EAGAIN-retry/stall verdict; and stalling pre-ESTABLISHED is **circular** with the agent's need to handshake over an ESTABLISHED connection (SQ2). |
| `sockops` defer-writability | Atomic/softirq context — **cannot sleep, cannot wait for userspace**; no callback holds the first segment (SQ2). |
| Newer 6.x/7.0 features | Verdict set still `{PASS,DROP,REDIRECT}`; sleepable BPF is cgroup-sockopt-only; **no pause/resume primitive exists** (SQ4). |
| `sk_msg` REDIRECT-to-holding-socket | Lossless and ordered, BUT the holding socket is **agent-owned → agent is back in the data path** (the ztunnel shape we avoid). Not "BPF-only." Re-inject `rec_seq` handoff unsolved (SQ3). |

### The PRECISE missing primitive (what a kernel patch would add)

The BPF socket-data verdict path has no **signal-driven, lossless HOLD** — no way to *park an in-flight `sendmsg()` and release it (in order, no loss) when an out-of-band "armed" event fires*, while keeping the data on its own socket. Concretely the kernel lacks any of:

1. A fourth sk_msg verdict (`SK_HOLD`/`SK_PARK`) that queues the message on the socket's own send path and is re-evaluated when userspace flips a per-socket flag — i.e. **the cork-release trigger and the arm signal are on different clocks, with no helper to bridge them** (SQ1).
2. A "pending-handshake" socket state that makes `write()`/`sendmsg()` **block on standard buffer-full backpressure semantics** until the agent arms kTLS (the app blocks exactly as on a full `SO_SNDBUF`, then proceeds — no loss, no reset). This is the small out-of-tree patch the prior research doc's "controlled-kernel implication" named.
3. An extension of `CONFIG_NET_HANDSHAKE` letting an agent register a handshake-pending hold on a transparently-intercepted (non-kernel-consumer) socket — converting tlshd's consumer-initiated hold into an agent-initiated one (SQ5).

Any one of these is a **kernel-source change** (a new UAPI verdict, a new socket state, or a new netlink/handshake entry point); none is reachable by loading a BPF program. Overdrive ships a Yocto appliance kernel, so option 2 (a "pending-kTLS write-block" socket state) is *available* — at the cost of carrying an out-of-tree patch across pin bumps (a verifier/security-review surface and a rebase burden).

### Residual (c) caveat — the one thing a Tier-3 spike could still upgrade

The classification is **(b)** for a *true BPF-only zero-hop* hold. The **transient sockmap-redirect-then-reinject** hybrid (SQ3) is **(c) PLAUSIBLE-BUT-UNVALIDATED** as a *lossless-with-transient-agent-hop* path — it is runtime-loadable (no kernel patch) and lossless, but is honestly a brief userspace proxy, not "agent out of the data path." A Tier-3 spike must settle exactly two things before it could be shipped: (i) that the agent can re-inject the held bytes onto the original connection after `setsockopt(TLS_TX)` with the **correct `rec_seq`** so the peer's TLS record stream is continuous (wire capture: unbroken incrementing record sequence across the proxy→kTLS handoff); and (ii) that the sockmap-redirect **TCP-memory-accounting gap** (cilium#6431) does not corrupt or stall under a fast-client/slow-server skew. Until both pass on a real 6.18/7.0 kernel, the hybrid is not decision-grade.

### Q1 recommendation (for the DESIGN decision)

**Ship lossy DROP-RESET as v1; track lossless as a follow-up. Do NOT commit to lossless now, and do NOT take a kernel patch for v1.**

- **v1 (now)**: the spike-proven `sk_msg` SK_DROP gate. It is **confidentiality-correct** (the security invariant — no cleartext ever egresses — holds unconditionally, proven on 7.0). On a pre-arm write it drops + resets the connection; close the connection deliberately when the drop counter is `>0` so the workload sees a clean reset and reconnects, rather than silent loss. **Name the assumption explicitly**: this is correct for **server-speaks-first / request-first protocols where the agent arms before the workload's first write lands**, and it eats one reset on a client-speaks-first protocol's first connection. (Per the spike, in-band kTLS is alive on 7.0 — do not regress to a permanent Cilium-style proxy to dodge this.)
- **Follow-up (tracked, not v1)**: if a workload class needs true lossless first-byte delivery, pursue **either** (iii-a) the transient sockmap-redirect-reinject hybrid — runtime-loadable, lossless, but agent-in-path-during-handshake and gated on the Tier-3 spike above; **or** (iii-b) the out-of-tree "pending-kTLS write-block" kernel patch — the only path that is *both* lossless *and* agent-out-of-path, at the cost of kernel-C ownership. The choice between (iii-a) and (iii-b) is a build-vs-maintain tradeoff (transient data-path hop vs. carried kernel patch), legitimately deferrable past v1.

**One-line**: *No runtime-loadable BPF closes the window losslessly with the agent out of the data path; v1 ships the confidentiality-correct lossy-DROP-RESET gate (server-speaks-first assumption named), and lossless is a tracked follow-up — via the transient-redirect hybrid (Tier-3-gated) or a small out-of-tree kernel write-block patch, decided later.*

### Confidence

**High** on the (b) classification and the missing-primitive statement — every blocking mechanism is sourced from primary kernel material (commits, current-master docs, the mailing-list softirq fix) and cross-referenced. **Medium** on the (c) hybrid's *feasibility* (the `rec_seq` handoff and accounting gap are real, well-attested obstacles with no demonstrated working implementation). **High** on the Q1 recommendation — it follows directly from the spike's empirical 7.0 result plus these mechanism findings.

---

## Source Analysis

| # | Source | Domain | Reputation | Type | Access Date | Cross-verified | Used in |
|---|--------|--------|------------|------|-------------|----------------|---------|
| 1 | [BPF_PROG_TYPE_SK_MSG — isovalent/ebpf-docs](https://github.com/isovalent/ebpf-docs/blob/master/docs/linux/program-type/BPF_PROG_TYPE_SK_MSG.md) | github.com | High (1.0) | Open source | 2026-06-11 | Y | SQ1 |
| 2 | [BPF_PROG_TYPE_SK_MSG — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SK_MSG/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-11 | Y | SQ1, SQ4 |
| 3 | [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) | man7.org | High (1.0) | Official | 2026-06-11 | Y | SQ1 |
| 4 | [kernel commit 91843d54 — msg_cork_bytes()](https://github.com/torvalds/linux/commit/91843d540a139eb8070bcff8aa10089164436deb) | github.com/torvalds | High (1.0) | Primary kernel source | 2026-06-11 | Y | SQ1 |
| 5 | [bpf,sockmap: sendmsg/sendfile ULP — LWN](https://lwn.net/Articles/748628/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-11 | Y | SQ1 |
| 6 | [CGroup Socket Address — Engineering Everything with eBPF](https://ebpf.hamza-megahed.com/docs/chapter4/5-cgroup_sock_addr/) | ebpf.hamza-megahed.com | Medium (0.6) | Community | 2026-06-11 | Y | SQ2 |
| 7 | [cgroup-based Policy Control — eunomia](https://eunomia.dev/tutorials/cgroup/) | eunomia.dev | Medium-High (0.8) | Technical | 2026-06-11 | Y | SQ2 |
| 8 | [BPF_PROG_TYPE_CGROUP_SOCK_ADDR — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-11 | Y | SQ2 |
| 9 | [cloudflare-blog ebpf_connect4 example](https://github.com/cloudflare/cloudflare-blog/tree/master/2022-02-connectx/ebpf_connect4) | github.com/cloudflare | Medium-High (0.8) | Open source | 2026-06-11 | Y | SQ2 |
| 10 | [bpf: Fix crash caused by bpf_setsockopt() — lore.kernel.org](https://lore.kernel.org/bpf/20230125000244.1109228-1-kuifeng@meta.com/T/) | lore.kernel.org | High (1.0) | Primary kernel ML | 2026-06-11 | Y | SQ2 |
| 11 | [BPF Workqueues for Sleepable Tasks — eunomia](https://eunomia.dev/tutorials/features/bpf_wq/) | eunomia.dev | Medium-High (0.8) | Technical | 2026-06-11 | Y | SQ2, SQ4 |
| 12 | [BPF_PROG_TYPE_SOCK_OPS — eBPF Docs](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-11 | Y | SQ2, SQ4 |
| 13 | [SOCKMAP — TCP splicing (Cloudflare)](https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/) | blog.cloudflare.com | High (1.0) | Open source | 2026-06-11 | Y | SQ3, SQ5 |
| 14 | [bpf_msg_redirect_hash — eBPF Docs](https://docs.ebpf.io/linux/helper-function/bpf_msg_redirect_hash/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-11 | Y | SQ3 |
| 15 | [BPF sockmap documentation — kernel.org](https://docs.kernel.org/bpf/map_sockmap.html) | kernel.org | High (1.0) | Official | 2026-06-11 | Y | SQ3 |
| 16 | [Istio / Ztunnel traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/) | istio.io | High (1.0) | Official | 2026-06-11 | Y | SQ3, SQ5 |
| 17 | [cilium#6431 — sockmap redirect TCP accounting](https://github.com/cilium/cilium/issues/6431) | github.com/cilium | High (1.0) | Open source | 2026-06-11 | Y | SQ3 |
| 18 | [sockmap integration for ktls — LWN](https://lwn.net/Articles/768371/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-11 | Y | SQ3 |
| 19 | [map_sockmap.rst (master) — torvalds/linux](https://github.com/torvalds/linux/blob/master/Documentation/bpf/map_sockmap.rst) | github.com/torvalds | High (1.0) | Primary kernel source | 2026-06-11 | Y | SQ4 |
| 20 | [sockmap: BPF_SK_SKB_VERDICT + UDP — LWN](https://lwn.net/Articles/851064/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-11 | Y | SQ4 |
| 21 | [Sleepable BPF on cgroup {get,set}sockopt — LWN](https://lwn.net/Articles/942810/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-11 | Y | SQ4 |
| 22 | [Sleepable BPF programs — LWN](https://lwn.net/Articles/825415/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-11 | Y | SQ4 |
| 23 | [lore.kernel.org netdev archive (null result for hold primitive)](https://lore.kernel.org/netdev/) | lore.kernel.org | High (1.0) | Primary kernel ML | 2026-06-11 | Partial | SQ4 |
| 24 | [CVE-2025-39913 SOCKMAP cork UAF writeup](https://bytrep.com/Article2.html) | bytrep.com | Medium (0.6) | Community security | 2026-06-11 | Y | SQ4 |
| 25 | [Cilium CFP#26480 — session key for pod encryption](https://github.com/cilium/cilium/issues/26480) | github.com/cilium | High (1.0) | Open source | 2026-06-11 | Y | SQ5 |
| 26 | [WireGuard Transparent Encryption — Cilium docs](https://docs.cilium.io/en/latest/security/network/encryption-wireguard/) | docs.cilium.io | High (1.0) | Official | 2026-06-11 | Y | SQ5 |
| 27 | [Mutual Authentication (Beta) — Cilium docs](https://docs.cilium.io/en/latest/network/servicemesh/mutual-authentication/mutual-authentication/) | docs.cilium.io | High (1.0) | Official | 2026-06-11 | Y | SQ5 |
| 28 | [Tetragon Enforcement — tetragon.io](https://tetragon.io/docs/concepts/enforcement/) | tetragon.io | High (1.0) | Official | 2026-06-11 | Y | SQ5 |
| 29 | [Inspektor Gadget](https://inspektor-gadget.io/) | inspektor-gadget.io | Medium-High (0.8) | Official | 2026-06-11 | Y | SQ5 |
| 30 | [In-Kernel TLS Handshake — kernel.org](https://docs.kernel.org/networking/tls-handshake.html) | kernel.org | High (1.0) | Official | 2026-06-11 | Y | SQ5 |

**Reputation distribution**: High (1.0): 19 sources (63%) | Medium-High (0.8): 9 sources (30%) | Medium (0.6): 2 sources (7%) | **Average: 0.92**

**Internal cross-references (not counted as external sources)**: the prior race-window research doc (`docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md`, 25 sources) and the Slice-00 spike findings (`docs/feature/transparent-mtls-host-socket/spike/findings.md`, empirical 7.0 result) — both treated as established prior work.

## Knowledge Gaps

### Gap 1 — No single positive "no SK_HOLD verdict exists" citation; argument is from exhaustive enumeration
**Issue**: The claim "there is no HOLD/DEFER/PAUSE verdict" rests on every authoritative source *enumerating* the verdict set as `{PASS, DROP, REDIRECT}` and on isovalent/ebpf-docs stating verbatim "There is no HOLD, DEFER, QUEUE, or PAUSE verdict described." This is a strong argument-from-exhaustive-enumeration across primary kernel docs, but not a single source that says "the kernel rejected a proposed HOLD verdict."
**Attempted**: Searched lore.kernel.org / netdev for sk_msg hold/pause/backpressure proposals (null result); read current-master sockmap.rst.
**Recommendation**: Not load-bearing — the verdict-set enumeration from current-master kernel source is authoritative-minimum-sufficient for an exact-semantics claim. If a reviewer wants belt-and-suspenders, grep the UAPI `enum sk_action` in `include/uapi/linux/bpf.h` directly (only `SK_DROP`/`SK_PASS` defined).

### Gap 2 — Transient-redirect-reinject `rec_seq` handoff has no demonstrated working implementation
**Issue**: The (c) hybrid's losslessness depends on re-injecting agent-held bytes after `setsockopt(TLS_TX)` with the correct TLS record sequence counter, so the peer's record stream is continuous. No source demonstrates this working; the prior research doc flags it as having "no known production implementation."
**Attempted**: Searched for ztunnel/Cilium sockmap-to-kTLS handoff with rec_seq; checked cilium#6431 (found the adjacent accounting gap, not the handoff). Could not find a worked rec_seq-continuity example.
**Recommendation**: This is exactly the Tier-3 spike named in the Verdict's (c) caveat — prototype the handoff and assert unbroken incrementing TLS record sequence across the proxy→kTLS transition via wire capture. Until then the hybrid is PLAUSIBLE-BUT-UNVALIDATED, not decision-grade.

### Gap 3 — bpf-next null result is an argument from absence
**Issue**: Finding 4.3 ("no pause/resume primitive in bpf-next") is a null result from a web-search of the netdev archive, not an exhaustive read of every 2025–2026 bpf-next series.
**Attempted**: Multiple targeted searches (sk_msg/sockops + pause/hold/lossless/backpressure, 2025–2026); all returned historical 2017–2022 introduction patches.
**Recommendation**: Low risk — a landed hold-verdict would have changed the current-master sockmap.rst (Finding 4.1), which it has not. If certainty is required before committing the v1 contract, one direct pass over `git log --since=2025 -- net/core/skmsg.c net/ipv4/tcp_bpf.c` on the 7.0 source tree settles it definitively (in-scope for the DESIGN's own kernel checkout).

## Recommendations for Further Research

1. **Tier-3 spike the (c) hybrid** before any commitment to lossless-via-redirect: validate `rec_seq` continuity across the proxy→kTLS handoff and the cilium#6431 accounting behaviour under fast-client/slow-server skew on a real 6.18/7.0 kernel (Gap 2). This is the single experiment that would move the hybrid from (c) to (a-for-the-transient-hop variant).
2. **Scope the out-of-tree "pending-kTLS write-block" patch** if a workload class genuinely needs lossless-AND-agent-out-of-path: a minimal socket state that returns standard `EAGAIN`/blocks on buffer-full backpressure until the agent arms, then proceeds. Estimate the rebase/verifier-review burden against the appliance kernel pin-bump cadence (ADR-0068).
3. **Confirm the v1 server-speaks-first assumption against the actual workload catalogue** — enumerate which first-party workloads are request-first vs server-speaks-first, to bound how often the lossy-DROP-RESET reset actually fires in practice.

## Full Citations

[1] isovalent. "BPF_PROG_TYPE_SK_MSG (ebpf-docs)". GitHub. https://github.com/isovalent/ebpf-docs/blob/master/docs/linux/program-type/BPF_PROG_TYPE_SK_MSG.md. Accessed 2026-06-11.
[2] eBPF Docs. "Program Type 'BPF_PROG_TYPE_SK_MSG'". docs.ebpf.io. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SK_MSG/. Accessed 2026-06-11.
[3] Linux man-pages project. "bpf-helpers(7)". man7.org. https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html. Accessed 2026-06-11.
[4] Fastabend, John. "bpf: sockmap, add msg_cork_bytes() helper" (commit 91843d54). torvalds/linux. 2018. https://github.com/torvalds/linux/commit/91843d540a139eb8070bcff8aa10089164436deb. Accessed 2026-06-11.
[5] Corbet, Jonathan. "bpf,sockmap: sendmsg/sendfile ULP". LWN.net. 2018. https://lwn.net/Articles/748628/. Accessed 2026-06-11.
[6] Megahed, Hamza. "CGroup Socket Address — Engineering Everything with eBPF". ebpf.hamza-megahed.com. https://ebpf.hamza-megahed.com/docs/chapter4/5-cgroup_sock_addr/. Accessed 2026-06-11.
[7] eunomia-bpf. "eBPF Tutorial: cgroup-based Policy Control". eunomia.dev. https://eunomia.dev/tutorials/cgroup/. Accessed 2026-06-11.
[8] eBPF Docs. "Program Type 'BPF_PROG_TYPE_CGROUP_SOCK_ADDR'". docs.ebpf.io. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/. Accessed 2026-06-11.
[9] Cloudflare. "ebpf_connect4 example (cloudflare-blog 2022-02-connectx)". GitHub. https://github.com/cloudflare/cloudflare-blog/tree/master/2022-02-connectx/ebpf_connect4. Accessed 2026-06-11.
[10] Feng, Kui-Feng. "bpf: Fix the kernel crash caused by bpf_setsockopt()". lore.kernel.org. 2023. https://lore.kernel.org/bpf/20230125000244.1109228-1-kuifeng@meta.com/T/. Accessed 2026-06-11.
[11] eunomia-bpf. "eBPF Tutorial: BPF Workqueues for Asynchronous Sleepable Tasks". eunomia.dev. https://eunomia.dev/tutorials/features/bpf_wq/. Accessed 2026-06-11.
[12] eBPF Docs. "Program Type 'BPF_PROG_TYPE_SOCK_OPS'". docs.ebpf.io. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/. Accessed 2026-06-11.
[13] Majkowski, Marek. "SOCKMAP - TCP splicing of the future". Cloudflare Blog. https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/. Accessed 2026-06-11.
[14] eBPF Docs. "Helper Function 'bpf_msg_redirect_hash'". docs.ebpf.io. https://docs.ebpf.io/linux/helper-function/bpf_msg_redirect_hash/. Accessed 2026-06-11.
[15] Linux kernel. "BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH". docs.kernel.org. https://docs.kernel.org/bpf/map_sockmap.html. Accessed 2026-06-11.
[16] Istio. "Ztunnel traffic redirection". istio.io. https://istio.io/latest/docs/ambient/architecture/traffic-redirection/. Accessed 2026-06-11.
[17] Cilium. "kernel: sockmap redirect sockets need additional TCP layer accounting" (issue #6431). GitHub. https://github.com/cilium/cilium/issues/6431. Accessed 2026-06-11.
[18] Corbet, Jonathan. "sockmap integration for ktls". LWN.net. 2018. https://lwn.net/Articles/768371/. Accessed 2026-06-11.
[19] Fastabend, John et al. "Documentation/bpf/map_sockmap.rst" (master). torvalds/linux. https://github.com/torvalds/linux/blob/master/Documentation/bpf/map_sockmap.rst. Accessed 2026-06-11.
[20] Corbet, Jonathan. "sockmap: introduce BPF_SK_SKB_VERDICT and support UDP". LWN.net. 2021. https://lwn.net/Articles/851064/. Accessed 2026-06-11.
[21] Corbet, Jonathan. "Sleepable BPF programs on cgroup {get,set}sockopt". LWN.net. https://lwn.net/Articles/942810/. Accessed 2026-06-11.
[22] Corbet, Jonathan. "Sleepable BPF programs". LWN.net. 2020. https://lwn.net/Articles/825415/. Accessed 2026-06-11.
[23] Linux kernel netdev mailing list archive. lore.kernel.org. https://lore.kernel.org/netdev/. Accessed 2026-06-11 (null-result scan for sk_msg hold/pause primitive).
[24] bytrep. "CVE-2025-39913 Deep Dive: Linux Kernel eBPF SOCKMAP UAF". bytrep.com. https://bytrep.com/Article2.html. Accessed 2026-06-11.
[25] Cilium. "CFP: Use mutual auth negotiated session key for pod-to-pod encryption" (issue #26480). GitHub. https://github.com/cilium/cilium/issues/26480. Accessed 2026-06-11.
[26] Cilium. "WireGuard Transparent Encryption". docs.cilium.io. https://docs.cilium.io/en/latest/security/network/encryption-wireguard/. Accessed 2026-06-11.
[27] Cilium. "Mutual Authentication (Beta)". docs.cilium.io. https://docs.cilium.io/en/latest/network/servicemesh/mutual-authentication/mutual-authentication/. Accessed 2026-06-11.
[28] Tetragon. "Enforcement". tetragon.io. https://tetragon.io/docs/concepts/enforcement/. Accessed 2026-06-11.
[29] Inspektor Gadget. inspektor-gadget.io. https://inspektor-gadget.io/. Accessed 2026-06-11.
[30] Lever, Chuck et al. "In-Kernel TLS Handshake". docs.kernel.org. https://docs.kernel.org/networking/tls-handshake.html. Accessed 2026-06-11.

## Research Metadata

**Duration**: ~40 min | **Sources examined**: 35+ | **Sources cited**: 30 external (+ 2 internal prior-work refs) | **Cross-refs**: every SQ verdict backed by ≥3 sources (≥1 primary kernel source for SQ1/SQ4 exact-semantics claims) | **Confidence distribution**: High ~80% (the (b) classification + missing-primitive + Q1 rec), Medium ~20% (the (c) hybrid feasibility + bpf-next null result) | **Output**: `docs/research/dataplane/sockops-ktls-lossless-hold-bpf-only-research.md`
