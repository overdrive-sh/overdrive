# STEP 14-01 control-plane-prefix diagnostic

> Historical evidence launcher. The canonical runner's
> `OVERDRIVE_FWD_GUEST_EXACT` support now provides direct one-body filtering
> without source generation. These sources and raw receipts remain only to
> preserve how the earlier prefix evidence was produced; new focused runs use
> the canonical runner.

This one-shot launcher reuses the current canonical runner's private
`run_nested_vm_test_binary` helper with exactly
`STEP14_01_CONTROL_PLANE[..3]`. It retains the first two prerequisite
control-plane identities and the third identity in canonical order while
avoiding the unrelated retained-111 and CLI prefix.

The build script copies the current runner from its first Rust item, dropping
only file-level docs/attributes, and appends the private three-cell entrypoint.
It neutralizes the standalone `CARGO_MANIFEST_DIR` only for the canonical
workspace-root helper, exactly as the independently reviewed increment-a
launcher does.

The execution uses the same outer GuestKit image, nested module payload,
eight-vCPU outer VM, cgroup v2, bpffs, securityfs, XFS reflink volume, device
permissions, no-uplink boundary, `CGROUP_NOFILE_EXEC`, exact filters,
600-second per-cell bound, required summaries, runtime-SKIP rejection and
cleanup. Bodies, assertions, ordering and production composition are
unchanged. The diagnostic cannot count toward the canonical 117 result.

The unprivileged source gate builds and installs the reviewed executable
outside the rsync-managed workspace under its exact digest. The privileged
runtime prepares the ordinary GuestKit payload, requires that digest, then
executes it directly:

```text
cargo xtask metal run --no-sudo -- env CARGO_TARGET_DIR=/home/ubuntu/overdrive/target-codex-14-01 bash -lc 'set -eu; cargo clippy --locked --manifest-path spike-scratch/netns-density-295-cp-14-01/increment-b/Cargo.toml -- -D warnings; cargo build --locked --manifest-path spike-scratch/netns-density-295-cp-14-01/increment-b/Cargo.toml; binary=/home/ubuntu/overdrive/target-codex-14-01/debug/netns-density-295-cp-14-01-prefix-diagnostic; digest=$(sha256sum "$binary" | cut -d" " -f1); staged=/home/ubuntu/.cache/overdrive-step14-01-prefix/$digest/diagnostic; install -D -m 0755 "$binary" "$staged"; test "$(sha256sum "$staged" | cut -d" " -f1)" = "$digest"'
cargo xtask metal run -- bash -c 'set -eu; cargo xtask guest-kit-prepare; diagnostic=/home/ubuntu/.cache/overdrive-step14-01-prefix/3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6/diagnostic; test "$(sha256sum "$diagnostic" | cut -d" " -f1)" = 3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6; exec "$diagnostic"'
```
