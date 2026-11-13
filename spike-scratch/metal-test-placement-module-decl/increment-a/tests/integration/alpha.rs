use probe_macros::{overdrive_test, shared_guest};

shared_guest!(guest = "cgroup-default");

#[overdrive_test]
fn plain_no_test_attr() {
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "cgroup-default");
    placement_decl_probe::dump("alpha");
}

#[overdrive_test]
#[test]
fn ours_then_test() {
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "cgroup-default");
}

#[test]
#[overdrive_test]
fn test_then_ours() {
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "cgroup-default");
}

#[overdrive_test]
#[tokio::test]
async fn ours_then_tokio() {
    tokio::task::yield_now().await;
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "cgroup-default");
}

#[tokio::test]
#[overdrive_test]
async fn tokio_then_ours() {
    tokio::task::yield_now().await;
    assert_eq!(__OVERDRIVE_MODULE_PLACEMENT, "cgroup-default");
}
