# Slice 04 — Guest artifacts declared per group

Story: US-PLACE-04. Job: J-PLAT-007. Depends on: slice 01 (skeleton shows what the per-test path stages today).

## Outcome

What a guest of a given group needs (for `cgroup-default`: `rustc`, a C linker toolchain, `bpftool`, busybox and the group's other staged items) is declared once per group, outside the dispatcher's code paths, and every test of that group, migrated or not, gets it.

## In the slice

- One declaration location per group (shape and location are DESIGN's).
- The dispatcher's copy lists for the declared groups read from it, so there is a single definition.
- A migrated test that needs a tool passes; removing the tool makes the failure name the group and the artifact.

## Not in the slice

Changing what any group stages; adding groups; the dispatcher's batched/per-test decision (DESIGN).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests,kvm-tests -E 'test(<a cgroup-default test needing the tool>)'`. Evidence: PASS with the tool present; a negative run with the tool removed shows the naming failure.

## Acceptance

- [ ] One declaration per group; no duplicate list left for a declared group.
- [ ] Adding a tool to cgroup-default affects only cgroup-default guests.
- [ ] Missing-artifact failure names group and artifact; a declared path absent on the box fails naming group and path before the test body.
- [ ] Pass parity for existing tests of the declared groups.

## Estimate

2 days.
