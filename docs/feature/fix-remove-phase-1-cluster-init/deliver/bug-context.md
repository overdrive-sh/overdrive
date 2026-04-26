# Bug context — remove `cluster init` from Phase 1

**Source RCA**: `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`
**GitHub tracking issue (Phase 5 reintroduction)**: #81

## Defect

`overdrive cluster init` and `overdrive serve` both mint a fresh ephemeral
CA and write a trust triple to `<config_dir>/.overdrive/config`. With
both targeting the production default `$HOME/.overdrive/`, `serve` runs
second and overwrites the cert `init` produced. ADR-0010 §R5 (no cert
persistence on disk in the server process) makes Phase 1 structurally
incapable of honouring an init-produced cert.

`cluster init` is therefore a Phase 5 verb shipped early. The RCA
falsifies the obvious "make serve consume an existing triple" /
"split mint from endpoint-record" / "Talos two-file split" remediations
under Phase 1 constraints (R5 forbids the persistent CA private key
each presupposes). Only deletion-in-Phase-1 + Phase-5-reintroduction
passes backwards-chain validation.

## Regression invariant (test FIRST, RED)

**Phase 1 has exactly one cert-minting site, and it is `serve`.**

Concrete shape — pick the cleanest expression of the invariant:

- **Clap-parse test** in `crates/overdrive-cli/tests/integration/`
  asserting `Cli::try_parse_from(["overdrive", "cluster", "init"])`
  fails with an unknown-subcommand error.
- A second assertion (compile-fail-style trybuild OR a runtime
  symbol-existence test, your call) that
  `overdrive_cli::commands::cluster::init` is no longer a public
  symbol.

The crafter chooses the cleanest shape; the invariant is what matters.

## Deletion scope (single commit; no deprecation path)

| Path | Action |
|---|---|
| `crates/overdrive-cli/src/commands/cluster.rs` | Delete `init` fn, `InitArgs`, `InitOutput`. **KEEP** `status`, `default_operator_config_dir`, `default_operator_config_path` — used by `serve` and `job submit`. |
| `crates/overdrive-cli/src/cli.rs` | Remove `Init { force }` variant from `ClusterCommand`. |
| `crates/overdrive-cli/src/main.rs` | Remove `Command::Cluster(ClusterCommand::Init { force })` match arm. |
| `crates/overdrive-cli/tests/integration/cluster_init_serve.rs` | Delete file entirely. |
| `crates/overdrive-cli/tests/integration/walking_skeleton.rs` | Remove Phase 0 (`cluster::init` call + dual-tempdir façade). Test starts at `serve`. Keep all other phases. Reviewer comment on lines 96-176 is the implicit prompt for this simplification. |
| `crates/overdrive-cli/tests/integration.rs` | Remove `mod cluster_init_serve;` line if present. |
| Other callers | Grep before commit. |

## Out of scope for this fix

- ADR-0010 §R1/§R4 amendments → architect agent, separate dispatch after this lands.
- `docs/feature/phase-1-control-plane-core/` DELIVER artefact updates (roadmap.json, distill walking-skeleton, design wave-decisions) → architect agent.
- `docs/scenarios/phase-1-control-plane-core/` test scenarios → architect agent if SSOT, otherwise crafter may update inline.
- README.md → check for `cluster init` references; if present, leave a TODO and flag; do **NOT** edit inline.
- `.github/roadmap-issues.md` → leave alone; #81 already tracks Phase 5 reintroduction.

## Project rules to honour

- `.claude/rules/development.md` and `.claude/rules/testing.md`.
- `crates/overdrive-cli/CLAUDE.md` — direct-handler test pattern; no subprocess.
- Memory `feedback_single_cut_greenfield_migrations.md` — single commit, no deprecation, no flag.
- Memory `feedback_no_unrequested_ci_lefthook.md` — gate-then-stop; do NOT auto-wire CI.
- Memory `feedback_no_rename_stubs.md` — reject any "leave a stub" suggestion.
- Memory `feedback_no_git_stash.md` — never `git stash` to scope commits.

## Commit shape

Single commit at the end. Body must include `Refs #81` so the commit
shows on the issue timeline. Do **NOT** post a comment on #81 — the
user will do that manually.

## Branch

Stay on `marcus-sa/phase-1-control-plane-core`. Do not branch.
