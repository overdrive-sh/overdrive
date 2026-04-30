//! S-CLI-02 + S-CLI-06 — `--detach` / `IsTerminal` auto-detach truth table.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/03-02`
//! step 03-02 acceptance criteria:
//!
//! > Replace the unconditional NDJSON branch from 02-04 with
//! > `let stream = !args.detach && std::io::IsTerminal::is_terminal(&std::io::stdout());`
//! > per architecture.md §6. When `stream` is false (stdout is not a TTY
//! > OR `--detach`), send `Accept: application/json`; otherwise
//! > `Accept: application/x-ndjson`.
//!
//! Truth table per architecture.md §6 + DESIGN [D5]:
//!
//! | `--detach` | TTY?  | Accept                       | Lane      |
//! |------------|-------|------------------------------|-----------|
//! | set        | any   | `application/json`           | Detached  |
//! | unset      | true  | `application/x-ndjson`       | Streaming |
//! | unset      | false | `application/json`           | Detached  |
//!
//! Reference class (per [D5]): `docker run`, `nomad job run`, every
//! Unix-tradition CLI tool. The detection uses `std::io::IsTerminal`
//! (Rust 1.70+, in-stdlib) — no `atty` / `isatty`-via-libc dependency.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*, this file exercises the dispatch decision in-process
//! through the pure `should_stream(detach, is_terminal)` function plus
//! the `StdoutTerminalProbe` trait seam — production wires
//! `RealStdoutTerminal` (which calls
//! `std::io::IsTerminal::is_terminal(&std::io::stdout())` at the bin
//! boundary); Tier 1 wires fakes returning `false` (S-CLI-02) or `true`
//! (S-CLI-06).

use overdrive_cli::commands::job::{StdoutTerminalProbe, should_stream};

// ---------------------------------------------------------------------
// Fakes — the IsTerminal probe seam.
// ---------------------------------------------------------------------

struct FakeNonTty;
impl StdoutTerminalProbe for FakeNonTty {
    fn is_terminal(&self) -> bool {
        false
    }
}

struct FakeTty;
impl StdoutTerminalProbe for FakeTty {
    fn is_terminal(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------
// S-CLI-02 — stdout redirected (non-TTY) without --detach selects JSON
// ---------------------------------------------------------------------

#[test]
fn pipe_redirected_stdout_without_detach_selects_json_lane() {
    // Given: the CLI's stdout is redirected to a file (or the
    // IsTerminal probe returns false), AND --detach is not present.
    let probe = FakeNonTty;
    let detach = false;

    // When: the CLI submit command consults the dispatch decision.
    let stream = should_stream(detach, probe.is_terminal());

    // Then: the dispatch decision is the JSON-ack lane (Detached).
    // The wire-level Accept-header pinning happens at
    // `ApiClient::submit_job` (set to `application/json`); the
    // `should_stream == false` branch causes main.rs to call `submit`
    // (one-shot ack), not `submit_streaming`. The JSON-ack handler is
    // already exercised end-to-end by
    // `tests/integration/job_submit.rs::submit_with_valid_toml_against_in_process_server_returns_submit_output_with_intent_key_and_next_command`,
    // which is the wire witness for the `Accept: application/json`
    // header path.
    assert!(
        !stream,
        "S-CLI-02: non-TTY stdout without --detach must select the JSON-ack lane \
         (`Accept: application/json`); got stream=true",
    );
}

// ---------------------------------------------------------------------
// S-CLI-06 — TTY without --detach selects NDJSON streaming lane
// ---------------------------------------------------------------------

#[test]
fn tty_stdout_without_detach_selects_ndjson_lane() {
    // Given: the CLI's stdout is a TTY (the IsTerminal probe returns
    // true), AND --detach is not present.
    let probe = FakeTty;
    let detach = false;

    // When: the CLI submit command consults the dispatch decision.
    let stream = should_stream(detach, probe.is_terminal());

    // Then: the dispatch decision is the NDJSON streaming lane.
    // The wire-level Accept-header pinning happens at
    // `ApiClient::submit_job_streaming` (set to `application/x-ndjson`);
    // the `should_stream == true` branch causes main.rs to call
    // `submit_streaming` and engage the line-delimited consumer. The
    // NDJSON handler is exercised end-to-end by
    // `tests/integration/streaming_submit_happy_path.rs` (Tier 3
    // Linux-gated).
    assert!(
        stream,
        "S-CLI-06: TTY stdout without --detach must select the NDJSON streaming lane \
         (`Accept: application/x-ndjson`); got stream=false",
    );
}

// ---------------------------------------------------------------------
// `--detach` short-circuit — set, regardless of TTY, picks JSON lane
// ---------------------------------------------------------------------
//
// The truth table's first row: `--detach` set, TTY status irrelevant
// → Accept: application/json (Detached lane). This is the explicit
// operator escape valve from step 03-01 ([D4]). We pin BOTH branches
// of the TTY probe under `detach=true` so a future refactor that
// accidentally inverts the priority (TTY check before --detach) is
// caught.

#[test]
fn detach_with_tty_stdout_still_selects_json_lane() {
    let probe = FakeTty;
    let detach = true;

    let stream = should_stream(detach, probe.is_terminal());

    assert!(
        !stream,
        "S-CLI-02 truth-table row 1: --detach set with TTY stdout must select the \
         JSON-ack lane; --detach takes precedence over IsTerminal. got stream=true",
    );
}

#[test]
fn detach_with_pipe_redirected_stdout_selects_json_lane() {
    let probe = FakeNonTty;
    let detach = true;

    let stream = should_stream(detach, probe.is_terminal());

    assert!(
        !stream,
        "S-CLI-02 truth-table row 3 + --detach: --detach set with non-TTY stdout must \
         select the JSON-ack lane (both inputs already point at Detached). got stream=true",
    );
}
