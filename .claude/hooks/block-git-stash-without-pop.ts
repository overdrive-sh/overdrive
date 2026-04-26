#!/usr/bin/env bun
// PreToolUse/Bash hook — blocks `git stash` (creating forms) unless the
// same command also restores the stash via `git stash pop` or
// `git stash apply`.
//
// Rationale: agents occasionally stash work, run a check, and forget the
// restore — silently losing uncommitted changes. Requiring the restore
// to live in the same command prevents the split.
//
// Block policy:
//   git stash                     bare — no matching pop/apply
//   git stash push ...            without matching pop/apply
//   git stash save ...            without matching pop/apply
//   git stash -u / -k / -a / -m   without matching pop/apply
//
// Intentional non-blocks:
//   git stash pop / apply / list / show / drop / clear / branch
//   git stash -u ... ; git stash pop   (restore present)

// Anchor: start-of-command or after a shell separator (; && || |).
const BOUND = String.raw`(?:^|[\s;&|])`;

// Non-creating subcommands — harmless on their own.
const NON_CREATING =
  /^(?:pop|apply|list|show|drop|clear|branch|create|store)\b/;

function isStashCreate(cmd: string): boolean {
  const re = new RegExp(`${BOUND}git\\s+stash(?:\\s+(\\S+))?`, "g");
  let m: RegExpExecArray | null;
  while ((m = re.exec(cmd)) !== null) {
    const next = m[1] ?? "";
    if (!next) return true;                   // bare `git stash`
    if (NON_CREATING.test(next)) continue;    // pop / apply / list / etc.
    return true;                              // push, save, -u, -k, path, ...
  }
  return false;
}

function hasRestore(cmd: string): boolean {
  const re = new RegExp(`${BOUND}git\\s+stash\\s+(?:pop|apply)\\b`);
  return re.test(cmd);
}

function deny(): void {
  const verdict = {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        "`git stash` blocked by pre-tool hook — the same command must " +
        "also run `git stash pop` (or `git stash apply`) so uncommitted " +
        "changes cannot be lost if the session ends before the restore " +
        "runs. Use `;` before the pop (not `&&`) so the restore runs even " +
        "when the work in between fails — e.g. " +
        "`git stash -u && <work> ; git stash pop`. " +
        "If you genuinely need to stash without restoring in the same " +
        "command, ask the user first.",
    },
  };
  console.log(JSON.stringify(verdict));
}

let cmd = "";
try {
  const raw = await Bun.stdin.text();
  cmd = (JSON.parse(raw) as { tool_input?: { command?: string } }).tool_input?.command ?? "";
} catch {
  // missing stdin or malformed JSON — allow, don't break the tool call
}

if (cmd && isStashCreate(cmd) && !hasRestore(cmd)) {
  deny();
}
