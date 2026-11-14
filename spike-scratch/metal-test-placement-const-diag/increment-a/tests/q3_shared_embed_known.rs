use probe_macros::shared_guest_embed_known;
shared_guest_embed_known!(guest = "no-such-group");
#[test]
fn t() {}
