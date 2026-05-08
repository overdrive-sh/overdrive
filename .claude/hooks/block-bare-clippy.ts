#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `cargo clippy` not routed through
// `cargo xtask lima run --`.
//
// Block policy (all platforms):
//   cargo clippy ...                use `cargo xtask lima run -- cargo clippy ...`
//   cargo xtask lima run -- ...     allowed (already routed through Lima)
//
// Why: all cargo commands that compile or analyse code go through the
// Lima VM for reproducibility. Running clippy directly on the host may
// use a different toolchain version or system headers.

const CLIPPY_RUN = /(?:^|[\s;&|])cargo\s+clippy\b/;

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

function segments(cmd: string): string[] {
  return cmd.split(/[;&|]+/).map((s) => s.trim()).filter(Boolean);
}

function isBlockedClippyRun(cmd: string): boolean {
  if (/cargo\s+xtask\s+lima\s+run\b/.test(cmd)) return false;
  // `cargo xtask bpf-clippy` self-routes through Lima — allow it.
  if (/cargo\s+xtask\s+bpf-clippy\b/.test(cmd)) return false;
  return segments(cmd).some((seg) => CLIPPY_RUN.test(seg));
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

if (cmd && isBlockedClippyRun(cmd)) {
  deny(
    "`cargo clippy` is blocked by pre-tool hook " +
      "(.claude/hooks/block-bare-clippy.ts).\n\n" +
      "All cargo commands go through the Lima VM for reproducibility.\n\n" +
      "Route through Lima instead:\n" +
      "  cargo clippy [ARGS]\n" +
      "  →  cargo xtask lima run --no-sudo -- cargo clippy [ARGS]\n\n" +
      "Allowed exceptions:\n" +
      "  cargo xtask lima run --      already routed through Lima\n" +
      "  cargo xtask bpf-clippy       self-routes through Lima"
  );
}
