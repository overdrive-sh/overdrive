# Research: Host health probes and the mTLS interception path

**Date:** 2026-09-08–09. **Researcher:** Codex, bounded independent research. **Status:** Complete research assessment; no architecture or implementation authorization. **Confidence:** High in the observed socket path and established separation of health concerns; conditional in the assessment of this product's complete security and availability contract.

## Question and evidence boundary

Assess whether marking host TCP/HTTP health-probe sockets to select the existing mTLS interception exemption is an appropriate design. Separate direct application health from authenticated mesh-path health. Read current production owners and primary upstream documentation. No production changes, native execution, design amendments, issues, or commits are part of this research.

## Initial primary-source findings

- Istio documents the same two problems: kubelet HTTP probes cannot authenticate under strict mesh mTLS; a TCP handshake can succeed against interception even when the application port is closed. Its default probe rewrite delegates to the sidecar agent, whose TCP check avoids redirection. This is direct evidence that a deliberately separate application-health path is an established design. [Istio: Health Checking of Istio Services](https://istio.io/latest/docs/ops/configuration/mesh/app-health-check/) (accessed 2026-09-08).
- Istio ambient uses host-origin verification and link-local source translation for plaintext kubelet probes. Its exception is about a trusted local origin; a blanket application-port exception is explicitly unsuitable. It also distinguishes mesh enforcement from CNI NetworkPolicy. [Istio: Ambient and Kubernetes NetworkPolicy](https://istio.io/latest/docs/ambient/usage/networkpolicy/) (accessed 2026-09-08).
- Linkerd identifies kubelet readiness requests as non-mTLS traffic and provides separate traffic inspection to establish whether actual service connections use mTLS. [Linkerd: Validating your mTLS traffic](https://linkerd.io/docs/tasks/validating-your-traffic/) (accessed 2026-09-08).

## Assessment

**Using the mark is a sound mechanism for a trusted host agent to check the application's own listener. It is not merely an arbitrary workaround: Istio deliberately solves the same interception problems by avoiding interception for application probes. However, the mark alone is not a complete design for proving that a Service is usable through the mesh.** The distinction is the promise attached to the result and the trust boundary of the selected destination.

For a provisioned local VM endpoint, selecting the existing agent-to-application path makes TCP success describe the guest's listener instead of the worker's TLS listener, and makes an HTTP result describe the guest's HTTP response. The probe intentionally does not authenticate an inter-agent connection. This is appropriate for application startup and liveness and can provide the application component of readiness. It is insufficient if `Pass`, `Stable`, or `Backend.healthy` is promised to establish successful authenticated mesh traffic. Whether an additional availability requirement is wanted is a contract decision; this research establishes a coverage distinction, not a newly reproduced production failure.

The implementation also accepts explicit hosts and marks every resolved non-loopback address. Consequently, describing the current feature as “local VM probes only” would be incorrect. Explicit remote targets and other local workloads require a separately explicit scope/trust interpretation. That does not establish an exploitable bypass: a local socket mark is not a credential carried over an inter-node network, and the receiving node's rules still apply.

## Current production evidence

Read against working-tree HEAD `b653e1ad1758d11be33be457849b284f78141333`. The checkout contains concurrent unrelated work; this document neither changes nor validates those changes. Line references describe the inspected snapshot and can move with subsequent edits. Historical ADR reproductions below were not rerun by this researcher.

| Boundary | Observed implementation and evidence |
| --- | --- |
| Production composition | `crates/overdrive-control-plane/src/lib.rs:1692–1701` supplies `TokioTcpProber`, `HyperHttpProber`, and `CgroupExecProber`; `:1733–1741` passes the shared runner to VM composition. |
| Start ordering | `crates/overdrive-control-plane/src/action_shim/mod.rs:2190–2231` awaits `mtls_lifecycle.start_alloc`; an error takes `fail_closed_on_mtls_install`; successful install precedes VM execution release and `driver.on_alloc_running`. The restart path has the corresponding sequence at `:2758–2791`. Thus “probes pass with no intercept ever installed” cannot simply be assumed from the private probe adapter. |
| VM ownership | `crates/overdrive-worker/src/vm_driver.rs:1894–1903` starts the runner on Running, stops all allocation probes on terminal, and stops only Startup on Stable. |
| Task and target ownership | `crates/overdrive-worker/src/probe_runner/mod.rs:298–352` clones descriptors, projects targets, and starts per-role supervised tasks. `:452–481` projects only wildcard TCP and omitted/wildcard HTTP hosts to the provisioned workload address; other explicit hosts return unchanged. `:442–445` also preserves explicit hosts at attempt time. |
| Explicit target parsing | `crates/overdrive-core/src/aggregate/workload_spec.rs:1352–1369` accepts the TCP host string; `:1590–1632` validates HTTP path/port and preserves optional host. Neither inspected parsing branch imposes local-node or owning-allocation membership. |
| Result ownership | `probe_runner/mod.rs:536–555` invokes the adapters with target and timeout; `:600–619` converts their outcomes into `ProbeResultRow` and writes the observation store. It supplies no requesting-workload identity or expected mesh-peer identity to either network adapter. |
| TCP observation | `probe_runner/tcp_prober.rs:72–85` reports Pass on the first completed TCP connection and drops the stream without application data. `:95–144` resolves candidates, sets `SO_MARK` before connecting every non-loopback candidate, and preserves the connection error path if marking fails. |
| HTTP observation | `probe_runner/http_prober.rs:70–137` performs corresponding resolution and pre-connect marking. `:193–232` accepts `http://`, constructs a GET through that connector, and classifies the status: 2xx passes; redirects fail. This adapter performs no TLS handshake. |
| Exactly what the exemption selects | `crates/overdrive-core/src/dataplane/mtls_mark.rs:21` defines `0x2`. `crates/overdrive-worker/src/mtls_intercept.rs:349–391` installs per-address/per-port inbound and OUTPUT rules. `:680–695` documents the distinct `0x1` routing mark and the shared `0x2` exemption at each chain head. `crates/overdrive-netlink/src/nft.rs:591–619` encodes the actual mark comparison, OUTPUT divert, and accept exemption. |
| Capture scope | `crates/overdrive-worker/src/mtls_intercept_worker.rs:921–926` installs inbound interception for each declared Service port at the allocation's workload address. It is not a blanket divert of every destination or every guest port. |
| Startup verdict | `crates/overdrive-reconcilers/src/service_lifecycle.rs:547–588` produces Stable for Running plus the startup opt-out or observed startup Pass. This is not a TLS handshake verdict. |
| Readiness and backend membership | `service_lifecycle.rs:1092–1127` projects Running allocations into backend rows, with a terminal-startup veto. `:1176–1207` computes `healthy` from that veto and the readiness observation/threshold; absent readiness defaults true. This function has no TLS-availability input. |
| Mesh consumer of health | `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:491–502` selects a healthy backend. `:529–581` maps known unavailable mesh destinations to `MeshUnreachable`. `crates/overdrive-worker/src/mtls_intercept_worker.rs:1321–1383` enforces mTLS for Mesh, drops MeshUnreachable, and reserves cleartext pass-through for NonMesh. This is the relevant mesh path; the host/LB hydrator is not interchangeable with it. |
| Actual TLS enforcement | `crates/overdrive-dataplane/src/mtls/mod.rs:151–155,817–852` requires the allocation's held SVID and trust bundle; missing material returns typed failure. `mtls/inbound.rs:40–73` verifies the peer during the TLS handshake and arms kTLS before dialing the plaintext application leg. The worker/identity manager supplies the material; the application holds none. |
| Liveness responsibility | `service_lifecycle.rs:1048–1071` emits `StopAllocation` with `StoppedBy::LivenessProbe` at its threshold. `WorkloadLifecycle` owns the restart budget; a hypothetical mesh failure must not silently become an application liveness failure. |

`ADR-0092` and `ADR-0094` both retain **Proposed** status in their files (`docs/product/architecture/adr-0092-marked-host-http-probe-connector.md:3–7`; `adr-0094-marked-host-tcp-probe-sockets.md:3–7`). Their context sections record prior native reproductions of HTTP protocol failure and TCP false success respectively. Their presence, approval narrative, and implementation are evidence of intended scope and history; this assessment does not treat them as independent proof of correctness or new user authorization.

For completeness, the inspected probe loop waits on its clock or cancellation token, then awaits the attempt inside the selected branch (`probe_runner/mod.rs:651–677`). Adapter I/O is timeout-bounded; later attempts follow the descriptor interval. No ordering/cancellation bug is asserted here, and no private-task cancellation schedule has been manufactured to justify one.

## What the primary sources establish

### Application probes are commonly separate from mesh authentication

Istio's default rewrite and Linkerd's visible non-mTLS kubelet probes independently establish that plaintext application health checks can coexist intentionally with mesh authentication. The upstream pattern supports the **separation**, not every detail of Overdrive's implementation. In particular, Istio ambient identifies locally originating probes, whereas Overdrive's adapter accepts arbitrary explicit host targets. [Istio application checks](https://istio.io/latest/docs/ops/configuration/mesh/app-health-check/), [Istio ambient policy interaction](https://istio.io/latest/docs/ambient/usage/networkpolicy/), [Linkerd traffic validation](https://linkerd.io/docs/tasks/validating-your-traffic/).

Linkerd's per-route policy example keeps a dedicated probe route and authorization, including a network authentication policy for the probe route. This illustrates a different boundary: a limited health route can be allowed without granting that authorization to every application route. It does not prove that Overdrive needs a route-aware policy feature. [Linkerd 2.18 per-route authorization](https://linkerd.io/2.18/tasks/configuring-per-route-policy/).

**Confidence:** High for the broad pattern, based on independent Istio and Linkerd behavior. The precise Istio rewrite behavior is directly documented by its authoritative maintainer, rather than cross-validated against an independent implementation of Istio.

### Readiness combines distinct observations; it is not a universal reachability proof

Kubernetes distinguishes startup, liveness, and readiness, with readiness affecting Service endpoints. It also warns against turning recoverable/external failures into liveness restarts. Pod readiness combines container readiness and any declared readiness gates. [Kubernetes probes](https://kubernetes.io/docs/concepts/workloads/pods/probes/), [Kubernetes Pod readiness](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-readiness).

Istio 1.27.0's injection template assigns the proxy its own readiness check on `15021/healthz/ready`; its agent checks initial configuration and Envoy readiness. Those checks complement the rewritten application probe. They do not make a round trip through every peer, every authorization rule, every network route, or every future certificate rotation. [Version-pinned injection template](https://github.com/istio/istio/blob/1.27.0/manifests/charts/istio-control/istio-discovery/files/injection-template.yaml), [Version-pinned proxy readiness implementation](https://github.com/istio/istio/blob/1.27.0/pilot/cmd/pilot-agent/status/ready/probe.go).

Envoy upstream health checks normally use the configured cluster transport socket, including TLS where configured, and support separate health-check transport matching. That is an appropriate comparison for **proxy-to-upstream availability**, which is a different question from whether a local application started. Envoy also documents passive health checking via outlier detection. [Envoy health checking](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/health_checking), [Envoy health-check API](https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/core/v3/health_check.proto).

**Inference for Overdrive:** the current readiness calculation observes application health. Initial interception installation has its own production ordering, while per-connection TLS enforcement can still reject a connection. No direct probe result entails that the mesh handshake, policy, transport, and reply path succeeded. This is an observation-coverage limitation. Proving an actual healthy-backend/unusable-TLS availability defect requires a reachable production cause and reproducible outcome; missing an input from one function alone is not that proof. **Confidence:** High for the distinction; no verdict on prevalence or production reachability of a specific outage.

### The mark is a local privilege and routing mechanism

Linux v6.18 checks `CAP_NET_RAW` **or** `CAP_NET_ADMIN` in the socket network namespace's owning user namespace before allowing `SO_MARK`. The ADR's mention of `CAP_NET_ADMIN` is sufficient, but it is not the only capability that can authorize this operation on that kernel. The numeric mark is not an agent identity or cryptographic secret. [Linux v6.18 socket implementation](https://raw.githubusercontent.com/torvalds/linux/v6.18/net/core/sock.c).

Netfilter describes marks as packet metadata and specifies that `accept` terminates the current base chain, with later base chains/hooks still able to drop the packet. Thus the observed exception skips the Overdrive interception chain; it does not disable all network security. **Inference:** because this is local metadata rather than a TLS credential or IP-header field, setting the mark on host A does not by itself grant an exemption at host B. [nftables manual: metadata and overall ruleset evaluation](https://netfilter.org/projects/nftables/manpage.html).

The existing exemption checks the mark, not the probe's application path, allocation owner, or executable identity. Its legitimate use therefore depends on who can create marked traffic in the relevant namespace **and who may instruct the privileged prober to dial a target**. This assessment has not audited all deployment permissions, workload capabilities, namespace mark-scrubbing behavior, or external firewall configuration. No capability escape, cross-tenant exploit, or all-security bypass is demonstrated. **Confidence:** High for the kernel and nftables facts; deployment-specific trust sufficiency remains unproven.

## Scope-sensitive judgment

| Situation | Judgment |
| --- | --- |
| Trusted host agent checks its allocation's local application address; result means listener/application health | Correct design pattern. The mark restores the intended observation boundary and reuses the existing plaintext application leg. |
| Startup/liveness are application-local; mesh connectivity has separate evidence and failure ownership | Correct separation. A mesh outage need not imply the application should be restarted. |
| Readiness is explicitly the application component of eligibility, with limited mesh availability guarantees understood | Defensible, provided that the documented promise does not overstate what `healthy` establishes. Existing intercept install and connection enforcement remain relevant complementary mechanisms. |
| `Pass` or `healthy` is claimed to prove authenticated mesh requests can succeed | Incomplete evidence/contract. The marked local attempt excludes the path that would establish that promise. No new runtime failure is established by this statement. |
| Explicit host points to another local workload | The shared mark can select the existing exemption for that workload's matching port too. Whether the configuration author is entitled to request that host-agent access is a trust-contract question; it is not bounded to the allocating workload by these adapters. |
| Explicit host points to a remote non-mesh endpoint | The request remains a host-originated plaintext TCP/HTTP probe. No matching local divert means the exemption may make no routing difference in the inspected rules. It measures host-to-target connectivity, not originating-workload connectivity. |
| Explicit host points to a remote strictly meshed workload | A mark on the probing node does not authenticate at the destination node. Its inbound capture may still receive plaintext and reject it; connect-only success can still describe an intermediary. This is a predicted topology-dependent limitation, not a reproduced Overdrive multi-node defect. |
| Probe target is a Service frontend rather than the allocation endpoint | A direct host probe must not be assumed to exercise the workload-originating leg-F resolver. Moreover, a frontend that selects only healthy backends can be circular as the sole test that first makes a backend healthy. This is an alternative-design constraint, not an assertion of a current production deadlock. |

The mark does not resolve target ambiguity. A deliberately configured dependency endpoint can be a legitimate readiness input, but its success establishes the agent's access to that endpoint, which may differ from the application's DNS, network namespace, mesh identity, or policy context.

## Alternatives and their tradeoffs

These are options for a separately authorized contract discussion, not instructions to amend the current design or create public APIs.

| Option | What it proves | Costs and limits |
| --- | --- | --- |
| Retain direct local probes and separately observe mesh traffic or run representative mesh checks | Application health stays diagnosable independently; actual authenticated requests can establish selected mesh paths | Requires choosing the checked peer, identity, route, freshness, and failure owner. A representative path is still not every-client reachability. No automatic reason to move allocation Running or application liveness gates. |
| Agent-originated authenticated probe | Can test target TLS identity, handshake, and an application response on the selected protected route | The platform agent would own credentials and protocol behavior, consistent with identity-unaware workloads. It needs an explicit probing identity/authorization contract and exact approved signature; reusing the target's SVID as caller identity is not an assumed authorization. A successful TCP connect alone is still insufficient. |
| Execute a local check inside the actual workload/guest | Can test local application state without external interception | Needs genuine guest/container execution support and an appropriate endpoint. The existing `CgroupExecProber` spawns a host process and places it in a cgroup (`exec_prober.rs:187–232`); cgroup membership is not guest execution or network-namespace entry. It cannot simply be relabeled as this option. |
| Run an ordinary plaintext client from an actual enrolled workload network context | Can exercise workload egress capture, inter-agent TLS, target application, and return traffic with the platform holding identity material | More expensive and context-specific. Loopback/self-checks alone still omit inter-agent traffic. No need to distribute SVIDs to applications; the agent handles mesh TLS. |
| Exclude a whole application port from mTLS, or make the workload permissive | Allows plaintext callers | Broader exposure than a trusted probe path; not required to resolve the local probe problem and not recommended as the default response. Istio's recommended default remains probe rewrite. [Istio security FAQ](https://istio.io/latest/about/faq/security/) |

## Missing decisions and bounded ways to settle them

1. **Meaning of the health result:** pin whether configured readiness promises application health, a specified dependency check, or authenticated mesh availability. Preserve the separate owners of allocation Running, startup Stable, backend eligibility, and liveness. This research makes no new gate decision.
2. **Destination and requester scope:** pin whether trusted host probes may target any explicit host, only their own local allocation, or designated dependencies. Read existing operator/deployment authorization and allowed workload capabilities before labeling current breadth a security defect. An arbitrary host string alone proves scope, not unauthorized reachability.
3. **Desired mesh coverage:** identify the operator-visible guarantee already required. If existing examples already drive a real VM peer through the Service, retain that evidence as distinct from the direct probe. Do not call a passing health result the same evidence twice.
4. **If a specific availability failure is suspected:** first formulate a bounded safety/liveness/convergence invariant through existing production owners and existing Sim driving ports. Establish a healthy baseline, inject only an already-modelled real cause, retain a failing seed, and observe eligibility and actual request outcome through recovery. Do not seed the desired bad backend row, abort a future that production drains, or invent a seam to manufacture the hypothesis. If the needed boundary is absent, return that testability decision for approval.
5. **If target/path behavior needs confirmation:** a separately scheduled Tier-3/native comparison can cover bound versus unbound guest listeners with capture active, positive versus rejected HTTP responses, a real mesh peer's byte-distinct request/reply, and an explicit remote target if supported. Kernel routing, socket marks, and wire encryption require real-kernel evidence; a Sim result cannot prove them. Keep operator examples/expectations and in-process internal tests at their established boundaries.

No tests or native commands were run for this research. No unproven timing, restart, retry, security, or convergence hypothesis is promoted to a mandatory remediation finding.

## Source ledger and limitations

All cited external sources are primary maintainer documentation or source. Accessed during 2026-09-08–09; pages without an explicit publication date are recorded as undated live documentation. Pinned source versions establish their specific behavior, not the latest release's entire implementation.

| Source | Version/date | Authority and verification |
| --- | --- | --- |
| [Istio application health](https://istio.io/latest/docs/ops/configuration/mesh/app-health-check/) | Undated, live `latest` | Primary; explicit HTTP/TCP problem and default rewrite. |
| [Istio ambient NetworkPolicy](https://istio.io/latest/docs/ambient/usage/networkpolicy/) | Undated, live `latest` | Primary; local-origin probe exception and policy distinction. Same publisher as preceding source, not independent corroboration. |
| [Linkerd traffic validation](https://linkerd.io/docs/tasks/validating-your-traffic/) | Undated, live docs | Primary; independent confirmation of non-mTLS kubelet readiness traffic. |
| [Linkerd per-route policy](https://linkerd.io/2.18/tasks/configuring-per-route-policy/) | 2.18 documentation | Primary; route-specific probe authorization example. |
| [Kubernetes probes](https://kubernetes.io/docs/concepts/workloads/pods/probes/) | Undated, live docs | Primary; lifecycle purpose and endpoint consequences. |
| [Kubernetes Pod lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-readiness) | Undated, live docs | Primary; combined readiness; not independent of Kubernetes probes. |
| [Istio injection template](https://github.com/istio/istio/blob/1.27.0/manifests/charts/istio-control/istio-discovery/files/injection-template.yaml) | 1.27.0 source | Primary; independent proxy readiness assignment in this version. |
| [Istio readiness implementation](https://github.com/istio/istio/blob/1.27.0/pilot/cmd/pilot-agent/status/ready/probe.go) | 1.27.0 source | Primary; config and proxy checks; not a universal peer-path test. |
| [Envoy health checking](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/health_checking) | Undated, live `latest` | Primary; active TLS and passive health checking. |
| [Envoy health-check API](https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/core/v3/health_check.proto) | Undated, live `latest` | Primary; separate health transport matching. Same publisher as preceding source. |
| [Linux socket implementation](https://raw.githubusercontent.com/torvalds/linux/v6.18/net/core/sock.c) | v6.18 source | Primary; exact capability check. GitHub HTML search was insufficient; raw source verified. |
| [nftables manual](https://netfilter.org/projects/nftables/manpage.html) | Undated, live manual | Primary; mark metadata and accept/drop scope. |
| [Istio security FAQ](https://istio.io/latest/about/faq/security/) | Undated, live `latest` | Primary; recommends rewrite; documents broader alternatives. |

The broad health-separation conclusion has independent Istio, Linkerd, Kubernetes, and Envoy support. Product-specific mechanics necessarily rely on product source, and exact kernel semantics on kernel/Netfilter sources. Documentation from different pages of one project is not counted as independent corroboration. No cited source proves the security of this checkout or authorizes a feature change. Current source makes the observation boundary clear; production prevalence, explicit remote-host deployments, complete capability isolation, and the necessity of any additional availability gate remain outside the evidence gathered.

**Recommendation:** retain the socket-mark mechanism as the local application-probe solution, while explicitly deciding the probe destination/trust scope and the meaning of readiness. Treat authenticated mesh availability as a distinct proof obligation when the product contract requires it. Do not revert to unmarked probes, distribute SVIDs to workloads, or expand lifecycle/architecture scope merely because a direct probe does not test the mesh.
