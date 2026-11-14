use overdrive_testing::overdrive_test;

#[overdrive_test(guest = "no-such-group")]
#[test]
fn a_test() {}

fn main() {}
