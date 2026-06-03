# ADR-0052 — Backend discovery bridge reconciler + `EbpfDataplane` production single-mode boot

## Status

Accepted. 2026-05-13 (revised 2026-05-20 for ADR-0049 / ADR-0050 / ADR-0051
landing). Decision-makers: Morgan (proposing); pending user
ratification. Tags: phase-2, reconciler, dataplane-port, application-arch,
production-boot, j-plat-004.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-swap
primitive), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService` + `service_hydration_results`),
ADR-0045 (`bpf_redirect_neigh` datapath), ADR-0046 (collision-free
BackendId allocator), ADR-0047 (workload kind discriminator + Listener
spec), ADR-0048 (rkyv versioned envelope).

**Relates to**:

- **ADR-0049** (platform-issued Service VIP allocator) — the bridge
  consults `ServiceVipAllocator::get(&spec_digest)` to resolve the
  allocator-issued VIP for each Service; the operator never names a
  VIP, on the wire or in TOML.
- **ADR-0050** (intent-side workload aggregate) — intent reads are
  decoded as `WorkloadIntent::Service(ServiceV1)`; the bridge's
  `desired` projection sources `ServiceV1.listeners` (`(port,
  protocol)` only — no `vip` field on the listener).
- **ADR-0051** (wire-side `SubmitSpecInput`) — the wire layer the
  bridge never directly observes; admission decodes
  `SubmitSpecInput::Service(ServiceSpecInput)` into
  `WorkloadIntent::Service(ServiceV1)` before the bridge ever runs.

**Tracks**: GH #174 (backend discovery bridge) + GH #175 (wire
`EbpfDataplane` into production single-mode boot).

## Context

Phase 2.2 closed three end-to-end primitives:

1. The kernel-side XDP programs (`xdp_service_map_lookup` +
   `xdp_reverse_nat_lookup`) and BPF map shapes (`SERVICE_MAP` /
   `BACKEND_MAP` / `MAGLEV_MAP` / `REVERSE_NAT_MAP`) — ADR-0040 +
   ADR-0045.
2. The userspace `EbpfDataplane` adapter (loader, typed map handles,
   Maglev permutation, HoM atomic-swap, attach-mode fallback) —
   ADR-0040 + ADR-0046.
3. The `ServiceMapHydrator` reconciler that watches `service_backends`
   rows + `service_hydration_results` rows and emits
   `Action::DataplaneUpdateService` — ADR-0042.

But the intent-to-packet pipeline is **not actually wired end-to-end
in production today**. Two gaps:

- **No production code path writes `ServiceBackendRow`.** The row
  type exists, the `service_backends` redb table exists, the LWW
  semantics are pinned by ADR-0042 § 4, but only test fixtures call
  `ObservationStore::write(ObservationRow::ServiceBackend(...))`. The
  `ServiceMapHydrator` reads an empty row stream in production and
  emits zero actions. The dataplane is correct but unreached.

- **Production single-mode boot threads `NoopDataplane`.**
  `crates/overdrive-control-plane/src/lib.rs:652-658` explicitly wires
  `Arc::new(overdrive_host::NoopDataplane)` with a comment promising
  a later slice will swap in `EbpfDataplane`. Every `update_service`
  call returns `Ok(())` and programs no maps.

Both gaps are jointly load-bearing for J-PLAT-004 closure (the first
non-trivial reconciler against a real Dataplane port body — ADR-0042
§ Context). #175's value (real BPF map programming) is unobservable
without #174 (the bridge that produces the rows the hydrator reads).

Three architectural questions need settling:

1. **What component owns the bridge from `(Service spec, Running
   allocs)` to `ServiceBackendRow`?** A new reconciler, an extension
   of `WorkloadLifecycle`, or a side-effect of an existing action-shim
   path?

2. **How does production single-mode boot wire `EbpfDataplane`
   safely?** Operator configuration source, typed boot-error variant,
   shutdown sequencing, BPF object path resolution, attach-mode
   fallback emit location.

3. **How is the joint feature acceptance-gated?** One walking-skeleton
   end-to-end test or two separate per-component tests?

These extend the substrate from:

- **ADR-0035** — `Reconciler` trait collapsed to one sync method; runtime owns View persistence end-to-end.
- **ADR-0036** — runtime owns all hydration (intent + observation + view).
- **ADR-0042** — `ServiceMapHydrator` reconciler shape + `service_hydration_results` observation table.
- **ADR-0046** — `BackendIdAllocator` on `EbpfDataplane`; backend identity is monotonic + memoised at the userspace boundary.
- **ADR-0047** — Service-kind workload spec; Listener originally carried `Option<ServiceVip>`. Per ADR-0049 amendment 2026-05-14 the `vip` field on `Listener` was removed at the parser layer — VIPs are platform-issued only and structurally unrepresentable in the spec.
- **ADR-0049** — Platform-issued `ServiceVipAllocator`. VIPs are
  allocated synchronously at admission (before IntentStore write),
  keyed by `spec_digest = WorkloadIntent::spec_digest(&self)`. The
  bridge consults `ServiceVipAllocator::get(&spec_digest)` at hydrate
  time to surface the allocator-issued VIP. The intent aggregate
  itself carries no VIP field.
- **ADR-0050** — Intent-side `WorkloadIntent` aggregate. Persisted
  bytes at `IntentKey::for_workload(&workload_id)` decode through
  `WorkloadIntent::from_store_bytes(...)` into the kind-agnostic
  envelope; the bridge matches on the `Service(ServiceV1)` variant.
- **`.claude/rules/development.md` § Reconciler I/O** — pure sync `reconcile`, runtime-owned View, `BTreeMap<TargetResource, View>` semantics.
- **`.claude/rules/development.md` § Persist inputs, not derived state** — bridge View carries `last_written_fingerprint`, NOT a "next-write deadline."
- **`.claude/rules/development.md` § Errors / "Never flatten typed error to `Internal(String)`"** — boot-time `EbpfDataplane::new` failures need a dedicated `#[from]` variant on `ControlPlaneError`, NOT `.map_err(|e| ControlPlaneError::internal(...))`.

## Decision

### 1. New reconciler `BackendDiscoveryBridge`

A new reconciler kind, `backend-discovery-bridge`, lives at
`crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/`
(module structure mirrors `service_map_hydrator/`). The canonical types
(`BackendDiscoveryBridge` struct, `BackendDiscoveryBridgeState`,
`BackendDiscoveryBridgeView`, `ServiceListenerSet`, `ProjectedListener`,
`RunningAllocSet`) live in `overdrive-core::reconciler` because
`AnyReconciler` holds the concrete type in its variant and
`overdrive-core` cannot depend on `overdrive-control-plane` —
same layering as `WorkloadLifecycle` and `ServiceMapHydrator`.

Per-target keying = `WorkloadId` (one bridge per workload). The
`EvaluationBroker` collapses N alloc-state transitions for a single
workload to ONE pending evaluation, per ADR-0023. This is the same
keying `WorkloadLifecycle` already uses for the workload, so the
broker-pending enqueue site that fires for lifecycle transitions
fires for the bridge against the same target.

#### Bridge surface

```rust
impl Reconciler for BackendDiscoveryBridge {
    const NAME: &'static str = "backend-discovery-bridge";
    type State = BackendDiscoveryBridgeState;
    type View = BackendDiscoveryBridgeView;

    fn reconcile(
        &self,
        desired: &Self::State,    // ServiceListenerSet from intent + allocator-issued VIP
        actual:  &Self::State,    // RunningAllocSet from obs
        view:    &Self::View,     // BTreeMap<ServiceId, BackendSetFingerprint>
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

Pure sync per ADR-0035; no `.await`, no `Instant::now()`, no DB handle.

#### Trigger / hydration shape

Broker-pending enqueue on `AllocStatusRow` change — same surface
that already drives `WorkloadLifecycle`. The runtime's existing
convergence-loop spawn site gains one line: alongside enqueuing
`WorkloadLifecycle` on alloc state transitions, also enqueue
`BackendDiscoveryBridge` keyed by the same `WorkloadId`.

Hydration:

| Projection | Source | Hydrator surface |
|---|---|---|
| `desired.listeners` (intent listeners) | `WorkloadIntent::Service(ServiceV1).listeners` read via `IntentKey::for_workload(&workload_id)` and `WorkloadIntent::from_store_bytes` (ADR-0050) | New match arm in `hydrate_desired` |
| `desired.assigned_vip` | `ServiceVipAllocator::get(&spec_digest)` where `spec_digest = WorkloadIntent::spec_digest(&intent)` (ADR-0049 § 5a) | Same `hydrate_desired` arm — synchronous in-memory lookup against the `Arc<Mutex<PersistentServiceVipAllocator>>` field on `AppState` |
| `actual.running` | `ObservationStore::alloc_status_rows_for_workload(workload_id)` filtered to `state == Running` | New match arm in `hydrate_actual` |
| `view.last_written_fingerprint` | `RedbViewStore::bulk_load` at register; `write_through` after each tick | Runtime-owned per ADR-0035 |

The bridge **only emits a row when the allocator has issued a VIP for
the workload's `spec_digest`**. In Phase 1's submit-time admission path
(ADR-0049 § 4) the VIP allocation happens before the IntentStore write,
so by the time the bridge's first tick fires (post Running observation)
the allocator memo is already populated. If `ServiceVipAllocator::get`
unexpectedly returns `None` — a structural impossibility in Phase 1's
submit-time path — the bridge logs a debug event and skips the tick;
the natural convergence loop retries on the next enqueue.

#### Endpoint derivation

For each `(running_alloc, listener)` pair, the bridge constructs:

```rust
Backend {
    ipv4: self.host_ipv4,   // resolved once at boot from client_iface
    port: listener.port.get(),
    weight: 1,
    healthy: true,           // #170 ships real health
    _pad: 0,
}
```

Phase 2.2 is single-node, so every Running alloc resolves to the same
`host_ipv4`. Multi-node Phase 2 extension lookup-by-`node_id` is
structurally compatible (the bridge takes `host_ipv4: Ipv4Addr` at
construction; future multi-node injects a `NodeAddressLookup` port
instead).

#### View shape — persist inputs only

```rust
pub struct BackendDiscoveryBridgeView {
    pub last_written_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>,
}
```

Per `.claude/rules/development.md` § "Persist inputs, not derived
state" — the persisted input is the per-service fingerprint of the
last `ServiceBackendRow` the bridge successfully wrote. The dedup
decision ("should I write the row this tick?") is recomputed every
tick by comparing the fresh fingerprint against the persisted
fingerprint — never persisted as a derived "next_write_at" deadline.

The fingerprint covers both the allocator-issued VIP and the backend
set; a VIP change (which Phase 1 does not produce in steady state —
allocator VIPs are stable across resubmits of the same digest, per
ADR-0049's content-addressed memo) automatically falsifies the dedup
check and triggers a fresh row write. No additional View field is
required for VIP-change tracking.

`BTreeMap` per `.claude/rules/development.md` § "Ordered-collection
choice".

The View carries dedup state, not retry state — the bridge's writes
go to the local `ObservationStore` (in-process), which doesn't fail
in the way an external HTTP call does. If the `ObservationStore::write`
returns `Err`, the action shim surfaces it; the bridge's next tick
sees `view.last_written_fingerprint != new_fp` (no entry written yet)
and re-emits — the natural §18 convergence shape. No retry-budget
machinery needed at the bridge level.

#### LWW writer + counter

```rust
LogicalTimestamp {
    counter: tick.tick.saturating_add(1),
    writer:  self.writer_node_id.clone(),   // = AppState.node_id
}
```

Identical to `action_shim::dataplane_update_service.rs:121-122`.
`AppState.node_id` is the single source-of-truth; real node-bootstrap
identity in Phase 2 multi-node replaces the placeholder with no
bridge code change.

Multi-node future-compat: per-node monotonic counters + writer
tiebreak IS the CR-SQLite LWW semantics. The bridge does not
preclude a future cluster-wide convention that "the node hosting the
elected primary writer of a workload writes its backend row" — that
discipline is layered on top of the LWW model, not encoded in the
row shape.

#### VIP source

Per ADR-0049 § 5a (placement decision C — chosen): the
allocator's own persisted memo IS the source of truth for the assigned
VIP. The bridge consults `ServiceVipAllocator::get(&spec_digest)` at
hydrate time. The intent aggregate carries no VIP; the operator never
specifies one; the dataplane never invents one. Three structural
defenses (parser-level field removal per ADR-0049 § 5; intent-aggregate
absence per ADR-0050 § 2 § "What `ServiceV1` does NOT carry"; wire-level
`deny_unknown_fields` per ADR-0051 § 2) prevent a smuggled-VIP class of
bug at every layer; the bridge consumes the allocator's authoritative
mapping and nothing else.

`ServiceId` is derived from `(assigned_vip, listener.port,
listener.protocol, "service-map")` per ADR-0040 § 1 (as amended by the
ADR-0040 2026-06-03 companion revision, which added the L4-protocol
axis — Model A: one `ServiceId` per `(vip, port, proto)` slot) — the
existing `ServiceId::derive` constructor consumes the allocator-issued
VIP rather than a spec-side field, plus the listener's own `proto`.
Per-Service-slot identity is stable across resubmits because
`spec_digest` is stable across resubmits (ADR-0049 § 4 — spec digest
invariance) AND the allocator memo on that digest is stable (ADR-0049
§ 1 — memo-hit returns the existing VIP); each listener's `(port,
proto)` is part of the persisted spec, so a TCP listener and a UDP
listener on the same `(VIP, port)` derive two distinct, stable
`ServiceId`s.

### 2. New action variant `Action::WriteServiceBackendRow`

The reconciler is pure; all side effects flow through Actions per
ADR-0023. The bridge needs:

```rust
// in overdrive-core::reconciler::Action
WriteServiceBackendRow {
    row: ServiceBackendRow,
    correlation: CorrelationKey,
},
```

The `correlation` is derived from
`(target = "backend-discovery-bridge/<service_id>",
   spec_hash = ContentHash::of(fingerprint.to_le_bytes()),
   purpose = "write-service-backend-row")` — same constructor
precedent as `DataplaneUpdateService` per ADR-0042 § 1.

The action shim wrapper lives at
`crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs`
— file shape symmetric with `dataplane_update_service.rs`. Dispatch:

```rust
observation.write(ObservationRow::ServiceBackend(row.clone())).await
```

Error surface: pass-through `ObservationStoreError` per
`.claude/rules/development.md` § Errors / pass-through embedding. No
new `terminal: Option<TerminalCondition>` field — service-row writes
cannot terminate an allocation (ADR-0042 § 1 / ADR-0037 invariant).

The action-shim match in `action_shim/mod.rs` gains one new arm; the
match becomes non-exhaustive until added — caught at compile time
per ADR-0023's exhaustive-match property.

### 3. `EbpfDataplane` production single-mode boot

#### `[dataplane]` config section

A new `[dataplane]` section in `overdrive.toml`:

```toml
[dataplane]
client_iface = "lb_veth_a"
backend_iface = "lb_veth_b"
```

Required for production single-mode boot. Missing section produces
`ControlPlaneError::Validation { field: Some("dataplane"), ... }`
— same shape as missing `[tls]` per ADR-0010.

This `[dataplane]` section is structurally distinct from the
existing `[dataplane.vip_allocator]` subsection introduced by
ADR-0049 § 3 (which already carries `ranges` + `reserved`); both
nest under the same `[dataplane]` parent in `overdrive.toml`.
`client_iface` + `backend_iface` are this ADR's addition.

The configured `client_iface` flows into `EbpfDataplane::new(client_iface,
backend_iface)` and into the `BackendDiscoveryBridge`'s `host_ipv4`
resolution (`getifaddrs` on `client_iface` at boot).

#### `ControlPlaneError::DataplaneBoot` variant

Following the `ViewStoreBoot` / `Cgroup` / `CgroupBootstrap` /
`WorkloadsBootstrap` precedents in `crates/overdrive-control-plane/src/error.rs`:

```rust
#[derive(Debug, Error)]
pub enum DataplaneBootError {
    #[error("EbpfDataplane construction failed (client_iface={client_iface}, \
             backend_iface={backend_iface}): {source}\n\n\
             Try: `ip link show <iface>` to verify interfaces exist; ...")]
    Construct {
        client_iface: String,
        backend_iface: String,
        #[source]
        source: overdrive_core::traits::dataplane::DataplaneError,
    },

    #[error("EbpfDataplane probe failed: {source}\n\nTry: \
             `rm /sys/fs/bpf/overdrive/*` and retry; inspect dmesg.")]
    Probe {
        #[source]
        source: overdrive_core::traits::dataplane::DataplaneError,
    },

    #[error("EbpfDataplane iface IPv4 resolution failed for {iface}: {source}\n\n\
             Try: `ip -4 addr show <iface>`.")]
    IfaceAddrResolution {
        iface: String,
        #[source]
        source: std::io::Error,
    },
}
```

`ControlPlaneError` gains `#[error(transparent)] DataplaneBoot(#[from] DataplaneBootError)`.
The `to_response` arm is exhaustiveness-only (`StatusCode::INTERNAL_SERVER_ERROR`)
— boot failures precede listener bind and never reach an HTTP
response, same as the existing infra-error precedents.

This is a hard requirement per `.claude/rules/development.md` § Errors
("Never flatten a typed error to `Internal(String)`"). A
`.map_err(|e| ControlPlaneError::internal("ebpf", e))` is **forbidden**
on this path.

#### Earned-Trust probe

Per principle 12, every adapter that depends on something external
(kernel BPF subsystem + bpffs in this case) demonstrates it can honor
its contract in the live environment. `EbpfDataplane::new` performs
an implicit load-time probe (the kernel rejects malformed programs at
load); a runtime probe — write + read-back a sentinel `BACKEND_MAP`
entry — is added in the same composition root step:

```rust
let ebpf_dp = EbpfDataplane::new(client_iface, backend_iface)?;
ebpf_dp.probe().await?;   // ← Earned-Trust gate before first use
let dataplane: Arc<dyn Dataplane> = Arc::new(ebpf_dp);
```

Probe shape: write a sentinel `BackendEntryPod` at `BackendId::PROBE
= u32::MAX`, read it back, assert byte-equal, delete it. Failure
surfaces as `DataplaneBootError::Probe { source }`; boot refuses to
start with structured `health.startup.refused` event (same pattern as
`ViewStoreBootError::Probe`).

Composition root invariant per Earned Trust: **wire then probe then
use**. The `Arc<dyn Dataplane>` is only handed to `AppState` after
probe success.

#### Attach-mode fallback emit location

The `EOPNOTSUPP`/`ENOTSUP` fallback from `XdpFlags::DRV_MODE` →
`XdpFlags::SKB_MODE` already exists in the `EbpfDataplane::new`
loader path. The single structured `tracing::warn!` event with the
locked name `xdp.attach.fallback_generic` (per
`.claude/rules/development.md` § "Attach mode") emits **inside
`EbpfDataplane::new`** at the moment the fallback decision is taken
— per-iface, not aggregated. The pure decision function
`should_fallback_to_generic` stays as the classifier; the imperative
emit + retry lives at the same level as the retry call.

#### Shutdown — RAII via `Drop`

XDP detach is already RAII (`aya::programs::XdpLinkId::Drop` detaches).
The new project-owned cleanup is the bpffs pin unlink:

```rust
impl Drop for EbpfDataplane {
    fn drop(&mut self) {
        let pin_path = self.pin_dir.join(SERVICE_MAP_NAME);
        if let Err(e) = std::fs::remove_file(&pin_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!(
                    name: "xdp.shutdown.unlink_failed",
                    path = %pin_path.display(),
                    error = %e,
                );
            }
        }
    }
}
```

Drop runs on panic and on clean shutdown; `XdpLinkId` field drops
detach the programs automatically. A SIGKILL still leaks — that
recovery scenario is the
`.claude/rules/debugging.md` § "Leftover XDP attachments across runs"
operator-side cleanup discipline; not a code bug.

No new `Dataplane::shutdown` method on the trait surface; the
trait stays minimal.

#### BPF object path resolution

`include_bytes!`-embedded at build time (existing precedent in
`crates/overdrive-dataplane/build.rs`), with `OVERDRIVE_BPF_OBJECT`
env override for dev/test (per `.claude/rules/testing.md` § "BPF-object-dependent
crates work via env override"). No operator config field. The boot
composition doesn't know the BPF object's location — `EbpfDataplane::new`
resolves it internally.

#### `NoopDataplane` single-cut deletion

Per `feedback_single_cut_greenfield_migrations.md` and `.claude/rules/development.md`
§ "Deletion discipline", `NoopDataplane` is removed from the
production boot path AND from `crates/overdrive-host/src/dataplane.rs`
AND from the `lib.rs` re-export in the same commit that lands the
`EbpfDataplane` boot composition. No feature flag, no `[dataplane]
enabled = false` fallback, no deprecation.

Tests today install `Arc<SimDataplane>` directly per the docstring
at `dataplane.rs:13-15`; a workspace-wide grep after the swap is
expected to return zero active production users.

### 4. Joint walking-skeleton acceptance test

One Tier 3 integration test at
`crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`
gates both #174 and #175 jointly. Gated by the `integration-tests`
feature; runs via `cargo xtask lima run --` per
`.claude/rules/testing.md` § "Running tests — Lima VM".

#### Shape

```
GIVEN production control-plane configured with EbpfDataplane
       (client_iface=lb_veth_a, backend_iface=lb_veth_b)
  AND  BackendDiscoveryBridge + ServiceMapHydrator both registered
  AND  ServiceVipAllocator bulk-loaded from IntentStore (probe-passed)
WHEN  a Service spec is submitted with one listener
       (port=8080, protocol=tcp)
       — no operator-supplied VIP; the field is unrepresentable per
         ADR-0049 § 5
  AND  admission allocates VIP V via ServiceVipAllocator
       (synchronous, before IntentStore write — ADR-0049 § 4)
  AND  the alloc reaches Running
THEN  within ≤ 5 reconciler ticks (≤ 500ms wall-clock plus slack):
       - BACKEND_MAP contains an entry whose ipv4 matches the
         host's lb_veth_a IPv4 and whose port = 8080
       - SERVICE_MAP for (vip=V, port=8080) resolves to a
         non-empty inner map containing that BackendId
```

The test setup must explicitly assert the allocator memo is populated
for the submitted Service's `spec_digest` before declaring the
allocation precondition met — the bridge's correctness depends on
this allocator state being present at hydrate time. Submit-echo
already renders the assigned VIP per ADR-0049 § 4; the test reads
this value to construct the `ServiceKey` for the SERVICE_MAP lookup
rather than hard-coding a VIP.

The test asserts on **observable kernel side effects** via the typed
map handles (`BackendMapHandle::keys()`, `HashOfMapsHandle::get_inner_entries()`),
NOT on program internal reachability — per `.claude/rules/testing.md`
§ "Tier 3 → Assertion rules".

The "real TCP connection to VIP succeeds" step is **in-gate**
(D3 decision 2026-05-21 — do NOT defer). The walking-skeleton is
the joint e2e acceptance for #174 + #175: BPF map state alone proves
wiring, not reachability, and reachability IS the feature's value.
The test opens a TCP connection to `<assigned_vip>:<port>` and
asserts a round-trip payload through the kernel XDP / reverse-NAT
path. DISTILL pins the flake-mitigation shape: bind-readiness
poll-connect-with-timeout (Service `Running` ≠ port bound), and the
exec command's listener choice (plain `nc -l 8080` is unsuitable —
DISTILL specifies a `socat TCP-LISTEN:8080,fork EXEC:cat`-equivalent
or a baked-in echo binary).

#### Two new DST invariants (Tier 1)

Both run on every PR per `.claude/rules/testing.md` § Tier 1:

- **`BridgeEventuallyWritesBackendRow`** — for every Service workload
  with `≥ 1` listener AND an allocator-issued VIP for its
  `spec_digest` AND `≥ 1` Running alloc, the bridge writes a
  `ServiceBackendRow` whose `backends` field reflects exactly the
  Running endpoints, within a bounded number of ticks.

- **`BridgeIdempotentSteadyState`** — once `desired == actual` for
  every service, the bridge emits zero `Action::WriteServiceBackendRow`
  actions on subsequent ticks given unchanged inputs.

These pair with the existing `HydratorEventuallyConverges` /
`HydratorIdempotentSteadyState` from ADR-0042 to give a complete ESR
specification for the bridge → hydrator → dataplane convergence loop.

### 5. Sequencing across DELIVER slices

**Slice 1 (closes #174)**: Implement `BackendDiscoveryBridge` +
register in production boot. Bridge writes `ServiceBackendRow` under
`NoopDataplane` (still threaded). DST invariants land. No
walking-skeleton yet.

**Slice 2 (closes #175)**: Replace `NoopDataplane` with `EbpfDataplane`
in production boot. Add `DataplaneBootError`. Add `[dataplane]`
client_iface / backend_iface config keys (alongside the existing
`[dataplane.vip_allocator]` from ADR-0049 § 3). Add Earned-Trust
probe. Land the Tier 3 walking-skeleton test that subsumes both
deliveries.

This gives each issue its own gate: #174's is the DST invariant pair;
#175's is the walking-skeleton. The walking-skeleton fails until #175
lands because `NoopDataplane`'s no-op `update_service` doesn't
populate `BACKEND_MAP` — gating #175's PR naturally.

The VIP allocator (ADR-0049) is already closed (delivered 2026-05-19
per `docs/evolution/2026-05-19-service-vip-allocator.md`); both
slices proceed against the allocator's `AppState` integration as a
landed dependency.

## Alternatives Considered

### A — Extend `WorkloadLifecycle::reconcile` (no new reconciler)

The existing `WorkloadLifecycle` reconciler already reads
intent-side `WorkloadIntent` + observation-side `AllocStatusRow` (per
ADR-0050 OQ-5 keying + ADR-0021), and the lifecycle path was recently
extended to handle Service variants (VIP allocation/release
correlation via `workload_id`, `service_spec_digest` population in
`hydrate_desired`/`hydrate_actual`). The precedent is current.

**Rejected** because:

1. **Single-responsibility violation, intensified by the Service
   extension.** `WorkloadLifecycle.reconcile` already branches on
   kind, manages restart budget, emits terminal claims, and now
   emits `Action::ReleaseServiceVip` on Service terminal-state
   observation. Adding a second emit-channel kind (observation-row
   writes vs alloc-state actions) on top of those concerns triples
   the surface area concentrated in one reconciler.
2. **Forces a new Action variant either way.** The reconciler is
   pure; the obs-row write is a side effect; an action shim
   dispatches it. The extension shape would still emit
   `Action::WriteServiceBackendRow` — the action variant lands
   regardless.
3. **Inflates `WorkloadLifecycleView`.** The dedup memory
   (`last_written_fingerprint`) doesn't belong in lifecycle View; the
   View already grew with `released_for_terminal` per ADR-0049 § 6,
   and a third concern compounds.
4. **Loses pattern symmetry.** `ServiceMapHydrator` is a downstream
   peer reconciler; the bridge is its upstream peer. Two peer
   reconcilers, one per producer/consumer, mirrors the §18 reference
   shape.
5. **Failure isolation lost.** A regression in the bridge logic
   risks the lifecycle path — landing two changes in one reconciler
   amplifies blast radius, and the lifecycle is now the only path
   that drives VIP reclamation through `ReleaseServiceVip`.

### B — Action-shim-side bridge in `exit_observer` or the existing dispatch shims

Have `exit_observer` or the action shim write `ServiceBackendRow` as
a side effect of alloc-state transitions.

**Rejected** because:

1. Violates the §18 / ADR-0023 / ADR-0042 boundary — action shims
   execute actions; they don't read intent-side projections
   (`listener decls`) the way the bridge needs to.
2. Couples the observer to intent-side knowledge it has no business
   reading.
3. Hides the convergence loop. The bridge is exactly the §18
   reconciler shape: "keep the cluster's observable state reflecting
   the projection of intent + actual."

### C — Per-tick poll (no broker-pending trigger)

The bridge evaluates every 100ms regardless of trigger.

**Rejected** because the existing broker-pending mechanism is the
reference §18 ingress. The bridge has no special triggering needs
that justify a second mechanism.

### D — Keep `NoopDataplane` behind a feature flag / `[dataplane] disabled = true`

Operator escape hatch for hosts without XDP kernel support.

**Rejected** because:

1. Violates `feedback_single_cut_greenfield_migrations.md` — Phase
   1/2 is greenfield, no parallel old paths.
2. The escape hatch is itself a "future deferral that compounds" —
   the next operator's first question is "do I need this?", the next
   contributor maintains both code paths, the next bug-fix-cycle
   forgets the escape hatch case.
3. A host that cannot run XDP cannot run Service workloads. Honest
   admission-time refusal at boot is better than soft-on-no-op
   behavior that misleads operators.

### E — Single PR train landing both issues at once

**Rejected** because the surface area is large enough that
piecemeal review is structurally valuable. Slice 1 → Slice 2
sequencing isolates failures per issue.

### F — DST-only walking skeleton (no Tier 3 test)

**Rejected** because the existing `HydratorEventuallyConverges` /
`HydratorIdempotentSteadyState` invariants already cover the
hydrator-side DST surface, and the bridge's own DST invariants
(`BridgeEventually*`, `BridgeIdempotentSteadyState`) cover its DST
surface. The Tier 3 gate exercises the **boot composition + real
kernel BPF program load + bpffs interaction** that DST doesn't
cover.

### G — Generic `Action::ObservationStoreWrite { row }` variant

A single Action for all obs-row writes vs a typed
`WriteServiceBackendRow`.

**Rejected** for the same reason ADR-0042 § "Alternatives" rejected
`Action::DataplaneCall { op: DataplaneOp }`: over-engineering for the
present scope, and the per-row typing carries enough context that the
action-shim match arm stays trivially exhaustive per row kind.
Future obs-row writes (compiled-policy verdicts, revoked-cert lists)
get their own typed Action variants — the Action enum grows
linearly with the observation surface, which is structural strength
not weakness.

### H — Project the allocator-issued VIP onto a separate observation row instead of consulting the allocator at hydrate time

Have admission (or a dedicated reconciler) write a
`service_assignments` observation row carrying `(workload_id,
spec_digest, assigned_vip)`; the bridge reads that row instead of
calling `ServiceVipAllocator::get`.

**Rejected** for the same reason ADR-0049 § 5a rejected its Option
B (observation-only placement): creates a second source of truth for
the VIP assignment, requires synchronous obs-write at admission
(which the admission path does not do today), and faces chicken-and-egg
on post-restart hydration ordering. The allocator's persisted memo
IS the authoritative VIP store per ADR-0049 § 5a; reading from it
directly is the simplest correct shape.

## Consequences

### Positive

- **J-PLAT-004 closes end-to-end.** The intent-to-packet pipeline is
  wired in production single-mode for the first time. ASR-2.2-04
  becomes structurally achievable.
- **Pattern established for Phase 2+ observation-row writers.** Any
  future "watch X intent + Y observation, write Z observation row"
  shape (e.g., POLICY_MAP hydrator at GH #158, FS_POLICY_MAP at #26)
  follows the bridge's two-reconciler-peer + Action + ESR-invariant
  triad.
- **Earned-Trust probe on the dataplane.** First adapter that
  exercises the principle 12 "wire then probe then use" composition
  root invariant beyond the existing `ViewStore` probe — sets the
  precedent for the future `Driver`, `Llm`, and `Transport` adapters.
- **`NoopDataplane` deleted end-to-end.** No straggling production-default
  for the dataplane; the trait surface only has real adapters
  (`EbpfDataplane`, `SimDataplane`).
- **Type-safe boot-error path.** `DataplaneBootError` joins
  `ViewStoreBootError` / `CgroupBootstrapError` / `WorkloadsBootstrapError`
  as a structured boot-time error surface the CLI can branch on for
  diagnostics.
- **Structural VIP-source coherence.** The bridge reads the
  allocator's authoritative memo (ADR-0049 § 5a); the intent
  aggregate cannot represent a VIP (ADR-0050 § 2); the wire layer
  cannot smuggle one (ADR-0051 § 2). Three layers of defense; the
  bridge consumes one source.

### Negative

- **One new `Action` variant + one new action-shim wrapper.** Small
  maintenance cost; exhaustive-match catches every consumer.
- **One new `AnyReconciler` variant.** Small; ~10 LoC in the runtime
  for the hydrate arms.
- **Production binary cannot run without kernel BPF.** Honest
  trade-off (no `[dataplane] disabled = true`): operators on hosts
  without XDP cannot run Overdrive. Acceptable — Service workloads
  cannot run on such hosts anyway.
- **Walking-skeleton requires Lima on macOS dev hosts.** Already
  the project standard per `.claude/rules/testing.md`; not new debt.
- **Bridge's `hydrate_desired` arm depends on the
  `ServiceVipAllocator` being constructed and bulk-loaded before the
  reconciler runtime accepts evaluations.** The composition order is
  already correct (the allocator is built in
  `bulk_load_service_vip_allocator` before `AppState::new`, which is
  before `runtime` starts ticking); the bridge inherits this ordering
  invariant. Document it on the bridge's hydrate-arm docstring so a
  future reorder doesn't silently break the contract.

### Quality-attribute impact

- **Correctness — bug fix structurally closed**: positive (large).
  J-PLAT-004 closes; the data flow is end-to-end observable.
- **Maintainability — modifiability**: positive. New observation-row
  writers follow a single pattern.
- **Maintainability — testability**: positive. ESR invariants pair
  with the upstream hydrator's pair; full convergence-loop coverage.
- **Reliability — fault tolerance**: positive (small). Earned-Trust
  probe refuses boot on a malformed kernel BPF surface; operators
  see structured failure at startup, not silent dataplane drops at
  steady state.
- **Reliability — recoverability**: neutral. Crash semantics are
  unchanged; redb fsync per tick.
- **Operator usability**: positive. Structured boot-error messages
  with remediation hints in `Display` per the existing
  `CgroupBootstrapError` precedent.
- **Compatibility — coexistence**: neutral.
- **Performance — time behaviour**: positive (small). Dedup via
  fingerprint short-circuits the action-emit path when inputs
  haven't changed; steady-state bridge tick is a `BTreeMap::get` +
  `fingerprint(...)` + Eq check.
- **Performance — resource utilisation**: neutral. Bridge View
  grows by O(services); fingerprint is a `u64`. Bounded by the
  number of distinct Service VIPs.
- **Security**: neutral.
- **Portability**: neutral. The bridge is portable; `EbpfDataplane`
  remains Linux-only via `#[cfg(target_os = "linux")]` (existing).

## Compliance — what survives from prior ADRs

- **ADR-0035 (collapsed `Reconciler` trait)** — preserved verbatim. The bridge implements the canonical sync `reconcile` shape.
- **ADR-0036 (runtime-owned hydration)** — preserved. Two new free-function match arms in `reconciler_runtime.rs`.
- **ADR-0042 (`ServiceMapHydrator` + `service_hydration_results`)** — preserved verbatim. The bridge is the *upstream* counterpart; ADR-0042's downstream contract is unchanged.
- **ADR-0023 (action-shim placement + tick cadence + exhaustive match)** — preserved. New Action variant + new shim wrapper file + new match arm.
- **ADR-0040 (three-map split + HASH_OF_MAPS atomic-swap)** — preserved verbatim. The walking-skeleton tests the existing primitive.
- **ADR-0045 (`bpf_redirect_neigh` datapath)** — preserved verbatim.
- **ADR-0046 (collision-free `BackendId` allocator)** — preserved verbatim. The bridge writes `Backend` values; `EbpfDataplane::update_service` assigns BackendIds via the allocator per ADR-0046.
- **ADR-0047 (`WorkloadKind` discriminator + Service kind)** — preserved. The bridge reads listeners from the Service kind only; Job / Schedule produce no backend rows.
- **ADR-0048 (rkyv versioned envelope)** — preserved verbatim. `ServiceBackendRow = ServiceBackendRowV1`; the bridge writes the existing V1 payload.
- **ADR-0049 (platform-issued `ServiceVipAllocator`)** — consumed. The bridge reads `ServiceVipAllocator::get(&spec_digest)` to obtain the VIP for each Service workload; the field is the bridge's single VIP source.
- **ADR-0050 (intent-side `WorkloadIntent` aggregate)** — consumed. The bridge's intent read decodes `WorkloadIntent::Service(ServiceV1)` via `WorkloadIntent::from_store_bytes`; `ServiceV1.listeners` is the intent-side input.
- **ADR-0051 (wire-side `SubmitSpecInput`)** — transitively consumed. The bridge never sees the wire layer directly; admission projects `SubmitSpecInput::Service(ServiceSpecInput)` onto `WorkloadIntent::Service(ServiceV1)` per `ServiceV1::from_submit` before any bridge tick fires.
- **`.claude/rules/development.md` § Reconciler I/O** — followed.
- **`.claude/rules/development.md` § Persist inputs, not derived state** — followed; View carries `last_written_fingerprint` (an input-derived content hash) not "next-write deadline."
- **`.claude/rules/development.md` § Errors / "Never flatten typed error to `Internal(String)`"** — followed; `DataplaneBootError` is the dedicated `#[from]` variant.

## References

- GH #174 — Backend discovery bridge.
- GH #175 — Wire EbpfDataplane into production single-mode boot.
- ADR-0035 (collapsed `Reconciler` trait) — substrate.
- ADR-0036 (runtime-owned hydration) — substrate.
- ADR-0042 (`ServiceMapHydrator` + `service_hydration_results`) — upstream counterpart.
- ADR-0040 (three-map split + HASH_OF_MAPS) — substrate.
- ADR-0045 (`bpf_redirect_neigh` datapath) — substrate.
- ADR-0046 (collision-free BackendId allocator) — substrate.
- ADR-0047 (workload kind discriminator) — substrate.
- ADR-0048 (rkyv versioned envelope) — substrate.
- ADR-0049 (platform-issued Service VIP allocator) — consumed dependency; bridge's VIP source.
- ADR-0050 (intent-side workload aggregate) — consumed dependency; bridge's intent-read shape.
- ADR-0051 (wire-side `SubmitSpecInput`) — transitively consumed.
- `docs/evolution/2026-05-19-service-vip-allocator.md` — feature evolution doc for the landed VIP allocator.
- `docs/feature/backend-discovery-bridge-service-reachability/design/wave-decisions.md` — option analysis + recommendations + deferrals + **review** (Atlas, 2026-05-20, APPROVED).
- `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md` — component-level design with C4 diagrams and code shapes.
- `.claude/rules/development.md` § Reconciler I/O.
- `.claude/rules/development.md` § Persist inputs, not derived state.
- `.claude/rules/development.md` § Errors / pass-through embedding.
- `.claude/rules/development.md` § Attach mode — native vs generic.
- `.claude/rules/testing.md` § Tier 3 — Real-Kernel Integration.
- `.claude/rules/debugging.md` § Leftover XDP attachments across runs.

## Changelog

- 2026-05-13 — Initial accepted version. Backend discovery bridge reconciler + `EbpfDataplane` production single-mode boot. Closes J-PLAT-004.
- 2026-05-20 — Renumbered from ADR-0049 to ADR-0052 after ADR-0049 was reassigned to the platform-issued Service VIP allocator (landed PR #184, delivered feature `service-vip-allocator` 2026-05-19). § 1 "Trigger / hydration shape" updated to read `WorkloadIntent::Service(ServiceV1)` (ADR-0050) and consult `ServiceVipAllocator::get(&spec_digest)` (ADR-0049 § 5a) instead of `Job.workload_spec.service.listeners` with a `Listener.vip` field. § 1 "VIP-less listener handling" subsection deleted (operator-supplied VIPs are unrepresentable; no skip-vs-include decision remains). New alternative H rejected (project-VIP-onto-observation-row). Sequencing § 5 updated to note the VIP allocator is no longer a blocker. Walking-skeleton § 4 updated to source the assigned VIP from submit-echo rather than hard-code one.
