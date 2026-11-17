//! The spike's benchmark binary, run through `cargo xtask metal run --benchmark
//! cgroup-connect-hook --spike pig-connect-cost`.
// The reference to the declaring crate is required: rustc does not link a
// dependency nothing references, and its registrations would vanish.
use pig_connect_cost as _;

fn main() -> std::process::ExitCode {
    overdrive_bench::run_main(overdrive_bench::Evidence::Spike)
}
