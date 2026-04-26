//! @clap-config — argv parsing exercised via `Cli::try_parse_from`,
//! no subprocess.
//!
//! Regression invariant for `fix-remove-phase-1-cluster-init`:
//!
//! **Phase 1 has exactly one cert-minting site, and it is `serve`.**
//!
//! `cluster init` minted a fresh ephemeral CA and wrote a trust triple
//! to `<config_dir>/.overdrive/config`. So did `serve`. Both targeting
//! the same default config dir, `serve` ran second and overwrote the
//! cert `init` produced — see
//! `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.
//!
//! ADR-0010 §R5 (no cert persistence on disk in the server process)
//! makes Phase 1 structurally incapable of honouring an init-produced
//! cert, so `cluster init` is a Phase 5 verb shipped early. Step 01-02
//! deletes it; this test pins the deletion against future re-introduction
//! before Phase 5 (#81).
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, the test exercises the
//! binary-wrapper argv surface in-process — the Exception scope carved
//! out for "argv parsing for the binary wrapper itself", NOT a
//! subprocess smoke test.

use clap::Parser as _;
use clap::error::ErrorKind;
use overdrive_cli::cli::Cli;

#[test]
fn cluster_init_subcommand_is_not_registered() {
    let result = Cli::try_parse_from(["overdrive", "cluster", "init"]);

    let err = result.expect_err(
        "Phase 1 must have exactly one cert-minting site (`serve`); \
         `cluster init` is a Phase 5 verb (#81) and MUST NOT be a \
         registered clap subcommand. See \
         docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md \
         and ADR-0010 §R5.",
    );

    // Clap rejects an unknown subcommand under the `cluster` parent
    // with `ErrorKind::InvalidSubcommand` (or `UnknownArgument` if it
    // walks the token as a free argument). Both are acceptable
    // signals; what is NOT acceptable is `DisplayHelp` (which would
    // mean clap accepted the parse and produced a help screen) or
    // `MissingRequiredArgument` (which would mean clap recognised
    // `init` as a real subcommand and is asking for its arguments).
    assert!(
        matches!(err.kind(), ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument),
        "clap must classify `cluster init` as InvalidSubcommand or \
         UnknownArgument, not {:?} — anything else means the \
         subcommand is still registered",
        err.kind(),
    );

    let rendered = err.render().to_string();
    assert!(
        rendered.contains("init"),
        "clap error text must name the offending subcommand `init`; got: {rendered}",
    );
}

#[test]
fn legitimate_cluster_subcommands_still_parse_successfully() {
    // Positive control: deleting `cluster init` must not regress the
    // sibling `cluster status` subcommand. Without this, the negative
    // assertion above is not a reliable signal — clap might be
    // rejecting `cluster init` for an unrelated reason (e.g. the
    // entire `Cluster` variant got accidentally removed).
    let parsed = Cli::try_parse_from(["overdrive", "cluster", "status"])
        .expect("positive control: `cluster status` must remain a real subcommand");

    // Smoke check — we just want to prove parse succeeded.
    let _ = parsed.command;
}
