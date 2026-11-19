# Slice 03 — Guest failures are reported, never hidden, and no guest outlives the run

Story: US-GBS-03. Job: J-PLAT-008. Family: the guest family of slice 01.

## Goal
A guest that cannot boot, dies, or fails a replicate is a counted failed replicate with its cause, with the guest console retained, and no guest VMM survives the run (feature delta CA-1 and CA-4).

## In scope
- Boot failure (including an unprepared guest kit) is a failed replicate with its cause (`guest boot: <cause>`); no sample, no fallback to measuring on the host.
- A guest death in one replicate fails that replicate; the other replicates stay or run and the run finishes. A run is reported incomplete only when the benchmark process ends without a terminal line (a killed run).
- A failed replicate in the guest counted with replicate and arm.
- The guest console retained with the result directory in every end state.
- No guest VMM remains after a normal, failed or killed run.

## Out of scope
Failure accounting only: a failed replicate is counted, never retried or dropped, and a killed run is incomplete; leak sweeps of unrelated Metal state; VM-lifecycle behaviours.

## Learning hypothesis
Disproves: that the parent's failure accounting (counted, never dropped, never zero-filled) extends to a mechanism with a second process boundary. If a dead guest can read as an empty or fast result, or a guest survives a killed run, the host half does not own the guest's lifetime.

## Acceptance
- [ ] Boot failure named; no host fallback.
- [ ] A guest death fails that replicate (others retained or run, run finishes); only a killed run is reported incomplete.
- [ ] Failed replicate counted with replicate and arm.
- [ ] Console retained in every end state.
- [ ] No guest VMM left after a killed run.

## Dependencies
Slice 01.

## Estimate
1 to 2 days. Reference class: the in-guest lane's failure handling (`LaneNotPrepared`, `GuestKitVm` drop-kills).
