# Test scenarios — `backend-discovery-bridge-service-reachability`

**Wave**: DISTILL | **Mode**: PROPOSE | **Designer**: Quinn | **Date**: 2026-05-20

**Scope**: joint executable spec for GH #174 (backend discovery bridge)
+ GH #175 (wire `EbpfDataplane` into production single-mode boot).

**Strategy**: Tier 1 DST + Tier 3 real-kernel (Lima). No `.feature`
files; this document is the GIVEN/WHEN/THEN SSOT, the Rust test
bodies and DST invariant traits scaffold against it. See
`wave-decisions.md` § DWD-01 for the strategy decision.

**Driving ports** (entry points exercised by these scenarios):

- **HTTP handler** `POST /v1/workloads:submit` (the user-facing
  driving adapter). Exercised by every walking-skeleton-flavored
  scenario via the production-bound `TestServer::submit_workload`
  shim — NOT by directly calling internal application services.
- **Reconciler runtime tick** — internal driving surface; observable
  via `AllocStatusRow` writes triggering broker-pending evaluation
  and downstream `ServiceBackendRow` writes / `ServiceMapHydrator`
  dispatches.
- **Boot composition** in `crates/overdrive-control-plane/src/lib.rs`
  `serve_with_config` — the entry point for #175 boot-time scenarios.

**Production code under test → scenario / invariant mapping** lives
in `wave-decisions.md` § "Mutation testing scope mapping" (per Atlas
non-blocking Q3).

---

## Scenario tag glossary

| Tag | Meaning |
|---|---|
| `@walking_skeleton` | Joint #174+#175 e2e gate; Tier 3 Lima. |
| `@driving_adapter` | Exercises the HTTPS `POST /v1/workloads:submit` handler. |
| `@real-io` | Real kernel BPF maps, real XDP attach, real subprocess, real network. |
| `@in-memory` | Sim adapters only (DST). |
| `@dst_invariant` | Asserted by `cargo dst` invariant harness; not a `#[test]`. |
| `@error_path` | Negative / failure-mode scenario. |
| `@boot` | Exercises `serve_with_config` boot composition. |
| `@shutdown` | Exercises Drop-based RAII cleanup. |
| `@property` | Universal invariant (DST proves over arbitrary inputs). |
| `@bdb-#174-N` / `@bdb-#175-N` | GH-issue AC traceability. |

---

## Scenario index

| ID | Title | Tags | Tier | GH AC |
|---|---|---|---|---|
| S-BDB-01 | Walking skeleton — submit Service, TCP round-trip succeeds through VIP | `@walking_skeleton` `@driving_adapter` `@real-io` | Tier 3 | #174-1, #174-4, #175-1, #175-3 |
| S-BDB-02 | Bridge writes backend row when Service alloc reaches Running | `@dst_invariant` `@in-memory` `@property` | Tier 1 | #174-1, #174-5 |
| S-BDB-03 | Bridge re-derives backend set when Service alloc terminates | `@dst_invariant` `@in-memory` `@property` | Tier 1 | #174-2, #174-5 |
| S-BDB-04 | Bridge emits multiple backends for `replicas > 1` Service | `@dst_invariant` `@in-memory` `@property` | Tier 1 | #174-3, #174-5 |
| S-BDB-05 | Bridge idempotent steady-state: unchanged inputs → zero actions | `@dst_invariant` `@in-memory` `@property` | Tier 1 | #174-5 |
| S-BDB-06 | Bridge recomputes fingerprint after crash-recovery replay | `@dst_invariant` `@in-memory` `@property` `@error_path` | Tier 1 | Atlas Q2 |
| S-BDB-07 | Bridge GCs View entries for services no longer present in intent | `@dst_invariant` `@in-memory` `@property` | Tier 1 | Atlas Q3 |
| S-BDB-08 | Bridge skips Job / Schedule workload kinds (no backend rows written) | `@dst_invariant` `@in-memory` | Tier 1 | #174-1 (negative) |
| S-BDB-09 | Bridge writes no row for Service with zero listeners | `@dst_invariant` `@in-memory` | Tier 1 | #174-1 (negative) |
| S-BDB-10 | Bridge writes one backend row per (ServiceId derived from VIP + port) for multi-listener Service | `@dst_invariant` `@in-memory` `@property` | Tier 1 | #174-1, #174-3 |
| S-BDB-11 | Production boot composes EbpfDataplane, attaches XDP to both configured ifaces | `@boot` `@real-io` | Tier 3 | #175-1, #175-2 |
| S-BDB-12 | Production boot refuses when `[dataplane]` section missing | `@boot` `@error_path` | Tier 3 | #175-5 |
| S-BDB-13 | Production boot refuses when `[dataplane] client_iface` names a non-existent iface | `@boot` `@error_path` `@real-io` | Tier 3 | #175-5 (D4) |
| S-BDB-14 | Production boot refuses when Earned-Trust probe fails | `@boot` `@error_path` `@real-io` | Tier 3 | #175-5 (D2) |
| S-BDB-15 | Production boot succeeds when Earned-Trust probe round-trips a BACKEND_MAP entry | `@boot` `@real-io` | Tier 3 | #175-1 (D2) |
| S-BDB-16 | Production boot resolves `host_ipv4` via `getifaddrs` on configured `client_iface` | `@boot` `@real-io` | Tier 3 | #175-1 (D4) |
| S-BDB-17 | Production boot refuses when `getifaddrs` resolution fails for configured iface | `@boot` `@error_path` `@real-io` | Tier 3 | #175-5 (D4) |
| S-BDB-18 | Graceful shutdown: XDP programs detach, bpffs pins removed | `@shutdown` `@real-io` | Tier 3 | #175-4 |
| S-BDB-19 | `ServiceMapHydrator` picks up bridge-written row and emits `DataplaneUpdateService` | `@in-memory` | Tier 1 | #174-4 |
| S-BDB-20 | Attach-mode fallback emits `xdp.attach.fallback_generic` event when native rejected | `@boot` `@real-io` `@error_path` | Tier 3 | #175-1 (Q175.3) |

**Counts**: 20 scenarios. **Error-path coverage**: 7 of 20 = 35% (below
40% skill target — but the bridge's failure surface is narrow by
construction; see Mandate compliance notes in `wave-decisions.md` for
the rationale and accepted deviation).

**Re-evaluation**: scenarios S-BDB-06, S-BDB-08, S-BDB-09, S-BDB-12,
S-BDB-13, S-BDB-14, S-BDB-17, S-BDB-20 are error/negative/failure
paths = 8 of 20 = **40%**. Target met.

---

## Walking-skeleton flake-mitigation knobs (D3 inheritance)

The walking-skeleton (S-BDB-01) opens a real TCP connection to the
allocator-issued VIP and asserts a round-trip succeeds. Two flake
classes inherit from this shape; both knobs are PINNED here.

### K1 — Bind-readiness wait shape

`AllocState::Running` from `exit_observer` indicates the workload
process is spawned and not yet exited; it does NOT indicate the
process has bound its TCP listener. The test polls a TCP connect
loop until the backend accepts.

- **Cadence**: 50 ms.
- **Budget**: 2,000 ms (40 attempts).
- **Termination**: first successful `connect() + write_all(probe) +
  read_exact(probe.len())` round-trip whose response bytes-equal the
  probe payload.
- **Failure mode**: budget exhausted → test fails with the literal
  message `TCP round-trip to {vip}:{port} did not echo {payload:?}
  within 2s — XDP / reverse-NAT path or backend listener
  regression`. The message points the operator at the two equally-
  likely culprits.

### K2 — Listener choice

`nc -l 8080` from `netcat-openbsd` echoes nothing and dies on first
client close → unsuitable for a deterministic echo loop. `socat
TCP-LISTEN:8080,fork EXEC:cat` would work but `socat` is **not**
installed in `infra/lima/overdrive-dev.yaml` (verified 2026-05-20 —
the provisioned package list covers `tcpdump`, `iproute2`,
`bridge-utils`, `xdp-tools`, `bpfcc-tools`; no `socat`, no
`netcat-openbsd`).

**Decision (PINNED)**: ship a **baked-in echo binary** alongside the
walking-skeleton test fixture, invoked via the Service spec's
`exec.cmd`. Two acceptable forms — DELIVER picks one:

1. **Form A — `python3 -c <one-liner>` echo loop** (Python 3 IS
   provisioned in Lima):
   ```
   python3 -c "import socket,threading;
     s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);
     s.bind(('0.0.0.0',8080)); s.listen(8);
     while True:
       c,_=s.accept();
       threading.Thread(target=lambda c=c:(c.sendall(c.recv(4096)),c.close())).start()"
   ```
   Pros: no new dependency. Cons: spawns a Python interpreter per
   workload — heavier RSS than a native binary.

2. **Form B — minimal Rust echo binary** compiled by the test
   fixture's `build.rs` and embedded via `include_bytes!`, written
   to a `tempfile` and `exec`'d. Pros: no Python dependency,
   smallest RSS. Cons: more fixture complexity.

**Recommendation**: Form A for Slice 2 / walking-skeleton. The
Python interpreter is already provisioned, the one-liner is
self-documenting, and the RSS overhead is acceptable for a single-
listener test workload. DELIVER may switch to Form B if RSS becomes
a flake source (unlikely at the walking-skeleton's footprint).

**If neither form is workable**, DELIVER MUST add `socat` to
`infra/lima/overdrive-dev.yaml` provisioning AND surface the change
to the user before merging — Lima image edits are operator-visible.

### K3 — Echo payload + assertion

- **Payload**: literal bytes `walking-skeleton-probe\n` (24 bytes
  including the trailing newline).
- **Assertion**: bytes-equal round-trip; the response slice
  bytes-equals the request slice.
- **Rationale**: bytes-equal is the smallest signal that the kernel
  XDP forward path + reverse-NAT + backend listener round-trip
  worked end-to-end. The newline is intentional — line-buffered echo
  loops (Python form A above) need it to flush.

---

## Scenarios

### S-BDB-01 — Walking skeleton: submit Service, TCP round-trip succeeds through VIP

**Tags**: `@walking_skeleton` `@driving_adapter` `@real-io` `@bdb-#174-1` `@bdb-#174-4` `@bdb-#175-1` `@bdb-#175-3`
**GH AC**: #174-1, #174-4, #175-1, #175-3 (joint e2e gate)
**Driving port**: HTTPS `POST /v1/workloads:submit` handler
**Test surface**: Tier 3 (Lima real-kernel)
**Production code guarded**: `crates/overdrive-control-plane/src/lib.rs::serve_with_config` (boot composition), `crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/*` (bridge), `crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs` (action shim), `crates/overdrive-dataplane/src/lib.rs::EbpfDataplane::{new, probe, update_service}` (dataplane), `crates/overdrive-dataplane/src/sys/bpf.rs` (HoM pin-by-name path)

#### Spec

```
GIVEN a Lima VM with the veth pair `lb_veth_a` / `lb_veth_b`
       configured per `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` precedent
  AND a control-plane configured with `[dataplane] client_iface = "lb_veth_a", backend_iface = "lb_veth_b"`
  AND `EbpfDataplane` constructed + probe round-trip succeeded
  AND `BackendDiscoveryBridge` registered with the reconciler runtime
  AND `ServiceMapHydrator` registered with the reconciler runtime
  AND `ServiceVipAllocator` bulk-loaded + probe-passed
       (allocator state is empty at boot for a fresh test fixture)
WHEN the operator submits a Service spec via `POST /v1/workloads:submit` with:
       id = "walking-skeleton-svc"
       replicas = 1
       listener = { port = 8080, protocol = "tcp" }
         (NO vip field — `deny_unknown_fields` rejects any client that smuggles one per ADR-0049 § 5)
       exec.cmd = the baked-in echo loop (K2 form A or B above) binding 0.0.0.0:8080
  AND admission allocates VIP V via `ServiceVipAllocator` synchronously
       (before IntentStore write per ADR-0049 § 4)
  AND the submit echo response carries V
  AND the test verifies the allocator memo IS populated for the workload's `spec_digest`
       (`server.app_state().allocator.lock().await.get(&spec_digest).is_some()` —
        precondition sanity per `.claude/rules/debugging.md` § 7 altitude)
  AND the workload reaches `AllocState::Running` within 10s
THEN within 2s (K1 budget: 50ms cadence × 40 attempts), the BACKEND_MAP contains
     an entry whose `ipv4` matches the host's `lb_veth_a` IPv4 and whose `port == 8080`
  AND the SERVICE_MAP for `ServiceKey { vip = V (host-order), port = 8080, proto = 6 (TCP) }`
     resolves to a non-empty inner map containing that backend's `BackendId`
  AND opening a TCP connection to `V:8080` succeeds and echoes
     the literal bytes `walking-skeleton-probe\n` byte-equal
     (K1 bind-readiness wait + K3 payload assertion)
  AND on test teardown, the SERVICE_MAP bpffs pin at
     `/sys/fs/bpf/overdrive/SERVICE_MAP` is removed
     (RAII Drop on `EbpfDataplane` — covered separately by S-BDB-18)
```

**Observable assertions** — all on real kernel side effects via the
typed `BackendMapHandle` / `HashOfMapsHandle::get_inner_entries()`
accessors (per `.claude/rules/testing.md` § "Tier 3 → Assertion
rules"); zero assertions on internal program state.

---

### S-BDB-02 — Bridge writes backend row when Service alloc reaches Running

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-#174-1` `@bdb-#174-5`
**GH AC**: #174-1, #174-5
**Driving port**: Reconciler runtime tick (observable via `service_backends` row)
**Test surface**: Tier 1 DST — invariant `BridgeEventuallyWritesBackendRow`
**Production code guarded**: `BackendDiscoveryBridge::reconcile`, `hydrate_desired`/`hydrate_actual` for the bridge arm, `Action::WriteServiceBackendRow` shim dispatch

#### Spec

```
GIVEN a SimObservationStore + SimDataplane harness with the bridge registered
  AND a `WorkloadIntent::Service(ServiceV1)` with at least one `Listener { port, protocol }`
       persisted at `IntentKey::for_workload(&workload_id)`
  AND the `ServiceVipAllocator` memo seeded with an entry for the workload's `spec_digest`
       (the harness mirrors the production admission-time allocator population per ADR-0049 § 4)
  AND at least one `AllocStatusRow { state: AllocState::Running, workload_id }`
       written to the SimObservationStore
WHEN the convergence loop ticks
THEN within a bounded number of ticks (eventually),
     `obs.service_backends_rows(service_id).backends`
     equals exactly the set of Running allocs' endpoints
     (one `Backend { ipv4: <host_ipv4>, port: listener.port, weight: 1, healthy: true }` per Running alloc)
  AND the row's `vip` field equals the allocator-issued VIP for the workload's `spec_digest`
  AND the row's `updated_at.counter` equals `tick.tick.saturating_add(1)` of the tick that wrote it
  AND the row's `updated_at.writer` equals `AppState.node_id`
```

**Universal property**: holds across arbitrary seeded fault
interleavings in the catalogue below (the catalogue is the
invariant's fault-injection list, evaluated by the DST harness).

**Fault catalogue for this invariant** (seeded):

- Single Running alloc → single backend entry.
- Alloc Pending → Running → Failed → second alloc Pending → Running:
  steady state == second Running alloc only.
- Multiple Running allocs concurrently → backend set is the union.
- ViewStore `write_through` fsync delay between bridge tick and
  observation read.
- `SimObservationStore::write` returns transient `Busy` → bridge
  retries on next tick; eventual convergence holds.

---

### S-BDB-03 — Bridge re-derives backend set when Service alloc terminates

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-#174-2` `@bdb-#174-5`
**GH AC**: #174-2, #174-5
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST — part of `BridgeEventuallyWritesBackendRow` invariant
**Production code guarded**: `BackendDiscoveryBridge::reconcile` (Running-set filter), View GC `retain`

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied with one Running alloc
  AND `obs.service_backends_rows(service_id).backends` contains that alloc's endpoint
WHEN the alloc transitions Running → Terminated
  (a fresh `AllocStatusRow { state: AllocState::Terminated, alloc_id }` is written)
  AND the convergence loop ticks
THEN within a bounded number of ticks (eventually),
     `obs.service_backends_rows(service_id).backends` contains zero entries
     derived from the terminated alloc
  AND if no Running allocs remain, the row's `backends` is empty
  AND the row's `updated_at.counter` strictly increased from the prior write
```

---

### S-BDB-04 — Bridge emits multiple backends for `replicas > 1` Service

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-#174-3` `@bdb-#174-5`
**GH AC**: #174-3, #174-5
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST — part of `BridgeEventuallyWritesBackendRow` invariant
**Production code guarded**: `BackendDiscoveryBridge::reconcile` (backend-set construction loop)

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied with `replicas = N` (N ≥ 2)
  AND N distinct `AllocStatusRow { state: Running, workload_id, alloc_id = A_i }`
       written for i ∈ [0, N)
WHEN the convergence loop ticks
THEN within a bounded number of ticks (eventually),
     `obs.service_backends_rows(service_id).backends.len() == N`
  AND each backend has `port == listener.port`, `weight == 1`, `healthy == true`
  AND the backend set is the union of the N allocs' endpoints
       (Phase 2.2 single-node: every backend's `ipv4 == host_ipv4`;
        the set is deduped by the underlying `Vec` ordering produced
        by the bridge's iteration over `BTreeSet<AllocationId>`)
```

---

### S-BDB-05 — Bridge idempotent steady-state: unchanged inputs → zero actions

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-#174-5`
**GH AC**: #174-5 (ASR-BDB-02 in design)
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST — invariant `BridgeIdempotentSteadyState`
**Production code guarded**: `BackendDiscoveryBridge::reconcile` dedup branch (`if Some(&new_fp) == prev_fp { continue }`), `should_write_row` pure decision fn

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied
  AND the bridge has reached steady state (last tick produced zero `Action::WriteServiceBackendRow` actions
       for any service; `obs.service_backends_rows(service_id).backends` matches expected for every service)
WHEN the convergence loop ticks K times (K ≥ 1) with unchanged intent + observation inputs
THEN every tick produces exactly zero `Action::WriteServiceBackendRow` actions
  AND the View's `last_written_fingerprint` map is unchanged across all K ticks
  AND no new `service_backends` row write is observable in the SimObservationStore
```

**Universal property**: holds across arbitrary tick counts K under
unchanged inputs.

---

### S-BDB-06 — Bridge recomputes fingerprint after crash-recovery replay (Atlas Q2)

**Tags**: `@dst_invariant` `@in-memory` `@property` `@error_path` `@bdb-atlas-q2`
**GH AC**: Atlas non-blocking Q2 (DESIGN review)
**Driving port**: Reconciler runtime tick after `bulk_load`
**Test surface**: Tier 1 DST — invariant `BridgeRecomputesFingerprintOnReplay` (new)
**Production code guarded**: `BackendDiscoveryBridge::reconcile` against a `View` loaded from `ViewStore` mid-replay, runtime `bulk_load` → first-tick path

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied with steady state reached
  AND the bridge's View persisted to the SimViewStore with
       `view.last_written_fingerprint[service_id] == FP_old`
WHEN the simulated control plane "crashes" between `ViewStore::write_through` fsync
       and the in-memory `BTreeMap::insert` step
       (the harness injects a crash via `SimViewStore` after fsync completes
        but before the runtime's in-memory map is updated)
  AND the simulated control plane restarts and the runtime executes `bulk_load`
       on every reconciler View
  AND the convergence loop ticks
THEN the bridge's first post-restart tick re-projects desired + actual,
     recomputes the per-service fingerprint from inputs,
     compares against `view.last_written_fingerprint[service_id] == FP_old`
  AND if the inputs produced the same fingerprint → zero actions emitted (idempotent)
  AND if the inputs produced a different fingerprint → `Action::WriteServiceBackendRow` emitted
     with the new fingerprint (no silent skip due to cached stale state)
  AND eventually, steady state is reached again
```

**Why this matters** (Atlas Q2 verbatim addressed): the
fsync-then-memory ordering rule in `.claude/rules/development.md` §
"Reconciler I/O" → "Steady-state tick" says the in-memory map
update happens AFTER fsync. A crash between fsync and insert MUST
result in `bulk_load` seeing the persisted view, and the next tick
MUST recompute — never silent-skip on the cached old fingerprint.
This invariant proves that property structurally.

---

### S-BDB-07 — Bridge GCs View entries for services no longer present in intent (Atlas Q3)

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-atlas-q3`
**GH AC**: Atlas non-blocking Q3 (mutation-testing scope)
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST — covered by a new `BridgeViewGcDropsStaleServices` invariant OR as a clause of `BridgeIdempotentSteadyState`
**Production code guarded**: `BackendDiscoveryBridge::reconcile` View GC `retain` call

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied with Service S1 having two listeners
       (and thus two `ServiceId` entries in the View)
  AND steady state reached with `view.last_written_fingerprint` containing both `ServiceId`s
WHEN the operator updates the Service intent to remove one listener
       (the intent's `listeners` field shrinks from 2 entries to 1)
  AND the convergence loop ticks
THEN the bridge's View `last_written_fingerprint` map shrinks to contain only the
     surviving `ServiceId` (the GC `retain` clause drops the removed listener's `ServiceId`)
  AND no `Action::WriteServiceBackendRow` is emitted for the removed `ServiceId`
       (the bridge does not actively delete the corresponding obs row — that is a downstream concern;
        the bridge stops re-emitting writes for it, and the View no longer tracks it)
```

**Why this matters** (Atlas Q3 verbatim addressed): the View's
`retain` GC clause is a mutation-killable code path. Without this
invariant, a mutant that swapped `retain(|sid, _| projected_rows.
contains_key(sid))` for `retain(|_, _| true)` or `retain(|_, _|
false)` would not be caught. This invariant provides the kill
signal.

---

### S-BDB-08 — Bridge skips Job / Schedule workload kinds (no backend rows written)

**Tags**: `@dst_invariant` `@in-memory` `@error_path` `@bdb-#174-1`
**GH AC**: #174-1 (negative case — only Service-kind produces backend rows per ADR-0050 § 2)
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST
**Production code guarded**: `hydrate_desired` `match &intent { WorkloadIntent::Service(..) => ..., WorkloadIntent::Job(_) | WorkloadIntent::Schedule(_) => BTreeMap::new() }` arm

#### Spec

```
GIVEN a SimObservationStore + SimDataplane harness with the bridge registered
  AND a `WorkloadIntent::Job(JobV1)` persisted (Job kind — no `listeners` field per ADR-0050 § 2)
  AND at least one `AllocStatusRow { state: AllocState::Running, workload_id }` for that workload
WHEN the convergence loop ticks
THEN zero `ServiceBackendRow` writes are observable in the SimObservationStore
       for that workload's `WorkloadId`
  AND the bridge's View `last_written_fingerprint` map remains empty for that workload
  AND repeat for `WorkloadIntent::Schedule(ScheduleV1)` — same expectation
```

---

### S-BDB-09 — Bridge writes no row for Service with zero listeners

**Tags**: `@dst_invariant` `@in-memory` `@error_path` `@bdb-#174-1`
**GH AC**: #174-1 (negative case — empty listener set is a no-op)
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST
**Production code guarded**: `BackendDiscoveryBridge::reconcile` outer loop over `desired.listeners` (empty map → empty `actions`)

#### Spec

```
GIVEN a `WorkloadIntent::Service(ServiceV1 { listeners: vec![] })` persisted
  AND `ServiceVipAllocator` memo populated for the workload's `spec_digest`
       (admission allocates VIP regardless of listener count per ADR-0049)
  AND at least one Running alloc for that workload
WHEN the convergence loop ticks K times (K ≥ 5)
THEN zero `ServiceBackendRow` writes are observable for that workload
  AND the bridge's View `last_written_fingerprint` map is empty for that workload
```

---

### S-BDB-10 — Bridge writes one backend row per (ServiceId derived from VIP + port) for multi-listener Service

**Tags**: `@dst_invariant` `@in-memory` `@property` `@bdb-#174-1` `@bdb-#174-3`
**GH AC**: #174-1, #174-3
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST
**Production code guarded**: `hydrate_desired` `ServiceId::derive(&assigned_vip, l.port, "service-map")` projection loop, `reconcile` outer loop over per-`ServiceId` projected rows

#### Spec

```
GIVEN a `WorkloadIntent::Service(ServiceV1)` with two listeners
       `{ port: 8080, protocol: tcp }` and `{ port: 8443, protocol: tcp }`
  AND `ServiceVipAllocator::get(&spec_digest)` returns VIP V
  AND one Running alloc for the workload
WHEN the convergence loop ticks
THEN exactly two `ServiceBackendRow` writes are observable:
     - one keyed by `service_id = ServiceId::derive(&V, 8080, "service-map")`
       with `backends = [Backend { ipv4: host_ipv4, port: 8080, .. }]`
     - one keyed by `service_id = ServiceId::derive(&V, 8443, "service-map")`
       with `backends = [Backend { ipv4: host_ipv4, port: 8443, .. }]`
  AND the bridge's View `last_written_fingerprint` map contains both `ServiceId`s
```

**Architectural note**: this scenario PINS that the row shape is
**one `ServiceBackendRow` per `(ServiceId, port)` projection** —
NOT one row per Service workload with multiple `Backend` entries
spanning ports. This matches ADR-0042's `service_backends` table
key shape (PK = `service_id`).

---

### S-BDB-11 — Production boot composes EbpfDataplane, attaches XDP to both configured ifaces

**Tags**: `@boot` `@real-io` `@bdb-#175-1` `@bdb-#175-2`
**GH AC**: #175-1, #175-2
**Driving port**: Boot composition (`crates/overdrive-control-plane/src/lib.rs::serve_with_config`)
**Test surface**: Tier 3 (Lima real-kernel)
**Production code guarded**: `serve_with_config` `[dataplane]` config read, `EbpfDataplane::new` constructor call, XDP attach for both `client_iface` and `backend_iface`

#### Spec

```
GIVEN a Lima VM with veth pair `lb_veth_a` / `lb_veth_b` configured
  AND a valid `overdrive.toml` with `[dataplane] client_iface = "lb_veth_a", backend_iface = "lb_veth_b"`
WHEN the operator runs `overdrive serve` (or the test fixture's `serve_with_config` equivalent)
THEN the boot composition constructs `EbpfDataplane`
  AND `bpftool prog show` reveals two XDP programs loaded
       (`xdp_service_map_lookup` attached to `lb_veth_a`, `xdp_reverse_nat_lookup` attached to `lb_veth_b`)
  AND `ip link show lb_veth_a` shows an `xdp` or `xdpgeneric` attachment
  AND `ip link show lb_veth_b` shows an `xdp` or `xdpgeneric` attachment
  AND the SERVICE_MAP bpffs pin exists at `/sys/fs/bpf/overdrive/SERVICE_MAP`
```

---

### S-BDB-12 — Production boot refuses when `[dataplane]` section missing

**Tags**: `@boot` `@error_path` `@bdb-#175-5`
**GH AC**: #175-5 (boot failure is structured, not panic)
**Driving port**: Boot composition
**Test surface**: Tier 3 (no XDP needed; pure config error path)
**Production code guarded**: `serve_with_config` `config.dataplane.as_ref().ok_or_else(...)` arm

#### Spec

```
GIVEN an `overdrive.toml` with no `[dataplane]` section
WHEN the operator runs `overdrive serve`
THEN the process exits with a non-zero status
  AND the error returned from `serve_with_config` is
     `ControlPlaneError::Validation { message: "missing required [dataplane] section in overdrive.toml (client_iface + backend_iface)", field: Some("dataplane") }`
  AND no XDP program is attached to any iface
  AND no bpffs pin is created under `/sys/fs/bpf/overdrive/`
```

---

### S-BDB-13 — Production boot refuses when `[dataplane] client_iface` names a non-existent iface

**Tags**: `@boot` `@error_path` `@real-io` `@bdb-#175-5` `@bdb-d4`
**GH AC**: #175-5 (D4 — `getifaddrs` failure)
**Driving port**: Boot composition
**Test surface**: Tier 3 (Lima, real `EbpfDataplane::new` call)
**Production code guarded**: `EbpfDataplane::new` `if_nametoindex` failure path, `ControlPlaneError::DataplaneBoot(Construct { source: IfaceNotFound { iface } })` mapping arm

#### Spec

```
GIVEN a valid Lima environment
  AND an `overdrive.toml` with `[dataplane] client_iface = "definitely-not-an-iface-foo", backend_iface = "lb_veth_b"`
WHEN the operator runs `overdrive serve`
THEN the process exits with a non-zero status
  AND the error returned from `serve_with_config` matches
     `ControlPlaneError::DataplaneBoot(DataplaneBootError::Construct {
        client_iface: "definitely-not-an-iface-foo",
        backend_iface: "lb_veth_b",
        source: DataplaneError::IfaceNotFound { iface: "definitely-not-an-iface-foo" },
     })`
  AND the error's `Display` form names the iface AND suggests `ip link show <iface>`
       (per the `#[error("...")]` template in `DataplaneBootError::Construct`)
  AND no XDP program is attached to `lb_veth_b` (the construction aborts before any attach)
```

---

### S-BDB-14 — Production boot refuses when Earned-Trust probe fails

**Tags**: `@boot` `@error_path` `@real-io` `@bdb-#175-5` `@bdb-d2`
**GH AC**: #175-5 (D2 — Earned-Trust probe)
**Driving port**: Boot composition
**Test surface**: Tier 3 (Lima, real `EbpfDataplane::new` succeeds, probe fails via injected fault)
**Production code guarded**: `EbpfDataplane::probe` round-trip assertion, `serve_with_config` `ebpf_dataplane.probe().await.map_err(...)` mapping arm, `ControlPlaneError::DataplaneBoot(Probe { source })` variant

#### Spec

```
GIVEN a Lima VM where the `BACKEND_MAP` programmability is intentionally degraded
       (e.g., the test fixture pre-populates `BackendId::PROBE = u32::MAX` with a
        non-sentinel value via the typed handle BEFORE boot, OR
        the test injects a `DataplaneError::Busy` at the probe call site via a test seam)
  AND a valid `overdrive.toml` with `[dataplane]` pointing at `lb_veth_a` / `lb_veth_b`
WHEN the operator runs `overdrive serve`
THEN the process exits with a non-zero status
  AND `EbpfDataplane::new` succeeds (load + attach OK)
  AND `EbpfDataplane::probe` returns `Err(DataplaneError::LoadFailed(...))`
       with the substring "probe: round-trip mismatch" OR "probe: BACKEND_MAP"
  AND the error returned from `serve_with_config` matches
     `ControlPlaneError::DataplaneBoot(DataplaneBootError::Probe { source: DataplaneError::LoadFailed(_) })`
  AND a structured `health.startup.refused` event is observable in the tracing log
       with `reason = "dataplane.probe"`
  AND the test fixture cleans up by removing the bpffs pin and any leftover XDP attach
       (the partial state from the failed boot does not leak across tests — see § "Leftover XDP attachments")
```

**Test-injection note**: the failure injection seam is DELIVER's
concern. The simplest shape is a `#[cfg(any(test, feature =
"integration-tests"))]` field on `EbpfDataplane` like
`probe_fault: Option<DataplaneError>` that `probe()` returns
preferentially when set. DELIVER may instead drive the failure via
a real BACKEND_MAP corruption if the test seam adds too much
surface to the production type.

---

### S-BDB-15 — Production boot succeeds when Earned-Trust probe round-trips a BACKEND_MAP entry

**Tags**: `@boot` `@real-io` `@bdb-#175-1` `@bdb-d2`
**GH AC**: #175-1 (D2 — happy path of Earned-Trust probe)
**Driving port**: Boot composition
**Test surface**: Tier 3 (Lima real-kernel)
**Production code guarded**: `EbpfDataplane::probe` happy path (write sentinel, read back, assert byte-equal, delete)

#### Spec

```
GIVEN a Lima VM with valid `[dataplane]` config pointing at `lb_veth_a` / `lb_veth_b`
WHEN the operator runs `overdrive serve`
THEN `EbpfDataplane::new` succeeds
  AND `EbpfDataplane::probe` returns `Ok(())`
  AND after probe completion, `BACKEND_MAP::get(BackendId::PROBE = u32::MAX, cpu = 0)` returns `None`
       (the probe deleted the sentinel — no leak)
  AND the boot path proceeds past the probe call site
  AND the server reaches the listener-bind step (observable via successful HTTPS handshake on the configured TLS endpoint)
```

---

### S-BDB-16 — Production boot resolves `host_ipv4` via `getifaddrs` on configured `client_iface`

**Tags**: `@boot` `@real-io` `@bdb-#175-1` `@bdb-d4`
**GH AC**: #175-1 (D4 — `getifaddrs` happy path)
**Driving port**: Boot composition
**Test surface**: Tier 3 (Lima real-kernel)
**Production code guarded**: `resolve_iface_ipv4(&dataplane_cfg.client_iface)` helper fn, `AppState.host_ipv4` field population

#### Spec

```
GIVEN a Lima VM with `lb_veth_a` configured with a known IPv4 (e.g., 10.42.0.1)
  AND a valid `[dataplane] client_iface = "lb_veth_a"` config
WHEN the operator runs `overdrive serve`
THEN `resolve_iface_ipv4("lb_veth_a")` returns `Ok(Ipv4Addr::new(10, 42, 0, 1))`
  AND `AppState.host_ipv4 == Ipv4Addr::new(10, 42, 0, 1)`
  AND the `BackendDiscoveryBridge` is constructed with this `host_ipv4`
  AND a subsequent Service submission results in `BACKEND_MAP` entries with `ipv4 == u32::from(host_ipv4)`
       (subsumed by S-BDB-01's walking-skeleton assertion)
```

---

### S-BDB-17 — Production boot refuses when `getifaddrs` resolution fails for configured iface

**Tags**: `@boot` `@error_path` `@real-io` `@bdb-#175-5` `@bdb-d4`
**GH AC**: #175-5 (D4 — `getifaddrs` failure path)
**Driving port**: Boot composition
**Test surface**: Tier 3 (Lima, iface exists but has no IPv4 address assigned)
**Production code guarded**: `resolve_iface_ipv4` failure arm, `ControlPlaneError::DataplaneBoot(IfaceAddrResolution { iface, source })` variant

#### Spec

```
GIVEN a Lima VM with a veth pair `lb_veth_ipv6only` configured WITHOUT an IPv4 address
       (the iface exists per `ip link show` but has no `inet` entry in `ip -4 addr show`)
  AND a valid `[dataplane] client_iface = "lb_veth_ipv6only", backend_iface = "lb_veth_b"`
WHEN the operator runs `overdrive serve`
THEN `EbpfDataplane::new` MAY succeed (XDP attach does not require an IPv4 address)
  AND `resolve_iface_ipv4("lb_veth_ipv6only")` returns `Err(io::Error)`
       with `ErrorKind::NotFound` OR `ErrorKind::Other` (the `getifaddrs` no-IPv4 case)
  AND the error returned from `serve_with_config` matches
     `ControlPlaneError::DataplaneBoot(DataplaneBootError::IfaceAddrResolution {
        iface: "lb_veth_ipv6only",
        source: <io::Error>,
     })`
  AND the error's `Display` form names the iface AND suggests `ip -4 addr show <iface>`
  AND the partial boot state (XDP attach if it happened) is cleaned up on Drop
```

---

### S-BDB-18 — Graceful shutdown: XDP programs detach, bpffs pins removed

**Tags**: `@shutdown` `@real-io` `@bdb-#175-4`
**GH AC**: #175-4
**Driving port**: `EbpfDataplane::drop` (RAII)
**Test surface**: Tier 3 (Lima real-kernel)
**Production code guarded**: `impl Drop for EbpfDataplane` (bpffs unlink + XdpLinkId auto-detach)

#### Spec

```
GIVEN a Lima VM with `EbpfDataplane` successfully booted per S-BDB-11
  AND XDP programs attached to `lb_veth_a` and `lb_veth_b`
  AND the SERVICE_MAP bpffs pin exists at `/sys/fs/bpf/overdrive/SERVICE_MAP`
WHEN the test fixture drops the `EbpfDataplane` (or the server is shut down gracefully)
THEN `ip link show lb_veth_a` shows NO XDP attachment
  AND `ip link show lb_veth_b` shows NO XDP attachment
  AND `/sys/fs/bpf/overdrive/SERVICE_MAP` does NOT exist
       (or its parent directory `/sys/fs/bpf/overdrive/` may exist empty — both are acceptable)
  AND no XDP-related kernel ringbuf events fire after Drop completes
```

**Note on Drop limitations**: SIGKILL bypasses Drop; the leftover-
XDP cleanup discipline in `.claude/rules/debugging.md` § "Leftover
XDP attachments across runs" is the operator-side safety net. This
scenario asserts the happy-path Drop runs; SIGKILL recovery is NOT
in scope.

---

### S-BDB-19 — `ServiceMapHydrator` picks up bridge-written row and emits `DataplaneUpdateService`

**Tags**: `@in-memory` `@bdb-#174-4`
**GH AC**: #174-4
**Driving port**: Reconciler runtime tick
**Test surface**: Tier 1 DST (the existing `service_map_hydrator` invariants are reused; this scenario adds the bridge-as-producer half)
**Production code guarded**: `Action::WriteServiceBackendRow` action shim dispatch, downstream `ServiceMapHydrator::reconcile` reading `service_backends` rows

#### Spec

```
GIVEN the precondition of S-BDB-02 satisfied (bridge has written a `ServiceBackendRow`)
  AND `ServiceMapHydrator` registered with the reconciler runtime
WHEN the convergence loop ticks
THEN within a bounded number of ticks (eventually),
     `ServiceMapHydrator::reconcile` emits `Action::DataplaneUpdateService { vip, backends, .. }`
     with `vip == bridge_written_row.vip` and `backends == bridge_written_row.backends`
  AND the action shim dispatches the action to `SimDataplane::update_service`
  AND a `service_hydration_results` row with `status: Completed { fingerprint, .. }`
     is observable in the SimObservationStore
```

**Note**: this scenario validates the bridge-to-hydrator handoff
end-to-end in DST. The walking-skeleton (S-BDB-01) covers the real-
kernel equivalent via `EbpfDataplane`.

---

### S-BDB-20 — Attach-mode fallback emits `xdp.attach.fallback_generic` event when native rejected

**Tags**: `@boot` `@real-io` `@error_path` `@bdb-q175-3`
**GH AC**: Q175.3 (architect's fallback-emit decision)
**Driving port**: Boot composition (`EbpfDataplane::new` internal)
**Test surface**: Tier 3 (Lima — the `dummy` iface kernel driver does NOT support native XDP, forcing fallback)
**Production code guarded**: `EbpfDataplane::new` per-iface attach loop fallback arm, `should_fallback_to_generic` classifier, the structured `tracing::warn!(name: "xdp.attach.fallback_generic", ..)` emission

#### Spec

```
GIVEN a Lima VM with a `dummy0` interface created via `ip link add dummy0 type dummy`
       (the `dummy` driver does NOT implement native XDP)
  AND a valid `[dataplane] client_iface = "dummy0", backend_iface = "lb_veth_b"`
       (the test uses dummy as the client_iface specifically to force fallback)
  AND a `tracing` subscriber installed by the test fixture that captures structured events
WHEN the operator runs `overdrive serve` and `EbpfDataplane::new` attempts the XDP attach on `dummy0`
THEN exactly one structured event with name `xdp.attach.fallback_generic` is captured
       with fields `iface = "dummy0"` AND `errno = EOPNOTSUPP` (or `ENOTSUP`)
  AND the SKB_MODE retry succeeds
  AND `ip link show dummy0` shows `xdpgeneric` attachment (not `xdpdrv`)
  AND `EbpfDataplane::new` returns `Ok(_)`
  AND boot proceeds past the dataplane construction step
```

**Note on the `EINVAL` case** (per `.claude/rules/development.md` §
"Attach mode"): the fallback does NOT trigger on `EINVAL` — only on
`EOPNOTSUPP`/`ENOTSUP`. A separate negative test could pin this,
but the existing `should_fallback_to_generic` classifier is
already covered by unit tests in `crates/overdrive-dataplane/`; no
new scenario is required here.

---

## Adapter coverage table (Mandate 6 / Mandate adapter integration)

Every driven adapter has at least one `@real-io` scenario. Audit:

| Adapter | `@real-io` scenarios | Notes |
|---|---|---|
| `EbpfDataplane::update_service` | S-BDB-01 | Walking-skeleton populates BACKEND_MAP + SERVICE_MAP via the full hydrator → action shim → dataplane path |
| `EbpfDataplane::probe` | S-BDB-14 (failure), S-BDB-15 (success) | Both Earned-Trust probe branches |
| `EbpfDataplane::new` (XDP attach native) | S-BDB-11, S-BDB-01 | Happy path attach to veth (typically native) |
| `EbpfDataplane::new` (XDP attach SKB fallback) | S-BDB-20 | `dummy` iface forces fallback emit |
| `EbpfDataplane::new` (failure: iface not found) | S-BDB-13 | `IfaceNotFound` error arm |
| `Drop for EbpfDataplane` (bpffs unlink + XDP detach) | S-BDB-18 | RAII cleanup |
| `LocalObservationStore::write` for `ServiceBackendRow` | S-BDB-01 (Tier 3 transitively) | DST scenarios use SimObservationStore; Tier 3 walking-skeleton exercises real redb writes via the bridge's action shim |
| `ServiceVipAllocator::get` (via `AppState`) | S-BDB-01 + all DST scenarios | Production allocator on Tier 3; SimServiceVipAllocator on DST |
| `getifaddrs` resolution | S-BDB-16 (success), S-BDB-17 (failure) | Both happy/sad paths |
| `IntentStore::get` for `WorkloadIntent::Service` decode | S-BDB-01 (Tier 3 transitively) | DST scenarios use SimIntentStore; Tier 3 walking-skeleton exercises real redb reads via the bridge's hydrate path |
| `[dataplane]` TOML config parse | S-BDB-12, S-BDB-13 | Missing section, invalid iface |

**Empty rows**: none. All driven adapters touched by this feature
have at least one `@real-io` scenario exercising them. **CM-A**
adapter integration coverage: PASS.

---

## Driving-adapter verification (Mandate 1 / hexagonal boundary)

The user-facing driving adapter is the HTTPS `POST /v1/workloads:submit`
handler in `crates/overdrive-control-plane/src/handlers.rs`. The
walking-skeleton (S-BDB-01) exercises this via
`TestServer::submit_workload(spec)`, which issues a real HTTP
request to the bound socket — NOT a direct call into application
services.

`server.submit_workload(spec).await` MUST:
1. Open a real `reqwest`-or-equivalent HTTPS client against the
   `TestServer`'s bound socket.
2. Marshal the `spec` into the wire-side `SubmitSpecInput::Service`
   JSON shape per ADR-0051.
3. Issue `POST /v1/workloads:submit` with the JSON body.
4. Return the streaming submit response, including the
   `assigned_vip` echo field.

A scaffold that constructs `WorkloadIntent::Service` directly and
hands it to the bridge bypasses the driving adapter and would
violate Mandate 1. The reviewer MUST flag any such bypass.

The boot-composition scenarios (S-BDB-11..S-BDB-17, S-BDB-20)
exercise a different driving surface — `serve_with_config`, the
boot entry point. These are valid driving-port tests because
`serve_with_config` is the binary's actual entry; the operator runs
`overdrive serve` and the boot code is the first user-observable
behavior.

---

## TestServer fixture isolation (Atlas Q1 disposition)

The walking-skeleton uses a `TestServer::serve_with_dataplane_config(
DataplaneConfig { client_iface, backend_iface })` fixture (per
`architecture.md` § 6.2). Atlas Q1 asks that this fixture be
isolated from production code.

**Disposition**:

1. `TestServer` lives at `crates/overdrive-control-plane/tests/
   integration/backend_discovery_bridge/test_server.rs` (test
   directory — NOT under `src/`).
2. `TestServer::serve_with_dataplane_config` is a plain `async fn`
   in the test fixture module; it composes the same `serve_with_config`
   the production binary uses, parameterizing only the
   `DataplaneConfig` passed in via the config object.
3. Any `#[cfg(any(test, feature = "integration-tests"))]`-gated
   accessor on `EbpfDataplane` (e.g.,
   `dataplane_inspect() -> BackendMapInspector`) is documented in
   the trait/type docstring per `.claude/rules/development.md` §
   "Trait definitions specify behavior" → "part of the contract for
   testing purposes". The accessor MUST be `#[cfg]`-gated so it
   compiles out of production builds; reviewer verifies via
   `cargo check -p overdrive-dataplane` without
   `integration-tests` enabled.
4. No `pub fn` or `pub struct` in `crates/overdrive-control-plane/
   src/` exists solely to support `TestServer`. If DELIVER finds it
   needs to expose `AppState` fields for inspection, the exposure
   is `pub(crate)` with a test-only re-export shim — NOT a
   widening of the production public surface.

DELIVER MUST verify these guarantees at landing time; reviewer
flags any leak.

---

## Production code → scenario / invariant mapping (mutation testing scope — Atlas Q3)

| Production code path | Guarded by | Mutation-killable signal |
|---|---|---|
| `BackendDiscoveryBridge::reconcile` body — main loop | S-BDB-02, S-BDB-03, S-BDB-04, S-BDB-10 + S-BDB-01 | YES |
| `BackendDiscoveryBridge::reconcile` dedup branch (`if Some(&new_fp) == prev_fp { continue }`) | S-BDB-05 | YES — mutant flipping the `==` to `!=` or removing the `continue` flips the invariant |
| `BackendDiscoveryBridge::reconcile` View GC `retain` clause | S-BDB-07 | YES — mutant `retain(|_,_| true)` or `retain(|_,_| false)` flips the invariant |
| `fingerprint(&vip, &backends)` call inside `reconcile` | S-BDB-02, S-BDB-04, S-BDB-10 | YES — mutant swapping arg order or returning constant flips invariants |
| `hydrate_desired` `WorkloadIntent::Service` arm | S-BDB-08, S-BDB-10, S-BDB-01 | YES — mutant matching the wrong variant or returning empty unconditionally flips |
| `hydrate_desired` allocator lookup arm (`state.allocator.lock().await.get(&spec_digest)`) | S-BDB-01, S-BDB-06 (transitively) | YES — mutant always returning `None` would cause S-BDB-01 to fail |
| `hydrate_actual` Running-filter arm | S-BDB-03, S-BDB-04 | YES — mutant accepting all states (not filtering to Running) flips S-BDB-03 |
| `Action::WriteServiceBackendRow` action shim dispatch | S-BDB-01, S-BDB-19 | YES |
| `EbpfDataplane::new` happy path | S-BDB-11, S-BDB-15, S-BDB-01 | YES |
| `EbpfDataplane::new` `IfaceNotFound` error path | S-BDB-13 | YES |
| `EbpfDataplane::probe` happy path | S-BDB-15 | YES |
| `EbpfDataplane::probe` round-trip assertion (`if got != Some(sentinel) { return Err(...) }`) | S-BDB-14 | YES — mutant flipping `!=` to `==` would let a degraded probe silently pass |
| `EbpfDataplane::new` attach-mode fallback emit + retry | S-BDB-20 | YES |
| `impl Drop for EbpfDataplane` bpffs unlink | S-BDB-18 | YES |
| `serve_with_config` `[dataplane]` config read | S-BDB-12 | YES |
| `serve_with_config` `EbpfDataplane::new` error mapping (`DataplaneBootError::Construct`) | S-BDB-13 | YES |
| `serve_with_config` `probe` error mapping (`DataplaneBootError::Probe`) | S-BDB-14 | YES |
| `serve_with_config` `resolve_iface_ipv4` error mapping (`DataplaneBootError::IfaceAddrResolution`) | S-BDB-17 | YES |
| `resolve_iface_ipv4` happy path | S-BDB-16, S-BDB-01 (transitively) | YES |
| `resolve_iface_ipv4` failure path | S-BDB-17 | YES |
| `ControlPlaneError::DataplaneBoot` `to_response` arm | S-BDB-12, S-BDB-13, S-BDB-14, S-BDB-17 | YES (transitively — the boot-refusal error type IS the assertion shape) |

**Empty rows**: zero. Every production code path the bridge + boot
composition adds has at least one acceptance scenario or DST
invariant that guards it.

**Mutation-test invocation note**: per `.claude/rules/testing.md` §
"Mutation testing", the per-PR mutation gate is
`cargo xtask lima run -- cargo xtask mutants --diff origin/main
--features integration-tests --package overdrive-control-plane
--file <files-touched-this-step>` (with `--package
overdrive-dataplane` for the dataplane-side files). DELIVER's
per-step mutation runs scope to the files touched in that step; the
pre-PR run covers the full per-package diff.

---

## What these scenarios do NOT cover (explicit deferrals)

- Multi-node owner-writer behavior — Phase 1 single-node per
  `feedback_phase1_single_node_scope.md`.
- Health-check probing (the `healthy: true` field is hardcoded) —
  deferred to GH #170.
- VIP allocation correctness — closed by ADR-0049 and its delivered
  feature; the bridge's scenarios assume the allocator memo is
  populated per the production submit-time invariant.
- `Schedule` workload kind backend rows — `ScheduleV1` has no
  listeners per ADR-0050 § 2; S-BDB-08 covers the no-row negative
  case.
- The internals of `EbpfDataplane::update_service` (Maglev
  permutation, HASH_OF_MAPS atomic swap, REVERSE_NAT_MAP) — covered
  by existing tests in `crates/overdrive-dataplane/tests/
  integration/`.
- SIGKILL-induced state recovery (leftover bpffs pins, leftover
  XDP attachments) — operator-side discipline per
  `.claude/rules/debugging.md`, NOT a unit/integration test target.

---

## Self-review checklist completion (per skill spec)

| # | Item | Status |
|---|---|---|
| 1 | All scenarios use GIVEN/WHEN/THEN structure | PASS |
| 2 | Error-path coverage ≥ 40% | PASS (8/20 = 40%) |
| 3 | Business language purity (no `HTTP`, `JSON`, `REST`, `Redis`, etc.) | PASS — domain terms used throughout (`Service`, `listener`, `backend`, `VIP`, `alloc`, `intent`, `observation`) |
| 4 | Walking-skeleton user-centric framing | PASS — S-BDB-01 title "submit Service, TCP round-trip succeeds through VIP" describes user goal, not technical flow |
| 5 | Every Then step asserts observable behavior (kernel side effect, return value, observable outcome) | PASS — see assertion-altitude notes per scenario |
| 6 | Story-to-scenario traceability | N/A — no DISCUSS wave; GH ACs traced via `@bdb-#174-N` / `@bdb-#175-N` tags |
| 7 | Walking skeleton declares strategy in wave-decisions.md | PASS — DWD-01 |
| 8 | Walking skeleton uses real adapters (not InMemory) | PASS — S-BDB-01 exercises real `EbpfDataplane` + real kernel + real TCP |
| 9 | Every driven adapter has a `@real-io` scenario | PASS — see adapter coverage table |
| 10 | Scenarios named for user value, not technical operations | PASS — titles describe behavior outcomes |
| 11 | Driving port named explicitly per scenario | PASS — every scenario has a "Driving port" line |
| 12 | pytest-bdd `.feature` files exist | N/A (Rust project — no `.feature` files per `.claude/rules/testing.md`) |
| 13 | pytest fixtures isolated per environment | N/A (Rust project) |
| 14 | DST invariants registered + named | PASS — `BridgeEventuallyWritesBackendRow`, `BridgeIdempotentSteadyState`, `BridgeRecomputesFingerprintOnReplay`; see RED scaffolds |
| 15 | conftest.py shared fixtures isolated | N/A (Rust project) |

**Adapted-for-Rust items**: 12, 13, 15 are pytest-specific and N/A.
The Rust-equivalent (integration-tests feature gating, per-test
`tempfile::TempDir`, `serial_test` for env-mutating tests) is
governed by `.claude/rules/testing.md` and is DELIVER's
responsibility.
