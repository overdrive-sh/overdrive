# Pre-registration for increment-e (written after increment-d was interrupted, before increment-e ran)

increment-d was cut off by the tool's 30-minute limit after A1, the B4 follow-up and B5. Results kept
(increment-d/evidence): A1 200/200 and 200 associations for all policies (with `--order cutfirst` for
`empty` / `recreate`); B4 k=0, 400 cycles: `old` ordering + `recreate` 1 framed late reply, `cutfirst` 0
(both policies); k=1: 0 mismatches everywhere.

**B5 in increment-d used a wrong oracle and is not used for verdicts.** It paired odd-numbered
associations to count a "second" association, but each repetition makes one association, so parity alternated
between repetitions and the "delivered" frames in `recreate` (330) belong to repetitions whose single
association was legitimately paired. (Its numbers stay in increment-d/evidence.)

B5 revised (increment-e): same traffic (guest framing cell with 4 KiB buffers, 200 x 1,000 B burst finished
before the host answers, answer = Refused after 500 ms), but the host refuses EVERY association (no pairing
ever), so nothing can be delivered and the observable is the number of associations per repetition (each
repetition is a new socket, identified by its guest source port). All frames were sent before the clear, so any
association after the first is started by residue of the old frames. Predicted: `recreate`: exactly 1 per
repetition. `empty`: UNCERTAIN (> 1 means the emptied cell was refilled from the sender's queue / the psock
backlog; then K-B3 holds for "recreated" only). `none`: a storm.

increment-e also runs the rest of the suite that increment-d did not reach (R1, R2, R3, B1, B2, B3, B4), unchanged
from increment-c except `--order cutfirst` for `empty` / `recreate`.
