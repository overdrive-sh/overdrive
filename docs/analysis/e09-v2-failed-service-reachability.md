# E09 v2 failed-Service reachability diagnosis

2026-09-07. Diagnosis only; no production, example, expectation, or design change.

## Result

The negative peer-VM result is independently reproduced: both peers exited 43 after their own Services reported startup failure. Traffic reached **same-allocation-ID replacement attempts**, not the original failed VM instances. The existing `ServiceLifecycle` terminal veto still applies to those IDs, but its last-emitted backend fingerprint suppresses repair after `BackendDiscoveryBridge` removes and reintroduces membership with `healthy: true`.

This is a demonstrated implementation convergence bug. It is not evidence that the two-owner architecture must be replaced. A separate design-document assumption is wrong: ADR-0096 says replacements have distinct allocation IDs; existing production and ADR-0099 explicitly reuse the allocation ID. No new persistence, attempt-fencing, owner, or public API is justified by this diagnosis.

## Evidence identity and bounded reproduction

The earlier worker's smoke18 output was deleted. Its exit43 report is inherited chat evidence, not retained proof. This investigation's independent evidence is retained at `.context/e09-v2-reachability-20260907/`:

- `native-c.sh`: exact bounded diagnostic wrapper.
- `native-c-output.log`: verbatim canonical invocation, preflight, process identity, reports, cleanup failure, exit1.
- `native-c/`: timestamped per-workload descriptions, concurrent failure-Service descriptions while peer Jobs run, materialized cases/specs, measurements, control-plane log, source SHA256 identities, and per-thread syscall traces.
- `sim-output.log`: retained seeded invariant failure.
- `cleanup-c.sh`, `cleanup-c-output.log`: exact separately leased, task-owned remainder cleanup.

Native C used `ubuntu@151.115.99.251`, canonical `/run/lock/overdrive-metal-shared.lock`, lease token `da15481d9b45c25d3ccc63cf`. Preflight passed. Source checkpoint was `480501fd03df4b0e749e026246312b3f3099ae4e` plus dirty v2 scripts and diagnostic spike; source digest `719e07343dc3a5bbac9d82c3240c38d387ff1e58a78ef78c111318b0793392f9`. Product SHA256: `db5da18cb80fe4f5459998c5073fa76a35ade440b74279b1f562c509fab77e3f`. V2 runner SHA256: `e64927a7492acf976bc17dc16c8ced0bcb81a9cc16d84e8f186b146a5619a3bb`.

Invocation shape (the verbatim expanded command is in the output capture):

```sh
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
cargo xtask metal run -- bash -c '<contents of native-c.sh>'
```

The diagnostic sources unchanged v2 functions without final command dispatch, pins `EXAMPLE_DIR` because the source is `/dev/fd`, and calls only `if run_cohort 1 1 2; then ...`. It retains the canonical caller's conditional/errexit context. It builds once, prepares once, and starts one control-plane process. It does **not** execute the full100 suite: materialization/reporting still enumerate100 inputs, but only two paired trials execute. Added observations are timestamped describe copies and `strace -ff -ttt -yy -s 192 -e trace=connect,bind,kill,wait4,waitid` attached to the owned process. No payload/private-key syscall capture was requested. Tracing can perturb timing; no precise latency claim follows.

Two prior diagnostic attempts are explicitly invalid: A inherited `umask 077`, making root-owned VM staging directories inaccessible to the confined VMM UID; B called `run_cohort` directly, enabling errexit unlike the canonical conditional caller. Both stopped before the negative traffic interval. Captures are retained under `invalid-umask/` and `invalid-errexit/`; neither supports this root-cause claim.

## Observed native chronology

Times below are UTC on September7. One unchanged control plane: PID `2298755`, `/proc` start ticks `210349704`, executable `/home/ubuntu/overdrive/target/debug/overdrive`.

| Time | Observed event |
|---|---|
| 21:48:10.340 | Control-plane startup log. |
| 21:48:18.548 / 23.754 | Healthy c001/c002 VM EXEC released after mTLS install. Both Services become Stable with guest TCP18081 pass. |
| 21:48:32.600 / 37.850 | Healthy peer EXEC released; both Jobs subsequently report Succeeded/exit0. |
| 21:48:51.919 / 21:49:04.406 | Healthy Service descriptions show Terminated, reason stopped. Own healthy Service is stopped before its failure Service is deployed. |
| 21:49:08.622 / 14.473 | Failure c001/c002 original VM EXEC released. |
| 21:49:16 / 23 | Failure deploy commands exit1: startup probe0 failed after3 attempts, connection refused; no Stable. |
| 21:49:16.314 / 23.530 | Same-ID beacon binds fail EADDRINUSE for c001/c002. Traces then observe original VMMs reaped with SIGKILL. |
| 21:49:22.988 / 29.399 | Product logs same-ID recovery from Failed, restart_count1. |
| 21:49:23.051 / 29.465 | Replacement EXEC released after intercept install. |
| 21:49:34.516 / 39.993 | Negative peer EXEC released. |
| 21:49:34.531–34.559 / 40.004–40.018 | Control-plane threads connect to the respective replacement backend `10.99.128.2:18081` / `10.99.128.6:18081`. |
| 21:49:35.087 / 40.176 | Peer descriptions first expose crashed exit43. Later descriptions show Failed/43. Concurrent own-Service descriptions remain Running/restarts1 with continuing startup18999 failures. |

Exact workload IDs are `service-e09-v2-c001-f` and `service-e09-v2-c002-f`; allocation IDs are respectively `alloc-service-e09-v2-c001-f-0` and `alloc-service-e09-v2-c002-f-0`. Peer IDs append `-client`; their checked-in-derived specs address the matching `<failure-service>.svc.overdrive.local`, port18081, `expect-unreachable`, `raw`. No readiness or liveness probes are declared; the server binds18081 while startup probes target unbound18999. Failure c001 spec digest is `4027e17f8a3741241426aa01de5f02432d5cc58593d122d36ddbcd4720bce5c7`.

The public descriptions report Service VIPs10.96.0.4/c001 and10.96.0.5/c002. These are **not** the workload-name mesh frontend IPs: the latter use10.98.0.0/16. This capture verifies the exact requested DNS names and selected backend destinations, but does not retain the numerical DNS answers. Do not relabel the Service VIP as the mesh frontend. Source maps the shared workload-name allocator binding to the Service backend row by backend SPIFFE workload identity (`mtls_resolve_adapter.rs:457`), then selects that Service's healthy backend (`:491`, `:540`).

Exit43 is a source-identified payload witness, not a raw packet capture: `examples/service-kind-vm-workloads/client.rs:34` requires successful request write/read-to-end and the exact `SVM-E08-GUEST-OK` byte substring; `:70` exits43 only for that result in negative mode. The code does not require the entire response to equal that string. The native current-view/fingerprint values are not dumped; the precise internal convergence mechanism below is separately observed in the seeded in-process reproduction.

## Seeded invariant and complete owner path

`crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs` is an intentionally RED diagnostic, with per-test `CONTRACT_SHAPE: bounded-change` and printed seed257209. It failed twice through the same existing composition:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests,overdrive-control-plane/integration-tests \
  --test e09_v2_failed_service_reachability_spike --no-capture
```

The fixture uses validated Service input, existing intent/allocator ports, registered WorkloadLifecycle/ServiceLifecycle/BackendDiscoveryBridge/VmReclamation, production action dispatch, production ProbeRunner, SimObservationStore, SimClock, and an existing Driver-port adapter forwarding the normal probe hooks. It inserts no allocation/probe/backend observation, terminal marker, or memoized fingerprint. The smaller driver is Exec/SimDriver; it proves the driver-independent backend consequence, not VM kernel teardown. Native C establishes the same-ID VM triggering path. VmReclamation is registered but has no VM host artifacts in this smaller fixture.

Production-authored sequence: initial Running/no-readiness backend true (control); three failed probes; deciding ServiceLifecycle tick writes false and FinalizeFailed; bridge sees Failed and writes empty membership; WorkloadLifecycle restarts the same ID; bridge sees Running and inserts healthy=true; six subsequent ServiceLifecycle ticks fail to repair it. The runtime view still contains that ID in `terminal_announced`. Assertion fails with `seed=257209: stored backend regained eligibility despite unchanged ServiceLifecycle terminal veto`.

The causal path is:

1. Public deploy persists Service intent; production WorkloadLifecycle starts the VM, action shim accepts Running, installs intercept and calls `on_alloc_running`; VmDriver forwards to ProbeRunner. The native failure spec and failed18999 observations establish the real stimulus.
2. `service_lifecycle.rs:660`–`:704` records the terminal decision and constructs false backend write before FinalizeFailed. Action shim dispatches the action vector serially and continues on individual errors (`action_shim/mod.rs:927`). No injected write error is required here.
3. True terminal FinalizeFailed stops mTLS, tears down/releases structural network, stops probes, writes the terminal observation, and releases supervision (`action_shim/mod.rs:1767`–`:1795`). This is not an assertion that FinalizeFailed synchronously kills the VMM.
4. VmReclamation remains the real stranded-VM owner: terminal + unclaimed permits DiscardStrandedArtifacts; claimed live sessions are protected (`vm_reclamation.rs:155`, `:242`, `:308`). Its resync is30s. Existing failed-start cleanup and ordinary reclamation are distinct; neither missing stop-before-release nor watcher ownership is needed to explain the health defect.
5. WorkloadLifecycle alone chooses same-ID RestartAllocation (`workload_lifecycle.rs:1446`, existing budget/backoff). Restart dispatch awaits prior-driver stop or typed NotFound, then awaited restart cleanup before provisioning (`action_shim/mod.rs:2272`). ADR0099's accepted Running-write gate precedes effects. ADR0100's old watcher session guard is retained. Native binds/recovery prove a failed intermediate attempt and a subsequent live replacement, not resurrection of the original VMM.
6. Bridge hydrates Running-only membership and carries an observed matching backend's health; if absent it defaults true (`backend_discovery_bridge.rs:376`–`:396`, `:525`). Empty Failed membership erases the value it would otherwise carry.
7. ServiceLifecycle still computes false because the same ID remains terminal-announced (`service_lifecycle.rs:525`, `:1130`, `:1188`). However `:1146`–`:1150` compares the desired fingerprint with its **last emitted** fingerprint, not the actual current backend contents. They match, so no corrective write occurs. Hydration only supplies the prior backend-row timestamp (`:954`), not current health for this comparison.
8. Mesh resolution consumes the resulting healthy row; `first_healthy_backend_for` does enforce its stored healthy bit. It cannot reconstruct the ServiceLifecycle veto. Native traffic reaches the replacements while fresh startup failures continue.

There is no forced task abort in the seeded trigger. The real convergence task drains broker work and awaits each full convergence tick before its shutdown boundary (`overdrive-control-plane/src/lib.rs:3332`–`:3402`). The failure survives repeated ordinary service ticks; it is not justified solely by an imagined cancellation window.

## Contract classification and falsified alternatives

ADR0096 D1/D3 requires the terminal eligibility veto and existing health owner; it does not require memoizing away a correction after another existing writer changes membership. Its independent review ends APPROVED (`docs/feature/service-kind-vm-workloads/design/review-adr-0096.md:200`), although the ADR and feature-delta still retain Proposed wording. Record that metadata inconsistency rather than silently changing status.

There is also a substantive premise mismatch: ADR0096 D3/retained counterexamples say a fresh replacement has a distinct ID; production and ADR0099's gate table explicitly use a replacement attempt under the same ID. The invariant diagnoses the current owner's **unchanged terminal veto**, not a proposed policy that every future attempt must remain failed forever. Merely observing Running/restart_count1 does not establish eligibility; neither does the prior deploy exit1 alone establish permanent ineligibility.

The evidence falsifies wrong-case selection (peer spec names its own failure Service), traffic before original failure (recovery and replacement EXEC precede peer traffic), no actual payload witness (exit43 has the exact-byte detection branch), original-VMM survival as the traffic source (original reaped before replacement EXEC), and missing current health evaluation (same-ID terminal veto still computes false in the seeded production owner). No probe pass or new allocation identity makes this backend eligible under that current predicate. Store failure, broker loss, new persistence, and a missing VmReclamation owner are unnecessary hypotheses.

Minimum justified correction direction: restore convergence of the existing ServiceLifecycle-owned desired backend health against the **current observed row**, so a bridge membership rewrite cannot permanently defeat an unchanged veto. Keep initial nonterminal no-readiness compatibility, existing restart ownership, and accepted public API shape. Exact implementation shape must be pinned before any public state/API change; this diagnosis does not implement one. Reconcile the distinct-ID prose with established same-ID behavior, without inventing a new restart policy.

## Harness findings and cleanup disposition

Two bounded harness findings are separate from the product reachability bug. `all_service_alloc_ids` (`run-example.sh:351`) never leaves the allocation table, so the later `Addresses` line contributes a bogus trailing-colon allocation ID; retained `healthy-allocs` demonstrates this independently. `capture_case_resources` can return the last missing-path predicate status, which explains invalid direct-call diagnostic B. Neither value determines the peer's DNS target.

On reproduced exit43, `wait_for_job_succeeded` polls120s, while `stop_workload` only accepts Terminated, not a failed terminal Job. The cohort's180s worker deadline then kills workers before they dispose the failure Services. Native C's final cleanup reports remaining owned resources and returns1: this is not a clean expectation run or a100-pair completion. Preserve this failure for the v2 script owner rather than weakening the reachability assertion.

After ordinary suite cleanup, both VMM PIDs were gone but two empty scopes, task-owned netns/veth/intercept rules, and run directories remained. Exact audited handles3050–3055, netns0000/0001, veth0000/0001, and c001-f/c002-f scopes were removed under a fresh canonical cleanup lease `4a223529f72659b1c0e1e637`. Run directories and netns configuration were moved into the protected remote capture instead of discarded. Post-audit: lease free, no owned control-plane process/scopes/run directories/netns, BPF net attachments empty; unrelated pre-existing coinflip cgroups and veth-bk/veth-cli preserved. No native command remains running.
