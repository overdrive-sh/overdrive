//! The maintained benchmark binary: `cargo bench -p overdrive-bench-families --bench benchmarks`.
// The reference to the declaring crate is required: rustc does not link a
// dependency nothing references, and its registrations would vanish (DDD-3).
use overdrive_bench_families as _;

fn main() -> std::process::ExitCode {
    overdrive_bench::run_main(overdrive_bench::Evidence::Maintained)
}
