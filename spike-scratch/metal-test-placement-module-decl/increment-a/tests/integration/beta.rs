use probe_macros::{overdrive_test, shared_guest};

shared_guest!(guest = "probe");

#[overdrive_test]
fn resolves_its_own_group() {
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "probe");
    placement_decl_probe::dump("beta");
}
