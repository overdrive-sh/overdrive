# V-5(c) pre-registration (written before increment-b ran)

Claim (feature-delta V-5(c)): (a) after a host-only abort of a datagram association whose slot is still
owned, a CONNECTED guest UDP socket re-associates on its next datagram (A-18, U-2, R5-27); (b) K-B3:
emptying or recreating the guest framing / reassembly cells at the end of an association or at slot
release leaves no residue of the old frames.

Mechanism under test: the V-11 spike's guest mechanism (UDP slot socket with a stream verdict, TCP framing
cell p1->p2, TCP reassembly cell p3->p4 with strparser, `connect4`/`sendmsg4` capture) with the V-5(c)
additions: `--cells none|empty|recreate` (the cell policy applied when an association ends or a slot is
released) and host fault injection. `none` is the spike baseline (the design's cell step absent) and is the
CONTROL: where the claim has teeth, `none` must show the hazard.

Predictions (pass = holds; falsification = what would contradict the claim):

* A1 (connected, host aborts 80 ms after Paired, 200 request/echo cycles 250 ms apart, 3 s idle after):
  all three policies 200/200 byte-exact echoes, exactly 200 guest associations (no association without an
  application send). Falsified by: any timeout or mismatch, or associations != 200. Also run unconnected
  (A1u) as a comparison.
* R1 (every association refused, ONE datagram, 3 s idle): `empty` and `recreate`: exactly 1 association.
  `none`: more than 1 (parked frames re-trigger the association: the control).
* R2 (every association refused; 10 datagrams 200 ms apart; 20 back to back): associations == datagrams
  for the paced case, <= 20 for the burst, for `empty`/`recreate`.
* R3 (first 3 associations refused then accepted; connected socket, 8 sends): cycles 0-2 time out, 3-7
  echo, associations == 8, for `empty`/`recreate`.
* B1 (5 frames "A" parked, association refused / aborted / pairing never answered, then 3 "B" on the SAME
  socket; 40 / 20 / 15 repetitions, two slots so slots are reused): `empty` and `recreate`: the destination
  receives no "A" frame in any repetition, each repetition's 3 "B" frames arrive once, in order, echoed.
  `none`: "A" frames reach the destination (residue).
* B2 (slot released with frames parked while pairing is delayed; the single slot is then taken by a new
  socket sending to ANOTHER destination; 30 repetitions): `empty`/`recreate`: the second destination
  receives only the new socket's frames, the first none. `none`: old frames reach the second destination.
* B3 (host injects a truncated frame into the reply path, then aborts; the same socket then sends again;
  25 repetitions): `recreate`/`empty`: every later echo byte-exact. `none`: a later echo is lost or garbled
  (the strparser still waits for the rest of the old frame).
* B4 (30,000-byte replies cut by a host abort 0/1/2/4/8 ms after Paired; 60 cycles each): no mismatched
  (stale or corrupted) delivery under `empty`/`recreate`; timeouts are allowed (the abort cut the reply).

What each outcome changes: see findings-v5c.md.
