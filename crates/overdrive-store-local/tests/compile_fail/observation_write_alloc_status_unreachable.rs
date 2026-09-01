//! R1 type-level closure: `ObservationWrite` has no allocation-status variant.

use overdrive_core::traits::observation_store::ObservationWrite;

fn main() {
    let _ = ObservationWrite::AllocStatus;
}
