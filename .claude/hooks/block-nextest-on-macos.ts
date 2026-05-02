#!/usr/bin/env bun
// PreToolUse/Bash hook — on macOS, blocks `cargo nextest run` not routed
// through `cargo xtask lima run --`.
//
// Block policy (macOS only):
//   cargo nextest run ...           use `cargo xtask lima run -- cargo nextest run ...`
//   cargo nextest run ... --no-run  allowed (compile-check; no Linux surface needed)
//   cargo xtask lima run -- ...     allowed (already routed through Lima)
//
// No-op on Linux (and any non-darwin platform).
//
// Why: the Linux test surface (#[cfg(target_os = "linux")], cgroup writes,
// real driver processes, eBPF attachment) is unreachable on macOS. Running
// nextest directly on macOS gives a green signal from a degraded envelope
// — #[cfg(target_os = "linux")] items compile away and `cargo nextest run`
// skips them silently. The Lima VM is the canonical inner-loop path.
// See `.claude/rules/testing.md` § "Running tests on macOS — Lima VM".

const IS_MACOS = process.platform === "darwin";

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
 *   - We are on macOS.
 *   - The full command is NOT already wrapped in `cargo xtask lima run`.
 *   - At least one pipeline segment is `cargo nextest run ...` without `--no-run`.
 */
function isBlockedNextestRun(cmd: string): boolean {
  if (!IS_MACOS) return false;
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
    "`cargo nextest run` is blocked on macOS by pre-tool hook " +
      "(.claude/hooks/block-nextest-on-macos.ts).\n\n" +
      "The Linux test surface (#[cfg(target_os = \"linux\")], cgroup writes, " +
      "real driver processes, eBPF attachment) is unreachable on macOS. " +
      "Running nextest directly gives a green signal from a degraded envelope — " +
      "Linux-gated tests are silently skipped.\n\n" +
      "Route through Lima instead:\n" +
      "  cargo nextest run [ARGS]\n" +
      "  →  cargo xtask lima run -- cargo nextest run [ARGS]\n\n" +
      "Allowed exceptions (no Lima required):\n" +
      "  --no-run                compile-check only; no Linux surface involved\n" +
      "  cargo xtask lima run -- already routed through Lima\n\n" +
      "See .claude/rules/testing.md § \"Running tests on macOS — Lima VM\"."
  );
}
