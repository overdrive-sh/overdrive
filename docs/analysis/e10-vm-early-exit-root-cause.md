# E10 VM HTTP302 early exit — bounded causal diagnosis

Date: 2026-09-07. Native diagnosis with an authorized in-process seeded
regression extension. No production, design, DES, or expectation-predicate changes.

## Result

The reproduced early VMM death is a **platform-issued cgroup kill during a
failed same-allocation-ID restart**, not demonstrated guest failure, HTTP302
causing the guest to exit, or the first ordinary VmReclamation sweep.

After Service startup failure releases VM ending authorship, the original
VMM and its beacon socket pathname remain. WorkloadLifecycle selects a
replacement attempt under the same allocation ID. The replacement's beacon
`bind` fails with `EADDRINUSE`; its start-failure unwind kills the allocation's
existing cgroup, which still contains the original VMM. The original exit
watcher then obtains ending authorship while the replacement's `Starting`
claim is present and its crash observation wins the current-row write. The
replacement never creates a second VMM or reaches Running.

The **native host-effect sequence is directly captured**. The private
claim/writer interleaving of that historical run remains source-supported
inference. A newly executed seed-257205 regression now demonstrates a reachable
second ending through the registered production owners and real VmDriver with
simulated host resources; its no-restart reclamation control passes. The fresh
simulation evidence and its boundaries are recorded below, separately from the
inherited native captures. No remedy is selected or implemented.

## Diagnostic triple

- **Hypothesis:** an authorized same-ID restart collides with the first
  attempt's still-present beacon pathname, and its failed-start cleanup kills
  the first VMM before normal reclamation.
- **Prediction:** successful first beacon bind/EXEC; startup failure;
  second bind of the identical pathname returns `EADDRINUSE`; an Overdrive
  write to that allocation's `cgroup.kill` immediately precedes SIGKILL of
  the first VMM; no second VMM starts. An HTTP204 control has no second-bind
  failure and ends through the operator-stop path.
- **Falsification:** first VMM exits before that failed bind/kill, a different
  process/scope is killed, or there is no same-ID bind collision. None occurred
  in the captured HTTP302 run. The HTTP204 control followed its predicted path.

## Evidence identity and reproduction

Native host: `ubuntu@151.115.99.251`, exclusively leased through
`cargo xtask metal run --` after the E09 owner explicitly handed it off.
The canonical native/KVM preflight passed. Both runs used the same rebuilt
default-feature binary; no test harness was the system under test.

- HEAD: `75d049ded61aa814bf4023134f5f1d035988518f`, dirty workspace.
- Binary SHA-256: `0cec3e31115acc7fe2973f5a3378e4cf2c3d95d0a47706169652e7f167a56a3e`.
- `vm_driver.rs`: `e1526b750076e96bb0b540e5f5b44dc4c3886706db00aa2804d93e4e154ed0d0`.
- `action_shim/mod.rs`: `5a99b5b6eb2f13715bd2e08e3582cfe3c4d738fb09369a85b971ce16a1d0df56`.
- HTTP302 capture root **F**: `/tmp/e10-vm-early-exit.P9WDul` on that host.
- HTTP204 control root **C**: `/tmp/e10-vm-early-exit-control.W7tiu5` on that host.

The existing product example was executed unchanged with
`SVM_E08_SKIP_BUILD=1`, a unique `SVM_E08_OWNERSHIP_TOKEN`, and
`SVM_E08_CASE_CAPTURE_DIR` under the diagnostic root. The first invocation
built the binary before tracing; the control used `metal run --no-sync`, whose
source-identity check passed. Core execution commands:

```sh
strace -ff -ttt -yy -s 512 \
  -e trace=%process,bind,shutdown,openat,unlinkat,rmdir,close,write,writev \
  -e raw=write,writev -o "$diag_dir/syscalls" \
  examples/service-kind-vm-workloads/run-example.sh run case \
  http-vm-302.toml service-vm-http-302 startup-failed

# Same command/options; only the existing case changes for the control:
# http-vm-204.toml service-vm-http-204 stable
```

HTTP302 also ran beneath `bpftrace -c` with `sched_process_exec` tracking
`/usr/local/bin/cloud-hypervisor`, fd-2 `sys_enter_write` capture for those
TGIDs, and `sched_process_exit` reporting `curtask->exit_code`.
All other syscall write buffers were printed raw, not decoded, to avoid
capturing credentials. The stderr probe used `str(buf, count)`, which truncates
the last byte of non-NUL-terminated writes; it is **not a byte-exact/full stderr
record**. It captured fragments of the existing-TAP launch warning, no positive
fatal diagnostic. No absence-of-fatal-message claim depends on it.

HTTP302's **example exited 1**, at `F/syscalls.962468:1133`; its deploy also
exited 1 for the expected startup failure. The outer bpftrace/xtask returned 0:
that wrapper result is not the example verdict. HTTP204's direct strace/xtask
returned 0. Both examples performed their existing owned cleanup and recorded
all eight zero deltas. No manual remote cleanup or competing E09 access occurred.

## Actual chronology

Times below are UTC on 2026-09-07; `strace -ttt` records the equivalent UNIX
seconds. Counters 37/38 belong to this traced run, not historical counters 35/36.

| Time | Observation and exact retained reference |
| --- | --- |
| 14:15:47.713709 | Fresh serve listening; `F/case/serve.log:4`. |
| 14:15:48.435655 | First bind of `/run/overdrive/vm/alloc-service-vm-http-302-0/vsock_1234` succeeds; `F/syscalls.963797:8`. |
| 14:15:48.484246 | Original VMM PID 963916 execs Cloud Hypervisor; `F/syscalls.963916:244`. The two earlier bpftrace VMM PIDs are probe processes, not allocation attempts. |
| 14:15:54.718981 | Accepted READY path has dropped its listener FD, but not unlinked the socket pathname; `F/syscalls.963796:4`. |
| 14:15:54.784789 | EXEC released after intercept installation; `F/case/serve.log:6`. |
| 14:15:57.875 | Last startup observation reports HTTP302 failure; exact observation milliseconds `1788790557875`, `F/case/service-describe.log:12`. The later describe reports Failed(37), reason Started (`:5–6`); the streaming result names StartupProbeFailed. |
| 14:15:58.069123 | Overdrive shuts the accepted UNIX stream's write half; `F/syscalls.963797:99`. |
| 14:15:58.312007 | Example launches `overdrive job stop service-vm-http-302`; `F/syscalls.964049:26`. This is a command-process timestamp, **not** the stop-intent commit time. |
| 14:15:58.327754 | Second bind of the identical beacon pathname fails `EADDRINUSE`; `F/syscalls.963796:126`. No second VMM exec follows. |
| 14:15:58.328028–.328254 | Overdrive opens, successfully writes two bytes to, then closes the original allocation's `cgroup.kill`; `F/syscalls.963806:149–151`. `CgroupManager::cgroup_kill` supplies `1\n`. |
| 14:15:58.328278 onward | Original VMM threads die by SIGKILL; e.g. `F/syscalls.963921:28132`. Main VMM exit is recorded at .351207 (`F/syscalls.963916:255`). Independent bpftrace records exit code 9 for its process/thread group. |
| 14:15:58.329459–.340321 | Failed-start cleanup unlinks the beacon pathname and removes the run directory; `F/syscalls.963932:22–24`. |
| 14:15:58.363731 | Overdrive's child wait confirms `WIFSIGNALED && WTERMSIG == SIGKILL` for PID 963916; `F/syscalls.963795:79`. |
| 14:15:58.364199 | Exit watcher accounting read finds the allocation's `memory.events` already absent; `F/syscalls.963806:152`. This does not establish OOM; the kill is independently identified above. |
| 14:15:58.404108 | Counter-38 proposal loses to stored counter 38, writer local; `F/case/serve.log:7`. The stopped describe retains Failed(38), crashed, exit/signal None; `F/case/service-vm-http-302-stopped.log:5–6`. |
| 14:16:17.816973 | Later host sweep tries the already-absent `cgroup.kill` and gets ENOENT; `F/syscalls.963798:297`. It cannot cause the VMM death roughly 19 seconds earlier. |

The historical eight-line capture at `/tmp/svm-e10-matrix/e10-vm-302` has
the same observable failure shape, but no retained process wait status or
timestamped stop request. The new trace reproduces that shape and identifies
a concrete cause; it cannot retroactively prove each unrecorded event in the
12:05 historical execution.

## Complete owner path and causal depth

1. `spawn_convergence_loop` awaits registered reconciliation/action dispatch
   serially (`crates/overdrive-control-plane/src/lib.rs:3377`); shutdown is
   checked after the awaited loop body (`:3400`). Ordinary VmReclamation is
   armed one period out (`:3349`, `vm_reclamation.rs:242`). An operator request
   does not cancel a restart already executing in that frame.
2. ServiceLifecycle owns startup failure, not VMM termination. Its terminal
   shim path tears down allocation networking, stops probes, publishes the
   terminal row, and releases ending authorship (`action_shim/mod.rs:1766–1795`).
   `VmDriver::release_supervision` removes the entry (`vm_driver.rs:1870`);
   BeaconWriter drop shuts its writer (`:739–758`). This does not unlink the
   beacon pathname or call VMM terminate. Cloud Hypervisor uses
   `kill_on_drop(false)` (`overdrive-host/src/vmm.rs:291`).
3. WorkloadLifecycle sees a restartable Service row and emits its existing
   same-ID `RestartAllocation` (`workload_lifecycle.rs:951`, `:1446`, `:1468`).
   The shim accepts `Driver::stop -> NotFound` as absence before replacement
   provisioning (`action_shim/mod.rs:2271–2302`). Here that proves absence of
   the driver entry, not absence of the original VMM or pathname.
4. `VmDriver::provision_vmm` installs **Starting** before binding (`:1138–1154`).
   The allocation-derived run directory/path is reused; the failed bind invokes
   cleanup with `Some(&scope)` although this attempt has not created its cgroup
   or VMM (`:1198–1215`). `attempt_start_cleanup` calls cgroup kill/removal for
   that scope and removes the run directory (`:1030–1055`), with the concrete
   `1\n` write at `cgroup_manager.rs:274`. This is the captured kill owner.
5. Only the original attempt has an exit watcher: a replacement that fails
   this bind never reaches the watcher-spawn site (`vm_driver.rs:1538`). The
   watcher needs only an allocation-keyed Starting/Live entry to claim ending
   authorship (`:935`), so the replacement's Starting entry satisfies it.
   From the unique watcher, accepted crash row, and these guarded production
   paths, the replacement-claim takeover is a **source-supported inference**;
   private map transitions themselves were not instrumented.
6. `drain_guest_report` can return no report/no signal on socket EOF or read
   error without waiting for VMM status (`:2016–2055`). That maps to crashed
   (`:1971`), and the event carries `intentional_stop=false` (`:2129–2143`).
   Thus the row's `signal=None` does not contradict kernel SIGKILL. The exact
   EOF/read-error/select outcome was not captured; do not claim which won.
7. The exit observer fresh-stamps from its predecessor (`exit_observer.rs:458–476`),
   writes the compound observation, then releases supervision (`:300`). The
   failed restart builds its own Failed proposal from the earlier prior row
   (`action_shim/mod.rs:2477–2591`, write `:2612`). It is a failed-start proposal,
   **not a replacement Running proposal**. The accepted crash row plus equal
   rejection matches this competing-writer path; ADR-0099's successful-Running
   acknowledgement repair (`:2655`) is not activated.

These links explain the observed cause without inventing deeper organizational
or architectural WHY levels.

## Control, classification, and exclusions

HTTP204 (`C/capture.log`) reaches Stable and Running(17). It has no EADDRINUSE
bind. Operator stop launches at 14:18:09.757153
(`C/syscalls.968817:26`); after the existing stop grace, Overdrive explicitly
`kill(968717, SIGKILL)`s its VMM at 14:18:21.806068
(`C/syscalls.968595:140`). Its stopped describe is **Terminated(22), reason
stopped** (`C/case/service-vm-http-204-stopped.log:5–6`). Therefore SIGKILL alone
does not determine the allocation's semantic ending: the controlling owner
and promised effect distinguish normal stop from failed-restart cleanup.

The HTTP302 command reports that stop intent was accepted; it does not promise
to rewrite every already-Failed allocation. WorkloadLifecycle's stop branch
selects Running rows only (`workload_lifecycle.rs:637–655`). The observed
HTTP302 trajectory is **not evidence of expected non-crash operator termination**,
even though its resource cleanup completes. Preserve the current Terminated-only
example oracle; do not relax it on the strength of the cleanup ledger.

Ruled out as explanations of this captured VMM death: normal first reclamation
sweep; independent guest HTTP302-triggered exit; network/TAP teardown directly
killing the VMM; and ADR-0099's rejected successful Running publication. The
explicit cgroup kill is positive evidence for the immediate process cause.
The withdrawn stop-before-release/orphan hypothesis is not revived: the existing
terminal-unclaimed reclamation path remains valid when no replacement intervenes.

## Seeded regression — fresh execution

The existing seed-257204 ruling proves terminal-unclaimed disposal with
WorkloadLifecycle, ServiceLifecycle, VmReclamation, and real VmDriver. It does
not drive this intervening same-ID restart's stale-bind/failed-start unwind.
Seed 257203 is the separate successful Exec restart/write witness, not VM proof.

### Scope, audit, and hypothesis

The replacement investigator audited the predecessor's untracked
`crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs` before adopting it.
An initial fresh run of that inherited version failed the authorship invariant
and passed the control (run `e3385959-7947-4e46-8e5e-23599d818357`). The completed
version replaces its directly inserted probe row with the production
ProbeRunner consuming an existing SimHttpProber outcome, fixes logical-clock
progress independently of filesystem speed, and checks the exact startup
terminal condition. No allocation observation or lifecycle occurrence is seeded;
only valid Service intent and driven-port outcomes are supplied. Each test has
its required bounded-change Contract Shape declaration.

- **Hypothesis:** after the original VM's ending is authored, its exit watcher
  can use a same-ID replacement's allocation-keyed claim to author another
  ending, despite no replacement VMM being created.
- **Prediction:** registered owner dispatch reaches failed-start cgroup cleanup;
  the original VMM dies and adds a natural-crash occurrence. Without intervening
  restart, registered reclamation removes resources without changing the
  authored row or occurrence history.
- **Falsification:** the production owner path suppresses the second ending,
  or reaching it requires fabricated rows, a private claim mutation, owner
  cancellation, or an unavailable production seam. None is required here.

The invariant is the existing per-instance ending-authorship corollary in
`docs/product/architecture/brief.md:9111–9117` (§105a.3): an instance whose ending
has been authored must not author another ending. A rejected replacement may
record its own start failure; the invariant does not forbid that, prescribe a
remedy, or require already-Failed allocations to become Terminated after stop.

### Command and observed result

Executed in Linux through the documented Lima workflow:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e10_vm_early_exit_spike --no-capture --no-fail-fast
```

Final-source run `b2ed992a-95fb-4f9b-ac98-3b89e55b1d69` and unchanged rerun
`01fbba8b-69e1-4851-93ae-eec9551ee2f4` each execute two tests: **one failed,
one passed, zero skipped**, printing **seed 257205**. Nextest exits **100**;
the outer xtask command exits **1**. The failure is intentionally retained as
a failing regression, not converted to `should_panic` or ignored. The two
driver/action-shim source hashes above were rechecked and remain unchanged.

| Observation | Restart case | No-restart control |
|---|---|---|
| Original ending | Failed(35), ServiceFailed / StartupProbeFailed, HTTP 302, one attempt | Same |
| At claim release | Original VMM alive; real beacon pathname present; teardown observed VMM alive | Same |
| Host effect | Failed-start cgroup kill terminates simulated PID 1000000 before the 30-second sweep | Registered reclamation kills original VMM after advancing 30 seconds |
| VMM creation count | One; no replacement VMM | One |
| Final row | Failed(36), WorkloadCrashedImmediately, signal 9, terminal absent | Entire Failed(35) row unchanged |
| Occurrences | Adds Failed → Failed, Driver(Vm), natural crash at counter 36 | Entire history unchanged; host artifacts absent |

The failing assertion prints:

```text
seed=257205: already-ended VM authored a second natural crash while a replacement start had produced no new VM
```

### Production reachability and evidence boundary

Both cases use registered WorkloadLifecycle, ServiceLifecycle, and VmReclamation,
the real VmDriver, ProbeRunner, and exit observer. The existing convergence-tick
test entry point delegates normal hydration, reconciliation, validation, dispatch,
and re-enqueue (`reconciler_runtime.rs:1545,1703–1773`); only its existing network
provisioner port is substituted. The first start and Service startup failure are
therefore production decisions. A held-claim reclamation tick also verifies that
registered reclamation does not kill the running original VM.

The complete triggering path is Service FinalizeFailed writing the terminal
ending and releasing supervision (`action_shim/mod.rs:1778–1791`), followed by
WorkloadLifecycle's standing-Service restart policy
(`workload_lifecycle.rs:950–1051,1429`). The restart action awaits stop/cleanup and
the real VM start (`action_shim/mod.rs:2265–2411`). VmDriver installs Starting
before the real Unix beacon bind (`vm_driver.rs:1139–1215`). Its failure invokes
the existing cgroup cleanup (`vm_driver.rs:1029`); the fixture observes that
write killing the one VMM associated with that scope. The retained beacon path,
unchanged successful preflight inputs, no second Vmm::create, and cleanup path
identify the bind-failure branch; the test does not separately log its errno.
Direct EADDRINUSE evidence remains the native capture above.

The substrate fixture couples existing SimCgroupFs and SimVmm methods, including
closing the simulated VMM's Unix peer, without injecting driver events or
private claims. A seed-selected yield count after the kill models a legal
completion delay: RealCgroupFs awaits `tokio::fs::write`
(`overdrive-host/src/cgroup_fs.rs:98`), whose effect can precede resuming its
awaiting caller. No owner is aborted, and convergence ticks are dispatched
serially. The original watcher is the sole producer of the natural VM
crash; its existing allocation-keyed ClaimGuard accepts Starting or Live
(`vm_driver.rs:935,2080–2129`). The real exit observer accepts the event and
appends the observed occurrence (`worker/exit_observer.rs:458–476`). Thus the
second authored ending is observed, not a fabricated state. Private map writes
are not instrumented; attribution to the claim transition follows this sole
production emission path.

The production convergence loop awaits each action before checking shutdown
(`lib.rs:3377–3401`); the exit observer checks cancellation before receiving an
event and then awaits its retry operation (`worker/exit_observer.rs:198–206`).
No shutdown is requested in this regression. The restart's Failed publication
does not enter ADR-0099's Running-only acknowledgement repair
(`action_shim/mod.rs:2655`). The no-restart case instead obtains reclamation's
EndingInFlight lease, removes the substrate, and preserves the original ending
(`action_shim/reclamation.rs:63,299`), consistent with the corrected ownership
ruling. This is not a missing-stop-before-release finding.

This is a bounded deterministic schedule, not an exhaustive schedule search.
The tests explicitly invoke registered convergence ticks rather than the full
automatic broker/cadence loop; the 30-second first sweep is separately grounded
in `lib.rs:3340–3349` and `vm_reclamation.rs:242`. Temp-file Unix sockets and
intent/view stores are real; VMMs, cgroups, network provisioning, and host
artifact observations are simulated. No built Overdrive binary or host VMM is
spawned. The simulation reports signal 9 whereas native describe reported no
signal; raw EOF/exit-watch poll ordering is not asserted to be identical. The
simulation establishes the authorship failure under a reachable driven-port
completion schedule, not the exact private interleaving of the historical run.

### Verification limits and disposition

Targeted Clippy through Lima with the same package/features/test and
`-- -D warnings` is blocked by an existing out-of-scope
`service_lifecycle.rs:498` too-many-lines diagnostic. Adding `--no-deps` is
blocked in existing `overdrive-sim/src/harness.rs:617` and
`src/invariants/exit_event_observable_outcome.rs:143,149` large-futures diagnostics.
Both commands exit 1 externally / 101 internally. These files were not edited;
Clippy is not claimed green. Intermediate compile errors while completing the
fixture were corrected before the final-source runs above.
The completed test passes `rustfmt --edition 2024 --check`; both owned files
also pass a trailing-whitespace check.

**No correction is selected or implemented.** The bounded question for the next
authorized phase is the existing same-ID restart boundary after terminal VM
authorship release. The seeded authorship failure and passing reclamation
control are now demonstrated; remedy selection remains separately authorized
work. No persistence system, global writer fence, reclamation redesign, altered
Service startup meaning, or expectation-predicate relaxation is introduced.
