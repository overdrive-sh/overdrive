# ADR-0103 — Responsive VM stop and one guest workload supervisor

## Status

**APPROVED — independent DESIGN review APPROVED; user ratification APPROVED**, 2026-09-10.
[Review iteration 2](../../feature/vm-lifecycle-latency/design/review.md#iteration-2--remediation-re-review)
approved the proposal. The user ratified this design and its behavioral
decisions on 2026-09-10; implementation remains outstanding.

**Natural/post-EXEC driver-artifact cleanup amendment: PROPOSED**, 2026-09-11.
The user authorized the bounded remedy after qualified native-metal S10a and
post-EXEC S10b reproductions; independent DESIGN review is required before the
original DELIVER step `01-02` crafter resumes. The base decision above remains
approved. This amendment changes only the private `VmDriver` natural-exit
ordering described below.

Amends only the request/grace ordering and PID 1 duties in ADR-0082 D4/D7,
plus the private accepted-session natural/post-EXEC exit-watcher ownership and
driver-artifact cleanup ordering specified below.
Companion [ADR-0102](adr-0102-bounded-convergence-evaluation-ownership.md) owns
shared scheduling. The [feature delta](../../feature/vm-lifecycle-latency/feature-delta.md)
owns the lifecycle-gate matrix, benchmark profiles and evidence obligations.

## Host decision and exact result contract

Reuse the accepted BeaconWriter and `Vmm::terminate` ports. At a successful
Live → EndingInFlight stop claim, submit SHUTDOWN and begin the VMM termination
wait together. Retain `VM_SHUTDOWN_REQUEST_DEADLINE = 2 s` solely as the writer
bound and `VM_STOP_GRACE = 10 s` as the host VMM grace, both measured from this
stop operation's entry to its request/wait phase. They overlap. Normal VMM exit
and writer completion proceed immediately; never sleep to the end of either
successful-path window. On VMM exit, join the writer (abort and join it if still
pending); on writer timeout abort/join it while continuing the same VMM wait.
Missing writer skips only request submission, not process completion.

The approved healthy completion criterion is **normal reaper-observed VMM exit
plus verified required driver-artifact absence**, measured by the native
validation owner. It is explicitly **not a stronger `Driver::stop Ok` API**.
Retain the current generic postcondition, for both drivers, and the current
VM stop error/cleanup policy:

| Result / observation | Exact meaning |
|---|---|
| No Live entry at stop claim (including Starting, EndingInFlight or absence) | Existing `Err(DriverError::NotFound { alloc })`; no new stop-before-READY claim is introduced. |
| Live entry claimed | Before awaiting, move to EndingInFlight under the existing LiveMap lock; watcher identity and supervised-set behavior remain unchanged. |
| Writer error, disconnected session, missing task or writer timeout | Best-effort request failed/unconfirmed; continue VMM termination. No receipt acknowledgement and no new DriverError variant. |
| VMM completion | Await existing `Vmm::terminate(&control, VM_STOP_GRACE)`. Its `Killed` includes already gone; only a reaper/process observation establishes normal versus forced termination for measurement. |
| VMM error / cleanup errors | Preserve current best-effort handling (including currently discarded results); complete the remaining current cleanup calls. They are not newly promoted into success guarantees or a persistent retry plan. |
| VM `stop` returns `Ok(())` | Active status is relinquished, the writer is consumed and existing termination/cleanup calls have finished. A subsequent `status` returns NotFound. It does not assert normal exit or that every cleanup succeeded. |
| Healthy-stop verification | Independently require normal VMM exit and absence of VMM process, allocation cgroup scope, run directory/sockets, rootfs clone and clone-index link. Any missing evidence/error/forced exit fails this verdict even after `Ok`. |

Continue cgroup kill/remove, run-directory removal, clone removal then index
removal. The stop shim then awaits its existing mTLS/netns cleanup, probe hook,
latest-row reread, at most two LWW proposals, supervision release and index
removal (`action_shim/mod.rs:2811–2963`). Preserve competing terminal truth and
existing typed shim failures. Do not hold a global scheduler lock for these
effects or suppress the independent exit observer.

**Why retain the limited API:** the approved defect is waiting and control
responsiveness, not a reproduced cleanup/retry failure. Today EndingInFlight
contains no retained cleanup payload, and a later stop returns NotFound.
Promoting cleanup errors to a stronger stop contract would also require a
choice of retry state/ownership and terminal gating. That is a separate
contract amendment, not an implied part of removing the two-second floor.
The approved benchmark cannot conceal this limitation by timing `Ok` alone.

Unchanged interfaces, with no new public type/variant/parameter:

```
async fn Driver::stop(&self, handle: &AllocationHandle) -> Result<(), DriverError>
async fn Vmm::terminate(&self, control: &VmControl, grace: Duration) -> Result<VmTermination>
fn BeaconWriter::request_stop(&self) -> Option<JoinHandle<()>>
```

These are qualified interface descriptions; existing trait syntax/visibility
and type aliases remain in their owning files. Keep `VmTermination`, VmControl,
VmExitWatch, VmSupervision, LiveVm, ClaimGuard and Driver error shapes unchanged.
The Vmm port remains process-only; it acquires no guest connection or message.

## Proposed natural/post-EXEC driver-artifact cleanup amendment

### Revalidated production path and bounded problem

Qualified native-metal runs now establish a narrower cleanup defect than the
base latency decision had evidence for. S10a natural guest completion and the
post-EXEC S10b EOF/malformed/duplicate-control cases reach a finalized terminal
observation while the allocation's run directory, cgroup scope, rootfs clone and
clone-index link remain present. The pre-EXEC EOF/malformed/unexpected cases
start no guest child and are not part of this driver-artifact acceptance oracle.

The current production path is:

```
overdrive serve
  -> VmDriver::start
  -> VmDriver::spawn_exit_watcher_task
  -> run_exit_watcher
  -> ExitEvent
  -> worker::exit_observer::run_with_retry
  -> accepted AllocStatusRow + interest-router wake
  -> WorkloadLifecycle::reconcile
  -> Action::FinalizeFailed
  -> action_shim::dispatch
```

`run_exit_watcher` currently changes the originating accepted session from
`Live` to `EndingInFlight` and emits `ExitEvent` without driver teardown.
`EndingInFlight` intentionally retains no `LiveVm`, so a later
`VmDriver::stop` returns the existing `DriverError::NotFound`. The
`FinalizeFailed` arm owns mTLS, netns, probe-supervision and terminal-row work,
but receives no `AllocationSpec`, `RootfsPlan`, `VmRunDir` or `CgroupPath` and
therefore cannot perform the four driver-owned operations. Boot reclamation is
an eventual residue owner, not completion of this accepted live session.

### Decision and exact private implementation contract

Extend the existing `VmDriver` owner; do not add another cleanup authority.
Both operator stop and the natural-exit watcher atomically claim the same
originating `Live` entry under the existing `LiveMap` lock, move its unique
`LiveVm` value into the winning task, and replace the map entry with the
existing unit `VmSupervision::EndingInFlight` in the same critical section.
No lock is held across an await, and `EndingInFlight` gains no payload.

The only shared private cleanup extraction is exactly:

```rust
async fn cleanup_driver_artifacts(
    cgroup_manager: &CgroupManager,
    live_vm: &LiveVm,
)
```

It contains, in the current order, only the four operations already present in
`VmDriver::stop`: await `cgroup_kill(&live_vm.scope)`, await
`remove_workload_scope(&live_vm.scope)`, await
`tokio::fs::remove_dir_all(live_vm.run_dir.path())`, then await the existing
`remove_clone_then_index_link(&live_vm.rootfs)` helper. Results retain today's
best-effort/NotFound-tolerant policy and error set. The helper returns only
after every call has returned; scheduling it is not completion. Its two and
only two callers are `VmDriver::stop`, after the existing joined writer and
awaited `Vmm::terminate` composition, and `run_exit_watcher`, after it wins the
originating-session claim and before it sends `ExitEvent`.

`ClaimGuard::try_begin_ending` changes only its private return shape from
`bool` to the exact `Option<LiveVm>` below. `Some` is the unique cleanup
capability; `None` is the existing lost-claim verdict. Its exact method
signature inside the existing `impl ClaimGuard` is
`fn try_begin_ending(&mut self) -> Option<LiveVm>`. It still performs the
accepted-session `Weak<BeaconWriter>` identity check and `Live ->
EndingInFlight` replacement atomically, and still marks its Drop guard as
handed off only on success.

`VmDriver::stop` makes the equivalent existing non-session-qualified claim by
removing the whole `LiveVm`, rather than cloning its five cleanup fields, and
inserting `EndingInFlight` before releasing the map lock. Its public signature,
result, writer/VMM ordering and post-stop `status == NotFound` contract do not
change.

To route the already-composed cleanup owner into the detached watcher,
`spawn_exit_watcher_task` clones the existing `CgroupManager` and passes it as
one additional private argument. The resulting private watcher signature is:

```rust
async fn run_exit_watcher(
    alloc: AllocationId,
    exit: VmExitWatch,
    reader: BufReader<OwnedReadHalf>,
    live: Arc<LiveMap>,
    exit_tx: mpsc::Sender<ExitEvent>,
    cgroup_manager: CgroupManager,
    cgroup_accounting: Arc<dyn CgroupAccounting>,
    cgroup_root: PathBuf,
    scope: CgroupPath,
    limit_bytes: u64,
    gate_receiver: oneshot::Receiver<()>,
    beacon: Weak<BeaconWriter>,
)
```

The watcher ordering is exact: consume the guest report/direct status and VMM
exit; read the existing best-effort OOM fact; await the Running-confirmed gate;
atomically take the matching `LiveVm` and install `EndingInFlight`; await
`cleanup_driver_artifacts`; emit the existing
`vm.lifecycle.cleanup_calls_finished`; then send the one existing `ExitEvent`.
Only that send allows the exit observer and then `FinalizeFailed` to advance
the natural/post-EXEC terminal path. The `LiveVm` remains owned by the watcher
through cleanup and send; it is not persisted or copied into another owner.

Natural exit never calls `Vmm::terminate`, never requests SHUTDOWN and never
manufactures an operator stop: the VMM's direct exit has already been reaped,
and the existing guest report, `ExitKind`, OOM classification and
`intentional_stop: false` flow unchanged. `VmDriver::stop` retains the base
ADR's overlapped writer/VMM composition before calling the same cleanup helper.

### Concurrent stop versus natural exit

| Lock winner | Winning work | Losing observation | Exactly-once reason |
|---|---|---|---|
| `VmDriver::stop` | Moves the unique `LiveVm` to the stop task, installs `EndingInFlight`, joins/aborts the existing writer as specified, awaits VMM termination, then awaits shared driver cleanup. | The watcher fails its accepted-session claim, emits no `ExitEvent`, performs no driver cleanup and leaves the stop path's operator terminal authorship unchanged. | Only the stop task owns the moved `LiveVm`; the map is no longer `Live`. |
| `run_exit_watcher` | Moves the unique matching-session `LiveVm` to the watcher, installs `EndingInFlight`, awaits shared driver cleanup, then emits its natural `ExitEvent`. | A concurrent/later `VmDriver::stop` sees `EndingInFlight` and returns the existing `NotFound`; the stop shim retains its existing best-effort treatment and LWW terminal arbitration, but performs no second driver cleanup. | Only the watcher owns the moved `LiveVm`; the map is no longer `Live`. |

Reclamation sees `EndingInFlight` in both orderings and therefore cannot claim
the allocation. `release_supervision` remains after the accepted/failed/no-write
exit-observer outcome or after the stop shim's existing terminal arbitration.
No second EXIT, terminal state, durable claim or cleanup retry is introduced.

### Lifecycle Gate Ownership — natural/post-EXEC cleanup

The base lifecycle meanings remain unchanged. This amendment moves one private
gate: driver cleanup-call completion now precedes the natural watcher report.

#### Gate G-4 — driver artifacts before natural watcher report

- **Existing evidence:** the production path above; qualified native-metal
  S10a and post-EXEC S10b retained the run directory, cgroup scope, clone and
  clone-index link after terminal observation.
- **Owner:** the existing `VmDriver` exit watcher, and only after its
  originating accepted session wins the `LiveMap` claim.
- **Promise:** all four existing driver cleanup calls have returned before the
  watcher sends `ExitEvent`; on the healthy native substrate, the four named
  artifacts are absent. The separate artifact observation remains the proof of
  absence because the existing cleanup calls are best effort.
- **Affected state:** only the natural/post-EXEC `ExitEvent` delivery and its
  downstream observation/finalization path.
- **Failure projection:** existing ignored cleanup results and existing exit
  classification remain. No new `DriverError`, `InitError`, allocation state,
  retry state or durable recovery record is created.
- **Explicitly unaffected:** READY, allocation Running, Service Stable and
  eligibility; operator-stop classification; guest direct-child status and
  control-stream error; EXIT-at-most-once; writer/VMM overlap; immediate
  poweroff; mTLS/netns/probe cleanup; LWW terminal arbitration.
- **Ordering:** claim and take `LiveVm` atomically; await all four calls; emit
  `vm.lifecycle.cleanup_calls_finished`; send `ExitEvent`; let the existing
  observer/reconciler/shim path author its rows. No cleanup future is detached.
- **Counterexample:** emitting `ExitEvent` first lets `FinalizeFailed` publish a
  finalized terminal claim while no remaining owner can recover the discarded
  `RootfsPlan`, recreating the native failure.
- **Evidence lane:** existing in-process stop race/session tests plus qualified
  native-metal S10a and post-EXEC S10b artifact observation. Pre-EXEC no-child
  errors retain their typed/no-EXIT oracle without a new cleanup requirement.

### Reuse Analysis and rejected larger alternatives

| Existing owner/mechanism | Overlap | Decision | Contract shape / bounded universe |
|---|---|---|---|
| `VmDriver::stop` teardown sequence | Owns cgroup, run-directory, clone and clone-index cleanup | **EXTEND/REUSE** by one private helper called from stop and the natural watcher | bounded-change: one claimed allocation's four driver artifacts; native before/after complement plus existing clone-order tests |
| `LiveMap` + `ClaimGuard` | Serializes accepted-session ending authorship | **REUSE** the same `Live -> EndingInFlight` claim while moving the unique private `LiveVm` to the winner | bounded-change: one map entry and one Rust-owned `LiveVm`; stop/watcher race proves one winner and one cleanup invocation |
| Exit observer + `FinalizeFailed` | Classifies/persists the ending and performs shim-owned cleanup | **REUSE unchanged** after the watcher cleanup gate | bounded-change: existing allocation rows, lifecycle occurrence, mTLS/netns/probe owners and LWW winner |

Rejected: retaining `LiveVm` or a completion channel inside
`EndingInFlight`; adding another state/map/payload; persisting cleanup intent or
adding a retry/recovery subsystem; giving `FinalizeFailed` a `RootfsPlan` or a
second driver-cleanup authority; calling public `Driver::stop` from the natural
watcher; adding a cleanup method to `Driver`; or relying on a later reclamation
sweep. Each is larger than moving the already-private value to the atomic claim
winner, changes an interface/owner/system-of-record boundary, or cannot provide
the required awaited ordering.

## Guest decision and exact boundary

One PID 1 owner receives control while supervising the workload. Reuse std
Command and nix; the supervisor has no separately owned reader thread/task,
second parser, child restart policy or service manager. Preserve all current
pre-READY filesystem/module/network work and READY/EXEC host ordering.

| State / input | Required behavior |
|---|---|
| Awaiting first EXEC | Consume exactly one frame. EOF → existing `NoExecReceived`; malformed → `BeaconParse`; valid non-EXEC, including SHUTDOWN → `UnexpectedBeaconMessage`. No child starts; existing fatal poweroff path applies. This includes the reachable post-READY/intercept-install-failure unwind, where the host sends SHUTDOWN and never sends EXEC (ADR-0089). |
| EXEC accepted | Establish a new process group with PGID equal to the direct child's PID before its command executes, using std `CommandExt::process_group(0)`. Retain current session and inherited console stdio; no new session/TTY manager. Exactly one command starts. |
| Running | Service control and child-state changes concurrently; no blocking wait for child completion before reading control. Ten-millisecond maximum idle polling quantum (`GUEST_SUPERVISION_POLL`) is a responsiveness bound, not a mandatory shutdown sleep. |
| First SHUTDOWN while group lives | Record one shutdown deadline, send SIGTERM to the whole workload group, keep reading/reaping. `GUEST_STOP_GRACE = 5 s` starts on parsed receipt. Subsequent SHUTDOWN is idempotent and cannot extend grace. |
| Direct child exits naturally | Retain its exact status. If descendants remain in the group, begin the same graceful group termination; otherwise finish immediately. Natural finite Jobs do not await a later host SHUTDOWN. |
| Grace expires with live group members | SIGKILL the remaining workload group, then continue reaping. Poweroff is authorized after direct child is reaped and group membership is empty. The host's existing ten-second fallback contains a guest that cannot complete this rule; no second guest grace is appended. |
| Adopted children / group escape | PID 1 reaps all available direct/adopted children. Descendants retaining the group receive group signals. A child using setsid/setpgid to leave the group is outside graceful-group coverage; guest poweroff terminates it. Do not await escaped live descendants indefinitely or claim they exited gracefully. |
| EOF/I/O error or malformed/unexpected frame during execution | Start/continue the same bounded group termination, retain the original error, reap before returning it to the existing fatal poweroff path. Do not fabricate a successful EXIT. Duplicate EXEC is unexpected and never starts another child. |
| Group complete | Successful supervision returns the direct child's status, sends EXIT at most once via the existing sender, then immediately requests RB_POWER_OFF. A late SHUTDOWN cannot alter the saved status. Send/poweroff errors use existing InitError/fatal behavior; no new acknowledgement is emitted. |

Group signaling success alone is not completion; verify group disappearance
and reap available children. Preserve `exit_status_to_wire`: ordinary exit
code, `128 + signal` for signal death, existing `-1` fallback. Descendant exit
codes never replace the direct child's. The host stop claim still owns operator
stop classification; it does not reinterpret a returned guest EXIT as Job
success. Natural endings retain the existing guest-reported outcome classifier.

Rust's [stable process-group API](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.process_group)
supports setting PGID before command execution. Linux
[wait semantics](https://man7.org/linux/man-pages/man2/waitpid.2.html) require
collecting child status; [poweroff](https://man7.org/linux/man-pages/man2/reboot.2.html)
is a later guest transition. These substrate contracts motivate the separation;
the real-guest tests must establish that this implementation honors it.

### Exact private interface changes in `overdrive-init/src/main.rs`

Keep `run() -> Result<(), InitError>` and `recv_exec(conn: &File) ->
Result<Vec<String>, InitError>`. Change the existing command boundary to
`fn exec_operator_command(conn: &mut File, argv: &[String]) -> Result<i32, InitError>`.
It owns the control/group/reaping state described above. `recv_exec` must consume
only its first frame and leave following bytes for that same stream owner;
partial/coalesced reads cannot lose bytes at the handoff. Keep the existing
shared `BeaconMessage` codec and `send`/exit-mapping interfaces. Remove the
post-EXIT `read_shutdown_or_eof` call and its now-unused helper.

The existing private generic lifecycle composition boundary becomes:

```
fn complete_guest_lifecycle<Conn, Root, Modules, Connect, Network, Ready,
    ReceiveExec, Execute, SendExit, PowerOff>(
    root: Root, modules: Modules, connect: Connect, network: Network,
    ready: Ready, receive_exec: ReceiveExec, execute: Execute,
    send_exit: SendExit, power_off: PowerOff,
) -> Result<(), InitError>
where
    Root: FnOnce() -> Result<(), InitError>,
    Modules: FnOnce() -> Result<(), InitError>,
    Connect: FnOnce() -> Result<Conn, InitError>,
    Network: FnOnce() -> Result<(), InitError>,
    Ready: FnOnce(&mut Conn) -> Result<(), InitError>,
    ReceiveExec: FnOnce(&Conn) -> Result<Vec<String>, InitError>,
    Execute: FnOnce(&mut Conn, &[String]) -> Result<i32, InitError>,
    SendExit: FnOnce(&mut Conn, i32) -> Result<(), InitError>,
    PowerOff: FnOnce() -> Result<(), InitError>
```

Append only these private `InitError` variants for the new syscall outcomes:
`Wait(#[source] Errno)` and
`ProcessGroup { operation: &'static str, pgid: i32, source: Errno }`, with
`source` marked `#[source]`. Existing Spawn/Io/BeaconParse/Reboot mappings stay.
Group `ESRCH` means already absent, and interrupted signal/wait syscalls retry
inside the same deadline. `ECHILD` is completion only after the direct child's
status was already collected; otherwise it is `Wait`. EOF after EXEC projects
to `Io` with `std::io::ErrorKind::UnexpectedEof` after group termination; it
must not reuse the pre-EXEC `NoExecReceived` cause. Other errors retain their
typed cause. Constants are
private `Duration`: `GUEST_STOP_GRACE = Duration::from_secs(5)` and
`GUEST_SUPERVISION_POLL = Duration::from_millis(10)`.
No public process-supervision API, wire variant or runtime dependency is added.

## Bounded stage evidence

Retain the existing `vm.beacon.exec.released` event (`alloc` field) as the
successful EXEC-release boundary, after writer acknowledgement and before the
exit-emission gate is released. It is not a guest-receipt event. Add only the
following tracing events to the existing owners; no public port signature or
stored lifecycle state changes for measurement:

| Event | Emission point / fields |
|---|---|
| `vm.lifecycle.create_enter` | Immediately before delegating to Vmm::create; `alloc` |
| `vm.lifecycle.created` | Vmm::create returned its control; `alloc`, `pid` |
| `vm.lifecycle.ready` | Successful existing accept_ready arm; `alloc` |
| `vm.lifecycle.stop_enter` | Stop claimed Live and begins request/VMM wait; `alloc`, `pid` |
| `vm.lifecycle.writer_finished` | Stop consumed the writer task; `alloc`, `disposition` (`completed`, `aborted`, `absent`) |
| `vmm.process.reaped` | Host reaper collected actual child status, before outcome publication; `pid`, `exit_code`, `signal` |
| `vm.lifecycle.cleanup_calls_finished` | Current driver cleanup calls returned; `alloc` (does not claim artifact absence) |

Native in-process measurement captures these events synchronously with one
host monotonic clock and correlates the live allocation-to-VMM mapping. An
early exit that precedes correlation is retained as a failed/incomplete sample,
not silently omitted. Exact cleanup absence comes from independent kernel/file
observation after those calls. The benchmark also records driver-stop return
and shim terminal/cleanup endpoints separately. No subtraction of asynchronous
console-arrival time from a host timestamp is valid.

Guest diagnostics record `shutdown-received`, `group-complete` and
`poweroff-requested`, with elapsed monotonic milliseconds since that guest
supervisor started, on the existing console; they add no beacon frame or host
state. Primary host distributions and guest diagnostic durations remain
separate. The black-box example measures its public command/resource boundary
with an external monotonic clock; it does not inspect private events as its
stakeholder verdict or reproduce the native integration assertions.

## Changed Assumptions

| Superseded wording, quoted exactly | Accepted amendment |
|---|---|
| ADR-0082 D4: “`VmDriver::stop` then calls `Vmm::terminate(&control, grace)`, which awaits the VMM's exit for `grace` and `SIGKILL`s it on expiry.” | Request and VMM wait begin together; the writer's two-second maximum lies within the ten-second VMM grace. Writer success or normal VMM exit never waits for the bound to expire. |
| ADR-0082 D4 constants: “Bounds **step 1's write**.” and “Bounds **step 2**.” | Preserve values, amend their sequential composition to overlapping deadlines; the native implementation's unconditional completed-write sleep is removed. |
| ADR-0082 D7: “then reads at most one further `SHUTDOWN` (§ D4).” | PID 1 reads SHUTDOWN during execution, supervises the defined group, and powers off after its completion/report without a post-EXIT read. |
| This ADR, “the approved defect is waiting and control responsiveness, not a reproduced cleanup/retry failure.” | Qualified native-metal S10a and post-EXEC S10b now reproduce a driver-artifact cleanup gap. The amendment reuses the private current cleanup calls before the natural watcher report; it does not strengthen `Driver::stop Ok`, expose cleanup errors or add retry ownership. |

ADR-0082's historical statement “Running ⟹ READY arrived AND `EXEC` was delivered”
was already superseded by ADR-0089's deferred EXEC barrier and is not the current
state contract. Current `vm_driver.rs:1515–1522` returns start success on READY;
the shim writes Running, installs interception and only then releases EXEC.
ADR-0099 gates restart post-start effects on accepted Running publication.
This feature preserves that ordering: neither Running nor writer completion
proves guest execution/receipt. ADR-0082's historical pre-beacon-stop totality prose
does not match current `Starting → NotFound` stop behavior; this feature does
not claim that mismatch is a separately reproduced defect or fix it. Under
ADR-0102, a competing reconciliation stop waits for the start chain to finish.
ADR-0100's originating-session claim predicate, ADR-0083 reclamation protection
and the generic stop/status postcondition remain unchanged.

## Alternatives and consequences

Preferred: existing protocol plus one supervisor and overlapped host waits.
A dedicated control-reader thread adds another shutdown owner; a new receipt
ACK adds protocol state without proving workload/VMM/cleanup completion; a VMM
shutdown command bypasses the guest process policy. None is needed for the
proven defect. A verified-cleanup `Driver::stop` success contract is explicitly
deferred rather than partially implemented without retry ownership.

The five-second guest grace applies only to non-completing groups; healthy
cooperative groups proceed immediately. The ten-second host grace remains a
failure bound, not a healthy SLO or guaranteed upper bound on kernel reaping.
Rootfs owners must deploy the rebuilt init to receive guest responsiveness;
older init images retain the host's bounded force fallback. Native validation
must record this exact guest artifact and may not infer behavior from the
host binary version alone.
