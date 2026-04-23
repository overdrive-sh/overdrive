#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `cargo test` invocations, redirecting the
// agent to `cargo nextest run`. `.claude/rules/testing.md` §"Running tests
// — foreground, always" mandates nextest as the project-wide runner;
// `cargo test` is reserved for doctests (which nextest does not execute).
//
// Block policy:
//   cargo test                         reach for `cargo nextest run`
//   cargo test -p <crate>              reach for `cargo nextest run -p <crate>`
//   cargo test --workspace             reach for `cargo nextest run --workspace`
//   cargo test --features ...          reach for `cargo nextest run --features ...`
//   cargo test -- <filter>             reach for `cargo nextest run -E 'test(<filter>)'`
//
// Intentional allow (passes through without verdict):
//   cargo test --doc ...               nextest cannot run doctests; per
//                                       development.md rustdoc examples
//                                       MUST be executed — this is the
//                                       sole legitimate `cargo test` path
//   cargo nextest run ...              not matched
//   cargo mutants ...                  uses --test-tool=nextest internally

// Anchor: start-of-command or after a shell separator (; && || |).
const BOUND = String.raw`(?:^|[\s;&|])`;
const CARGO_TEST = new RegExp(`${BOUND}cargo\\s+test\\b`);

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
 * checked independently. A `cargo test --doc && cargo test` chain
 * must still block the second stage.
 */
function segments(cmd: string): string[] {
  return cmd.split(/[;&|]+/).map((s) => s.trim()).filter(Boolean);
}

/**
 * True iff the segment is a `cargo test` invocation that is NOT
 * `cargo test --doc ...`. Doctests are the one legitimate use per
 * .claude/rules/testing.md — nextest cannot run them.
 */
function isBlockedCargoTest(seg: string): boolean {
  if (!CARGO_TEST.test(seg)) return false;
  // `--doc` may appear anywhere in the argument list.
  if (/\s--doc\b/.test(seg)) return false;
  return true;
}

async function main(): Promise<void> {
  let raw: string;
  try {
    raw = await Bun.stdin.text();
  } catch {
    return; // no input — allow
  }

  let cmd = "";
  try {
    const parsed = JSON.parse(raw) as { tool_input?: { command?: string } };
    cmd = parsed.tool_input?.command ?? "";
  } catch {
    return; // malformed JSON — allow, don't break the tool call
  }
  if (!cmd) return;

  if (segments(cmd).some(isBlockedCargoTest)) {
    deny(
      "`cargo test` is blocked by pre-tool hook. This project uses " +
        "`cargo nextest run` as the test runner — see " +
        "`.claude/rules/testing.md` §\"Running tests — foreground, always\". " +
        "Swap the command:\n" +
        "  cargo test [ARGS]              →  cargo nextest run [ARGS]\n" +
        "  cargo test -p CRATE            →  cargo nextest run -p CRATE\n" +
        "  cargo test -- <filter>         →  cargo nextest run -E 'test(<filter>)'\n" +
        "The only legitimate `cargo test` usage is `cargo test --doc ...` " +
        "(nextest cannot execute doctests)."
    );
  }
}

await main();
