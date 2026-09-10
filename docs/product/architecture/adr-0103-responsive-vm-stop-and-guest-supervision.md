# ADR-0103 — Responsive VM stop and one guest workload supervisor

## Status

**APPROVED — independent DESIGN review APPROVED; user ratification APPROVED**, 2026-09-10.
[Review iteration 2](../../feature/vm-lifecycle-latency/design/review.md#iteration-2--remediation-re-review)
approved the proposal. The user ratified this design and its behavioral
decisions on 2026-09-10; implementation remains outstanding.
Amends only the request/grace ordering and PID 1 duties in ADR-0082 D4/D7.
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
