# fix-overdrive-config-path-doubled — Feature Evolution

**Feature ID**: fix-overdrive-config-path-doubled
**Type**: Bug fix (via `/nw-bugfix` pipeline)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-24
**Commit**: `0c61d3d` — `fix(cli): write operator config to ~/.overdrive/config (single segment)`
**Status**: Delivered (all TDD phases PASS; single step, single commit)

---

## What shipped

The operator CLI now writes and reads its TOML trust triple at the
canonical `~/.overdrive/config` (single `.overdrive` segment) on the
`$HOME`-fallback branch. The read-side and write-side now call a single
shared helper — `overdrive_cli::commands::cluster::default_operator_config_path()` —
eliminating the structural possibility of the two sites drifting again.

Two coupled bugs were resolved in one commit:

- **Write-path (user-visible):** `cluster init` previously produced
  `$HOME/.overdrive/.overdrive/config` (doubled segment). `resolve_config_dir`
  returned `$HOME/.overdrive` (already suffixed), and `write_trust_triple`
  joined `.overdrive/config` on top.
- **Read-path (latent):** `default_config_path` computed the correct
  single-segment path, but its doc comment documented the doubled form
  as *intended* behaviour — the lie in the docs was the drift surface
  that would have re-broken the read side the next time someone touched
  it. Every subsequent subcommand (`cluster status`, `job submit`,
  `alloc status`, `node list`) therefore read from a location that did
  not match where `init` had actually written, so any production session
  that ran `init` successfully would have failed every subsequent
  command with a missing-config error.

## Business context

Maps directly to the CLI-auth memory (`project_cli_auth.md`): the
Talos-shape mTLS flow depends on `cluster init` producing a trust
triple that every subsequent CLI invocation can read back. The doubled
path made that handshake silently broken in production (the one path
no integration test exercised), while passing every CI gate because
all integration tests pass an explicit `--config-dir` that skipped the
buggy `$HOME` branch. ADR-0010 / ADR-0014 / ADR-0019 and whitepaper §8
already specified `~/.overdrive/config` as canonical — this was a
conformance fix, not a design change.

## Key decisions

- **Fix 3 (structural extraction) is in scope, not deferred.**
  Without it, Fix 1 + Fix 2 solve the immediate bug but leave the
  class of bug (two sites computing the canonical path independently)
  live. User-confirmed at the `/nw-bugfix` Phase 2 review gate.
- **`serial_test` crate with `#[serial(env)]` is the env-mutation
  serialisation mechanism.** Dev-dep only, no test-binary split, no
  crate-local ad-hoc mutex. Lowest coupling of the options considered;
  matches the `.claude/rules/testing.md` "Tests that mutate
  process-global state" section. Added `serial_test = "3"` under
  `[workspace.dependencies]` `# --- Testing ---`; consuming crate
  pulls via `serial_test.workspace = true`.
- **Single-commit TDD cycle (RED + GREEN bundled).** Per
  `testing.md` §"RED scaffolds" which allows "bundle with the GREEN
  commit if the crafter judges a single commit cleaner," and per the
  `feedback_single_cut_greenfield_migrations` memory (no
  deprecations, no grace periods). Reviewer sees the RED → GREEN
  coupling in one diff; normal pre-commit hooks apply; no
  `--no-verify` exception taken.
- **No new ADR.** ADR-0010, ADR-0014, and ADR-0019 already specify
  the canonical path. This was a conformance fix, not a design
  change.
- **No auto-migration for operators on the old doubled path.**
  Phase 1 CA is ephemeral per ADR-0010 §R4; operators `mv` the file
  or re-run `cluster init`. Documented in the commit body.
- **Env mutations wrapped in explicit `unsafe { }` blocks** with
  `// SAFETY: #[serial(env)]` comments, per Rust 2024 + workspace-wide
  `unsafe_op_in_unsafe_fn = deny`. Save-restore via an RAII guard so
  a panicking test does not leak env state into subsequent tests in
  the same binary.

## Regression invariants locked in

Two integration tests in
`crates/overdrive-cli/tests/integration/cluster_init_serve.rs`, both
`#[serial_test::serial(env)]`:

1. **`resolve_config_dir_home_fallback_writes_at_canonical_path`** —
   primary guard. `cluster::init` with `config_dir: None` writes to
   `$HOME/.overdrive/config` and explicitly does NOT write to
   `$HOME/.overdrive/.overdrive/config` (negative assertion on the
   doubled form is the regression guard).
2. **`default_config_path_matches_init_write_location_on_home_fallback`** —
   structural invariant. The path the read side computes equals the
   path the write side creates. This is the invariant the defect
   violated; it is now enforced via a shared helper called from both
   sites.

## Steps completed

Single phase, single step, single commit.

| Step ID | Phase | Status | Notes |
|---|---|---|---|
| 01-01 | PREPARE | PASS | Verified bug sites on HEAD match RCA |
| 01-01 | RED_ACCEPTANCE | PASS | Test fails on pre-fix code with canonical-vs-doubled assertion |
| 01-01 | RED_UNIT | SKIPPED | NOT_APPLICABLE: acceptance regression covers structural invariant; unit test on shared helper would be a tautology |
| 01-01 | GREEN | PASS | Fix 1 + Fix 2 + Fix 3 applied; test passes; no pre-existing tests regress |
| 01-01 | COMMIT | PASS | Single conventional commit with `Step-ID: 01-01` trailer |

## Files touched

| File | Change |
|---|---|
| `crates/overdrive-cli/src/commands/cluster.rs` | Fix 1 + Fix 3 (new `default_operator_config_path`, fixed HOME-branch return) |
| `crates/overdrive-cli/src/main.rs` | Fix 2 (thin delegate to shared helper + corrected doc comment) |
| `crates/overdrive-cli/tests/integration/cluster_init_serve.rs` | Two new `#[serial(env)]` regression tests |
| `crates/overdrive-cli/Cargo.toml` | `serial_test.workspace = true` under `[dev-dependencies]` |
| `Cargo.toml` (workspace) | `serial_test = "3"` under `[workspace.dependencies]` testing block |

Diff footprint: +226 / −12 across 6 files (including `Cargo.lock`).

## Lessons learned

- **The drift surface was the bug, not the path.** Fix 1 and Fix 2
  alone would have passed the acceptance test while leaving two
  sibling functions computing the canonical path independently. The
  next touch to either site would have re-broken it. The
  "`PathBuf` carrying implicit `is-this-already-suffixed?` semantics"
  shape generalises to: whenever the same invariant is enforced in
  two call sites via string concatenation, extract a shared
  constructor — the type system cannot encode the invariant, so the
  code structure has to.
- **Integration tests that always pass `config_dir: Some(tmp.path())`
  silently skip the branch that matters in production.** The HOME
  fallback is the production path; the explicit-override path is a
  test convenience. New CLI tests should exercise the production
  branch by default, with `serial_test` serialising the env
  mutation.
- **`default_config_path` being private in `main.rs` plus
  `crates/overdrive-cli/CLAUDE.md`'s forbid-subprocess-tests rule
  made the buggy branch structurally unreachable from any approved
  test surface.** Fix 3 made it reachable by promoting the canonical
  computation into the library crate where it can be called from
  integration tests. The "private in `main.rs` so it cannot be
  tested" shape is itself a drift-surface signal.
- **Doc comments lying about intended behaviour is a named
  failure mode.** The `main.rs:141` comment documented the doubled
  form; the code at line 148 did the right thing; the commit that
  originally diverged them went unreviewed because the comment
  looked self-consistent. When code and doc disagree, trust neither
  — reconcile against the ADR.

## Notes on discarded workspace artifacts

Nothing lasting lived outside the evolution doc and the RCA:

- **`deliver/rca.md`** — authoritative RCA from `/nw-bugfix` Phase 1,
  user-approved at Phase 2. Its key findings (problem, root cause,
  three fixes, regression-test design) are folded into this
  evolution doc. The RCA file remains in the preserved workspace
  (`docs/feature/fix-overdrive-config-path-doubled/deliver/rca.md`)
  for historical reference; it was the source specification for the
  commit, not a lasting design document.
- `deliver/execution-log.json`, `deliver/roadmap.json`,
  `deliver/.develop-progress.json` — process scaffolding. Audit
  trail captured above; step plan superseded by the single landed
  commit.

No `design/`, `distill/`, `discuss/`, or ADR artifacts were produced
(single-step conformance bug fix). Nothing migrated to permanent
directories; the ADRs this fix conforms against already exist.

## Related

- **ADR-0010** (`~/.overdrive/config` canonical path — control plane trust triple)
- **ADR-0014** (operator config layout)
- **ADR-0019** (operator CLI config path specification)
- **Whitepaper §8** (Identity and mTLS — Operator Identity and CLI Authentication)
- **Memory**: `project_cli_auth.md` (Talos-shape mTLS, operator SPIFFE IDs, 8h TTL, Corrosion-gossiped revocation)
