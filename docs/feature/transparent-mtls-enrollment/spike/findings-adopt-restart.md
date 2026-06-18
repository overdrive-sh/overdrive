# SPIKE findings — 02-06 adopt-on-restart (increment-c)

**Feature:** transparent-mtls-enrollment (GH #236) · **Probe:** `spike-scratch/increment-c`
**Kernel (`uname -r`):** `7.0.0-22-generic` (Lima dev VM; run as root via `cargo xtask lima run --`)
**Date:** 2026-06-18

Validates the three runtime assumptions the 02-06 adopt-on-restart design rests on
(D-TME-12 "Amended 2026-06-18 (02-06 adopt-on-restart)" block in
`design/wave-decisions.md`). SPIKE-A and SPIKE-C are real-kernel runtime probes;
SPIKE-B is a code investigation.

---

## Verdicts

| Probe | Question | Verdict |
|---|---|---|
| **SPIKE-A** | Does a `setsid()` + `kill_on_drop(false)` + cgroup-scoped workload SURVIVE its parent ("serve") being SIGKILL'd? | **WORKS — survives** |
| **SPIKE-B** | Does `serve` boot re-drive already-`Running` allocs (firing the C3 slot-rebuild)? | **CONFIRMED: it does NOT** — a dedicated adopt-on-restart pass IS needed |
| **SPIKE-C** | Does `/proc/<survivor-pid>/ns/net` recover the `ovd-ns-<slot>` binding after the parent is dead? | **WORKS — slot recovered** |

**One-line gate recommendation:** **PROCEED-AS-DESIGNED** — all three assumptions hold; the 02-06 design (adopt-not-reassign, dedicated boot pass, recover bindings via cgroup→PID→`/proc/ns/net`) is validated, no pivot needed.

---

## SPIKE-A — workload survival (WORKS)

**Hypothesis:** the production ExecDriver shape (`setsid()` driver.rs:414, `kill_on_drop(false)` :395, cgroup-v2 scope) makes the workload outlive a CP crash.
**Prediction:** after SIGKILL of the parent, the workload is alive, reparented to init (ppid=1), still in its cgroup scope.
**Actual (pasted from the run):**

```
=== EVENT: SIGKILL the "serve" (CP) intermediary pid 1537025 ===
  kill(1537025, SIGKILL) -> 0
  reaped serve (status raw = 9)
  serve alive after kill (kill -0): false

=== SPIKE-A: workload survival AFTER serve death ===
  workload 1537026 alive (kill -0): true
  /proc/1537026/stat exists: true
  /proc/1537026/stat state=S ppid=1
  $ ps -o pid,ppid,sid,stat,comm -p 1537026
          PID    PPID     SID STAT COMMAND
      1537026       1 1537026 Ss   sleep
```

**Confirmed:** the workload survives, reparented to **ppid=1**, `Ss` (own session via setsid), still listed in the scope's `cgroup.procs` post-kill. A CP restart leaves the workload running in its old `ovd-ns-<slot>` netns — so the allocator MUST re-adopt the survivor's slot, never re-assign smallest-free onto it.

> Independent corroboration: the run's cleanup reaped a **leftover `sleep` pid 1536861** from an earlier hung probe attempt — i.e. a workload that had already survived a prior parent death. SPIKE-A reproduced twice.

---

## SPIKE-B — boot re-adoption behavior (CONFIRMED: reconciler does NOT re-drive survivors)

**Code investigation** (no runtime probe needed — the decision is in pure reconcile logic):

`WorkloadLifecycle::reconcile` (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`)
emits `Action::StartAllocation` **only** in the branch gated "No Running, no
failed-needs-restart → schedule a fresh allocation" (`:708-746`). An alloc that is
already `Running` in `actual.allocations` (a survivor observed after CP restart)
matches the Running branch and emits **no** Start action — the desired replica
count is already met by the survivor.

**Consequence:** on a CP restart the action-shim `StartAllocation` arm does NOT
re-fire for survivors, so the C3 `on_alloc_running` slot-assign (the in-RAM
allocator rebuild) **never runs**. The C6 wording "rebuilt on restart by
re-assigning for every still-Running alloc" has **no trigger** — re-assignment via
the normal reconcile path does not happen. This is exactly why a **dedicated
adopt-on-restart boot pass (02-06)** is required, not optional: nothing else
rebuilds the slot↔alloc map.

(Corroborated by session observation #45573, 2026-06-18.)

---

## SPIKE-C — PID→netns slot recovery for a survivor (WORKS)

**Hypothesis:** `/proc/<pid>/ns/net` inode recovers the `ovd-ns-<slot>` binding for a SURVIVOR (the in-tree `read_proc_netns_inode` precedent is proven only for a freshly-spawned child with a live parent).
**Prediction:** post-kill, `/proc/<survivor>/ns/net` inode == `stat(/var/run/netns/ovd-ns-7e57).st_ino`, and `ip netns identify` recovers the name.
**Actual (pasted from the run):**

```
=== SPIKE-C: PID->netns slot recovery for the SURVIVOR ===
  /proc/1537026/ns/net inode (post-kill) = Some(4026532488)
  stat(/var/run/netns/ovd-ns-7e57).st_ino                     = Some(4026532488)
  $ ip netns identify 1537026
      ovd-ns-7e57
  inode match (/proc ns/net == stat netns file): true
  `ip netns identify` recovered: "ovd-ns-7e57" (== ovd-ns-7e57? true)
```

**Confirmed:** the inode match holds for the survivor after its parent is dead, and
`ip netns identify` recovers the slot name. The 02-06 binding-recovery mechanism
(correlate surviving PID → its netns → `ovd-ns-<slot>`) is viable on this kernel.

---

## Edge cases / notes

- **The probe itself reproduced the inherited-fd hang** that the design must avoid
  in production: the survivor inherited the pipe write-end (`libc::pipe` lacked
  `O_CLOEXEC`) and the parent's `read_to_string` blocked for an EOF that never came
  until the survivor died. Fixed in the probe with `pipe2(O_CLOEXEC)` + workload
  `Stdio::null()`. **Design implication:** the production ExecDriver already
  detaches workload stdio, but any future supervisor/IPC that pipes to a survivor
  must close-on-exec / null the survivor's fds — a survivor holding a parent's pipe
  is a real hang vector.
- **cgroup.procs survives the parent SIGKILL** (the survivor remained enrolled),
  so the cgroup-scope walk (enumerate `alloc-<id>.scope` → PIDs) is a viable
  observe-actual surface for the 02-06 adopt pass.
- **Slot reuse hazard validated by implication:** since the survivor keeps
  `ovd-ns-7e57` and the in-RAM allocator starts empty on restart, an empty
  allocator re-assigning slot 0 onto a fresh alloc would collide with any survivor
  that had slot 0 — the B1 collision the adopt pass prevents.

## Design implications for 02-06

1. **Adopt, do not re-assign** (SPIKE-A): survivors keep their netns; the boot pass
   must re-claim each survivor's existing slot before any smallest-free assign.
2. **A dedicated boot pass is mandatory** (SPIKE-B): the reconciler will not rebuild
   the map; 02-06's `run_server` recovery pass is the only trigger.
3. **Recovery via cgroup→PID→`/proc/ns/net` is sound** (SPIKE-C): enumerate
   `overdrive.slice/workloads.slice/*.scope` → `cgroup.procs` → `/proc/<pid>/ns/net`
   inode → match against `/var/run/netns/ovd-ns-<slot>` → `NetSlotAllocator::adopt`.
4. Orphan GC: an `ovd-ns-<slot>` with no live PID in any scope is an orphan → tear
   down (the survivor-less leak).

## Cleanup

The probe is idempotent and self-cleaning: it killed both the survivor and the
leftover prior-run `sleep`, `rmdir`'d the cgroup scope, and `ip netns del`'d the
test netns. Post-run confirmation: `ip netns list` empty, workload dead, scope dir
gone. No kernel residue left in the Lima VM.
