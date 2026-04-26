#!/usr/bin/env bun
// PostToolUse/Bash hook — scans test-runner output for failure signals
// the agent might otherwise read past.
//
// Motivation: `cargo nextest | tail -N` eats the non-zero exit code
// because bash pipelines propagate the last command's status and
// `pipefail` isn't set per tool call. Combined with `--no-fail-fast`,
// a single FAIL line gets buried under dozens of PASSes and the agent
// happily labels the next step a "GREEN phase."
//
// This hook reads the command's captured output directly and emits a
// PostToolUse `block` verdict with a concrete list of matched failure
// markers, forcing the agent to address the failure before moving on.

// Only fire when the command actually invoked a test runner. Keeps the
// hook quiet for unrelated Bash (grep, cargo build, etc.). Doctest
// runs via `cargo test --doc` are deliberately included — they can
// also silently fail under pipes.
const TEST_COMMAND =
  /\bcargo\s+(?:nextest\s+run|test\b|xtask\s+(?:dst|bpf-unit|integration-test|mutants|verifier-regress|xdp-perf))\b/;

const FAILURE_PATTERNS: ReadonlyArray<{ pattern: RegExp; label: string }> = [
  // nextest per-test FAIL line: "    FAIL [   0.492s] crate::..."
  { pattern: /^\s*FAIL\s*\[/m, label: "nextest `FAIL [` per-test line" },
  // nextest terminal error
  { pattern: /\btest run failed\b/, label: "`test run failed`" },
  // classic `cargo test` summary
  { pattern: /\btest result:\s*FAILED\b/, label: "`test result: FAILED`" },
  // cargo's wrapper error
  { pattern: /\berror:\s*test failed\b/, label: "`error: test failed`" },
  // nextest summary with non-zero failed count:
  //   "86 tests run: 85 passed, 1 failed, 0 skipped"
  {
    pattern: /\b\d+\s+tests?\s+run:\s+\d+\s+passed(?:\s*\([^)]*\))?,\s+([1-9]\d*)\s+failed/,
    label: "nextest summary reports N failed",
  },
  // `cargo test --doc` summary with non-zero failed count:
  //   "test result: ok. 12 passed; 0 failed; ..."
  {
    pattern: /\btest result:\s+\w+\.\s+\d+\s+passed;\s+([1-9]\d*)\s+failed/,
    label: "doctest summary reports N failed",
  },
];

type Payload = {
  tool_input?: { command?: string };
  tool_response?: {
    stdout?: string;
    stderr?: string;
    output?: string;
    interrupted?: boolean;
  };
};

let payload: Payload = {};
try {
  payload = JSON.parse(await Bun.stdin.text()) as Payload;
} catch {
  process.exit(0); // malformed input — stay silent, don't break the tool
}

const cmd = payload.tool_input?.command ?? "";
if (!cmd || !TEST_COMMAND.test(cmd)) process.exit(0);

const resp = payload.tool_response ?? {};
const out = [resp.stdout, resp.stderr, resp.output].filter(Boolean).join("\n");
if (!out) process.exit(0);

const hits = FAILURE_PATTERNS
  .filter((p) => p.pattern.test(out))
  .map((p) => p.label);

if (hits.length === 0) process.exit(0);

// RED scaffold carve-out. testing.md §"RED scaffolds and intentionally-
// failing commits" marks unimplemented branches with
// `panic!("Not yet implemented -- RED scaffold")` or
// `todo!("RED scaffold: ...")`. These panics MUST be allowed through —
// Outside-In TDD depends on the bar being red until the implementation
// lands, and the paired tests are precisely what will validate the
// GREEN-next-commit loop.
//
// Split the output at each `thread '...' panicked at` marker so we can
// classify panics individually. A block with "RED scaffold" in its
// message is intentional; blocks without it are real failures. If
// every panic is a RED scaffold, stay silent. If any panic is not,
// block and surface the mix so the agent can tell the two apart.
const panicSplit = out.split(/(?=thread\s+'[^']*'\s+panicked\s+at)/);
const panicBlocks = panicSplit.filter((p) =>
  /^thread\s+'[^']*'\s+panicked\s+at/.test(p),
);
const redPanics = panicBlocks.filter((b) => /RED\s+scaffold/i.test(b));
const nonRedPanics = panicBlocks.length - redPanics.length;

if (panicBlocks.length > 0 && nonRedPanics === 0) {
  // Every failing panic carries a RED scaffold marker — intentional.
  process.exit(0);
}

const redNote =
  redPanics.length > 0
    ? ` ${redPanics.length} of ${panicBlocks.length} panic(s) are RED scaffolds (matched "RED scaffold") and are intentional — the remaining ${nonRedPanics} are NOT and need attention.`
    : "";

const verdict = {
  decision: "block",
  reason:
    `Test failure detected in the previous command's output — matched: ${hits.join("; ")}.${redNote} ` +
    `This hook fires when \`cargo nextest\` / \`cargo test\` / \`cargo xtask {dst,bpf-unit,integration-test,mutants,…}\` ` +
    `output contains a failure signal the agent might otherwise read past. ` +
    `Common causes: \`| tail -N\` eats the non-zero exit code (no pipefail); \`--no-fail-fast\` ` +
    `buries the FAIL line in a wall of PASSes. ` +
    `RED scaffolds (panic messages containing "RED scaffold", per testing.md) are allowed through automatically — if you see this message, at least one failure is NOT a RED scaffold. ` +
    `Do NOT label the next step a GREEN phase and do NOT move on. ` +
    `Re-run the failing test without the tail pipe, read the full failure, and address it before continuing.`,
};
console.log(JSON.stringify(verdict));
