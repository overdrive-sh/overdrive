# Slice 03 — The hypervisor process as a managed host process: stop, restart, death, and confinement

> DISCUSS brief (2026-08-01; amended 2026-08-02 — US-VM-7 folded in, re-sized 2–3 d →
> **4–6 d**). Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Stories: **US-VM-3**, **US-VM-4**, **US-VM-7**. Job: **J-OPS-003**. Gated by Slice 01
> (and by Slice 02 for US-VM-7's fail-closed reason vocabulary).
>
> **`superseded-by-DESIGN` (2026-08-11, GH #42).** Three statements below are
> corrected in place; the governing text is named at each site and **governs
> where this file and it disagree**. **C-1** — the fresh per-restart copy is the
> **`FICLONE` ioctl**, not `cp --reflink=auto` (ADR-0082 § D2 / brief § 102).
> **C-4** — the Landlock ruleset's **only** explicitly-declared rule is the
> **vsock socket's containing directory**; CH auto-derives the three paths this
> slice previously named (ADR-0082 § D2.2). **C-6** — `RLIMIT_FSIZE` is
> `max(rootfs image, guest RAM)`, encoded from Slice 01 (ADR-0082 § D2). The
> **graceful-shutdown mechanism** is also now pinned to two constants —
> `VM_SHUTDOWN_REQUEST_DEADLINE` (2 s) and `VM_STOP_GRACE` (10 s), ADR-0082 § D4
> — and this slice's Tier-3 stop AC is the **first evidence** for the host→guest
> `SHUTDOWN` write, which the spike never exercised (`findings.md:2787`).

## Goal (one line)

`overdrive job stop` gracefully shuts a VM down, backoff restarts a crashed one from a
**clean** rootfs, a hypervisor that exits without an agent report is classified `Crashed`
— **never** `CleanExit` — and the hypervisor process itself runs **confined**, or the
workload does not run.

## Why these three stories are one slice

The slice has one subject: **the `cloud-hypervisor` process treated as a managed host
process.** US-VM-3 classifies its death, US-VM-4 ends it deliberately, US-VM-7 bounds what
it can do while alive. They share the observation surface (`/proc/<vmm-pid>/…` against a
live allocation) and the same Tier-3 harness. Splitting confinement into its own slice was
unavailable anyway — the slice-composition gate forbids a slice whose every story is
`@infrastructure`, and a lone hardening story is exactly that shape. Here it sits beside two
operator-visible stories and the gate holds.

## Learning hypothesis

The existing exit-observer and restart/backoff machinery
(`worker/exit_observer.rs:610-636`, `workload_lifecycle.rs:673-743`) needs **no change**
for VM workloads — provided `ExitKind` is sourced from the guest agent's report and the
unreported-death case is classified distinctly at the driver.

**Predicted:** VM crash-restart is byte-for-byte the same reconciler path as exec
crash-restart, with the same ceiling and the same backoff.

**Second hypothesis (US-VM-7):** Cloud Hypervisor's own `--landlock` plus ordinary
`Command` uid/gid and `setrlimit` controls close the *filesystem and resource* half of a
jailer's job without building a chroot tree — so the `[D7]` posture is reachable with
spawn-time flags and no jail-construction machinery.

**Predicted:** the same VM that boots in Slice 01 boots identically with all three applied,
and a sentinel path outside the ruleset is denied to the live hypervisor.
**Falsified if** Slice 00 P5 shows the uid-dropped process cannot reach `/dev/kvm`, or the
ruleset cannot be derived from the spec's declared paths — in which case `[D7]`'s claim is
restated before the story is built.

## Thinnest `serve` + `deploy` loop

`overdrive serve` + `overdrive deploy` + six Tier-3 cases:
(1) `overdrive job stop` on a running VM; (2) a guest that ignores shutdown requests;
(3) a guest that kernel-panics while the hypervisor exits **0**; (4) a crash-restart whose
guest previously wrote garbage into its rootfs; (5) a running VM whose hypervisor PID is
read out of `/proc` for uid/gid, rlimits and a denied sentinel-path access; (6) a host that
cannot supply the required confinement.

## Behavior (DESIGN owns the API)

- **`ExitKind::CleanExit` for a VM comes only from an agent-reported guest exit status.**
  No code path derives it from the `cloud-hypervisor` process's exit status. *This is the
  named anti-pattern* — intake precedent warning #3: the reference implementation's
  `wait()` observed the host process, so a guest that boots, panics, and shuts down
  cleanly reported success.
- A VMM exit with **no** agent report → `ExitKind::Crashed` with a reason naming the
  unreported guest death.
- An operator stop → `intentional_stop: true`, no restart budget consumed.
- `Driver::stop` requests a graceful guest shutdown, escalating to a hard kill after a
  bounded grace period; an unresponsive guest still lands
  `Terminated / Stopped{by: Operator}`, never a crash.
- A restarted allocation boots from a **fresh clone** of the operator's rootfs; the
  operator's artifact on disk is byte-unchanged. **C-1 correction (DESIGN, ADR-0082 § D2 /
  brief § 102):** the clone is the **`FICLONE` ioctl issued directly**, **not**
  `cp --reflink=auto` — `auto` silently degrades to a full copy on a filesystem that cannot
  reflink (0.015 s / +0 MiB versus 3.970 s / +4096 MiB, ~260×, P4) with no error anywhere,
  and the ioctl has no such path. The clone lands on the rootfs master's own filesystem and
  its filename carries the allocation id.
- DESIGN pins the ordering where the agent's report and the VMM's exit race, so a reported
  exit is never overwritten by the subsequent teardown.
- **US-VM-7 — `[D7]` items 1–3 on the spawned hypervisor:** `--landlock` with a ruleset
  derived from **the paths this spec declares** — not a hardcoded artifact directory; a
  **uid/gid drop** to a non-root identity that retains `/dev/kvm`; and `setrlimit` on
  `RLIMIT_FSIZE` and `RLIMIT_NOFILE`.
  **C-4 correction (DESIGN, ADR-0082 § D2.2):** the three paths this slice previously
  named — kernel, per-launch rootfs copy, API socket — are **exactly the ones CH
  auto-derives** (along with `--serial file=`). The one path that needs an explicit rule is
  the **vsock socket's containing directory**, which CH does **not** auto-derive for the
  socket it binds itself; the failure is `CreateVsockBackend(UnixBind(EACCES))` and never
  mentions Landlock (P5). A read-only rule is insufficient, and the rule **cannot name the
  socket path** (CH validates rule paths for existence at config-parse time, before the
  socket exists), so the grant is `access=rw` on the **per-allocation run directory** — the
  reason that directory must hold nothing else (SD-2). `VmRunDir::landlock_grant()` is the
  only producer; there is no path list to get wrong.
  **C-6 correction (DESIGN, ADR-0082 § D2):** `RLIMIT_FSIZE` is
  **`max(rootfs image, guest RAM)`**, encoded from **Slice 01** rather than derived here.
  `--memory shared=on` (Slice 04) backs guest RAM with a memfd, and a memfd is a *file* for
  `RLIMIT_FSIZE` — a limit sized off the rootfs alone kills every volume-carrying VM with
  an opaque `SIGXFSZ`. The AC below (*"finite … strictly lower than `overdrive serve`"*)
  is unchanged and still binding; the `max` is the **floor** it must clear.
  **Fail-closed:** a host that cannot supply them yields a distinct `Failed` reason — a
  **fifth** variant minted in Slice 02's shape, not a reuse of one of that slice's four
  (US-VM-2's "no two share a variant" holds) — never a hypervisor started with confinement
  degraded.
- **Not here:** cgroup + netns placement and the seccomp-not-weakened constraint are
  **Slice 01** (inherent to `VmDriver::start`); the **mount namespace is not in this
  feature** — GH [#258](https://github.com/overdrive-sh/overdrive/issues/258) **amended
  2026-08-02 to own it** — see feature-delta `[D7]`. The **storage daemon** introduced by
  `[D8]` is **Slice 04**: this slice classifies the *hypervisor's* death, US-VM-9
  classifies `virtiofsd`'s under the same rule.
- **Forward-compatible by construction.** The classification this slice builds is stated as
  a general rule in **system constraint 9** — *a supervised sidecar's death is classified by
  the WORKLOAD's outcome, never by the sidecar's own exit status* — precisely so Slice 04
  extends it rather than minting a parallel model for `virtiofsd`. If the shape written here
  cannot absorb a second supervised process, that is a signal to reshape it now, not to fork
  it later.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — the operator stop verb and real crashes
  through `serve` + `deploy`, not a test-invoked `Driver::stop`.
- **Thinnest?** Yes — no new subsystem; the reconciler, observer and backoff are reused
  unchanged, and US-VM-7 reuses Slice 02's failure vocabulary. The work is classification,
  shutdown sequencing, and spawn-time confinement flags. **Now the second-largest slice at
  4–6 d**, up from 2–3 d, on the `[D7]` fold. *(Third-largest after Slice 04's re-budget to
  6–9 d.)*
- **Delivers operator-visible value alone?** Yes — lifecycle parity is the whole promise
  of "one control plane, all workload types."
- **Every story operator-visible?** Yes, so the slice-composition gate holds: US-VM-3 and
  US-VM-4 are plainly so, and US-VM-7's operator surface is its **fail-closed** reason —
  Ana sees `Failed` naming the unavailable confinement rather than a VM quietly running
  weaker than `[D7]` promises.
- **Guards against the most likely lie?** Yes. US-VM-3 is the feature's north-star KPI
  (K1) and its classification arm is a **mandatory mutation-testing target**: a mutation
  collapsing the unreported-death arm into `CleanExit` must be killed.

## Acceptance (= US-VM-3 + US-VM-4 + US-VM-7 ACs)

- [ ] `CleanExit` for a VM is produced **only** from an agent-reported guest exit status.
- [ ] A VMM exit with no agent report yields `Crashed` — Tier-3 case where the guest dies
      and `cloud-hypervisor` exits **0**.
- [ ] An operator stop yields `intentional_stop: true` and consumes no restart budget.
- [ ] VM crash-restart matches exec crash-restart: same reconciler, same ceiling, same
      backoff.
- [ ] The classification arm is covered by mutation testing; the `CleanExit`-collapse
      mutation is killed.
- [ ] `overdrive job stop` requests graceful guest shutdown before any hard kill.
      **This AC is the FIRST evidence for the host→guest `SHUTDOWN` write** — the spike
      exercised the vsock connection **guest→host only** (`findings.md:2787`), so no probe
      ever wrote host→guest nor had a guest agent read while supervising a child. Treat it
      as a mechanism proof, not a regression guard (ADR-0082 § D4).
- [ ] An unresponsive guest terminates within a bounded grace period and lands
      `Terminated / Stopped{by: Operator}` — bounded by `VM_SHUTDOWN_REQUEST_DEADLINE`
      (2 s, step 1) then `VM_STOP_GRACE` (10 s, step 2), ADR-0082 § D4. A stop arriving
      **before the guest has beaconed** has no session to write to: step 1 is skipped and
      the allocation still lands `Terminated / Stopped{by: Operator}`, never a crash.
- [ ] A restarted VM boots from an unmodified copy; the operator's artifact is
      byte-identical before and after.
- [ ] No leaked hypervisor processes or rootfs copies after terminal states.
- [ ] **US-VM-7 / item 2** — `/proc/<vmm-pid>/status` reports a non-zero real *and*
      effective `Uid:`/`Gid:` for the running hypervisor.
- [ ] **US-VM-7 / item 3** — `/proc/<vmm-pid>/limits` reports a finite `Max file size` and
      `Max open files`, **both strictly lower than the same fields on the `overdrive serve`
      process** (`/proc/<serve-pid>/limits`; `/proc/self/limits` only where the harness *is*
      that process — under a Tier-3 harness `self` is the test process and is the wrong
      anchor). The strictly-lower comparison is the binding half; it cannot be satisfied by
      inheriting the host default.
- [ ] **US-VM-7 / item 1 (a), production path** — the allocation boots (proving the
      hypervisor reached its declared artifacts *under* the ruleset), **and** a rootfs
      declared outside the default artifact directory also boots, proving the ruleset is
      derived from the spec rather than hardcoded. A hardcoded ruleset fails this case.
- [ ] **US-VM-7 / item 1 (b), denial evidence** — inherited from **Slice 00 P5**, where the
      probe controls the process and attempts an open outside the identical ruleset. *A live
      `cloud-hypervisor` exposes no command to open an arbitrary path, and a sibling test
      process is not covered by the VMM's ruleset — so no production-path executor exists.
      Runtime proof that Landlock is active on a **running** VM is **#258's runtime-EDD
      item** and is not claimed here.* **Argv presence is never acceptable evidence.**
- [ ] **US-VM-7 fail-closed** — a host that cannot supply the confinement lands `Failed`
      with a distinct named reason — **a fifth `TransitionReason` variant minted in the
      Slice 02 shape**, not a reuse of one of that slice's four (K3 targets "≥ 4 distinct");
      **no path starts the hypervisor degraded.** The
      unavailable-confinement condition is **injected at the `Vmm` port boundary** (system
      constraint 1 permits a `Sim*` adapter at a port; the whole test envelope runs on one
      Lima kernel, so no genuinely Landlock-less host exists in it). Mutation target:
      turning the fail-closed arm into warn-and-continue **must be killed**.
- [ ] **US-VM-7 claim discipline** — no artifact or string asserts isolation beyond `[D7]`;
      the forbidden sentence *"isolation identical to Firecracker"* appears nowhere.

## Dependencies

- **Slice 01** (a VM must run before it can stop, crash, or be observed confined).
- **Slice 02** for US-VM-7's fail-closed reason — it extends that slice's failure vocabulary
  with a fifth variant in the same shape, rather than minting a parallel vocabulary.
- **Slice 00 P5** must have proven the confinement flags compose with a real boot. If P5
  came back DOESN'T-WORK, US-VM-7's scope and `[D7]`'s claim are restated **before** this
  slice is built — the claim follows the code, never the reverse.
- SHIPPED and reused unchanged: `exit_observer`, `WorkloadLifecycle` restart/backoff,
  `TerminalCondition` vocabulary.
- **Open DESIGN input:** which uid/gid the hypervisor drops to, constrained to be
  resolvable **without appliance-image changes inside this feature**. If it is not, DESIGN
  returns a blocker rather than expanding into ADR-0068 territory.
