# overdrive-control-plane conventions

## Cgroup boot ordering — parent before child

The cgroup v2 kernel contract: writing a controller name to
`cgroup.subtree_control` requires that controller to be enabled in the
parent's `subtree_control` first. If it's not, the kernel returns
ENOENT. The boot sequence in `run_server` must maintain this ordering:

1. `cgroup_preflight::run_preflight()` — validate delegation
2. `create_and_enrol_control_plane_slice()` — delegates `+cpu +memory
   +io +pids` to `overdrive.slice/cgroup.subtree_control` (parent)
3. `create_workloads_slice_with_controllers()` — delegates the same
   controllers to `workloads.slice/cgroup.subtree_control` (child)

Moving step 3 before step 2 produces ENOENT on the child's
`subtree_control` write because the kernel does not see the controllers
at the parent level. This ordering is load-bearing and must not be
rearranged.
