#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks direct `cargo mutants` invocations,
// redirecting the agent to `cargo xtask mutants`.
// `.claude/rules/testing.md` §"Mutation testing (cargo-mutants)" → "Usage"
// mandates the xtask wrapper: it materialises the diff file
// cargo-mutants expects, pins `--test-tool=nextest` to match the project
// runner, writes `target/xtask/mutants-summary.json`, and implements the
// 80% / 60% kill-rate gate. Calling `cargo mutants` directly skips all
// of that — different flag shape (`--in-diff <FILE>` vs
// `--diff <BASE_REF>`), no gate, no structured summary, no nextest
// pinning.
//
// Block policy:
//   cargo mutants                         reach for `cargo xtask mutants --diff origin/main`
//   cargo mutants --in-diff <FILE>        reach for `cargo xtask mutants --diff origin/main`
//   cargo mutants --workspace             reach for `cargo xtask mutants --workspace`
//   cargo mutants --help                  reach for `cargo xtask mutants --help`
//
// Intentional allow (passes through without verdict):
//   cargo xtask mutants ...               the wrapper — this IS the sanctioned path
//   cargo install cargo-mutants           tool installation (binary name is hyphenated)
//   cargo install --locked cargo-mutants  same

// Anchor: start-of-command or after a shell separator (; && || |).
// `\s+` between `cargo` and `mutants` means `cargo-mutants` (one token,
// as in `cargo install cargo-mutants`) does not match, and
// `cargo xtask mutants` does not match (something sits between the two).
const BOUND = String.raw`(?:^|[\s;&|])`;
const CARGO_MUTANTS = new RegExp(`${BOUND}cargo\\s+mutants\\b`);

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
 * checked independently. A `cargo xtask mutants --diff origin/main &&
 * cargo mutants` chain must still block the second stage.
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

if (cmd && segments(cmd).some((seg) => CARGO_MUTANTS.test(seg))) {
  deny(
    "`cargo mutants` is blocked by pre-tool hook. This project runs " +
      "mutation testing through the `cargo xtask mutants` wrapper — see " +
      "`.claude/rules/testing.md` §\"Mutation testing (cargo-mutants)\" → " +
      "\"Usage\". The wrapper materialises the diff file cargo-mutants " +
      "expects, pins `--test-tool=nextest`, writes " +
      "`target/xtask/mutants-summary.json`, and gates on kill rate. " +
      "Calling `cargo mutants` directly skips all of that.\n" +
      "\n" +
      "Swap the command:\n" +
      "  cargo mutants                     →  cargo xtask mutants --diff origin/main\n" +
      "  cargo mutants --in-diff <FILE>    →  cargo xtask mutants --diff origin/main\n" +
      "  cargo mutants --workspace         →  cargo xtask mutants --workspace\n" +
      "  cargo mutants --help              →  cargo xtask mutants --help\n" +
      "  cargo mutants --file <PATH>       →  cargo xtask mutants --diff origin/main --file <PATH>\n" +
      "  cargo mutants --package <CRATE>   →  cargo xtask mutants --diff origin/main --package <CRATE>\n" +
      "  cargo mutants -- --features <LIST> →  cargo xtask mutants --diff origin/main --features <LIST>\n" +
      "\n" +
      "The wrapper exposes all of cargo-mutants' scope flags as " +
      "pass-throughs (--file, --package, --features), plus the gate, " +
      "plus auto-enables `integration-tests` when --package names a " +
      "crate that declares it — mutation runs without that feature " +
      "silently understate kill rate because the acceptance tests " +
      "don't compile.\n" +
      "\n" +
      "Flag note: the xtask wrapper takes `--diff <BASE_REF>` (a git " +
      "ref — e.g. `origin/main`). `--in-diff <FILE>` is the bare " +
      "`cargo mutants` flag and takes a file path; the wrapper handles " +
      "diff materialisation for you.\n" +
      "\n" +
      "Invocation shape: mutation is the exception to the foreground-only " +
      "test rule — use `run_in_background: true` and let it finish. Do " +
      "NOT `pkill -f \"cargo mutants\"` when it seems slow; that leaves " +
      "mutated source on disk."
  );
}
