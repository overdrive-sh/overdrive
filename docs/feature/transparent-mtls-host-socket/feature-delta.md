<!-- markdownlint-disable MD024 -->
# Feature Delta — transparent-mtls-host-socket (GH #26 · roadmap step 2.4)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Density**: `lean` + `ask-intelligent` (DISCUSS hard default) · **Re-grounded 2026-06-12 to ADR-0069** (the universal agent-light L4 proxy)

This is the single narrative artifact for the transparent-mtls-host-socket
feature. The DISCUSS `[REF]` sections below pin the WHAT and the acceptance
observables; the `## Wave: DESIGN` sections (from `## Wave: DESIGN / [REF]
Mechanism Decision` onward) pin the HOW. Both describe the **same mechanism**:
the **universal agent-light L4 proxy** locked by **ADR-0069** (2026-06-12). The
DISCUSS sections were re-grounded onto that mechanism — they no longer describe
the superseded in-band kTLS-on-the-workload's-own-socket model.

This feature is the **transparent-mTLS ENFORCEMENT mechanism** — the consumer
that finally **encrypts the wire** using the SVID that the CA mints
(J-SEC-001 / #28) and the IdentityMgr holds + exposes via the `IdentityRead` port
(J-SEC-002 / #35). The **job J-SEC-003 is authored this wave** (DISCUSS minted it —
there was NO DIVERGE wave for #26; see `wave-decisions.md` § Risk). The
**mechanism is pinned by ADR-0069**: ONE universal **agent-light L4 proxy** for
all workload kinds (process/exec in v1; guest-stack staged to #222), bidirectional
(outbound + inbound). The mechanism was settled empirically by 6 committed Tier-3
spikes (verdict: proxy; lossless on the stock pinned 6.18 kernel — no kernel patch,
no race window). DISCUSS pins the WHAT and the acceptance observables; the DESIGN
sections pin the contract (`MtlsEnforcement`, 4 methods, bidirectional) and the
F4–F7 limits/supervision/authn-boundary.

---

## Wave: DISCUSS / [REF] Feature Summary

**What**: For **host-socket workloads** (process via the exec driver in v1 — TCP
terminates in the HOST kernel), the Overdrive node agent turns a plaintext TCP
connection into a **kernel-encrypted TLS 1.3 session carrying the workload's own
SVID**, **bidirectionally**, with the **workload holding nothing** and the **agent
agent-light** (out of the per-byte path via in-kernel splice — not a userspace
proxy copying every byte). The mechanism (ADR-0069, the universal agent-light L4
proxy): the workload's connection is **transparently intercepted** to an
agent-owned leg (outbound: `cgroup_connect4`-rewrite to the agent's plaintext leg
F; inbound: TPROXY to the agent's `IP_TRANSPARENT` leg C); the agent drains the
pre-arm plaintext **losslessly** into a bounded userspace buffer and performs a
**rustls TLS 1.3 handshake on its own peer-facing leg** (leg B as client / leg C
as server) presenting the **held SVID** it reads via the `IdentityRead` port
(J-SEC-002/#35) and verifying the peer against the trust bundle → the negotiated
session keys install into **kTLS on the agent's peer-facing leg**
(`setsockopt TLS_TX/TLS_RX`) → steady state is handed to the kernel as
**agent-light** pumps that do NO userspace crypto but are NOT symmetric: the
DECRYPT/RX directions (outbound return, inbound deliver) are **zero-copy `splice`**
out of a plain kTLS-RX leg (`tls_sw_splice_read` decrypts on splice-out; ~1 splice
per record, no userspace plaintext copy); the ENCRYPT/TX directions (outbound
forward, inbound response) are a **bounded `read → write_all` COPY** into a kTLS-TX
leg (the kernel `tls_sw_sendmsg` encrypts each `write`; per-record `read`+`write`,
plaintext copied through a userspace buffer — NOT zero-copy, NOT agent-idle). A
`splice` INTO kTLS-TX loses records (the same `MSG_DONTWAIT` loss class the
abandoned sockmap egress redirect suffered — D-MTLS-13), so the encrypt directions
use a synchronous blocking `write_all`, not a splice. The peer-facing wire then
carries TLS 1.3 Application Data records, in-kernel.

**Why** (J-SEC-003): so design principle 3 — "every packet carries cryptographic
workload identity" — and principle 2 — "mTLS is in-kernel and undisableable" — are
**operationally true ON THE WIRE** for host-socket workloads, **provable by a
packet capture**. Identity is mintable (J-SEC-001) and held/readable (J-SEC-002),
but **nothing yet consumes it on the wire** — the promise is true in principle but
a packet capture would show cleartext. This feature is the **on-the-wire
ENFORCEMENT peer** of the mint(001)/hold(002) chain: 001 mints, 002 holds/reads,
**003 encrypts the wire**.

**Feature type**: Infrastructure / security primitive (D1) — no operator-facing
verb this phase; the HONEST observables are TEST-tier (wire capture on the
peer-facing leg, `ss -tie` showing the kTLS ULP, fail-closed negative test,
agent-light `strace`). Spans the dataplane (the intercept programs + kTLS arm +
the splice pumps), the node agent (rustls handshake + the per-connection
lifecycle), and the read of the `IdentityRead` port (`overdrive-core`).

**Scope boundary** — **RE-GROUNDED 2026-06-12 to ADR-0069** (the universal
agent-light L4 proxy). ADR-0069 amended whitepaper §7/§8: the prior "one identity
model, **two** enforcement mechanisms" framing collapses to **one** universal
mechanism. v1 host-socket #26 is **process/exec ONLY and BIDIRECTIONAL** (outbound
+ inbound). The original DISCUSS table read "host-socket ONLY; #222 a SEPARATE
feature; WASM in-scope" — corrected below:

| In scope (#26 v1) | Out of scope (referenced by issue #) |
|---|---|
| Process (exec driver) — TCP in host kernel — **the only workload kind in v1** | **Guest-stack** (microVM / unikernel, TCP in GUEST kernel) → **#222**, the **STAGED guest-stack intercept ADAPTER of the ONE universal proxy** (feeds the same `MtlsEnforcement` port from a tap/TPROXY/TC source) — **NOT a separate mechanism** (ADR-0069 folded #222 into #26) |
| **BIDIRECTIONAL**: outbound (client, `cgroup_connect4` intercept) **+** inbound (server, TPROXY intercept → server-mTLS → splice-to-server) | **WASM as a distinct mTLS path** → NONE in v1 (only `ExecDriver` exists); WASM host-socket workloads are **auto-covered by the same proxy** when a WASM driver lands — a separate roadmap concern |
| Authentication (chain-to-trust-bundle) + encryption (TLS 1.3 / kTLS), both directions, fail-closed | **Authorization** (allow/deny who-may-connect-to-whom) → BPF-LSM `socket_connect` **#27** fed by `policy_verdicts` **#38** (related **#49**) — a SEPARATE subsystem the proxy MUST NOT duplicate |
| | **Intended-peer identity pinning** → **#178** (east-west SPIFFE-ID resolution; VIP path **#61**). v1 = chain-to-bundle authn **only**, NO intended-peer pinning (a valid-but-unintended SVID is NOT prevented; not "protected" until #178) |
| | In-place SVID rekey / TLS 1.3 KeyUpdate → **#229** (v1 = teardown+reconnect); Certificate-rotation workflow → **#40** (depends on #39); Revocation (CRL/OCSP) → Phase 5; Multi-node transparent mTLS → OUT of v1 scope (Phase 1 is single-node); in-band restart-survival + 1-socket density → out of v1 scope, a post-v1 optimization tracked in **#231** (the proxy trade, ADR-0069 A1) |

**Evidence base**: the **6 committed Tier-3 spike findings** under
`docs/feature/transparent-mtls-host-socket/spike/` (the mechanism settled
empirically on a real 7.0 kernel — verdict: proxy, lossless, no kernel patch),
**ADR-0069** (the universal agent-light L4 proxy decision; folds #222 into #26),
`docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`
(the host L4 transparent-mTLS proxy recommendation; host-socket vs guest-stack
taxonomy), `.../sockops-mtls-ktls-installation-comprehensive-research.md`
(rustls→kTLS mechanics), **ADR-0068** (pinned 6.18 LTS kernel; KeyUpdate
kernel-ready / userspace-blocked → #229), whitepaper §7/§8/§6 addendum, vision
principles 2+3. Consumes the shipped `IdentityRead` port + `Arc<IdentityMgr>` (#35)
and the `Ca` hierarchy + leaf key (#28, ADR-0063 D9).

---

## Wave: DISCUSS / [REF] Persona

- **`sam-platform-security-engineer`** (Sam Okafor) — platform/security engineer
  who builds AND operates Overdrive's identity layer; has run SPIRE + Vault and
  hated it; threat-models by default; verifies with `openssl verify` / `tcpdump` /
  `ss -tie` rather than trusting the platform's word. SSOT:
  `docs/product/personas/sam-platform-security-engineer.yaml`. **Reused** from
  built-in-ca / workload-identity-manager (the persona already says "and future
  J-SEC-* jobs"); this wave adds `J-SEC-003` to `related_jobs` and a
  `j_sec_003_lens` — the **on-the-wire enforcement** lens (is the wire ACTUALLY
  TLS 1.3, not cleartext? is the auth-session the data-session? is kTLS armed on
  the agent's peer-facing leg — inspected via `ss -tie`, NOT `ss -K` on a
  workload socket the workload never owns? is the agent **agent-light** —
  splice/ppoll, no per-byte userspace copy — rather than a permanent userspace
  proxy? no cleartext on the peer wire before kTLS? fail-closed on wrong/absent
  SVID, in BOTH directions?). Per ADR-0069 the workload holds nothing and owns no
  TLS socket — the agent owns the peer-facing kTLS leg — so the verification
  surface is `ss -tie` on the agent's leg, not `ss -K` on the workload's socket;
  and the steady state is agent-light (zero per-byte copy), not "agent out of the
  data path". Same skeptical→confident security-review arc — no rich human
  emotional arc (D3 Lightweight).

---

## Wave: DISCUSS / [REF] JTBD One-liner

**J-SEC-003** — *"Transparently encrypt every host-socket workload's traffic with
its own SVID, in-kernel, both directions, via an agent-light L4 proxy the workload
can't disable — no sidecar, no cleartext on the peer wire."* `relates_to: J-SEC-002`.

> When a host-socket workload I run (process via the exec driver, TCP terminating
> in the host kernel) opens an outbound connection, OR an inbound connection
> arrives at one of my server workloads — and the platform already MINTS a
> forgery-proof SVID (#28) and HOLDS it readable via `IdentityRead` (#35) — but
> NOTHING yet consumes it on the wire, **I want** the platform to transparently
> intercept the connection to a node-local agent leg, perform the TLS 1.3
> handshake on the workload's behalf (rustls, on the agent's own peer-facing leg,
> presenting the held SVID), arm kTLS on the agent's leg, and hand steady state to
> the kernel (agent-light pumps — zero-copy `splice` on the decrypt/RX directions,
> a `read → write_all` copy on the encrypt/TX directions; no userspace crypto) —
> losslessly, with no
> cleartext on the peer wire before encryption is armed — **so** principles 2 + 3
> are operationally true ON THE WIRE for host-socket workloads (provable by wire
> capture showing TLS 1.3 records, both directions), the auth-session IS the
> data-session, encryption is in-kernel via kTLS, the agent is agent-light (not a
> per-byte userspace proxy), and a handshake against an absent/wrong SVID fails
> closed.

**Authored in DISCUSS (no DIVERGE wave) — see `wave-decisions.md` § Risk.** Full
job (functional/emotional/social dimensions + four forces) is in the SSOT
`docs/product/jobs.yaml` § J-SEC-003. Single dominant job → JTBD scoring is trivial
(one job, no competing candidates; opportunity is the unmet on-the-wire
enforcement that completes the mint→hold→enforce chain).

### Four-forces summary (drives the BDD scenario diversity below)

| Force | Statement | Scenario it seeds |
|---|---|---|
| **Push** | Identity is mintable (#28) + held/readable (#35) but NOTHING consumes it on the wire — principles 2/3 are aspirational until an enforcer ships. | Happy path: the wire carries TLS 1.3 records (US-MTLS-02/03). |
| **Pull** | transparent intercept → rustls handshake on the agent's leg presents the held SVID → kTLS arms on the agent's leg → kernel carries steady state (agent-light pumps: zero-copy `splice` on decrypt/RX, `read → write_all` copy on encrypt/TX; the kernel does the crypto, the agent runs no cipher) → peer wire carries TLS 1.3. | Happy path + "agent agent-light, in-kernel" (US-MTLS-03). |
| **Anxiety** | A sidecarless in-kernel mTLS mechanism is unshipped anywhere; will it actually compose on the real kernel without leaking cleartext? (Mitigated: 6 committed Tier-3 spikes settled it — proxy, lossless, no kernel patch, no race window; the 6.18 pin removes kernel anxiety; the composed walking skeleton (Slice 00) is the BLOCKING first gate.) | The composed walking skeleton (US-MTLS-00); fail-closed probe (US-MTLS-05). |
| **Habit** | Sidecar injection (Istio/Linkerd) / SPIRE Workload API (workload fetches its own SVID, userspace TLS); ztunnel's per-byte userspace proxy. | "Workload holds NOTHING / is identity-unaware; agent is agent-light not a userspace proxy" (US-MTLS-02/03). |

---

## Wave: DISCUSS / [REF] Brownfield Evaluation + Walking Skeleton (D2 — COMPOSED PROXY WS)

**This is a brownfield feature: a net-new ENFORCEMENT mechanism consuming
already-shipped seams (the held SVID + trust bundle via `IdentityRead`, the CA
hierarchy + leaf key). There is NO greenfield walking-skeleton proposal — and the
walking skeleton is the COMPOSED PROXY WS (Slice 00, BLOCKING): the Tier-3
spikes already settled the mechanism (verdict: proxy) — proving every primitive
in isolation AND the INBOUND flow composed end-to-end in one direction
(`findings-inbound-intercept.md` increment-i §2) — leaving three NARROW
composition gaps (outbound composed in one flow; bidirectional round-trip; real
netns/veth topology). The first DELIVER slice is a composed Tier-3 acceptance
test that wires the proven pieces into ONE bidirectional flow in the real
topology, closing those gaps — an integration gate, NOT a "prove-the-mechanism"
gate.**

| Already shipped (consumed, not rebuilt) | Where | This feature adds |
|---|---|---|
| `IdentityRead` port (`svid_for(&AllocationId) → Option<SvidMaterial>`, `current_bundle() → TrustBundle`) | `overdrive-core` (#35, J-SEC-002) | A READER of it: the agent reads the held SVID + bundle to drive the handshake |
| `Arc<IdentityMgr>` held-SVID map + hydrated `TrustBundle` | `overdrive-control-plane` (#35) | Nothing — it reads, never mutates the held set |
| `SvidMaterial` (cert PEM/DER + serial + spiffe_id + node-held `leaf_key`, ADR-0063 D9, redacted Debug) | `overdrive-core` (#28) | The material it presents in the rustls handshake (workload never holds it) |
| `Ca` hierarchy (Root → per-node Intermediate → workload SVID) + `trust_bundle()` | `overdrive-core` (#28, ADR-0063) | Verifies the peer against `current_bundle()` |
| Pinned 6.18 LTS kernel (in-kernel TLS 1.3 TX+RX + `CONFIG_NET_HANDSHAKE`) | ADR-0068 | The kernel the proxy runs on (no version anxiety) |
| eBPF + bpffs-pin discipline (`pinning = ByName`, `/sys/fs/bpf/overdrive/`) | `.claude/rules/development.md` | The `cgroup_connect4` outbound-intercept program + (link/map) pinning the proxy attaches (no sockops/`sk_skb` forward-redirect program — the forward path is an agent-light `splice` pump, D-MTLS-13) |
| `cgroup_connect4_service` program family + the connect4-rewrite shape | `overdrive-bpf` | The outbound transparent-intercept program (reuses the rewrite shape) |

**Walking skeleton (Slice 00, BLOCKING)**: a COMPOSED Tier-3 acceptance test on the
6.18 Lima/LVH kernel that proves the full proxy path holds end-to-end with **NO RST
post-arm**, for ONE composed flow per direction —
`transparent intercept (outbound cgroup_connect4-rewrite to leg F / inbound TPROXY
to leg C) → agent drains pre-arm plaintext losslessly → rustls TLS 1.3 handshake on
the agent's peer-facing leg (read held SVID via IdentityRead, verify the peer) →
kTLS arm on the agent's leg → post-arm bidirectional multi-record transfer →
tcpdump shows TLS 1.3 Application Data records on the peer-facing wire`, under BOTH
normal AND traced/delayed timing. **The observable IS the lossless, RST-free, TLS
1.3-on-the-peer-wire capture for both halves of one composed flow.**

**Named risk** (load-bearing): the spikes proved the primitives in isolation AND
the inbound flow composed end-to-end in one direction (`findings-inbound-intercept.md`
increment-i §2). Three NARROW gaps remain: (1) the OUTBOUND path composed in one
flow (its pieces were proven on SEPARATE harnesses — increment-f deliberately
removed the intercept to isolate the splice; increment-e's steady-state RST was
a *throwaway-harness intercept-lifecycle limitation, NOT a kernel finding*,
superseded by increment-f's clean-harness proof); (2) bidirectional steady-state
round-trip; (3) the real netns/veth topology. Slice 00 closes gaps 1–3 and is the
BLOCKING first slice — an integration gate, not a "prove-the-mechanism" gate.
Every downstream slice is additive on the composed walking skeleton.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate — Phase 1.5)

**Verdict: PASS — right-sized as ONE feature, sliced into 6 thin vertical cuts
(Slice 00 = the BLOCKING composed proxy walking skeleton).** Full signal table +
the DIVERGE-absence risk are in `wave-decisions.md` § Scope Assessment / § Risk.
Zero oversized signals fire (6 slices; 1 bounded context; composed WS thinned to
one flow per direction; ~7–9 days; 1 coherent outcome). The feature is already
correctly carved from the guest-stack ADAPTER (#222, staged), the
rekey/rotation/revocation concerns (#229/#40/Phase-5), and multi-node transparent
mTLS (OUT of v1 scope — Phase 1 is single-node); it is NOT split further.

---

## Wave: DISCUSS / [REF] Journey Visualization (ASCII flow + emotional arc + TUI/observable)

> Material honesty: there is NO operator GUI/CLI surface for this feature. The
> "TUI mockups" below are the **honest observable surfaces** — the wire capture
> (`tcpdump`), the socket-diag (`ss -tie`), and the test runner — which is what Sam
> actually looks at. CLI should feel like CLI; a security primitive's surface is
> its evidence, not a dashboard.

### Horizontal flow (the complete journey, all backbone activities)

```
[Trigger: host-socket    [A. Intercept to      [B. Handshake on        [C. Arm kTLS on the    [D. Prove on the wire]
 workload opens/accepts   the agent's leg]      the agent's leg,        agent's leg, kernel
 a TCP connection]                              present held SVID]       carries steady state]
        |                       |                       |                      |                      |
        v                       v                       v                      v                      v
  workload connect()      OUTBOUND: cgroup_     agent: rustls TLS 1.3   setsockopt(TCP_ULP    tcpdump on veth:
  (process/exec) or        connect4 rewrite     handshake on its OWN    'tls')+TLS_TX/RX on    TLS 1.3 App Data
  inbound arrival          to agent leg F;      peer-facing leg (B      the AGENT's leg;       records (0x17) on
                           INBOUND: TPROXY to   client / C server),     agent-light pumps,     the PEER leg, NOT
                           agent leg C +        presents the HELD SVID   NOT symmetric: RX =    cleartext; ss -tie:
                           getsockname orig-dst (read via IdentityRead), zero-copy splice out   kTLS ULP on the
                           agent drains pre-arm verifies peer vs        of kTLS-RX; TX = read   agent's leg
                           plaintext LOSSLESSLY trust bundle            ->write_all COPY into
                                                                        kTLS-TX (no crypto)
  Feels: (workload is     Feels(Sam): focused  Feels(Sam): focused      Feels(Sam):            Feels(Sam):
   identity-unaware,       'is the traffic      'does it present the     reassured 'ss -tie     CONFIDENT 'a capture
   holds nothing)          captured before it   WORKLOAD's identity,     shows kTLS on the      I ran shows TLS 1.3 —
                           reaches the peer      not the node's? both    agent's leg; the       principle made real,
                           un-encrypted?'        client AND server?'     kernel does crypto'    both directions'
  Artifacts: agent-       Artifacts: cgroup_    Artifacts: held         Artifacts: agent's     Artifacts: TLS 1.3
   owned leg (F / C)       connect4/TPROXY       SvidMaterial +          leg fd, kTLS           records on the peer
                           programs, recovered   TrustBundle (read       crypto_info            wire, ss -tie ULP
                           orig-dst, lossless    via IdentityRead)        (auth-session ==       state, strace
                           pre-arm buffer                                 data-session)          agent-light splice

  >>> LOSSLESS HANDSHAKE WINDOW: between A's intercept and C's kTLS arm, the agent captures the workload's
      pre-arm plaintext in a bounded USERSPACE BUFFER and flushes it as the first application_data after the
      handshake — fail-closed AND LOSSLESS for every protocol kind (no dropped pre-arm bytes, no RESET, no
      race window). No in-kernel gate, no write-block, no kernel patch. The buffer is bounded (F4 limits).

  >>> COMPOSED PROXY WS (Slice 00, BLOCKING): the ENTIRE flow above is composed end-to-end for ONE bidirectional
      flow per direction in the real netns/veth topology, with NO RST post-arm, under normal AND delayed timing
      — BEFORE any other slice lands. The spikes settled the mechanism (proxy) — primitives in isolation AND the
      INBOUND flow composed end-to-end (increment-i §2); Slice 00 closes the three narrow gaps (outbound
      composed in one flow; bidirectional round-trip; real netns/veth topology). Integration gate, not mechanism.
```

### Emotional arc (Sam — confidence-building pattern: skeptical → reassured → confident)

```
  skeptical/                                                                          confident/
  threat-modelling                                                                    relieved
       |                                                                                  ^
       |  "a sidecarless in-kernel mTLS                                                   |
       |   mechanism is unshipped — does it             reassured                         |
       |   compose? show me a packet capture"           incrementally                     |
       |                                       _____________________________             |
       |                                      /  composed WS proves one     \             |
       |     COMPOSED WS (Slice 00)          /   flow each way, no RST;      \   wire      |
       v____________________________________/    ss -tie shows kTLS; absent  \__capture___|
        Slice 00          Slice 01-02          /missing/untrusted creds       Slice 03-05
       (close the three   (transparent          fail closed                   (the agent is
        composition gaps  intercept + agent                                    agent-light;
        — inbound proven) handshake, both roles)                               tcpdump TLS 1.3,
                                                                               both directions)
```

No jarring transitions: confidence builds progressively (each slice's observable
is a small win Sam verifies himself with `tcpdump` / `ss -tie`); the error paths
guide to resolution (fail-closed + re-handshake-on-reconnect), not added anxiety;
the composed walking skeleton de-risks the peak-tension assumption (does the proxy
compose under a real intercept?) FIRST.

### Honest observable surfaces (the "what Sam sees")

```
+-- Wire capture (TEST tier — the headline observable) ----------------------+
| $ tcpdump -i overdrive-veth0 -X 'tcp port 8443'   # on the PEER-facing leg |
|   ... IP a.b.c.d.54321 > w.x.y.z.8443: ...                                 |
|     0x0000:  ... 1703 0304 00a7  ...   <-- TLS 1.3 App Data (0x17 03 03)   |
|   NO cleartext "GET /payments HTTP/1.1" anywhere in the capture            |
+----------------------------------------------------------------------------+

+-- Socket diag (kTLS ULP installed on the AGENT's leg) ---------------------+
| $ ss -tie                                                                  |
|   tcp ESTAB ... tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw    |
|     <-- the kTLS ULP is installed on the agent's peer-facing leg           |
+----------------------------------------------------------------------------+

+-- Fail-closed negative test (absent SVID / missing-or-untrusted peer) -----+
| flow with absent SVID (outbound) or nocert/wrongca (inbound) -> NO TLS 1.3 |
| App Data on the peer wire AND no cleartext -> fails closed (rustls aborts / |
| agent refuses); inbound delivers 0 bytes to the server workload            |
+----------------------------------------------------------------------------+

+-- Agent-light cost proof (strace) — per-direction, NOT symmetric ----------+
| DECRYPT/RX (outbound return, inbound deliver): only splice/ppoll,          |
|   ~1 splice per TLS record, zero userspace plaintext copy (zero-copy)      |
| ENCRYPT/TX (outbound forward, inbound response): per-record read+write     |
|   into kTLS-TX (a userspace COPY; NOT zero-copy, NOT splice). splice into   |
|   kTLS-TX loses records, so write_all is used.                             |
|   -> agent runs NO cipher in either direction (kernel kTLS does crypto);    |
|      it is NOT a userspace-crypto proxy (not the ztunnel shape)            |
+----------------------------------------------------------------------------+
```

---

## Wave: DISCUSS / [REF] Shared Artifacts Registry

Every `${artifact}` that flows across journey steps, with its single source of
truth and integration risk. (Companion to
`docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`
§ integration_validation.)

| Artifact | Source of truth | Consumers | Integration risk |
|---|---|---|---|
| **SVID / `SvidMaterial`** (cert + leaf key presented in the handshake) | J-SEC-002/#35's `Arc<IdentityMgr>` held map, read via `IdentityRead::svid_for(&AllocationId)` — **#26 is a READER, never an issuer** | the agent's rustls `ClientConfig`/`ServerConfig` (cert + leaf key for the handshake) | **HIGH** — if #26 re-issues or holds its own copy instead of reading the held map, it duplicates #35's source of truth and the credential drifts on rotation/drop. #26 MUST read via `IdentityRead`. |
| **`TrustBundle`** (peer-verification anchor) | J-SEC-002/#35's `IdentityRead::current_bundle()` (hydrated from `Ca::trust_bundle()`) | the agent's rustls peer-verification | **HIGH** — a stale bundle accepts a revoked/expired peer or rejects a valid one. Single source = the hydrated bundle behind `IdentityRead`; #26 never caches its own. |
| **agent's peer-facing leg** (the object kTLS arms on) | the agent's own dialed/accepted socket (leg B outbound / leg C inbound) — NOT the workload's socket | `setsockopt(TCP_ULP/TLS_TX/TLS_RX)`; the kTLS context (`icsk_ulp_data`); the forward/response `read → write_all` copy pumps (into kTLS-TX) + the return/deliver zero-copy splice pumps (out of kTLS-RX) | **HIGH** — kTLS lives on the AGENT's leg, not the workload's socket — that distinction IS the proxy model. The agent's kTLS-RX leg MUST be a PLAIN (no-psock) leg for the zero-copy splice pump to work (a psock fights kTLS RX); the kTLS-TX side carries no psock either. The workload holds a plaintext socket to the agent (leg F / leg S) and nothing else. |
| **kTLS `crypto_info`** (negotiated TLS 1.3 keys / IV / record seq) | the rustls handshake's extracted secrets — **auth-session == data-session** | `setsockopt(TLS_TX/TLS_RX)` on the agent's leg; observable as TLS 1.3 records (tcpdump on the peer leg) + the kTLS ULP (`ss -tie`) | **HIGH** — the SAME session that authenticated must carry the data; a mismatch breaks the "auth-session is the data-session" property the whole design rests on. |
| **`AllocationId` / `SpiffeId`** (which workload's identity to present) | the `SpiffeId` newtype + the allocation lifecycle (`spiffe://overdrive.local/job/<name>/alloc/<id>`, J-SEC-001 derivation); inbound, the SERVER workload's alloc recovered from the TPROXY orig-dst via `getsockname()` | the `IdentityRead` lookup key; the SAN the peer sees in the presented SVID | **MEDIUM** — the lookup key must match the held-map key (#35's `AllocationId`); a mismatch reads `None` and (correctly) fails the handshake closed. |

**Integration checkpoints** (validated before DESIGN handoff):

1. The agent reads SVID + bundle **only** via the `IdentityRead` port (no #26-local
   issuance, no #26-local cache) — single source of truth preserved across #35/#26.
2. The kTLS keys installed are the **rustls handshake's** extracted secrets
   (auth-session == data-session) — not a separately-negotiated session.
3. The pre-arm plaintext is captured **losslessly** in the agent's bounded
   userspace buffer and flushed after the handshake — no cleartext on the peer
   wire before encryption is armed, no dropped bytes, no RESET (the fail-closed
   AND lossless confidentiality property).

---

## Wave: DISCUSS / [REF] Story Map

**Persona**: Sam (platform/security engineer) · **Goal**: host-socket workloads
carry TLS 1.3 on the peer-facing wire with their own SVID, in-kernel, both
directions, agent agent-light (not a per-byte userspace proxy) — provable by a
packet capture, with no cleartext leak and fail-closed handshakes.

### Backbone (platform activities, left → right)

| A. Transparently intercept to the agent's leg | B. Handshake on the agent's leg (client + server) | C. Arm kTLS on the agent's leg, kernel carries steady state | D. Prove & guard on the wire |
|---|---|---|---|
| outbound `cgroup_connect4`-rewrite to leg F; inbound TPROXY to leg C + `getsockname` orig-dst (S00, S01) | rustls TLS 1.3 handshake on the agent's leg, both roles (S00, S02) | kTLS arms on the agent's leg B / leg C (S00, S03/S04) | tcpdump shows TLS 1.3 records on the peer leg, both ways (S00, S03/S04) |
| drain pre-arm plaintext losslessly; agent owns the leg (S00, S01) | present the held SVID via IdentityRead (S02) | agent-light pumps (RX = zero-copy splice; TX = read->write_all copy; no userspace crypto) (S03/S04) | fail-closed on absent SVID / nocert / wrongca (S05) |
| agent's own leg-B dial not re-intercepted; workload can't self-exempt (S01, S05) | verify peer vs trust bundle (S02) | (restart: new connections re-handshake) | resource limits + pump supervision + honest authn boundary (S05) |

### Walking Skeleton (thinnest end-to-end, all activities — COMPOSED PROXY WS, BLOCKING)

**Slice 00** (the composed proxy WS, BLOCKING): for ONE composed flow per direction
on the 6.18 kernel, prove the full A→B→C→D path holds end-to-end with **NO RST
post-arm** (transparent intercept → drain pre-arm plaintext losslessly → rustls
handshake on the agent's leg presenting the held SVID → kTLS arm → post-arm
bidirectional multi-record transfer → tcpdump shows TLS 1.3 records), under BOTH
normal AND traced/delayed timing. The minimum cut that touches every backbone
activity, one composed flow each way. The spikes settled the mechanism — the
primitives in isolation AND the inbound flow composed end-to-end in one direction
(`findings-inbound-intercept.md` increment-i §2); this slice closes the three
narrow remaining gaps (outbound composed in one flow; bidirectional round-trip;
real netns/veth topology). It is an integration gate, not a "prove-the-mechanism"
gate. (The outbound pieces were proven on separate harnesses; increment-e's
steady-state RST was a throwaway-harness intercept-lifecycle limitation, NOT a
kernel finding, superseded by increment-f's clean-harness steady-state proof.)

### Release 1 (the productionised happy path past the walking skeleton)

**Slice 01** (transparent intercept + leg-acquire, both directions, + the
intercept-recursion exemption mechanism), **Slice 02** (agent rustls handshake on
its peer-facing leg presenting the held SVID via IdentityRead, client AND server
roles), **Slice 03** (OUTBOUND enforce: kTLS arms on leg B + forward agent-light
splice + return agent-light splice + wire capture), **Slice 04** (INBOUND enforce:
orig-dst → server-mTLS → kTLS-RX → agent-light splice-to-server). Targets the
North-Star observable: the peer-facing wire carries TLS 1.3 records, both directions.

### Release 2 (the guardrails: fail-closed + limits + supervision + honest boundary)

**Slice 05** (fail-closed on absent SVID / nocert / wrongca with cause-distinct
reasons + the F4/F7 resource limits at concrete values + F6 pump supervision + the
F5 intercept-exemption negatives + the honest F1 authn-vs-authz boundary). Targets
the guardrail observables (fail-closed, no leak, no evade, honest claim).

### Slice list (each = one ≤1–1.5-day vertical cut; Slice 00 = the BLOCKING composed WS)

| Slice | Stories | Learning hypothesis (disproves X if it fails) | Brief |
|---|---|---|---|
| 00 (**composed proxy walking skeleton — BLOCKING**) | US-MTLS-00 | the agent-light L4 proxy COMPOSES under a real transparent intercept (outbound `cgroup_connect4` / inbound TPROXY) → pre-arm capture → handshake on the agent's leg → kTLS arm → post-arm bidirectional multi-record transfer holds with NO RST, under normal AND delayed timing. **FAIL → the composition does not hold; every later slice is blocked until the RST is engineered around** | `slices/slice-00-composed-proxy-walking-skeleton.md` |
| 01 | US-MTLS-01 | the platform transparently intercepts the workload's outbound connect (`cgroup_connect4`-rewrite to leg F) AND inbound arrival (TPROXY to leg C + `getsockname` orig-dst) to an agent-owned leg, drains the outbound pre-arm plaintext losslessly, and keeps the agent's own leg-B dial from recursing (the bypass agent-private) | `slices/slice-01-transparent-intercept-and-leg-acquire.md` |
| 02 | US-MTLS-02 | the agent performs the rustls TLS 1.3 handshake on its peer-facing leg presenting the HELD SVID (read via IdentityRead) and verifying the peer vs the trust bundle — as CLIENT (leg B) and as SERVER (leg C, REQUIRE+VERIFY the client SVID) — the workloads hold nothing | `slices/slice-02-agent-handshake-present-held-svid.md` |
| 03 | US-MTLS-03 | OUTBOUND enforce: the negotiated secrets arm kTLS on the agent's leg B (auth-session == data-session), forward is agent-light (a `read → write_all` COPY into leg B's kTLS-TX — NOT a splice, NOT zero-copy) and return is agent-light zero-copy (`splice` on a plain kTLS-RX leg), and tcpdump shows TLS 1.3 records on the peer wire (ss -tie shows the ULP) | `slices/slice-03-outbound-enforce-ktls-splice-wire-capture.md` |
| 04 | US-MTLS-04 | INBOUND enforce: orig-dst selects the server SVID → server-mTLS (REQUIRE+VERIFY the client SVID) → kTLS-RX arm on leg C → agent-light `splice` delivers byte-exact plaintext to the identity-unaware server workload, while the client-facing wire carries TLS `0x17` ciphertext only | `slices/slice-04-inbound-enforce-server-mtls-ktls-rx-splice.md` |
| 05 | US-MTLS-05 | the encryption cannot be bypassed and the boundary is honest: absent/missing/untrusted creds fail closed cause-distinct (both directions), the F4/F7 resource limits + F6 pump supervision hold at their concrete values with no leak, the intercept cannot be self-exempted (F5), and v1 is exactly chain-to-bundle authn — NO intended-peer pinning (the #178 upgrade) | `slices/slice-05-fail-closed-limits-supervision-authn-boundary.md` |

---

## Wave: DISCUSS / [REF] Priority Rationale

Execution order = **the composition first** (the blocking walking skeleton), then
the **happy-path dependency chain** (intercept → handshake → outbound enforce →
inbound enforce), then the **guardrails**. Per Value × Urgency / Effort with
Walking-Skeleton > Riskiest-Assumption > Highest-Value tie-breaking.

| Order | Slice | Why this position |
|---|---|---|
| 1 | S00 (composed WS) | **Riskiest assumption + walking skeleton.** Do the proven pieces COMPOSE into ONE bidirectional flow in the real netns/veth topology? The mechanism is spike-verified — primitives in isolation AND the inbound flow composed end-to-end (increment-i §2) — but three narrow gaps remain (outbound composed in one flow; bidirectional round-trip; real netns/veth topology; increment-e's steady-state RST was a throwaway-harness limitation, NOT a kernel finding, superseded by increment-f). If the composition does not hold (post-arm RST) every later slice is blocked. Cheapest place to learn it. Urgency 5 (derisks the fatal assumption). BLOCKING — integration gate, not mechanism. |
| 2 | S01 | Depends on S00 (productionises the transparent intercept + leg-acquire the WS composed). First step of the happy-path chain; the intercept-exemption mechanism the F5 negatives (S05) build on. |
| 3 | S02 | Depends on S01 (needs the agent-acquired leg + the recovered inbound orig-dst). The handshake on the agent's leg that presents the HELD SVID via `IdentityRead`, both roles — the integration seam with #35. Moderate uncertainty (rustls + reading a shipped port). |
| 4 | S03 | Depends on S02 (needs the handshake's extracted secrets). OUTBOUND enforce: kTLS arm on leg B + forward agent-light `read → write_all` copy (into kTLS-TX) + return agent-light zero-copy splice + the **North-Star wire-capture observable** (TLS 1.3 on the peer wire). Highest value (it IS the headline). |
| 5 | S04 | Depends on S02 (needs the server-role handshake) + S01 (the inbound orig-dst). INBOUND enforce: orig-dst → server-mTLS → kTLS-RX → agent-light splice-to-server. The second direction that makes "between two workloads" real. |
| 6 | S05 | Depends on S03 + S04 (needs both enforce paths to guard) + S02 (the handshakes to fail closed) + S01 (the intercept-exemption mechanism). The security teeth — fail-closed cause-distinct, resource limits + pump supervision, the intercept-exemption negatives, and the honest authn boundary. Resolves the guardrails last so the surface to guard is stable. |

Dependency chain: **S00 → S01 → S02 → {S03, S04} → S05** (S03 and S04 both depend
on S02 but not on each other — parallelisable after S02; S05 depends on both). The
order puts both enforce directions before the guardrails (S05) because the
guardrails harden a stable enforcement surface.

---

## Wave: DISCUSS / [REF] System Constraints (cross-cutting)

These apply to every story; stated once here rather than repeated per story.

- **Reader of `IdentityRead`, never an issuer (CORRECTNESS / source-of-truth
  constraint).** #26 reads the held SVID + trust bundle via the shipped
  `IdentityRead` port (#35); it MUST NOT mint, re-issue, or hold its own copy of
  the credential. The held map (#35) is the single source of truth; #26 duplicating
  it would drift on rotation/drop. Reading `None` for an allocation is the
  fail-closed signal (the agent refuses the handshake rather than presenting a
  stale credential).
- **Workload holds NOTHING; the platform does mTLS (whitepaper §7, CLAUDE.md §
  "Workload identity model").** The workload is identity-unaware — no cert, no key.
  It opens ordinary sockets; the agent-light L4 proxy terminates/originates TLS
  transparently on its OWN peer-facing leg using material the platform (the node
  agent) supplies. There is no SPIRE-agent-style workload-held copy. Any design
  that puts SVID material inside the workload is a violation.
- **Auth-session == data-session.** The SAME TLS 1.3 session that authenticated
  the peer (the rustls handshake on the agent's leg) carries the data (its
  extracted secrets install into kTLS on that same leg). This is the property that
  distinguishes Overdrive's mTLS from an out-of-band-auth model (where auth and
  encryption are separate sessions) — the spikes proved it round-trips on a real
  kernel.
- **Fail-closed AND lossless for confidentiality.** No cleartext byte reaches the
  peer wire before encryption is armed. The agent captures the workload's pre-arm
  plaintext in a bounded userspace handshake buffer and flushes it as the first
  `application_data` after the handshake — lossless (no dropped bytes, no RESET) for
  EVERY protocol kind. There is no in-kernel gate, no write-block, no kernel patch,
  and no race window (the userspace buffer is the lossless capture primitive).
- **Mechanism pinned by ADR-0069: the agent-light L4 proxy.** ONE universal
  mechanism for all workload kinds, bidirectional. The exact `MtlsEnforcement`
  contract (4 methods), kTLS `crypto_info` mapping, and intercept mechanics are
  pinned in the `## Wave: DESIGN` sections, grounded on the committed spike findings.
- **Pinned 6.18 LTS kernel (ADR-0068) — no version anxiety.** In-kernel TLS 1.3
  TX+RX, `CONFIG_NET_HANDSHAKE`, sockmap, and `splice`/`tls_sw_splice_read` are
  guaranteed; the kernel is a controlled constant, not a design axis. The platform
  tests exactly the one kernel it ships (+ bpf-next soft-fail).
- **NO restart-survival in v1 (the accepted proxy trade).** The agent owns both
  legs and the kTLS state, so an agent restart drops in-flight sessions — they
  re-handshake on reconnect (new connections re-run the
  intercept→handshake→arm path). v1 carries a 2-sockets-per-connection density cost.
  Restart-survival + 1-socket density were the superseded in-band model's unique
  win; they are NOT in v1 scope (the accepted proxy trade, ADR-0069 A1) — a post-v1
  optimization tracked in **#231**.
- **All workload kinds via the proxy: process/exec v1, guest-stack staged #222.**
  v1 ships process (exec driver) only — TCP terminates in the HOST kernel. There is
  no distinct WASM path (only `ExecDriver` exists; a future WASM driver's
  host-socket workloads are auto-covered by the same proxy). Guest-stack workloads
  (microVM/unikernel, TCP in the GUEST kernel) route through the SAME mechanism via
  a STAGED guest-stack intercept ADAPTER (**#222**, repurposed by ADR-0069 from "a
  separate mechanism" to "the guest-stack intercept adapter of the #26 universal
  proxy") — not a second enforcement mechanism. Do NOT pull #222 into v1.
- **In-place rekey deferred to #229; rotation to #40.** v1 SVID rotation on a
  long-lived connection = TEARDOWN + RECONNECT (kernel-side KeyUpdate IS present at
  v6.18, but the userspace rustls/ktls bridge is not — rustls/ktls#59 / #62, #229).
  A TRACKED DEPENDENCY, not an open design risk. The cert-rotation workflow is #40.
- **No operator CLI verb; #26 is a FOUNDATION feature (D1).** Encryption is
  automatic and undisableable (vision principle 2) — there is no `overdrive`
  subcommand to "encrypt this workload". The HONEST observable surfaces are
  TEST-tier: `tcpdump` showing TLS 1.3 records on the peer-facing leg, `ss -tie`
  showing the kTLS ULP on the agent's leg, a fail-closed negative test, an
  agent-light `strace`. Per CLAUDE.md the workload verb
  is `overdrive deploy <SPEC>`, never `job submit`. Do NOT invent a CLI verb.

---

## Wave: DISCUSS / [REF] User Stories

Every story traces to `job_id: J-SEC-003`. Every story's "After" references a real,
executable verification entry point (the wire capture / `ss -tie` / the test
runner — the honest user-invocable observable for a security primitive with no
operator verb). ACs are embedded and derived from the UAT scenarios.

> **The authoritative, fully-specified ACs + UAT scenarios for each story live in
> the slice files** (`slices/slice-00…05`, re-grounded to ADR-0069 / the agent-light
> L4 proxy). The summaries below are the DISCUSS-level narrative; each names its
> slice. US-MTLS-00→05 map 1:1 to Slices 00→05.

> **Elevator-Pitch "After" caveat (SAME as built-in-ca / workload-identity-manager
> — a security primitive with NO operator CLI verb)**: encryption is automatic and
> undisableable; there is no `overdrive` subcommand to "encrypt this workload".
> Each story's "After" references a real, executable verification entry point — a
> `tcpdump` wire capture showing TLS 1.3 records on the peer-facing leg (or `ss
> -tie` showing the kTLS ULP on the agent's leg, or a fail-closed negative test, or
> an agent-light `strace`) — which is the honest user-invocable observable output
> for this feature, not an invented subcommand. The DECISION enabled is Sam's trust
> decision (the genuine J-SEC-003 connection: is the wire actually encrypted with
> the workload's own SVID, in-kernel, both directions, agent agent-light, no
> cleartext leak, fail-closed?).
>
> **Foundation-feature exception to the strict elevator-pitch gate (recorded
> explicitly, NOT a silent pass — mirroring built-in-ca / #35)**: the strict nWave
> gate requires a real user-invocable entry point. #26 does not strictly satisfy
> that on its own — every Phase-2 proof is TEST-tier (a `tcpdump` capture / `ss
> -tie` / a fail-closed negative test / an agent-light `strace` in a gated
> `integration-tests` Lima run), because the feature is a foundation security
> primitive with no operator verb and encryption is undisableable by design. The
> gate is met by a **deliberate, documented foundation-feature exception mirroring
> built-in-ca and #35, NOT by a live operator surface and NOT by an invented CLI
> verb** (inventing a verb to dodge the gate is the dishonest move; recording the
> exception is the honest one). Recorded here, in `wave-decisions.md` (D1), and in
> the DoR validation (the elevator-pitch / slice-composition items note it with
> this evidence pointer).

### US-MTLS-00 — the COMPOSED proxy walking skeleton: a real intercept end-to-end, both directions, NO RST `@walking_skeleton`

**Type**: Composed Tier-3 acceptance test (the BLOCKING walking skeleton — production
code, NOT a spike; the 6 committed spikes already ran). **The authoritative ACs +
UAT scenarios are in `slices/slice-00-composed-proxy-walking-skeleton.md`.**

**Problem**: the Tier-3 spikes settled the MECHANISM (verdict: the agent-light L4
proxy) — proving every PRIMITIVE in isolation AND the INBOUND flow composed
end-to-end in one direction (`findings-inbound-intercept.md` increment-i §2:
real TPROXY intercept → orig-dst recovery → server-mTLS verifying C's client SVID
→ kTLS-RX arm → agent-light splice-to-S byte-exact, fail-closed on
`nocert`/`wrongca`). Three NARROW composition gaps remain: (1) the OUTBOUND path
composed in ONE flow (its pieces were proven on SEPARATE harnesses — increment-f
removed the intercept to isolate the splice; increment-e's steady-state RST was a
*throwaway-harness intercept-lifecycle limitation, NOT a kernel finding*,
superseded by increment-f's clean-harness proof); (2) bidirectional steady-state
round-trip; (3) the real netns/veth topology with cgroup-isolated workloads. Sam
will not let any later slice build on those gaps still open — this is an
integration gate that closes them, not a doubt about the mechanism.

**Who**: Platform/security engineer | closing the three remaining composition gaps
(integration, not the mechanism) | wants the WHOLE proxy path composed on a real
intercept in the real netns/veth topology, both directions, with no post-arm RST,
before any other slice lands.

**Solution**: A composed Tier-3 acceptance test on the 6.18 kernel that drives the full
agent-light L4 proxy path for ONE composed flow per direction — OUTBOUND (real
`cgroup_connect4` intercept → agent drains pre-arm plaintext losslessly → rustls TLS 1.3
CLIENT handshake on leg B presenting the held SVID (read via `IdentityRead`) → kTLS arm
on leg B → post-arm **bidirectional** multi-record transfer) AND INBOUND (real nft-TPROXY
intercept → `getsockname` orig-dst → server-mTLS → kTLS-RX arm → splice-to-server,
byte-exact plaintext at S) — under BOTH normal AND traced/delayed timing, with **NO RST
post-arm**. The observable IS the lossless, RST-free, TLS-1.3-on-the-peer-wire capture
for both halves.

#### Elevator Pitch

- **Before**: the primitives are spike-proven in isolation AND the INBOUND flow is
  proven composed end-to-end in one direction (`findings-inbound-intercept.md`
  increment-i §2), but three narrow gaps remain — the OUTBOUND path composed in one
  flow (its pieces proven on separate harnesses; increment-e's steady-state RST was a
  throwaway-harness limitation, NOT a kernel finding, superseded by increment-f),
  bidirectional round-trip, and the real netns/veth topology — so every later slice
  would build on those gaps still open.
- **After**: a composed Tier-3 acceptance test on the 6.18 Lima kernel shows, for one
  composed flow each way, a real intercept → handshake on the agent's leg → kTLS arm →
  post-arm bidirectional multi-record transfer with NO RST, under normal AND delayed
  timing — a `tcpdump` capture shows TLS 1.3 Application Data records on the peer-facing
  wire (both directions) and `ss -tie` shows the kTLS ULP on the agent's leg.
- **Decision enabled**: Sam decides the agent-light L4 proxy genuinely composes under a
  real transparent intercept — or learns the composition does not hold (post-arm RST)
  before any productionisation depends on it.

#### Domain Examples

1. **Outbound composed (happy path)** — On the 6.18 Lima kernel, process `client` (alloc
   `a1b2c3`, SVID `spiffe://overdrive.local/job/web/alloc/a1b2c3`) connects to `api`
   (alloc `d4e5f6`). `cgroup_connect4` rewrites the connect to the agent's leg-F
   listener; the agent drains the pre-arm plaintext losslessly, runs the rustls TLS 1.3
   CLIENT handshake on leg B presenting `a1b2c3`'s held SVID (read via `IdentityRead`)
   and verifying the peer chains to the bundle, arms kTLS on leg B, and post-arm
   bidirectional multi-record transfer completes with NO RST. `tcpdump -i
   overdrive-veth0` shows TLS 1.3 App Data records (0x17); `ss -tie` shows the kTLS ULP
   on the agent's leg.
2. **Inbound composed (happy path)** — a connection aimed at `d4e5f6`'s logical address
   is nft-TPROXY-redirected to the agent's `IP_TRANSPARENT` leg-C listener;
   `getsockname()` recovers the original destination → selects `d4e5f6`'s held SVID; the
   agent runs the server-side mutual-TLS handshake (presents the server SVID,
   `WebPkiClientVerifier` REQUIRE+VERIFY the client SVID), arms kTLS-RX, and `splice`s the
   decrypted plaintext to the identity-unaware server workload S byte-exact, NO RST.
3. **Timing robustness** — the same composed flow is exercised under a deliberate
   handshake-window delay; the post-arm transfer never RSTs in either timing regime.
   (increment-e's steady-state RST was a throwaway-harness intercept-lifecycle
   limitation, NOT a kernel finding — increment-f later proved the steady-state egress
   splice cleanly with the intercept removed; this AC pins that the production
   intercept lifecycle is engineered to hold under both timing regimes.)

#### UAT Scenarios (BDD)

##### Scenario: A real intercepted flow carries TLS 1.3 both ways with no reset
Given two host-socket workloads on the pinned 6.18 kernel, each with a held SVID, neither holding any cert or key
And the platform transparently intercepts the client's outbound connect and the server's inbound arrival
When the workloads exchange application bytes in both directions after the platform completes the handshake and arms encryption
Then a wire capture on the peer-facing leg shows TLS 1.3 Application Data records in both directions and no cleartext of the payload
And the connection is never reset after encryption is armed, under both normal and deliberately delayed timing

##### Scenario: The composed path holds before any other behaviour is built on it
Given the composed intercept-to-handshake-to-encrypt-to-transfer path for both directions
When it is exercised as the first acceptance gate
Then it passes before any other enforcement slice is accepted

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-00-composed-proxy-walking-skeleton.md`. Summary:

- [ ] OUTBOUND composed: a real `cgroup_connect4` intercept → lossless pre-arm drain →
  rustls TLS 1.3 CLIENT handshake on leg B presenting the held SVID (read via
  `IdentityRead`) → kTLS arm on leg B → post-arm bidirectional multi-record transfer,
  NO RST.
- [ ] INBOUND composed: a real nft-TPROXY intercept → `getsockname` orig-dst →
  server-mTLS (`WebPkiClientVerifier` REQUIRE+VERIFY) → kTLS-RX arm → splice-to-server,
  byte-exact plaintext at S, NO RST.
- [ ] The peer-facing leg carries TLS 1.3 Application Data records (`tcpdump` 0x17) in
  both directions; the workload's plaintext appears only on the host-internal leg F /
  leg S, never on the peer leg.
- [ ] The composed path holds under BOTH normal AND traced/delayed timing (no post-arm
  RST in either regime; increment-e's steady-state RST was a throwaway-harness
  limitation, not a kernel finding — increment-f proved the clean-harness steady state).
- [ ] The agent reads SVID + bundle ONLY via `IdentityRead` (a READER, never an
  issuer/cache); kTLS arms on the agent's leg (leg B / leg C), NOT the workload's socket.

#### Technical Notes

- The BLOCKING first DELIVER slice (F2): no other slice lands until this passes. It is a
  composed acceptance test, NOT a spike — a FAIL here is a real defect to engineer
  around, not a learning outcome.
- The cost is wiring the proven pieces into ONE bidirectional flow in the real
  netns/veth topology — composing the outbound intercept + steady-state splice (proven
  on separate harnesses), adding the bidirectional round-trip, and engineering the
  production intercept lifecycle so it holds under both timing regimes. (increment-e's
  steady-state RST was a throwaway-harness limitation, NOT a kernel finding, superseded
  by increment-f.) The primitives and the inbound composition are already proven; this
  is integration, not mechanism discovery.
- No Tier-2 backstop exists for these socket-context hooks (`BPF_PROG_TEST_RUN` is
  unavailable) — it can only be settled at Tier 3.

---

### US-MTLS-01 — the workload's traffic is transparently intercepted to the agent's leg, both directions, no recursion

**Problem**: For the platform to encrypt a host-socket workload's traffic, the
workload's connection must be transparently brought under the agent's control —
before the identity-unaware workload can reach the real peer un-encrypted — in BOTH
directions, and the agent's OWN peer-facing dial must not recurse into the same
intercept. Today there is no intercept path: a host-socket workload's connections
flow straight to the peer in cleartext. **The authoritative ACs are in
`slices/slice-01-transparent-intercept-and-leg-acquire.md`.**

**Who**: Platform/security engineer | wiring the transparent intercept | wants
host-socket connections redirected to an agent-owned leg before any cleartext
escapes, in both directions, with the agent's own dial exempt and the workload
unable to self-exempt.

**Solution**: OUTBOUND, a `cgroup_connect4`-rewrite program redirects the workload's
`connect()` to the agent's node-local leg-F listener (reusing the established
`cgroup_connect4_service` shape); the agent `accept()`s leg F and drains the pre-arm
plaintext losslessly into a bounded userspace buffer. INBOUND, an nft-TPROXY rule
redirects a connection aimed at a server workload's logical address to the agent's
`IP_TRANSPARENT` leg-C listener, and `getsockname()` recovers the original
destination (NOT `SO_ORIGINAL_DST`). The agent's own outbound leg-B dial is NOT
re-intercepted (a narrowly-scoped, agent-private `SO_MARK`/cgroup-scoping bypass the
program checks-and-skips), and a workload cannot replicate the bypass to self-exempt.

#### Elevator Pitch

- **Before**: a host-socket workload's TCP connections flow straight to the peer in
  cleartext — nothing intercepts them, so nothing can encrypt them, and a naive
  agent dial would recurse into its own intercept.
- **After**: the workload's outbound connect is transparently rewritten
  (`cgroup_connect4`) to the agent's leg F and its inbound arrival is TPROXY-redirected
  to the agent's leg C — observable in a Tier-3 test as the connect rewritten / the
  arrival redirected, the pre-arm plaintext drained losslessly, the inbound orig-dst
  recovered, and the agent's own leg-B dial NOT re-intercepted.
- **Decision enabled**: Sam decides the platform reliably brings host-socket
  connections under agent control before any cleartext can escape (both directions) and
  the workload cannot evade interception — or rejects an intercept that loses pre-arm
  bytes, cannot recover the inbound orig-dst, or recurses.

#### Domain Examples

1. **Outbound intercept (process client)** — process `web` (alloc `a1b2c3`) calls
   `connect()` to `api`. The `cgroup_connect4` mtls-variant rewrites the destination
   to the agent's leg-F listener; the agent `accept()`s and `recv()`s `web`'s pre-arm
   plaintext into a bounded userspace buffer losslessly (no dropped bytes; route by
   `local_port` only — `findings-userspace-relay.md` Unknown 1).
2. **Inbound intercept (process server)** — a connection aimed at `api`'s (alloc
   `d4e5f6`) logical address is nft-TPROXY-redirected to the agent's `IP_TRANSPARENT`
   leg-C listener; `getsockname()` on the accepted leg-C socket recovers the original
   destination, which selects `d4e5f6`'s `AllocationId` → its held SVID.
3. **No recursion / no self-exempt** — the agent's own leg-B dial to the real peer is
   NOT re-intercepted by the workload `cgroup_connect4` program (the agent's egress
   carries an agent-private bypass, outside the workload attach subtree); a workload
   setting the bypass on its own socket is STILL intercepted (the bypass is unreachable
   from the workload).

#### UAT Scenarios (BDD)

##### Scenario: A workload's outbound connection is brought under platform control before it reaches the peer
Given a host-socket workload that opens an outbound TCP connection
When the connection is established
Then the platform transparently routes it through the node agent before any byte reaches the real peer
And the agent captures the workload's first bytes without losing any or resetting the connection

##### Scenario: An inbound connection to a server workload is brought under platform control and the right identity is selected
Given an inbound connection aimed at a server workload's logical address
When the platform transparently intercepts it
Then the platform recovers the address the client aimed at and selects that server workload's own identity

##### Scenario: A workload cannot evade the platform's interception
Given a host-socket workload trying to bypass interception on its own sockets
When it opens a connection
Then it is still intercepted, because the bypass that exempts the agent's own connections is unreachable from the workload

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-01-transparent-intercept-and-leg-acquire.md`. Summary:

- [ ] A `cgroup_connect4`-rewrite program redirects a host-socket workload's `connect()` to the agent's leg-F listener; the agent `accept()`s and `recv()`s the pre-arm plaintext LOSSLESSLY (no dropped bytes), routing by `local_port` only.
- [ ] An nft-TPROXY rule redirects a connection aimed at a server workload's logical address to the agent's `IP_TRANSPARENT` leg-C listener; `getsockname()` recovers the original destination (NOT `SO_ORIGINAL_DST`).
- [ ] The agent's own leg-B dial is NOT re-intercepted (no recursion) via the agent-private `SO_MARK`/cgroup-scoping bypass; a workload CANNOT replicate it to self-exempt (the F5 negatives are S05; the mechanism is here).
- [ ] The intercept program + its maps/link are bpffs-pinned (`pinning = ByName`, `/sys/fs/bpf/overdrive/`); the `IP_TRANSPARENT` listener + nft-TPROXY setup succeed under `CAP_NET_ADMIN` (the agent is privileged; the workload is unprivileged and holds nothing).

#### Technical Notes

- The exact intercept attach mechanism + the nft-TPROXY triple are DESIGN's to pin
  (the `cgroup_connect4` mtls-variant reuses the established `cgroup_connect4_service`
  attach boundary). Productionises the Slice-00 composed walking skeleton's
  intercept + leg-acquire step.
- There is no distinct WASM path (only `ExecDriver` exists); a future WASM driver's
  host-socket workloads are auto-covered by the same intercept. Guest-stack is the
  staged #222 adapter, out of v1.

---

### US-MTLS-02 — the agent performs the TLS 1.3 handshake presenting the held SVID (client AND server roles)

**Problem**: Once a host-socket connection is intercepted, the platform must perform
the TLS 1.3 handshake **on the workload's behalf**, on the **agent's own peer-facing
leg** (NOT the workload's socket), presenting the **workload's own held SVID** (not
the node's, not the agent's) and verifying the peer — in BOTH roles (CLIENT outbound /
SERVER inbound) — because the workload is identity-unaware and holds nothing. There is
no handshake path today, and the credential it must present lives behind #35's
`IdentityRead` port. **The authoritative ACs are in
`slices/slice-02-agent-handshake-present-held-svid.md`.**

**Who**: Platform/security engineer | wiring the agent's mutual-TLS handshake (both
roles) | wants the agent to present the WORKLOAD's held SVID (read via IdentityRead)
and verify the peer against the trust bundle, with the workloads holding nothing.

**Solution**: The node agent performs a rustls TLS 1.3 handshake on its OWN
peer-facing leg (leg B outbound as CLIENT, leg C inbound as SERVER), presenting the
held `SvidMaterial` it reads via `IdentityRead::svid_for(&AllocationId)` (#35; leaf
key per ADR-0063 D9) and verifying the peer against `IdentityRead::current_bundle()`.
Inbound, it acts as the SERVER: presents the server workload's SVID AND
requires-and-verifies the client's presented SVID chains to the bundle via
`WebPkiClientVerifier` (REQUIRE+VERIFY). #26 is a READER of the held set — it never
mints, re-issues, or caches its own copy. v1 verification is chain-to-bundle ONLY
(NOT intended-peer pinning — that is #178).

#### Elevator Pitch

- **Before**: there is no path to perform mTLS on a host-socket workload's behalf —
  the held SVID (#35) is readable but nothing presents it on a handshake, and the
  workload (identity-unaware) cannot do it itself.
- **After**: the agent performs the rustls TLS 1.3 handshake presenting the
  workload's own held SVID (read via `IdentityRead`) and verifying the peer against
  the trust bundle — observable in a Tier-3 test as a completed mutual-TLS handshake
  whose presented leaf chains to the root (the SAN matches the workload's
  allocation), with the workload holding no cert and no key.
- **Decision enabled**: Sam decides the platform presents the right identity (the
  workload's own, read from the single source of truth) and verifies peers
  correctly — or rejects a handshake that presents the node's identity, caches its
  own credential copy, or skips peer verification.

#### Domain Examples

1. **Happy path** — for alloc `a1b2c3`, the agent reads the held SVID
   (`spiffe://overdrive.local/job/web/alloc/a1b2c3`) via
   `IdentityRead::svid_for(&a1b2c3)`, and performs the rustls TLS 1.3 handshake
   presenting it, verifying the peer `d4e5f6` against `current_bundle()`. The
   handshake completes; the presented leaf chains to the root and its SAN is
   `a1b2c3`'s SPIFFE URI.
2. **Reads from the single source of truth (no #26-local cache)** — the agent reads
   the bundle via `IdentityRead::current_bundle()` (the hydrated bundle behind
   #35's port), not a #26-local copy; when #35's bundle updates, the next handshake
   verifies against the current one — no drift.
3. **Workload holds nothing** — `web`'s process has no cert and no key in its own
   memory/filesystem; the leaf key (`SvidMaterial::leaf_key`, ADR-0063 D9) is held
   by the agent (read via the port) and used to drive the handshake — the workload
   is identity-unaware throughout.

#### UAT Scenarios (BDD)

##### Scenario: The agent presents the workload's own held identity in the handshake
Given a detected host-socket connection for a running workload whose SVID is held
When the platform performs the TLS handshake on the workload's behalf
Then it presents the workload's own held SVID (read from the identity read surface)
And it verifies the peer against the current trust bundle
And the workload itself holds no certificate or private key

##### Scenario: The handshake reads identity from the single source of truth
Given the held trust bundle is updated by the identity subsystem
When the platform performs a subsequent handshake
Then it verifies the peer against the current bundle read from the identity read surface
And it does not use a separately cached copy

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-02-agent-handshake-present-held-svid.md`. Summary:

- [ ] OUTBOUND (client): the agent performs a rustls TLS 1.3 CLIENT handshake on leg B presenting the held `SvidMaterial` read via `IdentityRead::svid_for(&AllocationId)` (#35) and verifying the peer chains to `IdentityRead::current_bundle()` — #26 reads, never mints/caches.
- [ ] INBOUND (server): the agent performs a rustls TLS 1.3 SERVER handshake on leg C presenting the server SVID AND requiring-and-verifying the client's SVID chains to the bundle via `WebPkiClientVerifier` (REQUIRE+VERIFY); the fail-closed negatives (absent SVID outbound; nocert/wrongca inbound) are Slice 05.
- [ ] The presented leaf chains to the root and its SAN is the workload's SPIFFE URI (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via `openssl verify` / the captured handshake at the TEST tier.
- [ ] BOTH workloads hold no cert and no key (the leaf key stays with the agent, read via the port) — the workloads are identity-unaware.

#### Technical Notes

- The exact rustls config shape (ClientConfig/ServerConfig, the
  `IdentityRead`-backed cert resolver, the server-side `WebPkiClientVerifier`) is
  DESIGN's to pin. #26 takes the `IdentityRead` port as a required constructor
  parameter (port-trait discipline, `.claude/rules/development.md`).
- Two server-config mechanics bind on DELIVER (`findings-inbound-intercept.md` §
  Mechanics): suppress `NewSessionTicket` (`send_tls13_tickets = 0` — a post-handshake
  ticket hits `-EIO` on raw kTLS-RX); read `peer_certificates()` for the fail-closed
  guard BEFORE `dangerous_extract_secrets()` consumes the connection.

---

### US-MTLS-03 — OUTBOUND enforce: kTLS arms on the agent's leg B, forward agent-light write_all copy + return agent-light zero-copy splice, wire carries TLS 1.3

**Problem**: A completed outbound handshake is not enough — for the encryption to be
**in-kernel** and the agent **agent-light** (NOT a per-byte userspace proxy — the
property that distinguishes Overdrive from ztunnel), the negotiated session keys must
arm **kTLS on the agent's peer-facing leg B** (auth-session == data-session) and the
kernel must carry the steady state. Then a packet capture must prove the peer-facing
wire actually carries TLS 1.3 records. **The authoritative ACs are in
`slices/slice-03-outbound-enforce-ktls-splice-wire-capture.md`.**

**Who**: Platform/security engineer | wiring the outbound kTLS arm + agent-light
pumps | wants the negotiated session armed into the kernel on the agent's leg, the
agent running no userspace cipher, and TLS 1.3 records provable on the peer wire.

**Solution**: After the outbound handshake (US-MTLS-02), the agent arms the rustls
handshake's extracted secrets into kTLS on leg B (`setsockopt TCP_ULP "tls"` +
`TLS_TX/TLS_RX`) — the SAME session that authenticated — and hands steady state to the
kernel as two **agent-light** pumps that are NOT symmetric: **forward** (F→B, the
liveness-observed primary) is a bounded `read(legF) → write_all(legB)` COPY into leg
B's kTLS-TX, the kernel `tls_sw_sendmsg` encrypting each blocking `write` → encrypted
egress (per-record `read`+`write`, plaintext copied through a userspace buffer — NOT
zero-copy, NOT agent-idle); **return** (B→F) is a bounded `splice(legB → pipe → legF)`
pump on a plain kTLS-RX leg, `tls_sw_splice_read` decrypting each record zero-copy
(~1 splice per record, no userspace plaintext copy). In BOTH the agent does NO crypto
(the kernel `tls_sw` does it). A `splice` INTO kTLS-TX is NOT used (it loses records).
(D-MTLS-13: the forward `write_all` copy replaced an earlier sockmap EGRESS-redirect
that `MSG_DONTWAIT`-stalled; see `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`.)

#### Elevator Pitch

- **Before**: even with a completed handshake, there is no in-kernel encryption on the
  agent's leg and no proof the wire is encrypted — and a userspace proxy staying in the
  path (the ztunnel shape) would not be agent-light.
- **After**: the negotiated session arms kTLS on the agent's leg B and the kernel
  carries steady state (forward = `read → write_all` copy into kTLS-TX; return =
  zero-copy `splice` out of kTLS-RX; the kernel does the crypto, the agent runs no
  cipher) — observable as `ss -tie` showing the kTLS ULP on the agent's leg and a
  `tcpdump` capture on the peer-facing wire showing TLS 1.3 Application Data records
  (content type 0x17), never cleartext.
- **Decision enabled**: Sam decides the encryption is genuinely in-kernel with the agent
  doing no userspace crypto (the auth-session is the data-session) — or rejects a design
  where a userspace proxy quietly runs the cipher for the whole connection.

#### Domain Examples

1. **Happy path (the North-Star observable)** — after `a1b2c3`↔`d4e5f6`'s handshake, the
   agent arms the extracted secrets into kTLS on leg B; the agent's forward pump reads
   leg F and `write_all`s into leg B's kTLS TX (the kernel `tls_sw_sendmsg` encrypts each
   write). `ss -tie` shows `tcp-ulp-tls 1.3 aes-gcm-256`; `tcpdump -i overdrive-veth0`
   shows TLS 1.3 App Data records (0x17 03 03); a `GET /payments HTTP/1.1` the workload
   sent never appears in cleartext on the peer wire (it lives only on the host-internal
   leg F).
2. **Forward agent-light (a COPY, not zero-copy)** — `strace` of the agent shows a
   per-record `read(legF)`+`write(legB)` on the forward path into leg B's kTLS-TX (a
   userspace plaintext copy; NOT `splice`/`ppoll`-only); the kernel's `tls_sw_sendmsg`
   encrypts each `write` — the agent does no crypto but does copy each record. `splice`
   into kTLS-TX is not used (it loses records) (`crates/overdrive-dataplane/src/mtls/splice.rs`;
   `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`).
3. **Return agent-light (zero-copy)** — `strace` shows only `splice`/`ppoll` on the return
   path (zero payload `read`/`write`), byte-exact plaintext on leg F, ~1 `splice` per TLS
   record (`findings-splice-return.md`).

#### UAT Scenarios (BDD)

##### Scenario: The wire carries TLS 1.3 records on the outbound peer-facing leg
Given two host-socket workloads whose outbound handshake has completed
When the platform arms encryption on its own peer-facing leg and hands steady state to the kernel
Then a wire capture on the peer-facing leg shows TLS 1.3 Application Data records and no cleartext of the payload
And the kTLS upper-layer protocol is installed on the agent's peer-facing leg

##### Scenario: Encryption is in-kernel with the agent running no userspace cipher
Given an outbound connection whose encryption is armed in the kernel
When the workloads exchange application bytes
Then the kernel performs the forward record framing and encryption on each write, the agent moving the forward path via a per-record read(legF)+write(legB) into kTLS-TX (a userspace plaintext copy, not a splice) and running no cipher
And the agent moves the return path via splice only (~1 splice per record), zero-copy, never copying a payload byte in userspace

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-03-outbound-enforce-ktls-splice-wire-capture.md`. Summary:

- [ ] The agent arms the rustls handshake's extracted secrets into kTLS on leg B (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's secrets (auth-session == data-session), not a separately negotiated session.
- [ ] Forward agent-light (a COPY, NOT zero-copy): `tcpdump` on the peer-facing wire shows 0x17 records and `strace` shows the agent's forward pump issues a per-record `read(legF)`+`write(legB)` into kTLS-TX (a userspace plaintext copy; the kernel `tls_sw_sendmsg` encrypts each write — the agent runs no cipher), NOT `splice`/`ppoll`-only; return agent-light zero-copy: `strace` shows only `splice`/`ppoll`, byte-exact plaintext on leg F, ~1 splice per record.
- [ ] `ss -tie` shows the kTLS ULP on leg B (`tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw`) (TEST tier, via Lima).
- [ ] A `tcpdump` capture on the peer-facing wire shows TLS 1.3 Application Data records (0x17) and NEVER the cleartext payload (the workload's plaintext is on leg F, host-internal, by design) — the K1 North-Star observable.

#### Technical Notes

- The exact kTLS `crypto_info` struct construction (mapping rustls extracted secrets →
  `TLS_TX/TLS_RX`) and the record-sequence handling are DESIGN's to pin — DISCUSS pins
  the observable (TLS 1.3 on the peer wire), not the struct shape.
- This is the outbound North-Star observable slice (the wire carries TLS 1.3); it is the
  headline value and is prioritised accordingly. Slice 04 mirrors it inbound.

---

### US-MTLS-04 — INBOUND enforce: orig-dst → server-mTLS → kTLS-RX → agent-light splice-to-server

**Problem**: The outbound half (US-MTLS-03) encrypts a workload's *client* traffic, but
"between two workloads" is only real when the *server* half is enforced too. After the
inbound intercept (US-MTLS-01) selects the server workload's identity from the
TPROXY-recovered original destination, the platform must complete the server-side
mutual-TLS handshake, arm kTLS-RX on the agent's client-facing leg, and deliver the
**byte-exact decrypted plaintext** to the identity-unaware server workload — while the
client-facing wire carries ciphertext only. **The authoritative ACs are in
`slices/slice-04-inbound-enforce-server-mtls-ktls-rx-splice.md`.**

**Who**: Platform/security engineer | wiring the inbound/server enforce path | wants the
server-side mutual-TLS to verify the client SVID, the kernel to decrypt, and the server
workload to read byte-exact plaintext without holding anything.

**Solution**: The TPROXY-recovered original destination (`getsockname` on leg C, Slice
01) selects the server workload's `AllocationId` → held SVID via `IdentityRead`. The
agent runs the server-side rustls TLS 1.3 handshake on leg C (presents the server SVID,
`WebPkiClientVerifier` REQUIRE+VERIFY the client's SVID chains to the bundle), arms
kTLS-RX on leg C, and drives an agent-light `splice(legC → pipe → legS)` pump that
delivers the byte-exact decrypted plaintext to the server workload S. The client-facing
leg carries TLS `0x17` app_data only; the plaintext appears only on the agent→S leg.

#### Elevator Pitch

- **Before**: only the outbound/client direction is encrypted — an inbound connection to
  a server workload would still be enforced by nothing, and "between two workloads" would
  be half-true.
- **After**: an inbound connection is TPROXY-intercepted, the server workload's identity
  is selected from the recovered orig-dst, the server-side mutual-TLS verifies the client
  SVID, kTLS-RX decrypts in-kernel, and the server workload reads the byte-exact request
  as plaintext — observable as `tcpdump` showing 0x17 on the client leg, byte-exact
  plaintext at S, `strace` showing splice-only delivery, and `ss -tie` showing kTLS-RX.
- **Decision enabled**: Sam decides the inbound half works agent-light and the server
  workload reads byte-exact plaintext while the wire carries ciphertext — or rejects an
  inbound path that leaks request cleartext or copies bytes in userspace.

#### Domain Examples

1. **Orig-dst → identity** — a connection aimed at `d4e5f6`'s logical address is
   TPROXY-redirected to the agent's leg C; `getsockname()` recovers
   `127.0.0.2:18443` → selects `d4e5f6`'s `AllocationId` → its held SVID via
   `IdentityRead` (`findings-inbound-intercept.md` §1).
2. **Server-mTLS + kTLS-RX** — the agent presents `d4e5f6`'s server SVID;
   `WebPkiClientVerifier` REQUIRE+VERIFY the client's SVID chains to the bundle; a valid
   client cert → handshake succeeds, kTLS-RX armed (`ss -tie` `rxconf:sw`).
3. **Byte-exact plaintext at S, agent-light** — the agent `splice`s the decrypted
   plaintext to the server workload S byte-exact; the client-facing leg carries `0x17`
   records only (cleartext-marker hits on the client leg = 0); `strace` shows the agent
   moves the payload via `splice`/`ppoll` only (`findings-inbound-intercept.md` §3/§5).

#### UAT Scenarios (BDD)

##### Scenario: An inbound connection is server-authenticated and the server workload reads byte-exact plaintext
Given an intercepted inbound connection aimed at a server workload whose SVID is held, presenting a valid client SVID
When the platform completes the server-side handshake, arms kTLS-RX, and delivers the request to the server workload
Then the server workload reads the byte-exact request as plaintext while it holds no certificate or private key
And the client-facing wire carries TLS 1.3 Application Data records and no cleartext of the request

##### Scenario: The platform stays out of the per-byte path on the inbound deliver
Given an inbound connection whose kTLS-RX is armed on the agent's client-facing leg
When the client streams a request
Then the kernel decrypts each record and the agent delivers it to the server workload via splice only (~1 splice per record)
And the agent never copies a payload byte through userspace

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-04-inbound-enforce-server-mtls-ktls-rx-splice.md`. Summary:

- [ ] Orig-dst → identity: the TPROXY-recovered original destination selects the server workload's `AllocationId` → its held SVID via `IdentityRead`.
- [ ] Server-mTLS: the agent presents the server SVID and `WebPkiClientVerifier` REQUIRE+VERIFY the client's SVID chains to the bundle; a valid client cert → handshake succeeds, kTLS-RX armed (`ss -tie` `rxconf:sw`).
- [ ] Byte-exact plaintext to S: the server workload reads the byte-exact request as plaintext; the client-facing leg carries `0x17` app_data only (cleartext-marker hits = 0); the decrypted plaintext appears ONLY on the agent→S leg.
- [ ] Agent-light: `strace` shows the agent moves the inbound payload via `splice`/`ppoll` only (zero per-byte payload I/O); leg C carries no psock on its RX (same plain-kTLS-RX invariant as the outbound return).

#### Technical Notes

- The inbound REQUEST direction is proven COMPOSED end-to-end on a loopback topology
  in `findings-inbound-intercept.md` increment-i §2 (*ok* mode, kernel 7.0): TPROXY
  intercept → orig-dst recovery → server-mTLS (`WebPkiClientVerifier` VERIFIES C's
  client SVID) → kTLS-RX arm → agent-light splice-to-S byte-exact, all in ONE flow,
  fail-closed on `nocert`/`wrongca` (§4). What is NOT yet proven for inbound is (a)
  the **response leg** (re-encrypt the server's reply onto leg C's kTLS-TX — the spike
  drove only the request direction; `findings-inbound-intercept.md` § "What was NOT
  tested") and (b) the **real netns/veth topology** (the spike was loopback + sibling
  processes). The slice productionises the identity-selection lookup (orig-dst →
  `AllocationId` → SVID via `IdentityRead`, which the spike hardcoded) and re-proves
  the loopback spike topology in the real netns/veth shape; the full bidirectional
  inbound flow (incl. the response leg) in that topology is demonstrated by the
  BLOCKING composed walking skeleton (Slice 00).
- The fail-closed negatives (nocert/wrongca, distinct reasons) are Slice 05; the verifier
  REQUIRE+VERIFY is wired here, the dedicated negative proofs are S05.

---

### US-MTLS-05 — the guardrails: fail-closed (cause-distinct), resource limits, pump supervision, intercept-exemption negatives, the honest authn boundary

**Problem**: The encryption guarantee is only real if it CANNOT be bypassed and the
platform claims exactly what it proves: a handshake against an absent SVID (outbound) or
a missing/untrusted client cert (inbound) must **fail closed** cause-distinct; the
bounded pre-arm buffer / handshake deadline / in-flight ceiling must be enforced
fail-closed at their concrete values; a stalled return/deliver pump must be torn down
with no leak; the agent's leg-B dial must provably not be re-intercepted and a workload
must provably be unable to self-exempt; and v1 must be documented as
**chain-to-bundle transport authn + encryption ONLY — NO intended-peer pinning**. **The
authoritative ACs are in
`slices/slice-05-fail-closed-limits-supervision-authn-boundary.md`.**

**Who**: Platform/security engineer | threat-modelling the bypass paths and the boundary
| wants fail-closed on wrong/absent/untrusted creds, enforced resource limits, supervised
pumps, an un-evadable intercept, and an honest v1 claim.

**Solution**: Fail-closed (both directions, cause-distinct): outbound `IdentityRead`
`None` → `AbsentSvid`; outbound non-chaining peer → `PeerVerificationFailed`; inbound
`nocert`/`wrongca` → `WebPkiClientVerifier` rejects with a distinct reason per case,
BEFORE any splice (S receives 0 bytes). Resource limits (F4/F7) at concrete values:
`max_prearm_bytes = 256 KiB` → `BufferLimitExceeded`; `handshake_deadline = 5 s` →
`HandshakeTimeout`; `max_inflight_per_alloc = 128` → `InFlightLimitExceeded` — all
fail-closed, cleanup leaks nothing. Pump supervision (F6): a return/deliver pump stalled
for `pump_stall_deadline = 30 s` with a record pending is `Stalled` → the worker tears
the connection down. Intercept-exemption negatives (F5): the agent's leg-B dial is NOT
re-intercepted AND a workload CANNOT self-exempt. The honest authn boundary (F1): v1
authenticates chain-to-bundle ONLY, with NO intended-peer pinning (the `PeerIdentityMismatch`
test is `#[ignore]`-gated on #178).

#### Elevator Pitch

- **Before**: an absent/wrong/untrusted cred could fall back to plaintext, an unbounded
  pre-arm buffer is a DoS surface, a stalled pump could strand resources, a workload might
  self-exempt the intercept, and a doc could overclaim intended-peer protection — the
  guarantee would be bypassable or dishonest.
- **After**: a flow with an absent/wrong/untrusted cred produces NO TLS Application Data
  and NO cleartext (fail-closed cause-distinct, both directions); the limits trip their
  cause-distinct errors at their concrete values with no leak; a stalled pump tears down
  and reports `Gone`; a workload that tries to self-exempt is still intercepted; and the
  platform claims exactly chain-to-bundle authn + encryption, no intended-peer pinning.
- **Decision enabled**: Sam decides the encryption cannot be bypassed AND the platform
  claims exactly what it proves — or rejects the feature if a cred leaks cleartext, a
  limit is unenforced, a pump leaks, a workload self-exempts, or a doc/test overclaims
  intended-peer protection.

#### Domain Examples

1. **Fail-closed cause-distinct** — alloc `g7h8i9` reached Running but its SVID is not
   yet held (one reconcile tick behind, #35); `IdentityRead::svid_for(&g7h8i9)` returns
   `None` → `AbsentSvid`, the agent refuses, no cleartext to the peer. Inbound, a client
   presenting no cert (`nocert`) and one with an untrusted CA (`wrongca`) each reject with
   their DISTINCT reason BEFORE any splice; the server workload receives 0 bytes.
2. **Resource limits at concrete values** — a workload streams > 256 KiB of pre-arm
   plaintext while the handshake stalls → `BufferLimitExceeded` (buffer dropped, leg
   reset, no cleartext); a handshake exceeding 5 s → `HandshakeTimeout`; the 129th
   concurrent in-flight connection for one alloc → `InFlightLimitExceeded`.
3. **Pump supervision + intercept exemption + honest boundary** — a return/deliver pump
   whose bytes-spliced counter has not advanced for 30 s with a record pending is
   `Stalled` → the worker tears the connection down → `Gone`, no leak; the agent's leg-B
   dial is NOT re-intercepted and a workload setting the bypass on its own socket is STILL
   intercepted; v1 verifies chain-to-bundle ONLY, the `PeerIdentityMismatch` test is
   `#[ignore]`-gated on #178 and no doc/test calls the wrong-but-valid-peer case
   "protected."

#### UAT Scenarios (BDD)

##### Scenario: A handshake against an absent or untrusted identity fails closed, cause-distinct
Given an intercepted connection whose held SVID is absent (outbound), or whose client presents no cert or an untrusted-CA cert (inbound)
When the platform attempts the handshake
Then the connection fails closed with a cause-distinct reason, no application data, and no cleartext of the payload
And no plaintext is delivered to the server workload

##### Scenario: Resource exhaustion and a stalled pump are bounded and fail-closed
Given a workload that streams into the pre-arm buffer while the handshake stalls, or a return/deliver pump that strands with a record pending
When the bounded limit is exceeded or the pump stalls past its deadline
Then each limit trips its cause-distinct error at its concrete value, the cleanup leaks nothing, and a stalled pump is torn down and reports Gone

##### Scenario: A workload cannot evade interception and the v1 claim is honest
Given a workload trying to self-exempt the intercept, and the platform's v1 security claim
When it opens a connection and a security reviewer reads the claim
Then the workload is still intercepted (the bypass is agent-private), and the claim is exactly chain-to-bundle authn + encryption with no intended-peer pinning

#### Acceptance Criteria

> Authoritative ACs in `slices/slice-05-fail-closed-limits-supervision-authn-boundary.md`. Summary:

- [ ] Outbound fail-closed: `IdentityRead::svid_for` `None` → `AbsentSvid`; a peer not chaining to the bundle → `PeerVerificationFailed` (no TLS app data, no cleartext). Inbound fail-closed, distinct reasons: `nocert` and `wrongca` each reject with their DISTINCT reason BEFORE any splice; S receives 0 bytes.
- [ ] Resource limits (concrete values): `max_prearm_bytes = 256 KiB` → `BufferLimitExceeded`; `handshake_deadline = 5 s` → `HandshakeTimeout`; `max_inflight_per_alloc = 128` → `InFlightLimitExceeded`; cleanup leaks no fd/pump/kTLS state (re-query `liveness` → `Gone`). Assert the CONCRETE values, not field existence.
- [ ] Pump supervision (F6): a pump stalled for `pump_stall_deadline = 30 s` with a record pending → `Stalled` → the worker tears the connection down → `Gone`, no leak; `mtls.pump.stalled` / `mtls.pump.teardown_on_stall` emitted.
- [ ] Intercept-exemption negatives (F5): the agent's leg-B dial is NOT re-intercepted (no recursion); a workload that sets the bypass on its own socket is STILL intercepted (the bypass is agent-private).
- [ ] Honest authn boundary (F1): a test asserts v1 verifies chain-to-bundle ONLY (both directions); the wrong-but-valid-peer `PeerIdentityMismatch` test is present but `#[ignore]`-gated on #178; NO AC/doc/test calls the wrong-but-valid-peer case "protected" until #178 lands.

#### Technical Notes

- The inbound fail-closed (nocert/wrongca, distinct reasons) is proven in
  `findings-inbound-intercept.md` §4; the resource-limit + pump-supervision tests need
  the deliberately-exceeded-buffer / stalled-handshake / paused-pump harnesses; the
  exemption negatives + the `#[ignore]`-gated boundary placeholder are small.
- Authorization (allow/deny) is the BPF-LSM `socket_connect` hook (#27) fed by
  `policy_verdicts` (#38; related #49) — a SEPARATE subsystem the proxy MUST NOT
  duplicate. Intended-peer SAN-match is the #178 upgrade (VIP path #61). Operator-tunable
  limits are a separate deferral (v1 = compile-time defaults).

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

By the end of #26, v1 host-socket (process/exec) workloads carry TLS 1.3 on the
peer-facing wire with their own SVID, in-kernel, BOTH directions, with the platform
agent **agent-light** (the kernel kTLS engine does all crypto, the agent runs no
cipher; the decrypt/RX pumps are zero-copy `splice`, ~1 splice per record; the
encrypt/TX pumps are a `read → write_all` userspace copy into kTLS-TX) — provable by
a packet capture — with no cleartext on the peer
wire (losslessly, via the userspace handshake buffer), handshakes failing closed
cause-distinct on absent/wrong/untrusted creds, and the agent-light L4 proxy's
**composition** validated on the pinned 6.18 kernel by the BLOCKING composed walking
skeleton.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | v1 host-socket flows (process/exec) | carry TLS 1.3 records on the peer-facing wire (not cleartext), both directions | 100% of established host-socket flows carry TLS 1.3 Application Data on the peer leg; 0% carry cleartext payload (fail-closed) | 0% (no enforcer exists — the CA mints, the IdentityMgr holds, nothing encrypts the wire) | Tier-3 `tcpdump` on the peer-facing leg: TLS 1.3 records (0x17) present, cleartext payload absent, both directions (Slices 03/04) | Leading (North Star) |
| K2 | cleartext bytes on the peer wire before encryption is armed | egress during the handshake window | 0 cleartext bytes on the peer wire before kTLS arms (the pre-arm plaintext is captured losslessly in the agent's userspace buffer and flushed after the handshake — no dropped bytes, no RESET) | n/a (no enforcer ⇒ no arm path today) | Tier-3: the pre-arm plaintext arrives at the peer exactly once, in order, as the first application_data; the peer capture never shows cleartext before 0x17 (Slices 03/04) | Guardrail |
| K3 | handshakes against an absent/wrong/untrusted cred | fall back to plaintext / proceed in cleartext | 0 plaintext fallbacks — every absent SVID (outbound) and missing/untrusted client cert (inbound `nocert`/`wrongca`) handshake fails closed cause-distinct (no TLS App Data, no cleartext) | n/a | Tier-3 negative test: outbound absent SVID (IdentityRead None) and non-chaining peer; inbound nocert/wrongca each fail closed with a distinct reason, 0 bytes to S (Slice 05) | Guardrail |
| K4 | the platform agent | runs the cipher in the steady-state per-byte data path (the ztunnel anti-pattern) | agent AGENT-LIGHT, not a userspace-crypto proxy: the kernel `tls_sw` does ALL crypto, the agent runs no cipher in either direction. The decrypt/RX pumps (outbound return, inbound deliver) are zero-copy `splice` out of kTLS-RX (~1 splice/record, no userspace plaintext copy); the encrypt/TX pumps (outbound forward, inbound response) are a `read → write_all` userspace COPY into kTLS-TX (per-record `read`+`write`, NOT zero-copy — a splice into kTLS-TX loses records). kTLS ULP armed on the agent's peer-facing leg (in-kernel encryption) | n/a | Tier-3 `strace`: decrypt/RX = only `splice`/`ppoll` (no per-byte payload copy); encrypt/TX = per-record `read`+`write` into kTLS-TX (a userspace plaintext copy, the kernel encrypts each write — agent runs no cipher); `ss -tie` shows the kTLS ULP on the agent's leg (Slices 03/04) | Guardrail |
| K5 | a return/deliver splice pump / a resource-exhausting workload | strands resources or pins the agent (unbounded pre-arm buffer / stalled pump) | every limit fail-closed at its concrete value (256 KiB pre-arm / 5 s handshake / 128 in-flight / 30 s pump-stall); a stalled pump is torn down (Gone, no leak) | n/a (no enforcer ⇒ no held proxy state today) | Tier-3: each limit trips its cause-distinct error at its concrete value, cleanup leaks nothing, a stalled pump reports Gone (Slice 05) | Guardrail |
| K6 | the agent-light L4 proxy's three remaining composition gaps | are closed in the real netns/veth topology (outbound composed in one flow; bidirectional round-trip; cgroup-isolated workloads) | the BLOCKING composed walking skeleton holds end-to-end (real intercept → handshake → kTLS arm → post-arm bidirectional transfer, NO RST, both directions, normal AND delayed timing) | mechanism spike-verified (primitives in isolation AND the INBOUND flow composed end-to-end, `findings-inbound-intercept.md` increment-i §2); three narrow gaps open — outbound-composed-in-one-flow (its pieces proven on separate harnesses; increment-e's steady-state RST was a throwaway-harness limitation, NOT a kernel finding, superseded by increment-f), bidirectional round-trip, real netns/veth topology | Tier-3 composed acceptance test for one flow each way (Slice 00) | Leading (riskiest-assumption gate) |

### Metric hierarchy

- **North Star**: K1 — % of host-socket flows that carry TLS 1.3 records on the
  peer-facing wire (the single signal that on-the-wire enforcement is operationally
  true, the reason J-SEC-003 exists).
- **Leading indicators**: K6 (the composed walking skeleton proves the composition —
  the gate that de-risks K1) and K1 itself.
- **Guardrails (must NOT degrade)**: K2 (no cleartext on the peer wire before arm, lossless),
  K3 (handshake fail-closed cause-distinct), K4 (agent agent-light / in-kernel), K5
  (resource limits + pump supervision fail-closed).

### Hypothesis

We believe that transparently intercepting host-socket connections (`cgroup_connect4`
outbound / TPROXY inbound), performing the TLS 1.3 handshake on the agent's peer-facing
leg (rustls, presenting the held SVID read via `IdentityRead`), arming the negotiated
session into kTLS on the agent's leg, and handing steady state to the kernel (forward,
return, and deliver all agent-light `splice` pumps) will, for v1 host-socket workloads, make
principles 2 + 3 operationally true on the wire, both directions. We will know this is
true when **100% of established host-socket flows carry TLS 1.3 records on the peer-facing
wire (K1)**, with 0 cleartext on the peer wire before arm (K2, lossless), every
absent/wrong/untrusted-cred handshake failing closed cause-distinct (K3), and the agent
agent-light (K4) — gated by the composed walking skeleton holding on the pinned kernel (K6).

### Handoff to DEVOPS (platform-architect)

- **Data collection**: the observables are TEST-tier — `tcpdump` showing TLS 1.3
  records on the **peer-facing leg** (the agent's kTLS leg), `ss -tie` showing the
  kTLS ULP on the **agent's peer-facing leg** (kTLS is installed there in the proxy
  model, not on the workload socket), `strace` of the agent showing the per-direction
  agent-light cost (decrypt/RX pumps = `splice`/`ppoll` only, zero-copy; encrypt/TX
  pumps = per-record `read`+`write` into kTLS-TX, a userspace copy), the pump-stall liveness
  telemetry (`PumpLiveness::Stalled` → `mtls.pump.stalled` / `mtls.pump.teardown_on_stall`),
  and the `MtlsLimits` rejection errors (`max_prearm_bytes` → `BufferLimitExceeded`,
  `handshake_deadline` → `HandshakeTimeout`, `max_inflight_per_alloc` →
  `InFlightLimitExceeded`) — instrument the Tier-3 harness to capture and assert on
  them (the EDD verification catalogue, `verification/expectations/`, graduates the
  operator-surface/qualitative ones).
- **Baselines**: K1–K5 baseline at 0% / n/a (no enforcer exists today); K6 is the
  spike verdict. Record first-GA measurements in this feature's evolution record
  (NOT in `kpi-contracts.yaml`, which is the docs-platform feature's
  single-feature contract per its scope note).
- **Guardrail thresholds**: K2/K3/K4/K5 are binary (0 cleartext / 0 fallback /
  agent-light, no per-byte userspace copy — the agent is in the data path for
  handshake + splice setup, splice-mediated with no per-byte copy, not absent / 0
  broken) — any violation is a blocking test failure, not a degradation warning.

---

## Wave: DISCUSS / [REF] Open Questions (resolved by DESIGN / ADR-0069)

DISCUSS pinned the WHAT (the wire carries TLS 1.3 with the workload's own SVID,
in-kernel, fail-closed, both directions) and left the mechanism un-pinned. The DESIGN
wave settled every open question; they are recorded here as closed inputs:

1. **Mechanism choice — RESOLVED (ADR-0069): the universal agent-light L4 proxy.** The 6
   committed Tier-3 spikes settled it empirically (verdict: proxy); the
   in-band-on-the-workload's-own-socket model is superseded as v1 (out of v1 scope —
   a post-v1 optimization tracked in **#231**; ADR-0069 A1), and there is no documented fallback to adopt. The exact
   `MtlsEnforcement` signature (OQ-1) is now ACCEPTED (user-approved 2026-06-12 —
   see the DESIGN § "MtlsEnforcement Port Contract (ACCEPTED)").
2. **Lossless capture for all protocol kinds — RESOLVED (ADR-0069):** the agent's
   userspace handshake buffer is lossless for every protocol kind (client- or
   server-first); no dropped pre-arm bytes, no RESET, no kernel patch.
3. **Three narrow composition gaps — gated by Slice 00 (the BLOCKING composed walking
   skeleton).** The mechanism is spike-verified — primitives in isolation AND the inbound
   flow composed end-to-end (`findings-inbound-intercept.md` increment-i §2). Three narrow
   gaps remain: the outbound path composed in one flow (its pieces proven on separate
   harnesses; increment-e's steady-state RST was a throwaway-harness limitation, NOT a
   kernel finding, superseded by increment-f), bidirectional round-trip, and the real
   netns/veth topology. Slice 00 is the empirical integration gate that closes them; every
   later slice is additive on the composed walking skeleton.
4. **The exact intercept attach mechanism + the kTLS `crypto_info` struct mapping** are
   pinned in the DESIGN sections (grounded on the committed spike findings); the
   `MtlsEnforcement` method signatures (OQ-1) are ACCEPTED (user-approved 2026-06-12).

None of these blocked the DISCUSS handoff — DISCUSS pins WHAT, not HOW; the DESIGN wave
(ADR-0069) owns the mechanism and is recorded in the `## Wave: DESIGN` sections below.

---

## Wave: DISCUSS / [REF] Definition of Ready (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Each story's Problem is in security-engineer domain language (identity-unaware workloads, host sockets, the held SVID, the agent's peer-facing leg, fail-closed, agent-light) — no "implement sockops". |
| 2 | User/persona with specific characteristics | PASS | Sam Okafor (`sam-platform-security-engineer.yaml`) with a J-SEC-003 lens — threat-models by default, verifies with `tcpdump`/`ss -tie`/`openssl verify`, distrusts security that can be turned off. |
| 3 | 3+ domain examples with real data | PASS | Every story has 3 examples with real allocs (`a1b2c3`/`d4e5f6`/`g7h8i9`), real SPIFFE URIs (`spiffe://overdrive.local/job/web/alloc/a1b2c3`), real protocols (HTTP/gRPC), real workloads (`web`/`api`/`coinflip`). No `user123`. |
| 4 | UAT in Given/When/Then (3–7 scenarios) | PASS | Each story has 2–3 business-outcome scenarios (titles describe WHAT the user achieves — "the wire carries TLS 1.3 records", "a handshake against an absent identity fails closed" — never "sockops fires" or kTLS struct names). 13 scenarios across 6 stories. |
| 5 | AC derived from UAT | PASS | Each story's ACs are derived from its scenarios + the four-forces/job-map edge cases; observable + testable (tcpdump/`ss -tie`/agent-light strace/fail-closed negative), not implementation ("use TLS_TX struct X"). |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | 6 ≤1–1.5-day slices (Slice 00 = the BLOCKING composed walking skeleton); each story 4–5 ACs, 2–3 scenarios. Scope assessment PASS (zero oversized signals). |
| 7 | Technical notes: constraints/dependencies | PASS | § System Constraints (cross-cutting) + per-story Technical Notes; the mechanism is deliberately NOT pinned (DESIGN's), with the spike as the gate. |
| 8 | Dependencies resolved or tracked | PASS | Consumes shipped #35 (`IdentityRead` + `Arc<IdentityMgr>`) + #28 (`Ca` + leaf key) + ADR-0068 (kernel). Carve-outs tracked by real issue #: #222 (guest-stack), #229 (rekey), #40 (rotation), Phase 5 (revocation); multi-node transparent mTLS is OUT of v1 scope (Phase 1 is single-node), no forward-pointer issue. NO DIVERGE risk recorded in `wave-decisions.md`. |
| 9 | Outcome KPIs defined with measurable targets | PASS | K1–K6 with Who/Does-what/By-how-much/Baseline/Measured-by; North Star K1 (100% host-socket flows carry TLS 1.3), guardrails K2–K5 (binary: 0 cleartext / 0 fallback / agent-out / 0-broken), gate K6 (spike verdict). |

**DoR Status: PASSED** (pending peer review). Elevator-Pitch gate: every story
(including the composed walking skeleton) has a Before/After/Decision-enabled triplet
whose "After" references a real executable observable (`tcpdump`/`ss -tie`/agent-light
`strace`/negative test); the foundation-feature exception (no operator verb; TEST-tier
observables) is recorded explicitly above (§ User Stories preamble), in
`wave-decisions.md` (D1), and here — mirroring built-in-ca and #35. Slice-composition
gate: no slice is empty `@infrastructure` — every slice has a genuine wire-capture /
`ss -tie` / fail-closed observable.

---

## Wave: DISCUSS / [REF] JTBD Traceability + Handoff

- **Every story traces to `job_id: J-SEC-003`** (the on-the-wire enforcement job,
  `docs/product/jobs.yaml` § J-SEC-003). N:1 mapping — 6 stories → 1 job.
- **Hands off to**: `solution-architect` (DESIGN) — journey (visual + YAML) +
  story map + user-stories + outcome KPIs + the BLOCKING composed walking skeleton;
  `acceptance-designer` (DISTILL) — the journey YAML (embedded Gherkin) + integration
  points + outcome KPIs; `platform-architect` (DEVOPS) — outcome KPIs (Tier-3
  wire-capture / `ss -tie` / strace agent-light instrumentation).
- **The DESIGN wave resolved the mechanism** (ADR-0069 — the universal agent-light L4
  proxy; the in-band-on-the-workload's-own-socket model is superseded as v1 and is not
  in v1 scope). DISCUSS pinned the WHAT and the acceptance observables, not the
  HOW (no kTLS struct shapes, no intercept attach mechanism); the agent's userspace
  handshake buffer is lossless, so no kernel patch is needed.

---

## Wave: DESIGN / [REF] Mechanism Decision (the locked decision, formalized)

**Agent**: Morgan (nw-solution-architect) · **Density**: `lean` + `ask-intelligent`
(Tier-1 `[REF]`) · **Mode**: formalize a user-LOCKED decision (2026-06-12) on
complete, kernel-source-pinned empirical evidence (6 Tier-3 spikes + 3 research
docs, kernel 7.0, `353cdc52`). **NOT relitigated** — designed and recorded.

**The decision** (ADR-0069): fold #222 into #26. Build ONE universal **transparent
mTLS via an agent-light L4 proxy** as THE enforcement mechanism for ALL workload
kinds (process/exec, WASM, microVM, unikernel). The previously-primary in-band
kTLS-on-the-workload's-own-socket model is SUPERSEDED as v1 and is out of v1 scope
— a post-v1 optimization tracked in **#231** (ADR-0069 A1) (it uniquely wins
restart-survival + 1-socket density; loses uniformity + losslessness). The **mechanism is
spike-verified** — the primitives in isolation AND the INBOUND flow composed end-to-end
in one direction (`findings-inbound-intercept.md` increment-i §2). **Three NARROW
composition gaps remain** — the outbound path composed in one flow (its pieces proven on
separate harnesses; increment-e's steady-state RST was a throwaway-harness limitation,
NOT a kernel finding, superseded by increment-f's clean-harness proof), bidirectional
round-trip, and the real netns/veth topology. Closing gaps 1–3 — composing the proven
pieces into ONE bidirectional walking skeleton in the real netns/veth topology — is the
FIRST DELIVER slice (a BLOCKING composed Tier-3 walking skeleton; an integration gate, NOT
a "prove-the-mechanism" gate; see the DESIGN Handoff § + ADR-0069 § Enforcement). No
Cilium fallback; no kernel patch.

**This amends** whitepaper §7/§8 (two enforcement mechanisms → one) and the DISCUSS
"mechanism is NOT pinned" framing. It re-grounds the back-propagation flagged in
`design/upstream-changes.md` (J-SEC-003 + slices 00–05).

## Wave: DESIGN / [REF] Proven-Mechanism Traceability

**Every design element below rests on a COMMITTED spike finding — not assertion,
not re-derivation.** The mechanism was empirically settled by 6 Tier-3 spikes (5
follow-ups + the original) on a real 7.0 kernel (≥ the pinned 6.18 floor,
ADR-0068), software kTLS, AES-256-GCM TLS 1.3, committed under
`docs/feature/transparent-mtls-host-socket/spike/` at `353cdc52`. **DELIVER
implements to the ADR-0069 contract using these findings as the reference for the
exact syscalls / flags / ordering — it does NOT re-discover the mechanism.** The
matrix maps each design element → the committed finding (doc + verdict/section)
that proved it → the proven constant/pattern DELIVER reuses → the gitignored probe
pointer (NON-DURABLE; see § Durability below).

| Design element | Committed finding (doc · verdict/section) | Proven constant / pattern (the reference DELIVER builds on) | Probe pointer (gitignored, non-durable) |
|---|---|---|---|
| **`MtlsEnforcement::enforce` — rustls→kTLS arm** (`dangerous_extract_secrets` → `ktls` → `setsockopt TCP_ULP "tls"`+`TLS_TX`/`TLS_RX`) | `findings.md` · Increment A WORKS (Unknowns 1+2 CONFIRMED) | `ss -tie`: `tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw`; agent writes/reads plaintext, kernel does crypto; `strings pcap \| grep MARKER` = 0; constants `SOL_TLS=282 TLS_TX=1 TLS_RX=2 TCP_ULP=31 AES_GCM_256=52 sizeof(crypto_info)=56` | `spike-scratch/increment-a/`, `increment-b/` |
| **`MtlsEnforcement::enforce` — forward steady state (agent-light COPY, D-MTLS-13)** a bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX; kernel `tls_sw_sendmsg` encrypts each blocking `write` | `crates/overdrive-dataplane/src/mtls/splice.rs` (the shipped `write_all` forward primitive) + `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` (why the redirect AND splice-into-TX were retired) | the blocking userspace `write_all` to leg B hits `tls_sw_sendmsg` SYNCHRONOUSLY (no `MSG_DONTWAIT`/backlog) → `1703 03` records on the peer wire; agent does ZERO crypto but DOES copy each record's plaintext through a userspace buffer (per-record `read`+`write` — NOT zero-copy, NOT `splice`/`ppoll`-only). A `splice` INTO kTLS-TX is NOT used (it loses records, the same `MSG_DONTWAIT` class). **~~Originally a `bpf_sk_redirect_map(flags=0)` sockmap egress redirect (15/15 spike, `findings-egress-ktls-splice.md`)~~ RETIRED — the redirect's deferred `MSG_DONTWAIT` workqueue `-EAGAIN`-stalls ~10–15% of records on the real kernel.** SHIPPED 20/20, commit `bb6489ef` | (production `crates/.../mtls/`) |
| **`MtlsEnforcement::enforce` — kTLS 0.5-RTT early-data drain (D-MTLS-13)** drain `conn.reader()` before `dangerous_extract_secrets`; forward ahead of the steady-state pump | shipped `mtls::drain_early_plaintext` (CorkStream-equivalent) | the rustls reader buffers any 0.5-RTT early application_data the peer sent during the handshake; the extracted `rx` `rec_seq` (`= read_seq`) already advanced past those records, so they'd be dropped if left in `conn.reader()`. Drained + forwarded → no loss | (production `crates/.../mtls/mod.rs`) |
| **return splice pump (agent-light) + `liveness`** `splice(legB→pipe→legF)` via `tls_sw_splice_read` on a **plain, no-psock** kTLS-RX leg | `findings-splice-return.md` · verdict (a) CONFIRMED, Assertions 1–4 | `strace` = only `splice`/`ppoll`, ZERO payload read/write; byte-exact (86 / 100000 B); ~1 `splice` per TLS record (≤16384 B/record); clean decrypted payload (no header/inner-type/tag); `einval_on_B=0` | `spike-scratch/increment-h-splice-return/` |
| **leg B carries NO psock/verdict on its RX** (D-MTLS-5; the return-`splice` precondition) | `findings-ktls-rx-splice.md` · verdict (b) NO (return-via-verdict foreclosed) + `findings-egress-ktls-splice.md` mechanic #2 | a psock on leg B's RX fights kTLS RX (`ConnectionAborted` 103) AND `tls_sw_read_sock` returns `-EINVAL` on a psock → no agent-idle push; the `splice`-on-no-psock leg sidesteps both | `spike-scratch/increment-g-ktls-rx-splice/` |
| **transparent intercept** `cgroup/connect4`-rewrite → agent `accept()`s leg F; lossless `recv()` capture | `findings-userspace-relay.md` · Unknown 1 WORKS; Unknown 2 PARTIAL (handshake-window LOSSLESS) | `cgroup/connect4` rewrites dest to the agent listener (workload unaware); ordinary `recv()` drains pre-arm plaintext (80/80); flush as in-order TLS 1.3 `0x17`, zero plaintext on leg B; gate/route the splice by `local_port` only (the `sk_msg`/`sock_ops` `local_ip4` byte-order disagreement) | `spike-scratch/increment-e-userspace-relay/` |
| ~~**arming invariant** SOCKMAP-insert-before-`TCP_ULP "tls"` (D-MTLS-7; `probe` check 3; `ArmingOrderViolation`)~~ **MOOT 2026-06-13 (D-MTLS-13)** | `findings.md` · Increment D (historical) | ~~reverse ordering = `EINVAL`; Tier-3 AC `tls-ULP-after-sockmap == EINVAL`~~ — with the sockmap retired there is no SOCKMAP insert on any path; the invariant is a true kernel fact but governs no Overdrive code path. The `D-MTLS-7` decision, the `probe` sub-sentinel, the `ArmingOrderViolation` variant, and the Tier-3 AC are all retired. | (n/a — retired) |
| **control records** (`NewSessionTicket`/KeyUpdate → `EIO` on raw kTLS RX → reuse `ktls::KtlsStream`) | `findings.md` · Increment B load-bearing finding + Design implication #4 | raw-kTLS RX only decrypts `application_data`; a control record returns `EIO`; `ktls::KtlsStream` runs the control-message loop → favoured over raw `setsockopt` | `spike-scratch/increment-a/`, `increment-b/` |
| **why proxy, NOT in-band** (D-MTLS-1/2; ADR-0069 Alternatives A1/A2) — in-band lossless foreclosed 3 ways | `findings-lossless-hybrid.md` (source-TX-bypass RST 3/3) · `findings-userspace-relay.md` Unknown 4 (#222-collapse) · `findings.md` Increment C (no `sk_msg` HOLD) + `sockmap-redirect-live-socket-liveness-research.md` / `sockops-ktls-lossless-hold-bpf-only-research.md` | (1) `sk_msg` has PASS/DROP/REDIRECT, no HOLD → pre-arm write `SK_DROP`→`EACCES`+dead conn; (2) redirecting a live socket's own egress bypasses its TX → RST; (3) lossless capture structurally requires a 2-socket proxy ⇒ the lossless variant of #26 *is* #222 | `spike-scratch/increment-{c,d,e}/` |

### Durability — committed findings are the foundation of record; `spike-scratch/` is throwaway

The **committed findings docs** (`spike/findings*.md`, at `353cdc52`) are the
**durable anchor and the foundation of record** for every citation above and in the
ADR / contract / upstream-changes. They survive a clean checkout and are the SSOT
DISCUSS re-grounds onto and DELIVER references.

The probe code under `spike-scratch/increment-{a..h}/` is **gitignored, throwaway,
per nW-spike discipline** — it may NOT survive a clean checkout, was never promoted,
and touched no `overdrive-*` API. It is cited above ONLY as a secondary convenience
pointer (a reviewer with the working tree may inspect it). **DELIVER may consult it
if present but MUST NOT depend on it**; the load-bearing evidence is always the
committed finding, never the probe dir. The throwaway code is NOT to be committed —
it stays throwaway.

## Wave: DESIGN / [REF] Domain (DDD) — bounded context + ubiquitous language

A single bounded context: **transparent-mTLS enforcement** (a supporting subdomain
of the security/identity core — it *consumes* identity, it does not own it). The
ubiquitous language, pinned so the crafter and acceptance-designer share terms:

| Term | Meaning |
|---|---|
| **leg F** | The agent-owned **plaintext** leg facing the workload (the intercept destination). |
| **leg B** | The agent-owned **kTLS** leg facing the real peer (carries TLS 1.3 records). |
| **transparent intercept** | Rewriting the workload's `connect()` destination to the agent's leg-F listener (`cgroup_connect4`-rewrite default; TPROXY alt) so the workload is unaware. |
| **handshake window** | The setup phase: drain pre-arm plaintext losslessly → rustls handshake on leg B → arm kTLS → flush captured plaintext. Userspace, lossless. |
| **forward encrypt pump (agent-light COPY, D-MTLS-13)** | Steady-state plaintext→ciphertext: a bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX; the kernel `tls_sw_sendmsg` encrypts each blocking `write`. Per-record `read`+`write`, plaintext copied through a userspace buffer — **NOT zero-copy, NOT a splice** (a `splice` into kTLS-TX loses records). The agent runs no cipher. (Replaced the agent-idle sockmap EGRESS-redirect, retired 2026-06-13; `crates/overdrive-dataplane/src/mtls/splice.rs`.) |
| **return / deliver splice (agent-light)** | Steady-state ciphertext→plaintext: `splice(legB → pipe → legF)` (outbound return) / `splice(legC → pipe → legS)` (inbound deliver) on a plain (no-psock) kTLS-RX leg (`tls_sw_splice_read`). Zero-copy, ~1 splice/record. |
| **early-data drain (D-MTLS-13)** | Before `dangerous_extract_secrets` arms kTLS-RX, drain `conn.reader()` of any 0.5-RTT early application_data and forward it ahead of the splice pump — the extracted `rx` `rec_seq` already accounts for the over-read records (`mtls::drain_early_plaintext`, the `ktls::CorkStream`-equivalent userspace drain). |
| **held SVID** | The workload's `SvidMaterial` (cert + leaf key), read via `IdentityRead`, presented by the agent; the workload holds NOTHING. |
| ~~**arming invariant**~~ (MOOT 2026-06-13, D-MTLS-7/13) | ~~SOCKMAP insert MUST precede `TCP_ULP "tls"` on leg B~~ — no sockmap insert on any path now. Retained as a historical kernel fact only. |

No aggregates own durable state in this context (the proxy is per-connection,
ephemeral; identity is owned by `IdentityMgr`). No new domain events beyond the
existing flow telemetry.

## Wave: DESIGN / [REF] Component Decomposition

(Full table + rationale in `docs/product/architecture/brief.md` § "Transparent mTLS
— universal agent-light L4 proxy extension" § 3.)

1. **`MtlsEnforcement` port** — `overdrive-core` (`core`, pure trait). The driven
   contract: intercept-arm → drive-handshake-and-arm-kTLS → run-steady-state-splice
   → teardown. Behaviour pinned in rustdoc + a DST equivalence harness.
2. **`HostMtlsEnforcement` adapter** — EXTEND **`overdrive-dataplane`**
   (`adapter-host`; the established userspace eBPF host adapter hosting
   `EbpfDataplane` — unsafe allowed, `aya` + BPF `build.rs` already present;
   OQ-2 resolved). The production proxy over
   sockops/sk_msg/sockmap/kTLS/`splice`/`cgroup_connect4`; consumes `IdentityRead`;
   reuses `ktls::KtlsStream`. (`overdrive-host` ruled out — `#![forbid(unsafe_code)]`.)
3. **`SimMtlsEnforcement` adapter** — `overdrive-sim` (`adapter-sim`). The DST
   double modelling the observable contract in-memory.
4. **mTLS proxy agent** — `overdrive-worker` (EXTEND; the node-agent home). Owns the
   per-connection lifecycle and the return splice pump supervision.
5. **New BPF programs** — `overdrive-bpf` (EXTEND): the `cgroup_connect4`
   mtls-variant (outbound intercept) + the `nft`-TPROXY inbound intercept setup.
   **(REVISED 2026-06-13, D-MTLS-13: the sockops `sock_ops_mtls_enroll` and the
   `sk_skb/stream_verdict` forward egress-redirect programs were RETIRED with the
   sockmap — the forward path is now a userspace `splice` pump with no kernel-side
   program. The aya-ebpf-0.1.1-`#[sk_skb]`-macro hand-roll note is obsolete.)**

## Wave: DESIGN / [REF] Ports (driving + driven)

### Driving (primary) port

**None operator-facing** (D1 foundation; no CLI verb — encryption is automatic and
undisableable). The driving surface is the **kernel-originated
connection-detect/intercept event** (the workload's transparently-rewritten
`connect()` + the sockops ESTABLISHED transition) that drives the agent's
per-connection enforcement. Acceptance surface is TEST-tier (`tcpdump` / `ss -tie`
/ fail-closed / race-window probe).

### Driven (secondary) port — `MtlsEnforcement` (CREATE-NEW)

**Why not `Dataplane`**: `Dataplane` models map writes (policy/service/local-
backend), keyed by service/policy identity — NOT per-connection socket operations.
The proxy's lifecycle (intercept a `connect()`, drive a handshake on an acquired
socket, arm kTLS on a leg, run a splice pump, tear down) is a different abstraction
with a per-connection lifecycle. CREATE-NEW is justified (ADR-0069 Decision).

**The model is fixed by ADR-0069; the exact method signatures are pinned here as
DESIGN decisions — the crafter MUST NOT invent public surface beyond this**
(CLAUDE.md "Implement to the design"). The port carries the four lifecycle phases.
The shape below is the **DESIGN-named contract** (object-oriented paradigm;
`async_trait` at the adapter-host boundary, never on a `core` compile path —
mirroring `Dataplane`):

- **`probe(&self) -> Result<(), MtlsEnforcementError>`** — Earned-Trust:
  wire→probe→use. Verify the kTLS arm + agent-light forward-encrypt round-trips
  (sentinel handshake mints a throwaway `rcgen` cert → arm kTLS-TX/RX → the forward
  encrypt pump `read → write_all`s one record into kTLS-TX (the EXACT production
  forward primitive, NOT a splice into TX) → peer's kTLS-RX `tls_sw_splice_read`
  reconstructs it byte-exact; reader leg drains 0.5-RTT early data). One composed
  sentinel (D-MTLS-13). Refuse-to-start (`health.startup.refused`) on failure.
- **A per-connection drive method** that, given the detected connection + the
  `AllocationId` (whose SVID to present), performs: lossless capture → rustls
  handshake (reading `IdentityRead::svid_for` + `current_bundle`) → drain 0.5-RTT
  early data → kTLS arm on leg B → start the agent-light forward `read → write_all`
  copy pump →
  return `Ok` once steady-state is established (or a typed fail-closed error on
  absent/wrong SVID or handshake failure). The adapter then drives both splice
  pumps; the worker supervises liveness.
- **A teardown method** for connection close (release legs, drop the pump).

> **OQ-1 — ACCEPTED (user-approved 2026-06-12)**: the EXACT method
> names, parameter types (the drive method takes an owned fd / an
> `AllocationId` + a connection descriptor — see SD-1…SD-4), and the return/error
> type (`MtlsEnforcementError` variants) are pinned in the § "MtlsEnforcement Port
> Contract (ACCEPTED)" section below — so the crafter does NOT improvise surface
> (the `workflow-result-error-model` precedent in CLAUDE.md). The ADR fixes the
> *model*; the *signature* is the accepted contract DELIVER implements to. The
> bidirectional 4-method shape (`probe`/`enforce`-dispatch-on-`Direction`/`liveness`/`teardown`)
> is the locked wire shape of the connection handle.

### Consumed port — `IdentityRead` (REUSE AS-IS)

The agent reads `svid_for(&AllocationId)` (present in the handshake) +
`current_bundle()` (verify the peer). `None` is the fail-closed signal (refuse the
handshake). #26 is a READER — never mints/re-issues/caches.

## Wave: DESIGN / [REF] Technology Choices

| Choice | Selection | Rationale | License |
|---|---|---|---|
| TLS 1.3 handshake | `rustls 0.23 [ring]` (in workspace) | Already the workspace TLS (ADR-0039/built-in-ca); `dangerous_extract_secrets()` is the kTLS-arm seam (spike-proven) | MPL-2.0 (rustls) / ISC+MIT (ring) — OSS |
| kTLS arm + control records | `ktls` crate 6.x (NEW dep) | `findings.md` #4: `NewSessionTicket`/KeyUpdate → `EIO` on raw kTLS RX; `ktls::KtlsStream` runs the control-message loop. Favoured over raw `setsockopt` | MIT / Apache-2.0 — OSS |
| BPF loader | aya 0.13.x (in workspace) + `pinning = ByName` (`/sys/fs/bpf/overdrive`) | The established loader + bpffs-pin discipline (ADR-0038/0040); reuse, do not reinvent | MIT / Apache-2.0 — OSS |
| Transparent intercept | `cgroup/connect4`-rewrite (default) — extends `cgroup_connect4_service` | Proven (`findings-userspace-relay.md` Unknown 1); reuses the connect4-rewrite shape | (in-tree) |
| Forward encrypt pump (D-MTLS-13) | bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX; kernel `tls_sw_sendmsg` encrypts each blocking `write` | Agent-light but a COPY (per-record `read`+`write`, NOT zero-copy, NOT a splice into TX — which loses records); NOT symmetric to the return's zero-copy splice; replaced the `bpf_sk_redirect_map(flags=0)` redirect (`MSG_DONTWAIT`-stall, `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`); SHIPPED 20/20 | (kernel + userspace copy) |
| Return / deliver splice | `splice(2)` + `tls_sw_splice_read` on a plain kTLS-RX leg | Agent-light zero-copy, ~1/record (`findings-splice-return.md`) | (kernel) |
| Early-data drain (D-MTLS-13) | userspace drain of `conn.reader()` before `dangerous_extract_secrets` (CorkStream-equivalent) | kTLS 0.5-RTT early application_data left in the rustls reader is otherwise dropped (the `rx` rec_seq already advanced past it); SHIPPED `mtls::drain_early_plaintext` | (`ktls` / in-tree) |
| Kernel floor | pinned 6.18 LTS (ADR-0068) | In-kernel TLS 1.3 TX+RX + `CONFIG_NET_HANDSHAKE` + splice/sockmap guaranteed; no kernel patch | (appliance) |

OSS-first honored; no proprietary tech. The one new dependency (`ktls`) is
MIT/Apache-2.0 and well-maintained.

## Wave: DESIGN / [REF] Decisions Table

| # | Decision | Rationale | Source |
|---|---|---|---|
| D-MTLS-1 | ONE universal agent-light L4 proxy for all workload kinds; fold #222 into #26 | Uniformity + losslessness over restart-survival + density (user-locked) | ADR-0069; user 2026-06-12 |
| D-MTLS-2 | In-band kTLS-on-own-socket = out of v1 scope — a post-v1 optimization tracked in **#231** (ADR-0069 A1) | No lossless client-speaks-first path in-band (foreclosed 3 ways); not universal | `findings-{lossless-hybrid,userspace-relay}.md` |
| D-MTLS-3 | NEW driven port `MtlsEnforcement`; do NOT reuse `Dataplane` | Per-connection socket ops ≠ map writes | ADR-0069; `dataplane.rs` |
| D-MTLS-4 | **(REVISED 2026-06-13 — see D-MTLS-13)** Forward and return are BOTH agent-light (no userspace crypto) but NOT the same primitive: the DECRYPT/RX directions (outbound return `splice(legB → legF)`, inbound deliver) are zero-copy `splice(2)` out of a plain (no-psock) kTLS-RX leg (`tls_sw_splice_read` decrypts on splice-out); the ENCRYPT/TX directions (outbound forward `read → write_all`(legF → legB), inbound response) are a bounded userspace COPY into a kTLS-TX leg (kernel `tls_sw_sendmsg` encrypts each `write`; per-record `read`+`write`, NOT zero-copy). A `splice` INTO kTLS-TX loses records, so the encrypt directions use a blocking `write_all`. ~~Forward = sockmap EGRESS-redirect (agent-idle)~~ — RETIRED (the redirect `MSG_DONTWAIT`-stalls ~10–15%) | The proven agent-light primitives: zero-copy `splice` out of kTLS-RX; `write_all` copy into kTLS-TX | `findings-splice-return.md` (the RX splice); `crates/overdrive-dataplane/src/mtls/splice.rs` (the TX `write_all`); `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` (forward pivot) |
| D-MTLS-5 | leg B carries NO psock/verdict on its RX (now: no psock on ANY leg, either direction — the sockmap is gone) | psock fights kTLS RX (`ConnectionAborted`) + forecloses the agent-idle path (`tls_sw_read_sock -EINVAL`) | `findings-ktls-rx-splice.md` |
| D-MTLS-6 | Transparent intercept = `cgroup_connect4`-rewrite (outbound), TPROXY (inbound) | Proven; reuses existing connect4-rewrite shape | `findings-userspace-relay.md`; `findings-inbound-intercept.md` |
| D-MTLS-7 | ~~SOCKMAP-insert-before-`TCP_ULP "tls"` (Tier-3 invariant)~~ **MOOT / SUPERSEDED 2026-06-13** — with the sockmap retired from the forward path (D-MTLS-13) there is NO sockmap insert sequenced against `TCP_ULP` on any leg. The invariant remains a true kernel fact (reverse = `EINVAL`; both replace `sk->sk_prot`) but governs no Overdrive code path; the `tls-ULP-after-sockmap == EINVAL` Tier-3 test is retired. | (kernel fact; no longer a design constraint) | `findings.md` increment D (historical) |
| D-MTLS-8 | Control records via `ktls::KtlsStream`, not raw `setsockopt` | `NewSessionTicket`/KeyUpdate → `EIO` on raw RX | `findings.md` #4 |
| D-MTLS-9 | Agent holds the leaf key (via `IdentityRead`); workload holds nothing | CLAUDE.md identity model; J-SEC-003 | ADR-0067; CLAUDE.md |
| D-MTLS-10 | Process topology = in-process (the proxy agent runs IN the node binary, reading `IdentityRead` in-process — no separate agent process, no gRPC/CSR) | O3 in-process read (whitepaper §7); the agent is `overdrive-worker` control logic, not a sidecar process. Resolves the prior guided-session "in-process control-plane vs separate agent" open item | ADR-0067 D7; whitepaper §7 |
| D-MTLS-11 | Earned-Trust `probe()` mandatory; wire→probe→use; refuse-to-start on failure | principle 12; exercises the catalogued substrate lies | ADR-0069 § Enforcement |
| D-MTLS-12 | `probe`'s handshake sentinel uses a THROWAWAY self-signed cert minted in-process via `rcgen` at call time (promote `rcgen` to `overdrive-dataplane` production dep). Substrate-self-test crypto, signed by NEITHER CA, never in the trust bundle, never on a real wire — #26 stays a reader, NOT an issuer. **STILL LIVE after the 2026-06-13 forward-mechanism pivot** — the shipped `probe` STILL does a loopback rustls handshake (`run_probe_sentinels`), so the rcgen-sentinel-cert core is unchanged; only the *sockmap-engagement / ARMED-gate* portion of the probe's substrate-lie catalogue was mooted (see D-MTLS-13). The handshake sentinel's reader leg now also exercises the kTLS 0.5-RTT early-data drain (D-MTLS-13). | sentinel is handshake-based (needs cert+key); `IdentityRead` is a reader; "workload-holds-nothing"/"two distinct CAs" intact; no key-at-rest in the binary (a-runtime over a-static) | SD-5; user 2026-06-12 |
| D-MTLS-13 | **(2026-06-13) Forward sockmap-egress-redirect → agent-light userspace pump pivot + kTLS 0.5-RTT early-data drain.** (1) The OUTBOUND forward retires the agent-idle `sk_skb/stream_verdict` + `bpf_sk_redirect_map(flags=0)` redirect for an agent-light bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX (the kernel `tls_sw_sendmsg` encrypts each `write`; per-record `read`+`write`, plaintext copied through a userspace buffer — **NOT zero-copy, NOT agent-idle, and NOT symmetric to the return/deliver pumps**, which `splice` zero-copy out of kTLS-RX). A `splice` INTO kTLS-TX is NOT used — it consumes the bytes and reports success but does not reliably emit the record (the same `MSG_DONTWAIT` loss class the redirect suffered). The inbound response leg (S→C) uses the SAME `write_all`-into-kTLS-TX copy. The whole sockmap apparatus (`MTLS_SOCKMAP`/`MTLS_FPORT`/`MTLS_ARMED`, the verdict program, the `sock_ops_mtls_enroll` enroll program, the ARMED gate, the engagement poll) is DELETED. (2) Every reader leg (outbound return, inbound deliver, probe sentinel) drains `conn.reader()` of 0.5-RTT early application_data via `mtls::drain_early_plaintext` BEFORE `dangerous_extract_secrets` arms kTLS-RX (or uses a `ktls::CorkStream`-equivalent) — the extracted `rx` `rec_seq` already accounts for the over-read records, so early data left only in `conn.reader()` would otherwise be silently dropped. Mooted with it: D-MTLS-7 (sockmap-before-`TCP_ULP` invariant) + the probe's `ForwardEgressRedirect`/`ArmingOrderEinval` sub-sentinels + `ArmingOrderViolation`/`ForwardRedirectFailed` error variants. | The `sk_skb` egress redirect defers delivery to a `MSG_DONTWAIT` workqueue that `-EAGAIN`-stalls ~10–15% of records; no production system runs it; the kernel does not test it. A `splice` into kTLS-TX has the same loss; a synchronous blocking userspace `write_all` into kTLS-TX is the correct, reliable mechanism. SHIPPED + verified 20/20 (commit `bb6489ef`). | `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`; `sockmap-strparser-engagement-race-research.md`; `spike/findings-sockmap-engagement-inkernel-enroll.md`; shipped `crates/overdrive-dataplane/src/mtls/splice.rs` |

## Wave: DESIGN / [REF] Reuse Analysis (HARD GATE)

Full table in `brief.md` § "Transparent mTLS … extension" § 6. Summary verdict
tally: **3 REUSE-AS-IS** (`IdentityRead`, `SvidMaterial`/`TrustBundle`, `rustls`) ·
**5 EXTEND** (`cgroup_connect4_service`, `overdrive-bpf`, `overdrive-worker`, the
aya loader/pin pattern, and **`overdrive-dataplane`** as the `HostMtlsEnforcement`
home — OQ-2 **resolved**, no new crate; `overdrive-host` ruled out for
`#![forbid(unsafe_code)]`) · **1 CREATE-NEW port** (`MtlsEnforcement` — `Dataplane`
does not fit) · **1 CREATE-NEW dep** (`ktls`) · **1 PROMOTED dep** (`rcgen`:
test-dep → production dep of `overdrive-dataplane`, for `probe`'s throwaway
self-test sentinel cert — D-MTLS-12 / SD-5; already in the workspace graph, NOT
an issuance path). Default-EXTEND honored throughout; the single CREATE-NEW
(`ktls`) justified, and the `rcgen` promotion is a substrate-self-test edge, not
identity issuance (#26 stays a reader).

## Wave: DESIGN / [REF] Open Questions / Deferrals (blockers for the orchestrator)

These need **user/product-owner decisions BEFORE the crafter dispatch / issue
creation** — surfaced here, NOT resolved unilaterally, NO GH issues created by the
architect (CLAUDE.md "Deferrals require GitHub issues — AND user approval BEFORE
creation").

- **OQ-1 (signature pin) — ACCEPTED (user-approved 2026-06-12)**: the EXACT
  `MtlsEnforcement` method names/params/error type are pinned in the § "MtlsEnforcement
  Port Contract (ACCEPTED)" section. The bidirectional 4-method contract
  (`probe`/`enforce`-dispatch-on-`Direction`/`liveness`/`teardown`,
  `InterceptedConnection { leg, routed, alloc, expected_peer }`, `MtlsLimits`, the
  cause-distinct errors) is the accepted contract DELIVER implements to. No longer a
  blocker.
- **OQ-2 (adapter home) — RESOLVED (user-decided 2026-06-12)**: **no new crate.**
  `HostMtlsEnforcement` EXTENDS **`overdrive-dataplane`** (the established
  `adapter-host` userspace eBPF crate — `unsafe` already allowed, `aya.workspace =
  true` + BPF `build.rs` already present; every reason a new crate would give is
  already satisfied here); the kernel-side `cgroup_connect4`-mtls intercept program
  EXTENDS **`overdrive-bpf`** (one shared BPF object; the sockops/`sk_skb`
  forward-redirect programs were retired 2026-06-13 with the sockmap, D-MTLS-13);
  `SimMtlsEnforcement`
  stays in `overdrive-sim`. **`overdrive-host` ruled out** —
  `src/lib.rs:21` is `#![forbid(unsafe_code)]` and the proxy is irreducibly
  `unsafe`. **Revisit trigger** (not a blocker): if mTLS later needs isolation from
  the LB/service dataplane (so the proxy's `ktls`/`rustls` stack does not couple the
  service-dataplane compile graph), split into a dedicated crate then.
- **In-band restart-survival + 1-socket density — NOT in v1 scope.** The in-band
  kTLS-on-own-socket model's two unique wins (restart-survival + 1-socket density)
  are not pursued in v1 (the proxy trade, ADR-0069 A1) — a post-v1 optimization
  tracked in **#231**.
- **Multi-node — OUT of v1 scope (Phase 1 is single-node).** Cross-node transparent
  mTLS (the peer on a different node) is out of #26's single-node v1 scope; the
  proxy contract is `SocketAddrV4` / single-node by construction. No forward-pointer
  issue.

  (The agent-light splice return is the design; a fully-agent-idle bidirectional
  return is a non-goal, not pursued — NO kernel patch is or will be required.)

## Wave: DESIGN / [REF] DESIGN Handoff

- **Hands off to**: `acceptance-designer` (DISTILL) — the proxy topology + the
  observable acceptance criteria (the TEST-tier wire-capture / `ss -tie` /
  fail-closed / no-cleartext observables, now grounded on the PROXY mechanism, NOT
  the in-band model); `platform-architect` (DEVOPS) — the Tier-3 instrumentation
  (`tcpdump`, `ss -tie`, the splice-pump liveness, the probe `health.startup.refused`
  path). **No external-integration contract tests** (both TLS sides are
  Overdrive-native east-west mTLS).
- **BLOCKING FIRST SLICE — composed Tier-3 walking skeleton (F2).** Before ANY
  other DELIVER slice: a composed Tier-3 acceptance test — real `cgroup_connect4`
  intercept → workload pre-arm write → leg-B handshake → kTLS arm → **post-arm
  bidirectional multi-record transfer with NO RST** — run under BOTH normal AND
  traced/delayed timing. This is an **integration/walking-skeleton gate, NOT a
  "prove-the-mechanism" gate**: the mechanism is spike-verified — the primitives
  in isolation AND the INBOUND flow composed end-to-end in one direction
  (`findings-inbound-intercept.md` increment-i §2, *ok* mode: real TPROXY
  intercept → orig-dst recovery → server-mTLS verifying C's client SVID →
  kTLS-RX arm → agent-light splice-to-S byte-exact, fail-closed on
  `nocert`/`wrongca`). Slice 00 closes the three NARROW remaining gaps: (1) the
  OUTBOUND path composed in ONE flow (increment-e proved outbound intercept +
  pre-arm capture + handshake-window flush, increment-f proved the steady-state
  egress splice — on SEPARATE harnesses; increment-e's steady-state RST was a
  *throwaway-harness intercept-lifecycle limitation, NOT a kernel finding*,
  superseded by increment-f's clean-harness proof); (2) bidirectional
  steady-state round-trip; (3) the real netns/veth topology with cgroup-isolated
  workloads. This gate composes the proven pieces into ONE bidirectional flow in
  that topology and **supersedes the old in-band walking skeleton**. (ADR-0069 §
  Consequences/Negative "Three narrow composition gaps remain" + § Enforcement.)
  Flagged for the slice re-grounding in `design/upstream-changes.md`.
- **Resource + identity ACs must appear in the slices (F1/F4/F5).** The slice
  re-grounding MUST surface: the F4 resource-limit fail-closed ACs
  (`BufferLimitExceeded` / `HandshakeTimeout` / `InFlightLimitExceeded` + cleanup
  no-leak); the F5 intercept-exemption ACs (leg B not re-intercepted; workload
  cannot self-exempt); and the F1 authn-vs-authz boundary (authn-only in v1;
  authorization is #27/#38; the wrong-but-valid-peer SAN-match negative test is a
  reserved placeholder gated on #178).
- **Authn-vs-authz boundary (F1).** This feature does authentication +
  encryption, NOT authorization. Allow/deny is the BPF-LSM `socket_connect` hook
  ([#27](https://github.com/overdrive-sh/overdrive/issues/27)) fed by compiled
  `policy_verdicts` ([#38](https://github.com/overdrive-sh/overdrive/issues/38);
  related [#49](https://github.com/overdrive-sh/overdrive/issues/49)).
  Expected-destination identity pinning is downstream of #26 via east-west
  resolution ([#178](https://github.com/overdrive-sh/overdrive/issues/178); VIP
  path [#61](https://github.com/overdrive-sh/overdrive/issues/61)). The proxy MUST
  NOT embed a policy engine. (No GH issues created here.)
- **No carried blockers.** In-band restart-survival/density is out of v1 scope
  (above) — a post-v1 optimization tracked in **#231**; multi-node is simply out of
  v1 scope (above) — no forward-pointer issue, nothing to create.
  OQ-1 (the `MtlsEnforcement` signature) is **ACCEPTED** (user-approved 2026-06-12 —
  the contract DELIVER implements to). OQ-2 (adapter home) is **resolved** (extend
  `overdrive-dataplane` + `overdrive-bpf`; no new crate).
- **Back-propagation**: COMPLETE. J-SEC-003 + slices 00–05 (and the persona lens,
  product/DISCUSS journeys, outcome registry, scope-boundary table) have been
  re-grounded on the proxy mechanism — including the composed walking-skeleton gate
  + the F1/F4/F5 ACs. See `design/upstream-changes.md` as the completed
  back-propagation record (past-tense rationale of record; not a live TODO). The
  architect did NOT edit `jobs.yaml` or the slice files — those are the
  product-owner's artifacts.

## Wave: DESIGN / [REF] MtlsEnforcement Port Contract (ACCEPTED)

> **STATUS: ACCEPTED (user-approved 2026-06-12 — bidirectional + F4–F7
> revised).** The contract is now **bidirectional** (F3):
> `InterceptedConnection` carries a `direction: Direction { Outbound, Inbound }`
> and `enforce` dispatches on it — the inbound/passive half (TPROXY intercept →
> orig-dst recovery → server-side mutual-TLS → kTLS-RX decrypt → splice-to-server)
> is now a first-class path, grounded in `findings-inbound-intercept.md`
> (increment-i, kernel 7.0). The earlier adversarial review's
> Findings 1/4/5 remain folded in (authn-vs-authz boundary + reserved
> `expected_peer`/`PeerIdentityMismatch` (F1); the `MtlsLimits` resource
> contract + `BufferLimitExceeded`/`HandshakeTimeout`/`InFlightLimitExceeded`
> (F4); the leg-B intercept-exemption postcondition (F5)), and the RE-review's
> F4–F7 are now revised in: the guest-stack adapter is STAGED to
> [#222](https://github.com/overdrive-sh/overdrive/issues/222) (F4 §); the
> authn-only boundary is scoped honestly everywhere (F5 §); the return/deliver
> pump supervision policy is pinned (F6 — `PumpLiveness` + `pump_stall_deadline`);
> and `MtlsLimits` carries CONCRETE default values (F7). The 4-method shape +
> SD-1…SD-4 + the OQ-2 home decision are UNCHANGED — the `direction` field, the
> F6 `pump_stall_deadline`, and the F7 concrete values are ADDITIVE.**
> This section pins the OQ-1 contract — now **ACCEPTED (user-approved
> 2026-06-12)** — the exact
> `MtlsEnforcement` trait signatures + SD-1…SD-4. OQ-2 (the host-adapter home) is
> already **resolved** — extend `overdrive-dataplane` (userspace) + `overdrive-bpf`
> (kernel); no new crate; `overdrive-host` ruled out (see § Open Questions and the
> § "OQ-2 resolution — `HostMtlsEnforcement` home" subsection below).
> It is the contract **DELIVER implements to** — the crafter MUST NOT invent public
> surface beyond what is accepted here (CLAUDE.md § "Implement to the design —
> never invent API surface"; the `workflow-result-error-model` precedent). The
> ADR-0069 *model* (four-phase lifecycle + the spike-pinned invariants) is fixed;
> this section pins the *signatures* within it. **Nothing here is implemented** —
> no `src/` exists yet. The accepted sub-decisions (SD-1 … SD-4) are recorded
> below with their rationale; they are the locked contract DELIVER builds to.

**Agent**: Morgan (nw-solution-architect) · **Mode**: pin a contract within a
user-locked decision · **Conventions matched**: `traits/driver.rs` (the
`Driver`/`AllocationHandle`/`take_exit_receiver` shape — the closest analogue:
a per-allocation lifecycle owned by the node agent, with an opaque handle and an
event-stream surface), `traits/dataplane.rs` (the four-clause behaviour-docstring
discipline + the `*Probe` Earned-Trust error variants), `traits/identity_read.rs`
(the consumed port + the clause-mapped docstring SSOT the equivalence harness
enforces).

### Granularity decision (the load-bearing shape choice)

ADR-0069's four phases are **intercept-arm → drive-handshake-and-arm-kTLS → run
steady-state splice → teardown**, and they apply in BOTH directions (outbound
client side; inbound server side — F3). The contract does NOT expose four methods
1:1, and it does NOT expose separate outbound/inbound method families. The
deciding question (per the dispatch) is **what the in-process worker
(`overdrive-worker`, D-MTLS-10) actually calls**, and **what is DST-observable
through the Sim adapter**. The answer, grounded in the spikes:

**Bidirectional shape decision (F3 — `direction` field, NOT a sibling method).**
The same four lifecycle phases govern both directions; the differences are
adapter-internal mechanism (outbound: `cgroup_connect4` intercept + rustls
*client* handshake + agent-light forward `read → write_all` copy into kTLS-TX;
inbound: TPROXY intercept + `getsockname` orig-dst + rustls *server* handshake with
`WebPkiClientVerifier` + zero-copy splice-to-server). The contract therefore carries a `direction: Direction
{ Outbound, Inbound }` discriminant on `InterceptedConnection`, and `enforce`
dispatches on it — rather than a sibling `enforce_inbound` method. Rationale:
(a) the *observable contract* is identical in both directions (bring an
intercepted connection to steady-state-established mTLS, or fail-closed; observe
pump liveness; tear down) — a sibling method would duplicate every postcondition
and double the surface the sim must mirror; (b) the leg ownership is symmetric
(outbound: leg F plaintext / leg B kTLS; inbound: leg S plaintext / leg C kTLS) —
**one owned intercepted leg + the routing fact covers both**, with `direction`
selecting which leg the worker hands over (outbound: the plaintext leg F; inbound:
the client-facing kTLS leg C — NOT a plaintext fd); (c) `EnforcedConnection` /
`teardown` / `liveness` are
direction-agnostic (a torn-down connection and a stalled pump look the same
either way). The genuine inbound-only inputs (the original-destination the TPROXY
listener recovered, which selects the *server* SVID) are carried as a
`direction`-tagged variant payload, NOT new methods — see SD-1 below.

- The **setup phases collapse into ONE async drive call** (`enforce`). Phases 1–2
  (lossless handshake-window capture → rustls handshake on leg B presenting the
  held SVID → drain 0.5-RTT early data → arm kTLS on leg B → flush captured
  plaintext → start the forward `read → write_all` copy pump) are a single atomic
  "bring this connection to steady-state-established" unit with one natural
  `Ok(handle)` / `Err(fail-closed)` outcome. Splitting them into per-phase public
  methods would (a) expose ordering the adapter makes internal (the kTLS-arm-then-pump sequence),
  not caller-sequenced; and (b) leak adapter mechanism (which leg, which syscall)
  into the port, violating "the port models WHAT, the adapter owns HOW." This
  mirrors `Driver::start` — one call spawns the workload and returns an opaque
  `AllocationHandle`; the caller never drives the sub-steps. **(REVISED
  2026-06-13, D-MTLS-13: the original text cited the `arming invariant`
  (SOCKMAP-before-`TCP_ULP`) as the ordering the type system hides — that
  invariant is now MOOT, the sockmap is gone; the adapter-internal ordering it
  hides is the kTLS-arm → forward-copy-pump sequence.)**

- The **forward steady state is NOT a method** — it is the **agent-light forward
  encrypt pump** (a bounded `read(legF) → write_all(legB)` COPY into leg B's
  kTLS-TX; the kernel `tls_sw_sendmsg` encrypts each blocking `write`; NOT a splice
  into TX, which loses records; D-MTLS-13). `enforce` *starts* the adapter's own
  forward-pump task; nothing in the port drives it per-record (`liveness`/`teardown`
  observe and stop it, the same as the return pump). **~~Originally agent-IDLE — the kernel `sk_skb/stream_verdict` sockmap
  egress-redirect (`flags=0`).~~ RETIRED 2026-06-13** (the redirect
  `MSG_DONTWAIT`-stalls ~10–15% of records;
  `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`). Pinning the forward
  pump as a "run forward" method would invent surface the mechanism does not need.

- The **return steady state IS agent-light** (the `splice(legB → pipe → legF)`
  pump, ~1 splice/record; `findings-splice-return.md`). The agent must *drive*
  this pump for the connection's life. **SD-2 below** is the genuine fork on how
  the contract represents it (the port owns the pump internally vs the worker
  drives it via a returned handle). The recommendation (SD-2) is **the port owns
  the pump**; `enforce` returns once steady-state is established and the adapter's
  own task drives the splice — so the port surface stays "establish / observe /
  tear down," not "pump one record."

- **`teardown` is one async call** keyed by the handle (release both legs, stop the
  pump, drop the kTLS state) — the `Driver::stop` analogue.

- **`probe` is separate** (Earned Trust; composition-root "wire→probe→use";
  D-MTLS-11) — the `Dataplane`'s `*Probe` round-trip analogue, sync-or-async per
  SD-3.

So the minimal surface is **four methods**: `probe`, `enforce`, an **observation
surface** for pump liveness (SD-4), and `teardown`. No "phase 1 / phase 2 / phase
3" methods — the four ADR phases map to `enforce` (phases 1–2 + forward install),
the adapter-internal pump (phase 3, agent-light), and `teardown` (phase 4).

#### Per-method spike anchor (each method's mechanism is PROVEN, not assumed)

Each method drives a spike-proven mechanism; DELIVER implements to the syscall /
flag / ordering the named committed finding pins (see § Proven-Mechanism
Traceability for the full matrix). The crafter references the finding for the
exact wire shape — it does not re-derive the mechanism.

| Method | Mechanism it drives | Proving committed finding |
|---|---|---|
| `enforce` OUTBOUND (handshake + kTLS arm) | rustls *client* `dangerous_extract_secrets` → `ktls` → `setsockopt TCP_ULP/TLS_TX/TLS_RX` on leg B | `findings.md` Increment A (WORKS) + Increment B (`ktls::KtlsStream` for control records) |
| `enforce` OUTBOUND (transparent intercept + lossless capture) | `cgroup/connect4`-rewrite → `accept()` leg F → lossless `recv()` drain → flush | `findings-userspace-relay.md` Unknowns 1+2 |
| `enforce` OUTBOUND (forward pump, agent-light COPY) | bounded `read(legF) → write_all(legB)` COPY into leg B's kTLS-TX; kernel `tls_sw_sendmsg` encrypts each blocking `write` (D-MTLS-13). NOT zero-copy, NOT a splice into TX (which loses records). ~~`sk_skb/stream_verdict` sockmap EGRESS-redirect `flags=0`~~ RETIRED — `MSG_DONTWAIT`-stall | `crates/overdrive-dataplane/src/mtls/splice.rs` (the shipped `write_all` forward primitive); `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` (forward pivot) |
| `enforce` (kTLS 0.5-RTT early-data drain, both directions) | drain `conn.reader()` of early application_data before `dangerous_extract_secrets` arms kTLS-RX (`rx` `rec_seq` already accounts for the over-read), forward it ahead of the splice pump (D-MTLS-13) | shipped `mtls::drain_early_plaintext` (CorkStream-equivalent userspace drain) |
| `enforce` INBOUND (transparent intercept + orig-dst recovery) | `nft` TPROXY → `IP_TRANSPARENT` listener `accept()` leg C → `getsockname()` recovers ORIG_DST → selects server `AllocationId` | `findings-inbound-intercept.md` §1 + Mechanics #1 |
| `enforce` INBOUND (server-side mutual-TLS + client-auth verify) | rustls *server* `ServerConfig` presents server SVID + `WebPkiClientVerifier` REQUIRE+VERIFY client SVID chains to bundle; fail-closed on `nocert`/`wrongca` | `findings-inbound-intercept.md` §2 + §4 |
| `enforce` INBOUND (kTLS-RX arm + splice-to-server, agent-light) | arm kTLS-RX on leg C (suppress `NewSessionTicket`; read `peer_certificates` before `extract_secrets`) → `splice(legC→pipe→legS)` via `tls_sw_splice_read` | `findings-inbound-intercept.md` §3 + §5 + Mechanics #3/#6 |
| ~~`enforce` / `probe` (arming invariant)~~ MOOT 2026-06-13 (D-MTLS-7/13) | ~~SOCKMAP insert BEFORE `TCP_ULP "tls"` on leg B~~ — no sockmap insert on any path now | `findings.md` Increment D (historical kernel fact only) |
| `liveness` + the return/deliver pump | `splice(legB→pipe→legF)` (outbound return) / `splice(legC→pipe→legS)` (inbound deliver) via `tls_sw_splice_read` on a plain no-psock kTLS-RX leg (~1/record) | `findings-splice-return.md` (CONFIRMED) · `findings-inbound-intercept.md` §5 |
| (leg B / leg C no-psock precondition) | the kTLS-RX leg carries no sockmap/verdict on RX — else kTLS-RX fights it / `tls_sw_read_sock -EINVAL` | `findings-ktls-rx-splice.md` (verdict (b)) |
| `probe` (1 composed sentinel, D-MTLS-13) | kTLS arm + agent-light forward-encrypt round-trip on a loopback sentinel (`ProbeSentinel::KtlsArmRoundTrip`) — the forward `read → write_all` copy into kTLS-TX (the EXACT production primitive, NOT a splice into TX) moves one record, the peer's kTLS-RX reconstructs it byte-exact; reader leg also drains 0.5-RTT early data. ~~(2) forward egress-redirect emits ciphertext; (3) reverse arming order = `EINVAL`~~ dropped with the sockmap | `findings.md` A · `crates/overdrive-dataplane/src/mtls/outbound.rs` (the shipped probe; 1:1 with the single `ProbeSentinel` variant) |

### Newtypes (CREATE-NEW, minimal)

Three new domain values (`InterceptedConnection`, `EnforcedConnection`, and the
`MtlsLimits` resource contract added for F4). All are `overdrive-core` types per
`.claude/rules/development.md` § "Newtypes — STRICT by default" (no raw primitive
for a domain concept) — but kept to the **minimum the model + spikes + the
review's resource/identity findings require**.

1. **`InterceptedConnection`** — the descriptor the worker passes IN: the
   transparently-intercepted connection the proxy must enforce, in EITHER
   direction (F3). It is the *input identity* of one connection, carrying exactly
   what the adapter needs to own the intercepted leg + drive it to steady-state
   mTLS, and no more. **SD-1 below** is the genuine fork on its payload (owned
   intercepted-leg fd vs `pidfd` handle vs 4-tuple). The recommended shape (SD-1)
   is an **owned `OwnedFd` for the agent's accepted intercepted leg + a
   `direction`-tagged routing fact** (outbound: the plaintext leg F; inbound: the
   client-facing kTLS leg C):
   - **Outbound** (`Direction::Outbound`): the owned **leg F** (the workload-facing
     plaintext leg the agent `accept()`ed off the `cgroup_connect4`-rewrite
     intercept, `findings-userspace-relay.md` Unknown 1) + the **peer
     `SocketAddrV4`** leg B must dial.
   - **Inbound** (`Direction::Inbound`): the owned **leg C** (the client-facing
     leg the agent `accept()`ed off the TPROXY/`IP_TRANSPARENT` listener,
     `findings-inbound-intercept.md` §1) + the **recovered original destination**
     (`getsockname` on leg C) which selects the *server* `AllocationId` whose SVID
     to present. Leg S (the agent-owned plaintext leg to the server workload) is
     opened by the adapter *inside* `enforce` (a same-node dial to the server
     workload's real plaintext socket), so it is NOT a constructor input — the
     worker hands over leg C, the adapter produces leg S.

   The `direction` discriminant selects which mechanism `enforce` runs and which
   leg the owned fd is. (Contrast the SUPERSEDED in-band model, which would have
   passed a `pidfd_getfd` dup of the *workload's own* socket — the proxy owns its
   own legs, so no `pidfd` is in the v1 contract.)

   ```rust
   /// Which half of the proxy this intercepted connection is (F3 — bidirectional).
   /// Outbound = the workload is the CLIENT (its connect() was cgroup_connect4-
   /// rewritten to the agent); Inbound = the workload is the SERVER (a connection
   /// to its logical address was TPROXY-intercepted to the agent). `enforce`
   /// dispatches on this; the observable contract is identical either way.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum Direction {
       /// The intercepted workload is the connection's CLIENT (outbound connect()).
       /// `cgroup_connect4` intercept → rustls CLIENT handshake on leg B →
       /// agent-light forward pump (`legF → legB`, a `read → write_all` copy into
       /// leg B's kTLS-TX) + return splice (`splice(legB → legF)` out of kTLS-RX).
       Outbound,
       /// The intercepted workload is the connection's SERVER (inbound accept()).
       /// TPROXY intercept → `getsockname` orig-dst → rustls SERVER handshake on
       /// leg C (present server SVID, verify client SVID) → splice-to-server.
       Inbound,
   }

   /// One transparently-intercepted workload connection to enforce, in either
   /// direction (F3).
   ///
   /// OUTBOUND: produced after the `cgroup_connect4`-rewrite intercept lands the
   /// workload on the agent's leg-F listener and the agent `accept()`s it
   /// (`findings-userspace-relay.md` Unknown 1). The owned `leg` is leg F (the
   /// workload-facing plaintext leg — the one outbound case where the intercepted
   /// leg is plaintext); `routed` is `Outbound { peer }` (the real peer leg B
   /// dials).
   ///
   /// INBOUND: produced after the `nft` TPROXY + `IP_TRANSPARENT` intercept lands
   /// a connection aimed at the server workload's logical address on the agent's
   /// leg-C listener and the agent `accept()`s it (`findings-inbound-intercept.md`
   /// §1). The owned `leg` is leg C (the CLIENT-facing TLS/kTLS leg — NOT
   /// plaintext: for inbound the agent-owned kTLS leg IS the accepted intercepted
   /// leg, and the plaintext leg S to the server workload is opened by the adapter
   /// inside `enforce`). `routed` is `Inbound { orig_dst }` (the
   /// `getsockname`-recovered original destination that selects the server SVID).
   ///
   /// Workload-holds-nothing (D-MTLS-9): this descriptor never carries SVID
   /// material — only the plaintext/kTLS leg fd and the routing facts. The proxy
   /// reads the SVID through `IdentityRead` inside `enforce`. Both the client AND
   /// the server workload hold NOTHING.
   #[derive(Debug)]
   pub struct InterceptedConnection {
       /// The agent-owned leg the worker `accept()`ed for this intercepted
       /// connection, handed over by value (the port takes ownership, RAII close
       /// on teardown). OUTBOUND: leg F (workload-facing plaintext). INBOUND:
       /// leg C (client-facing — the kTLS leg the agent terminates TLS on).
       /// Owned, not borrowed: the port's lifecycle outlives the worker's call
       /// frame (the pump runs after `enforce` returns).
       pub leg: std::os::fd::OwnedFd,
       /// The direction discriminant + its direction-specific routing fact.
       pub routed: Routed,
       /// Whose held SVID to present. OUTBOUND: the CLIENT workload's SVID (the
       /// `IdentityRead::svid_for` key). INBOUND: the SERVER workload's SVID,
       /// selected by `Routed::Inbound { orig_dst }` → `AllocationId`. Either way
       /// `svid_for(alloc) == None` is the fail-closed signal (`enforce` returns
       /// `AbsentSvid`).
       pub alloc: AllocationId,
       /// OPTIONAL expected-destination SPIFFE identity (F1 / authn-vs-authz
       /// boundary). When `Some`, `enforce` SAN-matches the authenticated peer
       /// against it and returns `PeerIdentityMismatch` on a wrong-but-valid
       /// peer (one that chains to the trust bundle but is NOT the intended
       /// destination). **v1 leaves this `None` (authn-only)** in BOTH directions:
       /// #26 enforces chain-to-trust-bundle authentication only, and the
       /// expected-peer identity is supplied DOWNSTREAM by east-west SPIFFE-ID
       /// resolution ([#178](https://github.com/overdrive-sh/overdrive/issues/178),
       /// which "terminates in SPIFFE mTLS via sockops (#26)"). The field + the
       /// `PeerIdentityMismatch` variant are reserved now so the SAN-match wires
       /// the moment #178 supplies it — no contract change later. This is NOT
       /// authorization (allow/deny is #27's BPF-LSM `socket_connect` hook fed by
       /// #38's `policy_verdicts`); it is *identity pinning* of an
       /// already-authenticated peer. **v1 = chain-to-bundle transport authn +
       /// encryption, NO intended-peer pinning** (F5).
       pub expected_peer: Option<SpiffeId>,
   }

   /// The direction-specific routing fact carried alongside the owned leg (F3).
   #[derive(Debug, Clone, Copy)]
   pub enum Routed {
       /// OUTBOUND: the real peer `SocketAddrV4` leg B must dial to originate the
       /// outbound mTLS session. V4 per the single-node Phase-1 scope (multi-node
       /// transparent mTLS is OUT of v1 scope — Phase 1 is single-node);
       /// `SocketAddrV4` matches the `Dataplane` local-backend call shape.
       Outbound { peer: std::net::SocketAddrV4 },
       /// INBOUND: the original destination the TPROXY listener recovered via
       /// `getsockname()` on the accepted leg C (`findings-inbound-intercept.md`
       /// §1 / Mechanics #1 — under TPROXY the orig-dst IS the accepted socket's
       /// local addr, NOT `SO_ORIGINAL_DST`). This is what selects the SERVER
       /// workload's `AllocationId` → its held SVID. The adapter dials the
       /// server workload's real plaintext socket (leg S) inside `enforce`.
       Inbound { orig_dst: std::net::SocketAddrV4 },
   }
   ```

   > Note: `OwnedFd` is a primitive-ish std type, not a domain-bearing raw
   > primitive — wrapping it in a further newtype buys nothing (it is already a
   > typed, RAII-closing handle). The *domain* newtype is `InterceptedConnection`
   > itself (the descriptor), which the newtype rule is satisfied by. `alloc` uses
   > the existing `AllocationId` newtype; `peer`/`orig_dst` use `SocketAddrV4`
   > exactly as `Dataplane::register_local_backend` does; `expected_peer` uses the
   > existing `SpiffeId` newtype (`Option`, `None` in v1 — see the field doc /
   > F1/F5). `Direction`/`Routed` are minimal additive enums, not raw
   > discriminants (the make-invalid-states-unrepresentable lever: an inbound
   > connection cannot carry a `peer` dial target, an outbound cannot carry an
   > `orig_dst`).

2. **`EnforcedConnection`** — the **opaque handle** the port returns from `enforce`
   (the `AllocationHandle` analogue). The worker holds it to (a) tear the
   connection down and (b) observe pump liveness. Its contents are the port's
   private tracking state — the worker does NOT inspect them (mirrors
   `AllocationHandle`'s "the node agent does not inspect its contents" doc). A
   stable, `Clone`-able correlation id is the only field the worker reads, for
   logging / liveness correlation.

   ```rust
   /// Opaque handle to a connection the proxy has brought to
   /// steady-state-established. Returned by `MtlsEnforcement::enforce`; consumed
   /// by `teardown`; correlated by `liveness`. The worker does NOT inspect the
   /// adapter-private tracking state — only `id()` (a stable correlation key) is
   /// caller-readable, mirroring `Driver`'s opaque `AllocationHandle`.
   #[derive(Debug, Clone)]
   pub struct EnforcedConnection {
       /// Stable per-connection correlation id (the alloc + a monotonic
       /// per-connection counter), for log/liveness correlation only. NOT a
       /// security identity — the SVID identity is `alloc`'s, presented on leg B.
       id: EnforcedConnectionId,
       // adapter-private: leg-F/leg-B fds, the pump task handle, the sockmap
       // membership keys. Not exposed; the host adapter owns them.
   }
   ```

   `EnforcedConnectionId` is a thin newtype (`SpiffeId`-style: `Display` /
   `FromStr` / `Serialize` round-trip) so DST and telemetry can name a connection
   stably. It is derived `(AllocationId, u64)` — content-addressed within a node
   session, no entropy.

3. **`MtlsLimits`** — the **resource contract** for the lossless pre-arm buffer
   (F4 / ADR-0069 § "Resource & robustness constraints") plus the F6 pump-stall
   deadline. The pre-arm plaintext buffer is load-bearing but must be bounded: a
   workload can stream into leg F (outbound) / a peer can stream into leg C
   (inbound) while the handshake stalls, so an unbounded buffer is a DoS surface.
   These limits are **adapter construction parameters** (they apply across all
   connections, not per-connection), passed to the host/sim adapter's constructor
   alongside `Arc<dyn IdentityRead>`. They are NOT operator-tunable in v1 —
   **CONCRETE compile-time defaults pinned below (F7)**; operator-tunability of
   `MtlsLimits` is tracked in
   [#230](https://github.com/overdrive-sh/overdrive/issues/230) (created
   2026-06-12 — see the handoff note).

   ```rust
   /// Resource bounds for the lossless pre-arm capture (F4) + the F6 pump-stall
   /// deadline. Construction-time, not per-connection: the adapter holds one
   /// `MtlsLimits` and applies it to every `enforce`. Fail-closed on every limit
   /// — never queue-unbounded, never degrade to cleartext. The F7 defaults are
   /// CONCRETE (not "sensible defaults"): the acceptance tests assert these exact
   /// values, not merely field existence.
   #[derive(Debug, Clone, Copy)]
   pub struct MtlsLimits {
       /// Max pre-arm plaintext bytes buffered per connection before kTLS arms.
       /// Exceeding it ⇒ `BufferLimitExceeded`: drop the buffer, reset the
       /// plaintext leg (leg F outbound / leg S not-yet-opened inbound), no
       /// cleartext egresses. **F7 default: 256 KiB (262_144).** Rationale: covers
       /// a request-first protocol's first flight (HTTP/2 headers, a gRPC request,
       /// a Postgres startup) while the handshake completes in single-digit ms,
       /// two orders of magnitude below what a stalled peer could otherwise pin.
       pub max_prearm_bytes: usize,
       /// Deadline for the handshake-and-arm (leg B outbound / leg C inbound).
       /// Exceeding it ⇒ `HandshakeTimeout`: the stalled peer cannot pin agent
       /// resources. **F7 default: 5 s.** Rationale: a same-node / east-west mTLS
       /// handshake completes in ms; 5 s distinguishes a dead/stalled peer from
       /// normal GC/scheduler variance without false-tripping.
       pub handshake_deadline: Duration,
       /// Max concurrent in-flight (pre-arm, not-yet-armed) connections per
       /// `AllocationId`. Over-limit ⇒ the new intercept is refused fail-closed
       /// (`InFlightLimitExceeded`), so one workload cannot exhaust the agent by
       /// opening many stalled connections. **F7 default: 128.** Rationale: a
       /// healthy workload arms each connection in ms, so 128 concurrent *pre-arm*
       /// connections is far above any legitimate burst yet caps the
       /// amplification one workload can inflict.
       pub max_inflight_per_alloc: u32,
       /// F6 — the no-progress window after which a return/deliver splice pump is
       /// `PumpLiveness::Stalled`: the bytes-spliced counter has not advanced for
       /// this long WHILE a record is pending on the kTLS-RX leg. The worker tears
       /// the connection down on `Stalled` (teardown + fail-closed reset). A purely
       /// idle connection (no pending record) is `Running`, never `Stalled`.
       /// **F7 default: 30 s.** Rationale: generous enough that no healthy bursty
       /// connection trips it, tight enough that a stranded pump is reclaimed
       /// promptly.
       pub pump_stall_deadline: Duration,
   }

   impl Default for MtlsLimits {
       /// The F7 v1 defaults — pinned, not operator-tunable in v1. The acceptance
       /// tests assert these exact values.
       fn default() -> Self {
           Self {
               max_prearm_bytes: 256 * 1024,            // 256 KiB
               handshake_deadline: Duration::from_secs(5),
               max_inflight_per_alloc: 128,
               pump_stall_deadline: Duration::from_secs(30),
           }
       }
   }
   ```

   **Resource budget (F7 — the operator's exhaustion-reasoning surface; mirrors
   ADR-0069 § "Resource & robustness constraints"):** per pre-arm connection ≤ 256
   KiB buffer + ~3 fds (two legs + one `splice` pipe); per allocation ≤ 128 ×
   (256 KiB + 3 fds) = **≤ 32 MiB + ≤ 384 fds** in the pre-arm window
   (steady-state established connections drop the buffer, holding only ~3 fds
   each); per node = Σ over allocations, sized against `RLIMIT_NOFILE`. The
   in-flight ceiling makes the pre-arm contribution bounded and predictable.

### The trait

`#[async_trait]` at the boundary (mirroring `Dataplane` / `Driver` — async only
where the contract genuinely awaits kernel I/O; the trait lives in `overdrive-core`
but `async_trait` is a declarative macro with no runtime, so it stays off the
`core` *I/O* surface exactly as `Dataplane` does today). `Send + Sync + 'static`
to be held as `Arc<dyn MtlsEnforcement>` and shared across the worker's per-
connection tasks.

```rust
//! [`MtlsEnforcement`] — the per-connection transparent-mTLS enforcement port
//! (ADR-0069). The agent-light L4 proxy's driven contract, **bidirectional (F3)**:
//! bring a transparently-intercepted workload connection — OUTBOUND (the workload
//! is the client) OR INBOUND (the workload is the server) — to a
//! steady-state-established mTLS session, observe the agent-light pump's
//! liveness, and tear the connection down. `enforce` dispatches on
//! `InterceptedConnection::routed` (the `Direction`):
//! - **OUTBOUND**: lossless capture on leg F → rustls CLIENT handshake on leg B
//!   presenting the held SVID → arm kTLS-TX/RX → agent-light FORWARD pump
//!   (`legF → legB`, a `read → write_all` COPY into leg B's kTLS-TX; the kernel
//!   `tls_sw_sendmsg` encrypts each `write`) + RETURN pump (`splice(legB → legF)`,
//!   a zero-copy `splice` out of leg B's kTLS-RX) (D-MTLS-13).
//! - **INBOUND**: TPROXY-intercept → `getsockname` orig-dst → rustls SERVER
//!   handshake on leg C (present the server SVID, REQUIRE+VERIFY the client SVID
//!   chains to the bundle via `WebPkiClientVerifier`) → arm kTLS-RX → dial the
//!   server workload (leg S) → DELIVER pump (`splice(legC → legS)`, a zero-copy
//!   `splice` out of leg C's kTLS-RX) + RESPONSE pump (`legS → legC`, a
//!   `read → write_all` COPY into leg C's kTLS-TX); fail-closed on
//!   `nocert`/`wrongca` (`findings-inbound-intercept.md`).
//!
//! **Agent-light is per-direction and NOT symmetric.** "Agent-light" = the agent
//! does NO TLS crypto in userspace (the kernel kTLS engine does). The DECRYPT/RX
//! directions (outbound return, inbound deliver) are zero-copy `splice` out of a
//! plain kTLS-RX leg (`tls_sw_splice_read`); ~1 splice/record, no userspace copy —
//! agent-idle / zero-per-byte-syscall holds. The ENCRYPT/TX directions (outbound
//! forward, inbound response) are a bounded `read → write_all` COPY into a kTLS-TX
//! leg; per-record `read`+`write`, plaintext copied through a userspace buffer —
//! NOT zero-copy, NOT agent-idle. A `splice` INTO kTLS-TX loses records (the same
//! `MSG_DONTWAIT` loss class the abandoned sockmap egress redirect suffered), so
//! the encrypt directions use a synchronous blocking `write_all`. SSOT:
//! `crates/overdrive-dataplane/src/mtls/splice.rs`.
//!
//! Production wires `HostMtlsEnforcement` (over kTLS / `splice` /
//! `cgroup_connect4` / `nft`-TPROXY+`IP_TRANSPARENT`, consuming `IdentityRead`;
//! the sockops/sk_skb/sockmap surface was retired with the forward sockmap-egress
//! redirect, D-MTLS-13); simulation wires `SimMtlsEnforcement` (in-memory
//! observable-contract mirror). The `mtls_enforcement_equivalence` DST harness
//! drives both through the same call sequence (both directions) and asserts
//! identical observable state (`.claude/rules/development.md` § "The DST
//! equivalence test is the structural guard").
//!
//! Consumes `IdentityRead` (#35) as a REQUIRED constructor parameter — #26 is a
//! READER, never an issuer (D-MTLS-9). BOTH the client AND the server workload
//! hold NOTHING.
//!
//! **Scope (F1/F5 — authn + encryption, NOT authz; NO intended-peer pinning in
//! v1).** This port AUTHENTICATES the peer (**chain-to-trust-bundle** only — that
//! the peer is *some* valid cluster workload) and ENCRYPTS the wire (kTLS), in
//! BOTH directions. It does NOT AUTHORIZE the connection — allow/deny is the
//! BPF-LSM `socket_connect` hook
//! ([#27](https://github.com/overdrive-sh/overdrive/issues/27)) fed by compiled
//! `policy_verdicts` ([#38](https://github.com/overdrive-sh/overdrive/issues/38);
//! related [#49](https://github.com/overdrive-sh/overdrive/issues/49)), a SEPARATE
//! subsystem this port MUST NOT duplicate (no policy engine, no Regorus, no
//! `policy_verdicts` read here). It also does NOT pin the *intended* peer:
//! expected-destination identity pinning (`expected_peer` + `PeerIdentityMismatch`)
//! is the [#178](https://github.com/overdrive-sh/overdrive/issues/178) UPGRADE
//! (east-west SPIFFE-ID resolution supplies the expected peer); **v1 is authn-only**
//! (`expected_peer == None`). **A routing bug / VIP collision / malicious
//! in-cluster endpoint presenting a valid-but-unintended SVID is NOT prevented in
//! v1** — the honest v1 claim is "chain-to-bundle transport authn + encryption, no
//! intended-peer pinning." Docs/tests MUST NOT call the wrong-but-valid-peer case
//! "protected" until #178 lands (F5).
//!
//! Resource-bounded by `MtlsLimits` (F4/F7): bounded pre-arm buffer (256 KiB),
//! handshake deadline (5 s), per-allocation in-flight ceiling (128), pump-stall
//! deadline (30 s, F6) — all fail-closed, never queue-unbounded; CONCRETE v1
//! defaults the acceptance tests assert. Construction takes `MtlsLimits`
//! alongside `IdentityRead`.

#[async_trait]
pub trait MtlsEnforcement: Send + Sync + 'static {
    /// Earned-Trust probe (ADR-0069 § Enforcement; D-MTLS-11). Verify the proxy
    /// substrate honours its contract in the REAL environment BEFORE any
    /// connection is enforced. Composition-root invariant: wire → probe → use.
    ///
    /// # Preconditions
    /// None. Called once at node startup, after the adapter is constructed,
    /// before `enforce` is ever called.
    ///
    /// # Postconditions on `Ok(())`
    /// The substrate the proxy relies on has been exercised on a loopback
    /// sentinel and round-tripped clean: a sentinel rustls TLS 1.3 handshake on a
    /// loopback leg arms kTLS-TX/RX, the **agent-light forward encrypt pump**
    /// (`read → write_all` into the kTLS-TX leg — the EXACT production forward
    /// primitive, NOT a splice into TX) moves one sentinel record through it
    /// ENCRYPTED, and the sentinel peer's kTLS-RX reconstructs the exact sentinel
    /// plaintext via a single `tls_sw_splice_read` (`findings.md` A; the shipped
    /// `mtls/` code is the SSOT).
    /// The sentinel handshake needs a cert+key, minted as a throwaway self-signed
    /// `rcgen` sentinel at call time (substrate-self-test crypto only —
    /// D-MTLS-12/SD-5; #26 stays a reader, never an issuer). The reader leg ALSO
    /// drains 0.5-RTT early application_data from `conn.reader()` before
    /// `dangerous_extract_secrets` (D-MTLS-13 / `mtls::drain_early_plaintext`).
    /// After `Ok`, the proxy is declared usable; the node proceeds to serve.
    ///
    /// **(REVISED 2026-06-13, D-MTLS-13.)** This is now ONE composed sentinel
    /// (`ProbeSentinel::KtlsArmRoundTrip`). The obsolete `ForwardEgressRedirect`
    /// (the sockmap-egress-redirect emits ciphertext) and `ArmingOrderEinval`
    /// (SOCKMAP-after-`TCP_ULP` returns `EINVAL`) sub-sentinels were DROPPED with
    /// the sockmap — the forward path is now an agent-light `read → write_all` copy
    /// into kTLS-TX, so there is no redirect and no sockmap-insert ordering to probe.
    ///
    /// # Edge cases
    /// Any sentinel round-trip failure (kTLS arm refused, the forward encrypt pump
    /// produces cleartext or no bytes, the peer reconstructs the wrong plaintext)
    /// returns a typed `MtlsEnforcementError` and the node MUST refuse to start
    /// with a structured `health.startup.refused` event — it does NOT degrade to
    /// a cleartext path (fail-closed for confidentiality).
    ///
    /// # Observable invariants
    /// `probe` mutates no enforced connection (there are none yet) and leaks no
    /// sentinel state — the loopback legs are torn down before return regardless
    /// of outcome.
    async fn probe(&self) -> Result<()>;

    /// Bring `conn` to a steady-state-established mTLS session and return an
    /// opaque [`EnforcedConnection`] handle. Phases 1–2 of ADR-0069 + the
    /// steady-state install, as ONE atomic unit. **Dispatches on
    /// `conn.routed` (the `Direction`)** — outbound (workload = client) vs
    /// inbound (workload = server, F3).
    ///
    /// # Preconditions
    /// - `conn.leg` is an OWNED, ESTABLISHED socket the agent `accept()`ed for a
    ///   transparently-intercepted connection. OUTBOUND: leg F, the workload-facing
    ///   plaintext leg off the `cgroup_connect4`-rewrite intercept
    ///   (`findings-userspace-relay.md` Unknown 1). INBOUND: leg C, the
    ///   client-facing leg off the TPROXY/`IP_TRANSPARENT` intercept
    ///   (`findings-inbound-intercept.md` §1). The port takes ownership.
    /// - `conn.routed` matches the direction: `Outbound { peer }` carries the real
    ///   peer leg B dials; `Inbound { orig_dst }` carries the `getsockname`-
    ///   recovered original destination that selects the server SVID.
    /// - `conn.alloc` MAY be absent from the held set — see edge cases
    ///   (fail-closed). OUTBOUND: the client workload's alloc. INBOUND: the server
    ///   workload's alloc (selected by `orig_dst`). The caller does NOT pre-check
    ///   `svid_for`; `enforce` is the single fail-closed gate.
    ///
    /// # Postconditions on `Ok(EnforcedConnection)` — OUTBOUND
    /// After return, ALL of the following hold (the observable contract every
    /// adapter MUST satisfy — what the `mtls_enforcement_equivalence` harness and
    /// the Tier-3 wire tests check):
    /// - The pre-arm plaintext the workload wrote during the handshake window was
    ///   captured LOSSLESSLY and flushed to the peer as the first
    ///   `application_data` on leg B (no dropped pre-arm bytes; rec_seq starts at
    ///   0; `findings-userspace-relay.md` Unknown 2).
    /// - Leg B carries TLS 1.3 records (`0x17`) presenting `conn.alloc`'s held
    ///   SVID (read via `IdentityRead::svid_for(&conn.alloc)`); the peer was
    ///   **authenticated** against `IdentityRead::current_bundle()` (chains to
    ///   the trust bundle). Auth-session == data-session (the rustls handshake's
    ///   extracted secrets ARE the kTLS keys on leg B). NO cleartext appears on
    ///   the peer-facing wire (`tcpdump` oracle).
    /// - The forward steady state is AGENT-LIGHT but a COPY (D-MTLS-13): the
    ///   adapter's own task drives a bounded `read(legF) → write_all(legB)` pump;
    ///   leg B is kTLS-TX-armed, so the kernel `tls_sw_sendmsg` encrypts each
    ///   blocking `write` synchronously (NOT the `MSG_DONTWAIT` sockmap-backlog
    ///   path). The agent does ZERO crypto, but it DOES copy each record's
    ///   plaintext through a userspace buffer and issues a `read`+`write` per
    ///   record — NOT zero-copy, NOT agent-idle, NOT symmetric to the inbound
    ///   deliver pump (which `splice`s zero-copy out of kTLS-RX). A `splice` INTO
    ///   kTLS-TX is NOT used (it loses records — the same `MSG_DONTWAIT` class the
    ///   sockmap egress redirect suffered;
    ///   `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`). The forward
    ///   COPY pump is the OUTBOUND liveness-observed primary.
    /// - The return-splice pump is RUNNING (the adapter's own task drives a
    ///   zero-copy `splice(legB → pipe → legF)` on a plain — NO psock — kTLS-RX
    ///   leg B; D-MTLS-4 / D-MTLS-5). `liveness(&handle)` reports `Running`.
    /// - (The 0.5-RTT early application_data the peer sent before the agent's
    ///   first record was drained from `conn.reader()` and forwarded ahead of the
    ///   pump — D-MTLS-13 / `mtls::drain_early_plaintext`.)
    /// - **Leg B was dialed with the intercept-exemption bypass (F5).** The
    ///   agent's own outbound leg-B `connect()` is NOT re-intercepted by the
    ///   workload `cgroup_connect4` rewrite — via a narrowly-scoped `SO_MARK`
    ///   socket mark the program checks-and-skips, OR cgroup scoping (the program
    ///   attaches to the *workload* subtree, not the agent's — the existing
    ///   `cgroup_connect4_service` attach boundary). The bypass is agent-private:
    ///   a workload CANNOT replicate it to self-exempt from interception (proven
    ///   by the F5 Tier-3 obligations: leg B not re-intercepted AND workload
    ///   cannot self-exempt). Without this, the agent's dial would recurse
    ///   infinitely.
    ///
    /// # Postconditions on `Ok(EnforcedConnection)` — INBOUND (F3)
    /// After return, ALL of the following hold (grounded in
    /// `findings-inbound-intercept.md`; what the inbound Tier-3 tests check):
    /// - The original destination was recovered via `getsockname()` on leg C and
    ///   selected the server workload's `AllocationId` → its held SVID (§1).
    /// - Leg C carries TLS 1.3 records (`0x17`); the agent's rustls SERVER
    ///   handshake presented `conn.alloc`'s held server SVID (via
    ///   `IdentityRead::svid_for`) AND the client's presented SVID was
    ///   **REQUIRED + VERIFIED** to chain to `IdentityRead::current_bundle()` via
    ///   `WebPkiClientVerifier` (§2). Auth-session == data-session (the rustls
    ///   secrets ARE the kTLS-RX keys on leg C). NO cleartext of the request
    ///   appears on the client-facing wire (it carries `0x17` app_data; §3).
    /// - The server workload received the **byte-exact decrypted plaintext** on
    ///   leg S (the agent dialed the server workload's real plaintext socket and
    ///   spliced); the server workload holds NOTHING and is identity-unaware (§3).
    /// - The deliver pump (the request-carrying C→S direction, the INBOUND
    ///   primary) is RUNNING and is a zero-copy SPLICE: the adapter's own task
    ///   drives `splice(legC → pipe → legS)` on a plain — NO psock — kTLS-RX leg C
    ///   (`tls_sw_splice_read` decrypts; same primitive as the outbound return).
    ///   `liveness(&handle)` observes THIS pump and reports `Running`.
    /// - The response pump (S→C) is RUNNING and is a bounded
    ///   `read(legS) → write_all(legC)` COPY into leg C's kTLS-TX (the same
    ///   userspace-copy encrypt primitive as the outbound forward; NOT a splice,
    ///   NOT zero-copy). It is auxiliary (not `liveness`-observed).
    /// - Server-config mechanics honoured: `NewSessionTicket` suppressed
    ///   (`send_tls13_tickets = 0`) and `peer_certificates()` read for the
    ///   fail-closed guard BEFORE `dangerous_extract_secrets` consumed the
    ///   connection (§ Mechanics #3/#6).
    ///
    /// # Postconditions — BOTH directions
    /// - **Authn, NOT authz; NO intended-peer pinning in v1 (F1/F5).** This
    ///   establishes the peer is *a valid cluster workload* (chains to the bundle),
    ///   NOT that the connection is *authorized* (allow/deny is #27's BPF-LSM
    ///   `socket_connect` hook fed by #38's `policy_verdicts`, a SEPARATE subsystem
    ///   the proxy MUST NOT duplicate) and NOT that the peer is the *intended*
    ///   destination. If `conn.expected_peer == Some(id)`, the authenticated peer's
    ///   SPIFFE-SAN is additionally matched against `id` (expected-destination
    ///   pinning); a mismatch is fail-closed (`PeerIdentityMismatch`). In **v1
    ///   `expected_peer` is `None`** (authn-only) — the expected-peer identity is
    ///   the #178 UPGRADE; this clause is a no-op until then. A
    ///   valid-but-unintended SVID is NOT rejected in v1.
    ///
    /// # Edge cases (all FAIL-CLOSED — no cleartext, connection refused) — both directions
    /// - `IdentityRead::svid_for(&conn.alloc) == None` ⇒ `Err(AbsentSvid)`; the
    ///   handshake is refused, `conn.leg` is closed, no bytes egress (OUTBOUND: no
    ///   client SVID; INBOUND: no server SVID for the selected `orig_dst`). (`None`
    ///   is the held-set fail-closed signal — `identity_read.rs` clause 3.)
    /// - `current_bundle() == None`, or the peer does not chain to it ⇒
    ///   `Err(PeerVerificationFailed)` / `Err(AbsentBundle)`; refused, leg closed.
    ///   INBOUND: this is the `nocert`/`wrongca` fail-closed path proven in
    ///   `findings-inbound-intercept.md` §4 — the client SVID is absent or does not
    ///   chain to the bundle; NO plaintext is spliced to the server workload.
    /// - `conn.expected_peer == Some(id)` and the authenticated peer's SPIFFE-SAN
    ///   does NOT match `id` (a wrong-but-valid peer — chains to the bundle but
    ///   is not the intended destination) ⇒ `Err(PeerIdentityMismatch)`; refused,
    ///   leg closed. **v1: unreachable while `expected_peer` is `None`** — the #178
    ///   UPGRADE (F1/F5). A valid-but-unintended SVID is NOT rejected in v1.
    /// - The peer/workload streamed more than `limits.max_prearm_bytes` of pre-arm
    ///   plaintext before kTLS armed ⇒ `Err(BufferLimitExceeded)`: the buffer is
    ///   dropped, the plaintext leg reset, no cleartext egresses (F4 / DoS guard).
    /// - The handshake-and-arm exceeded `limits.handshake_deadline` ⇒
    ///   `Err(HandshakeTimeout)`; refused, legs closed (F4 — leg B outbound / leg C
    ///   inbound).
    /// - The per-allocation in-flight ceiling `limits.max_inflight_per_alloc` is
    ///   already reached for `conn.alloc` ⇒ `Err(InFlightLimitExceeded)`: the new
    ///   intercept is refused, no cleartext (F4).
    /// - The rustls handshake aborts (wrong SVID, alert, timeout) ⇒
    ///   `Err(HandshakeFailed)`; refused, legs closed.
    /// - The kTLS arm refuses on the kTLS leg (`TCP_ULP`/`TLS_TX`/`TLS_RX`
    ///   rejected, kernel TLS module absent) ⇒ `Err(KtlsArmFailed)`; refused,
    ///   legs closed.
    /// - (REMOVED 2026-06-13, D-MTLS-13: the `ArmingOrderViolation`
    ///   SOCKMAP-after-`TCP_ULP` edge case is gone — the forward path is an
    ///   agent-light `read → write_all` copy into kTLS-TX with no sockmap insert
    ///   and no ordering to violate.)
    /// On ANY error, the port owns the cleanup: every owned leg is closed (OUTBOUND:
    /// leg F + any opened leg B; INBOUND: leg C + any opened leg S), no pump or
    /// kTLS state leaks, and NO cleartext byte reached the wire
    /// (OUTBOUND: the peer wire; INBOUND: the server workload's leg S — nothing is
    /// spliced) — the confidentiality invariant the whole feature rests on.
    ///
    /// # Observable invariants
    /// `enforce` is NOT idempotent and NOT replayable — each call enforces ONE
    /// distinct connection (a fresh leg F). The returned `EnforcedConnection.id`
    /// is unique per call within a node session.
    async fn enforce(&self, conn: InterceptedConnection) -> Result<EnforcedConnection>;

    /// The current liveness of the agent-light primary pump for `handle` — the
    /// request-carrying direction. The two primaries are NOT the same primitive:
    /// OUTBOUND observes the FORWARD pump (`legF → legB`, a `read → write_all` COPY
    /// into kTLS-TX — per-record `read`+`write`, NOT a splice, NOT zero-copy);
    /// INBOUND observes the DELIVER pump (`splice(legC → legS)`, a zero-copy
    /// `splice` out of a plain no-psock kTLS-RX leg, ~1 splice/record —
    /// `findings-inbound-intercept.md` §5 / `findings-splice-return.md`). Both are
    /// agent-light (the agent runs no cipher). This method observes the primary;
    /// it does not drive it (the adapter's own task does — SD-2). The shared
    /// bytes-moved progress counter is the same shape for either primitive.
    ///
    /// # Preconditions
    /// `handle` was returned by a prior `enforce` on THIS adapter and not yet
    /// `teardown`'d. A handle for an unknown/torn-down connection reports
    /// `Gone` (NOT an error — the post-teardown observable, mirroring
    /// `Driver::status` returning `NotFound` after `stop`).
    ///
    /// # Postconditions
    /// Returns `Running` while the pump is draining records OR is idle-but-ready
    /// (no record pending); `Stalled { since }` when the pump's bytes-spliced
    /// progress metric has NOT advanced for `MtlsLimits::pump_stall_deadline`
    /// (F7 default 30 s) WHILE a record is pending on the kTLS-RX leg (a
    /// crashed/stranded pump — the reliability sensitivity point ADR-0069 § ATAM
    /// names); or `Gone` after teardown / leg close. A purely-idle connection
    /// (no pending record) is `Running`, never `Stalled` (no false positives on
    /// quiescent long-lived connections).
    ///
    /// # F6 supervision policy (what the worker does with `Stalled`)
    /// The worker (D-MTLS-10) point-queries this on its reconciler-tick cadence
    /// (SD-4). On observing `Stalled`, the worker MUST `teardown(handle)` —
    /// **teardown + fail-closed reset** (close the legs, stop the pump, reclaim
    /// kTLS/sockmap state). It does NOT reconnect-in-place (a foreign process
    /// cannot resume a kTLS record sequence) and does NOT degrade to a userspace
    /// copy loop (that re-enters the per-byte path A3 rejects). The connection
    /// drops; request-retry protocols re-handshake on reconnect. Telemetry:
    /// `mtls.pump.stalled` + `mtls.pump.teardown_on_stall` per allocation.
    ///
    /// # Observable invariants
    /// Read-only: `liveness` never mutates the pump or the connection. This is
    /// the worker's supervision surface (D-MTLS-10: the agent supervises the
    /// splice pump) — analogous to `Driver`'s exit-event observation, but a
    /// point query rather than a stream (SD-4 surfaces the stream alternative).
    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness;

    /// Tear `handle` down: stop BOTH pumps (outbound: forward `legF → legB`
    /// write_all copy + return `splice(legB → legF)`; inbound: deliver
    /// `splice(legC → legS)` + response `legS → legC` write_all copy), drop the
    /// kTLS state, and close both legs (outbound: leg F + leg B; inbound: leg C +
    /// leg S). No sockmap state exists to reclaim (the sockmap was retired
    /// 2026-06-13, D-MTLS-13). Phase 4 of ADR-0069. This is also the F6
    /// stall-recovery action (the worker calls it on observing
    /// `PumpLiveness::Stalled`).
    ///
    /// # Preconditions
    /// `handle` was returned by a prior `enforce`. Idempotent: tearing down an
    /// already-torn-down (or unknown) handle is `Ok(())`, NOT an error — mirrors
    /// `Driver::stop` / `deregister_local_backend` idempotency.
    ///
    /// # Postconditions on `Ok(())`
    /// Both legs are closed; both pump tasks have stopped; no kTLS state for this
    /// connection remains (and no sockmap state ever existed to leak —
    /// D-MTLS-13); `liveness(&handle)` returns `Gone`. The workload's connection
    /// is closed (the proxy owned both legs; no restart-survival in v1 —
    /// D-MTLS-2 / ADR-0069 Negative).
    ///
    /// # Observable invariants
    /// After `teardown`, no further bytes move for this connection in either
    /// direction; the per-connection resources are fully reclaimed (no fd/pump
    /// leak), which the equivalence harness asserts by re-querying `liveness`.
    async fn teardown(&self, handle: EnforcedConnection) -> Result<()>;
}
```

### Error type + Result alias

A `thiserror` enum with **cause-distinct** variants for exactly the failure modes
the spikes surfaced (no catch-all `Internal(String)` — `.claude/rules/development.md`
§ Errors), `#[from]` pass-through for the consumed `IdentityRead` absence boundary,
and a `*Probe`-style refuse-to-start variant. The matching `Result` alias follows
CLAUDE.md § "Rust library conventions".

```rust
/// Result alias used throughout the crate's mTLS-enforcement surface.
pub type Result<T, E = MtlsEnforcementError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum MtlsEnforcementError {
    /// `IdentityRead::svid_for(&alloc) == None` — no held SVID for the
    /// allocation whose connection was intercepted. The fail-closed signal:
    /// the proxy refuses the handshake rather than presenting a stale/absent
    /// credential (constraint: "Reader of `IdentityRead`, never an issuer";
    /// `identity_read.rs` clause 3). Distinct from `AbsentBundle` so the
    /// operator sees WHICH side of the identity read was empty.
    #[error("no held SVID for allocation {alloc}; refusing handshake (fail-closed)")]
    AbsentSvid { alloc: AllocationId },

    /// `IdentityRead::current_bundle() == None` — no hydrated trust bundle to
    /// verify the peer against. Fail-closed: the proxy will not complete a
    /// handshake it cannot verify. Distinct from `AbsentSvid` (own identity) —
    /// this is the peer-verification anchor.
    #[error("no hydrated trust bundle; cannot verify peer (fail-closed)")]
    AbsentBundle,

    /// The leg-B rustls TLS 1.3 handshake aborted — a TLS alert, a wrong/expired
    /// presented SVID rejected by the peer, or a handshake timeout. No kTLS was
    /// armed; no cleartext egressed. Carries the rustls-side reason as a string
    /// (the rustls error is not a stable typed surface to embed).
    #[error("leg-B TLS handshake failed: {reason}")]
    HandshakeFailed { reason: String },

    /// The peer's presented certificate did not chain to `current_bundle()`'s
    /// anchor. Fail-closed: the connection is refused. This is the
    /// **authentication** failure (the peer is not a valid cluster workload).
    /// Distinct from `HandshakeFailed` so a peer *identity* rejection is not
    /// conflated with a transport/alert failure — the operator's remediation
    /// differs (wrong peer identity vs broken TLS).
    #[error("peer verification failed against the trust bundle: {reason}")]
    PeerVerificationFailed { reason: String },

    /// The authenticated peer chains to the trust bundle (it IS a valid cluster
    /// workload) but its SPIFFE-SAN does NOT match the expected destination
    /// `InterceptedConnection::expected_peer` — a wrong-but-valid peer. This is
    /// **expected-destination identity pinning** (F1), NOT authorization (that is
    /// #27's BPF-LSM `socket_connect` hook). Reserved now; **v1 never produces it
    /// because `expected_peer` is `None`** — it wires the moment east-west
    /// SPIFFE-ID resolution
    /// ([#178](https://github.com/overdrive-sh/overdrive/issues/178)) supplies the
    /// expected peer. Distinct from `PeerVerificationFailed` (authn) so the
    /// "valid peer, wrong destination" case is diagnosable on its own.
    #[error("peer identity mismatch: authenticated peer is not the expected destination: {reason}")]
    PeerIdentityMismatch { reason: String },

    /// The workload streamed more than `MtlsLimits::max_prearm_bytes` of pre-arm
    /// plaintext into leg F before kTLS armed on leg B (F4 — the DoS guard on the
    /// lossless capture buffer). Fail-closed: the buffer is dropped, leg F reset,
    /// NO cleartext egresses. Cause-distinct (NOT a generic `Io`) so the operator
    /// sees a resource-limit trip, not an I/O error.
    #[error("pre-arm buffer limit exceeded for allocation {alloc}: capped at {max_prearm_bytes} bytes (fail-closed)")]
    BufferLimitExceeded { alloc: AllocationId, max_prearm_bytes: usize },

    /// The leg-B handshake-and-arm did not complete within
    /// `MtlsLimits::handshake_deadline` (F4 — a stalled peer must not pin agent
    /// resources). Fail-closed: legs closed, no cleartext. Distinct from
    /// `HandshakeFailed` (an active TLS abort) — this is the *deadline* trip, a
    /// different remediation (slow/stalled peer vs broken TLS).
    #[error("leg-B handshake exceeded deadline {deadline:?} for allocation {alloc} (fail-closed)")]
    HandshakeTimeout { alloc: AllocationId, deadline: Duration },

    /// The per-allocation in-flight (pre-arm) connection ceiling
    /// `MtlsLimits::max_inflight_per_alloc` is already reached for this
    /// allocation (F4 — one workload cannot exhaust the agent by opening many
    /// stalled connections). Fail-closed: the new intercept is refused, no
    /// cleartext. Backpressure is *refuse*, never *queue-unbounded*.
    #[error("in-flight connection limit {limit} reached for allocation {alloc}; refusing new intercept (fail-closed)")]
    InFlightLimitExceeded { alloc: AllocationId, limit: u32 },

    /// `setsockopt(TCP_ULP "tls")` / `TLS_TX` / `TLS_RX` refused on the kTLS leg
    /// (leg B outbound / leg C inbound) after a successful handshake — the kTLS
    /// arm itself failed (kernel rejected the crypto_info, the ULP was already
    /// set, the kernel TLS module is absent, etc.). The extracted secrets were
    /// valid (handshake completed) but the kernel would not take them. Distinct
    /// from `HandshakeFailed` per `.claude/rules/development.md` § Errors — the
    /// failing layer is the kTLS install, not the handshake.
    #[error("kTLS arm refused by kernel: {source}")]
    KtlsArmFailed {
        #[source]
        source: std::io::Error,
    },

    /// **(REMOVED 2026-06-13, D-MTLS-13.)** `ArmingOrderViolation` (SOCKMAP insert
    /// after `TCP_ULP "tls"` = `EINVAL`) and `ForwardRedirectFailed` (the sockmap
    /// egress-redirect install failed) are GONE from the shipped surface — the
    /// forward path is now an agent-light `read → write_all` copy into kTLS-TX with
    /// no sockmap insert and no redirect to fail. There is no `sk->sk_prot`-ordering
    /// invariant and no BPF attach on the forward path to surface as a typed
    /// variant. (Recorded here so a reader of this design narrative does not expect
    /// the deleted variants.)
    ///
    /// The Earned-Trust `probe` sentinel round-trip failed — the kTLS-arm +
    /// agent-light forward-encrypt substrate (the `write_all`-into-kTLS-TX copy +
    /// the peer's kTLS-RX decrypt) did NOT round-trip clean on the
    /// loopback sentinel. The node MUST refuse to start
    /// (`health.startup.refused`); the proxy is not trustworthy. Mirrors
    /// `DataplaneError::LocalBackendProbe` / `ReverseLocalProbe`. `which` names
    /// the sentinel that failed so the refusal is diagnosable without
    /// `Display`-grepping.
    #[error("mTLS proxy probe round-trip failed [{which}]: {message}")]
    Probe { which: ProbeSentinel, message: String },

    /// Teardown could not fully reclaim a connection's resources — a leg close or
    /// pump stop errored (no sockmap membership exists to reclaim post-2026-06-13,
    /// D-MTLS-13). Surfaced (not swallowed) so a resource leak is observable; the
    /// equivalence harness asserts no leak on the `Ok` path.
    #[error("teardown of connection {id} failed: {source}")]
    TeardownFailed {
        id: EnforcedConnectionId,
        #[source]
        source: std::io::Error,
    },

    /// Underlying host I/O not covered by a more specific variant (leg-F
    /// `accept`/fd plumbing, `splice` pump setup). `#[from] std::io::Error`
    /// keeps `?` ergonomic at the host-adapter boundary, mirroring
    /// `DriverError::Io` / `DataplaneError::Io`. Specific, diagnosable failures
    /// get their own variant above; this is the genuine residual only.
    #[error("mTLS enforcement I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Which probe sentinel failed (for the refuse-to-start diagnosis). The substrate
/// the proxy now relies on is "kTLS arm + agent-light forward encrypt round-trip"
/// — a single composed round-trip sentinel (**REVISED 2026-06-13, D-MTLS-13**: the
/// obsolete `ForwardEgressRedirect` and `ArmingOrderEinval` sockmap sentinels were
/// dropped with the sockmap mechanism).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSentinel {
    /// kTLS arm on a loopback leg + the agent-light forward encrypt pump
    /// (`read → write_all` into the kTLS-TX leg) moving one record through it,
    /// reconstructed byte-exact by the peer's kTLS-RX (`findings.md` A; the shipped
    /// `crates/overdrive-dataplane/src/mtls/`).
    KtlsArmRoundTrip,
}

/// Liveness of a connection's agent-light splice pump (return outbound / deliver
/// inbound). F6: the worker tears the connection down on `Stalled` (teardown +
/// fail-closed reset; see `liveness` § "F6 supervision policy").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpLiveness {
    /// The pump is draining records OR idle-but-ready (no record pending) — the
    /// path is live. A quiescent long-lived connection is `Running`, not `Stalled`.
    Running,
    /// The pump's bytes-spliced progress metric has NOT advanced since `since`,
    /// for at least `MtlsLimits::pump_stall_deadline` (F7 default 30 s), WHILE a
    /// record is pending on the kTLS-RX leg (a stranded/crashed pump — the path
    /// is broken; ADR-0069 § ATAM reliability sensitivity). The worker's F6 policy
    /// reacts by tearing the connection down.
    Stalled { since: UnixInstant },
    /// No live pump for this handle — torn down or never enforced (post-teardown
    /// observable; not an error).
    Gone,
}
```

> Note on `AbsentSvid` vs a `#[from]` IdentityRead error: `IdentityRead` returns
> `Option<SvidMaterial>` (absence is `None`, NOT a typed error — `identity_read.rs`
> is infallible by design), so there is no `IdentityReadError` to `#[from]`. The
> `#[from]` pass-through the dispatch asks for applies to the **I/O boundary**
> (`std::io::Error`, the host-syscall surface), which `Io` provides. The
> `IdentityRead` absence is mapped explicitly to the cause-distinct `AbsentSvid` /
> `AbsentBundle` variants — which is the *more* honest shape than a blanket
> `#[from]`, because it names WHICH read was empty (own-identity vs peer-anchor).

### `SimMtlsEnforcement` sketch (DST-testability confirmation)

The sim adapter confirms the contract is DST-observable: it mirrors the
**observable contract state** in memory (no kernel, no `splice`, no BPF), so the
`mtls_enforcement_equivalence` harness drives both adapters through the same call
sequence and asserts identical observable reads — exactly the `SimDataplane` /
`SimIdentityRead` pattern. What DST asserts on: the per-connection enforcement
RESULT (was it established or fail-closed-refused, and WHY), the pump liveness
transitions, and the teardown reclamation — NOT kernel internals (those are Tier-3).

```rust
/// In-memory [`MtlsEnforcement`] double for DST. Models the OBSERVABLE contract,
/// not the kernel mechanism: it records, per enforced connection, the outcome
/// (`Established` / a fail-closed reason), the pump liveness, and teardown — so a
/// DST scenario asserts the SAME observable trajectory the host adapter produces,
/// without sockops / kTLS / splice / BPF.
///
/// # What the scenario preloads (the fail-closed fork inputs)
/// Like `SimIdentityRead`, the double is driven by a preloaded `IdentityRead`
/// (the held-set snapshot) + a per-`AllocationId` scripted outcome map. To
/// exercise a fail-closed path a scenario preloads an `IdentityRead` WITHOUT the
/// alloc (→ `AbsentSvid`), or scripts a `HandshakeFailed` / `PeerVerificationFailed`
/// outcome for an alloc. The happy path (alloc present + no scripted failure)
/// yields `Ok(EnforcedConnection)` with `liveness == Running`. **Both directions
/// (F3)**: the double reads `conn.routed`'s `Direction` and models the SAME
/// observable outcome for `Outbound` and `Inbound` (the `nocert`/`wrongca` inbound
/// fail-closed maps to the same `PeerVerificationFailed` the host produces). **F6**:
/// a scenario can script a pump that transitions `Running → Stalled` (no progress
/// past `pump_stall_deadline`) so the worker's teardown-on-stall policy is
/// DST-observable without a real splice.
pub struct SimMtlsEnforcement {
    /// Consumed identity read surface — REQUIRED constructor param (no default),
    /// per port-trait discipline. The double reads `svid_for` / `current_bundle`
    /// exactly as the host adapter would, so the `AbsentSvid` / `AbsentBundle`
    /// fail-closed branches are driven by the SAME `None` the host sees.
    identity: Arc<dyn IdentityRead>,
    /// Resource bounds — REQUIRED constructor param (F4), same shape the host
    /// adapter takes. The double models the limit trips observably: a scenario
    /// can drive `BufferLimitExceeded` / `HandshakeTimeout` /
    /// `InFlightLimitExceeded` deterministically (e.g. preload N in-flight
    /// connections for an alloc, then assert the N+1th `enforce` refuses).
    limits: MtlsLimits,
    /// Scripted per-alloc outcomes (absent ⇒ derive outcome from `identity`):
    /// lets a scenario inject `HandshakeFailed` / `KtlsArmFailed` /
    /// `PeerIdentityMismatch` deterministically. (`ArmingOrderViolation` was
    /// removed 2026-06-13 with the sockmap — D-MTLS-13.)
    scripted: BTreeMap<AllocationId, ScriptedOutcome>,
    /// Observable state DST asserts on: per established connection, its liveness.
    /// `BTreeMap` for deterministic iteration (the equivalence guard walks it).
    connections: Mutex<BTreeMap<EnforcedConnectionId, PumpLiveness>>,
    /// Whether `probe` is scripted to refuse (default: Ok) — exercises the
    /// refuse-to-start path deterministically.
    probe_outcome: Result<(), (ProbeSentinel, String)>,
}

// enforce(): read svid_for(&conn.alloc) on the preloaded identity → None yields
//   AbsentSvid (the real fail-closed branch, same input as host); else consult
//   `scripted` for an injected failure; else record Running + return a handle.
//   The pre-arm-capture / handshake / kTLS-arm / forward-redirect are MODELLED as
//   "outcome decided," not performed — the double asserts the *contract outcome*,
//   the Tier-3 wire test asserts the *kernel mechanism*.
// liveness(): point-read `connections`. teardown(): set Gone, drop the entry.
// probe(): return the scripted outcome (Ok or a typed Probe error → refuse-to-start).
```

The equivalence harness (`mtls_enforcement_equivalence`, a future DELIVER step,
mirroring `identity_read_equivalence` / `ReverseNatLockstep`) drives both adapters
through, **for BOTH directions** (F3): probe → enforce(`Outbound`, present alloc)
→ assert `Running` → teardown → assert `Gone`; probe → enforce(`Inbound`, present
alloc) → assert `Running` → teardown → assert `Gone`; and
enforce(absent alloc) → assert `Err(AbsentSvid)` in each direction. It
additionally exercises:
- the **F4/F7 resource-limit fail-closed branches** (preload the in-flight
  ceiling for an alloc → assert the next `enforce` returns `InFlightLimitExceeded`;
  script an over-budget pre-arm stream → `BufferLimitExceeded`; a stalled
  handshake → `HandshakeTimeout`) — observable in BOTH adapters; and **asserts the
  CONCRETE F7 default values** (e.g. the `BufferLimitExceeded` trip fires at
  exactly 256 KiB + 1; the `HandshakeTimeout` at the 5 s deadline; the
  `InFlightLimitExceeded` at the 129th concurrent pre-arm; `pump_stall_deadline`
  == 30 s) — not merely that the fields exist.
- the **F6 pump-stall → teardown policy**: script a pump that stops making
  progress with a record pending → assert `liveness` transitions to
  `Stalled { since }` within `pump_stall_deadline`, assert the worker's
  supervision loop tears the connection down, assert `Gone` after (no leak). The
  sim models the same observable transition; the host adapter proves it at Tier-3
  (pause the splice task with a pending RX record).

The host adapter additionally carries the Tier-3 obligations the sim cannot model
(they are the kernel-mechanism layer, asserted at Tier 3 per ADR-0069 §
Enforcement):

- The **composed walking-skeleton gate (F2 — the FIRST DELIVER slice, BLOCKING)**:
  real `cgroup_connect4` intercept → pre-arm write → leg-B handshake → kTLS arm →
  **post-arm bidirectional multi-record transfer with NO RST**, under BOTH normal
  AND traced/delayed timing. This is an **integration gate, not a "prove-the-mechanism"
  gate**: the mechanism is spike-verified (the primitives in isolation AND the inbound
  flow composed end-to-end in `findings-inbound-intercept.md` increment-i §2). It closes
  the three narrow gaps — (1) the OUTBOUND path composed in one flow (its pieces proven
  on separate harnesses; increment-e's steady-state RST was a throwaway-harness
  limitation, NOT a kernel finding, superseded by increment-f), (2) bidirectional
  round-trip, (3) the real netns/veth topology — and supersedes the old in-band walking
  skeleton. Must pass before any other slice.
- The wire assertions (OUTBOUND): `tcpdump` shows TLS 1.3, zero cleartext on the
  peer wire, `tls-ULP-after-sockmap == EINVAL`, the handshake-window capture is
  lossless.
- The **inbound Tier-3 obligations (F3)**, grounded strictly in
  `findings-inbound-intercept.md`: (a) **orig-dst recovery** — a TPROXY-intercepted
  connection to a server workload's logical address recovers the original
  destination via `getsockname()` on leg C (§1); (b) **server-mTLS fail-closed on
  `nocert`/`wrongca`** — a client with no cert or a cert from an untrusted CA is
  rejected with a distinct reason and NO plaintext is spliced to the server
  workload (§4); (c) **byte-exact plaintext to the server workload** on a valid
  client cert, while the client-facing leg carries only TLS `0x17` records (§2/§3);
  (d) **agent-light** — `strace` shows the agent moves the inbound payload via
  `splice`/`ppoll` only, zero per-byte payload copy (§5); leg C carries no psock.
  (The loopback-only spike must be re-proven in the netns/veth topology — the
  spike's named scope boundary.)
- The **F5 intercept-exemption obligations**: (a) the agent's leg-B dial is NOT
  re-intercepted (no recursion); (b) a workload CANNOT self-exempt (the
  `SO_MARK`/cgroup bypass is agent-private). References the
  `cgroup_connect4_service` attach boundary. (For inbound, the agent's leg-S dial
  to the server workload is a same-node plaintext dial that must likewise not be
  TPROXY-re-intercepted — the TPROXY rule targets the workload's logical address,
  not the agent→server leg.)
- The **F6 pump-stall obligation**: pause the splice task with a pending RX record
  → `liveness` reports `Stalled` within `pump_stall_deadline` (30 s) → the worker
  tears down → `Gone`, no leak (both directions share the pump primitive).
- The **F1/F5 authn-only boundary**: `PeerVerificationFailed` fail-closed on a peer
  that does not chain to the bundle (OUTBOUND: the dialed peer's server cert;
  INBOUND: the client's SVID — the `nocert`/`wrongca` path proven in
  `findings-inbound-intercept.md` §4), AND a **reserved negative-test placeholder
  for the wrong-but-valid-peer case** (`PeerIdentityMismatch`) — `#[ignore = "gated
  on #178 supplying expected_peer"]` until #178 lands. **The docs/tests MUST NOT
  call the wrong-but-valid-peer case "protected" until #178 lands** — v1 is
  chain-to-bundle authn + encryption, no intended-peer pinning. Authorization is
  #27's LSM hook, not this feature.

### Sub-decisions (SD-1…SD-5) — ACCEPTED (user-approved 2026-06-12)

These were the real forks within the model. Each is recorded with its accepted
option (RECOMMENDED) + the rejected alternatives and trade-off; the user approved
them 2026-06-12. They are the locked contract DELIVER builds to. (SD-5 — the
`probe` sentinel-cert source — was added 2026-06-12 during DELIVER
back-propagation when the composed walking skeleton surfaced that the probe's
handshake-based sentinel #1 needs a cert source; user-approved.)

- **SD-1 — `InterceptedConnection` payload (the connection descriptor).** What
  does the worker pass IN? **Now bidirectional (F3): the answer is the same shape
  in both directions — an owned accepted-leg `OwnedFd` + a `direction`-tagged
  routing fact + `AllocationId`.**
  - **(a, RECOMMENDED) owned accepted-leg `OwnedFd` + `Routed { Outbound { peer }
    | Inbound { orig_dst } }` + `AllocationId`.** The proxy topology is symmetric:
    OUTBOUND — "the workload's `connect()` is rewritten to the agent's leg-F
    listener; the agent `accept()`s leg F" (`findings-userspace-relay.md` Unknown
    1); INBOUND — "the connection to the server's logical addr is TPROXY-redirected
    to the agent's `IP_TRANSPARENT` listener; the agent `accept()`s leg C, and
    `getsockname` recovers the orig-dst" (`findings-inbound-intercept.md` §1). In
    both, the agent already owns the accepted leg; handing the owned fd is the
    honest shape (port takes ownership, RAII-closes on teardown, no half-moved fd).
    The `direction`-tagged `Routed` carries the one direction-specific fact (the
    peer to dial vs the orig-dst that selects the server SVID), making the
    inbound/outbound mismatch unrepresentable. **Trade-off**: couples the contract
    to "the worker does the `accept()`," which is exactly the proxy model (a
    feature, not a leak).
  - (b) a `pidfd` + raw fd handle (the SUPERSEDED in-band shape) — **rejected**: the
    proxy owns its own legs; there is no workload-socket dup in v1. Passing a
    `pidfd` would smuggle the dead in-band model's surface into the contract.
  - (c) just the 4-tuple (`src`/`dst` addr+port) + `AllocationId`, port re-derives
    the legs — **rejected**: forces the port to re-discover the accepted socket
    from the tuple (racy, and re-does work the worker already did at `accept()`).
  - (d) separate `enforce_outbound` / `enforce_inbound` methods carrying
    direction-specific structs — **rejected**: duplicates every postcondition and
    doubles the sim's mirror surface; the observable contract is identical either
    way (see § "Bidirectional shape decision"). A `direction` discriminant on ONE
    method + ONE descriptor is minimal.
  - **Deciding factor**: who owns the accepted leg (the agent, both directions) +
    keep ONE observable contract. **Recommend (a).**

- **SD-2 — splice pump ownership (port-owns vs worker-drives).** The
  agent-light pump (`splice(legB→pipe→legF)` outbound return / `splice(legC→pipe→
  legS)` inbound deliver, ~1/record) must be driven for the connection's life,
  in EITHER direction. Who owns the driving loop?
  - **(a, RECOMMENDED) the PORT owns the pump.** `enforce` returns once
    steady-state is established; the host adapter spawns its own task that drives
    the splice pump; the worker observes liveness via `liveness(&handle)` and tears
    down via `teardown`. **Trade-off**: the port surface stays small (establish /
    observe / tear down — 4 methods); the pump-driving mechanism (`splice` cadence,
    `ppoll` readiness) is adapter-internal HOW, correctly hidden. The worker
    supervises *liveness*, not *each splice*. This matches `Driver` (the driver owns
    the per-alloc watcher task; the worker observes `ExitEvent`s, doesn't drive the
    process).
  - (b) the WORKER drives the pump: the port exposes `pump_once(&handle) ->
    Result<PumpProgress>` and the worker loops it. **Rejected**: leaks the
    per-record `splice` cadence into the port (mechanism in the contract), makes the
    worker responsible for readiness polling, and bloats the surface with a hot
    method the sim must model per-record. The agent-light cost (~1 splice/record) is
    an adapter concern, not a contract concern.
  - **Deciding factor**: keep mechanism out of the port. The pump is HOW the return
    path moves bytes; the port should expose only that it is *running / stalled /
    gone*. **Recommend (a)** — and it is what the trait above is written to.

- **SD-3 — `probe` async vs sync.** `Dataplane`'s probe-style checks are folded
  into its async methods; `CgroupFs::probe` is sync. The mTLS probe does a real
  loopback handshake + `splice` + BPF attach.
  - **(a, RECOMMENDED) `async fn probe`.** The probe performs real kernel I/O (a
    sentinel rustls handshake, a `tls_sw_splice_read`, a sockmap attach) — genuinely
    awaits. Async matches the I/O reality and the `#[async_trait]` boundary the rest
    of the trait uses. **Trade-off**: the composition root awaits it at startup
    (already an async context — `run_server`).
  - (b) sync `probe` that blocks — **rejected**: blocks a tokio worker on real
    kernel I/O (the no-blocking-in-async rule), and the composition root is already
    async. **Recommend (a)** (the trait above uses `async fn probe`).

- **SD-4 — pump-liveness as a point query vs an event stream.** The worker
  supervises the return pump. `liveness(&handle) -> PumpLiveness` is a point query
  (above). The alternative is a `take_pump_events() -> Receiver<PumpEvent>` stream
  (the `Driver::take_exit_receiver` shape).
  - **(a, RECOMMENDED for v1) point query `liveness`.** The worker's supervision
    need in v1 is "is this connection's return path still alive, and if not since
    when" — a periodic point query (the reconciler-tick cadence the rest of the
    worker already runs on) answers it. Minimal surface; trivially sim-modellable.
    **Trade-off**: the worker polls rather than being pushed; for a per-connection
    liveness check on a tick that is acceptable and matches the platform's
    converge-on-tick model.
  - (b) event stream `take_pump_events()` — **deferred, not rejected**: a push
    stream is the better shape IF the worker needs immediate pump-death
    notification (e.g. to re-handshake fast). That is a reliability optimisation of
    the ATAM sensitivity point (stranded pump), not a v1 need. If the user wants it,
    it is additive (the point query stays). **Recommend (a) for v1**; note (b) as a
    clean additive follow-up if fast pump-death reaction is wanted.

- **SD-5 — `probe` sentinel-cert source (substrate-self-test crypto).**
  ACCEPTED (user-approved 2026-06-12). `probe`'s sentinel #1 (the contract
  above, `probe` postcondition (1)) requires a **handshake-based** kTLS-arm
  round-trip — "a sentinel rustls handshake on a loopback leg B arms kTLS and a
  single `tls_sw_splice_read` of one record returns the exact sentinel
  plaintext." A rustls handshake needs a cert+key. But the production adapter
  holds only `IdentityRead` (a READER — #26 is **never an issuer**; CLAUDE.md
  "two distinct CAs" / "workload-holds-nothing"), and `rcgen` is currently a
  **test-only** dependency of `overdrive-dataplane`. So the adapter as designed
  cannot mint the sentinel cert the loopback self-test handshake needs. Where
  does the sentinel cert come from?
  - **(a-runtime, RECOMMENDED) `probe` mints an ephemeral self-signed sentinel
    cert+key in-process via `rcgen`.** Promote `rcgen` from `[dev-dependencies]`
    to production `[dependencies]` in `overdrive-dataplane`; `probe()` generates
    a throwaway self-signed cert+key at call time, uses it ONLY for the loopback
    self-test handshake (the agent talks to itself over leg F/leg B), and drops
    it before return (the existing § Observable-invariants "leaks no sentinel
    state" teardown already covers it). **Trade-off**: one new production dep on
    `overdrive-dataplane` — but `rcgen` is ALREADY in the workspace graph (this
    crate's own test-dep), so the cost is a one-line `[dependencies]` promotion
    with no new vendoring, and **no private key is baked into the shipped
    binary**. Matches the spike-scratch pattern the probe was de-risked against
    (`findings.md` A, `findings-egress-ktls-splice.md`).
  - (b-static) embed a FIXED self-signed sentinel cert+key as `const` bytes —
    **rejected**: no runtime cert-gen and no new production dep, but it compiles
    a private key into the shipped artifact. Even though that key is
    loopback-only and identifies nothing, a key-at-rest in the binary is a
    standing item every SBOM / secret-scanner flags and every future reviewer
    must reason about ("why is there a private key in `overdrive-dataplane`?"),
    for no benefit over (a-runtime) since `rcgen` is already on hand.
  - **The sentinel is substrate-probe crypto, NOT a workload identity.** The
    sentinel cert is signed by **neither** the operator / control-plane HTTPS CA
    (`mint_ephemeral_ca`) **nor** the workload-identity CA (`RcgenCa` /
    `boot_ca`); it **never enters the trust bundle**; it **never reaches a real
    peer** (loopback agent-to-itself only); it identifies no workload and is
    discarded after the self-test. Therefore #26 stays a **reader** of
    `IdentityRead` — generating a throwaway loopback self-test cert is NOT
    "issuing an SVID," and **"workload-holds-nothing" / "two distinct CAs"
    remain intact**. The `WebPkiClientVerifier` in `enforce` still verifies real
    peers against the real `TrustBundle`; `probe` does not touch that path.
  - **Deciding factor**: do the full 3-sentinel round-trip the contract already
    specifies, with no key-at-rest in the artifact, reusing a workspace dep.
    **Recommend (a-runtime).** Note for a future reviewer: the
    `overdrive-dataplane` → `rcgen` PRODUCTION-dep edge added by this decision is
    self-test crypto only; it does **not** make #26 an issuer.

### OQ-2 resolution — `HostMtlsEnforcement` home

**RESOLVED (user-decided 2026-06-12): NO new crate. Extend the existing crates.**
The `HostMtlsEnforcement` userspace adapter extends **`overdrive-dataplane`**; the
new kernel-side programs extend **`overdrive-bpf`**; `SimMtlsEnforcement` stays in
`overdrive-sim`. This reverses this section's prior "dedicated `overdrive-mtls-host`
crate" recommendation — the deciding factor that recommendation rested on
(`overdrive-host` cannot host irreducibly-`unsafe` code) is real, but it argued
only for *not `overdrive-host`*, not for a *new* crate. **`overdrive-dataplane`
already satisfies every requirement the new crate was invented to provide.**

**Verified facts (every prior new-crate rationale is already met by
`overdrive-dataplane`):**

1. **`unsafe` already allowed.** `overdrive-dataplane` is `crate_class =
   "adapter-host"` with **no `forbid`/`deny` on `unsafe`** — 9 `src` files already
   use `unsafe` (the raw `setsockopt`/`splice`/BPF-fd surface the proxy needs sits
   among code shaped exactly like this). No forbid-lift, no erosion: the unsafe is
   *expected* here, as it is the established userspace eBPF host adapter.
2. **`aya` already a dependency** (`aya.workspace = true`). The BPF loader +
   `pinning = ByName` discipline (ADR-0038/0040) is already wired here; reuse, do
   not re-add.
3. **A BPF `build.rs` already present.** `overdrive-dataplane`'s `build.rs` already
   carries the `overdrive_bpf.o` dependency (CLAUDE.md's bpf-build-prereq footgun is
   about *this* crate). The new `cgroup_connect4`-mtls program compiles into the
   same shared BPF object via `overdrive-bpf` — no new build coupling to invent.
   (The sockops/`sk_skb` forward-redirect programs were retired 2026-06-13,
   D-MTLS-13 — the forward path is now a userspace `splice` pump.)
4. **"The crate that talks to the kernel."** `overdrive-dataplane` already hosts
   `EbpfDataplane`; it IS the userspace↔kernel host adapter. Adding `ktls` +
   `rustls` is a modest dep bump on a crate already in the kernel-dataplane graph.

**`overdrive-host` ruled out** — `crates/overdrive-host/src/lib.rs:21` is
`#![forbid(unsafe_code)]` (the safe-bindings crate: OS clock, OS entropy, host TCP
transport, cgroup-fs, the `RcgenCa` adapter over safe `ring`/`rcgen`, the safe
`linux-keyutils` wrapper). The proxy is irreducibly `unsafe`
(`setsockopt(TCP_ULP/TLS_TX/TLS_RX)`, `splice(2)`, `pidfd`/BPF-fd plumbing through
`libc`; `findings.md` D's 56-byte `tls12_crypto_info_aes_gcm_256` hand-roll), so it
cannot share that crate without lifting a load-bearing safety property for every
unrelated safe module.

**Kernel programs → `overdrive-bpf`.** The new `cgroup_connect4`-mtls (outbound
intercept) program lives alongside the existing `cgroup_connect4_service.rs` / XDP
programs — one shared BPF object, per Component Decomposition #5. **(REVISED
2026-06-13, D-MTLS-13: the sockops `sock_ops_mtls_enroll` and the
`sk_skb/stream_verdict` forward egress-redirect programs were RETIRED with the
sockmap — the forward path is now a userspace `splice` pump with no kernel-side
program.)**

**Architectural-enforcement rule (ADR-0069 § Enforcement) restated for the resolved
homes**: `overdrive-dataplane`/`overdrive-bpf` own the kernel/eBPF surface; the
proxy's sockops/sk_msg/sockmap/kTLS/`splice` syscalls appear in no other crate —
consistent with `EbpfDataplane` already living in `overdrive-dataplane`, and
enforceable by a grep/ArchUnit-style gate asserting those syscalls are absent
elsewhere. dst-lint is unaffected (both crates are `adapter-host`, not scanned).

**The genuine trade — recorded as the revisit trigger, not a blocker.** A dedicated
crate would isolate the proxy's `ktls`/`rustls` TLS stack from the LB/service
dataplane's compile graph (concern isolation; a narrower blast radius). That was
weighed and judged NOT worth a new crate "for now." **Revisit if** mTLS later needs
isolation from the LB/service dataplane — then split `overdrive-dataplane`'s mTLS
surface into a dedicated `adapter-host` crate, the `MtlsEnforcement` port boundary
(unchanged) making that a non-breaking move.

### Guest-stack adapter handoff — STAGED to #222 (F4)

**The host-socket path (process/exec, WASM) is bidirectional v1 (outbound +
inbound, both designed above). The guest-stack intercept adapter (microVM /
unikernel) is STAGED to a follow-up:
[#222](https://github.com/overdrive-sh/overdrive/issues/222).** #222 is no longer
a *separate enforcement mechanism* — ADR-0069 folded it into #26's ONE universal
agent-light L4 proxy. What remains for #222 is the **guest-stack intercept
ADAPTER**: the same `MtlsEnforcement` port, the same `InterceptedConnection` /
`EnforcedConnection` contract, the same agent-light splice pumps — fed by a
guest-stack-specific intercept source instead of `cgroup_connect4` / `nft`-TPROXY.

Guest-stack workloads terminate TCP in the *guest* kernel (invisible to host
sockops / `cgroup_connect4`), so the intercept moves to the **tap/TPROXY/TC
boundary** where the guest's virtio-net/tap flow meets the host. The adapter's job
is to produce the SAME `InterceptedConnection` semantics the host path produces:

- **Intercept source**: the microVM/unikernel's tap (Cloud Hypervisor virtio-net
  tap) / TC / TPROXY boundary on the host side of the guest NIC — the place the
  research already recommended a host L4 transparent mTLS proxy
  (`transparent-mtls-recommended-architecture-research.md`;
  `findings-userspace-relay.md` concludes the lossless path collapses into this
  same two-socket host L4 proxy shape). Outbound: intercept the guest's egress
  flow; inbound: intercept the flow aimed at the guest's logical address (the same
  TPROXY mirror the host inbound path uses).
- **`AllocationId` lookup**: map the virtio-net/tap flow (the tap device / the
  guest's source or destination on that tap) to the owning `AllocationId` — the
  guest-stack analogue of "which workload owns this socket." The control plane
  owns the tap↔allocation binding (one tap per microVM allocation).
- **Original-destination recovery**: recover the flow's original destination from
  the tap/TPROXY boundary (TPROXY `getsockname` for the inbound mirror; the
  egress flow's dst for outbound) — the guest-stack analogue of the host
  `getsockname`/`connect4`-rewrite orig-dst.
- **Conversion into `InterceptedConnection`**: the adapter `accept()`s the
  intercepted leg and constructs the SAME `InterceptedConnection { leg, routed,
  alloc, expected_peer }` the host path constructs — so `enforce` /
  `liveness` / `teardown` and both splice pumps are reused VERBATIM. Only the
  intercept *source* differs; the port contract and the agent-light steady state
  are identical.

**Scope of the staging**: the `MtlsEnforcement` port + `HostMtlsEnforcement`
(host-socket, bidirectional) ship in #26 v1. The guest-stack intercept adapter
(tap/TPROXY/TC source → `AllocationId` lookup → orig-dst → `InterceptedConnection`)
is #222's deliverable, built on the unchanged v1 port boundary (so it is a
non-breaking addition — the port boundary makes that a new adapter, not a contract
change). **The orchestrator repurposes #222's body to "the guest-stack intercept
adapter for the #26 universal proxy"** (no new issue — #222 already exists and is
re-grounded by ADR-0069's fold). The product journey's stale "#222 is a SEPARATE
feature" line is corrected to the staged-adapter framing (see § F4 journey fix /
`upstream-changes.md`).

### Handoff note for the orchestrator

On user approval of this section: the crafter dispatch for the first
`MtlsEnforcement` DELIVER step MUST pin the exact signatures above verbatim and
explicitly forbid inventing surface (CLAUDE.md § "Implement to the design";
"Orchestrators dispatching crafters … pin the exact signature in the dispatch and
explicitly forbid inventing API"). The four-method surface (`probe` / `enforce` /
`liveness` / `teardown`), the newtypes (`InterceptedConnection` — now carrying the
`direction`-tagged `Routed` (F3) + the OPTIONAL `expected_peer` (F1) — / the
`Direction` + `Routed` enums (F3) / `EnforcedConnection` + `EnforcedConnectionId` /
`MtlsLimits` — now with the F7 CONCRETE defaults + the F6 `pump_stall_deadline`),
the `MtlsEnforcementError` variant set (including `PeerIdentityMismatch` (F1) +
`BufferLimitExceeded` / `HandshakeTimeout` / `InFlightLimitExceeded` (F4)), and the
`PumpLiveness` / `ProbeSentinel` enums are the complete public contract — nothing
beyond it is sanctioned without a new DESIGN decision. `enforce` dispatches on
`Direction` (one method, both directions — F3); do NOT add `enforce_inbound`.
**The dispatch MUST also carry**: the F2 composed walking-skeleton gate as the
FIRST DELIVER slice (BLOCKING, before any other slice); the **F3 inbound Tier-3
obligations** (orig-dst recovery, server-mTLS fail-closed on `nocert`/`wrongca`,
byte-exact plaintext to the server workload, agent-light strace — grounded in
`findings-inbound-intercept.md`); the F4/F7 resource-limit Tier-3 obligations
(**assert the CONCRETE values** 256 KiB / 5 s / 128 / 30 s, not field existence);
the **F6 pump-stall → teardown** obligation; the F5 intercept-exemption obligations;
the **D-MTLS-13 forward-mechanism obligations** (the OUTBOUND forward is an
agent-light `read → write_all` COPY into leg B's kTLS-TX — NOT a sockmap egress
redirect and NOT a splice into TX, which loses records; per-record `read`+`write`,
NOT zero-copy, NOT agent-idle, but no userspace crypto; the strace forward
observable is per-record `read`+`write` into kTLS-TX, while the return/deliver
strace is `splice`/`ppoll`-only; every reader leg drains 0.5-RTT early data via
`mtls::drain_early_plaintext` before `dangerous_extract_secrets`; the `probe` is
ONE composed `KtlsArmRoundTrip` sentinel; there is NO sockmap/`MTLS_*` map, NO
`sk_skb`/`sock_ops` program, NO `ArmingOrderViolation`/`ForwardRedirectFailed`
variant, NO arming-order Tier-3 test); and the F1/F5 authn-only boundary (authorization is #27/#38, NOT this
feature; the `expected_peer` SAN-match is the **#178 upgrade**, `#[ignore]`-gated,
and the docs/tests MUST NOT call the wrong-but-valid-peer case "protected" until
#178 lands).

**Operator-tunability — tracked in [#230](https://github.com/overdrive-sh/overdrive/issues/230)
(created 2026-06-12).** The `MtlsLimits` F7 values (256 KiB / 5 s / 128 / 30 s) are
compile-time defaults, NOT operator-tunable in v1. **Operator-tunability of
`MtlsLimits` is tracked in #230** — the v1 defaults stand as pinned, un-tunable,
compile-time constants until that work lands.
