//! US-02 Scenario 2.1 — Default lane uses `SimDriver`, no real processes.
//!
//! Default-lane fixture verifies that worker tests use `SimDriver`
//! (the in-memory `Driver` impl) and never spawn real processes.
//! PORT-TO-PORT: enters via the `Driver` driving port, asserts on
//! `AllocationHandle.pid` — `SimDriver` never sets a PID, so a
//! `Some(_)` would prove a real process was spawned.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverType, Resources};
use overdrive_sim::adapters::driver::SimDriver;

#[tokio::test]
async fn default_lane_does_not_spawn_real_processes() {
    // Driving port — `Driver` trait, wired to `SimDriver`.
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let spec = AllocationSpec {
        alloc: AllocationId::new("alloc-default-lane").expect("valid alloc id"),
        identity: SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
    };

    // Action — enter through the driving port.
    let handle = driver.start(&spec).await.expect("SimDriver::start succeeds");

    // Observable outcome — SimDriver never assigns a PID. If this
    // ever returns `Some(_)`, a real process was spawned in the
    // default lane and the @real-io / default-lane separation has
    // collapsed.
    assert!(
        handle.pid.is_none(),
        "default-lane fixture must not spawn real processes; got pid {:?}",
        handle.pid
    );
    assert_eq!(handle.alloc, spec.alloc);
}
