use probe_macros::shared_guest_embed;
shared_guest_embed!(guest = "no-such-group");
#[test]
fn t() {}
