# Research: Ingress route activation vs backend health (Gate G-1)

**Date**: 2026-11-08 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (Envoy, Gateway API, ALB, Linkerd, Cloudflare verified from official docs; NGINX/HAProxy/Traefik partial) | **Sources**: 17 fetched/searched, 14 cited

## Executive Summary

**Question.** Should an ingress gateway gate activation of a new route configuration on reaching and mTLS-authenticating a backend of the route's service (Overdrive Gate G-1, `Selected` branch), or should route activation be independent of backend health?

**Answer.** Across every system examined, activation is gated on *configuration validity, resolvable references, and the proxy's own credentials/capabilities*, never on a backend being healthy or on a backend's TLS identity verifying. Backend absence and backend authentication failure are runtime, per-request or per-connection outcomes (503 for "no endpoints", 502/503/5xx for connect/handshake/verification failure), reported as status/metrics, not as an activation blocker. The closest precedents to a probe in the activation path are (a) Envoy cluster warming, which waits for *one completed* active health-check round but not for success, and (b) ALB, where a target must pass an initial check before it receives traffic. Neither blocks a route or listener on the check's *outcome*. No system found validates an upstream certificate's identity at config load. Gateway-local self-checks (Envoy holding a listener closed until its own SDS server cert arrives) are well established and are the legitimate analogue of G-1's capability, SVID and public-key checks.

**Recommendation.** Keep G-1's gateway-local prerequisites (kTLS/splice/relay probe, connect hooks, Gateway SVID Current, Public Certified Key, candidate validity). Remove the `Selected` -> `SelectedPeerAuthenticated` requirement from the bind/promote gate: treat both `ReadyNoBackend` and a failed `Selected` exact-peer mTLS as activation-neutral, serve 503 (no backend) or 502/503 (peer unreachable / peer authentication failure) per request, and surface the probe result as a cause-distinct, non-blocking status condition. Detect gateway-side credential misconfiguration with a gateway-local check (SVID loads, chains to trust bundle, signs) instead of with a real tenant backend. The user's objection is supported by the evidence. See "Synthesis" for the caveats.

## Research Methodology
**Search Strategy**: Fetched official documentation and API source for each system; cross-checked each major claim in at least two systems. Searches restricted to the trusted domains where a search tool was used.
**Source Selection**: official / technical_docs / open_source. `gateway-api.sigs.k8s.io` is the Kubernetes SIG-Network Gateway API project site and is treated as official, equivalent to kubernetes.io, per the task instruction; the API types were read from the project's own repository on github.com (`kubernetes-sigs/gateway-api`), which is the source of truth for that site's reference text (the rendered `/reference/spec/` and `/api-types/backendtlspolicy/` pages returned 404 when fetched).
**Quality Standards**: 2-3 sources per major claim where available. Vendor docs for NGINX (nginx.org), HAProxy (docs.haproxy.org) and Traefik are not on the trusted-domain list; their GitHub issue trackers are. Anything sourced from them is flagged and rated lower. Access date for all sources: 2026-11-08.
**Fetch caveat**: WebFetch answers are produced by a summarising model. Quotes below are what the fetch returned; items the fetch said a page did not cover are recorded as gaps, not guessed.

## Per-system table

| System | What gates route/listener activation | Backend unavailable ("no backend") | Backend TLS/mTLS failure | Conf. |
|---|---|---|---|---|
| **Envoy (xDS)** | LDS listener waits for its RDS (bounded by `initial_fetch_timeout`); clusters warm on initial discovery load (DNS/EDS update) **and** one completed active-HC pass if configured; SDS server cert blocks listener activation, SDS client cert blocks cluster activation | Routes to unwarmed cluster: 404 or 503; no healthy hosts: panic mode, or 503 "no healthy upstream" when panic threshold is 0 | Per-connection upstream failure (Istio doc names flag `UF`); not a warming condition. SDS *fetch* failure for client cert: cluster activates but requests are rejected | High |
| **Istio** | Inherits Envoy; Gateway `credentialName` secret missing/invalid is a config analysis error (IST0161) | 503 family via Envoy | `UF` 503 on mTLS/`DestinationRule` conflict; per-request | Medium-High |
| **Linkerd** | Not a config-activation gate in the sources read | "Failfast": proxy has no available endpoints, returns 503 or 504 immediately, exits when endpoints appear | Not covered in sources read | Medium |
| **K8s Gateway API** | Route `Accepted` (config accepted by Gateway) and `ResolvedRefs` (references resolve); neither includes backend health | Service with no ready endpoints: implementations SHOULD return 503 | `BackendTLSPolicy`: invalid CA ref -> conditions `False`; "Connections using an invalid CACertificateRef MUST fail, and the client MUST receive an HTTP 5xx". Hostname/SAN mismatch is a connection-time check | High |
| **Gateway API invalid backend** | Route is still `Accepted`; `ResolvedRefs=False` (`BackendNotFound`, `InvalidKind`, `RefNotPermitted`) | Invalid backendRef: 500 MUST be returned for requests that would have gone there (config defect, distinct from 503 "no ready endpoints") | n/a | High |
| **NGINX** | Config syntax/parse; static `proxy_pass` hostnames resolved at load (from general knowledge, unverified in the fetch) | 502/503 per request | `proxy_ssl_verify` is off by default; page does not state the failure status (502 typical, unverified) | Low-Medium |
| **HAProxy** | Config parse; servers can start DOWN (`init-addr none`, from GitHub config examples) | 503 "when no server was available" (manual section 1.4.1) | Not verified (manual section 5.2 not reached) | Low |
| **Traefik** | Not verified from official docs | Not verified | Issues show backend TLS/handshake failures surface as 502 per request | Low |
| **Cloudflare** | Zone/LB config validity; Full (strict) is a mode setting with no documented origin probe at setting time | LB: all pools down and no fallback -> 530/1016; origin refused -> 521; timeout -> 522 | Full (strict): origin cert unverifiable -> 526 per request | Medium-High |
| **AWS ALB** | Listener rule referencing a target group; target must pass initial health check to receive traffic | No registered targets (or all `unused`): 503. All targets unhealthy: **fail open**, route to all | "SSL handshake error when connecting to a target": 502; handshake timeout: 504 | High |

## Findings

### Finding 1: Envoy gates warming on discovery data and one completed health-check round, not on backend health
**Evidence**: Clusters "do not become available until": "Initial service discovery load (e.g., DNS resolution, EDS update, etc.)" and "Initial active health check pass if active health checking is configured." A newly added cluster "will not appear to exist" until warmed; routes to it return "either a 404 or 503 (depending on configuration)"; an updated cluster keeps serving the old version until the new one warms, then swaps atomically.
**Source**: [Envoy cluster manager](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/cluster_manager), accessed 2026-11-08.
**Verification**: [Envoy initialization](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/operations/init): "If clusters use active health checking, Envoy also does a single active health check round"; LDS/RDS wait "for a bounded period of time for at least one response (or failure)"; Envoy "always initializes within `initial_fetch_timeout`".
**Confidence**: High (official, two pages; the docs do not say a failed first round blocks warming, and describe the gate as completing a pass).
**Analysis**: This is Overdrive's closest precedent for a probe on the activation path, and it is weaker than G-1: the wait is for the round to *complete*, and it is bounded. Interpretation (not stated in the doc): "pass" means "round done", since a fully-down cluster still activates and then yields 503/panic behavior (Finding 2). I did not find an explicit sentence confirming that a failed round does not block warming.

### Finding 2: With no healthy hosts Envoy keeps the route and degrades per request
**Evidence**: Zero availability is below the default 50% panic threshold, so Envoy enters panic mode: "traffic will go to all (or no) hosts in that priority level regardless of their availability." With panic threshold 0 Envoy "will fail to select a host and will instead immediately return error responses," specifically 503 "no healthy upstream".
**Source**: [Envoy panic threshold](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/panic_threshold), 2026-11-08.
**Verification**: ALB behaves the same way (Finding 7: fail-open); Linkerd failfast returns 503/504 (Finding 6).
**Confidence**: High.

### Finding 3: Gateway-local secrets gate activation; failure of the secret fetch does not
**Evidence**: With a remotely fetched server certificate, the listener "will NOT be marked as active, its port will not be opened before the certificates are fetched." If the fetch fails (connection failure or bad data) "the listener will be marked as active, and the port will be open, but the connection to the port will be reset." A cluster with a remotely fetched client certificate "will NOT be marked as active... before the certificates are fetched"; on fetch failure "the requests routed to that cluster will be rejected." "By default SDS certificate fetching blocks initialization of the listeners and the clusters that reference them."
**Source**: [Envoy SDS configuration](https://www.envoyproxy.io/docs/envoy/latest/configuration/security/secret), 2026-11-08.
**Verification**: [Istio IST0161](https://istio.io/latest/docs/reference/config/analysis/ist0161/) flags a Gateway `credentialName` that points at a missing or invalid secret as a configuration analysis error (found via search summary; page body not fetched). Gateway API `ResolvedRefs` for unresolvable certificate/CA references (Finding 5).
**Confidence**: High for Envoy; Medium for Istio (search summary only).
**Analysis**: Direct precedent for G-1's gateway-local checks (own identity, own public key, own capabilities). Note even here the system's behavior on a *failed* own-credential fetch is "activate and reject", not "stay unbound forever", after the bounded wait. It is the gateway's own credential, not a tenant backend.

### Finding 4: Upstream TLS verification failure is a per-connection error, not an activation blocker
**Evidence**: Istio: `UF` is "Failed to connect to upstream, if you're using Istio authentication, check for a mutual TLS configuration conflict"; 503s that begin after a `DestinationRule` is applied are attributed to a TLS mode mismatch. Cloudflare: "Error 526 indicates that Cloudflare is unable to verify the SSL certificate on your origin server", occurring when "Full SSL (Strict)" is set; the Full (strict) page documents no verification step when the mode is set and says only that otherwise "your visitors may experience a 526 error". ALB: HTTP 502 includes "The load balancer encountered an SSL handshake error when connecting to a target"; 504 includes SSL handshake timeout.
**Sources**: [Istio network issues](https://istio.io/latest/docs/ops/common-problems/network-issues/); [Cloudflare error 526](https://developers.cloudflare.com/support/troubleshooting/http-status-codes/cloudflare-5xx-errors/error-526/) and [Full (strict)](https://developers.cloudflare.com/ssl/origin-configuration/ssl-modes/full-strict/); [AWS ALB troubleshooting](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/load-balancer-troubleshooting.html), all 2026-11-08.
**Verification**: Gateway API BackendTLSPolicy (Finding 5). Traefik issues #11284, #10403 show handshake/verification failures surfacing as 502 per request (GitHub issues, community evidence, Low weight).
**Confidence**: High (three official sources agree). Negative finding: **no system examined validates the upstream certificate identity against a live backend at config load.** This is a negative result from the sources read, not proof of absence in every product (NGINX/HAProxy/Traefik official docs not fully read).

### Finding 5: Gateway API separates Route acceptance, reference resolution and backend health, with distinct status codes
**Evidence**:
- `Accepted`: "whether the route has been accepted or rejected by a Gateway, and why." `ResolvedRefs`: "whether the controller was able to resolve all the object references for the Route." `BackendNotFound`: "one of the Route's rules has a reference to a resource that does not exist."
- On invalid refs "the implementation MUST ensure the `ResolvedRefs` Condition on the Route is set to `status: False`", reason `BackendNotFound` for a missing resource.
- "When a HTTPBackendRef is invalid, 500 status codes MUST be returned" for the affected requests, while "When a HTTPBackendRef refers to a Service that has no ready endpoints, implementations SHOULD return a 503."
- BackendTLSPolicy: invalid CA refs set `ResolvedRefs=False` (`InvalidCACertificateRef`, `InvalidKind`), and if all are invalid `Accepted=False` (`NoValidCACertificate`); "Connections using an invalid CACertificateRef MUST fail, and the client MUST receive an HTTP 5xx error response." Hostname "MUST be used as the SNI" and for authentication; with `SubjectAltNames` the backend certificate "MUST have at least one Subject Alternate Name matching one of the specified SubjectAltNames."
**Sources**: [HTTPRoute types](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api/main/apis/v1/httproute_types.go), [shared types](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api/main/apis/v1/shared_types.go), [BackendTLSPolicy types](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api/main/apis/v1/backendtlspolicy_types.go), all `kubernetes-sigs/gateway-api` (backing gateway-api.sigs.k8s.io), 2026-11-08.
**Verification**: Envoy Gateway implements the 500/`BackendNotFound` behavior (search summary of gateway.envoyproxy.io troubleshooting, not fetched directly); Envoy panic/503 (Finding 2); Linkerd 503 (Finding 6).
**Confidence**: High for quoted spec text (single repository, but it is the normative source); Medium for implementations.
**Analysis**: This is the cleanest statement of the user's position. Config-level validity (is the reference resolvable, is the CA material present and parseable) is a Route/Policy status; whether the backend is up or its certificate matches is a per-request 5xx. The "missing Service" case is a *reference* defect (500, `ResolvedRefs=False`) and is a different case from "Service exists, zero endpoints" (503). For Overdrive, G-1's `NoBackend` corresponds to the latter, so 503 is the aligned code. The spec says `Accepted` stays independent of both.

### Finding 6: Linkerd fails fast per request when a Service has no endpoints
**Evidence**: Failfast: "the proxy has determined that there are no available endpoints for that service"; "The error will be either a 503 or a 504"; "To get out of failfast, some endpoints for the service have to become available." For multi-backend HTTPRoutes, per the circuit-breaking page (via search summary) the unavailable backend Service is treated as unavailable while another backendRef may be selected.
**Source**: [Linkerd failfast](https://linkerd.io/2-edge/common-errors/failfast/), 2026-11-08; [Linkerd circuit breaking](https://linkerd.io/2-edge/reference/circuit-breaking/) (summary only).
**Confidence**: Medium (one page fetched; second summary only). Linkerd's docs examined do not discuss config activation gating, so this finding supports only the per-request outcome.

### Finding 7: AWS ALB accepts rules for empty or unhealthy target groups and fails open
**Evidence**: 503 occurs when "The target groups for the load balancer have no registered targets, or all of the registered targets are in an `unused` state." "If a target group contains only unhealthy registered targets, the load balancer routes requests to all those targets, regardless of their health status... the load balancer fails open." "Before a target can receive requests from the load balancer, it must pass the initial health checks."
**Sources**: [ALB troubleshooting](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/load-balancer-troubleshooting.html); [ALB target group health checks](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/target-group-health-checks.html), 2026-11-08.
**Confidence**: High on runtime behavior. I did not find an explicit statement of whether listener rule creation is rejected for an empty target group; the 503 cause list implies it is accepted (inference).

### Finding 8: Cloudflare Load Balancing degrades per request when pools are down
**Evidence**: A pool in the Critical state stops receiving traffic "unless other pools are also unhealthy and it is set as the fallback pool"; with all pools down and the fallback disabled the load balancer is Critical and Cloudflare returns 530/1016; 521 = origin refused, 522 = timeout; zero-downtime failover retries once on 521/522/523/525/526 if another healthy endpoint exists.
**Source**: [Cloudflare Load Balancing health details and 5xx docs](https://developers.cloudflare.com/load-balancing/understand-basics/health-details/) and [5xx errors](https://developers.cloudflare.com/support/troubleshooting/http-status-codes/cloudflare-5xx-errors/), via search summary, 2026-11-08.
**Confidence**: Medium (search summaries only; pages not fetched in full). The search found no statement that LB creation is blocked by unhealthy pools; unverified.

## Synthesis and recommendation for Gate G-1

### Direct answer
Route activation should not require a backend of the Route's Service to exist, be reachable, or authenticate. Evidence (Findings 1-8): Gateway API defines `Accepted`/`ResolvedRefs` without reference to backend health and assigns 503 to "no ready endpoints"; Envoy and ALB both activate configuration whose backends are all down and degrade per request; every system maps backend TLS verification failure to a per-request 5xx (502/503/526). The gateway's *own* credential and capability prerequisites do gate activation (Envoy SDS listener hold), which matches the part of G-1 that is not about the backend.

### Where the user's objection is correct
G-1's `Selected` branch makes a *Service-role* fact (a backend's identity and reachability) a prerequisite of a *Route-role* event (binding :443 for a generation). The same-shaped dependency is absent from every system examined. Its observable consequence: whether the public listener binds depends on which backend the dataplane happens to select at probe time.

### Trade-off analysis (questions 5)
1. **Blast radius.** Probe-gated: one backend that is mid-restart, not yet carrying its SVID, or briefly failing mTLS can refuse a whole generation, which includes unrelated hostnames/routes in the same generation (inference from G-1's design: the gate is per candidate generation, and on first boot refusal leaves :443 unbound). Independent: the blast radius of the same fault is the requests routed to that Service only. Envoy's warm-then-swap and ALB's fail-open both exist to keep the blast radius per-backend. Confidence: Medium (reasoned from the docs plus G-1 text; I read only G-1 in feature-delta, not how a refused later generation behaves).
2. **Early misconfiguration detection.** The legitimate need is real: a wrong gateway client cert or trust bundle would make *every* backend fail. But a probe against whichever backend is selected is a poor detector: a failure is ambiguous between a backend fault and a gateway fault, and absence of a backend (`ReadyNoBackend`) yields no signal at all, so the check is skipped exactly when it cannot run. A gateway-local self-check (SVID parses, chains to the current trust bundle, key matches, and can sign a handshake against an in-process peer, as G-1 already does for `GatewayRelay::probe` with an ephemeral in-process certificate) detects the gateway-side faults without depending on tenant state. Precedent: Envoy SDS holds the listener for its own certificate (Finding 3). Confidence: Medium (reasoning; precedent is for presence of own credential, not for a handshake self-test; I found no proxy that does a loopback self-handshake).
3. **Rolling deploys.** During a deploy, backends churn, temporarily reaching zero ready or having peers whose SVID is not yet provisioned. Probe-gated activation couples route updates to that churn and can produce flapping refusals; independent activation lets the route stay current and returns 503 during the gap (Gateway API 503 SHOULD, Linkerd failfast, ALB 503). Conversely, Envoy keeps the *old* cluster serving until the new one is warmed (Finding 1), so a probe-gated design is tolerable only if a refused candidate leaves the previous generation serving. Whether Overdrive does that on later generations was not verified here. Confidence: Medium.

### Recommendation for Gate G-1 (interpretation, not a sourced claim)
1. Keep as bind/promote prerequisites: Public Certified Key usable, candidate Route/frontend valid, `GatewayRelay::probe`, `gateway_connect.probe`, Gateway Identity Current. These are the gateway-local and config-validity gates, consistent with Envoy SDS and Gateway API `Accepted`/`ResolvedRefs`.
2. Drop `SelectedPeerAuthenticated` as a prerequisite. Both `ReadyNoBackend` and a `Selected` result that fails identity resolution or exact-peer mTLS should leave activation unaffected. Serve 503 for no backend and 502/503 (pick one consistently and document it; Gateway API demands only "5xx" for TLS failure) for unreachable or authentication-failed peers, per request.
3. Keep the actual-frontend probe, but as an advisory status signal (cause-distinct: no backend / peer unreachable / peer authentication failed), analogous to Gateway API conditions and Envoy health/outlier stats, not a gate.
4. If early detection of gateway-side TLS misconfiguration is wanted, extend the gateway-local checks (SVID chain to trust bundle, key usable) rather than relying on a real backend. This is a proposal in the sense of `.claude/rules/design.md` ("agents propose, the user adds"): it adds scope and needs a user ruling, with cost and cheaper alternatives stated (the existing relay probe's in-process ephemeral certificate is the cheaper baseline).
5. Distinguish "Service reference does not resolve" (Route reference defect; Gateway API: `ResolvedRefs=False`, 500) from "Service resolves, no backend" (503). If Overdrive's Route can name a nonexistent Service, that case belongs in the candidate-validity gate, not in the probe.

Caveat on this recommendation: it is based on documented behavior of other systems and on G-1 as written in `feature-delta.md:426-473`. I did not read the ADR rationale for why the `Selected` branch was added. If it exists to catch a specific production failure (for example, the gateway's SVID being rejected by workloads, per spike evidence), that failure should be re-examined against recommendation 4 before the branch is removed (`design.md`: "trace why it was added" before removing a mechanism).

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Envoy cluster manager | envoyproxy.io | High | official/open_source | 2026-11-08 | Y |
| Envoy init | envoyproxy.io | High | official | 2026-11-08 | Y |
| Envoy SDS | envoyproxy.io | High | official | 2026-11-08 | Y (Istio IST0161) |
| Envoy panic threshold | envoyproxy.io | High | official | 2026-11-08 | Y |
| Istio network issues | istio.io | High | official | 2026-11-08 | Y |
| Istio IST0161 | istio.io | High | official (summary only) | 2026-11-08 | N |
| Linkerd failfast / circuit breaking | linkerd.io | High | official | 2026-11-08 | Partial |
| Gateway API types (httproute, shared, backendtlspolicy) | github.com/kubernetes-sigs (gateway-api.sigs.k8s.io project) | High (treated as official) | official source | 2026-11-08 | Y |
| AWS ALB troubleshooting / health checks | docs.aws.amazon.com | High | technical_docs | 2026-11-08 | Y |
| Cloudflare 526, Full (strict), LB health, 5xx | developers.cloudflare.com | High | open_source/official | 2026-11-08 | Y |
| Traefik GitHub issues #11284, #10403 | github.com | Medium | community | 2026-11-08 | N |
| HAProxy GitHub configs / manual section 1.4.1 | github.com, docs.haproxy.org (not on list) | Medium | community/vendor | 2026-11-08 | N |
| NGINX proxy module | nginx.org (not on list) | Medium | vendor | 2026-11-08 | N |

Reputation: High 10 of 13 (77%); Medium 3 (23%). The three Medium sources are used only for low-confidence rows and are flagged.

## Knowledge Gaps
### Gap 1: NGINX, HAProxy, Traefik official behavior
**Issue**: Failure status on upstream certificate verification failure, and config-load behavior, not confirmed from official docs. nginx.org fetch said the page does not state the status on verification failure; HAProxy manual section 5.2 (`verify`, `ca-file`) was beyond the fetched window. **Attempted**: nginx.org proxy module, docs.haproxy.org, GitHub searches for HAProxy and Traefik. **Recommendation**: read `haproxy/haproxy` `doc/configuration.txt` section 5.2 and Traefik docs/source for `serversTransport` and health-check semantics.

### Gap 2: Does Envoy's initial health-check pass block warming on failure?
**Issue**: The doc says warming waits for the pass; no sentence found on the failure case. **Attempted**: cluster manager and init pages; health-checking page did not cover it. **Recommendation**: read Envoy source (`cluster_manager_impl`, `health_checker`) or the cluster-warming code path in `envoyproxy/envoy`.

### Gap 3: Istio and Cloudflare gaps
**Issue**: Istio Gateway behavior with a missing secret rests on a search summary (IST0161); Cloudflare LB pages were summarized, not read in full; whether LB creation is blocked with all pools unhealthy was not found.

### Gap 4: Any proxy that validates upstream certificate identity at config load
**Issue**: None found; negative result limited to the systems and pages read. Also no precedent found for a proxy doing a loopback mTLS self-handshake as a pre-bind check.

### Gap 5: Overdrive-side behavior of a refused later generation
**Issue**: Not verified whether a refused candidate leaves the previous Current generation serving; only G-1 (lines ~426-473) was read.

## Conflicting Information
### Conflict 1: 500 vs 503 for backend problems (Gateway API)
**Position A**: Invalid backendRef (nonexistent, wrong kind, not permitted): 500 MUST — [Gateway API httproute_types.go](https://raw.githubusercontent.com/kubernetes-sigs/gateway-api/main/apis/v1/httproute_types.go).
**Position B**: Service with no ready endpoints: 503 SHOULD — same file.
**Assessment**: Not a contradiction; the spec distinguishes reference defects from runtime unavailability. Relevant to Overdrive because G-1's `NoBackend` is the latter.

### Conflict 2: Envoy "no healthy hosts" behavior
**Position A**: Panic mode sends traffic to all hosts regardless of health (default threshold) — Envoy panic threshold page. **Position B**: 503 "no healthy upstream" with threshold 0. **Assessment**: Configuration-dependent, same page. ALB fail-open matches A.

## Recommendations for Further Research
1. Read Envoy source to settle Gap 2 (does a failed initial HC round block warming).
2. Complete HAProxy 5.2 and Traefik official docs (Gap 1).
3. Review the ADR/spike history for why G-1 includes the `Selected` mTLS branch.

## Full Citations
[1] Envoy Project. "Cluster manager". https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/cluster_manager. Accessed 2026-11-08.
[2] Envoy Project. "Initialization". https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/operations/init. Accessed 2026-11-08.
[3] Envoy Project. "Secret discovery service (SDS)". https://www.envoyproxy.io/docs/envoy/latest/configuration/security/secret. Accessed 2026-11-08.
[4] Envoy Project. "Panic threshold". https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/load_balancing/panic_threshold. Accessed 2026-11-08.
[5] Istio. "Common problems: Network issues". https://istio.io/latest/docs/ops/common-problems/network-issues/. Accessed 2026-11-08.
[6] Istio. "IST0161 InvalidGatewayCredential". https://istio.io/latest/docs/reference/config/analysis/ist0161/. Accessed 2026-11-08 (search summary only).
[7] Linkerd. "Failfast". https://linkerd.io/2-edge/common-errors/failfast/. Accessed 2026-11-08.
[8] Kubernetes SIG-Network. Gateway API `apis/v1/httproute_types.go`, `shared_types.go`, `backendtlspolicy_types.go`. https://github.com/kubernetes-sigs/gateway-api (project site gateway-api.sigs.k8s.io, treated as official). Accessed 2026-11-08.
[9] AWS. "Troubleshoot your Application Load Balancers". https://docs.aws.amazon.com/elasticloadbalancing/latest/application/load-balancer-troubleshooting.html. Accessed 2026-11-08.
[10] AWS. "Health checks for Application Load Balancer target groups". https://docs.aws.amazon.com/elasticloadbalancing/latest/application/target-group-health-checks.html. Accessed 2026-11-08.
[11] Cloudflare. "Error 526". https://developers.cloudflare.com/support/troubleshooting/http-status-codes/cloudflare-5xx-errors/error-526/ ; "Full (strict)" https://developers.cloudflare.com/ssl/origin-configuration/ssl-modes/full-strict/ ; "How endpoints and pools become unhealthy" https://developers.cloudflare.com/load-balancing/understand-basics/health-details/. Accessed 2026-11-08.
[12] Traefik GitHub issues. https://github.com/traefik/traefik/issues/11284 , https://github.com/traefik/traefik/issues/10403. Accessed 2026-11-08.
[13] HAProxy. Configuration manual §1.4.1. https://docs.haproxy.org/3.0/configuration.html (not on trusted list). Accessed 2026-11-08.
[14] NGINX. "ngx_http_proxy_module". https://nginx.org/en/docs/http/ngx_http_proxy_module.html (not on trusted list). Accessed 2026-11-08.

## Research Metadata
Examined: ~22 fetches/searches | Cited: 14 | Cross-refs: 5 major claims across 2+ systems | Confidence: High 5 findings, Medium 3, Low rows for NGINX/HAProxy/Traefik | Output: docs/research/networking/ingress-route-activation-vs-backend-health-comprehensive-research.md
