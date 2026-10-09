# V-5(c) — guest UDP re-association after a host abort, and cell residue (K-B3, A-18, U-2 / R5-27)

**Verdict: the observable rule HOLDS; K-B3 is FALSIFIED for "emptied" cells and HOLDS for "recreated" cells.** Two further findings: end-of-association step order, and owner event-loop identity.

Probe: `spike-scratch/netns-density-295-v5c/` (README, `v5c.py`, `launch.py`, `vfwd/src/owner.rs`, `vfwd/src/main.rs`; raw evidence `increment-{a..e}/evidence/`, pre-registrations `increment-{b,d,e}/PREREGISTRATION.md`). aya-rs eBPF reused from the V-11 spike.

## Substrate

Native metal (AMD EPYC 8024P, `systemd-detect-virt` = none, `/dev/kvm` API 12), root over ssh with the spike's own flock (not `cargo xtask metal run`). Host and guest `7.0.0-29-generic` (not the 6.18 pin). Cloud Hypervisor: prebuilt v53.0.0 vhost-kernel binary from the earlier guest-vsock-capture spike, sha256 `bb8f7927…` (not rebuilt; that it was built from fork `41a619d19` was not checked). Guest CID 3261. Guest owner is a Rust stand-in for `overdrive-init`; host owner unframes on a private veth (`v5c-h`, 10.250.205.0/24) with the fragment-aware TC program.

Increment b is **invalid** (host unframe on the shared `lo` disrupted by concurrent spikes; fd-keyed epoll dispatch caused 207 associations for 200 sends) — kept as evidence. Increment d was cut off by the 30-min tool limit; its A1 and B4 are valid, its B5 used a wrong oracle and is not used. Increments c and e are the clean runs.

## Verdicts

| Sub-claim | Verdict |
|---|---|
| (a) After a host-only abort, a CONNECTED guest UDP socket re-associates on its next datagram (A-18) | **HOLDS** |
| (a') U-2 / R5-27: after `Refused` / `Abort` / pairing deadline the frames are discarded, the slot kept, no association starts until the application sends again, at most one attempt per pairing round trip | **HOLDS** when the cells are cleared |
| (b) K-B3, **recreated** framing / reassembly cells deliver nothing of the old frames | **HOLDS** |
| (b) K-B3, **emptied** cells (receive queue discarded, parser reset) deliver nothing of the old frames | **FALSIFIED** for frames still in the cell's sender-side queue / psock backlog (B5: 3–8 associations per repetition instead of 1) |
| Extra: end-of-association step order | **FINDING** — a late reply reached the application unstripped under the old order; 0 after cutting the reassembly cell's output first |
| Extra: owner event loop | **FINDING** — raw-fd readiness dispatch + recreated cells caused 7–8 spurious associations per 200 cycles; 0 with per-registration tokens |

## Key evidence

**A1** (Abort 80 ms after `Paired`, 200 cycles, connected socket): `none`, `empty`, `recreate` and an unconnected comparison all `ok 200/200`, exactly 200 associations, none during the 3 s idle.

**R1** (every association refused, one datagram, 3 s idle) — associations started: `none` (control) 3,512 / 3,348 (Abort); `empty` 1 / 1; `recreate` 1 / 1. **R2**: 10 sends 200 ms apart → 10 associations (`none` 3,805); 500-send burst → `empty` 8, `recreate` 5 (`none` 1,534). **R3**: `[false,false,false,true,true,true,true,true]`, 4 associations (3 refused + 1 paired).

**B1** (5 "A" frames parked, association refused / aborted / deadline, then 3 "B" frames): "A" delivered — `none` 200 / 95 / 75; `empty` and `recreate` 0 in every case; "B" once and in order 40/40, 20/20, 15/15.

**B2** (slot released with parked frames; next socket uses the slot to another destination Y): `empty`/`recreate`: X 0, Y 90, 30/30 echoed. `none`: Y received "A" frames addressed to X (cross-destination leak, P-35 hazard).

**B3** (truncated reply frame then Abort): `empty` 25/25, `recreate` 25/25, `none` 0/25.

**B4** (30,000-byte replies cut by Abort k ms after `Paired`): old order (legs closed → tuple removed → `up` disconnected → cells cleared) — one mismatch each in `none` and `recreate` at k=0, the application received the frame header:
```
{"got":30009,"head":"ZUD1..u1EL00","i":16,"mismatch":true,"ok":false,"want":30001}
```
`cutfirst` order: 0 mismatches in 1,600 cycles (increment d) and 0 in 600 (increment e).

**B5** (200 × 1,000 B burst, 4 KiB buffers, every association refused):
```
recreate  associations per rep [1,1,1,1,1,1,1,1,1,1]   reps with exactly one: 10/10
empty     associations per rep [8,5,4,4,6,5,5,4,6,3]   reps with exactly one:  0/10
none      associations per rep [9,6,6,5,6,6,6,5,6,4]   reps with exactly one:  0/10
```
`empty`, first repetition: the first clear discarded 38,304 bytes from p2; the cell refilled with 7,936 bytes ~10 ms later and after each clear for ~3 s, each refill starting another association. That the residue sits in the sender queue / psock backlog is inferred from the refill pattern, not read from the kernel.

## Design implications

The observable U-2 rule (R5-27) is unchanged. The guest mechanism text changes:

1. "emptied or recreated" becomes **recreated** (feature-delta § Per-kind total orders / UDP slot release, § Guest adaptation contract "Slot release", K-B3 row, the V-5(c) row, ADR-0150).
2. End-of-association order: **cut the reassembly cell's output first**, then close the legs, recreate the cells, then disconnect the slot socket, and remove the guest-`lo` unframe tuple last.
3. `overdrive-init` must not dispatch readiness by raw descriptor number once cells are recreated; use per-registration identity (slot index + generation).
4. Cost, `Abort` → guest teardown finished (stand-in owner, 2 vCPUs, 200 samples): `none` p50 326 µs / p99 408 µs; `empty` 432 / 599 µs; `recreate` 648 / 725 µs. Not a bound measurement; V-9 owns costs.

## Limits

Stand-in owner (the kernel mechanism is the real one; the late-`Paired` → `Abort` answer is not implemented). One kernel, 7.0.0-29. B5 residue location not resolved to a kernel symbol. B4 ordering conclusion rests on the unstripped frame, not a rate. Empty datagrams and > 30,000 B not run here (V-11 covered first-datagram behaviour). Cleanup complete; the `vsock_diag` module loaded during a smoke run was left loaded.

**Gate recommendation:** PROMOTE the observable rule; amend K-B3 and the cell-clearing wording to "recreated"; add the step order and owner-identity requirement to the guest adaptation contract; re-run B5 and A1 on 6.18 at Tier 3.
