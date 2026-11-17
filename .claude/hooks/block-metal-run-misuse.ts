#!/usr/bin/env bun
// PreToolUse/Bash hook — governs how `cargo xtask metal run` is invoked.
//
// A metal run executes under a durable owner on the box. Killing the local
// command (a `timeout` wrapper, the Bash tool's own limit, a user stop) does
// NOT stop the remote run: it keeps holding the checkout's lease, and every
// retry from the same checkout then fails with `checkout_timeout`. The
// recovery is `cargo xtask metal collect <RUN_ID>`, never a re-run.
//
// Blocked shapes (each was seen burning an hour of an agent's run):
//
//   timeout N cargo xtask metal run ...     a local kill orphans the remote
//                                           run; the Bash tool already caps
//                                           the call
//   sleep N; cargo xtask metal run ...      waiting does not release a lease;
//                                           collect the run holding it
//   cargo xtask metal run ... > file        hides the output and the
//   cargo xtask metal run ... | head/tail   OVERDRIVE_METAL_RUN_ID needed to
//                                           collect the run later
//
// `cargo xtask metal collect`, `pull`, `sync` and `shell` are not affected.
// See .claude/rules/testing.md § "Run records, collection and diagnostics".

import { parse, type ParseEntry } from "shell-quote";

function deny(reason: string): void {
  console.log(
    JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason: reason,
      },
    }),
  );
}

type Command = { words: string[]; ops: string[]; next: string };

/** Operators that end one command and start the next. */
const SEPARATORS = new Set([";", "&&", "||", "&", "|", "|&", ";;"]);
/** Redirections of a command's output to a file. `>&` followed by a digit
 *  is an fd duplication (`2>&1`) and is handled separately. */
const FILE_REDIRECTS = new Set([">", ">>", ">|", "&>", "&>>"]);
const TRUNCATING = new Set([
  "head", "tail", "grep", "egrep", "fgrep", "rg", "awk", "sed", "cut", "less", "more",
]);

/** Parse with shell-quote into commands. Quoted text stays one word (a
 *  nextest `-E 'a | b'` filter is not a pipe); `$VAR` is kept verbatim. */
function commands(raw: string): Command[] {
  const tokens: ParseEntry[] = parse(raw, (key) => `$${key}`);
  const out: Command[] = [];
  let cur: Command = { words: [], ops: [], next: "" };
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (typeof t === "string") {
      cur.words.push(t);
    } else if ("op" in t && SEPARATORS.has(t.op)) {
      cur.next = t.op;
      out.push(cur);
      cur = { words: [], ops: [], next: "" };
    } else if ("op" in t) {
      // `N>&M` duplicates a descriptor; `>& file` redirects to a file.
      const target = tokens[i + 1];
      if (t.op === ">&" && typeof target === "string" && /^\d+$/.test(target)) {
        i++;
        continue;
      }
      cur.ops.push(t.op === ">&" ? ">" : t.op);
    }
  }
  out.push(cur);
  return out.filter((c) => c.words.length > 0 || c.ops.length > 0);
}

/** Index of `cargo xtask metal run` within a command's words, or -1. */
function metalRunAt(words: string[]): number {
  for (let i = 0; i + 3 < words.length; i++) {
    if (words[i] === "cargo" && words[i + 1] === "xtask" &&
      words[i + 2] === "metal" && words[i + 3] === "run") return i;
  }
  return -1;
}

/** Returns the reason the command misuses `metal run`, or null. */
export function misuse(raw: string): string | null {
  let cmds: Command[];
  try {
    cmds = commands(raw);
  } catch {
    return null; // unparseable — let the shell report it
  }
  for (let i = 0; i < cmds.length; i++) {
    const c = cmds[i];
    const at = metalRunAt(c.words);
    if (at < 0) continue;
    if (c.words.slice(0, at).some((w) => w === "timeout" || w === "gtimeout")) {
      return "timeout";
    }
    if (cmds.slice(0, i).some((p) => p.words[0] === "sleep")) {
      return "sleep";
    }
    if (c.ops.some((op) => FILE_REDIRECTS.has(op))) {
      return "redirect";
    }
    if ((c.next === "|" || c.next === "|&") &&
      TRUNCATING.has(cmds[i + 1]?.words[0] ?? "")) {
      return "pipe";
    }
  }
  return null;
}

const RECOVERY =
  "A metal run keeps running on the box after the local command dies, and it " +
  "holds this checkout's lease until it finishes. To get its result:\n" +
  "  cargo xtask metal collect <RUN_ID>\n" +
  "(the id is printed as OVERDRIVE_METAL_RUN_ID=… when the run starts; a " +
  "`checkout_timeout` diagnostic names the run holding the lease as holder=…).\n\n" +
  "Run `cargo xtask metal run -- …` in the foreground with its full output, " +
  "and ask the Bash tool for its maximum timeout if the run is long. Scope " +
  "tests with `-E` rather than running a whole crate's integration suite.\n" +
  'See .claude/rules/testing.md § "Run records, collection and diagnostics".';

const WHY: Record<string, string> = {
  timeout:
    "`timeout … cargo xtask metal run` is blocked. Killing the local command " +
    "does not stop the remote run; it orphans it.",
  sleep:
    "`sleep …; cargo xtask metal run` is blocked. Waiting does not release a " +
    "lease; collect the run that holds it.",
  redirect:
    "Redirecting `cargo xtask metal run` output to a file is blocked. It hides " +
    "the output and the run id you need to collect the run.",
  pipe:
    "Piping `cargo xtask metal run` into head/tail/grep is blocked. It " +
    "truncates the output and drops the run id you need to collect the run.",
};

if (import.meta.main) {
  let cmd = "";
  try {
    const raw = await Bun.stdin.text();
    cmd =
      (JSON.parse(raw) as { tool_input?: { command?: string } }).tool_input
        ?.command ?? "";
  } catch {
    // missing stdin or malformed JSON — allow, don't break the tool call
  }
  const kind = cmd ? misuse(cmd) : null;
  if (kind) {
    deny(
      `${WHY[kind]}\n(.claude/hooks/block-metal-run-misuse.ts)\n\n${RECOVERY}`,
    );
  }
}
