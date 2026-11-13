use probe_macros::{overdrive_test, shared_guest};

shared_guest!(guest = "probe");
shared_guest!(guest = "core");

#[overdrive_test]
fn t() {}
