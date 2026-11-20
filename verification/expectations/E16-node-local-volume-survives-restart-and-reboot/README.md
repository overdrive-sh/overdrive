# E16 — a VM workload's node-local volume keeps its data through a node reboot, and `describe` shows the volume and a typed reason when the pool is too small

**Surface:** E (end-to-end) and O (operator CLI) · **KPI:** — · **Status:** `pending` (claim authored; no evidence captured)

- Anchor: VD-2 — the volume belongs to the workload and outlives stop and restart (`docs/product/architecture/adr-0311-a-volume-belongs-to-its-workload-and-outlives-stop-and-restart.md`; `docs/feature/node-local-persistent-volume/feature-delta.md` § Slice and decision index)
- Anchor: VD-8 — a replacement waits for the volume claim to clear (30 s), not for process exit (`docs/product/architecture/adr-0317-a-replacement-waits-for-the-volume-claim-not-for-the-vmm-process.md`)
- Anchor: VD-9 — the launch path creates or activates the volume; there is no volume reconciler (`docs/product/architecture/adr-0318-the-launch-path-creates-and-activates-the-volume-no-volume-reconciler-in-this-slice.md`)

This is a point-in-time feature-verification record (`.claude/rules/verification.md`), not a test. It does
**not** duplicate the recurring test S-VOL-T3-07
(`crates/overdrive-cli/tests/integration/volume_survives.rs`), which re-proves restart and stop/resume inside a test
guest on every run; reactivation of an inactive volume is covered by S-VOL-T3-05 and the in-process tests. What no test can do is reboot the node the product runs on.
This expectation records, once, what an operator sees across a real node reboot.

## The claim

An operator deploys a VM workload that declares `[vm.volume] size = "<n>GiB"` with `overdrive deploy`, and the guest writes
records to its second disk (serial `ovd-volume`). Restart and stop/resume are not re-claimed here: S-VOL-T3-07 proves them on
every run. This record claims only what a test cannot show:

1. A real node reboot: after the node boots, the volume is present but inactive; the next launch activates it (no operator
   action, no manual `lvchange`), and the guest reads the records written before the reboot.
2. What the operator sees on `overdrive workload describe`: `Volume  <n>GiB (guest disk: virtio serial ovd-volume)` before and
   after the reboot; and, for a second workload declaring a volume larger than the pool's free space, a typed reason that
   names the volume, the cause (`InsufficientSpace`, with the requested and free figures) and the remedy, rather than a
   generic failure.

## Substrate

`native-metal` (see `execution-substrate`): a KVM-capable x86_64 node provisioned with the volume pool (the partition
labelled `overdrive-volumes`, the physical volume and the `overdrive` volume group) and a built default-feature `overdrive`
binary. Lima, a virtualized host and a nested guest are not substitutes: the claim is about a node that really reboots.

**The node reboot needs the owner's authorisation.** Rebooting the shared metal host is physical maintenance, the same class
as module loading and provisioning (`.claude/rules/testing.md` § "Running tests — native x86_64 bare-metal KVM host"). A
capture must not reboot a host that other worktrees' runs share without its owner's explicit approval, and a reboot kills the
capture's own session, so the runner records the pre-reboot state, the reboot is triggered under that approval, and a second
invocation after the node is back records the post-reboot state against the same pinned SHA. Sub-claim 2's typed-reason half does not
need a reboot and can be captured before sub-claim 1 is authorised; an unauthorised or failed reboot leaves sub-claim 1
`pending`, never `satisfied`.

## Edge cases the capture must include

- One workload is running and one is stopped when the node reboots: both volumes are inactive afterwards, and a later launch
  or resume activates each.
- The volume is larger than the guest's first write: the data written is a few records, not the whole device, so the claim
  does not rest on a zero-filled device reading back as zero.
- A second workload with its own volume is deployed beside the first: after every step each workload reads only its own
  records (no volume is shared or swapped).
- A workload declared **without** `[vm.volume]` is deployed beside them and behaves as before: no second disk, no volume
  line in `describe`.

## Non-effects (what must not happen)

- No operator action between `deploy` and the data being readable: no `lvchange`, no `mkfs` on the host, no re-deploy.
- The volume is never recreated or blanked: the logical volume count in the `overdrive` group does not grow across the
  reboot, and the first record is byte-identical after each step.
- Neither the reboot nor the oversized second workload deletes or erases an existing volume (nothing in this slice does;
  deleting a workload is a later slice).
- The data is not read back from guest-reported state alone: the capture also reads the volume's device node from the host
  with direct I/O while no VMM holds it.

## What would refute it

Any one of these observed in the captured output refutes the claim:

- the post-reboot allocation reads no data, or a different first record, than was written;
- a post-reboot launch reaches `Failed` with a volume cause, or never reaches `Running`, when the node has free pool space;
- the post-reboot launch needs an operator step to activate the volume;
- `describe` omits the volume, shows another size or serial than declared, or shows a volume line for the workload declared
  without one;
- the oversized second workload's `describe` shows no typed volume reason (a generic failure, an empty reason, or a reason
  that omits the cause or the figures);
- the number of logical volumes for the workload is not exactly one at every step.

## Capture steps

1. Deploy the workload (and the workload declared without `[vm.volume]`), write records, record `lvs` and `describe`.
2. Deploy the oversized second workload; record its `describe` (the typed reason) and `lvs`.
3. Under the owner's authorisation, reboot; on return record `lvs`, `describe`, the guest's read-back, and the host's
   direct-I/O read of the device node while no VMM holds it.

## Capture plan (DELIVER or DEVOPS, not authored here)

Evidence is captured after the feature is delivered, against the built `overdrive` binary, by someone other than the
implementation author, then reviewed by a different reviewer (`.claude/rules/verification.md` § "The different fox audit").
A native-metal capture runs `cargo xtask metal run --exclusive -- env KEY=VALUE ... verification/harness/run-expectation.sh <ID>`
with the reboot authorisation above. This file carries no `verification.yaml`, no `evidence/` and no `satisfied` status.
