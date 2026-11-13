# SPIKE Decisions — metal-test-placement-attribute

## Assumption tested
- A test can carry its own Metal guest placement: the host half boots a guest
  of the declared group and re-runs the same test binary inside it, and the
  wrapper recognises the declaration without a `test-placement.json` entry while
  still refusing unplaced tests.

## Probe verdict
- No probe. The user chose to skip it (2026-11-13): `fwd_guest_suite` already
  ran named tests inside a guest with `--exact`, so a probe would re-prove a
  known fact and would have had to copy code out of a binary module.

## Promotion decision
- PROMOTE, user decision 2026-11-13: build the walking skeleton directly as the
  first committed slice (contract D4 in `feature-delta.md`).

## Walking skeleton
- Driving adapter: `cargo xtask metal run -- cargo nextest run …`
- Acceptance evidence: the migrated test
  `integration::cluster_and_node_commands::cluster_status_with_no_server_returns_transport_error`
  (`overdrive-cli`) passing on an ordinary Metal run through the attribute,
  with no table entry (see `findings.md`).
- Commit: `db6fb7589`
- Demo command: see `findings.md`.

## Design implications
- **Batching (O4):** one guest per test costs about 0.45 s staging plus 2.4 s
  boot (two samples). DESIGN decides whether that is acceptable at 771 guest
  placements under nextest parallelism, or whether a module-level batch
  declaration is needed.
- **Marker mechanism (O1):** the macro writes a record into the
  `overdrive_test_placement` ELF section; the wrapper reads it before the table
  lookup. DESIGN either ratifies this or replaces it; when the table is
  retired, the wrapper's refusal of unmarked tests must move to the lint.
- **Group names:** listed in the wrapper, the dispatcher plan and the
  dispatcher `main`. DESIGN picks one source the macro can also validate.
- **Guest-kit module:** the dispatcher moved whole into
  `overdrive-testing/src/guest_suite.rs`; DESIGN should split the per-test path
  from the batched dispatcher.
- **Public surface added beyond D4, needing ratification:** `RunInGuestError`,
  the `overdrive_testing::overdrive_test` re-export, and workspace dependencies
  `quote` and `proc-macro2`.
- **Ambiguity:** a marker overrides a table entry for the same test; there is
  no check for both.
- **`inside_guest()`** is an environment variable and can be set on the host.

## Constraints discovered
- `#[used]` alone does not keep the section record through linking; the test
  body must reference it.
- Off Metal the migrated test fails rather than running on the host; Lima and
  CI lanes running the whole `overdrive-cli` integration binary are affected.
- The Metal box ran out of disk during the skeleton runs; more timing samples
  need space freed first.
