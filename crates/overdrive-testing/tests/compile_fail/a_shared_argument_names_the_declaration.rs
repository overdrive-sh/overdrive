use overdrive_testing::overdrive_test;

#[test]
#[overdrive_test(guest = "cgroup-default", shared)]
fn a_test() {}

fn main() {}
