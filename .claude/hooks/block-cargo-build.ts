#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `cargo build` invocations, redirecting
// the agent to `cargo check`. `.claude/rules/development.md`
// §"Compile-checking" mandates `cargo check` for the iterative
// typecheck-and-diagnose loop; `cargo build` adds codegen + linking
// cost that the agent almost never actually needs.
//
// Block policy:
//   cargo build                    reach for `cargo check`
//   cargo build -p <crate>         reach for `cargo check -p <crate>`
//   cargo build --workspace        reach for `cargo check --workspace`
//   cargo build --all-targets      reach for `cargo check --all-targets`
//   cargo build --features ...     reach for `cargo check --features ...`
//   cargo build --release          reach for `cargo check --release`
//
// Intentional allow (passes through without verdict):
//   cargo check ...                the sanctioned path — not matched
//   cargo xtask ...                xtask subcommands may invoke
//                                    `cargo build` internally as part of
//                                    their compilation pipeline; the
//                                    agent's call is `cargo xtask ...`,
//                                    which does not match.
//   cargo install ...              tool installation, not compilation
//   cargo clippy ...               lints, runs under `cargo check`
//   cargo nextest run ...          nextest drives its own compile
//   cargo test --doc ...           doctest path (allowed by block-cargo-test)

// Anchor: start-of-command or after a shell separator (; && || |).
// `\s+` between `cargo` and `build` means `cargo-build` (one token, if
// it ever existed) would not match, and `cargo xtask build` would not
// match (something sits between the two).
const BOUND = String.raw`(?:^|[\s;&|])`;
const CARGO_BUILD = new RegExp(`${BOUND}cargo\\s+build\\b`);

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
 * Split the command on shell separators so each pipeline stage is
 * checked independently. A `cargo check && cargo build` chain must
 * still block the second stage.
 */
function segments(cmd: string): string[] {
  return cmd.split(/[;&|]+/).map((s) => s.trim()).filter(Boolean);
}

let cmd = "";
try {
  const raw = await Bun.stdin.text();
  cmd = (JSON.parse(raw) as { tool_input?: { command?: string } }).tool_input?.command ?? "";
} catch {
  // missing stdin or malformed JSON — allow, don't break the tool call
}

if (cmd && segments(cmd).some((seg) => CARGO_BUILD.test(seg))) {
  deny(
    "`cargo build` is blocked by pre-tool hook. For the iterative " +
      "typecheck-and-diagnose loop this project uses `cargo check` — " +
      "see `.claude/rules/development.md` §\"Compile-checking\". " +
      "`cargo check` skips codegen and linking; it catches every " +
      "`rustc` diagnostic `cargo build` would.\n" +
      "\n" +
      "Swap the command:\n" +
      "  cargo build                   →  cargo check\n" +
      "  cargo build -p CRATE          →  cargo check -p CRATE\n" +
      "  cargo build --workspace       →  cargo check --workspace\n" +
      "  cargo build --all-targets     →  cargo check --all-targets\n" +
      "  cargo build --features X      →  cargo check --features X\n" +
      "  cargo build --release         →  cargo check --release\n" +
      "\n" +
      "If you genuinely need a binary (real execution target, Tier 3 " +
      "VM artifact, xtask output), reach for the `cargo xtask ...` " +
      "wrapper that produces it — `cargo xtask integration-test vm`, " +
      "`cargo xtask xdp-perf`, etc. Those invocations are not matched " +
      "by this hook."
  );
}
