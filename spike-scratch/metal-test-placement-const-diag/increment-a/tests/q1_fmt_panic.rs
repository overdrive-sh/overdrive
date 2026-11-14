// Q1: panic! with a runtime &str argument inside a const fn evaluated in const context.
const fn declare(group: &str) -> usize {
    if group.len() > 100 {
        panic!("unknown guest group `{}`", group);
    }
    group.len()
}
const X: usize = declare("nonsense");
#[test]
fn t() { assert_eq!(X, 8); }
