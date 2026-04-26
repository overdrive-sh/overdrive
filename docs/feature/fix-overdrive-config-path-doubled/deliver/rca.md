# RCA — Overdrive operator CLI config written to doubled path

**Feature ID**: `fix-overdrive-config-path-doubled`
**Type**: Bug fix (regression test + fix via `/nw-bugfix`)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-24

## Problem

The Overdrive operator CLI writes its TOML trust triple to
`/Users/marcus/.overdrive/.overdrive/config` (doubled `.overdrive` segment).
The canonical path per ADR-0010 / ADR-0014 / ADR-0019 and whitepaper §8 is
`~/.overdrive/config` (single segment).

Two coupled symptoms:

- **Write-path (user-visible):** `cluster init` produces the doubled path
  on disk.
- **Read-path (latent):** Every subsequent subcommand (`cluster status`,
  `job submit`, `alloc status`, `node list`) computes a *third* path
  (`$HOME/.overdrive/config`) that disagrees with both the canonical
  specification AND the location `init` actually wrote — so production
  sessions that ran `init` successfully will fail every subsequent
  command because the config file "doesn't exist" at the read path.

## Root cause

Two bugs, one structural cause: `PathBuf` carries implicit "is this
already `.overdrive/`-suffixed or not?" semantics that the type system
doesn't enforce, and the two code paths arrived at incompatible
conventions.

- **Bug A — write path.** `crates/overdrive-cli/src/commands/cluster.rs:174`
  — `resolve_config_dir` returns `$HOME/.overdrive` on the HOME branch
  (already suffixed), then `write_trust_triple` joins `.overdrive/config`
  on top → doubled segment. The `--config-dir` override and
  `$OVERDRIVE_CONFIG_DIR` branches return unsuffixed, which is why every
  integration test passes — they only exercise the explicit-override
  branch.
- **Bug B — read path.** `crates/overdrive-cli/src/main.rs:143–151` —
  `default_config_path` on the HOME branch computes `$HOME/.overdrive/config`
  (single segment). The doc comment at line 141 literally documents
  `$HOME/.overdrive/.overdrive/config` as intended — the bug is encoded
  as "expected behaviour."

## Why tests missed it

- All `cluster init` integration tests pass `Some(tmp.path())` as
  `config_dir`, hitting the explicit-override branch and skipping the
  buggy HOME fallback entirely.
- `default_config_path` is private in `main.rs`;
  `crates/overdrive-cli/CLAUDE.md` forbids subprocess tests — so the
  branch that matters in production is unreachable from any approved
  test surface.

## Fixes (all in scope for this bugfix)

### Fix 1 — `resolve_config_dir` (cluster.rs:174)

Make `resolve_config_dir` always return a base directory;
`write_trust_triple` owns the `.overdrive` suffix.

```rust
// BEFORE (line 174)
Ok(PathBuf::from(home).join(".overdrive"))

// AFTER
Ok(PathBuf::from(home))
```

Also update the doc comment on line 157 to state that the returned path
is a base directory (not the `.overdrive/` directory itself).

### Fix 2 — `default_config_path` (main.rs:140–151)

Rewrite so `$OVERDRIVE_CONFIG_DIR` and `$HOME` are both treated as base
directories; append `.overdrive/config` exactly once. Fix the lying doc
comment.

```rust
/// Default config path per ADR-0010 / ADR-0019: `~/.overdrive/config`.
/// Resolves base dir from `$OVERDRIVE_CONFIG_DIR` first, then `$HOME`,
/// then the current directory as a last-resort fallback. The `.overdrive`
/// segment and `config` filename are always appended by this function —
/// callers pass bare base-dir env-var values. The CLI binary resolves
/// this once; library tests always pass an explicit path.
fn default_config_path() -> std::path::PathBuf {
    let base = std::env::var_os("OVERDRIVE_CONFIG_DIR")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join(".overdrive").join("config")
}
```

### Fix 3 — Collapse drift surface (structural)

Promote the canonical-path computation into a single shared function in
the `overdrive-cli` library so both reader (`main.rs`) and writer
(`cluster.rs`) call it. Concretely:

- Add `pub fn default_operator_config_path() -> PathBuf` to
  `overdrive_cli::commands::cluster` (or a new `overdrive_cli::config`
  module — crafter's call based on what reads cleanly).
- Replace the body of `main.rs::default_config_path` with a call to it.
- Replace the HOME-branch of `resolve_config_dir` with a call to its
  base-dir equivalent (factor the env-var resolution so the write path
  gets the same `$OVERDRIVE_CONFIG_DIR` → `$HOME` → `.` fallback chain
  the read path uses).

This removes the structural possibility of the two sites drifting
again. Without Fix 3, the class of bug recurs the next time one site is
touched.

## Regression tests

Both tests live in `crates/overdrive-cli/tests/integration/cluster_init_serve.rs`
and both are `#[serial_test::serial(env)]` to cooperate with other
env-mutating tests under nextest's parallel runner.

### Test 1 — `resolve_config_dir_home_fallback_writes_at_canonical_path`

Invariant: on the HOME fallback branch, `cluster::init` writes the
trust triple to `$HOME/.overdrive/config` and **not** to
`$HOME/.overdrive/.overdrive/config`.

Shape:

1. Create tempdir. Save prior `$HOME` / `$OVERDRIVE_CONFIG_DIR`. Set
   `$HOME` to tempdir; unset `$OVERDRIVE_CONFIG_DIR`. Env mutations
   require `unsafe { }` on rustc 1.80+.
2. Invoke `cluster::init` with `config_dir: None`.
3. Assert file exists at `tmp.path().join(".overdrive").join("config")`.
4. Assert file does NOT exist at
   `tmp.path().join(".overdrive").join(".overdrive").join("config")`
   (explicit negative assertion — this is the regression guard).
5. Assert returned `InitOutput::config_path` equals the canonical path.
6. Restore env.

### Test 2 — `default_config_path_matches_init_write_location_on_home_fallback`

Structural invariant: the path the read side computes (`main`'s
default) equals the path the write side creates. This is the invariant
the defect violated.

Implementation requires Fix 3 — once `default_operator_config_path`
is `pub` in the library, both the test and `main.rs` call it. The test
then asserts that a HOME-fallback `cluster::init` writes to exactly the
path that function returns.

## Files touched

| File | Change |
|---|---|
| `crates/overdrive-cli/src/commands/cluster.rs` | Fix 1 + Fix 3 (library-side shared fn + doc comment) |
| `crates/overdrive-cli/src/main.rs` | Fix 2 (rewrite `default_config_path` to call the shared fn) |
| `crates/overdrive-cli/tests/integration/cluster_init_serve.rs` | Two new `#[serial(env)]` regression tests |
| `crates/overdrive-cli/Cargo.toml` | `serial_test` as workspace dev-dep |
| `Cargo.toml` (workspace) | `serial_test` workspace dep declaration |

## Risk

**Low.** 1-line + ~10-line edits against functions whose contract is
already specified in ADR-0010 and ADR-0019. Existing integration tests
continue to pass (they take the unaffected override branch). No public
API changes. Only operational side effect: operators with an existing
broken-path file need a one-line `mv ~/.overdrive/.overdrive/config
~/.overdrive/config` or a re-run of `cluster init`. Acceptable in
Phase 1 per ADR-0010 §R4 (CA is ephemeral; re-init is documented as
normal).

## Roadmap hint for the crafter

Two RED → GREEN steps, in this order:

1. **RED: regression test 1.** Write `Test 1` in
   `cluster_init_serve.rs`. Add `serial_test` dev-dep. Run the test —
   it must FAIL against current code (bug manifests; doubled path
   created). Commit the failing test with `git commit --no-verify` and
   call out the intentional RED state per `.claude/rules/testing.md`
   — or bundle with the GREEN commit if the crafter judges a single
   commit cleaner. Either is fine; document the choice.
2. **GREEN: Fix 1 + Fix 2 + Fix 3 + Test 2.** Apply all three fixes
   plus the second regression test. Run all tests. Both regression
   tests pass, no other tests regress. Commit:
   `fix(cli): write operator config to ~/.overdrive/config (single segment)`.

No new ADR required — ADR-0010 and ADR-0019 already specify the
correct behaviour; this is a conformance fix to existing spec.
