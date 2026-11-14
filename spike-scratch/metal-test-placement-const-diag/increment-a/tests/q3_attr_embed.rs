use probe_macros::overdrive_test_embed;
#[overdrive_test_embed(guest = "no-such-group")]
fn it_runs() {}
