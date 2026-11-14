use probe_macros::{overdrive_test, overdrive_test_embed_known, shared_guest, shared_guest_embed_known};
shared_guest!(guest = "core");
shared_guest_embed_known!(guest = "ulp");
#[overdrive_test(guest = "mtls-relay", profile = "wide")]
fn a() {}
#[overdrive_test_embed_known(guest = "stress")]
fn b() {}
