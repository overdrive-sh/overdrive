//! S-ND295-00 Lima real-kernel inventory evidence for D-295-DISTILL-12.
//!
//! These examples stay separate from the source-local raw projection and
//! capture-failure tables in `guest_tcx.rs`. They own actual enumeration,
//! by-ID observation, exact bpffs paths, and retained-object evidence after
//! loader/adopted-handle release.

#![allow(clippy::doc_markdown)]

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 02-01: S-ND295-00 real eight-family TCX inventory"]
fn clean_and_receipted_inventory_observes_all_eight_exact_families() {
    // Activation contract: run as Lima root; capture through the real aya
    // source; load/pin/insert/attach through D12; execute explicit reverse
    // cleanup and both handle-release boundaries; then require exact zero for
    // EndpointMap, CounterMap, EndpointEntry, TcxProgram, TcxLink,
    // EndpointMapPin, CounterMapPin, and TcxLinkPin. Repeat with one
    // deliberately retained receipted family and require its exact positive
    // count while every other family remains observed zero.
    panic!("Not yet implemented -- RED scaffold (S-ND295-00 D12 real eight-family inventory)");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 02-01: S-ND295-00 retained unpinned TCX objects"]
fn retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable() {
    // Activation contract: retain an independent kernel reference to one
    // receipted endpoint map, counter map, classifier program, and TCX link;
    // remove their pins, close the loader, release adopted handles, and prove
    // real loaded-object/by-ID enumeration still reports the exact receipt.
    // Cleanup is exact-owner-only and restores the before/after BPF inventory.
    panic!("Not yet implemented -- RED scaffold (S-ND295-00 retained unpinned object inventory)");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 02-01: S-ND295-00 exact-path ownership and schema refusal"]
fn wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero() {
    // Activation contract: put a different valid BPF object at each planned
    // path and exercise valid-but-wrong map kinds/key/value/capacity. Require
    // source-less OwnershipMismatch/MapSchemaMismatch with opaque semantic
    // values; an unreceipted candidate (including a unique match) is
    // InventoryAmbiguous. None of these branches may return Ok(0).
    panic!("Not yet implemented -- RED scaffold (S-ND295-00 exact-path ownership/schema)");
}
