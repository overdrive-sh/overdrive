// Q2a: hand-written const item with a literal-only panic from declare.
use const_diag_probe::GuestPlacement;
const _: GuestPlacement = GuestPlacement::declare("no-such-group", "base");
#[test]
fn t() {}
