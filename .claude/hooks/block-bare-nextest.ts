#!/usr/bin/env bun
// PreToolUse/Bash hook — governs `cargo nextest run`. Two rules:
//
//   1. `--no-run` is BANNED (any form, even wrapped in Lima). It compiles
//      the test binaries but RUNS NOTHING, so it cannot gate runtime
//      behaviour — a boot-path regression sails through a `--no-run` "gate"
//      undetected. Compile-checking is `cargo check`'s job; running is
//      Lima's.
//   2. A real `cargo nextest run` must be routed through
//      `cargo xtask lima run --` for a reproducible Linux signal.
//
// Block policy (all platforms):
//   cargo nextest run ... --no-run                BANNED — use `cargo check
//                                                 --all-targets` to compile-check,
//                                                 or run it for real under Lima
//   cargo nextest run ...                         use `cargo xtask lima run --
//                                                 cargo nextest run ...`
//   cargo xtask lima run -- cargo nextest run ... allowed (already routed)
//
// Why ban --no-run: it links the test binaries but executes nothing, so a
// startup probe that refuses, a panic before bind, or a runtime-only
// #[cfg(target_os = "linux")] arm passes the "gate" green. This masked a
// real cold-boot CA regression (built-in-ca-operator-composition 02-02)
// once already. A step that wires into run_server / a composition root /
// a boot path MUST actually RUN the fixtures.
// See `.claude/rules/testing.md` § "Running tests — Lima VM".

// Matches `cargo nextest run` as a command or pipeline stage.
const NEXTEST_RUN = /(?:^|[\s;&|])cargo\s+nextest\s+run\b/;

function deny(reason: string): void {
  const verdict = {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: reason,
    },
  };
  console.log(JSON.stringify(verdict));
}

/**
 * Split on shell separators so each pipeline stage is checked
 * independently. Mirrors the pattern in block-cargo-test.ts.
 */
function segments(cmd: string): string[] {
  return cmd.split(/[;&|]+/).map((s) => s.trim()).filter(Boolean);
}

/** True iff any segment is a `cargo nextest run ... --no-run`. */
function isNoRunNextest(cmd: string): boolean {
  return segments(cmd).some(
    (seg) => NEXTEST_RUN.test(seg) && /\s--no-run\b/.test(seg),
  );
}

/**
 * True iff the command contains a `cargo nextest run` not already wrapped
 * in `cargo xtask lima run`. (--no-run is handled separately, above.)
 */
function isBareNextestRun(cmd: string): boolean {
  if (/cargo\s+xtask\s+lima\s+run\b/.test(cmd)) return false;
  return segments(cmd).some((seg) => NEXTEST_RUN.test(seg));
}

let cmd = "";
try {
  const raw = await Bun.stdin.text();
  cmd =
    (JSON.parse(raw) as { tool_input?: { command?: string } }).tool_input
      ?.command ?? "";
} catch {
  // missing stdin or malformed JSON — allow, don't break the tool call
}

if (cmd && isNoRunNextest(cmd)) {
  deny(
    "`cargo nextest run --no-run` is blocked by a pre-tool hook " +
      "(.claude/hooks/block-bare-nextest.ts).\n\n" +
      "`--no-run` COMPILES the test binaries but RUNS NOTHING, so it cannot " +
      "gate runtime behaviour: a boot-path regression — a startup probe that " +
      "refuses, a panic before bind, a runtime-only `#[cfg(target_os = " +
      '"linux")]` arm — passes a `--no-run` “gate” undetected. This ' +
      "masked a real cold-boot CA regression once already.\n\n" +
      "Use the right tool:\n" +
      "  • compile-check  →  cargo xtask lima run -- cargo check --all-targets --features integration-tests\n" +
      "  • RUN the tests  →  cargo xtask lima run -- cargo nextest run [ARGS]\n\n" +
      "A step that wires into run_server / a composition root / a boot path " +
      "MUST run the fixtures, not `--no-run` them.\n" +
      'See .claude/rules/testing.md § "Running tests — Lima VM".',
  );
} else if (cmd && isBareNextestRun(cmd)) {
  deny(
    "`cargo nextest run` is blocked by a pre-tool hook " +
      "(.claude/hooks/block-bare-nextest.ts).\n\n" +
      "All test execution goes through the Lima VM for reproducibility. " +
      "Running nextest directly on the host gives a degraded signal — " +
      "Linux-gated tests may be silently skipped and the toolchain may " +
      "differ from the canonical VM environment.\n\n" +
      "Route through Lima instead:\n" +
      "  cargo nextest run [ARGS]\n" +
      "  →  cargo xtask lima run -- cargo nextest run [ARGS]\n\n" +
      "To compile-check without running, use `cargo check --all-targets` " +
      "(also Lima-routed on macOS) — `cargo nextest run --no-run` is itself " +
      "blocked, because it compiles but runs nothing.\n\n" +
      'See .claude/rules/testing.md § "Running tests — Lima VM".',
  );
}
