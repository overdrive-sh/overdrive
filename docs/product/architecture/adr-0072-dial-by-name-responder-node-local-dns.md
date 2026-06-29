# ADR-0072: Dial-by-name responder — node-local in-agent DNS over the ObservationStore (the THIRD reader)

## Status

**Accepted** (2026-06-25). Decision-maker: Morgan (solution-architecture);
**ratified by the user** (Pass-1 decision points A1/B/C1/D1/E1/F/H1 all
decided 2026-06-24/25). Tags: phase-2, mesh, dns, name-layer, reachability,
observation-store, transparent-mtls, headless, ipv4-only, #243.

**REVISED (REV-2, 2026-06-25) — FINAL. Both open forks RATIFIED by the user;
the gating spike (BLOCKER-1) returned WORKS.** The user ratified shifting the
answered address from a *volatile per-instance backend addr* to a **stable
per-`<job>` frontend addr** (the "ClusterIP split"): DNS answers a stable
address, the already-live dataplane (nft-TPROXY + per-connection `MtlsResolve`,
ADR-0071 Path-A / ADR-0053) owns backend churn. This **supersedes the
headless-v1 answer contract below** (§ "The v1 DNS answer contract" and DDN-2's
byte-consistency framing). The two forks are now DECIDED — **REV-1a = 1a-A** (a
NEW per-`<job>` `FrontendAddrAllocator`, sibling to `NetSlotAllocator`, from a
distinct subnet) and **REV-1b = 1b-A** (an additive `by_frontend` map on
`BackendIndex` + a `classify` translation arm). A subsequent design-review
revision (2026-06-25, 3 High findings) tightened the executable contract without
changing this direction — see **§ Changed Assumptions → "REV-2 contract
tightenings"**: the `by_frontend` key carries the proto axis —
`FrontendKey = (SocketAddrV4, Proto)` = `(F, listener.port, listener.protocol)`
(a bare `SocketAddrV4` cannot distinguish `tcp/53` from `udp/53`; v1 captures TCP
only at the worker layer, so the key carries `Proto::Tcp` because the capture is
TCP, NOT because the axis was dropped); `F` is per-logical-workload (WITHHOLD on
zero-healthy, RELEASE only on deletion); and the DNS↔resolve race is made
non-exploitable by an ordered drain (`by_frontend` before `name_index`) PLUS a
fail-closed-on-frontend-subnet-miss `classify` arm. The frontend subnet is
**pinned to `10.98.0.0/16`** (NOT the spike's `10.96.0.0/16` candidate — that collides
with the live service-VIP allocator default; see § Changed Assumptions →
"Pinned frontend subnet"). See **§ Changed Assumptions (REV-2)** — it quotes the
superseded contract verbatim, states the replacement, and records both
ratified forks + the pinned subnet + the BLOCKER resolutions. Committed roadmap
steps 01-01 (`MeshServiceName`) / 01-02 (`NameAnswer` + `hickory` wire codec)
are unaffected (the `SocketAddrV4` substrate + the codec are agnostic to which
IPv4 addr they carry). This is NOT the deferred #61 IPv6-VIP / XDP-VIP-LB path.
Code-grounded inputs:
`docs/research/networking/dial-by-name-thin-path-canonical-address-vs-cilium-socketlb-research.md`
(SQ1/SQ2/SQ6) and
`docs/research/networking/dns-positive-ttl-vs-cache-invalidation-dial-by-name-research.md`
(sub-q 4).

**CORRECTION (2026-06-25) — review-surfaced correctness fix to DDN-7's `<job>`
label ceiling.** The original DDN-7 pinned `<job>` label ≤ `LABEL_MAX (253)`,
which conflated the DNS-*name* max (253, total FQDN, RFC 1035 §3.1) with the
DNS-*label* max. A `<job>` is a single DNS label, hard-capped at **63 octets**
(RFC 1035 §2.3.4). The corrected ceiling is 63. This prevents a latent
`wire::encode` panic: a 64–253-char `<job>` that `MeshServiceName::new` accepted
makes `Name::from_str("<job>.svc.overdrive.local")` return `Err` (the
`hickory-proto` codec enforces the 63-octet label limit), hitting an
`unreachable!` at the DNS boundary. `LABEL_MAX (253)` itself is correct and
unchanged — it remains the DNS-name ceiling for *derived* label-shaped ids
(`development.md` § "One shared length ceiling for label-shaped ids"); only the
`MeshServiceName` `<job>` DNS-*label* ceiling is corrected here.

**AMENDMENT (REV-3, 2026-06-25) — the missing `FrontendAddrAllocator` WRITER is
now named.** REV-2 named the two *readers* of the `<job> ↔ F` binding (the
`name_index` / `answer_for` DNS path, DDN-1/DDN-2; the re-keyed
`MtlsResolve.by_frontend`, 1b-A) and the *single-owner injection* (DDN-6: the
composition root constructs ONE `FrontendAddrAllocator` and injects the SAME
handle into both readers). It did NOT name the production actor that *calls*
`FrontendAddrAllocator::assign(<job>)` when a `<job>` is declared. The prose
described the binding passively — "bound on first running-AND-healthy backend
for the `<job>`" (REV-2 1a-A), "the allocator RETAINS `F` … idempotent
`assign(<job>)` returns the same `F`" (feature-delta § Frontend lifecycle
contract) — and the feature-delta C4 even drew the *write* (`drain — looks up /
binds <job> → F via the allocator`) inside the **health-gated ordered drain**,
which contradicts Finding 2 (the `<job> ↔ F` binding MUST be **orthogonal to
backend health**, surviving zero-healthy windows). With no named writer, the
just-landed `name_index` (commit `53236c6a`) made the **DNS read path** the
de-facto writer — `frontend_for` calls `self.allocator.assign(name).ok()`
(`name_index.rs:262`) on *every query*, so a DNS lookup *allocates* a stable
frontend addr as an order-dependent, non-deterministic side effect, turning the
DNS index into a second writer to the shared allocator.

**REV-3 names the writer (and corrects the read path):**

- **The writer is a deploy-time lifecycle assigner.** `assign(<job>)` is called
  at **`<job>` declaration** — in the **Service arm of `submit_workload`**
  (`POST /v1/jobs`, `handlers.rs` ~324, the SAME `if matches!(intent,
  WorkloadIntent::Service(_))` admission guard the `ServiceVipAllocator::allocate`
  uses). This is the exact in-codebase precedent: an addressing allocator keyed
  by the *logical workload*, bound at admission, fsynced before the handler
  returns. Frontends are a Service-name concern, so a Job-kind submit does NOT
  assign a frontend addr (mirroring the VIP allocate's Service-only guard).
- **Boot story = empty-on-boot Bar-1 converge-on-boot rebuild** (NOT the
  *persistent* `ServiceVipAllocator` `bulk_load` model). The allocator is
  ephemeral (ADR-0072 1a-A, the `NetSlotAllocator` precedent), so on every node
  boot it starts empty and a dedicated converge-on-boot pass re-derives the
  `<job> → F` set from the **currently-declared Service intents**
  (`IntentKey::for_workload` rows) — idempotent `assign` per `<job>` — run AFTER
  `AppState` and BEFORE the convergence loop / `DnsResponder` serve spawn, the
  mirror of `veth_provisioner::adopt_on_restart_recovery` (`lib.rs:1980`).
  Declared-Service intent is the SSOT the rebuild reads (`development.md` §
  "Persist inputs, not derived state"); the allocator is never persisted and
  never inferred from a prior allocator dump.
- **`frontend_for` (the DNS read path) is a PURE READER.** It performs a
  read-only lookup of the allocator's *existing* `<job> → F` binding (e.g.
  `snapshot().get(<job>)`), **never** `assign` on the query path. A `<job>` the
  assigner has not yet bound is WITHHELD (NXDOMAIN), not assigned-on-read. The
  `name_index.rs:262` `assign`-on-read is the defect REV-3 corrects.
- **`release(<job>)` has NO production trigger today — and that is correct, not a
  gap.** There is no logical-workload-DELETION verb in production: the router
  exposes only `POST /v1/jobs` (declare), `POST /v1/jobs/:id/stop` (TRANSIENT —
  the `IntentKey::for_workload` row PERSISTS, the `<job>` stays declared;
  `handlers.rs:718-764`), and GET reads (`lib.rs:2094-2100`). "stop" is the
  withhold-not-release case (Finding 2), NOT a release — and the
  `ServiceVipAllocator` confirms the pattern (the VIP releases ONLY on the
  conflict-rollback path, `handlers.rs:451-459`, never on stop). The
  `release(<job>)` *surface* is implemented and Tier-1-tested (FRONTEND-03
  Property 2), but it has no production CALL SITE because the deletion edge does
  not exist. Wiring `release` into the stop path would reintroduce the SQ1
  stale-`F` failure on every stop (explicitly rejected). The deletion-verb
  trigger is **tracked in `overdrive-sh/overdrive#211`** ("Implement workload
  deletion (intent withdrawal) + service/dataplane teardown") — that verb's
  teardown producer will drive `release(<job>)` alongside its existing
  `ReleaseServiceVip` / `DeregisterLocalBackend` teardown — and is out of *this*
  feature's scope. Until then `F` is retained for the process lifetime of every
  declared `<job>` (acceptable Phase-1 single-node — the empty-on-boot rebuild
  reading the current declared set naturally drops a binding for a `<job>` not
  re-declared after restart).

The single-owner invariant (DDN-2) is unchanged and now complete: the ONE
`FrontendAddrAllocator` is **written by the deploy-time assigner** and **read by
both** the `name_index` and `by_frontend` — the answered `F` is byte-identical to
the recognized `F` because there is exactly one writer and one instance. This
amendment is realized in the roadmap as REV-3 step 01-05 (the writer) + the
01-03 re-scope (the pure reader); the code correctives are a separate crafter
dispatch.

**AMENDMENT (REV-4, 2026-06-26) — Finding-3(i) coherence and the EQUIV-01
contract reconciled to the AS-IMPLEMENTED two-drains-one-allocator
architecture.** An adversarial review of step 02-00
(`docs/feature/dial-by-name-responder/deliver/review-02-00.md`, findings D1 +
D2), corroborated by independent verification against the landed code, surfaced
that two roadmap criteria (S-DBN-COHERENCE-01, S-DBN-EQUIV-01) were specified
against architecture the implementation does not have. This amendment reconciles
the DESIGN to implemented reality; the REV-2 DIRECTION (stable `10.98.0.0/16`
frontend + `MtlsResolve` translation + single-owner `FrontendAddrAllocator`) is
UNCHANGED, and the two genuinely-strong invariants (DDN-2 byte-identity,
Finding-3(ii) fail-closed) stay intact and become the load-bearing content.
REKEY-01..04 and FAILCLOSED-01 are untouched (the review confirmed all five
genuinely non-vacuous). **User-ratified 2026-06-26** (the ordering barrier was
an availability nicety, not a security invariant; security preserved by
fail-closed, independently re-verified in the 02-00 re-review).

**(1) Finding-3(i) "single ordered drain / write-time ordering barrier (STEP A
before STEP B)" — SUPERSEDED.** The Changed-Assumptions / DDN-1 / DDN-2 text
below (and the feature-delta § "Frontend-subnet coherence contract" (i)) pinned
a *single ordered drain* that updates `by_frontend` (STEP A) BEFORE exposing `F`
to `name_index` (STEP B) — a temporal write-time barrier. **No such single
shared drain exists.** The implementation has **two independent single-owner
drain tasks**, each owning its own `subscribe_all_events` subscription:

- the re-keyed `MtlsResolve`/`BackendIndex` drain that feeds `by_frontend`
  (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:55-57`), and
- the `NameIndex` drain that feeds the resolvable set
  (`crates/overdrive-control-plane/src/dns_responder/name_index.rs:41-43`),

both reading the **same `Arc`-shared `FrontendAddrAllocator`** (the roadmap 02-01
wiring constructs the re-keyed `MtlsResolve` with *its own* drain and a
*separate* `DnsResponder`/`name_index` — two components, one allocator). There is
no shared drain to order, so the temporal barrier describes a coordination
mechanism present nowhere. It is **superseded**.

The barrier was, at most, an **availability** nicety (avoid a transient
NXDOMAIN-then-refuse-then-retry hiccup when `name_index` exposes `F` for a `<job>`
whose `by_frontend` key the resolve drain has not yet applied), **NOT a security
invariant**. The security posture holds **regardless of inter-drain timing**:
because `10.98.0.0/16` is dedicated to mesh frontends, a dial to an `F` that
misses `by_frontend` classifies arm-2 (`∈ 10.98.0.0/16` miss) → `MeshUnreachable`
→ **fail-closed (refuse, never cleartext)** by Finding-3(ii) / FAILCLOSED-01. A
race window worst-cases to a refused connect + client retry, never silent
cleartext to a should-be-mesh peer.

**The enforced coherence invariant is byte-identity of `F` via the single
allocator (DDN-2 single-owner), not ordering.** Both drains call
`allocator.assign(<job>)` on the ONE `Arc`-shared `FrontendAddrAllocator`, so the
`F` `by_frontend` is keyed on equals the `F` `name_index` answers, byte-for-byte
— there is exactly one `<job> → F` source. A second `<job> → F` source (a
divergent allocator, a re-derivation, a stale cache) is the addressing-divergence
defect DDN-2 exists to prevent. S-DBN-COHERENCE-01 is rewritten to assert this:
**Property 1** (byte-identity via the ONE allocator — a mutant introducing a
second `F` source flips it) + **Property 2** (fail-closed regardless of
inter-drain timing — a mutant flattening the subnet-miss arm to `NonMesh` flips
it). It stays at roadmap step 02-00, Tier-1, test home
`dns_name_index.rs` — the in-process sibling of the Tier-3 S-DBN-SINGLE-SRC
(02-02) oracle. (The `mtls_resolve_adapter.rs` docstring that currently claims the
"STEP A before STEP B" barrier is an aspirational doc for an unenforced invariant,
CLAUDE.md "Never document behaviour that is not implemented" — the crafter fixes
that docstring; this amendment governs the design artifacts.)

**(2) S-DBN-EQUIV-01 "host build path AND in-memory test-double path" — RE-FRAMED
to reference-oracle equivalence.** `BackendIndex` is a **single internal struct of
ONE adapter**; there is no sim/host split for it (`SimMtlsResolve` is a different
level — the `MtlsResolve` trait — and was deliberately NOT re-keyed). A two-impl
"host-vs-in-memory" equivalence therefore collapses to driving two
`BackendIndex::default()` through identical calls → `assert_eq!(x, x)`, zero
independent kill power (review D1). Per `.claude/rules/development.md` § "Trait
definitions specify behavior": *if there is only one implementation, an
"equivalence" test of it against itself enforces nothing.* The reconciled
criterion replaces it with the genuine enforcement when only one implementation
exists: the re-keyed three-way `classify` trajectory over an arbitrary ordered
sequence of (row-apply | frontend-binding | classify-probe) steps matches an
**independent hand-written reference oracle** that re-derives the expected verdict
WITHOUT calling `BackendIndex` (the shape REKEY-01 already uses for its
first-by-`Ord` `min` recompute, generalised over the whole trajectory), AND the
trajectory is deterministic (seed → bit-identical). It stays at step 02-00,
Tier-1, test home `mtls_resolve_rekey.rs`.

The Finding-3(i) and DDN-2 framing in the REV-2 sections below, and the
feature-delta coherence contract, are read through this amendment: the enforced
property is byte-identity via the single allocator + timing-independent
fail-closed, NOT a temporal write-ordering barrier. The supersession is clean
(greenfield, single-cut — no deprecation, no transitional path).

**AMENDMENT (REV-5, 2026-06-27) — the mesh→mesh datapath includes an
output-hook leg-B interception companion; the "nft-TPROXY interception UNCHANGED
/ no enforcement surface" framing is CORRECTED.** This ADR's scope statement
(Context / "What this ADR does NOT touch") asserts *"the nft-TPROXY
both-directions interception … UNCHANGED"* and *"the responder … adds no
enforcement surface and modifies no security-critical path."* The dial-by-name
walking-skeleton RCA
(`docs/analysis/root-cause-analysis-dial-by-name-by-frontend-resolve-rst.md`)
**falsifies that for the mesh→mesh hop.** dial-by-name is the first path where the
dialed `orig_dst` (the stable frontend `F` ∈ `10.98.0.0/16`) ≠ the resolved
backend `workload_addr` (∈ `10.99.0.0/16`), so after `resolve(F) → Mesh(backend)`
the agent re-dials the backend (leg-B) from the **host netns** — a
locally-originated connect that traverses the kernel **OUTPUT** hook, while the
destination's inbound mTLS interception (`install_inbound_tproxy`) is a
**`type filter hook prerouting`**-only `tproxy` rule. leg-B is therefore
un-intercepted, hits the plaintext workload listener, and the agent's leg-B
client handshake reads cleartext → `InvalidContentType` → RST. The
canonical/keystone dial worked only because its `orig_dst` IS the backend addr (a
single `daddr`-matched prerouting hop, no re-dial).

**The correction:** the mesh→mesh datapath requires an additive **output-hook
companion** to the inbound interception — a `type route hook output priority
mangle` nft chain (the `type route` re-lookup is load-bearing; a `type filter`
counter-test lands on the plaintext decoy — spike-proven) carrying a leg-S-dial
exemption head rule + a per-virt `ip daddr <workload_addr> tcp dport <port> meta
mark != 0x2 meta mark set 0x1 accept` divert, reusing the existing fwmark
(`0x1`), `ip rule`/`local`-route policy routing (UNCHANGED), and `MTLS_LEG_S_DIAL_MARK`
(`0x2`) exemption, plus an `IP_FREEBIND` sockopt on the leg-C listener so it can
bind the non-local `workload_addr` on the output path. The mechanism is
**proven, falsification-tested, and production-promotable** on a real Lima kernel
(`7.0.0-22-generic`, root) — spike
`docs/feature/dial-by-name-responder/spike/findings-output-hook-legb.md`,
PROMOTE-ratified by the user 2026-06-27
(`docs/feature/dial-by-name-responder/spike/wave-decisions.md` 2nd probe section).
The leg-C output-path binding shape is pinned to **`IP_FREEBIND` bind** (option A,
spike-proven; DNAT/redirect rejected as unproven and as losing the `getsockname`
orig-dst recovery the intercept design depends on).

This is a **READER→datapath correction, not a change to the DNS/resolve
contracts** (DDN-1..DDN-8, REV-2/3/4 — the stable-frontend direction, the
`by_frontend` key, fail-closed, single-owner allocator — are ALL unchanged). It
amends only the ADR's scope claim that the interception is untouched: the
dial-by-name mesh→mesh hop does touch the inbound interception surface, additively
and behind the same leg-S exemption that already governs recursion. **The full
production contract** — the `IP_FREEBIND` / `NFT_OUTPUT_CHAIN` consts, the
`ensure_shared_routing_infra` / `install_inbound_tproxy` deltas, the leg-C
`IP_FREEBIND`, and the load-bearing teardown-classifier widening
(`TproxyInterceptGuard` now reaps TWO rules per inbound install;
`per_workload_rule_handles_in_dump` must collect the `meta mark set` output rule
across BOTH chains or it leaks across restarts) — is pinned in the feature-delta
**§ "REV-5 / Datapath amendment — the output-hook leg-B interception companion"**
(`docs/feature/dial-by-name-responder/feature-delta.md`). A crafter implements TO
that section (CLAUDE.md "Implement to the design — never invent API surface"); no
surface beyond the `IP_FREEBIND` + `NFT_OUTPUT_CHAIN` consts and the widened guard
is sanctioned. **Merge-gate caveat:** the spike is on dev-Lima 7.x; the
output-hook + `type route` + policy-routing surface MUST be re-confirmed on the
pinned-6.18 appliance kernel at Tier-3/DEVOPS (ADR-0068).

## Changed Assumptions (REV-2, 2026-06-25)

### What changed and why

REV-1's "headless v1" contract assumed the *existing* per-workload canonical
address is stable, so DNS could answer it directly. Two code findings (research
SQ1/SQ2, every claim `file:line`-pinned) falsify that assumption:

- **SQ1 — `workload_addr` is per-`AllocationId`, NOT per-logical-workload.** It
  is `WORKLOAD_SUBNET_BASE.network() + slot*4 + 2` (`veth_provisioner.rs:528-532`)
  and the slot is a smallest-free scan keyed on `AllocationId`
  (`NetSlotAllocator::assign`, `:709-729`; held map `:673-678`). A backend cycle
  mints a NEW `AllocationId` → a (frequently different) slot → a different
  address. Answering it goes STALE on the exact event the user wants to
  eliminate.
- **SQ2 — `MtlsResolve` keys on the BACKEND addr** (`by_addr`,
  `mtls_resolve_adapter.rs:207-300`; rustdoc `:89-91`), so a query for a stable
  frontend addr would MISS → `NonMesh` → silent cleartext.

The fix is the convergent industry pattern (Cilium ClusterIP→backend at the
socket-LB datapath, research SQ4; K8s/CoreDNS/Istio, DNS-TTL research sub-q 4):
introduce a **stable per-`<job>` frontend addr** and **re-key `MtlsResolve` to
translate it → a current live backend**. Delivered via the existing nft-TPROXY
datapath — VIP-*shaped* but NOT the #61 XDP/`SERVICE_MAP`/`fdc2::/16`/#167
machinery (research Conflict 2: a re-scoping, not a contradiction).

### The superseded contract (quoted verbatim from the REV-1 Decision below)

> The headless return-shape (D-TME-10): the answered `A` addr is a
> **running-and-healthy** `service_backends` addr — **byte-identical** to the addr
> `MtlsResolve.resolve` recognizes AND classifies `Mesh`. This is forced by the
> shipped byte-consistency guarantee: `MtlsResolve` resolves a running-and-healthy
> backend → `Mesh` … No VIP, no #167, no translation layer.

(REV-1 § "The v1 DNS answer contract"; mirrored in DDN-2.)

### The replacement contract (REV-2)

The answered `A` addr is the **stable per-`<job>` frontend addr** that
`MtlsResolve` is **re-keyed to recognize AND translate** to a current
running-AND-healthy backend (never a miss → `NonMesh`/cleartext, never an
unhealthy backend → `MeshUnreachable`). Byte-consistency is RESTATED as: *the
answered frontend addr is byte-identical to the addr `MtlsResolve` recognizes,
and the translation always lands a `Mesh` backend.* The `Backend.healthy == true`
gate (REV-1 DDN-2) moves from "which addr we answer" to "which backend the
translation selects" (and "is this `<job>` resolvable at all"). A stable answer
makes the positive `A_RECORD_TTL_SECS=1` no longer load-bearing (TTL-moot by
construction, DNS research sub-q 4) — it MAY relax, a named optional v1
sub-decision, not auto-applied.

### Two forks — RATIFIED (no longer DRAFT)

Both sub-decisions are DECIDED by the user (2026-06-25). The options + full
rationale live in the feature-delta (REV-2 § "The genuinely-open design forks");
the ratified choice is locked here:

- **REV-1a = 1a-A (RATIFIED).** A NEW per-`<job>` `FrontendAddrAllocator`
  (sibling to `NetSlotAllocator`) carving from a distinct subnet block. Rejected:
  1a-B (re-key `NetSlotAllocator` per-`<job>` — conflates the per-instance netns
  slot with the per-workload frontend, breaks multi-replica / rolling-restart
  overlap) and 1a-C (deterministic hash → addr — birthday-bound collision risk
  for an addressing primitive). The allocator is keyed by `<job>`
  (`WorkloadId` / `MeshServiceName`), assigns a stable IPv4 from
  `WORKLOAD_FRONTEND_BASE` (pinned below), is bound on first running-AND-healthy
  backend for the `<job>`, retained across alloc cycles, released only when the
  *logical workload* is removed, and is boot-rebuilt from the running allocs
  (empty-on-boot rebuild, mirroring `NetSlotAllocator` — the Phase-1 precedent).
- **REV-1b = 1b-A (RATIFIED).** An additive `by_frontend: BTreeMap<FrontendKey,
  ServiceId>` map (`FrontendKey = (SocketAddrV4, Proto)` — Finding 1, the proto
  axis is carried so `tcp/53`/`udp/53` never collide) on `BackendIndex` + a
  `classify` translation arm reusing the
  existing healthy-set selection. Rejected: 1b-B (responder writes
  resolve-consumed translation — inverts enforcement→name-layer dependency) and
  1b-C (separate `FrontendResolve` wrapper — ceremony for one map lookup). The
  multi-healthy-backend selection rule is pinned (BLOCKER-2 below):
  **deterministic first-by-`Ord`** for v1.

### Pinned frontend subnet — `WORKLOAD_FRONTEND_BASE = 10.98.0.0/16`

The REV-1a frontend allocator carves from **`10.98.0.0/16`** (const
`WORKLOAD_FRONTEND_BASE`, distinct from both `WORKLOAD_SUBNET_BASE` and the
service-VIP range). The spike's candidate `10.96.0.0/16` was **REJECTED** — it
collides with the live service-VIP allocator (see § "Collision check" below).

**Two hard constraints the spike pinned (both satisfied by `10.98.0.0/16`):**

1. **MUST NOT overlap `WORKLOAD_SUBNET_BASE = 10.99.0.0/16`**
   (`veth_provisioner.rs:307`). Overlap would make a frontend addr on-link to
   some per-alloc `/30` (changing the carrying route from the per-netns default
   route the spike validated) AND catch it in the `service_map_hydrator`
   mesh-gate membership test (`Backend.addr ∈ 10.99.0.0/16`,
   `service_map_hydrator.rs:265-274`). `10.98.0.0/16` is disjoint. ✓
2. **MUST NOT be made on-link / owned by any other host route** — it must fall
   through the per-netns `default via <gateway>` route (the load-bearing route
   `WorkloadVethStep::AddDefaultRoute` already installs, BLOCKER-1 evidence). A
   non-on-link, non-`/30`-host addr in `10.98.0.0/16` follows the default route
   to the host-side veth gateway and is captured by the destination-blind egress
   nft-TPROXY exactly as the spike's `10.96.0.1` was. `10.98.0.0/16` is not
   on-link to any existing host route. ✓

#### Collision check (codebase + ADR grep, 2026-06-25)

The IPv4 reservation landscape, verified against live code:

| CIDR | Owner | Source | Overlaps `10.98.0.0/16`? |
|---|---|---|---|
| `10.99.0.0/16` | per-netns `/30` workload slots (`WORKLOAD_SUBNET_BASE`; first `/18` allocated, rest headroom) | `veth_provisioner.rs:307` | No |
| **`10.96.0.0/16`** | **service-VIP allocator DEFAULT (`VipRange::default()`), in effect on EVERY boot absent a `[dataplane.vip_allocator]` override (ADR-0049 Alt-E); live examples `10.96.0.2/.10/.11`** | `crates/overdrive-dataplane/src/allocators/vip_range.rs:204-234`; `vip_allocator_config.rs`; `dataplane_config.rs:257` | No |
| `fdc2::/16` | IPv6 ULA VIP prefix (#61 / whitepaper §11 — IPv6, and whitepaper is NOT SSOT) | `docs/whitepaper.md:1462` | No (IPv6) |
| `10.0.0.0/24`, `10.244.0.0/16` | test-fixture / journey-doc addresses only — NOT production reservations | `xtask/src/main.rs:923`; `udp-service-support` journey docs | No |

**Why `10.96.0.0/16` is rejected (the collision the dispatch told me to flag,
not pick blindly past):** `VipRange::default()` covers the **entire
`10.96.0.0/16`** (`vip_range.rs:232-234`: network `10.96.0.0`, prefix `16`) and
is the in-effect service-VIP allocation range on every boot when the operator
does not override `[dataplane.vip_allocator]`. Putting the dial-by-name frontend
allocator on the same `/16` would have two independent allocators carving
addresses from one block with no coordination — the addressing-collision defect
class (`development.md` § "One shared length ceiling" / `## Check-and-act must be
atomic`: two distinct things silently sharing one address). `10.98.0.0/16` is
disjoint from both the VIP range and the workload range — collision-free.

> **Naming note for DELIVER:** the const is `WORKLOAD_FRONTEND_BASE` (the
> per-`<job>` dial-by-name frontend block). It is NOT the `ServiceFrontend`
> VIP type (`crates/overdrive-core/src/dataplane/service_frontend.rs`, the
> udp-service-support `(vip, port, proto)` triple on the `10.96.0.0/16` VIP
> path). Two different "frontend" concepts; keep them lexically distinct.

### Decisions this REV supersedes / amends

- **DDN-2 (mapping / byte-consistency)** — SUPERSEDED: the `by_name` index maps
  `<job>` → the stable frontend addr (1a-A), not → backend-addr set; the healthy
  gate selects the translated backend (REV-1b), not the answered addr. **The
  health gate now ALSO governs lifecycle (Finding 2):** a transient zero-healthy
  `<job>` → the `name_index` WITHHOLDS the answer (→ NXDOMAIN), while the
  `FrontendAddrAllocator` RETAINS `F` (release only on logical-workload deletion).
  **The single-owner invariant (REV-2, the byte-consistency anchor):** the `<job>
  ↔ F` binding has exactly ONE owner — the `FrontendAddrAllocator` (1a-A). The
  DNS answer path (`name_index` / `answer_for`) and the resolve translate path
  (`MtlsResolve.by_frontend`) BOTH derive `<job> ↔ F` from the **same
  `FrontendAddrAllocator` instance**. This is the "one source" invariant extended
  to `F` (the sibling of the shipped "one source = `service_backends` rows, three
  readers" model — here the second single source is the allocator's `<job> ↔ F`
  binding). It is what makes the `F` the DNS path *answers* byte-identical to the
  `F` the resolve path *recognizes/translates*: a second allocator source would
  assign a different `F` to the same `<job>`, the answered `F` would miss
  `by_frontend`, and the connection would fail-closed (`MeshUnreachable`) — the
  addressing-divergence defect REV-2 exists to prevent. The Finding-3 ordered
  drain (below) is the coherence mechanism that exposes the SAME allocator-owned
  `F` to both projections in the right order. **(REV-3 amendment — see the REV-3
  AMENDMENT block at the top of this ADR: the WRITER of the `<job> ↔ F` binding
  is the deploy-time lifecycle assigner (`assign(<job>)` at declaration in the
  `POST /v1/jobs` Service arm; empty-on-boot converge-on-boot rebuild from
  declared-Service intent), NOT the ordered drain and NOT the DNS read path. The
  ordered drain only EXPOSES the allocator-owned `F` to both projections in
  order; it does not CREATE the binding. The earlier "the drain looks up / binds
  `<job> → F`" framing conflated a read with a write — the binding is created by
  the assigner at declaration, orthogonal to backend health.)**
- **D-TME-10 "headless / NO VIP / NO translation layer"** — SUPERSEDED: a stable
  IPv4 frontend IS VIP-shaped and DOES introduce a (single-map) translation in
  `MtlsResolve`, delivered via nft-TPROXY (NOT #61 XDP).
- **DDN-1 "the addr-keyed intercept index struct stays untouched"** —
  SUPERSEDED: under 1b-A the struct is **extended additively** (`by_frontend:
  BTreeMap<FrontendKey, ServiceId>` where `FrontendKey = (SocketAddrV4, Proto)` =
  `(F, listener.port, listener.protocol)` per Finding 1 — the proto axis is
  carried so `tcp/53` and `udp/53` never collide on one key, + a THREE-way
  `classify` arm: `by_frontend` hit → translate;
  frontend-subnet miss → `MeshUnreachable` fail-closed per Finding 3; general miss
  → today's `by_addr` fall-through); the user ratified the in-place extension by
  choosing the thin path. The `<job> ↔ F` binding `by_frontend` is keyed on comes
  from the **single `FrontendAddrAllocator` instance** (the single-owner invariant
  pinned under DDN-2 above) — `by_frontend` does NOT spin up a second `<job> → F`
  source; the same allocator handle the `name_index` reads supplies the `F` here.
  The DNS↔resolve coherence is option (b) — a single ordered drain updates
  `by_frontend` before `name_index` (Finding 3), both projections reading the SAME
  allocator-owned `F`. The re-keyed contract (key type + first-by-`Ord` selection +
  fail-closed-on-subnet-miss arm) MUST be pinned in the `MtlsResolve` trait
  docstring + an equivalence test (`development.md` § "Trait definitions specify
  behavior").
- **DDN-3 / DDN-4 / DDN-5 / DDN-6 / DDN-7 / DDN-8** — UNCHANGED (codec, pure
  `answer_for` seam, bind/fallback, composition root, `MeshServiceName`,
  negative-TTL SOA are all addr-agnostic).

### Blocker resolutions (REV-2 final)

- **BLOCKER-1 — dataplane capture of the frontend subnet — RESOLVED → WORKS.**
  A Tier-3 spike on a real kernel (`7.0.0-22-generic`) confirmed the open
  dataplane-routing question:
  `docs/feature/dial-by-name-responder/spike/findings-blocker1-frontend-addr-capture.md`.
  Verdict: a non-`/30` frontend addr (`10.96.0.1` in the probe) **routes out the
  workload netns via the per-netns `default via <gateway>` route production
  already installs** (`veth_provisioner.rs` `WorkloadVethStep::AddDefaultRoute`),
  is **captured by the destination-blind egress nft-TPROXY** (`install_outbound_tproxy`,
  `mtls_intercept.rs`), and `orig_dst` is recovered **verbatim**. Negative
  control (default route removed) → `ENETUNREACH`. **No new routing/capture
  dataplane work is needed — REV-2 stays "thin."** The capture is
  destination-blind, so the verdict holds for any non-on-link addr; the pinned
  `10.98.0.0/16` (above) inherits it directly. (The probe used `10.96.0.1` as a
  routing test vector ONLY — that does NOT make `10.96.0.0/16` the production
  subnet; the production subnet is the collision-checked `10.98.0.0/16`.)
- **BLOCKER-2 — multi-replica selection rule (1b-A) — RESOLVED → pinned.** When
  a `<job>` has >1 running-AND-healthy backend, `resolve(F)` selects the
  **deterministic first-by-`Ord`** healthy backend for v1. v1 is single-replica
  today, so the choice is degenerate, but a deterministic tie-break keeps DST
  replay-equivalence (`testing.md` § "Tier 1" — seed → bit-identical trajectory)
  intact and is mutation-gate-able. Round-robin / load-aware selection is a
  named future churn-load concern, NOT v1 (it introduces per-connection
  non-determinism that the DST harness would have to model). The re-keyed
  `resolve` contract — including this selection rule — MUST be pinned in the
  `MtlsResolve` trait docstring + an equivalence test (`development.md` § "Trait
  definitions specify behavior").
- **#61 corrected-scope statement** still needs user approval before any GitHub
  edit (the orchestrator relays; this ADR does NOT edit the issue). See
  feature-delta REV-2 § "#61 reconciliation".

### REV-2 contract tightenings (design-review revision, 2026-06-25 — 3 High findings)

A review of the REV-2 design itself returned three High findings. The REV-2
DIRECTION (stable `10.98.0.0/16` frontend + `MtlsResolve` translation) is
UNCHANGED and not in question; these tighten the *executable contract* so the
artifact rewrite cannot implement an ambiguous or racy version. Every claim is
grounded on live code (`file:line`).

- **Finding 1 — the `by_frontend` key carries the proto axis: it is
  `(F, listener.port, Proto)`, NOT a bare `SocketAddrV4` (which is ip+port only)
  and NOT a bare `Ipv4Addr`.** Two facts force this, both grounded on PRODUCTION
  code:
  - `MtlsResolve::resolve` receives `orig_dst: SocketAddrV4`
    (`mtls_resolve_adapter.rs:521`) — ip+port. `ServiceId` is content-derived from
    `(vip, port, proto, purpose)` (`ServiceId::derive`, `id.rs:1189`), where the
    **proto axis is real** (`Proto { Tcp, Udp }`, `as_u8()` → `6`/`17`,
    `backend_key.rs:70-79`). The backend row carries the listener port —
    `Backend.addr = SocketAddr::new(IpAddr::V4(workload_addr ⟂ host_ipv4),
    listener.port.get())` (`backend_discovery_bridge.rs:364-367`, the PRODUCTION
    `reconcile` body) — but NOT the proto.
  - **A `SocketAddrV4` key cannot distinguish `tcp/53` from `udp/53`.** The
    `ServiceId` *value* only disambiguates AFTER the lookup; two same-`(F, port)`
    rows on different L4 protos collide on the `SocketAddrV4` key BEFORE the value
    is ever read. The prior "no schema change" framing was therefore false and is
    removed (below + feature-delta).
- **PRODUCTION evidence the v1 path is NOT TCP-only at the row-producing layer
  (so option (a) is not groundable; option (b) is chosen).** The path that
  POPULATES the `service_backends` rows `by_frontend` would fold is **proto-aware
  end-to-end and UDP-admissible today**:
  - Admission parses `"udp" => Proto::Udp` (`aggregate/mod.rs:567-568`;
    `workload_spec.rs:1008-1009`; the listener spec carries `protocol: Proto`,
    `workload_spec.rs:547`).
  - The bridge's PRODUCTION `reconcile` (`backend_discovery_bridge.rs:344-371`)
    iterates **every** `desired.desired.listeners` entry and emits a `Backend` row
    **regardless of `listener.protocol`** — there is NO production proto filter on
    this path. (`backend_discovery_bridge.rs:485` — `protocol: Proto::Tcp` — is a
    `#[cfg(test)] mod tests` helper `fn listener(...)`, line 451/481; it is
    **test-only** and proves nothing about production. The prior round's reliance
    on it is the error this revision corrects.)
  - The hydrator's projection threads `proto: listener.protocol`
    (`service_map_hydrator.rs:130`) and the C3 guard (`:59-80`, `:120`)
    **refuses to default to `Proto::Tcp`** — `ServiceProjectionError::NoListenerProto`
    rather than a silent TCP coercion (ADR-0060). A UDP listener with running
    backends is representable in production and the existing `udp-service-support`
    machinery (per-proto reverse-NAT, ADR-0060) exercises it.
  - The existing `BackendIndex.by_addr: BTreeMap<SocketAddrV4, …>`
    (`mtls_resolve_adapter.rs:214`) is itself proto-blind; a new
    `by_frontend: BTreeMap<SocketAddrV4, ServiceId>` would inherit the **same**
    tcp/53-vs-udp/53 collision hole.
- **CHOSEN: option (b) — widen the key to carry proto.** `by_frontend:
  BTreeMap<FrontendKey, ServiceId>` where `FrontendKey = (SocketAddrV4, Proto)`
  (`(F, listener.port, listener.protocol)`). Collision-free by construction;
  matches `ServiceId::derive`'s real proto axis and the row's `listener.protocol`.
  Option (a) ("v1 TCP-only via an explicit production filter") is **REJECTED**:
  it has no production code to cite (no such filter exists), and adding one would
  *fight* the C3 guard that deliberately refuses proto coercion — i.e. designing a
  regression onto the proto-aware path, not honoring it. One frontend IP `F`
  fronting N distinct `(port, proto)` listeners yields **N distinct entries**:
  `(F, p1, Tcp) → ServiceId_1`, `(F, p2, Udp) → ServiceId_2`, … The `ServiceId`
  VALUE is the one already on the folded `service_backends` row, not a
  re-derivation.
  **NOTE on the resolve key plumbing (DELIVER):** `resolve` receives only
  `orig_dst: SocketAddrV4` today (`mtls_resolve_adapter.rs:521`). Keying
  `by_frontend` on `Proto` requires the captured proto to reach `classify`. In v1
  the dataplane intercept (Path-A) is **TCP-only at the worker layer** — the
  outbound capture accepts a TCP listener and dials `std::net::TcpStream::connect`
  (`accept_outbound_and_recover_orig_dst` + `spawn_cleartext_passthrough`,
  `mtls_intercept_worker.rs:792-794`, `:1187`), so every captured `orig_dst` is a
  TCP dial. The resolve key therefore carries `Proto::Tcp` for every v1 lookup
  *because the capture is TCP*, NOT because the index dropped the axis. When the
  intercept path later captures UDP, the `Proto` axis is already present and
  `tcp/53`/`udp/53` are distinguished with **no `by_frontend` schema change** —
  the widening is in the *capture/plumbing* (surface the captured proto into the
  resolve call), never in the index key, which carries it from day one. (Full
  contract: feature-delta § "Frontend-key contract".)
- **Finding 2 — `F` is per-LOGICAL-WORKLOAD: WITHHOLD on zero-healthy, RELEASE
  only on deletion.** REV-2 was self-contradictory (1a-A "release only when the
  logical workload is removed" vs 03-01 "the allocator releases OR the index
  withholds"). Pinned: on a *transient* zero-healthy-backend state the
  `name_index` **WITHHOLDS the DNS answer** (→ NXDOMAIN + 1 s SOA, consistent with
  the DDN-2/DDN-8 fail-honest negative contract); the `FrontendAddrAllocator` does
  **NOT** release `F`. `F` is retained across alloc cycles AND zero-healthy
  windows; `release(<job>)` fires **only** on logical-workload deletion. Releasing
  `F` on a transient zero-healthy state would destroy the stability property and
  reintroduce the SQ1 stale-cached-`F` failure — explicitly rejected. (Full
  contract: feature-delta § "Frontend lifecycle contract".)
- **Finding 3 — the DNS↔resolve race is made non-exploitable (fail-CLOSED for
  mesh).** An answered `F` must never miss `MtlsResolve` — a miss is `NonMesh` →
  cleartext `PassThrough` (`mtls_intercept_worker.rs:1112`; `spawn_cleartext_
  passthrough` `:1180`), a fail-OPEN regression. Because `name_index` (DNS) and
  `by_frontend` (resolve) are separate readers, two complementary mechanisms are
  pinned, **both required**:
  > **REV-4 supersession (2026-06-26):** sub-bullet **(i) below is SUPERSEDED** —
  > there is no single ordered drain (the implementation has two independent
  > single-owner drains reading one allocator,
  > `mtls_resolve_adapter.rs:55-57` / `name_index.rs:41-43`). The enforced
  > coherence is **byte-identity of `F` via the single `FrontendAddrAllocator`**
  > (DDN-2); the temporal ordering barrier was an availability nicety, not a
  > security invariant. Sub-bullet **(ii) is unchanged and carries the security
  > half** — it holds regardless of inter-drain timing. See the REV-4 AMENDMENT
  > block at the top of this ADR; S-DBN-COHERENCE-01 is rewritten accordingly.
  - **(i) Ordering (coherence option (b), CHOSEN over (a)) — SUPERSEDED by REV-4
    (see the supersession note above; retained for the audit trail):** a SINGLE
    ordered drain updates `by_frontend` (+ a healthy backend) BEFORE `name_index`
    exposes `F`. The readers are projections off ONE ordered apply over the shared
    `service_backends` rows (the "one source, three readers" model — DDN-1 sibling
    readers, the single-owner drain `mtls_resolve_adapter.rs:56-87`), so a
    write-time barrier is the natural and stronger shape; (a) is the same idea as
    a per-query read gate and reintroduces a cross-reader coupling DDN-1 avoids.
    **The `F` both projections expose is the SAME allocator-owned binding** — the
    ordered drain looks up `<job> → F` from the single `FrontendAddrAllocator`
    instance (the single-owner invariant under DDN-2), binds it into `by_frontend`
    in STEP A, then exposes the SAME `F` to `name_index` in STEP B. Neither
    projection derives `F` from anywhere else; the `service_backends` rows supply
    *liveness* (is there a running-AND-healthy backend for `<job>`?), the allocator
    supplies *which `F`*. Two single sources, one drain reading both.
  - **(ii) Fail-closed-on-frontend-subnet-miss (ADOPTED structural defense):**
    a captured connection whose `orig_dst.ip() ∈ WORKLOAD_FRONTEND_BASE
    (10.98.0.0/16)` that MISSES `by_frontend` classifies **`MeshUnreachable`
    (refuse, NO cleartext)**, NOT `NonMesh`. This is sound precisely because the
    live rustdoc requires a *general* miss stay `NonMesh` for "legitimate external
    / non-mesh egress" (`mtls_resolve_adapter.rs:128-130`) — the subnet-membership
    discriminator distinguishes the two: `10.98.0.0/16` is DEDICATED to mesh
    frontends, so a miss there is a mesh dial that is early (race) or to a
    withdrawn `<job>`, never legitimate cleartext. A miss OUTSIDE the subnet keeps
    today's behaviour verbatim.

  **Net guarantee:** (i) closes the steady-state race; (ii) makes any residual
  race non-exploitable — worst case is a refused connect + client retry, never
  silent cleartext to a should-be-mesh peer. The dataplane fails **closed** for
  mesh. The re-keyed `MtlsResolve` trait docstring MUST pin all three: the
  `(F, listener.port, Proto)` key type, the first-by-`Ord` selection rule, and the
  fail-closed-on-frontend-subnet-miss arm; an equivalence test enforces it
  (`development.md` § "Trait definitions specify behavior"). (Full contract:
  feature-delta §§ "Frontend-key contract" / "Frontend-subnet coherence
  contract".)

**One stance was corrected against live code (2nd-round Finding 1).** The 1st-round
"`by_frontend` keyed by a bare `SocketAddrV4`, TCP-only with no schema change"
framing was falsified: a `SocketAddrV4` cannot distinguish `tcp/53` from `udp/53`,
and the cited "production TCP-only proof" (`backend_discovery_bridge.rs:485`) is a
`#[cfg(test)]` test helper, not production. The PRODUCTION row-producing path is
proto-aware and UDP-admissible (`aggregate/mod.rs:567-568`;
`service_map_hydrator.rs:130` + the C3 guard `:120`; the bridge `reconcile`
`:344-371` applies NO proto filter). The key now carries the proto axis —
`FrontendKey = (SocketAddrV4, Proto)` (option (b)) — collision-free by
construction; option (a) (a TCP-only production filter) was rejected as having no
production code to cite and as fighting the C3 guard. The other two fixes are
confirmed-and-safe against live code: `ServiceId::derive` carries proto
(`id.rs:1189`), and the live `NonMesh → PassThrough` path plus the "a general miss
must stay `NonMesh`" rustdoc (`mtls_resolve_adapter.rs:128-130`) both confirm AND
make safe the Finding-3 subnet-scoped fail-closed arm. All fixes are grounded on
live code, not designed around it.

### Active-termination posture (corrected framing — settled)

NO `sock_destroy` in the thin path — it is the wrong tool for a *terminating
proxy*. Backend death propagates through the per-connection pump task +
`TCP_USER_TIMEOUT` / keepalive (`mtls_intercept_worker.rs:34`). The v1 obligation
is a Tier-3 churn test (cycle a backend mid-connection → prompt reset bounded by
`TCP_USER_TIMEOUT`, not an indefinite hang) + confirming `TCP_USER_TIMEOUT` is
tuned sanely. `sock_destroy` belongs to #61's future connect-time XDP-VIP-LB (no
userspace proxy in path), NOT here. Even Cilium defaults TCP socket-termination
OFF (research SQ5.3).

---

> **The REV-1 Decision text below is RETAINED for the audit trail** (ADRs
> supersede, never silently rewrite). Read it through the lens of the Changed
> Assumptions above: the answer is a stable frontend addr, not a backend addr,
> and `MtlsResolve` translates it.

**Builds on ADR-0071** (transparent-mTLS enrollment, Path A) and its
name-layer integration (Q5a / D-TME-9/10/11). ADR-0071 SHIPPED the
`resolv.conf` injection (each per-netns `/etc/resolv.conf` points at the
per-netns gateway) and named the **node-local DNS responder daemon** as a
DEPENDENCY *"(#61 daemon, NOT built here)"*. The transparent-mTLS arc was
then finalized with the responder reframed from #61 (VIP path) onto **#243**
(headless path). **This ADR designs that responder** — the third reader of
the `service_backends` observation surface, alongside the outbound resolve
(ADR-0071 `ServiceBackendsResolve`) and the inbound install (#241).

**What this ADR does NOT touch — the locked core of ADR-0071/0069/0070 is
UNCHANGED**: the `MtlsResolve` driven port, the `ServiceBackendsResolve`
address-keyed intercept index, the `MtlsEnforcement` 4-method contract, the
nft-TPROXY both-directions interception, the netns/veth provisioner, and the
`resolv.conf` injection. The responder is a **new READER** of an existing
observation surface; it adds no enforcement surface and modifies no
security-critical path.

## Context

`overdrive deploy` provisions each exec workload into its own netns with an
injected `/etc/resolv.conf` whose `nameserver` is the per-netns gateway
(`plan.host_addr`, ADR-0071 D-TME-9). **Nothing answers there.** An
unmodified workload's `getaddrinfo("<peer>.svc.overdrive.local")` reaches the
stub resolver, the query egresses the veth toward the gateway, and times out
— name resolution fails in every deploy. The dial-by-name leg the
transparent-mTLS arc deferred (#236) is the gap; an *unmodified* workload
cannot even **initiate** a by-name connection, so the mesh's
"every-flow-is-identity-bearing" promise is unreachable from ordinary code.

The mechanism's one load-bearing, no-Tier-2-backstop assumption — *can ONE
host-side listener receive and answer DNS sent to N distinct per-netns gateway
addresses?* — was SPIKED (Slice 00, `docs/feature/dial-by-name-responder/spike/`,
real kernel under Lima as root). **Verdict: WORKS (PROMOTE).** One
`0.0.0.0:53` wildcard socket (`SO_REUSEADDR` + `IP_PKTINFO`) received and
answered queries to both per-netns gateways, validated through the real
`getaddrinfo`/`getent` path from both netns, replies source-pinned to the
queried gateway via `ipi_spec_dst`. The spike pins the production-design
constraints recorded under **Consequences → Design constraints inherited from
the spike**.

### The v1 DNS answer contract (the canonical table this ADR honors)

| Query | Name has ≥1 running-AND-healthy IPv4 backend | Name has 0 running-and-healthy backends\* |
|---|---|---|
| `A` | **NOERROR + A** (the running-and-healthy IPv4 addr) | **NXDOMAIN** (+ 1 s negative-TTL SOA) |
| `AAAA` | **NOERROR / NODATA** (ANCOUNT=0, same SOA in authority) | **NXDOMAIN** (+ 1 s SOA) |

\* *0 running-and-healthy backends covers declared-but-not-running, unhealthy /
not-ready, and unknown names alike — v1 does not distinguish them (the responder
reads only the running-and-healthy set, § DDN-2). A stale / cached / guessed /
unhealthy address is NEVER returned.*

The headless return-shape (D-TME-10): the answered `A` addr is a
**running-and-healthy** `service_backends` addr — **byte-identical** to the addr
`MtlsResolve.resolve` recognizes AND classifies `Mesh`. This is forced by the
shipped byte-consistency guarantee: `MtlsResolve` resolves a running-and-healthy
backend → `Mesh`, but a `Backend.healthy == false` backend →
`MeshUnreachable` (fail-closed). An unhealthy addr is therefore NOT "an addr
`MtlsResolve` recognizes" as reachable — answering it would point the dialer at
a backend the intercept path refuses, violating byte-consistency. Every answered
addr resolves to `Mesh`, never `MeshUnreachable`. No VIP, no #167, no
translation layer.

## Decision

A new **`DnsResponder`** host adapter in `overdrive-control-plane`
(`adapter-host`), owned by the composition root (`run_server`), answering
`<job>.svc.overdrive.local` from a **sibling name-keyed reader** over the
`ObservationStore`. **Eight sub-decisions, `DDN-1`..`DDN-8`** — one stable ID per
concern (sibling-reader / name-mapping / DNS-codec / DST-seam / bind-strategy /
composition / name-grammar / negative-TTL). Each cites the user-ratified Pass-1
decision point it implements (A1/B/C1/D1/E1/F/H1; the `F` point spanned two
concerns — the name→backend mapping AND the `MeshServiceName` newtype — so it
splits cleanly into `DDN-2` and `DDN-7`). The same `DDN-*` IDs are used in the
feature-delta decisions table. Each sub-decision's alternatives are below.

### DDN-1 (ratified point A1) — a NEW sibling name-keyed reader; do NOT extend the addr-keyed intercept index

The responder maintains its OWN `by_name` index
(`BTreeMap<MeshServiceName, BTreeSet<SocketAddrV4>>`) over the
**running-AND-healthy** `service_backends` set (the gate is mandatory, § DDN-2),
maintained by the **same List-then-Watch + relist-on-`Lagged` +
single-owner-drain + `probe()`** pattern as `ServiceBackendsResolve` (ADR-0071
D-TME-11) — but as an independent struct, NOT a widening of the addr-keyed
`ServiceBackendsResolve`/`MtlsResolve` intercept index. It reads the **same
`service_backends` rows from the same `ObservationStore`** (byte-consistency is a
property of the shared rows, not a shared struct — § "Byte-consistency").

**Alternatives considered:**
- **A2 — extend the addr-keyed `ServiceBackendsResolve` index with a
  secondary `by_name` map (REJECTED).** It would couple the name layer to the
  security-critical enforcement path: a name-layer change (a grammar tweak, a
  qtype arm) would edit the struct whose point-lookup the per-connection mTLS
  decision depends on, and the two consumers have genuinely different keys
  (`addr` vs `name`). The intercept index must stay untouched — A1 reads the
  *same source rows* (byte-consistency guaranteed at the row, §
  "Byte-consistency") without sharing the *struct*.
- **A1 (CHOSEN)** — a sibling reader. Both readers fold the *same*
  `service_backends` rows from the *same* `ObservationStore` via the *same*
  List-then-Watch contract; consistency is a property of the shared rows, not
  of a shared in-RAM structure. The enforcement path is provably unchanged
  (no edit to `mtls_resolve_adapter.rs`).

### DDN-2 (ratified point F, mapping concern) — name→backend mapping = VERIFIED mapping (i), gated running-AND-healthy

`<job>` is derived from `Backend.alloc: SpiffeId`. The SVID path is
`spiffe://overdrive.local/job/<WorkloadId>/alloc/<id>`
(`SpiffeId::for_allocation`), and `WorkloadId` **is** the deploy
`[service].id` — so `<job>.svc.overdrive.local`'s `<job>` label equals the
`WorkloadId` segment of the SVID path. `service_backends` rows are built by
`BackendDiscoveryBridge` from `actual.actual.running` **only**
(`backend_discovery_bridge.rs` ~351), so the "∩ running" filter holds **by
construction** — the index need only group the rows it already reads by their
SVID job segment. The `by_name` index **MUST additionally gate on
`Backend.healthy == true`** (the running-AND-ready set), matching the intercept
index's `Mesh` classification. This is NOT optional: `MtlsResolve` resolves a
running-and-healthy backend → `Mesh` but a `Backend.healthy == false` backend →
`MeshUnreachable` (fail-closed, `mtls_resolve_adapter.rs:124-135`), so an
unhealthy addr is NOT an addr `MtlsResolve` recognizes as reachable — answering
it would point the dialer at a backend the intercept path refuses, violating
byte-consistency. Every answered addr therefore resolves to `Mesh`, never
`MeshUnreachable`. A name with no running-and-healthy backend → NXDOMAIN
(§ contract table).

**Alternatives considered:**
- **A declared-service view keyed by `[service].id` directly (REJECTED for
  v1)** — would require a second observation surface (the declared, not the
  running-and-healthy, set) to distinguish declared-but-empty (→ NODATA) from
  unknown (→ NXDOMAIN). v1 collapses declared-but-not-running, unhealthy, and
  unknown all to NXDOMAIN by reading only the running-and-healthy set; a
  declared view is a named future refinement, not v1 (see § "Out of scope").
- **A new string-munging helper that re-parses `<job>` out of the SVID path
  (DEFERRED — surfaced as OQ-1).** No existing accessor on `SpiffeId` extracts
  the job segment; rather than improvise a public helper here, the exact
  accessor shape is left to DISTILL/DELIVER (see § "Open questions"). The
  mapping is verified; the *accessor signature* is the only open detail.

### DDN-3 (ratified point B) — hickory-dns for the wire codec; our own IP_PKTINFO socket loop

Use **`hickory-proto`** (Apache-2.0/MIT) for the DNS wire codec
(`Message`/`Header`/`Query`/`Record`/`RData::A`/`RData::SOA`/`ResponseCode`/
name-compression/EDNS). Do NOT hand-roll the DNS encoder/decoder.

**The `hickory-server`-vs-`hickory-proto` source-pinning verdict: use
`hickory-proto` codec + our OWN socket loop.** The spike empirically settled
this: source-pinning the reply to the *queried gateway* (`ipi_spec_dst` =
captured `IP_PKTINFO`) on a single multi-homed wildcard `0.0.0.0:53` socket is
MANDATORY — `getaddrinfo`/glibc rejects a reply whose source ≠ the queried
server, so a non-pinned reply fails resolution silently. `hickory-server`'s
UDP server does not expose per-packet reply-source control on a wildcard
multi-homed socket (it owns its own socket and reply path), so it cannot
satisfy the `ipi_spec_dst` requirement. We therefore own the
`recvmsg`/`sendmsg` `IP_PKTINFO` loop (the spike-validated shape) and use
`hickory-proto` purely as the byte codec behind a pure `answer_for` seam.

**Alternatives considered:**
- **Hand-rolled DNS wire codec (REJECTED)** — name compression, EDNS, the SOA
  RDATA layout, and qtype/qclass parsing are a well-specified but error-prone
  surface; an OSS-first, well-maintained codec (`hickory-proto`) removes a
  whole bug class. CLAUDE.md / OSS-first.
- **`hickory-server`'s `RequestHandler` (REJECTED — verified)** — it does not
  give per-packet `ipi_spec_dst` control on a multi-homed wildcard socket,
  which the spike proved mandatory for `getaddrinfo` acceptance. Adopting it
  would either break the `getent` acceptance signal or force N per-netns
  sockets where one wildcard suffices on the validated node config.

### DDN-4 (ratified point C1) — the DST seam is a pure `answer_for` + a separately-proptested encoder; NO port trait, NO Sim adapter

The deterministic, testable core is a **pure free function**
`answer_for(name: &MeshServiceName, qtype: hickory_proto::rr::RecordType, index:
&NameIndex) -> NameAnswer` plus a **separately proptested** DNS encoder
(`NameAnswer → Vec<u8>` via `hickory-proto`). `NameAnswer` is the pinned pure
result `enum NameAnswer { Records(Vec<SocketAddrV4>), NoData, NxDomain }` — its
variant names and the `qtype` type are PINNED in DESIGN (the qtype reuses
`hickory_proto::rr::RecordType` rather than minting a redundant local `QType`,
keeping the wire vocabulary single-source behind the `wire.rs` ACL — `NameAnswer`
itself stays hickory-free, only `qtype` crosses the boundary). The
socket/`IP_PKTINFO` recv/send loop is the irreducibly-Tier-3 shell the spike
proved is the only honest signal; it is NOT hidden behind a port trait.

**Alternatives considered:**
- **A `NameResponder` port trait + a `SimNameResponder` adapter (REJECTED).**
  The responder has no second production impl and no scheduling/clock concern
  the trait surface exists to inject; a Sim adapter of the *socket* would
  simulate exactly the part the spike proved cannot be honestly simulated
  (`IP_PKTINFO` multi-homing is real-kernel-only, no Tier-2 backstop). The
  port-trait machinery would add ceremony with no decoupling benefit and a
  *false* confidence (a green sim of a substrate that lies). The honest split
  is: pure `answer_for` (proptest + unit), pure encoder (proptest round-trip),
  socket loop (Tier-3 `getent` only).
- **No seam at all — answer inline in the recv loop (REJECTED).** Would make
  the DNS-contract logic (the canonical table) untestable without a real
  socket; the pure `answer_for` seam is the mutation-gate target and the unit
  surface.

### DDN-5 (ratified point D1) — bind `0.0.0.0:53` wildcard first; fall back to N per-gateway-addr sockets on EADDRINUSE

`probe()`/bind tries ONE `0.0.0.0:53` wildcard socket (`SO_REUSEADDR`,
`IP_PKTINFO`) first. On `EADDRINUSE` (an appliance image that holds a wildcard
`:53`) it falls back to N per-gateway-addr sockets in one process.

**The fallback gateway-set source is PINNED (no longer deferred — closes the
prior OQ-3):** the live per-netns gateway set is the
`NetSlotAllocator` held on `AppState` (`state.net_slot_allocator`,
`veth_provisioner::NetSlotAllocator`) — the SAME single source of truth that
already owns every live `alloc → NetSlot` binding and that the action-shim C3
site reads when it provisions a netns. The responder derives the gateway addr
for each currently-assigned slot via the existing pure
`responder_addr_for_slot(slot) -> Ipv4Addr` (`veth_provisioner.rs`, the SAME
arithmetic `derive_workload_netns_plan` uses for `plan.host_addr`/`plan.gateway`
— `WORKLOAD_SUBNET_BASE.network() + slot*4 + 1`). `DnsResponder::new` therefore
takes a concrete `NetSlotAllocator` handle (cheap `Arc`-shared clone; no new
port trait, no second source of slot truth). The wildcard path never touches it;
it is read ONLY when the fallback fires.

**The `NetSlotAllocator` (`slots`) is DISTINCT from the `FrontendAddrAllocator`
— two different allocators, two different concerns.** `slots` is the per-netns
reply-source-pin / per-addr fallback allocator (DDN-5; the gateway-set source for
the EADDRINUSE fallback). The `FrontendAddrAllocator` is the `<job> ↔ F` SSOT
(1a-A; the single-owner invariant under DDN-2) the responder reads to *answer*
`F`. So `DnsResponder::new` takes BOTH, as separate parameters:

```rust
impl DnsResponder {
    fn new(
        store: Arc<dyn ObservationStore>,
        clock: Arc<dyn Clock>,
        slots: veth_provisioner::NetSlotAllocator,   // DDN-5 per-addr fallback source (UNCHANGED)
        frontend: FrontendAddrAllocator,             // NEW (1a-A): the single <job> ↔ F owner
                                                     //   the name_index answers F from — the SAME
                                                     //   instance MtlsResolve.by_frontend derives F from
    ) -> Self;
    async fn probe(&self) -> Result<(), DnsResponderError>;
    async fn serve(self: Arc<Self>);
}
```

`frontend` is a cheap `Arc`-shared clone of the ONE `FrontendAddrAllocator` the
composition root constructs (DDN-6 below); it is the same instance injected into
the re-keyed `MtlsResolve`, so the `F` the `name_index` answers is byte-identical
to the `F` `by_frontend` recognizes. Adding a second `FrontendAddrAllocator` here
(or letting the responder derive `F` independently) is the addressing-divergence
defect the single-owner invariant forbids.

**Dynamic re-bind lifecycle (gateways come/go as allocs start/stop):**
`NetSlotAllocator` is a snapshot-on-demand map (`snapshot() ->
BTreeMap<AllocationId, NetSlot>`, mutated by `assign`/`release`/`adopt`); it
exposes **no change-subscription**. So the fallback path does **not** subscribe —
it **re-derives the desired gateway set on the converge cadence**: each tick it
computes `desired = { responder_addr_for_slot(slot) : slot ∈
net_slot_allocator.snapshot().values() }`, diffs it against the
currently-bound per-addr socket set (a `BTreeMap<Ipv4Addr, OwnedSocket>` the
responder holds), **binds a new per-addr `:53` socket for each added gateway and
drops the socket for each removed gateway** (idempotent add-if-missing /
drop-if-absent — the same converge-on-boot discipline `reconcilers.md` Bar-1
describes). This keeps the per-addr socket set tracking the live slot set with no
new event surface. Steady-state on the wildcard path pays nothing (the diff loop
runs only after a fallback `EADDRINUSE`).

**Alternatives considered:**
- **Wildcard-only, no fallback (REJECTED — too coupled to node image).** The
  spike showed the wildcard works on the dev-Lima config (systemd-resolved
  binds `127.0.0.53/54:53` as *specific* addresses, so a wildcard coexists),
  but flagged that an appliance image holding a *wildcard* `:53` would
  `EADDRINUSE`. The fallback is "a few lines" of insurance against a node-image
  coupling and was implemented (not fired) in the spike.
- **N per-addr sockets only, never wildcard (REJECTED — wasteful).** One
  wildcard socket is sufficient and simpler on the validated config;
  per-gateway-addr binding scales with allocation count and re-binds on every
  slot change. Wildcard-first keeps the steady-state simple and resorts to
  per-addr only when forced.

### DDN-6 (ratified point E1) — `run_server` owns the responder; wire → probe → use, gated by the real-dataplane block

`run_server_with_obs_and_driver` constructs the responder AFTER
`resolve.probe()`, runs `responder.probe()` (which binds + seeds the index via
List), refuses boot with a structured `health.startup.refused` on bind/List
failure (the Earned-Trust gate), `tokio::spawn`s the serve loop, and holds the
`JoinHandle` for shutdown — gated by the SAME `if mtls_worker.is_some()` real-
dataplane block that gates the netns/intercept path (a no-op on a non-mTLS /
`SimDataplane` boot, where no per-alloc netns exist to inject into).

**The composition root constructs ONE `FrontendAddrAllocator` and injects the
SAME handle into BOTH paths (the single-owner invariant, DDN-2).** Inside the
`if mtls_worker.is_some()` block, ordered so the one allocator reaches both
readers:

1. Construct the single `FrontendAddrAllocator` (1a-A, empty-on-boot rebuild) and
   share it on `AppState` beside the existing `net_slot_allocator`.
2. Construct the re-keyed `MtlsResolve` (1b-A) so its single ordered drain is fed
   the `<job> → F` binding from THAT allocator (the `by_frontend` translate path
   recognizes the same `F`). The probed-Ok resolve is then moved into the worker
   (the worker-held resolve and the responder share the one allocator, so
   `by_frontend` and `name_index` agree on `F`).
3. Construct the `DnsResponder` AFTER `resolve.probe()` with `store` +
   `config.clock` + the `state.net_slot_allocator` handle (DDN-5 fallback source)
   **+ the SAME `FrontendAddrAllocator` handle** (so the `name_index` answers the
   allocator-owned `F`). Then `responder.probe()` → spawn → hold `JoinHandle`.

The socket loop / bind / `IP_PKTINFO` / Earned-Trust gate are UNCHANGED; the
composition-root delta is the single `FrontendAddrAllocator` construction +
its injection into BOTH the re-keyed resolve and the responder. The shipped
construction site supports this: `resolve` is an `Arc<dyn MtlsResolve>`
(`lib.rs:1910`) moved into the worker (`:1937`), and the allocator is `Arc`-shared,
so one `let frontend = FrontendAddrAllocator::new()` binding can feed the resolve
before the move AND survive (held on `AppState`) to the responder construction
after `AppState::new` (`:1944`) — no contradiction with the real composition root.

**Alternatives considered:**
- **Spawn the responder lazily on first deploy / outside the composition root
  (REJECTED).** Violates wire→probe→use (principle 12): a responder that binds
  lazily cannot refuse boot on an unbindable port or an unreadable store, so
  the node would start and *then* fail to answer — the silent-degradation
  footgun the Earned-Trust gate exists to remove.
- **A standalone daemon process (REJECTED — D-TME-11 / arc reframe).** The arc
  pins the responder as *in-agent, userspace, same process* as the resolve
  index — a separate daemon would need its own copy of the index (a second
  source of truth) and its own boot/health story. In-agent keeps "one source,
  three readers."

### DDN-7 (ratified point F, newtype concern) — NEW newtype `MeshServiceName` in `overdrive-core::id`

A `MeshServiceName` label-shaped newtype: `const SUFFIX = "svc.overdrive.local"`;
a **single `<job>` label** in v1 (single-node, NO namespace segment);
case-insensitive `FromStr`, canonical lowercase `Display`, serde matching
`Display`/`FromStr`, `<job>` label ≤ **63 octets** (the DNS single-label maximum,
RFC 1035 §2.3.4). This is the DNS *label* limit — a single label of
`<job>.svc.overdrive.local` — and is **distinct from `LABEL_MAX` (253)**, which is
the DNS *name* (total FQDN) maximum (RFC 1035 §3.1). The `<job>` is one label, not a
whole name, so its hard protocol ceiling is 63 (a real protocol constant, like the
codebase's existing `RECONCILER_NAME_MAX` / `WORKFLOW_NAME_MAX` = 63), NOT 253. Full
newtype completeness + a mandatory proptest round-trip (per the codebase newtype
rules). It models the `<job>.svc.overdrive.local` grammar so the responder
parses+matches the suffix through a validated type, never ad-hoc string ops.

**Alternatives considered:**
- **Parse the name inline as a raw `String` (REJECTED).** Raw primitives for a
  domain concept are a blocking violation (CLAUDE.md / development.md §
  "Newtypes — STRICT by default"); a `normalize_*` helper at the call site is
  the documented symptom of a missing newtype constructor.
- **Reuse `WorkloadId` directly as the name key (REJECTED).** `WorkloadId` is
  the *job* label; the dialed *name* carries the suffix grammar
  (`.svc.overdrive.local`) and the case-folding/label-limit rules of the DNS
  name as typed by the workload. `MeshServiceName` owns the grammar;
  internally its `<job>` label maps to a `WorkloadId`-equal segment (the D2
  verified mapping). A future namespace segment extends `MeshServiceName`, not
  `WorkloadId`.

### DDN-8 (ratified point H1) — NXDOMAIN and NODATA both carry a synthetic SOA in authority (negative-TTL = 1 s)

- **NXDOMAIN** (0 running-and-healthy backends): `ResponseCode::NXDomain`, ANCOUNT=0, with
  a synthetic SOA in the AUTHORITY section whose **`MINIMUM` (RFC 2308 negative
  TTL) = 1 s** — so a retrying dialer re-resolves promptly once a backend
  reaches Running.
- **NODATA** (AAAA on a live name): `NOERROR`, ANCOUNT=0, with the **same SOA**
  in authority — so the stub resolver caches the negative answer for the
  queried type without treating the name as nonexistent.
- The SOA fields are pinned: MNAME/RNAME synthetic under the trust domain,
  `SERIAL` derived from the injected `Clock`, REFRESH/RETRY/EXPIRE fixed,
  `MINIMUM = 1`.

**Alternatives considered:**
- **NXDOMAIN/NODATA with no SOA (REJECTED).** Without a negative-TTL SOA the
  stub resolver applies an implementation-default negative cache (often
  seconds-to-minutes), so a workload that queried before its peer reached
  Running would be stuck on a stale negative for the default window — the
  opposite of the fail-honest, promptly-re-resolving posture the arc requires.
  The spike noted the NXDOMAIN/SOA shape as a walking-skeleton build concern;
  the 1 s minimum is the pin.
- **A longer negative TTL (REJECTED for v1).** Single-node allocs reach Running
  on a sub-second-to-seconds cadence; a 1 s negative TTL keeps the
  deploy-then-dial loop tight without hammering the responder.

## Byte-consistency — two single sources, not a shared struct (REV-2)

The honest framing under REV-2 (stable-frontend): the answered value is the
**stable per-`<job>` frontend addr `F`**, NOT a `service_backends` row's
`Backend.addr` (the REV-1 headless model this section described is SUPERSEDED).
Byte-consistency therefore rests on **TWO single sources**, both shared by the
DNS answer path and the resolve translate path:

1. **Frontend truth = the `FrontendAddrAllocator` (`<job> ↔ F`).** The single
   allocator instance is the ONLY owner of which `F` a `<job>` holds. The DNS
   path (`name_index` / `answer_for`) *answers* `F` and the resolve path
   (`MtlsResolve.by_frontend`) *recognizes/translates* `F`, BOTH reading the SAME
   allocator instance (DDN-2 single-owner invariant). The answered `F` is
   byte-identical to the recognized `F` because there is exactly one `F` per
   `<job>` — a second allocator source would diverge them and fail the connection
   closed.
2. **Liveness truth = the `service_backends` rows.** Both readers
   (`ServiceBackendsResolve` and `DnsResponder`) fold the **same
   `ServiceBackendRow` rows** from the **same `ObservationStore`** via the **same
   List-then-Watch contract** (`all_service_backends_rows` at probe,
   `subscribe_all_events` drain, relist-on-`Lagged`). The rows decide
   *resolvability* (is there a running-AND-healthy backend for `<job>`?) — the
   `name_index` WITHHOLDS the answer when zero, and `by_frontend` translates `F`
   to the first-by-`Ord` running-AND-healthy backend.

Consistency is a property of these two shared single sources read through the
single ordered drain (Finding 3), not of a shared in-RAM structure. This is the
"one source, THREE readers" contract for liveness (outbound resolve + inbound
install + name answers) PLUS the "one owner" contract for the `<job> ↔ F`
binding, made precise.

## Components

| Component | Home | Change |
|---|---|---|
| `MeshServiceName` newtype | `overdrive-core/src/id.rs` | **CREATE NEW** |
| `NameAnswer` enum (pure result of `answer_for`) | `overdrive-core` (id or a small `dns` module) | **CREATE NEW** |
| `FrontendAddrAllocator` (the single `<job> ↔ F` owner — frontend SSOT; feeds BOTH `name_index` and `by_frontend`) | `overdrive-control-plane/src/dns_responder/frontend_addr_allocator.rs` | **CREATE NEW** (1a-A) |
| Frontend-addr **WRITER** — deploy-time lifecycle assigner (REV-3): `assign(<job>)` at `<job>` declaration in the `POST /v1/jobs` Service arm + empty-on-boot converge-on-boot rebuild from declared-Service intent | `overdrive-control-plane/src/handlers.rs` (Service-arm admission ~324) + a converge-on-boot rebuild fn beside `adopt_on_restart_recovery` (`lib.rs` boot path) | **EXTEND** (REV-3, the missing writer — a new CALL SITE for the existing `FrontendAddrAllocator::assign`; NO new allocator method) |
| `dns_responder/name_index.rs` (`<job>` → stable `F` List-then-Watch index; **PURE READER** of the `FrontendAddrAllocator` binding — read-only `snapshot().get(<job>)`, NEVER `assign`-on-read, REV-3) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/answer.rs` (pure `answer_for`) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/wire.rs` (hickory-proto encode/decode) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/responder.rs` (`DnsResponder` host adapter + socket loop) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `BackendIndex` `by_frontend` re-key (`(F, listener.port, Proto)` → `ServiceId`; reads the SAME `FrontendAddrAllocator` binding) | `overdrive-control-plane/src/mtls_resolve_adapter.rs` | **EXTEND** (1b-A, additive) |
| `DnsResponderError` (typed `thiserror`) | `dns_responder/` | **CREATE NEW** |
| `run_server_with_obs_and_driver` composition | `overdrive-control-plane/src/lib.rs` (~1893-1957) | **EXTEND** (construct ONE `FrontendAddrAllocator`, share on `AppState`, inject the SAME handle into ALL of {the deploy-time **writer** (REV-3), the re-keyed `MtlsResolve`, the `DnsResponder`} — the single-owner invariant; run the empty-on-boot converge-on-boot rebuild (REV-3) AFTER `AppState` and BEFORE the convergence-loop / responder spawn, beside `adopt_on_restart_recovery`; construct the responder after `resolve.probe()` with `store` + `config.clock` + the `state.net_slot_allocator` handle for the fallback source (DDN-5) + the `FrontendAddrAllocator` handle for `F` answers (DDN-2); probe; spawn; hold handle; same `mtls_worker.is_some()` gate) |
| `hickory-proto` workspace dep | root `Cargo.toml [workspace.dependencies]` | **ADD** (Apache-2.0/MIT) |
| `nix` features (`socket`, `uio` for `recvmsg`/`sendmsg`/`ControlMessage::Ipv4PacketInfo`) | `overdrive-control-plane` + workspace `nix` features | **EXTEND** (no new public API) |

## Consequences

### Positive
- Closes the dial-by-name leg (#236 deferral): an unmodified workload reaches
  its mesh peer by name and lands at a LIVE instance the existing intercept
  path mTLS's — reachable from ordinary `getaddrinfo`, zero app config.
- "One source, three readers" is made precise and verifiable: name answers are
  byte-consistent with the intercept path's backend truth (same rows).
- The security-critical intercept index (`ServiceBackendsResolve`) is provably
  untouched (A1 sibling reader).
- OSS-first wire codec (`hickory-proto`) removes a DNS-encoding bug class; the
  pure `answer_for` + encoder are mutation-gate-strong, deterministic, and
  Tier-1-cheap.
- wire→probe→use: an unbindable port or unreadable store refuses boot rather
  than silently failing to answer.

### Negative / trade-offs
- The `IP_PKTINFO` socket loop is irreducibly Tier-3 (no Tier-2 backstop) — its
  acceptance signal MUST be `getaddrinfo`/`getent`, not `dig @gw` (which is
  lenient and masks a missing source-pin). The pure seams carry the unit
  burden; the socket is real-kernel-only.
- A new newtype + a new module are net-new surface (justified: no existing type
  models the name grammar; no existing reader keys by name).
- v1 is IPv4-only and cannot distinguish declared-but-empty from unknown (both
  → NXDOMAIN) — accepted, named refinements out of scope.

### Design constraints inherited from the spike (DELIVER MUST honor)
1. **`IP_PKTINFO` source-pinning (`ipi_spec_dst` = captured queried gateway) is
   MANDATORY.** Acceptance = `getent`/`getaddrinfo`, never `dig @gw` alone.
2. **Wildcard-first, per-addr-fallback bind** (D5) — keep the fallback as
   node-image insurance.
3. **The responder runs in the ROOT netns**, answers on each per-netns gateway
   addr (= `plan.host_addr`); no per-netns listener, no netns-entering.
4. **`ip_forward=1` is a prerequisite** (already modeled as the converge-on-boot
   `EnableIpForward` step) for the in-netns→root-netns query path.
5. **Verdict is pinned to dev-Lima `7.0.0-22-generic`, NOT the 6.18 appliance
   pin (ADR-0068).** The surfaces exercised are long-stable (well pre-6.18), so
   the verdict is expected to hold — **re-confirm on the 6.18 appliance kernel
   in the DELIVER Tier-3 matrix** (a DEVOPS/Tier-3 obligation).

## Open questions (deferred to DISTILL/DELIVER — NOT improvised here)

- **OQ-1 — the `SpiffeId` → `<job>` accessor signature.** The mapping is
  verified (D2: the `WorkloadId` segment of the SVID path = `<job>`), but no
  existing `SpiffeId` accessor returns the job segment. The exact accessor —
  whether a new `SpiffeId::workload_segment() -> Option<&str>` /
  `job_segment()` on the newtype, or a parse helper local to the index — is a
  small surface decision left to DISTILL/DELIVER per CLAUDE.md "Implement to the
  design — never invent API surface" (the design names the model, not the
  signature; the crafter must surface and pin it, not improvise). **(REV-3: the
  writer (the deploy-time assigner) needs the same OQ-1 surface in the *other*
  direction — `WorkloadId → MeshServiceName` for the `assign` call-site key;
  `WorkloadId → MeshServiceName::new(format!("{id}.{SUFFIX}"))` is the obvious
  shape, pinned as a crafter DECISION at the call site.)**

- **OQ-REV3 — `release(<job>)` has no production trigger (a USER decision, not a
  crafter improvisation).** The `release(<job>)` *surface* is implemented and
  Tier-1-tested (FRONTEND-03 Property 2), but there is **no logical-workload-
  DELETION verb** in production to call it: the router exposes only `POST
  /v1/jobs` (declare), `POST /v1/jobs/:id/stop` (TRANSIENT — the `<job>` stays
  declared, `handlers.rs:718-764`), and GET reads (`lib.rs:2094-2100`). "stop" is
  the withhold-not-release case (Finding 2), NOT a release; the
  `ServiceVipAllocator` confirms the pattern (VIP releases only on conflict-
  rollback, never on stop). REV-3 ships the **`assign` half only**; `F` is
  retained for the process lifetime of every declared `<job>` (acceptable
  Phase-1 single-node — the empty-on-boot rebuild reading the current declared
  set drops a binding for a `<job>` not re-declared after restart). The
  `release(<job>)` deletion-verb trigger is **tracked in
  `overdrive-sh/overdrive#211`** ("Implement workload deletion (intent
  withdrawal) + service/dataplane teardown"): the workload-deletion verb's
  teardown producer is the production actor that will drive
  `FrontendAddrAllocator::release(<job>)` alongside its existing
  `ReleaseServiceVip` / `DeregisterLocalBackend` teardown — the dial-by-name
  frontend addr is the same class of teardown those already name. Out of *this*
  feature's scope; the crafter MUST surface this as a BLOCKER and MUST NOT wire
  `release` into the stop path (doing so reintroduces the SQ1 stale-`F` failure
  on every stop).

*(Two former open questions are now PINNED in DESIGN and NO LONGER deferred: the
`NameAnswer` variant names + the `answer_for` qtype param are concrete — see
§ Components / the feature-delta pinned-signatures block; and the
per-addr-fallback gateway-set source is `NetSlotAllocator` + `responder_addr_for_slot`
with a re-derive-on-converge-tick re-bind lifecycle — DDN-5. The remaining
deferrals are OQ-1 (the `SpiffeId`/`WorkloadId` ↔ `<job>` accessor, a crafter
DECISION) and OQ-REV3 (the `release(<job>)` deletion-verb trigger, tracked in
`overdrive-sh/overdrive#211`).)*

## Out of scope (existing issues / named refinements)

- **VIP path** (`<job>.svc.overdrive.local → fdc2::/16` VIP + XDP `SERVICE_MAP`)
  — **#61** (depends on #167). Headless v1 (D-TME-10) avoids it.
- **Expected-SVID / intended-peer pinning** — **#242** (split from #178). v1 is
  authn-only; the responder answers an addr, not an expected identity.
- **Declared-but-empty → NODATA** (distinct from unknown → NXDOMAIN) — requires
  a declared-service view distinct from the running-and-healthy index; a named
  future refinement, NOT v1 (v1 collapses declared-but-not-running, unhealthy,
  and unknown all to NXDOMAIN).
- **IPv6 / real AAAA records** — widening the `SocketAddrV4`/`Ipv4Addr`
  substrate; out of v1 scope (AAAA is NODATA in v1).
- **Cross-node / multi-node name resolution, gossiped name state** — out of
  Phase-2 single-node scope.
