//! CPU benchmarks for `overdrive-core` pure hot paths.
//!
//! Run locally with plain divan (`cargo bench -p overdrive-bench`) or
//! under `CodSpeed` instrumentation:
//!
//! ```text
//! cargo codspeed build -m simulation -p overdrive-bench
//! codspeed run -m simulation -- cargo codspeed run -p overdrive-bench
//! ```
//!
//! Every benchmark targets a pure, deterministic function (no clock, no
//! entropy, no I/O), so results are stable under `CodSpeed`'s CPU
//! simulation instrument. Inputs are built outside the measured closure
//! via `with_inputs` / static setup.

#![allow(clippy::expect_used, clippy::wildcard_imports)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;

use divan::{Bencher, black_box};
use overdrive_core::aggregate::WorkloadSpecInput;
use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::guest_transport::wire::DatagramFrame;
use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
use overdrive_core::maglev::permutation::{Weight, generate};
use overdrive_core::traits::dataplane::Backend;

fn main() {
    divan::main();
}

// ---------------------------------------------------------------------------
// Maglev permutation table generation
// ---------------------------------------------------------------------------

mod maglev {
    use super::*;

    fn backends(n: u32) -> BTreeMap<BackendId, Weight> {
        (1..=n).map(|i| (BackendId::new(i).expect("non-zero BackendId"), 1)).collect()
    }

    /// Full table regeneration at the default `M = 16_381` (Cilium /
    /// Katran default) for growing backend counts.
    #[divan::bench(args = [1, 10, 100])]
    fn generate_default_m(bencher: Bencher, n: u32) {
        let m = MaglevTableSize::DEFAULT;
        let set = backends(n);
        bencher.bench_local(|| generate(black_box(&set), black_box(m)));
    }

    /// Weighted backends (multiplicity expansion) on a smaller table.
    #[divan::bench]
    fn generate_weighted_4093(bencher: Bencher) {
        let m = MaglevTableSize::new(4_093).expect("4093 is an allowed prime");
        let set: BTreeMap<BackendId, Weight> = (1..=16u32)
            .map(|i| {
                (BackendId::new(i).expect("non-zero BackendId"), u16::try_from(i).unwrap_or(1))
            })
            .collect();
        bencher.bench_local(|| generate(black_box(&set), black_box(m)));
    }
}

// ---------------------------------------------------------------------------
// Backend-set fingerprint (rkyv archive + blake3)
// ---------------------------------------------------------------------------

mod backend_fingerprint {
    use super::*;

    fn backends(n: u8) -> Vec<Backend> {
        (0..n)
            .map(|i| Backend {
                alloc: SpiffeId::new(&format!(
                    "spiffe://overdrive.local/workload/payments/alloc/a{i:03}"
                ))
                .expect("valid SPIFFE id"),
                addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, i)), 8080),
                weight: 100,
                healthy: true,
            })
            .collect()
    }

    #[divan::bench(args = [1, 16, 128])]
    fn fingerprint_backends(bencher: Bencher, n: u8) {
        let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1))).expect("valid VIP");
        let set = backends(n);
        bencher.bench_local(|| fingerprint(black_box(&vip), black_box(&set)));
    }
}

// ---------------------------------------------------------------------------
// Workload spec TOML parsing (operator-facing driving port)
// ---------------------------------------------------------------------------

mod workload_spec {
    use super::*;

    const JOB: &str = include_str!("../../../examples/coinflip.toml");
    const SCHEDULE: &str = include_str!("../../../examples/nightly-backup.toml");
    const SERVICE: &str = include_str!("../../../examples/service-kind-vm-workloads/service.toml");

    #[divan::bench]
    fn parse_job() -> WorkloadSpecInput {
        WorkloadSpecInput::from_toml_str(black_box(JOB)).expect("job spec parses")
    }

    #[divan::bench]
    fn parse_schedule() -> WorkloadSpecInput {
        WorkloadSpecInput::from_toml_str(black_box(SCHEDULE)).expect("schedule spec parses")
    }

    #[divan::bench]
    fn parse_service_with_probes() -> WorkloadSpecInput {
        WorkloadSpecInput::from_toml_str(black_box(SERVICE)).expect("service spec parses")
    }
}

// ---------------------------------------------------------------------------
// Identifier parsing
// ---------------------------------------------------------------------------

mod ids {
    use super::*;

    #[divan::bench]
    fn spiffe_id_parse() -> SpiffeId {
        SpiffeId::from_str(black_box("spiffe://overdrive.local/workload/payments/alloc/a1b2c3"))
            .expect("valid SPIFFE id")
    }
}

// ---------------------------------------------------------------------------
// Guest datagram frame codec
// ---------------------------------------------------------------------------

mod datagram_frame {
    use super::*;

    const SIZES: &[usize] = &[64, 1_500, 59_000];

    #[divan::bench(args = SIZES)]
    fn encode(bencher: Bencher, len: usize) {
        let payload = vec![0xA5u8; len];
        bencher.bench_local(|| {
            DatagramFrame::new(black_box(&payload)).expect("payload within max").encode()
        });
    }

    /// Decode validates the header and borrows the payload (no copy), so
    /// its cost is independent of payload size; one MTU-sized frame.
    #[divan::bench]
    fn decode(bencher: Bencher) {
        let payload = vec![0xA5u8; 1_500];
        let frame = DatagramFrame::new(&payload).expect("payload within max").encode();
        bencher.bench_local(|| {
            DatagramFrame::decode(black_box(&frame)).expect("canonical frame").payload().len()
        });
    }
}
