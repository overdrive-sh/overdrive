use probe_macros::overdrive_test_embed_known;
#[overdrive_test_embed_known(guest = "no-such-group")]
fn it_runs() {}
