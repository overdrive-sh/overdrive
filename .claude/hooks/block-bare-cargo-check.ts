#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `cargo check` on macOS when not routed
// through `cargo xtask lima run --`. On Linux the host already matches
// the canonical compile environment, so the hook is a no-op.
//
// Block policy (macOS only):
//   cargo check ...                use `cargo xtask lima run -- cargo check ...`
//   cargo xtask lima run -- ...    allowed (already routed through Lima)
//
// Linux: pass through unconditionally — the host IS the canonical
// compile environment.
//
// Why: typecheck signal must match the canonical Lima compile
// environment. macOS host rustc may resolve `#[cfg(target_os = "linux")]`
// items differently, miss conditional dependencies, and skip build.rs
// steps gated on Linux. A green `cargo check` on macOS without Lima is
// not the same signal as a green check inside Lima — and the next
// Lima-side compile diverges silently.
// See `.claude/rules/development.md` § "Compile-checking".

const CARGO_CHECK = /(?:^|[\s;&|])cargo\s+check\b/;

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

function isBlockedCargoCheck(cmd: string): boolean {
  // Already routed through Lima — allow it through.
  if (/cargo\s+xtask\s+lima\s+run\b/.test(cmd)) return false;
  return segments(cmd).some((seg) => CARGO_CHECK.test(seg));
}

// Linux host: this hook is a no-op. The canonical compile environment
// is already Linux; running `cargo check` directly produces the same
// signal as routing through Lima would.
if (process.platform === "linux") {
  process.exit(0);
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

if (cmd && isBlockedCargoCheck(cmd)) {
  deny(
    "`cargo check` on macOS is blocked by pre-tool hook " +
      "(.claude/hooks/block-bare-cargo-check.ts).\n\n" +
      "Typecheck signal must match the canonical Lima compile " +
      "environment. macOS host rustc resolves " +
      "`#[cfg(target_os = \"linux\")]` items differently, may miss " +
      "conditional dependencies, and skips build.rs steps gated on " +
      "Linux. A green `cargo check` on macOS without Lima is not the " +
      "same signal as a green check inside Lima.\n\n" +
      "Route through Lima instead:\n" +
      "  cargo check [ARGS]\n" +
      "  →  cargo xtask lima run -- cargo check [ARGS]\n\n" +
      "Allowed exceptions:\n" +
      "  cargo xtask lima run -- ...   already routed through Lima\n\n" +
      "On Linux this hook is a no-op — the host IS the canonical " +
      "compile environment.\n\n" +
      "See .claude/rules/development.md § \"Compile-checking\"."
  );
}
