//! Self-test for the `ban-infra-subprocess` lint — the FINAL slice of
//! `subprocess-free-veth-provisioner` (GH #233, ADR-0085 D8 / DDD-10).
//!
//! RED SCAFFOLDS (DISTILL, not yet implemented). Slice 05 lands a new
//! `xtask::dst_lint::scan_source_infra_subprocess`-shaped scanner clause
//! that bans `Command::new("<tool>")` for the seven named infra CLIs
//! (`ip`, `nft`, `ethtool`, `sysctl`, `tc`, `bpftool`, `iptables`) in
//! production `src/**` of runtime crates. This file is its in-process
//! self-test, mirroring `dst_lint_self_test.rs` (`BANNED_APIS` closure) and
//! `dst_lint_live_literal.rs` (scoped literal ban + marker suppression),
//! per ADR-0085 D8:
//!
//!   - **Scope:** crates whose `crate_class ∈ {core, adapter-host}`, MINUS
//!     an explicit exclusion of `overdrive-testing` (dev-dep-only Tier-3
//!     fixture that legitimately shells `ip netns add`). `binary`
//!     (`overdrive-cli`, `xtask`) and `adapter-sim` (`overdrive-sim`) are
//!     out of scope by class.
//!   - **What it bans:** the seven NAMED string-literal args to
//!     `Command::new`, NOT `Command::new` generically. Variable-binary
//!     spawns (`Command::new(var)`) and `run_ip()`-style indirection are
//!     out of the literal-only guarantee (mirrors dst-lint's literal
//!     scope — the structural backstop is that the swap leaves no
//!     infra-CLI helper in either file, plus code review).
//!   - **Marker:** `// subprocess-ok: <reason>` on the use-site line or the
//!     line immediately above (mirrors `// dst-lint: hashmap-ok`). After
//!     this feature there are no sanctioned production uses.
//!   - **Exempt:** `#[cfg(test)]` items and `bin/` tooling (mirrors
//!     dst-lint); the sanctioned variable-binary spawns (Cloud Hypervisor
//!     `vmm.rs` `Command::new(&wrapper[0])`, the workload drivers'
//!     `Command::new(spec.driver.command())`, guest PID-1) are safe BY
//!     CONSTRUCTION (not one of the seven literals) and MUST NOT be flagged.
//!
//! CRAFTER NOTE (slice 05): the exact scanner entry-point name is the
//! crafter's to define per the dst-lint mirror pattern (ADR-0085 does not
//! pin it — do NOT invent a public name here). When the clause lands,
//! replace each `panic!("… RED scaffold …")` body with a call to that
//! scanner over the SYNTHETIC SOURCE quoted in each test's doc comment and
//! the assertions described there, then drop the `#[should_panic]`
//! attribute. `Violation` carries `{ file, line, column, banned_path,
//! replacement_trait, kind }` — assert `banned_path` names the flagged
//! `"<tool>"` literal and `line`/`column` are 1-based positive, exactly as
//! `dst_lint_self_test::violation_reports_one_based_line_and_column_of_head_segment`
//! does.
//!
//! Gated behind the `integration-tests` feature — same convention as
//! `dst_lint_self_test.rs` / `dst_lint_live_literal.rs`. In-process scanner
//! self-test (no real infra / no Lima needed — the scanner is pure syn AST).

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
// Skip/scaffold diagnostics go to the test log.
#![allow(clippy::print_stderr)]

// S-LINT-01 — a named infra-CLI string literal in a scoped production src
// path is FLAGGED.
//
// SYNTHETIC SOURCE (scoped path e.g. `crates/overdrive-worker/src/mtls_intercept.rs`):
//     pub fn ensure() { let _ = std::process::Command::new("ip").arg("rule").output(); }
// EXPECTED: exactly one violation whose `banned_path` names the `"ip"`
// literal; `line`/`column` 1-based positive. Also assert the same shape for
// `"nft"`, `"ethtool"`, `"sysctl"` (the four this feature actually swaps) —
// loop over the seven banned tool names as `dst_lint_self_test` loops over
// BANNED_APIS.
#[test]
#[should_panic(expected = "RED scaffold")]
fn s_lint_01_named_infra_cli_literal_in_scoped_src_is_flagged() {
    panic!(
        "Not yet implemented -- RED scaffold (S-LINT-01 / ban-infra-subprocess flags Command::new(\"ip\") in scoped prod src)"
    );
}

// S-LINT-02 — the `// subprocess-ok: <reason>` marker (use-site line OR the
// line immediately above) SUPPRESSES the violation.
//
// SYNTHETIC SOURCE:
//     pub fn ensure() {
//         // subprocess-ok: sanctioned CH confinement wrapper
//         let _ = std::process::Command::new("ip").arg("rule").output();
//     }
// AND the trailing-comment placement:
//     let _ = std::process::Command::new("ip").output(); // subprocess-ok: reason
// EXPECTED: zero violations for BOTH placements (mirrors
// `.claude/rules/development.md` § Ordered-collection `// dst-lint:
// hashmap-ok` above-line + trailing forms).
#[test]
#[should_panic(expected = "RED scaffold")]
fn s_lint_02_subprocess_ok_marker_suppresses_violation() {
    panic!(
        "Not yet implemented -- RED scaffold (S-LINT-02 / `// subprocess-ok:` marker suppresses the infra-CLI literal ban)"
    );
}

// S-LINT-03 — `#[cfg(test)]` items and `bin/` tooling paths are EXEMPT
// (mirrors dst-lint), so the seven literals inside a test module or a
// `bin/*.rs` tool are NOT flagged.
//
// SYNTHETIC SOURCE (cfg-test):
//     #[cfg(test)]
//     mod tests { fn t() { let _ = std::process::Command::new("ip").output(); } }
// SYNTHETIC PATH (bin): `crates/<c>/bin/some_tool.rs` with the same literal.
// EXPECTED: zero violations for both the `#[cfg(test)]` item and the `bin/`
// path (the test harness legitimately shells `ip` to construct kernel
// fixtures — cf. `veth_provision_idempotent.rs`).
#[test]
#[should_panic(expected = "RED scaffold")]
fn s_lint_03_cfg_test_items_and_bin_tooling_are_exempt() {
    panic!(
        "Not yet implemented -- RED scaffold (S-LINT-03 / cfg(test) items + bin/ tooling exempt from the infra-CLI literal ban)"
    );
}

// S-LINT-04 — `overdrive-testing` (dev-dep-only Tier-3 fixture,
// `crate_class = "adapter-host"`) is EXCLUDED by scope even though it is an
// adapter-host crate — it legitimately shells `ip netns add` / `ethtool -K`
// (`crates/overdrive-testing/src/netns.rs`) and is never linked into a
// production binary (ADR-0085 D8; the same "own only what ships" discipline
// dst-lint uses to scan only `core`).
//
// SYNTHETIC PATH: `crates/overdrive-testing/src/netns.rs` carrying
// `Command::new("ip")` with NO marker.
// EXPECTED: zero violations (excluded by the crate-scope allowlist, not by a
// marker). Assert the SAME literal at a NON-excluded adapter-host path (e.g.
// `crates/overdrive-control-plane/src/veth_provisioner.rs`) IS flagged, so
// the exclusion is proven to be path-scoped and non-vacuous.
#[test]
#[should_panic(expected = "RED scaffold")]
fn s_lint_04_overdrive_testing_is_excluded_by_scope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-LINT-04 / overdrive-testing excluded by scope; a non-excluded adapter-host path IS flagged)"
    );
}

// S-LINT-05 — THE regression guard (the "flips green immediately" gate,
// ADR-0085 D8): after slices 01–04 swap both files, the scanner reports
// ZERO infra-CLI-literal violations across the real in-scope production
// tree (`crate_class ∈ {core, adapter-host}` minus `overdrive-testing`).
// Mirrors `dst_lint_live_literal::s_06_03_dst_lint_passes_on_migrated_codebase`
// (walk the real files, assert zero).
//
// EXPECTED: walk `veth_provisioner.rs` + `mtls_intercept.rs` (and every
// other in-scope `src/**`), assert zero violations. This is RED until slice
// 04 lands the last `nft` swap — it is the door-lock the whole feature
// closes.
#[test]
#[should_panic(expected = "RED scaffold")]
fn s_lint_05_scanner_passes_on_the_migrated_tree() {
    panic!(
        "Not yet implemented -- RED scaffold (S-LINT-05 / zero infra-CLI-literal violations across the migrated in-scope tree)"
    );
}
