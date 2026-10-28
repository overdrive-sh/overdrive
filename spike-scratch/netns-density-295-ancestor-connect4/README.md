# A-19 / V-15 ancestor cgroup `connect4` probe

This standalone Aya Rust probe validates the native premise used by D24: an
ADR-0053 style `cgroup/connect4` program attached at
`/sys/fs/cgroup/overdrive.slice` is effective for sockets created by a process
in the descendant `/sys/fs/cgroup/overdrive.slice/control-plane.slice`.

The probe runs inside one disposable Cloud Hypervisor guest on the canonical
Metal runner. The guest creates the production-shaped cgroup hierarchy,
attaches the retained V-11 `vip_connect4` Aya program at the ancestor, inserts
one TCP and one UDP VIP map entry, then runs the client sockets from the child.
It records `getpeername()` for both rewritten flows. It also runs an unmapped
VIP control and moves the process to a sibling cgroup outside the ancestor;
the mapped VIP is unreachable in both controls. No host-side owner, host BPF
attachment, physical host cgroup, or management network state is changed.

The source is standalone and is never wired into a workspace crate. `bpf/`
and `vfwd/` retain the working V-11 Aya Rust source shape; the only new probe
surface is the private `ancestor-vip` litmus command. Runtime output and
source/artifact hashes are retained under the successful increment.

Run the immutable increments through the launcher:

```text
python3 spike-scratch/netns-density-295-ancestor-connect4/launch.py increment-f
```

The launcher routes the run through `cargo xtask metal run --`, captures the
canonical run id and retrieves the raw evidence. Build output remains ignored
under `out/` and `target/`.
