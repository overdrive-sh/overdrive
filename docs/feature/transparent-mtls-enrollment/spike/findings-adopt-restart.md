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

---

# SPIKE-D — the nft-rule twin of the survivor problem (increment-d, 2026-06-20)

**Probe:** `spike-scratch/increment-d` · **Kernel (`uname -r`):** `7.0.0-22-generic`

Settles the SPIKE-D residual the design left open (wave-decisions.md § "Spike
boundary", SPIKE-D bullet): **do the per-workload nft-TPROXY rules in the shared
`overdrive-mtls prerouting` chain SURVIVE a real `serve` restart, and does the
by-handle delete fire on a guard-less survivor leaving only the shared infra?**
Self-contained nft/ip CLI probe (constants copied from `mtls_intercept.rs`, NOT a
crate dep), replicating the production `ensure_shared_routing_infra` +
`install_outbound_tproxy` (step 3) install and the `TproxyInterceptGuard::Drop`
by-handle delete verbatim.

## Verdicts

| Q | Question | Verdict |
|---|---|---|
| **Q1** | Do per-workload nft-TPROXY rules SURVIVE a `serve` restart on this kernel? | **WORKS — SURVIVES** |
| **Q2** | Does by-handle `nft delete rule … handle <N>` fire on a GUARD-LESS survivor, leaving ONLY the shared infra (F5 exemption)? | **WORKS** |

**One-line gate:** §5 reconcile is REQUIRED (rules survive ⇒ not a no-op) and the
sweep-by-handle MECHANISM is sound on this kernel. **BUT the §5 implementation is
a 04-04 BOUNDARY BLOCKER** — see § "Boundary consequence".

## Hypothesis / prediction / falsification

- **Q1.** H: the egress rule is APPENDED to a node-global chain production never
  tears down per-workload, so it persists when the owner dies. Prediction: the
  rule is in the re-dump after the restart. Falsification: absent from re-dump.
- **Q2.** H: the kernel handle is recoverable from `nft -a` (`# handle <N>`) and
  `nft delete rule … handle <N>` removes exactly that rule, keeping the F5
  exemption. Prediction: post-delete dump has F5, not the egress rule.
  Falsification: delete fails / removes F5 / leaves the egress rule.

## Pasted evidence (real run, kernel 7.0.0-22-generic)

```
=== SPIKE-D Q1: do the per-workload rules SURVIVE? (re-dump after the 'restart') ===
    table ip overdrive-mtls {
    	chain prerouting { # handle 1
    		type filter hook prerouting priority mangle; policy accept;
    		meta mark 0x00000002 accept # handle 2
    		iifname "ovd-hv-d00d" meta l4proto tcp tproxy to 127.0.0.1:41001 meta mark set 0x00000001 accept # handle 3
    	}
    }
  Q1 VERDICT: SURVIVES — the egress rule is still in the chain after process death

=== SPIKE-D Q2: by-handle delete on the GUARD-LESS survivor ===
  recovered survivor handle = '3'
  $ nft delete rule ip overdrive-mtls prerouting handle 3
  delete rc = 0
  chain AFTER by-handle sweep:
    table ip overdrive-mtls {
    	chain prerouting { # handle 1
    		type filter hook prerouting priority mangle; policy accept;
    		meta mark 0x00000002 accept # handle 2
    	}
    }
  Q2 VERDICT: WORKS — per-workload rule deleted by handle; F5 exemption (shared infra) intact

=== SUMMARY ===
  Q1 (rules survive restart) = SURVIVES
  Q2 (by-handle del on guard-less survivor) = WORKS
```

(`meta mark 0x00000002 accept` is the probe's stand-in for the F5
`MTLS_LEG_S_DIAL_MARK` exemption — its exact value is immaterial to the
survival + by-handle-delete questions; the probe only needs *a* shared-infra rule
at the chain head to prove the sweep keeps it. Production renders
`overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK` identically.)

## Boundary consequence — §5 is a 04-04 BOUNDARY BLOCKER (NOT implemented this step)

SPIKE-D confirms §5 is REQUIRED and its mechanism sound. But the design pins the
sweep to re-use "the landed by-handle delete + the landed dump-parse predicates
**verbatim — no new public surface**" (wave-decisions.md §5). Ground-truth of the
LANDED code contradicts the "no new public surface" premise:

- The sweep machinery lives in **`crates/overdrive-worker/src/mtls_intercept.rs`**
  — NOT in 04-04's `files_to_modify` (only `veth_provisioner.rs`, `lib.rs`, and
  the two test files are in scope).
- The dump-parse predicates the sweep must re-use are **PRIVATE**:
  `dump_has_egress_rule` (`:663`), `find_egress_rule_handle_in_dump` (`:640`),
  `dump_has_leg_s_exemption` (`:554`) are all `fn`, not `pub fn`; `NFT_TABLE` /
  `NFT_CHAIN` (`:56` / `:61`) are private `const`s; the by-handle delete is inside
  `TproxyInterceptGuard::Drop` (`:687-696`) with no standalone public delete entry.
- A sweep driven from `lib.rs`/`veth_provisioner.rs` (04-04's named files) thus
  REQUIRES either making those predicates+delete `pub` in `mtls_intercept.rs`, OR
  adding a new `pub fn sweep_per_workload_tproxy_rules(...)` there. BOTH add NEW
  public surface to a file OUTSIDE 04-04's boundary — exactly what BOUNDARY_RULES
  and CLAUDE.md § "Implement to the design — never invent API surface" forbid.

**This is NOT the SPIKE-D negative branch** (which would have collapsed §5 to a
no-op). The rules DO survive; the sweep IS needed; it cannot be built within
04-04's file boundary without inventing a new public sweep surface in
`mtls_intercept.rs`. Per the dispatch BOUNDARY instruction, §5 is surfaced as a
blocker, not built past the boundary. The netns half (§1–§4) is fully in-boundary
and IS implemented in 04-04.
