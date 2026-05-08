#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `cargo nextest run` not routed through
// `cargo xtask lima run --`.
//
// Block policy (all platforms):
//   cargo nextest run ...           use `cargo xtask lima run -- cargo nextest run ...`
//   cargo nextest run ... --no-run  allowed (compile-check; no Linux surface needed)
//   cargo xtask lima run -- ...     allowed (already routed through Lima)
//
// Why: all test execution goes through the Lima VM for reproducibility.
// Running nextest directly on the host gives a degraded signal —
// #[cfg(target_os = "linux")] items may compile away, cgroup writes fail,
// and the toolchain may differ from the canonical VM environment.
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

/**
 * True iff the command contains a `cargo nextest run` that must be blocked:
 *   - The full command is NOT already wrapped in `cargo xtask lima run`.
 *   - At least one pipeline segment is `cargo nextest run ...` without `--no-run`.
 */
function isBlockedNextestRun(cmd: string): boolean {
  // Already routed through Lima — allow it through.
  if (/cargo\s+xtask\s+lima\s+run\b/.test(cmd)) return false;
  return segments(cmd).some((seg) => {
    if (!NEXTEST_RUN.test(seg)) return false;
    // --no-run is a compile-check only; no Linux surface is exercised.
    if (/\s--no-run\b/.test(seg)) return false;
    return true;
  });
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

if (cmd && isBlockedNextestRun(cmd)) {
  deny(
    "`cargo nextest run` is blocked by pre-tool hook " +
      "(.claude/hooks/block-bare-nextest.ts).\n\n" +
      "All test execution goes through the Lima VM for reproducibility. " +
      "Running nextest directly on the host gives a degraded signal — " +
      "Linux-gated tests may be silently skipped and the toolchain may " +
      "differ from the canonical VM environment.\n\n" +
      "Route through Lima instead:\n" +
      "  cargo nextest run [ARGS]\n" +
      "  →  cargo xtask lima run -- cargo nextest run [ARGS]\n\n" +
      "Allowed exceptions (no Lima required):\n" +
      "  --no-run                compile-check only; no Linux surface involved\n" +
      "  cargo xtask lima run -- already routed through Lima\n\n" +
      "See .claude/rules/testing.md § \"Running tests — Lima VM\"."
  );
}
