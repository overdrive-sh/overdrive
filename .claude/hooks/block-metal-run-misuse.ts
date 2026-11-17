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

const METAL_RUN = /cargo\s+xtask\s+metal\s+run\b/;

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

/** Split into command segments on `;`, `&&`, `||` and `|`, keeping the
 *  separator that follows each segment so a pipe can be told apart. */
function segments(cmd: string): { text: string; next: string }[] {
  const out: { text: string; next: string }[] = [];
  const re = /(\|\||&&|;|\||&|\n)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(cmd)) !== null) {
    out.push({ text: cmd.slice(last, m.index).trim(), next: m[1] });
    last = m.index + m[1].length;
  }
  out.push({ text: cmd.slice(last).trim(), next: "" });
  return out.filter((s) => s.text.length > 0);
}

/** Returns the reason the command misuses `metal run`, or null. */
export function misuse(raw: string): string | null {
  if (!METAL_RUN.test(raw)) return null;
  // Blank out quoted strings (a nextest `-E 'a | b'` filter is not a pipe)
  // and fd duplications (`2>&1` is not a redirect to a file, and its `&`
  // is not a separator).
  const cmd = raw
    .replace(/'[^']*'/g, "''")
    .replace(/"(?:[^"\\]|\\.)*"/g, '""')
    .replace(/\d?>&\d/g, " ")
    .replace(/&>>?/g, " >");
  const segs = segments(cmd);
  for (let i = 0; i < segs.length; i++) {
    const seg = segs[i];
    if (!METAL_RUN.test(seg.text)) continue;
    // Words before `cargo xtask metal run` in the same segment.
    const prefix = seg.text.slice(0, seg.text.search(METAL_RUN));
    if (/(?:^|\s)(?:g?timeout|gtimeout)\s/.test(prefix + " ")) {
      return "timeout";
    }
    // An earlier segment of the same command that sleeps.
    if (segs.slice(0, i).some((s) => /^sleep\s+\d/.test(s.text))) {
      return "sleep";
    }
    // Output redirected to a file (stderr-to-stdout `2>&1` alone is fine).
    const afterRun = seg.text.slice(seg.text.search(METAL_RUN));
    if (/(?:^|\s)(?:\d?>>?|&>)\s*\S/.test(afterRun)) {
      return "redirect";
    }
    // Piped into something that truncates or filters.
    if (seg.next === "|") {
      const consumer = segs[i + 1]?.text ?? "";
      if (/^(?:head|tail|grep|rg|awk|sed|cut|less|more)\b/.test(consumer)) {
        return "pipe";
      }
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
