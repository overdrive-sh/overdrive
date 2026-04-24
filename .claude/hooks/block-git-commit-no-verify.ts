#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `git commit --no-verify`. The user has
// repeatedly flagged that reaching for `--no-verify` to push past a
// failing pre-commit hook (clippy drift, formatter disagreement, lint
// regression) is a shortcut the agent should not take on its own. The
// correct response to a hook failure is to fix the underlying issue.
//
// There is exactly one sanctioned exception in this repo:
// `.claude/rules/testing.md` §"RED scaffolds and intentionally-failing
// commits" — committing a deliberately-RED scaffold where a neutral
// stub would be worse than an acknowledged red bar. The hook still
// blocks that case; the user can approve via the permission prompt
// when it genuinely applies.
//
// Block policy:
//   git commit --no-verify            any form, any flag order
//   git commit -m ... --no-verify
//   git commit --no-verify -m ...
//
// Intentional non-blocks:
//   git commit -m "..."               normal commit — hooks run
//   git commit -n                     NOT blocked; see note below
//   git push --no-verify              separate concern, not matched
//   echo "--no-verify"                not a git commit
//
// Why not `-n`: `git commit -n` is equivalent to `--no-verify`, but the
// short flag composes with other letters (`-am`, `-ne`, etc.) and
// appears literally inside quoted commit messages often enough that a
// regex match produces false positives. The agent overwhelmingly
// reaches for the long form when deliberately skipping hooks; matching
// `--no-verify` covers the actual attack surface without the false
// positives.

// Anchor: start-of-command or after a shell separator (; && || |).
const BOUND = String.raw`(?:^|[\s;&|])`;
const GIT_COMMIT = new RegExp(`${BOUND}git\\s+commit\\b`);
const NO_VERIFY = /--no-verify\b/;

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
 * checked independently. A `cargo check && git commit --no-verify`
 * chain must still block the second stage.
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

if (cmd && segments(cmd).some((seg) => GIT_COMMIT.test(seg) && NO_VERIFY.test(seg))) {
  deny(
    "`git commit --no-verify` is blocked by pre-tool hook. Skipping " +
      "pre-commit hooks to push past a failing lint, clippy, formatter, " +
      "or test is the shortcut this guard exists to stop — the correct " +
      "response to a hook failure is to fix the underlying issue.\n" +
      "\n" +
      "If the hook is reporting real drift (clippy, rustfmt, etc.), " +
      "fix it in this commit. That is almost always cheaper than the " +
      "follow-up cleanup commit the `--no-verify` shortcut implies.\n" +
      "\n" +
      "The only sanctioned exception is intentionally-RED scaffold " +
      "commits per `.claude/rules/testing.md` §\"RED scaffolds and " +
      "intentionally-failing commits\" — committing a deliberately " +
      "failing test before the GREEN implementation lands. If that is " +
      "genuinely the case here, stop and tell the user so they can " +
      "approve via the permission prompt. Do not retry without " +
      "explicit approval."
  );
}
