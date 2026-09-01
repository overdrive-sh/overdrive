//! R1 type-level closure: the read projection has no reverse conversion into
//! the write projection.

use overdrive_core::traits::observation_store::{ObservationRow, ObservationWrite};

fn reverse(row: ObservationRow) -> ObservationWrite {
    row.into()
}

fn main() {}
