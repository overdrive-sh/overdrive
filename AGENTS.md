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

# [helios/baghdad-v1] recent context, 2026-09-25 8:30pm GMT+2

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision 🚨security_alert 🔐security_note
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 50 obs (26,255t read) | 550,421t work | 95% savings

### May 24, 2026
S8700 Mapping blue/green deployment scenario with intelligent VM provisioning onto Overdrive architecture (May 24 at 10:41 AM)
### Jun 17, 2026
S8704 Create GitHub issue for machine-provisioner primitive gap (Jun 17 at 10:11 AM)
S8705 Create GitHub issue for machine-provisioner primitive gap in Overdrive (Jun 17 at 10:11 AM)
S8706 Create GitHub issue documenting machine-provisioner primitive gap for elastic cloud VM provisioning (Jun 17 at 10:13 AM)
S13839 Architectural investigation: why Raft & Corrosion are both needed vs atomic broadcast with viewstamped replication and gossip with CRDTs (Jun 17 at 10:44 AM)
### Jul 29, 2026
57757 5:32p 🔵 TigerBeetle VOPR Fuzzing Fleet: 1,024 Cores Running 24/7 at 700× Real-Time Speed, 2 Millennia Simulated Per Day
57761 5:38p 🔵 Wayback Machine Rate Limit Persists Beyond 5-Minute Backoff
57762 " 🔵 CORS Proxy Services Fail to Access Wayback Machine Content
57763 " 🔵 AWS Systems Correctness Practices Research Findings via Alternative Sources
57764 5:39p 🔵 Archive Services Implement Coordinated Rate Limiting
57765 " 🔵 AWS Systems Correctness Paper Publication Details Located
57766 5:40p 🔵 Common Crawl Index Successfully Accessed for Web Archive Alternative
57767 " 🔵 Marc Brooker Publications Page Provides Direct Paper References
57768 " 🔵 Common Crawl Index Located Two Complete Captures of AWS Correctness Paper
57769 5:41p 🔵 Successfully Extracted Full AWS Correctness Paper from Common Crawl WARC Archive
57770 5:42p 🔵 Complete AWS Systems Correctness Paper Text Successfully Extracted
57771 " 🔵 Complete References and Metadata Extracted from AWS Correctness Paper
57772 " 🔵 Key Technical Concepts and Statistics Verified in AWS Correctness Paper
57773 " 🔵 Complete AWS Systems Correctness Paper Retrieved and Analyzed via Subagent
57775 5:43p 🔵 Woodcock-Larsen Critical Evaluation Paper Confirmed Open Access But No PDF Access Available
57774 5:45p 🔵 Related Critical Evaluation Paper on AWS Formal Methods Discovered as Open Access
57776 5:49p 🔵 Aarhus University OAI-PMH Endpoint Accessible While York Protected
57777 5:50p 🔵 Aarhus University OAI-PMH Repository Successfully Harvested for Publication Window
57778 5:51p 🔵 OAI-PMH Date Filtering Confirmed but Record Format Shows Person Names Not Paper Titles
57779 " 🔵 Aarhus Pure OAI Repository Sets Enable Publication-Specific Harvesting
57780 5:52p 🔵 All Alternative Access Methods for Woodcock-Larsen Paper Exhausted With Zero Success
57781 5:53p 🔵 Aarhus Pure Web Interface Returns HTTP 403 Cloudflare Protection for Woodcock Publication Listings
57782 5:54p 🔵 CrossRef Metadata Confirms Paper Existence But Lists Null License and Similarity-Checking PDF Only
57783 " 🔵 White Rose Repository Search by Author Name Accessible But Results Content Not Captured
57784 " 🔵 White Rose Repository Contains 7 Woodcock Publications from 2025-2026 But Target Paper Absent
57785 " 🔵 Browser Automation Infrastructure Available But Python Libraries Not Installed
57786 5:55p 🔵 Headless Chrome Blocked by Cloudflare Bot Detection on ACM DOI Page
57787 " 🔵 Python Virtual Environment Created With websocket-client for CDP Automation
57788 5:57p 🔵 Chrome DevTools Protocol Endpoint Successfully Accessible on Port 9333
57789 " 🔵 Chrome Headless Started Successfully With Anti-Bot-Detection Flags on Port 9335
57790 " 🔵 Chrome WebSocket Connection Rejected With HTTP 403 Due to Missing Origin Allowlist Flag
57791 " 🔵 CDP Automation Successfully Connected But Cloudflare Challenge Runs Indefinitely Without Resolving
### Sep 7, 2026
73345 2:24a ⚖️ Schema Evolution Coverage Excluded from Scope
73347 " 🟣 Timestamp Hardening Work Initiated
73350 " 🔵 Timestamp Inventory Agent Repeatedly Timing Out After Multiple Retry Attempts
73351 2:26a 🔵 Timestamp Inventory Agent Completed After Extended Timeout Retry Cycle
73348 " 🔵 Timestamp Inventory Agent Exceeded 60-Second Wait Timeout
73353 2:27a 🔵 UnixInstant Atomicity Investigation for Concurrent Timestamp Access
### Sep 24, 2026
77485 2:36a 🔵 Baghdad-v1 uses dual-store architecture: Raft for IntentStore and Corrosion for ObservationStore
77486 " 🔵 Corrosion ObservationStore architecture driven by Fly.io production incident learnings and DST testing
77487 2:37a 🔵 Whitepaper Design Principle #9 defines Intent/Observation split rationale rejecting consensus-for-everything
77488 " 🔵 Corrosion production scale evidence shows 800-server Fly.io deployment with three major incident classes informing guardrails
77489 " 🔵 CRDT theory distinguishes G-Counter from scalar LWW-Register with single-writer-per-key safety requirement for monotone counters
77490 2:39a 🔵 Phase 1 implements LocalIntentStore and LocalObservationStore placeholders with RaftStore and CorrosionStore planned for Phase 2
77491 " 🔵 Antithesis defines atomic broadcast as totally-ordered command sequence commonly implemented via consensus algorithms like Raft or Viewstamped Replication
S13840 Deep evaluation of viewstamp library as Viewstamped Replication alternative to openraft for IntentStore implementation in issue #67 (Sep 24 at 2:40 AM)
**77492** 2:42a 🔵 **Viewstamp repository is 8-month-old single-author project with 1 star and reconfiguration support implemented**
Investigation into viewstamp repository metadata reveals it is very recent library created January 29 2026 approximately 8 months before current date with minimal community adoption (1 star, 0 forks) and single primary developer al8n responsible for 282 of 285 total commits. Repository shows active development with last push September 5 2026 and Apache 2.0 license compatible with baghdad-v1 project. Despite pre-0.1 unstable status noted in documentation codebase demonstrates significant implementation completeness with membership reconfiguration as first-class feature evidenced by dedicated modules (membership, reconfigure_plan, endpoint/reconfig.rs, endpoint/reconfigure.rs) and extensive testing including simulation-based tests (reconfig_ingress.rs, reconfig_live.rs, epoch_ingress.rs, learner_membership.rs) plus integration tests for both compio and reactor I/O runtimes. Reconfiguration support addresses one of Raft's operational challenges (adding/removing nodes) suggesting viewstamp aims for production-readiness despite youth. Single-author concentration represents significant risk factor: al8n is sole domain expert with no demonstrated community to sustain project if author becomes unavailable or loses interest. Contrast with openraft which has established community, multiple contributors, and production deployments. Viewstamp's simulation-driven testing approach aligns well with baghdad-v1's Deterministic Simulation Testing methodology and Antithesis integration goals but library's 8-month age and 1-star adoption indicate unproven status compared to openraft's maturity. For Phase 2 consensus implementation this represents classic early-adopter risk/reward trade-off: better simulation testing alignment and TigerBeetle-derived storage fault model versus single-author project with zero demonstrated production usage.
~803t 🔍 2,350

**77493** " 🔵 **Viewstamp source code shows no evidence of linearizable read optimizations or lease-based read paths**
Source code inspection of viewstamp repository reveals absence of standard Raft read optimization patterns suggesting either immature read path implementation or fundamentally different approach. Grep searches across core library files (viewstamp-proto/src/lib.rs, viewstamp-driver/src/lib.rs) and test suite found zero matches for linearizable reads, read-index protocol, lease-based reads, stale-read modes, or read-only query optimizations that mature Raft implementations like openraft provide for serving reads without log round-trip. Single read-related test file read_delay.rs focuses on WAL read latency during crash recovery testing scenario where solo voter replica must wait for slow disk read to complete rather than optimized read serving to clients. Test exercises fault-injection path where minority of WAL slots answer only past give-up horizon forcing recovery wait discipline to become load-bearing, demonstrating storage-fault-first approach inherited from TigerBeetle but not client read optimization. README mentions learners as separate role for read/standby scale-out suggesting awareness of read scaling concern but providing no implementation detail on how linearizable reads work or whether they require primary round-trip. This gap matters for baghdad-v1 IntentStore use case where control plane reconcilers reading workload specs, policies, and scheduler decisions may generate significant read load requiring optimized path. Openraft provides ReadIndex and lease-based reads avoiding log writes for read-only operations - viewstamp showing no evidence of equivalent optimization means all reads may require primary involvement or lack linearizability guarantees. Combined with 8-month age, single author, and 1-star adoption this represents additional risk factor beyond unstable wire format: feature completeness for production IntentStore workload unknown without deeper investigation or prototype implementation.
~836t 🔍 5,802

S13842 Viewstamped Replication Scalability Analysis - Voting Group Constraints and System-Wide Scaling Strategies (Sep 24 at 2:42 AM)
**77494** 2:44a 🔵 **IntentStore trait contract explicitly requires linearizable reads with future RaftStore implementation planned**
Codebase inspection confirms IntentStore and ObservationStore dual-store architecture is deeply embedded across trait contracts, ADRs, and implementation expectations despite only local Phase 1 adapters currently existing. IntentStore trait documentation explicitly states linearizable authoritative storage requirement with openraft + redb planned for future RaftStore implementation. Trait contract provides get, put, delete, txn, watch primitives plus export_snapshot and bootstrap_from methods with StateSnapshot type explicitly designed for single-mode LocalStore to multi-node RaftStore migration path. Multiple codebase locations reinforce linearizable requirement: CA root key material documented as linearizable intent never observation, VIP allocator relies on IntentStore as linearizable-state flat key-value mapping, service map hydrator uses linearizable Raft path for intent flow. This confirms whitepaper claim that linearizable reads are promised contract not optional optimization. ObservationStore trait documented as live eventually-consistent cluster map with production implementation planned using Corrosion (cr-sqlite + SWIM/QUIC gossip) and simulation using injectable gossip-delay and partition support. Comments throughout ObservationStore trait reference single-writer-at-a-time discipline from ADR-0077, gossip in-flight scenarios, LWW idempotency for re-delivered rows, and Phase 2 Corrosion replacement gossiping rows under same LWW semantics. Twenty-three ADRs already mention Raft or Corrosion architectural decisions showing dual-store pattern influences CA material handling, workflow journal layout, reconciler terminal conditions, LWW counter semantics, and observation store server implementation. ADR-0020 explicitly states Intent/Observation split where intent is linearizable written once at commit while observation is owner-writer eventually consistent. This demonstrates viewstamp or any IntentStore implementation must provide linearizable read guarantee not just write ordering, making missing read-index/lease-read primitives significant gap requiring spike validation before adoption.
~1021t 🔍 7,624

**77495** 2:45a 🔵 **Codebase has single/HA mode infrastructure with Phase 1 hardcoded to single and CLI prepared for future HA mode**
Code inspection reveals baghdad-v1 Phase 1 implementation has infrastructure for single/HA mode split already designed into codebase architecture but currently hardcoded to single mode pending Phase 2 distributed implementation. Control plane handlers include comment explicitly stating Phase 1 scope mode is always single with HA arriving Phase 2+ and ClusterStatus response hardcodes mode string to single. However CLI argument parser already accepts --mode flag with value_parser allowing both single and ha options showing future HA support infrastructure prepared in user-facing tooling. IntentStore trait documentation describes mode split clearly stating Single mode backed by redb direct storage while HA mode uses openraft + redb consensus with simulation harness using single-mode path since Raft itself tested separately by dedicated consensus tests. This confirms whitepaper Design Principle #8 (one binary any topology with role declared at bootstrap not build time) and whitepaper §4 IntentStore description of LocalStore vs RaftStore implementations are already designed into trait surface and CLI tooling. Phase 1 delivers single-mode-only implementation with all HA mode references as forward-compatible placeholders awaiting Phase 2 openraft or viewstamp integration via issue #67. This mode split infrastructure means whichever consensus library adopted must implement same IntentStore trait behind mode configuration flag matching existing CLI surface and cluster info response shape.
~645t 🔍 2,160

S13843 Overdrive distributed architecture using blockchain-inspired gossip discovery and Viewstamped Replication consensus protocol validation (Sep 24 at 2:45 AM)
S13841 Evolved architectural design from Raft & Corrosion rationale into flat learner-based cluster model using viewstamp with auto-promotion reconciler (Sep 24 at 2:45 AM)
**77539** 2:47a ⚖️ **Overdrive Distributed Architecture Using Blockchain-Inspired Gossip and Consensus**
Overdrive workload orchestration platform adopting blockchain-inspired distributed architecture. Each node runs both worker (execution) and control plane (coordination) components creating fully distributed system without centralized control plane. Consensus mechanism coordinates scheduling decisions ensuring agreement across cluster on workload placement. Machine discovery uses gossip protocol: when machine configures WireGuard endpoint to another machine, connected machine broadcasts all available machines while new machine broadcasts itself, creating peer-to-peer discovery similar to blockchain node bootstrapping. Leader election mechanism selects coordinator machine from cluster. All machines participate in single topology enabling workload distribution. Workloads can specify explicit machine placement constraints or omit placement configuration allowing Overdrive to automatically spread instances across available machines. Architecture design principle: every component has two execution modes (local or remote) enabling flexible deployment patterns. This approach provides decentralized coordination, self-healing discovery, and horizontal scalability without single point of failure.
~519t ⚖️ 13,034

### Sep 25, 2026
S13844 Overdrive distributed architecture design: VR consensus scaling, per-node component roles, and blockchain-inspired discovery patterns (Sep 25 at 1:23 AM)
**Investigated**: Examined VR consensus voting group scaling limits and how consensus systems scale beyond small quorums. Analyzed issue #267 proposing per-node component enablement (gateway, web, control-plane, worker, telemetry, wasm, storage) for Overdrive orchestration platform. Mapped blockchain validator model and discovery patterns to Overdrive's VR-based architecture. Explored separation between gossip-based observation layer and consensus-based decision layer.

**Learned**: VR voting group stays small (3-5 nodes) due to primary egress bandwidth, tail latency from f-th fastest backup, no resilience gain beyond 2f+1, heavier view changes and recovery, and increased membership change frequency. Consensus systems scale via non-voting learners for reads, peer-to-peer log relay to offload primary fan-out, batching before sharding, and keeping high-volume observation data off consensus log. Flat cluster architecture uses 3-5 voters placed across failure domains with thousands of learners all holding complete log enabling instant promotion and local reads. Per-node component roles stored in replicated log (not gossip) with control-plane marking "may vote" eligibility while cluster autonomously selects actual voters. All components enabled by default preserving zero-config single-node and join workflows. Blockchain validator model validates bounded-voter approach with validators as voters and full nodes as learners using deterministic round-robin leader rotation. Admission control via join tokens required since no proof-of-work/stake prevents Sybil attacks. Leader election must use quorum-based view change not gossip to prevent split-brain across partition. Consensus commits placement decisions not scheduling computation preserving replica determinism across version skew during rolling upgrades. Workers read assignments from local log copy eliminating push from leader. Gossip layer carries ephemeral observation facts (endpoints, heartbeats, health, allocation status) while log layer carries durable decisions (membership, roles, voter selection, placement, "machine M down" rulings). Leader observes gossip then commits decisions creating clear layer separation.

**Completed**: Architectural design completed for Overdrive consensus-based orchestration combining VR protocol with blockchain-inspired discovery. Scaling model validated supporting thousands of nodes via small voter set with learner replicas. Per-node component role model designed supporting control-plane voter eligibility, worker scheduling capability, and other subsystem flags. Blockchain pattern mapping completed identifying useful elements (gossip discovery, validator model, deterministic leader rotation) and rejected elements (BFT 3f+1 overhead, probabilistic finality). Clear architectural principle established separating gossip observation layer from consensus decision layer.

**Next Steps**: Potential implementation of per-node component enablement from issue #267. Design admission control join token mechanism for WireGuard key authorization. Implement gossip protocol for endpoint and health propagation. Design automatic voter selection algorithm across failure domains. Define workload placement decision commit format in log.


Access 550k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>
