# DESIGN review — ADR-0097

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` (GH #257) |
| ADR | ADR-0097 — Exec network targets and startup-observation accounting |
| Review type | Independent DESIGN review |
| Review history | Iteration 1 — REJECTED; Iteration 2 — APPROVED |
| Current verdict | **APPROVED** |

## Scope and evidence

This review covers only the user-authorized allocation-network Exec HTTP/TCP target projection and the Startup ProbeResultRow accounting correction. It reviewed ADR-0097, ADR-0090, the amended feature delta, DESIGN decisions, DISTILL scenarios, architecture brief and C4, plus the production action-shim, Exec driver, ProbeRunner, observation-store, ServiceLifecycle, and runtime ViewStore paths.

The failure is reproduced on the intended product path. E10 drives the built binary with `http-exec-204.toml`, obtains `connection refused`, and renders 20 attempts despite `max_attempts = 3` ([E10 capture](../../../../verification/expectations/E10-vm-service-http-cross-driver-status/evidence/run.log), lines 22–25; [spec](../../../../examples/service-kind-vm-workloads/http-exec-204.toml), lines 9–23).

## Assessment

### D1 — allocation-network Exec target projection

**Accepted in substance.** The action shim assigns a network for VM or mTLS-composed workloads ([`action_shim/mod.rs:1184`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:1184)) and provisions/injects it before driver start ([`action_shim/mod.rs:1188`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:1188)). `ExecDriver` opens `spec.netns` and calls `setns(CLONE_NEWNET)` before `execve` ([`driver.rs:500`](../../../../crates/overdrive-worker/src/driver.rs:500), [`driver.rs:512`](../../../../crates/overdrive-worker/src/driver.rs:512), [`driver.rs:529`](../../../../crates/overdrive-worker/src/driver.rs:529)). The host-originated probe therefore must use the allocation's transit address, not host loopback.

The existing owner is correct: after the successful Running write the shim calls `on_alloc_running(&spec)` ([`action_shim/mod.rs:2198`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:2198)), and `ExecDriver` hands that same full spec to `ProbeRunner::start_alloc` ([`driver.rs:819`](../../../../crates/overdrive-worker/src/driver.rs:819)). The current private projection returns early for non-VM ([`probe_runner/mod.rs:452`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:452)-[`probe_runner/mod.rs:455`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:455)), then normalizes `None`/exact `0.0.0.0` to loopback ([`probe_runner/mod.rs:436`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:436)-[`probe_runner/mod.rs:446`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:446)); this is the proven mismatch.

ADR-0097 precisely limits the replacement to HTTP omission and exact textual IPv4 `0.0.0.0` plus TCP exact `0.0.0.0`. Explicit hosts (including explicit loopback), IPv6, DNS, ports, paths, and `ProbeMechanic::Exec` remain unchanged. Projection already applies only to the descriptor clone moved into the task ([`probe_runner/mod.rs:298`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:298)-[` :352`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:352)), so it does not rewrite intent or observations. Existing HTTP/TCP adapters already set the approved mark on each non-loopback socket before connect ([`http_prober.rs:94`](../../../../crates/overdrive-worker/src/probe_runner/http_prober.rs:94)-[` :116`](../../../../crates/overdrive-worker/src/probe_runner/http_prober.rs:116), [`tcp_prober.rs:91`](../../../../crates/overdrive-worker/src/probe_runner/tcp_prober.rs:91)-[` :123`](../../../../crates/overdrive-worker/src/probe_runner/tcp_prober.rs:123)). No new route, mark, adapter API, task owner, or lifecycle gate is required.

### D2 — count a stored Startup result once

The specified `ServiceAllocFact` field is the minimum transient hydration input and does not create public surface: the fact is an in-process hydration bundle ([`service_lifecycle.rs:48`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:48)-[`service_lifecycle.rs:70`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:70)). Hydration already selects the Startup/index-0 LWW winner ([`service_lifecycle.rs:845`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:845)-[`service_lifecycle.rs:905`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:905)). The LocalObservationStore accepts a replacement only when `last_observed_at_unix_ms` strictly increases ([`observation_backend.rs:1363`](../../../../crates/overdrive-store-local/src/observation_backend.rs:1363)-[`observation_backend.rs:1385`](../../../../crates/overdrive-store-local/src/observation_backend.rs:1385)), so the timestamp identifies the durable latest row. Pass may clear both maps; the terminal remains the current attempts/deadline/no-Pass predicate ([`service_lifecycle.rs:1213`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1213)-[`service_lifecycle.rs:1273`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1273)).

The following persisted-view recovery gap blocks approval.

## Findings

### R097-1 — Existing persisted views can retain an inflated counter without a timestamp

**Severity:** High. **Status:** Open, blocking.

`ServiceLifecycleView` persists both `startup_attempts_per_alloc` and `startup_last_fail_seen_at` ([`service_lifecycle.rs:272`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:272)-[`service_lifecycle.rs:300`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:300)). The runtime writes the next view before dispatch and reloads it after a process restart ([`reconciler_runtime.rs:1661`](../../../../crates/overdrive-control-plane/src/reconciler_runtime.rs:1661)-[`reconciler_runtime.rs:1665`](../../../../crates/overdrive-control-plane/src/reconciler_runtime.rs:1665)).

Current production calls `update_startup_attempts` with only status ([`service_lifecycle.rs:536`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:536)-[`service_lifecycle.rs:546`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:546)); that helper increments every Fail and never writes `startup_last_fail_seen_at` ([`service_lifecycle.rs:1180`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1180)-[`service_lifecycle.rs:1211`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1211)). A production restart can therefore hydrate `{ attempts: 20, last_fail_seen_at: None }` for E10's unchanged failure row. ADR-0097's proposed “timestamp differs” rule would regard the missing timestamp as new and increment to 21, contradicting its claim that old in-flight state is already-counted and that attempts become truthful.

**Necessary bounded correction:** Specify the legacy/unpaired branch, using only the two existing maps and the proposed fact: if a Fail has an existing counter but no recorded timestamp, replace the legacy counter with one and record that row's timestamp; do not increment. Timestamp-present Fail, Pass, and None remain as proposed. Add a production-runtime recovery regression that persists this old-shape view, hydrates the unchanged row after restart, and observes one attempt; then prove three strictly later stored rows render exactly three after restart. This is not new recovery machinery, storage, or API.

### R097-2 — The architecture brief still states the superseded Exec loopback rule

**Severity:** Medium. **Status:** Open, blocking SSOT correction.

The proposed ADR and feature artifacts say allocation-network Exec defaults use transit `workload_addr`, but the architecture SSOT still states that the same omitted HTTP host / `0.0.0.0` declaration for Exec/process preserves loopback ([`brief.md:10411`](../../../../docs/product/architecture/brief.md:10411)-[`brief.md:10418`](../../../../docs/product/architecture/brief.md:10418)). Its ADR index repeats that obsolete rule ([`brief.md:3500`](../../../../docs/product/architecture/brief.md:3500)). The feature delta's statement that the brief/C4 reconciled with zero contradictions is consequently false ([`feature-delta.md:695`](../feature-delta.md:695)-[`feature-delta.md:700`](../feature-delta.md:700)).

**Necessary bounded correction:** Amend the brief Target ownership paragraph and its ADR-0090 summary with the exact ADR-0097 three-way target rule and one-count Startup observation rule. Preserve explicit targets, VM `None`, lifecycle ownership, and marked-adapter behavior. No C4 relationship change is needed because no component, owner, or persistence boundary changes. Update the reconciliation claim after those SSOT edits.

## Required verification after remediation

1. Projection tests cover HTTP omission, HTTP/TCP exact `0.0.0.0`, explicit hosts including explicit loopback, VM, Exec `Some(workload_addr)`, Exec `None`, and unchanged stored descriptors.
2. Hydration/reconciler/runtime tests carry the Startup/index-0 timestamp, deduplicate unchanged rows, count strictly later LWW rows once, clear both maps on Pass, and prove R097-1 restart recovery.
3. E10 re-runs every Exec/VM × 204/302/404/503 built-product cell: both 204 cells Stable; 302/404/503 retain current status/redirect/body behavior; failures render three attempts; the 503 sentinel remains absent from every operator-visible surface.

## Disposition and verdict

| Item | Disposition |
| --- | --- |
| D1 target correction | Accepted in substance; retain it unchanged. |
| D2 fact/API shape | Accepted in substance; add only R097-1's legacy-view rule. |
| R097-1 | Open, blocking. |
| R097-2 | Open, blocking. |

**REJECTED.** The target correction is grounded and appropriately small, but a persisted current-version view can preserve the exact inflated count the amendment promises to correct, and the architecture brief still advertises the superseded loopback contract. Remediate only R097-1 and R097-2 and submit this ADR for iteration 2.

## Iteration 2 — re-review

### R097-1 — Resolved: legacy unpaired views converge to the current observation

ADR-0097 now makes the previously missing state explicit: a `Fail` with an
existing counter and no `startup_last_fail_seen_at` entry replaces that
counter with `1` and records the current row timestamp, without incrementing
the legacy value ([ADR-0097:106](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:106)-[110](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:110)). The ordinary rule then increments only where the hydrated timestamp differs
from the last counted value ([ADR-0097:111](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:111)-[116](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:116)); `Pass` removes both maps and `None` changes neither ([103](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:103)-[105](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:105)).

This is a source-grounded recovery rule. The current production helper records
only the counter for a failed status and never writes the timestamp
([`service_lifecycle.rs:1196`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1196)-[`service_lifecycle.rs:1211`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:1211)), while the serde view persists both maps
([`service_lifecycle.rs:272`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:272)-[`service_lifecycle.rs:300`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:300)) and runtime recovery reloads the durable next view
([`reconciler_runtime.rs:1661`](../../../../crates/overdrive-control-plane/src/reconciler_runtime.rs:1661)-[`reconciler_runtime.rs:1665`](../../../../crates/overdrive-control-plane/src/reconciler_runtime.rs:1665)). The amendment's required transient fact is exactly the latest stored Startup/index-0 timestamp, not a new persisted seam
([ADR-0097:88](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:88)-[99](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:99)). Its LWW identity is valid: an incoming result replaces the stored row only with a strictly greater `last_observed_at_unix_ms`
([`observation_backend.rs:1363`](../../../../crates/overdrive-store-local/src/observation_backend.rs:1363)-[`observation_backend.rs:1385`](../../../../crates/overdrive-store-local/src/observation_backend.rs:1385)).

The evidence obligation now exercises the reachable persisted legacy shape
`{ attempts: 20, last_fail_seen_at: absent }` through restart/hydration,
requires its replacement by one current observation, then requires Pass cleanup
and exactly three strictly later LWW failures ([ADR-0097:163](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:163)-[172](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md:172)). This closes the original 20-to-21 defect without a migration, storage change, public API, or revised terminal predicate.

### R097-2 — Resolved: target-policy SSOT is reconciled

The architecture brief's ADR summary now declares the exact three-way policy:
VM omission/wildcard uses guest `workload_addr`, allocation-network
Exec/process omission/wildcard uses transit `workload_addr`, and unnetworked
Exec/process defaults remain loopback ([`brief.md:3500`](../../../../docs/product/architecture/brief.md:3500)). Its target-ownership section states the same rule and preserves declared descriptors plus the existing marked-adapter behavior
([`brief.md:10415`](../../../../docs/product/architecture/brief.md:10415)-[`brief.md:10421`](../../../../docs/product/architecture/brief.md:10421)). The adjacent lifecycle paragraph also now specifies one-count LWW accounting and legacy-counter normalization
([`brief.md:10423`](../../../../docs/product/architecture/brief.md:10423)-[`brief.md:10429`](../../../../docs/product/architecture/brief.md:10429)).

The feature delta repeats the same bounded DDD-12 contract
([`feature-delta.md:433`](../feature-delta.md:433)-[`feature-delta.md:439`](../feature-delta.md:439)), its DISTILL obligation requires target preservation, legacy-view recovery, and truthful three-attempt E10 failures
([`feature-delta.md:693`](../feature-delta.md:693)), and its reconciliation result correctly records no remaining documented contradiction while retaining C4 unchanged because no component, owner, or persistence boundary changes
([`feature-delta.md:696`](../feature-delta.md:696)-[`feature-delta.md:703`](../feature-delta.md:703)). The design decision table and DISTILL S-SVM-12 likewise align an allocation-network process default with its provisioned allocation address
([`wave-decisions.md:35`](wave-decisions.md:35)-[`wave-decisions.md:45`](wave-decisions.md:45), [`test-scenarios.md:261`](../distill/test-scenarios.md:261)-[`test-scenarios.md:270`](../distill/test-scenarios.md:270)).

This remains consistent with production composition. The action shim assigns a
network to an Exec allocation only when mTLS is composed, but then provisions
the allocation network before driver start ([`action_shim/mod.rs:1184`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:1184)-[`action_shim/mod.rs:1199`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:1199)); `ExecDriver` enters the supplied netns before spawning its process
([`driver.rs:500`](../../../../crates/overdrive-worker/src/driver.rs:500)-[`driver.rs:533`](../../../../crates/overdrive-worker/src/driver.rs:533)). The existing registration path has the full `AllocationSpec` and presently leaves non-VM defaults at loopback
([`driver.rs:819`](../../../../crates/overdrive-worker/src/driver.rs:819)-[`driver.rs:831`](../../../../crates/overdrive-worker/src/driver.rs:831), [`probe_runner/mod.rs:298`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:298)-[`probe_runner/mod.rs:320`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:320), [`probe_runner/mod.rs:436`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:436)-[`probe_runner/mod.rs:479`](../../../../crates/overdrive-worker/src/probe_runner/mod.rs:479)). ADR-0097's proposed projection is therefore both necessary and limited to the proven mismatch.

## Iteration 2 verification and verdict

No implementation, test, runner, proposal, or roadmap was modified in this
re-review. Source inspection confirms that the remediated rule uses only the
existing persisted maps plus the one explicitly named transient fact, and that
the updated SSOT matches the reachable allocation-network Exec path.

| Item | Iteration 2 disposition |
| --- | --- |
| D1 target correction | Approved. |
| D2 transient fact and one-count rule | Approved. |
| R097-1 | Resolved. |
| R097-2 | Resolved. |

**APPROVED.**
