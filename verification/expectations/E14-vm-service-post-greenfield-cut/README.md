# E14 — post-cut healthy VM Service journey

Status: `pending` — capture complete; an independent evidence reviewer must
evaluate this receipt before any `satisfied` status is recorded.
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: yes; this is the post-cut receipt for the checked-in VM
Service journey.

## Expectation

The checked-in `examples/service-kind-vm-workloads/` healthy journey runs
through the built default-feature `overdrive` binary on native x86_64 metal.
The VM Service deploy is accepted and reaches Stable with guest TCP and HTTP
probe results. Its checked-in VM Job client reaches `Succeeded` after receiving
the byte-exact `SVM-E08-GUEST-OK` reply through the Service frontend. The
example then stops the owned workloads and removes its private materialization,
leaving zero owned VMM, probe-task, network, cgroup, run-directory, mount,
loop-device, and preparation-tree deltas.

## Anchors

- Anchor: S-SVM-01 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-1 and US-SVM-2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: post-cut black-box event in the `DISTILL / Post-cut black-box event` section of `docs/feature/remove-legacy-exec-workload-driver/feature-delta.md`.
- Anchor: DELIVER roadmap acceptance criteria for step 03-01 in `docs/feature/remove-legacy-exec-workload-driver/deliver/roadmap.json`.

## Exact public boundary

The capture used the repository metal wrapper and drove only the checked-in
journey:

```text
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
cargo xtask metal run -- examples/service-kind-vm-workloads/run-example.sh run healthy
```

The wrapper resolved `OVERDRIVE_METAL_TARGET` from the workspace `.env` as
`ubuntu@151.115.99.251`. The checked-in journey built
`overdrive-cli`'s default-feature `overdrive` binary, prepared the selected
guest kernel/rootfs, ran `serve` plus the public `deploy` and `workload
describe` commands, deployed the checked-in VM client, and performed its
owned stop and cleanup. No Rust test command, inline workload spec, or
`overdrive-*` crate import was used.

## Captured evidence

The successful second attempt is retained in `evidence/product-run.out` and
`evidence/attempt-2-product-run.out`. It records the built-product command,
Service `Accepted` → `Stable` transition, guest TCP/HTTP Pass observations,
the peer VM Job's `Verdict: Succeeded`, the exact guest reply oracle, and
`E08 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0
mount=0 loop=0 preparation=0` from the checked-in runner.

The first bounded attempt is retained in
`evidence/attempt-1-product-run.out` because evidence history is append-only.
It reached the same stakeholder-visible Service and peer-Job success and
reported zero cleanup deltas, but its runner teardown observation included a
signal-9 failed allocation and returned exit 1. The second attempt completed
with exit 0; the first attempt is not silently discarded or presented as the
successful receipt.

`evidence/product-run.meta` records both attempts, the configured target and
guest artifacts, and the capture timestamps. `evidence/verification.yaml`
pins the successful execution to the exact post-cut SHA and records the dirty
working tree. `E06` and `E08` remain historical receipts and are not cited as
post-cut proof here.

The capture author intentionally leaves this expectation `pending`; a
different reviewer must audit the evidence and decide whether the claim is
satisfied.
