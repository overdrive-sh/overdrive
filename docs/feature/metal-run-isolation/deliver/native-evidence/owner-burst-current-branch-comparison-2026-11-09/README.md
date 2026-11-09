# Owner burst cell comparison at the current branch source

This record compares the ordinary selected-cell route with the original
`integration::in_guest_lane` fixture at source commit
`5f4fc23fb4c5590b3f88b411915938be180fa64a`. Each raw invocation receipt
retains the exact source status, selector, timestamps and outer result. The
working tree included unrelated dirty files; those paths are preserved in the
receipts.

## Ordinary selected-cell route

The selected pair was run three times through the P4 dispatcher:

- attempt 094958: operator cell passed; owner cell failed after the existing
  one-second abort-settle bound, with 70 of 1,000 ends consumed;
- attempt 095856: operator cell passed; owner cell failed at the same assertion,
  with 78 ends consumed;
- attempt 100543: operator cell passed; owner cell failed at the same assertion,
  with 80 ends consumed.

Each outer Nextest command returned 100. These attempts used the successful
canonical preparation receipt in `selected-nextest/preparation-094442/`.
Failure-only dispatcher diagnostics were enabled in the working tree for
these captures; they preserved the selected test's exit status and included
its guest console.

## Original fixture path

The existing host fixture
`integration::in_guest_lane::a_burst_of_host_aborts_resets_every_connection_in_a_real_guest`
was then run directly through its original `run_owner_cell` composition using
the prepared in-guest test binary, the same 512 MiB / two-vCPU owner lane and
the authored host script. It also failed the same
`real_kernel.rs:1623` assertion: only 78 burst ends were consumed within the
one-second bound. Its host side then observed EOF waiting for `DONE`. The
outer command returned 100. Its preparation receipt is
`original-fixture/preparation-101347/`.

This comparison reproduces the owner-cell failure independently of the new
Nextest dispatcher. Both runs use the current branch source after the
P4 implementation. It does not establish whether the failure existed before
this branch or identify a permissible product-side correction.

All copied raw logs and invocation receipts were checked byte-for-byte against
their original `/tmp/metal-native-remaining-*` captures. No test body, oracle,
timeout, memory, or product owner was changed.
