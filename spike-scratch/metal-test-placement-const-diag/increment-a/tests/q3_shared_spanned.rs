use probe_macros::shared_guest_spanned;
shared_guest_spanned!(guest = "no-such-group");
#[test]
fn t() {}
