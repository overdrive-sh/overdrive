# BE10 local direct-VIP withdrawal — ADR-0101 revision 4 amendment

**Status:** Accepted; independent DESIGN review APPROVED, 2026-09-08,
[iteration 1](review-amendment-be10-local-backend-withdrawal.md#iteration-history),
with no findings.
**Author:** Codex acting as solution architect.
**Authority:** user-approved focused application/component DESIGN amendment;
ADR-0101 revision 3 remains independently approved with separate provenance.
Revision 4 approval is recorded by the independent review above, not self-approval.
This document does not implement revision 4 or change BE10's assertion.

## Scope and revalidated evidence

BE10's recorded run `21245211-7507-4f57-b5c1-48d450097fc5` at checkpoint
`cbcf9a6d205b187735c5a080be9711ed32a2116d` plus the in-flight ADR-0101 changes
ran 12 BE tests: 10 passed, BE02 and BE10 failed. The
[test-owner transfer](../deliver/adr-0101-test-transfer.md#be10-map-retention-versus-backend-selection)
records seed `257221`: healthy local map, real ProbeRunner readiness Fail,
authoritative unhealthy row, DNS/resolver withdrawal, queued hydrator dispatch,
then retained `Some(192.0.2.10:18081)` at the expected-None assertion.
The recovery suffix was unexecuted; independent BE09 covered mesh/DNS recovery.
This DESIGN re-read the current path; it ran no test or native experiment.

The current registration/removal path and BE10 body match that recorded
failure. No cancellation, shutdown, crash, retry fault, second concurrently
Running allocation or fabricated unhealthy row is needed. The allocation
remains Running while readiness fails. Sim exercises production owners through
existing ports; it does not prove that default native VM composition selects
host-local non-mesh addresses. Mesh-subnet backends are excluded before local
classification. The [original E09 diagnosis](../../../analysis/e09-v2-failed-service-reachability.md)
concerns replacement VMs selected through mesh resolution, not this local map.
This amendment is not claimed necessary for that original fix.

| Current production caller/owner path | Current source evidence |
|---|---|
| ServiceLifecycle hydrates allocation readiness/address, derives health once, writes full row, enqueues hydrator | `crates/overdrive-reconcilers/src/service_lifecycle.rs:863`, `:1092`, `:1154`, `:1163`, `:1176` |
| Row executor awaits ObservationStore write; runtime awaits action dispatch | `crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs:52`; `reconciler_runtime.rs:1566` |
| Hydrator reads keyed row and listener; actual hydration reads remote results, not local map contents | `crates/overdrive-reconcilers/src/service_map_hydrator.rs:496`, `:535` |
| Mesh exclusion precedes local partition; complete local fingerprint changes with health; helper ignores health | `service_map_hydrator.rs:353`, `:362`, `:452`, `:599`; `crates/overdrive-core/src/dataplane/fingerprint.rs:55` |
| Serial shim already dispatches both local action variants and awaits their ports | `crates/overdrive-control-plane/src/action_shim/mod.rs:922`, `:3046`, `:3050`; `register_local_backend.rs:66`; `deregister_local_backend.rs:53` |
| Host adapter installs reverse then forward; removal removes forward then caller-keyed reverse | `crates/overdrive-dataplane/src/lib.rs:2158`, `:2214`; Dataplane contracts at `crates/overdrive-core/src/traits/dataplane.rs:323`, `:387` |
| Sim adapter models both post-states through the same ports; BE10 drives owners | `crates/overdrive-sim/src/adapters/dataplane.rs:499`, `:541`; `tests/integration/service_backend_projection.rs:977` |
| New direct-VIP connect consumes retained forward entry without a health field/check | `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:44`, `:78`, `:88`; attachment at `crates/overdrive-dataplane/src/lib.rs:679` |
| Normal loop awaits complete ticks before shutdown; action batch drains after individual errors | `crates/overdrive-control-plane/src/lib.rs:3364`, `:3385`; `action_shim/mod.rs:922`–`:953` |

## Decision, alternatives and reuse

Select the existing removal operation for a still-present unhealthy local
backend. Its address already supplies the reverse key required by the port.
Priorities are BE10 correctness, preserved lifecycle authority, then minimal
implementation/verification scope. Current Rust/Tokio/BPF/Sim dependencies,
architecture, deployment and ownership remain; no technology, license,
security policy or new adapter/Earned-Trust probe choice is introduced.

| Option | Disposition |
|---|---|
| Filter unhealthy local members and emit nothing | Reject: an earlier registration remains usable; absence of an insert is not removal. |
| Select existing register/deregister action from materialized health | Selected: solves the reproduced successful-effect trajectory with no new API or memory; withdrawal costs the existing two-map removal. |
| Add local readback/acknowledgement/retry machinery | Not selected: unnecessary for the demonstrated BE10 trajectory; generalized consumer fault hardening is outside approved scope, not an added requirement. |

| Component / path | Decision | Contract shape, universe and assertion mechanism |
|---|---|---|
| ServiceLifecycle and row executor | REUSE | Existing bounded publication for one Service's listeners; BE10 production-authored row remains the oracle. No second health owner. |
| `overdrive-reconcilers/src/service_map_hydrator.rs` | EXTEND | Pure-function reconcile returns existing Actions/next View. Delta is local action variant/purpose for unhealthy candidates on the existing fingerprint change; assert unchanged remote plan and View complement. |
| Action variants, validation and local shims | REUSE | Bounded-change effect for exact forward `(vip, vip_port, proto)` and reverse `(backend IP, backend port, proto)` keys; retain listener validation and typed propagation. |
| Dataplane / EbpfDataplane / SimDataplane | REUSE | Existing dual-map operations; Sim post-state assertions and real-kernel port evidence. No new read port or default implementation. |
| BPF connect/sendmsg/recvmsg consumers | REUSE | No program/map-layout change. New-connect no-rewrite after removal, not firewall denial. |
| Mesh/DNS, broker and runtime | REUSE | Existing asynchronous observation/handoff and serial execution; retain BE10 DNS/resolver controls. |

System Context and Container diagrams remain those in the
[ruling](backend-eligibility-convergence-ruling.md#application-architecture-and-effects).
Focused component map (no new deployment unit):

```mermaid
flowchart LR
    SL[ServiceLifecycle] -->|publishes authoritative health| Row[ServiceBackendRow]
    Row -->|supplies local candidates and health| H[ServiceMapHydrator]
    H -->|returns existing register or deregister action| Shim[Serial action shim]
    Shim -->|awaits existing Dataplane port| DP[EbpfDataplane / SimDataplane]
    DP -->|updates or removes exact keys| Maps[Local forward and reverse maps]
    Maps -->|supply rewrite when present| Hook[Existing cgroup socket hooks]
```

## Exact contract surface

No public API changes. In `service_map_hydrator.rs` rename only the private
helper to `fn push_local_backend_actions(actions: &mut Vec<Action>, local:
&[&Backend], ctx: &LocalBackendEmit<'_>)`; replace its one production call.
`LocalBackendEmit<'a>` retains exactly `service_id: ServiceId`,
`vip_v4: Ipv4Addr`, `vip_port: u16`, `proto: Proto`, `target_str: &'a str`,
`spec_hash: &'a ContentHash`. No other helper signature changes are required.
Every public State, Desired, View, RetryMemory and constructor stays unchanged.

For every existing classifier-accepted local candidate, choose exactly one:

| Materialized health | Existing Action variant | Correlation purpose |
|---|---|---|
| `true` | `Action::RegisterLocalBackend` | `"register-local-backend"` |
| `false` | `Action::DeregisterLocalBackend` | `"deregister-local-backend"` |

Both variants retain exactly `service_id: ServiceId`, `vip: Ipv4Addr`,
`vip_port: u16`, `proto: Proto`, `backend: SocketAddrV4`,
`correlation: CorrelationKey`. Use `ctx.service_id`, `ctx.vip_v4`,
`ctx.vip_port`, `ctx.proto`, candidate's existing IPv4 SocketAddr, and
`CorrelationKey::derive(ctx.target_str, ctx.spec_hash, purpose)` respectively.
Port/protocol come from the existing keyed listener fact, not backend port or
a default. Target stays `service-map-hydrator/{service_id}`; spec hash stays
`ContentHash::of(desired_svc.fingerprint.to_le_bytes().as_slice())`.

The two reused Dataplane signatures are already async:

- `async fn register_local_backend(&self, vip: Ipv4Addr, vip_port: u16,
  backend: SocketAddrV4, proto: Proto) -> Result<(), DataplaneError>`.
- `async fn deregister_local_backend(&self, vip: Ipv4Addr, vip_port: u16,
  backend: SocketAddrV4, proto: Proto) -> Result<(), DataplaneError>`.

Each existing shim retains `pub async fn dispatch(action: &Action, dataplane:
&dyn Dataplane) -> Result<(), RegisterLocalBackendDispatchError>` or
`Result<(), DeregisterLocalBackendDispatchError>` in its respective module.
No second method, detached future, runtime lookup, new error or observation row.

## Desired, observed, ordering and errors

Keep the full local candidate vector, including unhealthy members, in the
existing local fingerprint. A health flip changes it and supplies the tuple
to deregister. Pre-filtering unhealthy members would lose that tuple. Preserve
remote fingerprint/action/backoff, mesh exclusion, address rejection, IPv6
handling, listener lookup, hydration and the no-change emission gate. Local
actions retain their position after the remote action, if any. View persistence
still precedes serial awaited dispatch.

`actual.actual` is the remote ServiceHydrationStatus projection, not local-map
readback. `last_applied_local_fingerprint` records emission despite its name,
not successful application. Neither gains stronger meaning. Successful local
registration installs both directions; successful deregistration removes
forward first, then caller-keyed reverse. Absent keys are idempotent success;
port/protocol siblings remain outside the mutation set. Recovery uses the same
fingerprint gate and existing registration path.

Deletion failure retains `DataplaneError::LocalBackendDelete`, wrapped by
`DeregisterLocalBackendDispatchError::Dataplane`,
`ShimError::DeregisterLocalBackend`, then `ConvergenceError::Shim`. Registration
retains corresponding existing Insert/dispatch/Shim variants. The shim drains
after errors; runtime re-enqueues emitted work. That enqueue is not a local
retry guarantee: unchanged fingerprint suppresses another local action. This
is a retained limitation, not a newly reproduced failure, promised repair or
authorization for more machinery. BE10 here concerns successful local effects
during the demonstrated health transitions; no fault-recovery guarantee is added.

Scope is current one-Running-per-workload normal convergence. No multi-replica
selection/arbitration rule or cardinality guard; BE02 remains separate.
Membership disappearance, historical-listener GC, map drift repair,
changed-address reverse-key cleanup and restart replay are not added requirements.

## Lifecycle Gate Ownership

No allocation/Service lifecycle gate is added, removed or moved. This closes
the local consumer's bypass of an already-published health gate at its effect
boundary; it does not give that consumer authority to decide health.

| State/result | Owner and promise | Inputs | Explicitly unaffected |
|---|---|---|---|
| Backend eligibility | ServiceLifecycle alone decides the materialized bit | Existing terminal/readiness policy | Running, Stable, WorkloadLifecycle restart authority |
| Local rewrite availability | ServiceMapHydrator chooses existing local action; Dataplane completes effect | Current local non-mesh candidate and its authoritative bit | Row publication, lifecycle failure reporting, mesh/DNS acknowledgement |

Gate G-BE10 affects local rewrite availability only. True registers; false
removes on the existing fingerprint change. The awaited typed local effect
owns failure; it never becomes StartupProbeFailed, driver failure or restart.
Unresolvable listener/read errors keep existing hydration behavior. No new
timeout, cancellation or reconnect path. Normal shutdown drains the tick;
forced abort does not motivate this design. Counterexample: a still-Running
allocation loses its rewrite after readiness withdrawal and later consumer
dispatch, without changing Running or delaying lifecycle reporting. Late Pass
can restore readiness only if ServiceLifecycle publishes true; the local
consumer cannot override its terminal veto.

## Changed Assumptions

ADR-0101 revision 3 D6 says: “ServiceMapHydrator retains its listener-fact lookup,
materialized healthy bit, retry behavior and supported dataplane paths.” The
ruling says: “The kernel consumes a materialized healthy bit, not a new join
against policy storage.” That is not true of the local address-only map: it
has no bit. Revised assumption: the local consumer expresses authoritative
health through register versus deregister; remote/mesh behavior and health
authority remain unchanged. This is the precise D6 behavior exception; public
consumer APIs stay unchanged. No DISCUSS outcome or DISTILL assertion is weakened.

## Acceptance-designer handoff and verification limits

Acceptance designer retains all test ownership. Preserve BE10 seed `257221`,
real ProbeRunner stimulus, owner dispatch and expected-None assertion; after
implementation execute its previously unexecuted recovery suffix. Keep original
E09 seed/native evidence independent. Pure-function coverage should prove local
action selection and unchanged remote/View complements; source-local properties
carry exactly `/// CONTRACT_SHAPE: pure-function.`. Reuse existing port evidence
for dual-removal, absent-key idempotence and listener/protocol isolation instead
of creating a new scenario family. BE10 remains the seeded composed convergence
obligation. Existing Rust trait/exhaustive Action matching and dst-lint preserve
the pure-reconcile/effect boundary; no new architecture-enforcement project.

Native unhealthy routing is not yet reproduced. A kernel claim needs the
existing local direct-VIP production dispatch/attached-cgroup boundary, with
healthy control and new connection after withdrawal; Sim alone is insufficient.
No BPF program change or Tier-2 synthetic socket hook is proposed. Existing TCP
connections are not actively terminated; a map miss passes the original VIP
destination unchanged, not guaranteed denial. Existing deregistration also
removes the paired unconnected-UDP reply rewrite under its existing contract;
this does not promise frozen per-datagram behavior for existing sockets.
Examples, black-box expectations and in-process tests retain their boundaries.
Native execution and further test work belong to the authorized downstream
workflow, not this DESIGN dispatch.

The independent review approved the minimal action-selection mechanism's
necessity and exact reused signatures with no findings. Implementation GREEN,
native unhealthy-routing reproduction and the BE10 recovery suffix remain
pending downstream evidence. No production changes, tests, roadmap, DES events,
commits or review artifacts were authored in this DESIGN amendment/finalization.
