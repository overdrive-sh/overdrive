// NEGATIVE: a plain #[test] that returns Result is refused by the V-1 rule
// "tests must return ()".
use probe_macros::{overdrive_test, shared_guest};
shared_guest!(guest = "cgroup-default");

#[overdrive_test]
fn returns_result() -> Result<(), String> { Ok(()) }
