use probe_macros::shared_guest;
shared_guest!(guest = "no-such-group");
#[test]
fn t() {}
