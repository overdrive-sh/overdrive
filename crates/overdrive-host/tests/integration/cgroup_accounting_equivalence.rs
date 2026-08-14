//! Tier-3 acceptance — `CgroupAccounting` adapter equivalence, including
//! the ADR-0082 §D8 probe fault table (S-VM-93).
//!
//! Gated `integration-tests` ONLY (see `tests/integration.rs`) —
//! deliberately NOT `kvm-tests`: this suite probes `CgroupAccounting`'s
//! three fault rows against synthetic/constructed cgroup-shaped fixture
//! files on a real filesystem (`tempfile::TempDir`). It never goes
//! through the `Vmm` port and never spawns `cloud-hypervisor`, so it
//! needs no real KVM substrate and runs under
//! `cargo xtask lima run -- cargo nextest run -p overdrive-host --features
//! integration-tests -E 'test(cgroup_accounting_equivalence)'`.
//!
//! Per Mandate 9 (`nw-tdd-methodology`): this is a FIXED, hand-enumerated
//! call sequence at layer 3 — `@example`, not `@property`.

use std::path::Path;

use overdrive_core::traits::cgroup_accounting::{CgroupAccounting, CgroupAccountingError};
use overdrive_host::RealCgroupAccounting;
use overdrive_sim::SimCgroupAccounting;

/// Well-formed `memory.events` content carrying a non-zero `oom_kill`
/// counter — the healthy-substrate fixture shared by both adapters'
/// read-once assertions.
const HEALTHY_BODY_OOM_KILL_3: &str = "low 0\nhigh 0\nmax 0\noom 0\noom_kill 3\noom_group_kill 0\n";

/// The shared call sequence S-VM-93 drives against BOTH adapters:
/// `oom_kill_count` against a healthy path returns the real counter, and
/// re-reads are NOT cached (read-once semantics per ADR-0082 §D8 — "no
/// adapter caches or re-reads the value across calls" means each call
/// independently re-observes the substrate, never memoizes a stale
/// answer).
async fn assert_oom_kill_count_reads_the_real_counter(
    acct: &dyn CgroupAccounting,
    memory_events_path: &Path,
) {
    let first = acct
        .oom_kill_count(memory_events_path)
        .await
        .expect("oom_kill_count succeeds against a well-formed memory.events");
    assert_eq!(first, 3, "must read the real oom_kill counter, not a default");

    let second = acct
        .oom_kill_count(memory_events_path)
        .await
        .expect("a second read against the same path succeeds identically");
    assert_eq!(second, 3, "a second read must observe the same substrate value");
}

#[tokio::test]
async fn cgroup_accounting_equivalence_sim() {
    let acct = SimCgroupAccounting::new();
    let path = std::path::PathBuf::from("/sim/alloc-cgroup-eq/memory.events");
    acct.set_oom_kill_count(path.clone(), 3);

    // probe -- idempotent, called twice, healthy by default.
    acct.probe().await.expect("probe succeeds against a healthy sim substrate");
    acct.probe().await.expect("probe is idempotent -- second call also succeeds");

    // oom_kill_count -- read-once semantics against a known path.
    assert_oom_kill_count_reads_the_real_counter(&acct, &path).await;
    assert_eq!(
        acct.read_call_count(&path),
        2,
        "each oom_kill_count call must independently re-observe the substrate -- no caching"
    );

    // Fault class 1 -- Substrate (probe read fails).
    acct.inject_probe_substrate_error(std::io::ErrorKind::NotFound);
    let err = acct.probe().await.expect_err("an injected substrate fault must surface");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::Substrate { .. }
        ),
        "expected Substrate, got {err:?}"
    );
    acct.probe().await.expect("the one-shot fault is consumed -- the next probe is healthy again");

    // Fault class 2 -- SubstrateCorrupt (probe read is not valid UTF-8).
    acct.inject_probe_substrate_corrupt(vec![0xFF, 0xFE, 0x00]);
    let err = acct.probe().await.expect_err("an injected corrupt-substrate fault must surface");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::SubstrateCorrupt { .. }
        ),
        "expected SubstrateCorrupt, got {err:?}"
    );

    // Fault class 3 -- MissingOomKillKey (probe read has no oom_kill line).
    acct.inject_probe_missing_oom_kill_key("low 0\nhigh 0\n");
    let err = acct.probe().await.expect_err("an injected missing-key fault must surface");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::MissingOomKillKey { .. }
        ),
        "expected MissingOomKillKey, got {err:?}"
    );

    // oom_kill_count's own two error variants (Io / Malformed),
    // exercised for parity with the real adapter's read path below.
    let absent = std::path::PathBuf::from("/sim/absent/memory.events");
    acct.inject_read_io_error(absent.clone(), std::io::ErrorKind::NotFound);
    let err = acct.oom_kill_count(&absent).await.expect_err("injected read Io fault must surface");
    assert!(matches!(err, CgroupAccountingError::Io { .. }), "expected Io, got {err:?}");

    let malformed = std::path::PathBuf::from("/sim/malformed/memory.events");
    acct.inject_read_malformed(malformed.clone(), "no oom_kill line here");
    let err = acct
        .oom_kill_count(&malformed)
        .await
        .expect_err("injected read Malformed fault must surface");
    assert!(
        matches!(err, CgroupAccountingError::Malformed { .. }),
        "expected Malformed, got {err:?}"
    );

    assert_eq!(acct.kind(), "overdrive_sim::SimCgroupAccounting");
}

#[tokio::test]
async fn cgroup_accounting_equivalence_real() {
    let tmp = tempfile::TempDir::new().expect("tempdir");

    // Healthy fixture shared by the probe round-trip and the read-once
    // oom_kill_count assertion.
    let healthy_path = tmp.path().join("memory.events");
    std::fs::write(&healthy_path, HEALTHY_BODY_OOM_KILL_3).expect("write healthy fixture");

    let acct = RealCgroupAccounting::new().with_probe_path(healthy_path.clone());

    // probe -- idempotent, called twice, against the real healthy file.
    acct.probe().await.expect("probe succeeds against a well-formed real memory.events");
    acct.probe().await.expect("probe is idempotent -- re-reading the same file also succeeds");

    // oom_kill_count -- read-once semantics against the same real file.
    // Every call independently re-`tokio::fs::read`s -- no adapter-side
    // cache to go stale.
    assert_oom_kill_count_reads_the_real_counter(&acct, &healthy_path).await;

    // Fault class 1 -- Substrate (a genuinely absent file: ENOENT).
    let absent_probe =
        RealCgroupAccounting::new().with_probe_path(tmp.path().join("absent-events"));
    let err = absent_probe.probe().await.expect_err("a missing memory.events must fail the probe");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::Substrate { .. }
        ),
        "expected Substrate, got {err:?}"
    );
    let err = absent_probe
        .oom_kill_count(&tmp.path().join("absent-events"))
        .await
        .expect_err("a missing memory.events must fail oom_kill_count with Io");
    assert!(matches!(err, CgroupAccountingError::Io { .. }), "expected Io, got {err:?}");

    // Fault class 2 -- SubstrateCorrupt (real bytes that are not valid UTF-8).
    let corrupt_path = tmp.path().join("corrupt-events");
    std::fs::write(&corrupt_path, [0xFF_u8, 0xFE, 0x00, 0x01]).expect("write non-UTF-8 fixture");
    let corrupt_probe = RealCgroupAccounting::new().with_probe_path(corrupt_path.clone());
    let err = corrupt_probe.probe().await.expect_err("non-UTF-8 content must fail the probe");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::SubstrateCorrupt { .. }
        ),
        "expected SubstrateCorrupt, got {err:?}"
    );

    // Fault class 3 -- MissingOomKillKey (real, valid UTF-8, but no
    // oom_kill line -- e.g. the memory controller was never enabled for
    // this scope).
    let missing_key_path = tmp.path().join("no-oom-kill-events");
    std::fs::write(&missing_key_path, "low 0\nhigh 0\nmax 0\n").expect("write missing-key fixture");
    let missing_key_probe = RealCgroupAccounting::new().with_probe_path(missing_key_path.clone());
    let err = missing_key_probe
        .probe()
        .await
        .expect_err("content with no oom_kill line must fail the probe");
    assert!(
        matches!(
            err,
            overdrive_core::traits::cgroup_accounting::CgroupAccountingProbeError::MissingOomKillKey { .. }
        ),
        "expected MissingOomKillKey, got {err:?}"
    );
    let err = missing_key_probe
        .oom_kill_count(&missing_key_path)
        .await
        .expect_err("content with no oom_kill line must fail oom_kill_count with Malformed");
    assert!(
        matches!(err, CgroupAccountingError::Malformed { .. }),
        "expected Malformed, got {err:?}"
    );

    assert_eq!(acct.kind(), "overdrive_host::RealCgroupAccounting");
}
