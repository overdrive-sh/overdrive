# Findings: waiting on `cgroup.events` instead of retrying `rmdir` (ADR-0349, RB-3)

Probe: `spike-scratch/node-local-persistent-volume-cgroup-events/` (own `Cargo.toml`, not a workspace member). Raw evidence:
`spike-scratch/node-local-persistent-volume-cgroup-events/evidence/run8-filtered.txt` (SPIKE lines of metal run `5fd61383377c29e4dd0a9c29`).

## Verdict: WORKS, and the finding changes the ADR's premise

- **The mechanism works.** A waiter on `cgroup.events` (`poll(POLLPRI)` or inotify `IN_MODIFY`) was woken for `populated 0` in 20/20 held trials, within
  -0.25 to +0.15 ms of a 200 us polling oracle, never early, and `rmdir` issued on the wake returned `Ok` in 0.07 to 0.44 ms in 20/20. A 250 ms poll
  timeout kept firing on schedule (5 to 6 times per 1.5 s hold), so the wait is cancellable by a timer.
- **The premise did not reproduce.** On the guest kernel `rmdir` never slept. With the scope's last task held in `do_exit` (PF_EXITING, D state), `rmdir`
  returned `Ok` in about 1 ms, 20/20 trials, at 0, 10, 50 and 500 ms after the kill. The scope directory was removed while the task was still alive and
  still holding the device. `populated` and `rmdir` therefore disagree on this kernel, and `kill_scope`'s `Ok` ("scope absent") does not mean the task is gone.

Kernel: `uname -r` inside every guest = `7.0.0-38-generic` (`/proc/version`: Ubuntu build `#38-Ubuntu SMP PREEMPT_DYNAMIC Fri Sep 4 09:10:14 UTC 2026`;
the `cgroup-default` / `volume-pool` guest runs the metal host's kernel). The vendored tree is the `v7.0` tag (`028ef9c96e96`); the running kernel is a
distro build of the 7.0 series, so the two are not the same source (see "Open").

## Placement

Every test carries `#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]` from `overdrive-testing` (path dependency). The wrapper
(`infra/metal/test-wrapper.sh`) reads the placement record from the test ELF regardless of workspace membership, so `cargo xtask metal run -- cargo nextest
run --manifest-path spike-scratch/<dir>/Cargo.toml -j 1` boots one fresh guest per test. The `volume-pool` profile guest has `dmsetup`, `losetup`,
device-mapper (major 252) and cgroup v2 at `/sys/fs/cgroup`. It has no `udev`, so the probe creates the `/dev/mapper` node itself from `dmsetup info -o major,minor`.
The guest kit was prepared with `cargo xtask metal run -- cargo xtask guest-kit-prepare`. No change to `crates/` was needed; nothing ran on the metal host.
The host-pure base profile has no coreutils, which is why the diagnostic that chose the profile used `std::fs`.

## Hypothesis, prediction, falsification (written before the runs)

- Hypothesis: `populated 0` is set in the same `css_set_lock` section that wakes `rmdir`'s dying-task wait, so it never reads 0 while `rmdir` would sleep.
- Prediction: held in exit, `populated 1` and a concurrent `rmdir` asleep in D state; on resume both settle within milliseconds; `rmdir` after `populated 0`
  returns at once in every trial; the poll waiter times out on schedule while `rmdir` is stuck.
- Falsification: a trial with `populated 0` followed by a blocking or `EBUSY` `rmdir`; a waiter that missed a `populated 0` the oracle saw; `populated 0`
  while the child was alive.

## Source (vendored `v7.0`, `vendors/linux`)

- `cgroup.events` `populated` is `cgroup_is_populated(cgrp)` (`kernel/cgroup/cgroup.c:3817-3824 @ v7.0`), the same predicate `cgroup_destroy_locked` refuses on
  (`:6168-6169`). It counts own csets plus populated children (`:839-868`). It cannot read 0 while `cgroup_destroy_locked` would return `-EBUSY` on
  population.
- `rmdir`'s sleep is `cgroup_drain_dying` (`:6254-6297`), entered only while `cgroup_has_tasks` (`nr_populated_csets`, `:391-394`) holds and the iterator
  sees nothing. `populated` goes to 0 in `do_cgroup_task_dead` (`:7072-7101`): `css_set_move_task` -> `css_set_update_populated` -> `cgroup_update_populated`
  (`:839-868`, `:905-935`), then the `dying_populated_waitq` wake, all under one `css_set_lock` hold. `cgroup_task_dead` runs from `finish_task_switch`
  (`kernel/sched/core.c:5188-5193 @ v7.0`). So by v7.0 source `populated 0` and the end of the drain are the same event.
- Delivery: `cgroup_update_populated` calls `cgroup_file_notify` (`:864`), rate-limited to one `kernfs_notify` per `CGROUP_FILE_NOTIFY_MIN_INTV`
  (`DIV_ROUND_UP(HZ, 100)`, `:69`); a notification inside the window is deferred to a timer (`timer_reduce`, `:4688-4705`), so the last state is always
  delivered, up to about 10 ms late. `kernfs_notify` bumps `on->event` and wakes pollers immediately, and queues `fsnotify(FS_MODIFY)` on a work item
  (`fs/kernfs/file.c:979-1013`, `:912-977`), so `IN_MODIFY` is asynchronous. `poll` reports `EPOLLPRI|EPOLLERR` when `of->event != on->event`
  (`:855-865`): the file must be read after arming, then tested, then polled.

## Predicted vs actual

| Claim | Predicted | Actual (guest `7.0.0-38-generic`) |
|---|---|---|
| Held in exit: `populated` | stays 1 | stays 1 for the whole 1.5 s hold, 40/40 (A2) |
| Held in exit: task | D state, invisible to `cgroup.procs` | `state=D wchan=folio_wait_bit_common flags=...PF_EXITING`, `procs=[]`, `events=[populated 1]` |
| Held in exit: concurrent `rmdir` | sleeps in D state | **returns `Ok` in 0.72 to 1.09 ms, 20/20; scope removed; task alive** |
| After that `rmdir`: the task | gone with the scope | alive in `0::/cgev.slice/A-N.scope (deleted)`, `dm ... open=1`, reaped 3.0 to 3.8 ms after resume |
| Held in a running write (not exiting) | `rmdir` `EBUSY` | `EBUSY` 10/10; `populated 1` until resume; `wchan=submit_bio_wait` |
| `populated 0` after resume | within ms | 2.1 to 3.9 ms after resume (median 3.56) |
| `populated 0` versus task exit | simultaneous | `reaped - populated0`: -0.25 to +0.11 ms, median -0.04 (the zombie can be reaped just before) |
| `populated 0` while task held | never | 0/20 early |
| Waiter wake (poll / inotify) versus oracle | prompt | poll -0.24..+0.05 ms; inotify -0.17..+0.12 ms; 0/20 missed |
| `rmdir` right after `populated 0` | immediate `Ok` | `Ok`, 0.04 to 0.44 ms; 0 violations in A2 odd trials (20) and C (40) |
| Bounded wait | poll times out while stuck | 6 timeouts of 250 ms in each 1.5 s hold |
| Control, no held I/O | settles in ms | kill to `populated 0` 0.85 to 1.13 ms; 0 violations |

Pasted output (run `5fd61383377c29e4dd0a9c29`):

```
SPIKE scen=A trial=0 HELD-BEFORE-RESUME state=D wchan=folio_wait_bit_common flags=4195404 events=[ERR No such file or directory (os error 2)] procs=[] dm=[cgev-a:L-sw:1] child_cgroup=[0::/cgev.slice/A-0.scope (deleted)]
SPIKE scen=A trial=0 timeline      0.86 ms  rmdir_thread_start delay=0 pre: state=R wchan=0 flags=4195404 events=[populated 1;frozen 0;]
SPIKE scen=A trial=0 timeline      1.09 ms  rmdir_thread_end errno=0 post: state=D wchan=folio_wait_bit_common flags=4195404 cgroup_of_child=[0::/cgev.slice/A-0.scope (deleted)]
SPIKE scen=A trial=0 timeline   1501.52 ms  resume
SPIKE scen=A trial=0 timeline   1504.87 ms  reaped sig=9
SPIKE verdict A.rmdir_returned_Ok_before_resume_while_task_still_held: HOLDS 20/20
SPIKE verdict A.task_still_alive_after_that_rmdir_until_resume: HOLDS 20/20
SPIKE scen=A2 trial=0 HELD-BEFORE-RESUME state=D wchan=folio_wait_bit_common flags=4195404 events=[populated 1;frozen 0;] procs=[] dm=[cgev-a2:L-sw:1] child_cgroup=[0::/cgev.slice/A2-0.scope]
SPIKE stat A2(even, no rmdir) resume->populated0 ms (oracle): n=20 min=2.09 median=3.55 max=3.86
SPIKE stat A2(even) oracle_pop0->poll_pop0 ms (poll POLLPRI waiter latency): n=20 min=-0.24 median=-0.13 max=0.05
SPIKE stat A2(even) oracle_pop0->inotify_pop0 ms (inotify IN_MODIFY waiter latency): n=20 min=-0.17 median=-0.05 max=0.12
SPIKE verdict A2.no_waiter_missed_populated0: HOLDS missed=0/20
SPIKE verdict A2.populated0_never_before_resume_while_task_held: HOLDS early=0/20
SPIKE verdict A2.rmdir_after_populated0_never_blocks_or_EBUSY: HOLDS violations=0/20
SPIKE verdict B.rmdir_EBUSY_while_task_visible: HOLDS EBUSY=10/10
SPIKE verdict C.rmdir_after_populated0_never_blocks_or_EBUSY: HOLDS violations=0/40
```

"Race versus rule": the early-`Ok` `rmdir` is a rule on this kernel (20/20, four start delays including 0 ms). `populated 0` after the task is gone and
never before is also a rule (0/20 early, 0/20 missed, 0/60 `rmdir` failures after it). The sub-0.3 ms ordering between the waitpid return and
`populated 0` is a race and varies in sign.

## Edge cases

- **Do not `rmdir` while `populated 1`.** On this kernel it deletes the scope with a live, held task in it. Everything keyed on the directory (`observe()`,
  the lease gate) then reads "gone" while the VMM still holds the device. The scope's `cgroup.events` is unreadable after that (`ENOENT`), so the wait
  must come first. A poll on a removed file returns at once (the probe's first run spun on this).
- **A task held in a running write** (not exiting) keeps `rmdir` at `EBUSY` and `populated 1`; `populated 0` still arrives 2.4 to 3.9 ms after resume.
- Read, then test, then poll (the `kernfs` event counter), or a change between the first read and the wait is lost. The inotify variant armed the watch
  before its first read.
- `populated` counts descendants. A scope with child cgroups reads `populated 1` while any of them has a task; `kill_scope`'s scopes have none.
- The hold reproduced is the close of a suspended dm device with dirty pages (`sync_blockdev` in `exit_files`). Other holds were not tried: an NFS-style
  or FUSE hold, a `D` state before `PF_EXITING` for `cgroup.kill`, or a hold inside `exit_mm`.
- The 1.5 s hold bounds what was measured; a hold of minutes was not run.

## What it means for ADR-0349 and the feature delta

1. The motivating hazard (an `rmdir` that never returns while the VMM is held) was not observed on the running kernel; a kernel with the v7.0
   `cgroup_drain_dying` would show it, and this one does not behave like that source.
2. The ADR's assumption K-C7 ("the scope is absent only after the device is released") is **false on this kernel** for a task held in exit: the scope
   was absent for the whole 1.5 s hold, until the device was resumed and the task finished exiting. Gating the lease release on `observe()` reporting the scope
   absent releases the lease while the VMM is alive.
3. `kill_scope` should be: write `cgroup.kill`; wait for `populated 0` on `cgroup.events` (arm, read, test, `poll(POLLPRI)`) bounded by `KILL_SCOPE_SETTLE_BOUND`
   on a timer; then `rmdir` (expected `Ok` immediately; `EBUSY` is a real error, not a retry); at the bound return `ResourceBusy` and leave the scope
   in place. The "one blocked removal per scope" `ClaimSet` and the blocking-pool `rmdir` are not needed, because nothing blocks. The port contract's
   postcondition should read: `Ok` means `populated 0` was observed and the directory removed, so no task of the scope is left. The lease gate can then
   stay "scope absent", but only because `kill_scope` is the sole remover; any other `rmdir` of a scope (for instance the metal wrapper's own cleanup, which already
   waits for `populated 0` first) must do the same.
4. Cost: one open fd and one waiter per busy scope, no thread blocked in the kernel, cancellable.

## Open

- The running kernel's cgroup source is not vendored, so the early-`Ok` `rmdir` is observed, not read. `vendors/linux` at `v7.0` contains
  `cgroup_drain_dying` and `cgroup_destroy_locked`'s populated check, which predict `EBUSY` or a sleep, not `Ok`; `kallsyms` of the guest has no
  `cgroup_drain_dying` symbol (static, so it may be inlined: not evidence either way). The design rule "cite `vendors/linux` at `v7.0`" is not enough for
  this claim; either vendor the exact distro source of `7.0.0-38` or pin the appliance kernel to a build whose source is vendored. Decision for the user.
- Whether the appliance kernel (ADR-0068) has the same `rmdir` as this Ubuntu build is unknown; the design must not depend on `rmdir` sleeping or
  returning early, which the `populated`-first order achieves.

## Recommendation

PROMOTE the `cgroup.events` wait, rewritten as "kill, wait `populated 0` with a timer bound, then `rmdir`", and amend ADR-0349 and item 11's K-C7 to say the
scope is removed only after `populated 0`, not that `rmdir` sleeps.
