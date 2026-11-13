# Slice 03 — Lane and launcher placement on the test

Story: US-PLACE-03. Job: J-PLAT-007. Depends on: slice 01.

## Outcome

Guest-lane operator and owner fixtures and native-launcher runner self-tests declare their placement on the test and pass on Metal with their table entries deleted. After this slice every placement kind in the table is declarable on a test.

## In the slice

- Mapping of the 16 non-guest-group, non-host-pure table entries onto the declaration (how is DESIGN's open question; the issue lists it as undecided).
- Lane fixtures keep running their first-level in-guest fixture in a fresh guest.
- Native-launcher tests keep the node view: private network (vsock local mode), mount and cgroup namespaces, own bpffs and per-run cgroup.

## Not in the slice

Re-deciding which tests are native-launcher (only runner self-tests qualify per testing.md); changing lane semantics.

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run --features integration-tests,kvm-tests -E 'test(a_burst_of_host_aborts_resets_every_application_connection)'` for the owner test (fixture `integration::in_guest_lane::a_burst_of_host_aborts_resets_every_connection_in_a_real_guest`), plus its operator counterpart and one native-launcher self-test picked from the table's entries at slice start.

## Acceptance

- [ ] Operator and owner lane tests pass with the fixture named on the test and no table entry.
- [ ] A lane test naming a missing fixture fails naming the fixture.
- [ ] A native-launcher self-test passes and still observes its own namespaces and cgroup.
- [ ] If a kind cannot be expressed, DESIGN records why and the story returns to DISCUSS rather than leaving a table remnant.

## Estimate

2 to 3 days (depends on DESIGN's mapping).
