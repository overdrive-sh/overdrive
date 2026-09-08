# ADR-0092: Use the existing mTLS dial-exemption mark for host HTTP probe sockets

## Status

**Proposed** (2026-09-06). The user has authorized this bounded DESIGN
amendment after a real native-metal reproduction. Independent DESIGN review is
required before the original DELIVER step `02-01` crafter resumes.

Amends the HTTP-adapter boundary implied by ADR-0090. It does not amend its
target-projection, persisted-intent, or lifecycle decisions.

## Context

ADR-0090 correctly projects an omitted or wildcard VM HTTP target to the
provisioned guest `workload_addr` at `ProbeRunner::start_alloc`. On the real
production path, however, the worker has already installed its existing nft
OUTPUT divert for that address. The divert routes an unmarked host-originated
TCP connection to the mTLS leg-C listener. A plain HTTP health GET sent there
is not that listener's protocol and fails as Hyper `SendRequest`.

The OUTPUT chain already accepts a socket carrying
`MTLS_LEG_S_DIAL_MARK` before any per-workload divert. That is the existing
agent-owned recursion exemption; it is not a new dataplane mark, rule, route,
or mTLS protocol. A direct production GET with that mark reached the same
guest endpoint and returned HTTP 204. E08 then showed the built product's
projected readiness probe returning `last=pass` with the provisional connector
implementation.

The reachable owner path is `run_server` -> shared `ProbeRunner` ->
`VmDriver::on_alloc_running` -> `ProbeRunner::start_alloc` -> `probe_tick` ->
`HyperHttpProber`. The target projection, descriptor, and lifecycle owners are
already correct; the missing effect is solely marking the adapter's socket
before its SYN.

### Upstream rationale — application health has a distinct probe path

[Istio's health-check documentation](https://istio.io/latest/docs/ops/configuration/mesh/app-health-check/)
describes the same boundary: kubelet HTTP probes lack the certificate needed
for mesh mTLS, while TCP interception can report an open port independently
of the application. Istio rewrites these probes to its sidecar agent, which
checks the application; its TCP check explicitly avoids traffic redirection.
Probe rewriting is enabled by default.

This supports the trusted local application-health pattern used here:
the host agent reaches the application's HTTP endpoint through its existing
exemption. It does not imply an identical Istio implementation or prove that
an authenticated mesh request can succeed. This ADR still marks every
non-loopback candidate and preserves explicit hosts; the upstream precedent
does not establish the trust or reachability contract for remote targets.
See the [path assessment](../../research/host-health-probes-mtls-path-assessment.md)
for those scope qualifications. This rationale adds no lifecycle gate or probe
contract change.

## Decision

`HyperHttpProber` may replace only its default `hyper-util` connector with a
private, cloneable connector implementing `tower_service::Service<hyper::Uri>`.
There is no change to either public constructor or port contract:

```rust
pub const fn HyperHttpProber::new() -> Self;
async fn probe(&self, url: &str, timeout: Duration)
    -> Result<ProbeOutcome, ProbeFailure>;
```

For each HTTP URI the private connector must:

1. retain the URI host and port unchanged (port 80 when omitted) and resolve
   them through Tokio's existing host-resolution path;
2. create the matching IPv4 or IPv6 `TcpSocket` for each resolved candidate;
3. set the existing `overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK` with
   `SO_MARK` **before** calling `connect(2)` for every non-loopback candidate;
4. leave loopback candidates unmarked; and
5. return the connected Tokio stream to the existing Hyper client, preserving
   its GET-only request, timeout, status classification, redirect refusal, and
   response-body-discard policy.

If resolution, socket creation, mark installation, or connection fails, the
existing `HttpProber` transport-failure path remains the result owner. The
connector must not retry outside Hyper's existing per-attempt behavior, mutate
probe intent, rewrite an explicit host, create a route/rule, or retain socket
state between probe attempts. A mark-install failure is not a new allocation,
health, or lifecycle state.

The mark is intentionally applied to every non-loopback HTTP probe socket, not
only to an address that happens to be VM-projected. The adapter receives only
the already-resolved URL under the unchanged `HttpProber` port; introducing
target-origin metadata or a second probe method would be new public contract
surface. This use does not change the explicit destination: mark `0x2` merely
selects the existing first OUTPUT-chain exemption when a per-workload divert
matches. For a destination with no such divert, it does not set the existing
TPROXY routing mark and therefore leaves normal routing intact. Exec probes are
not HTTP sockets and are untouched; loopback HTTP remains unmarked, preserving
the existing unprivileged local adapter probe.

### Dependency and manifest boundary

The only dependency change permitted by this ADR is the existing connector
contract crate, `tower-service` **version `0.3.3`**, already present in the
workspace lockfile through Hyper's dependency graph. It must be declared once
in root `Cargo.toml` under `[workspace.dependencies]`, and consumed only as
`tower-service.workspace = true` by `overdrive-worker`. No leaf version pin,
new crate, optional feature, TLS client, proxy, DNS crate, or Cargo.lock
resolution change is authorized.

### Lifecycle Gate Ownership

**Not applicable:** socket marking changes only the transport path of an
already-scheduled HTTP attempt. It adds, removes, and moves no gate.

| Signal/state | Owner | Unchanged promise |
| --- | --- | --- |
| Allocation `Running` | action shim + selected driver/Beacon | VM start succeeded and Running was committed before probes run |
| Service `Stable` | `ServiceLifecycle` startup branch | existing startup results alone decide it |
| `Backend.healthy` | `ServiceLifecycle` readiness branch | existing readiness results alone decide it |
| Liveness termination | `ServiceLifecycle` | existing threshold emits only existing `StopAllocation` |
| Restart/finalization | `WorkloadLifecycle` | sole restart authority under the existing unified budget |

A closed, unmarked, or mark-install-failing guest HTTP connection can produce
only the existing probe outcome; it may not delay, revoke, or reinterpret
`Running`. A late HTTP success remains subject to the existing terminal-wins
behavior. TCP projection and all Exec mechanics are explicitly unaffected.

## Alternatives considered

1. **Keep the default connector — rejected.** Native-metal E08 reproduced its
   self-interception through the real production owner path.
2. **Add VM-origin metadata or a marked `HttpProber` method — rejected.** The
   port accepts only URL plus timeout; changing it would add public API solely
   to specialize a private socket effect.
3. **Add a new nft rule, route, daemon, or mTLS proxy bypass — rejected.** The
   existing mark exemption is exactly the owned kernel contract needed; a new
   dataplane mechanism would broaden scope and alter security ownership.
4. **Mark after connect — rejected.** The OUTPUT decision observes the SYN, so
   a post-connect mark cannot prevent interception.
5. **Use a leaf-pinned connector dependency — rejected.** Repository Cargo
   policy requires workspace-pinned dependencies; `tower-service` already has
   a locked compatible version.

## Consequences

- Positive: projected VM HTTP probes reach their guest endpoint without
  self-interception through mTLS leg-C.
- Positive: declared targets, persisted descriptors, public ports, operator
  output policy, and all lifecycle ownership remain unchanged.
- Negative: the production HTTP adapter becomes responsible for a small Linux
  socket effect and needs `CAP_NET_ADMIN`, already held by the worker that
  installs the corresponding nft rules.
- Negative: the private connector must preserve Hyper's URI-resolution and
  error propagation behavior while applying the mark before connect.

## Evidence obligations

- A production-adapter test proves the private connector leaves loopback
  unmarked and applies the existing mark before a non-loopback `connect(2)`;
  it must not add a public `HttpProber` method or test-only production seam.
- Existing HTTP acceptance/integration coverage continues to prove unchanged
  2xx/3xx/4xx/5xx, no-redirect, bounded-body, explicit-target, and Exec
  behavior.
- The built default-feature E08 journey on qualified native metal drives only
  `overdrive serve` and `overdrive deploy` with its checked-in VM example. It
  must show the projected guest readiness `http GET .../ready` returning 204
  and rendering `last=pass`, while preserving the independent `Running` and
  later lifecycle boundaries. The runner may not install an address, route,
  listener, or mark that production did not install itself.
- A reviewer verifies the manifest boundary exactly: root workspace declaration
  plus worker workspace consumption, no new lockfile resolution and no other
  dependency growth.

## References

- ADR-0090
- `crates/overdrive-worker/src/mtls_intercept.rs` (existing OUTPUT exemption)
- `crates/overdrive-core/src/dataplane/mtls_mark.rs` (mark SSOT)
- `docs/feature/service-kind-vm-workloads/deliver/review-02-01.md`
- `verification/expectations/E08-vm-service-guest-health/evidence/`
