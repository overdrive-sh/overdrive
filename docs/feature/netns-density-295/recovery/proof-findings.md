# netns-density-295 — Correctness-Recovery Proof Findings

Results of the proof work in the correctness-recovery plan
(`.context/netns-density-295-correctness-recovery-plan.md`, §3). These are the
evidence inputs to the replacement DESIGN (plan §4). They record what today's
code does; they do not choose a design.

- Working tree for every run: HEAD `db3af70068fc14d87595f6f70ff1e1daf5ecc1e8`
  plus the uncommitted partial work from roadmap steps 02-03 / 04-01 / 04-02,
  except §3.5, which could only run against HEAD (see §3.5).
- Nothing is committed. How the RED proof tests land is a DISTILL decision.
- Metal kernel: `7.0.0-29-generic`, `systemd-detect-virt=none`,
  Cloud Hypervisor v53.0.

## Summary

| Plan § | Proof | Test | Verdict |
|---|---|---|---|
| 3.1 | CH TAP-fd handoff (non-persistent TAP) | `spike-scratch/netns-density-295-fd-tap/spike.py` | WORKS |
| 3.1 | CH TAP-fd handoff (persistent TAP, production lifecycle) | `spike-scratch/increment-y-netns-density-295-persistent-fd-tap/spike.py` | WORKS, two conditions |
| 3.2 | Node-wide attachment admission (seeded) | `crates/overdrive-sim/tests/netns_density_node_admission.rs` | RED |
| 3.3 | Complete shared-network supervisor (seeded) | `crates/overdrive-sim/tests/shared_network_supervisor_recovery_proof.rs` | RED |
| 3.4 | Fallible element cleanup | `crates/overdrive-control-plane/tests/integration/shared_element_cleanup_failure.rs` | RED |
| 3.5 | Boot-clear after process loss | `crates/overdrive-cli/tests/integration/serve_killed_restart_boot_clear.rs` | RED |
| 3.6 | CLI fail-stop | `crates/overdrive-cli/tests/integration/serve_lifetime_fail_stop.rs` | RED before the serve lifetime port, GREEN after |

## 3.1 — Cloud Hypervisor TAP-fd handoff

Two probes on native metal. Full evidence:

- Non-persistent TAP: `.context/netns-density-295-fd-tap-spike-findings.md`.
- Persistent TAP (the production-shaped lifecycle):
  `docs/feature/netns-density-295/spike/findings-persistent-fd-tap.md`.
  Promotion decision (DISCARD from promotion, hand off to DESIGN, probe retained):
  `docs/feature/netns-density-295/spike/wave-decisions.md`.

Result: a privileged owner can create the exact TAP, keep it administratively
down, and pass one queue fd to CH v53 via `--net fd=[N]` through the production
`prlimit`/`setpriv`/seccomp/landlock/cgroup chain. CH reaches the real guest
`READY` beacon with the TAP down, zero frames, and zero counters. CH never
raises the TAP in fd mode. The launcher's copy of the fd can be closed once CH
has started. The owner activates the TAP after interception is live, and all
captured frames are after the activation barrier.

Conditions for the persistent-TAP lifecycle:

1. **The launcher's `TUNSETIFF` must request `IFF_VNET_HDR`.** The production
   creator (`overdrive_netlink::create_persistent_tap`) does not set it. CH v53
   cannot add it to an attached fd (its `TUNSETIFF` returns `EEXIST`, which it
   ignores) and does not detect its absence. Without it, CH boots and exits 0
   but every guest frame carries a 12-byte zero prefix.
2. **The network owner must set the TAP down after VMM exit and before any
   relaunch attach.** CH exit leaves the persistent TAP admin-`UP`; the kernel
   raises carrier at every attach, so a relaunch attach queues host frames
   before any VMM runs.

Other facts DESIGN must account for:

- An attach fails with `EBUSY` while another queue fd is attached (including the
  creator's fd). `create_persistent_tap` already closes its fd.
- `IFF_VNET_HDR` is per-TAP and sticky: the last attacher sets it. Any other
  component that reattaches by name (e.g. `set_persistent_tap_owner` in
  `veth_provisioner.rs`) would clear it between launches and gets `EBUSY` while
  a VMM runs.
- A failed launch leaves the TAP persistent and down; the failure path must
  delete it or keep it for retry, explicitly.
- `RTM_DELLINK` succeeds even with a queue attached and yanks the holder's queue.
- Multiqueue needs one `IFF_MULTI_QUEUE` fd per queue pair; only one fd was
  proven.
- The current `VmNetworkAttachment` / `VmConfig` / `CloudHypervisorVmm` expose
  only named-TAP attachment. Adopting fd handoff is a DESIGN decision.

## 3.2 — Node-wide attachment admission

- Test: `node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads`,
  gated by `integration-tests`, in-process, no kernel state. The fill runs about
  190 s; `.config/nextest.toml` carries a timeout override for this binary.
- Command: `OVERDRIVE_ND295_ADMISSION_SEED=<seed> cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --no-capture -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)'`
- Seeds: `186055177052160001` and `295032`, identical verdicts.
- The test fills one node with 16,384 workloads through the real reconciler,
  scheduler, and action shim; only external ports are simulated.

Invariant asserted: the admitted population (in-flight + Running) is at most
16,384 at every lease acquisition.

| Check | Verdict | Observation |
|---|---|---|
| NA-1 placement at 16,384 Running | RED | admitted 16,385; scheduler given 0 workload-local rows |
| NA-2 placement with one admission in flight | RED | admitted 16,385; node-wide Running rows 16,383 |
| NA-4a crash replacement of a Service | RED | admitted 16,385 |
| NA-4b operator resume of a Service | RED | admitted 16,385 |
| NA-G peak over the run | RED | 16,385 |
| NA-5 release then admit (three variants) | GREEN | |
| NA-4a-L / NA-4b-L liveness | GREEN | |
| OBS-OVERLAP (observation) | — | peak lease population incl. retiring and replacement overlap 16,386 |

Findings (file:line evidence from the proof agent):

1. `schedule(...)` receives only the target workload's own rows
   (`workload_lifecycle.rs:1073-1079`). Actual-state hydration reads every row on
   the node and then filters to one workload (`:514-526`, filter at `:524`). The
   cap check (`scheduler.rs:107-114`) and `free_capacity` both see 0 or 1 rows.
2. The CPU/memory check has the same defect. On the baseline 4,000 mCPU / 8 GiB
   node, 16,384 workloads fit only at 0 mCPU and ≤512 KiB each, so the planned
   T1 density run is reachable today only because of this defect. (From reading
   the code, not a run; the test uses 0 mCPU / 256 KiB so it stays valid after a
   fix.) Out of #295 scope by user ruling of 2026-09-24; tracked by
   [GH #261](https://github.com/overdrive-sh/overdrive/issues/261).
3. `RestartAllocation` (crash replacement and operator resume,
   `workload_lifecycle.rs:920-1062`) never calls the scheduler.
4. There is no single linearization point for reservation today. Placement is
   pure code over a hydrated snapshot; up to 8 evaluations on different workloads
   run concurrently (`lib.rs:4629`, `:4655-4715`); no lock spans hydration
   through dispatch (`reconciler_runtime.rs:1477-1483` locks hydration only;
   dispatch at `:1589`). The only node-wide atomic claim is
   `GuestAddressPool::assign` (`guest_network.rs:454-507`), taken later in the
   action shim, and it enforces only the /16 pool size (65,533). Release is
   teardown followed by `release_action_plan` (`action_shim/mod.rs:1670-1679`).
   Even a node-wide count of Running rows is too late: an in-flight admission
   holds a lease but has no row yet.
5. Candidate reservation owners (no API proposed):
   - the scheduler — named by ADR-0121 / G-295-0, but pure code over a snapshot,
     so alone it cannot serialize concurrent evaluations or see in-flight
     admissions;
   - the address pool — the only atomic, node-wide claim covering every
     attachment class, but making it the cap owner contradicts G-295-0 ("never
     calls the pool at the cap") and ADR-0121 (pool exhaustion below the cap is
     infrastructure drift);
   - the evaluation broker — serializes per workload only.
6. Open question: ADR-0121 reserves pool headroom for overlap and cleanup
   residue but does not say whether a retiring attachment counts toward the cap,
   or bound the overlap.
7. Side observation (not asserted): operator resume of a stopped Job did nothing
   in the one evaluation observed; the action shim drops the restart when the
   previous allocation carries a terminal claim (`workload_lifecycle.rs:111-120`
   via `action_shim/mod.rs:2670-2674`). Out of #295 scope by user ruling of
   2026-09-24; tracked by
   [GH #301](https://github.com/overdrive-sh/overdrive/issues/301).

## 3.3 — Complete shared-network supervisor

- Test binary: `shared_network_supervisor_recovery_proof` (own binary; installs a
  global tracing subscriber), gated by `integration-tests`. It boots the real
  composition root (`run_server_with_obs_and_driver`) over Sim adapters; faults
  enter through `SimSharedGuestNetworkOwner` component slots plus two test-local
  owner faults (hanging audit, panicking audit).
- Command: `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --test shared_network_supervisor_recovery_proof --no-capture --no-fail-fast`
  (10 tests: 1 passed, 9 failed). Seeds `0x2953300000000001` and
  `0x295330005eed0002`; override with `OVERDRIVE_SUPERVISOR_PROOF_SEEDS`.

| Clause | Verdict |
|---|---|
| C0 healthy owner: admission open, no recovery | GREEN |
| C1 shared owner audited every second | RED — 0 audit calls in 10 s |
| C2 each of 12 components detected within 1 s, admission closes | RED — admission stays open, no owner call after the fault |
| C3 per-component TAP quiescence | UNREACHED |
| C4 repair through the owning component, single reopen | UNREACHED |
| C5 one typed fail-stop at 5 s / 20 attempts | UNREACHED |
| C6 unconfirmed quiescence → affected VMs killed, then fail-stop | UNREACHED |
| C7a healed owner cannot reopen after fail-stop | UNREACHED |
| C7b in-flight read-back cannot stretch the window or reopen | UNREACHED |
| C8 supervisor task loss → immediate fail-stop | UNREACHED |

UNREACHED means detection, the clause's precondition, never happened.

Findings:

- In this composition the supervisor has no mTLS worker: the worker is built only
  when `dataplane_override` is unset (`lib.rs:3734`), so the supervisor only
  waits for shutdown (`lib.rs:4338-4345`). That part is a test-composition
  artifact.
- The production defect is independent of that: `run_mtls_owner` audits only
  `mtls_worker.audit_shared_owner()` (`lib.rs:1394`). It never calls the shared
  owner's `audit_shared` or `converge_shared` at runtime; `converge_shared` runs
  only at boot (`lib.rs:3999`).
- The host `audit_shared` can report only Bridge, TcxLink, EndpointMap and
  CounterMap, so C1 and those four C2 cells are production-grounded REDs.
- Expected once detection exists (from reading the source, not a run): C3 TAP
  quiescence runs unconditionally, including for listener and DNS loss
  (`lib.rs:1406`); C4 never converges the shared switch; C6 returns an error
  giving `SupervisorFailed` without killing any VM cgroup; C7b has no deadline
  around owner calls; the unhealthy event carries no `cause`; a DNS task exit is
  replaced without closing admission.

Coverage gap (DESIGN input): IpRules, IpSets, LegF, LegC, Dns and the
listener/DNS task-loss classes cannot be driven through the real composition
root in-process. The mTLS worker is always built from the real host enforcement
and intercept adapters, and the DNS owner binds real `:53`; `ServerConfig` has no
way to inject `SimMtlsEnforcement` / `SimMtlsIntercept` or a DNS seam. User
ruling (2026-09-23): DESIGN pins this injection, as required ports composed at
the `serve` boundary like `kek`, not as `*_override: Option<…>` fields
(`dataplane_override` silently disables mTLS composition when set).

Contract conflicts for DESIGN:

1. S19's accepted call journal (`[TapSetDown]`, "no shared-switch audit")
   conflicts with ADR-0124 (each attempt is converge plus full read-back) and
   S-ND295-29 (attempts count only after both return). The proof asserts the
   ADR-0124 / S-ND295-29 version.
2. The host `audit_shared` reports guard loss as Bridge and never observes pins;
   RUN-295-B requires the exact pin and guard components.
3. Whether the shared-owner audit and recovery run when the mTLS worker is not
   composed. This proof and S-ND295-33 both assume they do.

## 3.4 — Fallible element cleanup

- Tests: `shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address`
  and `shared_element_cleanup_failure_readback_failed_retains_retirement_and_address`.
- Command: `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --no-fail-fast --no-capture -E 'test(shared_element_cleanup_failure)'`
  (2 run, 2 failed). Run on metal only because the Lima VM was unusable at the
  time; the test is in-process.
- The fault enters at the existing `MtlsIntercept` port used by the real
  `MtlsInterceptWorker`. No production seam was added. The real owners run
  unmodified: the worker's `begin_stop_alloc` and the shim's `StopAllocation`
  arm.

Both cases, with two Service ports (`2 + P` = 4 elements):

| Assertion | Verdict | Observed |
|---|---|---|
| Fault reached the lower effect; 4 elements left installed (witnesses) | GREEN | |
| Allocation stop reports failure | RED | `Ok(())` |
| Surfaced failure keeps the original cause | RED | `Ok(())` |
| Stop not reported converged while elements remain | RED | converged = true |
| Same-address successor refused until cleanup completes | RED | admitted, adopting the 4 leftover elements |
| No structural teardown after failed cleanup | RED | `TapDelete 100.95.0.2` |
| Address not reassigned while elements remain | RED | `100.95.0.2` reassigned |
| Row not terminal before cleanup converges | RED | `Terminated` |
| Retried stop re-runs removal of every element | RED | no removal attempted |
| After retry no predecessor element remains | RED | all 4 installed |

Findings: `SharedElementGuard::drop` (`mtls_intercept_port.rs`) restores its
refcounts, logs `health.mtls.shared_element_cleanup_failed`, and returns
nothing. The worker drops the elements and completes the drain
(`drop(drain.take_elements()); drain.complete();`), then returns `Ok(())`. The
registry record is removed, so a same-address registration is admitted.

## 3.5 — Boot-clear after process loss

- Test: `serve_killed_restart_boot_clear`, in-process `serve` on native metal.
- Command: `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --no-fail-fast --no-capture -E 'test(serve_killed_restart_boot_clear)'`
- Process loss is modelled by the serve lifetime port's killed mode (§3.6
  below); tests never spawn the `overdrive` binary.
- **It cannot run in the current working tree.** With the staged 04-01/04-02 work
  the TAP stays down until interception is live and CH fails raising it
  (`Enabling tap interface failed / Ioctl failed (35092) / Operation not
  permitted`), so no mesh VM reaches Running (gap 7). It ran against HEAD plus
  the serve lifetime port, in a temporary worktree since removed.

Residue read back before the reboot:

```
intercept_state=identity=3c1d494e5c743522 members={"inbound_destinations:100.95.0.2 . 18961", "managed_guest_ips:100.95.0.2", "outbound_sources:100.95.0.2"}
| ch_pids={46216} alive=[(46216, true)] | scope=…/alloc-boot-clear-residue-0.scope exists=true procs={46216}
| tap_present=true | tcx_link_pins={"ovd-tp-0002-ingress"} | vm_run_dir_present=true
```

With the old `abort_for_test` path the set members were deleted while the VM
survived (`members={} | ch_pids={51147} alive=[(51147, true)] … tap_present=true`),
confirming gap 6's second clause: the old fixture removed the residue it claimed
to test.

Reboot, reproduced in two runs:

| Assertion | Verdict | Observed |
|---|---|---|
| V0 restart recovers | RED | `shared mTLS rule/set convergence failed`; `health.startup.refused` reason `mtls.shared_owner` |
| V1 VMM reclamation precedes intercept recovery | GREEN | CH pid dead at 3,398 ms; scope gone at 3,419 ms |
| V2 boot clear runs | RED | 0 boot-two intercept batches |
| V3 sets empty after clear | RED | |
| V4 constant program converges | RED | |
| V5 admission opens | RED | |
| V6 no stale member survives | RED | 3 members never deleted |

Boot phases observed: `vm_reclamation` → `stale_sweep` (rebuilds the bridge
guard) → refusal. No production code calls the boot-clear adapter, and the test
does not call it by hand.

Killed mode fidelity: workload processes, cgroups, TAPs, the bridge, TCX pins and
nft objects are untouched. The mTLS worker is dropped on a thread that first
gives up every Linux capability, so its guard destructors run but the kernel
refuses them. One deviation from a real SIGKILL: `EbpfDataplane`'s destructor
still runs, so its SERVICE_MAP pin unlink and XDP detach happen.

## 3.6 — CLI fail-stop and the serve lifetime port

Before the port, `overdrive serve`'s lifetime logic was inline in
`crates/overdrive-cli/src/main.rs`: it waited only on `tokio::signal::ctrl_c()`,
never consumed the typed `ServeShutdownRequest`, had no ten-second bound, and
always exited 0. It was not testable in-process.

Serve lifetime port added (uncommitted; user-approved 2026-09-23 as "build it
with the test"), `overdrive_cli::commands::serve_lifetime`:

- `pub const FAIL_STOP_OUTER_BOUND: Duration` (10 s).
- `pub enum ServeSignal { Interrupt, Terminate, #[cfg(integration-tests)] Kill }`
  with `as_str()`. SIGTERM kept by user ruling 2026-09-23 (the feature-delta
  named only `ctrl_c()`).
- `pub trait ServeSignals: Send { fn recv(&mut self) -> impl Future<Output = ServeSignal> + Send; }`
  — cancellation-safe.
- `pub struct OsServeSignals` with `install() -> io::Result<Self>` — production
  binding for SIGINT + SIGTERM.
- `pub enum FailStopCleanup { DrainedBeforeExit, AbandonedAtExit }`.
- `pub enum ServeExit { Stopped { signal }, SharedGuestNetworkFailStop { request, cleanup }, #[cfg(integration-tests)] Killed }`
  with `exit_code()` → 0 / 1 / 137.
- `pub struct ServeLifetime<S>` — `new(signals: S, clock: Arc<dyn Clock>)` (both
  required); `run(self, handle: ServeHandle) -> Result<ServeExit, CliError>`
  selects the internal request before the operator signal; fail-stop races
  `handle.shutdown()` against `clock.sleep(10 s)`.
- `main.rs` builds `ServeLifetime::new(OsServeSignals::install()?, Arc::new(SystemClock))`
  and exits with `exit_code()`.
- Killed mode: `ServerHandle::kill_for_test` (test-gated, user-approved
  2026-09-23) replaces `ServeHandle::abort_for_test`. It uses `rustix`'s
  `thread` feature for the per-thread capability drop (`nix` has no `capset`
  wrapper); `rustix` was already a workspace dependency.
- Knock-on (not run): the `guest_stack_mtls_egress.rs` restart tests now use
  killed mode; `abrupt_shutdown_propagates_worker_failure_without_a_retry_capability`
  was deleted from `server_lifecycle.rs` because it asserted the removed
  teardown behaviour.

Test: `serve_lifetime_fail_stop` (3 tests), in-process `serve` on metal. The fault
is real: deleting the `ip overdrive-mtls` table makes the supervisor's own
recovery produce the request (`IpRules`, `RecoveryDeadlineExceeded`, 20
attempts, ~5.03 s). Determinism comes from a current-thread runtime, the injected
signal source, and a recording `SimClock`.

Command: `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --no-fail-fast --no-capture -E 'test(serve_lifetime_fail_stop)'`

| Assertion | Pre-port body | Port |
|---|---|---|
| Real fail-stop request reaches the owner | RED (`Stopped { Interrupt }`, exit 0) | GREEN |
| Internal request wins over a ready SIGINT | RED (SIGINT consumed) | GREEN |
| Outer bound is 10 s on the injected clock | RED (no sleep armed) | GREEN |
| Exit status 1 | RED | GREEN |
| Unobstructed shutdown drains before the bound | RED | GREEN |
| Shutdown abandoned at 10 s | RED | GREEN (`AbandonedAtExit`, exit 1) |
| SIGINT control: exit 0, no bound armed | GREEN | GREEN |

## Known quality blockers (not caused by the proofs)

- `cargo clippy -D warnings` fails in `overdrive-sim/src/invariants/*.rs`
  (8 × `large_futures`) and in the staged 04-01/04-02 control-plane hunks
  (`collapsible_if` in `action_shim/mod.rs`, `too_many_lines` in
  `guest_network.rs`).
- The §3.2, §3.3 and §3.4 tests compile only with the staged
  `GuestNetworkProvisioner::activate` method (their test doubles implement it);
  their verdicts do not depend on it.
