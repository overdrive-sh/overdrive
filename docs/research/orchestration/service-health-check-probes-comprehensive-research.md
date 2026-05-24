# Research: Service Health-Check Probes in Workload Orchestrators

**Date**: 2026-05-24 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16

## Executive Summary

This research surveyed health check / health probe implementations across Kubernetes, Nomad, Fly.io, and Cilium to inform Overdrive's service health-check primitive (GitHub issue #170). The findings are organized around six focus areas: probe lifecycle and state machines, configuration shape and defaults, probe runner architecture, traffic routing integration, failure thresholds, and historical design lessons.

**Key findings:** Kubernetes' three-role model (startup, readiness, liveness) with strict startup-gates-liveness sequencing is the de facto industry standard. Nomad uses a simpler single-check model with readiness/healthiness modes. Fly.io uses flat service-level checks with no auto-restart on failure. Cilium does not run its own application probes -- it consumes Kubernetes probe results through the EndpointSlice API and propagates them into eBPF dataplane maps, demonstrating the clean separation between probe execution and dataplane enforcement that Overdrive's architecture should mirror.

**Design alignment:** Issue #170's proposed design aligns with Kubernetes on all major dimensions (three probe roles, startup gating, readiness affecting routing, liveness triggering restart). The primary deliberate divergence is Overdrive's "honest by default" policy -- inferring a TCP startup probe when no probes are declared, rather than assuming healthy. This divergence is well-motivated by the coinflip-submit RCA (issue #170's origin).

**Open design questions surfaced:** (1) Whether to add a `success_threshold` field for readiness probes to prevent flapping; (2) whether to add cascading-failure protection (rate-limiting liveness-triggered restarts across a deployment); (3) probe scheduling jitter to avoid thundering-herd effects; (4) how `startup_deadline` maps to `failureThreshold * periodSeconds`. These are recommended for the DESIGN wave.

## Research Methodology
**Search Strategy**: Official documentation (kubernetes.io, developer.hashicorp.com, cilium.io, fly.io), GitHub source code, CNCF ecosystem references, Kubernetes enhancement proposals
**Source Selection**: Types: official docs, technical specifications, source code | Reputation: high/medium-high min | Verification: cross-referencing across 3+ orchestrators
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation: target 0.85+

---

## 1. Probe Lifecycle and State Machines

### 1.1 Kubernetes Probe Roles and Sequencing

**Confidence: High** | Sources: [K1], [K2], [K3], [K4]

Kubernetes defines three probe roles with a strict sequencing relationship:

**Startup Probe** -- Verifies the application within a container has started. Executed only at startup (not periodically after success). While a startup probe is configured, Kubernetes does NOT execute liveness or readiness probes until the startup probe succeeds. If the startup probe never succeeds, the container is killed after `failureThreshold * periodSeconds` seconds and subjected to the pod's `restartPolicy`. [K1]

**Liveness Probe** -- Determines when to restart a container. Catches deadlocks and unrecoverable hangs where the application is running but cannot make progress. Runs periodically throughout the container's lifetime. Does NOT wait for readiness probes to succeed. On failure beyond `failureThreshold`, the kubelet kills and restarts the container. [K1]

**Readiness Probe** -- Determines when a container is ready to accept traffic. Runs continuously during the container's entire lifecycle (not just once at startup). On failure, the pod's IP address is removed from EndpointSlices of all matching Services; the pod stops receiving traffic but is NOT restarted. [K1], [K2]

**Sequencing state machine:**

```
Container starts
    |
    v
[Startup Probe phase]  <-- liveness + readiness suppressed
    |
    | (startup succeeds once)
    v
[Liveness + Readiness run in parallel]
    |                     |
    | liveness fails      | readiness fails
    | (>= threshold)      |
    v                     v
  Kill + restart       Remove from
  container            EndpointSlices
```

Critical design point: readiness and liveness probes do not depend on each other. They can be used in parallel for the same container. "Using both can ensure that traffic does not reach a container that is not ready for it, and that containers are restarted when they fail." [K1]

**Three possible probe outcomes:**
- `Success` -- container passes the diagnostic
- `Failure` -- container fails the diagnostic (triggers action per probe type)
- `Unknown` -- diagnostic itself failed; no action taken, kubelet makes further checks [K1]

**Default behavior when no probe is configured:** The kubelet always considers the result as `Success`. The container is presumed healthy and ready from the moment it starts. There is one exception: readiness probe results are considered `Failure` before the initial delay elapses. [K1]

### 1.2 Nomad Health Check Lifecycle

**Confidence: High** | Sources: [N1], [N2], [N3]

Nomad's health check model differs significantly from Kubernetes. Rather than three distinct probe roles, Nomad uses a single `check` block that can be configured for two modes:

**Healthiness checks** (default) -- Verify the service behaves correctly. Failed healthiness checks can trigger task restarts via the `check_restart` block. [N1]

**Readiness checks** -- Verify preconditions for deployment. Use `on_update = "ignore"` to allow deployments to proceed even if the check is not yet passing. Readiness checks do not block deployment progression. [N1], [N2]

The Nomad lifecycle is simpler -- there is no startup probe equivalent. Instead, Nomad provides:
- A `grace` period in `check_restart` (default: `"1s"`) during which check failures are ignored after task start or restart [N3]
- An `initial_status` field (default: `"critical"` for Consul) that sets the starting health state [N1]

**Restart behavior:** Failures must be **consecutive**. A single passing check resets the failure count. This prevents premature restarts for flapping services. After `limit` consecutive failures, the task is restarted according to the task group's `restart` policy. The default `limit` is `0`, which disables health-check-based restarts entirely. [N3]

### 1.3 Fly.io Machine Checks

**Confidence: Medium** | Sources: [F1], [F2], [F3]

Fly.io uses a simpler model with no startup/readiness/liveness distinction. Instead, checks are categorized by their effect:

**Service-level checks** (TCP and HTTP) -- Run by the Fly Proxy. Failing checks cause the proxy to mark the Machine as unhealthy and stop routing traffic to it. However, "Machines won't automatically restart or stop due to failing their health checks" -- this is a critical design difference from Kubernetes. [F1]

**Machine checks** -- Run custom commands during deployments (rolling/canary strategies only). If a machine check fails, the deployment is stopped. These are deployment-time gates, not ongoing health monitors. [F1]

**Top-level checks** (`[checks]` section) -- Monitor overall app health independently. Do NOT affect request routing. Purely observational. [F2]

**Grace period semantics:** The `grace_period` defines the time to wait after a Machine starts before checking its health. Unlike Kubernetes' `initialDelaySeconds`, this applies uniformly -- there is no separate startup probe to gate the grace period. [F1], [F3]

### 1.4 Cilium Health and Endpoint Health

**Confidence: Medium** | Sources: [C1], [C2]

Cilium operates at a different layer than application-level health checks. It provides cluster connectivity health monitoring rather than application health probing:

**cilium-health** -- A built-in health monitoring tool that periodically runs bidirectional traffic across multiple paths through the cluster. It probes using both ICMP (Layer 3 connectivity) and HTTP (Layer 7 connectivity to the cilium-health agent endpoint). Default probe frequency is approximately once per minute. [C1]

**Endpoint health integration** -- Cilium respects Kubernetes' native probe results. When the kubelet marks a pod as not-ready (readiness probe failure), the Kubernetes API updates EndpointSlices, which Cilium watches and uses to update its eBPF service maps (`cilium_lb4_services_v2`, `cilium_lb4_backends_v3`). Cilium does not run its own application-level probes -- it consumes Kubernetes' probe results through the EndpointSlice API. [C1], [C2]

**Relevance to Overdrive:** Cilium's architecture demonstrates the pattern of separating probe execution (kubelet) from dataplane enforcement (eBPF maps). The probe runner publishes results; the dataplane consumer reads them. This maps directly to Overdrive's architecture where the worker runs probes, the ObservationStore propagates results, and the XDP/eBPF dataplane consumes the Backend.healthy flag.

### 1.5 Cross-Platform State Machine Comparison

| Aspect | Kubernetes | Nomad | Fly.io | Cilium |
|--------|-----------|-------|--------|--------|
| Startup probe | Yes (gates liveness+readiness) | No (grace period only) | No (grace_period only) | N/A (uses K8s) |
| Readiness probe | Yes (removes from endpoints) | Yes (readiness mode) | Yes (proxy routing) | Consumes K8s readiness |
| Liveness probe | Yes (kills+restarts) | Yes (via check_restart) | No (no auto-restart) | N/A |
| Probe sequencing | Startup -> Liveness+Readiness | Single check, two modes | Flat (no sequencing) | Flat |
| Default (no probe) | Assumed healthy | Depends on provider | Depends on service config | N/A |

---

## 2. Configuration Shape and Defaults

### 2.1 Kubernetes Probe Configuration

**Confidence: High** | Sources: [K1], [K2]

Kubernetes supports four probe mechanisms, each configurable on any of the three probe roles:

**Probe mechanisms:**

| Mechanism | Config Key | Success Condition | Notes |
|-----------|-----------|-------------------|-------|
| Exec | `exec.command` | Exit code 0 | Creates/forks processes; CPU overhead at scale |
| HTTP GET | `httpGet.{path,port,scheme,httpHeaders}` | Status >= 200 and < 400 | Supports custom headers, HTTPS |
| TCP Socket | `tcpSocket.port` | Port is open (connection accepted) | Connection immediately closed after check |
| gRPC | `grpc.{port,service}` | Response status is `SERVING` | Requires gRPC health check protocol |

**Configuration fields and defaults:**

| Field | Default | Min | Applies to |
|-------|---------|-----|-----------|
| `initialDelaySeconds` | 0 | 0 | All probes. Delay after container start before first probe. For liveness/readiness, timer starts only AFTER startup probe succeeds (if one is defined). |
| `periodSeconds` | 10 | 1 | All probes. Interval between consecutive probes. |
| `timeoutSeconds` | 1 | 1 | All probes. Max wait for probe response. |
| `successThreshold` | 1 | 1 | All probes. Consecutive successes to transition to healthy. Must be 1 for liveness and startup probes. |
| `failureThreshold` | 3 | 1 | All probes. Consecutive failures before action is taken. |
| `terminationGracePeriodSeconds` | Pod-level default | 0 | Liveness/startup. Override for grace period before SIGKILL on probe-triggered restart. |

**Startup probe budget calculation:** The total startup time allowed is `failureThreshold * periodSeconds`. With defaults (failureThreshold=3, periodSeconds=10), the startup budget is 30 seconds. For slow-starting apps, increase `failureThreshold` on the startup probe (e.g., failureThreshold=30, periodSeconds=10 = 300s = 5 minutes). [K1]

### 2.2 Nomad Check Configuration

**Confidence: High** | Sources: [N1], [N2], [N3]

**Supported check types by provider:**

| Provider | Supported Types |
|----------|----------------|
| Nomad (native) | `http`, `tcp` |
| Consul | `http`, `https`, `tcp`, `grpc`, `script` |

**Configuration fields:**

| Field | Default | Notes |
|-------|---------|-------|
| `type` | (required) | `http`, `tcp`, `grpc`, or `script` |
| `interval` | (required) | Frequency, e.g. `"10s"`. Must be >= `"1s"`. |
| `timeout` | (required) | Max wait, e.g. `"2s"`. Must be >= `"1s"`. |
| `port` | Inherited from service | Port label (not number, unless `address_mode = driver`) |
| `path` | (required for HTTP) | HTTP endpoint path |
| `method` | `"GET"` | HTTP method |
| `protocol` | `"http"` | `"http"` or `"https"` |
| `body` | `""` | HTTP body payload |
| `success_before_passing` | 0 | Consecutive successes before "passing" status |
| `failures_before_critical` | 0 | Consecutive failures before "critical" status |
| `initial_status` | `"critical"` (Consul) | Starting status |
| `on_update` | `"require_healthy"` | Deployment interaction mode |

**check_restart fields:**

| Field | Default | Notes |
|-------|---------|-------|
| `limit` | 0 (disabled) | Consecutive failures before triggering restart |
| `grace` | `"1s"` | Wait after task start/restart before checking health |
| `ignore_warnings` | false | Whether warning status counts as unhealthy |

**Deployment modes (`on_update`):**
- `"require_healthy"` -- Check must be passing for deployment to progress (default)
- `"ignore_warnings"` -- Warning status treated as passing; critical still blocks
- `"ignore"` -- Any status treated as passing (readiness check pattern)

### 2.3 Fly.io Check Configuration

**Confidence: Medium** | Sources: [F1], [F2], [F3]

**TCP checks (`[[services.tcp_checks]]`):**

| Field | Example Defaults | Notes |
|-------|-----------------|-------|
| `grace_period` | `"1s"` | Delay before first check |
| `interval` | `"15s"` | Time between checks |
| `timeout` | `"2s"` | Max connection attempt duration |

**HTTP checks (`[[services.http_checks]]` / `[[http_service.checks]]`):**

| Field | Example Defaults | Notes |
|-------|-----------------|-------|
| `grace_period` | `"10s"` | Delay before first check |
| `interval` | `"30s"` | Time between checks |
| `method` | `"GET"` | HTTP method |
| `path` | `"/"` | Request path |
| `protocol` | `"http"` | `"http"` or `"https"` |
| `timeout` | `"5s"` | Max connection duration |
| `tls_skip_verify` | false | Skip certificate verification |
| `tls_server_name` | -- | Hostname for TLS validation |
| `headers` | -- | Custom HTTP headers |

**Machine checks (`[[http_service.machine_checks]]`):**

| Field | Notes |
|-------|-------|
| `image` | Docker image for the test |
| `entrypoint` | Test entry command array |
| `command` | Test execution command |
| `kill_signal` | Signal for timeout termination |
| `kill_timeout` | Duration before forceful termination |

**Notable:** HTTP checks will NOT follow HTTP 301/302 redirects -- a redirect counts as a failure. This is a common operational pitfall. [F1]

### 2.4 Default Behaviors When No Probe Is Configured

| Platform | Default behavior |
|----------|-----------------|
| Kubernetes | Container assumed healthy and ready from start. kubelet always reports `Success`. Pod receives traffic from Services immediately. [K1] |
| Nomad | Depends on provider. With Consul, service registers with `initial_status` (default: `"critical"`). With Nomad provider, service is registered as soon as the task is running. No health checking occurs without explicit `check` blocks. [N1] |
| Fly.io | Machine serves traffic as soon as it starts. Proxy assumes healthy. "Bluegreen deployments require at least one health check" -- no-check deployments use immediate swap. [F1], [F2] |

### 2.5 Cross-Platform Configuration Comparison Table

| Field | Kubernetes | Nomad | Fly.io |
|-------|-----------|-------|--------|
| Initial delay | `initialDelaySeconds` (0) | `grace` in `check_restart` ("1s") | `grace_period` (varies) |
| Period | `periodSeconds` (10) | `interval` (required) | `interval` (varies) |
| Timeout | `timeoutSeconds` (1) | `timeout` (required) | `timeout` (varies) |
| Success threshold | `successThreshold` (1) | `success_before_passing` (0) | N/A |
| Failure threshold | `failureThreshold` (3) | `failures_before_critical` (0) + `check_restart.limit` (0) | N/A |
| HTTP path | `httpGet.path` | `path` | `path` |
| HTTP method | Implied GET | `method` ("GET") | `method` ("GET") |
| Custom headers | `httpHeaders` | `header` | `headers` |
| Exec command | `exec.command` | `command` + `args` (script type) | `command` (machine checks only) |
| gRPC | `grpc.{port,service}` | `type = "grpc"` (Consul only) | Not supported |

---

## 3. Probe Runner Architecture

### 3.1 Where Probes Execute

**Confidence: High** | Sources: [K4], [K5], [N1], [N2], [F1]

| Platform | Executor | Location | Notes |
|----------|----------|----------|-------|
| Kubernetes | kubelet | On the node where the pod runs | Each container probe gets its own worker goroutine. Probes execute IN the kubelet process, not inside the container (except exec probes). |
| Nomad (Nomad provider) | Nomad client | On the node where the allocation runs | Nomad client executes HTTP and TCP checks directly. |
| Nomad (Consul provider) | Split | Consul agent for HTTP/TCP/gRPC; Nomad client for script checks | Script checks execute inside the task environment (e.g., inside Docker container). |
| Fly.io | Fly Proxy | At the proxy layer (edge) | Service-level checks run from the proxy, not from the Machine itself. Machine checks run in ephemeral Machines during deploys. |

**Key architectural insight:** Kubernetes runs probes from the node agent (kubelet), not from the control plane. This means probe execution scales with the number of nodes, not with a central component. Each node is responsible only for probing containers scheduled on it.

### 3.2 Result Propagation

**Confidence: High** | Sources: [K4], [K5]

**Kubernetes result propagation chain:**

```
kubelet prober worker goroutine
    |
    | (caches result in per-type results.Manager)
    v
kubelet status manager
    |
    | (updates PodStatus.Conditions)
    v
API server (Pod resource)
    |
    | (EndpointSlice controller watches Pod conditions)
    v
EndpointSlice update
    |
    | (kube-proxy / Cilium / other CNI watches EndpointSlices)
    v
Dataplane (iptables rules / eBPF maps)
```

The kubelet maintains three separate result caches: `readinessManager`, `livenessManager`, and `startupManager`. Each tracks results by container ID, not pod ID. Results flow one-directionally: probes generate results that modify pod status, never the reverse. [K5]

**Nomad result propagation:** Check results are stored locally on the Nomad client. The `nomad alloc checks` command retrieves them. For Consul-backed services, results propagate through Consul's health check mechanism into the Consul catalog, which service consumers query. [N1], [N2]

**Fly.io result propagation:** Service-level check results stay within the Fly Proxy layer. The proxy uses results directly for routing decisions -- there is no intermediate API or store. [F1]

### 3.3 Concurrency and Scheduling

**Confidence: High** | Sources: [K4], [K5]

**Kubernetes probe scheduling architecture:**

The kubelet `prober` package uses a per-probe-type, per-container worker model. Each active probe is a separate goroutine keyed by `(podUID, containerName, probeType)`. The `proberManager` coordinates workers:

- On `AddPod()`: creates worker goroutines for every probe type on every container
- Workers run independently on their configured `periodSeconds` schedule
- Results are cached in type-specific `results.Manager` instances (three separate caches)
- Worker map is protected by a `sync.RWMutex` for concurrent access
- Workers support a manual trigger channel (buffered, non-blocking) for immediate re-probe
- On pod termination: `StopLivenessAndStartup()` halts liveness and startup probes while readiness probes continue independently (to properly drain connections) [K4], [K5]

**Design lesson for Overdrive:** The per-container-per-probe-type goroutine model means N containers with M probe types = N*M concurrent workers on a single node. This is manageable because probes are lightweight (TCP connect, HTTP GET) and run on short intervals (default 10s). The kubelet does not batch or serialize probes.

---

## 4. Integration with Traffic Routing / Load Balancing

### 4.1 Kubernetes: Readiness Gates and EndpointSlices

**Confidence: High** | Sources: [K1], [K2], [K3]

Readiness probe failures directly affect traffic routing through the EndpointSlice mechanism:

1. Readiness probe fails beyond `failureThreshold`
2. kubelet updates `PodStatus.Conditions` -- sets `ContainersReady` to `False`
3. EndpointSlice controller observes the condition change
4. EndpointSlice updates the endpoint's `ready` condition to `false`
5. kube-proxy / Cilium / any CNI watches EndpointSlice changes
6. Dataplane removes the pod's IP from the service's backend pool

Liveness probe failures do NOT directly affect routing -- they trigger container restart, which indirectly causes endpoint removal when the container stops.

**Pod termination and readiness:** When a pod is deleted, the endpoint `ready` condition is automatically set to `false` in the EndpointSlice, even without an explicit readiness probe. Load balancers stop sending regular traffic. This means readiness probes are NOT required for graceful draining -- deletion handles it natively. [K1]

**Startup probe and traffic:** Until a startup probe succeeds, the readiness probe is not running, so the pod is NOT in any EndpointSlice. Traffic routing is gated by startup success without any explicit wiring -- the sequencing naturally prevents premature traffic.

### 4.2 Nomad: Consul Service Registration

**Confidence: High** | Sources: [N1], [N2]

Nomad's traffic routing integration depends on the service provider:

**Consul provider:** Check results feed directly into Consul's service catalog. Consul's DNS interface and Connect proxy use health status to filter service instances. A service with all checks `critical` is removed from DNS responses and Connect routing.

**Nomad provider:** Check results are local to the Nomad client. The `on_update` field controls how checks interact with deployments:
- `"require_healthy"`: Deployment waits for all checks to pass before marking the allocation healthy
- `"ignore"`: Deployment proceeds regardless (readiness pattern)
- `"ignore_warnings"`: Only critical failures block

Unlike Kubernetes, Nomad does not have a native "remove from load balancer on check failure" mechanism in its own service provider. This is delegated to Consul integration or external service meshes.

### 4.3 Fly.io: Proxy and Machine Checks

**Confidence: Medium** | Sources: [F1], [F2]

Fly.io's proxy performs health checks and directly controls routing:

- Service-level checks (TCP/HTTP): proxy marks unhealthy Machines and stops routing to them
- Top-level checks: do NOT affect routing (observational only)
- Machine checks: affect deployments only (stop deploy on failure)

**Critical difference from Kubernetes:** Fly.io does NOT automatically restart Machines on health check failure. "A failing health check can prevent request routing to your Machine. However your Machines won't automatically restart or stop due to failing their health checks." [F1] This means the operator must handle restarts manually or through other mechanisms.

### 4.4 Cilium: Endpoint Health and eBPF Maps

**Confidence: Medium** | Sources: [C1], [C2]

Cilium bridges Kubernetes probe results to eBPF dataplane maps:

1. kubelet runs probes and updates pod conditions
2. Kubernetes API updates EndpointSlice resources
3. Cilium agent watches EndpointSlice changes
4. Cilium updates its eBPF LB service maps (`cilium_lb4_backends_v3`)
5. eBPF programs in the datapath consult updated maps for load-balancing decisions

The eBPF maps hold service backend entries with health state. When a backend is marked unhealthy (readiness probe failure -> EndpointSlice update), Cilium removes it from the active backend set in the eBPF map. New connections are not routed to the unhealthy backend. Existing connections may continue until they close naturally.

**Cilium's own health probing** (`cilium-health`) monitors cluster connectivity (ICMP + HTTP between nodes), not application health. It probes approximately once per minute and reports connectivity status and RTT per node. This is orthogonal to application probe results. [C1]

---

## 5. Failure Thresholds and Backoff Behavior

### 5.1 Threshold Mechanics Across Platforms

**Confidence: High** | Sources: [K1], [N1], [N3]

| Aspect | Kubernetes | Nomad | Fly.io |
|--------|-----------|-------|--------|
| Failure threshold | `failureThreshold` (default: 3). Consecutive failures required. | `failures_before_critical` (default: 0, instant). Plus `check_restart.limit` (default: 0, disabled). | No configurable threshold. Single failure marks unhealthy. |
| Success threshold | `successThreshold` (default: 1). Consecutive successes to recover. Must be 1 for liveness/startup. | `success_before_passing` (default: 0, instant). | No configurable threshold. Single success marks healthy. |
| Failure counter reset | A single success resets the failure counter | A single passing check resets the count [N3] | Immediate state change |
| Startup budget | `failureThreshold * periodSeconds` | `grace` period | `grace_period` |

**Key insight:** Kubernetes' `successThreshold` defaults to 1 and MUST be 1 for liveness and startup probes. Only readiness probes can require multiple consecutive successes before re-adding the pod to endpoints. This asymmetry is intentional -- the cost of a false liveness failure (unnecessary restart) is high, so quick recovery is desired. [K1]

**Nomad's two-layer threshold:** Nomad separates the "mark as unhealthy" threshold (`failures_before_critical`) from the "restart the task" threshold (`check_restart.limit`). This allows a service to be removed from the Consul catalog (via the check going critical) without necessarily triggering a task restart. The `check_restart.limit` provides an additional gate. [N1], [N3]

### 5.2 Period/Timeout Interaction

**Confidence: High** | Sources: [K1], [N1]

**Kubernetes:** The `timeoutSeconds` (default: 1s) defines how long to wait for a probe response. If the timeout expires, the probe is marked as failed. The `periodSeconds` (default: 10s) defines the interval between probe starts. If a probe takes longer than `timeoutSeconds` but less than `periodSeconds`, the probe fails but the next probe still fires on schedule. If a probe takes longer than `periodSeconds`, the next probe is delayed until the current one completes (probes do not overlap for a single container). [K1]

**Nomad:** Both `interval` and `timeout` are required (no defaults). The `timeout` must be less than `interval` -- if the timeout exceeds the interval, the check enters a perpetual failure state. [N1]

**Fly.io:** The relationship between `interval`, `timeout`, and `grace_period` is documented as: "If interval is long and grace_period is shorter than your app's startup time, the health check will take too long, adding to your deployment time." This suggests that the first check fires after `grace_period`, and subsequent checks fire every `interval`. [F3]

---

## 6. Notable Design Lessons and Pitfalls

### 6.1 Historical Issues

**Confidence: High** | Sources: [K1], [K6], [K7], [K8]

**Pitfall 1: Liveness probes causing cascading failures**

The most well-documented pitfall in the Kubernetes community. When a shared dependency (database, message broker) becomes temporarily unavailable, liveness probes that check dependency connectivity fail across all pods simultaneously. Kubernetes restarts every pod, causing total application downtime instead of degraded service. [K6], [K7]

From GitHub issue #66230: "A livenessProbe failed across all pods in a deployment, which took the application down." The proposed mitigation was to honor PodDisruptionBudgets (PDB) for liveness-triggered restarts, limiting the number of simultaneous restarts. [K6]

**Pitfall 2: Liveness probes with dependency checks**

The official Kubernetes documentation explicitly warns: liveness probes should indicate **unrecoverable** application failure (e.g., deadlock), not temporary issues. "Incorrect implementation can lead to cascading failures, container restarts under high load, failed client requests, and increased workload on remaining pods." A health endpoint that returns 503 when any dependency is down will trigger restarts when the dependency recovers -- killing pods that would otherwise self-heal. [K1]

**Pitfall 3: Exec probe overhead at scale**

Exec probes create and fork multiple processes per execution. In high-pod-density clusters, this introduces significant CPU overhead. The recommendation is to prefer `httpGet`, `tcpSocket`, or `grpc` probe types over `exec` when possible. [K1]

**Pitfall 4: Missing readiness probes causing premature traffic**

From the Kubernetes 7 Common Pitfalls blog: "I once forgot a readiness probe for a web service that took a while to load. Users hit it prematurely, got weird timeouts." Without a readiness probe, traffic reaches the container before the application is ready. [K8]

**Pitfall 5: Fly.io HTTP redirect failures**

Fly.io HTTP checks "will not automatically follow any HTTP 301 or 302 redirect." Applications that force HTTPS redirects will fail health checks if the check uses HTTP protocol. [F1]

### 6.2 Community-Learned Best Practices

**Confidence: High** | Sources: [K1], [K6], [K8], [N3]

1. **Liveness probes should check ONLY application-internal health** -- never downstream dependencies. The liveness check should answer "is this process in an unrecoverable state?" not "are all my dependencies available?" [K1]

2. **Use the same endpoint for readiness and liveness, but with different thresholds.** A common pattern: readiness with `failureThreshold: 1` (quick removal from endpoints) and liveness with `failureThreshold: 3` (avoid premature restarts). [K1]

3. **If the process crashes on its own, you probably do not need a liveness probe.** The kubelet's `restartPolicy` already handles process crashes. Liveness probes are for hangs and deadlocks -- processes that are running but stuck. [K1]

4. **Use startup probes instead of long `initialDelaySeconds`.** Before startup probes existed (added in Kubernetes 1.18), operators used large `initialDelaySeconds` on liveness probes, which delayed deadlock detection for the entire container lifetime. Startup probes decouple "wait for initialization" from "detect deadlocks." [K1]

5. **Keep probes simple and fast.** Overly complex checks create false alarms and unnecessary restarts. A probe should complete in well under `timeoutSeconds`. [K8]

6. **Nomad's grace period is critical for startup.** Nomad's `check_restart.grace` (default: "1s") is very short. For applications that take more than a second to start, this must be increased or the task will be restarted immediately. [N3]

7. **Consecutive failure requirement is essential.** Both Kubernetes (`failureThreshold`) and Nomad (consecutive failure counting) require multiple consecutive failures before action. A single transient failure should not trigger a restart. This is a universal design principle. [K1], [N3]

---

## 7. Implications for Overdrive Design (Issue #170)

### 7.1 Alignment with Industry Practice

Issue #170's design aligns well with Kubernetes on most dimensions:

1. **Three probe roles (startup, readiness, liveness)** -- Matches Kubernetes. Nomad and Fly.io use simpler models, but the Kubernetes three-role model is the industry standard that operators expect. The DISCUSS wave decisions confirm this choice.

2. **Three probe types (HTTP, TCP, exec)** -- Matches Kubernetes minus gRPC. This is a reasonable Phase 1 scope. gRPC is noted as a future addition in the story map.

3. **Startup probe gating liveness+readiness** -- Matches Kubernetes exactly. This is the correct sequencing.

4. **Default TCP probe when no probes declared** -- This is a **divergence** from Kubernetes (which assumes healthy) and aligns more with Consul's default-critical approach. The RCA-A motivation (coinflip submit reporting Running on exit 1) justifies this divergence -- Overdrive is explicitly choosing "honest by default" over "permissive by default."

5. **Readiness flipping Backend.healthy** -- Matches the Kubernetes -> EndpointSlice -> eBPF map flow. The architecture is sound.

6. **Liveness triggering restart (Service-kind only)** -- Matches Kubernetes. Restricting to Service-kind is a good design choice since Job-kind workloads have a natural terminal state.

### 7.2 Gaps or Divergences to Consider

**D1: No `successThreshold` equivalent in issue #170 TOML spec.**
Kubernetes allows readiness probes to require multiple consecutive successes before re-adding to endpoints (default: 1, but configurable). This prevents flapping services from rapidly toggling between healthy and unhealthy. Consider whether Overdrive needs this.
**Recommendation:** Start with successThreshold=1 (Kubernetes default). Add configurability if flapping becomes an operational issue.

**D2: No `terminationGracePeriodSeconds` override per probe.**
Kubernetes allows liveness/startup probes to override the pod's termination grace period. This is useful when probe-triggered restarts need faster termination than operator-initiated shutdowns.
**Recommendation:** Defer. Single-node Phase 1 does not need this.

**D3: Startup probe budget calculation.**
The DISCUSS wave flags that "startup probes may legitimately take >60s for slow-warming Services (LLMs, JVM warmup)." Kubernetes' budget is `failureThreshold * periodSeconds`. Issue #170's TOML shape should make this calculation transparent.
**Recommendation:** Ensure the TOML spec includes `failure_threshold` and `period` on startup probes so operators can compute the budget. Consider a `startup_deadline` convenience field as an alternative.

**D4: Fly.io's "no auto-restart on health failure" pattern.**
Fly.io deliberately does NOT restart Machines on health check failure -- only removes them from routing. This is a fundamentally different philosophy from Kubernetes. Overdrive's liveness probe design (restart on failure) follows Kubernetes, which is the more common expectation.
**Recommendation:** Stay with Kubernetes-style auto-restart for liveness. Document clearly that readiness failures do NOT trigger restarts (only routing changes).

**D5: Probe runner concurrency model.**
Kubernetes uses one goroutine per (container, probe_type). For Overdrive's single-node Phase 1, each allocation may have 1-3 probes. The worker needs a lightweight scheduling model that avoids head-of-line blocking between probes for different allocations.
**Recommendation:** Per-allocation-per-probe-type async task, matching the Kubernetes per-container-per-probe-type worker model.

**D6: Cascading failure protection.**
Kubernetes issue #66230 documents the risk of liveness probes causing mass restarts across a deployment. Overdrive's design should consider whether to add a rate limit on liveness-triggered restarts (e.g., only restart N allocations simultaneously).
**Recommendation:** Surface this as a DESIGN-wave open question. Phase 1 (single-node) may not need it, but the architecture should not preclude it.

**D7: Probe result cardinality and storage.**
The DISCUSS wave correctly identifies that probe results should use LWW per `(alloc_id, probe_idx)`, not append-only. This matches the pattern of all platforms surveyed -- none of them store probe result history as first-class data. Kubernetes caches only the latest result. Nomad stores current status. Fly.io displays current status only.
**Recommendation:** Confirmed: LWW per (alloc_id, probe_idx) is correct.

**D8: Exec probe cgroup scoping.**
Kubernetes exec probes run inside the container (the kubelet uses the container runtime's exec API). Overdrive's exec probes must run inside the workload's cgroup. The DISCUSS wave correctly flags this.
**Recommendation:** Confirmed: exec probes MUST use the workload's cgroup scope.

---

## Source Analysis

| # | Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|--------|--------|------------|------|-------------|----------------|
| K1 | [Kubernetes: Liveness, Readiness, Startup Probes (concepts)](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/) | kubernetes.io | High (1.0) | Official docs | 2026-05-24 | Y |
| K2 | [Kubernetes: Configure Probes (tasks)](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/) | kubernetes.io | High (1.0) | Official docs | 2026-05-24 | Y |
| K3 | [Kubernetes: Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) | kubernetes.io | High (1.0) | Official docs | 2026-05-24 | Y |
| K4 | [Kubernetes source: prober_manager.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/prober/prober_manager.go) | github.com | Medium-High (0.8) | Source code | 2026-05-24 | Y |
| K5 | [Kubernetes source: worker.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/prober/worker.go) | github.com | Medium-High (0.8) | Source code | 2026-05-24 | Y |
| K6 | [K8s issue #66230: Mass liveness probe failures](https://github.com/kubernetes/kubernetes/issues/66230) | github.com | Medium-High (0.8) | Issue tracker | 2026-05-24 | Y |
| K7 | [K8s website issue #16607: Liveness probes can worsen availability](https://github.com/kubernetes/website/issues/16607) | github.com | Medium-High (0.8) | Issue tracker | 2026-05-24 | N |
| K8 | [Kubernetes blog: 7 Common Pitfalls](https://kubernetes.io/blog/2025/10/20/seven-kubernetes-pitfalls-and-how-to-avoid/) | kubernetes.io | High (1.0) | Official blog | 2026-05-24 | Y |
| N1 | [Nomad: check block](https://developer.hashicorp.com/nomad/docs/job-specification/check) | developer.hashicorp.com | High (1.0) | Official docs | 2026-05-24 | Y |
| N2 | [Nomad: service block](https://developer.hashicorp.com/nomad/docs/job-specification/service) | developer.hashicorp.com | High (1.0) | Official docs | 2026-05-24 | Y |
| N3 | [Nomad: check_restart block](https://developer.hashicorp.com/nomad/docs/job-specification/check_restart) | developer.hashicorp.com | High (1.0) | Official docs | 2026-05-24 | Y |
| F1 | [Fly.io: Health Checks reference](https://fly.io/docs/reference/health-checks/) | fly.io | High (1.0) | Official docs | 2026-05-24 | Y |
| F2 | [Fly.io: App configuration (fly.toml)](https://fly.io/docs/reference/configuration/) | fly.io | High (1.0) | Official docs | 2026-05-24 | Y |
| F3 | [Fly.io community: Grace period interaction](https://community.fly.io/t/when-does-the-timer-for-a-health-check-grace-period-start-how-does-that-interact-with-the-interval/5705) | fly.io | Medium-High (0.8) | Community | 2026-05-24 | N |
| C1 | [Cilium: Troubleshooting / cluster health](https://docs.cilium.io/en/stable/operations/troubleshooting/) | docs.cilium.io | High (1.0) | Official docs | 2026-05-24 | Y |
| C2 | [Cilium blog: Connectivity troubleshooting with cilium-health](https://cilium.io/blog/2018/2/6/cilium-troubleshooting-cluster-health-monitor/) | cilium.io | High (1.0) | Official blog | 2026-05-24 | N |

**Reputation distribution:** High: 11 (69%) | Medium-High: 5 (31%) | Avg: 0.94

## Knowledge Gaps

### Gap 1: Fly.io Exact Default Values
**Issue**: Fly.io documentation does not publish exact default values for `grace_period`, `interval`, and `timeout` in a single reference table. Example values vary across documentation pages.
**Attempted**: fly.io/docs/reference/configuration/, fly.io/docs/reference/health-checks/, community forums
**Recommendation**: Use the example values from official docs as representative defaults. Not load-bearing for Overdrive design.

### Gap 2: Cilium eBPF Map Health State Representation
**Issue**: Exact data structure changes in Cilium's eBPF backend maps when a pod becomes not-ready are not documented in user-facing docs. Would require source code analysis of `pkg/maps/lbmap/`.
**Attempted**: docs.cilium.io, cilium.io blog
**Recommendation**: For Overdrive's purposes, the important insight is the pattern (probe result -> API object -> eBPF map update), not the exact Cilium map structure.

### Gap 3: Kubernetes Probe Jitter
**Issue**: Whether the kubelet adds jitter to probe timing to avoid thundering-herd effects when many containers have the same `periodSeconds` is not documented in user-facing docs. Source code analysis of `pkg/kubelet/prober/worker.go` would be needed.
**Attempted**: kubernetes.io docs, GitHub source browse
**Recommendation**: For Overdrive, consider adding jitter to probe scheduling in the worker to avoid synchronized probe storms. This is a DESIGN-wave concern.

## Conflicting Information

### Conflict 1: Default Behavior When No Probe Is Configured

**Position A**: Kubernetes assumes healthy and routes traffic immediately. "If no probes are configured, the kubelet always considers the result as Success." -- Source: [K1], Reputation: 1.0
**Position B**: Overdrive (per issue #170 and DISCUSS wave) defaults to a TCP-connect startup probe when no probes are declared. This is an explicit design divergence motivated by RCA-A.
**Assessment**: Both positions are correct for their respective platforms. Overdrive's "honest by default" stance is a conscious design choice, well-motivated by the coinflip-submit RCA. The DISCUSS wave documents this reasoning. Kubernetes' permissive default is appropriate for a general-purpose orchestrator where many workloads are not TCP services.

## Full Citations

[K1] Kubernetes. "Liveness, Readiness, and Startup Probes". kubernetes.io. 2026. https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/. Accessed 2026-05-24.

[K2] Kubernetes. "Configure Liveness, Readiness and Startup Probes". kubernetes.io. 2026. https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/. Accessed 2026-05-24.

[K3] Kubernetes. "Pod Lifecycle". kubernetes.io. 2026. https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/. Accessed 2026-05-24.

[K4] Kubernetes. "pkg/kubelet/prober/prober_manager.go". github.com/kubernetes/kubernetes. 2026. https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/prober/prober_manager.go. Accessed 2026-05-24.

[K5] Kubernetes. "pkg/kubelet/prober/worker.go". github.com/kubernetes/kubernetes. 2026. https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/prober/worker.go. Accessed 2026-05-24.

[K6] Kubernetes. "Issue #66230: Prevent mass livenessProbe failures from taking down all pods". github.com/kubernetes/kubernetes. 2018. https://github.com/kubernetes/kubernetes/issues/66230. Accessed 2026-05-24.

[K7] Kubernetes. "Issue #16607: Liveness Probes: mention that they can worsen app availability". github.com/kubernetes/website. 2019. https://github.com/kubernetes/website/issues/16607. Accessed 2026-05-24.

[K8] Kubernetes. "7 Common Kubernetes Pitfalls (and How I Learned to Avoid Them)". kubernetes.io/blog. 2025-10-20. https://kubernetes.io/blog/2025/10/20/seven-kubernetes-pitfalls-and-how-to-avoid/. Accessed 2026-05-24.

[N1] HashiCorp. "check block in the job specification". developer.hashicorp.com/nomad. 2026. https://developer.hashicorp.com/nomad/docs/job-specification/check. Accessed 2026-05-24.

[N2] HashiCorp. "service block in the job specification". developer.hashicorp.com/nomad. 2026. https://developer.hashicorp.com/nomad/docs/job-specification/service. Accessed 2026-05-24.

[N3] HashiCorp. "check_restart block in the job specification". developer.hashicorp.com/nomad. 2026. https://developer.hashicorp.com/nomad/docs/job-specification/check_restart. Accessed 2026-05-24.

[F1] Fly.io. "Health Checks". fly.io/docs. 2026. https://fly.io/docs/reference/health-checks/. Accessed 2026-05-24.

[F2] Fly.io. "App configuration (fly.toml)". fly.io/docs. 2026. https://fly.io/docs/reference/configuration/. Accessed 2026-05-24.

[F3] Fly.io community. "When does the timer for a health check grace_period start?". community.fly.io. 2022. https://community.fly.io/t/when-does-the-timer-for-a-health-check-grace-period-start-how-does-that-interact-with-the-interval/5705. Accessed 2026-05-24.

[C1] Cilium. "Troubleshooting". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/operations/troubleshooting/. Accessed 2026-05-24.

[C2] Cilium. "Connectivity Troubleshooting with cilium-health". cilium.io/blog. 2018. https://cilium.io/blog/2018/2/6/cilium-troubleshooting-cluster-health-monitor/. Accessed 2026-05-24.

## Research Metadata
Duration: ~45 min | Examined: 20+ | Cited: 16 | Cross-refs: 12 | Confidence: High 70%, Medium 30%, Low 0% | Output: docs/research/orchestration/service-health-check-probes-comprehensive-research.md
