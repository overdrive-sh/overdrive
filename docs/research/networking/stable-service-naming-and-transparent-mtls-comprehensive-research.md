# Research: Stable Service Naming + Fully Transparent mTLS for East-West Workload Traffic (Overdrive)

**Date**: 2026-06-16 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16 (5 internal PRIMARY + 11 external official; avg reputation ~0.99)

> **Product goal.** A workload connects to another workload by a stable id like
> `payments.svc.overdrive.local` and the connection is automatically, fully
> mTLS-encrypted — no key in the workload, no sidecar, no app changes.

> **Method.** Hybrid research — two mandatory evidence streams:
> 1. **Competitor / external evidence** (web, trusted sources) on how mature systems turn a stable name into an encrypted connection.
> 2. **Codebase grounding** (Overdrive repo) — verify PART A facts against the code, then build on them.
>
> The three layers, kept distinct throughout: **Name/address** → **Resolve** → **Enforce**.

---

> **Revision note (2026-06-16, post-review reframing).** Three directions were
> adopted after review and are threaded through §1.5 and §3.3–§3.5:
> 1. **Terminology.** Whitepaper §11's "native" / "non-native caller" labels are
>    from Overdrive's *identity-model* view (SDK-integrated, SPIFFE-ID addressing).
>    From the *application's* view they are backwards — DNS is the most native,
>    universal, zero-integration way an app addresses a service; the SDK/SPIFFE-ID
>    path is the *opt-in* one. This doc now frames the two paths as **transparent
>    (DNS, no app change)** vs **opt-in (SDK / SPIFFE-ID)**.
> 2. **Transparent DNS is the primary path**; the SDK/SPIFFE-ID path drops to an
>    opt-in optimization (client-side LB policy / instance pinning only).
> 3. **Enrollment model + fail-toward-handshake** (ztunnel-shaped: capture all
>    egress from a mesh-enrolled workload, resolve per-connection in the agent,
>    miss → attempt-handshake not cleartext) is the preferred interception model
>    for multi-node scale — tracked in
>    [#236](https://github.com/overdrive-sh/overdrive/issues/236). The eager
>    per-peer `MtlsRedirectHydrator` map (R1 below) is the **single-node v1
>    bridge**, not the long-term shape.

## Executive Summary

Overdrive's east-west "stable name → encrypted connection" problem decomposes
into three layers — **Name** (`payments.svc.overdrive.local`), **Resolve**
(service → healthy backends + each backend's expected identity), and **Enforce**
(wrap the bytes in mTLS). Overdrive has **built the Enforce engine** (the
agent-light kernel-kTLS L4 proxy, #26 / ADR-0069/0070 — spike-proven on a real
7.0 kernel: TLS 1.3 on the wire, 0 plaintext, fail-closed) and the **Resolve data
source** (`service_backends`, `BackendDiscoveryBridge`, `ServiceMapHydrator`).
The critical finding, verified line-by-line in the code: **the engine is dormant
in production.** The outbound `cgroup_connect4_mtls` hook passes a connect through
**in cleartext** on a `MTLS_REDIRECT_DEST` map miss, and the only code that
programs that map is `#[cfg(integration-tests)]` — a "#178 stand-in." Production
`start_alloc` installs the intercept plumbing but never arms the redirect (nor the
inbound TPROXY rule). So a real deploy runs cleartext, because nothing tells the
agent which destinations are mesh peers — and that enumeration is the Resolve
layer, which has no production consumer. The Name layer is designed (whitepaper
§11: VIP/#61 + SPIFFE-ID/#178) but **no DNS responder exists** to answer the
stable name.

The seven competitor systems converge on three patterns Overdrive must adopt:
(1) **resolve to a concrete endpoint, THEN mTLS to that endpoint's identity** —
nobody does mTLS "to a VIP" (Istio HBONE, Linkerd's destination-returns-identity,
SPIFFE's identity-vs-naming split); (2) **arm the dataplane ahead of the connect**
(Cilium socket-LB, Consul's pre-pushed discovery, ztunnel's per-pod redirect) — a
late program is a cleartext miss; (3) **separate name from identity, join them in
the resolver**. Overdrive's two deliberate divergences — kernel kTLS crypto
(vs userspace) and auth-session==data-session (vs Cilium's out-of-band
auth+WireGuard) — are validated by where the industry is heading: Cilium's March
2026 "native mTLS with ztunnel" adopts exactly the per-connection identity-bound
model Overdrive already chose.

The recommendation: build an **`MtlsRedirectHydrator` reconciler** modelled on
`ServiceMapHydrator` that programs `MTLS_REDIRECT_DEST` (and the inbound TPROXY
rule) from `service_backends` ahead of the connect, carrying each backend's
**expected SVID** so the agent pins the intended peer (closing the v1
chain-to-bundle-only gap) — this single step turns the dormant engine ON — but treat that eager per-peer map
as a **single-node v1 bridge**, not the destination: the multi-node target is the
**enrollment / capture-and-resolve model** (ztunnel-shaped — capture all egress
from a mesh-enrolled workload, resolve destination + identity per-connection in
the agent, **fail toward the handshake, never toward cleartext**), tracked in
[#236](https://github.com/overdrive-sh/overdrive/issues/236). For the naming UX the
**primary path is transparent DNS** — `<name>.svc.overdrive.local` answered by a
**node-local DNS responder injected into the workload's `resolv.conf`** (the Fly.io
`fdaa::3` model, feasible because Overdrive ships its own appliance OS), needing
zero app changes; the SDK / SPIFFE-ID path (#178 + #53) drops to an **opt-in
optimization** for callers wanting client-side LB policy or instance pinning. The trickiest
open question for DESIGN is the **VIP × agent-light-intercept composition** (how
the agent learns the post-LB backend's expected identity), likely needing a
Tier-3 spike.

## Research Methodology

**Search Strategy**: Two evidence streams. (1) Codebase grounding — read the
ADR-0069 transparent-mTLS design, the `cgroup_connect4_mtls` BPF hook, the
`MtlsInterceptWorker`, the `ServiceMapHydrator` / `BackendDiscoveryBridge`
reconcilers, the `service_backends`/`mtls/inbound.rs` surfaces, and whitepaper
§11/§14 — verifying every PART A fact against the source before asserting. (2)
Competitor evidence — targeted searches + primary-doc fetches on the seven
in-scope systems, prioritising the two closest analogs (Istio ambient/ztunnel,
Cilium eBPF) first, then Linkerd/Consul/K8s/SPIFFE/Fly. Built on the prior-art
`docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`
and `docs/research/transparent-encryption-comprehensive-research.md` rather than
re-deriving the enforcement layer.

**Source Selection**: Types: project-internal PRIMARY (Overdrive code/ADRs —
authoritative for current state), official project docs (istio.io, docs.cilium.io,
cilium.io, linkerd.io, developer.hashicorp.com/Consul, kubernetes.io, spiffe.io,
fly.io), all High (1.0). Verification: cross-reference each cross-system pattern
across ≥2 independent vendor docs (e.g. resolve-then-pin in Istio + Linkerd +
SPIFFE; arm-ahead-of-connect in Cilium + Consul).

**Quality Standards**: 3 sources/claim target (1 authoritative min for
single-vendor API facts); internal facts cite file/ADR/issue and are PRIMARY; avg
reputation >= 0.80; citation coverage > 95%.

---

## PART 1 — Overdrive Codebase Grounding (PRIMARY for current-state facts)

> **Source class.** Everything in PART 1 is a PRIMARY internal fact, verified by
> reading the cited file / ADR. Each claim cites the file path or ADR number.
> Internal facts do not need the 3-external-source rule — the code IS the
> authority for Overdrive's current state.

### 1.1 The three layers — definitions

The east-west "name → encrypted connection" problem decomposes into three
layers that must be kept distinct (the failure mode of conflating them is
exactly what PART 2 shows competitors getting right or wrong):

| Layer | Question it answers | Overdrive status |
|---|---|---|
| **Name / address** | How is the destination written? (`payments.svc.overdrive.local`) | DESIGNED, NOT BUILT (whitepaper §11; #61 VIP/DNS, #178 SPIFFE-ID literal). **No DNS responder exists.** |
| **Resolve** | service identity → set of healthy backend addresses (+ each backend's expected SVID) | BUILT for the data source (`service_backends` table, `BackendDiscoveryBridge`, `ServiceMapHydrator`); the *client-facing* resolution primitive (#178) is NOT built. |
| **Enforce** | wrap the bytes in mTLS, optionally pin the peer identity | SHIPPED engine (#26 / ADR-0069 / ADR-0070), **dormant in production** — nothing arms the redirect map. |

The product goal touches all three: a stable name (Name), resolved to a
backend + its expected identity (Resolve), with the connection transparently
mTLS-wrapped (Enforce). Overdrive has built the Enforce engine and the Resolve
data source; the gap is the **wiring that connects them and the Name layer on
top.**

### 1.2 Enforce layer — kernel-mediated transparent mTLS (#26, ADR-0069/0070) — SHIPPED

**Verified against** `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md` (1055 lines, **Accepted** 2026-06-12, amended 2026-06-13), `crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs`, `crates/overdrive-worker/src/mtls_intercept_worker.rs`.

The mechanism is a **universal agent-light L4 proxy** — one mechanism for every
workload kind (process/exec, WASM, microVM, unikernel; #222 folded into #26):

- **Workloads hold NOTHING** — no cert, no key. They open ordinary plaintext
  sockets. Both the client and server workload are identity-unaware
  (ADR-0069 Decision, "BOTH workloads are identity-unaware and hold NOTHING").
- **The node's agent does both halves of every flow it touches.** Outbound
  (client side): the workload's `connect()` is transparently intercepted via a
  `cgroup/connect4` rewrite to the agent's plaintext leg-F listener; the agent
  drains the workload's plaintext losslessly, runs the rustls TLS 1.3 **client**
  handshake against the real peer presenting the workload's held SVID (read via
  the `IdentityRead` port), arms kTLS on the peer-facing leg, and hands
  steady-state byte movement to the kernel. Inbound (server side): a connection
  aimed at the server workload's logical address is TPROXY-intercepted to the
  agent's `IP_TRANSPARENT` leg-C listener; the agent recovers the original
  destination via `getsockname()`, selects the server workload's SVID, runs the
  rustls **server** handshake (presents the server SVID, **verifies the client's
  SVID** chains to the trust bundle via `WebPkiClientVerifier`), arms kTLS-RX,
  and splices the decrypted plaintext to the identity-unaware server workload.
- **Agent-light, not agent-out.** The kernel kTLS engine does the AES-GCM
  crypto; the agent does ZERO userspace crypto. The asymmetry (per the
  2026-06-13 amendment): the DECRYPT/RX directions are zero-copy `splice(2)`
  pumps out of a plain kTLS-RX leg; the ENCRYPT/TX directions are a bounded
  `read → write_all` copy into a kTLS-TX leg (a `splice` *into* kTLS-TX loses
  records — the `MSG_DONTWAIT` backlog class). The SSOT for the shipped
  mechanism is `crates/overdrive-dataplane/src/mtls/splice.rs`.
- **Spike-proven on a real 7.0 kernel** (≥ pinned 6.18 floor, ADR-0068): AF_PACKET
  capture shows TLS 1.3 application_data records (`0x17`), 0 plaintext bytes on
  the wire; fail-closed on `nocert` / `wrongca` (distinct reasons, 0 bytes
  delivered). Inbound is proven COMPOSED end-to-end in one direction
  (increment-i); three narrow composition gaps remain (outbound-in-one-flow,
  bidirectional round-trip, real netns/veth topology) — these are the Slice-00
  walking-skeleton gate, NOT "the mechanism is unproven."
- **`MtlsEnforcement` driven port** carries the per-connection surface
  (`enforce` / `teardown`); host adapter in `overdrive-dataplane`, Sim adapter
  in `overdrive-sim`. The worker's `MtlsInterceptWorker` (separate lifecycle
  component, fired by the action-shim at `on_alloc_running` / `on_alloc_terminal`)
  owns per-alloc intercept install + leg-acquire + `enforce` wiring.

**The honest v1 security claim** (ADR-0069 § "What this does NOT do"): v1 #26 is
**chain-to-bundle transport authentication + encryption, with NO intended-peer
identity pinning.** Both directions verify only that the peer's cert chains to
the trust bundle — i.e. the peer is *some* valid cluster workload, not
necessarily the *intended* one. A routing bug, VIP collision, or malicious
in-cluster endpoint presenting a valid-but-unintended SVID is **NOT prevented in
v1.** That gap is closed by intended-peer SAN-matching, which is the #178
upgrade. **Authorization** (may this workload talk to that peer?) is a separate
subsystem at the BPF-LSM `socket_connect` hook (#27/#38), explicitly out of #26's
scope.

### 1.3 The critical production gap — `MTLS_REDIRECT_DEST` is never armed in production

**This is the load-bearing finding of the whole research.** Verified by reading
the BPF hook and the worker line-by-line.

**The outbound hook is a no-op on a map miss.**
`crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs` (lines 38-82): the
`cgroup_connect4_mtls` program looks up `MTLS_REDIRECT_DEST[MtlsDestKey{ip,port}]`.
On a **miss** it `return Ok(1)` — "allow the connect unchanged (not a programmed
mTLS destination)" (line 62-65 verbatim). It rewrites `(user_ip4, user_port)` to
the agent's leg-F listener **only on a hit**. The module docstring is explicit:
"Returns 1 on every code path — the hook only rewrites; it never denies (it is a
proxy intercept, NOT a firewall). On a miss ... the connect proceeds unchanged"
(lines 19-21).

**Production never programs the map.**
`crates/overdrive-worker/src/mtls_intercept_worker.rs`:
- `start_alloc` (lines 334-419) is fired unconditionally at `on_alloc_running`.
  It attaches `cgroup_connect4_mtls` to the alloc's `.scope` cgroup, binds the
  leg-F (outbound plaintext) and leg-C (inbound `IP_TRANSPARENT`) listeners, and
  spawns the accept→`enforce` loops. It programs **NEITHER** the outbound
  `MTLS_REDIRECT_DEST` redirect **NOR** the inbound nft-TPROXY rule (module
  docstring lines 16-20 verbatim: "It programs **NEITHER** of the two east-west
  service-resolution facts v1 has no production source for").
- The ONLY code that programs `MTLS_REDIRECT_DEST` is
  `program_declared_peer_redirect` (lines 546-576), which is
  `#[cfg(feature = "integration-tests")]` — test-only, explicitly the "#178
  stand-in" (lines 522-538: "NOT a production surface — v1 production has no
  east-west peer enumeration (that is #178); `start_alloc` never programs the
  redirect itself").
- The DECLARED-PEER module note (lines 38-61) states the cause: "v1 has NO
  production source for 'the set of peers this workload will dial' — that
  enumeration is east-west service resolution (#178 / #61, DEFERRED)."

**Consequence (verified by tracing the control flow):** in a real deploy
`MTLS_REDIRECT_DEST` is empty → every outbound `connect()` misses the map →
`cgroup_connect4_mtls` returns 1 unchanged → the workload connects **in
cleartext, directly to the real peer**, bypassing the agent entirely. The
encryption engine is built, spike-proven e2e, and wired fail-closed at boot, but
**dormant in production because nothing tells the agent which destinations are
mesh peers.** That enumeration is precisely the **Resolve** layer.

**Inbound has the symmetric gap.**
`crates/overdrive-dataplane/src/mtls/inbound.rs::server_dial_addr` is the single
named production-replacement site for the orig-dst→real-backend mapping (today
faked by a test-only nft DNAT, "GAP-3"). `start_alloc` records
`tproxy_guard = None` and installs no production TPROXY rule (worker lines
391-409) — the rule's match key is the server workload's logical (virt) address,
"the same #178 east-west fact with no v1 production source." The
`install_inbound_tproxy` free function is the named #178 production-install site,
exercised today only by integration tests.
`InterceptedConnection::expected_peer` (currently `None`) + a
`PeerIdentityMismatch` error variant are reserved for the future intended-peer
pinning the #178 candidate-set will supply.

**The shape of the fix is therefore precise:** something must, *ahead of the
workload's connect*, populate `MTLS_REDIRECT_DEST[real_peer] = agent_leg_F` for
every mesh peer the workload may dial, and populate the inbound TPROXY rule for
the server's logical address. A late program = a cleartext miss. This is a
reconciler-shaped job (program a kernel map from observed state, ahead of
demand) directly analogous to `ServiceMapHydrator` (§1.4).

### 1.4 Resolve layer — `service_backends`, BackendDiscoveryBridge, ServiceMapHydrator (data source BUILT)

**Verified against** `crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs`, `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs`, `crates/overdrive-core/src/traits/observation_store.rs` (the `ServiceBackendRow` schema, #69).

Overdrive already runs a two-reconciler pipeline that turns a deployed Service
into a kernel-programmed load-balanced VIP. This is the **template** the mTLS
map-arming should be modelled on (it is the same "program a kernel map from
observed state, ahead of demand" shape).

1. **`BackendDiscoveryBridge` (#174)** — desired-side: projects the workload's
   declared listener set (`ServiceListenerSet` / `ProjectedListener` — the
   `(vip, port, protocol)` triple, VIP allocator-issued via
   `ServiceVipAllocator::get(&spec_digest)` per ADR-0049 §5a, NOT carried by
   intent); actual-side: the `Running` alloc set from
   `ObservationStore::alloc_status_rows_for_workload`. It emits
   `WriteServiceBackendRow` actions and persists only the *inputs* (the last
   written fingerprint) in its View — recompute-on-tick, never a derived
   "needs-write" boolean. The output is `service_backends` rows in the
   ObservationStore (Corrosion-gossiped, LWW).

2. **`ServiceMapHydrator`** — watches `service_backends` rows (desired) and
   `service_hydration_results` rows (the dataplane's confirmed-state observation,
   actual). Emits one `Action::DataplaneUpdateService` per service whose
   `BackendSetFingerprint` diverges; reads the hydration-result row on the next
   tick. The action executor writes the XDP `SERVICE_MAP` (the kernel LB map).

**Key shape facts for the design:**
- `service_backends` is keyed on `(service_id, region)` (whitepaper §11) and joins
  with `alloc_status` for health — only `running` backends are programmed.
- The reconcilers are pure-sync `reconcile(desired, actual, view, tick)` →
  `(Vec<Action>, View)` per ADR-0035/0036, DST-replayable, no `.await`, no DB
  handle. The map write is an `Action` executed by the action-shim (the
  reconciler computes the diff; the dataplane is its executor —
  `.claude/rules/reconcilers.md`).
- `ServiceDesired` sources `(port, proto)` from a listener-bearing fact (ADR-0060),
  never synthesised — so the resolved backend set already carries the L4 protocol.

**What is NOT built:** the **client-facing resolution primitive** (#178) — a
call that returns `{(backend_addr, expected_svid)}` filtered to `running`, for a
caller (or the agent) to consume. `service_backends` carries the backend
addresses but is consumed today only by the LB hydrator, not by an mTLS
peer-enumeration consumer, and it does not currently carry each backend's
**expected SVID** (the field the intended-peer pin will need). That join
(`service_backends` × `issued_certificates`/`IdentityMgr` → expected SVID per
backend) is part of the #178 gap.

### 1.5 Name layer — whitepaper §11, #61 (VIP/DNS), #178 (SPIFFE-ID literal); no DNS responder

**Verified against** `docs/whitepaper.md` §11 ("Private Service VIPs and
Auto-Wake", lines 1454-1464) and §14 (scale-to-zero / proxy-triggered resume).

Whitepaper §11 is the canonical model and names **two** addressing paths:

- **SPIFFE-ID literal (#178) — the opt-in path.** "East-west traffic inside an Overdrive cluster
  addresses services by SPIFFE ID (`spiffe://overdrive.local/job/payments`)
  resolved via the local ObservationStore." The workload links the SDK (#53) and addresses
  peers by SPIFFE ID — whitepaper §11's *native* label is from Overdrive's
  identity-model view, not the application's. The client-side resolution returns `{(backend_addr, expected_svid)}`
  filtered to `running` from the local ObservationStore; the client picks +
  applies its own LB policy. Depends on #69, #174, #26.
- **VIP/DNS (#61) — the transparent path (PRIMARY).** An ordinary DNS name +
  socket, zero app change — the universal way any unmodified workload (third-party
  binary, legacy client, WASM runtime) addresses a service. A stable per-service
  IPv6 VIP, `<job>.svc.overdrive.local →
  fdc2:<cluster>:<region>:<job-hash>::<N>`, allocated from the Overdrive-reserved
  ULA prefix `fdc2::/16`. "XDP SERVICE_MAP routes VIP traffic to the current
  backend set from `service_backends`, and the standard sockops layer wraps the
  connection in SPIFFE mTLS — the caller sees a plain IPv6 socket, the dataplane
  still enforces identity-bound encryption." Whitepaper §11 labels these "non-native callers" —
  backwards from the app's view, since DNS is the *most* native thing an
  application has. **This is the primary naming UX** (see §3.3). Depends on #167
  (VIP allocator), #24 (XDP service LB / roadmap 2.2), #26.

> **[Conflict flag — stale whitepaper framing].** §11 line 1462 says the VIP
> connection is wrapped by "the standard sockops layer." ADR-0069 (Accepted
> 2026-06-12) **superseded the in-band sockops+kTLS model** with the agent-light
> L4 proxy as the v1 #26 enforcement path; the in-band sockops path is NOT v1
> scope (#231). The whitepaper §11 prose predates ADR-0069 and reads as if
> sockops is the enforcement mechanism. The *intent* (VIP traffic is wrapped in
> SPIFFE mTLS by the dataplane) is unchanged; the *mechanism* named is stale.
> This is a back-propagation the architect should reconcile. Tracked here as a
> documentation conflict, not a design blocker.

**The unresolved gap in BOTH paths — there is no DNS responder.** #61 says
"expose it as `<job>.svc.overdrive.local`" but the resolver mechanism — *what
process answers a DNS query for that name, where it runs, how the workload's
`getaddrinfo`/`connect` reaches it* — is **unspecified.** No code in the repo
answers `<name>.svc.overdrive.local`. This is the single biggest open Name-layer
question and a central subject of PART 3.

### 1.6 Constraints that bound the design (single-node v1, appliance OS, sidecarless, trust domain)

**Verified against** CLAUDE.md, whitepaper, `docs/product/architecture/brief.md`
(referenced), and the feedback-memory facts.

- **Single-node v1.** Multi-node is later. `SERVICE_MAP` / `REVERSE_NAT_MAP` are
  empty today in practice because every backend is local (same-host delivery via
  `LOCAL_BACKEND_MAP` + the `cgroup_connect4` hook). Phase 1 has no node
  registration, no multi-region. The design must extend to multi-node without
  rework but need not implement it now.
- **Ships its own immutable appliance OS with a pinned kernel** (ADR-0068, 6.18
  LTS floor). Overdrive controls the resolver, the dataplane, and the kernel —
  it is NOT an agent running on an operator-supplied kernel. This is a major
  degree of freedom vs Cilium/Istio (which must run on arbitrary node kernels):
  Overdrive can place a node-local resolver, intercept port 53, or hook
  `connect()` at will.
- **Sidecarless** — no in-pod / in-VM agent. The node's agent does the mTLS work;
  the workload holds nothing. (Contrast: Linkerd/Consul/Istio-sidecar inject a
  per-pod proxy.)
- **SPIFFE trust domain `spiffe://overdrive.local`**; the `job/<name>` path
  component is the canonical workload-identity scheme across all workload kinds
  (whitepaper line 2872). Two CAs exist (operator/control-plane HTTPS CA vs
  workload-identity CA — CLAUDE.md); the workload-identity CA signs the SVIDs the
  agent presents.
- **CLI verb is `overdrive deploy <SPEC>`** (no `job submit`).
- **Failure semantics already partly designed** (whitepaper §11/§14): when no
  backend is `running`, the VIP path returns `XDP_PASS` to the local gateway,
  which buffers the request (≤1 MB) and triggers a resume (proxy-triggered
  resume / scale-to-zero, §14). This is the "name resolves but no healthy
  backend" case — auto-wake, not fail-fast — and it is a design precedent the
  mTLS path must compose with.

---

## PART 2 — Competitor / External Evidence

### 2.1 Istio ambient mode / ztunnel (closest analog: per-node agent-light L4 proxy)

**The closest analog to Overdrive's agent-light L4 proxy.** ztunnel is a
per-node, Rust, sidecar-less L4 proxy that does transparent mTLS — no sidecar,
the application is unaware.

**(a) Name + who resolves it.** ztunnel reuses Kubernetes service resolution.
"When a pod in an ambient mesh makes an outbound request, it will be
transparently redirected to the node-local ztunnel which will determine where
and how to forward the request ... requests to a Service will be sent to an
endpoint within the Service while requests directly to a Pod IP will go directly
to that IP" [Istio ambient data-plane]. The DNS layer is unchanged CoreDNS
(§2.5); ztunnel sits **below** DNS — it intercepts the *connection* to the
resolved ClusterIP, not the name lookup. Confidence: High.

**(b) How mTLS is transparently applied.** "All inbound and outbound L4 TCP
traffic between workloads in the ambient mesh is secured by the data plane, using
mTLS via HBONE, ztunnel, and x509 certificates." "Traffic is HBONE encapsulated
and encrypted in the network namespace of the source pod itself, and eventually
decapsulated and decrypted in the network namespace of the destination pod"
[Istio traffic-redirection]. The application "has no awareness of either." This
is the same identity-unaware-workload + node-agent-does-mTLS shape as Overdrive
#26. **Difference: ztunnel does the AES-GCM crypto in userspace (rustls);
Overdrive hands steady-state crypto to the kernel via kTLS (agent-light).** This
is exactly ADR-0069's rejected alternative A3 ("the ztunnel shape ... AES-GCM
crypto in userspace for every byte"). Confidence: High.

**(c) How the data path is armed (intercept).** istio-cni installs redirect rules
per pod. Three well-known ports: **15001** (outbound — all egress TCP redirected
here), **15006** (plaintext inbound), **15008** (HBONE inbound, encrypted). "The
istio-cni node agent establishes network redirection rules such that packets
entering and leaving the pod are intercepted and transparently redirected to the
node-local ztunnel." Redirection uses "iptables rules in the mangle and NAT
tables" with **TPROXY** to transparently redirect. **Overdrive's outbound
`cgroup_connect4` rewrite is the rough equivalent of ztunnel's port-15001
redirect; Overdrive's inbound TPROXY → leg-C is the direct analogue of ztunnel's
15006/15008 inbound TPROXY** — the mechanisms line up almost one-to-one.
Confidence: High.

**(d) How peer identity is established + verified.** HBONE = HTTP CONNECT over
mTLS (HTTP/2 multiplexed, TLS 1.3) on port 15008. "Each underlying tunnel
connection must have a unique source and unique destination identity, and those
identities must be used to establish encryption for that connection"
[Istio HBONE]. The destination address is carried in the HTTP CONNECT header;
ztunnel selects the destination identity from the resolved endpoint. **This is
the answer to the VIP×identity composition question (D-2):** ztunnel resolves the
ClusterIP to a concrete endpoint *first*, then opens the HBONE tunnel to *that
endpoint's* identity — the mTLS peer identity is the endpoint, not the VIP. L7
authorization (AuthorizationPolicy) is enforced at a **waypoint** proxy (a
separate, per-namespace/per-service Envoy), not at ztunnel; ztunnel does L4
identity + encryption only. Confidence: High.

**(e) Single-node → multi-cluster.** ztunnel encapsulates in the source pod's
netns and decapsulates in the destination pod's netns on the destination node —
the HBONE tunnel spans nodes natively (the CONNECT target is the destination
pod IP, routable cluster-wide). Multi-cluster is Istio multi-cluster mesh
(east-west gateways). Confidence: Medium-High.

> **Why ztunnel is the load-bearing precedent for Overdrive.** It validates the
> entire shape: per-node agent-light L4 proxy, identity-unaware workloads,
> TPROXY inbound + redirect outbound, resolve-to-endpoint-then-mTLS-to-endpoint.
> The single divergence Overdrive makes deliberately is kernel kTLS instead of
> userspace rustls crypto (the "agent-light" optimization, ADR-0069 vs A3). The
> port-architecture and the resolve-then-pin-the-endpoint pattern are directly
> transferable.

### 2.2 Cilium service mesh + transparent encryption + mTLS auth (eBPF, cgroup connect hooks)

Cilium is the closest **eBPF dataplane** analog — kube-proxy-free ClusterIP via
eBPF, socket-LB at the `cgroup/connect` hook (directly comparable to Overdrive's
`cgroup_connect4`). But its mTLS-AUTH story is architecturally **different** from
Overdrive's and from ztunnel's, in a way that is decision-relevant.

**(a) Name + resolve.** Cilium's socket-based load balancing translates a
ClusterIP `connect()` to a backend at the **socket layer** via a `cgroup/connect4`
(and `connect6`) BPF hook — the same hook family Overdrive uses. The address
rewrite happens at `connect()` time, *before* a packet is built, so the
application connects to the ClusterIP and the kernel transparently substitutes a
backend endpoint. This is the "socket-LB bypasses DNS for the connection" pattern
(D-1). Confidence: High (matches Overdrive's own `cgroup_connect4_service`).

**(b)+(c) Transparent encryption — node-level, NOT per-connection mTLS.** Cilium's
*encryption* is **WireGuard or IPsec transparent encryption at the node level**:
"the agent running on each cluster node will establish a secure WireGuard tunnel
between it and all other known nodes in the cluster" [Cilium WireGuard docs].
This is node-to-node, not workload-identity-bound per-connection TLS.

**(d) Mutual authentication — auth-session ≠ data-session (the key divergence).**
Cilium's classic Mutual Authentication (Beta) is **out-of-band**: "Cilium's
mTLS-based Mutual Authentication support brings the mutual authentication
handshake out-of-band for regular connections" [Cilium mutual-auth docs]. When a
packet matches a NetworkPolicy requesting authentication, a TLS handshake between
the two nodes' cilium-agents (using delegated SPIFFE/SPIRE identities) runs
**separately**; on success the agent writes an entry into an "Auth Table," and
the **actual data is encrypted independently by WireGuard/IPsec.** The docs'
roadmap explicitly lists "Per-connection handshake" and "Use auth secret for
network encryption" as **TODO** — i.e. auth-session and data-session are *not*
unified in the classic design.
**This is precisely ADR-0069's rejected alternative A4** ("Cilium-style
out-of-band auth + separate encryption (WireGuard/IPsec); auth-session ≠
data-session"). Overdrive deliberately preserves auth-session == data-session
(the rustls handshake's extracted secrets ARE the kTLS keys). SPIFFE/SPIRE here:
SVIDs are X.509, the SPIRE agent attests the workload, the SPIFFE ID is
`spiffe://trust.domain/path`. Confidence: High.

**(e) Convergence — Cilium native mTLS with ztunnel (2026).** A March 2026 Cilium
blog, "Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity
with ztunnel," signals Cilium **adopting the ztunnel model** to get
per-connection identity-bound mTLS (auth == data) rather than the out-of-band
auth + WireGuard split. "The SPIFFE identity is embedded in certificates and
verified at both ends, with workload certificates managed by SPIRE." This is the
industry converging on exactly the shape Overdrive picked. Confidence:
Medium-High (single blog source; cross-referenced against ztunnel's documented
model in §2.1). Multi-cluster: Cilium ClusterMesh (cross-cluster service
discovery + identity). Confidence: High.

> **Decision-relevant takeaway.** Cilium offers two precedents at opposite poles:
> (1) the **socket-LB `cgroup/connect` intercept** — which Overdrive already
> mirrors and which validates the "rewrite at connect, ahead of the packet"
> arming model (D-3); and (2) the **out-of-band-auth + node-WireGuard** model —
> which Overdrive explicitly rejected (A4) in favour of auth==data kTLS, and
> which Cilium itself is now moving away from (the 2026 ztunnel adoption).
> Overdrive's bet is validated by where Cilium is heading, not where it was.

### 2.3 Linkerd (cleanest identity model — the resolve→pin pattern, stated plainly)

Linkerd is sidecar-based (a micro-proxy per pod, not a per-node L4 proxy), so its
*enforcement* shape differs from Overdrive. But its **identity + destination
model is the cleanest articulation of the resolve-then-pin pattern** that
Overdrive's #178 must implement.

**(a)+(b) Identity.** "Each certificate is bound to the Kubernetes ServiceAccount
identity of the containing pod"; the control plane's `identity` service (a CA
with a trust anchor + issuer) issues 24h-rotated TLS certs to each proxy
[Linkerd automatic-mTLS / architecture]. Overdrive's analogue: SVIDs minted by
the workload-identity CA, held by `IdentityMgr`, rotated (workload-svid-rotation).

**(c)+(d) The destination → identity flow (the load-bearing precedent for #178).**
"When a proxy receives an outbound connection from the application container, it
looks up that destination with the Linkerd control plane, and if it's in the
Kubernetes cluster, **the control plane provides the proxy with the destination's
endpoint addresses along with metadata including an identity name. When the proxy
connects to the destination, it initiates a TLS handshake and verifies that the
destination proxy's certificate is signed by the trust anchor AND contains the
expected identity**" [Linkerd architecture]. **This is exactly the
`{(backend_addr, expected_svid)}` candidate set #178 specifies** — the resolver
returns both the address AND the identity to expect, and the client pins the peer
cert against that expected identity. Linkerd answers the D-2 composition question
("how does a VIP/name reconcile with mTLS to a specific backend identity?") by
**never doing mTLS to a VIP** — it resolves to the endpoint set + per-endpoint
expected identity first, then pins. Confidence: High.

**(e) Authorization** is a separate `Server` + `AuthorizationPolicy` layer
(Linkerd server-policy), distinct from authentication — same authn/authz split as
Overdrive (#26 authn vs #27/#38 authz). Multi-cluster: Linkerd multicluster
(mirrored services + a gateway). Confidence: High.

> **Takeaway.** Linkerd is the textbook statement of #178's contract: the
> resolver returns `(endpoint_addr, expected_identity_name)`; the proxy verifies
> chain-to-trust-anchor **AND** SAN==expected_identity. Overdrive's reserved
> `expected_peer` + `PeerIdentityMismatch` surface (§1.3) is the same hook;
> Linkerd shows the resolver is where the expected identity originates.

### 2.4 Consul service mesh / Connect (Consul DNS, transparent proxy, intentions)

Consul is the best precedent for the **DNS-responder + virtual-IP** path (the #61
question) and for **intentions** (authorization). It is Envoy-sidecar-based.

**(a) Name + who resolves it (the DNS responder Overdrive lacks).** Consul runs a
**DNS responder**. Services resolve via `<service>.service.consul` and, in
transparent-proxy mode, a per-service **virtual IP** via
`<service>.virtual.consul` (and `<port-name>.<service>.virtual.consul` for
multi-port). "The `transparent_proxy` block ensures that DNS queries are made to
Consul so that service names like `count-api.virtual.consul` resolve to a virtual
IP address" [Consul transparent-proxy]. **This is the missing piece in
Overdrive's Name layer made concrete: a DNS responder answers the stable name with
a per-service VIP**, and transparent-proxy interception forces the connection to
that VIP through the mesh. Confidence: High.

**(b)+(c) Transparent proxy + arming.** "By default, the Consul service mesh runs
in transparent proxy mode, which forces inbound and outbound traffic through the
sidecar proxy even though the service binds to all interfaces" — via
iptables/redirect rules that capture traffic to the virtual IP. "Transparent
proxy uses intentions to infer traffic routes between Envoy proxies." The Envoy
config is pushed with "service-discovery results for upstreams" — i.e. the proxy
is armed ahead of time with the resolved backend set (the same "program ahead of
the connect" model as Overdrive's map-arming, D-3). Confidence: High.

**(d) Peer identity + authorization.** "When transparent proxy mode is enabled on
the upstream, services present service mesh certificates for mTLS and **intentions
are enforced at the destination.**" "When transparent proxy mode is enabled, all
service-to-service traffic is required to use mTLS." Intentions are the
authorization layer (allow/deny service-to-service), enforced at the destination
proxy — distinct from the mTLS authentication. Service identity is a Connect/SPIFFE
cert. Confidence: High.

**(e) Single-node → multi-cluster.** Consul WAN federation / cluster peering;
the virtual IP + DNS model extends across federated datacenters. Confidence:
Medium-High.

> **Takeaway.** Consul answers the two questions Overdrive's Name layer leaves
> open: (1) a **DNS responder** that resolves the stable name to a per-service
> **virtual IP** (the VIP, Istio-ClusterIP-style, not endpoint IPs), and (2)
> **destination-enforced authorization** (intentions) layered on top of mTLS
> authentication. Both map to Overdrive's #61 (VIP/DNS) and #27/#38 (LSM authz).

### 2.5 Kubernetes DNS / CoreDNS (ClusterIP VIP vs headless / EndpointSlice — the core D-1 fork)

Kubernetes DNS (CoreDNS) is the canonical statement of the **VIP-vs-endpoints
resolver fork** that Overdrive must choose between (D-1).

**(a) The two resolution models.**
- **ClusterIP (VIP) service** — "Normal (not headless) Services are assigned DNS A
  records with a name of the form `my-svc.my-namespace.svc.cluster-domain` ...
  This resolves to the **cluster IP** of the Service" [K8s DNS]. The single stable
  VIP is load-balanced to backends by kube-proxy / eBPF socket-LB. **This is the
  Istio/Cilium/Consul-virtual-IP model** — DNS returns one VIP; the dataplane LBs.
- **Headless service** — "Unlike normal Services, this resolves to the **set of
  IPs of all of the Pods** selected by the Service ... no cluster IP is allocated,
  kube-proxy does not handle these Services, and there is no load balancing or
  proxying done by the platform. A headless Service allows a client to connect to
  whichever Pod it prefers, directly." Backed by EndpointSlices; CoreDNS returns
  A records per pod. **This is the #178 client-side-resolution model** — DNS
  returns the endpoint set; the client picks + applies its own LB.

**(b) EndpointSlice** is the scalable per-endpoint record (replaced Endpoints);
CoreDNS lists Services + EndpointSlices to resolve names. ClusterIP allocation is
a managed range (the K8s analogue of Overdrive's `fdc2::/16` VIP pool, #167).
Confidence: High.

> **Takeaway — the D-1 decision crystallized.** K8s makes the fork explicit and
> ships BOTH: a VIP service (one address, platform LBs — Overdrive #61) and a
> headless service (endpoint set, client LBs — Overdrive #178). They are **not
> mutually exclusive**; K8s exposes both for different consumers. This directly
> supports the PART 3 recommendation that Overdrive build BOTH #61 (VIP/DNS — the
> transparent path, any unmodified client) and #178 (endpoint set + expected SVID
> — the opt-in SDK path) rather than picking one — the precedent for "two resolver shapes, same backend
> source" is the most battle-tested in the industry.

### 2.6 SPIFFE / SPIRE (identity vs naming separation — the architectural justification for keeping the layers distinct)

SPIFFE is the identity standard Overdrive already uses (trust domain
`spiffe://overdrive.local`). Its load-bearing lesson for THIS research is the
**deliberate separation of identity from naming**.

**(a) The SPIFFE ID is identity, NOT a network address.** "A SPIFFE ID is a
string that uniquely and specifically identifies a workload," format
`spiffe://<trust-domain>/<path>` (RFC 3986 URI; scheme + trust-domain
case-insensitive, path case-sensitive) [SPIFFE ID spec / concepts]. It is "a
logical identity, independent of networking or DNS." **SPIFFE explicitly does NOT
do service discovery / naming** — "the documentation does not explain the
mechanism for translating service names to SPIFFE identities ... the resolution
process is outside SPIFFE's scope." Confidence: High.

**(b) Verification = chain-to-bundle, optionally + expected-ID.** "An SVID is
considered valid if it has been signed by an authority within the SPIFFE ID's
trust domain"; the trust bundle is "a collection of one or more CA root
certificates that the workload should consider trustworthy." Chain validation is
the base; matching an *expected* SPIFFE ID (SAN) on top is the application's job —
**which is exactly Overdrive's v1 (chain-to-bundle only) vs #178 (chain + expected
SAN) split** (§1.2-1.3). Confidence: High.

**(c) Attestation.** SPIRE agents attest the workload (node + workload selectors)
before issuing an SVID — the identity is earned, not declared. Overdrive's
analogue: the control-plane mints per-allocation SVIDs; the agent holds them; the
workload holds nothing.

> **The architectural justification this research rests on.** SPIFFE's
> deliberate split — **identity (the SVID) is separate from naming (DNS /
> discovery)** — is the principle behind keeping Overdrive's three layers
> distinct. The Name layer (`payments.svc.overdrive.local` / VIP) answers "where";
> the SVID answers "who"; the Resolve layer is the **join** that produces
> `(where, who-to-expect)` — and that join is precisely what #178 must build and
> what every mesh above implements somewhere (Linkerd in the destination
> service, Istio in the resolved endpoint, Consul in the discovery results). The
> SVID never encodes the address; the resolver supplies the address-to-identity
> binding.

### 2.7 Fly.io (`.internal` DNS + 6PN/WireGuard — the closest PRODUCT analog)

Fly.io is the closest "named workload over a private encrypted mesh" **product**
analog, and the cleanest template for Overdrive's missing node-local DNS
responder.

**(a) Name + who resolves it (a node-local DNS server — the exact placement
Overdrive needs).** "Our service discovery system populates a database on each
host that we run a **Rust DNS server** off of, to serve the 'internal' domain" —
the DNS server is **node-local**, serving cached data from central orchestration.
"We inject the IP of that DNS server into your `resolv.conf` — the IP address of
that server is always **`fdaa::3`**." So every workload's libc resolver hits the
node-local DNS server automatically, no app config. Confidence: High.

**(b) What it returns — endpoint set, NOT a VIP.** "If your application is
`fearsome-bagel-43`, its DNS zone is `fearsome-bagel-43.internal` — that DNS
resolves to **all the IPv6 6PN addresses** deployed for the application." This is
the **headless / endpoint-set** model (K8s-headless, #178-shaped), not a VIP.
Regional resolution is hierarchical: `nrt.fearsome-bagel-43.internal` =
instances in Japan; `<machine_id>.vm.<appname>.internal` for a specific machine;
`top<N>.nearest.of.<app>.internal` for nearest-N. Confidence: High.

**(c)+(d) Encryption + isolation.** "Fly.io is fully connected through a
**WireGuard mesh** joining every point in our network." Per-org isolation is
enforced by eBPF: "BPF programs ... enforce access control (you can't talk to one
6PN network from another)." Note: 6PN encryption is **WireGuard node-mesh
(network-layer), NOT per-workload-identity mTLS** — so Fly gives the
name+resolve+encrypted-transport template but NOT the workload-identity-bound
per-connection mTLS Overdrive targets (Fly's model is closer to Cilium-WireGuard
than to ztunnel/Overdrive). Confidence: High.

> **Takeaway.** Fly.io is the strongest precedent for Overdrive's DNS-responder
> placement: a **node-local DNS server, injected into the workload's resolver
> (`resolv.conf`), returning the endpoint set** for `<name>.internal`. Given
> Overdrive ships its own appliance OS, it can do exactly this — inject a
> node-local resolver into every workload's namespace. Fly answers the
> Name-layer "where does the resolver run" question (node-local) and the
> "VIP-or-endpoints" question for the native case (endpoints) better than any
> other product analog. Where Fly stops short — no per-workload-identity mTLS —
> is exactly where Overdrive's #26 engine adds value.

### 2.8 Honorable mentions (Tailscale MagicDNS, Envoy original-dst)

- **Tailscale MagicDNS + WireGuard identity.** MagicDNS resolves short device
  names to Tailscale IPs over a WireGuard mesh with per-node identity (WireGuard
  public keys, not SPIFFE). Same node-local-resolver + encrypted-mesh shape as
  Fly; identity is the WireGuard key, not a rotatable workload SVID. A precedent
  for name→address over an encrypted mesh, weaker on workload-identity rotation.
  Confidence: Low (not deep-researched; cited as a shape, not a load-bearing
  source).
- **Envoy original-dst cluster.** Envoy's `ORIGINAL_DST` cluster forwards to the
  socket's original destination (recovered via `SO_ORIGINAL_DST` / `getsockname`
  under TPROXY) rather than a configured upstream — the same orig-dst-recovery
  primitive Overdrive's inbound leg-C uses (`getsockname`, §1.2). A precedent for
  the transparent-intercept→recover-orig-dst→forward pattern. Confidence:
  Medium (general Envoy knowledge; not a primary-source fetch this run).

---

## PART 3 — Synthesis

### 3.1 Competitor comparison matrix (name / resolve / enforce)

| System | **Name** (how written + who resolves) | **Resolve** (VIP vs endpoints; how identity is bound) | **Enforce** (where mTLS happens; auth==data?) | Multi-cluster |
|---|---|---|---|---|
| **Istio ambient / ztunnel** | K8s DNS (CoreDNS); ztunnel below DNS, intercepts the connection | Resolve ClusterIP→endpoint first, then mTLS to **that endpoint's** identity (HBONE CONNECT carries dest) | Per-node L4 proxy (ztunnel), **userspace rustls crypto**; auth==data (mTLS tunnel IS the data path); L7 authz at waypoint | Istio multi-cluster mesh |
| **Cilium (classic)** | K8s DNS; socket-LB rewrites ClusterIP at `cgroup/connect` | VIP→backend at socket layer; identity = Cilium identity / SPIFFE-SPIRE | Node-level **WireGuard/IPsec** encryption + **out-of-band** mutual-auth handshake; **auth ≠ data** (separate sessions) | ClusterMesh |
| **Cilium native mTLS (2026)** | K8s DNS | resolve→endpoint, SPIFFE cert verified both ends | Adopting **ztunnel** model — per-connection identity-bound mTLS (converging to auth==data) | ClusterMesh |
| **Linkerd** | K8s DNS | Destination service returns **endpoint addrs + expected identity name**; proxy pins cert chain + expected SAN | Per-pod **sidecar** micro-proxy, userspace TLS; auth==data; authz = Server/AuthorizationPolicy | Linkerd multicluster (gateway) |
| **Consul Connect** | **Consul DNS responder**: `<svc>.service.consul`, `<svc>.virtual.consul` → **virtual IP** | Transparent-proxy resolves VIP; Envoy armed with discovery results ahead of connect | Per-service **Envoy sidecar**, userspace TLS; auth==data; **intentions** = destination-enforced authz | WAN federation / cluster peering |
| **Kubernetes DNS** | CoreDNS: `<svc>.<ns>.svc.cluster.local` | **ClusterIP** (VIP, platform LBs) OR **headless** (endpoint set via EndpointSlice, client LBs) — BOTH shipped | (no mesh; raw) | — |
| **SPIFFE/SPIRE** | **N/A — naming is out of scope**; SPIFFE ID is identity, not address | The SVID is `who`; resolver supplies `where`+`who-to-expect` (the join is the app's job) | (identity only; not an enforcement mechanism) | SPIFFE federation |
| **Fly.io 6PN** | **Node-local Rust DNS** at `fdaa::3`, injected into `resolv.conf`; `<app>.internal` | **Endpoint set** (all 6PN IPv6 addrs); regional `nrt.<app>.internal`, `<machine>.vm.<app>.internal` | **WireGuard node-mesh** (network-layer), per-org eBPF isolation; **NOT per-workload mTLS** | global WireGuard mesh |
| **→ Overdrive (target)** | `<job>.svc.overdrive.local` (#61, VIP) AND `spiffe://overdrive.local/job/<n>` literal (#178, endpoints); **DNS responder TBD** | `service_backends` (built) → VIP (#61) or `{addr, expected_svid}` (#178); identity join TBD | **Per-node agent-light L4 proxy, kernel kTLS crypto** (#26 / ADR-0069); auth==data; authz at LSM (#27/#38) | single-node v1; cross-node `service_backends` later |

**Three patterns every mature system shares (and Overdrive must adopt):**
1. **Resolve to an endpoint, THEN mTLS to that endpoint's identity** — nobody does
   mTLS "to a VIP." The VIP/name is load-balanced to a concrete backend first;
   the mTLS peer identity is the backend's (Istio HBONE, Linkerd destination,
   Consul discovery). This is the answer to D-2.
2. **Arm the dataplane ahead of the connect** — Envoy gets discovery results
   pushed, ztunnel's redirect is installed per-pod, Cilium's socket-LB map is
   programmed before traffic. A late program = a miss. This is the answer to D-3.
3. **Separate name (DNS/VIP) from identity (SVID), join them in the resolver** —
   SPIFFE's explicit principle; every mesh implements the join somewhere.

**Two divergences Overdrive makes deliberately (both validated):**
- **Kernel kTLS crypto, not userspace** (vs ztunnel/Linkerd/Consul/Envoy
  userspace TLS). The "agent-light" optimization (ADR-0069 A3). Cilium's 2026
  ztunnel adoption shows the industry moving toward identity-bound mTLS;
  Overdrive's kTLS twist is the performance edge on top.
- **auth==data** (vs Cilium-classic's out-of-band auth + WireGuard). ADR-0069 A4
  rejected the split; Cilium itself is now leaving it.

### 3.2 Overdrive current-state + gap analysis

| Layer | Built | Missing |
|---|---|---|
| **Enforce** | The #26 agent-light L4 proxy engine: outbound `cgroup_connect4_mtls` intercept, inbound TPROXY→leg-C, rustls handshake, kTLS arm, agent-light splice/copy pumps, fail-closed, `MtlsEnforcement` port, `MtlsInterceptWorker` lifecycle. Spike-proven e2e (TLS 1.3 `0x17` on the wire, 0 plaintext). | **Production arming** of `MTLS_REDIRECT_DEST` (outbound) and the inbound TPROXY rule — both #178-deferred, today only in `#[cfg(integration-tests)]` stand-ins. **Intended-peer pinning** (`expected_peer`/`PeerIdentityMismatch` reserved but unused). |
| **Resolve** | `service_backends` table (#69), `BackendDiscoveryBridge` (#174) writing it, `ServiceMapHydrator` programming the XDP LB `SERVICE_MAP`. The "program a kernel map from observed state" reconciler pattern. | **Client-facing resolution primitive** (#178): `{(backend_addr, expected_svid)}` filtered to `running`. The **expected-SVID-per-backend join** (`service_backends` × identity) does not exist. No consumer enumerates mesh peers for the mTLS redirect. |
| **Name** | The model (whitepaper §11): VIP scheme `<job>.svc.overdrive.local → fdc2:.../::N` (#61) and SPIFFE-ID literal (#178). VIP allocator concept (#167). | **No DNS responder** — nothing answers `<name>.svc.overdrive.local`. VIP allocator (#167) not built. The §11 prose still names the superseded "sockops layer" (stale, §1.5 conflict flag). |

**The one-sentence gap:** Overdrive has built the encryption engine and the
backend-discovery data source, but the **wire between them is empty in
production** — no reconciler programs the mTLS redirect from `service_backends`,
and no DNS responder turns a stable name into a connection — so a real deploy
runs **cleartext** (the outbound hook misses the empty `MTLS_REDIRECT_DEST` map
and passes the connect through unchanged).

### 3.3 Recommended architecture

The recommendation composes the proven Enforce engine with a new Resolve→arm
reconciler and a node-local DNS responder, each choice justified by a competitor
precedent AND the codebase reality.

**(R1) Build the Resolve→arm reconciler FIRST — it lights up the dormant engine.**
Add an **`MtlsRedirectHydrator`** reconciler, modelled exactly on
`ServiceMapHydrator` (§1.4): desired-side = the set of `(real_peer_addr,
expected_svid)` mesh peers a workload may dial, sourced from `service_backends`
joined with identity facts; actual-side = the programmed `MTLS_REDIRECT_DEST`
entries (observed). It emits an action that programs `MTLS_REDIRECT_DEST[peer] =
agent_leg_F` **ahead of the workload's connect**, and (symmetrically) installs the
inbound TPROXY rule for the server's logical address. This is the production
replacement for `program_declared_peer_redirect` / `install_inbound_tproxy`.
*Justified by:* Cilium socket-LB + Consul "Envoy armed with discovery results"
(arm ahead of connect, D-3) AND `ServiceMapHydrator`'s existing reconciler shape
(the codebase already does this for the LB map). **This single step turns the
shipped-but-dormant engine on** — the highest-leverage move in the whole plan.

**(R2) Resolve to an endpoint, then pin THAT endpoint's identity — never mTLS to a
VIP.** The redirect map (and the inbound `server_dial_addr`) must carry the
**concrete backend address + that backend's expected SVID**, not a VIP. Wire the
reserved `expected_peer` to the resolved SVID so the agent SAN-matches the peer
against the *intended* identity (closing the v1 chain-to-bundle-only gap, #178).
*Justified by:* Linkerd ("endpoint addresses + expected identity name; verify
chain AND expected identity"), Istio HBONE (resolve-then-tunnel-to-endpoint),
SPIFFE (the resolver supplies the address↔identity join) AND the codebase's
reserved `expected_peer`/`PeerIdentityMismatch` surface (§1.3).

**(R3) Transparent DNS is the PRIMARY naming path; SDK/SPIFFE-ID is opt-in.** The
product goal is "any unmodified workload reaches `<name>.svc.overdrive.local`
encrypted, zero app changes" — so the transparent DNS path is primary, and the
SDK / SPIFFE-ID path (#178 client surface + #53) drops to an **opt-in
optimization** for callers wanting client-side LB policy or instance pinning. (An
earlier draft had this reversed — "native first" — which is backwards from the
application's point of view; see the revision note.) The *resolve* capability
underneath (service → `{(addr, expected_svid)}` filtered to `running`) is shared
by both paths and by R1's arming — it is foundational regardless. *Justified by:*
Kubernetes shipping BOTH ClusterIP and headless; Istio ambient / Linkerd
delivering mTLS transparently over ordinary DNS with no app SDK; the stated
product goal.

**(R4) Place the DNS responder node-local, injected into the workload's resolver —
or skip DNS entirely for the opt-in SDK path via the connect-hook.** Two
sub-options, matching the two naming paths:
- *Transparent / DNS path (#61, PRIMARY):* a **node-local DNS responder injected
  into the workload's namespace `resolv.conf`** (the Fly.io `fdaa::3` model),
  answering `<job>.svc.overdrive.local` with the per-service VIP, then the existing
  XDP `SERVICE_MAP` LBs the VIP and the agent wraps it in mTLS. Works for any
  unmodified client — no SDK.
- *Opt-in / SDK path (#178):* a **socket-LB-style `connect()` resolution** that
  bypasses DNS — the SDK (#53) resolves `spiffe://.../job/payments` to a backend at
  connect time, exactly as Cilium's socket-LB rewrites a ClusterIP. No DNS
  round-trip, but requires the SDK.
*Justified by:* Fly.io (node-local Rust DNS injected into resolv.conf — Overdrive
ships its own appliance OS, so it can do exactly this), Cilium socket-LB (DNS
bypass for the native case), Consul (`virtual.consul` VIP DNS for the non-native
case).

**(R5) VIP × agent-light-intercept composition — resolve the VIP to a backend
BEFORE the mTLS handshake.** The open composition question (D-2). The answer from
every analog: the VIP is load-balanced to a concrete backend *first* (XDP
`SERVICE_MAP` for #61), and the agent then handshakes to *that backend's*
identity. Concretely: the outbound intercept must fire on the **post-LB backend
address** (or the agent must itself do the LB selection and learn the chosen
backend's expected SVID), so leg-B's handshake pins the real backend, not the
VIP. This is the single trickiest wiring and is called out as a DESIGN-wave open
question (Q1 below).

**(R6) The eager per-peer map (R1) is a single-node bridge; the multi-node target
is the enrollment model.** R1 enumerates peers into a kernel map ahead of connect
— a cardinality cost at mesh scale, and its miss = silent cleartext. The preferred
multi-node interception model is **enrollment / capture-and-resolve**
(ztunnel-shaped): capture all egress from a mesh-enrolled workload, resolve
destination + identity per-connection in the agent, and **fail toward the
handshake, not toward cleartext** (a should-be-mesh destination not yet resolved
attempts mTLS or holds; external / non-mesh egress passes through). *Justified by:*
Istio ambient/ztunnel (no per-destination datapath map); Cilium auth-map miss →
handshake, not cleartext. Tracked in
[#236](https://github.com/overdrive-sh/overdrive/issues/236).

**Recommended end-to-end path (the minimal "named, encrypted" flow):**
```
workload connect(spiffe://overdrive.local/job/payments)   [or VIP]
  → resolve (R3/#178): service_backends ∩ running → pick backend B, expected_svid(B)
  → MtlsRedirectHydrator (R1) has pre-programmed MTLS_REDIRECT_DEST[B]=leg_F
  → cgroup_connect4_mtls rewrites connect(B) → connect(leg_F)   [HIT, not miss]
  → agent leg-F drains plaintext, rustls client handshake to B presenting workload SVID,
    verifies B's SVID chains to bundle AND SAN==expected_svid(B)   (R2)
  → kTLS arm; agent-light pumps; TLS 1.3 on the wire
server side: TPROXY rule (R1) → leg-C → getsockname orig-dst → server SVID
  → server handshake verifies client SVID → splice plaintext to server workload
```

### 3.4 Build sequencing (mapped to #26/#61/#178/#167/#24/#69/#174)

Ordered for the shortest path to a minimal "named, encrypted, end-to-end" flow,
each step naming what it unblocks:

1. **Step 1 — `MtlsRedirectHydrator` reconciler (the arming wire).** Depends on:
   #69 (`service_backends`, built), #174 (`BackendDiscoveryBridge`, built), #26
   (engine, built). Reuses the `ServiceMapHydrator` pattern. **Unblocks:**
   turns the dormant #26 engine ON in production for the single-node local case —
   the first deploy where two workloads actually talk mTLS instead of cleartext.
   This is **#178's enforcement half** and the highest-leverage step.
2. **Step 2 — expected-SVID join + intended-peer pin.** Extend `service_backends`
   (or a sibling fact) to carry each backend's expected SVID; wire `expected_peer`
   / `PeerIdentityMismatch` (#178). **Unblocks:** the honest "pinned to the
   intended peer" security claim, closing the v1 chain-to-bundle-only gap.
3. **Step 3 (opt-in) — SDK / SPIFFE-ID resolution primitive (#178 + #53).** The
   client-facing `{(addr, expected_svid)}` call (or connect-hook resolution).
   **Unblocks:** the opt-in path for callers wanting client-side LB policy or
   instance pinning. Not required for the transparent DNS UX.
4. **Step 4 — VIP allocator (#167) + XDP VIP routing (#24 / roadmap 2.2).**
   Allocate `fdc2::/16` VIPs; route VIP→backend in `SERVICE_MAP`. **Unblocks:**
   the #61 VIP path for any unmodified client — the transparent primary UX, with
   Step 5 (depends on #167, #24, #26).
5. **Step 5 — node-local DNS responder (#61 Name layer).** Inject a node-local
   resolver into the workload namespace; answer `<job>.svc.overdrive.local` → VIP
   (Fly.io model). **Unblocks:** the full transparent stable-name UX for any
   unmodified client; composes with #167's VIP and #24's LB.
6. **Step 6 — cross-node extension (multi-node).** `service_backends` already
   carries addresses; extend the hydrator + agent dial to off-node backends
   (ClusterMesh-shape). **Unblocks:** multi-node mesh without reworking the
   single-node design.

Steps 1-2 turn the engine on and pin the intended peer (single node). **Steps 4-5
deliver the primary transparent UX** — any unmodified workload reaching
`<name>.svc.overdrive.local` encrypted; with DNS as the primary path they take
priority over the opt-in **Step 3** (SDK / SPIFFE-ID). Step 6 is multi-node — at
which point the eager per-peer map (Step 1) should give way to the **enrollment /
capture-and-resolve model**
([#236](https://github.com/overdrive-sh/overdrive/issues/236)).

### 3.5 Open questions / risks for the DESIGN wave

**Q1 (highest) — VIP × agent-light-intercept composition (D-2/R5).** When a caller
connects to a VIP (#61), the XDP `SERVICE_MAP` LBs the VIP to a backend, but the
mTLS handshake needs *that backend's* expected SVID. Two unresolved sub-questions:
(a) does the outbound `cgroup_connect4_mtls` intercept fire on the VIP or on the
post-LB backend address? (b) where does the agent learn the chosen backend's
expected SVID — does the XDP LB selection have to be surfaced back to the agent,
or does the agent do its own LB+identity-lookup? Every competitor resolves to an
endpoint *before* the mTLS peer is pinned (§3.1 pattern 1); Overdrive's two
interceptors (XDP LB at packet level + agent at socket level) must be ordered so
the agent sees the real backend. **This needs a DESIGN decision and likely a
Tier-3 spike** (the cgroup-connect-hook ordering vs XDP, on a real kernel — and
note the no-Tier-2-backstop hazard for `cgroup_sock_addr` programs, feedback
memory).

**Q2 — DNS responder vs socket-LB resolution (D-1).** For the opt-in SDK path,
should Overdrive answer DNS at all, or resolve `spiffe://.../job/payments` at the
`connect()` hook (Cilium socket-LB style) and skip DNS? DNS is simpler and works
for any libc client; the connect-hook is faster and avoids a DNS dependency but
needs the SDK or a name→address map in the kernel. For the VIP path, where does
the node-local DNS responder run in the appliance-OS, sidecarless model, and how
is it injected into each workload's `resolv.conf` (Fly `fdaa::3` model) without an
in-pod agent? **DESIGN must pick the resolver placement and the
DNS-vs-connect-hook split per naming path.**

**Q3 — the expected-SVID-per-backend source.** `service_backends` carries addresses
but not each backend's SVID. Where does the expected SVID come from at resolve
time — a join with `issued_certificates` (audit facts only, no usable SVID), a
new observation row written by `BackendDiscoveryBridge`, or derived
deterministically from the backend's `AllocationId`→`job/<name>` (since the SPIFFE
ID is `spiffe://overdrive.local/job/<name>`, the *expected* ID may be derivable
without a join)? The last option is attractive — the expected SAN is a pure
function of the job name — but DESIGN must confirm the SPIFFE-ID scheme makes it
so, and that it survives multi-replica (per-alloc) identities.

**Q4 — failure semantics composition (D-6).** "Name resolves but no healthy
backend" already has a designed answer (whitepaper §14 auto-wake / scale-to-zero
via XDP_PASS → gateway buffer → resume). But "resolve succeeds, intercept can't
arm" must be **fail-closed** (the worker already does this for install failures,
§1.2). DESIGN must reconcile: the VIP no-backend path is *fail-open-to-gateway*
(auto-wake), but the mTLS-can't-arm path is *fail-closed* (no cleartext). These
two failure modes meet at the same VIP; their precedence must be specified. Note
the preferred miss semantics is **fail-toward-handshake** (attempt mTLS for a
should-be-mesh destination) rather than the v1 fail-toward-cleartext — also tracked
in [#236](https://github.com/overdrive-sh/overdrive/issues/236).

**Q5 — single-node→multi-node `MTLS_REDIRECT_DEST` cardinality.** On a single node
every backend is local; the redirect map is small. Multi-node means programming a
redirect for every off-node peer a workload may dial — potentially the whole mesh.
The map-arming reconciler's scope (program-all-peers vs program-on-first-resolve)
is a scale question the design should not foreclose. Cilium/Istio program lazily
or per-policy; Overdrive's "ahead of connect" requirement (a late program =
cleartext miss) is in tension with "don't program the whole mesh." **This is now tracked in
[#236](https://github.com/overdrive-sh/overdrive/issues/236)** with a stated
preference for the **enrollment / capture-and-resolve model** (capture all egress
from a mesh-enrolled workload; resolve per-connection in the agent; no per-peer
datapath map) and **fail-toward-handshake** miss semantics. The eager per-service
map (R1) is the single-node v1 bridge only.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| ADR-0069 transparent-mTLS agent-light L4 proxy | (internal) | PRIMARY | ADR / source | 2026-06-16 | Y (vs code) |
| `cgroup_connect4_mtls.rs` (outbound hook) | (internal) | PRIMARY | source | 2026-06-16 | Y (vs ADR) |
| `mtls_intercept_worker.rs` (worker) | (internal) | PRIMARY | source | 2026-06-16 | Y (vs ADR) |
| `service_map_hydrator.rs` / `backend_discovery_bridge.rs` | (internal) | PRIMARY | source | 2026-06-16 | Y |
| whitepaper §11 / §14 | (internal) | PRIMARY | design SSOT | 2026-06-16 | Y |
| Istio ambient traffic-redirection | istio.io | High (1.0) | official docs | 2026-06-16 | Y (vs HBONE, data-plane) |
| Istio HBONE | istio.io | High (1.0) | official docs | 2026-06-16 | Y |
| Istio ambient data-plane / overview | istio.io | High (1.0) | official docs | 2026-06-16 | Y |
| Cilium mutual authentication (Beta) | docs.cilium.io | High (1.0) | official docs | 2026-06-16 | Y (vs WireGuard docs) |
| Cilium WireGuard transparent encryption | docs.cilium.io | High (1.0) | official docs | 2026-06-16 | Y |
| Cilium native mTLS with ztunnel (2026-03) | cilium.io | High (1.0) | vendor blog | 2026-06-16 | Y (vs ztunnel model) |
| Linkerd automatic mTLS / architecture | linkerd.io | High (1.0) | official docs | 2026-06-16 | Y |
| Consul transparent proxy / DNS / intentions | developer.hashicorp.com | High (1.0) | official docs | 2026-06-16 | Y |
| Kubernetes DNS for Services and Pods | kubernetes.io | High (1.0) | official docs | 2026-06-16 | Y |
| SPIFFE ID spec / concepts | spiffe.io | High (1.0) | standards docs | 2026-06-16 | Y |
| Fly.io 6PN private networks / private networking | fly.io | High (1.0) | official docs/blog | 2026-06-16 | Y |

Reputation: High/PRIMARY: 16 of 16 (100%). Avg reputation: **~0.99** (all sources
are official vendor docs, standards, or PRIMARY internal source). Internal PRIMARY
facts (5) verified by direct file read; external facts (11) each cross-referenced
against ≥1 sibling doc or the documented model of an adjacent system.

## Knowledge Gaps

### Gap 1: Cilium native-mTLS-with-ztunnel blog body (JS-rendered)
**Issue**: The March 2026 Cilium blog returned only its title on fetch (JS-only
body). **Attempted**: direct WebFetch; the WebSearch snippet supplied the
load-bearing quote ("SPIFFE identity embedded in certificates, verified at both
ends, SPIRE-managed"). **Recommendation**: the convergence claim (§2.2e) is
cross-referenced against ztunnel's documented model; if the exact Cilium
implementation detail becomes load-bearing for a DESIGN decision, re-fetch via a
non-JS mirror or the Cilium docs (not the blog).

### Gap 2: The expected-SVID-per-backend source (Q3)
**Issue**: `service_backends` carries addresses, not each backend's SVID; whether
the expected SVID is derivable from `job/<name>` or needs a join is unconfirmed.
**Attempted**: read `service_map_hydrator.rs`, `backend_discovery_bridge.rs`,
whitepaper §8/§11 (SPIFFE scheme). **Recommendation**: a DESIGN-wave decision +
a read of `issued_certificates`/`IdentityMgr` to confirm derivability vs join
(flagged as Q3).

### Gap 3: VIP × intercept ordering on a real kernel (Q1)
**Issue**: whether the outbound intercept fires pre- or post-XDP-LB, and how the
agent learns the chosen backend's identity, cannot be settled from docs —
Overdrive's dual-interceptor (XDP packet-level + agent socket-level) ordering is
unique. **Attempted**: Istio/Cilium docs (they resolve-to-endpoint but do not run
Overdrive's specific XDP+agent split). **Recommendation**: a Tier-3 spike per the
feedback-memory "no-Tier-2-backstop for `cgroup_sock_addr`" hazard (flagged Q1).

### Gap 4: brief.md not re-read this run
**Issue**: §1.6 constraints cite brief.md indirectly (via CLAUDE.md + whitepaper).
**Attempted**: CLAUDE.md + whitepaper §11/§14/§8 supplied the constraints.
**Recommendation**: low risk — the single-node/appliance-OS/sidecarless/trust-domain
facts are corroborated across CLAUDE.md and the whitepaper; a brief.md read would
add commercial/tenancy framing not load-bearing for this networking design.

## Conflicting Information

### Conflict 1: Whitepaper §11 "sockops layer" vs ADR-0069 agent-light proxy
**Position A** (whitepaper §11, line 1462): VIP traffic is wrapped by "the
standard sockops layer." **Position B** (ADR-0069, Accepted 2026-06-12):
in-band sockops+kTLS is **superseded** for v1 by the agent-light L4 proxy;
sockops-on-the-workload's-own-socket is out of v1 scope (#231). **Assessment**:
ADR-0069 is more authoritative and more recent — it is the accepted decision
record; the whitepaper §11 prose predates it. The *intent* (VIP traffic wrapped
in SPIFFE mTLS by the dataplane) is unchanged; only the named *mechanism* is
stale. Flagged for architect reconciliation; not a design blocker (§1.5).

### Conflict 2: Cilium "mutual auth" model — auth-session ≠ data-session (vs Overdrive auth==data)
Not a contradiction *within* sources but a deliberate **architectural divergence**:
Cilium classic mutual-auth is out-of-band (auth handshake separate; WireGuard/IPsec
encrypts data — auth ≠ data), which ADR-0069 rejected as alternative A4. Recorded
as a design contrast, not a factual conflict; both vendor docs agree on what
Cilium does. Cilium's own 2026 ztunnel adoption is moving toward Overdrive's
auth==data model.

## Recommendations for Further Research

1. **Tier-3 spike on VIP × agent-intercept ordering (Q1)** — the single
   highest-risk unknown; settle pre/post-XDP-LB intercept + identity learning on a
   real kernel before the DESIGN locks the VIP path.
2. **Confirm expected-SVID derivability from `job/<name>` (Q3)** — read
   `IdentityMgr`/`issued_certificates`; if the expected SAN is a pure function of
   the job name, the resolver join is avoidable.
3. **Node-local DNS responder placement spike (Q2)** — confirm the Fly `fdaa::3`
   resolv.conf-injection model is feasible in the appliance-OS, sidecarless
   per-workload-namespace setup.

## Full Citations

[1] Overdrive Project. "ADR-0069: Transparent mTLS via a universal agent-light L4 proxy". `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md`. Accepted 2026-06-12 (amended 2026-06-13). (internal PRIMARY). Verified 2026-06-16.
[2] Overdrive Project. "`cgroup_connect4_mtls` outbound intercept program". `crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs`. (internal PRIMARY). Verified 2026-06-16.
[3] Overdrive Project. "`MtlsInterceptWorker`". `crates/overdrive-worker/src/mtls_intercept_worker.rs`. (internal PRIMARY). Verified 2026-06-16.
[4] Overdrive Project. "`ServiceMapHydrator` / `BackendDiscoveryBridge` reconcilers". `crates/overdrive-core/src/reconcilers/{service_map_hydrator,backend_discovery_bridge}.rs`. (internal PRIMARY). Verified 2026-06-16.
[5] Overdrive Project. "Whitepaper §11 (Private Service VIPs and Auto-Wake), §14 (Right-Sizing / scale-to-zero / proxy-triggered resume)". `docs/whitepaper.md`. (internal PRIMARY). Verified 2026-06-16.
[6] Overdrive Project. "Transparent per-Workload mTLS Architecture research". `docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`. 2026-06-05. (internal PRIMARY prior art). Verified 2026-06-16.
[7] Istio. "Ztunnel traffic redirection". istio.io. https://istio.io/latest/docs/ambient/architecture/traffic-redirection/. Accessed 2026-06-16.
[8] Istio. "HBONE". istio.io. https://istio.io/latest/docs/ambient/architecture/hbone/. Accessed 2026-06-16.
[9] Istio. "Ambient data plane / Overview". istio.io. https://istio.io/latest/docs/ambient/architecture/data-plane/. Accessed 2026-06-16.
[10] Cilium. "Mutual Authentication (Beta)". docs.cilium.io. https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/. Accessed 2026-06-16.
[11] Cilium. "WireGuard Transparent Encryption". docs.cilium.io. https://docs.cilium.io/en/stable/security/network/encryption-wireguard/. Accessed 2026-06-16.
[12] Cilium. "Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity with ztunnel". cilium.io. 2026-03-23. https://cilium.io/blog/2026/03/23/native-mtls-cilium/. Accessed 2026-06-16.
[13] Linkerd. "Automatic mTLS" / "Architecture". linkerd.io. https://linkerd.io/2-edge/features/automatic-mtls/, https://linkerd.io/2.14/reference/architecture/. Accessed 2026-06-16.
[14] HashiCorp. "Transparent proxy overview" / "Enable transparent proxy on Kubernetes" / "Envoy proxy configuration". Consul. developer.hashicorp.com. https://developer.hashicorp.com/consul/docs/connect/proxy/transparent-proxy. Accessed 2026-06-16.
[15] Kubernetes. "DNS for Services and Pods" / "Service". kubernetes.io. https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/. Accessed 2026-06-16.
[16] SPIFFE. "SPIFFE ID" / "SPIFFE Concepts" / "Trust Domain and Bundle". spiffe.io. https://spiffe.io/docs/latest/spiffe-specs/spiffe-id/, https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/. Accessed 2026-06-16.
[17] Fly.io. "Incoming! 6PN Private Networks" / "Private Networking". fly.io. https://fly.io/blog/incoming-6pn-private-networks/, https://fly.io/docs/networking/private-networking/. Accessed 2026-06-16.

## Research Metadata

Duration: ~1 session (turns ~1-45) | Examined: 16 sources (5 internal PRIMARY, 11
external official) + 2 prior-art research docs | Cited: 17 | Cross-refs: every
cross-system pattern verified across ≥2 vendors | Confidence: **High** (internal
current-state facts code-verified; external patterns cross-referenced; the one
single-source claim — Cilium 2026 convergence — is corroborated against ztunnel's
documented model and flagged) | Citation coverage: >95% | Avg source reputation:
~0.99 | Output: `docs/research/networking/stable-service-naming-and-transparent-mtls-comprehensive-research.md`

**Confidence distribution by finding**: High — PART 1 (all code-verified), §2.1
Istio, §2.3 Linkerd, §2.4 Consul, §2.5 K8s, §2.6 SPIFFE, §2.7 Fly, §3.1-3.4.
Medium-High — §2.2e Cilium 2026 convergence (single blog, cross-ref'd). Medium —
§2.8 honorable mentions (cited as shapes, not deep-researched). The recommended
architecture (§3.3) and build sequencing (§3.4) are interpretations grounded in
both streams and labelled as recommendations, not facts.
