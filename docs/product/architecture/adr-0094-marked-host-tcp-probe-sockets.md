# ADR-0094: Use the existing mTLS dial-exemption mark for host TCP probe sockets

## Status

**Proposed** (2026-09-07). The user authorized this bounded DESIGN remediation
after a real native-metal reproduction. Independent DESIGN review is required
before the original DELIVER step `02-03` crafter resumes.

Amends only the TCP-adapter boundary implied by ADR-0090. It does not amend
target projection, probe descriptors or persistence, TCP timeout/error
semantics, HTTP, UDP, rules/routes, lifecycle ownership/states, CLI, protocol,
daemon, or dependencies.

## Context

ADR-0090 correctly projects a VM Service's inferred or explicit TCP target to
its guest `workload_addr`. The worker also owns an existing OUTPUT TPROXY
divert for non-loopback traffic to that workload address. An unmarked
host-originated TCP probe SYN therefore reaches the worker mTLS leg-C listener
instead of the guest. A TCP connect reports success once that listener accepts
the connection, even when the guest listener is unbound.

The real built-product E13 `zero-probes-failure.toml` reproduction established
this path with its deliberately unbound port `18998`: the Service reached
Stable with an inferred TCP `Pass`. Consequently E09's unbound-listener
failure matrix is not trustworthy until it is re-run through the corrected
production adapter.

The OUTPUT chain already first accepts sockets carrying the existing
`overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK`. That is the worker's
agent-owned recursion exemption, not a new mark, nft rule, routing rule, or
mTLS mechanism. ADR-0092 separately authorized this same existing exemption
for private HTTP adapter sockets; it does not authorize TCP by implication.

The reachable owner path is `run_server` -> shared `ProbeRunner` ->
`VmDriver::on_alloc_running` -> `ProbeRunner::start_alloc` -> `probe_tick` ->
`TokioTcpProber`. The target projection and probe/lifecycle owners are already
correct. The missing effect is solely on the TCP adapter socket before its
SYN.

### Upstream rationale — a TCP probe must observe the application listener

[Istio's health-check documentation](https://istio.io/latest/docs/ops/configuration/mesh/app-health-check/)
identifies the same false-positive mechanism: intercepted TCP probes can
succeed because the sidecar is listening even when the application is not.
Istio's default probe rewrite delegates the check to its sidecar agent, which
avoids traffic redirection for TCP. The rewrite also addresses HTTP probes
that cannot authenticate under mesh mTLS.

This supports using the existing exemption for a trusted host agent to
observe its local application's listener. It is a shared design rationale,
not a claim of identical implementation or proof of authenticated mesh
reachability. The decision below still marks every non-loopback candidate
and preserves explicit hosts; this citation does not resolve remote-target
trust or reachability. See the
[path assessment](../../research/host-health-probes-mtls-path-assessment.md)
for those qualifications. Startup, readiness, and liveness retain their
existing owners and meanings.

## Decision

`TokioTcpProber` retains its exact public `TcpProber` implementation:

```rust
async fn probe(
    &self,
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<ProbeOutcome, ProbeFailure>;
```

For every call, its private implementation resolves the unchanged `host` and
`port` through Tokio's existing host-resolution path. For each resolved
candidate it creates the matching IPv4 or IPv6 Tokio `TcpSocket` and:

1. applies the existing `MTLS_LEG_S_DIAL_MARK` with `SO_MARK` **before**
   `connect(2)` when the candidate is non-loopback;
2. leaves a loopback candidate unmarked; and
3. attempts the connection using that socket, preserving the existing
   resolved-candidate ordering and existing timeout/result mapping.

The first successful TCP handshake remains `ProbeOutcome::Pass` and the stream
is dropped immediately. Invalid target, DNS, timeout, connection-refused, and
other I/O errors retain their existing `ProbeFailure`/`ProbeOutcome::Fail`
classification and operator-renderable strings. A mark-install failure is an
ordinary existing I/O connection failure; it creates no new health, allocation,
or lifecycle state and must not be retried outside the current probe attempt.

The mark applies to every non-loopback host-side TCP probe candidate, not only
ones known to originate from VM target projection. The unchanged `TcpProber`
port receives only host, port, and timeout; adding VM-origin metadata, a second
method, descriptor field, or public socket abstraction would be divergent API
surface. The mark does not rewrite the destination and has no effect where no
matching worker divert exists; it selects only the existing first OUTPUT-chain
exemption when such a divert is present.

### Scope boundary

No change is authorized to `TcpProber`, `TokioTcpProber::new`, `ProbeRunner`
target projection, `ProbeDescriptor`, persistence, timeout/error semantics,
HTTP adapter, UDP, nft/routing rules, lifecycle ownership/states, CLI, wire
protocol, daemon, or Cargo dependency/feature/lockfile state. UDP is explicitly
out of scope: a future host-side UDP probe needs its own reproduction and
DESIGN decision.

### Lifecycle Gate Ownership

**Not applicable:** socket marking changes only the transport path of an
already-scheduled TCP attempt. It adds, removes, and moves no gate.

| Signal/state | Owner | Unchanged promise |
| --- | --- | --- |
| Allocation `Running` | action shim + selected driver/Beacon | VM start succeeded and Running was committed before probes run |
| Service `Stable` | `ServiceLifecycle` startup branch | existing startup results alone decide it |
| `Backend.healthy` | `ServiceLifecycle` readiness branch | existing readiness results alone decide it |
| Liveness termination | `ServiceLifecycle` | existing threshold emits only existing `StopAllocation` |
| Restart/finalization | `WorkloadLifecycle` | sole restart authority under the existing unified budget |

An unbound, unmarked, or mark-install-failing guest TCP connection produces
only the existing probe result. It may not delay, revoke, or reinterpret
`Running`; a late success remains subject to existing terminal-wins behavior.

## Alternatives considered

1. **Keep `TcpStream::connect` unchanged — rejected.** Real E13 proved that
   its unmarked SYN self-intercepts on the actual production owner path.
2. **Treat ADR-0092 as TCP authorization — rejected.** HTTP and TCP are
   separate adapters; HTTP's connector decision neither specifies nor proves
   TCP behavior.
3. **Add target-origin metadata or a marked `TcpProber` method — rejected.**
   The existing port deliberately has only host, port, and timeout; a new
   public contract would specialize a private socket effect.
4. **Install a new nft rule, route, bypass, or daemon — rejected.** The
   existing mark exemption supplies the required trusted worker behavior;
   another dataplane mechanism broadens security and routing ownership.
5. **Mark after connect — rejected.** The OUTPUT decision observes the SYN,
   so a post-connect effect cannot stop interception.
6. **Mark UDP too — rejected as out of scope.** No production UDP probe path
   or evidence has established its behavior.

## Consequences

- Positive: inferred and explicit non-loopback TCP probes reach the intended
  guest endpoint instead of reporting a false pass from mTLS leg-C.
- Positive: loopback preserves the existing unmarked local probe behavior.
- Positive: existing public ports, descriptors, error strings, lifecycle, and
  dataplane ownership remain stable.
- Negative: the production TCP adapter now owns the same small Linux
  `SO_MARK` socket effect as its HTTP sibling and therefore relies on the
  worker capability already required to install the corresponding rules.

## Evidence obligations

- Direct production-adapter tests prove a non-loopback TCP socket carries
  exactly `MTLS_LEG_S_DIAL_MARK` before connection and a loopback TCP socket
  remains mark `0`; neither test may introduce a public probe method or a
  test-only production seam.
- Existing TCP adapter acceptance/integration coverage continues to prove the
  exact invalid-target, DNS, timeout, refusal, generic I/O, immediate-drop,
  and error-string contracts.
- The built default-feature E09 runner must repeat all 100 paired native-metal
  trials through `overdrive serve` and `overdrive deploy`, with no retries or
  discarded trials, and show the unbound guest case fails truthfully instead
  of connecting to leg-C.
- The built default-feature E13 runner must prove both inferred cases again:
  the bound guest listener passes and serves its eligible VM peer, while the
  deliberately unbound `18998` listener yields `StartupProbeFailed`, remains
  ineligible, and its peer cannot reach the Service. Neither runner may
  install a mark, route, address, or listener that production omits.

## Delivery handoff

Return to the original DELIVER step `02-03` crafter after independent DESIGN
approval. Make only the private `TokioTcpProber` socket construction change
specified above, add the direct mark/loopback-preservation tests, retain all
existing TCP result semantics, and re-capture E09 and E13 native-metal
evidence. Do not modify the HTTP adapter or extend the work to UDP.

## References

- ADR-0090
- ADR-0092
- `crates/overdrive-worker/src/probe_runner/tcp_prober.rs`
- `crates/overdrive-worker/src/mtls_intercept.rs`
- `crates/overdrive-core/src/dataplane/mtls_mark.rs`
- `verification/expectations/E09-vm-service-tcp-truthfulness-100/`
- `verification/expectations/E13-vm-service-inferred-tcp-startup/`
