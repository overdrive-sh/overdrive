use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_testing_macros::overdrive_bench;

#[overdrive_bench(
    family = "f",
    substrate = guest,
    arms = ["forwarding"],
    replicates = 30,
    metric(name = "m", unit = ns, table = "t", series),
    clock(guest = guest_monotonic),
    smoke(host_pure),
)]
fn b(_rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    Ok(())
}

fn main() {}
