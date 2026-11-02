# Feature Delta — `public-ingress-gateway`

> **DESIGN status (reconstruction on `main` 549d78fa6, 2026-11-02):
> review round 1 remediated; not approved.** Three items BLOCK design approval
> until the user rules (P-2, P-9, P-10 in the PROPOSED table). Contract-shaping
> validation V-GW-1..5 and the Quint model have
> run (see [Contract-shaping validation items](#wave-design--ref-contract-shaping-validation-items)
> and [Formal protocol model](#wave-design--ref-formal-protocol-model)); the
> optional proposals (P-1, P-3, P-4, P-6, P-8 simplification) await the user. This document reconstructs the previously
> user-ratified (2026-09-13) `public-ingress-gateway` DESIGN on current `main`.
> Unchanged decisions keep their 2026-09-13 ratification provenance; the
> decisions changed by the user rulings D1–D5 of 2026-11-01 (table in
> [User decisions](#wave-design--ref-reconstruction-user-decisions-and-proposals))
> are recorded as the current decisions and are not yet independently reviewed.
> Nothing here is marked accepted. Interaction mode: PROPOSE. Density: lean,
> Tier-1 `[REF]` only.
>
> Current-main premises (verified at `main` 549d78fa6, cited `M:path:line`):
> VM workloads are the only workload surface; each allocation owns an
> address in the guest prefix `100.95.0.0/16` and is reached through leg-C →
> kTLS → owner-opened vsock flow (ADR-0145, ADR-0173, ADR-0186, ADR-0190); the
> composition root passes every driven adapter into `run_server`
> (ADR-0219, `NodeAdapters`/`BootAdapters`); the metal host is the only Tier-3
> surface for VM behaviour. VM backends reach `SERVICE_MAP`/`BACKEND_MAP`/
> `REVERSE_NAT_MAP` through the ordinary `ServiceMapHydrator` path — proven on
> native metal Linux 7.0.0-38 by
> [the D4 findings](spike/findings-vm-service-map-programming.md) — so the
> gateway adds no backend-programming mechanism. System ADR-0220, 0223, 0224, 0229, 0230, 0234,
> Route ADR-0221, Domain ADR-0225/0228/0241 and Application
> ADR-0222/0226/0227/0231–0240, plus ADR-0242 and ADR-0243, pin the downstream
> choices; exact API contracts live only in the `[REF]` sections below. The
> complete cross-owner flow remains `AT-PIG-E2E-1`, a production Tier-3
> acceptance boundary on native metal.

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
| Current backend publication/hydration path | ✓ Re-read on `main` 549d78fa6: `service_lifecycle.rs:888-889, 1064-1068`, `service_map_hydrator.rs:349-414`, action shim `mod.rs:1466-1472, 2455-2458`, `EbpfDataplane::update_service` (`lib.rs:1771-2042`). A guest-prefix (`100.95.0.0/16`) VM backend is a remote survivor programmed into `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP`; D4 proved it on metal. `BackendId` is allocated in the Dataplane and not recycled within a process (`allocators/backend_id.rs:58-79`) |
| Current packet-entry and selected-peer path | ✓ Re-read: `cgroup_connect4_service` (reads only `LOCAL_BACKEND_MAP`, `:78`), leg-C inbound (`mtls/inbound.rs:56-90`), leg-B dial contract (ADR-0190, `mtls/mod.rs:946-997`), the authn-only verifier (`tls_config.rs:116-161`), `ResolvedBackend.expected_svid = None` (`traits/mtls_resolve.rs:26,101,225,274`): the attach path is `/sys/fs/cgroup/overdrive.slice` (`dataplane/lib.rs:119`), so the hook is attached over `serve`'s cgroup; gateway cookie receipt, non-allocation SVID use, the exact-peer equality and the gateway-client entry are decisions of this design |
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
| **Implemented fact** | The gateway surfaces are absent, but accepted foundations exist: `ServiceMapHydrator` programs a guest-prefix VM backend into `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` (D4, metal 7.0.0-38); `cgroup_connect4_service` is attached over `overdrive.slice` and today selects only from `LOCAL_BACKEND_MAP`; shared Service maps, leg-C kTLS/vsock delivery and rustls chain/single-SPIFFE-SAN verification exist; the reserved `expected_peer`/mismatch contract is not exercised because current production supplies `None`; allocation-keyed identity cannot represent the gateway unchanged. |
| **Whitepaper** | Historical illustration only. It establishes no binding API, owner, ordering or traffic-path contract. |
| **This SYSTEM proposal** | One `serve`-owned working gateway: IPv4-A/TCP/443 public TLS/HTTP; Public Route Set resolves exact Service Frontend; the dataplane adapter prepares a marked, loopback-bound upstream socket and registers its socket cookie against that frontend, then the gateway connector's `connect(2)` enters the attached `cgroup_connect4_service` hook; the hook selects from the existing `SERVICE_MAP` Maglev inner table and `BACKEND_MAP`, rewrites the destination, and records the actual `BackendId`; the rewritten dial enters leg-C; Dataplane joins that receipt to the exact applied `Backend.alloc`; gateway-held SVID mTLS pins that identity; no userspace selection. |
| **Later DESIGN resolution** | Route ADR-0221 owns only the singleton Public Route Set. Domain ADR-0223/0225/0228/0241 own custody/identity/receipt/applied-identity decisions. Application ADR-0222/0226/0227/0231–0240 separately own Route/application admission, preserve-old replacement, receipt, identity publication, exact-peer mTLS, public TLS/HTTP/routing/header/limit/streaming/attempt policy and operator status; ADR-0242 and ADR-0243 own backend unavailability and upstream-socket preparation. Exact schemas, signatures, config, records/codecs, errors and crate/module contracts live only in the `[REF]` sections of this document. SYSTEM copies use those names without redefining them. |

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
  the upstream connector hands the same frontend to the BPF-owned selection
  path without enumerating candidates. ServiceMapHydrator programs that
  frontend's backends through its ordinary path; the gateway publishes nothing
  to it.
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
| **A. Registered arm in the cgroup/connect4 Service dataplane (selected; arm placement beside or inside the existing program is P-10)** | The dataplane adapter creates a marked, loopback-bound socket in the `overdrive serve` task, reads its stable kernel cookie and registers `(cookie, exact ServiceKey)`. The connector's `connect(2)` enters the attached `cgroup_connect4_service`; its gateway-intent arm indexes the existing `SERVICE_MAP` Maglev inner table by a cookie-derived slot, resolves `BACKEND_MAP`, rewrites the destination, and writes a cookie-keyed selected/`NoBackend` outcome. The rewritten dial enters leg-C by the ADR-0190 route. | Reuses the attached hook and the ordinary Service-map programming D4 proved. The intent entry makes the gateway arm explicit and fail-closed while every unregistered connect retains today's `LOCAL_BACKEND_MAP` behavior. | Adds transient intent/receipt maps (and, only if P-9 honours it, one byte read of `BACKEND_MAP.healthy`), and a Dataplane-owned applied-identity association; no cgroup, netns, veth, process, or userspace LB. | **Selected** |
| **B. Dedicated gateway child cgroup + specialized connect4 branch** | Move a connector thread into a threaded child cgroup and recognize the child cgroup id before shared-map selection. | BPF still owns selection and a receipt can identify the peer. | Adds threaded-cgroup creation/migration/restart semantics although a socket cookie already scopes the gateway connect; no security or routing benefit over A. Becomes the fallback design only if V-GW-2 shows `serve` is outside the attach subtree. | Viable, rejected as dominated by A |
| **C. Gateway dials the VIP and relies on XDP/TC forwarding** | Route gateway upstream packets by an interface into `xdp_service_map_lookup`; add flow-selection receipt for peer identity. | Literal XDP path and existing wire algorithm. | The VM backend is reached through leg-C, not by an L3 forward to the guest (ADR-0145, ADR-0173); XDP selection would bypass the mTLS terminator and needs a new interface and flow correlation solely to reach an ingress hook. | Rejected |
| **D. Userspace backend selection** | Gateway watches `ServiceBackendRow` and round-robins. | Makes identity available before connect. | Forbidden second LB; duplicates ServiceMapHydrator/BPF choice and can diverge. | Rejected by constraint |

**Decisions D-SYS-6/D-SYS-7:** select Option A as the smallest working packet-
entry topology; [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md)
owns that choice. Separately, D-SYS-3/[ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md)
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
  Gateway listeners/handlers never read manual file paths or raw PEM. ADR-0223
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
- **D-SYS-6 — BPF-owned internal traffic:** the dataplane adapter creates an
  IPv4 TCP socket in the `overdrive serve` task (inside the cgroup the hook is
  attached over), marks it `MTLS_TPROXY_MARK`, binds `127.0.0.1` (ADR-0190),
  obtains its kernel socket cookie and registers only `(cookie, exact
  ServiceKey)` ([ADR-0243](../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)).
  The gateway connector then calls `connect(2)` on the exact Service Frontend.
  The already-attached `cgroup_connect4_service` checks the registered intent,
  indexes the existing `SERVICE_MAP` Maglev inner table using a deterministic
  hash of that cookie, resolves `BACKEND_MAP`, rewrites to the selected VM
  backend address, and writes the selected `BackendId` outcome before allowing
  the connect. `NoBackend` (no outer entry or no usable backend; the
  `healthy == 0` case is pending P-9) is
  recorded and denied rather than falling through to an unchanged VIP. A socket
  without a registered gateway intent follows today's `LOCAL_BACKEND_MAP`
  hit/miss behavior unchanged. The gateway never preselects a backend or
  touches a raw BPF map.
  The rewrite occurs at `connect(2)` before the route lookup; the marked,
  loopback-sourced dial leaves through table 100 to `lo`, the registered-tuple
  TPROXY rule diverts it to the node-shared leg-C listener, and leg-C
  terminates the gateway's TLS leg and relays plaintext to the guest over the
  owner-opened vsock flow (ADR-0145/0173/0186/0190). The response returns on that same
  authenticated socket. The XDP programs are not the selection hook for this
  host-originated gateway connect. No new netns, veth, route or nft rule is
  added.
- **D-SYS-7 — Backend authority/application:** ServiceLifecycle remains the
  sole complete `ServiceBackendRow`/eligibility publisher. The existing
  ServiceMapHydrator consumes those observations and programs the VM backend
  into `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` on its ordinary path
  (D1; D4 evidence); the gateway supplies the hydrator nothing. Because
  `EbpfDataplane` assigns `BackendId`, it also owns the
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
  `NoBackend`—an empty eligible set or no selectable BPF backend (a selected
  backend with `healthy == 0` joins it only if P-9 is adopted)—is the only 503 path and the typed
  outcome of the gateway's consuming port, with no readiness wait
  ([ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)). A
  `Selected` receipt whose applied identity is missing or mismatched, and every
  internal connect/receipt/mTLS/upstream failure, is 502. `GatewayControl`
  separately exposes active application/custody and
  relevant BPF hydration status as `GatewayStatusResponse` over operator-mTLS
  `GET /v1/gateway/status` (200 when readable, 500 on observation read error).
  receipt [ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md),
  selected identity [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md)
  and the Application HTTP contract own the exact cause/error mapping.
- **D-SYS-11 — Restart/drain:** boot rebuilds ServiceMapHydrator state and the
  Dataplane applied-id association; the gateway gate task then sweeps the
  previous process's residue and runs the sentinel-ServiceKey `NoBackend`
  receipt probe before Gateway Identity is Current; that probe proves
  only the registered-connect/hook/receipt bind capability and never attempts
  mTLS. After reissuing/holding the gateway SVID, the actual Service Frontend
  probe treats `NoBackend` as a successful 503-capable result, while `Selected`
  must complete exact-peer mTLS before Gateway Application is atomically
  promoted and TCP/443 binds.
  Shutdown runs the gateway stage first in `ServerHandle::shutdown`: stop
  accepts, drain admitted requests within the one `drain_deadline`, join
  connector/listener tasks, clear per-socket receipts, retire the closed
  application and clear the volatile Gateway SVID slot, and only then run the
  existing stages (the gateway stage runs before the leg-C worker stage because
  admitted connections ride leg-C; D5 section (c)). No HA.

## Wave: DESIGN / [REF] C4 System Context

The canonical Level-1 view is maintained once in
[Public ingress gateway canonical C4](../../product/architecture/c4-diagrams.md#public-ingress-gateway-canonical-c4).
It shows the external user and operator boundaries, operator-managed certified-
key files, public IPv4 DNS/TCP-443 reachability, the embedded Overdrive node and
the identity-unaware Service workload. System ADR-0220/0223/0224/0229/0230/0234 own the node,
credential-consumption, packet-entry, consistency, trust and listener-gate
choices; this feature delta does not duplicate the diagram source.

## Wave: DESIGN / [REF] C4 Container

The canonical Level-2 and Level-3 views are maintained at the same
[canonical C4 anchor](../../product/architecture/c4-diagrams.md#public-ingress-gateway-canonical-c4).
They are the sole Mermaid source for the `overdrive serve` container,
IntentStore/ObservationStore ownership, `GatewayApplicationOwner`,
`PublicCertifiedKeyCustody`, the ordinary ServiceMapHydrator/Dataplane,
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
| Public Certified Key current custody generation | `IntentStore` `PublicCertifiedKeyV1`, including `ProtectedOriginKeyV1` sealed by `PublicCertifiedKeyAeadCodec` | `PublicCertifiedKeyCustody` is sole writer/validity owner | `ManualCertifiedKeySource` paths/raw PEM stop before custody; public-key secrets never enter status, Route, handler, CLI or logs |
| Public Certified Key status | `ObservationStore.public_certified_key_status` / `PublicCertifiedKeyStatusRowV1` | `PublicCertifiedKeyCustody` sole LWW writer after Current publication | Redacted active/usability/install-failure facts only; status failure retries without rolling back Current; no paths, PEM, DER, key or ciphertext |
| Gateway SVID | Existing internal CA + issued-certificate audit plus volatile `GatewayIdentitySlot` | `GatewaySvidLifecycle` + control-plane action executor + `GatewayIdentitySlot` | Separate from allocation `IdentityMgr` and Public Certified Key; only `HostGatewayClientMtls` may use private material; no fabricated AllocationId |
| Eligible workload backend set and `Backend.alloc` | `ServiceBackendRow` | ServiceLifecycle, sole complete-row publisher | Gateway does not read, probe, author, filter or select this set |
| Applied Service frontend backend set | `SERVICE_MAP` outer + Maglev inner table + `BACKEND_MAP` | ServiceMapHydrator → Dataplane/EbpfDataplane (ordinary path, D1) | The gateway reads it only through the hook's registered arm; handler never mutates maps |
| Gateway Applied Generation | Active `GatewayApplicationOwner` Current `ArcSwapOption`, built from Route + exact Service Frontend + opaque certified-key snapshot | `GatewayApplicationOwner` | Atomic admission pointer retained per connection; excludes BPF generation/candidates and is not reconstructed from status |
| Gateway Application status | `ObservationStore.gateway_application_status` / `GatewayApplicationStatusRowV1` keyed by `NodeId` | `GatewayApplicationOwner` sole LWW writer | Listener/staged/current/draining/unavailable/non-secret identity/connect-path facts only; no BackendId, receipt, peer, BPF generation or credential secret |
| Upstream socket preparation and registered intent | Process-local `GATEWAY_CONNECT_INTENT_MAP` entry, keyed by the socket cookie | `EbpfDataplane` (`open_upstream`) | Never the gateway crate; never persisted; destroyed with the BPF object |
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
| Gateway application/listener/connector/identity tasks | One `overdrive serve` process and `ServerHandle` lifecycle; there is no independent gateway supervisor. An owned-task exit stops new public admission or fails its request cause-distinctly; a `serve` restart reconstructs custody/application/BPF/identity prerequisites before rebinding. A gateway task failure or failed gate never requests serve shutdown (only the shared guest-network fail-stop does). |
| Kernel Service dataplane | Existing `EbpfDataplane` link/map owner. Load failure or a missing/malformed boot-sentinel receipt refuses initial bind; a canonical sentinel `NoBackend` proves the registered-connect/hook/receipt path without gateway identity or mTLS. For the actual frontend, canonical `NoBackend` is a successful bind-capability result and the distinct public 503 path; a missing/invalid outcome closes the socket and returns public 502 without bypass. |
| Workload allocation / leg-C terminator / vsock flow owner | Existing WorkloadLifecycle, node-shared leg-C and guest-flow owners. Their failure changes existing workload/backend facts and eventually BPF hydration; a dial the owner cannot serve is reset or refused and returns public 502; it never transfers health ownership to the gateway. |
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

- **Existing evidence:** the operator listener and the node composition are
  owned by `boot_server` (`M:lib.rs:3625-4616`); the Service backends are
  programmed by the ordinary hydrator path (D4); the gateway gate runs after the
  operator listener task is spawned and after convergence is live (G4 of
  [boot](#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5)).
  No accepted ADR makes the public listener a prerequisite of any existing
  state.
- **Owner:** the gateway gate task owned by `GatewayApplicationOwner`
  (constructed by the `serve` composition root).
- **Promise:** Public Certified Key usable, Route/frontend application
  candidate valid, `gateway_connect.probe` (residue sweep plus the sentinel
  canonical `NoBackend` receipt) proved the registered-connect/hook/receipt
  capability, and Gateway Identity is Current. The actual Service Frontend
  probe then returned either bind-valid `ReadyNoBackend` or, for `Selected`,
  `SelectedPeerAuthenticated`; TCP/443 may now bind. Backend presence is not a
  listener prerequisite and the gate waits for none.
- **Affected state:** public listener bound/admission eligibility only.
- **Failure projection:** missing/malformed sentinel or actual-frontend receipt,
  probe timeout, connect-path failure, or a `Selected` result that cannot
  resolve identity or complete exact-peer mTLS keeps TCP/443 unbound and is
  cause-distinct on operator-mTLS gateway status, with a structured
  `health.startup.refused` event naming the gateway reason; malformed or
  duplicate, cancelled and disconnected outcomes map to the same unbound state.
  Actual-frontend `NoBackend` alone never blocks bind; the active listener
  serves 503 for that result.
- **Explicitly unaffected:** allocation `Running`, Service `Stable`, backend
  eligibility, BPF hydration, and operator HTTPS meaning.
- **Ordering:** custody → coherent Route/frontend application candidate →
  `gateway_connect.probe` (permitted before gateway identity) → gateway SVID
  Current → actual Service Frontend probe. That probe either stops
  successfully at `ReadyNoBackend`, or its `Selected` branch resolves the
  applied identity and completes exact-peer mTLS only after Gateway Identity
  Current → promote the admission object → bind TCP/443. The whole gate shares
  one `application_gate_deadline`; no step restarts it.
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
  publish Gateway Applied Generation. The previous Current becomes Draining
  and is retired only after every connection retaining it drains. BPF
  hydration remains independently generated: a new request that races its
  convergence receives `NoBackend`→503 rather than a userspace fallback.
- **Counterexample:** separate Route/key publication could admit a torn
  connection; the single whole-object swap prevents it.
- **Evidence lane:** pure/property object construction, seeded coherent-
  generation invariant, concurrent hot-swap integration.

### Gate G-4 — Gateway BPF selection receipt established

- **Existing evidence:** ServiceMapHydrator consumes `ServiceBackendRow` and
  programs a guest-prefix VM backend into `SERVICE_MAP`/`BACKEND_MAP`
  (`M:service_map_hydrator.rs:349-414`; D4 on metal); `cgroup_connect4_service`
  reads only `LOCAL_BACKEND_MAP` (`M:cgroup_connect4_service.rs:78`); the
  `ServiceBackendRow` → `update_service` → hook chain has no gateway entry
  today.
- **Owner:** `EbpfDataplane` for applied maps, the registered intent and the
  `BackendId`→exact applied `Backend.alloc` association (it prepares the
  socket, ADR-0243); the gateway connector owns the `connect(2)` call; the
  attached cgroup hook owns this socket's selection/rewrite.
- **Promise:** exact Service Frontend was BPF-selected/DNATed and the receipt
  identifies its actual `BackendId`; Dataplane resolves that non-recycled id to
  the exact applied workload identity before returning the connected socket.
- **Affected state:** current internal socket outcome only.
- **Failure projection:** unresolved frontend prevents application. Only the
  cgroup-BPF `NoBackend` receipt — empty set or no selectable backend (plus a
  selected backend with `healthy == 0` only if P-9 is adopted) — yields 503 (typed outcome, no wait,
  ADR-0242). A `Selected` receipt with missing or mismatched applied identity,
  or any hook/map/receipt inconsistency, duplicate or disconnected receipt,
  cancellation, or connect failure (including the reset of a stopped backend's
  unregistered tuple), yields typed 502 and never pass-through.
- **Explicitly unaffected:** Gateway Applied Generation, allocation `Running`,
  Service `Stable`, backend eligibility, and BPF generation ownership.
- **Ordering:** the hydrator's ordinary application is independent and earlier
  or later than the request. Per request: dataplane `open_upstream` (mark,
  loopback bind, `SO_COOKIE`, register) → connector `connect(2)` enters the
  already-attached hook → hook validates the intent, selects/rewrites or records
  `NoBackend` and denies → connector consumes the receipt → on `Selected`
  Dataplane takes its commit guard to read the identity; the read waits for any
  in-flight `update_service` and can observe only Applied → connector always
  removes the intent before it returns fd+identity or error. Inside
  `update_service`: validate and reserve each BackendId→`Backend.alloc`
  association as non-readable before BPF work → fallible BACKEND/private-inner/
  reverse-NAT work → the outer Service-map pointer swap commits the selectable
  generation → infallibly publish the reservations Applied and release the
  guard. Any pre-commit failure restores prior state and exposes no identity
  from the abandoned reservations.
- **Counterexample:** reading `ServiceBackendRow` and selecting a backend in the
  HTTP handler could disagree with Maglev/SERVICE_MAP and is the forbidden
  second load balancer.
- **Evidence lane:** pure selection/receipt association, seeded generation-
  consistency, Tier-3 hook/packet/receipt proof and `AT-PIG-E2E-1`.

### Gate G-5 — Gateway-SVID selected-peer mTLS authenticated

- **Existing evidence:** `Backend.alloc` exists in the ServiceLifecycle input
  from which BPF state is hydrated; the client verifier is authn-only and
  `ResolvedBackend.expected_svid` is `None` (`M:tls_config.rs:116-129`,
  `M:traits/mtls_resolve.rs:26,101,225,274`); leg-C presents the server SVID of
  the allocation that owns the dialed guest address and only verifies that the
  client SVID chains to the root (`M:tls_config.rs:249-269`). BPF—not the
  gateway—selects the backend, and the current mTLS client credential surface
  is allocation-keyed.
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

### Gate G-6 — Gateway shutdown stage precedes the leg-C worker stage

- **Existing evidence:** `ServerHandle::shutdown` order at
  `M:lib.rs:2890-2993`: convergence → emit_drain → interest_router → axum →
  server_task → exit_observer → shared_network_supervisor → mtls_worker_owner →
  guest_flow_owner → mtls_resolve_owner. Admitted gateway connections ride leg-C,
  which the `mtls_worker_owner` stage tears down.
- **Owner:** `ServerHandle::shutdown`, stage `gateway`, calling
  `GatewayHandle::shutdown`.
- **Promise:** when the stage returns, no public accept is possible, every
  admitted connection has ended or been force-joined, the intent and receipt maps
  hold nothing, the closed application is retired, and the volatile Gateway
  Identity slot is cleared (synchronously; no reconciler round-trip) or the
  exact typed failure is in the report.
- **Affected state:** gateway runtime state and the Gateway Identity slot only.
- **Failure projection:** a non-clean `GatewayShutdownReport` is appended to
  `ServerShutdownError` after every owner has been attempted; a gateway timeout
  consumes only the shared `drain_deadline`; an outer fail-stop bound abandons
  the stage and the process-local state dies with the process.
- **Explicitly unaffected:** operator HTTPS draining, workload and allocation
  state, the relative order of the existing stages, fail-stop exit status.
- **Ordering:** gateway stage → existing stages, unchanged. Leg-C and the
  guest-flow owner outlive every gateway upstream connection.
- **Counterexample:** running the gateway stage after the leg-C worker stage
  would cut admitted connections mid-request instead of draining them.
- **Evidence lane:** integration over the production `ServerHandle` asserting
  the ordered `serve.shutdown.stage_begin/stage_complete` events, and a
  native-metal graceful-shutdown run. Not a Quint protocol.

### Required boundary scenarios

DISTILL covers available/success, unavailable or
timeout with cause-distinct failure, explicitly unaffected lifecycle progress,
late success after supersession, disconnect/reconnect or restart/replay, and
feature-disabled behavior, for the affected and the declared-unaffected states
of every gate:

1. **Gate available:** G-1 binds and the first request returns the backend body;
   G-4 returns `Selected`; G-5 delivers bytes.
2. **Gate unavailable or timed out:** an absent sentinel receipt, a hook that
   does not fire, an identity that never becomes Current, and a probe timeout
   each leave TCP/443 unbound with the cause on status; no outer entry or no
   usable backend is 503 (the `healthy == 0` window is pending P-9: 503 if
   honoured, 502 if ignored); an unreachable guest is 502.
3. **Unrelated state:** allocation `Running`, Service `Stable`, hydration and the
   operator listener advance identically with the gateway enabled, disabled or
   gated.
4. **Late success:** a stale probe or application result after supersession and
   a late `Selected` after a newer generation never restore the old admission
   object.
5. **Disconnect/reconnect:** a guest-flow owner restart or fail-stop during an
   in-flight upstream request ends it with 502 and leaves no intent or receipt.
6. **Feature disabled:** `ServerConfig.gateway == None` binds nothing, prepares
   no socket, runs no sentinel and leaves the cgroup hook's unregistered
   behaviour byte-for-byte unchanged.

Ordering-dependent behavior requires seeded
`overdrive-sim` invariants; socket, public Web PKI, TPROXY, kTLS, vsock and wire
effects require the built default-feature binary on the native-metal
lane. A test CA is permitted only as the hermetic TLS mechanism and does
not satisfy the production public-trust claim.

Because the selected design adds a registered selection arm to the cgroup
connect4 dataplane (placement: P-10), DELIVER must run the repository's real-kernel verifier-regression and verifier-
budget gates plus the existing cgroup-connect performance lane. Any baseline
change requires the measured kernel record and review mandated by `bpf.md`; the
design does not pre-authorize a numeric bump. The new selection executes once
per upstream socket connect, not once per HTTP body chunk.

## Wave: DESIGN / [REF] Component decomposition

These are responsibilities, not approved Rust types or module names.

| System responsibility | Likely existing owner/path | Change | Boundary handed forward |
|---|---|---|---|
| Public listener | `GatewayApplicationOwner` / `GatewayHandle` under `boot_server` / `ServerHandle` (D5) | EXTEND | Dynamic TCP/443 bind only after all gateway/dataplane/identity prerequisites; owned accept-stop/drain/unbind |
| `PublicCertifiedKeyCustody` | `overdrive-gateway::certified_key` + `overdrive-host::ca::PublicCertifiedKeyAeadCodec` | CREATE NEW stable custody owner over existing KEK/AEAD mechanism | `ManualCertifiedKeySource` installs; custody validates/builds+seals, persists `PublicCertifiedKeyV1`, publishes Current/Usable, then writes `PublicCertifiedKeyStatusRowV1`; paths/raw PEM/secrets stop before Route/listener/status |
| Public TLS + HTTP/1.1 runtime | Existing rustls/hyper libraries | CREATE NEW in-process runtime | Active after prerequisite gate; bounded streaming/drain |
| `GatewayApplicationOwner` | `PublicRouteSetV1` + `IntentServiceFrontendResolver` + custody | CREATE ACTIVE OWNER | Atomic admission object retained per connection; sole writer of `GatewayApplicationStatusRowV1` |
| Eligible backend publication | ServiceLifecycle → `ServiceBackendRow` | REUSE UNCHANGED AS INDIRECT AUTHORITY | Gateway is not a consumer; ServiceMapHydrator remains the downstream consumer |
| BPF backend application | ServiceMapHydrator → Dataplane/EbpfDataplane | REUSE UNCHANGED (D1) | VM backends already enter `SERVICE_MAP` Maglev inner + `BACKEND_MAP` + `REVERSE_NAT_MAP`; the applied BackendId→`Backend.alloc` association is added inside Dataplane ownership |
| BPF selection and receipt | `cgroup_connect4_service` + `EbpfDataplane` | EXTEND (D3) | Registered socket-cookie intent/outcome selection in a `cgroup/connect4` program (placement: second program beside the existing hook or added arm, P-10); the dataplane adapter owns socket preparation (ADR-0243) |
| `GatewayIdentitySlot` + `GatewaySvidLifecycle` | Existing CA/`issue_and_audit`/identity patterns | CREATE NEW single-slot sibling and pure lifecycle | Own issue/audit/hold/use/reissue/restart/drain-ordered drop for gateway subject; only `HostGatewayClientMtls` receives the private use snapshot; never AllocationId |
| `GatewayUpstreamConnector` + `GatewayConnectDataplane` | Existing attached cgroup hook, `SERVICE_MAP` Maglev inner/`BACKEND_MAP`, `HostGatewayClientMtls`, leg-C | CREATE connector/registry; EXTEND hook | The dataplane adapter prepares the socket and registers only cookie+ServiceKey; BPF selects; Dataplane supplies actual applied identity; connector transfers fd+SpiffeId into exact-peer mTLS; no userspace LB |
| Gateway status | `GatewayControl` + exact ObservationStore point reads | EXTEND existing operator-mTLS `GET /v1/gateway/status` | Projects `GatewayApplicationStatusRowV1`, `PublicCertifiedKeyStatusRowV1`, non-secret identity/connect facts and relevant hydration into `GatewayStatusResponse`; 500 on observation read error |

## Wave: DESIGN / [REF] Driving ports

| Driving surface | System responsibility | Initial constraint |
|---|---|---|
| `overdrive serve` | Compose/probe prerequisites, bind TCP/443, own runtime/drain | Existing production entry point |
| `overdrive deploy <SERVICE_SPEC>` | Produce the existing Service/listener/workload intent and resulting ServiceLifecycle backend observations | Existing surface unchanged |
| `overdrive deploy <ROUTE_SPEC>` | Produce `PublicRouteSetV1` under [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md) through the [exact Application driving-port contract](#wave-design--ref-exact-driving-and-driven-ports) | Required operator journey; workload deploy arms remain unchanged |
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
| Gateway BPF connect/receipt | Extended existing cgroup/connect4 + Dataplane-owned cookie intent/outcome and applied-id association (`open_upstream`) | Prepare the marked loopback-bound socket and register exact ServiceKey, select/rewrite from existing maps, consume actual selected identity; no raw handler map access |
| Public TLS/HTTP | rustls/hyper | TLS 1.3 + bounded HTTP/1.1 |
| Gateway SVID lifecycle/hold/use | `GatewaySvidLifecycle` + action executor + `GatewayIdentitySlot` | `ensure_current` before bind, restart/near-expiry reissue, and `disable_after_drain`; no public/private-material getter |
| Selected-peer mTLS | `HostGatewayClientMtls` implementing `GatewayClientMtls` | Present `GatewayIdentitySlot` SVID and require exact receipt-selected workload SAN; transparent kTLS/`MtlsEnforcement` is not this L7 client's API |
| Clock/entropy | Existing injected `Clock` plus `PublicCertifiedKeyAeadCodec` ring `SystemRandom` | ADR-0223/0231, ADR-0225/0234 and ADR-0226/0235–0240 own the custody, SVID/mTLS and public-runtime choices; exact clock/entropy contracts are in [Application persistence/custody](#wave-design--ref-persistence-custody-and-redaction), [Gateway Application/status/HTTP](#wave-design--ref-gateway-application-status-and-http-contract), and [gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration) |

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
| `run_server` / `boot_server` / `ServerHandle` / `NodeAdapters` | `crates/overdrive-control-plane/src/{lib.rs,node_adapters.rs}`, ADR-0219 | Composition, probes, task ownership, bounded shutdown, driven-adapter bundles | **EXTEND** | The gateway is node infrastructure in the same process/failure domain; a second supervisor duplicates lifecycle ownership. Its driven adapters enter through `BootAdapters.gateway` and `ComposedDataplane.gateway_connect` (D5), never an override seam. |
| `Kek`/typed-codec protection pattern | ADR-0063/0048, `overdrive-host::ca` | Protect canonical secret input | **EXTEND per [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md)** | Exact `PublicCertifiedKeyAeadCodec`/record contracts live in [Application persistence/custody](#wave-design--ref-persistence-custody-and-redaction); the private AEAD engine is reused with distinct KEK/HKDF/AAD and public Web-PKI remains separate from both private CAs. |
| `IdentityMgr` / `IdentityRead` | ADR-0067 | Allocation-keyed holder pattern | **REUSE PATTERN, CREATE SEPARATE GATEWAY SLOT** | Avoids corrupting AllocationId API while reusing locking/redaction/bundle discipline |
| `ServiceLifecycle` / `ServiceBackendRow` | ADR-0101, `service_lifecycle.rs` | Backend membership and eligibility | **REUSE AS INDIRECT AUTHORITY** | Sole health publisher; gateway is not a backend-row consumer. |
| `ServiceMapHydrator` | ADR-0042/0101, `service_map_hydrator.rs:349-414` | Sole backend application owner | **REUSE UNCHANGED** | VM backends already reach `SERVICE_MAP`/`BACKEND_MAP` (D1; D4 evidence). The gateway adds no input, no field and no action variant; `Action::DataplaneUpdateService` is not extended. Contract shape: unbounded-preservation of the hydrator's state-delta universe. |
| `ListenerFactStore` | ADR-0062 | ServiceId-to-listener projection and workload cleanup index | **DO NOT EXTEND** | [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md) owns Route/application resolution; the [exact driven-port contract](#wave-design--ref-exact-driving-and-driven-ports) pins `IntentServiceFrontendResolver` over `WorkloadIntent` + `ServiceVipView`, because ListenerFactStore has the wrong derived key/absence semantics. |
| mTLS `client_config`/SPIFFE verifier + reserved `expected_peer` contract | ADR-0069/0071, `mtls/tls_config.rs:68-222`, `mtls_enforcement.rs` | rustls chain validation, one-SPIFFE-URI parsing, required intended-peer shape | **REUSE VERIFIER BUILDING BLOCKS; CREATE EXACT-PEER VERIFIER AND GATEWAY-CLIENT ENTRY UNDER THE SAME OWNER** | Current production is authn-only (`SpiffeServerVerifier` ignores the destination, `tls_config.rs:116-129`; `expected_peer=None`) and the public `MtlsEnforcement` port requires an intercepted leg + `AllocationId`. The L7 gateway must not fabricate either or route through the transparent kTLS pump; its client entry in `mtls/gateway_identity.rs` consumes gateway-held material + receipt-selected identity and adds SAN equality over the reused `root_store`/`present_chain`/`validate_single_spiffe_uri`. Contract shape: bounded-change (declared mutation set: none; pure verification decision), assertion by a wrong-valid-peer negative. |
| `rustls` + `hyper` server experience | `tls_bootstrap`, control-plane listener | TLS config/server serving/drain mechanics | **REUSE LIBRARIES/PATTERN** | Public cert trust/authority and HTTP proxy behavior are new; the operator CA/router are not reusable identities or handlers. |
| `cgroup_connect4_service` + shared Service maps | ADR-0053/0040, `cgroup_connect4_service.rs:49-102` | Host connect-time destination rewrite; the attach path is `overdrive.slice` | **EXTEND: REGISTERED ARM + RECEIPT** | Cookie-scoped intent selects the new Service-map arm; unregistered sockets preserve current `LOCAL_BACKEND_MAP` behavior (contract shape: unbounded-preservation of the unregistered universe, asserted by a byte-equal unregistered-connect complement); no child cgroup/netns/XDP entry is needed unless V-GW-2 falsifies that `serve` is inside the attach subtree. |

Every CREATE NEW row in component decomposition exists because no current owner
has that responsibility. No new deployed process or datastore is justified;
`PublicCertifiedKeyV1` and the two status tables extend the existing
IntentStore/ObservationStore.

## Wave: DESIGN / [REF] Decisions table

| ID | Locked system-level verdict | Owning decision record(s) |
|---|---|---|
| D-SYS-1 | `serve` owns working in-process gateway, connector, listener and drain | [ADR-0220](../../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) |
| D-SYS-2 | IPv4-A/TCP/443 binds after initial application/BPF/identity prerequisites | [ADR-0230](../../product/architecture/adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md) |
| D-SYS-3 | `PublicCertifiedKeyCustody` validates/builds+seals, persists complete `PublicCertifiedKeyV1`, atomically publishes the opaque Current/Usable snapshot, then writes `PublicCertifiedKeyStatusRowV1`; `GatewayApplicationOwner` atomically publishes Route/frontend/resolver admission and writes `GatewayApplicationStatusRowV1`; ACME #57 later uses the same install boundary | Custody and acquisition boundary [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md); replacement [ADR-0231](../../product/architecture/adr-0231-preserve-last-usable-public-certified-key-replacement.md) |
| D-SYS-4 | Public Web-PKI certified key, operator HTTPS CA, and internal workload/gateway SPIFFE CA remain three distinct credential domains | [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md), [ADR-0234](../../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md) |
| D-SYS-5 | Dedicated gateway identity lifecycle/holder owns non-allocation SVID issue/hold/use/reissue/drop | System trust [ADR-0234](../../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md); realization [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md) |
| D-SYS-6 | The dataplane adapter prepares the marked loopback-bound socket and registers `(SO_COOKIE, ServiceKey)`, then a cgroup/connect4 registered arm (placement: P-10) selects/rewrites through `SERVICE_MAP` Maglev inner + `BACKEND_MAP` and records actual BackendId | Packet entry and selection [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md); receipt [ADR-0232](../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md); socket preparation [ADR-0243](../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md) |
| D-SYS-7 | ServiceLifecycle publishes eligibility; ServiceMapHydrator programs VM backends on its ordinary path (D1); under the Dataplane guard identity is Reserved before BPF work, the outer swap commits selection, then identity becomes Applied; gateway has no backend watch/cursor and no demand channel | [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0233](../../product/architecture/adr-0233-commit-gated-backend-identity-publication.md), [ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) |
| D-SYS-8 | Gateway Applied and BPF applied generations remain independent; the transient receipt pins actual BackendId and its identity read blocks on the commit guard until the process-lifetime association is Applied | [ADR-0229](../../product/architecture/adr-0229-keep-gateway-and-service-dataplane-generations-independent.md) |
| D-SYS-9 | Strict HTTP/1.1; one BPF-owned frontend handoff; no retry/replay/userspace LB | HTTP [ADR-0235](../../product/architecture/adr-0235-public-http11-only.md); single attempt [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md); packet entry [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md) |
| D-SYS-10 | Public 404 Route miss; only `NoBackend`, including an empty eligible set/no selectable BPF backend, is 503; selected-receipt identity missing/mismatch and connect/receipt/mTLS/upstream failures are 502; separate operator-mTLS `GatewayControl` status reads active application/custody/hydration observations as `GatewayStatusResponse` 200/500 | Route [ADR-0236](../../product/architecture/adr-0236-canonical-public-route-match-key.md); attempt/error [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md); status [ADR-0227](../../product/architecture/adr-0227-redacted-operator-gateway-status.md); receipt [ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md); applied identity [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md) |
| D-SYS-11 | Boot sentinel `NoBackend` may prove connect/receipt capability before Gateway Identity Current; actual-frontend `NoBackend` is bind-valid/503-capable, while `Selected` waits for Gateway Identity Current and exact-peer mTLS; graceful drain runs as the first `ServerHandle::shutdown` stage (before the leg-C worker stage), joins connector/listener, clears receipts and clears the volatile identity slot synchronously (D5 section (c)) | [ADR-0230](../../product/architecture/adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md) |

## Wave: DESIGN / [REF] System traceability

| System obligation | Current authority/evidence | Status |
|---|---|---|
| Node-agent-embedded `hyper` + `rustls`, in-process BPF access | GH #54 (no comments) + [ADR-0220](../../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) | Feature intent plus approved single-node/process placement; gateway absent in current production |
| Production public trust from manual Public Certified Key producer | Explicit user ruling + [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md) + [ADR-0231](../../product/architecture/adr-0231-preserve-last-usable-public-certified-key-replacement.md) | `ManualCertifiedKeySource` → `PublicCertifiedKeyCustodyHandle::install` → validate/build+seal → persisted `PublicCertifiedKeyV1` → Current/Usable opaque resolver publication → redacted status |
| Gateway observability | [ADR-0227](../../product/architecture/adr-0227-redacted-operator-gateway-status.md) | `GatewayControl` uses the [exact Application status contract](#wave-design--ref-gateway-application-status-and-http-contract) for ObservationStore point reads of `GatewayApplicationStatusRowV1`/`PublicCertifiedKeyStatusRowV1` and relevant hydration; 200/500, secrets/raw dataplane failure strings omitted |
| ACME acquisition/renewal | Existing GH #57 | Excluded from first slice; additive producer only |
| Route resolves exact Service frontend | ADR-0049/0060/0062 + Route [ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md) + Application [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md) | Existing inputs; approved Route/application resolution boundary (2026-09-13) |
| Backend Eligibility publication | ADR-0101 / ServiceLifecycle | Existing sole owner; unchanged |
| Backend-set application | ADR-0042/0101 + [ADR-0233](../../product/architecture/adr-0233-commit-gated-backend-identity-publication.md) + [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md) + D1 (D4 findings) | ServiceMapHydrator remains sole owner and programs VM backends on its ordinary path; Dataplane records the exact applied identity association |
| Backend selection/DNAT | ADR-0053 cgroup hook + ADR-0040 maps + [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md) + [ADR-0243](../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md) | The attached hook gains a registered gateway-connect arm over the existing Maglev table and a cookie-keyed outcome; the rewritten dial enters leg-C (ADR-0190); XDP wire path remains unchanged |
| Backend unavailability | [ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) | Typed `NoBackend` (503) or connect failure (502) at the connector; no wait, wake or demand |
| Gateway/BPF consistency | [ADR-0229](../../product/architecture/adr-0229-keep-gateway-and-service-dataplane-generations-independent.md) | Gateway Applied and BPF Hydrated remain independently owned; one connect receipt correlates actual selection without a unified generation |
| Gateway SVID and selected-peer verification | ADR-0063/0067/0069/0071 + [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md) + [ADR-0234](../../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md) | `GatewaySvidLifecycle`/`GatewayIdentitySlot` + Dataplane selected-id owner + `HostGatewayClientMtls`; exact signatures live in [Application gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration), with SAN equality and no fabricated Allocation identity/private-material getter |
| Public listener gates | [ADR-0230](../../product/architecture/adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md) | Gateway-owned bind/admission prerequisites never redefine Allocation Running, Service Stable, Backend Eligibility, BPF Hydrated or operator HTTPS readiness |
| Whitepaper §11 | Historical prose | Context only; non-authoritative |

## Wave: DESIGN / [REF] Reconstruction: user decisions and proposals

### User decisions

| Id | Decision | Status | Evidence |
|---|---|---|---|
| D1 | The gateway adds no backend-programming mechanism. Ordinary `ServiceMapHydrator` → `Action::DataplaneUpdateService` → `EbpfDataplane::update_service` programs a VM backend into `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP`. There is no gateway-demand-gated Service-map TEACH. | Approved by user 2026-11-01 (conditional on D4; D4 held) | [D4 findings](spike/findings-vm-service-map-programming.md): native metal 7.0.0-38, `bpftool map dump` shows `10.96.0.2:18080/18081` → backend `100.95.0.2`, `LOCAL_BACKEND_MAP` empty |
| D2 | Backend unavailability is a bounded wait or a typed unavailable outcome at the gateway's own consuming port (the selection-receipt path); no demand, revision, acknowledgment or wake channel exists. [ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md). | Approved by user 2026-11-01 | D4 edge observations (healthy bit lags readiness about 1 s; map entries outlive `job stop`) |
| D3 | The registered gateway selection hook (a `cgroup/connect4` arm that reads `SERVICE_MAP`/Maglev/`BACKEND_MAP` for gateway-registered sockets), the selection receipt, the applied-identity join and exact-peer gateway-client mTLS are kept, pinned to the current leg-C → kTLS → `GuestStreamDial` vsock path and ADR-0190's `SO_MARK` + `127.0.0.1` source requirement. | Approved by user 2026-11-01 | `M:crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:78` reads only `LOCAL_BACKEND_MAP`; `M:crates/overdrive-dataplane/src/mtls/tls_config.rs:116-129` authn-only |
| D4 | Contract-shaping validation of D1 on native metal before design approval. | Held 2026-11-01 | findings above |
| D5 | The architect defines and pins the gateway adapter slot, boot construction/probe/bind point, shutdown order and the upstream-socket-preparation contract (below). | Approved by user 2026-11-01 | [Composition-root gateway slot, boot and shutdown](#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5) |

### Resolved without user ruling (rule-forced corrections or removals, review round 1)

1. **Gateway SVID reconciler wakeup (former P-5).** Resolved by `.claude/rules/reconcilers.md` § "The wakeup model": host-backed, empty `interests()` plus `resync_schedule()`; period is a DELIVER tuning constant.
2. **Lag-aware `IntentStore::watch` (former P-8).** Not a new decision: it is D-APP-10 of the DESIGN the user approved 2026-09-13, unchanged on main since the merge base; carried as previously approved (D5 section "Previously approved public surface carried unchanged"). The optional simplification is the PROPOSED P-8 below.
3. **Shutdown order (former ADR-0250, G-6 counterexample, P-11).** ADR-0250 was architect-added and never user-approved, and is not an independent decision; the order lives in D5 section (c) and narrowed Gate G-6 (gateway stage before the leg-C worker stage; the volatile slot is cleared synchronously). The identity-retire-before-convergence-stop requirement and P-11 are removed.
4. **ADR consolidation (former P-7; documentation structure only, decisions unchanged).** The custody-domain-boundary and acquisition-vs-consumption records merged into ADR-0223; the identity-slot record into ADR-0225; the packet-entry/existing-dataplane record into ADR-0224 (its TEACH half retired); public-TLS-vs-gateway-mTLS into ADR-0234; the state-based-domain-ownership record retired (its rejected ES/CQRS alternatives moved into ADR-0221). The demand lifecycle record is retired and replaced by ADR-0242. ADR-0222 is not split.
5. **Single hydration read-port.** `GatewayIdentityView` and one `HydrationContext` field replace the two read-ports and two fields.
6. **DISTILL obligations trimmed** to match (GA-2 dropped, GA-1 folded into the probe owner's unit-level test, obligation 3 narrowed to "a stale map entry yields 502, never a wrong-peer success", obligation 5 matched to item 3).
7. **Source-settled validation residue.** V-GW-2 (socket cgroup is the creating task's) and the V-GW-1 client-handshake question are settled by kernel and repository source (Contract-shaping validation items); no user decision is needed.

### PROPOSED — awaiting user

No item below is part of the design until the user adds it. **P-2, P-9 and P-10 BLOCK design approval** (each changes a contract or constrains DISTILL; the design body is neutral on them). P-1, P-3, P-4, P-6 and the P-8 simplification are optional and non-blocking.

| Id | Proposal | Reachable production failure it prevents | Cost | Cheaper alternative |
|---|---|---|---|---|
| P-1 | Carry the branch `.claude/rules/spike.md` addition ("A spike may drive an existing component"). | None gateway-specific. | One rule paragraph. | Do not carry it. |
| P-2 (**BLOCKING design approval**) | `BackendIdAllocator` keys its memo by `(ip, port, proto)` only (`M:crates/overdrive-dataplane/src/allocators/backend_id.rs:58-70`). If the guest address pool re-leases an address to a different allocation while the memo is live, the memo hit returns the old id and the applied-identity rule rejects the update (ADR-0241); ADR-0241's re-lease wedge exposure exists only if neither option below is adopted. **(a)** The memo key includes the owning `Backend.alloc`, so a re-leased address gets a fresh id. **(b) Reorder, no signature change:** release the memo entries of backends leaving the service before allocating the new set, inside one `update_service`. Main does NOT already do this: `update_service` allocates in step 1 (`M:crates/overdrive-dataplane/src/lib.rs:1903-1907`) and releases the removed ids only after the outer swap, in the orphan-GC sweep (`:2066-2082`, `alloc.release` at `:2079`). | A Service whose backend address was re-leased wedges its hydration until the id is released. | (a) One allocator signature change. (b) A reordering inside `update_service` that needs the prior per-id allocation (held by the applied association, ADR-0233) to know which memos to drop early, and a test proving it. | **(b) is the subtractive alternative** (no signature change). Recommendation: (b) if the wedge is judged reachable (it requires an address re-lease while the old memo is live, i.e. a guest-address pool reuse the design has not shown on a production path); otherwise neither, with the exposure recorded. The user rules. |
| P-3 (optional, non-blocking) | Correlate the connect intent and receipt through `BPF_MAP_TYPE_SK_STORAGE` on the upstream socket instead of cookie-keyed hash maps. `bpf_sk_storage_get` is available to `cgroup/connect4` programs (`net/core/filter.c:8215-8216 @ v7.0`) and the entry dies with the socket. **If adopted it would delete:** the cookie-keyed intent and receipt maps, the cleanup ledger and the `cleanup`/`cleanup_all_gateway_intents` actions, the live-count sweeps (`live_intent_count`/`live_receipt_count`), the 128-entry registry capacity, and the `SO_COOKIE` read step. **If adopted it changes ADR-0232's contract** (`GatewayConnectDataplane` and the intent/receipt shape) and is contract-shaping, so it requires a new spike (userspace update by socket fd, connect4 access to `ctx->sk`) BEFORE DISTILL, and re-running the V-GW-5/selection-receipt checks and the Quint `selection_receipt` model. | None; a simplification. | A new spike and a changed [ADR-0232](../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md) plus follow-on edits to ADR-0228/0243, the D5 surface and the Quint model. | Keep the cookie maps as designed (the design as written). |
| P-4 | Open a GitHub issue that `ServiceMapHydrator` leaves `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` entries (`healthy = 1`) at least 23 s after `job stop` (D4 findings, cause not investigated). The gateway does not depend on the fix: a stale entry still selects a backend whose registered inbound tuple is gone, the shared nft output rule resets the dial (`M:crates/overdrive-netlink/src/nft.rs:833`) and the request returns 502. | A stopped Service's backend stays selectable by the XDP and gateway paths. | An issue (needs user approval before creation). | Investigate under the gateway DELIVER. |
| P-6 | A scheduled metal CI lane for the gateway Tier-3 runner. | A regression in the gateway wire path ships unseen. | CI minutes and a metal lease. | Run `cargo xtask metal run --` by hand per release. |
| P-8 (optional, non-blocking) | **Simplification of the previously approved D-APP-10.** The approved contract (2026-09-13, carried unchanged) changes main's `IntentStore::watch` (`M:crates/overdrive-core/src/traits/intent_store.rs:318-332`, a `Stream<(Bytes,Bytes)>` where an empty value means delete) to a lag-aware `IntentSubscriptionEvent` (`Changed`/`Lagged`) and migrates every watcher (`LocalStore`, Sim, the action-shim test fake, existing consumers). Alternative: keep main's `watch` and relist the Route/custody keys on a fixed period (bounded staleness instead of fail-closed on lag); main's watch broadcast has a 1024 capacity and drops silently on lag (`M:crates/overdrive-store-local/src/redb_backend.rs:73, 403-417`). | A withdrawn Route stays served after a missed watch event (what the approved contract closes). | The approved contract costs one cross-crate trait change and a bounded watcher migration; the alternative removes both. | **The design keeps the approved contract unless you adopt the simplification.** Trade-off: the alternative accepts a bounded window (one relist period) of serving a withdrawn Route or key after a missed event, in place of a fail-closed admission clear. |
| P-9 (**BLOCKING design approval**) | Should the registered hook arm honour `BACKEND_MAP.healthy`? **Honour:** `healthy == 0` records `NoBackend` (503) in the ~1 s readiness window (V-GW-3: `NoBackend` → `Selected` about 1.05 s after startup `Stable`). **Ignore:** the byte is not read; a request in that window dials a not-yet-ready guest and surfaces as 502 (the nft output rule resets it). No BPF program reads `healthy` today (`M:crates/overdrive-dataplane/src/maps/mod.rs:135`); V-GW-3 also saw `healthy` flap 1 → 0 in 2 of 3 runs (cause not established), so honouring it exposes gateway 503s to that flap. | **Honour:** a request in the readiness window is a typed 503 instead of 502. **Ignore:** none; the window is 502. | **Honour:** one byte read on the connect-time path, an extra Quint run and invariant, and exposure to the unexplained flap. **Ignore:** nothing added. | **Ignore `healthy`** (the subtractive review's alternative): fewer moving parts, no dependence on the flap; the cost is 502 instead of 503 for about 1 s after startup. The design body is neutral: it states the decided core (no outer entry or no usable backend → `NoBackend` → 503; selected backend whose dial fails → 502; never a wrong-peer success) and marks the `healthy` handling "pending P-9". |
| P-10 (**BLOCKING design approval**) | **Placement of the registered gateway selection arm** (V-GW-4 measured 246 verified instructions for the arm vs 28 for the existing `cgroup_connect4`, 7.0.0-38-generic). **(a)** A separate `cgroup/connect4` program (registered-gateway selection only) attached beside `cgroup_connect4_service`; multi-attach coexistence was observed (`bpftool cgroup tree`: production plus two probes, all `multi`). The kernel runs every attached program in array order over the same context and any deny yields `-EPERM` (`kernel/bpf/cgroup.c:79-89 @ v7.0`), so the order contract is: an unregistered socket passes the new program unchanged (it looks up an empty cookie slot and returns allow), a registered socket is rewritten by it. The outcome does not depend on attach order, because `LOCAL_BACKEND_MAP` holds no VM backend (D4: empty), so the existing program misses and allows both the VIP and the rewritten guest address. Attach mode is stated explicitly as multi. The existing hook's verifier baseline is untouched. **(b)** Fold the arm into `cgroup_connect4_service` and re-baseline `perf-baseline/main/verifier-budget/` (append, never edit; `.claude/rules/bpf.md`); this requires your authorisation because the existing program would grow about 8.8x against the ">5% over baseline fails" rule (`.claude/rules/testing.md` Tier 4) and the growth is architectural, not a reduction. | Same production failure as D3 (a host-originated gateway `connect(2)` to a Service VIP is not selected). The choice affects only cost and blast radius. | (a) One more attached program and one verifier-baseline file; every ordinary same-host connect additionally pays one cookie-map lookup in program (a)'s run, while (b) pays it inside the existing program. (b) re-baselines a hot-path program every workload connect runs. | **Recommendation: (a).** The gateway arm is a distinct concern (registered-socket selection) with its own maps and receipt; keeping it out of the program every workload connect runs leaves that program's verifier cost and `LOCAL_BACKEND_MAP` behaviour byte-identical and needs no baseline authorisation (design.md "correct, and no more than correct"; the observed multi-attach makes it mechanically free). (b) saves one program slot and one cookie lookup but spends a 5% rule exception on a program unrelated to the gateway. Both are BPF-first and both satisfy ADR-0224. |

Items the design deliberately does not carry: a userspace backend load balancer, a named Gateway resource, ACME, WireGuard, in-guest mTLS, DNS mutation, any new persistence subsystem.

## Wave: DESIGN / [REF] Backend programming and selection on main (D1, D3)

**Backend programming (D1).** `ServiceLifecycle` publishes the complete `ServiceBackendRow` with `backend_ip = row.workload_addr.unwrap_or(host_ipv4)` and `backend_spiffe = SpiffeId::for_allocation(..)` (`M:crates/overdrive-reconcilers/src/service_lifecycle.rs:888-889`, published `:1064-1068`); the running row carries the guest address assigned by the action shim (`M:crates/overdrive-control-plane/src/action_shim/mod.rs:2455-2458`). `run_server` constructs the hydrator with the legacy `10.99.0.0/16` subnet (`M:crates/overdrive-control-plane/src/lib.rs:5426-5438`) and the hydrator excludes only that subnet (`M:crates/overdrive-reconcilers/src/service_map_hydrator.rs:349-354`), so a `100.95.x.y` backend (`M:crates/overdrive-core/src/guest_transport/mod.rs:594`) is a remote survivor of `Action::DataplaneUpdateService` (`:414`). `EbpfDataplane::update_service` writes `BACKEND_MAP`, the Maglev inner map, `REVERSE_NAT_MAP` and swaps the outer `SERVICE_MAP` slot (`M:crates/overdrive-dataplane/src/lib.rs:1890-1910, 1964, 1997-2028, 2033-2042`); `LOCAL_BACKEND_MAP` is written only by `register_local_backend` (`:2149-2179`). The D4 spike observed exactly this on metal.

**What the gateway still needs from BPF (D3).** `cgroup_connect4_service` reads only `LOCAL_BACKEND_MAP` and allows every miss (`M:crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:78-82`), so a host-originated gateway `connect(2)` to the Service VIP is not selected. A `cgroup/connect4` selection arm (placement: P-10) serves sockets registered by the dataplane adapter:

1. Lookup key: the socket cookie from `bpf_get_socket_cookie`, which `cgroup/connect4` programs may call (`net/core/filter.c:8199-8200 @ v7.0`) and which returns `__sock_gen_cookie(ctx->sk)` (`net/core/filter.c:5151-5154 @ v7.0`); userspace `getsockopt(SO_COOKIE)` returns `sock_gen_cookie(sk)` of the same generator (`net/core/sock.c:2109-2113 @ v7.0`), so the pre-connect cookie equals the hook's.
2. A socket with no registered intent follows today's `LOCAL_BACKEND_MAP` behaviour byte for byte.
3. A registered socket resolves the intent's `ServiceKey` in `SERVICE_MAP` (outer lookup, NULL-check, then chained inner Maglev lookup indexed by a deterministic hash of the cookie), then `BACKEND_MAP[id]`. Absent outer entry, empty slot or missing backend records `NoBackend` and returns verdict 0 (`healthy == 0` also does so only if P-9 is adopted). A returned verdict other than 1 makes `connect(2)` fail with `-EPERM` (`kernel/bpf/cgroup.c:1664-1665, 1694-1696 @ v7.0`). Otherwise it rewrites `user_ip4`/`user_port` to the selected backend and records `Selected(BackendId)`. Any registered-arm decode or map-write failure denies; there is no fall-through to an unchanged VIP.
4. The hook runs in `pre_connect`, before `tcp_v4_connect` performs its route lookup with the post-rewrite destination and the socket's bound source and mark (`net/ipv4/af_inet.c:679-683 @ v7.0`; `net/ipv4/tcp_ipv4.c:206-218, 254-256 @ v7.0`), and against the socket's own cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`).
5. `BACKEND_MAP.healthy` is written by the dataplane from `Backend.healthy` (`M:crates/overdrive-dataplane/src/maps/mod.rs:135`) but no current BPF program reads it. Whether the registered arm reads it is the pending user decision P-9: reading it is the only way the gateway honours ServiceLifecycle's readiness verdict (one byte read on the connect-time path; the readiness window is 503); ignoring it leaves that window to the dial (502). The selected-then-failed-dial and never-wrong-peer rules hold either way.

V-GW-4 measured the arm at 246 verified instructions against 28 for the existing `cgroup_connect4` (7.0.0-38-generic, [findings](spike/findings-gateway-connect-path.md)), so the placement of the arm (a second program beside the existing one, or folded into it with a Tier-4 re-baseline) is the user decision P-10; no baseline change is pre-authorized. Either way DELIVER runs the verifier-regression gates (`.claude/rules/bpf.md`, `.claude/rules/testing.md` Tier 4), states the attach mode explicitly (multi-attach; the loader reported `mode=Single` for aya's own call while `bpftool` listed `multi`), and pins the program order contract in P-10. The XDP wire programs are unchanged. The BPF maps `GATEWAY_CONNECT_INTENT_MAP`/`GATEWAY_SELECTION_RECEIPT_MAP` (8-byte PODs, capacity 128, keyed by host-order cookie, not pinned: their lifetime is the `EbpfDataplane` instance) are declared in `overdrive-bpf` `maps/gateway_connect.rs`.

## Wave: DESIGN / [REF] Gateway upstream path into leg-C (D3, D5-d)

The path of one gateway upstream request, with the owner of each step:

| # | Step | Owner | Source |
|---|---|---|---|
| 1 | Create an `AF_INET` `SOCK_STREAM` socket in the calling `serve` task. The socket belongs to the cgroup of the creating task, and the hook runs against that cgroup. | dataplane adapter (`open_upstream`) | `kernel/bpf/cgroup.c:1694 @ v7.0`; attach path `M:crates/overdrive-dataplane/src/lib.rs:119` (`/sys/fs/cgroup/overdrive.slice`); `serve` enrols itself `M:crates/overdrive-control-plane/src/lib.rs:3132` |
| 2 | `setsockopt(SO_MARK, MTLS_TPROXY_MARK = 0x1)`, `IP_BIND_ADDRESS_NO_PORT`, `bind(127.0.0.1:0)`, in that set before `connect`. Mark alone fails `ENETUNREACH` (101), bind alone fails `EINVAL` (22), both together route the dial through table 100 `local default dev lo` and the TPROXY rule with a loopback source that does not depend on host routing state. On a host with a default route an unprepared dial to a registered tuple also reaches leg-C, because the shared output chain marks any dial to a registered inbound destination (`M:crates/overdrive-netlink/src/nft.rs:822-833`); that dial takes the node default-route source address and the nft re-route path, so the preparation is what makes the source and route deterministic (V-GW-1 control (a)-(c)). | dataplane adapter | ADR-0190 (cites `net/ipv4/route.c:2848-2858`, `include/net/route.h:319-356`, `net/ipv4/devinet.c:143-170` @ v7.0; V-LEGB-MARK held); `M:crates/overdrive-dataplane/src/mtls/mod.rs:946-997` (private `dial_leg`); `M:crates/overdrive-core/src/dataplane/mtls_mark.rs:27` |
| 3 | `getsockopt(SO_COOKIE)`, then register `(cookie, ServiceKey)` in `GATEWAY_CONNECT_INTENT_MAP`. | dataplane adapter | `net/core/sock.c:2109-2113 @ v7.0` |
| 4 | `connect(2)` to the Service VIP and port; the hook selects, rewrites and records the receipt (previous section). | gateway connector (the call), cgroup hook (the selection) | `net/ipv4/af_inet.c:679-685 @ v7.0` |
| 5 | The rewritten packet (source `127.0.0.1`, mark 1, destination `workload_addr:declared port`) leaves through table 100 to `lo`. The shared output chain accepts a destination in the registered `inbound_destinations` set (rule 6) and resets any other guest-prefix TCP destination (rule 7); the prerouting rule diverts the registered tuple to leg-C by TPROXY (rule 1). | node-shared nft owner | `M:crates/overdrive-netlink/src/nft.rs:797-800, 822-833` |
| 6 | Leg-C accepts with the recovered original destination, runs the rustls server handshake presenting the held SVID of the allocation that owns that address and requiring a bundle-chained client SVID, arms kTLS, then relays to the guest through `GuestStreamDial` over vsock. | leg-C enforcement | `M:crates/overdrive-dataplane/src/mtls/inbound.rs:56-62, 69, 86-90` |
| 7 | Gateway-client mTLS on the connected fd: present `[gateway leaf, node intermediate]`, anchor at the root, require exactly one URI SAN, and require it to equal the `SpiffeId` the dataplane resolved from the receipt's `BackendId`. Only then is the stream sealed as `GatewayUpstream`. | `HostGatewayClientMtls` (dataplane crate) | verifier building blocks `M:crates/overdrive-dataplane/src/mtls/tls_config.rs:68-114, 196-222`; equality is new |

A selected-then-stopped backend whose registered tuple is gone is reset at step 5 and surfaces as a connect failure (public 502). The marked, loopback-bound dial is correct only while every Service backend address lies in the guest prefix (VM-only workloads on a single node, ADR-0190 "Consequences"); this is assumption A-3 and is not defended by a runtime check, because no production path produces another backend address.

**Why preparation stays (rationale, present tense).** The preparation defends against the node default route and the nft re-route path: without it the dial's source address and route depend on the host's routing state, and mark-only or bind-only dials fail with `ENETUNREACH` or `EINVAL` ([ADR-0190](../../product/architecture/adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md) consequence). It keeps the source on `127.0.0.1` so the dial takes table 100 and TPROXY exactly as the node's own leg-B dial does. Preparation is not what keeps an unregistered socket out of the selection arm; that is the cookie registration (V-GW-1 control (d): an unregistered cookie gets no receipt and today's behaviour).

**Contract of upstream-socket preparation (D5-d).** [ADR-0243](../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md) places steps 1-3 in the dataplane adapter. `register` is removed from the gateway-facing port, so a gateway socket cannot exist unmarked, unbound or unregistered:

```rust
// overdrive-gateway::ports (replaces the branch `register`; other methods keep their branch contracts)
#[async_trait::async_trait]
pub trait GatewayConnectDataplane: Send + Sync {
    /// First step of the boot gate: sweep residue, then run the sentinel
    /// `192.0.2.1:9/tcp` connect. Success = the connect is denied and a
    /// canonical `NoBackend` receipt for the sentinel cookie is consumed.
    async fn probe(&self, deadline: Instant, clock: &dyn Clock)
        -> Result<(), GatewayConnectDataplaneError>;
    /// Steps 1-3. Returns the prepared, not-yet-connected socket with its
    /// registered intent. The only constructor of a registered intent.
    fn open_upstream(&self, key: ServiceKey)
        -> Result<PreparedUpstream, GatewayConnectDataplaneError>;
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

/// A socket prepared by the dataplane adapter. Not Clone.
pub struct PreparedUpstream { /* private: tokio::net::TcpSocket + GatewayConnectIntent */ }
impl PreparedUpstream {
    pub fn intent(&self) -> &GatewayConnectIntent;
    /// Hands the socket to the caller for `connect(2)`; the intent stays registered
    /// until `cleanup(&intent)`.
    pub fn into_parts(self) -> (tokio::net::TcpSocket, GatewayConnectIntent);
}
```

Added error variants: `GatewayConnectDataplaneError::{SocketCreate, SocketOption, SocketBind, CookieRead}` (each carrying the originating `io::ErrorKind`) alongside the branch variants; `IntentRegistryFull`, `IntentAlreadyRegistered`, `IntentMissing`, `ReceiptMissing`, `ReceiptMismatch`, `BackendIdentityMissing`, `Kernel`, `Cleanup`, `Unavailable`, `ProbeTimeout` are unchanged. `SocketCookieReader` becomes a dataplane-internal helper; it is not a boot-adapter field and `UnboundGatewayBuilder::new` no longer takes it.

Permitted callers of `open_upstream`: `GatewayUpstreamConnector::{probe, connect}` only (crate-private). Enforcement is the existing `dst-lint` structural scan pinned in [Application component decomposition](#wave-design--ref-application-component-decomposition), extended to forbid `open_upstream`/`into_parts` outside `overdrive-gateway::connector`. The sentinel probe's own connect is made inside the dataplane adapter on its own prepared socket.

## Wave: DESIGN / [REF] Consuming-port availability contract (D2)

[ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) is the decision. The contract:

- **Owner.** `GatewayUpstreamConnector::{probe, connect}` (the gateway's own consuming port) is the only place backend availability becomes an outcome. `GatewayApplicationOwner` never waits for a backend.
- **No readiness wait.** `connect(frontend, deadline)` performs one selection. There is no poll, sleep, retry or second pick. The only time bound it consumes is the request's existing absolute deadline (`limits.upstream_connect_deadline()` within the owner-supplied request deadline); no budget is added or multiplied.
- **Outcome table (the failure projection).**

| Condition at the hook | Receipt | Connector result | Public status |
|---|---|---|---|
| No outer entry for the `ServiceKey`, empty Maglev slot, missing `BACKEND_MAP` entry | `NoBackend` | `GatewayUpstreamConnectError::NoBackend` | 503 |
| Selected backend has `healthy == 0` (not ready, or in the observed ~1 s window between startup `Stable` and the first readiness `pass`) | **pending P-9** — honour `healthy`: `NoBackend`; ignore `healthy`: `Selected` | honour: `NoBackend`; ignore: `Connect` (the dial to a not-yet-ready guest fails) | honour: 503; ignore: 502 |
| Selected, registered tuple gone (stopped backend still in the maps, P-4) | `Selected` | `Connect` (reset at the nft output chain) | 502 |
| `Selected`, applied identity missing or mismatched | `Selected` | `SelectedIdentity` / `ReceiptMismatch` | 502 |
| Hook, map or receipt inconsistency | missing or malformed | `Receipt` | 502 |
| Exact-peer mTLS failure | `Selected` | `Mtls(..)` | 502 |

- **Sim arm contract (D-PASS7-4).** `SimGatewayConnectDataplane` implements the same observable contract as the real hook, over the `SimDataplane` state it shares, and decides nothing the table above leaves open: for a prepared socket's `ServiceKey`, **no outer entry for the key, or an empty/absent slot in the chained lookup → receipt `NoBackend`**; otherwise → `Selected` carrying the actual `BackendId` the shared `SimDataplane` state holds for the key's slot (deterministic choice, the lowest `BackendId` of the key's current generation; the real Maglev choice is proven only at Tier 3). **Oracle strength:** because the real hook selects by Maglev over the cookie hash (item 3 above), Tier-1 / Sim-backed oracles assert only *membership* in the committed generation when it has N>1 backends; exact-`BackendId` assertions are limited to singleton generations. **Tier-3-only row:** the real outcome table also yields `NoBackend` for a missing `BACKEND_MAP` entry (outer and slot present, backend id absent); the Sim cannot produce that from the `update_service` input, so that row is observed only at Tier 3, and `selected_backend_identity` resolves it only for ids whose `update_service` has completed (Applied), mirroring the commit-guarded read. The Sim does not evaluate `Backend.healthy` (it is present in the `update_service` input but the Sim decides nothing on it): until P-9 is ruled the Sim returns `Selected` for any backend of the generation and the healthy window is neither modelled nor tested at Tier 1 (it is a Tier-3/Native observation, obligation 2). If P-9 rules "honour", the Sim adopts the real rule through the `Backend.healthy` value already in the `update_service` input, not through a scripting API. Cookies are deterministic nonzero values; `open_upstream` registers like the real adapter; `cleanup`/`take_receipt` follow the port contract (a receipt exists exactly between the Sim connect and `take_receipt`/`cleanup`). A scenario obtains `NoBackend` by never calling `update_service` for the key (or calling it with an empty set) and `Selected` by calling it with a non-empty set; no scripted receipt exists. **Fault surface the tests may use (existing ports only):** typed `GatewayConnectDataplaneError` results returned from the existing port operations (`probe`, `open_upstream`, `take_receipt`, `cleanup`, `selected_backend_identity`, `live_*`) via the Sim's existing typed-fault configuration at those operations, and the pinned `HoldConnectProbe`/`ReleaseConnectProbe` (see Application verification). Nothing else is scripted; a probe failure at boot is the typed `probe` error.
- **Boot.** An actual-frontend `NoBackend` probe result is bind-valid (unchanged gate G-1); backend absence never blocks TCP/443.
- **Late readiness.** The first request after the readiness `pass` reaches the hydrator-written `healthy = 1` and is `Selected`. The window before it returns 503 if P-9 honours `healthy`, otherwise a dial that fails (502); clients retry either way. The design adds no wake to shorten it. The decided core is independent of P-9: no outer entry or no usable backend → `NoBackend` → 503; a selected backend whose dial fails → 502; never a wrong-peer success.
- **Replacement obligation for DISTILL (restates acceptance gap C4a).** The unavailable-outcome path applied twice has an empty second observable delta: two consecutive public requests while no healthy backend exists both return 503 with identical bodies, `live_intent_count() == live_receipt_count() == 0` after each, no change in `GatewayApplicationStatusRowV1`, and no change in the Gateway Applied Generation. Custody replay and identity-hold repeat keep their existing coverage. DISTILL also proves: `healthy` 0 → 1 flips the next request with no gateway action (if P-9 is ruled "honour": 503 → backend response; if "ignore": 502 → backend response), matching obligation 2; a stopped backend yields 502 (not a wrong-peer success); late `Selected` after a superseded application generation is not delivered.

## Wave: DESIGN / [REF] Composition-root gateway slot, boot and shutdown (D5)

### New public surface (D5) — for user confirmation

Every item is new public or cross-crate surface. The user confirms or rejects each.

1. `BootAdapters.gateway: GatewayAdapters` (new required field) and `GatewayAdapters` (`certified_key_codec`, `client_mtls`).
2. `GatewayClientMtlsFactory` and `GatewayClientMtlsDeps` (`overdrive-gateway::ports`).
3. `ComposedDataplane.gateway_connect: Arc<dyn GatewayConnectDataplane>` (new required field).
4. `GatewayConnectDataplane::open_upstream` and `PreparedUpstream` (replace `register`), the four added error variants.
5. `UnboundGatewayBuilder::into_builder` (replaces `bind_demand_wake`), and the removal of the `cookie_reader` parameter from `UnboundGatewayBuilder::new`.
6. `GatewayAppStateComposition` without demand; `AppState.gateway`.
7. `GatewayShutdownReport.elapsed` and the ServerHandle gateway stage (`stage = "gateway"`).
8. `HostGatewayClientMtlsFactory` (dataplane crate) and `SimGatewayClientMtlsFactory`, `SimGatewayConnectDataplane` (sim crate).
9. `Action::IssueGatewaySvid`/`Action::DropGatewaySvid`, the `GatewaySvidLifecycle` reconciler and its `AnyReconciler`/`AnyState`/`AnyReconcilerView`/`AnyViewMap` variants (wakeup declaration: host-backed, empty `interests()` plus `resync_schedule()`, resolved by `.claude/rules/reconcilers.md`).
10. One NEW hydration read-port in `overdrive-core` — `GatewayIdentityView` (non-secret gateway-identity desired epoch/subject and current facts; contract in `overdrive-core::gateway_identity`) — and the one new mandatory `HydrationContext` field that carries it (`gateway_identity`, after `guest_attachments`). The port has one `Sim*` implementation (`SimGatewayIdentityActionTarget`) so `GatewaySvidLifecycle::hydrate_*` stays DST-injectable and reads no concrete `AppState` field (`.claude/rules/reconcilers.md` § "The hydration boundary").
11. `ServeArgs.gateway_*` clap fields, `ServerConfig.gateway: Option<GatewayConfig>` and `deploy_resource` (branch contract, unchanged shape, re-pinned below to `run_with_adapters` and `commands::deploy`).

#### Previously approved (2026-09-13), carried unchanged — reconfirm on main

12. `IntentStore::watch` changes in one cut to `IntentSubscriptionEvent` = `Changed | Lagged` (D-APP-10; modifies a main trait that is unchanged since the merge base). Provenance: the pre-reconstruction branch is preserved at git ref `backup/research-ingress-gateway-pre-reconstruct` (commit f3999011500f; tag `backup-public-ingress-pre-reconstruct-2026-11-01`). Its `docs/feature/public-ingress-gateway/feature-delta.md` lines 3-7 record "APPROVED — user ratification APPROVED, 2026-09-13 … The user explicitly approved the complete public-ingress-gateway DESIGN", and its line 1198 is "**D-APP-10** | `IntentStore::watch` changes in one cut to `Changed|Lagged`; …". Listed here for transparency so the user can reconfirm it on main.

**Previously approved public surface carried unchanged (not D5; listed for completeness).** Items the user approved in the 2026-09-13 DESIGN and that main still supports unchanged (classification: class 1, salvaged unchanged): the lag-aware `IntentStore::watch` change (`IntentSubscriptionEvent` = `Changed | Lagged`, D-APP-10; main's trait `M:crates/overdrive-core/src/traits/intent_store.rs:318-332` and `LocalStore` are unchanged since the merge base, so the same one-cut change and bounded watcher migration apply). It is not a new decision of this reconstruction; the optional simplification is P-8.

### (a) The gateway adapter slot

`run_server(config, NodeAdapters)` (`M:crates/overdrive-control-plane/src/lib.rs:3116-3120`) constructs no driven adapter (ADR-0219). The gateway's driven adapters therefore enter through the two existing carriers; there is no `*_override` field and no concrete downcast of `Arc<dyn Dataplane>`:

```rust
// overdrive-control-plane::node_adapters  (M:node_adapters.rs:62-98 and :418-423)
pub struct BootAdapters {
    /* every existing field unchanged */
    /// Required; no `Default`, no builder. Enablement is `ServerConfig.gateway`, not this field.
    pub gateway: GatewayAdapters,
}
pub struct GatewayAdapters {
    /// Seals and opens the protected public origin key (ADR-0223).
    /// `GatewayAdapters::host` builds it over the same `Kek` as `NodeAdapters::host`.
    pub certified_key_codec: Arc<overdrive_host::ca::PublicCertifiedKeyAeadCodec>,
    /// Builds the exact-peer client over the identity access the boot creates
    /// after the CA exists (ADR-0234). `HostGatewayClientMtlsFactory` in production.
    pub client_mtls: Arc<dyn GatewayClientMtlsFactory>,
}
impl GatewayAdapters {
    pub fn host(kek: Arc<dyn Kek>) -> Self;
}

// overdrive-gateway::ports  (the trait lives with `GatewayClientMtls`, so the
// dataplane crate implements it without depending on the control plane)
pub trait GatewayClientMtlsFactory: Send + Sync {
    fn client_mtls(&self, deps: GatewayClientMtlsDeps) -> Arc<dyn GatewayClientMtls>;
}
pub struct GatewayClientMtlsDeps {
    pub identity: GatewayClientIdentityAccess,
    pub seal: GatewayUpstreamSeal,
    pub limits: GatewayLimits,
}

pub struct ComposedDataplane {
    pub dataplane: Arc<dyn Dataplane>,
    pub host_ipv4: Ipv4Addr,
    /// The dataplane-side gateway selection capability of the SAME instance.
    /// `HostDataplaneComposer::compose` builds one `Arc<EbpfDataplane>` and
    /// coerces it to both trait objects; the simulation composer returns
    /// `SimGatewayConnectDataplane` sharing the `SimDataplane` state.
    pub gateway_connect: Arc<dyn GatewayConnectDataplane>,
}
```

The registered-selection program (placement P-10) and the two intent/receipt maps are part of the one BPF object `EbpfDataplane` already loads, so the capability exists whether or not the gateway is enabled; with `ServerConfig.gateway == None` nothing calls it, no socket is prepared, no sentinel connect runs and the pre-feature behaviour is unchanged. `SimDataplaneComposer` (the test swap of `M:crates/overdrive-cli/tests/integration/serve_fixture.rs:116-127`) supplies the Sim pair, so a test boots the production composition with the gateway driven ports replaced by name.

### (b) Construction, probe and bind in boot

Order in `boot_server` (`M:crates/overdrive-control-plane/src/lib.rs:3625`), each step with its refusal class:

| Step | Position | Action | If it fails |
|---|---|---|---|
| G0 | after `runtime.register(service_lifecycle())` (`:3807`), before `Arc::new(runtime)` (`:3808`) | `if config.gateway.is_some() { runtime.register(gateway_svid_lifecycle()).await? }` | boot refuses (existing registration error) |
| G1 | after `ca.trust_bundle()` (`:3912`), before `AppState::new_with_workflow_engine` (`:4085`) | `GatewayAppStateComposition::from_config(..)`: Disabled when `config.gateway` is `None`; otherwise creates the one `GatewayIdentitySlot`, its Action and Lifecycle handles and `GatewayControl` | boot refuses |
| G2 | after the DNS-responder block (`:4350`), before the exit-observer spawn (`:4369`) | `UnboundGatewayBuilder::new(..).await` (validates `GatewayConfig`; opens Route and custody over the IntentStore; reads and installs the initial manual key; spawns no task), then `into_builder()` and `client_mtls.client_mtls(deps)` | boot refuses with `GatewayBootError` (`health.startup.refused`, reason `gateway.boot`) after the existing `shutdown_failed_boot_network_owners(..)` and `dns_responder_owner.shutdown(..)` cleanup |
| G3 | inside the gateway gate task (G4) | `gateway_connect.probe(deadline, clock)`: sweeps residue left by the previous process, then runs the sentinel connect | not fatal: TCP/443 stays unbound, the operator surface and workloads are unaffected, status carries the cause, and the structured event `health.startup.refused` with reason `gateway.connect_probe` is emitted (Earned Trust: the adapter proves it can honour its contract before the listener binds) |
| G4 | after `server_task` is spawned (`:4616`), before `Ok(ServerHandle { .. })` (`:4618`) | `GatewayBuilder::start(deps).await`, then `into_parts()`: spawns the Route, custody, application, connection and gate tasks; the gate runs custody ready → identity `ensure_current` → G3 → actual-frontend probe → Gateway Application promotion → TCP/443 bind | not fatal (see G3); no later refusal path exists, so `FailedBootTasks` (`:3008`) is unchanged |

The convergence loop (`:4399`), interest router (`:4447`), emit-drain (`:4474`) and shared-network supervisor (`:4505`) are all running before G4, so the first identity evaluation and the first hydration see the final ports. `ServerHandle` gains `gateway: Option<GatewayHandle>`.

### (c) Shutdown order and the fail-stop path

`ServerHandle::shutdown(self, drain_deadline)` (`M:crates/overdrive-control-plane/src/lib.rs:2890-2993`) runs the gateway first, then the existing stages unchanged:

0. `gateway` (new stage `serve.shutdown.stage_begin`/`stage_complete`, `stage = "gateway"`): `GatewayHandle::shutdown(drain_deadline)` — stop accepts; close the application; join admitted users; sweep intents and receipts; retire the closed application; cancel the owners; then clear the volatile `GatewayIdentitySlot` synchronously (`identity.disable_after_drain`, whose `begin_disable` empties the slot and bumps its epoch in-process; no reconciler round-trip at shutdown, because the issuance audit row is already durable and a late Issue/Drop completion is a stale-epoch no-op).
1. `convergence` → `emit_drain` → `interest_router` → `axum_graceful` → `server_task` → `exit_observer` → `shared_network_supervisor` → `mtls_worker_owner` → `guest_flow_owner` → `mtls_resolve_owner`, unchanged.

The one ordering requirement is that the gateway stage runs first, before the existing stages and in particular before `mtls_worker_owner` (stage 6): admitted gateway connections ride leg-C, so they must end before the worker that terminates leg-C is shut down (Gate G-6). This is the whole decision; it is not a separate ADR, and no identity-retire-before-convergence-stop ordering exists (the slot is volatile and cleared in-process).

One budget, not two: `GatewayShutdownReport` gains `pub elapsed: Duration` measured on the gateway's injected `Clock`, and `ServerHandle` passes `drain_deadline.saturating_sub(report.elapsed)` to `graceful_shutdown`. With the gateway disabled the stage does not run and axum receives `drain_deadline` exactly as today. A non-clean report is appended to the existing `ServerShutdownError` (branch contract: `Gateway`/`Combined`).

Fail-stop: a shared guest-network fail-stop request still originates only from `ServerHandle::shutdown_requested` (`M:crates/overdrive-control-plane/src/lib.rs:2769`) and the CLI lifetime owner runs `ServeHandle::shutdown` under `FAIL_STOP_OUTER_BOUND = 10 s` with the 5 s drain (`M:crates/overdrive-cli/src/commands/serve_lifetime.rs:20-42`, `M:crates/overdrive-cli/src/commands/serve.rs:34`). The gateway stage consumes part of that same 5 s drain and the bound is unchanged. A gateway task exit or failed gate never requests serve shutdown; it stops public admission and is reported in status. If the outer bound abandons shutdown, the process-local intent/receipt maps and the Gateway SVID vanish with the process (the maps are not pinned) and the next boot's G3 sweep makes no difference to them.

### (d) Upstream-socket preparation

Owner `EbpfDataplane` (and `SimGatewayConnectDataplane`); sole permitted caller `GatewayUpstreamConnector`; contract and signatures are in [Gateway upstream path into leg-C](#wave-design--ref-gateway-upstream-path-into-leg-c-d3-d5-d).

### Re-pinned composition signatures

These replace the branch demand-bearing forms; every other branch signature in the Application sections keeps its contract.

```rust
// overdrive-control-plane::gateway_composition
#[derive(Clone)]
pub struct GatewayAppStateComposition {
    identity_view: Arc<dyn GatewayIdentityView>,
    identity_actions: GatewayIdentityActionComposition,
    control: Option<GatewayControl>,
}
impl GatewayAppStateComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(/* the identity view, the identity actions and `GatewayControl` */) -> Self;
    pub fn identity_view(&self) -> &dyn GatewayIdentityView;
    pub fn identity_actions(&self) -> &GatewayIdentityActionComposition;
    pub fn control(&self) -> Option<&GatewayControl>;
}

// overdrive-gateway
impl UnboundGatewayBuilder {
    pub async fn new(
        config: GatewayConfig,
        intent: Arc<dyn IntentStore>,
        observations: Arc<dyn ObservationStore>,
        service_vip_view: Arc<dyn ServiceVipView>,
        key_codec: Arc<PublicCertifiedKeyAeadCodec>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, GatewayBootError>;
    pub fn into_builder(self) -> (GatewayBuilder, GatewayUpstreamSeal);
}

// overdrive-core::reconcilers::hydration  (M:crates/overdrive-core/src/reconcilers/hydration.rs:57-84; after guest_attachments at :76)
pub gateway_identity: &'a dyn GatewayIdentityView,
```

`Action::DataplaneUpdateService` (`M:crates/overdrive-core/src/reconcilers/mod.rs:510`) is unchanged: it carries no gateway field. `IssueGatewaySvid`/`DropGatewaySvid` sit beside `IssueSvid` (`:601`) and `DropSvid` (`:627`). Every action-shim dispatcher gains `&GatewayIdentityActionComposition` immediately after `identity` and no demand argument.

## Wave: DESIGN / [REF] Formal protocol model

Target subsystem directory: `specs/quint/public-ingress-gateway/` (permanent; `checks.toml`, `hazard/`, `evidence/`, `README.md`). Three stateful modules; every conformance target declares a `step` relation and named runs. The model abstracts the kernel (maps, hook, routing) as assumptions A-K1..A-K3 below; real-kernel behaviour is proven at Tier 3.

Protocols judged not to need Quint: the Public Route Set single-slot claim ([ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md)) is one `IntentStore` compare-and-set covered by the store's own property tests; TLS/HTTP policy, routing and header rules are pure functions.

**Reduced scope (remediation r1).** The model covers only the concurrent or crash-ordered protocols that a prose review cannot enumerate: the identity issue/disable/reissue interleaving (`gateway_identity`), the generation drain/supersession interleaving (`gateway_application`), and the commit-gated selection receipt (`selection_receipt`). Route+key whole-object coherence, bind gating (`ListenerBoundImpliesGates`), replacement preservation (`ReplacementPreservesLastUsable`) and shutdown ordering are not modelled: they are sequential rules owned by a single task and become seeded `overdrive-sim` or unit DISTILL obligations (the Required boundary scenarios above). Shutdown order is a plain ordering of `ServerHandle::shutdown` stages (D5 section (c)) and is not a protocol.

### Module `gateway_identity` — decision ADR-0225

- **Variables and pinned accessors.** `desired_epoch`, `desired_enabled` ← `GatewayIdentitySlot::desired() -> GatewayIdentityDesired`. `held` (epoch, serial, not_after) ← `GatewayIdentitySlot::current() -> Option<GatewayIdentityFacts>`. `audited` (serials) ← `ObservationStore::issued_certificate_rows(..)` (`M:crates/overdrive-core/src/traits/observation_store.rs:1950`) filtered to the gateway SPIFFE ID. `now` is model-only (the injected `SimClock`).
- **Actions → driving ports.** `submit_evaluation` → `GatewayIdentityLifecycleControl::ensure_current`; `issue_complete` → `action_shim::gateway_svid::dispatch_issue`; `drop_complete` → `dispatch_drop`; `disable` → `GatewayIdentityLifecycleControl::disable_after_drain`; `advance_time` → `SimClock::advance`; `audit_fail` → a failing `ObservationStore` write injected through the Sim store (driven port).
- **Invariants.** `HoldImpliesAudit`, `StaleEpochNeverHolds`, `AtMostOneHeld`, `DisableClearsOrTimesOut` (`NoAllocationIdentity` is type-level, below).
- **Named runs.** `issue_hold`, `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue`.
- **Hazard (one per decision).** ADR-0225 (merged slot and lifecycle) carries one hazard: `noEpochCheck` (a stale Issue completion holds after disable) → `StaleEpochNeverHolds`. It is the interleaving defect that only exhaustive exploration finds. The former second hazards are dropped: `holdBeforeAudit` defends a straight-line ordering inside one task (audit persisted before hold), which the named run `audit_failure_no_hold` and the production-path test GI-1 already cover (the `HoldImpliesAudit` invariant stays as a cheap check); `twoGenerations` is redundant because "at most one held generation" is the shape of the slot's type (one optional facts value) and is implied by the epoch check (`AtMostOneHeld` stays as a cheap invariant).
- **Conformance placement (D-PASS7-1).** The driver is in the owning crate `overdrive-control-plane`: `crates/overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs`, declared from `gateway_identity_lifecycle.rs` as `#[cfg(all(test, feature = "integration-tests"))] mod quint_gateway_identity;`. `dispatch_issue`/`dispatch_drop` stay `pub(crate)` (reachable in-crate from the `#[cfg(test)]` child module; no visibility change, no new public API; on main the analogous `issue_svid` dispatchers are `pub` in a `pub` module, `crates/overdrive-control-plane/src/action_shim/issue_svid.rs:80,145` and `action_shim/mod.rs:232`, so the narrower visibility here needs no widening). The driver's read of `ObservationStore::issued_certificate_rows` (QC-1 `audited`) reads the owner's own output surface through the port trait (the owner wrote those rows), not a `Sim*` arbiter, so no arbiter marker or Tier-3 pairing applies and the dataplane `GatewayIdentitySlot` is reached through the existing `GatewayIdentityActionTarget`/`GatewayIdentityLifecycleTarget` ports. It is not a `tests/conformance` driver: it does not run through the exported server handler. The driver struct holds the production `ControlPlaneGatewayIdentityLifecycle` over a real `GatewayIdentitySlot` and `EvaluationBroker`, the production `GatewaySvidLifecycle` reconciler (its `hydrate_*`/`reconcile` produce the `Action` passed to `dispatch_issue`/`dispatch_drop`; the driver never constructs an action), a Sim CA, `SimObservationStore` and `SimClock` at their driven ports. `audit_fail` is a typed write fault on `SimObservationStore`, an existing driven port.
- **`NoAllocationIdentity`** is not modelled and not projected: no slot or lifecycle action takes or produces an `AllocationId`, so the property is a type-level fact of the interface (`GatewayIdentitySlot` is keyed by node, ADR-0225). DISTILL carries it as a compile-time obligation only if the contract does not already forbid it: the slot's public API has no `AllocationId` parameter, so no trybuild fixture is added (a fixture would test the absence of a parameter the type never had). Dropped as an obligation.

### Module `gateway_application` — decision ADR-0222 (drain and supersession only)

- **Protocol modelled.** The concurrent part of `GatewayApplicationOwner`: a new generation is staged, probed, promoted and the prior generation moves to Draining; connections hold leases on their generation; a draining generation is retired only when its lease count is zero; a probe result that arrives after its staged generation was superseded is discarded.
- **Variables and pinned accessors.** `current_generation`, `draining_generations` ← `GatewayControl::application_status() -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError>`. Model-only ghosts, not projected by the driver: `staged` (probe result per staged token), `leases` (per-generation connection counts).
- **Actions → driving ports.** `route_declare`/`route_withdraw` → `PublicRouteSetHandle` via `GatewayControl::routes()` (these trigger staging and supersession); `probe_result` → `GatewayUpstreamConnector::probe` over the Sim dataplane; `open_connection`/`close_connection` → `GatewayAdmission::acquire_connection` / lease drop.
- **Invariants.** `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` (both defend ADR-0222's single decision: the owner moves the prior generation to Draining, retires it after its users reach zero, and never restores a superseded generation).
- **Named runs.** `replace_valid` (supersede, drain, retire) and `route_withdraw_drain` (withdraw, drain, retire); one of the two, or a new `late_probe_after_supersession` if neither exhibits it, must show a probe result for a superseded staged token being discarded.
- **Hazard (one per decision).** ADR-0222: `retireWhileLeased` (retire a draining generation while its lease count is non-zero) → `DrainRetainsUntilLeasesZero`.
- **Not modelled (DISTILL seeded-sim or unit obligations instead).** Route/frontend/key whole-object coherence (`CurrentIsWholeObject`; a single task publishes one immutable value, a pure construction), bind gating (`ListenerBoundImpliesGates`, ADR-0230), last-usable preservation (`ReplacementPreservesLastUsable`, ADR-0231; a single-owner sequential install), identity-loss unbind, cold-boot order, and all shutdown ordering (`UsersJoinedBeforeIdentityRetire`, `IdentityRetiredBeforeConvergenceStops`, `shutdown_order`; the shutdown actions and stage projection are removed).

### Module `selection_receipt` — decisions ADR-0228, ADR-0232, ADR-0233, ADR-0241, ADR-0243 (unchanged by r1, except the P-9-dependent items)

- **Variables and pinned accessors.** `identity[id]` ← `GatewayConnectDataplane::selected_backend_identity(BackendId) -> Result<SpiffeId, ..>`. `intents`, `receipts` ← `live_intent_count()`, `live_receipt_count()`. The receipt value ← `take_receipt(&intent)`. `outer_committed` and `reserved` are model-only ghosts (the kernel map generation is assumption A-K1; the driver does not project them).
- **Actions → driving ports.** `update_begin`, `commit`, `rollback` → `Dataplane::update_service` (success, injected pre-commit failure, injected post-commit cleanup failure); `open` → `GatewayConnectDataplane::open_upstream`; `kernel_connect` → `GatewayUpstreamConnector::connect` (the hook selection is the Sim adapter's modelled arm; the real hook is proven at Tier 3 against the same module's runs); `take`, `cleanup` → the port methods; `restart` → drop and re-create the dataplane instance.
- **Invariants.** `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `RollbackPreservesPriorGeneration`, and `EmptyIsNoBackend` (no outer entry or no usable backend in the chained lookup yields `NoBackend`). **Pending P-9:** the healthy-dependent part of the former `UnhealthyOrEmptyIsNoBackend` is not in the current spec or `checks.toml`. A healthy-dependent invariant and an `unhealthy_is_no_backend` run are added by the modeler only if the user chooses "honour `BACKEND_MAP.healthy`".
- **Named runs.** `commit_then_select`, `abandoned_reservation_rolled_back`, `conflicting_alloc_rejected`, `receipt_waits_for_publish_guard`, `restart_destroys_receipts` (13 runs in total across the three modules).
- **Hazards.** ADR-0233: publish `Applied` before the outer swap. ADR-0241: rebind a live id to another allocation. ADR-0232: take a receipt that is not bound to its registered cookie and `ServiceKey`.
- **Conformance owner (D-PASS7-2).** The conformance owner is the real `EbpfDataplane` (it implements both `Dataplane` and `GatewayConnectDataplane`), on native metal, with the real `cgroup/connect4` selection arm loaded. No `Sim*` adapter is the owner or a projection source: a Sim-only owner would compare the model with the Sim's own modelled arm. Driver: `crates/overdrive-dataplane/src/quint_selection_receipt.rs`, `#[cfg(all(test, feature = "integration-tests"))]`, run through `cargo xtask metal run --` (root, bpffs, cgroup v2). It is `#[ignore = "pending DELIVER quint-conformance-selection_receipt"]` until the selection arm (P-10) and the owner are GREEN.
  - **Conformance main (modeler change).** A new main `receipt_conf` records `conf_step` over a *coarsened* step relation, because only observable owner transitions can be replayed: (1) `update_apply(key, sidx)` fuses `update_begin; commit(true); publish` (the write guard is held across a single non-yielding synchronous section, so the intermediate states are not observable and cannot be paused without a test seam); (2) `update_rejected` stays; (3) `open`, `kernel_connect(c)`, `take(c)`, `read_identity(c)`, `cleanup(c)`, `restart`, `idle` stay; (4) `rollback` and `commit(false)` are removed from this main; (5) `take(c, r)` is restricted to `r == c` (a mismatched receipt cannot be requested through `take_receipt(&intent)`); (6) the conformance catalog contains only empty and singleton backend sets, so the Maglev `pick` is determined by the state and is not an input the driver cannot control. The fine-grained `receipt_ok` main and all hazards are unchanged.
  - **Action to driving-port map.** `update_apply`, `update_rejected` → `Dataplane::update_service` (rejection is the real `DataplaneError::BackendIdentityConflict` from a conflicting backend set); `open` → `GatewayConnectDataplane::open_upstream`; `kernel_connect` → the driver issues the `connect(2)` on the socket returned by `PreparedUpstream::into_parts` toward the Service frontend (the same single syscall `GatewayUpstreamConnector::connect` issues; the connect result is not asserted for success, the hook has already written the receipt). The driver's `connect(2)` is deadline-bounded: a non-blocking connect with a bounded wait; an absent listener yields a refused/timeout outcome mapped to the model's failed dial, so a missing listener cannot stall the driver; `take` → `take_receipt`; `read_identity` → `selected_backend_identity`; `cleanup` → `cleanup`; `restart` → drop and re-create `EbpfDataplane` on the same pin directory (SR-2); `idle` → no call.
  - **State projection (owner accessors only).** `identity[id]` ← `selected_backend_identity(BackendId)` (`Ok` → bound, `BackendIdentityMissing` → absent); `intents`, `receipts` ← the *counts* `live_intent_count()` / `live_receipt_count()` (the spec projection for these two is the map size); the receipt value (`taken`) and `idres` are not projected State: they are checked from the port returns (`take_receipt(&intent)` at the `take` step, `selected_backend_identity` at `read_identity`) against the model's expected result for that step, consistent with the spec README. Cookie-indexed bookkeeping (`c` → held `GatewayConnectIntent`) is driver input state and is never projected. `outer_committed` and `reserved` stay model-only ghosts.
  - **Not conformance-replayable, and how they are covered.** `abandoned_reservation_rolled_back` and the `rollback`/`commit(false)` actions: the pre-commit and post-commit failures need a fault the owner cannot be handed through an existing driven port. They are covered by a Tier-3 test in `overdrive-dataplane/tests/integration/` that uses a real, production-reachable pre-commit failure produced through the existing `Dataplane::update_service` input (the only production-reachable pre-commit failure input is `BACKEND_MAP` capacity, `crates/overdrive-bpf/src/maps/backend_map.rs:46` `MAX_ENTRIES = 65_536`, so the update fails at `ServiceApplyStage::BackendMap`; filling it on native metal costs an unmeasured runtime, to be measured by DISTILL before the user decides. **This Tier-3 fill is an accepted-exposure candidate requiring the user's acknowledgement:** if not run, `RollbackPreservesPriorGeneration` has model + equivalence/unit coverage only. No seam is added) and asserts through the same accessors: prior generation preserved, no Applied identity for the abandoned ids, later fresh ids not recycled, zero residue. Post-commit cleanup debt does not affect any projected variable and is not conformance-checked (`service_cleanup_debt` is covered by the equivalence/unit lane). `receipt_waits_for_publish_guard`: the guard window cannot be held open without a test seam, so it is covered by a Tier-3 concurrency test (updates racing `selected_backend_identity` for a just-selected id; the read never returns a missing identity for a `Selected` id) plus the `publishEarly` hazard. The `takeAny` hazard has no driver path by design. Both remain model-checked runs/hazards: `abandoned_reservation_rolled_back` and `receipt_waits_for_publish_guard` run on a new `receipt_fine` main (REC=false, MAXUPD=3), not `receipt_ok` (which is bounded at 2 for the exhaustive check).
  - **Bound runs.** `#[quint_test]` for `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` (re-stated against `receipt_conf` in the modeler change above); `#[quint_run]` randomized traces on `receipt_conf`. The host/Sim `GatewayConnectDataplane` equivalence test remains the Sim pairing; it is not a conformance test.

### Model-check results (run on native metal; recorded evidence)

Spec, checks and evidence: [`specs/quint/public-ingress-gateway/`](../../../specs/quint/public-ingress-gateway/README.md), `checks.toml`, `evidence/`. The evidence was re-recorded for the reduced scope after the P-9 variant check and the dead identity hazard switches were removed: 21 checks, all as expected, and `verify-evidence` passes (`evidence/summary.md`; see the validation budget below).

Reduced-scope critical scenarios and their named runs:

| Critical scenario | Named run (module) |
|---|---|
| Identity issue and hold (happy path) | `issue_hold` (`gateway_identity`) |
| Near-expiry reissue | `reissue_near_expiry` |
| Disable races an in-flight issue | `disable_races_inflight_issue` |
| Audit failure leaves no hold | `audit_failure_no_hold` |
| Restart reissue from the empty volatile slot | `restart_reissue` |
| Supersede, drain, retire | `replace_valid` (`gateway_application`) |
| Withdraw, drain, retire | `route_withdraw_drain` |
| Late probe result after supersession is discarded | one of the above, or `late_probe_after_supersession` |
| Commit then select | `commit_then_select` (`selection_receipt`) |
| Abandoned reservation rolled back | `abandoned_reservation_rolled_back` |
| Conflicting allocation rejected | `conflicting_alloc_rejected` |
| Receipt waits for the publish guard | `receipt_waits_for_publish_guard` |
| Restart destroys receipts | `restart_destroys_receipts` |

| Decision | Hazard (violates) | Invariant that fails |
|---|---|---|
| ADR-0225 | `noEpochCheck` | `StaleEpochNeverHolds` |
| ADR-0222 | `retireWhileLeased` (new, replaces `separateSwap`, `installInvalid`, `bindBeforeProbe`, `convergenceFirst`) | `DrainRetainsUntilLeasesZero` |
| ADR-0233 | `publishEarly` | `IdentityReadableOnlyAfterCommit` |
| ADR-0241 | `rebind` | `IdentityImmutablePerBackendId` |
| ADR-0232 | `takeAny` | `ReceiptMatchesIntent` |

Validation budget (design.md cost budgets): 3 modules; 21 checks = 3 exhaustive TLC safety checks (identity, application, receipt) + 5 hazards + 13 named runs (down from 30). Only `receipt-safety-tlc` is slow (about 970 s; non-CI; timeout 2400 s; bounded at 2 accepted `update_service` calls on `receipt_ok`; the two runs above execute on the `receipt_fine` main, REC=false, MAXUPD=3); the other 20 checks are CI and each completes in seconds; the full recorded run is about 15 min wall at 16 jobs. The P-9 variant run and the dead identity hazard switches are removed from the spec; a P-9 variant check is added only if the user chooses "honour". Growth beyond this budget needs user approval. Open points and their resolution: SR-2 (answered from source in the assumptions table); GA-1 and SR-2 discharging items (DISTILL obligations in the table; GA-2 dropped, RAII); `NoAllocationIdentity` (type-level, dropped). The shutdown projection and shutdown actions are removed from the model.

### Assumptions and their discharging items

| Id | Assumption | Discharged by |
|---|---|---|
| A-K1 | The outer `SERVICE_MAP` slot replacement is atomic for the hook's lookup; a hook that sees a new inner map sees fully populated slots. | V-GW-1, existing atomic-swap Tier-3 (`.claude/rules/testing.md`) |
| A-K2 | The pre-connect `SO_COOKIE` equals the cookie the hook reads for the same socket. | V-GW-5 HELD (10/10 plain, 10/10 prepared, 7.0.0-38; source: `net/core/sock.c:2109-2113`, `net/core/filter.c:5151-5154 @ v7.0`) |
| A-K3 | A socket created by a `serve` task belongs to the attach cgroup subtree. | Source-settled: the socket's cgroup is fixed at allocation from the creating task (`cgroup_sk_alloc` takes `task_css_set(current)->dfl_cgrp`, `kernel/cgroup/cgroup.c:7420-7431`, called from `sk_prot_alloc` in `net/core/sock.c:2330 @ v7.0`) and `connect4` runs against that socket cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`); with all 19 `serve` threads observed in `/overdrive.slice/control-plane.slice` (V-GW-2) the attached hook fires. The end-to-end serve-created connect is exercised by AT-PIG-E2E-1 (DISTILL native obligation). |
| A-3 | Every Service backend address lies in the guest prefix. | ADR-0190 consequence; revisited by GH #317 |
| GI-1 | `issue_and_audit` is the existing CA issue-and-audit path; an audit failure leaves no held material. | `audit_failure_no_hold` run replayed against the production path with a failing Sim `ObservationStore` write (DISTILL obligation `S-PIG-51` / the `ca_issuance` tests) |
| GI-2 | The injected clock is monotonic. | `SimClock` contract (unit) |
| GA-1 | The connector probe result is delivered at most once per staged token. | Discharged by the probe owner's own unit-level test: completing a staged probe twice leaves the second ignored with the status row and application generation unchanged. The mechanism that makes duplicates impossible is the crafter's (`.claude/rules/rust.md` check-and-act applies). |
| GA-2 | An admitted connection holds one lease for its whole life and releases it exactly once. | Dropped as an obligation: the lease is an RAII guard, so release is single by construction. |
| SR-1 | The hook writes exactly one receipt per registered cookie and none for an unregistered one. | S-PIG-50 and the Tier-3 V-GW-1 run (control (d) observed `receipt=None` for an unregistered cookie on 7.0.0-38) |
| SR-2 | A dataplane restart empties the Service maps, the `BackendIdAllocator` memo, the selection receipts and the applied-identity associations together, and the maps are rebuilt by re-hydration. | **Source:** `EbpfDataplane::new_with_pin_dir` creates the directory, unlinks a stale `SERVICE_MAP` pin and pre-creates a fresh outer map (`M:crates/overdrive-dataplane/src/lib.rs:430-454`); the allocator starts at counter 1 with an empty memo and "all services are re-hydrated by the `ServiceMapHydrator` ... rebuilding the memo from scratch" (`M:crates/overdrive-dataplane/src/allocators/backend_id.rs:9-18`); the gateway intent/receipt maps are not pinned and live with the instance (feature-delta Backend programming and selection, D3); the applied-identity associations are dataplane-instance state ([ADR-0233](../../product/architecture/adr-0233-commit-gated-backend-identity-publication.md)). The one dataplane instance lives with the `serve` process, so a dataplane restart is a process restart. A selection attempted before re-hydration completes sees no outer entry and is `NoBackend` (503). DISTILL obligation: a `restart_destroys_receipts` replay plus a Tier-3 check that after `serve` restart the gateway returns 503 until the hydrator re-programs the Service, then the backend response. |

## Wave: DESIGN / [REF] Contract-shaping validation items

Each item's falsification changes a contract this document pins, so each runs as a spike during DESIGN, on native metal (`cargo xtask metal run --`), before the design is approved. `cgroup_sock_addr` has no `BPF_PROG_TEST_RUN` (`.claude/rules/spike.md`), so only a real `connect()` is a signal. None is carried into DISTILL as Open.

| Id | Hypothesis / prediction | Falsification | Contract it shapes |
|---|---|---|---|
| V-GW-1 | A socket prepared per step 1-3 and connected to the VIP of the D4 VM Service gets the hook's `Selected` receipt, the rewritten dial enters leg-C through table 100 and TPROXY, and a client TLS handshake with the allocation's SVID completes and carries bytes to the guest. | `EPERM` without a `NoBackend` receipt, `ENETUNREACH`, a reset at the nft output rule, a handshake failure, or no bytes at the guest. | The whole upstream path; the dataplane-adapter preparation contract (ADR-0243); selection hook (ADR-0224). Prerequisite for A-K1. |
| V-GW-2 | `serve`'s own tokio worker threads create sockets inside `/sys/fs/cgroup/overdrive.slice`, so the attached hook fires for them. The stale comment at `M:crates/overdrive-dataplane/src/mtls/mod.rs:928-931` says the agent is outside the attach subtree; the attach path says otherwise. | The hook does not fire for a `serve`-created socket. | Whether the gateway needs a dedicated cgroup (ADR-0224 alternative B) — a different packet-entry decision. |
| V-GW-3 | The registered arm returns `NoBackend` when there is no outer entry or no usable backend, and `Selected` once the backend is programmed; after `job stop` a still-present entry yields `Selected` then a connect reset (502). (Whether `healthy == 0` is also `NoBackend` is the pending user decision P-9; V-GW-3 observed the byte honoured by the probe arm and flapping 1 → 0 in 2 of 3 runs.) | A stopped backend produces a wrong-peer success. | The consuming-port outcome table (ADR-0242). |
| V-GW-4 | The chained `SERVICE_MAP` (hash-of-maps) → inner lookup verifies in a `cgroup_sock_addr` program and its verified-instruction count stays within the Tier-4 policy. | Verifier rejection, or a budget growth the policy does not authorize. | Whether the registered arm is folded into the existing hook or becomes a second program (P-10, ADR-0224). |
| V-GW-5 | The userspace `SO_COOKIE` before `connect(2)` equals `bpf_get_socket_cookie` in the hook. | Different values. | The intent key (ADR-0232). |

### Verdicts (run 2026-11-01, native metal, kernel 7.0.0-38-generic, tree `549d78fa6`)

Evidence: [findings-gateway-connect-path.md](spike/findings-gateway-connect-path.md) (V-GW-1..5) and [findings-vm-service-map-programming.md](spike/findings-vm-service-map-programming.md) (D4). Probes are throwaway under `spike-scratch/pig-vgw/` (aya Rust).

| Id | Verdict | What was observed | Contract effect |
|---|---|---|---|
| V-GW-1 | **PARTLY HELD**; remainder settled by source reading (below) | Selected receipt, rewrite to `100.95.0.2:18080`, `fwmark 0x1` + `127.0.0.1` source, accepted socket on leg-C (orig dst the guest address), TLS 1.3 server flight with the allocation chain. Not observed: client-SVID handshake and bytes at the guest (the dialer had no client certificate; leg-C refused `peer sent no certificates`). Control: the premise "an unprepared socket cannot reach leg-C" is false on a host with a default route (rationale corrected in step 2 above). | Preparation contract unchanged, rationale rewritten. |
| V-GW-2 | **HELD** (membership + proxy process, plus kernel source for the socket-cgroup rule) | All 19 `serve` threads in `/overdrive.slice/control-plane.slice`; the slice-attached hook counted a probe moved into that cgroup and not one outside. No connect created by a `serve` thread was observed (serve made none in the window); that gap is closed by source: a socket's cgroup is its creating task's cgroup at allocation (`kernel/cgroup/cgroup.c:7420-7431`; `net/core/sock.c:2330`; hook lookup `kernel/bpf/cgroup.c:1694 @ v7.0`), so membership implies firing. Contract shape source-settled; end-to-end observation of a serve-originated connect is a DISTILL native obligation (AT-PIG-E2E-1). | ADR-0224 alternative B (dedicated cgroup) is not needed; caveat recorded in ADR-0224 consequences; the stale comment at `mtls/mod.rs:928-931` is flagged there. |
| V-GW-3 | **HELD** | `NoBackend/no-outer-entry` → `NoBackend/healthy==0` → `Selected` about 1.05 s after the startup `Stable`; after `job stop` the entries persisted (about 12 s in run4, about 5 min in run2) and the result was `Selected` then `ECONNREFUSED`, never a wrong peer. `healthy` flapped 1 → 0 in 2 of 3 runs, cause not established. | ADR-0242 outcome table holds for the empty/no-outer-entry (503) and selected-then-dial-failure (502) rows; the `healthy == 0` row is pending P-9. The flap is an **observed exposure**: the gateway treats `healthy` as possibly flapping and never reads `Selected` as liveness. Recommend a separate investigation issue; it is not gateway scope (needs user approval before creation, as for P-4). Run4's V-GW-1 dials used a forced `healthy=1`. |
| V-GW-4 | **HELD** (verification) | The `SERVICE_MAP` → inner → `BACKEND_MAP` chain verifies; `vgw_select` 246 verified instructions vs 28 for the existing `cgroup_connect4`; production plus two probe programs coexisted under one cgroup (`multi`). | Placement is the user decision P-10. |
| V-GW-5 | **HELD** | Pre-connect `SO_COOKIE` equals the hook's `bpf_get_socket_cookie`, 10/10 plain and 10/10 prepared sockets. | ADR-0232 intent key sound on 7.0. |

**V-GW-1 remainder — does leg-C accept the gateway's dedicated SPIFFE identity? Settled by source.** Leg-C's server config is `tls_config::server_config`, which builds `WebPkiClientVerifier::builder(roots).build()` over a root store containing the trust bundle's root anchor only (`M:crates/overdrive-dataplane/src/mtls/tls_config.rs:249-269`, `root_store` `:68-86`), and `inbound::server_handshake` adds only a non-empty-peer-certificates guard (`M:crates/overdrive-dataplane/src/mtls/inbound.rs:508-516`). Neither compares the client certificate's SPIFFE URI SAN or any allowlist; the verifier is authentication-only (the `expected_peer` equality is #242 and the module comment says so, `:116-129`, `:224-228`). The gateway SVID is minted by the workload-identity CA through the Issue path pinned in ADR-0225, so it chains `leaf → node intermediate → root`, and the gateway client presents `[leaf, intermediate]` (the same `present_chain` shape, `:99-114`). Leg-C therefore accepts any bundle-chained, valid-signature client leaf, including the gateway's dedicated identity. One constraint the source adds: the workload SVID carries no extended-key-usage restriction today (no EKU is set in non-test CA issuance code; `rustls` `WebPkiClientVerifier` rejects a leaf whose EKU is present and lacks `clientAuth`), so the gateway SVID MUST NOT introduce an EKU that omits `clientAuth`; ADR-0225 states this. This settles the contract shape: the exact-peer equality lives on the gateway side (ADR-0234, `HostGatewayClientMtls`), not at leg-C. The remaining behaviour, kTLS arm and vsock relay to the guest, is main's existing proven mesh path ([ADR-0145](../../product/architecture/adr-0145-vsock-kernel-forwarded-guest-transport-replaces-shared-bridge.md) lineage, same code path the observed server flight already ran). **Client-side presentation is also source-settled.** Main already presents allocation SVIDs minted by the same workload CA as TLS client certificates on the egress leg-B → peer leg-C hop in production: `outbound::client_handshake` builds `tls_config::client_config(svid, bundle)` (`M:crates/overdrive-dataplane/src/mtls/outbound.rs:166-174`), which calls `with_client_auth_cert(certs, key)` over `present_chain` (`M:crates/overdrive-dataplane/src/mtls/tls_config.rs:229-244`) and then completes the handshake against the peer's leg-C `server_config`. The gateway client uses the same shape (`[gateway leaf, node intermediate]`), and the gateway SVID is issued through the same `ca_issuance::issue_and_audit` → `Ca::issue_svid` path (`M:crates/overdrive-control-plane/src/ca_issuance.rs:183`; `M:crates/overdrive-host/src/ca/rcgen_ca.rs:391`), in which no extended-key-usage is set (no EKU construct appears in `rcgen_ca.rs`). Leg-C acceptance of a bundle-chained client leaf is therefore source-settled by an existing production path. The residual is only the end-to-end composition (SVID-bearing gateway client to leg-C to bytes at the guest), which is a **DISTILL Tier-3/native obligation** (AT-PIG-E2E-1), not an open contract-shaping item; no user decision is needed because the contract (client presents chain; server verifier authentication-only) is unchanged by any outcome of that test.

## Wave: DESIGN / [REF] Handoff and open questions

This is the handoff record of the reconstructed DESIGN. The DDD items below
are resolved in this reconstruction; the application items remain required
decisions pinned by this document, not permission for a crafter to invent
surface and not optional feature deferrals.

### DDD stage — binding constraints from the system stage

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
- Gateway Application publishes nothing to ServiceMapHydrator. A superseded
  generation is retired after its admitted connections drain; there is no
  demand union, revision, acknowledgment or wake (D2).

### Application architecture stage

- Pin the exact `overdrive deploy <ROUTE_SPEC>` discrimination, HTTP API, intent
  keys/envelopes/codecs, config grammar, status rows and typed errors. A missing shape is a DESIGN
  blocker, never implementation latitude.
- Pin exact signatures for the selected topology: gateway single-slot identity
  holder/use; dataplane-adapter upstream socket preparation (mark, loopback
  bind, `SO_COOKIE`, registration); Dataplane register/consume/cleanup of exact
  ServiceKey intent and BPF outcome; BackendId→exact applied `Backend.alloc`
  association; and a gateway-client entry under the existing mTLS transport
  owner carrying gateway material + exact expected peer. Do not add userspace
  selection, raw handler map access, a child-cgroup API, or a fabricated
  Allocation identity.
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
- `GatewayOrdinaryServiceMapProgramming` (replaces the retired demand/TEACH
  obligation): a gateway-enabled and a gateway-disabled boot program a VM
  backend into `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` byte-identically; the
  gateway never changes `Action::DataplaneUpdateService` or the hydrator's
  classification.
- `GatewayUnavailableOutcomeIsTyped` (D2): see
  [Consuming-port availability contract](#wave-design--ref-consuming-port-availability-contract-d2).
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
  map-of-maps lookup verifies, registered failure denies, the selected VM
  address reaches leg-C through the marked loopback route and TPROXY, and
  unregistered local/non-service connects remain byte-for-byte behaviorally
  unchanged.
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

The DDD stage preserves D-SYS-1…D-SYS-11 and records the Route choice in
[ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md)
plus independently reversible Domain choices in
[ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md),
[ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md),
[ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md)
and [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md),
carrying their 2026-09-13 user-ratification provenance where the decision is unchanged and
**Proposed — reconstruction on main pending independent review (2026-11-01)** as
restated. GitHub issue #54 plus the user's corrections govern; the
whitepaper is contextual, the missing DISCUSS/SPIKE artifacts are not invented,
and no absent story, KPI or capacity claim is inferred.

| ID | Domain verdict |
|---|---|
| **D-DOM-1** | Create **Public Ingress** and **Public Certified-Key Custody** bounded contexts; extend existing **Service Dataplane** and **Workload Identity** contexts. Public Route Set, Public Certified Key and Gateway Identity Slot are the only new aggregates. |
| **D-DOM-2** | A Route has one exact Public Hostname, one exact-or-segment-prefix Path Match, one exact TCP Service Listener Reference and one Public Certified-Key Reference. |
| **D-DOM-3** | Route intent references the stable Service listener as `(WorkloadId, port, tcp)`. Resolution yields the exact existing Service Frontend `(ServiceVip, port, tcp)`; the Route never stores derived `ServiceId`, VIP, backend address, Allocation ID or SPIFFE ID. |
| **D-DOM-4** | [ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md): the singleton Public Route Set makes first-slice one-Route cardinality/ownership atomic on one aggregate key. A different Route ID cannot claim an occupied slot; it is rejected rather than last-write-wins. |
| **D-DOM-5** | [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md): create the distinct **Public Certified-Key Custody** context and **Public Certified Key** aggregate so Route, operator HTTPS, internal SPIFFE and allocation `IdentityMgr` remain non-owners. Protection/replacement/currentness mechanics belong to their decision-specific System/Application records and exact feature contract. |
| **D-DOM-6** | Manual operator input is the sole first-slice Certified Key producer. ACME account/order/challenge/renewal and TCP/80 are absent; GH #57 may later invoke the same custody command without changing runtime consumption. |
| **D-DOM-7** | [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md): Workload Identity owns a dedicated single-slot **Gateway Identity Slot** and **Gateway Identity Lifecycle** for `spiffe://overdrive.local/gateway/<node-id>`. It reuses issue/audit/profile policy, renews/restarts/drops explicitly, permits only gateway-client mTLS use and never fabricates `AllocationId`. |
| **D-DOM-8** | [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md): `GatewayApplicationOwner` atomically publishes one Route/frontend/opaque-resolver generation, retains it per connection and retires a superseded generation after its connections drain. It publishes no demand to any other owner; backend unavailability is a typed outcome at the connector ([ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)). |
| **D-DOM-9** | ServiceLifecycle remains sole Backend Eligibility publisher and existing Service Dataplane/BPF remains the backend selector. [ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md) assigns only the transient BPF Selection Receipt to Service Dataplane. The gateway never selects or reads backend rows/maps. |
| **D-DOM-10** | A Gateway Connect Intent is exactly `(Socket Cookie, ServiceKey)`. The cgroup/connect4 registered arm (placement P-10) chooses through existing `SERVICE_MAP` Maglev + `BACKEND_MAP`; its receipt is `Selected(BackendId)` or `NoBackend`. This host-connect path is distinct from unchanged XDP wire forwarding. |
| **D-DOM-11** | [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md): Selected Backend Identity is the receipt's `BackendId` resolved through Service Dataplane's exact Applied association. Gateway-client rustls presents the Gateway SVID and requires exact peer SPIFFE SAN equality before body delivery. |
| **D-DOM-12** | `Route Accepted`, `References Resolved`, `Certified Key Usable`, `Gateway Identity Current`, `Gateway Applied Generation`, `BPF Hydrated`, `Public Listener Bound` and `Selected Peer Authenticated` are distinct owned promises; none is shorthand for `Ready`/`Healthy`/`Programmed`. |
| **D-DOM-13** | State-based model (the rejected Event Sourcing/CQRS alternatives are recorded in [ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md); the former ADR-0221 is retired): Public Route Set, Public Certified Key and Gateway Identity Slot are state-based; BackendId identity association and connect receipts are derived/process-local state. No Event Sourcing, CQRS subsystem, ACME workflow, new deployed process or userspace load balancer is introduced. |

## Wave: DESIGN / [REF] Bounded contexts and context map

| Bounded context | Classification | Owns | Explicit non-ownership |
|---|---|---|---|
| **Public Ingress** | Core, **CREATE NEW** | Public Route Set; Route acceptance/withdrawal; exact Service-frontend resolution; Gateway Application lifecycle; Gateway Upstream Connector and its availability outcome; Public Listener and HTTP/1.1 semantics | Does not acquire certificates, decide Backend Eligibility, read backend rows, select backends, own BPF state or issue SVIDs |
| **Public Certified-Key Custody** | Supporting, **CREATE NEW** | Producer-neutral protected Public Certified Key aggregate; chain/key/hostname validation; preserve-old atomic replacement; validity/expiry currentness; redaction boundary | Has no first-slice withdrawal command; does not know manual paths after ingestion or own ACME account/order/renewal, Route, listener, request or internal SVID |
| **Workload Orchestration / Service Lifecycle** | Existing core, **REUSE** | Service aggregate/Listener intent; Allocation lifecycle; ServiceLifecycle Backend Eligibility publication | Does not expose a public Route or serve gateway requests |
| **Service Dataplane** | Existing core, **EXTEND OWNER** | Service Frontend/ServiceKey; BPF generations (ordinary hydrator application); upstream-socket preparation; Gateway Connect Intent/BPF Selection Receipt; process-lifetime Applied Backend Identity Association; cgroup-BPF selection and unchanged XDP forwarding | Does not own Route/key generation, public HTTP matching, gateway SVID custody or userspace selection |
| **Workload Identity** | Existing supporting, **EXTEND OWNER** | Existing CA/audit/allocation identity; dedicated Gateway Identity Slot/Lifecycle; gateway-client mTLS and exact selected-peer equality | Does not own public Web-PKI, Route, Backend Eligibility or BPF selection |

`IntentStore`, `ObservationStore`, holder state and reconciler View memory are
non-substitutable state contracts, not bounded contexts. Intent holds Public
Route Set; protected credential state holds Public Certified Key; Observation
holds Service/dataplane, custody/application and issuance-audit facts. Gateway
SVID material, BackendId identity association and connect
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
| **Gateway Connect Intent** | One transient Dataplane registration `(Socket Cookie, ServiceKey)` before connect | Backend candidate/id/identity, Route or durable state |
| **BPF Selection Receipt** | One transient registered-connect result `Selected(BackendId)` or `NoBackend`, correlated to the exact cookie and ServiceKey | Gateway-selected backend, Backend Eligibility or XDP packet fact |
| **Applied Backend Identity Association** | Dataplane-owned process-lifetime immutable `BackendId → exact applied Backend.alloc (SpiffeId)` join installed before Service-map swap | Address-based reread, Route state or allocation holder |
| **BackendId** | Existing process-local BPF backend token selected from Maglev and carried by the receipt | SPIFFE identity, AllocationId, address or gateway choice |
| **Selected Backend Identity** | The exact `Backend.alloc` resolved from the BPF receipt's actual BackendId for one connected socket | An arbitrary eligible backend, gateway prediction or asynchronous `BackendIndex` reread |
| **cgroup-BPF Service Selection** | Host-originated registered socket selection/rewrite by ancestor-attached `cgroup_connect4_service` through existing Maglev/BACKEND maps | XDP ingress, userspace round robin or new child cgroup/netns |
| **XDP Wire Forwarding** | Existing packet-ingress forward/reverse path on its current interfaces, unchanged by gateway selection | The gateway host-connect selection path |
| **BPF Hydrated** | ServiceMapHydrator/Dataplane applied its independent backend generation on its ordinary path | Gateway Applied Generation or proof a backend exists |
| **Selected Peer Authenticated** | Gateway-client mTLS verified exactly the receipt-selected workload SPIFFE SAN before application bytes | Chain-only verification, gateway preselection or persistent state |
| **Gateway Identity Current** | Gateway Identity Slot holds a time-usable SVID for the exact node gateway subject | Issuance-audit presence, allocation SVID currentness or public-key usability |

Precise promises are **Route Accepted**, **References Resolved**, **Certified
Key Usable**, **Gateway Identity Current**,
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
| **Declare Route** | Slot empty; values valid | `None → Some(full Route)`; `RouteDeclared`, then `RouteAccepted` | Service, Certified Key, Gateway Identity, Gateway Application and BPF state unchanged until downstream owners react |
| **Replace Route** | Slot occupied by same Route ID; replacement valid | Replace hostname/path/target/key-ref + Route Generation atomically; `RouteReplaced`, then `RouteAccepted` | Aggregate identity, Route ID and all referenced/other state unchanged |
| **Withdraw Route** | Slot occupied by same Route ID | `Some → None`; `RouteWithdrawn` | Service, Certified Key, Gateway Identity and BPF state unchanged until downstream Gateway Application performs ordered admission drain |
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
| Gateway application | Apply / Supersede / Withdraw / connection drain | Staged candidate, Current and Draining generations; Gateway Applied Generation or cause-distinct unavailable | Validate the whole candidate, swap atomically, retain current per connection, retire Draining after drain |
| Backend application | Eligibility row change | Backend Eligible; BPF Hydrated; Applied Backend Identity Association | ServiceMapHydrator applies on its ordinary path; Dataplane associates identity before Service-map swap |
| Gateway connect | Prepare+register `(Socket Cookie, ServiceKey)` / connect / consume | BPF Selection Receipt; Selected Backend Identity or NoBackend | Dataplane adapter prepares the socket; Public Ingress connector owns `connect(2)` and consumes cleanup on every return; Dataplane registry/hook owns transient state and selection |
| Internal trust | Connected socket + selected identity + current gateway identity | Selected Peer Authenticated or cause-distinct failure | Gateway-client rustls requires exact peer SPIFFE SAN; no transparent pump, plaintext or alternate peer |
| Public request | TLS + retained Gateway Application generation + Host/path | 404 Route miss; 503 NoBackend; 502 connect/receipt/identity/mTLS/upstream; upstream response otherwise | Public Runtime streams HTTP/1.1 with no retry/replay/userspace backend choice |

### Lifecycle boundary scenarios

| Scenario | Required outcome | Explicitly unaffected |
|---|---|---|
| **Feature disabled** | No Route owner, key consumer, gateway identity slot/lifecycle, prepared upstream socket, connect registry or public listener activates | Existing `serve`, operator HTTPS, Allocation `Running`, Service `Stable`, Backend Eligibility, cgroup/XDP and workload SVID behavior |
| **Enabled cold boot** | Load/validate Route/key, resolve frontend, issue/audit/hold gateway SVID, probe receipt/identity path, apply Gateway Application, then bind TCP/443; no earlier partial listener | Workload/Service lifecycles and independent dataplane convergence do not wait on public listener bind |
| **Route before target/key** | Route may be Accepted while References Resolved/Certified Key Usable/Application Applied remain absent with cause; listener stays unbound | Referenced Service, BPF and key aggregate are not mutated by Route acceptance |
| **Competing Route IDs** | One singleton-slot mutation wins; loser receives conflict with empty delta | Existing Route and all Service/key/identity/dataplane state |
| **Late old certified-key replacement** | Expected-generation check rejects stale candidate; cannot overwrite newer generation | Public Route Set, current key and BPF state complement-equal |
| **Route/frontend replacement** | The whole new candidate is validated before the admission swap; old generation becomes Draining and is retired after its connections drain | ServiceLifecycle authority and BPF generation remain independent |
| **Route deletion** | Withdraw Route intent only; stop new admission/accepts, move current application to Draining, retain it until accepted work drains, then retire it/unbind | Protected Public Certified Key remains in custody; Service Listener intent, Backend Eligibility, gateway/allocation SVIDs and operator HTTPS |
| **Certified-key refresh failure or expiry** | Missing/invalid/unusable refresh preserves the usable current generation. When that generation itself becomes Unusable/expired, stop new admission/TLS and publish the pinned status while retaining the protected record for a valid replacement | Route intent, Service/Backend Eligibility/BPF and Gateway Identity |
| **Gateway identity disable/expiry** | Stop new admission/upstream handshakes, drain authorized users, then drop/replace gateway SVID material | Route/Public Certified Key, Service/Backend Eligibility/BPF and allocation SVIDs |
| **No backend / unready backend** | Listener remains active when capability gates are sound; BPF receipt is NoBackend (no outer entry or no usable backend; `healthy == 0` only if P-9 is adopted), connect is denied and public response is 503; no wait (ADR-0242) | Route/key/identity generations and backend authority |
| **Missing/mismatched receipt** | No upstream bytes; cleanup intent/outcome and return 502 | Existing unregistered LOCAL_BACKEND_MAP/XDP traffic |
| **Wrong selected peer / disconnect** | Current request returns 502 before upstream body when possible; no plaintext, retry or alternate backend | Future requests and all lifecycle state |
| **Process restart** | Destroy connections/receipts before rebuilding BackendId associations and gateway SVID; rerun all boot gates; bind TCP/443 only after success | No cross-owner unified generation or BackendId rebinding |
| **Shutdown** | First `ServerHandle::shutdown` stage: stop accepts, withdraw current admission, drain with identity retained, join listener/connector, clear receipts, retire the closed application, then clear the volatile gateway SVID slot synchronously (D5 section (c)) | Existing workload/Service shutdown ownership |

**Gate ownership remains narrow.** Initial listener bind requires References
Resolved, Certified Key Usable, Gateway Identity Current, Gateway Connect
Path Trusted and Gateway Applied Generation.
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
| `ServiceMapHydrator` | Eligible-set application | **REUSE UNCHANGED (D1)** | VM backends already enter shared Service maps; D4 on metal |
| `SERVICE_MAP` Maglev inner + `BACKEND_MAP` | Selection tables | **REUSE** | Existing BPF selection; no gateway backend map/algorithm |
| `cgroup_connect4_service` ancestor attachment | Host connect selection | **EXTEND — REGISTERED ARM/RECEIPT** | Attached over `overdrive.slice` (V-GW-2 confirms `serve` is inside); unregistered sockets keep current `LOCAL_BACKEND_MAP` behavior and XDP wire path is unchanged |
| `BackendIdAllocator` | Selected backend identifier | **REUSE MONOTONIC-NONREUSE PROPERTY** | Process-lifetime non-rebinding is required for old receipts (`allocators/backend_id.rs:58-79`: the counter never recycles; `release` drops only the `(ip, port, proto)` memo). A re-leased guest address is P-2. |
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
| `GatewayApplicationOwner` | Atomic application | **CREATE DERIVED CAPABILITY, NO STORE** | Whole-object swap prevents torn generation; draining retention prevents early retirement |

| Context | Event Sourcing | CQRS/read-model verdict |
|---|---|---|
| Public Ingress | **No** — singleton Route and current application state need no temporal replay | Existing intent plus derived application/status views; no command/query bus |
| Public Certified-Key Custody | **No** — current protected generation and optimistic replacement suffice | Producer command and custody/runtime views use existing boundaries; no acquisition history |
| Workload Identity | **No** — current gateway slot plus issuance audit and retry inputs suffice | Holder currentness is not audit history; no workflow/event stream |
| Service Lifecycle/Dataplane | **No change** | Existing rows/maps plus transient receipt and process-lifetime identity projection remain autonomous |

## Wave: DESIGN / [REF] Domain alternatives and decision drivers

| Significant choice and drivers | Selected model | Viable alternatives rejected |
|---|---|---|
| One-Route atomic cardinality; public exposure independent from Service lifecycle | Singleton Public Route Set with optional Route entity; one IntentStore aggregate key owns slot conflict | Per-Route aggregates + global claim add a second consistency boundary; embedding PublicExposure in Service couples host/path/key withdrawal to workload/Backend Eligibility lifecycle |
| Web-PKI trust separation; protected rotation; manual now and ACME #57 later share consumption | Distinct Public Certified-Key Custody context/aggregate, reusing protection/resolver patterns | Route-owned key material makes every rotation a Route mutation; generalized allocation `IdentityMgr` mixes ephemeral SPIFFE state with protected hostname-bound Web-PKI |
| Exact non-allocation subject; auditable issue/reissue; one authorized consumer | Dedicated Gateway Identity Slot and sibling lifecycle, reusing `issue_and_audit`, audit/profile and SVID lifecycle patterns | Synthetic AllocationId falsifies lifecycle/audit; broad `IdentityMgr` key union expands every holder/read/action path and mixes node infrastructure with Running allocations |
| Direct currentness; no temporal query; transient receipts die on restart | State-based aggregates + reconcilers + derived views, reusing Intent/Observation/View boundaries | Event streams require secret/receipt replay, retention/upcasting and cross-owner ordering; CQRS adds projection lag and another source without scale/query need |

Detailed decision-specific trade-offs are in Route
[ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md)
and Domain
[ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md),
[ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md),
[ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md)
and [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md).
None changes manual certificate first, later GH #57 ACME, BPF-owned selection,
exact-peer mTLS or the active walking skeleton.

## Wave: DESIGN / [REF] Active flow and production acceptance handoff

System ADR-0220, 0223, 0224, 0229, 0230 and 0234 resolve the former `DESIGN-GAP-PIG-1`; it is not a remaining gate or
implementation permission. The complete active domain flow is:

```text
Route
  -> exact Service Frontend
  -> (independently) ServiceMapHydrator ordinary programming of the VM backend
  -> dataplane-prepared socket: SO_MARK, 127.0.0.1 bind, Gateway Connect Intent (SO_COOKIE, ServiceKey)
  -> cgroup/connect4 registered arm (placement P-10)
  -> SERVICE_MAP Maglev + BACKEND_MAP selection/rewrite (no usable backend -> NoBackend; healthy == 0 per P-9)
  -> BPF Selection Receipt Selected(BackendId) | NoBackend
  -> rewritten dial -> table 100 -> lo -> TPROXY -> leg-C -> kTLS -> vsock relay -> guest
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

Stage 3 preserves D-SYS-1…11 and D-DOM-1…13. The independently
reversible Application choices are recorded in ADR-0222, 0226, 0227, ADR-0231–0240
and ADRs 0242 and 0243; the table below links every decision to
its one-decision record. This feature delta—not those decision records—is the
implementation-contract SSOT for every exact Rust signature, variant,
persistence key, lifecycle ordering, error and verification handoff.
System ADR-0220, 0223, 0224, 0229, 0230 and 0234 resolve the former system gap; Application
pins active production contracts without a spike or placeholder seam.

| ID | Application verdict |
|---|---|
| **D-APP-1** | [ADR-0222] creates one non-deployable `overdrive-gateway` adapter-host library; `overdrive-control-plane` remains sole `serve` composition/ServerHandle owner and activates the real GatewayApplicationOwner/listener. |
| **D-APP-2** | Add parser-side `DeploySpecInput = Workload(WorkloadSpecInput) | Route(PublicRouteSpecInput)`, distinct HTTP `PublicRouteInput`, and persisted `Route`; never add Route to workload `SubmitSpecInput`, `WorkloadIntent`, `WorkloadKind`, `/v1/workloads`, or workload streaming. |
| **D-APP-3** | Persist the singleton `PublicRouteSetEnvelope::V1` at exact key `public-ingress/route-set`; persist even the empty Route set. A bounded `PublicRouteSetOwner` is the sole serialized writer. |
| **D-APP-4** | A Route carries exact validated `RouteId`, deterministic `RouteGeneration`, `PublicHostname`, `PathMatch`, `ServiceListenerReference(service: WorkloadId, NonZeroU16, Proto::Tcp)` and `PublicCertifiedKeyId`. The public target member is `service`; the internal identifier type remains `WorkloadId`, never allocator-derived `ServiceId`. |
| **D-APP-5** | A new read-only `ServiceFrontendResolve` port reads current Service intent plus the already-probed `ServiceVipView` and returns the exact existing `ServiceFrontend`; control-plane wraps its existing mutex-held allocator (`Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>`, `M:crates/overdrive-control-plane/src/lib.rs:319, 3810`; `PersistentServiceVipAllocator: ServiceVipView` at `M:crates/overdrive-dataplane/src/allocators/persistent_service_vip.rs:136`) in a private read adapter. It never reads backend/health/BPF state. |
| **D-APP-6** | `ServerConfig.gateway: Option<GatewayConfig>` is the only enablement gate. CLI `--gateway-address`, `--gateway-certified-key-id`, `--gateway-certificate-chain` and `--gateway-private-key` construct IPv4 TCP/443 only as an all-present set; all absent means disabled. Public bind occurs only after connect/mTLS/identity/application gates. |
| **D-APP-7** | Manual certificate/key paths are host-local `serve` configuration, like VM kernel/rootfs references only at the artifact boundary. The source validates then installs through producer-neutral custody; paths/raw PEM never enter Route, status or request state. |
| **D-APP-8** | [ADR-0223] makes Public Certified-Key Custody the one configured-ID producer-neutral owner; [ADR-0231] makes replacement validate/build/seal/persist/publish/status ordered so an unusable candidate never replaces the last usable generation. Its ID-free `install` accepts `Manual | Workflow { correlation }` provenance and is #57's future producer boundary. |
| **D-APP-9** | One active `GatewayApplicationOwner` atomically owns the staged candidate, one ArcSwap Current generation, N Draining generations, dynamic TCP/443 bind/unbind, RAII connection/request leases, the join set and application status. Shutdown force-joins leases before the SVID is released. |
| **D-APP-10** | `IntentStore::watch` changes in one cut to `Changed|Lagged`; owner subscribes-before-list and clears admission before gap/closure recovery. Previously approved (user, 2026-09-13), salvaged unchanged: main's `IntentStore::watch` trait and `LocalStore` are unchanged since the merge base (classification, class 1). An optional simplification is offered as PROPOSED P-8; the design keeps this contract unless the user adopts it. |
| **D-APP-11** | [ADR-0225] adds the dedicated non-allocation Gateway Identity Slot + `GatewaySvidLifecycle`, Issue/Drop actions and exact-current control. The slot atomically owns a checked desired epoch so a stale Issue cannot re-hold after disable; [ADR-0234] gives gateway-client mTLS alone the private access token and exact receipt-selected peer check. No fake AllocationId. |
| **D-APP-12** | [ADR-0242] makes backend unavailability a typed outcome at the connector (`NoBackend` 503 / connect failure 502) with no wait, demand, revision, acknowledgment or wake; ServiceMapHydrator is unchanged and BPF Hydrated remains independent. The gateway shutdown stage runs before the leg-C worker stage (D5 section (c)). |
| **D-APP-13** | [ADR-0224] keeps BPF as the gateway backend selector; [ADR-0232] adds registered `(SocketCookie, ServiceKey)` intent plus `Selected(BackendId)|NoBackend` receipt and cleanup; [ADR-0243] places socket preparation (mark, loopback bind, cookie, registration) in the dataplane adapter; [ADR-0233] commit-gates immutable BackendId→applied `Backend.alloc` identity publication. |
| **D-APP-14** | [ADR-0226], [ADR-0235], [ADR-0236], [ADR-0237], [ADR-0238], [ADR-0239] and [ADR-0240] activate strict rustls TLS1.3, Hyper HTTP/1.1, exact authority/raw-path routing, proxy-header policy, finite limits, streaming/backpressure and one upstream attempt; [ADR-0234] requires exact-peer mTLS. Existing 404 Route miss, 503 NoBackend and 502 internal/upstream mappings remain exact. |
| **D-APP-15** | Full composition is `AT-PIG-E2E-1`, a recurring built-binary Tier-3 walking skeleton, not an expectation or spike; it hand-installs no map, rule, identity, snapshot or credential outside production inputs. |
| **D-APP-16** | [ADR-0227] keeps operator-mTLS `GET /v1/gateway/status` as a separate redacted projection of application/custody and relevant independent hydration rows; public request status, receipts, secrets and a unified generation never enter it. |

Formal TLA+ is not introduced: this slice adds no new consensus,
cross-node transaction or replication protocol. Atomic snapshot/watch and
certificate-validity ordering are more directly verified through pure
properties and seeded DST.

[ADR-0222]: ../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md
[ADR-0223]: ../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md
[ADR-0224]: ../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md
[ADR-0225]: ../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md
[ADR-0226]: ../../product/architecture/adr-0226-public-listener-tls13-only.md
[ADR-0227]: ../../product/architecture/adr-0227-redacted-operator-gateway-status.md
[ADR-0231]: ../../product/architecture/adr-0231-preserve-last-usable-public-certified-key-replacement.md
[ADR-0242]: ../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md
[ADR-0243]: ../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md
[ADR-0232]: ../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md
[ADR-0233]: ../../product/architecture/adr-0233-commit-gated-backend-identity-publication.md
[ADR-0234]: ../../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md
[ADR-0235]: ../../product/architecture/adr-0235-public-http11-only.md
[ADR-0236]: ../../product/architecture/adr-0236-canonical-public-route-match-key.md
[ADR-0237]: ../../product/architecture/adr-0237-public-proxy-header-policy.md
[ADR-0238]: ../../product/architecture/adr-0238-fixed-finite-public-runtime-limits.md
[ADR-0239]: ../../product/architecture/adr-0239-stream-public-proxy-bodies-with-backpressure.md
[ADR-0240]: ../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md

## Wave: DESIGN / [REF] Application component decomposition

| Component | Exact home / class | Change | Contract shape and declared universe | Assertion mechanism |
|---|---|---|---|---|
| Route/parser/identifier/envelope types | `overdrive-core::public_ingress`, `api::route` / `core` | **EXTEND** | **pure-function**: input -> typed value/error only; codec universe is one Route-set value | per-value PBT, canonical-byte golden and state-delta complement |
| Public-certified-key record + AEAD codec | core record in `overdrive-core`; crypto adapter in `overdrive-host::ca` | **EXTEND** | **bounded-change**: one public-key ciphertext/record domain; Root/intermediate CA records are complement | codec tamper/CSPRNG fault integration + secret-sink trybuild |
| Lag-aware intent subscription | `overdrive-core::traits::IntentStore`, Local/Sim adapters | **EXTEND IN ONE CUT** | **bounded-change**: accepted events or one `Lagged` fact; store contents are read-only | production/Sim adapter-equivalence and seeded loss/relist DST |
| Shared gateway values/read ports | `overdrive-core::{public_ingress,gateway_identity}` / `core` | **EXTEND** | **pure-function/unbounded-preservation**: ServiceKey, SocketCookie and non-secret gateway-identity desired/current facts; no host I/O | value-law PBT + existing-core dependency lint |
| Gateway-owned driven ports | `overdrive-gateway::ports` / `adapter-host` | **CREATE NARROW PORTS** | **bounded-change**: `ServiceFrontendResolve`, `GatewayConnectDataplane`, `GatewayClientMtls` and `GatewayIdentityLifecycleControl` expose only application-required capabilities | Host/Sim equivalence + caller/API-surface scan |
| `PublicRouteSetOwner` | `overdrive-gateway::route_set` / `adapter-host` | **CREATE NEW** | **bounded-change**: fixed Route key, owner snapshot and one reply/event | command state-delta PBT + real-store ordering integration |
| `IntentServiceFrontendResolver` | `overdrive-gateway::frontend` / `adapter-host` | **CREATE NEW ADAPTER** | **unbounded-preservation**: reads WorkloadIntent/VIP; all store/allocator state preserved | before/after adapter-equivalence plus real malformed-prefix probe |
| `SocketCookieReader` | `overdrive-host::socket_cookie` / `adapter-host`; used only inside `EbpfDataplane::open_upstream` (ADR-0243) | **CREATE NEW ADAPTER** | **unbounded-preservation**: reads one borrowed fd; socket and all kernel tables preserved | same-method nonzero/stability probe + real-connector cookie equality (V-GW-5) |
| `ManualCertifiedKeySource` | `overdrive-gateway::manual_certified_key` / `adapter-host` | **CREATE NEW** | **unbounded-preservation**: reads only two configured files; whole artifact tree preserved | filesystem snapshot complement + permission/symlink/torn-read faults |
| `PublicCertifiedKeyCustody` | `overdrive-gateway::certified_key` / `adapter-host` | **CREATE NEW** | **bounded-change**: one protected intent record, one custody status row and one latest availability | store state-delta, generation-race PBT and expiry/clock integration |
| `GatewayApplicationOwner` | `overdrive-gateway::application` / `adapter-host` | **CREATE ACTIVE OWNER** | **bounded-change**: staged candidate, Current and Draining set, one ArcSwap generation with connection/request lease counters, listener state and one status row | seeded lifecycle/DST + retained-generation/cancellation tests |
| Public Runtime + connector | `overdrive-gateway::runtime`, `connector` / `adapter-host` | **CREATE ACTIVE OWNER** | **bounded-change** per accepted connection: its socket, headers/body budget, join-set task and one cookie intent/receipt; fixed cleanup-failure ledger | protocol matrix/PBT + connector/shutdown fault matrix + Tier-3 wire capture |
| `AppState::gateway` composition | control-plane / `adapter-host` | **EXTEND** | **bounded-change**: one Clone-safe composition owns Enabled(identity+control) or canonical Disabled across every hydrate/action/workflow/handler path | disabled legacy-equivalence + enabled boot integration |
| Connect registry + socket preparation + identity association | `overdrive-dataplane::gateway_connect`; BPF hook/maps | **EXTEND EXISTING DATAPLANE OWNER** | **bounded-change**: prepared sockets and registered cookie entries plus immutable BackendId association; unregistered sockets/maps preserved | POD/gold layout, Sim equivalence and real-kernel complement-restored probe |
| Gateway Identity Slot/lifecycle/client mTLS | dataplane mTLS + reconciler/control-plane executor | **CREATE SIBLING/DEDICATED SLOT** | **bounded-change**: one non-allocation slot, desired/retry state and one authenticated fd; allocation holder is complement | pure lifecycle PBT, audit/slot integration, wrong-peer TLS matrix |
| Gateway status persistence | core row/envelopes + store/sim adapters | **EXTEND** | **bounded-change**: exactly two new LWW row families/keys; existing tables preserved | golden codecs + LWW/timestamp state-delta PBT + redaction API tests |
| `serve`/operator composition | control-plane + CLI | **EXTEND** | **bounded-change**: mandatory Enabled/Disabled identity composition plus optional GatewayControl and complete GatewayHandle task/socket tree; existing operator/workload owners preserved | every-owner shutdown integration + disabled complement + built-binary AT |

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
    // Wakeup model (`.claude/rules/reconcilers.md` § "The wakeup model"):
    // host-backed. `actual` is the in-process `GatewayIdentitySlot`, not an
    // observation row, and this reconciler's own `IssueGatewaySvid` executor
    // authors the `IssuedCertificate` rows, so it declares NO interest in
    // that row family (the no-busy-loop rule) and its own `resync_schedule()`
    // is its level-triggered backstop. Explicit wakes from
    // `ensure_current`/`disable_after_drain` evaluations stay.
    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        Some(ResyncSchedule { period: GATEWAY_SVID_RESYNC_PERIOD, scope: ResyncScope::LocalNode })
    }
    fn interests(&self) -> &'static [ObservationRowKind] { &[] }
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
    .unwrap_or_else(|_| unreachable!("NodeId is non-empty and node/ is a canonical target prefix"));
let evaluation = Evaluation {
    reconciler: reconciler.name().clone(),
    target,
};
(AnyReconciler::GatewaySvidLifecycle(reconciler), evaluation)
```

The only production caller is `boot_server` (step G0 of the D5 boot order,
before `Arc::new(runtime)`): it calls `runtime.register(reconciler).await` once
when the gateway is enabled; the lifecycle control submits `evaluation` as
`EvaluationEligibility::Immediate` through the existing broker
(`runtime.broker().submit(evaluation, clock.now(), ..)`) at G2.
`ControlPlaneGatewayIdentityLifecycle` reuses the
same factory-derived name/target for ensure, reissue and disable wakes.

The existing closed dispatches (`AnyState` `M:crates/overdrive-reconcilers/src/lib.rs:114`, `AnyReconciler` `:146`, `AnyReconcilerView` `:478`; the control-plane-private `AnyViewMap` `M:crates/overdrive-control-plane/src/reconciler_runtime.rs:74`) gain exactly these variants:

```rust
AnyReconciler::GatewaySvidLifecycle(GatewaySvidLifecycle)
AnyState::GatewaySvidLifecycle(GatewaySvidLifecycleState)
AnyReconcilerView::GatewaySvidLifecycle(GatewaySvidLifecycleView)
AnyViewMap::GatewaySvidLifecycle(
    BTreeMap<TargetResource, GatewaySvidLifecycleView>,
)
```

Every exhaustive `AnyReconciler` forwarder (`resync_schedule` `:206`, `interests` `:224`, `next_evaluation_at` `:319`, and the others) gains the matching arm: `name`,
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

`GATEWAY_SVID_RESYNC_PERIOD` is a DELIVER tuning constant, bounded above by
the SVID reissue margin so a lost wake cannot let a held gateway SVID reach
expiry unreissued; `next_evaluation_at` continues to schedule the expiry-driven
reissue. P-5 is resolved by the rule (not a user decision).

Hydration requires the target equal the configured `node/<node-id>`, reads
desired/current through the single `GatewayIdentityView` read port, and derives
`ever_issued` only from the existing issued-certificate audit rows for the
gateway SPIFFE subject.

The existing public `HydrationContext<'a>` (`M:crates/overdrive-core/src/reconcilers/hydration.rs:57-84`, which already carries `guest_attachments` at `:76`) gains exactly one mandatory field, placed after `guest_attachments`:

```rust
pub gateway_identity: &'a dyn GatewayIdentityView,
```

`GatewayIdentityView` is one narrow read-port (non-secret facts only) with two
read methods: `desired()` (epoch and optional subject) and `current()`
(`Option<GatewayIdentityFacts>`); its contract lives in
`overdrive-core::gateway_identity`.

The sole `build_hydration_context` (`M:crates/overdrive-control-plane/src/reconciler_runtime.rs:1720-1739`) supplies them from `AppState` at every
existing construction site (including test-only constructions such as `M:crates/overdrive-control-plane/src/vm_reclamation_boot.rs:385`). Enabled `serve` supplies the one
`GatewayIdentitySlot`; disabled `serve` supplies a control-plane-private
canonical empty adapter whose desired value is epoch one with `spiffe_id=None`
and whose current value is `None`. Only the gateway lifecycle reads the
identity field. This preserves a total HydrationContext without optional
per-call parameters or a second gateway-only runtime. The port has one `Sim*`
implementation (`SimGatewayIdentityActionTarget`), so the hydration boundary
stays DST-injectable (`.claude/rules/reconcilers.md` § "The hydration
boundary"). `build_hydration_context` borrows from the AppState-owned Arc, never
a temporary; the returned HydrationContext cannot outlive its `&AppState`
input.

Dependency direction is `control-plane -> {gateway,dataplane,reconcilers} ->
core`; dataplane depends on gateway only for the `GatewayConnectDataplane`,
`GatewayClientMtls` and `GatewayClientMtlsFactory` ports and on host for the
`SocketCookieReader` it uses inside `open_upstream`; gateway depends on
host/core and never dataplane, control-plane, reconcilers, worker, store-local
or aya.

## Wave: DESIGN / [REF] Exact driving and driven ports

### Driving surfaces

| Surface | Exact first-slice contract |
|---|---|
| `overdrive serve` | Scalar `--gateway-address`, `--gateway-certified-key-id`, `--gateway-certificate-chain` and `--gateway-private-key` arguments are all absent or all present. The complete set activates identity, the connect probe, `GatewayApplicationOwner` and dynamic fixed TCP/443 after gates. |
| `overdrive deploy <SERVICE_SPEC>` | Existing workload path byte/behavior unchanged. |
| `overdrive deploy <ROUTE_SPEC>` | `deploy_resource(args, stream_workloads)` (new; `main`'s `Command::Deploy` arm calls it instead of choosing `deploy`/`deploy_streaming` itself) parses once; `[route]` + `[route.target]` selects one-shot `POST /v1/routes` and `RouteDeployOutput`; `--detach` has no additional effect and no NDJSON opens. A workload document is delegated unchanged to the existing `commands::deploy::{deploy, deploy_streaming}` (`M:crates/overdrive-cli/src/commands/deploy.rs:180, 500`), whose output functions/types remain unchanged. |
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

Exact checked-in `examples/public-ingress-gateway/route.toml.template`:

```toml
[route]
id = "public-api"
hostname = "__PUBLIC_HOSTNAME__"
path = "/"
path_match = "segment_prefix"
certified_key = "api-origin"

[route.target]
service = "api"
port = 8080
protocol = "tcp"
```

`__PUBLIC_HOSTNAME__` occurs exactly once. The checked-in `run-example.sh`
materializes only this template to `<absolute-scratch-dir>/specs/route.toml`
using its `--public-hostname` argv value; the Tier-3 and E15 runners invoke the
example and never construct an inline Route document. The rendered value is
validated by the same production `PublicHostname::new` path as every other
Route deploy. Missing/multiple placeholders or an invalid rendered TOML/Route
fail the example before any Route is accepted.

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
    pub service: String,
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
    service: WorkloadId,
    port: NonZeroU16,
    protocol: Proto,
}
impl ServiceListenerReference {
    pub fn new(
        service: WorkloadId,
        port: NonZeroU16,
        protocol: Proto,
    ) -> Result<Self, RouteValidationError>; // accepts only Proto::Tcp
    pub fn service(&self) -> &WorkloadId;
    pub const fn port(&self) -> NonZeroU16;
    pub const fn protocol(&self) -> Proto;
}
pub enum RouteValidationError {
    RouteId(IdParseError), CertifiedKeyId(IdParseError),
    Service(IdParseError), Hostname(PublicHostnameError),
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

// overdrive-cli::commands::serve (M:crates/overdrive-cli/src/commands/serve.rs:38-59);
// existing fields retain their exact types.
pub struct ServeArgs {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub vsock_ports: NodeVsockPortConfig, // existing on M (ADR-0218)
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
// Additive overdrive-control-plane::ServerConfig field
// (M:crates/overdrive-control-plane/src/lib.rs:820-923); ServerConfig stays
// configuration only (ADR-0219 NO-OVERRIDE-SEAMS) and `ServerConfig::new()`
// (no arguments, `:940`) initializes the new field to `None`.
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
`commands::serve::run_with_adapters` (`M:crates/overdrive-cli/src/commands/serve.rs:153`),
before the `CgroupFs` probe (`:166`), any file read, bundle build, store
open or listener bind. Its closed truth table is:

| Supplied fields | Result |
|---|---|
| all four `None` | `Ok(None)`; `ServerConfig.gateway = None` and the gateway remains disabled |
| all four `Some` | `Ok(Some(GatewayConfig::new(address, ManualCertifiedKeyConfig::new(id, chain, key))))`; `GatewayConfig::new` fixes the public bind port to 443 |
| every other combination | `Err(GatewayConfigError::PartialEnablement { missing })`; `missing` is emitted in `Address, CertifiedKeyId, CertificateChainPath, PrivateKeyPath` order and no startup effect occurs |

`run_with_adapters` maps only that error to
`CliError::GatewayConfiguration { source }`, then moves the successful value
unchanged into the additive `ServerConfig.gateway: Option<GatewayConfig>`
field through its existing struct update (`serve.rs:179-185`, rest-pattern
`..ServerConfig::new()`), which is the only enabling site. `main`
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

Both parser/wire inputs require exactly `target.service`; `target.workload` is
an unknown field and there is no serde/TOML alias. `Route::from_submit` parses
that string through the existing `WorkloadId` grammar and reports
`RouteValidationError::Service` at public field `route.target.service`.
`ServiceListenerReference::service()` deliberately returns `&WorkloadId`:
Service intent is keyed by the existing logical-workload identifier, whereas
allocator-derived `ServiceId` names the resolved `(VIP, port, protocol)`
Dataplane projection only after `ServiceFrontendResolve` succeeds. These types
must not be renamed, converted or conflated.

Every field on `Route`, `GatewayConfig` and `ManualCertifiedKeyConfig` has only
the accessor shown above; their `Debug` implementations redact manual paths.
`RouteGeneration::from_route_fields` SHA-256 hashes
`b"overdrive/gateway/route-generation/v1\0"`, then each string-like field as
`u32::to_be_bytes(len) || bytes` in this order: Route ID, hostname, path-match
tag (`0` exact / `1` segment-prefix), raw path, the target Service's canonical
`WorkloadId`, then the two-byte big-endian port, one-byte `Proto::as_u8`, and
length-framed certified-key ID.
No archive layout, map iteration or platform endianness enters the digest.
`PublicHostname::new` and
`PublicPath::new` plus their complete errors/accessors follow the validation
rules in D-APP-4; neither offers an unchecked public constructor.

### Driven ports/adapters

| Port/effect | Exact contract/adapters |
|---|---|
| `IntentStore` | Public Route Set and protected key envelopes; lag-aware empty-prefix subscription. LocalStore production, existing LocalStore/Sim lanes. |
| `ServiceFrontendResolve` | `probe` + `resolve(&ServiceListenerReference) -> ServiceFrontend`; gateway adapter reads WorkloadIntent + `ServiceVipView`, and control-plane privately adapts the existing `Arc<Mutex<PersistentServiceVipAllocator>>`; Sim adapter returns scripted typed outcomes. |
| Gateway Connect Dataplane | Prepare+register `(SocketCookie,ServiceKey)` (`open_upstream`), take/cleanup, lifecycle-only `cleanup_all_gateway_intents` + live counts, Selected/NoBackend receipt and immutable BackendId identity lookup; EbpfDataplane/Sim. Drop failures enter the runtime cleanup ledger. |
| Gateway identity | Dedicated slot facts + desired/current hydration, `GatewaySvidLifecycle`, Issue/Drop actions and lifecycle control; no allocation holder/read. |
| Gateway-client mTLS | Connected fd + exact receipt-selected SpiffeId -> authenticated `GatewayUpstream`; Host/Sim implementations; no InterceptedConnection/kTLS pump. |
| `Kek` | Existing mandatory provider; `overdrive-host::ca::PublicCertifiedKeyAeadCodec` uses fixed `overdrive-public-certified-key` KEK and a distinct gateway HKDF/AAD domain over the existing private AEAD engine. |
| `ObservationStore` | Exact `public_certified_key_status_row(id)` and `gateway_application_status_row(node)` point reads plus two typed LWW writes; tables/keys/codecs are pinned in the Application persistence/status `[REF]` sections below. Issuance audit and Service hydration rows remain unchanged. |
| Public TLS/HTTP | Active `tokio-rustls` 0.26.4 + rustls 0.23.39 + hyper 1.9.0 + hyper-util 0.1.20; fixed finite GatewayLimits policy with no operator input. |

### Exact Service-frontend and Dataplane ports

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
    ServiceAbsent { service: WorkloadId },
    ServiceIntent(IntentStoreError),
    ServiceIntentDecode,
    TargetNotService { service: WorkloadId },
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

// ServiceKey is the only frontend-identity value the gateway passes to the
// dataplane. There is no demand revision, snapshot, acknowledgment or wake type
// (D2, ADR-0242).

// overdrive-gateway::ports
#[async_trait::async_trait]
pub trait GatewayConnectDataplane: Send + Sync {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError>;
    /// Marks (`MTLS_TPROXY_MARK`), loopback-binds, reads the cookie of and
    /// registers one new upstream socket (ADR-0243). The only constructor of a
    /// registered intent; there is no public `register`.
    fn open_upstream(&self, key: ServiceKey)
        -> Result<PreparedUpstream, GatewayConnectDataplaneError>;
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
/// A socket prepared by the dataplane adapter. Not Clone.
pub struct PreparedUpstream { /* private: tokio::net::TcpSocket + intent */ }
impl PreparedUpstream {
    pub fn intent(&self) -> &GatewayConnectIntent;
    pub fn into_parts(self) -> (tokio::net::TcpSocket, GatewayConnectIntent);
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
    SocketCreate(std::io::ErrorKind),
    SocketOption(std::io::ErrorKind),
    SocketBind(std::io::ErrorKind),
    CookieRead(std::io::ErrorKind),
    IntentRegistryFull,
    IntentAlreadyRegistered,
    IntentMissing,
    ReceiptMissing,
    ReceiptMismatch,
    BackendIdentityMissing { backend_id: BackendId },
    Kernel,
    Cleanup,
}

// `Action::DataplaneUpdateService` (`M:crates/overdrive-core/src/reconcilers/mod.rs:510`)
// is NOT extended. The hydrator, the action shim's dataplane arm and
// `Dataplane::update_service` keep their current signatures (D1/D2).

```

`action_shim::dataplane_update_service::dispatch`, `DispatchOutcome` and
`ServiceHydrationDispatchError` keep their current definitions on `main`; the
gateway adds no outcome, error variant or argument there.

```rust
// overdrive-control-plane::action_shim  (M:crates/overdrive-control-plane/src/action_shim/mod.rs:1015 `dispatch`,
// :1072 `dispatch_with_network_provisioner`, :1852 `dispatch_single`; every parameter of `main`'s
// current signature is retained in its current position)
pub async fn dispatch(
    /* ..main's parameters through */ identity: &IdentityMgr,
    gateway_identity: &GatewayIdentityActionComposition, // NEW
    /* ..main's remaining parameters.. */
) -> Result<(), ShimError>;
// dispatch_with_network_provisioner gains the same parameter at the same position.
// dispatch_with_workflow_intent(actions, state: &AppState, tick) keeps its
// signature and reads `state.gateway.identity_actions()`.
```

The private `dispatch_single` receives the same `gateway_identity` immediately
after `identity`; every public dispatcher passes it unchanged. No overload
preserves the old signatures.

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
empty selection (and, only if P-9 is adopted, a selected `BACKEND_MAP` entry whose `healthy` byte is 0).
Missing intent follows the existing LOCAL_BACKEND_MAP branch;
any registered-arm decode/map-write failure denies without falling through.
`take_receipt` accepts only those two canonical value shapes, binds the typed
result back to the caller's exact cookie/ServiceKey, and removes the receipt;
the RAII cleanup removes both keys on every later result.

Production `GatewayConnectDataplane::probe` exercises this same port before a
public bind: it first sweeps any intent or receipt residue, then calls
`open_upstream` for the exact sentinel ServiceKey `192.0.2.1:9/Tcp` in the real
`serve` cgroup and calls `connect(2)`. Because the reserved documentation VIP
cannot have a Service-map entry, success means the connect is denied and a
matching canonical `NoBackend` receipt is consumed; missing/malformed receipt,
an allowed connect or nonzero residual intent/receipt fails the probe. This is
a real existing-component drive, not a hand-installed test seam or scratch
spike. It runs only when `ServerConfig.gateway` is `Some` (step G3).
The caller supplies the application-gate absolute deadline and injected Clock;
the sentinel connect races the remaining duration and maps expiry only to
`GatewayConnectDataplaneError::ProbeTimeout`.

`cleanup_all_gateway_intents` keeps its selected public name but its lifecycle
contract deletes both `GATEWAY_CONNECT_INTENT_MAP` and
`GATEWAY_SELECTION_RECEIPT_MAP`, returning exact removed counts. The two live
count methods enumerate their distinct maps after cleanup; failure to enumerate
is typed and never treated as zero.

`IntentServiceFrontendResolver::new(intent, vip_view)` is crate-private and the
only adapter constructor; `UnboundGatewayBuilder::new` is its only caller. The
resolver probe and `resolve` calls from `GatewayApplicationOwner` are the only
production calls through the public port. It reads the Service-keyed
`WorkloadIntent` using `reference.service()`: missing intent is
`ServiceAbsent`, an intent-store read failure is `ServiceIntent`, decode failure
is `ServiceIntentDecode`, and any resolved kind other than
`WorkloadKind::Service` is `TargetNotService { service }`. No Job or Schedule
kind is a valid Route target; the Service's driver axis remains orthogonal.
The resolver is a read of Service intent and the VIP view only; the gateway
never derives or stores a `ServiceId` and publishes nothing to the hydrator.

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
    updated_backend_pods: Vec<(BackendId, BackendEntryPod)>, // prior values
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
   identity conflict (the same id for a different `Backend.alloc`, P-2) or
   differing reverse-NAT VIP before mutation. A `BACKEND_MAP` entry whose
   address/port/proto equal the memo hit and whose `weight`/`healthy` differ is
   the ordinary in-place update `update_service` already performs on `main`
   (the `healthy` byte flips 0 → 1 at readiness, D4) and is not a conflict; its
   prior value is saved in `ServiceApplyUndo` so rollback restores it. An
   identical existing entry is reused and never rewritten. Allocate monotonic fresh IDs, retain their planned identity
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
publication. The action shim writes the Completed hydration row only after
that point; a pre-commit or rollback error writes the existing Failed
hydration row. This is userspace transactional publication around existing
kernel primitives, not a claim that four independent BPF maps update
atomically (assumption A-K1).

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
| `GatewayApplicationStatusRowEnvelope::V1` | table `gateway_application_status`, key canonical NodeId bytes | listener fact; staged/current/draining application generations; unavailable cause; non-secret gateway identity/connect-probe facts | Backend candidates, BackendId/receipt/peer, ServiceBackendRow/BPF generation, credential bytes |

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
`GatewayAdapters::host(kek)` constructs one codec over the boot-probed
`Kek`; `boot_server` G2 passes it to `UnboundGatewayBuilder::new`. No adapter receives a generic
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
}
impl GatewayAppliedGeneration {
    pub(crate) const fn from_promotion(
        application_generation: GatewayApplicationGenerationId,
    ) -> Self;
    pub const fn application_generation(&self) -> GatewayApplicationGenerationId;
}

#[derive(Clone)]
pub struct GatewayAppStateComposition {
    identity_view: Arc<dyn GatewayIdentityView>,
    identity_actions: GatewayIdentityActionComposition,
    control: Option<GatewayControl>,
}
impl GatewayAppStateComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(
        identity_view: Arc<dyn GatewayIdentityView>,
        identity_actions: GatewayIdentityActionComposition,
        control: GatewayControl,
    ) -> Self;
    pub fn identity_view(&self) -> &dyn GatewayIdentityView;
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
impl GatewayIdentityView for DisabledGatewayIdentityRead {
    fn desired(&self) -> GatewayIdentityDesired;
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
        clock: Arc<dyn Clock>,
    ) -> Result<Self, GatewayBootError>;
    pub fn into_builder(self) -> (GatewayBuilder, GatewayUpstreamSeal);
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
pub struct GatewayBuilder { /* sealed typestate */ }
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
        // every existing parameter unchanged (on `main` through `guest_pool`,
        // `M:lib.rs:4085-4115`), then the new last parameter:
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
    dataplane: Arc<dyn GatewayConnectDataplane>,
    cleanup_ledger: Arc<GatewayCleanupLedger>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    clock: Arc<dyn Clock>,
    application_shutdown: CancellationToken,
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
    /// Time the stage consumed on the injected clock; `ServerHandle` subtracts it
    /// from the shared drain deadline before axum's graceful shutdown (D5-c).
    pub elapsed: Duration,
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
}
pub enum GatewayApplicationCommandError {
    OwnerUnavailable,
    Timeout { phase: GatewayApplicationCommandPhase },
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
    Listener, Connections, Application, ManualCertifiedKeySource,
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

`boot_server` (`M:crates/overdrive-control-plane/src/lib.rs:3625`; `run_server`
at `:3116-3120` delegates to it) is the sole caller of
`UnboundGatewayBuilder::{new,into_builder}`, constructs one
`GatewayLimits::first_slice` value, uses it for the `GatewayClientMtlsFactory`
product and one `GatewayRuntimeDependencies`, calls `GatewayBuilder::start`
(steps G2-G4 of the D5 boot table), puts
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
The owner checks the command deadline before beginning; every awaited
operation uses that same deadline, and all local state changes after it are
non-awaiting. A timed-out queued command therefore cannot execute later. For
`CloseForShutdown`, it first
cancels/drops the in-flight application-gate future, discards `staged` (a
staged candidate holds no external state: no Service-map demand exists), and
closes all Current/Draining retentions and clears admission. It returns all
retentions. The three
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

`boot_server` constructs the Enabled/Disabled identity composition (G1)
before `AppState` and registers the gateway reconciler when enabled (G0). The
gateway task tree starts at G4, after the convergence loop, interest router,
emit-drain and shared-network supervisor are running and `server_task` is
spawned, so every first identity evaluation and hydration sees the final ports
(D5 boot table, `M:crates/overdrive-control-plane/src/lib.rs:4399-4616`). The
gateway tasks may wait for convergence, but cannot bind TCP/443 until their
gates complete. The `StartedGateway::into_parts` return is the only transfer
into AppState/ServerHandle; disabled configuration stores `gateway=None`.
The existing integration-only `abort_for_test` consumes the same handle,
invokes gateway shutdown with `Duration::ZERO`, and returns its typed report in
`AbruptServerResidue.gateway`; it cannot discard the new field or inject a
snapshot/identity/map.

`UnboundGatewayBuilder::into_builder` returns the `GatewayBuilder` and the
builder's one `GatewayUpstreamSeal`. Composition obtains
`slot.client_access(clock.clone())` and passes it, the seal and
`limits.clone()` as `GatewayClientMtlsDeps` to
`BootAdapters.gateway.client_mtls.client_mtls(deps)` (the
`HostGatewayClientMtlsFactory` in production). It then moves the
same limits value into `GatewayRuntimeDependencies::new(limits, dataplane,
client_mtls, identity_lifecycle)` where `dataplane` is
`ComposedDataplane.gateway_connect`; no second policy constructor call occurs.

`GatewayApplicationGenerationId::from_inputs` is called only by
`GatewayApplicationOwner` after resolving all three inputs; status consumes
`as_bytes`. `GatewayAppliedGeneration` is constructed only at
promotion and consumed by the admission snapshot plus application-status row.
The private candidate owns the exact Route, resolved Service Frontend and
opaque Public Certified-Key snapshot; the private admission snapshot adds the
Applied Generation and one Arc-owned `GatewayGenerationRetention`.
Both are created only in the owner turn. This is the single atomic binding
required by the Domain model.

`GatewayApplicationCandidate::new` requires the Route's certified-key ID and
hostname equal the supplied snapshot before computing its generation. On
promotion the owner constructs one
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
saturation is typed/non-clean even if later cleanup succeeds. Application
close/retire command failure goes to `application_errors`; gateway identity
drop failure goes to `identity_error`. Neither is a Dataplane-ledger entry.
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
owner alone waits and then retires the generation; no per-lease event queue exists.

`GatewayAppliedGeneration::from_promotion` is called only after all gate inputs
(Route, key, frontend, identity Current, connect probe) hold and only inside
`GatewayApplicationOwner`; external crates can read but cannot forge Applied
authority.

The Application-generation hash is exactly SHA-256 over
`b"overdrive/gateway/application-generation/v1\0" ||
route_generation.as_bytes() || frontend.vip_v4().octets() ||
frontend.port().get().to_be_bytes() || [frontend.proto().as_u8()] ||
certified_key_generation.as_bytes()`. IPv4 octets are network order and the
port's two bytes are big-endian. It contains no
BPF generation, BackendId, receipt or gateway-identity fact.

There is no Service-map demand composition: the hydrator, the action shim and
`Action::DataplaneUpdateService` are unchanged by the gateway (D1, ADR-0242).
`AppState` carries only the identity reads, the identity action target and the
control handle below.

`GatewayAppStateComposition::disabled` creates one
`DisabledGatewayIdentityRead::canonical()` Arc used for both identity trait
fields, `GatewayIdentityActionComposition::disabled()`, and `control=None`.
The disabled desired value is exactly epoch one/`spiffe_id=None`; current is
always None. Production enabled composition stores
the same `Arc<GatewayIdentitySlot>` behind both read traits, the Enabled action
target, and the live control handle. Because the outer composition clones only
Arcs/handles, existing `AppState: Clone` remains valid and every HydrationContext
borrow points into an AppState-owned Arc.

The broad fixture `AppState::new` keeps its exact signature and delegates with
`GatewayAppStateComposition::disabled()`. The full
`new_with_workflow_engine` appends the mandatory `gateway` parameter after
`frontend_addr_allocator` (`M:crates/overdrive-control-plane/src/lib.rs:4085`
call site); production supplies Enabled when `config.gateway` is set, while every existing
non-gateway caller and direct `AppState { .. }` literal adds exactly
`gateway: GatewayAppStateComposition::disabled()`. Only gateway-focused Sim/
integration composition may pass Enabled. `build_hydration_context`, both
workflow-intent dispatch forms, the workflow emit loop and Route/status
handlers read exclusively through `state.gateway` accessors; no parallel
AppState identity/control field exists.

One active `GatewayApplicationOwner` holds at most one staged candidate, one
ArcSwap Current generation and N draining generations with retained connection
counts. The application ID hashes only Route Generation, exact frontend and
Certified Key Generation; Applied wraps it only at the atomic Current swap.
BPF generation, BackendId, receipt and SPIFFE material never
enter the object.

The owner stage/gate/promote order is load-bearing. It publishes exact frontend
as Staged and computes one absolute
`gate_deadline = clock.now() + limits.application_gate_deadline()`. It
revalidates Route/key/frontend, ensures current gateway identity
against the same deadline, drives the same connector through one sentinel and
one actual-frontend probe with that deadline (an actual-frontend `NoBackend`
is bind-valid, ADR-0242), rechecks identity remains
Current, swaps Current, moves the
prior Current to Draining, then binds TCP/443 if needed and enables accepts. A failed initial
bind accepts nothing and removes the just-promoted Current through the
zero-connection Draining phase before publishing a typed failure.
On an already-bound replacement, the
existing listener remains owned while the Current pointer swaps.
Any application-gate timeout removes only the staged candidate. With no
prior Current the listener remains unbound; on replacement the prior Current
and listener remain unchanged. The same absolute deadline is never restarted
by a watch wake or retry.
Withdraw/unresolved/key or
identity loss stops accepts/unbinds before clearing Current; a Draining
generation remains until the last retained connection closes. A late gate result
cannot restore a superseded generation. Intent/custody/identity watch recovery is
subscribe-before-list and fail-closed.

No Service-map state is staged, acknowledged or retired by the owner. The
frontend's `ServiceKey` is
`ServiceId::derive(vip, port, proto, "service-map")`, computed from
`ServiceFrontend` and never stored in Route; the ordinary hydrator programs the
backend independently (D1). Backend availability at request time is the
connector's typed outcome (ADR-0242).

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
}
pub enum GatewayApplicationUnavailableCause {
    RouteAbsent,
    RouteUnreadable,
    CertifiedKeyAbsent,
    CertifiedKeyUnusable,
    FrontendUnresolved,
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
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GatewayListenerStatusBody { Unbound, Bound { address: String } }
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatewayApplicationUnavailableCauseBody {
    RouteAbsent, RouteUnreadable, CertifiedKeyAbsent, CertifiedKeyUnusable,
    FrontendUnresolved, GatewayIdentityUnusable,
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
does not roll back admission or listener state or become desired
truth; the owner retains the newest row value and retries it on its next
input/wake. The operator may therefore see the last accepted LWW value, never
a pre-transition promise.

`GET /v1/gateway/status` exposes enabled/configured bind/listener, active
application row, custody, non-secret Gateway Identity Current, connect-path
state. It contains no
raw dataplane failure string, receipt, Service-map hydration state or unified
BPF/gateway generation.
`GatewayConnectPathStatus::Ready` means the registered hook/receipt contract is
trusted; it includes `GatewayConnectProbeOutcome::ReadyNoBackend` and never
claims a selectable backend. Backend absence is observable on the request as
503 (ADR-0242) and, for the Service itself, through the existing
`ServiceBackendRow`/service status surfaces, not through gateway status.
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
| Application gate | 30 s | Reuses the accepted no-progress recovery window to bound gateway identity Current and the pre-bind sentinel/actual-frontend connect probe as one absolute attempt. Lower false-times normal convergence/issuance; higher retains a stale staged generation longer. |
| Public TLS / request head / upstream connect / response head | 5 s each | TLS reuses the accepted `MtlsLimits::handshake_deadline`; the other pre-header phases share that dead-peer/normal-scheduling tolerance. Lower false-times slow legitimate peers; higher amplifies slowloris/dead-backend occupancy. |
| Body no-progress / keep-alive idle | 30 s / 30 s | Reuses the accepted no-progress recovery window and applies it only when bytes are pending or a keep-alive is idle. Lower harms slow streams; higher retains dead sockets/generations. |
| Whole request | 120 s | Matches the existing bounded CLI/control-plane long-request envelope while remaining greater than every phase deadline. Lower truncates legitimate streamed responses; higher delays reclamation. |

The BPF crate mirrors `GATEWAY_MAX_INFLIGHT_CONNECTS = 128` because it cannot
link the std host crate; a host ABI test asserts both BPF map `max_entries`
values equal `GatewayLimits::first_slice().max_inflight_connects()`. No CLI,
environment, config, status or operator knob exposes these values.

`GatewayLimits::first_slice` is the sole constructor. `GatewayBuilder::start`
receives the one value constructed by `boot_server`; the public runtime and
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
/// The one gateway-identity hydration read-port (non-secret facts only).
pub trait GatewayIdentityView: Send + Sync {
    fn desired(&self) -> GatewayIdentityDesired;
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
    /// `open_upstream` failed (SocketCreate/SocketOption/SocketBind/CookieRead/
    /// IntentRegistryFull/IntentAlreadyRegistered); ADR-0243.
    Prepare(GatewayConnectDataplaneError),
    ConnectTimeout,
    Connect,
    Receipt(GatewayConnectDataplaneError),
    ReceiptMismatch,
    NoBackend,
    SelectedIdentity(GatewayConnectDataplaneError),
    Mtls(GatewayClientMtlsError),
}

// overdrive-host::socket_cookie -- dataplane-internal helper (ADR-0243); not
// re-exported to overdrive-gateway, not a BootAdapters field.
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

`GatewayIdentityView` intentionally has no subscription method: its
only live callers are `GatewaySvidLifecycle::hydrate_actual` and the redacted
status projection. The active `GatewayApplicationOwner` is the sole caller of
`GatewayIdentityLifecycleControl::{ensure_current,subscribe,
disable_after_drain}`; the owner needs those commands and wake stream, whereas
the read-only reconciler/status port does not. `GatewayUpstreamConnector::new`
is called once by `GatewayBuilder::start`; the HTTP request path is the sole
caller of `connect`. `SocketCookieReader` is constructed and held only inside
`EbpfDataplane` (and has no Sim counterpart: `SimGatewayConnectDataplane`
mints deterministic nonzero cookies); `read` is called once per prepared
upstream socket by `open_upstream` and `probe` runs inside
`GatewayConnectDataplane::probe`. The gateway crate neither names nor receives
the reader.

`GatewayUpstreamConnector::connect` calls `open_upstream(ServiceKey::from_frontend(frontend))`
(one marked, loopback-bound IPv4 socket with its nonzero cookie registered by
the dataplane adapter), takes the socket out with `into_parts`, and
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

The real `GatewayUpstreamConnector` is also the sole producer of one
structured, non-secret receipt occurrence. It emits at `DEBUG` with tracing
metadata name and explicit field `event` both equal to
`gateway.upstream.receipt.validated`, target
`overdrive::gateway_connector`, and message
`"validated gateway upstream BPF selection receipt"`. The existing tracing
subscriber/filter is reused; no port, status row, ring buffer or observer task
is added.

| Structured field | `Selected` occurrence | `NoBackend` occurrence | Exact type/rule |
|---|---|---|---|
| `event` | `gateway.upstream.receipt.validated` | same | Static string, always present so compact text output retains the metadata name semantically even with target rendering disabled |
| `operation` | `probe` or `request` | `probe` or `request` | Static string supplied by the connector's existing `probe`/`connect` caller; no generated operation ID |
| `socket_cookie` | Receipt/intent cookie | Receipt/intent cookie | Nonzero `u64` from `SocketCookie::get().get()`, recorded as decimal |
| `service_key_vip` | Receipt/intent Service VIP | Receipt/intent Service VIP | Canonical `ServiceVip` Display string; IPv4 in this slice |
| `service_key_port` | Receipt/intent port | Receipt/intent port | Nonzero `u16` numeric value |
| `service_key_protocol` | `tcp` | `tcp` | Canonical `Proto::as_str()` static string |
| `receipt_outcome` | `selected` | `no_backend` | Closed static string |
| `backend_id` | Selected BackendId | omitted | `u32` from `BackendId::get()`; omission is structural, never zero/null/empty sentinel |
| `expected_peer_spiffe_id` | Resolved applied workload identity | omitted | Canonical `SpiffeId` Display string; omission is structural, never zero/null/empty sentinel |

The two branch call sites have this exact shape; `operation: &'static str` is
the private caller label and is exactly `"probe"` or `"request"`:

```rust
// Selected, after selected_backend_identity succeeds and before mTLS.
tracing::debug!(
    name: "gateway.upstream.receipt.validated",
    target: "overdrive::gateway_connector",
    event = "gateway.upstream.receipt.validated",
    operation = operation,
    socket_cookie = intent.socket_cookie.get().get(),
    service_key_vip = %intent.service_key.vip(),
    service_key_port = intent.service_key.port().get(),
    service_key_protocol = intent.service_key.protocol().as_str(),
    receipt_outcome = "selected",
    backend_id = backend_id.get(),
    expected_peer_spiffe_id = %expected_peer,
    "validated gateway upstream BPF selection receipt",
);

// NoBackend, after receipt validation and before result mapping.
tracing::debug!(
    name: "gateway.upstream.receipt.validated",
    target: "overdrive::gateway_connector",
    event = "gateway.upstream.receipt.validated",
    operation = operation,
    socket_cookie = intent.socket_cookie.get().get(),
    service_key_vip = %intent.service_key.vip(),
    service_key_port = intent.service_key.port().get(),
    service_key_protocol = intent.service_key.protocol().as_str(),
    receipt_outcome = "no_backend",
    "validated gateway upstream BPF selection receipt",
);
```

Emission order is load-bearing. `take_receipt(intent)` must first remove the
receipt and the connector must validate that its cookie and ServiceKey equal
the registered intent. A missing, malformed, duplicate, cookie-mismatched or
ServiceKey-mismatched receipt emits no occurrence. For `NoBackend`, the
connector emits immediately after that validation, performs no identity lookup
or mTLS, then returns its existing probe/request result. For `Selected`, it
must successfully resolve `backend_id` through
`selected_backend_identity`; only then does it emit, immediately before
`GatewayClientMtls::authenticate`. Identity absence/error emits no occurrence.
The Selected occurrence remains valid if the subsequent mTLS handshake fails:
it proves only validated BPF selection and the expected peer passed to mTLS,
not peer authentication or request success.

The connector's existing RAII cleanup reservation remains live across the
occurrence and executes afterward on every Selected/NoBackend return path,
including mTLS failure. Cleanup success/failure cannot retract or rewrite the
already-emitted occurrence, and emission never waits for, retries or delays
cleanup. Exactly one occurrence is emitted per validated receipt consumed by
one connector operation; neither the BPF/Dataplane adapter nor a Sim adapter
emits a duplicate.

All fields are process-local correlation or non-secret routing/identity facts.
The occurrence contains no certificate/key bytes, Gateway SVID material,
public-certified-key path, request headers/body, client address, raw error
string or credential-derived fingerprint. It is not persisted to IntentStore/
ObservationStore, exposed by status/API, read by production logic or used to
gate lifecycle. It is append-only observability evidence of an occurrence, not
a second receipt, source of truth, current-state projection or audit ledger.

`GatewayApplicationOwner` calls crate-private `probe(frontend, deadline)` after
gateway identity currentness but before promotion/bind (the sentinel probe
runs once earlier, inside `GatewayConnectDataplane::probe`, ADR-0230). Probe and request `connect` share one private socket/register/
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
fixed limit. Identity wait, connector probe and request connect
therefore have one owner-supplied absolute deadline and one injected Clock;
none discovers a Tokio timer or starts a fresh whole-operation budget.

The Dataplane-local holder and client constructor are:

```rust
// overdrive-dataplane::mtls::gateway_identity  (file: crates/overdrive-dataplane/src/mtls/gateway_identity.rs —
// slot, action/lifecycle handles and GatewayClientIdentityAccess)
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

// overdrive-dataplane::mtls::gateway_client  (file: crates/overdrive-dataplane/src/mtls/gateway_client.rs —
// the exact-peer client; the registry artifact of OUT-PIG-GATEWAY-CLIENT-EXACT-PEER)
pub struct HostGatewayClientMtls { /* slot, Clock, limits; private rustls state */ }
impl HostGatewayClientMtls {
    pub fn new(
        identity: GatewayClientIdentityAccess,
        seal: GatewayUpstreamSeal,
        limits: GatewayLimits,
    ) -> Self;
}
pub struct HostGatewayClientMtlsFactory; // implements GatewayClientMtlsFactory by calling `new`
```

File ownership (reconciles the two earlier names): `mtls/gateway_identity.rs`
owns the slot, handles and `GatewayClientIdentityAccess` (including its
crate-private `with_current`); `mtls/gateway_client.rs` owns
`HostGatewayClientMtls` and `HostGatewayClientMtlsFactory` and is the only
caller of `with_current`. The exact-peer outcome artifact is therefore
`mtls/gateway_client.rs`.

`GatewayIdentitySlot` implements the `GatewayIdentityView` read trait above; its `subscribe` is not
exposed through `GatewayIdentityView`. Serve composition obtains one
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
`begin_disable`, which synchronously empties the slot, and returns the epoch
it cleared; no evaluation is submitted and there is no race or wait
(`Timeout { DisableAfterDrain }` is not reachable from a synchronous clear).
`current`/`subscribe`
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
plus `GatewayIdentityView` over one
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
[ADR-0225] and [ADR-0234] supersede any unguarded Application shorthand while
D-DOM-1…13's semantic owner/invariants remain unchanged.

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

Composition (`boot_server`, D5 steps G1-G4) calls `UnboundGatewayBuilder::into_builder` to
obtain the builder and the one non-Clone `GatewayUpstreamSeal`, builds the
client through `GatewayClientMtlsFactory`, then calls
`GatewayBuilder::start` with `ComposedDataplane.gateway_connect`,
gateway-client mTLS and identity lifecycle control. `AppState` always holds the
`GatewayAppStateComposition` (identity reads into HydrationContext, the identity
action composition through every action/workflow-emission dispatch wrapper);
disabled uses the canonical disabled identity read plus no control handle.
`StartedGateway::into_parts` yields `GatewayHandle` + `GatewayControl` for
ServerHandle/AppState; the handle receives the complete listener/connection/
application/source/custody/Route task tree and the exact same retained
Application shutdown, Dataplane, cleanup-ledger and identity roots it
uses during close/drain/shutdown. Route absence is non-error and leaves public TCP/443
unbound until a complete generation converges. Connect/identity/mTLS/bind gate
failure likewise leaves the active operator-mTLS status surface available and
TCP/443 unbound; only configuration, Route/custody construction/storage or the
initial manual certified-key source is a fatal `GatewayBootError`.

`ServerHandle::shutdown` (`M:crates/overdrive-control-plane/src/lib.rs:2890-2993`)
first calls `GatewayHandle::shutdown` when present, as new stage 0
(D5 section (c)), before every existing stage, because admitted connections
ride leg-C; the remaining
`serve_lifetime` fail-stop bound is unchanged (D5-c above). The gateway handle computes one
absolute `deadline = clock.now() + grace` and always executes this sequence:

1. Call `connections.stop_accepting()`, await the listener task, and prove no
   further accept can acquire a connection lease through
   `tasks.join_listener(deadline, clock)`. Then call
   `application.close_for_shutdown(deadline)`, which closes every Current/
   Draining retention, atomically clears admission, and discards any staged
   candidate. Append command unavailable/timeout failure
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
   counts are zero. Then
   cancel Application, Manual Source, Custody and
   Route Set in that order. `join_listener` ran in step 1,
   `join_connections` runs after the force-join in step 2, and
   `join_remaining(deadline, clock)` joins every owner task or aborts+awaits it
   at the same absolute deadline. Command, inner-task, channel, timeout,
   cancellation and panic failures are appended and never skip a later task.
5. Call `identity.disable_after_drain(deadline)`. `begin_disable` clears the
   volatile `GatewayIdentitySlot` and bumps its epoch synchronously, so Current
   is Empty when the call observes it and no reconciler round-trip is needed at
   shutdown (the issuance audit row is already durable; a stale-epoch Issue or
   Drop completion is a no-op). The call therefore does not depend on the
   convergence loop being live. Append the exact typed error to
   `identity_error` and set `identity_empty=false` on timeout/error; success
   sets `identity_empty=true`. Close/retire command failures are accumulated in
   `application_errors`.

Only after that report returns does ServerHandle cancel/join convergence, then
resume the existing emit-drain, interest-router, axum-graceful, server-task,
exit-observer, shared-network-supervisor, mtls-worker, guest-flow and
mtls-resolve order (`M:crates/overdrive-control-plane/src/lib.rs:2890-2993`).
It accumulates the existing
mTLS failure plus any non-clean gateway report into `ServerShutdownError` after
all owners have been attempted. Deadline, cancellation, keep-alive and task-
panic paths therefore join users before application retirement/SVID drop and
expose typed residue instead of logging it away. Upstream connections end at
stage 0, before the worker that terminates leg-C (stage 6).

### Lifecycle Gate Ownership — application completion

| Gate | Owner / affected promise | Failure projection | Explicitly unaffected |
|---|---|---|---|
| Public Listener Bound | GatewayHandle/serve; dynamic TCP/443 after gates | no degraded/partial listener | Allocation Running, Service Stable, Backend Eligibility, operator HTTPS |
| Certified Key Usable | Custody current public key | expiry/loss stops accepts first | Route Accepted, Service/BPF, Gateway SVID |
| Gateway Identity Current | dedicated slot/lifecycle | reissue; no new admission/upstream when unusable | allocation SVIDs/public key/BPF |
| Connect Path Ready | `GatewayUpstreamConnector::probe` over the dataplane adapter's sentinel + actual-frontend probe (ADR-0230, ADR-0242) | staged cannot become Current; actual-frontend `NoBackend` is bind-valid | Route truth, Service Stable, Backend Eligibility, ServiceBackendRow |
| Gateway Applied Generation | GatewayApplicationOwner Current Arc | atomic current or no admission | BPF Hydrated/Backend Eligibility |
| Selected Peer Authenticated | gateway-client mTLS per request | 502/no fallback or retry | future requests/lifecycle state |

Seeded invariants cover staged-before-swap, current-to-draining/no-early-retire,
stale gate-result rejection, key/identity loss before unbind, typed
NoBackend/unavailable outcome without wait (ADR-0242), receipt cleanup/isolation, gateway SVID issue/reissue/drop and
unchanged allocation/Service/BPF meanings.

## Wave: DESIGN / [REF] Updated Reuse Analysis

| Existing component | Overlap | Final application decision | Why |
|---|---|---|---|
| `boot_server` (`M:crates/overdrive-control-plane/src/lib.rs:3625`) / `ServerHandle::shutdown` (`:2890-2993`) | startup/task/drain | **EXTEND** | Sole active gateway/app/listener/connector/drain owner (D5 slot, boot G0-G4, shutdown stage 0); no supervisor, no `*_override` |
| workload deploy parser/API | generic `deploy` journey | **EXTEND WITH OUTER DISCRIMINATOR** | Route stays outside workload enum/endpoint/stream |
| IntentStore + ADR-0048 codec | durable intent and wake | **EXTEND** | Reuse store/envelopes; lag-aware watch is required for fail-closed withdrawal |
| `ListenerFactStore` | listener/VIP projection | **DO NOT EXTEND** | Derived ServiceId key, stop eviction and binary absence cannot fulfill stable reference/cause contract |
| WorkloadIntent + `ServiceVipView` | exact frontend inputs | **REUSE** | Direct authoritative join; no duplicate map |
| ServiceLifecycle / ServiceBackendRow | Backend Eligibility | **REUSE INDIRECTLY; FORBID GATEWAY READ** | Sole complete-row owner remains |
| ServiceMapHydrator / Dataplane / XDP | backend application/selection | **REUSE HYDRATOR AND `Action::DataplaneUpdateService` UNCHANGED (D1); EXTEND DATAPLANE AND `cgroup_connect4_service`; XDP UNCHANGED** | Ordinary Service-map programming covers VM backends (D4 held); registered cgroup arm + dataplane-prepared upstream socket + receipt (D3, ADR-0243); no selector/raw handler access |
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
| serve + workload deploy | **bounded-change**: mandatory Enabled/Disabled AppState identity composition, `BootAdapters.gateway`, optional gateway handle/control/task tree and shutdown stage 0 only | disabled-state delta (boot with `gateway == None` is byte-equal to pre-feature), existing workload regression, boot/shutdown integration |
| IntentStore + ObservationStore | **bounded-change**: fixed Route/key and two status keys/tables only | codec goldens, store state-delta PBT, lag/relist DST |
| ListenerFactStore | **unbounded-preservation**: no call/import/change | dependency lint + source-diff guard |
| WorkloadIntent + ServiceVipView | **unbounded-preservation** read of exact reference inputs | adapter equivalence with whole-store/allocator complement |
| ServiceLifecycle / ServiceBackendRow | **unbounded-preservation** indirect input; gateway cannot read/write | import lint + pre/post row equality |
| ServiceMapHydrator / Dataplane / XDP | **bounded-change**: registered-socket arm of `cgroup_connect4_service`, the two intent/receipt maps, BackendId-to-identity association and upstream-socket preparation; hydrator, `Action::DataplaneUpdateService`, SERVICE/BACKEND/REVERSE_NAT contents, LOCAL_BACKEND_MAP behaviour for unregistered sockets and XDP preserved | state-delta DST, BPF POD tests, real-kernel complement probe (unregistered socket byte-for-byte unchanged) |
| BackendIdAllocator | **bounded-change**: read-only `existing_id`; allocator state unchanged by preflight | allocator snapshot PBT + conflict no-mutation integration |
| KEK/AEAD mechanism | **bounded-change**: distinct public-key HKDF/AAD/record; CA records preserved | tamper/fault integration + domain-separation golden |
| rustls/hyper | **bounded-change** per public/upstream connection and its finite budget | protocol matrix, wrong-peer integration and wire AT |
| tracing + CLI compact `EnvFilter` subscriber | **unbounded-preservation** of runtime state with one connector-owned DEBUG occurrence after validated receipt/identity resolution | append-only existing Tier-3 process log; no status/store/ring-buffer state |
| CA issue/audit + IdentityMgr | **bounded-change**: one gateway-subject audit output; allocation holder untouched | audit-before-hold integration + allocation-held-set complement |
| SvidLifecycle | **pure-function** sibling inputs -> gateway Actions/View only | paired lifecycle PBT + existing workload reconciler byte equality |
| arc-swap/tokio-rustls/async-trait/rustix/zeroize/x509-parser | **bounded-change** inside the owning snapshot/connection/syscall/secret values | API/trybuild guards, parser/syscall fault matrix and drain tests |

The mandatory CREATE NEW challenge is satisfied: no new component duplicates an
existing owner, and the new crate adds no deployment unit. `cargo xtask
dst-lint` enforces `control-plane -> {gateway,dataplane,reconcilers} -> core`,
the gateway's host edge (public-key codec; the cookie reader is
dataplane-internal, ADR-0243), and Dataplane's
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
| Four typed gateway clap/`ServeArgs` fields, `GatewayConfig`/`ManualCertifiedKeyConfig`, `GatewayConfigError` and accessors | clap `Command::Serve` → `main` → first `ServeArgs::gateway_config` call | `commands::serve::run_with_adapters` (`M:crates/overdrive-cli/src/commands/serve.rs:153`) moves only complete config into `ServerConfig.gateway`; `boot_server` and `UnboundGatewayBuilder::new` consume it; handlers never receive paths |
| `GatewayAppStateComposition`, canonical disabled identity read and appended AppState field/constructor parameter | broad fixtures default Disabled; production/Tier-3/Sim may pass Enabled | Hydration builder, every action/workflow wrapper and Route/status handlers through accessors only; Arc ownership preserves AppState Clone/borrow lifetime |
| `ControlPlaneError::GatewayDisabled` and exact response arm | Route POST/DELETE pre-parse control check | exhaustive `to_response` and live OpenAPI 409 ErrorBody |
| `ServiceFrontendResolve::{probe,resolve}` and closed errors | private `IntentServiceFrontendResolver` construction in the builder | gateway owner startup/application convergence only |
| `BootAdapters.gateway`/`GatewayAdapters::host`, `GatewayClientMtlsFactory`/`GatewayClientMtlsDeps`, `ComposedDataplane.gateway_connect` | `NodeAdapters::host`/`HostDataplaneComposer::compose` and their Sim swaps (`SimDataplaneComposer`, `SimGatewayClientMtlsFactory`) | `boot_server` G2-G4 only; no `*_override`, no downcast of `Arc<dyn Dataplane>` |
| `GatewayConnectDataplane` probe/`open_upstream`/receipt/identity/cleanup methods and `PreparedUpstream` | `EbpfDataplane` and `SimGatewayConnectDataplane` | `GatewayUpstreamConnector::{probe,connect}` (sole caller of `open_upstream`/`into_parts`), handle shutdown sweep and status live-count projection |
| Commit-gated backend-identity publication inside `EbpfDataplane::update_service` (ADR-0233) and crate-private `BackendIdAllocator::existing_id` | existing hydrator → unchanged `Action::DataplaneUpdateService` → action shim | `selected_backend_identity` reads only after the outer swap commits; no new action, `DispatchOutcome` or gateway argument |
| `IntentSubscriptionEvent` and revised `IntentStore::watch` | LocalStore and Sim | every existing watcher plus Route/custody/application subscribe-before-list loops |
| `SocketCookie`, `GatewayIdentityEpoch`, `CertifiedKeyGeneration`, `CertificateFingerprint` constructors/accessors | dataplane-internal socket reader, gateway slot, certified-key candidate/record decode | BPF adapter (`open_upstream`, receipt), lifecycle/action dispatch, custody/application/status respectively |
| `CertifiedKeyCandidate`, install command/outcome/errors and `PublicCertifiedKeyCustodyHandle::{id,install,record_install_failure,snapshot,subscribe}` | manual source/custody owner | manual source calls both mutations; Route resolution calls `id`; application owner calls snapshot/subscribe. There is no custody `withdraw`. |
| `PublicCertifiedKeyAeadCodec::{new,seal,open}` and protection context/record/envelopes | `GatewayAdapters::host(kek)` (same `Kek` as `NodeAdapters::host`, boot-probed) | custody only; feature-gated `with_random_fill_for_test` is called only by cross-crate codec integration; future #57 reuses custody `install`, never codec directly |
| `PublicCertifiedKeySnapshot` non-secret accessors plus crate-private TLS capability | custody only | application generation/status use metadata; public TLS runtime alone uses `server_config` |
| `GatewayApplicationGenerationId`, private-constructor `GatewayAppliedGeneration`, retention and non-Clone leases | application owner after all gate inputs | private admission, connection/request runtime and application status only |
| `UnboundGatewayBuilder` (`new`, `into_builder`), `GatewayBuilder`, `GatewayRuntimeDependencies`, `StartedGateway`, `GatewayUpstreamSeal` | `boot_server` (G2-G4) | same composition call chain; the seal is passed only to Host/Sim mTLS; no alternate factory/partial start |
| `GatewayHandle`, task tree/shutdown capabilities/report and ServerHandle field/error extension | `StartedGateway::into_parts` | `ServerHandle::{shutdown,abort_for_test}` only; typed residue is never dropped |
| `GatewayControl::{routes,application_status,certified_key_status}` | `StartedGateway::into_parts` | Route POST/DELETE and operator-mTLS Gateway GET only; no task/admission/key access |
| Status rows/envelopes/write variants/kinds/point reads and wire bodies | custody/application writers plus LocalStore/Sim adapters | `GatewayControl` and Gateway GET projection only |
| `GatewayLimits::first_slice` and read-only accessors | one call in `boot_server` | builder/runtime, `HostGatewayClientMtls` and BPF capacity assertion; no deserializer/builder/mutator |
| `normalize_route_authority`, `rewrite_request_headers` and `rewrite_response_headers` | public HTTP connection/request path | Route selection and upstream/downstream Hyper framing; all three are crate-private and their source-local PBT calls the same production functions |
| `GatewayIdentityView` | `GatewayIdentitySlot` | lifecycle hydration and status only; `GatewayIdentityView` has no `subscribe` |
| `GatewayIdentityLifecycleControl::{ensure_current,current,subscribe,disable_after_drain}` | control-plane adapter over slot/broker plus named Sim adapter over Sim target/real broker/Sim Clock | active application owner in production and seeded application composition in Sim |
| `GatewayIdentityActionComposition`/target and Sim target | Enabled/Disabled AppState composition | every exact action-shim dispatcher and only the Issue/Drop arms; no private-material getter |
| `GatewayIdentitySlot` constructor/non-secret reads plus ActionHandle, LifecycleHandle and client-access capabilities | `boot_server` (G1) | lifecycle-control adapter, Gateway SVID action executor and `HostGatewayClientMtls`; slot mutators/access closure are crate-private and only the matching opaque handle/adapter can reach them |
| `GatewaySvidLifecycle::canonical`, factory, Any* variants and Issue/Drop actions | control-plane canonical registration | existing reconciler runtime/broker/exhaustive dispatch and action shim only |
| `GatewayClientMtls::authenticate`, opaque `GatewayUpstream`, Host/Sim mTLS constructors | `GatewayClientMtlsFactory` (Host: `HostGatewayClientMtlsFactory` in `GatewayAdapters::host`; Sim: `SimGatewayClientMtlsFactory`) called by `boot_server` G2 | private connector readiness probe and request path; only successful exact-peer adapters hold the unforgeable seal needed to construct the authenticated stream |
| `gateway.upstream.receipt.validated` tracing occurrence | real `GatewayUpstreamConnector` only, after exact receipt validation and selected-identity resolution when applicable | existing tracing subscriber plus append-only `AT-PIG-E2E-1` process-log capture; no production reader, state/status projection or duplicate adapter emitter |
| `Invariant::{GatewayApplicationGenerationLifecycleIsSafe,GatewayApplicationShutdownConverges}` and evaluator functions | DISTILL scaffolds enum/canonical/evaluator/exhaustive-dispatch shapes plus ignored direct bodies; pending variants are absent from `Invariant::ALL` | typed ignored bodies alone call `Harness::only(variant)` until DELIVER activation; after real owners are GREEN, activation key `DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS` makes direct tests GREEN before adding both variants to the default catalogue |

The same audit removes the two previously dormant proposals:
`PublicCertifiedKeyCustodyHandle::withdraw` does not exist, and
`GatewayIdentityView::subscribe` does not exist. Subscription belongs
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
| tracing / tracing-subscriber | existing workspace pins and CLI compact `EnvFilter` subscriber | Existing structured occurrence path; one DEBUG connector event, no new logging backend/sidecar |
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
    /// Holds the Sim dataplane's `probe`/`connect` completion for one frontend
    /// (a driven-port fault on `SimGatewayConnectDataplane`; D2 replaces demand
    /// completion with the connect-probe gate).
    HoldConnectProbe(ServiceFrontend),
    ReleaseConnectProbe(ServiceFrontend),
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
`gateway-application-shutdown-converges`. DISTILL adds both enum variants,
canonical arms, evaluator signatures and the two exhaustive
`Harness::evaluate` arms below, so the crate compiles with closed matches while
the complete real-owner drivers remain pending activation:

```rust
Invariant::GatewayApplicationGenerationLifecycleIsSafe =>
    gateway_application::evaluate_generation_lifecycle_is_safe(seed).await,
Invariant::GatewayApplicationShutdownConverges =>
    gateway_application::evaluate_shutdown_converges(seed).await,
```

Pending registration is exact:

- Neither variant appears in `Invariant::ALL`, the expected-name arrays in
  `crates/overdrive-sim/tests/integration/{dst_harness_smoke.rs,dst_clean_clone_green.rs}`
  or any other default catalogue during DISTILL.
- `Harness::run(seed)` without `.only(...)` therefore cannot execute either
  evaluator. `Invariant::from_str` continues scanning `Invariant::ALL`, so
  `cargo dst --only <pending-name>` returns the existing unknown-invariant
  parse error until activation; there is no hidden CLI route.
- The exhaustive `as_canonical` and `Harness::evaluate` arms are present from
  the scaffold commit. Omitting a new arm is a compile failure; omission from
  `ALL` is deliberate reachability state, not an exhaustiveness exception.
- `crates/overdrive-sim/tests/acceptance/gateway_application_lifecycle.rs`
  may name the variants directly and call
  `Harness::new().only(Invariant::<variant>).run(seed)`. Its four complete
  acceptance bodies use exactly
  `#[ignore = "pending DELIVER activation DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS"]`.
  Removing that marker while any production owner is still a scaffold produces
  a real RED through `GatewayBuilder`, `GatewayControl`, or `GatewayHandle`.
- DISTILL authors both evaluator bodies completely. They apply and withdraw
  Route through `GatewayControl`, retain/release a real public TLS connection,
  recompose the owner through `GatewayBuilder`, and consume
  `GatewayHandle::shutdown`. Held/released completion uses only the approved
  Sim connect-probe hold/release fault. The ordered trace contains only redacted
  application status, non-secret identity reads, the dataplane intent/receipt
  live counts and typed shutdown projections. It cannot fabricate an `ObservableTracePoint`, encode
  a parallel owner model or return a handcrafted pass. `InvariantStatus`
  remains the existing `Pass | Fail`; no `Pending`, expected-fail result or
  default-DST exception is added.

The four direct body names are
`stale_completion_withdrawal_restart_and_retained_generation_are_safe`,
`shutdown_joins_users_before_gateway_identity_retire`,
`twin_generation_runs_at_one_seed_have_identical_ordered_observation_traces`
and `twin_shutdown_runs_at_one_seed_have_identical_ordered_observation_traces`;
no second activation test file is introduced.

The exact DELIVER activation step is
`DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS` (“Activate real Gateway
Application lifecycle invariants”). It starts only after the real
`GatewayApplicationOwner`, current/draining retention owner, listener/
connection owner and `GatewayHandle` shutdown composition are GREEN through
their preceding production-owner acceptance slices. Within that one step the
order is fixed:

1. Remove the exact pending markers from the four direct acceptance bodies;
   do not edit `Invariant::ALL` yet and do not rewrite their DISTILL-authored
   drivers or oracles.
2. Run the direct typed bodies and record semantic RED at any still-incomplete
   real Gateway Application owner call.
3. Complete only the DESIGN-pinned production owners needed to make all direct
   bodies GREEN, including twin-seed ordered-trace identity; no parallel
   simulation state machine or preauthored passing trace is permitted.
4. Append both variants exactly once to `Invariant::ALL` and to the two
   existing default-catalogue expected-name arrays. That existing `ALL` change
   simultaneously enables `FromStr`, both `cargo dst --only` spellings and
   default `Harness::run` execution.
5. Run the default catalogue GREEN. The activation step lands no intermediate
   state in which a pending/failing variant is default-reachable.

After step 5, the only CLI inputs remain the existing `--seed <u64>` and
`--only <NAME>`; these spellings become valid without a second parser:

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
the active listener. Connect-probe hold/release is implemented only by
`SimGatewayConnectDataplane` (a driven-port fault carrying the real
`ServiceFrontend`); it neither invokes a private owner method nor
adds a production hook. `SimClock` and the existing Sim identity/dataplane/mTLS
adapters supply time and effect completion. The seed and generated schedule
are included in the failure cause, and twin runs at one seed must produce the
same ordered observation trace.

`generation_lifecycle(seed)` always applies three generations of the same
Route ID, retains a connection on the first Current, holds the second
generation's connect-probe completion, supersedes it with the third, releases the
stale second completion, promotes the third, withdraws it, releases the first
connection, and restarts once against the unchanged durable intent at a
seed-selected legal boundary. Its oracle reads only
`GatewayApplicationStatusRowV1` plus the dataplane live intent/receipt counts and
asserts at every observation:

- Staged precedes promotion, Current changes in one generation step, and the
  displaced Current remains Draining while its retained connection is open.
- No generation retires before its retained connection closes.
- A released probe completion whose generation no longer names Staged
  never becomes Current and never retires the successor.
- Withdrawal unbinds/stops admission before clearing Current, then converges
  to no Staged/Current/Draining generation after retained leases close. Restart
  relists durable Route intent and never resurrects a withdrawn/superseded
  generation.

`shutdown(seed)` begins with one Current retained connection and one Staged
generation whose connect-probe completion is held, then permutes completion/connection
release around `GatewayHandle::shutdown`. Its oracle uses the final
`GatewayShutdownReport`, the last application status row
and non-secret identity read. A passing trace has listener unbound and no new
admission, no Staged or Current application,
all retained leases joined, `identity_empty == true`, zero ledger entries, and
`live_intents == Some(0)` plus `live_receipts == Some(0)`. Any typed task,
application, identity or cleanup failure makes the invariant fail with the
seed; the evaluator never treats a non-clean report as convergence. These are
observable-owner oracles, not copies of private owner fields or a simulated
replacement state machine.

| Evidence lane | Required contract |
|---|---|
| Pure/PBT | `PathMatch::matches_raw_path`, `runtime::routing::normalize_route_authority` and both `runtime::proxy_headers::rewrite_*_headers` functions own the Route-path/authority/header properties; Route/hash laws, receipt matching/cleanup, immutable BackendId identity, gateway SVID lifecycle and finite limit ordering remain in their owning source modules. Every live property carries `/// CONTRACT_SHAPE: pure-function.` |
| DST | `GatewayApplicationGenerationLifecycleIsSafe` owns staged/current/draining, stale completion, withdrawal and restart safety; `GatewayApplicationShutdownConverges` owns joined shutdown convergence. DISTILL scaffolds their typed ignored direct paths but excludes both from `Invariant::ALL`; `DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS` activates them only after real owners are GREEN and direct bodies pass. Registered/unregistered isolation, the typed NoBackend/unavailable outcome (ADR-0242), connector cleanup and key/identity fail-closed complements remain registered with their owning component evidence. |
| In-process integration | Real stores/Route/custody/application/status; Sim connect/identity/mTLS equivalence (the host/Sim `GatewayConnectDataplane` equivalence test); typed boot/shutdown and concurrent retained generations; production `ServerHandle` ordering. Rust tests do not spawn the product binary. |
| `AT-PIG-E2E-1` recurring production-composition test (native metal) | An in-process Rust test that boots the production composition (`run_server` with the real `NodeAdapters::host` and `GatewayAdapters::host`, no `*_override`) on the native x86_64 KVM host via `cargo xtask metal run --` (the `tests/conformance` crate declares only the `integration-tests` feature; the test is named `native_metal_kvm_public_ingress_gateway_*`, carries a reasoned `#[ignore]`, and is activated only by `tests/conformance/run-native-metal.sh`, which runs `--features integration-tests --run-ignored ignored-only -E 'test(/^native_metal_kvm_/)'`; no `kvm-tests` feature exists in that crate), deploys a VM `Service` and the Route through the exported server handler and its public API, and makes the external TLS1.3/HTTP1.1 request over a real socket. It does not spawn the `overdrive` binary (`.claude/rules/testing.md` § "Classify external execution"). Placement is `tests/conformance` because it crosses the gateway, control-plane, dataplane and worker owners and is observed through the exported server handler; the subtree `CLAUDE.md` defines composition. Per completed request it reads the `gateway.upstream.receipt.validated` occurrence from an in-process tracing capture layer: Selected proves the exact socket cookie, ServiceKey components, BackendId and resolved expected-peer SpiffeId before the exact-peer mTLS/streamed-response evidence; NoBackend proves the same cookie/ServiceKey with structurally absent selected-only fields before the 503. It is not an expectation. |
| Tier 3 (native metal) | The production connector occurrence replaces timing-based receipt-map polling and strace as the transient receipt/identity oracle. Real-kernel complements: registered-socket selection through the attached `cgroup/connect4` (V-GW-1..5), unregistered socket nonmutation (`LOCAL_BACKEND_MAP` behaviour, XDP), the packet entry (`SO_MARK` + `127.0.0.1` source → table 100 → TPROXY → leg-C) with the captured leg-B/leg-C protected segment showing TLS 1.3 records (not a workload veth), kTLS (`ss -K`), wrong-valid-peer and final cleanup. Stable map/attachment cleanup checks may continue but cannot claim a consumed receipt's values. |
| Point-in-time EDD `E15-PIG-OPERATOR-JOURNEY` | `verification/expectations/E15-public-ingress-operator-journey/runner.sh` requires a clean harness-captured HEAD, runs the exact `cargo xtask bpf-build` path, hashes that BPF object, builds and hashes that checkout's default-feature binary on the native metal host (uid 0), and proves the binary embeds that exact object before independently driving the checked-in example with the operator hostname/public chain/key and normal system trust. It records build provenance plus stakeholder-visible status/TLS/response evidence only and neither calls nor replaces `AT-PIG-E2E-1`. The outer invocation is `cargo xtask metal run -- verification/harness/run-expectation.sh E15`. |

### Artifact-boundary handoff to DISTILL

DESIGN creates no tests or examples, but fixes their distinct homes so DISTILL
cannot collapse the evidence layers:

| Artifact | Exact planned path and role |
|---|---|
| Operator-runnable journey | `examples/public-ingress-gateway/README.md`, `service.toml` (a VM `Service`, `[vm]`; never `[exec]`), `route.toml.template`, `prepare-test-ca.sh`, and `run-example.sh`. The guest-side HTTP/1.1 server is carried by the VM Service fixture shape the E08 VM Service already uses (D4 findings); there is no `workload.py` Exec workload. The script accepts the exact argv contract below, materializes the checked-in Route template under the caller's scratch directory, drives real `serve` plus Service/Route deploy (`overdrive deploy <SPEC>`), and makes the external TLS1.3/HTTP1.1 request. It owns no regression oracle and is run only by the E15 expectation and by operators. |
| Recurring production-composition test | `AT-PIG-E2E-1`, the in-process Rust test described in the evidence-lane table above (`tests/conformance`, native metal). It replaces the former shell runner under `tests/tier3/public-ingress-gateway/`, which spawned the built binary and is obsolete (a test never spawns the `overdrive` binary). It uses the in-process receipt occurrence plus existing public/kernel/wire/cleanup surfaces and creates no new CI lane (a scheduled metal lane is PROPOSED P-6). |
| Point-in-time expectation | `verification/expectations/E15-public-ingress-operator-journey/{README.md,runner.sh}`. Its operator input set is exactly `PIG_E15_GATEWAY_ADDRESS`, `PIG_E15_PUBLIC_HOSTNAME`, `PIG_E15_CERTIFICATE_CHAIN` and `PIG_E15_PRIVATE_KEY`. The metal/uid-0 runner produces the BPF object with the exact `cargo xtask bpf-build` path from the harness-captured clean HEAD, records its command/output/status/digest, builds the default-feature product, externally proves exact-object embedding, records source/product provenance, invokes the same root example in production-trust mode, and captures stakeholder-visible output. It accepts neither a product-binary nor public-CA input and has no recurring-regression role. |

For `AT-PIG-E2E-1`, the connector occurrence is captured by a tracing capture
layer installed in the test process before `run_server` starts; the capture
buffer is append-only for the run, is never truncated or replaced, and retains
the first occurrence even when a later assertion or cleanup fails. Before each
gated public request or NoBackend operation the test records the buffer length;
after the operation reaches its bounded public outcome it reads only the newly
appended records. That suffix must contain exactly one occurrence whose
structured `event` field is `gateway.upstream.receipt.validated` and whose
`operation` field is `request`; it must satisfy the Selected/NoBackend field
rules above. Probe occurrences remain distinguishable by structured
`operation = "probe"`.

The test does not poll the transient receipt map, install a cleanup delay or
infer receipt values from strace: the receipt has already been consumed before
either tool can observe those semantics. `bpftool` may still prove stable
post-operation cleanup/attachment facts, and strace may still diagnose syscall
reachability, but neither is an oracle for cookie/ServiceKey/outcome/
BackendId/expected-peer fields.

The root example has one argv-only input contract; no environment variable may
override its gateway address, Public Hostname, certificate paths or trust mode:

```text
examples/public-ingress-gateway/run-example.sh \
  <absolute-overdrive-binary> \
  <absolute-scratch-dir> \
  <gateway-ipv4> \
  [--public-hostname <PUBLIC_HOSTNAME> \
   --certificate-chain <absolute-chain-path> \
   --private-key <absolute-key-path>]
```

The first three positionals are mandatory. The three named options are
all-or-none and each occurs once; partial, repeated or unknown options exit 2
before startup. With no named options, hermetic mode uses
`api.example.com` as its only default, creates a bounded test CA/certificate/key
under the scratch directory, and the example's public curl trusts that generated
CA with `--cacert`. With all three options, production-trust mode uses the
operator-supplied `PublicHostname`, leaf-first public chain and matching PKCS#8
key. It uses normal system trust for the public request and permits neither
`--cacert` nor an insecure verification bypass.

In both modes the script replaces the one `__PUBLIC_HOSTNAME__` token in the
checked-in template, writes `<absolute-scratch-dir>/specs/route.toml`, and
deploys that materialized file. The same hostname drives Route `hostname`,
curl URL, TLS SNI, HTTP Host and certificate validation. The Route's
`certified_key = "api-origin"` remains byte-equal to
`--gateway-certified-key-id api-origin`; `[route.target].service = "api"`
remains byte-equal to the checked-in Service's logical `WorkloadId`. The
example passes `<gateway-ipv4>` only to `--gateway-address` and curl
`--resolve`; it never writes that address or a derived `ServiceId` into Route.

E15's four operator environment variables are inputs to the expectation runner,
not alternate inputs to the root example. `PIG_E15_OVERDRIVE_BIN` and
`PIG_E15_PUBLIC_CA` are absent inputs: setting either does not select an
artifact and is rejected as stale configuration. (E14 on main is
`E14-vm-service-post-greenfield-cut`; this feature's expectation is E15.) The
exact outer invocation is
`cargo xtask metal run -- verification/harness/run-expectation.sh E15`, which
must enter the native x86_64 KVM host as uid 0 and fail closed on the
substrate probes (`.claude/rules/testing.md` § "Running tests — native x86_64
bare-metal KVM host"), with the existing cgroup-v2/bpffs checks.

Before creating or updating E15 evidence, the generic expectation harness
captures `git rev-parse HEAD` and
`git status --porcelain --untracked-files=all`. E15 requires the status result
to be empty, exports that SHA as `EXPECTATION_SOURCE_SHA` and exports
`EXPECTATION_SOURCE_DIRTY=false` beside the existing `REPO_ROOT` and
`EVIDENCE_DIR` boundary. The E15 runner re-reads HEAD and requires byte-equality
with `EXPECTATION_SOURCE_SHA`; it also rechecks the tree excluding only the
harness-owned `verification/expectations/**/evidence/**` outputs written after
the clean capture. Dirty source, an absent/invalid captured SHA or either SHA/
dirty-state mismatch fails before build.

Before the product build, the runner runs exactly `cargo xtask bpf-build` from
the same captured clean HEAD. It captures the literal command, complete
stdout/stderr, exit status, resolved
`<REPO_ROOT>/target/bpf/overdrive_bpf.o` path and SHA-256, then rechecks HEAD
and clean source state. Only after that succeeds does the runner create a
private temporary product-build root and run exactly:

```text
cargo build --locked --package overdrive-cli --bin overdrive \
  --target-dir <runner-build-root>/target
```

Omitting `--features`, `--all-features` and `--no-default-features` is
load-bearing: this is the package's default-feature binary. The resolved
artifact is exactly `<runner-build-root>/target/debug/overdrive`; it must be a
regular executable file. The runner records in E15 evidence the captured source
SHA, the empty dirty-state result, literal build command, build exit, resolved
artifact path, `rustc -Vv`, `cargo -V` and
`sha256sum <resolved-binary>`. It then externally finds the complete captured
BPF-object byte sequence exactly once in the built binary, recording its byte
offset and digest. Immediately before invoking the example it recomputes both
SHA-256 values and requires equality with the recorded digests. HEAD mismatch,
dirty checkout, either build failure, missing/non-executable artifact,
binary/BPF digest mismatch or absent/duplicate exact-object embedding fails
closed. These are ordinary E15 evidence files; no attestation API, process,
service or sidecar is introduced.

The evidence paths are fixed under
`<EVIDENCE_DIR>/build-provenance/`: `source.txt` contains
`source_sha=<EXPECTATION_SOURCE_SHA>` and `dirty=false`; `build-command.txt`
contains the normalized command above with `<runner-build-root>` replaced by
the actual absolute path; `build.out` contains complete stdout/stderr plus
`exit=<code>`; `toolchain.txt` contains the complete cargo/rustc outputs;
`artifact.txt` contains the resolved absolute binary path; and
`overdrive.sha256` contains the GNU `sha256sum` line. The sibling
`bpf-build-command.txt`, `bpf-build.out`, `bpf-object.txt`,
`overdrive_bpf.sha256`, `source-after-bpf-build.txt` and
`bpf-consumption.txt` records capture the exact BPF producer, clean source,
object and product-consumption proof. The generic
`verification.yaml` records the same source SHA and `working_tree_dirty: false`.

Only after provenance succeeds does the runner map its four operator inputs to
the root example's argv contract:

```text
run-example.sh <runner-build-root>/target/debug/overdrive \
  <runner-scratch>/product <PIG_E15_GATEWAY_ADDRESS> \
  --public-hostname <PIG_E15_PUBLIC_HOSTNAME> \
  --certificate-chain <PIG_E15_CERTIFICATE_CHAIN> \
  --private-key <PIG_E15_PRIVATE_KEY>
```

The production-trust public request uses
`curl --resolve "<PIG_E15_PUBLIC_HOSTNAME>:443:<PIG_E15_GATEWAY_ADDRESS>"`
with URL `https://<PIG_E15_PUBLIC_HOSTNAME>/stream`. `--resolve` is the explicit
client-local hostname override: it changes only
that curl process's address selection; it preserves the runtime hostname for
SNI, Host and system-trust certificate verification. E15 never accepts or uses
`--cacert` or `--insecure` for this public request, and no step uses DNS or
edits `/etc/hosts`. The hermetic test-CA mode likewise reaches `api.example.com`
only through `curl --resolve`.
`api.example.com` is therefore confined to hermetic test-CA mode and is never
the E15 production hostname.

Production reachability requires the operator to provision an IPv4 A record
from the chosen Public Hostname to the configured gateway IPv4 before serving
real users. Overdrive has no public-DNS provider/API, does not edit
`/etc/hosts`, and does not use curl `--resolve` outside the checked-in example
and its black-box consumers.

The E15 runner supplies the public certified key only through the real
`overdrive serve` arguments; `AT-PIG-E2E-1` supplies it through the same
`ServerConfig.gateway` field `serve` fills. Neither may hand-program a BPF map,
install a TPROXY rule or nft rule, inject a Route/application snapshot, fabricate a gateway
identity or (E15) use an implementation crate. The recurring test cleans
its process/cgroup/socket state even on assertion failure. DISTILL owns the
executable scenarios and matrices; this section fixes only their architecture
boundary, production composition and non-substitution rule.

### Full-stack handoff status

The reconstructed DESIGN is the final ADR set 0220-0243 (system, Route, Domain
and Application decisions; numbering per [`adr-renumbering.md`](adr-renumbering.md)) plus this document. It is PROPOSED against main
549d78fa6 and is not an approved handoff until the user confirms the D5 "New
public surface" list and rules on the BLOCKING proposals P-2, P-9 and P-10 (the optional P-1, P-3, P-4, P-6 and
the P-8 simplification may be ruled later). The contract-shaping items V-GW-1..5 have run on native metal; the
residuals are source-settled or DISTILL native obligations (see Contract-shaping validation items). The 2026-09-13 approval (preserved at git ref `backup/research-ingress-gateway-pre-reconstruct`) covers the decisions carried unchanged (class 1, including item 12 of the D5 list, reconfirmed on main); the reconstructed and changed parts need this review plus the user's D1-D5 rulings and the blocking P-items. The cgroup-BPF hook—not XDP—
selects the host-originated gateway socket; XDP wire ingress remains unchanged.
The Gateway Application never programs, waits for or observes backend state;
ordinary `ServiceMapHydrator` programming covers VM backends (D1). Public 404,
NoBackend 503 and internal 502 are active exact
mappings. Manual Public Certified Key is the only producer; #57 ACME remains
later and additive.

`AT-PIG-E2E-1` is the first recurring production-composition acceptance
boundary, not an expectation or spike. No isolated scratch
mechanism or roadmap is introduced in DESIGN. DISTILL may author draft
scaffolds against the exact interfaces, lifecycle/error semantics and
evidence lanes above after the approval above.

### Obligations handed to DISTILL

Each line is a behaviour with its required evidence lane. Tier 1 = seeded
`overdrive-sim`; Tier 3 = real kernel via `cargo xtask metal run --`; Native =
native-metal real VM Service through leg-C and vsock. No obligation spawns the
`overdrive` binary except E15.

1. Replacement for acceptance gap C4a (old "demand apply twice"): two consecutive
   public requests while no healthy backend exists both return 503 with identical
   bodies, `live_intent_count() == live_receipt_count() == 0` after each, and no
   change in `GatewayApplicationStatusRowV1` or the Gateway Applied Generation;
   the outcome is typed `NoBackend` at the consuming port, bounded by the request
   deadline, never fabricated and never a wait (ADR-0242). Tier 1 (Sim dataplane
   `NoBackend` script) plus Tier 3 (real hook, no outer entry for the `ServiceKey`).
2. D4 edge observation, `healthy` lag (**pending P-9**): the `BACKEND_MAP` healthy bit lags readiness by
   about 1 s. If P-9 honours `healthy`, a request in that window is 503; if it ignores it, 502. In
   either case the first request after `healthy` 0 → 1 reaches the backend with no gateway action.
   Tier 3 and Native.
3. A stale map entry yields 502, never a wrong-peer success (D4: entries outlive `job stop`; the nft
   output chain resets the dial for the unregistered tuple). Tier 3 and Native. The map lingering
   itself is not a gateway fix (P-4).
4. Boot gate: actual-frontend `NoBackend` is bind-valid; sentinel probe failure keeps
   TCP/443 unbound, serve and workloads unaffected, status carries the cause and the
   `health.startup.refused` event with reason `gateway.connect_probe` is emitted. Tier 1 plus Tier 3.
5. D5 boot/shutdown: gateway disabled (`ServerConfig.gateway == None`) leaves boot
   byte-identical; enabled boot orders G0-G4; shutdown stage 0 runs before the leg-C worker stage, joins
   users, sweeps intents/receipts and clears the volatile identity slot, inside the one 5 s drain
   and the 10 s fail-stop bound; a gateway task failure never requests serve shutdown. Tier 1 (production
   `ServerHandle` stage events with `SimDataplaneComposer`) plus Tier 3.
6. Selection and entry: a registered socket is selected by the hook, an unregistered
   socket behaves exactly as before, and the rewritten dial enters leg-C via table 100 and TPROXY with
   `SO_MARK` and the `127.0.0.1` source (ADR-0190). Tier 3 (hook not testable at Tier 2).
7. Exact-peer mTLS: wrong-valid-peer, expired, absent identity and matching peer, evidenced on the
   captured leg-B/leg-C protected segment (TLS 1.3 records, zero cleartext) and kTLS state, not on a workload veth. Tier 3.
8. End to end: external TLS1.3 client → gateway → leg-C → kTLS → `GuestStreamDial` → vsock → VM Service
   guest, streamed response, 404/421/400/502/503 mappings. Native (`AT-PIG-E2E-1`).
9. Gateway SVID lifecycle (issue, audit-before-hold, near-expiry reissue, epoch-stale no-op, drop after
   drain, restart reissue). Tier 1.
10. Custody replacement preserves last usable key; Route/custody/application watch recovery
    (subscribe-before-list, relist on lag). Tier 1.
11. Receipt/intent correlation: no residue after cleanup; restart destroys receipts; BackendId identity
    immutable and readable only after the outer swap commits. Tier 1 plus Tier 3.
12. Quint conformance (`.claude/rules/testing.md` § "Quint conformance"): specification directory `specs/quint/public-ingress-gateway/`
    (modules `gateway_identity`, `gateway_application`, `selection_receipt`); one quint-connect driver per module in the owning
    crate's `src/` behind `#[cfg(all(test, feature = "integration-tests"))]` (gateway_identity: `overdrive-control-plane`, `src/gateway_identity_lifecycle/quint_gateway_identity.rs`, reaching the
    `pub(crate)` dispatch functions in-crate; gateway_application: `overdrive-gateway`; selection_receipt:
    `overdrive-dataplane`, `src/quint_selection_receipt.rs`, owner = the real `EbpfDataplane` on native metal, bound runs
    `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` over the coarsened `receipt_conf` main,
    see Formal protocol model; the remaining `selection_receipt` runs and hazards are model-checked and covered at Tier 3 as stated
    there); every driver has `#[quint_test]` for each named
    run in the Formal protocol model section and `#[quint_run]` randomized traces; every action handler calls the pinned
    driving port; `State` projects only the pinned accessors. Tier 1 conformance (no `quint` CLI/REPL).
    The `gateway_application` module covers only drain/supersession; Route+key coherence, bind gating and
    replacement preservation are seeded-sim/unit obligations (items 1, 4, 10 and the Required boundary scenarios).
13. Seeded invariants `GatewayApplicationGenerationLifecycleIsSafe` and `GatewayApplicationShutdownConverges`
    with the connect-probe hold/release fault. Tier 1.
14. E15 verification expectation: `curl --resolve` explicit hostname override, VM Service, clean captured
    HEAD, BPF-object embedding proof. Point-in-time capture on metal, reviewed by a different agent; not a test.
15. Scaffold file ownership in `overdrive-dataplane/src/mtls/`: `gateway_identity.rs` (slot, handles,
    `GatewayClientIdentityAccess`) and `gateway_client.rs` (`HostGatewayClientMtls`, `HostGatewayClientMtlsFactory`).
    The DISTILL scaffold table below names only `gateway_identity.rs` for both and must be corrected; the
    outcome registry artifact `gateway_client.rs` is unchanged.

Fault stimuli are injected through driven ports only (Sim dataplane/identity/mTLS, probe hold/release); the
production path authors the resulting observations. Each scenario records contract, owner path, stimulus,
oracle and tier, per `.claude/rules/testing.md` § "DISTILL — prove composed system behavior".

Class-5 blockers for DISTILL scaffolding: none new. The only missing primitives
are the D5 "New public surface" items, which are pinned here and await
confirmation. Every quint-connect driver lives in its owning crate's `src/`
(none spans owners through the exported server handler); only `AT-PIG-E2E-1`
is in `tests/conformance`. Quint conformance runs under `integration-tests` via
the repository's quint runner (`cargo xtask lima run --` where a Lima VM exists,
otherwise `cargo xtask metal run --`); real-kernel and VM evidence runs via
`cargo xtask metal run --`.

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

## Wave: DISTILL / [REF] Reconciliation and warnings

**DISTILL status: in progress — code reconciliation pending (phase 2).** This block specifies the acceptance suite for the reconstructed DESIGN (final ADR set 0220–0243, this document's DESIGN part). No test, scaffold, example or expectation file has been written yet; every "Scaffolds" and "RED classification" statement below is a plan, not an execution result. The full working plan with per-scenario main-API pins, batch file lists and cross-batch compile order is [`distill/code-reconciliation-plan.md`](distill/code-reconciliation-plan.md) (not a delivery artifact). **All phase-2 code batches (B1–B5) wait for DESIGN approval**: the user's rulings on the D5 shape confirmation (items 1–11 and item 12) and on the three blocking proposals P-2, P-9 and P-10. Until then DISTILL is document and plan only.

Reconciliation: DISCUSS, DEVOPS and separate DESIGN `wave-decisions.md` files do not exist (this feature uses one narrative `feature-delta.md`); 0 contradictions among the DESIGN sections. Observation-surface provenance (batch B1 check): the two exact `ObservationStore` point reads (`public_certified_key_status_row`, `gateway_application_status_row`) and the new `ObservationWrite`/`ObservationRow`/`ObservationRowKind` variants `PublicCertifiedKeyStatus` and `GatewayApplicationStatus` were part of the DESIGN the user approved on 2026-09-13 (`git show backup/research-ingress-gateway-pre-reconstruct:docs/feature/public-ingress-gateway/feature-delta.md`: status banner lines 3–12 and the closed-observation-surface block at lines 3618–3640; identical signatures in this document at lines 3844–3858). They are carried as previously approved and are not on the confirmation list; only item 12 gates B1's watch files. Warnings, not blockers: no story-ID traceability; the project ATDD policy (`docs/architecture/atdd-infrastructure-policy.md`) has no gateway rows yet (appended in phase 2); there is no Lima instance in the authoring environment, so Linux-side compilation and runs use `cargo xtask metal run --`.

**The DESIGN is not an approved handoff** (Full-stack handoff status above). DISTILL therefore treats every reconstructed contract as "proposed, user confirmation pending" and stays neutral on the three BLOCKING proposals: **P-2** (BackendId memo key or order), **P-9** (whether the registered hook honours `BACKEND_MAP.healthy`) and **P-10** (placement of the selection arm). No scenario's oracle depends on an unruled option; the affected behaviours are listed as blockers in the Completeness audit and are not authored.

Language mode Rust; deliverable type application; density lean, Tier-1 `[REF]` sections only. Repository rules override generic nWave examples: no `.feature` files, no Python steps; Gherkin-style text below is specification only (`.claude/rules/testing.md`). Acceptance bodies are complete and executable; future scenarios carry `#[ignore = "pending DELIVER <step>"]` naming a domain-named activation step; no placeholder panics.

Retired premises (explicit, not carried): the branch DISTILL's gateway-frontend-demand scenarios and apply-twice gap (D2, ADR-0242), the demand-gated Service-map TEACH (D1), the shell Tier-3 runner under `tests/tier3/public-ingress-gateway/` that spawned the built binary (tests never spawn the `overdrive` binary), the Lima GitHub workflow lane, the `[exec]` workload example, and the branch E14 expectation id (E14 on main is `E14-vm-service-post-greenfield-cut`; this feature's expectation is E15).

## Wave: DISTILL / [REF] Scenario list and tags

Counts against the 55 pre-reconstruction scenarios: **22 KEEP, 32 ADAPT, 1 DROP** (S-PIG-11, demand revision type removed) **plus 20 NEW** (S-PIG-56..75) = 74 live scenarios (22+32+20; S-PIG-75 is conditional on a user acknowledgement) and 75 scenario ids including the DROP row. Count reconciliation: the earlier document figure of 69 counted live scenarios (22+32+15) and the plan's 70 counted ids including the DROP row; both are now stated in these two units. Three quint-connect conformance drivers (QC-1..3, 11 bound `#[quint_test]` plus 3 `#[quint_run]`) and one EDD expectation (E15). Scenario ids are stable identifiers. Every scenario carries one `@contract-shape:` tag; error/edge/boundary/security/degraded/lifecycle/concurrency share is above 80% (gate 40%). Lane: **U** unit/proptest, **I** `integration-tests` in-process, **S** seeded `overdrive-sim`, **T3** real kernel on native metal, **N** native metal with a real VM Service, **QC** quint-connect.

| ID | Scenario contract | Disp. | Lane | Tags | Owning crate / target on main | Activating step |
|---|---|---|---|---|---|---|
| S-PIG-01 | An external user receives the api workload's streamed response through the public Route (`AT-PIG-E2E-1`) | ADAPT | N | @walking_skeleton @driving_port @real-io @contract-shape:bounded-change | `tests/conformance/tests/public_ingress_gateway_end_to_end.rs` | public-request-end-to-end |
| S-PIG-02 | A sound listener with no selectable backend returns 503 and no fallback; unavailable path is typed, never a wait | ADAPT | I+N | @error @security @contract-shape:bounded-change | control-plane integration `public_ingress_gateway_boot_shutdown.rs` + conformance file | consuming-port-unavailable-outcome |
| S-PIG-03–10 | Route path/host/input/generation laws (exact and segment match, rejected partitions, label 62/63/64, input channels, Route/key/application generation) | KEEP | U | @property/@boundary/@error @contract-shape:pure-function | `overdrive-core/src/public_ingress.rs` | route-value-types |
| S-PIG-11 | Demand revisions nonzero, monotone, refuse wrap | DROP | — | retired (D2) | — | — |
| S-PIG-12–17 | Canonical Host/SNI, authority errors, header rewrite both directions, hop-by-hop removal, fixed finite limits | KEEP | U | @property/@security/@boundary @contract-shape:pure-function | `overdrive-gateway/src/runtime/{routing,proxy_headers,limits}.rs` | gateway-application-owner |
| S-PIG-18–20 | Four serve flags; repeated scalar rejected; all-or-none enablement masks name exactly the missing fields | KEEP | U | @driving_adapter @error @contract-shape:pure-function | `overdrive-cli/tests/acceptance/public_ingress_gateway.rs` | route-operator-surface |
| S-PIG-21–24 | Route declare/replay/replace/withdraw lifecycle; competing id; wrong-id withdrawal; two concurrent claims | ADAPT | I | @real-io @error @concurrency @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway.rs` | gateway-application-owner |
| S-PIG-25 | Gateway status contains no path, key, ciphertext, nonce or salt | ADAPT | I | @security @contract-shape:unbounded-preservation | same | gateway-application-owner |
| S-PIG-26 | Clean shutdown leaves identity empty and no connect residue | ADAPT | I | @lifecycle @contract-shape:bounded-change | same | gateway-boot-and-shutdown |
| S-PIG-27 | Unreadable manual key files refuse boot without Route mutation | ADAPT | I | @error @degraded-resource @contract-shape:unbounded-preservation | same | certified-key-custody |
| S-PIG-28–30 | Missing receipt / other-ServiceKey receipt / wrong-valid selected peer fail closed, clean up, never retry another backend | ADAPT | U/I | @error @security @contract-shape:bounded-change | `overdrive-gateway/src/connector.rs` | upstream-socket-preparation, selection-receipt-and-applied-identity |
| S-PIG-31 | Stale probe completion, replacement, withdrawal, retained connection and restart preserve generation safety (invariant `GatewayApplicationGenerationLifecycleIsSafe`) | ADAPT | S | @property @seeded-sim @contract-shape:bounded-change | `overdrive-sim/tests/acceptance/gateway_application_lifecycle.rs` | gateway-application-invariants |
| S-PIG-32 | Shutdown joins admitted users before the identity slot clears (`GatewayApplicationShutdownConverges`) | ADAPT | S | @property @seeded-sim @interruption @contract-shape:bounded-change | same | gateway-application-invariants |
| S-PIG-33 | Withdrawing from Empty is Unchanged; the empty set survives reopen | ADAPT | I | @edge @real-io @contract-shape:bounded-change | gateway integration | gateway-application-owner |
| S-PIG-34–36 | Route deploy one-shot for both detach modes; real POST/DELETE handlers via ApiClient; disabled composition rejects before parse | ADAPT | I | @driving-adapter @error @contract-shape:bounded-change | `overdrive-cli/tests/integration/public_ingress_gateway.rs` (boots through `SimDataplaneComposer`) | route-operator-surface |
| S-PIG-37 | Cleanup ledger 0/1/128, refuses 129 before registration | ADAPT | U | @boundary @contract-shape:bounded-change | `overdrive-gateway/src/application.rs` | upstream-socket-preparation |
| S-PIG-38, 43, 52 | Real rustls: valid peer B rejected when selected is A before any body byte; peer observes the dedicated gateway SVID; absent/expired/malformed identity, non-peer fd, elapsed handshake, URI-SAN shape | ADAPT | I | @security @error @real-io @contract-shape:bounded-change | `overdrive-dataplane/tests/integration/public_ingress_gateway_mtls.rs` | exact-peer-client-mtls |
| S-PIG-39 | Request outside a narrow replacement Route returns 404 | ADAPT | N | @error @public-https @contract-shape:bounded-change | conformance file | public-request-end-to-end |
| S-PIG-40 | A selected upstream closing before response maps to 502 exactly once, no retry | ADAPT | N | @error @single-attempt @contract-shape:bounded-change | conformance file | public-request-end-to-end |
| S-PIG-41 | An unregistered cgroup socket keeps non-service behaviour; `LOCAL_BACKEND_MAP` and XDP attachments unchanged (**expected-GREEN preservation row**) | ADAPT | T3 | @security @real-kernel @contract-shape:unbounded-preservation | `overdrive-dataplane/tests/integration/public_ingress_gateway_hook.rs` | gateway-selection-hook |
| S-PIG-42 | Status point-read failure returns 500 without credential detail | ADAPT | I | @error @degraded-resource @contract-shape:unbounded-preservation | CLI integration | route-operator-surface |
| S-PIG-44 | Gateway newtypes round-trip Display/FromStr/TryFrom/serde/rkyv and reject each noncanonical partition | KEEP | U | @property @codec @contract-shape:pure-function | `overdrive-core` | route-value-types, gateway-identity-epoch |
| S-PIG-45 | Certificate chains 0/1/many/9 inside one validity interval | KEEP | U | @boundary @security @contract-shape:bounded-change | `overdrive-gateway/src/certified_key.rs` | certified-key-custody |
| S-PIG-46 | Key install replay, replacement, invalid replacement, expiry deltas | ADAPT | I | @lifecycle @error @contract-shape:bounded-change | gateway integration | certified-key-custody |
| S-PIG-47 | Tamper, AAD mismatch and CSPRNG failure expose no plaintext or protected record | KEEP | U | @security @error @contract-shape:bounded-change | `overdrive-host/src/ca/public_certified_key.rs` | certified-key-custody |
| S-PIG-48 | A lagged IntentStore subscription preserves the missed count and resumes with the next `Changed` | KEEP | I | @interruption @error @real-io @contract-shape:bounded-change | `overdrive-store-local` acceptance/edges | lag-aware-intent-subscription (complete body, reasoned ignore; the type/signature port is gated on item-12 reconfirmation) |
| S-PIG-49 | Frontend resolution closed errors; real persisted Job/Schedule rejected; intent universe preserved (uses `IntentKey::workload_prefix`) | ADAPT | U/I | @error @contract-shape:unbounded-preservation | `overdrive-gateway/src/frontend.rs` | gateway-application-owner |
| S-PIG-50 | Every `GatewayConnectDataplaneError` crosses its real owner op and collapses to the redacted `ConnectPathUnavailable` | ADAPT | I | @error @adapter-contract @contract-shape:bounded-change | gateway integration | upstream-socket-preparation |
| S-PIG-51 | Gateway identity lifecycle errors cause-distinct at the owner, collapsed to `GatewayIdentityUnusable` in status | ADAPT | I | @error @lifecycle @security @contract-shape:bounded-change | gateway integration | gateway-svid-lifecycle |
| S-PIG-53 | A failed cleanup is retained and retried without re-registering, reconnecting or re-authenticating | ADAPT | U | @error @degraded-resource @contract-shape:bounded-change | connector | upstream-socket-preparation |
| S-PIG-54 | **C4a restated (D2).** The unavailable-outcome path applied twice has an empty second delta (see below); custody replay and identity hold repeat keep coverage | ADAPT | I | @idempotency @contract-shape:bounded-change | gateway + control-plane integration | consuming-port-unavailable-outcome |
| S-PIG-55 | Route parsers accept exactly `target.service`; no `target.workload` alias | KEEP | U | @error @security @parser @contract-shape:pure-function | `overdrive-core` | route-value-types |
| S-PIG-56 | A stale Service-map entry after `job stop` never yields a wrong-peer success: the dial fails (P-9-independent invariant); exactly 502 is asserted only where `BACKEND_MAP.healthy == 1` is observed for the stale entry, otherwise the exact status is BLK-P9 (obl 3) | NEW | T3+N | @error @security @contract-shape:bounded-change | hook file + conformance file | consuming-port-unavailable-outcome |
| S-PIG-57 | Sentinel probe failure keeps TCP/443 unbound; serve and workloads unaffected; status cause and `health.startup.refused` reason `gateway.connect_probe` (obl 4, G-1) | NEW | I+T3 | @error @degraded-resource @contract-shape:bounded-change | control-plane integration boot/shutdown file + hook file | gateway-boot-and-shutdown |
| S-PIG-58 | Actual-frontend `NoBackend` is bind-valid; the listener binds and serves 503 (G-1) | NEW | I | @edge @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-59 | Gateway disabled (`ServerConfig.gateway == None`): boot unchanged, no socket prepared, no sentinel, unregistered hook behaviour unchanged (boundary 6); **expected-GREEN preservation row** | NEW | I | @config-disabled @contract-shape:unbounded-preservation | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-60 | Enabled boot orders G0–G4; shutdown stage `gateway` runs before `mtls_worker_owner`, inside one 5 s drain, one budget (G-6) | NEW | I (+N graceful run) | @lifecycle @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-61 | A gateway task failure or gated gateway never requests serve shutdown; the 10 s fail-stop bound is unchanged | NEW | I | @interruption @error @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-62 | Allocation `Running`, Service `Stable`, hydration and the operator listener advance identically with the gateway enabled, disabled or gated (boundary 3) | NEW | I | @orthogonality @contract-shape:unbounded-preservation | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-63 | A registered socket is selected by the hook; the rewritten dial enters leg-C via table 100 and TPROXY with `SO_MARK` and `127.0.0.1` source (obl 6) | NEW | T3 | @real-kernel @contract-shape:bounded-change | hook file | gateway-selection-hook, upstream-socket-preparation |
| S-PIG-64 | Exact-peer mTLS wire evidence on the leg-B/leg-C segment: TLS 1.3 records, zero cleartext, kTLS armed; matching, wrong-valid, expired, absent identity (obl 7, G-5) | NEW | T3 | @security @real-kernel @contract-shape:bounded-change | hook file | exact-peer-client-mtls |
| S-PIG-65 | Gateway SVID lifecycle: issue, audit-before-hold, near-expiry reissue, stale-epoch no-op, drop after drain, restart reissue (obl 9) | NEW | U+I | @lifecycle @security @contract-shape:bounded-change | reconcilers + control-plane action shim | gateway-svid-lifecycle |
| S-PIG-66 | Route/custody/application watch recovery: subscribe before list, relist on `Lagged`, admission closed meanwhile (obl 10) | NEW | I | @interruption @contract-shape:bounded-change | gateway integration | lag-aware-intent-subscription (complete body, reasoned ignore; type port gated on item 12; restated as relist-period if P-8 is adopted) |
| S-PIG-67 | BackendId identity readable only after the outer swap commits; immutable per id; rollback preserves the prior generation; no residue; restart destroys receipts (obl 11) | NEW | I+T3 | @lifecycle @contract-shape:bounded-change | dataplane host/Sim equivalence + source-local | selection-receipt-and-applied-identity |
| S-PIG-68 | A late `Selected` or probe result after a superseded generation is never delivered and never restores the old admission object (boundary 4) | NEW | I | @interruption @contract-shape:bounded-change | gateway integration | gateway-application-owner |
| S-PIG-69 | Guest-flow owner restart or fail-stop mid-request ends the request with 502 and leaves no intent or receipt (boundary 5) | NEW | I+N | @interruption @error @contract-shape:bounded-change | gateway integration + conformance file | public-request-end-to-end |
| S-PIG-70 | Public status mappings 400, 404, 421, 502, 503 end to end (obl 8) | NEW | N | @error @public-https @contract-shape:bounded-change | conformance file | public-request-end-to-end |
| S-PIG-71 | After a serve restart (dataplane instance recreated on the same pin directory) the gateway returns 503 until the ordinary hydrator re-programs the Service, then the backend response; old identity and receipts are gone (SR-2, obl 11) | NEW | T3+N | @lifecycle @interruption @contract-shape:bounded-change | hook file (instance half) + conformance file (serve restart) | selection-receipt-and-applied-identity, public-request-end-to-end |
| S-PIG-72 | Updates racing `selected_backend_identity` for a just-selected id never return a missing identity for a `Selected` id (run `receipt_waits_for_publish_guard`, hazard `publishEarly`; existing ports only) | NEW | T3 | @concurrency @contract-shape:bounded-change | hook file | selection-receipt-and-applied-identity |
| S-PIG-73 | A connector probe result completed twice for one staged token is ignored the second time; status row and application generation unchanged (GA-1) | NEW | U | @idempotency @error @contract-shape:bounded-change | `overdrive-gateway/src/application.rs` | gateway-application-owner |
| S-PIG-74 | Gate-unavailable boundary (boundary 2), four sub-cases: (a) absent sentinel receipt, (b) hook not firing, (c) identity never Current, (d) probe timeout each leave TCP/443 unbound with a cause-distinct status and `health.startup.refused` | NEW | I | @error @degraded-resource @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-75 | **Conditional (accepted-exposure candidate 5a).** Pre-commit rollback through the real owner: a `BACKEND_MAP` capacity failure leaves the prior generation selectable, no Applied identity for abandoned ids, no recycled ids, zero residue (`RollbackPreservesPriorGeneration`) | NEW | T3 | @error @lifecycle @contract-shape:bounded-change | hook file | selection-receipt-and-applied-identity (body authored only after the user acknowledges 5a and the fill runtime is measured) |

Conformance (QC): QC-1 `gateway_identity` (5 bound runs), QC-2 `gateway_application` (3), QC-3 `selection_receipt` (3 bound over `receipt_conf`) = 11 `#[quint_test]` plus one `#[quint_run]` per driver; see Scaffolds. EDD: E15 operator journey; point-in-time, never a test.

**Restated C4a — proof of the D2 replacement without fabricated demand.** There is no demand, revision, acknowledgment or wake to apply twice. The obligation is the consuming-port form (obligation 1): with no healthy backend, two consecutive public requests both return 503 with identical bodies; after each request `live_intent_count() == live_receipt_count() == 0`; `GatewayApplicationStatusRowV1` and the Gateway Applied Generation are unchanged; the outcome is a typed `NoBackend` bounded by the request deadline and never a wait. Proof: (i) Tier 1 (I): production `GatewayBuilder` over real redb Route and custody, `SimGatewayConnectDataplane` sharing the `SimDataplane` state with no `update_service` ever issued, so the receipt is the ordinary "no outer entry" `NoBackend`; a real public TLS client issues the request twice; the oracle reads only public bytes/status, the two live counts, `GatewayControl::application_status()` (before equals after), the unchanged injected `SimClock` (no sleep or poll) and the `gateway.upstream.receipt.validated` occurrence with `operation=request`; (ii) Tier 3/N: the same oracle against the real hook with a Service whose VM never reaches Running, so the real `ServiceMapHydrator` never programs a backend. No receipt is scripted and no row is seeded. The honest limit: the Sim arm is the modelled hook; only T3/N prove the real hook.

Walking-skeleton specification (S-PIG-01, `AT-PIG-E2E-1`):

    Given overdrive serve (the production composition) has a complete manual Public Certified Key
      and the operator has deployed the checked-in api VM Service and the public-api Route
    When an external user requests https://api.example.com/stream over TLS 1.3
      using the Route hostname as SNI and Host (client-local address override only)
    Then the user receives status 200, X-Workload api, and the streamed body
      and the receipt occurrence shows a Selected backend whose applied identity is the api allocation
      and the leg-B/leg-C segment carries TLS 1.3 records with no body cleartext
      and Route, Service, gateway intent, receipt and identity state converge on cleanup

## Wave: DISTILL / [REF] Cross-boundary scenario obligations

Per `.claude/rules/testing.md` ("DISTILL — prove composed system behavior under deliberate failure") each cross-boundary scenario records the production owner path, the driving stimulus, the fault (existing driven ports only; none for healthy controls), the oracle and the evidence lane. Single-owner scenarios are pure, source-local or single-owner and are not repeated here; their lane and contract are in the scenario list: S-PIG-03–10, 12–20 (pure or clap-only), 25 (one status projection read), 33 (one store key, reopen), 37 (pure ledger), 44, 45, 47 (pure codec), 49 and 53, 55, 73 (source-local or single owner).

**Oracle rules (apply to every row).**
1. *Membership-only on Sim.* A Sim-backed oracle on a generation with N>1 backends asserts only that the selected `BackendId` (and its applied identity) belongs to the committed generation; an exact `BackendId` is asserted only for singleton generations. The real Maglev choice is proven only at Tier 3.
2. `NoBackend` caused by a missing `BACKEND_MAP` entry (outer entry and slot present, backend id absent) is observable at Tier 3 only; the Sim cannot produce it from the `update_service` input.
3. The `healthy` byte is not evaluated by the Sim and no oracle depends on P-9 except where a row says so (S-PIG-56); the healthy window is not authored (BLK-P9).
4. The transient receipt oracle is the connector's `gateway.upstream.receipt.validated` occurrence plus `live_intent_count()`/`live_receipt_count()`, never map polling. Layers 3 and above are example-based.

| ID | Production owner path | Stimulus | Fault | Oracle | Lane |
|---|---|---|---|---|---|
| 01 | `run_server` (Host adapters) -> `ServiceMapHydrator` -> `EbpfDataplane::update_service` -> gateway runtime -> `open_upstream` -> hook -> leg-C -> kTLS -> `GuestStreamDial` -> vsock -> VM Service | External TLS 1.3 client requests `https://api.example.com/stream` after `deploy` of the Service and Route | none (healthy control) | 200, `X-Workload: api`, streamed body; receipt occurrence `Selected` with applied identity = api allocation; leg-B/leg-C segment carries TLS 1.3 records, zero body cleartext; after cleanup Route/Service/intent/receipt/identity state converged (live counts 0) | N |
| 02 | Public listener -> connector -> `open_upstream` -> hook/Sim arm -> receipt | Public request for a Route whose Service has no programmed backend | I: `update_service` never issued; N: Service VM never reaches Running | 503, no fallback body; I: `SimClock` unchanged (no sleep or poll); N: elapsed < `upstream_connect_deadline`; live counts 0; `NoBackend` occurrence with selected-only fields absent | I+N |
| 21-23 | `overdrive-gateway` Route Set owner -> `LocalIntentStore` (real redb) -> `GET /v1/gateway/status` projection | Declare, exact replay, replace (same Route ID, new generation), withdraw; declare a different Route ID while occupied (22); withdraw with a different Route ID (23) | none (healthy control and deterministic conflicts) | Outcomes `Declared`/`Unchanged`/`Replaced`/`Withdrawn` exactly; the persisted singleton slot holds the one Route (empty slot persisted, not deleted); conflicts return the cause-distinct typed slot-conflict error with no `RouteAccepted` fact and the whole universe (Route Set, Service, key, identity, BPF) equal; typed error kept apart from the redacted status projection | I |
| 24 | Two concurrent Declare commands for different Route IDs through the exported handler's Route Set owner over one real redb singleton key | Two simultaneous declares with distinct Route IDs, bounded rounds with the seed printed | none (race is the stimulus; no seam) | exactly one `Declared`, the other the typed slot conflict; the store holds exactly the winner's Route; exactly one `RouteDeclared`/`RouteAccepted` pair; no interleaved partial Route | I |
| 26 | `ServerHandle` shutdown stage `gateway` -> join users -> sweep -> slot clear | Graceful shutdown with one admitted connection | none | identity slot empty only after the user joined; live counts 0; stage events ordered | I |
| 27 | Boot `ManualCertifiedKeySource::probe` -> custody (`PublicCertifiedKeyAeadCodec`, protected redb record) -> `ServerHandle` boot | Boot with gateway flags naming unreadable (permission) and malformed (not PEM / mismatched key) material files | unreadable file and malformed content on the real filesystem (production-reachable inputs) | boot refused with `health.startup.refused` and a cause-distinct reason per case; the Route Set, custody record, identity and BPF state equal to pre-boot (no Route mutation); error text and status carry no path contents or key bytes | I |
| 28-30 | Connector -> `open_upstream` / `take_receipt` / `cleanup` at the dataplane port | Public request | Typed port results at those operations: receipt missing; receipt for another `ServiceKey`; `Selected` peer valid but not the selected one | 502; `cleanup` invoked once; one attempt only (no second backend); no body byte sent upstream | U/I |
| 34 | `deploy_resource` (CLI) -> `ApiClient` -> production `POST /v1/routes` handler -> Route Set store | `deploy <ROUTE_SPEC>` with and without detach | none (healthy control) | one-shot outcome identical for both detach modes (201 `Declared`, 200 `Replaced`/`Unchanged`); no streaming wait; Route Set holds the Route | I |
| 35 | `ApiClient` -> production `POST`/`DELETE /v1/routes` and `GET /v1/gateway/status` handlers -> store and `ObservationStore` | Declare, conflicting declare (409), invalid id (400), delete (200 `Withdrawn`/`Unchanged`), status read | none | exact HTTP statuses per case; typed `ApiClient` error kept apart from the redacted status body; Route Set and status row match the outcome | I |
| 36 | `deploy_resource` -> `ApiClient` -> handler of a composition with `ServerConfig.gateway == None` | `deploy <ROUTE_SPEC>` including an unparsable spec | gateway disabled by composition (not a seam) | rejected before the spec is parsed with the typed disabled-gateway error; Route Set untouched | I |
| 31 | Gateway application owner under seeded `overdrive-sim` | Route replace/withdraw interleaved with connection open/close and restart | `HoldConnectProbe` / `ReleaseConnectProbe`; restart | invariant `GatewayApplicationGenerationLifecycleIsSafe` holds; seed printed on failure | S |
| 32 | Same, shutdown | Shutdown with admitted users | `HoldConnectProbe` | invariant `GatewayApplicationShutdownConverges`; users joined before slot clear | S |
| 42 | `GET /v1/gateway/status` handler -> `ObservationStore` typed point reads (`public_certified_key_status_row`, `gateway_application_status_row`) -> `ApiClient`/CLI | Status request | point-read failure injected through `SimObservationStore` | HTTP 500 with a fixed redacted body; no key, path, ciphertext or error source text; CLI output carries the same redacted cause; subsequent healthy read returns the normal projection | I |
| 38, 43, 52 | `HostGatewayClientMtlsFactory` handshake against a real rustls peer | Gateway client dial to peer A | Selected A but peer B presents a valid SVID; absent/expired/malformed identity; non-peer fd; elapsed handshake; wrong URI-SAN shape | handshake error before any body byte reaches the peer; on success the peer observes the dedicated gateway SVID | I |
| 39, 40 | Public listener -> full production path | Request outside a narrow Route; request to a backend that closes before response | none; guest closes | 404 without upstream dial; 502 exactly once with the request-count marker = 1 | N |
| 41 | Real cgroup hook, unregistered socket | `connect(2)` from an unregistered socket | none | behaviour identical to main; `LOCAL_BACKEND_MAP` and XDP attachments unchanged (expected GREEN) | T3 |
| 46 | Boot custody ingestion (`ManualCertifiedKeySource` files -> `PublicCertifiedKeyAeadCodec` -> protected redb record) -> `GET /v1/gateway/status` | Install a key; replay the identical files; replace with a valid new generation; present an invalid replacement and an expired replacement; reopen | invalid/expired/stale-generation material on the real filesystem | `Installed`/`Unchanged`/`Replaced` exactly; invalid or expired replacement leaves the current generation and record intact and surfaces only a cause-distinct redacted install status; expiry derived from persisted inputs plus `SimClock`; reopen yields the same current generation | I |
| 48 | `LocalIntentStore::watch` (real redb) -> `IntentSubscriptionEvent` consumer | Subscribe, then force more committed deltas than the broadcast capacity while the consumer is held, then one further delta | broadcast lag (production-reachable overflow) | the consumer receives `Lagged` carrying the exact missed count, then the next `Changed`; complete body under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]` (the type/signature port is gated on item-12 reconfirmation) | I |
| 50 | Each dataplane port operation -> connector -> status | Public request or boot probe per operation | each `GatewayConnectDataplaneError` variant at its owning operation | status carries only redacted `ConnectPathUnavailable`; no variant text leaks | I |
| 51 | `GatewayIdentityView` / `ensure_current` / `disable_after_drain` -> status | Boot and drain | each identity lifecycle error | cause-distinct at the owner, `GatewayIdentityUnusable` in status | I |
| 54 | Public request path, twice (C4a) | Two consecutive public requests, no backend | none (no `update_service`) | both 503 with identical bodies; counts 0 after each; status row and Applied Generation unchanged; `SimClock` unchanged | I (+T3/N) |
| 56 | Real hook + nft output rule, stale map entry | Public request after `job stop` while the map entry lingers | none (production leaves the entry, P-4) | never a wrong-peer success, the dial fails; exact 502 only where `healthy == 1` is observed for the entry, else BLK-P9 | T3+N |
| 57 | Boot G0-G4 -> sentinel probe | Boot with enabled gateway | sentinel probe fault (T3: real hook absent before the arm lands is RED, not a seam) | TCP/443 unbound; serve and workloads unaffected; status cause; `health.startup.refused` reason `gateway.connect_probe` | I+T3 |
| 58 | Boot -> actual-frontend probe | Boot with Service present, no backend | none | listener binds; public request returns 503 | I |
| 59 | `run_server` with `ServerConfig.gateway == None` | Boot | none | bound sockets, reconcilers, intent/receipt counts and stage events equal the pre-feature set; unregistered hook behaviour unchanged (expected GREEN) | I |
| 60 | `ServerHandle` boot and shutdown | Enabled boot, graceful shutdown | none | order G0-G4; stage `gateway` before `mtls_worker_owner`; one 5 s drain; one budget | I (+N) |
| 61 | Gateway task failure -> serve lifetime | Fail the gateway task; gate the gateway | gateway task failure | serve shutdown not requested; 10 s fail-stop bound unchanged | I |
| 62 | Allocation, Service, hydration and operator listener | Boot enabled, disabled, gated | gated gateway | Running, Stable, hydration and operator listener advance identically in all three | I |
| 63 | Real hook -> table 100 -> TPROXY -> leg-C | Dial from a registered prepared socket | none | receipt `Selected`; rewritten dial enters leg-C with `SO_MARK` and `127.0.0.1` source | T3 |
| 64 | leg-B/leg-C segment, kTLS | Gateway dial through leg-C | matching, wrong-valid, expired and absent gateway identity | TLS 1.3 records, zero cleartext, kTLS armed (`ss -K`); wrong/expired/absent fail closed before body | T3 |
| 65 | `GatewaySvidLifecycle` -> action shim -> CA -> audit -> slot | Reconcile ticks over issue, near expiry, stale epoch, drop after drain, restart | stale-epoch completion; audit write fault | hold only after audit; stale epoch is a no-op; reissue on near expiry and restart; no allocation identity | U+I |
| 66 | Route/custody/application watchers over the intent store | Declare/withdraw while the watch lags | lag event | subscribe before list; relist on `Lagged`; admission closed meanwhile (ignored until DELIVER) | I |
| 67 | `EbpfDataplane` vs `SimDataplane` same call sequence | `update_service` sequences with success and rejection | conflicting backend set (real `BackendIdentityConflict`) | identity readable only after the outer swap; immutable per id; no residue; membership-only on N>1 | I+T3 |
| 68 | Gateway application owner | Supersede a generation while a probe/`Selected` is in flight | `HoldConnectProbe` then `ReleaseConnectProbe` | late result never delivered; old admission object never restored | I |
| 69 | Public request -> guest-flow owner | Request in flight | guest-flow owner restart or fail-stop | 502; no intent or receipt left (counts 0) | I+N |
| 70 | Public listener status mapping | 400 malformed, 404, 421 SNI/authority mismatch, 502, 503 requests | the corresponding upstream or input condition | exact status per case | N |
| 71 | `EbpfDataplane` re-creation on the same pin dir; `run_server` restart; ordinary hydrator | Serve restart, then public requests | process/instance restart (production path, no seam) | T3 deterministic: `selected_backend_identity(old id)` missing and receipt count 0 before any `update_service`; N: any response observed while the Service map shows no outer entry is 503 (safety), then the backend response within the bounded recovery (liveness) | T3+N |
| 72 | `Dataplane::update_service` racing `selected_backend_identity` on the real owner | Bounded rounds of concurrent updates and identity reads for a just-selected id | none (no pause seam) | for every id from a `Selected` receipt the read never returns missing; rounds and seed printed | T3 |
| 74 | Boot G0-G4 via `SimDataplaneComposer` | Boot with enabled gateway | (a) sentinel connect completes but `take_receipt` has no receipt; (b) hook not firing: registered probe yields no capability result; (c) `SimGatewayIdentityLifecycleControl` never reaches Current; (d) `HoldConnectProbe` with `SimClock` advanced past the probe deadline. Typed results at existing operations only | per case: TCP/443 refused (unbound); `GET /v1/gateway/status` shows the cause-distinct redacted cause; `health.startup.refused` with the reason; operator listener and allocation `Running` unaffected | I |
| 75 | Real `EbpfDataplane::update_service` to `BACKEND_MAP` capacity (65 536) | Fill via repeated production updates, then one more update | capacity exhaustion (production-reachable input) | typed rollback error; prior generation still selectable (singleton exact, else membership); no Applied identity for abandoned ids; fresh ids not recycled; counts 0 | T3 (conditional) |

If the Sim cannot distinguish sub-cases (a) and (b) of S-PIG-74 with the existing typed results at `probe` and `take_receipt`, that is a DESIGN gap to report, not a reason to add a scripting API.

## Wave: DISTILL / [REF] Strategy and project-policy mapping

Architecture-of-Reference treatment is inherited; no per-feature strategy. Driving surfaces use their real protocols (public TLS/HTTP, operator HTTPS API through the exported handler, clap parse). Driven-internal state uses `LocalIntentStore` over real redb, `SimObservationStore` for deterministic focused composition, and the real `EbpfDataplane` and kernel on native metal for Tier 3. External/non-deterministic ports (`Clock`, CA entropy/KEK, guest dial) use the existing Sim ports. Layers 3 and above are example-based; generative coverage is confined to pure and in-memory properties (S-PIG-03–10, 12–17, 44).

No scenario hand-installs a map, rule, identity, application snapshot, certificate or protected record that production installs. The only test-supplied credential is an operator input accepted by the production serve flags or `ServerConfig.gateway`. Faults enter only through existing driven ports (Sim dataplane/identity/mTLS typed faults, `HoldConnectProbe`/`ReleaseConnectProbe`, `SimObservationStore` faults, `SimClock`). `AT-PIG-E2E-1` and the hook tests run only on the native x86_64 KVM host via `cargo xtask metal run --`. Policy rows to append in phase 2: Driving: public HTTPS listener (conformance harness public client), `Route` operator surface; Driven internal: `GatewayConnectDataplane` (Sim pair in Tier 1/2, `EbpfDataplane` in Tier 3), custody over real redb; Driven external: `GatewayClientMtlsFactory` (Sim/Host).

## Wave: DISTILL / [REF] Adapter coverage

| Driven adapter / effect | Real-I/O or contract scenario | Coverage boundary |
|---|---|---|
| `LocalIntentStore` / Public Route Set | S-PIG-21–24, 27, 33, 48, 66 | Real redb deltas, persisted empty set, reopen, singleton claim, concurrency, exact lag count and recovery |
| `ManualCertifiedKeySource` and files | S-PIG-27, 46, 01 | Unreadable and malformed material; test-CA or operator-supplied chain |
| `PublicCertifiedKeyAeadCodec` / KEK / protected redb record | S-PIG-45–47 | Production codec and custody; 0/1/N chains, replace/expire, tamper/AAD/CSPRNG faults, secret-free status |
| `ObservationStore` gateway status and issuance audit | S-PIG-25, 35, 36, 42, 65 | Typed point reads, redacted JSON, 500 fault mapping, audit-before-hold |
| `ServiceFrontendResolve` | S-PIG-49, 01 | Service intent plus platform VIP resolves one frontend; closed failures preserve the intent universe |
| `GatewayConnectDataplane` / `EbpfDataplane` / cgroup hook and maps | S-PIG-02, 28–30, 37, 41, 50, 53, 56, 57, 63, 67, 71, 72, 75 | Tier 1 uses `SimGatewayConnectDataplane`; Tier 3 uses the real hook, `open_upstream` preparation (mark, loopback bind, cookie, registration), receipts and cleanup. The transient receipt oracle is the connector's `gateway.upstream.receipt.validated` occurrence, not map polling |
| Ordinary `ServiceMapHydrator` -> `update_service` (D1) | S-PIG-01, 56, 67 | Real hydrator programs the VM backend; the gateway adds no mechanism |
| `HostGatewayClientMtls` / rustls / leg-C / kTLS / vsock | S-PIG-38, 43, 52, 64, 01 | Exact selected peer, dedicated gateway SVID, wrong-valid and absent/expired identity fail before body; wire evidence on the leg-B/leg-C segment |
| Gateway identity slot, issue-and-audit, reconciler | S-PIG-51, 65, 26 | Dedicated non-allocation subject, epoch idempotency, stale no-op, drain-ordered empty |
| Public HTTP runtime (hyper) | S-PIG-12–17, 39, 40, 70 | 200/400/404/421/502/503, authority/header/limit policy, single attempt |
| `ServerHandle` / `GatewayHandle` boot and shutdown (D5) | S-PIG-26, 57–62, 69, 74 | G0–G4 order, stage `gateway`, one budget, fail-stop interaction, disabled path |

Public DNS is not an Overdrive adapter. Production reachability needs an operator-managed IPv4 A record for the Route's Public Hostname. Hermetic runs and E15 use only `curl --resolve <hostname>:443:<gateway-ipv4>` (client-local address selection; the hostname remains SNI, Host and certificate identity); no step edits `/etc/hosts` or uses ambient DNS.

## Wave: DISTILL / [REF] Scaffolds

Only shapes the DESIGN pins are scaffolded; bodies of production scaffolds fail with SCAFFOLD text under file-level lint allowances. Every test body is complete and executable, including every KEEP unit body (S-PIG-03–10, 12–20, 44, 45, 47, 55 alike), each carrying a reasoned `#[ignore = "pending DELIVER <step>"]`. Activation: remove only the reasoned `#[ignore]`, observe a semantic RED, implement, GREEN, commit (testing.md "RED acceptance scaffolds and activation").

| Scaffold (crate / file) | DESIGN anchor | Notes |
|---|---|---|
| `overdrive-core`: `public_ingress.rs`, `gateway_identity.rs`, `SpiffeId::for_gateway`, two `ObservationRow` variants and point reads (previously approved 2026-09-13), `Action::{IssueGatewaySvid,DropGatewaySvid}`, `HydrationContext.gateway_identity` | Persistence, custody and redaction; Gateway identity; D5 items 9–10 | Demand types, `gateway_demand`, `gateway_frontend_demand` are not carried |
| `overdrive-core` + `overdrive-store-local` + implementer fallout: **`IntentSubscriptionEvent` type and the `IntentStore::watch` return-type change only** | D5 item 12 | Compile prerequisite only: `LocalIntentStore` maps every broadcast event to `Changed` and detects no lag (main's silent drop on lag is unchanged); lag behaviour is DELIVER's. Gated on the user's reconfirmation of item 12 and on P-8 not being adopted |
| `overdrive-host`: `ca/public_certified_key.rs`, `socket_cookie.rs` | Persistence; component decomposition | |
| `overdrive-gateway` (new crate): ports (`GatewayConnectDataplane` with `open_upstream`/`PreparedUpstream`, `GatewayClientMtlsFactory`), application, connector, certified key, route set, frontend, runtime, `UnboundGatewayBuilder`/`GatewayBuilder`/`GatewayControl`/`GatewayHandle` | D5 (a)(b)(d), Application sections | Needs D5 confirmation |
| `overdrive-bpf`: `maps/gateway_connect.rs` (two 8-byte POD maps, capacity 128, unpinned) | Backend programming and selection (D3) | **No selection-arm program is scaffolded** — placement is P-10 |
| `overdrive-dataplane`: `gateway_connect.rs`, `mtls/gateway_identity.rs` (slot), `mtls/gateway_client.rs`, commit-guard types, `DataplaneError` variants | Commit-guarded transaction; identity integration; obligation 15 | Allocator internals beyond `existing_id` untouched (P-2) |
| `overdrive-reconcilers`: `gateway_svid_lifecycle.rs` + `Any*` arms | Closed-dispatch contract; D5 item 9 | Host-backed wakeup (empty `interests()`, `resync_schedule()`) |
| `overdrive-control-plane`: `BootAdapters.gateway`, `ComposedDataplane.gateway_connect`, composition, action shim, boot G0–G4, `ServerHandle` stage `gateway`, handlers, regenerated `api/openapi.yaml` | D5 | Dispatcher fallout across the control-plane tests |
| `overdrive-cli`: four `ServeArgs.gateway_*` fields, Route deploy dispatch, `ApiClient` calls | D5 item 11 | Constructor fallout across the CLI tests |
| `overdrive-sim`: `adapters/gateway.rs`, `invariants/gateway_application.rs`, two `Invariant` variants outside `Invariant::ALL` | Exact seeded invariant registration | Activation key `DELIVER-PIG-GATEWAY-APPLICATION-INVARIANTS` |
| `tests/conformance`: end-to-end file and harness helpers; QC-1..3 drivers | Evidence-lane table; Formal protocol model | See drivers below |
| `examples/public-ingress-gateway/` and `verification/expectations/E15-public-ingress-operator-journey/` | Artifact-boundary handoff | See below |

**Lag-aware watch decision.** DISTILL ports only what no body can compile without: the `IntentSubscriptionEvent` (`Changed | Lagged`) type, the `IntentStore::watch` return-type change and its implementer fallout (`LocalIntentStore`, `FaultInjectingIntentStore`, `CountingIntentStore`, Sim). `LocalIntentStore` maps every broadcast event to `Changed` and implements no lag detection, so main's silent-drop-on-lag behaviour is unchanged. S-PIG-48 and S-PIG-66 stay complete executable bodies under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]`; reporting `Lagged` with the exact missed count, relist on `Lagged` and admission closed meanwhile are DELIVER's. This remains gated on the user's reconfirmation of D5 item 12 (D-APP-10 of the DESIGN approved 2026-09-13; `IntentStore` unchanged on main since the merge base). If the user adopts the P-8 simplification, S-PIG-48 is dropped, S-PIG-66 is restated as a relist-period test and nothing is ported.

**quint-connect drivers** (spec referenced in place at `specs/quint/public-ingress-gateway/`; each driver module is `#[cfg(all(test, feature = "integration-tests"))]` in the owner's crate; `#[quint_test(max_samples = 1)]` per bound run and one `#[quint_run]`; every action handler calls the pinned driving port; `State` projects only the pinned accessors; no always-false cfg):

| Driver | Module / conformance main | Owner and placement | Runs bound to `#[quint_test]` | Projection (pinned accessors) | Ignore reason |
|---|---|---|---|---|---|
| QC-1 | `gateway_identity` / `identity_conf` | `overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs` (D-PASS7-1: in-crate child module; `dispatch_issue`/`dispatch_drop` stay `pub(crate)`) | `issue_hold`, `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue` (5) | `GatewayIdentitySlot::{desired,current}`, `ObservationStore::issued_certificate_rows` filtered to the gateway SPIFFE id | `pending DELIVER quint-conformance-gateway_identity` |
| QC-2 | `gateway_application` / `app_conf` | `overdrive-gateway/src` | `replace_valid`, `route_withdraw_drain`, `late_probe_after_supersession` (3) | `GatewayControl::application_status()` | `pending DELIVER quint-conformance-gateway_application` |
| QC-3 | `selection_receipt` / `receipt_conf` | `overdrive-dataplane/src/quint_selection_receipt.rs`; owner = the real `EbpfDataplane` on native metal with the real `cgroup/connect4` selection arm (D-PASS7-2); no `Sim*` owner or projection source; run through `cargo xtask metal run --` | `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` (3, over `receipt_conf`) | `selected_backend_identity`, `live_intent_count`, `live_receipt_count`, `take_receipt` return | `pending DELIVER quint-conformance-selection_receipt` until the P-10 arm and the owner are GREEN |

Total `#[quint_test]` = 5 + 3 + 3 = 11, plus one `#[quint_run]` per driver (3). Model-only runs not bound: `abandoned_reservation_rolled_back` and `receipt_waits_for_publish_guard` run on `receipt_fine` and are covered by Tier-3 tests through existing ports (S-PIG-75 conditional, S-PIG-72). The host/Sim equivalence test (S-PIG-67) is the Sim pairing, not conformance.

`unhealthy_is_no_backend` is not bound (P-9). `cargo xtask conformance-lint` must pass.

**Examples.** `examples/public-ingress-gateway/` is a VM Service journey in the style of `examples/service-kind-vm-workloads/`: `service.toml` (`[vm]`, never `[exec]`), `prepare.sh`, an example-local guest HTTP/1.1 server (`/stream`, `X-Workload: api`, request-count marker), `prepare-test-ca.sh`, `route.toml.template`, `route-narrow.toml.template`, `README.md`, and `run-example.sh` with the exact argv contract in "Artifact-boundary handoff" (hermetic mode curls with `--cacert`; production-trust mode with system trust; both only via `curl --resolve`). The pre-reconstruction shell runner, regression scripts, `receipt-oracle.py`, `workload.py` and the GitHub workflow are not carried.

**E15.** `verification/expectations/E15-public-ingress-operator-journey/{README.md,runner.sh}` plus the INDEX row (status `pending`) and the harness `EXPECTATION_SOURCE_SHA`/`EXPECTATION_SOURCE_DIRTY` capture pinned in the Artifact-boundary handoff. Point-in-time capture against the built default-feature binary on native metal with a clean harness-captured HEAD, exact-object BPF embedding proof, `curl --resolve` and normal system trust; reviewed by a different agent; not a test.

## Wave: DISTILL / [REF] Test placement

Pure Route and value properties are source-local in `overdrive-core` (public signatures are the driving ports). Authority, header, limit and connector properties are source-local in `overdrive-gateway` (DESIGN keeps them crate-private). CLI parsing uses the existing `overdrive-cli` acceptance entrypoint. Real redb and application composition use the `integration-tests`-gated `overdrive-gateway` integration binary. Boot, shutdown and operator-surface composition use `overdrive-control-plane` and `overdrive-cli` integration binaries through `SimDataplaneComposer` (`NodeAdapters` swap; no `*_override`). Real-kernel hook, packet-entry and exact-peer wire tests use the `overdrive-dataplane` integration binary on native metal. The cross-owner production walking skeleton and its native cases use `tests/conformance` (`native_metal_kvm_public_ingress_gateway_*`, selected by `tests/conformance/run-native-metal.sh`). Ordering, late completion, restart and shutdown use the `overdrive-sim` catalogue. Quint drivers sit in their owners' `src/`. No test spawns the `overdrive` binary; only E15 drives the built CLI.

## Wave: DISTILL / [REF] Driving-adapter coverage

| Driving surface | Scenario / mechanism |
|---|---|
| `overdrive serve` four gateway flags | S-PIG-18–20 (clap and config, in-process); S-PIG-59/60 through `ServerConfig.gateway` (the field `serve` fills); E15 through the built binary |
| `overdrive deploy <SERVICE_SPEC>` (VM Service) | S-PIG-01 via the exported handler's public API; E15 via the example |
| `overdrive deploy <ROUTE_SPEC>` | S-PIG-34/35 (`deploy_resource` and `ApiClient` against the production handler); materialized checked-in templates in S-PIG-01/39 |
| `DELETE /v1/routes/{id}`, `GET /v1/gateway/status` | S-PIG-21, 23, 25, 33, 35, 36, 42 |
| Public TLS 1.3 / HTTP/1.1 listener | S-PIG-01, 02, 39, 40, 56, 69, 70; E15 |
| Serve shutdown and fail-stop | S-PIG-26, 60, 61, 32 |

## Wave: DISTILL / [REF] Prerequisites

- Native x86_64 bare-metal KVM host provisioned per `infra/metal`, uid 0, fail-closed on the substrate probes (`.claude/rules/testing.md`). No Lima instance and no CI metal lane (P-6 is PROPOSED). Real-kernel and VM scenarios run through `cargo xtask metal run --`; the default and `integration-tests` lanes compile and run the in-process scenarios.
- BPF object produced by `cargo xtask bpf-build` before anything that compiles `overdrive-dataplane`; the selection arm does not exist until P-10 is ruled and DELIVER lands it, so hook scenarios are RED for the right reason.
- Existing production owners compose the rest: workload-identity CA, Service deploy, ordinary `ServiceMapHydrator`, leg-C -> kTLS -> `GuestStreamDial`, the node-shared nft owner. Tests do not substitute their effects.
- Operator inputs: a complete leaf-first chain and matching PKCS#8 key (E15), or the example's bounded test CA (hermetic mode). Tools for independent oracles: `bpftool`, `ss`, `nft`, `curl`, `openssl`.
- User confirmations (**all phase-2 code batches B1–B5 wait for them**): D5 "New public surface" items 1–11 (gate batches 2–5), item 12 (gates the watch type/signature port and its fallout), and the blocking P-2, P-9 and P-10 (design approval). The observation point reads and row variants are previously approved (2026-09-13) and not on this list.

## Wave: DISTILL / [REF] Outcome registration

No new outcome row is introduced. The scenarios refine the registered contracts `OUT-PIG-CERTIFIED-KEY-CUSTODY`, `OUT-PIG-GATEWAY-APPLICATION`, `OUT-PIG-BPF-SELECTION-IDENTITY`, `OUT-PIG-GATEWAY-EXACT-PEER`, `OUT-PIG-PUBLIC-REQUEST`, `OUT-CA-SVID`, `OUT-WIM-SVID-LIFECYCLE`, `OUT-MTLS-HANDSHAKE-FAIL-CLOSED`, `OUT-MTLS-COMPOSED-PROXY-SKELETON`, `OUT-MTLS-INBOUND-SERVER-MTLS` and `OUT-MTLS-WIRE-TLS13`. Phase 2 restates the registry's demand/revision/acknowledgment text under D2 and re-runs `nwave-ai outcomes check-delta` (nonzero set, 0 collisions required).

## Wave: DISTILL / [REF] Completeness audit

Provisional and specification-level: authored bodies do not exist yet, so no check is credited as PASS until phase 2 writes and compiles them. Nothing below is waived.

| Check | Status | Evidence or gap |
|---|---|---|
| C1a empty/zero/minimum | Specified | All-absent config, empty withdrawal, zero-chain, empty set, no-outer-entry `NoBackend` |
| C1b partition boundaries | Specified | Labels 62/63/64, epochs, chain 0/1/N/9, ledger 0/1/128/129, limits |
| C2a state machine documented | Specified | Gate G-1..G-6 and Quint modules |
| C2b illegal event per state | Specified | Stale epoch, competing claim, wrong-id withdrawal, late probe, duplicate probe completion (S-PIG-73), restart |
| C3 zero/one/many | Specified | S-PIG-45, header PBT, ledger, Route 0/1/N |
| C4a apply twice | **Open until the S-PIG-54 body is authored** | Restated as the consuming-port form above; replaces the retired demand gap; not waived |
| C4b inverse without prerequisite | Specified | S-PIG-33 |
| C5a mode combinations | Specified | 16 enablement masks, hermetic vs production-trust, exact vs prefix Route |
| C5b orthogonality | Specified | S-PIG-34 (both detach modes), S-PIG-62 |
| C6a malformed parameters | Specified | Route fields, `target.service`, authority, path, hostname, argv, key files |
| C6b every declared error | Specified, **needs re-pin** | S-PIG-50/51 matrices must follow the new `GatewayConnectDataplaneError` variants and `ensure_current`/`disable_after_drain` |
| C6c closed error set | Specified | Exhaustive classifier per set; status projection exhaustive |
| C7a degraded resource | Specified | Unreadable key, CSPRNG, observation fault, NoBackend, cleanup failure, full ledger, boundary-2 causes (S-PIG-74) |
| C7b interruption | Specified | Lag, hold/release probe, withdrawal, restart, mid-request flow-owner restart, serve restart (S-PIG-71) |
| C7c concurrent actors | Specified | S-PIG-24 |

Documented gaps and blockers (no silent waivers):
1. **BLK-P9** — obligation 2: the in-window result (503 vs 502) and the `unhealthy_is_no_backend` run are not authored; the post-`healthy` 0 -> 1 leg is covered by S-PIG-02/56/01 once P-9 is ruled.
2. **BLK-P2** — the BackendId memo re-lease wedge exposure is not authored; `conflicting_alloc_rejected` (rebind rejected) is covered for every option.
3. **BLK-P10** — verifier-regression, `perf-baseline` and attach-mode gates are not authored; S-PIG-41/63 are placement-neutral.
4. **Item 12** — lag-aware watch reconfirmation gates the `IntentSubscriptionEvent` type and `watch` signature port (and its implementer fallout); S-PIG-48/66 are complete reasoned-ignored bodies and the lag behaviour is DELIVER's.
5. **DESIGN approval gates every code batch** — D5 items 1–11, item 12, and the blocking P-2/P-9/P-10 gate all phase-2 batches B1–B5.
5a. **Accepted-exposure candidate (user acknowledgement required)** — the pre-commit rollback real-owner Tier-3 check (S-PIG-75; specified, body authored only after acknowledgement and a measured fill runtime): the only production-reachable pre-commit failure input is `BACKEND_MAP` capacity (`crates/overdrive-bpf/src/maps/backend_map.rs:46`, `MAX_ENTRIES = 65_536`); the fill's native-metal runtime is unmeasured. If not run, `RollbackPreservesPriorGeneration` has model + equivalence/unit coverage only. No seam is added.
6. **DESIGN gaps:** none open that block authoring. The former U1 (QC-1 placement), U2 (QC-3 owner) and U4 (Sim arm contract) are resolved by D-PASS7-1, D-PASS7-2 and D-PASS7-4 and are not gaps. Notes only: U7 stale `unhealthy_is_no_backend` mentions and a Lima remark in the Obligations list (documentation drift, no effect on scenarios); U8 `kvm-tests` named for `tests/conformance`, which has no such feature (DISTILL follows the crate's `native_metal_kvm_` selector). Conditional: if the Sim cannot distinguish S-PIG-74 sub-cases (a) and (b) with the existing typed results, report it to DESIGN.
7. **Real-guest evidence** (S-PIG-01, 39, 40, 56, 69, 70, 71, 60 graceful run, A-K3's serve-created connect) exists only on native metal; no recurring CI lane is added (P-6).

Mandate evidence plan: every executable scenario carries a Contract Shape; state-delta assertions name public Route/status/receipt/cleanup surfaces, not private owner fields; PBT is confined to pure/in-memory layers; Tier 3 is example-based; Rust domain nouns reuse the exact DESIGN types. Adapted step-reuse ratio is informational and is measured after phase 2.

## Wave: DISTILL / [REF] Pre-DELIVER RED classification

**Not executed.** Phase 1 wrote no code. The classification below states what phase 2 must demonstrate before handoff, per scenario family; it is not a result.

- Pure and in-memory scenarios (KEEP/ADAPT in `overdrive-core`, `overdrive-gateway`, `overdrive-cli`): compile green with the reasoned ignore; once un-ignored, RED must be `MISSING_FUNCTIONALITY` at a production scaffold call (SCAFFOLD text), never an import, fixture or setup failure.
- **Expected-GREEN preservation rows:** S-PIG-41 (unregistered socket on the current hook) and S-PIG-59 (gateway disabled). They are not MISSING_FUNCTIONALITY candidates: they must pass on first run once their scaffolds compile (41 on native metal only); a RED here is a scaffold-wiring regression and blocks handoff.
- S-PIG-56, 57 (T3), 63, 64, 71, 72, 75: RED because the selection arm program does not exist (P-10 unruled) or `open_upstream` is a scaffold; the first assertion is the missing `NoBackend`/`Selected` receipt on a registered socket, not a harness error. If P-10 is not ruled the family stays blocked and unclassified.
- `AT-PIG-E2E-1` and N cases: RED at the first missing production behaviour on native metal (`open_upstream` scaffold), classified only after a metal run.
- Quint drivers: compile green under `integration-tests`; ignored with `pending DELIVER quint-conformance-<module>` (`selection_receipt` until the P-10 arm and the real-`EbpfDataplane` owner are GREEN); `conformance-lint` clean.
- S-PIG-48 and S-PIG-66: complete bodies under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]`; no body is active. Once un-ignored, RED must be the missing `Lagged` event and missed count (MISSING_FUNCTIONALITY), because the ported `LocalIntentStore` reports every event as `Changed`.
- Phase 2 records one classification line per failing scenario in this section (no separate file); any `IMPORT_ERROR`, `FIXTURE_BROKEN` or `OBSERVABLE_NOT_AT_PORT` blocks handoff.
