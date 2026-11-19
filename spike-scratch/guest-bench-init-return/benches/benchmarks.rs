//! Run through `cargo xtask metal run --benchmark guest-init-return --spike guest-bench-init-return`.
use guest_bench_init_return as _;

fn main() -> std::process::ExitCode {
    overdrive_bench::run_main(overdrive_bench::Evidence::Spike)
}
