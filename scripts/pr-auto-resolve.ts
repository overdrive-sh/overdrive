#!/usr/bin/env bun
/**
 * PR Auto-Resolve — Deterministic orchestration with Claude Agent SDK, Greptile edition.
 *
 * TypeScript handles: PR creation, Greptile review polling, thread counting, loop control.
 * Claude handles: reading code, understanding Greptile review comments, classifying work,
 *                 inline fixes, and dispatching specialized nwave agents for deeper work.
 *
 * Non-trivial review items are NOT appended to a roadmap. Instead, Claude classifies each
 * unresolved thread and dispatches one of two specialized agents:
 *   - nw-software-crafter   — for implementation work (TDD, refactors, fixes, coverage)
 *   - nw-solution-architect — for architectural changes (API boundaries, tech choices,
 *                             component restructuring)
 *
 * Usage: bun scripts/pr-auto-resolve.ts [base-branch]
 *
 * Env:
 *   GITHUB_TOKEN or GITHUB_PAT — PAT with pull_request write.
 *   Claude auth is resolved by the Agent SDK (OAuth / ~/.claude).
 *
 * The Greptile MCP server and GitHub MCP server are expected to be configured in
 * the repository's .mcp.json (settingSources: ["project"] picks them up).
 */
import { query } from "@anthropic-ai/claude-agent-sdk";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const OWNER = "overdrive-sh";
const REPO = "overdrive";
const MAX_CYCLES = 5;
const POLL_INTERVAL_MS = 30_000;
const POLL_TIMEOUT_MS = 10 * 60_000;
const POST_PUSH_SETTLE_MS = 60_000;

const GITHUB_TOKEN = process.env.GITHUB_TOKEN ?? process.env.GITHUB_PAT;
if (!GITHUB_TOKEN) {
  console.error("Error: GITHUB_TOKEN or GITHUB_PAT env var is required");
  process.exit(1);
}

// NOTE: Do NOT add an ANTHROPIC_API_KEY check here. The Claude Agent SDK
// resolves its own auth via OAuth / ~/.claude config. Adding a key check
// breaks environments that use OAuth (e.g. Conductor workspaces).

// ---------------------------------------------------------------------------
// GitHub REST + GraphQL
// ---------------------------------------------------------------------------

async function githubRest<T = unknown>(
  path: string,
  opts?: { method?: string; body?: unknown },
): Promise<T> {
  const res = await fetch(`https://api.github.com${path}`, {
    method: opts?.method ?? "GET",
    headers: {
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      ...(opts?.body ? { "Content-Type": "application/json" } : {}),
    },
    body: opts?.body ? JSON.stringify(opts.body) : undefined,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(
      `GitHub API ${opts?.method ?? "GET"} ${path}: ${res.status} ${text}`,
    );
  }
  const text = await res.text();
  return text ? (JSON.parse(text) as T) : (undefined as T);
}

async function githubGraphql<T = unknown>(
  gqlQuery: string,
  variables: Record<string, unknown> = {},
): Promise<T> {
  const res = await fetch("https://api.github.com/graphql", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ query: gqlQuery, variables }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GitHub GraphQL: ${res.status} ${text}`);
  }
  const json = (await res.json()) as {
    data?: T;
    errors?: { message: string }[];
  };
  if (json.errors) {
    throw new Error(
      `GitHub GraphQL errors: ${json.errors.map((e) => e.message).join(", ")}`,
    );
  }
  return json.data as T;
}

// ---------------------------------------------------------------------------
// Shell / Git helpers
// ---------------------------------------------------------------------------

async function git(...args: string[]): Promise<string> {
  const proc = Bun.spawn(["git", ...args], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = await new Response(proc.stdout).text();
  const stderr = await new Response(proc.stderr).text();
  const code = await proc.exited;
  if (code !== 0) {
    throw new Error(`git ${args[0]} failed (exit ${code}): ${stderr.trim()}`);
  }
  return stdout.trim();
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function log(msg: string) {
  console.log(`[pr-auto-resolve] ${msg}`);
}

async function currentBranch(): Promise<string> {
  return git("rev-parse", "--abbrev-ref", "HEAD");
}

// ---------------------------------------------------------------------------
// nwave feature detection (context only — does not alter flow)
// ---------------------------------------------------------------------------

type NwaveFeature = {
  featureId: string;
  roadmapPath: string;
};

/**
 * Detect if the branch corresponds to an nwave feature. The feature ID is
 * extracted from the last segment of the branch name (e.g.
 * "marcus-sa/phase-1-foundation" → "phase-1-foundation"). Presence of a
 * roadmap.json under docs/feature/{id}/deliver/ confirms the branch.
 *
 * This is used only to give dispatched agents additional context — review
 * items are NOT translated into roadmap steps.
 */
async function detectNwaveFeature(
  branch: string,
): Promise<NwaveFeature | null> {
  const featureId = branch.split("/").pop();
  if (!featureId) return null;
  const roadmapPath = `docs/feature/${featureId}/deliver/roadmap.json`;
  try {
    if (await Bun.file(roadmapPath).exists()) {
      log(`Detected nwave feature: ${featureId} (${roadmapPath})`);
      return { featureId, roadmapPath };
    }
  } catch {
    /* not an nwave feature */
  }
  return null;
}

async function commitSummary(base: string): Promise<string> {
  const raw = await git("log", `${base}..HEAD`, "--oneline");
  return raw || "(no commits)";
}

// ---------------------------------------------------------------------------
// PR helpers
// ---------------------------------------------------------------------------

type PR = { number: number; title: string; html_url: string };

type GithubPR = {
  number: number;
  title: string;
  html_url: string;
  head: { ref: string; sha: string };
};

async function findOrCreatePR(branch: string, base: string): Promise<PR> {
  const prs = await githubRest<GithubPR[]>(
    `/repos/${OWNER}/${REPO}/pulls?head=${OWNER}:${branch}&state=open&per_page=1`,
  );
  if (prs.length > 0) {
    log(`Found existing PR #${prs[0].number}`);
    return prs[0];
  }

  const commits = await commitSummary(base);
  const lines = commits
    .split("\n")
    .map((l) => `- ${l.replace(/^[a-f0-9]+ /, "")}`)
    .join("\n");

  const firstLine = commits.split("\n")[0] ?? "auto-resolve";
  const title = firstLine.replace(/^[a-f0-9]+ /, "");

  const body = [
    "## Summary",
    lines,
    "",
    "## Test plan",
    "- [ ] Tests pass locally",
    "",
    "🤖 Generated with [Claude Code](https://claude.com/claude-code)",
  ].join("\n");

  log("Pushing branch...");
  await git("push", "-u", "origin", branch);

  log("Creating PR...");
  try {
    const pr = await githubRest<GithubPR>(`/repos/${OWNER}/${REPO}/pulls`, {
      method: "POST",
      body: { title, head: branch, base, body },
    });
    log(`Created PR #${pr.number}: ${pr.html_url}`);
    return pr;
  } catch (err) {
    if (err instanceof Error && err.message.includes("No commits between")) {
      console.error(
        `Error: no commits between ${base} and ${branch}. Nothing to open a PR for.`,
      );
      process.exit(1);
    }
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Greptile — wait for review completion
// ---------------------------------------------------------------------------
//
// Greptile auto-reviews non-draft PRs. It integrates as a GitHub Check Run
// named "greptile" (case-insensitive) and additionally submits a PR review
// from `greptile-apps[bot]` (or `greptile-apps-staging[bot]`). Either signal
// indicates the review has finished for the current head SHA.
//
// We do NOT post `@greptile review` ourselves — that would double-trigger on
// every cycle. If a PR is in draft and Greptile is silent, that's a PR-state
// problem, not something this script should paper over.

type CheckRun = {
  name: string;
  status: string; // queued | in_progress | completed
  conclusion: string | null; // success | failure | neutral | cancelled | ...
};

type CheckRunsResponse = {
  check_runs: CheckRun[];
};

type PrReview = {
  id: number;
  user: { login: string } | null;
  state: string;
  submitted_at: string;
  commit_id: string;
};

async function getPrHeadSha(prNumber: number): Promise<string> {
  const pr = await githubRest<GithubPR>(
    `/repos/${OWNER}/${REPO}/pulls/${prNumber}`,
  );
  return pr.head.sha;
}

function isGreptileBot(login: string | undefined): boolean {
  if (!login) return false;
  const lower = login.toLowerCase();
  return lower.includes("greptile");
}

async function hasGreptileReviewForSha(
  prNumber: number,
  sha: string,
): Promise<boolean> {
  const reviews = await githubRest<PrReview[]>(
    `/repos/${OWNER}/${REPO}/pulls/${prNumber}/reviews?per_page=100`,
  );
  return reviews.some(
    (r) =>
      isGreptileBot(r.user?.login) &&
      r.commit_id === sha &&
      (r.state === "APPROVED" ||
        r.state === "CHANGES_REQUESTED" ||
        r.state === "COMMENTED"),
  );
}

/**
 * Wait until Greptile has finished reviewing the current head SHA. Accepts
 * either signal: a completed "greptile" check-run OR a PR review from the
 * Greptile bot against the head SHA.
 */
async function waitForGreptile(prNumber: number): Promise<void> {
  const start = Date.now();
  log("Waiting for Greptile to finish reviewing...");

  while (Date.now() - start < POLL_TIMEOUT_MS) {
    const sha = await getPrHeadSha(prNumber);

    const [checks, reviewed] = await Promise.all([
      githubRest<CheckRunsResponse>(
        `/repos/${OWNER}/${REPO}/commits/${sha}/check-runs`,
      ),
      hasGreptileReviewForSha(prNumber, sha),
    ]);

    const greptile = checks.check_runs.find((c) =>
      c.name.toLowerCase().includes("greptile"),
    );

    if (greptile && greptile.status === "completed") {
      log(`Greptile check completed (conclusion: ${greptile.conclusion})`);
      return;
    }

    if (reviewed) {
      log("Greptile finished (PR review submitted)");
      return;
    }

    const statusMsg = greptile ? `check: ${greptile.status}` : "no check yet";
    log(`Greptile review in progress (${statusMsg})...`);
    await sleep(POLL_INTERVAL_MS);
  }

  log("Timed out waiting for Greptile (10 min). Proceeding anyway.");
}

// ---------------------------------------------------------------------------
// Unresolved review threads — GraphQL (the real signal)
// ---------------------------------------------------------------------------

type ReviewThread = {
  id: string;
  isResolved: boolean;
  path: string;
  line: number | null;
  author: string;
  snippet: string;
};

const THREADS_QUERY = `
  query($owner: String!, $repo: String!, $pr: Int!) {
    repository(owner: $owner, name: $repo) {
      pullRequest(number: $pr) {
        reviewThreads(first: 100) {
          nodes {
            id
            isResolved
            path
            line
            comments(first: 1) {
              nodes {
                author { login }
                body
              }
            }
          }
        }
      }
    }
  }
`;

type ThreadsQueryResult = {
  repository: {
    pullRequest: {
      reviewThreads: {
        nodes: {
          id: string;
          isResolved: boolean;
          path: string;
          line: number | null;
          comments: {
            nodes: { author: { login: string }; body: string }[];
          };
        }[];
      };
    };
  };
};

async function getUnresolvedThreads(
  prNumber: number,
): Promise<ReviewThread[]> {
  const data = await githubGraphql<ThreadsQueryResult>(THREADS_QUERY, {
    owner: OWNER,
    repo: REPO,
    pr: prNumber,
  });

  return data.repository.pullRequest.reviewThreads.nodes
    .filter((t) => !t.isResolved)
    .map((t) => ({
      id: t.id,
      isResolved: false,
      path: t.path,
      line: t.line,
      author: t.comments.nodes[0]?.author?.login ?? "unknown",
      snippet: (t.comments.nodes[0]?.body ?? "").slice(0, 100),
    }));
}

// ---------------------------------------------------------------------------
// Prompts — classify, fix inline, dispatch agents for deeper work
// ---------------------------------------------------------------------------

function nwaveContextBlock(nwaveFeature: NwaveFeature | null): string {
  if (!nwaveFeature) return "";
  return `
This PR is for nwave feature "${nwaveFeature.featureId}" with roadmap at
${nwaveFeature.roadmapPath}. When dispatching nw-software-crafter or
nw-solution-architect, pass the feature id so they can read prior-wave
context from docs/feature/${nwaveFeature.featureId}/.
`;
}

function resolveCommentsPrompt(
  prNumber: number,
  nwaveFeature: NwaveFeature | null,
): string {
  return `\
You are resolving Greptile review threads on PR #${prNumber} in ${OWNER}/${REPO}.
${nwaveContextBlock(nwaveFeature)}
Instructions:

1. Fetch unresolved review threads on this PR. Use the github MCP server or
   \`gh api graphql\` via Bash. Focus on comments authored by Greptile
   (logins containing "greptile", e.g. greptile-apps[bot]) — but do process
   human-authored unresolved threads too if they exist.

2. CLASSIFY each thread into one of three categories BEFORE acting:

   A) INLINE FIX — simple, mechanical changes. Fix these directly:
      - Naming / formatting / typo fixes
      - Small bug fixes (off-by-one, wrong variable, missing null check)
      - Import changes, visibility changes
      - Doc comment updates
      → Read the file, make the fix, run \`cargo check\` (or the right
        project check for the affected code), commit with a message
        referencing the review, and RESOLVE the thread via the GraphQL
        resolveReviewThread mutation with a brief explanation.

   B) DISPATCH TO nw-software-crafter — non-trivial implementation work
      that fits within the existing architecture:
      - New behaviour inside an existing module
      - Non-trivial refactoring that does NOT change public contracts
      - Adding test coverage (TDD, proptest, DST) for untested paths
      - Performance improvements to existing code
      - Bug fixes that need a regression test first
      → Use the Agent tool with subagent_type="nw-software-crafter".
      → Pass a self-contained prompt that includes: PR number, thread
        context, exact files/lines, review comment quoted verbatim,
        acceptance criteria, the nwave feature id (if any), and the
        instruction to commit + push.
      → After the sub-agent returns successfully, RESOLVE the thread
        with "Implemented by nw-software-crafter in <commit-sha>".

   C) DISPATCH TO nw-solution-architect — work that changes architecture,
      public contracts, or cross-module boundaries:
      - API / trait shape changes
      - New component boundaries or crate reorganisation
      - Technology choice or consistency-boundary decisions
      - ADR-worthy design changes
      → Use the Agent tool with subagent_type="nw-solution-architect".
      → Pass a self-contained prompt that includes: PR number, thread
        context, exact files/lines, review comment quoted verbatim, the
        design question being raised, and the nwave feature id (if any).
      → If the architect concludes with a design change, have it either
        (a) implement the change directly if mechanical, or (b) hand off
        to nw-software-crafter via its own Agent tool call.
      → After the work returns successfully, RESOLVE the thread with
        "Resolved by nw-solution-architect in <commit-sha>".

   D) CROSS-CUTTING — affects multiple features beyond this PR:
      → Create a GitHub issue (github MCP issue_write method="create").
      → Resolve the thread with: "Deferred to #NNN — cross-cutting concern."

3. Err on the side of DISPATCH over INLINE FIX. If a review comment would
   take more than a few lines to implement, correctly requires new tests,
   or touches design, dispatch the appropriate agent.

4. Err on the side of nw-solution-architect when the change is about
   "shape" (types, boundaries, names, contracts); err on the side of
   nw-software-crafter when the change is about "behaviour" (correctness,
   coverage, perf, refactor inside an established shape).

5. Fix all INLINE FIX items first and push once. Then dispatch agents for
   B/C items one at a time, letting each commit and push before moving on.

6. Resolve review threads via:

       gh api graphql -f query='
         mutation($id: ID!) {
           resolveReviewThread(input: {threadId: $id}) {
             thread { isResolved }
           }
         }' -f id='<THREAD_ID>'

7. Push all commits with \`git push\` before returning.

Do NOT explain what you're doing. Classify, act, commit, push.`;
}

function resolveActionableItemsPrompt(prNumber: number): string {
  return `\
You are resolving actionable items from the PR description of PR #${prNumber}
in ${OWNER}/${REPO}.

Instructions:
1. Fetch the PR body (github MCP pull_request_read method="get", owner="${OWNER}",
   repo="${REPO}", pullNumber=${prNumber}).

2. Extract ALL actionable items from the PR body:
   - Checkbox tasks: unchecked \`- [ ]\` items
   - Inline actionable items: warnings, "missing" sections, "not implemented"
     statements, quantified shortfalls

3. Classify:
   - Actionable: "Update X", "Add Y", "Remove Z", "Fix W", missing tests,
     shortfalls
   - Skip: "Review changes", "Verify ...", subjective assessments, items
     already complete

4. For each actionable item:
   - Make the change (dispatch nw-software-crafter via the Agent tool if the
     work is non-trivial; otherwise fix inline).
   - Run \`cargo check\` and \`cargo nextest run\` for the affected crate
     (NEVER \`cargo test\` — see .claude/rules/testing.md).
   - Commit with message: \`feat: <item description>\` (or \`fix:\` / \`test:\`
     as appropriate).

5. Update the PR body (github MCP update_pull_request):
   - Replace \`- [ ] <completed>\` with \`- [x] <completed>\`
   - Keep all other content unchanged.

6. Push all commits with \`git push\`.

Do NOT explain what you're doing. Just act and push.`;
}

// ---------------------------------------------------------------------------
// Claude Agent SDK invocation
// ---------------------------------------------------------------------------

async function invokeClaudeAgent(
  prompt: string,
): Promise<{ success: boolean; result: string }> {
  let result = "";
  let success = true;

  // Agent is enabled so Claude can dispatch nw-software-crafter and
  // nw-solution-architect as sub-agents. Skill is enabled so the outer
  // agent can compose nw-* skills if it chooses to.
  const tools = [
    "Read",
    "Edit",
    "Write",
    "Bash",
    "Glob",
    "Grep",
    "Agent",
    "Skill",
    "Task",
    "TodoWrite",
  ];

  try {
    for await (const message of query({
      prompt,
      options: {
        cwd: process.cwd(),
        model: "claude-opus-4-6",
        permissionMode: "bypassPermissions",
        allowDangerouslySkipPermissions: true,
        tools,
        // Rely on project .mcp.json for github + greptile MCP servers.
        settingSources: ["project"],
        thinking: { type: "disabled" },
        effort: "high",
      },
    })) {
      if ("result" in message && message.type === "result") {
        const msg = message as { result: string; is_error?: boolean };
        result = msg.result;
        if (msg.is_error) success = false;
      }
    }
  } catch (err) {
    success = false;
    result = err instanceof Error ? err.message : String(err);
  }

  return { success, result };
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

async function main() {
  const baseBranch = process.argv[2] ?? "main";
  const branch = await currentBranch();

  if (branch === baseBranch) {
    console.error(
      `Error: cannot auto-resolve on the base branch (${baseBranch})`,
    );
    process.exit(1);
  }

  log(`Branch: ${branch}`);
  log(`Base: ${baseBranch}`);

  // 1. nwave feature detection — for agent context only.
  const nwaveFeature = await detectNwaveFeature(branch);
  if (nwaveFeature) {
    log(`nwave context: ${nwaveFeature.featureId} — agents will receive this`);
  }

  // 2. Find or create PR.
  const pr = await findOrCreatePR(branch, baseBranch);

  // 3. Wait for Greptile to finish reviewing (auto-triggered by GitHub).
  await waitForGreptile(pr.number);

  // 4. Initial thread count.
  let threads = await getUnresolvedThreads(pr.number);
  log(`Found ${threads.length} unresolved thread(s)`);

  if (threads.length === 0) {
    printReport(pr, 0, true, 0, nwaveFeature);
    return;
  }

  // 5. Resolution loop — driven by unresolved thread count.
  let cycle = 0;

  while (threads.length > 0 && cycle < MAX_CYCLES) {
    cycle++;
    log(
      `--- Resolution cycle ${cycle}/${MAX_CYCLES} (${threads.length} unresolved threads) ---`,
    );

    // 5a. Resolve review comments (inline + agent dispatch).
    log("Invoking Claude to classify and resolve review threads...");
    const commentResult = await invokeClaudeAgent(
      resolveCommentsPrompt(pr.number, nwaveFeature),
    );
    if (!commentResult.success) {
      log(`Warning: comment resolution had issues: ${commentResult.result}`);
    }

    // 5b. Resolve actionable items from PR body.
    log("Invoking Claude to resolve actionable items in PR body...");
    const itemResult = await invokeClaudeAgent(
      resolveActionableItemsPrompt(pr.number),
    );
    if (!itemResult.success) {
      log(
        `Warning: actionable item resolution had issues: ${itemResult.result}`,
      );
    }

    // 5c. Wait for Greptile to re-review after push (auto-triggered).
    log(`Waiting ${POST_PUSH_SETTLE_MS / 1000}s for checks to trigger...`);
    await sleep(POST_PUSH_SETTLE_MS);
    await waitForGreptile(pr.number);

    // 5d. Re-check unresolved threads.
    threads = await getUnresolvedThreads(pr.number);
    log(`${threads.length} unresolved thread(s) remaining`);
  }

  // 6. Final report.
  printReport(pr, cycle, threads.length === 0, threads.length, nwaveFeature);
}

function printReport(
  pr: PR,
  cycles: number,
  resolved: boolean,
  remainingThreads: number,
  nwaveFeature: NwaveFeature | null = null,
) {
  console.log(`
PR Auto-Resolve Complete

PR: #${pr.number} - ${pr.title}
URL: ${pr.html_url}
${nwaveFeature ? `nwave Feature: ${nwaveFeature.featureId}` : ""}
Resolution Cycles: ${cycles} of ${MAX_CYCLES}
Final Status: ${resolved ? "RESOLVED" : "UNRESOLVED"}
Remaining Threads: ${remainingThreads}${
    !resolved
      ? `\n\nRemaining issues require manual review.
URL: ${pr.html_url}
Rerun: bun --env-file=.env scripts/pr-auto-resolve.ts ${process.argv[2] ?? "main"}`
      : ""
  }
`);

  process.exit(resolved ? 0 : 1);
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(2);
});
