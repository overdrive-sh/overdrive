//! Family `cgroup-connect-hook` (DDD "Connect-hook family"): the per-connect cost of
//! the gateway connect program, service-only against service-plus-gateway.
// SCAFFOLD: true

use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_testing_macros::overdrive_bench;

#[overdrive_bench(
    family = "cgroup-connect-hook",
    substrate = native,
    arms = ["service-only", "service+gateway"],
    replicates = 30,
    bpf_stats,
    metric(name = "hook_ns_per_connect", unit = ns, table = "hook-time"),
    metric(name = "wall_ns_per_connect", unit = ns, table = "wall-time"),
    reference(
        metric = "hook_ns_per_connect",
        value = 117.0,
        label = "spike evidence, different run",
        source = "docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md"
    ),
    smoke(guest = "cgroup-default"),
)]
#[allow(
    dead_code,
    clippy::needless_pass_by_ref_mut,
    reason = "DISTILL scaffold: the attribute is a passthrough until DELIVER"
)]
fn connect_hook_cost(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let _ = rep;
    Err(ReplicateFailure::fixture("RED scaffold: connect-hook fixture is not built"))
}
