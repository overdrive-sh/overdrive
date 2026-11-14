# ADR-0294 — Guest groups and environments are a closed registry of named profiles

## Status

**Proposed — awaiting user approval (DDD-5 in the feature delta).**

## Context

The guest group set is listed in the wrapper and in the dispatcher, and about a
dozen hand-kept tables keyed by test name (plus one literal test name) choose
staged files, loaded modules, setup scripts, memory and vCPUs per test. A
renamed test silently drops its variant. The inventory (V-3) reduces them to 20
environments over 8 groups.

## Decision

The guest kit holds one closed registry: the eight groups and, per group, named
profiles (20 environments). The attribute selects `guest = "<group>"` and an
optional `profile = "<profile>"` (omitted means the group's base profile),
validated by constant evaluation against the registry, so a typo is a build
error and the macro crate and the wrapper hold no group list. Each profile has
one recipe, the single place a copy list is declared, and one exec bound
(ADR-0292). A shared guest's scope includes the profile. A Quint model file a
test replays is declared on the test (`quint_model`), not in the registry,
because it is orthogonal to the environment.

## Alternatives

- **Keep test-name-keyed tables as library data**: a renamed test silently loses
  its variant; rejected.
- **Make each variant a new group**: changes the group vocabulary and
  multiplies CID slots; rejected.
- **A `quint` profile per spec, or one profile staging the whole `specs/quint`
  tree**: multiplies environments, or stages every subsystem's specs and
  evidence for five tests; rejected.

## Consequences

Adding a group or profile is one registry entry. The registry cannot live in the
macro crate (a proc-macro crate cannot export types and `overdrive-testing`
depends on it), which is why validation is by constant evaluation; the exact
build-error text is pending probe V-11.

## Links

- [Decision DDD-5 and profile names](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-5--group-registry-profiles-per-group-artifacts-questions-1-and-4-f-1)
- [V-3 inventory](../../feature/metal-test-placement-attribute/spike/findings-validation.md)
