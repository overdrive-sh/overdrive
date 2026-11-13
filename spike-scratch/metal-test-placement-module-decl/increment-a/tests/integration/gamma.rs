use probe_macros::{overdrive_test, shared_guest};

shared_guest!(guest = "core");

#[overdrive_test]
fn outer_test() {
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "core");
}

// Edge: a nested inline module does NOT inherit the parent's const by name;
// it sees it only through an explicit glob import.
mod inner_with_glob {
    use super::*;
    use probe_macros::overdrive_test;

    #[overdrive_test]
    fn nested_via_glob() {
        assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "core");
    }
}
