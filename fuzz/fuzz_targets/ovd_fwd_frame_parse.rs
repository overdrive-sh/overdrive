#![no_main]

use libfuzzer_sys::{Corpus, fuzz_target};
use overdrive_fwd_frame_parser_test::parse;

fuzz_target!(|input: &[u8]| -> Corpus {
    let mut raw_avail = [0u8; 8];
    let (avail_bytes, buf) = if input.len() < raw_avail.len() {
        (input, &[][..])
    } else {
        input.split_at(raw_avail.len())
    };
    raw_avail[..avail_bytes.len()].copy_from_slice(avail_bytes);
    let avail = u64::from_le_bytes(raw_avail);

    // `Box<[u8]>` has an allocation layout of exactly `buf.len()` bytes. C is
    // passed that same length, so ASan detects any read beyond the declared
    // input rather than into spare Vec capacity.
    let exact: Box<[u8]> = buf.to_vec().into_boxed_slice();
    let _ = parse(&exact, avail);

    Corpus::Keep
});
