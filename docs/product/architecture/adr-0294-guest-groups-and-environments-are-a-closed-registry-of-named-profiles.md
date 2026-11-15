# ADR-0294 — Guest groups and environments are a closed registry of named profiles

## Status

**Accepted — approved by the user, 2026-11-14 (DDD-5 in the feature delta). The `mtls-relay` `fib-hit` profile (22 environments) was approved by the user, 2026-11-15.**

## Context

The guest group set is listed in the wrapper and in the dispatcher, and about a
dozen hand-kept tables keyed by test name (plus one literal test name) choose
staged files, loaded modules, setup scripts, memory and vCPUs per test. A
renamed test silently drops its variant. The inventory (V-3) reduces them to 20
environments over 8 groups; two further variants (the Quint-replaying gateway
tests, and the three XDP `fib-hit` tests whose veth and route setup was selected
by test name) are named profiles too.

## Decision

The guest kit holds one closed registry: the eight groups and, per group, named
profiles (22 environments: the 20 of the inventory plus `quint` and the
`mtls-relay` profile `fib-hit`). The attribute selects `guest = "<group>"` and an
optional `profile = "<profile>"` (omitted means the group's base profile),
validated by constant evaluation against the registry, so a typo is a build
error and the macro crate and the wrapper hold no group list. Each profile has
one recipe, the single place a copy list is declared, and one exec bound
(ADR-0292). A shared guest's scope includes the profile. The five gateway
tests that replay a Quint specification select the `quint` profile of
`cgroup-default`, which stages
the Quint runtime and the `specs/quint` tree (3.9 MB measured, of which the
`.qnt` files are 856 KB).

## Alternatives

- **Keep test-name-keyed tables as library data**: a renamed test silently loses
  its variant; rejected.
- **Make each variant a new group**: changes the group vocabulary and
  multiplies CID slots; rejected.
- **A model-file argument on each test, or a profile per spec**: a second
  declaration surface, or an environment per spec, to avoid staging 3.9 MB;
  rejected.

## Consequences

Adding a group or profile is one registry entry. The registry cannot live in the
macro crate (a proc-macro crate cannot export types and `overdrive-testing`
depends on it), which is why validation is by constant evaluation. The
build-error text names the offending value (V-11).

## Links

- [Decision DDD-5 and profile names](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-5--group-registry-profiles-per-group-artifacts-questions-1-and-4-f-1)
- [V-3 inventory](../../feature/metal-test-placement-attribute/spike/findings-validation.md)
