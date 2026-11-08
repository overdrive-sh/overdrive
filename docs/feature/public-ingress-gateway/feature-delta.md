# Feature Delta — `public-ingress-gateway`

> **DESIGN status (reconstruction on `main` 549d78fa6): rulings R-1..R-19 (user, 2026-11-02) and R-20..R-44
> (user, 2026-11-03; the former proposals P-20..P-31 and the scope items that accompanied them) applied; pending
> independent re-review.** Nothing is marked accepted by review; recorded model-check evidence is re-recorded by
> `quint check --record`. The public data path is redesigned by G1: userspace rustls handshakes, kernel TLS
> record layer, gateway-parsed heads, bodies spliced between the two kTLS sockets
> ([Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1); ADR-0244, 0245, 0239, 0246, 0247 and 0249-0254).
> The user ruled on 2026-11-02 (rulings R-1..R-19 in
> [User decisions](#wave-design--ref-reconstruction-user-decisions-and-proposals)): P-2, P-9, P-10, B-1, DG-11, DG-12,
> QC-1..QC-3, P-12, P-13a-d, P-17, P-18, the upstream-leg and Q-1 rulings, the D5 confirmation list (items 1-13) and
> `relay_ktls` removal are accepted contract text below; P-16 is not adopted. R-20..R-44 (2026-11-03) are accepted
> contract text too. R-45..R-52 (SVID issuance failure occurrence for the gateway and workload SVIDs, 2026-11-04) are recorded
> contract text with the provenance stated per row (R-45, R-46 user decisions; R-47..R-50 user decisions delegated as "prefer
> correctness"; R-51 an application of the user's versioned-envelope instruction; R-52 user-confirmed 2026-11-04). ADR-0274 and ADR-0275 record the two occurrence decisions and are Proposed, pending
> independent review. The optional proposals (P-1, P-3, P-4, P-6, P-8 simplification, P-32, P-33, P-34, P-35, P-36, P-40) remain PROPOSED and await the user. P-37 was rejected by the user on 2026-11-04. P-38 (`ApiClient` route DELETE and gateway status methods) was ADOPTED by the user on 2026-11-04 (R-53); the hook tests and small gaps ruled the same day are R-54. The user rulings of 2026-11-05 are R-55 (P-41, the per-connect measurement) and the confirmations recorded in its row, and R-56 (the store-emitted skipped-row warning); R-57 and R-58 are user rulings of 2026-11-05 and R-59, R-60 and R-61 (the Service-map reverse-NAT conflict, the rollback restore failure and the cleanup-debt retry) are user rulings of 2026-11-06; the ruled contract text is R-1..R-61.
> Contract-shaping validation V-GW-1..5 and the G1 spike (a-e) have
> run (see [Contract-shaping validation items](#wave-design--ref-contract-shaping-validation-items)
> and [Formal protocol model](#wave-design--ref-formal-protocol-model)). This document reconstructs the previously
> user-ratified (2026-09-13) `public-ingress-gateway` DESIGN on current `main`.
> Unchanged decisions keep their 2026-09-13 ratification provenance; the
> decisions changed by the user rulings D1–D5 of 2026-11-01 (table in
> [User decisions](#wave-design--ref-reconstruction-user-decisions-and-proposals))
> are recorded as the current decisions; rulings R-1..R-19 (user, 2026-11-02) are applied and the text is pending
> independent re-review.
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
> ADR-0222/0226/0227/0231–0240, plus ADR-0242, ADR-0243 and ADR-0248, pin the downstream
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
| **Current feature intent** | Issue #54 plus user direction: node-agent-embedded gateway (issue text: `hyper` + `rustls`; the `hyper` data path is replaced by user ruling G1, 2026-11-02, with rustls handshakes, kernel TLS and spliced bodies), in-process BPF map access, public north-south ingress, operator-supplied production Web-PKI certified key, gateway SVID on the internal hop, and no second userspace backend load balancer. |
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
| **A. Registered arm in a separate cgroup/connect4 program beside the existing Service hook (selected; R-1)** | The dataplane adapter creates a marked, loopback-bound socket in the `overdrive serve` task, reads its stable kernel cookie and registers `(cookie, exact ServiceKey)`. The connector's `connect(2)` enters the attached gateway selection program (beside `cgroup_connect4_service`); it indexes the existing `SERVICE_MAP` Maglev inner table by a cookie-derived slot, resolves `BACKEND_MAP`, rewrites the destination, and writes a cookie-keyed selected/`NoBackend` outcome. The rewritten dial enters leg-C by the ADR-0190 route. | Reuses the attached hook and the ordinary Service-map programming D4 proved. The intent entry makes the gateway arm explicit and fail-closed while every unregistered connect retains today's `LOCAL_BACKEND_MAP` behavior. | Adds a second attached program, transient intent/receipt maps (`BACKEND_MAP.healthy` is not read, R-2), and a Dataplane-owned applied-identity association; no cgroup, netns, veth, process, or userspace LB. | **Selected** |
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
  The gateway selection program (a separate `cgroup/connect4` program attached
  beside `cgroup_connect4_service`, R-1) checks the registered intent,
  indexes the existing `SERVICE_MAP` Maglev inner table using a deterministic
  hash of that cookie, resolves `BACKEND_MAP`, rewrites to the selected VM
  backend address, and writes the selected `BackendId` outcome before allowing
  the connect. `NoBackend` (no outer entry or no usable backend; the
  `healthy` byte is not read, R-2) is
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
  HTTP/2 (and gRPC) on the public listener is the tracked follow-up
  [#341](https://github.com/overdrive-sh/overdrive/issues/341).
- **D-SYS-12 — Data path (G1):** both legs run TLS 1.3 handshakes in userspace
  (rustls, unbuffered API) and hand the record layer to the kernel (kTLS TX+RX,
  [ADR-0244](../../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md));
  the gateway parses only message heads and framing lines
  ([ADR-0245](../../product/architecture/adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)) and bodies
  move between the two kTLS sockets by `splice(2)`
  ([ADR-0239](../../product/architecture/adr-0239-stream-public-proxy-bodies-with-backpressure.md),
  [ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md)); the exchange
  is full-duplex ([ADR-0247](../../product/architecture/adr-0247-gateway-exchange-is-full-duplex.md)).
  The claim is bounded memory and no userspace payload copy; no CPU gain is claimed. Backend selection (D-SYS-6) is unchanged
  and still happens at `connect(2)`, before either handshake.
- **D-SYS-10 — Availability/status:** Route miss is 404; BPF selection receipt
  `NoBackend`—an empty eligible set or no selectable BPF backend (a selected
  backend's `healthy` byte is not read, R-2)—is the only 503 path and the typed
  outcome of the gateway's consuming port, with no readiness wait
  ([ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)). A
  `Selected` receipt whose applied identity is missing or mismatched, and every
  internal connect/receipt/mTLS/upstream failure, is 502. `GatewayStatusReads`
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
  probe treats both `NoBackend` and `Selected` as successful results: it proves
  only that the frontend resolves through the registered connect path, performs
  no identity lookup and no mTLS, and the Gateway Application is then atomically
  promoted and TCP/443 binds (R-65). The selected backend's exact-peer mTLS is a
  per-request outcome (502), never a bind or promotion condition.
  Shutdown runs the gateway stage first in `ServerHandle::shutdown`: stop
  accepts, drain admitted requests until the gateway drain deadline (the shutdown deadline minus the join reserve, R-64), join
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
`NoBackend` without identity use; the actual-frontend edge, only after Gateway
Identity Current, ends at the validated receipt (`NoBackend` or `Selected`) and
never continues into exact-peer mTLS (R-65).

## Wave: DESIGN / [REF] Source-of-truth and trust boundaries

| State/fact | Source of truth | Owner | Explicit non-owner/non-use |
|---|---|---|---|
| Gateway enablement and public bind | `ServerConfig.gateway: Option<GatewayConfig>` plus live socket fact | `serve` composition root / `GatewayApplicationOwner` | All-or-none operator config; bind occurs only after application/dataplane/identity gates |
| Public Route Set | `IntentStore` `PublicRouteSetV1` | `PublicRouteSetOwner` | Service `[[listener]]` is backend intent, not public exposure intent; no backend candidate/BPF state |
| Public Certified Key current custody generation | `IntentStore` `PublicCertifiedKeyV1`, including `ProtectedOriginKeyV1` sealed by `PublicCertifiedKeyAeadCodec` | `PublicCertifiedKeyCustody` is sole writer/validity owner | `ManualCertifiedKeySource` paths/raw PEM stop before custody; public-key secrets never enter status, Route, handler, CLI or logs |
| Public Certified Key status | `ObservationStore` table `observation_public_certified_key_status` / `PublicCertifiedKeyStatusRowV1` | `PublicCertifiedKeyCustody` sole LWW writer after Current publication | Redacted active/usability/install-failure facts only; status failure retries without rolling back Current; no paths, PEM, DER, key or ciphertext |
| Gateway SVID | Existing internal CA + issued-certificate audit plus volatile `GatewayIdentitySlot` | `GatewaySvidLifecycle` + control-plane action executor + `GatewayIdentitySlot` | Separate from allocation `IdentityMgr` and Public Certified Key; only `HostGatewayClientMtls` may use private material; no fabricated AllocationId |
| Eligible workload backend set and `Backend.alloc` | `ServiceBackendRow` | ServiceLifecycle, sole complete-row publisher | Gateway does not read, probe, author, filter or select this set |
| Applied Service frontend backend set | `SERVICE_MAP` outer + Maglev inner table + `BACKEND_MAP` | ServiceMapHydrator → Dataplane/EbpfDataplane (ordinary path, D1) | The gateway reads it only through the hook's registered arm; handler never mutates maps |
| Gateway Applied Generation | Active `GatewayApplicationOwner` Current `ArcSwapOption`, built from Route + exact Service Frontend + opaque certified-key snapshot | `GatewayApplicationOwner` | Atomic admission pointer retained per connection; excludes BPF generation/candidates and is not reconstructed from status |
| Gateway Application status | `ObservationStore.gateway_application_status` / `GatewayApplicationStatusRowV1` keyed by `NodeId` | `GatewayApplicationOwner` sole LWW writer | Listener/staged/current/draining/unavailable/non-secret identity/connect-path facts and the SVID issue-failure occurrence record (count, cause class, instant; R-45) only; no BackendId, receipt, peer, BPF generation, credential secret or CA/audit error text |
| Upstream socket preparation and registered intent | Process-local `GATEWAY_CONNECT_INTENT_MAP` entry, keyed by the socket cookie | `EbpfDataplane` (`open_upstream`) | Never the gateway crate; never persisted; destroyed with the BPF object |
| Operator gateway-status response | `GatewayStatusReads` plus exact ObservationStore point reads for application/custody and relevant Service hydration | Existing operator-mTLS `GET /v1/gateway/status` handler | `GatewayStatusResponse`; 200 for readable active/disabled state, 500 on observation read error; raw dataplane failure strings and secrets omitted |
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
  candidate valid, `GatewayRelay::probe` proved at this boot that the node's kernel
  image can arm kTLS, hold a pipe of at least 64 KiB and splice exactly (no SVID or
  network peer involved; it does not re-run per staged generation), `gateway_connect.probe` (residue sweep plus the sentinel
  canonical `NoBackend` receipt) proved the registered-connect/hook/receipt
  capability, and Gateway Identity is Current. The actual Service Frontend
  probe then returned a validated receipt, either `ReadyNoBackend` or
  `ReadySelected`, both bind-valid; TCP/443 may now bind. Route activation does
  not depend on backend health or backend authentication (R-65): backend
  presence, the selected backend's identity resolution and its exact-peer mTLS
  are not listener prerequisites and the gate waits for none.
- **Affected state:** public listener bound/admission eligibility only.
- **Failure projection:** a failed `GatewayRelay::probe` (typed `RelayProbeError`,
  event reason `gateway.relay_probe`, status `ConnectPathUnavailable`); missing/malformed sentinel or actual-frontend receipt,
  probe timeout or connect-path failure keeps TCP/443 unbound and is
  cause-distinct on operator-mTLS gateway status, with a structured
  `health.startup.refused` event naming the gateway reason; malformed or
  duplicate, cancelled and disconnected outcomes map to the same unbound state.
  Every failure here is gateway-local. Actual-frontend `NoBackend` or `Selected`
  never blocks bind: the active listener serves 503 for `NoBackend`, and a
  `Selected` backend whose applied identity cannot be resolved, or whose
  exact-peer mTLS fails, is a per-request upstream failure (502, no fallback,
  G-5) on the request that dialed it. A Route naming a nonexistent Service is a
  reference defect caught in candidate validity (G-3), not by the probe.
- **Explicitly unaffected:** allocation `Running`, Service `Stable`, backend
  eligibility, BPF hydration, and operator HTTPS meaning.
- **Ordering:** custody → coherent Route/frontend application candidate →
  `GatewayRelay::probe` (permitted before gateway identity; it uses an in-process
  ephemeral certificate, never the Gateway SVID or the public key) →
  `gateway_connect.probe` (permitted before gateway identity) → gateway SVID
  Current → actual Service Frontend probe. That probe stops at the validated
  receipt, `ReadyNoBackend` or `ReadySelected`; it resolves no applied identity
  and performs no mTLS → promote the admission object → bind TCP/443. The whole
  gate shares one `application_gate_deadline`; no step restarts it.
- **Failed-candidate rule:** a candidate that fails the gate (gateway identity
  unusable, connect-path or relay probe refusal, gate deadline, listener bind
  failure) is recorded by its Route/key generation and is not re-staged until
  an owner input changes: a Route change, a watch gap (`Lagged`), a certified-key
  change or a Gateway Identity change. Each of those clears the record. Backend
  facts, hydration and backend authentication are not owner inputs, so no
  backend event re-stages or clears anything. Promotion rechecks identity and
  key currency in memory; a gate result whose staged token has been replaced is
  discarded, never delivered. A Route committed to intent but not yet consumed
  by the owner is not owner-observable: the owner may promote the candidate it
  holds, which is indistinguishable from the Route committing just after
  promotion, and the Route event then stages the replacement and drains the
  promoted generation. No Route re-read happens at promotion.
- **Counterexample:** withholding allocation `Running` because host TCP/443 is
  occupied would make an unrelated workload state depend on a node listener it
  neither owns nor needs.
- **Evidence lane:** pure prerequisite/branch ordering; Tier-3 sentinel receipt,
  actual-frontend `NoBackend` and `Selected` (both bind, a `Selected` backend with a failing or absent peer identity still binds and returns 502 per request) and startup/refusal evidence;
  the relay probe's gold test (Sim lies, Tier 1) and one real-kernel run (Tier 3, once per boot);
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
  cgroup-BPF `NoBackend` receipt — empty set or no selectable backend (the
  `healthy` byte is not read, R-2) — yields 503 (typed outcome, no wait,
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
  URI SAN equals the workload identity actually selected by BPF, and only then is
  kernel TLS armed on that socket, before any application octet is written or read
  on it. (The public leg's equivalent: handshake, SNI and ALPN accepted, then armed,
  before any head is read.)
- **Affected state:** current internal request result only.
- **Failure projection:** missing gateway material/receipt identity,
  wrong-valid-peer, a kernel refusal to arm (`GatewayClientMtlsError::Arm`), timeout, disconnect and cancellation fail closed with no
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
  admitted connection has ended or been force-joined (an in-flight exchange's two halves, both sockets and
  both pipes are closed with it; an idle keep-alive connection is closed at once), the intent and receipt maps
  hold nothing, the closed application is retired, and the volatile Gateway
  Identity slot is cleared (synchronously; no reconciler round-trip) or the
  exact typed failure is in the report.
- **Affected state:** gateway runtime state and the Gateway Identity slot only.
- **Failure projection:** a non-clean `GatewayShutdownReport` is appended to
  `ServerShutdownError` after every owner has been attempted; a gateway timeout
  consumes only the one grace budget (`ServerHandle`'s `drain_deadline` argument; the gateway splits it into its drain deadline and the join reserve, R-64); an outer fail-stop bound abandons
  the stage and the process-local state dies with the process.
- **Explicitly unaffected:** operator HTTPS draining, workload and allocation
  state, the relative order of the existing stages, fail-stop exit status.
- **Ordering:** gateway stage → existing stages, unchanged. Leg-C and the
  guest-flow owner outlive every gateway upstream connection. Inside the stage
  (R-64): drain until `drain_deadline = shutdown_deadline - reserve`, then the
  force-abort, sweep, retirement and final owner joins until `shutdown_deadline`,
  one budget, never extended; the join reserve is not a second timeout.
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
   usable backend is 503 (the `healthy == 0` window is not consulted, R-2: the dial
   to a not-yet-ready guest is 502); an unreachable guest is 502.
3. **Unrelated state:** allocation `Running`, Service `Stable`, hydration and the
   operator listener advance identically with the gateway enabled, disabled or
   gated.
4. **Late success:** a stale probe or application result after supersession and
   a late `Selected` after a newer generation never restore the old admission
   object.
5. **Disconnect/reconnect:** a guest-flow owner restart or fail-stop during an
   in-flight upstream request ends it with 502 and leaves no intent or receipt.
6. **Feature disabled:** `ServerConfig.gateway == None` binds nothing, prepares
   no socket, runs no sentinel, runs no relay probe and leaves the cgroup hook's unregistered
   behaviour byte-for-byte unchanged.
7. **Relay gate (G1):** `GatewayRelay::probe` available: the gate proceeds; a probe refused by
   each substrate lie (ULP absent, cipher refused, splice losing octets, pipe clamped below 64 KiB) keeps
   TCP/443 unbound with `gateway.relay_probe`; an unrelated allocation, Service and the operator
   listener advance identically. The probe runs once per boot, before the gate's first candidate; it is
   not re-run for later staged generations, so a staged-generation probe result is the connector probe's
   only (boundary 4 covers its late result).
8. **Exchange boundary (G1):** a keep-alive reuse happens only after the response completed and the
   request body was fully relayed; an early response, a close-delimited response, a close_notify mid-body
   and a drain each close both legs and never reuse; a peer KeyUpdate inside a head, inside a chunk-size
   line and inside a body leaves the relayed octets byte-exact and is never forwarded (Tier 3; the
   kTLS lock exclusion is covered by A-K5 and the Tier-3 check, not by a model, R-19).

Ordering-dependent behavior requires seeded
`overdrive-sim` invariants; socket, public Web PKI, TPROXY, kTLS, vsock and wire
effects require the built default-feature binary on the native-metal
lane. A test CA is permitted only as the hermetic TLS mechanism and does
not satisfy the production public-trust claim.

Because the selected design adds a separate registered-selection `cgroup/connect4` program beside the existing one (R-1), DELIVER must run the repository's real-kernel verifier-regression and verifier-
budget gates (the pinned baseline and attach gates are in "Pinned: selection program, baselines and gates"). A per-connect cost gate is not part of the design (R-55: one report-only measurement, no gate). Any baseline
change requires the measured kernel record and review mandated by `bpf.md`; the
design does not pre-authorize a numeric bump. The separate program is attached beside the existing hook, so
it runs on every `connect4` in the attach cgroup subtree, not only on gateway upstream connects. For an
unregistered workload connect the added per-connect cost is one program run, one socket-cookie read, one
map lookup that misses, and an early allow; the selection work (outer and inner map lookups, rewrite,
receipt write) runs only for a registered socket, once per upstream connect and not once per HTTP body
chunk. Tier-4 `verifier-regress` covers the program's instruction count; no further budget is introduced.

## Wave: DESIGN / [REF] Component decomposition

These are responsibilities, not approved Rust types or module names.

| System responsibility | Likely existing owner/path | Change | Boundary handed forward |
|---|---|---|---|
| Public listener | `GatewayApplicationOwner` / `GatewayHandle` under `boot_server` / `ServerHandle` (D5) | EXTEND | Dynamic TCP/443 bind only after all gateway/dataplane/identity prerequisites; owned accept-stop/drain/unbind |
| `PublicCertifiedKeyCustody` | `overdrive-gateway::certified_key` + `overdrive-host::ca::PublicCertifiedKeyAeadCodec` | CREATE NEW stable custody owner over existing KEK/AEAD mechanism | `ManualCertifiedKeySource` installs; custody validates/builds+seals, persists `PublicCertifiedKeyV1`, publishes Current/Usable, then writes `PublicCertifiedKeyStatusRowV1`; paths/raw PEM/secrets stop before Route/listener/status |
| Public TLS handshake + HTTP/1.1 head/framing runtime | `overdrive-gateway::runtime` (pure `http1`, exchange driver) over `GatewayRelay` (`HostGatewayRelay` in `overdrive-dataplane`); rustls handshake only, httparse for heads (G1) | CREATE NEW in-process runtime | Active after prerequisite gate; bodies move kernel-side; bounded drain ([Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)) |
| `GatewayApplicationOwner` | `PublicRouteSetV1` + `IntentServiceFrontendResolver` + custody | CREATE ACTIVE OWNER | Atomic admission object retained per connection; sole writer of `GatewayApplicationStatusRowV1` |
| Eligible backend publication | ServiceLifecycle → `ServiceBackendRow` | REUSE UNCHANGED AS INDIRECT AUTHORITY | Gateway is not a consumer; ServiceMapHydrator remains the downstream consumer |
| BPF backend application | ServiceMapHydrator → Dataplane/EbpfDataplane | REUSE UNCHANGED (D1) | VM backends already enter `SERVICE_MAP` Maglev inner + `BACKEND_MAP` + `REVERSE_NAT_MAP`; the applied BackendId→`Backend.alloc` association is added inside Dataplane ownership |
| BPF selection and receipt | `cgroup_connect4_service` + `EbpfDataplane` | EXTEND (D3) | Registered socket-cookie intent/outcome selection in a separate `cgroup/connect4` program beside the existing hook (R-1, ADR-0224); the dataplane adapter owns socket preparation (ADR-0243) |
| `GatewayIdentitySlot` + `GatewaySvidLifecycle` | Existing CA/`issue_and_audit`/identity patterns | CREATE NEW single-slot sibling and pure lifecycle | Own issue/audit/hold/use/reissue/restart/drain-ordered clear for gateway subject; only `HostGatewayClientMtls` receives the private use snapshot; never AllocationId |
| `GatewayUpstreamConnector` + `GatewayConnectDataplane` | Existing attached cgroup hook, `SERVICE_MAP` Maglev inner/`BACKEND_MAP`, `HostGatewayClientMtls`, leg-C | CREATE connector/registry; EXTEND hook | The dataplane adapter prepares the socket and registers only cookie+ServiceKey; BPF selects; Dataplane supplies actual applied identity; connector transfers fd+SpiffeId into exact-peer mTLS; no userspace LB |
| Gateway status | `GatewayStatusReads` + exact ObservationStore point reads | EXTEND existing operator-mTLS `GET /v1/gateway/status` | Projects `GatewayApplicationStatusRowV1`, `PublicCertifiedKeyStatusRowV1`, non-secret identity/connect facts and relevant hydration into `GatewayStatusResponse`; 500 on observation read error |

## Wave: DESIGN / [REF] Driving ports

| Driving surface | System responsibility | Initial constraint |
|---|---|---|
| `overdrive serve` | Compose/probe prerequisites, bind TCP/443, own runtime/drain | Existing production entry point |
| `overdrive deploy <SERVICE_SPEC>` | Produce the existing Service/listener/workload intent and resulting ServiceLifecycle backend observations | Existing surface unchanged |
| `overdrive deploy <ROUTE_SPEC>` | Produce `PublicRouteSetV1` under [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md) through the [exact Application driving-port contract](#wave-design--ref-exact-driving-and-driven-ports) | Required operator journey; workload deploy arms remain unchanged |
| Operator-mTLS `GET /v1/gateway/status` | `GatewayStatusReads` reads active application/custody plus relevant BPF hydration status on control-plane HTTPS | `GatewayStatusResponse`; 200 when observations are readable, 500 on read error; separate from public response status |
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
| Public and upstream kernel-TLS legs, head/body relay | `GatewayRelay` (`HostGatewayRelay` over rustls `KernelConnection`, kTLS, splice; `SimGatewayRelay`) | TLS 1.3 + bounded HTTP/1.1; bodies spliced between kTLS sockets; probe before bind (ADR-0244, ADR-0246) |
| Gateway SVID lifecycle/hold/use | `GatewaySvidLifecycle` + action executor + `GatewayIdentitySlot` | `ensure_current` before bind, restart/near-expiry reissue, and `disable_after_drain`; no public/private-material getter |
| Selected-peer mTLS | `HostGatewayClientMtls` implementing `GatewayClientMtls` | Present `GatewayIdentitySlot` SVID and require exact receipt-selected workload SAN; transparent kTLS/`MtlsEnforcement` is not this L7 client's API |
| Clock/entropy | Existing injected `Clock` plus `PublicCertifiedKeyAeadCodec` ring `SystemRandom` | ADR-0223/0231, ADR-0225/0234 and ADR-0226/0235–0240 own the custody, SVID/mTLS and public-runtime choices; exact clock/entropy contracts are in [Application persistence/custody](#wave-design--ref-persistence-custody-and-redaction), [Gateway Application/status/HTTP](#wave-design--ref-gateway-application-status-and-http-contract), and [gateway identity/lifecycle](#wave-design--ref-gateway-identity-and-lifecycle-integration) |

## Wave: DESIGN / [REF] Technology choices

| Choice | Pin / status | Reason and honesty boundary |
|---|---|---|
| Rust | Workspace Rust 2024 | Existing single-process architecture and type/port discipline |
| Tokio | Workspace runtime | Existing lifecycle/cancellation owner |
| `httparse` / `http` | Workspace lock `1.10.1` / existing; direct dependencies of `overdrive-gateway` (already in the lock through `hyper`; the choice of `httparse` as the head tokenizer is this row's, not an ADR's) | MIT/Apache-2.0; head tokenizing and `HeaderMap`/`Authority` types. `hyper`, `hyper-util` and `tokio-rustls` are not used by the public data path (the operator listener keeps `hyper`) |
| `rustls` | Workspace `0.23.39`, provider `ring`, unbuffered handshake API and `kernel::KernelConnection` | Public TLS + internal gateway mTLS handshakes and kTLS rekey; no FIPS claim |
| Linux kTLS, `splice(2)` | Pinned appliance kernel 7.0 (ADR-0068) | Record layer and body relay; GPL kernel facility, no library |
| Existing workload-identity CA | ADR-0063/0067 | Issuer foundation reused by dedicated gateway holder; never public Web PKI |

No cache, queue, load balancer, sharding, replication, CDN, DNS provider, or
external proxy is added: the one-node/one-origin envelope has no bottleneck
that justifies them.

### Architecture enforcement

- Preserve the repository's single-process hexagonal boundary: core policy and
  snapshot compilation depend on ports; `rustls`, sockets, kTLS, `splice`, storage
  and kernel effects stay in host/wiring adapters (the `unsafe` kernel-TLS calls only in
  `overdrive-dataplane::gateway_relay`). The Rust structural enforcement for the relay is module privacy (the connector alone holds
  the dataplane handle; the relay's opaque halves are constructible only by their backend) plus the pure-module no-I/O scan for
  `runtime::{http1,routing,proxy_headers}`.
- `cargo xtask dst-lint` remains the Rust structural enforcement tool for its existing crate-class rules; it has no
  gateway-specific clause (`xtask/src/dst_lint.rs`), and the
  [Application component decomposition](#wave-design--ref-application-component-decomposition)
  and [Updated Reuse Analysis](#wave-design--ref-updated-reuse-analysis) state the gateway/dataplane/core dependency
  directions as review rules.
- By review rule (not a lint), the gateway runtime does not import/invoke ServiceLifecycle policy or consume
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
| `rustls` config experience; `hyper` server (operator listener) | `tls_bootstrap`, control-plane listener | TLS config mechanics | **REUSE rustls CONFIG PATTERN; DO NOT REUSE `hyper` FOR THE PUBLIC PATH** | Public cert trust/authority and HTTP proxy behavior are new; the operator CA/router are not reusable identities or handlers; the public data path is `GatewayRelay` + gateway-owned framing (G1). Contract shape: bounded-change (the operator listener is the preserved complement). |
| `mtls/ktls.rs` (kTLS arm) and `mtls/splice.rs` (splice pumps) | `overdrive-dataplane::mtls`, ADR-0070 | kTLS TX/RX arming from rustls secrets; splice across a kTLS boundary | **EXTEND `ktls.rs` PATTERN IN A NEW `gateway_relay` MODULE; DO NOT REUSE `splice.rs`** | `ktls.rs` arms AES-256-GCM from `dangerous_extract_secrets` and has no rekey, control-record or AES-128 path; `splice.rs` pumps plaintext to or from a non-TLS leg on blocking threads and is single-writer-per-leg with no body framing. The gateway needs `KernelConnection`, both AES-GCM suites, rekey, record-boundary-exact reads and per-message framing. Contract shape: unbounded-preservation of the transparent-mTLS arm and pumps (no edit), asserted by their unchanged tests. |
| `cgroup_connect4_service` + shared Service maps | ADR-0053/0040, `cgroup_connect4_service.rs:49-102` | Host connect-time destination rewrite; the attach path is `overdrive.slice` | **REUSE UNCHANGED; NEW SEPARATE PROGRAM FOR THE REGISTERED ARM + RECEIPT (R-1)** | `cgroup_connect4_service` and its Tier-4 baseline are not modified; a new `cgroup/connect4` program, attached beside it, is the cookie-scoped selection arm; unregistered sockets preserve current `LOCAL_BACKEND_MAP` behavior (contract shape: unbounded-preservation of the unregistered universe, asserted by a byte-equal unregistered-connect complement); no child cgroup/netns/XDP entry is needed unless V-GW-2 falsifies that `serve` is inside the attach subtree. |

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
| D-SYS-5 | Dedicated gateway identity lifecycle/holder owns non-allocation SVID issue/hold/use/reissue/clear | System trust [ADR-0234](../../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md); realization [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md) |
| D-SYS-6 | The dataplane adapter prepares the marked loopback-bound socket and registers `(SO_COOKIE, ServiceKey)`, then a separate registered-selection `cgroup/connect4` program (R-1) selects/rewrites through `SERVICE_MAP` Maglev inner + `BACKEND_MAP` and records actual BackendId | Packet entry and selection [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md); receipt [ADR-0232](../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md); socket preparation [ADR-0243](../../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md) |
| D-SYS-7 | ServiceLifecycle publishes eligibility; ServiceMapHydrator programs VM backends on its ordinary path (D1); under the Dataplane guard identity is Reserved before BPF work, the outer swap commits selection, then identity becomes Applied; gateway has no backend watch/cursor and no demand channel | [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0233](../../product/architecture/adr-0233-commit-gated-backend-identity-publication.md), [ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) |
| D-SYS-8 | Gateway Applied and BPF applied generations remain independent; the transient receipt pins actual BackendId and its identity read blocks on the commit guard until the process-lifetime association is Applied | [ADR-0229](../../product/architecture/adr-0229-keep-gateway-and-service-dataplane-generations-independent.md) |
| D-SYS-9 | Strict HTTP/1.1; one BPF-owned frontend handoff; no retry/replay/userspace LB | HTTP [ADR-0235](../../product/architecture/adr-0235-public-http11-only.md); single attempt [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md); packet entry [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md) |
| D-SYS-12 | Userspace rustls handshakes, kernel TLS record layer, gateway-parsed heads, spliced bodies; bounded memory, no CPU claim | [ADR-0244](../../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), [ADR-0245](../../product/architecture/adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [ADR-0239](../../product/architecture/adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md), [ADR-0247](../../product/architecture/adr-0247-gateway-exchange-is-full-duplex.md) |
| D-SYS-10 | Public 404 Route miss; only `NoBackend`, including an empty eligible set/no selectable BPF backend, is 503; selected-receipt identity missing/mismatch and connect/receipt/mTLS/upstream failures are 502; separate operator-mTLS `GatewayStatusReads` status reads active application/custody/hydration observations as `GatewayStatusResponse` 200/500 | Route [ADR-0236](../../product/architecture/adr-0236-canonical-public-route-match-key.md); attempt/error [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md); status [ADR-0227](../../product/architecture/adr-0227-redacted-operator-gateway-status.md); receipt [ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md); applied identity [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md) |
| D-SYS-11 | Boot sentinel `NoBackend` may prove connect/receipt capability before Gateway Identity Current; actual-frontend `NoBackend` and `Selected` are both bind-valid (R-65: the probe resolves no identity and performs no mTLS; a `Selected` backend's failure is a per-request 502); graceful drain runs as the first `ServerHandle::shutdown` stage (before the leg-C worker stage), joins connector/listener, clears receipts and clears the volatile identity slot synchronously (D5 section (c)) | [ADR-0230](../../product/architecture/adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md) |

## Wave: DESIGN / [REF] System traceability

| System obligation | Current authority/evidence | Status |
|---|---|---|
| Node-agent-embedded gateway, in-process BPF access (issue text names `hyper` + `rustls`; G1 replaces the `hyper` path) | GH #54 (no comments) + [ADR-0220](../../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) + G1 | Feature intent plus approved single-node/process placement; gateway absent in current production |
| Production public trust from manual Public Certified Key producer | Explicit user ruling + [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md) + [ADR-0231](../../product/architecture/adr-0231-preserve-last-usable-public-certified-key-replacement.md) | `ManualCertifiedKeySource` → `PublicCertifiedKeyCustodyHandle::install` → validate/build+seal → persisted `PublicCertifiedKeyV1` → Current/Usable opaque resolver publication → redacted status |
| Gateway observability | [ADR-0227](../../product/architecture/adr-0227-redacted-operator-gateway-status.md) | `GatewayStatusReads` uses the [exact Application status contract](#wave-design--ref-gateway-application-status-and-http-contract) for ObservationStore point reads of `GatewayApplicationStatusRowV1`/`PublicCertifiedKeyStatusRowV1` and relevant hydration; 200/500, secrets/raw dataplane failure strings omitted |
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
| D5 | The architect defines and pins the gateway adapter slot, boot construction/probe/bind point, shutdown order and the upstream-socket-preparation contract (below). The shapes are the confirmed D5 list (items 1-13) in the handoff section. | Approved by user 2026-11-01 (scope); shapes confirmed by user 2026-11-02 (R-18) | [Composition-root gateway slot, boot and shutdown](#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5) |
| G1 | Replace the userspace `hyper` data path with: userspace rustls performs the public TLS 1.3 handshake and the upstream exact-peer mTLS handshake, then installs kTLS TX+RX on each socket; backend selection stays in the kernel (cgroup/connect4, ADR-0224/0232); userspace parses only each request head and response head; bodies move kernel-side by `splice(2)` between the two kTLS sockets, by exact `Content-Length` or per chunk; userspace tracks message boundaries for keep-alive, body limits, deadlines and drain. | Approved by user 2026-11-02 | [Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1); ADR-0244, 0245, 0239, 0246, 0247 |
| G2 | The accepted L7 feature set is unchanged: routing ADR-0236, header policy 0237, limits 0238, streaming 0239, single attempt 0240, typed 404/502/503 0242, drain, exact-peer mTLS 0234, TLS 1.3 only 0226, HTTP/1.1 only 0235, keep-alive per RFC 9112. | Approved by user 2026-11-02 | Message-framing contract; status mapping delta |
| G3 | Run the contract-shaping spikes on native metal (items a-e) and take their results as design inputs, including the pipe rule, `KernelConnection`-only rekey with the KeyUpdate procedure, no CPU claim, and no kTLS-plus-sockmap dependency. | Approved by user 2026-11-02 | [findings-ktls-splice-relay.md](spike/findings-ktls-splice-relay.md); G1 contract-shaping items |
| G4 | Reopen P-2, P-9 and P-10, restated against the new design; ruled by R-1, R-2 and R-3 below. | Approved by user 2026-11-02 | Rulings table below |

### User rulings of 2026-11-02 (accepted contract text)

| Id | Ruling | Owner of the contract text |
|---|---|---|
| R-1 (P-10) | The gateway selection arm is a separate `cgroup/connect4` program attached beside `cgroup_connect4_service`; the existing program and its verifier baseline are unchanged. | [ADR-0224](../../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md) |
| R-2 (P-9) | The arm ignores `BACKEND_MAP.healthy`; a request in the ~1 s startup window dials a not-yet-ready guest and yields 502. | [ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) |
| R-3 (P-2) | `BackendIdAllocator` allocation and release semantics are unchanged (memo keyed by `(ip, port, proto)`). The read-only accessor `existing_id` changes neither, and is contract (R-24). | [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md), Reuse Analysis row |
| R-4 (B-1) | `disable_after_drain` is a synchronous clear of the volatile slot; `Action::DropGatewaySvid` is not part of the design; `gateway_identity.qnt` models the synchronous clear. | [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md) |
| R-5 (DG-11) | The cleanup-ledger slot is reserved before `open_upstream`; the intent is bound to the reserved slot after. | [ADR-0248](../../product/architecture/adr-0248-connector-reserves-its-cleanup-ledger-slot-before-preparation.md) |
| R-6 (DG-12) | Each gateway test composition binds a distinct `127/8` address; there is no port seam and no serialisation beyond node-global kernel state. | Test placement (Conformance placement) |
| R-7 (QC-1/2/3) | The stutter `idle` step and the owner-autonomous `stage`/`retire` steps are handled as stated in "Conformance action maps". | Conformance action maps |
| R-8 (P-12) | Both legs offer only AES-128-GCM and AES-256-GCM. | [ADR-0249](../../product/architecture/adr-0249-gateway-legs-offer-only-aes-gcm-tls13-suites.md) |
| R-9 (P-13a) | Ambiguous HTTP framing is rejected strictly. | [ADR-0251](../../product/architecture/adr-0251-gateway-rejects-ambiguous-http-framing.md) |
| R-10 (P-13b) | The exchange is full-duplex. | [ADR-0247](../../product/architecture/adr-0247-gateway-exchange-is-full-duplex.md) |
| R-11 (P-13c) | An interim `100` and `Expect` are forwarded; other interim `1xx` (not `101`) are dropped. | [ADR-0252](../../product/architecture/adr-0252-gateway-forwards-interim-100-and-expect.md) |
| R-12 (P-13d) | Close-delimited responses and HTTP/1.0 responses with an explicit length are supported. | [ADR-0253](../../product/architecture/adr-0253-gateway-supports-close-delimited-and-http10-length-responses.md) |
| R-13 (P-17) | The gateway never originates a KeyUpdate; only a peer-initiated KeyUpdate is handled. | [ADR-0250](../../product/architecture/adr-0250-gateway-never-originates-a-tls-keyupdate.md) |
| R-14 (P-18) | After an early or gateway-authored response the exchange does `close_write`, then a bounded `discard_remaining`. | [ADR-0254](../../product/architecture/adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md) |
| R-15 | The upstream leg carries one request per connection (`Connection: close`); downstream keep-alive is retained. | [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md) |
| R-16 (P-16) | Not adopted: no change to pipe lifetime and no explicit per-exchange pipe-release operation; the single rule stands (a read half's pipe is released when the half starts its next head read or is dropped). | [ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md) |
| R-17 (Q-1) | The >= 64 KiB empty-pipe invariant stays until [GH #339](https://github.com/overdrive-sh/overdrive/issues/339) (kernel 7.3 upgrade) lands. | ADR-0246 |
| R-18 | D5 contract list items 1-13 are confirmed, item 12 (lag-aware `IntentStore::watch`) reconfirmed; item 9 carries no `Action::DropGatewaySvid`. | D5 section |
| R-19 | The `relay_ktls` model is dropped; chunk framing is covered by property tests only. | Formal protocol model |

### User rulings of 2026-11-03 (accepted contract text)

The user ruled "proceed with the recommendations (prefer correctness)" on the proposals P-20..P-31 and the scope items that accompanied them. The ruled contract is stated once, in the section named in the last column; the P-id stays in the row for traceability.

| Id | Ruling | Owner of the contract text |
|---|---|---|
| R-20 (P-20), 2026-11-03 | `GatewayClientIdentityAccess` and its source trait `GatewayClientIdentitySource` are defined in `overdrive-gateway::ports`; `overdrive-dataplane` implements the source and `overdrive-gateway` names no dataplane type (dependency direction `overdrive-dataplane -> overdrive-gateway`). | Gateway identity and lifecycle integration ("Placement of `GatewayClientIdentityAccess`"); D5 item 2 |
| R-21 (P-21), 2026-11-03 | The Tier-3 `BACKEND_MAP`-fill test (S-PIG-75) is dropped; the rollback exposure is stated and bounded. | Formal protocol model, `selection_receipt` ("Not conformance-replayable") |
| R-22 (P-22), 2026-11-03 | `GatewayRelay::discard_remaining` returns `Result<(), LegReadError>`. | Kernel relay data path, `GatewayRelay` |
| R-23 (P-23), 2026-11-03 | The real-kernel ULP-absent stimulus (`rmmod tls`, `unshare(CLONE_NEWUSER)` child) is dropped; each substrate lie is refused at Tier 1 and the honest real-kernel pass stays (S-PIG-114 is reduced to the honest pass). | Earned-Trust probe for the relay |
| R-24 (P-24), 2026-11-03 | R-3 constrains allocation and release behaviour; the read-only `BackendIdAllocator::existing_id` changes none and is contract. | Commit-guarded Service-map and applied-identity transaction |
| R-25 (P-25), 2026-11-03 | `GatewayControl` is replaced by `GatewayRouteCommands` (the Route write handle) and `GatewayStatusReads` (the two redacted reads); a status-only caller cannot reach the write handle. | Gateway Application, "Exact owner, composition, generation and shutdown surface" |
| R-26 (P-26), 2026-11-03 | The bundled ADRs are split at the ruling boundaries; one ADR records one decision: ADR-0243 keeps preparation by the dataplane adapter and ADR-0248 takes the ledger order; ADR-0244 keeps kernel TLS after a userspace handshake, ADR-0249 takes the cipher set and ADR-0250 takes the no-KeyUpdate-origination rule; ADR-0245 keeps head parsing and message framing, ADR-0251 takes strict ambiguous-framing rejection, ADR-0252 takes interim `1xx` and `Expect`, ADR-0253 takes close-delimited and HTTP/1.0-with-length responses; ADR-0247 keeps the full-duplex exchange and ADR-0254 takes the lingering close. | ADR-0243 through ADR-0254 |
| R-27 (P-27), 2026-11-03 | `memo` and `next_bid` of `selection_receipt.qnt` are model-only; the observable consequence is compared through `ident` and the per-cookie `receipts` projection. | Formal protocol model, `selection_receipt` |
| R-28 (P-28), 2026-11-03 | Generation ids are content digests with no start value or order; the QC-2 driver compares by equality through a naming translator. | Formal protocol model, `gateway_application` |
| R-29 (P-29), 2026-11-03 | Shutdown holds a cleanup-only capability, `GatewayConnectCleanup`, which `GatewayConnectDataplane` extends; only the connector holds a handle that can call `open_upstream`. | Gateway upstream path into leg-C |
| R-30 (P-30), 2026-11-03 | `GatewayIdentityLifecycleControl::disable_after_drain` takes no deadline, is not async and is infallible (the u64 epoch cannot be exhausted; `GatewayIdentityLifecycleError::EpochExhausted` does not exist); `GatewayIdentityWaitPhase` is removed and `GatewayIdentityLifecycleError::Timeout` is a unit variant. | Gateway identity and lifecycle integration |
| R-31 (P-31), 2026-11-03 | `issues` of `gateway_identity.qnt` is model-only; the `issue_complete` handler fails the step when no Issue of that epoch is pending; the `submit_evaluation` handler asserts only that no pending Issue carries an epoch beyond `desired()`'s (a handler has no model state; a duplicate or early Issue is visible through `audited`; the literal projection, P-37, was rejected by the user on 2026-11-04). | Formal protocol model, `gateway_identity` |
| R-32, 2026-11-03 | The pure `http1` functions and the relay-pipe floor functions are pinned (signatures and error taxonomies) for the source-local tests of S-PIG-76, 77-83 (U), 85 and 86. | Pure `http1` and relay-pipe functions |
| R-33, 2026-11-03 | `GatewayConnectCleanup::live_receipts()` is the one read-only per-cookie receipt snapshot (the residual receipt count is its length; there is no separate count accessor); QC-3 projects `receipts` through it and compares the selected `BackendId` per cookie; `taken` is a function of projected state asserted by its handler, and the spec has no `idres` variable (`read_identity` is a step whose handler only calls `selected_backend_identity`; the State comparison checks `ident` and the per-cookie `sel`). | Gateway upstream path into leg-C; Formal protocol model, `selection_receipt` |
| R-34, 2026-11-03 | The QC-2 generation translator stays a pure naming map populated only from `application_status()` values; no accessor is added. | Formal protocol model, `gateway_application` |
| R-35, 2026-11-03 | `DataplaneError::{BackendIdentityConflict, ServiceApplyRollback}`, `ServiceApplyStage` and `ServiceApplyRollbackFailure` live in `overdrive-core`; `ReverseNatDelete` carries `{ backend: SocketAddrV4, proto: Proto }`. | Commit-guarded Service-map and applied-identity transaction |
| R-36, 2026-11-03 | The two runs that need a fault or pause seam are not replayed (no owner call without a seam); the rollback path (the abandoned reservation) is model-only because `update_rejected` fails in the owner's preflight, before any identity is reserved. The existing `receipt_conf` run `conflicting_alloc_rejected` carries the rejection's empty-update tail (memo released, `ident` history kept, counter not recycled, the next fresh backend gets the id `next_bid` held), and carries a `#[quint_test]`. | Formal protocol model, `selection_receipt` |
| R-37, 2026-11-03 | The response-head budget starts when the upstream request is completely written; a request body in flight is bounded by the request deadlines, not by `response_head_deadline`. | Message-framing contract (deadline composition) |
| R-38, 2026-11-03 | The model allocator follows the production allocator: the memo is released at orphan GC, the counter never recycles a number, `ident` keeps historical identities; the BackendId reuse divergence across an empty update is a model defect, not an implementation one. | Formal protocol model, `selection_receipt` |
| R-39, 2026-11-03 | S-PIG-123 stays: the host adapter's refusal of real handshakes is observable only on a real handshake; it asserts the observable outcomes (refused, no HTTP read, no upstream, nothing armed, descriptors restored), not the `PublicHandshakeError` variant, and the error taxonomy is unchanged. | Authoring status by batch |
| R-40, 2026-11-03 | The two new dependency edges conform to `development.md` § "Port-trait dependencies": `overdrive-sim -> overdrive-host` is a `[dependencies]` edge of the adapter-sim crate on an adapter-host crate (not an `overdrive-host` dev-dependency) and adds no node that `overdrive-sim -> overdrive-gateway -> overdrive-host` does not already reach; `overdrive-dataplane -> overdrive-sim` is a `[dev-dependencies]` edge (the sanctioned placement) forming a dev-only cycle with the existing `overdrive-sim -> overdrive-dataplane` dev-dependency; `xtask` depends on neither. Both are kept. | Technology choices ("Architecture enforcement") |
| R-41, 2026-11-03 | `CliError::ServerShutdown.source` is `Box<ServerShutdownError>` (`clippy::result_large_err`). | Handoff |
| R-42, 2026-11-03 | Extra scope is dropped: the four listener-level tests of S-PIG-79..82 (the S-PIG-77 and S-PIG-78 listener arms stay), the pre-effect second test of S-PIG-20, the kernel-cite re-derivation of S-PIG-116 (the scenario itself is deleted, SUB4) and S-PIG-75; the Sim test-support surface that realises a pinned capability stays. | Authoring status by batch |
| R-43, 2026-11-03 | The `dst-lint` struct-literal violations in `action_shim/replacement_acceptance.rs` (`AllocStatusRow { .. }` and `LogicalTimestamp { .. }` in `predecessor_row`) are cleared by a plain `#[cfg(test)]` on the fixture function: the scanner exempts only an item whose own attributes hold a top-level `cfg(test)` (it cannot see the `cfg(all(test, feature = "integration-tests"))` on the module declaration in `mod.rs`); no scanner change and no marker. | Technology choices ("Architecture enforcement") |
| R-44, 2026-11-03 | No control-event field is added to the Sim leg transcript; S-PIG-90 keeps the octet check, because the port has no operation that carries a control record between legs. | Authoring status by batch |
| R-45 (P-39), 2026-11-04, user decision | A gateway SVID issuance failure (`CaIssuanceError::Ca` or `::Audit`) is an operator-visible occurrence: `GatewayApplicationStatusRowV1` carries a monotone `issue_failures: u64` and a depth-1 `last_issue_failure: Option<SvidIssueFailure { cause: Ca \| Audit, at }>`, projected into `GatewayApplicationStatusBody` beside `gateway_identity`. A successful hold never clears them. Mechanism (corrected from P-39's text, which routed the facts through `GatewayIdentityView`): the executor records the failure through `GatewayIdentityActionTarget::record_issue_failure`; the owner reads it through the new `issue_record()` read method beside `current()` (woken by the existing `subscribe` watch), because the owner is the row's sole writer and the reconciler's hydration port does not need the facts. ADR-0274. | SVID issuance failure occurrence |
| R-46, 2026-11-04, user decision | The same occurrence surface exists for WORKLOAD SVIDs, on its own row: `WorkloadSvidIssueStatusRowV1` keyed by `AllocationId`, written only by the `IssueSvid` executor through the atomic `ObservationStore::record_workload_svid_issue_failure`, read by `GET /v1/allocs` into `AllocStatusRowBody` and rendered by `render::workload_describe` (the live renderer; `render::alloc_status` is its pre-#220 name). The allocation row (`AllocStatusRow`) is NOT changed: its lifecycle writer rejects an update that does not dominate the stored `updated_at` (`observation_backend.rs:517-525`), so a second writer cannot share it. ADR-0275. | SVID issuance failure occurrence |
| R-47 (a), 2026-11-04 (user decision, delegated: prefer correctness) | Gateway DISABLED: no `GatewayApplicationOwner` exists, so a disabled node never writes `GatewayApplicationStatusRowV1`, and `GET /v1/gateway/status` returns the fixed body `enabled=false`, `configured_address=None`, `listener_bound=false`, `application=None`, `certified_key=None` without any ObservationStore read. A row left by an earlier enabled boot of the same node is neither read nor projected while disabled. The occurrence fields live inside `application`, so a disabled response carries none; an enabled node with no row yet is `enabled=true`, `application=None`. | SVID issuance failure occurrence |
| R-48 (b), 2026-11-04 (user decision, delegated: prefer correctness) | `GatewayIdentityStatus` is a pure projection of the slot's held facts and the owner's injected Clock at the projection instant: no held facts = `Absent`; held facts whose `not_after` is not after `now` = `Unusable` (the `IdentityExpired` case); otherwise `Current`. The owner projects at its existing wakes plus the failure wake (no timer is added), so `Unusable` is reached when a projection runs with expired held material; in the case that matters, rotation is failing, so failure wakes occur. Staleness bound: until the next projection the row may still read `Current` for material that has since expired. Lifecycle outcomes never write it. A boot identity wait that ends in `GatewayIdentityLifecycleError::Timeout` maps to NEITHER value: it sets `unavailable = Some(GatewayIdentityUnusable)` (the S-PIG-51 collapse, an application-level cause) and `gateway_identity` stays what the slot says (`Absent` when empty). A never-succeeded issuance reads `Absent` with `issue_failures > 0` and `last_issue_failure = Some`; a fresh start reads `Absent`, `0`, `None`; a recovered node reads `Current` with the count and last failure retained. | SVID issuance failure occurrence |
| R-49 (c), 2026-11-04 (user decision, delegated: prefer correctness) | `GatewayIdentityLifecycleError::IssueFailed` is DELETED. `ensure_current` is the only waiting operation and it ends in `Current`, `Timeout`, `Closed` or `ReconcilerUnavailable`; an issuance failure does not end the wait because the reconciler retries on its backoff, and ending the wait on a recorded failure would be a guess that issuance will not succeed. The cause of a failure is on the status row (R-45). | SVID issuance failure occurrence |
| R-50 (d), 2026-11-04 (user decision, delegated: prefer correctness) | The occurrence surfaces are written from the CONFIRMED failure: the executor's `Err` from `issue_and_audit`, observed after it returns, never at emit. The reconciler View `IssueRetry` / `GatewaySvidLifecycleView.retry` is stamped on emit by design (retry-policy memory, ADR-0067 D8): it stays as is, is not an operator surface and is not read by the new surfaces. Deriving retry attempts from the confirmed record instead is the PROPOSED P-40. | SVID issuance failure occurrence |
| R-51, 2026-11-04, application of the user's instruction to follow the versioned-envelope procedure | Envelopes. The gateway row has no production writer and no persisted bytes (`archive_for_store` is a scaffold), so the two new fields are added to `GatewayApplicationStatusRowV1` IN PLACE (no `V2`); its `FIXTURE_V1` golden bytes are regenerated because that V1 was never persisted. The workload half changes no existing persisted envelope: it adds the new envelope `WorkloadSvidIssueStatusRowEnvelope::V1` with its own golden fixture. `AllocStatusRowEnvelope` is untouched. The `AllocStatusRowBody` JSON gains two additive `#[serde(default)]` fields (a wire change, not a persisted one). | SVID issuance failure occurrence |
| R-52, 2026-11-04, **user-confirmed 2026-11-04 (persistence/recovery behaviour)** (of the user's "monotone count": without it a crash-looping CA failure would reset the count on every restart) | Gateway count monotonicity across restarts: the slot's facts are process-local; the owner reads the prior row (`gateway_application_status_row`) at its first projection and projects `base + process facts`. A failed read is retried like any failed row write (the facts stay in the slot; no boot refusal, no new boot-error variant) and is never defaulted to zero. An undecodable prior row reads as absent (ADR-0048 observation policy; the store emits one `observation.row.skipped` warning, R-56) and the count then restarts at zero, which bounds the exposure to the count and `last`. | SVID issuance failure occurrence |
| R-53 (P-38), 2026-11-04, user decision | `ApiClient` gains exactly two methods, `withdraw_route` (`DELETE /v1/routes/{id}`) and `gateway_status` (`GET /v1/gateway/status`), with the signatures pinned in "Exact Route, deploy, configuration and owner surfaces" (`impl ApiClient`). They have no production caller: the CLI gains no verb (the dormant-surface gate of "Public surface and live-caller audit" is waived for these two by this ruling; they exist so the operator-surface tests drive delete and status through the client's trust handling and error typing). The authored callers are the S-PIG-36 DELETE arm, S-PIG-35 (route withdraw and status read) and S-PIG-42 (status read); so `gateway_status()` has a caller and a test (user ruling 2026-11-05, which moves S-PIG-35 and S-PIG-42 from plain HTTPS onto `ApiClient`). `withdraw_route(&str)` stays as pinned (confirmed by the user, 2026-11-05); no pinned signature changes. | Driving surfaces; Public surface and live-caller audit |
| R-54, 2026-11-04, user decision (delegated pins by the architect) | (a) Tier-3 hook tests over a real cgroup with real `connect(2)` (no Tier-2: `BPF_PROG_TEST_RUN` returns `ENOTSUPP` for `cgroup_sock_addr`); (b) the new program joins the `cargo verifier-regress` corpus with its own baseline file; (c) the attach/load gate is rig construction success plus S-PIG-124's registered connect (see "Pinned: selection program, baselines and gates", gate 3); (d) no benchmark lane is invented (see R-55 for the one report-only measurement); (e) `SpiffeId::for_gateway` is tested directly; (f) = R-53; (g) the gateway owner restart over an undecodable prior status row is driven through the gateway `World` over the real `LocalObservationStore` (test-support seam in "Pinned: selection program, baselines and gates"); (h) S-PIG-51's ignore key is `gateway-identity-error-collapse`. Pinned by the architect under the delegation: the program file and function name and the baseline file name. | Pinned: selection program, baselines and gates; Scenario list |
| R-55 (P-41), 2026-11-05, user ruling | (a) **Per-connect cost.** No lane, no gate, no baseline and no harness binary. One report-only point-in-time measurement during DELIVER; no lane, gate, baseline or harness binary; GH #287 comment URL https://github.com/overdrive-sh/overdrive/issues/287#issuecomment-6652745629; methodology in the roadmap criterion of the selection-hook step. (b) **Confirmations.** `withdraw_route(&str)` stays as pinned (R-53). The selection hook reads neither `user_ip4` nor `user_port`, so the read half of the low-16-NBO rule is a stated residual, accepted by the user: it is carried by the sibling S-PIG-41 and the `development.md` rule. The re-keying of the five test markers to `gateway-application-owner` is confirmed and adds no scope. The BPF program stays `cgroup_connect4_gateway` at `crates/overdrive-bpf/src/programs/cgroup_connect4_gateway.rs`. S-PIG-107 stays as the only Tier-3 discharge of assumption A-K5 (`key_update_request_during_a_large_splice_out_never_tears_the_record_stream`, key `kernel-relay-host-adapter`). (c) **Declined by the user.** The Local-vs-Sim row-persistence test for the gateway status rows (evidence stays the first consumers) and a source-local test for the `gateway.upstream.receipt.validated` emitter (evidence stays the end-to-end file of the last step). No roadmap step is inserted before the Gateway Application owner step. | Pinned: selection program, baselines and gates; Scenario list; DELIVER roadmap |
| R-56, 2026-11-05, user ruling ("go with recommendations") | The observation store emits the skipped-row warning itself, as the structured tracing event `observation.row.skipped` with SEPARATE fields `table` (the redb table name) and `key` (the row's id), the shape of the existing `observation.envelope.decode_failed` event in `crates/overdrive-store-local/src/observation_backend.rs` (`table = table_name`, `key`, `source`). The codec signature `from_store_bytes(bytes, key)` is unchanged and the codec does not log. An undecodable stored row (envelope or layout failure) reads as absent. This holds for the point reads `public_certified_key_status_row`, `gateway_application_status_row` and `workload_svid_issue_status_row`, and for the prior-row read inside `record_workload_svid_issue_failure`, in both `LocalObservationStore` and `SimObservationStore`. | SVID issuance failure occurrence; Persisted format |
| R-57, 2026-11-05, user ruling (option 1) | `UnboundGatewayBuilder::new` gains `node_id: NodeId` as its first parameter, before `config`. The Gateway Application owner writes `GatewayApplicationStatusRowV1` keyed by `NodeId` and, at its first projection, reads its own prior row through `ObservationStore::gateway_application_status_row(node_id)` (R-52); no other pinned input gave the owner its node. `boot_server` G2 passes the node it already holds, the same `NodeId` it passes to `GatewayIdentitySlot::new(node_id, ..)`. `GatewayConfig` stays operator configuration only (bind address and manual key). Rejected: a `node_id` field on `GatewayConfig` (mixes operator configuration with node identity the platform already owns); a new `ObservationStore` "my node" accessor (new port surface for a value `run_server` already holds). | Re-pinned composition signatures; Gateway Application status projection |
| R-58, 2026-11-05, user ruling (option 2) | The Gateway Application owner writes no `GatewayApplicationStatusRowV1` until a Route exists, and identity or failure wakes write none before a Route exists either. User's reason: writing `RouteAbsent` when no Route has been configured makes no sense; the status of an unconfigured gateway is the absence of a row (`enabled=true`, `application=None`, R-47), not a failure cause. When a prior row exists from an earlier boot, the owner carries its base as R-52 states. Rejected: writing `RouteAbsent` before any Route exists (it reports a failure for a configuration the operator never made). | Gateway Application status projection; SVID issuance failure occurrence |
| R-59, 2026-11-06, user ruling | `update_service` rejects, before any mutation, an update whose reverse-NAT key already maps to a different VIP, with the typed `DataplaneError::ReverseNatConflict { key, current_vip, attempted_vip }` in `overdrive-core` beside the R-35 variants. The check is part of planning (`plan_service_apply`), so nothing is overwritten and rollback never restores another Service's reverse-NAT entry. Field types are pinned once, in the DataplaneError taxonomy under Commit-guarded Service-map transaction. | Commit-guarded Service-map and applied-identity transaction |
| R-60, 2026-11-06, user ruling | `ServiceApplyRollbackFailure::BackendRestore(BackendId)` exists. A failed in-place BACKEND_MAP restore during rollback is returned as `ServiceApplyRollback` carrying that failure; it is not only logged. Rollback saves the prior value of each in-place update and restores it; a failed restore is typed. | Commit-guarded Service-map and applied-identity transaction |
| R-61, 2026-11-06, user ruling | The Dataplane `service_cleanup_debt` set is retried at three points: the next `update_service`, the Dataplane probe and the shutdown sweep. The probe and shutdown retries are delivered in roadmap step 05-01, which owns `EbpfDataplane::probe` and the shutdown stage. | Commit-guarded Service-map and applied-identity transaction |
| R-62, 2026-11-06, user ruling | The admission scaffolds no step owned are assigned to roadmap step 04-07, which already owns `GatewayAdmission::acquire_connection` and `GatewayGenerationRetention::{try_connection, try_request, counts}`: `GatewayAdmissionBudget::{try_public_connection, try_request}`, the `try_connect` refusal arm (`Err(_) => todo!`, the admission refusal) and `GatewayConnectionLease::try_request`. Ownership only; the existing signatures are the contract and no API is added. | Gateway admission and retention; DELIVER roadmap |
| R-63, 2026-11-06, user ruling | The connector's manual per-arm `release_intent` is replaced by the one private RAII cleanup guard the connector paragraph already pins ("One private RAII guard calls `cleanup` on every result and owns the exact pre-registered cleanup reservation until cleanup succeeds"), so a `connect_for` future dropped mid-`authenticate(..).await` or at the connect `select!` still cleans the registered intent and records any failure on the reservation. One default-lane cancellation test, in the connector's acceptance tests, drops a held connect future at `authenticate` and asserts the ledger length and `live_intent_count()` return to 0 (or the failure is retained on the record for `retry_all`). Ownership of the work: roadmap step 03-04 (the connector sequence it completes). No public API is added; the guard is private and the reservation methods are the already pinned `bind`, `record_intent_failure`, `record_receipt_failure` and `complete`. | Gateway upstream path (connector paragraph); Gate G-1 cleanup ledger |
| R-64, 2026-11-07, user ruling ("Reserve time for the joins inside the one grace period") | Gateway shutdown keeps ONE operator-visible grace budget. `shutdown_deadline = start + grace`; `drain_deadline = shutdown_deadline - GATEWAY_SHUTDOWN_JOIN_RESERVE`. Drain (in-flight exchanges and lingering discards) ends at `drain_deadline`; every step after drain, including the final owner joins, runs until `shutdown_deadline`, so the whole stage still completes within `grace`. `GATEWAY_SHUTDOWN_JOIN_RESERVE` is a crate-private `const` of `overdrive-gateway` equal to `Duration::from_millis(500)`; it is not operator-configurable (a config field would be new surface and is not proposed here). A `grace` no larger than the reserve has an empty drain window by definition (`drain_deadline = start`, computed with `checked_sub`, never a wrapped or clamped deadline that lengthens drain): no exchange is drained, every connection still open is force-aborted at once and counted in the existing `GatewayShutdownReport.forced_connections`, and the final owner joins still run to `shutdown_deadline = start + grace`, never beyond it. Production cannot reach this case: the only production caller passes the fixed 5 s `DEFAULT_DRAIN_DEADLINE`. No new report field, error variant or config surface is added by this ruling. Roadmap owners: step 04-03 (`application.rs` shutdown change) and step 05-01 (the `gateway` shutdown stage). | Gateway shutdown ordering; Lifecycle Gate Ownership (shutdown); scenarios S-PIG-60, 101, 115 |
| R-65, 2026-11-08, user ruling (approval "yes") | Route activation does not depend on backend health or backend authentication. Gate G-1 keeps only gateway-local prerequisites: custody / Public Certified Key usable; a coherent Route/frontend candidate; `GatewayRelay::probe`; `gateway_connect.probe`; Gateway Identity Current. The selected backend's exact-peer mTLS (`SelectedPeerAuthenticated`) is no longer a bind or promotion condition: a failed exact-peer mTLS to a selected backend is a per-request upstream failure (502, the existing mapping), as `ReadyNoBackend` already does not block (503). The actual-frontend probe stays only as a gateway-local proof that the frontend resolves through the registered connect path: it validates the receipt (`NoBackend` or `Selected`), resolves no applied identity and performs no mTLS; `GatewayConnectProbeOutcome` is `{ ReadyNoBackend, ReadySelected }`. The probe records no backend-outcome status and the operator status surface gains no field (a probe-derived backend-outcome status would be new scope: a proposal only). A Route naming a nonexistent Service is a reference defect caught in candidate validity (G-3), not by probing. Consequence for the owner's failed-candidate rule: a candidate fails only for gateway-local reasons, all driven by owner inputs (Route, certified key, Gateway Identity, watch gap) or gateway-local substrate (relay and connect probe, gate deadline, listener bind); a failed generation is not re-staged until a Route change, a watch gap, a certified-key change or a Gateway Identity change clears it. Consequence for the held probe (DG-9): the gate no longer reaches `GatewayClientMtls::authenticate`; the connector probe awaits no driven-port operation, so a held gate parks at an awaited operation the gate owns and needs no `Selected` receipt. Rejected: keeping `Selected` -> exact-peer mTLS as a bind condition (it ties node ingress to the readiness and authentication of one arbitrary backend, which Envoy, Istio, Gateway API, AWS ALB, Cloudflare and Linkerd do not do; research `docs/research/networking/ingress-route-activation-vs-backend-health-comprehensive-research.md`); retiring the actual-frontend probe (the user ruled it stays while it proves something gateway-local). | Gate G-1 and the Lifecycle Gate Ownership gate rows; D-SYS-11; G3/G4 boot rows; `GatewayConnectProbeOutcome`; scenarios S-PIG-58, 68, 74; DG-9 and the QC-2 driver pins; `gateway_application` model |

Earlier approvals: D1-D5 (2026-11-01), G1-G4 (2026-11-02). GH #338 (raw TCP ingress) is out of scope; GH #339 is the kernel 7.3 upgrade.

### Decided by rule (no user ruling needed)

1. **Gateway SVID reconciler wakeup.** `.claude/rules/reconcilers.md` § "The wakeup model": host-backed, empty `interests()` plus `resync_schedule()`; the period is a DELIVER tuning constant.
2. **Lag-aware `IntentStore::watch`.** Decision D-APP-10 of the design the user approved 2026-09-13; carried unchanged (D5 section "Previously approved public surface carried unchanged"). The optional simplification is the PROPOSED P-8 below.
3. **Shutdown order.** The order lives in D5 section (c) and Gate G-6: the gateway stage runs before the leg-C worker stage and clears the volatile slot synchronously. It is not an independent decision and has no ADR of its own.
4. **ADR structure.** Custody domain boundary and acquisition-versus-consumption: ADR-0223. Identity slot: ADR-0225. Packet entry in the existing dataplane: ADR-0224. Public TLS versus gateway mTLS: ADR-0234. Rejected ES/CQRS alternatives: ADR-0221. Backend unavailability: ADR-0242. ADR-0222 is not split.
5. **Single hydration read-port.** `GatewayIdentityView` and one `HydrationContext` field.
6. **Source-settled validation residue.** V-GW-2 (socket cgroup is the creating task's) and the V-GW-1 client-handshake question are settled by kernel and repository source (Contract-shaping validation items).

### Recorded observations (no ruling)

An observation is recorded for DESIGN; it rules no change and adds no scope.

- **D8 — restart re-emit on every wake.** In `GatewaySvidLifecycle::reconcile` (`crates/overdrive-reconcilers/src/gateway_svid_lifecycle.rs`),
  the restart branch (`actual` empty and `ever_issued` true, meaning the audit rows already name the gateway subject) clears the retry entry
  and emits `Action::IssueGatewaySvid` without consulting the backoff gate. While that state persists, every wake (the 30 s
  `resync_schedule()` and every failure wake) re-emits the reissue action and again clears the retry entry, so a failing reissue after a
  restart is retried on each wake rather than on the `backoff_for_attempt` schedule. Whether that bypass is intended for repeated failures is
  not ruled here; no code change is ruled.

### PROPOSED — awaiting user

No item below is part of the design until the user adds it. All are optional and non-blocking (P-38 is shown as adopted for the record). R-1..R-65 above are ruled contract text (R-47..R-51 with the provenance stated per row; R-52 user-confirmed 2026-11-04; R-53 and R-54 user rulings of 2026-11-04; R-55, R-56, R-57 and R-58 user rulings of 2026-11-05; R-59 through R-63 user rulings of 2026-11-06; R-64 user ruling of 2026-11-07; R-65 user ruling of 2026-11-08), not proposals.

| Id | Proposal | Reachable production failure it prevents | Cost | Cheaper alternative |
|---|---|---|---|---|
| P-1 | Carry the branch `.claude/rules/spike.md` addition ("A spike may drive an existing component"). | None gateway-specific. | One rule paragraph. | Do not carry it. |
| P-3 (optional, non-blocking) | Correlate the connect intent and receipt through `BPF_MAP_TYPE_SK_STORAGE` on the upstream socket instead of cookie-keyed hash maps. `bpf_sk_storage_get` is available to `cgroup/connect4` programs (`net/core/filter.c:8215-8216 @ v7.0`) and the entry dies with the socket. **If adopted it would delete:** the cookie-keyed intent and receipt maps, the cleanup ledger and the `cleanup`/`cleanup_all_gateway_intents` actions, the live-count sweeps (`live_intent_count`/`live_receipts`), the 128-entry registry capacity, and the `SO_COOKIE` read step. **If adopted it changes ADR-0232's contract** (`GatewayConnectDataplane` and the intent/receipt shape) and is contract-shaping, so it requires a new spike (userspace update by socket fd, connect4 access to `ctx->sk`) BEFORE DISTILL, and re-running the V-GW-5/selection-receipt checks and the Quint `selection_receipt` model. | None; a simplification. | A new spike and a changed [ADR-0232](../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md) plus follow-on edits to ADR-0228/0243, the D5 surface and the Quint model. | Keep the cookie maps as designed (the design as written). |
| P-4 | Open a GitHub issue that `ServiceMapHydrator` leaves `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` entries (`healthy = 1`) at least 23 s after `job stop` (D4 findings, cause not investigated). The gateway does not depend on the fix: a stale entry still selects a backend whose registered inbound tuple is gone, the shared nft output rule resets the dial (`M:crates/overdrive-netlink/src/nft.rs:833`) and the request returns 502. | A stopped Service's backend stays selectable by the XDP and gateway paths. | An issue (needs user approval before creation). | Investigate under the gateway DELIVER. |
| P-6 | A scheduled metal CI lane for the gateway Tier-3 runner. | A regression in the gateway wire path ships unseen. | CI minutes and a metal lease. | Run `cargo xtask metal run --` by hand per release. |
| P-8 (optional, non-blocking) | **Simplification of the previously approved D-APP-10.** The approved contract (2026-09-13, carried unchanged) changes main's `IntentStore::watch` (`M:crates/overdrive-core/src/traits/intent_store.rs:318-332`, a `Stream<(Bytes,Bytes)>` where an empty value means delete) to a lag-aware `IntentSubscriptionEvent` (`Changed`/`Lagged`) and migrates every watcher (`LocalStore`, Sim, the action-shim test fake, existing consumers). Alternative: keep main's `watch` and relist the Route/custody keys on a fixed period (bounded staleness instead of fail-closed on lag); main's watch broadcast has a 1024 capacity and drops silently on lag (`M:crates/overdrive-store-local/src/redb_backend.rs:73, 403-417`). | A withdrawn Route stays served after a missed watch event (what the approved contract closes). | The approved contract costs one cross-crate trait change and a bounded watcher migration; the alternative removes both. | **The design keeps the approved contract unless you adopt the simplification.** Trade-off: the alternative accepts a bounded window (one relist period) of serving a withdrawn Route or key after a missed event, in place of a fail-closed admission clear. |
| P-32 (new; R-36) | Add a fault or pause seam to `EbpfDataplane::update_service` (for example an injected pre-commit failure and a hold of the commit guard), so the two runs `abandoned_reservation_rolled_back` and `receipt_waits_for_publish_guard` can carry `#[quint_test]` replays. | A rollback or publish-guard defect that the model proves for the design but no real-kernel test would catch. Reachable production producer: a pre-commit `update_service` failure (map ENOMEM, syscall error), which no test can cause without the seam; the failure is real in principle. | A production fault-injection branch or pause hook whose only caller is a test (against `development.md` § "Production code is not shaped by simulation"), plus replay drivers and Tier-3 time. | Keep R-36: replay `conflicting_alloc_rejected` with its empty-update tail, keep the two runs model-only, and accept the exposure stated in R-21. |
| P-33 (new) | Replace the typed rollback taxonomy (`DataplaneError::ServiceApplyRollback`, `ServiceApplyStage`, `ServiceApplyRollbackFailure` and the `ReverseNatDelete` core conversion; R-35) by a simpler rule: on any pre-commit failure of `update_service`, remove the Reserved identities and the fresh memo entries, keep the prior outer pointer, and return the original stage error; a restore-delete that itself fails goes to the existing `ServiceCleanupDebt` and a warning. Also part of this decision, not decided here: the cleanup-debt retry has three triggers (`update_service`, the probe, shutdown) while the debt set is private and in-memory and dies with the instance, so trimming the triggers is decided with the taxonomy. **R-35 as written stays the contract meanwhile** (it is part of the user-confirmed D5 contract). | (a) None new: it removes a type family, it does not add a defence. The reachable failure it concerns is the same pre-commit `update_service` failure R-21 bounds (map ENOMEM, syscall error, `BACKEND_MAP` capacity). | (b) Runtime: none (cold path). Validation: removes `ServiceApplyRollback`-specific assertions and their equivalence cases; the host/Sim `Dataplane` contract text and R-35 change; ADR-0233/0241 text follows. | Keep the typed taxonomy (R-35 as written): an operator-facing error names the stage that failed and the restore that failed. |
| P-34 (new) | Remove the `overdrive-sim -> overdrive-host` `[dependencies]` edge (R-40) by defining the public-certified-key codec as a trait in `overdrive-gateway::ports`, the same pattern as R-20, so `overdrive-sim` implements the trait and depends on no `adapter-host` crate. **R-40 (edge kept; not a dev-dependency) stays meanwhile.** | (a) None reachable in production: the edge conforms to `development.md` § "Port-trait dependencies" (R-40) and adds no node that `overdrive-sim -> overdrive-gateway -> overdrive-host` does not already reach. | (b) Runtime: none (one more trait object at boot). Validation: one new port trait, a Sim impl and a host impl, and a changed `UnboundGatewayBuilder::new` parameter type; a codec-equivalence test. | Keep the edge (R-40). Adopting P-34 would also let the client-mTLS scenarios S-PIG-38, 43, 52 and 64 live beside the dataplane adapter (they are placed in `overdrive-control-plane/tests/integration` meanwhile, because the seal can be minted only through `UnboundGatewayBuilder::into_builder`, whose `::new` needs the host codec and `overdrive-dataplane` has no `overdrive-host` edge). |
| P-35 (new) | Add a VM-level (N lane) 400 and 421 case to S-PIG-70, so obligation 8's "400/421 end to end" is observed against a real VM Service and a real client. **Not adopted:** 400/421/431/505 are covered by the pure-function S-PIG-82 and the 400, 505, 502 listener arms of S-PIG-77/78, and 413/400/421/431/505 reach a real client in the S-PIG-115 native test (the 408 cases are S-PIG-70). | (a) A mis-mapped 400 or 421 at the real listener that the pure function and the Sim listener arms do not reproduce; production producer: a malformed request or SNI/authority mismatch from a real client. | (b) Runtime: none. Validation: two N-lane cases, about 5 s each plus a native-metal lease and a VM readiness wait (600 s worst case per binary run). | Keep S-PIG-70 as restated (authored 408 cases) and accept pure-function plus listener-arm evidence for 400/421. |
| P-37 (**Rejected by the user, 2026-11-04:** production keeps no pending-request list by design; the reconciler is level-triggered and the action shim executes actions rather than queueing them) | Project the model variable `issues` through a pinned owner accessor (for example a read-only list of the pending issue epochs on the reconciler's emission), so the QC-1 `submit_evaluation` handler's State comparison checks the model value literally instead of the weaker "no pending Issue beyond `desired()`'s epoch". **Rejected; R-31 (model-only `issues`) stands.** | (a) A reconciler that emits a duplicate or early same-epoch Issue which is later completed and so is distinguished from a necessary one only through `audited`; production producer: the host-backed `GatewaySvidLifecycle` re-emitting an Issue on each tick while the slot is empty. | (b) Runtime: none (read-only accessor, no hot path). Validation: one pinned accessor on the emission, a State field, no new model check. | Keep R-31: `issues` model-only, accept the residual exposure bounded by the `audited` comparison. |
| P-38 (**ADOPTED by the user, 2026-11-04; R-53**) | Add route DELETE and gateway status methods to `ApiClient` (the typed operator client), so the operator-surface tests drive delete and status through the client instead of plain HTTPS. **Adopted with the exact signatures in "Exact Route, deploy, configuration and owner surfaces".** The S-PIG-36 DELETE arm is the first caller. The S-PIG-35 withdraw and status read and the S-PIG-42 status read use the new methods too (user ruling 2026-11-05, R-53). | (a) None found: the production handlers are reached over HTTPS today and no CLI verb needs the methods; no production path fails for their absence. The only effect is test fidelity, since plain HTTPS bypasses the client's trust handling and error typing. | (b) Runtime: none. Validation: two client methods with their existing `CliError` taxonomy; no new error variant. | (Superseded by adoption.) |
| P-36 (new) | Remove the `gateway.upstream.receipt.validated` tracing occurrence (its event spec) and AT-PIG-E2E-1's in-process tracing capture layer. The occurrence has no sanctioning ruling found and no production reader; it exists only as a test oracle. **Not adopted; the occurrence stays meanwhile.** | (a) None: no production failure depends on the occurrence; it is read only by AT-PIG-E2E-1. Removing it removes a production emitter, a stable event schema and a capture layer that exist for one test. | (b) Runtime: one emit per upstream request today (removal saves it). Validation: removal requires re-pointing AT-PIG-E2E-1's receipt oracle; no model or lane is added. | The cheaper oracles already pinned: the S-PIG-63 port read, `live_intent_count()` and `live_receipts()` post-state, and the exact-peer SVID plus a distinguishing backend response header (`X-Workload: api`) for the selected backend. |
| P-40 (new) | Derive the `SvidLifecycle` retry attempts from the confirmed failure record (`WorkloadSvidIssueStatusRowV1.issue_failures`, read back into `actual`) instead of stamping `IssueRetry` on emit, and likewise for `GatewaySvidLifecycleView.retry`. **Not adopted; R-50 (View stamped on emit, unchanged) stays meanwhile.** | (a) A failed issue that never reached the executor (a dropped action, a crash between the View fsync and dispatch) is counted as an attempt and backs the next issue off; production producer: a control-plane crash after the runtime fsyncs the View and before the action dispatch (`development.md` § "Reconciler I/O" steps 7-9). The cost is one extra backoff window, not a lost issuance (the next tick re-emits). | (b) Runtime: one observation read per allocation per tick on the reconciler hydration path. Validation: a new `actual` field and read-port for `SvidLifecycle` and `GatewaySvidLifecycle`, `Sim*` impls and changed retry property tests; the View schema drops `attempts` (a CBOR additive change). | Keep the View stamped on emit (R-50): it is retry-policy memory, never an operator surface, and the cost above is bounded to one window. |

Ruled proposals leave this table; the ruled contract is stated once, in the owning ADR and the sections that cite it (R-1..R-61 above; R-52 user-confirmed 2026-11-04; P-41 is ruled as R-55, 2026-11-05). P-37 was rejected by the user on 2026-11-04 and is recorded in the table.

Items the design deliberately does not carry: a userspace backend load balancer, a named Gateway resource, ACME, WireGuard, in-guest mTLS, DNS mutation, any new persistence subsystem.

## Wave: DESIGN / [REF] Backend programming and selection on main (D1, D3)

**Backend programming (D1).** `ServiceLifecycle` publishes the complete `ServiceBackendRow` with `backend_ip = row.workload_addr.unwrap_or(host_ipv4)` and `backend_spiffe = SpiffeId::for_allocation(..)` (`M:crates/overdrive-reconcilers/src/service_lifecycle.rs:888-889`, published `:1064-1068`); the running row carries the guest address assigned by the action shim (`M:crates/overdrive-control-plane/src/action_shim/mod.rs:2455-2458`). `run_server` constructs the hydrator with the legacy `10.99.0.0/16` subnet (`M:crates/overdrive-control-plane/src/lib.rs:5426-5438`) and the hydrator excludes only that subnet (`M:crates/overdrive-reconcilers/src/service_map_hydrator.rs:349-354`), so a `100.95.x.y` backend (`M:crates/overdrive-core/src/guest_transport/mod.rs:594`) is a remote survivor of `Action::DataplaneUpdateService` (`:414`). `EbpfDataplane::update_service` writes `BACKEND_MAP`, the Maglev inner map, `REVERSE_NAT_MAP` and swaps the outer `SERVICE_MAP` slot (`M:crates/overdrive-dataplane/src/lib.rs:1890-1910, 1964, 1997-2028, 2033-2042`); `LOCAL_BACKEND_MAP` is written only by `register_local_backend` (`:2149-2179`). The D4 spike observed exactly this on metal.

**What the gateway still needs from BPF (D3).** `cgroup_connect4_service` reads only `LOCAL_BACKEND_MAP` and allows every miss (`M:crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:78-82`), so a host-originated gateway `connect(2)` to the Service VIP is not selected. A separate `cgroup/connect4` selection program, attached beside `cgroup_connect4_service` (R-1, ADR-0224), serves sockets registered by the dataplane adapter:

1. Lookup key: the socket cookie from `bpf_get_socket_cookie`, which `cgroup/connect4` programs may call (`net/core/filter.c:8199-8200 @ v7.0`) and which returns `__sock_gen_cookie(ctx->sk)` (`net/core/filter.c:5151-5154 @ v7.0`); userspace `getsockopt(SO_COOKIE)` returns `sock_gen_cookie(sk)` of the same generator (`net/core/sock.c:2109-2113 @ v7.0`), so the pre-connect cookie equals the hook's.
2. A socket with no registered intent follows today's `LOCAL_BACKEND_MAP` behaviour byte for byte.
3. A registered socket resolves the intent's `ServiceKey` in `SERVICE_MAP` (outer lookup, NULL-check, then chained inner Maglev lookup indexed by a deterministic hash of the cookie), then `BACKEND_MAP[id]`. Absent outer entry, empty slot or missing backend records `NoBackend` and returns verdict 0 (`healthy` is not consulted, R-2). A returned verdict other than 1 makes `connect(2)` fail with `-EPERM` (`kernel/bpf/cgroup.c:1664-1665, 1694-1696 @ v7.0`). Otherwise it rewrites `user_ip4`/`user_port` to the selected backend and records `Selected(BackendId)`. Any registered-arm decode or map-write failure denies; there is no fall-through to an unchanged VIP.
4. The hook runs in `pre_connect`, before `tcp_v4_connect` performs its route lookup with the post-rewrite destination and the socket's bound source and mark (`net/ipv4/af_inet.c:679-683 @ v7.0`; `net/ipv4/tcp_ipv4.c:206-218, 254-256 @ v7.0`), and against the socket's own cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`).
5. `BACKEND_MAP.healthy` is written by the dataplane from `Backend.healthy` (`M:crates/overdrive-dataplane/src/maps/mod.rs:135`) but no current BPF program reads it. The registered arm does not read it (R-2, P-9): the readiness window (about 1 s after startup `Stable`) is left to the dial and surfaces as 502. The selected-then-failed-dial and never-wrong-peer rules hold.

V-GW-4 measured the arm at 246 verified instructions against 28 for the existing `cgroup_connect4` (7.0.0-38-generic, [findings](spike/findings-gateway-connect-path.md)), so the arm is a second program beside the existing one (R-1, P-10); the existing program and its Tier-4 baseline are unchanged. DELIVER runs the verifier-regression gates (`.claude/rules/bpf.md`, `.claude/rules/testing.md` Tier 4), states the attach mode explicitly (multi-attach; the loader reported `mode=Single` for aya's own call while `bpftool` listed `multi`), and honours the program order contract in ADR-0224: the kernel runs every attached program in array order over the same context and any deny yields `-EPERM` (`kernel/bpf/cgroup.c:79-89 @ v7.0`); an unregistered socket passes the new program unchanged and a registered socket is rewritten by it; the outcome does not depend on attach order because `LOCAL_BACKEND_MAP` holds no VM backend. The XDP wire programs are unchanged. The BPF maps `GATEWAY_CONNECT_INTENT_MAP`/`GATEWAY_SELECTION_RECEIPT_MAP` (8-byte PODs, capacity 128, keyed by host-order cookie, not pinned: their lifetime is the `EbpfDataplane` instance) are declared in `overdrive-bpf` `maps/gateway_connect.rs`.

### Pinned: selection program, baselines and gates (R-54; DISTILL addition, 2026-11-04)

Pinned here under the user's delegation, following the sibling convention `cgroup_<hook>_<role>` (`cgroup_connect4_service`, `cgroup_sendmsg4_service`, `cgroup_recvmsg4_service`):

| Item | Pinned value |
|---|---|
| Program source file | `crates/overdrive-bpf/src/programs/cgroup_connect4_gateway.rs`, declared `pub mod cgroup_connect4_gateway;` in `crates/overdrive-bpf/src/programs/mod.rs` beside `cgroup_connect4_service` |
| Entry function and program name | `cgroup_connect4_gateway`, attribute `#[cgroup_sock_addr(connect4)]`; the name in the compiled object `target/bpf/overdrive_bpf.o` and in `Ebpf::program_mut("cgroup_connect4_gateway")` is the same string |
| Name as the verifier-regress corpus sees it | `cgroup_connect4_gateway` (the baseline's `prog=` value). The binary dispatches on the `cgroup_` prefix to `CgroupSockAddr` (`crates/overdrive-dataplane/bin/verifier_regress.rs`), so the prefix is load-bearing |
| Baseline file | `perf-baseline/main/verifier-budget/veristat-cgroup-connect4-gateway.txt` (convention: the program name with `_` replaced by `-`, prefix `veristat-`, as `veristat-cgroup-connect4-service.txt`). One line, same format as the siblings: `file=target/bpf/overdrive_bpf.o prog=cgroup_connect4_gateway verdict=success verified_insns=<N>`. `<N>` is the first measurement on the pinned 7.0 kernel by DELIVER (`bpf.md`: the pinned kernel is authoritative); V-GW-4's 246 (7.0.0-38, spike program) is an expectation, not a pre-authorized number. Header comments follow the sibling file (budget gates, rationale, first-baseline note); later changes append, never edit (`bpf.md`) |
| How `cargo verifier-regress` finds it | It reads every `*.txt` under `perf-baseline/main/verifier-budget/` and loads each named program from the object; there is no corpus list to edit and no new xtask subcommand. The pre-pinned `SERVICE_MAP` it already creates is the only map the program needs pinned; the two gateway maps are unpinned and aya creates them at load. It fails on more than 5% growth over the baseline, on a program within 10% of the 1M ceiling, and on a baseline naming a program the object lacks. The cgroup siblings carry no inline `within_20pct` test (only the two XDP programs do), so none is added |
| Loader wiring (contract of attach) | `EbpfDataplane` loads and attaches `cgroup_connect4_gateway` at the same `cgroup_attach_path` as its siblings, after `cgroup_recvmsg4_service`, with `CgroupAttachMode::Single` (aya reported `Single` for its own call while `bpftool` listed `multi`; the comment states the mode, as the delta requires). A load or attach failure is `DataplaneError::LoadFailed(String)` naming the program, as for `cgroup_connect4_service`; no new `DataplaneError` variant (`CgroupSendRecvAttach` stays reserved for the send/recv kernel-floor refusal) |
| `user_port` shape | The program writes `user_port = u32::from(port_host.to_be())` and `user_ip4 = ip_host.to_be()` (`development.md` § "`bpf_sock_addr.user_port`"). Per steps 1 to 3 above it keys on the cookie intent and reads neither `user_ip4` nor `user_port`; the READ half of the low-16-NBO hazard has no oracle in this program (S-PIG-124 asserts only the write half: `getpeername` equals the programmed backend, whose port 9000 differs byte-asymmetrically from the frontend port 8080, so a wrong low-16-NBO write misses, and `SO_COOKIE` equality); the read half is covered by the sibling program (S-PIG-41's `7777` VIP) and by the `development.md` rule, and if DELIVER adds any read it uses `u16::from_be(ctx.user_port as u16)` |

**Gates and the evidence tier of each (no Tier-2: `cgroup_sock_addr` has no `BPF_PROG_TEST_RUN`).**

1. **Real-kernel hook behaviour** (S-PIG-124, 125), native metal, `crates/overdrive-dataplane/tests/integration/public_ingress_gateway_hook.rs`, fixtures `HostRig`, `service_key`, `frontend`, `backend`, `dial`, `routing_snapshot` from `tests/integration/helpers/gateway_connect_rig.rs`. The attach target is the root cgroup `/sys/fs/cgroup` exactly as the existing tests (the real `EbpfDataplane::new_with_pin_dir`), and a registered socket is created only by the production `open_upstream`; no test writes a gateway map or an nft rule.
2. **Verifier budget** (S-PIG-127): the baseline file above plus `cargo xtask metal run -- cargo verifier-regress` after `bpf-build`.
3. **Attach/load assertion** (no dedicated scenario): the existing Tier-3 suite asserts a cgroup hook's attachment in two ways: the construction `EbpfDataplane::new_with_pin_dir(..).expect(..)` in `HostRig::build` (a `load()` or `attach()` failure panics every test of the binary), and the behaviour of the hook on a real `connect(2)` (`local_backend_proto_connect.rs`, S-PIG-41). The new program gets the same two. Construction success proves the loader ran without error; the rig builds without the gateway program being exercised, so attachment of `cgroup_connect4_gateway` is proven by S-PIG-124's registered `connect(2)` landing on the programmed backend, not by construction.
4. **Performance**: no existing harness fits a cgroup connect hook (`xdp-perf` and `perf-baseline/main/xdp-perf/` are XDP-only). No lane, gate, baseline or harness binary is created (R-55). One report-only point-in-time measurement during DELIVER; no lane, gate, baseline or harness binary; GH #287 comment URL https://github.com/overdrive-sh/overdrive/issues/287#issuecomment-6652745629; methodology in the roadmap criterion of the selection-hook step.

**Test-support seam for gap 9a (S-PIG-129; test support, no production API).** The gateway integration `World` already takes any `Arc<dyn ObservationStore>` in `World::start_over`. The undecodable prior row is produced by composing the production `LocalObservationStore::open(<redb path>)` (already reachable: the file uses `LocalIntentStore` from the same `overdrive-store-local` crate) over a per-test redb file, after raw bytes were written under the pinned table `observation_gateway_application_status` and key `NodeId::as_str().as_bytes()` (see "Persisted format"). The raw write reuses the existing back-door `write_raw_bytes_to_gateway_application_status_table(redb_path, node, bytes)` of `crates/overdrive-store-local/tests/integration/envelope_helpers.rs` through a `#[path = "../../../overdrive-store-local/tests/integration/envelope_helpers.rs"] mod envelope_helpers;` include in the gateway integration binary (the same `#[path]` precedent as `issue_fault_store`); if that file's imports do not resolve in the gateway binary, the fallback is a ten-line test-local redb writer for the same table and key. The back-door opens its own `Database`, so it runs BEFORE `LocalObservationStore::open` holds the file. `Site` (test-local) gains an `observation_path`. No `SimObservationStore` byte layer and no production accessor is added.

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
// overdrive-gateway::ports
#[async_trait::async_trait]
pub trait GatewayConnectDataplane: GatewayConnectCleanup + Send + Sync {
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
    fn selected_backend_identity(&self, backend_id: BackendId)
        -> Result<SpiffeId, GatewayConnectDataplaneError>;
}
// `GatewayConnectCleanup` (cleanup, cleanup_all_gateway_intents, live_intent_count, live_receipts) is the one normative
// definition of those methods; it is pinned under "Cleanup-only capability (R-29)" below.

/// A socket prepared by the dataplane adapter. Not Clone.
pub struct PreparedUpstream { /* private: tokio::net::TcpSocket + GatewayConnectIntent */ }
impl PreparedUpstream {
    /// Backends only (cross-crate construction contract below). `socket` is the unconnected, marked, loopback-bound
    /// socket; `intent` is the intent the backend has ALREADY registered for it. Validates nothing.
    pub fn new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent) -> PreparedUpstream;
    pub fn intent(&self) -> &GatewayConnectIntent;
    /// Hands the socket to the caller for `connect(2)`; the intent stays registered
    /// until `cleanup(&intent)`.
    pub fn into_parts(self) -> (tokio::net::TcpSocket, GatewayConnectIntent);
}
```

**Cross-crate construction of `PreparedUpstream` (DG-8).** `PreparedUpstream` is defined in `overdrive-gateway::ports`; its
only producers are `open_upstream` implementations in other crates: `EbpfDataplane` (`overdrive-dataplane`),
`SimGatewayConnectDataplane` (`overdrive-sim`) and driver-local doubles (the QC-2 connect double). The value is a pairing of two
things the adapter has already made, so the constructor is plain: `PreparedUpstream::new(socket, intent)`, public, in the block
above. Invariants:
1. **The constructor registers nothing.** Marking, loopback bind, cookie read and `GATEWAY_CONNECT_INTENT_MAP` registration (steps
   1-3) are the producing `open_upstream`'s work and its postcondition ("the only constructor of a registered intent"). `new` does no
   syscall, validates nothing, cannot fail.
2. **The value carries no cleanup guard.** Dropping a `PreparedUpstream`, or the socket from `into_parts`, cleans nothing. Cleanup is
   the connector's one RAII guard calling `cleanup(&intent)` (Gateway upstream path, connector paragraph), never the value.
3. **`intent()` is the only identity.** The connector never reads the cookie off the socket; the intent returned by `intent()` is
   what `take_receipt` and `cleanup` are keyed by. Host: the intent's cookie is the socket's `SO_COOKIE`. A double: whatever
   cookie it minted for the intent it registered.
4. **Only `open_upstream` implementations call `new`.** A review convention, as for the opaque legs (no scan is added).
5. **The socket is always a real unconnected `TcpSocket`, for every backend.** A non-Host backend (Sim, double) creates one with
   `tokio::net::TcpSocket::new_v4()` and need not mark or bind it; the connector's `connect(2)` is a real kernel connect for
   every backend, so what that connect observes is decided by the destination the test gives the Service Frontend, not by the
   backend (consequence for parking: next paragraph).
Alternative rejected for the value: a backend-defined state object behind `Box<dyn Any + Send>` as for `KtlsLeg` (DG-1). The legs
have no concrete contents the gateway may name; `PreparedUpstream`'s contents (a socket and an intent) are exactly what the
connector consumes, so hiding them would only add a downcast. A carried cleanup guard or closure is rejected by invariant 2:
`cleanup` is fallible and its failure must reach the connector's ledger reservation, which a `Drop` cannot do.

**Ledger order (R-5, DG-11).** `open_upstream` is atomic: it creates the socket and registers the intent as one operation, so the
connector cannot interpose between them. The connector therefore reserves the ledger slot first and binds the returned intent to
that slot after `open_upstream` returns. A full ledger refuses before any socket or registration exists; a slot reserved for an
`open_upstream` that fails is released; and once an intent is registered a slot is already held for its cleanup record, so no
registered intent is ever without one.

**Where a double may park a probe (DG-9).** The connector's sequence for `probe` is: connect permit, cleanup-ledger slot reservation (R-5), `open_upstream` (sync),
binding of the returned intent to the slot, `into_parts`, `connect(2)` on that socket to the Service Frontend (real, raced against the injected clock), and `take_receipt` (sync).
For a `Selected` receipt the probe then stops: it performs no `selected_backend_identity` and no `GatewayClientMtls::authenticate` (R-65).
The connector probe therefore awaits no driven-port operation, and `GatewayClientMtls::authenticate`, the former park point, is not reachable from the gate.
The awaited driven-port operations a staged gate still owns are `GatewayConnectDataplane::probe` and `GatewayRelay::probe` (each runs while its
once-per-boot proof is not yet established, so a gate before the first success parks there) and the identity `ensure_current` wait; the connector's `connect(2)` is a kernel call on a real socket.
Consequences, all within the pinned shapes (no production change, no new item):
1. A held-probe scenario parks the gate at an awaited operation above. A failed `GatewayConnectDataplane::probe` is the model's failed gate result (`probe_result(tok, false)`)
   and a successful one its ok result; a held probe no longer requires a `Selected` receipt or a backend. Once the connect dataplane proof has succeeded in a boot,
   a later candidate's gate has no double-controllable await; a scenario that needs a second in-flight candidate parks the connector's `connect(2)` instead,
   by giving the Service Frontend a loopback listener whose accept backlog is full and releasing it by draining the backlog. That fixture is test support (DISTILL's).
2. The Sim and the doubles identify a held entry by the call that parked it (the gate token the owner staged), not by arrival order or by any backend identity.
3. Release is the parked operation returning: ok = `Ok(())`; failed = the typed `GatewayConnectDataplaneError::Unavailable` (the gate reports `ConnectProbe(..)`, status `ConnectPathUnavailable`).
4. The real `connect(2)` must reach a peer that completes it, or fail quickly (the connector still takes the receipt, as the
   outcome table rows 1 and 3 require). A test fixture gives the Service Frontend a loopback address with a listening socket:
   `ServiceVip::new` accepts any `IpAddr` (`crates/overdrive-core/src/id.rs:1067`). That fixture is test support (DISTILL's).
5. A probe parked at the connector's `connect(2)` holds one connect permit (`max_inflight_connects`) until it returns; the number of simultaneously
   held entries stays below that limit.
6. The DESIGN does not say whether the owner keeps awaiting, or drops, the probe of a superseded candidate. Either is
   compatible: release of an entry whose waiter is gone is a no-op for the double, and the QC-2 postcondition is on
   `application_status()` either way.
The Sim hold (`HoldConnectProbe`/`ReleaseConnectProbe`) is realised at the same points. That `SimGatewayConnectDataplane` and
`SimGatewayClientMtls` share the hold state is Sim test support (capabilities, not signatures; DISTILL shapes it, as it does
`set_peer`).

Added error variants: `GatewayConnectDataplaneError::{SocketCreate, SocketOption, SocketBind, CookieRead}` (each carrying the originating `io::ErrorKind`) alongside the branch variants; `IntentRegistryFull`, `IntentAlreadyRegistered`, `IntentMissing`, `ReceiptMissing`, `ReceiptMismatch`, `BackendIdentityMissing`, `Kernel`, `Cleanup`, `Unavailable`, `ProbeTimeout` are unchanged. `SocketCookieReader` becomes a dataplane-internal helper; it is not a boot-adapter field and `UnboundGatewayBuilder::new` no longer takes it.

Intended caller of `open_upstream`: `GatewayUpstreamConnector::{probe, connect}` only. The enforcement is module privacy and review; no lint scans for it (none exists in `xtask/src/dst_lint.rs` for this port). The connector module holds the `Arc<dyn GatewayConnectDataplane>` as a private field and is the only holder of a handle that can call `open_upstream`: the shutdown path holds a `GatewayConnectCleanup` capability (R-29, below), which has no `open_upstream`. The sentinel probe's own connect is made inside the dataplane adapter on its own prepared socket.

**Cleanup-only capability (R-29, P-29).** The cleanup sweep, the cleanup of one intent and the live counts are a separate port that `GatewayConnectDataplane` extends, so one adapter implements both and shutdown holds only the narrow one:

```rust
// overdrive-gateway::ports
pub trait GatewayConnectCleanup: Send + Sync {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError>;
    fn cleanup_all_gateway_intents(&self) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError>;
    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError>;
    /// Read-only snapshot of the receipts the hook has recorded and `take_receipt` has not yet consumed, by the cookie
    /// of the intent each is bound to (R-33). It mutates nothing and consumes nothing. The one receipt accessor: the
    /// residual receipt count is `live_receipts()?.len()`. It is the observation accessor of the QC-3 `receipts`
    /// projection; the connector never calls it.
    fn live_receipts(&self) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError>;
}
// GatewayConnectDataplane: GatewayConnectCleanup + Send + Sync keeps probe, open_upstream, take_receipt and
// selected_backend_identity (block above); it is the single normative pinning of both traits.
// `GatewayBuilder::start` obtains the `Arc<dyn GatewayConnectCleanup>` for `GatewayShutdownCapabilities.cleanup` by trait
// upcasting the `Arc<dyn GatewayConnectDataplane>` it receives in `GatewayRuntimeDependencies` (stable on the pinned 1.95 toolchain).
```

`EbpfDataplane` and `SimGatewayConnectDataplane` implement both traits; `ComposedDataplane.gateway_connect` stays `Arc<dyn GatewayConnectDataplane>`. The connector still calls `cleanup` through its own handle; the cleanup ledger's retry at shutdown calls it through the narrow one.

## Wave: DESIGN / [REF] Kernel relay data path and driven ports (G1)

User ruling G1 (2026-11-02) replaces the userspace `hyper` data path. Decisions:
[ADR-0244](../../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md)
(kernel TLS after a userspace handshake),
[ADR-0245](../../product/architecture/adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
(head parsing and framing),
[ADR-0239](../../product/architecture/adr-0239-stream-public-proxy-bodies-with-backpressure.md)
(bodies move kernel-side),
[ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md)
(the pipe invariant),
[ADR-0247](../../product/architecture/adr-0247-gateway-exchange-is-full-duplex.md)
(full-duplex exchange) and its single-decision companions
[ADR-0249](../../product/architecture/adr-0249-gateway-legs-offer-only-aes-gcm-tls13-suites.md) (cipher set),
[ADR-0250](../../product/architecture/adr-0250-gateway-never-originates-a-tls-keyupdate.md) (no KeyUpdate origination),
[ADR-0251](../../product/architecture/adr-0251-gateway-rejects-ambiguous-http-framing.md) (ambiguous-framing rejection),
[ADR-0252](../../product/architecture/adr-0252-gateway-forwards-interim-100-and-expect.md) (interim `1xx` and `Expect`),
[ADR-0253](../../product/architecture/adr-0253-gateway-supports-close-delimited-and-http10-length-responses.md) (close-delimited and HTTP/1.0 responses) and
[ADR-0254](../../product/architecture/adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md) (lingering close). Every contract below is the SSOT; the ADRs carry no signatures.
Spike evidence: [findings-ktls-splice-relay.md](spike/findings-ktls-splice-relay.md) (items a-e). Backend selection is
unchanged: the cgroup/connect4 arm still selects before the TLS handshake (item d: a pre-connect rewrite composes with the
later handshake and kTLS install on the same fd).

### What runs where

| Step | Where | Owner |
|---|---|---|
| Public TLS 1.3 handshake, SNI/ALPN, key custody use | userspace, rustls unbuffered API | `GatewayRelay::accept_public` (host: `overdrive-dataplane`) with config from the custody snapshot |
| Upstream mTLS handshake, exact-peer SAN equality | userspace, rustls unbuffered API | `HostGatewayClientMtls` (unchanged owner, ADR-0234) |
| TLS record layer of both legs | kernel (kTLS TX+RX) | armed by the same host relay from `rustls::kernel::KernelConnection` |
| Request/response heads, chunk-size lines, trailer lines | userspace, read through the terminator only | `GatewayRelay::{read_through, write_all}` driven by the gateway exchange |
| Framing decision, Route match, header policy, limits, 4xx/5xx mapping | userspace, pure functions | `overdrive-gateway::runtime::http1` (crate-private) |
| Body octets | kernel, `splice(2)` through a private pipe | `GatewayRelay::{relay_exact, relay_to_end}` |
| KeyUpdate, NewSessionTicket, close_notify, alerts | userspace, per leg, never forwarded | the host relay's leg backend (ADR-0244) |
| Backend selection | kernel BPF (unchanged) | ADR-0224 |

### Component ownership and crate class (ADR-0003)

| Component | Home / class | Contract shape and universe | Assertion mechanism |
|---|---|---|---|
| `http1` head validator, request/response parser, `BodyFraming` decision, chunk-line parser, `rewrite_*_headers`, `normalize_route_authority` | `overdrive-gateway::runtime::{http1,routing,proxy_headers}` / `adapter-host` crate, pure modules (no I/O, no clock) | **pure-function**: bytes and limits to typed value or typed error | per-function PBT in source-local `#[cfg(test)]` modules; mutation-test target |
| `ExchangeMachine` | `overdrive-gateway::runtime::http1::exchange` / pure | **pure-function** state machine: (state, event) to (state, disposition) | Quint conformance (below) + PBT |
| Exchange driver (two concurrent halves per request) | `overdrive-gateway::runtime::exchange` | **bounded-change**: the exchange's two sockets, its leases, its deadlines; no other state | seeded `overdrive-sim` against `SimGatewayRelay`; Tier-3 wire tests |
| `GatewayRelay` port, `KtlsLeg`/`KtlsReadHalf`/`KtlsWriteHalf`, error taxonomy | `overdrive-gateway::ports` | trait and opaque values only | host/Sim equivalence test (`gateway_relay_equivalence.rs`, over faults the Host can also produce) |
| `HostGatewayRelay`: kTLS arming, control-record dispatch, rekey, private pipe, splice, probe | `overdrive-dataplane::gateway_relay` / `adapter-host` (extends `mtls/ktls.rs`; owns the `unsafe` `setsockopt`/`recvmsg` calls; `overdrive-gateway` stays free of them) | **bounded-change**: per leg, its socket, its pipe, its key epochs | Tier-3 on native metal: wire bytes at a real rustls peer, `/proc/net/tls_stat` deltas, negative controls |
| `SimGatewayRelay` | `overdrive-sim::adapters` / `adapter-sim` | same observable contract, userspace copy between in-memory record queues | equivalence test; fault surface listed below |
| `HostGatewayClientMtls` | `overdrive-dataplane::mtls::gateway_client` (unchanged owner) | arms the authenticated socket through the same `HostGatewayRelay` | wrong-valid-peer matrix (unchanged) |

dst-lint scans only `core` crates; the relay is host I/O and is not scanned. `overdrive-gateway` gains `httparse` and `http`
as direct dependencies and drops `hyper`, `hyper-util` and `tokio-rustls` (the operator HTTPS listener and probes keep
`hyper`); `overdrive-dataplane` already depends on `rustls` and `libc`.

### Driven port `GatewayRelay` (cross-crate contract)

```rust
// overdrive-gateway::ports
/// Opaque, non-Clone kernel-TLS leg: a TCP socket on which TLS 1.3 TX and RX are armed.
pub struct KtlsLeg { /* private */ }
impl KtlsLeg { pub fn into_halves(self) -> (KtlsReadHalf, KtlsWriteHalf); }
/// Receive half. Owns at most one private pipe (ADR-0246): created on the half's first `relay_*` call, released when the
/// half starts its next `read_through(HeaderBlockEnd, ..)` or is dropped (Contract of the legs, item 7). Every splice-in
/// goes into an empty pipe of at least 64 KiB until GH #339 (kernel 7.3 upgrade) lands (R-17).
pub struct KtlsReadHalf { /* private */ }
/// Transmit half. Shares the leg's transmit lock with the read half (the read half answers a peer KeyUpdate).
pub struct KtlsWriteHalf { /* private */ }
pub struct AcceptedPublicLeg { pub leg: KtlsLeg, pub sni: PublicHostname }

pub enum HeadTerminator { HeaderBlockEnd, LineEnd }
pub struct ThroughBytes(Vec<u8>);          // as_slice() only; includes the terminator
pub struct RelayDeadlines { pub absolute: Instant, pub no_progress: Duration }
/// `first_octet`: the absolute deadline for the first octet of the item;
/// `complete_within`: how long the item may take once its first octet has arrived (`None`: no such budget);
/// `complete_by`: an absolute ceiling on the whole item, first-octet wait included (`None`: no ceiling).
/// The port applies the earliest of the three and clamps nothing else: the caller (the relay driver) passes every
/// value already reduced to `min(phase budget, request_deadline)`, so an item can never outlive its request.
pub struct ReadDeadlines { pub first_octet: Instant, pub complete_within: Option<Duration>,
    pub complete_by: Option<Instant> }

#[async_trait::async_trait]
pub trait GatewayRelay: Send + Sync {
    /// Earned-Trust probe (see below). Called once per boot, in gate G-1, before the listener binds.
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError>;
    /// Completes the TLS 1.3 server handshake on `accepted` with `tls`, then arms kTLS TX+RX. `tls` is
    /// `PublicCertifiedKeySnapshot::server_config()`, the custody snapshot's single builder (ADR-0226:
    /// TLS 1.3 only, ALPN `http/1.1`, no tickets, no 0-RTT, secret extraction enabled, the cipher-suite set
    /// of the Contract of the legs); there is no wrapper type.
    async fn accept_public(&self, accepted: OwnedFd, tls: Arc<rustls::ServerConfig>, deadline: Instant)
        -> Result<AcceptedPublicLeg, PublicHandshakeError>;
    /// Consumes application octets through and including the first terminator, and no octet beyond it.
    /// Fails `Deadline` if no octet arrives by `first_octet`, if the terminator has not arrived `complete_within` (if `Some`) after that
    /// first octet, or if it has not arrived by `complete_by` (if `Some`).
    /// A call that starts a `HeaderBlockEnd` read also releases the half's pipe, if any (item 7). A close_notify
    /// pending before the first octet is `CleanEof`. Control records met on the way are handled (never
    /// returned); a partial head is retained by the half, not lost, including when the returned future is dropped before it
    /// completes (cancel-safe: the driver re-issues the call with new deadlines, R-37). Fails `LimitExceeded` when `limit`
    /// octets contain no terminator.
    async fn read_through(&self, from: &mut KtlsReadHalf, terminator: HeadTerminator,
        limit: NonZeroUsize, deadlines: ReadDeadlines) -> Result<ThroughBytes, LegReadError>;
    /// Writes every octet or fails; one writer at a time per half (`&mut`).
    async fn write_all(&self, to: &mut KtlsWriteHalf, bytes: &[u8], deadline: Instant)
        -> Result<(), LegWriteError>;
    /// Moves exactly `len` application octets from `from` to `to` by splice. Reads nothing beyond `len`.
    async fn relay_exact(&self, from: &mut KtlsReadHalf, to: &mut KtlsWriteHalf, len: NonZeroU64,
        deadlines: RelayDeadlines) -> Result<(), RelayError>;
    /// Moves octets until the source's close_notify, failing `LimitExceeded` after more than `max`. Returns the count.
    /// Serves close-delimited responses and HTTP/1.0 responses with an explicit length (R-12, P-13d).
    async fn relay_to_end(&self, from: &mut KtlsReadHalf, to: &mut KtlsWriteHalf, max: NonZeroU64,
        deadlines: RelayDeadlines) -> Result<u64, RelayError>;
    /// Sends close_notify, then shuts down the transmit direction.
    async fn close_write(&self, to: &mut KtlsWriteHalf, deadline: Instant) -> Result<(), LegWriteError>;
    /// R-14 (P-18), R-22 (P-22). Reads and drops application octets from `from` until the peer's close_notify (`Ok`), `max`
    /// octets (`Ok`), or `deadlines` (`Deadline`). Returns no count: the only caller, the lingering close, closes the
    /// socket on every outcome and needs none. Control records are handled as in `read_through`. It sends nothing to the
    /// source peer and does not end its leg (the caller's `close_write` already sent close_notify); a peer close_notify
    /// met on the way is the `Ok` end, handled as `read_through` handles control records. Used only by the
    /// lingering close.
    async fn discard_remaining(&self, from: &mut KtlsReadHalf, max: NonZeroU64, deadlines: RelayDeadlines)
        -> Result<(), LegReadError>;
}

pub enum RelayProbeError { Arm(KtlsArmError), Pipe(RelayPipeError),
    SpliceExactness { moved: u64, expected: u64 }, Timeout, Io { kind: std::io::ErrorKind } }
pub enum PublicHandshakeError { Timeout, Tls { alert: Option<u8> }, AlpnMismatch, ServerNameMissing, Arm(KtlsArmError), Io { kind: std::io::ErrorKind } }
pub enum KtlsArmError { UlpInstall { errno: i32 }, CryptoInfo { errno: i32 }, CipherNotSupported, KernelConnection }
/// A peer-initiated KeyUpdate that the relay failed to answer (the gateway never originates one).
pub enum KeyUpdateError { RustlsRefused, RearmRx { errno: i32 }, RearmTx { errno: i32 },
    Io { kind: std::io::ErrorKind } }
pub enum LegReadError { CleanEof, Truncated, Alert { description: u8 }, UnexpectedRecord { content_type: u8 },
    KeyUpdate(KeyUpdateError), LimitExceeded { limit: usize }, Deadline, Io { kind: std::io::ErrorKind } }
pub enum LegWriteError { PeerClosed, Alert { description: u8 }, KeyUpdate(KeyUpdateError), Deadline,
    Io { kind: std::io::ErrorKind } }
pub enum RelayError { Read(LegReadError), Write(LegWriteError), SourceEndedMidBody { remaining: u64 },
    LimitExceeded { max: u64 }, NoProgress, Deadline, Pipe(RelayPipeError) }
/// The pipe floor is a typestate fact (ADR-0246); the only runtime outcomes are a failed creation or a
/// capacity that reads back below the floor.
pub enum RelayPipeError { Create { kind: std::io::ErrorKind }, CapacityBelowFloor { got: usize } }
```

A half is only ever built by the backend that owns the leg, so there is no wrong-backend half to report: composition builds the
relay and the mTLS client from one backend (`GatewayAdapters::host`, `SimGatewayAdapters`); a half handed to a different
backend is a wiring defect the implementation treats as `unreachable!()` (`.claude/rules/rust.md`). The "empty before splice-in"
and "len at most capacity" laws are typestate on the pipe value, not runtime errors.

`GatewayUpstream` (the proof that the peer was authenticated and the leg armed) is re-pinned:

```rust
// overdrive-gateway::ports (replaces GatewayByteStream and the Pin<Box<dyn ..>> forms)
pub struct GatewayUpstreamSeal { _private: () }
pub struct GatewayUpstream { leg: KtlsLeg }
impl GatewayUpstreamSeal { pub fn seal_authenticated(&self, leg: KtlsLeg) -> GatewayUpstream; }
impl GatewayUpstream { pub fn into_halves(self) -> (KtlsReadHalf, KtlsWriteHalf); }
// GatewayClientMtls::authenticate is unchanged in shape; GatewayClientMtlsError gains Arm(KtlsArmError).
```

#### Cross-crate construction of the opaque leg values

`KtlsLeg`, `KtlsReadHalf`, `KtlsWriteHalf` and `ThroughBytes` are defined in `overdrive-gateway::ports` but their only
producers are backends in other crates: `HostGatewayRelay` (`overdrive-dataplane`, `adapter-host`) and `SimGatewayRelay`
(`overdrive-sim`, `adapter-sim`). `overdrive-gateway` is `adapter-host` (ADR-0003) and both backend crates depend on it for the
port, never the reverse. The port is used as `Arc<dyn GatewayRelay>`, so it cannot carry associated types or generics. The
contract is therefore: **each opaque value owns a backend-defined state object behind a trait object; a backend builds a leg
from its own state and recovers that state by downcast inside its own port methods.** The gateway crate never inspects the state.

```rust
// overdrive-gateway::ports
impl KtlsLeg {
    /// The one constructor. `read` and `write` are the backend states of the two halves (host: socket, key epochs, pipe;
    /// sim: in-memory record queue); the two share the leg's transmit lock (Contract of the legs, item 5) inside the
    /// backend. The caller guarantees TLS 1.3 TX and RX are armed; the type has no other way to exist, so "a leg is
    /// never usable unarmed" (item 1) holds by construction. `into_halves` hands the two states back, wrapped.
    pub fn from_armed(read: Box<dyn Any + Send>, write: Box<dyn Any + Send>) -> KtlsLeg;
}
// KtlsReadHalf and KtlsWriteHalf have no public constructor: only `KtlsLeg::into_halves` makes them.
impl KtlsReadHalf  { pub fn backend_mut<T: Any>(&mut self) -> &mut T; }   // unreachable!() on a T the backend did not store
impl KtlsWriteHalf { pub fn backend_mut<T: Any>(&mut self) -> &mut T; }   // likewise
impl ThroughBytes  { pub fn new(bytes: Vec<u8>) -> ThroughBytes; pub fn as_slice(&self) -> &[u8]; }
```

Invariants:
1. **Single backend per composition.** `GatewayAdapters::host` (and `SimGatewayAdapters`) builds one relay and builds the mTLS client
   factory from that same instance, so every leg and half the gateway holds came from the relay it calls. `backend_mut::<T>` with
   a `T` other than the producing backend's state type is a wiring defect and is `unreachable!()` (`.claude/rules/rust.md`); no
   `Result` and no wrong-backend error variant exists.
2. **Only the two backends construct, and the QC-2 driver's client double.** `KtlsLeg::from_armed` is called only from
   `overdrive-dataplane::gateway_relay` (after a successful arm), `overdrive-sim::adapters::gateway`, and the QC-2 driver's
   test-local `GatewayClientMtls` double (it seals an inert leg for a request-path `authenticate`; no half is ever used);
   `ThroughBytes::new` likewise, minus the driver. This is a review convention; an automated scan for it would be new tooling and
   is not added here.
3. **A half carries no capability beyond its backend state.** The gateway crate can move a half and pass it by `&mut` to the
   port; it cannot read, clone or fabricate one. `GatewayUpstream`/`AcceptedPublicLeg` hold a `KtlsLeg` and gain nothing.
4. `ThroughBytes` includes the terminator (port doc); `new` does not validate that, the backend upholds it and the host/Sim
   equivalence test compares it.

### Contract of the legs

1. **Arming order.** Public: handshake complete, SNI and ALPN accepted, then arm. Upstream: handshake complete, exactly one
   URI SAN equals the receipt-selected `SpiffeId`, then arm (a failed equality arms nothing and sends no octet). The unbuffered
   handshake reads one TLS record at a time so no application octet is consumed before arming. A leg is never usable unarmed.
   The arm installs `TLS_TX`/`TLS_RX` from the connection's `KernelConnection` (`dangerous_extract_secrets` is not used).
   **Cipher suites.** Both legs offer only the TLS 1.3 suites the kernel's kTLS arming accepts and the spike exercised: the
   spiked set is AES-128-GCM and AES-256-GCM, and the offer is exactly that set (R-8, P-12; ChaCha20-Poly1305 is not offered, so a
   client offering no AES-GCM suite is refused before arming).
2. **No sockmap.** No leg is ever a sockmap/sockhash member (`net/tls/tls_main.c:636-645 @ v7.3`; accepted in one order only at
   v7.0, findings (e)).
3. **Receive.** Application octets are read with `recvmsg` (heads and framing lines: `MSG_PEEK` to find the terminator, then a
   `recv` of exactly that many octets; `MSG_PEEK` returns octets of one record type only and stops at a non-DATA record,
   `net/tls/tls_sw.c:1849-1862, 2208-2211 @ v7.0`) or by splice. Because a peek cannot pass a control record
   (`tls_sw.c:2089-2090 @ v7.0`), a partial head is consumed into the half's accumulator before the control record is handled.
4. **Control records** (from `recvmsg` with a control buffer of at least `CMSG_SPACE(1)`, else `-EIO`,
   `net/tls/tls_sw.c:1854-1858 @ v7.0`; or after `splice` returns `-EINVAL`, `tls_sw.c:2310-2314 @ v7.0`, which leaves the record
   readable and is sticky):
   - handshake/KeyUpdate: feed the record to the `KernelConnection`; re-arm RX with `setsockopt(TLS_RX)` (until then reads fail
     `-EKEYEXPIRED` and `poll` reports no read event, `tls_sw.c:1369-1371`, `net/tls/tls_main.c:423-427` (`tls_sk_poll` clears
     `EPOLLIN` while `key_update_pending`), `Documentation/networking/tls.rst:217-220`, all @ v7.0);
     when the peer requested an update, send `18 00 00 01 00` as record type 22 with `TLS_SET_RECORD_TYPE` under the old key
     (`tls_main.c:228-268 @ v7.0` flushes any open record first), then re-arm TX. Never forwarded; each leg rekeys alone.
   - handshake/NewSessionTicket: discarded.
   - alert close_notify: end of that direction (`CleanEof`); the relay half-closes the other leg after the message in flight is handled.
   - any other alert: `Alert`. Any other record type or a malformed handshake record: `UnexpectedRecord`; both legs close.
5. **Transmit rekey is serialised** with every writer of that socket. Each kernel call takes the socket lock, so one call is
   atomic; the kernel does not serialise a multi-call logical write (a `write_all` or a splice-out step spanning several calls)
   against a `setsockopt(TLS_TX)` re-arm (`net/tls/tls_main.c:635-783 @ v7.0`). One lock per leg is shared by the write half
   and the read half; a write holds it for the whole `write_all`/splice-out step, a rekey holds it for reply plus re-arm.
   The lock exclusion is covered by assumption A-K5 and the Tier-3 check (obligation 16(b)); no model covers it (R-19).
6. **The gateway never originates a KeyUpdate** (R-13, P-17). Only a peer-initiated KeyUpdate is handled
   (item 4). The upstream leg terminates at leg-C, which never originates one (assumption A-K7): leg-C's rustls `ServerConnection` is
   consumed by `dangerous_extract_secrets` (`crates/overdrive-dataplane/src/mtls/inbound.rs:503-524`), the kernel is armed once
   by `arm_ktls_tx_rx` (`mtls/ktls.rs:39-48`, `TLS_TX`/`TLS_RX` set once, no re-arm path), and no code in `mtls/` calls
   `refresh_traffic_keys` or re-arms a socket. After arming, no TLS state machine remains on leg-C that could originate a
   KeyUpdate. The gateway does not rely on this for correctness: the upstream read half uses the same item-4 handling as the
   public read half, so a KeyUpdate that did arrive would be answered. The Tier-3 lock check drives the public leg, because a peer KeyUpdate there is the case the suite can drive.
7. **Pipe and splice** (ADR-0246): a read half's pipe is private, set to at least 64 KiB and read back; a splice into it happens
   only when it is empty, with `len = min(remaining, capacity)`; it is drained to the destination before the next splice-in.
   The empty-pipe invariant stays until [GH #339](https://github.com/overdrive-sh/overdrive/issues/339) (the kernel 7.3 upgrade) lands (R-17).
   The destination socket is `O_NONBLOCK`; a short count or `EAGAIN` on the splice out is backpressure and the relay waits for
   writability (`fs/splice.c:874-888 @ v7.0`, nonblocking only via the socket's own flag).
   **Pipe lifetime (one rule).** A read half's pipe is created lazily on the half's first `relay_*` call and released when that
   half starts its next `read_through(HeaderBlockEnd, ..)` (the next message's head) or is dropped. No operation is added to the
   port. The upstream half is per exchange, so its pipe closes with the exchange; the public half holds its pipe from its first
   body relay until its next head read begins, so a keep-alive connection waiting
   for its next request holds no pipe. Chunk-size and trailer lines are `LineEnd` reads and do not release it. Holding the pipe
   until connection close (P-16) is not adopted, and no explicit per-exchange release operation exists (R-16).
   **No `SPLICE_F_MORE`.** The relay never passes it. The kernel sets `MSG_MORE` itself within one splice-out call only when
   that call's batch of at most 16 pipe buffers leaves octets of the same call in the pipe (`fs/splice.c:799, 877-878 @ v7.0`),
   so the last `sendmsg` of every call carries no `MSG_MORE`, and kTLS then closes the record
   (`eor = !(msg->msg_flags & MSG_MORE)`, `net/tls/tls_sw.c:1046 @ v7.0`). No record stays open between calls, so no idle
   connection holds received octets unsent. Wire cost, stated and not measured: a body that
   trickles in small pieces is sent as small TLS records (one record per splice-out call at least); no CPU claim is made. This configuration is not the one the spike ran: findings (c) used `SPLICE_F_MORE` while `remaining > 0`, so
   Tier-3 obligation 16(j) runs the exact-length, chunked, trickle and keep-alive cases in the no-flag configuration. A pipe that
   cannot be created or that reads back below the floor fails the relay with `RelayPipeError` before any octet moves.
8. **Progress and deadlines.** `no_progress` restarts whenever octets move; `absolute` never restarts. Both are evaluated on the
   injected `Clock`.

### Message-framing contract (ADR-0245; replaces hyper)

**Deadline composition (stated once).** Every `read_through` the relay driver issues carries `ReadDeadlines` whose values are
`min(phase budget, request_deadline)`, where `request_deadline` is the request's first octet plus `whole_request_deadline`
(ADR-0238); the port does no clamping of its own. Phase budgets:
- *Request head, first request of a connection:* `first_octet = complete_by = now + request_head_deadline` (so the whole head,
  first-octet wait included, takes at most `request_head_deadline`; there is no 2x), `complete_within = None`.
- *Request head, later request (keep-alive):* `first_octet = now + keep_alive_idle_deadline`, `complete_within =
  Some(request_head_deadline)`, `complete_by = None` (the first octet starts the request, and `request_head_deadline` is
  below `whole_request_deadline`). Idle expiry before the first octet closes the connection silently; it is not a 408.
- *Chunk-size, chunk-terminator and trailer lines:* `first_octet = now + body_no_progress_deadline`, `complete_within =
  Some(body_no_progress_deadline)`, `complete_by = Some(request_deadline)`.
- *Response head (interim heads included; R-37):* `t0` is the instant **the upstream request is completely written** (the
  request head, and the body if there is one; for a request with no body, the instant the head write completes). The response
  half starts reading when the upstream request head is written (full-duplex), before `t0` when a body is still in flight. While
  the request half is unfinished the head is bounded by the request's own deadlines: `first_octet = complete_by =
  request_deadline`, `complete_within = Some(response_head_deadline)` (an origin that answers early must finish its head within
  `response_head_deadline` of its first octet). From `t0`: `first_octet = complete_by = min(t0 + response_head_deadline,
  request_deadline)`, `complete_within = Some(response_head_deadline)`. The port applies the earliest of the three, so a head
  whose first octet arrived before `t0` is also bounded from that octet. An origin that is silent after the request is complete
  therefore costs at most `response_head_deadline` (and never more than the request's remaining `whole_request_deadline`);
  an origin still reading a slow upload is never cut off by `response_head_deadline`, only by `body_no_progress_deadline` and
  `whole_request_deadline`. The driver re-issues the response-head read at `t0` with the second set of values; `read_through`
  retains a partial head, so a re-issue loses nothing (Contract of the legs, port `read_through`).

**Head reading.** `read_through(HeaderBlockEnd, max_request_head_bytes, ReadDeadlines)` with the request-head values above.
The one validator is applied to
every line the gateway reads (head, chunk-size line, trailer line): it rejects bare CR, bare LF (no bare-LF tolerance), NUL, and
field values containing octets outside RFC 9110 §5.5 before the parser runs; the parser receives a header array of `max_*_headers`
entries; an obsolete line fold is rejected. Framing is derived from the ORIGINAL headers, before any
`Connection`-nominated field is removed by the header policy (a `Connection: Content-Length` nomination must not delete the
framing header). The framing rows below are the strict ambiguous-framing rejection set (R-9, P-13a) unless marked otherwise; ADR-0245 holds the decision and this table is the SSOT.

**Request rules (all rejections are before any upstream octet):**

| Condition | Result |
|---|---|
| Not HTTP/1.1, or `HTTP/1.0` | 505 |
| Method `CONNECT`, or non-origin-form target | 400 |
| Host missing / duplicate / invalid / userinfo / port other than 443 | 400; Host differs from SNI: 421 (ADR-0236 unchanged) |
| `Transfer-Encoding` and `Content-Length` both present | 400 |
| More than one `Content-Length` line, or a value that is not decimal digits, or a value that overflows `u64` | 400 |
| More than one `Transfer-Encoding` line, `chunked, chunked`, a `Transfer-Encoding` whose final coding is not `chunked`, or any other coding | 400 |
| Chunk-size hex that overflows `u64`, or a malformed chunk-size line | 400 before the response head, else both legs close |
| Declared length above `max_request_body_bytes` | 413 |
| `Expect` (any value; R-11, P-13c) | forwarded to the origin unchanged; the gateway decides nothing; the origin's interim `100` is forwarded to the client (below), and any other answer, including 417, is the origin's response |
| Head above `max_request_head_bytes` / more than `max_request_headers` fields | 431 |
| No complete head within `request_head_deadline` before any response (first request: from handshake completion; later request: from its first octet) | 408 |

**Request body.** None (no CL, no TE): nothing relayed. `Content-Length: n`: head forwarded with `Content-Length: n`, then
`relay_exact(n)` (n=0 relays nothing). Chunked: per chunk, `read_through(LineEnd, max_request_head_bytes)` (the head ceiling bounds every framing line; no separate limit exists; every framing-line read uses the framing-line values of the deadline composition above) parses the size
(hex, extensions discarded), the gateway writes `<size>\r\n`, `relay_exact(size)`, then `read_through(LineEnd)` must return
`\r\n`, which the gateway writes; the last chunk's trailer lines are read (cumulative at most `max_request_head_bytes`) and
dropped, then the gateway writes `0\r\n\r\n`. The running total above `max_request_body_bytes` is 413 if no public response
octet was sent, else both legs close. Upstream request heads carry `Connection: close` and the gateway-authored framing header.

**Response rules.** `read_through(HeaderBlockEnd, max_response_head_bytes, ReadDeadlines)` with the response-head values of the
deadline composition above: the response head, interim heads included, has one budget measured from `t0`, the instant the upstream request is completely written (R-37). An interim `100` head (R-11, P-13c) is forwarded to the client as a rewritten head with no body;
every other interim `1xx` other than `101` is read and dropped. Interim heads count against the same cumulative head limit and
deadline. `101`, a status line that is not `HTTP/1.1` (or `HTTP/1.0` with an explicit length, R-12), the framing
rejections above applied to the response (both CL and TE, repeated CL, repeated or unsupported coding, overflow), a declared
length above `max_response_body_bytes`, head overflow, and any failure or deadline before the public response head are 502 (503
only for `NoBackend`). Body length of the final response: `HEAD` request, `204`, `205`, `304`: none; `Transfer-Encoding: chunked`:
chunked; `Content-Length: n`: n; otherwise close-delimited (R-12, P-13d). Chunked responses are forwarded as chunked; close-delimited
responses are forwarded with `Connection: close` and `relay_to_end(max_response_body_bytes)`. The public response head is the
rewritten head ([ADR-0237](../../product/architecture/adr-0237-public-proxy-header-policy.md)); the gateway alone emits framing
headers.

**Upstream leg per request.** The upstream request head carries `Connection: close` and one request is sent per upstream leg.
This follows [ADR-0240](../../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md) (one attempt per
accepted public request) and [ADR-0232](../../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md) (one
registered intent per socket); the downstream public connection still keeps alive (R-15). Cost: one exact-peer mTLS handshake and one
cookie registration per request.

**After the public response head is sent**, any failure, deadline, overflow or mid-body end closes both legs without a further
response (`SourceEndedMidBody` and a close_notify before the declared end are failures, never a clean end).

### Pure `http1` and relay-pipe functions (R-32; crate-private, pinned for source-local tests)

The framing, head and deadline rules above are decided by these pure functions. Their signatures and error taxonomies are
the interface the source-local tests of S-PIG-76, 77, 78, 79, 80, 81, 82, 83 (U half) and 85 call; their bodies, helpers and
internal types are the crafter's. No function reads a clock, a socket or a global.

```rust
// overdrive-gateway::runtime::http1 (crate-private)
/// Byte-level validator applied to every line the gateway reads: a request or response head (the whole header block), a
/// chunk-size line, a chunk terminator, a trailer line. Runs before any parser.
pub(crate) fn validate_lines(bytes: &[u8]) -> Result<(), LineViolation>;
pub(crate) enum LineViolation { BareCr, BareLf, Nul, OctetOutsideFieldValueRange, ObsoleteLineFold }

/// Request head -> typed value. `sni` is the handshake's server name (Host/SNI equality, ADR-0236).
pub(crate) fn parse_request_head(bytes: &[u8], sni: &PublicHostname, limits: &GatewayLimits)
    -> Result<ParsedRequestHead, RequestHeadError>;
pub(crate) struct ParsedRequestHead { /* method, origin-form target, original HeaderMap, Host authority, framing */ }
impl ParsedRequestHead { pub(crate) fn framing(&self) -> BodyFraming; pub(crate) fn method_is_head(&self) -> bool; }
pub(crate) enum RequestHeadError {
    Line(LineViolation), VersionNotHttp11, Http10, Connect, NonOriginFormTarget,
    HostMissing, HostDuplicate, HostInvalid, HostUserinfo, HostPortNot443, HostSniMismatch,
    Framing(FramingFault), DeclaredLengthAboveCeiling, HeadTooLarge, TooManyHeaders,
}
impl RequestHeadError { /// 400, 413, 421, 431 or 505 per the Request rules table.
    pub(crate) fn status(&self) -> http::StatusCode; }

/// Response head -> typed value. `request_was_head` selects the no-body rule of the response.
pub(crate) fn parse_response_head(bytes: &[u8], request_was_head: bool, limits: &GatewayLimits)
    -> Result<ParsedResponseHead, ResponseHeadError>;
pub(crate) enum ParsedResponseHead {
    /// An interim head: `100` (forwarded) or another `1xx` other than `101` (dropped).
    Interim { status: http::StatusCode, forward: bool },
    Final(ParsedFinalResponseHead),   // status, original HeaderMap, framing
}
impl ParsedFinalResponseHead { pub(crate) fn framing(&self) -> BodyFraming; }
pub(crate) enum ResponseHeadError { Line(LineViolation), Switching101, VersionNotAccepted, Framing(FramingFault),
    DeclaredLengthAboveCeiling, HeadTooLarge, TooManyHeaders }   // every variant is a 502 before the public head

/// The one framing decision (RFC 9112 section 6.3), over the ORIGINAL headers: nothing a `Connection` field nominates has
/// been removed yet (S-PIG-81). `role` selects the request rules (no framing = no body) or the response rules.
pub(crate) fn derive_framing(headers: &http::HeaderMap, role: FramingRole) -> Result<BodyFraming, FramingFault>;
pub(crate) enum FramingRole { Request, FinalResponse { request_was_head: bool, status: http::StatusCode } }
pub(crate) enum FramingFault { BothLengthAndCoding, RepeatedContentLength, ContentLengthNotDecimal, ContentLengthOverflow,
    RepeatedTransferEncoding, RepeatedChunked, FinalCodingNotChunked, UnsupportedCoding }

/// Chunk framing lines (S-PIG-80). Each takes one line as returned by `read_through(LineEnd, ..)` (terminator included).
pub(crate) fn parse_chunk_size_line(line: &[u8]) -> Result<u64, ChunkLineError>;        // hex size; extensions discarded
pub(crate) fn check_chunk_terminator(line: &[u8]) -> Result<(), ChunkLineError>;        // exactly CRLF
pub(crate) fn is_trailer_end(line: &[u8]) -> Result<bool, ChunkLineError>;              // true for the empty line; other lines are validated and dropped
pub(crate) enum ChunkLineError { Line(LineViolation), SizeNotHex, SizeOverflow, SizeLineMalformed, TerminatorNotCrlf }

/// Deadline composition (S-PIG-85): the `ReadDeadlines` the driver passes to `read_through` for each phase, already reduced
/// to `min(phase budget, request_deadline)`. `request_deadline` is `request_deadline(first_octet_at, limits)`.
pub(crate) fn request_deadline(first_octet_at: Instant, limits: &GatewayLimits) -> Instant;
pub(crate) fn compose_read_deadlines(phase: ReadPhase, now: Instant, request_deadline: Instant, limits: &GatewayLimits)
    -> ReadDeadlines;
pub(crate) enum ReadPhase {
    FirstRequestHead, LaterRequestHead, FramingLine,
    /// `request_complete_at`: `Some(t0)` once the upstream request is completely written (R-37), else `None`.
    ResponseHead { request_complete_at: Option<Instant> },
}
```

`validate_lines` is applied to the whole header block as one input for heads and to one line for the other callers; the
`ObsoleteLineFold` variant applies only to a header block. The error-to-status mapping of `RequestHeadError::status` is the Status
mapping delta; `ResponseHeadError` has no status method because every variant is the single 502.

```rust
// overdrive-dataplane::gateway_relay (crate-private, pure; the typestate of ADR-0246)
pub(crate) const PIPE_FLOOR: usize = 65_536;
/// A capacity read back from `F_GETPIPE_SZ`; it cannot be below `PIPE_FLOOR`.
pub(crate) struct PipeCapacity(usize);
impl PipeCapacity {
    pub(crate) fn new(read_back: usize) -> Result<PipeCapacity, RelayPipeError>;   // Err(CapacityBelowFloor { got }) below the floor
    pub(crate) fn get(&self) -> usize;
}
/// `min(octets owed, capacity)`: the length of the next splice into an empty pipe (never above either).
pub(crate) fn splice_in_len(owed: NonZeroU64, capacity: PipeCapacity) -> NonZeroU64;
```

The empty-before-splice-in and drain-before-next-splice rules are the pipe value's own typestate (the crafter names its types); S-PIG-86
tests `PIPE_FLOOR`, `PipeCapacity::new` and `splice_in_len` only; the typestate itself gets no test and no compile-fail fixture.

### Exchange state machine, keep-alive, drain (ADR-0222, 0238, 0240, 0242)

One request is in flight per public connection. An exchange runs two concurrent halves: the request half (`public.read` to
`upstream.write`) and the response half (`upstream.read` to `public.write`); the response half starts after the upstream request
head is written and may finish before the request body does, so bidirectional streaming cannot deadlock on full buffers (the
exchange is full-duplex, R-10, P-13b). Each half owns its pipe; the leg's transmit lock serialises the
response half's writes with the request half's KeyUpdate replies.

| Event | Rule |
|---|---|
| Both halves complete (`finish()` returns `Reuse`: response not close-delimited, request fully relayed including a chunked body's last chunk, connection not draining, public peer not half-closed) | The machine is `Idle`; the upstream leg gets `close_write` and is dropped; the public connection waits for the next head |
| Response complete while the request body is unfinished (early response, including an early 401/413; a `ChunkedOpen` request body is unfinished) | `finish()` returns `Close`; the driver performs the **lingering close** (below) |
| Close-delimited response complete (R-12) | `Close` |
| close_notify from the public peer while idle | `Close` |
| close_notify from the public peer after the request is fully read | the response is delivered, then `Close` |
| close_notify or reset from the public peer mid-request-body | abort both legs |
| Upstream close_notify before the response head | 502; after the head, mid-body: abort both legs; a close-delimited body ends cleanly (R-12) |
| Drain begins (`close_for_shutdown`/supersession) | idle connections close at once; an in-flight exchange runs to completion within the one drain deadline, then is force-aborted; `try_request` fails, so the hundredth rule and drain share one `Connection: close` path and no later request is parsed |

**Early response and lingering close (R-14, P-18).** Closing a socket that holds
unread received data makes the kernel send a reset, which can destroy the response the client has not yet read. After the
gateway has delivered a complete final response to the public leg while request octets remain unread, the driver calls
`close_write` (close_notify, FIN) and then `discard_remaining` on the public read half, and closes the socket on every outcome.
- **Scope.** Every such case, whether or not the request reached `ExchangeMachine`: an early origin response (including 401/413),
  and the gateway-authored 400, 404, 413 (declared length), 421, 431, 502, 503 and 505 that are sent before or without a
  request body being read. Not 408 (the peer stalled; nothing is in flight to destroy) and not a leg that already reported
  EOF, alert or reset.
- **Bounds (no new limit).** `max` is `max_request_body_bytes`; `no_progress` is `body_no_progress_deadline`; `absolute` is the
  earlier of `whole_request_deadline` from the request's first octet and, once drain has begun, the drain deadline (R-64: the
  shutdown deadline minus `GATEWAY_SHUTDOWN_JOIN_RESERVE`, so the final owner joins keep their reserve). The
  discard ends earlier at the peer's close_notify, which a well-behaved client sends after reading the response.
- **Drain.** A discard in progress or begun after drain starts is not extended: it ends at the drain deadline at the latest and
  the socket is then closed. It is part of the in-flight exchange for the drain accounting, not an idle connection.
- **Outcomes.** `discard_remaining` returns once and the socket is closed whatever it returned; a discard failure changes
  nothing the client has been told.

**Documented exposure (not defended by a new limit).** A peer that sends a body in one-octet chunks costs about eight syscalls
per octet at the 16 MiB ceiling; it is bounded by the absolute exchange deadline and the body ceiling.

`ExchangeMachine` (crate-private, pure) is the only owner of this table.

```rust
// overdrive-gateway::runtime::http1::exchange (crate-private; pinned for Quint conformance)
// Connection-level phase. The two concurrent halves are NOT phases: each half is read through its own accessor.
pub(crate) enum ExchangePhase { Idle, Relay, Closed }
pub(crate) enum ResponseState { Pending, Body, Complete }
pub(crate) enum Remaining { None, Exactly(u64), ChunkedOpen, UntilClose }
pub(crate) enum Disposition { Reuse, Close }
pub(crate) enum Side { Public, Upstream }
/// The body delimitation the pure framing decision derived from a head (ADR-0245). It is the machine's only framing input.
pub(crate) enum BodyFraming {
    /// No body: a request without Content-Length or Transfer-Encoding; a response to HEAD, or 204, 205, 304.
    NoBody,
    /// `Content-Length: n`. `Length(0)` is equivalent to `NoBody`.
    Length(u64),
    /// `Transfer-Encoding: chunked` (ends at `request_chunked_end` / `response_chunked_end`).
    Chunked,
    /// Close-delimited response body (R-12). Illegal as a request framing.
    UntilClose,
}
/// A call that contradicts the machine's state. Peer framing faults (ambiguous, overflowing, repeated or unsupported
/// framing) are rejected by the pure head parser before the machine is called (Message-framing contract), so every
/// violation is an internal-consistency failure of the exchange driver's own call sequence, never a client or origin fault.
pub(crate) enum FramingViolation {
    /// The call is not legal in the current phase (e.g. `begin_request` outside `Idle`, any other call in `Idle`/`Closed`).
    WrongPhase { phase: ExchangePhase },
    /// `response_head` when `response_state()` is not `Pending`.
    ResponseHeadRepeated,
    /// `UntilClose` passed to `begin_request`.
    UntilCloseRequest,
    /// `*_octets_moved(n)` with `n > 0` above the side's `Remaining::Exactly(k)`, or with `n > 0` when that side's remaining
    /// is `None` (including a response before its head). `ChunkedOpen` and `UntilClose` accept any `n`: the chunk-line
    /// parser bounds chunk data, the driver bounds the running total.
    OctetsBeyondFraming { side: Side, n: u64 },
    /// `request_chunked_end` / `response_chunked_end` when that side's remaining is not `ChunkedOpen`.
    ChunkedEndNotOpen { side: Side },
    /// `finish()` while `response_state()` is not `Complete`.
    ResponseNotComplete,
}
impl ExchangeMachine {
    /// Fresh machine: `Idle`, `ResponseState::Pending`, both remainings `Remaining::None`, not draining, public peer not
    /// half-closed. One machine per public connection; it is reused across keep-alive exchanges, never shared.
    pub(crate) fn new() -> ExchangeMachine;
    /// Idle -> Relay (the upstream leg is connected by the exchange driver before this call; a connect failure is a
    /// 502 before any exchange begins and never reaches the machine).
    pub(crate) fn begin_request(&mut self, method_is_head: bool, framing: BodyFraming) -> Result<(), FramingViolation>;
    pub(crate) fn request_octets_moved(&mut self, n: u64) -> Result<(), FramingViolation>;   // n beyond Remaining is an error
    pub(crate) fn response_head(&mut self, framing: BodyFraming) -> Result<(), FramingViolation>;
    pub(crate) fn response_octets_moved(&mut self, n: u64) -> Result<(), FramingViolation>;
    /// Ends a `ChunkedOpen` body (last chunk and trailers consumed). Chunk framing is a pure-parser concern and is
    /// not modelled in Quint; these two are exercised by PBT only.
    pub(crate) fn request_chunked_end(&mut self) -> Result<(), FramingViolation>;
    pub(crate) fn response_chunked_end(&mut self) -> Result<(), FramingViolation>;
    pub(crate) fn peer_closed(&mut self, side: Side, clean: bool);
    pub(crate) fn drain_begin(&mut self);
    pub(crate) fn force_abort(&mut self);
    /// Fails `FramingViolation` unless `response_state() == Complete`. Relay -> Idle and `Reuse` when `reusable()`, else
    /// Relay -> Closed and `Close`.
    pub(crate) fn finish(&mut self) -> Result<Disposition, FramingViolation>;
    // observable projection (accessors the Quint driver reads)
    pub(crate) fn phase(&self) -> ExchangePhase;
    pub(crate) fn request_remaining(&self) -> Remaining;
    pub(crate) fn response_state(&self) -> ResponseState;
    pub(crate) fn response_remaining(&self) -> Remaining;
    pub(crate) fn public_half_closed(&self) -> bool;
    pub(crate) fn is_draining(&self) -> bool;
    /// Derived (not a projected variable): `finish()` returns `Reuse` exactly when this holds.
    pub(crate) fn reusable(&self) -> bool;
}
```

**Framing-to-remaining mapping (the machine's one rule).** `NoBody` and `Length(0)` give `Remaining::None`; `Length(n>0)` gives
`Exactly(n)`; `Chunked` gives `ChunkedOpen`; `UntilClose` gives `UntilClose`. For a `HEAD` request (`method_is_head`), the
response remaining is `None` whatever framing `response_head` is given (the framing a `HEAD` response carries describes a body
that is not sent). `request_octets_moved(n)`/`response_octets_moved(n)` reduce `Exactly(k)` to `Exactly(k-n)` (`None` at zero);
they leave `ChunkedOpen` and `UntilClose` unchanged (the driver counts the running total against `max_*_body_bytes`).
`response_head` also sets `response_state()`: `Complete` when the resulting remaining is `None` (so `Length(0)` and `NoBody`
complete at once, as the spec's `resp_head(true, 0)` does), else `Body`. `response_octets_moved` reducing `Exactly` to zero, `response_chunked_end`,
and `peer_closed(Upstream, true)` during an `UntilClose` body set `Complete`; `peer_closed(Upstream, _)` before the head, or
during a `Length` or `ChunkedOpen` body, moves the machine to `Closed` (abort), as the Exchange table states.

**Status mapping rule for `FramingViolation`.** Every variant means the driver's call sequence contradicted the machine; none is
caused by a peer. The driver therefore closes both legs, answers 502 if no public response octet has been written (never a 4xx:
the client is not at fault), writes nothing further otherwise, and emits one error-level structured event naming the variant.
The pre-head and post-head projections of the Status mapping delta are unchanged by it.

`reusable()` holds when the request and response remainings are `None` (a `ChunkedOpen` body is unfinished, so it is
false and the early-response rule applies), the response is `Complete` and not `UntilClose`-delimited, the machine is not
draining and the public peer has not half-closed. A chunked response's delimitation is by length (`Remaining::ChunkedOpen`,
not `UntilClose`); only `Remaining::UntilClose` is close-delimited (R-12). The driver uses `reusable()` as the `conclude`
handler's precondition and run expectation, and checks the returned `Disposition` against it.

### Status mapping delta (extends the table in the Application HTTP contract)

400 malformed head, framing ambiguity, non-origin-form, Host; 408 head/body no-progress before any response octet; 413 request
body; 421 SNI/Host; 431 request head; 502 every upstream, receipt-
identity, mTLS, arming, response-head, response-framing or response-overflow-before-head failure; 503 `NoBackend` only; 505 not
HTTP/1.1. 404 Route miss unchanged. A pipe that cannot be created or reads back below the floor (`RelayPipeError`) is 502 before the
response head and closes the connection after it. There is no 504, retry or replay.

### Earned-Trust probe for the relay (ADR-0244, ADR-0246)

`GatewayRelay::probe` runs once per boot in gate G-1 after custody is ready and before identity and the connect probe. It does
not re-run per staged generation. Its scope is what catches image-configuration drift: over a loopback socket pair with an
in-process ephemeral certificate it (1) completes a rustls handshake on both ends and arms kTLS TX+RX with the spiked cipher; (2)
creates a private pipe, sets it to 64 KiB and reads the size back (`F_SETPIPE_SZ` read-back); (3) relays 64 KiB plus one octet
through that pipe pattern-exactly. **Substrate lies it must catch:** the `tls` ULP absent or refused (`Arm(UlpInstall)`), a cipher
the kernel refuses (`Arm(CipherNotSupported)`), a kernel that loses octets on splice (`SpliceExactness`), a pipe-size clamp
(`Pipe(CapacityBelowFloor)`; the one pipe fact has the one variant `RelayPipeError`). KeyUpdate re-arm and control-record handling are not probed: the Tier-3 relay suite proves them on the kernel.
There is no real-kernel stimulus for "ULP absent" (R-23, P-23): on the appliance the pinned kernel configuration is the image's
(ADR-0068), so a node without the `tls` ULP is an image build defect, not a runtime state any production path produces, and
unloading a module on the shared metal host is a node-global mutation that needs a restore guard. The lie is refused at Tier 1
(below); the real kernel proves only the honest pass. No test seam is added to the production probe.
Failure keeps TCP/443 unbound, emits `health.startup.refused` reason `gateway.relay_probe` and sets
`connect_path: Unavailable` with cause `ConnectPathUnavailable`.
Enforcement in two layers: (a) subtype: `probe` is a required method of `GatewayRelay` with no default body and
`GatewayRuntimeDependencies::new` takes `Arc<dyn GatewayRelay>`; (b) behavioural: a gold test drives `SimGatewayRelay` through
each substrate lie above and requires the probe to refuse, plus one Tier-3 run on the real kernel. The requirement on the Sim is
abstract: it can present each substrate lie to the probe; its configuration surface, fault names and scripted peer events are test
support that DISTILL shapes.

### Pinned observable projection for Quint conformance

**Conformance target: module `relay_exchange` only** (specs/quint/public-ingress-gateway/relay_exchange.qnt). Owner `ExchangeMachine`
(pure; no `Sim*`). This section is the ONLY home of the action-to-method and variable-to-accessor tables (the spec header and the
README point here). Every action calls exactly one owner method. `idle` is not an action of the conformance main (`relay_conf`):
the spec keeps a stutter only in the exhaustive main `relay_ok`, so no conformance step maps to "no call" and no lint exemption
is needed.

| Spec action | `ExchangeMachine` call |
|---|---|
| `read_head(n)` | `begin_request(false, BodyFraming::Length(n))` |
| `req_unit` | `request_octets_moved(1)` |
| `resp_head(ld, n)` | `response_head(Length(n))` when `ld`, else `response_head(UntilClose)` (R-12) |
| `resp_unit` | `response_octets_moved(1)` |
| `up_close` | `peer_closed(Side::Upstream, true)` |
| `conclude` | `finish()`; the handler checks the `Disposition` against `reusable()` evaluated before the call |
| `p_close` | `peer_closed(Side::Public, true)` |
| `drain_begin` | `drain_begin()` |
| `force_abort` | `force_abort()` |

The abstraction: one model "unit" is one octet (`request_octets_moved(1)`), and a model unit count `k` is `Remaining::Exactly(k)`
(`Length(0)` yields `Remaining::None`). Chunk framing (`ChunkedOpen`, `request_chunked_end`, `response_chunked_end`) is not
modelled: it is a pure-parser property covered by PBT only. Variable projection (each state variable has one accessor):

| Model variable | Accessor | Mapping |
|---|---|---|
| `phase` | `phase()` | `Idle`/`Relay`/`Closed` one to one |
| `req_left` | `request_remaining()` | `Exactly(k)` is `k`; `None` is 0 |
| `resp_state` (`RNone`/`RBody`/`RDone`) | `response_state()` | `Pending`/`Body`/`Complete` |
| `resp_left` | `response_remaining()` | `Exactly(k)` is `k`; `None` and `UntilClose` are 0 |
| `len_delim` (R-12) | `response_remaining()` | true unless the value is `UntilClose` |
| `draining` | `is_draining()` | boolean |
| `p_eof` | `public_half_closed()` | boolean |

Seven model variables, six accessors; `reusable()` is a derived accessor (the spec's `reusableOf`) used for handler preconditions
and run expectations, not a projected State field. The spec's `init` is the fresh `ExchangeMachine` (`Idle`, `Pending`, both remainings `None`, not draining, not half-closed). Values the driver must see (spec `closedS` and the reuse branch of `conclude`): in `Closed`,
`request_remaining()` and `response_remaining()` are `None`, `response_state()` is `Pending`, `len_delim` is true, and
`is_draining()` and `public_half_closed()` keep their values; `drain_begin` from `Idle` yields `Closed` with `is_draining()`
true; `peer_closed(Public, true)` from `Idle` yields `Closed` with `public_half_closed()` false; `finish()` returning `Reuse`
leaves `Idle` with `response_state() == Complete` and both remainings `None` until the next `begin_request`.

**Close-delimited responses (R-12).** `len_delim`, the model bound on close-delimited units (`ud_units`, MAXU), `Remaining::UntilClose`,
`relay_to_end` and the run `close_delimited_clean_end` are part of the design.

The two concurrent halves are read through two independent accessors (`request_remaining()`, and `response_state()` with
`response_remaining()`); `phase()` is connection-level only, which is what resolves the single-phase ambiguity. Model-only (not
projected): `exch` and `ud_units` (exploration bounds) and `overread` (the hazard's detector). Every named run of `relay_exchange`
is `#[quint_test]`-replayable (seven); `#[quint_run]` randomized traces are added. The reuse rule has no separate invariant: it is guard-derived
(reuse is reachable only through `finish()`'s `reusable()` guard, the spec's `conclude` guard `reusableOf`, pinned by `expect` in
the named runs) and has no hazard; the one safety invariant is `NoBytePastMessageBoundary`.

**No kTLS lock model (R-19).** The public leg's transmit-lock / peer-KeyUpdate exclusion of ADR-0244 has no Quint model. Pausing a
kernel writer needs a test seam this design does not add, so the exclusion is covered by assumption A-K5 and the Tier-3 check. Its
Tier-3 oracle is the bytes a real rustls peer receives plus `/proc/net/tls_stat` deltas (`TlsRxRekeyOk`, `TlsTxRekeyOk`,
`TlsDecryptError == 0`). A KeyUpdate inside a head or chunk-size line (the read half's accumulator across a control record) is a
Tier-3 obligation. Chunk framing is covered by property tests only.

### Budgets

- **Hot path (per request, plus one connection setup).** Syscalls: heads 2 `recvmsg` pairs (peek, recv) and 2 `sendmsg`; body
  per MiB about 67 splice calls at 16 KiB peer records and up to 4098 at 256 octets (findings (c)); chunked adds about 4 syscalls
  per chunk. Without `SPLICE_F_MORE` a trickling body is sent as small TLS records (wire overhead, not measured); the spike
  figures above were taken with the flag and are not re-measured for the no-flag configuration. No per-octet allocation; head accumulators are bounded by the head limits. **No CPU or throughput gain is claimed**
  (findings (c)); the gain is bounded memory and no userspace payload copy.
- **Memory and descriptors, accounted per connection.** An active exchange holds two 32 KiB head accumulators plus up to two
  64 KiB kernel pipe buffers (about 192 KiB, 24 MiB at 128 exchanges); descriptors are 2 sockets plus up to 4 pipe descriptors,
  up to 768 at 128 exchanges. Pipe lifetime is the one rule of Contract item 7 (created on the
  half's first `relay_*` call, released when the half starts its next head read), so a keep-alive connection waiting for its next
  request holds sockets only (holding the pipe until connection close is not adopted, R-16). The process `RLIMIT_NOFILE` must admit 768
  plus the node's other descriptors; this is a DISTILL Tier-3 check at 128 concurrent exchanges, not a new limit.
- **Validation budget.** The Quint check counts (per module and for the whole subsystem) and timings are stated only in the
  `specs/quint/public-ingress-gateway/checks.toml` header; this document restates none of them. Tier 3: the relay suite on native metal (KeyUpdate, control records,
  chunked, backpressure, pipe floor, no-flag cases, lingering close, 128-exchange descriptor check). Growth beyond this needs
  user approval.

### Lifecycle Gate Ownership review (G1)

Gates G-2, G-3 and G-4 are unchanged. G-1 gains one input (`GatewayRelay::probe`), G-5 arms after equality, G-6's promise text
covers in-flight pipes; the gate sections above carry the edits. No gate is added: per-leg "armed" is a precondition of the
leg value existing (`KtlsLeg` is only constructed armed), so it has no lifecycle owner or state meaning of its own.

### Kernel claims

| Claim | Source |
|---|---|
| One splice returns at most one record's octets; a non-DATA record gives `-EINVAL` and stays readable | `net/tls/tls_sw.c:2285-2327, 2310-2314 @ v7.0` |
| Splice-in loses octets when the pipe cannot map the whole span, and why 64 KiB empty suffices | [ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md) Context (the cites live there) |
| Splice-out `MSG_MORE`: `SPLICE_F_MORE` sets it (`fs/splice.c:875-876`), the kernel sets it when the same call leaves octets in the pipe (`:877-878`), at most 16 pipe buffers per `sendmsg` (`:799`); kTLS closes the record when `MSG_MORE` is absent, and `tls_sw_sendmsg` accepts `MSG_MORE` (`tls_sw.c:1046, 1283-1286`) | `fs/splice.c:799, 874-888 @ v7.0`; `net/tls/tls_sw.c:1046, 1283-1286 @ v7.0` |
| Record type via `TLS_GET_RECORD_TYPE`; no control buffer gives `-EIO` | `tls_sw.c:1844-1865 @ v7.0` |
| KeyUpdate pauses RX with `-EKEYEXPIRED` until `TLS_RX` is re-armed; poll reports nothing | `tls_sw.c:1369-1371, 1802-1806, 2779-2786`; `net/tls/tls_main.c:423-427`; `Documentation/networking/tls.rst:214-220`, all @ v7.0 |
| Re-arm is `setsockopt(TLS_TX/TLS_RX)` on an armed socket; version and cipher must match; each call takes the socket lock, but no serialisation of a multi-call logical write against a re-arm | `net/tls/tls_main.c:661-687, 635-783 @ v7.0` |
| Sending a control record flushes an open record first | `tls_main.c:228-268 @ v7.0` |
| The ULP needs an established connection | `tls_main.c:1059-1066 @ v7.0` |
| Splice to a socket honours only the socket's `O_NONBLOCK`; partial counts are returned | `fs/splice.c:874-888, 916 @ v7.0`; `tls_sw.c:1273-1275 @ v7.0` |
| kTLS plus sockmap: rejected one way at v7.0 and both at v7.3 | `net/core/skmsg.c:756 @ v7.0`; `net/tls/tls_main.c:636-645 @ v7.3` |
| The kernel does not originate KeyUpdate | no such code in `net/tls/tls_sw.c`, `tls_main.c @ v7.0` |
| Default pipe is 16 pages (`PIPE_DEF_BUFFERS`), but an unprivileged user over the soft limit gets `PIPE_MIN_DEF_BUFFERS` (2); irrelevant because the relay sets and reads back the size; resize above the limit needs privilege | `include/linux/pipe_fs_i.h:5`; `fs/pipe.c:809-811, 1385-1394 @ v7.0` |

v7.3 citations anywhere in this document are informational provenance, not the design basis. All contract-shaping items for G1 are
settled by the spike (findings a-e). One empirical limit remains and is named, not hidden: the pipe invariant is validated on one
kernel build (7.0.0-38) and by the source reading above, not proven.

## Wave: DESIGN / [REF] Consuming-port availability contract (D2)

[ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md) is the decision. The contract:

- **Owner.** `GatewayUpstreamConnector::{probe, connect}` (the gateway's own consuming port) is the only place backend availability becomes an outcome. `GatewayApplicationOwner` never waits for a backend.
- **No readiness wait.** `connect(frontend, deadline)` performs one selection. There is no poll, sleep, retry or second pick. The only time bound it consumes is the request's existing absolute deadline (`limits.upstream_connect_deadline()` within the owner-supplied request deadline); no budget is added or multiplied.
- **Outcome table (the failure projection).**

| Condition at the hook | Receipt | Connector result | Public status |
|---|---|---|---|
| No outer entry for the `ServiceKey`, empty Maglev slot, missing `BACKEND_MAP` entry | `NoBackend` | `GatewayUpstreamConnectError::NoBackend` | 503 |
| Selected backend has `healthy == 0` (not ready, or in the observed ~1 s window between startup `Stable` and the first readiness `pass`); the arm ignores `healthy` (R-2, P-9) | `Selected` | `Connect` (the dial to a not-yet-ready guest fails) | 502 |
| Selected, registered tuple gone (stopped backend still in the maps, P-4) | `Selected` | `Connect` (reset at the nft output chain) | 502 |
| `Selected`, applied identity missing or mismatched | `Selected` | `SelectedIdentity` / `ReceiptMismatch` | 502 |
| Hook, map or receipt inconsistency | missing or malformed | `Receipt` | 502 |
| Exact-peer mTLS failure | `Selected` | `Mtls(..)` | 502 |

- **Sim arm contract (D-PASS7-4).** `SimGatewayConnectDataplane` implements the same observable contract as the real hook, over the `SimDataplane` state it shares, and decides nothing the table above leaves open: for a prepared socket's `ServiceKey`, **no outer entry for the key, or an empty/absent slot in the chained lookup → receipt `NoBackend`**; otherwise → `Selected` carrying the actual `BackendId` the shared `SimDataplane` state holds for the key's slot (deterministic choice, the lowest `BackendId` of the key's current generation; the real Maglev choice is proven only at Tier 3). **Oracle strength:** because the real hook selects by Maglev over the cookie hash (item 3 above), Tier-1 / Sim-backed oracles assert only *membership* in the committed generation when it has N>1 backends; exact-`BackendId` assertions are limited to singleton generations. **Tier-3-only row:** the real outcome table also yields `NoBackend` for a missing `BACKEND_MAP` entry (outer and slot present, backend id absent); the Sim cannot produce that from the `update_service` input, so that row is observed only at Tier 3, and `selected_backend_identity` resolves it only for ids whose `update_service` has completed (Applied), mirroring the commit-guarded read. The Sim does not evaluate `Backend.healthy` (it is present in the `update_service` input but, as in the real arm, nothing decides on it; R-2): the Sim returns `Selected` for any backend of the generation and the healthy window is neither modelled nor tested at Tier 1 (it is a Tier-3/Native observation, obligation 2). Cookies are deterministic nonzero values; `open_upstream` registers like the real adapter; `cleanup`/`take_receipt` follow the port contract (a receipt exists exactly between the Sim connect and `take_receipt`/`cleanup`). A scenario obtains `NoBackend` by never calling `update_service` for the key (or calling it with an empty set) and `Selected` by calling it with a non-empty set; no scripted receipt exists. **Fault surface the tests may use (existing ports only):** typed `GatewayConnectDataplaneError` results returned from the existing port operations (`probe`, `open_upstream`, `take_receipt`, `cleanup`, `selected_backend_identity`, `live_*`) via the Sim's existing typed-fault configuration at those operations, and the pinned `HoldConnectProbe`/`ReleaseConnectProbe` (see Application verification). Nothing else is scripted; a probe failure at boot is the typed `probe` error.
- **Boot.** An actual-frontend `NoBackend` or `Selected` probe result is bind-valid (gate G-1, R-65); backend absence, and a selected backend's identity or mTLS failure, never block TCP/443.
- **Late readiness.** The first request after the readiness `pass` reaches the hydrator-written `healthy = 1` and is `Selected`. The window before it is a dial to a not-yet-ready guest that fails (502, R-2); clients retry. The design adds no wake to shorten it. The core: no outer entry or no usable backend → `NoBackend` → 503; a selected backend whose dial fails → 502; never a wrong-peer success.
- **Replacement obligation for DISTILL (restates acceptance gap C4a).** The unavailable-outcome path applied twice has an empty second observable delta: two consecutive public requests while no healthy backend exists both return 503 with identical bodies, `live_intent_count() == 0` and an empty `live_receipts()` after each, no change in `GatewayApplicationStatusRowV1`, and no change in the Gateway Applied Generation. Custody replay and identity-hold repeat keep their existing coverage. DISTILL also proves: `healthy` 0 → 1 flips the next request with no gateway action (502 → backend response; R-2), matching obligation 2; a stopped backend yields 502 (not a wrong-peer success); late `Selected` after a superseded application generation is not delivered.

## Wave: DESIGN / [REF] Composition-root gateway slot, boot and shutdown (D5)

### New public surface (D5) — confirmed by the user 2026-11-02 (R-18)

Every item is new public or cross-crate surface. The user confirmed items 1-13 on 2026-11-02, item 12 (lag-aware `IntentStore::watch`) reconfirmed.

1. `BootAdapters.gateway: GatewayAdapters` (new required field) and `GatewayAdapters` (`certified_key_codec`, `client_mtls`, `relay`); `GatewayAdapters::host(kek, clock)` (the clock is the injected one the relay's deadlines use).
2. `GatewayClientMtlsFactory` and `GatewayClientMtlsDeps` (`overdrive-gateway::ports`), with `GatewayClientIdentityAccess` and its source trait `GatewayClientIdentitySource` also defined in `overdrive-gateway::ports` so that `Deps.identity` names no dataplane type (R-20; "Gateway identity and lifecycle integration").
3. `ComposedDataplane.gateway_connect: Arc<dyn GatewayConnectDataplane>` (new required field).
4. `GatewayConnectDataplane::open_upstream` and `PreparedUpstream` (replace `register`), the four added error variants.
5. `UnboundGatewayBuilder::into_builder` (replaces `bind_demand_wake`), and the removal of the `cookie_reader` parameter from `UnboundGatewayBuilder::new`.
6. `GatewayAppStateComposition` without demand; `AppState.gateway`.
7. `GatewayShutdownReport.elapsed` and the ServerHandle gateway stage (`stage = "gateway"`).
8. `HostGatewayClientMtlsFactory` (dataplane crate; now constructed from `Arc<HostGatewayRelay>`) and `SimGatewayClientMtlsFactory`, `SimGatewayConnectDataplane`, `SimGatewayRelay` (sim crate); `HostGatewayRelay` (`overdrive-dataplane::gateway_relay`).
9. `Action::IssueGatewaySvid` (there is no `DropGatewaySvid` action: disable is a synchronous clear, R-4), the `GatewaySvidLifecycle` reconciler and its `AnyReconciler`/`AnyState`/`AnyReconcilerView`/`AnyViewMap` variants (wakeup declaration: host-backed, empty `interests()` plus `resync_schedule()`, resolved by `.claude/rules/reconcilers.md`).
10. One NEW hydration read-port in `overdrive-core` — `GatewayIdentityView` (non-secret gateway-identity desired epoch/subject and current facts; contract in `overdrive-core::gateway_identity`) — and the one new mandatory `HydrationContext` field that carries it (`gateway_identity`, after `guest_attachments`). The port has one `Sim*` implementation (`SimGatewayIdentityActionTarget`) so `GatewaySvidLifecycle::hydrate_*` stays DST-injectable and reads no concrete `AppState` field (`.claude/rules/reconcilers.md` § "The hydration boundary").
11. `ServeArgs.gateway_*` clap fields, `ServerConfig.gateway: Option<GatewayConfig>` and `deploy_resource` (branch contract, unchanged shape, re-pinned below to `run_with_adapters` and `commands::deploy`).

#### Previously approved (2026-09-13), carried unchanged — reconfirmed on main 2026-11-02 (R-18)

12. `IntentStore::watch` changes in one cut to `IntentSubscriptionEvent` = `Changed | Lagged` (D-APP-10; modifies a main trait that is unchanged since the merge base). Provenance: the pre-reconstruction branch is preserved at git ref `backup/research-ingress-gateway-pre-reconstruct` (commit f3999011500f; tag `backup-public-ingress-pre-reconstruct-2026-11-01`). Its `docs/feature/public-ingress-gateway/feature-delta.md` lines 3-7 record "APPROVED — user ratification APPROVED, 2026-09-13 … The user explicitly approved the complete public-ingress-gateway DESIGN", and its line 1198 is "**D-APP-10** | `IntentStore::watch` changes in one cut to `Changed|Lagged`; …". Reconfirmed by the user on main, 2026-11-02 (R-18).

#### New with G1

13. **G1 data-path surface.** Exactly the types, port methods, error enums and `ExchangeMachine` accessors pinned in
    [Kernel relay data path and driven ports](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1) (driven port `GatewayRelay`,
    the opaque `KtlsLeg` family, `ReadDeadlines`, the error enums, the re-pinned sealed `GatewayUpstream`, `GatewayClientMtlsError::Arm`,
    `GatewayRuntimeDependencies::new(.., relay)`); that section is the SSOT and this item only lists that it is new public surface.
    The `GatewayByteStream` forms are removed.
    **DG-7 (QC-2 driver placement) adds no public item.** The resolution is test-local doubles inside the QC-2 driver module (Formal
    protocol model, `gateway_application`, "Conformance placement and test-local doubles (DG-7)"); it is a placement ruling confirmed with item 13 (R-18). No `PROPOSED` public surface arises from it.
    **Confirmed 2026-11-02 (R-18; added to close DISTILL gaps DG-1..DG-3; part of item 13).** Public items not
    in the previously listed G1 surface: `KtlsLeg::from_armed`, `KtlsReadHalf::backend_mut`, `KtlsWriteHalf::backend_mut`,
    `ThroughBytes::new` (cross-crate construction contract); `HostGatewayRelay::new`, `SimGatewayRelay::new`,
    `SimGatewayClientMtlsFactory::new`, `SimGatewayAdapters` with `new`/`relay`/`client_mtls`/`sim_relay`. Crate-private,
    pinned for the Quint driver and PBT: `ExchangeMachine::new`, `BodyFraming` (`NoBody`, `Length(u64)`, `Chunked`,
    `UntilClose`) and `FramingViolation` (six variants). Alternative considered and rejected for the leg values: a generic or
    associated-type port (`GatewayRelay` is used as `Arc<dyn GatewayRelay>`, so it cannot be generic); a sealed constructor trait
    (the backends live in other crates, so a seal in `overdrive-gateway` would exclude both).
    **Confirmed 2026-11-02 (R-18; added to close DISTILL gaps DG-8..DG-10; part of item 13).** Public:
    `PreparedUpstream::new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent)` (Gateway upstream path, "Cross-crate
    construction of `PreparedUpstream`"; alternative rejected: a `Box<dyn Any>` backend state as for the legs, and a carried
    cleanup guard). Crate-private, pinned for the QC-2 driver and used by `GatewayBuilder::start`: `GatewayAdmission::new`,
    `GatewayAdmission::budget`, `GatewayBuilder::start_with` (Formal protocol model, `gateway_application`, "Admission access
    (DG-10)"; alternatives rejected: a `pub(crate)` admission accessor on a running composition, whose only caller would be the
    driver, and hand-assembling the owner in the driver, which cannot produce `GatewayRouteCommands` and `GatewayStatusReads`). DG-9 adds no item: the park
    points are the existing awaited driven-port operations the gate owns (`GatewayConnectDataplane::probe`, `GatewayRelay::probe`,
    identity `ensure_current`) and, for a later in-flight candidate, the connector's `connect(2)` against a fixture-controlled
    destination (Gateway upstream path, "Where a double may park a probe (DG-9)"). DG-11 (ledger slot reserved before `open_upstream`, intent bound after; R-5) adds
    no public item: `GatewayCleanupLedger` and its reservation are crate-private. DG-12 (R-6) adds none either.

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
    /// The kernel-TLS relay backend (G1): probes, arms, reads heads, relays bodies. One backend
    /// serves both legs; `HostGatewayRelay` in production.
    pub relay: Arc<dyn GatewayRelay>,
}
impl GatewayAdapters {
    pub fn host(kek: Arc<dyn Kek>, clock: Arc<dyn Clock>) -> Self;
}

// overdrive-dataplane::gateway_relay
pub struct HostGatewayRelay { /* injected Clock; no descriptor, no kernel state */ }
impl HostGatewayRelay {
    /// Infallible and kernel-silent: it opens no socket and touches no ULP, so construction claims no capability.
    /// Every kernel fact is proven by `GatewayRelay::probe` (Earned Trust), never assumed at construction. The clock
    /// is the injected one all `ReadDeadlines`/`RelayDeadlines` are evaluated on. The TLS 1.3 suite set is a constant of
    /// the contract (Contract of the legs, item 1), not an input.
    pub fn new(clock: Arc<dyn Clock>) -> Self;
}
// The arming of an authenticated upstream socket (after the peer equality) is a same-crate call from
// `HostGatewayClientMtls` into `HostGatewayRelay`; its shape is internal to `overdrive-dataplane` (crafter's), its
// observable contract is the Contract of the legs item 1 and `GatewayClientMtlsError::Arm`.

// overdrive-sim::adapters::gateway
pub struct SimGatewayRelay { /* in-memory leg state, injected SimClock */ }
impl SimGatewayRelay { pub fn new(clock: Arc<dyn Clock>) -> Self; }
pub struct SimGatewayClientMtlsFactory { /* Arc<SimGatewayRelay>, Clock */ }
impl SimGatewayClientMtlsFactory { pub fn new(relay: Arc<SimGatewayRelay>, clock: Arc<dyn Clock>) -> Self; }
// implements GatewayClientMtlsFactory by `SimGatewayClientMtls::new(deps.seal, relay.clone(), clock.clone())`.
/// The Sim counterpart of `GatewayAdapters::host`'s relay and client pair: one backend, so no mixing (Invariant 1 of
/// "Cross-crate construction of the opaque leg values"). Tests assemble `GatewayAdapters { certified_key_codec, client_mtls,
/// relay }` from its two accessors; the Sim crate does not name the control-plane type.
pub struct SimGatewayAdapters { /* Arc<SimGatewayRelay>, Arc<SimGatewayClientMtlsFactory> */ }
impl SimGatewayAdapters {
    pub fn new(clock: Arc<dyn Clock>) -> Self;
    pub fn relay(&self) -> Arc<dyn GatewayRelay>;
    pub fn client_mtls(&self) -> Arc<dyn GatewayClientMtlsFactory>;
    /// The concrete Sim relay, for the test-support surface DISTILL shapes (scripting, transcripts).
    pub fn sim_relay(&self) -> &Arc<SimGatewayRelay>;
}
```

`HostGatewayRelay::new` has no error type because it has no fallible step; the only failure surfaces are `probe` (boot gate
G-1, `RelayProbeError`), `accept_public` (`PublicHandshakeError`) and the client arm (`GatewayClientMtlsError::Arm`). `GatewayAdapters::host` builds exactly one
`Arc<HostGatewayRelay>` (`HostGatewayRelay::new(clock.clone())`) and passes it to both `HostGatewayClientMtlsFactory::new` and
the `relay` field. The scripting and observation surface of `SimGatewayRelay` / `SimGatewayClientMtls` (peer scripts, substrate
lies, transcripts) remains test support that DISTILL shapes. The new public items in this block (`HostGatewayRelay::new`,
`SimGatewayRelay::new`, `SimGatewayClientMtlsFactory::new`, `SimGatewayAdapters` and its four methods) are listed in the D5 note
on item 13.

```rust
// (continues the D5 listing)

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
// The client arms the authenticated socket through the same relay backend as `GatewayAdapters.relay`:
// `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)` (was a unit struct) and
// `SimGatewayClientMtls::new(.., Arc<SimGatewayRelay>, ..)` hold it; `GatewayAdapters::host` builds one
// `Arc<HostGatewayRelay>` and coerces it to `Arc<dyn GatewayRelay>` for the `relay` field.

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
| G1 | after `ca.trust_bundle()` (`:3912`), before `AppState::new_with_workflow_engine` (`:4085`) | `GatewayAppStateComposition::from_config(..)`: Disabled when `config.gateway` is `None`; otherwise creates the one `GatewayIdentitySlot`, its Action and Lifecycle handles and, when enabled, `GatewayRouteCommands` and `GatewayStatusReads` | boot refuses |
| G2 | after the DNS-responder block (`:4350`), before the exit-observer spawn (`:4369`) | `UnboundGatewayBuilder::new(node_id, ..).await` (G2 passes the `NodeId` it already holds, the one it gives `GatewayIdentitySlot::new`; R-57; validates `GatewayConfig`; opens Route and custody over the IntentStore; reads and installs the initial manual key; spawns no task), then `into_builder()` and `client_mtls.client_mtls(deps)` | boot refuses with `GatewayBootError` (`health.startup.refused`, reason `gateway.boot`) after the existing `shutdown_failed_boot_network_owners(..)` and `dns_responder_owner.shutdown(..)` cleanup |
| G3 | inside the gateway gate task (G4) | `relay.probe(deadline)` first (kernel-TLS relay capability, reason `gateway.relay_probe`), then `gateway_connect.probe(deadline, clock)`: sweeps residue left by the previous process, then runs the sentinel connect | not fatal: TCP/443 stays unbound, the operator surface and workloads are unaffected, status carries the cause, and the structured event `health.startup.refused` with reason `gateway.connect_probe` is emitted (Earned Trust: the adapter proves it can honour its contract before the listener binds) |
| G4 | after `server_task` is spawned (`:4616`), before `Ok(ServerHandle { .. })` (`:4618`) | `GatewayBuilder::start(deps).await`, then `into_parts()`: spawns the Route, custody, application, connection and gate tasks; the gate runs custody ready → relay probe and sentinel connect probe (G3) → identity `ensure_current` → actual-frontend probe (validated receipt only; no identity lookup, no mTLS, R-65) → Gateway Application promotion → TCP/443 bind | not fatal (see G3); no later refusal path exists, so `FailedBootTasks` (`:3008`) is unchanged |

The convergence loop (`:4399`), interest router (`:4447`), emit-drain (`:4474`) and shared-network supervisor (`:4505`) are all running before G4, so the first identity evaluation and the first hydration see the final ports. `ServerHandle` gains `gateway: Option<GatewayHandle>`.

### (c) Shutdown order and the fail-stop path

`ServerHandle::shutdown(self, drain_deadline)` (`M:crates/overdrive-control-plane/src/lib.rs:2890-2993`) runs the gateway first, then the existing stages unchanged:

0. `gateway` (new stage `serve.shutdown.stage_begin`/`stage_complete`, `stage = "gateway"`): `GatewayHandle::shutdown(drain_deadline)` (the argument is the one grace budget; the gateway drains until that budget minus its 500 ms join reserve and runs the final owner joins until the budget ends, R-64) — stop accepts; close the application; join admitted users; sweep intents and receipts; retire the closed application; cancel the owners; then clear the volatile `GatewayIdentitySlot` synchronously (`identity.disable_after_drain`, whose `begin_disable` empties the slot and bumps its epoch in-process; no reconciler round-trip at shutdown, because the issuance audit row is already durable and a late Issue completion is a stale-epoch no-op).
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
    routes: Option<GatewayRouteCommands>,
    status: Option<GatewayStatusReads>,
}
impl GatewayAppStateComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(/* the identity view, the identity actions, `GatewayRouteCommands` and `GatewayStatusReads` */) -> Self;
    pub fn identity_view(&self) -> &dyn GatewayIdentityView;
    pub fn identity_actions(&self) -> &GatewayIdentityActionComposition;
    pub fn routes(&self) -> Option<&GatewayRouteCommands>;
    pub fn status(&self) -> Option<&GatewayStatusReads>;
}

// overdrive-gateway
impl UnboundGatewayBuilder {
    pub async fn new(
        node_id: NodeId,
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

`Action::DataplaneUpdateService` (`M:crates/overdrive-core/src/reconcilers/mod.rs:510`) is unchanged: it carries no gateway field. `IssueGatewaySvid` sits beside `IssueSvid` (`:601`); no gateway drop action exists (R-4). Every action-shim dispatcher gains `&GatewayIdentityActionComposition` immediately after `identity` and no demand argument.

## Wave: DESIGN / [REF] Formal protocol model

Target subsystem directory: `specs/quint/public-ingress-gateway/` (permanent; `checks.toml`, `hazard/`, `evidence/`, `README.md`). Four stateful modules (`gateway_identity`, `gateway_application`, `selection_receipt` and `relay_exchange`, the last new with G1; the check counts are stated once in the `checks.toml` header); every conformance target declares a `step` relation and named runs. There is no kTLS-lock model: the transmit-lock exclusion of ADR-0244 is covered by assumption A-K5 and the Tier-3 check, and chunk framing by property tests only (R-19). The SVID issue-failure occurrence record (R-45, R-46) is not modelled and the spec is unchanged: no decision, guard or invariant of any module reads the count or the last failure (they are write-only for the protocol), and the audit failure that produces one of them is already the modelled action `audit_fail` (run `audit_failure_no_hold`) whose model effect, no hold, is unchanged; a CA signing failure has the same model effect and needs no separate action. The record adds no modelled variable, so no owner accessor is pinned for it here.

### Module `relay_exchange` (conformance target) — decisions ADR-0239, ADR-0247 (G1)

- **`relay_exchange` protocol.** One public connection's exchange: two concurrent halves (request and response), the message
  boundary (`overread` ghost, the relayPastBoundary hazard's detector), keep-alive reuse, early response, upstream and public
  close_notify, close-delimited end (R-12), drain with an in-flight body. The upstream leg is one request per connection
  (R-15), so the model has no upstream reuse. Phases `Idle`, `Relay`, `Closed` (`finish()` moves
  `Relay` to `Idle` or `Closed`; there is no intermediate phase). Invariant: `NoBytePastMessageBoundary` (`Safety`). No reuse after a mid-body close, control-record forwarding, reuse only at
  a boundary, no reuse after drain and no read while RX is blocked are held by the step guards and carry no separate invariant.
- **Abstracted, not modelled** (they change no phase or counter): interim `100` and `Expect` forwarding (R-11), and the lingering
  close that follows an early or gateway-authored response (R-14), which is post-`Closed` I/O of the driver.
- **Named runs (scenario to run).**

| Critical scenario | Run (module) | Replay |
|---|---|---|
| Two keep-alive exchanges on one public connection, response overlapping the request body | `exchange_cl_keepalive` (`relay_exchange`) | `#[quint_test]` |
| Early response closes, never reuses | `early_response_closes` | `#[quint_test]` |
| Drain with an in-flight body: completes, not reused | `drain_with_inflight_body` | `#[quint_test]` |
| Public close_notify after the request is read: response delivered, then close | `half_close_after_request_read` | `#[quint_test]` |
| Upstream close_notify mid-body aborts | `upstream_close_mid_body_aborts` | `#[quint_test]` |
| Close-delimited response ends cleanly on up_close, then not reusable | `close_delimited_clean_end` | `#[quint_test]` |
| Public close_notify mid-request-body aborts | `public_close_mid_request_aborts` | `#[quint_test]` |

  No run is added for upstream close before the response head, `force_abort` at the drain deadline, or a KeyUpdate inside a head or
  chunk-size line: these are seeded `overdrive-sim` obligations (the first two) and a Tier-3 obligation (the last).
- **Hazards (one per decision).** ADR-0239: `relayPastBoundary` violates `NoBytePastMessageBoundary` (on `relay_exchange`). ADR-0244, ADR-0249 and ADR-0250,
  ADR-0246 (pipe floor), and ADR-0245, ADR-0251, ADR-0252 and ADR-0253 (pure framing) are kernel-contract, typestate and pure-function properties and carry no model hazard; the probe's pipe-floor step (`Pipe(CapacityBelowFloor)`) and framing PBT cover them. ADR-0247 carries no model hazard: the full-duplex deadlock it prevents is a property of kernel buffer capacity that the model abstracts; ADR-0254's lingering close is post-`Closed` I/O outside the machine; the seeded `overdrive-sim` exchange invariants and the Tier-3 lingering-close scenario (S-PIG-115) cover it. ADR-0240 (one request per upstream leg) is a structural property of the machine (no upstream reuse state exists) and needs no hazard.
- **Projection and conformance.** Pinned only in the Kernel relay section ("Pinned observable projection"): the action-to-method
  table and the accessor per variable. Driver placement: `crates/overdrive-gateway/src/runtime/http1/quint_exchange.rs` behind
  `#[cfg(all(test, feature = "integration-tests"))]`, owner = the real `ExchangeMachine` (pure; no `Sim*`).
- **Interpretations the modeler made, pinned as design rules** (already in the Message-framing and Exchange tables): a close-delimited
  response ends cleanly only on close_notify after the headers; a close_notify before the response head, or mid a length-delimited
  body, aborts; a public close_notify before the request is fully read aborts and after it lets the response finish and then blocks
  reuse; an early response closes (the lingering close that follows is a driver step outside the machine, R-14).
- **Not modelled, not conformance-replayable** (no seam is added): the kTLS lock interleavings (R-19). Covered by A-K5, the Tier-3
  KeyUpdate suite on native metal (obligation 16(b), a KeyUpdate request while a large splice-out is in flight) and the Sim's
  scripted peer events. The model abstracts the kernel as assumptions A-K1..A-K6 below; real-kernel behaviour is proven at Tier 3.

Protocols judged not to need Quint: the Public Route Set single-slot claim ([ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md)) is serialized by the single owner task, the sole Route writer (get, decide, put; `IntentStore` has no value compare-and-set), and is covered by the store's own property tests; TLS/HTTP policy, routing and header rules are pure functions.

**Scope of the model.** The model covers only the concurrent or crash-ordered protocols that a prose review cannot enumerate: the identity issue/disable/reissue interleaving (`gateway_identity`), the generation drain/supersession interleaving (`gateway_application`), and the commit-gated selection receipt (`selection_receipt`). Route+key whole-object coherence, bind gating (`ListenerBoundImpliesGates`), replacement preservation (`ReplacementPreservesLastUsable`) and shutdown ordering are not modelled: they are sequential rules owned by a single task and become seeded `overdrive-sim` or unit DISTILL obligations (the Required boundary scenarios above). Shutdown order is a plain ordering of `ServerHandle::shutdown` stages (D5 section (c)) and is not a protocol.

### Module `gateway_identity` — decision ADR-0225

- **Variables and pinned accessors.** `desired_epoch`, `desired_enabled` ← `GatewayIdentitySlot::desired() -> GatewayIdentityDesired`. `held` (epoch, serial, not_after) ← `GatewayIdentitySlot::current() -> Option<GatewayIdentityFacts>`. `audited` (serials) ← `ObservationStore::issued_certificate_rows(..)` (`M:crates/overdrive-core/src/traits/observation_store.rs:1950`) filtered to the gateway SPIFFE ID. `now` is model-only (the injected `SimClock`).
- **Classification of the remaining spec variables** (the `gateway_identity` module's state variables):

  | Variable | Class | Source or reason |
  |---|---|---|
  | `issues` (in-flight `IssueGatewaySvid` Actions, by epoch) | model-only (R-31, P-31) | Asserted by the runs `issue_hold`, `reissue_near_expiry`, `disable_races_inflight_issue` (`issues == Set(1)`, `issues.size() == 0`), which the model checker evaluates on the model. No owner accessor yields it and none is added: the owner that holds an in-flight Action is the action shim, which the driver replaces by its pending-Action list (production reconciler outputs only; the driver never constructs one). The consequences that matter are compared through `held`, `audited` and `desired`, and the `issue_complete(e)` handler **fails the step** when the pending list holds no `IssueGatewaySvid` of epoch `e` (a reconciler that emits no Issue, or one for the wrong epoch, is therefore a failed replay, never a skipped step). **What the handlers assert (truthful contract; a quint-connect handler receives only the action name and its picks, never model state, so `issues` cannot be compared literally):** the `submit_evaluation` handler, after it has drained the broker and run the production reconciler, asserts that **no pending `IssueGatewaySvid` carries an epoch beyond the slot's `desired()` epoch** (owner-sourced on both sides). The production reconciler may emit an Issue on each tick while the slot is empty, and the driver drops (consumes) every pending Issue action of epoch `e` when it handles `issue_complete(e)`. A reconciler that emits no Issue, or one for the wrong epoch, fails the later `issue_complete(e)` step (no pending Issue of epoch `e`). A duplicate or early Issue is visible only through `audited`, which `audited` compares (a duplicate completes through `dispatch_issue` and adds an audited serial). **Residual exposure (stated plainly):** an extra same-epoch Issue that is completed later is not distinguished from a necessary one except through `audited`; the model's `issues` value itself is never compared. The proposal to close this, P-37, was rejected by the user on 2026-11-04. |
  | `next_serial` | model-only | The CA mints serials; the observable consequence is the `serial` of `held` and the serials in `audited`, both projected. |
  | `audit_armed` | model-only, driver input | The armed fault of the `audit_fail` action (a typed write fault on `SimObservationStore`); its effect is observed through `audited` and `held`. |
  | `g.staleHold` | model-only (ghost) | Exists only to state `StaleEpochNeverHolds`; the observable form of the same property is `held.epoch <= desired_epoch`, compared through the pinned `current()` and `desired()`. |
  | `conf_step` | model-only | Conformance step recorder; never projected. |
- **Actions → driving ports.** `submit_evaluation` → `GatewayIdentityLifecycleControl::ensure_current`; `issue_complete` → `action_shim::gateway_svid::dispatch_issue`; `disable` → `GatewayIdentityLifecycleControl::disable_after_drain` (a synchronous clear: epoch bump and an empty slot in the same step, R-4); `advance_time` → `SimClock::advance`; `audit_fail` → a failing `ObservationStore` write injected through the Sim store (driven port).
- **Invariants.** `HoldImpliesAudit`, `StaleEpochNeverHolds`, `DisabledSlotIsEmpty` (a disabled slot has no Current; `AtMostOneHeld` and `NoAllocationIdentity` are type-level, below).
- **Named runs.** `issue_hold`, `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue`.
- **Hazard (one per decision).** ADR-0225 carries one hazard: `noEpochCheck` (a stale Issue completion holds after disable) → `StaleEpochNeverHolds`. It is the interleaving defect that only exhaustive exploration finds. The audit-before-hold ordering is a straight-line ordering inside one task, covered by the named run `audit_failure_no_hold` and the production-path test GI-1 (`HoldImpliesAudit` is a cheap invariant, not a hazard). "At most one held generation" is the shape of the slot's type (one optional facts value), so it is not modelled (`AtMostOneHeld` is type-level).
- **Conformance placement (D-PASS7-1).** The driver is in the owning crate `overdrive-control-plane`: `crates/overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs`, declared from `gateway_identity_lifecycle.rs` as `#[cfg(all(test, feature = "integration-tests"))] mod quint_gateway_identity;`. `dispatch_issue` stays `pub(crate)` (reachable in-crate from the `#[cfg(test)]` child module; no visibility change, no new public API; on main the analogous `issue_svid` dispatchers are `pub` in a `pub` module, `crates/overdrive-control-plane/src/action_shim/issue_svid.rs:80,145` and `action_shim/mod.rs:232`, so the narrower visibility here needs no widening). The driver's read of `ObservationStore::issued_certificate_rows` (QC-1 `audited`) reads the owner's own output surface through the port trait (the owner wrote those rows), not a `Sim*` arbiter, so no arbiter marker or Tier-3 pairing applies and the dataplane `GatewayIdentitySlot` is reached through the existing `GatewayIdentityActionTarget`/`GatewayIdentityLifecycleTarget` ports. It is not a `tests/conformance` driver: it does not run through the exported server handler. The driver struct holds the production `ControlPlaneGatewayIdentityLifecycle` over a real `GatewayIdentitySlot` and `EvaluationBroker`, the production `GatewaySvidLifecycle` reconciler (its `hydrate_*`/`reconcile` produce the `Action` passed to `dispatch_issue`; the driver never constructs an action), a Sim CA, `SimObservationStore` and `SimClock` at their driven ports. `audit_fail` is a typed write fault on `SimObservationStore`, an existing driven port.
- **`NoAllocationIdentity`** is not modelled and not projected: no slot or lifecycle action takes or produces an `AllocationId`, so the property is a type-level fact of the interface (`GatewayIdentitySlot` is keyed by node, ADR-0225). DISTILL carries it as a compile-time obligation only if the contract does not already forbid it: the slot's public API has no `AllocationId` parameter, so no trybuild fixture is added (a fixture would test the absence of a parameter the type never had). Dropped as an obligation.

### Module `gateway_application` — decision ADR-0222 (drain and supersession only)

- **Protocol modelled.** The concurrent part of `GatewayApplicationOwner`: a new generation is staged, probed, promoted and the prior generation moves to Draining; connections hold leases on their generation; a draining generation is retired only when its lease count is zero, and eagerly: a demotion that happens at zero leases retires the generation in the same owner step, and while a retirement is pending no other step runs before it; a probe result that arrives after its staged generation was superseded is discarded; a candidate whose gate fails (gateway-local reasons only, R-65) is recorded by its Route generation (`failed_rg`) and is not re-staged until the Route changes (`route_declare` or `route_withdraw`; the Route is the only owner input the model represents, so the certified-key and Gateway Identity clears of Gate G-1 are not modelled). `route_declare` models the owner's observation of the Route: a gate result for a candidate whose Route the owner has not yet consumed may promote it, and one for a candidate whose Route it has consumed is discarded.
- **Variables and pinned accessors.** `current_generation`, `draining_generations` ← `GatewayStatusReads::application_status() -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError>`. Value mapping: the spec's `current` object `{gid, rg}` ↔ `GatewayApplicationGenerationStatus { application_generation, route_generation, .. }` (`gid` ↔ `application_generation`, `rg` ↔ `route_generation`); `draining` is the set of such objects. Classification of the remaining spec variables (`gateway_application.qnt` state `S`, `conf_step`): `staged` is not projected as State (the `stage` step asserts `application_status().staged` as its own postcondition); `inflight` (probe tokens outstanding) and `leases` (per-generation connection counts) are model-only (driver input state: the driver holds the leases and the probe parks); `route_on` and `route_gen` are model-only driver input state (the driver applies and withdraws the Route itself and counts its own applies; no pinned accessor reads the live Route generation, and none is invented); `next_gid` is model-only (the owner allocates `GatewayApplicationGenerationId`; its effect is visible only as the ids in `current`/`draining`); `conf_step` is model-only. **Generation naming (R-28, P-28; R-34).** The model's `gid` and `rg` values (1, 2, ..) are model names. `GatewayApplicationGenerationId` is a 32-byte content digest of `(route_generation, frontend, certified_key_generation)` and the owner mints it, so it has no start value and no chronological order (its derived `Ord` is the byte order of a digest). The driver therefore compares by **equality through a naming translator**: at each `stage` step it records the id the owner reports in `application_status().staged` against the model's `gid` (and the route generation against `rg`), and `from_driver` maps every id the owner reports in `current` and `draining` back to a model name through that translator. An id the translator has never seen is a failed comparison. The translator is populated only from values `application_status()` reported; it never supplies a value, never orders ids, and `draining` is compared as a set. No start value is pinned and no new accessor is added.
- **Actions → driving ports.** `route_declare`/`route_withdraw` → `PublicRouteSetHandle` via `GatewayRouteCommands::routes()` (these trigger staging and supersession); `probe_result` → `GatewayUpstreamConnector::probe` over the driver's test-local `GatewayConnectDataplane` double (not `SimGatewayConnectDataplane`; see "Conformance placement and test-local doubles (DG-7)" below); `open_connection`/`close_connection` → `GatewayAdmission::acquire_connection` / lease drop.
- **Conformance placement and test-local doubles (DG-7).**
  - **Facts.** `overdrive-gateway` owns its port traits (`overdrive-gateway::ports`), and `overdrive-sim` implements them (`SimGatewayConnectDataplane`, `SimGatewayRelay`, `SimGatewayClientMtls`, the Sim identity lifecycle control), so `overdrive-sim` depends on `overdrive-gateway`. A driver compiled inside the library's own unit-test build (`cfg(test)`, the placement `.claude/rules/testing.md` § "Quint conformance" requires) sees two distinct `overdrive_gateway` crates when `overdrive-sim` is a dev-dependency: the test build of the crate under test, and the copy `overdrive-sim` links. A Sim type implements the second copy's traits, so it does not satisfy `crate::ports::*`. Sims of `overdrive-core` traits (`SimClock`, `SimObservationStore`, `SimEntropy`, the Sim CA) are unaffected: `overdrive-core` is one copy. The repository already meets this in `overdrive-control-plane`: `src/guest_network/quint_cid_lease.rs` takes every core-trait adapter from `overdrive-sim` and defines a driver-local `HeldProvisioner` for the crate-local `GuestNetworkProvisioner` (`overdrive-sim` depends on `overdrive-control-plane`); the `overdrive-sim` imports of the other in-crate drivers (`quint_flows_beacon.rs`, `quint_quiescence.rs`, `quint_owner_flows.rs`, `quint_fwd_pair.rs`) were checked by name only: they are Sims of `overdrive-core` traits (`SimBeaconIo`, `SimMtlsResolve`, `SimClock`, ...) or of a trait owned by a different crate (`SimMtlsIntercept` implements an `overdrive-worker` trait and is used from `overdrive-control-plane`). The integration binaries of `overdrive-gateway` (`tests/integration/*.rs`) are separate crates linking one `overdrive_gateway` copy, so they use the `overdrive-sim` gateway adapters without a problem.
  - **Resolution.** The QC-2 driver defines its own private doubles of the gateway-crate driven ports it composes the real owner over (`GatewayConnectDataplane`, `GatewayRelay`, `GatewayClientMtls`, `GatewayIdentityLifecycleControl`), inside the driver module. They are driven-port fault injection, not a second implementation of the protocol: staging, supersession, leases, drain and retire are the production `GatewayApplicationOwner`'s. The held-probe capability of the DISTILL shaping (hold each probe as its own entry; release one entry with a usable or unusable outcome; observe the held count) is realised by the connect double, parked in the awaited operations the gate owns with entries identified by the staged gate token (Gateway upstream path, "Where a double may park a probe (DG-9)"; R-65 removed `GatewayClientMtls::authenticate` from the gate): the connect double answers every `open_upstream` with a `PreparedUpstream::new(TcpSocket::new_v4()?, intent)` (DG-8) and every `take_receipt` with a receipt of either outcome; a held probe needs no `Selected` receipt and the client double is not used by the gate. The identity double answers `ensure_current` immediately with current facts and `subscribe` with them; the relay double's `probe` succeeds. Methods the QC-2 path does not exercise return the port's typed unavailable error. Real mTLS, relay and receipt paths are not QC-2's: they are covered by the seeded invariants, the host/Sim equivalence tests and Tier 3. `SimClock`, `SimObservationStore` and the real redb Route store come from their usual crates.
  - **What this adds.** No public production item; three crate-private production items (`GatewayAdmission::new`, `GatewayAdmission::budget`, `GatewayBuilder::start_with`, the DG-10 bullet below, each with a production caller in `start`); no `overdrive-sim` change, no new feature, no `Cargo.toml` change beyond the dev-dependencies the crate already needs. Accessor shapes are unchanged: `GatewayStatusReads::application_status()` (QC-2 `State` projection) stays as pinned. The Sim connect-probe hold/release surface stays in `overdrive-sim` for the seeded invariants and the `overdrive-gateway` integration binary (S-PIG-68); the QC-2 driver does not use it. The duplication is the accepted cost: the doubles are checked by nothing but their use, so they must stay as small as the path above.
  - **Rule for the driver module.** It imports nothing from `overdrive_sim` that implements a gateway-crate trait; only core-trait Sims. A reviewer or the crafter checks this with a grep for `overdrive_sim::` in the module; `from_driver` reads only `application_status()`, never a double.
  - **Driver path.** `crates/overdrive-gateway/src/application/quint_gateway_application.rs`, declared from `application` as `#[cfg(all(test, feature = "integration-tests"))] mod quint_gateway_application;`. It is a child of the module that owns `GatewayApplicationOwner` and its admission state, as QC-1 and QC-4 are children of their owners, so crate-private `GatewayAdmission::acquire_connection` and `GatewayConnectionLease` are reached in-crate with no visibility change.
  - **Rejected.** (a) `overdrive-sim` gateway adapters behind a gateway test-support feature: puts Sim code in a production crate and breaks the sim/host split. (b) Driver in `overdrive-control-plane`: `acquire_connection`, the lease and the admission are crate-private to `overdrive-gateway`, so it needs new public surface used only by tests, and the control plane is not the owner of this protocol. (c) Widening those items to `pub` for the Sim or another crate: a test seam in production API. (d) Moving the gateway ports into `overdrive-core` or a new ports crate: restructures the ADR-0222 crate boundary, and `ports` carries `rustls`, `OwnedFd` and `tokio` types that `overdrive-core` forbids.
  - **Admission access (DG-10; crate-private, pinned because production uses it).** Facts: the driver must hold the `Arc<GatewayAdmission>` that the owner publishes into and the listener acquires from. `GatewayAdmission` has private fields and no pinned constructor; `StartedGateway::into_parts` yields only `GatewayHandle`, `GatewayRouteCommands` and `GatewayStatusReads`; the owner is moved into its task by `GatewayBuilder::start`; and the two ports (the pinned `State` projection and `routes()`) have no constructor other than `start`. So the driver can neither read the admission off a running composition nor rebuild the composition around its own owner. The answer is to inject, not to expose: the admission is created before the composition and handed to it, which `start` must do anyway because the owner and the listener share one `Arc<GatewayAdmission>`. Pinned (crate-private; every item has a production caller):
    ```rust
    // overdrive-gateway, crate-private
    impl GatewayAdmission {
        /// `current` empty. The one constructor.
        pub(crate) fn new(budget: Arc<GatewayAdmissionBudget>) -> Arc<Self>;
        /// The budget this admission was built over: the one shared with the connector and every generation snapshot.
        pub(crate) fn budget(&self) -> &Arc<GatewayAdmissionBudget>;
    }
    impl GatewayBuilder {
        /// `start` is exactly: build the one budget from `dependencies`' limits, `GatewayAdmission::new(budget)`, `start_with`.
        pub(crate) async fn start_with(self, dependencies: GatewayRuntimeDependencies,
            admission: Arc<GatewayAdmission>) -> StartedGateway;
    }
    ```
    Production callers: `start` calls `new` and `start_with`; `start_with` calls `budget()` to hand the one budget to the connector, the cleanup path and the generation snapshots. Invariants: `start_with` creates no second admission or budget; the `admission` it receives is the one given to the owner and to `GatewayListenerControl::bind`; the admission's budget was built from the same `GatewayLimits` value as `dependencies`' (the caller's obligation, as for `start`). The driver builds one `GatewayLimits`, `GatewayAdmissionBudget::new(Arc::new(limits.clone()))`, `GatewayAdmission::new(..)`, keeps the `Arc`, obtains the builder through the public `UnboundGatewayBuilder::new(..).await` and `into_builder()` (core-trait Sims for the observation store and clock; a real redb Route store), calls `start_with`, and takes `GatewayRouteCommands` and `GatewayStatusReads` from `into_parts()`. `open_connection` is then `admission.acquire_connection(now)` on the retained `Arc`. No accessor exists whose only caller is the driver. Consequence for the driver, not a contract: the composition is the production one, so promotion binds the configured address on TCP/443 through the real `GatewayListenerControl`; DISTILL decides how the driver isolates that real bind.
- **Invariants.** `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` (both defend ADR-0222's single decision: the owner moves the prior generation to Draining, retires it after its users reach zero, and never restores a superseded generation).
- **Named runs.** `replace_valid` (supersede, drain, retire), `route_withdraw_drain` (withdraw, drain, retire) and `late_probe_after_supersession` (a probe result for a superseded staged token is discarded).
- **Hazard (one per decision).** ADR-0222: `retireWhileLeased` (retire a draining generation while its lease count is non-zero) → `DrainRetainsUntilLeasesZero`.
- **Not modelled (DISTILL seeded-sim or unit obligations instead).** Route/frontend/key whole-object coherence (`CurrentIsWholeObject`; a single task publishes one immutable value, a pure construction), bind gating (`ListenerBoundImpliesGates`, ADR-0230), last-usable preservation (`ReplacementPreservesLastUsable`, ADR-0231; a single-owner sequential install), identity-loss unbind, cold-boot order, and all shutdown ordering (`UsersJoinedBeforeIdentityRetire`, `IdentityRetiredBeforeConvergenceStops`, `shutdown_order`; the shutdown actions and stage projection are removed).

### Module `selection_receipt` — decisions ADR-0232, ADR-0233, ADR-0241

ADR-0228 (the Dataplane owns the selection receipt) is an ownership decision with no interleaving of its own, and ADR-0248 (the connector reserves its ledger slot, then calls `open_upstream`, then binds the intent) is a sequential rule of one connector task. Neither carries a model hazard: 0228 is exercised through the receipt invariants below, and the ledger order is covered by the seeded `overdrive-sim` invariants S-PIG-37 and S-PIG-53.

- **Variables and pinned accessors.** `identity[id]` ← `GatewayConnectDataplane::selected_backend_identity(BackendId) -> Result<SpiffeId, ..>`. `intents` ← `live_intent_count()`; `receipts` ← `GatewayConnectCleanup::live_receipts()`. The receipt value ← `take_receipt(&intent)`. **Classification of every spec variable (the spec's names; `selection_receipt.qnt`):**

  | Spec variable | Class | Source |
  |---|---|---|
  | `ident` (the spec's `identity[id]`) | projected | `GatewayConnectDataplane::selected_backend_identity(BackendId)`; value mapping: model `BackendId` n ↔ `BackendId::new(n)` (every `u32` is valid, `overdrive-core/src/id.rs`). This holds because the real `BackendIdAllocator` (`overdrive-dataplane/src/allocators/backend_id.rs`) starts its counter at 1, is monotonic and never recycles a number (`release` drops only the memo entry), a fresh `EbpfDataplane` (and so a fresh allocator, as after `restart`) starts at 1 again as the model's `restart` does, and the `receipt_conf` catalogue (conformance main, empty or singleton sets) puts at most one backend in each update, so the order in which a single update assigns ids is deterministic; model alloc ↔ the `SpiffeId` of that allocation (the driver supplies the allocations, so it knows the expected `SpiffeId`) (`Ok(spiffe)` ↔ bound, `BackendIdentityMissing` ↔ absent) |
  | `intents` | projected (as a count) | `live_intent_count()` |
  | `receipts` (cookie to selected `BackendId` or `NoBackend`; the spec's `sel`) | projected per cookie (R-33) | `GatewayConnectCleanup::live_receipts()`, the read-only snapshot pinned with the cleanup-only capability (Gateway upstream path into leg-C, R-29/R-33); it is the one receipt accessor (a residual count is its `len()`) |
  | the identity read (`read_identity`) | a step, no spec variable (R-33) | the spec has no `idres` variable: a variable that holds the result of an accessor already projected as `ident` would compare the accessor with itself. `read_identity` is a step whose handler calls `selected_backend_identity(BackendId)` for the cookie's `Selected` backend and asserts nothing about the return; the values the State comparison checks are the per-cookie `sel` (through `receipts`) and `ident`. The publish-guard window is Tier 3 (S-PIG-71's concurrency test) |
  | `taken` | derived, not State (R-33) | the receipt `take_receipt(&intent)` consumes is the one the hook recorded: the `take` handler reads `live_receipts()` first and asserts the consumed receipt equals that cookie's entry (both owner-sourced; the model's `sel` was already compared through `receipts` at the preceding step) |
  | `outer` (the kernel map generation, A-K1), `resv`, `infl`, `guard`, `g`, `ckey`, `conf_step` | model-only | not projected; no owner accessor is pinned for them (the prior text named `outer_committed` and `reserved`: those are the spec's `outer` and `resv`) |
  | `upd` | model-only | the spec's exploration bound on accepted `update_service` calls (`MAXUPD`); the driver counts its own calls |
  | `next_cookie`, `conn` | model-only | driver input state: the model cookie `c` maps to the `GatewayConnectIntent` the driver holds; the real `SocketCookie` is the kernel's and is not controlled. The `lost` value of `conn` after `restart` is observable only through `intents`/`receipts` being 0 |
  | `memo`, `next_bid` | model-only (R-27, P-27) | the allocator memo and counter. Run `restart_destroys_receipts` asserts `next_bid == 1` after `restart` and other runs assert `memo`; the model checker evaluates those assertions on the model. No accessor exposes either and none is added. The observable consequence is compared through `ident` and through the `BackendId` each live receipt carries in the projected `receipts` map (R-33), so an allocator that recycles ids across a restart or an orphan release is detected as a different `BackendId` for the same backend, not as a memo difference. **The model follows the production allocator exactly (R-38):** the memo entry of an endpoint is released when no service's backend set still contains the endpoint after a commit (orphan GC, `release` in `update_service`), the counter never recycles a number, and `ident` keeps historical Applied identities for old receipts. |
  The kernel map generation is assumption A-K1; the driver does not project the model-only variables.
- **Actions → driving ports (the `receipt_conf` actions only).** `update_apply`, `update_rejected`, `update_rejected_rnat` → `Dataplane::update_service`; `open` → `GatewayConnectDataplane::open_upstream`; `kernel_connect` → the `connect(2)` the driver issues on the prepared socket (the exhaustive action map below is normative); `take`, `cleanup` → the port methods; `read_identity` → `selected_backend_identity`; `restart` → drop and re-create the dataplane instance. Runs needing a fault or pause seam (P-32) are not replayed.
- **Invariants.** `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `ReverseNatKeyOneVip` (R-59: no reverse-NAT key, i.e. no memoised `BackendId`, appears in the committed generations of two Services), `RollbackPreservesPriorGeneration`, and `EmptyIsNoBackend` (no outer entry or no usable backend in the chained lookup yields `NoBackend`). The arm ignores `BACKEND_MAP.healthy` (R-2), so there is no healthy-dependent invariant and no `unhealthy_is_no_backend` run.
- **Named runs.** `commit_then_select`, `abandoned_reservation_rolled_back`, `conflicting_alloc_rejected`, `reverse_nat_conflict_rejected`, `identity_conflict_precedes_reverse_nat`, `receipt_waits_for_publish_guard`, and `restart_destroys_receipts`; the `receipt_conf` run `conflicting_alloc_rejected` also carries the empty-update tail of the rejection scenario (R-36).
- **Hazards.** ADR-0233: publish `Applied` before the outer swap. ADR-0241: rebind a live id to another allocation. ADR-0232: take a receipt that is not bound to its registered cookie and `ServiceKey`. R-59 (reverse-NAT key held by one VIP): `receipt_bugRnatShare` (check `receipt-hazard-rnat-share`, expect violation) lets an update through whose endpoint is already held by another Service and violates `ReverseNatKeyOneVip`.
- **R-59 rule and check order.** `update_service` plans in one pass before any mutation: (1) the identity-conflict check (`BackendIdentityConflict`, ADR-0241), then (2) the R-59 check (`ReverseNatConflict { key, current_vip, attempted_vip }`). When both are violated the outcome is `BackendIdentityConflict`. The model encodes this order (the run `identity_conflict_precedes_reverse_nat` ends on a successful `update_rejected` and does not itself assert that the R-59 rejection is disabled for that input; the precedence rests on the `not(conflict)` guard below): `update_rejected_rnat(key, sidx)` is enabled only when there is no identity conflict and the endpoint's reverse-NAT key is held by a Service other than `key`; `update_apply` and `update_begin` additionally require not(rnat). Replacing a Service's own backend set is not sharing. Model-vs-production assumption: the model's reverse-NAT key is the endpoint's memoised `BackendId`, and a key "held" by another Service means that `BackendId` is in that Service's committed generation (`outer`); the production check compares the reverse-NAT key against the reverse-NAT map's current VIP. This holds because the reverse-NAT key is derived one-to-one from the backend's `BackendId` and the map is written only by committed generations (the failed-rollback orphan keys of R-21 are unreferenced debt and are not "held").
- **Conformance owner (D-PASS7-2).** The conformance owner is the real `EbpfDataplane` (it implements both `Dataplane` and `GatewayConnectDataplane`), on native metal, with the real `cgroup/connect4` selection arm loaded. No `Sim*` adapter is the owner or a projection source: a Sim-only owner would compare the model with the Sim's own modelled arm. Driver: `crates/overdrive-dataplane/src/quint_selection_receipt.rs`, `#[cfg(all(test, feature = "integration-tests"))]`, run through `cargo xtask metal run --` (root, bpffs, cgroup v2). It is `#[ignore = "pending DELIVER quint-conformance-selection_receipt"]` until the separate gateway selection program (R-1) and the owner are GREEN.
  - **Conformance main (modeler change).** A new main `receipt_conf` records `conf_step` over a *coarsened* step relation, because only observable owner transitions can be replayed: (1) `update_apply(key, sidx)` fuses `update_begin; commit(true); publish` (the write guard is held across a single non-yielding synchronous section, so the intermediate states are not observable and cannot be paused without a test seam); (2) `update_rejected` and `update_rejected_rnat` (R-59) stay; (3) `open`, `kernel_connect(c)`, `take(c)`, `read_identity(c)`, `cleanup(c)`, `restart` stay (`idle` is excluded from this main, R-7); (4) `rollback` and `commit(false)` are removed from this main; (5) `take(c, r)` is restricted to `r == c` (a mismatched receipt cannot be requested through `take_receipt(&intent)`); (6) the conformance catalog contains only empty and singleton backend sets, so the Maglev `pick` is determined by the state and is not an input the driver cannot control. (7) the allocator follows production exactly (R-38): after every commit, including the empty-set commit, a memo entry is released when no service's committed backend set still contains its endpoint, the counter `next_bid` never recycles a number, `ident` keeps historical Applied identities for old receipts, and the identity-conflict check consults only the live memo, so an endpoint released by orphan GC may be bound to a different allocation by a later update; (8) the existing run `conflicting_alloc_rejected` carries the empty-update tail of the rejection scenario (R-36): after the rejected update, an empty update releases the memo entries, `ident` keeps its history, the counter does not recycle a number, and the next fresh backend receives the id `next_bid` held before the rejection. The exhaustive `receipt_ok` main and all hazards keep their rules; the memo-release rule of (7) applies to every main of the module.
  - **Action to driving-port map.** `update_apply`, `update_rejected`, `update_rejected_rnat` → `Dataplane::update_service` (`update_rejected` is the real `DataplaneError::BackendIdentityConflict` from a conflicting backend set; `update_rejected_rnat` is the real `DataplaneError::ReverseNatConflict` (R-59) from a backend set naming an endpoint the committed generation of another Service already holds; the driver feeds the second Service's `key`/`sidx` and asserts the variant of the returned error); `open` → `GatewayConnectDataplane::open_upstream`; `kernel_connect` → the driver issues the `connect(2)` on the socket returned by `PreparedUpstream::into_parts` toward the Service frontend (the same single syscall `GatewayUpstreamConnector::connect` issues; the connect result is not asserted for success, the hook has already written the receipt). The driver's `connect(2)` is deadline-bounded: a non-blocking connect with a bounded wait; an absent listener yields a refused/timeout outcome mapped to the model's failed dial, so a missing listener cannot stall the driver; `take` → `take_receipt`; `read_identity` → `selected_backend_identity`; `cleanup` → `cleanup`; `restart` → drop and re-create `EbpfDataplane` on the same pin directory (SR-2). `receipt_conf` has no `idle` step (R-7).
  - **State projection (owner accessors only).** `identity[id]` ← `selected_backend_identity(BackendId)` (`Ok` → bound, `BackendIdentityMissing` → absent); `intents` ← the *count* `live_intent_count()`; `receipts` ← the per-cookie snapshot `GatewayConnectCleanup::live_receipts()` (R-33: for each model cookie `c`, the driver looks up the `socket_cookie` of the `GatewayConnectIntent` it holds for `c`, and the entry's `service_key` and outcome map to the spec's `{key, sel}`; the spec's `sel` is the `BackendId` of a `Selected` outcome and 0 for `NoBackend`). `taken` is not projected State; it is a function of projected State at its own step and is asserted by its handler (R-33): the `take` handler asserts that the receipt `take_receipt(&intent)` returns equals the cookie's `live_receipts()` entry read immediately before the call (the spec's `taken[c]` is that entry). The spec has no `idres` variable: `read_identity` is a step whose handler calls `selected_backend_identity(sel)` for the cookie's `Selected` backend and asserts nothing about the return; the projected `ident` and the per-cookie `receipts` are what the State comparison checks after the step. The model's own invariants `ReceiptMatchesIntent` and `IdentityReadableOnlyAfterCommit` constrain the same values on the model side, and the spec README states the same classification. Cookie-indexed bookkeeping (`c` → held `GatewayConnectIntent`) is driver input state and is never projected. `outer` and `resv` (and every other variable classified model-only in the table above) stay model-only; the table above is the single classification.
  - **Rejection outcome projection (`update_rejected`, `update_rejected_rnat`).** Neither rejection changes a spec variable; its observable is the conf_step variant, which the driver replays as the typed `Err` the driving port `Dataplane::update_service` returns (`BackendIdentityConflict`, `ReverseNatConflict`), asserted by the handler (the same pattern for both; no accessor is pinned or added for the outcome). The unchanged state is checked by the State comparison after the step through the pinned `ident`, `intents` and `receipts` accessors. R-59's `ReverseNatKeyOneVip` has no spec variable of its own to project: it is `outer`, which is model-only, so the invariant is held on the model and its observable consequence is that each Service still selects its own backend (the per-cookie `receipts` and `ident` after `open`, `kernel_connect_det`, `take`, `read_identity`, as the run `reverse_nat_conflict_rejected` ends).
  - **Not conformance-replayable, and how they are covered.** `abandoned_reservation_rolled_back` and the `rollback` action: the pre-commit and post-commit failures need a fault the owner cannot be handed through an existing driven port. They carry no Tier-3 test (R-21, P-21). The only pre-commit failure a test can produce through the existing `Dataplane::update_service` input is `BACKEND_MAP` capacity exhaustion (`crates/overdrive-bpf/src/maps/backend_map.rs:46`, `MAX_ENTRIES = 65_536`, failing at `ServiceApplyStage::BackendMap`); a single-node VM platform does not approach 65,536 distinct backend endpoints, and no seam is added to inject another failure. **Accepted exposure:** `RollbackPreservesPriorGeneration` has model coverage (the run `abandoned_reservation_rolled_back`) and the host/Sim `Dataplane` equivalence lane, and no real-kernel evidence. The exposure is bounded: a pre-commit failure happens before the one outer `SERVICE_MAP` swap, so the prior generation stays selectable by construction; a rollback delete that itself fails leaves only unreferenced `BACKEND_MAP` or reverse-NAT keys, recorded as closed cleanup debt and removed by the next orphan sweep, and exposes no Applied identity for the abandoned ids. **Runs needing a fault or pause seam, and `#[quint_test]` (R-36).** `abandoned_reservation_rolled_back` and `receipt_waits_for_publish_guard` need steps (an injected pre-commit failure; the commit guard held open between the outer swap and Applied publication) that have no owner call without a production fault or pause seam. A `#[quint_test]` for either would need a handler that fabricates the fault, which `.claude/rules/testing.md` § "Quint conformance" rejects, so neither is replayed. **The rollback path (the abandoned reservation) is model-only:** `update_rejected` on `receipt_conf` fails in the owner's identity-conflict preflight, before any identity is reserved, so replaying it exercises the rejection, not the rollback. What `receipt_conf` replays of that neighbourhood is the rejection and its aftermath, in the existing run `conflicting_alloc_rejected` (an `update_rejected` followed by an empty `update_apply` and an `update_apply` of a fresh backend: the memo entries are released, `ident` keeps its history, the counter is not recycled, and the fresh backend receives the id `next_bid` held before the rejection, so the rejected update consumed no id). The publish-guard scenario has no `receipt_conf` form (`update_apply` is one atomic step by design); its real-kernel evidence is the Tier-3 concurrency test of S-PIG-71 plus the `publishEarly` hazard. The runs that carry `#[quint_test]` are `commit_then_select`, `conflicting_alloc_rejected`, `reverse_nat_conflict_rejected`, `identity_conflict_precedes_reverse_nat` and `restart_destroys_receipts`, all on `receipt_conf`; `#[quint_run]` randomized traces remain. Whether to add a fault or pause seam so those two runs can be replayed is the PROPOSED item P-32. Post-commit cleanup debt is unmodelled in the spec (no invariant depends on it) and is covered by the equivalence/unit lane. `receipt_waits_for_publish_guard`: the guard window cannot be held open without a test seam, so it is covered by a Tier-3 concurrency test (updates racing `selected_backend_identity` for a just-selected id; the read never returns a missing identity for a `Selected` id) plus the `publishEarly` hazard. The `takeAny` hazard has no driver path by design. Both remain model-checked runs; the `checks.toml` header is the single home of the check inventory.
  - **Bound runs.** `#[quint_test]` for `commit_then_select`, `conflicting_alloc_rejected`, `reverse_nat_conflict_rejected`, `identity_conflict_precedes_reverse_nat` and `restart_destroys_receipts` (R-36, R-59; all on `receipt_conf`, as the modeler change above states); `#[quint_run]` randomized traces on `receipt_conf`. The host/Sim `GatewayConnectDataplane` equivalence test remains the Sim pairing; it is not a conformance test.

### Conformance action maps for the three older mains (closes per-driver checklist item 3)

This section is the exhaustive action map for `identity_conf`, `app_conf` and `receipt_conf`; it supersedes the shorter
"Actions → driving ports" bullets above where they differ. Each row names the owner call; the State comparison after every step
reads only the accessors pinned above. Two kinds of step are not stimuli and are pinned here once.

**No stutter step in the conformance mains (R-7).** The stutter `idle` is guarded `not(REC)` in the three step relations, so it
exists only in the exhaustive mains (for example `receipt_ok`) and in none of `identity_conf`,
`app_conf` and `receipt_conf`; no conformance step maps to "no call" and no `idle` row exists in the tables below. This is the
same shape `relay_exchange` uses.

**Owner-autonomous step (QC-2 `stage` and `retire`; R-7: they stay explicit steps and are not coarsened into `route_declare` and
`close_connection`).** These transitions are not commands any port accepts: the owner stages a
candidate by itself after a Route change and retires a Draining generation by itself when its leases reach zero (Application
section, "owner alone waits and then retires"). The driver runs the owner's task on a paused single-thread runtime, so the owner
acts only when the driver yields. The handler yields until the owner is idle and then asserts the step's own postcondition through
the pinned accessor; it is not a bookkeeping update. Retirement is eager: a demotion that happens at zero leases retires the
generation in the same owner step, and while a retirement is pending no other step runs before it.

**`identity_conf` (owner: `ControlPlaneGatewayIdentityLifecycle` over a real `GatewayIdentitySlot`, `EvaluationBroker`, the production `GatewaySvidLifecycle`).**

| Spec action | Owner call |
|---|---|
| `submit_evaluation` | `GatewayIdentityLifecycleControl::ensure_current(deadline)`, polled once so the retained `Evaluation` is submitted and not awaited to completion; the driver then drains the broker and runs the production reconciler's `hydrate_*` and `reconcile` at the `SimClock` tick and queues the `IssueGatewaySvid` it emits (the driver never constructs one); the handler then asserts that no queued, not-yet-completed `IssueGatewaySvid` carries an epoch beyond `GatewayIdentitySlot::desired()`'s epoch (a handler has no model state, so `issues` is not compared; a duplicate or early Issue shows through `audited`, residual exposure in the classification row; the literal projection, P-37, was rejected on 2026-11-04) |
| `issue_complete(e)` | `action_shim::gateway_svid::dispatch_issue` with a queued `IssueGatewaySvid` of epoch `e` (the spec guard `issues.contains(e)` is "such an action is queued"); the driver then drops every pending Issue action of epoch `e` |
| `disable` | `GatewayIdentityLifecycleControl::disable_after_drain()`: a synchronous clear, so `GatewayIdentitySlot::current()` is `None` and `desired()` is disabled with the bumped epoch when it returns (R-4) |
| `advance_time` | `SimClock::advance` (the owner reads time at its next call) |
| `audit_fail` | arm a failing write on `SimObservationStore` for the next `IssuedCertificate` row (driven-port fault) |
| `restart` | drop the owner composition and rebuild it: `GatewayIdentitySlot::new` (epoch 1, enabled, empty), a new `EvaluationBroker` and `ControlPlaneGatewayIdentityLifecycle::new`, over the same `SimObservationStore` (audit rows are durable) and the same CA serial counter; queued `Action`s and a pending `ensure_current` future are discarded with the process |
**`disable` is a synchronous clear (R-4, B-1).** `disable_after_drain` = `begin_disable` clears the volatile slot and bumps the
epoch in the same call, with no evaluation, no wait and no reconciler drop ([ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md)
Consequences). `gateway_identity.qnt` models exactly that: `disable` sets the epoch, `enabled = false` and an empty `held` in one
step; there is no `drop_complete`, `settle_ok` or `settle_timeout` step and no drop `Action` in the design. The runs
`restart_reissue` and `disable_races_inflight_issue` expect `held` empty immediately after `disable`; an in-flight Issue completing
afterwards is a stale-epoch no-op.

**`app_conf` (owner: the real `GatewayBuilder`/`GatewayApplicationOwner` composition over a real redb Route store, the driver's test-local gateway-port doubles (DG-7, "Conformance placement and test-local doubles" above; `SimGatewayConnectDataplane` cannot be used in the in-crate test build), `SimClock`).**

| Spec action | Owner call |
|---|---|
| `route_declare` | `PublicRouteSetHandle` apply of a Route (declare or replace) via `GatewayRouteCommands::routes()`; the owner has not yet run |
| `route_withdraw` | `PublicRouteSetHandle` withdraw via `GatewayRouteCommands::routes()`; then the owner runs to idle (withdrawal moves Current to Draining and discards staged) |
| `stage` | owner-autonomous step: yield until idle; postcondition `application_status().staged` is `Some` for the live Route generation |
| `probe_result(tok, ok)` | complete the held probe of the staged candidate `tok` with outcome `ok` through the hold/release capability of the driver's test-local `GatewayConnectDataplane` double (DG-7; not the `overdrive-sim` adapter), so `GatewayUpstreamConnector::probe` returns it to the owner; then the owner runs to idle. For a superseded `tok` the owner must discard the result and leave Current, staged and Draining unchanged ("a late gate result cannot restore a superseded generation", Application section); that is the modelled discard and the State comparison checks it. The park points are the awaited operations the gate still owns, not `GatewayClientMtls::authenticate` (R-65; Gateway upstream path, "Where a double may park a probe (DG-9)"), and a held entry is addressed by the gate token the owner staged, not by arrival position or a backend identity; a held probe requires no `Selected` receipt. Releasing a superseded `tok` must follow the owner's consumption of the superseding Route event: a real owner that has not yet consumed it may correctly promote the candidate it holds (Gate G-1, failed-candidate rule), so the driver orders the owner's observation of the `route_declare` before the release. The mapping from `tok` to the held operation and the scripting surface are test support DISTILL shapes inside the driver module |
| `retire(gid)` | owner-autonomous step: yield until idle; postcondition `application_status().draining` no longer lists `gid` |
| `open_connection` | `GatewayAdmission::acquire_connection(now)`; the driver keeps the returned `GatewayConnectionLease` |
| `close_connection(gid)` | drop the kept `GatewayConnectionLease` of generation `gid`; the owner has not yet run |

State comparison: `current_generation` and `draining_generations` from `GatewayStatusReads::application_status()` (`current`, `draining`),
after the step's owner run. `staged` and `leases` stay model-only; `stage` additionally asserts `status.staged` as its own postcondition.

**`receipt_conf` (owner: the real `EbpfDataplane`, native metal).** Action map unchanged from the bullets above (`update_apply`,
`update_rejected`, `update_rejected_rnat`, `open`, `kernel_connect_det`, `take(c,c)`, `read_identity`, `cleanup`, `restart`); it has no `idle` step (R-7).

### Model checks

Spec and checks: [`specs/quint/public-ingress-gateway/`](../../../specs/quint/public-ingress-gateway/README.md), `checks.toml`. **The check counts (per kind and per module) are stated once, in the `checks.toml` header; this document restates none of them.** Their expected outcomes are stated in `checks.toml`. Evidence is recorded at the spec hashes in `evidence/summary.md`; `cargo xtask quint verify-evidence` fails when it no longer matches the specs. `evidence/` is written only by `cargo xtask quint check --record` and is re-recorded whenever a spec changes. Verdicts are not restated in this document.

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
| Late probe result after supersession is discarded | `late_probe_after_supersession` |
| A failed candidate is not re-staged until the Route changes (R-65) | `failed_candidate_not_restaged` |
| Commit then select | `commit_then_select` (`selection_receipt`) |
| Abandoned reservation rolled back | `abandoned_reservation_rolled_back` |
| Conflicting allocation rejected; the rejected update consumes no `BackendId` (R-36) | `conflicting_alloc_rejected` (`receipt_conf`) |
| Endpoint already held by another Service is rejected with `ReverseNatConflict`; nothing changes and each Service still selects its own backend (R-59) | `reverse_nat_conflict_rejected` (`receipt_conf`) |
| Identity conflict and R-59 both violated: the replay asserts `update_rejected` is taken and the owner returns `BackendIdentityConflict`; precedence rests on the model's `not(conflict)` guard and the owner's typed error check (R-59) | `identity_conflict_precedes_reverse_nat` (`receipt_conf`) |
| Receipt waits for the publish guard | `receipt_waits_for_publish_guard` |
| Restart destroys receipts | `restart_destroys_receipts` |

| Decision | Hazard (violates) | Invariant that fails |
|---|---|---|
| ADR-0225 | `noEpochCheck` | `StaleEpochNeverHolds` |
| ADR-0222 | `retireWhileLeased` | `DrainRetainsUntilLeasesZero` |
| R-65 (failed-candidate rule, Gate G-1) | `restageFailed` | `FailedCandidateNotRestaged` |
| ADR-0233 | `publishEarly` | `IdentityReadableOnlyAfterCommit` |
| ADR-0241 | `rebind` | `IdentityImmutablePerBackendId` |
| ADR-0232 | `takeAny` | `ReceiptMatchesIntent` |
| ADR-0239 | `relayPastBoundary` (`relay_exchange`) | `NoBytePastMessageBoundary` |

R-64 (shutdown drain deadline and join reserve) is not a Quint protocol and changes no spec: the specs model the Application drain/supersession (`gateway_application.qnt`, whose header states shutdown ordering is outside its scope) and the exchange `drain_begin`/`force_abort` actions (`relay_exchange.qnt`), and none has a clock, deadline or task-join variable; the deadline split is sequential arithmetic in one shutdown task, covered by S-PIG-60, 101 and 115.

Decisions with no hazard, and why: ADR-0228 (ownership; no interleaving of its own), ADR-0248 (single-task sequential ledger order; seeded invariants S-PIG-37 and S-PIG-53), ADR-0247 (kernel-buffer property the model abstracts), ADR-0254 (post-`Closed` I/O; Tier-3 S-PIG-115), ADR-0240, ADR-0244, ADR-0249, ADR-0250, ADR-0245, ADR-0251, ADR-0252, ADR-0253 and ADR-0246 (typestate, kernel-contract and pure-function properties, above).

Validation budget (design.md cost budgets): the check counts and timings of the whole subsystem are stated once, in the `checks.toml` header. Only `receipt-safety-tlc` is slow (non-CI); every other check is CI. The arm ignores `healthy` (R-2), so the spec carries no healthy-dependent invariant or run. Growth beyond this budget needs user approval. Open points and their resolution: SR-2 (answered from source in the assumptions table); GA-1 and SR-2 discharging items (DISTILL obligations in the table; GA-2 dropped, RAII); `NoAllocationIdentity` (type-level, dropped). The shutdown projection and shutdown actions are removed from the model.

### Assumptions and their discharging items

| Id | Assumption | Discharged by |
|---|---|---|
| A-K1 | The outer `SERVICE_MAP` slot replacement is atomic for the hook's lookup; a hook that sees a new inner map sees fully populated slots. | V-GW-1, existing atomic-swap Tier-3 (`.claude/rules/testing.md`) |
| A-K2 | The pre-connect `SO_COOKIE` equals the cookie the hook reads for the same socket. | V-GW-5 HELD (10/10 plain, 10/10 prepared, 7.0.0-38; source: `net/core/sock.c:2109-2113`, `net/core/filter.c:5151-5154 @ v7.0`) |
| A-K3 | A socket created by a `serve` task belongs to the attach cgroup subtree. | Source-settled: the socket's cgroup is fixed at allocation from the creating task (`cgroup_sk_alloc` takes `task_css_set(current)->dfl_cgrp`, `kernel/cgroup/cgroup.c:7420-7431`, called from `sk_prot_alloc` in `net/core/sock.c:2330 @ v7.0`) and `connect4` runs against that socket cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`); with all 19 `serve` threads observed in `/overdrive.slice/control-plane.slice` (V-GW-2) the attached hook fires. The end-to-end serve-created connect is exercised by AT-PIG-E2E-1 (DISTILL native obligation). |
| A-K4 | After a received KeyUpdate the kernel fails RX reads with `-EKEYEXPIRED` and `poll` reports no read event until `TLS_RX` is re-armed (`net/tls/tls_sw.c:1369-1371`, `net/tls/tls_main.c:423-427`, `Documentation/networking/tls.rst:217-220` @ v7.0). Not modelled. | Tier-3 relay suite: a peer KeyUpdate inside a body, with a negative control that omits the re-arm and observes `EKEYEXPIRED` (spike findings (b)) |
| A-K5 | The kernel serialises each `sendmsg`/`setsockopt` call but not a multi-call logical write against a `TLS_TX` re-arm (`net/tls/tls_main.c:635-783 @ v7.0`); the gateway's per-leg lock is therefore load-bearing. Not modelled (R-19). | Tier-3 relay suite: KeyUpdate request while a large splice-out is in flight, `TlsDecryptError == 0` at the real peer |
| A-K6 | A splice out of a kTLS socket never returns octets of two records and never crosses a record boundary (`net/tls/tls_sw.c:2285-2327 @ v7.0`). Relied on by the one-unit abstraction of `relay_exchange`. | Tier-3 relay suite (spike (a) shape: byte-exactness over records of mixed size) |
| A-K7 | Leg-C (the upstream leg's peer) never originates a KeyUpdate: after the handshake its rustls `ServerConnection` is consumed by `dangerous_extract_secrets` (`crates/overdrive-dataplane/src/mtls/inbound.rs:503-524`), the kernel is armed once by `arm_ktls_tx_rx` (`mtls/ktls.rs:39-48`), and the kernel's TLS ULP never emits handshake records itself. Not modelled. | Tier-3 relay suite: inject a KeyUpdate at the upstream leg (a test peer standing in for leg-C) and observe that the relay answers it by the item-4 path with `TlsDecryptError == 0`; and observe that a production leg-C never sends one across a response of at least 16 MiB. Not source-settled for rustls internals (rustls originates a KeyUpdate only when the application calls `refresh_traffic_keys` or at a confidentiality limit, neither reachable once the connection is consumed); the Tier-3 observation is the discharge. |
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
| V-GW-3 | The registered arm returns `NoBackend` when there is no outer entry or no usable backend, and `Selected` once the backend is programmed; after `job stop` a still-present entry yields `Selected` then a connect reset (502). (The production arm does not read `healthy`, R-2; V-GW-3 observed the byte honoured by the probe arm and flapping 1 → 0 in 2 of 3 runs.) | A stopped backend produces a wrong-peer success. | The consuming-port outcome table (ADR-0242). |
| V-GW-4 | The chained `SERVICE_MAP` (hash-of-maps) → inner lookup verifies in a `cgroup_sock_addr` program and its verified-instruction count stays within the Tier-4 policy. | Verifier rejection, or a budget growth the policy does not authorize. | The registered arm is a second program beside the existing hook (R-1, ADR-0224). |
| V-GW-5 | The userspace `SO_COOKIE` before `connect(2)` equals `bpf_get_socket_cookie` in the hook. | Different values. | The intent key (ADR-0232). |

### Verdicts (run 2026-11-01, native metal, kernel 7.0.0-38-generic, tree `549d78fa6`)

Evidence: [findings-gateway-connect-path.md](spike/findings-gateway-connect-path.md) (V-GW-1..5) and [findings-vm-service-map-programming.md](spike/findings-vm-service-map-programming.md) (D4). Probes are throwaway under `spike-scratch/pig-vgw/` (aya Rust).

| Id | Verdict | What was observed | Contract effect |
|---|---|---|---|
| V-GW-1 | **PARTLY HELD**; remainder settled by source reading (below) | Selected receipt, rewrite to `100.95.0.2:18080`, `fwmark 0x1` + `127.0.0.1` source, accepted socket on leg-C (orig dst the guest address), TLS 1.3 server flight with the allocation chain. Not observed: client-SVID handshake and bytes at the guest (the dialer had no client certificate; leg-C refused `peer sent no certificates`). Control: the premise "an unprepared socket cannot reach leg-C" is false on a host with a default route (rationale corrected in step 2 above). | Preparation contract unchanged, rationale rewritten. |
| V-GW-2 | **HELD** (membership + proxy process, plus kernel source for the socket-cgroup rule) | All 19 `serve` threads in `/overdrive.slice/control-plane.slice`; the slice-attached hook counted a probe moved into that cgroup and not one outside. No connect created by a `serve` thread was observed (serve made none in the window); that gap is closed by source: a socket's cgroup is its creating task's cgroup at allocation (`kernel/cgroup/cgroup.c:7420-7431`; `net/core/sock.c:2330`; hook lookup `kernel/bpf/cgroup.c:1694 @ v7.0`), so membership implies firing. Contract shape source-settled; end-to-end observation of a serve-originated connect is a DISTILL native obligation (AT-PIG-E2E-1). | ADR-0224 alternative B (dedicated cgroup) is not needed; caveat recorded in ADR-0224 consequences; the stale comment at `mtls/mod.rs:928-931` is flagged there. |
| V-GW-3 | **HELD** | `NoBackend/no-outer-entry` → `NoBackend/healthy==0` → `Selected` about 1.05 s after the startup `Stable`; after `job stop` the entries persisted (about 12 s in run4, about 5 min in run2) and the result was `Selected` then `ECONNREFUSED`, never a wrong peer. `healthy` flapped 1 → 0 in 2 of 3 runs, cause not established. | ADR-0242 outcome table holds for the empty/no-outer-entry (503) and selected-then-dial-failure (502) rows; the `healthy == 0` row is 502 because the arm ignores `healthy` (R-2). The flap is an **observed exposure**: the gateway treats `healthy` as possibly flapping and never reads `Selected` as liveness. Recommend a separate investigation issue; it is not gateway scope (needs user approval before creation, as for P-4). Run4's V-GW-1 dials used a forced `healthy=1`. |
| V-GW-4 | **HELD** (verification) | The `SERVICE_MAP` → inner → `BACKEND_MAP` chain verifies; `vgw_select` 246 verified instructions vs 28 for the existing `cgroup_connect4`; production plus two probe programs coexisted under one cgroup (`multi`). | Placement ruled: a second program beside the existing hook (R-1). |
| V-GW-5 | **HELD** | Pre-connect `SO_COOKIE` equals the hook's `bpf_get_socket_cookie`, 10/10 plain and 10/10 prepared sockets. | ADR-0232 intent key sound on 7.0. |

### G1 contract-shaping items (run 2026-11-02, native metal, kernel 7.0.0-38-generic)

Evidence: [findings-ktls-splice-relay.md](spike/findings-ktls-splice-relay.md). Probes: `spike-scratch/gw-splice-a/`, `spike-scratch/gw-ktls-bpf/`. All are settled; none is carried to DISTILL as Open.

| Id | Item | Verdict | Contract effect |
|---|---|---|---|
| V-G1-a | Splice kTLS-RX to kTLS-TX; record boundary; non-DATA record | **HELD under a pipe rule; kernel defect found at v7.0** (silent loss of 5 octets per record when the pipe cannot map the record; fixed at v7.3) | ADR-0246: empty private pipe of at least 64 KiB, `len <= capacity`; probe pipe-floor and exactness steps |
| V-G1-b | TLS 1.3 kTLS incl. KeyUpdate mid-relay | **HELD** with `rustls::kernel::KernelConnection` (not `dangerous_extract_secrets`); EKEYEXPIRED until re-armed | ADR-0244; control-record contract; transmit lock |
| V-G1-c | Exact-length and per-chunk splice, backpressure, keep-alive | **HELD**: bounded memory, no over-read; **no CPU advantage**. Ran with `SPLICE_F_MORE` while `remaining > 0`; the no-flag configuration of the design is not spike-validated (falsification changes only the splice-out flag, not a contract; Tier-3 obligation 16(j)) | ADR-0239 consequence: no CPU claim; budgets |
| V-G1-d | connect4 rewrite then TLS handshake then kTLS on one fd | **HELD** (20/20) | Selection unchanged; arming after the rewritten connect |
| V-G1-e | kTLS plus sockmap | accepted one order at 7.0.0-38, rejected both at v7.3 | No sockmap on any leg (ADR-0244) |

Source reading added by this DESIGN (`vendors/linux` v7.0): the splice-to-socket flag handling (only the socket's own `O_NONBLOCK`
yields `EAGAIN`; `fs/splice.c:874-888`), the control-record send path that flushes an open record (`net/tls/tls_main.c:228-268`), the
absence of any transmit-rekey serialisation or record counting in the kernel, and `MSG_PEEK` semantics on kTLS (one record type per
call; `net/tls/tls_sw.c:1849-1862, 2089-2090, 2208-2211`). Residual empirical limits, named: the pipe invariant is validated on one
kernel build; ChaCha20, TLS 1.2, IPv6 and a real NIC were not exercised (the design excludes the first two and the slice has no IPv6).

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
  frontend's `NoBackend` and `Selected` results both permit bind (503 service
  for `NoBackend`; a `Selected` backend's identity or mTLS failure is a
  per-request 502), and the probe performs no mTLS (R-65).

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
- `GatewayOrdinaryServiceMapProgramming`: a gateway-enabled and a gateway-disabled boot program a VM
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

### Open items

None. The narrow shutdown capability (R-29), the deadline-free `disable_after_drain` and the unit `Timeout` variant (R-30) are pinned in
the sections that own them.

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
| **D-DOM-4** | [ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md): the singleton Public Route Set makes first-slice one-Route cardinality/ownership live on one aggregate key, serialized by the single owner task (sole Route writer: get, decide, put; no value compare-and-set on `IntentStore`). A different Route ID cannot claim an occupied slot; it is rejected rather than last-write-wins. |
| **D-DOM-5** | [ADR-0223](../../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md): create the distinct **Public Certified-Key Custody** context and **Public Certified Key** aggregate so Route, operator HTTPS, internal SPIFFE and allocation `IdentityMgr` remain non-owners. Protection/replacement/currentness mechanics belong to their decision-specific System/Application records and exact feature contract. |
| **D-DOM-6** | Manual operator input is the sole first-slice Certified Key producer. ACME account/order/challenge/renewal and TCP/80 are absent; GH #57 may later invoke the same custody command without changing runtime consumption. |
| **D-DOM-7** | [ADR-0225](../../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md): Workload Identity owns a dedicated single-slot **Gateway Identity Slot** and **Gateway Identity Lifecycle** for `spiffe://overdrive.local/gateway/<node-id>`. It reuses issue/audit/profile policy, renews/restarts/clears explicitly, permits only gateway-client mTLS use and never fabricates `AllocationId`. |
| **D-DOM-8** | [ADR-0222](../../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md): `GatewayApplicationOwner` atomically publishes one Route/frontend/opaque-resolver generation, retains it per connection and retires a superseded generation after its connections drain. It publishes no demand to any other owner; backend unavailability is a typed outcome at the connector ([ADR-0242](../../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)). |
| **D-DOM-9** | ServiceLifecycle remains sole Backend Eligibility publisher and existing Service Dataplane/BPF remains the backend selector. [ADR-0228](../../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md) assigns only the transient BPF Selection Receipt to Service Dataplane. The gateway never selects or reads backend rows/maps. |
| **D-DOM-10** | A Gateway Connect Intent is exactly `(Socket Cookie, ServiceKey)`. The separate registered-selection cgroup/connect4 program (R-1) chooses through existing `SERVICE_MAP` Maglev + `BACKEND_MAP`; its receipt is `Selected(BackendId)` or `NoBackend`. This host-connect path is distinct from unchanged XDP wire forwarding. |
| **D-DOM-11** | [ADR-0241](../../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md): Selected Backend Identity is the receipt's `BackendId` resolved through Service Dataplane's exact Applied association. Gateway-client rustls presents the Gateway SVID and requires exact peer SPIFFE SAN equality before body delivery. |
| **D-DOM-12** | `Route Accepted`, `References Resolved`, `Certified Key Usable`, `Gateway Identity Current`, `Gateway Applied Generation`, `BPF Hydrated`, `Public Listener Bound` and `Selected Peer Authenticated` are distinct owned promises; none is shorthand for `Ready`/`Healthy`/`Programmed`. |
| **D-DOM-13** | State-based model (the rejected Event Sourcing/CQRS alternatives are recorded in [ADR-0221](../../product/architecture/adr-0221-singleton-public-route-set-aggregate.md);): Public Route Set, Public Certified Key and Gateway Identity Slot are state-based; BackendId identity association and connect receipts are derived/process-local state. No Event Sourcing, CQRS subsystem, ACME workflow, new deployed process or userspace load balancer is introduced. |

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
| **cgroup-BPF Service Selection** | Host-originated registered socket selection/rewrite by an ancestor-attached gateway `cgroup/connect4` program (beside `cgroup_connect4_service`) through existing Maglev/BACKEND maps | XDP ingress, userspace round robin or new child cgroup/netns |
| **XDP Wire Forwarding** | Existing packet-ingress forward/reverse path on its current interfaces, unchanged by gateway selection | The gateway host-connect selection path |
| **BPF Hydrated** | ServiceMapHydrator/Dataplane applied its independent backend generation on its ordinary path | Gateway Applied Generation or proof a backend exists |
| **Selected Peer Authenticated** | Gateway-client mTLS verified exactly the receipt-selected workload SPIFFE SAN before application bytes; a per-request fact, never a bind, probe or promotion condition (R-65) | Chain-only verification, gateway preselection, persistent state or a Gate G-1 input |
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

**Invariants.** One store key holds the empty/occupied slot, and the single
owner task (the sole Route writer: get, decide, put; `IntentStore` has no value
compare-and-set) serializes claims, so competing Route IDs cannot both succeed. An occupied Route has exactly one hostname, Path Match,
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
empty/held state. Enabled+empty issues; enabled+near-expiry reissues. Disable is
a synchronous clear of the slot by the gateway shutdown caller, not a reconciler
Action (R-4). Issuance/reissuance reuses `issue_and_audit`; audit must
succeed before atomic hold/replace. Failed reissue preserves old material only
while time-usable. Restart begins empty and reissues before listener bind.
Shutdown stops admission and drains authenticated work before the slot is cleared. Only
gateway-client mTLS may consume an opaque use capability; allocation
`IdentityMgr`/`IdentityRead`, public TLS, Route, handlers and status cannot read
the material.

| Command | Delta/facts | Complement equality |
|---|---|---|
| Ensure Gateway SVID | Empty → fresh audited Held generation; GatewayIdentityHeld | Allocation held set, public key, Route and BPF |
| Reissue Gateway SVID | Held old → fresh audited Held new atomically; GatewayIdentityReplaced | Allocation held set and all non-gateway identity state |
| Clear Gateway SVID | Held → Empty by the synchronous disable, after admission/drain ordering; GatewayIdentityCleared | Allocation SVIDs/audit history, public key and BPF |
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
| Gateway identity | Ensure / Reissue / Clear; boot/clock/disable | Issued-certificate audit occurrence; GatewayIdentityHeld/Replaced/Cleared; Gateway Identity Current | Dedicated lifecycle converges one non-allocation slot; issue/audit precedes hold; only gateway-client mTLS may use material |
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
| **Gateway identity disable/expiry** | Stop new admission/upstream handshakes, drain authorized users, then clear/replace gateway SVID material | Route/Public Certified Key, Service/Backend Eligibility/BPF and allocation SVIDs |
| **No backend / unready backend** | Listener remains active when capability gates are sound; BPF receipt is NoBackend (no outer entry or no usable backend; `healthy` is not read, R-2), connect is denied and public response is 503; no wait (ADR-0242) | Route/key/identity generations and backend authority |
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
| `cgroup_connect4_service` ancestor attachment | Host connect selection | **REUSE UNCHANGED; ADD a separate gateway selection `cgroup/connect4` program (R-1)** | Both attached over `overdrive.slice` (V-GW-2 confirms `serve` is inside); unregistered sockets keep current `LOCAL_BACKEND_MAP` behavior and XDP wire path is unchanged |
| `BackendIdAllocator` | Selected backend identifier | **REUSE MONOTONIC-NONREUSE PROPERTY** | Process-lifetime non-rebinding is required for old receipts (`allocators/backend_id.rs:58-79`: the counter never recycles; `release` drops only the `(ip, port, proto)` memo). Allocation and release semantics are unchanged (R-3, P-2); one crate-private read-only accessor, `existing_id`, is contract (R-24). The exposure for a re-leased guest address is recorded in ADR-0241. |
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

System ADR-0220, 0223, 0224, 0229, 0230 and 0234 resolve `DESIGN-GAP-PIG-1`; it is not a remaining gate or
implementation permission. The complete active domain flow is:

```text
Route
  -> exact Service Frontend
  -> (independently) ServiceMapHydrator ordinary programming of the VM backend
  -> dataplane-prepared socket: SO_MARK, 127.0.0.1 bind, Gateway Connect Intent (SO_COOKIE, ServiceKey)
  -> separate cgroup/connect4 gateway selection program (R-1)
  -> SERVICE_MAP Maglev + BACKEND_MAP selection/rewrite (no usable backend -> NoBackend; `healthy` not read, R-2)
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
System ADR-0220, 0223, 0224, 0229, 0230 and 0234 resolve the system gap; Application
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
| **D-APP-11** | [ADR-0225] adds the dedicated non-allocation Gateway Identity Slot + `GatewaySvidLifecycle`, the Issue action and exact-current control; disable is a synchronous clear (R-4). The slot atomically owns a checked desired epoch so a stale Issue cannot re-hold after disable; [ADR-0234] gives gateway-client mTLS alone the private access token and exact receipt-selected peer check. No fake AllocationId. |
| **D-APP-12** | [ADR-0242] makes backend unavailability a typed outcome at the connector (`NoBackend` 503 / connect failure 502) with no wait, demand, revision, acknowledgment or wake; ServiceMapHydrator is unchanged and BPF Hydrated remains independent. The gateway shutdown stage runs before the leg-C worker stage (D5 section (c)). |
| **D-APP-13** | [ADR-0224] keeps BPF as the gateway backend selector; [ADR-0232] adds registered `(SocketCookie, ServiceKey)` intent plus `Selected(BackendId)|NoBackend` receipt and cleanup; [ADR-0243] places socket preparation (mark, loopback bind, cookie, registration) in the dataplane adapter and [ADR-0248](../../product/architecture/adr-0248-connector-reserves-its-cleanup-ledger-slot-before-preparation.md) the connector's ledger order; [ADR-0233] commit-gates immutable BackendId→applied `Backend.alloc` identity publication. |
| **D-APP-14** | [ADR-0226], [ADR-0235], [ADR-0236], [ADR-0237], [ADR-0238], [ADR-0239] and [ADR-0240] activate strict rustls TLS1.3, gateway-parsed HTTP/1.1 heads with gateway-owned framing ([ADR-0245](../../product/architecture/adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)), exact authority/raw-path routing, proxy-header policy, finite limits, kernel-side body relay with backpressure ([ADR-0239]) over kernel-TLS legs ([ADR-0244](../../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), pipe invariant [ADR-0246](../../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md)) and one upstream attempt; [ADR-0234] requires exact-peer mTLS. Existing 404 Route miss, 503 NoBackend and 502 internal/upstream mappings remain exact. |
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
| `serve`/operator composition | control-plane + CLI | **EXTEND** | **bounded-change**: mandatory Enabled/Disabled identity composition plus optional GatewayRouteCommands and GatewayStatusReads and complete GatewayHandle task/socket tree; existing operator/workload owners preserved | every-owner shutdown integration + disabled complement + built-binary AT |

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
returns `enabled=false`, `listener_bound=false` and empty optional projections (R-47: no ObservationStore read, and a stale row from an earlier enabled boot is not projected).
`ApiClient` adds `submit_route`, the production caller used by deploy, and
(R-53, P-38 adopted 2026-11-04) `withdraw_route` and `gateway_status`, which
have no production caller and exist for the operator-surface tests (S-PIG-35, 36, 42); no CLI
command or verb is introduced for them. Live OpenAPI registers
all three server paths/schemas.

Contract of the two methods (signatures in "Exact Route, deploy, configuration and owner surfaces"; the response types are the existing `overdrive_control_plane::api::{WithdrawRouteResponse, GatewayStatusResponse}`):

- `withdraw_route(&self, id: &str)` issues `DELETE v1/routes/{id}` with `id` as one percent-encoded path segment. The id is `&str`, like `stop_workload`, `restart_workload` and `describe_workload`, deliberately not `&RouteId`: the client performs no validation, so the server's disabled-gateway check, which precedes `RouteId` parsing (above), is observable through the client with an id the server would refuse to parse. Both server outcomes `Withdrawn` and `Unchanged` are 200 and decode as `WithdrawRouteResponse`.
- `gateway_status(&self)` issues `GET v1/gateway/status` and decodes `GatewayStatusResponse` on 200.
- Errors are the existing `CliError` taxonomy and no variant is added: a non-2xx is `CliError::HttpStatus { status, body: ErrorBody }` (disabled gateway: 409, `error = "gateway_disabled"`; invalid id: 400, `error = "validation"`; store or observation fault: 500), a transport failure is `CliError::Transport`, an undecodable 2xx body is `CliError::BodyDecode`, a malformed base URL is `CliError::ConfigLoad`. Neither method retries.
- Both are scaffolded like `submit_route` (a `todo!("SCAFFOLD: ...")` body with the same `#[allow(...)]` set) so the tests compile against the pinned shape; their bodies are DELIVER's, in the step that owns the Route operator surface.

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
    // R-53 (P-38 adopted, 2026-11-04). Exact signatures; siblings of `stop_workload`.
    pub async fn withdraw_route(&self, id: &str)
        -> Result<WithdrawRouteResponse, CliError>;
    pub async fn gateway_status(&self)
        -> Result<GatewayStatusResponse, CliError>;
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

**Test compositions (R-6, DG-12).** `GatewayConfig::new` takes the bind address and fixes only the port, so each gateway test
composition that reaches promotion binds its own distinct `127/8` address on port 443 (for example `127.0.0.2:443`); two
compositions then never contend for one socket. There is no listener-port seam, no Sim listener and no override field, and no
test serialises on TCP/443. Consequence for test placement: a gateway test binary joins the `host-kernel-shared` nextest group
(`.claude/rules/testing.md` § "Node-global kernel state requires a catch-all nextest group") only for node-global kernel state it
mutates (the cgroup attach, the `overdrive-mtls` nft table, bpffs pins, the `:53` bind), never because of the public bind;
compositions over `SimGatewayRelay` and the Sim dataplane that touch none of those run in parallel with distinct addresses.

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
| Gateway identity | Dedicated slot facts + desired/current hydration, `GatewaySvidLifecycle`, the Issue action and lifecycle control (disable is a synchronous clear); no allocation holder/read. |
| Gateway-client mTLS | Connected fd + exact receipt-selected SpiffeId -> authenticated, kTLS-armed `GatewayUpstream` (sealed `KtlsLeg`); Host/Sim implementations; arms through the same `GatewayRelay` backend; no InterceptedConnection and no transparent-mTLS pump. |
| `Kek` | Existing mandatory provider; `overdrive-host::ca::PublicCertifiedKeyAeadCodec` uses fixed `overdrive-public-certified-key` KEK and a distinct gateway HKDF/AAD domain over the existing private AEAD engine. |
| `ObservationStore` | Exact `public_certified_key_status_row(id)` and `gateway_application_status_row(node)` point reads plus two typed LWW writes; tables/keys/codecs are pinned in the Application persistence/status `[REF]` sections below. Issuance audit and Service hydration rows remain unchanged. |
| Kernel-TLS relay (public and upstream legs) | `GatewayRelay` ([Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)): rustls 0.23.39 unbuffered handshake and `KernelConnection`, kTLS, `splice(2)`, `httparse` heads; `HostGatewayRelay` / `SimGatewayRelay`; fixed finite GatewayLimits policy with no operator input. |

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
// `GatewayConnectDataplane` (probe, open_upstream, take_receipt, selected_backend_identity; supertrait
// `GatewayConnectCleanup`) and `GatewayConnectCleanup` (cleanup, cleanup_all_gateway_intents, live_intent_count,
// live_receipts) are pinned once, in "Gateway upstream path into leg-C" (Contract of upstream-socket preparation and
// Cleanup-only capability, ADR-0243, R-29/R-33). This section pins the value and error types they use.
pub struct GatewayConnectCleanupSweep {
    pub removed_intents: u32,
    pub removed_receipts: u32,
}
/// A socket prepared by the dataplane adapter. Not Clone.
pub struct PreparedUpstream { /* private: tokio::net::TcpSocket + intent */ }
impl PreparedUpstream {
    pub fn new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent) -> PreparedUpstream; // see "Cross-crate construction of PreparedUpstream"
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
empty selection (`BACKEND_MAP.healthy` is not read, R-2).
A missing intent passes the gateway selection program unchanged (it allows; the existing
program's LOCAL_BACKEND_MAP branch is untouched);
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
// overdrive-core::traits::dataplane (R-35): the variants of the core `DataplaneError` returned by `Dataplane::update_service`,
// and the helper types they name, live in `overdrive-core` and name only core types, so the core trait can carry them.
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
    /// R-59: the reverse-NAT key already maps to a different VIP. Both VIPs are the frontend VIP newtype
    /// `overdrive_core::id::ServiceVip`; the key is the existing core `overdrive_core::dataplane::BackendKey`
    /// (the type the `REVERSE_NAT_MAP` key is built from, converted at the adapter from `BackendKeyPod`).
    ReverseNatConflict {
        key: BackendKey,
        current_vip: ServiceVip,
        attempted_vip: ServiceVip,
    },
    ServiceApplyRollback {
        stage: ServiceApplyStage,
        rollback: Vec<ServiceApplyRollbackFailure>,
    },
}
pub enum ServiceApplyRollbackFailure {
    BackendDelete(BackendId),
    /// R-60: the in-place BACKEND_MAP restore of this id's prior value failed.
    BackendRestore(BackendId),
    /// The reverse-NAT key that could not be deleted, as the backend endpoint it keys
    /// (`overdrive_core::dataplane::Proto`); the dataplane-private `BackendKeyPod` is converted at the adapter.
    ReverseNatDelete { backend: SocketAddrV4, proto: Proto },
}
// overdrive-dataplane::allocators::backend_id (crate-private; R-24, P-24)
impl BackendIdAllocator {
    /// Read-only memo lookup: the `BackendId` already assigned to this endpoint, if any. Allocation and release
    /// semantics are unchanged (R-3).
    pub(crate) fn existing_id(
        &self,
        ip_host: u32,
        port_host: u16,
        proto: u8,
    ) -> Option<BackendId>;
}
```

**Why `existing_id` is within R-3 (R-24).** R-3 fixes allocation and release behaviour: a memo keyed by `(ip, port, proto)`, a monotonic
counter that is never recycled, release at orphan GC. `existing_id` reads the memo and mutates nothing, so it changes none of that. It
is needed for a correct projection of the commit-gated transaction: the identity-conflict rejection must precede every mutation (the
`update_rejected` step of `selection_receipt.qnt`), and without the read the only way to learn that an endpoint is memoised is
`allocate`, which mints an id for an absent endpoint and would have to be undone.

For a non-empty update, exact ordering under the write guard is:

1. Purely validate/deduplicate Backend pods and reject one endpoint carrying
   two `Backend.alloc` identities.
2. Resolve existing allocator memo hits (through `BackendIdAllocator::existing_id`, R-24) and reject any Applied/Reserved
   identity conflict (the same id for a different `Backend.alloc`, R-3) before mutation. Reject, in the same planning pass
   (`plan_service_apply`) and before any mutation, an update whose reverse-NAT key already maps to a different VIP with
   `DataplaneError::ReverseNatConflict` (R-59), so nothing is overwritten and rollback never restores another Service's
   reverse-NAT entry. A `BACKEND_MAP` entry whose
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
   at the next `update_service`, the Dataplane probe and the shutdown sweep (R-61; the probe and shutdown retries are delivered in roadmap step 05-01, which owns `EbpfDataplane::probe` and the shutdown stage); it never
   changes a committed update into a Failed hydration result.

Any error before the outer swap replays `ServiceApplyUndo` in reverse order,
deleting only keys proven absent before this turn, removes every Reserved
identity, releases fresh allocator memos without recycling their numeric IDs,
drops the private inner map and leaves the prior outer pointer. Existing equal
entries were never rewritten and need no restore; each in-place BACKEND_MAP
update is restored from the prior value saved in `ServiceApplyUndo`. Rollback
attempts every inserted key and every restore even after one failure. A failed
restore is the typed `ServiceApplyRollbackFailure::BackendRestore(BackendId)` (R-60), never only logged. A partial
rollback returns `ServiceApplyRollback`, retains the still-inserted safe-delete
keys as closed cleanup debt, and still exposes no Applied identity for the
abandoned IDs; future appearance receives fresh IDs, so abandoned reservations
cannot poison reuse.
When rollback fully succeeds, `update_service` returns the original existing
stage error; `ServiceApplyRollback` replaces it only when one or more restore
or delete operations also failed, with `stage` retaining the original failure location.
The empty-backend path takes the same write guard, commits by deleting the
outer ServiceKey first, retains historical Applied identities for old receipts,
then performs idempotent post-commit GC.

#### Test seam for the rollback restore (user-approved 2026-11-06; scenario S-PIG-132)

**Decision.** `EbpfDataplane` reaches `BACKEND_MAP` through a crate-private
abstraction instead of holding the concrete aya map. The production
implementation is the real aya map, unchanged in behaviour; a decorator that
delegates to it and can fail chosen writes exists only in a
`#[cfg(all(test, feature = "integration-tests"))]` module inside
`overdrive-dataplane/src/`. **No new public API; R-62 is not needed.** No
cross-crate type, trait, constructor, feature or re-export is added, and
`DataplaneError`, `ServiceApplyRollbackFailure` and `Dataplane` are unchanged.

**What is pinned (interface contract), and what is not.** Pinned:

1. *Scope.* Every `BACKEND_MAP` operation `EbpfDataplane` performs (the read,
   write and remove used by `update_service`, rollback, the probe and the
   inspectors) goes through the one seam, so a test-time substitution cannot be
   bypassed by a second path. `REVERSE_NAT_MAP`, `SERVICE_MAP` and the local
   maps are not behind it: the scenario needs no fault there (see S-PIG-132).
2. *Production cost.* The seam is dynamic dispatch on the `BACKEND_MAP` handle
   only. Every call it carries is a `bpf(2)` map syscall made by the
   control-plane `update_service`, the probe or an inspector; the XDP and
   `cgroup_sock_addr` programs read the kernel map directly and never pass
   through the userspace handle, so the per-packet and per-connect paths
   are unchanged. One indirect call per map syscall is below the syscall's own
   cost, and `update_service` holds the commit guard for far longer than that.
   `EbpfDataplane` stays a non-generic public type.
3. *What test support must be able to do.* Wrap the dataplane's real `BACKEND_MAP`
   handle in a decorator that delegates every operation to it, and fail a
   chosen write: "fail the Nth `insert` after arming" for any set of N, with an
   `io::Error`-shaped failure the production code already maps. Arming and
   the fail-set are test-side state; nothing in production reads them.
4. *Where it lives and how a test builds it.* The decorator and the S-PIG-132
   test live in one module in `overdrive-dataplane/src/`, declared in `lib.rs`
   beside `quint_selection_receipt` (the existing in-crate driver module,
   `testing.md` § "Placement and lane": the test needs a crate-private seam).
   The test builds the real `EbpfDataplane` through the production
   constructor `EbpfDataplane::new_with_pin_dir` (the shape
   `deregister_retry_safety.rs` uses: a veth pair, a per-test bpffs pin dir,
   cgroup attach path `/sys/fs/cgroup`), then, as a descendant module of the
   crate root that owns the struct, replaces the private `backend_map` field with
   the decorator around the handle it took out. No setter, no builder and no extra
   constructor exists for this. Gated by `integration-tests` and run on the
   native metal host (the kernel the real maps need). It runs in the
   `overdrive-dataplane` library test binary, which `.config/nextest.toml`
   already assigns wholesale to `host-kernel-shared` (`package(overdrive-dataplane)
   & kind(lib)`); no nextest change is needed.
5. *Not pinned (the crafter's call, `CLAUDE.md` § "Implement to the design").*
   The trait's name, method set and signatures, whether the field is a boxed trait
   object or another crate-private indirection, the decorator's internals, and
   the helper that builds the fault set.

**Rejected alternatives.**

- *A test in `tests/integration/` with a fault impl in test support.* An external
  test crate can name only public items, so the trait, the constructor taking
  it and the field would all have to be public: a new cross-crate API that
  production code never calls (R-62 territory), for one scenario.
- *A generic parameter on `EbpfDataplane`* (`EbpfDataplane<B = AyaBackendMap>`).
  A public type-parameter ripples into every `Arc<EbpfDataplane>` in
  `overdrive-control-plane`, the action shim and the composition root, to
  save an indirect call on a syscall path.
- *A `cfg(any(test, feature = "integration-tests"))` fault field, the shape of
  `probe_fault`/`set_probe_fault`.* It forks the body of
  `rollback_service_apply` between the tested build and the shipped build (the
  code under test is not the code that ships), needs a public setter for a
  cross-crate test, and adds a test-only branch to the function being
  proven. `probe_fault` is not extended and not removed by this ruling.
- *A real-kernel fault with no seam* (filling or freezing the map). A map-level
  fault applies to the forward in-place write and to its restore alike (the same
  operation on the same key), so it cannot fail only the restore; and the
  test holds no handle to the map file descriptor. Only a call-ordinal
  stimulus separates the two writes.
- *`SimDataplane` modelling rollback.* The Sim adapter has no map layer;
  teaching it rollback would add production-visible Sim behaviour to test an
  aya-map write, and would test the model, not `rollback_service_apply`.

**Lifecycle Gate Ownership.** Not applicable: the seam adds, removes and moves
no gate and changes no state meaning (it substitutes a map handle; the commit
guard, `Reserved`/`Applied` publication and the outer swap are unchanged).
**Formal protocol model.** Not applicable: no protocol or ordering is added or
changed; rollback order and the R-60 failure are the ruled behaviour of the
existing `selection_receipt` model, and a test-only decorator is not a protocol.

`Dataplane::update_service` returns `Ok` only after outer commit plus Applied
publication. The action shim writes the Completed hydration row only after
that point; a pre-commit or rollback error writes the existing Failed
hydration row. This is userspace transactional publication around existing
kernel primitives, not a claim that four independent BPF maps update
atomically (assumption A-K1).

**Sim equivalence pin (S-PIG-67).** `SimDataplane::update_service` returns the same `DataplaneError::BackendIdentityConflict`
as the host adapter, before any mutation, when a backend endpoint that still holds a live allocator association (the memo)
is re-leased to a different `Backend.alloc` (a re-leased address); it leaves the prior generation, the memo and every
`BackendId` identity unchanged. An endpoint whose memo entry was released by orphan GC may be bound to another allocation by
a later update (R-38), in both adapters.

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
    pub const fn get(self) -> NonZeroU64;
}
pub enum GatewayIdentityEpochError { Empty, InvalidDecimal, Zero }
// FromStr/TryFrom parse canonical decimal through new; Display and matching
// string Serialize/Deserialize are canonical decimal.

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
Actions, reconciliation and slot hold/drop carry the typed value. `begin_disable` is infallible (R-30): it performs the `NonZeroU64` checked add privately and treats an overflow as
`unreachable!()`, because a u64 epoch bumped once per disable cannot be exhausted. The type pins no increment operation and no exhaustion error.

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
| `PublicCertifiedKeyStatusRowEnvelope::V1` | redb table `observation_public_certified_key_status`, key the UTF-8 bytes of the canonical certified-key ID string | Active usable metadata or retained-generation unusable cause, plus optional latest install failure | Paths, PEM/key bytes, raw parser/crypto error string |
| `GatewayApplicationStatusRowEnvelope::V1` | redb table `observation_gateway_application_status`, key the UTF-8 bytes of the `NodeId` string (`NodeId::as_str().as_bytes()`) | listener fact; staged/current/draining application generations; unavailable cause; non-secret gateway identity/connect-probe facts; the issue-failure occurrence record (`issue_failures`, `last_issue_failure`: cause class and instant only, R-45) | Backend candidates, BackendId/receipt/peer, ServiceBackendRow/BPF generation, credential bytes, CA or audit error text |
| `WorkloadSvidIssueStatusRowEnvelope::V1` | redb table `observation_workload_svid_issue_status`, key the UTF-8 bytes of the `AllocationId` string (`AllocationId::as_str().as_bytes()`) (defined in "SVID issuance failure occurrence", R-46; listed for completeness, not owned by the gateway) | `issue_failures`, `last_issue_failure` (cause class and instant), `node_id`, LWW stamp | Certificate or key bytes, CA or audit error text |

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

Intent decode uses ADR-0048 fail-fast `health.startup.refused`; an observation
codec returns the decode error without logging, and the observation store emits one
`observation.row.skipped` warning (separate `table` and `key` fields, R-56) and skips only that row.
Writers construct envelopes only through `VersionedEnvelope::latest`; no raw
rkyv invocation exists outside these payload methods. Golden bytes and
round-trip/schema-evolution properties pin all five V1 formats (the fifth is the workload SVID issue-status row of R-46).

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
   generation and cleared `last_install_failure` (replaying the identical file of the current key clears a stale `last_install_failure`
   without changing the stored record). Status failure is retained
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
`GatewayAdapters::host(kek, clock)` constructs one codec over the boot-probed
`Kek`; `boot_server` G2 passes it to `UnboundGatewayBuilder::new` (with the node it already holds, R-57). No adapter receives a generic
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
    routes: Option<GatewayRouteCommands>,
    status: Option<GatewayStatusReads>,
}
impl GatewayAppStateComposition {
    pub fn disabled() -> Self;
    pub(crate) fn enabled(
        identity_view: Arc<dyn GatewayIdentityView>,
        identity_actions: GatewayIdentityActionComposition,
        routes: GatewayRouteCommands,
        status: GatewayStatusReads,
    ) -> Self;
    pub fn identity_view(&self) -> &dyn GatewayIdentityView;
    pub fn identity_actions(&self) -> &GatewayIdentityActionComposition;
    /// Both are `None` exactly when the composition is disabled.
    pub fn routes(&self) -> Option<&GatewayRouteCommands>;
    pub fn status(&self) -> Option<&GatewayStatusReads>;
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
        node_id: NodeId,
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
        relay: Arc<dyn GatewayRelay>,
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
    pub fn into_parts(self) -> (GatewayHandle, GatewayRouteCommands, GatewayStatusReads);
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

/// Read-only status port (R-25, P-25): redacted ObservationStore point reads; no Route write capability is reachable from it.
#[derive(Clone)]
pub struct GatewayStatusReads { /* redacted reads only */ }
impl GatewayStatusReads {
    pub async fn application_status(
        &self,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError>;
    pub async fn certified_key_status(
        &self,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, GatewayStatusReadError>;
}
/// Route command port (R-25, P-25): the write handle of the Public Route Set; it exposes no status read.
#[derive(Clone)]
pub struct GatewayRouteCommands { /* Route command handle only */ }
impl GatewayRouteCommands {
    pub fn routes(&self) -> &PublicRouteSetHandle;
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
    /// Cleanup sweep and live counts only (R-29): shutdown holds no handle that can prepare an upstream socket.
    cleanup: Arc<dyn GatewayConnectCleanup>,
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
    /// Time the stage consumed on the injected clock, at most `grace` (R-64: drain
    /// ends at `grace - GATEWAY_SHUTDOWN_JOIN_RESERVE`, the final owner joins end
    /// at `grace`); `ServerHandle` subtracts it from the shared drain deadline
    /// before axum's graceful shutdown (D5-c).
    pub elapsed: Duration,
    pub forced_connections: u32,
    pub task_failures: Vec<GatewayTaskFailure>,
    pub cleanup_failures: Vec<GatewayCleanupFailure>,
    pub application_errors: Vec<GatewayApplicationCommandError>,
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
    // no receipt-count variant: the residual receipt count is `live_receipts()?.len()` (R-33); a failed
    // `live_receipts()` read leaves `GatewayResidualConnectState.live_receipts` as `None`, so the report is not clean
}
pub(crate) struct GatewayConnectionOwnerHandle { /* close/drain/join commands */ }
pub(crate) struct GatewayCleanupLedger {
    /* private: capacity, the slot table (a reserved-unbound slot becomes a
       record keyed by the intent's cookie once bound), saturation_count */
}
pub(crate) struct GatewayCleanupRecord { /* private: intent, connect_intent_failed, receipt_failed */ }
pub(crate) struct GatewayCleanupReservation { /* private: ledger handle and its slot */ }
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
    /// R-5: takes one slot before `open_upstream`; the slot is bound to the intent afterwards.
    /// Crate-private pinned shape (it has production callers in the connector). Dropping a
    /// reservation that was never bound releases its slot.
    pub(crate) fn reserve(
        self: &Arc<Self>,
    ) -> Result<GatewayCleanupReservation, GatewayCleanupLedgerFull>;
    pub(crate) fn retry_all(
        &self,
        dataplane: &dyn GatewayConnectCleanup,
    ) -> Vec<GatewayCleanupFailure>;
    pub(crate) fn len(&self) -> u32;
}
impl GatewayCleanupReservation {
    /// R-5: binds the reserved slot to the intent `open_upstream` registered. Crate-private
    /// pinned shape.
    pub(crate) fn bind(&mut self, intent: GatewayConnectIntent);
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
    pub(crate) fn new(budget: Arc<GatewayAdmissionBudget>) -> Arc<Self>; // DG-10, Application conformance bullet
    pub(crate) fn budget(&self) -> &Arc<GatewayAdmissionBudget>;
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
identity_empty && residual_connect.is_none()`. The report has no identity-error field: shutdown never calls `ensure_current` (R-30), and `disable_after_drain` is infallible, so no shutdown path produces an identity lifecycle error.

`boot_server` (`M:crates/overdrive-control-plane/src/lib.rs:3625`; `run_server`
at `:3116-3120` delegates to it) is the sole caller of
`UnboundGatewayBuilder::{new,into_builder}`, constructs one
`GatewayLimits::first_slice` value, uses it for the `GatewayClientMtlsFactory`
product and one `GatewayRuntimeDependencies`, calls `GatewayBuilder::start`
(steps G2-G4 of the D5 boot table), puts
`GatewayRouteCommands` and `GatewayStatusReads` in `AppState`, and transfers `GatewayHandle` into
`ServerHandle`. `ServerHandle::shutdown` is the sole caller of
`GatewayHandle::shutdown`. Route POST/DELETE are the sole callers of
`GatewayRouteCommands::routes`; `GET /v1/gateway/status` is the sole caller of its two
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
`limits.max_cleanup_ledger_entries() == 128`. Before `open_upstream` (R-5, DG-11),
connector `reserve()` takes one slot; after `open_upstream` returns, the connector
binds the registered intent to that slot. A full ledger increments the saturating
`saturation_count`, returns `GatewayCleanupLedgerFull`, and the connector refuses
before any socket exists or any registration happens (the dataplane adapter, not the
connector, makes registration, so the slot must exist first); no cleanup intent is lost,
and a slot is never needed after registration. If `open_upstream` fails, or the
reservation is dropped before `bind`, the slot is released. The reservation marks exact intent/receipt
failure bits, and `complete` removes the record only after both keys are known
clean. `retry_all` snapshots records under its mutex, drops the guard, calls
Dataplane cleanup for each exact intent, then reacquires the guard to remove
only successful records; a failed record remains for the next sweep. Its
return groups retained failure bits into ConnectIntent/Receipt counts and emits
`LedgerSaturated` whenever the historical saturation counter is nonzero, so
saturation is typed/non-clean even if later cleanup succeeds. Application
close/retire command failure goes to `application_errors`; it is not a Dataplane-ledger entry. The report carries no identity-lifecycle error (shutdown never calls `ensure_current`).
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
or `Selected` is bind-valid and the probe performs no mTLS, ADR-0242, R-65), rechecks identity remains
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
    /// Occurrence record (R-45): monotone count of failed gateway SVID issuances; never cleared by a success.
    /// `issue_failures == 0` exactly when `last_issue_failure` is `None`.
    pub issue_failures: u64,
    /// Depth-1 snapshot of the latest failed issuance (`SvidIssueFailure`, `overdrive-core::ca::issue_failure`, defined once in
    /// "SVID issuance failure occurrence"). No source text.
    pub last_issue_failure: Option<SvidIssueFailure>,
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
    /// R-45; `0` and `None` on a node that never failed an issuance.
    pub issue_failures: u64,
    pub last_issue_failure: Option<SvidIssueFailureBody>,
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
direct point indexes at redb table `observation_public_certified_key_status`
keyed by the canonical certified-key ID string bytes and redb table
`observation_gateway_application_status` keyed by the `NodeId` string bytes
(names and encodings pinned in "SVID issuance failure occurrence" § Persisted
format). `PublicCertifiedKeyCustody` is the only writer/caller
of the former variant; `GatewayApplicationOwner` is the only writer/caller of
the latter. `GatewayStatusReads` and `GET /v1/gateway/status` are the only callers
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
`GatewayStatusReads::application_status` and
`GatewayStatusReads::certified_key_status` are the real redacted, ID-scoped
ObservationStore point reads used by this handler; read failure is 500 and no
status DTO can reach an opaque key/identity-use snapshot.
Dedicated `*Body` enums project every application/connect/custody failure and
provenance variant; domain enums are not serialized directly. All time fields
remain typed `UnixInstant` using its existing quoted seconds+nanos JSON form.
`configured_address` retains `SocketAddrV4`'s canonical `a.b.c.d:443` string
serde form and declares `#[schema(value_type = String)]`; every DTO
`UnixInstant` field carries the same String schema override.

Public TLS is TLS1.3/exact SNI/ALPN `http/1.1`, the spiked cipher-suite set, AES-128-GCM and AES-256-GCM only (Contract of the legs; R-8), no default cert/0-RTT/tickets
(handshake in userspace, record layer in the kernel: [Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)).
The gateway's own head parser alone derives framing (ADR-0245). Only origin-form request targets are accepted;
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
the gateway regenerates legal framing. Trailers are consumed and not forwarded. The
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
`http::uri::Authority`, rejects userinfo (`@`), permits no port or
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
The gateway alone regenerates framing after these functions return (`HeaderMap` is `http::HeaderMap`).

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
| Public connections | 128 | Reuses the accepted pre-authentication concurrency class. An active exchange holds 2 sockets plus up to 4 pipe descriptors (ADR-0246), so the gateway tree reaches 768 descriptors at 128 exchanges, above the existing 384-fd envelope for 128 three-leg mTLS intercepts; the Tier-3 descriptor check at 128 concurrent exchanges validates the process limit. Lower rejects ordinary bursts sooner; higher increases unauthenticated TLS CPU/fds. |
| In-flight connects / requests | 128 / 128 | Connect capacity equals both BPF transient-map capacities; HTTP/1.1 has at most one active request per connection. Lower wastes admitted sockets; higher cannot be represented by the maps. |
| Cleanup-ledger entries | 128 | One slot is reserved per admitted upstream connect before BPF registration, matching connect/map capacity. Lower can refuse a connect while map capacity remains; higher cannot represent additional simultaneously dirty cookies and retains more failed cleanup state. |
| Request/response head bytes | 32 KiB / 32 KiB | Sized for large cookies and metadata at 32 KiB, enforced by the gateway's own head reader. Across 128 active connections, the conservative two-head upper envelope is 8 MiB. Lower breaks moderately large cookies/metadata; higher expands slowloris/parser memory. |
| Request/response header count | 100 / 100 | 100 fields sizes the parser's header array. Lower rejects interoperable clients; higher increases per-field allocation/work. |
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

The head reader receives `max_request_head_bytes`/`max_response_head_bytes` as the `read_through` limit and
`httparse` receives a header array of `max_*_headers` entries (the framing contract in the Kernel relay section).
Excess public connection closes before TLS;
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
// There is no DropGatewaySvid action (R-4): disable is the synchronous clear below.

// overdrive-gateway::ports; implemented by the control-plane composition.
#[async_trait::async_trait]
pub trait GatewayIdentityLifecycleControl: Send + Sync {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError>;
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    /// R-45: the process-local issue-failure record (`None` = no failure this process). Read beside `current()`; the
    /// `subscribe` item type and `current()` are unchanged. A recorded failure wakes `subscribe` receivers (see "Reading and wake").
    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>;
    /// Synchronous clear (R-4, R-30): bumps the epoch and empties the slot in the same call; it waits for nothing, so it
    /// takes no deadline, is not async and is infallible: the u64 epoch cannot be exhausted (the slot's private checked add
    /// overflowing is `unreachable!()`), so there is no failure to return.
    fn disable_after_drain(&self);
}
pub enum GatewayIdentityLifecycleError {
    ReconcilerUnavailable,
    /// `ensure_current` did not reach Current by its deadline (the only operation that waits).
    Timeout,
    Closed,
}
pub trait GatewayIdentityLifecycleTarget: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>; // R-45, as on the Control trait
    /// Infallible (R-30): the epoch is a u64; the slot's private checked add overflowing is `unreachable!()`.
    fn begin_disable(&self) -> GatewayIdentityEpoch;
}

// GatewayUpstreamSeal, GatewayUpstream and KtlsLeg are pinned in
// [Kernel relay data path](#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1):
// `seal_authenticated(&self, leg: KtlsLeg) -> GatewayUpstream`; `GatewayUpstream` has no public
// constructor, no AsyncRead/AsyncWrite, no Clone and no conversion; its one consumer is
// `GatewayUpstream::into_halves`, called by the exchange driver.

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
    /// Raised after the peer equality held: the kernel refused to arm TLS on the authenticated socket.
    Arm(KtlsArmError),
}

// overdrive-sim
pub struct SimGatewayClientMtls { /* scripted peer + one seal + the Sim relay backend */ }
impl SimGatewayClientMtls {
    pub fn new(seal: GatewayUpstreamSeal, relay: Arc<SimGatewayRelay>, clock: Arc<dyn Clock>) -> Self;
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
    ReadySelected,
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

`GatewayUpstreamConnector::connect` first reserves one cleanup-ledger slot (a full ledger refuses here, before any
socket exists; R-5), then calls `open_upstream(ServiceKey::from_frontend(frontend))`
(one marked, loopback-bound IPv4 socket with its nonzero cookie registered by
the dataplane adapter), binds the returned intent to the reserved slot, takes the socket out with `into_parts`, and
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
error. A validated `Selected` receipt returns `ReadySelected` and closes the
dialed socket: the probe performs no `selected_backend_identity` lookup and no
`GatewayClientMtls::authenticate`, so the selected backend's identity and
authentication never gate bind (R-65). For the probe operation the
`Selected` occurrence is emitted immediately after receipt validation, like
`NoBackend`, with `receipt_outcome = "selected"` and `backend_id` but no
`expected_peer_spiffe_id`; the request operation's `Selected` occurrence is
unchanged. Request-time `connect` instead maps canonical
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
    pub fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>; // R-45
    pub fn client_access(
        self: &Arc<Self>,
        clock: Arc<dyn Clock>,
    ) -> GatewayClientIdentityAccess;
    pub fn action_target(self: &Arc<Self>) -> GatewayIdentityActionHandle;
    pub fn lifecycle_target(self: &Arc<Self>) -> GatewayIdentityLifecycleHandle;
    pub(crate) fn begin_disable(&self) -> GatewayIdentityEpoch;
    pub(crate) fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    pub(crate) fn record_issue_failure(&self, failure: SvidIssueFailure);
}

pub struct GatewayIdentityActionHandle { slot: Arc<GatewayIdentitySlot> }
pub struct GatewayIdentityLifecycleHandle { slot: Arc<GatewayIdentitySlot> }
impl GatewayIdentityActionTarget for GatewayIdentityActionHandle {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn record_issue_failure(&self, failure: SvidIssueFailure);
}
impl GatewayIdentityLifecycleTarget for GatewayIdentityLifecycleHandle {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>;
    fn begin_disable(&self) -> GatewayIdentityEpoch;
}

// `client_access` builds a `GatewayClientIdentityAccess` (defined in `overdrive-gateway::ports`, R-20, below) over a
// dataplane-private `GatewayClientIdentitySource` implementation that holds the slot and the injected clock.

// overdrive-dataplane::mtls::gateway_client  (file: crates/overdrive-dataplane/src/mtls/gateway_client.rs —
// the exact-peer client; the registry artifact of OUT-PIG-GATEWAY-EXACT-PEER)
pub struct HostGatewayClientMtls { /* slot, Clock, limits; private rustls state */ }
impl HostGatewayClientMtls {
    pub fn new(
        identity: GatewayClientIdentityAccess,
        seal: GatewayUpstreamSeal,
        limits: GatewayLimits,
        relay: Arc<HostGatewayRelay>,
    ) -> Self;
}
pub struct HostGatewayClientMtlsFactory { /* Arc<HostGatewayRelay> */ } // implements GatewayClientMtlsFactory by calling `new`
impl HostGatewayClientMtlsFactory { pub fn new(relay: Arc<HostGatewayRelay>) -> Self; }
```

**Placement of `GatewayClientIdentityAccess` (R-20, P-20).** The type and its source trait are defined in
`overdrive-gateway::ports`, because `GatewayClientMtlsDeps` (also `overdrive-gateway::ports`) names it and
`overdrive-dataplane` depends on `overdrive-gateway`, never the reverse. The dependency direction is
`overdrive-dataplane -> overdrive-gateway -> {overdrive-core, overdrive-host}`; `overdrive-gateway` names no dataplane type.

```rust
// overdrive-gateway::ports (SvidMaterial, TrustBundle: overdrive-core::traits::ca; GatewayClientMtlsError: this module)
pub trait GatewayClientIdentitySource: Send + Sync {
    /// Runs `use_material` while the source keeps the held material borrowed. Absent material is
    /// `Err(GatewayClientMtlsError::IdentityAbsent)` and expired material `Err(GatewayClientMtlsError::IdentityExpired)`;
    /// the closure is not async and the material is never returned or cloned. The clock read lives in the implementation.
    fn with_current(&self, use_material: &mut dyn FnMut(&SvidMaterial, &TrustBundle)
        -> Result<(), GatewayClientMtlsError>) -> Result<(), GatewayClientMtlsError>;
}
pub struct GatewayClientIdentityAccess { /* Arc<dyn GatewayClientIdentitySource> */ }
impl GatewayClientIdentityAccess {
    pub fn new(source: Arc<dyn GatewayClientIdentitySource>) -> Self;
    pub fn with_current<R>(&self, use_material: impl FnOnce(&SvidMaterial, &TrustBundle)
        -> Result<R, GatewayClientMtlsError>) -> Result<R, GatewayClientMtlsError>;
}
```

`overdrive-dataplane` keeps `GatewayIdentitySlot::client_access(self: &Arc<Self>, clock: Arc<dyn Clock>) -> GatewayClientIdentityAccess`
(signature unchanged); the slot-and-clock source it wraps is dataplane-private. `GatewayClientMtlsDeps`,
`GatewayClientMtlsFactory`, `HostGatewayClientMtls::new`, `HostGatewayClientMtlsFactory::new`, boot G2 and the Sim factory
keep the shapes pinned in this document. `with_current` is `pub` because its only production caller,
`overdrive-dataplane::mtls::gateway_client`, is in another crate; the closure-only shape still prevents the material from leaving
the call, and "only `mtls/gateway_client.rs` calls it" is a review and grep rule (no new tooling). The Sim factory ignores
`Deps.identity`. Moving the slot into `overdrive-core` is not available (the slot uses `tokio::sync::watch`, which
`overdrive-core` forbids); rejected alternatives: a two-phase factory binding (a factory that exists before it can authenticate), a factory
trait owned by `overdrive-dataplane` (an `overdrive-sim -> overdrive-dataplane` edge that `development.md` § "xtask" records as removed on
purpose), and the identity as `Box<dyn Any + Send>` (a downcast with no type check).

File ownership (reconciles the two earlier names): `mtls/gateway_identity.rs`
owns the slot, handles and `client_access` (and the private source it wraps); `mtls/gateway_client.rs` owns
`HostGatewayClientMtls` and `HostGatewayClientMtlsFactory` and is the only
caller of `with_current`. The exact-peer outcome artifact is therefore
`mtls/gateway_client.rs`.

`GatewayIdentitySlot` implements the `GatewayIdentityView` read trait above; its `subscribe` is not
exposed through `GatewayIdentityView`. Serve composition obtains one
ActionHandle and one LifecycleHandle; direct slot mutators are crate-private.
Only the ActionHandle implements `GatewayIdentityActionTarget`, delegates to
the audited hold, and is installed in AppState; the executor passes the exact action
epoch, calls existing `issue_and_audit` before hold, and treats `StaleEpoch` as
a successful stale no-op. Only the LifecycleHandle implements
`GatewayIdentityLifecycleTarget`; the control-plane lifecycle adapter calls
begin-disable through it. `GatewayClientIdentityAccess::with_current` (through the dataplane's source) reads the
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
or `Timeout` wins. `disable_after_drain()` calls
`begin_disable`, which synchronously empties the slot; no evaluation is submitted
and there is no race or wait, so the method takes no deadline, is not async and
has no disable-timeout outcome and no failure (R-30: the u64 epoch cannot be exhausted, so the slot's private checked add overflowing is `unreachable!()`). `current`/`subscribe`/`issue_record`
delegate only non-secret facts. `ensure_current` loops: a `subscribe` wake with an unchanged item (a recorded failure) re-checks Current and keeps waiting. No detached task or second reconcile call path
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
    /// R-45: record one confirmed failed issuance. Epoch-independent (a failure that completes after a disable is still an
    /// occurrence), infallible and in-memory; it never touches the held identity.
    fn record_issue_failure(&self, failure: SvidIssueFailure);
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
    pub fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>;
    pub fn begin_disable(&self) -> GatewayIdentityEpoch;
}
impl GatewayIdentityActionTarget for SimGatewayIdentityActionTarget {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome;
    fn record_issue_failure(&self, failure: SvidIssueFailure);
}
impl GatewayIdentityLifecycleTarget for SimGatewayIdentityActionTarget {
    fn current(&self) -> Option<GatewayIdentityFacts>;
    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>>;
    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>;
    fn begin_disable(&self) -> GatewayIdentityEpoch;
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
    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord>;
    fn disable_after_drain(&self);
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
```

`AppState` carries one mandatory `GatewayIdentityActionComposition`.
The existing `dispatch`, `dispatch_with_network_provisioner`,
`dispatch_with_workflow_intent`, the integration-only network-provisioner form,
and their shared `dispatch_single` each gain `&GatewayIdentityActionComposition`
in the same position and every production wrapper passes `AppState`'s value.
`IssueGatewaySvid` invokes `issue_and_audit` and then `hold_audited`; audit/
issuance failure first calls `record_issue_failure` (R-45, R-50; see "SVID issuance failure occurrence")
and then returns `ShimError::GatewaySvidIssue(source)`, while
`StaleEpoch` is a successful stale no-op. A disabled composition returns
`ShimError::GatewayIdentityDisabled(source)`. No executor obtains
private material from the slot and no dispatcher overload omits this mandatory
composition.

Both ShimError variants have fixed outer Display text—`"gateway SVID issuance
or audit failed"` and the `GatewayIdentityDisabled` text above—while retaining
their typed source for matching; no SVID/key bytes render. Existing
first-error/per-action-isolation semantics remain unchanged.

`dispatch_issue` accepts only `Action::IssueGatewaySvid`, passes its exact
node/SPIFFE identity to `issue_and_audit`, and passes the returned material plus
action epoch to the target. The shared `dispatch_single`
resolves `gateway_identity.target()?` in the `IssueGatewaySvid` arm, calls the one
function, and maps both `Applied` and `StaleEpoch` to `Ok(())`.
`GatewayIdentitySlot` implements `GatewayIdentityActionTarget` by delegating to
its identically named inherent operations. The Sim type implements that port
plus `GatewayIdentityView` over one
mutex-held deterministic state; its current projection never contains key
material. `SimGatewayIdentityLifecycleControl` submits its retained Evaluation
to the same real `EvaluationBroker` and uses the injected Sim Clock plus target
watch for the same `ensure_current` absolute-deadline races as production (`disable_after_drain` takes no deadline, R-30); it
never sets Current directly. The Sim reconciler/action path must produce
`hold_audited` and the synchronous `begin_disable` clear, preserving adapter equivalence.

`SpiffeId::for_gateway(node)` yields exactly
`spiffe://overdrive.local/gateway/<node-id>`. A dedicated
`GatewayIdentitySlot` under the existing dataplane mTLS owner holds exactly one
non-allocation SVID generation; it never enters `IdentityMgr` or generic
`IdentityRead`. The slot's same lock owns a checked process-local desired epoch:
constructor epoch one means enabled; `begin_disable` increments and sets None;
the Issue Action carries the epoch; `hold_audited` rejects a
stale action, so an in-flight Issue cannot re-hold after disable (R-4: disable
clears the volatile slot synchronously, so the design has no drop Action).
`GatewaySvidLifecycle` reuses workload SVID TTL/half-life/retry/restart policy
with the epoch-bearing `IssueGatewaySvid` Action. Executors reuse
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
Only after that equality succeeds does Host arm kernel TLS on the socket through
`HostGatewayRelay` (ADR-0244; a failed arm is `GatewayClientMtlsError::Arm` and no octet was
written) and call `GatewayUpstreamSeal::seal_authenticated(KtlsLeg)`. The Sim adapter calls it only after
its scripted peer equality succeeds. Those are the only two production/test
call sites; arbitrary sockets or legs cannot construct, convert into, clone or
unwrap `GatewayUpstream`.
It never fabricates AllocationId or uses transparent `InterceptedConnection`/
kTLS pump. Expiry/loss stops new admission/upstreams; reissue preserves old
only while usable. Shutdown drains authenticated work before the slot is cleared.

Composition (`boot_server`, D5 steps G1-G4) calls `UnboundGatewayBuilder::into_builder` to
obtain the builder and the one non-Clone `GatewayUpstreamSeal`, builds the
client through `GatewayClientMtlsFactory`, then calls
`GatewayBuilder::start` with `ComposedDataplane.gateway_connect`,
gateway-client mTLS and identity lifecycle control. `AppState` always holds the
`GatewayAppStateComposition` (identity reads into HydrationContext, the identity
action composition through every action/workflow-emission dispatch wrapper);
disabled uses the canonical disabled identity read plus no control handle.
`StartedGateway::into_parts` yields `GatewayHandle` + `GatewayRouteCommands` + `GatewayStatusReads` for
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
absolute `shutdown_deadline = clock.now() + grace` (the one operator-visible
grace budget) and derives `drain_deadline = shutdown_deadline -
GATEWAY_SHUTDOWN_JOIN_RESERVE` (R-64; the reserve is a crate-private 500 ms
constant; when `grace` is no larger than the reserve the drain window is empty,
`drain_deadline = start`). Drain, which covers in-flight exchanges and
lingering discards, ends at `drain_deadline`; the exchange driver's drain bound
(the lingering-close bound included) is `drain_deadline`. Everything after
drain, including the final owner joins, runs until `shutdown_deadline`, so
the whole stage completes within `grace`. The handle always executes this
sequence:

1. Call `connections.stop_accepting()`, await the listener task, and prove no
   further accept can acquire a connection lease through
   `tasks.join_listener(shutdown_deadline, clock)`. Then call
   `application.close_for_shutdown(shutdown_deadline)`, which closes every Current/
   Draining retention, atomically clears admission, and discards any staged
   candidate. Append command unavailable/timeout failure
   and continue if it cannot complete.
2. Await each retention `wait_zero` and the connection owner until
   `drain_deadline`; at `drain_deadline` abort every remaining connection
   task, then await every aborted JoinHandle so Drop accounting and cleanup
   guards have run, and take the connections owner JoinHandle through
   `tasks.join_connections(shutdown_deadline, clock)`.
3. Retry the bounded cleanup ledger, call
   `cleanup_all_gateway_intents`, and independently read
   `live_intent_count` plus `live_receipts()` (the residual receipt count is its length). The cleanup method removes and
   reports both map families. Only successful sweep + `Some(0)` for both counts
   + empty ledger yields `residual_connect=None`. Sweep/count error, unknown
   count, remaining intent, remaining receipt or ledger entry produces
   `GatewayResidualConnectState` with the exact typed failure/count/category.
   Never discard it.
4. Call `application.retire_closed(shutdown_deadline)` only after the joined lease
   counts are zero. Then
   cancel Application, Manual Source, Custody and
   Route Set in that order. `join_listener` ran in step 1,
   `join_connections` runs after the force-join in step 2, and
   `join_remaining(shutdown_deadline, clock)` joins every owner task or aborts+awaits it
   at `shutdown_deadline`. The owners exit cooperatively on cancellation and
   do no I/O, so the reserve is spent only here and on the post-drain steps
   (the force-join, the sweep, `retire_closed`); a drain that ran to
   `drain_deadline` therefore leaves every owner task joined, not reported as
   `Timeout`. Command, inner-task, channel, timeout,
   cancellation and panic failures are appended and never skip a later task.
5. Call `identity.disable_after_drain()`. `begin_disable` clears the
   volatile `GatewayIdentitySlot` and bumps its epoch synchronously, so Current
   is Empty when the call observes it and no reconciler round-trip is needed at
   shutdown (the issuance audit row is already durable; a stale-epoch Issue
   completion is a no-op; R-4). The call therefore does not depend on the
   convergence loop being live, and it is infallible (R-30): it sets `identity_empty=true`. The
   shutdown report has no identity-error field (no shutdown path calls `ensure_current`). Close/retire command failures are accumulated in
   `application_errors`.

Only after that report returns does ServerHandle cancel/join convergence, then
resume the existing emit-drain, interest-router, axum-graceful, server-task,
exit-observer, shared-network-supervisor, mtls-worker, guest-flow and
mtls-resolve order (`M:crates/overdrive-control-plane/src/lib.rs:2890-2993`).
It accumulates the existing
mTLS failure plus any non-clean gateway report into `ServerShutdownError` after
all owners have been attempted. Deadline, cancellation, keep-alive and task-
panic paths therefore join users before application retirement and the slot clear, and
expose typed residue instead of logging it away. Upstream connections end at
stage 0, before the worker that terminates leg-C (stage 6).

### Lifecycle Gate Ownership — application completion

| Gate | Owner / affected promise | Failure projection | Explicitly unaffected |
|---|---|---|---|
| Public Listener Bound | GatewayHandle/serve; dynamic TCP/443 after gates | no degraded/partial listener | Allocation Running, Service Stable, Backend Eligibility, operator HTTPS |
| Certified Key Usable | Custody current public key | expiry/loss stops accepts first | Route Accepted, Service/BPF, Gateway SVID |
| Gateway Identity Current | dedicated slot/lifecycle | reissue; no new admission/upstream when unusable | allocation SVIDs/public key/BPF |
| Connect Path Ready | `GatewayUpstreamConnector::probe` over the dataplane adapter's sentinel + actual-frontend probe (ADR-0230, ADR-0242) | staged cannot become Current; actual-frontend `NoBackend` and `Selected` are bind-valid (R-65: backend health and backend authentication do not gate it) | Route truth, Service Stable, Backend Eligibility, ServiceBackendRow |
| Gateway Applied Generation | GatewayApplicationOwner Current Arc | atomic current or no admission | BPF Hydrated/Backend Eligibility |
| Selected Peer Authenticated | gateway-client mTLS per request | 502/no fallback or retry | future requests/lifecycle state |

Seeded invariants cover staged-before-swap, current-to-draining/no-early-retire,
stale gate-result rejection, key/identity loss before unbind, typed
NoBackend/unavailable outcome without wait (ADR-0242), receipt cleanup/isolation, gateway SVID issue/reissue/clear and
unchanged allocation/Service/BPF meanings.

## Wave: DESIGN / [REF] SVID issuance failure occurrence

Rulings R-45..R-52 (2026-11-04; provenance per row in the rulings table of the reconstruction section); decision records
[ADR-0274](../../product/architecture/adr-0274-gateway-svid-issuance-failure-is-a-recorded-occurrence-on-the-application-status-row.md)
(gateway) and [ADR-0275](../../product/architecture/adr-0275-workload-svid-issuance-failure-is-a-recorded-occurrence-on-its-own-row.md) (workload),
both Proposed pending independent review. Two ADRs because the owners, rows and reversibility differ; the shared value type below is the only
common contract. R-47 (disabled status), R-48 (`Absent`/`Unusable`) and R-49 (`IssueFailed` deleted) are rulings of this delta only; they are
not part of either ADR.
Cost budget: cold path only. One extra row write per failed issuance attempt (gateway: one owner projection; workload: one atomic store
operation). Reads: the gateway status request adds none (the `application_status` read already exists); the workload `GET /v1/allocs` adds one
point read per allocation row. No hot-path cost.

**Grounded premise (`development.md` § "Ground the premise").** Both failures are reachable through `overdrive serve` with the real
composition and no `*_override`. Gateway: `serve` with gateway config registers `GatewaySvidLifecycle` on `node/<id>`
(`gateway_identity_lifecycle::gateway_svid_lifecycle_registration`, called from `run_server`), which emits `IssueGatewaySvid`; the
executor calls `ca_issuance::issue_and_audit`. Workload: `run_server` registers `svid_lifecycle()`, which emits `IssueSvid` for every
Running allocation, including every `rotate-svid` near-expiry reissue; `action_shim::issue_svid::dispatch_issue` calls the same
function, from two callers (the convergence arm and `ensure_intercept_identity` in `action_shim/mod.rs`). `issue_and_audit` returns `CaIssuanceError::Ca` when the CA cannot sign and `::Audit` when the `issued_certificates` write
fails (a redb write error, a full disk). Today the error's only sink is the `convergence.evaluation.completed` tracing event; on the
appliance OS there is no operator shell to read it, so a tracing-only failure is not an operator surface, and a gateway or workload
whose SVID never issues reads exactly like one that has just started.

### Shared value types (`overdrive-core::ca::issue_failure`, new module)

```rust
/// Failure class of a rejected issuance; the `CaIssuanceError` variant class only, never its source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum SvidIssueFailureCause { Ca, Audit }
// No `as_str`: no named consumer. The stable lowercase wire/CLI label ("ca" | "audit") comes from the serde `snake_case`
// rename on `SvidIssueFailureCauseBody` below; a log site that needs one adds it then.
/// Depth-1 snapshot of the latest failed issuance. Not nestable: it carries no failure of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SvidIssueFailure {
    pub cause: SvidIssueFailureCause,
    pub at: UnixInstant, // the injected Clock's `unix_now()` when the executor observed the `Err`
}

// overdrive-control-plane::ca_issuance
impl CaIssuanceError {
    pub fn failure_cause(&self) -> SvidIssueFailureCause; // Ca => Ca, Audit => Audit; pure and total, `&self`, source text never read
}
```

`failure_cause` is a public method (cross-crate: both executors call it from `overdrive-control-plane::action_shim`; whether it is
also `const` is the crafter's, callers cannot tell).

Wire mirrors (`overdrive-control-plane::api`, `Serialize + Deserialize + ToSchema`, defined once, used by both halves):

```rust
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SvidIssueFailureCauseBody { Ca, Audit }
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct SvidIssueFailureBody {
    pub cause: SvidIssueFailureCauseBody,
    #[schema(value_type = String)]
    pub at: UnixInstant,
}
// The one conversion from the domain value to its wire mirror, used by both halves (gateway `GatewayApplicationStatusBody`, workload
// `AllocStatusRowBody`). Infallible and total: `cause` maps variant-for-variant (Ca => Ca, Audit => Audit), `at` is copied.
impl From<SvidIssueFailure> for SvidIssueFailureBody { fn from(failure: SvidIssueFailure) -> Self; }
```

The cause mapping inside that `From` is internal; no `From` for `SvidIssueFailureCause` is part of the contract. There is no inverse conversion
(the wire mirror is never read back into the domain), and no `TryFrom`: no input is rejected.

### Gateway half (R-45, R-47, R-48, R-49, R-52)

```rust
// overdrive-core::gateway_identity
/// Process-local issue-failure record of the gateway identity slot. The slot holds `Option<GatewayIdentityIssueRecord>`: `None`
/// is "no failure this process", so a nonzero count without a `last` (or the reverse) is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayIdentityIssueRecord {
    pub count: NonZeroU64,
    pub last: SvidIssueFailure,
}
```

- **Recording (R-45, R-50).** In `action_shim::gateway_svid::dispatch_issue`, when `issue_and_audit` returns `Err(e)`, the executor builds
  `SvidIssueFailure { cause: e.failure_cause(), at }` from the injected `Clock` and calls `target.record_issue_failure(failure)`;
  it then returns `ShimError::GatewaySvidIssue(e)` exactly as before. No row is written by the executor. The slot adds one to the count and
  replaces `last` (creating the record on the first failure) and notifies the owner. The record is epoch-independent: a failed issuance that
  completes after `disable_after_drain` really happened and is still an occurrence, so there is no `StaleEpoch` outcome here. The facts survive
  `begin_disable` and `disable_after_drain` within a process (they clear only with the process). `hold_audited` success never touches them. The
  held identity, `desired()` and `current()` are unchanged by recording (a recording is bounded-change over the facts alone).
- **Reading and wake (pinned carrier).** One new read method, `issue_record() -> Option<GatewayIdentityIssueRecord>`, on
  `GatewayIdentityLifecycleControl` and `GatewayIdentityLifecycleTarget` (port blocks above), implemented by the slot's lifecycle handle, the
  control-plane adapter (delegating), `SimGatewayIdentityActionTarget` / `SimGatewayIdentityLifecycleControl`, and every other implementer
  of `GatewayIdentityLifecycleControl` / `GatewayIdentityLifecycleTarget` (production, Sim, and test-support doubles, including those under
  `crates/overdrive-gateway/src/application/held_probe_doubles.rs` and the authored tests) returns `None` or delegates. The inherent and
  trait copies of `issue_record` on the slot and on the Sim target follow the existing `current`/`subscribe` pattern, and the crafter may
  collapse them. The wake reuses the existing `subscribe()` watch: a recorded failure resolves `changed()` on every receiver although the
  `Option<GatewayIdentityFacts>` item is unchanged, and the owner, woken, reads `current()` and `issue_record()`. The owner reads
  `current()` and `issue_record()` as two reads whose pair is not atomic; every mutation notifies, so the next wake corrects any transient
  mismatch. The `subscribe` item type, `current()`, `GatewayIdentityView` and every admission reader are
  untouched. Rationale: `Option<GatewayIdentityFacts>` is `None` when nothing was ever held, so the record cannot ride in that item; a read
  method plus the existing notification is the smallest change (no second watch, no changed item type). Other `subscribe` waiters
  (`ensure_current`) tolerate an unchanged-item wake by re-checking. At most one wake per failed issuance (one per retry-backoff tick).
- **No row before a Route (R-58).** The owner writes no `GatewayApplicationStatusRowV1` until it has observed a Route in the Public Route Set,
  and neither an application transition nor an identity or failure wake writes one beforehand; the status of a gateway with no Route and no
  prior row is the absence of a row (`application = None`, R-47). Once a Route has been observed, every projection writes (including
  `RouteAbsent` after a later withdrawal). When a prior row exists, the owner projects over its base exactly as R-52 states.
- **Projection (the owner, the sole writer; R-52).** At its first projection the owner reads the prior row with
  `ObservationStore::gateway_application_status_row(node_id)`, where `node_id` is the `NodeId` that `boot_server` G2 passes to
  `UnboundGatewayBuilder::new` (R-57) and that also keys every row the owner writes, and keeps `(base_failures, base_last)` privately (`(0, None)` when absent). A
  read `Err` is retried like any failed row write: the facts stay in the slot, no row is written from a defaulted zero, and the next
  projection reads again (no boot refusal, no new `GatewayBootError` variant). An undecodable prior row reads as absent through the point read, which emits one
  `observation.row.skipped` warning (R-56, ADR-0048), and the count then restarts at zero (R-52). Every row the owner writes carries
  `issue_failures = base_failures.saturating_add(record.count)` and `last_issue_failure = record.last` (else the base values when the record
  is `None`). A failed owner row write loses nothing the slot holds: the record stays in the slot and the next projection carries it. The
  count is monotone across restarts through the base read; "sole writer" means the owner function alone writes this table by convention, not
  by a type that forbids other writers.
- **Projection runs during the boot identity wait (pinned; R-45 correctness).** `GET /v1/gateway/status` is served by `server_task`, which
  G4 spawns before the gateway tasks start, and it reads only `GatewayStatusReads` and ObservationStore point reads; it depends on neither the
  gate nor the identity. The status is therefore readable while the gate is blocked in `ensure_current`, and a never-issuing identity is the
  case R-45 exists for. So the owner's handling of an identity-watch wake is not sequenced behind the gate: a failure wake that arrives while
  the gate waits produces a projection, and the row then reads `gateway_identity = Absent` (R-48: `current()` is `None`),
  `issue_failures >= 1` and the latest `last_issue_failure`, beside whatever application facts the owner holds at that moment (a staged
  candidate, or none). The same holds after the gate times out and for the life of the process; the projection never waits for the gate to
  finish or fail. Identity and issue-failure facts are written on identity-watch wakes; application facts are written on owner transitions;
  every row write carries the newest value of both from the owner's one write path (the sole-writer and retry-on-next-wake rules above are
  unchanged). Before a Route exists, and with no prior row, an identity or failure wake writes no row (R-58); the slot keeps the facts and the
  first row after a Route is observed carries them. How the owner schedules the wait and the projection (a select arm, a sub-task) is the crafter's.
- **Status availability when the identity never issues (pinned; no new timer).** The endpoint's availability is independent of identity and
  has no bound of its own: it answers 200 from the moment the operator listener serves (500 only on an observation read error, unchanged).
  The row it reads is bounded by two existing timers. (a) The gate's wait ends at the absolute `gate_deadline = clock.now() +
  application_gate_deadline` (30 s by default, `GatewayLimits::first_slice`, `crates/overdrive-gateway/src/runtime/limits.rs:123`), never
  restarted; at it the staged candidate is removed and `unavailable = GatewayIdentityUnusable`, and the status keeps being served with the
  count still growing. (b) Failures are recorded at the cadence of the existing `GatewaySvidLifecycle`: an attempt when the gate's
  `ensure_current` submits its Immediate evaluation, otherwise within one `GATEWAY_SVID_RESYNC_PERIOD` (30 s,
  `crates/overdrive-reconcilers/src/gateway_svid_lifecycle.rs:41`, host-backed resync), later attempts after `backoff_for_attempt` (a constant
  1 s, `workload_lifecycle.rs:63`). So the first failure appears in the status within one resync period of boot, each next one within one
  period of the previous, and a recovery appears within one period of the fault clearing. Tests may poll in real time under `serve` up to
  two periods (60 s) for each of these moments.
- **Identity classification (R-48).** `gateway_identity` is a pure function of `current()` and the owner's injected `Clock`: `None` => `Absent`;
  `Some(f)` with `f.not_after` not after `clock.now_unix` => `Unusable`; else `Current { epoch, spiffe_id, serial, not_after }`. The owner
  projects at its existing wakes plus the failure wake; no timer or expiry reaction is added. `Unusable` is therefore reached when a
  projection runs with held material expired, and in the real case rotation is failing, so failure wakes occur. Staleness bound: until the
  next projection the row may read `Current` for material that has since expired. Lifecycle errors (`Timeout`, `Closed`,
  `ReconcilerUnavailable`) set only `unavailable = Some(GatewayIdentityUnusable)` (S-PIG-51) and never write `gateway_identity`. The
  application-level cause `GatewayApplicationUnavailableCause::GatewayIdentityUnusable` and the identity value
  `GatewayIdentityStatus::Unusable` are different fields: the former is set whenever the application cannot promote on identity (Absent,
  Unusable, or an `ensure_current` error), the latter only for held-but-expired material.
- **Disabled (R-47).** `GatewayAppStateComposition::disabled()` has `status() == None`; the handler returns the fixed disabled body before
  any store call. The disabled composition's `DisabledGatewayIdentityRead` is unchanged. No record exists because no slot is enabled. (Within
  a process, `begin_disable` keeps the record; a disabled boot has none.)
- **`IssueFailed` (R-49).** Removed from `GatewayIdentityLifecycleError` (edited in place in the port block above). Nothing produced it.
- **Body (R-45).** `GatewayApplicationStatusBody.issue_failures` and `.last_issue_failure` (defined in place above) are a pure projection of
  the row: `last_issue_failure = row.last_issue_failure.map(SvidIssueFailureBody::from)` (the infallible `From<SvidIssueFailure> for
  SvidIssueFailureBody` pinned in "Shared value types").
- **Recordable and unrecordable failures.** The record is in-memory in the slot, so a gateway failure is always retained, including an `Audit`
  failure caused by a store outage (the gateway half keeps its facts in the slot even when every store write fails; the row then carries them
  once a write succeeds). The `Ca` cause is reachable only from a CA-backend error; no production path is observed to produce it today, and the
  failure expected in practice is `Audit`.
- **Row invariant (owner-side).** `issue_failures == 0` iff `last_issue_failure` is `None` holds because the owner's projection is the one
  write path and builds both fields from one `(count, last)` pair (base plus record); no other constructor of the row's issue fields exists.
- **Owner accessors.** The status row is read through the existing `GatewayStatusReads::application_status()`. No model variable is
  projected (the Quint spec is unchanged), so no conformance accessor is pinned.

### Workload half (R-46, R-50, R-51)

```rust
// overdrive-core::traits::observation_store (new row; alloc rows untouched)
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct WorkloadSvidIssueStatusRowV1 {
    pub alloc_id: AllocationId,
    pub node_id: NodeId,
    /// Monotone count of failed issuances for this allocation (first issue and `rotate-svid` reissue alike); never cleared by a success.
    pub issue_failures: u64,
    /// Latest failed issuance. Not optional: a row exists only after at least one failure.
    pub last_issue_failure: SvidIssueFailure,
    pub updated_at: LogicalTimestamp,
}
pub type WorkloadSvidIssueStatusRow = WorkloadSvidIssueStatusRowV1;
pub enum WorkloadSvidIssueStatusRowEnvelope { V1(WorkloadSvidIssueStatusRowV1) } // VersionedEnvelope, Latest = V1 payload
impl WorkloadSvidIssueStatusRowV1 {
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError>;
    pub fn from_store_bytes(bytes: &[u8], key: Option<&str>) -> Result<Self, ObservationStoreError>;
}
// No ObservationWrite, ObservationRow or ObservationRowKind variant is added: nothing subscribes to this row (no reconciler declares an
// interest in it); it is read by point read only, and the operation below is the only authoring route.

// ObservationStore (two adapters implement: LocalObservationStore, SimObservationStore)
/// Atomically read the prior row for `alloc_id`, install `issue_failures = prior + 1` (saturating; 1 when absent or undecodable, the
/// undecodable case emitting the store's one `observation.row.skipped` warning, as the point read does; R-56),
/// `last_issue_failure = failure`, `updated_at = LogicalTimestamp { counter: prior.counter + 1 (1 when absent), writer: node_id }`.
/// One mutation; the only authoring route for this table. Nothing reads a returned row.
async fn record_workload_svid_issue_failure(
    &self,
    alloc_id: &AllocationId,
    node_id: &NodeId,
    failure: SvidIssueFailure,
) -> Result<(), ObservationStoreError>;
/// The stored row for `alloc_id`, or `None` when no failure was recorded. An undecodable stored row reads as `None` with one
/// `observation.row.skipped` warning emitted by the store (R-56).
async fn workload_svid_issue_status_row(
    &self,
    alloc_id: &AllocationId,
) -> Result<Option<WorkloadSvidIssueStatusRowV1>, ObservationStoreError>;
```

- **The sole writer** is the function `action_shim::issue_svid::dispatch_issue` (by convention; nothing in the type system forbids another
  caller of the store operation), through `record_workload_svid_issue_failure`, called with the action's `alloc_id` and `node_id` after
  `issue_and_audit` returned `Err(e)` (R-50), with `SvidIssueFailure { cause: e.failure_cause(), at }`. Both callers of `dispatch_issue`
  (the convergence arm and `ensure_intercept_identity` in `action_shim/mod.rs`) record, because the recording lives inside the function. The
  operation is atomic so two failed issuances for one allocation can never lose a count, with no reliance on dispatch ordering
  (`rust.md` § "Check-and-act must be atomic").
- **Executor result.** A failed recording is not absorbed: the executor surfaces both the original issuance failure and the recording failure
  in the error it returns to the shim (the crafter shapes the error; no variant name is pinned), logs the recording failure, and holds
  nothing. An unrecorded failure changes no count; the next failed attempt records. What stays recordable: a transient per-write failure of the
  `ObservationStore` (the `Audit` failure that caused the issuance failure need not fail the record write). The workload half cannot retain the
  fact when every store write fails (the record shares the `ObservationStore` with the audit write), so an all-writes outage loses the count
  while it lasts; the same outage fails the status read, which the handler does not hide. The `Ca` cause is reachable only from a CA-backend error;
  no production path is observed to produce it today.
- **Handler.** `GET /v1/allocs` reads `workload_svid_issue_status_row(alloc_id)` once per allocation row in the response (the same
  per-allocation read pattern as probe results) and stamps the two body fields below, as it already stamps `network_cleanup_pending`. Cost: one
  redb point read per allocation row per operator request; no new table-scan operation is added for it. The stamped
  `last_svid_issue_failure` is the row's `last_issue_failure` through the infallible `From<SvidIssueFailure> for SvidIssueFailureBody`
  pinned in "Shared value types". A read `Err` is returned as the
  handler's existing internal error, never defaulted to zero.
- **Wire (`AllocStatusRowBody`, additive, `#[serde(default)]`).** `svid_issue_failures: u64` (0 when no row) and
  `last_svid_issue_failure: Option<SvidIssueFailureBody>` (`skip_serializing_if = "Option::is_none"`). An old body without them deserializes
  to `0` / `None`.
- **Renderer (`render::workload_describe`).** A presence-guarded per-row detail helper beside `render_network_cleanup_pending_detail`
  prints, only when `svid_issue_failures > 0`, the line `    svid issuance failures: <N> (last: <cause> at <at>)` (`<cause>` is
  `SvidIssueFailureCauseBody`'s serde `snake_case` label, `ca` or `audit`; `<at>` is the `UnixInstant` `Display`), under both the Service and the Job arm; a row with no
  failure renders byte-identically to before.
- **Retention.** No deletion path; at most one fixed-size row per `AllocationId` that ever failed an issuance. `DropSvid` and a later success
  do not touch it.
- **Out of scope.** The trust-bundle refresh failure after a successful issuance (`issue_svid.rs` cites it) is not a failed issuance and is
  not recorded here. It is GH #223, open and disjoint (a standing-state row); this change neither covers nor constrains it. DELIVER updates
  the #223 comments in `issue_svid.rs:113-114,129` if the new recording changes what they describe.

### Persisted format (interface contract; pinned for DISTILL, 2026-11-04)

The redb tables and keys of the two rows are persisted formats, so they are contract, not implementation. They follow the convention of the
existing observation-row tables in `crates/overdrive-store-local/src/observation_backend.rs` (`observation_alloc_status`,
`observation_node_health`, `observation_service_backends`, `observation_probe_results`, `observation_issued_certificates`, ... ; the two
tables without the prefix, `alloc_lifecycle_occurrences` and `issuance_ordinal_counter`, are an occurrence ring and a counter, not LWW
rows). Pinned under R-45, R-46 and R-52; no decision of those rulings changes.

| Row | redb table (`TableDefinition<&[u8], &[u8]>`) | Key | Value |
|---|---|---|---|
| `GatewayApplicationStatusRowEnvelope::V1` | `observation_gateway_application_status` | `NodeId::as_str().as_bytes()` (the same encoding as `observation_node_health`) | `GatewayApplicationStatusRowV1::archive_for_store()` bytes |
| `WorkloadSvidIssueStatusRowEnvelope::V1` | `observation_workload_svid_issue_status` | `AllocationId::as_str().as_bytes()` (the same encoding as `observation_alloc_status`) | `WorkloadSvidIssueStatusRowV1::archive_for_store()` bytes |
| `PublicCertifiedKeyStatusRowEnvelope::V1` | `observation_public_certified_key_status` | the certified-key ID string's UTF-8 bytes | its `archive_for_store()` bytes (named here only so the three new status tables share one convention; its content is not part of this change) |

One row per key; the key has no composite part and no separator. An undecodable stored value (envelope or layout failure) reads as absent
through the point read (R-56). The store, not the codec, emits exactly one warning named `observation.row.skipped` with separate `table`
(the redb table name above) and `key` (the row's id string) fields, the shape of the `observation.envelope.decode_failed` event
(`table`, `key`, `source`) that the scan path emits. `from_store_bytes(bytes, key)` returns the decode error and does not log. The
existing `observation_alloc_status` path emits `observation.envelope.decode_failed` (pinned by `envelope_observation_skip.rs`) and is
unrelated; the `observation.row.skipped` emission of `IssuedCertificateRow::from_store_bytes`, with `key` and `envelope_error` and no
`table` field, is a separate existing emitter and is not part of this ruling. For the workload row,
`record_workload_svid_issue_failure` treats an undecodable prior row as absent: the same warning, then `issue_failures = 1` and
`updated_at.counter = 1`. For the gateway row, the owner's first-projection read (R-52) treats it the same way: the count restarts at zero.
A raw-bytes back-door that writes garbage under these table and key names is therefore a faithful stimulus for O-SIF-G3.

### Contract shapes (`Reuse Analysis` extension)

| Component | Shape | Mutation set / universe | Assertion mechanism |
|---|---|---|---|
| `record_workload_svid_issue_failure` | bounded-change | exactly one row of `observation_workload_svid_issue_status` at `alloc_id`; universe: every other family the `ObservationStore` port exposes (below) unchanged and no subscription event | before/after read of every family with an unscoped bulk read (`alloc_status_rows`, `node_health_rows`, `issued_certificate_rows`, `all_service_backends_rows`, `workflow_terminal_rows`) plus the `subscribe_all_events` stream drained empty; Local/Sim equivalence run. Families readable only by key (hydration results, reconcile conflicts, probe rows, occurrences, the two gateway status rows) cannot be enumerated through the port; for them the exposure is that a Local implementation which opened another table would not be seen, and the bound is the single table the operation opens (reviewed in the diff, not tested) |
| `GatewayIdentityActionTarget::record_issue_failure` | bounded-change | slot issue record (plus a `subscribe` notification with an unchanged item); universe: held identity, `desired()`, `current()` unchanged | `current()`/`desired()` equal before and after |
| `dispatch_issue` (both executors), failure arm | bounded-change | the record (gateway) or the one row (workload); universe: `IdentityMgr` held map, slot identity, `issued_certificates` | Sim-store delta; held map and slot equal before and after |
| Owner row projection (`base + record`) | pure-function | none | property test over (base, record, sequence) |

### Lifecycle Gate Ownership — SVID issuance failure occurrence

Not applicable: no gate is added, removed or moved. Evidence: the recording happens after `issue_and_audit` returned `Err`, inside the
executors, and no gate input reads it: Gateway Identity Current (bind, admission, handshakes) reads `current()` only, `SvidLifecycle` and
`GatewaySvidLifecycle` read held sets and audit rows, and `IssueRetry` is not read (R-50). The `gateway_identity` classification of R-48 gives
`Unusable` the meaning the existing `IdentityExpired` outcome already has; it does not change what the existing gate does with it.

### DISTILL obligations (the acceptance designer authors the rows; these are required coverage)

Fault stimuli at driven ports only: a failing Sim `Ca` (`issue_svid` returns an error) for `Ca`; a Sim `ObservationStore` fault scoped to the
`ObservationWrite::IssuedCertificate` write for `Audit` (scoped so the status and failure-record writes still succeed); a Sim
`ObservationStore` fault that also fails the failure-record write for the unrecorded arm; a failing status-row write for the owner; the Sim
clock for time. The fault seam itself is DISTILL's to shape (the existing one-shot `inject_write_failure` is the starting point) and is not
pinned here. Causes are driven, never seeded: the test submits the Route or the workload and lets the production reconciler emit the
issue action. Seeded `overdrive-sim` invariants for ordering: repeated-failure monotonicity under seeds and restart interleavings.
The rows are seeds of distinct behaviours; DISTILL owns the full matrix.

| Id | Obligation |
|---|---|
| O-SIF-G1 | Gateway, repeated failure: `n` failed issuances (a `Ca` failure then an `Audit` failure among them) give `issue_failures == n` and `last_issue_failure` with the latest cause and the Sim-clock instant; the count never decreases across failures, successes and restart (property over sequences); the status body carries no CA or audit error text |
| O-SIF-G2 | Gateway, recovery: failures then a success give `Current` with the count and `last` retained; `Absent` with `n > 0` is distinguishable from the fresh `Absent` with `0` and `None` |
| O-SIF-G3 | Re-composing over the same durable store continues the count (`base + new`) and keeps the prior last failure until a new failure; a prior-row read error is retried, no row is written from a defaulted zero, and the row later carries `base + process`. An undecodable stored row (garbage under the pinned table and key of § Persisted format) reads as absent through the point read with exactly one `observation.row.skipped` warning emitted by the store with separate `table` and `key` fields (R-56), and the workload record operation then restarts at one. Tests: `a_restarted_gateway_continues_the_failure_count_from_the_prior_row`, `a_prior_row_read_error_is_retried_and_never_defaulted_to_zero` (gateway integration); `an_undecodable_gateway_status_row_reads_as_absent_with_one_skipped_warning`, `an_undecodable_workload_issue_status_row_reads_as_absent_and_the_record_restarts_at_one` (store-local integration, `envelope_observation_skip.rs`). The owner's base-0 restart over an undecodable row is the owner reading that `None`; the gateway `World` composes the Sim store, which has no byte layer, so that half is a recorded gap | NEW | I | @error @degraded-resource @contract-shape:bounded-change | gateway integration; `overdrive-store-local` integration | gateway-svid-issue-failure-occurrence, workload-svid-issue-failure-record |
| O-SIF-G4 | Gateway disabled (R-47): `GET /v1/gateway/status` returns the fixed body with zero ObservationStore reads (counting Sim store), including when the store holds a row from an earlier enabled boot; no row is written by a disabled boot |
| O-SIF-G5 | Gateway `Unusable` versus `Absent` (R-48), with a Route declared (R-58: with no Route and no prior row, an identity failure writes no row): empty slot => `Absent`; a projection that runs with held material past `not_after` (rotation failing) => `Unusable` with the count above zero; a boot `ensure_current` timeout => `unavailable == GatewayIdentityUnusable` and `gateway_identity` as the slot says |
| O-SIF-G6 | Gateway: a failing owner status write does not lose the record; the next successful write carries the full count |
| O-SIF-G7 | Gateway row: `FIXTURE_V1` regenerated for the new layout (R-51), with the round-trip property |
| O-SIF-W1 | Driven by the production `svid-lifecycle` reconciler over a Running allocation: first-issue `Ca` and `Audit` failures and a `rotate-svid` reissue failure each raise the count by exactly one and carry the latest cause and Sim-clock instant, through the store and through `GET /v1/allocs`; a success never clears them; failures leave no audit row; a failing record write is surfaced (the tick's error differs from the one returned when the same failure is recorded, because it also carries the recording failure), not counted, and the next failed attempt records. Tests: `repeated_failed_issuances_give_a_monotone_count_and_the_latest_cause_on_the_allocations_surface`, `a_failing_record_write_is_surfaced_and_not_counted_and_the_next_failure_records` (control-plane acceptance); the operator line is O-SIF-W3 | NEW | U | @error @contract-shape:bounded-change | `overdrive-control-plane` acceptance `svid_issue_failure_occurrence.rs` | workload-svid-issue-failure-occurrence |
| O-SIF-W2 | `record_workload_svid_issue_failure` is the same operation in `LocalObservationStore` and `SimObservationStore` (count rises by one, latest failure, logical counter rises from one, no row for an allocation that never failed), changes no other observation family (the port-expressible universe: every family with an unscoped bulk read, `alloc_status_rows`, `node_health_rows`, `issued_certificate_rows`, `all_service_backends_rows`, `workflow_terminal_rows`, plus the `subscribe_all_events` stream; a key-only family is bounded by the operation opening one table, reviewed in the diff), and is atomic under 32 simultaneous calls on one allocation in both adapters. Tests: `the_failure_record_is_the_same_operation_in_the_local_and_sim_stores`, `concurrent_recordings_on_one_allocation_never_lose_a_count` (`overdrive-store-local` acceptance) | NEW | U | @property @adapter-contract @concurrency @contract-shape:bounded-change | `overdrive-store-local/tests/acceptance/workload_svid_issue_status.rs` | workload-svid-issue-failure-record |
| O-SIF-W3 | CLI: `render::workload_describe` (the renderer the CLI calls; `render::alloc_status` is its pre-#220 name and no longer exists) over a snapshot with `svid_issue_failures > 0` prints the pinned line under a Service row and a Job row; with 0 the output is byte-identical to before |
| O-SIF-W4 | Handler: an observation read error yields the handler's error, not a zero |
| O-SIF-W5 | Schema evolution: golden fixture and round-trip for `WorkloadSvidIssueStatusRowEnvelope::V1` |

Existing artifacts the acceptance designer must align (not edited here, they are test and code surfaces): the exhaustive match over
`GatewayIdentityLifecycleError` in `crates/overdrive-gateway/tests/integration/public_ingress_gateway.rs` loses the `IssueFailed` arm and
`crates/overdrive-gateway/src/ports.rs` loses the variant; `FIXTURE_V1` in
`crates/overdrive-core/tests/schema_evolution/gateway_application_status_row.rs` is regenerated for the new V1 layout (R-51: that V1 never
shipped, so this is not a touch of a historical fixture; DISTILL rewrites the file's "never touched" header text accordingly); the QC-1 `audit_fail` handler is unchanged (the count is not a model variable).

## Wave: DESIGN / [REF] Updated Reuse Analysis

| Existing component | Overlap | Final application decision | Why |
|---|---|---|---|
| `boot_server` (`M:crates/overdrive-control-plane/src/lib.rs:3625`) / `ServerHandle::shutdown` (`:2890-2993`) | startup/task/drain | **EXTEND** | Sole active gateway/app/listener/connector/drain owner (D5 slot, boot G0-G4, shutdown stage 0); no supervisor, no `*_override` |
| workload deploy parser/API | generic `deploy` journey | **EXTEND WITH OUTER DISCRIMINATOR** | Route stays outside workload enum/endpoint/stream |
| IntentStore + ADR-0048 codec | durable intent and wake | **EXTEND** | Reuse store/envelopes; lag-aware watch is required for fail-closed withdrawal |
| `ListenerFactStore` | listener/VIP projection | **DO NOT EXTEND** | Derived ServiceId key, stop eviction and binary absence cannot fulfill stable reference/cause contract |
| WorkloadIntent + `ServiceVipView` | exact frontend inputs | **REUSE** | Direct authoritative join; no duplicate map |
| ServiceLifecycle / ServiceBackendRow | Backend Eligibility | **REUSE INDIRECTLY; FORBID GATEWAY READ** | Sole complete-row owner remains |
| ServiceMapHydrator / Dataplane / XDP | backend application/selection | **REUSE HYDRATOR AND `Action::DataplaneUpdateService` UNCHANGED (D1); EXTEND DATAPLANE AND ADD a separate gateway `cgroup/connect4` program (`cgroup_connect4_service` UNCHANGED); XDP UNCHANGED** | Ordinary Service-map programming covers VM backends (D4 held); registered cgroup arm + dataplane-prepared upstream socket + receipt (D3, ADR-0243); no selector/raw handler access |
| `BackendIdAllocator` | endpoint-to-id memo | **REUSE MONOTONIC NONREUSE; one crate-private read, `existing_id` (R-24)** | `existing_id` permits the identity-conflict preflight before mutation; allocation/release semantics stay owned by Dataplane |
| KEK/HKDF/AES-GCM mechanism | key protection | **REUSE MECHANISM, DISTINCT PUBLIC-KEY RECORD/DOMAIN** | Avoid CA/public-key type confusion and subkey reuse |
| rustls | TLS handshake mechanics and `KernelConnection` | **REUSE LIBRARY; CREATE PUBLIC RUNTIME** | Operator CA/router are a different trust boundary; `hyper` is not on the public data path (G1) |
| `Ca` / `ca_issuance` / IdentityMgr | SVID issue/audit/hold foundations | **REUSE ISSUE/AUDIT; CREATE DEDICATED SLOT** | Allocation holder stays unchanged; no fake AllocationId |
| `SvidLifecycle` | allocation identity convergence policy | **REUSE POLICY; CREATE GATEWAY SIBLING** | One node slot has different desired/actual/actions |
| ObservationStore | convergent non-secret status | **EXTEND THREE ROW FAMILIES** (the prior two plus the workload SVID issue status row, R-46) | Existing store/LWW mechanism; no new store/event sourcing |
| `arc-swap`, `async-trait`, `rustix`, `zeroize`, `x509-parser` (`tokio-rustls` is no longer promoted: G1 removes it from the public path) | atomic pointer/object-safe async/safe SO_COOKIE/secret/X.509 mechanics | **PROMOTE LOCKED OSS DEPENDENCIES** | Narrow necessary mechanics; MIT/Apache/ISC-compatible, no proprietary dependency |

Contract-shape extension of the Reuse Analysis (the declared universe is the
only mutation set; everything else is complement-equal):

| Reuse rows | Contract shape / declared universe | Assertion mechanism |
|---|---|---|
| serve + workload deploy | **bounded-change**: mandatory Enabled/Disabled AppState identity composition, `BootAdapters.gateway`, optional gateway handle/control/task tree and shutdown stage 0 only | disabled-state delta (boot with `gateway == None` is byte-equal to pre-feature), existing workload regression, boot/shutdown integration |
| IntentStore + ObservationStore | **bounded-change**: fixed Route/key and the three status keys/tables only (the third, `workload_svid_issue_status`, is R-46's) | codec goldens, store state-delta PBT, lag/relist DST |
| ListenerFactStore | **unbounded-preservation**: no call/import/change | dependency lint + source-diff guard |
| WorkloadIntent + ServiceVipView | **unbounded-preservation** read of exact reference inputs | adapter equivalence with whole-store/allocator complement |
| ServiceLifecycle / ServiceBackendRow | **unbounded-preservation** indirect input; gateway cannot read/write | import lint + pre/post row equality |
| ServiceMapHydrator / Dataplane / XDP | **bounded-change**: the separate registered-socket gateway selection program, the two intent/receipt maps, BackendId-to-identity association and upstream-socket preparation; hydrator, `Action::DataplaneUpdateService`, SERVICE/BACKEND/REVERSE_NAT contents, LOCAL_BACKEND_MAP behaviour for unregistered sockets and XDP preserved | state-delta DST, BPF POD tests, real-kernel complement probe (unregistered socket byte-for-byte unchanged) |
| BackendIdAllocator | **bounded-change**: read-only `existing_id` (R-24); allocator state unchanged by preflight | allocator snapshot PBT + conflict no-mutation integration |
| KEK/AEAD mechanism | **bounded-change**: distinct public-key HKDF/AAD/record; CA records preserved | tamper/fault integration + domain-separation golden |
| rustls + `GatewayRelay` | **bounded-change** per public/upstream connection and its finite budget (two sockets, up to four pipe descriptors) | protocol matrix, wrong-peer integration, relay Tier-3 suite and wire AT |
| tracing + CLI compact `EnvFilter` subscriber | **unbounded-preservation** of runtime state with one connector-owned DEBUG occurrence after validated receipt/identity resolution | append-only existing Tier-3 process log; no status/store/ring-buffer state |
| CA issue/audit + IdentityMgr | **bounded-change**: one gateway-subject audit output; allocation holder untouched | audit-before-hold integration + allocation-held-set complement |
| SvidLifecycle | **pure-function** sibling inputs -> gateway Actions/View only | paired lifecycle PBT + existing workload reconciler byte equality |
| arc-swap/async-trait/rustix/zeroize/x509-parser | **bounded-change** inside the owning snapshot/connection/syscall/secret values | API/trybuild guards, parser/syscall fault matrix and drain tests |

The mandatory CREATE NEW challenge is satisfied: no new component duplicates an
existing owner, and the new crate adds no deployment unit. The crate-class
rules of `cargo xtask dst-lint` (`control-plane -> {gateway,dataplane,reconcilers} -> core`,
the gateway's host edge, and Dataplane's gateway-port/host-reader edges) apply as they do to any
workspace crate; `xtask/src/dst_lint.rs` has no gateway-specific clause. The following are **review rules,
not lint rules**: gateway production code does not reference `ServiceBackendRow`, its all-row reader, or raw
SERVICE/BACKEND/MAGLEV map symbols; `GatewayClientIdentityAccess::with_current` is called only in
`overdrive-dataplane/src/mtls/gateway_client.rs`; and `GatewayUpstreamSeal::seal_authenticated` is called only there and in the exact
`overdrive-sim` gateway-client adapter. Trybuild/API tests keep secret types
non-renderable/non-serializable and application fields private.

## Wave: DESIGN / [REF] Public surface and live-caller audit

Every new cross-crate item is reachable from the first-slice production
composition. Items needed only inside `overdrive-gateway` are crate-private;
the table is the final dormant-surface gate:

| Exact surface group | Constructor/producer | Live production caller/consumer |
|---|---|---|
| Route IDs/hostname/path/listener/Route/generation plus parser and HTTP input types | top-level deploy parser and `Route::from_submit` | `deploy_resource` → `ApiClient::submit_route` → Route POST; Route owner/application/status use accessors; public runtime calls `PathMatch::matches_raw_path` |
| `PublicRouteSetHandle::{apply,withdraw}` | crate-private Route owner startup inside `UnboundGatewayBuilder::new` | Route POST/DELETE through `GatewayRouteCommands::routes`; there is no CLI DELETE verb |
| `ApiClient::{withdraw_route, gateway_status}` (R-53, P-38 adopted 2026-11-04) | `overdrive-cli::http_client` | **No production caller, by user ruling** (the dormant-surface gate is waived for these two methods only): authored callers are the operator-surface integration tests (S-PIG-35, S-PIG-36 DELETE arm, S-PIG-42); no CLI verb |
| Four typed gateway clap/`ServeArgs` fields, `GatewayConfig`/`ManualCertifiedKeyConfig`, `GatewayConfigError` and accessors | clap `Command::Serve` → `main` → first `ServeArgs::gateway_config` call | `commands::serve::run_with_adapters` (`M:crates/overdrive-cli/src/commands/serve.rs:153`) moves only complete config into `ServerConfig.gateway`; `boot_server` and `UnboundGatewayBuilder::new` consume it; handlers never receive paths |
| `GatewayAppStateComposition`, canonical disabled identity read and appended AppState field/constructor parameter | broad fixtures default Disabled; production/Tier-3/Sim may pass Enabled | Hydration builder, every action/workflow wrapper and Route/status handlers through accessors only; Arc ownership preserves AppState Clone/borrow lifetime |
| `ControlPlaneError::GatewayDisabled` and exact response arm | Route POST/DELETE pre-parse control check | exhaustive `to_response` and live OpenAPI 409 ErrorBody |
| `ServiceFrontendResolve::{probe,resolve}` and closed errors | private `IntentServiceFrontendResolver` construction in the builder | gateway owner startup/application convergence only |
| `BootAdapters.gateway`/`GatewayAdapters::host`, `GatewayClientMtlsFactory`/`GatewayClientMtlsDeps`, `ComposedDataplane.gateway_connect` | `NodeAdapters::host`/`HostDataplaneComposer::compose` and their Sim swaps (`SimDataplaneComposer`, `SimGatewayClientMtlsFactory`) | `boot_server` G2-G4 only; no `*_override`, no downcast of `Arc<dyn Dataplane>` |
| `GatewayConnectDataplane` probe/`open_upstream`/receipt/identity methods, the `GatewayConnectCleanup` supertrait (cleanup, sweep, `live_intent_count`, `live_receipts`) and `PreparedUpstream` | `EbpfDataplane` and `SimGatewayConnectDataplane` | `GatewayUpstreamConnector::{probe,connect}` (intended sole caller of `open_upstream`/`into_parts`; module privacy, see "Permitted callers"), handle shutdown sweep and status live-count projection |
| Commit-gated backend-identity publication inside `EbpfDataplane::update_service` (ADR-0233) and the crate-private `BackendIdAllocator::existing_id` (R-24) | existing hydrator → unchanged `Action::DataplaneUpdateService` → action shim | `selected_backend_identity` reads only after the outer swap commits; no new action, `DispatchOutcome` or gateway argument |
| `IntentSubscriptionEvent` and revised `IntentStore::watch` | LocalStore and Sim | every existing watcher plus Route/custody/application subscribe-before-list loops |
| `SocketCookie`, `GatewayIdentityEpoch`, `CertifiedKeyGeneration`, `CertificateFingerprint` constructors/accessors | dataplane-internal socket reader, gateway slot, certified-key candidate/record decode | BPF adapter (`open_upstream`, receipt), lifecycle/action dispatch, custody/application/status respectively |
| `CertifiedKeyCandidate`, install command/outcome/errors and `PublicCertifiedKeyCustodyHandle::{id,install,record_install_failure,snapshot,subscribe}` | manual source/custody owner | manual source calls both mutations; Route resolution calls `id`; application owner calls snapshot/subscribe. There is no custody `withdraw`. |
| `PublicCertifiedKeyAeadCodec::{new,seal,open}` and protection context/record/envelopes | `GatewayAdapters::host(kek, clock)` (same `Kek` as `NodeAdapters::host`, boot-probed) | custody only; feature-gated `with_random_fill_for_test` is called only by cross-crate codec integration; future #57 reuses custody `install`, never codec directly |
| `PublicCertifiedKeySnapshot` non-secret accessors plus crate-private TLS capability | custody only | application generation/status use metadata; public TLS runtime alone uses `server_config` |
| `GatewayApplicationGenerationId`, private-constructor `GatewayAppliedGeneration`, retention and non-Clone leases | application owner after all gate inputs | private admission, connection/request runtime and application status only |
| `UnboundGatewayBuilder` (`new`, `into_builder`), `GatewayBuilder`, `GatewayRuntimeDependencies`, `StartedGateway`, `GatewayUpstreamSeal` | `boot_server` (G2-G4) | same composition call chain; the seal is passed only to Host/Sim mTLS; no alternate factory/partial start |
| `GatewayHandle`, task tree/shutdown capabilities/report and ServerHandle field/error extension | `StartedGateway::into_parts` | `ServerHandle::{shutdown,abort_for_test}` only; typed residue is never dropped |
| `GatewayRouteCommands::routes`, `GatewayStatusReads::{application_status,certified_key_status}` | `StartedGateway::into_parts` | Route POST/DELETE and operator-mTLS Gateway GET only; no task/admission/key access |
| Status rows/envelopes/write variants/kinds/point reads and wire bodies | custody/application writers plus LocalStore/Sim adapters | `GatewayStatusReads` and Gateway GET projection only |
| `GatewayLimits::first_slice` and read-only accessors | one call in `boot_server` | builder/runtime, `HostGatewayClientMtls` and BPF capacity assertion; no deserializer/builder/mutator |
| `normalize_route_authority`, `rewrite_request_headers` and `rewrite_response_headers` | public HTTP connection/request path | Route selection and the gateway's head serialiser/framing (ADR-0245); all three are crate-private and their source-local PBT calls the same production functions |
| `GatewayRelay`, `KtlsLeg`/`KtlsReadHalf`/`KtlsWriteHalf`, `AcceptedPublicLeg`, relay error taxonomy | `HostGatewayRelay` (`overdrive-dataplane::gateway_relay`) and `SimGatewayRelay`, built once in `GatewayAdapters::host` / `SimGatewayAdapters` | `GatewayUpstreamConnector` (via the client mTLS adapter), the listener (`accept_public`, taking `Arc<rustls::ServerConfig>`), the exchange driver (head/body methods), the gate task (`probe`) |
| `ExchangeMachine` and its seven accessors | exchange driver | the exchange driver and the Quint conformance driver (all transitions and accessors) |
| `GatewayIdentityView` | `GatewayIdentitySlot` | lifecycle hydration and status only; `GatewayIdentityView` has no `subscribe` |
| `GatewayIdentityLifecycleControl::{ensure_current,current,subscribe,disable_after_drain}` | control-plane adapter over slot/broker plus named Sim adapter over Sim target/real broker/Sim Clock | active application owner in production and seeded application composition in Sim |
| `GatewayIdentityActionComposition`/target and Sim target | Enabled/Disabled AppState composition | every exact action-shim dispatcher and only the Issue arm; no private-material getter |
| `GatewayIdentitySlot` constructor/non-secret reads plus ActionHandle, LifecycleHandle and client-access capabilities | `boot_server` (G1) | lifecycle-control adapter, Gateway SVID action executor and `HostGatewayClientMtls`; slot mutators/access closure are crate-private and only the matching opaque handle/adapter can reach them |
| `GatewaySvidLifecycle::canonical`, factory, Any* variants and the Issue action | control-plane canonical registration | existing reconciler runtime/broker/exhaustive dispatch and action shim only |
| `GatewayClientMtls::authenticate`, opaque `GatewayUpstream`, Host/Sim mTLS constructors | `GatewayClientMtlsFactory` (Host: `HostGatewayClientMtlsFactory` in `GatewayAdapters::host`; Sim: `SimGatewayClientMtlsFactory`) called by `boot_server` G2 | private connector readiness probe and request path; only successful exact-peer adapters hold the unforgeable seal needed to construct the authenticated stream |
| `gateway.upstream.receipt.validated` tracing occurrence | real `GatewayUpstreamConnector` only, after exact receipt validation and selected-identity resolution when applicable | existing tracing subscriber plus append-only `AT-PIG-E2E-1` process-log capture; no production reader, state/status projection or duplicate adapter emitter |
| `Invariant::{GatewayApplicationGenerationLifecycleIsSafe,GatewayApplicationShutdownConverges}` and evaluator functions | DISTILL scaffolds enum/canonical/evaluator/exhaustive-dispatch shapes plus ignored direct bodies; pending variants are absent from `Invariant::ALL` | typed ignored bodies alone call `Harness::only(variant)` until DELIVER activation; after real owners are GREEN, activation key `gateway-application-invariants` makes direct tests GREEN before adding both variants to the default catalogue |

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
| httparse / http | lock 1.10.1 / existing (direct dependencies of `overdrive-gateway`) | MIT/Apache-2.0; message-head tokenizing (the parser `hyper` itself uses) and `HeaderMap`/`Authority` types; hyper, hyper-util and tokio-rustls are not used on the public data path |
| rustls | lock 0.23.39 | Apache-2.0/MIT/ISC; unbuffered handshake API, `kernel::KernelConnection` (rekey), public TLS 1.3 exact-SNI config per connection, AES-GCM suites only |
| libc (`overdrive-dataplane` only) | workspace 0.2 | MIT/Apache-2.0; `setsockopt(TCP_ULP/TLS_TX/TLS_RX)`, `recvmsg` control messages, `splice`, `pipe2`/`F_SETPIPE_SZ` in the one `unsafe` module `gateway_relay::sys` (rustix has no TLS cmsg surface) |
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
    /// Holds the staged gate of one frontend until released. The gate no longer
    /// reaches `GatewayClientMtls::authenticate` (R-65), so the park point is one of
    /// the awaited operations the gate still owns, which DISTILL shapes as Sim
    /// test support (Gateway upstream path, DG-9); the held probe needs no
    /// `Selected` receipt. D2 replaces demand completion with the connect-probe gate.
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
  `#[ignore = "pending DELIVER gateway-application-invariants"]` (the descriptive activation key, per
  `.claude/rules/domain-naming.md`; it is not a roadmap step id).
  Removing that marker while any production owner is still a scaffold produces
  a real RED through `GatewayBuilder`, `GatewayRouteCommands`, or `GatewayHandle`.
- DISTILL authors both evaluator bodies completely. They apply and withdraw
  Route through `GatewayRouteCommands`, retain/release a real public TLS connection,
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

The DELIVER activation step is identified by the descriptive key
`gateway-application-invariants` (“Activate real Gateway
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
`WithdrawRoute` drive `GatewayRouteCommands::routes`, restart re-composes the same
durable stores through `GatewayBuilder::start`, and `BeginShutdown` consumes
the real `GatewayHandle`. Retain/release drives a public TLS connection through
the active listener. Connect-probe hold/release is implemented only by the Sim adapters
(a driven-port fault carrying the real `ServiceFrontend`, parked at an awaited
operation the gate owns, not `GatewayClientMtls::authenticate`; Gateway upstream path, DG-9, R-65); it neither invokes a private owner method nor
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
| DST | `GatewayApplicationGenerationLifecycleIsSafe` owns staged/current/draining, stale completion, withdrawal and restart safety; `GatewayApplicationShutdownConverges` owns joined shutdown convergence. DISTILL scaffolds their typed ignored direct paths but excludes both from `Invariant::ALL`; `gateway-application-invariants` activates them only after real owners are GREEN and direct bodies pass. Registered/unregistered isolation, the typed NoBackend/unavailable outcome (ADR-0242), connector cleanup and key/identity fail-closed complements remain registered with their owning component evidence. |
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

The reconstructed DESIGN is the final ADR set 0220-0246 (system, Route, Domain
and Application decisions, plus the G1 data-path decisions 0244-0246; numbering per [`adr-renumbering.md`](adr-renumbering.md)) plus this document. It is
reconstructed against main 549d78fa6. The user confirmed the D5 "New public surface" list (items 1-13, item 12
reconfirmed) and ruled on P-2, P-9, P-10, B-1, DG-11, DG-12, QC-1..QC-3, P-12, P-13a-d, P-16, P-17, P-18, the upstream-leg shape and Q-1 on
2026-11-02 (R-1..R-19), 2026-11-03 (R-20..R-44) and 2026-11-04 (R-45..R-54, R-52 user-confirmed, R-53 and R-54 user rulings) and 2026-11-05 (R-55, R-56, R-57 and R-58, user rulings) and 2026-11-06 (R-59, R-60 and R-61, user rulings), User decisions. The optional P-1, P-3, P-4, P-6, the P-8 simplification and P-32, P-33, P-34, P-35, P-36 and P-40 may be ruled later (P-37 was rejected and P-38 adopted on 2026-11-04; P-41 was ruled as R-55 on 2026-11-05); none blocks a scaffold. The
contract-shaping items V-GW-1..5 have run on native metal; the
residuals are source-settled or DISTILL native obligations (see Contract-shaping validation items). The 2026-09-13 approval (preserved at git ref `backup/research-ingress-gateway-pre-reconstruct`) covers the decisions carried unchanged (class 1, including item 12 of the D5 list, reconfirmed on main). The cgroup-BPF selection program—not XDP—
selects the host-originated gateway socket; XDP wire ingress remains unchanged.
The Gateway Application never programs, waits for or observes backend state;
ordinary `ServiceMapHydrator` programming covers VM backends (D1). Public 404,
NoBackend 503 and internal 502 are active exact
mappings. Manual Public Certified Key is the only producer; #57 ACME remains
later and additive.

**Rulings carried here (2026-11-02).**
1. Q-1 (R-17): the pipe invariant of ADR-0246 stands until [GH #339](https://github.com/overdrive-sh/overdrive/issues/339) (the kernel 7.3 upgrade) lands; the appliance image decision is that issue's.
2. Chunk framing (`ChunkedOpen`, `request_chunked_end`, `response_chunked_end`) is not modelled and is covered by property tests only (R-19).
3. There is no `relay_ktls` model; the lock exclusion is covered by A-K5 plus the Tier-3 check (obligation 16(b)) (R-19).
4. The relay passes no `SPLICE_F_MORE`, a configuration the spike did not run; Tier-3 obligation 16(j) is carried by S-PIG-105 and S-PIG-111, which run in that configuration (the relay never passes the flag) and log the peer-side record sizes.
   Tiny-event streaming on one connection is not defended by a gateway-originated KeyUpdate (R-13).

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
   bodies, `live_intent_count() == 0` and an empty `live_receipts()` after each, and no
   change in `GatewayApplicationStatusRowV1` or the Gateway Applied Generation;
   the outcome is typed `NoBackend` at the consuming port, bounded by the request
   deadline, never fabricated and never a wait (ADR-0242). Tier 1 (Sim dataplane
   `NoBackend` script) plus Tier 3 (real hook, no outer entry for the `ServiceKey`).
2. D4 edge observation, `healthy` lag (R-2): the `BACKEND_MAP` healthy bit lags readiness by
   about 1 s. The arm ignores `healthy`, so a request in that window dials a not-yet-ready guest and is 502. The
   first request after `healthy` 0 → 1 reaches the backend with no gateway action.
   Tier 3 and Native.
3. A stale map entry yields 502, never a wrong-peer success (D4: entries outlive `job stop`; the nft
   output chain resets the dial for the unregistered tuple). Tier 3 and Native. The map lingering
   itself is not a gateway fix (P-4).
4. Boot gate: actual-frontend `NoBackend` and `Selected` are bind-valid (R-65); sentinel probe failure keeps
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
9. Gateway SVID lifecycle (issue, audit-before-hold, near-expiry reissue, epoch-stale no-op, synchronous clear
   on disable, restart reissue). Tier 1.
10. Custody replacement preserves last usable key; Route/custody/application watch recovery
    (subscribe-before-list, relist on lag). Tier 1.
11. Receipt/intent correlation: no residue after cleanup; restart destroys receipts; BackendId identity
    immutable and readable only after the outer swap commits. Tier 1 plus Tier 3.
12. Quint conformance (`.claude/rules/testing.md` § "Quint conformance"): specification directory `specs/quint/public-ingress-gateway/`
    (conformance modules `gateway_identity`, `gateway_application`, `selection_receipt` and `relay_exchange`; there is no kTLS-lock model); one quint-connect driver per conformance module in the owning
    crate's `src/` behind `#[cfg(all(test, feature = "integration-tests"))]` (gateway_identity: `overdrive-control-plane`, `src/gateway_identity_lifecycle/quint_gateway_identity.rs`, reaching the
    `pub(crate)` dispatch functions in-crate; gateway_application: `overdrive-gateway`, `src/application/quint_gateway_application.rs`, with test-local doubles of the gateway-crate driven ports because `overdrive-sim` cannot satisfy crate-local traits in the unit-test build (DG-7); selection_receipt:
    `overdrive-dataplane`, `src/quint_selection_receipt.rs`, owner = the real `EbpfDataplane` on native metal, bound runs
    `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` over the coarsened `receipt_conf` main,
    see Formal protocol model; the remaining `selection_receipt` runs and hazards are model-checked and covered at Tier 3 as stated
    there; relay_exchange: `overdrive-gateway`, `src/runtime/http1/quint_exchange.rs`, owner = the pure `ExchangeMachine`, seven
    bound runs, see obligation 17); every driver has `#[quint_test]` for each named
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

16. **G1 relay (Tier 3 on native metal unless stated; oracles are bytes at a real rustls peer, `/proc/net/tls_stat` deltas
    and the typed outcomes).** (a) A spliced body through an `Content-Length` and a chunked exchange arrives byte-exact, with
    the next request head untouched (keep-alive over two exchanges). (b) A peer KeyUpdate (requesting and not requesting) inside
    a head, inside a chunk-size line, at a message boundary and inside a body: octets exact, `TlsRxRekeyOk`/`TlsTxRekeyOk` advance,
    `TlsDecryptError == 0`, nothing forwarded; a negative control without the re-arm stays `EKEYEXPIRED`; a KeyUpdate request while a
    large splice-out is in flight (the lock, A-K5). (c) A NewSessionTicket
    before the first upstream response and a `close_notify` mid-body, mid-head and after the response (clean end only when the
    framing says so). (d) Backpressure: a stalled public reader freezes the upstream sender within the socket buffers and
    resident memory stays flat; a stalled body hits `body_no_progress_deadline`; a trickling `Content-Length` response is delivered
    to the client without waiting for the next octets (each splice-out call ends its TLS record; no `SPLICE_F_MORE`). (e) 128 concurrent exchanges: descriptor count,
    memory, no leaked pipe or socket after drain. (f) The relay probe is refused by each substrate lie at Tier 1 (Sim); on the real kernel the
    honest pass is asserted. There is no real-kernel ULP-absent stimulus (R-23): on the appliance the kernel configuration is the
    image's, so the lie does not occur on a production node, and a test that unloads `tls` or drops a capability to produce it
    would test the stimulus, not the gate. (g) The pipe floor: `PipeCapacity` and the typestate laws (unit). (h) Early response with a
    lingering close (R-14): the client receives the 413 or 401, not a reset, for an early origin response and for each
    pre-exchange rejection (413 on declared length, 400, 404, 421, 431, 505), with a body sent after the rejection; the discard
    ends at the peer's close_notify, at `max_request_body_bytes`, and at the deadline; a drain during a discard ends it at the drain
    deadline. Also: close-delimited response (R-12), interim `100`, `Expect` (R-11),
    oversize head, framing rejections, request-body overflow with and without a sent response head, upstream closes before the head and
    mid-body, pipe-creation failure, and the typed 502/503/413/431/408/400/421/505 mapping, each with its pre-/post-head projection.
    (i) Pipe lifetime: a keep-alive connection waiting for its next request holds no pipe (descriptor count), across a chunked body
    and a `Content-Length` body. (j) The relay runs without `SPLICE_F_MORE`, a configuration the spike did not run (findings (c)
    used the flag while `remaining > 0`): the exact-length, chunked, trickling-body and keep-alive cases of (a) and (d) are
    re-run in that configuration with byte-exact octets and the next head untouched, and the record sizes at the peer are
    recorded; the kernel cites of Contract item 7 are re-derived with `git show v7.0:<path>` against `vendors/linux`.
17. **G1 pure and seeded.** Tier 1: framing decision, head validator, chunk-line parser, `rewrite_*` and `normalize_route_authority`
    are pure PBT; `ExchangeMachine` transitions are PBT-driven. Quint conformance: the `relay_exchange` driver in
    `overdrive-gateway` replays the seven `relay_exchange` named runs with `#[quint_test]` and randomized traces with `#[quint_run]`,
    over the single action-to-method table and the accessor table of "Pinned observable projection"; there is no kTLS-lock model (its
    Tier-3 oracle is obligation 16(b)). Seeded `overdrive-sim` runs drive the exchange driver over `SimGatewayRelay`
    with scripted peer events (KeyUpdate, NewSessionTicket, close_notify, alert, reset), upstream close
    before the response head, and drain/force-abort at the drain deadline, asserting no octet past a message boundary, no reuse
    after drain, and zero residue (leases, pipes, legs) after every outcome.
18. **Host/Sim equivalence** for `GatewayRelay` (`gateway_relay_equivalence.rs`) over the faults the Host can also produce (peer
    events, deadlines, short counts, backpressure). Presenting a lying substrate (ULP, cipher, splice exactness, pipe floor) belongs
    only to the probe gold test; the shape of that test support is DISTILL's.

Fault stimuli are injected through driven ports only (Sim dataplane/identity/mTLS/relay, probe hold/release); the
production path authors the resulting observations. Each scenario records contract, owner path, stimulus,
oracle and tier, per `.claude/rules/testing.md` § "DISTILL — prove composed system behavior".

Class-5 blockers for DISTILL scaffolding: none new. The only missing primitives
are the D5 "New public surface" items, which are pinned here and confirmed (R-18). Every quint-connect driver lives in its owning crate's `src/`
(none spans owners through the exported server handler); only `AT-PIG-E2E-1`
is in `tests/conformance`. Quint conformance runs under `integration-tests` via
the repository's quint runner (`cargo xtask lima run --` where a Lima VM exists,
otherwise `cargo xtask metal run --`); real-kernel and VM evidence runs via
`cargo xtask metal run --`.

### DISTILL content made stale by G1 (for the acceptance designer to reconcile; not edited here)

- Sections "DISTILL / Reconciliation and warnings", "Scenario list and tags", "Cross-boundary scenario obligations", "Adapter
  coverage", "Scaffolds", "Test placement", "Driving-adapter coverage", "Completeness audit", "Pre-DELIVER RED classification":
  every scenario or scaffold that names the hyper-based public runtime, `GatewayByteStream`, the `Pin<Box<dyn ..>>` form of
  `GatewayUpstream`, the old `SimGatewayClientMtls::new(seal, clock)` or `HostGatewayClientMtlsFactory` unit struct, and the
  "Public HTTP runtime (hyper)" adapter-coverage row (S-PIG-12..17, 39, 40, 70).
- Scenarios S-PIG-12..17 (HTTP matrix), the limits scenarios and the drain scenarios need the framing, exchange-boundary,
  control-record and pipe/backpressure cases of obligations 16 and 17; scenario counts and the owning-crate column change
  (`overdrive-gateway` runtime, `overdrive-dataplane::gateway_relay`, `overdrive-sim` relay).
- Scaffold file ownership (obligation 15) gains `overdrive-dataplane/src/gateway_relay/`, `overdrive-gateway/src/runtime/http1/`
  and the Sim relay; the Quint driver table gains `relay_exchange` (QC-4) in `overdrive-gateway` (there is no kTLS-lock model).
- The "drive a public TLS connection through the active listener" step of the seeded Gateway Application invariants (obligation
  13) runs over `SimGatewayRelay` in Tier 1; the real kTLS path is Tier 3.
- `distill/code-reconciliation-plan.md`: every batch that touches the public runtime, the gateway-client mTLS adapter, the
  `GatewayAdapters`/`GatewayClientMtlsDeps`/`GatewayRuntimeDependencies` signatures, `GatewayLimits` documentation, the
  Cargo dependency list (`hyper`, `hyper-util`, `tokio-rustls` out; `httparse`, `http` in) and the
  fixture list; the hyper-specific main-API pins.
- E15 and `AT-PIG-E2E-1` keep their composition and oracles; their evidence additionally needs a kTLS-armed leg per request
  (`ss -K` / `tls_stat`) and the relay probe event.

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

**DISTILL status: specification reconciled with the revised G1 DESIGN (2026-11-02: kernel relay data path, ADR-0244/0245/0246, rewritten ADR-0235/0239, `specs/quint/public-ingress-gateway/relay_exchange.qnt`); code reconciliation authored (batches B1–B5; see "Authoring status by batch").** This block specifies the acceptance suite for the reconstructed DESIGN (final ADR set 0220–0254 and this document's DESIGN part, including "Kernel relay data path and driven ports (G1)"). The acceptance bodies, production scaffolds, shell scripts and the example are authored in the working tree (batch record: "Authoring status by batch"); "Pre-DELIVER RED classification" states what was executed and what was not. The E15 expectation (`verification/expectations/E15-public-ingress-operator-journey/README.md`, its `execution-substrate` file, the INDEX row (status `pending`) and `runner.sh`) is authored; the runner has never been run and no evidence exists. The full working plan with per-scenario main-API pins, batch file lists and cross-batch compile order is [`distill/code-reconciliation-plan.md`](distill/code-reconciliation-plan.md) (rewritten for G1; not a delivery artifact). The user ruled on 2026-11-03 (R-20..R-44, DESIGN "User rulings of 2026-11-03") on the DESIGN's former proposals P-20..P-31: **R-20** (the client-mTLS factory crate cycle) unblocks the scaffolds and tests listed under "Authoring status by batch" as unblocked by R-20. The user ruled on 2026-11-02 (R-1..R-19, DESIGN "User decisions"): the D5 shape list (items 1–13, item 12 reconfirmed), P-2, P-9, P-10, B-1, DG-11, DG-12, QC-1..QC-3 and the G1 proposals (below) are decided and are not open. Item 13 also carries the **confirmed list** (R-18) that closes DG-1..DG-5 and DG-8..DG-10 (`KtlsLeg::from_armed`, `backend_mut`, `ThroughBytes::new`, `HostGatewayRelay::new`, `SimGatewayRelay::new`, `SimGatewayClientMtlsFactory::new`, `SimGatewayAdapters`, `ExchangeMachine::new`, `BodyFraming`, `FramingViolation`, `PreparedUpstream::new`, `GatewayAdmission::new`, `GatewayAdmission::budget`, `GatewayBuilder::start_with`); DISTILL scaffolds those shapes, including `SimGatewayClientMtlsFactory::new` and `SimGatewayAdapters`, which R-20 unblocks. Item 13 also lists the **DG-7 placement ruling** (the QC-2 driver lives in `overdrive-gateway` with driver-local doubles of the four gateway-crate ports; confirmed with item 13). **DESIGN gaps DG-8, DG-9 and DG-10 are closed by the architect** (`PreparedUpstream::new(socket, intent)`; the probe parks at an awaited operation the gate owns, identified by the staged gate token (R-65); the admission is injected through crate-private `GatewayAdmission::new`, `budget()` and `GatewayBuilder::start_with`); DG-8 and DG-10 add item 13 entries, confirmed (R-18); DG-9 adds none. **DESIGN gap DG-11 is ruled 2026-11-02 (R-5)**: the connector reserves the cleanup-ledger slot before `open_upstream` and binds the returned intent to it after (DESIGN, "Gateway upstream path into leg-C", "Ledger order"); S-PIG-37 asserts the refusal before `open_upstream` (no socket prepared, no registration) and S-PIG-53's retained record is created at the pre-reservation; the scenario list carries no pending tag for it. **DESIGN gap DG-12 is ruled 2026-11-02 (R-6)**: each gateway test composition binds a distinct `127/8` address (`GatewayConfig::new` takes the address); there is no port seam, and a gateway binary joins `host-kernel-shared` only for node-global kernel state it mutates (DESIGN, "Test compositions (R-6, DG-12)"); the DG-12 paragraph below records the analysis the ruling closes. **Blocker B-1 is ruled 2026-11-02 (R-4)**: `disable_after_drain` is a synchronous clear, `gateway_identity.qnt` models it, and the design has no drop action; the QC-1 runs `restart_reissue` and `disable_races_inflight_issue` expect `held` empty right after `disable`, no `settle_ok`/`settle_timeout` step exists, and all five QC-1 runs and the `#[quint_run]` carry the single ignore reason `pending DELIVER quint-conformance-gateway_identity`.

Reconciliation (hard gate, re-run for G1): DISCUSS, DEVOPS and separate DESIGN `wave-decisions.md` files do not exist (this feature uses one narrative `feature-delta.md`); the only wave-decisions file is `spike/wave-decisions.md` (G1 spike promotions (a)–(e)), which agrees with the DESIGN sections. **Reconciliation passed — 0 contradictions** among the DESIGN sections (the G1 section, the Formal protocol model tables, the ADRs 0244/0245/0246/0239 and the specs name the same ports, accessors and runs). Observation-surface provenance (batch B1 check): the two exact `ObservationStore` point reads (`public_certified_key_status_row`, `gateway_application_status_row`) and the new `ObservationWrite`/`ObservationRow`/`ObservationRowKind` variants `PublicCertifiedKeyStatus` and `GatewayApplicationStatus` were part of the DESIGN the user approved on 2026-09-13 (`git show backup/research-ingress-gateway-pre-reconstruct:docs/feature/public-ingress-gateway/feature-delta.md`: status banner lines 3–12 and the closed-observation-surface block at lines 3618–3640; identical signatures in this document at lines 3844–3858). They are carried as previously approved and are not on the confirmation list; only item 12 gates B1's watch files. Warnings, not blockers: no story-ID traceability; the project ATDD policy (`docs/architecture/atdd-infrastructure-policy.md`) now carries the gateway rows appended by batch B5 (Driving 2, Driven internal 6, Driven external 2); there is no Lima instance in the authoring environment, so Linux-side compilation and runs use `cargo xtask metal run --`.

**Rulings of 2026-11-02 on the former blocking proposals.** **P-2** (R-3): the BackendId allocator is unchanged (the read-only accessor `existing_id` is contract, R-24). **P-9** (R-2): the registered arm ignores `BACKEND_MAP.healthy`, so the ~1 s startup window is 502 and the in-window oracle is authorable (the in-window oracle of S-PIG-56 is 502 and is authored). **P-10** (R-1): the selection arm is a separate `cgroup/connect4` program beside `cgroup_connect4_service`.

**G1 proposals P-12, P-13a–d, P-17, P-18 are ruled 2026-11-02 as authored (R-8..R-14); P-16 is not adopted (R-16) and the key-update/transmit-lock model is dropped (R-19).** The option each scenario was authored for is the ruled contract, so the scenario list carries no proposal-pending tag. The rows below name what each ruling covers; the last column records the alternative that was not taken and is not open.

| Ruled item | Option authored (design body) | Scenarios and drivers governed by the ruling | Alternative not taken |
|---|---|---|---|
| P-12 | Both legs offer only AES-128-GCM and AES-256-GCM | 118 (suite matrix and the server-config suite list); the suite parameter of 105 and 106 | ChaCha20 offered: the ChaCha-only case of 118 inverts to arm-and-relay and 106 gains a ChaCha20 rekey case; nothing else changes |
| P-13a | Strict ambiguous-framing rejection set (request 400/505, response 502) | 77, 78, 79, 80, 81 | Narrower set: delete the corresponding rows of 77/78/79/80/81; widening has no authored alternative (forwarding ambiguity is the rejected smuggling defect) |
| P-13b | Full-duplex exchange (response may finish before the request body) | 87 (overlap step), 93; QC-4 runs `exchange_cl_keepalive`, `early_response_closes` | Half-duplex: 87 loses the overlap step, 93 reduces to "request fully relayed, then response", the two QC-4 runs change in DESIGN's spec first |
| P-13c | Interim `100` forwarded, other interim `1xx` dropped, `Expect` forwarded unchanged | 83, 119 (interim heads count against the cumulative head limit), 120 (the two dropped `103` heads) | Reject all interim `1xx`: 83 inverts to 502; 119 loses its interim-head clause; 120 loses its two `103` cases (the silent-origin, 118 s and final-head cases stay) |
| P-13d | Close-delimited and HTTP/1.0 explicit-length responses supported | 76 (close-delimited arm), 78 (HTTP/1.0 response row), 84 (`UntilClose` arm), 91, 104 (`relay_to_end` case), 117; QC-4 run `close_delimited_clean_end` | Declined: rows 91 and 117 and the marked clauses are deleted, 78 gains "close-delimited response is a 502", and `relay_to_end` leaves the scaffold |
| P-16 (not adopted, R-16) | A read half's pipe is released when the half starts its next head read (the design body) | 110 | Hold until close (not adopted) |
| P-17 | The gateway never originates a KeyUpdate | the "no gateway-originated KeyUpdate" clause of 106 and 109 | Origination before 2^23 records: add a record-counter scenario (one new row) |
| P-18 | Lingering close with `discard_remaining` | 93, 101 (discard during drain), 115, 121 (its early-answer clause: no 408 once a response octet was written, the lingering close governs), the discard case of 104; `discard_remaining` on the port scaffold | (a) bounded discard or (b) `close_write` only: 93/115 invert to reset-tolerant and 121's early-answer clause is restated to match; the discard case of 104 and the port method are removed |
| Key-update/transmit-lock model (dropped, R-19) | No model; chunk framing is covered by property tests and the transmit lock by Tier 3 only | none (107 stays under A-K5) | Model kept (not taken) |

**DG-11 (ruled 2026-11-02, R-5; the text below is the analysis the ruling closes).** The DESIGN's cleanup-ledger paragraph (Gateway Application, "`GatewayBuilder::start` creates exactly one `Arc<GatewayAdmissionBudget>`") says the connector `reserve(intent)`s one ledger slot after socket-cookie acquisition and before BPF registration, and that a full ledger returns `GatewayCleanupLedgerFull` so the connector closes without registering. ADR-0243 makes `open_upstream` perform cookie acquisition and `GATEWAY_CONNECT_INTENT_MAP` registration atomically (the port method returns a registered intent), so the connector has no point between the two at which to reserve, and a full ledger cannot refuse registration. The ruled fix: reserve a ledger slot before `open_upstream` and bind the intent to it afterwards. The ledger scenarios are: S-PIG-37 (ledger 0/1/128, the 129th refused before registration) and S-PIG-53 (a failed cleanup is retained in the ledger and retried). Cross-boundary rows that assert only `cleanup` call counts (28-30, 63, 98) do not depend on where the slot is reserved and are unaffected. Under the ruled fix: S-PIG-37 asserts the refusal before `open_upstream` (no socket prepared, no registration) instead of "before registration", and S-PIG-53's retained record is created at the pre-reservation instead of after the cookie; no id is renumbered.

**DG-12 (ruled 2026-11-02, R-6).** Finding: `GatewayConfig::new` takes the bind address and fixes the port to 443 (`GatewayConfig::bind()` is always port 443); `GatewayListenerControl::bind` is the one listener path and the gate task's last step (G4) is the Gateway Application promotion bind; `SimGatewayRelay` substitutes the TLS, kTLS and splice adapter, not the listener. Every composition that reaches promotion therefore binds a real IPv4 address on port 443, which needs root (`cargo xtask metal run --` runs as root; the Sim-only binaries need no KVM) and collides with any other process holding the same address. Ruling: option (c), each gateway test composition binds a distinct `127/8` address (`GatewayConfig::new` takes the address); a serial run and a listener-port seam are not taken. A gateway binary joins `host-kernel-shared` only for node-global kernel state it mutates; the `overdrive-gateway` library and integration binaries and the `overdrive-sim` acceptance binary mutate none and are not assigned (Test placement). Each test file documents its address table in its header (second octets: gateway integration file 41..=62, in-crate drivers 63..=67, relay exchange file 68..=96, boot/shutdown file 97, conformance 100..=119, `overdrive-sim` 120..=123).

Language mode Rust; deliverable type application; density lean, Tier-1 `[REF]` sections only. Repository rules override generic nWave examples: no `.feature` files, no Python steps; Gherkin-style text below is specification only (`.claude/rules/testing.md`). Acceptance bodies are complete and executable; future scenarios carry `#[ignore = "pending DELIVER <descriptive-name>"]` naming a domain-named piece of DELIVER work (never a roadmap step id); no placeholder panics. The two expected-GREEN preservation rows (S-PIG-41, S-PIG-59) carry **no** `#[ignore]`: they are un-ignored from authoring (an ignored test cannot be expected-GREEN); S-PIG-41 is compiled under `integration-tests` and executes only on the native metal host.

Retired premises (explicit, not carried): the branch DISTILL's gateway-frontend-demand scenarios and apply-twice gap (D2, ADR-0242), the demand-gated Service-map TEACH (D1), the shell Tier-3 runner under `tests/tier3/public-ingress-gateway/` that spawned the built binary (tests never spawn the `overdrive` binary), the Lima GitHub workflow lane, the `[exec]` workload example, the branch E14 expectation id (E14 on main is `E14-vm-service-post-greenfield-cut`; this feature's expectation is E15), and (G1) every artifact that assumed a userspace `hyper` stream: the "Public HTTP runtime (hyper)" coverage row, `GatewayByteStream`, the `Pin<Box<dyn ..>>` form of `GatewayUpstream`, the `SimGatewayClientMtls::new(seal, clock)` form and the unit-struct `HostGatewayClientMtlsFactory`.

## Wave: DISTILL / [REF] G1 reconciliation

Resolution of every item the DESIGN listed in "DISTILL content made stale by G1". Scenario ids persist (stable identifiers); nothing is renumbered; S-PIG-11 stays the only retired id. New ids are S-PIG-76..123 (S-PIG-119..123 were appended by the review remediation: response-side limits, the response-head deadline, the request-body stall, the public half-close and the public handshake refusals).

| Stale item | Resolution |
|---|---|
| "Reconciliation and warnings": ADR range 0220–0243, no G1 proposals | Rewritten: ADR set 0220–0246, hard gate re-run (0 contradictions), ruled-P-item table (P-12, P-13a–d, P-17, P-18 ruled as authored; P-16 not adopted; `relay_ktls` dropped; all 2026-11-02) |
| Scenario list, S-PIG-12..17 "HTTP matrix" | The six rows stay pure (`routing`, `proxy_headers`, `limits` do not depend on any HTTP engine) and keep their ids and bodies. What G1 adds is new rows, not edits: framing, line validation, chunk lines, request-head boundaries, interim heads, exchange laws, deadline composition and the pipe floor (S-PIG-76..86), then the exchange behaviour over the Sim relay (S-PIG-87..101), seeded (S-PIG-102..103) and over the real kernel (S-PIG-104..118) |
| Scenario list, limits scenarios | S-PIG-17 unchanged (values are `GatewayLimits::first_slice`); the head, header-count and body-ceiling boundary cases at the new sites are S-PIG-82 and 94, the deadline boundaries S-PIG-85 and 96 |
| Scenario list, drain scenarios | S-PIG-26/60 unchanged; the exchange-level drain, discard-during-drain and force-abort are S-PIG-101 and S-PIG-103 |
| S-PIG-39, 40, 70 (public status/404/502 over the old runtime) | Contract unchanged. Oracle gains "public leg and upstream leg are kernel-TLS armed" (`ss -K`, `tls_stat`) for S-PIG-70. S-PIG-39 and 40 are listener-level tests, not native-metal ones: the route-miss 404 is a case of `an_early_or_gateway_authored_response_ends_with_a_half_close_and_a_bounded_discard` and the single 502 is `an_upstream_failure_is_one_502_before_the_head_and_an_unclean_close_after_it`, both in `public_ingress_gateway_relay.rs`; the one-attempt, no-retry rule is asserted by the connector's own tests (`overdrive-gateway/src/connector.rs`) and the one-upstream-leg-per-request rule by S-PIG-97. The 400/421/431/505 mappings are covered at the pure-function level (S-PIG-82 and the R-32 `http1` functions; R-42 drops the extra listener-level tests); the typed 408/413/502/503 projections with pre-/post-head behaviour are S-PIG-92..96 (Sim) and S-PIG-115 (real kernel); S-PIG-70 keeps its existing real-path cases |
| "drive a public TLS connection through the active listener" in the seeded invariants (obligation 13; S-PIG-31/32, C4a proof (i)) | In Tier 1 the connection is a client socket through `SimGatewayRelay` (the Sim terminates the modelled handshake); real TLS is only T3/N. Row text of S-PIG-02, 31, 32, 54 and the C4a paragraph updated |
| S-PIG-38, 43, 52 (real rustls peer, built through `HostGatewayClientMtlsFactory`) | The factory is `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)`; the negative matrix stays an in-process real-rustls test and additionally asserts a failed equality arms nothing; the positive arm (peer observes the gateway SVID, leg armed) is S-PIG-64; the arm refusal after equality is S-PIG-98 |
| S-PIG-57, 60, 74 | Gate G-1 gains `GatewayRelay::probe` (reason `gateway.relay_probe`); the relay probe gold test is S-PIG-99 and the relay gate (boundary 7, G3 order) is S-PIG-100; the "no relay probe runs when the gateway is disabled" assertion (boundary 6) lives in S-PIG-100's disabled case, not in S-PIG-59: it needs a relay instance to count calls on, which is a scaffold that fails with SCAFFOLD text, so it cannot sit in the expected-GREEN row |
| "Public HTTP runtime (hyper)" adapter-coverage row | Replaced by rows for `HostGatewayRelay`, `SimGatewayRelay`, the exchange driver and the head/framing pure modules (Adapter coverage) |
| Scaffolds naming `GatewayByteStream` / `Pin<Box<dyn ..>>` `GatewayUpstream` | Removed; scaffolds use `GatewayUpstream { leg: KtlsLeg }`, `GatewayUpstreamSeal::seal_authenticated(&self, leg: KtlsLeg)` and the `GatewayRelay` port family exactly as pinned |
| Scaffolds naming `SimGatewayClientMtls::new(seal, clock)` / unit-struct `HostGatewayClientMtlsFactory` | `SimGatewayClientMtls::new(seal, Arc<SimGatewayRelay>, clock)`, `HostGatewayClientMtls::new(identity, seal, limits, Arc<HostGatewayRelay>)`, `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)`, `GatewayAdapters { certified_key_codec, client_mtls, relay }` |
| Scaffold file ownership (obligation 15) | `overdrive-dataplane/src/mtls/gateway_identity.rs` (slot, handles, `GatewayClientIdentityAccess`) and `mtls/gateway_client.rs` were already separated; added: `overdrive-dataplane/src/gateway_relay/` (host relay, one `unsafe` module), `overdrive-gateway/src/runtime/http1/` and `runtime/exchange.rs`, `overdrive-sim` relay adapter |
| Quint driver table | QC-4 `relay_exchange` specified in full (spec, runs, invariants, owner, action→call, State→accessor, placement, ignore reason); there is no key-update/transmit-lock model (R-19); per-driver checklist filled for QC-1..4 (Scaffolds). Second pass (DESIGN "Conformance action maps for the three older mains"): every action of QC-2 and QC-3 has a pinned map; the user accepted the owner-autonomous handling of QC-2 `stage`/`retire` and the removal of `idle` from the three conformance mains (R-7, 2026-11-02), and B-1 is ruled (R-4: synchronous clear; no `settle_ok`/`settle_timeout`), so item 3 is no longer conditional |
| DESIGN gaps DG-1..DG-12 (second pass and DG-7..DG-12) | DG-1 closed ("Cross-crate construction of the opaque leg values": `KtlsLeg::from_armed`, `backend_mut`, `ThroughBytes::new`); DG-2 closed (`HostGatewayRelay::new(clock)`, `SimGatewayRelay::new(clock)`, `SimGatewayClientMtlsFactory::new(relay, clock)`, `SimGatewayAdapters`); DG-3 closed (`ExchangeMachine::new()`, `BodyFraming` four variants, `FramingViolation` six variants with the 502 status rule); DG-5 closed (the DESIGN now pins `pending DELIVER gateway-application-invariants`); DG-6 closed for QC-2 and QC-3 and for the QC-1 actions `restart`/`idle`; QC-1 `settle_ok`/`settle_timeout` do not exist (B-1 ruled, R-4); **DG-7 closed** (placement ruling: the QC-2 driver is `overdrive-gateway/src/application/quint_gateway_application.rs` with driver-local private doubles of the four gateway-crate ports, because `overdrive-sim`'s impls of crate-local traits cannot satisfy them in the library's `cfg(test)` build; it adds no public item and is confirmed on the item 13 list); **DG-8 closed** (`PreparedUpstream::new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent)`, public, registers nothing, no cleanup guard); **DG-9 closed** (a gate parks at an awaited operation it owns, `GatewayConnectDataplane::probe`, `GatewayRelay::probe` or the identity `ensure_current` wait, entries identified by the staged gate token, not by arrival order or a backend identity; it needs no `Selected` receipt; R-65); **DG-10 closed** (crate-private `GatewayAdmission::new(budget)`, `budget()`, `GatewayBuilder::start_with(deps, admission)`; the driver goes through `UnboundGatewayBuilder::new`/`into_builder`/`start_with` and takes `GatewayRouteCommands` and `GatewayStatusReads` from `into_parts()`); **DG-11 ruled** (R-5: reserve the slot before `open_upstream`, bind after; paragraph above); **DG-12 ruled** (R-6: distinct `127/8` address per composition; paragraph above). The scaffolds that were described, not scaffolded, are now specified (Scaffolds) |
| Test placement | New rows for the pure http1 modules, the Sim-driven relay integration file, the seeded relay file, the host relay Tier-3 files and the equivalence file (Test placement) |
| Completeness audit; Pre-DELIVER RED classification | Rewritten for the G1 scenario set |
| `distill/code-reconciliation-plan.md` | Rewritten: relay port family, host and Sim relay, http1 pure modules, `ExchangeMachine`, exchange driver, kTLS arming, probe; dependency list (`httparse`, `http` in; `hyper`, `hyper-util`, `tokio-rustls` never added to `overdrive-gateway`); fixture list; main-API pins |
| E15 and `AT-PIG-E2E-1` | Composition and oracles kept; evidence additionally needs a kTLS-armed leg per request (`ss -K` / `tls_stat`) and the relay probe event; E15 README authored |
| Counts | 75 ids / 74 live → 122 ids / 120 live (S-PIG-116 deleted, S-PIG-75 dropped; Scenario list; 118 / 117 before the review remediation appended S-PIG-119..123) |

## Wave: DISTILL / [REF] Authoring status by batch

Recorded from the working tree: test names and ignore reasons were checked against `crates/` and `tests/conformance` (grep of `#[ignore = "pending DELIVER ..."]` and the `S-PIG-nn` citations), not copied from the batch notes. Legend: **ignored** = a complete executable body under `#[ignore = "pending DELIVER <key>"]` (it fails at its first production scaffold call when forced); **active** = no `#[ignore]`; **unblocked by R-20** = the DESIGN pin of `GatewayClientMtlsFactory`/`GatewayClientMtlsDeps` named a dataplane type in a port (a crate cycle) when the batch first ran; R-20 (2026-11-03) moved `GatewayClientIdentityAccess` into `overdrive-gateway::ports`, and the rows of that column are now authored, ignored under the pending key named in the scenario table; **not authored** = no body, reason given. "Changes under R-20..R-44" lists what the rulings add, drop or reshape. The compile order was B1, B2a, B3b, B2b, B3a, B4a, B4b, B5; every batch compiles under `integration-tests` and `cargo xtask conformance-lint` was recorded clean.

| Batch | Authored, ignored (activating key) | Authored, active | Unblocked by R-20, authored and ignored | Not authored |
|---|---|---|---|---|
| B1 core, host, stores | S-PIG-03–10, 44, 55 (`overdrive-core`: `public_ingress.rs` 11 tests, `gateway_identity.rs` 3; `route-value-types`, `gateway-identity-epoch`); S-PIG-47 (`overdrive-host`, 2; `certified-key-custody`); S-PIG-48 (`overdrive-store-local/tests/local_store_edges.rs`, 1; `lag-aware-intent-subscription`) | the four golden-bytes decode fixtures of the new rkyv envelopes (`tests/schema_evolution/{public_route_set,public_certified_key,public_certified_key_status_row,gateway_application_status_row}.rs`) | none | none |
| B2a gateway crate: ports, application, connector, pure runtime | S-PIG-12–13 (`routing.rs`, 4), 14–16 (`proxy_headers.rs`, 6), 17 (`limits.rs`, 1), 45 (1), 49 (`frontend.rs`, 2), 73 (1), 28–30, 37, 53 (`connector.rs`) and 37 (`ledger_acceptance.rs`, 5); the gateway integration binary `public_ingress_gateway.rs` (22 tests: S-PIG-21–27, 33, 46, 50, 51, 54 (custody replay across a restart and an identical Route re-apply), 66, 68); the connector arm of S-PIG-69 (`public-request-end-to-end`); QC-2 (3 `#[quint_test]` + 1 `#[quint_run]`, `quint-conformance-gateway_application`) | none | scaffolded: `GatewayClientMtlsFactory`, `GatewayClientMtlsDeps`, `GatewayClientIdentitySource`, `GatewayClientIdentityAccess` (R-20), `GatewayConnectCleanup` with `live_receipts` and the narrowed `GatewayShutdownCapabilities` (R-29, R-33), `GatewayRouteCommands` and `GatewayStatusReads` in place of `GatewayControl` (R-25), the deadline-free `disable_after_drain` and unit `Timeout` (R-30), `discard_remaining` returning `()` (R-22); the source-local `http1` bodies of S-PIG-76, 77–83 (U half) and 85 are authored and ignored against the signatures pinned by R-32 | none (S-PIG-54 has no Tier-1 connector arm; the S-PIG-02 I arm runs twice and asserts the empty second delta) |
| B3b kernel relay (dataplane + `SimGatewayRelay`) | S-PIG-104 (`gateway_relay_equivalence.rs`, 18 ignored tests expanded by `equivalence_cases!`); `gateway_relay.rs` (15 tests): S-PIG-105, 106, 107, 108, 109, 110, 111 (2), 112, 114, 115 (2), 117, 118, 123 (`kernel-relay-host-adapter`, `relay-earned-trust-probe`, `relay-lingering-close`) | none | S-PIG-86 (authored in `overdrive-dataplane/src/gateway_relay/mod.rs`: the `PIPE_FLOOR` constant test is active and expected GREEN; the `PipeCapacity::new` and `splice_in_len` tests are ignored `pending DELIVER kernel-relay-host-adapter`) | none; S-PIG-114 is reduced to the honest real-kernel pass (R-23: its `unshare`/`rmmod` child and restoring guard are deleted); S-PIG-116 and its two tests are deleted (SUB4); `gateway_relay_equivalence.rs` compares the returned `Result<(), _>` class and the peer-observable leg state after `discard_remaining`, not a count (R-22) |
| B2b relay exchange consumers | `public_ingress_gateway_relay.rs` (29 tests, second octets 68..=96): S-PIG-77–82 (at the public listener), 83 (I half), 87–99, 101, 118 (server configuration: exactly the two AES-GCM suites), 119, 120, 121, 122; S-PIG-84 (`http1/exchange.rs`, 10 source-local tests); QC-4 (7 `#[quint_test]` + 1 `#[quint_run]`, `quint-conformance-relay_exchange`) | none | authored and ignored: S-PIG-76 and S-PIG-85 (R-32), the U halves of S-PIG-77–83 (R-32), S-PIG-120's 118 s arm and S-PIG-121's 408 and 120 s arms (R-37: the response-head budget starts when the upstream request is completely written, so a stalled request body reaches its own deadlines) | the four listener-level tests of S-PIG-79–82 are deleted (R-42: the source-local bodies replace them; the S-PIG-77 and S-PIG-78 listener arms stay); S-PIG-93's EOF/alert/reset-not-lingered arm for gateway-authored rejections (not constructible: they are sent before any further read; S-PIG-122 covers the mid-request-body arms); the connector-level carry of `Arm(kind)` in S-PIG-98 |
| B3a connect hook, mTLS identity, maps, Sim connect adapters | `public_ingress_gateway_hook.rs`: S-PIG-56 (T3 half), 57 (T3 half), 63, 71 (instance half), 72; S-PIG-67 (`gateway_connect_dataplane_equivalence.rs`, 1); QC-3 (3 + 1, `quint-conformance-selection_receipt`) | S-PIG-41 (hook file, no ignore; expected GREEN, recorded passing on metal) | authored and ignored `pending DELIVER exact-peer-client-mtls`: S-PIG-38, 43, 52 and S-PIG-64 (`public_ingress_gateway_mtls.rs` is in `overdrive-control-plane/tests/integration`, not in the dataplane crate); the `DataplaneError::{BackendIdentityConflict, ServiceApplyRollback}` variants, `ServiceApplyStage` and `ServiceApplyRollbackFailure` are scaffolded in `overdrive-core` (R-35) and the rejected-set oracles of QC-3 and S-PIG-67 assert `BackendIdentityConflict`; QC-3 projects `receipts` through `live_receipts()` and the `take` handler asserts as R-33 states; `conflicting_alloc_rejected` is replayed with its empty-update tail (R-36) | S-PIG-75 is dropped (R-21) |
| B4a core actions, reconciler, control plane | S-PIG-65: `gateway_svid_lifecycle.rs` (5), `action_shim/gateway_svid.rs` (3), `public_ingress_gateway_hydration.rs` (5); S-PIG-100 disabled arm (`a_disabled_gateway_never_probes_the_relay`, `kernel-relay-sim-adapter`); QC-1 (5 + 1, `quint-conformance-gateway_identity`) | S-PIG-59 (`a_disabled_gateway_boots_and_shuts_down_exactly_as_before`, no ignore; recorded passing on metal); four further S-PIG-65 tests that assert only pinned, already-real facts (the reconciler's wakeup-model declaration, the shim errors' fixed Display and source, the disabled composition refusing a target, the disabled hydration read) | the enabled-boot arms, all in `public_ingress_gateway_boot_shutdown.rs` and ignored `pending DELIVER` their activating step: S-PIG-02 (I arm), 57 (I arm), 58, 60 (I arm), 61, 62, 74 (a), (c), (d) and the enabled arms of S-PIG-100 (lying relay, probe-call count across a replacement); the enabled boot arm of `lib.rs` is scaffolded with `GatewayAdapters.client_mtls` and boot G1–G4 | none |
| B4b CLI | S-PIG-18 (2 tests), 20 (the partial-mask test; its pre-effect second test is deleted, R-42) and 36 (1), all `route-operator-surface` | S-PIG-19 (`repeated_or_malformed_gateway_arguments_are_rejected_at_argv`, expected GREEN on the scaffolded clap declaration) | authored and ignored `pending DELIVER route-operator-surface`: S-PIG-34, 35, 42 (the enabled composition behind the operator listener) | the DELETE arm of S-PIG-36 (specified by R-53: `ApiClient::withdraw_route` against the disabled composition) |
| B5 sim invariants, conformance, examples, E15, docs | `overdrive-sim`: S-PIG-31, 32 (4 tests including the twin-run traces; `gateway-application-invariants`), S-PIG-102, 103 (2; `kernel-relay-exchange-driver`); conformance binary `public_ingress_gateway_end_to_end` (11 tests, all `public-request-end-to-end`, second octets 100, 104..=106 and 108..=114 within the owned range 100..=119; 101..=103 and 107 are unused): S-PIG-01, 56 (N), 60 (N graceful run), 69 (N; the guest flow is severed by `SIGKILL` of the VMM), 70 (the two 408 cases, a stalled head and a stalled body; the typed 413/400/421/431/505 statuses on native metal are carried by the S-PIG-115 test), 71 (N), 105 (N), 109 (N), 113, 115 (N) | none | none: the conformance tests set only `ServerConfig.gateway` and call the production `run_server`; they fail first at the enabled-boot `todo!` | S-PIG-115's N-arm drain-during-discard clause (the port-level file and the Sim rows hold it); the relay-probe success occurrence of S-PIG-01 (the DESIGN pins no event name; the test asserts the connect-probe occurrence precedes the bind) |

Non-Rust artifacts of B5: `examples/public-ingress-gateway/` (`README.md`, `prepare.sh`, `prepare-test-ca.sh`, `route.toml.template`, `run-example.sh`, `service.toml`, `guest_server.rs`) and the conformance guest `tests/conformance/guest/pig_guest.rs`; E15 `runner.sh` (never run) and the harness `EXPECTATION_SOURCE_SHA`/`EXPECTATION_SOURCE_DIRTY` capture; the `OUT-PIG-PUBLIC-REQUEST` artifact path in `docs/product/outcomes/registry.yaml`; the gateway rows of `docs/architecture/atdd-infrastructure-policy.md`. None was executed on the metal host or under KVM.

**Ids with no authored body:** S-PIG-11 (retired), 75 (dropped, R-21), and the arms the table's "Not authored" column names with their reason. Every other id has a body in the tree, ignored under a pending key except the active expected-GREEN rows named above. **Ids authored in part:** S-PIG-54, 93, 98, 115 (arms above), 83 (I half and U half in separate files), 100 (disabled arm, refused-relay arm and probe-count arm), 60 (I arm and N graceful run).

**Extra or reshaped tests, decided by the 2026-11-03 rulings** (R-42 and the rulings named): (1) Of the six S-PIG-77–82 listener-level tests, the four whose content is an exhaustive pure classification (S-PIG-79, 80, 81, 82) are deleted and the source-local U bodies against the R-32 signatures replace them; the two arms observable only end to end stay as the S-PIG-77 listener arm (a typed 400 or 505 and no octet at the upstream peer) and the S-PIG-78 listener arm (502 before the public head). (2) The second S-PIG-20 test (the pre-effect half) is deleted: no accepted contract orders the refusal before the cgroup probe, and the probe is read-only. (3) S-PIG-114's child test, its `unshare(CLONE_NEWUSER)` re-execution and its `tls`-module restore guard are deleted (R-23); the test keeps the honest real-kernel pass. (4) S-PIG-123 stays (R-39). (5) The Sim test-support surface (`SimGatewayRelay` capability set, `SimGatewayConnectDataplane::{new, fail_next, connect_hold}`, `SimGatewayClientMtls::with_hold`, `SimConnectHold`, the `fault_slots` field and the frontend constructor argument of the gateway application invariant case) stays: each item realises a capability the DESIGN pins (the typed fault surface and the hold/release capability of the consuming-port contract; the scripted peer and substrate-lie capability of the relay probe and S-PIG-90); none is DESIGN surface and none is extended. The `overdrive-sim` `[dependencies]` edge on `overdrive-host` stays (R-40). (6) `CliError::ServerShutdown.source` is `Box<ServerShutdownError>` (R-41). (7) Constructor bodies were filled in other batches' scaffold files so default-lane fallout runs without a `todo!` panic (`HostGatewayRelay::new`, `PublicCertifiedKeyAeadCodec::new`, `SimGatewayConnectDataplane::new` store their argument only).

**Contradictions between the batch notes and the tree** (the tree is followed): the B4a note counts four ignored hydration tests and the tree has five (S-PIG-65 `hydration_*`, including `a_disabled_desired_identity_hydrates_the_epoch_with_no_subject`); the B5 conformance note suggests deleting or restating S-PIG-110 because P-16 is not adopted, but the authored test asserts the ruled single rule (the pipe is released when the read half starts its next head read, R-16), so it stays and only its tag text changed.

**Changes under R-20..R-44 (the acceptance designer's list).**

- *Authored (unblocked or newly specified):* S-PIG-38, 43, 52, 64 (`overdrive-control-plane/tests/integration/public_ingress_gateway_mtls.rs`; the seal's host codec edge exists only in that crate, P-34); S-PIG-02 I arm, 34, 35, 42, 57 I arm, 58, 60 I arm, 61, 62, 74 (a), (c), (d), 100 enabled arms; S-PIG-76, 85 and the U halves of S-PIG-77-83 as source-local tests of the R-32 functions; S-PIG-86 against `PIPE_FLOOR`, `PipeCapacity::new`, `splice_in_len`; S-PIG-120's 118 s arm and S-PIG-121's 408 and 120 s arms (R-37); the QC-3 `receipts` projection and the `take` handler assertion (R-33); the QC-3 BackendId-reuse trace through an empty update, carried by `conflicting_alloc_rejected` (R-36, R-38); the QC-1 `submit_evaluation` assertion that no pending Issue carries an epoch beyond `desired()`'s (R-31; `issues` stays model-only; P-37 was rejected on 2026-11-04). All are ignored under pending DELIVER keys.
- *Change:* `discard_remaining` callers and the S-PIG-104 comparison (R-22); `GatewayControl` users to `GatewayRouteCommands`/`GatewayStatusReads` (R-25: QC-2 driver, `gateway_composition.rs`, handlers, tests); `disable_after_drain()` and `Timeout` callers and the Sim control adapter (R-30); the shutdown capability field and its tests (R-29); the QC-3 rejected-set oracles assert `DataplaneError::BackendIdentityConflict` (R-35); `compose_read_deadlines` is the one place the response-head values are computed (R-32, R-37); the `existing_id` scaffold becomes a body (R-24); `predecessor_row` in `action_shim/replacement_acceptance.rs` gains `#[cfg(test)]` (R-43).
- *Delete:* S-PIG-75 (never authored); four of the six listener-level tests of S-PIG-77-82 in `public_ingress_gateway_relay.rs`: `lines_with_bare_control_octets_*` (S-PIG-79), `chunk_size_and_trailer_lines_*` (S-PIG-80), `connection_nominations_*` (S-PIG-81) and `the_request_head_policy_*` (S-PIG-82); `ambiguous_request_framing_*` (S-PIG-77) and `ambiguous_origin_framing_*` (S-PIG-78) stay as the two listener arms; the second S-PIG-20 test; the S-PIG-114 child test, its re-exec helper and restore guard; S-PIG-116 (row and its two tests, SUB4).
- *Keep unchanged:* S-PIG-90's octet check (R-44), S-PIG-123 (R-39), `CliError::ServerShutdown.source` as boxed (R-41), the Sim test-support surface (R-42).

**SVID issuance failure occurrence pass (R-45..R-52; ADR-0274, ADR-0275).** Authored in a separate pass over the DESIGN section "SVID issuance failure occurrence"; recorded from the working tree. The rows O-SIF-G1..G7 and O-SIF-W1..W5 are in the "SVID issuance failure occurrence scenarios" table under "Scenario list and tags" (the one home of each contract, test name and activating key): 20 tests carry complete bodies and are ignored under those keys, and six pure tests are active (one pre-existing). **Changed:** the `IssueFailed` arm and its compile guard are removed from the gateway integration file (R-49); the gateway row `FIXTURE_V1` is regenerated and its "never touched" header rewritten (R-51). **Not authored:** the gateway owner's undecodable-prior-row half through the gateway `World` (see "Documented gaps and blockers" item 9); the `GET /v1/allocs` stamping and `GET /v1/gateway/status` projection are handler bodies (DELIVER), so only their operator-visible outcomes are specified.

## Wave: DISTILL / [REF] Scenario list and tags

Counts against the 55 pre-reconstruction scenarios: **22 KEEP, 32 ADAPT, 1 DROP** (S-PIG-11, demand revision type removed) **plus 67 NEW** (S-PIG-56..75 from the reconstruction and S-PIG-76..123 from G1 and its review remediation, without the deleted S-PIG-116) = **121 live scenarios** (22+32+67 = 121 before the drop of S-PIG-75 by R-21) and **122 scenario ids** including the DROP row; with S-PIG-75 dropped (R-21) **120 scenarios are live**. G1 adds 47 scenarios by lane: **U 11** (76–86), **I 19** (87–101 and 119–122), **S 2** (102–103), **T3 14** (104–112, 114, 115, 117, 118 and 123; 105 and 115 add a native-metal production-path case, 118 a unit assertion) and **N 1** (113); 11+19+2+14+1 = 47. The earlier figures (69 live / 70 ids; 74 / 75; 117 / 118) are superseded by these two units. Four quint-connect conformance drivers (QC-1..4, 18 bound `#[quint_test]` plus 4 `#[quint_run]`; QC-4 `relay_exchange` is specified in Scaffolds) and one EDD expectation (E15, claim authored, `pending`). The 2026-11-04 rulings (R-53, R-54) append five NEW rows S-PIG-124, 125, 127, 128 and 129 (T3 2, gate 1, U 1, I 1; id 126 is not used) in the table "Hook, gate and small-gap scenarios" below; the totals above are not recomputed (125 live scenarios after the append, 127 ids counting the two DROP rows). Scenario ids are stable identifiers and none is renumbered; what is authored, ignored, active, blocked or not authored for each id is recorded in "Authoring status by batch". Every scenario carries one `@contract-shape:` tag; error/edge/boundary/security/degraded/lifecycle/concurrency share is above 80% (gate 40%; the happy-path rows are 1, 87, 88, 105 and the pure derivation 76; the five rows 119–123 are all error, boundary or interruption cases). Lane: **U** unit/proptest, **I** `integration-tests` in-process, **S** seeded `overdrive-sim`, **T3** real kernel on native metal, **N** native metal with a real VM Service, **QC** quint-connect.

| ID | Scenario contract | Disp. | Lane | Tags | Owning crate / target on main | Activating step |
|---|---|---|---|---|---|---|
| S-PIG-01 | An external user receives the api workload's streamed response through the public Route (`AT-PIG-E2E-1`); the public leg and the upstream leg are kernel-TLS armed for the request (`ss -K`, `tls_stat`) and the relay probe preceded the bind | ADAPT | N | @walking_skeleton @driving_port @real-io @contract-shape:bounded-change | `tests/conformance/tests/public_ingress_gateway_end_to_end.rs` | public-request-end-to-end |
| S-PIG-02 | A sound listener with no selectable backend returns 503 and no fallback; unavailable path is typed, never a wait (the I arm only: a client socket through `SimGatewayRelay`, run twice with an empty second delta; the real-hook 503 is observed on the native-metal arm of S-PIG-71); **the I arm is authored, ignored (`a_service_with_no_backend_is_answered_503_twice_with_an_empty_second_delta`)** | ADAPT | I | @error @security @contract-shape:bounded-change | control-plane integration `public_ingress_gateway_boot_shutdown.rs` | consuming-port-unavailable-outcome |
| S-PIG-03–10 | Route path/host/input/generation laws (exact and segment match, rejected partitions, label 62/63/64, input channels, Route/key/application generation) | KEEP | U | @property/@boundary/@error @contract-shape:pure-function | `overdrive-core/src/public_ingress.rs` | route-value-types |
| S-PIG-11 | Demand revisions nonzero, monotone, refuse wrap | DROP | — | retired (D2) | — | — |
| S-PIG-12–17 | Canonical Host/SNI, authority errors, header rewrite both directions, hop-by-hop removal, fixed finite limits | KEEP | U | @property/@security/@boundary @contract-shape:pure-function | `overdrive-gateway/src/runtime/{routing,proxy_headers,limits}.rs` | gateway-application-owner |
| S-PIG-18–20 | Four serve flags; repeated scalar rejected; all-or-none enablement masks name exactly the missing fields; the S-PIG-20 mask test is a CLI integration test in `overdrive-cli/tests/integration/public_ingress_gateway.rs` (no ordering test against the cgroup probe, R-42); the S-PIG-19 argv rejection test is expected GREEN on the scaffolded clap declaration (no ignore) | KEEP | U | @driving_adapter @error @contract-shape:pure-function | `overdrive-cli/tests/acceptance/public_ingress_gateway.rs` | route-operator-surface |
| S-PIG-21–24 | Route declare/replay/replace/withdraw lifecycle; competing id; wrong-id withdrawal; two concurrent claims | ADAPT | I | @real-io @error @concurrency @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway.rs` | gateway-application-owner |
| S-PIG-25 | Gateway status contains no path, key, ciphertext, nonce or salt | ADAPT | I | @security @contract-shape:unbounded-preservation | same | gateway-application-owner |
| S-PIG-26 | Clean shutdown leaves identity empty and no connect residue | ADAPT | I | @lifecycle @contract-shape:bounded-change | same | gateway-boot-and-shutdown |
| S-PIG-27 | Unreadable manual key files refuse boot without Route mutation | ADAPT | I | @error @degraded-resource @contract-shape:unbounded-preservation | same | certified-key-custody |
| S-PIG-28–30 | Missing receipt / other-ServiceKey receipt / wrong-valid selected peer fail closed, clean up, never retry another backend | ADAPT | U/I | @error @security @contract-shape:bounded-change | `overdrive-gateway/src/connector.rs` | upstream-socket-preparation, selection-receipt-and-applied-identity |
| S-PIG-31 | Stale probe completion, replacement, withdrawal, retained connection (a client socket held through `SimGatewayRelay`) and restart preserve generation safety (invariant `GatewayApplicationGenerationLifecycleIsSafe`) | ADAPT | S | @property @seeded-sim @contract-shape:bounded-change | `overdrive-sim/tests/acceptance/gateway_application_lifecycle.rs` | gateway-application-invariants |
| S-PIG-32 | Shutdown joins admitted users before the identity slot clears (`GatewayApplicationShutdownConverges`) | ADAPT | S | @property @seeded-sim @interruption @contract-shape:bounded-change | same | gateway-application-invariants |
| S-PIG-33 | Withdrawing from Empty is Unchanged; the empty set survives reopen | ADAPT | I | @edge @real-io @contract-shape:bounded-change | gateway integration | gateway-application-owner |
| S-PIG-34–36 | Route deploy one-shot for both detach modes; real POST handlers via ApiClient; disabled composition rejects before parse, **for the Route deploy and for the route DELETE through `ApiClient::withdraw_route` (R-53: DELETE of an id the server cannot parse, `Not A Valid Route Id`, returns 409 `gateway_disabled` (proving rejection before the id is parsed), message `public ingress gateway is disabled`, `field` None, before the id is parsed, with the intent store gaining no Route or gateway key; test `route_withdraw_on_a_gateway_disabled_node_is_rejected_before_the_id_is_parsed`, second S-PIG-36 test, authored in DISTILL follow-up and ignored `pending DELIVER route-operator-surface`)**; **S-PIG-34, 35 and 36 are authored, ignored `pending DELIVER route-operator-surface`; declare, S-PIG-35's route withdraw and status read (and S-PIG-42's status read) go through `ApiClient` (`submit_route`, `withdraw_route`, `gateway_status`, R-53; user ruling 2026-11-05)** | ADAPT | I | @driving-adapter @error @contract-shape:bounded-change | `overdrive-cli/tests/integration/public_ingress_gateway.rs` (boots through `SimDataplaneComposer`) | route-operator-surface |
| S-PIG-37 | Cleanup ledger 0/1/128, the 129th reservation refused before `open_upstream` (no socket prepared, no registration; R-5) | ADAPT | U | @boundary @contract-shape:bounded-change | `overdrive-gateway/src/application.rs` | gateway-application-owner (reserve/bind/complete/saturation/Prepare-release tests) and upstream-socket-preparation (cleanup-retry and ledger-full-refusal tests) |
| S-PIG-38, 43, 52 | Real rustls (client built by `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)`): valid peer B rejected when selected is A before any body byte and nothing is armed; absent/expired/malformed identity, non-peer fd, elapsed handshake, URI-SAN shape (the positive arm, where the peer observes the dedicated gateway SVID and the leg is armed, is S-PIG-64); **authored, ignored `pending DELIVER exact-peer-client-mtls` (a `GatewayUpstreamSeal` is minted only by `UnboundGatewayBuilder::into_builder`, so the test builds the deps through the production composition or the Sim factory)** | ADAPT | I | @security @error @real-io @contract-shape:bounded-change | `overdrive-control-plane/tests/integration/public_ingress_gateway_mtls.rs` (the seal's host codec edge exists only there; P-34) | exact-peer-client-mtls |
| S-PIG-39 | Request outside a Route returns 404 (no upstream dial; head-only parse by the gateway); carried by the route-miss case of `an_early_or_gateway_authored_response_ends_with_a_half_close_and_a_bounded_discard` | ADAPT | I | @error @public-https @contract-shape:bounded-change | gateway integration `public_ingress_gateway_relay.rs` | relay-lingering-close |
| S-PIG-40 | A selected upstream closing before response maps to 502 exactly once, no retry; carried by `an_upstream_failure_is_one_502_before_the_head_and_an_unclean_close_after_it` and the connector's one-attempt, no-retry tests (`overdrive-gateway/src/connector.rs`) | ADAPT | I | @error @single-attempt @contract-shape:bounded-change | gateway integration `public_ingress_gateway_relay.rs` + connector | kernel-relay-exchange-driver |
| S-PIG-41 | An unregistered cgroup socket keeps non-service behaviour; `LOCAL_BACKEND_MAP` and XDP attachments unchanged (**expected-GREEN preservation row; un-ignored from authoring, no `#[ignore]`**) | ADAPT | T3 | @security @real-kernel @contract-shape:unbounded-preservation | `overdrive-dataplane/tests/integration/public_ingress_gateway_hook.rs` | gateway-selection-hook |
| S-PIG-42 | Status point-read failure returns 500 without credential detail, then a healthy read returns the normal projection; **authored, ignored `pending DELIVER route-operator-surface`; the CLI test reads status through `ApiClient::gateway_status` (R-53; user ruling 2026-11-05), so no CLI-output clause is asserted (no CLI status command is pinned)** | ADAPT | I | @error @degraded-resource @contract-shape:unbounded-preservation | CLI integration | route-operator-surface |
| S-PIG-44 | Gateway newtypes round-trip Display/FromStr/TryFrom/serde/rkyv and reject each noncanonical partition | KEEP | U | @property @codec @contract-shape:pure-function | `overdrive-core` | route-value-types, gateway-identity-epoch |
| S-PIG-45 | Certificate chains 0/1/many/9 inside one validity interval | KEEP | U | @boundary @security @contract-shape:bounded-change | `overdrive-gateway/src/certified_key.rs` | certified-key-custody |
| S-PIG-46 | Key install replay, replacement, invalid replacement, expiry deltas | ADAPT | I | @lifecycle @error @contract-shape:bounded-change | gateway integration | certified-key-custody |
| S-PIG-47 | Tamper, AAD mismatch and CSPRNG failure expose no plaintext or protected record | KEEP | U | @security @error @contract-shape:bounded-change | `overdrive-host/src/ca/public_certified_key.rs` | certified-key-custody |
| S-PIG-48 | A lagged IntentStore subscription preserves the missed count and resumes with the next `Changed` | KEEP | I | @interruption @error @real-io @contract-shape:bounded-change | `overdrive-store-local` acceptance/edges | lag-aware-intent-subscription (complete body, reasoned ignore; the type/signature port landed with item 12, reconfirmed 2026-11-02, R-18) |
| S-PIG-49 | Frontend resolution closed errors; real persisted Job/Schedule rejected; intent universe preserved (uses `IntentKey::workload_prefix`) | ADAPT | U/I | @error @contract-shape:unbounded-preservation | `overdrive-gateway/src/frontend.rs` | gateway-application-owner |
| S-PIG-50 | Every `GatewayConnectDataplaneError` crosses its real owner op and collapses to the redacted `ConnectPathUnavailable` | ADAPT | I | @error @adapter-contract @contract-shape:bounded-change | gateway integration | upstream-socket-preparation |
| S-PIG-51 | Gateway identity lifecycle errors cause-distinct at the owner, collapsed to `GatewayIdentityUnusable` in status | ADAPT | I | @error @lifecycle @security @contract-shape:bounded-change | gateway integration | gateway-identity-error-collapse |
| S-PIG-53 | A failed cleanup is retained and retried without re-registering, reconnecting or re-authenticating (the retained record is created at the pre-reservation and bound after `open_upstream`; R-5) | ADAPT | U | @error @degraded-resource @contract-shape:bounded-change | connector | upstream-socket-preparation |
| S-PIG-54 | **C4a restated (D2).** The unavailable-outcome path applied twice has an empty second delta (see below); custody replay and identity hold repeat keep coverage; **authored: custody replay across a restart and an identical Route re-apply (gateway integration file). S-PIG-54 has no Tier-1 connector arm: the S-PIG-02 I arm runs twice and asserts the empty second delta** | ADAPT | I | @idempotency @contract-shape:bounded-change | gateway + control-plane integration | consuming-port-unavailable-outcome |
| S-PIG-55 | Route parsers accept exactly `target.service`; no `target.workload` alias | KEEP | U | @error @security @parser @contract-shape:pure-function | `overdrive-core` | route-value-types |
| S-PIG-56 | A stale Service-map entry after `job stop` never yields a wrong-peer success: the dial fails with exactly 502 (the arm ignores `BACKEND_MAP.healthy`, R-2; obl 3) | NEW | T3+N | @error @security @contract-shape:bounded-change | hook file + conformance file | consuming-port-unavailable-outcome |
| S-PIG-57 | Sentinel probe failure keeps TCP/443 unbound; serve and workloads unaffected; status cause and `health.startup.refused` reason `gateway.connect_probe` (obl 4, G-1; the relay probe's own refusal is S-PIG-100); **the I arm is authored, ignored `pending DELIVER gateway-boot-and-shutdown` (it also carries the `Unavailable`-at-probe cause); the T3 arm is the passing probe and the elapsed deadline** | NEW | I+T3 | @error @degraded-resource @contract-shape:bounded-change | control-plane integration boot/shutdown file + hook file | gateway-boot-and-shutdown |
| S-PIG-58 | Actual-frontend `NoBackend` is bind-valid; the listener binds and serves 503 (G-1); **authored, ignored `pending DELIVER gateway-boot-and-shutdown`; the no-backend premise is asserted before the request** | NEW | I | @edge @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-59 | Gateway disabled (`ServerConfig.gateway == None`): boot unchanged, no socket prepared, no sentinel, unregistered hook behaviour unchanged (boundary 6); **expected-GREEN preservation row; un-ignored from authoring, no `#[ignore]`; it presupposes the disabled composition constructs no gateway scaffold, as the DESIGN's Disabled composition creates none; the relay-probe-count assertion for the disabled case is S-PIG-100's** | NEW | I | @config-disabled @contract-shape:unbounded-preservation | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-60 | Enabled boot orders G0–G4 (the relay probe, then the connect probe, run inside the gate task); shutdown stage `gateway` runs before `mtls_worker_owner`, inside one 5 s grace, one budget (drain ends at 4.5 s, the final owner joins end by 5 s, R-64); in-flight pipes end with the exchange (G-6); **the I arm is authored, ignored `pending DELIVER gateway-boot-and-shutdown`; the N graceful run is authored (native metal)** | NEW | I (+N graceful run) | @lifecycle @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-61 | A gated gateway (relay probe refused) never requests serve shutdown while the node keeps ticking, and a requested shutdown completes inside the unchanged 10 s fail-stop bound (the gateway stage inside the 5 s drain is asserted by S-PIG-60); no driven-port fault fails the gateway task itself, so a task-failure stimulus is not authored; **authored, ignored `pending DELIVER gateway-boot-and-shutdown`** | NEW | I | @interruption @error @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-62 | Allocation `Running`, Service `Stable`, hydration and the operator listener advance identically with the gateway enabled, disabled or gated (boundary 3); **authored, ignored `pending DELIVER gateway-boot-and-shutdown` (disabled, enabled and gated compositions are compared)** | NEW | I | @orthogonality @contract-shape:unbounded-preservation | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-63 | A registered socket is selected by the hook; the rewritten dial enters leg-C via table 100 and TPROXY with `SO_MARK` and `127.0.0.1` source (obl 6) | NEW | T3 | @real-kernel @contract-shape:bounded-change | hook file | gateway-selection-hook |
| S-PIG-64 | Exact-peer mTLS evidence on the dial segment, observed by a recording TCP proxy between the dial and a real rustls peer on loopback (TLS record types after the hello flight, body marker absent) and by `getsockopt(TCP_ULP)` on a duplicate of the dialed descriptor (kTLS armed after the equality); the test PKI carries a gateway SPIFFE-identity leaf and the peer observes it; not a kernel packet capture (no AF_PACKET, `ss -K` or `tls_stat`); the matching-identity case only (obl 7, G-5): the wrong-valid, expired and absent negatives are run in-process against a real rustls peer by S-PIG-38/43/52, so they are not repeated on the wire; **authored, ignored `pending DELIVER exact-peer-client-mtls`** | NEW | T3 | @security @real-kernel @contract-shape:bounded-change | `overdrive-control-plane/tests/integration/public_ingress_gateway_mtls.rs` (not the dataplane hook file: the seal's host codec edge exists only in the control-plane crate; P-34) | exact-peer-client-mtls |
| S-PIG-65 | Gateway SVID lifecycle: issue, audit-before-hold, near-expiry reissue, stale-epoch no-op, disable clears the slot synchronously (no drop action), restart reissue (obl 9) | NEW | U+I | @lifecycle @security @contract-shape:bounded-change | reconcilers + control-plane action shim | gateway-svid-lifecycle-transitions, gateway-svid-issue-and-audit-executor, gateway-svid-lifecycle-hydration |
| S-PIG-66 | Route/custody/application watch recovery: subscribe before list, relist on `Lagged`, admission closed meanwhile (obl 10) | NEW | I | @interruption @contract-shape:bounded-change | gateway integration | lag-aware-intent-subscription (complete body, reasoned ignore; the type port landed with item 12, reconfirmed 2026-11-02, R-18; P-8 is not adopted) |
| S-PIG-67 | BackendId identity readable only after the outer swap commits; immutable per id; rollback preserves the prior generation; no residue; restart destroys receipts (obl 11) | NEW | I+T3 | @lifecycle @contract-shape:bounded-change | dataplane host/Sim equivalence + source-local | selection-receipt-and-applied-identity |
| S-PIG-68 | A late `Selected` or probe result after a superseded generation is never delivered and never restores the old admission object (boundary 4; the held gate needs no programmed backend and parks at an awaited operation the gate owns, DG-9, R-65) | NEW | I | @interruption @contract-shape:bounded-change | gateway integration | gateway-application-owner |
| S-PIG-69 | Guest-flow owner restart or fail-stop mid-request ends the request with 502 and leaves no intent or receipt (boundary 5) | NEW | I+N | @interruption @error @contract-shape:bounded-change | gateway integration + conformance file | public-request-end-to-end |
| S-PIG-70 | Authored: the two 408 cases (a stalled head and a stalled request body) through the real listener on native metal (the gateway parses only heads). Not authored here: obligation 8's 404, 502 and 503 are carried at the listener by the relay-file tests of S-PIG-39 and 40 and the S-PIG-02 I arm; 400 and 421 by S-PIG-82 plus the listener arms of S-PIG-77/78; a VM-level 400/421 case is P-35 (not adopted). The typed 413/400/421/431/505 statuses reaching a real client on native metal are carried by the S-PIG-115 native test | NEW | N | @error @public-https @contract-shape:bounded-change | conformance file | public-request-end-to-end |
| S-PIG-71 | After a serve restart (dataplane instance recreated on the same pin directory) the gateway returns 503 until the ordinary hydrator re-programs the Service, then the backend response; old identity and receipts are gone (SR-2, obl 11) | NEW | T3+N | @lifecycle @interruption @contract-shape:bounded-change | hook file (instance half) + conformance file (serve restart) | selection-receipt-and-applied-identity, public-request-end-to-end |
| S-PIG-72 | Updates racing `selected_backend_identity` for a just-selected id never return a missing identity for a `Selected` id (run `receipt_waits_for_publish_guard`, hazard `publishEarly`; existing ports only) | NEW | T3 | @concurrency @contract-shape:bounded-change | hook file | selection-receipt-and-applied-identity |
| S-PIG-73 | A connector probe result completed twice for one staged token is ignored the second time; status row and application generation unchanged (GA-1) | NEW | U | @idempotency @error @contract-shape:bounded-change | `overdrive-gateway/src/application.rs` | gateway-application-owner |
| S-PIG-74 | Gate-unavailable boundary (boundary 2), four sub-cases: (a) absent sentinel receipt, (b) hook not firing, (c) identity never Current, (d) probe timeout each leave TCP/443 unbound; (a) and (d) share the redacted `ConnectPathUnavailable` status and the `gateway.connect_probe` refusal reason by design (S-PIG-50 redaction), (c) alone carries the distinct `GatewayIdentityUnusable` status and no probe reason (no `health.startup.refused` reason is pinned for it); (b) is the `Unavailable`-at-probe stimulus of the S-PIG-57 I arm, which asserts the same status and reason; **(a), (c) and (d) are authored, ignored `pending DELIVER gateway-boot-and-shutdown`** | NEW | I | @error @degraded-resource @contract-shape:bounded-change | boot/shutdown file | gateway-boot-and-shutdown |
| S-PIG-75 | **Dropped (R-21).** Pre-commit rollback through the real owner by a `BACKEND_MAP` capacity failure; the exposure is stated in the Formal protocol model and `RollbackPreservesPriorGeneration` keeps model and equivalence coverage; the rollback path (the abandoned reservation) is model-only, and the owner-reachable neighbour (a rejected update consuming no id) is replayed by `conflicting_alloc_rejected` (R-36) | NEW | T3 | @error @lifecycle @contract-shape:bounded-change | hook file | — (dropped) |
| S-PIG-76 | Message length is derived once, from the original headers, by RFC 9112 §6.3: none for `HEAD`, `1xx`, `204`, `205`, `304` responses and for requests with neither length nor coding; exactly `Content-Length` octets; chunked; close-delimited only where permitted (R-12); **source-local body against `derive_framing` (R-32); its arms are also exercised through S-PIG-87–89, 91 and 119** | NEW | U | @property @boundary @contract-shape:pure-function | `overdrive-gateway/src/runtime/http1` (framing decision; source-local) | http1-head-validation-and-framing |
| S-PIG-77 | Ambiguous request framing is refused before any upstream octet: both `Content-Length` and `Transfer-Encoding`; repeated, non-decimal or `u64`-overflowing `Content-Length`; repeated `Transfer-Encoding`, `chunked, chunked`, a final coding other than `chunked`, any other coding: 400; an `HTTP/1.0` request: 505 | NEW | U | @error @security @boundary @contract-shape:pure-function | same | http1-head-validation-and-framing |
| S-PIG-78 | The same ambiguity on an origin response (both lengths, repeated or overflowing length, repeated or unsupported coding, a status line that is not `HTTP/1.1`, `101`) is a 502 before the public response head; an `HTTP/1.0` response with an explicit length is accepted (R-12) | NEW | U | @error @security @contract-shape:pure-function | same | http1-head-validation-and-framing |
| S-PIG-79 | Every line the gateway reads (head, chunk-size line, trailer line) holding a bare CR, a bare LF, a NUL, an octet outside RFC 9110 §5.5 or an obsolete line fold is refused before the parser runs | NEW | U | @property @security @contract-shape:pure-function | same | http1-head-validation-and-framing |
| S-PIG-80 | Chunk framing lines: hexadecimal size parsed with extensions discarded; a size overflowing `u64` or a malformed size line is refused (400 before the response head, else both legs close); trailer lines are consumed and dropped within the head ceiling | NEW | U | @property @boundary @contract-shape:pure-function | same | http1-head-validation-and-framing |
| S-PIG-81 | Framing is read from the original headers: a `Connection` field nominating `Content-Length` or `Transfer-Encoding` removes nothing before the framing decision, and the forwarded head carries only the framing headers the gateway itself writes | NEW | U | @security @property @contract-shape:pure-function | same + `proxy_headers.rs` | http1-head-validation-and-framing |
| S-PIG-82 | Request head policy at its boundaries: non-origin-form target or `CONNECT` 400; Host missing, duplicated, invalid, with userinfo or a port other than 443 400; Host different from SNI 421; declared length above the ceiling 413 and at the ceiling accepted; head one octet over the ceiling or 101 fields 431 and at the ceiling or 100 fields accepted; `Upgrade` removed as hop-by-hop | NEW | U | @error @boundary @contract-shape:pure-function | same | http1-head-validation-and-framing |
| S-PIG-83 | Interim heads: an origin `100` reaches the client as a body-less rewritten head; every other interim `1xx` except `101` is read and dropped within the same head limit and deadline; `Expect` is forwarded unchanged and the gateway decides nothing | NEW | U+I | @edge @error @contract-shape:pure-function | `runtime/http1` + the Sim-driven file | http1-head-validation-and-framing |
| S-PIG-84 | Exchange state laws: reuse only after the response completed, the request body was fully relayed, the response is length-delimited, the connection is not draining and the public peer has not half-closed; an early response, a close-delimited response, a mid-body close and a drain never reuse; an open chunked body is unfinished; octets beyond the declared remainder, `finish` before the response completes and every event after `Closed` are refused or ignored as pinned | NEW | U | @property @error @contract-shape:pure-function | `overdrive-gateway/src/runtime/http1/exchange.rs` (source-local PBT) | exchange-state-machine |
| S-PIG-85 | Deadline composition: every head or framing-line read receives `min(phase budget, request deadline)` for the first-request, keep-alive, framing-line and response-head budgets; an item can never outlive its request; no 2x on the first head. **Source-local body against `compose_read_deadlines` and `request_deadline` (R-32)**, including the response-head phase with `request_complete_at` `None` and `Some(t0)` (R-37); the first-request, keep-alive, later-request and response-head timelines are also exercised through S-PIG-96 and 120 | NEW | U | @property @boundary @contract-shape:pure-function | `runtime/http1` or `runtime/exchange.rs` (source-local) | exchange-state-machine |
| S-PIG-86 | Relay pipe floor (unit law): the floor constant is 65 536 octets; the splice-in length is `min(octets owed, capacity)` and never exceeds either; a pipe is drained before the next splice-in; creation failure and a read-back below the floor are the only runtime outcomes (`RelayPipeError`); the typestate that makes the other states unrepresentable is the crafter's and gets no compile-fail fixture. **Source-local body against `PIPE_FLOOR`, `PipeCapacity::new` and `splice_in_len` (R-32);** the drain-before-splice-in law is the typestate and has no test of its own | NEW | U | @boundary @contract-shape:pure-function | `overdrive-dataplane/src/gateway_relay` (source-local) | kernel-relay-host-adapter |
| S-PIG-87 | Two keep-alive exchanges on one public connection with `Content-Length` bodies, including a zero-length body: each body arrives octet-exact, nothing past the declared length is read, the next request head is untouched and (full-duplex, R-10) the response may start before the request body ends | NEW | I | @driving_port @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway_relay.rs` (production `GatewayBuilder`, `SimGatewayRelay`, `SimGatewayConnectDataplane`) | kernel-relay-exchange-driver |
| S-PIG-88 | Chunked bodies relay chunk by chunk in both directions: the gateway writes the size line, one exact chunk is spliced, the terminator is checked, extensions and trailers are not forwarded, and the gateway writes the last chunk; an open chunked request body is unfinished at an early response | NEW | I | @boundary @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-89 | Responses without a body (`HEAD`, `204`, `205`, `304`) relay nothing and leave the connection reusable; an interim `1xx` followed by one of them keeps the same rules | NEW | I | @edge @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-90 | Control events never reach the other leg: a peer KeyUpdate (requesting or not), a NewSessionTicket and a close_notify met inside a head, inside a chunk-size line, at a message boundary and inside a body leave the relayed octets exact; close_notify ends that direction after the message in flight; an alert or an unexpected record closes both legs; a KeyUpdate whose re-arm is refused (`KeyUpdateError::RustlsRefused`, `RearmRx`, `RearmTx`, `Io`) on either leg closes both legs and relays nothing past the failure: on the upstream leg before the public response head it is exactly one 502, after the head a close with no further response | NEW | I | @error @interruption @contract-shape:bounded-change | same (scripted peer events on the Sim relay) | kernel-relay-sim-adapter |
| S-PIG-91 | A close-delimited response is forwarded with `Connection: close`, ends cleanly on the origin's close_notify, is not reused, and closes both legs when it exceeds `max_response_body_bytes` (R-12) | NEW | I | @edge @error @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-92 | Upstream failure projection: a close before the response head is a 502 exactly once; a close or reset mid-body, a close_notify before the declared end, and any failure after the public head close both legs without a further response and never end cleanly | NEW | I | @error @interruption @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-93 | Early answer and lingering close (R-14, R-10): an origin answering before the request body is read (including 401 and 413) and each gateway rejection sent before a body is read (413 on declared length, 400, 404, 421, 431, 502, 503, 505) deliver the final response, half-close and discard the remaining request octets until the peer's close_notify, `max_request_body_bytes` octets or the deadline, then close; 408 and a leg that already reported EOF, alert or reset are not lingered (the gateway-authored rejections are sent before any further read, so only the 408 arm and the mid-request-body close_notify/reset arms of S-PIG-122, which assert zero `discard_remaining` calls, are executable) | NEW | I | @error @security @boundary @contract-shape:bounded-change | same | relay-lingering-close |
| S-PIG-94 | Body ceilings: a declared request length above the ceiling is 413 before any upstream octet; a chunked total above it is 413 if no public response octet was sent and otherwise both legs close; a response body above its ceiling closes both legs after the head; one octet under each ceiling passes | NEW | I | @error @boundary @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-95 | Relay pipe failure: a pipe that cannot be created or that reads back below the floor is a 502 before the response head and closes both legs after it; no octet moves | NEW | I | @error @degraded-resource @contract-shape:bounded-change | same | kernel-relay-sim-adapter |
| S-PIG-96 | Keep-alive and deadlines on the injected clock: idle expiry before the first octet closes silently (not a 408); no complete head in time is a 408 (first request counted from handshake completion, later request from its first octet); the hundredth response carries `Connection: close` and a 101st request is never parsed | NEW | I | @boundary @interruption @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-97 | Single upstream attempt: on every failure of S-PIG-92 and S-PIG-95 there is no retry, no second backend and no replay; exactly one upstream leg and one cookie registration per request; the upstream request head carries `Connection: close` | NEW | I | @error @single-attempt @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-98 | Arming refused after the exact-peer equality held, for each kind (`KtlsArmError::UlpInstall`, `CryptoInfo`, `CipherNotSupported`, `KernelConnection`): the result is `Arm(kind)` carried unchanged, no octet reaches the peer, cleanup runs once and the public request is a 502; the same four kinds refused at the public `accept_public` arm close the public socket without reading any HTTP octet and open no upstream leg; the carry-through of `Arm(kind)` is a connector-level fact, and the listener-level oracle is the 502, no octet at the peer, cleanup once and no residue | NEW | I | @error @security @contract-shape:bounded-change | same (`SimGatewayClientMtls` with a Sim arm refusal) | exact-peer-client-mtls |
| S-PIG-99 | Relay probe gold test: the honest Sim substrate passes; each substrate lie (`tls` ULP absent, cipher refused, splice losing octets, pipe clamped below 64 KiB) is refused with its typed `RelayProbeError` variant, and a probe timeout and an I/O error likewise; no lie is accepted | NEW | I | @error @adapter-contract @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway_relay.rs` | relay-earned-trust-probe |
| S-PIG-100 | Relay gate (boundary 7): a refused probe keeps TCP/443 unbound with `health.startup.refused` reason `gateway.relay_probe` and status `ConnectPathUnavailable`; the probe runs once per boot, after custody and before the connect probe, never again for a later staged generation; allocation `Running`, Service `Stable`, hydration and the operator listener advance identically; with the gateway disabled no relay probe runs | NEW | I | @error @orthogonality @lifecycle @contract-shape:bounded-change | `overdrive-control-plane/tests/integration/public_ingress_gateway_boot_shutdown.rs` | gateway-boot-and-shutdown |
| S-PIG-101 | Drain: an idle keep-alive connection closes at once; an in-flight exchange runs to completion until the drain deadline (`grace - 500 ms` join reserve, R-64) and is then force-aborted; no later request is parsed and the hundredth-response path is the same `Connection: close` path; a discard in progress ends at the drain deadline (R-14, R-64); the final owner joins then complete inside the reserve, so `report.task_failures` is empty and `report.elapsed <= grace` (plus at most one test clock step) | NEW | I | @lifecycle @interruption @contract-shape:bounded-change | `public_ingress_gateway_relay.rs` | kernel-relay-exchange-driver |
| S-PIG-102 | Seeded exchange invariant: under scripted peer events (KeyUpdate, NewSessionTicket, close_notify, alert, reset), upstream close before the response head and partial deliveries, no octet moves past a message boundary, no connection is reused after drain begins, and zero residue (leases, pipes, legs) remains after every outcome; the seed is printed | NEW | S | @property @seeded-sim @contract-shape:bounded-change | `overdrive-sim/tests/acceptance/gateway_relay_exchange.rs` | kernel-relay-exchange-driver |
| S-PIG-103 | Seeded drain and force-abort ordering: drain beginning at a seeded point of an exchange (head, request body, response body, discard) completes the exchange or force-aborts it at the drain deadline (a stalled discard instead ends at the drain deadline unforced and the socket is closed, R-14/R-64), never reuses, never leaves residue; twin runs at one seed produce the same ordered trace | NEW | S | @property @seeded-sim @interruption @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-104 | Host and Sim `GatewayRelay` agree: the same byte scripts and the same producible faults (peer KeyUpdate/NewSessionTicket/close_notify/alert, deadlines, short counts, backpressure, discard (R-14), `relay_to_end` (R-12)) give the same returned octets, error variants and close behaviour; substrate lies are not part of the comparison | NEW | T3 | @adapter-contract @contract-shape:bounded-change | `overdrive-dataplane/tests/integration/gateway_relay_equivalence.rs` | kernel-relay-host-adapter |
| S-PIG-105 | Byte-exact bodies over real kernel TLS: a `Content-Length` body and a chunked body arrive octet-exact at real rustls peers on both legs, and the next request head is untouched across two keep-alive exchanges (AES-128-GCM and AES-256-GCM only, R-8); the peer-side record sizes are recorded, as Tier-3 obligation 16(j) requires | NEW | T3+N | @real-kernel @driving_port @contract-shape:bounded-change | `overdrive-dataplane/tests/integration/gateway_relay.rs` (port level) + conformance file (production path) | kernel-relay-host-adapter |
| S-PIG-106 | A peer KeyUpdate (requesting and not requesting) inside a head, inside a chunk-size line, at a message boundary and inside a body leaves the octets exact, advances `TlsRxRekeyOk` and `TlsTxRekeyOk`, leaves `TlsDecryptError` unchanged, forwards nothing and the gateway originates none (R-13); without the re-arm the read stays `EKEYEXPIRED` (negative control); both AES suites (R-8) | NEW | T3 | @real-kernel @interruption @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-107 | A KeyUpdate request that arrives while a large splice-out is in flight never tears the record stream: the peer decrypts every octet and `TlsDecryptError` is unchanged (the transmit lock, assumption A-K5) | NEW | T3 | @real-kernel @concurrency @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-108 | A NewSessionTicket before the first upstream response is discarded; a close_notify in the middle of a head, in the middle of a body and after the response ends the exchange cleanly only when the framing says the message is complete; the NewSessionTicket arm is not testable at the port level (the only pinned way to obtain an armed leg is `accept_public`, server role) and belongs to the client-role leg of `HostGatewayClientMtls` | NEW | T3 | @real-kernel @interruption @error @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-109 | The upstream leg answers a peer-injected KeyUpdate by the same path and a production leg-C sends none across a response of exactly 16 MiB, the response ceiling (assumption A-K7): a non-zero `TlsRxRekeyReceived` delta from leg-C is a deterministic hard failure that falsifies A-K7 and is returned to DESIGN as a contract-shaping falsification, never recorded as inconclusive; the gateway originates none (R-13) | NEW | T3 | @real-kernel @contract-shape:bounded-change | `gateway_relay.rs` + conformance file | kernel-relay-host-adapter |
| S-PIG-110 | Pipe lifetime (the single rule stands, R-16): a keep-alive connection waiting for its next request holds no pipe descriptor, after a `Content-Length` body and after a chunked body | NEW | T3 | @real-kernel @lifecycle @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-111 | Backpressure: a stalled public reader freezes the upstream sender within the socket buffers and resident memory stays flat; a trickling `Content-Length` response is delivered to the client without waiting for further octets (no `SPLICE_F_MORE`); the stalled-body end at `no_progress` is the host/Sim equivalence case `a_stalled_body_ends_at_no_progress` of S-PIG-104 | NEW | T3 | @real-kernel @degraded-resource @interruption @contract-shape:bounded-change | `gateway_relay.rs` + `gateway_relay_equivalence.rs` | kernel-relay-host-adapter |
| S-PIG-112 | Record boundaries: one splice-in never returns octets of two records or crosses a record boundary; bodies built from records of mixed sizes (1 octet to 16 KiB, straddling heads and chunk-size lines) arrive exact and the relay pipe is empty at every splice-in (assumption A-K6) | NEW | T3 | @real-kernel @property-pinned @boundary @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-113 | 128 concurrent exchanges through the production composition: the process descriptor count stays within the accounted 768 above its baseline, resident-set growth plus the capacity of the process's pipe descriptors (read with `F_GETPIPE_SZ`) stays within 24 MiB (the DESIGN's accounted 192 KiB per exchange, 2 x 32 KiB head accumulators plus 2 x 64 KiB pipe buffers, times 128; no separate allowance for connection state, so a larger figure falsifies the budget and is returned to DESIGN), a 129th concurrent connection (`max_public_connections` is 128) is closed before any TLS octet, and every pipe and socket is released after drain | NEW | N | @real-kernel @concurrency @degraded-resource @contract-shape:unbounded-preservation | conformance file | public-request-end-to-end |
| S-PIG-114 | Real-kernel relay probe (R-23): the honest kernel passes, and the loopback pair and pipe are released; no real-kernel test unloads the `tls` module, and each substrate lie is refused at Tier 1 by S-PIG-99 | NEW | T3 | @real-kernel @error @adapter-contract @contract-shape:bounded-change | `gateway_relay.rs` | relay-earned-trust-probe |
| S-PIG-115 | Lingering close on a real kernel (R-14): a client that keeps sending a body after a 413 or an early 401 reads the response and then close_notify, not a reset; the discard ends at the peer's close_notify, at `max_request_body_bytes`, at the deadline; a drain during a discard ends it at the drain deadline; the pre-exchange rejections 400, 404, 421, 431, 505 behave the same; the 408, 413, 431 and 505 typed responses reach a real client | NEW | T3+N | @real-kernel @error @security @contract-shape:bounded-change | `gateway_relay.rs` + conformance file | relay-lingering-close |
| S-PIG-117 | Close-delimited response over real kernel TLS (R-12): forwarded with `Connection: close`, ends cleanly on the origin's close_notify, bounded by `max_response_body_bytes`, not reused | NEW | T3 | @real-kernel @edge @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-118 | Cipher set (R-8): clients offering AES-128-GCM and AES-256-GCM complete both legs and relay octet-exact; a client offering only ChaCha20-Poly1305 is refused before arming; the custody snapshot's server configuration lists exactly the two suites | NEW | U+T3 | @real-kernel @security @boundary @contract-shape:bounded-change | `gateway_relay.rs` + `overdrive-gateway/src/certified_key.rs` | kernel-relay-host-adapter (the server-configuration unit arm `the_public_server_configuration_lists_exactly_the_two_aes_gcm_suites` drives a connection through `SimGatewayRelay` and the accept loop: `kernel-relay-sim-adapter`) |
| S-PIG-119 | Response-side size limits at their boundaries, with the pre-/post-public-head projection: a declared response `Content-Length` of `max_response_body_bytes` plus one (16 MiB + 1) is a 502 before any public response octet and the ceiling itself passes; a response head of `max_response_head_bytes` plus one octet (32 KiB + 1) or with 101 fields (`max_response_headers` is 100) is a 502 before the public head, at the ceiling or at 100 fields it is relayed; interim heads count against the same cumulative head limit; after the public head a body that outgrows its ceiling closes both legs (S-PIG-94) | NEW | I | @error @boundary @contract-shape:bounded-change | `public_ingress_gateway_relay.rs` | kernel-relay-exchange-driver |
| S-PIG-120 | Response-head deadline on the injected clock (R-37): with the origin silent after the upstream request is completely written (instant t0: the head write for a bodyless request, the last body octet otherwise), the exchange waits until exactly t0 + `response_head_deadline` (5 s), then answers one 502 and closes the upstream leg (one tick earlier it is still waiting); dropped interim heads (a `103` at t0 + 2 s and a second `103` at t0 + 4 s) share that one budget and never restart it, so a final head that is absent or incomplete at t0 + 5 s, including a partly received one, is the same single 502, while a final head completing at t0 + 4.9 s is relayed; the budget is `min(response_head_deadline, remaining whole_request_deadline)` (a request whose body upload completes at 118 of its 120 s waits 2 s; drivable through the listener by a client whose body progresses within every `body_no_progress_deadline` window, because t0 follows the upload); an origin still reading a slow upload is never cut by `response_head_deadline`; no retry; the deadline does not apply once the public head is sent | NEW | I | @error @boundary @interruption @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-121 | Request-body no-progress at exchange level (a scripted body stall on the injected clock): after a complete head, a public client that moves no body octet for `body_no_progress_deadline` (30 s) gets exactly one 408 before any response octet, both legs close and the connection is not reused, for a `Content-Length` body, a stalled chunk-size line and a stalled chunk; no lingering discard follows a 408; a body that moves within every 30 s window is not cut by this rule (a request still unfinished at `whole_request_deadline`, 120 s, with no response octet sent is a 502); once any response octet has been written (an early origin answer) no 408 is written and the lingering close of S-PIG-93 governs. **All arms are reachable under R-37:** the response-head budget starts when the upstream request is completely written, so a stalled request body is bounded by `body_no_progress_deadline` (408 at 30 s with the origin silent) and `whole_request_deadline` (502 at 120 s with no response octet), and never by `response_head_deadline`; the early-answer arm is already authored | NEW | I | @error @interruption @boundary @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-122 | Public half-close after the request was fully read (QC-4 runs `half_close_after_request_read` and `public_close_mid_request_aborts`): a public peer that sends close_notify once the request is complete (a `Content-Length` body fully relayed, and a bodyless request) still receives the whole response, `Content-Length` or chunked, and then both legs close: the connection is not reused, no further head is parsed and the upstream leg is closed exactly once; close_notify while idle closes silently and opens no upstream leg; close_notify or reset mid-request-body aborts both legs with no response | NEW | I | @edge @interruption @contract-shape:bounded-change | same | kernel-relay-exchange-driver |
| S-PIG-123 | Public handshake refusals on a real kernel (real rustls client peers on loopback, port level): a client offering only ALPN `h2`, a client sending no SNI, a client silent until the passed handshake deadline and a client that resets mid-handshake are each refused: no HTTP octet is read, no upstream leg is opened, nothing is armed on the refused socket (the `tls_stat` install counters are unchanged) and the descriptor count returns to its start value; a cipher-set refusal is S-PIG-118 and an arming failure is S-PIG-98. The assertions are the observable outcomes only (refused, no HTTP read, no upstream, nothing armed, descriptors restored). Which `PublicHandshakeError` variant a stimulus maps to is an implementation detail of the host adapter and is not asserted; the error taxonomy is unchanged | NEW | T3 | @real-kernel @error @security @contract-shape:bounded-change | `overdrive-dataplane/tests/integration/gateway_relay.rs` | kernel-relay-host-adapter |

**Hook, gate and small-gap scenarios (R-54, 2026-11-04; appended after S-PIG-123, five NEW rows; ids are not renumbered).** Authored by the acceptance designer in a follow-up pass from these specifications; ignored under the pending keys named, except where stated. Descriptive keys carry no roadmap coordinates. All hook rows run on the native metal host through `cargo xtask metal run --` (kernel 7.0), in the `overdrive-dataplane` integration binary (already wholly in `host-kernel-shared`); there is no Tier-2 row (`BPF_PROG_TEST_RUN` is `ENOTSUPP` for `cgroup_sock_addr`). Program path, name and baseline are pinned in "Pinned: selection program, baselines and gates".

| ID | Scenario contract | Disp. | Lane | Tags | Owning crate / target on main | Activating key |
|---|---|---|---|---|---|---|
| S-PIG-124 | The hook's rewrite as the kernel shows it: a registered socket's `connect(2)` to the Service frontend `198.18.250.1:8080` (asymmetric port bytes `0x1F90`) completes with `getpeername` equal to the programmed backend `127.1.x.y:9000` (`0x2328`; the low-16-NBO WRITE shape: a host-order or high-half write lands elsewhere and the connect is refused), and the intent is registered for the socket's own kernel `SO_COOKIE`. It asserts only those two kernel-visible facts and does not re-assert S-PIG-63's routing-snapshot, receipt and cleanup oracle. The hook reads neither `user_ip4` nor `user_port`, so the READ half of the low-16-NBO hazard has no oracle in this program (sibling S-PIG-41 and the `development.md` rule cover it) | NEW | T3 | @real-kernel @security @contract-shape:bounded-change | `overdrive-dataplane/tests/integration/public_ingress_gateway_hook.rs`, test `a_registered_connect_lands_on_the_programmed_backend_with_the_port_in_network_order` | gateway-selection-hook-rewrite |
| S-PIG-125 | Sockets the hook must not select: (b) a non-registered (cleaned-up) cookie: a socket from `open_upstream` whose intent was removed by `cleanup` before it dials the programmed frontend is not selected; positive control: a second registered socket to the same programmed Service lands on the backend; the unregistered dial does not connect and fails with the kernel's own error, not `EPERM`, and creates no receipt; (c) a plain unregistered socket dialing the programmed frontend while a registered intent is outstanding is not rewritten (it does not connect) and creates no receipt, and the outstanding intent is untouched. The unprogrammed-Service denial is covered by S-PIG-67 step 1 | NEW | T3 | @real-kernel @error @security @contract-shape:unbounded-preservation | same file, tests `a_socket_whose_intent_was_cleaned_up_is_not_selected_and_leaves_no_receipt`, `a_plain_socket_dialing_the_frontend_is_not_rewritten_while_an_intent_is_outstanding` | gateway-selection-hook-unselected-sockets |
| S-PIG-127 | Verifier-budget gate for `cgroup_connect4_gateway`: the baseline file `perf-baseline/main/verifier-budget/veristat-cgroup-connect4-gateway.txt` exists and `cargo xtask metal run -- cargo verifier-regress` passes with the program measured on the pinned kernel (growth at most 5%, not within 10% of the 1M ceiling, program present in the object). A gate, not a Rust test: no `#[ignore]` marker exists; the pending state is the absence of the program and baseline file. DISTILL deliberately does not create the baseline file: a baseline naming a program the object lacks fails `cargo verifier-regress` for every existing program today, and the count cannot be known before the program exists; the DELIVER step that adds the program adds the file with its first measurement | NEW | gate (Tier 4) | @real-kernel @contract-shape:unbounded-preservation | `perf-baseline/main/verifier-budget/`, `crates/overdrive-dataplane/bin/verifier_regress.rs` (unchanged) | gateway-selection-hook-verifier-budget |
| S-PIG-128 | `SpiffeId::for_gateway(&NodeId)` yields exactly `spiffe://overdrive.local/gateway/<node-id>`: pinned example (`gateway-node`), and a proptest over generated valid `NodeId`s (the body's `unreachable!` rests on every valid node id forming a valid SPIFFE path): canonical `Display` equals the contracted URI `spiffe://overdrive.local/gateway/<node-id>`. Not a new newtype (so `rust.md` newtype-completeness adds nothing); the proptest follows the `for_allocation` precedent and `testing.md`'s rule for a pure function over a large argument space | NEW | U | @property @contract-shape:pure-function | `overdrive-core/tests/acceptance/spiffe_region_validation.rs`, tests `spiffe_for_gateway_derives_canonical_uri_for_any_valid_node_id` (proptest) and `for_gateway_matches_the_pinned_uri_on_a_representative_node` | gateway-spiffe-subject |
| S-PIG-129 | Gateway owner restart over an undecodable prior status row (gap 9a, O-SIF-G3 residual): garbage bytes under `observation_gateway_application_status` at the node-id key are written through the existing raw back-door, then the gateway `World` is started over the production `LocalObservationStore` on that file: the owner boots (no boot refusal, no new boot error), its first projection reads the row as absent (base zero), one recorded failure then projects `issue_failures == 1` with that failure as `last_issue_failure`, and the status row subsequently written decodes (the garbage is replaced). The one-warning count is asserted at the store by the store-local test and not here | NEW | I | @error @degraded-resource @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway.rs`, test `a_restarted_gateway_over_an_undecodable_prior_status_row_counts_from_zero`, second octet 187 (confirm free by grep) | gateway-svid-issue-failure-occurrence |
| S-PIG-130 | No gateway status row before a Route exists (R-58, R-52). Contract: the Gateway Application owner writes no `GatewayApplicationStatusRowV1` until a Route exists, and identity or failure wakes write none before a Route exists either; an earlier boot's row is still carried. Owner path: the production `UnboundGatewayBuilder` composition over the real `LocalIntentStore`, the Sim observation store and the scripted identity control; the oracle is `gateway_application_status_row(node)` read through the observation port. Fault stimulus: an identity wake (the identity is held) and two recorded issue failures (`Ca`, `Audit`) with no Route. (a) With no Route and no prior row the row stays absent for the watch window after boot and after the wakes. (b) Declaring the Route then writes the first row, carrying the held identity as `Current` and `issue_failures == 2` with the latest failure as `last_issue_failure`. (c) A row left by an earlier boot (two failures) and no Route in the new boot: the owner still writes, with `unavailable == RouteAbsent`, over the carried base of 2 and the same latest failure. Evidence tier: I (in-process, `integration-tests`) | NEW | I | @error @edge @degraded-resource @contract-shape:bounded-change | `overdrive-gateway/tests/integration/public_ingress_gateway.rs`, tests `no_gateway_status_row_is_written_before_a_route_exists`, `the_first_gateway_status_row_after_a_route_carries_what_was_held_before_it`, `a_status_row_from_an_earlier_boot_is_still_carried_when_no_route_exists`, second octets 188, 189, 190 | none (active) |
| S-PIG-131 | Reverse-NAT key owned by another Service (R-59). Contract: `update_service` rejects in planning (`plan_service_apply`), before any mutation, an update whose reverse-NAT key already maps to a different VIP, with `DataplaneError::ReverseNatConflict { key, current_vip, attempted_vip }`; nothing is overwritten and rollback never restores another Service's entry. Owner path: `Dataplane::update_service` on the production `EbpfDataplane` and on `SimDataplane`, driven through the same sequence. Fault stimulus: two programmed Services (VIP one on backend address 1, VIP two on backend address 2), then VIP two is updated to also name backend address 1, whose key already maps to VIP one. Oracle: the error names the contested key, VIP one as current and VIP two as attempted; both Services' map state (SERVICE_MAP programmed, reverse-NAT entry count, contested key present) is equal before and after through each adapter's own accessors; each Service still selects its own allocation with the same `BackendId`; Host and Sim traces are equal. Evidence tier: I (Sim half) + T3 (Host half, native metal) | NEW | I+T3 | @error @contract-shape:bounded-change | `overdrive-dataplane` `tests/integration/gateway_connect_dataplane_equivalence.rs`, test `host_and_sim_reject_an_update_whose_reverse_nat_key_belongs_to_another_service`; authored and ignored `pending DELIVER selection-receipt-and-applied-identity`. Scaffold: the `DataplaneError::ReverseNatConflict` variant (R-59) exists in `overdrive-core` with no behaviour | selection-receipt-and-applied-identity |
| S-PIG-132 | Failed in-place `BACKEND_MAP` restore during rollback (R-60). Contract: a pre-commit failure after an in-place `BACKEND_MAP` update whose restore also fails returns `ServiceApplyRollback { stage, rollback: [BackendRestore(id)] }`, never only a log line. Owner path: `Dataplane::update_service` on the production `EbpfDataplane` (real aya maps, native metal), reaching `rollback_service_apply`. Seam: the crate-private `BACKEND_MAP` seam and in-crate decorator of "Test seam for the rollback restore" (user-approved 2026-11-06; no new public API, no R-62). **Setup.** Generation 1 of one Service is programmed through a successful `update_service` with backends A (`healthy = false`) and X; endpoints are chosen so the planned order (ascending `(ip, port)`) is A, B, C. **Fault stimulus** (armed immediately before the generation-2 update): generation 2 names A (`healthy = true`, an in-place update), X (identical, never rewritten), B and C (fresh). `BACKEND_MAP` writes then occur as: #1 A in-place update (passes), #2 B insert (passes), #3 C insert, **failed by the decorator** (stage `BackendMap`), and in rollback #4 the restore of A's prior value, **failed by the decorator**. Fail-set = the 3rd and 4th `insert` after arming; removes and every other operation delegate to the real map. **Oracle, as the current rollback code produces it (`rollback_service_apply`, read, not inferred):** (1) the result is `Err(DataplaneError::ServiceApplyRollback { stage: ServiceApplyStage::BackendMap, rollback })` with `rollback == [ServiceApplyRollbackFailure::BackendRestore(id_of_A)]` exactly: B's delete succeeded (restores run first, then the inserted-id deletes, and every one is attempted after a failure), so no `BackendDelete` and no `ReverseNatDelete` appears; (2) `BACKEND_MAP`: A holds the generation-2 value (the restore failed, so the in-place update stands; that residual is exactly what the typed failure reports), X is byte-equal to generation 1, B is absent (removed by rollback), C is absent (its insert failed and was never recorded); (3) `REVERSE_NAT_MAP`: equal to generation 1 (the `ReverseNat` stage was never reached): the keys for A and X are present, none exists for B or C, and `service_cleanup_debt` is empty (debt is recorded only for a failed reverse-NAT delete); (4) `SERVICE_MAP`: the slot is still programmed with generation 1's inner map, because neither `InnerMap` nor `OuterCommit` was reached, and the service-backends tracker still names generation 1's ids; (5) identities: B and C hold no `Applied` identity and their allocator memos are released (`existing_id` is `None` for both endpoints), A and X keep their generation-1 `Applied` identities; (6) recovery: after disarming, repeating the generation-2 update returns `Ok`, A reads `healthy = true`, and B and C hold fresh `BackendId`s that are not the abandoned numeric ids (ids are never recycled). Reads (2)-(6) go through the existing inspectors (`backend_map_entries`, `reverse_nat_map_has_backend_proto`, `service_map_contains`) and, because the test is in-crate, the crate-private trackers and identity map. **Bound and cleanup:** one `update_service` call per arm, no wall-clock wait; the decorator is disarmed after the assertion and the dataplane, bpffs pin dir and veth pair are released by their guards. **Evidence tier:** T3 (real aya maps; there is no Tier-1 backstop because `SimDataplane` does not model rollback). **Contract shape:** bounded-change; declared mutation set = the `BACKEND_MAP` entry of A; universe = `BACKEND_MAP`, `REVERSE_NAT_MAP`, the `SERVICE_MAP` slot and the dataplane trackers; assertion = the before/after equality in (2)-(5) | NEW | T3 | @error @degraded-resource @contract-shape:bounded-change | `overdrive-dataplane/src/service_apply_rollback_faults.rs` (in-crate, `#[cfg(all(test, feature = "integration-tests"))]`, declared in `lib.rs`), test `a_failed_in_place_backend_restore_during_rollback_is_returned_as_service_apply_rollback`; authored by the acceptance designer with the marker `#[ignore = "pending DELIVER selection-receipt-and-applied-identity"]`. Owner step 03-02. Split: the acceptance designer authors the decorator (test support) and the body, which must compile while ignored; so the behaviour-neutral crate-private `BACKEND_MAP` seam (the indirection and its real aya implementation, no new behaviour) lands as a DISTILL scaffold in that same change, and its internal shape is the author's call. The typed `BackendRestore` return already exists in `rollback_service_apply`; the test pins it, and 03-02 activates the test when it removes the pending marker | selection-receipt-and-applied-identity |
| S-PIG-133 | Refused upstream dials release their cleanup-ledger slot and registered intent. Contract: when the upstream dial is refused, the connector cleans the exact registered intent and completes its reserved ledger record, returns `GatewayUpstreamConnectError::Connect`, and leaves no residue, so more refused dials than the 128-slot ledger holds never make a later request fail with `CleanupLedgerFull`. Owner path: `GatewayUpstreamConnector::connect` (the connector's `connect_for`) over a real loopback `connect(2)` to a port that was bound and then closed, with the test-local dataplane and mTLS doubles of the neighbouring connector rows. Fault stimulus: 130 consecutive refused dials against the same connector and ledger. Outcome oracle: each attempt returns `Connect`; after each attempt the ledger length and `live_intent_count()` are 0; after the run `opens` and cleanup calls equal the attempt count, no receipt is taken, no handshake starts and `retry_all` reports neither saturation nor a retained cleanup. Evidence tier: Tier 1, default-lane-shape unit test in the connector's `acceptance` module (gated `integration-tests` like S-PIG-37 and S-PIG-53), test `refused_upstream_dials_release_their_ledger_slot_and_intent`. The socket-conversion failure arm after the receipt is consumed (same release, receipt consumed) is not reachable through a real socket and no seam is added: covered by inspection of `connect_for`. | NEW | U | @error @degraded-resource @contract-shape:bounded-change | `overdrive-gateway/src/connector.rs` (`acceptance` module) | upstream-socket-preparation |
| S-PIG-134 | A connect dropped mid-handshake releases its cleanup-ledger slot and registered intent (R-63). Contract: when the future of `connect_for` is dropped while suspended inside the gateway-client mTLS `authenticate(..).await` (or at the connect `select!`), the one private RAII cleanup guard still cleans the exact registered intent and completes the reserved ledger record, or, if the cleanup fails, records the failure on the record for `retry_all`; no manual per-arm release is relied on. Owner path: `GatewayUpstreamConnector::connect` (the connector's `connect_for`) over a real loopback `connect(2)` to a bound listener, with the test-local dataplane and the held mTLS double (`ScriptedMtls { hold: true }`) of the neighbouring connector rows. **Given** a connector whose selected backend is resolved and whose mTLS leg parks inside `authenticate`. **When** a connect reaches the parked `authenticate` (ledger length 1, one live intent) and its future is then dropped without the handshake being released. **Then** the ledger length and `live_intent_count()` are 0, no receipt is live, `opens` is 1 and the exact intent was cleaned once, `retry_all` reports neither saturation nor a retained cleanup, the upstream connection is closed with no byte sent. Fault stimulus: cancellation of the connect future at the held `authenticate`. Evidence tier: Tier 1, default-lane-shape unit test in the connector's `acceptance` module (gated `integration-tests` like S-PIG-133), test `a_connect_dropped_mid_handshake_releases_its_ledger_slot_and_intent`; authored and ignored `pending DELIVER upstream-socket-preparation`. Owner step 03-04 (the RAII cleanup guard replaces the manual per-arm `release_intent`). | NEW | U | @error @degraded-resource @contract-shape:bounded-change | `overdrive-gateway/src/connector.rs` (`acceptance` module) | upstream-socket-preparation |
| S-PIG-135 | The early answer to the last request a keep-alive connection may carry still lingers (R-14, R-10; user-approved 2026-11-07). **Given** a connection that has carried `max_requests_per_connection - 1` (99) ordinary requests. **When** the hundredth request is a `POST` declaring `Content-Length: 100000` whose origin answers 401 (and, in a second composition, 413) before the request body is read, and the client keeps sending 70,000 octets and then close_notify. **Then** the hundredth response is complete and carries `Connection: close`, and the port-call order on the public leg is the final `write_all`, then `close_write`, then exactly one `discard_remaining`, then the socket closes; it is never the plain close the cap alone would choose. Fault stimulus: the connection cap coinciding with an unread request body. Regression of the 04-01 review finding fixed in `218f113d9`: the lingering arm of the exchange conclusion takes precedence over the cap's plain close. Test `an_early_answer_to_the_last_permitted_request_still_ends_with_the_lingering_close`. | NEW | I | @error @boundary @contract-shape:bounded-change | `public_ingress_gateway_relay.rs` | kernel-relay-exchange-driver |
| S-PIG-136 | Answering a peer KeyUpdate is cancel-safe while the transmit lock is held (user-approved 2026-11-07; regression of `6d11ab373`). `read_through` is cancel-safe (R-37) and the exchange driver drops it. **Given** a public leg whose peer is not reading and a large `write_all` to the same leg that is stalled holding the transmit lock, and a peer that sent a KeyUpdate (requesting an answer, and in the control arm not requesting one) and then its next request head. **When** the `read_through` that consumed the KeyUpdate is dropped three times while the answer cannot be sent, then the peer reads, then a fresh `read_through` completes and the relay writes application data. **Then** the head is exact, the next DATA record decrypts at the peer, the peer received exactly one KeyUpdate answer when one was requested and none otherwise, and `TlsDecryptError` is unchanged; both AES suites. Fault stimulus: cancellation at the transmit-lock wait. Evidence tier: Tier 3 (native metal). Test `a_key_update_answer_survives_a_read_dropped_while_the_transmit_lock_is_held`. | NEW | T3 | @real-kernel @interruption @concurrency @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |
| S-PIG-137 | Answering a peer KeyUpdate is cancel-safe while the send buffer is full (user-approved 2026-11-07; regression of `6d11ab373`). The same contract as S-PIG-136 with the transmit lock free and the leg's send buffer full (an earlier large write to a non-reading peer was abandoned part way), so the answer's record send itself cannot complete; the read is dropped three times before the peer reads. Same oracle: head exact, next DATA record decrypts at the peer, exactly one answer when requested and none otherwise, `TlsDecryptError` unchanged, both AES suites. Fault stimulus: cancellation at the answer's record send. Evidence tier: Tier 3 (native metal). Test `a_key_update_answer_survives_reads_dropped_while_the_send_buffer_is_full`. | NEW | T3 | @real-kernel @interruption @contract-shape:bounded-change | `gateway_relay.rs` | kernel-relay-host-adapter |

**SVID issuance failure occurrence scenarios (R-45..R-52; the ids are the DESIGN obligations').** Each failure is driven through the production reconciler or executor; faults enter only at driven ports: a CA whose signing fails (`SigningFailsCa` / `ToggleCa`, test-local `Ca` doubles), and `IssueFaultStore` (a test-local delegating `ObservationStore`, `crates/overdrive-control-plane/tests/common/issue_fault_store.rs`) whose faults are scoped to the `issued_certificates` audit write (`Audit`), the workload failure-record operation (the unrecorded arm), the failure-row read, the gateway status row write and read, and the certified-key status read, and which counts the two status point reads the status handler can issue. The existing one-shot `inject_write_failure` consumes the next write of any family, so it cannot scope the fault; the decorator replaces it for these rows. The Sim clock supplies every instant.

| ID | Scenario contract | Disp. | Lane | Tags | Owning crate / target | Activating key |
|---|---|---|---|---|---|---|
| O-SIF-G1 | A gateway whose identity never issues reads `Absent` with the failure count, the latest cause and the Sim-clock instant; the count equals the failures ever recorded and never decreases across failures, successes and restarts; the executor records exactly one confirmed `Ca` or `Audit` failure per failed issuance and nothing on a success or stale-epoch success; the operator status body carries the cause label and instant and none of the injected error text. Tests: `a_gateway_whose_identity_never_issues_reads_absent_with_the_failure_count_and_cause`, `the_failure_count_never_decreases_across_failures_successes_and_restarts` (gateway integration); `a_ca_signing_failure_is_recorded_once_as_a_ca_cause_and_holds_nothing`, `an_audit_write_failure_is_recorded_once_as_an_audit_cause`, `a_successful_or_stale_issue_records_no_failure` (`action_shim/gateway_svid.rs`); `a_gateway_whose_issuance_keeps_failing_reports_the_count_and_cause_on_the_status_body` (CLI integration: the wire projection through `serve`; polling ceiling: gap 9b); the active `the_failure_class_of_an_issuance_error_is_its_variant_class` and `the_issue_failure_wire_body_is_the_cause_label_and_the_instant_only` | NEW | I (owner), U (executor), I (operator surface) | @error @property @security @contract-shape:bounded-change | gateway integration; `overdrive-control-plane`; `overdrive-cli` integration | gateway-svid-issue-failure-occurrence, gateway-svid-issue-failure-recording |
| O-SIF-G2 | After failures a success reads `Current` with the count and last failure retained; `Absent` with a count above zero is distinguishable from the fresh `Absent`, `0`, `None` (the recovery arm of the gateway never-issues owner test) | NEW | I | @lifecycle @contract-shape:bounded-change | same | gateway-svid-issue-failure-occurrence |
| O-SIF-G3 | Re-composing over the same durable store continues the count (`base + new`) and keeps the prior last failure until a new failure; a prior-row read error is retried, no row is written from a defaulted zero, and the row later carries `base + process`. Tests: `a_restarted_gateway_continues_the_failure_count_from_the_prior_row` (octet 182), `a_prior_row_read_error_is_retried_and_never_defaulted_to_zero` (octet 183). The undecodable-stored-row half is authored at the store: garbage under the pinned table and key reads as absent with exactly one `observation.row.skipped` warning emitted by the store with separate `table` and `key` fields (R-56). Tests: `an_undecodable_gateway_status_row_reads_as_absent_with_one_skipped_warning` (table `observation_gateway_application_status`, key the node id) and `an_undecodable_workload_issue_status_row_reads_as_absent_and_the_record_restarts_at_one` (table `observation_workload_svid_issue_status`, key the allocation id), both in `overdrive-store-local` integration `envelope_observation_skip.rs`. Gap 9a (closed by R-54 (g)): the owner's base-zero restart over an undecodable row is S-PIG-129, driven through the gateway `World` over the production `LocalObservationStore` (the Sim store has no byte layer; the seam is pinned in "Pinned: selection program, baselines and gates") | NEW | I | @error @degraded-resource @contract-shape:bounded-change | gateway integration | gateway-svid-issue-failure-occurrence |
| O-SIF-G4 | A node whose earlier enabled boot left a status row, rebooted over the same directories with the gateway disabled, answers `GET /v1/gateway/status` with the fixed body (`enabled=false`, no address, listener unbound, no application, no certified key), issues neither status point read (`gateway_application_status_row`, `public_certified_key_status_row`, counted per method) for the request, projects nothing from the stale row and writes no gateway status row. Test: `a_disabled_node_answers_the_fixed_body_without_reading_a_stale_row` (CLI integration, octet 175; the stale row is produced by the production enabled boot, not seeded) | NEW | I | @edge @contract-shape:unbounded-preservation | `overdrive-cli` integration | gateway-svid-issue-failure-occurrence |
| O-SIF-G5 | Held material whose `not_after` is not after the owner's clock projects `Unusable` at the next projection (the failure wake of a failing rotation) with the count above zero; a boot `ensure_current` timeout sets `GatewayIdentityUnusable` as the application cause and leaves `gateway_identity` as the slot says (`Absent` when empty, `Current` once held), with no failure recorded. Tests: `held_identity_past_its_expiry_projects_unusable_at_the_failure_wake` (octet 184), `a_boot_identity_timeout_sets_the_application_cause_and_leaves_the_identity_as_the_slot_says` (octet 185) | NEW | I | @edge @error @contract-shape:bounded-change | gateway integration | gateway-svid-issue-failure-occurrence |
| O-SIF-G6 | A failing owner status write loses nothing: the record stays in the slot and the next successful write carries the full count and the latest failure. Test: `a_failing_status_write_loses_nothing_the_next_write_carries_the_full_count` (octet 186) | NEW | I | @error @degraded-resource @contract-shape:bounded-change | gateway integration | gateway-svid-issue-failure-occurrence |
| O-SIF-G7 | The gateway row `FIXTURE_V1` is regenerated for the layout with `issue_failures` and `last_issue_failure` (R-51) and the row round-trips every occurrence through the envelope. Tests: `gateway_application_status_v1_decodes_through_current_envelope`, `the_row_round_trips_every_occurrence_through_the_envelope` (**active**) | KEEP+NEW | U | @property @codec @contract-shape:pure-function | `overdrive-core/tests/schema_evolution` | none (active) |
| O-SIF-W1 | Driven by the production `svid-lifecycle` reconciler over a Running allocation: a first-issue `Ca` failure, a first-issue `Audit` failure and, after a success, a `rotate-svid` reissue failure give a count that rises by exactly one each time and the latest cause and Sim-clock instant, through the store and through `GET /v1/allocs`; a success never clears them; failures leave no audit row and the previous leaf stays held; a failing record write is surfaced (the tick returns the shim error), not counted, and the next failed attempt records. Tests: `repeated_failed_issuances_give_a_monotone_count_and_the_latest_cause_on_the_allocations_surface`, `a_failing_record_write_is_surfaced_and_not_counted_and_the_next_failure_records` (control-plane acceptance); the operator line is O-SIF-W3 | NEW | U | @error @contract-shape:bounded-change | `overdrive-control-plane` acceptance `svid_issue_failure_occurrence.rs` | workload-svid-issue-failure-occurrence |
| O-SIF-W2 | `record_workload_svid_issue_failure` is the same operation in `LocalObservationStore` and `SimObservationStore` (count rises by one, latest failure, logical counter rises from one, no row for an allocation that never failed, no `alloc_status`, `issued_certificates` or `node_health` row, no event) and is atomic under concurrent calls on one allocation (32 simultaneous calls give a count of 32, one barrier round per adapter). Tests: `the_failure_record_is_the_same_operation_in_the_local_and_sim_stores`, `concurrent_recordings_on_one_allocation_never_lose_a_count` (`overdrive-store-local` acceptance); and, on the redb store, `an_undecodable_workload_issue_status_row_reads_as_absent_and_the_record_restarts_at_one` (integration `envelope_observation_skip.rs`: garbage under `observation_workload_svid_issue_status` at the allocation id reads as absent with one store-emitted `observation.row.skipped` warning (separate `table` and `key` fields, R-56) and the next record gives a count of one) | NEW | U | @property @adapter-contract @concurrency @contract-shape:bounded-change | `overdrive-store-local/tests/acceptance/workload_svid_issue_status.rs` | workload-svid-issue-failure-record |
| O-SIF-W3 | `render::workload_describe` over a snapshot with `svid_issue_failures > 0` prints the pinned `svid issuance failures: <N> (last: <cause> at <at>)` line once, under a Service row and a Job row, for both causes, and the whole output equals the failure-free render plus that line; with zero it is byte-identical to before (the existing golden test `non_pending_allocations_render_byte_identically`). Test: `a_row_with_failed_issuances_renders_the_count_and_the_latest_cause` | NEW | U | @contract-shape:pure-function | `overdrive-cli/tests/acceptance/render_workload_describe.rs` | workload-svid-issue-failure-occurrence |
| O-SIF-W4 | An observation read error on the failure row is the allocations handler's error, never a zero; the next healthy read serves the recorded count. Test: `a_failure_row_read_error_is_the_handlers_error_and_never_a_zero` | NEW | U | @error @contract-shape:bounded-change | `overdrive-control-plane` acceptance | workload-svid-issue-failure-occurrence |
| O-SIF-W5 | The new workload row envelope's `FIXTURE_V1` decodes through `WorkloadSvidIssueStatusRowEnvelope` into the canonical payload, and the row round-trips every recorded occurrence through the envelope. Tests: `workload_svid_issue_status_v1_decodes_through_current_envelope`, `the_row_round_trips_every_occurrence_through_the_envelope` (`workload_svid_issue_status_row.rs`; both **active**) | NEW | U | @codec @contract-shape:pure-function | `overdrive-core/tests/schema_evolution/workload_svid_issue_status_row.rs` | none (active) |

Conformance (QC): QC-1 `gateway_identity` (5 bound runs), QC-2 `gateway_application` (3), QC-3 `selection_receipt` (3 bound over `receipt_conf`), QC-4 `relay_exchange` (7 over `relay_conf`) = 18 `#[quint_test]` plus one `#[quint_run]` per driver (4); see Scaffolds. EDD: E15 operator journey; point-in-time, never a test.

**Restated C4a — proof of the D2 replacement without fabricated demand.** There is no demand, revision, acknowledgment or wake to apply twice. The obligation is the consuming-port form (obligation 1): with no healthy backend, two consecutive public requests both return 503 with identical bodies; after each request `live_intent_count() == 0` and `live_receipts()` is empty; `GatewayApplicationStatusRowV1` and the Gateway Applied Generation are unchanged; the outcome is a typed `NoBackend` bounded by the request deadline and never a wait. Proof: (i) Tier 1 (I): production `GatewayBuilder` over real redb Route and custody, `SimGatewayConnectDataplane` sharing the `SimDataplane` state with no `update_service` ever issued, so the receipt is the ordinary "no outer entry" `NoBackend`; a client socket through `SimGatewayRelay` (the Sim terminates the modelled public handshake; real TLS is only the T3/N arm) issues the request twice; the oracle reads only public bytes/status, `live_intent_count()` and `live_receipts()`, `GatewayStatusReads::application_status()` (before equals after), and the unchanged injected `SimClock` (no sleep or poll); the Sim arm has no tracing capture seam, so the `gateway.upstream.receipt.validated` occurrence is not an oracle item here (it is read only where the end-to-end tracing capture layer exists, AT-PIG-E2E-1); (ii) the real hook's `NoBackend` 503, before the real `ServiceMapHydrator` programs a backend, is observed on the native-metal arm of S-PIG-71; S-PIG-02 has no native-metal arm. No receipt is scripted and no row is seeded. The honest limit: the Sim arm is the modelled hook; only T3/N prove the real hook.

Walking-skeleton specification (S-PIG-01, `AT-PIG-E2E-1`):

    Given overdrive serve (the production composition) has a complete manual Public Certified Key
      and the operator has deployed the checked-in api VM Service and the public-api Route
    When an external user requests https://api.example.com/stream over TLS 1.3
      using the Route hostname as SNI and Host (client-local address override only)
    Then the user receives status 200, X-Workload api, and the streamed body
      and the receipt occurrence shows a Selected backend whose applied identity is the api allocation
      and the leg-B/leg-C segment carries TLS 1.3 records with no body cleartext
      and both gateway legs are kernel-TLS armed while the body streams
      and Route, Service, gateway intent, receipt and identity state converge on cleanup

## Wave: DISTILL / [REF] Cross-boundary scenario obligations

Per `.claude/rules/testing.md` ("DISTILL — prove composed system behavior under deliberate failure") each cross-boundary scenario records the production owner path, the driving stimulus, the fault (existing driven ports only; none for healthy controls), the oracle and the evidence lane. Single-owner scenarios are pure, source-local or single-owner and are not repeated here; their lane and contract are in the scenario list: S-PIG-03–10, 12–20 (pure or clap-only), 25 (one status projection read), 33 (one store key, reopen), 37 (pure ledger), 44, 45, 47 (pure codec), 49 and 53, 55, 73 (source-local or single owner), and the G1 pure rows 76–82, 84–86 (pure functions or the one-owner `ExchangeMachine`).

**Oracle rules (apply to every row).**
1. *Membership-only on Sim.* A Sim-backed oracle on a generation with N>1 backends asserts only that the selected `BackendId` (and its applied identity) belongs to the committed generation; an exact `BackendId` is asserted only for singleton generations. The real Maglev choice is proven only at Tier 3.
2. `NoBackend` caused by a missing `BACKEND_MAP` entry (outer entry and slot present, backend id absent) is observable at Tier 3 only; the Sim cannot produce it from the `update_service` input.
3. The `healthy` byte is not evaluated by the Sim or by the real arm (R-2); no oracle depends on it. The ~1 s window before the first readiness `pass` is a failed dial (502), observed at Tier 3 / native only.
4. The transient receipt oracle is `live_intent_count()`/`live_receipts()` post-state, never map polling; the connector's `gateway.upstream.receipt.validated` occurrence is additionally read only in AT-PIG-E2E-1, the one place a tracing capture layer exists (P-36). Layers 3 and above are example-based.

| ID | Production owner path | Stimulus | Fault | Oracle | Lane |
|---|---|---|---|---|---|
| 01 | `run_server` (Host adapters) -> `ServiceMapHydrator` -> `EbpfDataplane::update_service` -> gateway runtime -> `open_upstream` -> hook -> leg-C -> kTLS -> `GuestStreamDial` -> vsock -> VM Service | External TLS 1.3 client requests `https://api.example.com/stream` after `deploy` of the Service and Route | none (healthy control) | 200, `X-Workload: api`, streamed body; receipt occurrence `Selected` with applied identity = api allocation; leg-B/leg-C segment carries TLS 1.3 records, zero body cleartext; the public leg and the upstream leg are kernel-TLS armed during the stream (`ss -tni` ULP, `tls_stat` install deltas); the relay probe occurrence precedes the bind; after cleanup Route/Service/intent/receipt/identity state converged (live counts 0) | N |
| 02 | Public listener -> connector -> `open_upstream` -> hook/Sim arm -> receipt | Public request for a Route whose Service has no programmed backend | I: no backend is programmed, asserted before the request | 503, no fallback body; `SimClock` unchanged by the request (no sleep or poll); live counts 0; `NoBackend` occurrence with selected-only fields absent | I |
| 21-23 | `overdrive-gateway` Route Set owner -> `LocalIntentStore` (real redb) -> `GET /v1/gateway/status` projection | Declare, exact replay, replace (same Route ID, new generation), withdraw; declare a different Route ID while occupied (22); withdraw with a different Route ID (23) | none (healthy control and deterministic conflicts) | Outcomes `Declared`/`Unchanged`/`Replaced`/`Withdrawn` exactly; the persisted singleton slot holds the one Route (empty slot persisted, not deleted); conflicts return the cause-distinct typed slot-conflict error with no `RouteAccepted` fact and the whole universe (Route Set, Service, key, identity, BPF) equal; typed error kept apart from the redacted status projection | I |
| 24 | Two concurrent Declare commands for different Route IDs through the exported handler's Route Set owner over one real redb singleton key | Two simultaneous declares with distinct Route IDs released together; 256 rounds, each on a fresh store, seed printed | none (race is the stimulus; no seam) | exactly one `Declared`, the other the typed slot conflict; the store holds exactly the winner's Route; exactly one `RouteDeclared`/`RouteAccepted` pair; no interleaved partial Route | I |
| 26 | `ServerHandle` shutdown stage `gateway` -> join users -> sweep -> slot clear | Graceful shutdown with one admitted connection | none | identity slot empty only after the user joined; live counts 0; stage events ordered | I |
| 27 | Boot `ManualCertifiedKeySource::probe` -> custody (`PublicCertifiedKeyAeadCodec`, protected redb record) -> `ServerHandle` boot | Boot with gateway flags naming unreadable (permission) and malformed (not PEM / mismatched key) material files | unreadable file and malformed content on the real filesystem (production-reachable inputs) | boot refused with `health.startup.refused` and a cause-distinct reason per case; the Route Set, custody record, identity and BPF state equal to pre-boot (no Route mutation); error text and status carry no path contents or key bytes | I |
| 28-30 | Connector -> `open_upstream` / `take_receipt` / `cleanup` at the dataplane port | Public request | Typed port results at those operations: receipt missing; receipt for another `ServiceKey`; `Selected` peer valid but not the selected one | 502; `cleanup` invoked once; one attempt only (no second backend); no body byte sent upstream | U/I |
| 34 | `deploy_resource` (CLI) -> `ApiClient` -> production `POST /v1/routes` handler -> Route Set store | `deploy <ROUTE_SPEC>` with and without detach | none (healthy control) | one-shot outcome identical for both detach modes (201 `Declared`, 200 `Replaced`/`Unchanged`); no streaming wait; Route Set holds the Route | I |
| 35 | `ApiClient::{submit_route, withdraw_route, gateway_status}` (R-53) -> production `POST`/`DELETE /v1/routes` and `GET /v1/gateway/status` handlers -> store and `ObservationStore` | Declare, conflicting declare (409), invalid id (400), delete (200 `Withdrawn`/`Unchanged`), status read | none | exact HTTP statuses per case; typed `ApiClient` error kept apart from the redacted status body; Route Set and status row match the outcome | I |
| 36 | `deploy_resource` -> `ApiClient` -> handler of a composition with `ServerConfig.gateway == None` | `deploy <ROUTE_SPEC>` including an unparsable spec; and `ApiClient::withdraw_route` with a valid id and with `Not A Valid Route Id` (R-53) | gateway disabled by composition (not a seam) | rejected before the spec or id is parsed with the typed disabled-gateway error (409 `gateway_disabled`); Route Set and intent store untouched | I |
| 31 | Gateway application owner under seeded `overdrive-sim` | Route replace/withdraw interleaved with connection open/close and restart | `HoldConnectProbe` / `ReleaseConnectProbe`; restart | invariant `GatewayApplicationGenerationLifecycleIsSafe` holds; seed printed on failure | S |
| 32 | Same, shutdown | Shutdown with admitted users | `HoldConnectProbe` | invariant `GatewayApplicationShutdownConverges`; users joined before slot clear | S |
| 42 | `GET /v1/gateway/status` handler -> `ObservationStore` typed point reads (`public_certified_key_status_row`, `gateway_application_status_row`) -> `ApiClient`/CLI | Status request | point-read failure injected through `SimObservationStore` | HTTP 500 with a fixed redacted body; no key, path, ciphertext or error source text; subsequent healthy read returns the normal projection. The CLI test drives the status handler through `ApiClient::gateway_status` (R-53); no CLI status command is pinned, so a CLI-output clause is not asserted | I |
| 38, 43, 52 | `HostGatewayClientMtlsFactory` handshake against a real rustls peer | Gateway client dial to peer A | Selected A but peer B presents a valid SVID; absent/expired/malformed identity; non-peer fd; elapsed handshake; wrong URI-SAN shape | handshake error before any body byte reaches the peer and nothing is armed on the socket; on success (S-PIG-64) the peer observes the dedicated gateway SVID | I |
| 39, 40 | Public listener over `SimGatewayRelay` (`public_ingress_gateway_relay.rs`); the connector for the single attempt | Request outside the Route (route miss); a selected upstream that closes before the response head | scripted origin close_notify or reset | 404 with no upstream leg opened; exactly one 502, upstream leg closed; one attempt and no retry (connector tests) | I |
| 41 | Real cgroup hook, unregistered socket | `connect(2)` from an unregistered socket | none | behaviour identical to main; `LOCAL_BACKEND_MAP` and XDP attachments unchanged (expected GREEN) | T3 |
| 46 | Boot custody ingestion (`ManualCertifiedKeySource` files -> `PublicCertifiedKeyAeadCodec` -> protected redb record) -> `GET /v1/gateway/status` | Install a key; replay the identical files; replace with a valid new generation; present an invalid replacement and an expired replacement; reopen | invalid/expired/stale-generation material on the real filesystem | `Installed`/`Unchanged`/`Replaced` exactly; invalid or expired replacement leaves the current generation and record intact and surfaces only a cause-distinct redacted install status; expiry derived from persisted inputs plus `SimClock`; reopen yields the same current generation | I |
| 48 | `LocalIntentStore::watch` (real redb) -> `IntentSubscriptionEvent` consumer | Subscribe, then force more committed deltas than the broadcast capacity while the consumer is held, then one further delta | broadcast lag (production-reachable overflow) | the consumer receives `Lagged` carrying the exact missed count, then the next `Changed`; complete body under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]` (the type/signature port landed with item 12, reconfirmed 2026-11-02, R-18) | I |
| 50 | Each dataplane port operation -> connector -> status | Public request or boot probe per operation | each `GatewayConnectDataplaneError` variant at its owning operation | status carries only redacted `ConnectPathUnavailable`; no variant text leaks | I |
| 51 | `GatewayIdentityView` / `ensure_current` -> status (shutdown has no identity-error projection, R-30) | Boot | each `ensure_current` lifecycle error | cause-distinct at the owner, `GatewayIdentityUnusable` in status | I |
| 54 | Public request path, twice (C4a), carried by the S-PIG-02 I arm run twice | Two consecutive public requests, no backend | none (no `update_service`) | both 503 with identical bodies; counts 0 after each; status row and Applied Generation unchanged; `SimClock` unchanged | I |
| 56 | Real hook (T3: `overdrive-dataplane` hook file); N: real hook + node-shared nft output rule | T3: dial to a Service whose only backend is unhealthy and dead; N: public request after `job stop` while the map entry lingers | none (production leaves the entry, P-4) | never a wrong-peer success, the dial fails; the arm ignores `healthy` (R-2), so the receipt is `Selected` (T3) and the public status is exactly 502 (N); the reset at the nft output rule for a stopped backend whose tuple is gone is observed on the N arm only | T3+N |
| 57 | Boot G0-G4 -> sentinel probe | Boot with enabled gateway | I: sentinel probe fault through the Sim composer (authored, ignored; `Unavailable` at probe is the stimulus); T3: the real hook cannot be made to fail without a seam, so the T3 case is the passing sentinel probe (residue 0) and a probe whose deadline has already elapsed (`ProbeTimeout`) | TCP/443 unbound; serve and workloads unaffected; status cause; `health.startup.refused` reason `gateway.connect_probe` (the relay probe refusal is row 100) | I+T3 |
| 58 | Boot -> actual-frontend probe (R-65) | (a) boot with Service present, no backend (asserted before the request); (b) boot with a backend programmed on the shared `SimDataplane` state and the Sim client mTLS refusing that backend (`GatewayClientMtlsError::Handshake`) or its applied identity missing; (c) boot with the same refusing backend, then a Route replacement | (b) and (c): the selected backend's exact-peer mTLS fails | (a) listener binds; public request returns 503. (b) listener binds, TCP/443 is bound, `GET /v1/gateway/status` shows no gateway failure cause, `health.startup.refused` is not emitted, no `authenticate` call occurs before the first public request, and that request returns exactly 502 with one `open_upstream` and no retry. (c) the replacement is promoted and drains the prior generation as without a backend; allocation, Service and hydration advance identically | I |
| 59 | `run_server` with `ServerConfig.gateway == None` | Boot | none | bound sockets, reconcilers, intent/receipt counts and stage events equal the pre-feature set; unregistered hook behaviour unchanged (expected GREEN) | I |
| 60 | `ServerHandle` boot and shutdown | Enabled boot, graceful shutdown | none | order G0-G4 with the relay probe before the connect probe inside the gate task; stage `gateway` before `mtls_worker_owner`; one 5 s grace; one budget (drain ends at 4.5 s, the final owner joins end by 5 s, R-64); in-flight pipes closed with their exchange | I (+N) |
| 61 | Gated gateway -> serve lifetime | Gate the gateway (relay probe refused through the Sim relay substrate lie); request shutdown | gated gateway; no gateway-task-failure stimulus (no driven-port fault fails the task itself) | serve shutdown not requested while the node ticks; shutdown completes inside the unchanged 10 s fail-stop bound | I |
| 62 | Allocation, Service, hydration and operator listener | Boot enabled, disabled, gated | gated gateway | Running, Stable, hydration and operator listener advance identically in all three | I |
| 63 | Real hook (T3: `overdrive-dataplane` hook file); N adds table 100 -> TPROXY -> leg-C | Dial from a registered prepared socket | none | receipt `Selected` bound to the cookie and `ServiceKey`, applied identity of the backend; the prepared socket carries `SO_MARK` = `MTLS_TPROXY_MARK` and a `127.0.0.1` bind; the rewritten dial reaches the registered backend address from `127.0.0.1`; the dial entering leg-C through the node-shared nft/table-100 rules is observed on the N arm | T3 (+N) |
| 64 | Dial segment through a recording TCP proxy to a real rustls peer on loopback; kTLS | Gateway dial built with a gateway SPIFFE-identity leaf from the test PKI | matching gateway identity (the wrong-valid, expired and absent negatives are S-PIG-38/43/52) | the peer observes the gateway SPIFFE identity; the proxy records TLS 1.3 `application_data` records after the hello flight and no body marker in cleartext; `getsockopt(TCP_ULP)` on a duplicate of the dialed descriptor names `tls` | T3 |
| 65 | `GatewaySvidLifecycle` -> action shim -> CA -> audit -> slot | Reconcile ticks over issue, near expiry, stale epoch, disable (synchronous clear), restart | stale-epoch completion; audit write fault | hold only after audit; stale epoch is a no-op; reissue on near expiry and restart; no allocation identity | U+I |
| 66 | Route/custody/application watchers over the intent store | Declare/withdraw while the watch lags | lag event | subscribe before list; relist on `Lagged`; admission closed meanwhile (ignored until DELIVER) | I |
| 67 | `EbpfDataplane` vs `SimDataplane` same call sequence | `update_service` sequences with success and rejection | conflicting backend set (real `BackendIdentityConflict`) | identity readable only after the outer swap; immutable per id; no residue; membership-only on N>1 | I+T3 |
| 68 | Gateway application owner | Supersede a generation while its gate is in flight | `HoldConnectProbe` (parks at an awaited operation the gate owns, DG-9, R-65; no backend is programmed), the superseding Route consumed by the owner, then `ReleaseConnectProbe` | late result never delivered; old admission object never restored | I |
| 69 | Public request -> guest-flow owner | Request in flight | guest-flow owner restart or fail-stop | 502; no intent or receipt left (counts 0) | I+N |
| 70 | Public listener status mapping | the authored cases: 408 stalled head, 408 stalled request body (head sent, then no body octet for `body_no_progress_deadline`); the obligation-8 statuses 404, 502, 503 are carried at the listener by the relay-file tests of S-PIG-39/40 and the S-PIG-02 I arm and 400/421 by S-PIG-82 and the S-PIG-77/78 listener arms, not authored in this row. 413, 400, 421, 431 and 505 reaching a real client on native metal are the S-PIG-115 native test; 400/421/431/505 are pure-function S-PIG-82 and the 400, 505, 502 listener arms of S-PIG-77/78 (whether 400/421 need a VM-level case: P-35) | the corresponding upstream or input condition | exact status per case | N |
| 71 | `EbpfDataplane` re-creation on the same pin dir; `run_server` restart; ordinary hydrator | Serve restart, then public requests | process/instance restart (production path, no seam) | T3 deterministic: `selected_backend_identity(old id)` missing and receipt count 0 before any `update_service`; N: any response observed while the Service map shows no outer entry (read with `bpftool map dump pinned /sys/fs/bpf/overdrive/SERVICE_MAP`) is 503 (safety), then the backend response within the 60 s recovery window (liveness) | T3+N |
| 72 | `Dataplane::update_service` racing `selected_backend_identity` on the real owner | 2,000 rounds of concurrent updates and identity reads for a just-selected id | none (no pause seam) | for every id from a `Selected` receipt the read never returns missing; rounds and seed printed | T3 |
| 74 | Boot G0-G4 via `SimDataplaneComposer` | Boot with enabled gateway | (a) the Sim connect dataplane's typed `ReceiptMissing` at the `probe` operation (typed `Unavailable` at `probe`, the registered probe yielding no capability result, is the stimulus of row 57); (c) the gateway's SVID is refused by a delegating CA at the existing `CaAdapters.factory` port, so the Gateway Identity never becomes Current; (d) typed `ProbeTimeout` at `probe` (the typed result at the existing operation; a hold is DISTILL Sim test support, DG-9, and is not the stimulus). Typed results at existing operations only | per case: TCP/443 refused (unbound); `GET /v1/gateway/status` shows `ConnectPathUnavailable` for (a), (d) and `GatewayIdentityUnusable` for (c); `health.startup.refused` reason `gateway.connect_probe` for (a), (d) and no probe reason for (c); operator listener and allocation `Running` unaffected | I |

**G1 cross-boundary rows (kernel relay).** Same rules and the same oracle rules above. Contract and invariant are the DESIGN names (Contract of the legs, Message-framing contract, Exchange state machine, Quint module `relay_exchange`). Faults enter only through the existing driven ports: for Tier 1 the `SimGatewayRelay` peer events and substrate presentation described in Scaffolds (test support shaped by DISTILL; no production surface), for Tier 3 a real rustls peer pair on loopback driven by the test. Lane codes as above; rows 104–112, 114, 115, 117, 118 and 123 run only on the native metal host; rows 119–122 run in the default `integration-tests` lane on the Sim relay.

| ID | Contract / invariant | Production owner path | Stimulus | Fault | Oracle | Lane |
|---|---|---|---|---|---|---|
| 83 | Interim heads: `100` forwarded, other `1xx` dropped, `101` is 502 (R-11) | public listener -> exchange driver -> `read_through` on the upstream half | origin script sends `103`, `100`, then the final head | none | client receives exactly the rewritten `100` head then the final response; `103` absent; same stimulus with `101` is a 502 | U+I |
| 87 | `NoBytePastMessageBoundary`; keep-alive reuse only at a clean boundary (relay_exchange `exchange_cl_keepalive`) | listener -> exchange driver -> `relay_exact` -> `close_write` per exchange | two requests on one public connection, `Content-Length` 0, 1, 65 535, 65 536, 65 537 (the pipe capacity is 65 536) and PBT over sizes at layer 2 | none (healthy control) | each body octet-exact at the origin script and the client; second request head intact; one upstream leg per exchange, none reused; `finish()` disposition `Reuse` observed through the second head being read | I |
| 88 | chunk-per-chunk relay | same | chunked request and chunked response with extensions, trailers, and a split size line | none | sizes written by the gateway, octets exact, extensions and trailers absent upstream and downstream, last chunk written by the gateway | I |
| 89 | bodyless message rules (S-PIG-76) | same | `HEAD`, `204`, `205`, `304` responses (some carrying `Content-Length`/`Transfer-Encoding`) | none | no `relay_*` call for a body; connection reusable | I |
| 90 | control records never forwarded; each leg rekeys alone; close_notify ends one direction | same, Sim relay peer events | KeyUpdate (requested and not), NewSessionTicket, close_notify, alert, unexpected record at seeded offsets: inside a head, a chunk-size line, a boundary, a body | scripted peer events; a KeyUpdate whose answer fails (`KeyUpdateError::RustlsRefused`, `RearmRx`, `RearmTx`, `Io`) on either leg | relayed octets exact; no control event visible on the other leg; alert or unexpected record closes both legs; `read_through` partial head retained across the event; a failed KeyUpdate answer: before the public response head exactly one 502, after it both legs close with no further response, nothing is relayed past the failure and the end is never clean | I |
| 91 | close-delimited end (relay_exchange `close_delimited_clean_end`, R-12) | same | origin response without length that ends in close_notify | peer close_notify at end; over-ceiling variant | forwarded with `Connection: close`; clean end only on close_notify after the head; not reused; over `max_response_body_bytes` closes both legs | I |
| 92 | upstream close before head / mid-body (relay_exchange `upstream_close_mid_body_aborts`) | same | origin closes before the head; mid length body; close_notify before declared end | scripted | pre-head: exactly one 502; post-head: both legs closed, no further response, never a clean end | I |
| 93 | early response then lingering close (relay_exchange `early_response_closes`; R-14, R-10) | same | origin answers 413/401 while a large request body is unfinished; each gateway rejection listed in S-PIG-93 with a body sent after | origin early answer; client keeps sending | complete response delivered; port-call order `close_write` then `discard_remaining` then close; the SimClock-bounded discard never extends past its bound; 408 and EOF/alert/reset legs make no `discard_remaining` call (only the 408 and mid-request-body arms are executable) | I |
| 135 | early response on the last permitted request then lingering close (R-14, R-10) | same | the hundredth request of a connection is a `POST` with `Content-Length: 100000`; the origin answers 401 (413 in a second composition) before the body is read; the client keeps sending | origin early answer; client keeps sending; connection cap reached | complete response with `Connection: close`; port-call order `write_all` then `close_write` then one `discard_remaining` then close | I |
| 94 | body ceilings | same | declared and chunked requests at, 1 above the ceiling; response above its ceiling | none | 413 before any upstream octet; chunked overflow 413 iff no public response octet sent else both legs close; response overflow closes after head | I |
| 95 | pipe floor (typed `RelayPipeError`) | same | any body relay | `SimGatewayRelay` pipe creation failure / read-back below floor | 502 before head; both legs close after head; no octet moved | I |
| 96 | deadline composition and keep-alive idle (obl 17) | same on `SimClock` | idle past `keep_alive_idle_deadline`; stalled first head; stalled later head; hundred requests | `SimClock` advance | silent close vs 408 as specified; the hundredth response `Connection: close`; 101st never parsed | I |
| 97 | single attempt (ADR-0240, 0232) | listener -> connector -> `open_upstream` -> client mTLS | each failure of rows 92, 95, 98 | typed faults | request-count marker 1; one `open_upstream`; no second dial; upstream head has `Connection: close` | I |
| 98 | arming only after equality; failed equality arms nothing | connector -> `GatewayClientMtls::authenticate`; public side: `accept_public` | selected peer A; a public client handshake | Sim arm failure of each `KtlsArmError` kind (`UlpInstall`, `CryptoInfo`, `CipherNotSupported`, `KernelConnection`) after a matching peer, and at `accept_public`; separate mismatch case | `Arm(kind)` surfaces unchanged at the connector (the listener-level oracle is the 502, no octet at the peer, cleanup once, no residue); no octet at the peer; cleanup once; public 502; at `accept_public` the public socket closes without any HTTP octet read and no upstream leg opens | I |
| 99 | Earned Trust of the relay (ADR-0244/0246) | boot gate task -> `GatewayRelay::probe` | boot with `SimGatewayRelay` presenting each substrate lie | `ulp_absent`, `cipher_refused`, `splice_inexact`, `pipe_clamped`, `probe_timeout`, `probe_io` | `probe` returns the matching `RelayProbeError`; the honest substrate returns `Ok`; no variant maps to two lies | I |
| 100 | G-1 relay input; boundary 7 | `boot_server` G3/G4 -> gate task | boot enabled with an honest and with a lying Sim relay; replace the Route after boot | substrate lie | TCP/443 unbound; `health.startup.refused` reason `gateway.relay_probe`; status `ConnectPathUnavailable`; probe call count 1 across the replacement; operator listener, allocation, Service and hydration advance identically; disabled gateway: zero relay-probe calls | I |
| 101 | G-6 drain; relay_exchange `drain_with_inflight_body` | `ServerHandle` -> `GatewayHandle::shutdown` -> exchange driver | drain with idle connections and one in-flight exchange at each phase | drain deadline on `SimClock` | idle closed at once; in-flight completes or is force-aborted at the drain deadline (`grace - 500 ms`, R-64); no request parsed after drain; zero residue; discard (R-14) ends at the drain deadline; owner joins finish inside the reserve with no `Timeout` task failure. Oracle for the discard-during-drain test: `report.elapsed` in `[GRACE - 500 ms, GRACE + one clock step]` (was `[GRACE, GRACE + 1 s]`), `forced_connections == 0`, `task_failures` empty | I |
| 102 | seeded exchange invariant | exchange driver over `SimGatewayRelay` | seeded schedules of scripted events | KeyUpdate, NewSessionTicket, close_notify, alert, reset, upstream close before head | no octet past a boundary; no reuse after drain; zero leases, pipes and legs after every outcome; seed printed | S |
| 103 | seeded drain/force-abort | same | drain at a seeded point | deadline expiry | exchange completes or aborts, never reused, no residue; twin runs equal | S |
| 104 | host/Sim equivalence of `GatewayRelay` | `HostGatewayRelay` vs `SimGatewayRelay` through the port | the same byte and event scripts | producible faults only (peer events, deadlines, short counts, backpressure, discard (R-14), `relay_to_end` (R-12)) | identical returned octets, error variants and close behaviour; substrate lies excluded | T3 |
| 105 | ADR-0239/0244/0246 byte-exact relay; keep-alive | `HostGatewayRelay` between two kernel-TLS legs; N: full production path | `Content-Length` and chunked bodies over two exchanges; around the 64 KiB capacity (cap-1, cap, cap+1, 2cap, 2cap+1) | records of mixed sizes straddling heads and chunk-size lines | octet-exact at both rustls peers; next head intact; no loss at any capacity-boundary length | T3 (+N) |
| 106 | control-record contract (A-K4); no gateway-originated KeyUpdate (R-13) | `read_through`/`relay_exact` on the read half | KeyUpdate (requested and not) in head, chunk-size line, boundary and body | peer KeyUpdate; negative control: a test-local raw kTLS receiver that does not re-arm | exact octets; `TlsRxRekeyReceived`, `TlsRxRekeyOk` advance by the number sent, `TlsTxRekeyOk` by the number requested; `TlsDecryptError` unchanged; nothing forwarded to the other peer; the control receiver reads `EKEYEXPIRED` | T3 |
| 107 | transmit lock (A-K5; no model, R-19) | `relay_exact` splice-out vs the read half's reply | KeyUpdate request arrives while a large splice-out is in flight | timing from a stalled receiver | peer decrypts every octet, `TlsDecryptError` unchanged, reply after the in-flight write | T3 |
| 136 | cancel-safe KeyUpdate answer (`read_through` is cancel-safe, R-37); transmit lock held | `read_through` dropped, `write_all`, then a fresh `read_through` | peer KeyUpdate (requested, and not requested) then a request head, while a large `write_all` holds the transmit lock | stall: the peer does not read; the read is dropped three times | head exact; the next DATA record decrypts at the peer; peer KeyUpdates received = 1 when requested else 0; `TlsDecryptError` unchanged | T3 |
| 137 | cancel-safe KeyUpdate answer; send buffer full, lock free | same | same, with an earlier write to the non-reading peer abandoned so the send buffer is full | the answer's record send cannot complete; the read is dropped three times | same as 136 | T3 |
| 108 | NewSessionTicket / close_notify handling | `read_through`, `relay_exact` | tickets before the first upstream response; close_notify mid-head, mid-body, after response | peer close_notify | ticket absent downstream; `Truncated`/`SourceEndedMidBody` mid-message; `CleanEof` only at a boundary | T3 |
| 109 | A-K7 | upstream read half; N: production leg-C | injected KeyUpdate at the upstream leg; a 16 MiB response through production leg-C | injected peer KeyUpdate | answered with `TlsDecryptError` unchanged; production run: `TlsRxRekeyReceived` delta 0 across the response (a non-zero delta is a deterministic hard failure that falsifies A-K7 and is returned to DESIGN; it is neither a pass nor inconclusive) | T3 (+N) |
| 110 | one pipe rule (ADR-0246, R-16) | public read half | body relayed, then the half waits | none | descriptor count returns to the pre-body value once the next head read starts, for `Content-Length` and chunked | T3 |
| 111 | backpressure bounded memory; progress deadlines | `relay_exact` | 256 MiB stream to a stalled reader; trickled `Content-Length` response (single-octet pieces) (the stalled body is the S-PIG-104 equivalence case) | stalled reader | in-flight octets <= socket buffers + 2x pipe capacity (read at run time); RSS growth <= 4 MiB; each trickled piece reaches the client within one `no_progress` without a following piece | T3 |
| 112 | A-K6 record boundary | `relay_exact` | records of 1 to 16 384 octets, heads and chunk-size lines straddling records | mixed sizes | octet-exact; splice-in only into an empty pipe (the private pipe is not observable from outside; emptiness at every splice-in is asserted through exactness over repeated bodies) | T3 |
| 113 | descriptor/memory budget (Budgets) | production composition (`run_server`) | 128 concurrent streaming requests | process `RLIMIT_NOFILE` (reported, not raised) | peak `/proc/self/fd` count <= baseline + 768; resident-set growth plus the capacity (`F_GETPIPE_SZ`) of the process's pipe descriptors <= 24 MiB (128 x 192 KiB, the DESIGN accounting: 2 x 32 KiB head accumulators and 2 x 64 KiB pipes per exchange; no allowance for connection state, a larger figure falsifies the budget and is returned to DESIGN); the 129th concurrent connection is closed before any TLS octet is read or written (`max_public_connections` = 128); after drain the count returns to baseline | N |
| 114 | Earned Trust of the real kernel (R-23) | `HostGatewayRelay::probe` | probe on the honest kernel; no substrate lie is produced on the real kernel (each lie is refused at Tier 1 by S-PIG-99) | none | `Ok`; the loopback pair and pipe are released (descriptor count) | T3 |
| 115 | lingering close (R-14) | `close_write`, `discard_remaining` on a real leg; N: full path | client sends a body after a 413 and after an early 401; the pre-exchange rejection list | peer keeps sending; peer never closes | client reads the whole response then close_notify, no reset (`ECONNRESET` absent); discard ends at close_notify, at max, at the deadline; during shutdown the deadline is the drain deadline `grace - 500 ms` (R-64) | T3 (+N) |
| 117 | close-delimited over real TLS (R-12) | `relay_to_end` | response without length | origin close_notify | octets exact, clean end only on close_notify, bounded | T3 |
| 118 | cipher set (R-8) | `accept_public`, upstream arm, snapshot config | clients offering AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305 only | n/a | both AES suites relay exact; ChaCha-only is refused before arming (`PublicHandshakeError::Tls`); snapshot lists exactly the two suites | U+T3 |
| 119 | response limits (ADR-0238, Message-framing contract response rules); status mapping with pre-/post-head projection | listener -> exchange driver -> `read_through` on the upstream half | origin script sends: a declared `Content-Length` of 16 777 217 then 16 777 216; a head of 32 769 then 32 768 octets; 101 then 100 header fields; interim heads whose cumulative size crosses the head limit; after the public head, a body that outgrows its ceiling | scripted origin | over-limit before the public head: exactly one 502, no public octet, upstream closed, one cookie registration; at the limit: relayed octet-exact; post-head overflow: both legs closed, no further response | I |
| 120 | `response_head_deadline` and the deadline composition (one budget from t0, interim heads included) | same, on `SimClock` | origin silent after the upstream request-head write; `103` at t0 + 2 s and a second `103` at t0 + 4 s with the final head withheld, and the same with the final head completing at t0 + 4.9 s; a request that has used 118 s | `SimClock` advance in 1 s steps and one 1 ms step across the boundary | still waiting at t0 + 5 s minus 1 ms; exactly one 502 at t0 + 5 s when the final head is withheld, the head relayed when it completes at t0 + 4.9 s; no `103` reaches the client; a 2 s wait for the 118 s request (authored in the relay file, ignored); upstream closed once, no retry (one `open_upstream`) | I |
| 121 | `body_no_progress_deadline`; 408 before any response octet (Status mapping delta) | same | head with `Content-Length: 1024` then no body octet; a chunked head then a stalled size line; a stall mid-chunk; a trickle of one octet every 29 s; a stall after an early origin response was written | scripted public-client stall on `SimClock` | one 408 exactly 30 s after the last octet moved and no earlier; the 29 s trickle is not cut until `whole_request_deadline` (120 s), then one 502; no `discard_remaining` call after a 408; legs closed, no reuse; with an early response already written no 408 and the S-PIG-93 lingering close governs. Only the early-answer arm is authored (S-PIG-121) | I |
| 122 | relay_exchange `half_close_after_request_read`, `public_close_mid_request_aborts` (the same contract the QC-4 runs replay) | same | a request fully read (a relayed `Content-Length` body, and a bodyless request) then public close_notify; close_notify while idle; close_notify and reset mid-request-body | scripted public-peer close_notify and reset | the whole response (`Content-Length` and chunked) delivered, then both legs closed, no reuse, no further head parsed, upstream closed once; idle: silent close, no upstream leg opened; mid-body: both legs aborted, no response | I |
| 123 | `accept_public` refusal taxonomy (`PublicHandshakeError`; ADR-0226 server configuration) | `HostGatewayRelay::accept_public` against real rustls client peers | ALPN `h2` only; no SNI; a peer silent until the passed deadline; a reset mid-handshake | peer behaviour (production-reachable inputs; no seam) | each stimulus is refused (the variant mapping is an implementation detail and is not asserted); no HTTP octet read; `tls_stat` install counters unchanged; no upstream leg opened; descriptor count returns to start | T3 |

**Native-metal (Tier 3) obligations: oracle, stimulus, bounds, cleanup.** All run only through `cargo xtask metal run --` (native x86_64 KVM host, uid 0, fail closed on the substrate probes; Lima, a virtualized host and nested KVM are non-signal for these). Common rules: the `overdrive-dataplane` integration binary is already wholly in nextest group `host-kernel-shared` (`.config/nextest.toml`), which serialises the `/proc/net/tls_stat` deltas (they are per network namespace and node-global); the conformance binary rows (113, N parts of 105, 109, 115) run under the existing native-metal selector and, through the whole-binary assignment named in Test placement, in the same group; counters are read before and after each test and compared exactly (no other kernel-TLS traffic can run in the window), names as the spike recorded them (`TlsDecryptError`, `TlsRxRekeyReceived`, `TlsRxRekeyOk`, `TlsTxRekeyOk`); every peer is a test-owned rustls endpoint on loopback; every test closes its sockets and pipes through RAII guards and asserts the descriptor count returned to its start value; a failed test must leave no process, socket, pipe or sysctl change behind (the metal lease is exclusive).

| ID | Oracle (independent of the code under test) | Stimulus and fault | Bounds | Cleanup and restoration |
|---|---|---|---|---|
| 104 | returned octets and error variants compared across Host and Sim | identical scripts; producible faults only | <= 30 s per script | sockets closed by guards |
| 105 | bytes at the real rustls peers; sha-256 of body; peer-side TLS record sizes logged | CL and chunked, two exchanges, capacity-boundary lengths | body <= 16 MiB; <= 30 s | guards; fd count equal |
| 106 | bytes at the peer; `tls_stat` deltas; a raw-socket control reading `EKEYEXPIRED` | KeyUpdate (requested and not) at 4 positions x 2 suites | <= 20 KeyUpdates per run; <= 30 s | guards |
| 107 | peer decrypt success; `TlsDecryptError` delta 0 | KeyUpdate request during a 64 MiB splice-out to a throttled reader | <= 30 s; the reader's throttle totals between 5 s and 20 s across the 64 MiB (long enough that the request lands mid-write, short enough to fit the bound) | guards |
| 136 | peer decrypt success; peer KeyUpdate count; `TlsDecryptError` delta 0 | KeyUpdate (requested and not) x 2 suites, lock held by a stalled write | <= 10 s; fill payload is twice the sum of the `tcp_wmem` and `tcp_rmem` ceilings plus 1 MiB, so the write cannot complete until the peer reads | guards |
| 137 | same | same with the send buffer full and the lock free | <= 10 s; same fill payload | guards |
| 108 | bytes at the peer; typed returns | tickets, close_notify at head/body/after response | <= 10 s | guards |
| 109 | peer-injected KeyUpdate answered; `TlsRxRekeyReceived` delta across the 16 MiB production response, asserted equal to 0: any other value fails the test deterministically, falsifies A-K7 and is returned to DESIGN (never inconclusive) | injected KeyUpdate (<= 4 per run); production leg-C response of exactly 16 MiB | <= 60 s | guards; VM Service stopped by the harness owner |
| 110 | `/proc/self/fd` count | body then idle wait | <= 10 s | guards |
| 111 | octets in flight read from `SO_SNDBUF`/`SO_RCVBUF` and pipe capacity; `/proc/self/statm` RSS; wall clock vs passed deadlines | stalled reader (3 s), trickled response | stream <= 256 MiB; <= 60 s | guards |
| 112 | octet-exactness over repeated bodies | mixed record sizes | <= 64 MiB total; <= 60 s | guards |
| 113 | `/proc/self/fd` peak (<= baseline + 768), resident-set growth plus `F_GETPIPE_SZ` capacity of the process's pipes (<= 24 MiB), `getrlimit` reported; the 129th connection's first read returns close with zero TLS octets | 128 concurrent streaming requests via the production composition held open, then one more connection | <= 120 s; guest streams <= 1 MiB each; the 129th connection attempted once, observed for <= 5 s | drain (one 5 s drain), then count equals baseline; VM Service stopped |
| 114 | typed return `Ok`; `/proc/self/fd` count | probe on the honest kernel | <= 10 s | guards; descriptor count equals its start value |
| 115 | client-side read results (full response, then close_notify, never `ECONNRESET`); `ss -tni` shows the leg closed | post-rejection body streams up to `max_request_body_bytes`; silent peer | <= 120 s (deadline cases use short passed deadlines at port level) | guards |
| 117 | octets; close behaviour | response without length | <= 16 MiB; <= 30 s | guards |
| 118 | handshake outcomes; octets | three client suite offers | <= 10 s | guards |
| 123 | typed `PublicHandshakeError` returns (variant per stimulus pinned at DELIVER); `/proc/net/tls_stat` install counters unchanged across each refusal; no HTTP octet read from the refused socket; `/proc/self/fd` count | four real rustls client peers on loopback: ALPN `h2` only, no SNI, silent until the passed deadline (a 2 s deadline passed at port level; the production 5 s `tls_handshake_deadline` is not waited), reset mid-handshake | <= 4 cases x 10 s | guards; descriptor count equals its start value |

**Bounds and cleanup for the fault rows 01–75 that run bounded rounds, windows or native-metal state.** Numbers are DISTILL test bounds consistent with `GatewayLimits::first_slice` (public TLS, request head, upstream connect and response head 5 s each; body no-progress and keep-alive idle 30 s; whole request 120 s; application gate 30 s; one 5 s drain; the 10 s fail-stop outer bound; `BACKEND_MAP` capacity 65 536) and are tuned only by a recorded measured run, never to make a test pass. The Sim rows run on `SimClock` (no wall-clock wait); every native-metal row ends with the leaked-state detection one-liners of `.claude/rules/testing.md` and `debugging.md` (cgroup scopes, XDP, netns, nft tables), run through `cargo xtask metal run --`.

| ID | Bounds (events, rounds, requests, windows) | Recovery or observation window | Cleanup and restoration |
|---|---|---|---|
| 01 | One deploy, one request, body <= 16 MiB, request <= 120 s (`whole_request_deadline`); kernel-TLS samples (`ss -tni`, `tls_stat`) taken before, three times while streaming (1 s apart) and after | From `overdrive deploy` returning to the first 200: poll the public URL once per second, at most 120 attempts, each <= 5 s (`upstream_connect_deadline`); a 503 is expected until the hydrator programs the Service | `DELETE` the Route, stop the Service, graceful `ServerHandle::shutdown` within the one 5 s drain; live intent and receipt counts 0; the harness owner stops the VM; leaked-state sweeps |
| 24 | 256 rounds of two simultaneous declares with distinct Route IDs, each round on a fresh store, <= 5 s per round; seed printed | None (the race resolves inside the call) | Per-round temporary store directory dropped; no kernel state |
| 56 | N: after `job stop` returns, 40 requests 250 ms apart (a 10 s window), stopping early once the outer entry is gone, then 5 more expecting 503; T3: one dial, <= 5 s | Window ends at the earlier of 10 s or the hydrator removing the entry (read by the independent `bpftool` oracle) | Route withdrawn, Service stopped; the test installs no map, rule or route (production leaves the entry, P-4); `nft list ruleset` and `ip rule`/`ip route show table 100` equal before and after (asserted on the T3 arm only: on the N arm `stop` legitimately removes the Service's per-allocation nft elements) |
| 63 | One dial from one registered socket, <= 5 s; one `cleanup(&intent)` | None | Socket closed, `cleanup` once, live counts 0; ruleset and table 100 snapshots equal before and after (the test installs none of them) |
| 64 | One matching-identity dial <= 5 s; proxy recording <= 10 s | None (no negative case alters the identity) | The recording proxy and the peer run inside the test and end with it; no capture process or file exists |
| 69 | I: one request held at the response-head phase, one guest-flow owner restart or fail-stop, `SimClock` advance <= 5 s; N: one restart, request <= 120 s | The owner is back or fail-stopped within the 10 s outer bound | Restart restores the owner; Service stopped; live counts 0 |
| 70 | N: the two authored 408 cases, 408 stalled head (silent client for 5 s plus a 2 s grace) and 408 stalled body (10 body octets then silence for 30 s plus a 2 s grace, <= 40 s) | None | Row 01 cleanup |
| 71 | T3: one instance recreate, zero requests; N: one serve restart, then one request every 500 ms for at most 120 requests (a 60 s recovery window) | Safety: any response seen while the Service map shows no outer entry is 503; liveness: the backend response within 60 s, else the test fails | Serve stopped gracefully; the test-owned pin directory removed and its bpffs pins unlinked; leaked-state sweeps |
| 72 | 2,000 rounds, <= 60 s, seed printed; each round races one `update_service` against 4 concurrent reads of the just-selected id | None | Every service emptied through `update_service` until both live counts are 0 and `BACKEND_MAP` is empty; pin directory removed |
| 75 | Fill to 65 536 entries in batches within the per-service backend limit, then exactly one more update; hard stop at 900 s with the measured fill runtime recorded (the measurement the user needs before acknowledging 5a); emptying (or recreating the dataplane on a fresh pin directory) hard stop 300 s | The rollback is observed within the failing call (no wait) | Every filled service emptied until the `bpftool map dump` count is 0, or the dataplane recreated on a fresh pin directory and the old pins removed |

**Isolation of hand-built real-netns fixtures (S-PIG-56, 63, 64).** The N halves of S-PIG-56 and 64, S-PIG-01 and the other conformance rows use only production-allocated netns, veth and slots (`run_server`'s own provisioner) and need no lease. The T3 halves in the `overdrive-dataplane` hook file may hand-build a veth/netns pair for the leg-C peer or the backend (the dataplane crate has no production provisioner): every such fixture takes its host gateway, workload address and prefix from `overdrive_testing::cidr_lease::TestCidrLease::acquire(owner)` with one stable owner name per topology module (the shared `10.250.0.0/16` pool, disjoint from `WORKLOAD_SUBNET_BASE` `10.99.0.0/16`), keeps any host-loopback backend address and route disjoint from production addresses, records the pool and lease name in the fixture's module documentation and names its netns uniquely; it never embeds a CIDR literal and never reuses production slot-0 addresses. Any test that instead reaches the production per-alloc provisioner draws its net slot from the per-file band registry (`crates/overdrive-control-plane/tests/common/net_slots.rs`; a band constant is added for the file), never `NetSlotAllocator::new()` and never a raw slot literal. Slot partitioning does not remove the node-global axes (the `overdrive-mtls` nft table, the root cgroup attach, `:53`): those stay serialized by `host-kernel-shared` (Test placement). S-PIG-64 (and S-PIG-38, 43, 52) live in `overdrive-control-plane/tests/integration/public_ingress_gateway_mtls.rs`, so any hand-built fixture there takes the same lease and band rules; its kernel-state mutation (kTLS, `overdrive-mtls` nft table) keeps it in the `host-kernel-shared` group by the catch-all rule of `.claude/rules/testing.md` (the crate's integration binary assignment is unchanged by the move). Before diagnosing a connection assertion, read `ip route`, `ip rule` and `ip netns list`.

S-PIG-74 authors sub-cases (a), (c) and (d) (an absent sentinel receipt at `take_receipt`, a Gateway Identity that never becomes Current, and a probe that times out); sub-case (b), a hook that does not fire, is the `Unavailable`-at-`probe` stimulus of the S-PIG-57 (I) arm. No scripting API is added.

Held-probe rule for the Sim (DG-9). `HoldConnectProbe`/`ReleaseConnectProbe` park the staged gate at an awaited operation the gate owns (`GatewayConnectDataplane::probe`, `GatewayRelay::probe` or the identity `ensure_current` wait), not at `GatewayClientMtls::authenticate`, which the gate no longer reaches (R-65). A held gate needs no `Selected` receipt, so S-PIG-31, 32 and 68 program no backend for the hold. The Sim hold state is Sim test support shaped by DISTILL. S-PIG-74 (d) holds the boot sentinel probe, which is the dataplane's own `probe`. If the existing hold cannot yield `ProbeTimeout` at the sentinel under an advanced `SimClock`, report it to DESIGN; no scripting API is added.

## Wave: DISTILL / [REF] Strategy and project-policy mapping

Architecture-of-Reference treatment is inherited; no per-feature strategy. Driving surfaces use their real protocols (public TLS/HTTP, operator HTTPS API through the exported handler, clap parse). Driven-internal state uses `LocalIntentStore` over real redb, `SimObservationStore` for deterministic focused composition, and the real `EbpfDataplane` and kernel on native metal for Tier 3. External/non-deterministic ports (`Clock`, CA entropy/KEK, guest dial) use the existing Sim ports. The `GatewayRelay` port (G1) is a driven port whose real adapter is the kernel itself: Tier 1/2 use `SimGatewayRelay` (userspace copy between in-memory record queues, same observable contract), Tier 3 uses `HostGatewayRelay` against real rustls peers on loopback, and the two are compared by the equivalence test over the faults both can produce. Layers 3 and above are example-based; generative coverage is confined to pure and in-memory properties (S-PIG-03–10, 12–17, 44, 76–86, 89, 102, 103 at layers 1–2).

No scenario hand-installs a map, rule, identity, application snapshot, certificate, protected record or kernel-TLS state that production installs. The only test-supplied credential is an operator input accepted by the production serve flags or `ServerConfig.gateway`. Faults enter only through existing driven ports (Sim dataplane/identity/mTLS typed faults, `HoldConnectProbe`/`ReleaseConnectProbe`, `SimObservationStore` faults, `SimClock`, the `SimGatewayRelay` capabilities listed in Scaffolds) or are production-reachable inputs to a real adapter (a peer that sends a KeyUpdate, closes, resets or stalls; a client that keeps sending a body; descriptor pressure). `AT-PIG-E2E-1` and the hook and relay tests run only on the native x86_64 KVM host via `cargo xtask metal run --`. Policy rows appended to `docs/architecture/atdd-infrastructure-policy.md` (Driving 2, Driven internal 6, Driven external 2): Driving: public HTTPS listener (conformance harness public client), `Route` operator surface; Driven internal: `GatewayConnectDataplane` (Sim pair in Tier 1/2, `EbpfDataplane` in Tier 3), `GatewayRelay` (`SimGatewayRelay` in Tier 1/2, `HostGatewayRelay` in Tier 3), custody over real redb; Driven external: `GatewayClientMtlsFactory` (Sim/Host).

## Wave: DISTILL / [REF] Adapter coverage

| Driven adapter / effect | Real-I/O or contract scenario | Coverage boundary |
|---|---|---|
| `LocalIntentStore` / Public Route Set | S-PIG-21–24, 27, 33, 48, 66 | Real redb deltas, persisted empty set, reopen, singleton claim, concurrency, exact lag count and recovery |
| `ManualCertifiedKeySource` and files | S-PIG-27, 46, 01 | Unreadable and malformed material; test-CA or operator-supplied chain |
| `PublicCertifiedKeyAeadCodec` / KEK / protected redb record | S-PIG-45–47 | Production codec and custody; 0/1/N chains, replace/expire, tamper/AAD/CSPRNG faults, secret-free status |
| `ObservationStore` gateway status and issuance audit | S-PIG-25, 35, 36, 42, 65 | Typed point reads, redacted JSON, 500 fault mapping, audit-before-hold |
| `ServiceFrontendResolve` | S-PIG-49, 01 | Service intent plus platform VIP resolves one frontend; closed failures preserve the intent universe |
| `GatewayConnectDataplane` / `EbpfDataplane` / cgroup hook and maps | S-PIG-02, 28–30, 37, 41, 50, 53, 56, 57, 63, 67, 71, 72, 75 | Tier 1 uses `SimGatewayConnectDataplane`; Tier 3 uses the real hook, `open_upstream` preparation (mark, loopback bind, cookie, registration), receipts and cleanup. The transient receipt oracle is the `live_intent_count()`/`live_receipts()` post-state (plus the occurrence in AT-PIG-E2E-1 only, P-36), not map polling |
| Ordinary `ServiceMapHydrator` -> `update_service` (D1) | S-PIG-01, 56, 67 | Real hydrator programs the VM backend; the gateway adds no mechanism |
| `HostGatewayClientMtls` / rustls / leg-C / vsock | S-PIG-38, 43, 52, 64, 01 | Exact selected peer, dedicated gateway SVID, wrong-valid and absent/expired identity fail before body and arm nothing (S-PIG-38, 43, 52); matching-identity wire evidence on the leg-B/leg-C segment (S-PIG-64) |
| **`HostGatewayRelay`: kernel-TLS arming from `KernelConnection`, control-record dispatch, rekey, private pipe and `splice`, probe** (`@real-io @adapter-integration`) | S-PIG-104–112, 114, 115, 117, 118, 123, 64, 01, 113 | Byte-exact CL/chunked relay, KeyUpdate in head/chunk line/boundary/body with `tls_stat` oracles, transmit lock, ticket/close_notify/alert handling, backpressure and stalls, pipe lifetime, record boundaries, lingering close, close-delimited, cipher set, real-kernel probe, 128-exchange budget |
| **`SimGatewayRelay`** (`@in-memory`) | S-PIG-83, 87–103, 119–122, 98 | Exchange behaviour, rejection projections, deadlines, drain and the lying-substrate probe gold test; the Sim cannot model kernel record layout, so no Sim result stands in for a Tier-3 oracle (S-PIG-104 compares them) |
| **Public handshake and custody server configuration** (`accept_public`, `PublicCertifiedKeySnapshot::server_config`) | S-PIG-118, 123, 105, 01 | TLS 1.3 only, ALPN `http/1.1`, exact SNI, no tickets or 0-RTT, secret extraction, cipher set; typed `PublicHandshakeError` (a refused client sends no HTTP) |
| **Exchange driver and head/framing pure modules** (`runtime::http1`, `runtime::exchange`) | S-PIG-76–86 (pure), 87–103 and 119–122 (through the production listener on the Sim relay), 105 (real) | Framing decision, line validation, chunk lines, interim heads, status mapping with pre-/post-head projection, single attempt, keep-alive, drain; replaces the retired "Public HTTP runtime (hyper)" row |
| Gateway identity slot, issue-and-audit, reconciler | S-PIG-51, 65, 26 | Dedicated non-allocation subject, epoch idempotency, stale no-op, drain-ordered empty |
| `ServerHandle` / `GatewayHandle` boot and shutdown (D5) | S-PIG-26, 57–62, 69, 74, 99–101 | G0–G4 order, relay probe then connect probe, stage `gateway`, one budget, fail-stop interaction, disabled path |

Coverage caveat ("Authoring status by batch"): the rows citing S-PIG-38, 43, 52 and 64 (the `HostGatewayClientMtls` handshake and wire evidence) are authored and ignored `pending DELIVER exact-peer-client-mtls`, and S-PIG-57–62, 74 and 100 carry authored enabled-composition arms ignored under their pending keys; until DELIVER activates them, the `HostGatewayClientMtls` adapter and the `ServerHandle`/`GatewayHandle` row are exercised on the production path by S-PIG-01, S-PIG-59 (disabled composition), S-PIG-26, S-PIG-60 (N graceful run), S-PIG-69 and S-PIG-99. S-PIG-75 is dropped (R-21) and covers nothing.

Public DNS is not an Overdrive adapter. Production reachability needs an operator-managed IPv4 A record for the Route's Public Hostname. Hermetic runs and E15 use only `curl --resolve <hostname>:443:<gateway-ipv4>` (client-local address selection; the hostname remains SNI, Host and certificate identity); no step edits `/etc/hosts` or uses ambient DNS.

## Wave: DISTILL / [REF] Scaffolds

Only shapes the DESIGN pins are scaffolded; bodies of production scaffolds fail with SCAFFOLD text under file-level lint allowances. Every test body is complete and executable, including every KEEP unit body (S-PIG-03–10, 12–20, 44, 45, 47, 55 alike), each carrying a reasoned `#[ignore = "pending DELIVER <descriptive-name>"]`, except the two expected-GREEN preservation rows S-PIG-41 and S-PIG-59, which are authored without `#[ignore]` (a test carrying a pending marker cannot be expected-GREEN; each must pass on its first run once its scaffold compiles). Activation: remove only the reasoned `#[ignore]`, observe a semantic RED, implement, GREEN, commit (testing.md "RED acceptance scaffolds and activation"). A scaffold never introduces a method, type, variant, trait or parameter the design does not name: a missing signature is a DESIGN gap listed in the Completeness audit, not scaffold.

| Scaffold (crate / file) | DESIGN anchor | Notes |
|---|---|---|
| `overdrive-core`: `public_ingress.rs`, `gateway_identity.rs`, `SpiffeId::for_gateway`, two `ObservationRow` variants and point reads (previously approved 2026-09-13), `Action::IssueGatewaySvid`, `HydrationContext.gateway_identity` | Persistence, custody and redaction; Gateway identity; D5 items 9–10 | Demand types, `gateway_demand`, `gateway_frontend_demand` are not carried |
| `overdrive-core` + `overdrive-store-local` + implementer fallout: **`IntentSubscriptionEvent` type and the `IntentStore::watch` return-type change only** | D5 item 12 | Compile prerequisite only: `LocalIntentStore` maps every broadcast event to `Changed` and detects no lag (main's silent drop on lag is unchanged); lag behaviour is DELIVER's. Item 12 was reconfirmed by the user on 2026-11-02 (R-18) and P-8 is not adopted; a committed delete is reported as `Changed { value: None }` and a put (including an empty value) as `Changed { value: Some(..) }` |
| `overdrive-host`: `ca/public_certified_key.rs`, `socket_cookie.rs` | Persistence; component decomposition | |
| `overdrive-gateway` (new crate): the relay port family in `ports` (below), `GatewayConnectDataplane` with `open_upstream`/`PreparedUpstream`, `GatewayClientMtls`/`GatewayClientMtlsFactory`/`GatewayClientMtlsDeps`/`GatewayClientMtlsError` (with `Arm(KtlsArmError)`), `GatewayUpstreamSeal`/`GatewayUpstream { leg: KtlsLeg }`, application, connector, certified key, route set, frontend, `runtime::{limits,proxy_headers,routing}`, `runtime::http1` (pure head validator, parsers, framing decision, chunk-line parser, `exchange` with `ExchangeMachine`), `runtime::exchange` (the exchange driver), `GatewayRuntimeDependencies::new(limits, dataplane, client_mtls, identity_lifecycle, relay)`, `UnboundGatewayBuilder`/`GatewayBuilder`/`GatewayRouteCommands`/`GatewayStatusReads`/`GatewayHandle` (R-25), `GatewayClientIdentitySource`/`GatewayClientIdentityAccess` (R-20), `GatewayConnectCleanup` (R-29), the pure `http1` functions of R-32 | D5 (a)(b)(d), Kernel relay data path (G1), Application sections | D5 items 1–13 are confirmed (R-18). `GatewayClientMtlsFactory`, `GatewayClientMtlsDeps` and the identity access are scaffolded in `ports` under R-20 (`Deps.identity` names a `ports` type; the dataplane implements the source). No hyper-based runtime exists to remove: the crate is new and never depends on `hyper`, `hyper-util` or `tokio-rustls` |
| `overdrive-bpf`: `maps/gateway_connect.rs` (two 8-byte POD maps, capacity 128, unpinned) | Backend programming and selection (D3) | **No selection-arm program is scaffolded**: it is the separate `cgroup/connect4` program (R-1), which DELIVER adds |
| `overdrive-dataplane`: `gateway_connect.rs`, **`gateway_relay/` (`HostGatewayRelay`; kTLS arming, control-record dispatch, rekey, private pipe and splice, probe; the one `unsafe` module `gateway_relay::sys`)**, `mtls/gateway_identity.rs` (slot, handles, `client_access` and the private identity source; `GatewayClientIdentityAccess` itself moves to `overdrive-gateway::ports`, R-20), `mtls/gateway_client.rs` (`HostGatewayClientMtls::new(identity, seal, limits, relay: Arc<HostGatewayRelay>)`, `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)`), commit-guard types, `DataplaneError` variants | Commit-guarded transaction; identity integration; obligation 15; Kernel relay data path (G1) | Allocator internals beyond `existing_id` untouched (P-2); `existing_id` is the scaffolded accessor and is contract (R-24). `HostGatewayRelay::new(clock: Arc<dyn Clock>)` is pinned (infallible, kernel-silent: it opens no socket and claims no capability; `probe` proves every kernel fact). **To scaffold under the 2026-11-03 rulings:** `HostGatewayClientMtlsFactory` (R-20) and `DataplaneError::{BackendIdentityConflict, ServiceApplyRollback}` with `ServiceApplyStage`/`ServiceApplyRollbackFailure` in `overdrive-core` (R-35; `ReverseNatDelete { backend: SocketAddrV4, proto: Proto }`, converted from `BackendKeyPod` at the adapter). The `GatewayClientIdentityAccess` currently scaffolded in `mtls/gateway_identity.rs` moves to `overdrive-gateway::ports` (R-20) |
| `overdrive-reconcilers`: `gateway_svid_lifecycle.rs` + `Any*` arms | Closed-dispatch contract; D5 item 9 | Host-backed wakeup (empty `interests()`, `resync_schedule()`) |
| `overdrive-control-plane`: `BootAdapters.gateway`, `GatewayAdapters { certified_key_codec, client_mtls, relay }` and `GatewayAdapters::host(kek, clock)` (one `Arc<HostGatewayRelay>` coerced to `Arc<dyn GatewayRelay>` and handed to the mTLS factory), `ComposedDataplane.gateway_connect`, composition, action shim, boot G0–G4 (relay probe then connect probe inside the gate task), `ServerHandle` stage `gateway`, handlers, regenerated `api/openapi.yaml` | D5 | Dispatcher fallout across the control-plane tests. The enabled composition (`GatewayAdapters.client_mtls`, boot G1–G4) is a documented `todo!("SCAFFOLD: enabled gateway composition (boot steps G1-G4)")` arm in `lib.rs` until DELIVER implements it; `GatewayAdapters` carries `certified_key_codec` and `relay` today, and `ServerHandle.gateway` is always `None` |
| `overdrive-cli`: four `ServeArgs.gateway_*` fields, Route deploy dispatch, `ApiClient` calls | D5 item 11 | Constructor fallout across the CLI tests. The four clap fields are real; `ServeArgs::gateway_config`, `deploy_resource` and `ApiClient::submit_route` are `todo!` scaffolds (R-53 adds `ApiClient::withdraw_route` and `ApiClient::gateway_status` as the same kind of scaffold, to be added by the follow-up DISTILL pass; the shape is pinned); `run_with_adapters` and `main`'s Deploy arm are not re-pointed until DELIVER (re-pointing now would panic the shipping workload deploy path) |
| `overdrive-sim`: `adapters/gateway.rs` with `SimGatewayClientMtlsFactory::new(Arc<SimGatewayRelay>, clock)`, `SimGatewayClientMtls::new(seal, Arc<SimGatewayRelay>, clock)`, `SimGatewayAdapters` (`new(clock)`, `relay()`, `client_mtls()`, `sim_relay()`), `SimGatewayConnectDataplane`, identity targets, and **`SimGatewayRelay::new(clock)` with its test-support surface (below)**; `invariants/gateway_application.rs`, two `Invariant` variants outside `Invariant::ALL` | Exact seeded invariant registration; Kernel relay data path (G1) | Activation key `gateway-application-invariants`, as the DESIGN now pins it (`#[ignore = "pending DELIVER gateway-application-invariants"]`). No `Invariant` variant is added for the relay seeded runs (S-PIG-102/103 are seeded acceptance tests; a catalogue variant would be new scope the user adds). `SimGatewayClientMtlsFactory` and `SimGatewayAdapters` are to be scaffolded (R-20). The `overdrive-sim` `[dependencies]` edge on `overdrive-host` (the pinned `UnboundGatewayBuilder::new` names `PublicCertifiedKeyAeadCodec`, which has no Sim twin) conforms to the port-trait dependency rules and stays (R-40) |
| **SVID issuance failure occurrence** (R-45..R-52): `overdrive-core`: `ca/issue_failure.rs` (`SvidIssueFailureCause`, `SvidIssueFailure`), `gateway_identity::GatewayIdentityIssueRecord`, `GatewayApplicationStatusRowV1.{issue_failures, last_issue_failure}` (in place), `WorkloadSvidIssueStatusRowV1` / `WorkloadSvidIssueStatusRowEnvelope` (codec bodies `todo!`), the two `ObservationStore` operations; `overdrive-gateway::ports`: `issue_record` on `GatewayIdentityLifecycleControl` and `GatewayIdentityLifecycleTarget`, `record_issue_failure` on `GatewayIdentityActionTarget`, `IssueFailed` removed; implementers (`overdrive-dataplane` slot handles, `overdrive-sim` identity target and control, `ControlPlaneGatewayIdentityLifecycle`, `LocalObservationStore`, `SimObservationStore`) fail with SCAFFOLD text, test doubles return `None` or delegate; `overdrive-control-plane`: `CaIssuanceError::failure_cause`, `api::{SvidIssueFailureCauseBody, SvidIssueFailureBody}` with `From<SvidIssueFailure>`, and the two additive `AllocStatusRowBody` / `GatewayApplicationStatusBody` fields; regenerated `api/openapi.yaml` | "SVID issuance failure occurrence" | `failure_cause` and the `From` conversion carry their real bodies (pure total mappings, so their tests are active); the executor recording, the owner projection, the handler stamping, `render::workload_describe`'s line and the status handler stay DELIVER's. The body field defaults in `From<AllocStatusRow>` (`0` / `None`) are the conversion's, overridden by the handler as `resources` and `network_cleanup_pending` are |
| `tests/conformance`: end-to-end file and harness helpers (the QC drivers live in their owning crates) | Evidence-lane table; Formal protocol model | See drivers below |
| `examples/public-ingress-gateway/` and `verification/expectations/E15-public-ingress-operator-journey/` | Artifact-boundary handoff | See below. The E15 README, `execution-substrate` file, INDEX row (status `pending`) and `runner.sh` exist; the runner has never been run and no evidence exists |

**Relay port family scaffold (exact shapes, all from "Driven port `GatewayRelay`", "Cross-crate construction of the opaque leg values" and "Exchange state machine").** In `overdrive-gateway::ports`: `KtlsLeg` (`from_armed(read: Box<dyn Any + Send>, write: Box<dyn Any + Send>) -> KtlsLeg`, the one constructor; `into_halves(self) -> (KtlsReadHalf, KtlsWriteHalf)`), `KtlsReadHalf` and `KtlsWriteHalf` (no public constructor; `backend_mut<T: Any>(&mut self) -> &mut T`, `unreachable!()` on a type the backend did not store), `AcceptedPublicLeg { leg, sni }`, `HeadTerminator`, `ThroughBytes` (`new(bytes: Vec<u8>)`, `as_slice()`), `RelayDeadlines { absolute, no_progress }`, `ReadDeadlines { first_octet, complete_within, complete_by }`, the trait `GatewayRelay` with `probe`, `accept_public`, `read_through`, `write_all`, `relay_exact`, `close_write` and the two methods `relay_to_end` (R-12) and `discard_remaining` (R-14), and the error enums `RelayProbeError`, `PublicHandshakeError`, `KtlsArmError`, `KeyUpdateError`, `LegReadError`, `LegWriteError`, `RelayError`, `RelayPipeError`. Pure crate-private: `ExchangePhase`, `ResponseState`, `Remaining` (`UntilClose`, R-12), `Disposition`, `Side`, `BodyFraming` (`NoBody`, `Length(u64)`, `Chunked`, `UntilClose`; `UntilClose` is the close-delimited-response variant (R-12) and is illegal as a request framing), `FramingViolation` (`WrongPhase { phase }`, `ResponseHeadRepeated`, `UntilCloseRequest`, `OctetsBeyondFraming { side, n }`, `ChunkedEndNotOpen { side }`, `ResponseNotComplete`), `ExchangeMachine` with `new()`, exactly the pinned methods (each fallible one returning `Result<_, FramingViolation>`) and the seven accessors. Backends: `HostGatewayRelay::new(clock: Arc<dyn Clock>)`, `SimGatewayRelay::new(clock: Arc<dyn Clock>)`, `SimGatewayClientMtlsFactory::new(relay: Arc<SimGatewayRelay>, clock: Arc<dyn Clock>)`, `SimGatewayAdapters::new(clock)` with `relay() -> Arc<dyn GatewayRelay>`, `client_mtls() -> Arc<dyn GatewayClientMtlsFactory>` and `sim_relay() -> &Arc<SimGatewayRelay>`; tests assemble the pinned ports directly (`SimGatewayClientMtlsFactory`, `SimGatewayAdapters`, `GatewayClientMtlsFactory` and `GatewayClientMtlsDeps` are scaffolded under R-20; the Sim crate does not name the control-plane type). Every scaffolded body above, including these constructors, `backend_mut` and `ExchangeMachine::new`, fails with SCAFFOLD text, so each test is RED at its first production call and never at an import. A `FramingViolation` result is the exchange driver's own call-sequence defect, never a peer fault: the driver closes both legs, answers 502 if no public octet was written (never a 4xx) and emits one error-level event naming the variant; the Sim exchange tests therefore treat a `FramingViolation` from a model-legal call as a failure of the run. All of this shape is on the item 13 list confirmed by R-18 (together with the R-20..R-44 items above); nothing beyond it is scaffolded, and no scaffold introduces a method, type, variant or parameter the DESIGN does not name.

**Cargo dependency list (from the "Technology choices" and "Application technology choices" rows).** `overdrive-gateway`: adds `httparse` (lock 1.10.1) and `http` as direct dependencies, `async-trait`, `rustls` (workspace 0.23.39), `arc-swap`, `zeroize`, `x509-parser`, `ring`, `rkyv`, `redb`, `tracing`; never `hyper`, `hyper-util` or `tokio-rustls` (the operator listener in `overdrive-control-plane` keeps `hyper`). `httparse`, `http`, `arc-swap` and `zeroize` are not yet `[workspace.dependencies]` entries on main, so they are added there first (`.claude/rules/rust.md`: workspace dependencies always); `Cargo.lock` is regenerated, never edited. `overdrive-dataplane`: `libc` and `rustls` already present (the unsafe `setsockopt`/`recvmsg`/`splice`/`pipe2`/`F_SETPIPE_SZ` calls stay in `gateway_relay::sys`). `overdrive-host`: `rustix` gains the safe `net` feature (already in the workspace feature list). `overdrive-sim`: no new external crate. Each crate declares `integration-tests = []`.

**`SimGatewayRelay` test-support surface (shaped by DISTILL because the DESIGN leaves it to DISTILL; capabilities, not signatures; lives only in `overdrive-sim`, never on a production type).** It implements the pinned `GatewayRelay` contract over in-memory leg state and nothing else is scripted:

| Capability | Used by |
|---|---|
| Present a substrate lie to `probe`: `tls` ULP absent, cipher refused, splice losing octets (`moved`, `expected`), pipe read-back below the floor, timeout, I/O error; or present none | S-PIG-99, 100 |
| Script each leg's inbound application octets together with control events placed at octet offsets: peer KeyUpdate (requesting or not), NewSessionTicket, close_notify, alert (description), reset, an unexpected record | S-PIG-83, 87–95, 102, 103 |
| Script an upstream peer's behaviour at the leg: respond with a head and body, close before the head, close mid-body, close with close_notify at the end (close-delimited) | S-PIG-91, 92 |
| Fail the pipe at its first `relay_*` call: creation error or read-back below the floor | S-PIG-95 |
| Enforce `ReadDeadlines` and `RelayDeadlines` against the injected `SimClock` (no sleeping; a scripted stall plus a clock advance yields `Deadline` or `NoProgress`) | S-PIG-85, 93, 96, 101 |
| Bound a per-leg send window so a short count or backpressure can be scripted | S-PIG-111's Sim twin (104) |
| Observe, per leg, the transcript of application octets written, close_write/close_notify issued, and the live leg and pipe counts (residue) | every Sim row |
| Script a failed answer to a peer KeyUpdate: `KeyUpdateError` kind `RustlsRefused`, `RearmRx`, `RearmTx` or `Io`, on either leg and in either direction | S-PIG-90 |
| Script an arm failure of a chosen `KtlsArmError` kind (`UlpInstall`, `CryptoInfo`, `CipherNotSupported`, `KernelConnection`) at the upstream arm after a matching peer, and at `accept_public` | S-PIG-98 |
| Hand out a `SimGatewayClientMtls` bound to the same backend (used with the arm-failure capability above) | S-PIG-98 |

The Sim path never produces a fault the Host cannot produce except the substrate lies, which belong only to the probe tests, and the two scripted failure kinds above (a failed KeyUpdate answer and an arm failure), which a healthy kernel does not produce: they are exercised only on the Sim (S-PIG-90, 98) and are excluded from the equivalence comparison. The equivalence test (S-PIG-104) compares the two adapters over the faults both can produce. These two capabilities are DISTILL test support in `overdrive-sim` only; nothing beyond the listed capabilities is added.

**Held-probe capability for QC-2 `probe_result(tok, ok)` (DISTILL test-support shaping inside the DESIGN facts "Where a double may park a probe (DG-9)"; since DG-7 it lives in the QC-2 driver's private connect and client doubles inside `overdrive-gateway`, not in `overdrive-sim`; capabilities, not signatures; never on a production type).** The DESIGN pins `HoldConnectProbe(ServiceFrontend)` and `ReleaseConnectProbe(ServiceFrontend)` for the seeded invariants, and DG-7 rules that the QC-2 driver cannot use `overdrive-sim`'s gateway adapters (they implement the other copy of the crate-local traits in the library's `cfg(test)` build). The spec action `probe_result(tok, ok)` completes the probe of one staged candidate, including a candidate that has since been superseded. The double provides: (1) hold: each staged gate parks as its own entry at an awaited operation the gate owns (R-65: not `GatewayClientMtls::authenticate`); (2) release exactly one entry, addressed by the gate it parked and never by arrival position; (3) a typed outcome: ok = the parked operation returns `Ok(())`, failed = it returns the typed `GatewayConnectDataplaneError::Unavailable` (the gate reports `ConnectProbe(..)`, status `ConnectPathUnavailable`); (4) observe the number of entries held. The driver learns which entry belongs to candidate `tok` by observing the one entry that parks while `application_status().staged` is `Some` for `tok`, and releases it. The owner's behaviour on the probe of a superseded candidate (keep awaiting or drop it) is unspecified by the DESIGN and both are compatible: releasing an entry whose waiter is gone is a no-op for the double, and the driver asserts only what the model requires, the `application_status()` State comparison (a superseded `tok` leaves current, staged and draining unchanged). `overdrive-sim` keeps its frontend-wide `HoldConnectProbe`/`ReleaseConnectProbe` for S-PIG-31/32 (seeded) and S-PIG-68 (the `overdrive-gateway` integration binary); the crate-private `GatewayApplicationSimFault` enum is not extended, and no scenario needs per-entry release in `overdrive-sim`.

**Hook point (settled by the DESIGN, DG-9; replaces DISTILL's earlier connect-parking design, which is withdrawn).** The port's own `probe` is the boot sentinel probe, run once (ADR-0230); the connector probe the gate awaits last is `GatewayUpstreamConnector::probe`: connect permit, `open_upstream` (sync), `into_parts`, `connect(2)` on that socket to the Service Frontend (a real kernel call on a real socket that no backend sees), `take_receipt` (sync); it stops there for either receipt (R-65). The sync port methods cannot park a caller and the connect cannot be parked by a double, so the park points are the other awaited operations the gate owns (`GatewayConnectDataplane::probe` and `GatewayRelay::probe` while their once-per-boot proofs are not yet established, and the identity `ensure_current` wait). Facts that bind the double and every held-probe scenario: (a) a held gate needs no `Selected` receipt and no programmed backend; once the connect dataplane proof has succeeded in a boot a later candidate has no double-controllable await, and a scenario needing a second in-flight candidate uses the connect-fixture hold of DG-9 consequence 1; (b) `open_upstream` returns `PreparedUpstream::new(tokio::net::TcpSocket::new_v4()?, intent)` (DG-8: plain constructor, registers nothing, carries no cleanup guard; the double records the cookie it minted for `intent`); (c) the real `connect(2)` must reach a peer that completes it: the fixture gives the Service Frontend a loopback address with a listening socket (`ServiceVip::new` accepts any `IpAddr`), which is DISTILL test support; (d) a gate parked at `connect(2)` holds one connect permit until it returns, so the driver keeps such entries below `max_inflight_connects` (128). The identity double answers `ensure_current` and `subscribe` immediately with current facts; the relay double's `probe` succeeds; methods the QC-2 path does not exercise return the port's typed unavailable error.

**QC-2 composition and isolation (DG-10 pinned by the DESIGN; the isolation decision is DISTILL's).** The admission is injected, not exposed: the driver builds one `GatewayLimits`, `GatewayAdmissionBudget::new(Arc::new(limits.clone()))` and `GatewayAdmission::new(budget)`, keeps the `Arc`, obtains the builder through the public `UnboundGatewayBuilder::new(..).await` and `into_builder()` (core-trait Sims for the observation store and clock; a real redb Route store), calls the crate-private `GatewayBuilder::start_with(deps, admission)` and takes `GatewayRouteCommands` and `GatewayStatusReads` from `into_parts()`. `open_connection` is `admission.acquire_connection(now)` on the retained `Arc`, the lease kept for `close_connection`. `GatewayAdmission::new`, `budget()` and `start_with` are crate-private items with production callers in `start`; no accessor exists whose only caller is the driver. Promotion goes through the real `GatewayListenerControl::bind`, which binds the configured address on port 443 (`GatewayConfig::bind` is "always port 443"). The driver supplies its own distinct `127/8` bind address through `GatewayConfig::new(bind_address, ..)` (R-6, DG-12: the address octets are a documented per-composition table in the driver's header, distinct from every other gateway test composition), so two compositions never contend for one socket; there is no port seam and none is invented. It runs as root (privileged port) through `cargo xtask metal run --` (no KVM is needed). The `overdrive-gateway` library-test binary touches no node-global kernel state, so it joins no `host-kernel-shared` assignment (`.claude/rules/testing.md` "Node-global kernel state requires a catch-all nextest group" applies only to cgroup, nft, bpffs or `:53` state). The same holds for every other gateway Sim-relay composition that reaches promotion.

**QC-2 driver pins (R-65 audit; the acceptance designer applies them in `crates/overdrive-gateway/src/application/quint_gateway_application.rs`).** (a) Generation naming: the naming translator maps one owner `GatewayApplicationGenerationId` to exactly one model `gid` and one model `gid` to exactly one owner id; two model generations never map to one owner id, and an owner id seen under a second `gid` is a failed comparison. (b) `stage` is an assertion on an owner step, never an adoption: the handler yields until idle, then asserts that `application_status().staged` is `Some` for the live Route generation; it does not accept a gate the owner staged on its own as the model's stage by adopting its probe or token. (c) `stage` asserts that the staged generation id is new: it differs from every id the translator already holds, `current` and `draining` included. (d) `probe_result` for a superseded `tok` first lets the owner consume the superseding Route event (Gate G-1, failed-candidate rule); the driver does not release the held gate before then. (e) A failed candidate's Route generation is not re-staged until `route_declare`; the driver asserts that a `stage` step after a failed `probe_result(tok, false)` for the same Route generation is not enabled and the owner has staged nothing.

**Lag-aware watch decision.** DISTILL ports only what no body can compile without: the `IntentSubscriptionEvent` (`Changed | Lagged`) type, the `IntentStore::watch` return-type change and its implementer fallout (`LocalIntentStore`, `FaultInjectingIntentStore`, `CountingIntentStore`, Sim). `LocalIntentStore` maps every broadcast event to `Changed` and implements no lag detection, so main's silent-drop-on-lag behaviour is unchanged. S-PIG-48 and S-PIG-66 stay complete executable bodies under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]`; reporting `Lagged` with the exact missed count, relist on `Lagged` and admission closed meanwhile are DELIVER's. D5 item 12 was reconfirmed by the user on 2026-11-02 (R-18; D-APP-10 of the DESIGN approved 2026-09-13; `IntentStore` unchanged on main since the merge base) and P-8 is not adopted, so the port is landed. S-PIG-48 is the test in `crates/overdrive-store-local/tests/local_store_edges.rs`; its accounting is stated at the port only (every committed write is delivered as `Changed` or counted in a `Lagged.skipped`, the delivered keys are the contiguous suffix after the gap), so it does not depend on the private notification-buffer size.

**quint-connect drivers** (spec referenced in place at `specs/quint/public-ingress-gateway/`; each driver module is `#[cfg(all(test, feature = "integration-tests"))]` in the owner's crate; `#[quint_test(max_samples = 1)]` per bound run and one `#[quint_run]`; every action handler calls the pinned driving port; `State` projects only the pinned accessors; no always-false cfg):

| Driver | Module / conformance main | Owner and placement | Runs bound to `#[quint_test]` | Projection (pinned accessors) | Ignore reason |
|---|---|---|---|---|---|
| QC-1 | `gateway_identity` / `identity_conf` | `overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs` (D-PASS7-1: in-crate child module; `dispatch_issue` stays `pub(crate)`) | `issue_hold`, `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue` (5) | `GatewayIdentitySlot::{desired,current}`, `ObservationStore::issued_certificate_rows` filtered to the gateway SPIFFE id | `pending DELIVER quint-conformance-gateway_identity` on all five named runs and the `#[quint_run]` |
| QC-2 | `gateway_application` / `app_conf` | `overdrive-gateway/src/application/quint_gateway_application.rs` (DG-7; declared from `application`, a child of the module that owns the owner and its admission state); owner = the real `GatewayBuilder`/`GatewayApplicationOwner` over a real redb Route store with **driver-local private doubles** of the four gateway-crate ports (`GatewayConnectDataplane`, `GatewayRelay`, `GatewayClientMtls`, `GatewayIdentityLifecycleControl`); only core-trait Sims (`SimClock`, `SimObservationStore`) come from `overdrive-sim`; the connect double holds a staged gate at an awaited operation the gate owns, by gate token (DG-9, R-65) | `replace_valid`, `route_withdraw_drain`, `late_probe_after_supersession`, `failed_candidate_not_restaged` (4) | `GatewayStatusReads::application_status()` | `pending DELIVER quint-conformance-gateway_application`; DG-8, DG-9 and DG-10 are closed and confirmed (R-18); composed through `UnboundGatewayBuilder`/`GatewayBuilder::start_with`; binds its own distinct `127/8` address on port 443 (DG-12 ruled, R-6), so it joins no nextest group for the public bind |
| QC-3 | `selection_receipt` / `receipt_conf` | `overdrive-dataplane/src/quint_selection_receipt.rs`; owner = the real `EbpfDataplane` on native metal with the real `cgroup/connect4` selection arm (D-PASS7-2); no `Sim*` owner or projection source; run through `cargo xtask metal run --` | `commit_then_select`, `conflicting_alloc_rejected` (carries the empty-update tail, R-36), `restart_destroys_receipts` (3, over `receipt_conf`) | `selected_backend_identity` (`ident`), `live_intent_count` (`intents`), `GatewayConnectCleanup::live_receipts()` (`receipts`, R-33); `take` is asserted by its handler and `read_identity` only calls the accessor | `pending DELIVER quint-conformance-selection_receipt` until the separate selection program (R-1) and the owner are GREEN |
| QC-4 | `relay_exchange` / `relay_conf` | `overdrive-gateway/src/runtime/http1/quint_exchange.rs` (declared from the `http1` module); owner = the pure `ExchangeMachine` (no `Sim*`) | `exchange_cl_keepalive`, `early_response_closes`, `drain_with_inflight_body`, `half_close_after_request_read`, `upstream_close_mid_body_aborts`, `close_delimited_clean_end` (R-12), `public_close_mid_request_aborts` (7) | `phase`, `request_remaining`, `response_state`, `response_remaining`, `public_half_closed`, `is_draining` (Pinned observable projection) | `pending DELIVER quint-conformance-relay_exchange` (the owner is a scaffold until DELIVER) |

Total `#[quint_test]` = 5 + 3 + 3 + 7 = 18, plus one `#[quint_run]` per driver (4), each authored ignored; all 18 bound runs are completable against the pinned shapes (B-1 is ruled, R-4). QC-1's `switch!` maps all seven conformance actions (`init`, `submit_evaluation`, `issue_complete`, `disable`, `advance_time`, `audit_fail`, `restart`); `idle` is absent from the conformance mains (R-7); there is no `_` arm and no empty handler. The key-update/transmit-lock model was dropped (R-19): the rekey exclusion has no driver and its Tier-3 oracle is S-PIG-106/107. Model-only runs not bound: `abandoned_reservation_rolled_back` and `receipt_waits_for_publish_guard` need a fault or pause seam and are not replayed (R-36, P-32); the publish-guard window is covered by the Tier-3 test of S-PIG-72, and the rollback exposure is stated and bounded (R-21; S-PIG-75 is dropped). The host/Sim equivalence tests (S-PIG-67, S-PIG-104) are Sim pairings, not conformance. There is no `unhealthy_is_no_backend` run (the arm ignores `healthy`, R-2). `cargo xtask conformance-lint` was recorded clean (rc 0) after batches B3a, B4a, B4b and B5.

**QC-4 driver specification (`relay_exchange`).** Spec `specs/quint/public-ingress-gateway/relay_exchange.qnt`, conformance main `relay_conf` (`relay_exchange(OFF = Set(), REC = true)`; the stutter `idle` exists only in `relay_ok`, so no step maps to "no call"). Driver struct: one field, the production `ExchangeMachine`, rebuilt at `init` with `ExchangeMachine::new()` (pinned: `Idle`, `ResponseState::Pending`, both remainings `None`, not draining, public peer not half-closed); no `Sim*`, no driver-maintained map, counter or literal. Every handler calls one owner method and, for the Result-returning methods only (`begin_request`, `request_octets_moved`, `response_head`, `response_octets_moved`, `finish`), checks the call returned `Ok` (a `FramingViolation` on a model-legal step fails the run: the six pinned variants are the driver's own call-sequence defects, never a model event); `peer_closed`, `drain_begin` and `force_abort` return `()`, have no result to check, and are verified by the per-step State comparison; a call is never made on a step the model forbids. `switch!` has no `_` arm; an unmapped action bails with its name.

| Spec action | `ExchangeMachine` call (the single pinned table) | Handler detail |
|---|---|---|
| `read_head(n)` | `begin_request(false, BodyFraming::Length(n))` | `n = 0` leaves `request_remaining() == None` |
| `req_unit` | `request_octets_moved(1)` | one model unit is one octet |
| `resp_head(ld, n)` | `response_head(Length(n))` when `ld`, else `response_head(UntilClose)` | the `ld == false` arm is the close-delimited response (R-12) |
| `resp_unit` | `response_octets_moved(1)` | |
| `up_close` | `peer_closed(Side::Upstream, true)` | |
| `conclude` | `finish()` | evaluates `reusable()` before the call and asserts the returned `Disposition` is `Reuse` exactly when it held |
| `p_close` | `peer_closed(Side::Public, true)` | |
| `drain_begin` | `drain_begin()` | |
| `force_abort` | `force_abort()` | |

| Model variable | Accessor | Mapping |
|---|---|---|
| `phase` | `phase()` | `Idle`/`Relay`/`Closed` one to one |
| `req_left` | `request_remaining()` | `Exactly(k)` is `k`, `None` is 0 |
| `resp_state` | `response_state()` | `Pending`/`Body`/`Complete` ↔ `RNone`/`RBody`/`RDone` |
| `resp_left` | `response_remaining()` | `Exactly(k)` is `k`; `None` and `UntilClose` are 0 |
| `len_delim` (P-13d) | `response_remaining()` | true unless `UntilClose` |
| `draining` | `is_draining()` | boolean |
| `p_eof` | `public_half_closed()` | boolean |

Seven model variables, six accessors; `reusable()` is a derived accessor used for the `conclude` precondition and run expectations, not a State field; `exch`, `overread`, `ud_units`, `conf_step` are model-only and never projected. Values the driver must see (spec `closedS` and the reuse branch of `conclude`): in `Closed`, `request_remaining()` and `response_remaining()` are `None`, `response_state()` is `Pending`, `len_delim` is true, `is_draining()` and `public_half_closed()` keep their values; `drain_begin` from `Idle` gives `Closed` with `is_draining()` true; `peer_closed(Public, true)` from `Idle` gives `Closed` with `public_half_closed()` false; `finish()` returning `Reuse` leaves `Idle` with `response_state() == Complete` and both remainings `None` until the next `begin_request`. The seven runs are `exchange_cl_keepalive`, `early_response_closes`, `drain_with_inflight_body`, `half_close_after_request_read`, `upstream_close_mid_body_aborts`, `close_delimited_clean_end` (R-12) and `public_close_mid_request_aborts`; the invariants checked at every step by the State comparison and in the run expectations are `NoBytePastMessageBoundary` and the `reusableOf` expectations of the runs (no reuse after a mid-body close), with the hazard `relayPastBoundary` model-only. `#[quint_run]` randomized traces print their seed. Chunk framing is not modelled and is covered by S-PIG-80/84 PBT.

**QC-1..QC-3 action and projection record (as pinned by the DESIGN "Conformance action maps for the three older mains"; every action of the conformance mains is mapped).** The conformance mains carry no stutter step: `idle` is absent from all three (R-7). For QC-2 the **owner-autonomous steps** `stage` and `retire` stay explicit and are accepted as pinned (R-7): the owner stages and retires by itself; the driver runs the owner on a paused single-thread runtime, yields until the owner is idle and asserts the step's own postcondition through `application_status()`.

| Driver | Invariants checked (spec) | Owner field | Action -> pinned call | State field -> accessor |
|---|---|---|---|---|
| QC-1 `identity_conf` | `HoldImpliesAudit`, `StaleEpochNeverHolds`, `DisabledSlotIsEmpty` (`Safety`; the spec has no `AtMostOneHeld` or `DisableClearsOrTimesOut`) | production `ControlPlaneGatewayIdentityLifecycle` over a real `GatewayIdentitySlot`, `EvaluationBroker` and `GatewaySvidLifecycle`; Sim CA, `SimObservationStore`, `SimClock` at their driven ports | `submit_evaluation` -> `GatewayIdentityLifecycleControl::ensure_current`; `issue_complete(e)` -> `action_shim::gateway_svid::dispatch_issue`; `disable` -> `disable_after_drain` (a synchronous clear); `advance_time` -> `SimClock::advance`; `audit_fail` -> failing `SimObservationStore` write for the next `IssuedCertificate` row; `restart` -> drop the owner composition and rebuild it (`GatewayIdentitySlot::new`, new `EvaluationBroker`, `ControlPlaneGatewayIdentityLifecycle::new`) over the same `SimObservationStore` and CA serial counter, discarding queued actions and the pending `ensure_current` future. `submit_evaluation` polls `ensure_current` once, drains the broker through the production `GatewaySvidLifecycle` (the driver never constructs an `Action`) and asserts that no emitted, not-yet-completed `IssueGatewaySvid` carries an epoch beyond `desired()`'s (no model state reaches a handler; `issues` stays model-only, R-31; P-37 was rejected on 2026-11-04; `issue_complete(e)` fails the step when no Issue of epoch `e` is pending and drops the pending Issues of epoch `e`; a duplicate or early Issue shows through `audited`) | `desired_epoch`, `desired_enabled` <- `GatewayIdentitySlot::desired()`; `held` <- `GatewayIdentitySlot::current()`; `audited` <- `ObservationStore::issued_certificate_rows` filtered to the gateway SPIFFE id; `now` model-only |
| QC-2 `app_conf` | `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` | real `GatewayBuilder`/`GatewayApplicationOwner` composition over a real redb Route store, `SimClock`, and the driver's **private doubles** of the four gateway-crate ports (DG-7; the connect and client doubles carry the held-probe capability, Scaffolds) | `route_declare`/`route_withdraw` -> `PublicRouteSetHandle` via `GatewayRouteCommands::routes()`; `probe_result(tok, ok)` -> the staged gate's result delivered by releasing the entry held at an awaited operation the gate owns, addressed by the gate it parked (DG-9, R-65); `open_connection`/`close_connection(gid)` -> `GatewayAdmission::acquire_connection` / drop of the kept `GatewayConnectionLease` (the `Arc<GatewayAdmission>` is injected through `GatewayAdmission::new` and `start_with`, DG-10); `stage`, `retire(gid)` -> owner-autonomous, **no driving-port call** (yield until idle; postconditions `application_status().staged` is `Some` / `draining` no longer lists `gid`; accepted, R-7). A superseded `tok` must leave current, staged and draining unchanged (the modelled discard) | `current_generation`, `draining_generations` <- `GatewayStatusReads::application_status()`; `staged`, `leases` model-only |
| QC-3 `receipt_conf` | `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `RollbackPreservesPriorGeneration`, `EmptyIsNoBackend` (checked at the State comparison; the rollback path is model-only, R-36) | the real `EbpfDataplane` (both `Dataplane` and `GatewayConnectDataplane`) with the real selection arm, native metal | `update_apply`, `update_rejected` -> `Dataplane::update_service` (rejection is the real `DataplaneError::BackendIdentityConflict`); `open` -> `open_upstream`; `kernel_connect_det` -> the driver's deadline-bounded `connect(2)` on the socket from `PreparedUpstream::into_parts`; `take(c, c)` -> `take_receipt`; `read_identity` -> `selected_backend_identity`; `cleanup` -> `cleanup`; `restart` -> drop and re-create `EbpfDataplane` on the same pin directory | `identity[id]` <- `selected_backend_identity(BackendId)`; `intents` <- `live_intent_count()`; `receipts` <- `GatewayConnectCleanup::live_receipts()` per cookie; the receipt value is checked from the `take_receipt` return at its handler and the spec has no `idres` variable |

**Conformance completeness-critic checklist (per driver; `.claude/rules/testing.md`).** Specification-level answers with their citations; each is re-verified against the written driver body at DELIVER activation, and the `cargo xtask conformance-lint` rules (items 1, 3, 5, 6, 7) run over every file.

| # | Item | QC-1 | QC-2 | QC-3 | QC-4 |
|---|---|---|---|---|---|
| 1 | In the owner crate's `src/` behind `#[cfg(all(test, feature = "integration-tests"))]`, no always-false cfg, reasoned ignore | yes: Formal protocol model § `gateway_identity` "Conformance placement (D-PASS7-1)" | yes: Scaffolds table row QC-2 (`overdrive-gateway/src/application/quint_gateway_application.rs`, a child of `application`; Formal protocol model § `gateway_application` "Conformance placement and test-local doubles (DG-7)"; reasoned ignore `pending DELIVER quint-conformance-gateway_application`); DG-8..DG-10 are closed and confirmed (R-18); the driver binds its own distinct `127/8` address on port 443 (DG-12 ruled, R-6) and joins no nextest group for the public bind | yes: § `selection_receipt` "Conformance owner (D-PASS7-2)" | yes: § "Pinned observable projection" and this table |
| 2 | Driver struct holds the production owner | `ControlPlaneGatewayIdentityLifecycle` over a real `GatewayIdentitySlot`, `EvaluationBroker`, `GatewaySvidLifecycle` (D-PASS7-1) | real `GatewayBuilder`/`GatewayApplicationOwner` composition and redb Route store, with driver-local private doubles of the four gateway-crate ports (DG-7); the module imports no `overdrive_sim` item that implements a gateway-crate trait (a grep for `overdrive_sim::` checks it); the `Arc<GatewayAdmission>` is injected (`GatewayAdmission::new`, `start_with`, DG-10), so the driver holds it with no accessor added | the real `EbpfDataplane` (D-PASS7-2) | `ExchangeMachine` (QC-4 specification above) |
| 3 | Every action of the spec `step` relation maps to an owner driving-port call, no empty handler, no `_` arm | yes: all seven conformance actions call a pinned owner port (`ensure_current` polled once, `dispatch_issue`, `disable_after_drain`, `SimClock::tick`, a Sim store fault, owner rebuild); `idle` is absent from the conformance main (R-7) | yes with one accepted exception: `route_declare`, `route_withdraw`, `probe_result`, `open_connection`, `close_connection` call pinned ports (DG-8..DG-10 are closed); **`stage` and `retire` (owner-autonomous) make no driving-port call**, which R-7 accepts (DESIGN "`app_conf`" table); `idle` is absent from the conformance main | yes: the eight actions of the map call `Dataplane`/`GatewayConnectDataplane` ports; `idle` is absent from the conformance main (R-7; DESIGN "`receipt_conf`") | yes — nine actions, nine calls; `idle` absent from `relay_conf` |
| 4 | Each `State` field has a pinned accessor, named | `GatewayIdentitySlot::{desired,current}`, `ObservationStore::issued_certificate_rows` | `GatewayStatusReads::application_status()` | `selected_backend_identity`, `live_intent_count`, `GatewayConnectCleanup::live_receipts` | `phase`, `request_remaining`, `response_state`, `response_remaining`, `public_half_closed`, `is_draining` |
| 5 | `from_driver` reads no `Sim*` adapter (except a marked arbiter read with a paired Tier-3 test), no driver map, counter or literal | yes: the observation read goes through a field typed `Arc<dyn ObservationStore>` (the core port trait; `SimObservationStore` is only what it is constructed with and is never named in `from_driver`), the slot read through `GatewayIdentitySlot` | yes: `from_driver` reads only `application_status()`, never a double | yes: no `Sim*` anywhere (D-PASS7-2) | yes: no `Sim*` exists in the module |
| 6 | Spec referenced in place; a `#[quint_test]` per critical named run; `#[quint_run]` present | 5 runs + 1 | 3 runs + 1 | 3 runs + 1 (the runs needing a fault or pause seam are not replayed, R-36, P-32) | 7 runs + 1 |
| 7 | Target module has a `step` relation; the spec is not evaluated by the `quint` CLI | yes (`step` relation) | yes (`step` relation) | yes (`step` relation) | yes (`step` relation) |

Item 3 is credited per the rulings of 2026-11-02: the stutter `idle` is absent from the three older conformance mains and QC-2 `stage`/`retire` are accepted as owner-autonomous steps (R-7), and B-1 is ruled (R-4: `disable` clears synchronously, so `settle_ok`/`settle_timeout` do not exist). QC-4 has no stutter and no autonomous step. The per-driver checklist is re-verified against the written driver bodies at DELIVER activation.

**Examples.** `examples/public-ingress-gateway/` is a VM Service journey in the style of `examples/service-kind-vm-workloads/`: `service.toml` (`[vm]`, never `[exec]`), `prepare.sh`, an example-local guest HTTP/1.1 server (`/stream` chunked with `X-Workload: api`, `/exact` with `Content-Length`, a request-count marker), `prepare-test-ca.sh`, `route.toml.template`, `README.md`, and `run-example.sh` with the exact argv contract in "Artifact-boundary handoff" (hermetic mode curls with `--cacert`; production-trust mode with system trust; both only via `curl --resolve`). The scripts are authored and are syntax- and shellcheck-clean only; none has been run. The pre-reconstruction shell runner, regression scripts, `receipt-oracle.py`, `workload.py` and the GitHub workflow are not carried.

**E15.** `verification/expectations/E15-public-ingress-operator-journey/` holds the authored claim (`README.md`: expectation, four `- Anchor:` lines, exact public boundary, observable refutation conditions, runner contract), the `execution-substrate` file (`native-metal`), the INDEX row (status `pending`) and `runner.sh`. `runner.sh` and the harness `EXPECTATION_SOURCE_SHA`/`EXPECTATION_SOURCE_DIRTY` capture pinned in the Artifact-boundary handoff are authored; the runner has not been run, no evidence exists and nothing is marked satisfied. It is a point-in-time capture against the built default-feature binary on native metal with a clean harness-captured HEAD, exact-object BPF embedding proof, `curl --resolve` and normal system trust, plus independent `ss -tni` and `/proc/net/tls_stat` samples that show kernel TLS on both legs while the response streams; reviewed by a different agent; not a test, never rerun as a gate.

## Wave: DISTILL / [REF] Test placement

Pure Route and value properties are source-local in `overdrive-core` (public signatures are the driving ports). Authority, header, limit, connector, **head-validation, framing, chunk-line, interim-head, deadline-composition and `ExchangeMachine`** properties are source-local in `overdrive-gateway` (`runtime::{routing,proxy_headers,limits}` and `runtime::http1`; DESIGN keeps them crate-private, so only source-local tests reach them). The DESIGN pins the head validator, parsers, framing decision, chunk-line parsers and deadline composition as crate-private pure functions (R-32, "Pure `http1` and relay-pipe functions"), so the source-local bodies of S-PIG-76, 77–83 (U half), 85 are authored against them; the two arms observable only end to end (S-PIG-77 and S-PIG-78) stay at the public listener in `public_ingress_gateway_relay.rs` (lane I), and S-PIG-83's I half likewise. The pipe-floor law (S-PIG-86) is source-local in `overdrive-dataplane::gateway_relay` against `PIPE_FLOOR`, `PipeCapacity::new` and `splice_in_len` (R-32). CLI parsing uses the existing `overdrive-cli` acceptance entrypoint. Real redb and application composition use the `integration-tests`-gated `overdrive-gateway` integration binary, which also hosts the Sim-relay exchange file `tests/integration/public_ingress_gateway_relay.rs` (S-PIG-77 and 78 (listener arms), 83 (I half), 87–99, 101, 118 (server configuration) and 119–122: the production `GatewayBuilder` and listener, `SimGatewayRelay`, `SimClock`, and test-local connect-dataplane, client-mTLS and identity doubles; the file imports no `SimGatewayConnectDataplane` and owns second octets 68..=96). Boot, shutdown and operator-surface composition use `overdrive-control-plane` and `overdrive-cli` integration binaries through `SimDataplaneComposer` (`NodeAdapters` swap; no `*_override`), including the relay gate (S-PIG-100); the enabled-composition arms (S-PIG-02 I, 34, 35, 42, 57 I, 58, 60 I, 61, 62, 74, 100 enabled) are unblocked by R-20 and are authored in those two binaries beside the disabled-composition rows (S-PIG-59, 100 disabled arm, 36) and the CLI rows 18–20. Real-kernel hook, packet-entry and exact-peer wire tests use the `overdrive-dataplane` integration binary on native metal; **the host relay Tier-3 suite (`tests/integration/gateway_relay.rs`, S-PIG-105–112, 114, 115, 117, 118, 123) and the equivalence file `tests/integration/gateway_relay_equivalence.rs` (S-PIG-104) live in that same binary, which `.config/nextest.toml` already assigns wholly to the `host-kernel-shared` group** (filter `package(overdrive-dataplane) & binary(integration)`; the dataplane lib-binary assignment and the S-PIG-115 T3 slow-timeout entry landed with the relay batch, and the conformance-binary assignment landed with the end-to-end binary because nextest rejects a filter naming a binary that does not exist yet). The cross-owner production walking skeleton and its native cases use `tests/conformance` (`native_metal_kvm_public_ingress_gateway_*`, selected by `tests/conformance/run-native-metal.sh`), including the production-path arms of S-PIG-105, 109, 113 and 115. The Sim-relay exchange rows 119–122 live in the gateway integration file with S-PIG-87–101. Ordering, late completion, restart, shutdown and the **seeded relay exchange runs (S-PIG-102, 103, `overdrive-sim/tests/acceptance/gateway_relay_exchange.rs`, no new `Invariant` variant)** use the `overdrive-sim` crate. Quint drivers sit in their owners' `src/`: QC-1 at `overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs`, QC-2 at `overdrive-gateway/src/application/quint_gateway_application.rs` (driver-local doubles of the four gateway-crate ports, DG-7), QC-3 at `overdrive-dataplane/src/quint_selection_receipt.rs`, QC-4 at `overdrive-gateway/src/runtime/http1/quint_exchange.rs`. No test spawns the `overdrive` binary; only E15 drives the built CLI.

**Nextest configuration (`.config/nextest.toml`; applied except where stated).** Two parts: (A) whole-binary `host-kernel-shared` assignments and (B) `slow-timeout` overrides.

**(A) Whole-binary `host-kernel-shared` assignments.** `.config/nextest.toml` already puts whole binaries in the `host-kernel-shared` group (`max-threads = 1`) with catch-all filters, never module or test names (`.claude/rules/testing.md` "Node-global kernel state requires a catch-all nextest group"): `package(overdrive-dataplane) & binary(integration)`, `package(overdrive-control-plane) & binary(integration)`, `package(overdrive-control-plane) & kind(lib)` (the precedent for a library test binary that hosts a kernel-touching conformance driver) and, for the conformance crate, only `package(overdrive-system-conformance) & binary(shared_guest_network_fail_stop_recovery)`. Two whole-binary assignments were added for this feature, both applied (the earlier gateway and `overdrive-sim` entries would have been assigned only because of the public bind; R-6 removes that reason, so they do not exist):
1. `filter = 'package(overdrive-system-conformance) & binary(public_ingress_gateway_end_to_end)'`, `test-group = 'host-kernel-shared'`: the new end-to-end binary boots `run_server` with real adapters and a real VM, so it mutates the node-global nft, cgroup and bpffs state that the other binaries mutate; the existing conformance filter names a different binary. A gateway binary joins the group only for node-global kernel state it mutates (R-6).
2. `filter = 'package(overdrive-dataplane) & kind(lib)'`, `test-group = 'host-kernel-shared'`: the `overdrive-dataplane` library test binary hosts QC-3 (`quint_selection_receipt`: the real `EbpfDataplane`, bpffs pins, the root cgroup attach and `/proc/net/tls_stat`); every existing dataplane entry matches `binary(integration)` or a `test(...)` module list under it, so the lib binary was unassigned before this feature.
Why the gateway and `overdrive-sim` binaries are not in the list (R-6): every gateway composition that reaches promotion binds its own distinct `127/8` address on port 443, so no two compositions contend for a socket; the privileged-port bind needs root, which `cargo xtask metal run --` provides, and the Sim-only binaries need no KVM. A gateway binary joins `host-kernel-shared` only for node-global kernel state it mutates, and the `overdrive-gateway` library and integration binaries and the `overdrive-sim` acceptance binary mutate none. Not needed: the control-plane `binary(integration)` and `kind(lib)` entries and the CLI `binary(integration)` entry (QC-1, S-PIG-02 I arm, 34–36, 57–62, 74 and 100 are already covered as whole binaries), and any relay-specific group (the dataplane integration binary is already whole-binary).

**(B) `slow-timeout` overrides.** The default profile kills a test after `slow-timeout = { period = "60s", terminate-after = 2 }`, i.e. 120 s (`.config/nextest.toml:44`). The file widens it per test with an override that names the package, binary and test and carries a comment giving the bound (for example the `overdrive-sim` entries at `:229-230` and `:239-240` and the `overdrive-cli` regex entry at `:568-569`); the group assignment is a separate override, so both can apply to one test. The first matching override wins per setting, so the per-test entries precede one binary-level fallback. The new tests whose stated bounds exceed 120 s are below; `terminate-after` is the smallest integer strictly greater than the worst-case sum divided by 60 s (the kill fires at `period x terminate-after`, so equality would kill a test at its own bound), the sum being the stated bounds of the row (Bounds tables). N tests that deploy a VM Service include the row 01 readiness window of 120 attempts of up to 5 s each, 600 s in the worst case (the one-second spacing is start-to-start, so the worst case is attempt-bound), plus one 5 s drain. The figures are derived from the stated bounds, not measured; the first recorded metal run replaces each with its measured runtime and the override comment names that measurement, as the cited entries do. Filters take the form `package(<crate>) & binary(<binary>) & test(<name>)` with the final test names (`native_metal_kvm_public_ingress_gateway_*` in the conformance binary). Applied: the conformance entries (13, 12 and the binary-level fallback 11), the S-PIG-115 T3 arm (3, two tests) and S-PIG-24 (22). S-PIG-64 needs no entry (one matching-identity case, 5 s dial + 10 s capture = 15 s, under the default kill). S-PIG-75 has no entry (dropped, R-21).

| Test (scenario) | Binary | Worst-case sum of the stated bounds | `terminate-after` |
|---|---|---|---|
| S-PIG-01 | `public_ingress_gateway_end_to_end` | 600 s readiness + 120 s request + 5 s drain = 725 s | 13 |
| S-PIG-69 N arm | same | 600 s + 120 s request + 10 s owner recovery + 5 s drain = 735 s | 13 |
| S-PIG-71 N arm | same | 600 s + 60 s recovery window + 2 x 5 s drain (serve stops, restarts) = 670 s | 12 |
| S-PIG-109 N arm | same | 600 s + 60 s + 5 s drain = 665 s | 12 |
| S-PIG-113 | same | 600 s + 120 s + 5 s (129th connection) + 5 s drain = 730 s | 13 |
| S-PIG-115 N arm | same | 600 s + 120 s + 5 s drain = 725 s | 13 |
| S-PIG-56 N, 60 N, 70 (the two 408 cases are separate tests; the stalled request body is the longest at 40 s), 105 N | same | at most 600 s + 40 s + 5 s = 645 s | 11 (the binary-level fallback entry, filter `package(overdrive-system-conformance) & binary(public_ingress_gateway_end_to_end)`, placed after the per-test entries) |
| S-PIG-115 T3 arm | same | 120 s (the stated bound equals the default kill, so it needs at least one more period) | 3 |
| S-PIG-24 | `overdrive-gateway` `integration` | 256 rounds x 5 s per-round cap = 1 280 s (a cap; the expected wall-clock is milliseconds per round) | 22 |

The native-metal rows not listed (104, 105 T3, 106 to 108, 110 to 112, 114, 117, 118, 123, 72) have stated bounds of at most 60 s each (row 104 runs scripts of at most 30 s each, one script at a time, so each script is its own test; S-PIG-109 T3 is at most 60 s) and keep the default.

Verification of the applied edit: `cargo xtask metal run -- cargo nextest show-config test-groups --profile default` and `--profile ci` were run after the relay, hook and end-to-end batches and returned rc 0 with the dataplane lib and integration binaries and the `public_ingress_gateway_end_to_end` tests under `host-kernel-shared`; `--profile mutants` was not run. The `overdrive-gateway` library and integration binaries and the `overdrive-sim` acceptance binary are deliberately absent from the group (R-6). The slow-timeout figures are derived from the stated bounds, not measured; the first recorded metal run replaces each. The smallest multi-binary lane that shares the kernel state (the dataplane integration binary beside the conformance binary) has not been run.

## Wave: DISTILL / [REF] Driving-adapter coverage

| Driving surface | Scenario / mechanism |
|---|---|
| `overdrive serve` four gateway flags | S-PIG-18–20 (clap and config, in-process); S-PIG-59 and the disabled arm of S-PIG-100 through `ServerConfig.gateway` (the field `serve` fills; S-PIG-60 enabled arms are authored and ignored); E15 through the built binary |
| `overdrive deploy <SERVICE_SPEC>` (VM Service) | S-PIG-01 via the exported handler's public API; E15 via the example |
| `overdrive deploy <ROUTE_SPEC>` | S-PIG-34/35 (`deploy_resource` and `ApiClient` against the production handler; **authored, ignored `pending DELIVER route-operator-surface`**); S-PIG-36 (the disabled composition, authored); the materialized checked-in template in S-PIG-01 |
| `DELETE /v1/routes/{id}`, `GET /v1/gateway/status` | S-PIG-21, 23, 25, 33 (through the Route Set owner); 35 and 42 through the operator handlers (authored, ignored); the disabled-composition DELETE of S-PIG-36 through `ApiClient::withdraw_route` (R-53) |
| Public TLS 1.3 / HTTP/1.1 listener | S-PIG-01, 56, 69, 70, 113, 115 (real TLS); S-PIG-02 (I arm), 39, 40, 87–101, 119–122, 102, 103 (client socket through the Sim relay); S-PIG-123 (real handshake refusals); E15 |
| Serve shutdown and fail-stop | S-PIG-26, 60, 61, 32, 101 |

## Wave: DISTILL / [REF] Production-entry-point obligations

The vertical slice is driven through `overdrive serve` + `overdrive deploy` in process, never by spawning the binary: `AT-PIG-E2E-1` boots `run_server` with `NodeAdapters::host` and `GatewayAdapters::host` (no `*_override`) and deploys a VM Service and the Route through the exported handler's public API; every request enters through the production listener, whose `accept_public` arms kernel TLS, whose connector prepares and dials through the registered-socket hook, and whose upstream arm is `HostGatewayClientMtls` through the same `HostGatewayRelay`. The relay probe and the connect probe run in the production gate task; no test calls either against a hand-built owner as the stand-in for the slice. S-PIG-104–112, 114, 115, 117, 118 and 123 are adapter-contract tests of `HostGatewayRelay` through its port (Mandate 6); each relay mechanism has a production-path exercise through S-PIG-01, 105, 109, 113 and 115 in the conformance file (105, 109, 113, 115 and S-PIG-01, 70) so no relay mechanism is proven only in isolation. No integration test installs a rule, binds a socket, programs a map, arms a socket or supplies an address that production does not install, bind, program, arm or supply itself. The only black-box, binary-driving artifact is E15 (`curl --resolve api.example.com:443:<ip> https://api.example.com/` against a real deployed VM Service in the hermetic example; the production-hostname form in E15), a point-in-time capture per `.claude/rules/verification.md`, never a test.

## Wave: DISTILL / [REF] Prerequisites

- Native x86_64 bare-metal KVM host provisioned per `infra/metal`, uid 0, fail-closed on the substrate probes (`.claude/rules/testing.md`), kernel 7.0.0-38 (the line the G1 spikes ran on; ADR-0068 pins 7.0). No Lima instance and no CI metal lane (P-6 is PROPOSED). Real-kernel and VM scenarios run through `cargo xtask metal run --`; the default and `integration-tests` lanes compile and run the in-process scenarios. The metal image must provide the `tls` ULP, `splice` and `F_SETPIPE_SZ` as the relay probe requires; a node that does not is the case S-PIG-99/114 refuse. The in-process scenarios that let the gateway reach promotion (the `overdrive-gateway` library and integration binaries, the `overdrive-sim` acceptance binary, and the control-plane and CLI integration binaries) bind the real TCP/443, so they run as root (the privileged-port bind) through `cargo xtask metal run --`; each binds its own distinct `127/8` address (R-6) and none is serialised for the public bind.
- BPF object produced by `cargo xtask bpf-build` before anything that compiles `overdrive-dataplane`; the separate gateway selection `cgroup/connect4` program (R-1) is not scaffolded and DELIVER adds it, so the hook scenarios are RED for the right reason; the object must be rebuilt after the new map file (`cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`).
- Existing production owners compose the rest: workload-identity CA, Service deploy, ordinary `ServiceMapHydrator`, leg-C -> kTLS -> `GuestStreamDial`, the node-shared nft owner. Tests do not substitute their effects.
- Operator inputs: a complete leaf-first chain and matching PKCS#8 key (E15), or the example's bounded test CA (hermetic mode). Tools for independent oracles: `bpftool`, `ss`, `nft`, `curl`, `openssl`, `/proc/net/tls_stat`.
- User rulings of 2026-11-02 (R-1..R-19) cover D5 items 1–13 (item 12 reconfirmed), P-2, P-9, P-10, B-1, DG-11, DG-12, QC-1..QC-3 handling, the DG-7 placement and the DG-8/DG-10 item 13 entries, and the G1 proposals P-12, P-13a–d, P-17, P-18 (as authored), P-16 (not adopted) and the key-update model (dropped). The observation point reads and row variants are previously approved (2026-09-13). The user ruled on 2026-11-03 (R-20..R-44) on the former proposals P-20..P-31 and the scope items: the client-mTLS factory crate cycle is broken by R-20 (the scaffolds and tests of S-PIG-02 I arm, 34, 35, 38, 42, 43, 52, 57 I arm, 58, 60 I arm, 61, 62, 64, 74, 100 enabled arms are authored and ignored); S-PIG-75 is dropped (R-21); the S-PIG-114 ULP-absent stimulus is dropped (R-23); S-PIG-123 stays (R-39); the `DataplaneError` variant placement (R-35), the pure `http1` and pipe-floor signatures (R-32) and the response-head budget start (R-37) are pinned. The optional P-1, P-3, P-4, P-6, P-8 and P-32 remain PROPOSED (DESIGN, "PROPOSED — awaiting user").

## Wave: DISTILL / [REF] Outcome registration

No new outcome row is introduced. The scenarios refine the registered contracts `OUT-PIG-CERTIFIED-KEY-CUSTODY`, `OUT-PIG-GATEWAY-APPLICATION`, `OUT-PIG-BPF-SELECTION-IDENTITY`, `OUT-PIG-GATEWAY-EXACT-PEER`, `OUT-PIG-PUBLIC-REQUEST`, `OUT-CA-SVID`, `OUT-WIM-SVID-LIFECYCLE`, `OUT-MTLS-HANDSHAKE-FAIL-CLOSED`, `OUT-MTLS-COMPOSED-PROXY-SKELETON`, `OUT-MTLS-INBOUND-SERVER-MTLS` and `OUT-MTLS-WIRE-TLS13`; the G1 relay scenarios refine `OUT-PIG-PUBLIC-REQUEST` (streamed response, typed status mapping) and `OUT-PIG-GATEWAY-EXACT-PEER` (arming only after equality). The registry carries no demand/revision/acknowledgment text (a search finds none); the only stale registry content was the `OUT-PIG-PUBLIC-REQUEST` artifact path (the removed shell runner), now `tests/conformance/tests/public_ingress_gateway_end_to_end.rs`. `nwave-ai outcomes check-delta` reports 12 outcomes checked, 0 collisions, exit 0.

## Wave: DISTILL / [REF] Completeness audit

Specification-level credits, recomputed after batches B1–B5 authored the bodies (they compile under `integration-tests`; apart from the expected-GREEN rows no body has been run to GREEN, because every other body is ignored until DELIVER). A credit means the scenario is specified and, where "Authoring status by batch" says so, its body is authored; it does not claim the behaviour passes. Nothing below is waived. Items are the 15 of the AT-completeness checklist.

| Check | Status | Evidence or gap |
|---|---|---|
| C1a empty/zero/minimum | Specified | All-absent config, empty withdrawal, zero-chain, empty set, no-outer-entry `NoBackend`; zero-length `Content-Length` body (S-PIG-87), no-body statuses (S-PIG-89), empty chunk list, zero headers |
| C1b partition boundaries | Specified | Labels 62/63/64, epochs, chain 0/1/N/9, ledger 0/1/128/129 (S-PIG-37), limits; head 32 KiB at/over, 100/101 fields, body ceiling at/over (S-PIG-82, 94), pipe capacity cap-1/cap/cap+1/2cap/2cap+1 (S-PIG-105), `u64` hex and length overflow (S-PIG-77, 80), probe 64 KiB plus one octet (S-PIG-99); response head 32 KiB at/over, 100/101 response fields, declared response length 16 MiB at/over (S-PIG-119); `response_head_deadline` one tick either side of 5 s (S-PIG-120); body stall 29 s versus 30 s (S-PIG-121); 128th/129th public connection (S-PIG-113) |
| C2a state machine documented | Specified | Gate G-1..G-6, Quint modules (four), `ExchangeMachine` phases/halves table (Exchange state machine). Identity disable settlement: closed by R-4 (`disable_after_drain` is a synchronous clear and `gateway_identity.qnt` models it). The enabled-boot arms of the gates are authored and ignored |
| C2b illegal event per state | Specified | Stale epoch, competing claim, wrong-id withdrawal, late probe, duplicate probe completion (S-PIG-73), restart; octets beyond the remainder, `finish` before complete, events after `Closed`, `drain_begin` twice (S-PIG-84); close_notify/reset/alert in each exchange phase (S-PIG-90, 92); public half-close after the request was fully read versus idle versus mid-body (S-PIG-122) |
| C3 zero/one/many | Specified | S-PIG-45, header PBT, ledger, Route 0/1/N; chunks 0/1/many, requests per connection 1/2/100 (S-PIG-87, 96), concurrent exchanges 1/128 (S-PIG-113) |
| C4a apply twice | Specified | Restated as the consuming-port form. Authored: custody replay across a restart and an identical Route re-apply (S-PIG-54, gateway integration file), the I arm of S-PIG-02 run twice with the empty second delta, two consecutive KeyUpdates and two identical keep-alive requests (S-PIG-106, 87). S-PIG-54 has no Tier-1 connector arm |
| C4b inverse without prerequisite | Specified | S-PIG-33; `finish` before the response completes and `close_write` after close (S-PIG-84, 92); a read after close_notify is `CleanEof` (S-PIG-108) |
| C5a mode combinations | Specified | 16 enablement masks, hermetic vs production-trust, exact vs prefix Route; framing matrix {CL, chunked, none, close-delimited} x {request, response} x {HEAD, 204, 205, 304, 1xx} (authored through S-PIG-77–83, 87–91, 119; the pure derivation S-PIG-76 is authored as a source-local test against the R-32 signatures); KeyUpdate {requesting, not} x {head, chunk line, boundary, body} x {AES-128, AES-256} (S-PIG-106) |
| C5b orthogonality | Specified, credited with a named residual | S-PIG-59 (disabled composition leaves boot unchanged), S-PIG-100 disabled arm, `Connection: close` does not alter framing (S-PIG-81). Residual: S-PIG-34 (both detach modes), S-PIG-62 and the enabled arm of S-PIG-100 are authored and ignored; without this credit the count is 13 of 15 |
| C6a malformed parameters | Specified | Route fields, `target.service`, authority, path, hostname, argv, key files; bare CR/LF/NUL, obs-fold, hex overflow, repeated headers (S-PIG-77–81) |
| C6b every declared error | Specified (credited with named residuals) | S-PIG-50/51 matrices follow the new `GatewayConnectDataplaneError` variants and `ensure_current`/`disable_after_drain`; the relay taxonomy is triggered per variant: `KeyUpdateError::{RustlsRefused, RearmRx, RearmTx, Io}` by the scripted failed KeyUpdate answer (S-PIG-90, Sim only), every `KtlsArmError` kind by the scripted arm failure at the upstream arm and at `accept_public` (S-PIG-98, Sim only; `UlpInstall` also by S-PIG-114 on a real kernel), every `PublicHandshakeError` variant by S-PIG-98 (`Arm`), 118 (`Tls`) and 123 (`AlpnMismatch`, `ServerNameMissing`, `Timeout`, `Io`; the variant per stimulus is pinned at DELIVER), `RelayProbeError` by S-PIG-99/114, `RelayPipeError` by S-PIG-95, `LegReadError`/`RelayError` by S-PIG-90–94, 108, 111, 119–121. Named residual: `LegWriteError::{PeerClosed, Io}` and `LegReadError::Io` have no dedicated stimulus; the seeded resets of S-PIG-102/103 hit them and the exhaustive close classification maps them |
| C6c closed error set | Specified | Exhaustive classifier per set; status projection exhaustive; the 400/408/413/421/431/502/503/505 mapping is an exhaustive match; 400/421/431/505 are asserted at the pure-function level (S-PIG-82), 408/413/502/503 at the listener and relay (S-PIG-92–96, 115) |
| C7a degraded resource | Specified | Unreadable key, CSPRNG, observation fault, NoBackend, cleanup failure, full ledger (the boundary-2 causes of S-PIG-74 are authored and ignored); pipe creation failure (S-PIG-95), stalled reader memory (S-PIG-111), 128-exchange descriptors (S-PIG-113) |
| C7b interruption | Specified | Lag, hold/release probe, withdrawal, restart, mid-request flow-owner restart, serve restart (S-PIG-71); peer close_notify/reset mid-head/body, drain and force-abort (S-PIG-90, 92, 101, 103), stall deadlines (S-PIG-96, 111, 120, 121), lingering-close interruption (S-PIG-115) |
| C7c concurrent actors | Specified | S-PIG-24; KeyUpdate against a writer (S-PIG-107, Tier 3 only; no model, R-19); 128 concurrent exchanges (S-PIG-113); seeded schedules (S-PIG-102, 103) |

Verdict at specification level (mechanical count, recomputed by the author after R-4 closed B-1; the reviewer recomputes it): **14 of 15 items credited** (C1a, C1b, C2a, C2b, C3, C4b, C5a, C5b, C6a, C6b, C6c, C7a, C7b, C7c); **1 not credited**: C4a (the Tier-1 connector arm is not authored). C6b is credited only because of the remediation additions (the two Sim capabilities of S-PIG-90 and S-PIG-98 and the new S-PIG-123); without S-PIG-123 the count would be 13. The count reaches the numeric COMPLETE threshold (>= 13) but **the audit does not call the specification COMPLETE while C4a is open**; the stated verdict is ACCEPTABLE_WITH_DOCUMENTED_GAPS, to be re-evaluated when the C4a arm exists. Gap classification (nw-at-completeness-check): DG-1..DG-10 are `SPECIFICATION_AMBIGUITY` findings routed to DESIGN and closed (DG-7 as a placement ruling; DG-8..DG-10 by the architect, confirmed with item 13, R-18); DG-11, DG-12 and B-1 are ruled (R-5, R-6, R-4). No `SPECIFICATION_AMBIGUITY` finding is open: the client-mTLS factory crate cycle (R-20), the `DataplaneError` variant placement (R-35), the signatures of the pure `http1` functions, the deadline composition and the pipe-floor law (R-32) and the response-head budget start (R-37) are pinned by the rulings of 2026-11-03. Open `AT_GAP_IN_DELIVERY_SCOPE`: the C4a connector arm (it can now be authored in the relay-driven file; its occurrence assertion needs a tracing capture seam the DESIGN does not pin).

**DISTILL completeness gate for cross-boundary criteria** (`.claude/rules/testing.md` "DISTILL — prove composed system behavior under deliberate failure"). Every cross-boundary scenario records its contract or invariant, production owner path, fault stimulus, outcome oracle and evidence tier: rows 01–74 in "Cross-boundary scenario obligations" (owner path, stimulus, fault, oracle, lane; the contract is the scenario-list column), and the G1 rows 83, 87–123 in the same section add an explicit contract/invariant column. Single-owner scenarios are pure (S-PIG-76–86 are pure functions or one-owner state machines) and carry only their lane. Fault bounds and restoration for the real-kernel faults are in "Native-metal (Tier 3) obligations". The Quint runs bound per invariant: `relay_exchange` (`NoBytePastMessageBoundary`; no reuse after a mid-body close through the run expectations) by QC-4; the rekey exclusion by the Tier-3 rows S-PIG-106/107; the older modules as in the driver table. Except for the arms named in the "Not authored" column of "Authoring status by batch", no cross-boundary criterion is covered only by an isolated test or a happy path: the kernel behaviours of the relay (record boundaries, rekey, lock, backpressure, lingering close, pipe lifetime) each have a Tier-3 row with a deliberate-fault stimulus and an independent oracle.

Documented gaps and blockers (no silent waivers):
1. **P-9 closed (R-2)** — the arm ignores `healthy`; the in-window oracle is 502 (S-PIG-56) and there is no `unhealthy_is_no_backend` run; the post-`healthy` 0 -> 1 leg is covered by S-PIG-02, 56 and 01.
2. **P-2 closed (R-3)** — the BackendId memo re-lease wedge is a recorded exposure (ADR-0241), not a scenario; `conflicting_alloc_rejected` (rebind rejected) is covered.
3. **P-10 closed (R-1)** — the separate `cgroup/connect4` program is DELIVER's; S-PIG-41/63 are placement-neutral. R-54 closes the gate gap: the program is pinned (`cgroup_connect4_gateway`), the hook behaviour is S-PIG-124 and 125, the verifier baseline is S-PIG-127, and the attach assertion is rig construction plus S-PIG-124's registered connect (gate 3); the performance measurement is the report-only receipt of R-55.
4. **Item 12** — reconfirmed by the user on 2026-11-02 (R-18); the `IntentSubscriptionEvent` type, the `watch` signature port and its implementer fallout landed with batch B1; S-PIG-48/66 are complete reasoned-ignored bodies and the lag behaviour is DELIVER's.
5. **Rulings of 2026-11-03 (R-20..R-44).** The former proposals P-20..P-31 and the scope items are ruled (DESIGN, "User rulings of 2026-11-03"); the follow-up authoring pass is listed in "Authoring status by batch" and in `distill/code-reconciliation-plan.md`. The optional P-1, P-3, P-4, P-6, P-8, P-32, P-33, P-34, P-35, P-36 and P-40 remain PROPOSED; P-37 was rejected and P-38 adopted by the user on 2026-11-04 (R-53); P-41 was ruled as R-55 on 2026-11-05. The rulings of 2026-11-04 (R-45..R-52, R-52 user-confirmed) are in "SVID issuance failure occurrence" (DESIGN).
5a. **Accepted exposure (R-21)** — the pre-commit rollback real-owner Tier-3 check (S-PIG-75) is dropped: the only production-reachable pre-commit failure input is `BACKEND_MAP` capacity (`crates/overdrive-bpf/src/maps/backend_map.rs:46`, `MAX_ENTRIES = 65_536`), a fill the check could reach only by programming 65 536 backends. `RollbackPreservesPriorGeneration` has model, equivalence and unit coverage. No seam is added (P-32 is the optional alternative).
6. **G1 proposals ruled as authored.** P-12, P-13a–d, P-17 and P-18 are ruled (R-8..R-14), P-16 is not adopted (R-16) and the key-update model is dropped (R-19); the scenario list carries no pending tag.
7. **DESIGN gaps DG-1..DG-12 and blocker B-1: all closed or ruled.** DG-1..DG-10 and the QC-2/QC-3 part of DG-6 were closed by the architect in the DESIGN sections and are confirmed with item 13 (R-18); DISTILL scaffolds the pinned shapes (Scaffolds). DG-11 is ruled (R-5), DG-12 is ruled (R-6) and B-1 is ruled (R-4).
   - **DG-1 closed** — "Cross-crate construction of the opaque leg values": `KtlsLeg::from_armed`, `KtlsReadHalf::backend_mut`, `KtlsWriteHalf::backend_mut`, `ThroughBytes::new`; one backend per composition; a wrong-backend `T` is `unreachable!()`.
   - **DG-2 closed** — D5: `HostGatewayRelay::new(clock)`, `SimGatewayRelay::new(clock)`, `SimGatewayClientMtlsFactory::new(relay, clock)`, `SimGatewayAdapters::{new, relay, client_mtls, sim_relay}`.
   - **DG-3 closed** — "Exchange state machine": `ExchangeMachine::new()`, `BodyFraming` (`NoBody`, `Length`, `Chunked`, `UntilClose`), `FramingViolation` (six variants) and the 502 status rule. Chunked PBT (`request_chunked_end`/`response_chunked_end`) and the QC-4 `init` are now writable.
   - **DG-4 closed (R-23)** — obligation 16(f): there is no real-kernel ULP-absent stimulus; each substrate lie is refused at Tier 1 and S-PIG-114 asserts the honest real-kernel pass.
   - **DG-5 closed** — the DESIGN pins `#[ignore = "pending DELIVER gateway-application-invariants"]`, the descriptive activation key DISTILL already used.
   - **DG-6 closed for QC-2 and QC-3** — "Conformance action maps for the three older mains" (stutter `idle`, owner-autonomous `stage`/`retire`). For QC-1 `restart` and `idle` are closed.
   - **DG-7 closed (placement ruling, no public item)** — the QC-2 driver is `overdrive-gateway/src/application/quint_gateway_application.rs` over driver-local private doubles of `GatewayConnectDataplane`, `GatewayRelay`, `GatewayClientMtls` and `GatewayIdentityLifecycleControl`, because `overdrive-sim`'s impls of crate-local traits cannot satisfy them in the library's `cfg(test)` build; the held-probe capability lives in the driver's connect double; `overdrive-sim` keeps its version for the seeded invariants and S-PIG-68; the `State` projection is unchanged. It is on the item 13 list confirmed by R-18.
   - **DG-8 closed** — "Cross-crate construction of `PreparedUpstream`": `PreparedUpstream::new(socket: tokio::net::TcpSocket, intent: GatewayConnectIntent)`, public; registers nothing, no cleanup guard, `intent()` is the only identity, the socket is a real unconnected `TcpSocket` for every backend. `EbpfDataplane`, `SimGatewayConnectDataplane` and the QC-2 connect double scaffold against it.
   - **DG-9 closed** — "Where a double may park a probe": the park point is an awaited operation the gate owns (`GatewayConnectDataplane::probe`, `GatewayRelay::probe`, identity `ensure_current`), not `GatewayClientMtls::authenticate` (R-65) and not the connector's `connect`; a held gate needs no `Selected` receipt; entries are identified by the staged gate, not by arrival order; the real `connect(2)` needs a loopback listener at the Service Frontend (test support); a gate parked at `connect(2)` holds one connect permit, so such entries stay below `max_inflight_connects`; the owner's behaviour on a superseded probe is unspecified and both behaviours are compatible. The held-probe capability, the hook point, the QC-2 doubles and the Sim held-probe rule (S-PIG-31, 32, 68) follow. No item.
   - **DG-10 closed** — "Admission access": crate-private `GatewayAdmission::new(budget)`, `GatewayAdmission::budget()` and `GatewayBuilder::start_with(deps, admission)`, each with a production caller in `start`; the driver goes through `UnboundGatewayBuilder::new`/`into_builder`/`start_with` and takes `GatewayRouteCommands` and `GatewayStatusReads` from `into_parts()`. Promotion binds port 443 through `GatewayListenerControl` on the address `GatewayConfig::new` takes; the driver supplies its own distinct `127/8` address (DG-12, R-6).
   - **DG-11 ruled (R-5, 2026-11-02)** — the connector reserves a cleanup-ledger slot before `open_upstream` and binds the returned intent to it afterwards; a full ledger refuses before any socket exists or any registration happens. S-PIG-37 asserts the refusal before `open_upstream` (the connect double's `open_upstream` call count is 0) and S-PIG-53's retained record exists at the pre-reservation.
   - **DG-12 ruled (R-6, 2026-11-02)** — each gateway test composition binds its own distinct `127/8` address on port 443 (`GatewayConfig::new` takes the address); there is no port seam and no whole-binary `host-kernel-shared` assignment for the public bind.
   - **Ruled or confirmed (2026-11-02):** the stutter and owner-autonomous handling (R-7: no `idle` step exists in the three conformance mains; QC-2 `stage`/`retire` stay explicit owner-autonomous steps), DG-7 and the DG-8/DG-10 item 13 entries (R-18), DG-12 (R-6: no whole-binary group for the public bind) and B-1 (R-4). S-PIG-123 stays (R-39): it closes C6b's public-handshake residual, which is observable only on a real handshake.
   - **B-1 ruled (R-4, 2026-11-02)** — `disable_after_drain` is a synchronous clear and `gateway_identity.qnt` models it, so QC-1 maps all seven conformance actions; `restart_reissue` and `disable_races_inflight_issue` expect `held` empty right after `disable` and carry the ignore reason `pending DELIVER quint-conformance-gateway_identity`.
   - **R-20 unblocks authoring:** S-PIG-38, 43, 52, 64 (handshake and wire evidence through `HostGatewayClientMtls`), S-PIG-34, 35, 42 (operator handlers behind the enabled composition) and the enabled-boot arms of S-PIG-02, 57, 58, 60, 61, 62, 74 and 100; `GatewayAdapters.client_mtls` and the enabled composition are scaffolded under R-20 and replace the documented `todo!` arm. A `GatewayUpstreamSeal` has no public constructor and is minted only by `UnboundGatewayBuilder::into_builder`, whose `::new` needs `overdrive_host::ca::PublicCertifiedKeyAeadCodec`; `overdrive-dataplane` has no `overdrive-host` edge (R-40 forbids a dev-dependency on it), so S-PIG-38, 43, 52 and 64 are placed in `overdrive-control-plane/tests/integration` (that crate has the `overdrive-host`, `overdrive-dataplane`, `overdrive-gateway` and `overdrive-sim` edges); lanes and oracles are unchanged, and P-34 would also allow placement beside the dataplane adapter.
   - **Gaps found while authoring, ruled 2026-11-03:** the placement of `DataplaneError::{BackendIdentityConflict, ServiceApplyRollback}` (R-35); the signatures of the head validator, parsers, framing decision, chunk-line parser, deadline composition and the pipe-floor law (R-32); when the response-head budget starts (R-37); the QC-3 `BackendId` reuse across an empty update (R-38: the model follows the production allocator). The zero-remaining case of the sentinel probe deadline (S-PIG-57 T3 reads it as `ProbeTimeout`) is an authoring detail of the T3 arm, not a contract question.
   - **DISTILL test-support shaping recorded (not a gap):** the held-probe capability of the QC-2 driver's connect and client doubles and the two `SimGatewayRelay` failure capabilities (Scaffolds).
   Notes only (not gaps): the pinned `Instant` type of the port deadlines is not named (consistent with the rest of the document); `PipeCapacity` is named by obligation 16(g) but is module-private typestate (the crafter's); the seeded relay runs have no catalogue `Invariant` variant (adding one is new scope for the user); U7 stale `unhealthy_is_no_backend` mentions and a Lima remark in the Obligations list (documentation drift); U8 `kvm-tests` named for `tests/conformance`, which has no such feature (DISTILL follows the crate's `native_metal_kvm_` selector). S-PIG-74 sub-cases (a), (c) and (d) are authored in the boot/shutdown file; (b) is the S-PIG-57 (I) stimulus.
8. **Real-guest evidence** (S-PIG-01, 56, 69, 70, 71, 60 graceful run, 105, 109, 113, 115 production arms, A-K3's serve-created connect) exists only on native metal; no recurring CI lane is added (P-6). The no-`SPLICE_F_MORE` configuration and the pipe invariant rest on one kernel build (7.0.0-38).

9. **SVID issuance failure occurrence: gaps.** The architect pinned the redb table names and key encodings (§ "Persisted format"), the owner projection during the boot identity wait, the availability bound of the status row and the `From<SvidIssueFailure>` conversion (2026-11-04); the stored-row cases are authored. Remaining: (a) the gateway owner's undecodable-prior-row half was not driven through the gateway `World` (its Sim store has no byte layer); it is now specified as S-PIG-129 over the production `LocalObservationStore` (R-54 (g); authored in the follow-up pass), beside the store-local point-read `None`. (b) The CLI never-issues row polls real time up to 60 s (two reconciler resync periods, the pinned bound); that ceiling is a recorded exposure, not an obligation.

Mandate evidence plan: every executable scenario carries a Contract Shape; state-delta assertions name public Route/status/receipt/cleanup surfaces and, for the relay, the port-exposed names (returned octets, typed variants, transcript, live leg and pipe counts, `tls_stat` deltas, descriptor counts), not private owner fields; PBT is confined to layers 1–2 (S-PIG-76–86, 89 PBT; the Sim exchange rows use PBT only over sizes and event offsets at layer 2, never at layers 3+); Tier 3 is example-based, sad paths named (Mandate 11); Rust domain nouns reuse the exact DESIGN types. Adapted step-reuse ratio is informational and has not been measured.

## Wave: DISTILL / [REF] Pre-DELIVER RED classification

Source: the per-batch verification records (metal runs); the expected reasons below are the batches' observations, and no single full-workspace run of the ignored set was made. Every ignored test was forced with `--run-ignored only` where the batch records that it was; the rest are named.

- **Expected-GREEN and active rows.** S-PIG-41 (unregistered socket) and S-PIG-59 (gateway disabled) are authored without `#[ignore]` and were recorded passing on metal (S-PIG-59 after the descriptor limit was raised and the guest-network kernel module loaded; the lane prerequisite otherwise fails every boot test of the control-plane binary at `GuestNetworkBoot(FdBudget)`). S-PIG-19 (argv rejection; the CLI default lane recorded 107 passed, 3 skipped), the four active S-PIG-65 tests and the golden-bytes decode fixtures are active. They are not MISSING_FUNCTIONALITY candidates; a RED here is a scaffold-wiring regression and blocks handoff.
- **Forced ignored tests, first failure at a production scaffold call (MISSING_FUNCTIONALITY, none a setup error):** B3a hook and equivalence tests and the four QC-3 tests (`EbpfDataplane::new_with_pin_dir` succeeds, they fail at `selected_backend_identity`); B3b (one forced: `HostGatewayRelay::new`); B4a's 20 ignored tests (`GatewaySvidLifecycle::reconcile`, `dispatch_issue`, `GatewayIdentitySlot::new`, `GatewayIdentityEpoch::new`, `SpiffeId::for_gateway`, `SimGatewayRelay::new`); B4b's ignored tests (three at `ServeArgs::gateway_config`; the S-PIG-20 pre-effect half fails with `ProbeRefused` because `gateway_config` is not yet the first startup operation; S-PIG-36 fails at `deploy_resource` and then hangs to the 120 s nextest kill, because a panic with a live server does not return, which is acceptable before DELIVER); B5 `overdrive-sim` (six forced: `Route::from_submit`, `SimGatewayRelay::new`); B5 conformance (one forced: it staged the rootfs, booted the production server and failed at the documented `todo!("SCAFFOLD: enabled gateway composition (boot steps G1-G4)")`; the aborted boot left the default veth pair and the `fwmark 0x1 lookup 100` rule, removed afterwards).
- **Not forced:** B1's ignored proptests (a forced proptest run as root on the metal host creates a root-owned `proptest-regressions` directory that fails every later sync); B2a's tests; B2b's 29 relay-file tests, the exchange-machine laws and QC-4 (expected: the first production call panics with SCAFFOLD text, `PublicCertifiedKeyAeadCodec::new` for the composition tests, `SimGatewayRelay::new` for S-PIG-99, `ExchangeMachine::new` for the laws and QC-4); S-PIG-114 (the honest-pass test panics first at the scaffold); the other conformance tests (each boots the full production composition and a VM, and a scaffold panic leaves node-global state behind). None of these is classified; a body that fails before its first production call is `FIXTURE_BROKEN` and blocks handoff.
- **G1 Tier-3 rows (S-PIG-104–115, 117, 118, 123):** RED because `HostGatewayRelay` is a scaffold; the first assertion is the missing arm/relay result at a real rustls peer, reached only after the loopback peers, certificates and counters are set up. A failing counter-baseline read (noisy host) or a failed peer handshake before the code under test is `FIXTURE_BROKEN` and blocks handoff. Hook rows (S-PIG-56, 57 T3, 63, 71, 72, and the R-54 rows 124, 125): RED because the separate selection program (R-1) does not exist and `open_upstream` is a scaffold. S-PIG-127 is RED because the baseline file names a program the object lacks (or, before the file exists, the gate does not cover the program).
- **Quint drivers:** compile green under `integration-tests`; ignored with `pending DELIVER quint-conformance-<module>` (QC-1 6 tests, QC-2 4, QC-3 4 until the selection program and the real-`EbpfDataplane` owner are GREEN, QC-4 8 until `ExchangeMachine` leaves scaffold); `cargo xtask conformance-lint` rc 0. QC-1 is complete: all five runs plus the `#[quint_run]` carry the single reason `pending DELIVER quint-conformance-gateway_identity` and fail first at `GatewayIdentitySlot::new`. Where an ignore is lifted, RED must be a State-comparison or owner-call mismatch at the scaffolded owner (MISSING_FUNCTIONALITY), never a bail on an unmapped action.
- **S-PIG-48 and S-PIG-66:** complete bodies under `#[ignore = "pending DELIVER lag-aware-intent-subscription"]`; once un-ignored, RED must be the missing `Lagged` event and missed count (MISSING_FUNCTIONALITY), because the ported `LocalIntentStore` reports every event as `Changed`.
- **SVID issuance failure occurrence (forced with `--run-ignored` on metal; last run):** the three ignored control-plane acceptance tests failed for the right reason, running through the production reconciler and failing at their first occurrence assertion (the executor does not yet record: the recorded count reads 0); the unrecorded-arm test fails at the distinct-error assertion, because the error is today identical to the recorded arm's and does not carry the injected record-write failure (it asserts the unrecorded arm's error chain contains the exported `ISSUE_FAULT_MARKER` and the recorded arm's does not). The gateway owner, executor, store-local and CLI integration tests fail at their first production scaffold call (SCAFFOLD); the render test fails at its first assertion (the occurrence line is not rendered). All MISSING_FUNCTIONALITY, none a setup error. The active tests pass: control-plane acceptance and `schema_evolution`.
- **Not classified because not authored:** every id in "Ids with no authored body" (S-PIG-11 and S-PIG-75). Workspace-level results recorded by the batches that are not attributable to this feature: 14 failures of the four-crate default-lane run reproduce on baseline HEAD (the `quint_cid_lease` and flows conformance tests, and the `workload_lifecycle` port-range flake); the `dst-lint` and clippy findings in `replacement_acceptance.rs`, `guest_network.rs`, `shared_network_recovery_acceptance.rs` and `vm_beacon_roundtrip.rs` pre-date the feature, and the `quint_selection_receipt.rs` veth fixture uses `overdrive-netlink`'s veth API (R-55).
