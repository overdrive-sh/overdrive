# netns-density-295 — V-22 spike (dead VM's host-side vsock connections vs CID reuse)

Throwaway. Findings: `docs/feature/netns-density-295/spike/findings-v22.md`.

- `probe/` — static Rust guest agent (control port 4000: `hold`, `close`, `tick`). libc only, not a workspace member.
- `guest-init.sh` — guest PID 1 (stock 7.0.0-29 kernel + stock vsock modules).
- `increment-*/run.py` — native-metal run script (root, under the canonical lease); host side in Python
  (AF_VSOCK listeners on 1234/1240/1241/1243, CID_ANY). Increments are immutable evidence:
  a failed (initramfs `/init` not executable), b (smoke 1x10, first sight of the finding), c (smoke, fixed
  harness), d (cross-round local-port reuse bug), e (guest local port 4000 clashed with the agent control
  port), **f (the verdict run: 4 rounds x 250 per port)**.
- `launch.py <increment> [args]` — acquires the lease via `cargo xtask metal run`, retrieves `evidence/`.
- Reuses the CH fork build cache of `netns-density-295-ch-vhost-vsock` (read-only copy) — fork commit
  41a619d19 on `overdrive/vhost-kernel-vsock`.
- Guest CIDs used: 3220 (VM under test, killed and relaunched), 3221 (bystander), 3229 (anchor). Every VM,
  lock and directory carries the `v22`/`netns-density-295-v22` prefix.
