use overdrive_testing::overdrive_test;

#[test]
#[overdrive_test(guest = "cgroup-default", profile = "no-such-profile")]
fn a_test() {}

fn main() {}
