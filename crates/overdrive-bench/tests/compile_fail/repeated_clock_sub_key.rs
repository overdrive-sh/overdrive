use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_testing_macros::overdrive_bench;

#[overdrive_bench(
    family = "f",
    substrate = native,
    arms = ["a", "b"],
    replicates = 9,
    metric(name = "m", unit = ns, table = "t"),
    clock(service = injected_monotonic, service = injected_monotonic),
    smoke(host_pure),
)]
fn b(_rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    Ok(())
}

fn main() {}
