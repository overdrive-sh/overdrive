#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks destructive git commands the agent might run
// without explicit user instruction. Prints a PreToolUse deny verdict JSON
// when a match is found; stays silent (allow) otherwise.
//
// Block policy:
//   git checkout -- <path>         explicit pathspec revert
//   git checkout <file>.<ext>      arg ends in a code/config file extension
//   git restore ...                modern destructive revert
//   git reset --hard               discards uncommitted work
//   git clean -f / -fd / -fx       deletes untracked files
//   git push --force / -f          rewrites remote history
//   git branch -D <name>           force-delete branch
//
// Intentional non-blocks:
//   git checkout <branch>          plain branch switch
//   git checkout feature/foo       slash-separated branch
//   git checkout v1.0.0            tag checkout
//   git switch ...                 safe branch op
//   git status / commit / log      read-only / additive

const CODE_EXTS = [
  "rs", "js", "jsx", "ts", "tsx", "py", "go", "md", "toml", "yaml", "yml",
  "json", "sh", "bash", "zsh", "rb", "java", "kt", "c", "cc", "cpp", "cxx",
  "h", "hpp", "sql", "html", "css", "scss", "vue", "svelte", "lua", "php",
  "pl", "lock",
].join("|");

// Anchor: start-of-command or after a shell separator (; && || |).
const BOUND = String.raw`(?:^|[\s;&|])`;

const RULES: ReadonlyArray<{ pattern: RegExp; label: string }> = [
  { pattern: new RegExp(`${BOUND}git\\s+checkout\\s+--(\\s|$)`),               label: "git checkout -- <path>" },
  { pattern: new RegExp(`${BOUND}git\\s+checkout\\s+\\S+\\.(${CODE_EXTS})(\\s|$|&|;|\\|)`), label: "git checkout <file>" },
  { pattern: new RegExp(`${BOUND}git\\s+restore(\\s|$)`),                      label: "git restore" },
  { pattern: new RegExp(`${BOUND}git\\s+reset\\s+--hard`),                     label: "git reset --hard" },
  { pattern: new RegExp(`${BOUND}git\\s+clean\\s+-[A-Za-z]*f`),                label: "git clean -f" },
  { pattern: new RegExp(`${BOUND}git\\s+push\\s+.*--force`),                    label: "git push --force" },
  { pattern: new RegExp(`${BOUND}git\\s+push\\s+(?:\\S+\\s+)*-f(\\s|$)`),        label: "git push -f" },
  { pattern: new RegExp(`${BOUND}git\\s+branch\\s+-D\\s`),                     label: "git branch -D" },
];

function deny(label: string): void {
  const verdict = {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        `Destructive git operation blocked by pre-tool hook (${label}). ` +
        "This hook guards against discarding uncommitted work without " +
        "explicit user instruction. If the user has just asked you to " +
        "run this, stop and confirm — do not retry until they approve " +
        "via the permission prompt.",
    },
  };
  console.log(JSON.stringify(verdict));
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

  const hit = RULES.find((r) => r.pattern.test(cmd));
  if (hit) deny(hit.label);
}

await main();
