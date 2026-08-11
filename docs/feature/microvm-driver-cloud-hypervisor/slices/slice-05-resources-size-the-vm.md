# Slice 05 — `[resources]` sizes the VM's vCPUs and memory

> DISCUSS brief (2026-08-01; **renumbered 04 → 05 on 2026-08-02** when the recovered intake
> decision `I-6` put volumes in the `04` slot — see
> `slice-04-vm-writes-output-the-operator-can-read.md`).
> Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Story: **US-VM-5**. Job: **J-OPS-003**. KPI: **K10** (declared-size fidelity, minted
> 2026-08-02 — the story had no KPI until peer review caught it). Gated by Slice 01;
> ordered after Slice 04.

## Goal (one line)

`cpu_milli` and `memory_bytes` from the existing driver-agnostic `[resources]` block
determine the guest's vCPU count and memory — observable **from inside the guest**.

## Learning hypothesis

Reusing `[resources]` rather than adding CPU/memory keys to `[vm]` keeps **one** declared
size per workload, which is the precondition for the GH
[#92](https://github.com/overdrive-sh/overdrive/issues/92) right-sizing reconciler: a
reconciler cannot converge desired-vs-actual against two sources of desired.

**Predicted:** the guest reports exactly the declared shape, and `Driver::resize` has a
single unambiguous target to move.

## Thinnest `serve` + `deploy` loop

`overdrive serve` + three deploys — `cpu_milli = 2000`, `cpu_milli = 250`,
`memory_bytes = 2147483648` — each with a guest command that reports the CPU count and
memory it actually sees.

**Each is parametrized over both memory backings** (`[D8b]`): a VM with no volume (private
memory) and a VM with a volume (`--memory shared=on`). One parametrized case, not a second
test.

## Behavior (DESIGN owns the API)

- vCPU count derived from `resources.cpu_milli` with a documented rounding rule and a
  **floor of 1** (a VM cannot have a fractional CPU; floor rather than refuse).
- Memory size derived from `resources.memory_bytes`.
  **C-3 correction (`superseded-by-DESIGN`, 2026-08-11; SD-4 in brief
  § *System Architecture*, ADR-0082 § D2).** The **guest's RAM** is
  `resources.memory_bytes` — the operator's declared figure is what the workload observes —
  but the allocation's **`memory.max` is `resources.memory_bytes + reserve(...)`**, never
  the declared figure alone. The cgroup charges the hypervisor's entire RSS **plus** the
  host page tables backing the guest mapping, which RSS structurally cannot see, so setting
  both from one number is a cgroup OOM **by construction** — surfacing as
  `WorkloadCrashedImmediately { signal: 9 }`, indistinguishable from `kill -9`, with no
  mention of memory. `MemoryPlan::derive` is the only constructor and
  `guest_bytes == cgroup_max_bytes` has no representation.
  **This bites from Slice 01, not from here** — Slice 01 already writes resource limits and
  must give the guest *some* size — so the derivation is encoded there and this slice
  extends it to the operator-declared figure. `reserve` is **measured in DELIVER** via
  `memory.current` / `memory.stat`, not RSS; a guessed constant between the two known
  RSS-derived floors (~5.4 MiB steady-state, ~11.9 MiB pre-residency, both P13/restore-path)
  is a rejection.
- **`[vm]` carries no CPU and no memory field** — deliberately. Two sources of truth here
  would break #92 before it is written.
- `workload describe` reports declared resources for a VM allocation as it does for a
  process allocation.
- `Driver::resize` is a required trait method. Whether this slice performs live CPU/memory
  hotplug or **rejects resize honestly** is DESIGN's call — but it **must not silently
  no-op**.
- **`[D8e]` interaction, decided here rather than left implicit.** A volume-carrying VM's
  `virtiofsd` sits in the allocation's cgroup scope, so its memory counts against
  `resources.memory_bytes`. DESIGN decides whether guest memory is derived from the declared
  figure as-is or net of a daemon allowance — but it must be **decided**, or a
  volume-carrying VM silently gets less guest RAM than an identical volume-free one.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — `serve` + `deploy`, with the assertion
  taken **from inside the guest**, not from the hypervisor config the platform generated.
  Asserting on generated config would prove only that we wrote what we wrote.
- **Thinnest?** Yes — a derivation rule and two config fields. No new subsystem.
- **Delivers operator-visible value alone?** Yes — a VM that honours its declared size,
  with one sizing model across workload classes.
- **Why last?** It unblocks the commercial pillar (#92 / whitepaper §14) but is worthless
  until Slices 01–03 make VMs trustworthy. Right-sizing an untrustworthy workload is
  optimising a lie. **It is still last for that original reason — volumes did not displace
  it.** Running after Slice 04 additionally makes its sizing case *stronger*, because it can
  then be parametrized over both memory backings.

## Acceptance (= US-VM-5 ACs)

- [ ] vCPU count derived from `cpu_milli` with a documented rounding rule and floor of 1;
      `[vm]` has no CPU field.
- [ ] Memory size derived from `memory_bytes`; `[vm]` has no memory field. **And the
      allocation's `memory.max` is strictly greater than the guest's RAM by the reserve
      (C-3)** — a VM booted at its declared `memory_bytes` reaches residency without being
      cgroup-OOM-killed.
- [ ] Both observable **from inside the guest** in a Tier-3 case.
- [ ] `workload describe` reports declared resources for VM allocations as for process
      allocations.
- [ ] `Driver::resize` either hotplugs or rejects with a named reason — never a silent
      no-op.
- [ ] **Sizing holds on both memory backings** — the Tier-3 case is parametrized over a
      volume-free VM (private memory) and a volume-carrying one (`--memory shared=on`). This
      closes the `shared=on` × sizing interaction **inside** this feature rather than leaving
      GH #92 to discover it.

## Dependencies

- **Slice 01** (a VM must boot before it can be sized).
- **Slice 04** — for the both-shapes sizing case only. **Not a hard dependency:** 04 and 05
  are mutually independent and can be resequenced; the only thing lost by swapping them is
  that parametrization, which would then fall to #92.
- **Slice 00 P6** supplies the measured `shared=on` cost and whether it changes the guest's
  observed memory size.
- Unblocks GH #92 (right-sizing reconciler / CPU hotplug). **State the case as "CPU hotplug
  unblocks #92" — never as "CH has hotplug", which is refutable: Firecracker shipped
  virtio-mem memory hotplug in v1.14.0 (2024-12-17). CPU hotplug is the surviving
  differentiator, and it is sufficient.**
- Accepted, quantified cost: the ~75 ms boot penalty attributed to CH's hotplug/vhost-user
  feature surface. Long-lived workloads (#92, #96–#100) collect on it; the reference
  implementation ran short-lived jobs and paid it for nothing.
