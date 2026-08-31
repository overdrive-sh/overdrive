//! R1 type-level closure: allocation current rows cannot reach the generic
//! observation writer.

use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};

async fn write_raw_alloc_row<S>(store: &S, row: ObservationRow)
where
    S: ObservationStore,
{
    store.write(row).await.expect("write should compile");
}

fn main() {}
