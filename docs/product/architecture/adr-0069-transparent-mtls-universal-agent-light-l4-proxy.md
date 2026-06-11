# ADR-0069: Transparent mTLS via a universal agent-light L4 proxy

## Status

**Accepted** (2026-06-12). **Amends** whitepaper §7 ("Transparent mTLS — one
identity model, two enforcement mechanisms") and §8 ("Identity and mTLS"):
collapses the two-enforcement-mechanism framing (host-socket in-band kTLS #26 +
guest-stack L4 tap proxy #222) into ONE universal mechanism. **Supersedes** the
in-band sockops+kTLS-on-the-workload's-own-socket model as the v1 #26 enforcement
path; that model is retained as a tracked FUTURE OPTIMIZATION (see *Alternatives*
and the deferral surfaced to the product-owner).

Folds GH [#222](https://github.com/overdrive-sh/overdrive/issues/222) into GH
[#26](https://github.com/overdrive-sh/overdrive/issues/26). Job **J-SEC-003**.

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

Six Tier-3 spikes (5 follow-ups + the original) and three research docs, all on a
real 7.0 kernel (≥ the pinned 6.18 floor, ADR-0068), software kTLS, AES-256-GCM
TLS 1.3. Committed under `docs/feature/transparent-mtls-host-socket/spike/` and
`docs/research/dataplane/` at `353cdc52`. The findings are decision-grade and the
mechanism is fully de-risked. The load-bearing results:

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
restart-survival + 1-socket density**. Restart-survival becomes a future
optimisation; density (2 sockets/conn) is the steady-state cost.

## Decision

**Build a single universal transparent-mTLS enforcement mechanism: an
agent-light L4 proxy.** For every workload kind, the workload's outbound TCP is
**transparently intercepted** to a node-local agent leg; the agent drains the
workload's plaintext losslessly, performs the rustls TLS 1.3 handshake with the
real peer presenting the workload's held SVID (read via `IdentityRead`), arms
kTLS on the peer-facing leg, and then **hands the steady-state byte movement to
the kernel**: forward via an in-kernel sockmap EGRESS-redirect (agent-idle),
return via a `splice(2)` pump on a plain kTLS-RX leg (agent-light, zero-copy).

The proxy topology, per-connection:

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

The structural facts pinned by the evidence and binding on DELIVER:

1. **Transparent intercept** routes the workload's `connect()` to the agent. The
   default mechanism is the `cgroup/connect4`-rewrite shape (proven in
   `findings-userspace-relay.md`), reusing the established `cgroup_connect4`
   program family; TPROXY is the documented alternative. The exact intercept is a
   DELIVER-pinnable detail within this decision; both reach the same proxy
   topology.
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
   NOTHING** — no cert, no key, an ordinary plaintext socket to the agent.

A **new driven port** (`MtlsEnforcement`, named as a DESIGN decision below — the
exact signature is pinned in `brief.md` / the feature-delta, NOT improvised by
the crafter) carries the proxy's per-connection enforcement surface. The existing
`Dataplane` port does **not** fit — it models map writes (policy/service/local-
backend), not per-connection socket operations (intercept, handshake-drive,
kTLS-arm, splice-pump). The new port has a host adapter (over
sockops/sk_msg/sockmap/kTLS/`splice`/`cgroup_connect4`) consuming `IdentityRead`,
and a `Sim` adapter for DST (the sim/host split, `.claude/rules/development.md` §
"Port-trait dependencies").

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
- **Retained as a tracked FUTURE OPTIMIZATION** (restart-survival + density),
  *not* v1. A `health.startup`-style follow-up would layer it for host-socket
  kinds where restart-survival matters, on top of the proxy default. This is a
  deferral surfaced to the product-owner for an issue (it has no issue number
  yet — see *Consequences* and `design/upstream-changes.md`); it carries no
  hand-wavy forward pointer here.

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
  Restart-survival is the in-band model's unique win and is the named future
  optimisation (A1) — **deferred, pending a product-owner-approved GH issue**
  (no issue number exists yet; surfaced as a blocker, not written as a forward
  pointer).
- **Transparent-intercept complexity.** Every workload's `connect()` must be
  transparently rewritten to the agent leg (`cgroup_connect4` rewrite or TPROXY)
  and the agent must manage the intercept lifecycle robustly — the throwaway
  spike harness hit an intercept-lifecycle RST that DELIVER must engineer around
  (`findings-userspace-relay.md` Unknown 3, named as a harness limitation, not a
  kernel finding).
- **J-SEC-003 / slices 00–05 re-grounding.** The DISCUSS-wave job and slices were
  authored on the in-band "agent fully out, restart-survivable, kTLS on the
  workload's own socket" model. Those properties no longer hold in v1. This is a
  back-propagation flagged for the product-owner in
  `design/upstream-changes.md` — the architect does NOT edit `jobs.yaml`.

### Sensitivity / trade-off points (ATAM)

- **Trade-off point**: the **return-direction splice pump** affects *performance*
  (agent scheduled per-record) AND *reliability* (the agent must keep the pump
  live for the connection's life — a crashed pump strands the return path). A
  future agent-idle bidirectional splice would need a kernel patch
  (push-driven kTLS-RX→sockmap, or relaxing `tls_sw_read_sock`'s psock refusal) —
  out-of-tree, deferred.
- **Sensitivity point**: leg B must be a **plain kTLS-RX socket (no psock)** for
  the return `splice` to work. Adding any sockmap verdict to leg B's RX (e.g. to
  also kernel-splice the return) breaks it (`-EINVAL` / `ConnectionAborted`).
  This is an enforceable architectural invariant (a Tier-3 test target).

## Enforcement

- **Architectural rule (ArchUnit-style, Rust)**: the new `MtlsEnforcement` host
  adapter is the ONLY crate permitted to call sockops/sk_msg/sockmap/kTLS/`splice`
  syscalls for the proxy path; the dst-lint gate (`xtask/src/dst_lint.rs`,
  ADR-0003) keeps these off any `core`-class compile path. The port trait lives
  in `overdrive-core` (no I/O); the host adapter lives in an `adapter-host` crate.
- **Earned-Trust probe (mandatory, principle 12)**: the host adapter ships a
  `probe()` specified in the design — at the composition root, "wire then probe
  then use": verify the kTLS arm round-trips (a sentinel handshake + a
  `tls_sw_splice_read` of one record on a loopback leg) and the sockmap
  egress-redirect fires (a sentinel byte F→B emerges encrypted) BEFORE the proxy
  is declared usable; on probe failure the node refuses to start with a structured
  `health.startup.refused` event. This exercises the specific substrate lies the
  spikes catalogued (sockmap-insert-before-ULP ordering; the kTLS-RX-no-psock
  invariant; the egress-flag-not-ingress-flag invariant).
- **Tier-3 invariants** pinned as tests: `tls-ULP-after-sockmap == EINVAL`;
  forward redirect uses `flags=0` (egress) not `BPF_F_INGRESS`; leg B carries no
  psock for the return path; `tcpdump` shows TLS 1.3 records and zero cleartext
  on the peer-facing wire; the handshake-window capture is lossless (no dropped
  pre-arm bytes).

## References

- Spikes: `docs/feature/transparent-mtls-host-socket/spike/findings.md`,
  `findings-lossless-hybrid.md`, `findings-userspace-relay.md`,
  `findings-egress-ktls-splice.md`, `findings-ktls-rx-splice.md`,
  `findings-splice-return.md` (committed `353cdc52`).
- Research: `docs/research/dataplane/sockmap-redirect-live-socket-liveness-research.md`,
  `sockops-ktls-lossless-hold-bpf-only-research.md`,
  `ktls-rx-agent-light-relay-research.md`.
- Ports: `IdentityRead` (`overdrive-core/src/traits/identity_read.rs`, ADR-0067),
  `Dataplane` (`overdrive-core/src/traits/dataplane.rs` — does NOT fit, see
  Decision).
- ADR-0068 (pinned 6.18 LTS kernel floor); ADR-0063 (built-in CA / leaf key);
  ADR-0067 (IdentityMgr / SVID lifecycle); ADR-0003 (crate-class taxonomy).
- Whitepaper §7/§8 (amended by this ADR).
