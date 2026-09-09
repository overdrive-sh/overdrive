# ADR-0097: Project default Exec network probes to their allocated address and count each startup observation once

## Status

**Accepted** (2026-09-07). Focused, user-authorized DESIGN amendment for the
native-metal E10 Exec/VM HTTP status matrix. The user directed the correction
below and immediate DELIVER resumption without another DESIGN review.

This amends ADR-0090's Exec default-target row and corrects the observed
startup-attempt accounting defect. It retains ADR-0092 and ADR-0094's existing
marked-socket rules; it does not broaden either socket decision.

## Context

E10's checked-in `http-exec-204.toml` starts `e08-server` as an Exec Service
with an omitted HTTP host. The server binds `0.0.0.0:18080`. In the normal
native-metal composition, the action shim evaluates
`network_assignment_required(Exec, true)`, provisions the allocation netns,
injects its transit `AllocationSpec.workload_addr`, and `ExecDriver` enters
that netns before `execve`.

`ProbeRunner`, in contrast, stays host-originated. Its current
`project_network_probe_target` returns without projection for every non-VM
driver, and `http_probe_host(None)` then selects host `127.0.0.1`. The host
loopback has no listener, so E10's Exec 204 control fails `connection refused`.
The production evidence records the failed deploy as `startup probe[0] failed
after 20 attempts: connection refused`. This is a real target mismatch, not a
fixture problem and not an HTTP status-policy or socket-mark problem.

The same evidence exposes a distinct accounting defect. Hydration reduces the
latest startup `ProbeResultRow` to `ProbeStatus` and
`ServiceLifecycle::reconcile` increments `startup_attempts_per_alloc` on each
reconciliation that sees that unchanged failing status. One stored failed row
is therefore counted repeatedly before the next probe attempt; the configured
`max_attempts = 3` renders as 20. The existing
`ServiceLifecycleView.startup_last_fail_seen_at` map already provides the
persisted input needed to deduplicate a latest LWW observation, but the current
path does not use it.

## Decision

### D1 — Allocation-network defaults use the allocated address for both drivers

At the existing `ProbeRunner::start_alloc(&AllocationSpec)` registration
boundary, project only an omitted HTTP host and the textual IPv4 bind wildcard
`0.0.0.0` as follows:

| Allocation condition | Default/wildcard TCP or HTTP effective destination |
| --- | --- |
| VM | Required provisioned `workload_addr` (the existing guest address rule from ADR-0090) |
| Exec with `Some(workload_addr)` | That allocation's provisioned transit `workload_addr` |
| Exec with `None` | Existing host-loopback normalization (`127.0.0.1`) |

The second row is the precise correction to ADR-0090: an Exec process placed
in an allocation netns must be probed at its address reachable from the host,
not at host loopback. It uses the already-provisioned transit address and adds
no route, namespace entry, registry, or second probe runner.

The declared probe intent remains unchanged. In particular:

- a non-wildcard explicit host, including explicit `127.0.0.1`, is preserved
  verbatim for both drivers;
- only HTTP omission and exact `0.0.0.0` are defaults/wildcards; no IPv6,
  DNS, port, path, or `ProbeMechanic::Exec` interpretation changes; and
- the task-local descriptor clone is projected once at registration and is not
  persisted or written back to `AllocationSpec` or `ProbeResultRow`.

`Vm + None` remains ADR-0090's unreachable registration invariant and gains
no behavior. An Exec allocation without a provisioned network is an existing
host-network case, so its `None` retains the existing loopback behavior rather
than becoming a new error or health result.

The effective non-loopback address is dialed by the already-authorized private
HTTP/TCP adapters. ADR-0092/ADR-0094 require their existing dial-exemption
mark before `connect(2)`, so an allocation-address SYN follows the normal
reachable path instead of self-intercepting. This amendment does not change
the mark, sockets, adapters, nft rules, routes, capabilities, or their result
semantics.

### D2 — Count a startup failure only when its stored LWW observation changes

`ServiceLifecycle` remains the sole owner of startup attempt accounting and of
the existing `StartupProbeFailed` terminal. Its view uses the existing
`startup_attempts_per_alloc` and `startup_last_fail_seen_at` maps; no new view
field, database row, observation schema, store operation, protocol, or
lifecycle state is added.

Hydration must convert the latest Startup/index-0 row's existing
`last_observed_at_unix_ms` into `UnixInstant` with its status before entering
the existing `ServiceAllocFact`. The exact additional fact field is:

```rust
pub latest_startup_probe_observed_at: Option<UnixInstant>
```

It is a transient hydration input only: `ServiceAllocFact` is neither serde nor
persisted. All direct struct literals must supply it as compiler-required
fallout. No constructor, method, port, wire type, or operator-facing API is
added. The pre-existing raw epoch-millisecond representation on
`ProbeResultRow` and the pre-existing view map are tracked for repository-wide
migration in GH #281; this bounded fix introduces no additional raw timestamp
field.

For a latest Startup/index-0 result:

1. `None` leaves both existing view maps unchanged.
2. `Pass` removes this allocation from both existing maps, then the existing
   Stable branch retains its current ownership.
3. A `Fail` with an existing counter but no
   `startup_last_fail_seen_at` entry is a legacy unpaired view: replace that
   counter with `1` and record the current row timestamp. Do not increment the
   legacy value. The next reconcile sees the paired timestamp and follows the
   ordinary rule below.
4. For every other `Fail`, increment `startup_attempts_per_alloc` and record
   its timestamp in `startup_last_fail_seen_at` **only when that timestamp
   differs from the map's last counted timestamp**. The same observed row on
   any number of later reconcile ticks does not increment. A later LWW row has
   a strictly greater timestamp under the existing `ObservationStore` contract
   and is counted once.

The existing terminal predicate remains exactly: observed startup failures
`>= max_attempts`, elapsed startup deadline, and no Pass. Its `attempts`
field is now truthful—the number of distinct stored failed observations since
the last Pass—not the number of reconcile invocations. No retry cadence,
deadline, max-attempt policy, terminal variant, renderer taxonomy, restart
authority, or probe-write behavior changes.

### Lifecycle and dataplane ownership

Neither correction adds or moves a gate. `Running` is still committed by the
action shim and selected driver before `on_alloc_running`; target projection
only changes the destination of a later probe attempt. `ServiceLifecycle`
still owns `Stable`, the existing `StartupProbeFailed` decision, and (under
ADR-0096) the existing backend-health withdrawal. `WorkloadLifecycle` remains
the sole restart owner. The action shim remains the network owner; the worker
remains the host-originated probe and socket owner.

## Consequences and compatibility

- An Exec Service with an allocated network now has its omitted/wildcard
  HTTP/TCP health check reach the listener it launched in that allocation
  network. This corrects an incompatible assumption in ADR-0090 and enables
  all E10 Exec cells.
- Explicit destinations, explicit loopback, unnetworked Exec defaults, VM
  guest projection, HTTP status classification, redirect refusal, bounded-body
  disposal, and socket marking behavior stay unchanged.
- Existing deployments that implicitly relied on an allocation-network Exec
  default probing an unrelated host-loopback listener change to the documented
  "own process" meaning. An explicit host remains available for that prior
  destination choice.
- Startup-failure output changes only where it was overstated: it reports the
  number of distinct failed ProbeResultRows, such as E10's configured three,
  rather than reconciliation passes. No durable migration is needed: ordinary
  reconciliation reuses the existing persisted timestamp map and normalizes a
  legacy unpaired counter to the current failed row's one observation before
  counting later LWW rows.

## Evidence obligations

1. Worker projection coverage proves, for both HTTP and TCP, VM and
   allocation-network Exec default/wildcard projection to their respective
   `workload_addr`, retained host-loopback behavior for Exec `None`, and
   verbatim preservation of explicit hosts (including `127.0.0.1`). It also
   proves stored descriptors remain unchanged. Every transitioned Rust test
   carries its required Contract Shape declaration.
2. Reconciler and production-runtime recovery coverage persists a legacy view
   with `{ attempts: 20, last_fail_seen_at: absent }`, restarts, and hydrates
   its unchanged failing Startup/index-0 row. It proves recovery replaces the
   count with one and records that row timestamp. A Pass then clears both
   existing maps; three strictly later stored failing rows after that restart
   render exactly `attempts: 3` at the unchanged deadline/max-attempt gate.
   Ordinary reconciler coverage also proves an unchanged row does not increment
   and each strictly later stored row increments once. Hydration coverage proves
   the LWW row timestamp is carried alongside its status. No test may invent a
   clock, store, or lifecycle seam.
3. E10 runs all eight existing Exec/VM × 204/302/404/503 built-default-feature
   cells through `overdrive serve` and `overdrive deploy`. Both 204 cells reach
   Stable; each 302/404/503 cell reports its existing numeric status and the
   configured three startup attempts; redirects remain unfollowed; the 503
   sentinel occurs zero times on every existing operator surface.
4. Existing E08/E09/E13 socket-mark and production-path evidence remains the
   proof for the shared marked adapter route. The E10 runner installs no
   listener, mark, route, address, or namespace artifact that production did
   not install.

## Alternatives considered

1. **Keep Exec defaults at host loopback — rejected.** E10 reproduces the
   listener/netns mismatch through the real start, provision, and probe-owner
   path.
2. **Enter the allocation netns for every probe — rejected.** The provisioned
   address is already host-reachable; namespace entry would add task ownership,
   failure modes, and a different execution model without need.
3. **Rewrite an explicit loopback or other host — rejected.** Explicit target
   semantics are a compatibility contract; only omission/wildcard is a default.
4. **Add a new target-origin marker, route, or socket API — rejected.** Existing
   projection and the ADR-0092/0094 marked adapters already supply the exact
   private mechanics.
5. **Infer attempts from elapsed time or reconcile ticks — rejected.** Neither
   is a ProbeResultRow. The existing LWW timestamp is the available observed
   identity.
6. **Add a new persisted counter/row or restart policy — rejected.** The two
   existing view inputs and existing terminal predicate are sufficient.

## References

- ADR-0090, ADR-0092, ADR-0094, ADR-0096
- `crates/overdrive-control-plane/src/action_shim/mod.rs`
- `crates/overdrive-worker/src/{driver.rs,probe_runner/mod.rs}`
- `crates/overdrive-reconcilers/src/service_lifecycle.rs`
- `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md` S-SVM-12, S-SVM-26
- `verification/expectations/E10-vm-service-http-cross-driver-status/`
