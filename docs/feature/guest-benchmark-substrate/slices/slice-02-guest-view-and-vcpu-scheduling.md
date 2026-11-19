# Slice 02 — The guest's view, the real host's state and vCPU scheduling in the record (walking skeleton)

Story: US-GBS-02. Job: J-PLAT-008. Family: the guest family of slice 01.

## Goal
A guest record says what the guest could see, what the real host was, and how the vCPUs were scheduled, and a drifted host is refused before any guest boots.

## In scope
- Host eligibility on the real host before any guest boots; refusal names setting, found and pinned values.
- A guest block in record and report: the conditions in `run_started`, and the guest kernel, vCPU count and clock source from the one `guest_observed` line; when every replicate failed before reporting a view these read `absent: no replicate returned a guest view` (CA-4).
- Settings a guest cannot see (governor, SMT, boost, frequency) recorded absent with the reason "not visible from a guest".
- vCPU count and `unpinned` scheduling stated.
- Eligibility as applied to a guest settled (open point O2) and written in the methodology.
- A guest record and a native record each state their substrate; no command takes both.

## Out of scope
Pinning vCPUs (#368); choosing another guest shape (#369); new pinned settings; failure shapes (slice 03).

## Learning hypothesis
Disproves: that the existing host-block rules carry over to a guest. They do not as written (no readable source refuses), so if O2's rule still refuses every guest or hides an unseeable setting as a default, the eligibility model is wrong. Confirms that both views fit one record without breaking committed native records (O4).

## Acceptance
- [ ] Drifted host refused before any guest boots; no sample.
- [ ] Host block and guest block both in record and report.
- [ ] Guest-unseeable settings absent with the stated reason, none zero-filled.
- [ ] vCPU count and `unpinned` stated; kernel, clocksource and vCPU count come from the `guest_observed` line and are `absent: no replicate returned a guest view` when every replicate failed first (CA-4).
- [ ] No command compares records across substrates.
- [ ] Every committed native record still validates.

## Dependencies
Slice 01 (record); O2 and O4 settled by DESIGN.

## Estimate
1 to 2 days. Reference class: the parent's host-settings slice (1 to 2 days).
