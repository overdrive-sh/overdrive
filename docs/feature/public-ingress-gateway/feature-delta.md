# Feature Delta — `public-ingress-gateway`

> **DESIGN status:** Stages 1–3 of full-stack DESIGN — SYSTEM architecture,
> DOMAIN model and APPLICATION architecture — are documented as Proposed on
> 2026-09-13. Application adversarial re-review is in remediation; full-stack
> handoff remains pending until its exact contracts receive APPROVED.
> Interaction mode: PROPOSE. Density: lean, Tier-1 `[REF]` only
> (`explicit_override`; optional expansions were not emitted). Stage 3 pins the
> public/config/persistence/application surfaces that current evidence supports.
> System ADR-0104/0117–0121 resolve former `DESIGN-GAP-PIG-1` with
> node-embedded placement, acquisition-neutral certificate consumption,
> demand-gated cgroup-BPF Service forwarding, independent Gateway/BPF
> generations, the public-TLS→gateway-SVID-mTLS trust transition and narrow
> public-listener gates. Route ADR-0105, Domain ADR-0112–0116 and Application
> ADR-0106–0111 pin the compatible downstream choices; exact API
> contracts live only in the Application `[REF]` sections below. The complete cross-owner flow remains
> `AT-PIG-E2E-1`, a production Tier-3 acceptance boundary—not a spike.

## Wave: DESIGN / [REF] Prior-wave and architecture consultation

The requested DISCUSS/SPIKE inputs do not exist. Current feature intent comes
from GitHub issue #54 plus the user's explicit corrections. The gateway
research supplies evidence and implementation inventory but cannot override
those inputs. The whitepaper is contextual and non-authoritative; this DESIGN
does not invent missing user stories, a story map, wave decisions, or KPIs.

| Artifact | Result |
|---|---|
| GitHub issue #54 (`gh issue view 54 --comments`) | ✓ Re-read; open, no comments; current intent is “Gateway subsystem (node-agent-embedded; hyper + rustls; in-process BPF map access)” and “Not a platform job — infrastructure” |
| GitHub issue #57 (`gh issue view 57 --comments`) | ✓ Re-read; open; manual certificate is explicitly allowed for #54 and later finite durable ACME publishes the same protected runtime input |
| `docs/research/gateway/public-ingress-gateway-minimum-comprehensive-research.md` | ✓ Read in full; evidence source, corrected where it conflicts with issue #54 or user direction |
| `docs/whitepaper.md` §11 | ✓ Consulted as historical context only; explicitly non-authoritative |
| `docs/feature/public-ingress-gateway/discuss/wave-decisions.md` | ⊘ Not found; research substituted by explicit user direction |
| `docs/feature/public-ingress-gateway/discuss/user-stories.md` | ⊘ Not found; no stories invented |
| `docs/feature/public-ingress-gateway/discuss/story-map.md` | ⊘ Not found; no release map invented |
| `docs/feature/public-ingress-gateway/discuss/outcome-kpis.md` | ⊘ Not found; no KPI/SLO invented |
| `docs/feature/public-ingress-gateway/spike/findings.md` | ⊘ Not found; unmeasured claims remain labelled assumptions or DELIVER proof obligations |
| `docs/product/architecture/brief.md` | ✓ Read; System Architecture is extended, not replaced |
| Accepted gateway-adjacent ADRs | ✓ Read/searched: control-plane TLS/config/startup; workload/wire/persistence shapes; Service listener/VIP/backend/health; built-in CA/SVID; workflow; transparent mTLS/networking; watcher/hydration/test boundaries |
| Current backend publication/hydration path | ✓ Re-read `service_lifecycle.rs`, `service_map_hydrator.rs`, action shim and `EbpfDataplane::update_service`: Path-A→neither map, same-host non-mesh→`LOCAL_BACKEND_MAP`, remote→`SERVICE_MAP`; BackendId is allocated in Dataplane and not recycled within a process |
| Current packet-entry and selected-peer path | ✓ Re-read `cgroup_connect4_service`, XDP Service lookup, cgroup/XDP production attach, `MtlsResolve`/`BackendIndex`/`ResolvedBackend.expected_svid`, and `MtlsEnforcement`/`expected_peer`: the ancestor connect4 hook covers `serve`, while gateway cookie receipt, non-allocation SVID use and gateway-client entry are new decisions |
| Relevant product journeys | ✓ `submit-a-service`, `issue-workload-identity`, `hold-identity-for-the-running-set`, `enforce-transparent-mtls-on-the-wire`, `dial-a-mesh-peer-by-name`, `author-a-durable-workflow`, `run-a-vm-workload` |
| `.nwave/trusted-source-domains.yaml` / `.nwave/des-config.json` | ✓ Read; official RFC and repository evidence used; rigor inherited |

Accepted-record coverage, grouped only to keep the checklist lean:

- **Composition/config/API:** ADR-0008, ADR-0009, ADR-0010, ADR-0012,
  ADR-0014, ADR-0015, ADR-0019, ADR-0025, ADR-0032, ADR-0039.
- **Persistence, reconciliation, status and schema:** ADR-0020, ADR-0021,
  ADR-0023, ADR-0035, ADR-0036 as partially superseded by ADR-0086,
  ADR-0037, ADR-0048, ADR-0077, ADR-0078, ADR-0086, ADR-0102.
- **Service discovery/health and networking/dataplane:** ADR-0038,
  ADR-0040–ADR-0043, ADR-0045, ADR-0046, ADR-0049, ADR-0052,
  ADR-0053, ADR-0054 (ProbeRunner), ADR-0055 (ServiceLifecycle),
  ADR-0056 (Service events), ADR-0057 (health-check spec), ADR-0058
  (default startup probe), ADR-0059 (Service terminal taxonomy), ADR-0060,
  ADR-0061, ADR-0062, ADR-0068–ADR-0072, ADR-0075, ADR-0076, ADR-0079,
  ADR-0080, ADR-0085, ADR-0087–ADR-0091, and accepted ADR-0097–ADR-0101.
- **CA/identity/workflows:** ADR-0063, workflow ADR-0064, ADR-0065,
  ADR-0066, ADR-0067.
- **Architecture/test structure:** ADR-0003–ADR-0006 plus the repository's
  mandatory development, Rust, testing, verification, debugging, BPF and
  design rules.

Relevant **Proposed**, not accepted, records were also checked where they
describe current adjacent work: ADR-0084 and ADR-0092–ADR-0096. No proposal is
treated as accepted authority in this decision.

Issue #54 plus user clarifications override conflicting research/whitepaper
prose:

1. Gateway means public north-south ingress from real external users.
2. The first production slice consumes an operator-supplied publicly trusted
   certificate/key. A self-signed/test CA is only the hermetic test mechanism.
   Certificate consumption is stable and independent of acquisition; GH #57
   owns later automated ACME acquisition/renewal as an additive producer.
3. The public user presents no SPIFFE identity. After public TLS terminates,
   the gateway presents its own internal SPIFFE SVID on the mTLS hop and pins
   the selected workload peer identity.
4. The gateway preserves the existing BPF/XDP Service dataplane. It resolves a
   Route to one exact Service frontend and does not enumerate/select backends
   or implement a second userspace load balancer.

### Evidence classification

| Classification | Established fact |
|---|---|
| **Current feature intent** | Issue #54 plus user direction: node-agent-embedded `hyper` + `rustls`, in-process BPF map access, public north-south ingress, operator-supplied production Web-PKI certified key, gateway SVID on the internal hop, and no second userspace backend load balancer. |
| **Accepted design** | `overdrive serve` is the single-node composition owner; Service listener intent and platform-issued VIPs are distinct from public listener intent; ServiceLifecycle solely publishes the complete `ServiceBackendRow`; ServiceMapHydrator consumes that row and atomically programs the existing Dataplane/BPF Service frontend; the persistent workload-identity CA is separate from the ephemeral operator HTTPS CA; the universal agent-light transparent-mTLS path is the east-west enforcement mechanism; workflow execution is durable and journaled. |
| **Implemented fact** | The gateway surfaces are absent, but accepted foundations exist: ServiceMapHydrator partitions Path-A to neither map, same-host non-mesh to LOCAL_BACKEND_MAP and remote to SERVICE_MAP; `cgroup_connect4_service` fires for Path-A; shared Service maps and rustls chain/single-SPIFFE-SAN verification exist; the reserved `expected_peer`/mismatch contract is not exercised because current production always supplies `None`; allocation-keyed identity cannot represent the gateway unchanged. |
| **Whitepaper** | Historical illustration only. It establishes no binding API, owner, ordering or traffic-path contract. |
| **This SYSTEM proposal** | One `serve`-owned working gateway: IPv4-A/TCP/443 public TLS/HTTP; Public Route Set resolves exact Service Frontend; the gateway connector registers that frontend against its socket cookie with the Dataplane owner, then its `connect(2)` enters the already-attached `cgroup_connect4_service` hook; the hook selects from the existing `SERVICE_MAP` Maglev inner table and `BACKEND_MAP`, rewrites the destination, and records the actual `BackendId`; Dataplane joins that receipt to the exact applied `Backend.alloc`; gateway-held SVID mTLS pins that identity; no userspace selection. |
| **Later DESIGN resolution** | Route ADR-0105 owns only the singleton Public Route Set. Domain ADR-0112–0116 own custody/identity/dataplane/demand/state-model decisions. Application ADR-0106–0111 own Route/application owner, custody, BPF selection/identity, gateway SVID/mTLS, public TLS/HTTP and operator status choices. Exact schemas, signatures, config, records/codecs, errors and crate/module contracts live only in the Application `[REF]` sections below. SYSTEM copies use those names without redefining them. |

The research's operator-supplied-certificate recommendation is restored by the
latest explicit scope ruling. Its userspace `ServiceBackendRow` selection
recommendation remains superseded by issue #54, the user correction, and the
existing ServiceMapHydrator/Dataplane ownership chain. Backend eligibility and
BPF application remain with their accepted owners. ACME is excluded and cited
to existing GH #57; no new deferral issue is created.

## Wave: DESIGN / [REF] Requirements and sizing boundary

### Functional requirements

- A real external user
  reaches one exact IPv4 A hostname on HTTPS and receives the selected Service
  workload's HTTP status, headers, and streaming body.
- Public Certified-Key Custody validates/protects one manual operator-supplied
  generation and grants an opaque resolver snapshot; Gateway Application binds
  it with Public Route Set + resolved Service Frontend in one atomic admission
  object. Request routing/listening never reads manual
  paths, raw PEM or secret key bytes directly.
- Public Route Set resolves its exact Service Frontend `(VIP, port, protocol)`;
  Gateway Application publishes derived live-frontend demand for that exact
  frontend to ServiceMapHydrator, and the upstream connector hands the same
  frontend to the BPF-owned selection path without enumerating candidates.
- The gateway uses its own internal SVID. The workload identity actually
  selected by the dataplane must propagate to intended-peer verification before
  application bytes are accepted. A gateway-specific identity lifecycle/holder
  owns the current gateway SVID; the Dataplane selection-receipt owner supplies
  the actual selected workload identity.
- ServiceLifecycle remains the sole backend-eligibility publisher and
  ServiceMapHydrator remains the sole consumer that atomically applies the
  current eligible set to BPF maps. The gateway never reads that set to choose a
  backend.
- The complete flow is reachable through real `serve`, Service deploy, Route
  deploy and external TLS and is verified by `AT-PIG-E2E-1`.

### Ranked quality attributes and constraints

1. **Security / authenticity:** public Web PKI at the north-south boundary,
   gateway SPIFFE identity at the internal boundary, and expected-workload SAN
   pinning; no plaintext or wrong-valid-peer fallback.
2. **Functional suitability / production drivability:** one real public origin
   works end to end through existing BPF-owned selection, proved by
   `AT-PIG-E2E-1`.
3. **Reliability / recoverability:** atomic certified-key replacement,
   independent honest gateway/dataplane convergence, bounded drain, and no
   false unified-generation claim.
4. **Maintainability / reuse:** extend the current `serve`, public-certified-key
   consumer, internal CA, Service frontend and Dataplane ownership boundaries
   without duplicating ServiceMapHydrator or BPF selection.
5. **Performance efficiency:** streaming/backpressure and bounded resources;
   no unproven throughput SLO and no distributed scaling machinery.

### Smallest quantified envelope

This is a correctness envelope, not an invented capacity promise:

| Dimension | Initial bound |
|---|---|
| Gateway nodes | **1** |
| Public origins / certified hostnames | **1 exact hostname**; no wildcard |
| Public listeners | **1:** IPv4 TCP/443 |
| Public route rules | **1** exact-host rule with exact or segment-aware path prefix |
| Route targets | **1** exact TCP Service listener |
| Gateway dataplane handoff | **1 exact Service frontend**; the gateway performs zero backend enumeration or selection |
| Protocols | TLS 1.3 public/internal + HTTP/1.1 downstream/upstream |
| Gateway runtime state | `O(R + C)` with `R=1`; no backend candidate vector |
| Gateway-owned sockets | About `2C` plus listener: public leg + one BPF-rewritten internal leg per active request/connection |
| Connect intents / selection receipts | `O(C)`, one transient cookie-keyed intent/outcome per connecting internal socket, consumed after `connect(2)` and cleaned on failure/drain |
| Applied identity association | `O(H)` for process-lifetime distinct BackendIds (`H`), one immutable Dataplane-owned `BackendId`→exact applied `Backend.alloc` association; ids/associations are not recycled until restart |
| Certificate acquisition | Outside the slice; one operator-supplied production Web-PKI certified key is consumed |

Every connection/header/body/deadline ceiling must be finite, but exact values
are deliberately handed to application DESIGN because no KPI or accepted config
surface authorizes numbers here.

## Wave: DESIGN / [REF] System options and selection

| Option | Shape | Security / lifecycle | Cost and trade-off | Verdict |
|---|---|---|---|---|
| **A. TEACH the already-attached cgroup Service hook (selected)** | The connector creates a socket inside the existing `overdrive serve` cgroup scope, obtains its stable kernel cookie, and asks the Dataplane owner to register `(cookie, exact ServiceKey)`. `connect(2)` enters the inherited `cgroup_connect4_service`; its gateway-intent arm indexes the existing `SERVICE_MAP` Maglev inner table by a cookie-derived slot, resolves `BACKEND_MAP`, rewrites the destination, and writes a cookie-keyed selected/`NoBackend` outcome. | Reuses the hook empirically proven to fire for Path-A and the accepted ADR-0053 GATE→TEACH trigger. The intent entry makes the gateway arm explicit and fail-closed while every unregistered connect retains today's `LOCAL_BACKEND_MAP` behavior. | Adds transient intent/receipt maps and a Dataplane-owned applied-identity association; no cgroup, netns, veth, process, or userspace LB. | **Selected** |
| **B. Dedicated gateway child cgroup + specialized connect4 branch** | Move a connector thread into a threaded child cgroup and recognize the child cgroup id before shared-map selection. | BPF still owns selection and a receipt can identify the peer. | Adds threaded-cgroup creation/migration/restart semantics although a socket cookie already scopes the gateway connect; no security or routing benefit over A. | Viable, rejected as dominated by A |
| **C. Dedicated gateway netns/veth into XDP client ingress** | Route gateway upstream packets across a new veth into `xdp_service_map_lookup`; add flow-selection receipt for peer identity. | Literal XDP path and existing wire algorithm. | Adds netns/veth/routes, packet-entry lifecycle, reverse path, and flow correlation solely to reach an ingress hook; larger failure domain. | Viable, rejected as dominated by A |
| **D. Userspace backend selection** | Gateway watches `ServiceBackendRow` and round-robins. | Makes identity available before connect. | Forbidden second LB; duplicates ServiceMapHydrator/BPF choice and can diverge. | Rejected by constraint |

**Decisions D-SYS-6/D-SYS-7:** select Option A as the smallest working packet-
entry topology; [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
owns that choice. Separately, D-SYS-3/[ADR-0117](../../product/architecture/adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
keeps manual Public Certified Key input first and GH #57 on the same
acquisition-neutral custody boundary.

## Wave: DESIGN / [REF] System decisions

- **D-SYS-1 — Node placement:** `overdrive serve` owns the in-process gateway,
  public listener, gateway-upstream connector and shutdown; it is not an
  Allocation or separate process.
- **D-SYS-2 — Public reachability:** the hostname has one IPv4 A record to the
  node; after prerequisite probes/initial state apply, `serve` binds IPv4
  TCP/443 and serves TLS 1.3 + HTTP/1.1.
- **D-SYS-3 — Public Certified-Key Custody/consumption:**
  `ManualCertifiedKeySource` validates the operator files and
  `PublicCertifiedKeyCustody` builds the usable rustls resolver material and
  seals the origin key with `PublicCertifiedKeyAeadCodec`, persists the complete
  `PublicCertifiedKeyV1` through IntentStore, atomically publishes the opaque
  Current/Usable snapshot, and only then writes redacted
  `PublicCertifiedKeyStatusRowV1`. `GatewayApplicationOwner` atomically binds it
  with Public Route Set + resolved Service Frontend as one atomic Gateway
  Application admission object.
  Manual operator input is the first producer.
  Gateway listeners/handlers never read manual file paths or raw PEM. ADR-0107
  pins the storage/API shape. ACME acquisition/renewal is excluded under GH #57
  and must later call the same producer-neutral install boundary.
- **D-SYS-4 — Three independent trust domains:** the ephemeral operator HTTPS
  CA, the persistent internal workload-identity CA, and the operator-supplied
  public Web-PKI certified key remain distinct. The public private key never
  enters `IdentityMgr`, an observation row, logs, Route status, or handlers.
- **D-SYS-5 — Gateway identity ownership:** a dedicated single-slot gateway
  identity lifecycle/holder, composed after the persistent internal CA, owns
  issuance, in-memory private material, near-expiry reissue, restart reissue and
  drop for `spiffe://overdrive.local/gateway/<node-id>`. It reuses
  `issue_and_audit` but never fabricates `AllocationId` or shares the
  allocation-keyed holder surface. The holder authorizes private-material use
  only inside the gateway-client mTLS owner; the HTTP runtime never receives a
  raw key/SVID getter result. Application DESIGN pins the exact capability.
- **D-SYS-6 — BPF-owned internal traffic:** the gateway connector creates an
  IPv4 TCP socket inside the existing `overdrive serve` cgroup scope, obtains
  its kernel socket cookie, and gives only `(cookie, exact ServiceKey)` to the
  Dataplane owner. It then calls `connect(2)` on the exact Service Frontend.
  The already-attached `cgroup_connect4_service` checks the registered intent,
  indexes the existing `SERVICE_MAP` Maglev inner table using a deterministic
  hash of that cookie, resolves `BACKEND_MAP`, rewrites to the selected Path-A
  workload address, and writes the selected `BackendId` outcome before allowing
  the connect. `NoBackend` is recorded and denied rather than falling through
  to an unchanged VIP. A socket without a registered gateway intent follows
  today's `LOCAL_BACKEND_MAP` hit/miss behavior unchanged. The gateway never
  preselects a backend or touches a raw BPF map.
  The rewrite occurs at `connect(2)` before host routing; the resulting socket
  routes over the existing per-allocation Path-A veth into the workload netns,
  where the existing inbound nft-TPROXY/agent-light terminator accepts the
  gateway's TLS leg and delivers plaintext to the workload. The response returns
  on that same authenticated socket. The XDP programs remain attached to their
  existing client/backend interfaces for wire ingress but are not the selection
  hook for this host-originated gateway connect. No new netns or veth is added.
- **D-SYS-7 — Backend authority/application:** ServiceLifecycle remains the
  sole complete `ServiceBackendRow`/eligibility publisher. The existing
  ServiceMapHydrator consumes those observations. Gateway Application supplies
  only derived live-frontend demand; for that exact demanded frontend the
  hydrator replaces ADR-0053's Path-A GATE with TEACH into the existing
  `SERVICE_MAP` Maglev inner table and `BACKEND_MAP`. Undemanded Path-A
  frontends remain gated, and the existing same-host non-mesh→
  `LOCAL_BACKEND_MAP` and remote→`SERVICE_MAP` classifications remain
  unchanged. Because `EbpfDataplane` assigns `BackendId`, it also owns the
  association from that id to the exact applied `Backend.alloc`. Under one
  serialized Dataplane commit guard, it validates and inserts the exact identity
  associations as non-readable `Reserved` before any BPF map work, performs the
  fallible BACKEND/private-inner/reverse-NAT work, makes the atomic outer
  `SERVICE_MAP` pointer swap the commit point, then infallibly publishes those
  reservations `Applied` before releasing the guard. A receipt may arise just
  after the outer swap, but its identity read blocks on that guard until Applied
  publication completes. Applied associations are retained immutably for the
  process lifetime. Backend withdrawal first swaps/deletes the Service-map
  slot so no new selection can name the id; old/in-flight receipts remain safe
  because neither the id nor its identity association is recycled. Restart
  destroys every socket/receipt before ids and associations are rebuilt. An
  attempt to associate one live BackendId with a different `Backend.alloc`
  rejects the new application before the outer-map swap and preserves the prior
  applied generation. The gateway neither
  List/Watches backend rows nor owns a round-robin cursor.
- **D-SYS-8 — Gateway and BPF generations:** Gateway Application atomically
  publishes Route/frontend/opaque-resolver state. ServiceMapHydrator atomically
  publishes its BPF backend-set application independently. They remain
  independent; the transient receipt names the actual selected `BackendId`,
  and Dataplane resolves it through the immutable applied-id association before
  any mTLS bytes are sent. The receipt does not pretend to unify or snapshot the
  Gateway Applied Generation with BPF state.
  ServiceLifecycle and
  ServiceMapHydrator independently converge backend observations and BPF map
  generations. Gateway Applied Generation does not mean BPF hydration is
  current, and BPF Hydrated does not mean a Gateway Application is applied.
- **D-SYS-9 — HTTP scope:** strict bounded HTTP/1.1 streams through the
  BPF-rewritten upstream socket; no gateway retry/replay/backend cursor.
- **D-SYS-10 — Availability/status:** Route miss is 404; BPF selection receipt
  `NoBackend`—including an empty eligible set/no selectable BPF backend—is the
  only 503 path. A
  `Selected` receipt whose applied identity is missing or mismatched, and every
  internal connect/receipt/mTLS/upstream failure, is 502. `GatewayControl`
  separately exposes active application/custody and
  relevant BPF hydration status as `GatewayStatusResponse` over operator-mTLS
  `GET /v1/gateway/status` (200 when readable, 500 on observation read error).
  [ADR-0114](../../product/architecture/adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md)
  and the Application HTTP contract own the exact receipt/error mapping.
- **D-SYS-11 — Restart/drain:** boot rebuilds ServiceMapHydrator state and the
  Dataplane applied-id association, then may run the sentinel-ServiceKey
  `NoBackend` receipt probe before Gateway Identity is Current; that probe proves
  only the registered-connect/hook/receipt bind capability and never attempts
  mTLS. After reissuing/holding the gateway SVID, the actual Service Frontend
  probe treats `NoBackend` as a successful 503-capable result, while `Selected`
  must complete exact-peer mTLS before Gateway Application is atomically
  promoted and TCP/443 binds.
  Shutdown stops accepts, drains admitted requests to the existing deadline,
  then joins connector/listener tasks and clears per-socket receipts. No HA.

## Wave: DESIGN / [REF] C4 System Context

The canonical Level-1 view is maintained once in
[Public ingress gateway canonical C4](../../product/architecture/c4-diagrams.md#public-ingress-gateway-canonical-c4).
It shows the external user and operator boundaries, operator-managed certified-
key files, public IPv4 DNS/TCP-443 reachability, the embedded Overdrive node and
the identity-unaware Service workload. System ADR-0104/0117–0121 own the node,
credential-consumption, packet-entry, consistency, trust and listener-gate
choices; this feature delta does not duplicate the diagram source.

## Wave: DESIGN / [REF] C4 Container

The canonical Level-2 and Level-3 views are maintained at the same
[canonical C4 anchor](../../product/architecture/c4-diagrams.md#public-ingress-gateway-canonical-c4).
They are the sole Mermaid source for the `overdrive serve` container,
IntentStore/ObservationStore ownership, `GatewayApplicationOwner`,
`PublicCertifiedKeyCustody`, demand-gated ServiceMapHydrator/Dataplane,
`GatewayIdentitySlot`, `HostGatewayClientMtls`, public runtime and operator
status relationships. The boot-sentinel edge terminates at canonical
`NoBackend` without identity use; the actual-frontend edge either establishes
503 capability from `NoBackend` or, only after Gateway Identity Current,
continues from `Selected` through exact-peer mTLS.

## Wave: DESIGN / [REF] Source-of-truth and trust boundaries

| State/fact | Source of truth | Owner | Explicit non-owner/non-use |
|---|---|---|---|
| Gateway enablement and public bind | `ServerConfig.gateway: Option<GatewayConfig>` plus live socket fact | `serve` composition root / `GatewayApplicationOwner` | All-or-none operator config; bind occurs only after application/dataplane/identity gates |
| Public Route Set | `IntentStore` `PublicRouteSetV1` | `PublicRouteSetOwner` | Service `[[listener]]` is backend intent, not public exposure intent; no backend candidate/BPF state |
| Live gateway Service-frontend demand | Derived from staged, current and draining coherent Gateway Application generations | `GatewayApplicationOwner` publishes demand; ServiceMapHydrator consumes it | Not desired truth, backend eligibility or a backend set; never persisted as a competing Route source |
| Public Certified Key current custody generation | `IntentStore` `PublicCertifiedKeyV1`, including `ProtectedOriginKeyV1` sealed by `PublicCertifiedKeyAeadCodec` | `PublicCertifiedKeyCustody` is sole writer/validity owner | `ManualCertifiedKeySource` paths/raw PEM stop before custody; public-key secrets never enter status, Route, handler, CLI or logs |
| Public Certified Key status | `ObservationStore.public_certified_key_status` / `PublicCertifiedKeyStatusRowV1` | `PublicCertifiedKeyCustody` sole LWW writer after Current publication | Redacted active/usability/install-failure facts only; status failure retries without rolling back Current; no paths, PEM, DER, key or ciphertext |
| Gateway SVID | Existing internal CA + issued-certificate audit plus volatile `GatewayIdentitySlot` | `GatewaySvidLifecycle` + control-plane action executor + `GatewayIdentitySlot` | Separate from allocation `IdentityMgr` and Public Certified Key; only `HostGatewayClientMtls` may use private material; no fabricated AllocationId |
| Eligible workload backend set and `Backend.alloc` | `ServiceBackendRow` | ServiceLifecycle, sole complete-row publisher | Gateway does not read, probe, author, filter or select this set |
| Applied Service frontend backend set | `SERVICE_MAP` outer + Maglev inner table + `BACKEND_MAP` | ServiceMapHydrator → Dataplane/EbpfDataplane | TEACH sends Path-A backends here for the live gateway consumer; handler never mutates maps |
| Gateway Applied Generation | Active `GatewayApplicationOwner` Current `ArcSwapOption`, built from Route + exact Service Frontend + opaque certified-key snapshot | `GatewayApplicationOwner` | Atomic admission pointer retained per connection; excludes BPF generation/candidates and is not reconstructed from status |
| Gateway Application status | `ObservationStore.gateway_application_status` / `GatewayApplicationStatusRowV1` keyed by `NodeId` | `GatewayApplicationOwner` sole LWW writer | Listener/staged/current/draining/unavailable/non-secret identity/connect-path facts only; no BackendId, receipt, peer, BPF generation or credential secret |
| Operator gateway-status response | `GatewayControl` plus exact ObservationStore point reads for application/custody and relevant Service hydration | Existing operator-mTLS `GET /v1/gateway/status` handler | `GatewayStatusResponse`; 200 for readable active/disabled state, 500 on observation read error; raw dataplane failure strings and secrets omitted |
| Actual selected workload identity | transient socket-cookie→BackendId receipt joined to the immutable process-lifetime association for the exact applied `Backend.alloc` | Dataplane gateway-connect registry | Consumed before mTLS bytes; gateway receives the identity but does not choose it; association is not rebound or recycled |
| Per-request connection state | Public leg + BPF-rewritten internal socket + consumed immutable selected-peer fact | Gateway runtime/connector/Dataplane owners | No userspace backend cursor/cache |

The existing authoritative identity input is ServiceLifecycle's complete
`ServiceBackendRow`, specifically `Backend.alloc` on the exact `Backend` handed
through ServiceMapHydrator to Dataplane. `BackendIndex` consumes the same rows
for the existing address/frontend resolver, but a later address-keyed read is
not authoritative for a socket BPF has already selected. The selection becomes
identifiable at the cgroup hook's synchronous cookie→BackendId receipt; only
Dataplane can join that id to the exact applied `Backend.alloc`, and only the
gateway-client mTLS owner receives the resulting expected `SpiffeId`.

The public network remains untrusted until rustls establishes the Public
Certified Key boundary; the internal leg remains untrusted until gateway/
selected-peer SVID authentication. The Service workload is identity-unaware.

### Failure domains

| Domain | Failure boundary and recovery |
|---|---|
| Gateway application/listener/connector/identity tasks | One `overdrive serve` process and `ServerHandle` lifecycle; there is no independent gateway supervisor. An owned-task exit stops new public admission or fails its request cause-distinctly; a `serve` restart reconstructs custody/application/demand/BPF/identity prerequisites before rebinding. |
| Kernel Service dataplane | Existing `EbpfDataplane` link/map owner. Load failure or a missing/malformed boot-sentinel receipt refuses initial bind; a canonical sentinel `NoBackend` proves the registered-connect/hook/receipt path without gateway identity or mTLS. For the actual frontend, canonical `NoBackend` is a successful bind-capability result and the distinct public 503 path; a missing/invalid outcome closes the socket and returns public 502 without bypass. |
| Workload allocation/netns/inbound terminator | Existing WorkloadLifecycle and agent-light owners. Their failure changes existing workload/backend facts and eventually BPF hydration; it never transfers health ownership to the gateway. |
| Public Certified-Key Custody | Protected input survives `serve` restart; invalid replacement preserves the prior current usable generation. It does not share failure or trust material with either private CA. |

No first-slice component fails over to another node, process, proxy, plaintext
path or userspace backend selector.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

### Existing state-ownership matrix

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Allocation `Running` | Existing WorkloadLifecycle/action-shim/driver path | The allocation reached its accepted driver-specific Running boundary | Existing driver/lifecycle inputs only | Gateway bind, Route, public certified key, gateway SVID, public listener application |
| Service `Stable` | Existing ServiceLifecycle | Existing probe-derived Service terminal-condition promise | Existing Service probe policy only | Gateway listener or route application; backend eligibility |
| `Backend.healthy` / backend eligibility | ServiceLifecycle | Current Running membership satisfies its existing terminal-veto/readiness policy | Existing allocation/probe facts; no readiness means eligible for a non-vetoed Running allocation | Gateway Applied Generation, Public Certified Key, public request success; `Stable` remains unnecessary |
| Service frontend BPF generation | ServiceMapHydrator + Dataplane/EbpfDataplane | The existing BPF maps atomically reflect the backend set that the hydrator applied | ServiceLifecycle observation plus existing hydrator retry/application behavior | Gateway Applied Generation, public TLS, request success, workload `Running`/`Stable` |
| Operator/control-plane listener readiness | Existing control-plane server owner | Operator HTTPS listener is bound with its independent ephemeral CA | Existing control-plane boot/probe sequence | Public Certified Key custody/usability, Gateway Applied Generation, public Route/backend availability |

### Gate G-1 — Gateway listener prerequisites complete

- **Existing evidence:** the selected topology assigns every prerequisite owner;
  current code supplies foundations but requires the TEACH/receipt/identity
  extensions in this design.
- **Owner:** `serve` composition root.
- **Promise:** Public Certified Key usable, Route/frontend application installed,
  initial demanded-frontend BPF application acknowledged (including a valid
  empty eligible set, with applied-id associations for every selectable id),
  the boot-sentinel canonical `NoBackend` receipt proved the registered-connect/
  hook/receipt bind capability, and Gateway Identity is Current. The actual Service
  Frontend probe then returned either bind-valid `ReadyNoBackend` or, for
  `Selected`, `SelectedPeerAuthenticated`; TCP/443 may now bind. Backend
  presence is not a listener prerequisite.
- **Affected state:** public listener bound/admission eligibility only.
- **Failure projection:** missing/malformed sentinel or actual-frontend receipt,
  connect-path failure, or a `Selected` result that cannot resolve identity or
  complete exact-peer mTLS keeps TCP/443 unbound and is cause-distinct on
  operator-mTLS gateway status. Actual-frontend `NoBackend` alone never blocks
  bind; the active listener serves 503 for that result.
- **Explicitly unaffected:** allocation `Running`, Service `Stable`, backend
  eligibility, and operator HTTPS meaning.
- **Ordering:** custody → coherent Route/frontend application candidate →
  publish exact live-frontend demand → ServiceMapHydrator initial apply/identity
  association → boot-sentinel `NoBackend` receipt/cleanup proof (permitted
  before gateway identity) → gateway SVID Current → actual Service Frontend
  probe. That probe either stops successfully at `ReadyNoBackend`, or its
  `Selected` branch resolves the applied identity and completes exact-peer mTLS
  only after Gateway Identity Current → promote the admission object → bind
  TCP/443.
- **Counterexample:** withholding allocation `Running` because host TCP/443 is
  occupied would make an unrelated workload state depend on a node listener it
  neither owns nor needs.
- **Evidence lane:** pure prerequisite/branch ordering; Tier-3 sentinel receipt,
  actual-frontend `NoBackend`, selected-peer mTLS and startup/refusal evidence;
  and `AT-PIG-E2E-1`.

### Gate G-2 — Public Certified Key generation usable in custody

- **Existing evidence:** no public certified-key owner exists. The operator CA
  and workload CA are explicitly different and cannot satisfy public Web PKI.
- **Owner:** `PublicCertifiedKeyCustody` only, using
  `PublicCertifiedKeyAeadCodec` for protected persistence.
- **Promise:** one complete, hostname/key/profile-valid, time-usable protected
  Public Certified Key generation is current and can yield an authorized opaque
  resolver snapshot to `GatewayApplicationOwner` without exposing secret bytes.
- **Affected state:** Certified Key Usable/current custody generation only.
- **Failure projection:** missing key, key/cert mismatch, untrusted/expired
  chain, superseded generation, cancellation, rustls-build/seal failure,
  persistence failure, or custody read failure leaves the prior durable/live
  usable generation unchanged; no alternate CA/plaintext fallback and no
  independent resolver swap. Late success cannot overwrite a newer generation.
- **Post-publication status failure:** retain the newly published Current/Usable
  generation, retry the redacted status write on custody's private maintenance
  cadence, and do not turn the committed install into an error or roll it back.
- **Explicitly unaffected:** workload `Running`, Service `Stable`, backend
  eligibility, and existing established connections.
- **Ordering:** `ManualCertifiedKeySource` validates the whole pair → custody
  builds usable rustls material and seals the complete candidate → persists the
  complete `PublicCertifiedKeyV1` → atomically publishes the opaque
  Current/Usable generation (whose publication wakes the subscribed
  `GatewayApplicationOwner`) → writes `PublicCertifiedKeyStatusRowV1`.
  `GatewayApplicationOwner` binds the published snapshot with Route/frontend
  and atomically publishes the whole admission object independently of status
  repair.
- **Restart adoption:** restart adopts only a complete persisted generation
  after decrypt/profile/time revalidation and successful rustls construction.
  Before persistence it retains the prior generation; after persistence but
  before live publication it adopts the complete new record; after publication
  but before status it adopts the same new record and repairs status.
- **Counterexample:** treating Route intent commit as “certificate applied”
  would report public readiness while every browser handshake still fails.
- **Evidence lane:** pure certified-key validation decisions; stale-generation
  replacement property; secret-redaction checks; integration with
  `GatewayApplicationOwner`.

### Gate G-3 — Gateway Application generation applied

- **Existing evidence:** no Public Route Set/Gateway Application runtime owner exists. Current Service
  intent plus listener/VIP allocation can resolve an exact Service frontend;
  backend application belongs to ServiceMapHydrator/Dataplane.
- **Owner:** `GatewayApplicationOwner`.
- **Promise:** one immutable candidate coherently binds the
  accepted Public Route Set generation, resolved exact Service Frontend, and a
  custody-authorized opaque resolver snapshot for one Public Certified Key
  generation. One atomic pointer is published and retained per accepted
  connection. It makes no BPF-generation-currentness statement.
- **Affected state:** Gateway Applied Generation/new-connection admission.
- **Failure projection:** invalid/ambiguous Route, unresolved Service frontend
  or certified-key reference, unavailable intent/reference read, custody
  snapshot failure, cancellation, or supersession never partially
  applies a gateway generation. Store/custody failure is not “empty.”
- **Explicitly unaffected:** allocation `Running`, Service `Stable`, backend
  eligibility ownership, in-flight requests already holding an older snapshot.
- **Ordering:** read accepted Public Route Set → resolve exact Service Frontend
  and corresponding usable Public Certified Key → obtain opaque resolver
  snapshot from custody → construct one whole object → atomic pointer swap →
  publish Gateway Applied Generation. The new frontend's derived demand is
  published before this swap; the old frontend's demand remains until every
  connection retaining the old admission object drains. BPF hydration remains
  independently generated: a new request that races its convergence receives
  `NoBackend`→503 rather than a userspace fallback.
- **Counterexample:** separate Route/key publication could admit a torn
  connection; the single whole-object swap prevents it.
- **Evidence lane:** pure/property object construction, seeded coherent-
  generation invariant, concurrent hot-swap integration.

### Gate G-4 — Gateway BPF selection receipt established

- **Existing evidence:** ServiceMapHydrator consumes `ServiceBackendRow`, but
  production removes every Path-A/workload-subnet backend from both LB paths,
  sends the same-host non-mesh case to `LOCAL_BACKEND_MAP`, and passes only
  remote survivors to `SERVICE_MAP`. The single-node walking skeleton expects
  no BACKEND_MAP/SERVICE_MAP call. No gateway traffic surface exists.
- **Owner:** extended ServiceMapHydrator/Dataplane for applied maps and the
  `BackendId`→exact applied `Backend.alloc` association; gateway connector owns
  the socket; the inherited cgroup hook owns this socket's selection/rewrite.
- **Promise:** exact Service Frontend was BPF-selected/DNATed and the receipt
  identifies its actual `BackendId`; Dataplane resolves that non-recycled id to
  the exact applied workload identity before returning the connected socket.
- **Affected state:** current internal socket outcome only.
- **Failure projection:** unresolved frontend prevents application. Only the
  cgroup-BPF `NoBackend` receipt, including an empty eligible set/no selectable
  BPF backend, yields 503. A
  `Selected` receipt with missing or mismatched applied identity, or any
  hook/map/receipt inconsistency or connect failure, yields typed 502 and never
  pass-through.
- **Explicitly unaffected:** Gateway Applied Generation, allocation `Running`,
  Service `Stable`, backend eligibility, and BPF generation ownership.
- **Ordering:** Gateway Application publishes exact live-frontend demand →
  ServiceMapHydrator TEACHes that frontend → Dataplane takes its commit guard,
  validates and reserves each BackendId→`Backend.alloc` association as
  non-readable before BPF work → performs fallible BACKEND/private-inner/
  reverse-NAT work → the outer Service-map pointer swap commits the selectable
  generation → Dataplane infallibly publishes the reservations
  Applied and releases the guard → connector creates a socket and obtains
  `SO_COOKIE` → Dataplane registers `(cookie, ServiceKey)` →
  `connect(2)` enters the already-attached hook → hook validates that intent,
  selects/rewrites or records `NoBackend` and denies → connector consumes the
  receipt; a post-swap identity read waits for the commit guard and can observe
  only Applied → connector always removes the intent before it returns
  fd+identity or error. Any pre-commit failure restores prior state and exposes
  no identity from the abandoned reservations.
- **Counterexample:** reading `ServiceBackendRow` and selecting a backend in the
  HTTP handler could disagree with Maglev/SERVICE_MAP and is the forbidden
  second load balancer.
- **Evidence lane:** pure selection/receipt association, seeded generation-
  consistency, Tier-3 hook/packet/receipt proof and `AT-PIG-E2E-1`.

### Gate G-5 — Gateway-SVID selected-peer mTLS authenticated

- **Existing evidence:** `Backend.alloc` exists in the ServiceLifecycle input
  from which BPF state is hydrated, and `expected_peer` models SAN pinning, but
  BPF—not the gateway—selects the backend. Current BPF values/connection path do
  not prove how that actual choice reaches the verifier; the current mTLS client
  credential surface is allocation-keyed.
- **Owner:** gateway identity holder supplies client material; Dataplane receipt
  owner supplies expected peer; gateway internal transport owns handshake.
- **Promise:** gateway SVID is used as client material and the peer SPIFFE
  URI SAN equals the workload identity actually selected by BPF before any
  application body is delivered.
- **Affected state:** current internal request result only.
- **Failure projection:** missing gateway material/receipt identity,
  wrong-valid-peer, timeout, disconnect and cancellation fail closed with no
  plaintext/retry; application DESIGN pins exact error types.
- **Explicitly unaffected:** allocation `Running`, Service `Stable`, backend
  health, public TLS certificate status, and future request selection.
- **Ordering:** gateway material + exact
  frontend → cgroup BPF actual selection/DNAT → actual Selected Peer Identity → mTLS
  expected-peer verification before body delivery, within the one request
  budget and without silently multiplying deadlines.
- **Counterexample:** chain-to-bundle alone would accept a different valid
  workload certificate after a routing/index collision, crossing a public
  security boundary with the wrong application peer.
- **Evidence lane:** wrong-valid-peer negative, selected receipt equality,
  Tier-3 wire proof, and full `AT-PIG-E2E-1`.

### Required boundary scenarios

DISTILL covers available/success, unavailable or
timeout with cause-distinct failure, explicitly unaffected lifecycle progress,
late success after supersession, disconnect/reconnect or restart/replay, and
feature-disabled behavior. Ordering-dependent behavior requires seeded
`overdrive-sim` invariants; socket, public Web PKI, netns/TPROXY, kTLS, and wire
effects require the built default-feature binary on the real-kernel/native
lane. A test CA is permitted only as the hermetic TLS mechanism and does
not satisfy the production public-trust claim.

Because the selected design architecturally grows `cgroup_connect4_service`,
DELIVER must run the repository's real-kernel verifier-regression and verifier-
budget gates plus the existing cgroup-connect performance lane. Any baseline
change requires the measured kernel record and review mandated by `bpf.md`; the
design does not pre-authorize a numeric bump. The new selection executes once
per upstream socket connect, not once per HTTP body chunk.

## Wave: DESIGN / [REF] Component decomposition

These are responsibilities, not approved Rust types or module names.

| System responsibility | Likely existing owner/path | Change | Boundary handed forward |
|---|---|---|---|
| Public listener | `GatewayApplicationOwner` / `GatewayHandle` under `run_server_with_obs_and_drivers` / `ServerHandle` | EXTEND | Dynamic TCP/443 bind only after all gateway/dataplane/identity prerequisites; owned accept-stop/drain/unbind |
| `PublicCertifiedKeyCustody` | `overdrive-gateway::certified_key` + `overdrive-host::ca::PublicCertifiedKeyAeadCodec` | CREATE NEW stable custody owner over existing KEK/AEAD mechanism | `ManualCertifiedKeySource` installs; custody validates/builds+seals, persists `PublicCertifiedKeyV1`, publishes Current/Usable, then writes `PublicCertifiedKeyStatusRowV1`; paths/raw PEM/secrets stop before Route/listener/status |
| Public TLS + HTTP/1.1 runtime | Existing rustls/hyper libraries | CREATE NEW in-process runtime | Active after prerequisite gate; bounded streaming/drain |
| `GatewayApplicationOwner` | `PublicRouteSetV1` + `IntentServiceFrontendResolver` + custody | CREATE ACTIVE OWNER | Atomic admission object retained per connection; sole writer of `GatewayApplicationStatusRowV1` |
| Live Service-frontend demand | Staged + current + draining coherent Gateway Application generations | CREATE NEW derived in-process boundary | ServiceMapHydrator TEACHes only demanded Path-A frontends; superseded staged demand withdraws, and old applied demand withdraws after old admission connections drain |
| Eligible backend publication | ServiceLifecycle → `ServiceBackendRow` | REUSE UNCHANGED AS INDIRECT AUTHORITY | Gateway is not a consumer; ServiceMapHydrator remains the downstream consumer |
| BPF backend application and selection | ServiceMapHydrator → Dataplane/EbpfDataplane | EXTEND (TEACH) | Path-A enters the existing `SERVICE_MAP` Maglev table and `BACKEND_MAP`; registered socket-cookie intent/outcome plus applied BackendId→`Backend.alloc` association are added inside Dataplane ownership |
| `GatewayIdentitySlot` + `GatewaySvidLifecycle` | Existing CA/`issue_and_audit`/identity patterns | CREATE NEW single-slot sibling and pure lifecycle | Own issue/audit/hold/use/reissue/restart/drain-ordered drop for gateway subject; only `HostGatewayClientMtls` receives the private use snapshot; never AllocationId |
| `GatewayUpstreamConnector` + `GatewayConnectDataplane` | Existing ancestor-attached cgroup hook, `SERVICE_MAP` Maglev inner/`BACKEND_MAP`, `SocketCookieReader`, `HostGatewayClientMtls` | CREATE connector/registry; EXTEND hook | Register only cookie+ServiceKey; BPF selects; Dataplane supplies actual applied identity; connector transfers fd+SpiffeId into exact-peer mTLS; no userspace LB |
| Gateway status | `GatewayControl` + exact ObservationStore point reads | EXTEND existing operator-mTLS `GET /v1/gateway/status` | Projects `GatewayApplicationStatusRowV1`, `PublicCertifiedKeyStatusRowV1`, non-secret identity/connect facts and relevant hydration into `GatewayStatusResponse`; 500 on observation read error |

## Wave: DESIGN / [REF] Driving ports

| Driving surface | System responsibility | Initial constraint |
|---|---|---|
| `overdrive serve` | Compose/probe prerequisites, bind TCP/443, own runtime/drain | Existing production entry point |
| `overdrive deploy <SERVICE_SPEC>` | Produce the existing Service/listener/workload intent and resulting ServiceLifecycle backend observations | Existing surface unchanged |
| `overdrive deploy <ROUTE_SPEC>` | Produce `PublicRouteSetV1` under [ADR-0106](../../product/architecture/adr-0106-public-ingress-route-and-gateway-application-owner.md) through the [exact Application driving-port contract](#wave-design--ref-exact-driving-and-driven-ports) | Required operator journey; workload deploy arms remain unchanged |
| Operator-mTLS `GET /v1/gateway/status` | `GatewayControl` reads active application/custody plus relevant BPF hydration status on control-plane HTTPS | `GatewayStatusResponse`; 200 when observations are readable, 500 on read error; separate from public response status |
| Public TLS + HTTP/1.1 TCP/443 | Serve external users | Active after prerequisite gate |
| Manual certified-key publication | `ManualCertifiedKeySource` validates files and calls producer-neutral `PublicCertifiedKeyCustodyHandle::install` | Custody builds+seals, persists complete `PublicCertifiedKeyV1`, publishes Current/Usable, then writes redacted status; handlers/status never read paths/raw PEM |
| Exact Service Frontend handoff | Gateway connector registers socket cookie + exact ServiceKey, then dials inside the existing cgroup attach scope | Already-attached BPF hook selects/rewrites; no backend input |

## Wave: DESIGN / [REF] Driven ports and adapters

| Driven effect | Existing mechanism / proposed adapter | System constraint |
|---|---|---|
| Desired Route/reference reads | `IntentStore` `PublicRouteSetV1` + `IntentServiceFrontendResolver` | Persist Route inputs; resolve exact Service listener/VIP; no backend-row read |
| Public Certified-Key Custody | `PublicCertifiedKeyCustody` + `PublicCertifiedKeyAeadCodec` + `IntentStore` `PublicCertifiedKeyV1` | Validate/build+seal, persist complete generation, publish opaque Current/Usable, then write only redacted `PublicCertifiedKeyStatusRowV1`; status failure retries without rollback |
| Gateway Application publication/status | `GatewayApplicationOwner` Current `ArcSwapOption` + `ObservationStore` `GatewayApplicationStatusRowV1` | Atomic admission pointer retained per connection; owner writes active listener/staged/current/draining/redacted failure status |
| Service frontend resolution | Existing Service listener intent + platform VIP assignment/identity derivation | Resolve exact `(VIP, port, protocol)` only; do not join backend rows in the gateway |
| Live Service-frontend demand | Gateway Application → ServiceMapHydrator | Derived staged+current+draining frontend set only; no backend rows, no persistence as truth |
| Gateway BPF connect/receipt | Extended existing cgroup/connect4 + Dataplane-owned cookie intent/outcome and applied-id association | Register exact ServiceKey, select/rewrite from existing maps, consume actual selected identity; no raw handler map access |
| Public TLS/HTTP | rustls/hyper | TLS 1.3 + bounded HTTP/1.1 |
| Gateway SVID lifecycle/hold/use | `GatewaySvidLifecycle` + action executor + `GatewayIdentitySlot` | `ensure_current` before bind, restart/near-expiry reissue, and `disable_after_drain`; no public/private-material getter |
| Selected-peer mTLS | `HostGatewayClientMtls` implementing `GatewayClientMtls` | Present `GatewayIdentitySlot` SVID and require exact receipt-selected workload SAN; transparent kTLS/`MtlsEnforcement` is not this L7 client's API |
| Clock/entropy | Existing injected `Clock` plus `PublicCertifiedKeyAeadCodec` ring `SystemRandom` | ADR-0107/0109/0110 own the custody, SVID and public-runtime choices; exact clock/entropy contracts are in [Application persistence/custody](#wave-design--ref-persistence-custody-and-redaction), [Gateway Application/status/HTTP](#wave-design--ref-gateway-application-status-and-http-contract), and [gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration) |

## Wave: DESIGN / [REF] Technology choices

| Choice | Pin / status | Reason and honesty boundary |
|---|---|---|
| Rust | Workspace Rust 2024 | Existing single-process architecture and type/port discipline |
| Tokio | Workspace runtime | Existing lifecycle/cancellation owner |
| `hyper` / `hyper-util` | Workspace `1` / `0.1` | Embedded bounded HTTP/1.1 runtime |
| `rustls` | Workspace `0.23`, provider `ring` | Public TLS + internal gateway mTLS; no FIPS claim |
| Existing workload-identity CA | ADR-0063/0067 | Issuer foundation reused by dedicated gateway holder; never public Web PKI |

No cache, queue, load balancer, sharding, replication, CDN, DNS provider, or
external proxy is added: the one-node/one-origin envelope has no bottleneck
that justifies them.

### Architecture enforcement

- Preserve the repository's single-process hexagonal boundary: core policy and
  snapshot compilation depend on ports; `hyper`, `rustls`, sockets, storage
  and kernel effects stay in host/wiring adapters.
- `cargo xtask dst-lint` remains the Rust structural enforcement tool; the
  [Application component decomposition](#wave-design--ref-application-component-decomposition)
  and [Updated Reuse Analysis](#wave-design--ref-updated-reuse-analysis) pin its
  gateway/dataplane/core dependency and forbidden-import extensions.
- The gateway runtime cannot import/invoke ServiceLifecycle policy or consume
  `ServiceBackendRow` for selection. Only ServiceMapHydrator consumes the
  accepted backend projection to drive Dataplane application.
- Public certified-key secrets cannot appear in status,
  `Debug`, telemetry, or request-handler data shapes.
- The Application [exact driving/driven ports](#wave-design--ref-exact-driving-and-driven-ports),
  [persistence/custody](#wave-design--ref-persistence-custody-and-redaction),
  [Gateway Application/status/HTTP](#wave-design--ref-gateway-application-status-and-http-contract),
  and [gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration)
  sections are the exact API/store SSOT. These System labels authorize no aliases
  or additional public methods, rows or adapters.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing component | File / accepted record | Overlap | Decision | Justification |
|---|---|---|---|---|
| `run_server_with_obs_and_drivers` / `ServerHandle` | `crates/overdrive-control-plane/src/lib.rs` | Composition, probes, task ownership, bounded shutdown | **EXTEND** | The gateway is node infrastructure in the same process/failure domain; a second supervisor duplicates lifecycle ownership. |
| `Kek`/typed-codec protection pattern | ADR-0063/0048, `overdrive-host::ca` | Protect canonical secret input | **EXTEND per [ADR-0107](../../product/architecture/adr-0107-public-certified-key-custody-and-preserve-old-replacement.md)** | Exact `PublicCertifiedKeyAeadCodec`/record contracts live in [Application persistence/custody](#wave-design--ref-persistence-custody-and-redaction); the private AEAD engine is reused with distinct KEK/HKDF/AAD and public Web-PKI remains separate from both private CAs. |
| `IdentityMgr` / `IdentityRead` | ADR-0067 | Allocation-keyed holder pattern | **REUSE PATTERN, CREATE SEPARATE GATEWAY SLOT** | Avoids corrupting AllocationId API while reusing locking/redaction/bundle discipline |
| `ServiceLifecycle` / `ServiceBackendRow` | ADR-0101, `service_lifecycle.rs` | Backend membership and eligibility | **REUSE AS INDIRECT AUTHORITY** | Sole health publisher; gateway is not a backend-row consumer. |
| `ServiceMapHydrator` | ADR-0042/0101/0053 | Sole backend application owner | **EXTEND DEMAND-GATED TEACH** | Replace Path-A GATE only for exact live gateway frontend demand; apply its Path-A set to `SERVICE_MAP` plus Dataplane identity association; other Path-A remains gated and gateway still does not consume rows |
| `ListenerFactStore` | ADR-0062 | ServiceId-to-listener projection and workload cleanup index | **DO NOT EXTEND** | [ADR-0106](../../product/architecture/adr-0106-public-ingress-route-and-gateway-application-owner.md) owns Route/application resolution; the [exact driven-port contract](#wave-design--ref-exact-driving-and-driven-ports) pins `IntentServiceFrontendResolver` over `WorkloadIntent` + `ServiceVipView`, because ListenerFactStore has the wrong derived key/absence semantics. |
| mTLS `client_config`/SPIFFE verifier + reserved `expected_peer` contract | ADR-0069/0071, `mtls/tls_config.rs`, `mtls_enforcement.rs` | rustls chain validation, one-SPIFFE-URI parsing, required intended-peer shape | **REUSE VERIFIER; EXTEND EQUALITY; CREATE GATEWAY-CLIENT ENTRY UNDER SAME OWNER** | Current production is authn-only (`expected_peer=None`) and the public `MtlsEnforcement` port requires an intercepted leg + `AllocationId`. The L7 gateway must not fabricate either or route through the transparent kTLS pump; its new client entry consumes gateway-held material + receipt-selected identity and requires exact SAN equality in the reused verifier. |
| `rustls` + `hyper` server experience | `tls_bootstrap`, control-plane listener | TLS config/server serving/drain mechanics | **REUSE LIBRARIES/PATTERN** | Public cert trust/authority and HTTP proxy behavior are new; the operator CA/router are not reusable identities or handlers. |
| `cgroup_connect4_service` + shared Service maps | ADR-0053/0040 | Host connect-time destination rewrite; ancestor attach already covers `serve` | **EXTEND TEACH + REGISTERED RECEIPT** | Empirically fires for Path-A. Cookie-scoped intent selects the new Service-map arm; unregistered sockets preserve current `LOCAL_BACKEND_MAP` behavior; no child cgroup/netns/XDP entry is needed. |

Every CREATE NEW row in component decomposition exists because no current owner
has that responsibility. No new deployed process or datastore is justified;
`PublicCertifiedKeyV1` and the two status tables extend the existing
IntentStore/ObservationStore.

## Wave: DESIGN / [REF] Decisions table

| ID | Locked system-level verdict | Owning decision record(s) |
|---|---|---|
| D-SYS-1 | `serve` owns working in-process gateway, connector, listener and drain | [ADR-0104](../../product/architecture/adr-0104-embed-single-node-public-ingress-in-overdrive-serve.md) |
| D-SYS-2 | IPv4-A/TCP/443 binds after initial application/BPF/identity prerequisites | [ADR-0121](../../product/architecture/adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md) |
| D-SYS-3 | `PublicCertifiedKeyCustody` validates/builds+seals, persists complete `PublicCertifiedKeyV1`, atomically publishes the opaque Current/Usable snapshot, then writes `PublicCertifiedKeyStatusRowV1`; `GatewayApplicationOwner` atomically publishes Route/frontend/resolver admission and writes `GatewayApplicationStatusRowV1`; ACME #57 later uses the same install boundary | System boundary [ADR-0117](../../product/architecture/adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md); custody [ADR-0107](../../product/architecture/adr-0107-public-certified-key-custody-and-preserve-old-replacement.md) |
| D-SYS-4 | Public Web-PKI certified key, operator HTTPS CA, and internal workload/gateway SPIFFE CA remain three distinct credential domains | [ADR-0117](../../product/architecture/adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md), [ADR-0120](../../product/architecture/adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md) |
| D-SYS-5 | Dedicated gateway identity lifecycle/holder owns non-allocation SVID issue/hold/use/reissue/drop | System trust [ADR-0120](../../product/architecture/adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md); realization [ADR-0109](../../product/architecture/adr-0109-dedicated-gateway-svid-lifecycle-and-exact-peer-mtls.md) |
| D-SYS-6 | Connector registers `(SO_COOKIE, ServiceKey)` with Dataplane, then the already-attached cgroup BPF hook selects/rewrites through `SERVICE_MAP` Maglev inner + `BACKEND_MAP` and records actual BackendId | System packet entry [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md); realization [ADR-0108](../../product/architecture/adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md) |
| D-SYS-7 | Gateway Application publishes only exact live-frontend demand; ServiceLifecycle publishes eligibility; ServiceMapHydrator demand-gates Path-A TEACH; under the Dataplane guard identity is Reserved before BPF work, the outer swap commits selection, then identity becomes Applied; gateway has no backend watch/cursor | [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md), [ADR-0108](../../product/architecture/adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md) |
| D-SYS-8 | Gateway Applied and BPF applied generations remain independent; the transient receipt pins actual BackendId and its identity read blocks on the commit guard until the process-lifetime association is Applied | [ADR-0119](../../product/architecture/adr-0119-keep-gateway-and-service-dataplane-generations-independent.md) |
| D-SYS-9 | Strict HTTP/1.1; one BPF-owned frontend handoff; no retry/replay/userspace LB | [ADR-0110](../../product/architecture/adr-0110-bounded-public-tls13-http11-runtime.md), [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md) |
| D-SYS-10 | Public 404 Route miss; only `NoBackend`, including an empty eligible set/no selectable BPF backend, is 503; selected-receipt identity missing/mismatch and connect/receipt/mTLS/upstream failures are 502; separate operator-mTLS `GatewayControl` status reads active application/custody/hydration observations as `GatewayStatusResponse` 200/500 | HTTP [ADR-0110](../../product/architecture/adr-0110-bounded-public-tls13-http11-runtime.md); status [ADR-0111](../../product/architecture/adr-0111-redacted-operator-gateway-status.md); receipt identity [ADR-0114](../../product/architecture/adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md) |
| D-SYS-11 | Boot sentinel `NoBackend` may prove connect/receipt capability before Gateway Identity Current; actual-frontend `NoBackend` is bind-valid/503-capable, while `Selected` waits for Gateway Identity Current and exact-peer mTLS; graceful drain joins connector/listener and clears receipts | [ADR-0121](../../product/architecture/adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md) |

## Wave: DESIGN / [REF] System traceability

| System obligation | Current authority/evidence | Status |
|---|---|---|
| Node-agent-embedded `hyper` + `rustls`, in-process BPF access | GH #54 (no comments) + [ADR-0104](../../product/architecture/adr-0104-embed-single-node-public-ingress-in-overdrive-serve.md) | Feature intent plus Proposed single-node/process placement; gateway absent in current production |
| Production public trust from manual Public Certified Key producer | Explicit user ruling + [ADR-0117](../../product/architecture/adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md) + [ADR-0107](../../product/architecture/adr-0107-public-certified-key-custody-and-preserve-old-replacement.md) | `ManualCertifiedKeySource` → `PublicCertifiedKeyCustodyHandle::install` → validate/build+seal → persisted `PublicCertifiedKeyV1` → Current/Usable opaque resolver publication → redacted status |
| Gateway observability | [ADR-0111](../../product/architecture/adr-0111-redacted-operator-gateway-status.md) | `GatewayControl` uses the [exact Application status contract](#wave-design--ref-gateway-application-status-and-http-contract) for ObservationStore point reads of `GatewayApplicationStatusRowV1`/`PublicCertifiedKeyStatusRowV1` and relevant hydration; 200/500, secrets/raw dataplane failure strings omitted |
| ACME acquisition/renewal | Existing GH #57 | Excluded from first slice; additive producer only |
| Route resolves exact Service frontend | ADR-0049/0060/0062 + Route [ADR-0105](../../product/architecture/adr-0105-singleton-public-route-set-aggregate.md) + Application [ADR-0106](../../product/architecture/adr-0106-public-ingress-route-and-gateway-application-owner.md) | Existing inputs; Route/application resolution boundary Proposed |
| Backend Eligibility publication | ADR-0101 / ServiceLifecycle | Existing sole owner; unchanged |
| Backend-set application | ADR-0042/0101/0053 + [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md) + [ADR-0108](../../product/architecture/adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md) | ServiceMapHydrator remains sole owner; exact Gateway Application live-frontend demand triggers Path-A TEACH while undemanded Path-A stays gated; Dataplane records exact applied identity association |
| Backend selection/DNAT | ADR-0053 cgroup hook + ADR-0040 maps + [ADR-0118](../../product/architecture/adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md) | Existing ancestor hook gains a registered gateway-connect arm over the existing Maglev table and a cookie-keyed outcome; XDP wire path remains unchanged |
| Gateway/BPF consistency | [ADR-0119](../../product/architecture/adr-0119-keep-gateway-and-service-dataplane-generations-independent.md) | Gateway Applied and BPF Hydrated remain independently owned; one connect receipt correlates actual selection without a unified generation |
| Gateway SVID and selected-peer verification | ADR-0063/0067/0069/0071 + [ADR-0120](../../product/architecture/adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md) + [ADR-0109](../../product/architecture/adr-0109-dedicated-gateway-svid-lifecycle-and-exact-peer-mtls.md) | `GatewaySvidLifecycle`/`GatewayIdentitySlot` + Dataplane selected-id owner + `HostGatewayClientMtls`; exact signatures live in [Application gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration), with SAN equality and no fabricated Allocation identity/private-material getter |
| Public listener gates | [ADR-0121](../../product/architecture/adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md) | Gateway-owned bind/admission prerequisites never redefine Allocation Running, Service Stable, Backend Eligibility, BPF Hydrated or operator HTTPS readiness |
| Whitepaper §11 | Historical prose | Context only; non-authoritative |

## Wave: DESIGN / [REF] Handoff and open questions

This is the Stage-1 handoff record. Stage 2 resolves the DDD items below; the
application items remain required next-stage decisions, not permission for a
crafter to invent surface and not optional feature deferrals.

### DDD stage — binding constraints for Stage-2 remediation

- Keep the approved Public Ingress, Public Certified-Key Custody, Public Route
  Set/Public Certified Key and Gateway Application vocabulary. Amend the stale
  no-owner boundary: the Workload Identity context must own a dedicated,
  non-allocation, single-slot gateway identity lifecycle/holder; exact aggregate
  and occurrence names remain DDD's decision.
- **Resolved by the Public Route Set/Public Certified Key contracts:** pin stable identities/generations, exact hostname and segment-aware path
  value semantics, ambiguity/conflict behavior, and the operator-facing exact
  Service-listener reference without using a derived `ServiceId` as an
  accidental API.
- The gateway SPIFFE-ID is
  `spiffe://overdrive.local/gateway/<node-id>` and never an Allocation identity.
  Issuance audit remains the occurrence surface. Service Dataplane owns the
  transient gateway-connect intent/outcome and the BackendId→exact applied
  `Backend.alloc` association; Selected Peer Identity is a per-connection fact,
  never gateway intent or a backend choice.
- Gateway Application's exact staged+current+draining Service-frontends form derived
  live-consumer demand only. It is not a new aggregate or backend authority;
  superseded staged demand withdraws immediately and old applied demand
  withdrawal follows old-admission connection drain.

### Application architecture stage

- Pin the exact `overdrive deploy <ROUTE_SPEC>` discrimination, HTTP API, intent
  keys/envelopes/codecs, config grammar, status rows and typed errors. A missing shape is a DESIGN
  blocker, never implementation latitude.
- Pin exact signatures for the selected topology: gateway single-slot identity
  holder/use; ServiceMapHydrator TEACH; gateway connector socket creation and
  `SO_COOKIE` acquisition; Dataplane register/consume/cleanup of exact ServiceKey
  intent and BPF outcome; BackendId→exact applied `Backend.alloc` association;
  and a gateway-client entry under the existing mTLS transport owner carrying
  gateway material + exact expected peer. Do not add userspace selection, raw
  handler map access, a child-cgroup API, or a fabricated Allocation identity.
- Pin the in-process staged+current+draining live-frontend demand port and the
  Application-owner→ServiceMapHydrator wakeup. It carries exact Service
  Frontends only; it must not carry candidates or read `ServiceBackendRow`.
- Pin Public Certified-Key Custody and its protected canonical generation, manual
  operator producer, validation/redaction/authorization, opaque resolver-
  snapshot grant, expiry/restart behavior, and exact storage/API. Gateway
  `GatewayApplicationOwner` atomically publishes the whole admission object. Do not expose
  acquisition-specific file paths or raw PEM to the listener/request handler.
  ACME is outside this slice under GH #57 and may later publish the same input.
- Pin Route-intent change wakeup and List/Watch/relist mechanics. The existing
  generic observation interest router does not establish an IntentStore Route
  subscription.
- Pin strict HTTP proxy semantics: authority/SNI mismatch, upstream Host,
  `Forwarded`/`X-Forwarded-*`, hop-by-hop removal and `Via`, request/response
  deadlines, header/body/connection ceilings, cancellation, keep-alive snapshot
  rule, Service-frontend handoff, and exact status/error mapping without a
  gateway backend cursor.
- Pin `GatewayApplicationOwner`, independent BPF generation/status, selection
  receipt outcomes and 404/502/503 mapping. The boot sentinel may establish
  registered-connect/receipt capability before gateway identity; the actual
  frontend's `NoBackend` result permits bind and 503 service, while its
  `Selected` branch may attempt exact-peer mTLS only after Gateway Identity
  Current.

### DISTILL proof obligations already fixed by SYSTEM

- `GatewayApplicationGenerationCoherent`: each connection retains one whole
  Route/frontend/resolver object; BPF generation remains independent.
- `GatewayDoesNotSelectBackends` safety: no gateway path List/Watches
  `ServiceBackendRow`, maintains candidates/round-robin state, or mutates raw
  BPF maps; ServiceMapHydrator/Dataplane remain the only application path.
- `GatewayConnectIntentIsolation`: only a cookie whose registered ServiceKey
  exactly equals the connect destination enters the new Service-map arm;
  unregistered sockets retain current `LOCAL_BACKEND_MAP` behavior, and mismatch
  or missing/malformed outcome sends no upstream bytes.
- `GatewayFrontendDemandTeach`: staged/current/draining demand TEACHes only its
  exact Path-A frontend; superseded staged demand withdraws, applied demand
  survives old-connection drain, and every undemanded Path-A frontend remains
  gated.
- `DataplaneSelectedIdentityPinned`: consumed receipt BackendId resolves to the
  exact `Backend.alloc` used for that cgroup-BPF selection, and wrong-valid-peer
  fails before body delivery.
- `BackendIdIdentityImmutable`: a live BackendId can map to exactly one
  `Backend.alloc`; conflicting input is rejected before the Service-map swap and
  leaves the prior applied generation byte-equivalent.
- `GatewayNoBackendFailClosed`: a registered connect with no applied Service-map
  backend records `NoBackend`, denies unchanged-VIP fall-through and maps to one
  public 503; absent/inconsistent receipt or missing/mismatched identity after
  `Selected` maps to 502 with no plaintext retry.
- Real-kernel verifier and packet-entry evidence proves `SO_COOKIE` and
  `bpf_get_socket_cookie` correlate on the production attach scope, the
  map-of-maps lookup verifies, registered failure denies, the selected Path-A
  address reaches inbound nft-TPROXY, and unregistered local/non-service
  connects remain byte-for-byte behaviorally unchanged.
- Manual certified-key replacement atomically publishes a whole new Gateway
  Application object; invalid input preserves prior applied state.
- Feature disabled: existing `serve`, workload `Running`, Service `Stable`, and
  backend eligibility are behaviorally unchanged and no public socket binds.
- `AT-PIG-E2E-1` proves built binary + real `serve` + Service deploy +
  Route deploy + operator-supplied public Web-PKI certified key + external HTTPS
  client + actual Service frontend/cgroup-BPF-selected-backend/gateway-SVID mTLS
  path and response. Hermetic tests use a test CA; production EDD proves trust.

### Explicit acquisition exclusion

- Automated ACME acquisition/renewal is not first-slice behavior. Existing GH
  #57 owns it. Its future producer must publish the same protected canonical
  certified-key input and must not change TCP/443 listener, Route matching, or
  request/dataplane ownership.

## Wave: DESIGN / [REF] Domain decisions

Stage 2 preserves D-SYS-1…D-SYS-11 and records the Route choice in
[ADR-0105](../../product/architecture/adr-0105-singleton-public-route-set-aggregate.md)
plus independently reversible Domain choices in
[ADR-0112](../../product/architecture/adr-0112-public-certified-key-custody-domain-boundary.md),
[ADR-0113](../../product/architecture/adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md),
[ADR-0114](../../product/architecture/adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md),
[ADR-0115](../../product/architecture/adr-0115-derived-gateway-frontend-demand-lifecycle.md)
and [ADR-0116](../../product/architecture/adr-0116-state-based-public-ingress-domain-ownership.md),
all **Proposed**. GitHub issue #54 plus the user's corrections govern; the
whitepaper is contextual, the missing DISCUSS/SPIKE artifacts are not invented,
and no absent story, KPI or capacity claim is inferred.

| ID | Domain verdict |
|---|---|
| **D-DOM-1** | Create **Public Ingress** and **Public Certified-Key Custody** bounded contexts; extend existing **Service Dataplane** and **Workload Identity** contexts. Public Route Set, Public Certified Key and Gateway Identity Slot are the only new aggregates. |
| **D-DOM-2** | A Route has one exact Public Hostname, one exact-or-segment-prefix Path Match, one exact TCP Service Listener Reference and one Public Certified-Key Reference. |
| **D-DOM-3** | Route intent references the stable Service listener as `(WorkloadId, port, tcp)`. Resolution yields the exact existing Service Frontend `(ServiceVip, port, tcp)`; the Route never stores derived `ServiceId`, VIP, backend address, Allocation ID or SPIFFE ID. |
| **D-DOM-4** | [ADR-0105](../../product/architecture/adr-0105-singleton-public-route-set-aggregate.md): the singleton Public Route Set makes first-slice one-Route cardinality/ownership atomic on one aggregate key. A different Route ID cannot claim an occupied slot; it is rejected rather than last-write-wins. |
| **D-DOM-5** | [ADR-0112](../../product/architecture/adr-0112-public-certified-key-custody-domain-boundary.md): create **Public Certified-Key Custody** with a state-based **Public Certified Key** aggregate. It owns protected, producer-neutral current certificate-chain/private-key material and atomic replacement. Manual operator input is the only first-slice producer. |
| **D-DOM-6** | Manual operator input is the sole first-slice Certified Key producer. ACME account/order/challenge/renewal and TCP/80 are absent; GH #57 may later invoke the same custody command without changing runtime consumption. |
| **D-DOM-7** | [ADR-0113](../../product/architecture/adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md): Workload Identity owns a dedicated single-slot **Gateway Identity Slot** and **Gateway Identity Lifecycle** for `spiffe://overdrive.local/gateway/<node-id>`. It reuses issue/audit/profile policy, renews/restarts/drops explicitly, permits only gateway-client mTLS use and never fabricates `AllocationId`. |
| **D-DOM-8** | [ADR-0115](../../product/architecture/adr-0115-derived-gateway-frontend-demand-lifecycle.md): `GatewayApplicationOwner` atomically publishes one Route/frontend/opaque-resolver generation and derives exact **Staged**, **Current** and **Draining Gateway Frontend Demand**. Demand is not persisted and carries no backend rows/candidates. |
| **D-DOM-9** | [ADR-0114](../../product/architecture/adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md): ServiceLifecycle remains sole Backend Eligibility publisher. ServiceMapHydrator demand-gates ADR-0053 TEACH; Service Dataplane owns BPF application, registered cgroup-BPF selection, BPF Selection Receipt and the immutable `BackendId → Backend.alloc` identity association. The gateway never selects or reads backend rows/maps. |
| **D-DOM-10** | A Gateway Connect Intent is exactly `(Socket Cookie, ServiceKey)`. `cgroup_connect4_service` chooses through existing `SERVICE_MAP` Maglev + `BACKEND_MAP`; its receipt is `Selected(BackendId)` or `NoBackend`. This host-connect path is distinct from unchanged XDP wire forwarding. |
| **D-DOM-11** | Selected Backend Identity is the receipt's `BackendId` resolved through Dataplane's exact applied association. Gateway-client rustls presents the Gateway SVID and requires exact peer SPIFFE SAN equality before body delivery. |
| **D-DOM-12** | `Route Accepted`, `References Resolved`, `Certified Key Usable`, `Gateway Identity Current`, `Gateway Frontend Demand Applied`, `Gateway Applied Generation`, `BPF Hydrated`, `Public Listener Bound` and `Selected Peer Authenticated` are distinct owned promises; none is shorthand for `Ready`/`Healthy`/`Programmed`. |
| **D-DOM-13** | [ADR-0116](../../product/architecture/adr-0116-state-based-public-ingress-domain-ownership.md): Public Route Set, Public Certified Key and Gateway Identity Slot are state-based; frontend demand, BackendId identity association and connect receipts are derived/process-local state. No Event Sourcing, CQRS subsystem, ACME workflow, new deployed process or userspace load balancer is introduced. |

## Wave: DESIGN / [REF] Bounded contexts and context map

| Bounded context | Classification | Owns | Explicit non-ownership |
|---|---|---|---|
| **Public Ingress** | Core, **CREATE NEW** | Public Route Set; Route acceptance/withdrawal; exact Service-frontend resolution; Gateway Application lifecycle; staged/current/draining frontend demand; Gateway Upstream Connector/socket; Public Listener and HTTP/1.1 semantics | Does not acquire certificates, decide Backend Eligibility, read backend rows, select backends, own BPF state or issue SVIDs |
| **Public Certified-Key Custody** | Supporting, **CREATE NEW** | Producer-neutral protected Public Certified Key aggregate; chain/key/hostname validation; preserve-old atomic replacement; validity/expiry currentness; redaction boundary | Has no first-slice withdrawal command; does not know manual paths after ingestion or own ACME account/order/renewal, Route, listener, request or internal SVID |
| **Workload Orchestration / Service Lifecycle** | Existing core, **REUSE** | Service aggregate/Listener intent; Allocation lifecycle; ServiceLifecycle Backend Eligibility publication | Does not expose a public Route, consume frontend demand or serve gateway requests |
| **Service Dataplane** | Existing core, **EXTEND OWNER** | Service Frontend/ServiceKey; demand-gated Path-A TEACH; BPF generations; Gateway Connect Intent/BPF Selection Receipt; process-lifetime Applied Backend Identity Association; cgroup-BPF selection and unchanged XDP forwarding | Does not own Route/key generation, public HTTP matching, gateway SVID custody or userspace selection |
| **Workload Identity** | Existing supporting, **EXTEND OWNER** | Existing CA/audit/allocation identity; dedicated Gateway Identity Slot/Lifecycle; gateway-client mTLS and exact selected-peer equality | Does not own public Web-PKI, Route, Backend Eligibility or BPF selection |

`IntentStore`, `ObservationStore`, holder state and reconciler View memory are
non-substitutable state contracts, not bounded contexts. Intent holds Public
Route Set; protected credential state holds Public Certified Key; Observation
holds Service/dataplane, custody/application and issuance-audit facts. Gateway
SVID material, frontend demand, BackendId identity association and connect
intent/receipt are process-local. Only retry inputs may survive in View memory;
derived deadlines and runtime snapshots are not persisted.

Public Ingress is new because no existing context owns public host/path exposure.
Public Certified-Key Custody is new because Web-PKI chain/private-key custody and
atomic replacement cannot inhabit either operator TLS or internal SPIFFE identity.
Service Dataplane and Workload Identity are extended rather than duplicated:
selection/identity relationships land with their accepted owners. The cgroup
hook selects this host-originated gateway socket; XDP remains the unchanged
wire-ingress path. Neither edge licenses gateway backend enumeration.

The detailed [Route/domain context map](../../product/architecture/c4-diagrams.md#public-ingress-gateway-route-domain-context)
is maintained only in `c4-diagrams.md`.

## Wave: DESIGN / [REF] Ubiquitous language

| Term | Pinned meaning | Explicitly not |
|---|---|---|
| **Public Listener** | `serve`-owned IPv4 TCP/443 socket role, bound only after all boot gates, terminating public TLS 1.3 and HTTP/1.1 | TCP/80, Service Listener/VIP, allocation socket or operator HTTPS listener |
| **Public Route Set** | Singleton first-slice intent aggregate/atomic slot, empty or occupied by one Route | Listener, runtime table or multi-route collection |
| **Route** | Public-exposure entity mapping Public Hostname + Path Match to one Service Listener Reference and one `PublicCertifiedKeyId` | Service field, backend list, BPF entry or certificate |
| **Route Generation** | Immutable identity of one canonical accepted Route state; semantic change replaces it, exact reapply retains it | Gateway Applied Generation or BPF Hydration Generation |
| **Public Hostname** | Lowercase canonical ASCII DNS name without wildcard, IP literal or terminal dot; one name in this slice | SPIFFE trust domain, Service name or `Host` including port |
| **Path Match** | `Exact(path)` or `SegmentPrefix(path)` over valid raw URI path before percent-decoding, excluding query/fragment; `/api` never matches `/apix` | Regex, rewrite, decoded filesystem path, method/header/query match |
| **Service Listener Reference / Route Target** | Stable desired reference `(WorkloadId, port, tcp)` to one listener in current Service intent | ServiceId, VIP, backend, Allocation ID or SPIFFE identity |
| **Service Frontend** | Resolved existing dataplane destination `(ServiceVip, listener port, tcp)` for one Service Listener | Backend address, public listener or userspace-selected peer |
| **ServiceKey** | Existing BPF lookup identity derived from exact Service Frontend `(VIP, port, tcp)` | ServiceId, Route Target, backend identity or public hostname |
| **Socket Cookie** | Kernel socket identity read through `SO_COOKIE` in userspace and `bpf_get_socket_cookie` in the cgroup hook for the same upstream socket | File descriptor, request ID, backend choice or durable identity |
| **Public Certificate** | Operator-supplied production Web-PKI chain binding Public Hostname to the Origin Private Key's public key | Private key, internal CA chain, SVID or aggregate |
| **Origin Private Key** | Protected private key corresponding to the Public Certificate | Workload/gateway SVID key, operator-client key or handler value |
| **Certified Key** | Complete matching Public Certificate + Origin Private Key after hostname/key/profile validation | Manual file path, raw PEM field, chain alone or rustls resolver table |
| **Certified Key Generation** | One atomically installed complete Certified Key plus non-secret provenance/validity inputs | Acquisition workflow state, Route Generation or BPF state |
| **Public Certified Key** | State-based custody aggregate identified by `PublicCertifiedKeyId` and holding zero or one current Certified Key Generation | Public Certificate chain alone, ACME order or runtime TLS snapshot |
| **Gateway Identity** | Required non-allocation subject `spiffe://overdrive.local/gateway/<node-id>` owned by the node's Gateway Identity Slot/Lifecycle | Allocation identity, public-user identity, Public Certified Key or operator certificate |
| **Gateway SVID** | One current internal certificate/private-key generation held in the dedicated single-slot holder and usable only by gateway-client mTLS | Persisted key, `IdentityMgr` allocation entry, public certificate or generic handler getter |
| **Backend Eligibility** | ServiceLifecycle's existing decision published through `ServiceBackendRow` | Gateway state, `Running`, `Stable` or request success |
| **Gateway Application Generation** | Immutable Route Generation + exact Service Frontend + opaque Certified Key resolver snapshot | Backend candidates, Gateway SVID material, receipt or BPF state |
| **Gateway Applied Generation** | Identity of the whole Gateway Application Generation atomically current for new accepted connections | BPF Hydration Generation or a backend-availability promise |
| **Gateway Frontend Demand** | Derived distinct set of exact Service Frontends referenced by Staged, Current or Draining Gateway Application generations | Route truth, backend row/set, health, persistence or userspace selection |
| **Gateway Connect Intent** | One transient Dataplane registration `(Socket Cookie, ServiceKey)` before connect | Backend candidate/id/identity, Route or durable state |
| **BPF Selection Receipt** | One transient registered-connect result `Selected(BackendId)` or `NoBackend`, correlated to the exact cookie and ServiceKey | Gateway-selected backend, Backend Eligibility or XDP packet fact |
| **Applied Backend Identity Association** | Dataplane-owned process-lifetime immutable `BackendId → exact applied Backend.alloc (SpiffeId)` join installed before Service-map swap | Address-based reread, Route state or allocation holder |
| **BackendId** | Existing process-local BPF backend token selected from Maglev and carried by the receipt | SPIFFE identity, AllocationId, address or gateway choice |
| **Selected Backend Identity** | The exact `Backend.alloc` resolved from the BPF receipt's actual BackendId for one connected socket | An arbitrary eligible backend, gateway prediction or asynchronous `BackendIndex` reread |
| **cgroup-BPF Service Selection** | Host-originated registered socket selection/rewrite by ancestor-attached `cgroup_connect4_service` through existing Maglev/BACKEND maps | XDP ingress, userspace round robin or new child cgroup/netns |
| **XDP Wire Forwarding** | Existing packet-ingress forward/reverse path on its current interfaces, unchanged by gateway selection | The gateway host-connect selection path |
| **BPF Hydrated** | ServiceMapHydrator/Dataplane applied its independent backend generation, including demanded Path-A TEACH | Gateway Applied Generation or proof a backend exists |
| **Selected Peer Authenticated** | Gateway-client mTLS verified exactly the receipt-selected workload SPIFFE SAN before application bytes | Chain-only verification, gateway preselection or persistent state |
| **Gateway Identity Current** | Gateway Identity Slot holds a time-usable SVID for the exact node gateway subject | Issuance-audit presence, allocation SVID currentness or public-key usability |

Precise promises are **Route Accepted**, **References Resolved**, **Certified
Key Usable**, **Gateway Identity Current**, **Gateway Frontend Demand Applied**,
**Gateway Applied Generation**, **BPF Hydrated**, **Public Listener Bound** and
**Selected Peer Authenticated**. Operator-mTLS gateway status is distinct from
public 404/502/503 responses. None is an alias for another or for a generic
`Ready`/`Healthy`/`Programmed` state.

## Wave: DESIGN / [REF] Aggregate contracts

### `Public Route Set` — singleton atomic Route boundary

The aggregate has one fixed gateway-wide identity; Stage 3 owns its exact key
representation. `RouteId` identifies the optional top-level Route entity, not an
independent aggregate key. Full observable state is:

`{ route: None | Some({ route_id, route_generation, public_hostname,
path_match, service_listener_reference, public_certified_key_id }) }` plus
ordered command facts.

**Invariants.** One store key owns the empty/occupied slot, so competing Route
IDs cannot both succeed. An occupied Route has exactly one hostname, Path Match,
Service Listener Reference and certified-key reference. Its target protocol is
TCP. Reference resolution proves current intent is Service kind, contains exact
`(port, tcp)`, has a platform-issued Service VIP, and yields exact Service
Frontend `(VIP, port, tcp)`. Certified-key reference resolution proves the
named aggregate exists and its hostname corresponds; **Certified Key Usable**
is a separate gate.

Matcher law is exact path before Segment Prefix at the same path, then longer
Segment Prefix before shorter. Equal-rank ambiguity is rejected if a later
approved envelope admits multiple candidates; this law does not lift the one-
Route bound.

| Command | Preconditions | Declared delta/fact order | Complement equality |
|---|---|---|---|
| **Declare Route** | Slot empty; values valid | `None → Some(full Route)`; `RouteDeclared`, then `RouteAccepted` | Service, Certified Key, Gateway Identity, Gateway Application/demand and BPF state unchanged until downstream owners react |
| **Replace Route** | Slot occupied by same Route ID; replacement valid | Replace hostname/path/target/key-ref + Route Generation atomically; `RouteReplaced`, then `RouteAccepted` | Aggregate identity, Route ID and all referenced/other state unchanged |
| **Withdraw Route** | Slot occupied by same Route ID | `Some → None`; `RouteWithdrawn` | Service, Certified Key, Gateway Identity and BPF state unchanged until downstream Gateway Application performs ordered admission/demand drain |
| **Declare different Route ID while occupied** | Occupied by another ID | Empty; cause-distinct slot conflict; no Accepted fact | Whole universe and all external state equal |
| **Idempotent/invalid command** | Same canonical state or invalid input | Empty; no fact for exact replay, cause-distinct rejection otherwise | Whole universe and all external state equal |

Vernon's rules hold: the true cardinality/ownership invariant is inside one
small root with at most one Route entity; references remain identity-only and
resolve outside its transaction.

### `Public Certified Key` — protected, producer-neutral custody aggregate

`PublicCertifiedKeyId` is stable and is the only Route credential reference.
Full authorized state is absent or
`{ certified_key_id, public_hostname, current_generation }`, where
`current_generation` is one complete
`{ certified_key_generation, protected_origin_key, certificate_chain,
not_before, not_after, nonsecret_provenance }`. Secret bytes are observable only
at the authorized custody/TLS boundary; status/facts expose identifier,
fingerprint/issuer and validity inputs, never key bytes.

**Invariants.** The leaf covers exactly Public Hostname, chain/profile is valid,
and leaf public key matches Origin Private Key. A generation replaces the whole
pair atomically. Replacement carries the expected current generation; an older
or racing producer cannot overwrite a newer one. `not_before`/`not_after` and
provenance are persisted inputs; `usable now` is derived from them plus live
clock/policy. No `valid`, acquisition state, manual path, raw handler PEM or
compiled rustls resolver snapshot is persisted as aggregate state.

| Command | Preconditions | Declared delta/fact order | Complement equality |
|---|---|---|---|
| **Install Certified Key** | Aggregate absent; candidate validates | Create full current generation; `CertifiedKeyInstalled` | Public Route Set, Service/dataplane and internal identity unchanged |
| **Replace Certified Key** | Expected generation is current; candidate validates and differs | Replace full generation atomically; `CertifiedKeyReplaced` | Public Certified Key ID/hostname and every external state unchanged |
| **Exact replay** | Candidate equals current generation | Empty; no fact | Whole universe and all external state equal |
| **Missing, stale, invalid or currently unusable manual refresh** | No acceptable replacement generation | Empty custody delta; cause-distinct install failure status only; retain the current generation while it remains usable | Current protected/live generation plus Route, Service/BPF and identity state |
| **Current generation reaches not-yet-valid/expired state** | Live clock makes current generation unusable | Retain protected record/generation; derive `Certified Key Usable = false` and publish Unusable status | Route and every non-custody aggregate; listener/application owner reacts through its own gate |

The first-slice manual operator producer ends at this aggregate command.
Acquisition-specific paths/PEM disappear at ingestion. GH #57 later may invoke
the identical command after a finite ACME workflow; no ACME state belongs here.

### `Gateway Identity Slot` — single non-allocation SVID aggregate

One in-process slot per node owns `Empty | Held(current Gateway SVID
generation)`. Desired identity is
`spiffe://overdrive.local/gateway/<node-id>` whenever the gateway is enabled.
Held material is a whole certificate/key generation with non-secret
subject/validity facts; private material stays encapsulated and is never
persisted.

The dedicated Gateway Identity Lifecycle converges enabled/disabled against
empty/held state. Enabled+empty issues; enabled+near-expiry reissues;
disabled+held drops. Issuance/reissuance reuses `issue_and_audit`; audit must
succeed before atomic hold/replace. Failed reissue preserves old material only
while time-usable. Restart begins empty and reissues before listener bind.
Shutdown stops admission/drains authenticated work before drop/zeroize. Only
gateway-client mTLS may consume an opaque use capability; allocation
`IdentityMgr`/`IdentityRead`, public TLS, Route, handlers and status cannot read
the material.

| Command | Delta/facts | Complement equality |
|---|---|---|
| Ensure Gateway SVID | Empty → fresh audited Held generation; GatewayIdentityHeld | Allocation held set, public key, Route and BPF |
| Reissue Gateway SVID | Held old → fresh audited Held new atomically; GatewayIdentityReplaced | Allocation held set and all non-gateway identity state |
| Drop Gateway SVID | Held → Empty after admission/drain ordering; GatewayIdentityDropped | Allocation SVIDs/audit history, public key and BPF |
| Failed issue/reissue | Empty, or old Held retained only while usable; cause-distinct failure | No unaudited material enters the slot |

### Explicit non-aggregates

| Concept | Why it is not an aggregate |
|---|---|
| **Public Listener** | `serve` lifecycle capability; it owns no durable domain state and binds only after all boot gates. |
| **Gateway Application Generation / Gateway Applied Generation** | Immutable derived application object and its current identity, rebuilt from authoritative Route/frontend/key inputs; never persisted truth. |
| **Gateway Frontend Demand** | Derived union of Staged/Current/Draining application frontends; never Route truth, backend set or store. |
| **Gateway Connect Intent / BPF Selection Receipt** | Transient per-socket registration/outcome values in the Dataplane registry; consumed and cleaned, never aggregate history. |
| **Applied Backend Identity Association** | Process-lifetime Dataplane projection from exact applied Backend values; not desired Route/Service state. |
| **Selected Backend Identity / Selected Peer Authenticated** | Per-connection selection/authentication facts, not desired state or aggregate. |
| **ACME account/order/challenge/renewal** | Outside first-slice scope under GH #57; no dormant workflow/domain model is introduced here. |

## Wave: DESIGN / [REF] Event model and lifecycle gates

| Lane | Command/trigger | Domain facts/status | Policy/effect |
|---|---|---|---|
| Route intent | Declare / Replace / Withdraw Route | `RouteDeclared/Replaced/Withdrawn`; `Route Accepted` + Route Generation | Singleton aggregate validates canonical values and slot ownership; current state persists through existing typed intent boundary |
| Reference resolution | Route Accepted, Service intent/VIP change, certified-key change, boot/relist | `RouteReferencesResolved` or cause-distinct unresolved; separate `Certified Key Usable` | Resolve only exact Service Frontend and corresponding certified-key ID/hostname; do not read ServiceBackendRow/BPF maps |
| Certified-key custody | Install / Replace from manual producer; clock validity transition | `CertifiedKeyInstalled/Replaced`; install-failure status; Certified Key Usable/Unusable + generation | Preserve-old validation/protection and atomic replacement; invalid/missing refresh never erases a usable current generation; no withdrawal command |
| Gateway identity | Ensure / Reissue / Drop; boot/clock/disable | Issued-certificate audit occurrence; GatewayIdentityHeld/Replaced/Dropped; Gateway Identity Current | Dedicated lifecycle converges one non-allocation slot; issue/audit precedes hold; only gateway-client mTLS may use material |
| Gateway application | Stage / Apply / Supersede / Withdraw / connection drain | Staged/Current/Draining demand; Gateway Applied Generation or cause-distinct unavailable | Publish staged demand before swap; retain current per connection; withdraw old demand only after drain |
| Backend application | Eligibility row or frontend-demand change | Backend Eligible; BPF Hydrated; Applied Backend Identity Association | ServiceMapHydrator demand-gates Path-A TEACH; Dataplane associates identity before Service-map swap |
| Gateway connect | Register `(Socket Cookie, ServiceKey)` / connect / consume | BPF Selection Receipt; Selected Backend Identity or NoBackend | Public Ingress connector owns socket/connect and consumes cleanup on every return; Dataplane registry/hook owns transient state and selection |
| Internal trust | Connected socket + selected identity + current gateway identity | Selected Peer Authenticated or cause-distinct failure | Gateway-client rustls requires exact peer SPIFFE SAN; no transparent pump, plaintext or alternate peer |
| Public request | TLS + retained Gateway Application generation + Host/path | 404 Route miss; 503 NoBackend; 502 connect/receipt/identity/mTLS/upstream; upstream response otherwise | Public Runtime streams HTTP/1.1 with no retry/replay/userspace backend choice |

### Lifecycle boundary scenarios

| Scenario | Required outcome | Explicitly unaffected |
|---|---|---|
| **Feature disabled** | No Route owner, key consumer, frontend demand, gateway identity slot/lifecycle, connect registry or public listener activates | Existing `serve`, operator HTTPS, Allocation `Running`, Service `Stable`, Backend Eligibility, cgroup/XDP and workload SVID behavior |
| **Enabled cold boot** | Load/validate Route/key, resolve frontend, issue/audit/hold gateway SVID, stage and apply frontend demand, probe receipt/identity path, apply Gateway Application, then bind TCP/443; no earlier partial listener | Workload/Service lifecycles and independent dataplane convergence do not wait on public listener bind |
| **Route before target/key** | Route may be Accepted while References Resolved/Certified Key Usable/Application Applied remain absent with cause; listener stays unbound | Referenced Service, BPF and key aggregate are not mutated by Route acceptance |
| **Competing Route IDs** | One singleton-slot mutation wins; loser receives conflict with empty delta | Existing Route and all Service/key/identity/dataplane state |
| **Late old certified-key replacement** | Expected-generation check rejects stale candidate; cannot overwrite newer generation | Public Route Set, current key and BPF state complement-equal |
| **Route/frontend replacement** | Staged demand precedes admission swap; old generation becomes Draining; same-frontend reference counts prevent premature demand withdrawal | ServiceLifecycle authority and BPF generation remain independent |
| **Route deletion** | Withdraw Route intent only; stop new admission/accepts, move current application to Draining, retain frontend demand until accepted work drains, then retire demand/unbind | Protected Public Certified Key remains in custody; Service Listener intent, Backend Eligibility, gateway/allocation SVIDs and operator HTTPS |
| **Certified-key refresh failure or expiry** | Missing/invalid/unusable refresh preserves the usable current generation. When that generation itself becomes Unusable/expired, stop new admission/TLS and publish the pinned status while retaining the protected record for a valid replacement | Route intent, Service/Backend Eligibility/BPF and Gateway Identity |
| **Gateway identity disable/expiry** | Stop new admission/upstream handshakes, drain authorized users, then drop/replace gateway SVID material | Route/Public Certified Key, Service/Backend Eligibility/BPF and allocation SVIDs |
| **No backend** | Listener remains active when capability gates are sound; BPF receipt is NoBackend, connect is denied and public response is 503 | Route/key/identity generations and backend authority |
| **Missing/mismatched receipt** | No upstream bytes; cleanup intent/outcome and return 502 | Existing unregistered LOCAL_BACKEND_MAP/XDP traffic |
| **Wrong selected peer / disconnect** | Current request returns 502 before upstream body when possible; no plaintext, retry or alternate backend | Future requests and all lifecycle state |
| **Process restart** | Destroy connections/receipts before rebuilding BackendId associations and gateway SVID; rerun all boot gates; bind TCP/443 only after success | No cross-owner unified generation or BackendId rebinding |
| **Shutdown** | Stop accepts, withdraw current admission, drain with generation demand/identity retained, join listener/connector, clear receipts, withdraw demand, then drop gateway SVID | Existing workload/Service shutdown ownership |

**Gate ownership remains narrow.** Initial listener bind requires References
Resolved, Certified Key Usable, Gateway Identity Current, Gateway Frontend
Demand Applied, Gateway Connect Path Trusted and Gateway Applied Generation.
Backend presence is not a bind gate: its absence is a valid NoBackend receipt
and public 503. Per-request Selected Peer Authenticated is not a global
generation. BPF Hydrated remains ServiceMapHydrator/Dataplane-owned. None
delays, advances, revokes or reinterprets
Allocation `Running`, Service `Stable`, Backend Eligibility, BPF hydration or
operator HTTPS readiness.

Gateway SVID issuance reuses the existing issued-certificate audit occurrence;
held/current is the in-memory slot and audit presence never substitutes for it.
Route/key/identity/application changes are state plus synchronous facts/status;
connect receipts and request outcomes are transient operational facts. Operator-
mTLS status stays distinct from public 404/502/503. No new event store is
introduced.

## Wave: DESIGN / [REF] Domain reuse and ES/CQRS assessment

| Existing concept/component | Overlap | Decision | Evidence/boundary |
|---|---|---|---|
| `WorkloadSpec::Service` + `Listener` | Stable target intent | **REUSE; DO NOT EMBED ROUTE** | Listener already owns `(port, protocol)`; public exposure has independent lifecycle |
| `WorkloadId`, VIP assignment, `ServiceId::derive`, `ListenerFactStore` | Resolve stable target to exact Service Frontend | **EXTEND READ PROJECTION** | Route persists no derived ServiceId/VIP; resolution never enumerates backends |
| `ServiceLifecycle` + `ServiceBackendRow` | Backend Eligibility | **REUSE AS INDIRECT AUTHORITY; NO GATEWAY READ** | Sole publisher; only ServiceMapHydrator consumes/applies it for this path |
| `ServiceMapHydrator` | Eligible-set application | **EXTEND — DEMAND-GATED ADR-0053 TEACH** | Only demanded Path-A enters shared Service maps; undemanded Path-A stays gated |
| `SERVICE_MAP` Maglev inner + `BACKEND_MAP` | Selection tables | **REUSE** | Existing BPF selection; no gateway backend map/algorithm |
| `cgroup_connect4_service` ancestor attachment | Host connect selection | **EXTEND — REGISTERED ARM/RECEIPT** | Already covers `serve`; unregistered sockets keep current `LOCAL_BACKEND_MAP` behavior and XDP wire path is unchanged |
| `BackendIdAllocator` | Selected backend identifier | **REUSE MONOTONIC-NONREUSE PROPERTY** | Process-lifetime non-rebinding is required for old receipts |
| Applied BackendId→`Backend.alloc` association | Receipt-time identity | **CREATE INSIDE DATAPLANE OWNER** | No current authoritative join exists; address/BackendIndex reread is racy |
| `BackendIndex` / `ResolvedBackend` | `Backend.alloc` identity semantics | **REUSE SEMANTICS ONLY; DO NOT USE FOR RECEIPT RESOLUTION** | Establishes workload SpiffeId meaning but not actual selected-socket identity |
| `IntentStore` typed aggregate/envelope discipline | Public Route Set | **EXTEND** | One new record family; no new store |
| KEK/typed-codec custody pattern + rustls resolver experience | Protected current Certified Key | **EXTEND PATTERN, CREATE NEW AGGREGATE** | Existing private CAs cannot own public Web-PKI material; producer-neutral custody is absent |
| `ca_issuance::issue_and_audit` + issued-certificate row | Gateway issue/audit | **REUSE AS-IS** | Accepts exact NodeId/SpiffeId and returns material only after auditable issuance |
| Workload `SvidLifecycle` | Desired/actual, renewal and retry policy | **REUSE PATTERN; CREATE GATEWAY SIBLING LIFECYCLE** | Allocation-keyed state/actions cannot represent one node slot |
| `IdentityMgr` / allocation `IdentityRead` | In-memory holding/redaction | **REUSE PATTERN; CREATE GATEWAY IDENTITY SLOT/HOLDER** | Existing API requires AllocationId; extending it conflates identities |
| mTLS rustls verifier | Chain + one-SPIFFE-URI verification | **REUSE; EXTEND EXACT EQUALITY** | Selected peer must equal receipt-selected applied identity |
| Transparent `MtlsEnforcement`/kTLS pump | Allocation traffic entry | **DO NOT REUSE AS GATEWAY ENTRY** | Requires intercepted AllocationId leg; gateway-client entry is distinct under same transport owner |
| Gateway-client mTLS entry | Connected fd + gateway material + selected identity | **CREATE UNDER EXISTING mTLS OWNER** | No existing consumer has this combination; no fake Allocation identity |
| Workflow engine/journal/`instant-acme` | Future acquisition | **OUT OF SCOPE / NO CHANGE** | GH #57 owns later producer; first slice creates no workflow/order/challenge model |
| `GatewayApplicationOwner` + frontend demand | Atomic application + live-consumer demand | **CREATE DERIVED CAPABILITY, NO STORE** | Staged-before-swap/current/draining-after-swap prevents torn generation and premature TEACH withdrawal |

| Context | Event Sourcing | CQRS/read-model verdict |
|---|---|---|
| Public Ingress | **No** — singleton Route and current application/demand state need no temporal replay | Existing intent plus derived application/demand/status views; no command/query bus |
| Public Certified-Key Custody | **No** — current protected generation and optimistic replacement suffice | Producer command and custody/runtime views use existing boundaries; no acquisition history |
| Workload Identity | **No** — current gateway slot plus issuance audit and retry inputs suffice | Holder currentness is not audit history; no workflow/event stream |
| Service Lifecycle/Dataplane | **No change** | Existing rows/maps plus transient receipt and process-lifetime identity projection remain autonomous |

## Wave: DESIGN / [REF] Domain alternatives and decision drivers

| Significant choice and drivers | Selected model | Viable alternatives rejected |
|---|---|---|
| One-Route atomic cardinality; public exposure independent from Service lifecycle | Singleton Public Route Set with optional Route entity; one IntentStore aggregate key owns slot conflict | Per-Route aggregates + global claim add a second consistency boundary; embedding PublicExposure in Service couples host/path/key withdrawal to workload/Backend Eligibility lifecycle |
| Web-PKI trust separation; protected rotation; manual now and ACME #57 later share consumption | Distinct Public Certified-Key Custody context/aggregate, reusing protection/resolver patterns | Route-owned key material makes every rotation a Route mutation; generalized allocation `IdentityMgr` mixes ephemeral SPIFFE state with protected hostname-bound Web-PKI |
| Exact non-allocation subject; auditable issue/reissue; one authorized consumer | Dedicated Gateway Identity Slot and sibling lifecycle, reusing `issue_and_audit`, audit/profile and SVID lifecycle patterns | Synthetic AllocationId falsifies lifecycle/audit; broad `IdentityMgr` key union expands every holder/read/action path and mixes node infrastructure with Running allocations |
| Direct currentness; no temporal query; transient demand/receipts die on restart | State-based aggregates + reconcilers + derived views, reusing Intent/Observation/View boundaries | Event streams require secret/receipt replay, retention/upcasting and cross-owner ordering; CQRS adds projection lag and another source without scale/query need |

Detailed decision-specific trade-offs are in Route
[ADR-0105](../../product/architecture/adr-0105-singleton-public-route-set-aggregate.md)
and Domain
[ADR-0112](../../product/architecture/adr-0112-public-certified-key-custody-domain-boundary.md),
[ADR-0113](../../product/architecture/adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md),
[ADR-0114](../../product/architecture/adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md),
[ADR-0115](../../product/architecture/adr-0115-derived-gateway-frontend-demand-lifecycle.md)
and [ADR-0116](../../product/architecture/adr-0116-state-based-public-ingress-domain-ownership.md).
None changes manual certificate first, later GH #57 ACME, BPF-owned selection,
exact-peer mTLS or the active walking skeleton.

## Wave: DESIGN / [REF] Active flow and production acceptance handoff

System ADR-0104/0117–0121 resolve the former `DESIGN-GAP-PIG-1`; it is not a remaining gate or
implementation permission. The complete active domain flow is:

```text
Route
  -> exact Service Frontend
  -> Staged/Current/Draining Gateway Frontend Demand
  -> ServiceMapHydrator demand-gated Path-A TEACH
  -> Gateway Connect Intent (SO_COOKIE, ServiceKey)
  -> ancestor-attached cgroup_connect4_service
  -> SERVICE_MAP Maglev + BACKEND_MAP selection/rewrite
  -> BPF Selection Receipt Selected(BackendId) | NoBackend
  -> Dataplane Applied Backend Identity Association
  -> exact selected Backend.alloc SpiffeId
  -> Gateway SVID + exact-peer gateway-client rustls
  -> workload response on the authenticated socket
```

The cgroup hook, not XDP, selects this host-originated gateway socket. XDP's
existing wire-forward/reverse path stays unchanged. No domain edge permits the
gateway to List/Watch `ServiceBackendRow`, enumerate candidates, keep a cursor,
choose `Backend.alloc` or touch raw maps.

Exact implementation contracts live only in [Application component decomposition](#wave-design--ref-application-component-decomposition),
[driving/driven ports](#wave-design--ref-exact-driving-and-driven-ports),
[custody/persistence](#wave-design--ref-persistence-custody-and-redaction),
[Gateway Application/status/HTTP](#wave-design--ref-gateway-application-status-and-http-contract)
and [gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration).
They must preserve these domain invariants rather than substitute an address
reread, fake Allocation identity, userspace selector or second map.

`AT-PIG-E2E-1` remains the real Tier-3 production walking skeleton, never a
spike. It drives the built default-feature binary with real `serve`, Service and
Route deploy, manual test-CA Certified Key custody, external TLS, registered
cgroup-BPF selection, BackendId receipt/identity resolution, gateway-SVID exact-
peer mTLS, streamed workload response and cleanup. Wrong-valid-peer,
NoBackend→503, missing/mismatched receipt→502, withdrawal/drain, restart and
feature-disabled nonmutation are mandatory complements. Production EDD, not the
hermetic test CA, proves a publicly trusted chain.

Automated ACME is not part of this first-slice acceptance. GH #57 later
owns its workflow-driven producer and must reuse the
`Install/Replace Certified Key` custody command.

No roadmap is produced in DESIGN.

## Wave: DESIGN / [REF] Application decisions

Stage 3 preserves D-SYS-1…11 and D-DOM-1…13. Six independently reversible
Application choices are Proposed in
[ADR-0106](../../product/architecture/adr-0106-public-ingress-route-and-gateway-application-owner.md),
[ADR-0107](../../product/architecture/adr-0107-public-certified-key-custody-and-preserve-old-replacement.md),
[ADR-0108](../../product/architecture/adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md),
[ADR-0109](../../product/architecture/adr-0109-dedicated-gateway-svid-lifecycle-and-exact-peer-mtls.md),
[ADR-0110](../../product/architecture/adr-0110-bounded-public-tls13-http11-runtime.md),
and [ADR-0111](../../product/architecture/adr-0111-redacted-operator-gateway-status.md).
This feature delta—not those decision records—is the implementation-contract
SSOT for every exact Rust signature, variant, persistence key, lifecycle
ordering, error and verification handoff. System ADR-0104/0117–0121 resolve the
former system gap; Application pins active production contracts without a spike or
placeholder seam.

| ID | Application verdict |
|---|---|
| **D-APP-1** | [ADR-0106] creates one non-deployable `overdrive-gateway` adapter-host library; `overdrive-control-plane` remains sole `serve` composition/ServerHandle owner and activates the real GatewayApplicationOwner/listener. |
| **D-APP-2** | Add parser-side `DeploySpecInput = Workload(WorkloadSpecInput) | Route(PublicRouteSpecInput)`, distinct HTTP `PublicRouteInput`, and persisted `Route`; never add Route to workload `SubmitSpecInput`, `WorkloadIntent`, `WorkloadKind`, `/v1/workloads`, or workload streaming. |
| **D-APP-3** | Persist the singleton `PublicRouteSetEnvelope::V1` at exact key `public-ingress/route-set`; persist even the empty Route set. A bounded `PublicRouteSetOwner` is the sole serialized writer. |
| **D-APP-4** | A Route carries exact validated `RouteId`, deterministic `RouteGeneration`, `PublicHostname`, `PathMatch`, `ServiceListenerReference(WorkloadId, NonZeroU16, Proto::Tcp)` and `PublicCertifiedKeyId`. |
| **D-APP-5** | A new read-only `ServiceFrontendResolve` port reads current Service intent plus the already-probed `ServiceVipView` and returns the exact existing `ServiceFrontend`; control-plane wraps its existing mutex-held allocator in a private read adapter. It never reads backend/health/BPF state. |
| **D-APP-6** | `ServerConfig.gateway: Option<GatewayConfig>` is the only enablement gate. CLI `--gateway-address`, `--gateway-certified-key-id`, `--gateway-certificate-chain` and `--gateway-private-key` construct IPv4 TCP/443 only as an all-present set; all absent means disabled. Public bind occurs only after connect/mTLS/identity/demand/application gates. |
| **D-APP-7** | Manual certificate/key paths are host-local `serve` configuration, like VM kernel/rootfs references only at the artifact boundary. The source validates then installs through producer-neutral custody; paths/raw PEM never enter Route, status or request state. |
| **D-APP-8** | [ADR-0107] makes Public Certified-Key Custody the one configured-ID owner, with validate/build/seal/persist/publish/status ordering that never replaces the last usable generation with an unusable candidate. Its ID-free `install` accepts `Manual | Workflow { correlation }` provenance and is #57's future producer boundary. |
| **D-APP-9** | One active `GatewayApplicationOwner` atomically owns staged/current/draining demand, one ArcSwap Current generation, dynamic TCP/443 bind/unbind, RAII connection/request leases, the join set and application status. Shutdown force-joins leases before demand/SVID release. |
| **D-APP-10** | `IntentStore::watch` changes in one cut to `Changed|Lagged`; owner subscribes-before-list and clears admission before gap/closure recovery. |
| **D-APP-11** | Add dedicated non-allocation Gateway Identity Slot + `GatewaySvidLifecycle`, Issue/Drop actions and exact-current control. The slot atomically owns a checked desired epoch so a stale Issue cannot re-hold after disable; gateway-client mTLS alone constructs the private access token and receives mandatory Clock injection. No fake AllocationId. |
| **D-APP-12** | Extend ServiceMapHydrator with exact frontend demand read/ack and the existing `ServiceId::derive(...,"service-map")`; a pre-effect exact-revision guard prevents stale Path-A TEACH while BPF Hydrated remains independent. |
| **D-APP-13** | [ADR-0108] adds registered `(SocketCookie, ServiceKey)` Dataplane intent, BPF `Selected(BackendId)|NoBackend` receipt, and commit-guarded immutable BackendId→applied `Backend.alloc` publication with cleanup on every return. |
| **D-APP-14** | [ADR-0109/0110] activates dedicated gateway-SVID exact-peer mTLS and strict rustls TLS1.3 + hyper HTTP/1.1 with finite bounded streaming/backpressure, 404 Route miss, 503 NoBackend, 502 internal/upstream failure and no retry/replay/alternate backend. |
| **D-APP-15** | Full composition is `AT-PIG-E2E-1`, a recurring built-binary Tier-3 walking skeleton, not an expectation or spike; it hand-installs no map, rule, identity, snapshot or credential outside production inputs. |
| **D-APP-16** | [ADR-0111] keeps operator-mTLS `GET /v1/gateway/status` as a separate redacted projection of application/custody and relevant independent hydration rows; public request status, receipts, secrets and a unified generation never enter it. |

Formal TLA+ is not introduced: this slice adds no new consensus,
cross-node transaction or replication protocol. Atomic snapshot/watch and
certificate-validity ordering are more directly verified through pure
properties and seeded DST.

[ADR-0106]: ../../product/architecture/adr-0106-public-ingress-route-and-gateway-application-owner.md
[ADR-0107]: ../../product/architecture/adr-0107-public-certified-key-custody-and-preserve-old-replacement.md
[ADR-0108]: ../../product/architecture/adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md
[ADR-0109]: ../../product/architecture/adr-0109-dedicated-gateway-svid-lifecycle-and-exact-peer-mtls.md
[ADR-0110]: ../../product/architecture/adr-0110-bounded-public-tls13-http11-runtime.md
[ADR-0111]: ../../product/architecture/adr-0111-redacted-operator-gateway-status.md

## Wave: DESIGN / [REF] Application component decomposition

| Component | Exact home / class | Change | Contract shape and declared universe | Assertion mechanism |
|---|---|---|---|---|
| Route/parser/identifier/envelope types | `overdrive-core::public_ingress`, `api::route` / `core` | **EXTEND** | **pure-function**: input -> typed value/error only; codec universe is one Route-set value | per-value PBT, canonical-byte golden and state-delta complement |
| Public-certified-key record + AEAD codec | core record in `overdrive-core`; crypto adapter in `overdrive-host::ca` | **EXTEND** | **bounded-change**: one public-key ciphertext/record domain; Root/intermediate CA records are complement | codec tamper/CSPRNG fault integration + secret-sink trybuild |
| Lag-aware intent subscription | `overdrive-core::traits::IntentStore`, Local/Sim adapters | **EXTEND IN ONE CUT** | **bounded-change**: accepted events or one `Lagged` fact; store contents are read-only | production/Sim adapter-equivalence and seeded loss/relist DST |
| Shared gateway values/read ports | `overdrive-core::{public_ingress,gateway_identity}` / `core` | **EXTEND** | **pure-function/unbounded-preservation**: ServiceKey, SocketCookie, demand revision/read/ack/wake and non-secret gateway-identity desired/current facts; no host I/O | value-law PBT + existing-core dependency lint |
| Gateway-owned driven ports | `overdrive-gateway::ports` / `adapter-host` | **CREATE NARROW PORTS** | **bounded-change**: `ServiceFrontendResolve`, `GatewayConnectDataplane`, `GatewayClientMtls` and `GatewayIdentityLifecycleControl` expose only application-required capabilities | Host/Sim equivalence + caller/API-surface scan |
| `PublicRouteSetOwner` | `overdrive-gateway::route_set` / `adapter-host` | **CREATE NEW** | **bounded-change**: fixed Route key, owner snapshot and one reply/event | command state-delta PBT + real-store ordering integration |
| `IntentServiceFrontendResolver` | `overdrive-gateway::frontend` / `adapter-host` | **CREATE NEW ADAPTER** | **unbounded-preservation**: reads WorkloadIntent/VIP; all store/allocator state preserved | before/after adapter-equivalence plus real malformed-prefix probe |
| `SocketCookieReader` | `overdrive-host::socket_cookie` / `adapter-host` | **CREATE NEW ADAPTER** | **unbounded-preservation**: reads one borrowed fd; socket and all kernel tables preserved | same-method nonzero/stability probe + real-connector cookie equality |
| `ManualCertifiedKeySource` | `overdrive-gateway::manual_certified_key` / `adapter-host` | **CREATE NEW** | **unbounded-preservation**: reads only two configured files; whole artifact tree preserved | filesystem snapshot complement + permission/symlink/torn-read faults |
| `PublicCertifiedKeyCustody` | `overdrive-gateway::certified_key` / `adapter-host` | **CREATE NEW** | **bounded-change**: one protected intent record, one custody status row and one latest availability | store state-delta, generation-race PBT and expiry/clock integration |
| `GatewayApplicationOwner` + demand | `overdrive-gateway::{application,demand}` / `adapter-host` | **CREATE ACTIVE OWNER** | **bounded-change**: staged/current/draining set, one ArcSwap generation with connection/request lease counters, listener state and one status row | seeded lifecycle/DST + retained-generation/cancellation tests |
| Public Runtime + connector | `overdrive-gateway::runtime`, `connector` / `adapter-host` | **CREATE ACTIVE OWNER** | **bounded-change** per accepted connection: its socket, headers/body budget, join-set task and one cookie intent/receipt; fixed cleanup-failure ledger | protocol matrix/PBT + connector/shutdown fault matrix + Tier-3 wire capture |
| Demand read/ack + Hydrator TEACH | core ports; `ServiceMapHydrator`; action shim | **EXTEND** | **bounded-change**: exact demanded ServiceKey slots and ack revision; undemanded/LOCAL/XDP universe preserved | Hydrator state-delta DST + byte-equal legacy classification tests |
| `AppState::gateway` composition | control-plane / `adapter-host` | **EXTEND** | **bounded-change**: one Clone-safe composition owns Enabled(read+ack+wake+identity+control) or canonical Disabled across every hydrate/action/workflow/handler path | real `run_convergence_tick` stale guard + disabled legacy-equivalence |
| Connect registry + identity association | `overdrive-dataplane::gateway_connect`; BPF hook/maps | **EXTEND EXISTING DATAPLANE OWNER** | **bounded-change**: registered cookie entries plus immutable BackendId association; unregistered sockets/maps preserved | POD/gold layout, Sim equivalence and real-kernel complement-restored probe |
| Gateway Identity Slot/lifecycle/client mTLS | dataplane mTLS + reconciler/control-plane executor | **CREATE SIBLING/DEDICATED SLOT** | **bounded-change**: one non-allocation slot, desired/retry state and one authenticated fd; allocation holder is complement | pure lifecycle PBT, audit/slot integration, wrong-peer TLS matrix |
| Gateway status persistence | core row/envelopes + store/sim adapters | **EXTEND** | **bounded-change**: exactly two new LWW row families/keys; existing tables preserved | golden codecs + LWW/timestamp state-delta PBT + redaction API tests |
| `serve`/operator composition | control-plane + CLI | **EXTEND** | **bounded-change**: mandatory Enabled/Disabled demand composition plus optional GatewayControl and complete GatewayHandle task/socket tree; existing operator/workload owners preserved | all-wrapper stale guard + every-owner shutdown integration + disabled complement + built-binary AT |

### Exact `GatewaySvidLifecycle` closed-dispatch contract

`GatewaySvidLifecycle` lives in
`overdrive-reconcilers::gateway_svid_lifecycle` and implements the existing
`Reconciler` trait without a parallel runtime:

```rust
pub struct GatewaySvidLifecycle { name: ReconcilerName }

impl GatewaySvidLifecycle {
    pub fn canonical() -> Self;
}

#[async_trait::async_trait]
impl Reconciler for GatewaySvidLifecycle {
    const NAME: &'static str = "gateway-svid-lifecycle";
    type State = GatewaySvidLifecycleState;
    type View = GatewaySvidLifecycleView;

    fn name(&self) -> &ReconcilerName;
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View);
    fn next_evaluation_at(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        next_view: &Self::View,
        tick: &TickContext,
    ) -> Option<UnixInstant>;
    async fn hydrate_desired(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError>;
    async fn hydrate_actual(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError>;
    fn resync_schedule(&self) -> Option<ResyncSchedule> { None }
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[ObservationRowKind::IssuedCertificate]
    }
}

// overdrive-control-plane::gateway_identity_lifecycle
pub(crate) fn gateway_svid_lifecycle_registration(
    node_id: &NodeId,
) -> (AnyReconciler, Evaluation);
```

`canonical()` constructs `ReconcilerName::new(Self::NAME)` and cannot accept a
caller-supplied name. The factory implementation is exactly:

```rust
let reconciler = GatewaySvidLifecycle::canonical();
let target = TargetResource::new(&format!("node/{node_id}"))
    .expect("NodeId is non-empty and node/ is a canonical target prefix");
let evaluation = Evaluation {
    reconciler: reconciler.name().clone(),
    target,
};
(AnyReconciler::GatewaySvidLifecycle(reconciler), evaluation)
```

The only production caller is
`run_server_with_obs_and_drivers`: it calls `runtime.register(reconciler).await`
once, then calls
`runtime.broker().submit(evaluation, clock.now(),
EvaluationEligibility::Immediate)` through the existing broker.
`ControlPlaneGatewayIdentityLifecycle` reuses the
same factory-derived name/target for ensure, reissue and disable wakes.

The existing closed dispatches gain exactly these variants:

```rust
AnyReconciler::GatewaySvidLifecycle(GatewaySvidLifecycle)
AnyState::GatewaySvidLifecycle(GatewaySvidLifecycleState)
AnyReconcilerView::GatewaySvidLifecycle(GatewaySvidLifecycleView)
AnyViewMap::GatewaySvidLifecycle(
    BTreeMap<TargetResource, GatewaySvidLifecycleView>,
)
```

Every exhaustive `AnyReconciler` forwarder gains the matching arm: `name`,
`static_name`, `resync_schedule`, `interests`, `reconcile`,
`next_evaluation_at`, `hydrate_desired`, and `hydrate_actual`. Runtime
`register`, view load/read/write/persist and test accessors gain the matching
`AnyViewMap`/`AnyReconcilerView` arm. No wildcard arm or second registry is
permitted. The concrete projection and persisted retry-memory types are:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySvidLifecycleState {
    pub desired: GatewayIdentityDesired,
    pub actual: Option<GatewayIdentityFacts>,
    pub ever_issued: bool,
}
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct GatewaySvidLifecycleView {
    pub retry: Option<crate::svid_lifecycle::IssueRetry>,
}
```

Hydration requires the target equal the configured `node/<node-id>`, reads
desired/current through the two gateway identity read ports, and derives
`ever_issued` only from the existing issued-certificate audit rows for the
gateway SPIFFE subject.

The existing public `HydrationContext<'a>` gains exactly three mandatory fields:

```rust
pub gateway_frontend_demand: &'a dyn GatewayFrontendDemandRead,
pub gateway_identity_desired: &'a dyn GatewayIdentityDesiredRead,
pub gateway_identity_current: &'a dyn GatewayIdentityCurrentRead,
```

The sole `build_hydration_context` supplies them from `AppState` at every
existing construction site. Enabled `serve` supplies the one
`GatewayIdentitySlot`; disabled `serve` supplies a control-plane-private
canonical empty adapter whose desired value is epoch one with `spiffe_id=None`
and whose current value is `None`. `gateway_frontend_demand` always comes from
`GatewayDemandComposition::read`; disabled composition supplies its canonical
empty snapshot. Only ServiceMapHydrator reads the demand field, and only the
gateway lifecycle reads the identity fields. This preserves a total
HydrationContext without optional per-call parameters or a second gateway-only
runtime.
`GatewayDemandComposition::read()` borrows the Arc-held reader from the
AppState-owned composition; `build_hydration_context` stores that borrow, never
a reference into a temporary cloned Arc. The returned HydrationContext cannot
outlive its `&AppState` input.

Dependency direction is `control-plane -> {gateway,dataplane,reconcilers} ->
core`; dataplane depends on gateway only for `GatewayConnectDataplane` and
`GatewayClientMtls` ports and on host only for the shared probed
`SocketCookieReader`; gateway depends on host/core and never dataplane,
control-plane, reconcilers, worker, store-local or aya.

## Wave: DESIGN / [REF] Exact driving and driven ports

### Driving surfaces

| Surface | Exact first-slice contract |
|---|---|
| `overdrive serve` | Scalar `--gateway-address`, `--gateway-certified-key-id`, `--gateway-certificate-chain` and `--gateway-private-key` arguments are all absent or all present. The complete set activates identity, connect/demand probes, `GatewayApplicationOwner` and dynamic fixed TCP/443 after gates. |
| `overdrive deploy <SERVICE_SPEC>` | Existing workload path byte/behavior unchanged. |
| `overdrive deploy <ROUTE_SPEC>` | `deploy_resource(args, stream_workloads)` parses once; `[route]` + `[route.target]` selects one-shot `POST /v1/routes` and `RouteDeployOutput`; `--detach` has no additional effect and no NDJSON opens. Existing workload output functions/types remain unchanged. |
| `DELETE /v1/routes/{id}` | Withdraws the same-ID singleton Route or returns cause-distinct conflict; persists empty Route Set rather than deleting its key; both `Withdrawn` and already-empty `Unchanged` return 200. |
| Public TCP/443 | Each accept snapshots one Current Gateway Application before TLS and retains it through bounded keep-alive; no Current means socket unbound/no accept. |
| Manual file replacement | Atomic replacement of configured files is discovered on the source owner's private finite maintenance cadence and invokes the same custody install command future ACME uses; cadence is not a knob/KPI. |

The clap surface is an exact additive delta to `overdrive-cli::cli::Command::Serve`:

```rust
#[arg(long = "gateway-address", value_name = "IPV4")]
gateway_address: Option<Ipv4Addr>,
#[arg(long = "gateway-certified-key-id", value_name = "ID")]
gateway_certified_key_id: Option<PublicCertifiedKeyId>,
#[arg(long = "gateway-certificate-chain", value_name = "PATH")]
gateway_certificate_chain: Option<PathBuf>,
#[arg(long = "gateway-private-key", value_name = "PATH")]
gateway_private_key: Option<PathBuf>,
```

Each `Option<T>` is clap's scalar `ArgAction::Set`: the flag occurs zero or one
time, and a repeated occurrence is an argument conflict. `Ipv4Addr` and
`PublicCertifiedKeyId` use their typed `FromStr` parsers at the argv boundary;
there is no address string or unvalidated ID in `main`. The chain argument is
one host-local file containing the complete leaf-first PEM certificate chain;
the key argument is one host-local file containing exactly one PKCS#8 private
key. They are paths, not inline PEM, and project unchanged into the manual
source boundary specified below. No ACME/account/order argument or environment
fallback exists.

Exact Route TOML:

```toml
[route]
id = "public-api"
hostname = "api.example.com"
path = "/"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
workload = "api"
port = 8080
protocol = "tcp"
```

Operator HTTP additions are `POST /v1/routes`,
`DELETE /v1/routes/{id}` and `GET /v1/gateway/status`; all remain on the
existing operator mTLS listener. `SubmitRouteResponse` reports
`Declared|Replaced|Unchanged` with Route ID/generation. Route conflict is 409;
validation is 400; store faults are 500. When gateway config is absent, POST
and DELETE return typed `ControlPlaneError::GatewayDisabled` as 409
`ErrorBody { error: "gateway_disabled", message: "public ingress gateway is
disabled", field: null }` before parse/owner/store, with an empty IntentStore
delta; GET
returns `enabled=false`, `listener_bound=false` and empty optional projections.
`ApiClient` adds only `submit_route`, the production caller used by deploy.
DELETE and gateway status are operator-mTLS HTTP/OpenAPI surfaces in this
slice; no dormant CLI methods/commands are introduced. Live OpenAPI registers
all three server paths/schemas.

The closed control-plane declaration and mapping are exact:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ControlPlaneError {
    // existing variants unchanged
    #[error("public ingress gateway is disabled")]
    GatewayDisabled,
}

// Exact new exhaustive arm in error::to_response.
ControlPlaneError::GatewayDisabled => (
    StatusCode::CONFLICT,
    ErrorBody {
        error: "gateway_disabled".into(),
        message: "public ingress gateway is disabled".into(),
        field: None,
    },
)
```

Route POST accepts `State<AppState>` plus raw `Bytes`, checks
`state.gateway.control()` first, and only then deserializes
`PublicRouteInput`; this is how disabled returns the fixed 409 even for invalid
JSON with no owner/store call. DELETE accepts the raw path segment, performs the
same control check, then parses RouteId. The live `utoipa::path` declarations
name `PublicRouteInput` as POST request body and include exact responses: 201/
200 `SubmitRouteResponse`, 400 `ErrorBody`, 409 `ErrorBody`, 500 `ErrorBody`;
DELETE includes 200 `WithdrawRouteResponse`, 400/409/500 `ErrorBody`; Gateway
GET includes 200 `GatewayStatusResponse` and 500 `ErrorBody`. All schemas and
paths are added to the production `ApiDoc`; none is test-only.

### Exact Route, deploy, configuration and owner surfaces

```rust
pub enum DeploySpecInput {
    Workload(WorkloadSpecInput),
    Route(PublicRouteSpecInput),
}
pub struct PublicRouteSpecInput {
    pub id: String,
    pub hostname: String,
    pub path: String,
    pub path_match: String,
    pub certified_key: String,
    pub target: ServiceListenerReferenceInput,
}
pub struct PublicRouteInput {
    pub id: String,
    pub hostname: String,
    pub path: String,
    pub path_match: String,
    pub certified_key: String,
    pub target: ServiceListenerReferenceInput,
}
pub struct ServiceListenerReferenceInput {
    pub workload: String,
    pub port: u16,
    pub protocol: String,
}

pub struct Route {
    route_id: RouteId,
    route_generation: RouteGeneration,
    public_hostname: PublicHostname,
    path_match: PathMatch,
    service_listener: ServiceListenerReference,
    public_certified_key_id: PublicCertifiedKeyId,
}
impl Route {
    pub fn from_submit(input: PublicRouteInput) -> Result<Self, RouteValidationError>;
    pub fn route_id(&self) -> &RouteId;
    pub const fn route_generation(&self) -> RouteGeneration;
    pub fn public_hostname(&self) -> &PublicHostname;
    pub fn path_match(&self) -> &PathMatch;
    pub fn service_listener(&self) -> &ServiceListenerReference;
    pub fn public_certified_key_id(&self) -> &PublicCertifiedKeyId;
}

pub struct RouteGeneration([u8; 32]);
impl RouteGeneration {
    pub fn new(raw: &str) -> Result<Self, DigestHexParseError>;
    pub fn from_route_fields(
        route_id: &RouteId,
        hostname: &PublicHostname,
        path_match: &PathMatch,
        listener: &ServiceListenerReference,
        certified_key_id: &PublicCertifiedKeyId,
    ) -> Self;
    pub const fn as_bytes(&self) -> &[u8; 32];
}
// RouteId/PublicCertifiedKeyId use the existing label-newtype new/as_str/
// Display/FromStr/TryFrom surface because they are operator inputs.
// RouteGeneration implements case-sensitive FromStr via new, lowercase-hex
// Display, matching string Serialize/Deserialize, and rkyv archive derives.

pub struct PublicHostname(String);
impl PublicHostname {
    pub fn new(raw: &str) -> Result<Self, PublicHostnameError>;
    pub fn as_str(&self) -> &str;
}
pub enum PublicHostnameError {
    Empty, TooLong, NonAscii, Wildcard, IpLiteral, TerminalDot,
    EmptyLabel, LabelTooLong, InvalidLabel,
}
pub struct PublicPath(String);
impl PublicPath {
    pub fn new(raw: &str) -> Result<Self, PublicPathError>;
    pub fn as_str(&self) -> &str;
}
pub enum PublicPathError {
    Empty, NotAbsolute, ContainsControl, ContainsSpace,
    ContainsQuery, ContainsFragment, MalformedPercentEncoding,
}
// PublicHostname/PublicPath implement canonical Display/FromStr/TryFrom and
// string serde/rkyv; FromStr returns the corresponding closed error.

pub enum PathMatch { Exact(PublicPath), SegmentPrefix(PublicPath) }
pub struct ServiceListenerReference {
    workload_id: WorkloadId,
    port: NonZeroU16,
    protocol: Proto,
}
impl ServiceListenerReference {
    pub fn new(
        workload_id: WorkloadId,
        port: NonZeroU16,
        protocol: Proto,
    ) -> Result<Self, RouteValidationError>; // accepts only Proto::Tcp
    pub fn workload_id(&self) -> &WorkloadId;
    pub const fn port(&self) -> NonZeroU16;
    pub const fn protocol(&self) -> Proto;
}
pub enum RouteValidationError {
    RouteId(IdParseError), CertifiedKeyId(IdParseError),
    WorkloadId(IdParseError), Hostname(PublicHostnameError),
    Path(PublicPathError), UnknownPathMatch, PortZero,
    UnknownProtocol, ProtocolNotTcp,
}

pub struct PublicRouteSetV1 { pub route: Option<Route> }
pub enum PublicRouteSetEnvelope { V1(PublicRouteSetV1) }
pub enum RouteApplyOutcome { Declared, Replaced, Unchanged }
pub enum RouteWithdrawOutcome { Withdrawn, Unchanged }
pub enum PublicRouteSetError {
    Intent(IntentStoreError), Decode(RouteCodecError),
    OccupiedByDifferentRoute { current: RouteId, attempted: RouteId },
    WithdrawIdMismatch { current: RouteId, attempted: RouteId },
    OwnerUnavailable,
}
pub enum RouteCodecError {
    Envelope(EnvelopeError),
    InvalidRoute(RouteValidationError),
}
#[derive(Clone)]
pub struct PublicRouteSetHandle { /* private command sender */ }
impl PublicRouteSetHandle {
    pub async fn apply(&self, route: Route)
        -> Result<RouteApplyOutcome, PublicRouteSetError>;
    pub async fn withdraw(&self, route_id: RouteId)
        -> Result<RouteWithdrawOutcome, PublicRouteSetError>;
}

pub struct SubmitRouteResponse {
    pub route_id: String,
    pub route_generation: String,
    pub outcome: RouteApplyOutcome,
}
pub struct WithdrawRouteResponse {
    pub route_id: String,
    pub outcome: RouteWithdrawOutcome,
}
impl ApiClient {
    pub async fn submit_route(&self, input: PublicRouteInput)
        -> Result<SubmitRouteResponse, CliError>;
}
pub async fn deploy_resource(
    args: DeployArgs,
    stream_workloads: bool,
) -> Result<DeployResourceOutput, CliError>;
pub enum DeployResourceOutput {
    WorkloadDetached(DeployOutput),
    WorkloadStreaming(DeployStreamingOutput),
    Route(RouteDeployOutput),
}
pub struct RouteDeployOutput {
    pub route_id: String,
    pub route_generation: String,
    pub outcome: RouteApplyOutcome,
    pub endpoint: Url,
}

// overdrive-cli::commands::serve; existing fields retain their exact types.
pub struct ServeArgs {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub gateway_address: Option<Ipv4Addr>,
    pub gateway_certified_key_id: Option<PublicCertifiedKeyId>,
    pub gateway_certificate_chain: Option<PathBuf>,
    pub gateway_private_key: Option<PathBuf>,
}

pub struct ManualCertifiedKeyConfig {
    id: PublicCertifiedKeyId,
    certificate_chain_path: PathBuf,
    private_key_path: PathBuf,
}
impl ManualCertifiedKeyConfig {
    pub fn new(id: PublicCertifiedKeyId, chain: PathBuf, key: PathBuf) -> Self;
    pub fn id(&self) -> &PublicCertifiedKeyId;
    pub fn certificate_chain_path(&self) -> &Path;
    pub fn private_key_path(&self) -> &Path;
}
pub struct GatewayConfig {
    bind: SocketAddrV4,
    manual_certified_key: ManualCertifiedKeyConfig,
}
impl GatewayConfig {
    pub fn new(bind_address: Ipv4Addr, key: ManualCertifiedKeyConfig) -> Self;
    pub const fn bind(&self) -> SocketAddrV4; // always port 443
    pub fn manual_certified_key(&self) -> &ManualCertifiedKeyConfig;
}
// Additive overdrive-control-plane::ServerConfig field; all existing fields
// and ServerConfig::new semantics remain unchanged.
pub struct ServerConfig {
    // existing fields unchanged
    pub gateway: Option<GatewayConfig>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayConfigField {
    Address, CertifiedKeyId, CertificateChainPath, PrivateKeyPath,
}
#[derive(Debug, thiserror::Error)]
pub enum GatewayConfigError {
    #[error("partial public ingress gateway enablement; missing {missing:?}")]
    PartialEnablement { missing: Vec<GatewayConfigField> },
}
impl ServeArgs {
    pub fn gateway_config(&self) -> Result<Option<GatewayConfig>, GatewayConfigError>;
}

// Exact additive overdrive-cli::http_client::CliError variant.
#[error("invalid public ingress gateway configuration: {source}")]
GatewayConfiguration {
    #[source]
    source: GatewayConfigError,
},
```

`ServeArgs::gateway_config` is called as the first operation in
`commands::serve::run_inner`, before the cgroup probe, any file read, store
open or listener bind. Its closed truth table is:

| Supplied fields | Result |
|---|---|
| all four `None` | `Ok(None)`; `ServerConfig.gateway = None` and the gateway remains disabled |
| all four `Some` | `Ok(Some(GatewayConfig::new(address, ManualCertifiedKeyConfig::new(id, chain, key))))`; `GatewayConfig::new` fixes the public bind port to 443 |
| every other combination | `Err(GatewayConfigError::PartialEnablement { missing })`; `missing` is emitted in `Address, CertifiedKeyId, CertificateChainPath, PrivateKeyPath` order and no startup effect occurs |

`run_inner` maps only that error to
`CliError::GatewayConfiguration { source }`, then moves the successful value
unchanged into the additive `ServerConfig.gateway: Option<GatewayConfig>`
field. `ServerConfig::new(kek)` initializes that field to `None`; the
production `run_inner` struct update is the only enabling override. `main`
destructures the four typed clap fields and moves them into the
same-named `ServeArgs` fields; it performs no second parse or file read. All
existing direct `ServeArgs` construction sites add the four fields as `None`
unless that fixture intentionally enables public ingress. There is no partial
default, implicit certificate location or fallback producer.

`PublicRouteSpecInput` is TOML-parser-only; `PublicRouteInput` is serde/OpenAPI
wire-only; `Route`/envelopes are rkyv-only. The presence walk rejects a Route
combined with any workload table before decoding. `ControlPlaneError::
GatewayDisabled` is the exact pre-parse POST/DELETE branch and maps to 409
`ErrorBody { error: "gateway_disabled", message: "public ingress gateway is
disabled", field: None }`. Apply maps Declared to 201 and Replaced/Unchanged to
200; DELETE maps Withdrawn/Unchanged to 200, invalid ID to 400, different live
ID to 409, and store error to 500.

Every field on `Route`, `GatewayConfig` and `ManualCertifiedKeyConfig` has only
the accessor shown above; their `Debug` implementations redact manual paths.
`RouteGeneration::from_route_fields` SHA-256 hashes
`b"overdrive/gateway/route-generation/v1\0"`, then each string-like field as
`u32::to_be_bytes(len) || bytes` in this order: Route ID, hostname, path-match
tag (`0` exact / `1` segment-prefix), raw path, workload ID, then the two-byte
big-endian port, one-byte `Proto::as_u8`, and length-framed certified-key ID.
No archive layout, map iteration or platform endianness enters the digest.
`PublicHostname::new` and
`PublicPath::new` plus their complete errors/accessors follow the validation
rules in D-APP-4; neither offers an unchecked public constructor.

### Driven ports/adapters

| Port/effect | Exact contract/adapters |
|---|---|
| `IntentStore` | Public Route Set and protected key envelopes; lag-aware empty-prefix subscription. LocalStore production, existing LocalStore/Sim lanes. |
| `ServiceFrontendResolve` | `probe` + `resolve(&ServiceListenerReference) -> ServiceFrontend`; gateway adapter reads WorkloadIntent + `ServiceVipView`, and control-plane privately adapts the existing `Arc<Mutex<PersistentServiceVipAllocator>>`; Sim adapter returns scripted typed outcomes. |
| Gateway Frontend Demand | `GatewayFrontendDemandRead` + sync exact-revision acknowledgment; Hydrator TEACH and action-shim ack; owner stage/promote/drain/retire. |
| Gateway Connect Dataplane | Register/take/cleanup `(SocketCookie,ServiceKey)`, lifecycle-only `cleanup_all_gateway_intents` + live count, Selected/NoBackend receipt and immutable BackendId identity lookup; EbpfDataplane/Sim. Drop failures enter the runtime cleanup ledger. |
| Gateway identity | Dedicated slot facts + desired/current hydration, `GatewaySvidLifecycle`, Issue/Drop actions and lifecycle control; no allocation holder/read. |
| Gateway-client mTLS | Connected fd + exact receipt-selected SpiffeId -> authenticated `GatewayUpstream`; Host/Sim implementations; no InterceptedConnection/kTLS pump. |
| `Kek` | Existing mandatory provider; `overdrive-host::ca::PublicCertifiedKeyAeadCodec` uses fixed `overdrive-public-certified-key` KEK and a distinct gateway HKDF/AAD domain over the existing private AEAD engine. |
| `ObservationStore` | Exact `public_certified_key_status_row(id)` and `gateway_application_status_row(node)` point reads plus two typed LWW writes; tables/keys/codecs are pinned in the Application persistence/status `[REF]` sections below. Issuance audit and Service hydration rows remain unchanged. |
| Public TLS/HTTP | Active `tokio-rustls` 0.26.4 + rustls 0.23.39 + hyper 1.9.0 + hyper-util 0.1.20; fixed finite GatewayLimits policy with no operator input. |

### Exact frontend-demand and Dataplane ports

```rust
// overdrive-gateway::ports
#[async_trait::async_trait]
pub trait ServiceFrontendResolve: Send + Sync {
    async fn probe(&self) -> Result<(), ServiceFrontendResolveError>;
    async fn resolve(
        &self,
        reference: &ServiceListenerReference,
    ) -> Result<ServiceFrontend, ServiceFrontendResolveError>;
}
pub enum ServiceFrontendResolveError {
    WorkloadAbsent { workload_id: WorkloadId },
    WorkloadIntent(IntentStoreError),
    WorkloadIntentDecode,
    WorkloadNotService { workload_id: WorkloadId },
    ListenerAbsent { port: NonZeroU16, protocol: Proto },
    ServiceVipUnavailable,
    InvalidServiceFrontend(IdParseError),
}

// overdrive-core::public_ingress
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct ServiceKey {
    vip: ServiceVip,
    port: NonZeroU16,
    protocol: Proto,
}
impl ServiceKey {
    pub fn from_frontend(frontend: ServiceFrontend) -> Self;
    pub const fn vip(&self) -> ServiceVip;
    pub const fn port(&self) -> NonZeroU16;
    pub const fn protocol(&self) -> Proto;
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct GatewayFrontendDemandRevision(NonZeroU64);
impl GatewayFrontendDemandRevision {
    pub fn new(raw: u64) -> Result<Self, GatewayFrontendDemandRevisionParseError>;
    pub const fn first() -> Self;
    pub fn checked_next(self)
        -> Result<Self, GatewayFrontendDemandError>;
    pub const fn get(self) -> u64;
}
pub enum GatewayFrontendDemandRevisionParseError { Empty, InvalidDecimal, Zero }
// FromStr delegates to decimal new; Display is canonical decimal; matching
// Serialize/Deserialize is string-shaped and rejects noncanonical/zero input.
pub enum GatewayDemandPhase { Staged, Current, Draining }
pub struct GatewayFrontendDemandEntry {
    pub application_generation: GatewayApplicationGenerationId,
    pub service_key: ServiceKey,
    pub frontend: ServiceFrontend,
    pub phase: GatewayDemandPhase,
}
pub struct GatewayFrontendDemandSnapshot {
    pub revision: GatewayFrontendDemandRevision,
    pub entries: Vec<GatewayFrontendDemandEntry>,
}
pub struct GatewayFrontendDemandApply {
    pub revision: GatewayFrontendDemandRevision,
    pub service_key: ServiceKey,
}
pub trait GatewayFrontendDemandRead: Send + Sync {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot>;
}
pub trait GatewayFrontendDemandAcknowledge: Send + Sync {
    fn applied(&self, apply: GatewayFrontendDemandApply)
        -> GatewayDemandAckOutcome;
}
pub enum GatewayDemandAckOutcome { Applied, Stale }
pub enum GatewayFrontendDemandError {
    RevisionExhausted,
    AlreadyStaged,
    UnknownGeneration,
    NotApplied,
    Superseded,
    ApplyTimeout,
    OwnerUnavailable,
}
pub(crate) struct GatewayFrontendDemandTicket {
    generation: GatewayApplicationGenerationId,
    revision: GatewayFrontendDemandRevision,
    service_key: ServiceKey,
}
pub(crate) struct GatewayFrontendDemandHandle { /* private command sender */ }
impl GatewayFrontendDemandHandle {
    pub(crate) async fn stage(
        &self,
        generation: GatewayApplicationGenerationId,
        frontend: ServiceFrontend,
    ) -> Result<GatewayFrontendDemandTicket, GatewayFrontendDemandError>;
    pub(crate) async fn wait_applied(
        &self,
        ticket: &GatewayFrontendDemandTicket,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError>;
    pub(crate) async fn promote(
        &self,
        ticket: GatewayFrontendDemandTicket,
    ) -> Result<(), GatewayFrontendDemandError>;
    pub(crate) async fn remove_staged(
        &self,
        generation: GatewayApplicationGenerationId,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError>;
    pub(crate) async fn begin_draining(
        &self,
        generation: GatewayApplicationGenerationId,
    ) -> Result<(), GatewayFrontendDemandError>;
    pub(crate) async fn retire(
        &self,
        generation: GatewayApplicationGenerationId,
    ) -> Result<(), GatewayFrontendDemandError>;
    pub(crate) async fn clear_all(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayFrontendDemandError>;
}
pub trait GatewayFrontendDemandWake: Send + Sync {
    fn wake(&self, service_id: ServiceId);
}

// overdrive-gateway::ports
#[async_trait::async_trait]
pub trait GatewayConnectDataplane: Send + Sync {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError>;
    fn register(&self, intent: GatewayConnectIntent)
        -> Result<(), GatewayConnectDataplaneError>;
    fn take_receipt(&self, intent: &GatewayConnectIntent)
        -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError>;
    fn cleanup(&self, intent: &GatewayConnectIntent)
        -> Result<(), GatewayConnectDataplaneError>;
    fn cleanup_all_gateway_intents(&self)
        -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError>;
    fn selected_backend_identity(&self, backend_id: BackendId)
        -> Result<SpiffeId, GatewayConnectDataplaneError>;
    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError>;
    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError>;
}
pub struct GatewayConnectCleanupSweep {
    pub removed_intents: u32,
    pub removed_receipts: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GatewayConnectIntent {
    pub socket_cookie: SocketCookie,
    pub service_key: ServiceKey,
}
pub enum GatewaySelectionReceipt {
    Selected {
        socket_cookie: SocketCookie,
        service_key: ServiceKey,
        backend_id: BackendId,
    },
    NoBackend {
        socket_cookie: SocketCookie,
        service_key: ServiceKey,
    },
}
pub enum GatewayConnectDataplaneError {
    Unavailable,
    ProbeTimeout,
    IntentRegistryFull,
    IntentAlreadyRegistered,
    IntentMissing,
    ReceiptMissing,
    ReceiptMismatch,
    BackendIdentityMissing { backend_id: BackendId },
    Kernel,
    Cleanup,
}

// Exact single-cut addition to the existing Action variant, after `backends`
// and before `correlation`; every constructor outside the new demanded
// Path-A branch supplies None.
Action::DataplaneUpdateService {
    service_id: ServiceId,
    vip: ServiceVip,
    port: NonZeroU16,
    proto: Proto,
    backends: Vec<Backend>,
    gateway_demand: Option<GatewayFrontendDemandApply>,
    correlation: CorrelationKey,
}

// Exact revised service-hydration action-shim surface.
pub enum DispatchOutcome { Completed, Failed, StaleGatewayDemand }
pub enum ServiceHydrationDispatchError {
    ObservationWrite { source: ObservationStoreError },
    Ipv6Unsupported { vip: ServiceVip },
    GatewayDemandUnavailable,
}

// in overdrive-control-plane::action_shim::dataplane_update_service
pub async fn dispatch(
    action: &Action,
    dataplane: &dyn Dataplane,
    observation: &dyn ObservationStore,
    tick: &TickContext,
    writer: &NodeId,
    gateway_demand: Option<&GatewayDemandDispatchPorts>,
) -> Result<DispatchOutcome, ServiceHydrationDispatchError>;
```

For `gateway_demand=Some(apply)`, `dispatch` first requires
`gateway_demand: Some(bundle)`, reads `bundle.read().snapshot()`, and compares the
exact revision/ServiceKey before IPv4 validation, fingerprinting, observation
read or Dataplane mutation. Missing bundle returns
`ServiceHydrationDispatchError::GatewayDemandUnavailable`; stale snapshot calls
`bundle.wake().wake(service_id)` and returns `StaleGatewayDemand` with no other
effect. Completed Dataplane application calls
`bundle.acknowledge().applied(apply)` and matches the result. `Applied` writes
the independent Completed hydration row. `Stale` suppresses that row, wakes
the just-applied Service target for purge/reconciliation, snapshots current
demand, wakes every distinct current Service target, and returns
`StaleGatewayDemand`; the already-committed map effect is corrected by those
fresh evaluations rather than falsely labelled current. The outer action-shim
maps only `ObservationWrite` to
`ShimError::Observation`, treats internal `Ipv6Unsupported` as unreachable as
today, maps `GatewayDemandUnavailable` to the new unit
`ShimError::GatewayDemandUnavailable` whose fixed Display is `"gateway demand
unavailable for demand-bearing dataplane action"`, and treats every
`DispatchOutcome` as a completed per-action dispatch. No error string/String
conversion is used.

The two public dispatcher forms change in one cut to these complete
signatures; the workflow/AppState wrappers keep their existing external
signatures and source both new arguments from `AppState`:

```rust
// overdrive-control-plane::action_shim
pub async fn dispatch(
    actions: Vec<Action>,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    ca: &dyn Ca,
    clock: &dyn Clock,
    identity: &IdentityMgr,
    gateway_identity: &GatewayIdentityActionComposition,
    gateway_demand: Option<&GatewayDemandDispatchPorts>,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
    mtls_lifecycle: Option<&dyn MtlsInterceptLifecycle>,
    net_slot_allocator: &NetSlotAllocator,
    host: &dyn VmHostState,
) -> Result<(), ShimError>;

pub async fn dispatch_with_network_provisioner(
    actions: Vec<Action>,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    ca: &dyn Ca,
    clock: &dyn Clock,
    identity: &IdentityMgr,
    gateway_identity: &GatewayIdentityActionComposition,
    gateway_demand: Option<&GatewayDemandDispatchPorts>,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
    mtls_lifecycle: Option<&dyn MtlsInterceptLifecycle>,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
    host: &dyn VmHostState,
) -> Result<(), ShimError>;

pub async fn dispatch_with_workflow_intent(
    actions: Vec<Action>,
    state: &AppState,
    tick: &TickContext,
) -> Result<(), ShimError>;

#[doc(hidden)]
#[cfg(any(test, feature = "integration-tests"))]
pub async fn dispatch_with_workflow_intent_and_network_provisioner_for_test(
    actions: Vec<Action>,
    state: &AppState,
    tick: &TickContext,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError>;
```

The private `dispatch_single` receives the same `gateway_identity` and
`gateway_demand` immediately after `identity`; every public dispatcher passes
them unchanged. `dispatch_with_workflow_intent` and its integration form bind
`let demand_ports = state.gateway.demand().dispatch_ports()` once and borrow that
same value into the complete batch. No overload preserves the old signatures.

The host and `overdrive-bpf` crates mirror this exact private POD ABI; they do
not introduce a public raw-map port:

```rust
pub(crate) const GATEWAY_CONNECT_MAP_CAPACITY: u32 = 128;
pub(crate) const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
pub(crate) const GATEWAY_RECEIPT_SELECTED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewaySelectionReceiptPod {
    pub outcome: u32,
    pub backend_id: u32,
}
```

Both structs are exactly eight bytes, fully initialized, `repr(C)` and keyed
by the socket cookie's host-order `u64`. `GATEWAY_CONNECT_INTENT_MAP` and
`GATEWAY_SELECTION_RECEIPT_MAP` are HASH maps with `max_entries=128`; the
host mirror has `aya::Pod` and compile-time size/alignment assertions, and BPF
has the same assertions. Registration first rejects an already-live cookie,
deletes a stale receipt for that cookie, then inserts the intent. The cgroup
hook uses the intent value byte-for-byte as the existing Service-map key. It
writes `{ outcome=2, backend_id=<selected nonzero id> }` before allowing the
rewritten connect, or `{ outcome=1, backend_id=0 }` before denying a missing/
empty selection. Missing intent follows the existing LOCAL_BACKEND_MAP branch;
any registered-arm decode/map-write failure denies without falling through.
`take_receipt` accepts only those two canonical value shapes, binds the typed
result back to the caller's exact cookie/ServiceKey, and removes the receipt;
the RAII cleanup removes both keys on every later result.

Production `EbpfDataplane::probe` exercises this same port before a public bind:
it creates an IPv4 socket in the real `serve` cgroup, reads its cookie through
the shared `SocketCookieReader`, registers the exact sentinel ServiceKey
`192.0.2.1:9/Tcp`, and calls `connect(2)`. Because the reserved documentation
VIP cannot have a Service-map entry, success means the connect is denied and a
matching canonical `NoBackend` receipt is consumed; missing/malformed receipt,
an allowed connect or nonzero residual intent/receipt fails the probe. This is
a real existing-component drive, not a hand-installed test seam or scratch
spike.
The caller supplies the application-gate absolute deadline and injected Clock;
the sentinel connect races the remaining duration and maps expiry only to
`GatewayConnectDataplaneError::ProbeTimeout`.

`cleanup_all_gateway_intents` keeps its selected public name but its lifecycle
contract deletes both `GATEWAY_CONNECT_INTENT_MAP` and
`GATEWAY_SELECTION_RECEIPT_MAP`, returning exact removed counts. The two live
count methods enumerate their distinct maps after cleanup; failure to enumerate
is typed and never treated as zero.

`IntentServiceFrontendResolver::new(intent, vip_view)` is crate-private and the
only adapter constructor; `UnboundGatewayBuilder::new` is its only caller. The resolver
probe and `resolve` calls from `GatewayApplicationOwner` are the only production
calls through the public port. `GatewayFrontendDemandHandle` is crate-private
and held only by that owner; no handler receives it. The cross-crate surface is
only the read/ack/wake ports above. Every demanded frontend, including an
empty-backend purge, carries `Some(GatewayFrontendDemandApply)` on
`Action::DataplaneUpdateService`; genuinely undemanded actions carry `None`.
The action shim re-reads the exact revision/key before the Dataplane call,
returns `DispatchOutcome::StaleGatewayDemand` with no effect when stale, calls
`acknowledge.applied` only after `update_service` returns committed `Ok`, and
writes the independent hydration observation only when that result is
`GatewayDemandAckOutcome::Applied`. A `Stale` result re-drives the old purge
target plus every latest target and returns Stale without a hydration row.

`wait_applied(ticket, deadline, clock)` registers its revision waiter before
rechecking acknowledgment and races notification/owner closure against
`clock.sleep(deadline.saturating_duration_since(clock.now()))`. It returns
`ApplyTimeout` without altering staged/current/draining state; no Tokio clock
lookup or per-retry deadline reset exists.

The Service target is always
`ServiceId::derive(&frontend.vip(), frontend.port(), frontend.proto(),
"service-map")`; it is never stored in Route or demand. Enabled AppState owns
read+ack+wake as one `GatewayDemandComposition`; disabled AppState owns the
canonical empty read and no dispatch bundle. The sole
`build_hydration_context`, both workflow-intent dispatch wrappers and workflow
emit path consume that same composition.

### Commit-guarded Service-map and applied-identity transaction

Current production `EbpfDataplane::update_service` already performs the same
physical sequence in `overdrive-dataplane/src/lib.rs`: allocate/memoize
BackendIds, write BACKEND_MAP, build/populate a fresh inner ARRAY, insert
reverse-NAT entries, atomically replace the outer SERVICE_MAP fd, then update
trackers and garbage-collect orphans. The production action-shim
`DataplaneUpdateService` arm awaits that method before it writes the hydration
result. The change below wraps that existing caller path and makes its former
post-swap fallible cleanup non-transactional cleanup debt; it does not invent a
second update path or claim rollback of a completed outer swap.

`EbpfDataplane` adds one private `service_apply_commit: RwLock<()>` shared by
`update_service` writers and `selected_backend_identity` readers. Its existing
allocator plus identity state becomes:

The adapter clones/validates its typed input before taking the parking-lot
guard. From guard acquisition through release it performs only synchronous aya
map syscalls and in-memory mutation and contains no `.await`; cancellation
cannot expose an in-between userspace publication state. The surrounding async
trait future can yield only before entering or after returning from this
non-yielding commit section.

```rust
struct BackendIdState {
    allocator: BackendIdAllocator,
    identities: BTreeMap<BackendId, BackendIdentityState>,
}
enum BackendIdentityState {
    Reserved { spiffe_id: SpiffeId, service_key: ServiceKey },
    Applied { spiffe_id: SpiffeId },
}
struct ServiceApplyUndo {
    fresh_ids: Vec<BackendId>,
    inserted_backend_ids: Vec<BackendId>,
    inserted_reverse_keys: Vec<BackendKeyPod>,
}
enum ServiceCleanupDebt {
    Backend(BackendId),
    ReverseNat(BackendKeyPod),
}
pub enum ServiceApplyStage {
    BackendMap, InnerMap, ReverseNat, OuterCommit,
}
pub enum DataplaneError {
    // existing variants unchanged
    BackendIdentityConflict {
        backend_id: BackendId,
        current: SpiffeId,
        attempted: SpiffeId,
    },
    ServiceApplyRollback {
        stage: ServiceApplyStage,
        rollback: Vec<ServiceApplyRollbackFailure>,
    },
}
pub enum ServiceApplyRollbackFailure {
    BackendDelete(BackendId), ReverseNatDelete(BackendKeyPod),
}
impl BackendIdAllocator {
    pub(crate) fn existing_id(
        &self,
        ip_host: u32,
        port_host: u16,
        proto: u8,
    ) -> Option<BackendId>;
}
```

For a non-empty update, exact ordering under the write guard is:

1. Purely validate/deduplicate Backend pods and reject one endpoint carrying
   two `Backend.alloc` identities.
2. Resolve existing allocator memo hits and reject any Applied/Reserved
   identity conflict, differing existing BACKEND_MAP pod, or differing
   reverse-NAT VIP before mutation. An identical existing entry is reused and
   never rewritten. Allocate monotonic fresh IDs, retain their planned identity
   values locally, and record only absent BACKEND_MAP/reverse-NAT keys that this
   turn will insert in `ServiceApplyUndo`.
3. Insert each newly allocated BackendId-to-`Backend.alloc` association as
   `Reserved` in `BackendIdState`; validated memo hits retain their existing
   identical `Applied` association. Reserved is not readable through
   `selected_backend_identity`.
4. Populate BACKEND_MAP, allocate/populate the fresh private Maglev inner map,
   and insert/verify reverse-NAT entries. The current outer pointer is still
   unchanged, so no new ID is selectable.
5. Commit with the one atomic `SERVICE_MAP[ServiceKey] = new_inner_fd` update.
   After it succeeds, change every reservation to Applied with infallible
   in-memory mutation, update trackers, and release the write guard. A receipt
   can occur immediately after the outer swap, but its identity read blocks on
   the guard until Applied publication completes.
6. Run orphan BACKEND/reverse-NAT cleanup after commit. Failure inserts the
   exact key into private `service_cleanup_debt: BTreeSet<ServiceCleanupDebt>`,
   emits a closed `dataplane.service_cleanup.deferred` warning, and is retried
   at the next `update_service`, Dataplane probe and shutdown sweep; it never
   changes a committed update into a Failed hydration result.

Any error before the outer swap replays `ServiceApplyUndo` in reverse order,
deleting only keys proven absent before this turn, removes every Reserved
identity, releases fresh allocator memos without recycling their numeric IDs,
drops the private inner map and leaves the prior outer pointer. Existing equal
entries were never rewritten, so rollback never has to reconstruct a prior
value. Rollback attempts every inserted key even after one failure. A partial
rollback returns `ServiceApplyRollback`, retains the still-inserted safe-delete
keys as closed cleanup debt, and still exposes no Applied identity for the
abandoned IDs; future appearance receives fresh IDs, so abandoned reservations
cannot poison reuse.
When rollback fully succeeds, `update_service` returns the original existing
stage error; `ServiceApplyRollback` replaces it only when one or more restore
delete operations also failed, with `stage` retaining the original failure location.
The empty-backend path takes the same write guard, commits by deleting the
outer ServiceKey first, retains historical Applied identities for old receipts,
then performs idempotent post-commit GC.

`Dataplane::update_service` returns `Ok` only after outer commit plus Applied
publication. The action shim therefore acknowledges Gateway Frontend Demand
only after that point; a pre-commit or rollback error writes the existing
Failed hydration row and sends no acknowledgment. This is userspace
transactional publication around existing kernel primitives, not a claim that
four independent BPF maps update atomically.

### Exact lag-aware intent subscription

The one-cut replacement of `IntentStore::watch` is:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentSubscriptionEvent {
    Changed { key: Bytes, value: Option<Bytes> },
    Lagged { skipped: NonZeroU64 },
}

#[async_trait::async_trait]
pub trait IntentStore: Send + Sync + 'static {
    async fn watch(
        &self,
        prefix: &[u8],
    ) -> Result<
        Box<dyn Stream<Item = IntentSubscriptionEvent> + Send + Unpin>,
        IntentStoreError,
    >;
    // every other existing method is unchanged
}
```

`Changed { value: Some(bytes) }` is a committed put/CAS/transaction value;
`Changed { value: None }` is a committed delete. Empty bytes are a legitimate
value and are never a delete sentinel. `Lagged.skipped` is the adapter-reported
nonzero count of missed events; LocalStore and Sim adapters preserve it.
Gateway Route/application/custody watchers and every existing IntentStore
watcher match both variants. On Lagged or stream closure, the owner closes new
admission, resubscribes before an authoritative prefix relist, then replaces
derived state; no wildcard arm may treat loss as an empty store.

### Required cross-crate value constructors and accessors

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketCookie(NonZeroU64);
impl SocketCookie {
    pub fn new(raw: u64) -> Result<Self, SocketCookieParseError>;
    pub const fn get(self) -> NonZeroU64;
}
pub enum SocketCookieParseError { Empty, InvalidDecimal, Zero }
// FromStr/TryFrom<&str>/TryFrom<String> delegate to decimal new;
// new validates u64 while FromStr maps lexical errors into the same type.
// Display and string Serialize/Deserialize use canonical decimal exactly.

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct GatewayIdentityEpoch(NonZeroU64);
impl GatewayIdentityEpoch {
    pub fn new(raw: u64) -> Result<Self, GatewayIdentityEpochError>;
    pub const fn first() -> Self; // exactly 1
    pub fn checked_next(self) -> Result<Self, GatewayIdentityEpochError>;
    pub const fn get(self) -> NonZeroU64;
}
pub enum GatewayIdentityEpochError { Empty, InvalidDecimal, Zero, Exhausted }
// FromStr/TryFrom parse canonical decimal through new; Display and matching
// string Serialize/Deserialize are canonical decimal; checked_next never wraps.

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct CertifiedKeyGeneration([u8; 32]);
impl CertifiedKeyGeneration {
    pub fn new(raw: &str) -> Result<Self, DigestHexParseError>;
    pub fn from_ordered_chain_der(chain: &[Vec<u8>]) -> Self;
    pub const fn as_bytes(&self) -> &[u8; 32];
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct CertificateFingerprint([u8; 32]);
impl CertificateFingerprint {
    pub fn new(raw: &str) -> Result<Self, DigestHexParseError>;
    pub fn from_leaf_der(leaf: &[u8]) -> Self;
    pub const fn as_bytes(&self) -> &[u8; 32];
}
pub enum DigestHexParseError { Empty, InvalidLength { got: usize }, NonHex, Uppercase }
// Both hash wrappers implement case-sensitive FromStr/TryFrom through new,
// 64-character lowercase-hex Display, matching string Serialize/Deserialize,
// and rkyv. Uppercase input is rejected rather than normalized.
```

`CertifiedKeyGeneration::from_ordered_chain_der` hashes the fixed domain
`b"overdrive/gateway/certified-key-generation/v1\0"` followed, in order, by
each certificate's `u64::to_be_bytes(len)` and DER bytes. Fingerprints hash
`b"overdrive/gateway/certificate-fingerprint/v1\0" || leaf_der`. The live
callers are `CertifiedKeyCandidate::from_pkcs8` and application-generation
construction/status projection. `SocketCookieReader` is the only raw-u64
constructor caller for SocketCookie; the BPF-map adapter uses `get`.
`GatewayIdentitySlot::new` and `begin_disable` are the only epoch constructors;
Actions, reconciliation and slot hold/drop carry the typed value.

For all three decimal newtypes, FromStr rejects whitespace, a plus sign,
negative input and leading zeroes; it parses base-10 u64, then delegates to the
validated numeric constructor. Serialize emits exactly the Display string and
Deserialize accepts only that FromStr grammar. All four SHA-256 wrappers accept
exactly 64 lowercase ASCII hex characters, reject uppercase/non-hex/other
lengths, and use the same Display string for serde. No derive is allowed to
silently choose numeric, byte-array or uppercase-compatible wire shape.

## Wave: DESIGN / [REF] Persistence, custody and redaction

| Record | Exact key/codec | Stored inputs | Forbidden content |
|---|---|---|---|
| `PublicRouteSetEnvelope::V1` | `public-ingress/route-set`; typed rkyv codec | Empty/occupied Route, deterministic Route Generation | VIP, ServiceId, backend, AllocationId, SPIFFE ID, BPF state |
| `PublicCertifiedKeyEnvelope::V1` | `public-ingress/certified-key/<PublicCertifiedKeyId>`; typed rkyv codec | ID/hostname/generation/leaf fingerprint/ordered chain DER/validity/producer provenance + AES-GCM protected key inputs | Plaintext key, file path, compiled rustls config, ACME/order state |
| `PublicCertifiedKeyStatusRowEnvelope::V1` | table `public_certified_key_status`, key canonical certified-key ID bytes | Active usable metadata or retained-generation unusable cause, plus optional latest install failure | Paths, PEM/key bytes, raw parser/crypto error string |
| `GatewayApplicationStatusRowEnvelope::V1` | table `gateway_application_status`, key canonical NodeId bytes | listener fact; staged/current/draining generations+demand revisions; unavailable cause; non-secret gateway identity/connect-probe facts | Backend candidates, BackendId/receipt/peer, ServiceBackendRow/BPF generation, credential bytes |

Each envelope implements existing `VersionedEnvelope` with `Latest = V1 payload`, and
each payload owns the established typed codec surface:

```rust
impl PublicRouteSetV1 {
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError>;
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError>;
}
impl PublicCertifiedKeyV1 {
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError>;
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError>;
}
impl PublicCertifiedKeyStatusRowV1 {
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError>;
    pub fn from_store_bytes(
        bytes: &[u8],
        key: Option<&str>,
    ) -> Result<Self, ObservationStoreError>;
}
impl GatewayApplicationStatusRowV1 {
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError>;
    pub fn from_store_bytes(
        bytes: &[u8],
        key: Option<&str>,
    ) -> Result<Self, ObservationStoreError>;
}
```

Intent decode uses ADR-0048 fail-fast `health.startup.refused`; observation
decode emits one `observation.row.skipped` warning and skips only that row.
Writers construct envelopes only through `VersionedEnvelope::latest`; no raw
rkyv invocation exists outside these payload methods. Golden bytes and
round-trip/schema-evolution properties pin all four V1 formats.

### Exact Public Certified-Key Custody API and record

Crate placement is exact: provenance/usability/failure value enums, generation/
fingerprint/context and V1 protected records/envelopes live in
`overdrive-core::public_ingress`; candidate/snapshot/availability/install
commands, custody handle and ownership errors live in
`overdrive-gateway::certified_key`; `PublicCertifiedKeyAeadCodec` alone lives in
`overdrive-host::ca`. Thus host depends only on core, while gateway may compose
the host adapter without a cycle.

```rust
pub enum CertifiedKeyProvenance {
    Manual,
    Workflow { correlation: CorrelationKey },
}
pub struct InstallPublicCertifiedKey {
    pub expected_generation: Option<CertifiedKeyGeneration>,
    pub candidate: CertifiedKeyCandidate,
    pub provenance: CertifiedKeyProvenance,
}
pub enum PublicCertifiedKeyInstallOutcome { Installed, Replaced, Unchanged }
pub struct RecordPublicCertifiedKeyFailure {
    pub expected_generation: Option<CertifiedKeyGeneration>,
    pub provenance: CertifiedKeyProvenance,
    pub failure: CertifiedKeyFailure,
}
pub enum PublicCertifiedKeyFailureRecordOutcome { Recorded, Superseded }
pub enum CertifiedKeyUsabilityFailure { NotYetValid, Expired }
pub enum CertifiedKeyFailure {
    SourceOpen, SourceMetadata, SourceOwnership, SourcePermissions,
    SourceBounds, PemShape, CertificateProfile, HostnameMismatch,
    KeyMismatch, Protection, Persistence, RustlsConfiguration,
}
pub enum PublicCertifiedKeyError {
    Source(CertifiedKeyFailure),
    ExpectedGenerationMismatch {
        expected: Option<CertifiedKeyGeneration>,
        current: Option<CertifiedKeyGeneration>,
    },
    Protection(PublicCertifiedKeyAeadError),
    Intent(IntentStoreError),
    Observation(ObservationStoreError),
    RustlsConfiguration,
    OwnerUnavailable,
}
pub enum PublicCertifiedKeyAeadError {
    KekUnavailable, SealFailed, OpenFailed, AuthenticationFailed,
}
pub enum PublicCertifiedKeyAvailability {
    Absent,
    Usable(PublicCertifiedKeySnapshot),
    Unusable {
        generation: CertifiedKeyGeneration,
        cause: CertifiedKeyUsabilityFailure,
    },
}
#[derive(Clone)]
pub struct PublicCertifiedKeyCustodyHandle { /* private command/watch state */ }
impl PublicCertifiedKeyCustodyHandle {
    pub fn id(&self) -> &PublicCertifiedKeyId;
    pub async fn install(
        &self,
        command: InstallPublicCertifiedKey,
    ) -> Result<PublicCertifiedKeyInstallOutcome, PublicCertifiedKeyError>;
    pub async fn record_install_failure(
        &self,
        command: RecordPublicCertifiedKeyFailure,
    ) -> Result<PublicCertifiedKeyFailureRecordOutcome, PublicCertifiedKeyError>;
    pub fn snapshot(&self) -> PublicCertifiedKeyAvailability;
    pub fn subscribe(&self)
        -> watch::Receiver<PublicCertifiedKeyAvailability>;
}

pub struct ProtectedOriginKeyV1 {
    pub kek_id: String,
    pub salt: [u8; 32],
    pub nonce: [u8; 12],
    pub ciphertext_and_tag: Vec<u8>,
}
pub struct PublicCertifiedKeyV1 {
    pub certified_key_id: PublicCertifiedKeyId,
    pub public_hostname: PublicHostname,
    pub certified_key_generation: CertifiedKeyGeneration,
    pub certificate_fingerprint: CertificateFingerprint,
    pub certificate_chain_der: Vec<Vec<u8>>,
    pub not_before: UnixInstant,
    pub not_after: UnixInstant,
    pub provenance: CertifiedKeyProvenance,
    pub protected_origin_key: ProtectedOriginKeyV1,
}
pub enum PublicCertifiedKeyEnvelope { V1(PublicCertifiedKeyV1) }

pub struct CertifiedKeyCandidate { /* fixed-redaction, zeroizing key owner */ }
impl CertifiedKeyCandidate {
    pub fn from_pkcs8(
        chain_der: Vec<Vec<u8>>,
        private_key_pkcs8: Zeroizing<Vec<u8>>,
        now: UnixInstant,
    ) -> Result<Self, PublicCertifiedKeyError>;
}
pub struct PublicCertifiedKeySnapshot { /* private metadata + ServerConfig */ }
impl PublicCertifiedKeySnapshot {
    pub fn certified_key_id(&self) -> &PublicCertifiedKeyId;
    pub fn public_hostname(&self) -> &PublicHostname;
    pub const fn generation(&self) -> CertifiedKeyGeneration;
    pub const fn fingerprint(&self) -> CertificateFingerprint;
    pub const fn not_before(&self) -> UnixInstant;
    pub const fn not_after(&self) -> UnixInstant;
    pub fn is_usable_at(&self, now: UnixInstant) -> bool;
    pub(crate) fn server_config(&self) -> Arc<rustls::ServerConfig>;
}

pub struct PublicCertifiedKeyProtectionContext {
    pub certified_key_id: PublicCertifiedKeyId,
    pub public_hostname: PublicHostname,
    pub generation: CertifiedKeyGeneration,
}
pub struct PublicCertifiedKeyAeadCodec { /* Kek + ring SystemRandom */ }
impl PublicCertifiedKeyAeadCodec {
    pub fn new(kek: Arc<dyn Kek>) -> Self;
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn with_random_fill_for_test(
        kek: Arc<dyn Kek>,
        fill: Arc<
            dyn Fn(&mut [u8]) -> Result<(), PublicCertifiedKeyAeadError>
                + Send
                + Sync,
        >,
    ) -> Self;
    pub fn seal(
        &self,
        context: &PublicCertifiedKeyProtectionContext,
        key_pkcs8: &Zeroizing<Vec<u8>>,
    ) -> Result<ProtectedOriginKeyV1, PublicCertifiedKeyAeadError>;
    pub fn open(
        &self,
        context: &PublicCertifiedKeyProtectionContext,
        protected: &ProtectedOriginKeyV1,
    ) -> Result<Zeroizing<Vec<u8>>, PublicCertifiedKeyAeadError>;
}
```

`PublicCertifiedKeySnapshot` and `CertifiedKeyCandidate` have private fields,
fixed-redaction `Debug`, no `Display`/serde/rkyv, and zeroizing plaintext key
ownership. `CertifiedKeyCandidate::from_pkcs8` is the only candidate
constructor.
`ProtectedOriginKeyV1`, `PublicCertifiedKeyV1` and their envelope derive only
the rkyv/value traits required by the typed store codec; their manual `Debug`
prints IDs/generation/validity plus `[REDACTED]` for ciphertext, nonce, salt,
chain bytes and provenance correlation, and they have no `Display` or wire
serde. Every public error renders only the closed failure category, never a
path, PEM/DER/ciphertext, key byte, parser source string or rustls error.
`PublicCertifiedKeyCustodyHandle::install` is the only first-slice credential
mutation; `record_install_failure` changes only redacted status.
There is no public `withdraw` method because no first-slice production caller
can reach it. The manual source calls `install` and `record_install_failure`;
`GatewayApplicationOwner` calls `snapshot` at authoritative relist and retains
`subscribe` for key replacement/expiry wakes; the Route resolver calls `id`.

The exact successful install transaction, serialized by the custody owner, is:

1. Read the current durable/live generation and evaluate the expected-
   generation guard. A mismatch has no record, publication or status delta.
2. Validate the complete chain/profile/hostname/key match and require
   `not_before <= now < not_after`. An invalid or currently unusable candidate
   is never sealed or persisted; generation-checked `record_install_failure`
   updates only the redacted status failure while the prior generation remains
   current, or returns `Superseded` with no delta if Current has changed.
3. Build the rustls `CertifiedKey`/resolver snapshot from the validated
   in-memory candidate, then seal that same candidate into the proposed
   `PublicCertifiedKeyV1`. Rustls-build or seal failure leaves durable/live
   current unchanged.
4. Persist the complete proven-usable V1 envelope at
   `public-ingress/certified-key/<id>`.
5. In the same owner turn, infallibly ArcSwap-publish the already-built opaque
   snapshot. Only now is the new generation Current/Usable.
6. After publication, write `PublicCertifiedKeyStatusRowV1` with the new active
   generation and cleared `last_install_failure`. Status failure is retained
   for retry on the private maintenance cadence and does not roll back or turn
   the already-committed install reply into an error.

Crash/restart adoption is exact: before step 4, restart sees/adopts the prior
record; after step 4 but before step 5, restart authenticates/decrypts,
revalidates time/profile and builds/adopts the new durable record; after step 5
but before step 6, restart adopts the same new record and repairs status. A
durable record that has since expired is retained but published as Unusable;
it is never silently replaced. Runtime expiry first clears new admission/live
Usable publication and then writes Unusable status. Status is always
after install/publication (or after a failed candidate decision); it is never a
pre-commit promise.

The key holder has no `Display`/`Serialize`, fixed-redaction `Debug`, and
zeroizes plaintext on drop. `GatewayApplicationSnapshot` holds only an opaque
`Arc<PublicCertifiedKeySnapshot>`. `CertifiedKeyGeneration` hashes the ordered
chain DER; `CertificateFingerprint` hashes the leaf DER; neither hashes the
private key.

Manual files must be absolute, final-component non-symlinks and regular files
owned by the effective UID. The key has no group/other mode bits, maximum
64 KiB, and exactly one PKCS#8 block; the certificate file is at most 1 MiB
with 1..=8 certificates. The single leaf dNSName is the `PublicHostname`;
ASCII case is normalized lowercase; wildcard/IP/multiple SANs and invalid/
expired/mismatched material fail closed.
Initial failure refuses enabled startup; later invalid refresh preserves the
whole prior generation only while custody still classifies it time-usable and
records failure. Custody schedules exact `not_after`, rechecks both validity
bounds on its private finite maintenance cadence, and atomically publishes `Unusable` before status when
the current generation is expired/not-yet-valid. The accept loop independently
checks the retained snapshot immediately after accept and before rustls, so a
timer race cannot start new TLS with invalid material. The protected record is
retained for a generation-checked valid replacement.

The future #57 producer ends at `PublicCertifiedKeyCustodyHandle::install` and
cannot add a second store, TLS manager or resolver. ACME account/order/challenge
state remains wholly outside these first-slice records.

`ManualCertifiedKeySource` is the only first-slice caller of
`CertifiedKeyCandidate::from_pkcs8`. Custody alone calls every snapshot
metadata method, `is_usable_at`, and the codec's `seal`/`open`; the public TLS
runtime alone calls crate-private `server_config`.
`run_server_with_obs_and_drivers` constructs one codec with the boot-probed
`Kek` and passes it to `UnboundGatewayBuilder::new`. No adapter receives a generic
AAD string or an untyped key identifier.

The codec contract is byte-exact: `kek_id` must equal
`"overdrive-public-certified-key"`; HKDF-SHA256 uses the record's 32 random
salt and info `b"overdrive/public-certified-key/aes-256-gcm/v1"` to derive the
32-byte AES-256-GCM key. GCM uses a fresh 12-byte nonce. AAD is
`b"overdrive/public-certified-key/record/v1\0"` followed by the certified-key
ID and hostname as `u32::to_be_bytes(len) || bytes`, then the 32 generation
bytes. Plaintext is exactly the canonical PKCS#8 DER bytes. Open rejects a
different KEK ID, nonce/salt size, AAD context or authentication tag before
returning a zeroizing buffer. CA root/intermediate codecs use none of these
domain bytes.

Earned Trust drives the real paths. `ManualCertifiedKeySource::probe` performs
the full bounded file/parse/validation path and returns the initial candidate,
which boot consumes without a second read. `PublicCertifiedKeyCustody::probe`
resolves the real KEK and either decodes/authenticates/rebuilds the configured
stored record or performs a non-persisted canary seal/open through the same
public-key codec when absent; `start` adopts that recovered object without a
second read. Source, store, KEK, malformed/tampered envelope, key-match and
snapshot-rebuild fault cases plus CSPRNG/seal `ProtectionFailed` are required
evidence. `PublicCertifiedKeyAeadCodec::new` uses ring `SystemRandom`;
an `integration-tests`-only random-fill closure constructor makes deterministic
salt/nonce and draw failure testable without a default-feature entropy seam.

## Wave: DESIGN / [REF] Gateway Application, status and HTTP contract

### Exact owner, composition, generation and shutdown surface

Only the composition root crosses the new library boundary. The candidate,
admission snapshot and leases stay crate-private so handlers cannot assemble a
partially gated generation:

```rust
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct GatewayApplicationGenerationId([u8; 32]);
impl GatewayApplicationGenerationId {
    pub fn new(raw: &str) -> Result<Self, DigestHexParseError>;
    pub fn from_inputs(
        route_generation: RouteGeneration,
        frontend: ServiceFrontend,
        certified_key_generation: CertifiedKeyGeneration,
    ) -> Self;
    pub const fn as_bytes(&self) -> &[u8; 32];
}
// GatewayApplicationGenerationId implements case-sensitive FromStr/TryFrom via
// new, 64-character lowercase-hex Display, matching string
// Serialize/Deserialize, and rkyv; uppercase input is rejected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct GatewayAppliedGeneration {
    application_generation: GatewayApplicationGenerationId,
    demand_revision: GatewayFrontendDemandRevision,
}
impl GatewayAppliedGeneration {
    pub(crate) const fn from_promotion(
        application_generation: GatewayApplicationGenerationId,
        demand_revision: GatewayFrontendDemandRevision,
    ) -> Self;
    pub const fn application_generation(&self) -> GatewayApplicationGenerationId;
    pub const fn demand_revision(&self) -> GatewayFrontendDemandRevision;
}

pub struct GatewayDemandComposition {
    kind: GatewayDemandCompositionKind,
}
enum GatewayDemandCompositionKind {
    Disabled {
        read: Arc<dyn GatewayFrontendDemandRead>,
    },
    Enabled {
        read: Arc<dyn GatewayFrontendDemandRead>,
        acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
        wake: Arc<dyn GatewayFrontendDemandWake>,
    },
}
impl GatewayDemandComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(
        read: Arc<dyn GatewayFrontendDemandRead>,
        acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
        wake: Arc<dyn GatewayFrontendDemandWake>,
    ) -> Self;
    pub fn read(&self) -> &dyn GatewayFrontendDemandRead;
    pub fn dispatch_ports(&self) -> Option<GatewayDemandDispatchPorts>;
}
#[derive(Clone)]
pub struct GatewayDemandDispatchPorts {
    read: Arc<dyn GatewayFrontendDemandRead>,
    acknowledge: Arc<dyn GatewayFrontendDemandAcknowledge>,
    wake: Arc<dyn GatewayFrontendDemandWake>,
}
impl GatewayDemandDispatchPorts {
    pub fn read(&self) -> &dyn GatewayFrontendDemandRead;
    pub fn acknowledge(&self) -> &dyn GatewayFrontendDemandAcknowledge;
    pub fn wake(&self) -> &dyn GatewayFrontendDemandWake;
}
#[derive(Debug, Default)]
pub struct EmptyGatewayFrontendDemand;
impl GatewayFrontendDemandRead for EmptyGatewayFrontendDemand {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot>;
}
impl GatewayFrontendDemandRead for GatewayDemandComposition {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot>;
}

#[derive(Clone)]
pub struct GatewayAppStateComposition {
    demand: Arc<GatewayDemandComposition>,
    identity_desired: Arc<dyn GatewayIdentityDesiredRead>,
    identity_current: Arc<dyn GatewayIdentityCurrentRead>,
    identity_actions: GatewayIdentityActionComposition,
    control: Option<GatewayControl>,
}
impl GatewayAppStateComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(
        demand: Arc<GatewayDemandComposition>,
        identity_desired: Arc<dyn GatewayIdentityDesiredRead>,
        identity_current: Arc<dyn GatewayIdentityCurrentRead>,
        identity_actions: GatewayIdentityActionComposition,
        control: GatewayControl,
    ) -> Self;
    pub fn demand(&self) -> &GatewayDemandComposition;
    pub fn identity_desired(&self) -> &dyn GatewayIdentityDesiredRead;
    pub fn identity_current(&self) -> &dyn GatewayIdentityCurrentRead;
    pub fn identity_actions(&self) -> &GatewayIdentityActionComposition;
    pub fn control(&self) -> Option<&GatewayControl>;
}
#[derive(Debug)]
pub(crate) struct DisabledGatewayIdentityRead {
    desired: GatewayIdentityDesired,
}
impl DisabledGatewayIdentityRead {
    pub(crate) fn canonical() -> Arc<Self>;
}
impl GatewayIdentityDesiredRead for DisabledGatewayIdentityRead {
    fn desired(&self) -> GatewayIdentityDesired;
}
impl GatewayIdentityCurrentRead for DisabledGatewayIdentityRead {
    fn current(&self) -> Option<GatewayIdentityFacts>;
}

pub struct UnboundGatewayBuilder { /* validated config + probed owners */ }
impl UnboundGatewayBuilder {
    pub async fn new(
        config: GatewayConfig,
        intent: Arc<dyn IntentStore>,
        observations: Arc<dyn ObservationStore>,
        service_vip_view: Arc<dyn ServiceVipView>,
        key_codec: Arc<PublicCertifiedKeyAeadCodec>,
        cookie_reader: Arc<SocketCookieReader>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, GatewayBootError>;
    pub fn bind_demand_wake(
        self,
        wake: Arc<dyn GatewayFrontendDemandWake>,
    ) -> (GatewayBuilder, GatewayDemandComposition, GatewayUpstreamSeal);
}
pub struct GatewayRuntimeDependencies { /* private fields */ }
impl GatewayRuntimeDependencies {
    pub fn new(
        limits: GatewayLimits,
        dataplane: Arc<dyn GatewayConnectDataplane>,
        client_mtls: Arc<dyn GatewayClientMtls>,
        identity_lifecycle: Arc<dyn GatewayIdentityLifecycleControl>,
    ) -> Self;
}
pub struct GatewayBuilder { /* wake-bound typestate */ }
impl GatewayBuilder {
    pub async fn start(
        self,
        dependencies: GatewayRuntimeDependencies,
    ) -> StartedGateway;
}
pub struct StartedGateway { /* private handle + control */ }
impl StartedGateway {
    pub fn into_parts(self) -> (GatewayHandle, GatewayControl);
}

// Exact AppState single-cut delta; all existing fields remain unchanged.
#[derive(Clone)]
pub struct AppState {
    // existing fields unchanged
    pub gateway: GatewayAppStateComposition,
}
impl AppState {
    // Existing `new(...)` signature is unchanged and always stores disabled().
    pub fn new_with_workflow_engine(
        // every existing parameter through `frontend_addr_allocator` unchanged,
        gateway: GatewayAppStateComposition,
    ) -> Self;
}

#[derive(Clone)]
pub struct GatewayControl { /* Route command + redacted reads only */ }
impl GatewayControl {
    pub fn routes(&self) -> &PublicRouteSetHandle;
    pub async fn application_status(
        &self,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError>;
    pub async fn certified_key_status(
        &self,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, GatewayStatusReadError>;
}
pub enum GatewayStatusReadError {
    Application(ObservationStoreError),
    CertifiedKey(ObservationStoreError),
}

pub(crate) struct GatewayTaskTree {
    listener: Option<JoinHandle<Result<(), GatewayOwnerTaskError>>>,
    connections: Option<JoinHandle<Result<(), GatewayOwnerTaskError>>>,
    application: JoinHandle<Result<(), GatewayOwnerTaskError>>,
    demand: JoinHandle<Result<(), GatewayOwnerTaskError>>,
    manual_source: JoinHandle<Result<(), GatewayOwnerTaskError>>,
    custody: JoinHandle<Result<(), GatewayOwnerTaskError>>,
    route_set: JoinHandle<Result<(), GatewayOwnerTaskError>>,
}
impl GatewayTaskTree {
    pub(crate) async fn join_listener(
        &mut self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Option<GatewayTaskFailure>;
    pub(crate) async fn join_connections(
        &mut self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Option<GatewayTaskFailure>;
    pub(crate) async fn join_remaining(
        self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Vec<GatewayTaskFailure>;
}
pub(crate) struct GatewayShutdownCapabilities {
    application: GatewayApplicationHandle,
    connections: GatewayConnectionOwnerHandle,
    demand: GatewayFrontendDemandHandle,
    dataplane: Arc<dyn GatewayConnectDataplane>,
    cleanup_ledger: Arc<GatewayCleanupLedger>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    clock: Arc<dyn Clock>,
    application_shutdown: CancellationToken,
    demand_shutdown: CancellationToken,
    manual_source_shutdown: CancellationToken,
    custody_shutdown: CancellationToken,
    route_set_shutdown: CancellationToken,
}
pub struct GatewayHandle {
    tasks: GatewayTaskTree,
    shutdown: GatewayShutdownCapabilities,
}
impl GatewayHandle {
    pub async fn shutdown(
        self,
        grace: Duration,
    ) -> GatewayShutdownReport;
}
pub struct GatewayShutdownReport {
    pub forced_connections: u32,
    pub task_failures: Vec<GatewayTaskFailure>,
    pub cleanup_failures: Vec<GatewayCleanupFailure>,
    pub application_errors: Vec<GatewayApplicationCommandError>,
    pub identity_error: Option<GatewayIdentityLifecycleError>,
    pub identity_empty: bool,
    pub residual_connect: Option<GatewayResidualConnectState>,
}
impl GatewayShutdownReport {
    pub fn is_clean(&self) -> bool;
}
pub struct GatewayResidualConnectState {
    pub live_intents: Option<u32>,
    pub live_receipts: Option<u32>,
    pub sweep_failures: Vec<GatewayConnectSweepFailure>,
    pub ledger_entries: u32,
}
pub enum GatewayConnectSweepFailure {
    CleanupAll(GatewayConnectDataplaneError),
    IntentCount(GatewayConnectDataplaneError),
    ReceiptCount(GatewayConnectDataplaneError),
}
pub(crate) struct GatewayConnectionOwnerHandle { /* close/drain/join commands */ }
pub(crate) struct GatewayCleanupLedger {
    capacity: NonZeroU32,
    entries: Mutex<BTreeMap<SocketCookie, GatewayCleanupRecord>>,
    saturation_count: AtomicU64,
}
pub(crate) struct GatewayCleanupRecord {
    intent: GatewayConnectIntent,
    connect_intent_failed: bool,
    receipt_failed: bool,
}
pub(crate) struct GatewayCleanupReservation {
    ledger: Arc<GatewayCleanupLedger>,
    socket_cookie: SocketCookie,
}
pub(crate) struct GatewayCleanupLedgerFull {
    pub capacity: NonZeroU32,
}
pub(crate) struct GatewayApplicationOwner {
    node_id: NodeId,
    intent: Arc<dyn IntentStore>,
    certified_key: PublicCertifiedKeyCustodyHandle,
    frontend: Arc<dyn ServiceFrontendResolve>,
    demand: GatewayFrontendDemandHandle,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    connector: Arc<GatewayUpstreamConnector>,
    admission: Arc<GatewayAdmission>,
    listener: GatewayListenerControl,
    observations: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
    limits: Arc<GatewayLimits>,
    commands: mpsc::Receiver<GatewayApplicationCommand>,
    shutdown: CancellationToken,
    staged: Option<GatewayApplicationCandidate>,
    current: Option<Arc<GatewayApplicationSnapshot>>,
    draining: BTreeMap<GatewayApplicationGenerationId, Arc<GatewayApplicationSnapshot>>,
}
pub(crate) struct GatewayApplicationHandle {
    commands: mpsc::Sender<GatewayApplicationCommand>,
    clock: Arc<dyn Clock>,
}
pub(crate) enum GatewayApplicationCommand {
    CloseForShutdown {
        deadline: Instant,
        reply: oneshot::Sender<
            Result<GatewayApplicationCloseResult, GatewayApplicationCommandError>,
        >,
    },
    RetireClosed {
        deadline: Instant,
        reply: oneshot::Sender<Result<(), GatewayApplicationCommandError>>,
    },
}
pub enum GatewayApplicationCommandPhase { Close, Retire }
pub(crate) struct GatewayApplicationCloseResult {
    pub(crate) retentions: Vec<Arc<GatewayGenerationRetention>>,
    pub(crate) staged_demand_failure: Option<GatewayStagedDemandFailure>,
}
pub(crate) struct GatewayStagedDemandFailure {
    pub(crate) generation: GatewayApplicationGenerationId,
    pub(crate) source: GatewayFrontendDemandError,
}
pub enum GatewayApplicationCommandError {
    OwnerUnavailable,
    Timeout { phase: GatewayApplicationCommandPhase },
    Demand(GatewayFrontendDemandError),
}
pub(crate) struct GatewayListenerControl { /* one listener command owner */ }
impl GatewayListenerControl {
    pub(crate) async fn bind(
        &self,
        address: SocketAddrV4,
        admission: Arc<GatewayAdmission>,
    ) -> Result<(), std::io::Error>;
    pub(crate) async fn unbind(&self) -> Result<(), std::io::Error>;
    pub(crate) fn is_bound(&self) -> bool;
}
impl GatewayApplicationOwner {
    pub(crate) async fn run(self) -> Result<(), GatewayOwnerTaskError>;
}
impl GatewayApplicationHandle {
    pub(crate) async fn close_for_shutdown(
        &self,
        deadline: Instant,
    ) -> Result<GatewayApplicationCloseResult, GatewayApplicationCommandError>;
    pub(crate) async fn retire_closed(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayApplicationCommandError>;
}
impl GatewayConnectionOwnerHandle {
    pub(crate) fn stop_accepting(&self);
    pub(crate) async fn drain_then_force_join(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<u32, GatewayTaskFailure>;
}
impl GatewayCleanupLedger {
    pub(crate) fn new(capacity: NonZeroU32) -> Arc<Self>;
    pub(crate) fn reserve(
        self: &Arc<Self>,
        intent: GatewayConnectIntent,
    ) -> Result<GatewayCleanupReservation, GatewayCleanupLedgerFull>;
    pub(crate) fn retry_all(
        &self,
        dataplane: &dyn GatewayConnectDataplane,
    ) -> Vec<GatewayCleanupFailure>;
    pub(crate) fn len(&self) -> u32;
}
impl GatewayCleanupReservation {
    pub(crate) fn record_intent_failure(&self);
    pub(crate) fn record_receipt_failure(&self);
    pub(crate) fn complete(self);
}
pub(crate) enum GatewayOwnerTaskError { Failed, ChannelClosed }
pub enum GatewayTaskName {
    Listener, Connections, Application, Demand, ManualCertifiedKeySource,
    PublicCertifiedKeyCustody, PublicRouteSet,
}
pub enum GatewayTaskFailureCause {
    OwnerFailed, ChannelClosed, Timeout, Cancelled, Panicked,
}
pub struct GatewayTaskFailure {
    pub task: GatewayTaskName,
    pub cause: GatewayTaskFailureCause,
}
pub enum GatewayCleanupFailure {
    ConnectIntent { count: u32 },
    Receipt { count: u32 },
    LedgerSaturated { attempts: u64, capacity: u32 },
}
pub enum GatewayBootError {
    Configuration(GatewayConfigError),
    RouteStore(PublicRouteSetError),
    CertifiedKey(PublicCertifiedKeyError),
    ManualCertifiedKey(PublicCertifiedKeyError),
}

pub(crate) struct GatewayApplicationCandidate {
    generation: GatewayApplicationGenerationId,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
}
impl GatewayApplicationCandidate {
    pub(crate) fn new(
        route: Arc<Route>,
        frontend: ServiceFrontend,
        certified_key: Arc<PublicCertifiedKeySnapshot>,
    ) -> Result<Self, GatewayApplicationError>;
}
pub(crate) struct GatewayApplicationSnapshot {
    applied: GatewayAppliedGeneration,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
    retention: Arc<GatewayGenerationRetention>,
    budget: Arc<GatewayAdmissionBudget>,
}
pub(crate) struct GatewayGenerationRetention {
    generation: GatewayApplicationGenerationId,
    closed: AtomicBool,
    connections: AtomicU32,
    requests: AtomicU32,
    owner_wake: Arc<Notify>,
}
impl GatewayGenerationRetention {
    pub(crate) fn new(
        generation: GatewayApplicationGenerationId,
        owner_wake: Arc<Notify>,
    ) -> Arc<Self>;
    pub(crate) fn try_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationConnectionPermit, GatewayAdmissionError>;
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationRequestPermit, GatewayAdmissionError>;
    pub(crate) fn close(&self);
    pub(crate) async fn wait_zero(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayDrainTimeout>;
    pub(crate) fn counts(&self) -> (u32, u32);
}
pub(crate) struct GatewayGenerationConnectionPermit {
    retention: Arc<GatewayGenerationRetention>,
}
pub(crate) struct GatewayGenerationRequestPermit {
    retention: Arc<GatewayGenerationRetention>,
}
pub(crate) struct GatewayAdmissionBudget {
    limits: Arc<GatewayLimits>,
    public_connections: AtomicU32,
    inflight_connects: AtomicU32,
    inflight_requests: AtomicU32,
}
impl GatewayAdmissionBudget {
    pub(crate) fn new(limits: Arc<GatewayLimits>) -> Arc<Self>;
    pub(crate) fn try_public_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayPublicConnectionPermit, GatewayAdmissionError>;
    pub(crate) fn try_connect(
        self: &Arc<Self>,
    ) -> Result<GatewayConnectPermit, GatewayAdmissionError>;
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGlobalRequestPermit, GatewayAdmissionError>;
}
pub(crate) struct GatewayPublicConnectionPermit {
    budget: Arc<GatewayAdmissionBudget>,
}
pub(crate) struct GatewayConnectPermit {
    budget: Arc<GatewayAdmissionBudget>,
}
pub(crate) struct GatewayGlobalRequestPermit {
    budget: Arc<GatewayAdmissionBudget>,
}
impl GatewayApplicationSnapshot {
    pub(crate) const fn applied(&self) -> GatewayAppliedGeneration;
    pub(crate) fn route(&self) -> &Route;
    pub(crate) const fn frontend(&self) -> ServiceFrontend;
    pub(crate) fn certified_key(&self) -> &PublicCertifiedKeySnapshot;
}
pub(crate) struct GatewayAdmission {
    current: Arc<ArcSwapOption<GatewayApplicationSnapshot>>,
    budget: Arc<GatewayAdmissionBudget>,
}
impl GatewayAdmission {
    pub(crate) fn acquire_connection(
        &self,
        now: UnixInstant,
    ) -> Result<GatewayConnectionLease, GatewayAdmissionError>;
}
pub(crate) struct GatewayConnectionLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationConnectionPermit,
    public_connection: GatewayPublicConnectionPermit,
    requests_started: AtomicU32,
    request_active: Arc<AtomicBool>,
}
impl GatewayConnectionLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot;
    pub(crate) fn try_request(
        &self,
    ) -> Result<GatewayRequestLease, GatewayAdmissionError>;
}
pub(crate) struct GatewayRequestLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationRequestPermit,
    global: GatewayGlobalRequestPermit,
    request_active: Arc<AtomicBool>,
    close_after_response: bool,
}
impl GatewayRequestLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot;
    pub(crate) const fn close_after_response(&self) -> bool;
}
pub(crate) enum GatewayAdmissionError {
    Unavailable,
    CertifiedKeyUnusable,
    ConnectionLimit,
    ConnectLimit,
    RequestLimit,
}
pub(crate) struct GatewayDrainTimeout {
    pub generation: GatewayApplicationGenerationId,
    pub connections: u32,
    pub requests: u32,
}
pub(crate) enum GatewayApplicationError {
    CertifiedKeyReferenceMismatch,
    CertifiedKeyHostnameMismatch,
    Frontend(ServiceFrontendResolveError),
    Demand(GatewayFrontendDemandError),
    Identity(GatewayIdentityLifecycleError),
    UpstreamProbe(GatewayUpstreamConnectError),
    Bind(std::io::ErrorKind),
    WatchGap,
    Superseded,
}
```

`GatewayShutdownReport::is_clean()` is exactly
`forced_connections == 0 && task_failures.is_empty() &&
cleanup_failures.is_empty() && application_errors.is_empty() &&
identity_error.is_none() && identity_empty && residual_connect.is_none()`.

`run_server_with_obs_and_drivers` is the sole caller of
`UnboundGatewayBuilder::{new,bind_demand_wake}`, constructs one
`GatewayLimits::first_slice` value, uses it for HostGatewayClientMtls and one
`GatewayRuntimeDependencies`, calls `GatewayBuilder::start`, puts
`GatewayControl` in `AppState`, and transfers `GatewayHandle` into
`ServerHandle`. `ServerHandle::shutdown` is the sole caller of
`GatewayHandle::shutdown`. Route POST/DELETE are the sole callers of
`GatewayControl::routes`; `GET /v1/gateway/status` is the sole caller of its two
status reads. No public `prepare_application`, `bind`, `publish`, `withdraw`,
task accessor or direct admission mutator exists.

Builder construction creates the Application command channel with capacity
two—exactly one shutdown close plus one retire command—and transfers its sole
sender into `GatewayShutdownCapabilities`; no handler or request owns it.
`GatewayApplicationOwner::run` is the only mutator of staged/current/draining,
listener control and application status. The listener task owns the socket,
the connections task owns its JoinSet, and only their narrow handles cross into
the owner/handle.

Both Application-handle methods put the absolute deadline on the command,
then race send/reply with the handle's injected
`clock.sleep(deadline.saturating_duration_since(clock.now()))`. Channel send or
reply closure is `OwnerUnavailable`; expiry is the phase-specific `Timeout`.
The owner checks the command deadline before beginning; every awaited demand
operation uses that same deadline, and all local state changes after it are
non-awaiting. A timed-out queued command therefore cannot execute later. For
`CloseForShutdown`, it first
cancels/drops the in-flight application-gate future, takes `staged`, and calls
`demand.remove_staged(generation, deadline, clock)` before closing all Current/
Draining retentions and clearing admission. It returns all retentions even when
staged removal failed, carrying the exact generation/error separately. The
GatewayHandle retries that same removal once before demand cancellation and
appends `GatewayApplicationCommandError::Demand` if it still fails. The three
`GatewayTaskTree`
join methods take each JoinHandle exactly once and register it before racing
the same remaining deadline; at expiry
it aborts every unfinished handle and awaits each abort. Inner `Err`, channel
closure, deadline abort, unexpected cancellation and `JoinError::is_panic()`
map respectively to `OwnerFailed`, `ChannelClosed`, `Timeout`, `Cancelled` and
`Panicked` with the exact `GatewayTaskName`; every failure is appended.

The existing control-plane owner changes exactly as follows; every other field
is retained:

```rust
pub struct ServerHandle {
    // existing fields unchanged
    gateway: Option<GatewayHandle>,
}
pub struct AbruptServerResidue {
    // existing fields unchanged
    pub gateway: Option<GatewayShutdownReport>,
}
pub enum ServerShutdownError {
    Mtls(MtlsInterceptOwnerShutdownError),
    Gateway(GatewayShutdownReport),
    Combined {
        mtls: MtlsInterceptOwnerShutdownError,
        gateway: GatewayShutdownReport,
    },
}
impl ServerShutdownError {
    pub fn teardown_failure(&self) -> Option<&MtlsInterceptOwnerShutdownError>;
    pub fn gateway_report(&self) -> Option<&GatewayShutdownReport>;
}
```

`ServerHandle::shutdown` retains its exact
`Result<(), ServerShutdownError>` signature. Existing private
`ServerShutdownError::new(mtls)` remains and constructs `Mtls`. A non-clean
gateway report constructs `Gateway`; both failures construct `Combined` after
all teardown attempts. Display is respectively `"server shutdown mTLS teardown
failed: {mtls}"`, `"server shutdown gateway teardown failed"`, or
`"server shutdown mTLS and gateway teardown failed: {mtls}"`; it never renders
credentials, intents or receipts. `Error::source()` returns the mTLS error for
`Mtls`/`Combined` and `None` for `Gateway`. `teardown_failure()` returns
`Some` for `Mtls`/`Combined`, `None` for `Gateway`; `gateway_report()` returns
`Some` for `Gateway`/`Combined`, `None` for `Mtls`.

The bounded migration is every current exhaustive match plus
`server_lifecycle.rs`'s direct accessor: replace
`err.teardown_failure()` with
`err.teardown_failure().expect("fixture induces mTLS teardown failure")`.
Generic `?`/Display callers remain unchanged. New gateway shutdown coverage
matches `Gateway`/`Combined` and asserts the exact report; no compatibility
error type, string flattening or overload is introduced.

`run_server_with_obs_and_drivers` constructs Enabled/Disabled demand and
identity compositions, registers the gateway reconciler when enabled, starts
the gateway task tree and installs `GatewayControl`/`GatewayHandle` before it
spawns convergence, emit-drain or interest-router tasks. This ordering makes
every first evaluation and emitted action see the final AppState ports. The
gateway tasks may wait for convergence, but cannot bind TCP/443 until their
gates complete. The `StartedGateway::into_parts` return is the only transfer
into AppState/ServerHandle; disabled configuration stores `gateway=None`.
The existing integration-only `abort_for_test` consumes the same handle,
invokes gateway shutdown with `Duration::ZERO`, and returns its typed report in
`AbruptServerResidue.gateway`; it cannot discard the new field or inject a
snapshot/identity/map.

`bind_demand_wake` transfers the builder's one `GatewayUpstreamSeal` in its
triple. Composition obtains `slot.client_access(clock.clone())` and passes
both plus `limits.clone()` to `HostGatewayClientMtls::new`. It then moves the
same limits value into `GatewayRuntimeDependencies::new(limits, dataplane,
client_mtls, identity_lifecycle)`; no second policy constructor call occurs.

`GatewayApplicationGenerationId::from_inputs` is called only by
`GatewayApplicationOwner` after resolving all three inputs; status and demand
consume `as_bytes`. `GatewayAppliedGeneration` is constructed only at
promotion and consumed by the admission snapshot plus application-status row.
The private candidate owns the exact Route, resolved Service Frontend and
opaque Public Certified-Key snapshot; the private admission snapshot adds the
Applied Generation and one Arc-owned `GatewayGenerationRetention`.
Both are created only in the owner turn. This is the single atomic binding
required by the Domain model.

`GatewayApplicationCandidate::new` requires the Route's certified-key ID and
hostname equal the supplied snapshot before computing its generation. On
promotion the owner consumes the exact applied demand ticket, constructs one
snapshot, and swaps its `Arc` into `GatewayAdmission.current` in one ArcSwap
operation. `acquire_connection` loads exactly one Arc, checks
`certified_key.is_usable_at(now)`, acquires one process-wide public-connection
permit plus that generation's retention count, and returns a lease retaining
that same snapshot. TLS, every keep-alive request and Route/
frontend/key access use only the retained lease; no request re-resolves desired
state. The non-`Clone` connection lease alone exposes `try_request`; it enforces
one in-flight HTTP/1.1 request per connection, the per-connection request count
and the process-wide request ceiling before incrementing the generation request
count. The hundredth permit sets `close_after_response=true`; a later call is
`RequestLimit`.
Neither lease implements `Clone`, `Default` or a public constructor.

`GatewayBuilder::start` creates exactly one `Arc<GatewayAdmissionBudget>` from
the one limits value and passes it to admission, every generation snapshot and
the connector. It also creates one bounded `Arc<GatewayCleanupLedger>` and
passes the same Arc to `GatewayUpstreamConnector::new` and
`GatewayShutdownCapabilities`; `GatewayApplicationOwner` reaches it only
through its connector. Its three permit acquisitions use checked CAS against
`max_public_connections`, `max_inflight_connects` and
`max_inflight_requests`; every private non-Clone permit Drop decrements exactly
once. `GatewayUpstreamConnector::{probe,connect}` acquires a connect permit
before socket creation and holds it through receipt consumption/mTLS. Thus
Current and every Draining generation share the same process-wide counts while
their retention counters remain independent.

The ledger is constructed only with
`limits.max_cleanup_ledger_entries() == 128`. After socket-cookie acquisition
and before BPF registration, connector `reserve(intent)` inserts one Active
record. A full ledger increments the saturating `saturation_count`, returns
`GatewayCleanupLedgerFull`, and the connector closes without registering the
socket; no cleanup intent is lost. The reservation marks exact intent/receipt
failure bits, and `complete` removes the record only after both keys are known
clean. `retry_all` snapshots records under its mutex, drops the guard, calls
Dataplane cleanup for each exact intent, then reacquires the guard to remove
only successful records; a failed record remains for the next sweep. Its
return groups retained failure bits into ConnectIntent/Receipt counts and emits
`LedgerSaturated` whenever the historical saturation counter is nonzero, so
saturation is typed/non-clean even if later cleanup succeeds. Demand retirement failure goes to
`application_errors`; gateway identity drop failure goes to `identity_error`.
Neither is a Dataplane-ledger entry.
`try_request` CASes `request_active false→true`, checks/increments
`requests_started`, then acquires global and generation permits; any later
failure rolls back the earlier changes. RequestLease Drop decrements both
counts and resets `request_active=false` before notifying the retention owner.

Before a Current generation becomes Draining or is cleared, the owner calls
`retention.close()` and then swaps/clears the admission pointer. Both
`try_connection` and `try_request` use a checked CAS, test `closed` before and
after increment, and roll back the count if close raced them; therefore a
retained keep-alive connection cannot begin another request after Draining.
Lease Drop decrements exactly one matching count and calls
`owner_wake.notify_one`. `wait_zero` checks both counts, registers `notified()`
before rechecking to prevent a lost wake, and races that future with
`clock.sleep(deadline.saturating_duration_since(clock.now()))`; at the absolute
deadline it returns `GatewayDrainTimeout` with the exact remaining counts. The
owner alone waits and then retires demand; no per-lease event queue exists.

`GatewayAppliedGeneration::from_promotion` is called only after the exact
demand ticket is acknowledged and only inside `GatewayApplicationOwner`;
external crates can read but cannot forge Applied authority.

The Application-generation hash is exactly SHA-256 over
`b"overdrive/gateway/application-generation/v1\0" ||
route_generation.as_bytes() || frontend.vip_v4().octets() ||
frontend.port().get().to_be_bytes() || [frontend.proto().as_u8()] ||
certified_key_generation.as_bytes()`. IPv4 octets are network order and the
port's two bytes are big-endian. It contains no
demand revision, BPF generation, BackendId, receipt or gateway-identity fact.

`GatewayDemandComposition::Disabled` is constructed by the control-plane only
when gateway config is absent. Its `read` is the canonical empty snapshot;
`dispatch_ports` returns `None`. The Enabled value is returned only by
`bind_demand_wake`; every hydration builder takes `read()`, and every
action/workflow dispatch wrapper takes the same `dispatch_ports()` value. No
optional raw read/ack/wake parameters remain at those call sites.

`disabled()` stores one `Arc<EmptyGatewayFrontendDemand>` whose every snapshot
is exactly `{ revision: GatewayFrontendDemandRevision::first(), entries: [] }`.
`enabled` stores the same read Arc in both the hydration-facing composition and
`GatewayDemandDispatchPorts`, so the pre-effect guard and hydration can never
observe different demand owners. There is no public constructor for a partial
Enabled bundle.

`GatewayAppStateComposition::disabled` creates one Arc demand composition plus
one `DisabledGatewayIdentityRead::canonical()` Arc used for both identity trait
fields, `GatewayIdentityActionComposition::disabled()`, and `control=None`.
The disabled desired value is exactly epoch one/`spiffe_id=None`; current is
always None. Production enabled composition stores one Arc demand composition,
the same `Arc<GatewayIdentitySlot>` behind both read traits, the Enabled action
target, and the live control handle. Because the outer composition clones only
Arcs/handles, existing `AppState: Clone` remains valid and every HydrationContext
borrow points into an AppState-owned Arc.

The broad fixture `AppState::new` keeps its exact signature and delegates with
`GatewayAppStateComposition::disabled()`. The full
`new_with_workflow_engine` appends the mandatory `gateway` parameter after
`frontend_addr_allocator`; production supplies Enabled, while every existing
non-gateway caller and direct `AppState { .. }` literal adds exactly
`gateway: GatewayAppStateComposition::disabled()`. Only gateway-focused Sim/
integration composition may pass Enabled. `build_hydration_context`, both
workflow-intent dispatch forms, the workflow emit loop and Route/status
handlers read exclusively through `state.gateway` accessors; no parallel
AppState demand/identity/control field exists.

One active `GatewayApplicationOwner` holds at most one staged candidate, one
ArcSwap Current generation and N draining generations with retained connection
counts. The application ID hashes only Route Generation, exact frontend and
Certified Key Generation; Applied wraps it only at the atomic Current swap.
Demand revision, BPF generation, BackendId, receipt and SPIFFE material never
enter the object.

The owner stage/ack/promote order is load-bearing. It publishes exact frontend
as Staged and computes one absolute
`gate_deadline = clock.now() + limits.application_gate_deadline()`. It wakes
ServiceMapHydrator, awaits the exact revision/ServiceKey ack against that
deadline, revalidates Route/key/frontend, ensures current gateway identity
against the same deadline, drives the same connector through one receipt/mTLS
probe with that deadline, rechecks identity remains
Current, swaps Current, moves the
prior Current to Draining, then binds TCP/443 if needed and enables accepts. A failed initial
bind accepts nothing, removes the just-promoted Current and retires its demand
through the zero-connection Draining phase before publishing a typed failure.
On an already-bound replacement, the
existing listener remains owned while the Current pointer swaps.
Any application-gate timeout removes only the staged demand/candidate. With no
prior Current the listener remains unbound; on replacement the prior Current
and listener remain unchanged. The same absolute deadline is never restarted
by a watch wake or retry.
Withdraw/unresolved/key or
identity loss stops accepts/unbinds before clearing Current; draining demand
remains until the last retained connection closes. Late result/ack cannot
restore a superseded generation. Intent/custody/identity watch recovery is
subscribe-before-list and fail-closed.

Every demand-dependent update carries its exact revision/ServiceKey. Before
any Dataplane call, the action shim re-reads current demand; a superseded guard
returns `DispatchOutcome::StaleGatewayDemand`, writes no hydration row/map
effect and re-wakes the latest target. That target is exactly
`ServiceId::derive(vip, port, proto, "service-map")`, computed from
`ServiceFrontend` and never stored in Route/demand. Only an authorized
successful update emits the exact-revision acknowledgment.
Revisions start at nonzero one on each process, advance only with checked
addition, return `RevisionExhausted` without changing state, and are never
compared across restart.

### Exact persisted status and operator response

The two new observation rows are closed, non-secret LWW projections. They do
not serialize owner error types or opaque capabilities:

```rust
pub enum PublicCertifiedKeyStatusState {
    Absent,
    Usable {
        generation: CertifiedKeyGeneration,
        certificate_fingerprint: CertificateFingerprint,
        public_hostname: PublicHostname,
        not_before: UnixInstant,
        not_after: UnixInstant,
        provenance: CertifiedKeyProvenance,
    },
    Unusable {
        generation: CertifiedKeyGeneration,
        cause: CertifiedKeyUsabilityFailure,
    },
}
pub struct PublicCertifiedKeyStatusRowV1 {
    pub certified_key_id: PublicCertifiedKeyId,
    pub state: PublicCertifiedKeyStatusState,
    pub last_install_failure: Option<CertifiedKeyFailure>,
    pub updated_at: LogicalTimestamp,
}
pub enum PublicCertifiedKeyStatusRowEnvelope {
    V1(PublicCertifiedKeyStatusRowV1),
}

pub enum GatewayListenerStatus {
    Unbound,
    Bound { address: SocketAddrV4 },
}
pub struct GatewayApplicationGenerationStatus {
    pub application_generation: GatewayApplicationGenerationId,
    pub route_id: RouteId,
    pub route_generation: RouteGeneration,
    pub service_key: ServiceKey,
    pub demand_revision: GatewayFrontendDemandRevision,
}
pub enum GatewayApplicationUnavailableCause {
    RouteAbsent,
    RouteUnreadable,
    CertifiedKeyAbsent,
    CertifiedKeyUnusable,
    FrontendUnresolved,
    DemandNotApplied,
    GatewayIdentityUnusable,
    ConnectPathUnavailable,
    ListenerBindFailed,
    WatchGap,
}
pub enum GatewayIdentityStatus {
    Absent,
    Current {
        epoch: GatewayIdentityEpoch,
        spiffe_id: SpiffeId,
        serial: CertSerial,
        not_after: UnixInstant,
    },
    Unusable,
}
pub enum GatewayConnectPathStatus { Unavailable, Ready }
pub struct GatewayApplicationStatusRowV1 {
    pub node_id: NodeId,
    pub listener: GatewayListenerStatus,
    pub staged: Option<GatewayApplicationGenerationStatus>,
    pub current: Option<GatewayApplicationGenerationStatus>,
    pub draining: Vec<GatewayApplicationGenerationStatus>,
    pub unavailable: Option<GatewayApplicationUnavailableCause>,
    pub gateway_identity: GatewayIdentityStatus,
    pub connect_path: GatewayConnectPathStatus,
    pub updated_at: LogicalTimestamp,
}
pub enum GatewayApplicationStatusRowEnvelope {
    V1(GatewayApplicationStatusRowV1),
}

// Exact additions to the existing closed observation surface.
ObservationWrite::PublicCertifiedKeyStatus(PublicCertifiedKeyStatusRowV1)
ObservationWrite::GatewayApplicationStatus(GatewayApplicationStatusRowV1)
ObservationRow::PublicCertifiedKeyStatus(PublicCertifiedKeyStatusRowV1)
ObservationRow::GatewayApplicationStatus(GatewayApplicationStatusRowV1)
ObservationRowKind::PublicCertifiedKeyStatus
ObservationRowKind::GatewayApplicationStatus

#[async_trait::async_trait]
pub trait ObservationStore: Send + Sync + 'static {
    // Existing methods remain exact; these two point reads are added.
    async fn public_certified_key_status_row(
        &self,
        id: &PublicCertifiedKeyId,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, ObservationStoreError>;
    async fn gateway_application_status_row(
        &self,
        node_id: &NodeId,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, ObservationStoreError>;
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct GatewayStatusResponse {
    pub enabled: bool,
    #[schema(value_type = String)]
    pub configured_address: Option<SocketAddrV4>,
    pub listener_bound: bool,
    pub application: Option<GatewayApplicationStatusBody>,
    pub certified_key: Option<PublicCertifiedKeyStatusBody>,
    pub hydration: Vec<GatewayFrontendHydrationStatusBody>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct GatewayApplicationStatusBody {
    pub listener: GatewayListenerStatusBody,
    pub staged: Option<GatewayApplicationGenerationStatusBody>,
    pub current: Option<GatewayApplicationGenerationStatusBody>,
    pub draining: Vec<GatewayApplicationGenerationStatusBody>,
    pub unavailable: Option<GatewayApplicationUnavailableCauseBody>,
    pub gateway_identity: GatewayIdentityStatusBody,
    pub connect_path: GatewayConnectPathStatusBody,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PublicCertifiedKeyStatusBody {
    pub id: String,
    pub state: PublicCertifiedKeyStateBody,
    pub last_install_failure: Option<CertifiedKeyFailureBody>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct GatewayApplicationGenerationStatusBody {
    pub application_generation: String,
    pub route_id: String,
    pub route_generation: String,
    pub service_vip: String,
    pub service_port: u16,
    pub service_protocol: String,
    pub demand_revision: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct GatewayFrontendHydrationStatusBody {
    pub service_vip: String,
    pub service_port: u16,
    pub service_protocol: String,
    pub demand_revision: u64,
    pub state: GatewayFrontendHydrationStateBody,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GatewayListenerStatusBody { Unbound, Bound { address: String } }
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayApplicationUnavailableCauseBody {
    RouteAbsent, RouteUnreadable, CertifiedKeyAbsent, CertifiedKeyUnusable,
    FrontendUnresolved, DemandNotApplied, GatewayIdentityUnusable,
    ConnectPathUnavailable, ListenerBindFailed, WatchGap,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GatewayIdentityStatusBody {
    Absent,
    Current {
        epoch: u64,
        spiffe_id: String,
        serial: String,
        #[schema(value_type = String)]
        not_after: UnixInstant,
    },
    Unusable,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayConnectPathStatusBody { Unavailable, Ready }
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PublicCertifiedKeyStateBody {
    Absent,
    Usable {
        generation: String,
        certificate_fingerprint: String,
        hostname: String,
        #[schema(value_type = String)]
        not_before: UnixInstant,
        #[schema(value_type = String)]
        not_after: UnixInstant,
        provenance: CertifiedKeyProvenanceBody,
    },
    Unusable { generation: String, cause: CertifiedKeyUsabilityFailureBody },
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "producer", rename_all = "snake_case")]
pub enum CertifiedKeyProvenanceBody { Manual, Workflow { correlation: String } }
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CertifiedKeyUsabilityFailureBody { NotYetValid, Expired }
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CertifiedKeyFailureBody {
    SourceOpen, SourceMetadata, SourceOwnership, SourcePermissions,
    SourceBounds, PemShape, CertificateProfile, HostnameMismatch,
    KeyMismatch, Protection, Persistence, RustlsConfiguration,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayFrontendHydrationStateBody { Pending, Completed, Failed }
```

Both row structs/envelopes and their closed enums derive the same rkyv/value
traits as existing observation rows; none derives wire serialization. Both
new `ObservationWrite` variants participate in the existing LWW acceptance,
subscription and `ObservationRowKind` dispatch arms. LocalStore and Sim add
direct point indexes at table `public_certified_key_status` keyed by canonical
certified-key ID bytes and table `gateway_application_status` keyed by
canonical NodeId bytes. `PublicCertifiedKeyCustody` is the only writer/caller
of the former variant; `GatewayApplicationOwner` is the only writer/caller of
the latter. `GatewayControl` and `GET /v1/gateway/status` are the only callers
of the two new reads. The response `*Body` enums are exhaustive projections
with snake-case tagged wire labels; the domain enums are never serialized
directly.

Application status is written only after the corresponding owner transition
(stage, promotion, bind/unbind, begin-drain or retire). An observation failure
does not roll back admission, demand or listener state or become desired
truth; the owner retains the newest row value and retries it on its next
input/wake. The operator may therefore see the last accepted LWW value, never
a pre-transition promise.

`GET /v1/gateway/status` exposes enabled/configured bind/listener, active
application row, custody, non-secret Gateway Identity Current, connect-path
state and separate staged/current/draining-demand Service hydration attempts. It contains no
raw dataplane failure string, receipt or unified BPF/gateway generation.
`GatewayConnectPathStatus::Ready` means the registered hook/receipt contract is
trusted; it includes `GatewayConnectProbeOutcome::ReadyNoBackend` and never
claims a selectable backend. Backend absence remains observable only on the
request as 503 and in the independent Service hydration projection.
`GatewayControl::application_status` and
`GatewayControl::certified_key_status` are the real redacted, ID-scoped
ObservationStore point reads used by this handler; read failure is 500 and no
status DTO can reach an opaque key/identity-use snapshot.
Dedicated `*Body` enums project every application/connect/custody failure and
provenance variant; domain enums are not serialized directly. All time fields
remain typed `UnixInstant` using its existing quoted seconds+nanos JSON form.
`configured_address` retains `SocketAddrV4`'s canonical `a.b.c.d:443` string
serde form and declares `#[schema(value_type = String)]`; every DTO
`UnixInstant` field carries the same String schema override.

Public TLS is TLS1.3/exact SNI/ALPN `http/1.1`, no default cert/0-RTT/tickets.
Hyper solely parses framing. Only origin-form request targets are accepted;
CONNECT plus authority-, absolute- and asterisk-form are 400, so the gateway
cannot become a tunnel or forward proxy. Exactly one syntactically valid Host
is required. An explicit port is accepted only when it is 443 and is removed
for Route comparison; missing/malformed/duplicate Host or another port is 400.
The lowercase Host name must equal the exact SNI or the response is 421. The
original raw path (never percent-decoded) drives exact/segment-prefix matching;
query is preserved upstream but does not participate in Route match. Miss is
404.

On both request and response, the runtime removes `Connection` and every header
named by its comma-token list plus `Keep-Alive`, `Proxy-Authenticate`,
`Proxy-Authorization`, `TE`, `Trailer`, `Transfer-Encoding` and `Upgrade`;
Hyper regenerates legal framing. Trailers are consumed and not forwarded. The
upstream `Host` remains the validated public hostname. The request path strips
all received `Forwarded` and `X-Forwarded-*`, emits no `X-Forwarded-*`, appends
`Via: 1.1 overdrive`, and adds exactly
`Forwarded: for=<accepted IPv4>;proto=https;host="<validated hostname>"`.
The response appends the same Via product token. All remaining end-to-end
headers, status and raw body octets stream with backpressure.

The input-sensitive routing/header logic is production code with pure,
source-local property entry points; no test-only facade is introduced. The
exact ownership and signatures are:

```rust
// overdrive-core::public_ingress
impl PathMatch {
    #[must_use]
    pub fn matches_raw_path(&self, raw_request_path: &str) -> bool;
}

// overdrive-gateway::runtime::routing
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RouteAuthorityError {
    #[error("Host header is missing")]
    MissingHost,
    #[error("Host header occurs more than once")]
    DuplicateHost,
    #[error("Host header is not visible ASCII")]
    NonAsciiHost,
    #[error("Host header is not a valid HTTP authority")]
    InvalidAuthority,
    #[error("Host authority port must be 443, got {got}")]
    UnsupportedPort { got: u16 },
    #[error("Host name is invalid: {0}")]
    InvalidHostname(PublicHostnameError),
    #[error("Host does not equal the TLS SNI")]
    SniMismatch,
}
pub(crate) fn normalize_route_authority(
    headers: &HeaderMap,
    sni: &PublicHostname,
) -> Result<PublicHostname, RouteAuthorityError>;

// overdrive-gateway::runtime::proxy_headers
#[must_use]
pub(crate) fn rewrite_request_headers(
    headers: HeaderMap,
    accepted_client: Ipv4Addr,
    authority: &PublicHostname,
) -> HeaderMap;
#[must_use]
pub(crate) fn rewrite_response_headers(headers: HeaderMap) -> HeaderMap;
```

The owning files are
`crates/overdrive-core/src/public_ingress.rs`,
`crates/overdrive-gateway/src/runtime/routing.rs` and
`crates/overdrive-gateway/src/runtime/proxy_headers.rs`. Their property modules
remain source-local `#[cfg(test)]` modules in those same files; no integration
test visibility widening is permitted.

The approved Route-path value is `PublicPath` inside `PathMatch`; no second
`RoutePath` alias/type is created. `PathMatch::matches_raw_path` consumes the
exact `Uri::path()` bytes rendered as `&str`; the query is never passed.
`Exact(p)` is true only for byte equality.
`SegmentPrefix(p)` is true when the request path equals `p`, when `p == "/"`
and the request is absolute, when `p` ends in `/` and the request begins with
`p`, or when the request begins with `p` and the next byte is `/`; `/api`
therefore matches `/api` and `/api/v1`, never `/apix`, while `/api/` matches
`/api/` and `/api/v1`.
Neither operand is percent-decoded, Unicode-normalized or slash-normalized.

`normalize_route_authority` reads `HOST` with `HeaderMap::get_all` and requires
exactly one value. It rejects non-visible-ASCII bytes, parses the value with
`hyper::http::uri::Authority`, rejects userinfo (`@`), permits no port or
explicit port 443 only,
lowercases the host with ASCII case-folding, then calls
`PublicHostname::new`. The resulting canonical hostname must equal the typed
TLS SNI. `MissingHost`, `DuplicateHost`, `NonAsciiHost`, `InvalidAuthority`,
`UnsupportedPort` and `InvalidHostname` map to the already-selected public
400; `SniMismatch` alone maps to 421. The runtime calls this function before
hostname/path Route selection; it then calls `matches_raw_path` on that Route's
`PathMatch`.

Both header functions take ownership and return a transformed map without
I/O, clock, global state or hidden input. They remove `Connection`, all valid
case-insensitive header names nominated by every comma-separated Connection
value, and the fixed hop-by-hop set already stated above. A malformed
Connection token names no header, while `Connection` itself is always removed.
The request function additionally removes the prior `Host`, all `Forwarded`
values and every field whose case-insensitive name starts `x-forwarded-`, then
inserts exactly one canonical `Host`, appends `Via: 1.1 overdrive`, and inserts
the one approved `Forwarded` value. The response function appends the same Via
value after hop-by-hop removal. Existing Via values are preserved in order
unless `Connection` explicitly nominated `Via`; the Overdrive value is always
last. All nonremoved field names and each field's value sequence are preserved.
Hyper alone regenerates framing after these functions return.

`overdrive-gateway::runtime::limits::GatewayLimits::first_slice()` is the sole
host-side limit SSOT and returns exactly:

```rust
#[derive(Debug, Clone)]
pub struct GatewayLimits {
    max_public_connections: NonZeroU32,
    max_inflight_connects: NonZeroU32,
    max_inflight_requests: NonZeroU32,
    max_cleanup_ledger_entries: NonZeroU32,
    max_request_head_bytes: NonZeroUsize,
    max_request_headers: NonZeroUsize,
    max_response_head_bytes: NonZeroUsize,
    max_response_headers: NonZeroUsize,
    max_requests_per_connection: NonZeroU32,
    max_request_body_bytes: NonZeroU64,
    max_response_body_bytes: NonZeroU64,
    application_gate_deadline: Duration,
    tls_handshake_deadline: Duration,
    request_head_deadline: Duration,
    upstream_connect_deadline: Duration,
    response_head_deadline: Duration,
    body_no_progress_deadline: Duration,
    keep_alive_idle_deadline: Duration,
    whole_request_deadline: Duration,
}
impl GatewayLimits {
    pub fn first_slice() -> Self;
    pub const fn max_public_connections(&self) -> NonZeroU32;
    pub const fn max_inflight_connects(&self) -> NonZeroU32;
    pub const fn max_inflight_requests(&self) -> NonZeroU32;
    pub const fn max_cleanup_ledger_entries(&self) -> NonZeroU32;
    pub const fn max_request_head_bytes(&self) -> NonZeroUsize;
    pub const fn max_request_headers(&self) -> NonZeroUsize;
    pub const fn max_response_head_bytes(&self) -> NonZeroUsize;
    pub const fn max_response_headers(&self) -> NonZeroUsize;
    pub const fn max_requests_per_connection(&self) -> NonZeroU32;
    pub const fn max_request_body_bytes(&self) -> NonZeroU64;
    pub const fn max_response_body_bytes(&self) -> NonZeroU64;
    pub const fn application_gate_deadline(&self) -> Duration;
    pub const fn tls_handshake_deadline(&self) -> Duration;
    pub const fn request_head_deadline(&self) -> Duration;
    pub const fn upstream_connect_deadline(&self) -> Duration;
    pub const fn response_head_deadline(&self) -> Duration;
    pub const fn body_no_progress_deadline(&self) -> Duration;
    pub const fn keep_alive_idle_deadline(&self) -> Duration;
    pub const fn whole_request_deadline(&self) -> Duration;
}
```

| Ceiling | Exact first-slice value | Basis and sensitivity |
|---|---:|---|
| Public connections | 128 | Reuses the accepted pre-authentication concurrency class. At two socket legs this caps the normal gateway tree at 256 fds, below the existing 384-fd envelope for 128 three-leg mTLS intercepts. Lower rejects ordinary bursts sooner; higher increases unauthenticated TLS CPU/fds. |
| In-flight connects / requests | 128 / 128 | Connect capacity equals both BPF transient-map capacities; HTTP/1.1 has at most one active request per connection. Lower wastes admitted sockets; higher cannot be represented by the maps. |
| Cleanup-ledger entries | 128 | One slot is reserved per admitted upstream connect before BPF registration, matching connect/map capacity. Lower can refuse a connect while map capacity remains; higher cannot represent additional simultaneously dirty cookies and retains more failed cleanup state. |
| Request/response head bytes | 32 KiB / 32 KiB | Four times Hyper 1.9's documented 8 KiB minimum; across 128 active connections, the conservative two-head upper envelope is 8 MiB. Lower breaks moderately large cookies/metadata; higher expands slowloris/parser memory. |
| Request/response header count | 100 / 100 | Uses Hyper 1.9's existing default rather than inventing a divergent parser policy. Lower rejects interoperable clients; higher increases per-field allocation/work. |
| Requests per keep-alive connection | 100 | Bounds how long one retained Gateway Application generation can be extended. Lower increases reconnect/TLS cost; higher delays Route replacement or key-expiry drain. |
| Request/response body bytes | 16 MiB / 16 MiB | Deliberate first-slice product ceiling, not a measured KPI; streaming makes it bandwidth/abuse containment rather than a whole-body allocation. Lower excludes larger legitimate streams; higher increases per-request occupancy and abuse duration. |
| Application gate | 30 s | Reuses the accepted no-progress recovery window to bound demand apply, gateway identity Current and the pre-bind connect receipt/mTLS probe as one absolute attempt. Lower false-times normal convergence/issuance; higher retains a stale staged generation longer. |
| Public TLS / request head / upstream connect / response head | 5 s each | TLS reuses the accepted `MtlsLimits::handshake_deadline`; the other pre-header phases share that dead-peer/normal-scheduling tolerance. Lower false-times slow legitimate peers; higher amplifies slowloris/dead-backend occupancy. |
| Body no-progress / keep-alive idle | 30 s / 30 s | Reuses the accepted no-progress recovery window and applies it only when bytes are pending or a keep-alive is idle. Lower harms slow streams; higher retains dead sockets/generations. |
| Whole request | 120 s | Matches the existing bounded CLI/control-plane long-request envelope while remaining greater than every phase deadline. Lower truncates legitimate streamed responses; higher delays reclamation. |

The BPF crate mirrors `GATEWAY_MAX_INFLIGHT_CONNECTS = 128` because it cannot
link the std host crate; a host ABI test asserts both BPF map `max_entries`
values equal `GatewayLimits::first_slice().max_inflight_connects()`. No CLI,
environment, config, status or operator knob exposes these values.

`GatewayLimits::first_slice` is the sole constructor. `GatewayBuilder::start`
receives the one value constructed by `run_server_with_obs_and_drivers`; the public runtime and
`HostGatewayClientMtls` consume only its accessors, and the BPF ABI assertion
consumes `max_inflight_connects`. There is no public field, builder, mutation
method or deserialization path, so exposing the fixed policy across the
gateway→dataplane adapter boundary does not create an operator setting.

Both Hyper server and client builders receive their respective
`max_buf_size`/`max_headers`. Excess public connection closes before TLS;
public TLS timeout closes without HTTP; request-head overflow is 431;
request-body overflow is 413; public request-head/body no-progress timeout
before response headers is 408. Global request/connect permit refusal and
upstream connect/TLS/response-head/whole-request failures are 502 before public
headers. NoBackend
alone is 503. Response overflow or any failure after headers closes both legs.
The hundredth response carries `Connection: close`; a 101st request is never
parsed. Non-HTTP/1.1 is 505. No 504/retry/replay/alternate backend exists.

## Wave: DESIGN / [REF] Gateway identity and lifecycle integration

### Exact identity, connector and authenticated-stream ports

The cross-crate surface is deliberately capability-shaped. Reconciliation and
status can read only non-secret facts; only the Dataplane-owned exact-peer TLS
adapter can borrow held private material, and it returns an authenticated byte
stream rather than material:

```rust
// overdrive-core::gateway_identity
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayIdentityFacts {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: SpiffeId,
    pub serial: CertSerial,
    pub not_after: UnixInstant,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayIdentityDesired {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: Option<SpiffeId>,
}
pub trait GatewayIdentityDesiredRead: Send + Sync {
    fn desired(&self) -> GatewayIdentityDesired;
}
pub trait GatewayIdentityCurrentRead: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
}

// Exact additions to overdrive-core::reconcilers::Action.
Action::IssueGatewaySvid {
    epoch: GatewayIdentityEpoch,
    spiffe_id: SpiffeId,
    node_id: NodeId,
    correlation: CorrelationKey,
}
Action::DropGatewaySvid {
    epoch: GatewayIdentityEpoch,
    correlation: CorrelationKey,
}

// overdrive-gateway::ports; implemented by the control-plane composition.
#[async_trait::async_trait]
pub trait GatewayIdentityLifecycleControl: Send + Sync {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError>;
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    async fn disable_after_drain(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError>;
}
pub enum GatewayIdentityWaitPhase { EnsureCurrent, DisableAfterDrain }
pub enum GatewayIdentityLifecycleError {
    ReconcilerUnavailable,
    EpochExhausted,
    IssueFailed,
    DropFailed,
    Timeout { phase: GatewayIdentityWaitPhase },
    Closed,
}
pub trait GatewayIdentityLifecycleTarget: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
}

pub trait GatewayByteStream:
    AsyncRead + AsyncWrite + Unpin + Send + 'static
{}
impl<T> GatewayByteStream for T where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static
{}
pub struct GatewayUpstreamSeal { _private: () }
pub struct GatewayUpstream {
    io: Pin<Box<dyn GatewayByteStream>>,
}
impl GatewayUpstreamSeal {
    pub fn seal_authenticated(
        &self,
        io: Pin<Box<dyn GatewayByteStream>>,
    ) -> GatewayUpstream;
}
// GatewayUpstream implements AsyncRead + AsyncWrite by delegating to `io`;
// it has no public constructor, inner-stream accessor, Clone or conversion.

#[async_trait::async_trait]
pub trait GatewayClientMtls: Send + Sync {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError>;
}
pub enum GatewayClientMtlsError {
    IdentityAbsent,
    IdentityExpired,
    PeerAddress,
    HandshakeTimeout,
    Handshake,
    PeerCertificateMissing,
    PeerSpiffeShape,
    PeerSpiffeMismatch { expected: SpiffeId, actual: SpiffeId },
}

// overdrive-sim
pub struct SimGatewayClientMtls { /* scripted peer + one seal */ }
impl SimGatewayClientMtls {
    pub fn new(seal: GatewayUpstreamSeal, clock: Arc<dyn Clock>) -> Self;
    pub fn set_peer(&self, peer: SpiffeId);
}
// Its GatewayClientMtls implementation returns GatewayUpstream only after the
// scripted peer equals expected_peer; otherwise the exact mismatch error.

// overdrive-gateway::connector
pub(crate) struct GatewayUpstreamConnector { /* private collaborators */ }
impl GatewayUpstreamConnector {
    pub(crate) fn new(
        dataplane: Arc<dyn GatewayConnectDataplane>,
        cookie_reader: Arc<SocketCookieReader>,
        client_mtls: Arc<dyn GatewayClientMtls>,
        limits: GatewayLimits,
        clock: Arc<dyn Clock>,
        budget: Arc<GatewayAdmissionBudget>,
        cleanup_ledger: Arc<GatewayCleanupLedger>,
    ) -> Self;
    pub(crate) async fn probe(
        &self,
        frontend: ServiceFrontend,
        deadline: Instant,
    ) -> Result<GatewayConnectProbeOutcome, GatewayUpstreamConnectError>;
    pub(crate) async fn connect(
        &self,
        frontend: ServiceFrontend,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayUpstreamConnectError>;
}
pub(crate) enum GatewayConnectProbeOutcome {
    ReadyNoBackend,
    SelectedPeerAuthenticated,
}
pub(crate) enum GatewayUpstreamConnectError {
    Capacity(GatewayAdmissionError),
    CleanupLedgerFull { capacity: NonZeroU32 },
    SocketCreate,
    Cookie(SocketCookieReadError),
    Register(GatewayConnectDataplaneError),
    ConnectTimeout,
    Connect,
    Receipt(GatewayConnectDataplaneError),
    ReceiptMismatch,
    NoBackend,
    SelectedIdentity(GatewayConnectDataplaneError),
    Mtls(GatewayClientMtlsError),
}

// overdrive-host::socket_cookie
#[derive(Debug, Default)]
pub struct SocketCookieReader;
impl SocketCookieReader {
    pub const fn new() -> Self;
    pub fn probe(&self) -> Result<(), SocketCookieReadError>;
    pub fn read(
        &self,
        socket: BorrowedFd<'_>,
    ) -> Result<SocketCookie, SocketCookieReadError>;
}
pub enum SocketCookieReadError {
    Unsupported,
    Syscall { source: std::io::Error },
    Zero,
}
```

`GatewayIdentityCurrentRead` intentionally has no subscription method: its
only live callers are `GatewaySvidLifecycle::hydrate_actual` and the redacted
status projection. The active `GatewayApplicationOwner` is the sole caller of
`GatewayIdentityLifecycleControl::{ensure_current,subscribe,
disable_after_drain}`; the owner needs those commands and wake stream, whereas
the read-only reconciler/status port does not. `GatewayUpstreamConnector::new`
is called once by `GatewayBuilder::start`; the HTTP request path is the sole
caller of `connect`. `SocketCookieReader::{new,probe}` are called by `serve`
boot and both the connector and `EbpfDataplane` retain that same `Arc`;
`read` is called for each created upstream socket and by the Dataplane's
real-cgroup TEACH probe.

`GatewayUpstreamConnector::connect` creates one IPv4 `TcpSocket`, reads its
nonzero cookie, registers `(cookie, ServiceKey::from_frontend(frontend))`, and
connects to that exact Service Frontend. It races connect against
`min(deadline, clock.now() + limits.upstream_connect_deadline())`; the Host mTLS
adapter similarly races its handshake against
`min(deadline, clock.now() + limits.tls_handshake_deadline())`. The caller's
absolute deadline is never extended by moving between phases. The
ancestor-attached cgroup program
alone selects and rewrites. The connector then takes the matching receipt,
maps `NoBackend` without opening TLS, resolves `Selected.backend_id` through
`selected_backend_identity`, and transfers the connected fd plus only that
`SpiffeId` into `GatewayClientMtls::authenticate`. One private RAII guard calls
`cleanup` on every result and owns the exact pre-registered cleanup reservation
until cleanup succeeds. It marks intent/receipt failure on that reservation
before it drops; it never returns an unrecorded cleanup failure. The handle
later retries that same intent-bearing record before projecting only
closed-category counts into its
report. The connector never enumerates, chooses, retries or replays a
backend.

`GatewayApplicationOwner` calls crate-private `probe(frontend, deadline)` after
exact demand acknowledgment and gateway identity currentness but before
promotion/bind. Probe and request `connect` share one private socket/register/
connect/receipt/identity function. A canonical `NoBackend` receipt is
`ReadyNoBackend` and proves the bind capability without mTLS; it is not a boot
error. `Selected` must resolve identity, complete exact-peer mTLS and close the
authenticated stream without HTTP before returning
`SelectedPeerAuthenticated`. Request-time `connect` instead maps canonical
`NoBackend` to its closed error so the runtime returns 503. Missing/malformed
receipt remains an error on both paths. No separate transport implementation
or test-only input exists.

For a public request, the retained connection owner computes one
`request_deadline = clock.now() + limits.whole_request_deadline()` and threads
it through request-head/body, connector, mTLS, response-head/body and response
streaming. Every phase uses the minimum of that absolute deadline and its own
fixed limit. Demand wait, identity wait, connector probe and request connect
therefore have one owner-supplied absolute deadline and one injected Clock;
none discovers a Tokio timer or starts a fresh whole-operation budget.

The Dataplane-local holder and client constructor are:

```rust
// overdrive-dataplane::mtls::gateway_identity
pub struct GatewayIdentitySlot { /* private desired + held state */ }
impl GatewayIdentitySlot {
    pub fn new(node_id: NodeId, trust_bundle: TrustBundle) -> Self;
    pub fn desired(&self) -> GatewayIdentityDesired;
    pub fn current(&self) -> Option<GatewayIdentityFacts>;
    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    pub fn client_access(
        self: &Arc<Self>,
        clock: Arc<dyn Clock>,
    ) -> GatewayClientIdentityAccess;
    pub fn action_target(self: &Arc<Self>) -> GatewayIdentityActionHandle;
    pub fn lifecycle_target(self: &Arc<Self>) -> GatewayIdentityLifecycleHandle;
    pub(crate) fn begin_disable(
        &self,
    ) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
    pub(crate) fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    pub(crate) fn drop_after_drain(
        &self,
        epoch: GatewayIdentityEpoch,
    ) -> GatewayIdentityMutationOutcome;
}

pub struct GatewayIdentityActionHandle { slot: Arc<GatewayIdentitySlot> }
pub struct GatewayIdentityLifecycleHandle { slot: Arc<GatewayIdentitySlot> }
impl GatewayIdentityActionTarget for GatewayIdentityActionHandle {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn drop_after_drain(
        &self,
        epoch: GatewayIdentityEpoch,
    ) -> GatewayIdentityMutationOutcome;
}
impl GatewayIdentityLifecycleTarget for GatewayIdentityLifecycleHandle {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
}

pub struct GatewayClientIdentityAccess {
    slot: Arc<GatewayIdentitySlot>,
    clock: Arc<dyn Clock>,
}
impl GatewayClientIdentityAccess {
    pub(crate) fn with_current<R>(
        &self,
        use_material: impl FnOnce(
            &SvidMaterial,
            &TrustBundle,
        ) -> Result<R, GatewayClientMtlsError>,
    ) -> Result<R, GatewayClientMtlsError>;
}

pub struct HostGatewayClientMtls { /* slot, Clock, limits; private rustls state */ }
impl HostGatewayClientMtls {
    pub fn new(
        identity: GatewayClientIdentityAccess,
        seal: GatewayUpstreamSeal,
        limits: GatewayLimits,
    ) -> Self;
}
```

`GatewayIdentitySlot` implements both read traits above; its `subscribe` is not
exposed through `GatewayIdentityCurrentRead`. Serve composition obtains one
ActionHandle and one LifecycleHandle; direct slot mutators are crate-private.
Only the ActionHandle implements `GatewayIdentityActionTarget`, delegates to
hold/drop, and is installed in AppState; the executor passes the exact action
epoch, calls existing `issue_and_audit` before hold, and treats `StaleEpoch` as
a successful stale no-op. Only the LifecycleHandle implements
`GatewayIdentityLifecycleTarget`; the control-plane lifecycle adapter calls
begin-disable through it. `GatewayClientIdentityAccess::with_current` reads the
injected Clock, rejects absent/expired material, and executes its non-async
closure while the slot keeps the material borrowed; it cannot return or clone
the SVID/key/bundle. `HostGatewayClientMtls::new` is called once by `serve`
composition and the resulting adapter is supplied only as
`Arc<dyn GatewayClientMtls>`.

The control-plane adapter has one crate-private constructor and retains the
same canonical evaluation rather than reconstructing names/targets:

```rust
pub(crate) struct ControlPlaneGatewayIdentityLifecycle { /* slot + broker eval */ }
impl ControlPlaneGatewayIdentityLifecycle {
    pub(crate) fn new(
        target: Arc<dyn GatewayIdentityLifecycleTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self;
}
```

`ensure_current(deadline)` first returns an already time-usable Current,
otherwise submits its retained evaluation as Immediate and races the slot
subscription against `clock.sleep(deadline.saturating_duration_since(
clock.now()))` until the same desired epoch becomes Current, the stream closes,
or `Timeout { EnsureCurrent }` wins. `disable_after_drain(deadline)` calls
`begin_disable`, submits that same evaluation and performs the identical race
until Current is empty for the returned epoch or
`Timeout { DisableAfterDrain }`. `current`/`subscribe`
delegate only non-secret facts. No detached task or second reconcile call path
exists.

The action target is a narrow write-only port; neither production nor Sim
exposes held material:

```rust
// overdrive-gateway::ports
pub enum GatewayIdentityMutationOutcome { Applied, StaleEpoch }
pub trait GatewayIdentityActionTarget: Send + Sync {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn drop_after_drain(
        &self,
        epoch: GatewayIdentityEpoch,
    ) -> GatewayIdentityMutationOutcome;
}

// overdrive-control-plane
#[derive(Clone)]
pub struct GatewayIdentityActionComposition {
    kind: GatewayIdentityActionCompositionKind,
}
#[derive(Clone)]
enum GatewayIdentityActionCompositionKind {
    Disabled,
    Enabled(Arc<dyn GatewayIdentityActionTarget>),
}
impl GatewayIdentityActionComposition {
    pub const fn disabled() -> Self;
    pub fn enabled(target: Arc<dyn GatewayIdentityActionTarget>) -> Self;
    pub(crate) fn target(
        &self,
    ) -> Result<&dyn GatewayIdentityActionTarget, GatewayIdentityDisabled>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("gateway identity action unavailable: public ingress gateway is disabled")]
pub struct GatewayIdentityDisabled;

// overdrive-sim
pub struct SimGatewayIdentityActionTarget { /* epoch/current facts only */ }
impl SimGatewayIdentityActionTarget {
    pub fn new(desired: GatewayIdentityDesired) -> Self;
    pub fn current(&self) -> Option<GatewayIdentityFacts>;
    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    pub fn begin_disable(
        &self,
    ) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
}
impl GatewayIdentityActionTarget for SimGatewayIdentityActionTarget {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn drop_after_drain(
        &self,
        epoch: GatewayIdentityEpoch,
    ) -> GatewayIdentityMutationOutcome;
}
impl GatewayIdentityLifecycleTarget for SimGatewayIdentityActionTarget {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn begin_disable(&self) -> Result<GatewayIdentityEpoch, GatewayIdentityEpochError>;
}

pub struct SimGatewayIdentityLifecycleControl {
    target: Arc<SimGatewayIdentityActionTarget>,
    broker: Arc<Mutex<EvaluationBroker>>,
    evaluation: Evaluation,
    clock: Arc<dyn Clock>,
}
impl SimGatewayIdentityLifecycleControl {
    pub fn new(
        target: Arc<SimGatewayIdentityActionTarget>,
        broker: Arc<Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self;
}
#[async_trait::async_trait]
impl GatewayIdentityLifecycleControl for SimGatewayIdentityLifecycleControl {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError>;
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    async fn disable_after_drain(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError>;
}

// Exact additions to the existing ShimError enum.
pub enum ShimError {
    // existing variants unchanged
    GatewayDemandUnavailable,
    GatewaySvidIssue(#[source] CaIssuanceError),
    GatewayIdentityDisabled(#[from] GatewayIdentityDisabled),
}

// overdrive-control-plane::action_shim::gateway_svid
pub(crate) async fn dispatch_issue(
    action: &Action,
    ca: &dyn Ca,
    observation: &dyn ObservationStore,
    clock: &dyn Clock,
    target: &dyn GatewayIdentityActionTarget,
) -> Result<GatewayIdentityMutationOutcome, ShimError>;
pub(crate) fn dispatch_drop(
    action: &Action,
    target: &dyn GatewayIdentityActionTarget,
) -> Result<GatewayIdentityMutationOutcome, ShimError>;
```

`AppState` carries one mandatory `GatewayIdentityActionComposition`.
The existing `dispatch`, `dispatch_with_network_provisioner`,
`dispatch_with_workflow_intent`, the integration-only network-provisioner form,
and their shared `dispatch_single` each gain `&GatewayIdentityActionComposition`
in the same position and every production wrapper passes `AppState`'s value.
`IssueGatewaySvid` invokes `issue_and_audit` and then `hold_audited`; audit/
issuance failure returns `ShimError::GatewaySvidIssue(source)`, while
`StaleEpoch` is a successful stale no-op. `DropGatewaySvid` calls
`drop_after_drain`; disabled composition returns
`ShimError::GatewayIdentityDisabled(source)`. No executor obtains
private material from the slot and no dispatcher overload omits this mandatory
composition.

Both ShimError variants have fixed outer Display text—`"gateway SVID issuance
or audit failed"` and the `GatewayIdentityDisabled` text above—while retaining
their typed source for matching; no SVID/key bytes render. Existing
first-error/per-action-isolation semantics remain unchanged.

`dispatch_issue` accepts only `Action::IssueGatewaySvid`, passes its exact
node/SPIFFE identity to `issue_and_audit`, and passes the returned material plus
action epoch to the target. `dispatch_drop` accepts only
`Action::DropGatewaySvid` and passes its epoch. The shared `dispatch_single`
resolves `gateway_identity.target()?` in each corresponding arm, calls the one
function, and maps both `Applied` and `StaleEpoch` to `Ok(())`.
`GatewayIdentitySlot` implements `GatewayIdentityActionTarget` by delegating to
its identically named inherent operations. The Sim type implements that port
plus `GatewayIdentityDesiredRead` and `GatewayIdentityCurrentRead` over one
mutex-held deterministic state; its current projection never contains key
material. `SimGatewayIdentityLifecycleControl` submits its retained Evaluation
to the same real `EvaluationBroker` and uses the injected Sim Clock plus target
watch for the same ensure/disable absolute-deadline races as production; it
never sets Current directly. The Sim reconciler/action path must produce
`hold_audited`/`drop_after_drain`, preserving adapter equivalence.

`SpiffeId::for_gateway(node)` yields exactly
`spiffe://overdrive.local/gateway/<node-id>`. A dedicated
`GatewayIdentitySlot` under the existing dataplane mTLS owner holds exactly one
non-allocation SVID generation; it never enters `IdentityMgr` or generic
`IdentityRead`. The slot's same lock owns a checked process-local desired epoch:
constructor epoch one means enabled; `begin_disable` increments and sets None;
Issue/Drop Actions carry the epoch; `hold_audited`/`drop_after_drain` reject a
stale action, so an in-flight Issue cannot re-hold after disable.
`GatewaySvidLifecycle` reuses workload SVID TTL/half-life/retry/restart policy
with those epoch-bearing `IssueGatewaySvid`/`DropGatewaySvid` Actions. Executors reuse
`issue_and_audit`; audit succeeds before atomic epoch-checked hold. Only
`HostGatewayClientMtls` alone has the access capability's crate-private use
path; status/reconciler see
SPIFFE ID/serial/expiry only.

This epoch is the Application concurrency realization of DDD's existing
Ensure/Reissue/Drop commands, not a new domain identity or persisted fact;
[ADR-0109] supersedes any unguarded Application shorthand while D-DOM-1…13's
semantic owner/invariants remain unchanged.

Gateway-client mTLS accepts a connected fd plus the receipt-selected exact
SpiffeId, presents `[gateway leaf, node intermediate]` (never root), anchors
verification at only the root, derives rustls's IP `ServerName` from
`getpeername` on the actual BPF-rewritten fd, validates exactly one URI SAN and
enforces equality before returning authenticated `GatewayUpstream`.
Only after that equality succeeds does Host call
`GatewayUpstreamSeal::seal_authenticated`. The Sim adapter calls it only after
its scripted peer equality succeeds. Those are the only two production/test
call sites; arbitrary byte streams cannot construct, convert into, clone or
unwrap `GatewayUpstream`.
It never fabricates AllocationId or uses transparent `InterceptedConnection`/
kTLS pump. Expiry/loss stops new admission/upstreams; reissue preserves old
only while usable. Shutdown drains authenticated work before Drop/zeroize.

Composition uses `UnboundGatewayBuilder::bind_demand_wake` typestate to return
one Enabled `GatewayDemandComposition` containing the builder-owned read/ack
plus broker wake and the one non-Clone `GatewayUpstreamSeal`, then calls
`GatewayBuilder::start` with connect Dataplane,
gateway-client mTLS and identity lifecycle control. `AppState` always holds the
composition: enabled passes its read into HydrationContext and its full bundle
through every action/workflow-emission dispatch wrapper; disabled uses the
canonical Empty read plus no dispatch bundle.
`StartedGateway::into_parts` yields `GatewayHandle` + `GatewayControl` for
ServerHandle/AppState; the handle receives the complete listener/connection/
application/demand/source/custody/Route task tree and the exact same retained
Application shutdown, Demand, Dataplane, cleanup-ledger and identity roots it
uses during close/drain/shutdown. Route absence is non-error and leaves public TCP/443
unbound until a complete generation converges. Connect/identity/mTLS/bind gate
failure likewise leaves the active operator-mTLS status surface available and
TCP/443 unbound; only configuration, Route/custody construction/storage or the
initial manual certified-key source is a fatal `GatewayBootError`.

`ServerHandle::shutdown` first calls `GatewayHandle::shutdown` when present,
before touching `convergence_shutdown`. The gateway handle computes one
absolute `deadline = clock.now() + grace` and always executes this sequence:

1. Call `connections.stop_accepting()`, await the listener task, and prove no
   further accept can acquire a connection lease through
   `tasks.join_listener(deadline, clock)`. Then call
   `application.close_for_shutdown(deadline)`, which closes every Current/
   Draining retention, atomically clears admission, and removes any staged
   candidate/demand; retry a returned staged-removal failure through the demand
   handle before continuing. Append command/demand unavailable/timeout failure
   and continue if it cannot complete.
2. Await each retention `wait_zero(deadline, clock)` and the connection owner;
   at the same deadline abort every remaining connection task, then await every
   aborted JoinHandle so Drop accounting and cleanup guards have run, and take
   the connections owner JoinHandle through
   `tasks.join_connections(deadline, clock)`.
3. Retry the bounded cleanup ledger, call
   `cleanup_all_gateway_intents`, and independently read
   `live_intent_count` plus `live_receipt_count`. The cleanup method removes and
   reports both map families. Only successful sweep + `Some(0)` for both counts
   + empty ledger yields `residual_connect=None`. Sweep/count error, unknown
   count, remaining intent, remaining receipt or ledger entry produces
   `GatewayResidualConnectState` with the exact typed failure/count/category.
   Never discard it.
4. Call `application.retire_closed(deadline)` only after the joined lease
   counts are zero, then call `demand.clear_all(deadline, clock)` as the
   authoritative staged/current/draining complement before cancelling any
   owner. Append `GatewayApplicationCommandError::Demand` if it fails. Then
   cancel Application, Demand, Manual Source, Custody and
   Route Set in that order. `join_listener` ran in step 1,
   `join_connections` runs after the force-join in step 2, and
   `join_remaining(deadline, clock)` joins every owner task or aborts+awaits it
   at the same absolute deadline. Command, inner-task, channel, timeout,
   cancellation and panic failures are appended and never skip a later task.
5. Call `identity.disable_after_drain(deadline)`, await Current Empty and append
   the exact typed error to `identity_error` and set `identity_empty=false` on
   timeout/error; success sets `identity_empty=true`. Convergence/action
   dispatch remains live through this step. Close/retire/staged-demand command
   failures are accumulated in `application_errors`.

Only after that report returns does ServerHandle cancel/join convergence, then
resume the existing emit-drain, interest-router, operator-server,
exit-observer, DNS and workload-mTLS shutdown order. It accumulates the existing
mTLS failure plus any non-clean gateway report into `ServerShutdownError` after
all owners have been attempted. Deadline, cancellation, keep-alive and task-
panic paths therefore join users before demand retirement/SVID drop and expose
typed residue instead of logging it away.

### Lifecycle Gate Ownership — application completion

| Gate | Owner / affected promise | Failure projection | Explicitly unaffected |
|---|---|---|---|
| Public Listener Bound | GatewayHandle/serve; dynamic TCP/443 after gates | no degraded/partial listener | Allocation Running, Service Stable, Backend Eligibility, operator HTTPS |
| Certified Key Usable | Custody current public key | expiry/loss stops accepts first | Route Accepted, Service/BPF, Gateway SVID |
| Gateway Identity Current | dedicated slot/lifecycle | reissue; no new admission/upstream when unusable | allocation SVIDs/public key/BPF |
| Frontend Demand Applied | demand owner + Hydrator ack | staged cannot become Current | Route truth/backend authority |
| Gateway Applied Generation | GatewayApplicationOwner Current Arc | atomic current or no admission | BPF Hydrated/Backend Eligibility |
| Selected Peer Authenticated | gateway-client mTLS per request | 502/no fallback or retry | future requests/lifecycle state |

Seeded invariants cover staged-before-swap, current-to-draining/no-early-retire,
stale ack/result rejection, key/identity loss before unbind, demand TEACH
convergence, receipt cleanup/isolation, gateway SVID issue/reissue/drop and
unchanged allocation/Service/BPF meanings.

## Wave: DESIGN / [REF] Updated Reuse Analysis

| Existing component | Overlap | Final application decision | Why |
|---|---|---|---|
| `run_server_with_obs_and_drivers` / `ServerHandle` | startup/task/drain | **EXTEND** | Sole active gateway/app/listener/connector/drain owner; no supervisor |
| workload deploy parser/API | generic `deploy` journey | **EXTEND WITH OUTER DISCRIMINATOR** | Route stays outside workload enum/endpoint/stream |
| IntentStore + ADR-0048 codec | durable intent and wake | **EXTEND** | Reuse store/envelopes; lag-aware watch is required for fail-closed withdrawal |
| `ListenerFactStore` | listener/VIP projection | **DO NOT EXTEND** | Derived ServiceId key, stop eviction and binary absence cannot fulfill stable reference/cause contract |
| WorkloadIntent + `ServiceVipView` | exact frontend inputs | **REUSE** | Direct authoritative join; no duplicate map |
| ServiceLifecycle / ServiceBackendRow | Backend Eligibility | **REUSE INDIRECTLY; FORBID GATEWAY READ** | Sole complete-row owner remains |
| ServiceMapHydrator / Dataplane / XDP | backend application/selection | **EXTEND HYDRATOR/DATAPLANE; XDP UNCHANGED** | Demand-gated Path-A TEACH + registered cgroup receipt; no selector/raw handler access |
| `BackendIdAllocator` | endpoint-to-id memo | **EXTEND ONE CRATE-PRIVATE READ; REUSE MONOTONIC NONREUSE** | `existing_id` permits identity-conflict preflight before mutation; allocation/release semantics stay owned by Dataplane |
| KEK/HKDF/AES-GCM mechanism | key protection | **REUSE MECHANISM, DISTINCT PUBLIC-KEY RECORD/DOMAIN** | Avoid CA/public-key type confusion and subkey reuse |
| rustls/hyper | TLS/HTTP mechanics | **REUSE LIBRARIES; CREATE PUBLIC RUNTIME** | Operator CA/router are a different trust boundary |
| `Ca` / `ca_issuance` / IdentityMgr | SVID issue/audit/hold foundations | **REUSE ISSUE/AUDIT; CREATE DEDICATED SLOT** | Allocation holder stays unchanged; no fake AllocationId |
| `SvidLifecycle` | allocation identity convergence policy | **REUSE POLICY; CREATE GATEWAY SIBLING** | One node slot has different desired/actual/actions |
| ObservationStore | convergent non-secret status | **EXTEND TWO ROW FAMILIES** | Existing store/LWW mechanism; no new store/event sourcing |
| `arc-swap`, `tokio-rustls`, `async-trait`, `rustix`, `zeroize`, `x509-parser` | atomic pointer/TLS/object-safe async/safe SO_COOKIE/secret/X.509 mechanics | **PROMOTE LOCKED OSS DEPENDENCIES** | Narrow necessary mechanics; MIT/Apache/ISC-compatible, no proprietary dependency |

Contract-shape extension of the Reuse Analysis (the declared universe is the
only mutation set; everything else is complement-equal):

| Reuse rows | Contract shape / declared universe | Assertion mechanism |
|---|---|---|
| serve + workload deploy | **bounded-change**: mandatory Enabled/Disabled AppState demand composition, optional gateway handle/control/task tree and outer dispatch result only | real hydrate/all-wrapper stale guard, disabled-state delta, existing workload regression, boot/shutdown integration |
| IntentStore + ObservationStore | **bounded-change**: fixed Route/key and two status keys/tables only | codec goldens, store state-delta PBT, lag/relist DST |
| ListenerFactStore | **unbounded-preservation**: no call/import/change | dependency lint + source-diff guard |
| WorkloadIntent + ServiceVipView | **unbounded-preservation** read of exact reference inputs | adapter equivalence with whole-store/allocator complement |
| ServiceLifecycle / ServiceBackendRow | **unbounded-preservation** indirect input; gateway cannot read/write | import lint + pre/post row equality |
| ServiceMapHydrator / Dataplane / XDP | **bounded-change**: demanded ServiceKey maps, ack and identity association; LOCAL/undemanded/XDP preserved | state-delta DST, BPF POD tests, real-kernel complement probe |
| BackendIdAllocator | **bounded-change**: read-only `existing_id`; allocator state unchanged by preflight | allocator snapshot PBT + conflict no-mutation integration |
| KEK/AEAD mechanism | **bounded-change**: distinct public-key HKDF/AAD/record; CA records preserved | tamper/fault integration + domain-separation golden |
| rustls/hyper | **bounded-change** per public/upstream connection and its finite budget | protocol matrix, wrong-peer integration and wire AT |
| CA issue/audit + IdentityMgr | **bounded-change**: one gateway-subject audit output; allocation holder untouched | audit-before-hold integration + allocation-held-set complement |
| SvidLifecycle | **pure-function** sibling inputs -> gateway Actions/View only | paired lifecycle PBT + existing workload reconciler byte equality |
| arc-swap/tokio-rustls/async-trait/rustix/zeroize/x509-parser | **bounded-change** inside the owning snapshot/connection/syscall/secret values | API/trybuild guards, parser/syscall fault matrix and drain tests |

The mandatory CREATE NEW challenge is satisfied: no new component duplicates an
existing owner, and the new crate adds no deployment unit. `cargo xtask
dst-lint` enforces `control-plane -> {gateway,dataplane,reconcilers} -> core`,
the gateway's host edge (public-key codec + cookie reader), and Dataplane's
gateway-port/host-reader edges; it rejects gateway
production references to `ServiceBackendRow`, its all-row reader, or raw
SERVICE/BACKEND/MAGLEV map symbols. Trybuild/API tests keep secret types
non-renderable/non-serializable and application fields private. The same lint
permits `GatewayClientIdentityAccess::with_current` only in
`overdrive-dataplane/src/mtls/gateway_client.rs`, and permits
`GatewayUpstreamSeal::seal_authenticated` only there and in the exact
`overdrive-sim` gateway-client adapter; any other production call is a failure.

## Wave: DESIGN / [REF] Public surface and live-caller audit

Every new cross-crate item is reachable from the first-slice production
composition. Items needed only inside `overdrive-gateway` are crate-private;
the table is the final dormant-surface gate:

| Exact surface group | Constructor/producer | Live production caller/consumer |
|---|---|---|
| Route IDs/hostname/path/listener/Route/generation plus parser and HTTP input types | top-level deploy parser and `Route::from_submit` | `deploy_resource` → `ApiClient::submit_route` → Route POST; Route owner/application/status use accessors; public runtime calls `PathMatch::matches_raw_path` |
| `PublicRouteSetHandle::{apply,withdraw}` | crate-private Route owner startup inside `UnboundGatewayBuilder::new` | Route POST/DELETE through `GatewayControl::routes`; there is no CLI DELETE helper |
| Four typed gateway clap/`ServeArgs` fields, `GatewayConfig`/`ManualCertifiedKeyConfig`, `GatewayConfigError` and accessors | clap `Command::Serve` → `main` → first `ServeArgs::gateway_config` call | `run_inner` moves only complete config into `ServerConfig.gateway`; `run_server_with_obs_and_drivers` and `UnboundGatewayBuilder::new` consume it; handlers never receive paths |
| `GatewayAppStateComposition`, canonical disabled identity read and appended AppState field/constructor parameter | broad fixtures default Disabled; production/Tier-3/Sim may pass Enabled | Hydration builder, every action/workflow wrapper and Route/status handlers through accessors only; Arc ownership preserves AppState Clone/borrow lifetime |
| `ControlPlaneError::GatewayDisabled` and exact response arm | Route POST/DELETE pre-parse control check | exhaustive `to_response` and live OpenAPI 409 ErrorBody |
| `ServiceFrontendResolve::{probe,resolve}` and closed errors | private `IntentServiceFrontendResolver` construction in the builder | gateway owner startup/application convergence only |
| Demand values/read/ack/wake and `GatewayDemandComposition` | private demand owner plus `bind_demand_wake` | all HydrationContext builders and every action/workflow dispatch wrapper; application owner uses stage/wait/promote/drain/retire and GatewayHandle uses deadline-bound `clear_all` before demand cancellation |
| `GatewayConnectDataplane` intent/receipt/identity/cleanup methods | `EbpfDataplane` and Sim adapters | builder probe, private connector per upstream, handle shutdown sweep and status live-count projection |
| Revised `DataplaneUpdateService`, `DispatchOutcome`, transaction stages/rollback failures and crate-private `BackendIdAllocator::existing_id` | demanded ServiceMapHydrator branch and existing Dataplane action shim | guarded `EbpfDataplane::update_service`, hydration observation and exact demand acknowledgment only |
| `IntentSubscriptionEvent` and revised `IntentStore::watch` | LocalStore and Sim | every existing watcher plus Route/custody/application subscribe-before-list loops |
| `SocketCookie`, `GatewayIdentityEpoch`, `CertifiedKeyGeneration`, `CertificateFingerprint` constructors/accessors | socket reader, gateway slot, certified-key candidate/record decode | BPF adapter, lifecycle/action dispatch, custody/application/status respectively |
| `CertifiedKeyCandidate`, install command/outcome/errors and `PublicCertifiedKeyCustodyHandle::{id,install,record_install_failure,snapshot,subscribe}` | manual source/custody owner | manual source calls both mutations; Route resolution calls `id`; application owner calls snapshot/subscribe. There is no custody `withdraw`. |
| `PublicCertifiedKeyAeadCodec::{new,seal,open}` and protection context/record/envelopes | `run_server_with_obs_and_drivers` with boot-probed Kek | custody only; feature-gated `with_random_fill_for_test` is called only by cross-crate codec integration; future #57 reuses custody `install`, never codec directly |
| `PublicCertifiedKeySnapshot` non-secret accessors plus crate-private TLS capability | custody only | application generation/status use metadata; public TLS runtime alone uses `server_config` |
| `GatewayApplicationGenerationId`, private-constructor `GatewayAppliedGeneration`, retention and non-Clone leases | application owner after all inputs/demand ack | private admission, connection/request runtime, demand and application status only |
| `UnboundGatewayBuilder`, `GatewayBuilder`, `GatewayRuntimeDependencies`, `StartedGateway`, `GatewayUpstreamSeal` | `run_server_with_obs_and_drivers` | same composition call chain; the seal is passed only to Host/Sim mTLS; no alternate factory/partial start |
| `GatewayHandle`, task tree/shutdown capabilities/report and ServerHandle field/error extension | `StartedGateway::into_parts` | `ServerHandle::{shutdown,abort_for_test}` only; typed residue is never dropped |
| `GatewayControl::{routes,application_status,certified_key_status}` | `StartedGateway::into_parts` | Route POST/DELETE and operator-mTLS Gateway GET only; no task/admission/key access |
| Status rows/envelopes/write variants/kinds/point reads and wire bodies | custody/application writers plus LocalStore/Sim adapters | `GatewayControl` and Gateway GET projection only |
| `GatewayLimits::first_slice` and read-only accessors | one call in `run_server_with_obs_and_drivers` | builder/runtime, `HostGatewayClientMtls` and BPF capacity assertion; no deserializer/builder/mutator |
| `normalize_route_authority`, `rewrite_request_headers` and `rewrite_response_headers` | public HTTP connection/request path | Route selection and upstream/downstream Hyper framing; all three are crate-private and their source-local PBT calls the same production functions |
| `GatewayIdentityDesiredRead`/`GatewayIdentityCurrentRead` | `GatewayIdentitySlot` | lifecycle hydration and status only; `GatewayIdentityCurrentRead` has no `subscribe` |
| `GatewayIdentityLifecycleControl::{ensure_current,current,subscribe,disable_after_drain}` | control-plane adapter over slot/broker plus named Sim adapter over Sim target/real broker/Sim Clock | active application owner in production and seeded application composition in Sim |
| `GatewayIdentityActionComposition`/target and Sim target | Enabled/Disabled AppState composition | every exact action-shim dispatcher and only the Issue/Drop arms; no private-material getter |
| `GatewayIdentitySlot` constructor/non-secret reads plus ActionHandle, LifecycleHandle and client-access capabilities | `run_server_with_obs_and_drivers` | lifecycle-control adapter, Gateway SVID action executor and `HostGatewayClientMtls`; slot mutators/access closure are crate-private and only the matching opaque handle/adapter can reach them |
| `GatewaySvidLifecycle::canonical`, factory, Any* variants and Issue/Drop actions | control-plane canonical registration | existing reconciler runtime/broker/exhaustive dispatch and action shim only |
| `GatewayClientMtls::authenticate`, opaque `GatewayUpstream`, Host/Sim mTLS constructors | `run_server_with_obs_and_drivers` / Sim composition | private connector readiness probe and request path; only successful exact-peer adapters hold the unforgeable seal needed to construct the authenticated stream |
| `Invariant::{GatewayApplicationGenerationLifecycleIsSafe,GatewayApplicationShutdownConverges}` and evaluator functions | `Invariant::ALL` plus existing `cargo dst --only` parser | exhaustive `Harness::evaluate` calls the crate-private evaluators with the run seed; no production or test-only gateway API is added |

The same audit removes the two previously dormant proposals:
`PublicCertifiedKeyCustodyHandle::withdraw` does not exist, and
`GatewayIdentityCurrentRead::subscribe` does not exist. Subscription belongs
only to the command-capable lifecycle control used by the active application
owner (and to the Dataplane-local slot that backs that adapter). No method is
kept solely for a hypothetical implementation convenience.

## Wave: DESIGN / [REF] Application C4 reference

The sole detailed C4 source for this subsystem is
[canonical public-ingress C4](../../product/architecture/c4-diagrams.md#public-ingress-gateway-canonical-c4).
It depicts the active Application owner, custody replacement commit order,
commit-guarded Dataplane publication, dedicated gateway identity, exact-peer
mTLS and public runtime relationships. This feature delta owns their exact
implementation-facing contracts; it intentionally carries no duplicate C4
source.

## Wave: DESIGN / [REF] Application technology choices

| Technology | Exact pin/status | License / use |
|---|---|---|
| Rust | workspace edition 2024, MSRV 1.88 | Existing implementation language |
| Tokio | workspace 1 | Existing runtime, cancellation and bounded task ownership |
| async-trait | workspace 0.1 | Apache-2.0/MIT; object-safe async driven ports used as `Arc<dyn ...>` |
| hyper / hyper-util | lock 1.9.0 / 0.1.20 | MIT; strict HTTP/1.1 parser/client and Tokio IO adapter |
| rustls / tokio-rustls | lock 0.23.39 / 0.26.4 | Apache-2.0/MIT; public TLS 1.3, exact-SNI config per connection |
| ring | workspace 0.17 | ISC/OpenSSL; honest current provider for key match/AEAD; no FIPS claim |
| rustix | workspace 1; add safe `net` beside existing `std,fs` | Apache-2.0/MIT; `rustix::net::sockopt::socket_cookie`, preserving `overdrive-host`'s `#![forbid(unsafe_code)]` |
| arc-swap | already locked, promote direct | Apache-2.0/MIT; atomic whole admission pointer |
| zeroize | already locked, promote direct | Apache-2.0/MIT; plaintext origin-key drop |
| x509-parser | workspace 0.18, promote runtime in gateway | Apache-2.0/MIT; SAN/profile/validity extraction |
| rkyv / redb | workspace 0.8 / 2 | MIT / Apache-2.0; typed versioned intent and status persistence |

`instant-acme` remains unused. No external proxy, queue, cache, load balancer,
CDN, DNS provider, ACME client, new store or proprietary component is added.

## Wave: DESIGN / [REF] Application verification and full-stack handoff

### Exact seeded Gateway Application invariant registration

The Gateway Application lifecycle uses the existing `cargo dst` catalogue and
seed controls; it does not gain a second runner or one variant per example.
Two truths are independently useful: generation lifecycle safety while the
owner is live, and shutdown convergence after ownership is revoked. The exact
additions are:

```rust
// overdrive-sim::invariants
pub mod gateway_application;

pub enum Invariant {
    // existing variants unchanged
    GatewayApplicationGenerationLifecycleIsSafe,
    GatewayApplicationShutdownConverges,
}

// overdrive-sim::invariants::gateway_application
pub(crate) async fn evaluate_generation_lifecycle_is_safe(
    seed: u64,
) -> InvariantResult;
pub(crate) async fn evaluate_shutdown_converges(seed: u64) -> InvariantResult;

pub(crate) enum GatewayApplicationSimInput {
    ApplyRoute(Route),
    WithdrawRoute(RouteId),
    RetainCurrentPublicConnection,
    ReleaseRetainedPublicConnection,
    RestartOwner,
    BeginShutdown,
}

pub(crate) enum GatewayApplicationSimFault {
    HoldDemandCompletion(GatewayFrontendDemandApply),
    ReleaseDemandCompletion(GatewayFrontendDemandApply),
}

pub(crate) struct GatewayApplicationInvariantCase {
    pub(crate) inputs: Vec<GatewayApplicationSimInput>,
    pub(crate) faults: Vec<GatewayApplicationSimFault>,
}
impl GatewayApplicationInvariantCase {
    pub(crate) fn generation_lifecycle(seed: u64) -> Self;
    pub(crate) fn shutdown(seed: u64) -> Self;
}
```

The evaluator/control definitions live only in
`crates/overdrive-sim/src/invariants/gateway_application.rs`; catalogue
variants and canonical names live only in
`crates/overdrive-sim/src/invariants/mod.rs`, and dispatch lives only in the
existing exhaustive match in `crates/overdrive-sim/src/harness.rs`.

`Invariant::as_canonical` maps the variants respectively to
`gateway-application-generation-lifecycle-is-safe` and
`gateway-application-shutdown-converges`; both variants are appended exactly
once to `Invariant::ALL`. The existing exhaustive `Harness::evaluate` adds the
two arms below, so existing `Display`, `FromStr`, `Harness::only` and report
rendering require no parallel registration surface:

```rust
Invariant::GatewayApplicationGenerationLifecycleIsSafe =>
    gateway_application::evaluate_generation_lifecycle_is_safe(seed).await,
Invariant::GatewayApplicationShutdownConverges =>
    gateway_application::evaluate_shutdown_converges(seed).await,
```

The only CLI inputs remain the existing `--seed <u64>` and `--only <NAME>`:

```text
cargo dst --seed <N> --only gateway-application-generation-lifecycle-is-safe
cargo dst --seed <N> --only gateway-application-shutdown-converges
```

There is no lifecycle-specific CLI flag. Each case constructor uses
`StdRng::seed_from_u64(seed)` to order only concurrently deliverable items;
every seed still contains all mandatory inputs/faults. `ApplyRoute` and
`WithdrawRoute` drive `GatewayControl::routes`, restart re-composes the same
durable stores through `GatewayBuilder::start`, and `BeginShutdown` consumes
the real `GatewayHandle`. Retain/release drives a public TLS connection through
the active listener. Demand hold/release is implemented only by the
`overdrive-sim` adapter for the already-approved
`GatewayFrontendDemandAcknowledge`/wake path and carries the real
`GatewayFrontendDemandApply`; it neither invokes a private owner method nor
adds a production hook. `SimClock` and the existing Sim identity/dataplane/mTLS
adapters supply time and effect completion. The seed and generated schedule
are included in the failure cause, and twin runs at one seed must produce the
same ordered observation trace.

`generation_lifecycle(seed)` always applies three generations of the same
Route ID, retains a connection on the first Current, holds the second
generation's demand completion, supersedes it with the third, releases the
stale second completion, promotes the third, withdraws it, releases the first
connection, and restarts once against the unchanged durable intent at a
seed-selected legal boundary. Its oracle reads only
`GatewayApplicationStatusRowV1` plus `GatewayFrontendDemandRead::snapshot` and
asserts at every observation:

- Staged precedes promotion, Current changes in one generation step, and the
  displaced Current remains Draining while its retained connection is open.
- The demand snapshot is exactly the phase-precedence union of
  Staged/Current/Draining ServiceKeys; no generation retires before its
  retained connection closes.
- A released completion whose `(revision, ServiceKey)` no longer names Staged
  never becomes Current and never acknowledges or retires the successor.
- Withdrawal unbinds/stops admission before clearing Current, then converges
  to no Staged/Current/Draining demand after retained leases close. Restart
  relists durable Route intent and never compares demand revisions across
  processes or resurrects a withdrawn/superseded generation.

`shutdown(seed)` begins with one Current retained connection and one Staged
generation whose completion is held, then permutes completion/connection
release around `GatewayHandle::shutdown`. Its oracle uses the final
`GatewayShutdownReport`, the last application status row, the demand snapshot
and non-secret identity read. A passing trace has listener unbound and no new
admission, no Staged or Current application, an empty demand snapshot after
all retained leases join, `identity_empty == true`, zero ledger entries, and
`live_intents == Some(0)` plus `live_receipts == Some(0)`. Any typed task,
application, identity or cleanup failure makes the invariant fail with the
seed; the evaluator never treats a non-clean report as convergence. These are
observable-owner oracles, not copies of private owner fields or a simulated
replacement state machine.

| Evidence lane | Required contract |
|---|---|
| Pure/PBT | `PathMatch::matches_raw_path`, `runtime::routing::normalize_route_authority` and both `runtime::proxy_headers::rewrite_*_headers` functions own the Route-path/authority/header properties; Route/hash laws, demand phase/union/revision, receipt matching/cleanup, immutable BackendId identity, gateway SVID lifecycle and finite limit ordering remain in their owning source modules. Every live property carries `/// CONTRACT_SHAPE: pure-function.` |
| DST | `GatewayApplicationGenerationLifecycleIsSafe` owns staged/current/draining, stale completion, withdrawal and restart safety; `GatewayApplicationShutdownConverges` owns joined shutdown convergence. Existing demand TEACH, registered/unregistered isolation, connector cleanup and key/identity fail-closed complements remain registered with their owning component evidence. |
| In-process integration | Real stores/Route/custody/application/status; Hydrator/action demand guard+ack; Sim connect/identity/mTLS equivalence; typed boot/shutdown and concurrent retained generations; production `ServerHandle` ordering. Rust tests do not spawn the product binary. |
| `AT-PIG-E2E-1` recurring Tier-3 production composition | `tests/tier3/public-ingress-gateway/runner.sh` drives the built default-feature binary plus the checked-in root example through real serve/Service/Route/external test-CA TLS; it proves demand TEACH -> registered cgroup-BPF Maglev/rewrite -> BackendId receipt/identity -> gateway-SVID exact-peer mTLS -> streamed workload response. It is not an expectation or Rust test. |
| Tier 3 | Real-kernel cookie equality, Selected/NoBackend receipt, identity association, unchanged unregistered LOCAL/XDP behavior, Path-A nft-TPROXY, wrong-valid-peer and cleanup. |
| Point-in-time EDD `E14-PIG-OPERATOR-JOURNEY` | `verification/expectations/E14-public-ingress-operator-journey/runner.sh` independently drives the checked-in example through one SHA-pinned built binary and a publicly trusted operator-supplied chain, recording stakeholder-visible evidence only. It neither calls nor replaces `AT-PIG-E2E-1`. |

### Artifact-boundary handoff to DISTILL

DESIGN creates no tests or examples, but fixes their distinct homes so DISTILL
cannot collapse the evidence layers:

| Artifact | Exact planned path and role |
|---|---|
| Operator-runnable journey | `examples/public-ingress-gateway/README.md`, `service.toml`, `route.toml`, `workload.py`, `prepare-test-ca.sh`, and `run-example.sh`. The README/run script invoke a supplied built `overdrive` binary, use caller-supplied chain/key/CA paths when all are present or otherwise generate a bounded test CA/cert/key in a caller-provided scratch directory, run real `serve`, deploy the checked-in Service and Route specs, and make the external TLS1.3/HTTP1.1 request. It is explanatory/operator-runnable and owns no regression oracle. |
| Recurring production-composition test | `tests/tier3/public-ingress-gateway/runner.sh <absolute-overdrive-binary> <scratch-dir>`. The shell runner requires a default-feature binary built before invocation, calls the checked-in example rather than recreating specs/workload, and asserts the public response plus kernel/wire/cleanup facts with external tools. It imports/links no crate and invokes neither `cargo test`, a Rust test binary nor `verification/expectations`. This is the sole artifact carrying ID `AT-PIG-E2E-1`. |
| Point-in-time expectation | `verification/expectations/E14-public-ingress-operator-journey/{README.md,runner.sh}`. It invokes the same root example with a SHA-pinned built default-feature binary and operator-supplied publicly trusted chain, captures only stakeholder-visible command/status/TLS/response output, and has no recurring-regression or internal/kernel oracle role. |

The checked-in `run-example.sh` starts the supplied product binary with this
exact gateway argv shape (its local variable/positional-argument plumbing may
not rename or omit a flag):

```sh
"$overdrive_bin" serve \
  --gateway-address "$gateway_address" \
  --gateway-certified-key-id api-origin \
  --gateway-certificate-chain "$certificate_chain_path" \
  --gateway-private-key "$private_key_path"
```

`route.toml`'s `certified_key = "api-origin"` is byte-equal to the flag value.
When the example creates its bounded test CA, it passes the generated
leaf-first chain file and PKCS#8 key file through these same two flags; when an
operator supplies files, it substitutes only the path values. Neither mode
creates an alternate config file, environment-only credential channel or ACME
producer.

Both external runners supply the public certified key only through the real
`overdrive serve` arguments. Neither may hand-program a BPF map, install a
TPROXY rule, inject a Route/application/demand snapshot, fabricate a gateway
identity or use an implementation crate. The recurring Tier-3 runner cleans
its process/cgroup/socket state even on assertion failure. DISTILL owns the
executable scenarios and matrices; this section fixes only their architecture
boundary, production composition and non-substitution rule.

### Full-stack handoff

System ADR-0104/0117–0121, Route ADR-0105, Domain ADR-0112–0116 and
Application ADR-0106–0111 now
form one working contract. The cgroup-BPF hook—not XDP—
selects the host-originated gateway socket; XDP wire ingress remains unchanged.
Gateway Application supplies frontend demand only, never backend state or
selection. Public 404, NoBackend 503 and internal 502 are active exact
mappings. Manual Public Certified Key is the only producer; #57 ACME remains
later and additive.

`AT-PIG-E2E-1` is the first recurring production-composition acceptance
boundary, not an expectation or spike. No
isolated scratch mechanism or roadmap is introduced in DESIGN. Application
open questions: none; DISTILL receives the exact interfaces, lifecycle/error
semantics and evidence lanes above.

## Wave: DESIGN / [REF] Application outcomes

Registered typed contracts: `OUT-PIG-CERTIFIED-KEY-CUSTODY`,
`OUT-PIG-GATEWAY-APPLICATION`, `OUT-PIG-BPF-SELECTION-IDENTITY`,
`OUT-PIG-GATEWAY-EXACT-PEER`, and `OUT-PIG-PUBLIC-REQUEST`. They relate to
existing `OUT-CA-SVID`, `OUT-WIM-SVID-LIFECYCLE`,
`OUT-MTLS-HANDSHAKE-FAIL-CLOSED`, `OUT-MTLS-COMPOSED-PROXY-SKELETON`,
`OUT-MTLS-INBOUND-SERVER-MTLS`, and `OUT-MTLS-WIRE-TLS13` rather than
duplicating them.

Post-Reuse-Analysis collision gate:
`nwave-ai outcomes check-delta docs/feature/public-ingress-gateway/feature-delta.md`
reports **11 outcomes checked, 0 collisions**. The checked set is nonzero; no
unresolved outcome collision is carried into DISTILL.
