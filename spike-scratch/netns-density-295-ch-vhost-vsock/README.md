# netns-density-295 — Cloud Hypervisor kernel vhost-vsock backend spike (V-1(b))

Throwaway spike. Findings: `docs/feature/netns-density-295/spike/ch-vhost-vsock-findings.md`.

- CH changes live in the `vendors/cloud-hypervisor` submodule, branch
  `overdrive/vhost-kernel-vsock` (commits 9b68dbb57, 41a619d19 on top of v53.0).
- `probe/` — standalone static Rust probe (host litmus client/server/recorder and
  guest agent). libc only; not a workspace member.
- `guest-init.sh` — guest PID 1 (stock 7.0.0-29 kernel + stock vsock modules).
- `increment-*/run.py` — native-metal run script (root, under the canonical lease);
  `increment-d/patch_from_c.py` records how increment-d's run.py was derived.
- `launch.py <increment> [probes...]` — acquires the lease via `cargo xtask metal run`,
  streams the log (address redacted), retrieves `evidence/`. Increments are immutable.
- `rsync-owned-filter.sh` — keeps the metal sync away from root-owned evidence dirs.
- `lima-ch.sh` — compile/lint/unit-test surface for the CH fork in Lima (NOT validation).
- `linux-reference/drivers-vhost-vsock.c` — upstream v7.0 source cited for the TX src_cid check.

CH runs on the physical metal host (no nesting). Only stock kernel modules are loaded,
and the runs unload them again.
