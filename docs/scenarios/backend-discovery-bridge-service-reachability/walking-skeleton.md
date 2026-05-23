# Walking skeleton — `backend-discovery-bridge-service-reachability`

**Wave**: DISTILL | **Designer**: Quinn | **Date**: 2026-05-20

**Scope**: joint #174 + #175 e2e gate. One Tier 3 test, one user
goal, one demo-able outcome.

---

## User-centric framing

**Goal as the operator sees it**:

> "I submit a Service workload with a TCP listener; I get back an
> allocator-issued VIP; I can open a TCP connection to that VIP and
> talk to my workload. The platform routed me through the kernel
> XDP load-balancer to the backend process I just deployed."

This sentence is the litmus test for the walking-skeleton: a
non-technical stakeholder can confirm "yes, that is what users
need." Technical phrases like "the bridge writes a row" or "the
hydrator dispatches an action" do NOT appear in the user
framing — they are implementation details the test asserts on
because the kernel-side state IS the only externally observable
side effect of the convergence loop.

---

## What the walking skeleton proves

In order, from outermost to innermost:

1. **The operator can submit a Service spec.** `POST /v1/workloads:submit`
   accepts the spec, validates, allocates a VIP, persists the intent.
2. **The platform spawns the workload process.** `WorkloadLifecycle`
   reconciler emits `StartAllocation`, the action shim spawns the
   process via `ExecDriver`, the exit observer writes
   `AllocStatusRow { state: Running }`.
3. **The backend discovery bridge produces a backend row.** Watching
   `AllocStatusRow` changes for Service workloads, it derives the
   backend set and writes `ServiceBackendRow` (the entirety of #174's
   surface).
4. **The service-map hydrator picks it up.** Reads the
   `service_backends` row, emits `Action::DataplaneUpdateService`
   with the allocator-issued VIP + the backend set.
5. **The real `EbpfDataplane` programs the kernel.** The action shim
   calls `EbpfDataplane::update_service`; BACKEND_MAP gets the
   backend entry, SERVICE_MAP's inner map for the VIP+port resolves
   to that backend's `BackendId` (the entirety of #175's surface).
6. **The kernel forwards traffic.** A real TCP connection to
   `<assigned_vip>:<port>` traverses the XDP `xdp_service_map_lookup`
   program → reverse-NAT'd → delivered to the backend process →
   echoed back through the reverse path → received by the test
   client byte-equal.

Steps 1–2 are pre-existing infrastructure. Steps 3–5 are the new
surface. Step 6 is the user-visible reachability that ASR-2.2-04
demands.

---

## Strategy

**Tier 1 DST + Tier 3 real-kernel.** Per `wave-decisions.md` § DWD-01.

- **Tier 1 (DST)**: pure-Rust under `SimDataplane` +
  `SimObservationStore` + `SimServiceVipAllocator`. The bridge's
  reconcile logic is exercised over arbitrary alloc transitions and
  fault interleavings. Two named invariants
  (`BridgeEventuallyWritesBackendRow`, `BridgeIdempotentSteadyState`)
  plus one Atlas-Q2 invariant (`BridgeRecomputesFingerprintOnReplay`).
- **Tier 3 (Lima real-kernel)**: one walking-skeleton test that
  exercises the entire chain through real kernel BPF maps, real XDP
  attach, real TCP round-trip.

**No fakes in the e2e gate**. Sim adapters live only inside the DST
harness for fault-injection coverage. The Tier 3 walking-skeleton
exercises the production-bound `EbpfDataplane`, the
production-bound `PersistentServiceVipAllocator`, the production-
bound `LocalObservationStore`, and the production-bound
`LocalStore`.

---

## Walking-skeleton scenario

**S-BDB-01** — see `test-scenarios.md` for the full
GIVEN/WHEN/THEN spec.

**RED scaffold**: `crates/overdrive-control-plane/tests/integration/
backend_discovery_bridge/walking_skeleton.rs`.

**Convention**: `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]`
+ `#[should_panic(expected = "RED scaffold")]` body until DELIVER
lands the real test. Per `.claude/rules/testing.md` § "RED
scaffolds and intentionally-failing commits", this is the only
sanctioned RED test shape; bare `panic!()` and `#[ignore]` for
"production code doesn't exist yet" are forbidden.

---

## Pre-condition fixture requirements (DELIVER concerns)

DELIVER's first task in Slice 2 (closes #175) is to build the
fixture infrastructure that S-BDB-01 needs:

1. **Lima veth pair setup**: `lb_veth_a` / `lb_veth_b` configured
   per the existing `crates/overdrive-dataplane/tests/integration/
   atomic_swap.rs` precedent. The fixture's `LimaFixture::
   setup_veth_pair()` async fn returns the iface names + their
   IPv4 addresses.
2. **`TestServer` fixture**: lives in
   `crates/overdrive-control-plane/tests/integration/
   backend_discovery_bridge/test_server.rs`. Wraps
   `serve_with_config` with a `DataplaneConfig` parameter; binds
   to an OS-assigned port (`127.0.0.1:0`); exposes
   `submit_workload(spec) -> StreamingSubmitResponse` (real HTTPS
   client against the bound socket); exposes
   `read_workload_intent(name) -> WorkloadIntent` (`pub(crate)`
   accessor; NOT production surface).
3. **`dataplane_inspect()` accessor**: `#[cfg(any(test, feature =
   "integration-tests"))]`-gated method on `EbpfDataplane`
   returning a `BackendMapInspector` and `ServiceMapInspector` that
   wrap the typed `BackendMapHandle` / `HashOfMapsHandle`. NOT
   compiled into production builds.
4. **Echo listener exec command**: per K2 in
   `test-scenarios.md`. Form A (Python one-liner) recommended.
5. **`poll_until` test helper**: 50 ms cadence, 2 s budget; returns
   `Option<T>` on first `Some(_)` or `None` on budget exhaustion.
   May reuse an existing helper (`crates/overdrive-control-plane/
   tests/integration/workload_lifecycle/wait.rs` has the precedent
   shape).

These fixture pieces are NOT production code; they live entirely
under `tests/integration/` and the `integration-tests` feature
gate.

---

## Demo script

When the walking-skeleton goes green, the demo to a stakeholder is:

```
# Operator-side (the user)
$ overdrive workload submit --kind service \
    --id walking-skeleton-svc --replicas 1 \
    --listener tcp/8080 \
    --exec "python3 -c 'import socket, threading; ...'"
✓ Workload accepted. Assigned VIP: 10.42.0.5.

# Operator-side (verify reachability)
$ echo "walking-skeleton-probe" | nc 10.42.0.5 8080
walking-skeleton-probe
```

The first `nc` echo back IS the demo. Everything between
"submitted" and "echoed back" is platform internals — and the
walking-skeleton's CI gate proves the platform delivers that
end-to-end behavior.
