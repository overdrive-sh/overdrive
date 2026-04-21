# Slice 5 — Nondeterminism Traits + CI Lint Gate

**Story**: US-05
**Estimated effort**: ≤ 1 day
**Walking-skeleton member**: yes (backbone row 4)

## Hypothesis

If the lint gate misses a real `Instant::now()` in a core crate, the DST claim is performative — the harness cannot catch a bug routed around an un-intercepted side channel.

## What ships

- `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm` traits in `overdrive-core::traits`
- Real implementations (`SystemClock`, `TcpTransport`, `OsEntropy`; stubs for `EbpfDataplane`, `CloudHypervisorDriver`, `RigLlm` where Phase 2+ will provide real impls)
- Sim implementations (`SimClock`, `SimTransport`, `SimEntropy`, `SimDataplane`, `SimDriver`, `SimLlm`) in `overdrive-sim`
- `cargo xtask dst-lint` entry point that scans every "core" crate for banned APIs
- Banned-API list as a single `BANNED_APIS` constant in `xtask`; every violation message names the file/line/column, the banned symbol, the replacement trait, and a link to `development.md`
- CI step wired to fail on non-zero exit
- xtask self-test covering every banned symbol against a synthetic source file

## Demonstrable end-to-end value

Engineer inserts `Instant::now()` into `overdrive-core` — `cargo xtask dst-lint` fails with a pointed error. Reverts the change — lint gate goes silent. CI blocks a real smuggle PR.

## Carpaccio taste tests

- **Real data**: the lint scans real crates, not mocks.
- **Ships end-to-end**: traits, real impls, sim impls, CLI command, CI wiring — all in the slice.
- **Enforces the §21 claim**: if this slice doesn't ship green, the testing discipline claim is fiction.

## Definition of Done (slice level)

- [ ] Six traits exist with real + sim implementations.
- [ ] `cargo xtask dst-lint` exists.
- [ ] Every banned symbol has a test case.
- [ ] Zero false positives on wiring crates.
- [ ] CI blocks merges on non-zero exit.
