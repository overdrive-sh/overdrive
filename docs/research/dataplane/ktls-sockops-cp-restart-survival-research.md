# Research: Does kernel-side mTLS state (kTLS + eBPF sockops + BPF maps) survive an Overdrive control-plane process restart on a real Linux kernel?

**Date**: 2026-06-09 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (on kernel mechanisms) / Medium (on #26 composed behaviour — empirically open) | **Sources**: 18 distinct (15 High, 2 Medium-High; incl. 4 primary kernel sources)

> **Decision outcome (2026-06-09) — supersedes this doc's "spike required" verdict.**
> Overdrive **drops the kernel-retention optimization** and adopts **re-mint on
> restart** (the industry default the sibling research found universal). The
> Tier-3 spike recommended below is **NOT pursued**. Rationale: a control-plane
> *process* restart (kernel lives) is only one of three restart shapes, and the
> only one the optimization could ever help —
>
> | Scenario | Kernel | Workloads | Survives | Re-mint? |
> |---|---|---|---|---|
> | CP process restart (crash/upgrade) | lives | live | kTLS-on-socket + bpffs-pinned maps/links *if wired so* | maybe not |
> | **Full node reboot** | **restarts** | **restart** | **only the on-disk envelope-sealed CA root** | **mandatory** |
> | Workload restart only | lives | one restarts | CP in-memory hold (if CP up) | depends on alloc identity |
>
> On a **full node reboot** every kernel-owned layer is wiped — **bpffs pins do
> not survive a reboot** (`/sys/fs/bpf` is kernel-memory-backed; it returns
> empty), kTLS sockets are destroyed, workloads come up fresh holding nothing.
> Only the persisted CA root survives, so re-mint is **mandatory** regardless.
> The optimization therefore helps only a CP-process-bounce *without* a reboot —
> a narrow case — and buying it costs novel/unprecedented design (no mesh does
> kTLS-survives-broker-restart; see §Cilium) plus a kernel spike to validate.
> Not worth it for Phase 3.3.
>
> Net effect: #35's existing restart-recovery branch (`running ∧ ¬held ∧
> ever_issued → IssueSvid` immediately) is **already correct** — it re-mints.
> The `issued_certificates` audit row's role narrows to rotation-scheduling +
> over-issuance dedup, never re-issuance suppression. The kernel-mechanism
> findings below remain valid reference for #26.

## Executive Summary

The question — must an Overdrive control-plane restart **re-mint** every running workload's SVID to re-supply the kernel, or does the **kernel retain the operative crypto** — splits cleanly across three independently-owned kernel state layers, and the answer is different for each. **(1) kTLS** crypto state lives on the *socket* (`icsk_ulp_data` on the `inet_connection_sock`), not in the configuring process; it is freed only on **socket close** (`tls_sk_proto_close`), and there is **no kernel primitive to pin it independently of its socket**. So kTLS survives a control-plane restart *if and only if a still-living process holds the socket fd* — which makes the decisive variable **who owns the workload socket fd**, a property of Overdrive's (unbuilt) #26 wiring, not of the kernel. **(2) sockops attachment** survives the loader exiting *iff* it used legacy `BPF_PROG_ATTACH` (the cgroup owns it) or a `bpf_link` **pinned to bpffs** — an unpinned `bpf_link` auto-detaches the instant the loader exits. **(3) BPF maps** survive *iff* pinned to bpffs. Layers 2 and 3 are well-trodden, reachable in Overdrive's aya stack today (`map_pin_path`, `Map::from_pin`, `PinnedLink`), and already partly practised (the project pins maps under `/sys/fs/bpf/overdrive/`).

The canonical analog, **Cilium**, validates the sockops+map half conclusively: the agent is explicitly *not* in the forwarding critical path, and pinned eBPF programs/maps keep forwarding across a `cilium-agent` restart (`--restore`). But it does **not** validate the kTLS half — and this research **resolves the prior doc's open gap**: Cilium's transparent encryption is **IPsec / WireGuard / ztunnel, not kTLS**. No comparable mesh (Cilium, Istio/ztunnel, Linkerd) terminates TLS in the kernel via per-socket kTLS and then restarts its broker — they all either tunnel-encrypt or terminate in a userspace proxy, and all **re-issue on restart**. So Overdrive's "kernel retains the operative crypto, no re-mint" posture is **novel relative to the field** and reachable only by diverging from the universal re-issue pattern.

**Verdict: a Tier-3 spike is still required.** The kernel *mechanisms* are settled by primary/authoritative sources; the sockops+map survival has a production precedent. But the kTLS-survives-restart claim depends on (a) an *undecided* fd-ownership choice in #26 and (b) a *composed runtime behaviour* — kTLS sequence-number/record continuity across the configuring process's death, plus clean re-hydration of a fresh CP — that no documentation guarantees and no Tier-2 harness (`BPF_PROG_TEST_RUN` is unavailable for the relevant socket-context hooks) can exercise. Per this project's hard-won precedent that real-kernel runtime behaviour must not be overstated from docs, the design must be validated on Lima before "no re-mint" is locked. The minimal experiment is specified in the Verdict §C.

## Research Methodology
**Search Strategy**: Layer-by-layer (kTLS → sockops → BPF maps → composed sequence → Cilium precedent), prioritising kernel.org / docs.kernel.org / man7.org (authoritative for syscall & setsockopt semantics), LWN (industry-medium-high, cross-referenced), Cilium/Isovalent/ebpf.io (open-source canonical analog), aya-rs.dev / docs.rs (technical docs for Overdrive's stack). Kernel source (elixir.bootlin.com / git.kernel.org) cited as primary source where docs are silent.
**Source Selection**: Types: official (kernel docs, man-pages, RFC), open_source (Cilium), technical_documentation (aya, ebpf.io), industry (LWN — cross-referenced). Reputation: high min for kernel mechanism claims; LWN/GitHub cross-referenced.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative). All kernel-runtime-behaviour claims explicitly separated into "documented mechanism" vs "contingent on wiring" vs "empirically open". Adversarial posture per project precedent (a cgroup-hook firing-scope decision shipped wrong because research+review disagreed with real-kernel behaviour).

## The precise question
On a control-plane / worker process restart, must Overdrive **re-mint** every running workload's SVID to re-supply the kernel, or does the **kernel retain the operative crypto** (so the CP only re-hydrates a management view, no re-mint)? Answered per layer: **what owns the state, what tears it down, whether the death/restart of the configuring userspace process matters.**

## Findings

### Layer 1 — kTLS (kernel TLS, `SOL_TLS` / `TCP_ULP="tls"`)

**Finding 1.1 — kTLS crypto state lives on the *socket*, not the process: it is stored in `icsk_ulp_data` on the `inet_connection_sock`.**
**Evidence**: The original kernel TLS patch defines the canonical accessor:
```c
static inline struct tls_context *tls_get_ctx(const struct sock *sk) {
    struct inet_connection_sock *icsk = inet_csk(sk);
    return icsk->icsk_ulp_data;
}
```
"This retrieves the TLS context by accessing the `icsk_ulp_data` field of the inet connection socket, confirming the crypto state lives on the socket itself." kTLS is "an Upper Layer Protocol (ULP) that runs over TCP"; "A standard TCP socket is converted to a TLS socket using a setsockopt."
**Source**: [kernel.org Patchwork — "[v3,net-next,3/4] tls: kernel TLS support" (Dave Watson, 2017-06-14)](https://patchwork.kernel.org/project/linux-crypto/patch/20170614183739.GA80368@davejwatson-mba.dhcp.thefacebook.com/) — PRIMARY SOURCE (kernel C) — Accessed 2026-06-09
**Confidence**: High (primary kernel source + two cross-refs)
**Verification**: [LWN — "kernel TLS" (Articles/725721)](https://lwn.net/Articles/725721/) ("A standard TCP socket is converted to a TLS socket using a setsockopt"); [docs.kernel.org — Kernel TLS](https://docs.kernel.org/networking/tls.html) ("Upper Layer Protocol (ULP)… replacement for the record layer").
**Analysis**: `icsk_ulp_data` is a field of the kernel `inet_connection_sock` object — the socket. The configuring process is irrelevant to where the state lives: it lives in the socket's kernel struct, reachable by any holder of the fd. This is the load-bearing fact for the whole question. The symmetric keys, IV, and record sequence number are kernel-resident, not userspace-resident, once installed.

**Finding 1.2 — All kTLS options are set/inspected *per-socket*; state is observable via `getsockopt` and socket diag (`ss`), independent of the configuring process.**
**Evidence**: "All options are set per-socket using setsockopt(), and their state can be checked using getsockopt() and via socket diag (ss)." A dedicated `tls: add socket diag` patch series adds `ss`-visibility of TLS socket state.
**Source**: [docs.kernel.org — Kernel TLS](https://docs.kernel.org/networking/tls.html) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [lkml — "[PATCH net-next v3 0/3] net: tls: add socket diag"](https://lkml.kernel.org/netdev/cover.1567158431.git.dcaratti@redhat.com/T/) (state inspectable via socket diag, i.e. an attribute of the socket); [LWN — kernel TLS](https://lwn.net/Articles/725721/).
**Analysis**: "Per-socket" + "inspectable via socket diag" is independent confirmation that the state is an attribute of the kernel socket object, queryable by any tool with access to the netlink socket-diag interface — not a private property of the userspace process that ran `setsockopt`.

**Finding 1.3 — kTLS crypto state is torn down on *socket close*, via `tls_sk_proto_close` freeing the context.**
**Evidence**:
```c
static void tls_sk_proto_close(struct sock *sk, long timeout) {
    struct tls_context *ctx = tls_get_ctx(sk);
    ...
    ctx->free_resources(sk);
    kfree(ctx->rec_seq);
    kfree(ctx->iv);
    ...
    kfree(ctx);
}
```
"This ensures all cryptographic material and context structures are freed upon socket closure."
**Source**: [kernel.org Patchwork — tls: kernel TLS support](https://patchwork.kernel.org/project/linux-crypto/patch/20170614183739.GA80368@davejwatson-mba.dhcp.thefacebook.com/) — PRIMARY SOURCE — Accessed 2026-06-09
**Confidence**: High (primary kernel source)
**Verification**: Cross-referenced against [docs.kernel.org — Kernel TLS](https://docs.kernel.org/networking/tls.html) which documents no API to detach the ULP from a live socket (no `TLS_TX`-disable path) — i.e. the only documented teardown is closing the socket. _Single primary source for the exact free path; the close-bound teardown is corroborated structurally by the socket-ownership mechanism in 1.1._
**Analysis**: The teardown trigger is `close()` on the socket fd — when the last fd referencing the socket is closed and the kernel `struct sock` is destroyed, `tls_sk_proto_close` runs and the keys are zeroed/freed. The *exit of the configuring process* is only a teardown trigger insofar as that process exiting closes its copy of the fd. If another process still holds the fd (inheritance via `fork`, or transfer via `SCM_RIGHTS`), the kernel socket — and its kTLS context — persists.

**Finding 1.4 — After install, userspace is out of the per-record data path: the kernel does record framing + symmetric crypto autonomously.**
**Evidence**: "Only symmetric crypto is done in the kernel, keys are passed by setsockopt after the handshake is complete." Once installed, "the kernel's TLS ULP layer intercepts user write requests and handles them without requiring ongoing userspace involvement in the data path. Userspace remains in the control plane (making the initial enable/install request) but not the data plane."
**Source**: [kernel.org Patchwork — tls: kernel TLS support](https://patchwork.kernel.org/project/linux-crypto/patch/20170614183739.GA80368@davejwatson-mba.dhcp.thefacebook.com/); [docs.kernel.org — Kernel TLS offload](https://docs.kernel.org/networking/tls-offload.html) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [LWN — kernel TLS](https://lwn.net/Articles/725721/) ("only symmetric crypto… in the kernel… handshake remains in userspace").
**Analysis**: This is what makes "the kernel retains the operative crypto" a coherent design: once keys are installed, the configuring process can leave the data loop entirely. The workload's own `send()/recv()` (or a sidecar's) drives the encrypted bytes; the control plane is not in the path. **Layer-1 verdict: kTLS state survives configuring-process exit IFF a still-living process holds the socket fd. It does NOT survive the socket closing.**

### Layer 2 — eBPF sockops attachment (`BPF_PROG_TYPE_SOCK_OPS` / `BPF_CGROUP_SOCK_OPS`)

**Finding 2.1 — sockops programs attach at the cgroup (`BPF_CGROUP_SOCK_OPS`), via either legacy `BPF_PROG_ATTACH` or a `bpf_link`. These two attach mechanisms have *different* survival semantics — this distinction is the crux of Layer 2.**
**Evidence**: "Socket ops programs are attached to cgroups via the BPF_PROG_ATTACH syscall or via BPF link." The kernel exposes `BPF_LINK_CREATE` ("Attach an eBPF program to a target_fd at the specified attach_type hook and return a file descriptor handle for managing the link") alongside the older `BPF_PROG_ATTACH` ("Attach an eBPF program to a target_fd at the specified attach_type hook").
**Source**: [docs.ebpf.io — BPF_PROG_TYPE_SOCK_OPS](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/); [docs.kernel.org — eBPF Syscall (userspace-api)](https://www.kernel.org/doc/html/latest/userspace-api/ebpf/syscall.html) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [docs.ebpf.io — BPF_LINK_CREATE](https://docs.ebpf.io/linux/syscall/BPF_LINK_CREATE/).
**Analysis**: The attach *target* is the cgroup, not the workload socket and not the loader process. A sockops program fires for sockets created by processes in that cgroup. Whether the attachment outlives the loader depends entirely on which of the two attach mechanisms was used (2.2 / 2.3).

**Finding 2.2 — Legacy `BPF_PROG_ATTACH` to a cgroup: the *cgroup owns* the attachment (it bumps `cgroup->bpf.refcnt`), so the attachment survives the loader process exiting by default.**
**Evidence**: "As opposed to direct bpf_prog attachment, cgroup itself doesn't 'own' bpf_link… But bpf_link doesn't bump cgroup->bpf.refcnt as well." (Stated as the *contrast*: direct `BPF_PROG_ATTACH` DOES make the cgroup own the attachment and DOES hold a reference.) "BPF_PROG_ATTACH … holds a reference preventing cgroup cleanup."
**Source**: [lore.kernel.org — "[PATCH v3 bpf-next 0/4] Add support for cgroup bpf_link" (Andrii Nakryiko)](https://lore.kernel.org/all/869adb74-5192-563d-0e8a-9cb578b2a601@solarflare.com/T/) — PRIMARY SOURCE (kernel mailing list cover letter) — Accessed 2026-06-09 _(fetched via search-engine index; direct fetch blocked by lore.kernel.org bot-protection — flagged as single-render-path, cross-referenced below)_
**Confidence**: High (primary kernel source, cross-referenced)
**Verification**: [docs.kernel.org — eBPF Syscall](https://www.kernel.org/doc/html/latest/userspace-api/ebpf/syscall.html) — an eBPF object is deallocated only after "all file descriptors referring to the object have been closed and no references remain pinned to the filesystem **or attached** (for example, bound to a program or device)"; "attached" is the cgroup-attach reference.
**Analysis**: With the legacy attach API, the loader can `close()` its program fd and exit entirely — the cgroup holds the reference and the program keeps firing for new sockets in that cgroup. This is the historically simple "fire and forget" model. The downside the kernel devs called out: an abrupt loader crash leaves the attachment orphaned (no auto-cleanup) — which motivated `bpf_link`.

**Finding 2.3 — `bpf_link`-based cgroup attach: the link is destroyed (and the program auto-detached) when the last fd referencing it is closed — i.e. it auto-detaches on loader exit UNLESS the link is pinned to bpffs.**
**Evidence**: "bpf_link is destroyed and automatically detached when the last open FD holding the reference to bpf_link is closed. This means that by default, when the process that created bpf_link exits, attached BPF program will be automatically detached due to bpf_link's clean up code." And: "Cgroup bpf_link, like any other bpf_link, can be pinned in BPF FS and by those means survive the exit of process that created the link." Also: "Auto-detachment of cgroup bpf_link is implemented. When cgroup is dying it will automatically detach all active bpf_links."
**Source**: [lore.kernel.org — Add support for cgroup bpf_link (Andrii Nakryiko)](https://lore.kernel.org/all/869adb74-5192-563d-0e8a-9cb578b2a601@solarflare.com/T/) — PRIMARY SOURCE — Accessed 2026-06-09 _(via search-engine index; direct fetch bot-blocked)_
**Confidence**: High (primary kernel source, cross-referenced)
**Verification**: [docs.ebpf.io — BPF_LINK_CREATE](https://docs.ebpf.io/linux/syscall/BPF_LINK_CREATE/) ("the link is destroyed once no more references to it exist, which might happen if the loader exits without pinning the link or if the pin gets deleted"); [docs.kernel.org — eBPF Syscall](https://www.kernel.org/doc/html/latest/userspace-api/ebpf/syscall.html) (pin "retains a reference to the eBPF object, preventing deallocation when the original bpf_fd is closed").
**Analysis**: This is the modern (and aya-default — see 5.4) path, and it is the dangerous one for a restart story: **a `bpf_link` sockops attachment that is NOT pinned to bpffs auto-detaches the instant the loader process exits.** A control-plane restart that used an unpinned link would tear down the attachment. The fix is explicit bpffs-pinning of the link. **Layer-2 verdict: sockops attachment survives loader/CP exit IFF either (a) legacy `BPF_PROG_ATTACH` was used (cgroup owns it), or (b) a `bpf_link` was used AND pinned to bpffs. An unpinned `bpf_link` does NOT survive.**

### Layer 3 — BPF map lifetime (the well-trodden part)

**Finding 3.1 — A BPF map is reference-counted by open fds; when the last fd closes (e.g. the creating process exits), the map is deallocated — UNLESS it is pinned to bpffs.**
**Evidence**: From the authoritative `bpf(2)` man page: "Maps can be deleted by calling close(fd). Maps held by open file descriptors will be deleted automatically when a process exits… when the user-space program that created a map exits, all maps will be deleted automatically." And on pinning: `BPF_OBJ_PIN` "pathname retains a reference to the eBPF object, preventing deallocation of the object when the original bpf_fd is closed. This allows the eBPF object to live beyond close(bpf_fd), and hence the lifetime of the parent process." Unpinning: "Applying unlink(2)… unpins the object from the filesystem, removing the reference. If no other file descriptors or filesystem nodes refer to the same object, it will be deallocated."
**Source**: [man7.org — bpf(2)](https://www.man7.org/linux/man-pages/man2/bpf.2.html) — AUTHORITATIVE (Linux man-pages, official for syscall semantics) — Accessed 2026-06-09
**Confidence**: High (authoritative + two cross-refs)
**Verification**: [docs.kernel.org — eBPF Syscall](https://www.kernel.org/doc/html/latest/userspace-api/ebpf/syscall.html) ("deallocated only after all file descriptors referring to the object have been closed and no references remain pinned to the filesystem or attached"); [docs.ebpf.io — Pinning](https://docs.ebpf.io/linux/concepts/pinning/) and [LWN — "Persistent BPF objects" (Articles/664688)](https://lwn.net/Articles/664688/) (bpffs pinning is the mechanism that gives maps lifetime beyond the creating process).
**Analysis**: This is the canonical, unambiguous result. A control plane that creates maps, populates them, then exits without pinning them — loses them. With a bpffs pin, the map (and its contents — the populated VIP→backend / identity entries) survives the process exit and the next process re-acquires it via `BPF_OBJ_GET` on the pin path. **Layer-3 verdict: map contents survive CP restart IFF the map is pinned to bpffs (or another process holds an fd). Otherwise they are lost on the last-fd-close at process exit.**

### Layer 4 — The composed sequence (kTLS keys + sockops attach + maps, then CP exit/restart)

**Finding 4.1 — The three layers have *three different owners*, so a single restart story does not exist — each must be tracked separately.**

| State | Owner | Survives CP exit by default? | Survives IFF |
|---|---|---|---|
| kTLS crypto (keys/IV/seq) | the **socket** (`icsk_ulp_data`) | only if a live process still holds the socket fd | a non-CP process (the workload, or a surviving sidecar) owns the fd; torn down on socket close |
| sockops attachment | the **cgroup** (legacy attach) **or** the **`bpf_link`** | legacy attach: **yes**; `bpf_link`: **no** | legacy `BPF_PROG_ATTACH`, OR `bpf_link` pinned to bpffs |
| BPF maps (identity/VIP entries) | the **map object** (fd-refcounted) | **no** (freed at last-fd-close on CP exit) | pinned to bpffs (or another process holds an fd) |

**Source**: Synthesis of Findings 1.1/1.3 ([kernel.org Patchwork](https://patchwork.kernel.org/project/linux-crypto/patch/20170614183739.GA80368@davejwatson-mba.dhcp.thefacebook.com/)), 2.2/2.3 ([lore.kernel.org cgroup bpf_link](https://lore.kernel.org/all/869adb74-5192-563d-0e8a-9cb578b2a601@solarflare.com/T/)), 3.1 ([man7.org bpf(2)](https://www.man7.org/linux/man-pages/man2/bpf.2.html)).
**Confidence**: High (each row independently sourced above)
**Analysis**: The table is the heart of the answer. Two of the three states (sockops with legacy attach; maps with bpffs pin) are straightforwardly made restart-survivable by well-documented mechanisms. **The kTLS row is the hard one** — and it is hard for a reason that no pin can fix: kTLS state is bound to a *socket*, and a socket is owned by whatever process holds its fd. There is no "pin a kTLS context to bpffs" primitive.

**Finding 4.2 — The decisive variable for kTLS survival is *who owns the workload socket fd*, and that is a property of Overdrive's #26 design, not of the kernel.**
**Evidence**: kTLS state is freed by `tls_sk_proto_close` only when the socket's last fd closes (Finding 1.3); the configuring process exiting matters *only* because it closes that process's copy of the fd (Finding 1.1, 1.4 — userspace is out of the data path post-install).
**Source**: Findings 1.1, 1.3, 1.4 (see Layer 1 citations).
**Confidence**: High for the mechanism; the *applicability* to #26 is contingent (see Verdict §B/§C).
**Analysis**: Two sub-cases, and Overdrive's wiring decides which one holds:
- **(a) The workload owns the socket fd** (the workload calls `connect()/accept()`, the CP/worker only ran `setsockopt(TCP_ULP)` + installed keys on that already-open fd, e.g. via a `bpf_sockops`-triggered path or an fd passed transiently). Then CP exit does **not** close the socket — the workload still holds it — and the kTLS context **persists**. The CP can restart and re-hydrate a management view without re-minting, because the operative crypto is on a socket the CP never owned.
- **(b) The CP/worker owns the socket fd** (it terminates TLS on a fd it holds, e.g. a transparent-proxy / sidecar-in-the-CP model). Then CP exit **closes the fd**, `tls_sk_proto_close` fires, the keys are freed, the connection drops. Re-mint is unavoidable for any *new* connection; existing connections are simply gone.
This is exactly the "kernel *can* preserve this" vs "Overdrive's design *will* preserve this" split the brief demands. The kernel mechanism is settled; the outcome for #26 hinges on a wiring choice that is **not yet made** (#26 is NOT YET BUILT).

**Finding 4.3 — Even where state survives, there is a *re-hydration* obligation: the restarted CP must re-acquire handles (re-`BPF_OBJ_GET` pinned maps, `PinnedLink::from_pin` the link) — survival of state ≠ automatic re-attachment of the management view.**
**Evidence**: A pinned object "can be reloaded and accessed by a new process instance" via the pin path ([man7.org bpf(2)](https://www.man7.org/linux/man-pages/man2/bpf.2.html), [docs.rs aya MapData](https://docs.rs/aya/latest/aya/maps/struct.MapData.html)); aya `PinnedLink::from_pin` "Creates a PinnedLink from a valid path on bpffs" ([docs.rs aya PinnedLink](https://docs.rs/aya/latest/aya/programs/links/struct.PinnedLink.html)).
**Source**: [man7.org — bpf(2)](https://www.man7.org/linux/man-pages/man2/bpf.2.html); [docs.rs — aya PinnedLink](https://docs.rs/aya/latest/aya/programs/links/struct.PinnedLink.html)
**Confidence**: High
**Analysis**: "No re-mint" does NOT mean "no restart work." The restarted CP still has to walk the bpffs pin directory, reattach its handles, and rebuild its in-memory index of which workload maps to which kernel state. This is the "re-hydrate a management view" the brief anticipates — and it is exactly Cilium's `--restore` model (Finding 5.1). The audit-row-of-issuance-facts (per #26's design) is what lets the CP recognise *which* surviving kernel state belongs to *which* identity without re-minting.

### Layer 5 — Cilium precedent (canonical agent-restart-resilient analog) + aya reachability

**Finding 5.1 — Cilium's datapath survives a `cilium-agent` restart by design: the agent is explicitly NOT in the critical forwarding path; the eBPF programs and (bpffs-pinned) maps run in the kernel decoupled from the agent process.**
**Evidence**: "The Cilium agent is not in the critical path for any forwarding or network policy decision, and a cluster will generally continue to function if the agent is temporarily unavailable." "Packets will continue to be forwarded and network policy rules will continue to be enforced… the actual forwarding logic runs directly in the kernel through eBPF programs, decoupled from the Cilium agent process itself." Agent restart uses `--restore` ("restores state if possible from the previous daemon"); `clean-cilium-bpf-state` "removes all eBPF state from the filesystem on startup" and `clean-cilium-state` "removes recoverable state such as eBPF state pinned to the filesystem" — i.e. the steady-state is that BPF programs/maps ARE pinned to bpffs and ARE recovered on restart.
**Source**: [docs.cilium.io — Component Overview](https://docs.cilium.io/en/stable/overview/component-overview/); [docs.cilium.io — Configuration](https://docs.cilium.io/en/stable/network/kubernetes/configuration/) — Accessed 2026-06-09
**Confidence**: High (official Cilium docs, two pages, mutually corroborating)
**Verification**: [docs.cilium.io — eBPF Maps](https://docs.cilium.io/en/stable/network/ebpf/maps/) (LB-map resize "results in connection disruptions as the new map is repopulated with existing service entries" — implies maps normally persist and are repopulated, not torn down); [docs.cilium.io — ebpf intro](https://docs.cilium.io/en/v1.9/concepts/ebpf/intro/) (socket-layer enforcement via sockops/sockmap).
**Analysis**: This is the direct precedent for Overdrive's question and it validates the architecture: a control plane CAN restart without tearing down kernel datapath state, *provided the kernel objects are bpffs-pinned and the CP is engineered out of the data path*. Cilium proves the sockops/sockmap + map half of the story (Layers 2–3). It does NOT validate the kTLS half (5.2).

**Finding 5.2 — RESOLVING THE PRIOR GAP: Cilium's transparent encryption is IPsec / WireGuard / ztunnel — NOT kTLS. The prior research doc's "unconfirmed" status is now resolved: Cilium does not use kTLS for data-path encryption.**
**Evidence**: "Cilium supports the transparent encryption of Cilium-managed host traffic and traffic between Cilium-managed endpoints using IPsec, WireGuard®, or ztunnel." The transparent-encryption page "does not mention TLS or kTLS anywhere in relation to data-path encryption between endpoints. The only encryption methods discussed are IPsec, WireGuard, and ztunnel." WireGuard: the agent "establishes a secure WireGuard tunnel between it and all other known nodes"; IPsec uses "Kubernetes secrets to distribute the IPsec keys."
**Source**: [docs.cilium.io — Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption/) — Accessed 2026-06-09
**Confidence**: High (official Cilium docs, explicit enumeration; absence-of-kTLS confirmed against the canonical encryption overview page)
**Verification**: [docs.cilium.io — WireGuard Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-wireguard/); [docs.cilium.io — IPsec Transparent Encryption](https://docs.cilium.io/en/latest/security/network/encryption-ipsec/); [Isovalent Labs — Transparent Encryption with IPsec and WireGuard](https://isovalent.com/labs/cilium-transparent-encryption-with-ipsec-and-wireguard/).
**Analysis**: This is a **material divergence** for Overdrive to flag. Cilium encrypts at the *network/tunnel* layer (node-to-node IPsec/WireGuard), or via a ztunnel proxy — not by terminating TLS *in the kernel on the socket* via kTLS the way #26 proposes. Consequence: **Cilium is a valid precedent for "sockops attach + maps survive agent restart" (Layers 2–3) but NOT for "kTLS crypto survives CP restart" (Layer 1).** WireGuard/IPsec key state lives in the WireGuard/xfrm kernel subsystems (node-scoped, agent-reprovisioned), a different lifetime model from per-socket kTLS. There is **no large-scale production precedent found** for per-socket kTLS surviving a control-plane restart in an identity-brokered mesh — the closest analogs (Cilium, Istio ztunnel, Linkerd) all keep TLS termination in a *userspace proxy* or use *tunnel-layer* encryption, both of which sidestep the per-socket-kTLS-ownership question entirely. (See Knowledge Gaps.)

**Finding 5.3 — kTLS does have an in-kernel handshake/upcall facility (`tls-handshake`), and userspace mesh-TLS-in-kernel work exists — but none of it is documented as surviving a control-plane restart.**
**Evidence**: The kernel ships an "In-Kernel TLS Handshake" facility (docs.kernel.org/networking/tls-handshake) that hands off the handshake to a userspace agent then installs kTLS; this is used by in-kernel consumers (e.g. NFS/RPC, QUIC experiments), not by a workload-identity mesh.
**Source**: [docs.kernel.org — In-Kernel TLS Handshake](https://docs.kernel.org/networking/tls-handshake.html) — Accessed 2026-06-09
**Confidence**: Medium (single official source; relevance is by analogy, not direct)
**Verification**: PENDING — no second source found that ties kTLS-handshake-upcall to control-plane-restart survival. Flagged as a single-source observation.
**Analysis**: The existence of a kernel handshake-upcall does not change the ownership answer (the resulting kTLS context still lives on the socket per Layer 1). It is noted only to pre-empt the objection "but the kernel can do the handshake too" — it can, but that is orthogonal to whether the *installed* crypto survives the *configuring agent* restarting.

**Finding 5.4 — aya (Overdrive's stack) fully exposes the survival mechanism: `BpfLoader::map_pin_path`, `Map::from_pin`, and `PinnedLink` / `PinnedLink::from_pin` reach bpffs pinning and reload-after-restart. The Layer-2/3 survival path is reachable in Rust today.**
**Evidence**: aya `PinnedLink` is "A pinned file descriptor link. This link has been pinned to the BPF filesystem. On drop, the file descriptor that backs this link will be closed. **Whether or not the program remains attached is dependent on the presence of the file in BPFFS**." `from_pin()` "Creates a PinnedLink from a valid path on bpffs"; `unpin()` "removes the pinned link from the filesystem and returns an FdLink." For maps: "When a BPF object is pinned to a BPF filesystem it will remain loaded after Aya has unloaded the program… to remove the program, the file on the BPF filesystem must be removed"; `BpfLoader::map_pin_path` loads/pins maps under a bpffs path; `Map::from_pin` reloads a pinned map.
**Source**: [docs.rs — aya `PinnedLink`](https://docs.rs/aya/latest/aya/programs/links/struct.PinnedLink.html); [docs.rs — aya `MapData`](https://docs.rs/aya/latest/aya/maps/struct.MapData.html) — Accessed 2026-06-09
**Confidence**: High (official aya API docs)
**Verification**: [aya-rs.dev — Getting Started book](https://aya-rs.dev/book/); [docs.rs — aya `BpfLoader`](https://docs.rs/aya/0.10.5/aya/struct.BpfLoader.html).
**Analysis**: Critically, aya's `PinnedLink` doc restates the exact Layer-2.3 kernel rule — attachment survival "is dependent on the presence of the file in BPFFS." So the unpinned-`bpf_link`-auto-detaches hazard is a live concern in the aya path specifically: if Overdrive attaches sockops via a plain `bpf_link` (aya's default for link-based attach) and does not call the pin path, a worker restart drops the attachment. The mechanism to avoid that (`PinnedLink`) exists and is one call away — but it must be deliberately wired. **Note:** this maps onto the project's existing `pinning = ByName` discipline already used for HASH_OF_MAPS (per `.claude/rules/development.md` § "Sharing the outer HoM… `pinning = ByName`"), confirming Overdrive already pins maps under `/sys/fs/bpf/overdrive/`. Pinning *links* is the same idiom not-yet-applied to sockops (#26 unbuilt).

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| kernel TLS support patch (Dave Watson) | patchwork.kernel.org | High (1.0) | official / primary kernel C | 2026-06-09 | Y |
| Kernel TLS docs | docs.kernel.org | High (1.0) | official | 2026-06-09 | Y |
| Kernel TLS offload docs | docs.kernel.org | High (1.0) | official | 2026-06-09 | Y |
| In-Kernel TLS Handshake docs | docs.kernel.org | High (1.0) | official | 2026-06-09 | N (single-source, flagged) |
| eBPF Syscall (userspace-api) | kernel.org | High (1.0) | official | 2026-06-09 | Y |
| bpf(2) man page | man7.org | High (1.0) | authoritative (man-pages) | 2026-06-09 | Y |
| "Add support for cgroup bpf_link" cover letter | lore.kernel.org | High (1.0) | official / primary kernel ML | 2026-06-09 | Y (direct fetch bot-blocked; via index + cross-ref) |
| LWN "kernel TLS" (725721) | lwn.net | Medium-High (0.8) | industry | 2026-06-09 | Y |
| LWN "Persistent BPF objects" (664688) | lwn.net | Medium-High (0.8) | industry | 2026-06-09 | Y |
| socket diag patch (lkml) | lkml.kernel.org / kernel.org | High (1.0) | official / primary kernel ML | 2026-06-09 | Y |
| docs.ebpf.io BPF_PROG_TYPE_SOCK_OPS / BPF_LINK_CREATE / Pinning | ebpf.io | High (1.0) | technical_documentation | 2026-06-09 | Y |
| Cilium Component Overview | docs.cilium.io | High (1.0) | open_source | 2026-06-09 | Y |
| Cilium Transparent Encryption | docs.cilium.io | High (1.0) | open_source | 2026-06-09 | Y |
| Cilium Configuration / eBPF Maps / ebpf intro | docs.cilium.io | High (1.0) | open_source | 2026-06-09 | Y |
| Isovalent Labs — Transparent Encryption | isovalent.com | High (1.0) | open_source | 2026-06-09 | Y |
| aya PinnedLink / MapData / BpfLoader | docs.rs | High (1.0) | technical_documentation | 2026-06-09 | Y |
| aya book | aya-rs.dev | High (1.0) | technical_documentation | 2026-06-09 | Y |

Reputation: High: 15 (~88%) | Medium-High: 2 (~12%) | Avg: ~0.97. Cross-verified: 16/18 distinct sources (the 2 not cross-verified — the in-kernel-handshake observation and one LWN — are explicitly flagged as single-source / corroborating-only and carry no load-bearing claim alone).

## Knowledge Gaps

### Gap 1: No production precedent for per-socket kTLS surviving a control-plane/identity-broker restart
**Issue**: Every comparable mesh (Cilium, Istio/ztunnel, Linkerd) either encrypts at the tunnel/network layer (WireGuard/IPsec — Finding 5.2) or terminates TLS in a *userspace proxy* (Envoy/linkerd-proxy/ztunnel — prior research doc Findings 3.2, 4.1). None terminates TLS in the kernel via kTLS on a workload socket and then restarts the broker. So there is no external "this works in production" datapoint for Overdrive's exact Layer-1 claim.
**Attempted**: Searched docs.cilium.io, isovalent.com, ebpf.io for kTLS + agent-restart; searched kernel docs for kTLS persistence APIs. Found the handshake-upcall facility (5.3) but no restart-survival evidence.
**Recommendation**: Treat the kTLS-survival claim as Overdrive-novel; validate via the Tier-3 spike (Verdict §C) rather than by precedent. Do not assume "Cilium does it" — Cilium does not.

### Gap 2: kTLS sequence-number / record-state continuity across configuring-process death is undocumented for this handoff
**Issue**: Docs confirm the *context* is socket-owned, but none demonstrates that a kTLS data exchange continues correctly (correct decryption, no record-boundary corruption, sequence numbers intact) *after* the configuring process dies while another process drives the socket. This is a runtime property.
**Attempted**: kernel.org TLS docs/offload docs, LWN articles, bpf(2). All silent on the cross-process-after-configurator-death data path.
**Recommendation**: This is the precise observable in Tier-3 spike step 5. It cannot be settled by reading.

### Gap 3: #26's intended fd-ownership and kTLS-install path is not yet specified
**Issue**: Whether the workload or the worker ends up owning the socket fd — and from what context kTLS keys are installed — is a design decision not yet made (#26 NOT YET BUILT). The entire kTLS restart story (Verdict §B, sub-cases 4.2(a) vs 4.2(b)) pivots on it.
**Attempted**: Reviewed `.claude/rules` (workflows/reconcilers/development) and the prior SVID-lifecycle research doc; the kernel-mediated-mTLS model is described (worker holds SVID material, workload identity-unaware) but the socket-fd-ownership detail for kTLS is not pinned.
**Recommendation**: Pin the fd-ownership decision in the #26 design BEFORE the spike, so the spike validates the *chosen* shape (4.2(a)) rather than exploring. If 4.2(b) (CP owns fd) is chosen, the answer is already "re-mint forced" and no spike is needed for that path.

## Conflicting Information

### Conflict 1: "kernel-mediated mTLS survives broker restart" (Overdrive design hope) vs the universal industry pattern of re-issue-on-restart
**Position A** (kernel *can* preserve it): kTLS state is socket-owned and survives configuring-process exit while the fd lives (Findings 1.1–1.4, primary kernel source). Reputation: 1.0.
**Position B** (every comparable system re-issues): SPIRE, Istio, Linkerd all hold leaf material in memory and **re-issue / re-attest on restart** — they do NOT preserve kernel/operative crypto across a broker restart (prior research doc Findings 1.5, 3.2, 4.1). Reputation: 1.0.
**Assessment**: Not a true contradiction — they answer different questions. Position B describes systems where the broker (agent/sidecar) *owns* the crypto material in *userspace*, so its restart necessarily loses it (the 4.2(b)-analog). Position A is only reachable because Overdrive proposes to push crypto into the *socket* (4.2(a)), which those systems do not do. The conflict resolves to: **Overdrive's "no re-mint" is achievable only by diverging from the universal pattern via kernel kTLS + workload-owned fd — which is exactly why it needs empirical validation, not precedent.** The industry weight of Position B is a caution flag, not a refutation.

## Research-Resolved vs Spike-Required (THE VERDICT)

### A. Settled by documented kernel mechanism (independent of Overdrive's wiring)

These are true regardless of how #26 is built — they are properties of the Linux kernel, each with a primary or authoritative source:

1. **kTLS crypto state lives on the socket** (`icsk_ulp_data` on `inet_connection_sock`), not in the configuring process. It is freed on **socket close** (`tls_sk_proto_close`), and is observable per-socket via socket-diag. *It is NOT freed merely because the process that called `setsockopt` exits — only because that exit closed the fd.* [Findings 1.1–1.4; kernel.org Patchwork PRIMARY + docs.kernel.org + LWN]
2. **There is no kernel primitive to persist a kTLS context independently of its socket.** No bpffs-pin equivalent for kTLS. The only thing that keeps it alive is a live fd on the socket. [Finding 1.3; settled by absence of any documented detach/persist API across all kTLS sources]
3. **A sockops attachment survives the loader exiting IFF** it was made via legacy `BPF_PROG_ATTACH` (cgroup owns it) **or** via a `bpf_link` that is **pinned to bpffs**. An **unpinned `bpf_link` auto-detaches on loader exit.** [Findings 2.2–2.3; lore.kernel.org PRIMARY + docs.kernel.org + docs.ebpf.io]
4. **A BPF map's contents survive the creating process exiting IFF the map is pinned to bpffs** (or another process holds an fd). Otherwise freed at last-fd-close. [Finding 3.1; man7.org bpf(2) AUTHORITATIVE + docs.kernel.org + LWN]
5. **Cilium proves the sockops+map half at production scale**: the agent is explicitly out of the forwarding critical path; pinned eBPF programs/maps keep forwarding across `cilium-agent` restart (`--restore`). [Finding 5.1; docs.cilium.io official]
6. **Cilium does NOT use kTLS** for data-path encryption — it uses IPsec / WireGuard / ztunnel. So Cilium is **not** a precedent for the kTLS row. [Finding 5.2; docs.cilium.io official — RESOLVES the prior doc's gap]
7. **aya exposes the pinning mechanism** (`map_pin_path`, `Map::from_pin`, `PinnedLink::from_pin`) — the Layer-2/3 survival path is reachable in Overdrive's stack today. [Finding 5.4; docs.rs aya official]

### B. Contingent on Overdrive design choices (crisp preconditions)

Each of these is *achievable* but only if #26 is deliberately wired for it:

- **Sockops attachment survives CP restart** *IFF* Overdrive pins the sockops `bpf_link` to bpffs (e.g. via aya `PinnedLink`) — or uses legacy `BPF_PROG_ATTACH`. The project already pins *maps* under `/sys/fs/bpf/overdrive/` (`pinning = ByName`); pinning the sockops *link* is the same idiom, not-yet-applied. **Precondition: pin the link.**
- **BPF map contents (identity/VIP/backend entries) survive CP restart** *IFF* the maps are bpffs-pinned. Overdrive already does this for HASH_OF_MAPS. **Precondition: maps pinned (already the project default).**
- **kTLS crypto survives CP restart** *IFF* **the workload — not the control plane / worker — owns the socket fd** (sub-case 4.2(a)). If the CP terminates TLS on a fd *it* holds (sub-case 4.2(b)), CP exit closes the socket and the connection (and its kTLS state) is gone — re-mint is forced for new connections. **Precondition: the operative socket fd is owned by a process that outlives the CP restart (the workload, or a separately-restarting sidecar).** This precondition is the single most load-bearing design decision for #26's restart story, and it is currently **undecided** (#26 NOT YET BUILT).

### C. Genuinely empirically open → Tier-3 spike required

Documentation settles the *mechanisms* but cannot settle the *behaviour of #26's specific composed sequence on a real kernel*, for three reasons the docs are silent on — and this project has hard-won precedent (a cgroup-hook firing-scope decision shipped wrong because research+review disagreed with real-kernel behaviour; `cgroup_sock_addr` has no `BPF_PROG_TEST_RUN`, so it can only be settled at Tier 3):

1. **Does a kTLS context actually keep encrypting/decrypting correctly after the configuring process exits, when a *different* process holds the fd?** Every source confirms the *state* is socket-owned, but none demonstrates a *live data exchange continuing across the configuring-process death* with the keys/IV/sequence-number intact. The sequence-number continuity in particular is a runtime property no doc guarantees for this exact handoff.
2. **In Overdrive's intended #26 flow, *who ends up owning the socket fd*, and does the sockops program install kTLS on a fd the workload owns or one the worker owns?** This is a function of the (unbuilt) `bpf_sockops` wiring — whether kTLS is installed from the sockops hook context, from a worker holding a transferred fd, or otherwise. The fd-ownership outcome is an emergent property of the wiring, not a documented constant.
3. **Does a pinned sockops `bpf_link` + pinned maps + a workload-owned kTLS socket, taken together, actually resume cleanly after a `overdrive` worker/CP `kill -9` + restart** — with no re-mint, the management view re-hydrated from the audit row, and in-flight encrypted connections uninterrupted? This is the integrated claim the design rests on, and no combination of docs validates the *composition*.

**The minimal Tier-3 experiment** (narrowed to exactly what must be observed):
> On the Lima VM (real kernel, cgroup v2, eBPF + kTLS), in a test harness: (1) establish a TCP connection where a **child/workload process owns the socket fd**; (2) from a **separate "control-plane" process**, attach a `bpf_link` sockops program (pinned to bpffs) and install kTLS keys on that socket; (3) confirm encrypted bytes flow; (4) `kill -9` the control-plane process; (5) **observe via `ss -K` (socket diag) that the kTLS context is still present on the socket, and confirm the workload can still `send()/recv()` correct ciphertext with sequence-number continuity**; (6) confirm `bpftool link show` / the bpffs pin still shows the sockops attachment, and `bpftool map dump` shows the populated maps; (7) start a fresh CP process, `PinnedLink::from_pin` + `Map::from_pin`, and confirm it re-hydrates the management view **without re-minting**. The pass/fail signal is steps 5–7: state survival under real `kill -9`, observed at the socket and bpffs layers, with a live data exchange — none of which `BPF_PROG_TEST_RUN` or documentation can produce.

### D. One-line verdict

**A Tier-3 spike IS still required** — the kernel *mechanisms* are fully settled (kTLS = socket-owned/freed-on-close; sockops & maps survive *iff* bpffs-pinned), and the sockops+map half has a production precedent in Cilium, but the **kTLS-survives-CP-restart claim hinges on an undecided fd-ownership choice in #26 and on a composed runtime behaviour that no documentation guarantees and no Tier-2 harness can exercise** — so before locking the "no re-mint" design, run the minimal Lima experiment above (workload owns fd → CP `kill -9` → observe kTLS state + pinned attachment survive → fresh CP re-hydrates without re-mint).

## Full Citations

[1] Watson, Dave. "[v3,net-next,3/4] tls: kernel TLS support". kernel.org Patchwork (linux-crypto). 2017-06-14. https://patchwork.kernel.org/project/linux-crypto/patch/20170614183739.GA80368@davejwatson-mba.dhcp.thefacebook.com/. Accessed 2026-06-09. _(PRIMARY SOURCE — kernel C: `tls_get_ctx`, `tls_sk_proto_close`.)_
[2] The Linux Kernel. "Kernel TLS". docs.kernel.org/networking/tls.html. https://docs.kernel.org/networking/tls.html. Accessed 2026-06-09.
[3] The Linux Kernel. "Kernel TLS offload". docs.kernel.org/networking/tls-offload.html. https://docs.kernel.org/networking/tls-offload.html. Accessed 2026-06-09.
[4] The Linux Kernel. "In-Kernel TLS Handshake". docs.kernel.org/networking/tls-handshake.html. https://docs.kernel.org/networking/tls-handshake.html. Accessed 2026-06-09.
[5] The Linux Kernel. "eBPF Syscall" (userspace-api). https://www.kernel.org/doc/html/latest/userspace-api/ebpf/syscall.html. Accessed 2026-06-09.
[6] Linux man-pages. "bpf(2)". https://www.man7.org/linux/man-pages/man2/bpf.2.html. Accessed 2026-06-09. _(AUTHORITATIVE for syscall/map-lifetime semantics.)_
[7] Nakryiko, Andrii. "[PATCH v3 bpf-next 0/4] Add support for cgroup bpf_link". lore.kernel.org. https://lore.kernel.org/all/869adb74-5192-563d-0e8a-9cb578b2a601@solarflare.com/T/. Accessed 2026-06-09. _(PRIMARY SOURCE — cgroup bpf_link vs BPF_PROG_ATTACH ownership; direct fetch bot-blocked, content via search index + cross-ref.)_
[8] LWN.net. "kernel TLS". 2017. https://lwn.net/Articles/725721/. Accessed 2026-06-09.
[9] LWN.net. "Persistent BPF objects". 2015. https://lwn.net/Articles/664688/. Accessed 2026-06-09.
[10] Caratti, Davide et al. "[PATCH net-next v3 0/3] net: tls: add socket diag". lkml/kernel.org. 2019. https://lkml.kernel.org/netdev/cover.1567158431.git.dcaratti@redhat.com/T/. Accessed 2026-06-09.
[11] eBPF Docs. "Program Type 'BPF_PROG_TYPE_SOCK_OPS'". https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/. Accessed 2026-06-09.
[12] eBPF Docs. "Syscall command 'BPF_LINK_CREATE'". https://docs.ebpf.io/linux/syscall/BPF_LINK_CREATE/. Accessed 2026-06-09.
[13] eBPF Docs. "Pinning". https://docs.ebpf.io/linux/concepts/pinning/. Accessed 2026-06-09.
[14] Cilium. "Component Overview". docs.cilium.io. https://docs.cilium.io/en/stable/overview/component-overview/. Accessed 2026-06-09.
[15] Cilium. "Transparent Encryption". docs.cilium.io. https://docs.cilium.io/en/stable/security/network/encryption/. Accessed 2026-06-09.
[16] Cilium. "Configuration" / "eBPF Maps" / "Introduction (eBPF)". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/configuration/, https://docs.cilium.io/en/stable/network/ebpf/maps/, https://docs.cilium.io/en/v1.9/concepts/ebpf/intro/. Accessed 2026-06-09.
[17] Isovalent Labs. "Cilium Transparent Encryption with IPsec and WireGuard". https://isovalent.com/labs/cilium-transparent-encryption-with-ipsec-and-wireguard/. Accessed 2026-06-09.
[18] aya-rs. "PinnedLink" / "MapData" / "BpfLoader" (docs.rs) and the aya book (aya-rs.dev). https://docs.rs/aya/latest/aya/programs/links/struct.PinnedLink.html, https://docs.rs/aya/latest/aya/maps/struct.MapData.html, https://aya-rs.dev/book/. Accessed 2026-06-09.

## Research Metadata
Duration: ~1 session | Examined: 20+ sources | Cited: 18 distinct | Cross-refs: 16/18 distinct sources cross-verified | Primary kernel sources: 4 ([1] tls patch C, [6] bpf(2) man-page, [7] cgroup bpf_link ML, [10] socket-diag ML) | Confidence: kernel mechanisms High; #26 composed behaviour Medium (empirically open → Tier-3 spike) | Output: docs/research/dataplane/ktls-sockops-cp-restart-survival-research.md

### Adversarial-validation note (per operational-safety + project precedent)
Web-fetched content was treated as untrusted: every load-bearing claim is tied to a primary kernel source or authoritative man-page, with industry sources (LWN, ebpf.io, Cilium) used for corroboration only. Two sources ([1] kernel TLS patch, [7] cgroup bpf_link) could not be fetched directly (Anubis / elixir JS rendering) and were obtained via search-engine index — both are cross-referenced against independently-fetched official docs ([2]/[5]/[6]/docs.ebpf.io) that state the same mechanism, so no claim rests on an unverified single render path. The verdict deliberately refuses to assert that #26's composed runtime behaviour "will" work from documentation alone — the kernel *can* preserve the state; whether Overdrive's wiring *will* is split out as empirically open, consistent with the project's hard-won precedent that real-kernel runtime behaviour must be observed, not inferred.
