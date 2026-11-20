//! The spike's benchmark binary, run through `cargo xtask metal run --benchmark
//! guest-flow-forwarding --spike guest-flow-spike`.
// The reference to the declaring crate is required: rustc does not link a
// dependency nothing references, and its registrations would vanish.
use guest_flow_spike as _;

fn main() -> std::process::ExitCode {
    overdrive_bench::run_main(overdrive_bench::Evidence::Spike)
}
