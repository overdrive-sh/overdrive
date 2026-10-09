# Pre-registration for increment-d (written after increment-c, before increment-d ran)

increment-c (private veth fixture; tokenised epoll) matched the increment-b predictions for A1, R1, R2, B1, B2, B3.
Two things came out of it that increment-d tests on purpose:

1. **B4 residue at k=0.** One late reply (1 of 60 cycles, both `none` and `recreate`, k=0 ms) reached the
   application still framed (30,009 bytes, `ZUD1` header). Hypothesis: the end-of-association sequence
   (legs closed -> unframe tuple removed -> `up` disconnected -> cells cleared) removes the guest-`lo` unframe
   tuple while the reassembly cell can still deliver a frame to `up`. Predicted: with `--order cutfirst`
   (cut the reassembly cell's output first, clear the cells, disconnect `up`, remove the tuple last) 0
   mismatches in 400 cycles at k=0 and k=1 for `empty` and `recreate`; with `--order old`, >= 1 mismatch in
   400 cycles at k=0 for `recreate` (rate was 1/60; not guaranteed). Falsification of the hypothesis: a
   mismatch under `cutfirst`.
2. **B5: frames still in a psock backlog when the cells are cleared.** The increment-c B1 frames were already
   in the cell's receive queue when it was cleared. B5 makes them wait in the egress target's psock backlog:
   guest framing cell with 4 KiB socket buffers (`--cell-bufs 4096`), 200 x 1,000-byte datagrams sent in a
   burst that ends before the host answers; the host answers Refused after 500 ms (`--refuse-delay-ms 500`)
   and, because only odd associations are refused, would pair a second association if one were started.
   Everything sent was sent before the clear, so any delivery after it is residue. Predicted: `recreate`:
   no "A" frame at the destination, 1 association per repetition (closing the cell's sockets drops their
   psock and its backlog). `empty`: UNCERTAIN; the emptied receive queue frees window so the sender's queue
   and the psock backlog can refill it after the clear (residue, a second association, A frames delivered).
   If `empty` delivers any, K-B3 holds for "recreated" only. `none`: A frames delivered.

The full suite is run again with `--order cutfirst` for `empty` and `recreate` (`none` keeps `--order old`), so
the verdicts for (a) and (b) rest on one ordering.
