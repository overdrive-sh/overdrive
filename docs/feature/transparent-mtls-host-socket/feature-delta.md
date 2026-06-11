<!-- markdownlint-disable MD024 -->
# Feature Delta — transparent-mtls-host-socket (GH #26 · roadmap step 2.4)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Density**: `lean` + `ask-intelligent` (DISCUSS hard default)

This is the single narrative artifact for the transparent-mtls-host-socket
feature. All DISCUSS content lives here under
`## Wave: DISCUSS / [REF] <Section>` headings. Tier-1 `[REF]` sections are emitted
(lean default); no Tier-2 expansions were auto-rendered — the fired
`ask-intelligent` triggers are reported to the orchestrator (see
`wave-decisions.md` § Density & Triggers).

This feature is the **host-socket transparent-mTLS ENFORCEMENT mechanism** — the
consumer that finally **encrypts the wire** using the SVID that the CA mints
(J-SEC-001 / #28) and the IdentityMgr holds + exposes via the `IdentityRead` port
(J-SEC-002 / #35). The **job J-SEC-003 is authored this wave** (DISCUSS minted it —
there was NO DIVERGE wave for #26; see `wave-decisions.md` § Risk). The
**mechanism is NOT pinned** (in-band kTLS [Arch A] vs proxy [Arch C] vs Cilium
out-of-band-auth fallback) — DISCUSS pins the WHAT and the acceptance observables,
and runs a **spike-first walking skeleton (Slice 00)** to settle the riskiest
mechanism question empirically before DESIGN locks.

---

## Wave: DISCUSS / [REF] Feature Summary

**What**: For **host-socket workloads** (process via the exec driver; WASM via
wasmtime — whose TCP terminates in the HOST kernel), the Overdrive node agent
turns a plaintext TCP connection into a **kernel-encrypted TLS 1.3 session
carrying the workload's own SVID**, with the **workload holding nothing** and the
**agent out of the steady-state data path**. The mechanism (subject to the Slice-00
spike): kernel **sockops** detects the connection's ESTABLISHED transition →
the node agent acquires the workload's socket (process: `pidfd_getfd`; WASM:
in-process host fd) and performs a **rustls TLS 1.3 handshake** presenting the
**held SVID** it reads via the `IdentityRead` port (J-SEC-002/#35), verifying the
peer against the trust bundle → the negotiated session keys install into **kTLS**
on the workload's socket (`setsockopt TLS_TX/TLS_RX`) → the agent **exits the data
path**. The wire then carries TLS 1.3 Application Data records, in-kernel.

**Why** (J-SEC-003): so design principle 3 — "every packet carries cryptographic
workload identity" — and principle 2 — "mTLS is in-kernel and undisableable" — are
**operationally true ON THE WIRE** for host-socket workloads, **provable by a
packet capture**. Identity is mintable (J-SEC-001) and held/readable (J-SEC-002),
but **nothing yet consumes it on the wire** — the promise is true in principle but
a packet capture would show cleartext. This feature is the **on-the-wire
ENFORCEMENT peer** of the mint(001)/hold(002) chain: 001 mints, 002 holds/reads,
**003 encrypts the wire**.

**Feature type**: Infrastructure / security primitive (D1) — no operator-facing
verb this phase; the HONEST observables are TEST-tier (wire capture, `ss -K`,
fail-closed negative test, race-window probe). Spans the dataplane (sockops +
kTLS install), the node agent (rustls handshake), and the read of the
`IdentityRead` port (`overdrive-core`).

**Scope boundary — host-socket ONLY** (corrected whitepaper §7 "one identity
model, two enforcement mechanisms"):

| In scope (#26) | Out of scope (referenced by issue #) |
|---|---|
| Process (exec driver) — TCP in host kernel | Guest-stack: microVM / unikernel (TCP in GUEST kernel) → **#222** (host L4 tap proxy, SEPARATE feature) |
| WASM (wasmtime, in-process host sockets) | In-place SVID rekey / TLS 1.3 KeyUpdate → **#229** (userspace rustls/ktls bridge gap; v1 = teardown+reconnect) |
| | Certificate-rotation workflow → **#40** (depends on #39) |
| | Revocation (CRL/OCSP) → Phase 5 · Multi-node → **#36** |

**Evidence base**: `docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`
(the recommended hybrid; host-socket vs guest-stack taxonomy — Finding 1),
`.../sockops-mtls-ktls-installation-comprehensive-research.md` (rustls→kTLS
mechanics; Gap-4 RESOLVED), `.../sockops-ktls-plaintext-race-window-research.md`
(the race-window deep-dive + comparison matrix — the WS hypothesis),
`.../ktls-sockops-cp-restart-survival-research.md` (CP-restart survival — feeds an
error path), **ADR-0068** (pinned 6.18 LTS kernel; KeyUpdate kernel-ready /
userspace-blocked), whitepaper §7/§8/§6 addendum, vision principles 2+3. Consumes
the shipped `IdentityRead` port + `Arc<IdentityMgr>` (#35) and the `Ca` hierarchy +
leaf key (#28, ADR-0063 D9).

---

## Wave: DISCUSS / [REF] Persona

- **`sam-platform-security-engineer`** (Sam Okafor) — platform/security engineer
  who builds AND operates Overdrive's identity layer; has run SPIRE + Vault and
  hated it; threat-models by default; verifies with `openssl verify` / `tcpdump` /
  `ss -K` rather than trusting the platform's word. SSOT:
  `docs/product/personas/sam-platform-security-engineer.yaml`. **Reused** from
  built-in-ca / workload-identity-manager (the persona already says "and future
  J-SEC-* jobs"); this wave adds `J-SEC-003` to `related_jobs` and a
  `j_sec_003_lens` — the **on-the-wire enforcement** lens (is the wire ACTUALLY
  TLS 1.3, not cleartext? is the auth-session the data-session? is the agent out
  of the data path? no cleartext before kTLS? fail-closed on wrong/absent SVID?).
  Same skeptical→confident security-review arc — no rich human emotional arc (D3
  Lightweight).

---

## Wave: DISCUSS / [REF] JTBD One-liner

**J-SEC-003** — *"Transparently encrypt every host-socket workload's traffic with
its own SVID, in-kernel, with the platform out of the data path — no sidecar, no
cleartext on the wire."* `relates_to: J-SEC-002`.

> When a host-socket workload I run (process or WASM, TCP terminating in the host
> kernel) opens a connection — and the platform already MINTS a forgery-proof SVID
> (#28) and HOLDS it readable via `IdentityRead` (#35) — but NOTHING yet consumes
> it on the wire, **I want** the platform to detect the connection in the kernel
> (sockops), perform the TLS 1.3 handshake on the workload's behalf (rustls,
> presenting the held SVID), install the session keys into kTLS on the workload's
> own socket, and LEAVE THE DATA PATH — and close the plaintext race window
> fail-closed — **so** principles 2 + 3 are operationally true ON THE WIRE for
> host-socket workloads (provable by wire capture showing TLS 1.3 records), the
> auth-session IS the data-session, encryption is in-kernel via kTLS, the agent is
> out of the steady-state path, and a handshake against an absent/wrong SVID fails
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
| **Pull** | sockops detects → rustls handshake presents the held SVID → kTLS installs → agent exits → wire carries TLS 1.3, agent out of the path. | Happy path + "agent out of the data path" (US-MTLS-03). |
| **Anxiety** | In-band sidecarless kTLS is unshipped; the plaintext race window may be unclosable. (Mitigated: spike FIRST; 6.18 pin removes kernel anxiety; documented Cilium fallback.) | The spike (US-MTLS-00); race-window probe (US-MTLS-04). |
| **Habit** | Sidecar injection (Istio/Linkerd) / SPIRE Workload API (workload fetches its own SVID, userspace TLS). | "Workload holds NOTHING / is identity-unaware" (US-MTLS-02). |

---

## Wave: DISCUSS / [REF] Brownfield Evaluation + Walking Skeleton (D2 — SPIKE-FIRST)

**This is a brownfield feature: a net-new ENFORCEMENT mechanism consuming
already-shipped seams (the held SVID + trust bundle via `IdentityRead`, the CA
hierarchy + leaf key). There is NO greenfield walking-skeleton proposal — and the
walking skeleton is a SPIKE (D2), because the issue mandates a Tier-3 spike before
the design locks.**

| Already shipped (consumed, not rebuilt) | Where | This feature adds |
|---|---|---|
| `IdentityRead` port (`svid_for(&AllocationId) → Option<SvidMaterial>`, `current_bundle() → TrustBundle`) | `overdrive-core` (#35, J-SEC-002) | A READER of it: the agent reads the held SVID + bundle to drive the handshake |
| `Arc<IdentityMgr>` held-SVID map + hydrated `TrustBundle` | `overdrive-control-plane` (#35) | Nothing — it reads, never mutates the held set |
| `SvidMaterial` (cert PEM/DER + serial + spiffe_id + node-held `leaf_key`, ADR-0063 D9, redacted Debug) | `overdrive-core` (#28) | The material it presents in the rustls handshake (workload never holds it) |
| `Ca` hierarchy (Root → per-node Intermediate → workload SVID) + `trust_bundle()` | `overdrive-core` (#28, ADR-0063) | Verifies the peer against `current_bundle()` |
| Pinned 6.18 LTS kernel (in-kernel TLS 1.3 TX+RX + `CONFIG_NET_HANDSHAKE`) | ADR-0068 | The kernel the spike + enforcement run on (no version anxiety) |
| eBPF + bpffs-pin discipline (`pinning = ByName`, `/sys/fs/bpf/overdrive/`) | `.claude/rules/development.md` | The sockops program + (link/map) pinning the enforcement attaches |

**Walking skeleton (D2 — SPIKE-FIRST, Slice 00)**: a Tier-3 spike on the 6.18
Lima/LVH kernel that proves the full handoff for **ONE process→process flow**:
`sockops ACTIVE_ESTABLISHED detected → agent acquires the workload's socket
(pidfd_getfd) → rustls TLS 1.3 handshake presenting the held SVID (read via
IdentityRead) and verifying the peer → kTLS install (setsockopt TLS_TX/TLS_RX) →
agent exits → tcpdump shows TLS 1.3 Application Data records on the wire`, AND the
plaintext race window is closed fail-closed (no cleartext byte egresses before
kTLS install). **The spike's observable IS the wire capture for one flow.**

**Named falsification** (load-bearing): if the handoff cannot be made race-free for
one process flow on our kernel, in-band sidecarless kTLS is **disproven for
Overdrive** → fall back to the **Cilium model** (out-of-band auth + separate
encryption, or a userspace proxy that stays in the data path, à la Architecture C).
The spike is the cheapest place to learn the riskiest assumption; every downstream
slice is additive on a proven (or fallen-back) mechanism.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate — Phase 1.5)

**Verdict: PASS — right-sized as ONE feature, sliced into 1 spike + 5 thin
vertical cuts.** Full signal table + the DIVERGE-absence risk are in
`wave-decisions.md` § Scope Assessment / § Risk. Zero oversized signals fire
(5 stories + 1 spike; 1 bounded context; spike-thinned WS; ~7–9 days; 1 coherent
outcome). The feature is already correctly carved from the guest-stack path (#222)
and the rekey/rotation/revocation/multi-node concerns (#229/#40/Phase-5/#36); it is
NOT split further.

---

## Wave: DISCUSS / [REF] Journey Visualization (ASCII flow + emotional arc + TUI/observable)

> Material honesty: there is NO operator GUI/CLI surface for this feature. The
> "TUI mockups" below are the **honest observable surfaces** — the wire capture
> (`tcpdump`), the socket-diag (`ss -K`), and the test runner — which is what Sam
> actually looks at. CLI should feel like CLI; a security primitive's surface is
> its evidence, not a dashboard.

### Horizontal flow (the complete journey, all backbone activities)

```
[Trigger: host-socket    [A. Detect the       [B. Handshake on        [C. Install kTLS,      [D. Prove on the wire]
 workload opens a TCP     connection in        the workload's          agent exits the
 connection]              the kernel]          behalf, present SVID]    data path]
        |                       |                       |                      |                      |
        v                       v                       v                      v                      v
  workload write()        sockops ACTIVE_       agent: pidfd_getfd      setsockopt(TCP_ULP    tcpdump on veth:
  (process / WASM)         ESTABLISHED fires    -> rustls TLS 1.3       'tls') + TLS_TX/RX     TLS 1.3 App Data
                           synchronously        handshake, presents     on the WORKLOAD's      records (0x17),
                           (before connect()    the HELD SVID (read     own socket; agent      NOT cleartext;
                           returns)             via IdentityRead),      EXITS the data path    ss -K: kTLS ULP
                                                verifies peer vs                                installed
                                                trust bundle
  Feels: (workload is     Feels(Sam): focused  Feels(Sam): focused      Feels(Sam):            Feels(Sam):
   identity-unaware,       'does the kernel     'does it present the     reassured 'ss -K       CONFIDENT 'a capture
   holds nothing)          catch it before      WORKLOAD's identity,     shows kTLS; the        I ran shows TLS 1.3 —
                           the workload can      not the node's?'        agent left; kernel     principle made real'
                           write?'                                       does the crypto'
  Artifacts: workload     Artifacts: sockops    Artifacts: held         Artifacts: workload    Artifacts: TLS 1.3
   socket fd               program, the          SvidMaterial +          socket fd, kTLS         records on the wire,
                           ESTABLISHED event     TrustBundle (read       crypto_info             ss -K ULP state
                                                 via IdentityRead)        (auth-session ==
                                                                          data-session)

  >>> RACE-WINDOW GATE (load-bearing): between B's ESTABLISHED and C's kTLS install, a gate armed before
      connect()/accept() returns keeps the wire FAIL-CLOSED for confidentiality (no cleartext byte reaches
      the wire before kTLS is armed). The gate MECHANISM (sk_msg DROP-until-armed vs sockmap redirect vs an
      out-of-tree write-block) is a DESIGN choice the spike informs -- NOT pinned in DISCUSS. Residual
      data-loss window (writes during the 10-100ms agent turnaround dropped, not buffered) -> connection
      RESET if drops>0, request-first protocols retry. The 6.18 pin makes an out-of-tree write-block
      (lossless) a DESIGN option.

  >>> SPIKE-FIRST (Slice 00): the ENTIRE flow above is proven for ONE process->process flow on the 6.18
      kernel BEFORE the design locks. Falsification -> fall back to Cilium out-of-band-auth + separate encryption.
```

### Emotional arc (Sam — confidence-building pattern: skeptical → reassured → confident)

```
  skeptical/                                                                          confident/
  threat-modelling                                                                    relieved
       |                                                                                  ^
       |  "in-band sidecarless kTLS is unshipped;                                         |
       |   the race window may be unclosable —          reassured                         |
       |   show me a packet capture"                    incrementally                     |
       |                                       _____________________________             |
       |                                      /  spike proves one flow;     \             |
       |        SPIKE (Slice 00)             /   ss -K shows kTLS; no        \   wire      |
       v____________________________________/    cleartext before install;   \__capture___|
        Slice 00          Slice 01-02          a wrong SVID fails closed       Slice 03-05
       (riskiest          (sockops detect +    (Slice 04 race-window probe,    (the agent is out
        assumption        agent handshake,     Slice 03 fail-closed)           of the path; tcpdump
        cheapest to        held SVID presented)                                shows TLS 1.3)
        learn)
```

No jarring transitions: confidence builds progressively (each slice's observable
is a small win Sam verifies himself with `tcpdump` / `ss -K`); the error/race
paths guide to resolution (fail-closed + reset-and-retry), not added anxiety; the
spike de-risks the peak-tension assumption (unshipped in-band kTLS) FIRST.

### Honest observable surfaces (the "what Sam sees")

```
+-- Wire capture (TEST tier — the headline observable) ----------------------+
| $ tcpdump -i overdrive-veth0 -X 'tcp port 8443'                            |
|   ... IP a.b.c.d.54321 > w.x.y.z.8443: ...                                 |
|     0x0000:  ... 1703 0304 00a7  ...   <-- TLS 1.3 App Data (0x17 03 03)   |
|   NO cleartext "GET /payments HTTP/1.1" anywhere in the capture            |
+----------------------------------------------------------------------------+

+-- Socket diag (kTLS ULP installed) ----------------------------------------+
| $ ss -K  (or: ss -ti, ulp tls)                                            |
|   tcp ESTAB ... ulp tls  <-- the kTLS Upper Layer Protocol is installed    |
+----------------------------------------------------------------------------+

+-- Fail-closed negative test (wrong/absent SVID) ---------------------------+
| flow with absent/wrong SVID -> NO TLS 1.3 App Data on the wire AND no      |
| cleartext -> connection fails closed (rustls aborts / agent refuses)       |
+----------------------------------------------------------------------------+

+-- Race-window probe (no cleartext before kTLS install) --------------------+
| write("PLAINTEXT_PROBE") immediately on connect() return, kTLS install     |
| deliberately delayed 500ms -> tcpdump shows EITHER TLS 1.3 records OR zero  |
| app bytes -> NEVER the string "PLAINTEXT_PROBE" on the wire; drop-counter   |
| signals whether the window fired                                           |
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
| **workload socket fd** (the object kTLS installs on) | the workload's own socket — process: `pidfd_getfd` from the sockops-triggered event; WASM: in-process host fd | `setsockopt(TCP_ULP/TLS_TX/TLS_RX)`; the kTLS context (`icsk_ulp_data`) | **HIGH** — fd OWNERSHIP is load-bearing for restart survival (kTLS state is socket-owned; survives an agent restart iff the workload owns the fd). A DESIGN decision the spike's handoff targets (workload-owns-fd). |
| **kTLS `crypto_info`** (negotiated TLS 1.3 keys / IV / record seq) | the rustls handshake's extracted secrets — **auth-session == data-session** | `setsockopt(TLS_TX/TLS_RX)` on the workload socket; observable as TLS 1.3 records (tcpdump) + the kTLS ULP (`ss -K`) | **HIGH** — the SAME session that authenticated must carry the data; a mismatch breaks the "auth-session is the data-session" property the whole design rests on. |
| **`AllocationId` / `SpiffeId`** (which workload's identity to present) | the `SpiffeId` newtype + the allocation lifecycle (`spiffe://overdrive.local/job/<name>/alloc/<id>`, J-SEC-001 derivation) | the `IdentityRead` lookup key; the SAN the peer sees in the presented SVID | **MEDIUM** — the lookup key must match the held-map key (#35's `AllocationId`); a mismatch reads `None` and (correctly) fails the handshake closed. |

**Integration checkpoints** (validated before DESIGN handoff):

1. The agent reads SVID + bundle **only** via the `IdentityRead` port (no #26-local
   issuance, no #26-local cache) — single source of truth preserved across #35/#26.
2. The kTLS keys installed are the **rustls handshake's** extracted secrets
   (auth-session == data-session) — not a separately-negotiated session.
3. The sockops gate is inserted **synchronously at ESTABLISHED** (before
   connect()/accept() returns) — the race-window fail-closed property.

---

## Wave: DISCUSS / [REF] Story Map

**Persona**: Sam (platform/security engineer) · **Goal**: host-socket workloads
carry TLS 1.3 on the wire with their own SVID, in-kernel, agent out of the path —
provable by a packet capture, with no cleartext leak and fail-closed handshakes.

### Backbone (platform activities, left → right)

| A. Detect the connection in the kernel | B. Handshake on the workload's behalf | C. Install kTLS, agent exits | D. Prove & guard on the wire |
|---|---|---|---|
| sockops ACTIVE/PASSIVE_ESTABLISHED detect (S00 spike, S01) | rustls TLS 1.3 handshake (S00 spike, S02) | kTLS install on the workload socket (S00 spike, S03) | tcpdump shows TLS 1.3 records (S00 spike, S03) |
| acquire the workload socket fd (S00 spike, S01) | present the held SVID via IdentityRead (S02) | agent EXITS the data path (S03) | fail-closed on absent/wrong SVID (S04) |
| gate the race window (S04) | verify peer vs trust bundle (S02) | (restart: new conns re-handshake) (S05) | no cleartext before kTLS install (S04) · restart re-handshake; in-flight survival spike-gated (S05) |

### Walking Skeleton (thinnest end-to-end, all activities — SPIKE-FIRST)

**Slice 00** (the spike): for ONE process→process flow on the 6.18 kernel, prove
the full A→B→C→D handoff (sockops detect → fd acquire → rustls handshake presenting
the held SVID → kTLS install → agent exits → tcpdump shows TLS 1.3 records),
race-free. The minimum cut that touches every backbone activity, on one flow.
Falsification → Cilium fallback.

### Release 1 (the productionised happy path past the spike)

**Slice 01** (sockops detection + fd acquisition, productionised),
**Slice 02** (agent rustls handshake presenting the held SVID via IdentityRead),
**Slice 03** (kTLS install + agent exits + wire-capture acceptance). Targets the
North-Star observable: the wire carries TLS 1.3 records for host-socket flows.

### Release 2 (the guards: fail-closed + race-window + durability)

**Slice 04** (handshake fail-closed on absent/wrong SVID + no-cleartext-before-kTLS
race-window probe), **Slice 05** (CP/agent-restart survival of in-flight kTLS +
new-connection re-handshake; the WASM-in-process variant). Targets the guardrail
observables (fail-closed, no-cleartext, restart-survival).

### Slice list (each = one ≤1-day vertical cut; Slice 00 = ~2-day spike)

| Slice | Stories | Learning hypothesis (disproves X if it fails) | Brief |
|---|---|---|---|
| 00 (**walking skeleton — SPIKE**) | US-MTLS-00 | in-band sidecarless kTLS is achievable race-free on the 6.18 kernel for ONE process flow: sockops→pidfd_getfd→rustls(present held SVID)→kTLS install→agent exits→tcpdump shows TLS 1.3, no cleartext before install. **FAIL → Cilium out-of-band-auth + separate-encryption fallback** | `slices/slice-00-spike-inband-ktls-one-flow.md` |
| 01 | US-MTLS-01 | sockops detects a host-socket ESTABLISHED transition synchronously (before the workload can write) and the agent acquires the workload's socket fd (process: pidfd_getfd) — productionised from the spike | `slices/slice-01-sockops-detect-and-acquire-fd.md` |
| 02 | US-MTLS-02 | the agent performs the rustls TLS 1.3 handshake presenting the HELD SVID (read via IdentityRead) and verifying the peer vs the trust bundle — the workload holds nothing / is identity-unaware | `slices/slice-02-agent-handshake-present-held-svid.md` |
| 03 | US-MTLS-03 | the negotiated session keys install into kTLS on the workload's socket (auth-session == data-session), the agent EXITS the data path, and tcpdump shows TLS 1.3 records (ss -K shows the ULP) | `slices/slice-03-ktls-install-agent-exits-wire-capture.md` |
| 04 | US-MTLS-04 | a handshake against an absent/wrong SVID fails closed (no cleartext) AND no cleartext byte egresses before kTLS install (race-window probe, fail-closed for confidentiality) | `slices/slice-04-fail-closed-and-race-window.md` |
| 05 | US-MTLS-05 | after a node-agent restart, new connections re-handshake and re-install kTLS (the unconditional Phase-2 promise) and the WASM-in-process variant works identically; in-flight kTLS survival across the restart is spike/DESIGN-gated (holds IFF workload-owns-fd + bpffs-pinned link/maps — not an unconditional Phase-2 AC) | `slices/slice-05-restart-survival-and-wasm-variant.md` |

---

## Wave: DISCUSS / [REF] Priority Rationale

Execution order = **riskiest assumption first** (the spike), then the **happy-path
dependency chain** (detect → handshake → install/prove), then the **guards**
(fail-closed/race-window, then durability). Per Value × Urgency / Effort with
Walking-Skeleton > Riskiest-Assumption > Highest-Value tie-breaking.

| Order | Slice | Why this position |
|---|---|---|
| 1 | S00 (spike) | **Riskiest assumption + walking skeleton.** Does in-band sidecarless kTLS work race-free on our kernel AT ALL? If the handoff cannot be made race-free for one process flow, the entire in-band mechanism is wrong and the design must adopt the Cilium fallback. Cheapest place to learn it; everything downstream is moot if it fails. Urgency 5 (derisks the fatal assumption). |
| 2 | S01 | Depends on S00 (productionises the sockops-detect + fd-acquire the spike proved). First step of the happy-path chain; the gate insertion point the race-window guard (S04) builds on. |
| 3 | S02 | Depends on S01 (needs the acquired fd). The handshake that presents the HELD SVID via `IdentityRead` — the integration seam with #35. Moderate uncertainty (rustls + reading a shipped port). |
| 4 | S03 | Depends on S02 (needs the handshake's extracted secrets). The kTLS install + agent-exit + the **North-Star wire-capture observable** (TLS 1.3 on the wire). Highest value (it IS the headline). |
| 5 | S04 | Depends on S03 (needs the install path to gate). The two guardrails — fail-closed on wrong/absent SVID, no-cleartext-before-install. High value (the security invariants), moderate effort (the gate + negative tests). |
| 6 | S05 | Depends on S03 (needs in-flight kTLS sessions to survive). Lowest mechanism uncertainty (additive durability + the WASM variant on a proven path) but carries the fd-ownership-for-restart caveat. Resolves the restart-survival error path last so the durability surface is stable. |

Dependency chain: **S00 → S01 → S02 → S03 → {S04, S05}** (S04 and S05 both depend
on S03 but not on each other — parallelisable after S03). The order puts
fail-closed/race-window (S04) before restart-survival (S05) because the
confidentiality guardrails are higher-value security invariants than durability.

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
- **Workload holds NOTHING; kernel does mTLS (whitepaper §7, CLAUDE.md § "Workload
  identity model").** The workload is identity-unaware — no cert, no key. It opens
  ordinary sockets; the kernel-side sockops + kTLS terminate/originate TLS
  transparently using material the platform (the node agent) supplies. There is no
  SPIRE-agent-style workload-held copy. Any design that puts SVID material inside
  the workload is a violation.
- **Auth-session == data-session.** The SAME TLS 1.3 session that authenticated
  the workload (the rustls handshake) carries the data (its extracted secrets
  install into kTLS). This is the property that distinguishes Overdrive's target
  from Cilium's out-of-band-auth model (where auth and encryption are separate
  sessions). The spike validates it; if it cannot hold, the Cilium fallback
  (separate sessions) is the documented alternative.
- **Fail-closed for confidentiality (the race window).** A gate armed in kernel
  context before connect()/accept() returns to the workload keeps NO cleartext byte
  on the wire before kTLS is armed. The gate MECHANISM (sk_msg DROP-until-armed vs
  sockmap redirect vs an out-of-tree write-block) is a DESIGN choice the spike
  informs — DISCUSS pins the observable, not the mechanism. The residual is a
  DATA-LOSS window (writes during the 10–100ms agent turnaround are SK_DROP'd, not
  buffered — sk_msg has no lossless HOLD), closed by connection RESET if drops > 0;
  request-first protocols (HTTP, gRPC, PostgreSQL) retry. **Server-speaks-first
  protocols (SMTP/FTP/SSH)** have an irreducible data-loss window without a
  write-block and are a **DESIGN scope call** (see Open Questions).
- **Mechanism is NOT pinned (DISCUSS pins WHAT, not HOW).** In-band kTLS
  (Architecture A) vs sockmap proxy (Architecture C) vs Cilium out-of-band-auth +
  separate encryption — the choice is DESIGN's, informed by the Slice-00 spike. The
  6.18 pin (ADR-0068) legitimises an out-of-tree write-block patch (lossless
  backpressure) as an appliance-OS option, but that too is a DESIGN call. DISCUSS
  does NOT pin kTLS struct shapes, the exact sockops attach mechanism, or the
  write-block decision.
- **Pinned 6.18 LTS kernel (ADR-0068) — no version anxiety.** In-kernel TLS 1.3
  TX+RX and `CONFIG_NET_HANDSHAKE` are guaranteed; the kernel is a controlled
  constant, not a design axis. The platform tests exactly the one kernel it ships
  (+ bpf-next soft-fail).
- **kTLS state is socket-owned (restart survival).** kTLS crypto lives on the
  socket (`icsk_ulp_data`), freed only on socket close — so an in-flight session
  survives an agent restart IFF the WORKLOAD owns the socket fd, and the sockops
  bpf_link + maps are bpffs-pinned (`pinning = ByName`, `/sys/fs/bpf/overdrive/`).
  fd-ownership is a DESIGN decision the spike's handoff targets. A FULL NODE REBOOT
  wipes everything kernel-owned (bpffs pins do not survive a reboot) → every
  connection re-handshakes from scratch.
- **Host-socket ONLY (the scope boundary).** Process (exec driver) + WASM
  (wasmtime in-process host sockets) — TCP terminates in the HOST kernel.
  Guest-stack workloads (microVM/unikernel, TCP in the GUEST kernel) are **#222**
  (host L4 tap proxy, a SEPARATE feature). Do NOT pull #222 in.
- **In-place rekey deferred to #229; rotation to #40.** v1 SVID rotation on a
  long-lived connection = TEARDOWN + RECONNECT (kernel-side KeyUpdate IS present at
  v6.18, but the userspace rustls/ktls bridge is not — rustls/ktls#59 / #62, #229).
  A TRACKED DEPENDENCY, not an open design risk. The cert-rotation workflow is #40.
- **No operator CLI verb; #26 is a FOUNDATION feature (D1).** Encryption is
  automatic and undisableable (vision principle 2) — there is no `overdrive`
  subcommand to "encrypt this workload". The HONEST observable surfaces are
  TEST-tier: `tcpdump` showing TLS 1.3 records, `ss -K` showing the kTLS ULP, a
  fail-closed negative test, a race-window probe. Per CLAUDE.md the workload verb
  is `overdrive deploy <SPEC>`, never `job submit`. Do NOT invent a CLI verb.

---

## Wave: DISCUSS / [REF] User Stories

Every story traces to `job_id: J-SEC-003`. Every non-`@infrastructure` story has an
Elevator Pitch whose "After" references a real, executable verification entry point
(the wire capture / `ss -K` / the test runner — the honest user-invocable
observable for a security primitive with no operator verb). ACs are embedded and
derived from the UAT scenarios.

> **Elevator-Pitch "After" caveat (SAME as built-in-ca / workload-identity-manager
> — a security primitive with NO operator CLI verb)**: encryption is automatic and
> undisableable; there is no `overdrive` subcommand to "encrypt this workload".
> Each pitch's "After" references a real, executable verification entry point — a
> `tcpdump` wire capture showing TLS 1.3 records (or `ss -K` showing the kTLS ULP,
> or a fail-closed negative test) — which is the honest user-invocable observable
> output for this feature, not an invented subcommand. The DECISION enabled is
> Sam's trust decision (the genuine J-SEC-003 connection: is the wire actually
> encrypted with the workload's own SVID, in-kernel, agent out of the path, no
> cleartext leak, fail-closed?).
>
> **Foundation-feature exception to the strict elevator-pitch gate (recorded
> explicitly, NOT a silent pass — mirroring built-in-ca / #35)**: the strict nWave
> gate requires a real user-invocable entry point. #26 does not strictly satisfy
> that on its own — every Phase-2 proof is TEST-tier (a `tcpdump` capture / `ss -K`
> / a race-window probe in a gated `integration-tests` Lima run), because the
> feature is a foundation security primitive with no operator verb and encryption
> is undisableable by design. The gate is met by a **deliberate, documented
> foundation-feature exception mirroring built-in-ca and #35, NOT by a live
> operator surface and NOT by an invented CLI verb** (inventing a verb to dodge the
> gate is the dishonest move; recording the exception is the honest one). Recorded
> here, in `wave-decisions.md` (D1), and in the DoR validation (the
> elevator-pitch / slice-composition items note it with this evidence pointer).

### US-MTLS-00 — SPIKE: prove in-band sidecarless kTLS race-free on the pinned kernel (or fall back) `@spike`

**Type**: Spike (time-boxed Tier-3 investigation; the walking skeleton). **Fixed
duration**: ~2 days. **Clear learning objective**: validate (or disprove) the two
load-bearing hypotheses before any design locks.

**Problem**: Sam will not let the design lock on in-band sidecarless kTLS — a
mechanism **no production mesh has shipped** — without proof it works race-free on
Overdrive's actual pinned kernel. Cilium does out-of-band auth + WireGuard/IPsec;
Istio ztunnel keeps a userspace proxy in the data path. The plaintext race window
(sk_msg has no lossless HOLD) might be unclosable. He needs a Tier-3 spike on the
6.18 Lima/LVH kernel that either proves the full handoff for ONE process→process
flow, or honestly falls back to the documented Cilium model.

**Who**: Platform/security engineer | de-risking the riskiest mechanism assumption
| wants empirical proof on the real kernel before betting the design on an
unshipped mechanism.

**Solution**: A Tier-3 spike harness on the 6.18 kernel that drives ONE
process→process host-socket flow through the full handoff — sockops
`ACTIVE_ESTABLISHED` detect → `pidfd_getfd` the workload socket → rustls TLS 1.3
handshake presenting the held SVID (read via `IdentityRead`) and verifying the peer
→ kTLS install (`setsockopt TLS_TX/TLS_RX`) → agent exits → `tcpdump` shows TLS 1.3
records — with a deliberately delayed kTLS install widening the race window to test
fail-closed (no cleartext before install). Time-boxed; the deliverable is a
PASS/FAIL verdict + the wire capture, NOT production code.

#### Elevator Pitch

- **Before**: in-band sidecarless kTLS is a mechanism no one has shipped; betting
  the #26 design on it without proof on the real kernel risks a wrong contract that
  propagates through DESIGN and DELIVER.
- **After**: a Tier-3 spike on the 6.18 Lima kernel produces a `tcpdump` capture
  for one process→process flow showing TLS 1.3 Application Data records (with `ss
  -K` showing the kTLS ULP installed and a race-window probe showing zero cleartext
  before install) — OR a documented FAIL verdict that triggers the Cilium
  out-of-band-auth + separate-encryption fallback.
- **Decision enabled**: Sam (and the DESIGN wave) decides whether to proceed with
  in-band kTLS (Architecture A) or adopt the documented fallback — the riskiest
  mechanism question settled empirically, not assumed.

#### Domain Examples

1. **Happy path (spike PASS)** — On the 6.18 Lima kernel, process `client` (alloc
   `a1b2c3`, SVID `spiffe://overdrive.local/job/web/alloc/a1b2c3`) connects to
   process `server` (alloc `d4e5f6`, SVID `.../job/api/alloc/d4e5f6`). sockops
   fires `ACTIVE_ESTABLISHED`; the agent `pidfd_getfd`s the client socket, runs the
   rustls TLS 1.3 handshake presenting `a1b2c3`'s held SVID (read via
   `IdentityRead`) and verifying `d4e5f6` against the trust bundle, installs kTLS,
   and exits. `tcpdump -i overdrive-veth0` shows TLS 1.3 App Data records (0x17);
   `ss -K` shows the kTLS ULP on both sockets.
2. **Race-window probe (fail-closed proof)** — `client` calls `write("PLAINTEXT_PROBE")`
   immediately on `connect()` return; the spike harness delays the kTLS install by
   500ms. `tcpdump` shows EITHER TLS 1.3 records OR zero application bytes —
   NEVER the string `PLAINTEXT_PROBE` on the wire. The sk_msg drop-counter records
   whether the window fired.
3. **Falsification (spike FAIL → fallback)** — If the handoff cannot be made
   race-free (e.g. cleartext leaks before the gate is active, or the kTLS install
   cannot be driven from the acquired fd on this kernel), the spike returns a FAIL
   verdict naming the failure mode, and the design adopts the Cilium model
   (out-of-band auth + separate encryption, or a userspace proxy à la
   Architecture C). This is a SUCCESSFUL spike outcome (it answered the question),
   not a failure.

#### UAT Scenarios (BDD)

##### Scenario: One host-socket flow carries TLS 1.3 on the wire after the in-kernel handoff
Given two host-socket workloads on the pinned 6.18 kernel, each with a held SVID
When the platform performs the sockops-detect to rustls-handshake to kTLS-install handoff for one connection between them
Then a wire capture between them shows TLS 1.3 Application Data records and no cleartext
And the kTLS ULP is installed on each workload's socket

##### Scenario: No cleartext escapes before encryption is armed, even when install is delayed
Given a host-socket workload that writes immediately after its connection is established
And the kTLS installation is deliberately delayed
When the platform captures the wire during the delay
Then no cleartext application bytes appear on the wire before kTLS is installed

##### Scenario: The spike returns an honest verdict that decides the mechanism
Given the spike has run the full handoff on the pinned kernel
When the handoff cannot be made race-free for one flow
Then the spike reports a FAIL verdict naming the failure mode
And the documented fallback (out-of-band auth + separate encryption) is selected for the design

#### Acceptance Criteria

- [ ] On the 6.18 Lima/LVH kernel, a Tier-3 spike drives ONE process→process
  host-socket flow through `sockops ACTIVE_ESTABLISHED → pidfd_getfd → rustls TLS
  1.3 handshake presenting the held SVID (read via `IdentityRead`) and verifying
  the peer → kTLS install (`setsockopt TLS_TX/TLS_RX`) → agent exits`.
- [ ] A `tcpdump` capture on the veth shows TLS 1.3 Application Data records
  (content type 0x17) and NO cleartext payload; `ss -K` shows the kTLS ULP
  installed on the socket.
- [ ] A race-window probe (write immediately on connect() return + deliberately
  delayed kTLS install) shows NO cleartext byte on the wire before install (a
  drop-counter signals whether the window fired).
- [ ] The spike produces an explicit PASS/FAIL verdict; on FAIL it names the
  failure mode and selects the documented Cilium fallback (out-of-band auth +
  separate encryption / userspace proxy) — a successful spike outcome either way.
- [ ] The verdict + the wire capture are recorded (the spike's deliverable is the
  evidence + the verdict, not production code).

#### Technical Notes

- Time-boxed to ~2 days; the deliverable is evidence + a verdict, not a shippable
  mechanism. If the spike PASSES, Slices 01–03 productionise the proven handoff; if
  it FAILS, the DESIGN wave adopts the fallback and the downstream slices re-shape.
- The fd-ownership choice (workload owns the socket fd, per the CP-restart-survival
  research) is targeted by the spike's handoff so Slice 05's restart-survival
  observable is reachable.
- This is the ONE place a spike is the right tool (the mechanism is genuinely
  unshipped + has no Tier-2 backstop — `BPF_PROG_TEST_RUN` is unavailable for the
  relevant socket-context hooks; it can only be settled at Tier 3).

---

### US-MTLS-01 — sockops detects the host-socket connection and the agent acquires its socket

**Problem**: For the platform to encrypt a host-socket workload's traffic, it must
first NOTICE the connection in the kernel — synchronously, before the
identity-unaware workload can write a cleartext byte — and get a handle to the
workload's own socket. Today there is no detection path: a host-socket workload's
connections are invisible to the platform's identity layer.

**Who**: Platform/security engineer | wiring the kernel detection path | wants
host-socket connections detected in-kernel before the workload writes, with the
workload's socket acquired for the agent to drive TLS on.

**Solution**: A kernel sockops program detects the `ACTIVE_ESTABLISHED` /
`PASSIVE_ESTABLISHED` transition synchronously (in kernel context, before
connect()/accept() returns to the workload), and the node agent acquires the
workload's socket fd (process: `pidfd_getfd` from the sockops-triggered event).
This is the productionised detection+acquire step the Slice-00 spike proved.

#### Elevator Pitch

- **Before**: a host-socket workload's TCP connections are invisible to the
  platform's identity layer — nothing notices them, so nothing can encrypt them.
- **After**: a host-socket connection is detected in the kernel the moment it
  reaches ESTABLISHED (before the workload can write), and the agent holds the
  workload's socket — observable in a Tier-3 test as the sockops program firing on
  the connection and the agent acquiring the fd (the connection is now under the
  platform's control for the handshake to follow).
- **Decision enabled**: Sam decides the platform reliably notices host-socket
  connections in-kernel before any cleartext can escape — or rejects a detection
  path that fires too late (after the workload could write) or misses connections.

#### Domain Examples

1. **Active side (process client)** — process `web` (alloc `a1b2c3`) calls
   `connect()` to `api`. The sockops program fires `ACTIVE_ESTABLISHED`
   synchronously during the ESTABLISHED transition (before `connect()` returns);
   the agent `pidfd_getfd`s `web`'s socket. The detection precedes any `write()`.
2. **Passive side (process server)** — process `api` (alloc `d4e5f6`) has a
   listening socket; the final ACK of the 3WHS arrives. The sockops program fires
   `PASSIVE_ESTABLISHED` before `accept()` returns the new socket to `api`; the
   agent acquires the accepted socket. For request-first protocols the server reads
   first, so detection precedes the server's first write naturally.
3. **Non-host-socket connection (correctly ignored)** — a guest-stack workload
   (microVM) opens a connection; its TCP terminates in the GUEST kernel, so there
   is no host `struct sock` and the host sockops program never sees it — correctly
   out of #26's scope (that is #222's path). The detection path does not fire and
   does not error.

#### UAT Scenarios (BDD)

##### Scenario: A host-socket connection is detected before the workload can write
Given a host-socket workload that opens a TCP connection
When the connection reaches the established state in the kernel
Then the platform detects the connection synchronously before the workload's first write
And the platform holds a handle to the workload's own socket

##### Scenario: A guest-stack connection is correctly not detected by the host path
Given a guest-stack workload (microVM or unikernel) whose TCP terminates in the guest kernel
When that workload opens a connection
Then the host detection path does not fire for it and does not error

#### Acceptance Criteria

- [ ] A kernel sockops program fires on a host-socket workload's `ACTIVE_ESTABLISHED` / `PASSIVE_ESTABLISHED` transition, synchronously in kernel context (before connect()/accept() returns to the workload) — observable in a Tier-3 test (the program runs; the connection is gated before the workload writes).
- [ ] The node agent acquires the workload's own socket fd (process: `pidfd_getfd`) for the detected connection.
- [ ] A guest-stack workload's connection (TCP in the guest kernel) does NOT trigger the host detection path and does not error (correctly out of scope — #222).
- [ ] The sockops program + its maps/link are bpffs-pinned (`pinning = ByName`, `/sys/fs/bpf/overdrive/`) — the prerequisite for the restart-survival observable (Slice 05).

#### Technical Notes

- The exact sockops attach mechanism (legacy `BPF_PROG_ATTACH` vs `bpf_link`
  pinned to bpffs) and the fd-acquisition path are DESIGN's to pin — both reach
  restart survival (the link must be pinned either way). Productionises the
  Slice-00 spike's detect+acquire step.
- WASM (in-process host fd) detection is the Slice-05 variant; US-MTLS-01's core is
  the process path.

---

### US-MTLS-02 — the agent performs the TLS 1.3 handshake presenting the held SVID

**Problem**: Once a host-socket connection is detected, the platform must perform
the TLS 1.3 handshake **on the workload's behalf**, presenting the **workload's own
held SVID** (not the node's, not the agent's) and verifying the peer — because the
workload is identity-unaware and holds nothing. There is no handshake path today,
and the credential it must present lives behind #35's `IdentityRead` port.

**Who**: Platform/security engineer | wiring the agent's mutual-TLS handshake |
wants the agent to present the WORKLOAD's held SVID (read via IdentityRead) and
verify the peer against the trust bundle, with the workload holding nothing.

**Solution**: The node agent performs a rustls TLS 1.3 handshake on the workload's
acquired socket, presenting the held `SvidMaterial` it reads via
`IdentityRead::svid_for(&AllocationId)` (#35; leaf key per ADR-0063 D9) and
verifying the peer against `IdentityRead::current_bundle()`. #26 is a READER of the
held set — it never mints, re-issues, or caches its own copy.

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

- [ ] The node agent performs a rustls TLS 1.3 handshake on the detected workload's socket, presenting the held `SvidMaterial` read via `IdentityRead::svid_for(&AllocationId)` (#35) — #26 reads, never mints/caches.
- [ ] The agent verifies the peer against `IdentityRead::current_bundle()` (the hydrated bundle behind #35's port); a peer that does not chain to it aborts the handshake (the fail-closed proof is Slice 04).
- [ ] The presented leaf chains to the root and its SAN is the workload's SPIFFE URI (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via `openssl verify` / the captured handshake at the TEST tier.
- [ ] The workload process holds no cert and no key (the leaf key stays with the agent, read via the port) — the workload is identity-unaware.

#### Technical Notes

- The exact rustls config shape (ClientConfig/ServerConfig, the
  `IdentityRead`-backed cert resolver) is DESIGN's to pin. #26 takes the
  `IdentityRead` port as a required constructor parameter (port-trait discipline,
  `.claude/rules/development.md`).
- Server-speaks-first vs request-first protocol handling interacts with the race
  window (Slice 04) and is a DESIGN scope call (see Open Questions).

---

### US-MTLS-03 — kTLS install on the workload's socket, agent exits, wire carries TLS 1.3

**Problem**: A completed handshake is not enough — for the encryption to be
**in-kernel** and the **agent out of the data path** (the property that
distinguishes Overdrive from ztunnel), the negotiated session keys must install
into **kTLS on the workload's own socket** (auth-session == data-session) and the
agent must LEAVE. Then a packet capture must prove the wire actually carries TLS
1.3 records.

**Who**: Platform/security engineer | wiring the kTLS install + agent-exit | wants
the negotiated session installed into the kernel on the workload's socket, the
agent out of the steady-state path, and TLS 1.3 records provable on the wire.

**Solution**: After the handshake (US-MTLS-02), the agent installs the rustls
handshake's extracted secrets into kTLS on the workload's socket (`setsockopt
TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the SAME session that authenticated — and exits
the data path. The kernel then does TLS record framing + symmetric crypto
autonomously; the agent is out of the steady-state byte path.

#### Elevator Pitch

- **Before**: even with a completed handshake, there is no in-kernel encryption on
  the workload's socket and no proof the wire is encrypted — and a userspace proxy
  staying in the path (the ztunnel shape) would not be "agent out of the data path".
- **After**: the negotiated session installs into kTLS on the workload's own socket
  and the agent exits — observable as `ss -K` showing the kTLS ULP installed and a
  `tcpdump` capture on the veth showing TLS 1.3 Application Data records (content
  type 0x17), never cleartext, between two host-socket workloads.
- **Decision enabled**: Sam decides the encryption is genuinely in-kernel with the
  agent out of the path (the auth-session is the data-session) — or rejects a
  design where a userspace proxy quietly handles every byte for the whole
  connection.

#### Domain Examples

1. **Happy path (the North-Star observable)** — after `a1b2c3`↔`d4e5f6`'s
   handshake, the agent installs the extracted secrets into kTLS on both sockets
   and exits. `ss -K` shows `ulp tls` on each socket; `tcpdump -i overdrive-veth0`
   shows TLS 1.3 App Data records (0x17 03 03); a `GET /payments HTTP/1.1` the
   workload sent never appears in cleartext in the capture.
2. **Agent out of the steady-state path** — after install, the agent's process is
   absent from the byte path: the kernel encrypts/decrypts each record; the agent
   does no per-byte work (strictly cheaper than ztunnel's userspace per-byte copy).
3. **Auth-session == data-session** — the kTLS `crypto_info` installed is the
   rustls handshake's own extracted secrets (same keys/IV/record-seq) — not a
   separately negotiated session; the data records continue the authenticated
   session's record sequence.

#### UAT Scenarios (BDD)

##### Scenario: The wire carries TLS 1.3 records between two host-socket workloads
Given two host-socket workloads whose handshake has completed
When the platform installs the negotiated session into the kernel and steps out of the data path
Then a wire capture between them shows TLS 1.3 Application Data records and no cleartext
And the kTLS upper-layer protocol is installed on each workload's socket

##### Scenario: Encryption is in-kernel with the platform out of the steady-state data path
Given a host-socket connection whose encryption is armed in the kernel
When the workloads exchange application bytes
Then the kernel performs the record framing and encryption
And the platform agent is not in the steady-state byte path

#### Acceptance Criteria

- [ ] The agent installs the rustls handshake's extracted secrets into kTLS on the workload's own socket (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's secrets (auth-session == data-session), not a separately negotiated session.
- [ ] After install the agent EXITS the data path — the kernel does the steady-state record framing + crypto; the agent does no per-byte work.
- [ ] `ss -K` shows the kTLS ULP installed on each workload's socket (TEST tier, via Lima).
- [ ] A `tcpdump` capture on the veth between two host-socket workloads shows TLS 1.3 Application Data records (content type 0x17) and NEVER the cleartext payload (TEST tier, gated `integration-tests`, via Lima).

#### Technical Notes

- The exact kTLS `crypto_info` struct construction (mapping rustls extracted
  secrets → `TLS_TX/TLS_RX`) and the record-sequence handling are DESIGN's to pin —
  DISCUSS pins the observable (TLS 1.3 on the wire), not the struct shape.
- This is the North-Star observable slice (the wire carries TLS 1.3); it is the
  headline value and is prioritised accordingly (P4 in the chain, highest value).

---

### US-MTLS-04 — fail-closed on absent/wrong SVID; no cleartext before kTLS install

**Problem**: The encryption guarantee is only real if it CANNOT be bypassed: a
handshake against an absent or wrong SVID must **fail closed** (no plaintext
fallback), and no cleartext byte may **leak before kTLS is armed** (the plaintext
race window). Both are the security invariants a reviewer pushes hardest on.

**Who**: Platform/security engineer | threat-modelling the bypass paths | wants the
handshake to fail closed on a wrong/absent SVID and zero cleartext to egress before
kTLS install.

**Solution**: The handshake fails closed when `IdentityRead` returns absent for the
allocation (the agent refuses rather than presenting a stale credential) or when
the peer does not chain to the trust bundle (rustls aborts). A gate armed before
connect()/accept() returns keeps no cleartext on the wire before kTLS is armed (the
gate mechanism — sk_msg DROP-until-armed vs sockmap redirect vs an out-of-tree
write-block — is a DESIGN choice the spike informs, not pinned here); the residual
data-loss window is closed by connection reset if drops > 0.

#### Elevator Pitch

- **Before**: an absent/wrong SVID could fall back to plaintext, and a workload
  write during the agent's handshake turnaround could leak cleartext — the
  encryption guarantee would be bypassable.
- **After**: a flow with an absent/wrong SVID produces NO TLS Application Data and
  NO cleartext on the wire (fail-closed), and a race-window probe (write immediately
  on connect() return + deliberately delayed kTLS install) shows zero cleartext
  bytes before install — observable as a `tcpdump` capture that never contains the
  plaintext probe string and a drop-counter signalling the window.
- **Decision enabled**: Sam decides the encryption cannot be bypassed by an
  absent/wrong SVID or a write-before-install race — or rejects the feature if
  either leaks cleartext.

#### Domain Examples

1. **Absent SVID (bounded convergence window, #35's O1)** — alloc `g7h8i9` reached
   Running but its SVID is not yet held (one reconcile tick behind, #35).
   `IdentityRead::svid_for(&g7h8i9)` returns `None`; the agent refuses the
   handshake; the connection does not proceed in cleartext. `tcpdump` shows no TLS
   App Data and no cleartext.
2. **Wrong peer (does not chain to the bundle)** — a peer presents a credential not
   chaining to `current_bundle()`; rustls aborts the handshake. No TLS App Data, no
   cleartext.
3. **Race-window probe** — `web` calls `write("PLAINTEXT_PROBE")` immediately on
   `connect()` return; the kTLS install is delayed 500ms. The gate
   (armed before connect() returned; mechanism per DESIGN/spike) drops the write;
   `tcpdump` shows EITHER TLS 1.3 records OR zero app bytes — never `PLAINTEXT_PROBE`
   on the wire; the drop-counter increments; the connection resets and the app
   retries (request-first).

#### UAT Scenarios (BDD)

##### Scenario: A handshake against an absent or wrong identity fails closed
Given a host-socket connection whose held SVID is absent, or whose peer does not chain to the trust bundle
When the platform attempts the handshake
Then the connection fails closed with no TLS application data and no cleartext on the wire

##### Scenario: No cleartext escapes before encryption is armed
Given a host-socket workload that writes immediately after its connection is established
And the kTLS installation is deliberately delayed
When the wire is captured during the delay
Then no cleartext application bytes appear on the wire before kTLS is installed
And the dropped-write count signals that the race window fired

#### Acceptance Criteria

- [ ] A handshake where `IdentityRead::svid_for` returns absent for the allocation fails closed — the agent refuses rather than presenting a stale credential; no cleartext egresses (TEST tier).
- [ ] A handshake where the peer does not chain to `IdentityRead::current_bundle()` aborts (rustls) — no TLS Application Data, no cleartext on the wire.
- [ ] No cleartext byte reaches the wire before kTLS is armed (the confidentiality fail-closed property): the gate is armed before connect()/accept() returns. The gate MECHANISM (sk_msg DROP-until-armed vs sockmap redirect vs out-of-tree write-block) is a DESIGN choice the spike informs — NOT pinned here.
- [ ] A race-window probe (write immediately on connect() return + deliberately delayed kTLS install) captured by `tcpdump` shows EITHER TLS 1.3 records OR zero application bytes — NEVER the plaintext probe string; a drop-counter signals the window; the connection resets if drops > 0 and a request-first app retries (TEST tier, via Lima).

#### Technical Notes

- The residual data-loss window (sk_msg has no lossless HOLD — `bpf_msg_cork_bytes`
  does not buffer) is closed by connection reset (drops > 0), correct for
  request-first protocols. Server-speaks-first protocols (SMTP/FTP/SSH) need a
  write-block (out-of-tree, legitimised by the 6.18 pin) and are a DESIGN scope
  call (Open Questions).
- The exact gate mechanism (sk_msg DROP-until-armed vs sockmap proxy redirect vs
  the write-block patch) is DESIGN's, informed by the Slice-00 spike — DISCUSS pins
  the observable (no cleartext before install), not the mechanism.

---

### US-MTLS-05 — in-flight kTLS survives an agent restart; new connections re-handshake; WASM variant

**Problem**: A node-agent restart (crash/upgrade) must not silently break in-flight
encrypted connections or leave them unencrypted — and the platform must encrypt
WASM workloads (in-process host sockets) identically to processes. kTLS state is
socket-owned, so survival hinges on fd-ownership; new connections after a restart
must re-handshake cleanly.

**Who**: Platform/security engineer | threat-modelling restart + covering the WASM
workload kind | wants in-flight kTLS sessions to survive an agent restart (when the
workload owns the fd), new connections to re-handshake, and WASM to work like
process.

**Solution**: Because kTLS crypto lives on the socket (`icsk_ulp_data`, freed only
on socket close) and the sockops bpf_link + maps are bpffs-pinned, an in-flight
session survives an agent restart IFF the workload owns the socket fd; new
connections re-run the detect→handshake→install handoff (the held SVID is
re-readable via `IdentityRead`). The WASM path uses the in-process host fd (no
`pidfd_getfd`) but is otherwise identical.

#### Elevator Pitch

- **Before**: a node-agent restart could break in-flight encrypted connections or
  leave them unencrypted, and WASM workloads have no enforcement path.
- **After**: after an agent restart, new connections re-handshake and re-install
  kTLS (the unconditional promise), and a WASM workload's wire carries TLS 1.3
  records identically to a process (`tcpdump`); AND — spike/DESIGN-gated — where the
  workload owns the socket and the sockops link/maps are bpffs-pinned, an in-flight
  kTLS session survives the restart (observable as `ss -K` still showing the ULP
  after the agent is `kill -9`'d and the live exchange continuing). In-flight
  survival is NOT an unconditional Phase-2 acceptance gate; the spike validates the
  composed shape.
- **Decision enabled**: Sam decides a restart does not silently degrade encryption
  and that WASM is covered — or rejects a design where a restart drops kTLS state or
  WASM is unencrypted.

#### Domain Examples

1. **In-flight survival** — `a1b2c3`↔`d4e5f6` have a live kTLS session; the
   workload owns the socket fd; the sockops link + maps are bpffs-pinned. The
   node-agent process is `kill -9`'d. `ss -K` still shows the kTLS ULP on the
   sockets; the workloads continue exchanging TLS 1.3 records (record-sequence
   continuity intact); a fresh agent re-hydrates its management view from the
   bpffs pins without re-handshaking the live connection (CP-restart research §C
   shape).
2. **New connection after restart** — after the agent restart, `web` opens a NEW
   connection to `api`; the detect→handshake→install handoff re-runs (the held SVID
   is re-readable via `IdentityRead`); `tcpdump` shows TLS 1.3 records on the new
   connection.
3. **WASM variant** — a WASM workload `coinflip` (wasmtime, in-process host socket)
   opens a connection; the agent acquires the in-process host fd (no `pidfd_getfd`),
   performs the handshake presenting `coinflip`'s held SVID, installs kTLS, exits;
   `tcpdump` shows TLS 1.3 records identical to the process path.

#### UAT Scenarios (BDD)

##### Scenario: An in-flight encrypted connection survives a node-agent restart
Given two host-socket workloads with a live kernel-encrypted session, where the workload owns the socket and the detection state is pinned
When the node agent process restarts
Then the kernel encryption stays installed on the sockets
And the workloads continue exchanging TLS 1.3 records without re-handshaking the live connection
And a new connection opened after the restart re-handshakes and is encrypted

##### Scenario: A WASM workload's wire is encrypted identically to a process workload
Given a WASM workload (in-process host sockets) that opens a connection
When the platform performs the detect-handshake-install handoff for it
Then a wire capture shows TLS 1.3 records identical to the process path
And the WASM workload holds no certificate or private key

#### Acceptance Criteria

- [ ] (Spike/DESIGN-gated — NOT an unconditional Phase-2 AC) WHERE the spike confirms the composed shape (workload owns the socket fd; sockops bpf_link + maps bpffs-pinned), an in-flight kTLS session survives a node-agent `kill -9` + restart — observable: `ss -K` still shows the kTLS ULP and the workloads continue exchanging TLS 1.3 records with record-sequence continuity (TEST tier, via Lima). If the spike does not confirm it, the documented behaviour is new-connection re-handshake.
- [ ] A fresh agent re-hydrates its management view from the bpffs pins (`PinnedLink::from_pin` / `Map::from_pin`) without re-handshaking the live connection.
- [ ] A NEW connection opened after the restart re-runs the detect→handshake→install handoff (held SVID re-read via `IdentityRead`) and carries TLS 1.3 records.
- [ ] A WASM workload (wasmtime, in-process host fd — no `pidfd_getfd`) is detected, handshaked, and kTLS-installed identically; `tcpdump` shows TLS 1.3 records; the WASM workload holds no cert/key.
- [ ] (Documented, not an AC) a FULL NODE REBOOT wipes all kernel-owned state (bpffs pins do not survive a reboot) → every connection re-handshakes from scratch — stated as expected behaviour, not promised survival.

#### Technical Notes

- fd-ownership (workload owns the socket) is the load-bearing precondition for
  in-flight survival (kTLS state is socket-owned; CP-restart-survival research §B/§C)
  — targeted by the Slice-00 spike's handoff. The restart-survival GUARANTEE is a
  DESIGN/spike observable, not promised beyond "new connections re-handshake" if
  the spike does not confirm the composed behaviour.
- The WASM variant differs only in fd acquisition (in-process host fd vs
  `pidfd_getfd`); the handshake/install/observable are identical.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

By the end of #26, host-socket workloads (process, WASM) carry TLS 1.3 on the wire
with their own SVID, in-kernel, with the platform agent out of the steady-state
data path — provable by a packet capture — with no cleartext egressing before kTLS
install (fail-closed), handshakes failing closed on absent/wrong SVIDs, and the
in-band sidecarless kTLS mechanism validated on the pinned 6.18 kernel by a
spike-first walking skeleton (or the documented Cilium fallback adopted).

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | host-socket flows (process + WASM) | carry TLS 1.3 records on the wire (not cleartext) | 100% of established host-socket flows carry TLS 1.3 Application Data; 0% carry cleartext payload (fail-closed) | 0% (no enforcer exists — the CA mints, the IdentityMgr holds, nothing encrypts the wire) | Tier-3 `tcpdump` on the veth: TLS 1.3 records (0x17) present, cleartext payload absent (Slice 03) | Leading (North Star) |
| K2 | cleartext bytes before kTLS install | egress on the wire during the sockops→kTLS race window | 0 cleartext bytes before kTLS install (fail-closed for confidentiality) | n/a (no enforcer ⇒ no install path today) | Tier-3 race-window probe: write on connect() return + delayed install; `tcpdump` never shows the plaintext probe; drop-counter signals the window (Slice 04) | Guardrail |
| K3 | handshakes against an absent/wrong SVID | fall back to plaintext / proceed in cleartext | 0 plaintext fallbacks — every absent/wrong-SVID handshake fails closed (no TLS App Data, no cleartext) | n/a | Tier-3 negative test: absent SVID (IdentityRead None) and wrong peer (no chain to bundle) both yield no TLS App Data and no cleartext (Slice 04) | Guardrail |
| K4 | the platform agent | stays in the steady-state byte data path (the ztunnel anti-pattern) | agent absent from the steady-state byte path; kTLS ULP installed on the workload's own socket (in-kernel encryption) | n/a | Tier-3 `ss -K` shows the kTLS ULP on the workload socket; the agent does no per-byte work after install (Slice 03) | Guardrail |
| K5 | node-agent restart | breaks new connections (no re-handshake) — and, where the spike confirms the shape, breaks in-flight sessions | UNCONDITIONAL: new connections after a restart re-handshake and re-install kTLS. SPIKE/DESIGN-GATED: 0 in-flight sessions broken IFF the workload owns the fd + bpffs-pinned link/maps (validated by the Slice-00 spike; not an unconditional Phase-2 promise) | n/a (no enforcer ⇒ no held kTLS state today) | Tier-3: `kill -9` the agent → a new connection re-handshakes (unconditional); where fd-owned + pinned, `ss -K` still shows the ULP and the in-flight session continues (Slice 05) | Guardrail |
| K6 | the in-band sidecarless kTLS mechanism | is assumed without empirical proof on the real kernel | the walking-skeleton spike returns an explicit PASS/FAIL verdict on the 6.18 kernel for one flow (PASS = proceed; FAIL = documented Cilium fallback) | unproven (no shipping precedent for in-band sidecarless kTLS) | Tier-3 spike verdict + wire capture for one process→process flow (Slice 00) | Leading (riskiest-assumption gate) |

### Metric hierarchy

- **North Star**: K1 — % of host-socket flows that carry TLS 1.3 records on the
  wire (the single signal that on-the-wire enforcement is operationally true, the
  reason J-SEC-003 exists).
- **Leading indicators**: K6 (the spike validates the mechanism — the gate that
  de-risks K1) and K1 itself.
- **Guardrails (must NOT degrade)**: K2 (no cleartext before install), K3 (handshake
  fail-closed), K4 (agent out of the data path / in-kernel), K5 (restart survival).

### Hypothesis

We believe that detecting host-socket connections in the kernel (sockops),
performing the TLS 1.3 handshake on the workload's behalf (rustls, presenting the
held SVID read via `IdentityRead`), installing the negotiated session into kTLS on
the workload's socket, and exiting the data path will, for host-socket workloads,
make principles 2 + 3 operationally true on the wire. We will know this is true
when **100% of established host-socket flows carry TLS 1.3 records on the wire
(K1)**, with 0 cleartext bytes before install (K2), every absent/wrong-SVID
handshake failing closed (K3), and the agent out of the steady-state path (K4) —
gated by the spike's PASS verdict on the pinned kernel (K6).

### Handoff to DEVOPS (platform-architect)

- **Data collection**: the observables are TEST-tier (`tcpdump` wire captures, `ss
  -K` socket-diag, sk_msg drop-counters) — instrument the Tier-3 harness to capture
  and assert on them (the EDD verification catalogue, `verification/expectations/`,
  graduates the operator-surface/qualitative ones).
- **Baselines**: K1–K5 baseline at 0% / n/a (no enforcer exists today); K6 is the
  spike verdict. Record first-GA measurements in this feature's evolution record
  (NOT in `kpi-contracts.yaml`, which is the docs-platform feature's
  single-feature contract per its scope note).
- **Guardrail thresholds**: K2/K3/K4/K5 are binary (0 cleartext / 0 fallback / agent
  absent / 0 broken) — any violation is a blocking test failure, not a degradation
  warning.

---

## Wave: DISCUSS / [REF] Open Questions (surfaced as blockers, NOT invented answers)

These are surfaced for DESIGN, not resolved in DISCUSS (DISCUSS pins WHAT, not HOW):

1. **Mechanism choice (in-band kTLS [Arch A] vs proxy [Arch C] vs Cilium
   out-of-band-auth fallback).** Un-narrowed (no DIVERGE wave). The Slice-00 spike
   settles the riskiest input; DESIGN owns the choice. **Blocker only if the spike
   is inconclusive** — then DESIGN may want a focused options pass.
2. **Server-speaks-first protocol coverage (SMTP/FTP/SSH).** These have an
   irreducible data-loss window without a write-block (the server's greeting is
   SK_DROP'd before kTLS is armed). The 6.18 pin legitimises an out-of-tree
   write-block patch (lossless backpressure), but adopting it is a DESIGN scope call
   (kernel-maintenance cost vs protocol coverage). DISCUSS does NOT pin it.
3. **fd-ownership for restart survival.** kTLS state is socket-owned; in-flight
   survival needs the WORKLOAD to own the fd (CP-restart-survival research §B/§C).
   The spike's handoff targets workload-owns-fd; DESIGN must pin it before the
   restart-survival AC (Slice 05) is committed-to as a guarantee vs "new connections
   re-handshake".
4. **The exact sockops attach mechanism (legacy `BPF_PROG_ATTACH` vs bpffs-pinned
   `bpf_link`) and the kTLS `crypto_info` struct mapping.** DESIGN's to pin; both
   reach restart survival (link must be pinned either way).

None of these block the DISCUSS handoff — they are DESIGN-wave inputs, with the
spike (Slice 00) the empirical gate for #1/#3.

---

## Wave: DISCUSS / [REF] Definition of Ready (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Each story's Problem is in security-engineer domain language (identity-unaware workloads, host sockets, the held SVID, fail-closed, the plaintext race window) — no "implement sockops". |
| 2 | User/persona with specific characteristics | PASS | Sam Okafor (`sam-platform-security-engineer.yaml`) with a J-SEC-003 lens — threat-models by default, verifies with `tcpdump`/`ss -K`/`openssl verify`, distrusts security that can be turned off. |
| 3 | 3+ domain examples with real data | PASS | Every story has 3 examples with real allocs (`a1b2c3`/`d4e5f6`/`g7h8i9`), real SPIFFE URIs (`spiffe://overdrive.local/job/web/alloc/a1b2c3`), real protocols (HTTP/gRPC), real workloads (`web`/`api`/`coinflip`). No `user123`. |
| 4 | UAT in Given/When/Then (3–7 scenarios) | PASS | Each story has 2–3 business-outcome scenarios (titles describe WHAT the user achieves — "the wire carries TLS 1.3 records", "a handshake against an absent identity fails closed" — never "sockops fires" or kTLS struct names). 13 scenarios across 6 stories. |
| 5 | AC derived from UAT | PASS | Each story's ACs are derived from its scenarios + the four-forces/job-map edge cases; observable + testable (tcpdump/ss -K/drop-counter), not implementation ("use TLS_TX struct X"). |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | 5 ≤1-day slices + 1 ~2-day spike; each story 4–5 ACs, 2–3 scenarios. Scope assessment PASS (zero oversized signals). |
| 7 | Technical notes: constraints/dependencies | PASS | § System Constraints (cross-cutting) + per-story Technical Notes; the mechanism is deliberately NOT pinned (DESIGN's), with the spike as the gate. |
| 8 | Dependencies resolved or tracked | PASS | Consumes shipped #35 (`IdentityRead` + `Arc<IdentityMgr>`) + #28 (`Ca` + leaf key) + ADR-0068 (kernel). Carve-outs tracked by real issue #: #222 (guest-stack), #229 (rekey), #40 (rotation), #36 (multi-node), Phase 5 (revocation). NO DIVERGE risk recorded in `wave-decisions.md`. |
| 9 | Outcome KPIs defined with measurable targets | PASS | K1–K6 with Who/Does-what/By-how-much/Baseline/Measured-by; North Star K1 (100% host-socket flows carry TLS 1.3), guardrails K2–K5 (binary: 0 cleartext / 0 fallback / agent-out / 0-broken), gate K6 (spike verdict). |

**DoR Status: PASSED** (pending peer review). Elevator-Pitch gate: every
non-`@spike` story has a Before/After/Decision-enabled triplet whose "After"
references a real executable observable (`tcpdump`/`ss -K`/negative test); the
foundation-feature exception (no operator verb; TEST-tier observables) is recorded
explicitly above (§ User Stories preamble), in `wave-decisions.md` (D1), and here —
mirroring built-in-ca and #35. Slice-composition gate: no slice is empty
`@infrastructure` — every slice has a genuine wire-capture / `ss -K` / fail-closed
observable.

---

## Wave: DISCUSS / [REF] JTBD Traceability + Handoff

- **Every story traces to `job_id: J-SEC-003`** (the on-the-wire enforcement job,
  `docs/product/jobs.yaml` § J-SEC-003). N:1 mapping — 6 stories → 1 job.
- **Hands off to**: `solution-architect` (DESIGN) — journey (visual + YAML) +
  story map + user-stories + outcome KPIs + the un-pinned mechanism question + the
  spike-first walking skeleton; `acceptance-designer` (DISTILL) — the journey YAML
  (embedded Gherkin) + integration points + outcome KPIs; `platform-architect`
  (DEVOPS) — outcome KPIs (Tier-3 wire-capture / ss-K / drop-counter
  instrumentation).
- **The DESIGN wave owns the mechanism choice** (in-band kTLS vs proxy vs Cilium
  fallback), informed by the Slice-00 spike. DISCUSS pinned the WHAT and the
  acceptance observables, not the HOW (no kTLS struct shapes, no sockops attach
  mechanism, no write-block decision).
