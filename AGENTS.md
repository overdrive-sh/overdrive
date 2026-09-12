YOU MUST READ ALL THE FOLLOWING FILES:
- CLAUDE.md
- .claude/rules/bpf.md
- .claude/rules/debugging.md
- .claude/rules/design.md
- .claude/rules/development.md
- .claude/rules/rust.md
- .claude/rules/testing.md
- .claude/rules/verification.md

## Commit attribution

Do not change the Git author when creating a commit. Attribute the coding
agent with a co-author trailer instead. Every commit created by Codex must
include exactly `Co-Authored-By: Codex <codex@openai.com>`; never add or retain
Claude Code, Claude, or Anthropic attribution (including generated-by text or
co-author trailers) on a Codex-created commit.

nwave skills can be found at $HOME/.claude/skills/nw-*/SKILL.md
nwave agents can be found at $HOME/.claude/agents/nw/

## Implement to the design — never invent API surface

When implementing against an accepted design (an ADR, `brief.md`, a
feature-delta, a roadmap step), match the design's **exact public API
shape**. Do **not** invent new public surface — a new method, type, enum
variant, trait, or parameter — to make tests green or to fill a gap the
design left underspecified. The design is a contract, not a suggestion;
an implementation that adds API the ADR did not call for has *diverged*,
even if every test passes.

When the design specifies a *model* but not the exact *signature* (e.g.
"the transient is the step's `Err` re-driven by the engine" without the
function shape), the gap is **not** licence to improvise. **STOP and
surface the gap** to the user / orchestrator and get the shape pinned —
never reach for the nearest mechanism that compiles. A subagent that
grades itself on "tests green" will invent surface; that is the failure
mode this rule exists to prevent.

This binds three roles:

- **Crafters**: build only the API the design names. If you need a
  primitive the design doesn't specify, return a blocker — do not add a
  public method/type/variant on your own initiative.
- **Orchestrators dispatching crafters**: point the crafter at the
  authoritative design (the ADR / feature-delta / roadmap step) and
  forbid inventing API. Do **not** pre-explore the codebase or restate
  the signature yourself — the crafter reads the design and is bound by
  the crafter rule above; duplicating that read is wasted work. The
  orchestrator's job is to not *loosen* the contract: granting latitude
  ("pick the cleanest shape," "add a variant if needed") *causes*
  divergence — do not.
- **Reviewers / orchestrators accepting work**: verify the output
  against the design's API shape, not just "tests pass." A green suite
  over a divergent API is a rejection, not an approval.

**Precedent** (the `workflow-result-error-model` feature, ADR-0065):
crafters twice invented surface the ADR did not sanction — a
`TerminalErrorKind::Retryable` variant (a "terminal error" that wasn't
terminal, flatly contradicting the ADR's "retryable never reaches the
return type"), then a second `ctx.run_retryable` step method instead of
the ADR's single `ctx.run`. Both compiled and passed their tests; both
were design divergences caught only in adversarial review and by the
user, and both cost a rework cycle. The cost of surfacing a gap is one
message; the cost of inventing past it is a wrong contract that
propagates until someone notices.

## DELIVER orchestration rules

These rules are mandatory for `/nw-deliver` and `/nw-execute` work in this
repository.

- Every roadmap step gets its own fresh, isolated crafter agent. Spawn it
  without inherited conversational turns and give it the complete DES prompt.
  Never reuse a crafter from an earlier step for a later step.
- The only crafter-reuse exception is review remediation: findings for a step
  go back to the original crafter for that same step. They never go to a later
  step's crafter.
- Every step gets its own fresh, isolated reviewer. Do not reuse a reviewer
  across roadmap steps. The same reviewer may perform iteration 2 for its own
  step after remediation.
- Before reporting completion, each reviewer writes its full review artifact to
  `docs/feature/{feature-id}/deliver/review-{step-id}.md`. The artifact records
  every iteration, verdict, finding, and remediation disposition. A returned
  chat verdict alone is not a completed review, and the next step must not start
  until the on-disk review exists.
- Persist review artifacts as native Markdown following the repository's
  existing DELIVER review convention: headings, metadata, findings, evidence,
  verification, remediation dispositions, and verdicts rendered as prose,
  lists, and tables. Do not put a YAML response inside a `.md` file. Structured
  YAML may be returned to the orchestrator for machine parsing, but it does not
  replace the Markdown review artifact.
- Crafters and reviewers do not send progress updates, partial findings, or
  status checkpoints back to the orchestrator. They report only when their
  bounded work is complete, or when they have reached a genuine blocker that
  prevents further progress. The orchestrator must not poll or prompt them for
  intermediate status.
- After dispatching an agent, the orchestrator waits silently for that agent's
  completed report or genuine blocker. Do not repeatedly check for progress or
  emit recurring "still running" commentary while the agent works. A wait
  timeout is not a status event and must not be surfaced to the user.
- An incidental request made during an active DELIVER run, such as updating an
  orchestration rule or documentation, does not end or pause the run. Complete
  the incidental request, then immediately resume the pending DELIVER phase
  unless the user explicitly stops, pauses, or replaces the active workflow.
- Treat a user statement, observation, correction, or question as
  conversational by default: answer it, but do not infer authorization to
  take an action from it. Act only when the user explicitly requests an
  action. In particular, pointing out a model mismatch, defect, risk, or
  surprising behavior does not authorize stopping, interrupting, replacing,
  or modifying an active agent or workflow.
- The step sequence is strict: fresh crafter RED -> GREEN -> COMMIT, fresh
  reviewer, original-crafter remediation if required, reviewer re-review, then
  and only then the next roadmap step.
- Review/remediation has no iteration cap. Keep cycling a step's original
  crafter for remediation and that step's reviewer for re-review until the
  reviewer returns `APPROVED`. Agent-definition defaults such as "max 2 review
  iterations" do not apply to this repository's DELIVER workflow and must not
  trigger human escalation or advancement with unresolved findings. If an
  original agent is no longer addressable, dispatch a fresh isolated
  replacement for the same role and step, then continue the cycle.
- Do not run mutation testing during individual roadmap steps. Mutation testing
  is a single final DELIVER-wave gate, after all steps and their reviews are
  complete. Per-step crafters must not delay RED -> GREEN -> COMMIT for mutation
  testing or edit mutation exclusions as part of a step unless the user
  explicitly overrides this rule.
- Keep executable tests and verification expectations as independent evidence
  layers. Rust tests exercise the production composition root in-process and
  must not spawn the built Overdrive production binary, emit expectation
  evidence, or act as an expectation runner. A `verification/expectations/*`
  runner is black-box: it directly drives the built default-feature binary and
  observes operator/kernel/wire/cleanup surfaces with crate-independent
  external tools. It must not invoke `cargo test`, `nextest`, a Rust test
  binary, or import/link an `overdrive-*` crate. Legitimate external Tier-3
  fixtures such as Cloud Hypervisor or guest workload processes remain allowed
  when the test contract requires them; they do not make the test harness the
  system-under-test process boundary.
- Keep examples, expectations, and integration tests at their distinct
  boundaries. Repository-root `examples/` contains checked-in,
  operator-runnable product examples that express a user journey. An
  expectation drives one of those examples through the built product and
  verifies only the stakeholder-visible black-box outcome. An integration test
  uses the production crates in-process to prove the internal guarantees that
  make the outcome true, including private lifecycle state, protocol framing,
  decoder behavior, normalized kernel-program identity, loss detection,
  generation stability, exact counters, and cleanup complements. Expectations
  must not recreate specs or workload programs inline, absorb integration-test
  assertions, invoke the test harness, or duplicate the Rust implementation in
  Python, shell, or another helper.
- A green suite, complete DES log, clean commit scope, or 100% mutation score
  does not replace the reviewer. Step 01-01 passed all of those and the
  reviewer still caught a blocking Contract Shape declaration defect.
- The orchestrator checks mechanical evidence only: DES phase order, commit
  trailers/stat, expected file scope, command results, and reviewer verdict.
  Deep correctness, design compliance, test honesty, and diff scrutiny belong
  to the dedicated reviewer per `.claude/rules/development.md`.
- Use precise domain terminology in every explanation, handoff, design, and
  review. Preserve the exact owner, object, state, boundary, and scope named
  by the code or design; do not replace it with convenient shorthand, a
  broader subsystem name, or an adjacent concept. If a concise label would
  lose any of those distinctions, state the precise term instead.
- Implementation review remediation must stay within the architecture already
  approved by DESIGN. A reviewer may identify an architectural gap, but it
  must not invent or iteratively prescribe a new persistence subsystem,
  system-of-record boundary, ownership model, consistency protocol, recovery
  protocol, or other architectural mechanism inside a DELIVER step. If a
  finding cannot be closed without such a choice, record the finding as a
  blocking DESIGN gap and return it for a separate DESIGN remediation and
  independent design review. Resume the original DELIVER step only after that
  design is approved; do not evolve implementation review iterations into an
  unreviewed architecture-design process.
- Do exactly the work the user requested. Do not replace a bounded fix with
  adjacent hardening, generalized lifecycle correctness, speculative failure
  handling, architectural cleanup, or a mechanism an agent considers more
  complete. A reviewer finding does not expand the task. If a finding or
  proposed dependency is not necessary to satisfy the user's stated outcome
  and accepted design, reject it as out of scope. Technical plausibility,
  severity, or elegance is not authorization. When the requested fix is small,
  the design and implementation must remain small unless the user explicitly
  expands them.
- If a designer, reviewer, crafter, or orchestrator believes something outside
  the approved scope should be added, changed, hardened, generalized, or
  redesigned, it must surface that proposal to the user and obtain explicit
  approval before acting on it. Record the rationale and likely impact, but do
  not edit design artifacts, add findings that mandate the expansion, change
  production code, or dispatch remediation for it while approval is pending.
  Agent judgment that an addition is useful, safer, cleaner, or a good idea is
  not authorization to invent or implement it.
- Treat every review finding as a hypothesis until its failure is proven
  reachable through the current production code. Before accepting a finding
  for DESIGN or remediation, the reviewer must cite the concrete production
  entry point, complete caller/owner path, exact state and ordering that
  trigger it, and the current shutdown/cancellation/retry behavior with
  file-and-line evidence. A theoretically cancellable Rust future, a forced
  test-only abort, or an internally consistent hypothetical state is not a
  production defect when the real owner drains the operation or the state dies
  with its process. Findings without this reachability proof are rejected, not
  converted into design requirements.
- Before accepting that anything needs a fix, remediation, or DESIGN change,
  reproduce the claimed failure with a bounded spike or a regression test that
  fails against the current implementation through the real production entry
  point and owner path. Static suspicion, design prose, a theoretical trace,
  or a test-only state that production cannot reach is not proof. The spike or
  failing test must isolate the original claimed defect without depending on
  the proposed remedy. If the failure cannot be reproduced, keep it recorded
  only as an unproven hypothesis; do not change production code or expand the
  design to address it.
- For suspected control-plane defects whose correctness depends on ordering,
  timing, concurrency, crash/restart, retry, or convergence, the required
  failing regression is first a seeded `overdrive-sim` safety, liveness, or
  convergence invariant against the current implementation. Designers and
  reviewers must not promote an imagined schedule into a finding or design
  requirement until that invariant fails reproducibly and prints its seed. A
  real-production-binary spike may additionally prove that the triggering state
  is reachable and that the Sim model matches production composition; it does
  not replace the invariant. Real-kernel Tier-3 tests remain the evidence for
  host-adapter effects that simulation cannot observe, such as actual netns,
  veth, TAP, nftables, cgroup, process, and redb behavior. Do not add a Sim
  seam, production API, or architectural mechanism merely to manufacture the
  hypothesized state; surface any missing testability boundary to the user for
  approval first.
- Revalidate the premise before designing the remedy. Designers must read the
  affected production paths and distinguish observed code facts from proposed
  behavior; accepted DESIGN prose and a reviewer assertion are not substitutes
  for implementation evidence. If the proposed remedy reaches into subsystems
  outside the proven path--for example broker scheduling, hydration, probe
  persistence, replay, task ownership, or recovery protocols--stop and prove
  that dependency is unavoidable before adding it. Do not make an invented
  failure model internally consistent.
- Remediation reviews must test both the fix and its necessity. When successive
  findings concern machinery introduced by the previous remediation rather
  than the original reachable defect, reopen the mechanism choice and simplify
  or remove it instead of continuing a patch-review-patch loop. No-iteration-cap
  means genuine defects are resolved until approval; it is not permission for
  an unbounded architecture-growth loop.
- When an existing synchronous interface gains work that must be awaited, make
  that existing interface async and update its bounded implementations and call
  sites. Do not preserve a stale synchronous signature by discovering a Tokio
  runtime, spawning a detached future, or adding a second public method. Task
  submission is not effect completion: release, commit, install, cleanup, and
  other ordering-sensitive operations must return only after their promised
  async effect or typed failure handling has completed.
- Only the isolated crafter executing a step may write that step's DES phase
  events. An interrupted or replacement agent must not claim inherited work;
  it independently reruns RED and logs only phases it actually executes.
- Preserve all pre-existing dirty work. Never reset, discard, overwrite, or
  silently commit unrelated files. A replacement agent must audit partial
  step changes left by an interrupted agent before adopting any of them.
- Use the user-selected GPT 5.6 Luna model with maximum thinking for every
  crafter and reviewer. When the agent interface inherits the current
  Conductor session model/reasoning and exposes no per-agent selector, inherit
  it; never downgrade reviewers to their legacy Haiku frontmatter default.
- Keep orchestrator reads minimal: the mandatory project files above, the
  command skill being invoked, the selected roadmap, DES rigor/log state, and
  mechanical results. Specialized agents load their own role skills and the
  code/design context needed for their bounded task. Do not preload unrelated
  skill trees in the orchestrator.
- New or transitioned tests must carry their required per-test Contract Shape
  declaration. For source-local pure-function Rust properties, use the exact
  rustdoc line `/// CONTRACT_SHAPE: pure-function.` on every live property.
- Roadmap allowlists must account for compiler-required fallout. Adding a
  public `AllocationSpec` field requires neutral updates at every existing
  struct literal; enabling nix ioctl macros requires the workspace `nix`
  dependency's `ioctl` feature in `Cargo.toml`. Treat those as tightly bounded
  mechanical fallout, not as permission for unrelated behavior changes.
- Roadmap `implementation_scope`, `files_to_modify`, and similar file lists are
  guidance, not restrictive allowlists. The acceptance criteria define the
  required implementation boundary. Crafters may change additional production,
  API, renderer, test, harness, configuration, and compiler-fallout files when
  those changes are necessary to satisfy the step honestly. They must document
  why each expansion is required and keep it tightly related to the criterion;
  they do not stop merely because a necessary file was omitted from the roadmap
  list.
- Initialize `execution-log.json` with `des-init-log`; never hand-write phase
  events. In this environment the DES launchers require
  `PYTHONPATH=/Users/marcus/.claude/lib/python` so they can import the bundled
  `des` package.
- A roadmap with `validation.status = pending` is not executable unless it is
  reviewed or the user explicitly directs that it be marked approved. Do not
  silently self-approve it.

The installed nWave skill layout is
`$HOME/.claude/skills/nw-*/SKILL.md`; agent definitions are under the agent
directory listed above.
