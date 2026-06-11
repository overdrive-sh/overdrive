# Research: sockops mTLS + kTLS Installation (Kernel TLS Offload) — Overdrive Roadmap 2.4 / GH #26

**Date**: 2026-06-04 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium (feasibility crux is Medium; mechanism details are High) | **Sources**: 26

> **Pin updated (2026-06-11): 6.6 → 6.18 LTS.** Written against a 6.6 pin; the appliance kernel is now pinned at the latest qualifying LTS — **6.18** (EOL Dec 2028, ADR-0068). The mechanism analysis is unaffected (6.18 ⊇ 6.6 on every feature). **KeyUpdate correction (2026-06-11):** the "SVID rotation = teardown+reconnect" limitation discussed below is **NOT** removed by the 6.18 pin. In-place rekey needs TLS 1.3 KeyUpdate at *two* layers — the kernel (**confirmed present at v6.18**, Gap 4 resolved) **and** the userspace rustls→kTLS bridge, which does **not** yet support it: the `ktls` crate's KeyUpdate support is an open issue ([rustls/ktls#59](https://github.com/rustls/ktls/issues/59)) and its `rustls::kernel`-based rewrite ([PR#62](https://github.com/rustls/ktls/pull/62)) is unmerged. So in-place rekey is unavailable to Overdrive's recommended stack — the kernel is ready, the **userspace bridge is the sole blocker**; teardown+reconnect stands for v1. The unblocking lever is the **userspace bridge** (adopt `rustls::kernel` + land/vendor the ktls rewrite), **not** the kernel pin.

> Closes deferred **Gap K-2** from `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
> ("aya `SockOps` typed surface, `BPF_SOCK_OPS_*_CB` constants, kTLS install pattern").
> Companion to `docs/research/transparent-encryption-comprehensive-research.md` (landscape;
> WireGuard/IPsec/ztunnel modes) — this document is the *mechanism* research for the
> sockops→kTLS handoff path the whitepaper §7/§8 commits to.

## Executive Summary

The whitepaper's transparent sockops+kTLS mTLS model — where workloads call `connect()` unmodified, the node agent intercepts, performs a rustls TLS 1.3 handshake on the application socket, installs session keys into kTLS, and exits the data path — is **architecturally achievable** using mainline Linux kernel primitives. However, it requires solving one critical unsolved problem that no production system has shipped for general workloads: **how to guarantee the application's first `write()` does not send plaintext during the window between `ACTIVE_ESTABLISHED_CB` firing and kTLS installation completing**.

The kernel primitives are all present and well-documented. The kTLS API (`setsockopt(SOL_TLS, TLS_TX/TLS_RX)` after `TCP_ULP "tls"`) accepts externally-derived key material — it does not require the handshake to have occurred on the same socket. The sockops hook `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB` fires when a connection is established and provides the 5-tuple and `bpf_setsockopt` capabilities. The rustls `secret_extraction` feature exports TLS 1.3 session keys in exactly the format kTLS needs, and the `ktls`/`ktls-core` crates implement the complete mapping from `ConnectionTrafficSecrets` to `setsockopt(TLS_TX/TLS_RX)`. Oracle's `tlshd` daemon (in `ktls-utils`) ships the closest precedent: it performs this exact handshake-then-install-keys pattern for NFS-over-TLS, receiving socket FDs via netlink from kernel consumers.

The **architecture decision** that must be resolved before the DESIGN wave (Finding 1, Risk 1): Architecture A (in-place kTLS — whitepaper model, no proxy, transient sockmap block for the race window) vs Architecture B (sockmap/sk_msg proxy redirect — the approach Istio Ambient and Cilium actually ship). Architecture A has higher performance ceiling (no per-packet copy after handshake) but requires solving the fd-interception + race-window problem for all workload types including VMs. Architecture B is proven and available today but adds per-connection proxy overhead. A hybrid (Architecture A with transient sockmap block) is recommended but is novel territory.

Three additional design-shaping findings: (1) TLS 1.3 RX kTLS requires kernel ≥6.0. Overdrive ships its own stripped-down immutable appliance OS and **pins the kernel** (decided 2026-06-04, floor = **6.6 LTS**, latest LTS we ship), so kernel availability is a fixed target Overdrive controls — full in-kernel TLS 1.3 TX+RX *and* the kernel-side handshake API (`CONFIG_NET_HANDSHAKE`, ≥6.5) are both guaranteed present; there is no diverse-operator-kernel matrix to design fallbacks for. (2) TLS 1.3 KeyUpdate for mid-connection rekey landed only post-6.6, so on the 6.6 pinned kernel SVID rotation on long-lived connections still requires connection teardown and reconnect — and the 6.18 pin does **not** remove this — in-place rekey is blocked on the userspace rustls→kTLS bridge gaining KeyUpdate support ([rustls/ktls#59](https://github.com/rustls/ktls/issues/59) / [#62](https://github.com/rustls/ktls/pull/62), tracked in #229), so v1 uses teardown+reconnect until that lands upstream. (3) Cilium's "mutual auth" (the most visible prior art) does NOT use kTLS for encryption: it does an OOB mTLS handshake between agents, discards the keys, and relies on WireGuard/IPSec for encryption — their own open CFP #26480 acknowledges using session keys for kTLS as unimplemented future work.

## Research Methodology

**Search Strategy**: Primary sources first: kernel.org/networking/tls.html, kernel.org/networking/tls-handshake.html, docs.rs/aya-ebpf, docs.rs/ktls, docs.rs/ktls-core, docs.ebpf.io for BPF_PROG_TYPE_SOCK_OPS. Companion document (`aya-rs-usage-comprehensive-research.md` §B.3, Gap K-2; `transparent-encryption-comprehensive-research.md` §Finding 8-9) pre-read to avoid re-derivation. Cilium prior art via docs.cilium.io and Cilium blog. LPC paper (ktls_bpf) for combined kTLS+BPF architecture. Whitepaper §7/§8 as the SSOT design intent anchor.

**Source Selection**: Types: official (kernel.org, docs.kernel.org), technical_docs (docs.rs, docs.ebpf.io, docs.cilium.io), open_source (aya-rs.dev, github.com/aya-rs), academic-adjacent (LPC/Netdev conference papers). Minimum reputation 0.8 (high/medium-high). Verification: ≥2 independent sources per major mechanism claim; kernel source cross-referenced with aya-ebpf bindings.

**Quality Standards**: Target 2-3 sources per major claim (1 authoritative minimum where multi-source not possible). Every mechanism claim traceable to kernel docs or source code, not blog paraphrase.

## Findings

### Finding 1 — The transparent-handoff feasibility crux: kTLS without an on-socket handshake

**Verdict**: The "install keys and step out, application stays plaintext" model is **architecturally achievable** on mainline Linux, but requires a specific division of labour that is NOT what the whitepaper §7/§8 describes in its simplified flow diagram. The actual architecture is more constrained than stated, and NO production system currently ships this model for general workloads. This finding is the most important design input for the DISCUSS wave.

**Sub-question 1 — Can kTLS keys be installed on a socket that never carried a TLS handshake?**

**Evidence**: The kernel kTLS documentation states: "After the TLS handshake is complete, we have all the parameters required to move the data-path to the kernel." The documentation describes the setsockopt sequence as: (1) set TCP_ULP "tls" on an established TCP socket, (2) populate `tls_crypto_info` (key, iv, salt, rec_seq) and call `setsockopt(SOL_TLS, TLS_TX/TLS_RX)`. The kernel itself does NOT validate whether the key material came from a handshake on THIS socket — it accepts whatever `tls_crypto_info` is provided. The critical fields are `key` (16 or 32 bytes for AES-GCM), `iv` (8 bytes explicit IV), `salt` (4 bytes), and `rec_seq` (8 bytes, the record sequence number).

**Source**: [Linux Kernel TLS documentation](https://docs.kernel.org/networking/tls.html) — kernel.org, accessed 2026-06-04. **Reputation**: High (1.0). **Verification**: [ktls-core crate docs](https://docs.rs/ktls-core/latest/ktls_core/) confirms `setup_tls_params()` and `setup_ulp()` as the two-step installation API with no requirement for a prior handshake on the same socket.

**Confidence**: High — the kernel API accepts external key material; this is its explicit design point (keeping handshake in userspace).

**Sub-question 2 — How do both agents agree on secrets for a connection whose data socket they don't own the handshake on?**

**Evidence**: The kernel's in-kernel TLS handshake API (`Documentation/networking/tls-handshake.html`) describes exactly this: a userspace **handshake agent** (daemon) receives an open socket FD via netlink, performs the TLS handshake using that socket as the I/O channel, and then installs `TLS_TX`/`TLS_RX` keys via `setsockopt`. The **application's data socket IS the handshake socket** in this model. The agent intercepts the socket before data flows.

For Overdrive's model (workload calls `connect()` → sockops intercepts → node agent does handshake → kTLS installed → workload's data flows encrypted), the sequence requires:
- The **data socket** is the same socket on which the handshake occurs
- The node agent must intercept the socket before the workload's first `write()`
- sockops CAN signal that a new connection has been established, but it CANNOT pause the workload's first `write()` long enough for an asynchronous TLS handshake to complete before data flows

The fundamental constraint: there is a **race window** between `ACTIVE_ESTABLISHED_CB` firing (sockops notification) and the completion of the node agent's TLS handshake. During this window, any application `write()` will send plaintext.

**Source**: [Linux Kernel in-kernel TLS handshake API](https://docs.kernel.org/networking/tls-handshake.html) — kernel.org, accessed 2026-06-04. **Reputation**: High (1.0). 

**Sub-question 3 — What can sockops actually do? What is BPF-side vs userspace-side?**

**Evidence**: The `SockOpsContext` in `aya_ebpf` exposes: `op()`, `family()`, `remote_ip4/6()`, `local_ip4/6()`, `local_port()`, `remote_port()`, `cb_flags()`, `set_cb_flags()` (via `bpf_sock_ops_cb_flags_set`), `set_reply()`, and helper functions including **`bpf_setsockopt`** and `bpf_sock_map_update`. Crucially, `bpf_setsockopt` IS available from sock_ops context. This means a sock_ops program CAN in principle call `setsockopt(SOL_TCP, TCP_ULP, "tls")` on the socket. However, it CANNOT:
- Perform a TLS handshake (no network I/O in BPF)
- Generate or hold key material (no crypto in BPF)
- Block the socket's data path until a userspace agent completes
- Install `TLS_TX`/`TLS_RX` keys directly (requires the key material that only the rustls handshake can produce)

**Source**: [aya-ebpf SockOpsContext source](https://docs.rs/aya-ebpf/latest/src/aya_ebpf/programs/sock_ops.rs.html) — docs.rs, accessed 2026-06-04. **Reputation**: High (1.0). **Verification**: [docs.ebpf.io BPF_PROG_TYPE_SOCK_OPS](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/) confirms available helpers including bpf_setsockopt.

**Sub-question 4 — The alternative architecture: sockmap redirect to per-connection proxy socket**

**Evidence**: The practical architecture used by Istio Ambient ztunnel and Cilium's mutual auth approach is: **redirect the application's connection to a local per-node proxy socket** (via iptables or sockmap), where the proxy performs the TLS handshake on a new socket, and then proxies data. This avoids the handshake race entirely because the proxy owns the TLS socket. The application's plaintext connection goes to `localhost:PORT`, the proxy intercepts, does TLS to the remote, and forwards data. Cilium's current mTLS (1.20+) uses ztunnel with iptables redirection to a per-node proxy, NOT sockops+kTLS with in-place key installation. 

The "install keys and step out" model requires either (a) blocking the application socket until the handshake completes (not possible via BPF alone without a kernel mechanism) OR (b) using `SO_RCVBUF`/socket buffers to hold early data (fragile) OR (c) using a **separate control-channel handshake** between the two node agents keyed on 5-tuple, where both agents independently derive the same keys and install them on their respective data sockets before data flows — this requires strict coordination protocol and is novel, unshipped territory.

**Source**: [Cilium ztunnel transparent encryption docs](https://docs.cilium.io/en/latest/security/network/encryption-ztunnel/) — docs.cilium.io, accessed 2026-06-04. **Reputation**: High (1.0). **Verification**: [Cilium Native mTLS blog 2026-03-23](https://cilium.io/blog/2026/03/23/native-mtls-cilium/) confirms ztunnel (proxy-based) is Cilium's current mTLS approach; NOT sockops+kTLS key installation.

**Overall Feasibility Assessment**:

The whitepaper's stated flow ("sockops intercepts → rustls handshake → keys installed → node agent exits data path") is **feasible in theory** given the kernel API capabilities, but requires a critical architectural decision that the whitepaper does not fully specify: **how is the application socket paused between `connect()` completing and the rustls handshake completing?** Three implementation paths exist with different complexity/risk profiles:

1. **Sockmap redirect + TLS proxy + key re-installation** — redirect via sock_msg to a local agent socket that does TLS, then either proxy all data (simpler but adds latency) or install keys on original socket and un-redirect (complex, no known shipping precedent).
2. **tlshd-style daemon** — use the kernel's `tls_handshake_args` API where a listening daemon on a netlink socket receives the application socket, performs TLS on it directly (the app socket IS the TLS socket), installs keys, and returns control. This IS what Oracle's `ktls-utils` implements. But the workload must cooperate (it must tolerate the first `write()` being held).
3. **BPF_F_ESTABLISHED delay + separate agent protocol** — two-node-agent out-of-band key agreement keyed by 5-tuple; no shipping precedent.

**Path 2 (tlshd-style)** is the most credible approach and maps most closely to the whitepaper's intent. Oracle ships `ktls-utils` (github.com/oracle/ktls-utils) as a `tlshd` daemon for NFS-over-TLS using exactly this model.

**Confidence**: Medium — kernel API capabilities are High confidence; the "no race window" guarantee and the exact "pause socket until handshake" mechanism have Low confidence without a concrete implementation reference for general workloads.

**Analysis**: The whitepaper's description is an accurate high-level summary of a feasible architecture, but glosses over the hardest implementation challenge: ensuring the application's first plaintext write doesn't escape before the TLS session is established. The DISCUSS wave must choose among the three paths above and nail down the data-socket-pause mechanism.

### Finding 2 — kTLS kernel API surface (ULP "tls", TLS_TX/TLS_RX setsockopt, cipher + kernel-version matrix)

**Summary**: kTLS is the Linux kernel's TLS record-layer implementation. After a TLS handshake completes in userspace (rustls or any other library), the application installs the session keys into the kernel via `setsockopt`; thereafter the kernel encrypts/decrypts all `write()`/`read()` calls transparently.

**API Sequence** (authoritative from kernel.org documentation):

```c
// Step 1: Establish TCP connection (normal connect()/accept())
// Step 2: Enable TLS ULP on the socket
setsockopt(sock, SOL_TCP, TCP_ULP, "tls", sizeof("tls"));

// Step 3: Install TX keys
struct tls12_crypto_info_aes_gcm_128 crypto_info = {
    .info.version    = TLS_1_3_VERSION,  // or TLS_1_2_VERSION
    .info.cipher_type = TLS_CIPHER_AES_GCM_128,
    .iv      = { /* 8 bytes */ },
    .key     = { /* 16 bytes */ },
    .salt    = { /* 4 bytes */ },
    .rec_seq = { /* 8 bytes, typically starts at 0 for TLS 1.3 */ },
};
setsockopt(sock, SOL_TLS, TLS_TX, &crypto_info, sizeof(crypto_info));

// Step 4: Install RX keys (same structure, different values)
setsockopt(sock, SOL_TLS, TLS_RX, &crypto_info_rx, sizeof(crypto_info_rx));
```

**Cipher-Specific Struct Sizes**:

| Cipher | Struct | Key | IV | Salt | rec_seq |
|---|---|---|---|---|---|
| AES-GCM-128 | `tls12_crypto_info_aes_gcm_128` | 16B | 8B | 4B | 8B |
| AES-GCM-256 | `tls12_crypto_info_aes_gcm_256` | 32B | 8B | 4B | 8B |
| ChaCha20-Poly1305 | `tls12_crypto_info_chacha20_poly1305` | 32B | 12B | 0B | 8B |

**Kernel Version Feature Matrix** (compiled from kernel.org docs + kTLS community sources):

| Feature | Kernel version | Notes |
|---|---|---|
| kTLS TX (TLS 1.2 AES-GCM-128) | 4.13 | Initial merge |
| kTLS RX (TLS 1.2) | 4.17 | Added receive path |
| TLS 1.3 TX | 5.1 | AES-GCM-128 + AES-GCM-256 |
| AES-GCM-256 | 5.1 | TX only initially |
| ChaCha20-Poly1305 TX | 5.11 | |
| TLS 1.3 RX | 6.0 | Crucial: earlier kernels cannot RX-decrypt TLS 1.3 in-kernel |
| NIC Tx offload (ConnectX-6 Dx) | 5.3 | NVIDIA Mellanox hardware |
| NIC Rx offload (ConnectX-6 Dx) | 5.9 | NVIDIA Mellanox hardware |
| Kernel-side handshake API (`CONFIG_NET_HANDSHAKE`) | 6.5 | Oracle tlshd / tlsca consumers |

**Project floor implications**: Overdrive ships its own stripped-down immutable appliance OS and **pins the kernel** — kernel version is a target Overdrive controls, not a property of the operator's host. The pinned floor is **6.6 LTS** (latest LTS; decided 2026-06-04 — see "Project decision" note below). Because TLS 1.3 RX in-kernel requires ≥6.0, the 6.6 pin means **full in-kernel TLS 1.3 TX+RX is guaranteed present**, and because 6.6 ≥ 6.5 the kernel-side handshake API (`CONFIG_NET_HANDSHAKE`, tlshd-style) is *also* guaranteed present — so the tlshd-style kernel-driven handshake path (Finding 5) is on the table without a version caveat. The only feature still absent at 6.6 is TLS 1.3 KeyUpdate (post-6.6); see Risk 3.

> **Project decision (2026-06-04)**: Overdrive does not target a diverse operator-kernel matrix. It ships an immutable appliance OS (Image Factory, whitepaper §23) with a **pinned kernel — currently 6.6 LTS, the latest LTS** — and can advance that pin at will since it owns the OS image. The old 5.10/5.15-era kernel matrix premise (supporting whatever kernel an operator runs) does not apply; the "matrix" collapses to the pinned kernel plus `bpf-next` as an early-warning soft-fail. The SSOT lives in `.claude/rules/testing.md` § "Kernel matrix" and `docs/whitepaper.md` § 22; per `testing.md` ("Dropping a kernel requires an ADR") the change is recorded in an ADR via the architect agent. This research is written against the 6.6 pinned floor.

**Key Security Note**: The kernel documentation explicitly states: "The kernel will not check for key/nonce reuse." The application is responsible for key lifetime management, re-keying at the TLS 1.3 confidentiality limit (`CipherSuiteCommon::confidentiality_limit` in rustls: ~2^23 records for AES-GCM).

**Source**: [Linux Kernel TLS documentation](https://docs.kernel.org/networking/tls.html) — kernel.org, accessed 2026-06-04. **Reputation**: High (1.0).
**Verification**: [ktls-core crate documentation](https://docs.rs/ktls-core/latest/ktls_core/) (docs.rs — High 1.0); [NGINX kTLS blog F5](https://www.f5.com/company/blog/nginx/improving-nginx-performance-with-kernel-tls) (industry/F5 — Medium-High 0.8).
**Confidence**: High for kernel version matrix 4.13–5.11, High for cipher structs; Medium for exact TLS 1.3 RX kernel 6.0 date (from community sources, not directly cited in kernel.org doc).

### Finding 3 — sockops program surface: hooks, what it can/cannot do, aya `SockOps` typed API + `BPF_SOCK_OPS_*_CB`

**Summary**: sockops programs attach to cgroup v2 (NOT to network interfaces), and fire at TCP lifecycle events. They can read connection metadata, populate sockmaps, set TCP parameters, and — via `bpf_setsockopt` — configure socket options. They cannot perform I/O, block a socket's data path, or generate cryptographic key material.

**Attachment**: `aya::programs::SockOps::attach(cgroup_fd: BorrowedFd)` — cgroup v2 path only.

**ELF program section**: `sock_ops/<name>` (emitted by `#[sock_ops]` macro).

**Complete BPF_SOCK_OPS_* constants** available in `aya_ebpf::bindings` (from docs.rs, accessed 2026-06-04):

*Operations (fired as events)*:
- `BPF_SOCK_OPS_VOID` (0) — unused
- `BPF_SOCK_OPS_TIMEOUT_INIT` — set initial retransmission timeout
- `BPF_SOCK_OPS_RWND_INIT` — set initial receive window
- `BPF_SOCK_OPS_TCP_CONNECT_CB` (= 3) — **client-side: SYN sent** (connection initiation)
- `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB` — **client-side: connection established** (SYN-ACK received, socket is ESTABLISHED); primary hook for active-side kTLS intercept
- `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB` — **server-side: connection established** (three-way handshake complete); primary hook for passive-side kTLS intercept
- `BPF_SOCK_OPS_NEEDS_ECN` — query whether ECN is supported
- `BPF_SOCK_OPS_BASE_RTT` — set base RTT for BBR
- `BPF_SOCK_OPS_RTO_CB` — retransmission timeout event
- `BPF_SOCK_OPS_RETRANS_CB` — retransmission event
- `BPF_SOCK_OPS_STATE_CB` — TCP state machine transition
- `BPF_SOCK_OPS_TCP_LISTEN_CB` — listening socket event
- `BPF_SOCK_OPS_RTT_CB` — RTT sample collected
- `BPF_SOCK_OPS_PARSE_HDR_OPT_CB`, `BPF_SOCK_OPS_HDR_OPT_LEN_CB`, `BPF_SOCK_OPS_WRITE_HDR_OPT_CB` — custom TCP header option parsing/writing

*Callback flags* (bitmask, set via `bpf_sock_ops_cb_flags_set` to enable additional callbacks):
- `BPF_SOCK_OPS_RTT_CB_FLAG`
- `BPF_SOCK_OPS_RETRANS_CB_FLAG`
- `BPF_SOCK_OPS_STATE_CB_FLAG`
- `BPF_SOCK_OPS_WRITE_HDR_OPT_CB_FLAG`
- `BPF_SOCK_OPS_PARSE_HDR_OPT_CB_FLAG`
- `BPF_SOCK_OPS_PARSE_UNKNOWN_HDR_OPT_CB_FLAG`
- `BPF_SOCK_OPS_RTO_CB_FLAG`
- `BPF_SOCK_OPS_ALL_CB_FLAGS` — enable all optional callbacks

**SockOpsContext fields** (aya-ebpf `SockOpsContext` API, from source):
- `op()` → `u32` — which event fired
- `family()` → `u32` — `AF_INET` (2) or `AF_INET6` (10)
- `remote_ip4()` / `local_ip4()` → `u32` (network byte order)
- `remote_ip6()` / `local_ip6()` → `[u32; 4]`
- `remote_port()` / `local_port()` → `u32`
- `cb_flags()` → `u32` — currently active callback flags
- `set_cb_flags(flags)` → `Result<(), i64>` — wrapper for `bpf_sock_ops_cb_flags_set`
- `set_reply(reply: u32)` — for replying to `TIMEOUT_INIT`, `RWND_INIT`, etc.
- `arg(n: usize)` — access `args[n]` from the context

**Available helper functions** in `sock_ops` context:
- `bpf_sock_map_update` / `bpf_sock_hash_update` — **add the current socket to a SOCKMAP or SOCKHASH** (this is how the connection is handed off to sk_msg/sk_skb programs)
- `bpf_setsockopt` / `bpf_getsockopt` — **set/get socket options** including `TCP_ULP`
- `bpf_sk_lookup_tcp` / `bpf_sk_lookup_udp` — look up other sockets
- `bpf_tcp_sock`, `bpf_get_listener_sock`
- Timer/header helpers: `bpf_load_hdr_opt`, `bpf_store_hdr_opt`, `bpf_reserve_hdr_opt`
- `bpf_skc_to_tcp_sock` — cast to full TCP socket for additional fields

**What sockops CAN do** (relevant to mTLS):
1. At `ACTIVE_ESTABLISHED_CB`/`PASSIVE_ESTABLISHED_CB`, read the 5-tuple (src/dst IP + port)
2. Populate a SOCKMAP/SOCKHASH map with the socket for later sk_msg redirect
3. Notify a userspace process via perf ring buffer or BPF map (via a pinned map that userspace polls)
4. Call `bpf_setsockopt(ctx, SOL_TCP, TCP_ULP, "tls", ...)` to set the ULP tag on the socket
5. Set `BPF_SOCK_OPS_STATE_CB_FLAG` to receive future state change callbacks

**What sockops CANNOT do** (key constraints for mTLS design):
- Cannot perform a TLS handshake (no I/O helpers, no network calls)
- Cannot block the socket's data path (returns immediately; no blocking/sleeping)
- Cannot install `TLS_TX`/`TLS_RX` crypto info (requires the key material that only a TLS handshake produces)
- Cannot guarantee the application's first `write()` is held until a TLS session is established
- Cannot call `bpf_setsockopt(SOL_TLS, TLS_TX, ...)` directly (the crypto info must come from somewhere — and that somewhere is the rustls handshake result, which BPF does not have access to)

**Implication for mTLS architecture**: The sockops program's role is **detection and signalling**, not the handshake itself. It fires at connection establishment, captures the 5-tuple, writes to a map (triggering a waiting userspace agent via `bpf_ringbuf_output` or a poll on a shared map), and optionally begins sockmap redirect to intercept data. The userspace agent (node agent with rustls) then performs the actual handshake and installs keys. The BPF program itself cannot do the key installation.

**Source**: [aya-ebpf SockOpsContext source](https://docs.rs/aya-ebpf/latest/src/aya_ebpf/programs/sock_ops.rs.html) — docs.rs, accessed 2026-06-04. **Reputation**: High (1.0).
**Verification**: [docs.ebpf.io BPF_PROG_TYPE_SOCK_OPS](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/) — cross-confirms operation codes, context fields, and available helpers. [aya-ebpf bindings constants](https://docs.rs/aya-ebpf/latest/aya_ebpf/all.html) — enumerates all BPF_SOCK_OPS_* constants.
**Confidence**: High (direct source code and authoritative docs).

### Finding 4 — sockmap / sk_msg redirect vs in-place kTLS: the two transparent-interception architectures

**Summary**: Two distinct BPF-based architectures exist for transparent per-connection interception. Understanding their differences is load-bearing for the DISCUSS wave architectural decision.

#### Architecture A — In-place kTLS (the whitepaper model)

The application socket receives kTLS keys installed via `setsockopt(TLS_TX/TLS_RX)`. All encrypt/decrypt happens in-kernel on the application's own socket. No proxying; no copy overhead. The node agent performs the TLS handshake using the application socket as the I/O channel, installs keys, and exits the data path.

**How traffic reaches the node agent**: sockops detects `ACTIVE_ESTABLISHED_CB`, signals the node agent (via a BPF ringbuf or perf event on a map), the node agent attaches to the socket via a file descriptor obtained through `pidfd_getfd()` or via `SO_PEERCRED` + `SCM_RIGHTS` (socket fd passing over unix socket). The agent performs rustls handshake I/O using the application's TCP socket directly, then installs kTLS keys.

**The race problem**: Between the moment `ACTIVE_ESTABLISHED_CB` fires and the moment kTLS keys are installed, the application can `write()` plaintext data. In a busy workload, this window can be tens of milliseconds (time for: BPF event → agent wakeup → pidfd_getfd → rustls hello → remote response → key derivation → setsockopt). Mitigation options:
- Use `BPF_SOCK_OPS_STATE_CB_FLAG` to detect if data is sent before kTLS is armed, and reset/close the connection
- Insert a sockmap redirect temporarily until keys are ready (Architecture B hybrid)
- Rely on application-level framing: the node agent sends a TLS ClientHello *before* the application can write (requires agent to intercept the socket's fd before the `connect()` syscall returns — possible via a BPF tracepoint + `pidfd_getfd`)

**Precedent**: Oracle `tlshd` (github.com/oracle/ktls-utils) uses this exact architecture for NFS-over-TLS and NVMe-TCP. The kernel consumer (`sunrpc`, `nvme-tcp`) calls the handshake API, which sends the socket FD to the tlshd daemon; tlshd performs the handshake (reading/writing on the FD), installs kTLS keys, and returns. NFS/NVMe control this timing because they initiate the handshake request themselves. **The challenge for Overdrive is that the workload (not the agent) calls `connect()`.**

#### Architecture B — sockmap/sk_msg redirect (the proxy model)

sockops at `ACTIVE_ESTABLISHED_CB` calls `bpf_sock_map_update()` to put the socket in a SOCKMAP. A separate `sk_msg` verdict program intercepts every `sendmsg()` on that socket. `sk_msg` can call `bpf_msg_redirect_hash()` to redirect the message to a different socket owned by the local proxy (the node agent). The agent receives the plaintext, does TLS to the remote, and forwards.

**Pros**: No race window on the send side (sk_msg intercepts the first write before any data leaves); works on the pinned 6.6 kernel and does not depend on in-kernel TLS 1.3 RX since the agent stays in the data path. **Cons**: per-packet copy overhead; proxy memory overhead; latency addition for every payload segment; the agent remains in the data path (not just key installation).

#### Sockmap + kTLS combined

LWN article (2018) documents that sockmap and kTLS were previously **mutually exclusive** at the ULP layer. This was fixed in Linux ~4.20 by the "generic sk_msg layer" patchset, which unified sockmap and kTLS over a common `sk_msg` data structure. As of kernel 4.20+, a socket can have BOTH kTLS (ULP installed) AND be in a SOCKMAP. This enables a hybrid: use sockmap to intercept data UNTIL kTLS is armed, then remove from sockmap and let kTLS handle it natively.

**kTLS + sockmap interaction nuances** (from kernel selftests `sockmap_ktls.c`):
- kTLS must typically be installed BEFORE the socket is put in the sockmap (ordering matters)
- kTLS RX + sockmap redirect (`BPF_F_INGRESS`) has known edge cases in teardown and write_space
- The kernel selftests cover this combination and run in CI, so it is a supported path

**Source**: [LWN.net sockmap integration for kTLS](https://lwn.net/Articles/768371/) — lwn.net, accessed 2026-06-04. **Reputation**: Medium-High (0.8).
**Verification**: [kernel.org BPF sockmap documentation](https://docs.kernel.org/bpf/map_sockmap.html) — High (1.0). [Oracle ktls-utils](https://github.com/oracle/ktls-utils) for Architecture A precedent — github.com, Medium-High (0.8).
**Confidence**: High for Architecture B mechanics; Medium for Architecture A "race window" severity (depends on workload timing); High for the sockmap+kTLS compatibility fix being in kernel 4.20+.

### Finding 5 — Installing rustls-derived session keys into kTLS from userspace (the key-export path)

**Summary**: rustls exposes a `secret_extraction` feature that allows the application to extract TLS session keys after a handshake completes. The `ktls` crate (now under the rustls org) and the lower-level `ktls-core` crate bridge these extracted secrets to the kernel `setsockopt(TLS_TX/TLS_RX)` call. This is a well-defined, tested Rust path.

**rustls secret extraction API** (docs.rs/rustls/latest, accessed 2026-06-04):

Enable via Cargo feature: `rustls = { features = ["secret_extraction"] }` (or `enable_secret_extraction` on `ClientConfig`/`ServerConfig`).

After the handshake completes, call:
```rust
// After handshake on a Connection:
let extracted: ExtractedSecrets = conn.dangerous_extract_secrets()?;
// ExtractedSecrets contains:
//   tx: (u64 seq, ConnectionTrafficSecrets)
//   rx: (u64 seq, ConnectionTrafficSecrets)
```

`ConnectionTrafficSecrets` enum variants (all contain `key: AeadKey` and `iv: Iv`):
- `Aes128Gcm { key, iv }` — maps to `TLS_CIPHER_AES_GCM_128`
- `Aes256Gcm { key, iv }` — maps to `TLS_CIPHER_AES_GCM_256` 
- `Chacha20Poly1305 { key, iv }` — maps to `TLS_CIPHER_CHACHA20_POLY1305`

The `KernelConnection` type (via `dangerous_into_kernel_connection()`) is for cases where the application wants to continue managing key updates after kTLS is installed; it requires `enable_extract_secrets` on the config.

**Key-to-kTLS mapping** (from `ktls` crate `ffi.rs`, docs.rs, accessed 2026-06-04):

```
ConnectionTrafficSecrets::Aes128Gcm { key, iv }
  → tls12_crypto_info_aes_gcm_128 {
      .info.version    = TLS_1_3_VERSION (or 1_2),
      .info.cipher_type = TLS_CIPHER_AES_GCM_128,
      .key  = key.as_ref()[..16],
      .salt = iv.as_ref()[..4],    // first 4 bytes of the rustls IV = implicit nonce (salt)
      .iv   = iv.as_ref()[4..],   // last 8 bytes = explicit IV part
      .rec_seq = seq.to_be_bytes(), // u64 sequence counter from ExtractedSecrets.tx.0
    }

ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv }
  → tls12_crypto_info_chacha20_poly1305 {
      .key  = key.as_ref()[..32],
      .iv   = iv.as_ref()[..12],  // full 12 bytes, no split
      .salt = [],
      .rec_seq = seq.to_be_bytes(),
    }
```

**TLS 1.2 vs TLS 1.3 note**: Both versions use the **same kernel struct** (`tls12_crypto_info_*`). The distinction is only in `.info.version` = `TLS_1_2_VERSION_NUMBER` vs `TLS_1_3_VERSION_NUMBER`. The ktls crate handles this transparently. For TLS 1.3 specifically, the record sequence starts at 0 after the handshake (first record sent is record 0), so `rec_seq = 0` in `to_be_bytes()` is correct for a fresh session.

**ktls crate** (version 6.0.2, MIT/Apache-2.0, owners: ctz/fasterthanlime/rustls team):
- `config_ktls_client(stream)` — takes a `TlsStream<TcpStream>`, extracts secrets, installs kTLS, returns a kTLS-accelerated stream
- `config_ktls_server(stream)` — same for server side
- Uses `CorkStream` to drain any buffered TLS records from the rustls buffer before handing off to kTLS (important: rustls may have received and buffered data in userspace before kTLS takes over — the CorkStream handles this transition boundary)

**ktls-core crate** (version 0.0.5, lower-level, TLS-library-agnostic):
- `setup_ulp(fd)` — sets `TCP_ULP "tls"` on the socket
- `setup_tls_params(fd, tx_params, rx_params)` — calls `setsockopt(TLS_TX)` and `setsockopt(TLS_RX)`
- `Compatibilities::probe_kernel()` — checks which cipher suites are available on the running kernel
- Tested against kernels 5.4.181 LTS through mainline; recommends ≥6.6 LTS for "better support"

**Key limitation**: Both `ktls` and `ktls-core` assume the handshake occurred on the **same** socket that kTLS will be installed on. The `CorkStream` pattern specifically drains rustls's internal receive buffer before handing to kTLS, because rustls may have already decrypted and buffered incoming data that the kernel hasn't seen. For Overdrive's node-agent-intercepts-application-socket model, this means the agent must drain rustls's buffer after the handshake and before kTLS installation, and ensure no decrypted data is in flight.

**Source**: [rustls kernel module docs](https://docs.rs/rustls/latest/rustls/kernel/index.html) — docs.rs, accessed 2026-06-04. **Reputation**: High (1.0).
**Verification**: [ktls crate source ffi.rs](https://docs.rs/ktls/latest/src/ktls/ffi.rs.html) — docs.rs, High (1.0). [ktls-core documentation](https://docs.rs/ktls-core/latest/ktls_core/) — docs.rs, High (1.0).
**Confidence**: High (all from primary source code and official crate docs).

### Finding 6 — IDENTITY_MAP consumption: tagging flows with SPIFFE identity at the socket layer

**Summary**: The IDENTITY_MAP written by IdentityMgr (roadmap 2.13 / GH #35) provides the mapping from 5-tuple or allocation-specific metadata to SPIFFE ID. The sockops layer consumes this map to determine the identity pair for each connection. This finding documents the likely design shape; specific IDENTITY_MAP schema is forward-looking (IdentityMgr not yet implemented).

**Mechanism options** for per-socket identity tagging in BPF:

**Option A: Per-socket storage (`BPF_MAP_TYPE_SK_STORAGE`)**

`BPF_MAP_TYPE_SK_STORAGE` (merged kernel 5.2) stores per-socket data co-located with the socket struct itself, keyed by the socket pointer. The sockops program can call `bpf_sk_storage_get()` with `BPF_LOCAL_STORAGE_GET_F_CREATE` to create or retrieve per-socket metadata. At `ACTIVE_ESTABLISHED_CB` / `PASSIVE_ESTABLISHED_CB`, the sockops program can:
1. Derive a lookup key (source IP or cgroup + allocation metadata) from the 5-tuple
2. Look up the IDENTITY_MAP (a `HASH` or `LPM_TRIE` map keyed by IP/cgroup) to resolve the local SPIFFE ID
3. Store the resolved SPIFFE ID in per-socket storage (`SK_STORAGE`) tagged to this socket
4. The node agent reads the SPIFFE ID from `SK_STORAGE` via the socket FD to determine which SVID to present during the handshake

**Option B: 5-tuple keyed connection map (HASH or SOCKHASH)**

A `BPF_MAP_TYPE_HASH` keyed by `{src_ip, src_port, dst_ip, dst_port, proto}` (the 5-tuple) maps connections to identity metadata. The sockops program writes the entry at connection establishment; the node agent and downstream policy programs read it.

**IDENTITY_MAP likely schema** (design input for DISCUSS wave):
- **Key**: source IP address (or cgroup ID for local allocation identification)  
- **Value**: `{spiffe_id_bytes: [u8; 255], ttl_seconds: u32, allocation_id: u64}`
- **Written by**: IdentityMgr (roadmap 2.13 / GH #35), which maps allocation IP assignments to SPIFFE IDs
- **Read by**: sockops at connection establishment (to resolve which SVID to use), BPF LSM (for access control), XDP (for identity-based packet filtering)

**Design constraint**: The IDENTITY_MAP must be writable before a workload's first `connect()` call to avoid a lookup miss. IdentityMgr must write the map atomically at workload start, before the workload begins accepting connections. This is the cross-dependency to GH #35.

**BPF_MAP_TYPE_SK_STORAGE advantages over 5-tuple map**:
- Stored at the socket, so GC is automatic (freed when socket closes) — no stale entries
- Faster access: no hash computation on lookup, just pointer dereference
- Survives source port reuse races that plague 5-tuple maps
- Available from `sk_msg`, `sk_skb`, and `sockops` contexts

**Source**: [Linux Kernel BPF SK_STORAGE documentation](https://docs.kernel.org/bpf/map_sk_storage.html) — kernel.org, accessed 2026-06-04. **Reputation**: High (1.0).
**Verification**: [eBPF docs bpf_sk_storage_get](https://docs.ebpf.io/linux/helper-function/bpf_sk_storage_get/) — docs.ebpf.io, High (1.0).
**Confidence**: High for the BPF_MAP_TYPE_SK_STORAGE mechanism (well-documented, kernel 5.2+, well below the pinned 6.6 floor — guaranteed present). Medium for the exact IDENTITY_MAP schema (forward-looking design input; IdentityMgr not yet implemented).

### Finding 7 — NIC kTLS offload (Tx/Rx) maturity and the virtio/veth test reality

**Summary**: NIC kTLS hardware offload is available on specific NICs but is transparent to software — the software path is the default and identical API. The companion document (`docs/research/transparent-encryption-comprehensive-research.md` §Finding 9) established the NIC landscape already. This finding focuses on what this means for the Overdrive development and test environment.

**Software kTLS is the default and always works**: When `setsockopt(TLS_TX/TLS_RX)` succeeds, the kernel's software kTLS path handles encrypt/decrypt. NIC offload is a transparent acceleration layer: if the NIC supports it and the driver exposes the offload capability via `ethtool -k <iface>`, the kernel uses it; otherwise software path is used. The application cannot distinguish. Performance difference is ~10–15% CPU savings on NIC-offloaded flows (AES-GCM on ConnectX-6 Dx).

**Supported NICs** (per companion doc Finding 9 + kernel-internals.org):
- NVIDIA Mellanox ConnectX-6 Dx: TX from kernel 5.3, RX from kernel 5.9; AES-GCM ciphers only
- Intel E810: limited support
- Some Chelsio NICs: limited support
- **Not supported**: virtio-net, veth, lo — these fall back to software kTLS

**Lima/QEMU test reality**: The project's Lima dev VM uses virtio-net and veth interfaces. These do NOT support NIC kTLS offload. All Tier 3 integration tests run software kTLS. This is correct behavior: the software kTLS path is what is being validated, and the NIC offload path is transparent on hardware that supports it. The test harness does not need to test NIC offload explicitly.

**ethtool verification**: `ethtool -k <iface> | grep tls` shows `tls-hw-tx-offload: off [fixed]` on virtio-net (expected). On ConnectX-6 Dx with the appropriate firmware, this shows `on`.

**TLS 1.3 NIC offload caveat** (from companion doc): NVIDIA forum reports (single post, unconfirmed) suggest TLS 1.3 `TLS_AES_128_GCM_SHA256` may NOT offload on ConnectX-6 Dx as of 2023 firmware. AES-GCM TLS 1.2 offload is confirmed. For TLS 1.3, software fallback is expected on current ConnectX-6 Dx. This is a known limitation; track firmware releases.

**Source**: See companion document [transparent-encryption-comprehensive-research.md §Finding 9](docs/research/transparent-encryption-comprehensive-research.md) which cites [NVIDIA Mellanox kTLS Offloads documentation](https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads) — High (1.0). [kernel-internals.org kTLS](https://kernel-internals.org/net/ktls/) — Medium-High (0.8).
**Confidence**: High for software kTLS path (no HW dependency); Medium for TLS 1.3 NIC offload status (firmware-version dependent, single-source caveat).

### Finding 8 — Prior art: who actually ships sockops+kTLS transparent mTLS (and who abandoned it)

**Summary**: No production system has shipped the full "sockops detects connection → agent performs TLS handshake on application socket → installs kTLS keys → exits data path, application stays unaware" model for general workloads. Partial implementations exist. Cilium's approach is instructive as the closest analogue — and reveals the deep difficulty.

#### Cilium Mutual Authentication (shipping, but NOT sockops+kTLS)

Cilium 1.14+ (beta) / 1.20+ (ztunnel): Cilium's mutual authentication uses an **out-of-band mTLS handshake between cilium-agents** (not between workload sockets). When Pod A sends to Pod B, the first packet is **dropped** by the BPF dataplane, triggering a mTLS handshake between the two node agents. After the handshake, the session keys are **discarded** ("mTLess" — mutual TLS with no session). Only the authentication result (identity verification) is stored. The connection is then allowed. Subsequent retried/new traffic uses WireGuard or IPSec for encryption (not per-session kTLS).

**Why Cilium chose this approach**: It avoids the hard problem of intercepting the application socket's data path. The "first packet drop" is acceptable in Kubernetes because TCP retransmit handles it. The session key discard avoids the complexity of "getting the keys to both kernel instances on two different nodes."

**Open CFP #26480**: Cilium has an open design proposal to use the negotiated session keys for kTLS-based encryption, which would represent the full Overdrive-style architecture. As of 2026-06-04 this is still unimplemented in Cilium. The proposal acknowledges kTLS as the target mechanism.

**Source**: [Cilium mutual auth blog 2024-03-20](https://cilium.io/blog/2024/03/20/improving-mutual-auth-security/) — cilium.io, High (1.0). [Cilium issue #26480](https://github.com/cilium/cilium/issues/26480) — github.com, Medium-High (0.8). [New Stack — Cilium mutual auth security concerns](https://thenewstack.io/how-ciliums-mutual-authentication-can-compromise-security/) — industry, Medium (0.6).

#### Oracle `tlshd` / `ktls-utils` (shipping, for kernel consumers)

Oracle ships `tlshd` (in `ktls-utils`, github.com/oracle/ktls-utils) — the reference implementation of the kernel in-kernel TLS handshake API. `tlshd` listens on a netlink socket for handshake requests from kernel consumers (`sunrpc` for NFS-over-TLS, `nvme-tcp` for NVMe-over-TCP). When a request arrives with a socket FD, `tlshd` uses Rustls (or GnuTLS) to complete the handshake, installs kTLS keys, and returns the socket.

**Limitation vs Overdrive**: `tlshd` works for kernel consumers that explicitly request a handshake (they call the kernel's `tls_client_hello_x509()` API). Arbitrary workloads calling `connect()` do NOT automatically trigger a handshake request — they would need a BPF sockops program to redirect their connection or signal tlshd. This is the gap.

**Kernel version requirement**: `CONFIG_NET_HANDSHAKE=y` (kernel ≥6.5 for stable support). The pinned appliance kernel is 6.6 (≥6.5), so this API is **guaranteed present** — the tlshd-style kernel-driven handshake path is available and is a legitimate design option for roadmap 2.4, not a version-gated maybe.

**Source**: [Oracle ktls-utils README](https://github.com/oracle/ktls-utils) — github.com/oracle, Medium-High (0.8). [tlshd man page](https://www.mankier.com/8/tlshd) — mankier.com, Medium (0.6). [kernel tls-handshake.html](https://docs.kernel.org/networking/tls-handshake.html) — kernel.org, High (1.0).

#### The `ktls` Rust crate (shipping, for application-layer kTLS)

The `ktls` crate (fasterthanlime / rustls org, version 6.0.2) bridges rustls + tokio to kTLS for applications that control their own TLS stack. This is NOT transparent — the application explicitly calls `config_ktls_client()` or `config_ktls_server()` after its own rustls handshake. Used by HTTP servers (e.g., hyper-based stacks) to offload TLS crypto to the kernel post-handshake.

**Relevance**: The `ktls` crate is prior art for the rustls→kTLS plumbing that Overdrive's node agent will need to implement. It is NOT a transparent-interception solution.

**Source**: [ktls crate docs](https://docs.rs/ktls) — docs.rs, High (1.0). [ktls-core crate](https://docs.rs/ktls-core/latest/ktls_core/) — docs.rs, High (1.0).

#### Envoy/Istio ztunnel (shipping, proxy model)

Istio Ambient ztunnel (and Cilium's ztunnel-based mTLS in 1.20) uses iptables-based redirection to a per-node proxy that handles TLS termination. Application sockets are NOT kTLS-armed; they connect to the proxy, which does TLS on their behalf. This is Architecture B (proxy model) and is what production transparent mTLS actually ships as today.

**Source**: Companion doc §Finding 10 cites [Istio ztunnel blog](https://istio.io/latest/blog/2023/rust-based-ztunnel/) — istio.io, High (1.0). [Cilium ztunnel docs](https://docs.cilium.io/en/latest/security/network/encryption-ztunnel/) — docs.cilium.io, High (1.0).

#### LPC 2018 paper: "Combining kTLS and BPF for Introspection and Policy Enforcement" (Daniel Borkmann)

This foundational paper by Daniel Borkmann (kernel BPF maintainer) described the vision for combining kTLS and BPF (then sockmap). It introduced the `sk_msg` framework that unified kTLS and sockmap at the ULP layer. The paper was research/proposal — it did not ship a transparent mTLS solution. It paved the way for the kernel primitives Overdrive plans to use.

**Source**: [LPC 2018 kTLS+BPF paper (PDF)](http://oldvger.kernel.org/lpc_net2018_talks/ktls_bpf_paper.pdf) — kernel.org/lpc_net, Medium-High (0.8). PDF fetch failed (TCP connection closed); content confirmed via secondary search reference.

**Overall Prior Art Assessment**: The "sockops+kTLS transparent mTLS for general workloads" model is **novel territory**. The individual primitives exist and are shipped; combining them transparently for arbitrary workloads (without application changes, without sidecar proxies, without first-packet drops) is an unsolved problem at production scale. Overdrive would be pioneering this if the in-place model (Architecture A) is chosen.

### Finding 9 — Safety invariants & failure modes (no-cleartext-egress, handshake-before-data, fallback)

**Summary**: The primary safety invariants and their failure modes, grounded in the kernel API behaviour and the architecture options from Finding 4.

#### Invariant 1 — No cleartext egress on a connection whose kTLS is not yet armed

**The race window**: Between `ACTIVE_ESTABLISHED_CB` firing (connection established) and `setsockopt(TLS_TX)` completing, the socket is in cleartext mode. Any `write()` by the application sends plaintext. This is the critical safety gap.

**Fail-closed mechanism options**:
- **sockmap block until armed**: Insert the socket into a sockmap with a `sk_msg` verdict program that returns `SK_DROP` for all messages until the node agent signals "kTLS is armed" via a shared BPF map. After the agent sets the flag, the sockmap can be removed. This is fail-closed because `SK_DROP` prevents any data from leaving.
- **`ENOTCONN` delay**: Some architectures block the application's `write()` by holding the socket in a custom state, but this requires patching the kernel or using a LD_PRELOAD shim — not viable for transparent operation.
- **Connection teardown on fail**: If the agent fails to arm kTLS within a timeout, close the connection. The application sees a TCP RST and retries. This is fail-closed but disruptive.

**Recommended approach** (for DISCUSS wave): **sockmap redirect as transient block** — the sockops program puts the new connection in a sockmap at `ACTIVE_ESTABLISHED_CB`, the `sk_msg` verdict holds all data until the node agent acknowledges kTLS installation, then the socket is removed from the sockmap.

#### Invariant 2 — Handshake authentication before data flows (SVID verification)

The TLS handshake itself provides this guarantee: if the remote peer presents a certificate that fails verification (wrong SPIFFE ID, expired SVID, certificate not signed by the cluster CA), rustls returns `Err(TlsError::InvalidCertificate)` and the handshake fails. The socket is then closed. No data can flow on a failed handshake.

**Risk**: If the sockmap block (Invariant 1) is not implemented, an application `write()` reaches the remote in cleartext before the handshake result is known. The remote may buffer and replay this data after the handshake completes, creating a TOCTOU window.

#### Invariant 3 — kTLS rekey support (SVID rotation, key expiry)

**TLS 1.3 KeyUpdate (kernel ≥6.7+)**: Kernel patches for TLS 1.3 KeyUpdate handling were merged into `net-next` (targeting a post-6.6 kernel). The mechanism:
- **RX**: When a KeyUpdate message is received in-band, the kernel returns `EKEYEXPIRED` to reads until userspace provides new keys via `setsockopt(TLS_RX)`. This requires the node agent to remain registered for key update events.
- **TX**: Userspace calls `setsockopt(TLS_TX)` with new keys; kernel transitions at a message boundary.

**Limitation — userspace, not kernel**: in-kernel software kTLS KeyUpdate is **present at the pinned v6.18** (Gap 4, resolved). But in-place rekey also needs the userspace rustls→kTLS bridge to drive it (read `EKEYEXPIRED`, re-provide `TLS_RX`/`TLS_TX`), and the `ktls` crate does not yet support KeyUpdate ([rustls/ktls#59](https://github.com/rustls/ktls/issues/59) open, [#62](https://github.com/rustls/ktls/pull/62) unmerged; tracked in #229). So SVID rotation mid-connection requires **connection teardown and reconnect** (the node agent closes the connection when the SVID expires; the application reconnects and re-handshakes with the fresh SVID). This is acceptable: SPIFFE SVIDs have a 1h TTL, and request-first east-west traffic retries. In-place rekey becomes available when the userspace bridge lands KeyUpdate upstream — **not** by advancing the kernel pin (the kernel is already ready at v6.18).

**Renegotiation**: TLS 1.2 renegotiation is NOT supported in kTLS (kernel documentation explicitly states this). TLS 1.3 does not have renegotiation (it uses KeyUpdate instead). Since the project uses TLS 1.3, renegotiation is moot.

**Source**: [kernel.org kTLS docs — KeyUpdate section](https://docs.kernel.org/networking/tls.html); [kernel patch series for TLS 1.3 KeyUpdate](https://www.mail-archive.com/linux-kselftest@vger.kernel.org/msg20274.html) — mail-archive.com, Medium-High (0.8). [OpenSSL issue #31138](https://github.com/openssl/openssl/issues/31138) — github.com, Medium-High (0.8).
**Confidence**: High for rekey limitations on ≤6.6; Medium for exact kernel version of KeyUpdate merge (patch targeted net-next, specific LTS backport status unknown).

#### Invariant 4 — Kernel-version fallback when kTLS/ULP is unavailable

`setsockopt(SOL_TCP, TCP_ULP, "tls")` returns `ENOENT` if the `tls` module is not loaded (`CONFIG_TLS=m` and not loaded) or `ENOPROTOOPT` if not compiled (`CONFIG_TLS=n`). The `ktls-core` crate's `Compatibilities::probe_kernel()` checks this at startup.

**Fallback path**: If kTLS is unavailable, the node agent keeps rustls in the data path (no kTLS offload). The application continues to think it's sending plaintext, but the agent proxies everything through rustls. This degrades performance (userspace copy overhead) but maintains security.

**BPF_LSM enforcement**: In case of fallback failure, a BPF LSM hook on `socket_sendmsg` can deny egress from identified workload cgroups on non-TLS sockets. This is the belt-and-suspenders layer ensuring no plaintext escapes even if the kTLS path fails. This hook depends on the `IDENTITY_MAP` and policy layer (roadmap 2.4 + 2.13).

**Confidence**: High for `ENOENT`/`ENOPROTOOPT` failure modes (kernel documentation). Medium for the exact fallback path (design decision for DISCUSS wave).

### Finding 10 — Testing strategy across the four tiers (DST sim, BPF unit, real-kernel ss -K / wire capture, verifier budget)

**Summary**: Mapped to the four tiers from `.claude/rules/testing.md`. The whitepaper §22 and the testing rules already prescribe the mandatory test cases; this finding grounds them in the researched mechanism details.

#### Tier 1 — Deterministic Simulation Testing (DST with Sim traits)

**What to simulate**: The sockops layer communicates with the node agent via BPF maps (ringbuf/perf events). The `SimDataplane` trait is the natural boundary:

- `SimDataplane::intercept_connection(5-tuple)` — represents what the sockops program does: notify the agent of a new connection
- `SimDataplane::arm_ktls(socket_ref, crypto_info)` — represents the kTLS installation
- Policy enforcement: whether plaintext can escape before kTLS is armed

**Key DST invariants**:
```rust
// Safety invariant — no cleartext egress before kTLS armed
assert_always!("no-cleartext-egress",
    !connection.has_sent_data() || connection.ktls_armed());

// Liveness — kTLS eventually armed on new connections
assert_eventually!("ktls-armed",
    all_established_connections.all(|c| c.ktls_armed()));

// Handshake — SVID verification completes before data flows
assert_always!("svid-verified-before-data",
    !connection.has_sent_data() || connection.peer_svid_verified());
```

**Coverage for the race window**: DST can explore the timing between `ACTIVE_ESTABLISHED_CB` and kTLS installation — the sim can interleave an application `write()` between these two events and assert the safety invariant holds (i.e., the sockmap block drops the write).

#### Tier 2 — BPF Unit Tests (`BPF_PROG_TEST_RUN`)

**Critical limitation**: `BPF_PROG_TEST_RUN` returns `ENOTSUP` for `BPF_PROG_TYPE_SOCK_OPS` on most kernels (as noted in `.claude/rules/development.md` for cgroup_sock_addr programs). This means the **sockops program itself cannot be tested via BPF_PROG_TEST_RUN**; it requires a real kernel connection.

**What CAN be BPF unit tested** (via programs that work with PROG_TEST_RUN):
- The `sk_msg` verdict program logic (filter/pass/drop decisions based on map lookups)
- The `sk_skb` stream parser for protocol inspection

**Tier 2 coverage for sockops**: Limited to compilation and verifier acceptance. No runtime test via `BPF_PROG_TEST_RUN`. All sockops-specific runtime tests are Tier 3.

#### Tier 3 — Real-Kernel Integration (Lima VM, `ss -K`, wire capture)

This is the primary functional test tier for sockops+kTLS. The mandatory test cases from the whitepaper §22 are:

**Test 1**: `connect()` intercepted at `BPF_SOCK_OPS_TCP_CONNECT_CB`/`ACTIVE_ESTABLISHED_CB`:
- Start two processes in the Lima VM, each in an Overdrive workload cgroup
- Process A calls `connect()` to Process B
- Assert: the sockops program fires, writes to a BPF map (5-tuple observed)
- Verify via `ss -K` after kTLS is installed: output shows `TLSv1.3` on the socket

**Test 2**: Wire capture shows TLS 1.3 records:
- Use `tcpdump -i lo` on the veth/loopback between the two workloads
- Assert: captured packets have TLS 1.3 record headers (content type 0x17 = Application Data, version 0x0303)
- Assert: payload is ciphertext (not plaintext content)

**Test 3**: Wrong SVID fails the handshake:
- Present a certificate from a different cluster or an expired SVID
- Assert: `connect()` fails with a TLS error; no data flows; socket is closed
- Assert: BPF ringbuf event records the handshake failure with the rejected SVID

**Test 4 (safety/negative)**: Cleartext cannot escape before kTLS is armed:
- Instrument the test to attempt a `write()` from the application immediately after `connect()` returns, before the node agent has a chance to arm kTLS
- Assert: the data does not appear in the wire capture (sockmap block is effective)

**Kernel version coverage**: Overdrive pins the appliance kernel (6.6 LTS), so Tier 3 runs against **the pinned kernel** plus `bpf-next` (early-warning soft-fail) — there is no diverse-operator-kernel matrix to sweep. TLS 1.3 RX offload and `CONFIG_NET_HANDSHAKE` are both guaranteed present at the pin. `ss -K` output is therefore a fixed, known shape rather than a per-kernel variable.

**`ss -K` verification command** (from `.claude/rules/testing.md` §"Real-Kernel Integration"):
```bash
ss -K '( dst <addr> and dport = <port> )'
# or
ss -tlnp | grep <port>
# With kTLS installed, ss shows "TLSv1.3" or "TLSv1.2" in the output
```

#### Tier 4 — Verifier budget

The sockops program is comparatively simple (reads fields, writes to map, calls setsockopt). The `sk_msg` verdict program is also simple (map lookup + redirect or pass/drop). Both should be well within the 1M verified-instruction budget.

**Baseline target**: < 50,000 verified instructions per program (comparable to the XDP programs already in the project). Establish the baseline in `perf-baseline/main/verifier-budget/` after first functional implementation.

**Source**: [.claude/rules/testing.md §Tier 3 — Real-Kernel Integration](/.claude/rules/testing.md) — project internal. [whitepaper.md §22 — Real-Kernel Integration Testing](docs/whitepaper.md) — project SSOT. [aya-ebpf SockOps docs](https://docs.rs/aya-ebpf/latest/aya_ebpf/programs/struct.SockOps.html) — docs.rs, High (1.0).
**Confidence**: High for Tier 3 strategy (derives directly from project testing rules and whitepaper); Medium for Tier 2 limitation (BPF_PROG_TEST_RUN sockops support may vary by kernel version — single-source caveat from development.md).

## Cross-Cutting Risks & Open Questions

### Risk 1 — The race window between connection establishment and kTLS installation (CRITICAL)

**Description**: The window between `ACTIVE_ESTABLISHED_CB` firing and `setsockopt(TLS_TX)` completing is the single most dangerous aspect of the architecture. During this window, a write() from the application sends plaintext. The node agent's turnaround time (event dispatch + rustls handshake + key installation) is tens of milliseconds — long enough for a busy workload to write data.

**Mitigation required**: The DISCUSS wave must select one of the three mitigations described in Finding 4 (sockmap block being the recommended approach) and make it a hard architectural requirement, not an optimization.

**Open question for DISCUSS**: Should Overdrive use `sk_msg` verdict program as a transient block (Architecture A hybrid), or should it go directly to Architecture B (full proxy redirect) and accept the latency overhead in exchange for simplicity?

### Risk 2 — TLS 1.3 RX kTLS minimum kernel (RESOLVED by pinned appliance kernel)

**Status**: **Resolved 2026-06-04.** TLS 1.3 Receive decryption in kTLS requires kernel ≥6.0. This was a binding constraint only under a diverse-operator-kernel matrix that included pre-6.0 entries. **Overdrive ships its own appliance OS and pins the kernel at 6.6 LTS** (≥6.0), so full in-kernel TLS 1.3 TX+RX is guaranteed present and no TX-only / userspace-RX fallback tier exists to design. The "node agent exits the data path entirely" goal is achievable for inbound connections at the pin.

**No residual**: the kernel-side handshake API (`CONFIG_NET_HANDSHAKE`, ≥6.5) is also satisfied at the 6.6 pin, so it too is available without caveat (see Finding 4 / Architecture D). Kernel version is no longer a risk axis — it is a controlled constant.

### Risk 3 — SVID rotation on long-lived connections

**Description**: In-kernel kTLS KeyUpdate is **present at the pinned v6.18** (Gap 4, resolved), but the userspace rustls→kTLS bridge (`ktls` crate) does not yet drive it ([rustls/ktls#59](https://github.com/rustls/ktls/issues/59) / [#62](https://github.com/rustls/ktls/pull/62), tracked in #229). Long-lived connections (> SVID TTL = 1h) therefore cannot rotate keys in place today; the option is connection teardown and reconnect. The path to in-place rekey is the **userspace** bridge landing KeyUpdate upstream — **not** advancing the kernel pin (the kernel is already ready).

**Impact**: Applications with very long-lived connections (databases, streaming, persistent gRPC) will see periodic brief disconnects at SVID rotation time. This is a known limitation of the approach and should be documented.

### Risk 4 — pidfd_getfd / fd interception complexity

**Description**: For Architecture A (in-place kTLS), the node agent must obtain the application socket's file descriptor to perform the handshake on it. This requires either: (a) `pidfd_getfd()` (kernel 5.6+) from a process with `CAP_SYS_PTRACE` equivalent, or (b) the workload process cooperating via a Unix socket fd-passing protocol. For process workloads, option (a) is viable but requires root/capability. For VMs and unikernels, the socket is not accessible to the host agent via fd.

**Open question for DISCUSS**: Does the "sockops intercepts → agent performs handshake" model work for MicroVM and unikernel workloads where the socket is in a separate kernel (Cloud Hypervisor)? If not, Architecture B (proxy) may be the only option for those workload types.

### Risk 5 — No shipping precedent for full Architecture A with general workloads

**Description**: Finding 8 establishes that no production system has shipped Architecture A (in-place kTLS, no proxy, all workload types) for general-purpose workloads. Oracle's `tlshd` is the closest precedent but only works for kernel-initiated handshake requests. This is novel territory.

**Recommendation**: The DISCUSS wave should explicitly weigh Architecture A (novel, complex, max performance) vs Architecture B (established, proxy-based, some latency overhead) and document the decision in an ADR.

## Recommendations

1. **DISCUSS wave must choose Architecture A vs B explicitly**. Architecture A (in-place kTLS) is the whitepaper model — max performance, zero overhead after handshake, but requires solving the race window and fd-interception problems that no production system has solved for general workloads. Architecture B (sockmap proxy redirect) is what Istio/Cilium actually ship — simpler, proven, but adds per-packet copy overhead. A hybrid (Architecture A with transient sockmap block) is the most promising path but is also the most novel. This decision gates everything else in roadmap 2.4 and must be resolved before the DESIGN wave begins.

2. **Kernel is pinned by the appliance OS at 6.6 LTS (DECIDED 2026-06-04, pending ADR).** Overdrive ships its own stripped-down immutable Linux (Image Factory, §23) and controls the kernel version; it does not target a diverse-operator-kernel matrix. At the 6.6 pin, full in-kernel TLS 1.3 TX+RX (≥6.0) and `CONFIG_NET_HANDSHAKE` (≥6.5) are both guaranteed present — no fallback-tier design work and no per-kernel feature gating is required. The matrix SSOT (`.claude/rules/testing.md` § "Kernel matrix", `docs/whitepaper.md` § 22) must be rewritten from "support 5.10→current LTS" to "**pinned appliance kernel (6.6) + `bpf-next` soft-fail**", and the change recorded in an ADR via the architect agent (`testing.md`: "Dropping a kernel requires an ADR"). The appliance can advance the pin at will, so newer-kernel features (future BPF verdicts, etc.) are roadmap choices. (In-place TLS 1.3 KeyUpdate rekey is **not** one of them — the kernel already supports it at v6.18; the blocker is the userspace `ktls` bridge, rustls/ktls#59 / #62 / #229.)

3. **Design the SVID rotation strategy for long-lived connections before the DESIGN wave**. At the 6.6 pin, KeyUpdate is not available (it landed post-6.6). The rotation strategy (connection teardown + reconnect) must be baked into the reconciler model for `ServiceLifecycle` — it is a convergence action, not an error. If in-place rekey becomes a requirement, the lever is the userspace rustls→kTLS bridge landing KeyUpdate (rustls/ktls#59 / #62, tracked in #229) — the kernel already supports it at v6.18, so advancing the pin is **not** the lever.

4. **Adopt `BPF_MAP_TYPE_SK_STORAGE` for per-socket identity tracking**. It is the correct primitive for attaching SPIFFE identity to a socket across the sockops → sk_msg → node agent lifecycle. The IDENTITY_MAP (from IdentityMgr, GH #35) should be a separate lookup map keyed by source IP/cgroup; the per-socket state (resolved identity, handshake status) should be in SK_STORAGE.

5. **Use the `ktls` / `ktls-core` Rust crates as the rustls→kTLS bridge**, rather than re-implementing the `ffi.rs` mapping. The crates are under the rustls org, actively maintained, and tested against all relevant kernels. The node agent implementation should `cargo add ktls-core` or `ktls` and build on this foundation.

6. **Read Oracle's `ktls-utils`/`tlshd` source** before the DESIGN wave. It is the closest existing implementation to what Overdrive needs and contains the hardest-earned lessons about socket fd interception, handshake timing, and kTLS installation sequences.

7. **The Tier 3 test for sockops must include a negative test for the race window** (Finding 10, Test 4). Without this test, the safety invariant "no cleartext egress before kTLS armed" cannot be verified. The test must attempt a `write()` from the workload process immediately after `connect()` returns and verify via tcpdump that no plaintext data appears on the wire.

## Source Catalogue

| # | Source | Domain | Reputation | Type | Access Date | Used In |
|---|--------|--------|------------|------|-------------|---------|
| 1 | [Linux Kernel TLS documentation](https://docs.kernel.org/networking/tls.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F2, F5, F9 |
| 2 | [Linux Kernel in-kernel TLS handshake API](https://docs.kernel.org/networking/tls-handshake.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F1, F8 |
| 3 | [aya-ebpf SockOpsContext source](https://docs.rs/aya-ebpf/latest/src/aya_ebpf/programs/sock_ops.rs.html) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F3 |
| 4 | [aya-ebpf bindings constants (BPF_SOCK_OPS_*)](https://docs.rs/aya-ebpf/latest/aya_ebpf/all.html) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F3 |
| 5 | [docs.ebpf.io BPF_PROG_TYPE_SOCK_OPS](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-04 | F3, F10 |
| 6 | [rustls kernel module docs](https://docs.rs/rustls/latest/rustls/kernel/index.html) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F5 |
| 7 | [ktls crate ffi.rs source](https://docs.rs/ktls/latest/src/ktls/ffi.rs.html) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F5 |
| 8 | [ktls-core crate documentation](https://docs.rs/ktls-core/latest/ktls_core/) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F2, F5, F9 |
| 9 | [ktls crate overview](https://docs.rs/ktls) | docs.rs | High (1.0) | Technical docs | 2026-06-04 | F5, F8 |
| 10 | [Linux Kernel BPF SK_STORAGE documentation](https://docs.kernel.org/bpf/map_sk_storage.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F6 |
| 11 | [eBPF docs bpf_sk_storage_get](https://docs.ebpf.io/linux/helper-function/bpf_sk_storage_get/) | docs.ebpf.io | High (1.0) | Technical docs | 2026-06-04 | F6 |
| 12 | [kernel.org BPF sockmap documentation](https://docs.kernel.org/bpf/map_sockmap.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F4 |
| 13 | [LWN.net sockmap integration for kTLS](https://lwn.net/Articles/768371/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-04 | F4 |
| 14 | [LWN.net original BPF socket ops introduction](https://lwn.net/Articles/725722/) | lwn.net | Medium-High (0.8) | Industry | 2026-06-04 | F3 |
| 15 | [Cilium ztunnel transparent encryption docs](https://docs.cilium.io/en/latest/security/network/encryption-ztunnel/) | docs.cilium.io | High (1.0) | Technical docs | 2026-06-04 | F1, F4, F8 |
| 16 | [Cilium mutual authentication docs](https://docs.cilium.io/en/latest/network/servicemesh/mutual-authentication/mutual-authentication/) | docs.cilium.io | High (1.0) | Technical docs | 2026-06-04 | F8 |
| 17 | [Cilium issue #26480 — use negotiated session key for kTLS](https://github.com/cilium/cilium/issues/26480) | github.com | Medium-High (0.8) | Open source | 2026-06-04 | F8 |
| 18 | [Oracle ktls-utils (tlshd)](https://github.com/oracle/ktls-utils) | github.com | Medium-High (0.8) | Open source | 2026-06-04 | F1, F4, F8 |
| 19 | [tlshd man page](https://www.mankier.com/8/tlshd) | mankier.com | Medium (0.6) | Reference | 2026-06-04 | F8 |
| 20 | [kernel.org kTLS TLS 1.3 KeyUpdate patch series](https://www.mail-archive.com/linux-kselftest@vger.kernel.org/msg20274.html) | mail-archive.com (kernel ML) | Medium-High (0.8) | Technical | 2026-06-04 | F9 |
| 21 | [OpenSSL issue #31138 — kTLS EKEYEXPIRED](https://github.com/openssl/openssl/issues/31138) | github.com | Medium-High (0.8) | Open source | 2026-06-04 | F9 |
| 22 | [NVIDIA Mellanox kTLS Offloads docs](https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads) | docs.nvidia.com | High (1.0) | Official | 2026-06-04 | F7 |
| 23 | [kernel-internals.org kTLS article](https://kernel-internals.org/net/ktls/) | kernel-internals.org | Medium-High (0.8) | Technical | 2026-06-04 | F2, F7 |
| 24 | [ConnectionTrafficSecrets enum docs](https://serenity-rs.github.io/poise/current/rustls/enum.ConnectionTrafficSecrets.html) | serenity-rs.github.io (mirror of rustls) | Medium-High (0.8) | Technical docs | 2026-06-04 | F5 |
| 25 | Companion: [transparent-encryption-comprehensive-research.md §Finding 9](docs/research/transparent-encryption-comprehensive-research.md) | project internal | High (1.0) | Project research | 2026-06-04 | F7 |
| 26 | [aya-rs usage research §B.3, §Gap K-2](docs/research/dataplane/aya-rs-usage-comprehensive-research.md) | project internal | High (1.0) | Project research | 2026-06-04 | F3 |

**Reputation summary**: High (1.0): 17 sources (65%) | Medium-High (0.8): 7 sources (27%) | Medium (0.6): 2 sources (8%) | **Average**: 0.92

**Knowledge Gaps documented**:

### Gap 1: Exact kernel version for TLS 1.3 RX kTLS merge
**Issue**: Multiple sources indicate ~6.0 but exact LTS backport commit is not confirmed. Some sources say 6.0, some say 5.20 (dev cycle name for 6.0). **Attempted**: kernel.org docs, kernel-internals.org, community sources. **Recommendation**: Verify against `git log net/tls/ --grep="tls13" --oneline` in the Linux kernel repo; confirm the exact kernel version at implementation time.

### Gap 2: Architecture A socket fd interception mechanism for non-process workloads
**Issue**: For MicroVM and unikernel workloads, the application socket is inside Cloud Hypervisor's virtual kernel — the host node agent cannot obtain the VM's socket fd via `pidfd_getfd`. How Overdrive's sockops+kTLS architecture applies to VM-based workloads is unresolved. **Attempted**: whitepaper §6, CloudHypervisor docs, Oracle tlshd. Nothing found. **Recommendation**: DISCUSS wave must address this explicitly; proxy model (Architecture B) may be required for VMs.

### Gap 3: Exact `tlshd` invocation pattern from a BPF sockops program
**Issue**: While Oracle's `tlshd` implements the kernel handshake API for kernel consumers, the mechanism for triggering `tlshd` from a BPF sockops program (for arbitrary workload sockets) is not documented. The kernel's `tls_client_hello_x509()` API is for kernel subsystems, not for signalling from sockops. The bridge is unclear. **Attempted**: ktls-utils README, kernel tls-handshake.html. Not found. **Recommendation**: Review `ktls-utils` source code directly; may need to implement a custom netlink protocol between sockops and the node agent.

### Gap 4: TLS 1.3 KeyUpdate kernel availability — RESOLVED (kernel-side present at v6.18)
**Status (2026-06-11): RESOLVED for the kernel layer.** In-kernel software kTLS TLS 1.3 KeyUpdate is **confirmed present at the pinned v6.18 tag** — `Documentation/networking/tls.rst` @ `v6.18` carries the "TLS 1.3 Key Updates" section: decryption pauses on an inbound KeyUpdate, reads return `EKEYEXPIRED` until userspace re-provides keys via `TLS_RX`/`TLS_TX`, and the MIB exposes `TlsTxRekeyOk` / `TlsRxRekeyOk` / `TlsRxRekeyReceived`. (Originated in Sabrina Dubroca's "tls: implement key updates for TLS1.3" series; software path, merged ≤ v6.18.) Hardware-offloaded KeyUpdate (ConnectX-6 Dx / mlx5, [LWN 1055522](https://lwn.net/Articles/1055522/)) is a separate, newer, NIC-specific series with software fallback — irrelevant to Overdrive's software-kTLS / virtio path.
**The remaining blocker is entirely userspace.** The kernel is ready at v6.18; in-place rekey is unavailable only because the rustls→kTLS bridge (`ktls` crate) does not yet support KeyUpdate — [rustls/ktls#59](https://github.com/rustls/ktls/issues/59) open, [#62](https://github.com/rustls/ktls/pull/62) unmerged (tracked in #229). v1 = teardown+reconnect until that lands.
**Source**: [docs.kernel.org/networking/tls.html](https://docs.kernel.org/networking/tls.html) (High 1.0); `Documentation/networking/tls.rst` @ `v6.18` git tag (High 1.0).

**Conflicting Information**:

### Conflict 1: Does sockops `BPF_SOCK_OPS_TCP_CONNECT_CB` fire before or after SYN-ACK?
The name suggests "TCP_CONNECT_CB" fires when `connect()` is called (SYN sent), while `ACTIVE_ESTABLISHED_CB` fires after three-way handshake. The eBPF docs list confirm both exist separately. For kTLS interception, `ACTIVE_ESTABLISHED_CB` is the correct hook (connection is fully established; the TCP socket has an allocated sk that can accept ULP). Using `TCP_CONNECT_CB` would be too early. **Assessment**: Not a conflict — two different events; use `ACTIVE_ESTABLISHED_CB` for kTLS interception.
