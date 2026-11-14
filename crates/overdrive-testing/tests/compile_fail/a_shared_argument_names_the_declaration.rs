use overdrive_testing::overdrive_test;

#[overdrive_test(guest = "cgroup-default", shared)]
#[test]
fn a_test() {}

fn main() {}
