use probe_macros::overdrive_test;
#[overdrive_test(guest = "core", profile = "no-such-profile")]
fn it_runs() {}
