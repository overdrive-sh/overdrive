//! Run through `cargo xtask metal run --benchmark <family> --spike guest-bench-return`.
use guest_bench_return as _;

fn main() -> std::process::ExitCode {
    overdrive_bench::run_main(overdrive_bench::Evidence::Spike)
}
