# STEP 14-01 fifth-control-plane diagnostic

> Historical evidence launcher. The canonical runner's
> `OVERDRIVE_FWD_GUEST_EXACT` support now provides this path directly without
> source generation. These sources and raw receipts remain only to preserve
> how the earlier evidence was produced; new focused runs use the canonical
> runner.

This one-shot launcher exists because the production test-support runner keeps
`run_nested_vm_test_binary` private and its public `cgroup-default` selection runs
the retained 111 bodies and CLI identity before the five control-plane
identities. Its build script copies the current runner source from its first
item, dropping only file-level docs and crate attributes, then appends a
private wrapper that invokes the existing helper with only the fifth entry of
its existing `STEP14_01_CONTROL_PLANE` manifest.

The included runner's private `workspace_root()` normally derives the root
from its own crate's `CARGO_MANIFEST_DIR`. In this isolated crate that macro
would name `increment-a` and make the unchanged BPF staging helper look under
`spike-scratch/target`. The generator requires exactly one byte-identical
canonical helper and replaces only that helper with the canonicalized
grandparent of this manifest, passed through a compile-time environment value.
That value is the same repository root the original runner derives; the BPF
artifact and every other staged path therefore remain identical.

The resulting execution retains the ordinary STEP 14-01 outer GuestKit image,
nested module payload, eight-vCPU outer VM, cgroup v2, bpffs, securityfs, XFS
reflink volume, device permissions, no-uplink boundary, `CGROUP_NOFILE_EXEC`,
exact libtest filter, 600-second cell bound, required one-pass summary,
runtime-SKIP rejection and cleanup. The acceptance body, production
composition, phase diagnostics and canonical runner are unchanged.

The crate has a one-way path dependency on `overdrive-testing`. It is not a
workspace member, public selector, CI lane, product hook or replacement test.

Source gate and the single authorized runtime command are run on Metal from
the repository root:

```text
cargo xtask metal run --no-sudo -- env CARGO_TARGET_DIR=/home/ubuntu/overdrive/target-codex-14-01 bash -lc 'set -eu; cargo clippy --locked --manifest-path spike-scratch/netns-density-295-cp-14-01/increment-a/Cargo.toml -- -D warnings; cargo build --locked --manifest-path spike-scratch/netns-density-295-cp-14-01/increment-a/Cargo.toml; install -D -m 0755 /home/ubuntu/overdrive/target-codex-14-01/debug/netns-density-295-cp-14-01-diagnostic /home/ubuntu/.cache/overdrive-step14-01/1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff/diagnostic; test "$(sha256sum /home/ubuntu/.cache/overdrive-step14-01/1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff/diagnostic | cut -d" " -f1)" = 1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff'
cargo xtask metal run -- bash -c 'set -eu; cargo xtask guest-kit-prepare; diagnostic=/home/ubuntu/.cache/overdrive-step14-01/1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff/diagnostic; test "$(sha256sum "$diagnostic" | cut -d" " -f1)" = 1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff; exec "$diagnostic"'
```

The diagnostic executable is built once in the unprivileged source gate and
installed under the Metal user's cache outside the rsync-managed workspace,
in a directory named by its SHA-256. The privileged runtime command requires
the exact digest before invoking it. This keeps root-owned Cargo output out of
the standalone source directory and prevents a later workspace sync from
deleting the reviewed executable before use.

The isolated execution is diagnostic evidence only. It cannot contribute to
the unchanged full-runner total of 117 bodies.
