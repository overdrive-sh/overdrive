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

**Scope — interface contracts, not internal structure.** The line is
nWave's: *architecture owns WHAT, the crafter owns HOW*. The design pins
**interface contracts**: the public and cross-crate API (pub types, traits
and their methods, variants of public enums, port and composition-root
signatures), wire and persisted formats, config and CLI surface,
ownership, lifecycle and state meanings, invariants, and the error
taxonomy an owner branches on. It does **not** pin internal structure:
private functions and their parameters, module-private types and
visibility, or how internal state is threaded. The crafter decides those,
and the DELIVER reviewer checks them against the contract. Test-support
surface (fixtures, test-local port implementations, sim scripting used
only by tests) is DISTILL's to shape, not DESIGN's. A gap in internal
structure or test support is **not** a blocker and is never routed back
to DESIGN; a review finding about internal structure is a note for the
DELIVER reviewer, not a DESIGN finding.

This binds three roles:

- **Crafters**: build only the API the design names. If you need an
  interface-contract primitive the design doesn't specify, return a
  blocker — do not add a public method/type/variant on your own
  initiative.
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

## Typed error contract gate

When an accepted design requires a typed error, it must identify the semantic
error set and pin its representation at the relevant interface: the variants,
payloads, and mappings callers rely on. Naming a generic return type such as
`NetlinkError` or `io::Error` does not resolve an unspecified semantic error
contract. An existing error envelope is not evidence that every representation
inside it is approved.

If the required semantic distinctions or their interface representation are
missing, report `SPECIFICATION_AMBIGUITY` and pause implementation of the
affected error path. Return the gap to DESIGN to pin the exact contract before
resuming. Do not silently encode the missing distinctions in string messages,
choose an arbitrary generic variant, or invent public variants or payloads.
Diagnostic strings may add context to an approved typed cause; they must not
replace semantic distinctions the design requires to be typed.

This gate applies at each handoff:

- **DESIGN**: declare the typed error set and its exact interface
  representation, including mappings from the designed failure conditions.
- **DISTILL**: check that contract before authoring error scenarios. A missing
  set or representation is an upstream specification ambiguity, not a test
  fixture decision. Exercise each declared error and verify that errors outside
  the declared set do not escape.
- **Crafters**: check the error contract before RED/GREEN implementation. An
  underspecified typed error must be surfaced, even when an existing generic
  carrier with a string message would compile and make tests pass.
- **Reviewers**: include an explicit mapping from each designed semantic
  failure to its approved and implemented typed representation. An unresolved
  contract gap blocks approval; a typed outer envelope and green tests do not
  establish compliance.
- **Orchestrators**: include this gate in crafter and reviewer handoffs. Route
  genuine error-contract gaps to DESIGN; do not authorize a generic carrier or
  string message as a shortcut around pinning the contract.

The interface ownership rule requires consistency with the approved contract,
not avoidance of new public API. These workspace interfaces may change when
DESIGN pins the required change and its bounded caller fallout. Private error
types and internal implementation remain the crafter's responsibility when the
approved interface and semantic error contract are already complete.

## Correctness and invariant precedence

When constraints conflict, apply this order:

1. User-approved outcomes and correctness/security invariants.
2. Reproduced production evidence and primary-source facts.
3. Architectural consistency and ownership.
4. Exact API shape.
5. Implementation scope, compatibility, cost, and diff size.

"Smallest change" means the smallest solution that satisfies every
higher-order constraint. A smaller solution that weakens an outcome or
invariant is invalid, not preferable.

Necessary API, ownership, lifecycle, compiler-fallout, and test changes
required to preserve an accepted invariant are not scope expansion, adjacent
hardening, or optional cleanup. Scope controls apply only after every accepted
outcome and invariant is satisfied.

A user waiver of a review cycle does not waive this correctness gate.

## Native falsification gate

When native or otherwise reproduced evidence disproves a DESIGN premise:

- Immediately invalidate every decision that depends on that premise.
- Return the affected design and roadmap validation to `pending` unless the
  user explicitly directs a different status.
- Stop implementation against the contradicted design.
- Reopen previously rejected alternatives whose rejection depended on the
  falsified premise.
- Do not modify tests or weaken the contract to fit the current
  implementation.
- Resume DELIVER only after an exact replacement design is recorded.

Prior acceptance or rejection is not authority after its premise is
falsified. Implementation cost or existing API shape cannot preserve a
falsified decision.

## Contract weakening requires explicit approval

No agent may change `zero` to `some`, `must` to `may`, remove a `never`, permit
a previously forbidden state, weaken a security boundary, or reduce an
observable outcome without quoting the user's explicit approval for that exact
weakening.

"Cheaper", "smaller", "less surface", "preserves the existing API", and
"avoids broader changes" are never sufficient reasons to weaken a contract.
When the only invariant-preserving solution requires broader API, ownership,
lifecycle, compiler-fallout, or test work, that work is required rather than
optional scope expansion.

Before committing a design change, inspect its normative contract diff. Any
removal or relaxation involving terms such as `zero`, `never`, `must`,
`exactly`, `fail-closed`, `forbidden`, or `required` blocks the commit unless
the exact user approval for that relaxation is recorded. This is a semantic
gate: synonym substitution or prose restructuring must not conceal a weaker
outcome.

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
- Subject to **Correctness and invariant precedence**, do exactly the work the
  user requested. Do not replace a bounded fix with
  adjacent hardening, generalized lifecycle correctness, speculative failure
  handling, architectural cleanup, or a mechanism an agent considers more
  complete. A reviewer finding does not expand the task. If a finding or
  proposed dependency is not necessary to satisfy the user's stated outcome
  and accepted design, reject it as out of scope. Technical plausibility,
  severity, or elegance is not authorization. When the requested fix is small,
  the design and implementation must remain small unless preserving an
  accepted outcome or invariant requires the additional work, or the user
  explicitly expands them.
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
- Model routing is role- and wave-specific:
  - Use GPT 5.6 Luna with maximum thinking **only for DELIVER crafters and
    DELIVER reviewers**.
  - Use GPT 5.6 Sol with extra-high (`xhigh`) thinking for **every other nWave
    wave and role**, including DISCOVER, DIVERGE, DISCUSS, DESIGN, DEVOPS,
    DISTILL, their reviewers, and orchestrators (including the DELIVER
    orchestrator).
  - When the agent interface inherits the current Conductor session model and
    reasoning, inherit only when that resolves to the required model above.
    When a per-agent selector is available, select the required model and
    reasoning explicitly. Never fall back to a role's legacy Haiku default.
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


<claude-mem-context>
# Memory Context

# [helios/wellington-v2] recent context, 2026-10-05 1:11am GMT+2

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision 🚨security_alert 🔐security_note
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 50 obs (35,995t read) | 1,675,997t work | 98% savings

### Sep 30, 2026
S13940 User provided decisions on netns-density-295 open items: proceed with kill-time bounds option a, treat thread-spawn panic as recoverable error reinforcing no-panic principle, delegate B3c/H3 firewall test rewrites to agent not primary session (Sep 30 at 8:29 PM)
S13937 User expressed frustration about netns-density-295 complexity asking why simple tap replacement became convoluted and noting safety classifier blocks (Sep 30 at 8:29 PM)
S13941 User decided netns-density-295 open items and primary session investigated netlink runtime panic location violating no-panic architectural principle (Sep 30 at 9:54 PM)
S13939 User frustrated about netns-density-295 complexity asking why simple TAP replacement became convoluted, primary session explained root causes and retrieved original issue scope showing density motivation (Sep 30 at 9:55 PM)
S13969 Commit netns-density-295 DESIGN finalization work including R19 withdrawal decision and roadmap approval (Sep 30 at 9:56 PM)
### Oct 3, 2026
78112 12:19p 🔵 Native evidence testing reveals R18 reproduction and R19 falsification with E14(e) blocker
78117 12:20p 🔵 Platform Blocked TIME_WAIT Experiment for Cybersecurity Risk
78118 " ✅ Workflow Pattern Documented: Delegate Reading to Agents
78119 " ⚖️ Metal Host Environment Diagnosis Selected for E14(e) Blocker
78120 12:21p 🔵 Metal Host Contains Stale nftables Configuration from Previous Run
78121 12:22p ✅ Metal Residue Inspection Results Documented
78127 12:42p 🔵 User Requests Problem and Solution Clarification After E14(e) Test Success
78128 12:45p 🔵 Primary Session Stuck in Decision Loop Despite E14(e) Evidence Being Complete
78137 1:00p ✅ ADR-0140 Updated to Record R19 Withdrawal and E14(e) Named-Service Test Completion
78138 1:05p ⚖️ ADR-0140 Withdrawn — TPROXY-Before-Mark Reorder Not Required
78139 1:16p ⚖️ R19 Conditional Withdrawal Finalized with Native E14(e) Evidence
78140 1:22p 🔵 Communication breakdown on decision ownership in primary session
78142 1:46p ⚖️ R19 TPROXY-before-mark Rule Reordering Withdrawn Under Native Condition
78143 " 🔵 NETNS-Density-295 Delivery Blocked on R19 Falsification Contradiction
78141 1:49p ✅ R19 TPROXY rule reordering withdrawn - existing order provides fail-closed behavior
78144 1:52p 🔵 D-295-R19 Rule Reorder Withdrawn on Native Evidence; Documentary Reconciliation Required Before Roadmap Gate
78145 2:01p ✅ Roadmap Step 08-01 Updated to Reflect R19 Withdrawal and R18 Confirmation
78146 2:03p ✅ Roadmap step 08-01 reconciled after D-295-R19 rule-order withdrawal
78147 2:11p 🔵 Roadmap validation status audit across 77 feature deliverables
78150 2:13p ✅ netns-density-295 roadmap approved after DESIGN faithfulness verification
S13970 Complete netns-density-295 DESIGN handoff including R19 withdrawal decision, E14 evidence collection, roadmap approval, and prepare commit of finalized artifacts (Oct 3 at 2:13 PM)
S13972 Query remaining roadmap steps after completing netns-density-295 DESIGN finalization and commit (Oct 3 at 2:16 PM)
S13971 Commit netns-density-295 DESIGN finalization work including R19 withdrawal, E14(e) evidence resolution, and roadmap approval (Oct 3 at 4:20 PM)
S13997 Vsock replacement design for netns-density-295 feature after Linux bridge port limit falsified original design (Oct 3 at 4:24 PM)
### Oct 4, 2026
78281 2:11p 🔵 Pasted Text Attachment Analysis Required
78285 " 🔵 Linux Bridge Port Limit Falsified Shared-Bridge Networking Design
78286 " 🟣 Vsock Transport Capacity Proven at 16,384 Activated Devices
78287 " ⚖️ Vsock Selected as Replacement Transport for VM Networking Attachment
78288 " ✅ Replacement Design Workflow Status Set to Pending Approval
78289 " ✅ Guest Networking and mTLS Intercept Components Modified for Vsock Integration
78291 " 🔵 Feature Delta Documents 16,550 Lines of Replacement Design with Falsification Records
78293 2:14p 🔵 Driver Type Hierarchy with Unikernel and VM Variants
78294 " 🔵 DNS and Guest Network Boot Sequence with DDN-2 Single-Owner Pattern
78295 " 🔵 Network Namespace Density Benchmark with 16,384 Population Target
78297 2:22p 🔵 DNS Responder Wildcard Binding Strategy with IP_PKTINFO Source Pinning
78298 " 🔵 Overdrive-Init In-Guest PID 1 Agent with Vsock Beacon and Network Bootstrap
78299 " 🔵 Vsock Usage Scope Limited to Host-Guest Beacon, Not Network Transport
78300 " 🔵 Shared Bridge Production Architecture with TAP Attachment EXFULL Surface
78301 " 🔵 Feature-Delta Document Structure Inventory for Vsock Architecture Replacement Design
78303 2:25p 🔵 Native E18 Bridge Capacity Falsification at 1024-Port Linux Kernel Limit
78304 " 🔵 Vsock Replacement Transport Selection with 16,384-Device Native Capacity Validation
78305 " 🔵 R19 Conditional Withdrawal Approval for Ordinary Flow with Pending E14(e) Gate
78306 " 🔵 Netns-Density-295 Delivery Roadmap with 84-Scenario Test Matrix and 10-Phase Structure
78307 2:26p 🔵 Comprehensive Roadmap Inventory with Bridge-Transport Dependency Classification
78309 2:27p 🔵 In-guest kernel mTLS architecture validated for VM workloads
78310 " 🔵 Linux bridge port ceiling blocks 16K guest density target
78311 " 🔵 VMM evaluation compares Cloud Hypervisor against Firecracker for microVM platform
78312 2:33p 🔵 E18 benchmark hits Linux bridge port ceiling blocking 16K guest density target
78313 " ⚖️ Vsock packet uplink proposed to replace bridge-based microVM network attachment
**78314** 2:38p 🟣 **Vsock attachment replacement design completed with activation, classification, and measurement ADRs**
On October 4, 2026, the primary session completed the vsock attachment replacement design by proposing three additional Architecture Decision Records addressing activation/quiescence mechanisms, traffic classification, and measurement strategy. ADR-0148 defines a forwarder per-allocation gate mechanism replacing the TAP-based activation and quiescence of ADR-0131 and ADR-0124, with the gate having three states (Closed for registered with no uplink, Open, and Quiesced) and activation being a serialized operation owned by the shared guest-network owner that connects the uplink per ADR-0147, completes the preamble, inserts the active endpoint entry, and opens the gate after the exact mtls.intercept.install.success event and before EXEC. Quiescence sets the owner's latch then closes every Open gate, with a gate not Open meaning nothing crosses the forwarder for that allocation in either direction, and a gate the forwarder cannot confirm closed within the quiescence bound being reported unconfirmed with the VM killed. The quiescence mechanism is an in-process latch rather than up to 16,384 netlink link-down calls so the quiescence bound is expected to stay at its 1s floor though this expectation must be measured not assumed. Loss of an uplink is terminal for the allocation with the allocation becoming condemned and killed and the owner never reconnecting an uplink. ADR-0149 defines one aya-rs TCX ingress classifier attached first in order to the ovd-gtun0 shared TUN device, with the classifier reading a node-global endpoint map keyed by guest IPv4 source carrying an active flag value with a maximum of 65,536 entries, and per-packet behavior parsing from the IPv4 header with no Ethernet or ARP layer marking TCP 0x295a, marking non-TCP to the gateway 0x295b, dropping map misses with a counter, dropping malformed packets, and dropping non-TCP direct to any other address. The IP-family guard table ip overdrive-guest-guard owns a prerouting chain at filter priority -300 on iifname ovd-gtun0 with exactly three rules: accept and preserve packets marked 0x295a, clear the mark and accept packets marked 0x295b, and count and drop everything else. The single-loss outcome is preserved with TCX link loss causing the guard to drop unmarked datagrams, endpoint entry loss causing TCX to drop on map miss, guard loss causing TCX to mark and the intercept program to capture, IP program loss causing the R18 guard to drop marked TCP, and forwarder or TUN loss causing nothing to enter the host. ADR-0150 defines the E18 and T1 receipt measurement strategy on 16,384 activated vsock uplink attachments terminated at the muxer boundary, with the measurement running the production owner, forwarder, shared TUN, TCX classifier, guards, and intercept owner all composed as in run_server and each of 16,384 attachments being assigned, provisioned, given its intercept elements, and activated. Uplink peers are benchmark-owned peer processes that bind the exact per-allocation muxer socket path, answer the stock Cloud Hypervisor line protocol, and run the guest side of the preamble and frame protocol, with every attachment required to show a completed activation, a datagram round trip through ovd-gtun0, and read-back. Device-layer cost is cited from the approved spike as a separate receipt and is never summed into the E18 result as if both were measured together. The measurement approach means no receipt measures a VMM process, a guest, or end-to-end VM density with D-295-R9 standing that no VM density claim is made. The complete vsock replacement design now spans seven decision records forming a coherent alternative to the failed bridge-based approach: V1 ADR-0145 vsock packet uplink removing virtio-net, V2 ADR-0146 shared TUN and in-serve forwarder, V3 ADR-0147 uplink identity via allocation muxer socket, V4 ADR-0149 shared TUN TCX classifier and IP-family guard, V5 ADR-0148 activation and quiescence gate mechanism, and V7 ADR-0150 E18 measurement on uplink attachments at muxer boundary. All seven ADRs carry PROPOSED status pending explicit user approval and independent DESIGN review with nothing implementable from any of these records until approved.
~2006t 🛠️ 32,253

**78325** 2:45p 🔵 **User Questions Vsock Design Alignment with GH #303 and Userspace Forwarding Trade-off**
User raised five critical questions about the proposed vsock replacement design for netns-density-295 feature. First question concerns whether V1's guest traffic mechanism (overdrive-init creates tun0 and pumps packets over vsock) aligns with the architecture described in GH #303 and PR #306, and why virtio-net is not being used instead. Second question challenges why V2's design requires a forwarder running inside the serve process to feed the shared TUN. Third and most significant concern addresses V8's performance trade-off introducing per-packet userspace forwarding, which appears to contradict the stated goal of GH #303 and PR #306 to eliminate the host L4 proxy (ADR-0069 being replaced). Fourth question expresses alarm that making serve the data path creates a single point of failure where serve death terminates all guest connectivity. These questions suggest potential fundamental misalignments between the proposed vsock design and the approved in-guest mTLS work, particularly around performance goals and failure modes. The user's tone indicates these are blocking concerns requiring resolution before design approval can proceed.
~466t 🔍 86,508

**78326** 5:15p 🔵 **GitHub Issue #303 Reveals Fundamental Conflict with Vsock Replacement Design**
User investigated GitHub issue #303 "In-guest kernel mTLS for VM workloads: Camblet-style socket hook with a host-held key" revealing fundamental architectural conflict with proposed vsock replacement design. Issue #303 describes moving transparent mTLS from host L4 proxy (ADR-0069 being superseded) into guest kernel using loadable kernel module that hooks connect/accept, relays handshake records to host agent over vsock, then installs kTLS on workload's own TCP socket so application data flows directly over kTLS without touching host. Key architectural principle in #303: vsock is used ONLY for handshake relay (RESOLVE/ACCEPT frames), application ciphertext travels over guest's own TCP socket with kernel kTLS carrying record layer. Host agent explicitly "leaves the data path after the handshake" as documented benefit, enabling in-flight connections to survive host-agent restart because session state lives in guest kernel. Nine spikes (B/B2/B3/C/D/E) proved this mechanism on Unikraft and Linux stock kernel as out-of-tree .ko module. In direct conflict, vsock replacement design decisions V1/V2/V8 propose using vsock for ALL packet traffic: V1 has overdrive-init create tun0 and pump IPv4 packets over one vsock stream per VM, V2 feeds shared host TUN ovd-gtun0 via forwarder running inside serve, V8 introduces per-packet userspace forwarding through Cloud Hypervisor muxer then serve forwarder. This architectural conflict explains user's alarm: vsock design puts serve in data path (if serve dies all guests lose connectivity, contradicting #303's restart-survival claim) and introduces per-packet userspace forwarding (contradicting #303's "no host proxy hop" and "host leaves data path" goals). The two proposals use vsock for opposite purposes: #303 for control-plane handshake coordination only, vsock design for data-plane packet forwarding. Issue notes host proxy ADR-0069 is being replaced so "leave unikernels on host proxy" is not a path, and at 16,384-guest target one flow per guest means about 65,536 pump threads which in-guest mechanism avoids.
~815t 🔍 9,213

S13998 User challenged vsock design revealing fundamental architectural error requiring withdrawal and redesign (Oct 4 at 5:17 PM)
**78327** 5:23p ⚖️ **Vsock Data-Path Replacement Design Rejected and Scheduled for Withdrawal**
User rejected proposed vsock replacement design for netns-density-295 feature after discovering fundamental architectural conflict with GitHub issue #303 in-guest kernel mTLS proposal. Vsock design decisions V1/V2/V8 incorrectly framed vsock as data-plane transport carrying all application traffic via overdrive-init tun0 packet pump over vsock stream to serve-internal forwarder feeding shared host TUN. This introduced per-packet userspace forwarding through Cloud Hypervisor muxer then serve forwarder, putting serve process in data path so serve death terminates all guest connectivity. Architecture directly contradicts GH #303's approved principle that host agent leaves data path after handshake, with vsock used only for control-plane handshake relay and application ciphertext flowing over guest's own TCP socket with kTLS through virtio-net device. Design error originated from incorrect framing treating vsock as data transport rather than control channel. User directed immediate withdrawal of all uncommitted vsock design artifacts: six new ADRs (0145-0150), feature-delta V-0 through V-19 sections, and proposed-status notes added to ten existing ADRs (0114/0117/0124/0126/0127/0128/0130/0131/0142/0143). Rejection preserves approved research recommendation for routed TAP with virtio-net as first bounded replacement experiment keeping existing TPROXY/kTLS until #303 replaces it.
~557t ⚖️ 2,701

**78328** 5:24p ✅ **Vsock Data-Path Design Documentation Withdrawn and Reverted**
Vsock data-path replacement design documentation completely withdrawn from repository. Bash command deleted six newly created ADR files (0145-microvm-network-attachment-over-vsock-packet-uplink, 0146-node-shared-tun-and-in-serve-uplink-forwarder, 0147-uplink-identity-is-the-allocation-muxer-socket, 0148-attachment-activation-and-quiescence-are-the-forwarder-gate, 0149-shared-tun-classifier-and-ip-family-guard, 0150-e18-attachment-population-is-activated-vsock-uplinks) and reverted eleven modified files back to their HEAD state using git show to restore original content. Reverted files include feature-delta.md (removing V-0 through V-19 vsock sections) and ten existing ADRs where proposed supersession or amendment status notes had been added (0114 node-local bridge, 0117 density target, 0124 recovery, 0126 bridge MAC, 0127 TAP queue descriptor, 0128 VMM adapter queue ownership, 0130 unprivileged owner grant, 0131 activation timing, 0142 egress MAC classifier, 0143 seccomp filter). Git status confirms working tree now contains only legitimate in-progress work: AGENTS.md modifications, crate source edits in overdrive-control-plane/overdrive-dataplane/overdrive-worker, execution-log.json, and bin/ directory. All vsock packet-forwarding design artifacts from current session eliminated leaving no trace in documentation tree. Repository state restored to pre-vsock-design baseline preserving bridge-based implementation and its pending validation status.
~569t 🛠️ 4,224


Access 1676k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>
