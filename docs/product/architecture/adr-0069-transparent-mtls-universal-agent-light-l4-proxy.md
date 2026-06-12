# ADR-0069: Transparent mTLS via a universal agent-light L4 proxy

## Status

**Accepted** (2026-06-12). **Amends** whitepaper §7 ("Transparent mTLS — one
identity model, two enforcement mechanisms") and §8 ("Identity and mTLS"):
collapses the two-enforcement-mechanism framing (host-socket in-band kTLS #26 +
guest-stack L4 tap proxy #222) into ONE universal mechanism. **Supersedes** the
in-band sockops+kTLS-on-the-workload's-own-socket model as the v1 #26 enforcement
path; that model is NOT in v1 scope — a post-v1 optimization tracked in **#231**
(see *Alternatives* A1).

Folds GH [#222](https://github.com/overdrive-sh/overdrive/issues/222) into GH
[#26](https://github.com/overdrive-sh/overdrive/issues/26) as the **STAGED
guest-stack intercept adapter** of the one universal proxy (NOT a separate
mechanism). Job **J-SEC-003**.

**Amendment (2026-06-12, re-review F3–F7).** The proxy is **BIDIRECTIONAL** in
host-socket v1 — both the outbound/client half AND the inbound/server half
(TPROXY intercept → `getsockname` orig-dst → server-side mutual-TLS verifying the
client → splice-to-server) are designed and spike-proven
(`findings-inbound-intercept.md`, increment-i, kernel 7.0). BOTH workloads are
identity-unaware and hold nothing; each node's agent does its side. The
**guest-stack** intercept adapter (microVM/unikernel) is STAGED to #222. The
honest v1 security claim is **chain-to-bundle transport authentication +
encryption, with NO intended-peer identity pinning** (the intended-peer SAN-match
is the #178 upgrade, not a v1 prereq). Resource limits (`MtlsLimits`) carry
concrete values (F7); the return/deliver pump has a teardown-on-stall supervision
policy (F6). The core fold decision, OQ-2, and SD-1…SD-4 are UNCHANGED — F3/F6/F7
are additive.

## Context

Overdrive's identity model is universal: one SPIFFE CA, one set of per-allocation
SVIDs, one trust bundle, one SPIFFE-ID `POLICY_MAP` (whitepaper §7/§8). The
question this ADR settles is **the enforcement mechanism** — how the held SVID
(minted by #28, held/read via the `IdentityRead` port, #35) becomes TLS 1.3
records *on the wire*.

Whitepaper §7 previously split enforcement by where the workload's TCP stack
terminates:

- **Host-socket** (process/exec, WASM) — TCP in the host kernel — got the
  **in-band** model: sockops detects ESTABLISHED → the agent `pidfd_getfd`s the
  workload's own socket → rustls handshake presenting the held SVID → kTLS
  installs on the **workload's own socket** → the agent exits → the kernel
  carries the steady-state crypto. Workload owns the fd; restart-survivable; one
  socket per connection.
- **Guest-stack** (microVM, unikernel) — TCP in the *guest* kernel, invisible to
  host BPF — got a separate **host L4 tap proxy** (#222): intercept the guest's
  TCP, terminate on the host, re-originate an outbound mTLS connection, install
  host kTLS on the proxy's egress socket.

This "one identity model, two enforcement mechanisms" framing was carried as a
DESIGN-pending decision: the precise tap intercept and the in-kernel
`sockmap`/`bpf_sk_redirect` splice were flagged as open, "pending a Tier-3
kTLS+sockmap spike" (§7, verbatim).

### The empirical evidence base (this is what settles it)

Spikes and research docs, all on a
real 7.0 kernel (≥ the pinned 6.18 floor, ADR-0068), software kTLS, AES-256-GCM
TLS 1.3. Committed under `docs/feature/transparent-mtls-host-socket/spike/` and
`docs/research/dataplane/` at `353cdc52` (and the inbound increment-i added
since). The findings are decision-grade. **The *mechanism* is spike-verified —
both the primitives in isolation AND a composed real-intercept flow in one
direction.** The primitives are each proven (forward sockmap-egress splice into
kTLS-TX, return `splice(2)` pump on a no-psock kTLS-RX leg, kTLS arm, the
SOCKMAP-before-`TCP_ULP` ordering invariant). And the **inbound half is proven
COMPOSED end-to-end**: `findings-inbound-intercept.md` (increment-i §2, *ok*
mode) demonstrated a real TPROXY transparent intercept → `getsockname` orig-dst
recovery → server-side mutual-TLS (the agent presents S's SVID and
`WebPkiClientVerifier` VERIFIES C's client SVID chains to the bundle) → kTLS-RX
arm → agent-light `splice` of the decrypted plaintext to an identity-unaware
server S, with S reading byte-exact plaintext while the client leg carries TLS
`0x17` ciphertext, and fail-closed on `nocert`/`wrongca` (distinct reasons, 0
bytes to S). That is a composed real-intercept flow, end-to-end, in one
direction.

**Three NARROW composition gaps remain** — these are the integration/
walking-skeleton scope, NOT "the mechanism is unproven":
1. **Outbound composed in ONE flow** — increment-e proved outbound
   intercept + lossless pre-arm capture + handshake-window flush; increment-f
   proved the steady-state egress splice; but on SEPARATE harnesses (increment-f
   deliberately removed the `cgroup_connect4` intercept to isolate the splice
   primitive, and increment-e's steady-state was blocked by a *throwaway-harness
   intercept-lifecycle RST — explicitly a harness limitation, NOT a kernel
   finding*). The two were never wired into one outbound flow.
2. **Bidirectional steady-state round-trip** — inbound drove only C→S
   (request); outbound forward drove only F→B; neither composed the response
   leg. (The agent-IDLE sockmap-verdict bidirectional splice has a known
   leg-B-RX-psock vs kTLS-RX conflict — increment-f "Load-bearing mechanics" #2 —
   so the return/deliver direction uses the proven agent-LIGHT `splice(2)` path
   from increment-i/h, not the agent-idle redirect.)
3. **Real netns/veth topology + cgroup-isolated workloads** — every spike was
   loopback + sibling processes.

**Closing gaps 1–3 — composing the proven pieces into ONE bidirectional walking
skeleton in the real netns/veth topology — is the FIRST DELIVER slice (Slice 00,
BLOCKING).** Slice 00 is an integration/walking-skeleton gate, NOT a
"prove-the-mechanism" gate (see *Consequences* → *Composition gate* and the
feature-delta DESIGN handoff). The load-bearing results:

1. **The in-band lossless path is foreclosed three independent ways** — proving
   the in-band model cannot be made *lossless* for a client-speaks-first flow on
   runtime-loadable BPF:
   - `sk_msg` has **PASS/DROP/REDIRECT and no HOLD**; `bpf_msg_cork_bytes` does
     not buffer. A pre-arm write is `SK_DROP`'d → `EACCES` + dead connection
     (`findings.md` increment C).
   - A sockmap **redirect of a live socket's own egress bypasses the source
     socket's TX path** (`__SK_REDIRECT` never calls `tcp_sendmsg_locked` on the
     source → `write_seq`/`snd_nxt` never advance → deterministic RST, 3/3). This
     is by-design redirect semantics, invariant through 7.0 (`findings-lossless-
     hybrid.md`; `sockmap-redirect-live-socket-liveness-research.md` SQ2).
   - Lossless capture of the workload's pre-arm egress **structurally requires
     transparent interception** (you cannot read a workload's TX off a
     `pidfd_getfd` dup — `recv` drains RX, never TX), which makes the topology a
     **two-socket proxy** — i.e. the lossless variant of #26 *is* #222
     (`findings-userspace-relay.md`).

2. **The universal agent-light L4 proxy is proven agent-light in BOTH directions
   on 7.0**:
   - **Forward** (workload-plaintext leg F → peer-facing kTLS-TX leg B):
     **agent-IDLE**. An in-kernel sockmap **EGRESS** redirect
     (`bpf_sk_redirect_map`, `flags=0`) into a kTLS-armed target drives
     `tcp_sendmsg_locked` → encrypted egress; the agent issues ZERO per-byte
     syscalls (`findings-egress-ktls-splice.md`, 15/15 deterministic; strace
     proof).
   - **Return** (peer-facing kTLS-RX-decrypt leg B → workload-plaintext leg F):
     **agent-LIGHT, zero-copy `splice(2)`**. On a **plain kTLS-RX socket (NO
     sockmap, NO psock)**, `tls_sw_splice_read` decrypts each TLS record and
     splices the *clean decrypted plaintext* (no header, no inner-type byte, no
     GCM tag) into a pipe → leg F. The agent issues only `splice()` + `ppoll()`
     — **no per-byte userspace copy** — at ~1 splice per TLS record, kernel-paced
     and bounded (`findings-splice-return.md`; research verdict (c) →
     **(a) CONFIRMED**).
   - NOTE the sockmap-verdict return path was foreclosed as a return mechanism
     (`findings-ktls-rx-splice.md`: the kTLS-RX decrypted-verdict runs only inside
     `tls_sw_recvmsg`; `tls_sw_read_sock` returns `-EINVAL` on a psock → no
     agent-idle push path). The chosen return mechanism is the **`splice(2)`
     pump on a no-psock kTLS-RX leg**, which sidesteps that `-EINVAL` entirely.

3. **The basic mechanism is proven**: `sockops → rustls handshake →
   dangerous_extract_secrets → kTLS arm`; the `pidfd_getfd` cross-process handoff;
   the **SOCKMAP-insert-before-`TCP_ULP "tls"`** ordering invariant (reverse is
   `EINVAL` — both replace `sk->sk_prot`); control-record handling
   (`NewSessionTicket`/KeyUpdate → `EIO` on raw kTLS RX → favours
   `ktls::KtlsStream` over raw `setsockopt`). All in `findings.md`.

### The forcing question

The user has decided (2026-06-12): given the evidence, **fold #222 into #26 and
build ONE universal "transparent mTLS via an agent-light L4 proxy"** as THE
enforcement mechanism for ALL workload kinds (process/exec, WASM, microVM,
unikernel). The in-band model wins only two things — restart-survival and
1-socket density — and loses **uniformity** (a separate guest-stack mechanism is
unavoidable for #222 regardless) and **losslessness** (no lossless
client-speaks-first path exists in-band). The proxy is lossless for every kind,
needs no kernel patch, and is one mechanism instead of two. This ADR designs that
decision; it does not relitigate it.

### Quality attributes driving the decision (ISO 25010)

| Attribute | Why it dominates here |
|---|---|
| **Security — confidentiality, authenticity** | The whole feature. The wire must carry TLS 1.3 records with the workload's own SVID; no cleartext leak; auth-session == data-session; fail-closed on absent/wrong SVID. |
| **Functional suitability — universality** | One mechanism that works for process, WASM, AND guest-stack (microVM/unikernel) — the user's primary driver. The proxy is the only shape that is lossless for *every* kind. |
| **Reliability — losslessness** | No dropped pre-arm bytes, no connection RESET on client-speaks-first. The proxy buffers in userspace at the handshake window (trivially lossless); the in-band model cannot (no HOLD). |
| **Maintainability — one mechanism, DST-testable port** | A single driven port with host + Sim adapters, exercised by a DST equivalence harness, beats two divergent kernel paths. |
| **Performance efficiency — agent-light** | Steady state is agent-idle forward (kernel splice) + agent-light return (zero-copy `splice`, ~1/record). No userspace per-byte copy in either direction. Two sockets/connection and a per-connection handshake are the accepted cost. |

The trade-off the user accepted: **uniformity + losslessness over
restart-survival + 1-socket density**. Restart-survival becomes a post-v1
optimisation tracked in **#231**; density (2 sockets/conn) is the steady-state
cost.

## Decision

**Build a single universal transparent-mTLS enforcement mechanism: an
agent-light L4 proxy that enforces in BOTH directions.** Every node's agent
enforces both halves of every flow it touches:

- **OUTBOUND (client side)** — the workload's outbound `connect()` is
  **transparently intercepted** (`cgroup/connect4`-rewrite) to a node-local
  agent leg; the agent drains the workload's plaintext losslessly, performs the
  rustls TLS 1.3 **client** handshake with the real peer presenting the
  workload's held SVID (read via `IdentityRead`), arms kTLS on the peer-facing
  leg, and **hands the steady-state byte movement to the kernel**.
- **INBOUND (server side)** — an inbound connection aimed at a server
  workload's logical address is **transparently intercepted via TPROXY**
  (the mirror of `connect4`) to the agent's `IP_TRANSPARENT` listener; the
  agent recovers the original destination via `getsockname()`, selects the
  **server** workload's SVID from that original destination, performs the
  rustls TLS 1.3 **server** handshake (presents the server SVID, **verifies the
  client's SVID** chains to the trust bundle via `WebPkiClientVerifier`), arms
  kTLS-RX, and **splices the decrypted plaintext to the identity-unaware server
  workload** (agent-light, zero-copy `splice`).

**BOTH workloads are identity-unaware and hold NOTHING** — no cert, no key.
Each is paired with its node's agent: the client-side agent does the outbound
half (intercept connect → present client SVID); the server-side agent does the
inbound half (TPROXY intercept → present server SVID + verify client → deliver
plaintext). The "peer" a given agent dials/accepts is **the other workload's
agent**, not a TLS-aware workload — this resolves the C4 self-contradiction
(`c4-diagrams.md`: "peer presents its own SVID" was shorthand for "the peer's
*agent* presents the peer workload's SVID"; the workload itself holds nothing).
Both directions are real-kernel proven on 7.0: outbound in increments-f/g
(`findings-egress-ktls-splice.md` / `findings-splice-return.md`), inbound in
increment-i (`findings-inbound-intercept.md` — TPROXY intercept + orig-dst
recovery + server-side mutual-TLS + kTLS-RX decrypt + agent-light splice-to-S,
fail-closed on `nocert`/`wrongca`).

The proxy topology, per-connection — OUTBOUND (client side):

```
workload ──plaintext──▶ [leg F: agent-owned plaintext leg]
                              │  (handshake window: agent drains plaintext losslessly;
                              │   rustls handshake on leg B presenting the held SVID;
                              │   arm kTLS on leg B; flush captured plaintext)
                              ▼
                        [leg B: agent-owned kTLS leg] ──TLS 1.3 records──▶ real peer
   steady state:
     forward  F → B : in-kernel sockmap EGRESS redirect (bpf_sk_redirect_map, flags=0)
                      → tcp_sendmsg_locked on leg B → kTLS encrypt        [AGENT-IDLE]
     return   B → F : splice(legB → pipe → legF) on a plain kTLS-RX leg
                      → tls_sw_splice_read decrypts → clean plaintext      [AGENT-LIGHT]
```

The proxy topology, per-connection — INBOUND (server side, the mirror; proven in
`findings-inbound-intercept.md`):

```
real peer's agent ──TLS 1.3 records (presents client SVID)──▶ [server workload's logical addr]
                              │  (nft TPROXY prerouting → IP_TRANSPARENT listener;
                              │   getsockname() recovers ORIG_DST → selects the server
                              │   workload's AllocationId → its held SVID)
                              ▼
                        [leg C: agent-owned client-facing kTLS leg]
                              │  (rustls SERVER handshake: present the server SVID;
                              │   WebPkiClientVerifier REQUIRE+VERIFY the client SVID
                              │   chains to the bundle; arm kTLS-RX)
                              ▼
   steady state:
     deliver  C → S : splice(legC → pipe → legS) on a plain kTLS-RX leg
                      → tls_sw_splice_read decrypts → clean plaintext      [AGENT-LIGHT]
                              ▼
                        [leg S: agent-owned plaintext leg facing the server workload]
                              ▼
                        server workload (plain TCP, holds NOTHING — reads byte-exact plaintext)
```

Inbound naming (mirror of outbound's leg F / leg B): **leg C** = the agent-owned
**c**lient-facing kTLS leg (the TPROXY-intercepted connection the agent
`accept()`s on its `IP_TRANSPARENT` listener — the inbound analogue of leg B);
**leg S** = the agent-owned plaintext leg facing the **s**erver workload (the
inbound analogue of leg F). The inbound steady state is the kTLS-RX decrypt →
`splice`-to-server pump (agent-light, `findings-inbound-intercept.md` §3/§5) —
the same `tls_sw_splice_read` primitive the outbound *return* uses, applied to
the request direction. The server-speaks-first response leg (re-encrypt the
server's reply onto leg C's kTLS-TX) reuses the outbound forward primitive
(`findings-egress-ktls-splice.md`); composing it into the inbound server shape
is part of the composed walking-skeleton gate (it was NOT exercised in the
inbound spike — `findings-inbound-intercept.md` § "What was NOT tested").

The structural facts pinned by the evidence and binding on DELIVER:

1. **Transparent intercept** routes the workload's traffic to the agent, by
   direction:
   - **Outbound**: the workload's `connect()` is rewritten to the agent's leg-F
     listener via the `cgroup/connect4`-rewrite shape (proven in
     `findings-userspace-relay.md`), reusing the established `cgroup_connect4`
     program family.
   - **Inbound**: the connection aimed at the server workload's logical address
     is **TPROXY**-redirected to the agent's `IP_TRANSPARENT` leg-C listener
     (proven in `findings-inbound-intercept.md` §1: `nft` prerouting
     `tproxy to <agent>` + `ip rule fwmark` + `ip route local … table` triple).
     **Original-destination recovery is `getsockname()`** on the accepted leg-C
     socket — NOT `SO_ORIGINAL_DST` (under TPROXY the kernel keeps the
     intercepted socket's local address as the original destination;
     `findings-inbound-intercept.md` § "Mechanics that mattered" #1). The
     recovered original destination selects the **server** workload's
     `AllocationId` → its held SVID.
   The two intercept mechanisms are the mirror image of each other
   (`connect4`-rewrite for the active side, TPROXY for the passive side); both
   reach the same agent-light proxy topology. The `IP_TRANSPARENT` listener and
   the `nft`-TPROXY setup need `CAP_NET_ADMIN` — the host-side agent runs
   privileged for intercept setup; the workload holds nothing and is
   unprivileged (`findings-inbound-intercept.md` § "Mechanics" #1 / "Design
   implications" #5).
2. **Lossless handshake-window capture is userspace.** The agent owns leg F and
   leg B; it `recv()`s the workload's pre-arm plaintext into a buffer (trivially
   lossless), handshakes on leg B, arms kTLS, and flushes the captured bytes as
   the first application_data (rec_seq starts at 0 on TLS 1.3, proven gapless in
   `findings-lossless-hybrid.md`).
3. **Forward steady state is agent-idle**: leg F's RX is egress-redirected into
   leg B's kTLS TX by a hand-rolled `sk_skb/stream_verdict` (`flags=0`).
4. **Return steady state is agent-light**: leg B is a **plain kTLS-RX socket with
   NO sockmap/psock**; the agent drives a bounded `splice(legB → pipe → legF)`
   pump (~1 splice per record). Putting a psock verdict on leg B's RX is
   forbidden — it both fights kTLS RX (`ConnectionAborted`) and forecloses the
   agent-idle path (`tls_sw_read_sock` `-EINVAL`); the splice pump is the chosen
   shape.
5. **The SOCKMAP-insert-before-`TCP_ULP "tls"`** ordering invariant holds for
   leg B's sockmap membership (forward direction); a Tier-3 test pins
   `tls-ULP-after-sockmap == EINVAL`.
6. **Control records** (`NewSessionTicket`, KeyUpdate, renegotiation) are handled
   by reusing `ktls::KtlsStream`'s control-message loop, not raw `setsockopt`.
7. **The agent holds the leaf key** (read via `IdentityRead`, held by the
   control-plane `IdentityMgr` per CLAUDE.md / ADR-0067); **the workload holds
   NOTHING** — no cert, no key, an ordinary plaintext socket to the agent. This
   holds for BOTH the client workload (leg F) and the server workload (leg S).
8. **Inbound server-side mutual-TLS** (`findings-inbound-intercept.md` §2): the
   agent's rustls `ServerConfig` presents the **server** workload's SVID
   (selected by the recovered original destination → `AllocationId` →
   `IdentityRead::svid_for`) AND requires-and-verifies the **client**'s
   presented SVID chains to `IdentityRead::current_bundle()` via
   `WebPkiClientVerifier`. A missing client cert (`nocert`) or a client cert
   from an untrusted CA (`wrongca`) is **fail-closed**: the handshake is
   rejected with a distinct reason, no plaintext is spliced to the server
   workload (`findings-inbound-intercept.md` §4 — the two rejections carry
   distinct reasons, proving the verifier genuinely evaluates the chain).
9. **Inbound steady state is agent-light** (`findings-inbound-intercept.md`
   §3/§5): leg C is a plain (no-psock) kTLS-RX leg; the agent drives a bounded
   `splice(legC → pipe → legS)` pump, `tls_sw_splice_read` decrypting each
   record into clean plaintext — the same return-direction primitive the
   outbound half uses, applied to the request direction. Two server-config
   mechanics carry over from the spike and bind on DELIVER: suppress
   `NewSessionTicket` (`send_tls13_tickets = 0` — a post-handshake ticket record
   hits `-EIO` on raw kTLS-RX) and read `peer_certificates()` for the
   fail-closed guard BEFORE `dangerous_extract_secrets()` consumes the
   connection (`findings-inbound-intercept.md` § "Mechanics" #3/#6).

### What this does NOT do — authentication vs. authorization (scope boundary)

This proxy does **authentication** (the peer presents a valid SVID chaining to
the trust bundle) and **encryption** (TLS 1.3 records on the wire, in-kernel
kTLS). It does **NOT** do **authorization** — deciding whether *this* workload is
*allowed* to talk to *that* peer. That is a **separate, already-tracked
subsystem** and is deliberately out of #26's scope:

> **The honest v1 security claim (read this before citing "#26 gives you
> mTLS").** v1 #26 provides **chain-to-bundle transport authentication +
> encryption, with NO intended-peer identity pinning**. Both directions verify
> only that the peer's certificate **chains to the trust bundle** — i.e. that
> the peer is *some* valid cluster workload. v1 does **NOT** verify the peer is
> the *intended* destination. A routing bug, a VIP collision, or a malicious
> in-cluster endpoint that presents a **valid-but-unintended SVID** (one that
> chains to the bundle but is not the workload the client meant to reach) is
> **NOT prevented in v1**. That gap is closed by **intended-peer SAN-matching**,
> which is the [#178](https://github.com/overdrive-sh/overdrive/issues/178)
> **upgrade** — NOT a v1 prerequisite. Until #178 lands, documentation and tests
> MUST NOT describe the wrong-but-valid-peer case as "protected" /
> "prevented" / "pinned"; the only honest v1 phrasing is "authenticated as a
> valid cluster workload, encrypted end-to-end." The wrong-but-valid-peer
> negative test stays `#[ignore]`-gated on #178 (§ Enforcement, "Authn-only
> boundary").

- **Authorization is enforced at the BPF-LSM `socket_connect` hook**
  ([#27](https://github.com/overdrive-sh/overdrive/issues/27) "[2.5] BPF LSM
  programs … `socket_connect` … per-workload MAC"), fed by compiled
  **`policy_verdicts`**
  ([#38](https://github.com/overdrive-sh/overdrive/issues/38) "[3.1] Regorus →
  verdict compilation → `policy_verdicts` → BPF map hydration"; related
  [#49](https://github.com/overdrive-sh/overdrive/issues/49) job-security → BPF
  map). The connect-time allow/deny decision lives there, **not** in the mTLS
  proxy. The proxy MUST NOT embed a policy engine, evaluate Regorus, or read
  `policy_verdicts`; doing so would duplicate #27/#38's verdict and create two
  sources of authorization truth that can drift.
- **Expected-destination identity pinning** (verifying the authenticated peer is
  the *intended* one, not merely *some* cluster workload chaining to the bundle)
  depends on **east-west SPIFFE-ID resolution**
  ([#178](https://github.com/overdrive-sh/overdrive/issues/178) "Native
  east-west SPIFFE-ID resolution", which is *downstream* of #26 — it "terminates
  in SPIFFE mTLS via sockops (#26)"; the VIP path is
  [#61](https://github.com/overdrive-sh/overdrive/issues/61)). #178 supplies the
  *expected peer* SPIFFE identity; the proxy then SAN-matches the authenticated
  peer against it. **v1 #26 enforces chain-to-trust-bundle authn only**
  (fail-closed on absent/invalid SVID); the expected-destination SAN-match lands
  **with #178**. The contract reserves an OPTIONAL `expected_peer` input + a
  `PeerIdentityMismatch` error variant so the SAN-match wires the moment #178
  supplies it, with v1 leaving it unset (authn-only).

The boundary is **intentional and documented, not a silent gap**: a connection
that authenticates here can still be *denied* by #27's LSM hook before it is
ever established, and can later be *identity-pinned* to its expected destination
by #178. This ADR closes the authn + encryption half of the wire-security story;
#27/#38 own authorization; #178 owns expected-destination identity.

A **new driven port** (`MtlsEnforcement`, named as a DESIGN decision below — the
exact signature is pinned in `brief.md` / the feature-delta, NOT improvised by
the crafter) carries the proxy's per-connection enforcement surface. The existing
`Dataplane` port does **not** fit — it models map writes (policy/service/local-
backend), not per-connection socket operations (intercept, handshake-drive,
kTLS-arm, splice-pump). The new port has a host adapter (over
sockops/sk_msg/sockmap/kTLS/`splice`/`cgroup_connect4`) consuming `IdentityRead`,
and a `Sim` adapter for DST (the sim/host split, `.claude/rules/development.md` §
"Port-trait dependencies").

The adapter home (OQ-2, user-decided 2026-06-12): **NO new crate.** The
`HostMtlsEnforcement` host adapter extends **`overdrive-dataplane`** (the
established `adapter-host` userspace eBPF crate that already hosts
`EbpfDataplane` — `unsafe` already allowed, `aya.workspace = true` already a dep,
`build.rs` for the `overdrive_bpf.o` object already present); the new kernel-side
sockops/`sk_skb/stream_verdict`/`cgroup_connect4`-mtls programs extend
**`overdrive-bpf`** alongside the existing `cgroup_connect4_service`/XDP programs
(one shared BPF object); the `Sim` adapter stays in `overdrive-sim`.
**`overdrive-host` is ruled out** — `src/lib.rs:21` is
`#![forbid(unsafe_code)]` and the proxy is irreducibly `unsafe` (raw
`setsockopt(TCP_ULP/TLS_TX/TLS_RX)`, `splice(2)`, BPF-fd plumbing). Extending an
existing dataplane crate that already satisfies every requirement (unsafe, aya,
BPF `build.rs`) beats a new crate "for now"; the concern-isolation trade — not
coupling the LB/service dataplane's compile to the `ktls`/`rustls` TLS stack — was
weighed and is the **revisit trigger**, not a v1 blocker.

## Alternatives Considered

### A1. In-band kTLS on the workload's OWN socket (the prior #26 v1)

The model whitepaper §7 named first: sockops detects → `pidfd_getfd` the
workload's own socket → handshake → kTLS arms **on the workload's socket** → agent
exits → the kernel carries crypto on one socket the workload owns.

- **Proven viable** (`findings.md`: A/B/C/D all confirm the mechanism). It
  uniquely wins two things: **restart-survival** (kTLS state is socket-owned; an
  in-flight session survives an agent restart iff the workload owns the fd + the
  link/maps are bpffs-pinned) and **1-socket density** (no proxy second socket).
- **Rejected for v1 on two grounds the evidence makes decisive**:
  1. **Not lossless.** There is no lossless client-speaks-first path on
     runtime-loadable BPF — foreclosed three ways (no `sk_msg` HOLD; source-TX-
     bypass RST on redirecting the live socket; lossless capture requires a
     proxy). v1 in-band would have to ship the **lossy DROP-then-RESET** gate
     under a *named server-speaks-first assumption* — a real, operator-visible
     limitation.
  2. **Not universal.** It cannot serve guest-stack workloads at all (no host
     `struct sock`), so #222's separate proxy mechanism was unavoidable
     regardless. Keeping in-band means shipping **two** mechanisms; the proxy
     unifies to **one**.
- **Not in v1 scope** (restart-survival + density) — a post-v1 optimization
  tracked in **#231**. Should it ever be wanted, a follow-up would layer it for
  host-socket kinds where restart-survival matters, on top of the proxy default.
  v1 does not pursue it; #231 is the tracking issue for the deferred alternative.

### A2. In-band lossy DROP-RESET gate as the universal v1

Ship the spike-1-proven `sk_msg` DROP-until-armed gate (confidentiality-correct;
a pre-arm write is dropped → connection RESET) as the v1 enforcement for
host-socket kinds.

- **Confidentiality-correct** and the simplest in-band shape; proven on 7.0.
- **Rejected**: it is **lossy by construction** (drops pre-arm bytes, RESETs the
  connection if `drops > 0`), requires naming a server-speaks-first assumption
  that excludes SMTP/FTP/SSH-shaped protocols, and is still **not universal**
  (guest-stack unaddressed). The uniform lossless proxy supersedes it on every
  axis except 1-socket density — which the user deprioritised. The DROP-RESET
  gate's *confidentiality guarantee* (fail-closed, no cleartext before arm)
  carries forward into the proxy's intercept design (the workload never reaches
  the real peer un-intercepted), but the lossy gate itself is not shipped.

### A3. A permanent userspace-copy L4 proxy (the ztunnel shape)

The agent stays in the per-byte data path for the connection's life: `recv`
plaintext from leg F, `write` ciphertext to leg B, and the reverse — a classic
userspace copy loop.

- **Simplest to implement** (no BPF splice, no sockmap, no kTLS — plain rustls
  over two sockets) and trivially lossless.
- **Rejected**: it keeps a userspace proxy in the steady-state data path for
  every byte of every connection — exactly the ztunnel/Istio shape Overdrive's
  thesis rejects (whitepaper §7: "No userspace proxies in the data path";
  principle 2). The agent-light splice supersedes it: the evidence proves the
  steady-state byte movement can ride the kernel (agent-idle forward via
  sockmap-egress-redirect → kTLS-TX; agent-light return via zero-copy `splice`),
  so the agent leaves the per-byte path after the handshake window. The userspace
  copy loop is retained only as the **honest fallback baseline** if a deployment
  cannot use the kernel splice (e.g. a kernel below the splice/sockmap floor —
  not a concern on the pinned 6.18 appliance, ADR-0068).

### A4. Cilium-style out-of-band auth + separate encryption (WireGuard/IPsec)

The documented fallback if in-band kTLS had been disproven: authenticate
out-of-band, encrypt with a separate WireGuard/IPsec session (auth-session ≠
data-session).

- **Rejected**: the spike did NOT select this fallback — in-band kTLS is alive on
  7.0 (`findings.md`). It also breaks the **auth-session == data-session**
  property (J-SEC-003's social/emotional core: "the SAME TLS 1.3 session that
  authenticated carries the data"), which the proxy preserves (the rustls
  handshake's extracted secrets ARE the kTLS keys on leg B). Out of scope.

## Consequences

### Positive

- **One mechanism for all workload kinds.** Process/exec, WASM, microVM,
  unikernel all route through the same agent-light L4 proxy. #222 folds into #26;
  whitepaper §7's two-mechanism split collapses to one. Maintainability and
  test surface both shrink to a single driven port + DST equivalence harness.
- **Lossless for every kind, including guest-stack and client-speaks-first.** The
  handshake-window capture is a userspace buffer (trivially lossless); no dropped
  pre-arm bytes, no RESET. The named server-speaks-first assumption the in-band v1
  required is **gone**.
- **No kernel patch.** Every primitive (sockops, `cgroup_connect4`, sockmap
  egress-redirect, kTLS, `tls_sw_splice_read`) is in-tree at the pinned 6.18
  floor. The out-of-tree write-block patch that lossless in-band would have needed
  is not required.
- **J-SEC-003 identity model preserved.** The agent presents the workload's own
  held SVID (read via `IdentityRead`, never minted/cached by this feature),
  verifies the peer against the trust bundle, auth-session == data-session
  (rustls secrets → leg B kTLS), workload holds nothing. The wire carries TLS 1.3
  records, provable by `tcpdump`.
- **Agent-light steady state.** Forward is agent-idle (kernel splice); return is
  agent-light zero-copy `splice` (~1/record). No userspace per-byte copy in
  either direction — strictly better than the ztunnel baseline.

### Negative

- **Two sockets per connection** (leg F + leg B), vs the in-band model's one. A
  density cost the user accepted.
- **The agent does per-connection work**: a rustls handshake at connection setup
  plus a return-direction `splice` pump (~1 splice + readiness `ppoll` per TLS
  record) for the connection's life. The agent is *light*, not *out* — it stays
  scheduled per-record on the return path. (Forward is genuinely idle.)
- **No restart-survival in v1.** The agent owns both legs and the kTLS state; an
  agent restart drops in-flight sessions (they re-handshake on reconnect).
  Restart-survival is the in-band model's unique win; it is **not in v1 scope**
  (the accepted proxy trade, A1) — a post-v1 optimization tracked in **#231**.
- **Transparent-intercept complexity, both directions.** Outbound: every
  workload's `connect()` is rewritten to the agent's leg-F listener
  (`cgroup_connect4`). Inbound: every connection aimed at a server workload's
  logical address is TPROXY-redirected to the agent's `IP_TRANSPARENT` leg-C
  listener, and original-destination recovery (`getsockname`) selects the server
  SVID. The agent manages BOTH intercept lifecycles robustly — the throwaway
  outbound spike harness hit an intercept-lifecycle RST that DELIVER must
  engineer around (`findings-userspace-relay.md` Unknown 3, a harness
  limitation); the inbound spike was clean and bounded but loopback-only, so the
  netns/veth topology re-proving is a DELIVER obligation
  (`findings-inbound-intercept.md` § "What was NOT tested").
- **Inbound is proven COMPOSED end-to-end (one direction); bidirectional
  round-trip is a remaining gap.** `findings-inbound-intercept.md` (increment-i
  §2, *ok* mode) demonstrated the full inbound flow composed — TPROXY intercept →
  orig-dst recovery → server-side mutual-TLS (verifies C's client SVID) →
  kTLS-RX arm → agent-light splice-to-S, with fail-closed on `nocert`/`wrongca`.
  What is NOT yet exercised is the **response leg** (re-encrypt the server's
  reply onto leg C's kTLS-TX); the inbound spike drove only the request
  direction (C→S) (`findings-inbound-intercept.md` § "What was NOT tested"). The
  forward kTLS-TX primitive that leg needs is proven separately
  (`findings-egress-ktls-splice.md`); composing it into the inbound server shape
  for a full round-trip is gap #2 of the Slice-00 walking-skeleton scope, not a
  doubt about the mechanism.
- **Three narrow composition gaps remain; closing them is the walking-skeleton
  gate (NOT optional).** The mechanism is spike-verified — the primitives in
  isolation (forward splice, return splice, kTLS arm, arming order) AND the
  inbound flow composed end-to-end in one direction (increment-i §2). What is NOT
  yet demonstrated is (1) the **outbound** path composed in ONE flow — increment-e
  proved outbound intercept + pre-arm capture + handshake-window flush and
  increment-f proved the steady-state egress splice, but on SEPARATE harnesses
  (increment-f removed the `cgroup_connect4` intercept to isolate the splice;
  increment-e's steady-state was blocked by a *throwaway-harness
  intercept-lifecycle RST — a harness limitation, NOT a kernel finding*, later
  superseded by increment-f's clean-harness steady-state proof); (2)
  **bidirectional steady-state round-trip** in either direction; (3) the **real
  netns/veth topology with cgroup-isolated workloads** (every spike was loopback +
  sibling processes). DELIVER MUST land a **composed Tier-3 acceptance test as its
  FIRST slice (Slice 00, walking skeleton)** before any other slice — wiring the
  proven pieces into ONE bidirectional flow in the real netns/veth topology,
  closing gaps 1–3, under BOTH normal AND traced/delayed timing, asserting zero
  RST post-arm. This is an integration gate, not a "prove-the-mechanism" gate, and
  it supersedes the old in-band walking skeleton. (See *Consequences* →
  *Composition gate* below and the feature-delta DESIGN handoff.)
- **J-SEC-003 / slices 00–05 re-grounding.** The DISCUSS-wave job and slices were
  authored on the in-band "agent fully out, restart-survivable, kTLS on the
  workload's own socket" model. Those properties no longer hold in v1. This is a
  back-propagation flagged for the product-owner in
  `design/upstream-changes.md` — the architect does NOT edit `jobs.yaml`.

### Sensitivity / trade-off points (ATAM)

- **Trade-off point**: the **return-direction splice pump** (outbound) and the
  **inbound deliver pump** (both are `splice`-on-a-plain-kTLS-RX-leg pumps;
  `findings-splice-return.md` / `findings-inbound-intercept.md`) affect
  *performance* (agent scheduled per-record) AND *reliability* (the agent must
  keep the pump live for the connection's life — a crashed/stranded pump
  strands the affected direction). The agent-light splice return is the design;
  a fully-agent-idle bidirectional return is a non-goal, not pursued — NO kernel
  patch is or will be required.

  **Pump supervision policy (F6 — binding on DELIVER; not just observation).**
  `liveness()`/`PumpLiveness::Stalled` is the *observation* surface; the policy
  that consumes it is pinned here:
  - **Progress metric** — bytes-spliced-this-connection, a monotonic counter the
    adapter's pump task advances on every `splice_out`. The pump is *making
    progress* iff that counter advanced since the last observation OR the leg-C/B
    RX has no readable record pending (idle-but-ready is `Running`, not stalled).
  - **Stall threshold** — the pump is `Stalled { since }` when the
    bytes-spliced counter has NOT advanced for **`pump_stall_deadline`
    (default 30 s)** *while* the kTLS-RX leg has a readable record pending (i.e.
    bytes are waiting but the pump is not moving them). A purely-idle connection
    with no pending records is `Running`, never `Stalled` (no false positives on
    quiescent long-lived connections).
  - **Who reacts** — the **node-agent / worker** (D-MTLS-10), which already
    supervises per-connection lifecycle; it point-queries `liveness(&handle)` on
    its existing reconciler-tick cadence (SD-4 point-query, not a push stream in
    v1).
  - **Action — teardown + fail-closed reset** (chosen, justified): on observing
    `Stalled`, the worker calls `teardown(handle)` — the connection's legs are
    closed, both pumps stopped, kTLS/sockmap state reclaimed. The workload's
    connection drops; a client-speaks-first / request-retry protocol
    re-handshakes on reconnect (the same recovery v1 already relies on for
    agent restart). **Why teardown, not reconnect-or-degrade**: a stranded pump
    means the steady-state byte movement for that direction is broken with no
    in-band repair (the kTLS session's record sequence cannot be resumed from a
    foreign process); silently *degrading* to a userspace copy loop would
    re-enter the per-byte path the whole design rejects (A3); *refusing new
    connections* punishes healthy traffic for one stranded flow. Teardown +
    fail-closed reset is the minimal, confidentiality-safe action — it never
    leaks cleartext and bounds the blast radius to the one stranded connection.
  - **Telemetry** — a `mtls.pump.stalled` counter (per allocation) and a
    `mtls.pump.teardown_on_stall` event; the bytes-spliced high-water mark and
    last-progress timestamp are the operator's window into pump health (the
    feature has no CLI verb — these are the only observability surface).
  - **Acceptance test** — a Tier-3 test injects a stalled pump (pause the
    `splice` task while a record is pending on the RX leg), asserts
    `liveness(&handle)` transitions to `Stalled` within `pump_stall_deadline`,
    asserts the worker's supervision loop tears the connection down, and asserts
    no fd/sockmap/kTLS leak after (re-querying `liveness` returns `Gone`). The
    sim adapter models the same observable transition (scriptable `Stalled` →
    worker teardown → `Gone`) for the DST equivalence harness.
- **Sensitivity point**: leg B must be a **plain kTLS-RX socket (no psock)** for
  the return `splice` to work. Adding any sockmap verdict to leg B's RX (e.g. to
  also kernel-splice the return) breaks it (`-EINVAL` / `ConnectionAborted`).
  This is an enforceable architectural invariant (a Tier-3 test target).

### Resource & robustness constraints (binding on DELIVER)

The lossless handshake-window capture (Decision §2) buffers the workload's
pre-arm plaintext in userspace. That buffer is **load-bearing but must be
bounded** — a workload can stream into leg F while the peer handshake on leg B
stalls, so an unbounded buffer is a denial-of-service surface (one connection can
exhaust agent memory). The `MtlsEnforcement` contract therefore pins, as part of
the `enforce` precondition surface:

The concrete v1 defaults (F7 — pinned, not "sensible defaults"; the acceptance
tests assert these values, not merely field existence). They are compile-time
defaults, NOT operator-tunable in v1 — operator-tunability of `MtlsLimits` is a
separate concern tracked in [#230](https://github.com/overdrive-sh/overdrive/issues/230):

- **Bounded pre-arm buffer** — `max_prearm_bytes = 256 KiB` (262 144 bytes) per
  connection. Rationale: comfortably covers a request-first protocol's first
  flight (an HTTP/2 SETTINGS + a large header block, a gRPC request, a Postgres
  startup packet are all ≪ 256 KiB) while the leg-B/leg-C handshake completes
  in single-digit milliseconds on loopback / same-node; it is two orders of
  magnitude below the per-connection memory a stalled peer could otherwise
  pin. Exceeding it is **fail-closed**: drop the buffer, reset leg F (outbound)
  / leg S (inbound), and return `MtlsEnforcementError::BufferLimitExceeded`
  (cause-distinct; never a generic `Io`/`Internal`). No cleartext egresses.
- **Handshake deadline** — `handshake_deadline = 5 s`. Rationale: a same-node /
  east-west mutual-TLS handshake completes in milliseconds; 5 s is a generous
  ceiling that distinguishes a genuinely-stalled or dead peer from normal
  variance without false-tripping under GC / scheduler jitter. Exceeding it is
  fail-closed → `MtlsEnforcementError::HandshakeTimeout`; the stalled peer
  cannot pin agent resources indefinitely.
- **Per-allocation in-flight connection limit** — `max_inflight_per_alloc = 128`
  concurrent pre-arm (not-yet-armed) connections per allocation. Rationale: a
  healthy workload arms each connection in milliseconds, so 128 concurrent
  *pre-arm* connections is far above any legitimate burst yet caps the
  amplification a single workload opening many stalled connections can inflict.
  Over-limit is fail-closed (refuse the new intercept; the workload's
  `connect()` fails, no cleartext) → `MtlsEnforcementError::InFlightLimitExceeded`.

**Expected resource budget (F7 — the operator's exhaustion-reasoning surface):**
- **Per pre-arm connection**: ≤ `max_prearm_bytes` (256 KiB) buffer +
  the two leg fds + one `splice` pipe fd ≈ **3 fds + ≤ 256 KiB** while
  pre-arm; once armed the buffer is flushed and freed, so a *steady-state*
  connection holds **~3 fds + ~16 KiB** of pipe/kTLS bookkeeping (no app
  buffer).
- **Per allocation**: at most `max_inflight_per_alloc` (128) connections in the
  pre-arm window → **≤ 128 × 256 KiB = 32 MiB** worst-case pre-arm memory and
  **≤ 128 × 3 = 384 fds** in-flight; steady-state established connections are
  bounded by the workload's own connection count (the proxy adds ~3 fds each,
  not the pre-arm buffer).
- **Per node**: the agent's fd budget = Σ over allocations of (in-flight pre-arm
  + established) × ~3 fds. The node sizes its `RLIMIT_NOFILE` against this; the
  in-flight ceiling makes the *pre-arm* contribution bounded and predictable
  (≤ `num_allocs × 128 × 3` fds), so the unbounded term is only legitimate
  established connections — the same fd pressure any L4 proxy carries.
- **Fail-closed cleanup is total.** On any limit/deadline trip the port owns the
  cleanup — the pre-arm buffer is dropped, the leg is reset, no sockmap/kTLS
  state leaks, no cleartext reaches the peer wire. Backpressure is *refuse*, not
  *queue-unbounded*.
- **Observability.** The buffer high-water mark, deadline trips, limit refusals,
  and in-flight counts are metrics/telemetry surfaces (the operator's only window
  into the proxy's resource health; the feature has no CLI verb).

### The intercept-recursion / agent-leg-B exemption (binding on DELIVER)

The workload's `connect()` is transparently rewritten (`cgroup_connect4`) to the
agent's leg-F listener; the agent then dials **leg B** to the real peer. Leg B's
own `connect()` MUST NOT be re-intercepted by the same `cgroup_connect4` program
— that would recurse infinitely (every agent dial intercepted, dialing again,
…). The exemption mechanism is **pinned, not left implicit**:

- The agent's own outbound sockets carry a **narrowly-scoped bypass** — either an
  `SO_MARK` socket mark the `cgroup_connect4` program checks-and-skips, OR
  **cgroup scoping** so the agent's egress is not under the workload
  `cgroup_connect4` attach subtree. The attach boundary is the existing
  `cgroup_connect4_service` precedent (the program is attached to the *workload*
  cgroup subtree, not the agent's).
- **Two Tier-3 obligations, both required**: (a) the agent's leg-B dial is NOT
  re-intercepted (no recursion); AND (b) the workload CANNOT self-exempt — the
  bypass is not a hole a workload can set on its own sockets to escape interception
  (the `SO_MARK` value / cgroup membership is agent-private, unreachable from the
  workload). A bypass that the workload can replicate would be an
  authentication-evasion vulnerability, not a convenience.

## Enforcement

- **Architectural rule (ArchUnit-style, Rust)**: `overdrive-dataplane` (userspace
  proxy syscalls) and `overdrive-bpf` (kernel-side programs) own the kernel/eBPF
  surface for the proxy path — consistent with `EbpfDataplane` already living in
  `overdrive-dataplane`; sockops/sk_msg/sockmap/kTLS/`splice` syscalls for the
  proxy appear nowhere else. The dst-lint gate (`xtask/src/dst_lint.rs`, ADR-0003)
  keeps these off any `core`-class compile path. The port trait lives in
  `overdrive-core` (no I/O); the `HostMtlsEnforcement` host adapter extends
  `overdrive-dataplane` (`adapter-host`), its kernel programs extend
  `overdrive-bpf` (per OQ-2, user-decided 2026-06-12).
- **Earned-Trust probe (mandatory, principle 12)**: the host adapter ships a
  `probe()` specified in the design — at the composition root, "wire then probe
  then use": verify the kTLS arm round-trips (a sentinel handshake + a
  `tls_sw_splice_read` of one record on a loopback leg) and the sockmap
  egress-redirect fires (a sentinel byte F→B emerges encrypted) BEFORE the proxy
  is declared usable; on probe failure the node refuses to start with a structured
  `health.startup.refused` event. This exercises the specific substrate lies the
  spikes catalogued (sockmap-insert-before-ULP ordering; the kTLS-RX-no-psock
  invariant; the egress-flag-not-ingress-flag invariant).
- **Composed walking-skeleton gate (the FIRST DELIVER slice, Slice 00,
  BLOCKING)**: a composed Tier-3 acceptance test — real `cgroup_connect4`
  intercept → workload pre-arm write → leg-B handshake → kTLS arm → **post-arm
  bidirectional multi-record transfer with NO RST** — repeated under BOTH normal
  AND traced/delayed timing. This is an **integration gate, not a
  "prove-the-mechanism" gate**: the mechanism is spike-verified (the primitives in
  isolation AND the inbound flow composed end-to-end in increment-i §2). Slice 00
  closes the three remaining composition gaps — (1) the OUTBOUND path composed in
  ONE flow (increment-e and increment-f proved its pieces on separate harnesses;
  increment-e's steady-state RST was a *throwaway-harness intercept-lifecycle
  limitation, NOT a kernel finding*, superseded by increment-f's clean-harness
  steady-state proof); (2) bidirectional steady-state round-trip; (3) the real
  netns/veth topology with cgroup-isolated workloads (the spikes were loopback +
  sibling processes). It MUST pass before any other DELIVER slice lands. It
  supersedes the old in-band walking skeleton.
- **Tier-3 invariants (outbound)** pinned as tests: `tls-ULP-after-sockmap ==
  EINVAL`; forward redirect uses `flags=0` (egress) not `BPF_F_INGRESS`; leg B
  carries no psock for the return path; `tcpdump` shows TLS 1.3 records and zero
  cleartext on the peer-facing wire; the handshake-window capture is lossless
  (no dropped pre-arm bytes).
- **Tier-3 invariants (inbound — F3)** pinned as tests, grounded strictly in
  `findings-inbound-intercept.md`: (a) **orig-dst recovery** — a TPROXY-
  intercepted connection to a server workload's logical address recovers the
  original destination via `getsockname()` on leg C (§1); (b) **server-mTLS
  fail-closed on `nocert`/`wrongca`** — a client presenting no cert or a cert
  from an untrusted CA is rejected with a distinct reason and NO plaintext is
  spliced to the server workload (§4); (c) **byte-exact plaintext to the server
  workload** — on a valid client cert, the server workload reads the exact
  request bytes as plaintext while the client-facing leg carries only TLS `0x17`
  app_data records (§2/§3); (d) **agent-light** — `strace` shows the agent moves
  the inbound payload via `splice`/`ppoll` only, zero per-byte
  `read`/`write`/`recv`/`send` of the payload (§5); leg C carries no psock on
  its RX (same plain-kTLS-RX invariant as the outbound return). The inbound
  intercept (`nft` TPROXY + `IP_TRANSPARENT`) and the server-side programs
  extend `overdrive-bpf`/`overdrive-dataplane` per OQ-2.
- **Resource-limit invariants (F4) pinned as tests**: a pre-arm stream exceeding
  the bounded buffer trips `BufferLimitExceeded` fail-closed (buffer dropped, leg
  reset, no cleartext); a stalled handshake trips `HandshakeTimeout`;
  over-the-per-allocation-limit concurrent intercepts are refused; and the
  cleanup path leaks no fd/sockmap/kTLS state (re-querying `liveness` shows
  `Gone`).
- **Intercept-exemption invariants (F5) pinned as tests**: the agent's leg-B dial
  is NOT re-intercepted (no recursion); a workload CANNOT self-exempt (the
  `SO_MARK`/cgroup bypass is agent-private and unreachable from the workload's
  sockets).
- **Authn-only boundary (F1/F5) pinned as tests**: a peer that does not chain to
  `IdentityRead::current_bundle()` is refused fail-closed
  (`PeerVerificationFailed`) — in BOTH directions (outbound: the dialed peer's
  server cert; inbound: the client's presented SVID, proven fail-closed on
  `nocert`/`wrongca` in `findings-inbound-intercept.md` §4). A **negative-test
  placeholder for the wrong-but-valid-peer case** (a peer that chains correctly
  but is NOT the intended destination) is reserved and stays
  **`#[ignore]`-gated on #178** — until #178 supplies the expected-peer
  identity, v1 authenticates chain-to-bundle only and the SAN-match
  (`PeerIdentityMismatch`) is not yet wired. **The docs/tests MUST NOT call the
  wrong-but-valid-peer case "protected" until #178 lands** — the honest v1 claim
  is chain-to-bundle authn + encryption, no intended-peer pinning (§ Decision,
  "The honest v1 security claim"). Authorization (allow/deny this connection) is
  #27's LSM hook, NOT this feature.

## References

- Spikes: `docs/feature/transparent-mtls-host-socket/spike/findings.md`,
  `findings-lossless-hybrid.md`, `findings-userspace-relay.md`,
  `findings-egress-ktls-splice.md`, `findings-ktls-rx-splice.md`,
  `findings-splice-return.md` (committed `353cdc52`); and the **inbound half**
  `findings-inbound-intercept.md` (increment-i, kernel 7.0 — the proof for the
  F3 inbound/passive path: TPROXY intercept + `getsockname` orig-dst recovery +
  server-side mutual-TLS + kTLS-RX decrypt + agent-light splice-to-server,
  fail-closed on `nocert`/`wrongca`).

  **Evidence durability — this decision rests on committed evidence, not
  assertion.** The committed findings docs above are the **foundation of record**:
  they survive a clean checkout and are the load-bearing source for every
  structural fact in *Decision*, every rejection in *Alternatives*, and every
  invariant in *Enforcement* (e.g. Decision §3/§4 ← `findings-egress-ktls-splice.md`
  / `findings-splice-return.md`; §5 ← `findings.md` Increment D; §6 ← `findings.md`
  Increment B; Alternatives A1's three-way foreclosure ← `findings-lossless-hybrid.md`
  / `findings-userspace-relay.md` / `findings.md` Increment C). The per-element
  map lives in the feature-delta § "Wave: DESIGN / [REF] Proven-Mechanism
  Traceability". The throwaway probe code under `spike-scratch/increment-{a..h}/`
  is **gitignored, never promoted, and NOT load-bearing** — it may not survive a
  clean checkout and is a secondary convenience pointer only; cite the committed
  finding, never the probe dir.
- Research: `docs/research/dataplane/sockmap-redirect-live-socket-liveness-research.md`,
  `sockops-ktls-lossless-hold-bpf-only-research.md`,
  `ktls-rx-agent-light-relay-research.md`.
- Ports: `IdentityRead` (`overdrive-core/src/traits/identity_read.rs`, ADR-0067),
  `Dataplane` (`overdrive-core/src/traits/dataplane.rs` — does NOT fit, see
  Decision).
- ADR-0068 (pinned 6.18 LTS kernel floor); ADR-0063 (built-in CA / leaf key);
  ADR-0067 (IdentityMgr / SVID lifecycle); ADR-0003 (crate-class taxonomy).
- Whitepaper §7/§8 (amended by this ADR).
- Authorization boundary (out of #26 scope; see *Decision* → "What this does NOT
  do"): [#27](https://github.com/overdrive-sh/overdrive/issues/27) (BPF-LSM
  `socket_connect` per-workload MAC), [#38](https://github.com/overdrive-sh/overdrive/issues/38)
  (Regorus → `policy_verdicts` → BPF map hydration), related
  [#49](https://github.com/overdrive-sh/overdrive/issues/49) (job-security → BPF
  map). Expected-destination identity (downstream of #26):
  [#178](https://github.com/overdrive-sh/overdrive/issues/178) (native east-west
  SPIFFE-ID resolution), VIP path [#61](https://github.com/overdrive-sh/overdrive/issues/61).
