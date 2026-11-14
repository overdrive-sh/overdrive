use overdrive_testing::overdrive_test;

#[overdrive_test(guest = "cgroup-default", profile = "no-such-profile")]
#[test]
fn a_test() {}

fn main() {}
