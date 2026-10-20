//! S-ND295-65 / NO-OVERRIDE-SEAMS (ADR-0219) — `run_server`'s driven adapters
//! are one bundle with every field required: no `Default`, no builder, so a
//! composition root cannot leave an adapter out and let the boot construct
//! or skip it. One invariant per fixture: omitting the guest-flow seam (the
//! adapter the owner's listeners and forwarder probe run over) does not
//! compile. The sibling `.stderr` is the assertion.

use std::sync::Arc;

use overdrive_control_plane::{BootAdapters, NodeAdapters, ProbeAdapters, VmAdapters};
use overdrive_core::traits::cgroup_fs::CgroupFs;

fn without_the_guest_flow_seam(
    boot: BootAdapters,
    cgroup_fs: Arc<dyn CgroupFs>,
    probers: ProbeAdapters,
    vm: VmAdapters,
) -> NodeAdapters {
    NodeAdapters { boot, cgroup_fs, probers, vm }
}

fn main() {
    let _ = without_the_guest_flow_seam;
}
