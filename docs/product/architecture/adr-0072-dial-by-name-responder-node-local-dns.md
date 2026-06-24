# ADR-0072: Dial-by-name responder — node-local in-agent DNS over the ObservationStore (the THIRD reader)

## Status

**Accepted** (2026-06-25). Decision-maker: Morgan (solution-architecture);
**ratified by the user** (Pass-1 decision points A1/B/C1/D1/E1/F/H1 all
decided 2026-06-24/25). Tags: phase-2, mesh, dns, name-layer, reachability,
observation-store, transparent-mtls, headless, ipv4-only, #243.

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
`Display`/`FromStr`, `<job>` label ≤ `LABEL_MAX` (253). Full newtype
completeness + a mandatory proptest round-trip (per the codebase newtype
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

## Byte-consistency — the same rows, not a shared struct

The honest framing: the responder is the **third reader of the
`ObservationStore` `service_backends` surface** — NOT a reader of the
addr-keyed intercept index *struct*. Both readers (`ServiceBackendsResolve` and
`DnsResponder`) fold the **same `ServiceBackendRow` rows** from the **same
`ObservationStore`** via the **same List-then-Watch contract**
(`all_service_backends_rows` at probe, `subscribe_all_events` drain,
relist-on-`Lagged`). The answered `A` addr and the addr `MtlsResolve.resolve`
recognizes are byte-identical because they derive from the same row's
`Backend.addr` — consistency is a property of the shared rows, not of a shared
in-RAM structure. This is the "one source, THREE readers" contract (outbound
resolve + inbound install + name answers) made precise.

## Components

| Component | Home | Change |
|---|---|---|
| `MeshServiceName` newtype | `overdrive-core/src/id.rs` | **CREATE NEW** |
| `NameAnswer` enum (pure result of `answer_for`) | `overdrive-core` (id or a small `dns` module) | **CREATE NEW** |
| `dns_responder/name_index.rs` (`by_name` List-then-Watch index) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/answer.rs` (pure `answer_for`) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/wire.rs` (hickory-proto encode/decode) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `dns_responder/responder.rs` (`DnsResponder` host adapter + socket loop) | `overdrive-control-plane/src/dns_responder/` | **CREATE NEW** |
| `DnsResponderError` (typed `thiserror`) | `dns_responder/` | **CREATE NEW** |
| `run_server_with_obs_and_driver` composition | `overdrive-control-plane/src/lib.rs` (~1893-1957) | **EXTEND** (construct after `resolve.probe()` with `store` + `config.clock` + the `state.net_slot_allocator` handle for the fallback source — DDN-5; probe; spawn; hold handle; same `mtls_worker.is_some()` gate) |
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
  signature; the crafter must surface and pin it, not improvise).

*(Two former open questions are now PINNED in DESIGN and NO LONGER deferred: the
`NameAnswer` variant names + the `answer_for` qtype param are concrete — see
§ Components / the feature-delta pinned-signatures block; and the
per-addr-fallback gateway-set source is `NetSlotAllocator` + `responder_addr_for_slot`
with a re-derive-on-converge-tick re-bind lifecycle — DDN-5. OQ-1 above is the
sole remaining deferral.)*

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
