# VM lifecycle latency — issue #283 investigation

<!-- DES-ENFORCEMENT : exempt -->

Investigator: Codex, applying `nw-root-why`, `nw-troubleshooter`,
`nw-investigation-techniques`, and `nw-five-whys-methodology`.
Baseline: `d880ce983276e7d4107450b264f6ceab64c70eff`. `AGENTS.md` was already
modified on entry; it is user/parallel work and is preserved.

Scope is VM lifecycle responsiveness and the shared convergence execution
path. This investigation does not implement a remedy or select an API. Issue
#283 governs over historical acceptance/deferral wording in #260 and ADR-0082.
Both issue bodies and their empty comment arrays were read through
`gh issue view --comments --json number,title,body,comments`.

## Result

The shared convergence task waits for each allocation effect to finish. Two
new seed-283001 liveness invariants fail through the actual server, HTTP
submission, broker and convergence owner: an independent workload stays
unstarted while another Driver start **or** stop is pending, although HTTP
queries remain responsive. Both progress after release; the no-delay control
passes. No new production composition boundary was necessary.

On qualified native metal, a healthy Service stop took **12,020.339 ms**
without process tracing. The host writer's application-level SHUTDOWN
`write_all` and `flush` calls returned `Ok(())` within **0.119 ms**,
then deliberately waited out **2,002.164 ms** before entering VMM termination;
that call consumed another **10,017.545 ms**, ending in SIGKILL. The following
owned-artifact cleanup calls took **0.630 ms**. Two process-traced controls
show the same partition. These are measurements, not the constants mistaken
for observations. These writer observations establish prompt application-level
call return, not wire delivery, peer consumption, or a guest-read syscall.

The additional grace is explained by the guest program: PID 1 synchronously
waits for the operator child before reading SHUTDOWN. The selected image's
actual `/sbin/init` is byte-identical to a fresh build of the current
`overdrive-init` source. A long-lived Service cannot reach that read while
its child keeps serving. A finite checked-in client Job did report EXIT,
power off and produce a normal VMM exit in **30.445 ms after EXEC release**
in the untraced control. Thus the ten-second wait is not an unavoidable
Cloud Hypervisor exit/reap or host-cleanup duration.

Diagnosis is complete for these mechanisms; no remedy, latency SLO, public API,
architecture approval, or expectation satisfaction is claimed. The codebase
still contains the defect and the new diagnostic intentionally leaves two
failing tests. Independent review is pending with the orchestrator.

## Predictions recorded before probes

### P1 — existing seeded overlap witness

Hypothesis: the existing seed-257210 witness still demonstrates that the
selected serial per-tick order can exhaust the original Service stream cap.
Prediction: overlapping control fails with Timeout, separated control passes
with StartupProbeFailed. Falsification: no Timeout or the separated control
also fails. This witness does not exercise broker selection or native VMM
timing; it is retained as a distinct earlier evidence boundary.

### P2 — complete broker/convergence owner

Hypothesis: a slow Driver start or stop blocks an independently submitted
workload in the real server's broker-driven convergence task.
Prediction: while the injected driven-port effect remains pending, the
independent workload cannot reach Running despite continued clock advancement
and a responsive HTTP owner; it advances after the effect returns. The
zero-delay control progresses. Falsification: independent Running is observed
while the effect remains held, or the healthy/released controls fail.
The observation window is a diagnostic bound, not a new product latency KPI.
Use the existing `run_server_with_obs_and_driver` entry point and existing
Driver/Clock/ObservationStore ports; do not add a production seam.

### P3 — native healthy/failure lifecycle stages

Hypothesis: the host writer's SHUTDOWN `write_all`/`flush` calls return promptly,
but guest PID 1 remains in the synchronous wait for the long-lived child;
it cannot read SHUTDOWN until
the child exits, so the host consumes the 2-second request window plus the
10-second VMM grace and kills the VMM. A naturally finishing child is a
distinct control that can reach the guest's post-exit read/poweroff sequence.
Prediction: a checked-in healthy Service has prompt `Ok(())` returns from the
application-level SHUTDOWN `write_all`/`flush` calls, no normal VMM exit during
either window, and a host SIGKILL around
12 seconds later. A finite guest Job exits without that 12-second hold.
Falsification: SHUTDOWN writer calls stall, the VMM exits normally but its completion
is not observed, or a guest that finished its child still waits the full grace.

Native captures must first pass the canonical exclusive lease and fail-closed
x86_64/non-virtualized/KVM preflight. Retain every attempt, source/binary/image
identity, timestamp and exit code. Use checked-in product examples. Syscall
observations establish host effects; guest-read ordering is a source-supported
inference unless a guest observation independently establishes it. No fixture,
assertion, timeout, security policy or production behavior is changed.

### P4 — tracing overhead and guest boot partition

After two process-traced healthy runs, VMM-create-to-READY measured several
seconds. Hypothesis: tracing the confined VMM contributes to the absolute boot
duration, separately from the production queue and stop windows. Prediction:
an untraced run retains the 2s + 10s stop partition, while guest console kernel
timestamps locate boot work before READY. Falsification: stop windows vanish
without strace, or the console shows READY long before host acceptance. This
control does not change the kernel, guest program, spec, or resource limits.

## Current production owner path

Line references below are against baseline HEAD, after removing the temporary
observation-only patch.

1. `crates/overdrive-cli/src/main.rs:189` handles `serve`;
   `commands/serve.rs` constructs the real server. The production composition
   is `overdrive-control-plane/src/lib.rs:1608` (`run_server`) through
   `run_server_with_obs_and_drivers` at 2115. The existing single-driver
   wrapper at 2088 is the seeded test's composition entry, not a new seam.
2. HTTPS submission/stop routes are registered in `lib.rs:3135` onward.
   `handlers.rs:257` (`submit_workload`) commits intent, then its successful
   paths enqueue at 508/538. `handlers.rs:835` (`stop_workload`) records stop
   intent and enqueues at 890. The common helper at 58 submits a
   `workload-lifecycle`, `workload/<id>` Evaluation to the real broker.
3. `lib.rs:3066` spawns the convergence owner; its implementation at 3320
   submits due resyncs and drains the pending broker batch at 3355–3361.
   The `for eval in pending` at 3363 awaits each `run_convergence_tick` before
   proceeding. New submissions cannot enter that already-drained batch.
4. `reconciler_runtime.rs:1352`/1400 onward builds `TickContext`, hydrates,
   reconciles, persists the next view at 1501, and awaits action dispatch at
   1535 onward. The source constructs `deadline` but this dispatch path does
   not race lifecycle work against it. Re-enqueue at 1599 occurs only after
   the awaited dispatch returns.
5. `action_shim/mod.rs:900` (`dispatch_with_network_provisioner`) loops over
   actions at 922 and awaits `dispatch_single`. The StartAllocation arm at
   1796 provisions the assigned network/identity before the registry's
   `driver.start(&spec).await` at 1930. The StopAllocation arm at 2811
   resolves the allocation's existing driver index and awaits stop at 2825,
   then completes its other cleanup and terminal publication. RestartAllocation
   at 2253 likewise awaits stop and then start. Error isolation is per action;
   it is not concurrent execution.
6. `overdrive-worker/src/vm_driver.rs:1456` calls `provision_vmm` (1134):
   claim, run directory, artifact checks, beacon listener, cgroup/limits,
   clone-index link; then `Vmm::create` at 1343. The Cloud Hypervisor adapter
   at `overdrive-host/src/vmm.rs:372` clones the rootfs, copies/chowns the
   confined kernel/run paths and spawns the confinement chain. `ip → prlimit
   → setpriv → cloud-hypervisor` retains the same PID, as the native traces
   independently confirm. Driver start then places that PID in its scope
   and races READY, VMM exit and the 30s boot deadline at 1505. READY ends
   start; the shim installs the intercept before the existing EXEC-release
   hook at `vm_driver.rs:1796` onward writes the command.
7. `VmDriver::stop` at 1651 extracts the Live allocation and synchronously
   moves its claim to EndingInFlight. A non-Live/absent claim returns NotFound.
   With an accepted beacon writer it signals shutdown and awaits the full
   two-second deadline at 1715–1735, even if the writer already returned.
   It then awaits `terminate(control, 10s)` at 1741 and performs its owned
   cgroup/run-directory/clone/index cleanup at 1752–1755.
8. `CloudHypervisorVmm::terminate` at `vmm.rs:534` checks the existing outcome;
   a dead process returns immediately. Otherwise the nonzero-grace branch
   waits for the reaper's outcome or a Tokio timer. It sends **no initial
   graceful VMM signal**. On timer expiry it notifies the existing reaper,
   which calls `child.start_kill()` and `child.wait()` at 499–507.
9. Guest `overdrive-init/src/main.rs:142` composes the actual lifecycle.
   `complete_guest_lifecycle` at 200–205 calls `execute` **before**
   `send_exit`, `shutdown` and `power_off`. `exec_operator_command` at 1029
   uses blocking `Command::status()` at 1039–1042. `read_shutdown_or_eof`
   at 1008 is therefore unreachable during a still-running Service child.

### Cancellation/shutdown and retry are not alternate escape paths

`ServeHandle::shutdown` (`commands/serve.rs:95`) passes a five-second HTTP
drain duration to `ServerHandle::shutdown` (`lib.rs:1409`). That method first
cancels and **joins** the convergence task. The convergence task only checks
cancellation in the cadence select after the entire pending batch
(`lib.rs:3386`), not inside the active start/stop. Consequently the HTTP drain
argument is not an active lifecycle-operation deadline. The source-supported
consequence is delayed graceful-server shutdown, not spontaneous cancellation
of a driver future. The seeded witness releases the external effect and joins
normal shutdown; it never invokes `abort_for_test`, kills a Tokio task, or
fabricates crash residue. No crash/recovery defect is asserted.

The native example's `stop_serve` sends SIGTERM to its owned process group
after workload cleanup; it is not a native measurement of the CLI's SIGINT
graceful-shutdown branch (`main.rs:202` onward). The active-operation drain
consequence above is source-supported; the seeded tests exercise successful
normal joins after releasing their held effect, not a five-second shutdown SLO.

The beacon writer's stop signal is out-of-band; its two branches call
`write_all(b"SHUTDOWN\n")` and `flush` at `vm_driver.rs:773` onward. If the
deadline wins, stop aborts and joins that writer only. An interrupted EXEC
write closes the session
fail-closed; this is distinct from cancelling the convergence owner. The
normal native healthy stop observed `Ok(())` writer-call returns, not that
failure branch. Ordinary retry/re-enqueue cannot run before the serial owner
returns, as established both by source and the release controls.

## Seeded results and the historical witness correction

`crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs` uses seed
**283001**, real server boot, real HTTPS handlers, real intent/view storage,
the production broker/convergence loop and WorkloadLifecycle. Sim Clock,
ObservationStore, Dataplane, KEK and Driver are existing driven ports. The
private Driver decorator holds one external start or stop pending; it does
not author observation rows or select evaluations. The stop decorator makes
status cease being Running before holding completion, matching the relevant
VmDriver stop contract. This is a shared-owner invariant, not a simulation
of Cloud Hypervisor or a measurement of native grace.

The independent Job is submitted only after the slow effect is entered.
During ten seeded logical clock advances (about one logical second), public
HTTP cluster queries succeed but independent Running is absent. After
release, Running is observed. The HTTP/redb scheduling settle uses real 5ms
sleeps, explicitly not a latency KPI; external-effect barriers determine the
ordering, not those wall-clock sleeps. Each live test declares
`CONTRACT_SHAPE: bounded-change.`

| Run | Healthy | Slow start invariant | Slow stop invariant |
| --- | --- | --- | --- |
| First compiled attempt | pass | fail, seed printed | not run: nextest fail-fast |
| Replay with `--no-fail-fast` | pass | fail, then released control passes | fail, then released control passes |
| Final replay, observation patch removed | pass | same failure/control | same failure/control |
| Final lint-only annotation replay | pass | same failure/control | same failure/control |

Important premise correction: seed **257210 does not currently fail**.
The checked-in historical witness now sets `state.streaming_cap = 120s` at
`e09_v2_failure_stream_overlap_spike.rs:176`. Commit
`373b335c9ab623938eaa8e07992327fcc0e52d45` introduced that override after the
older RCA's 90s failure capture. Both overlap and separated cases passed in
both current executions. This falsifies P1 as stated, not the older retained
90s evidence. It is not a fix for #283 and does not prove current broker-owner
responsiveness. No stream cap or assertion was changed in this investigation.
The new full-owner witnesses avoid relying on the historical timeout result.

## Native measurements and identities

Every runtime attempt used the supplied target explicitly, the canonical
metal runner's exclusive lease, and its fail-closed non-virtualized x86_64/KVM
preflight. The host was `em-determined-roentgen`, Ubuntu kernel
`7.0.0-29-generic`, Cloud Hypervisor `v53.0`, Rust `1.95.0`. No provisioning,
fixture regeneration policy, security relaxation or broad cleanup was used.

The selected base kernel SHA-256 was
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`;
base rootfs `addf306cfbb412a12be7fb7189b3a1850bd853bb7d45266f6b1aae269393b65e`.
The example's normal preparation copies that image and installs its checked-in
server/client programs. The guest console proves the kernel executed
`/sbin/init`; final read-only debugfs extraction proved `/init`, `/sbin/init`
and freshly built current-source musl `overdrive-init` are byte-identical:
`4be39b389cddd9f4d32f7b0082a60fac0ed130a0287a79409b5b057f832a6443`.
The guest image was **not** replaced by this comparison build.

All timed runs used the default-feature CLI plus the saved observation-only
patch `docs/analysis/native-observation-283.patch`; binary SHA-256
`b0b3dfaac2c7e1181e2c89d0a132dca697f270273a875a92cb65c9c4bd892858`.
The patch only records existing boundaries/results; no additional driver
await, timeout, protocol message or control branch was introduced. It has
been removed from all production files. A final canonical build restored the
unmodified-source remote binary, SHA-256
`47a08da94f59cba6543465913cd09a2d09ad6401b02fd66b2d87063cbdd74d03`.

### Stage partition, milliseconds

Checked-in `service.toml` and `client-healthy.toml` were driven by the existing
E08 example runner; no inline workloads/specs were created. Service allocation
is `alloc-service-vm-e08-0`; the finite Job is
`alloc-service-vm-e08-client-0`. Numbers below are individual observations,
not percentiles or a supported performance target.

| Service stage | First process-traced | Second process-traced | Untraced control |
| --- | ---: | ---: | ---: |
| VMM PID | 4075088 | 4085209 | 4095960 |
| Handler enqueue → evaluation (queue only) | 101.738 | 24.314 | 18.249 |
| Evaluation → Driver start (hydrate/persist/dispatch/network/identity) | 91.791 | 91.766 | 50.169 |
| Driver host preparation → VMM create entry | 2.341 | 2.228 | 0.830 |
| VMM adapter clone/copy/spawn → create return | 12.388 | 12.251 | 10.500 |
| Create return → guest READY accepted | 4,518.459 | 5,535.323 | 1,131.665 |
| READY → actual EXEC release | 27.832 | 27.652 | 6.086 |
| Stop intent enqueue → Driver stop | 86.245 | 90.318 | 68.251 |
| Stop entry → application-level SHUTDOWN `write_all`/`flush` returned `Ok(())` | 0.432 | 0.314 | 0.119 |
| Stop entry → VMM terminate entry | 2,001.990 | 2,001.171 | 2,002.164 |
| Terminate entry → completion (timer + reap) | 10,026.042 | 10,018.599 | 10,017.545 |
| Owned artifact cleanup calls | 2.396 | 2.444 | 0.630 |
| Total Driver stop | 12,030.428 | 12,022.214 | 12,020.339 |

The application-level writer-call-return interval is **inside**, not in
addition to, the request window. Cleanup-call timings are best-effort method
completion; independent
example cleanup probes and final existence checks establish actual absence.
These native cases did not submit an unrelated workload during a held stop;
the seed-283001 invariant owns that blocked-queue proof. Native queue numbers
above are ordinary low-contention samples, not the historic 4-VM staircase.

The first trace independently records `kill(4075088, SIGKILL) = 0` at
epoch `1788983505.761533` in `healthy/process.4074948`, followed by
`wait4(4075088, ... SIGKILL ...)` at `1788983505.785195`.
Terminate entered at `19:51:35.759706Z`; this is a real ten-second wait then
roughly 24ms reap, not an already-exited process hidden by a stale watch.
The other traced run and untraced observation agree on signal 9.

Finite Job controls used PIDs 4075270, 4085389 and 4096149. All reported
Succeeded and VMM exit code 0/no signal. EXEC-release-to-reap was 112.179,
113.641 and 30.445ms respectively. The untraced console records the checked-in
client's exact-reply success, `tx EXIT`, ACPI S5 and `reboot: Power down`.
It is a natural-completion control, not proof that a live-child stop can
currently be graceful.

P4 confirmed that process tracing materially perturbed absolute start timing:
the untraced Service/Job boot intervals were 1,131.665/1,123.686ms, not the
4.5–5.5s traced values. The untraced Service kernel console reaches
`Run /sbin/init` at guest-relative 0.763472s and PF_VSOCK at 0.768552s, then
READY/EXEC. The Service console never records EXIT or poweroff; the finite
Job does. Exact subpartition of the remaining guest initialization / vsock
retry / host acceptance time was not instrumented, and guest-relative and
host wall clocks are not interchangeable. Kernel/config optimization is a
bounded follow-up question, not a proven new defect or an authorized change.

### Failure controls retained, not recast as passes

Both checked-in `tcp-startup-failure.toml` trials produced the authored
three-attempt StartupProbeFailed result. Their existing example cleanup then
returned **exit 1**, observing a replacement attempt's `bind beacon listener:
Address already in use`. All eight reported teardown deltas were zero.
These unsuccessful complete example runs are retained unchanged. They are
not additional successful acceptance evidence, and no cleanup assertion or
timeout was loosened to turn them green.

In the first failure trial PID 4079157 had an observed stop entry but no
request-window/terminate-complete markers, and was reaped by SIGKILL about
3.49s after EXEC. The source explains why the healthy Live-stop timing must
not be generalized to this partition: genuine finalization releases the
driver supervision (`action_shim/mod.rs:1770` onward), while
`VmDriver::stop` returns NotFound without a Live claim. The earlier RCA covers
that separate failure/reclamation path. This investigation does not turn the
encountered collision into new DESIGN/remediation scope.

## Multi-causal 5 Whys, with evidence stopping points

### A — unrelated convergence stalls

1. Why does independent Running not appear? Seed 283001 observes absent
   Running under both held effects, with live HTTP queries and positive
   released/no-delay controls.
2. Why cannot it get execution time? The only convergence task remains inside
   `run_convergence_tick` for the earlier evaluation; it awaits the action
   shim, which awaits the Driver effect (owner path above).
3. Why does one external operation monopolize that owner? Both the drained
   Evaluation batch and each action list are serial awaited loops. Completion,
   not submission, is their iteration boundary. A constructed TickContext
   deadline is not an enforced timeout in this path.
4. Why was serial execution retained? #260 records the earlier serial design
   and deferral discussion; #283 explicitly rejects treating that as resolution.
   This is historical evidence of the retained mechanism, not evidence for an
   organizational motive. **Stop here:** no invented fifth-level process cause.

### B — two seconds despite prompt application-level writer return

1. Why does terminate begin two seconds after stop? Native markers show the
   same ~2002ms interval with and without strace.
2. Why is the two-second interval not time spent awaiting these writer calls?
   Application-level SHUTDOWN `write_all`/`flush` returned `Ok(())` in
   0.119–0.432ms; stop nevertheless waits for the same deadline. This does not
   establish peer consumption or rule out other transport effects.
3. Why does successful completion not end the window? Both the writer-task
   completion arm and no-task arm explicitly `deadline.await` at
   `vm_driver.rs:1728`/1732. This is an unconditional minimum for a captured
   beacon writer, unlike the ten-second process grace which can end early.
4. Why is this policy present? Initial commit `8f0622a7c` called the SHUTDOWN
   writer then slept two seconds; commit `93539c74f` bounded the writer and explicitly
   preserved that grace. ADR-0082 D4's constants table describes two seconds
   as bounding the **write**, not a measured healthy minimum. The causal
   history explains persistence; it does not validate the latency choice.
   **Stop here:** no evidence supports an unavoidable two-second substrate need.

### C — the additional ten seconds reaches SIGKILL

1. Why does the grace expire? The healthy VMM remains alive until the actual
   SIGKILL at its deadline; the native reaper is not hiding an earlier exit.
2. Why did prompt writer-call return not imply guest poweroff? The byte-matched
   guest PID 1 executes blocking `Command::status()` before reading the shutdown session. A
   long-lived server child keeps that sequential call pending. Native shows
   READY/EXEC, no guest EXIT/poweroff, `Ok(())` returns from the host writer's
   application-level SHUTDOWN `write_all`/`flush` calls, and
   eventual signal 9. The guest read's non-execution is a **source-supported
   inference** combined with those observations, not a traced guest read syscall.
3. Why is the requested live-child stop outside the guest's active path?
   `complete_guest_lifecycle` has execution → EXIT → read shutdown → poweroff
   ordering. There is no concurrent shutdown-during-execution owner.
4. Why was that combination shipped? The file's current scope note at
   `overdrive-init/src/main.rs:43` explicitly deferred concurrent shutdown to
   Slice 03. ADR-0082 D4 describes PID 1 powering off after SHUTDOWN but labels
   the concurrent-supervision host→guest path as unprobed. Current production
   retains the sequential implementation. **Stop here:** no deeper motive
   or additional subsystem dependency is established.

### Forward/backward and counterfactual validation

A slow externally awaited effect + both serial loops is sufficient to block
the next independent Evaluation; the seeded release restores progress without
changing broker ordering or adding a mechanism. Independently, prompt
application-level `write_all`/`flush` return still takes B's unconditional
deadline. A still-running guest child prevents C's subsequent shutdown read,
so the otherwise early-exit-capable
VMM grace reaches kill. B + C explains measured ~12s; A turns each such cost
into serial queue amplification. The causes do not contradict one another.

Conversely, the finite child's EXIT/poweroff and normal reaped outcome rule out
an intrinsic ten-second VMM cleanup floor. Sub-millisecond cleanup plus zero
post-run artifact deltas rule out treating pre-stop resources as a leak or
attributing the twelve seconds to clone/cgroup removal. Untraced repetition
rules out ptrace as the cause of the stop windows while identifying its
material start-time bias. No claim is made about all possible hosts, failure
partitions, or production percentiles from these bounded samples.

## Bounded DESIGN/research questions for #283 and #260

- What explicit healthy start/stop millisecond budgets and failure caps are
  intended, with queueing, host preparation, guest READY, guest shutdown and
  artifact cleanup named separately? Existing 2/10/30s constants and example
  timeouts are not stakeholder latency requirements.
- How should independent allocation effects progress without blocking the
  shared convergence owner, while preserving the existing per-allocation
  ownership and completion-before-terminal guarantees? Coordinate this one
  shared execution-path decision with #260; no executor API, queue type,
  concurrency limit, persistence mechanism or cancellation protocol is chosen
  here.
- What is the exact guest live-child SHUTDOWN contract, and what completion
  ends its healthy host wait? Resolve the proven sequential guest/host mismatch
  and the minimum-two-second policy together, through DESIGN rather than a
  timeout-only change. Keep forced failure caps distinct from healthy latency.
- If the desired start budget requires reducing the measured ~1.13s untraced
  boot interval, profile the selected kernel/init/vsock subpartition with an
  approved observational experiment. Do not treat the perturbed 5s strace
  number, a proposed kernel setting, or adjacent dataplane/persistence work
  as an established required remedy.

## Commands, evidence ledger, and preservation caveat

The reproducible final seeded command is:

```sh
bash docs/analysis/capture-vm-lifecycle-283.sh sim-final-replay \
  cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests --test vm_lifecycle_latency_283_spike \
  --no-capture --no-fail-fast
```

Native runtime commands used the observation patch saved above; it must be
present to regenerate stage markers. This is a diagnostic patch, not a fix.
Do not run an unpatched helper and interpret absent markers as timing evidence.

```sh
bash docs/analysis/capture-vm-lifecycle-283.sh native-untraced env \
  OVERDRIVE_METAL_TARGET=ubuntu@151.115.99.251 \
  OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
  OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
  OVERDRIVE_METAL_SCENARIO=issue283-untraced-control \
  cargo xtask metal run -- bash docs/analysis/native-vm-lifecycle-283.sh untraced
```

The first traced invocation omitted `untraced`; the final identity/cleanup
invocation replaced it with `finalize`, after reverting observation lines.
`summarize-vm-lifecycle-283.py` accepts retained `*/logs/serve.log` paths and
prints calculated durations with the exact source line numbers. It does not
drive Overdrive or emit expectation evidence.

All paths below are workspace-relative unless explicitly `/tmp`. Every native
raw directory remains on the metal host as well as copied under `.context/`.

The existing `diagnostic-sources.tgz` files are **partial source snapshots**,
not complete diagnostic bundles. Their four members are the native runner,
capture wrapper, RCA, and Sim test as captured at that attempt. They do not
contain `native-observation-283.patch`, `observe-vm-console-283.sh`, or
`summarize-vm-lifecycle-283.py`; those separate artifacts remain in
`docs/analysis/`. The original archives are preserved byte-for-byte, not
rewritten to add missing members or the later patch-metadata correction.

| Capture | Result / retained purpose |
| --- | --- |
| `.context/issue283-seed257210.1P2LL6` | First build failed: missing workspace BPF object; no test ran |
| `.context/issue283-bpfbuild.wKHLNC` | Canonical unprivileged BPF build failed on root-owned Lima target lock |
| `.context/issue283-bpfbuild-sudo.HxDCKP` | Canonical sudo Lima BPF build succeeded; no permission/config change |
| `.context/issue283-sim-owner-attempt1.DF88HV` | Historical two controls pass; new healthy pass/start fail; stop skipped by fail-fast |
| `.context/issue283-sim-owner-replay1.RcvAXN` | Historical two pass; new healthy pass/start and stop fail |
| `.context/issue283-sim-final-replay.kfOatn` | Final uninstrumented source: healthy pass, start/stop red; wrapper exit 1, nextest exit 100 |
| `.context/issue283-sim-binary-identity.bpMpq7` | Lima environment and compiled diagnostic binary hashes |
| `.context/issue283-diagnostic-clippy.sTL779` | First focused Clippy check failed only on the mandated literal CONTRACT_SHAPE rustdoc lines |
| `.context/issue283-diagnostic-clippy-replay.G8EcML` | Focused Clippy exit 0 after documenting the narrow `doc_markdown` exemption; no assertion changed |
| `.context/issue283-sim-final-lint-only-replay.byetfd` | Final test after annotation-only edit: healthy pass, start/stop red, seed 283001 |
| `.context/issue283-sim-final-binary-identity.vtnooQ` | Final diagnostic executable SHA-256 `ad3816e9963dde5d439a43cf0c552200909d2bef3e50ae86463fe75f4b29b178` |
| `.context/issue283-native-attempt1.MNp8CL` | Initial source diff/archive and local launcher transcript; see caveat below |
| `.context/issue283-native.Uhhwly` | Complete first native raw stdout/stderr/status and per-trial logs/process traces; healthy exit 0, failure exit 1 |
| `.context/issue283-native.B9sklj` | Separately retained repeated native raw run; same healthy/failure dispositions; guest extraction |
| `.context/issue283-native-untraced.cnmuYH` / `.context/issue283-native.nwgtIF` | Untraced control exit 0; fresh guest-build identity; append-only changed console snapshots and timestamp index |
| `.context/issue283-native-finalize.ZA2CPS` / `.context/issue283-native.Z7x9kk` | Exit 0; actual `/sbin/init` byte equality, restored binary, final exact artifact absence checks |
| `/tmp/issue283-prior-remote.Zs9S9F` (local machine) | 88MB archive copy of prior remote `.context` and all expectation directories, before first canonical rsync |

Preservation caveat, explicitly an investigator error: the local capture
helper was edited while the first invocation still had it open. Bash resumed
at the shifted file offset and re-executed the native command once, printing
`-lifecycle-283.sh: command not found`; the second tee replaced the first
**local launcher** stdout/stderr. This was not a product retry. The first
native run's independent `/tmp/issue283-native.Uhhwly` stdout, stderr, status,
source digest, owner token, binary identity, healthy trial and failed trial
were not overwritten and were copied locally before any further diagnostic
work. The repeated run used a fresh canonical lease and its own
`/tmp/issue283-native.B9sklj`; both failures remain separate. This report does
not claim perfect append-only local launcher preservation. The helper was
not edited during subsequent invocations. First guest extraction also used
the wrong `/sbin/overdrive-init` path (debugfs reported missing); later
`/init` and actual `/sbin/init` extractions and byte comparisons supersede
that failed observation without erasing it.

An opportunistic early console read is retained at
`.context/issue283-second-failure-console.log`; a later read returned rsync
23 because the owned directory had already disappeared. Neither is used to
infer shutdown or kernel duration. The untraced observer's complete changed
snapshots are the console evidence used above.

## Changed files and disposition

- New diagnostic regression:
  `crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs`.
- `crates/overdrive-sim/Cargo.toml` and `Cargo.lock`: only existing workspace
  reqwest/base64/toml dev-dependency edges needed to drive real HTTPS and
  read its generated trust configuration; no package/version upgrade.
- This RCA and `docs/analysis/{capture-vm-lifecycle-283.sh,
  native-vm-lifecycle-283.sh,observe-vm-console-283.sh,
  summarize-vm-lifecycle-283.py,native-observation-283.patch}`.
- Production source is back to baseline; no behavior fix or public surface
  remains. `AGENTS.md` is external/user work, not owned by this investigation.

Rustfmt check on the new test, shell syntax checks, focused canonical Lima
Clippy with `-D warnings`, and `git diff --check` passed. The seeded runtime
check is intentionally red as detailed above;
there is no broad-suite-green claim. No commits or external issue/PR writes
were made. No E09 expectation was rerun, modified, or marked satisfied.

All example-owned materializations were removed by the existing example
cleanup, with zero VM/probe/network/cgroup/run-directory/mount/loop/preparation
deltas in every native trial. Final canonical checks independently found the
three exact allocation run directories/scopes and the E08 materialization
absent. The canonical lease owner file was absent after final runner exit;
no lease is held by this investigation. The pre-existing host fixture and
unrelated directories were preserved. Only bounded diagnostic evidence and
ordinary build artifacts remain; no workload/serve process from these runs
is intentionally left running.

## Scoped review corrections — F-01 and F-02

Following the independent `CHANGES_REQUESTED` review, only the saved patch
metadata and this RCA were revised. No production file, test assertion,
diagnostic marker, timeout, or architecture changed; no new native probe or
test run was needed.

F-01: corrected the final saved-patch hunk header from
`@@ -1754,6 +1763,8 @@` to `@@ -1754,5 +1763,7 @@`, matching its existing
five old/seven new lines. The observation content is unchanged. Ordinary
applicability verification, without `--recount`, passed:

```sh
bash docs/analysis/capture-vm-lifecycle-283.sh review-patch-applicability \
  git apply --check docs/analysis/native-observation-283.patch
```

Exit status was 0; retained stdout/stderr, command and status are in
`.context/issue283-review-patch-applicability.ax3QEu/`.
The corrected patch SHA-256 is
`7a9ec310738d3df3dd6b0ff5c4d1f6028a376e4cda8090e0e158ce0665cedf97`.
The existing archives are now explicitly labeled partial above; they were
not rewritten or supplemented in place.

F-02: result, prediction, stage table and causal wording now describe only
observed application-level `write_all`/`flush` returns. They claim neither
wire delivery nor peer consumption nor a guest-read syscall. The measured
durations and source-supported guest-ordering inference are unchanged.

These are submitted corrections, not a self-approved review verdict; the
independent reviewer owns re-review.
