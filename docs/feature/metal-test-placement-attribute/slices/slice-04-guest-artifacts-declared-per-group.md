# Slice 04 — Guest environments from a closed registry

Story: US-PLACE-04. Job: J-PLAT-007. Depends on: slice 01 (skeleton shows what the per-test path stages today).

## Outcome

The guest groups and their named profiles are one closed registry (21 environments over 8 groups, DDD-5). A test selects an environment by name on the attribute; a name the registry does not hold, or a profile of another group, fails the build with the pinned diagnostic. Variants that were keyed by test name become profiles, so a rename keeps its environment.

## In the slice

- The closed registry with its group and profile names; an omitted profile selects the group's base profile (`native` for `mtls-relay`).
- Build-time rejection of an unknown profile or a profile of another group: ``unknown guest profile `<value>` for group `<group>` ``, pointing at the profile argument.
- The five Quint gateway tests select the `quint` profile of `cgroup-default`.

## Not in the slice

Changing what any group stages; adding groups or profiles beyond the 21; artifact-presence scenarios (checked per migration batch by the parity check of DDD-15).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p overdrive-gateway -E 'test(/^application::quint_gateway_application::/)'` for the five tests declaring `profile = "quint"`. Evidence: PASS in guests staged for the profile; a test declaring a mistyped profile fails the build naming it.

## Acceptance

- [ ] The registry holds exactly the 21 named environments over 8 groups; an omitted profile selects the base profile.
- [ ] An unknown profile and a profile of another group are rejected at build with the pinned diagnostic.
- [ ] A renamed test keeps its profile.
- [ ] Pass parity for existing tests of the declared groups, per migration batch.

## Estimate

2 days.
