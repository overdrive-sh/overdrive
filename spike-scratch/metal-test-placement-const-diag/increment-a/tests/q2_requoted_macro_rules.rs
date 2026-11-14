// Q2b: a macro re-quotes the user's literal into the const item.
use const_diag_probe::GuestPlacement;
macro_rules! shared_guest {
    (guest = $g:literal) => { const _: GuestPlacement = GuestPlacement::declare($g, "base"); };
}
shared_guest!(guest = "no-such-group");
#[test]
fn t() {}
