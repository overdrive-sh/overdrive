# Public-Ingress Gateway Minimum: Comprehensive Research

- **Status:** complete
- **Scope:** north-south ingress arriving from the public internet/outside an
  Overdrive node
- **Repository evidence:** Overdrive
  `ed52e9571137ca7175fdb1ff29c27c1473573fc9` (2026-09-12)
- **Cilium evidence:** Cilium
  `e99150f8d8f403eca51ed82138d4ae20a265c8f3` (2026-05-06)
- **Access date:** 2026-09-12

## Executive Summary

Overdrive should build the first gateway as a **small L7 HTTP reverse proxy
inside `overdrive serve`**, not as an L4 passthrough and not as a scheduled
workload. The minimum useful slice is one public TLS listener, HTTP/1.1,
exact-host plus segment-aware path-prefix matching, one Service listener as the
route target, and selection only from the ServiceLifecycle-authored healthy
`service_backends` set. It must be driven by a real `overdrive serve` and real
`overdrive deploy <SPEC>` commands and must return the backend's status, headers,
and streaming body to an external client.

That recommendation is deliberately narrower than whitepaper §11. Defer
HTTP/2, gRPC, WebSocket, middleware, ACME, request replay, region hints,
scale-to-zero, TLS passthrough, and multi-node ingress. They do not establish the
first public request/response loop. HTTP/2 is valuable later, but RFC 9113 adds
multiplexing, flow control, connection-wide resource limits, and GOAWAY drain
semantics that HTTP/1.1 does not require for the first proof
([RFC 9113 §§1–3](https://www.rfc-editor.org/rfc/rfc9113.html)).

**TLS should not be deferred.** A plaintext-only public listener proves parsing
and routing but does not provide a production public-ingress boundary. TLS 1.3
provides server authentication, confidentiality, and integrity against an
attacker controlling the network
([RFC 8446 §1](https://www.rfc-editor.org/rfc/rfc8446.html)). The smallest safe
issuance lane is an **operator-supplied publicly trusted certificate**, not ACME.
ACME is a separate durable lifecycle: domain-control challenge, issuance,
installation, renewal, and potentially revocation
([RFC 8555 §§1, 7–8](https://www.rfc-editor.org/rfc/rfc8555.html)). Issue
[#57](https://github.com/overdrive-sh/overdrive/issues/57) and whitepaper
§11 already make ACME a separate gateway primitive.

Two unresolved contracts block implementation from starting today:

1. **Route submission/public-certificate intent shape.** Accepted aspiration and
   issue [#60](https://github.com/overdrive-sh/overdrive/issues/60) require a
   top-level `Route` resource, but `overdrive deploy` currently accepts only
   `Job`, `Service`, or `Schedule` workload specs
   (`crates/overdrive-cli/src/cli.rs:39-62`,
   `crates/overdrive-core/src/api/submit.rs:23-49`). DESIGN must pin how a Route
   and its certificate reference reach the IntentStore through the existing
   `overdrive deploy <SPEC>` verb. This research does not invent a signature or
   TOML schema.
2. **Gateway-to-backend identity.** The existing transparent-mTLS port presents
   the SVID selected by `InterceptedConnection.alloc: AllocationId`; the
   `IdentityMgr` holds only per-allocation SVIDs
   (`crates/overdrive-core/src/traits/mtls_enforcement.rs:148-203`,
   `crates/overdrive-control-plane/src/identity_mgr.rs:44-64`). A gateway is a
   node subsystem, not an allocation. DESIGN must establish the gateway's own
   SPIFFE identity, issuance/hold/rotation owner, and exact outbound transport
   port. Borrowing a backend or arbitrary workload SVID, sending plaintext to
   the mesh address, or adding an ad-hoc public method would violate accepted
   identity and API discipline. The selected backend's published
   `Backend.alloc: SpiffeId` must also become the expected peer so the public
   gateway does not accept an arbitrary valid workload certificate; open issue
   [#242](https://github.com/overdrive-sh/overdrive/issues/242) tracks that
   existing authn-to-intended-peer gap
   (`crates/overdrive-core/src/traits/dataplane.rs:153-160`).

The first implementation should begin only after those two shapes are approved.
Everything else required for the slice has a strong existing foundation:
Service listener intent, platform VIP allocation, authoritative healthy backend
publication, ObservationStore List/Watch, `hyper`, `rustls`, graceful
`ServerHandle` ownership, and a real `serve` + `deploy` composition path.

## Research Scope and Method

### Question

What is the minimum first production-drivable vertical slice that makes
Overdrive accept a public north-south HTTP request and proxy it to a deployed
Service, while choosing listener, intent, discovery, TLS, lifecycle, and test
primitives that remain correct as the gateway grows?

“Gateway” in this document never means:

- the per-netns IP gateway used by workload networking;
- the node-local DNS responder;
- the operator/control-plane HTTPS listener on `127.0.0.1:7001`;
- an east-west Service VIP alone; or
- the transparent-mTLS per-allocation intercept listener.

### Evidence method

The research used four independent lanes:

1. accepted Overdrive ADRs/brief plus whitepaper aspirations;
2. current production code at the pinned commit, following the real
   `overdrive serve` and `overdrive deploy <SPEC>` entries;
3. every relevant GitHub issue read with `gh issue view <N> --comments`, so
   comment-only corrections are included; and
4. local Cilium Gateway API/Ingress/Envoy source at a pinned commit, supported by
   IETF RFCs and the official Gateway API repository on the trusted `github.com`
   domain.

External sources obey `.nwave/trusted-source-domains.yaml`. RFC Editor is a
configured high-reputation official source; GitHub is a configured
medium-high-reputation source and is used here only for official project
repositories. No excluded or medium-trust source is used. Local repository
source is direct primary evidence, not a web reputation proxy.

### Evidence labels

- **Accepted design** means an accepted ADR or the architecture brief.
- **Implemented fact** means reachable current production code at the pinned
  commit.
- **Aspiration** means whitepaper prose or an open issue not yet implemented.
- **Recommendation** is this research's synthesis; it is not an approved public
  API or ADR.

## Overdrive: Accepted Architecture and Implemented Production Facts

### 1. The whitepaper's gateway is an aspiration, not a current subsystem

Whitepaper §11 says the gateway is a node-agent subsystem built with `hyper` and
`rustls`, enabled by node configuration, reading route/identity/telemetry state
in process, terminating public TLS, and proxying into the mTLS mesh
(`docs/whitepaper.md:1280-1349`, `:1350-1421`). It also sketches HTTP/1.1,
HTTP/2, gRPC, gRPC-Web, WebSocket, middleware, ACME, request replay, regional
hints, and auto-wake (`:1342-1464`). These are broad product goals.

At the current commit:

- there is no `overdrive-gateway` crate and no gateway production module;
- no parsed `node.gateway`/`gateway.enabled` configuration exists;
- `ServerConfig.bind` is the operator/control-plane HTTPS address, defaulting to
  `127.0.0.1:7001` (`crates/overdrive-control-plane/src/lib.rs:728-752`);
- the only axum routes are `/v1/workloads`, workload lifecycle endpoints,
  `/v1/allocs`, `/v1/nodes`, and `/v1/cluster/info`
  (`crates/overdrive-control-plane/src/lib.rs:3128-3140`);
- the only bound HTTP server is that control-plane router, using the ephemeral
  operator CA (`:2143-2159`, `:3142-3174`); and
- `instant-acme = "0.8"` is declared at workspace scope but has no Rust call
  site and no `Cargo.lock` package entry (`Cargo.toml:99-107`;
  repository-wide `instant_acme` and lockfile package searches are empty).

Therefore, “the gateway exists” is false as an implementation claim. The
reusable HTTP/TLS dependencies and server lifecycle exist, but the public
listener, route model, routing snapshot, public certificate owner, and proxy
handler do not.

### 2. `overdrive serve` is the correct owner and already has a bounded drain

The real operator path is:

```
overdrive serve
  -> overdrive_cli::commands::serve::run
  -> run_inner
  -> overdrive_control_plane::run_server
  -> run_server_with_obs_and_drivers
  -> bind control-plane TcpListener
  -> axum_server::from_tcp_rustls(...)
```

Evidence: `crates/overdrive-cli/src/commands/serve.rs:114-135,233-311` and
`crates/overdrive-control-plane/src/lib.rs:1608-1673,2088-2159,3142-3190`.
This composition root already owns observation, intent, reconcilers, dataplane,
identity, DNS responder, workflow drain, and shutdown tasks. That matches the
whitepaper's bootstrap argument: a gateway is infrastructure and cannot depend
on being scheduled as a workload (`docs/whitepaper.md:1282-1308`).

`ServerHandle::shutdown` stops convergence and action drains, asks axum to stop
accepting new connections and drain in-flight work under a caller-supplied
deadline, then drains observation/DNS/mTLS owners
(`crates/overdrive-control-plane/src/lib.rs:1392-1487`). The gateway should join
this same ownership boundary, with its own listener/task handles and cooperative
cancellation. It must not be a detached task whose bind or runtime failure is
invisible to `serve`.

### 3. `overdrive deploy <SPEC>` already carries usable Service listener intent

The CLI's public workload verb is `overdrive deploy <SPEC>`; it selects
streaming only for an attached TTY and otherwise uses a one-shot JSON
acknowledgement (`crates/overdrive-cli/src/main.rs:52-138`). A Service TOML is
parsed and projected to `ServiceSpecInput`, validated on the client, posted to
`POST /v1/workloads`, revalidated on the server, archived, and committed to the
IntentStore (`crates/overdrive-cli/src/commands/deploy.rs:180-341,627-678`;
`crates/overdrive-control-plane/src/handlers.rs:261-346`).

Each Service `[[listener]]` is exactly `(port, protocol)`; port is non-zero,
protocol is the existing TCP/UDP type, and the operator cannot provide the VIP
(`crates/overdrive-core/src/aggregate/workload_spec.rs:562-620`;
`crates/overdrive-core/src/api/submit.rs:51-124`). This is **backend listener
intent**, not public gateway listener intent. It says where a Service accepts
traffic, not which public hostname, path, TLS certificate, or gateway port owns
external traffic. Conflating the two would make the same field serve two
different owners and prevent one Service listener from being referenced by
multiple public routes.

### 4. Backend discovery and health are implemented, but the read model is not gateway-ready

Current ServiceLifecycle is the sole authoritative `ServiceBackendRow`
publisher. It combines:

- current Service listeners and allocator-issued VIP;
- `AllocStatusRow` membership filtered to `Running`;
- canonical backend identity/address;
- terminal veto and readiness-derived `healthy`; and
- a full-row comparison before publishing

(`docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md:71-120`;
`crates/overdrive-reconcilers/src/service_lifecycle.rs:644-675,815-864,1101-1180`).
The row contains `service_id`, IPv4 VIP, backend vector, and LWW timestamp
(`crates/overdrive-core/src/traits/observation_store.rs:1754-1775`). The
ObservationStore exposes both a keyed read and a deterministic all-row snapshot,
the latter expressly serving List/Watch consumers
(`:2339-2405`).

This is the correct **dynamic backend observation source** for a gateway. It
must not inspect process state, rerun readiness policy, read BPF maps as its
source of truth, or treat `Running` as healthy. A healthy backend row is a
published eligibility decision; downstream application is asynchronous, not a
new allocation-lifecycle gate (ADR-0101 `:255-278,489-505`).

There is, however, a route-to-backend join gap:

- `ServiceBackendRow` carries no `WorkloadId`, public route name, or listener
  protocol/port.
- `ListenerFacts::fact_for` only maps `ServiceId -> ListenerRow`
  (`crates/overdrive-core/src/traits/listener_facts.rs:25-64`).
- The concrete listener projection internally has a secondary
  `WorkloadId -> Vec<ServiceId>` cleanup index, but no such public read port is
  exposed (`crates/overdrive-control-plane/src/listener_facts.rs:14-37,59-76`).

DESIGN must choose the smallest read model that lets a Route target one exact
Service listener without exposing a mutable store or leaking a concrete
control-plane type. Likely inputs already exist, but the exact API shape is
unapproved and must not be invented in delivery.

### 5. Service VIPs are not the first public gateway address

The current allocator assigns private IPv4 Service VIPs, persists assignments in
IntentStore, and is rebuilt on boot (ADR-0049; current code is consumed by
ServiceLifecycle at `service_lifecycle.rs:826-863`). The single-node XDP path is
attached at `serve` boot and is fed by the ServiceMapHydrator
(`crates/overdrive-control-plane/src/lib.rs:2240-2376`). These VIPs are
east-west/service dataplane identities, not automatically advertised public
addresses.

Issue [#61](https://github.com/overdrive-sh/overdrive/issues/61) explicitly
corrects the older whitepaper framing: the future IPv6 VIP/XDP path is a
separate connect-time east-west path, while dial-by-name currently uses a stable
frontend address and terminating proxy. A public gateway should therefore bind
an explicitly configured node address/port and proxy to healthy backend
observations; it should not require #61, public BGP/VIP advertisement, or a new
XDP ingress path to prove the first north-south loop.

### 6. Three certificate domains must remain separate

1. **Operator/control-plane HTTPS CA:** `tls_bootstrap::mint_ephemeral_ca()`
   mints a fresh self-signed CA and server/client leaves; the control-plane
   listener presents this material and writes the CLI trust triple
   (`crates/overdrive-control-plane/src/tls_bootstrap.rs:1-15,192-237,500-545`;
   `crates/overdrive-control-plane/src/lib.rs:2143-2159,3150-3173`). It is not
   publicly trusted and must not be reused as the public gateway certificate.
2. **Workload-identity CA:** `serve` boots and adopts a persistent KEK-sealed
   root and node intermediate, then seeds `IdentityMgr` with its trust bundle
   (`crates/overdrive-control-plane/src/lib.rs:2501-2558`;
   `crates/overdrive-control-plane/src/ca_boot.rs:133-185,188-257`). It issues
   workload SVIDs and authenticates east-west mesh peers. It is not a public Web
   PKI issuer.
3. **Public gateway certificate:** absent from production code. Whitepaper §11
   names operator upload or ACME and issue #57 tracks ACME. This certificate
   authenticates a DNS origin to arbitrary internet clients; it must not be
   minted under either internal CA.

The current crypto provider is also a documented design/code drift. ADR-0039
selects `aws-lc-rs`, but `Cargo.toml:91-92` and the `serve` entry at
`lib.rs:2143-2149` use `ring`. Open issue
[#204](https://github.com/overdrive-sh/overdrive/issues/204) records that the ADR
was never implemented. Gateway DESIGN must not claim FIPS or an aws-lc-rs
provider until #204 lands.

## GitHub Issue Evidence

All issues below were read with `--comments`. Empty comment arrays are recorded
because absence of later correction matters when a body is only a placeholder.

| Issue | State / comments | Evidence and current interpretation |
|---|---|---|
| [#54](https://github.com/overdrive-sh/overdrive/issues/54) gateway subsystem | Open; no comments | Only a summary, whitepaper pointer, dependencies, and TODO acceptance. It is an umbrella aspiration, not executable scope. |
| [#55](https://github.com/overdrive-sh/overdrive/issues/55) protocols | Open; no comments | Lists the eventual full protocol set. It does not require all protocols in the first slice. |
| [#56](https://github.com/overdrive-sh/overdrive/issues/56) middleware | Open; no comments | Middleware is explicitly downstream of gateway/protocol support. Safe exclusion from MVP. |
| [#57](https://github.com/overdrive-sh/overdrive/issues/57) ACME | Open; no comments | Separate public-certificate primitive, dependent on gateway and identity. Its all-challenge body is broader than a first request loop. |
| [#58](https://github.com/overdrive-sh/overdrive/issues/58) request replay | Open; no comments | Separate application-driven routing primitive with buffering and loop control. Exclude. |
| [#60](https://github.com/overdrive-sh/overdrive/issues/60) Route resource | Open; no comments | Requires top-level Route intent, not route fields embedded in workload specs. Required for the MVP, but its acceptance and API shape are still TODO. |
| [#106](https://github.com/overdrive-sh/overdrive/issues/106) regional gateway | Open; no comments | Multi-region local-read behavior is later integration, not a single-node prerequisite. |
| [#267](https://github.com/overdrive-sh/overdrive/issues/267) component enablement | Open; no comments | Corrects implicit capability inference and proposes explicit `gateway` enablement. This is a real dependency for node-role/config ownership, but its exact config shape is not approved here. |
| [#98](https://github.com/overdrive-sh/overdrive/issues/98) persistent-VM auto-route | Open; no comments | Downstream integration of `expose = true`; not needed for an explicit Route MVP. |
| [#4](https://github.com/overdrive-sh/overdrive/issues/4) per-workload URLs | Open; no comments | Correct intent/observation split, but stale claim that TLS/ACME are “solved” and stale `lers` library name conflict with current code/#57/whitepaper. Do not use it as implementation evidence. |
| [#150](https://github.com/overdrive-sh/overdrive/issues/150) node-listener policy | Open; no comments | Correctly identifies 80/443 as deliberately public platform listeners and handshake DoS as a residual concern. General host firewall is not an MVP dependency; connection/resource limits are. |
| [#164](https://github.com/overdrive-sh/overdrive/issues/164) Service listeners | Closed; three comments | Comments changed the name to `[[listener]]`, changed `proto` to `protocol`, temporarily made VIP optional, and record implementation. Later ADR-0049/current code remove operator VIP entirely. The original body is stale. |
| [#167](https://github.com/overdrive-sh/overdrive/issues/167) VIP allocator | Closed; one comment | Primitive shipped, but body is stale: it still says optional/pinned VIP and release-on-stop. ADR-0049 amendments/current code govern platform-only VIP and later lifecycle. |
| [#170](https://github.com/overdrive-sh/overdrive/issues/170) health checks | Closed; one comment | Provides the health producer. Its comment's host/loopback backend address is stale after per-workload networking; current ADR-0101/code publish materialized workload address when present. |
| [#174](https://github.com/overdrive-sh/overdrive/issues/174) backend discovery | Closed; no comments | Historical gap closure. Its proposed BackendDiscoveryBridge is superseded by ADR-0101's single ServiceLifecycle publisher. |
| [#175](https://github.com/overdrive-sh/overdrive/issues/175) production eBPF | Closed; no comments | Historical Noop-to-eBPF boot transition; current `run_server` proves production wiring. |
| [#24](https://github.com/overdrive-sh/overdrive/issues/24) XDP L4 LB | Closed; no comments | L4 service dataplane exists, but it does not parse HTTP host/path or terminate public TLS. Useful backend substrate, not the gateway itself. |
| [#241](https://github.com/overdrive-sh/overdrive/issues/241) canonical workload address | Closed; no comments | Productionizes backend address and inbound mesh termination through real `serve` + `deploy`. It is the current backend-network constraint the gateway must respect. |
| [#242](https://github.com/overdrive-sh/overdrive/issues/242) intended-peer SVID pinning | Open; no comments | Current transparent mTLS authenticates any chain-to-bundle workload, not the selected backend identity. A public gateway should not expose that endpoint-substitution gap; the selected row already carries the expected SPIFFE ID. |
| [#243](https://github.com/overdrive-sh/overdrive/issues/243) name responder | Closed; no comments | Body says “headless backend address,” but ADR-0072 REV-2/current code changed to a stable frontend address. It is east-west DNS, not public gateway ingress. |
| [#35](https://github.com/overdrive-sh/overdrive/issues/35) IdentityMgr | Closed; no comments | Body still says near-expiry rotation will be a workflow; issue #40's later comment supersedes that. Current manager remains allocation-keyed. |
| [#40](https://github.com/overdrive-sh/overdrive/issues/40) SVID rotation | Closed; two comments | The 2026-06-09 comment explicitly separates one-step internal SVID reissue (reconciler action) from multi-step external ACME rotation. This correction is load-bearing for gateway scope. |
| [#204](https://github.com/overdrive-sh/overdrive/issues/204) crypto provider drift | Open; no comments | Confirms ADR-0039 is accepted but unimplemented; current provider is `ring`. |
| [#215](https://github.com/overdrive-sh/overdrive/issues/215) persistent workload CA | Closed; one comment | Comment distinguishes the persistent workload root from the deliberately ephemeral operator CA; current code has since landed the boot-side path. |

## Cilium Gateway/Ingress Precedent

### What Cilium actually does

Cilium was inspected locally at
`/Users/marcus/git/cilium/cilium`, commit
`e99150f8d8f403eca51ed82138d4ae20a265c8f3`.

Its Gateway reconciler watches Gateway/GatewayClass, backend Services,
HTTPRoute/GRPCRoute/TLSRoute, Secrets, namespaces, grants, backend TLS policy,
and the resources it owns
(`/Users/marcus/git/cilium/cilium/operator/pkg/gateway-api/gateway.go:60-167`).
On each Gateway reconcile it:

1. loads the selected gateway and dependent routes/services/secrets/grants;
2. validates and updates route status;
3. ingests accepted resources into a protocol-neutral listener/route/backend
   model;
4. translates that model to Envoy listener/route/cluster resources plus
   Kubernetes exposure objects; and
5. publishes Gateway status

(`/Users/marcus/git/cilium/cilium/operator/pkg/gateway-api/gateway_reconcile.go:43-275`).

Cilium's older Kubernetes Ingress controller reaches the same normalized
model/translator boundary by a different API adapter. It chooses dedicated or
shared load-balancer mode, cleans resources belonging to the other mode,
translates Ingress intent into the common HTTP/TLS-passthrough model, creates or
updates Envoy/exposure resources, and then updates load-balancer status
(`/Users/marcus/git/cilium/cilium/operator/pkg/ingress/ingress_reconcile.go:39-146,160-293`).
The transferable lesson is the shared normalized listener/route/backend model,
not support for both Kubernetes APIs in Overdrive.

The shared ingestion model separates listener ownership from routes and
backends. An HTTP listener carries port, hostname, TLS secret references, and
routes; a TLS-passthrough listener is a different type
(`/Users/marcus/git/cilium/cilium/operator/pkg/model/model.go:16-82,114-168`).
Gateway API ingestion intersects listener attachment, listener port, protocol,
and hostname before extracting route rules
(`/Users/marcus/git/cilium/cilium/operator/pkg/model/ingestion/gateway.go:76-141,173-244`).

The translator emits separate exposure and data-plane resources. Gateway
listeners become an Envoy config plus a LoadBalancer Service and EndpointSlice
(`/Users/marcus/git/cilium/cilium/operator/pkg/model/translation/gateway-api/translator.go:46-107`).
HTTP upstreams are EDS clusters with round-robin LB and outlier detection; by
default the upstream protocol follows the downstream, with explicit HTTP/2 for
gRPC or h2c
(`/Users/marcus/git/cilium/cilium/operator/pkg/model/translation/envoy_cluster.go:24-59,62-127`).

TLS termination uses SNI-specific filter chains. The listener groups hostnames
by referenced secret, then emits an SDS secret reference on the transport
socket (`/Users/marcus/git/cilium/cilium/operator/pkg/model/translation/envoy_listener.go:320-355,431-471`).
Secrets referenced by Gateways are watched and copied to a dedicated secret
namespace; the agent normally consumes them by SDS without direct permission to
read arbitrary source Secrets
(`/Users/marcus/git/cilium/cilium/operator/pkg/gateway-api/secretsync.go:23-68`;
`/Users/marcus/git/cilium/cilium/pkg/crypto/certificatemanager/certificate_manager.go:151-193`).

HTTP route order is made deterministic and standards-shaped because Envoy
matches sequentially: exact path, longer regex/prefix, method, number of
headers, then query parameters
(`/Users/marcus/git/cilium/cilium/operator/pkg/model/translation/envoy_virtual_host.go:34-105,127-166`).
The official Gateway API type gives the same core order and defines 404 when no
attached rule matches
([official `HTTPRoute` source](https://github.com/kubernetes-sigs/gateway-api/blob/main/apis/v1/httproute_types.go)).

Cilium's xDS owner waits for clusters before listeners when both are added,
waits for listener ACK/NACK under a bounded context, and reverts the resource
transaction on failure
(`/Users/marcus/git/cilium/cilium/pkg/envoy/xds_server.go:1989-2122`). Its xDS
server recognizes an Envoy restart from the last applied version and continues
versioned delivery; NACKs are observable
(`/Users/marcus/git/cilium/cilium/pkg/envoy/xds/server.go:316-416`). Standalone
Envoy is supervised and restarted on crash; explicit stop terminates xDS and
uses the admin quit path with kill only as fallback
(`/Users/marcus/git/cilium/cilium/pkg/envoy/standalone_envoy.go:200-279,350-359`).

### Transferable primitives versus Kubernetes-specific machinery

| Transfer to Overdrive | Do not copy |
|---|---|
| Separate gateway-listener intent, route intent, and dynamic backend observation. | GatewayClass, CRDs, namespaces, ReferenceGrant, controller-runtime indexes, and Kubernetes owner references. |
| Compile all accepted inputs into one immutable normalized routing snapshot. | `CiliumEnvoyConfig`, protobuf xDS, Envoy process supervision, and SDS; Overdrive explicitly chose in-process Rust/hyper. |
| Make route precedence total and deterministic before serving it. | Kubernetes creation-timestamp/namespace tie-breaks if Overdrive's Route ID/version provides a simpler canonical order. DESIGN must still pin the total order. |
| Select TLS identity by SNI and reject an HTTP authority that does not belong to that listener. | Copying TLS Secrets into a special Kubernetes namespace. Overdrive needs an equivalent least-privilege key owner, but not the Kubernetes mechanism. |
| Keep intent validity (`Accepted`/`ResolvedRefs`) separate from data-plane application (`Programmed`) and traffic health. | Kubernetes LoadBalancer Service, NodePort, EndpointSlice, externalTrafficPolicy, loadBalancerClass, and LB-IPAM integration for the first node-bound listener. |
| Watch every dependency, relist after loss/restart, and atomically replace the route/backend snapshot. | Envoy cluster/outlier-detection configuration as the MVP health source; Overdrive already has ServiceLifecycle readiness eligibility. |
| Bind listeners only after prerequisites are resolved, and fail/revert visibly when application fails. | Recreating Envoy's ACK protocol inside one process. A successfully bound socket plus installed snapshot can be acknowledged directly by the owner. |

The official Gateway API model reinforces these boundaries: listeners own
hostname, port, protocol, termination, TLS settings, and attachable routes
([official Gateway API source](https://github.com/kubernetes-sigs/gateway-api/blob/main/config/crd/standard/gateway.networking.k8s.io_gateways.yaml));
HTTPRoute is appropriate only after TLS is terminated and HTTP is inspectable,
whereas TLSRoute routes opaque encrypted streams by SNI
([official TLS routing guide](https://github.com/kubernetes-sigs/gateway-api/blob/main/site/content/en/guides/user-guides/tls-routing.md)).

## Standards and Authoritative External Sources

### HTTP intermediary obligations

HTTP/1.1 requires a single valid Host field and a 400 response for missing,
duplicated, or invalid Host. It also defines strict message framing because
parsing differences enable request smuggling
([RFC 9112 §§3.2, 6.3, 11.2](https://www.rfc-editor.org/rfc/rfc9112.html)).
The gateway must use one parser interpretation end-to-end, reject ambiguous
`Content-Length`/`Transfer-Encoding`, and strip hop-by-hop fields before
forwarding. It must not implement a lenient second parser around `hyper`.

RFC 9110 requires HTTP gateways to add `Via`, preserve the target path/query
except where a configured transform explicitly applies, and reject misdirected
requests when the connection's authority does not authorize the target
([RFC 9110 §§4.3.3, 7.4, 7.6.3](https://www.rfc-editor.org/rfc/rfc9110.html)).
The first slice has no rewrite feature, so path and query pass unchanged. The
public Host/authority remains the route-selection input; whether the upstream
Host is preserved or replaced must be pinned in DESIGN rather than happen as an
incidental client-library default.

### TLS and certificate selection

TLS 1.3's authenticated secure channel is the right public boundary, but its
server certificate proves authority only for the DNS origin represented by the
certificate. A private Overdrive CA is therefore insufficient for arbitrary
browsers/SDKs ([RFC 8446](https://www.rfc-editor.org/rfc/rfc8446.html);
[RFC 9110 §4.3.3](https://www.rfc-editor.org/rfc/rfc9110.html)).

`rustls::ServerConfig` supports a dynamic certificate resolver and explicit
ALPN; `ResolvesServerCertUsingSni` is the standard SNI-indexed helper
([rustls 0.23 server docs](https://docs.rs/rustls/latest/rustls/server/),
[ServerConfig docs](https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html)).
This makes “immutable map of hostname -> certified key, atomically replaced” a
safe primitive for both operator-provided certificates now and ACME-produced
certificates later. It does not decide the persistence/upload API.

### Status is part of the contract

Gateway API's standard distinction is useful even without Kubernetes:

- **Accepted:** syntax/semantics are valid enough to produce a data-plane
  effect;
- **ResolvedRefs:** referenced backend/certificate objects exist and are
  permitted; and
- **Programmed:** the accepted config has been sent/applied to the data plane,
  with `observedGeneration` identifying which intent generation the status
  describes

([official troubleshooting source in the Gateway API repository](https://github.com/kubernetes-sigs/gateway-api/blob/main/site/content/en/docs/concepts/troubleshooting.md),
[Gateway API implementation repository](https://github.com/kubernetes-sigs/gateway-api)).
Cilium concretely stores Gateway and per-listener conditions with generation
and transition time
(`/Users/marcus/git/cilium/cilium/operator/pkg/gateway-api/gateway_status.go:13-157`).

Overdrive need not copy the names as a public API before DESIGN approves them,
but it does need distinct facts. “Route committed” must not mean “certificate
loaded, listener bound, backend healthy, and request succeeded.” Those are
different owners and times.

## Gateway Design Comparisons

### Decision table

| Axis | Option A | Option B | Recommendation for first slice | Why |
|---|---|---|---|---|
| Traffic layer | L4 TCP/TLS passthrough | L7 HTTP reverse proxy | **L7** | Host/path routing and TLS termination are the gateway's minimum differentiated value. L4 duplicates existing TCP/VIP primitives and cannot inspect HTTP. |
| Public encryption | Plaintext HTTP first | TLS 1.3 now | **TLS now** | Plaintext is useful only as a local test lane. Public traffic otherwise lacks authentication/confidentiality/integrity. |
| Certificate acquisition | Operator supplies certified key | Embedded ACME now | **Operator-supplied first** | Proves public trust without building challenge/renewal workflow, DNS provider ports, and account-key lifecycle. ACME remains additive behind the same certified-key snapshot. |
| HTTP version | HTTP/1.1 | HTTP/2 | **HTTP/1.1** | Widest simple interoperability and a smaller lifecycle/security surface. H2 later adds ALPN `h2`, multiplexing limits, flow control, and GOAWAY drain. |
| Listener placement | Scheduled workload | `serve`-owned subsystem | **`serve`-owned** | Avoids bootstrap dependency and shares existing state/task/shutdown ownership. |
| Route placement | Fields embedded in Service spec | Top-level Route intent | **Top-level Route** | Matches whitepaper/#60 and keeps backend listener intent separate from public exposure intent. |
| Matching | Catch-all only | Exact host + path prefix | **Exact host + segment-aware prefix** | One useful multi-service gateway without regex/header/query complexity. A total precedence rule is required. |
| Backend source | Direct process/driver probing | `service_backends` observation | **Observation row** | ServiceLifecycle already owns readiness/terminal eligibility; gateway must consume, not recreate it. |
| Balancing | First healthy forever | Deterministic round-robin | **Round-robin over current healthy set** | Avoids hotspot and preserves simple, observable behavior. Existing row weight stays 1; weighted routing can follow later. |
| Health | Active gateway probes | Consume `Backend.healthy` | **Consume health** | Prevents competing health owners and inconsistent readiness. Connection failure remains per-request failure, not a new health writer. |
| Update form | Mutate shared tables piecemeal | Compile and atomic-swap immutable snapshot | **Snapshot swap** | A request observes one coherent generation; route/backend/cert readers do not see torn updates. |
| Empty backend | Keep stale backend | Explicit unavailable response | **No stale fallback; return 503** | Honest current availability. A route may remain accepted/programmed while its dynamic backend set is empty. |

### L4 ingress versus L7 reverse proxy

An L4 listener can preserve arbitrary TCP and optionally route TLS by SNI, but
it cannot implement whitepaper host/path Route intent after TLS termination. It
also leaves public certificate termination to the backend, which contradicts
whitepaper §11's public-boundary model (`docs/whitepaper.md:1408-1421`).

The L7 proxy has more parser and resource-limit responsibility, but it closes an
actual operator journey: public DNS origin -> TLS -> HTTP route -> deployed
Service -> HTTP response. Cilium makes the same type distinction: HTTP/HTTPS
listeners terminate and proxy HTTP; TLS-passthrough listeners route opaque
streams (`operator/pkg/model/model.go:45-82,114-142` at the pinned Cilium
commit). Start with L7 and add an explicitly separate TLSRoute/L4 capability
later if demanded.

### TLS now versus plaintext first

Plaintext-first appears smaller only if certificate intent and key ownership are
ignored. It produces a non-production path that must be replaced before public
use and creates pressure to normalize insecure operation. ADR-0010 rejected the
same migration shape for the operator interface
(`docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md:144-160`).

TLS with an operator-provided key is the narrow middle:

- keeps one `rustls` termination path from day one;
- proves SNI/certificate/authority isolation;
- requires no ACME workflow or DNS provider; and
- lets ACME later publish the same certified-key input to the same runtime
  resolver.

The private key must never be logged, rendered by `describe`, copied into an
observation row, or handed to request handlers. At-rest encryption and upload
authority remain DESIGN decisions; raw PEM in ordinary Route fields is not a
safe default.

### HTTP/1.1 versus HTTP/2 first

HTTP/1.1 supplies the required request semantics, streaming bodies, persistent
connections, and universal clients. The proxy must obey strict framing rules
and bounded parsing. HTTP/2 offers fewer TCP connections, multiplexed streams,
and compressed fields, but adds stream concurrency, flow-control, compressed
field-block memory accounting, connection errors, and GOAWAY lifecycle
([RFC 9113 §§1–5, 6.8, 10.5](https://www.rfc-editor.org/rfc/rfc9113.html)).

Therefore the first public listener should advertise only `http/1.1`. Adding
`h2` is an additive protocol capability only after per-connection and per-stream
limits plus GOAWAY drain tests exist. The existing control-plane listener's ALPN
`h2,http/1.1` is reusable experience, not proof the proxy semantics are done
(`crates/overdrive-control-plane/src/tls_bootstrap.rs:500-545`).

### Route matching and precedence

The minimum matching model is:

1. exact, canonical DNS hostname;
2. path match is exact or segment-aware prefix;
3. exact path wins over prefix;
4. longer prefix wins over shorter prefix; and
5. any remaining tie has a deterministic Route identity/generation ordering
   pinned by DESIGN, or admission rejects it as ambiguous.

Do not use naive byte prefix: `/api` must not unintentionally match `/apix`.
Do not add regex, method, header, query, rewrite, mirror, or weighted-split
matches in the MVP. Cilium's explicit sort exists because sequential proxy
matching makes order observable (`envoy_virtual_host.go:34-105`), and the
official Gateway API source specifies a total core order
([HTTPRoute type](https://github.com/kubernetes-sigs/gateway-api/blob/main/apis/v1/httproute_types.go)).

### Backend selection and health

On each request, the route snapshot identifies exactly one Service listener;
the backend snapshot supplies only the currently published backends with
`healthy == true`. A round-robin cursor selects one. The cursor is scratch/live
runtime state, not intent or observation. A connection failure may try at most a
small bounded number of *different* candidates if DESIGN explicitly approves
retry semantics; request-body replay is unsafe without method/body rules, so the
first contract should prefer **one selection, one attempt**.

If the backend set is absent or empty, return `503 Service Unavailable`; if no
route matches, return `404`; if the Host is invalid/misdirected, return `400` or
`421` as DESIGN pins. Do not serve a last-known backend after
ServiceLifecycle withdraws eligibility. Existing in-flight connections may
complete; eligibility governs new selections unless DESIGN explicitly adds
connection revocation.

### Listener ownership, restart, and drain

The gateway listener is node-level state owned by the `serve` process. A bind,
certificate load, or initial intent/observation List failure must produce a
typed startup refusal before the gateway claims `Programmed`. A route-specific
bad reference should reject that route/listener without taking down unrelated
valid routes, once partial validity is intentionally designed; the narrow MVP
may fail the one gateway listener closed because it owns only one certified
origin.

On graceful shutdown:

1. stop accepting public connections;
2. stop admitting new HTTP requests on keep-alive connections;
3. let in-flight request/response streams finish within the existing drain
   budget or a separately approved gateway budget;
4. cancel/join route and backend watchers; and
5. release the public listener and key snapshot.

On crash/restart, intent and observation are re-Listed before the listener is
trusted, then watches begin with lag/relist recovery. No route snapshot should
be persisted as a second source of truth. Cilium's resource-version replay and
ACK ordering are the external precedent; Overdrive can make the same invariant
smaller in process.

## Minimum First Vertical Slice (MVP)

### Operator-visible outcome

Given a gateway-enabled single node, a publicly trusted certificate for one DNS
hostname, one deployed HTTP Service with one TCP listener, and one top-level
Route pointing to that listener:

```
overdrive serve
overdrive deploy <SERVICE_SPEC>
overdrive deploy <ROUTE_SPEC>     # exact resource grammar is a DESIGN gap
curl https://api.example.com/<matched-path>
```

the external client receives the backend's real status, end-to-end headers, and
streaming body through the built production binary. No test installs a route,
binds the gateway socket, seeds a backend row, supplies a private side channel,
or bypasses the real CLI commands.

The notation above recommends that the existing generic `deploy` verb handle
the top-level Route resource; it does **not** specify the public enum, TOML, HTTP
endpoint, or function signature. If DESIGN chooses a different way to preserve
the real `overdrive deploy <SPEC>` journey while honoring #60, that approved
shape governs.

### Required now

1. **Component enablement and ownership**
   - an approved gateway-enabled node configuration per #267;
   - one public bind address/port distinct from `ServerConfig.bind`;
   - `serve` owns startup, health, cancellation, and join handles.
2. **Top-level Route intent**
   - stable Route identity and generation;
   - exact public hostname;
   - exact or segment-aware path prefix;
   - reference to one exact TCP Service listener;
   - reference to one operator-supplied certified key;
   - validation, conflict detection, withdrawal, and status semantics.
3. **Certificate boundary**
   - TLS 1.3 via the current rustls provider (`ring`, honestly named);
   - SNI selects the certified key;
   - SNI/HTTP authority mismatch is rejected;
   - private key is held only by the TLS owner and protected at rest through an
     approved mechanism.
4. **Read models**
   - initial List + watch/relist for Route intent;
   - exact Route target -> ServiceId/listener join;
   - initial `all_service_backends_rows()` List + accepted-row watch/relist;
   - only `healthy` backend entries eligible.
5. **Proxy**
   - HTTP/1.1 downstream and upstream;
   - strict request framing and bounded header/request/body streaming;
   - deterministic route match and round-robin selection;
   - strip hop-by-hop fields, add `Via`, preserve path/query;
   - stream response without whole-body buffering.
6. **Backend transport**
   - an approved gateway identity and outbound mTLS path to the canonical
     workload backend; no workload-identity impersonation and no plaintext
     downgrade;
   - pin the authenticated peer to the selected backend's published SPIFFE
     identity; chain-to-bundle authentication alone is insufficient for a
     public routing boundary (#242).
7. **Lifecycle/status**
   - coherent immutable configuration snapshots;
   - typed `Accepted`/reference/application equivalents with generation;
   - graceful listener/request drain and crash-rebuild behavior;
   - request, connection, route-resolution, TLS, backend-connect, and response
     telemetry without secret material.

### Safe exclusions

- HTTP/2, gRPC, gRPC-Web, WebSocket, CONNECT, HTTP/3/QUIC;
- UDP and arbitrary TCP public listeners;
- TLS passthrough and backend TLS policy variants;
- ACME account/order/challenge/renewal/revocation and every DNS provider;
- multiple public certificates except the one exact-host certificate needed by
  the walking skeleton;
- wildcard hostnames and per-workload URL auto-provisioning;
- middleware: auth, JWT, CORS, rate limit policy, circuit breaking, request
  mirroring, rewrites, header manipulation beyond mandatory proxy hygiene;
- request replay, request-body buffering, retries, hedging, and region hints;
- scale-to-zero/auto-wake;
- weighted routing, canary, sticky sessions, active outlier ejection;
- multi-node gateway placement, HA VIP/BGP/anycast, cross-region routing;
- public Service VIP allocation and issue #61; and
- operator-facing FIPS claims pending #204.

These are exclusions, not promises of a future issue. Existing issues #55–#58,
#61, #98, and #106 already cover the major named continuations.

### Dependency order

```
D0  Approve route/certificate and gateway-identity contracts
 |
 +--> D1  Route intent + deploy/read/status path
 |
 +--> D2  Public certified-key protected storage + rustls resolver
 |
 +--> D3  Gateway-to-mesh authenticated transport
          (D1-D3 can be designed together; none may invent API)
               |
               v
D4  Pure normalized listener/route/backend snapshot compiler
               |
               v
D5  Serve-owned public listener + HTTP/1.1 proxy + bounded resources
               |
               v
D6  Observation List/Watch/relist + atomic snapshot replacement
               |
               v
D7  Real serve + service deploy + route deploy + external curl evidence
```

The order prevents three horizontal-layer traps: a listener with no real route
writer, a route compiler with no production deploy path, and a proxy with no
authenticated backend transport.

### Control flow

1. `overdrive deploy <SERVICE_SPEC>` commits validated Service intent.
2. Existing WorkloadLifecycle starts the allocation; probes publish facts;
   ServiceLifecycle publishes complete healthy backend rows.
3. `overdrive deploy <ROUTE_SPEC>` commits validated Route/certificate
   references to IntentStore through the approved top-level resource path.
4. The gateway's route owner List/Watch projects intent and the backend watcher
   List/Watch projects observation into a normalized immutable snapshot.
5. Only after certified key, listener bind, route target, and initial snapshots
   are valid does the owner publish its application/programmed fact.

### Data flow

```
Route intent --------------------+
                                 |
Service listener intent -- join -+--> immutable GatewaySnapshot
                                 |       {listener, cert-ref, routes,
ServiceBackendRow observation ---+        healthy backend sets, generation}

Public request -> rustls TLS -> hyper HTTP/1.1 -> route match
              -> healthy backend pick -> gateway mTLS client transport
              -> existing inbound mesh termination -> workload plaintext socket
              -> response streamed back through gateway TLS
```

The traffic flow intentionally does not state an exact Rust API for the gateway
mTLS client. That is the unresolved D3 contract.

### Failure behavior

| Failure | Required behavior |
|---|---|
| Public bind unavailable | Typed `serve` startup refusal; no “gateway ready” status. Control-plane behavior on a gateway-enabled node must follow the approved lifecycle-gate decision. |
| Missing/malformed/mismatched certified key | Route/listener unresolved; never fall back to operator CA, workload CA, or plaintext. |
| Unknown SNI/authority | Reject before routing; never use a default certificate for another tenant/origin. |
| Invalid/ambiguous Route | Reject intent or mark unresolved according to approved admission semantics; retain the last fully accepted snapshot only if DESIGN explicitly allows it and reports generation honestly. |
| Backend observation unavailable at boot | Gateway listener must not claim programmed; fail closed or start with explicit unavailable status as DESIGN decides. Never treat read error as empty. |
| No healthy backend | Route remains known; new request returns 503; no stale backend fallback. |
| Selected backend connect/handshake failure | One bounded failure response (normally 502); no unapproved replay of a non-idempotent request. |
| Watch lag/closure | Relist authoritative state, atomically replace snapshot, expose degraded status; do not keep an unbounded event queue. |
| Request timeout/client disconnect | Cancel the backend request and release per-request resources; do not hold locks across await. |
| Graceful shutdown | Close admission, drain in-flight work to deadline, then cancel/join watchers and backend connections. |
| Crash/restart | Rebuild derived snapshots from intent + observation; rebind and resume only after prerequisites are proven. |

## Security, Lifecycle, and Observability Boundaries

### Security minimum

- TLS 1.3 server authentication with an origin-valid public chain.
- No TLS 1.3 0-RTT in the first slice; replay semantics are deliberately absent.
- Exact SNI/certificate/HTTP authority isolation.
- Strict HTTP/1.1 framing; reject malformed Host, duplicate Host, and ambiguous
  body framing per RFC 9112.
- Fixed maximum request-line/header count/header bytes, TLS-handshake deadline,
  header-read deadline, per-connection request cap, maximum concurrent
  connections, backend-connect deadline, response-header deadline, idle
  timeout, and total in-flight request ceiling. Exact values are DESIGN/config
  decisions; the existence and boundedness are required.
- Streaming request/response bodies with backpressure; no whole-body buffer.
- Remove hop-by-hop fields and add `Via`; define trusted proxy/X-Forwarded-For
  behavior before honoring client-supplied forwarding headers.
- Certificate private key accessible only to the certificate/TLS owner; zero
  secret values in logs, observations, CLI render, or route status.
- Authenticated gateway-to-workload mesh leg with a distinct gateway identity.
- Per-route/backend selection telemetry uses stable IDs, not raw secret or
  credential data.

Issue #150 shows why network admission limits matter even when application TLS
is correct: an unauthenticated remote peer can consume TCP/TLS handshake CPU
before identity is established. A complete DDoS product is excluded, but bounded
listener resources are part of correctness, not middleware scope.

### Lifecycle gate ownership

The first DESIGN must explicitly complete the repository's required Lifecycle
Gate Ownership section. The research identifies the likely distinct promises,
not their final names:

| Signal | Owner | Exact promise | Must not imply |
|---|---|---|---|
| Route accepted | Route admission/intent owner | Syntax and references are acceptable for reconciliation. | Listener bound, cert installed, backend healthy. |
| Route references resolved | Gateway route compiler | Certificate and Service listener target resolve for this generation. | Socket accepts traffic. |
| Gateway listener applied | Gateway runtime owner | Certified key and coherent route snapshot are installed on the bound listener. | Any backend is currently healthy. |
| Backend eligible | ServiceLifecycle | Backend is current Running membership and satisfies the existing readiness/terminal-veto policy. | Gateway has observed/applied the row. |
| Request success | Per-request proxy owner | One request was routed and a backend response returned. | Future requests will succeed. |

The gateway's readiness must not redefine allocation `Running`, Service
`Stable`, or backend `healthy`. Conversely, Service `Stable` must not gate the
public listener: ADR-0101 expressly says a non-vetoed Running allocation with no
readiness can be eligible and uses it as a counterexample to adding a Stable
gate (`adr-0101:489-498`).

### Observability minimum

The gateway should expose structured facts for:

- listener bind success/failure and active generation;
- route acceptance/reference/application state and last transition;
- current healthy candidate count per route (not backend secrets);
- TLS handshake success/failure class and SNI route identity;
- request count, active requests, duration, response class, bytes, and chosen
  backend identity;
- backend connect/handshake timeout versus HTTP response timeout;
- watch lag/relist/recovery; and
- graceful-drain start, forced deadline expiry, and completion.

Do not infer “programmed” from an intent commit alone. Gateway API's
`observedGeneration` and Cilium's ACK-aware xDS update are the precedents for
making stale status visible, even though Overdrive's in-process application can
use a smaller mechanism.

## Tests and Evidence Plan

### Pure/default lane

Property-based tests should cover:

- route hostname canonicalization and rejection grammar;
- exact path and segment-prefix behavior (`/api` matches `/api` and
  `/api/x`, not `/apix`);
- total deterministic precedence under every input ordering;
- ambiguity/conflict detection;
- Route target -> exact Service listener projection;
- backend filtering: selection never returns `healthy == false`;
- round-robin selection stays within the current candidate set and is fair
  within one cycle;
- immutable snapshot compiler output is byte/order deterministic for logically
  equivalent inputs; and
- hop-by-hop header removal and `Via` handling.

Every live pure-function property carries the repository-required exact rustdoc
line `/// CONTRACT_SHAPE: pure-function.`.

### Tier 1 deterministic simulation

Seeded invariants are required because correctness depends on event ordering,
watch loss, shutdown, and convergence:

- **Safety:** a request snapshot never combines Route generation N's target
  with generation N-1's certificate/reference facts.
- **Safety:** an unhealthy/withdrawn backend is never selected after the
  gateway applies the dominating `ServiceBackendRow`.
- **Liveness:** after route intent and backend observations stabilize, the
  gateway snapshot eventually converges to them.
- **Convergence:** watch lag/closure followed by relist reaches the same snapshot
  as an uninterrupted watch.
- **Late success:** an old certificate/backend resolution completion cannot
  overwrite a newer withdrawn or superseding generation.
- **Shutdown:** cancellation closes admission and every admitted request/task
  reaches completed or deadline-expired; no detached owner survives.
- **Feature disabled:** current `serve` and workload lifecycles remain
  byte/behavior equivalent and no public socket binds.

Simulation must drive real production owners through their port boundaries. It
must not seed a compiled gateway snapshot or fabricated healthy row as the
claimed composition proof.

### In-process integration

- route deploy request commits through the real handler/IntentStore path;
- initial List then Watch closes the boot window;
- malformed route/cert/backend reference produces the exact typed status/error;
- a real HTTP/1.1 client exercises strict Host/path matching, headers, streaming
  body, client disconnect, timeout, and 404/502/503 distinctions;
- atomic snapshot update under concurrent requests: each request observes one
  complete old or new generation, never a mix;
- certificate hot replacement affects new handshakes while existing
  connections follow the approved drain/keepalive rule; and
- server shutdown stops accepts then drains active requests within its budget.

### Tier 3 real-kernel/native production composition

The walking skeleton must build the default-feature `overdrive` binary and
exercise:

1. real `overdrive serve` with the gateway enabled;
2. real `overdrive deploy <SERVICE_SPEC>`;
3. real `overdrive deploy <ROUTE_SPEC>` through the approved route surface;
4. external `curl`/TLS client using an independently supplied trust root and
   DNS/SNI hostname;
5. the actual gateway listener -> authenticated gateway mesh client -> existing
   inbound mTLS/TPROXY -> workload path; and
6. backend response byte/status/header propagation.

Contrasting cases: unmatched host/path -> 404; no healthy backend -> 503;
untrusted/wrong-host certificate -> TLS failure; graceful shutdown permits an
in-flight bounded response but refuses a new connection. The test must not
manually bind the product socket, inject the Route snapshot, write
`ServiceBackendRow`, install TPROXY, or present a test-only SVID that production
does not compose.

The recurring test uses a hermetic test CA trusted explicitly by the external
client; that proves certificate selection and TLS behavior, not public Web PKI
issuance. A production/EDD capture may use a controlled DNS name and
operator-supplied public chain. The gateway never generates a fake public
certificate merely to make the integration test green.

### Black-box EDD expectation

One point-in-time expectation can retain the operator journey receipt at the
delivered SHA: commands for `serve`, both `deploy` operations, external TLS
client output, one unmatched request, one no-healthy-backend response, and
cleanup evidence. It directly drives the built binary and uses external tools;
it must not import an Overdrive crate or invoke the Rust test harness. The same
behavior remains defended by the recurring Tier 1/Tier 3 tests; the expectation
is not a fifth test tier.

## Explicit DESIGN Questions and Contract Gaps

These questions are blocking where marked. Research intentionally does not
answer them with invented public API.

1. **[BLOCKING] What exact top-level Route aggregate, persisted envelope,
   IntentKey family, HTTP wire shape, and `overdrive deploy <SPEC>`
   discrimination are approved?** Issue #60 supplies only intent placement, not
   signatures or schema.
2. **[BLOCKING] How does a Route reference exactly one Service listener?** A
   workload may have TCP and UDP on the same port or multiple TCP ports.
   Workload ID alone is ambiguous; raw `ServiceId` is derived from VIP and is
   not operator-friendly.
3. **[BLOCKING] Who owns the public certified key and how is it protected at
   rest?** Pin upload/ingest, validation, KEK/envelope use, authorization,
   rotation replacement, deletion, and redaction. Whitepaper storage prose is
   insufficient as an exact contract.
4. **[BLOCKING] What is the gateway's SPIFFE identity and issuer/lifecycle?**
   Pin the identity grammar, issuance owner, held-material owner, restart
   behavior, rotation action/workflow boundary, and gateway-to-backend
   verification policy. The selected `Backend.alloc` identity should close
   #242's intended-peer gap, but the exact enforcement shape remains a DESIGN
   contract.
5. **[BLOCKING] What exact outbound mesh transport surface does the gateway
   call?** The current `MtlsEnforcement` surface is intercepted-connection and
   `AllocationId` shaped. DESIGN must reuse or amend an approved port; delivery
   cannot add a convenient second method.
6. **What is the gateway enablement/config contract under #267?** Bind address,
   port, enabled set, boot refusal, and whether a gateway bind failure refuses
   all of `serve` or only a separately reportable gateway component.
7. **Which component compiles Route intent plus observation into the runtime
   snapshot?** If it is a Reconciler, pin State/View/Action and hydration; if a
   direct subsystem watch, explain why no convergent desired/actual owner is
   needed. No direct IntentStore writes from the proxy.
8. **What are the route/application status types and where do they live?** Pin
   accepted, unresolved, applied, degraded, observed generation, and typed
   causes without conflating them with Service health.
9. **What is the exact hostname/path precedence and conflict behavior?** Pin
   case normalization, IDNA policy, trailing dot, wildcard exclusion, path
   segment behavior, and final tie-break/rejection.
10. **What Host/authority reaches the backend?** Preserve public Host, replace
    with a Service authority, or carry both via Forwarded/X-Forwarded-Host.
    Pin trusted-proxy and spoofing rules.
11. **Which resource-limit values are fixed defaults versus operator input?**
    The first slice requires boundedness, but adding config fields without
    accepted shape is forbidden.
12. **What does one backend failure do?** Pin no retry versus bounded retry,
    which methods/bodies are retryable, and status mapping. Request replay #58
    must not leak into MVP accidentally.
13. **What happens to existing keep-alive requests/connections after route,
    certificate, or backend withdrawal?** Pin snapshot-at-request versus
    snapshot-at-connection and drain behavior.
14. **How are route intent changes awakened?** Current observation interests
    cover row families, but IntentStore has no demonstrated Route subscription
    surface in this research. Pin edge wake plus level backstop/relist.
15. **Does the current single-node public gateway require host firewall/rate
    admission integration?** #150 is broader; decide the minimum SYN/TLS
    handshake protection without importing a general firewall feature.
16. **Does the provider remain `ring` for the slice or must #204 land first?**
    Either is technically viable; documentation and compliance claims must
    match code.

## Traceability Matrix

| Conclusion / obligation | Issues | ADR / whitepaper | Current Overdrive code | Cilium / standards evidence | Conflict or status |
|---|---|---|---|---|---|
| Gateway is `serve`-owned infrastructure, not workload | #54, #267 | whitepaper `1282-1308` | `serve.rs:114-135`; `lib.rs:1608-1673` | Cilium node agent owns/supervises Envoy (`standalone_envoy.go:200-279`) | Accepted aspiration; gateway itself absent. |
| First slice is L7 HTTP termination | #54, #55 | whitepaper `1342-1421` | `hyper`/rustls deps exist; no gateway handler | Cilium distinct HTTP vs TLS passthrough model; Gateway API HTTPRoute/TLSRoute | Recommendation narrows open umbrella issues. |
| Service listener is not public gateway listener | #164, #60 | ADR-0047/0049; whitepaper `1391-1406` | `Listener { port, protocol }` at `workload_spec.rs:578-620` | Gateway API listener owns host/port/protocol/TLS | #164 body/comments stale on VIP; current type governs. |
| Route remains top-level intent | #60, #98 | whitepaper `1391-1406` | `SubmitSpecInput` has only workload kinds | Cilium Gateway and HTTPRoute are separate desired resources | Blocking API gap. |
| Healthy backend source is ServiceLifecycle observation | #170, #174 | ADR-0101 `71-120,255-278` | `service_lifecycle.rs:1101-1180`; row `observation_store.rs:1764-1775` | Cilium separates route model from dynamic EDS backend set | #174's BackendDiscoveryBridge is superseded. |
| Public bind does not require Service VIP/BGP | #24, #61, #175 | ADR-0049/0052/0061; whitepaper private VIP `1454-1464` | eBPF boot `lib.rs:2240-2376` | Cilium exposure Service is K8s-specific; host-network listener can bind directly | #61 explicitly corrects old whitepaper scope. |
| TLS now, operator cert first | #57, #4 | whitepaper `1350-1389`; ADR-0010 plaintext rejection | only private operator/workload CAs exist | RFC 8446/9110; rustls SNI resolver | #4 falsely says ACME “solved”; #57 open. |
| ACME is later multi-step lifecycle | #57, #40 | whitepaper `1354-1375`; workflow discipline | `instant-acme` unused | RFC 8555 domain validation + renewal | #40 comment corrects SVID/ACME conflation. |
| Gateway needs its own mesh identity | #35, #215 | ADR-0063/0067; whitepaper `1411-1421` | `IdentityMgr` keyed by `AllocationId`; `InterceptedConnection.alloc` | Gateway is distinct infrastructure identity in model terms | Blocking contract gap; whitepaper “shared IdentityMgr” lacks shape. |
| Selected backend identity must be pinned | #242 | ADR-0071 authn-only boundary | `Backend.alloc: SpiffeId`; `expected_peer` exists but v1 leaves it empty | TLS peer authentication must bind the routed origin, not only a shared trust root | Open security dependency/contract gap. |
| HTTP/1.1 first, H2 later | #55 | whitepaper lists both | control-plane supports both ALPN, proxy absent | RFC 9112 vs RFC 9113 multiplex/flow/GOAWAY burden | Compatible narrowing, not contradiction. |
| Atomic immutable snapshot + deterministic precedence | #60 | intent/observation split, deterministic-state rules | deterministic all-row snapshot; no gateway compiler | Cilium model compilation, route sort, xDS ACK/revert | Required primitive, API unapproved. |
| Status separates accepted/references/applied/healthy | #54, #60 | design lifecycle-gate discipline | no gateway status types | Gateway API conditions; Cilium generation/status | Must be designed; do not overload Service state. |
| Graceful shutdown is owned and bounded | #54 | whitepaper node subsystem | `ServerHandle::shutdown` `lib.rs:1392-1487` | Cilium supervised restart/stop; RFC 9113 GOAWAY later | Existing pattern reusable; gateway not yet joined. |
| Current crypto is `ring`, not ADR-0039 aws-lc-rs | #204 | ADR-0039 | `Cargo.toml:91-92`; `lib.rs:2143-2149` | rustls supports either provider | Explicit accepted-design/code contradiction. |

## Conflicts, Staleness, and Confidence

### Confirmed contradictions/stale artifacts

- Whitepaper §11 presents a complete gateway and unified public/internal
  `IdentityMgr`; current code has neither a gateway nor public certificate
  store/rotation or gateway identity.
- Whitepaper changelog `docs/whitepaper.md:2871` says the user verb remains
  `overdrive job submit`; current public API is top-level `overdrive deploy`, as
  `CLAUDE.md` and `cli.rs:39-62` prove.
- Issue #4 calls public TLS “solved” and names `lers`; current whitepaper/#57 use
  `instant-acme`, which is declared but unused.
- Issue #164's body and comments retain operator/pinned/optional VIP history;
  ADR-0049/current parser make VIP input unrepresentable.
- Issue #167 retains release-on-stop and pinned-VIP language; ADR-0049's later
  amendments changed lifecycle and platform-only assignment.
- Issue #170's comment assumes host/loopback backend endpoints; per-workload
  networking and ADR-0101 now materialize `workload_addr` when present.
- Issue #174 describes BackendDiscoveryBridge as publisher; ADR-0101/current
  code remove it in favor of ServiceLifecycle.
- Issue #243 describes headless backend-address answers; ADR-0072 REV-2/current
  code use a stable frontend address.
- Issue #35 describes internal SVID rotation as a workflow; issue #40's
  2026-06-09 comment supersedes it with a reconciler action and reserves the
  genuine multi-step workflow shape for external ACME.
- ADR-0039 selects aws-lc-rs, but issue #204 and current Cargo/composition code
  prove `ring` remains active.

### Confidence by major conclusion

| Conclusion | Confidence | Evidence count/quality |
|---|---|---|
| No gateway is currently implemented | High | Direct repository inventory plus CLI/server/config call paths and unused dependency. |
| Gateway belongs inside `serve` | High | Whitepaper, #54/#267, current composition/shutdown owner, Cilium node ownership. |
| L7 + TLS + HTTP/1.1 is the narrow correct slice | High | Product requirement, IETF standards, Cilium/Gateway API implementation precedent, scope dependency analysis. |
| Operator-supplied public cert should precede ACME | High | RFC 8555 lifecycle, #57 separation, current absence of any ACME call site. |
| `service_backends` is the correct backend source | High | Accepted ADR-0101, current publisher/read APIs, Cilium route/EDS separation. |
| Gateway-to-mesh identity is a blocker | High | Exact `AllocationId`-keyed current contracts versus subsystem ownership; no gateway identity exists. |
| Exact public Route API/storage shape | Intentionally unresolved | #60/whitepaper name the model but do not define signatures; design rule forbids invention. |
| Exact resource-limit numbers and retry policy | Medium/unresolved | Required by standards/security, but workload and operator semantics must be approved in DESIGN. |

## Sources

### Primary repository sources

All accessed 2026-09-12.

- Overdrive source and accepted documents at
  `ed52e9571137ca7175fdb1ff29c27c1473573fc9`; direct primary evidence,
  reputation 1.0, verified against production entry points.
- Cilium local source at
  `e99150f8d8f403eca51ed82138d4ae20a265c8f3`; direct primary evidence,
  reputation 1.0, verified across controller, model, translation, xDS, secret,
  status, and process-owner paths.
- GitHub issues #4, #24, #35, #40, #54–#58, #60–#61, #98, #106, #150,
  #164, #167, #170, #174–#175, #204, #215, #241–#243, #267; `github.com`,
  reputation 0.8 under trusted-source config, every issue verified with full
  comments.

### Standards and official project documentation

All accessed 2026-09-12.

- [RFC 8446 — TLS 1.3](https://www.rfc-editor.org/rfc/rfc8446.html),
  `rfc-editor.org`, reputation 1.0, IETF Standards Track, verified.
- [RFC 8555 — ACME](https://www.rfc-editor.org/rfc/rfc8555.html),
  `rfc-editor.org`, reputation 1.0, IETF Standards Track, verified.
- [RFC 9110 — HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html),
  `rfc-editor.org`, reputation 1.0, IETF Standards Track, verified.
- [RFC 9112 — HTTP/1.1](https://www.rfc-editor.org/rfc/rfc9112.html),
  `rfc-editor.org`, reputation 1.0, IETF Standards Track, verified.
- [RFC 9113 — HTTP/2](https://www.rfc-editor.org/rfc/rfc9113.html),
  `rfc-editor.org`, reputation 1.0, IETF Standards Track, verified.
- [Kubernetes SIG Network Gateway API repository](https://github.com/kubernetes-sigs/gateway-api),
  `github.com`, reputation 0.8, official project repository, verified.
- [Gateway listener schema](https://github.com/kubernetes-sigs/gateway-api/blob/main/config/crd/standard/gateway.networking.k8s.io_gateways.yaml),
  `github.com`, reputation 0.8, official generated standard CRD, verified.
- [HTTPRoute API source](https://github.com/kubernetes-sigs/gateway-api/blob/main/apis/v1/httproute_types.go),
  `github.com`, reputation 0.8, official API source, verified.
- [Gateway API TLS routing guide source](https://github.com/kubernetes-sigs/gateway-api/blob/main/site/content/en/guides/user-guides/tls-routing.md),
  `github.com`, reputation 0.8, official project documentation, verified.
- [rustls 0.23 server module](https://docs.rs/rustls/latest/rustls/server/)
  and [ServerConfig](https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html),
  `docs.rs`, reputation 1.0 under trusted-source config, official crate docs,
  verified. Note that docs.rs currently renders 0.23.44 while this workspace's
  `Cargo.lock:3072-3073` resolves rustls 0.23.39. The cited dynamic resolver and
  ALPN surfaces are present in the shared 0.23 API line; DELIVERY must still use
  the actual locked API rather than copy a signature from these notes.

### Quality summary

- Major claims have direct local code plus accepted-design/issue or external
  implementation/standard corroboration.
- External average configured reputation is above 0.9 when weighted by the
  five RFCs and official project sources used in the analysis.
- Citation coverage is above 95% for factual architecture, protocol, and issue
  claims; recommendations and explicitly unresolved questions are labeled as
  such.
