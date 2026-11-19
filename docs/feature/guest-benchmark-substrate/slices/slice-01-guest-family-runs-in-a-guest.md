# Slice 01 — A guest family runs inside a guest and returns the same record (walking skeleton)

Story: US-GBS-01. Job: J-PLAT-008. Family: the first guest family (working name `guest-flow-forwarding`, open point O1), `guest` substrate.

## Goal
A benchmark function declared with the `guest` substrate runs through `cargo xtask metal run --benchmark <family>`: the host half boots a guest on the exclusive Metal host, the half inside the guest measures a real in-guest production component, and the retained record and report come back in the existing schema.

## In scope
- `guest` accepted as a registered substrate beside `native`; an unregistered one still refused by name.
- The host half and the in-guest half of the runner, returning every sample to the record.
- One guest family, one profile, one arm, at least 9 replicates: the production guest flow owner's forwarding path (observable per O1).
- Record and report state `substrate guest` and that the guest was launched on the host, not nested.
- An unprepared kit fails every replicate with `guest boot: <cause>` naming `cargo xtask guest-kit-prepare`; no guest boots and no sample is taken (feature delta CA-4; not a refusal).
- Catalogue row for the family; the D9 rule stated; `vm-lifecycle` stays native.

## Out of scope
Guest view, host-block refusal changes and vCPU marking (slice 02); failure shapes beyond the unprepared-kit failed replicates (slice 03); smoke variant (slice 04). This feature delivers one one-arm family; further families are out of scope of this feature (the in-guest mTLS comparison is #303; the other forwarding paths are #370).

## Learning hypothesis
Disproves: that a runner half inside a guest-kit guest can return unchanged-schema samples through the Metal benchmark form. If the record needs a second schema or the form needs a new command, the runner-half shape is wrong. Confirms D3, D4 and D5 end to end with production data (a real in-guest component, not a synthetic loop).

## Acceptance
- [ ] Every in-guest sample retained; report derived from them; substrate `guest`.
- [ ] `bench-report validate` accepts the record unchanged.
- [ ] Record states the guest was not nested.
- [ ] Committed native records still validate; `vm-lifecycle` still says native.
- [ ] Unprepared kit: every replicate fails naming the setup command; no guest boots, no sample.
- [ ] No launch of the `overdrive` binary in the command log.

## Dependencies
`native-benchmark-program` shipped; guest kit and in-guest lane; O1 (user), O6 (DESIGN), O3 (spike) before the design is approved.

## Estimate
2 to 3 days. Reference class: the in-guest lane skeleton (guest kit plus lane, 2 to 3 days). Pre-slice spike: O3 (boot granularity), because a fresh guest per replicate changes the record's contents.
