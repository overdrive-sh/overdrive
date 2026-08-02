# Slice 00 — SPIKE: does the pinned kernel boot under Cloud Hypervisor, and does vsock carry the beacon?

> DISCUSS brief (2026-08-01; amended 2026-08-02 — **P6 added, re-sized 2–3 d → 3–4 d**).
> Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> **Blocking spike — gates Slice 01.** Not a user story. Governed by
> `.claude/rules/spike.md`.

## Why this slice exists

Intake **precedent warning #2**: the reference implementation rated its equivalent spike
`HIGH (blocker)` / *"Blocks all CH work"*, never ran it, and wrote 43 KB of
implementation on top of the unvalidated assumption. This slice is that spike, run.

Everything in Slices 01–05 rests on **five** unproven premises (P1, P2, P3, P5, P6) plus
two unmeasured numbers (P4, and P6's memory cost). They are cheap to test and expensive to
discover late — P2's netns half, P5, and P6 in particular can invalidate a
`VmDriver::start` path that is already built.

## Hypotheses, predictions, falsifications

Per `.claude/rules/debugging.md` § 4 — each probe carries the triple.

### P1 — Kernel boots under CH from an ext4 `virtio-blk` rootfs

- **Hypothesis:** the pinned 6.18 appliance kernel (ADR-0068) boots under
  `cloud-hypervisor` with `--disk path=<ext4>` and `root=/dev/vda`.
- **Predicted:** the guest reaches userspace and PID 1 runs; console shows the init's
  output.
- **Falsification:** the guest panics on mount, or CH refuses the disk.

### P2 — vsock carries a beacon and an exit status, **including from inside a netns**

- **Hypothesis:** a static guest binary can write a ready beacon and an exit status over
  `virtio-vsock` to a host-side listener, before and independent of guest networking —
  **and the channel still works when the `cloud-hypervisor` process has been placed in a
  per-workload network namespace while the host listener stays in the host netns.**
- **Predicted:** the host reads both messages, in order, with the exit status matching a
  deliberately non-trivial guest exit code (**7**), in **both** placements (VMM in the host
  netns, and VMM `setns`-ed into a fresh netns).
- **Falsification:** the channel is unavailable pre-network, messages are lost, the
  ordering is not observable, **or the netns-placed run loses the beacon.**

> **Why the netns half is here.** `[D7]` item 5 places the hypervisor in the per-workload
> netns the shim already provisions for every alloc, and `[D2]`'s entire Running gate rides
> on the beacon. AF_VSOCK's namespace behaviour is kernel-version-dependent — discovering a
> conflict in Slice 01 means unpicking a spawn path that is already built. **If the
> netns-placed run loses the beacon, that is a PIVOT, and it is cheap here.**

### P3 — The two kernels agree

- **Hypothesis:** P1 and P2 hold on **both** the Lima dev kernel and the pinned 6.18
  appliance kernel.
- **Predicted:** identical verdicts on both.
- **Falsification:** a divergence — which is itself the most valuable possible result,
  because it would land in DELIVER as an untraceable Tier-3 flake.

### P4 (measurement, not pass/fail) — per-launch rootfs copy cost

Research **Gap 2** flags host reflink (`FICLONE`) vs overlay as unmeasured, and `[D5]`
depends on it. Measure `cp --reflink=auto` of the rootfs on the appliance filesystem;
record whether reflink is taken or a full copy falls back, and the wall-clock. **Record
the number; do not design around an assumption.**

### P5 — the `[D7]` confinement flags compose with a real boot

- **Hypothesis:** `cloud-hypervisor` boots the same VM as P1 while simultaneously
  (a) running under `--landlock` with a ruleset covering only that VM's kernel, rootfs copy
  and API socket, (b) running as a **non-root** uid/gid, (c) under reduced `RLIMIT_FSIZE`
  and `RLIMIT_NOFILE`, and (d) with seccomp left at its default (never `false`/`log`).
- **Predicted:** the guest reaches userspace exactly as in P1; the live VMM's
  `/proc/<pid>/status` shows a non-zero `Uid:`/`Gid:` and a non-zero `Seccomp:` mode,
  `/proc/<pid>/limits` shows the reduced ceilings, and an open of a sentinel path **outside**
  the ruleset is **denied**.
- **Falsification:** any of — the uid-dropped process cannot open `/dev/kvm`; the Landlock
  ruleset omits a path CH needs and the boot fails opaquely; a reduced rlimit breaks the
  boot; `--landlock` is absent from the installed CH build.
- **Also record:** the installed `cloud-hypervisor --version` and whether `--landlock` is
  present — the evidence for the **CH version floor** decision under feature-delta constraint
  7 (*"name what breaks below it, or declare none"*) — **and the exact uid / gid / group the
  probe used**, plus how it obtained `/dev/kvm` access. *"Which uid"* is an open DESIGN
  question (feature-delta Handoff item 8); the probe may select **any** workable unprivileged
  identity, but DESIGN must inherit evidence rather than assume the result generalises, so
  findings.md names the identity the verdict was obtained with.
- **This probe is also the source of US-VM-7's denial evidence.** The production path has no
  executor for "the hypervisor is denied a path outside its ruleset" — CH exposes no command
  to open an arbitrary path, and a sibling test process is not covered by the VMM's ruleset.
  Here the probe controls the process, so it can apply the identical ruleset and attempt the
  open. **Capture that denial explicitly** (pasted output): it is cited by US-VM-7 AC item
  1(b) and cannot be reconstructed later.

> **Why here and not in Slice 03.** These are the two integration risks that can invalidate
> a spawn path *after* it is built (a uid-dropped VMM that cannot reach `/dev/kvm`; a
> ruleset missing a path CH needs). Both are cheap to answer against a probe and expensive
> to answer against Slice 01's production `VmDriver::start`. **Probe only — no production
> code, no `crates/` file.**

### P6 — `virtiofsd` + `--memory shared=on` compose with the boot AND with the confinement

Added 2026-08-02 with intake **`I-6`** / `[D8]`, which put shared writable storage
(virtiofs for **volumes**; block stays the **rootfs** mechanism) into scope as **Slice 04**.

- **Hypothesis:** the same VM that boots in P1 also boots when (a) a `virtiofsd` is spawned
  first over a vhost-user socket with `--cache=never` and `--sandbox=namespace`, (b) CH is
  launched with `--memory shared=on` and the `fs` device, and (c) the P5 confinement flags
  are applied simultaneously — and a file written by the guest inside the share is visible
  on the host, and vice versa.
- **Predicted:** the guest reaches userspace as in P1; a guest write to the mounted share
  appears byte-identical in the host source directory; a `read_only` share refuses the
  guest's write; and the P5 `/proc` captures are unchanged with the fs device present.
- **Falsification:** any of — `--sandbox=namespace` is unavailable or refused in the probe
  environment; the uid-dropped, Landlock-confined CH cannot open the vhost-user socket;
  `shared=on` conflicts with a confinement flag; the guest cannot mount the share; writes
  do not round-trip.

**Also measure and record (not pass/fail):**

- **What `--memory shared=on` actually costs.** `[D8b]` makes it *conditional* precisely
  because a volume-free VM should not pay for it — but the size of "it" is currently
  unstated. Compare host memory accounting for the same guest with and without `shared=on`
  and **record the number**. Same discipline as P4's reflink measurement: measure it, do
  not design around an assumption.
- **Whether `shared=on` changes the guest's observed memory size**, which is the
  `[D8b]`-flagged interaction with US-VM-5 (Slice 05) and GH #92.
- **Which paths the CH Landlock ruleset needs once the fs device is present** — at minimum
  the vhost-user socket. **Confirm the volume *source* directory is NOT required by CH**
  (`[D8e]`: only `virtiofsd` reaches the data). If CH turns out to need it, that is a
  material finding — it would mean volumes widen `[D7]`'s hypervisor confinement, and
  US-VM-8's non-widening AC must be restated before it is built.
- **The installed `virtiofsd --version`**, and whether `--sandbox=namespace` is available —
  the second half of the version-floor evidence under feature-delta constraint 7.
- **Whether host-side read-only export works** (`[D8g]`). `read_only` is framed as a
  security control, which is only honest if it is enforced host-side; a guest-side `-o ro`
  is guest-cooperative and void against an uncooperative guest. **Probe it by attempting the
  write from the guest against a host-side read-only export.** If host-side enforcement is
  unavailable, `[D8a]`'s security framing is struck before US-VM-8 is built — the claim
  follows the code.
- **Volume I/O cost with `--cache=never`.** This document cites FUSE's measured 0.78× native
  when arguing about the *rootfs* role, then `[D8c]` chooses `never` for the *volume* role
  without applying the same measurement discipline — on the one path that carries the
  workload's actual output. Measure write throughput and per-file latency through the share
  for a representative payload, against a direct host write. **Record the number; it is
  informational, not pass/fail** — but a 120-frame render is the canonical example, so
  DESIGN should not have to guess.
- **Whether the guest can mount the share at all**, and what a *failed* mount looks like from
  inside the guest — the evidence `overdrive-init`'s refuse-to-exec path (`[D4]` amendment,
  `[D8g]`) is built against.

> **Why here and not in Slice 04.** Identical reasoning to P5. A `virtiofsd` that cannot
> compose with a uid-dropped, Landlock-confined hypervisor fails **opaquely**, and finding
> that out against Slice 04's production supervision code means unpicking a lifecycle that
> is already built. The reference implementation's `VirtiofsdManager` was 415 lines; this
> probe costs a fraction of that and de-risks all of it.

## Method

- Probe code in gitignored `spike-scratch/increment-a/` (P1/P2), `increment-b/` (P3/P4),
  `increment-c/` (P5) and `increment-d/` (P6) — **self-contained, own `Cargo.toml`, never
  under `crates/`**, never committed. Preserve earlier increments as evidence; do not
  overwrite.
- Run **for real, as root, inside Lima**: `cargo xtask lima run -- …`. **No `--no-run`,
  no compile-only gate** — a compile proves nothing about whether a kernel boots.
- Record `uname -r` for every verdict; the verdict is pinned to a kernel.
- Paste real command output into findings. Narrated evidence is not evidence.

## Deliverables

- `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md` — binary verdict per
  hypothesis (WORKS / DOESN'T-WORK), predicted-vs-actual with pasted output, the P4
  measurement, **the P5 `/proc` captures and the installed `cloud-hypervisor --version` +
  `--landlock` availability** (the CH version-floor evidence for constraint 7), **the P6
  round-trip evidence, the measured `shared=on` cost, the CH-ruleset path list, and the
  installed `virtiofsd --version` + `--sandbox=namespace` availability**, design
  implications, one-line gate recommendation.
- `docs/feature/microvm-driver-cloud-hypervisor/spike/wave-decisions.md` — the
  PROMOTE / DISCARD / PIVOT decision.

## Gate

| Outcome | Meaning |
|---|---|
| **PROMOTE** | P1–P3, P5 and P6 hold on both kernels → Slices 01, 03 and 04 proceed as designed. |
| **PIVOT** | P1 or P2 fails → `[D5]` (rootfs format) or `[D4]` (agent channel) is revisited **before** Slice 01, not during it. **P2's netns half failing** → `[D7]` item 5's netns placement is revisited before Slice 01 builds on it. **P5 failing** → `[D7]` items 1–3 and the US-VM-7 ACs are revisited, and the residual isolation posture is restated honestly **before** anything claims it. **P6 failing** → `[D8]`'s volume mechanism, `[D8b]`'s `shared=on` conditionality, or `[D8d]`'s `--sandbox=namespace` is revisited **before** Slice 04 is built. |
| **DISCARD** | CH cannot boot the pinned kernel at all → the feature's premise is refuted and returns to the user. |

**Per-probe gating, so a partial result does not block everything.** P1–P3 gate **Slice 01
and therefore the whole feature**. P5 gates **Slice 03's US-VM-7 only**. P6 gates **Slice 04,
and US-VM-5's both-memory-backings sizing case in Slice 05** (`[D8b]` / US-VM-5 AC 5 — P6
also supplies whether `shared=on` changes the guest's observed memory size). A P6
DOESN'T-WORK is *not* a reason to hold Slices 01–03 — it is a reason to re-decide `[D8]`
before Slice 04, which is precisely why it is measured here rather than there.

**A failing probe never silently weakens a claim.** If the P5 confinement flags or the P6
storage mechanism cannot compose with a real boot, the honest move is to **restate the
claim in `feature-delta.md`** and hand the removed items to #258 — never to keep the claim
and drop the code. That inversion is precedent warning #6 verbatim, and `[D8d]` exists
because the reference implementation did exactly it with `--sandbox`: it downgraded
`namespace` → `chroot` and left every downstream document asserting the stronger posture.

**Report DOESN'T-WORK honestly.** A negative result that kills a candidate here is the
spike succeeding. Cross-check any surprising verdict against upstream CH docs before
trusting it.

## Explicitly NOT in this slice

No production code. No `crates/` file created or modified. No `Vmm` trait, no driver, no
spec surface — those are Slice 01, and writing them now is precisely the reference
implementation's mistake. **P5 proves the confinement flags compose; it does not build
them** — the production uid-drop, Landlock ruleset and rlimits are US-VM-7 in Slice 03,
and the cgroup/netns placement is US-VM-1 in Slice 01. **P6 likewise proves the storage
mechanism composes; it does not build it** — the `[[vm.volume]]` surface, the supervised
`virtiofsd` lifecycle, and its exit-honesty classification are US-VM-8 and US-VM-9 in
Slice 04. Specifically **not** in this spike: process supervision, socket-readiness
waiting, signal escalation, `Drop` guards, or any `ExitKind` classification.
